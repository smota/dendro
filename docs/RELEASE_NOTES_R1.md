# Dendro R1 (v0.1.0) release notes

## Use a fresh analytics database

Ingest into a new, empty analytics database. Databases written by earlier R1 candidates use an older schema and are refused ("unsupported analytics schema version"). Source files are never changed, so a fresh ingest is safe. Also start a fresh analytics database after an OpenRig reset or reinstall (limit a below).

## Pi and Codex session ids (K10)

- A Pi or Codex session file is one native session, named by the id on its first line: the Pi `session` header id or the Codex `session_meta` payload id. Line 1 is read on its own, with a bounded buffer.
- If line 1 is missing or malformed, the file counts under its own id `pi-file:<source>` or `codex-file:<source>`. It stays unlinked and records the warning "missing session header". A later id never rescues it.
- A later header, or a Codex line `session_id`, that names a different session records "conflicting session header". Tokens stay under the first header and the file is not reclassified.
- Zero-usage files are discovered under the same id, and Codex counters stay per file.

## Known limits (K9, sponsor scope decision)

In v0.1.0, OpenRig event sources are treated as append-only. In-place changes to past rows, source replacement with reused sequence keys, and multi-generation replay are deferred to G2 #13.

- (a) If `openrig.sqlite` is reset or replaced, a rerun over an existing analytics database silently drops new OpenRig rows whose seq or id were already used (undercount). After an OpenRig reset or reinstall, use a fresh analytics database.
- (b) Transitions or snapshots deleted by OpenRig retention keep their facts, with no "gone" marker.
- (c) OpenRig 0.6.6 has no update or delete path for `events`. `queue_items` changes in place but is re-read on every run. Re-verify if the OpenRig version changes.
- (d) Use exactly one OpenRig root per analytics database. The CLI accepts a second root, but that root's facts whose seq or id are already used are silently dropped, and its record counts and source keys are not reliable. Multi-root fleets are deferred to Goal 3 #27.
- For OpenRig rows, the trace gives the source revision of the latest ingest, not a per-row producing generation. Claude, Codex and Pi files keep their per-input generation.
