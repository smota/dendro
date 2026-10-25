//! Unprivileged process/resource sampling for agent sessions (Windows and Linux).
//!
//! Discovery: the caller's own processes whose executable name is a known
//! harness, plus their descendants (by parent pid, verified by start time so a
//! reused pid is never adopted). Each tick records CPU time, resident/working
//! set and disk bytes. Only the executable *name* is read: never a path, an
//! argument vector, an environment or a working-directory text. The working
//! directory of a root is kept as a keyed fingerprint, for session linking only.
//!
//! Nothing unreadable is stored as zero: a missing value is NULL plus a
//! visibility limit (`permission_denied`, `process_exited`, `counter_reset`,
//! `truncated`, ...). A wrapper that knows the exact root pid feeds it with
//! [`Config::pinned_roots`] instead of relying on the name match.
//!
//! Unit mapping (the closest allowed domain units; `sysinfo` is the only reader):
//! * CPU: total user+system time, `Unit::Microseconds` (sysinfo cannot split it).
//! * Memory: `Unit::LinuxRssBytes` on Linux, `Unit::WindowsWorkingSetBytes` on
//!   Windows. Elsewhere `Unit::Bytes` with `CaptureMethod::Unknown`.
//! * Disk: cumulative `Unit::Bytes` read/written. On Windows sysinfo reports
//!   all I/O transfers of the process, not only disk.
//! * Process creation time: whole Unix seconds (`start_time_unix_s`), the finest
//!   sysinfo reads; the domain `CreationMarker` ticks/FILETIME are not available.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use dendro_core::domain_v1::{CaptureMethod, EnvironmentKind, Unit, VisibilityLimit};
use dendro_core::{opaque_id, redact, ProcessIdentityFact, ProcessSampleFact};

/// At most this many processes are tracked at once.
pub const MAX_TRACKED: usize = 64;
/// At most this many samples per process per watch session.
pub const MAX_SAMPLES_PER_PROCESS: usize = 600;
pub const DEFAULT_HARNESSES: &[&str] = &["claude", "codex", "grok", "pi"];
/// A session links to a root only when its first record falls in
/// `[start - 1s, start + LINK_WINDOW_S]`.
const LINK_WINDOW_S: i64 = 60;
/// Interval flushes that fail in a row before the watch stops and reports
/// how many samples are still unsaved.
const MAX_CONSECUTIVE_FLUSH_FAILURES: u32 = 5;
/// Stop, once, and the end of a watch retry the last flush this many times.
const FINAL_FLUSH_ATTEMPTS: u32 = 3;
const FINAL_FLUSH_BACKOFF: Duration = Duration::from_millis(50);

#[derive(Clone, Debug)]
pub struct RawProcess {
    pub pid: u32,
    pub parent: Option<u32>,
    /// Creation time, whole Unix seconds; 0 when the OS refused to say.
    pub start_s: i64,
    /// Executable name only.
    pub name: String,
    /// Owned by the same user as this process.
    pub own: bool,
}

/// What one refresh could read for one process; unreadable values are `None`
/// with a limit, never zero.
#[derive(Clone, Debug, Default)]
pub struct RawMetrics {
    pub cpu_time_us: Option<u64>,
    pub rss_bytes: Option<u64>,
    pub disk_read_bytes: Option<u64>,
    pub disk_write_bytes: Option<u64>,
    pub limits: BTreeSet<VisibilityLimit>,
}

/// The OS view the collector samples; a fake implements it in tests.
pub trait ProcessSource {
    /// Every process visible to the caller (structure only).
    fn scan(&mut self) -> Vec<RawProcess>;
    /// Resource readings for the given pids; a pid that vanished is absent.
    fn metrics(&mut self, pids: &[u32]) -> BTreeMap<u32, RawMetrics>;
    /// Working directory, read once for a new root and fingerprinted at once.
    fn cwd(&mut self, pid: u32) -> Option<PathBuf>;
}

