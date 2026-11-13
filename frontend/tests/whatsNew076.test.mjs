// 0.7.6 "What's new in kwant X.Y.Z" (next-076/whats-new/spec.md): the content checks, the show-once rules
// (an upgrade shows it, a brand-new person never sees it, an existing person with no key does, it waits
// while a dialog is open, once per version), the sheet, the update banner's button and Settings › About.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { readFileSync, readdirSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_default', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'MutationObserver', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const dir = new URL('../src/whatsnew/', import.meta.url);
const files = readdirSync(dir).filter((n) => n.endsWith('.json'));
const real = Object.fromEntries(files.map((n) => [n.slice(0, -5), JSON.parse(readFileSync(new URL(n, dir), 'utf8'))]));

const hl = (id, over = {}) => ({ id, title: `Title ${id}`, text: `Text for ${id}.`, image: null, open: null, ...over });
const v076 = {
  version: '0.7.6',
  date: '2026-11-14',
  highlights: [hl('recipe', { open: 'settings/teams' }), hl('goals'), hl('phone')],
  fixes: ['Fix one.', 'Fix two.', 'Fix three.', 'Fix four.', 'Fix five.', 'Fix six.'],
};
const content = {
  entries: [real['0.7.5'], v076],
  changelog: { '0.7.6': '### Added\n- **Recipe cards.** Example text.', '0.7.5': '### Fixed\n- Older.' },
};

const s = await loadBundled(`export * from './src/whatsNewSheet';`);
const ui = await loadBundled(
  `
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { WhatsNewHost, WhatsNewSheetView, hasWhatsNew } from './src/components/WhatsNewSheet';
  export { VersionCardView } from './src/components/VersionCard';
  export { AppUpdateBannerView } from './src/components/AppUpdateBanner';
  export { openWhatsNew, SEEN_KEY } from './src/whatsNewSheet';
  export { WHATS_NEW } from './src/whatsNewContent';
`,
  { whatsNew: content },
);
const { React, createRoot } = ui;
// React is bundled as a production build (no act): render, then let effects and timers run.
const act = async (fn) => {
  await fn();
};

const memStore = (init = {}) => {
  const m = new Map(Object.entries(init));
  return { m, getItem: (k) => m.get(k) ?? null, setItem: (k, v) => m.set(k, String(v)), removeItem: (k) => m.delete(k) };
};

// ---------- content ----------

test('validator: every bundled whatsnew file keeps the limits, its name matches its version, and every "open" is an app place', () => {
  assert.ok(files.includes('0.7.5.json'), 'the 0.7.5 backfill ships');
  for (const [v, e] of Object.entries(real)) {
    assert.deepEqual(s.validateEntry(e, v), [], `${v}.json`);
    for (const h of e.highlights) if (h.open) assert.ok(s.highlightView(h), `${v} ${h.id}: ${h.open}`);
  }
  assert.equal(real['0.7.5'].highlights.find((h) => h.id === 'teams').open, 'settings/teams');
  assert.equal(real['0.7.5'].highlights.find((h) => h.id === 'telegram').open, 'settings/apps');
});

