# `link-id-v1` Session Link Configuration (`MTN-19 / D3`)

Dendro implements the [`link-id-v1`](contracts/link-id-v1/SPEC.md) contract (pinned digest `07dff9562958ec286c1b096f0eb15c91deddb284c26d99904afeba2b882d0ab5`) so that Dendro and `agent-otel-bridge` (AOB) emit byte-identical `agent.session.link_id` values for the same harness-minted session without persisting or exporting the raw `native_session`.

---

## 1. Security Boundary: `K_link` Only on Endpoints

- **Ingest / Endpoint Path (`dendro_core::SessionLinkConfig`):**
  - Accepts **only** the pre-derived 32-byte endpoint key `K_link` (`LinkKey`) and scope slugs (`org`, `endpoint_scope`, optional `team`, optional `epoch`).
  - **Refuses `K_org`:** If any documented raw organization master key variable (`DENDRO_LINK_K_ORG_HEX`, `DENDRO_K_ORG_HEX`, `DENDRO_LINK_K_ORG_FILE`, `DENDRO_K_ORG_FILE`, `DENDRO_LINK_K_ORG`, `DENDRO_K_ORG`, `AGENT_OTEL_LINK_K_ORG_HEX`, `AGENT_OTEL_K_ORG_HEX`, `AGENT_OTEL_LINK_K_ORG_FILE`, `AGENT_OTEL_K_ORG_FILE`, `AGENT_OTEL_LINK_K_ORG`, `AGENT_OTEL_K_ORG`, `K_ORG`, `K_ORG_HEX`, `K_ORG_FILE`, including `export <VAR>=...` syntax in `consumer.env` or key files) is present anywhere in the process environment, `consumer.env`, or key file, `SessionLinkConfig` refuses link configuration, emits a one-time redacted warning on `stderr` containing no key material, and fails closed to `"unlinked"` (`invalid_key_material`).
- **Admin Provisioning (`dendro_core::link_id::admin`):**
  - HKDF-SHA256 derivation of `K_link` from `K_org` (`AdminOrgMasterKey`) is isolated inside `dendro_core::link_id::admin` for offline key provisioning and contract vector tests only. It is never called on the ingest path.
- **Zeroization, No-Clone & Non-Persistence:**
  - `LinkKey` and `AdminOrgMasterKey` do not implement `Clone`, redact `Debug` output, use constant-time byte-fold equality (`LinkKey`), and overwrite key bytes with zeros on `Drop` (`fill(0)` + `std::hint::black_box`). Temporary hex strings read from files or environment variables are overwritten before deallocation. `K_link` is never logged or written to DuckDB.

---

## 2. Configuration Sources & Precedence

`SessionLinkConfig::from_env` never reads link configuration relative to the current working directory (`CWD` isolation holds by construction: `std::env::current_dir()`, relative `XDG_CONFIG_HOME`/`HOME` values, and relative `DENDRO_LINK_KEY_FILE` paths are never consulted). For link settings, `dendro_core::read_consumer_env` reads `$XDG_CONFIG_HOME/observability/consumer.env` or `$HOME/.config/observability/consumer.env` (capped at `16 KiB`) only when the root is an absolute path and `consumer.env` is a non-symlink regular file; **a symlinked `consumer.env` is ignored for link settings** (whereas `dendro-cli` telemetry retains its dotfile-friendly loader and continues to read a symlinked `consumer.env` for OTLP telemetry settings). Dendro's private data root is resolved via `dendro_core::default_private_data_root` (`$XDG_DATA_HOME/dendro` or `$HOME/.local/share/dendro` on Unix, `%LOCALAPPDATA%\Dendro` on Windows).

### Key Material Precedence (Highest First)

