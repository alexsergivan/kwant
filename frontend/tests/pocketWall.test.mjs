// Photo wall Pocket card (pa14, autonomy/pocket-apps/pa14/design.md §1, §4, §5, card-brief.md): validWall for a good
// block per state, missing fields falling back, junk rejected (a guest id, a file size or a photo list anywhere in
// the block), optimistic Approve and Hide with rollback, the 5 s Undo on hiding a waiting photo, Approve all from one
// guest, one Needs-you item per wall, the Picked holo only when earned, buttons with no route yet saying why, and Snap.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.matchMedia = g.window.matchMedia;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { MarkdownRenderer } from './src/components/MarkdownRenderer';
  export { validPocket, kitLabel } from './src/components/results/Pocket';
  export { WallCard, WallDoneRow, validWall, validWallView, wallStage, wallNeedsYou, wallDoneText, earnedPicked, SNAP_MOOD, WALL_LATER, WALL_FORBIDDEN, photosLeft, keepLine, meterLine } from './src/components/results/PocketWall';
  export { WALL_FIXTURES, WALL_HOST_VIEWS, POCKET_FIXTURES, SANTA_FIXTURES } from './src/components/results/fixtures';
  export { wallZipUrl, wallPhotoUrl } from './src/pocketApi';
  export { pickCreature, AVATAR_STYLES } from './src/components/ModernAvatar';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const mounted = new Set();
const realFetch = g.fetch;
afterEach(() => {
  for (const v of mounted) v.done();
  mounted.clear();
  g.fetch = realFetch;
});

const F = m.WALL_FIXTURES;
const raw = (name) => structuredClone(F[name].results.blocks[0]);
const withWall = (name, over) => {
  const b = raw(name);
  b.wall = { ...b.wall, ...over };
  return b;
};
const ok = (b) => assert.ok(m.validPocket(b), 'should validate');
const bad = (b, why) => assert.equal(m.validPocket(b), null, why);

/**
 * The box: `GET /api/pockets/:code` answers with the fixture's host view; actions go to `onAction`, which may answer
 * with a new host view, throw (network down) or hold the answer until the test lets it go.
 */
