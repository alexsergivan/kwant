// Idea jar (0.7, Iris): section order, the ready-only badge, every action state, the Look now 409 copy,
// `error` never rendered, reduced motion, and parse defensiveness. Fixtures: Juno's GET /api/ideas (#320).
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'HTMLTextAreaElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
let phoneQuery = false;
g.window.matchMedia = (q) => ({ matches: /max-width/.test(q) ? phoneQuery : false, media: q, addEventListener() {}, removeEventListener() {} });

const FIX = JSON.parse(readFileSync(new URL('./fixtures/ideas.json', import.meta.url), 'utf8'));
const NOW = Date.parse('2026-11-05T10:51:00Z');
const realNow = Date.now;
Date.now = () => NOW;

const m = await loadBundled(`
  export * from './src/ideas';
  export * from './src/components/ideas';
  export { IdeaSlip } from './src/components/ideas/IdeaSlip';
  export { TakeCard } from './src/components/ideas/TakeCard';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
  export { act } from 'react';
`);
const h = m.createElement;

const ID = {
  fresh: '6a1f0c2e-1b7d-4c55-9e0a-3f2d8b9c0a01',
  peeking: '6a1f0c2e-1b7d-4c55-9e0a-3f2d8b9c0a02',
  ready: '6a1f0c2e-1b7d-4c55-9e0a-3f2d8b9c0a03',
  picked: '6a1f0c2e-1b7d-4c55-9e0a-3f2d8b9c0a04',
  later: '6a1f0c2e-1b7d-4c55-9e0a-3f2d8b9c0a05',
  failed: '6a1f0c2e-1b7d-4c55-9e0a-3f2d8b9c0a06',
};
const BOTS = [
  { id: 'k-lin-0001', name: 'Lin', role: 'Backend' },
  { id: 'k-iris-0002', name: 'Iris', role: 'Frontend' },
  { id: 'k-vega-0003', name: 'Vega' },
  { id: 'k-pixel-0004', name: 'Pixel', role: 'Frontend owner' },
  { id: 'k-rigel-0005', name: 'Rigel', role: 'QA' },
];
const clone = (v) => JSON.parse(JSON.stringify(v));
const listOf = (over = {}) => ({ ...clone(FIX), ...over });

// ---------------------------------------------------------------------------------------------------
// A fake server: routes answer from `routes`, every call is logged.

