'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs');
const net = require('node:net');
const { spawn } = require('node:child_process');

process.env.PLAYWRIGHT_BROWSERS_PATH = path.resolve(
  __dirname,
  '../.agentflow/oracles/issue101/browser/.cache/ms-playwright'
);

const { chromium } = require(
  path.resolve(__dirname, '../.agentflow/oracles/issue101/browser/node_modules/playwright')
);

function getFreePort() {
  return new Promise((resolve, reject) => {
    const s = net.createServer();
    s.listen(0, '127.0.0.1', () => {
      const port = s.address().port;
      s.close(() => resolve(port));
    });
    s.on('error', reject);
  });
}

function waitForServer(url, timeoutMs = 15000) {
  const start = Date.now();
  return new Promise((resolve, reject) => {
    function check() {
      fetch(url)
        .then((res) => {
          if (res.ok) resolve();
          else setTimeout(check, 100);
        })
        .catch(() => {
          if (Date.now() - start > timeoutMs) {
            reject(new Error(`Server at ${url} failed to become ready within ${timeoutMs}ms`));
          } else {
            setTimeout(check, 100);
          }
        });
    }
    check();
  });
}

test('Direct Overview Browser Journey & Geometry SLA (<900px top-fold on 1080p)', async (t) => {
  const port = await getFreePort();
  const testId = `browser-${Date.now()}-${Math.floor(Math.random() * 10000)}`;
  const rootDir = `/home/sam/code/dendro/target/${testId}`;

  fs.mkdirSync(rootDir, { mode: 0o700, recursive: true });

  const binPath = [
    process.env.CARGO_TARGET_DIR ? path.join(process.env.CARGO_TARGET_DIR, 'debug/dendro') : null,
    path.resolve(__dirname, '../target/debug/dendro'),
    path.resolve(__dirname, '../../target/debug/dendro'),
    '/home/sam/code/dendro/target/debug/dendro'
  ].find((p) => p && fs.existsSync(p));
  assert.ok(binPath, `Dendro binary must exist in target/debug`);

  // Launch dashboard server child process
  const child = spawn(
    binPath,
    [
      'dashboard',
      '--storage-root',
      rootDir,
      '--rig',
      'test-rig',
      '--from',
      '2026-10-01T00:00:00Z',
      '--to',
      '2026-10-02T00:00:00Z',
      '--serve',
      `127.0.0.1:${port}`,
      '--site-dir',
      path.resolve(__dirname, '../site')
    ],
    {
      stdio: ['ignore', 'pipe', 'pipe']
    }
  );

  let childError = '';
  child.stderr.on('data', (d) => {
    childError += d.toString();
  });

  try {
    const dashboardUrl = `http://127.0.0.1:${port}/dashboards/index.html`;
    await waitForServer(dashboardUrl, 15000);

    const browser = await chromium.launch({ headless: true, timeout: 30000 });
    try {
      const page = await browser.newPage({
        viewport: { width: 1920, height: 1080 },
        deviceScaleFactor: 1
      });

      page.on('console', (msg) => console.log('PAGE LOG:', msg.text()));
      page.on('pageerror', (err) => console.log('PAGE ERROR:', err));

      await page.goto(dashboardUrl, { waitUntil: 'domcontentloaded' });
      await page.waitForSelector('#direct-overview-fold', { timeout: 10000 });

      // 1. Geometry verification (< 900px top-fold and no initial window scroll)
      const geom = await page.evaluate(() => {
        const fold = document.getElementById('direct-overview-fold');
        const rect = fold.getBoundingClientRect();
        return {
          scrollY: window.scrollY,
          innerHeight: window.innerHeight,
          scrollHeight: document.documentElement.scrollHeight,
          foldBottom: rect.bottom,
          isModeDirect: document.body.classList.contains('mode-direct')
        };
      });

      assert.strictEqual(geom.isModeDirect, true, 'Default mode must be Modo Direto');
      assert.strictEqual(geom.scrollY, 0, 'Initial scrollY must be 0');
      assert.ok(
        geom.scrollHeight <= 1080,
        `Document scrollHeight (${geom.scrollHeight}) must be <= 1080 to prevent window scrollbar`
      );
      assert.ok(
        geom.foldBottom <= 900,
        `Direct overview top-fold bottom (${geom.foldBottom}px) must be <= 900px on 1080p`
      );

      // 2. Decision cards existence & initial content
      const cards = await page.evaluate(() => {
        return {
          focus: !!document.getElementById('card-critical-focus'),
          projects: !!document.getElementById('card-cwd-projects'),
          tax: !!document.getElementById('card-context-tax'),
          throughput: !!document.getElementById('card-throughput-cost')
        };
      });
      assert.strictEqual(cards.focus, true, 'Card 1 (Critical Focus) must exist');
      assert.strictEqual(cards.projects, true, 'Card 2 (CWD Projects) must exist');
      assert.strictEqual(cards.tax, true, 'Card 3 (Context Tax) must exist');
      assert.strictEqual(cards.throughput, true, 'Card 4 (Throughput & Cost) must exist');

      // 3. Lateral Contextual Drawer click & keyboard interaction
      await page.click('#card-critical-focus');
      await page.waitForTimeout(100);

      const drawerStateOpen = await page.evaluate(() => {
        const drawer = document.getElementById('direct-overview-drawer');
        const rect = drawer.getBoundingClientRect();
        return {
          isOpen: drawer.classList.contains('open') && !drawer.hasAttribute('hidden'),
          width: rect.width,
          hasTraceBox: !!drawer.querySelector('.trace-cli-box'),
          activeElementId: document.activeElement ? document.activeElement.id : null
        };
      });

      assert.strictEqual(drawerStateOpen.isOpen, true, 'Drawer must open on card click');
      assert.ok(
        drawerStateOpen.width >= 480 && drawerStateOpen.width <= 640,
        `Drawer width (${drawerStateOpen.width}px) must be between 480px and 640px`
      );
      assert.strictEqual(drawerStateOpen.hasTraceBox, true, 'Drawer must contain reproducible CLI trace box');

      // Press Escape -> drawer closes and restores focus
      await page.keyboard.press('Escape');
      await page.waitForTimeout(100);

      const drawerStateClosed = await page.evaluate(() => {
        const drawer = document.getElementById('direct-overview-drawer');
        return {
          isHidden: drawer.hasAttribute('hidden'),
          activeElementId: document.activeElement ? document.activeElement.id : null
        };
      });

      assert.strictEqual(drawerStateClosed.isHidden, true, 'Escape key must close drawer');
      assert.strictEqual(
        drawerStateClosed.activeElementId,
        'card-critical-focus',
        'Escape must restore focus to clicked trigger card'
      );

      // 4. Predefined question lenses
      await page.click('.lens-chip[data-lens="v_seat_idle_ranking"]');
      await page.waitForSelector('#direct-overview-drawer.open:not([hidden])', { timeout: 10000 });

      const lensDrawerOpen = await page.evaluate(() => {
        const drawer = document.getElementById('direct-overview-drawer');
        return drawer.classList.contains('open') && !drawer.hasAttribute('hidden');
      });
      assert.strictEqual(lensDrawerOpen, true, 'Predefined lens chip must open drawer');

      await page.keyboard.press('Escape');
      await page.waitForTimeout(100);

      // 5. Dual-Mode Toggle: switch to Modo Forense and back
      await page.click('#btn-mode-forensic');
      await page.waitForTimeout(100);

      const forensicState = await page.evaluate(() => {
        return {
          isForensic: document.body.classList.contains('mode-forensic'),
          isDirect: document.body.classList.contains('mode-direct'),
          cardsDisplay: window.getComputedStyle(document.querySelector('.direct-cards-grid')).display
        };
      });
      assert.strictEqual(forensicState.isForensic, true, 'Body must have mode-forensic class');
      assert.strictEqual(forensicState.cardsDisplay, 'none', 'Direct cards grid must be hidden in Modo Forense');

      await page.click('#btn-mode-direct');
      await page.waitForTimeout(100);

      const directState = await page.evaluate(() => {
        return {
          isForensic: document.body.classList.contains('mode-forensic'),
          isDirect: document.body.classList.contains('mode-direct'),
          cardsDisplay: window.getComputedStyle(document.querySelector('.direct-cards-grid')).display
        };
      });
      assert.strictEqual(directState.isDirect, true, 'Body must have mode-direct class');
      assert.notStrictEqual(directState.cardsDisplay, 'none', 'Direct cards grid must be visible in Modo Direto');
    } finally {
      await browser.close();
    }
  } finally {
    child.kill('SIGTERM');
    try {
      fs.rmSync(rootDir, { recursive: true, force: true });
    } catch (_) {}
  }
});
