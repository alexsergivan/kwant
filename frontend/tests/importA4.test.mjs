// 0.8.0 A4: the import dialog's three new sources (Mira's release-080/designs/a4-import.md), one test per
// state: choose C1–C4, Found on this computer F1–F6, Folder D1–D4, GitHub link G1–G7. The server is
// mocked with contract-shaped demo data (tests/importA4Demo.mjs).
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';
import * as demo from './importA4Demo.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = (q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {} });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { ImportDialog, SourceList, PreviewView, DoneView } from './src/components/ImportDialog';
  export { foundPrimaryLabel } from './src/components/ImportSources';
  export * from './src/importSources';
  export { ImportError } from './src/imports';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
`);
const { React, createRoot, flushSync } = m;
const h = React.createElement;
const settle = async (n = 8) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const text = (s) => s.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ');
const html = (C, props) => text(m.renderToStaticMarkup(h(C, props)));
const btn = (root, re) => [...root.querySelectorAll('button')].find((b) => re.test(b.textContent.trim()));
const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'Content-Type': 'application/json' } });
const fail = (status, code, error = '') => json({ error, code }, status);

/** Routes for the import API; `over` replaces any route by its path. */
function server(over = {}) {
  const calls = [];
  const routes = {
    'GET /api/import/prompts': () => json([{ id: 'grok', label: 'Grok', where_to_send: 'Open Grok.', prompt: 'Export yourself.' }]),
    'GET /api/import/local': () => json(demo.localList()),
    'POST /api/import/local/preview': () => json(demo.localPreview()),
    'POST /api/import/github/preview': () => json(demo.githubPreview()),
    'POST /api/import/preview': () => json(demo.folderPreview()),
    'POST /api/import/apply': (init) => json(demo.applied(JSON.parse(init.body).bundle ? { bundle: JSON.parse(init.body).bundle } : demo.githubPreview())),
    ...over,
  };
  const fetch = async (url, init = {}) => {
    const method = init.method ?? 'GET';
    const path = String(url).split('?')[0];
    calls.push({ method, url: String(url), body: init.body });
    const r = routes[`${method} ${path}`];
    return r ? r(init, calls.filter((c) => `${c.method} ${c.url.split('?')[0]}` === `${method} ${path}`).length) : json({}, 404);
  };
  return { calls, fetch, count: (key) => calls.filter((c) => `${c.method} ${c.url.split('?')[0]}` === key).length };
}

let cleanup = [];
async function open({ desktop = false, folders = false, over = {} } = {}) {
  const s = server(over);
  g.fetch = g.window.fetch = s.fetch;
  if (desktop) g.window.__KWANT_APP__ = 'mac';
  else delete g.window.__KWANT_APP__;
  if (folders) Object.defineProperty(g.HTMLInputElement.prototype, 'webkitdirectory', { value: false, configurable: true, writable: true });
  else delete g.HTMLInputElement.prototype.webkitdirectory;
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  const changed = [];
  flushSync(() => root.render(h(m.ImportDialog, { spaceName: 'Home', onClose() {}, onChanged: () => changed.push(1) })));
  cleanup.push(() => (root.unmount(), host.remove()));
  await settle();
  const dlg = () => document.querySelector('[role="dialog"]');
  return { s, dlg, changed, click: async (re) => (btn(dlg(), re).click(), settle()) };
}
test.afterEach(() => {
  cleanup.forEach((f) => f());
  cleanup = [];
  delete g.window.__KWANT_APP__;
});

const footer = (dlg) => dlg().lastElementChild;
const primaryOf = (dlg) => [...footer(dlg).querySelectorAll('button')].pop();

// ------------------------------------------------------------------ choose (C1–C4)

test('C3 web app: no "On this computer" group and no look at the usual places; folder row needs folder picking', async () => {
  const d = await open();
  assert.equal(d.dlg().querySelector('[data-found-row]'), null);
  assert.doesNotMatch(d.dlg().textContent, /On this computer/);
  assert.equal(d.s.count('GET /api/import/local'), 0);
  assert.equal(d.dlg().querySelector('[data-folder-row]'), null, 'hidden where the browser can’t pick folders');
  assert.ok(d.dlg().querySelector('[data-github-row]'));
  assert.match(d.dlg().textContent, /Or bring files.*Upload a file.*GitHub link/);
});

test('C1 found: the lit row with names and "N found"; Folder row when folders can be picked', async () => {
  const d = await open({ desktop: true, folders: true });
  const row = d.dlg().querySelector('[data-found-row]');
  assert.equal(row.dataset.foundRow, 'found');
  assert.match(row.className, /border-sky-500\/45/);
  assert.equal(row.getAttribute('aria-label'), 'Found on this computer, 2 found');
  assert.match(row.textContent, /OpenClaw and Claude Code skills · nothing is read yet/);
  assert.ok(d.dlg().querySelector('[data-folder-row]'));
  assert.equal(d.s.count('GET /api/import/local'), 1);
});

test('C2 loading, C4 nothing, and a failed look', async () => {
  let d = await open({ desktop: true, over: { 'GET /api/import/local': () => new Promise(() => {}) } });
  let row = d.dlg().querySelector('[data-found-row]');
  assert.equal(row.dataset.foundRow, 'loading');
  assert.match(row.querySelector('[role="status"]').textContent, /Looking in 5 usual places…/);
  cleanup.forEach((f) => f());
  cleanup = [];
  d = await open({ desktop: true, over: { 'GET /api/import/local': () => json({ available: true, sources: [] }) } });
  row = d.dlg().querySelector('[data-found-row]');
  assert.equal(row.dataset.foundRow, 'nothing');
  assert.doesNotMatch(row.className, /border-sky-500/);
  assert.match(row.textContent, /Nothing in the usual places\. See where kwant looked/);
  cleanup.forEach((f) => f());
  cleanup = [];
  d = await open({ desktop: true, over: { 'GET /api/import/local': () => fail(500, 'internal') } });
  assert.match(d.dlg().querySelector('[data-found-row]').textContent, /Couldn’t look just now/);
});

test('available:false in the desktop app hides the group like the web app', async () => {
  const d = await open({ desktop: true, over: { 'GET /api/import/local': () => json({ available: false, reason: 'No home folder', sources: [] }) } });
  assert.equal(d.dlg().querySelector('[data-found-row]'), null);
});

// ------------------------------------------------------------------ Found on this computer (F1–F6)

test('F1 loading: status, skeleton rows, Preview disabled', async () => {
  const d = await open({ desktop: true, over: { 'GET /api/import/local': () => new Promise(() => {}) } });
  await d.click(/^Found on this computer/);
  assert.match(d.dlg().querySelector('h2').textContent, /Found on this computer/);
  assert.equal(d.dlg().querySelector('[data-found-state]').dataset.foundState, 'loading');
  assert.match(d.dlg().querySelector('[role="status"]').textContent, /Looking in 5 usual places/);
  assert.equal(primaryOf(d.dlg).disabled, true);
});

test('F2 found: rows start unticked, too_large is disabled with the Folder hint, ticking counts sources', async () => {
  const d = await open({ desktop: true });
  await d.click(/^Found on this computer/);
  const boxes = [...d.dlg().querySelectorAll('input[type="checkbox"]')];
  assert.equal(boxes.length, 3);
  assert.ok(boxes.every((b) => !b.checked), 'all rows start unticked');
  assert.equal(boxes[2].disabled, true, 'too big');
  assert.match(d.dlg().textContent, /Too big to bring from here\. Use Folder to pick just the parts you want\./);
  assert.match(d.dlg().textContent, /1 bot · 3 skills · 2 memory files/);
  assert.match(d.dlg().textContent, /~\/\.openclaw\/workspace/, 'path as sent');
  assert.match(d.dlg().textContent, /Read-only\. kwant never follows a link out of these folders/);
  assert.equal(primaryOf(d.dlg).textContent.trim(), 'Preview');
  assert.equal(primaryOf(d.dlg).disabled, true);
  assert.ok(btn(footer(d.dlg), /^Cancel$/));
  boxes[0].click();
  boxes[1].click();
  await settle();
  assert.equal(primaryOf(d.dlg).textContent.trim(), 'Preview 2 sources');
  assert.equal(primaryOf(d.dlg).disabled, false);
  // Names on request.
  const show = btn(d.dlg(), /^Show names$/);
  assert.equal(show.getAttribute('aria-expanded'), 'false');
  show.click();
  await settle();
  assert.match(d.dlg().textContent, /seo.*support.*triage/);
});

test('F6 reading, then F5 preview: per-bot source and the shared-skills line', async () => {
  let release;
  const d = await open({ desktop: true, over: { 'POST /api/import/local/preview': () => new Promise((r) => (release = () => r(json(demo.localPreview())))) } });
  await d.click(/^Found on this computer/);
  const boxes = [...d.dlg().querySelectorAll('input[type="checkbox"]')];
  boxes[0].click();
  boxes[1].click();
  await settle();
  primaryOf(d.dlg).click();
  await settle();
  assert.match(primaryOf(d.dlg).textContent, /Reading 2 sources…/);
  assert.equal(primaryOf(d.dlg).disabled, true);
  assert.ok([...d.dlg().querySelectorAll('input[type="checkbox"]')].every((b) => b.disabled), 'ticks locked while reading');
  const sent = JSON.parse(d.s.calls.find((c) => c.url === '/api/import/local/preview').body);
  assert.deepEqual(sent, { sources: ['openclaw', 'claude-skills'] });
  release();
  await settle();
  const out = d.dlg().textContent;
  assert.match(d.dlg().querySelector('h2').textContent, /Check before importing/);
  assert.match(out, /1 bot will be created from 2 sources on this computer/);
  assert.match(out, /From OpenClaw · ~\/\.openclaw\/workspace/);
  assert.match(out, /Also: 7 shared skills from ~\/\.claude\/skills\./);
  assert.match(out, /openclaw\.json and credentials\/: never opened/);
  assert.equal(primaryOf(d.dlg).textContent.trim(), 'Import 1 bot');
  // Back keeps the ticks.
  await d.click(/^$/); // Back is the icon button first in the header
  assert.equal(d.dlg().querySelector('h2').textContent, 'Found on this computer');
  assert.equal([...d.dlg().querySelectorAll('input[type="checkbox"]')].filter((b) => b.checked).length, 2);
});

test('F2 errors: too_large keeps the ticks; not_found looks again and drops a vanished source', async () => {
  let d = await open({ desktop: true, over: { 'POST /api/import/local/preview': () => fail(413, 'too_large', 'over 50 MB') } });
  await d.click(/^Found on this computer/);
  d.dlg().querySelectorAll('input[type="checkbox"]')[0].click();
  await settle();
  primaryOf(d.dlg).click();
  await settle();
  assert.match(d.dlg().querySelector('[role="alert"]').textContent, /Together these are too big \(over 500 files or 50 MB\)\. Tick fewer\./);
  assert.doesNotMatch(d.dlg().textContent, /over 50 MB/, 'nothing raw');
  assert.equal(d.dlg().querySelectorAll('input[type="checkbox"]')[0].checked, true);
  cleanup.forEach((f) => f());
  cleanup = [];

  d = await open({
    desktop: true,
    over: {
      'GET /api/import/local': (_i, n) => json(n === 1 ? demo.localList() : demo.localList({ sources: demo.localList().sources.slice(1) })),
      'POST /api/import/local/preview': () => fail(404, 'not_found', 'gone'),
    },
  });
  await d.click(/^Found on this computer/);
  const boxes = d.dlg().querySelectorAll('input[type="checkbox"]');
  boxes[0].click();
  boxes[1].click();
  await settle();
  primaryOf(d.dlg).click();
  await settle(12);
  assert.match(d.dlg().textContent, /One of these isn’t there any more\. kwant looked again\./);
  assert.equal(d.s.count('GET /api/import/local'), 2, 'looked again');
  const now = [...d.dlg().querySelectorAll('input[type="checkbox"]')];
  assert.equal(now.length, 2, 'the vanished row is gone');
  assert.equal(now.filter((b) => b.checked).length, 1);
  assert.equal(primaryOf(d.dlg).textContent.trim(), 'Preview 1 source');
});

test('F3 nothing: the five places, Pick a folder, footer Back', async () => {
  const d = await open({ desktop: true, folders: true, over: { 'GET /api/import/local': () => json({ available: true, sources: [] }) } });
  await d.click(/^Found on this computer/);
  assert.equal(d.dlg().querySelector('[data-found-state]').dataset.foundState, 'nothing');
  for (const p of m.USUAL_PLACES) assert.ok(d.dlg().textContent.includes(p), p);
  assert.deepEqual([...footer(d.dlg).querySelectorAll('button')].map((b) => b.textContent.trim()), ['Back']);
  await d.click(/^Pick a folder$/);
  assert.equal(d.dlg().querySelector('h2').textContent, 'Bring a folder');
});

test('F4 error: Couldn’t look, Try again looks again, and the other ways stay open', async () => {
  const d = await open({ desktop: true, folders: true, over: { 'GET /api/import/local': (_i, n) => (n === 1 ? fail(500, 'internal', 'raw boom') : json(demo.localList())) } });
  await d.click(/^Found on this computer/);
  assert.equal(d.dlg().querySelector('[data-found-state]').dataset.foundState, 'error');
  assert.match(d.dlg().textContent, /Couldn’t look on this computer/);
  assert.match(d.dlg().textContent, /You can still pick a folder or upload a file\./);
  assert.doesNotMatch(d.dlg().textContent, /raw boom/);
  await d.click(/^Try again$/);
  assert.equal(d.dlg().querySelector('[data-found-state]').dataset.foundState, 'found');
});

// ------------------------------------------------------------------ Folder (D1–D4)

function pickFolder(dlg, list) {
  const input = dlg().querySelector('input[type="file"]');
  assert.ok(input.hasAttribute('webkitdirectory'));
  const files = list.map(([path, body]) => {
    const f = new File([body], path.split('/').pop());
    Object.defineProperty(f, 'webkitRelativePath', { value: path });
    return f;
  });
  Object.defineProperty(input, 'files', { value: files, configurable: true });
  input.dispatchEvent(new window.Event('change', { bubbles: true }));
}
const zipNames = (bytes) => {
  const names = [];
  for (let i = 0; i + 46 <= bytes.length; i++)
    if (bytes[i] === 0x50 && bytes[i + 1] === 0x4b && bytes[i + 2] === 0x01 && bytes[i + 3] === 0x02) {
      const n = bytes[i + 28] | (bytes[i + 29] << 8);
      names.push(new TextDecoder().decode(bytes.slice(i + 46, i + 46 + n)));
    }
  return names.sort();
};

test('D1 empty → D2 picked: what works, left-out list, then a zip of only readable files as <folder>.zip', async () => {
  const d = await open({ folders: true });
  await d.click(/^Folder/);
  assert.equal(d.dlg().querySelector('h2').textContent, 'Bring a folder');
  assert.match(d.dlg().textContent, /A skills folder.*Up to 20 MB once packed/);
  assert.equal(primaryOf(d.dlg).disabled, true);
  pickFolder(d.dlg, demo.folderFiles());
  await settle();
  assert.match(d.dlg().querySelector('[data-picked]').textContent, /molty-workspace4 files · 1 KB/);
  assert.match(d.dlg().textContent, /Left out before packing/);
  assert.match(d.dlg().textContent, /Sign-in and settings files \(never sent\): 2 files/);
  assert.ok(btn(footer(d.dlg), /^Change folder$/));
  primaryOf(d.dlg).click();
  await settle(12);
  const call = d.s.calls.find((c) => c.url.startsWith('/api/import/preview'));
  assert.equal(call.url, '/api/import/preview?filename=molty-workspace.zip');
  const names = zipNames(new Uint8Array(await call.body.arrayBuffer()));
  assert.deepEqual(names, ['molty-workspace/AGENTS.md', 'molty-workspace/IDENTITY.md', 'molty-workspace/SOUL.md', 'molty-workspace/skills/triage/SKILL.md']);
  assert.match(d.dlg().textContent, /1 bot will be created from molty-workspace/);
  assert.match(d.dlg().textContent, /Files that weren't read: notes\.txt/);
});

test('D3 too big: checked before sending, rose alert, Preview disabled', async () => {
  const d = await open({ folders: true });
  await d.click(/^Folder/);
  const many = Array.from({ length: 501 }, (_, i) => [`big/skills/s${i}/SKILL.md`, 'x']);
  pickFolder(d.dlg, many);
  await settle();
  const alert = d.dlg().querySelector('[data-folder-problem]');
  assert.equal(alert.getAttribute('role'), 'alert');
  assert.match(alert.textContent, /That folder has 501 readable files, over the 500 limit\. Pick a smaller part of it, such as its skills folder\./);
  assert.equal(primaryOf(d.dlg).disabled, true);
  assert.equal(d.s.count('POST /api/import/preview'), 0);
});

test('D4 nothing recognised (415): the copy box and what works, no raw text', async () => {
  const d = await open({ folders: true, over: { 'POST /api/import/preview': () => fail(415, 'unsupported', 'raw: no format matched') } });
  await d.click(/^Folder/);
  pickFolder(d.dlg, [['notes/agents/readme.md', 'hi']]);
  await settle();
  primaryOf(d.dlg).click();
  await settle(12);
  assert.match(d.dlg().textContent, /Nothing kwant can bring in this folder/);
  assert.match(d.dlg().textContent, /looked for SOUL\.md, AGENTS\.md, SKILL\.md and agents\/\*\.md/);
  assert.doesNotMatch(d.dlg().textContent, /raw:/);
  assert.equal(d.dlg().querySelector('h2').textContent, 'Bring a folder');
});

// ------------------------------------------------------------------ GitHub link (G1–G7)

function typeUrl(dlg, value) {
  const input = dlg().querySelector('input[type="url"]');
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(input, value);
  input.dispatchEvent(new window.Event('input', { bubbles: true }));
  return input;
}

test('G1 → G2: the empty field, then a non-GitHub link gets the rose line and blocks Preview', async () => {
  const d = await open();
  await d.click(/^GitHub link/);
  assert.equal(d.dlg().querySelector('h2').textContent, 'Bring from a GitHub link');
  const input = d.dlg().querySelector('input[type="url"]');
  assert.equal(input.getAttribute('inputmode'), 'url');
  assert.equal(input.getAttribute('autocomplete'), 'off');
  assert.equal(input.getAttribute('spellcheck'), 'false');
  assert.equal(document.activeElement, input, 'autofocused');
  assert.equal(primaryOf(d.dlg).disabled, true);
  typeUrl(d.dlg, 'https://gitlab.com/a/b');
  await settle();
  primaryOf(d.dlg).click();
  await settle();
  assert.match(d.dlg().querySelector('[role="alert"]').textContent, /Only github\.com links work here\. kwant doesn’t fetch from other sites\./);
  assert.equal(input.getAttribute('aria-invalid'), 'true');
  assert.equal(primaryOf(d.dlg).disabled, true);
  assert.equal(d.s.count('POST /api/import/github/preview'), 0);
});

test('G3 fetching → G4 error → Try again', async () => {
  let release;
  const d = await open({ over: { 'POST /api/import/github/preview': (_i, n) => (n === 1 ? new Promise((r) => (release = () => r(fail(404, 'not_found', 'raw 404')))) : json(demo.githubPreview())) } });
  await d.click(/^GitHub link/);
  typeUrl(d.dlg, 'github.com/example-team/support-skills');
  await settle();
  primaryOf(d.dlg).click();
  await settle();
  assert.match(d.dlg().querySelector('[role="status"]').textContent, /Downloading from github\.com…/);
  assert.equal(primaryOf(d.dlg).textContent.trim(), 'Fetching…');
  assert.equal(primaryOf(d.dlg).disabled, true);
  assert.deepEqual(JSON.parse(d.s.calls.find((c) => c.url === '/api/import/github/preview').body), { url: 'github.com/example-team/support-skills' });
  release();
  await settle();
  assert.match(d.dlg().textContent, /No public repo at that link/);
  assert.match(d.dlg().textContent, /Only public repos work for now\./);
  assert.doesNotMatch(d.dlg().textContent, /raw 404/);
  assert.equal(primaryOf(d.dlg).textContent.trim(), 'Try again');
  primaryOf(d.dlg).click();
  await settle(12);
  assert.equal(d.dlg().querySelector('h2').textContent, 'Check before importing');
});

test('every GitHub error code gets Mira’s words', () => {
  const want = {
    not_found: 'No public repo at that link',
    too_large: 'That repo is too big',
    unsupported: 'Nothing kwant can bring here',
    redirect: 'GitHub sent kwant somewhere else',
    invalid: 'That download wasn’t a safe zip',
    fetch_failed: 'GitHub didn’t answer',
  };
  for (const [code, title] of Object.entries(want)) assert.equal(m.githubErrorText(new m.ImportError('raw', code, 422)).title, title, code);
});

test('GitHub too_large: over 20 MB gets Mira’s smoother line (Later 4); the 500-file case keeps the folder-link advice', () => {
  const want = 'This repo is over 20 MB, so kwant can’t download it. Download it from GitHub yourself, then bring just the part you need with Folder, or zip that part and upload it.';
  const big = m.githubErrorText(new m.ImportError("This repository is larger than 20 MB, so kwant can't fetch it. Download the folder you want, zip it and import the zip instead.", 'too_large', 413));
  assert.equal(big.title, 'That repo is too big');
  assert.equal(big.detail, want);
  // Still matched if the server line is reworded the same way later.
  assert.equal(m.githubErrorText(new m.ImportError('This repo is over 20 MB.', 'too_large', 413)).detail, want);
  const many = m.githubErrorText(new m.ImportError('too many files', 'too_large', 413));
  assert.match(many.detail, /Link to the folder you need instead/);
});

test('G5 preview with MIT: source block, chip, skill chips, Import N skills; the attribution isn’t repeated', async () => {
  const d = await open();
  await d.click(/^GitHub link/);
  typeUrl(d.dlg, 'https://github.com/example-team/support-skills');
  await settle();
  primaryOf(d.dlg).click();
  await settle(12);
  const block = d.dlg().querySelector('[data-github-source]');
  assert.match(block.textContent, /example-team\/support-skills/);
  assert.match(block.textContent, /default branch · commit 3f2a91c/);
  assert.equal(block.querySelector('a').getAttribute('href'), 'https://github.com/example-team/support-skills');
  assert.equal(block.querySelector('a').getAttribute('rel'), 'noopener noreferrer');
  const chip = block.querySelector('[data-licence]');
  assert.match(chip.textContent, /MIT licence/);
  assert.doesNotMatch(chip.className, /kw-chip-warn/);
  assert.match(block.textContent, /Free to reuse\. Keep the author’s notice\./);
  const out = d.dlg().textContent;
  assert.match(out, /No bots in this one\. 12 skills will be added to Home\./);
  assert.match(out, /and 4 more/);
  assert.equal(d.dlg().querySelectorAll('ul[aria-label="Skills"] li.font-mono').length, 8);
  assert.match(out, /scripts\/: 6 files that run code/);
  assert.doesNotMatch(out, /From github\.com\/example-team/, 'attribution is in the source block, not Not brought over');
  assert.equal(primaryOf(d.dlg).textContent.trim(), 'Import 12 skills');
  assert.equal(primaryOf(d.dlg).disabled, false);
  // G7 done.
  primaryOf(d.dlg).click();
  await settle(12);
  assert.match(d.dlg().textContent, /Imported 12 skills/);
  assert.match(d.dlg().querySelector('[data-github-done]').textContent, /From github\.com\/example-team\/support-skills \(MIT\)\. They wait for your OK in Settings › This space › Skills\./);
  assert.ok(btn(footer(d.dlg), /Undo import/));
  assert.equal(d.changed.length, 1);
});

test('GitHub preview, Back, then a skills-only folder: Done has no GitHub line and no licence chip (Ada’s #459 review)', async () => {
  const skillsOnly = () => {
    const p = demo.githubPreview();
    delete p.github;
    p.bundle.source = { kind: 'skills', label: 'support-skills', exported_at: null };
    p.bundle.omitted = [];
    return json(p);
  };
  const d = await open({ folders: true, over: { 'POST /api/import/preview': skillsOnly } });
  await d.click(/^GitHub link/);
  typeUrl(d.dlg, 'https://github.com/example-team/support-skills');
  await settle();
  primaryOf(d.dlg).click();
  await settle(12);
  assert.ok(d.dlg().querySelector('[data-github-source]'), 'GitHub preview shown first');
  const back = async () => (d.dlg().querySelector('button[aria-label="Back"]').click(), settle());
  await back();
  await back();
  await d.click(/^Folder/);
  pickFolder(d.dlg, [['my-skills/triage/SKILL.md', '---\nname: triage\n---\n'], ['my-skills/seo/SKILL.md', '---\nname: seo\n---\n']]);
  await settle();
  primaryOf(d.dlg).click();
  await settle(12);
  assert.equal(d.dlg().querySelector('[data-github-source]'), null, 'folder preview has no GitHub block');
  primaryOf(d.dlg).click();
  await settle(12);
  assert.match(d.dlg().textContent, /Imported 12 skills/);
  assert.doesNotMatch(d.dlg().textContent, /From github\.com/);
  assert.equal(d.dlg().querySelector('[data-github-done]'), null);
  assert.equal(d.dlg().querySelector('[data-licence]'), null);
  assert.doesNotMatch(d.dlg().textContent, /MIT/);
});

test('G6 no licence: amber chip and the warning in Worth a look, never blocking', () => {
  const p = demo.githubPreview({ licence: null });
  const out = html(m.PreviewView, { preview: p, github: p.github, skip: new Set(), onToggle() {}, spaceName: 'Home' });
  const raw = m.renderToStaticMarkup(h(m.PreviewView, { preview: p, github: p.github, skip: new Set(), onToggle() {}, spaceName: 'Home' }));
  assert.match(raw, /kw-chip kw-chip-warn[^"]*" data-licence/);
  assert.match(out, /No licence/);
  assert.match(out, /Worth a look.*no licence file/);
});

test('unknown licence: plain chip and a link to read it', () => {
  const p = demo.githubPreview({ licence: { spdx: null, name: 'Custom', file: 'LICENSE.md' }, ref: 'main', path: 'skills' });
  const raw = m.renderToStaticMarkup(h(m.PreviewView, { preview: p, github: p.github, skip: new Set(), onToggle() {} }));
  const out = text(raw);
  assert.match(out, /Unknown licence/);
  assert.match(out, /Read the licence before you share these\./);
  assert.match(raw, /href="https:\/\/github\.com\/example-team\/support-skills\/blob\/3f2a91c08d5e4b7a9c1f2e3d4c5b6a7980716253\/[^"]*LICENSE\.md"/);
  assert.match(out, /main · commit 3f2a91c · \/skills/);
});

// ------------------------------------------------------------------ pure helpers

test('githubLinkOk takes the contract’s three shapes on github.com only', () => {
  for (const ok of ['https://github.com/a/b', 'github.com/a/b', 'https://www.github.com/a/b.git', 'github.com/a/b/tree/main', 'github.com/a/b/tree/main/skills/x', 'github.com/a/b/blob/main/SKILL.md'])
    assert.ok(m.githubLinkOk(ok), ok);
  for (const bad of ['http://github.com/a/b', 'https://gitlab.com/a/b', 'https://github.com.evil.com/a/b', 'https://user@github.com/a/b', 'https://github.com:8443/a/b', 'github.com/a', 'github.com/a/b/issues/1', 'github.com/a/b/tree/main/%2e%2e', 'github.com/a/b\\c', ''])
    assert.equal(m.githubLinkOk(bad), null, bad);
});

test('found helpers: hint, counts, primary label, origin', () => {
  const s = demo.localList().sources;
  assert.equal(m.foundHint(s.slice(0, 2)), 'OpenClaw and Claude Code skills');
  assert.equal(m.countsLine(s[0]), '1 bot · 3 skills · 2 memory files');
  assert.equal(m.countsLine({ bots: 0, skills: 7, memories: 0 }), '7 skills');
  assert.equal(m.foundPrimaryLabel(0, false), 'Preview');
  assert.equal(m.foundPrimaryLabel(1, false), 'Preview 1 source');
  assert.equal(m.foundPrimaryLabel(2, true), 'Reading 2 sources…');
  const o = m.localOrigin(s.slice(0, 2));
  assert.equal(o.botSource.id, 'openclaw');
  assert.equal(o.skillSource.id, 'claude-skills');
  assert.equal(m.localOrigin([s[0], { ...s[0], id: 'x' }]).botSource, null, 'two bot sources: not certain');
});

test('folder picking is off on iPhone and iPad, even where the property exists', () => {
  const proto = { webkitdirectory: false };
  assert.equal(m.folderPickSupported(proto, { userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)', maxTouchPoints: 0 }), true);
  assert.equal(m.folderPickSupported(proto, { userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)' }), false);
  assert.equal(m.folderPickSupported(proto, { userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)', maxTouchPoints: 5 }), false);
  assert.equal(m.folderPickSupported({}, { userAgent: 'x' }), false);
});

// ------------------------------------------------------------------ A4 follow-ups (release-080/a4-followups.md, Pixel 1–11)

test('item 1, F1 focus: opening Found while it looks keeps focus in the dialog, then moves to the first checkbox', async () => {
  let release;
  const d = await open({ desktop: true, over: { 'GET /api/import/local': () => new Promise((r) => (release = () => r(json(demo.localList())))) } });
  await d.click(/^Found on this computer/);
  const status = d.dlg().querySelector('[data-found-state="loading"] [role="status"]');
  assert.equal(document.activeElement, status, 'the Looking… line holds focus');
  assert.equal(status.getAttribute('tabindex'), '-1');
  assert.ok(d.dlg().contains(document.activeElement));
  release();
  await settle(12);
  const first = d.dlg().querySelector('input[type="checkbox"]');
  assert.ok(first, 'list arrived');
  assert.equal(document.activeElement, first, 'focus moves to the first checkbox');
});

test('item 1, F1 focus: a failed look moves focus to Try again', async () => {
  let release;
  const d = await open({ desktop: true, over: { 'GET /api/import/local': () => new Promise((r) => (release = () => r(fail(500, 'internal')))) } });
  await d.click(/^Found on this computer/);
  release();
  await settle(12);
  assert.match(document.activeElement.textContent, /Try again/);
});

test('item 2: Enter on the GitHub field runs Preview, with Preview’s rules', async () => {
  const d = await open();
  await d.click(/^GitHub link/);
  const form = d.dlg().querySelector('input[type="url"]').closest('form');
  assert.ok(form, 'the field is in a form');
  form.requestSubmit();
  await settle();
  assert.equal(d.s.count('POST /api/import/github/preview'), 0, 'empty: nothing sent');
  typeUrl(d.dlg, 'https://gitlab.com/a/b');
  await settle();
  form.requestSubmit();
  await settle();
  assert.equal(d.s.count('POST /api/import/github/preview'), 0, 'bad link: nothing sent');
  assert.match(d.dlg().querySelector('[role="alert"]').textContent, /Only github\.com links/);
  typeUrl(d.dlg, 'github.com/example-team/support-skills');
  await settle();
  d.dlg().querySelector('input[type="url"]').closest('form').requestSubmit();
  await settle(12);
  assert.equal(d.s.count('POST /api/import/github/preview'), 1);
  assert.equal(d.dlg().querySelector('h2').textContent, 'Check before importing');
});

test('item 3, D4: after a 415 Preview stays off until another folder is picked', async () => {
  const d = await open({ folders: true, over: { 'POST /api/import/preview': (_i, n) => (n === 1 ? fail(415, 'unsupported', 'raw') : json(demo.folderPreview())) } });
  await d.click(/^Folder/);
  pickFolder(d.dlg, [['notes/agents/readme.md', 'hi']]);
  await settle();
  primaryOf(d.dlg).click();
  await settle(12);
  assert.match(d.dlg().textContent, /Nothing kwant can bring in this folder/);
  assert.equal(primaryOf(d.dlg).disabled, true, 'Preview off for the same folder');
  assert.ok(btn(footer(d.dlg), /^Change folder$/));
  pickFolder(d.dlg, demo.folderFiles());
  await settle();
  assert.equal(primaryOf(d.dlg).disabled, false, 'a new folder clears it');
});

test('item 4: the not-GitHub line says why — http://, a github.com link of the wrong shape, or another site', async () => {
  const cases = [
    ['http://github.com/a/b', 'http', 'Use the https:// link. kwant only downloads from github.com over https.'],
    ['http://www.github.com/a/b', 'http', 'Use the https:// link. kwant only downloads from github.com over https.'],
    ['https://github.com/owner', 'shape', 'Link to a repo, a branch or a folder in it, like github.com/owner/repo.'],
    ['github.com/a/b/issues/1', 'shape', 'Link to a repo, a branch or a folder in it, like github.com/owner/repo.'],
    ['https://github.com/a/b/pull/2', 'shape', 'Link to a repo, a branch or a folder in it, like github.com/owner/repo.'],
    ['https://gist.github.com/a/b', 'other_site', 'Only github.com links work here. kwant doesn’t fetch from other sites.'],
    ['https://gitlab.com/a/b', 'other_site', 'Only github.com links work here. kwant doesn’t fetch from other sites.'],
    ['https://user@github.com/a/b', 'other_site', 'Only github.com links work here. kwant doesn’t fetch from other sites.'],
    ['https://github.com:8443/a/b', 'other_site', 'Only github.com links work here. kwant doesn’t fetch from other sites.'],
  ];
  for (const [url, problem, line] of cases) {
    assert.deepEqual(m.githubLinkCheck(url), { problem }, url);
    assert.equal(m.githubLinkProblemText(problem), line, url);
  }
  assert.deepEqual(m.githubLinkCheck('github.com/a/b'), { owner: 'a', repo: 'b' });
  const d = await open();
  await d.click(/^GitHub link/);
  for (const [url, problem, line] of cases.slice(0, 3).concat([cases[6]])) {
    typeUrl(d.dlg, url);
    await settle();
    primaryOf(d.dlg).click();
    await settle();
    const alert = d.dlg().querySelector('[role="alert"]');
    assert.equal(alert.dataset.githubProblem, problem, url);
    assert.equal(alert.textContent, line, url);
  }
  assert.equal(d.s.count('POST /api/import/github/preview'), 0);
});

test('item 4: the server’s 422 not_github keeps the plain other-sites line', async () => {
  const d = await open({ over: { 'POST /api/import/github/preview': () => fail(422, 'not_github', 'raw') } });
  await d.click(/^GitHub link/);
  typeUrl(d.dlg, 'github.com/a/b');
  await settle();
  primaryOf(d.dlg).click();
  await settle(12);
  assert.equal(d.dlg().querySelector('[role="alert"]').textContent, 'Only github.com links work here. kwant doesn’t fetch from other sites.');
});

test('item 5, Decision 3: GitHub preview with bots → Import → Done shows "From github.com/" and the licence', async () => {
  const p = demo.githubBotsPreview();
  const d = await open({ over: { 'POST /api/import/github/preview': () => json(p), 'POST /api/import/apply': () => json(demo.applied(p)) } });
  await d.click(/^GitHub link/);
  typeUrl(d.dlg, 'https://github.com/example-team/ai-team');
  await settle();
  primaryOf(d.dlg).click();
  await settle(12);
  assert.equal(primaryOf(d.dlg).textContent.trim(), 'Import 3 bots');
  primaryOf(d.dlg).click();
  await settle(12);
  assert.match(d.dlg().textContent, /Imported 3 bots/);
  const line = d.dlg().querySelector('[data-github-done]');
  assert.ok(line, 'GitHub credit on Done with bots');
  assert.equal(line.textContent, 'From github.com/example-team/ai-team (MIT).');
  assert.match(line.className, /text-\[11px\] text-zinc-400/);
  assert.match(d.dlg().textContent, /Give them an account and tools in Settings › This space/);
  // Order: title, the From line, then the names.
  assert.match(d.dlg().textContent, /Imported 3 bots.*From github\.com\/example-team\/ai-team \(MIT\)\..*Ada, Ben, Cy/);
});

test('item 6: 44 px targets on phone — per-bot tick rows and the Open on GitHub link', () => {
  const p = demo.githubBotsPreview();
  const raw = m.renderToStaticMarkup(h(m.PreviewView, { preview: p, github: p.github, skip: new Set(), onToggle() {} }));
  const ticks = raw.match(/<label class="[^"]*"[^>]*data-bot-tick/g) ?? [];
  assert.equal(ticks.length, 3);
  // The phone layout runs to 767 px (PHONE_QUERY), so the 44 px targets use max-md:, not an sm: reset at 640 px.
  for (const t of ticks) {
    assert.match(t, /max-md:min-h-11/);
    assert.doesNotMatch(t, /(^|[\s"])(min-h-11|sm:min-h-0)\b/);
  }
  const link = raw.match(/<a [^>]*class="([^"]*)"[^>]*>Open/)?.[1] ?? '';
  for (const c of ['max-md:min-h-11', 'max-md:-my-3.5', 'max-md:px-2', 'max-md:-mr-2']) assert.ok(link.split(/\s+/).includes(c), `${c} in ${link}`);
  assert.doesNotMatch(link, /\bsm:/);
});

test('item 7: an empty GitHub preview doesn’t say "Untick any you don’t want"', () => {
  const p = demo.githubEmptyPreview();
  const out = html(m.PreviewView, { preview: p, github: p.github, skip: new Set(), onToggle() {} });
  assert.doesNotMatch(out, /Untick/);
  assert.match(out, /Nothing here can be brought in: no bots and no skills\./);
  assert.match(out, /SKILL\.md is over 8,000 characters/);
  // Nothing to bring, so no "To keep you safe" line (ui-check-471 Later 2).
  assert.doesNotMatch(out, /To keep you safe/);
  // With bots, the untick line and the safety line stay.
  const b = demo.githubBotsPreview();
  const withBots = html(m.PreviewView, { preview: b, github: b.github, skip: new Set(), onToggle() {} });
  assert.match(withBots, /3 bots will be created\. Untick any you don’t want\./);
  assert.match(withBots, /To keep you safe, imported bots start with no tools/);
});

test('item 11: the phone header has one horizontal padding (px-2), desktop keeps px-4', async () => {
  const header = (d) => d.dlg().querySelector('#import-title').parentElement.className.split(/\s+/);
  let d = await open();
  assert.ok(header(d).includes('px-4') && !header(d).includes('px-2'), header(d).join(' '));
  cleanup.forEach((f) => f());
  cleanup = [];
  const was = g.window.matchMedia;
  g.window.matchMedia = (q) => ({ matches: true, media: q, addEventListener() {}, removeEventListener() {} });
  try {
    d = await open();
    const c = header(d);
    assert.ok(c.includes('h-14') && c.includes('px-2') && !c.includes('px-4'), c.join(' '));
  } finally {
    g.window.matchMedia = was;
  }
});

test('ui-check-471 must-fix: on phone the title gets pl-2 on Choose and Done (no Back), not on other steps', async () => {
  const cls = (d) => d.dlg().querySelector('#import-title').className.split(/\s+/);
  // Desktop: never pl-2.
  let d = await open();
  assert.ok(!cls(d).includes('pl-2'), cls(d).join(' '));
  cleanup.forEach((f) => f());
  cleanup = [];
  const was = g.window.matchMedia;
  g.window.matchMedia = (q) => ({ matches: true, media: q, addEventListener() {}, removeEventListener() {} });
  try {
    const p = demo.githubBotsPreview();
    d = await open({ over: { 'POST /api/import/github/preview': () => json(p), 'POST /api/import/apply': () => json(demo.applied(p)) } });
    assert.ok(cls(d).includes('pl-2'), `choose: ${cls(d).join(' ')}`);
    await d.click(/^GitHub link/);
    assert.ok(!cls(d).includes('pl-2'), `github: ${cls(d).join(' ')}`);
    typeUrl(d.dlg, 'https://github.com/example-team/ai-team');
    await settle();
    primaryOf(d.dlg).click();
    await settle(12);
    assert.equal(primaryOf(d.dlg).textContent.trim(), 'Import 3 bots');
    assert.ok(!cls(d).includes('pl-2'), `preview: ${cls(d).join(' ')}`);
    primaryOf(d.dlg).click();
    await settle(12);
    assert.match(d.dlg().textContent, /Imported 3 bots/);
    assert.ok(cls(d).includes('pl-2'), `done: ${cls(d).join(' ')}`);
  } finally {
    g.window.matchMedia = was;
  }
});