1. **Process Environment (`DENDRO_LINK_KEY_HEX` / `AGENT_OTEL_LINK_KEY_HEX` or `DENDRO_LINK_KEY_FILE` / `AGENT_OTEL_LINK_KEY_FILE`):**
   - `DENDRO_LINK_KEY_HEX` (or `AGENT_OTEL_LINK_KEY_HEX`) in the process environment takes highest priority.
   - `DENDRO_LINK_KEY_FILE` (or `AGENT_OTEL_LINK_KEY_FILE`) is accepted **only** from the process environment (never from `consumer.env`) and **must be an absolute path** (`path.is_absolute()`). Relative key paths are refused.
   - On Windows, if the parent directory of `DENDRO_LINK_KEY_FILE` is a `platform_v1` `PrivateRoot` (`SE_DACL_PROTECTED` with exact current-user SID + `SYSTEM` + `BA` ACL), `platform_v1::open_existing_private_file` enforces the full private-root and private-file checks; when the parent is an arbitrary absolute directory without `SE_DACL_PROTECTED`, `load_link_key_file` enforces `platform_v1` handle-based non-reparse (`FILE_FLAG_OPEN_REPARSE_POINT`), regular-file, `nNumberOfLinks == 1`, and 128-bit `(VolumeSerialNumber, FileId)` identity checks (while parent-directory `SE_DACL_PROTECTED` and per-file SDDL ACL checks are not enforced outside a `PrivateRoot`).
2. **Private Key File in Dendro's Private Data Root (`<private_data_root>/link.key`):**
   - Loaded only from Dendro's verified private data root (never from the output database's parent directory or `CWD`) via `platform_v1::open_existing_private_root` + `platform_v1::open_existing_private_file(root, "link.key")`.
   - On Unix, `open_existing_private_file` opens the key file with `O_NOFOLLOW | O_CLOEXEC` and verifies it is a regular file owned by the current effective UID with no group or world permissions (`mode & 0o077 == 0`), `nlink == 1`, and matching path/fd `(dev, ino)`.
   - On Windows, `open_existing_private_file` opens `<private_data_root>/link.key` inside the `SE_DACL_PROTECTED` `%LOCALAPPDATA%\Dendro` `PrivateRoot` with `FILE_FLAG_OPEN_REPARSE_POINT`, verifying handle identity, non-reparse regular-file attributes, `nNumberOfLinks == 1`, and restricted per-user ACL (`current user SID` + `SYSTEM` + `BA`, denying `WD`/`AU`/`BU`/`AN`/`BG`).
3. **`$XDG_CONFIG_HOME/observability/consumer.env` (`DENDRO_LINK_KEY_HEX` / `AGENT_OTEL_LINK_KEY_HEX`):**
   - Fallback when neither process environment key variables nor `<private_data_root>/link.key` are present. `DENDRO_LINK_KEY_FILE` / `AGENT_OTEL_LINK_KEY_FILE` entries inside `consumer.env` are ignored, and a symlinked `consumer.env` is ignored for link settings.

| Parameter | Primary Variable | Shared AOB Fallback | Format / Rule |
| :--- | :--- | :--- | :--- |
| `K_link` hex | `DENDRO_LINK_KEY_HEX` | `AGENT_OTEL_LINK_KEY_HEX` | 64 hex chars (32 bytes); process env > `<private_data_root>/link.key` > `consumer.env` |
| `K_link` file | `DENDRO_LINK_KEY_FILE` | `AGENT_OTEL_LINK_KEY_FILE` | Process-env only; absolute path to regular file (`0600` on Unix) containing 64 hex chars |
| `org` | `DENDRO_LINK_ORG` | `AGENT_OTEL_LINK_ORG` | Required printable ASCII slug `^[\x21-\x7E]{1,1024}$` (process env > `consumer.env`) |
| `endpoint_scope` | `DENDRO_LINK_ENDPOINT_SCOPE` | `AGENT_OTEL_LINK_ENDPOINT_SCOPE` | Required printable ASCII slug `^[\x21-\x7E]{1,1024}$` (process env > `consumer.env`) |
| `team` | `DENDRO_LINK_TEAM` | `AGENT_OTEL_LINK_TEAM` | Optional printable ASCII slug `^[\x21-\x7E]{1,1024}$` (process env > `consumer.env`) |
| `epoch` | `DENDRO_LINK_EPOCH` | `AGENT_OTEL_LINK_EPOCH` | Optional printable ASCII slug `^[\x21-\x7E]{1,1024}$` (process env > `consumer.env`) |

---

## 3. Fail-Closed Unlinked Behavior & Pre-D3 Databases

