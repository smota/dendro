use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        Path::new(&manifest_dir)
            .parent()
            .expect("crates dir")
            .parent()
            .expect("repo root")
            .to_path_buf()
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        if cwd.join("Cargo.toml").exists() && cwd.join("docs").exists() {
            cwd
        } else if cwd
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("Cargo.toml").exists())
            .unwrap_or(false)
        {
            cwd.parent().unwrap().parent().unwrap().to_path_buf()
        } else {
            cwd
        }
    }
}

fn to_kebab_case(s: &str) -> String {
    let mut result = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('-');
            }
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}

fn parse_subcommands_from_enum(source: &str, enum_header: &str) -> BTreeSet<String> {
    let mut subcommands = BTreeSet::new();
    let Some(enum_pos) = source.find(enum_header) else {
        return subcommands;
    };
    let after_enum = &source[enum_pos + enum_header.len()..];
    let Some(open_brace) = after_enum.find('{') else {
        return subcommands;
    };
    let body = &after_enum[open_brace + 1..];

    let mut depth = 1;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with("#[") {
            continue;
        }

        if depth == 1 {
            if trimmed.starts_with('}') {
                break;
            }
            let ident: String = trimmed
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !ident.is_empty()
                && ident
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_uppercase())
                    .unwrap_or(false)
                && ident != "Extensions"
            {
                subcommands.insert(to_kebab_case(&ident));
            }
        }

        for ch in line.chars() {
            if ch == '{' {
                depth += 1;
            } else if ch == '}' {
                depth -= 1;
            }
        }
    }

    subcommands
}

#[test]
fn test_cli_subcommands_appear_in_cli_docs() {
    let root = repo_root();
    let main_rs = fs::read_to_string(root.join("crates/dendro-cli/src/main.rs"))
        .expect("main.rs must be readable");
    let extensions_rs = fs::read_to_string(root.join("crates/dendro-cli/src/extensions.rs"))
        .expect("extensions.rs must be readable");
    let cli_r1 =
        fs::read_to_string(root.join("docs/CLI_R1.md")).expect("docs/CLI_R1.md must be readable");

    let mut declared_subcommands = parse_subcommands_from_enum(&main_rs, "enum Commands");
    let ext_subcommands = parse_subcommands_from_enum(&extensions_rs, "pub enum Commands");
    declared_subcommands.extend(ext_subcommands);

    // Also include canonical known subcommands for vNext
    let canonical_commands = [
        "storage-init",
        "recover-selection",
        "migrate",
        "rollback",
        "status",
        "watch",
        "run-agent",
        "run",
        "serve",
        "advise",
        "insights",
        "report",
        "trace",
        "assert",
        "issue-draft",
        "dashboard",
        "ui",
        "ask",
        "simulate",
        "portfolio",
        "materialize",
        "export",
        "init-nao",
    ];
    for cmd in canonical_commands {
        declared_subcommands.insert(cmd.to_string());
    }

    // Assert that every declared subcommand appears in docs/CLI_R1.md
    let mut missing = Vec::new();
    for cmd in &declared_subcommands {
        let needle1 = format!("`{cmd}`");
        let needle2 = format!("dendro {cmd}");
        if !cli_r1.contains(&needle1) && !cli_r1.contains(&needle2) {
            missing.push(cmd.clone());
        }
    }

    assert!(
        missing.is_empty(),
        "Subcommands missing from docs/CLI_R1.md: {missing:?}"
    );

    // If binary is available, verify dendro --help lists commands
    if let Ok(bin_path) = std::env::var("CARGO_BIN_EXE_dendro") {
        if Path::new(&bin_path).exists() {
            let output = Command::new(&bin_path)
                .arg("--help")
                .output()
                .expect("dendro --help must succeed");
            let help_str = String::from_utf8_lossy(&output.stdout);
            for cmd in ["status", "run", "dashboard", "ui", "ask", "report", "trace"] {
                assert!(
                    help_str.contains(cmd),
                    "dendro --help output missing command: {cmd}"
                );
            }
        }
    }
}

#[test]
fn test_canonical_schema5_tables_appear_in_docs() {
    let root = repo_root();
    let data_dict = fs::read_to_string(root.join("docs/DATA_DICTIONARY.md"))
        .expect("docs/DATA_DICTIONARY.md must be readable");
    let architecture = fs::read_to_string(root.join("docs/ARCHITECTURE.md"))
        .expect("docs/ARCHITECTURE.md must be readable");

    const CANONICAL_SCHEMA5_TABLES: &[&str] = &[
        "harness_envelope",
        "_dendro_tripwire_outbox",
        "harness_trajectory",
        "_dendro_pricing_tiers",
        "_dendro_redacted_diagnostics",
        "usage_facts",
        "tool_facts",
        "message_facts",
        "identity_facts",
        "activity_facts",
        "queue_facts",
        "skill_facts",
        "security_facts",
        "session_edges",
        "process_identities",
        "process_samples",
        "file_source_baselines",
        "warnings",
        "dendro_meta",
        "dendro_semantic_state",
    ];

    let mut missing_dict = Vec::new();
    let mut missing_arch = Vec::new();

    for table in CANONICAL_SCHEMA5_TABLES {
        if !data_dict.contains(table) {
            missing_dict.push(*table);
        }
        if !architecture.contains(table) {
            missing_arch.push(*table);
        }
    }

    assert!(
        missing_dict.is_empty(),
        "Canonical Schema 5 tables missing from docs/DATA_DICTIONARY.md: {missing_dict:?}"
    );
    assert!(
        missing_arch.is_empty(),
        "Canonical Schema 5 tables missing from docs/ARCHITECTURE.md: {missing_arch:?}"
    );
}

#[test]
fn test_version_stamps_match_backlog_and_cli() {
    let root = repo_root();
    let cargo_toml =
        fs::read_to_string(root.join("Cargo.toml")).expect("root Cargo.toml must be readable");
    let backlog =
        fs::read_to_string(root.join("docs/BACKLOG.md")).expect("docs/BACKLOG.md must be readable");

    // Extract workspace package version
    let mut version = None;
    let mut in_workspace_package = false;
    for line in cargo_toml.lines() {
        let trimmed = line.trim();
        if trimmed == "[workspace.package]" {
            in_workspace_package = true;
            continue;
        } else if trimmed.starts_with('[') {
            in_workspace_package = false;
        }
        if in_workspace_package && trimmed.starts_with("version") {
            if let Some(val) = trimmed.split('=').nth(1) {
                let v = val.trim().trim_matches('"');
                version = Some(v.to_string());
                break;
            }
        }
    }

    let version = version.expect("version must be found in [workspace.package] in Cargo.toml");
    assert!(!version.is_empty(), "version string must not be empty");

    // Verify version appears in docs/BACKLOG.md
    assert!(
        backlog.contains(&version) || backlog.contains(&format!("v{version}")),
        "docs/BACKLOG.md must reference version {version}"
    );

    // Verify version against CLI --version if binary exists
    if let Ok(bin_path) = std::env::var("CARGO_BIN_EXE_dendro") {
        if Path::new(&bin_path).exists() {
            let output = Command::new(&bin_path)
                .arg("--version")
                .output()
                .expect("dendro --version must succeed");
            let version_output = String::from_utf8_lossy(&output.stdout);
            assert!(
                version_output.contains(&version),
                "dendro --version ({version_output}) must match Cargo.toml version ({version})"
            );
        }
    }
}
