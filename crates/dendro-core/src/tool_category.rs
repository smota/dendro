use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::facts::ToolCategory;

/// Detail for a single tool category, maintaining count and AC7 traceability locators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryDetail {
    pub count: usize,
    pub source_keys: Vec<String>,
}

/// Aggregated counts and traceability for tool categories.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCategoryBreakdown {
    pub ceremony_calls: usize,
    pub engineering_calls: usize,
    pub inspect_calls: usize,
    pub other_calls: usize,
    pub total_calls: usize,
    pub ceremony_ratio: Option<f64>,
    pub categories: BTreeMap<String, CategoryDetail>,
}

impl Default for ToolCategoryBreakdown {
    fn default() -> Self {
        Self::empty()
    }
}

impl ToolCategoryBreakdown {
    /// Initialize an empty breakdown with all 8 category entries present.
    pub fn empty() -> Self {
        let mut categories = BTreeMap::new();
        for cat in ToolCategory::ALL {
            categories.insert(
                cat.as_str().to_string(),
                CategoryDetail {
                    count: 0,
                    source_keys: Vec::new(),
                },
            );
        }
        Self {
            ceremony_calls: 0,
            engineering_calls: 0,
            inspect_calls: 0,
            other_calls: 0,
            total_calls: 0,
            ceremony_ratio: None,
            categories,
        }
    }

    /// Record a single tool call category with its source locator key.
    pub fn record(&mut self, category: ToolCategory, source_key: &str) {
        let label = category.as_str();
        let entry = self
            .categories
            .entry(label.to_string())
            .or_insert_with(|| CategoryDetail {
                count: 0,
                source_keys: Vec::new(),
            });
        entry.count += 1;
        entry.source_keys.push(source_key.to_string());

        if category.is_ceremony() {
            self.ceremony_calls += 1;
        } else if matches!(
            category,
            ToolCategory::EngBuildTest | ToolCategory::EngCodeEdit | ToolCategory::EngVcs
        ) {
            self.engineering_calls += 1;
        } else if category == ToolCategory::InspectRead {
            self.inspect_calls += 1;
        } else {
            self.other_calls += 1;
        }

        self.total_calls += 1;
        self.ceremony_ratio = if self.total_calls > 0 {
            Some(self.ceremony_calls as f64 / self.total_calls as f64)
        } else {
            None
        };
    }
}

/// Fleet-wide, per-seat, and per-mission aggregation summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ToolCategoriesSummary {
    pub overall: ToolCategoryBreakdown,
    pub by_seat: BTreeMap<String, ToolCategoryBreakdown>,
    pub by_mission: BTreeMap<String, ToolCategoryBreakdown>,
}

impl ToolCategoriesSummary {
    /// Convert to the canonical JSON representation for payload["tool_categories"].
    pub fn to_payload_value(&self) -> Value {
        json!({
            "ceremony_calls": self.overall.ceremony_calls,
            "engineering_calls": self.overall.engineering_calls,
            "inspect_calls": self.overall.inspect_calls,
            "other_calls": self.overall.other_calls,
            "total_calls": self.overall.total_calls,
            "ceremony_ratio": self.overall.ceremony_ratio,
            "categories": self.overall.categories,
            "tool_categories": self.overall.categories,
            "overall": self.overall,
            "by_seat": self.by_seat,
            "by_mission": self.by_mission,
        })
    }
}

/// Classify a tool or command invocation into one of the 8 closed operational categories.
pub fn classify_tool_invocation(
    _harness: &str,
    tool_name: &str,
    raw_command_or_input: Option<&str>,
) -> ToolCategory {
    // 1. Structured tool names
    match tool_name {
        "apply_patch"
        | "patch_apply"
        | "Write"
        | "Edit"
        | "MultiEdit"
        | "NotebookEdit"
        | "write"
        | "edit"
        | "replace_file_content"
        | "write_to_file"
        | "multi_replace_file_content" => return ToolCategory::EngCodeEdit,

        "Read" | "read" | "view_file" | "Glob" | "Grep" | "LS" | "View" | "grep_search"
        | "find_by_name" | "list_dir" | "view_file_outline" | "view_code_item" => {
            return ToolCategory::InspectRead
        }

        _ => {}
    }

    // 2. Command / shell inspection
    let Some(raw) = raw_command_or_input else {
        return ToolCategory::classify_tool_name(tool_name);
    };

    let command_text = extract_command_text(raw);
    if command_text.trim().is_empty() {
        return ToolCategory::classify_tool_name(tool_name);
    }

    classify_shell_command(&command_text)
}

