# Dendro v0.1.1 release notes

Product content from the `pm.manager` draft, with independent QA results for candidate `45f3e59` (report SHA256 `23c5affd`). Historical comparison measurements retain their code and input provenance. Nothing here is published until the release freeze is lifted.

---

## Dendro v0.1.1: local AI Fleet Intelligence

Dendro reads the local records of an OpenRig agent fleet (OpenRig, Claude Code, Codex and Pi) and answers three questions:
- How do seats spend their time?
- How does work move between seats?
- Where do the tokens go?

Every number traces back to its source records, except the tool-loop count (known limit 9). Dendro runs locally and opens its sources read-only. It never changes how agents run.

Commands: `dendro run` (ingest), `dendro report`, and `dendro trace`.

### What v0.1.1 fixes

v0.1.1 fixes how Dendro reads Claude Code and Pi tool calls. Token totals do not change.

- **Streamed tool calls are no longer lost (#35).** Claude Code writes one reply over several log lines that share a message id. Dendro counted the reply's tokens once, correctly, but also dropped the tool calls on the later lines. It now keeps each tool call once, by its tool-call id. A repeated copy of the same call is not a new call.
- **Rejected records give no facts (#36).** A record of an unknown type, or a malformed one, is skipped with its warning and contributes nothing: no tokens, tool calls or links.
- **Tool failures are read where Claude Code records them (#37).** A tool call fails when its `tool_result` says `is_error: true`, matched by the tool-call id in the same session and file.
  - `false` means success.
  - A missing result, a missing or non-true/false flag, or results that disagree mean the outcome is unknown.
  - A result that matches no call is ignored.
  - A result that arrives in a later ingest settles the call then. Ingesting a file in parts gives the same answer as ingesting it whole.
- **Pi tool calls are read.** Each Pi `toolCall` is matched to its `toolResult` by the call id, in the session named by line 1 of the file, under the same outcome rules as Claude Code. Pi `write` and `edit` calls end an intake window; `bash` and `read` never do.
- **Tool inputs are compared as written.** Two calls have the same input only if the input's JSON text is identical, apart from whitespace at the ends. This is needed for loops to be found on real Claude Code and Pi logs, whose tool inputs are JSON objects.

Measurements on the sealed real-fleet input (manifest `40c65db7`), window 2026-10-08 to 2026-10-11. Independent QA confirmed the final tool counts, outcomes, literal-input fingerprints and intake endpoints on `45f3e59`; historical developer comparisons are labelled below.

Claude Code (code commit `c8f5f7a`, compared with the v0.1.0 candidate `2abeb8f`):

| | `2abeb8f` | v0.1.1 code |
| --- | --- | --- |
| Claude tool calls kept | 524 | 1,989 (equal to an independent count of distinct tool-call ids) |
| Claude file writes among them | 5 | 105 |
| Claude calls with a known outcome | 0 | 1,838 (27 failed, 1,811 succeeded; 151 unknown) |
| Total intake estimate, `agentflow-dev` handoffs | 140,750,841 tokens | 102,119,087 tokens |

Pi (code commit `8c80907`, compared with a rebuild of `d5ebb07`):

| | `d5ebb07` | v0.1.1 code |
| --- | --- | --- |
| Pi tool calls kept | 0 | 3,652 (equal to an independent count of distinct call ids) |
| Pi file writes and edits among them | 0 | 439 |
| Pi calls with a known outcome | 0 | 3,651 (86 failed, 3,565 succeeded; 1 unknown) |
| Intake estimates whose end moves earlier to a Pi write or edit | — | 42 of 62 across the full fleet; 39 of 56 in `agentflow-dev` (all 62 match independent endpoints) |
| Total intake estimate, `agentflow-dev` handoffs | 102,119,087 tokens | 79,759,804 tokens |

In both comparisons:
- Claude and Pi tool loops found: 0. Independent source replay also finds 0: the longest consecutive same-input failure run is 1 in each harness. Codex coverage is unavailable, not measured as zero.
- Total tokens: 859,950,405, unchanged.
- Codex tool calls: not read (see known limit 11).

The Claude change lowers the intake estimate by 38,631,754 tokens: the earlier candidate missed four real file writes that end the intake window, and each of the four now matches QA's independent computation. The Pi change lowers the `agentflow-dev` estimate by a further 22,359,283 tokens. Across the full fleet, the Pi reduction is 28,058,110 tokens. No no-data rows convert: those rows lack a claim or a linked session.

### First run

- First-run time: **288 s** (287.961 s measured: ingest 286.090 s plus first report 1.870 s). One isolated attempt on a fresh analytics database, with a shared monotonic 300 s deadline and no retry. Build and input preparation are excluded.
- Measurement conditions: sealed real-fleet sources (97 files), candidate `45f3e59`, Linux x86_64 under WSL2 (kernel 6.18.33.2), AMD Ryzen AI 9 HX 470, 18 logical CPUs, 64 GB RAM, Rust/Cargo 1.98.0, Python 3.14.7. No concurrent QA suites; one-minute load rose from 0.66 to 2.63. Measured 2026-10-12 03:24:53–03:29:42 UTC.
- **Only 12 s of margin remained below the 300 s limit.** This is a measurement on these inputs and this machine, not a guarantee for other loads or larger fleets. Performance headroom is tracked in #41.
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
   - The released v0.1.1 tag does not enforce this limit. Use a separate analytics database for each OpenRig root to avoid silent undercounts when records from different roots share sequence numbers or ids.
   - The reviewed local v0.2.0 candidate for #31 adds the refusal; it was not shipped in v0.1.1 and has not been published. Two different OpenRig files in one run are refused with "v0.1.0 supports exactly one OpenRig root per analytics database", leaving the analytics database absent or byte-unchanged.
   - In that candidate, a database already bound to one root refuses a different root without changing the database bytes. The message names the bound root and the refused one and tells the operator to use a fresh analytics database. The same file under another spelling (a relative path, `..`, or a symlink) is accepted.
   - In that candidate, a database that already holds two OpenRig roots is refused for ingest. Reports against that database still run. Several roots in one view is Goal 3 (#27).
2. **After an OpenRig reset or reinstall, start a new analytics database.** If you re-run over an existing database, new OpenRig records that reuse old sequence numbers or ids are dropped silently, so totals come out low.
3. **OpenRig records are treated as append-only.**
   - Records that OpenRig deletes under its retention policy keep their facts in Dendro, and nothing marks them as gone.
   - Retroactive changes to past OpenRig records are not tracked.
   - Planned for v0.2.0 (#13).
4. **Trace detail for OpenRig rows.** The trace gives the OpenRig source revision of the latest ingest, not the revision that first produced each row. File sources (Claude Code, Codex and Pi) keep per-input revisions.
5. **Seat presence without stop events.** OpenRig 0.6.6 emits no stop or removal event for a single seat. A seat removed without a rig stop counts as present until the next event or the end of the window, so idle time can be overstated.
6. **Late handoffs.** A handoff counts as returned only if it comes back inside the same report window. Handoffs near the end of the window have had less time to come back, so the return rate can look better than it is.
7. **Sources that are written very fast.**
   - Dendro copies a live source before reading it. If a source changes too fast to copy consistently, ingest stops cleanly and reports that the source changed and nothing was ingested. Retry when the source is quieter. Your sources are not changed.
   - In QA tests, ingest succeeded every time with sources written at up to 50 commits per second (15 of 15 runs). The busiest real source seen was 26 events in 1 s.
   - This is a tested range, not a guaranteed rate.
8. **Deleted or replaced session files.** If a session file is deleted (Claude Code prunes old sessions, for example) or replaced, Dendro keeps the facts it already ingested. The trace does not yet mark the file as gone or superseded. Planned for v0.2.0 (#32).
9. **Tool-loop trace.** The tool-loop count cannot yet be traced back to its source records the way every other number can. Planned for v0.2.0 (#33).
10. **Whole-second durations.** Wait times and their medians are shown in whole seconds, rounded down (for example, 24.7 s shows as 24 s). Planned for v0.2.0 (#34).
11. **Codex tool calls are not read yet.** Codex loop counts are not available. `dendro run` shows `tool_calls=not read` for Codex, and each Codex file records the warning "tool calls not read for codex". A Codex intake window ends only at a queue action, because Codex records no structured file write. Planned for v0.2.0 (#40).

Activity trace replay passed all 365 independent comparisons, including all 28 seats; the earlier activity-trace limitation is removed.

### Unknown record types

Dendro knows the record types written by the Claude Code, Codex and Pi versions in use on 2026-10-11 (28 types in all). If a source file contains a type outside that list, Dendro skips those records and warns once per file and type, without showing any content. A new harness version that adds record types will produce these warnings until Dendro learns the new types.

### Tool-loop rule

A tool loop is the same seat, in the same session, running the same tool with the same input and failing 3 or more times in a row.
- It counts when any 3 consecutive failures fall within 10 minutes. The whole run is one loop, with its full count.
- A success, an unknown outcome or any other tool call breaks the run.
- The thresholds are fixed in v0.1.1.

### Privacy

Dendro never stores or prints the content of prompts, responses, tool arguments, tool results or error messages. Tool inputs are compared only through a keyed fingerprint. Identifiers such as mission tags, tool names and file paths do appear, except values that look like secrets, which are redacted. Nothing leaves the machine.
