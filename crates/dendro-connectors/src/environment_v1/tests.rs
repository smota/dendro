use super::*;

const KEY: &[u8] = b"test-key";
const SECRET_CWD: &str = "/home/someone/secret-project-ZZ";

#[derive(Default)]
struct Fake {
    procs: Vec<RawProcess>,
    metrics: BTreeMap<u32, RawMetrics>,
    cwds: BTreeMap<u32, PathBuf>,
}

impl Fake {
    fn add(&mut self, pid: u32, parent: Option<u32>, start_s: i64, name: &str, own: bool) {
        self.procs.retain(|p| p.pid != pid);
        self.procs.push(RawProcess {
            pid,
            parent,
            start_s,
            name: name.into(),
            own,
        });
        self.metrics.insert(pid, healthy(1_000, 4096));
    }
    fn remove(&mut self, pid: u32) {
        self.procs.retain(|p| p.pid != pid);
        self.metrics.remove(&pid);
    }
}

impl ProcessSource for Fake {
    fn scan(&mut self) -> Vec<RawProcess> {
        self.procs.clone()
    }
    fn metrics(&mut self, pids: &[u32]) -> BTreeMap<u32, RawMetrics> {
        pids.iter()
            .filter_map(|p| self.metrics.get(p).map(|m| (*p, m.clone())))
            .collect()
    }
    fn cwd(&mut self, pid: u32) -> Option<PathBuf> {
        self.cwds.get(&pid).cloned()
    }
}

fn healthy(cpu_us: u64, rss: u64) -> RawMetrics {
    RawMetrics {
        cpu_time_us: Some(cpu_us),
        rss_bytes: Some(rss),
        disk_read_bytes: Some(10),
        disk_write_bytes: Some(20),
        limits: BTreeSet::new(),
    }
}

fn t(second: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000 + second, 0).unwrap()
}

fn collector(fake: Fake, config: Config) -> Collector<Fake> {
    Collector::new(fake, config, UNDETECTED)
}

fn tracked_pids(c: &Collector<Fake>) -> Vec<u32> {
    c.tracked.keys().copied().collect()
}

#[test]
fn tracks_own_harness_roots_and_their_descendants_only() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "claude.exe", true);
    fake.add(11, Some(10), 101, "node.exe", true);
    fake.add(12, Some(11), 102, "git.exe", true);
    fake.add(20, Some(1), 100, "notepad.exe", true);
    fake.add(21, Some(1), 100, "claude", false); // someone else's harness
    fake.add(22, Some(20), 101, "codex", true); // own harness under an untracked parent
    let mut c = collector(fake, Config::default());
    c.tick(t(1));
    assert_eq!(tracked_pids(&c), vec![10, 11, 12, 22]);
    let batch = c.drain();
    let root = batch.identities.iter().find(|i| i.pid == 10).unwrap();
    assert_eq!(root.harness.as_deref(), Some("claude.exe"));
    assert_eq!(root.attribution.as_deref(), Some("discovered"));
    let leaf = batch.identities.iter().find(|i| i.pid == 12).unwrap();
    assert_eq!(leaf.root_key, root.identity_key);
    assert_eq!(leaf.harness, None);
    assert_eq!(leaf.attribution, None);
    assert_eq!(
        batch.samples.len(),
        4,
        "everything adopted is sampled at once"
    );
}

#[test]
fn a_child_older_than_its_tracked_parent_is_a_reused_parent_pid_not_a_descendant() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "claude", true);
    // Parent pid 10 in this record belonged to a previous owner of that pid.
    fake.add(11, Some(10), 50, "stale-child", true);
    let mut c = collector(fake, Config::default());
    c.tick(t(1));
    assert_eq!(tracked_pids(&c), vec![10]);
}

