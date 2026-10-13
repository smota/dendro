use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::facts::SeatBound;
use crate::lifecycle;
use crate::metrics::{self, Window};
use crate::output::{self, load_or_create_key, refuse_aliased_output, with_output_lock};
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
    let parent = db.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let lock_path = parent.join(".dendro-output.lock");
    with_output_lock(&lock_path, || {
        run_inner(db, source_roots, openrig_roots, ingest)
    })
}

fn run_inner<F>(
    db: &Path,
    source_roots: &[PathBuf],
    openrig_roots: &[PathBuf],
    ingest: F,
) -> Result<Value>
where
    F: FnOnce(&Path, &[u8]) -> Result<Vec<String>>,
{
    // S31-2 / S31-5: an existing database is judged before the key sidecar,
    // the private stage, or any source read.
    if db.exists() {
        refuse_bound_openrig_root(db, openrig_roots)?;
    }
    refuse_aliased_output(db, source_roots)?;
    let before = fs::symlink_metadata(db).ok();
    if before
        .as_ref()
        .is_some_and(|meta| meta.file_type().is_symlink())
    {
        return Err(anyhow!("refusing symlink analytics output"));
    }
    let key_path = PathBuf::from(format!("{}.key", db.display()));
    let creating = !db.exists();
    let key = load_or_create_key(&key_path, creating || key_path.exists())?;
    if db.exists() {
        let preflight = AnalyticsStore::open_read_only(db)?;
        if preflight.key_id != key.id {
            return Err(anyhow!("analytics key id does not match the sidecar"));
        }
    }
    let parent = db.parent().unwrap_or_else(|| Path::new("."));
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
        File::open(&staged)?.sync_all()?;
        cancel.check()?;
        stage.check_anchor()?;
        let after = fs::symlink_metadata(db).ok();
        if after
            .as_ref()
            .is_some_and(|meta| meta.file_type().is_symlink())
        {
            return Err(anyhow!("refusing symlink analytics output"));
        }
        if let (Some(left), Some(right)) = (&before, &after) {
            use std::os::unix::fs::MetadataExt;
            if left.ino() != right.ino() || left.dev() != right.dev() {
                return Err(anyhow!("analytics output identity changed during ingest"));
            }
        }
        if before.is_none() && after.is_some() {
            return Err(anyhow!("analytics output appeared during ingest"));
        }
        // Publication: after this rename the output is the new complete db.
        fs::rename(&staged, db)?;
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

/// When `incoming` is the bound file under another spelling, return the
/// display path already stored so source keys stay stable (S31-3).
pub fn align_openrig_spelling(db: &Path, incoming: &Path) -> Result<PathBuf> {
    let bound = bound_openrig_roots(db)?;
    if bound.len() == 1 && bound[0].canonical == canonical_root(incoming) {
        return Ok(PathBuf::from(&bound[0].display));
    }
    Ok(incoming.to_path_buf())
}

fn refuse_bound_openrig_root(db: &Path, incoming: &[PathBuf]) -> Result<()> {
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
                    "refusing second OpenRig root: analytics database is bound to OpenRig root {}; refused {}",
                    redact(&bound.display),
                    redact(&path.display().to_string())
                ));
            }
        }
    }
    Ok(())
}

fn refuse_secret_path(path: &Path) -> Result<()> {
    let shown = path.display().to_string();
    if contains_secret(&shown) {
        return Err(anyhow!(
            "refusing a path that contains a secret-shaped value"
        ));
    }
    Ok(())
}

