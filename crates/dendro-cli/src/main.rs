use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
            let summary = tokio::task::spawn_blocking(move || {
                dendro_core::run_locked(&db, &paths, |staged, key| {
                    dendro_connectors::ingest_staged(staged, &roots, frozen, key)
                })
            })
            .await??;
            println!("{summary}");
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
