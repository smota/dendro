#!/usr/bin/env node
/**
 * Dendro Deterministic Architecture & Security Guardrails Verification
 *
 * Verifies core Rust & security invariants across the repository:
 * 1. Safe Rust (#![deny(unsafe_code)] / #![forbid(unsafe_code)]) across all crate roots,
 *    with no un-audited raw `unsafe {` blocks outside the sole audited Windows FFI boundary.
 * 2. SQL Injection Prevention: no raw string-interpolated SQL value literals
 *    (`format!("SELECT ... WHERE ... = '{}'"` etc.).
 * 3. Loopback & CORS Isolation in `crates/dendro-cli/src/connected_web.rs`:
 *    enforces `127.0.0.1` loopback binding, restricts `Access-Control-Allow-Origin`
 *    to `http://127.0.0.1:`, and forbids wildcard `Access-Control-Allow-Origin: *`.
 * 4. License & Crate Metadata: verifies `LICENSE` (`FSL-1.1-Apache-2.0`), `NOTICE`,
 *    and root `Cargo.toml` `license-file = "LICENSE"`.
 */

import fs from 'node:fs';
import path from 'node:path';

const repoRoot = process.cwd();
const findings = [];

function fail(code, message) {
  findings.push({ code, message });
}

function walkRustFiles(dir, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      walkRustFiles(full, out);
    } else if (entry.isFile() && entry.name.endsWith('.rs')) {
      out.push(full);
    }
  }
  return out;
}

// ---------------------------------------------------------------------------
// 1. Safe Rust (#![deny(unsafe_code)]) Check
// ---------------------------------------------------------------------------
const rootCargoPath = path.join(repoRoot, 'Cargo.toml');
if (!fs.existsSync(rootCargoPath)) {
  fail('CARGO_ROOT_MISSING', 'Root Cargo.toml not found');
} else {
  const rootCargo = fs.readFileSync(rootCargoPath, 'utf8');
  const hasWorkspaceLint =
    rootCargo.includes('[workspace.lints.rust]') &&
    /unsafe_code\s*=\s*"(deny|forbid)"/.test(rootCargo);

  const cratesDir = path.join(repoRoot, 'crates');
  const crateDirs = fs.existsSync(cratesDir)
    ? fs
        .readdirSync(cratesDir, { withFileTypes: true })
        .filter((d) => d.isDirectory())
        .map((d) => path.join(cratesDir, d.name))
    : [];

  if (crateDirs.length === 0) {
    fail('CRATES_MISSING', 'No workspace crates found under crates/');
  }

  for (const crateDir of crateDirs) {
    const crateName = path.basename(crateDir);
    const rootFiles = ['src/lib.rs', 'src/main.rs']
      .map((rel) => path.join(crateDir, rel))
      .filter((p) => fs.existsSync(p));

    if (rootFiles.length === 0) {
      fail('CRATE_ENTRY_MISSING', `Crate ${crateName} has neither src/lib.rs nor src/main.rs`);
      continue;
    }

    for (const rootFile of rootFiles) {
      const content = fs.readFileSync(rootFile, 'utf8');
      const enforcesDeny =
        content.includes('#![deny(unsafe_code)]') ||
        content.includes('#![forbid(unsafe_code)]') ||
        hasWorkspaceLint;
      if (!enforcesDeny) {
        fail(
          'UNSAFE_CODE_LINT_MISSING',
          `${path.relative(repoRoot, rootFile)} does not declare #![deny(unsafe_code)] or #![forbid(unsafe_code)]`,
        );
      }
    }
  }

  // Explicitly audited allowlist for raw `unsafe` blocks:
  // `crates/dendro-windows-private-io/src/ffi.rs` is the sole audited Windows Win32/NT boundary (see AUDIT.md).
  const AUDITED_UNSAFE_ALLOWLIST = new Set([
    'crates/dendro-windows-private-io/src/ffi.rs',
  ]);

  const allRustFiles = walkRustFiles(cratesDir);
  for (const rsFile of allRustFiles) {
    const rel = path.relative(repoRoot, rsFile).replace(/\\/g, '/');
    const content = fs.readFileSync(rsFile, 'utf8');
    if (AUDITED_UNSAFE_ALLOWLIST.has(rel)) {
      if (!content.includes('AUDIT.md') && !content.includes('#![allow(unsafe_code)]')) {
        fail(
          'AUDITED_BOUNDARY_MISSING_HEADER',
          `${rel} is in the unsafe allowlist but is missing its AUDIT.md / #![allow(unsafe_code)] boundary header`,
        );
      }
      continue;
    }

    // Strip line comments and string literals roughly to detect raw `unsafe {` or `unsafe fn`
    const lines = content.split('\n');
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      const trimmed = line.trim();
      if (trimmed.startsWith('//')) continue;
      if (/\bunsafe\s*\{/.test(line) || /\bunsafe\s+fn\b/.test(line)) {
        fail(
          'RAW_UNSAFE_BLOCK',
          `${rel}:${i + 1} contains an un-audited unsafe block or function: ${trimmed}`,
        );
      }
    }
  }
}

