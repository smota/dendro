//! Security observations from explicit harness fields. Closed values only.
//! Command text and approved-prefix words stay in the parser and are dropped.

use std::collections::HashMap;

use serde_json::Value;

use crate::dangerous_command;

#[derive(Debug, Default)]
pub(crate) struct PolicyDedupe {
    last: HashMap<(String, String, String), String>,
    ordinal: HashMap<(String, String, String), u32>,
}

pub(crate) struct ObserveCtx<'a> {
    pub harness: &'a str,
    pub source_id: &'a str,
    pub generation: i64,
    pub line_no: i64,
    pub tool_key: Option<&'a [u8]>,
    pub file_session: Option<&'a str>,
    pub ts: Option<chrono::DateTime<chrono::Utc>>,
}

pub(crate) fn observe(
    ctx: &ObserveCtx<'_>,
    value: &Value,
    dedupe: &mut PolicyDedupe,
    out: &mut Vec<dendro_core::SecurityFact>,
) {
    match ctx.harness {
        "claude" => {
            observe_claude_policy(ctx, value, dedupe, out);
            let session = value.get("sessionId").and_then(Value::as_str);
            observe_content_tools(
                ctx,
                value,
                session,
                "tool_use",
                "input",
                "input.command",
                out,
            );
        }
        "codex" => observe_codex(ctx, value, dedupe, out),
        "pi" => observe_content_tools(
            ctx,
            value,
            None,
            "toolCall",
            "arguments",
            "arguments.command",
            out,
        ),
        _ => {}
    }
}

fn observe_claude_policy(
    ctx: &ObserveCtx<'_>,
    value: &Value,
    dedupe: &mut PolicyDedupe,
    out: &mut Vec<dendro_core::SecurityFact>,
) {
    let session = value.get("sessionId").and_then(Value::as_str);
    if let Some(mode) = value.get("permissionMode").and_then(Value::as_str) {
        emit_policy(
            ctx,
            dedupe,
            out,
            session,
            "permission_mode",
            map_mode(mode),
            "permissionMode",
        );
    }
    if let Some(decision) = value.get("permissionDecision").and_then(Value::as_object) {
        if let Some(raw) = decision.get("decision").and_then(Value::as_str) {
            emit_policy(
                ctx,
                dedupe,
                out,
                session,
                "permission_decision",
                map_decision(raw),
                "permissionDecision",
            );
        }
    }
    if let Some(sandboxed) = value.get("sandboxed").and_then(Value::as_bool) {
        emit_policy(
            ctx,
            dedupe,
            out,
            session,
            "sandbox_policy",
            if sandboxed { "true" } else { "false" },
            "sandboxed",
        );
    }
}

fn observe_codex(
    ctx: &ObserveCtx<'_>,
    value: &Value,
    dedupe: &mut PolicyDedupe,
    out: &mut Vec<dendro_core::SecurityFact>,
) {
    if value.get("type").and_then(Value::as_str) == Some("turn_context") {
        let payload = value.get("payload");
        if let Some(policy) = payload
            .and_then(|item| item.get("approval_policy"))
            .and_then(Value::as_str)
        {
            emit_policy(
                ctx,
                dedupe,
                out,
                None,
                "approval_policy",
                map_approval(policy),
                "payload.approval_policy",
            );
        }
        if let Some(kind) = payload
            .and_then(|item| item.pointer("/sandbox_policy/type"))
            .and_then(Value::as_str)
        {
            emit_policy(
                ctx,
                dedupe,
                out,
                None,
                "sandbox_policy",
                map_sandbox(kind),
                "payload.sandbox_policy.type",
            );
        }
        if payload
            .and_then(|item| item.pointer("/permission_profile/type"))
            .and_then(Value::as_str)
            .is_some_and(|kind| !kind.is_empty())
        {
            emit_policy(
                ctx,
                dedupe,
                out,
                None,
                "permission_profile",
                "other",
                "payload.permission_profile.type",
            );
        }
        if let Some(prefixes) = payload
            .and_then(|item| item.pointer("/state/permissions/approved_command_prefixes"))
            .and_then(Value::as_array)
        {
            emit_policy(
                ctx,
                dedupe,
                out,
                None,
                "approved_prefix_count",
                &prefixes.len().to_string(),
                "payload.state.permissions.approved_command_prefixes",
            );
        }
    }
    if let Some(command) = codex_command(value) {
        emit_command(ctx, None, &command.text, command.field, command.index, out);
        if command.disable_sandbox {
            emit_override(ctx, None, command.index, out);
        }
    }
}