#[derive(Clone, Debug)]
pub struct Config {
    pub harnesses: Vec<String>,
    pub max_tracked: usize,
    pub max_samples: usize,
    /// Exact root pids supplied by a launcher; tracked whatever their name.
    pub pinned_roots: BTreeSet<u32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            harnesses: DEFAULT_HARNESSES.iter().map(|h| h.to_string()).collect(),
            max_tracked: MAX_TRACKED,
            max_samples: MAX_SAMPLES_PER_PROCESS,
            pinned_roots: BTreeSet::new(),
        }
    }
}

/// A harness session known from ingested records. `cwd_fingerprint` is the
/// same keyed fingerprint [`cwd_fingerprint`] produces; no connector records a
/// working directory today, so sessions currently carry `None` and never link.
#[derive(Clone, Debug)]
pub struct SessionCandidate {
    pub native_session: String,
    pub first_ts_unix_s: i64,
    pub cwd_fingerprint: Option<String>,
}

/// Link a root to a session only on real evidence: matching working-directory
/// fingerprint AND a first record inside the start window, and exactly one such
/// session. Anything else leaves the root unlinked.
pub fn link_session(
    start_s: i64,
    cwd_fp: Option<&str>,
    sessions: &[SessionCandidate],
) -> Option<String> {
    let fp = cwd_fp?;
    let mut hits = sessions.iter().filter(|s| {
        s.cwd_fingerprint.as_deref() == Some(fp)
            && (start_s - 1..=start_s + LINK_WINDOW_S).contains(&s.first_ts_unix_s)
    });
    let first = hits.next()?;
    hits.next().is_none().then(|| first.native_session.clone())
}