#[test]
fn pid_reuse_ends_the_old_identity_and_never_continues_it() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "claude", true);
    let mut c = collector(fake, Config::default());
    c.tick(t(1));
    let first = c.drain().identities[0].identity_key.clone();
    // Same pid, later start time, a harness again: a new process.
    c.source.add(10, Some(1), 500, "claude", true);
    c.tick(t(2));
    let batch = c.drain();
    let old = batch
        .identities
        .iter()
        .find(|i| i.identity_key == first)
        .unwrap();
    assert_eq!(old.ended_at, Some(t(2)));
    assert!(old
        .visibility_limits
        .contains(&VisibilityLimit::ProcessExited));
    let new = batch
        .identities
        .iter()
        .find(|i| i.identity_key != first)
        .unwrap();
    assert_eq!(new.start_time_unix_s, 500);
    assert_eq!(new.ended_at, None);
    // Reused pid that is not a harness: the old one ends, nothing new starts.
    c.source.add(10, Some(1), 900, "calc", true);
    c.tick(t(3));
    assert!(tracked_pids(&c).is_empty());
}

#[test]
fn an_exited_pin_stores_the_file_name_and_not_the_path() {
    let mut c = collector(Fake::default(), Config::default());
    c.pin_launched(9, None);
    c.note_exited_pin(9, r"C:\secret\dir\cmd.exe");
    let batch = c.drain();
    assert_eq!(batch.identities.len(), 1);
    assert_eq!(batch.identities[0].exe_name, "cmd.exe");
    assert_eq!(batch.identities[0].attribution.as_deref(), Some("launched"));
    assert!(batch.identities[0].ended_at.is_some());
    assert!(batch.identities[0].cwd_fingerprint.is_none());
    assert!(!batch.identities[0].exe_name.contains("secret"));
    assert!(batch.identities[0].start_time_unix_s > 0);
}

#[test]
fn a_pinned_root_is_adopted_once_whatever_its_name() {
    let mut fake = Fake::default();
    fake.add(30, Some(1), 100, "python", false);
    let config = Config {
        pinned_roots: [30].into(),
        ..Config::default()
    };
    let mut c = collector(fake, config);
    c.tick(t(1));
    assert_eq!(tracked_pids(&c), vec![30]);
    let pinned = c.drain();
    assert_eq!(
        pinned.identities[0].attribution.as_deref(),
        Some("launched")
    );
    assert_eq!(pinned.identities[0].start_time_unix_s, 100);
    c.source.add(30, Some(1), 700, "python", false); // reused pid
    c.tick(t(2));
    assert!(tracked_pids(&c).is_empty(), "the pin was consumed");
}

#[test]
fn exit_is_recorded_as_a_limit_not_as_zero_samples() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "codex", true);
    let mut c = collector(fake, Config::default());
    c.tick(t(1));
    c.source.remove(10);
    c.tick(t(2));
    let batch = c.drain();
    assert_eq!(batch.samples.len(), 1, "no sample is invented after exit");
    let id = &batch.identities[0];
    assert_eq!(id.ended_at, Some(t(2)));
    assert!(id
        .visibility_limits
        .contains(&VisibilityLimit::ProcessExited));
    assert!(tracked_pids(&c).is_empty());
}

#[test]
fn vanishing_between_scan_and_reading_is_a_limit_with_null_values() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "codex", true);
    let mut c = collector(fake, Config::default());
    c.source.metrics.remove(&10);
    c.tick(t(1));
    let sample = &c.drain().samples[0];
    assert_eq!(sample.cpu_time_us, None);
    assert_eq!(sample.rss_bytes, None);
    assert!(sample
        .visibility_limits
        .contains(&VisibilityLimit::ProcessExited));
}

#[test]
fn unreadable_values_stay_null_with_the_limit_the_source_reported() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "claude", true);
    fake.metrics.insert(
        10,
        RawMetrics {
            limits: [VisibilityLimit::PermissionDenied].into(),
            ..RawMetrics::default()
        },
    );
    let mut c = collector(fake, Config::default());
    c.tick(t(1));
    let sample = &c.drain().samples[0];
    assert_eq!(sample.cpu_time_us, None);
    assert_eq!(sample.rss_bytes, None);
    assert_eq!(sample.disk_read_bytes, None);
    assert_eq!(sample.disk_write_bytes, None);
    assert!(sample
        .visibility_limits
        .contains(&VisibilityLimit::PermissionDenied));
}

