#![deny(unsafe_code)]

mod extensions;
pub mod tui;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::Duration;
use tokio::signal::unix::{signal, Signal, SignalKind};
use tokio::task::JoinHandle;

/// How long a signalled run may take to stop its worker and remove its stage.
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(30);

const AFTER_HELP: &str = "\
Quickstart Examples:
  dendro
      Status report. Same as `dendro status`.
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
  Reports      report, trace
  Analysis     simulate, portfolio
  Governance   assert, issue-draft
  Cache        materialize, export, init-nao
";

#[derive(Parser)]
#[command(name = "dendro")]
#[command(about = "Dendro AI Fleet Intelligence. Operational Intelligence for seat, handoff, and token evidence. Fleet Governance stays with the operator.", long_about = None)]
#[command(version, after_help = AFTER_HELP)]
struct Cli {
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
    #[arg(long, default_value = "dendro.duckdb")]
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
        let (from, to) = if scope.window.is_some() {
            let (f, t, _) = dendro_core::presentation::resolve_window(
                &scope.db,
                scope.rig.as_deref(),
                scope.window.as_deref(),
                None,
                None,
            )?;
            (f, t)
        } else {
            (scope.from, scope.to)
        };
        dendro_core::report_json(&scope.db, "decisions", scope.rig.as_deref(), &from, &to)
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
    /// Inspect coverage, discovered harness roots, fleet topology, and next commands.
    #[command(next_help_heading = "Database & Scope")]
    Status {
        #[arg(long, default_value = "dendro.duckdb")]
        db: PathBuf,
        #[arg(long)]
        rig: Option<String>,
        /// Text report or the raw status document.
        #[arg(long, value_parser = ["text", "json"])]
        format: Option<String>,
    },
    /// Ingest local harness roots into an analytics database.
    #[command(next_help_heading = "Ingest")]
    Run {
        #[arg(short, long, default_value = "dendro.duckdb")]
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
        #[arg(long, default_value = "dendro.duckdb")]
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
        #[arg(long, default_value = "dendro.duckdb")]
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
        default_roots()
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
    let db_for_align = db.clone();
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
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
                        root.path = dendro_core::align_openrig_spelling(&db_for_align, &root.path)?;
                    }
                }
                dendro_connectors::ingest_staged_with_manifest(
                    staged, &roots, frozen, key, manifest,
                )
            },
        )
    });
    let code = tokio::select! {
        joined = &mut worker => {
            return joined?;
        }
        _ = interrupt.recv() => 130,
        _ = terminate.recv() => 143,
    };
    std::process::exit(stop_cooperatively(worker, interrupt, terminate, code).await);
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let parse_result = if std::env::args_os().len() == 1 {
        Cli::try_parse_from(["dendro", "status"])
    } else {
        Cli::try_parse()
    };
    let cli = match parse_result {
        Ok(cli) => cli,
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

    match cli.command {
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
                        !db.exists()
                            || dendro_core::presentation::database_status(&db, rig.as_deref())
                                .map(|s| s["status"] != "ready")
                                .unwrap_or(true)
                    }
                })
                .await?;
                if needs_ingest {
                    let _ = run_ingest(db, Vec::new(), false, None, None).await?;
                }
            }
            extensions::run(command).await?;
        }
        Commands::Status { db, rig, format } => {
            let roots = default_roots();
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
                let (from, to, mode) = if window.is_some() {
                    let (f, t, m) = dendro_core::presentation::resolve_window(
                        &db,
                        rig.as_deref(),
                        window.as_deref(),
                        None,
                        None,
                    )?;
                    (f, t, Some(m))
                } else {
                    (from, to, None)
                };
                let mut r = dendro_core::report_json_with_analysis(
                    &db,
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
                let (from, to, mode) = if window.is_some() {
                    let (f, t, m) = dendro_core::presentation::resolve_window(
                        &db,
                        rig.as_deref(),
                        window.as_deref(),
                        None,
                        None,
                    )?;
                    (f, t, Some(m))
                } else {
                    (from, to, None)
                };
                let mut r = dendro_core::trace_json_with_analysis(
                    &db,
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
    }

    Ok(())
}

/// Asks the worker to stop, waits for it (its SQLite and DuckDB handles close
/// and its stage is removed inside the worker), and returns the exit code. A
/// second signal or the time limit stops waiting without deleting anything the
/// worker may still write; that is reported as a failed cleanup, and the next
/// run's recovery removes the stage.
async fn stop_cooperatively(
    mut worker: JoinHandle<Result<serde_json::Value>>,
    mut interrupt: Signal,
    mut terminate: Signal,
    code: i32,
) -> i32 {
    dendro_core::RUN_CANCEL.request();
    eprintln!("dendro: stopping; removing private staging");
    let waited = tokio::time::timeout(SHUTDOWN_LIMIT, async {
        tokio::select! {
            joined = &mut worker => Some(joined),
            _ = interrupt.recv() => None,
            _ = terminate.recv() => None,
        }
    })
    .await;
    match waited {
        Ok(Some(Ok(Ok(summary)))) => {
            // The run finished before the request took effect.
            println!("{summary}");
            eprintln!("dendro: the run had already completed");
            0
        }
        Ok(Some(Ok(Err(error)))) if error.downcast_ref::<dendro_core::Interrupted>().is_some() => {
            eprintln!("dendro: interrupted; private staging removed");
            code
        }
        Ok(Some(Ok(Err(error)))) => {
            eprintln!(
                "dendro: interrupted with error: {}",
                dendro_core::redact(&format!("{error:#}"))
            );
            code
        }
        Ok(Some(Err(_))) => {
            eprintln!("dendro: interrupted; the worker failed; staging cleanup not confirmed");
            code
        }
        Ok(None) | Err(_) => {
            eprintln!("dendro: interrupted before shutdown finished; staging cleanup FAILED and is left for the next run's recovery");
            code
        }
    }
}

pub(crate) fn default_roots() -> Vec<dendro_connectors::SourceRoot> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let openrig = std::env::var("OPENRIG_HOME").unwrap_or_else(|_| format!("{home}/.openrig"));
    vec![
        dendro_connectors::SourceRoot {
            name: "claude".into(),
            path: PathBuf::from(&home).join(".claude/projects"),
        },
        dendro_connectors::SourceRoot {
            name: "codex".into(),
            path: PathBuf::from(&home).join(".codex/sessions"),
        },
        dendro_connectors::SourceRoot {
            name: "pi".into(),
            path: PathBuf::from(&openrig).join("state/pi"),
        },
        dendro_connectors::SourceRoot {
            name: "pi".into(),
            path: PathBuf::from(&home).join(".pi/agent/sessions"),
        },
        dendro_connectors::SourceRoot {
            name: "openrig".into(),
            path: PathBuf::from(&openrig).join("openrig.sqlite"),
        },
        dendro_connectors::SourceRoot {
            name: "agy".into(),
            path: PathBuf::from(&home).join(".gemini/antigravity-cli/brain"),
        },
        dendro_connectors::SourceRoot {
            name: "grok".into(),
            path: PathBuf::from(&home).join(".grok"),
        },
    ]
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
        let defaults = default_roots();
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