function box(name, onAction = null) {
  const calls = [];
  let hv = structuredClone(m.WALL_HOST_VIEWS[name]);
  g.fetch = async (url, init = {}) => {
    const method = init.method ?? 'GET';
    const body = init.body ? JSON.parse(init.body) : null;
    calls.push({ url: String(url), method, body });
    if (method === 'GET') return new Response(JSON.stringify({ pocket: {}, summary: {}, removed: 0, host_view: hv, join: null }), { status: 200 });
    if (onAction) {
      const r = await onAction({ url: String(url), method, body, hv });
      if (r === 'fail') throw new TypeError('network down');
      if (r === 'gone') return new Response(JSON.stringify({ error: 'not_now', message: "That can't happen right now." }), { status: 409 });
      if (r) hv = r;
    }
    return new Response(JSON.stringify({ pocket: {}, summary: {}, removed: 0, host_view: hv, join: null }), { status: 200 });
  };
  return calls;
}
const actions = (calls) => calls.filter((c) => c.method === 'POST' && c.url.endsWith('/action')).map((c) => c.body);
async function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  const v = { host, done: () => (root.unmount(), host.remove()) };
  mounted.add(v);
  return v;
}
const card = (name, opts = {}) => mount(h(m.WallCard, { block: m.validPocket(opts.block ?? raw(name)), ask: opts.ask ?? (() => {}), undoMs: opts.undoMs ?? 5000 }));
const inChat = (name, block = raw(name)) => mount(h(m.MarkdownRenderer, { content: F[name].reply, results: { version: 1, blocks: [block] }, messageId: `m-${name}` }));
const click = async (el) => {
  el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const button = (v, text) => [...v.host.querySelectorAll('button, a')].find((b) => b.textContent.trim() === text);
const tile = (v, label) => Number(v.host.querySelector(`[data-tile="${label}"]`).textContent.replace(label, '').replace(/,/g, ''));
const rows = (v) => [...v.host.querySelectorAll('[data-row]:not([data-leaving])')].map((r) => r.dataset.row);
const holos = (v) => v.host.querySelectorAll('.kw-sticker-holo, [data-tone="holo"]').length;

// ---------------------------------------------------------------------------------------------------
// The validator

test('a good block per state: each validates, routes to the wall card and shows its stage and Snap mood', async () => {
  const stages = { ready: 'ready', waiting: 'waiting', caughtUp: 'caught_up', space: 'space', picking: 'picking', picked: 'picked', shown: 'shown', shownByHost: 'shown' };
  const moods = { ready: 'idle', waiting: 'needs_you', caught_up: 'working', space: 'needs_you', picking: 'thinking', picked: 'finished', shown: 'finished' };
  for (const [name, stage] of Object.entries(stages)) {
    const b = m.validPocket(raw(name));
    assert.ok(b, `${name} validates`);
    assert.equal(m.wallStage(b.wall), stage, name);
    box(name);
    const v = await inChat(name);
    const section = v.host.querySelector('[data-wall]');
    assert.ok(section, `${name} renders the wall card`);
    assert.equal(section.dataset.wall, stage);
    assert.equal(v.host.querySelector('[data-snap]').dataset.snap, moods[stage], `${name}: Snap is ${moods[stage]}`);
    assert.match(v.host.textContent, /Pocket app · Photo wall/);
    v.done();
    mounted.delete(v);
  }
  assert.equal(m.kitLabel('wall'), 'Photo wall');
  // The other kits never grow a wall.
  for (const fx of [m.POCKET_FIXTURES.live, m.SANTA_FIXTURES.collecting]) assert.equal(m.validPocket(fx.results.blocks[0]).wall, undefined);
});

test('missing optional fields fall back: dates, host name, limits, keep days, best_max, captions', () => {
  const b = raw('waiting');
  for (const k of ['closes_at', 'delete_on', 'host_name', 'best_by', 'photo_cap_mb', 'per_guest_max', 'keep_days', 'best_max', 'captions', 'best', 'best_set', 'picking']) delete b.wall[k];
  const w = m.validPocket(b).wall;
  assert.equal(w.closes_at, null);
  assert.equal(w.delete_on, null);
  assert.equal(w.host_name, null);
  assert.equal(w.keep_days, 30);
  assert.equal(w.best_max, 30);
  assert.equal(w.captions, true);
  assert.deepEqual(w.best, []);
  assert.equal(w.best_set, false);
  assert.equal(m.keepLine(w), 'Photos go to you. They leave this box 30 days after the party: download them before then.');
  assert.equal(m.meterLine(w), '452 MB of 1,024 MB photo space used');
  const full = m.validPocket(raw('waiting')).wall;
  assert.equal(m.keepLine(full), 'Photos go to you. They leave this box on Tue, Jan 5 (30 days after the party): download them before then.');
  assert.equal(m.meterLine(full), '452 MB of 1,024 MB photo space used · up to 15 MB a photo, 40 a guest');
  // picking only counts while closed with nothing proposed.
  assert.equal(m.validPocket(withWall('waiting', { picking: true })).wall.picking, false);
  assert.equal(m.validPocket(withWall('picked', { picking: true })).wall.picking, false);
});

test('junk is rejected: a guest id, a file size or a photo list anywhere in the block, and contradictions', () => {
  ok(raw('waiting'));
  for (const k of m.WALL_FORBIDDEN) {
    bad(withWall('waiting', { [k]: 'x' }), `wall.${k}`);
    bad(withWall('waiting', { extra: { deep: [{ [k]: 1 }] } }), `nested ${k}`);
  }
  bad(withWall('waiting', { queue: m.WALL_HOST_VIEWS.waiting.queue }), 'the photo list never rides in the block');
  const kitless = raw('waiting');
  delete kitless.wall;
  bad(kitless, 'kit wall without a wall');
  const quizWithWall = structuredClone(m.POCKET_FIXTURES.live.results.blocks[0]);
  quizWithWall.wall = raw('waiting').wall;
  bad(quizWithWall, 'a quiz carrying a wall');
  bad({ ...raw('waiting'), state: 'ended' }, 'open wall must be live');
  bad({ ...raw('shown'), state: 'live' }, 'closed wall must be ended');
  bad(withWall('waiting', { state: 'paused' }), 'unknown wall state');
  bad(withWall('waiting', { counts: { ...raw('waiting').wall.counts, waiting: 10 } }), 'counts that do not add up');
  bad(withWall('waiting', { counts: { ...raw('waiting').wall.counts, guests: -1 } }), 'negative count');
  bad(withWall('waiting', { used_mb: -1 }), 'negative MB');
  bad(withWall('waiting', { pocket_cap_mb: 0 }), 'no cap');
  bad(withWall('waiting', { used_mb: '452' }), 'MB as text');
  bad(withWall('shown', { best: Array.from({ length: 31 }, (_, i) => `ph_${i}`) }), 'more than best_max');
  bad(withWall('shown', { best: ['../etc/passwd'] }), 'a photo id that is not an id');
  bad(withWall('waiting', { best_set: 'yes' }), 'best_set as text');
  bad(withWall('waiting', { closes_at: 42 }), 'closes_at not text');
  bad({ ...raw('waiting'), results: structuredClone(m.POCKET_FIXTURES.results.results.blocks[0].results) }, 'quiz results on a wall');
});

test('a removed guest who sent photos (senders > guests) still shows the card (Rigel D1)', async () => {
  const b = withWall('waiting', { counts: { total: 12, waiting: 3, approved: 8, hidden: 1, guests: 4, senders: 5 } });
  const w = m.validPocket(b)?.wall;
  assert.ok(w, 'removing a guest who uploaded is a normal state');
  assert.equal(w.counts.senders, 5);
  assert.equal(w.counts.guests, 4);
  box('waiting');
  const v = await inChat('waiting', b);
  assert.ok(v.host.querySelector('[data-wall="waiting"]'), 'the card renders');
});

test('best_set with an empty best list (the last best photo was hidden) reads as no best set (Rigel D2)', () => {
  const w = m.validPocket(withWall('shown', { best: [] }))?.wall;
  assert.ok(w, 'the card stays');
  assert.equal(w.best_set, false);
  const hv = { ...structuredClone(m.WALL_HOST_VIEWS.shown), best: [], best_set: true };
  assert.equal(m.validWallView(hv).best_set, false);
});

test('the space line takes the per-photo and per-guest caps from host_view.limits when the summary lacks them (D5)', async () => {
  const b = raw('waiting');
  delete b.wall.photo_cap_mb;
  delete b.wall.per_guest_max;
  assert.equal(m.meterLine(m.validPocket(b).wall), '452 MB of 1,024 MB photo space used');
  box('waiting', null);
  const hv = m.WALL_HOST_VIEWS.waiting;
  g.fetch = ((f) => async (url, init) => {
    const res = await f(url, init);
    const data = await res.json();
    data.host_view = { ...hv, limits: { photo_cap_mb: 15, pocket_cap_mb: 1024, per_guest_max: 40, caption_max: 80 } };
    return new Response(JSON.stringify(data), { status: 200 });
  })(g.fetch);
  const v = await card('waiting', { block: b });
  assert.match(v.host.textContent, /452 MB of 1,024 MB photo space used · up to 15 MB a photo, 40 a guest/);
});

test('the host view: bad photos are dropped, turns kept, and a non-wall view is null', () => {
  const hv = structuredClone(m.WALL_HOST_VIEWS.waiting);
  hv.queue.push({ id: 'bad id!', by: 'x', by_name: 'X' }, 'junk', { id: 'ph_x', by: 'pl_x', by_name: 'Xu Li', creature: 13, rotate: 45 });
  const v = m.validWallView(hv);
  assert.equal(v.queue.length, 10);
  const x = v.queue.at(-1);
  assert.equal(x.creature, 5, 'creature wraps to 0-7');
  assert.equal(x.rotate, 0, 'an odd turn is ignored');
  assert.equal(v.counts.waiting, 9);
  assert.equal(m.validWallView({ phase: 'drawn' }), null);
  assert.equal(m.wallPhotoUrl('wl8m3q', 'ph_q1'), '/api/pockets/wl8m3q/photo/ph_q1');
  assert.equal(m.wallZipUrl('wl8m3q'), '/api/pockets/wl8m3q/wall.zip');
  assert.equal(m.wallZipUrl('wl8m3q', 'all'), '/api/pockets/wl8m3q/wall.zip?all=1');
  assert.equal(m.wallZipUrl('wl8m3q', 'best'), '/api/pockets/wl8m3q/wall.zip?best=1');
});

// ---------------------------------------------------------------------------------------------------
// The queue: optimistic approve and hide

test('the queue: groups per guest, 5 rows then "Show 4 more waiting", counts and tabs', async () => {
  box('waiting');
  const v = await card('waiting');
  assert.equal(tile(v, 'Waiting for you'), 9);
  assert.equal(tile(v, 'On the wall'), 119);
  assert.ok(v.host.querySelector('[data-tile="Waiting for you"][data-on]'), 'waiting tile is outlined');
  assert.match(v.host.textContent, /Live · 9 waiting/);
  assert.match(v.host.textContent, /Mia · 4 waiting/);
  assert.ok(button(v, 'Approve all 4'));
  assert.equal(rows(v).length, 5);
  assert.ok(button(v, 'Show 4 more waiting'));
  await click(button(v, 'Show 4 more waiting'));
  assert.equal(rows(v).length, 9);
  const tabs = [...v.host.querySelectorAll('[role="tab"]')].map((t) => t.textContent);
  assert.deepEqual(tabs, ['Waiting · 9', 'On the wall · 119']);
  assert.match(v.host.textContent, /“?First dance!”?/);
  assert.match(v.host.textContent, /No caption/);
  await click(v.host.querySelector('[data-tab="wall"]'));
  assert.equal(v.host.querySelectorAll('[data-grid] li').length, 15);
  assert.ok(button(v, 'Show all 119'));
  assert.match(v.host.textContent, /Hide takes a photo off the TV and every phone at once and deletes it\./);
});

test('every guest in the queue gets a group row, N = 1 too, with no Approve all for one photo', async () => {
  box('waiting');
  const v = await card('waiting');
  const heads = [...v.host.querySelectorAll('[data-group-header]')];
  const groupsWithRows = [...v.host.querySelectorAll('[data-group]')].filter((li) => li.querySelector('[data-row]'));
  assert.equal(heads.length, groupsWithRows.length, 'one header per guest with rows shown');
  const one = heads.find((hd) => / · 1 waiting/.test(hd.textContent));
  assert.ok(one, 'a guest with 1 waiting has a header');
  assert.ok(!one.querySelector('[data-action="approve-all"]'), 'no Approve all for 1');
  assert.ok(!one.querySelector('.max-sm\\:sr-only'), 'the count stays visible on phones for 1');
  assert.match(one.className, /min-h-\[44px\]/);
  assert.match(one.className, /border-t/, 'a hairline above a later group');
  assert.equal(rows(v).length, 5, 'the 5-row cap counts photos, not headers');
});

test('Ready: the wall join box speaks of guests and photos, with the hint and the strip promise; quiz is unchanged', async () => {
  const v = await inChat('ready');
  const t = v.host.textContent;
  assert.match(t, /How guests add photos/);
  assert.doesNotMatch(t, /How friends join|quiz night|play in their phone/);
  assert.match(t, /Print the QR for the tables, or put it on the TV: the slideshow shows it in a corner\./);
  assert.match(t, /Nothing shows on the TV or other phones until you approve it\. Location and camera details are removed when a photo arrives\./);
  assert.match(t, /Guests on Wi-Fi or mobile data scan the code and add photos in their phone's browser\. No app, no account\./);
  assert.match(t, /Fine for a party of up to 60 guests\./);
  const wifi = [...v.host.querySelectorAll('[role="radio"]')].find((r) => r.textContent.includes('Same Wi-Fi'));
  await click(wifi);
  assert.match(v.host.textContent, /Guests on your Wi-Fi scan the code and add photos in their phone's browser\. No app, no account\./);
  assert.match(v.host.textContent, /At a venue, pick From anywhere\./);
  const q = await mount(h(m.MarkdownRenderer, { content: 'x', results: { version: 1, blocks: [structuredClone(Object.values(m.POCKET_FIXTURES).find((f) => f.results?.blocks?.[0]?.join?.wifi_url)?.results.blocks[0])] }, messageId: 'm-quiz' }));
  assert.match(q.host.textContent, /How friends join/);
  assert.doesNotMatch(q.host.textContent, /How guests add photos|Print the QR for the tables/);
});

test('approve is optimistic: the row goes and counts move at once; on failure it comes back with a plain line', async () => {
  let release;
  const gate = new Promise((r) => (release = r));
  const calls = box('waiting', async () => {
    await gate;
    return 'fail';
  });
  const v = await card('waiting');
  const first = v.host.querySelector('[data-row="ph_q1"] [data-action="approve"]');
  await click(first);
  assert.ok(!rows(v).includes('ph_q1'), 'row leaves at once');
  assert.equal(tile(v, 'Waiting for you'), 8);
  assert.equal(tile(v, 'On the wall'), 120);
  assert.deepEqual(actions(calls), [{ name: 'approve', body: { id: 'ph_q1' } }]);
  release();
  await settle();
  assert.ok(rows(v).includes('ph_q1'), 'row comes back');
  assert.equal(tile(v, 'Waiting for you'), 9);
  const err = v.host.querySelector('[data-row="ph_q1"] [data-row-error]');
  assert.equal(err.textContent.trim(), "Couldn't reach the box. Try again");
  assert.ok(!v.host.querySelector('.text-bad.kw-chip, [data-chip="error"]'), 'no error chip, a plain line');
});

test('approve answered 409 not_now: a plain "not waiting any more" line, never Try again (D3)', async () => {
  const calls = box('waiting', async () => 'gone');
  const v = await card('waiting');
  await click(v.host.querySelector('[data-row="ph_q1"] [data-action="approve"]'));
  await settle();
  const err = v.host.querySelector('[data-row="ph_q1"] [data-row-error]');
  assert.equal(err?.textContent.trim(), "That photo isn't waiting any more.");
  assert.ok(calls.filter((c) => c.method === 'GET').length >= 2, 'the card reads the box again');
});

test('approve that works takes the box answer as the new lists', async () => {
  const calls = box('waiting', ({ hv, body }) => {
    const p = hv.queue.find((q) => q.id === body.body.id);
    return { ...hv, counts: { ...hv.counts, waiting: 8, approved: 120 }, queue: hv.queue.filter((q) => q !== p), wall: [{ ...p, status: 'approved' }, ...hv.wall] };
  });
  const v = await card('waiting');
  await click(v.host.querySelector('[data-row="ph_q3"] [data-action="approve"]'));
  await sleep(200);
  await settle();
  assert.equal(actions(calls).length, 1);
  assert.ok(!rows(v).includes('ph_q3'));
  assert.equal(tile(v, 'Waiting for you'), 8);
  assert.equal(tile(v, 'On the wall'), 120);
  assert.ok(!v.host.querySelector('[data-row-error]'));
});

test('Approve all N from one guest sends approve_all_from with that guest, never one call per photo', async () => {
  const calls = box('waiting', ({ hv, body }) => {
    const mine = hv.queue.filter((q) => q.by === body.body.guest);
    return { ...hv, counts: { ...hv.counts, waiting: hv.queue.length - mine.length, approved: hv.counts.approved + mine.length }, queue: hv.queue.filter((q) => q.by !== body.body.guest), wall: [...mine, ...hv.wall] };
  });
  const v = await card('waiting');
  await click(button(v, 'Approve all 4'));
  assert.deepEqual(actions(calls), [{ name: 'approve_all_from', body: { guest: 'pl_mia' } }]);
  await sleep(200);
  await settle();
  assert.equal(tile(v, 'Waiting for you'), 5);
  assert.ok(!v.host.textContent.includes('Mia · '), 'Mia has nothing waiting');
  assert.ok(button(v, 'Approve all 2'), "Priya's group is next");
});

test('Hide on a waiting photo: 5 s Undo, nothing sent until it runs out; Undo brings it back', async () => {
  const calls = box('waiting');
  const v = await card('waiting', { undoMs: 120 });
  await click(v.host.querySelector('[data-row="ph_q3"] [data-action="hide"]'));
  assert.match(v.host.textContent, /Hid Leo's photo\. It's deleted in 1 s, and nobody else saw it\./);
  assert.equal(document.activeElement?.dataset.action, 'undo', 'focus moves to Undo');
  assert.equal(tile(v, 'Waiting for you'), 8);
  assert.equal(actions(calls).length, 0, 'nothing sent yet');
  await click(button(v, 'Undo'));
  assert.ok(rows(v).includes('ph_q3'));
  assert.equal(tile(v, 'Waiting for you'), 9);
  await sleep(200);
  assert.equal(actions(calls).length, 0, 'an undone hide is never sent');
  // Let it run out this time.
  await click(v.host.querySelector('[data-row="ph_q3"] [data-action="hide"]'));
  await sleep(200);
  await settle();
  assert.deepEqual(actions(calls), [{ name: 'hide', body: { id: 'ph_q3' } }]);
  assert.ok(!v.host.querySelector('[data-undo]'));
});

test('the Undo text counts the real 5 s down from 5', async () => {
  box('waiting');
  const v = await card('waiting');
  await click(v.host.querySelector('[data-row="ph_q3"] [data-action="hide"]'));
  assert.match(v.host.textContent, /It's deleted in 5 s/);
});

test('approving another photo, or leaving the card, sends a pending hide at once', async () => {
  const calls = box('waiting');
  const v = await card('waiting', { undoMs: 60_000 });
  await click(v.host.querySelector('[data-row="ph_q3"] [data-action="hide"]'));
  await click(v.host.querySelector('[data-row="ph_q1"] [data-action="approve"]'));
  assert.deepEqual(actions(calls), [
    { name: 'hide', body: { id: 'ph_q3' } },
    { name: 'approve', body: { id: 'ph_q1' } },
  ]);
  await click(v.host.querySelector('[data-row="ph_q2"] [data-action="hide"]'));
  v.done();
  mounted.delete(v);
  await settle();
  assert.deepEqual(actions(calls).at(-1), { name: 'hide', body: { id: 'ph_q2' } });
});

test('a failed hide brings the row back with the plain line', async () => {
  box('waiting', () => 'fail');
  const v = await card('waiting', { undoMs: 30 });
  await click(v.host.querySelector('[data-row="ph_q3"] [data-action="hide"]'));
  await sleep(80);
  await settle();
  assert.ok(rows(v).includes('ph_q3'));
  assert.match(v.host.querySelector('[data-row="ph_q3"]').textContent, /Couldn't reach the box\. Try again/);
});

test('Hide on an approved photo is immediate (no Undo)', async () => {
  const calls = box('caughtUp', ({ hv, body }) => ({ ...hv, counts: { ...hv.counts, approved: hv.counts.approved - 1, hidden: hv.counts.hidden + 1 }, wall: hv.wall.filter((p) => p.id !== body.body.id) }));
  const v = await card('caughtUp');
  assert.equal(v.host.querySelector('[aria-selected="true"]').dataset.tab, 'wall', 'all caught up opens on the wall');
  await click(v.host.querySelector('[data-grid] [data-action="hide-approved"]'));
  assert.deepEqual(actions(calls), [{ name: 'hide', body: { id: 'ph_w212' } }]);
  assert.ok(!v.host.querySelector('[data-undo]'));
  assert.equal(tile(v, 'On the wall'), 127);
  await settle();
  assert.equal(tile(v, 'On the wall'), 127, 'the box answer agrees');
});

test('the box not answering the first read: a plain line and Try again', async () => {
  g.fetch = async () => {
    throw new TypeError('down');
  };
  const v = await card('waiting');
  assert.match(v.host.querySelector('[data-load-error]').textContent, /Couldn't reach the box\. Try again/);
  assert.equal(tile(v, 'Waiting for you'), 9, 'counts from the block meanwhile');
  box('waiting');
  await click(button(v, 'Try again'));
  assert.equal(rows(v).length, 5);
});

// ---------------------------------------------------------------------------------------------------
// Needs you, Done, holo, actions, Snap

test('one Needs-you item per wall while photos wait, never one per photo', () => {
  const a = m.validPocket(raw('waiting'));
  const older = m.validPocket(withWall('waiting', { counts: { total: 3, waiting: 2, approved: 1, hidden: 0, guests: 3, senders: 2 } }));
  const other = m.validPocket({ ...raw('caughtUp'), code: 'zz9kk1' });
  const quiz = m.validPocket(m.POCKET_FIXTURES.live.results.blocks[0]);
  const items = m.wallNeedsYou([older, a, other, quiz]);
  assert.equal(items.length, 1);
  assert.equal(items[0].text, '9 photos waiting on Mia & Jonas’s wedding');
  assert.equal(items[0].code, 'wl8m3q');
  const one = m.validPocket(withWall('waiting', { counts: { total: 1, waiting: 1, approved: 0, hidden: 0, guests: 1, senders: 1 } }));
  assert.equal(m.wallNeedsYou([one])[0].text, '1 photo waiting on Mia & Jonas’s wedding');
  assert.deepEqual(m.wallNeedsYou([other]), []);
});

test('holo only where earned: Picked only when Snap picked the shown best set; never on picked, host picks or warnings', async () => {
  const expect = { ready: 0, waiting: 0, caughtUp: 0, space: 0, picking: 0, picked: 0, shown: 1, shownByHost: 0 };
  for (const [name, n] of Object.entries(expect)) {
    box(name);
    const v = await card(name);
    assert.equal(holos(v), n, `${name}: ${n} holo`);
    v.done();
    mounted.delete(v);
  }
  assert.equal(m.earnedPicked({ best_set: true, best_by: 'snap' }), true);
  assert.equal(m.earnedPicked({ best_set: false, best_by: 'snap' }), false);
  assert.equal(m.earnedPicked({ best_set: true, best_by: 'host' }), false);
  box('space');
  const s = await card('space');
  assert.ok(s.host.querySelector('[data-chip="space"].kw-chip-warn'), 'space is a plain warn chip');
});

test('best 30: Snap picked copy, strip with +N, and Show the 30 calls set_best with the ids', async () => {
  const calls = box('picked', ({ hv }) => ({ ...hv, best_set: true }));
  const v = await card('picked');
  assert.match(v.host.textContent, /Snap's best 30/);
  assert.match(v.host.textContent, /Snap picked these for sharp, smiling, and not too alike\. Swap any, then show them to guests\./);
  assert.equal(v.host.querySelectorAll('[data-strip] li').length, 10, '9 + "+21" on the phone');
  assert.match(v.host.querySelector('[data-strip]').textContent, /\+21/);
  await click(button(v, 'Show the 30 to guests'));
  const sent = actions(calls)[0];
  assert.equal(sent.name, 'set_best');
  assert.equal(sent.body.ids.length, 30);
  assert.equal(v.host.querySelector('[data-wall]').dataset.wall, 'shown');
  assert.match(v.host.textContent, /Guests see these 30 on their page now\./);
});

test('a host pick reads "Your best 30" with no sticker', async () => {
  box('shownByHost');
  const v = await card('shownByHost');
  assert.match(v.host.textContent, /Your best 30/);
  assert.equal(holos(v), 0);
});

test('actions per state, downloads, Delete now asks once, and buttons with no route say why', async () => {
  const labels = (v) => [...v.host.querySelectorAll('[data-pocket] > div > div:last-child button, [data-pocket] > div > div:last-child a')].map((b) => b.textContent.trim()).filter(Boolean);
  box('ready');
  let v = await card('ready');
  assert.deepEqual(labels(v), ['Share link', 'Big screen', 'Try adding one', 'Edit', 'Stop']);
  assert.match(v.host.textContent, /Ready · no photos yet/);
  v.done();
  box('waiting');
  v = await card('waiting');
  assert.deepEqual(labels(v).slice(0, 3), ['Big screen', 'Download all', 'Share link']);
  assert.equal(button(v, 'Download all').getAttribute('href'), '/api/pockets/wl8m3q/wall.zip');
  await click(v.host.querySelector('[data-action="download-menu"]'));
  assert.equal(button(v, 'Include the 9 waiting').getAttribute('href'), '/api/pockets/wl8m3q/wall.zip?all=1');
  v.done();
  box('space');
  v = await card('space');
  const two = button(v, 'Make it 2 GB');
  assert.equal(two.getAttribute('aria-disabled'), 'true');
  assert.equal(two.dataset.later, m.WALL_LATER.twoGb);
  assert.match(v.host.textContent, /About 20 photos left\. When the wall is full, guests read 'The wall is full for now\. Oleks can make room\.' Nothing is lost that's already here\./);
  assert.ok(!/GB free\b.*This Mac/.test(v.host.textContent), 'no free-disk number: the box does not send one');
  v.done();
  box('picking');
  v = await card('picking');
  assert.match(v.host.textContent, /Closed · Sun 12 PM/);
  assert.match(v.host.textContent, /Snap is picking the best 30 from the 212\. You look them over before guests see them\./);
  assert.ok(button(v, 'Download all · 212'));
  assert.ok(button(v, 'Open the wall'));
  v.done();
  box('picked');
  v = await card('picked');
  assert.equal(button(v, 'Swap photos').dataset.later, m.WALL_LATER.swap);
  v.done();
  const calls = box('shown');
  v = await card('shown');
  assert.equal(button(v, 'Download the best 30').getAttribute('href'), '/api/pockets/wl8m3q/wall.zip?best=1');
  await click(button(v, 'Delete now'));
  const dialog = v.host.querySelector('[role="alertdialog"]');
  assert.equal(document.activeElement?.textContent.trim(), 'Keep them', 'focus lands on the safe answer, not on delete (D4)');
  assert.match(dialog.textContent, /Delete all 212 photos from this box now\? Download them first if you want to keep them\./);
  await click(button(v, 'Delete them now'));
  assert.ok(calls.some((c) => c.method === 'DELETE' && c.url === '/api/pockets/wl8m3q'));
  assert.match(v.host.textContent, /Deleted\./);
});

test('Done row: count, guests and best 30 shown with Picked; a kept wall says when it leaves, no sticker', async () => {
  const shown = m.validPocket(raw('shown'));
  assert.equal(m.wallDoneText(shown), 'Mia & Jonas’s wedding · 212 photos from 41 guests · best 30 shown');
  let v = await mount(h(m.WallDoneRow, { block: shown }));
  assert.equal(holos(v), 1);
  assert.ok(v.host.querySelector('[data-mood], [aria-label^="Snap"]'), 'Snap is on the row');
  v.done();
  const kept = m.validPocket(withWall('picking', { delete_on: 'Dec 28' }));
  assert.equal(m.wallDoneText(kept), 'Mia & Jonas’s wedding · 212 photos from 41 guests · leaves this box Dec 28');
  v = await mount(h(m.WallDoneRow, { block: kept }));
  assert.equal(holos(v), 0);
  const host = m.validPocket(raw('shownByHost'));
  v = await mount(h(m.WallDoneRow, { block: host }));
  assert.equal(holos(v), 0, 'a host pick earns no sticker');
});

test('Snap: a picker style "Photo wall", pink with the camera; no other style holds the camera', () => {
  assert.ok(m.AVATAR_STYLES.some((s) => s.id === 'snap' && s.label === 'Photo wall'));
  const c = m.pickCreature('snap', 'Anything');
  assert.equal(c.accessory, 'camera');
  for (const s of m.AVATAR_STYLES) if (s.id !== 'snap') assert.notEqual(m.pickCreature(s.id, 'x').accessory, 'camera', s.id);
  assert.notEqual(m.pickCreature('default', 'Snap the photo bot').accessory, 'camera', 'picked style only');
});

test('space left reads in tens past 10', () => {
  const w = m.validPocket(raw('space')).wall;
  assert.equal(m.photosLeft(w), 20);
});
