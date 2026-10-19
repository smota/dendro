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
    /// Run deterministic historical what-if simulations over canonical reports.
    Simulate {
        #[command(subcommand)]
        command: SimulateCommand,
    },
}

#[derive(Subcommand)]
pub enum SimulateCommand {
    /// Model historical seat removal, direct handoffs, tier multipliers, and boundary resets.
    Topology {
        #[arg(long, default_value = "dendro.duckdb")]
        db: PathBuf,
        #[arg(long)]
        rig: Option<String>,
        #[arg(long, requires = "to")]
        from: Option<String>,
        #[arg(long, requires = "from")]
        to: Option<String>,
        #[arg(long)]
        scenario: Option<PathBuf>,
        #[arg(long, value_delimiter = ',')]
        remove: Vec<String>,
        #[arg(long)]
        replacement_seat: Option<String>,
        #[arg(long)]
        direct_handoffs: bool,
        #[arg(long)]
        tier_policy: Option<PathBuf>,
        #[arg(long)]
        boundary_reset_threshold: Option<u64>,
        #[arg(long)]
        enforce_model_family_diversity: bool,
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

#[allow(clippy::too_many_arguments)]
fn build_simulation_scenario(
    scenario_path: Option<&Path>,
    remove: Vec<String>,
    replacement_seat: Option<String>,
    direct_handoffs: bool,
    tier_policy_path: Option<&Path>,
    boundary_reset_threshold: Option<u64>,
    enforce_model_family_diversity: bool,
) -> Result<dendro_core::simulation::Scenario> {
    let mut scenario: dendro_core::simulation::Scenario = match scenario_path {
        Some(path) => serde_json::from_value(json_file(path)?)
            .map_err(|_| anyhow::anyhow!("invalid simulation scenario"))?,
        None => Default::default(),
    };
    if !remove.is_empty() {
        scenario.remove = remove;
    }
    if let Some(seat) = replacement_seat {
        scenario.replacement_seat = Some(seat);
    }
    if direct_handoffs {
        scenario.direct_handoffs = true;
    }
    if let Some(path) = tier_policy_path {
        scenario.tier_policy = serde_json::from_value(json_file(path)?)
            .map_err(|_| anyhow::anyhow!("invalid tier policy"))?;
    }
    if let Some(threshold) = boundary_reset_threshold {
        scenario.boundary_reset_threshold = Some(threshold);
    }
    if enforce_model_family_diversity {
        scenario.enforce_model_family_diversity = true;
    }
    Ok(scenario)
}

fn write_new_private_output_with(
    output: &Path,
    bytes: &[u8],
    write_fn: impl FnOnce(&mut fs::File, &[u8]) -> std::io::Result<()>,
) -> Result<()> {
    let mut file = dendro_core::lifecycle::create_private_file(output)
        .context("export requires a new output file")?;
    if let Err(err) = write_fn(&mut file, bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(output);
        return Err(err.into());
    }
    Ok(())
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
                match fs::symlink_metadata(&p) {
                    Ok(meta) => {
                        if meta.file_type().is_symlink() {
                            bail!("hold directory escapes its authorized root");
                        }
                        return resolve(path);
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => bail!("hold directory unavailable"),
                }
                let parent = p
                    .parent()
                    .context("hold directory needs a parent")?
                    .canonicalize()
                    .map_err(|_| anyhow::anyhow!("hold directory escapes its authorized root"))?;
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
                write_new_private_output_with(&output, html.as_bytes(), |file, data| {
                    file.write_all(data)
                })?;
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
            Commands::Simulate {
                command:
                    SimulateCommand::Topology {
                        db,
                        rig,
                        from,
                        to,
                        scenario,
                        remove,
                        replacement_seat,
                        direct_handoffs,
                        tier_policy,
                        boundary_reset_threshold,
                        enforce_model_family_diversity,
                    },
            } => {
                let (from, to) = match (from, to) {
                    (Some(f), Some(t)) => (f, t),
                    _ => dendro_core::presentation::trailing_window(1)?,
                };
                let scenario = build_simulation_scenario(
                    scenario.as_deref(),
                    remove,
                    replacement_seat,
                    direct_handoffs,
                    tier_policy.as_deref(),
                    boundary_reset_threshold,
                    enforce_model_family_diversity,
                )?;
                let report =
                    dendro_core::report_json(&db, "decisions", rig.as_deref(), &from, &to)?;
                println!("{}", dendro_core::simulation::simulate(&report, &scenario)?);
            }
            Commands::Ui { .. } => unreachable!(),
        }
        Ok(())
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(prefix: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "dendro-ext-{prefix}-{}-{nanos}",
                std::process::id(),
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn failed_export_write_removes_partial_private_file_and_preserves_existing() {
        let dir = TempDir::new("export");
        let output = dir.0.join("report.html");
        let err = write_new_private_output_with(&output, b"complete", |file, _| {
            file.write_all(b"partial")?;
            Err(std::io::Error::other("injected write failure"))
        });
        assert!(err.is_err());
        assert!(!output.exists(), "partial private file must be removed");

        write_new_private_output_with(&output, b"complete", |file, data| file.write_all(data))
            .unwrap();
        assert_eq!(fs::read(&output).unwrap(), b"complete");

        let duplicate =
            write_new_private_output_with(&output, b"overwrite", |file, data| file.write_all(data));
        assert!(duplicate.is_err());
        assert_eq!(
            fs::read(&output).unwrap(),
            b"complete",
            "existing file must not be removed or overwritten"
        );
    }

    #[test]
    fn dangling_symlink_hold_dir_is_refused_with_explicit_containment_error() {
        let dir = TempDir::new("hold-symlink");
        let root = dir.0.join("source");
        fs::create_dir_all(&root).unwrap();
        let db = root.join("analytics.duckdb");
        fs::write(&db, b"placeholder").unwrap();
        std::os::unix::fs::symlink(
            dir.0.join("outside-missing-target"),
            root.join("dangling-holds"),
        )
        .unwrap();
        fs::write(
            root.join("dendro-source.json"),
            serde_json::to_vec(&json!({
                "db": "analytics.duckdb",
                "rigs": ["rig-a"],
                "hold_dir": "dangling-holds"
            }))
            .unwrap(),
        )
        .unwrap();
        let err = sources(None, std::slice::from_ref(&root)).unwrap_err();
        assert!(
            err.to_string()
                .contains("hold directory escapes its authorized root"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn simulation_scenario_overlays_supplied_flags_and_refuses_unknown_fields() {
        let dir = TempDir::new("sim");
        let scenario_file = dir.0.join("scenario.json");
        fs::write(
            &scenario_file,
            serde_json::to_vec(&json!({
                "remove": ["dev-build-jr@team"],
                "replacement_seat": "dev-build@team",
                "direct_handoffs": false,
                "boundary_reset_threshold": 300000,
                "weights": {"uncached": 1.0, "cache_read": 0.1},
                "boundary": {
                    "turns": 15,
                    "stale_context_tokens": 350000,
                    "clean_context_tokens": 60000,
                    "cold_boot_tokens": 35000
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let tier_file = dir.0.join("tiers.json");
        fs::write(
            &tier_file,
            serde_json::to_vec(&json!([{
                "seat": "dev-build@team",
                "uncached_multiplier": 0.8,
                "cache_read_multiplier": 1.0,
                "work_class": "P1",
                "model_family": "claude",
                "effort": "medium"
            }]))
            .unwrap(),
        )
        .unwrap();

        let merged = build_simulation_scenario(
            Some(&scenario_file),
            vec![],
            None,
            true,
            Some(&tier_file),
            Some(250000),
            true,
        )
        .unwrap();
        assert_eq!(merged.remove, vec!["dev-build-jr@team"]);
        assert_eq!(merged.replacement_seat.as_deref(), Some("dev-build@team"));
        assert!(merged.direct_handoffs);
        assert_eq!(merged.boundary_reset_threshold, Some(250000));
        assert!(merged.enforce_model_family_diversity);
        assert_eq!(merged.tier_policy.len(), 1);
        assert_eq!(merged.tier_policy[0].seat, "dev-build@team");

        let unknown_file = dir.0.join("unknown.json");
        fs::write(&unknown_file, br#"{"unknown_field": true}"#).unwrap();
        assert!(build_simulation_scenario(
            Some(&unknown_file),
            vec![],
            None,
            false,
            None,
            None,
            false
        )
        .is_err());
    }
}
