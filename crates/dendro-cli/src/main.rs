#![deny(unsafe_code)]

mod extensions;
pub mod tui;

use anyhow::Result;
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::Duration;
use tokio::task::JoinHandle;

/// How long a signalled run may take to stop its worker and remove its stage.
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(30);

const DEFAULT_DB_HELP: &str = "Analytics database. When omitted, Dendro uses dendro.duckdb in the private per-user default folder (Windows: %LOCALAPPDATA%\\Dendro, Linux: $XDG_DATA_HOME/dendro or ~/.local/share/dendro). A database outside a private folder is refused; Dendro prints a copy command and does not copy the file.";

const AFTER_HELP: &str = "\
Quickstart Examples:
  dendro
      Status report. Same as `dendro status`. The database is dendro.duckdb in the private default folder.
  dendro dashboard --auto-ingest
      Open the 6-screen cockpit. Ingest local roots when the database is missing.
  dendro ask --format text \"highest idle\"
      Route a supported question. Text on a terminal, json when piped or requested.
  dendro report decisions --window 7d
  dendro trace --metric-id handoff_friction --window 7d
  dendro simulate topology --help

Interactive TUI Shortcuts:
  1 Pulse                  2 Seats & Timeline       3 Handoffs & Flow
  4 Tokens & Cache         5 Topology Simulator     6 Trace & Spotlights
  / filter seats           x show queue endpoints and system gates
  m ribbon or timeline     v cycle the handoff heatmap metric
  r cycle named rig        w cycle window (1h, 24h, 7d, 30d, all)
  : ask Nao                i preview an issue draft
  ? help                   q quit

Report Kinds:
  links, activity, handoffs, tokens, decisions
  advisory, anomalies, spotlights

Command Groups:
  Start        status, run
  Cockpit      dashboard, ui
  Ask          ask
  Reports      report, trace, insights
  Analysis     simulate, portfolio
  Governance   assert, issue-draft
  Cache        materialize, export, init-nao
";

#[derive(Parser)]
#[command(name = "dendro")]
#[command(about = "Dendro AI Fleet Intelligence. Operational Intelligence for seat, handoff, and token evidence. Fleet Governance stays with the operator.", long_about = None)]
#[command(version, after_help = AFTER_HELP)]
struct Cli {
    /// Verified existing private storage root; default database becomes <root>/dendro.duckdb.
    /// Omit both this flag and --db to use the private per-user default folder.
    #[arg(long, global = true)]
    storage_root: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Friction Thresholds")]
struct FrictionArgs {
    #[arg(long, value_parser = parse_return_rate)]
    friction_return_rate: Option<f64>,
    #[arg(long)]
    friction_chain_returns: Option<u64>,
    #[arg(long)]
    friction_rework_tokens: Option<u64>,
    #[arg(long, value_parser = dendro_core::parse_friction_duration)]
    friction_loop_duration: Option<u64>,
}

fn parse_return_rate(value: &str) -> Result<f64, String> {
    let rate = value
        .parse::<f64>()
        .map_err(|_| "friction-return-rate must be finite and between 0 and 1")?;
    if !rate.is_finite() || !(0.0..=1.0).contains(&rate) {
        return Err("friction-return-rate must be finite and between 0 and 1".into());
    }
    Ok(rate)
}
impl FrictionArgs {
    fn supplied(&self) -> bool {
        self.friction_return_rate.is_some()
            || self.friction_chain_returns.is_some()
            || self.friction_rework_tokens.is_some()
            || self.friction_loop_duration.is_some()
    }
    fn thresholds(&self) -> dendro_core::FrictionThresholds {
        let defaults = dendro_core::FrictionThresholds::default();
        dendro_core::FrictionThresholds {
            return_rate: self.friction_return_rate.unwrap_or(defaults.return_rate),
            chain_returns: self
                .friction_chain_returns
                .unwrap_or(defaults.chain_returns),
            rework_tokens: self
                .friction_rework_tokens
                .unwrap_or(defaults.rework_tokens),
            loop_duration_secs: self
                .friction_loop_duration
                .unwrap_or(defaults.loop_duration_secs),
        }
    }
}

#[derive(Args, Debug, Default)]
#[command(next_help_heading = "Advisory & Spotlights")]
struct AdvisoryArgs {
    #[arg(long)]
    advisory_container_hold_secs: Option<u64>,
    #[arg(long)]
    advisory_leaf_stall_secs: Option<u64>,
    #[arg(long)]
    advisory_detached_target_secs: Option<u64>,
    #[arg(long, value_parser = parse_return_rate)]
    spotlight_ceremony_ratio: Option<f64>,
    #[arg(long, value_parser = parse_return_rate)]
    spotlight_serialization_ratio: Option<f64>,
    #[arg(long, value_parser = parse_nonnegative)]
    spotlight_parallelism: Option<f64>,
    #[arg(long)]
    spotlight_minimum_seats: Option<usize>,
    #[arg(long, value_parser = parse_return_rate)]
    spotlight_container_ratio: Option<f64>,
    #[arg(long, value_parser = parse_return_rate)]
    spotlight_recovery_ratio: Option<f64>,
    #[arg(long, value_parser = parse_return_rate)]
    spotlight_context_drop_ratio: Option<f64>,
    #[arg(long)]
    spotlight_cache_bleed_tokens: Option<u64>,
    #[arg(long, value_parser = parse_return_rate)]
    spotlight_builder_duty_ratio: Option<f64>,
    #[arg(long, value_parser = parse_return_rate)]
    spotlight_peer_saturation_ratio: Option<f64>,
    #[arg(long, value_delimiter = ',')]
    spotlight_builder_seats: Vec<String>,
}
impl AdvisoryArgs {
    fn supplied(&self) -> bool {
        self.advisory_container_hold_secs.is_some()
            || self.advisory_leaf_stall_secs.is_some()
            || self.advisory_detached_target_secs.is_some()
            || self.spotlight_ceremony_ratio.is_some()
            || self.spotlight_serialization_ratio.is_some()
            || self.spotlight_parallelism.is_some()
            || self.spotlight_minimum_seats.is_some()
            || self.spotlight_container_ratio.is_some()
            || self.spotlight_recovery_ratio.is_some()
            || self.spotlight_context_drop_ratio.is_some()
            || self.spotlight_cache_bleed_tokens.is_some()
            || self.spotlight_builder_duty_ratio.is_some()
            || self.spotlight_peer_saturation_ratio.is_some()
            || !self.spotlight_builder_seats.is_empty()
    }
    fn options(&self) -> dendro_core::advisory::AdvisoryOptions {
        let d = dendro_core::advisory::AdvisoryOptions::default();
        dendro_core::advisory::AdvisoryOptions {
            container_hold_secs: self
                .advisory_container_hold_secs
                .unwrap_or(d.container_hold_secs),
            leaf_stall_secs: self.advisory_leaf_stall_secs.unwrap_or(d.leaf_stall_secs),
            detached_target_secs: self
                .advisory_detached_target_secs
                .unwrap_or(d.detached_target_secs),
            ceremony_ratio: self.spotlight_ceremony_ratio.unwrap_or(d.ceremony_ratio),
            serialization_ratio: self
                .spotlight_serialization_ratio
                .unwrap_or(d.serialization_ratio),
            parallelism: self.spotlight_parallelism.unwrap_or(d.parallelism),
            minimum_seats: self.spotlight_minimum_seats.unwrap_or(d.minimum_seats),
            container_ratio: self.spotlight_container_ratio.unwrap_or(d.container_ratio),
            recovery_ratio: self.spotlight_recovery_ratio.unwrap_or(d.recovery_ratio),
            context_drop_ratio: self
                .spotlight_context_drop_ratio
                .unwrap_or(d.context_drop_ratio),
            cache_bleed_tokens: self
                .spotlight_cache_bleed_tokens
                .unwrap_or(d.cache_bleed_tokens),
            builder_duty_ratio: self
                .spotlight_builder_duty_ratio
                .unwrap_or(d.builder_duty_ratio),
            peer_saturation_ratio: self
                .spotlight_peer_saturation_ratio
                .unwrap_or(d.peer_saturation_ratio),
            builder_seats: self.spotlight_builder_seats.clone(),
        }
    }
}
fn parse_nonnegative(value: &str) -> Result<f64, String> {
    let n = value
        .parse::<f64>()
        .map_err(|_| "expected a finite nonnegative number")?;
    if !n.is_finite() || n < 0.0 {
        return Err("expected a finite nonnegative number".into());
    }
    Ok(n)
}
#[derive(Args, Debug)]
struct ReportScope {
    #[arg(long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP)]
    db: PathBuf,
    #[arg(long)]
    rig: Option<String>,
    #[arg(
        long,
        value_parser = ["1h", "24h", "7d", "30d", "all"],
        conflicts_with_all = ["from", "to"]
    )]
    window: Option<String>,
    #[arg(long, default_value = "2026-10-08T00:00:00Z")]
    from: String,
    #[arg(long, default_value = "2026-10-11T00:00:00Z")]
    to: String,
}
async fn canonical_decisions(scope: ReportScope) -> Result<serde_json::Value> {
    tokio::task::spawn_blocking(move || {
        let request = dendro_core::presentation::ReadRequest::open(&scope.db)?;
        let (from, to) = if scope.window.is_some() {
            let (f, t, _) =
                request.window(scope.rig.as_deref(), scope.window.as_deref(), None, None)?;
            (f, t)
        } else {
            (scope.from, scope.to)
        };
        dendro_core::pipeline::report_json_physical(
            request.database(),
            "decisions",
            scope.rig.as_deref(),
            &from,
            &to,
        )
    })
    .await?
}

fn issue_publish_args(repo: &str, body: &str) -> Result<Vec<String>> {
    let parts: Vec<_> = repo.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || *p == "."
                || *p == ".."
                || !p
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
    {
        anyhow::bail!("repository must be OWNER/REPO");
    }
    Ok(vec![
        "issue".into(),
        "create".into(),
        "--repo".into(),
        repo.into(),
        "--title".into(),
        "Investigate observed fleet operational debt".into(),
        "--body".into(),
        dendro_core::redact(body),
    ])
}

