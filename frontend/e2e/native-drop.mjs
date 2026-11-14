// 0.8.2 (Pixel): files dragged in from Finder or Explorer must reach the chat. A real OS drop is not a
// JS dispatchEvent: the browser runs a microtask checkpoint after each listener, so the window watch's
// capture-phase drop listener used to re-render the chat and unmount the drop layer (the drop's own
// target) before React saw the drop; the window guard then swallowed it. JS dispatch has no checkpoint
// between listeners, which is why the 0.8.0 checks passed. This test drops real files with Chromium's
// own drag path (CDP Input.dispatchDragEvent with data.files) on the built app served by a real
// `kwnt serve`. It fails on main d61edbff (0.8.1) and passes with the fix.
//
//   node e2e/native-drop.mjs <kwnt binary> [port] [out dir]
//
// DROP=synthetic runs the same cases with page-made DragEvents (JS dispatchEvent), and BROWSER=webkit
// runs those in WebKit (no CDP there), so the 0.8.0 synthetic checks keep passing too.
//
// The binary serves the frontend it was built with (a debug build reads frontend/dist from its source
// tree). Scratch HOME and data dir; invented bots; nothing leaves 127.0.0.1.
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
const pw = await import(process.env.PW_CORE || 'playwright-core');
const MODE = process.env.DROP || 'native';
const BROWSER = process.env.BROWSER || 'chromium';
if (MODE === 'native' && BROWSER !== 'chromium') throw new Error('native drops need Chromium (CDP)');