test('validator: names each broken limit', () => {
  const ok = { version: '0.7.6', date: '2026-11-14', highlights: [hl('a'), hl('b')], fixes: [] };
  assert.deepEqual(s.validateEntry(ok), []);
  const bad = (over) => s.validateEntry({ ...ok, ...over });
  assert.match(bad({ highlights: [hl('a')] })[0], /highlights: 1, needs 2–6/);
  assert.match(bad({ highlights: 'abcdefg'.split('').map((x) => hl(x)) })[0], /highlights: 7/);
  assert.match(bad({ highlights: [hl('a', { title: 'x'.repeat(61) }), hl('b')] })[0], /title is 61 characters, max 60/);
  assert.deepEqual(bad({ highlights: [hl('a', { title: 'é'.repeat(60) }), hl('b')] }), [], 'characters, not bytes');
  assert.match(bad({ highlights: [hl('a', { text: 'x'.repeat(201) }), hl('b')] })[0], /text is 201 characters, max 200/);
  assert.match(bad({ highlights: [hl('a'), hl('a')] })[0], /used twice/);
  assert.match(bad({ highlights: [hl('a', { image: 'a.png' }), hl('b')] })[0], /\.webp/);
  assert.match(bad({ highlights: [hl('a', { open: 'settings/nope' }), hl('b')] })[0], /in-app place/);
  assert.match(bad({ highlights: [hl('a', { open: 'members' }), hl('b')] })[0], /in-app place/, 'old aliases are not allowed');
  assert.match(bad({ fixes: Array(16).fill('x') })[0], /fixes: 16, max 15/);
  assert.match(bad({ fixes: ['x'.repeat(141)] })[0], /fixes\[0\] is 141 characters, max 140/);
  assert.match(bad({ version: '0.7' })[0], /X\.Y\.Z/);
  assert.match(s.validateEntry(ok, '0.7.5')[0], /doesn't match the file name/);
  assert.deepEqual(s.validateEntry([]), ['not a JSON object']);
});

test('the newest good file is the one shown; broken ones are left out', () => {
  const list = s.sortEntries([real['0.7.5'], { ...v076, version: '0.7.10' }, v076, { version: '9.9.9' }]);
  assert.deepEqual(list.map((e) => e.version), ['0.7.10', '0.7.6', '0.7.5']);
  assert.equal(s.compareVersions('0.7.10', '0.7.9'), 1);
  assert.equal(s.compareVersions('0.7.5', '0.7.5'), 0);
  assert.equal(s.compareVersions('0.7.5', null), 1);
  assert.deepEqual(ui.WHATS_NEW.map((e) => e.version), ['0.7.6', '0.7.5']);
});

test('All changes is only that version’s CHANGELOG section', () => {
  const log = '# Changelog\n\n## Unreleased\n- soon\n\n## [Mac 0.7.6] - 2026-11-14\n\n### Added\n- New.\n\n## [Mac 0.7.5] - 2026-11-07\n- Old.\n';
  assert.equal(s.changelogSection(log, '0.7.6'), '### Added\n- New.');
  assert.equal(s.changelogSection(log, '0.7.5'), '- Old.');
  assert.equal(s.changelogSection(log, '0.7.4'), null);
  const repo = readFileSync(new URL('../../CHANGELOG.md', import.meta.url), 'utf8');
  assert.match(s.changelogSection(repo, '0.7.5'), /^### Added\n- \*\*This Mac and your servers, side by side\.\*\*/);
});

// ---------- show once: the rules ----------

test('show-once: an upgrade (a newer version than this device saw) shows it', () => {
  assert.equal(s.decideOnLoad('0.7.5', '0.7.6', null), 'show', "doesn't wait on the account check");
  assert.equal(s.decideOnLoad('0.7.5', '0.7.6', false), 'show');
});

test('show-once: a brand-new person never sees it; the version is recorded silently', () => {
  assert.equal(s.decideOnLoad(null, '0.7.6', false), 'record');
  const store = memStore();
  s.markSeen('0.7.6', store);
  assert.equal(s.decideOnLoad(s.seenVersion(store), '0.7.6', true), 'none', 'and not later either');
});

test('show-once: an existing person with no key yet (upgrading from 0.7.5) does see it', () => {
  assert.equal(s.decideOnLoad(null, '0.7.6', true), 'show');
  assert.equal(s.decideOnLoad(null, '0.7.6', null), 'wait', 'until the space has loaded');
});

test('show-once: once per version, per device; the stored version never moves back', () => {
  const store = memStore();
  s.markSeen('0.7.6', store);
  assert.equal(store.m.get('kwant:whatsnew:seen'), '0.7.6');
  assert.equal(s.decideOnLoad(s.seenVersion(store), '0.7.6', true), 'none');
  s.markSeen('0.7.5', store);
  assert.equal(store.m.get('kwant:whatsnew:seen'), '0.7.6', 'an older tab cannot reset it');
  assert.equal(s.decideOnLoad(s.seenVersion(store), '0.7.7', true), 'show', 'the next version shows again');
  assert.equal(s.decideOnLoad('0.7.6', null, true), 'none', 'nothing bundled');
});

test('otherDialogOpen sees modals, alerts and open <dialog>s, but not the sheet itself', () => {
  const d = new JSDOM('<body><div data-whats-new><div role="dialog" aria-modal="true"></div></div></body>').window.document;
  assert.equal(s.otherDialogOpen(d), false);
  d.body.insertAdjacentHTML('beforeend', '<div role="alertdialog"></div>');
  assert.equal(s.otherDialogOpen(d), true);
  const d2 = new JSDOM('<body><dialog open></dialog></body>').window.document;
  assert.equal(s.otherDialogOpen(d2), true);
});

// ---------- show once: in the app ----------

let mounted = [];
afterEach(async () => {
  for (const r of mounted) await act(async () => r.unmount());
  mounted = [];
  document.body.innerHTML = '';
  localStorage.clear();
});
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
async function host(props) {
  const el = document.createElement('div');
  document.body.append(el);
  const root = createRoot(el);
  mounted.push(root);
  const render = async (p) => {
    await act(async () => root.render(React.createElement(ui.WhatsNewHost, { onOpen: () => {}, ...props, ...p })));
    await settle();
  };
  await render({});
  return render;
}
const sheet = () => document.querySelector('[data-whats-new] [role="dialog"]');
const click = async (el) => {
  await act(async () => el.dispatchEvent(new MouseEvent('click', { bubbles: true })));
  await settle();
};

test('host: an upgrade shows "What’s new in kwant bots 0.7.6" once, "Updated from 0.7.5", and stores the version', async () => {
  localStorage.setItem(ui.SEEN_KEY, '0.7.5');
  await host({ established: null });
  assert.equal(sheet()?.querySelector('h2')?.textContent, 'What’s new in kwant bots 0.7.6');
  assert.match(document.querySelector('[data-whats-new-eyebrow]').textContent, /^Updated from 0\.7\.5 · Nov 14$/);
  assert.equal(localStorage.getItem(ui.SEEN_KEY), '0.7.6');
  assert.equal(document.activeElement, sheet(), 'focus moves into the sheet');
  await click(document.querySelector('[data-whats-new-done]'));
  assert.equal(sheet(), null, 'Got it closes it');
  await host({ established: true });
  assert.equal(sheet(), null, 'and the next load shows nothing');
});

test('host: a brand-new person never sees it, and the version is recorded', async () => {
  const render = await host({ established: null });
  assert.equal(sheet(), null, 'waits while unknown');
  assert.equal(localStorage.getItem(ui.SEEN_KEY), null);
  await render({ established: false });
  assert.equal(sheet(), null);
  assert.equal(localStorage.getItem(ui.SEEN_KEY), '0.7.6');
  await render({ established: true });
  assert.equal(sheet(), null, 'adding an account later does not bring it up');
});

test('host: an existing person with no key sees it, with "Updated" and no earlier version named', async () => {
  const render = await host({ established: null });
  assert.equal(sheet(), null);
  await render({ established: true });
  assert.ok(sheet());
  assert.match(document.querySelector('[data-whats-new-eyebrow]').textContent, /^Updated · Nov 14$/);
});

test('host: waits while another dialog is open, then opens when it closes', async () => {
  localStorage.setItem(ui.SEEN_KEY, '0.7.5');
  const other = document.createElement('div');
  other.setAttribute('role', 'dialog');
  other.setAttribute('aria-modal', 'true');
  document.body.append(other);
  await host({ established: true });
  assert.equal(sheet(), null, 'not on top of a dialog');
  assert.equal(localStorage.getItem(ui.SEEN_KEY), '0.7.5', 'not counted as seen yet');
  const alert = document.createElement('div');
  alert.setAttribute('role', 'alertdialog');
  document.body.append(alert);
  other.remove();
  await settle();
  assert.equal(sheet(), null, 'an approval still blocks it');
  alert.remove();
  await settle();
  assert.ok(sheet(), 'opens once nothing is open');
  assert.equal(localStorage.getItem(ui.SEEN_KEY), '0.7.6');
});

test('host: the banner and Settings open a version by hand; Show me opens the place and closes the sheet', async () => {
  localStorage.setItem(ui.SEEN_KEY, '0.7.6');
  const opened = [];
  await host({ established: true, onOpen: (v) => opened.push(v) });
  assert.equal(sheet(), null);
  await act(async () => ui.openWhatsNew('0.7.5'));
  await settle();
  assert.equal(sheet()?.querySelector('h2')?.textContent, 'What’s new in kwant bots 0.7.5');
  assert.equal(document.querySelector('[data-whats-new-eyebrow]').textContent, 'Released Nov 7', 'opened by hand: Released + date');
  const show = [...sheet().querySelectorAll('button')].find((b) => b.getAttribute('aria-label') === 'Show me: Ready-made teams');
  await click(show);
  assert.deepEqual(opened, [{ kind: 'settings', scope: 'space', tab: 'teams' }]);
  assert.equal(sheet(), null);
  await act(async () => ui.openWhatsNew('0.6.0'));
  await settle();
  assert.equal(sheet(), null, 'a version with no file opens nothing');
});

// ---------- the sheet ----------

const view = (props) =>
  ui.renderToStaticMarkup(
    React.createElement(ui.WhatsNewSheetView, { entry: v076, changelog: content.changelog['0.7.6'], phone: false, updatedFrom: '0.7.5', onClose() {}, onOpen() {}, ...props }),
  );

test('sheet: highlights, Show me only where there is a place, folded fixes, one main button', () => {
  const html = view({});
  assert.match(html, /role="dialog" aria-modal="true"/);
  assert.equal((html.match(/data-highlight=/g) ?? []).length, 3);
  assert.equal((html.match(/>Show me /g) ?? []).length, 1);
  assert.match(html, /Smaller changes and fixes: 6/);
  assert.equal((html.match(/Fix (one|two|three|four|five|six)\./g) ?? []).length, 3, 'more than 4 fixes: 3 shown');
  assert.match(html, /Show 3 more/);
  assert.equal((html.match(/Got it/g) ?? []).length, 1);
  assert.match(html, /All changes/);
  const four = view({ entry: { ...v076, fixes: v076.fixes.slice(0, 4) } });
  assert.equal((four.match(/Fix (one|two|three|four)\./g) ?? []).length, 4, '4 fixes: not folded');
  assert.doesNotMatch(four, /more/);
  assert.doesNotMatch(view({ entry: { ...v076, fixes: [] } }), /Smaller changes/);
  assert.doesNotMatch(view({ changelog: null }), /All changes/, 'no CHANGELOG section: no All changes');
});

test('sheet: All changes renders the CHANGELOG section as markdown, with the GitHub release link', () => {
  const html = view({ initialPage: 'all' });
  assert.match(html, /All changes in 0\.7\.6/);
  assert.match(html, /<strong[^>]*>Recipe cards\.<\/strong>/);
  assert.match(html, /href="https:\/\/github\.com\/alexsergivan\/kwant\/releases\/tag\/mac-v0\.7\.6"/);
  assert.match(html, /What’s new<\/button>/, 'a way back');
});

test('sheet: phone is a bottom sheet with 44 px+ targets', () => {
  const html = view({ phone: true });
  assert.match(html, /flex items-end/);
  assert.match(html, /rounded-t-\[20px\]/);
  assert.match(html, /max-h-\[88vh\]/);
  assert.match(html, /h-11 w-11/, 'close');
  assert.match(html, /h-12 min-w-\[120px\][^"]*flex-1/, 'Got it fills the row');
});

test('sheet: the first picture leads, the others are thumbnails, all from /whatsnew/<version>/', () => {
  const entry = { ...v076, highlights: [hl('a'), hl('b', { image: 'b.webp' }), hl('c', { image: 'c.webp' })] };
  const html = view({ entry });
  assert.match(html, /src="\/whatsnew\/0\.7\.6\/b\.webp" alt="" class="mb-3 block aspect-\[2\/1\]/);
  assert.match(html, /src="\/whatsnew\/0\.7\.6\/c\.webp" alt="" class="mt-1 block aspect-\[16\/10\]/);
});

// ---------- the banner and Settings › About ----------

test('banner: "See what’s new" on the "Updated to" bar only, and only when that version has a sheet', () => {
  const banner = (v, onWhatsNew) => ui.renderToStaticMarkup(React.createElement(ui.AppUpdateBannerView, { view: v, onUpdate() {}, onDismiss() {}, onWhatsNew }));
  assert.match(banner({ kind: 'updated', version: '0.7.6' }, () => {}), /Updated to kwant bots 0\.7\.6\..*data-see-whats-new[^>]*>See what’s new</s);
  assert.doesNotMatch(banner({ kind: 'updated', version: '0.7.6' }), /See what’s new/);
  assert.equal(ui.hasWhatsNew('0.7.6'), true);
  assert.equal(ui.hasWhatsNew('0.7.4'), false);
});

test('Settings › kwant: What’s new in the running version opens its sheet; a newer one opens its release notes', async () => {
  const model = (available) => ({
    status: { current: '0.7.6', canInstall: false, lastChecked: Date.now(), how: null, state: available ? 'available' : 'idle', available, progress: null, error: null },
    unsupported: false,
    current: '0.7.6',
    phase: 'idle',
    target: null,
    actionError: null,
    installError: null,
    checkedAt: 1,
  });
  const props = (available) => ({
    model: model(available),
    now: Date.now(),
    onCheck() {},
    onInstall() {},
    onWhatsNew: (v) => ui.openWhatsNew(v),
    onReleaseNotes: (v, notes, date) => ui.openWhatsNew(v, { notes, date }),
  });
  const html = ui.renderToStaticMarkup(React.createElement(ui.VersionCardView, props(null)));
  assert.match(html, /What’s new in 0\.7\.6<\/b><span[^>]*>Nov 14 · 3 new things, 6 fixes/);
  assert.doesNotMatch(html, /Earlier versions|0\.7\.5/);
  assert.match(html, /All releases on GitHub/);

  localStorage.setItem(ui.SEEN_KEY, '0.7.6');
  await host({ established: true });
  const el = document.createElement('div');
  document.body.append(el);
  const root = createRoot(el);
  mounted.push(root);
  await act(async () => root.render(React.createElement(ui.VersionCardView, props(null))));
  await settle();
  await click(el.querySelector('button[data-whats-new-current="0.7.6"]'));
  assert.equal(sheet()?.querySelector('h2')?.textContent, 'What’s new in kwant bots 0.7.6');
  await click(sheet().querySelector('[data-whats-new-done]'));
  assert.equal(sheet(), null);

  // A version this app doesn't bundle: only its release notes, no way "back" to a sheet it doesn't have
  const next = { version: '9.9.9', notes: '### Added\n- **Faster bots.**', date: '2026-11-12T20:49:18Z', url: 'https://github.com/alexsergivan/kwant/releases/tag/mac-v9.9.9' };
  await act(async () => root.render(React.createElement(ui.VersionCardView, props(next))));
  await settle();
  await click(el.querySelector('button[data-whats-new-next="9.9.9"]'));
  assert.equal(sheet()?.querySelector('h2')?.textContent, 'What’s new in kwant bots 9.9.9');
  assert.match(sheet().textContent, /Released Nov 12/);
  assert.match(sheet().textContent, /Faster bots\./);
  assert.doesNotMatch(sheet().textContent, /All changes in/);
});
