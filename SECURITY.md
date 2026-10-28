# Security Policy

Dendro is a **local-first AI Fleet Intelligence engine** engineered with strict local sovereignty, cryptographic integrity, and zero external telemetry.

---

## Supported Versions

We actively maintain and provide security updates for the current minor release series:

| Version | Supported | Notes |
| :--- | :--- | :--- |
| `0.9.x` | Yes | Active Schema 5 + 25-Insight Reference Catalog + Connected HTTP/SSE Studio |
| `< 0.9.0` | No | Historical releases (`0.5.x` – `0.8.x`); upgrade to `0.9.x` via `dendro migrate` |

---

## Local-First Security Architecture

Dendro enforces five non-negotiable architectural security invariants across all workspace crates (`crates/*`) and web surfaces (`site/*`), verified on every commit via `node scripts/verify-architecture-guardrails.mjs` and `node scripts/verify-design-guardrails.mjs`:

1. **Loopback-Only Binding & Strict CORS Isolation (`127.0.0.1`):**
   - The Connected HTTP & Server-Sent Events (SSE) server (`dendro dashboard --serve` / `dendro serve`) validates socket addresses via `validate_dashboard_bind_address` and refuses any non-loopback or wildcard interface (`0.0.0.0`).
   - HTTP responses restrict `Access-Control-Allow-Origin` strictly to loopback (`http://127.0.0.1:4242`) and never emit wildcard `Access-Control-Allow-Origin: *`.
   - Static asset resolution in `crates/dendro-cli/src/connected_web.rs` canonicalizes paths and rejects any `..` traversal or symlink escape outside `site/`.

2. **Safe Rust Policy (`#![deny(unsafe_code)]`):**
   - Every workspace crate (`dendro-core`, `dendro-platform-policy`, `dendro-connectors`, `dendro-resolver`, `dendro-watcher`, `dendro-cli`, `dendro-windows-private-io`) enforces `#![deny(unsafe_code)]` or `#![forbid(unsafe_code)]`.
   - The sole audited OS boundary (`crates/dendro-windows-private-io/src/ffi.rs`) is isolated behind `#![cfg(windows)]` and documented in [`crates/dendro-windows-private-io/AUDIT.md`](crates/dendro-windows-private-io/AUDIT.md) to enforce Win32 DACL owner-only file descriptors and reparse-point refusal.

3. **Mandatory Parameterized SQL Queries:**
   - All DuckDB (`dendro.duckdb`) and SQLite queries use parameterized bindings (`duckdb::params![...]`, `rusqlite::params![...]`).
   - Constructing SQL value predicates via string interpolation (`format!("SELECT ... WHERE id = '{}'", input)`) is prohibited and blocked by CI guardrails.

4. **Automatic Secret & Credential Redaction:**
   - Session transcripts, shell commands, and environment snapshots are scrubbed before persistence.
   - Known credential prefixes (`sk-`, `ghp_`, `gho_`, `xai-`, `Bearer` tokens, and sensitive environment variables) are deterministically redacted, and sensitive auth configuration files (`~/.claude/settings.json`, `~/.codex/auth.json`) are refused at admission.

5. **Zero External CDNs & Offline Sovereignty:**
   - Every HTML surface (`site/index.html`, `site/dashboards/index.html`, `site/dashboards/views/*.html`, and `dendro report --format html` exports) operates 100% offline with zero external `<script>`, `<link>`, `@import`, or third-party font/CDN requests.

---

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues, discussions, or pull requests.**

Instead, please report vulnerabilities privately through **GitHub Private Vulnerability Reporting**:

- **Report URL:** [https://github.com/smota/dendro/security/advisories/new](https://github.com/smota/dendro/security/advisories/new)

### What to Include
- Affected Dendro version (`dendro --version`) and operating system (Linux / Windows / WSL2).
- The component or crate involved (e.g., `dendro-core`, `dendro-connectors`, `dendro-cli`, `site/dashboards`).
- A minimal, reproducible proof-of-concept or step-by-step reproduction guide.
- Impact assessment (e.g., path traversal, secret redaction bypass, non-loopback exposure, privilege escalation).

### Response SLA
- **Initial Acknowledgment:** Within 48 hours of submission.
- **Triage & Remediation Plan:** Within 5 business days.
- **Coordinated Disclosure:** Security fixes are promoted via `hotfix/* -> main` and published with a coordinated GitHub Security Advisory once a patch release is tagged.