#[derive(Subcommand)]
enum Commands {
    /// Create or verify the private default data folder and print its path.
    StorageInit,
    /// Reconcile a prepared selection under its recorded writer/publisher locks.
    // Recovery returns a JSON status. resume_required is incomplete and exits nonzero.
    RecoverSelection {
        #[arg(long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP)]
        db: PathBuf,
        /// Reconcile a prepared cutoff manifest after recovering the analytics selection.
        #[arg(long)]
        cutoff: Option<PathBuf>,
    },
    /// Explicitly select a validated schema4 migration while retaining schema3 custody.
    Migrate {
        #[arg(long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP)]
        db: PathBuf,
    },
    /// Select the retained schema3 preimage and preserve schema4 plus a loss receipt.
    Rollback {
        #[arg(long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP)]
        db: PathBuf,
    },
    /// Inspect coverage, discovered harness roots, fleet topology, and next commands.
    #[command(next_help_heading = "Database & Scope")]
    Status {
        #[arg(long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP)]
        db: PathBuf,
        #[arg(long)]
        rig: Option<String>,
        /// Text report or the raw status document.
        #[arg(long, value_parser = ["text", "json"])]
        format: Option<String>,
    },
    /// Sample agent processes (CPU, memory, disk, exit) into the analytics database.
    #[command(next_help_heading = "Ingest")]
    Watch {
        #[arg(short, long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP)]
        db: PathBuf,
        /// Time between samples, for example 500ms, 1s or 2m.
        #[arg(long, default_value = "1s", value_parser = parse_watch_duration)]
        interval: Duration,
        /// Stop after this long. Without it, watch runs until Ctrl-C.
        #[arg(long, value_parser = parse_watch_duration)]
        duration: Option<Duration>,
        /// Take one sample of everything found, then stop.
        #[arg(long)]
        once: bool,
        /// Executable name of a harness to track (repeatable). Defaults to claude, codex, grok, pi.
        #[arg(long)]
        harness: Vec<String>,
        /// How often collected samples are written to the database.
        #[arg(long, default_value = "60s", value_parser = parse_watch_duration)]
        flush_every: Duration,
    },
    /// Launch a command and sample that process tree from its exact pid.
    #[command(next_help_heading = "Ingest")]
    RunAgent {
        #[arg(short, long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP)]
        db: PathBuf,
        /// Harness label stored on the launched root. The pid is pinned either way.
        #[arg(long)]
        harness: Option<String>,
        /// Time between samples, for example 50ms, 1s or 2m.
        #[arg(long, default_value = "1s", value_parser = parse_watch_duration)]
        interval: Duration,
        /// How often collected samples are written to the database.
        #[arg(long, default_value = "60s", value_parser = parse_watch_duration)]
        flush_every: Duration,
        /// Command to launch. Put `--` before it so its flags are not Dendro's.
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Ingest local harness roots into an analytics database.
    #[command(next_help_heading = "Ingest")]
    Run {
        #[arg(short, long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP)]
        db: PathBuf,
        #[arg(long)]
        root: Vec<String>,
        #[arg(long)]
        frozen: bool,
        /// Write the exact file and SQLite cutoffs consumed by this run.
        #[arg(long, visible_alias = "manifest-out", conflicts_with = "sealed")]
        write_manifest: Option<PathBuf>,
        /// Replay only the prefixes recorded in this cutoff manifest.
        #[arg(long, conflicts_with = "write_manifest")]
        sealed: Option<PathBuf>,
    },
    #[command(flatten, next_help_heading = "Cockpit, Ask & Analysis")]
    Extensions(extensions::Commands),
    /// Run each compiled reference insight against the selected database.
    #[command(next_help_heading = "Report")]
    Insights {
        #[arg(long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP)]
        db: PathBuf,
        /// Print the result array instead of the text table.
        #[arg(long)]
        json: bool,
        #[arg(long)]
        harness: Option<String>,
        #[arg(long)]
        session: Option<String>,
    },
    /// Print an Operational Intelligence report.
    #[command(next_help_heading = "Report")]
    Report {
        kind: String,
        #[arg(long,value_parser=["json","table","markdown","ndjson"])]
        format: Option<String>,
        /// Print terminal block graphs instead of a serialized report.
        #[arg(long, conflicts_with = "format")]
        viz: bool,
        #[arg(long,value_parser=["idle-claimed"])]
        anomaly: Option<String>,
        /// Request the handoff friction field (also present without this flag).
        #[arg(long)]
        friction: bool,
        #[arg(long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP)]
        db: PathBuf,
        #[arg(long)]
        rig: Option<String>,
        #[arg(
            long,
            value_parser = ["1h", "24h", "7d", "30d", "all"],
            conflicts_with_all = ["from", "to"]
        )]
        window: Option<String>,
        #[arg(long, default_value = "2026-10-08T00:00:00Z")]
        from: String,
        #[arg(long, default_value = "2026-10-11T00:00:00Z")]
        to: String,
        #[arg(long, default_value_t = 3)]
        tool_loop_failures: usize,
        #[arg(long, default_value_t = 600, allow_hyphen_values = true)]
        tool_loop_window_secs: i64,
        #[arg(long, default_value_t = 2)]
        handoff_loop_returns: usize,
        #[command(flatten)]
        friction_thresholds: FrictionArgs,
        #[command(flatten)]
        advisory_thresholds: AdvisoryArgs,
    },
    /// Print the trace inputs for one metric id.
    #[command(next_help_heading = "Trace")]
    Trace {
        #[arg(long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP)]
        db: PathBuf,
        #[arg(long)]
        metric_id: String,
        #[arg(long)]
        rig: Option<String>,
        #[arg(
            long,
            value_parser = ["1h", "24h", "7d", "30d", "all"],
            conflicts_with_all = ["from", "to"]
        )]
        window: Option<String>,
        #[arg(long, default_value = "2026-10-08T00:00:00Z")]
        from: String,
        #[arg(long, default_value = "2026-10-11T00:00:00Z")]
        to: String,
        #[arg(long, default_value_t = 3)]
        tool_loop_failures: usize,
        #[arg(long, default_value_t = 600, allow_hyphen_values = true)]
        tool_loop_window_secs: i64,
        #[arg(long, default_value_t = 2)]
        handoff_loop_returns: usize,
        #[command(flatten)]
        friction_thresholds: FrictionArgs,
        #[command(flatten)]
        advisory_thresholds: AdvisoryArgs,
    },
    /// Emit an advisory CI verdict: 0 pass, 2 breach, 3 insufficient evidence.
    #[command(next_help_heading = "Governance")]
    Assert {
        #[command(flatten)]
        scope: ReportScope,
        #[arg(long,default_value="0.15",value_parser=parse_return_rate, help_heading = "Verdict")]
        max_rework_rate: f64,
        #[arg(long,default_value="0.20",value_parser=parse_return_rate, help_heading = "Verdict")]
        max_idle_rate: f64,
    },
    /// Draft local Markdown. Publish only with explicit --publish --repo.
    #[command(next_help_heading = "Governance")]
    IssueDraft {
        #[command(flatten)]
        scope: ReportScope,
        #[arg(long, default_value = "m3", value_parser=["m3","handoff_friction","advisory_idle_claimed","anomaly_spotlights"])]
        metric: String,
        #[arg(long,value_parser=parse_nonnegative)]
        usd_per_million: Option<f64>,
        #[arg(long, requires = "repo")]
        publish: bool,
        #[arg(long, requires = "publish")]
        repo: Option<String>,
    },
}

async fn run_ingest(
    db: PathBuf,
    root: Vec<String>,
    frozen: bool,
    write_manifest: Option<PathBuf>,
    sealed: Option<PathBuf>,
) -> Result<serde_json::Value> {
    let mut roots = if root.is_empty() {
        default_roots()?
    } else {
        parse_roots(&root)?
    };
    for root in &mut roots {
        if root.name == "openrig" {
            root.path = dendro_core::absolute_openrig_root(&root.path);
        }
    }
    let paths: Vec<PathBuf> = roots.iter().map(|root| root.path.clone()).collect();
    let openrig_roots: Vec<PathBuf> = roots
        .iter()
        .filter(|root| root.name == "openrig")
        .map(|root| root.path.clone())
        .collect();
    let mut events = ShutdownEvents::new()?;
    let mut worker = tokio::task::spawn_blocking(move || {
        dendro_core::run_locked_with_manifest(
            &db,
            &paths,
            &openrig_roots,
            write_manifest.as_deref(),
            sealed.as_deref(),
            |staged, key, manifest| {
                let mut roots = roots;
                for root in &mut roots {
                    if root.name == "openrig" {
                        root.path = dendro_core::align_openrig_spelling(staged, &root.path)?;
                    }
                }
                dendro_connectors::ingest_staged_with_manifest(
                    staged, &roots, frozen, key, manifest,
                )
            },
        )
    });
    let cause = tokio::select! {
        joined = &mut worker => {
            return joined?;
        }
        cause = events.next() => cause,
    };
    std::process::exit(stop_cooperatively(worker, events, cause).await);
}

/// `500ms`, `30s`, `5m` or `1h`; never zero.
fn parse_watch_duration(value: &str) -> Result<Duration, String> {
    let (digits, unit_ms) = if let Some(n) = value.strip_suffix("ms") {
        (n, 1)
    } else if let Some(n) = value.strip_suffix('s') {
        (n, 1_000)
    } else if let Some(n) = value.strip_suffix('m') {
        (n, 60_000)
    } else if let Some(n) = value.strip_suffix('h') {
        (n, 3_600_000)
    } else {
        return Err("expected a duration such as 500ms, 1s, 5m or 1h".into());
    };
    let millis = digits
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(unit_ms))
        .filter(|n| *n > 0)
        .ok_or("expected a positive duration such as 500ms, 1s, 5m or 1h")?;
    Ok(Duration::from_millis(millis))
}

async fn run_watch(
    db: PathBuf,
    interval: Duration,
    duration: Option<Duration>,
    once: bool,
    harness: Vec<String>,
    flush_every: Duration,
) -> Result<serde_json::Value> {
    use std::sync::atomic::{AtomicBool, Ordering};
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let mut events = ShutdownEvents::new()?;
    let worker_stop = stop.clone();
    let mut worker = tokio::task::spawn_blocking(move || {
        watch_blocking(
            &db,
            interval,
            duration,
            once,
            harness,
            flush_every,
            &worker_stop,
        )
    });
    tokio::select! {
        joined = &mut worker => return joined?,
        _ = events.next() => {}
    }
    // Ctrl-C ends sampling; what was collected is still written.
    stop.store(true, Ordering::Relaxed);
    eprintln!("dendro: stopping; writing collected samples");
    worker.await?
}

