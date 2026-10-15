#![deny(unsafe_code)]

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
        #[arg(long, default_value_t = 3)]
        tool_loop_failures: usize,
        #[arg(long, default_value_t = 600, allow_hyphen_values = true)]
        tool_loop_window_secs: i64,
        #[arg(long, default_value_t = 2)]
        handoff_loop_returns: usize,
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
        #[arg(long, default_value_t = 3)]
        tool_loop_failures: usize,
        #[arg(long, default_value_t = 600, allow_hyphen_values = true)]
        tool_loop_window_secs: i64,
        #[arg(long, default_value_t = 2)]
        handoff_loop_returns: usize,
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
            let mut roots = if root.is_empty() {
                default_roots()
            } else {
                parse_roots(&root)?
            };
            // Bind and store an absolute OpenRig path. A relative path would
            // mean a different file after the working directory changes.
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
            tool_loop_failures,
            tool_loop_window_secs,
            handoff_loop_returns,
        } => {
            let thresholds = dendro_core::LoopThresholds {
                tool_loop_failures,
                tool_loop_window_secs,
                handoff_loop_returns,
            };
            let report = dendro_core::report_json_with_thresholds(
                &db,
                &kind,
                rig.as_deref(),
                &from,
                &to,
                thresholds,
            )?;
            println!("{report}");
        }
        Commands::Trace {
            db,
            metric_id,
            rig,
            from,
            to,
            tool_loop_failures,
            tool_loop_window_secs,
            handoff_loop_returns,
        } => {
            let thresholds = dendro_core::LoopThresholds {
                tool_loop_failures,
                tool_loop_window_secs,
                handoff_loop_returns,
            };
            let report = dendro_core::trace_json_with_thresholds(
                &db,
                &metric_id,
                rig.as_deref(),
                &from,
                &to,
                thresholds,
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
        if root.name != "openrig" {
            return true;
        }
        if seen.contains(&root.path) {
            return false;
        }
        seen.push(root.path.clone());
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