// ---------------------------------------------------------------------------
// 2. SQL Injection Prevention Check
// ---------------------------------------------------------------------------
const cratesDir = path.join(repoRoot, 'crates');
for (const rsFile of walkRustFiles(cratesDir)) {
  const rel = path.relative(repoRoot, rsFile).replace(/\\/g, '/');
  const isProdSrc = /^crates\/[^/]+\/src\//.test(rel);
  const content = fs.readFileSync(rsFile, 'utf8');
  const lines = content.split('\n');
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (line.trim().startsWith('//')) continue;
    // Detect SQL string value interpolation via format!("SELECT/INSERT/UPDATE/DELETE ... = '{}'" or VALUES ('{}'))
    // or mutating INSERT/UPDATE/DELETE constructed via format! in production crate sources.
    const sqlFormatMatch =
      /format!\(\s*"(SELECT|INSERT|UPDATE|DELETE)\b[^"]*'\{[^}]*\}'/i.test(line) ||
      (isProdSrc && /format!\(\s*"(INSERT\s+INTO|UPDATE|DELETE\s+FROM)\b/i.test(line));
    if (sqlFormatMatch) {
      fail(
        'SQL_INJECTION_FORMAT',
        `${rel}:${i + 1} constructs a mutating or value-interpolated SQL query via format!: ${line.trim()}`,
      );
    }
  }
}

// ---------------------------------------------------------------------------
// 3. Loopback & CORS Isolation Check
// ---------------------------------------------------------------------------
const connectedWebPath = path.join(repoRoot, 'crates/dendro-cli/src/connected_web.rs');
if (!fs.existsSync(connectedWebPath)) {
  fail('CONNECTED_WEB_MISSING', 'crates/dendro-cli/src/connected_web.rs not found');
} else {
  const cw = fs.readFileSync(connectedWebPath, 'utf8');
  if (!cw.includes('127.0.0.1') || !cw.includes('is_loopback()')) {
    fail(
      'LOOPBACK_BIND_CHECK_MISSING',
      'crates/dendro-cli/src/connected_web.rs must enforce 127.0.0.1 / is_loopback() binding',
    );
  }
  if (!cw.includes('Access-Control-Allow-Origin: http://127.0.0.1:')) {
    fail(
      'CORS_LOOPBACK_RESTRICTION_MISSING',
      'crates/dendro-cli/src/connected_web.rs must restrict Access-Control-Allow-Origin to http://127.0.0.1:',
    );
  }
  if (/Access-Control-Allow-Origin:\s*\*/i.test(cw)) {
    fail(
      'CORS_WILDCARD_FORBIDDEN',
      'crates/dendro-cli/src/connected_web.rs contains forbidden wildcard Access-Control-Allow-Origin: *',
    );
  }
}

// ---------------------------------------------------------------------------
// 4. License & Crate Metadata Check
// ---------------------------------------------------------------------------
const licensePath = path.join(repoRoot, 'LICENSE');
const noticePath = path.join(repoRoot, 'NOTICE');

if (!fs.existsSync(licensePath)) {
  fail('LICENSE_MISSING', 'LICENSE file not found at repository root');
} else {
  const licenseText = fs.readFileSync(licensePath, 'utf8');
  if (!licenseText.includes('FSL-1.1-Apache-2.0') && !licenseText.includes('Functional Source License')) {
    fail('LICENSE_CONTENT_INVALID', 'LICENSE does not contain FSL-1.1-Apache-2.0 / Functional Source License text');
  }
}

if (!fs.existsSync(noticePath)) {
  fail('NOTICE_MISSING', 'NOTICE file not found at repository root');
}

if (fs.existsSync(rootCargoPath)) {
  const rootCargo = fs.readFileSync(rootCargoPath, 'utf8');
  if (!/license-file\s*=\s*"LICENSE"/.test(rootCargo)) {
    fail('CARGO_LICENSE_FILE_MISSING', 'Root Cargo.toml must declare license-file = "LICENSE"');
  }
}

// ---------------------------------------------------------------------------
// Report Verdict
// ---------------------------------------------------------------------------
if (findings.length > 0) {
  console.error('[verify-architecture-guardrails] FAILED');
  for (const f of findings) {
    console.error(`  ERROR ${f.code}: ${f.message}`);
  }
  process.exit(1);
} else {
  console.log('[verify-architecture-guardrails] PASS (Safe Rust, SQL parameterization, loopback/CORS isolation, and FSL-1.1-Apache-2.0 metadata verified)');
  process.exit(0);
}