const [BIN, PORT = '9682', OUT = fs.mkdtempSync(path.join(os.tmpdir(), 'native-drop-'))] = process.argv.slice(2);
if (!BIN) throw new Error('usage: node e2e/native-drop.mjs <kwnt binary> [port] [out dir]');
const BASE = `http://127.0.0.1:${PORT}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
fs.mkdirSync(OUT, { recursive: true });

// ---------- a real server ----------
const home = fs.mkdtempSync(path.join(os.tmpdir(), 'native-drop-home-'));
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

// ---------- real files on disk ----------
const files = path.join(OUT, 'files');
fs.mkdirSync(files, { recursive: true });
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==', 'base64');
const file = (name, body) => (fs.writeFileSync(path.join(files, name), body), path.join(files, name));

// ---------- the browser ----------
const browser = BROWSER === 'webkit' ? await pw.webkit.launch() : await pw.chromium.launch({ executablePath: process.env.CHROMIUM || undefined, args: ['--no-sandbox'] });
console.log(`${MODE} drops in ${BROWSER}`);
// The chips show before the POST /api/attachments responses come back, so a check that counts
// `uploads` first waits for `n` of them, at most 5 s (CI flake on #614, Nov 14).
const uploadsArrive = async (uploads, n, ms = 5000) => {
  for (const end = Date.now() + ms; uploads.length < n && Date.now() < end; ) await sleep(50);
};
const results = [];
const check = (id, name, pass, detail = '') => {
  results.push({ id, name, pass: !!pass, detail });
  console.log(`${pass ? 'PASS' : 'FAIL'} ${id}: ${name}${pass ? '' : ` -> ${detail}`}`);
};

async function newPage() {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 820 }, colorScheme: 'dark', serviceWorkers: 'block' });
  await ctx.addInitScript(() => {
    try {
      localStorage.setItem('kwant:whatsnew:seen', '9.9.9');
      localStorage.setItem('kwnt.installTip.dismissed', '1');
    } catch {}
  });
  const page = await ctx.newPage();
  const uploads = [];
  const errors = [];
  page.on('response', (r) => {
    if (r.request().method() === 'POST' && new URL(r.url()).pathname.startsWith('/api/attachments')) uploads.push(r.status());
  });
  // Headless WebKit has no audio device for the drop's sound; that is the runner, not the app.
  page.on('pageerror', (e) => !/Failed to start the audio device/.test(e.message) && errors.push(e.message));
  await page.goto(BASE + '/');
  await page.waitForTimeout(1500);
  await page.getByRole('button', { name: 'Got it' }).click({ timeout: 1000 }).catch(() => {});
  return { ctx, page, uploads, errors };
}

const openChat = async (page, label) => {
  await page.getByText(label, { exact: true }).first().click();
  await page.locator('[data-drop-target="chat"]').first().waitFor({ timeout: 10000 });
  await page.waitForTimeout(600);
};

const chips = (page) =>
  page.evaluate(() =>
    Array.from(document.querySelectorAll('[data-composer-form] button[aria-label^="Remove "], [data-composer-form] button[aria-label^="Cancel uploading "]')).map((b) =>
      b.getAttribute('aria-label').replace(/^(Remove|Cancel uploading) /, ''),
    ),
  );
const layer = (page) => page.evaluate(() => document.querySelector('[data-drop-layer]')?.getAttribute('data-phase') ?? null);

/** A real drag from outside the window: enter, two overs, then (optionally) the drop, at one point. */
async function nativeDrag(page, pt, paths, opts) {
  return MODE === 'native' ? cdpDrag(page, pt, paths, opts) : jsDrag(page, pt, paths, opts);
}
// Every phase the chat root shows from the drop on ("over>landing>-"), so a lost landing or an "armed"
// frame between the drop and the landing shows up.
const watchPhases = (page) =>
  page.evaluate(() => {
    const root = document.querySelector('[data-drop-target="chat"]');
    const seen = [root?.getAttribute('data-drop-phase') ?? '-'];
    window.__phases = seen;
    if (root) new MutationObserver(() => seen.push(root.getAttribute('data-drop-phase') ?? '-')).observe(root, { attributes: true, attributeFilter: ['data-drop-phase'] });
  });
const phases = (page) => page.evaluate(() => (window.__phases ?? []).join('>'));
async function cdpDrag(page, [x, y], paths, { drop = true } = {}) {
  const cdp = await page.context().newCDPSession(page);
  const data = { items: [], files: paths, dragOperationsMask: 1 };
  for (const type of ['dragEnter', 'dragOver', 'dragOver']) {
    await cdp.send('Input.dispatchDragEvent', { type, x, y, data });
    await page.waitForTimeout(60);
  }
  const before = await layer(page);
  await watchPhases(page);
  if (drop) await cdp.send('Input.dispatchDragEvent', { type: 'drop', x, y, data });
  await page.waitForTimeout(700);
  await cdp.detach();
  return before;
}
/** The 0.8.0 way: page-made DragEvents with a DataTransfer of real File objects, dispatched from JS. */
async function jsDrag(page, [x, y], paths, { drop = true } = {}) {
  const specs = paths.map((p) => ({ name: path.basename(p), type: p.endsWith('.png') ? 'image/png' : p.endsWith('.md') ? 'text/markdown' : 'text/plain', b64: fs.readFileSync(p).toString('base64') }));
  const fire = (type) =>
    page.evaluate(
      ([type, x, y, specs]) => {
        const dt = new DataTransfer();
        for (const s of specs) dt.items.add(new File([Uint8Array.from(atob(s.b64), (c) => c.charCodeAt(0))], s.name, { type: s.type }));
        const t = document.elementFromPoint(x, y);
        t.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, composed: true, clientX: x, clientY: y, dataTransfer: dt }));
      },
      [type, x, y, specs],
    );
  for (const type of ['dragenter', 'dragover', 'dragover']) {
    await fire(type);
    await page.waitForTimeout(60);
  }
  const before = await layer(page);
  await watchPhases(page);
  if (drop) await fire('drop');
  await page.waitForTimeout(700);
  return before;
}
const centerOf = async (page, sel) => {
  const b = await page.locator(sel).first().boundingBox();
  if (!b) throw new Error(`no box for ${sel}`);
  return [Math.round(b.x + b.width / 2), Math.round(b.y + b.height / 2)];
};
// Middle of the message list: the chat column above the composer.
const chatPoint = async (page) => {
  const root = await page.locator('[data-drop-target="chat"]').first().boundingBox();
  const form = await page.locator('[data-composer-form]').first().boundingBox();
  return [Math.round(form.x + form.width / 2), Math.round(root.y + (form.y - root.y) / 2)];
};

async function landsIn(id, name, label, { mention = false } = {}) {
  const { ctx, page, uploads, errors } = await newPage();
  try {
    await openChat(page, label);
    if (mention) {
      await page.locator('textarea').first().click();
      await page.keyboard.type('@Pike', { delay: 20 });
      await page.keyboard.press('Enter');
      await page.waitForTimeout(300);
    }
    const name1 = `${id}-pic.png`;
    const name2 = `${id}-note.txt`;
    const before = await nativeDrag(page, await chatPoint(page), [file(name1, PNG), file(name2, `hello from ${id}\n`)]);
    const got = await chips(page);
    const after = await layer(page);
    const field = await page.locator('textarea').first().inputValue();
    const seq = await phases(page);
    await page.screenshot({ path: path.join(OUT, `${id}.png`) });
    await uploadsArrive(uploads, 2);
    const ok = before === 'over' && got.includes(name1) && got.includes(name2) && uploads.length === 2 && uploads.every((s) => s < 300) && after === null && !errors.length;
    check(id, name, ok, JSON.stringify({ layerBeforeDrop: before, chips: got, uploads, layerAfter: after, field, errors }));
    // "-" before "landing" is the 0.8.0 exit effect's first commit; React flushes that effect in the same
    // sync render (before paint), so it is never seen. An "armed" commit would be: "Drop here…" on screen.
    check(`${id}-landing`, 'the drop plays the landing straight from the card, with no "Drop here" frame between', /^over>(?:->)?landing>-$/.test(seq), seq);
    if (mention) check(`${id}-to`, '@mention: the field still names Pike after the drop', /@Pike/.test(field), JSON.stringify(field));
  } finally {
    await ctx.close();
  }
}

try {
  await landsIn('bot', "a bot's own chat takes the drop", 'Wren');
  await landsIn('room', 'a team room takes the drop', '@Kitchen');
  await landsIn('general', '#general takes the drop', 'general');
  await landsIn('mention', 'an @mention in #general takes the drop', 'general', { mention: true });

  // /idea in the field: the card refuses and the drop adds nothing.
  {
    const { ctx, page, uploads } = await newPage();
    try {
      await openChat(page, 'Wren');
      await page.locator('textarea').first().click();
      await page.keyboard.type('/idea a pricing card', { delay: 15 });
      const before = await nativeDrag(page, await chatPoint(page), [file('idea-pic.png', PNG)]);
      const got = await chips(page);
      check('idea', '/idea refuses the drop: refused card, no chip, no upload, layer gone', before === 'refused' && !got.length && !uploads.length && (await layer(page)) === null, JSON.stringify({ before, chips: got, uploads }));
    } finally {
      await ctx.close();
    }
  }

  // The bot's browser pane beside the chat is not a drop target.
  {
    const { ctx, page, uploads } = await newPage();
    try {
      await openChat(page, 'Wren');
      await page.getByRole('button', { name: 'Bot browser' }).first().click();
      await page.waitForTimeout(800);
      const root = await page.locator('[data-drop-target="chat"]').first().boundingBox();
      const form = await page.locator('[data-composer-form]').first().boundingBox();
      const pt = [Math.round((form.x + form.width + root.x + root.width) / 2), Math.round(root.y + root.height * 0.4)];
      const url = page.url();
      const before = await nativeDrag(page, pt, [file('pane-pic.png', PNG)]);
      const got = await chips(page);
      await page.screenshot({ path: path.join(OUT, 'pane.png') });
      check('pane', 'the side pane takes no drop: armed card only, no chip, no upload, page stays', before === 'armed' && !got.length && !uploads.length && page.url() === url && (await layer(page)) === null, JSON.stringify({ before, chips: got, uploads, url: page.url() }));
    } finally {
      await ctx.close();
    }
  }

  // The Import dialog's own drop zone takes its file; the chat behind it doesn't.
  {
    const { ctx, page, uploads } = await newPage();
    try {
      await openChat(page, 'Wren');
      await page.getByRole('button', { name: 'Settings for My space' }).first().click();
      await page.getByText('Import & export', { exact: true }).first().click();
      await page.getByRole('button', { name: 'Import bots' }).first().click();
      await page.getByText('Upload a file', { exact: true }).first().click();
      const zone = page.locator('div.border-dashed:has([data-picked])').first();
      await zone.waitFor({ timeout: 5000 });
      const before = await nativeDrag(page, await centerOf(page, 'div.border-dashed:has([data-picked])'), [file('import-SOUL.md', '# Import me\n')]);
      const picked = await page.locator('[data-picked]').first().innerText();
      const got = await chips(page);
      await page.screenshot({ path: path.join(OUT, 'import.png') });
      check('import', 'the Import dialog drop zone takes its file; no chat layer, chip or upload', before === null && /import-SOUL\.md/.test(picked) && !got.length && !uploads.length, JSON.stringify({ chatLayerDuringDrag: before, picked, chips: got, uploads }));
    } finally {
      await ctx.close();
    }
  }
} catch (e) {
  check('run', 'the run finished', false, e.stack?.split('\n').slice(0, 4).join(' / '));
} finally {
  await browser.close();
  stop();
}
fs.writeFileSync(path.join(OUT, 'results.json'), JSON.stringify(results, null, 1));
const failed = results.filter((r) => !r.pass).length;
console.log(`${results.length - failed}/${results.length} passed (out: ${OUT})`);
process.exit(failed ? 1 : 0);