- **Unconfigured Hosts:** When no `K_link` is configured, ingestion succeeds normally and every session is recorded in `session_facts` with `link_id = 'unlinked'` and `unlinked_reason = 'invalid_key_material'`.
- **Short or Malformed Keys:** Any key shorter than 32 bytes (or non-32-byte hex) fails closed to `link_id = 'unlinked'` (`unlinked_reason = 'invalid_key_material'`).
- **Compute Before Seal, Parser Provenance & Re-Link Eligibility:** During JSONL ingestion (`crates/dendro-connectors/src/stream.rs`), `link_id` is computed from the **raw** harness-minted `native_session` **before** `seal_session` is called. `origin` is assigned by Dendro's parser (`"harness"` for native harness-log parsers; non-harness for export or non-native paths), never probed from transcript content. Sessions where the parser synthesizes a fallback identifier (header-less files, per-file fallbacks, `claude-session` when `sessionId` is absent, or conflicting headers across generations) resolve to `"unlinked"`. **Only a real header conflict (`session_facts.header_conflict = TRUE`, keyed by `source_id`) permanently locks a source to `"unlinked"` across generations.** Sessions unlinked for configuration or input reasons (`invalid_key_material`, `missing_org`, `missing_endpoint_scope`, missing/invalid `team` or `epoch`) and fallback `{harness}-file:` / `claude-session` rows are recomputed on the next ingest, and a fallback row created before a real session header appeared (zero-byte or partial line-1 first pass) is retired when the real header session is ingested.
- **Pre-D3 Databases & Downstream Export (`D4b`):** `session_facts` is new in D3. Opening a pre-D3 database creates `session_facts` without modifying existing tables; pre-D3 sessions have no row in `session_facts` (`NULL` on `LEFT JOIN` means *"not computed; full rebuild computes it"*). Downstream export (`D4b`) **must** treat a missing `session_facts` row or `NULL` link as `"unlinked"`.

---

## 4. `fact-export-v1` Pseudonym Key Configuration (`MTN-21 / D4c` & `D4b`)

`dendro_core::FactExportConfig` (`crates/dendro-core/src/git_evidence.rs`) reuses D3's generic secret-key loader in `crates/dendro-core/src/link_id.rs` (`SecretKeyLoaderSpec` + `load_secret_key_from_sources`, parameterized by variable names, key file name, and domain) to load customer-held 32-byte key material (`FACT_EXPORT_KEY_MIN_BYTES = 32`) and `pseudonym_key_id` for `fact-export-v1` HMAC-SHA256 pseudonyms (`fact-export.v1/source`, `fact-export.v1/session`, `fact-export.v1/commit`, `fact-export.v1/git_evidence`):

1. **Shared D3/D4c Loader, Skip-Empty Fallback & Precedence (Highest First):**
   - **Skip-Empty Semantics (`R6` / `M1`):** Across both `SessionLinkConfig` and `FactExportConfig`, every variable lookup (`pick_var` / `pick_var_with_fallback`) skips empty values (`""`) so an empty primary variable (e.g. `DENDRO_LINK_KEY_HEX=""` or `DENDRO_FACT_EXPORT_KEY_HEX=""`) never blocks a valid fallback variable (`AGENT_OTEL_*`) or `consumer.env` entry. `strip_export_prefix` checks UTF-8 char boundaries (`trimmed.get(..6)`) before stripping `export` prefixes (`R6` / `M2`).
   - **Process Environment:** `DENDRO_FACT_EXPORT_KEY_HEX` (or `AGENT_OTEL_FACT_EXPORT_KEY_HEX`), or process-env-only `DENDRO_FACT_EXPORT_KEY_FILE` (or `AGENT_OTEL_FACT_EXPORT_KEY_FILE`, must be an absolute path to a verified private regular file), plus `DENDRO_FACT_EXPORT_KEY_ID` (or `AGENT_OTEL_FACT_EXPORT_KEY_ID`).
   - **Private Key File in Dendro's Private Data Root Only:** `<private_data_root>/fact_export.key` resolved via `dendro_core::default_private_data_root()` and opened via `platform_v1::open_existing_private_root` + `platform_v1::open_existing_private_file(root, "fact_export.key")` (`0600` on Unix / `SE_DACL_PROTECTED` on Windows). Never loaded from the database's parent directory or `CWD`, and never falls back to `USERPROFILE`.
   - **Lenient `$XDG_CONFIG_HOME/observability/consumer.env` (or `$HOME/.config/observability/consumer.env`):** Loaded via D3's `read_consumer_env_from` (`MAX_CONSUMER_ENV_BYTES = 16384` non-symlink regular file; an unusable or symlinked `consumer.env` is ignored, never fatal). `DENDRO_FACT_EXPORT_KEY_FILE` / `AGENT_OTEL_FACT_EXPORT_KEY_FILE` entries inside `consumer.env` are ignored.
