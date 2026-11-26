// santa-card-live (autonomy/santa-card-live/brief.md): the Secret Santa host card reads the live pocket. It reads
// GET /api/pockets/:code on show and every 30 s while the tab is visible (never two at once), swaps in the host view
// each host action answers with, keeps what it shows on a network error, and ends cleanly on a 404. The box's own
// host view (phase collecting | drawn | revealed, nested counts and draw, ISO dates) becomes what the card shows.
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
  export { validPocket } from './src/components/results/Pocket';
  export { validSanta, santaFromHost, SANTA_POLL_MS, SANTA_GONE_TEXT } from './src/components/results/PocketSanta';
  export { santaHostView, santaRead, SANTA_LIVE_BLOCK, SANTA_LIVE_JOIN, SANTA_RESET_CODE } from './src/components/results/fixtures';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};

// The 30 s timer: the card's interval is caught here and ticked by hand.
let ticks = [];
const realSetInterval = g.window.setInterval.bind(g.window);
const realClearInterval = g.window.clearInterval.bind(g.window);
g.window.setInterval = (fn, ms, ...rest) => {
  if (ms === m.SANTA_POLL_MS) {
    const t = { fn, live: true };
    ticks.push(t);
    return 900000 + ticks.length;
  }
  return realSetInterval(fn, ms, ...rest);
};
g.window.clearInterval = (id) => {
  if (id > 900000) ticks[id - 900001].live = false;
  else realClearInterval(id);
};
const tick = async () => {
  for (const t of ticks.filter((x) => x.live)) t.fn();
  await settle();
};

let vis = 'visible';
Object.defineProperty(g.document, 'visibilityState', { get: () => vis, configurable: true });
const setVisible = async (v) => {
  vis = v;
  g.document.dispatchEvent(new dom.window.Event('visibilitychange'));
  await settle();
};