fn watch_blocking(
    db: &std::path::Path,
    interval: Duration,
    duration: Option<Duration>,
    once: bool,
    harness: Vec<String>,
    flush_every: Duration,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<serde_json::Value> {
    use dendro_connectors::environment_v1 as env;
    let mut config = env::Config::default();
    if !harness.is_empty() {
        config.harnesses = harness;
    }
    let mut collector =
        env::Collector::new(env::SysinfoSource::new(), config, env::detect_isolation());
    // Each flush runs on this sampling thread and the sampler waits for it:
    // one guarded, transactional publication of the batch collected since
    // the last one.
    let summary = env::watch(
        &mut collector,
        interval,
        duration,
        once,
        flush_every,
        stop,
        |batch| {
            if batch.identities.is_empty() && batch.samples.is_empty() {
                return Ok(());
            }
            dendro_core::run_locked_with_manifest(db, &[], &[], None, None, |staged, key, _| {
                // No harness session records a working directory yet, so no
                // session candidates exist and roots stay unlinked.
                let sealed = batch.seal(key, &[]);
                let store = dendro_core::AnalyticsStore::open_staged(staged)?;
                store.commit_process_samples(&sealed.identities, &sealed.samples)?;
                store.checkpoint()?;
                Ok((Vec::new(), dendro_core::CutoffManifest::default()))
            })?;
            Ok(())
        },
    )?;
    Ok(serde_json::json!({
        "status": "watched",
        "ticks": summary.ticks,
        "identity_updates": summary.identities,
        "samples": summary.samples,
        "refused_by_cap": summary.refused_by_cap,
    }))
}

fn child_exit_code(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    1
}

async fn run_agent(
    db: PathBuf,
    harness: Option<String>,
    interval: Duration,
    flush_every: Duration,
    command: Vec<String>,
) -> Result<i32> {
    use std::sync::atomic::{AtomicBool, Ordering};
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let mut events = ShutdownEvents::new()?;
    let worker_stop = stop.clone();
    let mut worker = tokio::task::spawn_blocking(move || {
        run_agent_blocking(&db, harness, interval, flush_every, &command, &worker_stop)
    });
    tokio::select! {
        joined = &mut worker => return joined?,
        _ = events.next() => {}
    }
    // Ctrl-C already reached the child: Unix delivers SIGINT to the foreground
    // process group, and Windows delivers it to every process on this console.
    // The child was not placed in a new group, so signalling it again would
    // double-deliver. Stop sampling and let the child exit on its own.
    stop.store(true, Ordering::Relaxed);
    eprintln!("dendro: stopping; writing collected samples");
    worker.await?
}

fn run_agent_blocking(
    db: &std::path::Path,
    harness: Option<String>,
    interval: Duration,
    flush_every: Duration,
    command: &[String],
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<i32> {
    use dendro_connectors::environment_v1 as env;
    use std::process::Command;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::time::Instant;

    let program = command
        .first()
        .map(String::as_str)
        .filter(|program| !program.is_empty())
        .ok_or_else(|| anyhow::anyhow!("run-agent needs a command after --"))?;
    let mut config = env::Config::default();
    // Only the launched tree. The default name list would attribute other harnesses.
    config.harnesses.clear();
    let mut collector =
        env::Collector::new(env::SysinfoSource::new(), config, env::detect_isolation());
    // Build the sampler before spawn so a short child is not exiting during init.
    // Inherited stdin/stdout/stderr, cwd, and env. No DENDRO_* marker.
    let mut child = Command::new(program)
        .args(&command[1..])
        .spawn()
        .map_err(|err| anyhow::anyhow!("could not launch {program}: {err}"))?;
    let pid = child.id();
    collector.pin_launched(pid, harness);
    // Sample before any other work. `cmd /c exit 3` can leave the process list
    // within a few milliseconds of CreateProcess returning.
    collector.tick_now();

    let (exit_tx, exit_rx) = mpsc::channel();
    let (kill_tx, kill_rx) = mpsc::channel::<()>();
    let child_stop = stop.clone();
    std::thread::spawn(move || {
        let status = loop {
            if kill_rx.try_recv().is_ok() {
                let _ = child.kill();
                break child.wait();
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => std::thread::sleep(Duration::from_millis(15)),
                Err(err) => break Err(err),
            }
        };
        child_stop.store(true, Ordering::Relaxed);
        let _ = exit_tx.send(status);
    });
    // A `cmd /c exit` process can disappear between scans. Tick until it is
    // adopted or it has already ended.
    let deadline = Instant::now() + Duration::from_millis(500);
    while !collector.tracks(pid) && Instant::now() < deadline {
        collector.tick_now();
        if collector.tracks(pid) || stop.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if !collector.tracks(pid) {
        // The process can exit before the snapshot syscall under load.
        let grace = Instant::now() + Duration::from_millis(200);
        while !stop.load(Ordering::Relaxed) && Instant::now() < grace {
            std::thread::sleep(Duration::from_millis(5));
        }
        if stop.load(Ordering::Relaxed) {
            collector.note_exited_pin(pid, program);
        }
    }

    let watched = env::watch(
        &mut collector,
        interval,
        None,
        false,
        flush_every,
        stop,
        |batch| {
            if batch.identities.is_empty() && batch.samples.is_empty() {
                return Ok(());
            }
            dendro_core::run_locked_with_manifest(db, &[], &[], None, None, |staged, key, _| {
                let sealed = batch.seal(key, &[]);
                let store = dendro_core::AnalyticsStore::open_staged(staged)?;
                store.commit_process_samples(&sealed.identities, &sealed.samples)?;
                store.checkpoint()?;
                Ok((Vec::new(), dendro_core::CutoffManifest::default()))
            })?;
            Ok(())
        },
    );
    if let Err(err) = watched {
        let _ = kill_tx.send(());
        let _ = exit_rx.recv_timeout(Duration::from_secs(5));
        return Err(err);
    }
    let status = exit_rx
        .recv()
        .map_err(|_| anyhow::anyhow!("launched process waiter ended before reporting a status"))?
        .map_err(|err| anyhow::anyhow!("could not wait for {program}: {err}"))?;
    let code = child_exit_code(status);
    println!(
        "{}",
        serde_json::json!({
            "status": "launched",
            "exit_code": code,
            "pid": pid,
        })
    );
    let _ = std::io::Write::flush(&mut std::io::stdout());
    Ok(code)
}

fn database_slot(command: &mut Commands) -> Option<&mut PathBuf> {
    match command {
        Commands::RecoverSelection { db, .. }
        | Commands::Migrate { db }
        | Commands::Rollback { db }
        | Commands::Status { db, .. }
        | Commands::Run { db, .. }
        | Commands::Watch { db, .. }
        | Commands::RunAgent { db, .. }
        | Commands::Report { db, .. }
        | Commands::Trace { db, .. }
        | Commands::Insights { db, .. } => Some(db),
        Commands::Assert { scope, .. } | Commands::IssueDraft { scope, .. } => Some(&mut scope.db),
        Commands::Extensions(extension) => match extension {
            extensions::Commands::Dashboard { db, .. }
            | extensions::Commands::Ui { db, .. }
            | extensions::Commands::Ask { db, .. }
            | extensions::Commands::InitNao { db, .. } => Some(db),
            extensions::Commands::Materialize { scope }
            | extensions::Commands::Export { scope, .. } => Some(&mut scope.db),
            extensions::Commands::Simulate {
                command: extensions::SimulateCommand::Topology { db, .. },
            } => Some(db),
            extensions::Commands::Portfolio { .. } => None,
        },
        Commands::StorageInit => None,
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn command_opens_database(command: &Commands) -> bool {
    !matches!(
        command,
        Commands::StorageInit
            | Commands::Extensions(extensions::Commands::Portfolio { .. })
            | Commands::Extensions(extensions::Commands::Ask {
                list_intents: true,
                ..
            })
            | Commands::Extensions(extensions::Commands::Ask {
                query: None,
                question: None,
                ..
            })
    )
}

#[cfg(any(windows, target_os = "linux"))]
fn admitted_private_root(path: &std::path::Path) -> bool {
    dendro_core::platform_v1::open_existing_private_root(path).is_ok()
}

#[cfg(any(windows, target_os = "linux"))]
fn parent_is_admitted_private_root(path: &std::path::Path) -> bool {
    std::path::absolute(path)
        .ok()
        .and_then(|absolute| absolute.parent().map(std::path::Path::to_path_buf))
        .is_some_and(|parent| admitted_private_root(&parent))
}

#[cfg(any(windows, target_os = "linux"))]
fn quoted_path(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    #[cfg(windows)]
    {
        format!("\"{}\"", text.replace('`', "``").replace('"', "`\""))
    }
    #[cfg(target_os = "linux")]
    {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn copy_command(source: &std::path::Path, destination: &std::path::Path) -> String {
    #[cfg(windows)]
    {
        format!(
            "Copy-Item -LiteralPath {} -Destination {}",
            quoted_path(source),
            quoted_path(destination)
        )
    }
    #[cfg(target_os = "linux")]
    {
        format!("cp -- {} {}", quoted_path(source), quoted_path(destination))
    }
}

/// Refuse before any database open. Names the private default folder and the
/// OS copy command. Does not copy or move the file.
#[cfg(any(windows, target_os = "linux"))]
fn refuse_database_outside_private_folder(
    source: &std::path::Path,
    folder: &std::path::Path,
) -> String {
    let source = std::path::absolute(source).unwrap_or_else(|_| source.to_path_buf());
    let mut message = format!(
        "database {} is outside a private folder and cannot be opened. Move it into the private default folder {}: {}",
        source.display(),
        folder.display(),
        copy_command(&source, &folder.join("dendro.duckdb"))
    );
    let wal = std::path::PathBuf::from(format!("{}.wal", source.display()));
    if wal.is_file() {
        message.push_str(&format!(
            " Also copy the WAL: {}",
            copy_command(&wal, &folder.join("dendro.duckdb.wal"))
        ));
    }
    message
}

#[cfg(any(windows, target_os = "linux"))]
fn prepare_default_database(command: &mut Commands, db_explicit: bool) -> Result<()> {
    if !command_opens_database(command) {
        return Ok(());
    }
    let Some(db) = database_slot(command) else {
        return Ok(());
    };
    if db_explicit {
        if parent_is_admitted_private_root(db) {
            return Ok(());
        }
        let root = dendro_core::platform_v1::open_or_create_default_data_root()?;
        let folder = root.path().to_path_buf();
        drop(root);
        anyhow::bail!("{}", refuse_database_outside_private_folder(db, &folder));
    }
    if db.as_path() != std::path::Path::new("dendro.duckdb") {
        return Ok(());
    }
    let root = dendro_core::platform_v1::open_or_create_default_data_root()?;
    let folder = root.path().to_path_buf();
    let default_db = folder.join("dendro.duckdb");
    drop(root);
    let legacy = std::path::Path::new("dendro.duckdb");
    if legacy.exists() && !default_db.exists() {
        if std::env::current_dir().is_ok_and(|cwd| admitted_private_root(&cwd)) {
            return Ok(());
        }
        anyhow::bail!(
            "{}",
            refuse_database_outside_private_folder(legacy, &folder)
        );
    }
    *db = default_db;
    Ok(())
}

fn has_explicit_database(matches: &clap::ArgMatches) -> bool {
    (matches.ids().any(|id| id.as_str() == "db")
        && matches.value_source("db") == Some(clap::parser::ValueSource::CommandLine))
        || matches
            .subcommand()
            .is_some_and(|(_, matches)| has_explicit_database(matches))
}

#[cfg(any(windows, target_os = "linux"))]
fn hex_selection(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let parse_result = if std::env::args_os().len() == 1 {
        Cli::command().try_get_matches_from(["dendro", "status"])
    } else {
        Cli::command().try_get_matches()
    };
    let (cli, db_explicit) = match parse_result {
        Ok(matches) => (
            Cli::from_arg_matches(&matches)?,
            has_explicit_database(&matches),
        ),
        Err(err) => {
            use clap::error::ErrorKind;
            if matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) {
                err.print()?;
                std::process::exit(0);
            }
            let code = err.exit_code();
            eprintln!("{}", dendro_core::redact(&err.to_string()));
            std::process::exit(code);
        }
    };

    let mut command = cli.command;
    if matches!(command, Commands::StorageInit) {
        if cli.storage_root.is_some() {
            anyhow::bail!("storage-init creates a new native private root; omit --storage-root, then reuse the printed path with consumer commands");
        }
        #[cfg(any(windows, target_os = "linux"))]
        {
            let root = dendro_core::platform_v1::open_or_create_default_data_root()?;
            let path = root.path().to_path_buf();
            drop(root);
            println!(
                "{}",
                serde_json::json!({"status":"created", "storage_root":path, "reuse":"pass the exact storage_root path with --storage-root, or omit --db to use storage_root/dendro.duckdb"})
            );
            return Ok(());
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        anyhow::bail!("storage-init selected storage is supported only on Windows and Linux");
    }
    if let Some(root) = cli.storage_root {
        if db_explicit {
            anyhow::bail!("use either an explicit --db (including dendro.duckdb) or --storage-root, never both");
        }
        if matches!(
            command,
            Commands::Extensions(extensions::Commands::Portfolio { .. })
        ) {
            anyhow::bail!(
                "portfolio requires explicit source mappings; --storage-root does not apply"
            );
        }
        if root == std::path::Path::new("userprofile")
            || root == std::path::Path::new("localappdata")
        {
            anyhow::bail!("a persistent verified root path is required; run dendro storage-init, then reuse its exact path with --storage-root <path>");
        }
        #[cfg(any(windows, target_os = "linux"))]
        let _admitted = dendro_core::platform_v1::open_existing_private_root(&root)?;
        #[cfg(not(any(windows, target_os = "linux")))]
        if !root.is_dir() {
            anyhow::bail!("storage root must be an existing directory");
        }
        if let Some(db) = database_slot(&mut command) {
            if db.as_path() == std::path::Path::new("dendro.duckdb") {
                *db = root.join("dendro.duckdb");
            } else {
                anyhow::bail!(
                    "use either --db or --storage-root, so the selected location is explicit"
                );
            }
        }
    } else {
        #[cfg(any(windows, target_os = "linux"))]
        prepare_default_database(&mut command, db_explicit)?;
        #[cfg(not(any(windows, target_os = "linux")))]
        let _ = db_explicit;
    }
    let rollback_requested = matches!(&command, Commands::Rollback { .. });
    match command {
        Commands::StorageInit => unreachable!(),
        Commands::RecoverSelection { db, cutoff } => {
            if let Some(path) = cutoff.as_ref() {
                dendro_core::refuse_secret_path(path)?;
            }
            #[cfg(any(windows, target_os = "linux"))]
            {
                let logical = std::path::absolute(&db)?;
                let choice = dendro_core::platform_v1::StorageRootChoice::Explicit(
                    logical
                        .parent()
                        .ok_or_else(|| anyhow::anyhow!("logical database has no parent"))?
                        .to_path_buf(),
                );
                use dendro_core::selection_binding::{
                    RecoveryDisposition, SelectionRecoveryOutcome,
                };
                let (digest, status, message) =
                    match dendro_core::selection_binding::recover_logical_db_with_outcome(
                        &db, &choice,
                    ) {
                        Ok((recovered, outcome)) => (
                            Some(hex_selection(&recovered.selection().sha256)),
                            match outcome {
                                SelectionRecoveryOutcome::Reconciled => "reconciled",
                                SelectionRecoveryOutcome::Unchanged => "unchanged",
                            },
                            None,
                        ),
                        Err(error) => {
                            if let Some(disposition) = error.downcast_ref::<RecoveryDisposition>() {
                                let status = match disposition {
                                    RecoveryDisposition::NothingToRecover => "nothing_to_recover",
                                    RecoveryDisposition::ResumeRequired => "resume_required",
                                };
                                (None, status, Some(disposition.to_string()))
                            } else {
                                return Err(error);
                            }
                        }
                    };
                let cutoff_outcome = if let Some(cutoff) = cutoff {
                    use dendro_core::sealed_run::CutoffRecoveryOutcome;
                    match dendro_core::sealed_run::recover_cutoff_manifest(&cutoff) {
                        Ok(CutoffRecoveryOutcome::Reconciled) => {
                            serde_json::json!({"status":"reconciled", "path":cutoff, "custody_reconciled":true})
                        }
                        Ok(CutoffRecoveryOutcome::Unchanged) => {
                            serde_json::json!({"status":"unchanged", "path":cutoff, "published_by_recovery":false, "recapture_required_for_retained_draft":true, "message":"No cutoff pointer was changed. Any retained draft remains unpublished; re-capture it before use."})
                        }
                        Err(error) => {
                            println!(
                                "{}",
                                serde_json::json!({"status":"partial_recovery", "analytics_selection_sha256":digest, "cutoff":{"status":"recovery_required", "path":cutoff, "error":dendro_core::redact(&error.to_string())}})
                            );
                            anyhow::bail!("cutoff recovery failed after analytics recovery; inspect the explicit recovery receipt");
                        }
                    }
                } else {
                    serde_json::json!({"status":"not_requested"})
                };
                println!(
                    "{}",
                    serde_json::json!({"status":status, "message":message, "published_by_recovery":false, "analytics_selection_sha256":digest, "cutoff":cutoff_outcome})
                );
                if status == "resume_required" {
                    anyhow::bail!(
                        "selected storage first publication requires resuming dendro run"
                    );
                }
            }
            #[cfg(not(any(windows, target_os = "linux")))]
            {
                let _ = (db, cutoff);
                anyhow::bail!("recover-selection is supported only on Windows and Linux");
            }
        }
        Commands::Migrate { db } | Commands::Rollback { db } => {
            #[cfg(any(windows, target_os = "linux"))]
            {
                let rollback = rollback_requested;
                let mut events = ShutdownEvents::new()?;
                let mut worker = tokio::task::spawn_blocking(
                    move || -> Result<serde_json::Value> {
                        dendro_core::refuse_secret_path(&db)?;
                        let logical = std::path::absolute(&db)?;
                        let choice = dendro_core::platform_v1::StorageRootChoice::Explicit(
                            logical
                                .parent()
                                .ok_or_else(|| anyhow::anyhow!("logical database has no parent"))?
                                .to_path_buf(),
                        );
                        if rollback {
                            let (selected, loss) =
                                dendro_core::selection_binding::rollback_logical_db(
                                    &db, &choice, false,
                                )?;
                            Ok(
                                serde_json::json!({"status":"schema3_selected", "schema_version":3, "analytics_selection_sha256":hex_selection(&selected.selection().sha256), "loss_receipt":loss}),
                            )
                        } else {
                            let selected =
                                dendro_core::selection_binding::migrate_logical_db(&db, &choice)?;
                            Ok(
                                serde_json::json!({"status":"schema4_selected", "schema_version":4, "analytics_selection_sha256":hex_selection(&selected.selection().sha256), "schema3_preimage_retained":true}),
                            )
                        }
                    },
                );
                let receipt = tokio::select! {
                    joined = &mut worker => joined??,
                    cause = events.next() => { std::process::exit(stop_cooperatively(worker, events, cause).await); }
                };
                println!("{receipt}");
            }
            #[cfg(not(any(windows, target_os = "linux")))]
            {
                let _ = db;
                anyhow::bail!(
                    "selected migration and rollback are supported only on Windows and Linux"
                );
            }
        }
        Commands::Extensions(command) => {
            let auto_ingest_target = match &command {
                extensions::Commands::Ui {
                    db,
                    rig,
                    auto_ingest: true,
                    ..
                }
                | extensions::Commands::Dashboard {
                    db,
                    rig,
                    auto_ingest: true,
                    ..
                } => Some((db.clone(), rig.clone())),
                _ => None,
            };
            if let Some((db, rig)) = auto_ingest_target {
                let needs_ingest = tokio::task::spawn_blocking({
                    let db = db.clone();
                    move || {
                        dendro_core::presentation::database_status(&db, rig.as_deref())
                            .map(|s| s["status"] == "not_initialized")
                    }
                })
                .await??;
                if needs_ingest {
                    let _ = run_ingest(db, Vec::new(), false, None, None).await?;
                }
            }
            extensions::run(command).await?;
        }
        Commands::Status { db, rig, format } => {
            let roots = match default_roots() {
                Ok(roots) => roots,
                Err(error) => {
                    eprintln!("dendro: {error}");
                    Vec::new()
                }
            };
            let status = tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
                let mut s = dendro_core::presentation::database_status(&db, rig.as_deref())?;
                let discovered: Vec<serde_json::Value> = roots
                    .into_iter()
                    .map(|r| {
                        serde_json::json!({
                            "name": r.name,
                            "path": dendro_core::redact(&r.path.display().to_string()),
                            "exists": r.path.exists()
                        })
                    })
                    .collect();
                s["discovered_sources"] = serde_json::json!(discovered);
                Ok(s)
            })
            .await??;
            if format.as_deref() == Some("json") {
                println!("{status}");
            } else {
                print!("{}", dendro_core::presentation::human_status(&status));
            }
        }
        Commands::IssueDraft {
            scope,
            metric,
            usd_per_million,
            publish,
            repo,
        } => {
            if publish {
                let _ = issue_publish_args(repo.as_deref().expect("clap requires repo"), "")?;
            }
            let report = canonical_decisions(scope).await?;
            let body =
                dendro_core::spotlight::issue_draft(&report["payload"], &metric, usd_per_million)?;
            if publish {
                let args = issue_publish_args(repo.as_deref().expect("clap requires repo"), &body)?;
                let result = tokio::task::spawn_blocking(move || {
                    std::process::Command::new("gh").args(args).output()
                })
                .await
                .map_err(|_| anyhow::anyhow!("issue publication worker failed"))?
                .map_err(|_| anyhow::anyhow!("issue publication command could not run"))?;
                if !result.status.success() {
                    anyhow::bail!("issue publication command failed");
                }
                println!(
                    "{}",
                    dendro_core::redact(&String::from_utf8_lossy(&result.stdout))
                );
            } else {
                print!("{body}");
            }
        }
        Commands::Assert {
            scope,
            max_rework_rate,
            max_idle_rate,
        } => {
            let report = canonical_decisions(scope).await?;
            let verdict = dendro_core::spotlight::assertion(
                &report["payload"],
                max_rework_rate,
                max_idle_rate,
            );
            eprintln!("{verdict}");
            std::process::exit(verdict["exit_code"].as_i64().unwrap_or(3) as i32);
        }
        Commands::Watch {
            db,
            interval,
            duration,
            once,
            harness,
            flush_every,
        } => {
            let summary = run_watch(db, interval, duration, once, harness, flush_every).await?;
            println!("{summary}");
            return Ok(());
        }
        Commands::RunAgent {
            db,
            harness,
            interval,
            flush_every,
            command,
        } => {
            let code = run_agent(db, harness, interval, flush_every, command).await?;
            std::process::exit(code);
        }
        Commands::Run {
            db,
            root,
            frozen,
            write_manifest,
            sealed,
        } => {
            let summary = run_ingest(db, root, frozen, write_manifest, sealed).await?;
            println!("{summary}");
            return Ok(());
        }
        Commands::Report {
            kind,
            format,
            viz,
            anomaly,
            friction,
            db,
            rig,
            window,
            from,
            to,
            tool_loop_failures,
            tool_loop_window_secs,
            handoff_loop_returns,
            friction_thresholds,
            advisory_thresholds,
        } => {
            let thresholds = dendro_core::LoopThresholds {
                tool_loop_failures,
                tool_loop_window_secs,
                handoff_loop_returns,
            };
            if friction && kind != "handoffs" {
                anyhow::bail!("--friction is supported only by report handoffs");
            }
            if friction_thresholds.supplied() && !matches!(kind.as_str(), "handoffs" | "decisions")
            {
                anyhow::bail!("friction thresholds require report handoffs or decisions");
            }
            if (advisory_thresholds.supplied() || anomaly.is_some())
                && !matches!(
                    kind.as_str(),
                    "advisory" | "anomalies" | "spotlights" | "decisions"
                )
            {
                anyhow::bail!("advisory options and formats require advisory, anomalies, spotlights or decisions");
            }
            if format.as_deref() == Some("ndjson")
                && !matches!(
                    kind.as_str(),
                    "advisory" | "anomalies" | "spotlights" | "decisions"
                )
            {
                anyhow::bail!(
                    "NDJSON findings require advisory, anomalies, spotlights or decisions"
                );
            }
            if anomaly.is_some() && kind != "advisory" {
                anyhow::bail!("--anomaly requires report advisory");
            }
            let evaluated_kind = kind.clone();
            let report = tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
                let request = dendro_core::presentation::ReadRequest::open(&db)?;
                let (from, to, mode) = if window.is_some() {
                    let (f, t, m) =
                        request.window(rig.as_deref(), window.as_deref(), None, None)?;
                    (f, t, Some(m))
                } else {
                    (from, to, None)
                };
                let mut r = dendro_core::pipeline::report_json_with_analysis_physical(
                    request.database(),
                    &evaluated_kind,
                    rig.as_deref(),
                    &from,
                    &to,
                    thresholds,
                    friction_thresholds.thresholds(),
                    advisory_thresholds.options(),
                )?;
                if let Some(m) = mode {
                    r["window_resolution"] = serde_json::json!(m);
                }
                Ok(r)
            })
            .await??;
            if viz {
                print!("{}", dendro_core::presentation::viz(&report));
                return Ok(());
            }
            let format = format.unwrap_or_else(|| {
                if std::io::stdout().is_terminal() {
                    "table".into()
                } else {
                    "json".into()
                }
            });
            match format.as_str() {
                "table" => print!("{}", dendro_core::presentation::human(&report, false)),
                "markdown" if !matches!(kind.as_str(), "advisory" | "anomalies" | "spotlights") => {
                    print!("{}", dendro_core::presentation::human(&report, true))
                }
                "markdown" => {
                    let field = if kind == "advisory" {
                        "advisory_idle_claimed"
                    } else {
                        "anomaly_spotlights"
                    };
                    print!(
                        "{}",
                        dendro_core::spotlight::markdown(&report["payload"][field])
                    );
                }
                "ndjson" => {
                    let field = if kind == "advisory" {
                        "advisory_idle_claimed"
                    } else {
                        "anomaly_spotlights"
                    };
                    let value = &report["payload"][field];
                    for record in dendro_core::spotlight::ndjson_records(value) {
                        println!("{record}");
                    }
                }
                _ => println!("{report}"),
            }
        }
        Commands::Trace {
            db,
            metric_id,
            rig,
            window,
            from,
            to,
            tool_loop_failures,
            tool_loop_window_secs,
            handoff_loop_returns,
            friction_thresholds,
            advisory_thresholds,
        } => {
            let thresholds = dendro_core::LoopThresholds {
                tool_loop_failures,
                tool_loop_window_secs,
                handoff_loop_returns,
            };
            let report = tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
                let request = dendro_core::presentation::ReadRequest::open(&db)?;
                let (from, to, mode) = if window.is_some() {
                    let (f, t, m) =
                        request.window(rig.as_deref(), window.as_deref(), None, None)?;
                    (f, t, Some(m))
                } else {
                    (from, to, None)
                };
                let mut r = dendro_core::pipeline::trace_json_with_analysis_physical(
                    request.database(),
                    &metric_id,
                    rig.as_deref(),
                    &from,
                    &to,
                    thresholds,
                    friction_thresholds.thresholds(),
                    advisory_thresholds.options(),
                )?;
                if let Some(m) = mode {
                    r["window_resolution"] = serde_json::json!(m);
                }
                Ok(r)
            })
            .await??;
            println!("{report}");
        }
        Commands::Insights {
            db,
            json,
            harness,
            session,
        } => {
            let results = tokio::task::spawn_blocking(
                move || -> Result<Vec<dendro_core::insight_v1::InsightResult>> {
                    let request = dendro_core::presentation::ReadRequest::open(&db)?;
                    let store = request.open_store()?;
                    Ok(dendro_core::insight_v1::evaluate(
                        &store,
                        &dendro_core::insight_v1::Scope {
                            harness,
                            session,
                            from: None,
                            to: None,
                        },
                    ))
                },
            )
            .await??;
            println!("{}", render_insights(&results, json)?);
        }
    }

    Ok(())
}

fn render_insights(
    results: &[dendro_core::insight_v1::InsightResult],
    json: bool,
) -> Result<String> {
    if json {
        Ok(serde_json::to_string(results)?)
    } else {
        Ok(dendro_core::insight_v1::format_text(results))
    }
}

/// Native `STATUS_CONTROL_C_EXIT`. This is a Windows console status, not a Unix signal.
#[cfg(any(windows, test))]
const WINDOWS_CTRL_C_STATUS: i32 = 0xC000_013A_u32 as i32;

const CLOSE_CLEANUP_MESSAGE: &str = "\
dendro: console close, logoff, or shutdown does not promise asynchronous cleanup time; \
staging cleanup FAILED and is left for the next run's recovery";

/// What actually woke the process. Unix variants are only produced from Unix
/// signals. Windows variants are only produced from console control events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShutdownCause {
    #[cfg(any(unix, test))]
    UnixInterrupt,
    #[cfg(any(unix, test))]
    UnixTerminate,
    #[cfg(any(windows, test))]
    WindowsCtrlC,
    #[cfg(any(windows, test))]
    WindowsCtrlBreak,
    #[cfg(any(windows, test))]
    WindowsCtrlClose,
    #[cfg(any(windows, test))]
    WindowsCtrlLogoff,
    #[cfg(any(windows, test))]
    WindowsCtrlShutdown,
}