struct CodexCommand {
    text: String,
    field: &'static str,
    index: usize,
    disable_sandbox: bool,
}

fn codex_command(value: &Value) -> Option<CodexCommand> {
    let envelope = value.get("type").and_then(Value::as_str)?;
    let payload = value.get("payload")?;
    match envelope {
        "event_msg" if payload.get("type").and_then(Value::as_str) == Some("item_completed") => {
            let item = payload.get("item")?;
            if item.get("type").and_then(Value::as_str) != Some("CommandExecution") {
                return None;
            }
            let text = command_text(item.get("command")?)?;
            Some(CodexCommand {
                text,
                field: "command",
                index: 0,
                disable_sandbox: false,
            })
        }
        "response_item"
            if matches!(
                payload.get("type").and_then(Value::as_str),
                Some("function_call" | "custom_tool_call" | "tool_call")
            ) =>
        {
            let name = payload.get("name").and_then(Value::as_str).unwrap_or("");
            if !is_exec_or_shell(name) {
                return None;
            }
            let parsed = payload.get("arguments").or_else(|| payload.get("input"));
            let object = parsed.and_then(as_object)?;
            let (text, field) = if let Some(cmd) = object.get("command").and_then(command_text) {
                (cmd, "arguments.command")
            } else {
                let cmd = object.get("cmd").and_then(command_text)?;
                (cmd, "arguments.cmd")
            };
            let disable_sandbox = object
                .get("dangerouslyDisableSandbox")
                .and_then(Value::as_bool)
                == Some(true);
            Some(CodexCommand {
                text,
                field,
                index: 0,
                disable_sandbox,
            })
        }
        _ => None,
    }
}

fn observe_content_tools(
    ctx: &ObserveCtx<'_>,
    value: &Value,
    session: Option<&str>,
    call_type: &str,
    input_field: &str,
    command_field: &'static str,
    out: &mut Vec<dendro_core::SecurityFact>,
) {
    let Some(content) = value.pointer("/message/content").and_then(Value::as_array) else {
        return;
    };
    for (index, block) in content.iter().enumerate() {
        if block.get("type").and_then(Value::as_str) != Some(call_type) {
            continue;
        }
        let Some(input) = block.get(input_field).and_then(as_object) else {
            continue;
        };
        if input
            .get("dangerouslyDisableSandbox")
            .and_then(Value::as_bool)
            == Some(true)
        {
            emit_override(ctx, session, index, out);
        }
        let name = block.get("name").and_then(Value::as_str).unwrap_or("");
        if !name.eq_ignore_ascii_case("bash") && !name.eq_ignore_ascii_case("shell") {
            continue;
        }
        let Some(command) = input.get("command").and_then(Value::as_str) else {
            continue;
        };
        emit_command(ctx, session, command, command_field, index, out);
    }
}

fn emit_command(
    ctx: &ObserveCtx<'_>,
    session: Option<&str>,
    command: &str,
    source_field: &'static str,
    index: usize,
    out: &mut Vec<dendro_core::SecurityFact>,
) {
    let sealed = sealed_session(ctx, session);
    for category in dangerous_command::categories(command) {
        out.push(fact(
            ctx,
            format!(
                "{}:{}:security:dangerous_command:{}:{index}:{}",
                ctx.source_id,
                ctx.generation,
                ctx.line_no,
                category.as_str()
            ),
            session_column(&sealed),
            "dangerous_command",
            category.as_str(),
            source_field,
        ));
    }
}

