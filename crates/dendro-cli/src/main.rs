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
        Commands::Run { db } => {
            info!("Starting Dendro batch extraction against {:?}", db);
            let summary = dendro_core::run_locked(&db, &[])?;
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
        Commands::Trace { db, metric_id } => {
            let report = dendro_core::report_json(&db, &metric_id, None, "", "")?;
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