impl ShutdownCause {
    fn exit_code(self) -> i32 {
        match self {
            #[cfg(any(unix, test))]
            Self::UnixInterrupt => 130,
            #[cfg(any(unix, test))]
            Self::UnixTerminate => 143,
            #[cfg(any(windows, test))]
            Self::WindowsCtrlC | Self::WindowsCtrlBreak => WINDOWS_CTRL_C_STATUS,
            #[cfg(any(windows, test))]
            Self::WindowsCtrlClose | Self::WindowsCtrlLogoff | Self::WindowsCtrlShutdown => 2,
        }
    }

    /// First interactive cancel may wait for the worker. Console close, logoff,
    /// and shutdown do not. This does not translate one platform into the other.
    fn allows_bounded_cleanup(self) -> bool {
        match self {
            #[cfg(any(unix, test))]
            Self::UnixInterrupt | Self::UnixTerminate => true,
            #[cfg(any(windows, test))]
            Self::WindowsCtrlC | Self::WindowsCtrlBreak => true,
            #[cfg(any(windows, test))]
            Self::WindowsCtrlClose | Self::WindowsCtrlLogoff | Self::WindowsCtrlShutdown => false,
        }
    }

    #[cfg(test)]
    fn native_name(self) -> &'static str {
        match self {
            Self::UnixInterrupt => "unix-sigint",
            Self::UnixTerminate => "unix-sigterm",
            Self::WindowsCtrlC => "windows-ctrl-c",
            Self::WindowsCtrlBreak => "windows-ctrl-break",
            Self::WindowsCtrlClose => "windows-ctrl-close",
            Self::WindowsCtrlLogoff => "windows-ctrl-logoff",
            Self::WindowsCtrlShutdown => "windows-ctrl-shutdown",
        }
    }
}

