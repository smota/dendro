#!/usr/bin/env node
/**
 * Dendro Deterministic UI/UX & Offline Design System Guardrails Verification
 *
 * Verifies UI/UX and offline sovereignty invariants across `site/` and `crates/dendro-core`:
 * 1. Zero External CDN / Offline Sovereignty Check across `site/index.html`,
 *    `site/dashboards/index.html`, and all `site/dashboards/views/*.html`.
 * 2. Shared Obsidian Design System & Live-Sync Check (`site/dashboards/style.css`
 *    and `site/dashboards/live-sync.js` exist and are referenced by all 12 console views).
 * 3. 4-Layer Capability Taxonomy & Demo Fixture Check (`site/fixtures/fleet-explorer-demo.json`
 *    parses as valid JSON, and `crates/dendro-core/src/presentation.rs` defines the 4-layer
 *    capability taxonomy).
 */

import fs from 'node:fs';
import path from 'node:path';

const repoRoot = process.cwd();
const findings = [];

function fail(code, message) {
  findings.push({ code, message });
}

// ---------------------------------------------------------------------------
// 1. Zero External CDN / Offline Sovereignty Check
// ---------------------------------------------------------------------------
const viewsDir = path.join(repoRoot, 'site/dashboards/views');
const viewFiles = fs.existsSync(viewsDir)
  ? fs
      .readdirSync(viewsDir)
      .filter((f) => f.endsWith('.html'))
      .sort()
      .map((f) => path.join(viewsDir, f))
  : [];

const htmlFilesToScan = [
  path.join(repoRoot, 'site/index.html'),
  path.join(repoRoot, 'site/dashboards/index.html'),
  ...viewFiles,
];

