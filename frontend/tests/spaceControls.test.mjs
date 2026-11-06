// 0.7.2 space controls (dev-mode-explainer spec §1–§5): the Dev mode and Approvals rows under the space
// name, the §2 sub-line priority, the "?" explainer (popover on desktop, sheet on phone) and the switch v3
// wiring. Demo data only.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_demo', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;

const off = { on: false, repo: null, branch: null, deploy_hint: null, backup_command: null, last_seen_sha: null, polled_at: null, poll_error: null, calls_today: null, calls_cap: null };
const R = (repo, status, extra = {}) => ({ repo, host: 'github.com', status, branch: 'main', team: 'Ship', from: 'repo', added_by: 'team', ...extra });
let server = { state: { ...off }, calls: [] };
g.fetch = g.window.fetch = async (url, init = {}) => {
  const u = String(url);
  const method = init.method ?? 'GET';
  const body = init.body ? JSON.parse(init.body) : undefined;
  server.calls.push([method, u.split('?')[0], body]);
  const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'Content-Type': 'application/json' } });
  if (server.fail && server.fail.method === method && u.includes(server.fail.path)) return json({ error: 'nope' }, server.fail.status);
  if (u.includes('/dev_mode')) {
    if (method === 'PUT') server.state = { ...server.state, ...body };
    return json(server.state);
  }
  return json({});
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { devSubLine, devListPending, parseDevMode, devStore, DEV_GUIDE_URL, EXPLAINER } from './src/devMode';
  export { approvalsRow } from './src/approvalMode';
  export { SpaceControls, SHEET_CLOSE_MS, APPROVALS_WAIT_MS } from './src/components/SpaceControls';
