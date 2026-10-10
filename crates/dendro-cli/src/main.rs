use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::info;

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
    /// Run daemon in continuous file watching mode
    Watch {
        #[arg(short, long, default_value = "dendro.duckdb")]
        db: PathBuf,
        #[arg(short, long, default_value = "2")]
        debounce_secs: u64,
    },
    /// Initialize Nao semantic layer files for natural language querying
    InitNao {
        #[arg(short, long, default_value = ".")]
        target: PathBuf,
        #[arg(short, long, default_value = "dendro.duckdb")]
        db: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Run { db, root, frozen } => {
            let roots = if root.is_empty() {
                default_roots()
            } else {
                parse_roots(&root)?
            };
            let paths: Vec<PathBuf> = roots.iter().map(|root| root.path.clone()).collect();
            let summary = tokio::task::spawn_blocking(move || {
                dendro_core::run_locked(&db, &paths, |staged| {
                    dendro_connectors::ingest_staged(staged, &roots, frozen)
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
        Commands::Watch { db, debounce_secs } => {
            info!(
                "Starting Dendro watcher against {:?} (debounce: {}s)",
                db, debounce_secs
            );
            info!("Dendro watch is outside this R1 proof. Press Ctrl+C to stop.");
        }
        Commands::InitNao { target, db } => {
            info!(
                "Initializing Nao semantic models in {:?} pointing to {:?}",
                target, db
            );
            let config_content = format!(
                r#"# Nao configuration for Dendro
version: 1
data_source:
  type: duckdb
  path: "{}"

metrics:
  - name: total_cost_usd
    sql: "SUM(estimated_cost_usd)"
    description: "Total cost across agent sessions in USD"
  - name: active_sessions
    sql: "COUNT(*)"
    description: "Total number of completed or active agent sessions"
  - name: avg_tool_calls
    sql: "AVG(tool_calls_count)"
    description: "Average number of tool executions per session"
"#,
                db.display()
            );
            let target_config = target.join("nao_config.yaml");
            std::fs::write(&target_config, config_content)?;
            info!("Generated {:?}", target_config);
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
