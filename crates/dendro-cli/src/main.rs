use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;
use tokio::signal::unix::{signal, Signal, SignalKind};
use tokio::task::JoinHandle;

/// How long a signalled run may take to stop its worker and remove its stage.
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(30);

#[derive(Parser)]
#[command(name = "dendro")]
#[command(about = "Dendro AI Fleet Intelligence. Operational Intelligence for seat, handoff, and token evidence. Fleet Governance stays with the operator.", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run one-shot ingest into a local analytics database.
    Run {
        #[arg(short, long, default_value = "dendro.duckdb")]
        db: PathBuf,
        #[arg(long)]
        root: Vec<String>,
        #[arg(long)]
        frozen: bool,
    },
    /// Print an Operational Intelligence report as JSON.
    Report {
        kind: String,
        #[arg(long, default_value = "dendro.duckdb")]
        db: PathBuf,
        #[arg(long)]
        rig: Option<String>,
        /// Optional G2 friction configuration (versioned JSON).
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long, default_value = "2026-10-08T00:00:00Z")]
        from: String,
        #[arg(long, default_value = "2026-10-11T00:00:00Z")]
        to: String,
    },
    /// Print the trace inputs for one metric id.
    Trace {
        #[arg(long, default_value = "dendro.duckdb")]
        db: PathBuf,
        #[arg(long)]
        metric_id: String,
        #[arg(long)]
        rig: Option<String>,
        /// Optional G2 friction configuration (versioned JSON).
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long, default_value = "2026-10-08T00:00:00Z")]
        from: String,
        #[arg(long, default_value = "2026-10-11T00:00:00Z")]
        to: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = match Cli::try_parse() {
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
        Commands::Run { db, root, frozen } => {
            let roots = if root.is_empty() {
                default_roots()
            } else {
                parse_roots(&root)?
            };
            let paths: Vec<PathBuf> = roots.iter().map(|root| root.path.clone()).collect();
            // Listeners exist before the worker, so no signal can kill it mid-copy.
            let mut interrupt = signal(SignalKind::interrupt())?;
            let mut terminate = signal(SignalKind::terminate())?;
            let mut worker = tokio::task::spawn_blocking(move || {
                dendro_core::run_locked(&db, &paths, |staged, key| {
                    dendro_connectors::ingest_staged(staged, &roots, frozen, key)
                })
            });
            let code = tokio::select! {
                joined = &mut worker => {
                    println!("{}", joined??);
                    return Ok(());
                }
                _ = interrupt.recv() => 130,
                _ = terminate.recv() => 143,
            };
            std::process::exit(stop_cooperatively(worker, interrupt, terminate, code).await);
        }
        Commands::Report {
            kind,
            db,
            rig,
            config,
            from,
            to,
        } => {
            let report = dendro_core::report_json_configured(
                &db,
                &kind,
                rig.as_deref(),
                &from,
                &to,
                config.as_deref(),
            )?;
            println!("{report}");
        }
        Commands::Trace {
            db,
            metric_id,
            rig,
            config,
            from,
            to,
        } => {
            let report = dendro_core::trace_json_configured(
                &db,
                &metric_id,
                rig.as_deref(),
                &from,
                &to,
                config.as_deref(),
            )?;
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

fn default_roots() -> Vec<dendro_connectors::SourceRoot> {
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
    Ok(roots)
}