#[test]
fn a_falling_counter_is_flagged_not_trusted() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "claude", true);
    let mut c = collector(fake, Config::default());
    c.tick(t(1));
    c.source.metrics.insert(10, healthy(500, 4096));
    c.tick(t(2));
    let samples = c.drain().samples;
    assert!(samples[0].visibility_limits.is_empty());
    assert!(samples[1]
        .visibility_limits
        .contains(&VisibilityLimit::CounterReset));
}

#[test]
fn samples_per_process_are_bounded_and_the_cut_is_recorded() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "claude", true);
    let config = Config {
        max_samples: 3,
        ..Config::default()
    };
    let mut c = collector(fake, config);
    for second in 1..=6 {
        c.tick(t(second));
    }
    let batch = c.drain();
    assert_eq!(batch.samples.len(), 3);
    assert_eq!(
        batch.samples.iter().map(|s| s.seq).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert!(batch.identities[0]
        .visibility_limits
        .contains(&VisibilityLimit::Truncated));
}

#[test]
fn tracked_processes_are_bounded_and_the_refusals_are_counted_once() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "claude", true);
    for pid in 11..16 {
        fake.add(pid, Some(10), 101, "worker", true);
    }
    let config = Config {
        max_tracked: 3,
        ..Config::default()
    };
    let mut c = collector(fake, config);
    c.tick(t(1));
    c.tick(t(2));
    assert_eq!(c.tracked.len(), 3);
    assert_eq!(
        c.summary().refused_by_cap,
        3,
        "same refused process is not recounted"
    );
    let batch = c.drain();
    let root = batch.identities.iter().find(|i| i.pid == 10).unwrap();
    assert!(root.visibility_limits.contains(&VisibilityLimit::Truncated));
}

#[test]
fn a_descendant_whose_start_time_is_unreadable_is_not_adopted_and_marks_its_tree() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "claude", true);
    fake.add(11, Some(10), 0, "protected", true);
    let mut c = collector(fake, Config::default());
    c.tick(t(1));
    assert_eq!(tracked_pids(&c), vec![10]);
    let root = &c.drain().identities[0];
    assert!(root
        .visibility_limits
        .contains(&VisibilityLimit::PermissionDenied));
}

#[test]
fn nothing_stored_carries_a_path_or_arguments() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "claude", true);
    fake.cwds.insert(10, PathBuf::from(SECRET_CWD));
    let mut c = collector(fake, Config::default());
    c.tick(t(1));
    let batch = c.drain().seal(KEY, &[]);
    let text = format!("{batch:?}");
    assert!(!text.contains("secret-project"), "{text}");
    assert!(!text.contains("someone"), "{text}");
    let fp = batch.identities[0].cwd_fingerprint.clone().unwrap();
    assert_eq!(fp, cwd_fingerprint(KEY, Path::new(SECRET_CWD)));
    assert_eq!(fp.len(), 64);
    assert_ne!(fp, cwd_fingerprint(b"other-key", Path::new(SECRET_CWD)));
}

#[test]
fn a_root_links_to_a_session_only_on_matching_fingerprint_and_start_window() {
    let fp = cwd_fingerprint(KEY, Path::new(SECRET_CWD));
    let session = |name: &str, first: i64, fp: Option<&str>| SessionCandidate {
        native_session: name.into(),
        first_ts_unix_s: first,
        cwd_fingerprint: fp.map(str::to_string),
    };
    let start = 1_000;
    let good = session("good", start + 5, Some(&fp));
    assert_eq!(
        link_session(start, Some(&fp), std::slice::from_ref(&good)).as_deref(),
        Some("good")
    );
    // Right window, no recorded directory: no evidence.
    assert_eq!(
        link_session(start, Some(&fp), &[session("a", start + 5, None)]),
        None
    );
    // Right directory, outside the window (before start, or long after).
    assert_eq!(
        link_session(start, Some(&fp), &[session("b", start - 30, Some(&fp))]),
        None
    );
    assert_eq!(
        link_session(start, Some(&fp), &[session("c", start + 3_600, Some(&fp))]),
        None
    );
    // Two sessions fit: ambiguous, so unlinked.
    assert_eq!(
        link_session(
            start,
            Some(&fp),
            &[good.clone(), session("other", start + 9, Some(&fp))]
        ),
        None
    );
    // Root without a readable directory never links.
    assert_eq!(link_session(start, None, &[good]), None);
}

