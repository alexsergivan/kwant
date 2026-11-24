// 36b a11y-basics (ux-a11y-1124/a11y-audit.md): the audit found 89 serious axe-core nodes, concentrated in
// two always-mounted components (no <main> landmark, sidebar rows nesting a second interactive control
// inside their own button). This runs axe-core against the real rendered app, on a real `kwnt serve`, across
// the highest-value screens the audit flagged (first run/sidebar, a sent message, Settings, the New bot
// modal) and fails the step only on `serious`/`critical` violations — the audit's proposed threshold, which
// alone would have caught every regression it found (Findings 1-6 are all `serious`). The approval-card
// screen is covered instead by the DOM-level test (tests/a11yBasics.test.mjs), not here: reliably driving a
// live approval needs a scripted model reply, which is out of scope for this smallest-footprint guard.
// Not required yet (continue-on-error: true in ci.yml) until the ranked findings above the threshold clear.
//
//   node e2e/a11y-check.mjs <kwnt binary> [port] [out dir]
//
// AXE_CORE points at axe-core's bundled script (a path, not a module specifier); falls back to this repo's
// own frontend/node_modules/axe-core/axe.min.js so a local run needs no separate install.
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const pw = await import(process.env.PW_CORE || 'playwright-core');

const [BIN, PORT = '9684', OUT = fs.mkdtempSync(path.join(os.tmpdir(), 'a11y-check-'))] = process.argv.slice(2);
if (!BIN) throw new Error('usage: node e2e/a11y-check.mjs <kwnt binary> [port] [out dir]');
const BASE = `http://127.0.0.1:${PORT}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
fs.mkdirSync(OUT, { recursive: true });

const defaultAxePath = fileURLToPath(new URL('../node_modules/axe-core/axe.min.js', import.meta.url));
const axeSource = fs.readFileSync(process.env.AXE_CORE || defaultAxePath, 'utf8');

// ---------- a real server ----------
const home = fs.mkdtempSync(path.join(os.tmpdir(), 'a11y-check-home-'));
const dataDir = path.join(home, '.kwnt');
const srv = spawn(BIN, ['serve', '--host', '127.0.0.1', '--port', PORT, '--no-laya', '--data-dir', dataDir], {
  env: { ...process.env, HOME: home, RUST_LOG: 'warn' },
  stdio: ['ignore', fs.openSync(path.join(OUT, 'serve.log'), 'w'), 'inherit'],
});
const stop = () => { try { srv.kill('SIGTERM'); } catch {} };
process.on('exit', stop);
let token = '';
for (let i = 0; i < 120 && !token; i++) {
  await sleep(500);
  try {
    await fetch(`${BASE}/api/kwants`);
    token = fs.readFileSync(path.join(dataDir, 'api-token'), 'utf8').trim();
  } catch {}
}
if (!token) throw new Error('kwnt serve did not start');
const api = async (method, p, body) => {
  const r = await fetch(BASE + p, { method, headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' }, body: body ? JSON.stringify(body) : undefined });
  const t = await r.text();
  if (!r.ok) throw new Error(`${method} ${p}: ${r.status} ${t.slice(0, 200)}`);
  return t ? JSON.parse(t) : null;
};
const bot = async (name) =>
  (await api('POST', '/api/kwants', { name, title: 'Tester', persona_prompt: `You are ${name}, invented for a test.`, soul: '', avatar_sprite: 'architect', driver_type: 'cli', preferred_model: 'claude' })).id;
const wren = await bot('Wren');
const pike = await bot('Pike');
const folder = path.join(home, 'kitchen');
fs.mkdirSync(folder, { recursive: true });
await api('POST', '/api/teams', { name: 'Kitchen', description: 'Test team', icon: null, lead_kwant_id: wren, member_kwants: [wren, pike], workspace_path: folder });

// ---------- the browser ----------
const browser = await pw.chromium.launch({ executablePath: process.env.CHROMIUM || undefined, args: ['--no-sandbox'] });
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: 'dark', serviceWorkers: 'block' });
await ctx.addInitScript(() => {
  try {
    localStorage.setItem('kwant:whatsnew:seen', '9.9.9');
    localStorage.setItem('kwnt.installTip.dismissed', '1');
  } catch {}
});
const page = await ctx.newPage();
const pageErrors = [];
page.on('pageerror', (e) => pageErrors.push(e.message));

const results = [];
const check = (id, name, pass, detail = '') => {
  results.push({ id, name, pass: !!pass, detail });
  console.log(`${pass ? 'PASS' : 'FAIL'} ${id}: ${name}${pass ? '' : ` -> ${detail}`}`);
};

// Runs axe-core on the current page, restricted to `serious`/`critical` impact (the audit's threshold);
// `moderate`/`minor`/best-practice are ignored entirely at first, per the audit's proposed guard.
async function axeCheck(id, name) {
  await page.addScriptTag({ content: axeSource });
  const violations = await page.evaluate(async () => {
    const results = await window.axe.run(document, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'] } });
    return results.violations
      .filter((v) => v.impact === 'serious' || v.impact === 'critical')
      .map((v) => ({ id: v.id, impact: v.impact, nodes: v.nodes.map((n) => n.target) }));
  });
  await page.screenshot({ path: path.join(OUT, `${id}.png`) });
  fs.writeFileSync(path.join(OUT, `${id}-axe.json`), JSON.stringify(violations, null, 1));
  check(id, name, violations.length === 0, JSON.stringify(violations));
}

try {
  await page.goto(BASE + '/');
  await page.waitForTimeout(1500);
  await page.getByRole('button', { name: 'Got it' }).click({ timeout: 1000 }).catch(() => {});
  await page.locator('[data-main-pane]').first().waitFor({ timeout: 10000 });
  await axeCheck('first-run', 'first load: main pane, skip link and sidebar are on screen');

  await page.getByText('Wren', { exact: true }).first().click();
  await page.locator('[data-composer-form]').first().waitFor({ timeout: 10000 });
  await page.locator('textarea').first().click();
  await page.keyboard.type('a message with a ```js\ncode fence\n``` block', { delay: 5 });
  await page.keyboard.press('Enter');
  await page.waitForTimeout(600);
  await axeCheck('sent-message', 'a sent message in a bot chat, composer and sidebar on screen');

  await page.getByRole('button', { name: 'Settings for My space' }).first().click();
  await page.waitForTimeout(500);
  await axeCheck('settings', 'the Settings panel is open');
  await page.keyboard.press('Escape');
  await page.waitForTimeout(300);

  await page.getByRole('button', { name: /^New bot/ }).first().click();
  await page.waitForTimeout(500);
  await axeCheck('new-bot-modal', 'the New bot modal is open, background should be inert');
  await page.keyboard.press('Escape');

  if (pageErrors.length) check('console', 'no uncaught page errors across the run', false, JSON.stringify(pageErrors));
} catch (e) {
  check('run', 'the run finished', false, e.stack?.split('\n').slice(0, 4).join(' / '));
} finally {
  await ctx.close();
  await browser.close();
  stop();
}
fs.writeFileSync(path.join(OUT, 'results.json'), JSON.stringify(results, null, 1));
const failed = results.filter((r) => !r.pass).length;
console.log(`${results.length - failed}/${results.length} passed (out: ${OUT})`);
process.exit(failed ? 1 : 0);
