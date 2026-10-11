# Dendro v0.1.1 release notes

Product content from the `pm.manager` draft (sha256 `ad1346d1`), placed for hotfix v0.1.1. Values that independent QA owns are marked **PENDING independent QA**. The final QA report names the candidate they come from and supplies them. Developer measurements are labelled with the code commit and inputs they come from; QA confirms them. Nothing here is published until the release freeze is lifted.

---

## Dendro v0.1.1: local AI Fleet Intelligence

Dendro reads the local records of an OpenRig agent fleet (OpenRig, Claude Code, Codex and Pi) and answers three questions:
- How do seats spend their time?
- How does work move between seats?
- Where do the tokens go?

Every number traces back to its source records. Dendro runs locally and opens its sources read-only. It never changes how agents run.

Commands: `dendro run` (ingest), `dendro report`, and `dendro trace`.

### What v0.1.1 fixes

v0.1.1 fixes how Dendro reads Claude Code tool calls. Token totals do not change.

- **Streamed tool calls are no longer lost (#35).** Claude Code writes one reply over several log lines that share a message id. Dendro counted the reply's tokens once, correctly, but also dropped the tool calls on the later lines. It now keeps each tool call once, by its tool-call id. A repeated copy of the same call is not a new call.
- **Rejected records give no facts (#36).** A record of an unknown type, or a malformed one, is skipped with its warning and contributes nothing: no tokens, tool calls or links.
- **Tool failures are read where Claude Code records them (#37).** A tool call fails when its `tool_result` says `is_error: true`, matched by the tool-call id in the same session and file.
  - `false` means success.
  - A missing result, a missing or non-true/false flag, or results that disagree mean the outcome is unknown.
  - A result that matches no call is ignored.
  - A result that arrives in a later ingest settles the call then. Ingesting a file in parts gives the same answer as ingesting it whole.
- **Tool inputs are compared as written.** Two calls have the same input only if the input's JSON text is identical, apart from whitespace at the ends. This is needed for loops to be found on real Claude Code logs, whose tool inputs are JSON objects.

Developer measurement on code commit `c8f5f7a`, sealed real-fleet input (manifest `40c65db7`), window 2026-10-08 to 2026-10-11. To be confirmed by independent QA:

| | v0.1.0 candidate `2abeb8f` | v0.1.1 code |
| --- | --- | --- |
| Claude tool calls kept | 524 | 1,989 (equal to an independent count of distinct tool-call ids) |
| Claude file writes among them | 5 | 105 |
| Tool calls with a known outcome | 0 | 1,838 (27 failed, 1,811 succeeded; 151 unknown) |
| Tool loops found | 0 | 0 (an independent count also finds 0) |
| Total intake estimate, `agentflow-dev` handoffs | 140,750,841 tokens | 102,119,087 tokens |
| Total tokens | 859,950,405 | 859,950,405 (unchanged) |

The intake estimate falls by 38,631,754 tokens because the earlier candidate missed four real file writes that end the intake window. Each of the four now matches QA's independent computation.

### First run

- First-run time, from the start of ingest to the first report, with the build excluded: **PENDING independent QA** (one isolated run on a new analytics database, at most 300 s, no retry).
- Measurement conditions: **PENDING independent QA**.
- For earlier candidates, independent QA measured 244.7 s (`6292fff`) and 247.1 s (`2abeb8f`). During development a heavy concurrent process once pushed it to 314 s. Expect a longer first run on a busy machine or with more source data.

### Before you upgrade: use a new analytics database

- Ingest into a new analytics database to get correct numbers. Earlier development builds identified Pi and Codex sessions incorrectly, and v0.1.0 lost Claude Code tool calls.
- An older (v1) analytics database is refused with "unsupported analytics schema version 1". Your data is not changed. There is no migration.
- Also start a new analytics database after you reset or reinstall OpenRig. See the known limits below.

### How Pi and Codex sessions are identified

- A session's id comes from line 1 of its session file: the Pi `session` header or the Codex `session_meta` header.
- If a later header in the same file has a different id, Dendro keeps the first one and prints the warning "conflicting session header". Tokens are counted once.
- If line 1 has no header, or the header is malformed, the file gets its own id, `pi-file:<source>` or `codex-file:<source>`. That session is shown as unlinked, a warning is printed, and it is never merged with another file.

### Known limits in v0.1.1

1. **One OpenRig root per analytics database.**
   - If you add a second OpenRig root to the same database, its records whose sequence numbers or ids were already used are dropped silently, so totals come out low.
   - Use a separate analytics database for each OpenRig root.
   - A refusal guard is planned (#31). Several roots in one view is Goal 3 (#27).
2. **After an OpenRig reset or reinstall, start a new analytics database.** If you re-run over an existing database, new OpenRig records that reuse old sequence numbers or ids are dropped silently, so totals come out low.
3. **OpenRig records are treated as append-only.**
   - Records that OpenRig deletes under its retention policy keep their facts in Dendro, and nothing marks them as gone.
   - Retroactive changes to past OpenRig records are not tracked.
   - Planned for v0.2.0 (#13).
4. **Trace detail for OpenRig rows.** The trace gives the OpenRig source revision of the latest ingest, not the revision that first produced each row. File sources (Claude Code, Codex and Pi) keep per-input revisions.
5. **Seat presence without stop events.** OpenRig 0.6.6 emits no stop or removal event for a single seat. A seat removed without a rig stop counts as present until the next event or the end of the window, so idle time can be overstated.
6. **Late handoffs.** A handoff counts as returned only if it comes back inside the same report window. Handoffs near the end of the window have had less time to come back, so the return rate can look better than it is.
7. **Sources that are written very fast.**
   - Dendro copies a live source before reading it. If a source changes too fast to copy consistently, ingest stops cleanly and asks you to retry. Your sources are not changed.
   - In QA tests, ingest succeeded every time with sources written at up to 50 commits per second (15 of 15 runs). The busiest real source seen was 26 events in 1 s.
   - This is a tested range, not a guaranteed rate.
8. **Deleted or replaced session files.** If a session file is deleted (Claude Code prunes old sessions, for example) or replaced, Dendro keeps the facts it already ingested. The trace does not yet mark the file as gone or superseded. Planned for v0.2.0 (#32).
9. **Tool-loop trace.** The tool-loop count cannot yet be traced back to its source records the way every other number can. Planned for v0.2.0 (#33).
10. **Whole-second durations.** Wait times and their medians are shown in whole seconds, rounded down (for example, 24.7 s shows as 24 s). Planned for v0.2.0 (#34).
11. **Activity trace detail.**
    - Each seat's activity bands add up exactly. They cannot yet all be recomputed from the trace alone, because the queue actions of the sending seat are used but not shown in the trace.
    - Mismatch count: **PENDING independent QA**.
    - Planned for v0.2.0 (#33).
12. **Tool calls come from Claude Code only.** Dendro reads tool calls, and so tool loops and file writes that end an intake window, from Claude Code logs. Codex and Pi tool calls are not read in this release.

### Unknown record types

Dendro knows the record types written by the Claude Code, Codex and Pi versions in use on 2026-10-11 (28 types in all). If a source file contains a type outside that list, Dendro skips those records and warns once per file and type, without showing any content. A new harness version that adds record types will produce these warnings until Dendro learns the new types.

### Tool-loop rule

A tool loop is the same seat, in the same session, running the same tool with the same input and failing 3 or more times in a row.
- It counts when any 3 consecutive failures fall within 10 minutes. The whole run is one loop, with its full count.
- A success, an unknown outcome or any other tool call breaks the run.
- The thresholds are fixed in v0.1.1.

### Privacy

Dendro never stores or prints the content of prompts, responses, tool arguments, tool results or error messages. Tool inputs are compared only through a keyed fingerprint. Identifiers such as mission tags, tool names and file paths do appear, except values that look like secrets, which are redacted. Nothing leaves the machine.