pub(crate) struct ShutdownEvents {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(windows)]
    ctrl_c: tokio::signal::windows::CtrlC,
    #[cfg(windows)]
    ctrl_break: tokio::signal::windows::CtrlBreak,
    #[cfg(windows)]
    ctrl_close: tokio::signal::windows::CtrlClose,
    #[cfg(windows)]
    ctrl_logoff: tokio::signal::windows::CtrlLogoff,
    #[cfg(windows)]
    ctrl_shutdown: tokio::signal::windows::CtrlShutdown,
}

impl ShutdownEvents {
    pub(crate) fn new() -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                interrupt: tokio::signal::unix::signal(
                    tokio::signal::unix::SignalKind::interrupt(),
                )?,
                terminate: tokio::signal::unix::signal(
                    tokio::signal::unix::SignalKind::terminate(),
                )?,
            })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                ctrl_c: tokio::signal::windows::ctrl_c()?,
                ctrl_break: tokio::signal::windows::ctrl_break()?,
                ctrl_close: tokio::signal::windows::ctrl_close()?,
                ctrl_logoff: tokio::signal::windows::ctrl_logoff()?,
                ctrl_shutdown: tokio::signal::windows::ctrl_shutdown()?,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            compile_error!("ShutdownEvents requires Unix signals or Windows console events");
        }
    }

    pub(crate) async fn next(&mut self) -> ShutdownCause {
        #[cfg(unix)]
        {
            tokio::select! {
                biased;
                _ = self.interrupt.recv() => ShutdownCause::UnixInterrupt,
                _ = self.terminate.recv() => ShutdownCause::UnixTerminate,
            }
        }
        #[cfg(windows)]
        {
            tokio::select! {
                biased;
                _ = self.ctrl_c.recv() => ShutdownCause::WindowsCtrlC,
                _ = self.ctrl_break.recv() => ShutdownCause::WindowsCtrlBreak,
                _ = self.ctrl_close.recv() => ShutdownCause::WindowsCtrlClose,
                _ = self.ctrl_logoff.recv() => ShutdownCause::WindowsCtrlLogoff,
                _ = self.ctrl_shutdown.recv() => ShutdownCause::WindowsCtrlShutdown,
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            compile_error!("ShutdownEvents requires Unix signals or Windows console events");
        }
    }
}

