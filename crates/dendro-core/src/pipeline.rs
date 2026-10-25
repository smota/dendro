use std::fs;
#[cfg(not(any(windows, target_os = "linux")))]
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::facts::{SeatBound, SkillFact};
use crate::lifecycle;
use crate::metrics::{self, LoopThresholds, SessionBinding, Window};
use crate::output::refuse_aliased_output_with_openrig;
#[cfg(not(any(windows, target_os = "linux")))]
use crate::output::{self, load_or_create_key, with_output_lock};
use crate::privacy::{contains_secret, opaque_id, redact};
use crate::storage::AnalyticsStore;

pub struct RunRequest<'a> {
    pub db: &'a Path,
    pub source_roots: &'a [PathBuf],
}

pub fn run_locked<F>(db: &Path, source_roots: &[PathBuf], ingest: F) -> Result<Value>
where
    F: FnOnce(&Path, &[u8]) -> Result<Vec<String>>,
{
    run_locked_with_openrig(db, source_roots, &[], ingest)
}

/// Ingest, refusing a second distinct OpenRig root before any source read or
/// analytics write. `openrig_roots` are the OpenRig paths only; `source_roots`
/// remains every harness path used for alias and privacy checks.
pub fn run_locked_with_openrig<F>(
    db: &Path,
    source_roots: &[PathBuf],
    openrig_roots: &[PathBuf],
    ingest: F,
) -> Result<Value>
where
    F: FnOnce(&Path, &[u8]) -> Result<Vec<String>>,
{
    if source_roots.is_empty() {
        return Err(anyhow!("at least one source root is required"));
    }
    // S31-1: two different OpenRig files in one run never create a database.
    refuse_multiple_openrig_roots(openrig_roots)?;
    refuse_secret_path(db)?;
    for root in source_roots.iter().chain(openrig_roots.iter()) {
        refuse_secret_path(root)?;
    }
    // Source identity must be established before mkdir, locking, SQLite
    // preflight, key lookup or any other output/source-content effect.
    refuse_aliased_output_with_openrig(db, source_roots, openrig_roots)?;
    #[cfg(any(windows, target_os = "linux"))]
    {
        crate::selection_binding::run_locked_with_selection(
            db,
            &crate::platform_v1::StorageRootChoice::LocalAppData,
            source_roots,
            openrig_roots,
            ingest,
        )
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    let parent = db
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        fs::create_dir_all(parent)?;
        let lock_path = parent.join(".dendro-output.lock");
        with_output_lock(&lock_path, || run_inner(db, openrig_roots, ingest))
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn run_inner<F>(db: &Path, openrig_roots: &[PathBuf], ingest: F) -> Result<Value>
where
    F: FnOnce(&Path, &[u8]) -> Result<Vec<String>>,
{
    // S31-2 / S31-5: an existing database is judged before the key sidecar,
    // the private stage, or any source read.
    if db.exists() {
        refuse_bound_openrig_root(db, openrig_roots)?;
    }
    output::refuse_existing_output_wal(db)?;
    let before = output_identity_if_present(db)?;

    let key_path = PathBuf::from(format!("{}.key", db.display()));
    let creating = !db.exists();
    let key = load_or_create_key(&key_path, creating || key_path.exists())?;
    if db.exists() {
        let preflight = AnalyticsStore::open_read_only(db)?;
        if preflight.key_id != key.id {
            return Err(anyhow!("analytics key id does not match the sidecar"));
        }
    }
    let parent = db
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // Private stage first (marker before any copy), then stale recovery under
    // the same lock, then work. The stage is removed explicitly; Drop is only
    // a fallback.
    let stage = lifecycle::Stage::create(parent)?;
    let recovery = lifecycle::recover_stale(&stage);
    let staged = stage.path().join("analytics.duckdb");
    let published = (|| {
        let recovered = recovery?;
        let cancel = &lifecycle::RUN_CANCEL;
        if db.exists() {
            lifecycle::private_copy(db, &staged, cancel)?;
        } else {
            let created = AnalyticsStore::open_path(&staged, &key.id)?;
            created.checkpoint()?;
        }
        let notes = ingest(&staged, &key.bytes)?;
        cancel.check()?;
        let staged_store = AnalyticsStore::open_staged(&staged)?;
        staged_store.checkpoint()?;
        drop(staged_store);
        sync_staged_output(&stage)?;
        cancel.check()?;
        stage.check_anchor()?;
        let after = output_identity_if_present(db)?;
        if let (Some(left), Some(right)) = (&before, &after) {
            if left != right {
                return Err(anyhow!("analytics output identity changed during ingest"));
            }
        }
        if before.is_none() && after.is_some() {
            return Err(anyhow!("analytics output appeared during ingest"));
        }
        if before.is_some() && after.is_none() {
            return Err(anyhow!("analytics output disappeared during ingest"));
        }
        // Publication: after this rename the output is the new complete db.
        publish_staged_output(&staged, db)?;
        if let Some(parent) = db.parent() {
            output::sync_dir(parent)?;
        }
        Ok((notes, recovered))
    })();
    let cleanup = stage.finish();
    let (notes, recovered) = match (published, cleanup) {
        (Ok(done), Ok(())) => done,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(cleanup)) => return Err(cleanup),
        (Err(error), Err(cleanup)) => return Err(error.context(cleanup.to_string())),
    };
    Ok(json!({
        "status": "ready",
        "product": "AI Fleet Intelligence",
        "key_id": key.id,
        "sources": notes,
        "staging": recovered,
    }))
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn sync_staged_output(stage: &lifecycle::Stage) -> Result<()> {
    File::open(stage.path().join("analytics.duckdb"))?.sync_all()?;
    Ok(())
}

#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn sync_staged_output(stage: &lifecycle::Stage) -> Result<()> {
    use crate::platform_v1 as platform;
    stage.check_anchor()?;
    let root = platform::open_existing_private_root(stage.path())?;
    let name = platform::parse_child_name("analytics.duckdb", platform::current_native_platform())
        .map_err(platform::PlatformError::Rejected)?;
    // Admission checks the private regular file before a verified handle is
    // reopened with write access, which Windows FlushFileBuffers requires.
    let file = platform::open_existing_private_file(&root, &name)?;
    file.file().sync_all()?;
    root.check_anchor()?;
    stage.check_anchor()?;
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn publish_staged_output(staged: &Path, db: &Path) -> Result<()> {
    fs::rename(staged, db)?;
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn output_identity_if_present(path: &Path) -> Result<Option<crate::platform_v1::FileIdentity>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    if meta.file_type().is_symlink() {
        return Err(anyhow!("refusing symlink analytics output"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(Some(crate::platform_v1::FileIdentity::Unix {
            device: meta.dev(),
            inode: meta.ino(),
        }))
    }
}

pub fn trace_tokens(store: &AnalyticsStore, window: Window) -> Result<Value> {
    let usage = store.load_usage()?;
    let (input, output, cache_read, cache_write) = metrics::token_totals(
        &usage
            .into_iter()
            .filter(|u| match u.ts {
                Some(ts) => ts >= window.from && ts < window.to,
                None => false,
            })
            .collect::<Vec<_>>(),
    );
    Ok(metrics::report_value(
        "tokens",
        json!({
            "input_uncached": input,
            "output": output,
            "cache_read": cache_read,
            "cache_write": cache_write,
            "from": window.from.to_rfc3339(),
            "to": window.to.to_rfc3339(),
        }),
    ))
}

/// AC6 / §4.1: reasoning is a subset of output. It is shown apart and never
/// added to the four categories again; Claude does not record it.
pub const REASONING_RULE: &str =
    "reasoning is part of output, shown separately and never added again; claude does not record it";

/// The four token categories of a set of usage records, plus reasoning.
#[derive(Debug, Clone, Default)]
struct Categories {
    input_uncached: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    reasoning: i64,
    reasoning_recorded: bool,
}

impl Categories {
    fn tokens_of(row: &crate::UsageFact) -> i64 {
        row.input_uncached + row.output + row.cache_read + row.cache_write
    }

    fn add(&mut self, row: &crate::UsageFact) {
        self.input_uncached += row.input_uncached;
        self.output += row.output;
        self.cache_read += row.cache_read;
        self.cache_write += row.cache_write;
        if row.harness != "claude" {
            self.reasoning += row.reasoning;
            self.reasoning_recorded = true;
        }
    }

    fn tokens(&self) -> i64 {
        self.input_uncached + self.output + self.cache_read + self.cache_write
    }

    fn sum_tokens(parts: &[&Categories]) -> i64 {
        parts.iter().map(|part| part.tokens()).sum()
    }

    /// `harness` names a single session's harness; Claude shows "not recorded".
    fn json(&self, harness: Option<&str>) -> Value {
        let recorded = match harness {
            Some(name) => name != "claude",
            None => self.reasoning_recorded,
        };
        json!({
            "input_uncached": self.input_uncached,
            "output": self.output,
            "cache_read": self.cache_read,
            "cache_write": self.cache_write,
            "tokens": self.tokens(),
            "reasoning": if recorded { json!(self.reasoning) } else { json!("not recorded") },
        })
    }
}

fn cost_json(rollup: crate::accounting_v1::CostRollup) -> Value {
    json!({
        "status": match rollup.status {
            crate::accounting_v1::CostStatus::Complete => "complete",
            crate::accounting_v1::CostStatus::Partial => "partial",
            crate::accounting_v1::CostStatus::Unknown => "unknown",
        },
        "usd": rollup.total_usd(),
        "unknown_rows": rollup.unknown_rows,
        "known_rows": rollup.known_rows,
    })
}

fn usage_slice(row: &crate::UsageFact, model: Option<&str>) -> crate::accounting_v1::UsageSlice {
    crate::accounting_v1::UsageSlice {
        source_key: row.source_key.clone(),
        harness: row.harness.clone(),
        session: row.native_session.clone(),
        input_uncached: row.input_uncached,
        output: row.output,
        cache_read: row.cache_read,
        cache_write: row.cache_write,
        reasoning: row.reasoning,
        observed_usd: row.currency_total.filter(|amount| amount.is_finite()),
        estimated_usd: crate::accounting_v1::estimate_usd(
            model,
            row.input_uncached,
            row.output,
            row.cache_read,
            row.cache_write,
        ),
    }
}

pub fn opaque_session(key: &[u8], harness: &str, native: &str) -> String {
    opaque_id(key, "session", &format!("{harness}\n{native}"))
}

/// Filesystem identity when the path exists (symlinks included); otherwise
/// a lexical absolute path with `.` and `..` removed.
pub fn canonical_root(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// S31-1. Distinct files only; two spellings of one file are not a conflict.
pub fn refuse_multiple_openrig_roots(paths: &[PathBuf]) -> Result<()> {
    if distinct_canonical_roots(paths).len() > 1 {
        return Err(anyhow!(
            "v0.1.0 supports exactly one OpenRig root per analytics database"
        ));
    }
    Ok(())
}

fn distinct_canonical_roots(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut distinct = Vec::new();
    for path in paths {
        let canonical = canonical_root(path);
        if !distinct.iter().any(|seen| seen == &canonical) {
            distinct.push(canonical);
        }
    }
    distinct
}

struct BoundOpenRig {
    display: String,
    canonical: PathBuf,
}

fn bound_openrig_roots(db: &Path) -> Result<Vec<BoundOpenRig>> {
    if !db.exists() {
        return Ok(Vec::new());
    }
    let store = AnalyticsStore::open_read_only(db)?;
    let mut bound = Vec::new();
    for (_id, harness, display_path, _generation, _session) in store.load_sources()? {
        if harness != "openrig" {
            continue;
        }
        let canonical = canonical_root(Path::new(&display_path));
        if bound
            .iter()
            .any(|row: &BoundOpenRig| row.canonical == canonical)
        {
            continue;
        }
        bound.push(BoundOpenRig {
            display: display_path,
            canonical,
        });
    }
    Ok(bound)
}

/// Path stored for an OpenRig root. Always absolute, so a later run from
/// another working directory still names the same file (S31-2 / S31-3).
pub fn absolute_openrig_root(path: &Path) -> PathBuf {
    canonical_root(path)
}

/// When `incoming` is the bound file under another spelling, return the
/// absolute path used for source keys (S31-3). A relative stored spelling is
/// replaced by that absolute path so the binding does not follow cwd.
pub fn align_openrig_spelling(db: &Path, incoming: &Path) -> Result<PathBuf> {
    let incoming_abs = absolute_openrig_root(incoming);
    let request = crate::presentation::ReadRequest::open(db)?;
    let bound = bound_openrig_roots(request.database())?;
    if bound.len() == 1 && bound[0].canonical == incoming_abs {
        let stored = PathBuf::from(&bound[0].display);
        if stored.is_absolute() && absolute_openrig_root(&stored) == incoming_abs {
            return Ok(stored);
        }
        return Ok(incoming_abs);
    }
    Ok(incoming_abs)
}

pub(crate) fn refuse_bound_openrig_root(db: &Path, incoming: &[PathBuf]) -> Result<()> {
    let bound = bound_openrig_roots(db)?;
    if bound.len() > 1 {
        return Err(anyhow!(
            "refusing second OpenRig root: analytics database is bound to OpenRig root {} and {}",
            redact(&bound[0].display),
            redact(&bound[1].display)
        ));
    }
    if let Some(bound) = bound.first() {
        for path in incoming {
            if canonical_root(path) != bound.canonical {
                return Err(anyhow!(
                    "refusing second OpenRig root: analytics database is bound to OpenRig root {}; refused {}; use a fresh analytics database for this root",
                    redact(&bound.display),
                    redact(&path.display().to_string())
                ));
            }
        }
    }
    Ok(())
}

pub fn refuse_secret_path(path: &Path) -> Result<()> {
    let shown = path.display().to_string();
    if contains_secret(&shown) {
        return Err(anyhow!(
            "refusing a path that contains a secret-shaped value"
        ));
    }
    Ok(())
}

struct ReportEvaluation {
    report: Value,
    loop_trace: Value,
    file_states: Option<crate::file_state::Captured>,
    friction_replay: Value,
}

pub fn report_json(
    db: &Path,
    kind: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
) -> Result<Value> {
    report_json_with_thresholds(db, kind, rig, from, to, LoopThresholds::default())
}

pub fn report_json_with_thresholds(
    db: &Path,
    kind: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
    thresholds: LoopThresholds,
) -> Result<Value> {
    report_json_with_options(
        db,
        kind,
        rig,
        from,
        to,
        thresholds,
        crate::FrictionThresholds::default(),
    )
}

/// Additive friction settings; legacy loop settings remain independent.
#[allow(clippy::too_many_arguments)]
pub fn report_json_with_options(
    db: &Path,
    kind: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
    thresholds: LoopThresholds,
    friction: crate::FrictionThresholds,
) -> Result<Value> {
    report_json_with_analysis(
        db,
        kind,
        rig,
        from,
        to,
        thresholds,
        friction,
        crate::advisory::AdvisoryOptions::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn report_json_with_analysis(
    db: &Path,
    kind: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
    thresholds: LoopThresholds,
    friction: crate::FrictionThresholds,
    advisory: crate::advisory::AdvisoryOptions,
) -> Result<Value> {
    #[cfg(any(windows, target_os = "linux"))]
    {
        use crate::selection_binding::{resolve_logical_db, SelectedDb};
        match resolve_logical_db(db, &crate::platform_v1::StorageRootChoice::LocalAppData)? {
            SelectedDb::Selected(selected) => report_json_with_analysis_physical(
                &selected.database,
                kind,
                rig,
                from,
                to,
                thresholds,
                friction,
                advisory,
            ),
            SelectedDb::Legacy(legacy) => report_json_with_analysis_physical(
                &legacy.database,
                kind,
                rig,
                from,
                to,
                thresholds,
                friction,
                advisory,
            ),
            SelectedDb::Uninitialized => Err(anyhow!("analytics database is uninitialized")),
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        report_json_with_analysis_physical(db, kind, rig, from, to, thresholds, friction, advisory)
    }
}

/// Physical read for callers that retain the selected version's custody for
/// the complete composite request. It does not resolve a logical DB leaf.
#[allow(clippy::too_many_arguments)]
pub fn report_json_with_analysis_physical(
    db: &Path,
    kind: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
    thresholds: LoopThresholds,
    friction: crate::FrictionThresholds,
    advisory: crate::advisory::AdvisoryOptions,
) -> Result<Value> {
    Ok(evaluate_report(
        db, kind, rig, from, to, false, thresholds, friction, &advisory, None,
    )?
    .report)
}

pub fn report_json_physical(
    db: &Path,
    kind: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
) -> Result<Value> {
    report_json_with_analysis_physical(
        db,
        kind,
        rig,
        from,
        to,
        LoopThresholds::default(),
        crate::FrictionThresholds::default(),
        crate::advisory::AdvisoryOptions::default(),
    )
}

#[allow(clippy::too_many_arguments)]
fn evaluate_report(
    db: &Path,
    kind: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
    evidence: bool,
    thresholds: LoopThresholds,
    friction: crate::FrictionThresholds,
    advisory: &crate::advisory::AdvisoryOptions,
    requested_metric: Option<&str>,
) -> Result<ReportEvaluation> {
    advisory.validate()?;
    let friction = friction.validate()?;
    refuse_secret_path(db)?;
    let window = parse_window(from, to)?;
    if thresholds.tool_loop_failures == 0 {
        return Err(anyhow!("tool-loop-failures must be at least 1"));
    }
    if thresholds.tool_loop_window_secs < 0 {
        return Err(anyhow!("tool-loop-window-secs must be non-negative"));
    }
    if chrono::Duration::try_seconds(thresholds.tool_loop_window_secs).is_none() {
        return Err(anyhow!("tool-loop-window-secs is out of bounds"));
    }
    if !matches!(
        kind,
        "links"
            | "activity"
            | "handoffs"
            | "tokens"
            | "decisions"
            | "advisory"
            | "anomalies"
            | "spotlights"
    ) {
        return Err(anyhow!("unknown report kind {}", redact(kind)));
    }
    let store = AnalyticsStore::open_read_only(db)?;
    let mut evaluation = store.read_snapshot(|store| {
        evaluate_report_snapshot(
            store,
            kind,
            rig,
            from,
            to,
            window,
            evidence,
            thresholds,
            friction,
            advisory,
            requested_metric,
        )
    })?;
    for (field, metric) in [
        ("advisory_idle_claimed", "advisory_idle_claimed"),
        ("anomaly_spotlights", "anomaly_spotlights"),
    ] {
        if let Some(value) = evaluation.report["payload"].get_mut(field) {
            crate::spotlight::attach_drilldown(
                value,
                &db.display().to_string(),
                rig,
                from,
                to,
                metric,
            );
        }
    }
    // No analytics connection survives into the trace filesystem phase.
    drop(store);
    Ok(evaluation)
}

#[allow(clippy::too_many_arguments)]
fn evaluate_report_snapshot(
    store: &AnalyticsStore,
    kind: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
    window: Window,
    evidence: bool,
    thresholds: LoopThresholds,
    friction: crate::FrictionThresholds,
    advisory: &crate::advisory::AdvisoryOptions,
    requested_metric: Option<&str>,
) -> Result<ReportEvaluation> {
    let identities = store.load_identity()?;
    // One binding rule for links, tokens, missions, intakes and rig scope.
    let bindings = metrics::session_bindings(&identities);
    let binding = |harness: &str, native: &str| metrics::binding_of(&bindings, harness, native);
    // Rig scope admits only sessions singly linked to that rig; a collision
    // never enters through one of its candidate identities.
    let allowed: Option<std::collections::HashSet<(String, String)>> = rig.map(|name| {
        bindings
            .iter()
            .filter(|(_, bound)| bound.rig() == Some(name))
            .map(|(session, _)| session.clone())
            .collect()
    });
    let in_scope = |harness: &str, native: &str| match &allowed {
        Some(set) => set.contains(&(harness.to_string(), native.to_string())),
        None => true,
    };
    let rig_seats: Option<std::collections::HashSet<String>> = rig.map(|name| {
        identities
            .iter()
            .filter(|fact| fact.rig == name)
            .map(|fact| fact.seat.clone())
            .collect()
    });
    let sources = store.load_sources()?;
    let destination_snapshots = store.load_destination_snapshots()?;
    // AC12 provenance: the original OpenRig path and the source revision the
    // run's coherent private copy was accepted at.
    let mut openrig_sources = Vec::new();
    for (source_id, harness, path, generation, _) in &sources {
        if harness == "openrig" {
            if let Some((_, bytes, revision)) = store.source_state(source_id)? {
                openrig_sources.push(json!({
                    "source_id": source_id,
                    "generation": (*generation).max(1),
                    "path": path,
                    "revision": revision,
                    "bytes": bytes,
                }));
            }
        }
    }
    let usage_all = store.load_usage()?;
    let untimestamped: Vec<_> = usage_all
        .iter()
        .filter(|row| row.ts.is_none() && in_scope(&row.harness, &row.native_session))
        .collect();
    let untimestamped_count = untimestamped.len();
    let usage: Vec<_> = usage_all
        .iter()
        .filter(|row| match row.ts {
            Some(ts) => ts >= window.from && ts < window.to,
            None => false,
        })
        .filter(|row| in_scope(&row.harness, &row.native_session))
        .cloned()
        .collect();
    if matches!(
        kind,
        "handoffs" | "decisions" | "advisory" | "anomalies" | "spotlights"
    ) {
        crate::vortex::validate_persisted_usage(&usage)?;
    }
    let queue_context = store.load_queue()?;
    let queue_all: Vec<_> = queue_context
        .iter()
        .filter(|row| match &rig_seats {
            Some(seats) => seats.contains(&row.source_seat) || seats.contains(&row.dest_seat),
            None => true,
        })
        .cloned()
        .collect();
    let edges_all = metrics::handoff_edges(&queue_all);
    let edges: Vec<_> = edges_all
        .iter()
        .filter(|edge| {
            queue_all.iter().any(|row| {
                row.source_key == edge.source_key && row.ts >= window.from && row.ts < window.to
            })
        })
        .cloned()
        .collect();
    let roles = metrics::seat_roles(&store.load_seat_roles()?);
    let m3 = metrics::m3_rework(&edges, &roles);
    let rework = m3.rework.len() as i64;
    let chains = m3.chains;
    let paths: Vec<_> = m3.rework.iter().map(|item| item.path.clone()).collect();
    let rework_members = &m3.rework;
    let m3_unknown = &m3.unknown;
    let m3_proportion = if chains == 0 {
        serde_json::Value::String("no data".into())
    } else {
        json!(rework as f64 / chains as f64)
    };
    let handoffs = edges.len() as i64;
    let returns = edges.iter().filter(|edge| edge.is_return).count() as i64;
    let (input, output, cache_read, cache_write) = metrics::token_totals(&usage);
    let tokens = input + output + cache_read + cache_write;
    let nonreturn = edges
        .iter()
        .filter(|edge| !edge.returned_within(window))
        .count() as i64;
    let mut seat_tokens = std::collections::BTreeMap::<String, i64>::new();
    // Per seat and overall four-category period totals; reasoning is part of
    // output and is shown apart, never added again.
    let mut seat_categories = std::collections::BTreeMap::<(String, String), Categories>::new();
    let mut overall = Categories::default();
    let mut unlinked = Categories::default();
    let mut collision = Categories::default();
    for row in &usage {
        overall.add(row);
        match binding(&row.harness, &row.native_session) {
            metrics::SessionBinding::Linked { rig, seat } => {
                *seat_tokens.entry(seat.clone()).or_default() += Categories::tokens_of(row);
                seat_categories
                    .entry((seat.clone(), rig.clone()))
                    .or_default()
                    .add(row);
            }
            metrics::SessionBinding::Unlinked => unlinked.add(row),
            metrics::SessionBinding::Collision => collision.add(row),
        }
    }
    let mut snapshot_destinations =
        std::collections::BTreeMap::<String, Vec<crate::DestinationSnapshot>>::new();
    for (qitem, snapshot) in &destination_snapshots {
        snapshot_destinations
            .entry(qitem.clone())
            .or_default()
            .push(snapshot.clone());
    }
    let k2_all = metrics::k2_closures(&queue_all, &snapshot_destinations, window);
    let k2_insufficient: Vec<_> = k2_all
        .iter()
        .filter(|(destination, _)| destination.is_none())
        .map(|(_, item)| item.clone())
        .collect();
    let mut m1_seat_names = std::collections::BTreeSet::new();
    // A seat that closed items is listed even with no tokens or handoffs.
    for (destination, _) in &k2_all {
        if let Some(seat) = destination {
            if rig_seats.as_ref().is_none_or(|seats| seats.contains(seat)) {
                m1_seat_names.insert(seat.clone());
            }
        }
    }
    for seat in seat_tokens.keys() {
        m1_seat_names.insert(seat.clone());
    }
    for edge in &edges {
        m1_seat_names.insert(edge.from_seat.clone());
    }
    let m1_by_seat: Vec<_> = m1_seat_names
        .iter()
        .map(|seat| {
            let seat_total = *seat_tokens.get(seat).unwrap_or(&0);
            let deliveries = edges
                .iter()
                .filter(|edge| edge.from_seat == *seat && !edge.returned_within(window))
                .count() as i64;
            let closed =
                metrics::closed_items(seat, &queue_all, &snapshot_destinations, window);
            let per_closed = if closed.is_empty() {
                serde_json::Value::String("no data".into())
            } else {
                json!(seat_total as f64 / closed.len() as f64)
            };
            json!({
                "seat": seat,
                "tokens": seat_total,
                "nonreturn_deliveries": deliveries,
                "yield": metrics::m1_yield(seat_total, deliveries),
                "closed_items": closed.len(),
                "closed_item_keys": closed.iter().map(|item| item.transition.clone()).collect::<Vec<_>>(),
                "closed_item_members": closed,
                "tokens_per_closed_item": per_closed,
            })
        })
        .collect::<Vec<_>>();
    let mut pair_counts = std::collections::BTreeMap::<(String, String), (i64, i64)>::new();
    for edge in &edges {
        let slot = pair_counts
            .entry((edge.from_seat.clone(), edge.to_seat.clone()))
            .or_insert((0, 0));
        slot.1 += 1;
        if edge.is_return {
            slot.0 += 1;
        }
    }
    // Every in-window handoff with both classifications kept apart:
    // is_return (M2 numerator) and returned_within_window (M1, K6).
    let handoff_members: Vec<_> = edges
        .iter()
        .map(|edge| {
            json!({
                "source_key": edge.source_key,
                "at": edge.ts.to_rfc3339(),
                "from": edge.from_seat,
                "to": edge.to_seat,
                "qitem_id": edge.qitem_id,
                "chain": edge.chain_id,
                "is_return": edge.is_return,
                "returned_by": edge.returned_by.as_ref().map(|(key, at)| {
                    json!({"source_key": key, "at": at.to_rfc3339()})
                }),
                "returned_within_window": edge.returned_within(window),
            })
        })
        .collect();
    let m2_pairs: Vec<_> = pair_counts
        .iter()
        .map(|((from_seat, to_seat), (pair_returns, pair_handoffs))| {
            json!({
                "from": from_seat,
                "to": to_seat,
                "returns": pair_returns,
                "handoffs": pair_handoffs,
                "return_rate": metrics::m2_return_rate(*pair_returns, *pair_handoffs),
            })
        })
        .collect();
    let activity: Vec<_> = store
        .load_activity()?
        .into_iter()
        .filter(|row| match &rig_seats {
            Some(seats) => seats.contains(&row.seat),
            None => true,
        })
        .collect();
    let bounds = store.load_seat_bounds()?;
    let mut seats: Vec<String> = activity.iter().map(|row| row.seat.clone()).collect();
    for bound in &bounds {
        if rig_seats
            .as_ref()
            .is_none_or(|seats| seats.contains(&bound.seat))
        {
            seats.push(bound.seat.clone());
        }
    }
    seats.sort();
    seats.dedup();
    let seat_lives: Vec<_> = bounds
        .iter()
        .filter(|bound| seats.contains(&bound.seat))
        .map(|bound| {
            json!({
                "seat": bound.seat,
                "launched_at": bound.created_at.map(|t| t.to_rfc3339()),
                "stopped_at": bound.stopped_at.map(|t| t.to_rfc3339()),
            })
        })
        .collect();
    let scoped_tools: Vec<_> = store
        .load_tools()?
        .into_iter()
        .filter(|tool| tool.harness != "codex")
        .filter(|tool| match &allowed {
            Some(set) => set.contains(&(tool.harness.clone(), tool.native_session.clone())),
            None => rig_seats.is_none(),
        })
        .collect();
    let tools: Vec<_> = scoped_tools
        .iter()
        .filter(|tool| {
            tool.ts
                .is_some_and(|ts| ts >= window.from && ts < window.to)
        })
        .cloned()
        .collect();
    // K11: a failure without a timestamp stays in its run and in the count,
    // though it can never be the 1st or 3rd of a qualifying three.
    let loop_tools: Vec<_> = scoped_tools
        .iter()
        .filter(|tool| tool.ts.is_none_or(|ts| ts >= window.from && ts < window.to))
        .cloned()
        .collect();
    let native_evaluation = metrics::evaluate_tool_loops(
        &loop_tools,
        thresholds.tool_loop_failures,
        thresholds.tool_loop_window_secs,
    );
    let mut loops: Vec<_> = native_evaluation
        .episodes
        .iter()
        .map(|e| e.projection.clone())
        .collect();
    let source_coverage = store.load_source_coverage()?;
    let source_in_scope = |source: &&crate::SourceCoverage| {
        rig.is_none()
            || source
                .native_session
                .as_deref()
                .is_some_and(|native| in_scope(&source.harness, native))
    };
    let excluded_sources = if rig.is_some() {
        source_coverage
            .iter()
            .filter(|source| {
                source
                    .native_session
                    .as_deref()
                    .is_none_or(|native| binding(&source.harness, native).seat().is_none())
            })
            .count()
    } else {
        0
    };
    let codex_observations: Vec<_> = store
        .load_codex_observations()?
        .into_iter()
        .filter(|row| in_scope("codex", &row.fact.native_session))
        .filter(|row| {
            row.fact
                .ts
                .is_none_or(|ts| ts >= window.from && ts < window.to)
        })
        .collect();
    let trusted_codex: Vec<_> = codex_observations
        .iter()
        .filter(|row| binding("codex", &row.fact.native_session).seat().is_some())
        .cloned()
        .collect();
    let trusted_indices: Vec<_> = codex_observations
        .iter()
        .enumerate()
        .filter(|(_, row)| binding("codex", &row.fact.native_session).seat().is_some())
        .map(|(i, _)| i)
        .collect();
    let codex_evaluation = metrics::evaluate_codex_loops_with(
        &trusted_codex,
        thresholds.tool_loop_failures,
        thresholds.tool_loop_window_secs,
    );
    loops.extend(
        codex_evaluation
            .episodes
            .iter()
            .map(|e| e.projection.clone()),
    );
    let tool_categories = store.load_tool_categories()?;
    let mut tool_categories_summary = crate::tool_category::ToolCategoriesSummary::default();
    for seat in &seats {
        tool_categories_summary.by_seat.insert(
            seat.clone(),
            crate::tool_category::ToolCategoryBreakdown::empty(),
        );
    }

    let in_window_codex: Vec<_> = codex_observations
        .iter()
        .filter(|row| {
            row.fact
                .ts
                .is_some_and(|ts| ts >= window.from && ts < window.to)
        })
        .map(|row| &row.fact)
        .collect();

    for tool in tools.iter().chain(in_window_codex.iter().copied()) {
        let cat = tool_categories
            .get(&tool.source_key)
            .copied()
            .unwrap_or_else(|| crate::facts::ToolCategory::classify_tool_name(&tool.name));
        tool_categories_summary
            .overall
            .record(cat, &tool.source_key);

        let seat_opt = binding(&tool.harness, &tool.native_session)
            .seat()
            .map(String::from);
        let mission_label = match (&seat_opt, tool.ts) {
            (Some(seat_name), Some(ts)) => {
                let evidence =
                    metrics::mission_evidence(seat_name, &queue_all, &snapshot_destinations, ts);
                match evidence.bucket {
                    metrics::MissionBucket::Mission(name) => format!("mission:{name}"),
                    metrics::MissionBucket::None => "no_mission".to_string(),
                    metrics::MissionBucket::Ambiguous(_) => "ambiguous".to_string(),
                }
            }
            (None, _) => "unlinked".to_string(),
            _ => "untimestamped".to_string(),
        };

        if let Some(seat_name) = &seat_opt {
            tool_categories_summary
                .by_seat
                .entry(seat_name.clone())
                .or_insert_with(crate::tool_category::ToolCategoryBreakdown::empty)
                .record(cat, &tool.source_key);
        }
        tool_categories_summary
            .by_mission
            .entry(mission_label)
            .or_insert_with(crate::tool_category::ToolCategoryBreakdown::empty)
            .record(cat, &tool.source_key);
    }

    let queue_meta = store.load_queue_item_meta()?;
    let fleet_concurrency =
        crate::concurrency::fleet_concurrency(window, &seats, &activity, &bounds);

    let activity_rows: Vec<_> = seats
        .iter()
        .map(|seat| {
            let lives: Vec<SeatBound> = bounds
                .iter()
                .filter(|bound| bound.seat == *seat)
                .cloned()
                .collect();
            let bands = metrics::activity_bands(seat, &activity, &queue_all, window, &lives);
            let spans = metrics::observation_spans(window, &lives);
            let evidence = |recorded: bool| if recorded { "recorded" } else { "unavailable" };
            let seat_bd = tool_categories_summary
                .by_seat
                .get(seat)
                .cloned()
                .unwrap_or_else(crate::tool_category::ToolCategoryBreakdown::empty);
            let disambiguation = crate::concurrency::disambiguate_container_claims_with_bands(
                seat,
                &activity,
                &queue_all,
                window,
                &lives,
                &queue_meta,
                &bands,
            );
            json!({
                "seat": seat,
                "observed_secs": metrics::observed_secs(&spans),
                "observed_spans": spans
                    .iter()
                    .map(|span| json!({"from": span.start.to_rfc3339(), "to": span.end.to_rfc3339()}))
                    .collect::<Vec<_>>(),
                "launch_evidence": evidence(lives.iter().any(|life| life.created_at.is_some())),
                "stop_evidence": evidence(lives.iter().any(|life| life.stopped_at.is_some())),
                "active_secs": bands.active_secs,
                "idle_with_work_secs": bands.idle_with_work_secs,
                "idle_without_work_secs": bands.idle_without_work_secs,
                "waiting_human_secs": bands.waiting_human_secs,
                "unknown_secs": bands.unknown_secs,
                "sum": bands.sum(),
                "unparked_container_secs": disambiguation.unparked_container_secs,
                "leaf_stall_secs": disambiguation.leaf_stall_secs,
                "container_hold_evidence": disambiguation.container_hold_evidence,
                "ceremony_calls": seat_bd.ceremony_calls,
                "engineering_calls": seat_bd.engineering_calls,
                "ceremony_ratio": seat_bd.ceremony_ratio,
                "tool_categories": seat_bd.categories,
            })
        })
        .collect();
    let loop_trace = if evidence {
        let source_ids = store.tool_source_ids()?;
        let prefixes = store.source_prefixes()?;
        crate::loop_trace::build(crate::loop_trace::EvidenceInput {
            native: &loop_tools,
            codex: &codex_observations,
            trusted_indices: &trusted_indices,
            native_evaluation: &native_evaluation,
            codex_evaluation: &codex_evaluation,
            source_ids: &source_ids,
            prefixes: &prefixes,
            coverage: &source_coverage,
            bindings: &bindings,
            rig,
            from,
            to,
            failures: thresholds.tool_loop_failures,
            window_secs: thresholds.tool_loop_window_secs,
        })
    } else {
        Value::Null
    };
    let outcome_coverage: Vec<_> = [("claude", "tool_call"), ("pi", "tool_call"),
        ("codex", "tool_call"), ("codex", "command_execution")].into_iter().map(|(harness, unit)| {
        let relevant: Vec<_> = source_coverage.iter().filter(|source| source.harness == harness)
            .filter(source_in_scope).collect();
        let covered = unit != "tool_call" || harness != "codex";
        let read = if covered { relevant.iter().filter(|source| source.complete).count() } else { 0 };
        let facts: Vec<_> = if harness == "codex" {
            codex_observations.iter().filter(|row| row.fact.name == crate::CODEX_COMMAND).map(|row| &row.fact).collect()
        } else { loop_tools.iter().filter(|row| row.harness == harness).collect() };
        let unknowns = if covered { json!({
            "input": facts.iter().filter(|row| row.fingerprint.is_none()).count(),
            "outcome": facts.iter().filter(|row| row.failed.is_none()).count(),
            "timestamp": facts.iter().filter(|row| row.ts.is_none()).count(),
            "linkage": facts.iter().filter(|row| binding(harness, &row.native_session).seat().is_none()).count(),
        }) } else { json!({"input": null, "outcome": null, "timestamp": null, "linkage": null}) };
        json!({"harness": harness, "unit": unit, "status": if read > 0 && read == relevant.len() { "read" } else { "not read" },
            "sources_total": relevant.len(), "sources_read": read, "unknowns": unknowns})
    }).collect();
    let seat_of = |harness: &str, native: &str| binding(harness, native).seat().map(String::from);
    let linked_seats: std::collections::BTreeSet<String> = bindings
        .iter()
        .filter(|((harness, native), _)| in_scope(harness, native))
        .filter_map(|(_, bound)| bound.seat().map(String::from))
        .collect();
    let intakes =
        metrics::handoff_intakes(&edges, &queue_all, &tools, &usage, &linked_seats, seat_of);
    let medians = metrics::pair_medians(&edges, &queue_all, &intakes);
    // AC6 / K3: each in-window usage record lands in exactly one band. Per seat,
    // missions + no_mission + ambiguous = the seat total. Unlinked usage stays
    // apart from "no mission"; untimestamped usage is counted all-time only.
    let mut mission_tokens = std::collections::BTreeMap::<String, i64>::new();
    let mut mission_records = Vec::new();
    let mut ambiguous_by_reason = std::collections::BTreeMap::<String, i64>::new();
    let mut seat_bands =
        std::collections::BTreeMap::<String, std::collections::BTreeMap<String, i64>>::new();
    let mut seat_reasons =
        std::collections::BTreeMap::<String, std::collections::BTreeMap<String, i64>>::new();
    for row in &usage {
        let seat = binding(&row.harness, &row.native_session)
            .seat()
            .map(String::from);
        let amount = row.input_uncached + row.output + row.cache_read + row.cache_write;
        let (Some(seat), Some(ts)) = (seat, row.ts) else {
            *mission_tokens.entry("unlinked".into()).or_insert(0) += amount;
            continue;
        };
        let evidence = metrics::mission_evidence(&seat, &queue_all, &snapshot_destinations, ts);
        mission_records.push(json!({
            "source_key": row.source_key,
            "ts": ts.to_rfc3339(),
            "seat": seat,
            "input_uncached": row.input_uncached,
            "output": row.output,
            "cache_read": row.cache_read,
            "cache_write": row.cache_write,
            "tokens": amount,
            "bucket": match &evidence.bucket {
                metrics::MissionBucket::Mission(name) => format!("mission:{name}"),
                metrics::MissionBucket::None => "no_mission".to_string(),
                metrics::MissionBucket::Ambiguous(_) => "ambiguous".to_string(),
            },
            "primary_reason": evidence.reasons.first(),
            "secondary_reasons": evidence.reasons.iter().skip(1).collect::<Vec<_>>(),
            "items": evidence.items,
        }));
        let label = match evidence.bucket {
            metrics::MissionBucket::Mission(name) => format!("mission:{name}"),
            metrics::MissionBucket::None => "no_mission".into(),
            metrics::MissionBucket::Ambiguous(reason) => {
                *ambiguous_by_reason.entry(reason.into()).or_insert(0) += amount;
                *seat_reasons
                    .entry(seat.clone())
                    .or_default()
                    .entry(reason.into())
                    .or_insert(0) += amount;
                "ambiguous".into()
            }
        };
        *mission_tokens.entry(label.clone()).or_insert(0) += amount;
        *seat_bands
            .entry(seat)
            .or_default()
            .entry(label)
            .or_insert(0) += amount;
    }
    let mission_by_seat: Vec<_> = seat_bands
        .iter()
        .map(|(seat, bands)| {
            let total: i64 = bands.values().sum();
            json!({
                "seat": seat,
                "total": total,
                "bands": bands,
                "ambiguous_by_reason": seat_reasons.get(seat).cloned().unwrap_or_default(),
                "conserved": total
                    == bands.iter().filter(|(label, _)| label.starts_with("mission:")).map(|(_, v)| v).sum::<i64>()
                        + bands.get("no_mission").copied().unwrap_or(0)
                        + bands.get("ambiguous").copied().unwrap_or(0),
            })
        })
        .collect();
    let snapshot_rows: Vec<_> = destination_snapshots
        .iter()
        .map(|(qitem, snapshot)| {
            json!({
                "qitem_id": qitem,
                "source_key": format!("queue_items:{qitem}:{}:{}:{}", snapshot.destination, snapshot.cutoff, json!(snapshot.missions)),
                "destination": snapshot.destination,
                "snapshot_cutoff": snapshot.cutoff,
                "reroute_marked": snapshot.reroute_marked,
                "missions": snapshot.missions,
                "historical": false,
            })
        })
        .collect();
    let untimestamped_tokens: i64 = untimestamped
        .iter()
        .map(|row| Categories::tokens_of(row))
        .sum();
    // Each discovered session with how it was found: usage records, a
    // zero-usage session file, or only an OpenRig identity event.
    let mut discovered = std::collections::BTreeSet::<(String, String)>::new();
    let mut found_by =
        std::collections::BTreeMap::<(String, String), std::collections::BTreeSet<&str>>::new();
    let mut pi_origins =
        std::collections::BTreeMap::<String, std::collections::BTreeSet<&'static str>>::new();
    for row in &usage_all {
        let session = (row.harness.clone(), row.native_session.clone());
        found_by.entry(session.clone()).or_default().insert("usage");
        discovered.insert(session);
    }
    for fact in &identities {
        let session = (fact.harness.clone(), fact.native_session.clone());
        found_by
            .entry(session.clone())
            .or_default()
            .insert("identity");
        discovered.insert(session);
        if fact.harness == "pi" {
            pi_origins
                .entry(fact.native_session.clone())
                .or_default()
                .insert("openrig");
        }
    }
    for (source_id, harness, path, _generation, native) in &sources {
        if harness == "pi" {
            let session_id = native
                .clone()
                .unwrap_or_else(|| format!("file:{source_id}"));
            let origin = if path.contains("/.pi/agent/sessions")
                || path.contains("\\.pi\\agent\\sessions")
            {
                "pi-home"
            } else {
                "openrig"
            };
            pi_origins.entry(session_id).or_default().insert(origin);
        }
        // An OpenRig database is a provenance row, not a harness session.
        if harness == "openrig" {
            continue;
        }
        let prefix = format!("{source_id}:");
        let has_usage = usage_all
            .iter()
            .any(|row| row.source_key.starts_with(&prefix));
        if !has_usage {
            // A Pi or Codex file is discovered under its line-1 native session
            // (K10), so a zero-usage session can still link to its seat.
            let session = native
                .clone()
                .unwrap_or_else(|| format!("file:{source_id}"));
            found_by
                .entry((harness.clone(), session.clone()))
                .or_default()
                .insert("zero-usage file");
            discovered.insert((harness.clone(), session));
        }
    }
    let mut link_rates = std::collections::BTreeMap::<String, (i64, i64)>::new();
    for (harness, native) in &discovered {
        let entry = link_rates.entry(harness.clone()).or_insert((0, 0));
        entry.1 += 1;
        if binding(harness, native).seat().is_some() {
            entry.0 += 1;
        }
    }
    // AC6: one row per discovered session in scope, with its period categories
    // and its untimestamped records, which stay outside every period figure.
    // Fleet totals sum each `native_session` once. Grok child sessions have
    // their own rows; `subagent_finished.tokens_used` is not added again.
    // Claude sidechain usage is stored under `sessionId`, which is the parent
    // session, so that parent's total already includes it. It is not re-keyed
    // onto `agentId`.
    let mut session_period =
        std::collections::BTreeMap::<(String, String), (Categories, usize)>::new();
    for row in &usage {
        let slot = session_period
            .entry((row.harness.clone(), row.native_session.clone()))
            .or_default();
        slot.0.add(row);
        slot.1 += 1;
    }
    let usage_models = store.load_usage_models()?;
    let slices: Vec<_> = usage
        .iter()
        .map(|row| {
            let model = usage_models
                .get(&row.source_key)
                .and_then(|(model, _)| model.as_deref());
            usage_slice(row, model)
        })
        .collect();
    let fleet_cost = crate::accounting_v1::fleet(&slices);
    let exclusive_cost = crate::accounting_v1::exclusive_sessions(&slices);
    let lineage = store
        .load_session_edges()?
        .into_iter()
        .map(|edge| crate::accounting_v1::SessionLink {
            harness: edge.harness,
            parent_session: edge.parent_session,
            child_session: edge.child_session,
        })
        .collect::<Vec<_>>();
    let inclusive_cost = crate::accounting_v1::inclusive_roots(&slices, &lineage);
    let exclusive_by_session = exclusive_cost
        .iter()
        .map(|session| ((session.harness.clone(), session.session.clone()), session))
        .collect::<std::collections::BTreeMap<_, _>>();
    let inclusive_by_session = inclusive_cost
        .iter()
        .map(|session| ((session.harness.clone(), session.session.clone()), session))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut session_untimestamped =
        std::collections::BTreeMap::<(String, String), (usize, i64)>::new();
    for row in &untimestamped {
        let slot = session_untimestamped
            .entry((row.harness.clone(), row.native_session.clone()))
            .or_default();
        slot.0 += 1;
        slot.1 += Categories::tokens_of(row);
    }
    let session_rows: Vec<_> = discovered
        .iter()
        .filter(|(harness, native)| in_scope(harness, native))
        .map(|(harness, native)| {
            let key = (harness.clone(), native.clone());
            let bound = binding(harness, native);
            let (categories, records) = session_period.get(&key).cloned().unwrap_or_default();
            let (untimestamped_records, untimestamped_session_tokens) =
                session_untimestamped.get(&key).copied().unwrap_or_default();
            let mut row = categories.json(Some(harness));
            row["harness"] = json!(harness);
            row["session"] = json!(native);
            row["status"] = json!(bound.status());
            row["rig"] = json!(bound.rig());
            row["seat"] = json!(bound.seat());
            if harness == "pi" {
                let origin_str = if let Some(origins) = pi_origins.get(native) {
                    if origins.contains("openrig") && origins.contains("pi-home") {
                        "both"
                    } else if origins.contains("pi-home") {
                        "pi-home"
                    } else {
                        "openrig"
                    }
                } else {
                    "openrig"
                };
                row["pi_origin"] = json!(origin_str);
            }
            row["period_records"] = json!(records);
            let exclusive = exclusive_by_session
                .get(&key)
                .map(|session| session.population.clone())
                .unwrap_or_else(crate::accounting_v1::Population::empty);
            row["observed_cost"] = cost_json(exclusive.observed);
            row["estimated_cost"] = cost_json(exclusive.estimated);
            if let Some(inclusive) = inclusive_by_session.get(&key) {
                let tokens = inclusive.population.tokens;
                row["inclusive"] = json!({
                    "label": inclusive.label,
                    "non_additive": inclusive.non_additive,
                    "input_uncached": tokens.input_uncached,
                    "output": tokens.output,
                    "cache_read": tokens.cache_read,
                    "cache_write": tokens.cache_write,
                    "tokens": tokens.tokens(),
                    "reasoning": if harness == "claude" {
                        json!("not recorded")
                    } else {
                        json!(tokens.reasoning)
                    },
                    "observed_cost": cost_json(inclusive.population.observed),
                    "estimated_cost": cost_json(inclusive.population.estimated),
                });
            }
            row["discovered_by"] = json!(found_by.get(&key).cloned().unwrap_or_default());
            row["untimestamped"] = json!({
                "records": untimestamped_records,
                "tokens": untimestamped_session_tokens,
                "scope": "session only, outside every period figure",
            });
            row
        })
        .collect();
    let seat_category_rows: Vec<_> = seat_categories
        .iter()
        .map(|((seat, seat_rig), categories)| {
            let mut row = categories.json(None);
            row["seat"] = json!(seat);
            row["rig"] = json!(seat_rig);
            row
        })
        .collect();
    let links: Vec<_> = discovered
        .into_iter()
        .map(|(harness, native)| {
            let matches: Vec<_> = identities
                .iter()
                .filter(|fact| fact.harness == harness && fact.native_session == native)
                .collect();
            let bound = binding(&harness, &native);
            json!({
                "harness": harness,
                "native_session": native,
                "status": bound.status(),
                "rig": bound.rig(),
                "seat": bound.seat(),
                "evidence": matches.iter().map(|fact| fact.source_key.clone()).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let trace_inputs: Vec<_> = usage
        .iter()
        .map(|row| {
            let suffix = format!(":{}:{}", row.generation, row.seq);
            let source_id = row.source_key.strip_suffix(&suffix);
            let path = source_id.and_then(|id| {
                sources
                    .iter()
                    .find(|source| source.0 == id)
                    .map(|source| source.2.clone())
            });
            json!({
                "source_key": row.source_key,
                "generation": row.generation,
                "line": row.seq,
                "path": path,
                "predecessor_reason": row.predecessor_reason,
                "predecessor_source_key": row.predecessor_source_key,
                "prior_input_tokens": row.prior_input_tokens,
                "prior_cached_input_tokens": row.prior_cached_input_tokens,
                "prior_cache_write_input_tokens": row.prior_cache_write_input_tokens,
                "prior_output_tokens": row.prior_output_tokens,
                "prior_reasoning_output_tokens": row.prior_reasoning_output_tokens,
                "input_uncached": row.input_uncached,
                "output": row.output,
                "cache_read": row.cache_read,
                "cache_write": row.cache_write,
            })
        })
        .collect();
    let handoff_loops = metrics::handoff_loops(&edges, thresholds.handoff_loop_returns);
    let all_skill_facts = store.load_skills()?;
    let resolve_skill_seat = |skill: &SkillFact| -> Option<String> {
        if let Some((harness, native_session)) = skill.seat.split_once(':') {
            if matches!(harness, "claude" | "codex" | "pi") {
                if let SessionBinding::Linked {
                    rig: bound_rig,
                    seat: bound_seat,
                } = binding(harness, native_session)
                {
                    if rig.is_none_or(|r| r == bound_rig) {
                        return Some(bound_seat.clone());
                    }
                }
                return None;
            }
        }
        if rig_seats
            .as_ref()
            .is_none_or(|seats| seats.contains(&skill.seat))
        {
            return Some(skill.seat.clone());
        }
        None
    };
    let mut resolved_skills: Vec<(String, SkillFact)> = Vec::new();
    for fact in all_skill_facts {
        if let Some(canonical_seat) = resolve_skill_seat(&fact) {
            resolved_skills.push((canonical_seat, fact));
        }
    }
    let m4_skill_exposure = metrics::m4_skill_exposure(&resolved_skills);
    let mut payload = json!({
        "product": "AI Fleet Intelligence",
        "rig": rig.map(redact),
        "from": redact(from),
        "to": redact(to),
        "usage_rows": usage.len(),
        "untimestamped_count": untimestamped_count,
        "token_categories": overall.json(None),
        "seat_token_categories": seat_category_rows,
        "unlinked_tokens": Categories::sum_tokens(&[&unlinked, &collision]),
        "unlinked_token_categories": {
            "unlinked": unlinked.json(None),
            "collision": collision.json(None),
        },
        "sessions": session_rows,
        "reasoning_rule": REASONING_RULE,
        "link_rates": link_rates.iter().map(|(harness, (linked, discovered_count))| json!({
            "harness": harness,
            "linked": linked,
            "discovered": discovered_count,
            "status": if *discovered_count == 0 { "no data" } else { "ok" },
        })).collect::<Vec<_>>(),
        "tokens": tokens,
        "observed_cost": cost_json(fleet_cost.observed),
        "estimated_cost": cost_json(fleet_cost.estimated),
        "cost_rule": "observed and estimated are never added; unknown is not zero; inclusive session totals are non-additive",
        "m1_yield": metrics::m1_yield(tokens, nonreturn),
        "m1_by_seat": m1_by_seat,
        "k2_closed_item": metrics::K2_CLOSED_ITEM,
        "k2_destination_rule": metrics::K2_DESTINATION_RULE,
        "k2_closed_items_with_insufficient_history": k2_insufficient.len(),
        "k2_insufficient_history_members": k2_insufficient,
        "m2_return_rate": metrics::m2_return_rate(returns, handoffs),
        "m2_pairs": m2_pairs,
        "m3_rework_chains": rework,
        "m3_chains": chains,
        "m3_paths": paths,
        "m3_rework_members": rework_members,
        "m3_unknown_chains": m3_unknown.len(),
        "m3_unknown_members": m3_unknown,
        "m3_rework_proportion": m3_proportion,
        "m4_impact": metrics::m4_impact(),
        "kind": redact(kind),
        "activity": activity_rows,
        "fleet_concurrency": fleet_concurrency,
        "seat_lives": seat_lives,
        "links": links,
        "handoff_edges": edges.iter().map(|edge| json!({
            "from": edge.from_seat,
            "to": edge.to_seat,
            "return": edge.is_return,
            "qitem_id": edge.qitem_id,
            "source_key": edge.source_key,
        })).collect::<Vec<_>>(),
        "intakes": intakes.iter().map(|row| json!({
            "qitem_id": row.qitem_id,
            "to": row.to_seat,
            "status": row.status,
            "reason": row.reason,
            "tokens": row.tokens,
            "claim": row.claim,
            "write": row.write,
            "samples": row.samples,
        })).collect::<Vec<_>>(),
        "pair_medians": medians.iter().map(|row| json!({
            "from": row.from_seat,
            "to": row.to_seat,
            "wait_samples": row.wait_samples,
            "wait_sample_keys": row.wait_sample_keys,
            "wait_invalid": row.wait_invalid,
            "wait_median": row.wait_median,
            "intake_samples": row.intake_samples,
            "intake_invalid": row.intake_invalid,
            "intake_median": row.intake_median,
        })).collect::<Vec<_>>(),
        "openrig_sources": openrig_sources,
        "mission_definition": metrics::MISSION_DEFINITION,
        "mission_tokens": mission_tokens,
        "mission_records": mission_records,
        "mission_ambiguous_by_reason": ambiguous_by_reason,
        "mission_by_seat": mission_by_seat,
        "untimestamped_tokens_all_time": untimestamped_tokens,
        "tool_loops": loops.iter().map(|item| json!({"session": item.session, "tool": item.tool, "count": item.count,
            "harness": item.harness, "unit": if item.harness == "codex" { "command_execution" } else { "tool_call" },
            "seat": seat_of(&item.harness, &item.session)})).collect::<Vec<_>>(),
        "outcome_coverage": outcome_coverage,
        "outcome_coverage_exclusions": { "unresolved_sources": excluded_sources },
        "activity_events": activity.iter().map(|row| json!({
            "source_key": row.source_key,
            "seat": row.seat,
            "state": row.state,
            "ts": row.ts.to_rfc3339(),
        })).collect::<Vec<_>>(),
        "demand_events": queue_all.iter().map(|row| json!({
            "source_key": row.source_key,
            "qitem_id": row.qitem_id,
            "state": row.state,
            "dest": row.dest_seat,
            "ts": row.ts.to_rfc3339(),
        })).collect::<Vec<_>>(),
        // M1 denominator membership: deliveries not returned inside the window (K6).
        "nonreturn_handoffs": edges.iter().filter(|edge| !edge.returned_within(window)).map(|edge| json!({
            "source_key": edge.source_key,
            "from": edge.from_seat,
            "to": edge.to_seat,
            "qitem_id": edge.qitem_id,
        })).collect::<Vec<_>>(),
        "handoff_members": handoff_members,
        "destination_snapshots": snapshot_rows,
        "trace_inputs": trace_inputs,
        "tool_categories": tool_categories_summary.to_payload_value(),
    });
    if !thresholds.is_default() {
        payload["loop_thresholds"] = json!(thresholds);
        payload["handoff_loops"] = json!(handoff_loops);
    }
    if !m4_skill_exposure.is_empty() {
        payload["m4_skill_exposure"] = json!(m4_skill_exposure);
    }
    let compaction = crate::compaction::evaluate(crate::compaction::Input {
        usage: &usage_all,
        queue: &queue_all,
        history_queue: &queue_context,
        meta: &store.load_queue_item_meta()?,
        snapshots: &snapshot_destinations,
        bindings: &bindings,
        rig,
        window,
    });
    for field in [
        "context_compactions",
        "context_depth_brackets",
        "cross_slice_cache_bleed",
    ] {
        payload[field] = compaction[field].clone();
    }
    let wave1 = crate::turnaround::evaluate(crate::turnaround::Input {
        queue: &queue_all,
        history_queue: &queue_context,
        meta: &queue_meta,
        edges: &edges,
        closures: &k2_all,
        identities: &identities,
        usage: &usage,
        models: &store.load_usage_models()?,
        bindings: &bindings,
        snapshots: &snapshot_destinations,
        roles: &roles,
        window,
    });
    for field in [
        "watchdog_recovery",
        "ring_boundaries",
        "exec_turnaround",
        "model_effort_tokens",
        "four_eyes_independence",
        "issue_lineage",
        "task_intake_overhead",
    ] {
        payload[field] = wave1[field].clone();
    }
    for row in payload["pair_medians"].as_array_mut().expect("pair rows") {
        if let Some(exec) = wave1["exec_pairs"]
            .as_array()
            .expect("execution pairs")
            .iter()
            .find(|e| e["from"] == row["from"] && e["to"] == row["to"])
        {
            for field in [
                "exec_samples",
                "exec_sample_keys",
                "exec_invalid",
                "exec_median",
            ] {
                row[field] = exec[field].clone();
            }
        }
    }
    for row in payload["m1_by_seat"].as_array_mut().expect("M1 rows") {
        if let Some(counts) = wave1["watchdog_recovery"]["by_seat"]
            .as_array()
            .expect("recovery seats")
            .iter()
            .find(|c| c["seat"] == row["seat"])
        {
            for field in [
                "productive_closed_items",
                "recovery_closed_items",
                "recovery_tax_ratio",
                "recovery_by_reason",
            ] {
                row[field] = counts[field].clone();
            }
        } else {
            row["productive_closed_items"] = json!(0);
            row["recovery_closed_items"] = json!(0);
            row["recovery_tax_ratio"] = Value::Null;
            row["recovery_by_reason"] = json!({});
        }
    }
    let friction_evaluation = if matches!(
        kind,
        "handoffs" | "decisions" | "advisory" | "anomalies" | "spotlights"
    ) && (!evidence
        || matches!(
            requested_metric,
            Some("handoff_friction" | "anomaly_spotlights")
        )) {
        let friction_usage: Vec<_> = usage_all
            .iter()
            .filter(|u| in_scope(&u.harness, &u.native_session))
            .cloned()
            .collect();
        crate::vortex::evaluate(crate::vortex::Input {
            queue: &queue_context,
            metadata: &store.load_queue_metadata_evidence()?,
            edges: &edges,
            usage: &friction_usage,
            identities: &identities,
            activity: &store.load_activity()?,
            snapshots: &snapshot_destinations,
            window,
            thresholds: friction,
            capture_replay: evidence && requested_metric == Some("handoff_friction"),
        })
    } else {
        crate::vortex::Evaluation {
            value: Value::Null,
            replay: Value::Null,
        }
    };
    // Add after legacy provenance annotation so report/trace metric values stay identical.
    annotate_openrig_evidence(store, &sources, &mut payload, evidence)?;
    if matches!(
        kind,
        "handoffs" | "decisions" | "advisory" | "anomalies" | "spotlights"
    ) {
        payload["handoff_friction"] = friction_evaluation.value;
    }
    if matches!(kind, "advisory" | "anomalies" | "spotlights" | "decisions")
        && (!evidence
            || matches!(
                requested_metric,
                Some("advisory_idle_claimed" | "anomaly_spotlights")
            ))
    {
        let scoped_seats: std::collections::BTreeSet<_> = identities
            .iter()
            .filter(|i| rig.is_none_or(|r| i.rig == r))
            .map(|i| i.seat.clone())
            .chain(seats.iter().cloned())
            .chain(
                queue_all
                    .iter()
                    .map(|q| q.dest_seat.clone())
                    .filter(|seat| rig_seats.as_ref().is_none_or(|set| set.contains(seat))),
            )
            .collect();
        let mut local_activity = Vec::new();
        let mut unresolved_activity = Vec::new();
        let mut add_local =
            |key: &str, harness: &str, native: &str, ts: Option<chrono::DateTime<chrono::Utc>>| {
                if !in_scope(harness, native) || ts.is_some_and(|t| t >= window.to) {
                    return;
                }
                let seat = binding(harness, native).seat().map(str::to_owned);
                match (seat, ts) {
                    (Some(seat), Some(ts)) => local_activity.push(crate::advisory::ActivityPoint {
                        seat,
                        ts,
                        source_key: key.into(),
                    }),
                    (seat, _) => unresolved_activity.push(crate::advisory::UnresolvedActivity {
                        source_key: key.into(),
                        seat,
                        reason: "missing_timestamp_or_unproven_session_binding",
                    }),
                }
            };
        for u in &usage_all {
            add_local(&u.source_key, &u.harness, &u.native_session, u.ts);
        }
        for t in scoped_tools
            .iter()
            .chain(codex_observations.iter().map(|r| &r.fact))
        {
            add_local(&t.source_key, &t.harness, &t.native_session, t.ts);
        }
        let raw_meta = store.load_queue_metadata_evidence()?;
        let idle = crate::advisory::evaluate(crate::advisory::Input {
            queue: &queue_context,
            metadata: &raw_meta,
            activity: &activity,
            bounds: &bounds,
            local_activity: &local_activity,
            unresolved_activity: &unresolved_activity,
            snapshots: &snapshot_destinations,
            seats: &scoped_seats,
            nudge_metadata: &queue_meta,
            window,
            options: advisory,
        });
        payload["advisory_idle_claimed"] = idle;
        let represented: std::collections::BTreeSet<_> = queue_all
            .iter()
            .filter(|q| q.ts >= window.from && q.ts < window.to)
            .map(|q| q.qitem_id.as_str())
            .collect();
        payload["advisory_queue_volume"] = json!({"items":represented.len(),"recovery_items":represented.iter().filter(|id|queue_meta.get(**id).is_some_and(|m|m.is_recovery||m.recovery_reason.is_some())).count(),
            "source_keys":queue_all.iter().filter(|q|q.ts>=window.from&&q.ts<window.to).map(|q|&q.source_key).collect::<Vec<_>>()});
        let routing_tools: Vec<_> = tools
            .iter()
            .chain(in_window_codex.iter().copied())
            .cloned()
            .collect();
        let routing = crate::routing::evaluate(
            &usage,
            &routing_tools,
            &store.load_usage_models()?,
            &tool_categories,
            &bindings,
            window,
            &payload["outcome_coverage"],
        );
        for field in [
            "routing_effort_status",
            "routing_effort_evidence",
            "routing_effort_coverage",
        ] {
            payload[field] = routing[field].clone();
        }
        payload["anomaly_spotlights"] = crate::spotlight::evaluate(&payload, advisory);
        if evidence {
            payload["advisory_trace_inputs"] = json!({"queue":queue_context,"metadata":raw_meta,"nudge_metadata":queue_meta,"activity":activity,"seat_bounds":bounds,
                "local_activity":local_activity,"unresolved_activity":unresolved_activity,"destination_snapshots":snapshot_destinations,"identities":identities});
        }
    }
    let report = metrics::report_value(kind, payload);
    let file_states = if evidence {
        Some(crate::file_state::Captured::capture(
            store,
            &usage,
            &untimestamped,
            &sources,
            &source_coverage,
            &bindings,
            rig,
            &loop_trace,
        )?)
    } else {
        None
    };
    Ok(ReportEvaluation {
        report,
        loop_trace,
        file_states,
        friction_replay: friction_evaluation.replay,
    })
}

/// Enrich accepted facts without rereading live source contents. Only trace
/// probes source existence, and that probe never changes a computed total.
#[allow(clippy::type_complexity)]
fn annotate_openrig_evidence(
    store: &AnalyticsStore,
    sources: &[(String, String, String, i64, Option<String>)],
    payload: &mut Value,
    trace: bool,
) -> Result<()> {
    let records = store.load_openrig_row_records()?;
    let missing: std::collections::HashSet<_> = sources
        .iter()
        .filter(|(_, harness, path, _, _)| {
            harness == "openrig" && trace && !Path::new(path).exists()
        })
        .map(|(id, _, _, _, _)| id.as_str())
        .collect();
    let record_json = |r: &crate::OpenRigRecord| {
        let state = if missing.contains(r.source_id.as_str()) {
            "gone"
        } else {
            &r.row_state
        };
        json!({"source_id":r.source_id, "generation":r.generation,
            "producing_revision":r.producing_revision, "table_name":r.table_name,
            "row_id":r.row_id, "row_digest":r.row_digest,
            "source_key":r.source_key, "state":state,
            "status":if state == "gone" { "no longer available" } else { state }})
    };
    let by_key: std::collections::HashMap<_, _> = records
        .iter()
        .filter(|r| !r.source_key.is_empty())
        .map(|r| (r.source_key.clone(), record_json(r)))
        .collect();
    fn annotate(value: &mut Value, by_key: &std::collections::HashMap<String, Value>) {
        match value {
            Value::Array(items) => {
                for item in items {
                    annotate(item, by_key);
                }
            }
            Value::Object(object) => {
                for value in object.values_mut() {
                    annotate(value, by_key);
                }
                if let Some(evidence) = object
                    .get("source_key")
                    .or_else(|| object.get("transition"))
                    .and_then(Value::as_str)
                    .and_then(|key| by_key.get(key))
                {
                    for field in [
                        "source_id",
                        "generation",
                        "producing_revision",
                        "state",
                        "status",
                    ] {
                        // Preserve semantic activity/queue state, exposing row availability separately.
                        let name = if field == "state" && object.contains_key("state") {
                            "row_state"
                        } else {
                            field
                        };
                        object.insert(name.into(), evidence[field].clone());
                    }
                }
                if let Some(evidence) = object.get("evidence").and_then(Value::as_array) {
                    let provenance: Vec<_> = evidence
                        .iter()
                        .filter_map(Value::as_str)
                        .filter_map(|key| by_key.get(key))
                        .cloned()
                        .collect();
                    if let Some(last) = provenance
                        .iter()
                        .max_by_key(|p| p["generation"].as_i64().unwrap_or(0))
                    {
                        object.insert("generation".into(), last["generation"].clone());
                        object.insert(
                            "producing_revision".into(),
                            last["producing_revision"].clone(),
                        );
                    }
                    if !provenance.is_empty() {
                        object.insert("evidence_provenance".into(), json!(provenance));
                    }
                }
            }
            _ => {}
        }
    }
    annotate(payload, &by_key);
    if let Some(sources) = payload["openrig_sources"].as_array_mut() {
        for source in sources {
            let id = source["source_id"].as_str().unwrap_or("");
            let state = if missing.contains(id) {
                "gone"
            } else if records
                .iter()
                .any(|r| r.source_id == id && r.row_state == "changed")
            {
                "changed"
            } else {
                "present"
            };
            source["state"] = json!(state);
            source["status"] = json!(if state == "gone" {
                "no longer available"
            } else {
                state
            });
        }
    }
    payload["openrig_row_states"] = json!(records.iter().map(record_json).collect::<Vec<_>>());
    let disclosures = store.load_mutable_source_disclosures()?;
    if !disclosures.is_empty() {
        if let Some(seats) = payload["m1_by_seat"].as_array_mut() {
            for seat in seats {
                let affected: Vec<_> = disclosures
                    .iter()
                    .filter(|d| {
                        d["seat"] == seat["seat"]
                            || d["seat"] == "unknown"
                            || d["seat"] == ""
                            || seat["closed_item_members"]
                                .as_array()
                                .is_some_and(|members| {
                                    members
                                        .iter()
                                        .any(|member| d["qitem_id"] == member["qitem"])
                                })
                    })
                    .cloned()
                    .collect();
                if !affected.is_empty() {
                    seat["tokens_per_closed_item"] = json!("not reproducible: mutable source");
                    seat["mutable_source_disclosures"] = json!(affected);
                    if let Some(members) = seat["closed_item_members"].as_array_mut() {
                        for member in members {
                            if affected.iter().any(|d| {
                                d["qitem_id"] == member["qitem"]
                                    || d["seat"] == "unknown"
                                    || d["seat"] == ""
                            }) {
                                member["status"] = json!("not reproducible: mutable source");
                                member["mutable_source_disclosures"] = json!(affected);
                            }
                        }
                    }
                }
            }
        }
        if let Some(members) = payload["k2_insufficient_history_members"].as_array_mut() {
            for member in members {
                let affected: Vec<_> = disclosures
                    .iter()
                    .filter(|d| {
                        d["qitem_id"] == member["qitem"]
                            || d["seat"] == "unknown"
                            || d["seat"] == ""
                    })
                    .cloned()
                    .collect();
                if !affected.is_empty() {
                    member["status"] = json!("not reproducible: mutable source");
                    member["mutable_source_disclosures"] = json!(affected);
                }
            }
        }
        payload["mission_tokens"]["status"] = json!("not reproducible: mutable source");
        payload["mission_tokens"]["mutable_source_disclosures"] = json!(disclosures);
    }
    payload["mutable_source_disclosures"] = json!(disclosures);
    Ok(())
}

fn parse_window(from: &str, to: &str) -> Result<Window> {
    let from = chrono::DateTime::parse_from_rfc3339(from)
        .map_err(|_| anyhow!("invalid from timestamp"))?
        .with_timezone(&chrono::Utc);
    let to = chrono::DateTime::parse_from_rfc3339(to)
        .map_err(|_| anyhow!("invalid to timestamp"))?
        .with_timezone(&chrono::Utc);
    if to <= from {
        return Err(anyhow!("report window must be non-empty and half-open"));
    }
    Ok(Window { from, to })
}

pub fn trace_json(
    db: &Path,
    metric_id: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
) -> Result<Value> {
    trace_json_with_thresholds(db, metric_id, rig, from, to, LoopThresholds::default())
}

pub fn trace_json_physical(
    db: &Path,
    metric_id: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
) -> Result<Value> {
    trace_json_with_analysis_physical(
        db,
        metric_id,
        rig,
        from,
        to,
        LoopThresholds::default(),
        crate::FrictionThresholds::default(),
        crate::advisory::AdvisoryOptions::default(),
    )
}

pub fn trace_json_with_thresholds(
    db: &Path,
    metric_id: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
    thresholds: LoopThresholds,
) -> Result<Value> {
    trace_json_with_options(
        db,
        metric_id,
        rig,
        from,
        to,
        thresholds,
        crate::FrictionThresholds::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn trace_json_with_options(
    db: &Path,
    metric_id: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
    thresholds: LoopThresholds,
    friction: crate::FrictionThresholds,
) -> Result<Value> {
    trace_json_with_analysis(
        db,
        metric_id,
        rig,
        from,
        to,
        thresholds,
        friction,
        crate::advisory::AdvisoryOptions::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn trace_json_with_analysis(
    db: &Path,
    metric_id: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
    thresholds: LoopThresholds,
    friction: crate::FrictionThresholds,
    advisory: crate::advisory::AdvisoryOptions,
) -> Result<Value> {
    #[cfg(any(windows, target_os = "linux"))]
    {
        use crate::selection_binding::{resolve_logical_db, SelectedDb};
        match resolve_logical_db(db, &crate::platform_v1::StorageRootChoice::LocalAppData)? {
            SelectedDb::Selected(selected) => trace_json_with_analysis_physical(
                &selected.database,
                metric_id,
                rig,
                from,
                to,
                thresholds,
                friction,
                advisory,
            ),
            SelectedDb::Legacy(legacy) => trace_json_with_analysis_physical(
                &legacy.database,
                metric_id,
                rig,
                from,
                to,
                thresholds,
                friction,
                advisory,
            ),
            SelectedDb::Uninitialized => Err(anyhow!("analytics database is uninitialized")),
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        trace_json_with_analysis_physical(
            db, metric_id, rig, from, to, thresholds, friction, advisory,
        )
    }
}

/// Physical trace read; caller must retain selected custody until it returns.
#[allow(clippy::too_many_arguments)]
pub fn trace_json_with_analysis_physical(
    db: &Path,
    metric_id: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
    thresholds: LoopThresholds,
    friction: crate::FrictionThresholds,
    advisory: crate::advisory::AdvisoryOptions,
) -> Result<Value> {
    let mut evaluation = evaluate_report(
        db,
        "decisions",
        rig,
        from,
        to,
        true,
        thresholds,
        friction,
        &advisory,
        Some(metric_id),
    )?;
    let mut report = evaluation.report;
    let file_source_states = evaluation
        .file_states
        .take()
        .expect("trace snapshot")
        .annotate(
            &mut report["payload"]["trace_inputs"],
            &mut evaluation.loop_trace,
        );
    let payload = &report["payload"];
    let (formula, value) = match metric_id {
        "advisory_idle_claimed" => ("advisory-idle-claimed-v1",payload["advisory_idle_claimed"].clone()),
        "anomaly_spotlights" => ("anomaly-spotlights-v1",payload["anomaly_spotlights"].clone()),
        "handoff_friction" => (crate::vortex::FORMULA_VERSION, payload["handoff_friction"].clone()),
        "m1" => (
            "seat tokens / non-returned deliveries, plus tokens per closed item",
            json!({
                "yield": payload["m1_yield"],
                "by_seat": payload["m1_by_seat"],
                "closed_item": payload["k2_closed_item"],
                "destination_rule": payload["k2_destination_rule"],
                "closed_items_with_insufficient_history": payload["k2_closed_items_with_insufficient_history"],
                "insufficient_history_members": payload["k2_insufficient_history_members"],
            }),
        ),
        "m2" => ("returns / handoffs", payload["m2_return_rate"].clone()),
        "m3" => (
            "rework_chains / chains",
            payload["m3_rework_chains"].clone(),
        ),
        "m4" => {
            if let Some(exposure) = payload.get("m4_skill_exposure") {
                (
                    "impact is insufficient data",
                    json!({
                        "impact": payload["m4_impact"],
                        "exposure": exposure,
                    }),
                )
            } else {
                ("impact is insufficient data", payload["m4_impact"].clone())
            }
        }
        "tokens" => (
            "sum of four token buckets in the half-open window",
            payload["tokens"].clone(),
        ),
        "intake" => (
            "claim-inclusive to the receiver's first structured write or same-item queue transition, exclusive",
            payload["intakes"].clone(),
        ),
        "medians" => (
            "wait seconds = claim timestamp minus handoff timestamp before presentation rounding; odd median = sorted central sample; even median = arithmetic mean of the two central samples; intake samples and medians are in tokens",
            payload["pair_medians"].clone(),
        ),
        "activity" => (
            "five disjoint seat bands clipped to the half-open window",
            payload["activity"].clone(),
        ),
        "fleet_concurrency" => (
            "fleet concurrency distribution across integer active-seat counts k=0..=N with effective parallelism and fleet duty cycle",
            payload["fleet_concurrency"].clone(),
        ),
        "tool_categories" => (
            "ceremony_calls / total_calls across ceremony vs engineering categories",
            payload["tool_categories"].clone(),
        ),
        field @ ("watchdog_recovery" | "ring_boundaries" | "exec_turnaround" | "model_effort_tokens"
            | "four_eyes_independence" | "issue_lineage" | "task_intake_overhead"
            | "context_compactions" | "context_depth_brackets" | "cross_slice_cache_bleed") => (
            "additive Wave 1 projection from accepted facts; definition and source membership accompany each projection",
            payload[field].clone(),
        ),
        other => return Err(anyhow!("unknown metric id {}", redact(other))),
    };
    let mut inputs = json!({
        "file_source_states": file_source_states,
        "tool_loops": payload["tool_loops"],
        "outcome_coverage": payload["outcome_coverage"],
        "outcome_coverage_exclusions": payload["outcome_coverage_exclusions"],
        "tool_loop_trace": evaluation.loop_trace,
        "tokens": payload["tokens"],
        "observed_cost": payload["observed_cost"],
        "estimated_cost": payload["estimated_cost"],
        "cost_rule": payload["cost_rule"],
        "token_categories": payload["token_categories"],
        "seat_token_categories": payload["seat_token_categories"],
        "unlinked_tokens": payload["unlinked_tokens"],
        "unlinked_token_categories": payload["unlinked_token_categories"],
        "sessions": payload["sessions"],
        "reasoning_rule": payload["reasoning_rule"],
        "untimestamped_count": payload["untimestamped_count"],
        "untimestamped_tokens_all_time": payload["untimestamped_tokens_all_time"],
        "m1_yield": payload["m1_yield"],
        "m1_by_seat": payload["m1_by_seat"],
        "k2_closed_item": payload["k2_closed_item"],
        "k2_destination_rule": payload["k2_destination_rule"],
        "k2_closed_items_with_insufficient_history": payload["k2_closed_items_with_insufficient_history"],
        "k2_insufficient_history_members": payload["k2_insufficient_history_members"],
        "m2_pairs": payload["m2_pairs"],
        "m2_return_rate": payload["m2_return_rate"],
        "m3_rework_chains": payload["m3_rework_chains"],
        "m3_chains": payload["m3_chains"],
        "m3_rework_members": payload["m3_rework_members"],
        "m3_unknown_chains": payload["m3_unknown_chains"],
        "m3_unknown_members": payload["m3_unknown_members"],
        "m3_rework_proportion": payload["m3_rework_proportion"],
        "rows": payload["trace_inputs"],
        "intakes": payload["intakes"],
        "pair_medians": payload["pair_medians"],
        "activity": payload["activity"],
        "fleet_concurrency": payload["fleet_concurrency"],
        "tool_categories": payload["tool_categories"],
        "seat_lives": payload["seat_lives"],
        "openrig_sources": payload["openrig_sources"],
        "openrig_row_states": payload["openrig_row_states"],
        "mutable_source_disclosures": payload["mutable_source_disclosures"],
        "links": payload["links"],
        "handoff_edges": payload["handoff_edges"],
        "mission_definition": payload["mission_definition"],
        "mission_tokens": payload["mission_tokens"],
        "mission_ambiguous_by_reason": payload["mission_ambiguous_by_reason"],
        "mission_by_seat": payload["mission_by_seat"],
        "mission_records": payload["mission_records"],
        "activity_events": payload["activity_events"],
        "demand_events": payload["demand_events"],
        "destination_snapshots": payload["destination_snapshots"],
        "nonreturn_handoffs": payload["nonreturn_handoffs"],
        "handoff_members": payload["handoff_members"],
        "window": {"from": from, "to": to},
    });
    if matches!(metric_id, "advisory_idle_claimed" | "anomaly_spotlights") {
        inputs["advisory_idle_claimed"] = payload["advisory_idle_claimed"].clone();
        inputs["anomaly_spotlights"] = payload["anomaly_spotlights"].clone();
        inputs["advisory_trace_inputs"] = payload["advisory_trace_inputs"].clone();
        inputs["advisory_queue_volume"] = payload["advisory_queue_volume"].clone();
    }
    if metric_id == "handoff_friction" {
        inputs["handoff_friction"] = evaluation.friction_replay;
    }
    if !thresholds.is_default() {
        inputs["loop_thresholds"] = json!(thresholds);
        inputs["handoff_loops"] = payload["handoff_loops"].clone();
    }
    for field in [
        "context_compactions",
        "context_depth_brackets",
        "cross_slice_cache_bleed",
        "watchdog_recovery",
        "ring_boundaries",
        "exec_turnaround",
        "model_effort_tokens",
        "four_eyes_independence",
        "issue_lineage",
        "task_intake_overhead",
    ] {
        inputs[field] = payload[field].clone();
    }
    if let Some(exposure) = payload.get("m4_skill_exposure") {
        inputs["m4_skill_exposure"] = exposure.clone();
    }
    Ok(json!({
        "metric_id": metric_id,
        "formula_version": if metric_id == "advisory_idle_claimed" {"advisory-idle-claimed-v1"}
        else if metric_id == "anomaly_spotlights" {"anomaly-spotlights-v1"}
        else if metric_id == "handoff_friction" {
            crate::vortex::FORMULA_VERSION
        } else if metric_id == "medians" {
            "acceptance-r1-a1-wait-fractional-v1"
        } else {
            "acceptance-r1-a1"
        },
        "formula": formula,
        "value": value,
        "inputs": inputs,
        "from": redact(from),
        "to": redact(to),
        "rig": rig.map(redact),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AnalyticsStore;

    // These pre-selection fixtures deliberately create physical DuckDB files to
    // exercise report arithmetic. Windows logical-selection behavior has its
    // own admitted-root integration tests in windows_selection.rs.
    #[cfg(any(windows, target_os = "linux"))]
    fn report_json(
        db: &Path,
        kind: &str,
        rig: Option<&str>,
        from: &str,
        to: &str,
    ) -> Result<Value> {
        super::report_json_physical(db, kind, rig, from, to)
    }
    #[cfg(any(windows, target_os = "linux"))]
    fn trace_json(
        db: &Path,
        metric: &str,
        rig: Option<&str>,
        from: &str,
        to: &str,
    ) -> Result<Value> {
        super::trace_json_physical(db, metric, rig, from, to)
    }
    #[cfg(any(windows, target_os = "linux"))]
    fn report_json_with_thresholds(
        db: &Path,
        kind: &str,
        rig: Option<&str>,
        from: &str,
        to: &str,
        thresholds: LoopThresholds,
    ) -> Result<Value> {
        super::report_json_with_analysis_physical(
            db,
            kind,
            rig,
            from,
            to,
            thresholds,
            crate::FrictionThresholds::default(),
            crate::advisory::AdvisoryOptions::default(),
        )
    }
    #[cfg(any(windows, target_os = "linux"))]
    fn trace_json_with_thresholds(
        db: &Path,
        metric: &str,
        rig: Option<&str>,
        from: &str,
        to: &str,
        thresholds: LoopThresholds,
    ) -> Result<Value> {
        super::trace_json_with_analysis_physical(
            db,
            metric,
            rig,
            from,
            to,
            thresholds,
            crate::FrictionThresholds::default(),
            crate::advisory::AdvisoryOptions::default(),
        )
    }

    #[test]
    fn compaction_depth_and_bleed_roundtrip_reports_traces_and_rig_scope() {
        use chrono::{DateTime, Utc};
        let at = |s| DateTime::<Utc>::from_timestamp(s, 0).unwrap();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!(".test-issue54-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "test").unwrap();
        store
            .insert_identity(&crate::IdentityFact {
                source_key: "identity".into(),
                harness: "codex".into(),
                native_session: "persistent".into(),
                seat: "builder".into(),
                rig: "worker".into(),
                ts: Some(at(0)),
            })
            .unwrap();
        for (id, start, end, mission) in [("a", 1, 8, "A"), ("b", 8, 20, "B")] {
            for (ts, state, key) in [
                (start - 1, "pending", format!("events:queue.created:{id}")),
                (start, "in-progress", format!("claim:{id}")),
                (end, "done", format!("done:{id}")),
            ] {
                store
                    .insert_queue(&crate::QueueFact {
                        source_key: key,
                        transition_id: ts,
                        qitem_id: id.into(),
                        ts: at(ts),
                        state: state.into(),
                        source_seat: "actor".into(),
                        dest_seat: "builder".into(),
                        closure_reason: None,
                        closure_target: None,
                        chain_id: None,
                        missions: vec![mission.into()],
                    })
                    .unwrap();
            }
            store
                .insert_destination_snapshot(
                    id,
                    "builder",
                    &at(end).to_rfc3339(),
                    false,
                    &[mission.into()],
                )
                .unwrap();
        }
        let mut usages: Vec<crate::UsageFact> = [
            (5, 240_000, 200_000),
            (10, 260_000, 200_000),
            (11, 325_000, 300_000),
            (12, 34_000, 30_000),
            (21, 100_000, 90_000),
            (22, 10_000, 5_000),
            (100, 50_000, 0),
        ]
        .into_iter()
        .map(|(seq, context, cache)| {
            serde_json::from_value(json!({
                "source_key": format!("source:1:{seq}"), "harness": "codex",
                "native_session": if seq == 100 { "unlinked" } else { "persistent" },
                "ts": at(if seq == 100 { 10 } else { seq }), "seq": seq,
                "input_uncached": context - cache, "cache_read": cache,
                "cache_write": 0, "output": 1000, "reasoning": 500, "generation": 1
            }))
            .unwrap()
        })
        .collect();
        let mut untimed = usages[0].clone();
        untimed.source_key = "source:1:101".into();
        untimed.seq = 101;
        untimed.ts = None;
        usages.push(untimed);
        store
            .commit_file_batch(
                "source",
                "codex",
                "fixture.jsonl",
                1,
                0,
                "hash",
                None,
                &usages,
                &[],
            )
            .unwrap();
        store.checkpoint().unwrap();
        drop(store);
        let from = at(8).to_rfc3339();
        let to = at(23).to_rfc3339();
        for kind in ["tokens", "decisions"] {
            let report = report_json(&db, kind, None, &from, &to).unwrap();
            let p = &report["payload"];
            assert_eq!(p["tokens"], 785_000);
            assert_eq!(p["unlinked_tokens"], 51_000);
            assert_eq!(p["untimestamped_count"], 1);
            assert_eq!(p["mission_tokens"]["mission:B"], 622_000);
            assert_eq!(p["mission_tokens"]["no_mission"], 112_000);
            assert_eq!(p["task_intake_overhead"]["conserved"], true);
            assert_eq!(p["context_compactions"]["mid_item_hazard_count"], 1);
            assert_eq!(p["context_compactions"]["boundary_reset_count"], 1);
            let event = &p["context_compactions"]["events"][0];
            assert_eq!(event["pre_drop_context_tokens"], 325_000);
            assert_eq!(event["post_drop_context_tokens"], 34_000);
            assert_eq!(event["active_item_id"], "b");
            assert_eq!(event["pre_source_key"], "source:1:11");
            assert_eq!(event["post_source_key"], "source:1:12");
            assert!(event["queue_source_keys"]
                .as_array()
                .unwrap()
                .contains(&json!("claim:b")));
            let d = &p["context_depth_brackets"]["overall"];
            assert_eq!(d["under_100k"]["turn_count"], 3);
            assert_eq!(d["100k_to_250k"]["turn_count"], 1);
            assert_eq!(d["over_250k"]["turn_count"], 2);
            let b = &p["cross_slice_cache_bleed"];
            assert_eq!(b["total_inherited_entry_tokens"], 200_000);
            assert_eq!(b["total_cumulative_bleed_tokens"], 440_000);
            assert!(b["events"][0]["source_keys"]
                .as_array()
                .unwrap()
                .contains(&json!("source:1:5")));
            for metric in [
                "context_compactions",
                "context_depth_brackets",
                "cross_slice_cache_bleed",
            ] {
                let trace = trace_json_with_thresholds(
                    &db,
                    metric,
                    None,
                    &from,
                    &to,
                    LoopThresholds::default(),
                )
                .unwrap();
                assert_eq!(trace["value"], p[metric]);
                for input in [
                    "context_compactions",
                    "context_depth_brackets",
                    "cross_slice_cache_bleed",
                ] {
                    assert_eq!(trace["inputs"][input], p[input]);
                }
            }
        }
        let p = report_json(&db, "tokens", Some("worker"), &from, &to).unwrap();
        assert_eq!(p["payload"]["tokens"], 734_000);
        assert_eq!(
            p["payload"]["context_depth_brackets"]["by_session"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            p["payload"]["context_depth_brackets"]["overall"]["under_100k"]["turn_count"],
            2
        );
        let p = report_json(&db, "tokens", Some("other"), &from, &to).unwrap();
        assert_eq!(p["payload"]["context_compactions"]["events"], json!([]));
        assert_eq!(p["payload"]["cross_slice_cache_bleed"]["events"], json!([]));
        assert_eq!(
            p["payload"]["context_depth_brackets"]["by_session"],
            json!([])
        );
        // A narrow window still detects the drop using its pre-window predecessor.
        let p = report_json(
            &db,
            "tokens",
            None,
            &at(12).to_rfc3339(),
            &at(13).to_rfc3339(),
        )
        .unwrap();
        assert_eq!(
            p["payload"]["context_compactions"]["mid_item_hazard_count"],
            1
        );
        assert_eq!(
            p["payload"]["context_compactions"]["events"][0]["pre_source_key"],
            "source:1:11"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn report_json_does_not_invent_a_yield_for_an_empty_database() {
        let dir = std::env::temp_dir().join(format!("dendro-report-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "kid").unwrap();
        store.checkpoint().unwrap();
        drop(store);
        let report = report_json(
            &db,
            "decisions",
            Some("agentflow-dev"),
            "2026-10-08T00:00:00Z",
            "2026-10-11T00:00:00Z",
        )
        .unwrap();
        assert!(report["payload"]["m1_yield"].is_null());
        assert!(report["payload"]["m4_impact"]
            .as_str()
            .unwrap()
            .contains("insufficient data"));
        assert!(report_json(
            &db,
            "not-a-kind",
            None,
            "2026-10-08T00:00:00Z",
            "2026-10-11T00:00:00Z"
        )
        .is_err());
        assert!(report_json(&db, "tokens", None, "not-a-time", "2026-10-11T00:00:00Z").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn report_and_trace_expose_the_lifecycle_behind_the_bands() {
        use chrono::TimeZone;
        let at = |h: u32| chrono::Utc.with_ymd_and_hms(2026, 10, 9, h, 0, 0).unwrap();
        let dir = std::env::temp_dir().join(format!("dendro-lives-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "kid").unwrap();
        let life = |seat: &str, created: Option<u32>, stopped: Option<u32>| crate::SeatBound {
            seat: seat.into(),
            created_at: created.map(at),
            stopped_at: stopped.map(at),
        };
        store
            .replace_seat_bounds(&[
                life("relaunched", Some(1), Some(2)),
                life("relaunched", Some(3), None),
                life("no-stop", Some(1), None),
                life("stop-only", None, Some(2)),
            ])
            .unwrap();
        store
            .insert_activity(&crate::ActivityFact {
                source_key: "a1".into(),
                seat: "relaunched".into(),
                ts: at(1),
                state: "running".into(),
                seq: 1,
            })
            .unwrap();
        store.checkpoint().unwrap();
        drop(store);
        let (from, to) = ("2026-10-09T00:00:00Z", "2026-10-09T04:00:00Z");
        let report = report_json(&db, "activity", None, from, to).unwrap();
        let rows = report["payload"]["activity"].as_array().unwrap();
        let row = |seat: &str| rows.iter().find(|row| row["seat"] == seat).unwrap().clone();
        let relaunched = row("relaunched");
        assert_eq!(relaunched["observed_secs"], 7200);
        assert_eq!(relaunched["sum"], 7200);
        assert_eq!(relaunched["active_secs"], 3600);
        assert_eq!(
            relaunched["unknown_secs"], 3600,
            "no state carries past the stop"
        );
        assert_eq!(
            relaunched["observed_spans"],
            json!([
                {"from": "2026-10-09T01:00:00+00:00", "to": "2026-10-09T02:00:00+00:00"},
                {"from": "2026-10-09T03:00:00+00:00", "to": "2026-10-09T04:00:00+00:00"},
            ])
        );
        assert_eq!(relaunched["stop_evidence"], "recorded");
        let no_stop = row("no-stop");
        assert_eq!(no_stop["stop_evidence"], "unavailable");
        assert_eq!(no_stop["launch_evidence"], "recorded");
        assert_eq!(no_stop["unknown_secs"], 3 * 3600);
        let stop_only = row("stop-only");
        assert_eq!(stop_only["launch_evidence"], "unavailable");
        assert_eq!(stop_only["observed_secs"], 2 * 3600);
        let trace = trace_json(&db, "activity", None, from, to).unwrap();
        assert_eq!(trace["value"], report["payload"]["activity"]);
        let lives = trace["inputs"]["seat_lives"].as_array().unwrap();
        assert_eq!(lives.len(), 4);
        assert!(lives.contains(&json!({
            "seat": "stop-only",
            "launched_at": null,
            "stopped_at": "2026-10-09T02:00:00+00:00",
        })));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn m1_report_and_trace_carry_k2_verbatim_and_member_evidence() {
        use chrono::TimeZone;
        let at = |h: u32| chrono::Utc.with_ymd_and_hms(2026, 10, 9, h, 0, 0).unwrap();
        let dir = std::env::temp_dir().join(format!("dendro-k2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "kid").unwrap();
        let fact = |key: &str,
                    id,
                    item: &str,
                    h,
                    state: &str,
                    source: &str,
                    dest: &str,
                    reason: Option<&str>| {
            crate::QueueFact {
                source_key: key.into(),
                transition_id: id,
                qitem_id: item.into(),
                ts: at(h),
                state: state.into(),
                source_seat: source.into(),
                dest_seat: dest.into(),
                closure_reason: reason.map(str::to_string),
                closure_target: (state == "done" && reason == Some("handed_off_to"))
                    .then(|| dest.to_string()),
                chain_id: None,
                missions: vec![],
            }
        };
        for row in [
            fact(
                "events:queue.created:1",
                1,
                "x",
                1,
                "pending",
                "orch",
                "seat-a",
                None,
            ),
            fact(
                "queue_transitions:10",
                10,
                "x",
                2,
                "done",
                "seat-a",
                "seat-b",
                Some("handed_off_to"),
            ),
            fact(
                "events:queue.created:2",
                2,
                "y",
                1,
                "pending",
                "orch",
                "seat-a",
                None,
            ),
            fact(
                "queue_transitions:11",
                11,
                "y",
                2,
                "done",
                "seat-c",
                "seat-d",
                Some("handed_off_to"),
            ),
        ] {
            store.insert_queue(&row).unwrap();
        }
        store.checkpoint().unwrap();
        drop(store);
        let (from, to) = ("2026-10-09T00:00:00Z", "2026-10-09T04:00:00Z");
        let register = include_str!("../../../docs/ACCEPTANCE_R1_CLARIFICATIONS.md");
        let report = report_json(&db, "decisions", None, from, to).unwrap();
        let k2 = report["payload"]["k2_closed_item"].as_str().unwrap();
        assert!(
            register.contains(&format!("| {k2} **The M1 report")),
            "K2 text is verbatim"
        );
        let seats = report["payload"]["m1_by_seat"].as_array().unwrap();
        let seat = |name: &str| {
            seats
                .iter()
                .find(|row| row["seat"] == name)
                .unwrap()
                .clone()
        };
        let a = seat("seat-a");
        assert_eq!(
            a["closed_items"], 2,
            "destination, not actor: y was closed by seat-c"
        );
        assert_eq!(
            a["closed_item_keys"],
            json!(["queue_transitions:10", "queue_transitions:11"])
        );
        assert_eq!(
            a["closed_item_members"][1],
            json!({
                "qitem": "y",
                "transition": "queue_transitions:11",
                "at": "2026-10-09T02:00:00+00:00",
                "state": "done",
                "closure_reason": "handed_off_to",
                "destination_evidence": "events:queue.created:2",
                "snapshot_cutoff": null,
            })
        );
        assert_eq!(
            report["payload"]["k2_closed_items_with_insufficient_history"],
            0
        );
        let c = seat("seat-c");
        assert_eq!(c["closed_items"], 0);
        assert_eq!(c["tokens_per_closed_item"], "no data");
        let trace = trace_json(&db, "m1", None, from, to).unwrap();
        assert_eq!(trace["value"]["closed_item"], k2);
        assert_eq!(trace["value"]["closed_items_with_insufficient_history"], 0);
        assert_eq!(
            trace["value"]["destination_rule"],
            report["payload"]["k2_destination_rule"]
        );
        assert_eq!(trace["inputs"]["k2_closed_item"], k2);
        assert_eq!(trace["value"]["by_seat"], report["payload"]["m1_by_seat"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exposed_handoff_sets_reproduce_m1_m2_and_pair_samples() {
        use chrono::TimeZone;
        let at = |m: u32| chrono::Utc.with_ymd_and_hms(2026, 10, 9, 1, m, 0).unwrap();
        let dir = std::env::temp_dir().join(format!("dendro-m1m2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "kid").unwrap();
        let fact = |key: &str, m: u32, item: &str, state: &str, actor: &str, to: &str| {
            let handoff = state == "done";
            crate::QueueFact {
                source_key: key.into(),
                transition_id: m as i64,
                qitem_id: item.into(),
                ts: at(m),
                state: state.into(),
                source_seat: actor.into(),
                dest_seat: to.into(),
                closure_reason: handoff.then(|| "handed_off_to".to_string()),
                closure_target: handoff.then(|| to.to_string()),
                chain_id: Some("i1".into()),
                missions: vec![],
            }
        };
        for row in [
            fact("events:queue.created:1", 0, "i1", "pending", "orch", "A"),
            fact("events:queue.created:2", 9, "i2", "pending", "A", "B"),
            fact("queue_transitions:10", 10, "i1", "done", "A", "B"),
            fact("events:queue.claimed:3", 12, "i2", "in-progress", "B", "B"),
            fact("events:queue.created:4", 19, "i3", "pending", "B", "A"),
            fact("queue_transitions:20", 20, "i2", "done", "B", "A"),
            fact("events:queue.claimed:5", 26, "i3", "in-progress", "A", "A"),
            // Returned after the window: the A delivery of i3 stays not returned (K6).
            fact("queue_transitions:40", 40, "i3", "done", "A", "B"),
        ] {
            store.insert_queue(&row).unwrap();
        }
        store.checkpoint().unwrap();
        drop(store);
        let (from, to) = ("2026-10-09T01:00:00Z", "2026-10-09T01:30:00Z");
        let report = report_json(&db, "decisions", None, from, to).unwrap();
        let p = &report["payload"];
        let members = p["handoff_members"].as_array().unwrap();
        assert_eq!(members.len(), 2);
        let returns = members.iter().filter(|m| m["is_return"] == true).count();
        assert_eq!(returns, 1, "B->A returns to A across items");
        assert_eq!(p["m2_return_rate"], json!(0.5));
        let returned: Vec<_> = members
            .iter()
            .filter(|m| m["returned_within_window"] == true)
            .map(|m| m["source_key"].as_str().unwrap())
            .collect();
        assert_eq!(returned, vec!["queue_transitions:10"]);
        let nonreturn: Vec<_> = p["nonreturn_handoffs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["source_key"].as_str().unwrap())
            .collect();
        assert_eq!(nonreturn, vec!["queue_transitions:20"]);
        let seats = p["m1_by_seat"].as_array().unwrap();
        let deliveries = |seat: &str| {
            seats.iter().find(|row| row["seat"] == seat).unwrap()["nonreturn_deliveries"].clone()
        };
        assert_eq!(
            deliveries("A"),
            0,
            "A's delivery came back inside the window"
        );
        assert_eq!(
            deliveries("B"),
            1,
            "B's return to A is not answered inside the window"
        );
        let medians = p["pair_medians"].as_array().unwrap();
        let ab = medians.iter().find(|row| row["from"] == "A").unwrap();
        assert_eq!(ab["wait_samples"], json!([120.0]));
        assert_eq!(
            ab["wait_sample_keys"],
            json!([["queue_transitions:10", "events:queue.claimed:3"]])
        );
        let trace = trace_json(&db, "m2", None, from, to).unwrap();
        assert_eq!(trace["inputs"]["handoff_members"], p["handoff_members"]);
        assert_eq!(
            trace["inputs"]["nonreturn_handoffs"],
            p["nonreturn_handoffs"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fractional_waits_agree_across_public_reports_and_traces() {
        let dir =
            std::env::temp_dir().join(format!("dendro-fractional-waits-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "kid").unwrap();
        // Literal persisted endpoints independently imply [0, 1us], median 0.5us.
        for (index, at, state) in [
            (0, "2026-10-14T00:00:00Z", "done"),
            (0, "2026-10-14T00:00:00Z", "in-progress"),
            (1, "2026-10-14T00:00:01Z", "done"),
            (1, "2026-10-14T00:00:01.000001Z", "in-progress"),
        ] {
            let handoff = state == "done";
            store
                .insert_queue(&crate::QueueFact {
                    source_key: format!("{state}-{index}"),
                    transition_id: index * 2 + i64::from(!handoff),
                    qitem_id: format!("item-{index}"),
                    ts: chrono::DateTime::parse_from_rfc3339(at)
                        .unwrap()
                        .with_timezone(&chrono::Utc),
                    state: state.into(),
                    source_seat: if handoff { "A" } else { "B" }.into(),
                    dest_seat: "B".into(),
                    closure_reason: handoff.then(|| "handed_off_to".into()),
                    closure_target: handoff.then(|| "B".into()),
                    chain_id: None,
                    missions: vec![],
                })
                .unwrap();
        }
        store.checkpoint().unwrap();
        drop(store);
        let before = fs::read(&db).unwrap();
        let (from, to) = ("2026-10-14T00:00:00Z", "2026-10-14T00:01:00Z");
        let report = report_json(&db, "handoffs", None, from, to).unwrap();
        let pairs = &report["payload"]["pair_medians"];
        assert_eq!(pairs.as_array().unwrap().len(), 1);
        assert_eq!(pairs[0]["wait_samples"], json!([0.0, 0.000001]));
        assert_eq!(pairs[0]["wait_median"], json!(0.0000005));
        assert_eq!(
            pairs[0]["wait_sample_keys"],
            json!([["done-0", "in-progress-0"], ["done-1", "in-progress-1"]])
        );
        assert_eq!(pairs[0]["wait_invalid"], 0);
        for kind in ["links", "activity", "handoffs", "tokens", "decisions"] {
            assert_eq!(
                report_json(&db, kind, None, from, to).unwrap()["payload"]["pair_medians"],
                *pairs
            );
        }
        for metric in [
            "m1", "m2", "m3", "m4", "tokens", "intake", "medians", "activity",
        ] {
            let trace = trace_json(&db, metric, None, from, to).unwrap();
            assert_eq!(trace["inputs"]["pair_medians"], *pairs);
            if metric == "medians" {
                assert_eq!(trace["value"], *pairs);
                assert_eq!(
                    trace["formula_version"],
                    "acceptance-r1-a1-wait-fractional-v1"
                );
            } else {
                assert_eq!(trace["formula_version"], "acceptance-r1-a1");
            }
        }
        assert_eq!(
            fs::read(&db).unwrap(),
            before,
            "report-only reads preserve database bytes"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_rows_conserve_categories_seats_unlinked_and_rig_scope() {
        use chrono::TimeZone;
        let at = |m: u32| chrono::Utc.with_ymd_and_hms(2026, 10, 9, 1, m, 0).unwrap();
        let dir = std::env::temp_dir().join(format!("dendro-sessions-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "kid").unwrap();
        #[allow(clippy::too_many_arguments)]
        let usage = |source: &str,
                     seq: i64,
                     harness: &str,
                     session: &str,
                     ts: Option<chrono::DateTime<chrono::Utc>>,
                     buckets: [i64; 5]| crate::UsageFact {
            source_key: format!("{source}:1:{seq}"),
            harness: harness.into(),
            native_session: session.into(),
            message_id: Some(format!("m{seq}")),
            ts,
            seq,
            input_uncached: buckets[0],
            output: buckets[1],
            cache_read: buckets[2],
            cache_write: buckets[3],
            reasoning: buckets[4],
            currency_total: None,
            generation: 1,
            predecessor_source_key: None,
            predecessor_reason: None,
            prior_input_tokens: None,
            prior_cached_input_tokens: None,
            prior_cache_write_input_tokens: None,
            prior_output_tokens: None,
            prior_reasoning_output_tokens: None,
        };
        let batches: Vec<(&str, &str, Vec<crate::UsageFact>)> = vec![
            (
                "sc",
                "claude",
                vec![
                    usage("sc", 1, "claude", "c1", Some(at(0)), [1, 2, 3, 4, 0]), // at `from`: in
                    usage("sc", 2, "claude", "c1", Some(at(30)), [10, 20, 30, 40, 0]),
                    usage(
                        "sc",
                        3,
                        "claude",
                        "c1",
                        Some(chrono::Utc.with_ymd_and_hms(2026, 10, 9, 2, 0, 0).unwrap()),
                        [500, 0, 0, 0, 0],
                    ), // at `to`: out
                    usage("sc", 4, "claude", "c1", None, [6, 0, 4, 0, 0]),
                ],
            ),
            (
                "sx",
                "codex",
                vec![usage("sx", 1, "codex", "x1", Some(at(5)), [3, 7, 11, 0, 5])],
            ),
            (
                "sp",
                "pi",
                vec![
                    usage("sp", 1, "pi", "p1", Some(at(5)), [100, 1, 0, 0, 1]),
                    usage("sp", 2, "pi", "p1", Some(at(6)), [0, 4, 0, 0, 2]),
                    usage("sp", 3, "pi", "p2", Some(at(7)), [1000, 0, 0, 0, 0]),
                    usage("sp", 4, "pi", "p3", Some(at(8)), [9, 0, 0, 0, 0]),
                    usage("sp", 5, "pi", "p3", None, [70, 0, 0, 0, 0]),
                ],
            ),
        ];
        for (source, harness, rows) in &batches {
            store
                .commit_file_batch(source, harness, "p", 1, 0, "h", None, rows, &[])
                .unwrap();
        }
        // A zero-usage Pi file discovered under its header session.
        store
            .commit_file_batch("zf", "pi", "p", 1, 0, "h", None, &[], &[])
            .unwrap();
        store.set_source_session("zf", "pz").unwrap();
        for (key, harness, session, rig, seat) in [
            ("i1", "claude", "c1", "r", "a"),
            ("i2", "claude", "c1", "r", "a"), // agrees: one binding
            ("i3", "codex", "x1", "r", "b"),
            ("i4", "pi", "p1", "r", "a"), // disagrees: collision
            ("i5", "pi", "p1", "r", "b"),
            ("i6", "pi", "p3", "other", "c"),
            ("i7", "claude", "c9", "r", "a"), // identity only
            ("i8", "pi", "pz", "r", "b"),
        ] {
            store
                .insert_identity(&crate::IdentityFact {
                    source_key: key.into(),
                    harness: harness.into(),
                    native_session: session.into(),
                    rig: rig.into(),
                    seat: seat.into(),
                    ts: Some(at(0)),
                })
                .unwrap();
        }
        store.checkpoint().unwrap();
        drop(store);
        let (from, to) = ("2026-10-09T01:00:00Z", "2026-10-09T02:00:00Z");
        let session_sum = |rows: &[Value], field: &str, keep: &dyn Fn(&Value) -> bool| -> i64 {
            rows.iter()
                .filter(|row| keep(row))
                .map(|row| row[field].as_i64().unwrap())
                .sum()
        };

        let report = report_json(&db, "tokens", None, from, to).unwrap();
        let p = &report["payload"];
        let rows = p["sessions"].as_array().unwrap().clone();
        let find = |session: &str| {
            rows.iter()
                .find(|row| row["session"] == session)
                .unwrap()
                .clone()
        };
        // Every discovered session has one row, including zero usage.
        let mut names: Vec<_> = rows
            .iter()
            .map(|row| row["session"].as_str().unwrap().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["c1", "c9", "p1", "p2", "p3", "pz", "x1"]);
        assert_eq!(find("c1")["status"], "linked");
        assert_eq!(find("c1")["seat"], "a");
        assert_eq!(find("p1")["status"], "collision");
        assert_eq!(find("p1")["seat"], Value::Null);
        assert_eq!(find("p2")["status"], "unlinked");
        assert_eq!(find("c9")["discovered_by"], json!(["identity"]));
        assert_eq!(
            find("pz")["discovered_by"],
            json!(["identity", "zero-usage file"])
        );
        assert_eq!(find("pz")["tokens"], 0);
        // Half-open window: the row at `from` counts, the row at `to` does not.
        assert_eq!(find("c1")["input_uncached"], 11);
        assert_eq!(find("c1")["tokens"], 110);
        assert_eq!(find("c1")["period_records"], 2);
        // Untimestamped records stay session-only, outside every period figure.
        assert_eq!(find("c1")["untimestamped"]["records"], 1);
        assert_eq!(find("c1")["untimestamped"]["tokens"], 10);
        assert_eq!(p["untimestamped_count"], 2);
        assert_eq!(p["untimestamped_tokens_all_time"], 80);
        // Reasoning is shown apart and never added; Claude does not record it.
        assert_eq!(find("c1")["reasoning"], "not recorded");
        assert_eq!(find("x1")["reasoning"], 5);
        assert_eq!(find("x1")["tokens"], 21);
        assert_eq!(find("p1")["reasoning"], 3);
        assert_eq!(find("p1")["tokens"], 105);
        // Conservation: sessions = total; non-linked sessions = unlinked total.
        let all = |_: &Value| true;
        assert_eq!(
            session_sum(&rows, "tokens", &all),
            p["tokens"].as_i64().unwrap()
        );
        assert_eq!(p["tokens"], 110 + 21 + 105 + 1000 + 9);
        for field in ["input_uncached", "output", "cache_read", "cache_write"] {
            assert_eq!(
                session_sum(&rows, field, &all),
                p["token_categories"][field].as_i64().unwrap(),
                "{field}"
            );
        }
        assert_eq!(p["token_categories"]["reasoning"], 8);
        assert_eq!(p["observed_cost"]["status"], "unknown");
        assert_eq!(p["estimated_cost"]["status"], "unknown");
        assert_eq!(p["observed_cost"]["usd"], Value::Null);
        assert_eq!(p["observed_cost"]["unknown_rows"], p["usage_rows"]);
        assert_eq!(
            find("c1")["inclusive"]["label"],
            "inclusive (with sub-agents)"
        );
        assert_eq!(find("c1")["inclusive"]["non_additive"], true);
        assert_eq!(find("c1")["inclusive"]["tokens"], find("c1")["tokens"]);
        let not_linked = |row: &Value| row["status"] != "linked";
        assert_eq!(
            session_sum(&rows, "tokens", &not_linked),
            p["unlinked_tokens"].as_i64().unwrap()
        );
        assert_eq!(p["unlinked_tokens"], 1105);
        assert_eq!(p["unlinked_token_categories"]["collision"]["tokens"], 105);
        // Linked sessions per seat = seat categories = M1 seat tokens = mission total.
        for seat in ["a", "b", "c"] {
            let mine = |row: &Value| row["status"] == "linked" && row["seat"] == seat;
            let sessions = session_sum(&rows, "tokens", &mine);
            let categories = p["seat_token_categories"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["seat"] == seat)
                .unwrap();
            assert_eq!(categories["tokens"].as_i64().unwrap(), sessions, "{seat}");
            for field in ["input_uncached", "output", "cache_read", "cache_write"] {
                assert_eq!(
                    session_sum(&rows, field, &mine),
                    categories[field].as_i64().unwrap(),
                    "{seat} {field}"
                );
            }
            let m1 = p["m1_by_seat"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["seat"] == seat)
                .unwrap();
            assert_eq!(m1["tokens"].as_i64().unwrap(), sessions, "{seat}");
            let mission = p["mission_by_seat"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["seat"] == seat)
                .unwrap();
            assert_eq!(mission["total"].as_i64().unwrap(), sessions, "{seat}");
        }
        // The collision never reaches a seat through one candidate identity.
        let seat_a = p["m1_by_seat"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["seat"] == "a")
            .unwrap();
        assert_eq!(seat_a["tokens"], 110);
        // Report and trace agree.
        let trace = trace_json(&db, "tokens", None, from, to).unwrap();
        for field in [
            "sessions",
            "token_categories",
            "seat_token_categories",
            "unlinked_tokens",
            "untimestamped_count",
        ] {
            assert_eq!(trace["inputs"][field], p[field], "{field}");
        }

        // Rig scope: only sessions singly linked to the rig, with their untimestamped rows.
        let scoped = report_json(&db, "tokens", Some("r"), from, to).unwrap();
        let q = &scoped["payload"];
        let rows = q["sessions"].as_array().unwrap().clone();
        let mut names: Vec<_> = rows
            .iter()
            .map(|row| row["session"].as_str().unwrap().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["c1", "c9", "pz", "x1"]);
        assert_eq!(q["tokens"], 131);
        assert_eq!(session_sum(&rows, "tokens", &all), 131);
        let seats: i64 = q["seat_token_categories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["tokens"].as_i64().unwrap())
            .sum();
        assert_eq!(seats, 131);
        assert_eq!(q["unlinked_tokens"], 0);
        assert_eq!(q["untimestamped_count"], 1);
        assert_eq!(q["untimestamped_tokens_all_time"], 10);
        let other = report_json(&db, "tokens", Some("other"), from, to).unwrap();
        assert_eq!(other["payload"]["tokens"], 9);
        assert_eq!(other["payload"]["untimestamped_count"], 1);
        assert_eq!(other["payload"]["untimestamped_tokens_all_time"], 70);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mission_bands_conserve_per_seat_with_reasons_and_separate_counts() {
        use chrono::TimeZone;
        let at = |m: u32| chrono::Utc.with_ymd_and_hms(2026, 10, 9, 1, m, 0).unwrap();
        let dir = std::env::temp_dir().join(format!("dendro-missions-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "kid").unwrap();
        let usage = |key: &str, session: &str, m: Option<u32>, tokens: i64| crate::UsageFact {
            source_key: key.into(),
            harness: "claude".into(),
            native_session: session.into(),
            message_id: Some(key.into()),
            ts: m.map(at),
            seq: 0,
            input_uncached: tokens,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            currency_total: None,
            generation: 0,
            predecessor_source_key: None,
            predecessor_reason: None,
            prior_input_tokens: None,
            prior_cached_input_tokens: None,
            prior_cache_write_input_tokens: None,
            prior_output_tokens: None,
            prior_reasoning_output_tokens: None,
        };
        store
            .commit_file_batch(
                "src",
                "claude",
                "p",
                0,
                0,
                "h",
                None,
                &[
                    usage("u-mission", "s-build", Some(5), 100), // one in progress: mission
                    usage("u-conflict", "s-build", Some(15), 10), // two missions: ambiguous
                    usage("u-none", "s-build", Some(25), 1),     // closed: no mission
                    usage("u-history", "s-qa", Some(5), 7),      // claim without creation
                    usage("u-unlinked", "s-nobody", Some(5), 1000),
                    usage("u-untimed", "s-build", None, 50),
                ],
                &[],
            )
            .unwrap();
        for (session, seat) in [("s-build", "dev.build"), ("s-qa", "dev.qa")] {
            store
                .insert_identity(&crate::IdentityFact {
                    source_key: format!("id-{session}"),
                    harness: "claude".into(),
                    native_session: session.into(),
                    rig: "r".into(),
                    seat: seat.into(),
                    ts: Some(at(0)),
                })
                .unwrap();
        }
        let fact = |key: &str, m: u32, item: &str, state: &str, seat: &str| crate::QueueFact {
            source_key: key.into(),
            transition_id: m as i64,
            qitem_id: item.into(),
            ts: at(m),
            state: state.into(),
            source_seat: seat.into(),
            dest_seat: seat.into(),
            closure_reason: None,
            closure_target: None,
            chain_id: None,
            missions: vec![],
        };
        for row in [
            fact("events:queue.created:1", 0, "i1", "pending", "dev.build"),
            fact(
                "events:queue.claimed:2",
                1,
                "i1",
                "in-progress",
                "dev.build",
            ),
            fact("events:queue.created:3", 0, "i2", "pending", "dev.build"),
            fact(
                "events:queue.claimed:4",
                10,
                "i2",
                "in-progress",
                "dev.build",
            ),
            fact("queue_transitions:5", 20, "i1", "done", "dev.build"),
            fact("queue_transitions:6", 20, "i2", "done", "dev.build"),
            fact("events:queue.claimed:7", 1, "old", "in-progress", "dev.qa"),
        ] {
            store.insert_queue(&row).unwrap();
        }
        for (item, seat, mission) in [
            ("i1", "dev.build", "one"),
            ("i2", "dev.build", "two"),
            ("old", "dev.qa", "one"),
        ] {
            store
                .insert_destination_snapshot(item, seat, "c", false, &[mission.to_string()])
                .unwrap();
        }
        store.checkpoint().unwrap();
        drop(store);
        let report = report_json(
            &db,
            "decisions",
            None,
            "2026-10-09T01:00:00Z",
            "2026-10-09T02:00:00Z",
        )
        .unwrap();
        let p = &report["payload"];
        assert_eq!(
            p["mission_tokens"],
            json!({"mission:one": 100, "ambiguous": 17, "no_mission": 1, "unlinked": 1000})
        );
        assert_eq!(
            p["mission_ambiguous_by_reason"],
            json!({"conflicting missions": 10, "insufficient history": 7})
        );
        let seats = p["mission_by_seat"].as_array().unwrap();
        let build = seats.iter().find(|row| row["seat"] == "dev.build").unwrap();
        assert_eq!(build["total"], 111);
        assert_eq!(build["conserved"], true);
        assert_eq!(
            build["bands"],
            json!({"mission:one": 100, "ambiguous": 10, "no_mission": 1})
        );
        let qa = seats.iter().find(|row| row["seat"] == "dev.qa").unwrap();
        assert_eq!(
            qa["ambiguous_by_reason"],
            json!({"insufficient history": 7})
        );
        assert!(seats.iter().all(|row| row["conserved"] == true));
        assert_eq!(
            p["untimestamped_tokens_all_time"], 50,
            "untimed usage is outside every window"
        );
        assert_eq!(p["untimestamped_count"], 1);
        // The trace keeps one evidence record per linked in-window usage
        // record (unlinked usage is counted apart), and the bands and primary
        // shares recompute exactly from those records.
        let trace = trace_json(
            &db,
            "tokens",
            None,
            "2026-10-09T01:00:00Z",
            "2026-10-09T02:00:00Z",
        )
        .unwrap();
        let inputs = &trace["inputs"];
        assert_eq!(inputs["mission_definition"], p["mission_definition"]);
        assert!(p["mission_definition"]
            .as_str()
            .unwrap()
            .contains("insufficient history > detectable tag change > conflicting missions"));
        let records = inputs["mission_records"].as_array().unwrap();
        assert_eq!(records.len(), 4);
        let mut bands = std::collections::BTreeMap::<String, i64>::new();
        let mut shares = std::collections::BTreeMap::<String, i64>::new();
        for record in records {
            let tokens = record["tokens"].as_i64().unwrap();
            let parts: i64 = ["input_uncached", "output", "cache_read", "cache_write"]
                .iter()
                .map(|k| record[*k].as_i64().unwrap())
                .sum();
            assert_eq!(parts, tokens, "categories sum to the record total");
            *bands
                .entry(record["bucket"].as_str().unwrap().to_string())
                .or_default() += tokens;
            if let Some(primary) = record["primary_reason"].as_str() {
                *shares.entry(primary.to_string()).or_default() += tokens;
            }
        }
        let mut expected = p["mission_tokens"].as_object().unwrap().clone();
        expected.remove("unlinked");
        assert_eq!(json!(bands), json!(expected));
        assert_eq!(json!(shares), p["mission_ambiguous_by_reason"]);
        assert_eq!(
            shares.values().sum::<i64>(),
            p["mission_tokens"]["ambiguous"].as_i64().unwrap()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_roots_fail_before_a_database_is_created() {
        let dir = std::env::temp_dir().join(format!("dendro-empty-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let err = run_locked(&db, &[], |_, _| Ok(vec![])).unwrap_err();
        assert!(err.to_string().contains("source root"));
        assert!(!db.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    fn scratch(name: &str) -> PathBuf {
        #[cfg(any(windows, target_os = "linux"))]
        {
            crate::platform_v1::create_user_private_root_with_choice(
                &format!("dendro-s31-{name}-"),
                &crate::platform_v1::StorageRootChoice::UserProfile,
            )
            .unwrap()
            .path()
            .to_path_buf()
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            let dir = std::env::temp_dir().join(format!(
                "dendro-s31-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            dir
        }
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"openrig").unwrap();
    }

    fn bind_openrig(db: &Path, paths: &[&Path]) {
        let store = AnalyticsStore::open_path(db, "kid").unwrap();
        for (index, path) in paths.iter().enumerate() {
            store
                .commit_file_batch(
                    &format!("src-{index}"),
                    "openrig",
                    &path.display().to_string(),
                    0,
                    1,
                    "rev",
                    None,
                    &[],
                    &[],
                )
                .unwrap();
        }
        store.checkpoint().unwrap();
    }

    fn publish_openrig(db: &Path, source: &Path) {
        let source = source.to_path_buf();
        run_locked_with_openrig(
            db,
            std::slice::from_ref(&source),
            std::slice::from_ref(&source),
            |staged, _key| {
                let store = AnalyticsStore::open_staged(staged)?;
                store.commit_file_batch(
                    "src-openrig",
                    "openrig",
                    &source.display().to_string(),
                    0,
                    1,
                    "rev",
                    None,
                    &[],
                    &[],
                )?;
                Ok(vec!["bound".into()])
            },
        )
        .unwrap();
    }

    #[test]
    fn two_distinct_openrig_roots_do_not_create_a_database() {
        let dir = scratch("two");
        let first = dir.join("a.sqlite");
        let second = dir.join("b.sqlite");
        touch(&first);
        touch(&second);
        let db = dir.join("analytics.duckdb");
        let mut called = false;
        let err = run_locked_with_openrig(
            &db,
            &[first.clone(), second.clone()],
            &[first, second],
            |_, _| {
                called = true;
                Ok(vec![])
            },
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("v0.1.0 supports exactly one OpenRig root per analytics database"));
        assert!(!called);
        assert!(!db.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_different_bound_root_is_refused_and_the_same_file_is_not() {
        let dir = scratch("bound");
        let bound = dir.join("bound.sqlite");
        let other = dir.join("other.sqlite");
        touch(&bound);
        touch(&other);
        let db = dir.join("analytics.duckdb");
        publish_openrig(&db, &bound);
        let before = fs::read(&db).unwrap();
        let mut called = false;
        let err = run_locked_with_openrig(
            &db,
            std::slice::from_ref(&other),
            std::slice::from_ref(&other),
            |_, _| {
                called = true;
                Ok(vec![])
            },
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(message
            .contains("refusing second OpenRig root: analytics database is bound to OpenRig root"));
        assert!(message.contains(&bound.display().to_string()));
        assert!(message.contains("refused"));
        assert!(message.contains(&other.display().to_string()));
        assert!(message.contains("fresh analytics database"));
        assert!(!called);
        assert_eq!(fs::read(&db).unwrap(), before);

        let link = dir.join("bound-link.sqlite");
        std::os::unix::fs::symlink(&bound, &link).unwrap();
        let spelled = dir.join("extra").join("..").join("bound.sqlite");
        for alias in [link, spelled] {
            let mut saw = false;
            run_locked_with_openrig(
                &db,
                std::slice::from_ref(&alias),
                std::slice::from_ref(&alias),
                |_, _| {
                    saw = true;
                    Ok(vec!["same".into()])
                },
            )
            .unwrap();
            assert!(saw, "same file must not be refused");
        }
        let aligned =
            align_openrig_spelling(&db, &dir.join("extra").join("..").join("bound.sqlite"))
                .unwrap();
        assert!(aligned.is_absolute());
        assert_eq!(aligned, absolute_openrig_root(&bound));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_without_openrig_still_runs_and_a_mixed_database_is_refused() {
        #[cfg(any(windows, target_os = "linux"))]
        let fixture_root = crate::platform_v1::create_user_private_root_with_choice(
            "dendro-pipeline-run-",
            &crate::platform_v1::StorageRootChoice::UserProfile,
        )
        .expect("explicit UserProfile pipeline run fixture");
        #[cfg(any(windows, target_os = "linux"))]
        let dir = fixture_root.path().to_path_buf();
        #[cfg(not(any(windows, target_os = "linux")))]
        let dir = scratch("mixed");
        let claude = dir.join("claude");
        fs::create_dir_all(&claude).unwrap();
        let db = dir.join("analytics.duckdb");
        run_locked_with_openrig(&db, std::slice::from_ref(&claude), &[], |staged, _key| {
            let store = AnalyticsStore::open_staged(staged)?;
            store.commit_file_batch(
                "claude-1",
                "claude",
                &claude.display().to_string(),
                0,
                1,
                "rev",
                None,
                &[],
                &[],
            )?;
            Ok(vec!["claude".into()])
        })
        .unwrap();
        run_locked_with_openrig(&db, std::slice::from_ref(&claude), &[], |_, _| {
            Ok(vec!["again".into()])
        })
        .unwrap();

        let first = dir.join("one.sqlite");
        let second = dir.join("two.sqlite");
        touch(&first);
        touch(&second);
        let mixed = dir.join("mixed.duckdb");
        bind_openrig(&mixed, &[&first, &second]);
        let before = fs::read(&mixed).unwrap();
        let mut called = false;
        let err = run_locked_with_openrig(
            &mixed,
            std::slice::from_ref(&first),
            std::slice::from_ref(&first),
            |_, _| {
                called = true;
                Ok(vec![])
            },
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("refusing second OpenRig root"),
            "{err:#}"
        );
        assert!(err.to_string().contains(&first.display().to_string()));
        assert!(err.to_string().contains(&second.display().to_string()));
        assert!(!called);
        assert_eq!(fs::read(&mixed).unwrap(), before);
        assert!(super::report_json(
            &mixed,
            "decisions",
            None,
            "2026-10-08T00:00:00Z",
            "2026-10-11T00:00:00Z",
        )
        .is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_relative_binding_stays_the_same_file_after_the_working_directory_changes() {
        #[cfg(any(windows, target_os = "linux"))]
        let fixture_root = crate::platform_v1::create_user_private_root_with_choice(
            "dendro-pipeline-binding-",
            &crate::platform_v1::StorageRootChoice::UserProfile,
        )
        .expect("explicit UserProfile pipeline binding fixture");
        #[cfg(any(windows, target_os = "linux"))]
        let dir = fixture_root.path().to_path_buf();
        #[cfg(not(any(windows, target_os = "linux")))]
        let dir = scratch("cwd");
        let nested = dir.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let file = nested.join("openrig.sqlite");
        touch(&file);
        let previous = std::env::current_dir().unwrap();
        struct RestoreCwd(PathBuf);
        impl Drop for RestoreCwd {
            fn drop(&mut self) {
                std::env::set_current_dir(&self.0).expect("restore test working directory");
            }
        }
        let _restore = RestoreCwd(previous.clone());
        std::env::set_current_dir(&dir).unwrap();
        let relative = PathBuf::from("nested/openrig.sqlite");
        let absolute = absolute_openrig_root(&relative);
        assert!(absolute.is_absolute());
        assert_eq!(absolute, fs::canonicalize(&file).unwrap());
        let db = dir.join("analytics.duckdb");
        publish_openrig(&db, &absolute);
        std::env::set_current_dir(std::env::temp_dir()).unwrap();
        // The same relative spelling from another directory is a different path.
        assert_ne!(absolute_openrig_root(&relative), absolute);
        let mut saw = false;
        run_locked_with_openrig(
            &db,
            std::slice::from_ref(&file),
            std::slice::from_ref(&file),
            |_, _| {
                saw = true;
                Ok(vec!["again".into()])
            },
        )
        .unwrap();
        assert!(saw, "the absolute file is still the bound root");
        let other = dir.join("other.sqlite");
        touch(&other);
        let err = run_locked_with_openrig(
            &db,
            std::slice::from_ref(&other),
            std::slice::from_ref(&other),
            |_, _| Ok(vec![]),
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(message.contains(&absolute.display().to_string()));
        assert!(message.contains("refused"));
        assert!(message.contains(&other.display().to_string()));
        #[cfg(any(windows, target_os = "linux"))]
        let selected = match crate::selection_binding::resolve_logical_db(
            &db,
            &crate::platform_v1::StorageRootChoice::UserProfile,
        )
        .unwrap()
        {
            crate::selection_binding::SelectedDb::Selected(read) => read,
            _ => panic!("expected selected database"),
        };
        #[cfg(any(windows, target_os = "linux"))]
        let store = AnalyticsStore::open_read_only_selected(&selected).unwrap();
        #[cfg(not(any(windows, target_os = "linux")))]
        let store = AnalyticsStore::open_read_only(&db).unwrap();
        let stored = store
            .load_sources()
            .unwrap()
            .into_iter()
            .find(|row| row.1 == "openrig")
            .unwrap();
        assert!(Path::new(&stored.2).is_absolute());
        assert_eq!(absolute_openrig_root(Path::new(&stored.2)), absolute);
        std::env::set_current_dir(&previous).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
    #[test]
    fn alias_guard_precedes_output_artifacts_sqlite_and_ingest() {
        let dir = std::env::temp_dir().join(format!("dendro-outer-guard-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let source_dir = dir.join("source");
        fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("source.sqlite");
        let bytes = b"synthetic source, not an analytics schema";
        fs::write(&source, bytes).unwrap();
        for (dest, root, typed) in [
            (source.clone(), source.clone(), true),
            (source_dir.join("source.sqlite-wal"), source.clone(), true),
            (
                source_dir.join("absent/deep/analytics.duckdb"),
                source_dir.clone(),
                false,
            ),
        ] {
            let openrig = if typed { vec![source.clone()] } else { vec![] };
            let err = run_locked_with_openrig(&dest, &[root], &openrig, |_, _| {
                panic!("refused source alias must never reach ingest")
            })
            .unwrap_err();
            assert!(
                err.to_string().contains("aliases a source file")
                    || err.to_string().contains("inside a source root")
            );
            assert_eq!(fs::read(&source).unwrap(), bytes);
            assert_eq!(fs::read_dir(&source_dir).unwrap().count(), 1);
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn pipeline_threshold_validation_rejects_invalid_values() {
        let dir = std::env::temp_dir().join(format!("dendro-val-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "kid").unwrap();
        store.checkpoint().unwrap();
        drop(store);

        let from = "2026-10-08T00:00:00Z";
        let to = "2026-10-11T00:00:00Z";

        // Zero failures
        let zero_failures = LoopThresholds {
            tool_loop_failures: 0,
            tool_loop_window_secs: 600,
            handoff_loop_returns: 2,
        };
        let err = report_json_with_thresholds(&db, "decisions", None, from, to, zero_failures)
            .unwrap_err();
        assert_eq!(err.to_string(), "tool-loop-failures must be at least 1");

        let err_trace =
            trace_json_with_thresholds(&db, "m1", None, from, to, zero_failures).unwrap_err();
        assert_eq!(
            err_trace.to_string(),
            "tool-loop-failures must be at least 1"
        );

        // Negative window seconds
        let neg_window = LoopThresholds {
            tool_loop_failures: 3,
            tool_loop_window_secs: -1,
            handoff_loop_returns: 2,
        };
        let err2 =
            report_json_with_thresholds(&db, "decisions", None, from, to, neg_window).unwrap_err();
        assert_eq!(
            err2.to_string(),
            "tool-loop-window-secs must be non-negative"
        );

        let err2_trace =
            trace_json_with_thresholds(&db, "m1", None, from, to, neg_window).unwrap_err();
        assert_eq!(
            err2_trace.to_string(),
            "tool-loop-window-secs must be non-negative"
        );

        let oob_window = LoopThresholds {
            tool_loop_failures: 3,
            tool_loop_window_secs: i64::MAX,
            handoff_loop_returns: 2,
        };
        let err3 =
            report_json_with_thresholds(&db, "decisions", None, from, to, oob_window).unwrap_err();
        assert_eq!(err3.to_string(), "tool-loop-window-secs is out of bounds");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn m4_skill_exposure_preserves_byte_identity_when_empty_and_reports_exposure_when_present() {
        let dir = scratch("m4-test");
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "kid").unwrap();
        // Insert identities mapping sessions to seats
        store
            .insert_identity(&crate::IdentityFact {
                source_key: "id-1".into(),
                harness: "claude".into(),
                native_session: "sess-claude".into(),
                rig: "rig-1".into(),
                seat: "dev.build".into(),
                ts: None,
            })
            .unwrap();
        store
            .insert_identity(&crate::IdentityFact {
                source_key: "id-2".into(),
                harness: "pi".into(),
                native_session: "sess-pi".into(),
                rig: "rig-1".into(),
                seat: "dev.qa".into(),
                ts: None,
            })
            .unwrap();
        store.checkpoint().unwrap();
        drop(store);

        let (from, to) = ("2026-10-09T00:00:00Z", "2026-10-09T04:00:00Z");

        // 1. When no skills are stored:
        // Report payload has NO m4_skill_exposure key, m4_impact is untouched.
        let report_empty = report_json(&db, "decisions", None, from, to).unwrap();
        let payload_empty = &report_empty["payload"];
        assert!(payload_empty.get("m4_skill_exposure").is_none());
        assert_eq!(payload_empty["m4_impact"], crate::m4_impact());

        // Trace for m4 when empty: formula is "impact is insufficient data", value is string, inputs has no m4_skill_exposure
        let trace_empty = trace_json(&db, "m4", None, from, to).unwrap();
        assert_eq!(trace_empty["formula"], "impact is insufficient data");
        assert_eq!(trace_empty["value"], crate::m4_impact());
        assert!(trace_empty["inputs"].get("m4_skill_exposure").is_none());

        let store = AnalyticsStore::open_path(&db, "kid").unwrap();
        // 2. Insert skills:
        // - OpenRig loaded skills for dev.build (invoked: false)
        store
            .insert_skill(&crate::SkillFact {
                source_key: "snapshots:skill:dev.build:git-tools".into(),
                seat: "dev.build".into(),
                skill: "git-tools".into(),
                invoked: false,
            })
            .unwrap();
        store
            .insert_skill(&crate::SkillFact {
                source_key: "snapshots:skill:dev.build:rust-analyzer".into(),
                seat: "dev.build".into(),
                skill: "rust-analyzer".into(),
                invoked: false,
            })
            .unwrap();
        // - Harness stream invoked skill via native session sess-claude -> dev.build (invoked: true)
        store
            .insert_skill(&crate::SkillFact {
                source_key: "claude-src:0:15:skill:0".into(),
                seat: "claude:sess-claude".into(),
                skill: "git-tools".into(),
                invoked: true,
            })
            .unwrap();
        // Duplicate invocation on dev.build
        store
            .insert_skill(&crate::SkillFact {
                source_key: "claude-src:0:20:skill:0".into(),
                seat: "claude:sess-claude".into(),
                skill: "git-tools".into(),
                invoked: true,
            })
            .unwrap();
        // - Harness stream invoked skill via native session sess-pi -> dev.qa (invoked: true)
        store
            .insert_skill(&crate::SkillFact {
                source_key: "pi-src:0:22:skill:0".into(),
                seat: "pi:sess-pi".into(),
                skill: "test-runner".into(),
                invoked: true,
            })
            .unwrap();
        // - (c) Unlinked and colliding native sessions: must be excluded from m4_skill_exposure
        store
            .insert_skill(&crate::SkillFact {
                source_key: "claude-src:0:99:skill:0".into(),
                seat: "claude:sess-unlinked".into(),
                skill: "orphan-skill".into(),
                invoked: true,
            })
            .unwrap();
        store
            .insert_identity(&crate::IdentityFact {
                source_key: "id-c1".into(),
                harness: "claude".into(),
                native_session: "sess-conflict".into(),
                rig: "rig-1".into(),
                seat: "dev.build".into(),
                ts: None,
            })
            .unwrap();
        store
            .insert_identity(&crate::IdentityFact {
                source_key: "id-c2".into(),
                harness: "claude".into(),
                native_session: "sess-conflict".into(),
                rig: "rig-1".into(),
                seat: "dev.qa".into(),
                ts: None,
            })
            .unwrap();
        store
            .insert_skill(&crate::SkillFact {
                source_key: "claude-src:0:98:skill:0".into(),
                seat: "claude:sess-conflict".into(),
                skill: "conflict-skill".into(),
                invoked: true,
            })
            .unwrap();
        store.checkpoint().unwrap();
        drop(store);

        // 3. Evaluate report with skills
        let report_populated = report_json(&db, "decisions", None, from, to).unwrap();
        let payload = &report_populated["payload"];
        assert_eq!(payload["m4_impact"], crate::m4_impact());
        let exposure = payload["m4_skill_exposure"]
            .as_array()
            .expect("m4_skill_exposure should be present");
        assert_eq!(
            exposure.len(),
            2,
            "unlinked and colliding sessions must be excluded"
        );
        assert!(!exposure
            .iter()
            .any(|row| row["seat"] == "sess-unlinked" || row["seat"] == "sess-conflict"));
        assert!(!exposure.iter().any(|row| {
            let inv = row["invoked_skills"].as_array().unwrap();
            inv.iter()
                .any(|s| s == "orphan-skill" || s == "conflict-skill")
        }));

        let build = exposure
            .iter()
            .find(|row| row["seat"] == "dev.build")
            .expect("dev.build row");
        assert_eq!(
            build["loaded_skills"],
            json!(["git-tools", "rust-analyzer"])
        );
        assert_eq!(build["invoked_skills"], json!(["git-tools"]));
        assert_eq!(
            build["source_keys"],
            json!([
                "claude-src:0:15:skill:0",
                "claude-src:0:20:skill:0",
                "snapshots:skill:dev.build:git-tools",
                "snapshots:skill:dev.build:rust-analyzer"
            ])
        );

        let qa = exposure
            .iter()
            .find(|row| row["seat"] == "dev.qa")
            .expect("dev.qa row");
        assert_eq!(qa["loaded_skills"], json!([] as [&str; 0]));
        assert_eq!(qa["invoked_skills"], json!(["test-runner"]));
        assert_eq!(qa["source_keys"], json!(["pi-src:0:22:skill:0"]));

        // 4. Trace for m4 with skills
        let trace = trace_json(&db, "m4", None, from, to).unwrap();
        assert_eq!(trace["formula"], "impact is insufficient data");
        assert_eq!(
            trace["value"],
            json!({
                "impact": crate::m4_impact(),
                "exposure": exposure
            })
        );
        assert_eq!(trace["inputs"]["m4_skill_exposure"], json!(exposure));

        // 5. Rig scope: scoped to rig-1 includes both seats
        let report_rig1 = report_json(&db, "decisions", Some("rig-1"), from, to).unwrap();
        assert!(report_rig1["payload"]["m4_skill_exposure"]
            .as_array()
            .is_some());

        // Rig scope: scoped to other rig excludes these seats
        let report_other = report_json(&db, "decisions", Some("rig-other"), from, to).unwrap();
        assert!(report_other["payload"].get("m4_skill_exposure").is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pipeline_default_thresholds_produce_byte_identical_json() {
        let dir = std::env::temp_dir().join(format!("dendro-byte-id-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "kid").unwrap();
        store.checkpoint().unwrap();
        drop(store);

        let from = "2026-10-08T00:00:00Z";
        let to = "2026-10-11T00:00:00Z";

        let r_default = report_json(&db, "decisions", None, from, to).unwrap();
        let r_with = report_json_with_thresholds(
            &db,
            "decisions",
            None,
            from,
            to,
            LoopThresholds::default(),
        )
        .unwrap();
        assert_eq!(r_default, r_with);
        assert_eq!(r_default.to_string(), r_with.to_string());
        assert!(r_with["payload"].get("loop_thresholds").is_none());
        assert!(r_with["payload"].get("handoff_loops").is_none());

        let t_default = trace_json(&db, "m1", None, from, to).unwrap();
        let t_with =
            trace_json_with_thresholds(&db, "m1", None, from, to, LoopThresholds::default())
                .unwrap();
        assert_eq!(t_default, t_with);
        assert_eq!(t_default.to_string(), t_with.to_string());
        assert!(t_with["inputs"].get("loop_thresholds").is_none());
        assert!(t_with["inputs"].get("handoff_loops").is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pipeline_custom_thresholds_applied_to_tool_and_handoff_loops() {
        use chrono::TimeZone;
        let at = |s: u32| chrono::Utc.with_ymd_and_hms(2026, 10, 9, 0, 0, s).unwrap();
        let dir = std::env::temp_dir().join(format!("dendro-custom-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "kid").unwrap();

        // Register identity for dev.build
        store
            .insert_identity(&crate::IdentityFact {
                source_key: "id-build".into(),
                harness: "claude".into(),
                native_session: "sess-1".into(),
                rig: "rig-1".into(),
                seat: "dev.build".into(),
                ts: Some(at(0)),
            })
            .unwrap();

        // 2 tool failures for "Bash" within 30 seconds
        let tool1 = crate::ToolFact {
            source_key: "t1".into(),
            harness: "claude".into(),
            native_session: "sess-1".into(),
            ts: Some(at(5)),
            seq: 1,
            name: "Bash".into(),
            fingerprint: Some("fp-cmd".into()),
            failed: Some(true),
            structured_write: false,
            generation: 1,
        };
        let tool2 = crate::ToolFact {
            source_key: "t2".into(),
            harness: "claude".into(),
            native_session: "sess-1".into(),
            ts: Some(at(35)),
            seq: 2,
            name: "Bash".into(),
            fingerprint: Some("fp-cmd".into()),
            failed: Some(true),
            structured_write: false,
            generation: 1,
        };
        store
            .commit_file_batch(
                "src-1",
                "claude",
                "session.json",
                1,
                100,
                "prefix",
                None,
                &[],
                &[tool1, tool2],
            )
            .unwrap();

        // Add handoff edges: dev.build -> dev.qa -> dev.build -> dev.qa -> dev.build
        let q_fact = |key: &str,
                      s: u32,
                      item: &str,
                      state: &str,
                      src: &str,
                      dst: &str,
                      closure_reason: Option<&str>| crate::QueueFact {
            source_key: key.into(),
            transition_id: s as i64,
            qitem_id: item.into(),
            ts: at(s),
            state: state.into(),
            source_seat: src.into(),
            dest_seat: dst.into(),
            closure_reason: closure_reason.map(Into::into),
            closure_target: None,
            chain_id: Some("chain-1".into()),
            missions: vec![],
        };
        store
            .insert_queue(&q_fact(
                "q1",
                1,
                "item-1",
                "closed",
                "dev.build",
                "dev.qa",
                Some("handed_off_to"),
            ))
            .unwrap();
        store
            .insert_queue(&q_fact(
                "q2",
                2,
                "item-1",
                "closed",
                "dev.qa",
                "dev.build",
                Some("handed_off_to"),
            ))
            .unwrap();
        store
            .insert_queue(&q_fact(
                "q3",
                3,
                "item-1",
                "closed",
                "dev.build",
                "dev.qa",
                Some("handed_off_to"),
            ))
            .unwrap();
        store
            .insert_queue(&q_fact(
                "q4",
                4,
                "item-1",
                "closed",
                "dev.qa",
                "dev.build",
                Some("handed_off_to"),
            ))
            .unwrap();

        store.checkpoint().unwrap();
        drop(store);

        let from = "2026-10-08T00:00:00Z";
        let to = "2026-10-11T00:00:00Z";

        // Under defaults:
        // - 2 tool failures do not trigger loop (default failures = 3)
        // - handoff loops are not present
        let rep_def = report_json(&db, "decisions", None, from, to).unwrap();
        assert_eq!(
            rep_def["payload"]["tool_loops"].as_array().unwrap().len(),
            0
        );
        assert!(rep_def["payload"].get("loop_thresholds").is_none());
        assert!(rep_def["payload"].get("handoff_loops").is_none());

        // Under custom thresholds:
        // failures = 2, window_secs = 60, returns = 1
        let custom = LoopThresholds {
            tool_loop_failures: 2,
            tool_loop_window_secs: 60,
            handoff_loop_returns: 1,
        };
        let rep_custom =
            report_json_with_thresholds(&db, "decisions", None, from, to, custom).unwrap();
        let tool_loops = rep_custom["payload"]["tool_loops"].as_array().unwrap();
        assert_eq!(tool_loops.len(), 1);
        assert_eq!(tool_loops[0]["count"], 2);
        assert_eq!(tool_loops[0]["tool"], "Bash");

        assert_eq!(
            rep_custom["payload"]["loop_thresholds"],
            json!({
                "tool_loop_failures": 2,
                "tool_loop_window_secs": 60,
                "handoff_loop_returns": 1
            })
        );
        let handoff_loops = rep_custom["payload"]["handoff_loops"].as_array().unwrap();
        assert!(!handoff_loops.is_empty());
        assert_eq!(handoff_loops[0]["chain"], "chain-1");

        // Trace also contains loop_thresholds, handoff_loops, and custom definition
        let trace_custom = trace_json_with_thresholds(&db, "m1", None, from, to, custom).unwrap();
        assert_eq!(
            trace_custom["inputs"]["loop_thresholds"],
            json!({
                "tool_loop_failures": 2,
                "tool_loop_window_secs": 60,
                "handoff_loop_returns": 1
            })
        );
        assert_eq!(
            trace_custom["inputs"]["handoff_loops"],
            rep_custom["payload"]["handoff_loops"]
        );
        let def = &trace_custom["inputs"]["tool_loop_trace"]["definition"];
        assert_eq!(def["failures"], 2);
        assert_eq!(def["window_seconds"], 60);
        assert_eq!(def["native_span_predicate"], "span<=60");
        assert_eq!(def["codex_span_predicate"], "0<=span<=60");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn m4_skill_exposure_redacts_secret_shaped_skills() {
        let dir = scratch("m4-secret-test");
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "kid").unwrap();
        let secret =
            "sk-ant-api03-abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890-test";
        let sealed_secret = crate::seal_identifier(b"test-key", "skill", secret);
        assert!(sealed_secret.contains("[redacted]"));
        assert!(!sealed_secret.contains(secret));

        store
            .insert_skill(&crate::SkillFact {
                source_key: format!("snapshots:skill:dev.sec:{sealed_secret}"),
                seat: "dev.sec".into(),
                skill: sealed_secret.clone(),
                invoked: false,
            })
            .unwrap();
        store.checkpoint().unwrap();
        drop(store);

        let report = report_json(
            &db,
            "decisions",
            None,
            "2026-10-09T00:00:00Z",
            "2026-10-09T04:00:00Z",
        )
        .unwrap();
        let payload_str = serde_json::to_string(&report["payload"]).unwrap();
        assert!(!payload_str.contains(secret));
        assert!(payload_str.contains("[redacted]"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tool_categories_report_and_trace_end_to_end() {
        use crate::{ToolCategory, ToolFact};

        let dir = std::env::temp_dir().join(format!("dendro-cat-pipeline-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "kid").unwrap();

        let ts1 = chrono::DateTime::parse_from_rfc3339("2026-10-09T01:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let ts2 = chrono::DateTime::parse_from_rfc3339("2026-10-09T02:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let ts3 = chrono::DateTime::parse_from_rfc3339("2026-10-09T03:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        // Setup seat identity
        store
            .insert_identity(&crate::IdentityFact {
                source_key: "id-1".into(),
                ts: Some(ts1),
                harness: "claude".into(),
                native_session: "sess-1".into(),
                seat: "dev.builder".into(),
                rig: "alpha".into(),
            })
            .unwrap();
        store
            .insert_activity(&crate::ActivityFact {
                source_key: "act-1".into(),
                seat: "dev.builder".into(),
                ts: ts1,
                state: "running".into(),
                seq: 1,
            })
            .unwrap();

        // Ingest tool facts
        // 1. Ceremony call (coord_queue)
        let t1 = ToolFact {
            source_key: "src1:1:1:0".into(),
            harness: "claude".into(),
            native_session: "sess-1".into(),
            ts: Some(ts1),
            seq: 100,
            name: "bash".into(),
            fingerprint: None,
            failed: Some(false),
            structured_write: false,
            generation: 1,
        };
        // 2. Engineering call (eng_build_test)
        let t2 = ToolFact {
            source_key: "src1:1:2:0".into(),
            harness: "claude".into(),
            native_session: "sess-1".into(),
            ts: Some(ts2),
            seq: 200,
            name: "bash".into(),
            fingerprint: None,
            failed: Some(false),
            structured_write: false,
            generation: 1,
        };
        // 3. Engineering call (eng_code_edit)
        let t3 = ToolFact {
            source_key: "src1:1:3:0".into(),
            harness: "claude".into(),
            native_session: "sess-1".into(),
            ts: Some(ts3),
            seq: 300,
            name: "Write".into(),
            fingerprint: None,
            failed: Some(false),
            structured_write: true,
            generation: 1,
        };

        let baseline = crate::FileBaselineAttempt {
            attempted_at: "2026-10-09T01:00:00Z".into(),
            metadata: None,
        };
        store
            .commit_file_batch_observed(
                "src1",
                "claude",
                "/path/to/claude.jsonl",
                1,
                1000,
                "sha256",
                None,
                &[],
                &[t1, t2, t3],
                &[],
                &crate::ReaderState {
                    complete: true,
                    native_session: Some("sess-1".into()),
                },
                &baseline,
            )
            .unwrap();

        store
            .record_tool_categories(&[
                ("src1:1:1:0".into(), ToolCategory::CoordQueue),
                ("src1:1:2:0".into(), ToolCategory::EngBuildTest),
                ("src1:1:3:0".into(), ToolCategory::EngCodeEdit),
            ])
            .unwrap();

        store.checkpoint().unwrap();
        drop(store);

        let from = "2026-10-09T00:00:00Z";
        let to = "2026-10-09T04:00:00Z";

        // 1. Report: activity
        let rep_act = report_json(&db, "activity", None, from, to).unwrap();
        let payload_act = &rep_act["payload"];
        let tc_act = &payload_act["tool_categories"];

        assert_eq!(tc_act["ceremony_calls"], 1);
        assert_eq!(tc_act["engineering_calls"], 2);
        assert_eq!(tc_act["total_calls"], 3);
        let ratio = tc_act["ceremony_ratio"].as_f64().unwrap();
        assert!((ratio - (1.0 / 3.0)).abs() < 1e-6);

        // Check categories map has all 8 variants
        for cat in ToolCategory::ALL {
            assert!(
                tc_act["categories"].get(cat.as_str()).is_some(),
                "missing category {}",
                cat.as_str()
            );
        }
        assert_eq!(tc_act["categories"]["coord_queue"]["count"], 1);
        assert_eq!(
            tc_act["categories"]["coord_queue"]["source_keys"],
            json!(["src1:1:1:0"])
        );
        assert_eq!(tc_act["categories"]["eng_build_test"]["count"], 1);
        assert_eq!(
            tc_act["categories"]["eng_build_test"]["source_keys"],
            json!(["src1:1:2:0"])
        );
        assert_eq!(tc_act["categories"]["eng_code_edit"]["count"], 1);
        assert_eq!(
            tc_act["categories"]["eng_code_edit"]["source_keys"],
            json!(["src1:1:3:0"])
        );

        // Check payload["activity"] rows are enriched
        let act_rows = payload_act["activity"].as_array().unwrap();
        assert!(!act_rows.is_empty());
        let builder_row = act_rows
            .iter()
            .find(|r| r["seat"] == "dev.builder")
            .unwrap();
        assert_eq!(builder_row["ceremony_calls"], 1);
        assert_eq!(builder_row["engineering_calls"], 2);
        let seat_ratio = builder_row["ceremony_ratio"].as_f64().unwrap();
        assert!((seat_ratio - (1.0 / 3.0)).abs() < 1e-6);
        assert_eq!(builder_row["tool_categories"]["coord_queue"]["count"], 1);

        // 2. Report: decisions
        let rep_dec = report_json(&db, "decisions", None, from, to).unwrap();
        assert_eq!(rep_dec["payload"]["tool_categories"]["ceremony_calls"], 1);
        assert_eq!(
            rep_dec["payload"]["tool_categories"]["engineering_calls"],
            2
        );

        // 3. Trace: tool_categories
        let trace = trace_json(&db, "tool_categories", None, from, to).unwrap();
        assert_eq!(trace["metric_id"], "tool_categories");
        assert_eq!(trace["value"], rep_act["payload"]["tool_categories"]);
        assert_eq!(
            trace["inputs"]["tool_categories"],
            rep_act["payload"]["tool_categories"]
        );

        // AC7 traceability: source_keys point to exact source_keys
        let trace_keys = trace["value"]["categories"]["eng_build_test"]["source_keys"]
            .as_array()
            .unwrap();
        assert_eq!(trace_keys[0], "src1:1:2:0");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fleet_concurrency_and_container_disambiguation_end_to_end() {
        use crate::{ActivityFact, QueueFact, QueueItemMeta};
        use chrono::TimeZone;
        let at = |s: i64| chrono::Utc.timestamp_opt(1_760_000_000 + s, 0).unwrap();
        let from_str = at(0).to_rfc3339();
        let to_str = at(6000).to_rfc3339();

        let dir = std::env::temp_dir().join(format!("dendro-concurrency-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "test-kid").unwrap();

        // 1. Seat bounds: orch.lead has no stop; dev.build stopped at 5400s
        store
            .replace_seat_bounds(&[
                SeatBound {
                    seat: "orch.lead".into(),
                    created_at: Some(at(0)),
                    stopped_at: Some(at(6000)),
                },
                SeatBound {
                    seat: "dev.build".into(),
                    created_at: Some(at(0)),
                    stopped_at: Some(at(5400)),
                },
            ])
            .unwrap();

        // 2. Activity facts
        // orch.lead:
        // 0..600: idle
        // 600..1800: running (1200s)
        // 1800..4200: idle
        // 4200..4800: running (600s)
        // 4800..6000: idle
        // Total active: 1800s
        for (i, (state, s)) in [
            ("idle", 0),
            ("running", 600),
            ("idle", 1800),
            ("running", 4200),
            ("idle", 4800),
        ]
        .iter()
        .enumerate()
        {
            store
                .insert_activity(&ActivityFact {
                    source_key: format!("act:orch:{s}"),
                    seat: "orch.lead".into(),
                    ts: at(*s),
                    state: (*state).into(),
                    seq: i as i64 + 1,
                })
                .unwrap();
        }

        // dev.build:
        // 0..1200: idle
        // 1200..2400: running (1200s)
        // 2400..3600: idle
        // 3600..5400: running (1800s until stop at 5400)
        // Total active: 3000s
        for (i, (state, s)) in [
            ("idle", 0),
            ("running", 1200),
            ("idle", 2400),
            ("running", 3600),
        ]
        .iter()
        .enumerate()
        {
            store
                .insert_activity(&ActivityFact {
                    source_key: format!("act:dev:{s}"),
                    seat: "dev.build".into(),
                    ts: at(*s),
                    state: (*state).into(),
                    seq: i as i64 + 1,
                })
                .unwrap();
        }

        // 3. Queue facts for container disambiguation
        // orch.lead claims Q-PARENT at 2400s, closed at 4200s
        store
            .insert_queue(&QueueFact {
                source_key: "q:parent:2400".into(),
                transition_id: 1,
                qitem_id: "Q-PARENT".into(),
                ts: at(2400),
                state: "in-progress".into(),
                source_seat: "orch.lead".into(),
                dest_seat: "orch.lead".into(),
                closure_reason: None,
                closure_target: None,
                chain_id: None,
                missions: vec!["mission-alpha".into()],
            })
            .unwrap();
        store
            .insert_queue(&QueueFact {
                source_key: "q:parent:4200".into(),
                transition_id: 4,
                qitem_id: "Q-PARENT".into(),
                ts: at(4200),
                state: "closed".into(),
                source_seat: "orch.lead".into(),
                dest_seat: "orch.lead".into(),
                closure_reason: Some("completed".into()),
                closure_target: None,
                chain_id: None,
                missions: vec!["mission-alpha".into()],
            })
            .unwrap();

        // dev.build claims Q-CHILD at 3000s, closed at 3600s
        store
            .insert_queue(&QueueFact {
                source_key: "q:child:3000".into(),
                transition_id: 2,
                qitem_id: "Q-CHILD".into(),
                ts: at(3000),
                state: "in-progress".into(),
                source_seat: "orch.lead".into(),
                dest_seat: "dev.build".into(),
                closure_reason: None,
                closure_target: None,
                chain_id: None,
                missions: vec!["mission-alpha".into()],
            })
            .unwrap();
        store
            .insert_queue(&QueueFact {
                source_key: "q:child:3600".into(),
                transition_id: 3,
                qitem_id: "Q-CHILD".into(),
                ts: at(3600),
                state: "closed".into(),
                source_seat: "dev.build".into(),
                dest_seat: "dev.build".into(),
                closure_reason: Some("completed".into()),
                closure_target: None,
                chain_id: None,
                missions: vec!["mission-alpha".into()],
            })
            .unwrap();

        // Queue item metadata linking Q-CHILD to Q-PARENT
        store
            .record_queue_item_meta(&QueueItemMeta {
                qitem_id: "Q-CHILD".into(),
                is_recovery: false,
                recovery_reason: None,
                last_nudge_result: None,
                issue_tags: vec!["issue-53".into()],
                phase_tags: vec![],
                parent_qitem_id: Some("Q-PARENT".into()),
            })
            .unwrap();

        store.checkpoint().unwrap();
        drop(store);

        // 4. Test report activity
        let rep_act = report_json(&db, "activity", None, &from_str, &to_str).unwrap();
        let payload_act = &rep_act["payload"];

        // Check fleet_concurrency
        let fc = &payload_act["fleet_concurrency"];
        assert_eq!(fc["window_secs"], 6000);
        assert_eq!(fc["seat_count"], 2);
        assert_eq!(fc["conserved"], true);
        assert_eq!(fc["total_active_seat_secs"], 4800);

        let ep = fc["effective_parallelism"].as_f64().unwrap();
        assert!((ep - 0.80).abs() < 1e-6);
        assert_eq!(fc["effective_parallelism_milli"], 800);

        let dc = fc["fleet_duty_cycle"].as_f64().unwrap();
        assert!((dc - 0.40).abs() < 1e-6);
        assert_eq!(fc["fleet_duty_cycle_bps"], 4000);

        let hist = fc["histogram"].as_array().unwrap();
        assert_eq!(hist.len(), 3);
        assert_eq!(hist[0]["secs"], 2400);
        assert_eq!(hist[1]["secs"], 2400);
        assert_eq!(hist[2]["secs"], 1200);
        assert_eq!(hist[0]["conserved"], true);
        assert_eq!(hist[1]["conserved"], true);
        assert_eq!(hist[2]["conserved"], true);

        // Check activity rows
        let act_rows = payload_act["activity"].as_array().unwrap();
        let orch_row = act_rows.iter().find(|r| r["seat"] == "orch.lead").unwrap();
        assert_eq!(orch_row["active_secs"], 1800);
        assert_eq!(orch_row["idle_with_work_secs"], 1800);
        assert_eq!(orch_row["unparked_container_secs"], 600);
        assert_eq!(orch_row["leaf_stall_secs"], 1200);
        assert_eq!(
            orch_row["unparked_container_secs"].as_i64().unwrap()
                + orch_row["leaf_stall_secs"].as_i64().unwrap(),
            orch_row["idle_with_work_secs"].as_i64().unwrap()
        );
        assert_eq!(orch_row["sum"], 6000);

        // Check container_hold_evidence AC7 provenance
        let evidence = orch_row["container_hold_evidence"].as_array().unwrap();
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0]["parent_qitem_id"], "Q-PARENT");
        assert_eq!(evidence[0]["parent_source_key"], "q:parent:2400");
        assert_eq!(evidence[0]["child_qitem_id"], "Q-CHILD");
        assert_eq!(evidence[0]["child_seat"], "dev.build");
        assert_eq!(evidence[0]["child_source_key"], "q:child:3000");
        assert_eq!(evidence[0]["secs"], 600);

        // dev.build activity
        let dev_row = act_rows.iter().find(|r| r["seat"] == "dev.build").unwrap();
        assert_eq!(dev_row["active_secs"], 3000);
        assert_eq!(dev_row["sum"], 5400); // 5400s observed due to stop_at 5400

        // 5. Test report decisions
        let rep_dec = report_json(&db, "decisions", None, &from_str, &to_str).unwrap();
        assert_eq!(
            rep_dec["payload"]["fleet_concurrency"]["total_active_seat_secs"],
            4800
        );

        // 6. Test trace fleet_concurrency
        let trace = trace_json(&db, "fleet_concurrency", None, &from_str, &to_str).unwrap();
        assert_eq!(trace["metric_id"], "fleet_concurrency");
        assert_eq!(trace["formula_version"], "acceptance-r1-a1");
        assert_eq!(trace["value"], *fc);
        assert_eq!(trace["inputs"]["fleet_concurrency"], *fc);

        let _ = fs::remove_dir_all(&dir);
    }
    #[test]
    fn unrelated_trace_skips_new_parent_metadata_and_friction_capture() {
        let dir = std::env::temp_dir().join(format!("dendro-lazy-replay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("data.duckdb");
        let store = AnalyticsStore::open_path(&db, "fixture").unwrap();
        store.checkpoint().unwrap();
        drop(store);
        crate::storage::FRICTION_METADATA_READS.with(|n| n.set(0));
        let trace = trace_json(
            &db,
            "m1",
            None,
            "2026-10-15T00:00:00Z",
            "2026-10-15T01:00:00Z",
        )
        .unwrap();
        crate::storage::FRICTION_METADATA_READS.with(|n| assert_eq!(n.get(), 0));
        assert!(trace["inputs"].get("handoff_friction").is_none());
        let trace = trace_json(
            &db,
            "handoff_friction",
            None,
            "2026-10-15T00:00:00Z",
            "2026-10-15T01:00:00Z",
        )
        .unwrap();
        crate::storage::FRICTION_METADATA_READS.with(|n| assert_eq!(n.get(), 1));
        assert!(trace["inputs"]["handoff_friction"].is_object());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_pi_origins_openrig_pi_home_both() {
        let dir = std::env::temp_dir().join(format!(
            "dendro-pi-origins-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("test.duckdb");
        let store = AnalyticsStore::open_path(&db, "key").unwrap();

        // 1. Session "pi-openrig-only": source path is OpenRig state, no identity
        store
            .commit_file_batch(
                "s_openrig",
                "pi",
                "/home/user/.openrig/state/pi/session-openrig.jsonl",
                1,
                0,
                "prefix1",
                None,
                &[],
                &[],
            )
            .unwrap();
        store
            .set_source_session("s_openrig", "pi-openrig-only")
            .unwrap();

        // 2. Session "pi-home-only": source path is ~/.pi/agent/sessions, no OpenRig identity
        store
            .commit_file_batch(
                "s_home",
                "pi",
                "/home/user/.pi/agent/sessions/session-home.jsonl",
                1,
                0,
                "prefix2",
                None,
                &[],
                &[],
            )
            .unwrap();
        store.set_source_session("s_home", "pi-home-only").unwrap();

        // 3. Session "pi-both": source path contains agent/sessions AND has identity fact
        store
            .commit_file_batch(
                "s_both",
                "pi",
                "/home/user/.pi/agent/sessions/session-both.jsonl",
                1,
                0,
                "prefix3",
                None,
                &[],
                &[],
            )
            .unwrap();
        store.set_source_session("s_both", "pi-both").unwrap();
        store
            .insert_identity(&crate::IdentityFact {
                source_key: "id_both".into(),
                harness: "pi".into(),
                native_session: "pi-both".into(),
                rig: "my-rig".into(),
                seat: "my-seat".into(),
                ts: Some(chrono::Utc::now()),
            })
            .unwrap();

        store.checkpoint().unwrap();
        drop(store);

        let report = report_json(
            &db,
            "tokens",
            None,
            "2000-01-01T00:00:00Z",
            "2099-01-01T00:00:00Z",
        )
        .unwrap();
        let sessions = report["payload"]["sessions"].as_array().unwrap();
        let find = |id: &str| sessions.iter().find(|s| s["session"] == id).unwrap();

        assert_eq!(find("pi-openrig-only")["pi_origin"], "openrig");
        assert_eq!(find("pi-home-only")["pi_origin"], "pi-home");
        assert_eq!(find("pi-both")["pi_origin"], "both");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(all(test, windows))]
mod windows_publication_tests {
    use super::*;

    #[test]
    fn staged_flush_uses_verified_writable_handle_and_refuses_hardlinks() {
        use crate::platform_v1::{self as platform, StorageRootChoice};
        use std::io::Write;
        let root = platform::create_user_private_root_with_choice(
            "dendro-pipeline-flush-",
            &StorageRootChoice::UserProfile,
        )
        .unwrap();
        let stage = lifecycle::Stage::create(root.path()).unwrap();
        let path = stage.path().join("analytics.duckdb");
        let mut file = lifecycle::create_private_file(&path).unwrap();
        file.write_all(b"closed staged candidate").unwrap();
        drop(file);
        sync_staged_output(&stage).expect("native verified writable staged flush");
        let alias = root.path().join("foreign-alias.duckdb");
        fs::hard_link(&path, &alias).unwrap();
        assert!(
            sync_staged_output(&stage).is_err(),
            "multiply linked stage cannot acquire a flush handle"
        );
        assert_eq!(fs::read(&alias).unwrap(), b"closed staged candidate");
        fs::remove_file(alias).unwrap();
        stage.finish().unwrap();
        let cleanup = root.path().to_path_buf();
        drop(root);
        fs::remove_dir(cleanup).unwrap();
    }

    #[test]
    fn no_argument_publication_probe_never_selects_data() {
        let error = crate::platform_v1::publish_selection_manifest().unwrap_err();
        assert!(error
            .to_string()
            .contains("atomic_existing_manifest_replace_unproved"));
    }
}