`);
const { React, createRoot } = m;
const h = React.createElement;
const ourEntry = () => !!window.history.state?.kwBrowserOverlay;
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const settle = async (n = 10) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const reset = (state = off) => {
  server = { state: { ...state }, calls: [], fail: null };
  m.devStore._reset();
  document.body.innerHTML = '';
};
const mount = async (props) => {
  const host = document.createElement('div');
  host.setAttribute('data-kw-sidebar', '');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(h(m.SpaceControls, { space: { id: 'spc_demo', name: 'Demo', approvals: 'ask' }, bots: [], onOpenSettings() {}, ...props }));
  await settle();
  return { host, root };
};
const click = async (el) => {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};
const key = async (el, k, o = {}) => {
  el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, ...o }));
  await settle();
};
const text = (sub) => sub.parts.map((p) => p.text).join('');
const S = (o) => m.parseDevMode({ ...off, on: true, polled_at: 'x', status: 'watching', ...o });

// ---------------------------------------------------------------- §2 sub-line

test('§2 sub-line: one repo names it and its branch; several count; partial and errors say so', () => {
  const one = m.devSubLine(S({ repos: [R('demo-org/app', 'watching', { branch: 'trunk' })] }));
  assert.equal(one.state, 'watching');
  assert.equal(text(one), 'Watching demo-org/app · trunk');
  assert.equal(one.led, 'ok');
  assert.equal(one.action, null);
  assert.deepEqual(one.parts.filter((p) => p.repo).map((p) => p.text), ['demo-org/app']);
  const noBranch = m.devSubLine(S({ repos: [R('demo-org/app', 'watching', { branch: null })] }));
  assert.equal(text(noBranch), 'Watching demo-org/app · main');
  const three = m.devSubLine(S({ repos: [R('demo-org/app', 'watching'), R('demo-org/api', 'watching'), R('demo-org/web', 'watching')] }));
  assert.equal(text(three), 'Watching 3 repos');
  assert.equal(three.action, 'repos');
  const partial = m.devSubLine(S({ repos: [R('demo-org/app', 'watching'), R('demo-org/api', 'error')] }));
  assert.deepEqual([partial.state, text(partial), partial.led, partial.action], ['partial', '1 of 2 watched', 'warn', 'repos']);
  const err1 = m.devSubLine(S({ repos: [R('demo-org/app', 'error')] }));
  assert.deepEqual([text(err1), err1.led], ['Can’t read a repo', 'bad']);
  assert.equal(text(m.devSubLine(S({ repos: [R('demo-org/app', 'error'), R('demo-org/api', 'error')] }))), 'Can’t read 2 repos');
});

test('§2 sub-line: GitHub not connected, other hosts, a local folder, no repo, never a checking line', () => {
  const nc = m.devSubLine(S({ repos: [R('demo-org/app', 'not_connected')] }));
  assert.deepEqual([text(nc), nc.led, nc.action], ['Needs GitHub', 'warn', 'connect']);
  const gl = m.devSubLine(S({ repos: [R('gitlab.example.test/demo/app', 'other_host', { host: 'gitlab.example.test' })] }));
  assert.equal(text(gl), 'gitlab.example.test isn’t read yet');
  assert.equal(gl.led, 'off');
  const local = m.devSubLine(S({ repos: [R('app', 'other_host', { host: null, from: 'local' })] }));
  assert.equal(text(local), 'Local folder, no remote');
  const none = m.devSubLine(S({ repos: [], status: 'no_repo' }));
  assert.deepEqual([none.state, text(none), none.action], ['no_repo', 'No repo yet', 'link']);
  // Answer 1 + addendum 3: no "checking" words. Until the first repo list arrives the line is a skeleton,
  // never "No repo yet": no state yet, on with neither a status nor a repo, or a press on still saving.
  for (const [s, o] of [[null, { loading: true }], [m.parseDevMode({ ...off, on: true, repos: [] }), {}], [m.parseDevMode(off), { turningOn: true }]]) {
    const l = m.devSubLine(s, o);
    assert.deepEqual([l.state, text(l), l.led, l.action], ['loading', '', null, null]);
  }
  assert.equal(m.devSubLine(null, { disabled: true }).state, 'disabled', 'not the owner never waits on a list');
  assert.equal(m.devSubLine(m.parseDevMode(off)).state, 'off');
  assert.equal(m.devSubLine(null).state, 'off', 'a failed first load stops waiting');
});

test('§2 sub-line: armed beats everything, off says what it is for, not-owner is its own state', () => {
  const s = S({ repos: [R('demo-org/app', 'error')] });
  assert.deepEqual([m.devSubLine(s, { armed: true }).state, text(m.devSubLine(s, { armed: true }))], ['armed', 'Press again to turn off']);
  assert.equal(text(m.devSubLine(m.parseDevMode(off))), 'For bots that write code');
  assert.equal(text(m.devSubLine(null)), 'For bots that write code');
  assert.equal(m.devSubLine(null, { disabled: true }).state, 'disabled');
});

// ---------------------------------------------------------------- Approvals row

test('approvals row: chip and sub-line per §2, no row on an older server, never counts bots that ask', () => {
  const b = (e) => ({ approvals_effective: e });
  assert.equal(m.approvalsRow(undefined, [], false), null);
  const ask = m.approvalsRow('ask', [b('ask')], false);
  assert.deepEqual([ask.chip, ask.tone, ask.sub, ask.title], ['Ask', 'neutral', null, null]);
  assert.equal(ask.label, 'Approvals: Ask. Change in space settings');
  assert.equal(m.approvalsRow('smart', [b('off'), b('off'), b('smart')], false).sub, 'Off for 2 bots');
  assert.equal(m.approvalsRow('ask', [b('off')], false).sub, 'Off for 1 bot');
  const offRow = m.approvalsRow('off', [b('off')], false);
  assert.deepEqual([offRow.chip, offRow.tone, offRow.sub], ['Off', 'warn', 'Bots act without asking']);
  assert.match(offRow.title, /Payments still ask you\./);
  assert.doesNotMatch(offRow.title, /With Dev mode on/);
  const offDev = m.approvalsRow('off', [b('off')], true);
  assert.equal(offDev.sub, 'Main and deploys still ask');
  assert.match(offDev.title, /With Dev mode on, pushes to main, deploys and deleting data still ask\./);
  assert.equal(m.approvalsRow('off', [b('off'), b('ask')], true).sub, '1 bot still asks');
  assert.equal(m.approvalsRow('off', [b('ask'), b('smart')], false).sub, '2 bots still ask');
});

// ---------------------------------------------------------------- DOM

test('DOM: two labelled rows; the switch is named by its label and described by the sub-line', async () => {
  reset({ ...off, on: true, polled_at: 'x', status: 'watching', repos: [R('demo-org/app', 'watching')] });
  const { host, root } = await mount({});
  const group = host.querySelector('[data-space-controls]');
  assert.equal(group.getAttribute('role'), 'group');
  assert.equal(group.getAttribute('aria-label'), 'Space controls');
  assert.deepEqual([...group.querySelectorAll('[data-sc-row]')].map((r) => r.dataset.scRow), ['dev', 'approvals']);
  const sw = host.querySelector('[role="switch"]');
  assert.equal(document.getElementById(sw.getAttribute('aria-labelledby')).textContent, 'Dev mode');
  assert.equal(document.getElementById(sw.getAttribute('aria-describedby')).textContent, 'Watching demo-org/app · main');
  assert.equal(sw.getAttribute('aria-checked'), 'true');
  assert.equal(host.querySelector('[data-dev-sub]').dataset.devSub, 'watching');
  assert.equal(!!host.querySelector('[data-dev-sub-action]'), false, 'one repo: no See repos');
  const value = host.querySelector('[data-approvals-value]');
  assert.equal(value.getAttribute('aria-label'), 'Approvals: Ask. Change in space settings');
  root.unmount();
});

test('DOM: three repos say Watching 3 repos with See repos; the Approvals value opens settings', async () => {
  reset({ ...off, on: true, polled_at: 'x', status: 'watching', repos: [R('demo-org/app', 'watching'), R('demo-org/api', 'watching'), R('demo-org/web', 'watching')] });
  let seen = 0;
  let settings = 0;
  const { host, root } = await mount({ onSeeRepos: () => seen++, onOpenSettings: () => settings++ });
  assert.equal(host.querySelector('[data-dev-sub-text]').textContent, 'Watching 3 repos');
  const act = host.querySelector('[data-dev-sub-action]');
  assert.equal(act.textContent, 'See repos');
  assert.equal(act.tagName, 'BUTTON');
  await click(act);
  assert.equal(seen, 1);
  await click(host.querySelector('[data-approvals-value]'));
  assert.equal(settings, 1);
  root.unmount();
});

test('DOM: armed off shows Off?, the sub-line Press again to turn off with an Esc hint on desktop; Escape keeps it on', async () => {
  reset({ ...off, on: true, polled_at: 'x', status: 'watching', repos: [R('demo-org/app', 'watching')] });
  const { host, root } = await mount({});
  const sw = host.querySelector('[role="switch"]');
  await click(sw);
  assert.equal(sw.querySelector('[data-dv-word]').textContent, 'Off?');
  const sub = host.querySelector('[data-dev-sub]');
  assert.equal(sub.dataset.devSub, 'armed');
  assert.equal(host.querySelector('[data-dev-sub-text]').textContent, 'Press again to turn off');
  assert.equal(sub.querySelector('kbd')?.textContent, 'Esc');
  await key(sw, 'Escape');
  assert.equal(host.querySelector('[data-dev-sub]').dataset.devSub, 'watching');
  assert.equal(server.calls.filter(([meth]) => meth === 'PUT').length, 0);
  root.unmount();
});

test('DOM: the "?" opens the popover on desktop, focus goes to Turn on, Escape closes it back to "?"', async () => {
  reset({ ...off });
  const { host, root } = await mount({});
  const q = host.querySelector('[data-dv-help]');
  assert.equal(q.getAttribute('aria-label'), 'What is Dev mode?');
  assert.equal(q.getAttribute('aria-haspopup'), 'dialog');
  assert.equal(q.getAttribute('aria-expanded'), 'false');
  await click(q);
  assert.equal(q.getAttribute('aria-expanded'), 'true');
  const pop = document.querySelector('[data-dev-explainer]');
  assert.equal(pop.dataset.devExplainer, 'popover');
  assert.equal(pop.id, q.getAttribute('aria-controls'));
  assert.equal(pop.getAttribute('role'), 'dialog');
  assert.equal(pop.hasAttribute('aria-modal'), false, 'non-modal on desktop');
  assert.equal(document.getElementById(pop.getAttribute('aria-labelledby')).textContent, m.EXPLAINER.title);
  assert.equal(pop.querySelectorAll('[data-dev-feature]').length, 3);
  const guide = pop.querySelector('[data-dvx-guide]');
  assert.equal(guide.getAttribute('href'), 'https://kwant.si/docs/dev-mode.html');
  assert.equal(guide.getAttribute('target'), '_blank');
  assert.match(guide.getAttribute('rel'), /noopener/);
  assert.match(guide.textContent, /opens in a new tab/);
  const primary = pop.querySelector('[data-dvx-primary]');
  assert.equal(primary.dataset.dvxPrimary, 'on');
  assert.equal(document.activeElement === primary, true, 'focus lands on Turn on Dev mode');
  await key(primary, 'Escape');
  await settle();
  assert.equal(!!document.querySelector('[data-dev-explainer]'), false);
  assert.equal(q.getAttribute('aria-expanded'), 'false');
  assert.equal(document.activeElement === q, true, 'focus back on "?"');
  root.unmount();
});

test('DOM: Turn on in the explainer presses the switch once (PUT {on:true}) and closes', async () => {
  reset({ ...off });
  let turnedOn = 0;
  const { host, root } = await mount({ onTurnedOn: () => turnedOn++ });
  await click(host.querySelector('[data-dv-help]'));
  await click(document.querySelector('[data-dvx-primary="on"]'));
  await settle(20);
  assert.deepEqual(server.calls.filter(([meth]) => meth === 'PUT').map(([, , b]) => b), [{ on: true }]);
  assert.equal(turnedOn, 1);
  assert.equal(!!document.querySelector('[data-dev-explainer]'), false);
  assert.equal(host.querySelector('[role="switch"]').getAttribute('aria-checked'), 'true');
  root.unmount();
});

test('DOM: when on, the explainer offers Open Dev desk; an outside press closes it', async () => {
  reset({ ...off, on: true, polled_at: 'x', status: 'watching', repos: [R('demo-org/app', 'watching')] });
  let desk = 0;
  const { host, root } = await mount({ onOpenDevDesk: () => desk++ });
  await click(host.querySelector('[data-dv-help]'));
  const primary = document.querySelector('[data-dvx-primary]');
  assert.equal(primary.dataset.dvxPrimary, 'desk');
  assert.equal(primary.textContent, 'Open Dev desk');
  await click(primary);
  assert.equal(desk, 1);
  if (document.querySelector('[data-dev-explainer]')) {
    document.body.dispatchEvent(new window.Event('pointerdown', { bubbles: true }));
    await settle();
  }
  assert.equal(!!document.querySelector('[data-dev-explainer]'), false);
  root.unmount();
});

test('DOM: not the owner (403): the switch is aria-disabled, the explainer says who can turn it on', async () => {
  reset({ ...off });
  server.fail = { method: 'GET', path: '/dev_mode', status: 403 };
  const { host, root } = await mount({});
  const sw = host.querySelector('[role="switch"]');
  assert.equal(sw.getAttribute('aria-disabled'), 'true');
  await click(host.querySelector('[data-dv-help]'));
  const pop = document.querySelector('[data-dev-explainer]');
  assert.equal(!!pop.querySelector('[data-dvx-primary]'), false);
  assert.equal(pop.querySelector('[data-dvx-not-owner]').textContent, m.EXPLAINER.notOwner);
  root.unmount();
});

test('DOM: on a phone the explainer is a modal sheet; the page behind is inert; the X closes it', async () => {
  reset({ ...off });
  const { host, root } = await mount({ phone: true });
  assert.ok(host.querySelector('[data-space-controls]').classList.contains('kw-sc-phone'));
  await click(host.querySelector('[data-dv-help]'));
  const sheet = document.querySelector('[data-dev-explainer]');
  assert.equal(sheet.dataset.devExplainer, 'sheet');
  assert.equal(sheet.getAttribute('aria-modal'), 'true');
  assert.equal(host.hasAttribute('inert'), true, 'the sidebar behind is inert');
  assert.ok(document.querySelector('[data-dvx-scrim]'));
  await click(sheet.querySelector('[data-dvx-close]'));
  await wait(m.SHEET_CLOSE_MS + 40);
  assert.equal(!!document.querySelector('[data-dev-explainer]'), false);
  assert.equal(host.hasAttribute('inert'), false);
  root.unmount();
});

// ---------------------------------------------------------------- Ada's answers 1, 3 and 4 (0.7.2)

test('answer 4: the phone sheet slides down for 150 ms before it goes; the X takes its history entry off', async () => {
  reset({ ...off });
  const { host, root } = await mount({ phone: true });
  assert.equal(ourEntry(), false);
  await click(host.querySelector('[data-dv-help]'));
  assert.equal(ourEntry(), true, 'opening the sheet pushes a history entry');
  const sheet = document.querySelector('[data-dev-explainer]');
  await click(sheet.querySelector('[data-dvx-close]'));
  assert.equal(sheet.dataset.closing, 'true', 'the sheet is sliding down');
  assert.equal(document.querySelector('[data-dvx-scrim]').dataset.closing, 'true', 'the scrim fades with it');
  assert.ok(document.querySelector('[data-dev-explainer]'), 'still there during the slide');
  await wait(m.SHEET_CLOSE_MS + 40);
  assert.equal(!!document.querySelector('[data-dev-explainer]'), false);
  await wait(30);
  assert.equal(ourEntry(), false, 'closing with the X took the entry off');
  root.unmount();
});

test('answer 4: the back gesture closes the phone sheet', async () => {
  reset({ ...off });
  const { host, root } = await mount({ phone: true });
  await click(host.querySelector('[data-dv-help]'));
  assert.ok(document.querySelector('[data-dev-explainer]'));
  assert.equal(ourEntry(), true);
  window.history.back();
  await wait(m.SHEET_CLOSE_MS + 80);
  assert.equal(ourEntry(), false);
  assert.equal(!!document.querySelector('[data-dev-explainer]'), false);
  assert.equal(host.hasAttribute('inert'), false);
  root.unmount();
});

test('answer 4: under reduced motion the sheet closes at once; desktop pushes no history entry', async () => {
  reset({ ...off });
  const had = window.matchMedia;
  window.matchMedia = (q) => ({ matches: q.includes('reduce'), media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
  try {
    const { host, root } = await mount({ phone: true });
    await click(host.querySelector('[data-dv-help]'));
    await click(document.querySelector('[data-dvx-close]'));
    assert.equal(!!document.querySelector('[data-dev-explainer]'), false, 'gone with no slide');
    root.unmount();
  } finally {
    window.matchMedia = had;
  }
  await wait(30);
  const { host, root } = await mount({});
  await click(host.querySelector('[data-dv-help]'));
  assert.equal(ourEntry(), false, 'the desktop popover adds no history entry');
  root.unmount();
  const css = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
  assert.match(css, /\.kw-dvx-sheet\[data-closing\][^{]*\{[^}]*150ms/);
  assert.match(css, /prefers-reduced-motion: reduce\)\s*\{[^@]*\.kw-dvx-sheet\[data-closing\][^}]*animation: none/);
});

test('answer 3: while approvals load, the row is a same-height skeleton with no text', async () => {
  reset({ ...off });
  const { host, root } = await mount({ space: { id: 'spc_demo', name: 'Demo' } });
  const sk = host.querySelector('[data-sc-row="approvals"]');
  assert.ok(sk.hasAttribute('data-sc-skeleton'));
  assert.ok(sk.classList.contains('kw-sc-row'), 'same row class, so the same height');
  assert.equal(sk.getAttribute('aria-hidden'), 'true');
  assert.equal(sk.textContent, '');
  root.render(h(m.SpaceControls, { space: { id: 'spc_demo', name: 'Demo', approvals: 'ask' }, bots: [], onOpenSettings() {} }));
  await settle();
  const row = host.querySelector('[data-sc-row="approvals"]');
  assert.equal(row.hasAttribute('data-sc-skeleton'), false);
  assert.match(row.textContent, /Ask/);
  root.unmount();
});

test('answer 3: an older server never sends approvals, so the skeleton goes and no row is drawn', async () => {
  reset({ ...off });
  const { host, root } = await mount({ space: { id: 'spc_demo', name: 'Demo' } });
  assert.ok(host.querySelector('[data-sc-skeleton]'));
  await wait(m.APPROVALS_WAIT_MS + 60);
  assert.equal(!!host.querySelector('[data-sc-row="approvals"]'), false);
  root.unmount();
});

test('addendum 3: a same-height skeleton sub-line until the first repo list arrives; never a flash of "No repo yet"', async () => {
  reset({ ...off, on: true, status: 'no_repo', repos: [] });
  let release;
  const realFetch = g.fetch;
  g.fetch = g.window.fetch = async (url, init = {}) => {
    if ((init.method ?? 'GET') === 'GET' && String(url).includes('/dev_mode')) await new Promise((r) => (release = r));
    return realFetch(url, init);
  };
  const seen = [];
  const obs = new window.MutationObserver(() => seen.push(document.body.textContent));
  obs.observe(document.body, { subtree: true, childList: true, characterData: true });
  try {
    const { host, root } = await mount({});
    const sk = host.querySelector('[data-dev-sub]');
    assert.equal(sk.dataset.devSub, 'loading');
    assert.ok(sk.classList.contains('kw-sc-sub'), 'the sub-line class, so the same height');
    assert.equal(sk.getAttribute('aria-hidden'), 'true');
    assert.equal(sk.textContent, '');
    assert.ok(host.querySelector('[data-sc-skeleton] .kw-sc-skel-sub'));
    assert.ok(seen.every((t) => !/No repo yet/.test(t)), 'no "No repo yet" while the list loads');
    release();
    await settle(20);
    assert.equal(host.querySelector('[data-dev-sub]').dataset.devSub, 'no_repo');
    assert.match(host.querySelector('[data-dev-sub]').textContent, /No repo yet/);
    root.unmount();
  } finally {
    obs.disconnect();
    g.fetch = g.window.fetch = realFetch;
  }
});

test('addendum 3: a failed first load stops the skeleton (the off line shows), it never waits forever', async () => {
  reset({ ...off });
  server.fail = { method: 'GET', path: '/dev_mode', status: 500 };
  const { host, root } = await mount({});
  await settle(20);
  const sub = host.querySelector('[data-dev-sub]');
  assert.equal(sub.dataset.devSub, 'off');
  assert.equal(sub.textContent, 'For bots that write code');
  root.unmount();
});

test('answer 1: no checking sub-line; the switch is busy only while its PUT is in flight', async () => {
  reset({ ...off });
  const { host, root } = await mount({});
  const sw = host.querySelector('[role="switch"]');
  let release;
  const realFetch = g.fetch;
  g.fetch = g.window.fetch = async (url, init = {}) => {
    if ((init.method ?? 'GET') === 'PUT') await new Promise((r) => (release = r));
    return realFetch(url, init);
  };
  try {
    sw.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await settle();
    const dv = host.querySelector('[data-saving]');
    assert.ok(dv, 'the knob spins while saving');
    assert.equal(sw.getAttribute('aria-busy'), 'true');
    assert.doesNotMatch(host.textContent, /Looking for repos/);
    release();
    await settle(20);
  } finally {
    g.fetch = g.window.fetch = realFetch;
  }
  assert.equal(!!host.querySelector('[data-saving]'), false);
  assert.equal(!!host.querySelector('[aria-busy="true"]'), false);
  assert.doesNotMatch(host.textContent, /Looking for repos/);
  root.unmount();
});

test('copy: no emoji and no HTML from data in the controls or the explainer', async () => {
  reset({ ...off, on: true, polled_at: 'x', status: 'watching', repos: [R('demo-org/<b>x</b>', 'watching')] });
  const { host, root } = await mount({});
  assert.equal(!!host.querySelector('b'), false, 'a repo name is text, never markup');
  assert.match(host.querySelector('[data-dev-sub-text]').textContent, /demo-org\/<b>x<\/b>/);
  await click(host.querySelector('[data-dv-help]'));
  const all = host.textContent + document.querySelector('[data-dev-explainer]').textContent;
  assert.doesNotMatch(all, /\p{Extended_Pictographic}/u);
  root.unmount();
});