2. **Separate Keys, Case-Insensitive `K_org` Refusal, `FactExportKeyState`, No-Clone & Constant-Time Equality:**
   - The fact-export key and `K_link` are strictly separate (different environment variables, key files `fact_export.key` vs `link.key`, and HMAC domains).
   - `FactExportConfig::state()` returns `dendro_core::FactExportKeyState`:
     - `FactExportKeyState::Ready`: Valid $\ge 32$-byte key (`FACT_EXPORT_KEY_MIN_BYTES = 32`) and valid `pseudonym_key_id` (`^[A-Za-z0-9._:-]{1,64}$`).
     - `FactExportKeyState::Unconfigured`: No key material configured in any source.
     - `FactExportKeyState::InvalidKeyMaterial`: Any forbidden organization master key variable (`DENDRO_ORG_KEY*`, `AGENT_OTEL_ORG_KEY*`, `DENDRO_K_ORG*`, `DENDRO_LINK_K_ORG*`, `DENDRO_FACT_EXPORT_K_ORG*`, `AGENT_OTEL_*_K_ORG*`, `K_ORG*`, matched case-insensitively with any whitespace after `export`), malformed hex, key shorter than 32 bytes (enforced across `FactExportKey::from_hex`, `FactExportKey::from_slice`, `FactExportKey::from_raw_key_bytes`, and `FactExportConfig::from_raw_key`), or failed `platform_v1` private-file check.
     - `FactExportKeyState::InvalidKeyId`: Valid key material was provided, but `pseudonym_key_id` is missing or fails `is_valid_key_id` (`^[A-Za-z0-9._:-]{1,64}$`).
   - `FactExportKey` and `FactExportConfig` do not implement `Clone`; `FactExportKey` implements constant-time byte-fold equality (`PartialEq`/`Eq`), redacts `Debug` output, and overwrites key bytes with zeros on `Drop` (`fill(0)` + `std::hint::black_box`).
3. **Local-Only `D4c` Persistence & Downstream `D4b` Handoff:**
   - When `FactExportConfig::state()` is not `FactExportKeyState::Ready` (`Unconfigured`, `InvalidKeyMaterial`, or `InvalidKeyId`), `git_evidence_facts` rows are persisted with their locally sealed commit identity (`commit_sealed`, `parent_sealed`) and `export_status = 'non_exportable'` (`pseudonym_key_id = NULL`, `commit_hmac = NULL`, `parent_hmacs = '[]'`). Once valid key material and `pseudonym_key_id` are configured on a later ingest pass, `D4a` re-reads the source from byte `0` and upgrades rows in place to `export_status = 'exportable'` with `pseudonym_key_id`, `commit_hmac`, and `parent_hmacs`.
   - `D4c` does **not** persist contract `source_id`, `session_key`, or `git_evidence_key`. Downstream `D4b` (`fact-export-v1` exporter) reuses `dendro_core::FactExportConfig`, reads cross-source resolved rows via `AnalyticsStore::load_git_evidence()`, exports only `export_status = 'exportable'` rows whose `pseudonym_key_id` matches the active key (`R5`), maps local `'not_observed'` statuses (`merge_status`, `default_branch_status`) to the wire enum `'unknown'`, and computes `source_id` (`derive_export_source_id`), `session_key` (`derive_export_session_key`), and `git_evidence_key` (`derive_export_git_evidence_key`) at export time from a provisioned logical source ID.

| Parameter | Primary Variable | Shared AOB Fallback | Format / Rule |
| :--- | :--- | :--- | :--- |
| Export key hex | `DENDRO_FACT_EXPORT_KEY_HEX` | `AGENT_OTEL_FACT_EXPORT_KEY_HEX` | 64 hex chars (32 bytes); process env > `<private_data_root>/fact_export.key` > `consumer.env` |
| Export key file | `DENDRO_FACT_EXPORT_KEY_FILE` | `AGENT_OTEL_FACT_EXPORT_KEY_FILE` | Process-env only; absolute path to private regular file containing 64 hex chars |
| `pseudonym_key_id` | `DENDRO_FACT_EXPORT_KEY_ID` | `AGENT_OTEL_FACT_EXPORT_KEY_ID` | Required when key is present; ASCII identifier `^[A-Za-z0-9._:-]{1,64}$` (process env > `consumer.env`) |