fn extract_command_text(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
            if let Some(cmd) = extract_command_from_value(&val) {
                return cmd;
            }
        }
    }
    if trimmed.starts_with('"') {
        if let Ok(s) = serde_json::from_str::<String>(trimmed) {
            return s;
        }
    }
    trimmed.to_string()
}

fn format_argv(parts: &[&str]) -> String {
    if parts.len() >= 3
        && matches!(basename(parts[0]), "bash" | "sh" | "zsh")
        && matches!(parts[1], "-c" | "-lc" | "-cl")
    {
        return parts[2].to_string();
    }
    parts
        .iter()
        .map(|part| {
            if part
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, ';' | '|' | '&' | '\'' | '"' | '(' | ')'))
            {
                serde_json::to_string(part).unwrap_or_else(|_| (*part).to_string())
            } else {
                (*part).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_command_from_value(val: &Value) -> Option<String> {
    match val {
        Value::Object(map) => {
            for key in &["command", "cmd", "script"] {
                if let Some(v) = map.get(*key) {
                    if let Some(s) = v.as_str() {
                        return Some(s.to_string());
                    }
                    if let Some(arr) = v.as_array() {
                        let parts: Vec<&str> = arr.iter().filter_map(Value::as_str).collect();
                        if !parts.is_empty() {
                            return Some(format_argv(&parts));
                        }
                    }
                }
            }
            for key in &["arguments", "input"] {
                if let Some(v) = map.get(*key) {
                    if let Some(s) = v.as_str() {
                        let s_trimmed = s.trim();
                        if s_trimmed.starts_with('{') || s_trimmed.starts_with('[') {
                            if let Ok(sub) = serde_json::from_str::<Value>(s_trimmed) {
                                if let Some(cmd) = extract_command_from_value(&sub) {
                                    return Some(cmd);
                                }
                            }
                        }
                        return Some(s.to_string());
                    }
                    if let Some(cmd) = extract_command_from_value(v) {
                        return Some(cmd);
                    }
                }
            }
            None
        }
        Value::Array(arr) => {
            let parts: Vec<&str> = arr.iter().filter_map(Value::as_str).collect();
            if !parts.is_empty() {
                Some(format_argv(&parts))
            } else {
                None
            }
        }
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// Classify a raw shell command string. Handles shell chaining (&&, ||, ;, |),
/// environment assignments, and shell wrappers.
fn classify_shell_command(cmd_line: &str) -> ToolCategory {
    let segments = split_command_segments(cmd_line);
    for segment in segments {
        let cat = classify_single_segment(&segment);
        if cat != ToolCategory::Other {
            return cat;
        }
    }
    ToolCategory::Other
}

/// Split compound shell commands by ;, &&, ||, or | while respecting quoted strings.
fn split_command_segments(cmd_line: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let chars: Vec<char> = cmd_line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if escaped {
            cur.push(c);
            escaped = false;
            i += 1;
            continue;
        }
        if c == '\\' && !in_single {
            escaped = true;
            cur.push(c);
            i += 1;
            continue;
        }
        if c == '\'' && !in_double {
            in_single = !in_single;
            cur.push(c);
            i += 1;
            continue;
        }
        if c == '"' && !in_single {
            in_double = !in_double;
            cur.push(c);
            i += 1;
            continue;
        }

        if !in_single && !in_double && (c == ';' || c == '|' || c == '&') {
            let is_double_op = (c == '|' || c == '&') && i + 1 < chars.len() && chars[i + 1] == c;
            let trimmed = cur.trim().to_string();
            if !trimmed.is_empty() {
                segments.push(trimmed);
            }
            cur.clear();
            if is_double_op {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        cur.push(c);
        i += 1;
    }

    let trimmed = cur.trim().to_string();
    if !trimmed.is_empty() {
        segments.push(trimmed);
    }

    segments
}

/// Tokenize a shell command segment into unquoted tokens.
fn tokenize_shell_segment(segment: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut token_started = false;

    for c in segment.chars() {
        if escaped {
            cur.push(c);
            escaped = false;
            token_started = true;
            continue;
        }
        if c == '\\' && !in_single {
            escaped = true;
            token_started = true;
            continue;
        }
        if c == '\'' && !in_double {
            in_single = !in_single;
            token_started = true;
            continue;
        }
        if c == '"' && !in_single {
            in_double = !in_double;
            token_started = true;
            continue;
        }

        if c.is_whitespace() && !in_single && !in_double {
            if token_started || !cur.is_empty() {
                tokens.push(cur.clone());
                cur.clear();
                token_started = false;
            }
            continue;
        }

        cur.push(c);
        token_started = true;
    }

    if token_started || !cur.is_empty() {
        tokens.push(cur);
    }

    tokens
}

enum UnwrappedCommand {
    Tokens(Vec<String>),
    InnerScript(String),
}

/// Strip environment assignments, wrappers (bash -c, sudo, rtk bypass, timeout, etc.),
/// and classify the core command invocation.
fn classify_single_segment(segment: &str) -> ToolCategory {
    let tokens = tokenize_shell_segment(segment);
    if tokens.is_empty() {
        return ToolCategory::Other;
    }

    let unwrapped = match unwrap_tokens(&tokens) {
        UnwrappedCommand::InnerScript(script) => return classify_shell_command(&script),
        UnwrappedCommand::Tokens(tokens) => tokens,
    };
    if unwrapped.is_empty() {
        return ToolCategory::Other;
    }

    // Skip utility commands like cd, export, echo
    let first_base = basename(&unwrapped[0]);
    if matches!(first_base, "cd" | "export" | "echo") {
        return ToolCategory::Other;
    }

    classify_command_tokens(&unwrapped)
}

fn unwrap_tokens(tokens: &[String]) -> UnwrappedCommand {
    let mut i = 0;
    while i < tokens.len() {
        let tok = &tokens[i];
        let base = basename(tok);

        // Env variable assignment FOO=bar
        if is_env_assignment(tok) {
            i += 1;
            continue;
        }

        // rtk bypass
        if base == "rtk" && i + 1 < tokens.len() && tokens[i + 1] == "bypass" {
            i += 2;
            continue;
        }

        // sudo [options]
        if base == "sudo" {
            i += 1;
            while i < tokens.len() {
                if tokens[i].starts_with('-') {
                    if matches!(tokens[i].as_str(), "-u" | "-g" | "-h" | "-p") {
                        i += 2;
                    } else {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            continue;
        }

        // env [options] [VAR=val...]
        if base == "env" {
            i += 1;
            while i < tokens.len() {
                if tokens[i].starts_with('-') {
                    if matches!(tokens[i].as_str(), "-u") {
                        i += 2;
                    } else {
                        i += 1;
                    }
                } else if is_env_assignment(&tokens[i]) {
                    i += 1;
                } else {
                    break;
                }
            }
            continue;
        }

        // timeout [options] duration
        if base == "timeout" {
            i += 1;
            while i < tokens.len() {
                if tokens[i].starts_with('-') {
                    if matches!(tokens[i].as_str(), "-s" | "-k") {
                        i += 2;
                    } else {
                        i += 1;
                    }
                } else {
                    // skip duration token (e.g., "10", "10s", "1m")
                    i += 1;
                    break;
                }
            }
            continue;
        }

        // Shell wrappers: bash -c "...", sh -c "...", zsh -lc "..."
        if matches!(base, "bash" | "sh" | "zsh")
            && i + 1 < tokens.len()
            && matches!(tokens[i + 1].as_str(), "-c" | "-lc" | "-cl")
            && i + 2 < tokens.len()
        {
            return UnwrappedCommand::InnerScript(tokens[i + 2].clone());
        }

        if matches!(base, "nohup" | "exec" | "eval" | "xargs") {
            i += 1;
            continue;
        }

        break;
    }

    UnwrappedCommand::Tokens(tokens[i..].to_vec())
}

fn is_env_assignment(tok: &str) -> bool {
    if let Some((key, _)) = tok.split_once('=') {
        !key.is_empty()
            && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !key.chars().next().unwrap().is_ascii_digit()
    } else {
        false
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Match unwrapped command tokens against the 8 categories.
fn classify_command_tokens(tokens: &[String]) -> ToolCategory {
    if tokens.is_empty() {
        return ToolCategory::Other;
    }

    let cmd0 = basename(&tokens[0]);
    let cmd1 = tokens.get(1).map(|s| s.as_str()).unwrap_or_default();

    // coord_queue:
    // rig queue, rig send, rig handoff, handoff deliver, handoff ack, handoff hold
    if cmd0 == "rig" && matches!(cmd1, "queue" | "send" | "handoff") {
        return ToolCategory::CoordQueue;
    }
    if cmd0 == "handoff" && matches!(cmd1, "deliver" | "ack" | "hold") {
        return ToolCategory::CoordQueue;
    }

    // coord_status:
    // rig whoami, rig ps, rig startup-proof, rig health, rig seat, rig context, rig status, rig capture
    if cmd0 == "rig"
        && matches!(
            cmd1,
            "whoami"
                | "ps"
                | "startup-proof"
                | "health"
                | "seat"
                | "context"
                | "status"
                | "capture"
        )
    {
        return ToolCategory::CoordStatus;
    }

    // governance_phase:
    // agentflow-sdlc, phase append, phase read, gh issue, gh pr, gh api
    if cmd0 == "agentflow-sdlc" || cmd0.starts_with("agentflow-sdlc") {
        return ToolCategory::GovernancePhase;
    }
    if cmd0 == "phase" && matches!(cmd1, "append" | "read") {
        return ToolCategory::GovernancePhase;
    }
    if cmd0 == "gh" && matches!(cmd1, "issue" | "pr" | "api") {
        return ToolCategory::GovernancePhase;
    }

    // eng_build_test:
    // cargo, rustc, rustfmt, clippy-driver, npm test, pnpm test, pytest, go test
    if matches!(
        cmd0,
        "cargo" | "rustc" | "rustfmt" | "clippy-driver" | "pytest"
    ) {
        return ToolCategory::EngBuildTest;
    }
    if cmd0 == "npm"
        && (cmd1 == "test"
            || cmd1 == "t"
            || (cmd1 == "run" && tokens.get(2).map(|s| s.as_str()) == Some("test")))
    {
        return ToolCategory::EngBuildTest;
    }
    if cmd0 == "pnpm"
        && (cmd1 == "test"
            || cmd1 == "t"
            || (cmd1 == "run" && tokens.get(2).map(|s| s.as_str()) == Some("test")))
    {
        return ToolCategory::EngBuildTest;
    }
    if cmd0 == "go" && cmd1 == "test" {
        return ToolCategory::EngBuildTest;
    }

    // eng_code_edit:
    // apply_patch
    if cmd0 == "apply_patch" || cmd0 == "patch_apply" {
        return ToolCategory::EngCodeEdit;
    }

    // eng_vcs:
    // git
    if cmd0 == "git" {
        return ToolCategory::EngVcs;
    }

    // inspect_read:
    // rg, fd, cat, ls, sed, jq, head, tail, find, grep, wc, stat, python3, python, node
    if matches!(
        cmd0,
        "rg" | "fd"
            | "cat"
            | "ls"
            | "sed"
            | "jq"
            | "head"
            | "tail"
            | "find"
            | "grep"
            | "wc"
            | "stat"
            | "python3"
            | "python"
            | "node"
    ) {
        return ToolCategory::InspectRead;
    }

    ToolCategory::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_structured_tools() {
        assert_eq!(
            classify_tool_invocation("claude", "Write", None),
            ToolCategory::EngCodeEdit
        );
        assert_eq!(
            classify_tool_invocation("claude", "edit", None),
            ToolCategory::EngCodeEdit
        );
        assert_eq!(
            classify_tool_invocation("claude", "Read", None),
            ToolCategory::InspectRead
        );
        assert_eq!(
            classify_tool_invocation("claude", "view_file", None),
            ToolCategory::InspectRead
        );
        assert_eq!(
            classify_tool_invocation("claude", "Grep", None),
            ToolCategory::InspectRead
        );
        assert_eq!(
            classify_tool_invocation("claude", "replace_file_content", None),
            ToolCategory::EngCodeEdit
        );
    }

    #[test]
    fn test_shell_commands_all_categories() {
        // coord_queue
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("rig queue pop")),
            ToolCategory::CoordQueue
        );
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("handoff deliver --to dev")),
            ToolCategory::CoordQueue
        );
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("handoff ack")),
            ToolCategory::CoordQueue
        );

        // coord_status
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("rig whoami")),
            ToolCategory::CoordStatus
        );
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("rig ps -a")),
            ToolCategory::CoordStatus
        );
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("rig status")),
            ToolCategory::CoordStatus
        );

        // governance_phase
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("agentflow-sdlc next")),
            ToolCategory::GovernancePhase
        );
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("phase append --phase dev")),
            ToolCategory::GovernancePhase
        );
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("gh pr list")),
            ToolCategory::GovernancePhase
        );
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("gh issue view 52")),
            ToolCategory::GovernancePhase
        );

        // eng_build_test
        assert_eq!(
            classify_tool_invocation(
                "claude",
                "bash",
                Some("CARGO_TARGET_DIR=/tmp cargo test --workspace")
            ),
            ToolCategory::EngBuildTest
        );
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("pytest tests/")),
            ToolCategory::EngBuildTest
        );
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("npm test")),
            ToolCategory::EngBuildTest
        );
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("go test ./...")),
            ToolCategory::EngBuildTest
        );

        // eng_code_edit
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("apply_patch foo.patch")),
            ToolCategory::EngCodeEdit
        );

        // eng_vcs
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("rtk bypass git -C /repo status")),
            ToolCategory::EngVcs
        );
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("git diff HEAD~1")),
            ToolCategory::EngVcs
        );

        // inspect_read
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("rg 'pattern' crates/")),
            ToolCategory::InspectRead
        );
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("cat README.md")),
            ToolCategory::InspectRead
        );
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("python3 check.py")),
            ToolCategory::InspectRead
        );

        // other
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("curl https://example.com")),
            ToolCategory::Other
        );
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some("echo 'hello'")),
            ToolCategory::Other
        );
    }

    #[test]
    fn test_wrappers_and_json_extraction() {
        let json_input = r#"{"command": "bash -lc \"git status\""}"#;
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some(json_input)),
            ToolCategory::EngVcs
        );

        let sudo_cmd = "sudo timeout 10s cargo check";
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some(sudo_cmd)),
            ToolCategory::EngBuildTest
        );

        let chained = "cd /home/sam/code && pytest";
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some(chained)),
            ToolCategory::EngBuildTest
        );

        let wrapped_chained = "bash -lc 'cd /repo && rig queue pop'";
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some(wrapped_chained)),
            ToolCategory::CoordQueue
        );

        let codex_argv = r#"["/bin/bash", "-lc", "rig queue pop"]"#;
        assert_eq!(
            classify_tool_invocation("codex", "CommandExecution", Some(codex_argv)),
            ToolCategory::CoordQueue
        );

        let codex_argv_chained = r#"["/bin/bash", "-lc", "cd /repo && cargo test --workspace"]"#;
        assert_eq!(
            classify_tool_invocation("codex", "CommandExecution", Some(codex_argv_chained)),
            ToolCategory::EngBuildTest
        );

        let codex_argv_positional = r#"["bash", "-c", "echo", "; rig queue pop"]"#;
        assert_eq!(
            classify_tool_invocation("codex", "CommandExecution", Some(codex_argv_positional)),
            ToolCategory::Other
        );

        let wrapped_positional = "bash -c 'echo' '; rig queue pop'";
        assert_eq!(
            classify_tool_invocation("claude", "bash", Some(wrapped_positional)),
            ToolCategory::Other
        );
    }

    #[test]
    fn test_breakdown_aggregation() {
        let mut breakdown = ToolCategoryBreakdown::empty();
        assert_eq!(breakdown.total_calls, 0);
        assert_eq!(breakdown.ceremony_ratio, None);

        breakdown.record(ToolCategory::CoordQueue, "key-1");
        assert_eq!(breakdown.ceremony_calls, 1);
        assert_eq!(breakdown.total_calls, 1);
        assert_eq!(breakdown.ceremony_ratio, Some(1.0));

        breakdown.record(ToolCategory::EngVcs, "key-2");
        assert_eq!(breakdown.engineering_calls, 1);
        assert_eq!(breakdown.total_calls, 2);
        assert_eq!(breakdown.ceremony_ratio, Some(0.5));

        breakdown.record(ToolCategory::InspectRead, "key-3");
        assert_eq!(breakdown.inspect_calls, 1);
        assert_eq!(breakdown.total_calls, 3);
        assert_eq!(
            breakdown.categories.get("coord_queue").unwrap().source_keys,
            vec!["key-1"]
        );
        assert_eq!(
            breakdown.categories.get("eng_vcs").unwrap().source_keys,
            vec!["key-2"]
        );
        assert_eq!(
            breakdown
                .categories
                .get("inspect_read")
                .unwrap()
                .source_keys,
            vec!["key-3"]
        );
    }
}
