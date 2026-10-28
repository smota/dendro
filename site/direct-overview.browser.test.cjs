'use strict';

// Native fixture exporter is built by cargo test --test direct_overview_regressions.
// Set DENDRO_ISSUE101_FIXTURE to that export's fixture.json; never search for an ambient binary.
const test = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs');
const net = require('node:net');
const crypto = require('node:crypto');
const { spawn, spawnSync } = require('node:child_process');

const repo = path.resolve(__dirname, '..');
const fixturePath = process.env.DENDRO_ISSUE101_FIXTURE;
assert.ok(fixturePath, 'Export a populated native fixture and set DENDRO_ISSUE101_FIXTURE=.../fixture.json');
const fixture = JSON.parse(fs.readFileSync(fixturePath, 'utf8'));
assert.equal(fixture.kind, 'issue101-native-populated-fixture');
assert.equal(path.resolve(fixture.site_dir), __dirname, 'receipt must identify this candidate site');
assert.ok(fs.statSync(fixture.database).size > 0, 'native DB is populated, not an empty directory');
const binaryBytes = fs.readFileSync(fixture.binary);
assert.equal(crypto.createHash('sha256').update(binaryBytes).digest('hex'), fixture.binary_sha256, 'binary must match exported build receipt');
const head = spawnSync('git', ['rev-parse', 'HEAD'], { cwd: repo, encoding: 'utf8' });
assert.equal(head.status, 0);
assert.equal(head.stdout.trim(), fixture.source_sha, 're-export after changing candidate HEAD');
const prodClean = spawnSync('git', ['diff', '--quiet', 'HEAD', '--', 'crates/*/src', 'site/dashboards', 'Cargo.toml', 'Cargo.lock'], { cwd: repo });
assert.equal(prodClean.status, 0, 'candidate production inputs must match identified HEAD');
const playwrightModule = process.env.DENDRO_PLAYWRIGHT_MODULE || path.join(repo, '.agentflow/oracles/issue101/browser/node_modules/playwright');
process.env.PLAYWRIGHT_BROWSERS_PATH = process.env.PLAYWRIGHT_BROWSERS_PATH || path.join(repo, '.agentflow/oracles/issue101/browser/.cache/ms-playwright');
const { chromium } = require(playwrightModule);
let child, browser, base, stderr = '';
const windowValue = fixture.from + '..' + fixture.to;

function getFreePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const port = server.address().port;
      server.close(() => resolve(port));
    });
  });
}
async function ready(url) {
  const until = Date.now() + 20000;
  while (Date.now() < until) {
    if (child.exitCode !== null) throw Error('Candidate exited: ' + stderr);
    try { const r = await fetch(url, { signal: AbortSignal.timeout(1000) }); if (r.ok) return; } catch (_) {}
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw Error('Candidate server did not become ready: ' + stderr);
}
function query(view, rig = fixture.rig) {
  return base + '/api/query?' + new URLSearchParams({ view, rig, window: windowValue });
}
async function api(view = 'direct_overview', rig) {
  const r = await fetch(query(view, rig), { signal: AbortSignal.timeout(15000) });
  assert.equal(r.status, 200);
  return r.json();
}
async function pageFor(t, rig = fixture.rig) {
  const context = await browser.newContext({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  t.after(() => context.close());
  const page = await context.newPage();
  await page.goto(base + '/dashboards/index.html?' + new URLSearchParams({ rig, window: windowValue, mode: 'direct' }), { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.DendroState && window.DendroState.get().lastPayload?.direct_overview, null, { timeout: 15000 });
  return page;
}
async function drawer(page, card) {
  await page.locator(card).click();
  const d = page.locator('#direct-overview-drawer');
  await d.waitFor({ state: 'visible' });
  return d;
}
async function settle(page) { await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))); }
function assertScope(url) {
  const u = new URL(url);
  assert.equal(u.origin, base);
  assert.equal(u.searchParams.get('rig'), fixture.rig);
  const combined = u.searchParams.get('window');
  if (combined === windowValue) return;
  assert.equal(u.searchParams.get('from'), fixture.from);
  assert.equal(u.searchParams.get('to'), fixture.to);
}
// Parse a POSIX copy command without evaluating it. Reject shell syntax outside quotes.
function shellWords(command) {
  let word = '', quote = '', escape = false, started = false;
  const words = [];
  for (const c of command) {
    if (escape) { word += c; escape = false; started = true; continue; }
    if (quote === "'") { if (c === "'") quote = ''; else word += c; continue; }
    if (c === '\\') { escape = true; started = true; continue; }
    if (quote === '"') { if (c === '"') quote = ''; else { assert.ok(!'$`'.includes(c), 'no command expansion'); word += c; } continue; }
    if (c === "'" || c === '"') { quote = c; started = true; continue; }
    if (/\s/.test(c)) { if (started) words.push(word); word = ''; started = false; continue; }
    assert.ok(!';|&<>$`()'.includes(c), 'unsafe unquoted shell metacharacter');
    word += c; started = true;
  }
  assert.equal(quote, ''); assert.equal(escape, false);
  if (started) words.push(word);
  return words;
}
function traceArgs(command, digest) {
  const words = shellWords(command);
  assert.equal(words.shift(), 'dendro'); assert.equal(words.shift(), 'trace');
  const required = { '--metric-id': 'direct_overview', '--rig': fixture.rig, '--from': fixture.from, '--to': fixture.to, '--expect-evidence-digest': digest };
  assert.equal(words.length, 10, 'copy command has only the five supported scope/replay flags');
  for (let i = 0; i < words.length; i += 2) { assert.equal(words[i + 1], required[words[i]], 'exact captured flag value for ' + words[i]); delete required[words[i]]; }
  assert.deepEqual(Object.keys(required), []);
  return ['trace', '--db', fixture.database, ...words];
}

test.before(async () => {
  const port = await getFreePort(); base = 'http://127.0.0.1:' + port;
  child = spawn(fixture.binary, ['dashboard', '--db', fixture.database, '--rig', fixture.rig, '--from', fixture.from, '--to', fixture.to, '--serve', '127.0.0.1:' + port, '--site-dir', fixture.site_dir], { stdio: ['ignore', 'pipe', 'pipe'] });
  child.stderr.on('data', d => { stderr += d.toString(); });
  child.stdout.resume();
  await ready(base + '/dashboards/index.html');
  browser = await chromium.launch({ headless: true, timeout: 30000 });
  console.log(JSON.stringify({ candidate: fixture.source_sha, binary_sha256: fixture.binary_sha256, playwright: require(path.join(playwrightModule, 'package.json')).version, chromium: browser.version(), native_fixture: fixture.database }));
});
test.after(async () => { if (browser) await browser.close(); if (child) { child.kill('SIGTERM'); await new Promise(resolve => { if (child.exitCode !== null) resolve(); else { child.once('exit', resolve); setTimeout(() => { child.kill('SIGKILL'); resolve(); }, 2000).unref(); } }); } });

test('B1/B4/B6 populated native API and all four cards show literal values', { timeout: 40000 }, async t => {
  const payload = await api(); const ov = payload.direct_overview;
  assert.equal(ov.total_tokens, 160);
  assert.equal(ov.critical_focus.category, 'Leaf Stall');
  assert.equal(ov.critical_focus.entity_id, 'dev-build@test-rig');
  assert.ok(ov.critical_focus.selected_source_keys.includes('stall:claim'));
  assert.equal(ov.context_tax.uncalled_tools_count, 2);
  assert.equal(ov.context_tax.wasted_schema_tokens, null);
  assert.equal(ov.throughput_and_cost.closed_issue_qitems, 3);
  assert.equal(ov.throughput_and_cost.avg_cost_per_closed_issue_qitem, 10);
  assert.equal(ov.project_groups[0].active_hours, 1);
  assert.equal(ov.project_groups[0].closed_issue_qitems, 3);
  const page = await pageFor(t);
  assert.match(await page.locator('#card-critical-focus').innerText(), /Leaf Stall/);
  assert.match(await page.locator('#card-cwd-projects').innerText(), /fp-a81f/);
  assert.match(await page.locator('#card-cwd-projects').innerText(), /160/);
  assert.match(await page.locator('#card-context-tax').innerText(), /2 Uncalled Tools/i);
  assert.match(await page.locator('#card-throughput-cost').innerText(), /3.*(?:Issue Tasks|Closed Issue)/i);
  assert.match(await page.locator('#card-throughput-cost').innerText(), /\$10\.00/);
  assert.match(await page.locator('#card-throughput-cost').innerText(), /\$30\.00/);
  assert.equal(ov.snapshot, true); assert.equal(ov.live_alert, false);
  assert.match(await page.locator('#direct-overview-fold').innerText(), /snapshot|historical|as.of|stale/i);
});

test('B6 actual geometry and Direct/Forensic retain active controls and URL scope', { timeout: 40000 }, async t => {
  const page = await pageFor(t);
  const geometry = await page.evaluate(() => {
    const r = document.getElementById('direct-overview-fold').getBoundingClientRect();
    return { innerWidth, innerHeight, scrollY, height: document.documentElement.scrollHeight, bottom: r.bottom, foldHeight: r.height, cardCount: document.querySelectorAll('.direct-cards-grid [id^="card-"]').length };
  });
  assert.equal(geometry.innerWidth, 1920); assert.equal(geometry.innerHeight, 1080);
  assert.equal(geometry.scrollY, 0); assert.ok(geometry.height <= 1080);
  assert.ok(geometry.bottom <= 900); assert.ok(geometry.foldHeight <= 900); assert.equal(geometry.cardCount, 4);
  await page.locator('#btn-mode-forensic').click();
  await page.locator('#view-search').fill('handoff');
  await page.locator('.filter-chip[data-category="coordination"]').click();
  await page.locator('#btn-view-mode-layers').click();
  const retained = await page.evaluate(() => ({ search: document.getElementById('view-search').value, category: document.querySelector('.filter-chip.active').dataset.category, layers: document.getElementById('btn-view-mode-layers').classList.contains('active'), rig: new URL(location).searchParams.get('rig'), window: new URL(location).searchParams.get('window') }));
  await page.locator('#btn-mode-direct').click(); await page.locator('#btn-mode-forensic').click();
  const after = await page.evaluate(() => ({ search: document.getElementById('view-search').value, category: document.querySelector('.filter-chip.active').dataset.category, layers: document.getElementById('btn-view-mode-layers').classList.contains('active'), rig: new URL(location).searchParams.get('rig'), window: new URL(location).searchParams.get('window') }));
  assert.deepEqual(after, retained); assert.equal(after.search, 'handoff'); assert.equal(after.category, 'coordination'); assert.equal(after.layers, true); assert.equal(after.rig, fixture.rig); assert.equal(after.window, windowValue);
});

test('B5 every card drawer captures source keys/status and copy replays exact snapshot', { timeout: 60000 }, async t => {
  const page = await pageFor(t); const ov = (await api()).direct_overview;
  for (const [card, source] of [['#card-critical-focus', 'stall:claim'], ['#card-cwd-projects', 'usage:main'], ['#card-context-tax', 'usage:main:skill:0'], ['#card-throughput-cost', 'done:101']]) {
    const d = await drawer(page, card); const rect = await d.boundingBox();
    assert.ok(rect.width >= 480 && rect.width <= 640);
    assert.equal(await page.evaluate(() => document.getElementById('direct-overview-drawer').contains(document.activeElement)), true, 'initial focus belongs inside nonmodal drawer');
    const text = await d.innerText(); assert.ok(text.includes(source), 'drawer must expose actual source key ' + source);
    assert.ok(text.includes(ov.digest), 'captured evidence digest must be visible');
    assert.match(text, /snapshot|historical|as.of|stale/i);
    const shown = await d.locator('.trace-cli-code').innerText();
    const args = traceArgs(shown, ov.digest);
    await d.locator('.btn-copy-trace').click();
    const copied = await page.evaluate(() => navigator.clipboard.readText());
    assert.equal(copied, shown);
    const out = spawnSync(fixture.binary, args, { encoding: 'utf8', timeout: 20000 });
    assert.equal(out.status, 0, out.stderr);
    const replay = JSON.parse(out.stdout).value;
    assert.equal(replay.digest, ov.digest); assert.equal(replay.throughput_and_cost.closed_issue_qitems, 3);
    const bad = args.slice(); bad[bad.indexOf('--expect-evidence-digest') + 1] = '0'.repeat(64);
    const mismatch = spawnSync(fixture.binary, bad, { encoding: 'utf8', timeout: 20000 });
    assert.notEqual(mismatch.status, 0); assert.match(mismatch.stderr, /evidence digest mismatch/);
    await page.keyboard.press('Escape'); await d.waitFor({ state: 'hidden' });
    assert.equal(await page.evaluate(() => document.activeElement.id), card.slice(1));
  }
});

test('B5 predefined lens requests preserve captured exact rig and time scope', { timeout: 40000 }, async t => {
  const page = await pageFor(t);
  for (const view of ['v_seat_idle_ranking', 'skill_facts', 'v_cwd_fingerprint_rollup']) {
    const req = page.waitForRequest(r => new URL(r.url()).searchParams.get('view') === view && new URL(r.url()).pathname === '/api/query');
    await page.locator('.lens-chip[data-lens="' + view + '"]').click();
    assertScope((await req).url());
    await page.locator('#direct-overview-drawer').waitFor({ state: 'visible' });
    await page.keyboard.press('Escape');
  }
});

test('B5 late lens response cannot replace or reopen a newer card drawer', { timeout: 40000 }, async t => {
  const page = await pageFor(t);
  let release, arrived; const gate = new Promise(r => { release = r; }); const seen = new Promise(r => { arrived = r; });
  await page.route('**/api/query?**', async route => {
    if (new URL(route.request().url()).searchParams.get('view') !== 'skill_facts') return route.continue();
    arrived(); await gate; await route.continue();
  });
  await page.locator('.lens-chip[data-lens="skill_facts"]').click(); await seen;
  await drawer(page, '#card-throughput-cost');
  const title = await page.locator('#drawer-title').innerText();
  const response = page.waitForResponse(r => new URL(r.url()).searchParams.get('view') === 'skill_facts');
  release(); await response; await settle(page);
  assert.equal(await page.locator('#drawer-title').innerText(), title, 'obsolete response replaced newer card');
  await page.keyboard.press('Escape');
  await settle(page); assert.equal(await page.locator('#direct-overview-drawer').isVisible(), false);
});

test('B5 fetch failure is visible as failure, with no false empty-success wording', { timeout: 40000 }, async t => {
  const page = await pageFor(t);
  await page.route('**/api/query?**', route => new URL(route.request().url()).searchParams.get('view') === 'skill_facts' ? route.abort('failed') : route.continue());
  await page.locator('.lens-chip[data-lens="skill_facts"]').click();
  const d = page.locator('#direct-overview-drawer'); await d.waitFor({ state: 'visible' });
  const text = await d.innerText(); assert.match(text, /failed|error|unavailable/i); assert.doesNotMatch(text, /No records found/);
});

test('B5 loading is distinct and absent-data lenses are disabled', { timeout: 40000 }, async t => {
  const page = await pageFor(t); let release, arrived;
  const gate = new Promise(r => { release = r; }); const seen = new Promise(r => { arrived = r; });
  await page.route('**/api/query?**', async route => {
    if (new URL(route.request().url()).searchParams.get('view') !== 'skill_facts') return route.continue();
    arrived(); await gate; return route.continue();
  });
  t.after(() => release());
  await page.locator('.lens-chip[data-lens="skill_facts"]').click(); await seen;
  assert.match(await page.locator('#direct-overview-drawer').innerText(), /loading/i);
  release();
  const other = await pageFor(t, 'other-rig');
  const absent = other.locator('.lens-chip[data-lens="skill_facts"]');
  assert.equal(await absent.isDisabled(), true, 'foreign population cannot enable a no-data skill lens');
});

test('B6 scope switch loads other rig values without leaking first rig sources', { timeout: 40000 }, async t => {
  const page = await pageFor(t); const payload = await api('direct_overview', 'other-rig');
  assert.equal(payload.direct_overview.total_tokens, 9999);
  assert.equal(payload.direct_overview.throughput_and_cost.closed_issue_qitems, 0);
  assert.equal(payload.direct_overview.context_tax.uncalled_tools_count, null);
  await page.goto(base + '/dashboards/index.html?' + new URLSearchParams({ rig: 'other-rig', window: windowValue, mode: 'direct' }));
  await page.waitForFunction(() => window.DendroState?.get().lastPayload?.direct_overview?.total_tokens === 9999);
  assert.match(await page.locator('#card-cwd-projects').innerText(), /9,?999/);
  await drawer(page, '#card-throughput-cost');
  assert.doesNotMatch(await page.locator('#direct-overview-drawer').innerText(), /done:101|usage:main/);
});