const mounted = new Set();
const realFetch = g.fetch;
afterEach(() => {
  for (const v of mounted) v.done();
  mounted.clear();
  ticks = [];
  vis = 'visible';
  g.fetch = realFetch;
});
async function mount(block = structuredClone(m.SANTA_LIVE_BLOCK)) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(h(m.MarkdownRenderer, { content: 'Your office Secret Santa is set up.', results: { version: 1, blocks: [block] }, messageId: 'm-live' }));
  await settle();
  const v = { host, done: () => (root.unmount(), host.remove()) };
  mounted.add(v);
  return v;
}
const click = async (el) => {
  el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const button = (v, text) => [...v.host.querySelectorAll('button, a')].find((b) => b.textContent.trim() === text);
const json = (body, status = 200) => new Response(JSON.stringify(body), { status });
/** A fake box: `reads` answers each GET in turn (the last one repeats); `post` answers the rest. */
function box(reads, post = () => json({})) {
  const calls = [];
  let i = 0;
  g.fetch = async (url, init) => {
    calls.push([init.method, url, init.body ?? null]);
    if (init.method !== 'GET') return post(url, init);
    const r = reads[Math.min(i++, reads.length - 1)];
    return typeof r === 'function' ? r() : json(r);
  };
  return calls;
}
const gets = (calls) => calls.filter((c) => c[0] === 'GET').length;
const read = (n, phase, over) => m.santaRead(m.santaHostView(n, phase, over));

test('the chat block that santa_create hands over (the raw host view) validates and draws the card', () => {
  const b = m.validPocket(structuredClone(m.SANTA_LIVE_BLOCK));
  assert.ok(b && b.santa, 'a santa block');
  assert.equal(b.santa.phase, 'collecting');
  assert.equal(b.santa.joined, 0);
  assert.equal(b.santa.expected, 6);
  assert.deepEqual(b.santa.waiting_for, ['Finn', 'Anna', 'Ben', 'Cleo', 'Dev', 'Esme']);
  assert.deepEqual(b.santa.keep_apart, [['Anna', 'Ben']]);
  assert.equal(b.santa.can_draw, false);
  assert.equal(b.santa.blocked, 'A draw needs at least 3 people.');
  // Still: anything match-like anywhere rejects it.
  const bad = structuredClone(m.SANTA_LIVE_BLOCK);
  bad.santa.people = [{ id: 'o1', name: 'Finn', avatar: 'antenna', looked: false, gift_ready: false, match: 'Anna' }];
  bad.santa.joined = 1;
  bad.santa.counts.joined = 1;
  assert.equal(m.validPocket(bad), null);
});

test('santaFromHost: phases, counts, dates in words, and only an address that stays', () => {
  const now = Date.parse('2099-12-02T12:00:00Z');
  const c = m.santaFromHost(m.santaHostView(6), { now });
  assert.equal(c.phase, 'collecting');
  assert.equal(c.can_draw, true);
  assert.equal(c.looked, null, 'no looked count before the draw');
  assert.equal(c.people[0].looked, null);

  const d = m.santaFromHost(m.santaHostView(6, 'drawn'), { now, pocket: read(6).pocket, join: m.SANTA_LIVE_JOIN });
  assert.equal(d.phase, 'drawn');
  assert.equal(d.looked, 5);
  assert.equal(d.gifts_ready, 1);
  assert.equal(d.asks_asked, 0);
  assert.match(d.drawn_at, /^Tue, Dec 1 at \d{1,2}:00 [AP]M$/);
  assert.match(d.deletes_on, /^(Thu|Fri), Dec 2[45]$/);
  assert.equal(d.join.mode, 'anywhere');
  assert.equal(d.join.anywhere_url, m.SANTA_LIVE_JOIN.anywhere_url);

  // A drawn Santa on the gift day is Reveal day; a revealed one too; an ended pocket is Ended.
  assert.equal(m.santaFromHost(m.santaHostView(6, 'drawn'), { now: Date.parse('2099-12-18T20:00:00') }).phase, 'reveal_day');
  const r = m.santaFromHost(m.santaHostView(6, 'revealed'), { now });
  assert.equal(r.phase, 'reveal_day');
  assert.equal(r.reveal, 'shown');
  const e = m.santaFromHost(m.santaHostView(6, 'revealed'), { now, pocket: { state: 'ended', ended_at: '2099-12-19T10:00:00Z', expires_at: '2099-12-25T00:00:00Z' } });
  assert.equal(e.phase, 'ended');
  assert.equal(e.ended_on, 'Dec 19');

  // A quick tunnel's address changes on restart: dropped, Same Wi-Fi stays.
  const q = m.santaFromHost(m.santaHostView(2), { now, join: { ...m.SANTA_LIVE_JOIN, anywhere_url: 'https://x.trycloudflare.com/p/of6sn7', anywhere_kind: 'quick', quick_tunnel: true } });
  assert.equal(q.join.mode, 'wifi');
  assert.equal(q.join.anywhere_url, null);

  // What the box doesn't say stays as the card knew it: the draw rule, who is "you", a lock.
  const prev = { ...c, draw_rule: 'when all 6 are in', people: c.people.map((p) => ({ ...p, you: p.id === 'o1', locked: p.id === 'o3' })) };
  const k = m.santaFromHost(m.santaHostView(6, 'drawn'), { now, prev });
  assert.equal(k.draw_rule, 'when all 6 are in');
  assert.equal(k.people.find((p) => p.id === 'o1').you, true);
  assert.equal(k.people.find((p) => p.id === 'o3').locked, true);
  // ...unless the box says it.
  const said = m.santaHostView(6, 'drawn');
  said.people[2].locked = false;
  assert.equal(m.santaFromHost(said, { now, prev }).people.find((p) => p.id === 'o3').locked, false);

  assert.equal(m.santaFromHost(m.santaHostView(6, 'drawn', { phase: 'shuffling' }), { now }), undefined, 'an unknown phase');
  assert.equal(m.santaFromHost(m.santaHostView(6, 'drawn', { ring: ['o1', 'o2'] }), { now }), undefined, 'the loop');
  assert.equal(m.santaFromHost(null), undefined);
});

test('joins and the draw show without a reload: 0 of 6 -> 6 of 6 -> names are drawn, on the same card', async () => {
  const calls = box([read(0), read(6), read(6, 'drawn')]);
  const v = await mount();
  const card = v.host.querySelector('[data-santa]');
  assert.equal(gets(calls), 1, 'read once on show');
  assert.equal(calls[0][1], '/api/pockets/of6sn7');
  assert.match(card.textContent, /0 of 6 joined · waiting for 6 more/);
  assert.match(card.textContent, /Uses your Mac's Tailscale Funnel address/, 'the join comes from the read');

  await tick();
  assert.equal(v.host.querySelector('[data-santa]'), card, 'the same card, not a remount');
  assert.match(card.textContent, /6 of 6 joined/);
  assert.equal(button(v, 'Draw now').getAttribute('aria-disabled'), null, 'the box says a draw can run');

  await tick();
  assert.equal(card.dataset.santa, 'drawn');
  assert.match(card.textContent, /Drawn Tue, Dec 1 at .+ · everyone has their match/);
  assert.match(card.textContent, /5 of 6 have looked at their match · Dev hasn't yet/);
  assert.match(card.textContent, /1 of 6 gifts ready/);
  assert.match(card.querySelector('[data-chip], .kw-chip').textContent, /Drawn/);
  assert.equal(gets(calls), 3);
});

test('a host action swaps in the view it answers with, at once', async () => {
  const calls = box([read(4)], (url, init) => (JSON.parse(init.body).name === 'draw_now' ? json(read(4, 'drawn')) : json({})));
  const v = await mount();
  assert.match(v.host.textContent, /4 of 6 joined · waiting for Dev and Esme/);
  await click(button(v, 'Draw now'));
  await click(button(v, 'Draw names'));
  assert.deepEqual(calls.filter((c) => c[0] === 'POST').pop(), ['POST', '/api/pockets/of6sn7/action', JSON.stringify({ name: 'draw_now' })]);
  assert.equal(v.host.querySelector('[data-santa]').dataset.santa, 'drawn');
  assert.match(v.host.textContent, /Drawn\. Everyone can open their own match now\./);
  assert.ok(button(v, 'Open my Santa page'), "the drawn card's buttons are back");
  assert.equal(gets(calls), 1, 'the answer was enough: no extra read');
});

test('Remove swaps in the answer; Reset PIN shows the one-time code and a poll keeps it open', async () => {
  const locked = read(6);
  const calls = box([locked], (url) => (url.endsWith('/remove') ? json(read(5)) : url.endsWith('/reset-pin') ? json(m.SANTA_RESET_CODE) : json({})));
  const block = structuredClone(m.SANTA_LIVE_BLOCK);
  const v = await mount(block);
  await click(button(v, 'People'));
  let panel = v.host.querySelector('[data-people]');
  assert.match(panel.textContent, /People and PINs · 6/);

  await click(panel.querySelector('[data-person-menu="o3"]'));
  await click(button(v, 'Reset PIN'));
  await click(button(v, 'Reset and show the code'));
  assert.equal(panel.querySelector('[data-reset-code] [data-code]').textContent, 'K7QX');
  assert.match(panel.querySelector('[data-reset-code]').textContent, /Send it to Ben yourself/);
  await tick();
  assert.equal(panel.querySelector('[data-reset-code] [data-code]')?.textContent, 'K7QX', 'a read in between keeps the code on screen');
  await click(button(v, 'Done'));

  await click(panel.querySelector('[data-person-menu="o6"]'));
  await click(button(v, 'Remove'));
  await click(button(v, 'Remove Esme'));
  panel = v.host.querySelector('[data-people]');
  assert.match(panel.textContent, /People and PINs · 5/);
  assert.match(v.host.textContent, /5 of 6 joined · waiting for Esme/, 'the count above comes from the answer');
  assert.ok(calls.some((c) => c[1] === '/api/pockets/of6sn7/remove'));
});

test('a 404 read ends the card cleanly; reads stop', async () => {
  let gone = false;
  const calls = box([() => (gone ? json({ error: 'not_found', message: "There's no pocket here." }, 404) : json(read(3)))]);
  const v = await mount();
  assert.match(v.host.textContent, /3 of 6 joined/);
  gone = true;
  await tick();
  const card = v.host.querySelector('[data-santa]');
  assert.equal(card.dataset.santa, 'ended');
  assert.equal(card.dataset.gone, 'true');
  assert.equal(v.host.querySelector('[data-done="gone"]').textContent, m.SANTA_GONE_TEXT);
  assert.equal(card.querySelector('[data-join]'), null, 'no link to a pocket that is gone');
  assert.equal(button(v, 'Copy invite'), undefined, 'no actions left');
  assert.equal(button(v, 'Draw now'), undefined);
  const n = gets(calls);
  await tick();
  await setVisible('visible');
  assert.equal(gets(calls), n, 'no reads after it is gone');
});

test('only the box\'s own not_found ends the card: Pocket apps off or a bare 404 keep the view and keep reading', async () => {
  for (const answer of [
    () => json({ error: 'pocket_apps_off', message: 'Pocket apps are off. Turn them on in Settings first.' }, 404),
    () => new Response('nope', { status: 404 }),
  ]) {
    let off = false;
    const calls = box([() => (off ? answer() : json(read(3)))]);
    const v = await mount();
    assert.match(v.host.textContent, /3 of 6 joined/);
    off = true;
    await tick();
    const card = v.host.querySelector('[data-santa]');
    assert.notEqual(card.dataset.gone, 'true', 'not "deleted"');
    assert.equal(card.dataset.santa, 'collecting');
    assert.equal(v.host.querySelector('[data-done="gone"]'), null);
    assert.match(v.host.textContent, /3 of 6 joined/, 'the last view stays');
    const n = gets(calls);
    off = false;
    await tick();
    assert.equal(gets(calls), n + 1, 'it keeps reading');
    for (const x of mounted) x.done();
    mounted.clear();
    ticks = [];
  }
});

test('gone after an action: the gone line replaces the action line (D1)', async () => {
  let gone = false;
  box([() => (gone ? json({ error: 'not_found', message: "There's no pocket here." }, 404) : json(read(4)))], () => json(read(4, 'drawn')));
  const v = await mount();
  await click(button(v, 'Draw now'));
  await click(button(v, 'Draw names'));
  assert.ok(v.host.querySelector('[data-done="ok"]'), 'the draw line first');
  gone = true;
  await tick();
  assert.equal(v.host.querySelector('[data-done="gone"]').textContent, m.SANTA_GONE_TEXT);
  assert.equal(v.host.querySelector('[data-done="ok"]'), null, 'no green Drawn line under an Ended chip');
});

test('a network error keeps the last view quietly, and the next tick tries again', async () => {
  let down = false;
  const calls = box([() => (down ? Promise.reject(new TypeError('Failed to fetch')) : json(read(2)))]);
  const v = await mount();
  assert.match(v.host.textContent, /2 of 6 joined/);
  down = true;
  await tick();
  assert.match(v.host.textContent, /2 of 6 joined/);
  assert.equal(v.host.querySelector('[role=alert], [data-done="error"]'), null, 'quiet');
  down = false;
  g.fetch = (() => {
    const f = g.fetch;
    return async (url, init) => (init.method === 'GET' ? (calls.push(['GET', url, null]), json(read(3))) : f(url, init));
  })();
  await tick();
  assert.match(v.host.textContent, /3 of 6 joined/);
});

test('reads stop while the tab is hidden, come back at once when it shows, and never overlap', async () => {
  let release;
  let slow = false;
  const calls = box([() => (slow ? new Promise((r) => (release = () => r(json(read(4))))) : json(read(1)))]);
  const v = await mount();
  assert.equal(gets(calls), 1);
  await setVisible('hidden');
  await tick();
  await tick();
  assert.equal(gets(calls), 1, 'no reads while hidden');
  await setVisible('visible');
  assert.equal(gets(calls), 2, 'one read as soon as it shows');

  slow = true;
  await tick();
  await tick();
  await tick();
  assert.equal(gets(calls), 3, 'one read at a time while a slow one is out');
  release();
  await settle();
  assert.match(v.host.textContent, /4 of 6 joined/);
});

test('a read that started before an action never paints over the action’s answer', async () => {
  let release;
  const calls = box(
    [read(4), () => new Promise((r) => (release = () => r(json(read(4)))))],
    () => json(read(4, 'drawn')),
  );
  const v = await mount();
  await tick(); // a read is now out, slowly
  await click(button(v, 'Draw now'));
  await click(button(v, 'Draw names'));
  assert.equal(v.host.querySelector('[data-santa]').dataset.santa, 'drawn');
  release();
  await settle();
  assert.equal(v.host.querySelector('[data-santa]').dataset.santa, 'drawn', 'the older read is dropped');
  assert.equal(gets(calls), 2);
});
