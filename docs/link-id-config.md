# `link-id-v1` Session Link Configuration (`MTN-19 / D3`)

Dendro implements the [`link-id-v1`](contracts/link-id-v1/SPEC.md) contract (pinned digest `07dff9562958ec286c1b096f0eb15c91deddb284c26d99904afeba2b882d0ab5`) so that Dendro and `agent-otel-bridge` (AOB) emit byte-identical `agent.session.link_id` values for the same harness-minted session without persisting or exporting the raw `native_session`.

---

## 1. Security Boundary: `K_link` Only on Endpoints

- **Ingest / Endpoint Path (`dendro_core::SessionLinkConfig`):**
  - Accepts **only** the pre-derived 32-byte endpoint key `K_link` (`LinkKey`) and scope slugs (`org`, `endpoint_scope`, optional `team`, optional `epoch`).
  - **Refuses `K_org`:** If any raw organization master key variable (`DENDRO_LINK_K_ORG_HEX`, `DENDRO_K_ORG_HEX`, `DENDRO_LINK_K_ORG_FILE`, `DENDRO_K_ORG_FILE`, `AGENT_OTEL_LINK_K_ORG_HEX`, `AGENT_OTEL_K_ORG_HEX`, `AGENT_OTEL_LINK_K_ORG_FILE`, `AGENT_OTEL_K_ORG_FILE`) is present on the endpoint environment, `SessionLinkConfig` rejects key material and fails closed to `"unlinked"` (`invalid_key_material`).
- **Admin Provisioning (`dendro_core::link_id::admin`):**
  - HKDF-SHA256 derivation of `K_link` from `K_org` (`AdminOrgMasterKey`) is isolated inside `dendro_core::link_id::admin` for offline key provisioning and contract vector tests only. It is never called on the ingest path.
- **Zeroization & Non-Persistence:**
  - `LinkKey` and `AdminOrgMasterKey` redact `Debug` output and overwrite key bytes with zeros on `Drop` (`fill(0)` + `std::hint::black_box`). Temporary hex strings read from files or environment variables are overwritten before deallocation. `K_link` is never logged or written to DuckDB.

---

## 2. Configuration Sources

`SessionLinkConfig::from_env_with_root` resolves link material from environment variables (with fallback to `observability/consumer.env` and `<private_data_root>/link.key`), supporting both `DENDRO_LINK_*` and `AGENT_OTEL_LINK_*` names so a single host configuration serves both AOB and Dendro:

| Parameter | Primary Variable | Shared AOB Fallback | Format / Rule |
| :--- | :--- | :--- | :--- |
| `K_link` file | `DENDRO_LINK_KEY_FILE` | `AGENT_OTEL_LINK_KEY_FILE` (or `<private_data_root>/link.key`) | Path to file containing 64 hex chars (32 bytes) |
| `K_link` hex | `DENDRO_LINK_KEY_HEX` | `AGENT_OTEL_LINK_KEY_HEX` | 64 hex chars (32 bytes) |
| `org` | `DENDRO_LINK_ORG` | `AGENT_OTEL_LINK_ORG` | Required printable ASCII slug `^[\x21-\x7E]{1,1024}$` |
| `endpoint_scope` | `DENDRO_LINK_ENDPOINT_SCOPE` | `AGENT_OTEL_LINK_ENDPOINT_SCOPE` | Required printable ASCII slug `^[\x21-\x7E]{1,1024}$` |
| `team` | `DENDRO_LINK_TEAM` | `AGENT_OTEL_LINK_TEAM` | Optional printable ASCII slug `^[\x21-\x7E]{1,1024}$` |
| `epoch` | `DENDRO_LINK_EPOCH` | `AGENT_OTEL_LINK_EPOCH` | Optional printable ASCII slug `^[\x21-\x7E]{1,1024}$` |

---

## 3. Fail-Closed Unlinked Behavior

- **Unconfigured Hosts:** When no `K_link` is configured, ingestion succeeds normally and every session is recorded in `session_facts` with `link_id = 'unlinked'` and `unlinked_reason = 'invalid_key_material'`.
- **Short or Malformed Keys:** Any key shorter than 32 bytes (or non-32-byte hex) fails closed to `link_id = 'unlinked'` (`unlinked_reason = 'invalid_key_material'`).
- **Compute Before Seal:** During JSONL ingestion (`crates/dendro-connectors/src/stream.rs`), `link_id` is computed from the **raw** harness-minted `native_session` **before** `seal_session` is called. Synthetic, fallback, or header-less sessions (`codex-file:{source_id}`, `claude-session`, conflicting headers) and non-harness origins (any explicit `origin` other than `"harness"`, including `"Harness"`) resolve to `"unlinked"`.