let calls = [];
let routes = {};
const json = (status, body) => new Response(body === undefined ? null : JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
g.fetch = async (url, init = {}) => {
  const method = init.method ?? 'GET';
  const path = String(url).replace(/\?.*$/, '');
  calls.push({ method, path, body: init.body ? JSON.parse(init.body) : undefined });
  const key = `${method} ${path}`;
  const r = routes[key] ?? routes[`${method} *${path.slice(path.lastIndexOf('/'))}`];
  if (!r) return json(404, { error: 'not_found', message: 'no route' });
  const out = typeof r === 'function' ? await r(init) : r;
  return out instanceof Response ? out : json(out[0], out[1]);
};
const reset = (list = listOf()) => {
  calls = [];
  routes = { 'GET /api/ideas': () => [200, clone(list)] };
};
const deferred = () => {
  let resolve;
  const p = new Promise((r) => (resolve = r));
  return { p, resolve };
};

// ---------------------------------------------------------------------------------------------------
// DOM helpers

const mounted = new Set();
afterEach(() => {
  for (const u of mounted) u();
  document.body.replaceChildren();
  phoneQuery = false;
});
const settle = async (n = 30) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const mount = async (C, props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(h(C, props));
  await settle();
  const u = () => {
    if (!mounted.delete(u)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(u);
  return host;
};
const jar = (props = {}) => mount(m.IdeaJarView, { spaceId: 'spc_default', spaceName: 'Trailhead', onOpenMission: () => {}, bots: BOTS, ...props });
const norm = (s) => s.replace(/\s+/g, ' ').trim();
const text = (el) => norm(el?.textContent ?? '');
const buttons = (el) => [...el.querySelectorAll('button')];
const button = (el, t) => buttons(el).find((b) => text(b).startsWith(t));
const click = async (el) => {
  assert.ok(el, 'element to click exists');
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const card = (host, id) => host.querySelector(`[data-card="${id}"]`);
const sectionOfCard = (host, id) => card(host, id)?.closest('[data-section]')?.getAttribute('data-section') ?? null;
const sections = (host) => [...host.querySelectorAll('[data-section]')].map((s) => s.getAttribute('data-section'));
const typeInto = async (el, value) => {
  const proto = el.tagName === 'TEXTAREA' ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, value);
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
};
const submit = async (form) => {
  form.dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
};

// ---------------------------------------------------------------------------------------------------
// Sections

test('sections run Takes ready, Being peeked at, In the jar, Later, Picked; Later starts folded', async () => {
  reset();
  const host = await jar();
  assert.deepEqual(sections(host), ['ready', 'peeking', 'jar', 'later', 'picked']);
  const heads = [...host.querySelectorAll('[data-section] > h3')].map((x) => text(x).replace(/\d+$/, '').trim());
  assert.deepEqual(heads, ['Takes ready', 'Being peeked at', 'In the jar', 'Later', 'Picked']);
  // Later is folded and its toggle says so.
  const toggle = host.querySelector('#jar-sec-later');
  assert.equal(toggle.getAttribute('aria-expanded'), 'false');
  assert.equal(card(host, ID.later), null);
  await click(toggle);
  assert.equal(toggle.getAttribute('aria-expanded'), 'true');
  assert.ok(card(host, ID.later));
  // In the jar holds new and failed, oldest first.
  const jarIds = [...host.querySelectorAll('[data-section="jar"] [data-card]')].map((c) => c.getAttribute('data-card'));
  assert.deepEqual(jarIds, [ID.failed, ID.fresh]);
  // The first ready take is open on desktop.
  assert.ok(host.querySelector(`[data-take-card="${ID.ready}"]`));
});

test('empty sections are hidden; Picked folds after 3 rows', async () => {
  const base = FIX.ideas.find((i) => i.status === 'picked');
  const picked = [1, 2, 3, 4, 5].map((n) => ({ ...base, id: `p${n}`, mission_id: `m-${n}`, updated_at: `2026-11-0${n}T00:00:00Z` }));
  reset(listOf({ ideas: picked }));
  const host = await jar();
  assert.deepEqual(sections(host), ['picked']);
  assert.equal(host.querySelectorAll('[data-picked-row]').length, 3);
  const more = host.querySelector('[data-picked-toggle]');
  assert.equal(text(more), 'Show all 5');
  await click(more);
  assert.equal(host.querySelectorAll('[data-picked-row]').length, 5);
});

test('groupIdeas sorts each section the way the spec says', () => {
  const g2 = m.groupIdeas(m.parseIdeaList(FIX).ideas);
  assert.deepEqual(
    m.SECTION_ORDER.map((k) => g2[k].map((i) => i.id)),
    [[ID.ready], [ID.peeking], [ID.failed, ID.fresh], [ID.later], [ID.picked]],
  );
});

// ---------------------------------------------------------------------------------------------------
// Badge

test('the badge counts ready takes only, and follows live updates', async () => {
  reset();
  let seen = [];
  const Probe = ({ spaceId }) => {
    const n = m.useIdeaJarBadge(spaceId);
    seen.push(n);
    return h('span', { 'data-n': n }, String(n));
  };
  const host = await mount(Probe, { spaceId: 'spc_default' });
  assert.equal(host.querySelector('[data-n]').textContent, '1', 'one ready take; peeking, new, failed, later, picked do not count');

  // A take arrives: the server now has two ready ones.
  const two = listOf();
  two.ideas = two.ideas.map((i) => (i.id === ID.peeking ? { ...FIX.ideas.find((x) => x.status === 'ready'), id: ID.peeking } : i));
  reset(two);
  m.emitIdeaUpdated({ id: ID.peeking, status: 'ready', space_id: 'spc_default' });
  await new Promise((r) => setTimeout(r, 300));
  await settle();
  assert.equal(host.querySelector('[data-n]').textContent, '2');

  // Another space's event is ignored.
  calls = [];
  m.emitIdeaUpdated({ id: 'x', status: 'ready', space_id: 'other' });
  await new Promise((r) => setTimeout(r, 300));
  assert.equal(calls.length, 0);
});

test('a ready idea without a valid take is not counted (it parses back to new)', () => {
  const raw = { ...FIX.ideas.find((i) => i.status === 'ready'), take: { verdict: 'nope' } };
  assert.equal(m.parseIdea(raw).status, 'new');
  assert.equal(m.countIdeas([m.parseIdea(raw)]).ready, 0);
});

// ---------------------------------------------------------------------------------------------------
// Peeking, failed, `error`

test('peeking reads "Lin is peeking · 1 min" and shows a disabled Peeking…', async () => {
  reset();
  const host = await jar();
  const c = card(host, ID.peeking);
  assert.equal(text(c.querySelector('[data-peeking-text]')), 'Lin is peeking · 1 min');
  const b = button(c, 'Peeking');
  assert.ok(b.disabled);
  // Without a name for the bot.
  assert.equal(m.peekingText(null, null), 'A bot is peeking');
});

test('failed shows its chip and line, and the raw error is never rendered anywhere', async () => {
  reset();
  const host = await jar();
  const c = card(host, ID.failed);
  assert.equal(text(c.querySelector('[data-failed-chip]')), 'Couldn’t read this one');
  assert.equal(text(c.querySelector('[data-failed-line]')), 'Rigel’s take came back garbled twice. Nothing was changed.');
  assert.ok(button(c, 'Look now'));
  assert.ok(!host.innerHTML.includes('plan must have'), 'the raw error string never reaches the DOM');
  // Static render of a failed slip with a nasty error, too.
  const failed = { ...m.parseIdea(FIX.ideas.find((i) => i.status === 'failed')), error: '<b>boom</b> stack trace' };
  const s = m.renderToStaticMarkup(h(m.IdeaSlip, { idea: failed, botName: 'Rigel', botNames: [], now: NOW }));
  assert.ok(!s.includes('boom') && !s.includes('stack trace'));
});

test('idea text is plain text: markup in data is shown as text, never as HTML', () => {
  const idea = { ...m.parseIdea(FIX.ideas[0]), text: '<img src=x onerror=alert(1)> ask @Pixel' };
  const s = m.renderToStaticMarkup(h(m.IdeaSlip, { idea, botName: null, botNames: ['Pixel'], now: NOW }));
  assert.ok(!s.includes('<img'));
  assert.ok(s.includes('&lt;img'));
  assert.ok(s.includes('data-mention'));
});

// ---------------------------------------------------------------------------------------------------
// Look now

test('Look now: Asking… while it waits, then the slip moves to Being peeked at with the bot from the 202', async () => {
  reset();
  const d = deferred();
  routes[`POST /api/ideas/${ID.fresh}/look`] = () => d.p.then(() => [202, FIX._examples['POST /api/ideas/:id/look 202']]);
  const host = await jar({ bots: BOTS.filter((b) => b.name !== 'Pixel') });
  await click(button(card(host, ID.fresh), 'Look now'));
  assert.ok(button(card(host, ID.fresh), 'Asking'), 'busy text');
  const asking = button(card(host, ID.fresh), 'Asking');
  assert.equal(asking.getAttribute('aria-disabled'), 'true');
  assert.equal(asking.disabled, false, 'busy stays focusable (M3)');
  d.resolve();
  await settle();
  assert.equal(sectionOfCard(host, ID.fresh), 'peeking');
  assert.match(text(card(host, ID.fresh).querySelector('[data-peeking-text]')), /^Pixel is peeking/, 'kwant_name from the 202 names the bot');
});

for (const [code, expected, refetch] of [
  ['no_free_bot', 'No bot is free right now. Try again in a few minutes.', false],
  ['already_peeking', 'A bot is already peeking at this one.', true],
]) {
  test(`Look now 409 ${code}: the copy shows, the idea stays where it is, the button comes back`, async () => {
    reset();
    const body = FIX._examples['POST /api/ideas/:id/look 409'].find((x) => x.error === code);
    routes[`POST /api/ideas/${ID.fresh}/look`] = [409, body];
    const host = await jar();
    const before = [...host.querySelectorAll('[data-section="jar"] [data-card]')].map((c) => c.getAttribute('data-card'));
    calls = [];
    await click(button(card(host, ID.fresh), 'Look now'));
    const info = card(host, ID.fresh).querySelector('[data-info]');
    assert.equal(text(info), expected);
    assert.equal(info.getAttribute('role'), 'status');
    assert.equal(info.getAttribute('data-info'), 'sky');
    assert.equal(sectionOfCard(host, ID.fresh), 'jar');
    assert.deepEqual([...host.querySelectorAll('[data-section="jar"] [data-card]')].map((c) => c.getAttribute('data-card')), before, 'no reorder');
    assert.ok(!host.innerHTML.includes(code), 'the code itself is never shown');
    assert.ok(!host.innerHTML.includes(body.message), 'nor the server message');
    assert.equal(button(card(host, ID.fresh), 'Look now').disabled, false);
    assert.equal(calls.some((c) => c.method === 'GET'), refetch);
  });
}

test('409 copy: already_peeking names the bot; no_tools_unavailable has no special copy', () => {
  assert.equal(m.lookRefusalText('already_peeking', 'Lin', null), 'Lin is already peeking at this one.');
  assert.equal(m.lookRefusalText('picked', null, null), 'This idea is already a mission.');
  assert.equal(m.lookRefusalText('no_tools_unavailable', null, 'nope'), 'Nope.');
  assert.equal(m.lookRefusalText('weird', null, null), 'Couldn’t ask a bot just now. Try again.');
});

// ---------------------------------------------------------------------------------------------------
// Do it

test('Do it: hint says draft; busy disables every action; done shows Picked, the amber chip and focuses Open on board', async () => {
  reset();
  const d = deferred();
  routes[`POST /api/ideas/${ID.ready}/pick`] = () => d.p.then(() => [201, FIX._examples['POST /api/ideas/:id/pick 201']]);
  let opened = null;
  const host = await jar({ onOpenMission: (id) => (opened = id) });
  const tc = host.querySelector(`[data-take-card="${ID.ready}"]`);
  assert.equal(text(tc.querySelector('[data-do-it-hint]')), 'Makes a draft mission for Iris. Nothing starts until you approve it.');
  assert.equal(text(tc.querySelector('[data-read-only]')), 'Read only: Iris used no tools and changed nothing.');
  await click(tc.querySelector('[data-action="do-it"]'));
  assert.equal(text(tc.querySelector('[data-action="do-it"]')), 'Making the draft…');
  for (const b of tc.querySelectorAll('[data-action-bar] button, [data-idea-menu]')) {
    assert.equal(b.getAttribute('aria-disabled'), 'true', `${text(b) || 'menu'} is aria-disabled while busy`);
    assert.equal(b.disabled, false, `${text(b) || 'menu'} keeps focus while busy (M3)`);
  }
  // A second press while busy does nothing.
  await click(tc.querySelector('[data-action="do-it"]'));
  assert.equal(calls.filter((c) => c.path.endsWith('/pick')).length, 1);
  d.resolve();
  await settle();
  const panel = host.querySelector(`[data-take-card="${ID.ready}"] [data-picked-panel]`);
  assert.ok(panel, 'the action bar is replaced by the done panel');
  assert.ok(!host.querySelector(`[data-take-card="${ID.ready}"] [data-action-bar]`));
  assert.match(text(panel), /Picked/);
  assert.match(text(panel), /Draft mission made for Iris · 3 tasks\./);
  assert.equal(text(panel.querySelector('[data-waiting-chip]')), 'Waiting for your OK');
  assert.match(text(panel), /Approve it on the board to start\. Until then nothing runs\./);
  assert.ok(!/started|running now/i.test(text(panel)), 'never a started state');
  const open = panel.querySelector('[data-action="open-board"]');
  assert.equal(document.activeElement, open, 'focus goes to Open on board');
  await click(open);
  assert.equal(opened, 'm-1b2c3d4e');
});

test('Do it: no chip when the mission is not a draft; no task count when it is missing', () => {
  const idea = m.parseIdea(FIX.ideas.find((i) => i.status === 'ready'));
  const s = m.renderToStaticMarkup(h(m.TakeCard, { idea, authorName: 'Iris', botNames: [], now: NOW, pick: { kind: 'done', result: { idea_id: idea.id, mission_id: 'm', mission_status: 'active', tasks: null } } }));
  assert.ok(!s.includes('Waiting for your OK'));
  assert.ok(s.includes('Draft mission made for Iris.'));
});

test('Do it error: rose alert, nothing created, the button comes back', async () => {
  reset();
  routes[`POST /api/ideas/${ID.ready}/pick`] = [500, { error: 'internal', message: 'boom' }];
  const host = await jar();
  await click(host.querySelector(`[data-take-card="${ID.ready}"] [data-action="do-it"]`));
  const alert = host.querySelector(`[data-take-card="${ID.ready}"] [role="alert"]`);
  assert.equal(text(alert), 'Couldn’t make the draft. Nothing was created. Try again.');
  assert.equal(host.querySelector(`[data-take-card="${ID.ready}"] [data-action="do-it"]`).disabled, false);
  assert.ok(!host.innerHTML.includes('boom'));
});

// ---------------------------------------------------------------------------------------------------
// Later, Restore, Edit, Delete

test('Later is optimistic with "Moved to Later · Undo"; Undo calls restore', async () => {
  reset();
  const readyIdea = FIX.ideas.find((i) => i.status === 'ready');
  routes[`POST /api/ideas/${ID.ready}/later`] = [200, { idea: { ...readyIdea, status: 'later' } }];
  routes[`POST /api/ideas/${ID.ready}/restore`] = [200, { idea: readyIdea }];
  const host = await jar();
  await click(host.querySelector(`[data-take-card="${ID.ready}"] [data-action="later"]`));
  assert.equal(sectionOfCard(host, ID.ready), null, 'Later is folded, so the card is out of view');
  assert.ok(!host.querySelector('[data-section="ready"]'));
  const toast = host.querySelector('[data-undo-toast]');
  assert.match(text(toast), /^Moved to Later · Undo/);
  assert.ok(document.activeElement && document.activeElement !== document.body, 'focus never drops to body');
  await click(toast.querySelector('[data-undo]'));
  assert.ok(calls.some((c) => c.method === 'POST' && c.path.endsWith('/restore')));
  assert.equal(sectionOfCard(host, ID.ready), 'ready');
});

test('Later failure puts it back with "Couldn’t move it. Try again."', async () => {
  reset();
  routes[`POST /api/ideas/${ID.ready}/later`] = [500, { error: 'internal', message: 'x' }];
  const host = await jar();
  await click(host.querySelector(`[data-take-card="${ID.ready}"] [data-action="later"]`));
  assert.equal(sectionOfCard(host, ID.ready), 'ready');
  assert.match(text(card(host, ID.ready)), /Couldn’t move it\. Try again\./);
  assert.ok(!host.querySelector('[data-undo-toast]'));
});

test('Restore from Later uses what the API returns', async () => {
  reset();
  const laterIdea = FIX.ideas.find((i) => i.status === 'later');
  routes[`POST /api/ideas/${ID.later}/restore`] = [200, { idea: { ...laterIdea, status: 'ready' } }];
  const host = await jar();
  await click(host.querySelector('#jar-sec-later'));
  await click(button(card(host, ID.later), 'Restore'));
  assert.equal(sectionOfCard(host, ID.ready), 'ready');
  assert.equal(sectionOfCard(host, ID.later), 'ready', 'it still has a take, so it goes to Takes ready');
});

test('Edit opens in place; Save and put back PATCHes and the idea is back in the jar', async () => {
  reset();
  const readyIdea = FIX.ideas.find((i) => i.status === 'ready');
  routes[`PATCH /api/ideas/${ID.ready}`] = (init) => [200, { idea: { ...readyIdea, text: JSON.parse(init.body).text, status: 'new', take: null, kwant_id: null } }];
  const host = await jar();
  await click(host.querySelector(`[data-take-card="${ID.ready}"] [data-action="edit"]`));
  const form = host.querySelector('[data-idea-edit]');
  assert.ok(form);
  assert.match(text(form), /Saving clears this take, and a free bot looks again\./);
  const ta = form.querySelector('textarea');
  await typeInto(ta, '');
  assert.ok(form.querySelector('[data-save]').disabled, 'empty: Save is off');
  await typeInto(ta, 'x'.repeat(2001));
  assert.ok(form.querySelector('[data-save]').disabled, 'too long: Save is off');
  assert.match(text(form), /too long for the jar/);
  await typeInto(ta, 'Dark mode, but only for the board');
  await submit(form);
  const patch = calls.find((c) => c.method === 'PATCH');
  assert.deepEqual(patch.body, { text: 'Dark mode, but only for the board' });
  assert.equal(sectionOfCard(host, ID.ready), 'jar');
});

test('Delete hides it with "Idea deleted · Undo" and only calls DELETE when the toast ends; Undo keeps it', async () => {
  reset();
  routes[`DELETE /api/ideas/${ID.fresh}`] = () => new Response(null, { status: 204 });
  const host = await jar();
  await click(card(host, ID.fresh).querySelector('[data-idea-menu]'));
  await click(card(host, ID.fresh).querySelector('[data-menu-item="delete"]'));
  assert.equal(card(host, ID.fresh), null);
  let toast = host.querySelector('[data-undo-toast]');
  assert.match(text(toast), /^Idea deleted · Undo/);
  assert.ok(!calls.some((c) => c.method === 'DELETE'), 'not yet');
  await click(toast.querySelector('[data-undo]'));
  assert.ok(card(host, ID.fresh), 'Undo brings it back');
  assert.ok(!calls.some((c) => c.method === 'DELETE'));
  // Again, and let the toast close.
  await click(card(host, ID.fresh).querySelector('[data-idea-menu]'));
  await click(card(host, ID.fresh).querySelector('[data-menu-item="delete"]'));
  toast = host.querySelector('[data-undo-toast]');
  await click(toast.querySelector('[aria-label="Close"]'));
  assert.ok(calls.some((c) => c.method === 'DELETE' && c.path.endsWith(ID.fresh)));
  assert.equal(card(host, ID.fresh), null);
});

// ---------------------------------------------------------------------------------------------------
// Header, notices, empty, loading, error, live

test('header chip and the cap / off lines come from the server numbers', async () => {
  reset();
  let host = await jar();
  assert.equal(text(host.querySelector('[data-peeks-left]')), '1 of 3 peeks left today');
  for (const u of mounted) u();
  reset(listOf({ cap_reached: true, peeks_today: 3 }));
  host = await jar();
  assert.equal(text(host.querySelector('[data-peeks-left]')), '0 of 3 peeks left today');
  assert.match(text(host), /Bots have had their 3 peeks for today in Trailhead\. The rest wait for tomorrow, or press Look now on one\./);
  for (const u of mounted) u();
  let settings = 0;
  reset(listOf({ daily_cap: 0, cap_reached: true }));
  host = await jar({ onOpenSettings: () => settings++ });
  assert.equal(host.querySelector('[data-peeks-left]'), null);
  assert.match(text(host), /Bots don’t peek in this jar on their own\. Ideas wait here until you press Look now\./);
  await click(host.querySelector('[data-action="turn-on"]'));
  assert.equal(settings, 1);
  // The server's own count wins when it sends one.
  assert.equal(m.peeksLeft({ daily_cap: 3, peeks_today: 3, cap_reached: false, peeks_left: 2 }), 2);
});

test('empty state: the jar is empty, Try one fills the field without dropping it', async () => {
  reset(listOf({ ideas: [], counts: {} }));
  const host = await jar();
  assert.match(text(host.querySelector('[data-empty]')), /^The jar is empty ?Drop in anything/);
  const chip = [...host.querySelectorAll('[data-example]')].find((b) => text(b) === 'A weekly summary of what the team did');
  await click(chip);
  assert.equal(host.querySelector('[data-quick-add] input').value, 'A weekly summary of what the team did');
  assert.ok(!calls.some((c) => c.method === 'POST'));
});

test('dropping an idea: POST, then it shows in the jar and the live region says so', async () => {
  reset();
  const made = { ...FIX.ideas[0], id: 'new-1', text: 'A faster way to find old chats', created_at: '2026-11-05T10:50:59Z' };
  routes['POST /api/ideas'] = [201, { idea: made }];
  const host = await jar();
  const input = host.querySelector('[data-quick-add] input');
  await typeInto(input, '  A faster way to find old chats ');
  await submit(host.querySelector('[data-quick-add]'));
  assert.deepEqual(calls.find((c) => c.method === 'POST').body, { space_id: 'spc_default', text: 'A faster way to find old chats' });
  assert.ok(card(host, 'new-1'));
  assert.equal(input.value, '');
  assert.equal(text(host.querySelector('[data-live]')), 'Dropped in the jar: A faster way to find old chats.');
});

test('loading has aria-busy; a load error offers Retry', async () => {
  reset();
  const d = deferred();
  routes['GET /api/ideas'] = () => d.p.then(() => [500, { error: 'x', message: 'down' }]);
  const host = await jar();
  assert.equal(host.querySelector('[data-loading]').getAttribute('aria-busy'), 'true');
  d.resolve();
  await settle();
  assert.match(text(host), /Couldn’t open the jar\. Try again\./);
  routes['GET /api/ideas'] = () => [200, clone(FIX)];
  await click(host.querySelector('[data-action="retry"]'));
  assert.deepEqual(sections(host), ['ready', 'peeking', 'jar', 'later', 'picked']);
});

test('a take arriving live: New chip, live region speaks, focus does not move', async () => {
  reset();
  const host = await jar();
  const before = document.activeElement;
  const arrived = listOf();
  const readyIdea = FIX.ideas.find((i) => i.status === 'ready');
  arrived.ideas = arrived.ideas.map((i) => (i.id === ID.peeking ? { ...readyIdea, id: ID.peeking, kwant_id: 'k-lin-0001', updated_at: '2026-11-05T10:51:00Z' } : i));
  reset(arrived);
  m.emitIdeaUpdated({ id: ID.peeking, status: 'ready', space_id: 'spc_default' });
  await new Promise((r) => setTimeout(r, 300));
  await settle();
  assert.equal(sectionOfCard(host, ID.peeking), 'ready');
  assert.ok(card(host, ID.peeking).querySelector('[data-new-chip]'));
  assert.match(text(host.querySelector('[data-live]')), /^Lin’s take is ready: Worth it|^Lin’s take is ready: /);
  assert.equal(document.activeElement, before);
});

test('Picked rows read the mission chip from the missions App has; unknown shows none', async () => {
  reset();
  let host = await jar({ missions: [{ id: 'm-7c9d2a10-draft', status: 'draft' }] });
  assert.equal(text(host.querySelector('[data-mission-chip]')), 'Waiting for your OK');
  for (const u of mounted) u();
  reset();
  host = await jar({ missions: [] });
  assert.equal(host.querySelector('[data-mission-chip]'), null);
  assert.ok(host.querySelector('[data-picked-row] [data-action="open-board"]'));
});

test('phone: ready takes are rows that open their own screen with a back button and folded lists', async () => {
  phoneQuery = true;
  reset();
  const host = await jar();
  assert.equal(host.querySelector('[data-take-card]'), null, 'no inline card on phone');
  await click(host.querySelector(`[data-take-row="${ID.ready}"] [data-action="open"]`));
  assert.ok(host.querySelector(`[data-take-screen="${ID.ready}"]`));
  assert.equal(text(host.querySelector('[data-action="do-it"]')), 'Do it · make a draft');
  assert.equal(host.querySelectorAll('details[data-fold]').length, 2);
  for (const d of host.querySelectorAll('details')) assert.equal(d.open, false);
  await click(host.querySelector('[data-action="back"]'));
  assert.ok(host.querySelector('[data-sections]'));
});

test('the view scrolls itself inside a flex-1 min-h-0 pane (list and phone take screen)', async () => {
  const scroller = (host) => {
    const el = host.firstElementChild;
    assert.ok(el, 'mounted');
    const sc = el.matches('[data-jar-scroll]') ? el : el.querySelector('[data-jar-scroll]');
    assert.ok(sc, 'has a scroller');
    for (const c of ['overflow-y-auto', 'min-h-0', 'flex-1']) assert.ok(sc.className.includes(c), `scroller has ${c}`);
    return sc;
  };
  reset();
  const host = await jar();
  assert.ok(scroller(host).querySelector('[data-sections]'));
  phoneQuery = true;
  reset();
  const phone = await jar();
  await click(phone.querySelector(`[data-take-row="${ID.ready}"] [data-action="open"]`));
  const screen = phone.querySelector(`[data-take-screen="${ID.ready}"]`);
  assert.ok(screen.className.includes('min-h-0') && screen.className.includes('flex-1'));
  assert.ok(screen.querySelector('[data-jar-scroll] [data-action-bar]'), 'sticky Do it bar sits inside the scroller');
  const card = screen.querySelector('[data-take-card]');
  assert.ok(!/\boverflow-(hidden|auto|scroll)\b/.test(card.className), 'the card clips without becoming a scroll box, so the bar can stick');
  assert.ok(/\bsticky\b/.test(screen.querySelector('[data-action-bar]').className));
});

// ---------------------------------------------------------------------------------------------------
// Reduced motion

test('reduced motion: every jar animation is off, spinners stay still, nothing loops', () => {
  const css = readFileSync(new URL('../src/styles/ideas.css', import.meta.url), 'utf8');
  const reduce = css.slice(css.indexOf('@media (prefers-reduced-motion: reduce)'));
  assert.ok(reduce.length > 40, 'has a reduced-motion block');
  for (const cls of ['.kw-jar-wobble', '.kw-jar-wobble-late', '.kw-jar-peek', '.kw-jar-arrive']) assert.ok(reduce.includes(cls), `${cls} is turned off`);
  assert.match(reduce, /\.kw-jar-slip\s*\{\s*display:\s*none/);
  assert.ok(!/infinite/.test(css), 'nothing loops');
  // Durations stay inside the 520 ms drop.
  const ends = [...css.matchAll(/(\d+)ms(?:\s+(\d+)ms)?/g)].map((x) => Number(x[1]) + Number(x[2] ?? 0));
  assert.ok(Math.max(...ends) <= 520, `longest step ends at ${Math.max(...ends)} ms`);
  // Animated properties are transform and opacity only.
  for (const kf of css.matchAll(/@keyframes[^{]+\{([\s\S]*?)\}\s*\}/g)) {
    const props = [...kf[1].matchAll(/([a-z-]+)\s*:/g)].map((x) => x[1]);
    for (const p of props) assert.ok(['opacity', 'transform'].includes(p), `keyframes animate ${p}`);
  }
  const spin = m.renderToStaticMarkup(h(m.TakeCard, { idea: m.parseIdea(FIX.ideas.find((i) => i.status === 'ready')), authorName: 'Iris', botNames: [], now: NOW, pick: { kind: 'busy' } }));
  assert.ok(spin.includes('motion-reduce:animate-none'));
});

// ---------------------------------------------------------------------------------------------------
// Parsing

test('parsing is defensive: unknown status, bad lists and long plans are tamed', () => {
  assert.equal(m.parseIdea(null), null);
  assert.equal(m.parseIdea({ id: 'a', text: 'x', status: 'zzz', created_at: 'c', updated_at: 'u' }).status, 'new');
  const t = m.parseTake({ verdict: 'maybe', effort: 'M', why: 'w', plan: ['one', { title: 'two', done_when: 'd' }, 3, ...Array(10).fill('more')], risks: [1, 'r', ''], questions: 'no' });
  assert.equal(t.plan.length, 6);
  assert.deepEqual(t.plan[0], { title: 'one', done_when: '' });
  assert.deepEqual(t.risks, ['r']);
  assert.deepEqual(t.questions, []);
  const l = m.parseIdeaList({ ideas: [FIX.ideas[0], 'junk', null], daily_cap: 'x' });
  assert.equal(l.ideas.length, 1);
  assert.equal(l.counts.new, 1);
});

test('copy has no emoji', async () => {
  reset();
  const host = await jar();
  assert.ok(!/\p{Extended_Pictographic}/u.test(host.textContent));
});

test.after(() => {
  Date.now = realNow;
});

// ---------------------------------------------------------------------------------------------------
// Round 2 (Mira's #324 review): focus, busy, the badge after Do it, menu, header

const focusOn = async (el) => {
  assert.ok(el, 'element to focus exists');
  el.focus();
  await settle();
};

test('ruling 2: a just-picked idea stays in Takes ready but leaves the badge at once', async () => {
  reset();
  const d = deferred();
  routes[`POST /api/ideas/${ID.ready}/pick`] = () => d.p.then(() => [201, FIX._examples['POST /api/ideas/:id/pick 201']]);
  const Probe = ({ spaceId }) => h('span', { 'data-n': '' }, String(m.useIdeaJarBadge(spaceId)));
  const badge = await mount(Probe, { spaceId: 'spc_default' });
  const host = await jar();
  assert.equal(text(badge), '1', 'one take ready before Do it');
  await click(host.querySelector(`[data-take-card="${ID.ready}"] [data-action="do-it"]`));
  d.resolve();
  await settle();
  // Before any refetch (the server here still says ready), the jar has already stopped counting it.
  assert.equal(sectionOfCard(host, ID.ready), 'ready', 'still in Takes ready for the session');
  assert.ok(host.querySelector(`[data-take-card="${ID.ready}"] [data-picked-panel]`));
  assert.equal(text(badge), '0', 'the badge does not count the picked idea');
  assert.equal(m.countIdeas([]).ready, 0);
});

test('M1: when the pick returns, focus stays where the person went (the quick-add)', async () => {
  reset();
  const d = deferred();
  routes[`POST /api/ideas/${ID.ready}/pick`] = () => d.p.then(() => [201, FIX._examples['POST /api/ideas/:id/pick 201']]);
  const host = await jar();
  const doIt = host.querySelector(`[data-take-card="${ID.ready}"] [data-action="do-it"]`);
  await focusOn(doIt);
  await click(doIt);
  assert.equal(document.activeElement, doIt, 'the busy Do it keeps focus (M3)');
  const input = host.querySelector('[data-quick-add] input');
  await focusOn(input);
  d.resolve();
  await settle();
  assert.ok(host.querySelector('[data-picked-panel]'));
  assert.equal(document.activeElement, input, 'focus is not taken to Open on board');
});

test('M1: focus on the busy Do it moves to Open on board when the pick returns', async () => {
  reset();
  const d = deferred();
  routes[`POST /api/ideas/${ID.ready}/pick`] = () => d.p.then(() => [201, FIX._examples['POST /api/ideas/:id/pick 201']]);
  const host = await jar();
  const doIt = host.querySelector(`[data-take-card="${ID.ready}"] [data-action="do-it"]`);
  await focusOn(doIt);
  await click(doIt);
  d.resolve();
  await settle();
  assert.equal(document.activeElement, host.querySelector('[data-picked-panel] [data-action="open-board"]'));
});

test('M2: Esc and Cancel return focus to Edit, or to the menu when Edit came from it', async () => {
  reset();
  const host = await jar();
  const tc = () => host.querySelector(`[data-take-card="${ID.ready}"]`);
  // Edit from the bar, Esc.
  await click(tc().querySelector('[data-action="edit"]'));
  const ta = tc().querySelector('[data-idea-edit] textarea');
  assert.equal(document.activeElement, ta);
  ta.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  await settle();
  assert.ok(!tc().querySelector('[data-idea-edit]'));
  assert.equal(document.activeElement, tc().querySelector('[data-action="edit"]'), 'Esc → Edit');
  // Edit from the menu, Cancel.
  await click(tc().querySelector('[data-idea-menu]'));
  await click(tc().querySelector('[data-menu-item="edit"]'));
  await click(button(tc().querySelector('[data-idea-edit]'), 'Cancel'));
  assert.equal(document.activeElement, tc().querySelector('[data-idea-menu]'), 'Cancel → ⋯');
  // A slip (Edit only lives in its menu).
  await click(card(host, ID.fresh).querySelector('[data-idea-menu]'));
  await click(card(host, ID.fresh).querySelector('[data-menu-item="edit"]'));
  await click(button(card(host, ID.fresh).querySelector('[data-idea-edit]'), 'Cancel'));
  assert.equal(document.activeElement, card(host, ID.fresh).querySelector('[data-idea-menu]'));
});

test('M3: Look now keeps focus while it asks, and the 409 line shows next to it', async () => {
  reset();
  const d = deferred();
  const body = FIX._examples['POST /api/ideas/:id/look 409'].find((x) => x.error === 'no_free_bot');
  routes[`POST /api/ideas/${ID.fresh}/look`] = () => d.p.then(() => [409, body]);
  const host = await jar();
  const look = card(host, ID.fresh).querySelector('[data-action="look"]');
  await focusOn(look);
  await click(look);
  assert.equal(document.activeElement, look);
  await click(look);
  assert.equal(calls.filter((c) => c.path.endsWith('/look')).length, 1, 'a press while busy is ignored');
  d.resolve();
  await settle();
  assert.equal(document.activeElement, look);
  assert.equal(look.getAttribute('aria-disabled'), null);
});

test('S1: when a card leaves, focus goes to the next card’s Do it or Look, not ⋯', async () => {
  reset();
  routes[`POST /api/ideas/${ID.failed}/later`] = [200, { idea: { ...FIX.ideas.find((i) => i.id === ID.failed), status: 'later' } }];
  const host = await jar();
  await click(card(host, ID.failed).querySelector('[data-idea-menu]'));
  // Later isn't in a slip's menu; Delete leaves the same way.
  await click(card(host, ID.failed).querySelector('[data-menu-item="delete"]'));
  assert.equal(document.activeElement, card(host, ID.fresh).querySelector('[data-action="look"]'));
});

test('S2: Undo puts focus on the restored card (Later and Delete)', async () => {
  reset();
  const readyIdea = FIX.ideas.find((i) => i.status === 'ready');
  routes[`POST /api/ideas/${ID.ready}/later`] = [200, { idea: { ...readyIdea, status: 'later' } }];
  routes[`POST /api/ideas/${ID.ready}/restore`] = [200, { idea: readyIdea }];
  const host = await jar();
  await click(host.querySelector(`[data-take-card="${ID.ready}"] [data-action="later"]`));
  const undo = host.querySelector('[data-undo-toast] [data-undo]');
  await focusOn(undo);
  await click(undo);
  assert.equal(document.activeElement, host.querySelector(`[data-take-card="${ID.ready}"] [data-action="do-it"]`), 'Later → Undo → Do it');
  await click(card(host, ID.fresh).querySelector('[data-idea-menu]'));
  await click(card(host, ID.fresh).querySelector('[data-menu-item="delete"]'));
  const undo2 = host.querySelector('[data-undo-toast] [data-undo]');
  await focusOn(undo2); // a browser focuses the button it clicks; jsdom doesn't
  await click(undo2);
  assert.equal(document.activeElement, card(host, ID.fresh).querySelector('[data-action="look"]'), 'Delete → Undo → Look now');
});

test('S2: Undo does not take focus from somewhere else once the restore lands', async () => {
  reset();
  const readyIdea = FIX.ideas.find((i) => i.status === 'ready');
  const d = deferred();
  routes[`POST /api/ideas/${ID.ready}/later`] = [200, { idea: { ...readyIdea, status: 'later' } }];
  routes[`POST /api/ideas/${ID.ready}/restore`] = () => d.p.then(() => [200, { idea: readyIdea }]);
  const host = await jar();
  await click(host.querySelector(`[data-take-card="${ID.ready}"] [data-action="later"]`));
  await click(host.querySelector('[data-undo-toast] [data-undo]'));
  const input = host.querySelector('[data-quick-add] input');
  await focusOn(input);
  d.resolve();
  await settle();
  assert.equal(document.activeElement, input);
});

test('S3: the ⋯ menu is a plain disclosure (no role=menu), Esc returns to ⋯', async () => {
  reset();
  const host = await jar();
  const trigger = card(host, ID.fresh).querySelector('[data-idea-menu]');
  assert.equal(trigger.getAttribute('aria-haspopup'), null);
  assert.equal(trigger.getAttribute('aria-expanded'), 'false');
  await click(trigger);
  assert.equal(trigger.getAttribute('aria-expanded'), 'true');
  assert.ok(!host.querySelector('[role="menu"], [role="menuitem"]'));
  const first = card(host, ID.fresh).querySelector('[data-menu-item="edit"]');
  assert.equal(document.activeElement, first);
  first.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await settle();
  assert.equal(trigger.getAttribute('aria-expanded'), 'false');
  assert.equal(document.activeElement, trigger);
});

test('S4–S6: phone Drop in is 44 px, Look again does not wrap, Settings is icon-only below sm with chips on line 2', async () => {
  reset();
  phoneQuery = true;
  const host = await jar({ onOpenSettings: () => {} });
  const drop = host.querySelector('[data-quick-add] button[type="submit"]');
  assert.ok(drop.className.includes('!min-h-[44px]') && !drop.className.includes('!min-h-[40px]'));
  const s = host.querySelector('[data-action="settings"]');
  assert.equal(s.getAttribute('aria-label'), 'Settings');
  assert.ok(/max-sm:w-11/.test(s.className) && /max-sm:h-11/.test(s.className) && /\bml-auto\b/.test(s.className));
  assert.ok(s.querySelector('span.hidden.sm\\:inline'), 'the word shows from sm up');
  const chips = host.querySelector('[data-jar-chips]');
  assert.ok(chips.contains(host.querySelector('[data-peeks-left]')));
  assert.ok(/\bbasis-full\b/.test(chips.className) && /\border-last\b/.test(chips.className) && /sm:order-none/.test(chips.className));
  await click(host.querySelector(`[data-card="${ID.ready}"] [data-action="open"]`));
  assert.ok(/whitespace-nowrap/.test(host.querySelector('[data-action="look"]').className));
});

// Round 3 (Mira's #324 round 2): M5 and M6

test('M5: Undo of a deleted take: no New chip, no arrival announcement, it comes back open, focus is not body', async () => {
  const two = listOf();
  const readyIdea = two.ideas.find((i) => i.status === 'ready');
  const ID2 = '6a1f0c2e-1b7d-4c55-9e0a-3f2d8b9c0a07';
  two.ideas.push({ ...clone(readyIdea), id: ID2, text: 'Second take', updated_at: '2026-11-05T09:00:00Z' });
  reset(two);
  const host = await jar();
  const open = host.querySelector('[data-take-card]').getAttribute('data-take-card');
  const other = open === ID2 ? ID.ready : ID2;
  const tc = () => host.querySelector(`[data-take-card="${open}"]`);
  await click(tc().querySelector('[data-idea-menu]'));
  await click(tc().querySelector('[data-menu-item="delete"]'));
  assert.equal(card(host, open), null, 'hidden while the toast is up');
  assert.ok(host.querySelector(`[data-take-card="${other}"]`), 'the other take opens meanwhile');
  const undo = host.querySelector('[data-undo-toast] [data-undo]');
  await focusOn(undo); // a browser focuses the button it clicks; jsdom doesn't
  await click(undo);
  await settle();
  assert.ok(tc(), 'the take comes back open, as it was');
  assert.equal(card(host, open).querySelector('[data-new-chip]'), null, 'no New chip');
  assert.doesNotMatch(text(host.querySelector('[data-live]')), /take is ready/, 'no arrival announcement');
  assert.notEqual(document.activeElement, document.body, 'focus is not dropped to body');
  assert.equal(document.activeElement, tc().querySelector('[data-action="do-it"]'), 'focus on its Do it');
  assert.ok(!calls.some((c) => c.method === 'DELETE'));
});

for (const [name, id, label] of [
  ['Look again on the open take', ID.ready, 'Look again'],
  ['Look now on a failed slip', ID.failed, 'Look now'],
]) {
  test(`M6: ${name}: when it succeeds, focus is not dropped to body`, async () => {
    reset();
    routes[`POST /api/ideas/${id}/look`] = [202, FIX._examples['POST /api/ideas/:id/look 202']];
    const host = await jar();
    const look = button(card(host, id), label);
    await focusOn(look);
    await click(look);
    await settle();
    assert.equal(sectionOfCard(host, id), 'peeking', 'it moved to Being peeked at');
    assert.ok(document.activeElement && document.activeElement !== document.body, `focus is on ${document.activeElement?.outerHTML?.slice(0, 80)}`);
    assert.ok(host.contains(document.activeElement), 'focus stays in the jar');
  });
}

test('M6: phone take screen: Look again that succeeds keeps focus off body', async () => {
  phoneQuery = true;
  reset();
  routes[`POST /api/ideas/${ID.ready}/look`] = [202, FIX._examples['POST /api/ideas/:id/look 202']];
  const host = await jar();
  await click(card(host, ID.ready).querySelector('button'));
  assert.ok(host.querySelector('[data-take-screen]'), 'take screen open');
  const look = button(host.querySelector('[data-take-screen]'), 'Look again');
  await focusOn(look);
  await click(look);
  await settle();
  assert.ok(document.activeElement && document.activeElement !== document.body, 'focus is not body');
  assert.ok(host.contains(document.activeElement));
});