pub fn cwd_fingerprint(key: &[u8], cwd: &Path) -> String {
    let mut text = cwd.to_string_lossy().replace('\\', "/");
    while text.len() > 1 && text.ends_with('/') {
        text.pop();
    }
    if cfg!(windows) {
        text = text.to_lowercase();
    }
    opaque_id(key, "process-cwd-v1", &text)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Isolation {
    pub kind: EnvironmentKind,
    pub hint: &'static str,
}

const UNDETECTED: Isolation = Isolation {
    kind: EnvironmentKind::Unknown,
    hint: "undetected",
};

/// Only certain markers count; absence of a marker proves nothing.
pub fn classify_linux(dockerenv: bool, proc_version: &str, cgroup: &str) -> Isolation {
    let cgroup = cgroup.to_ascii_lowercase();
    if dockerenv {
        Isolation {
            kind: EnvironmentKind::Container,
            hint: "docker",
        }
    } else if cgroup.contains("kubepods") {
        Isolation {
            kind: EnvironmentKind::Container,
            hint: "kubernetes",
        }
    } else if ["docker", "containerd", "libpod"]
        .iter()
        .any(|m| cgroup.contains(m))
    {
        Isolation {
            kind: EnvironmentKind::Container,
            hint: "container",
        }
    } else if proc_version.to_ascii_lowercase().contains("microsoft") {
        Isolation {
            kind: EnvironmentKind::Vm,
            hint: "wsl",
        }
    } else {
        UNDETECTED
    }
}

pub fn detect_isolation() -> Isolation {
    #[cfg(target_os = "linux")]
    {
        let read = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
        classify_linux(
            Path::new("/.dockerenv").exists(),
            &read("/proc/version"),
            &read("/proc/1/cgroup"),
        )
    }
    #[cfg(not(target_os = "linux"))]
    UNDETECTED
}

fn platform() -> (&'static str, CaptureMethod, Unit) {
    if cfg!(windows) {
        (
            "windows",
            CaptureMethod::WindowsProcessApi,
            Unit::WindowsWorkingSetBytes,
        )
    } else if cfg!(target_os = "linux") {
        ("linux", CaptureMethod::Procfs, Unit::LinuxRssBytes)
    } else {
        ("other", CaptureMethod::Unknown, Unit::Bytes)
    }
}

struct Tracked {
    fact: ProcessIdentityFact,
    samples: usize,
    /// Last CPU, disk-read and disk-write totals that were actually readable.
    /// `None` is not a zero baseline and is never compared.
    last: (Option<u64>, Option<u64>, Option<u64>),
}

/// What one flush holds, still unkeyed: ids are placeholders and root working
/// directories are paths. Not `Debug` on purpose; [`Batch::seal`] makes the
/// storable form.
pub struct Batch {
    pub identities: Vec<ProcessIdentityFact>,
    pub samples: Vec<ProcessSampleFact>,
    cwds: BTreeMap<String, PathBuf>,
}

/// The storable form: keyed opaque ids, fingerprints instead of paths.
#[derive(Debug, Default)]
pub struct Sealed {
    pub identities: Vec<ProcessIdentityFact>,
    pub samples: Vec<ProcessSampleFact>,
}

impl Batch {
    /// Replace placeholder ids with keyed ones and fingerprint root working
    /// directories. A root links to a session only through [`link_session`].
    pub fn seal(self, key: &[u8], sessions: &[SessionCandidate]) -> Sealed {
        let id = |raw: &str| opaque_id(key, "process-identity-v1", raw);
        let mut identities = self.identities;
        for i in &mut identities {
            if let Some(cwd) = self.cwds.get(&i.identity_key) {
                let fp = cwd_fingerprint(key, cwd);
                i.session_link = link_session(i.start_time_unix_s, Some(&fp), sessions);
                i.cwd_fingerprint = Some(fp);
            }
            i.root_key = id(&i.root_key);
            i.identity_key = id(&i.identity_key);
        }
        let mut samples = self.samples;
        for s in &mut samples {
            s.identity_key = id(&s.identity_key);
            s.watch_id = opaque_id(key, "process-watch-v1", &s.watch_id)[..16].to_string();
        }
        Sealed {
            identities,
            samples,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WatchSummary {
    pub ticks: usize,
    pub identities: usize,
    pub samples: usize,
    /// Distinct processes not tracked because the tracking bound was reached.
    pub refused_by_cap: usize,
}

pub struct Collector<S> {
    source: S,
    config: Config,
    watch_id: String,
    isolation: Isolation,
    /// Root working directories, held in memory only until the batch is sealed
    /// with the analytics key into a fingerprint.
    cwds: BTreeMap<String, PathBuf>,
    tracked: BTreeMap<u32, Tracked>,
    dirty: BTreeMap<String, ProcessIdentityFact>,
    samples: Vec<ProcessSampleFact>,
    refused: BTreeSet<(u32, i64)>,
    summary: WatchSummary,
}

impl<S: ProcessSource> Collector<S> {
    pub fn new(source: S, config: Config, isolation: Isolation) -> Self {
        let watch = format!(
            "{}:{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        Self {
            source,
            config,
            watch_id: watch,
            isolation,
            cwds: BTreeMap::new(),
            tracked: BTreeMap::new(),
            dirty: BTreeMap::new(),
            samples: Vec::new(),
            refused: BTreeSet::new(),
            summary: WatchSummary::default(),
        }
    }

    pub fn summary(&self) -> WatchSummary {
        WatchSummary {
            refused_by_cap: self.refused.len(),
            ..self.summary.clone()
        }
    }

    fn matches_harness(&self, name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        let base = lower.strip_suffix(".exe").unwrap_or(&lower);
        self.config
            .harnesses
            .iter()
            .any(|h| h.eq_ignore_ascii_case(base))
    }

    /// One observation: end what vanished, adopt what qualifies, sample all.
    pub fn tick(&mut self, now: DateTime<Utc>) {
        self.summary.ticks += 1;
        let scan = self.source.scan();
        let live: BTreeMap<u32, &RawProcess> = scan.iter().map(|p| (p.pid, p)).collect();
        // Refusals are only for processes still in this scan, so a long watch
        // does not keep every pid it ever declined.
        self.refused.retain(|(pid, start_s)| {
            live.get(pid)
                .is_some_and(|p| p.start_s == 0 || p.start_s == *start_s)
        });
        // A pid now carrying another start time is a reused pid: the old
        // process exited, the new one is judged afresh below. 0 means the
        // start time was unreadable this tick: keep the identity, do not end
        // it, and do not open a new one.
        let gone: Vec<u32> = self
            .tracked
            .iter()
            .filter(|(pid, t)| match live.get(pid) {
                None => true,
                Some(p) if p.start_s == 0 => false,
                Some(p) => p.start_s != t.fact.start_time_unix_s,
            })
            .map(|(pid, _)| *pid)
            .collect();
        for pid in gone {
            if let Some(mut t) = self.tracked.remove(&pid) {
                t.fact.ended_at = Some(now);
                t.fact
                    .visibility_limits
                    .insert(VisibilityLimit::ProcessExited);
                self.dirty.insert(t.fact.identity_key.clone(), t.fact);
            }
        }
        self.discover(&scan, now);
        self.sample(now);
    }

    fn discover(&mut self, scan: &[RawProcess], now: DateTime<Utc>) {
        let mut candidates: Vec<&RawProcess> = scan
            .iter()
            .filter(|p| !self.tracked.contains_key(&p.pid))
            .collect();
        // A launcher's pinned roots claim the tracking bound before name matches.
        let pinned = &self.config.pinned_roots;
        candidates.sort_by_key(|p| (!pinned.contains(&p.pid), p.start_s, p.pid));
        loop {
            let mut added = false;
            for p in &candidates {
                if self.tracked.contains_key(&p.pid) {
                    continue;
                }
                if p.start_s <= 0 {
                    // Identity cannot be verified, so it is never adopted; a
                    // tracked parent's tree says it may be incomplete.
                    if let Some(root) = p
                        .parent
                        .and_then(|pp| self.tracked.get(&pp))
                        .map(|t| t.fact.root_key.clone())
                    {
                        self.limit_root(&root, VisibilityLimit::PermissionDenied);
                    }
                    continue;
                }
                // A child cannot be older than its parent: otherwise the parent
                // pid was reused and the link is stale.
                let parent = p
                    .parent
                    .and_then(|pp| self.tracked.get(&pp))
                    .filter(|t| p.start_s >= t.fact.start_time_unix_s);
                let root_key = parent.map(|t| t.fact.root_key.clone());
                let is_root = parent.is_none()
                    && (self.config.pinned_roots.contains(&p.pid)
                        || (p.own && self.matches_harness(&p.name)));
                if root_key.is_none() && !is_root {
                    continue;
                }
                if self.tracked.len() >= self.config.max_tracked {
                    self.refused.insert((p.pid, p.start_s));
                    if let Some(root) = &root_key {
                        self.limit_root(root, VisibilityLimit::Truncated);
                    }
                    continue;
                }
                self.track(p, root_key, now);
                added = true;
            }
            if !added {
                break;
            }
        }
    }

    fn limit_root(&mut self, root_key: &str, limit: VisibilityLimit) {
        if let Some(t) = self
            .tracked
            .values_mut()
            .find(|t| t.fact.identity_key == root_key)
        {
            if t.fact.visibility_limits.insert(limit) {
                self.dirty
                    .insert(t.fact.identity_key.clone(), t.fact.clone());
            }
        }
    }

    fn track(&mut self, p: &RawProcess, root_key: Option<String>, now: DateTime<Utc>) {
        let (platform, _, _) = platform();
        // A pinned pid is consumed once adopted, so a reused pid is not pinned.
        self.config.pinned_roots.remove(&p.pid);
        // Unkeyed placeholder; `Batch::seal` replaces it with the keyed id.
        let identity_key = format!("{platform}:{}:{}", p.pid, p.start_s);
        let harness = if root_key.is_none() {
            if let Some(cwd) = self.source.cwd(p.pid) {
                self.cwds.insert(identity_key.clone(), cwd);
            }
            Some(redact(&p.name))
        } else {
            None
        };
        let fact = ProcessIdentityFact {
            root_key: root_key.unwrap_or_else(|| identity_key.clone()),
            identity_key,
            pid: p.pid,
            parent_pid: p.parent,
            platform: platform.to_string(),
            start_time_unix_s: p.start_s,
            exe_name: redact(&p.name),
            harness,
            cwd_fingerprint: None,
            session_link: None,
            environment_kind: self.isolation.kind,
            isolation_hint: self.isolation.hint.to_string(),
            first_seen: now,
            last_seen: now,
            ended_at: None,
            visibility_limits: BTreeSet::new(),
        };
        self.tracked.insert(
            p.pid,
            Tracked {
                fact,
                samples: 0,
                last: (None, None, None),
            },
        );
    }

    fn sample(&mut self, now: DateTime<Utc>) {
        let pids: Vec<u32> = self.tracked.keys().copied().collect();
        let mut readings = self.source.metrics(&pids);
        let (_, capture_method, rss_unit) = platform();
        let to_i64 = |v: u64| i64::try_from(v).ok();
        for (pid, t) in &mut self.tracked {
            t.fact.last_seen = now;
            if t.samples >= self.config.max_samples {
                t.fact.visibility_limits.insert(VisibilityLimit::Truncated);
                self.dirty
                    .insert(t.fact.identity_key.clone(), t.fact.clone());
                continue;
            }
            // The process vanished between the scan and the reading.
            let m = readings.remove(pid).unwrap_or_else(|| RawMetrics {
                limits: [VisibilityLimit::ProcessExited].into(),
                ..RawMetrics::default()
            });
            let mut limits = m.limits;
            // Compare a counter only when this sample and the previous one
            // both actually read it. An unreadable field is not a reset.
            let (prev_cpu, prev_read, prev_write) = t.last;
            let fell = |now, prev| matches!((now, prev), (Some(now), Some(prev)) if now < prev);
            if fell(m.cpu_time_us, prev_cpu)
                || fell(m.disk_read_bytes, prev_read)
                || fell(m.disk_write_bytes, prev_write)
            {
                limits.insert(VisibilityLimit::CounterReset);
            }
            t.last = (
                m.cpu_time_us.or(prev_cpu),
                m.disk_read_bytes.or(prev_read),
                m.disk_write_bytes.or(prev_write),
            );
            self.samples.push(ProcessSampleFact {
                watch_id: self.watch_id.clone(),
                identity_key: t.fact.identity_key.clone(),
                seq: t.samples as i64,
                ts: now,
                cpu_time_us: m.cpu_time_us.and_then(to_i64),
                rss_bytes: m.rss_bytes.and_then(to_i64),
                rss_unit,
                disk_read_bytes: m.disk_read_bytes.and_then(to_i64),
                disk_write_bytes: m.disk_write_bytes.and_then(to_i64),
                capture_method,
                visibility_limits: limits,
            });
            t.samples += 1;
            self.dirty
                .insert(t.fact.identity_key.clone(), t.fact.clone());
        }
    }

    /// Take everything not yet published.
    pub fn drain(&mut self) -> Batch {
        let identities: Vec<ProcessIdentityFact> =
            std::mem::take(&mut self.dirty).into_values().collect();
        let cwds = identities
            .iter()
            .filter_map(|i| {
                Some((
                    i.identity_key.clone(),
                    self.cwds.get(&i.identity_key)?.clone(),
                ))
            })
            .collect();
        // The ending batch still carries the directory; the collector does not
        // keep it after the root has ended.
        for i in &identities {
            if i.ended_at.is_some() {
                self.cwds.remove(&i.identity_key);
            }
        }
        let batch = Batch {
            identities,
            samples: std::mem::take(&mut self.samples),
            cwds,
        };
        self.summary.identities += batch.identities.len();
        self.summary.samples += batch.samples.len();
        batch
    }

    /// Put a batch `drain` already counted back, so a failed flush loses nothing.
    fn restore_unflushed(&mut self, batch: Batch) {
        self.summary.identities = self
            .summary
            .identities
            .saturating_sub(batch.identities.len());
        self.summary.samples = self.summary.samples.saturating_sub(batch.samples.len());
        for (key, cwd) in batch.cwds {
            self.cwds.entry(key).or_insert(cwd);
        }
        for fact in batch.identities {
            self.dirty.insert(fact.identity_key.clone(), fact);
        }
        let mut samples = batch.samples;
        samples.append(&mut self.samples);
        self.samples = samples;
    }

    fn sample_buffer_limit(&self) -> usize {
        self.config
            .max_tracked
            .saturating_mul(self.config.max_samples)
    }
}

enum FlushAttempt {
    Saved,
    Held(anyhow::Error),
}

/// Publish one batch. On error the same rows are back in the collector.
fn publish<S: ProcessSource>(
    collector: &mut Collector<S>,
    flush: &mut impl FnMut(Batch) -> anyhow::Result<()>,
) -> anyhow::Result<FlushAttempt> {
    let batch = collector.drain();
    let replay = Batch {
        identities: batch.identities.clone(),
        samples: batch.samples.clone(),
        cwds: batch.cwds.clone(),
    };
    match flush(batch) {
        Ok(()) => Ok(FlushAttempt::Saved),
        Err(err) => {
            collector.restore_unflushed(replay);
            Ok(FlushAttempt::Held(err))
        }
    }
}

fn unsaved_samples(n: usize, failures: u32, err: &anyhow::Error) -> anyhow::Error {
    anyhow::anyhow!("{n} samples were not saved after {failures} consecutive flush failures: {err}")
}

/// Sample until `duration` elapses, `stop` is set, or after one tick when
/// `once`. Publishes through `flush` every `flush_every` and once at the end,
/// so a stop request still flushes what was collected. A failed flush keeps
/// the batch and tries again next interval; the final flush retries briefly
/// before giving up.
pub fn watch<S: ProcessSource>(
    collector: &mut Collector<S>,
    interval: Duration,
    duration: Option<Duration>,
    once: bool,
    flush_every: Duration,
    stop: &AtomicBool,
    mut flush: impl FnMut(Batch) -> anyhow::Result<()>,
) -> anyhow::Result<WatchSummary> {
    let started = Instant::now();
    let mut last_flush = started;
    let mut consecutive_flush_failures = 0u32;
    let give_up = |collector: &Collector<S>, failures: u32| {
        failures >= MAX_CONSECUTIVE_FLUSH_FAILURES
            || collector.samples.len() > collector.sample_buffer_limit()
    };
    loop {
        let tick_started = Instant::now();
        collector.tick(Utc::now());
        if once || stop.load(Ordering::Relaxed) {
            break;
        }
        if last_flush.elapsed() >= flush_every {
            match publish(collector, &mut flush)? {
                FlushAttempt::Saved => consecutive_flush_failures = 0,
                FlushAttempt::Held(err) => {
                    consecutive_flush_failures += 1;
                    let unsaved = collector.samples.len();
                    tracing::warn!(
                        "process sample flush failed ({} in a row, {} samples kept): {}",
                        consecutive_flush_failures,
                        unsaved,
                        err
                    );
                    if give_up(collector, consecutive_flush_failures) {
                        return Err(unsaved_samples(unsaved, consecutive_flush_failures, &err));
                    }
                }
            }
            // Restart the interval on failure too, so the retry waits for it
            // instead of running again on the next tick.
            last_flush = Instant::now();
        }
        // Sleep in short steps so a stop request is noticed promptly.
        let next = tick_started + interval;
        while Instant::now() < next && !stop.load(Ordering::Relaxed) {
            std::thread::sleep(
                next.saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(100)),
            );
        }
        if stop.load(Ordering::Relaxed) || duration.is_some_and(|d| started.elapsed() >= d) {
            break;
        }
    }
    let mut warned = false;
    for attempt in 0..FINAL_FLUSH_ATTEMPTS {
        match publish(collector, &mut flush)? {
            FlushAttempt::Saved => break,
            FlushAttempt::Held(err) => {
                consecutive_flush_failures += 1;
                let unsaved = collector.samples.len();
                if !warned {
                    tracing::warn!(
                        "final process sample flush failed ({} samples kept); retrying: {}",
                        unsaved,
                        err
                    );
                    warned = true;
                }
                if give_up(collector, consecutive_flush_failures)
                    || attempt + 1 == FINAL_FLUSH_ATTEMPTS
                {
                    return Err(unsaved_samples(unsaved, consecutive_flush_failures, &err));
                }
                std::thread::sleep(FINAL_FLUSH_BACKOFF);
            }
        }
    }
    Ok(collector.summary())
}

/// The real source: `sysinfo` with only the fields needed (no command line,
/// no environment, no executable path; the working directory on request).
pub struct SysinfoSource {
    sys: sysinfo::System,
    me: Option<sysinfo::Uid>,
}

impl Default for SysinfoSource {
    fn default() -> Self {
        Self::new()
    }
}

impl SysinfoSource {
    pub fn new() -> Self {
        let mut source = Self {
            sys: sysinfo::System::new(),
            me: None,
        };
        source.refresh_structure();
        source.me = source
            .sys
            .process(sysinfo::Pid::from_u32(std::process::id()))
            .and_then(|p| p.user_id().cloned());
        source
    }

    fn refresh_structure(&mut self) {
        self.sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing()
                .with_user(sysinfo::UpdateKind::OnlyIfNotSet)
                .without_tasks(),
        );
    }
}

fn clean_name(name: &std::ffi::OsStr) -> String {
    redact(&name.to_string_lossy().chars().take(64).collect::<String>())
}

impl ProcessSource for SysinfoSource {
    fn scan(&mut self) -> Vec<RawProcess> {
        self.refresh_structure();
        self.sys
            .processes()
            .values()
            .map(|p| RawProcess {
                pid: p.pid().as_u32(),
                parent: p.parent().map(|pid| pid.as_u32()),
                start_s: i64::try_from(p.start_time()).unwrap_or(0),
                name: clean_name(p.name()),
                own: self.me.is_some() && p.user_id() == self.me.as_ref(),
            })
            .collect()
    }

    fn metrics(&mut self, pids: &[u32]) -> BTreeMap<u32, RawMetrics> {
        let list: Vec<sysinfo::Pid> = pids.iter().map(|p| sysinfo::Pid::from_u32(*p)).collect();
        self.sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&list),
            false,
            sysinfo::ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_disk_usage()
                .without_tasks(),
        );
        let mut out = BTreeMap::new();
        for (pid, id) in pids.iter().zip(&list) {
            let Some(p) = self.sys.process(*id) else {
                continue;
            };
            let mut m = RawMetrics::default();
            let rss = p.memory();
            if rss == 0 {
                // A live process always holds memory: zero means the OS gave
                // none of the readings (or the process is a zombie).
                m.limits
                    .insert(if p.status() == sysinfo::ProcessStatus::Zombie {
                        VisibilityLimit::ProcessExited
                    } else {
                        VisibilityLimit::PermissionDenied
                    });
            } else {
                m.rss_bytes = Some(rss);
                m.cpu_time_us = Some(p.accumulated_cpu_time().saturating_mul(1000));
                if disk_readable(*pid) {
                    let disk = p.disk_usage();
                    m.disk_read_bytes = Some(disk.total_read_bytes);
                    m.disk_write_bytes = Some(disk.total_written_bytes);
                } else {
                    m.limits.insert(VisibilityLimit::PermissionDenied);
                }
            }
            out.insert(*pid, m);
        }
        out
    }

    fn cwd(&mut self, pid: u32) -> Option<PathBuf> {
        let id = sysinfo::Pid::from_u32(pid);
        self.sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[id]),
            false,
            sysinfo::ProcessRefreshKind::nothing()
                .with_cwd(sysinfo::UpdateKind::OnlyIfNotSet)
                .without_tasks(),
        );
        self.sys.process(id)?.cwd().map(Path::to_path_buf)
    }
}

/// Linux hides `/proc/<pid>/io` of processes it will not let us read, and
/// sysinfo then reports zero: check first, so the zero is never stored.
fn disk_readable(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::fs::File::open(format!("/proc/{pid}/io")).is_ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        true
    }
}

#[cfg(test)]
mod tests;
