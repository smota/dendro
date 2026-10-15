//! Explicit, local entry points around canonical analytics.
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Args)]
pub struct Scope {
    #[arg(long, default_value = "dendro.duckdb")]
    pub db: PathBuf,
    #[arg(long)]
    pub rig: Option<String>,
    #[arg(long)]
    pub from: String,
    #[arg(long)]
    pub to: String,
}
#[derive(Subcommand)]
pub enum Commands {
    /// Refresh the canonical, explicitly scoped Nao projection cache.
    Materialize {
        #[command(flatten)]
        scope: Scope,
    },
    /// Answer a supported local intent against an explicitly materialized scope.
    Ask {
        #[arg(long, default_value = "dendro.duckdb")]
        db: PathBuf,
        #[arg(long)]
        question: String,
        #[arg(long)]
        baseline: Option<PathBuf>,
    },
    /// Create a new Nao workspace; existing directories are never overwritten.
    InitNao {
        #[arg(long, default_value = "dendro.duckdb")]
        db: PathBuf,
        #[arg(long, visible_alias = "target")]
        output: PathBuf,
    },
    /// Export a scrubbed, self-contained offline visualization.
    Export {
        #[command(flatten)]
        scope: Scope,
        #[arg(long,default_value="html",value_parser=["html"])]
        format: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Explore a cached 24h/7d report with keyboard navigation.
    Ui {
        #[arg(long, default_value = "dendro.duckdb")]
        db: PathBuf,
        #[arg(long)]
        rig: Option<String>,
        #[arg(long,value_parser=["24h","7d"],conflicts_with_all=["from","to"],help="Trailing window (default: 24h); conflicts with fixed --from/--to")]
        window: Option<String>,
        #[arg(long, requires = "to")]
        from: Option<String>,
        #[arg(long, requires = "from")]
        to: Option<String>,
        #[arg(long)]
        once: bool,
    },
    /// Aggregate explicitly mapped, independently ingested analytics databases.
    Portfolio {
        #[arg(long, required_unless_present = "source_dir")]
        sources: Option<PathBuf>,
        #[arg(long, required_unless_present = "sources")]
        source_dir: Vec<PathBuf>,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long,default_value="json",value_parser=["json","ndjson","table"])]
        format: String,
        /// Threshold for recorded failed-wake pending age; never a live failure duration.
        #[arg(long, default_value_t = 300)]
        pending_threshold_secs: u64,
    },
}
fn json_file(path: &Path) -> Result<Value> {
    dendro_core::refuse_secret_path(path)?;
    if !fs::symlink_metadata(path)
        .map(|m| m.is_file())
        .unwrap_or(false)
    {
        bail!("JSON input must be an explicit regular file");
    }
    let file = fs::File::open(path).context("JSON input unavailable")?;
    let mut bytes = Vec::new();
    file.take(10 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 10 * 1024 * 1024 {
        bail!("JSON input exceeds 10 MiB");
    }
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid JSON input"))
}
fn sources(
    mapping: Option<&Path>,
    directories: &[PathBuf],
) -> Result<Vec<dendro_core::portfolio::PortfolioSource>> {
    let mut sources = Vec::new();
    if let Some(path) = mapping {
        sources = serde_json::from_value(json_file(path)?)
            .map_err(|_| anyhow::anyhow!("invalid portfolio source mapping"))?;
    }
    for directory in directories {
        if fs::symlink_metadata(directory)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(true)
        {
            bail!("source directory must be an explicit non-symlink directory");
        }
        let root = directory
            .canonicalize()
            .context("source directory unavailable")?;
        let path = root.join("dendro-source.json");
        let mut source: dendro_core::portfolio::PortfolioSource =
            serde_json::from_value(json_file(&path)?)
                .map_err(|_| anyhow::anyhow!("invalid source directory descriptor"))?;
        let resolve = |path: &Path| -> Result<PathBuf> {
            let p = if path.is_absolute() {
                path.to_owned()
            } else {
                root.join(path)
            };
            let p = p.canonicalize().context("descriptor source unavailable")?;
            if !p.starts_with(&root) {
                bail!("source directory descriptor escapes its authorized root");
            }
            Ok(p)
        };
        source.db = resolve(&source.db)?;
        source.hold_dir = source
            .hold_dir
            .as_deref()
            .map(|path| -> Result<PathBuf> {
                let p = if path.is_absolute() {
                    path.to_owned()
                } else {
                    root.join(path)
                };
                if p.exists() {
                    return resolve(path);
                }
                let parent = p
                    .parent()
                    .context("hold directory needs a parent")?
                    .canonicalize()
                    .context("hold parent unavailable")?;
                if !parent.starts_with(&root) {
                    bail!("hold directory escapes its authorized root");
                }
                Ok(parent.join(p.file_name().context("hold directory name missing")?))
            })
            .transpose()?;
        sources.push(source);
    }
    Ok(sources)
}
pub async fn run(command: Commands) -> Result<()> {
    if let Commands::Ui {
        db,
        rig,
        window,
        from,
        to,
        once,
    } = command
    {
        // Signals request cooperative exit; the blocking input loop has a 100ms read timeout.
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut worker = tokio::task::spawn_blocking(move || -> Result<()> {
            let (from, to) = match (from, to) {
                (Some(f), Some(t)) => (f, t),
                _ => dendro_core::presentation::trailing_window(
                    if window.as_deref() == Some("7d") {
                        7
                    } else {
                        1
                    },
                )?,
            };
            let report = dendro_core::report_json(&db, "decisions", rig.as_deref(), &from, &to)?;
            let mut traces = Vec::new();
            if !once {
                for id in ["handoff_friction", "activity", "tokens"] {
                    traces.push(dendro_core::trace_json(
                        &db,
                        id,
                        rig.as_deref(),
                        &from,
                        &to,
                    )?);
                }
            }
            dendro_core::presentation::terminal_ui(&report, &traces, once)
        });
        tokio::select! {r=&mut worker=>r??,_=interrupt.recv()=>{dendro_core::RUN_CANCEL.request();worker.await??;},_=terminate.recv()=>{dendro_core::RUN_CANCEL.request();worker.await??;}}
        return Ok(());
    }
    tokio::task::spawn_blocking(move || -> Result<()> {
        match command {
            Commands::Materialize { scope } => println!(
                "{}",
                dendro_core::semantic::refresh(
                    &scope.db,
                    scope.rig.as_deref(),
                    &scope.from,
                    &scope.to
                )?
            ),
            Commands::Ask {
                db,
                question,
                baseline,
            } => {
                let baseline = baseline.as_deref().map(json_file).transpose()?;
                println!(
                    "{}",
                    dendro_core::semantic::answer(&db, &question, baseline.as_ref())?
                );
            }
            Commands::InitNao { db, output } => {
                dendro_core::semantic::init_workspace(&output, &db)?;
                println!("{{\"status\":\"created\",\"local_only\":true}}");
            }
            Commands::Export {
                scope,
                format: _,
                output,
            } => {
                dendro_core::refuse_secret_path(&output)?;
                let report = dendro_core::report_json(
                    &scope.db,
                    "decisions",
                    scope.rig.as_deref(),
                    &scope.from,
                    &scope.to,
                )?;
                let html = dendro_core::presentation::offline_html(&report)?;
                let mut file = dendro_core::lifecycle::create_private_file(&output)
                    .context("export requires a new output file")?;
                file.write_all(html.as_bytes())?;
                file.sync_all()?;
                println!(
                    "{{\"status\":\"exported\",\"format\":\"html\",\"seat_labels\":\"aliases\"}}"
                );
            }
            Commands::Portfolio {
                sources: mapping,
                source_dir,
                from,
                to,
                format,
                pending_threshold_secs,
            } => {
                let sources = sources(mapping.as_deref(), &source_dir)?;
                let mut report = dendro_core::portfolio::portfolio_json(&sources, &from, &to)?;
                // Portfolio's producer is independent of single-root report evaluation.
                report["payload"]["anomaly_spotlights"] = dendro_core::spotlight::evaluate(
                    &report["payload"],
                    &dendro_core::advisory::AdvisoryOptions {
                        detached_target_secs: pending_threshold_secs,
                        ..Default::default()
                    },
                );
                match format.as_str() {
                    "table" => print!("{}", dendro_core::portfolio::portfolio_table(&report)?),
                    "ndjson" => print!("{}", dendro_core::portfolio::portfolio_ndjson(&report)?),
                    _ => println!("{report}"),
                }
            }
            Commands::Ui { .. } => unreachable!(),
        }
        Ok(())
    })
    .await?
}
