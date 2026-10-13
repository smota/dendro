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
            let openrig_roots: Vec<PathBuf> = roots
                .iter()
                .filter(|root| root.name == "openrig")
                .map(|root| root.path.clone())
                .collect();
            let db_for_align = db.clone();
            // Listeners exist before the worker, so no signal can kill it mid-copy.
            let mut interrupt = signal(SignalKind::interrupt())?;
            let mut terminate = signal(SignalKind::terminate())?;
            let mut worker = tokio::task::spawn_blocking(move || {
                dendro_core::run_locked_with_openrig(&db, &paths, &openrig_roots, |staged, key| {
                    let mut roots = roots;
                    for root in &mut roots {
                        if root.name == "openrig" {
                            root.path =
                                dendro_core::align_openrig_spelling(&db_for_align, &root.path)?;
                        }
                    }
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
            from,
            to,
        } => {
            let report = dendro_core::report_json(&db, &kind, rig.as_deref(), &from, &to)?;
            println!("{report}");
        }
        Commands::Trace {
            db,
            metric_id,
            rig,
            from,
            to,
        } => {
            let report = dendro_core::trace_json(&db, &metric_id, rig.as_deref(), &from, &to)?;
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
    let openrig: Vec<PathBuf> = roots
        .iter()
        .filter(|root| root.name == "openrig")
        .map(|root| root.path.clone())
        .collect();
    // Before the database is opened or any source is read.
    dendro_core::refuse_multiple_openrig_roots(&openrig)?;
    let mut seen = Vec::new();
    roots.retain(|root| {
        if root.name != "openrig" {
            return true;
        }
        let canonical = dendro_core::canonical_root(&root.path);
        if seen.contains(&canonical) {
            return false;
        }
        seen.push(canonical);
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
        assert_eq!(openrig[0].path, file);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
