// 32r (autonomy/ready-team-tile-comes-back/brief.md; Mira's 0.9.36 UX pass fix #3 and minors a, c).
// After Create, a ready-made team's tile leaves "Pick a team"; after Undo it comes straight back, with no reload:
// ReadyTeamsProvider asks the server for the catalogue again (its `used_in_space` is the truth). Also: a taken
// handle shows the same rose hint as a taken bot name and keeps Create off, and the Created toast stays 6 s.
// Settles on rendered state with a deadline, never on a fixed number of ticks. Red at main 97423d86.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_demo', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'CustomEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
window.matchMedia = (q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {} });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { setTeamsTransport } from './src/teams/teamTemplates';
  export { fixtureTransport } from './src/teams/suggestFixtures';
  export { TEMPLATES_WIRE, TEMPLATES_FIXTURE } from './src/teams/templateFixtures';
  export { TeamPreviewSheet } from './src/components/TeamPreviewSheet';
  export { TeamCreatedToast } from './src/components/TeamNeedsCard';
  export { ReadyTeamsProvider, useReadyTeams } from './src/components/ReadyTeams';
  export { teamStepView } from './src/firstScreen';
`);
const { React, createRoot } = m;
const h = React.createElement;

const tick = () => new Promise((r) => setTimeout(r, 0));
/** Waits for the rendered state, with a deadline. */
const until = async (ok, what, ms = 3000) => {
  const end = Date.now() + ms;
  while (!ok()) {
    if (Date.now() > end) assert.fail(`never happened: ${what}`);
    await new Promise((r) => setTimeout(r, 5));
  }
};
const click = (el) => {
  assert.ok(el, 'element to click exists');
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
};
const byText = (root, sel, re) => [...root.querySelectorAll(sel)].find((el) => re.test(el.textContent ?? ''));
const mount = (el) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  return { host, unmount: () => (root.unmount(), host.remove()) };
};

// A small server: the catalogue says `used_in_space` only while a team made from the template still exists.
const server = () => {
  const s = { used: new Set(), catalogue: 0, undos: 0 };
  const fixture = m.fixtureTransport({});
  m.setTeamsTransport(async (url, method, body, hd) => {
    const path = url.split('?')[0];
    if (path === '/api/team-templates') {
      s.catalogue++;
      return { status: 200, body: { templates: m.TEMPLATES_WIRE.map((t) => ({ ...t, used_in_space: s.used.has(t.id) })) } };
    }
    if (/^\/api\/teams\/[^/]+\/template$/.test(path)) return { status: 404, body: { error: 'not found' } };
    const r = await fixture(url, method, body, hd);
    if (path === '/api/teams/from-template' && r.status === 201) s.used.add(r.body.template_id);
    return r;
  });
  g.fetch = g.window.fetch = async (url) => {
    if (/\/api\/bot-actions\/act_demo\/undo$/.test(String(url))) {
      s.undos++;
      s.used.clear();
      return new Response(JSON.stringify({ id: 'act_demo', status: 'undone' }), { status: 200, headers: { 'Content-Type': 'application/json' } });
    }
    return new Response('{}', { status: 200, headers: { 'Content-Type': 'application/json' } });
  };
  return s;
};

/** Step 3's tiles, as GetStarted computes them (teams stay [] here: only the catalogue can hide a tile). */
const Step3 = () => {
  const rt = m.useReadyTeams();
  const v = m.teamStepView({ teams: [], bots: [], templates: rt?.templates ?? [], madeFrom: (id) => rt?.made(id)?.templateId });
  return h(
    'div',
    { id: 'step3' },
    h('button', { id: 'open-sales', onClick: () => rt.open({ kind: 'template', templateId: 'sales' }) }, 'Sales'),
    v.templateIds.map((id) => h('span', { key: id, 'data-tile': id }, id)),
  );
};
const tiles = () => [...document.querySelectorAll('#step3 [data-tile]')].map((e) => e.dataset.tile);

test('32r: Create hides the Sales tile, Undo brings it straight back, Create hides it again (no reload)', async () => {
  localStorage.clear();
  const s = server();
  const v = mount(h(m.ReadyTeamsProvider, { space: 'spc_demo', bots: [], teams: [], onChanged() {}, onOpenTeam() {} }, h(Step3)));
  await until(() => tiles().includes('sales'), 'Sales is offered at first');

  for (const round of [1, 2]) {
    click(document.querySelector('#open-sales'));
    await until(() => document.querySelector('[data-team-sheet="sales"]'), 'the Sales sheet opens');
    const sheet = document.querySelector('[data-team-sheet="sales"]');
    await until(() => byText(sheet, 'button', /^Create team/) && !byText(sheet, 'button', /^Create team/).disabled, 'Create is on');
    click(byText(sheet, 'button', /^Create team/));
    await until(() => !tiles().includes('sales'), `round ${round}: the Sales tile leaves step 3 after Create`);

    await until(() => byText(document, '[data-team-toast] button', /Undo/), 'the Created toast offers Undo');
    click(byText(document, '[data-team-toast] button', /Undo/));
    await until(() => /Removed @sales\./.test(document.querySelector('[data-team-toast]')?.textContent ?? ''), 'the Removed toast');
    assert.equal(s.undos, round);
    await until(() => tiles().includes('sales'), `round ${round}: the Sales tile is back after Undo, no reload`);
  }
  assert.ok(s.catalogue >= 5, `the catalogue was asked again after each Create and Undo (asked ${s.catalogue} times)`);
  v.unmount();
});

test('32r minor a: a taken handle shows "@x is taken. Try x2." in rose and keeps Create off', async () => {
  server();
  const v = mount(
    h(m.TeamPreviewSheet, {
      open: true,
      space: 'spc_demo',
      mode: { kind: 'template', templateId: 'inbox' },
      templates: m.TEMPLATES_FIXTURE,
      takenNames: [],
      takenHandles: ['helpers', 'sales'],
      spaceBots: [],
      onClose() {},
      onCreated() {},
    }),
  );
  await until(() => document.querySelector('[aria-label="Team handle"]'), 'the handle field');
  const sheet = document.querySelector('[data-team-sheet="inbox"]');
  assert.ok(sheet, 'the Inbox sheet');
  const input = document.querySelector('[aria-label="Team handle"]');
  const create = () => byText(sheet, 'button', /^Create team/);
  await until(() => create() && !create().disabled, 'Create is on with a free handle');
  const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  set.call(input, 'Helpers');
  input.dispatchEvent(new window.Event('input', { bubbles: true }));
  await until(() => /is taken/.test(document.getElementById(input.getAttribute('aria-describedby'))?.textContent ?? ''), 'the taken hint shows');
  const hint = document.getElementById(input.getAttribute('aria-describedby'));
  assert.equal(hint.textContent.trim(), '@helpers is taken. Try helpers2.');
  assert.match(hint.className, /text-rose-300/);
  assert.equal(create().disabled, true, 'Create is off while the handle is taken');

  set.call(input, 'helpers2');
  input.dispatchEvent(new window.Event('input', { bubbles: true }));
  await until(() => !create().disabled, 'Create is on again with a free handle');
  assert.doesNotMatch(document.getElementById(input.getAttribute('aria-describedby')).textContent, /taken/);
  v.unmount();
});

test('32r minor c: the Created toast leaves after 6 s by default (was 10 s)', async () => {
  const realNow = Date.now;
  const realSet = window.setInterval;
  let now = realNow();
  let fn = null;
  Date.now = () => now;
  window.setInterval = (f) => ((fn = f), 1);
  let gone = 0;
  try {
    const v = mount(h(m.TeamCreatedToast, { text: 'Created @sales: 3 bots.', onDismiss: () => gone++ }));
    await until(() => fn, 'the toast starts its clock');
    now += 5_750;
    fn();
    assert.equal(gone, 0, 'still there at 5.75 s');
    now += 250;
    fn();
    assert.equal(gone, 1, 'gone at 6 s');
    v.unmount();
  } finally {
    Date.now = realNow;
    window.setInterval = realSet;
  }
  await tick();
});