for (const htmlPath of htmlFilesToScan) {
  const rel = path.relative(repoRoot, htmlPath).replace(/\\/g, '/');
  if (!fs.existsSync(htmlPath)) {
    fail('HTML_FILE_MISSING', `Expected HTML file missing: ${rel}`);
    continue;
  }
  const content = fs.readFileSync(htmlPath, 'utf8');

  // Check for remote <script src="http...">
  const scriptMatches = content.match(/<script\b[^>]*\bsrc\s*=\s*["']https?:\/\/[^"']+["'][^>]*>/gi) || [];
  for (const m of scriptMatches) {
    fail('EXTERNAL_SCRIPT_CDN', `${rel} loads external script over HTTP(S): ${m}`);
  }

  // Check for remote <link ... href="http..."> (ignoring rel="canonical")
  const linkMatches = content.match(/<link\b[^>]*\bhref\s*=\s*["']https?:\/\/[^"']+["'][^>]*>/gi) || [];
  for (const m of linkMatches) {
    if (/rel\s*=\s*["']canonical["']/i.test(m)) continue;
    fail('EXTERNAL_LINK_CDN', `${rel} loads external stylesheet/font/asset over HTTP(S): ${m}`);
  }

  // Check for remote CSS @import url("http...")
  const importMatches = content.match(/@import\s+(?:url\s*\(\s*)?["']?https?:\/\/[^"')]+["']?\s*\)?/gi) || [];
  for (const m of importMatches) {
    fail('EXTERNAL_CSS_IMPORT', `${rel} imports external CSS over HTTP(S): ${m}`);
  }

  // Check for Google Fonts or common third-party CDN hosts in runtime asset tags
  if (/fonts\.googleapis\.com|fonts\.gstatic\.com|cdn\.jsdelivr\.net|unpkg\.com|cdnjs\.cloudflare\.com/i.test(content)) {
    fail('EXTERNAL_CDN_HOST', `${rel} references an external CDN or Google Fonts domain`);
  }
}

// ---------------------------------------------------------------------------
// 2. Shared Obsidian Design System & Live-Sync Check
// ---------------------------------------------------------------------------
const styleCssPath = path.join(repoRoot, 'site/dashboards/style.css');
const liveSyncJsPath = path.join(repoRoot, 'site/dashboards/live-sync.js');

if (!fs.existsSync(styleCssPath)) {
  fail('STYLE_CSS_MISSING', 'site/dashboards/style.css does not exist');
}
if (!fs.existsSync(liveSyncJsPath)) {
  fail('LIVE_SYNC_JS_MISSING', 'site/dashboards/live-sync.js does not exist');
}

if (viewFiles.length !== 12) {
  fail(
    'CONSOLE_VIEW_COUNT_MISMATCH',
    `Expected 12 console views in site/dashboards/views/*.html, found ${viewFiles.length}`,
  );
}

for (const viewPath of viewFiles) {
  const rel = path.relative(repoRoot, viewPath).replace(/\\/g, '/');
  const content = fs.readFileSync(viewPath, 'utf8');
  if (!content.includes('style.css')) {
    fail('VIEW_MISSING_STYLE_CSS', `${rel} does not reference shared style.css`);
  }
  if (!content.includes('live-sync.js')) {
    fail('VIEW_MISSING_LIVE_SYNC_JS', `${rel} does not reference shared live-sync.js`);
  }
}

// ---------------------------------------------------------------------------
// 3. 4-Layer Capability Taxonomy & Demo Fixture Check
// ---------------------------------------------------------------------------
const demoFixturePath = path.join(repoRoot, 'site/fixtures/fleet-explorer-demo.json');
if (!fs.existsSync(demoFixturePath)) {
  fail('DEMO_FIXTURE_MISSING', 'site/fixtures/fleet-explorer-demo.json does not exist');
} else {
  try {
    const parsed = JSON.parse(fs.readFileSync(demoFixturePath, 'utf8'));
    if (!parsed || typeof parsed !== 'object' || !parsed.payload) {
      fail('DEMO_FIXTURE_SCHEMA', 'site/fixtures/fleet-explorer-demo.json is missing top-level payload object');
    }
  } catch (err) {
    fail('DEMO_FIXTURE_INVALID_JSON', `site/fixtures/fleet-explorer-demo.json failed to parse: ${err.message}`);
  }
}

const presentationPath = path.join(repoRoot, 'crates/dendro-core/src/presentation.rs');
if (!fs.existsSync(presentationPath)) {
  fail('PRESENTATION_RS_MISSING', 'crates/dendro-core/src/presentation.rs does not exist');
} else {
  const pres = fs.readFileSync(presentationPath, 'utf8');
  // Capability taxonomy layers in presentation.rs (either canonical domain layers or capability layer slugs)
  const canonicalFourLayers = ['model', 'harness', 'coordination', 'environment'];
  const capabilityLayerSlugs = [
    'fleet_topology',
    'execution_mechanics',
    'context_economics',
    'governance_risk',
  ];

  const hasCanonicalLayers = canonicalFourLayers.every((layer) => pres.includes(`"${layer}"`));
  const hasCapabilitySlugs = capabilityLayerSlugs.every((slug) => pres.includes(slug));

  if (!hasCanonicalLayers && !hasCapabilitySlugs) {
    fail(
      'CAPABILITY_TAXONOMY_MISSING',
      'crates/dendro-core/src/presentation.rs does not define all 4 capability taxonomy layers',
    );
  }
}

// ---------------------------------------------------------------------------
// Report Verdict
// ---------------------------------------------------------------------------
if (findings.length > 0) {
  console.error('[verify-design-guardrails] FAILED');
  for (const f of findings) {
    console.error(`  ERROR ${f.code}: ${f.message}`);
  }
  process.exit(1);
} else {
  console.log(
    `[verify-design-guardrails] PASS (Zero external CDNs across ${htmlFilesToScan.length} HTML surfaces, 12/12 views linked to style.css & live-sync.js, demo fixture & 4-layer taxonomy verified)`,
  );
  process.exit(0);
}