fn emit_override(
    ctx: &ObserveCtx<'_>,
    session: Option<&str>,
    index: usize,
    out: &mut Vec<dendro_core::SecurityFact>,
) {
    let sealed = sealed_session(ctx, session);
    out.push(fact(
        ctx,
        format!(
            "{}:{}:security:sandbox_override_requested:{}:{index}",
            ctx.source_id, ctx.generation, ctx.line_no
        ),
        session_column(&sealed),
        "sandbox_override_requested",
        "true",
        "dangerouslyDisableSandbox",
    ));
}

fn emit_policy(
    ctx: &ObserveCtx<'_>,
    dedupe: &mut PolicyDedupe,
    out: &mut Vec<dendro_core::SecurityFact>,
    session: Option<&str>,
    kind: &str,
    value: &str,
    source_field: &str,
) {
    let sealed = sealed_session(ctx, session);
    let key = (sealed.clone(), kind.to_string(), source_field.to_string());
    if dedupe.last.get(&key).is_some_and(|prior| prior == value) {
        return;
    }
    dedupe.last.insert(key.clone(), value.to_string());
    let ordinal = {
        let slot = dedupe.ordinal.entry(key).or_insert(0);
        *slot += 1;
        *slot
    };
    out.push(fact(
        ctx,
        format!(
            "{}:{}:security:{kind}:{source_field}:{ordinal}:{sealed}",
            ctx.source_id, ctx.generation
        ),
        session_column(&sealed),
        kind,
        value,
        source_field,
    ));
}

fn session_column(sealed: &str) -> Option<String> {
    (!sealed.is_empty()).then(|| sealed.to_string())
}

fn fact(
    ctx: &ObserveCtx<'_>,
    source_key: String,
    native_session: Option<String>,
    kind: &str,
    value: &str,
    source_field: &str,
) -> dendro_core::SecurityFact {
    let native_session = native_session.filter(|session| !session.is_empty());
    dendro_core::SecurityFact {
        source_key,
        source_id: ctx.source_id.to_string(),
        harness: ctx.harness.to_string(),
        native_session,
        ts: ctx.ts,
        kind: kind.to_string(),
        value: value.to_string(),
        source_field: source_field.to_string(),
        generation: ctx.generation,
    }
}

fn sealed_session(ctx: &ObserveCtx<'_>, record_session: Option<&str>) -> String {
    let Some(raw) = record_session
        .filter(|session| !session.is_empty())
        .or(ctx.file_session)
        .filter(|session| !session.is_empty())
    else {
        return String::new();
    };
    seal(ctx.tool_key, ctx.harness, raw)
}

fn seal(key: Option<&[u8]>, harness: &str, raw: &str) -> String {
    match key {
        Some(key) => dendro_core::seal_identifier(key, &format!("session:{harness}"), raw),
        None => raw.to_string(),
    }
}

fn map_mode(raw: &str) -> &'static str {
    match raw {
        "default" => "default",
        "auto" => "auto",
        "acceptEdits" => "acceptEdits",
        "plan" => "plan",
        "bypassPermissions" => "bypassPermissions",
        _ => "other",
    }
}

fn map_decision(raw: &str) -> &'static str {
    match raw {
        "allow" => "allow",
        "deny" => "deny",
        _ => "other",
    }
}

fn map_approval(raw: &str) -> &'static str {
    match raw {
        "never" => "never",
        "on-request" => "on-request",
        _ => "other",
    }
}

fn map_sandbox(raw: &str) -> &'static str {
    match raw {
        "danger-full-access" => "danger-full-access",
        "read-only" => "read-only",
        "workspace-write" => "workspace-write",
        _ => "other",
    }
}

fn as_object(value: &Value) -> Option<Value> {
    match value {
        Value::Object(_) => Some(value.clone()),
        Value::String(text) => serde_json::from_str(text).ok().filter(Value::is_object),
        _ => None,
    }
}

fn command_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    let parts = value.as_array()?;
    if parts.is_empty() || !parts.iter().all(Value::is_string) {
        return None;
    }
    Some(
        parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn is_exec_or_shell(name: &str) -> bool {
    matches!(
        name,
        "shell" | "exec" | "exec_command" | "bash" | "local_shell" | "shell_command"
    )
}
