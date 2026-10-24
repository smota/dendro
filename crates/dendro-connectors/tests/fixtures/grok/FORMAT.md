# Grok CLI session format (observed)

Sampled 4 local sessions (1,936 event lines) plus an integrity scan of all 399 sessions.
Structure only: no real values are reproduced. Fixture: `session/` in this directory (synthetic).

## File layout

```
~/.grok/sessions/<url-encoded cwd>/<session-id>/
  updates.jsonl          primary event log (one JSON object per line)
  events.jsonl           lifecycle, permission and tool-timing log
  chat_history.jsonl     message model: user/system/assistant/tool_result/reasoning
  rewind_points.jsonl    per-prompt file snapshots (FULL file contents; see Privacy)
  summary.json           session metadata (ids, cwd, model, effort, kind)
  usage.json             session totals + per-turn totals
  prompt_context.json, system_prompt.txt, tool_definitions.json, signals.json,
  resources_state.json, plan.json, plan_mode.json, announcement_state.json,
  title_refresh_idx, compaction/, compaction_checkpoints/, terminal/
  subagents/<subagent_id>/meta.json, output.json
  *.lock                 sibling lock files (ignore)
~/.grok/sessions/<url-encoded cwd>/prompt_history.jsonl   per-cwd prompt log
```

- `<url-encoded cwd>` is the cwd percent-encoded (`:` -> `%3A`, `\` -> `%5C`).
- `<session-id>` is a UUID with a time-ordered prefix (`01a1...` in the sample).
- Subagent child sessions are full session dirs. All 77 `child_session_id` values
  referenced by `subagents/*/meta.json` resolve to a `sessions/*/<id>/` directory.

## updates.jsonl envelope (every line)

```
{ "method": "session/update" | "_x.ai/session/update",
  "params": { "sessionId": str,
              "_meta": { "agentTimestampMs": int(13 digits), "eventId": "<uuid>-<n>", ... },
              "update": { "sessionUpdate": <discriminator>, ... } },
  "timestamp": int (10 digits, epoch seconds) }
```

- Discriminator: `params.update.sessionUpdate`. `method` splits ACP-standard
  `session/update` from the xAI extension `_x.ai/session/update`.
- `params._meta` carries chunk and tool bookkeeping (`updateType`, `chunkId`, `promptId`,
  `requestTrimmedTokens`, `totalTokens`, `updateParams{toolCallId,title,kind,status}`).

## Record kinds (sample counts, 4 sessions)

| method | sessionUpdate | count | key fields |
|---|---|---|---|
| session/update | user_message_chunk | 6 | update.content.text, update._meta.{modelId,promptIndex} |
| session/update | agent_thought_chunk | 110 | update.content.text, _meta.{chunkId,promptId,totalTokens} |
| session/update | agent_message_chunk | 17 | update.content.text |
| session/update | tool_call | 354 | update.toolCallId, title, rawInput, _meta["x.ai/tool"].name |
| session/update | tool_call_update | 706 | update.toolCallId; progress (no status) or completed/failed |
| _x.ai/session/update | hook_execution | 727 | event_name, runs[], tool_name (tool hooks only) |
| _x.ai/session/update | turn_completed | 5 | prompt_id, stop_reason, elapsed_ms, usage{} |
| _x.ai/session/update | subagent_spawned | 3 | subagent_id, child_session_id, attempt_id, model |
| _x.ai/session/update | subagent_finished | 3 | subagent_id, status, output, tokens_used, tool_calls |
| _x.ai/session/update | background_tasks | 2 | tasks[] (empty in sample) |
| _x.ai/session/update | compaction_checkpoint | 1 | checkpoint_id, checkpoint_file, schema_version |
| _x.ai/session/update | auto_compact_started | 1 | tokens_used, context_window, percentage, reason |
| _x.ai/session/update | auto_compact_completed | 1 | tokens_before, tokens_after, elapsed_ms |

## Session id, cwd, timestamps

- Session id: `params.sessionId` (every update), the directory name, `summary.json` `info.id`,
  `events.jsonl` turn_started `session_id`, `prompt_history.jsonl` `session_id`.
- cwd: `summary.json` `info.cwd` (also `prompt_context.json` `working_directory`). Absolute
  Windows path. The parent directory name is the URL-encoded form.
- Time: envelope `timestamp` (int seconds); `_meta.agentTimestampMs` (int ms). `events.jsonl`
  `ts` is a string (format not verified). `summary.json` `created_at`/`updated_at` are strings.

## Model, reasoning effort

- Model name: `user_message_chunk.update._meta.modelId`; `events.jsonl` turn_started
  `model_id`; `chat_history.jsonl` assistant `model_id`; `summary.json` `current_model_id`;
  keys of `usage.modelUsage`. Only `grok-4.7-build-fast` observed.
- Reasoning effort: `summary.json` `reasoning_effort` (session) and `chat_history.jsonl`
  assistant `reasoning_effort` (per message). Not present in updates.jsonl. Only `medium` seen.

## Token usage

Source: `turn_completed.update.usage` (also `usage.json` `turns[]` / `session`).

- Top level: `inputTokens`, `outputTokens`, `reasoningTokens`, `cachedReadTokens`,
  `cacheCreationTokens`, `totalTokens`, `modelCalls`, `numTurns`, `apiDurationMs`,
  `costUsdTicks`.
- Per model: `usage.modelUsage.<model>.{same fields}`.
- `totalTokens == inputTokens + outputTokens` held in 4/4 sampled turns (reasoning is
  inside output). `elapsed_ms` and `prompt_id` sit on `update`, not inside `usage`.
- Chunk `_meta.requestTrimmedTokens` / `totalTokens` are context-size gauges, not billing.

## Tool calls and results

- Call: `session/update` `tool_call`. `update.toolCallId` (string, 43 chars, starts `call`).
  Tool name at `update._meta["x.ai/tool"].name`; also `kind` (read/search/edit/execute/other),
  `label`, `read_only`. `rawInput` keys vary by tool (see Ambiguities).
- Progress: `tool_call_update` with no `status` key; carries `kind`, `locations[]`, `title`,
  `rawInput`. `params._meta.updateParams.status` is null.
- Result: `tool_call_update` with `update.status` = `completed` (most) or `failed` (2 in sample).
  `update.content[]` items are `{type:"content", content:{type:"text",text}}` or
  `{type:"diff", path, oldText, newText}`. `update.rawOutput` is typed by its `type` field
  (e.g. `FileContent`, `EditsApplied`, `ListDir`, `GrepSearch`, `Bash`, `MultiResult`).
- Pairing: `tool_call_update.toolCallId == tool_call.toolCallId`. Across 399 sessions: 16,046
  `tool_call` and 31,562 `tool_call_update` records; 0 missing ids; 0 orphan results.
- chat_history: assistant `tool_calls[].{id,name,arguments}` (arguments is a JSON string);
  `tool_result.tool_call_id` matches (32/32 in sample).
- events.jsonl: `tool_started` has no call id; `tool_completed` has `tool_call_id`, `outcome`,
  `duration_ms`.

## Subagents and hooks

- `subagent_spawned`: `subagent_id`, `child_session_id`, `attempt_id`, `parent_session_id`,
  `parent_prompt_id`, `model`, `subagent_type`, `description`, `effective_context_source`.
- `subagent_finished`: `subagent_id`, `child_session_id`, `attempt_id`, `status`, `output`,
  `tokens_used`, `tool_calls`, `turns`, `duration_ms`, `will_wake`.
- Child `summary.json` has `session_kind: "subagent"`; the parent's is `"headless"`.
- `hook_execution.update.event_name`: pre_tool_use, post_tool_use, user_prompt_submit,
  session_start, session_end, stop, subagent_stop. `runs[].status.status`: success | failed.

## Append-only and versioning

- Append-only in appearance: `updates.jsonl`, `events.jsonl`, `chat_history.jsonl`,
  `rewind_points.jsonl` (each with a `.lock`). No rewrite observed. Not verified byte-wise;
  a truncated final line after a crash is possible.
- Rewritten: `summary.json`, `usage.json`, `signals.json`, `prompt_context.json`.
- No per-line version field. Versions: `chat_format_version` (1, summary.json);
  `schema_version` ("1.0" on events turn_started; 1 on compaction_checkpoint);
  `prompt_context.version` (number); `~/.grok/version.json` (`version`, `stable_version`).

## Ambiguities

1. Casing differs: `updateParams.status` is `Pending`/`Completed`/`Failed`; `update.status` is
   `completed`/`failed`.
2. `tool_call` updateParams.kind is `Other`; `x.ai/tool.kind` is `read`/`search`/`edit`/`execute`.
3. rawInput keys vary per tool and were not fully mapped; treat as opaque JSON.
4. Only `end_turn` seen for stop_reason; other values unknown.
5. Whether `cachedReadTokens` is a subset of `inputTokens` is unverified.
6. `turn_completed.prompt_id` (uuid) and `user_message_chunk.promptIndex` (int) both exist;
   the join between them was not verified.
7. `background_tasks.tasks[]` item shape unknown (always empty in sample).
8. `chat_history` `reasoning.encrypted_content` is opaque; do not expect to decode it.

## Privacy (parser must not persist raw text)

`updates.jsonl` text, `chat_history.jsonl`, `prompt_history.jsonl` (user prompts), and
`rewind_points.jsonl` (full file contents) all contain user or file data. Extract counts,
ids, timestamps and token fields only. Never read `~/.grok/auth.json` or any token file.
