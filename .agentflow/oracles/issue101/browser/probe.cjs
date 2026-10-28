'use strict';

// Tool availability and real geometry only: no Dendro candidate assertions.
const assert = require('node:assert/strict');
const path = require('node:path');
process.env.PLAYWRIGHT_BROWSERS_PATH = path.join(__dirname, '.cache', 'ms-playwright');
const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch({ headless: true, timeout: 30_000 });
  try {
    const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
    await page.setContent('<!doctype html><html><head><style>html,body{margin:0}#probe{box-sizing:border-box;width:600px;height:480px;position:absolute;left:40px;top:30px}</style></head><body><main id="probe" aria-label="Controlled geometry probe">Probe only</main></body></html>');
    const geometry = await page.evaluate(() => {
      const r = document.getElementById('probe').getBoundingClientRect();
      return { innerWidth, innerHeight, devicePixelRatio, scrollY, scrollHeight: document.documentElement.scrollHeight,
        rect: { x: r.x, y: r.y, width: r.width, height: r.height, bottom: r.bottom } };
    });
    assert.deepEqual(geometry, { innerWidth: 1920, innerHeight: 1080, devicePixelRatio: 1, scrollY: 0, scrollHeight: 1080,
      rect: { x: 40, y: 30, width: 600, height: 480, bottom: 510 } });
    console.log(JSON.stringify({ kind: 'controlled-browser-preflight', candidate_ui_verified: false,
      playwright: require('playwright/package.json').version, chromium: browser.version(), geometry }, null, 2));
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