#[test]
fn sealed_roots_link_only_with_session_evidence() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "claude", true);
    fake.cwds.insert(10, PathBuf::from(SECRET_CWD));
    let mut c = collector(fake, Config::default());
    c.tick(t(1));
    let fp = cwd_fingerprint(KEY, Path::new(SECRET_CWD));
    let session = |cwd: Option<&str>| SessionCandidate {
        native_session: "s1".into(),
        first_ts_unix_s: 101,
        cwd_fingerprint: cwd.map(str::to_string),
    };
    let sealed = c.drain().seal(KEY, &[session(None)]);
    assert_eq!(sealed.identities[0].session_link, None);
    c.tick(t(2));
    let sealed = c.drain().seal(KEY, &[session(Some(&fp))]);
    assert_eq!(sealed.identities[0].session_link.as_deref(), Some("s1"));
    assert_eq!(
        sealed.identities[0].cwd_fingerprint.as_deref(),
        Some(fp.as_str())
    );
}

#[test]
fn isolation_needs_a_certain_marker() {
    let kind = |d, v, g| classify_linux(d, v, g);
    assert_eq!(kind(true, "", "").hint, "docker");
    assert_eq!(kind(false, "", "12:cpu:/kubepods/x").hint, "kubernetes");
    assert_eq!(
        kind(false, "", "1:name=systemd:/docker/abc").kind,
        EnvironmentKind::Container
    );
    assert_eq!(
        kind(false, "Linux version 5.15 microsoft-standard-WSL2", "0::/"),
        Isolation {
            kind: EnvironmentKind::Vm,
            hint: "wsl"
        }
    );
    assert_eq!(kind(false, "Linux version 6.1 generic", "0::/"), UNDETECTED);
    assert_eq!(UNDETECTED.kind, EnvironmentKind::Unknown);
}

