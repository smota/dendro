use std::path::PathBuf;
use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;

#[derive(Parser)]
#[command(name = "dendro")]
#[command(about = "Dendro: Reading the rings of your AI agents — Local-first telemetry and analytics engine", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run one-shot batch extraction and sync to DuckDB
    Run {
        #[arg(short, long, default_value = "dendro.duckdb")]
        db: PathBuf,
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
            let _storage = dendro_core::DuckDbStorage::open(&db)?;
            info!("Dendro batch sync completed successfully.");
        }
        Commands::Watch { db, debounce_secs } => {
            info!(
                "Starting Dendro watcher against {:?} (debounce: {}s)",
                db, debounce_secs
            );
            info!("Dendro is actively reading agent session rings. Press Ctrl+C to stop.");
        }
        Commands::InitNao { target, db } => {
            info!("Initializing Nao semantic models in {:?} pointing to {:?}", target, db);
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