/// Asks the owned worker to stop and returns the process exit code.
/// Unix SIGINT/SIGTERM and Windows CTRL_C/CTRL_BREAK wait up to [`SHUTDOWN_LIMIT`].
/// A second native event or that time limit stops waiting without deleting
/// anything the worker may still write. Windows close, logoff, and shutdown do
/// not promise asynchronous cleanup time, so they do not wait. Unfinished
/// cleanup is left for the next run's recovery.
async fn stop_cooperatively(
    mut worker: JoinHandle<Result<serde_json::Value>>,
    mut events: ShutdownEvents,
    cause: ShutdownCause,
) -> i32 {
    let code = cause.exit_code();
    dendro_core::RUN_CANCEL.request();
    if !cause.allows_bounded_cleanup() {
        if worker.is_finished() {
            return report_joined(Some(worker.await), code);
        }
        eprintln!("{CLOSE_CLEANUP_MESSAGE}");
        return code;
    }
    eprintln!("dendro: stopping; removing private staging");
    let waited = tokio::time::timeout(SHUTDOWN_LIMIT, async {
        tokio::select! {
            joined = &mut worker => Some(joined),
            _ = events.next() => None,
        }
    })
    .await;
    let joined = waited.unwrap_or_default();
    report_joined(joined, code)
}