pub fn report_json(
    db: &Path,
    kind: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
) -> Result<Value> {
    refuse_secret_path(db)?;
    let window = parse_window(from, to)?;
    if !matches!(
        kind,
        "links" | "activity" | "handoffs" | "tokens" | "decisions"
    ) {
        return Err(anyhow!("unknown report kind {}", redact(kind)));
    }
    let store = AnalyticsStore::open_read_only(db)?;
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
    for (source_id, harness, path, _, _) in &sources {
        if harness == "openrig" {
            if let Some((_, bytes, revision)) = store.source_state(source_id)? {
                openrig_sources.push(json!({
                    "source_id": source_id,
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
    let queue_all: Vec<_> = store
        .load_queue()?
        .into_iter()
        .filter(|row| match &rig_seats {
            Some(seats) => seats.contains(&row.source_seat) || seats.contains(&row.dest_seat),
            None => true,
        })
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
            })
        })
        .collect();
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
    let loops = metrics::tool_loops(&loop_tools, 3, 600);
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
    }
    for (source_id, harness, _path, _generation, native) in &sources {
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
    let mut session_period =
        std::collections::BTreeMap::<(String, String), (Categories, usize)>::new();
    for row in &usage {
        let slot = session_period
            .entry((row.harness.clone(), row.native_session.clone()))
            .or_default();
        slot.0.add(row);
        slot.1 += 1;
    }
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
            row["period_records"] = json!(records);
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
    Ok(metrics::report_value(
        kind,
        json!({
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
            "tool_loops": loops.iter().map(|item| json!({"session": item.session, "tool": item.tool, "count": item.count})).collect::<Vec<_>>(),
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
        }),
    ))
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
    let report = report_json(db, "decisions", rig, from, to)?;
    let payload = &report["payload"];
    let (formula, value) = match metric_id {
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
        "m4" => ("impact is insufficient data", payload["m4_impact"].clone()),
        "tokens" => (
            "sum of four token buckets in the half-open window",
            payload["tokens"].clone(),
        ),
        "intake" => (
            "claim-inclusive to the receiver's first structured write or same-item queue transition, exclusive",
            payload["intakes"].clone(),
        ),
        "medians" => (
            "even-sample median of wait seconds and intake tokens",
            payload["pair_medians"].clone(),
        ),
        "activity" => (
            "five disjoint seat bands clipped to the half-open window",
            payload["activity"].clone(),
        ),
        other => return Err(anyhow!("unknown metric id {}", redact(other))),
    };
    Ok(json!({
        "metric_id": metric_id,
        "formula_version": "acceptance-r1-a1",
        "formula": formula,
        "value": value,
        "inputs": {
            "tokens": payload["tokens"],
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
            "seat_lives": payload["seat_lives"],
            "openrig_sources": payload["openrig_sources"],
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
        },
        "from": redact(from),
        "to": redact(to),
        "rig": rig.map(redact),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AnalyticsStore;

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
        assert_eq!(
            report["payload"]["m4_impact"]
                .as_str()
                .unwrap()
                .contains("insufficient data"),
            true
        );
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
        run_locked_with_openrig(db, &[source.clone()], &[source.clone()], |staged, _key| {
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
        })
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
        let err = run_locked_with_openrig(&db, &[other.clone()], &[other.clone()], |_, _| {
            called = true;
            Ok(vec![])
        })
        .unwrap_err();
        let message = err.to_string();
        assert!(message
            .contains("refusing second OpenRig root: analytics database is bound to OpenRig root"));
        assert!(message.contains(&bound.display().to_string()));
        assert!(message.contains("refused"));
        assert!(message.contains(&other.display().to_string()));
        assert!(!called);
        assert_eq!(fs::read(&db).unwrap(), before);

        let link = dir.join("bound-link.sqlite");
        std::os::unix::fs::symlink(&bound, &link).unwrap();
        let spelled = dir.join("extra").join("..").join("bound.sqlite");
        for alias in [link, spelled] {
            let mut saw = false;
            run_locked_with_openrig(&db, &[alias.clone()], &[alias.clone()], |_, _| {
                saw = true;
                Ok(vec!["same".into()])
            })
            .unwrap();
            assert!(saw, "same file must not be refused");
        }
        let aligned =
            align_openrig_spelling(&db, &dir.join("extra").join("..").join("bound.sqlite"))
                .unwrap();
        assert_eq!(aligned, bound);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_without_openrig_still_runs_and_a_mixed_database_is_refused() {
        let dir = scratch("mixed");
        let claude = dir.join("claude");
        fs::create_dir_all(&claude).unwrap();
        let db = dir.join("analytics.duckdb");
        run_locked_with_openrig(&db, &[claude.clone()], &[], |staged, _key| {
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
        run_locked_with_openrig(&db, &[claude.clone()], &[], |_, _| Ok(vec!["again".into()]))
            .unwrap();

        let first = dir.join("one.sqlite");
        let second = dir.join("two.sqlite");
        touch(&first);
        touch(&second);
        let mixed = dir.join("mixed.duckdb");
        bind_openrig(&mixed, &[&first, &second]);
        let before = fs::read(&mixed).unwrap();
        let mut called = false;
        let err = run_locked_with_openrig(&mixed, &[first.clone()], &[first.clone()], |_, _| {
            called = true;
            Ok(vec![])
        })
        .unwrap_err();
        assert!(err.to_string().contains("refusing second OpenRig root"));
        assert!(err.to_string().contains(&first.display().to_string()));
        assert!(err.to_string().contains(&second.display().to_string()));
        assert!(!called);
        assert_eq!(fs::read(&mixed).unwrap(), before);
        assert!(report_json(
            &mixed,
            "decisions",
            None,
            "2026-10-08T00:00:00Z",
            "2026-10-11T00:00:00Z",
        )
        .is_ok());
        let _ = fs::remove_dir_all(&dir);
    }
}
