//! Explicit, local entry points around canonical analytics.
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use serde_json::Value;
const DEFAULT_DB_HELP: &str = super::DEFAULT_DB_HELP;

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Args, Clone, Debug)]
#[command(next_help_heading = "Scope")]
pub struct Scope {
    #[arg(long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP)]
    pub db: PathBuf,
    #[arg(long)]
    pub rig: Option<String>,
    #[arg(
        long,
        value_parser = ["1h", "24h", "7d", "30d", "all"],
        conflicts_with_all = ["from", "to"],
        help = "Relative window anchored to recorded DB data (default: all)"
    )]
    pub window: Option<String>,
    #[arg(long, requires = "to")]
    pub from: Option<String>,
    #[arg(long, requires = "from")]
    pub to: Option<String>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Open the 6-screen cockpit, or export a self-contained HTML dashboard.
    #[command(next_help_heading = "Scope")]
    Dashboard {
        #[arg(long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP)]
        db: PathBuf,
        #[arg(long)]
        rig: Option<String>,
        #[arg(
            long,
            value_parser = ["1h", "24h", "7d", "30d", "all"],
            conflicts_with_all = ["from", "to"],
            help = "Window (default: all recorded data); conflicts with fixed --from/--to"
        )]
        window: Option<String>,
        #[arg(long, requires = "to")]
        from: Option<String>,
        #[arg(long, requires = "from")]
        to: Option<String>,
        /// Export self-contained offline HTML dashboard to path (default: dendro-dashboard.html).
        #[arg(long, num_args = 0..=1, default_missing_value = "dendro-dashboard.html", help_heading = "Cockpit")]
        html: Option<PathBuf>,
        #[arg(long, help_heading = "Cockpit")]
        once: bool,
        /// Ingest default local harness roots first when the database is missing or not ready.
        #[arg(long, help_heading = "Cockpit")]
        auto_ingest: bool,
    },
    /// Open the cockpit for an explicit window. Auto-anchors to recorded data.
    #[command(next_help_heading = "Scope")]
    Ui {
        #[arg(long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP)]
        db: PathBuf,
        #[arg(long)]
        rig: Option<String>,
        #[arg(
            long,
            value_parser = ["1h", "24h", "7d", "30d", "all"],
            conflicts_with_all = ["from", "to"],
            help = "Window (default: 7d anchored to recorded data); conflicts with fixed --from/--to"
        )]
        window: Option<String>,
        #[arg(long, requires = "to")]
        from: Option<String>,
        #[arg(long, requires = "from")]
        to: Option<String>,
        #[arg(long, help_heading = "Cockpit")]
        once: bool,
        /// Ingest default local harness roots first when the database is missing or not ready.
        #[arg(long, help_heading = "Cockpit")]
        auto_ingest: bool,
    },
    /// Answer a supported local question. Refreshes a stale snapshot unless --no-refresh is set.
    #[command(next_help_heading = "Question")]
    Ask {
        /// Question in plain English (may also be passed via --question).
        query: Option<String>,
        #[arg(long)]
        question: Option<String>,
        #[arg(long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP, help_heading = "Scope")]
        db: PathBuf,
        #[arg(long, help_heading = "Scope")]
        baseline: Option<PathBuf>,
        #[arg(long, help_heading = "Scope")]
        rig: Option<String>,
        #[arg(
            long,
            value_parser = ["1h", "24h", "7d", "30d", "all"],
            conflicts_with_all = ["from", "to"],
            help_heading = "Scope"
        )]
        window: Option<String>,
        #[arg(long, requires = "to", help_heading = "Scope")]
        from: Option<String>,
        #[arg(long, requires = "from", help_heading = "Scope")]
        to: Option<String>,
        /// text or json. Defaults to text on a terminal and json when stdout is not a terminal.
        #[arg(long, value_parser = ["text", "json"], help_heading = "Output")]
        format: Option<String>,
        /// List supported intents and example questions. Does not need a database.
        #[arg(long, help_heading = "Output")]
        list_intents: bool,
        /// Do not auto-materialize when the semantic snapshot is missing or stale.
        #[arg(long, help_heading = "Output")]
        no_refresh: bool,
    },
    /// Run a deterministic historical what-if over a canonical decisions report.
    #[command(next_help_heading = "Analysis")]
    Simulate {
        #[command(subcommand)]
        command: SimulateCommand,
    },
    /// Aggregate explicitly mapped, independently ingested analytics databases.
    #[command(next_help_heading = "Analysis")]
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
    /// Refresh the canonical, explicitly scoped Nao projection cache.
    #[command(next_help_heading = "Cache & Export")]
    Materialize {
        #[command(flatten)]
        scope: Scope,
    },
    /// Export a scrubbed, self-contained offline visualization.
    #[command(next_help_heading = "Cache & Export")]
    Export {
        #[command(flatten)]
        scope: Scope,
        #[arg(long, default_value = "html", value_parser = ["html"], help_heading = "Output")]
        format: String,
        #[arg(long, help_heading = "Output")]
        output: PathBuf,
    },
    /// Create a Nao snapshot scaffold and report its integration status. Existing directories are never overwritten.
    #[command(next_help_heading = "Cache & Export")]
    InitNao {
        #[arg(long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP)]
        db: PathBuf,
        /// New workspace label. Windows allocates a private unique sibling and returns its actual path.
        #[arg(long, visible_alias = "target")]
        output: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum SimulateCommand {
    /// Model historical seat removal, direct handoffs, tier multipliers, and boundary resets.
    #[command(next_help_heading = "Scenario")]
    Topology {
        #[arg(long, default_value = "dendro.duckdb", hide_default_value = true, help = DEFAULT_DB_HELP)]
        db: PathBuf,
        #[arg(long)]
        rig: Option<String>,
        #[arg(
            long,
            value_parser = ["1h", "24h", "7d", "30d", "all"],
            conflicts_with_all = ["from", "to"]
        )]
        window: Option<String>,
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
    write_fn: impl FnOnce(&mut dyn std::io::Write, &[u8]) -> std::io::Result<()>,
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
            .map(|meta| link_or_reparse(&meta))
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
                        if link_or_reparse(&meta) {
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

/// Symlinks and, on Windows, any reparse point (junction, mount, or symlink).
/// A junction is not reported as a Unix symlink. Both escape forms are refused.
fn link_or_reparse(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn offline_dashboard(
    request: &dendro_core::presentation::ReadRequest,
    mut report: serde_json::Value,
    from: &str,
    to: &str,
) -> Result<String> {
    let store = request.open_store()?;
    dendro_core::presentation::attach_dashboard_surface(&mut report, &store, from, to)?;
    dendro_core::presentation::offline_html(&report)
}

pub async fn run(command: Commands) -> Result<()> {
    let ui_request = match &command {
        Commands::Ui {
            db,
            rig,
            window,
            from,
            to,
            once,
            ..
        } => Some((
            db.clone(),
            rig.clone(),
            Some(window.clone().unwrap_or_else(|| "7d".into())),
            from.clone(),
            to.clone(),
            *once,
        )),
        Commands::Dashboard {
            db,
            rig,
            window,
            from,
            to,
            html: None,
            once,
            ..
        } => {
            use std::io::IsTerminal;
            let effective_once =
                *once || !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal();
            Some((
                db.clone(),
                rig.clone(),
                Some(window.clone().unwrap_or_else(|| "all".into())),
                from.clone(),
                to.clone(),
                effective_once,
            ))
        }
        _ => None,
    };
    if let Some((db, rig, window, from, to, once)) = ui_request {
        let watch_paths: Vec<PathBuf> = match crate::default_roots() {
            Ok(roots) => roots.into_iter().map(|root| root.path).collect(),
            Err(error) => {
                eprintln!("dendro: {error}");
                Vec::new()
            }
        };
        crate::tui::run_interactive_or_once(db, rig, window, from, to, once, watch_paths).await?;
        return Ok(());
    }
    tokio::task::spawn_blocking(move || -> Result<()> {
        match command {
            Commands::Materialize { scope } => {
                let request = dendro_core::presentation::ReadRequest::open(&scope.db)?;
                let (from, to, mode) = request.window(
                    scope.rig.as_deref(),
                    scope.window.as_deref(),
                    scope.from.as_deref(),
                    scope.to.as_deref(),
                )?;
                let (mut res, _published) = dendro_core::semantic::refresh_request(
                    &request,
                    scope.rig.as_deref(),
                    &from,
                    &to,
                )?;
                if scope.from.is_none() {
                    res["window_resolution"] = serde_json::json!(mode);
                }
                println!("{res}");
            }
            Commands::Ask {
                query,
                db,
                question,
                baseline,
                rig,
                window,
                from,
                to,
                format,
                list_intents,
                no_refresh,
            } => {
                use std::io::IsTerminal;
                let q = question.or(query);
                if list_intents || q.is_none() {
                    println!("{}", dendro_core::semantic::supported_intents());
                    return Ok(());
                }
                let q = q.unwrap();
                let baseline = baseline.as_deref().map(json_file).transpose()?;
                let scope_supplied =
                    rig.is_some() || window.is_some() || from.is_some() || to.is_some();
                let mut request = dendro_core::presentation::ReadRequest::open(&db)?;
                let cached = dendro_core::semantic::cached_scope_physical(request.database());
                let needs_refresh = !no_refresh
                    && (scope_supplied || cached.as_ref().is_none_or(|(_, _, _, fresh)| !*fresh));
                let mut auto_materialized = false;
                let mut resolution_mode: Option<String> = None;
                if needs_refresh && request.initialized() {
                    let effective_rig = if !scope_supplied {
                        cached
                            .as_ref()
                            .and_then(|(r, _, _, _)| r.clone())
                            .or_else(|| rig.clone())
                    } else {
                        rig.clone()
                    };
                    let (f, t, mode) = if !scope_supplied {
                        if let Some((_, c_from, c_to, _)) = cached {
                            (c_from, c_to, "cached_scope".into())
                        } else {
                            request.window(
                                effective_rig.as_deref(),
                                window.as_deref(),
                                from.as_deref(),
                                to.as_deref(),
                            )?
                        }
                    } else {
                        request.window(
                            effective_rig.as_deref(),
                            window.as_deref(),
                            from.as_deref(),
                            to.as_deref(),
                        )?
                    };
                    let (_, published) = dendro_core::semantic::refresh_request(
                        &request,
                        effective_rig.as_deref(),
                        &f,
                        &t,
                    )?;
                    if let Some(published) = published {
                        request = published;
                    }
                    auto_materialized = true;
                    resolution_mode = Some(mode);
                }
                let mut ans = dendro_core::semantic::answer_physical(
                    request.database(),
                    &q,
                    baseline.as_ref(),
                )?;
                #[cfg(any(windows, target_os = "linux"))]
                if let Some(selected) = request.selected() {
                    ans["analytics_selection_sha256"] = serde_json::json!(crate::hex_selection(&selected.selection().sha256));
                }
                if auto_materialized {
                    ans["auto_materialized"] = serde_json::json!(true);
                    if let Some(mode) = resolution_mode {
                        ans["window_resolution"] = serde_json::json!(mode);
                    }
                }
                let fmt = format.unwrap_or_else(|| {
                    if std::io::stdout().is_terminal() {
                        "text".into()
                    } else {
                        "json".into()
                    }
                });
                if fmt == "text" {
                    print!("{}", dendro_core::presentation::human_ask_answer(&q, &ans));
                } else {
                    println!("{ans}");
                }
            }
            Commands::InitNao { db, output } => {
                let snapshot = dendro_core::semantic::init_workspace_with_receipt(&output, &db)?;
                #[cfg(windows)]
                let status = "snapshot_created_integration_pending";
                #[cfg(not(windows))]
                let status = "created";
                println!(
                    "{}",
                    serde_json::json!({"status":status, "local_only":true, "workspace":snapshot.get("workspace").cloned().unwrap_or_else(|| serde_json::json!(output)), "snapshot":snapshot})
                );
            }
            Commands::Export {
                scope,
                format: _,
                output,
            } => {
                dendro_core::refuse_secret_path(&output)?;
                let request = dendro_core::presentation::ReadRequest::open(&scope.db)?;
                let (from, to, _) = request.window(
                    scope.rig.as_deref(),
                    scope.window.as_deref(),
                    scope.from.as_deref(),
                    scope.to.as_deref(),
                )?;
                let report = dendro_core::pipeline::report_json_physical(
                    request.database(),
                    "decisions",
                    scope.rig.as_deref(),
                    &from,
                    &to,
                )?;
                let html = offline_dashboard(&request, report, &from, &to)?;
                write_new_private_output_with(&output, html.as_bytes(), |file, data| {
                    file.write_all(data)
                })?;
                println!(
                    "{{\"status\":\"exported\",\"format\":\"html\",\"seat_labels\":\"aliases\"}}"
                );
            }
            Commands::Dashboard {
                db,
                rig,
                window,
                from,
                to,
                html: Some(output),
                ..
            } => {
                dendro_core::refuse_secret_path(&output)?;
                let request = dendro_core::presentation::ReadRequest::open(&db)?;
                let (from, to, mode) = request.window(
                    rig.as_deref(),
                    Some(window.as_deref().unwrap_or("all")),
                    from.as_deref(),
                    to.as_deref(),
                )?;
                let report = dendro_core::pipeline::report_json_physical(
                    request.database(),
                    "decisions",
                    rig.as_deref(),
                    &from,
                    &to,
                )?;
                let html_str = offline_dashboard(&request, report, &from, &to)?;
                write_new_private_output_with(&output, html_str.as_bytes(), |file, data| {
                    file.write_all(data)
                })?;
                println!(
                    "{}",
                    serde_json::json!({
                        "status": "exported",
                        "format": "html",
                        "seat_labels": "aliases",
                        "output": dendro_core::redact(&output.display().to_string()),
                        "from": from,
                        "to": to,
                        "window_resolution": mode
                    })
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
                        window,
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
                let request = dendro_core::presentation::ReadRequest::open(&db)?;
                let (from, to, _) = request.window(
                    rig.as_deref(),
                    Some(window.as_deref().unwrap_or("24h")),
                    from.as_deref(),
                    to.as_deref(),
                )?;
                let scenario = build_simulation_scenario(
                    scenario.as_deref(),
                    remove,
                    replacement_seat,
                    direct_handoffs,
                    tier_policy.as_deref(),
                    boundary_reset_threshold,
                    enforce_model_family_diversity,
                )?;
                let report = dendro_core::pipeline::report_json_physical(
                    request.database(),
                    "decisions",
                    rig.as_deref(),
                    &from,
                    &to,
                )?;
                println!("{}", dendro_core::simulation::simulate(&report, &scenario)?);
            }
            Commands::Ui { .. } | Commands::Dashboard { html: None, .. } => unreachable!(),
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
            #[cfg(any(windows, target_os = "linux"))]
            {
                let root = dendro_core::platform_v1::create_user_private_root_with_choice(
                    &format!(".dendro-cli-ext-{prefix}-"),
                    &dendro_core::platform_v1::StorageRootChoice::UserProfile,
                )
                .expect("explicit USERPROFILE private fixture root");
                Self(root.path().to_path_buf())
            }
            #[cfg(not(any(windows, target_os = "linux")))]
            {
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

    /// Unix keeps a dangling symlink. Windows creates an unprivileged directory
    /// junction to an outside directory. Neither fixture is treated as the other OS.
    fn install_escaping_hold(link: &Path, target: &Path) {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).expect("unix dangling symlink");
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            fs::create_dir_all(target).expect("junction target directory");
            // One `cmd /C` string. Separate argv tokens are not a reliable mklink command line.
            let status = std::process::Command::new("cmd")
                .arg("/C")
                .raw_arg(format!(
                    "mklink /J \"{}\" \"{}\"",
                    link.display(),
                    target.display()
                ))
                .status()
                .expect("mklink /J");
            assert!(
                status.success(),
                "windows directory junction could not be created: {status}"
            );
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (link, target);
            panic!("hold escape fixture requires a unix symlink or a windows junction");
        }
    }

    #[test]
    fn dangling_symlink_hold_dir_is_refused_with_explicit_containment_error() {
        let dir = TempDir::new("hold-symlink");
        let root = dir.0.join("source");
        fs::create_dir_all(&root).unwrap();
        let db = root.join("analytics.duckdb");
        fs::write(&db, b"placeholder").unwrap();
        install_escaping_hold(
            &root.join("dangling-holds"),
            &dir.0.join("outside-missing-target"),
        );
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

    #[test]
    fn dashboard_html_embeds_refused_insights_for_empty_store() {
        let dir = TempDir::new("insights");
        let db = dir.0.join("empty.duckdb");
        dendro_core::AnalyticsStore::open_path(&db, "insights")
            .unwrap()
            .checkpoint()
            .unwrap();
        let request = dendro_core::presentation::ReadRequest::open(&db).unwrap();
        let html = offline_dashboard(
            &request,
            json!({"metric":"decisions","payload":{"product":"AI Fleet Intelligence"}}),
            "2026-01-01T00:00:00Z",
            "2026-01-02T00:00:00Z",
        )
        .unwrap();
        assert!(html.contains("\"id\":\"I09\""));
        assert!(html.contains("\"state\":\"refused\""));
        assert!(html.contains("empty population"));
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("\"state\":\"computed\""));
    }
}