#[test]
fn watch_flushes_on_stop_and_once_takes_a_single_tick() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "claude", true);
    let mut c = collector(fake, Config::default());
    let stop = AtomicBool::new(false);
    let mut flushed = Vec::new();
    let summary = watch(
        &mut c,
        Duration::from_millis(1),
        None,
        true,
        Duration::from_secs(60),
        &stop,
        |batch| {
            flushed.push((batch.identities.len(), batch.samples.len()));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(flushed, vec![(1, 1)]);
    assert_eq!(summary.ticks, 1);
    assert_eq!((summary.identities, summary.samples), (1, 1));

    // A stop request ends a long watch and still publishes what it held.
    let stop = AtomicBool::new(true);
    let mut flushed = Vec::new();
    watch(
        &mut c,
        Duration::from_secs(3_600),
        None,
        false,
        Duration::from_secs(3_600),
        &stop,
        |batch| {
            flushed.push(batch.samples.len());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(flushed, vec![1]);
}

#[test]
fn a_flush_that_fails_twice_then_succeeds_keeps_every_sample_once() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "claude", true);
    let mut c = collector(fake, Config::default());
    let stop = AtomicBool::new(false);
    let mut calls = 0;
    let mut saved = Vec::new();
    let summary = watch(
        &mut c,
        Duration::ZERO,
        Some(Duration::from_secs(2)),
        false,
        Duration::ZERO,
        &stop,
        |batch| {
            calls += 1;
            if calls <= 2 {
                return Err(anyhow::anyhow!("database is locked"));
            }
            saved.extend(batch.samples.iter().map(|s| s.seq));
            stop.store(true, Ordering::Relaxed);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(saved, vec![0, 1, 2]);
    assert_eq!(summary.samples, saved.len());
    assert!(
        calls < 6,
        "flush retried instead of spinning, calls={calls}"
    );

    // Stop / once / the end of the watch retries the final flush in place.
    let mut c = collector(
        {
            let mut fake = Fake::default();
            fake.add(10, Some(1), 100, "claude", true);
            fake
        },
        Config::default(),
    );
    let mut calls = 0;
    let mut saved = Vec::new();
    let summary = watch(
        &mut c,
        Duration::from_secs(60),
        None,
        true,
        Duration::from_secs(60),
        &AtomicBool::new(false),
        |batch| {
            calls += 1;
            if calls <= 2 {
                return Err(anyhow::anyhow!("disk full"));
            }
            saved.extend(batch.samples.iter().map(|s| s.seq));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(calls, 3);
    assert_eq!(saved, vec![0]);
    assert_eq!(summary.samples, 1);
}

#[test]
fn an_unreadable_counter_is_not_a_reset() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "claude", true);
    let mut c = collector(fake, Config::default());
    c.tick(t(1));
    c.source.metrics.insert(
        10,
        RawMetrics {
            cpu_time_us: Some(1_000),
            rss_bytes: Some(4096),
            limits: [VisibilityLimit::PermissionDenied].into(),
            ..RawMetrics::default()
        },
    );
    c.tick(t(2));
    c.source.metrics.insert(
        10,
        RawMetrics {
            cpu_time_us: Some(1_000),
            rss_bytes: Some(4096),
            disk_read_bytes: Some(5),
            disk_write_bytes: Some(20),
            ..RawMetrics::default()
        },
    );
    c.tick(t(3));
    let samples = c.drain().samples;
    assert!(samples[1]
        .visibility_limits
        .contains(&VisibilityLimit::PermissionDenied));
    assert!(
        !samples[1]
            .visibility_limits
            .contains(&VisibilityLimit::CounterReset),
        "a counter that became unreadable is not a reset"
    );
    assert_eq!(samples[1].disk_read_bytes, None);
    assert!(samples[2]
        .visibility_limits
        .contains(&VisibilityLimit::CounterReset));
}

#[test]
fn ended_roots_drop_cwd_and_exited_pids_leave_refused() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "claude", true);
    fake.cwds.insert(10, PathBuf::from(SECRET_CWD));
    fake.add(11, Some(10), 101, "worker", true);
    let mut c = collector(
        fake,
        Config {
            max_tracked: 1,
            ..Config::default()
        },
    );
    c.tick(t(1));
    assert_eq!(c.cwds.len(), 1);
    assert!(c.refused.contains(&(11, 101)));
    c.source.remove(10);
    c.source.remove(11);
    c.tick(t(2));
    assert!(
        c.refused.is_empty(),
        "a pid that has exited is not kept in the refusal set"
    );
    let batch = c.drain();
    assert!(
        c.cwds.is_empty(),
        "an ended root does not keep its working directory"
    );
    let root = batch.identities.iter().find(|i| i.pid == 10).unwrap();
    assert_eq!(root.ended_at, Some(t(2)));
    assert_eq!(
        batch.cwds.get(&root.identity_key),
        Some(&PathBuf::from(SECRET_CWD))
    );
}

#[test]
fn a_transient_unreadable_start_is_not_pid_reuse() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "claude", true);
    let mut c = collector(fake, Config::default());
    c.tick(t(1));
    let key = c.tracked.get(&10).unwrap().fact.identity_key.clone();
    c.drain();
    c.source.add(10, Some(1), 0, "claude", true);
    c.tick(t(2));
    assert_eq!(tracked_pids(&c), vec![10]);
    let fact = &c.tracked.get(&10).unwrap().fact;
    assert_eq!(fact.identity_key, key);
    assert_eq!(fact.start_time_unix_s, 100);
    assert_eq!(fact.ended_at, None);
    assert!(c
        .dirty
        .values()
        .all(|f| f.ended_at.is_none() && f.identity_key == key));
}

#[test]
fn a_pinned_root_claims_the_tracking_bound_before_name_matches() {
    let mut fake = Fake::default();
    fake.add(10, Some(1), 100, "claude", true); // older harness match
    fake.add(30, Some(1), 200, "python", false);
    let config = Config {
        max_tracked: 1,
        pinned_roots: [30].into(),
        ..Config::default()
    };
    let mut c = collector(fake, config);
    c.tick(t(1));
    assert_eq!(tracked_pids(&c), vec![30]);
    assert_eq!(c.summary().refused_by_cap, 1);
}