fn report_joined(
    joined: Option<Result<Result<serde_json::Value, anyhow::Error>, tokio::task::JoinError>>,
    code: i32,
) -> i32 {
    match joined {
        Some(Ok(Ok(summary))) => {
            println!("{summary}");
            eprintln!("dendro: the run had already completed");
            0
        }
        Some(Ok(Err(error))) if error.downcast_ref::<dendro_core::Interrupted>().is_some() => {
            eprintln!("dendro: interrupted; private staging removed");
            code
        }
        Some(Ok(Err(error))) => {
            eprintln!(
                "dendro: interrupted with error: {}",
                dendro_core::redact(&format!("{error:#}"))
            );
            code
        }
        Some(Err(_)) => {
            eprintln!("dendro: interrupted; the worker failed; staging cleanup not confirmed");
            code
        }
        None => {
            eprintln!("dendro: interrupted before shutdown finished; staging cleanup FAILED and is left for the next run's recovery");
            code
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostKind {
    Windows,
    Linux,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HomeQuery {
    home: Option<String>,
    userprofile: Option<String>,
    openrig_home: Option<String>,
    euid_is_root: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HomeRefusal {
    NoTrustworthyRoot,
    PrivilegedForeignHome,
    RelativeRoot,
    UnadmittedUnc,
    UnadmittedDevice,
    NonRoundtrip,
    #[cfg(target_os = "linux")]
    EuidUnknown,
    UnsupportedPlatform,
}

impl HomeRefusal {
    fn message(self) -> &'static str {
        match self {
            Self::NoTrustworthyRoot => {
                "no trustworthy default source root; current directory is not a fallback"
            }
            Self::PrivilegedForeignHome => "privileged process will not adopt a foreign home",
            Self::RelativeRoot => "default source root must be an absolute native path",
            Self::UnadmittedUnc => "UNC default source root is not admitted",
            Self::UnadmittedDevice => {
                "device, volume-guid, or alternate-stream default source root is not admitted"
            }
            Self::NonRoundtrip => "default source root is not valid Unicode",
            #[cfg(target_os = "linux")]
            Self::EuidUnknown => "process effective user is unknown; default source root refused",
            Self::UnsupportedPlatform => {
                "this operating system has no admitted default source root"
            }
        }
    }
}

pub(crate) fn default_roots() -> Result<Vec<dendro_connectors::SourceRoot>> {
    let host = live_host().map_err(|refusal| anyhow::anyhow!(refusal.message()))?;
    let query = live_home_query().map_err(|refusal| anyhow::anyhow!(refusal.message()))?;
    configured_source_roots(host, &query).map_err(|refusal| anyhow::anyhow!(refusal.message()))
}

fn configured_source_roots(
    host: HostKind,
    query: &HomeQuery,
) -> Result<Vec<dendro_connectors::SourceRoot>, HomeRefusal> {
    let home = admitted_user_home(host, query)?;
    let openrig = admitted_openrig_home(host, query, &home)?;
    Ok(vec![
        source_root("claude", home.join(".claude").join("projects")),
        source_root("codex", home.join(".codex").join("sessions")),
        source_root("pi", openrig.join("state").join("pi")),
        source_root("pi", home.join(".pi").join("agent").join("sessions")),
        source_root("openrig", openrig.join("openrig.sqlite")),
        source_root(
            "agy",
            home.join(".gemini").join("antigravity-cli").join("brain"),
        ),
        source_root("grok", home.join(".grok")),
    ])
}

fn source_root(name: &str, path: PathBuf) -> dendro_connectors::SourceRoot {
    dendro_connectors::SourceRoot {
        name: name.to_string(),
        path,
    }
}

fn admitted_user_home(host: HostKind, query: &HomeQuery) -> Result<PathBuf, HomeRefusal> {
    match host {
        HostKind::Windows => {
            let Some(profile) = query.userprofile.as_deref() else {
                return Err(HomeRefusal::NoTrustworthyRoot);
            };
            admit_windows_absolute(profile)?;
            Ok(PathBuf::from(profile))
        }
        HostKind::Linux => {
            if query.euid_is_root {
                return match query.home.as_deref().map(|path| path.trim_end_matches('/')) {
                    Some("/root") => Ok(PathBuf::from("/root")),
                    Some(_) => Err(HomeRefusal::PrivilegedForeignHome),
                    None => Err(HomeRefusal::NoTrustworthyRoot),
                };
            }
            let Some(home) = query.home.as_deref() else {
                return Err(HomeRefusal::NoTrustworthyRoot);
            };
            admit_linux_absolute(home)?;
            Ok(PathBuf::from(home.trim_end_matches('/')))
        }
    }
}

fn admitted_openrig_home(
    host: HostKind,
    query: &HomeQuery,
    home: &std::path::Path,
) -> Result<PathBuf, HomeRefusal> {
    let Some(explicit) = query.openrig_home.as_deref() else {
        return Ok(home.join(".openrig"));
    };
    match host {
        HostKind::Windows => admit_windows_absolute(explicit)?,
        HostKind::Linux => {
            admit_linux_absolute(explicit)?;
            if query.euid_is_root && !linux_under_root_home(explicit) {
                return Err(HomeRefusal::PrivilegedForeignHome);
            }
        }
    }
    Ok(PathBuf::from(explicit))
}

fn admit_linux_absolute(path: &str) -> Result<(), HomeRefusal> {
    if path.contains('\0') {
        return Err(HomeRefusal::NonRoundtrip);
    }
    if !path.starts_with('/') || lexical_has_dot_segments(path) {
        return Err(HomeRefusal::RelativeRoot);
    }
    Ok(())
}

fn admit_windows_absolute(path: &str) -> Result<(), HomeRefusal> {
    if path.contains('\0') {
        return Err(HomeRefusal::NonRoundtrip);
    }
    if lexical_has_dot_segments(path) {
        return Err(HomeRefusal::RelativeRoot);
    }
    match dendro_core::platform_v1::classify_windows_path(path) {
        dendro_core::platform_v1::PathClass::LocalDrive
        | dendro_core::platform_v1::PathClass::LocalDriveVerbatim => Ok(()),
        dendro_core::platform_v1::PathClass::Unc => Err(HomeRefusal::UnadmittedUnc),
        dendro_core::platform_v1::PathClass::Device
        | dendro_core::platform_v1::PathClass::VolumeGuid
        | dendro_core::platform_v1::PathClass::AlternateStream => {
            Err(HomeRefusal::UnadmittedDevice)
        }
        dendro_core::platform_v1::PathClass::Relative
        | dendro_core::platform_v1::PathClass::Other => Err(HomeRefusal::RelativeRoot),
    }
}

fn lexical_has_dot_segments(path: &str) -> bool {
    path.split(['/', '\\'])
        .any(|segment| segment == "." || segment == "..")
}

fn linux_under_root_home(path: &str) -> bool {
    let trimmed = path.trim_end_matches('/');
    trimmed == "/root" || trimmed.starts_with("/root/")
}

fn live_host() -> Result<HostKind, HomeRefusal> {
    if cfg!(windows) {
        Ok(HostKind::Windows)
    } else if cfg!(target_os = "linux") {
        Ok(HostKind::Linux)
    } else {
        Err(HomeRefusal::UnsupportedPlatform)
    }
}

fn live_home_query() -> Result<HomeQuery, HomeRefusal> {
    let home = match optional_env("HOME") {
        Ok(value) => value,
        Err(HomeRefusal::NonRoundtrip) if cfg!(windows) => None,
        Err(refusal) => return Err(refusal),
    };
    let euid_is_root = {
        #[cfg(target_os = "linux")]
        {
            linux_euid_is_root()?
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    };
    Ok(HomeQuery {
        home,
        userprofile: optional_env("USERPROFILE")?,
        openrig_home: optional_env("OPENRIG_HOME")?,
        euid_is_root,
    })
}

fn optional_env(key: &str) -> Result<Option<String>, HomeRefusal> {
    match std::env::var_os(key) {
        None => Ok(None),
        Some(value) => match value.to_str() {
            Some("") => Ok(None),
            Some(text) => Ok(Some(text.to_string())),
            None => Err(HomeRefusal::NonRoundtrip),
        },
    }
}

#[cfg(target_os = "linux")]
fn linux_euid_is_root() -> Result<bool, HomeRefusal> {
    let text =
        std::fs::read_to_string("/proc/self/status").map_err(|_| HomeRefusal::EuidUnknown)?;
    let euid = dendro_core::platform_v1::parse_proc_status_euid(&text)
        .map_err(|_| HomeRefusal::EuidUnknown)?;
    Ok(euid == 0)
}

fn parse_roots(values: &[String]) -> Result<Vec<dendro_connectors::SourceRoot>> {
    let mut roots = Vec::new();
    for value in values {
        let Some((name, path)) = value.split_once('=') else {
            anyhow::bail!("root overrides use name=path");
        };
        roots.push(dendro_connectors::SourceRoot {
            name: name.to_string(),
            path: PathBuf::from(path),
        });
    }
    for root in &mut roots {
        if root.name == "openrig" {
            root.path = dendro_core::absolute_openrig_root(&root.path);
        }
    }
    let openrig: Vec<PathBuf> = roots
        .iter()
        .filter(|root| root.name == "openrig")
        .map(|root| root.path.clone())
        .collect();
    // Before the database is opened or any source is read.
    dendro_core::refuse_multiple_openrig_roots(&openrig)?;
    let mut seen = Vec::new();
    roots.retain(|root| {
        let key = (root.name.clone(), dendro_core::canonical_root(&root.path));
        if seen.contains(&key) {
            return false;
        }
        seen.push(key);
        true
    });
    Ok(roots)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dendro-cli-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn two_openrig_roots_in_any_position_are_refused() {
        let dir = scratch("two");
        let first = dir.join("a.sqlite");
        let second = dir.join("nested").join("b.sqlite");
        std::fs::create_dir_all(second.parent().unwrap()).unwrap();
        std::fs::write(&first, []).unwrap();
        std::fs::write(&second, []).unwrap();
        let err = parse_roots(&[
            format!("claude={}", dir.display()),
            format!("openrig={}", first.display()),
            format!("codex={}", dir.display()),
            format!("openrig={}", second.display()),
        ])
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("v0.1.0 supports exactly one OpenRig root per analytics database"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_openrig_file_under_two_spellings_is_kept_once() {
        let dir = scratch("spell");
        let file = dir.join("sub").join("openrig.sqlite");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, []).unwrap();
        let via_parent = dir
            .join("sub")
            .join("extra")
            .join("..")
            .join("openrig.sqlite");
        let roots = parse_roots(&[
            format!("openrig={}", file.display()),
            format!("pi={}", dir.display()),
            format!("openrig={}", via_parent.display()),
        ])
        .unwrap();
        let openrig: Vec<_> = roots.iter().filter(|root| root.name == "openrig").collect();
        assert_eq!(openrig.len(), 1);
        assert!(openrig[0].path.is_absolute());
        assert_eq!(openrig[0].path, dendro_core::absolute_openrig_root(&file));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_relative_openrig_root_is_stored_as_an_absolute_path() {
        let dir = scratch("rel");
        let file = dir.join("nested").join("openrig.sqlite");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, []).unwrap();
        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let roots = parse_roots(&["openrig=nested/openrig.sqlite".into()]).unwrap();
        std::env::set_current_dir(&previous).unwrap();
        assert_eq!(roots.len(), 1);
        assert!(roots[0].path.is_absolute());
        assert_eq!(roots[0].path, std::fs::canonicalize(&file).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_roots_and_parse_roots_pi_dual_root() {
        let defaults = configured_source_roots(
            HostKind::Linux,
            &HomeQuery {
                home: Some("/home/dendro".into()),
                userprofile: None,
                openrig_home: None,
                euid_is_root: false,
            },
        )
        .expect("absolute linux home");
        let pi_roots: Vec<_> = defaults.iter().filter(|r| r.name == "pi").collect();
        assert_eq!(pi_roots.len(), 2);
        assert!(pi_roots[0].path.ends_with("state/pi"));
        assert!(pi_roots[1].path.ends_with(".pi/agent/sessions"));

        let dir = scratch("pi_roots");
        let path1 = dir.join("openrig_state_pi");
        let path2 = dir.join("pi_agent_sessions");
        std::fs::create_dir_all(&path1).unwrap();
        std::fs::create_dir_all(&path2).unwrap();

        // Distinct pi roots are both kept
        let roots = parse_roots(&[
            format!("pi={}", path1.display()),
            format!("pi={}", path2.display()),
        ])
        .unwrap();
        let parsed_pi: Vec<_> = roots.iter().filter(|r| r.name == "pi").collect();
        assert_eq!(parsed_pi.len(), 2);

        // Identical pi roots are deduplicated
        let path1_redundant = dir.join("sub").join("..").join("openrig_state_pi");
        let roots_dedup = parse_roots(&[
            format!("pi={}", path1.display()),
            format!("pi={}", path1_redundant.display()),
            format!("pi={}", path2.display()),
        ])
        .unwrap();
        let parsed_dedup: Vec<_> = roots_dedup.iter().filter(|r| r.name == "pi").collect();
        assert_eq!(parsed_dedup.len(), 2);
        assert_eq!(
            dendro_core::canonical_root(&parsed_dedup[0].path),
            dendro_core::canonical_root(&path1)
        );
        assert_eq!(
            dendro_core::canonical_root(&parsed_dedup[1].path),
            dendro_core::canonical_root(&path2)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn home_query(
        home: Option<&str>,
        userprofile: Option<&str>,
        openrig_home: Option<&str>,
        euid_is_root: bool,
    ) -> HomeQuery {
        HomeQuery {
            home: home.map(str::to_string),
            userprofile: userprofile.map(str::to_string),
            openrig_home: openrig_home.map(str::to_string),
            euid_is_root,
        }
    }

    #[test]
    fn windows_userprofile_wins_over_foreign_home() {
        let roots = configured_source_roots(
            HostKind::Windows,
            &home_query(
                Some("/c/Users/foreign"),
                Some(r"C:\Users\native"),
                None,
                false,
            ),
        )
        .expect("native profile");
        assert_eq!(roots.len(), 7);
        assert!(roots
            .iter()
            .all(|root| root.path.starts_with(r"C:\Users\native")));
        assert!(roots
            .iter()
            .all(|root| !root.path.to_string_lossy().contains("foreign")));
        let pi: Vec<_> = roots.iter().filter(|root| root.name == "pi").collect();
        assert_eq!(pi.len(), 2);
        assert!(pi[0].path.ends_with(r"state\pi") || pi[0].path.ends_with("state/pi"));
        assert!(
            pi[1].path.ends_with(".pi/agent/sessions")
                || pi[1].path.ends_with(r".pi\agent\sessions")
        );
    }

    #[test]
    fn windows_home_or_dot_is_not_a_default_root() {
        let missing = configured_source_roots(
            HostKind::Windows,
            &home_query(Some(r"C:\Users\from-home"), None, None, false),
        )
        .expect_err("HOME is not the Windows authority");
        assert_eq!(missing, HomeRefusal::NoTrustworthyRoot);
        let dotted =
            configured_source_roots(HostKind::Windows, &home_query(None, Some("."), None, false))
                .expect_err("dot profile");
        assert_eq!(dotted, HomeRefusal::RelativeRoot);
        let unc = configured_source_roots(
            HostKind::Windows,
            &home_query(None, Some(r"\\server\share\Users\a"), None, false),
        )
        .expect_err("unc profile");
        assert_eq!(unc, HomeRefusal::UnadmittedUnc);
    }

    #[test]
    fn linux_missing_or_relative_home_does_not_become_dot() {
        let missing =
            configured_source_roots(HostKind::Linux, &home_query(None, None, None, false))
                .expect_err("missing home");
        assert_eq!(missing, HomeRefusal::NoTrustworthyRoot);
        assert!(!missing.message().contains("HOME fallback dot"));
        assert!(missing
            .message()
            .contains("current directory is not a fallback"));
        let dotted =
            configured_source_roots(HostKind::Linux, &home_query(Some("."), None, None, false))
                .expect_err("dot home");
        assert_eq!(dotted, HomeRefusal::RelativeRoot);
        let walked = configured_source_roots(
            HostKind::Linux,
            &home_query(Some("/home/dendro/../other"), None, None, false),
        )
        .expect_err("dotdot home");
        assert_eq!(walked, HomeRefusal::RelativeRoot);
    }

    #[test]
    fn privileged_linux_refuses_foreign_home_and_admits_own_root() {
        let foreign = configured_source_roots(
            HostKind::Linux,
            &home_query(Some("/home/other"), None, None, true),
        )
        .expect_err("foreign home");
        assert_eq!(foreign, HomeRefusal::PrivilegedForeignHome);
        let explicit_foreign = configured_source_roots(
            HostKind::Linux,
            &home_query(Some("/root"), None, Some("/home/other/.openrig"), true),
        )
        .expect_err("foreign openrig");
        assert_eq!(explicit_foreign, HomeRefusal::PrivilegedForeignHome);
        let roots = configured_source_roots(
            HostKind::Linux,
            &home_query(Some("/root"), None, None, true),
        )
        .expect("root own home");
        assert!(roots.iter().all(|root| root.path.starts_with("/root")));
        assert_eq!(
            roots
                .iter()
                .find(|root| root.name == "openrig")
                .expect("openrig")
                .path,
            PathBuf::from("/root/.openrig/openrig.sqlite")
        );
    }

    #[test]
    fn relative_openrig_home_is_refused_for_a_normal_linux_user() {
        let err = configured_source_roots(
            HostKind::Linux,
            &home_query(Some("/home/dendro"), None, Some("relative/.openrig"), false),
        )
        .expect_err("relative openrig");
        assert_eq!(err, HomeRefusal::RelativeRoot);
        let roots = configured_source_roots(
            HostKind::Linux,
            &home_query(Some("/home/dendro"), None, Some("/opt/openrig"), false),
        )
        .expect("absolute openrig");
        assert_eq!(
            roots
                .iter()
                .find(|root| root.name == "openrig")
                .expect("openrig")
                .path,
            PathBuf::from("/opt/openrig/openrig.sqlite")
        );
    }

    #[test]
    fn shutdown_causes_keep_platform_exit_codes_without_signal_emulation() {
        assert_eq!(ShutdownCause::UnixInterrupt.exit_code(), 130);
        assert_eq!(ShutdownCause::UnixTerminate.exit_code(), 143);
        assert!(ShutdownCause::UnixInterrupt.allows_bounded_cleanup());
        assert!(ShutdownCause::UnixTerminate.allows_bounded_cleanup());
        assert_eq!(
            ShutdownCause::WindowsCtrlC.exit_code(),
            WINDOWS_CTRL_C_STATUS
        );
        assert_eq!(
            ShutdownCause::WindowsCtrlBreak.exit_code(),
            WINDOWS_CTRL_C_STATUS
        );
        assert_ne!(ShutdownCause::WindowsCtrlC.exit_code(), 130);
        assert_ne!(ShutdownCause::WindowsCtrlBreak.exit_code(), 143);
        assert!(ShutdownCause::WindowsCtrlC.allows_bounded_cleanup());
        assert!(ShutdownCause::WindowsCtrlBreak.allows_bounded_cleanup());
        for cause in [
            ShutdownCause::WindowsCtrlClose,
            ShutdownCause::WindowsCtrlLogoff,
            ShutdownCause::WindowsCtrlShutdown,
        ] {
            assert!(!cause.allows_bounded_cleanup());
            assert_eq!(cause.exit_code(), 2);
            assert!(!cause.native_name().contains("sig"));
        }
        assert_eq!(ShutdownCause::WindowsCtrlC.native_name(), "windows-ctrl-c");
        assert!(ShutdownCause::UnixInterrupt
            .native_name()
            .contains("sigint"));
    }

    #[test]
    fn live_default_roots_do_not_use_the_current_directory() {
        match default_roots() {
            Ok(roots) => {
                assert!(!roots.is_empty());
                assert!(roots.iter().all(|root| root.path.is_absolute()));
                assert!(roots.iter().all(|root| {
                    let text = root.path.to_string_lossy();
                    text != "." && !text.starts_with("./") && !text.starts_with(".\\")
                }));
            }
            Err(error) => {
                let text = error.to_string();
                assert!(
                    text.contains("current directory is not a fallback")
                        || text.contains("foreign home")
                        || text.contains("absolute native path")
                        || text.contains("not admitted")
                        || text.contains("not valid Unicode")
                        || text.contains("effective user is unknown")
                        || text.contains("no admitted default source root"),
                    "unexpected refusal: {text}"
                );
            }
        }
    }

    #[test]
    fn cli_report_and_trace_threshold_flags() {
        let cli_default = Cli::try_parse_from(["dendro", "report", "decisions"]).unwrap();
        match cli_default.command {
            Commands::Report {
                tool_loop_failures,
                tool_loop_window_secs,
                handoff_loop_returns,
                ..
            } => {
                assert_eq!(tool_loop_failures, 3);
                assert_eq!(tool_loop_window_secs, 600);
                assert_eq!(handoff_loop_returns, 2);
            }
            _ => panic!("expected Report command"),
        }

        let cli_custom = Cli::try_parse_from([
            "dendro",
            "report",
            "decisions",
            "--tool-loop-failures",
            "4",
            "--tool-loop-window-secs",
            "1200",
            "--handoff-loop-returns",
            "1",
        ])
        .unwrap();
        match cli_custom.command {
            Commands::Report {
                tool_loop_failures,
                tool_loop_window_secs,
                handoff_loop_returns,
                ..
            } => {
                assert_eq!(tool_loop_failures, 4);
                assert_eq!(tool_loop_window_secs, 1200);
                assert_eq!(handoff_loop_returns, 1);
            }
            _ => panic!("expected Report command"),
        }

        let cli_trace = Cli::try_parse_from([
            "dendro",
            "trace",
            "--metric-id",
            "m1",
            "--tool-loop-failures",
            "2",
            "--tool-loop-window-secs",
            "60",
            "--handoff-loop-returns",
            "0",
        ])
        .unwrap();
        match cli_trace.command {
            Commands::Trace {
                metric_id,
                tool_loop_failures,
                tool_loop_window_secs,
                handoff_loop_returns,
                ..
            } => {
                assert_eq!(metric_id, "m1");
                assert_eq!(tool_loop_failures, 2);
                assert_eq!(tool_loop_window_secs, 60);
                assert_eq!(handoff_loop_returns, 0);
            }
            _ => panic!("expected Trace command"),
        }

        let cli_neg = Cli::try_parse_from([
            "dendro",
            "report",
            "decisions",
            "--tool-loop-window-secs",
            "-1",
        ])
        .unwrap();
        match cli_neg.command {
            Commands::Report {
                tool_loop_window_secs,
                ..
            } => {
                assert_eq!(tool_loop_window_secs, -1);
            }
            _ => panic!("expected Report command"),
        }
    }
}

#[cfg(test)]
mod friction_tests {
    use super::*;

    #[test]
    fn friction_defaults_and_report_trace_duration_normalization() {
        for args in [
            vec!["dendro", "report", "handoffs"],
            vec!["dendro", "trace", "--metric-id", "handoff_friction"],
        ] {
            let parsed = Cli::try_parse_from(args).unwrap();
            let thresholds = match parsed.command {
                Commands::Report {
                    friction_thresholds,
                    ..
                }
                | Commands::Trace {
                    friction_thresholds,
                    ..
                } => friction_thresholds.thresholds(),
                _ => panic!("expected report/trace"),
            };
            assert_eq!(thresholds, dendro_core::FrictionThresholds::default());
        }
        for value in ["600s", "10m"] {
            let cli = Cli::try_parse_from([
                "dendro",
                "report",
                "handoffs",
                "--friction-loop-duration",
                value,
                "--friction-return-rate",
                "0.30",
                "--friction-chain-returns",
                "0",
            ])
            .unwrap();
            if let Commands::Report {
                friction_thresholds,
                ..
            } = cli.command
            {
                assert_eq!(friction_thresholds.thresholds().loop_duration_secs, 600);
                assert_eq!(friction_thresholds.thresholds().return_rate, 0.30);
                assert_eq!(friction_thresholds.thresholds().chain_returns, 0);
            } else {
                panic!("expected report")
            }
        }
    }
    #[test]
    fn invalid_thresholds_fail_during_clap_parsing() {
        for (flag, values) in [
            ("--friction-return-rate", vec!["NaN", "inf", "-1", "1.01"]),
            (
                "--friction-chain-returns",
                vec!["-1", "1.5", "18446744073709551616"],
            ),
            ("--friction-rework-tokens", vec!["-1", "1.5"]),
            (
                "--friction-loop-duration",
                vec!["600", "-1s", "1.5m", "18446744073709551615m"],
            ),
        ] {
            for value in values {
                assert!(
                    Cli::try_parse_from(["dendro", "report", "handoffs", flag, value]).is_err(),
                    "{flag} {value}"
                );
            }
        }
        assert!(Cli::try_parse_from([
            "dendro",
            "trace",
            "--metric-id",
            "handoff_friction",
            "--friction"
        ])
        .is_err());
    }
}

#[cfg(test)]
mod advisory_cli_tests {
    use super::*;
    #[test]
    fn advice_thresholds_formats_and_assert_defaults_parse() {
        let cli = Cli::try_parse_from([
            "dendro",
            "report",
            "spotlights",
            "--format",
            "markdown",
            "--spotlight-ceremony-ratio",
            "0.2",
            "--spotlight-builder-seats",
            "junior,senior",
        ])
        .unwrap();
        if let Commands::Report {
            advisory_thresholds,
            format,
            ..
        } = cli.command
        {
            assert_eq!(format.as_deref(), Some("markdown"));
            assert_eq!(advisory_thresholds.options().ceremony_ratio, 0.2);
            assert_eq!(
                advisory_thresholds.options().builder_seats,
                vec!["junior", "senior"]
            );
        } else {
            panic!("expected report")
        };
        let cli = Cli::try_parse_from(["dendro", "assert"]).unwrap();
        if let Commands::Assert {
            max_rework_rate,
            max_idle_rate,
            ..
        } = cli.command
        {
            assert_eq!(max_rework_rate, 0.15);
            assert_eq!(max_idle_rate, 0.2);
        } else {
            panic!("expected assert")
        }
    }
    #[test]
    fn publication_is_explicit_and_uses_argument_vector() {
        assert!(Cli::try_parse_from(["dendro", "issue-draft", "--publish"]).is_err());
        assert!(Cli::try_parse_from(["dendro", "issue-draft", "--repo", "owner/repo"]).is_err());
        let args = issue_publish_args("owner/repo", "literal `command` $(never run)\nsecond line")
            .unwrap();
        assert_eq!(args[0..4], ["issue", "create", "--repo", "owner/repo"]);
        assert!(args.last().unwrap().contains("$(never run)"));
        assert!(issue_publish_args("--evil", "body").is_err());
        assert!(issue_publish_args("owner/repo;command", "body").is_err());
    }
    #[test]
    fn malformed_advice_values_fail_without_db_access() {
        for (flag, value) in [
            ("--spotlight-ceremony-ratio", "NaN"),
            ("--spotlight-recovery-ratio", "2"),
            ("--spotlight-parallelism", "-1"),
            ("--advisory-leaf-stall-secs", "1.5"),
            ("--advisory-detached-target-secs", "-1"),
        ] {
            assert!(Cli::try_parse_from(["dendro", "report", "advisory", flag, value]).is_err());
        }
    }
}

#[cfg(test)]
mod insights_cli_tests {
    use super::*;

    #[test]
    fn insights_json_shape_lists_each_registered_result() {
        let cli = Cli::try_parse_from([
            "dendro",
            "insights",
            "--json",
            "--harness",
            "claude",
            "--session",
            "s",
        ])
        .unwrap();
        match cli.command {
            Commands::Insights {
                json,
                harness,
                session,
                ..
            } => {
                assert!(json);
                assert_eq!(harness.as_deref(), Some("claude"));
                assert_eq!(session.as_deref(), Some("s"));
            }
            _ => panic!("expected insights"),
        }
        let store = dendro_core::AnalyticsStore::open_in_memory("insights-cli").unwrap();
        let results =
            dendro_core::insight_v1::evaluate(&store, &dendro_core::insight_v1::Scope::default());
        let json = render_insights(&results, true).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let rows = value.as_array().unwrap();
        assert_eq!(rows.len(), dendro_core::insight_v1::REFERENCE.len());
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row["id"].as_str().unwrap().to_string());
            assert!(row["version"].as_str().unwrap().contains('.'));
            assert!(row["name"].is_string());
            assert!(row["domain"].is_string());
            assert_eq!(row["status"]["state"], "refused");
            assert_eq!(row["status"]["reason"], "empty population");
            assert!(row["population"]["what"].is_string());
            assert_eq!(row["population"]["n"], 0);
            assert!(row["measures"].as_array().unwrap().is_empty());
            assert!(row["coverage"].is_string());
            assert!(row["evidence"].as_array().unwrap().is_empty());
        }
        assert_eq!(ids, vec!["I02", "I04", "I08", "I09", "I14", "I18"]);
        let text = render_insights(&results, false).unwrap();
        assert!(text.contains("I02") && text.contains("refused"));
        assert!(!text.trim_start().starts_with('['));
    }
}
