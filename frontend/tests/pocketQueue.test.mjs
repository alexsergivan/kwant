// Queue ticket Pocket card (pa11b, autonomy/pocket-apps/pa11/queue-design.md §1, queue-card-brief.md, queue-impl.md):
// validQueue reads Vega's host view (missing fields fall back, junk is rejected), the states draw, Call next and
// Served · call next send the right action, Skip and Bring back, Remove hides at once with a 5 s Undo, and N calls
// the next number only while the card has focus (never from a text field).
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'HTMLTextAreaElement', 'HTMLInputElement']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.matchMedia = g.window.matchMedia;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { validPocket, kitLabel } from './src/components/results/Pocket';
  export { validQueue, queueFromRead, queueMood, QueueCard, hourLabel, QUEUE_UNDO_MS, QUEUE_CLOSE_DEFAULT, QUEUE_POLL_MS, oneLine } from './src/components/results/PocketQueue';
  export { QUEUE_FIXTURES } from './src/components/results/fixtures';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const mounted = new Set();
const realFetch = g.fetch;
afterEach(() => {
  for (const v of mounted) v.done();
  mounted.clear();
  g.fetch = realFetch;
});
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
const click = async (el) => {
  assert.ok(el, 'element to click');
  el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const key = async (el, k) => {
  el.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: k, bubbles: true }));
  await settle();
};
const F = m.QUEUE_FIXTURES;
const raw = (name) => structuredClone(F[name].results.blocks[0]);
const withQ = (name, over) => {
  const b = raw(name);
  b.queue = { ...b.queue, ...over };
  return b;
};
const ok = (b) => assert.ok(m.validPocket(b), 'should validate');
const bad = (b, why) => assert.equal(m.validPocket(b), null, why);

const calls = [];
/** fetch stub: GET read → the given view; POST action → `answer(name, body)` (a view, or [status, error]). */
function stub(view, answer = () => view) {
  calls.length = 0;
  g.fetch = async (url, init = {}) => {
    const method = init.method ?? 'GET';
    const body = init.body ? JSON.parse(init.body) : null;
    calls.push([method, String(url), body]);
    const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'content-type': 'application/json' } });
    const read = (v) => json({ pocket: { state: 'live' }, summary: {}, removed: [], host_view: v, join: view.join });
    if (method === 'GET') return read(view);
    if (String(url).endsWith('/end')) return json({ pocket: { code: 'bakery', state: 'ended' } });
    const r = answer(body.name, body.body);
    if (Array.isArray(r)) return json({ error: r[1], message: r[1] }, r[0]);
    return read(r);
  };
}
const actions = () => calls.filter((c) => c[0] === 'POST').map((c) => c[2]);
const card = async (name, over) => {
  const b = m.validPocket(over ? withQ(name, over) : raw(name));
  assert.ok(b, `${name} validates`);
  const v = await mount(h(m.QueueCard, { block: b, ask: (t) => v.asked.push(t) }));
  v.asked = [];
  v.$ = (s) => v.host.querySelector(s);
  return v;
};

// ---- the validator

test('every fixture validates as a queue Pocket and reads its state', () => {
  const want = { setup: 'setup', empty: 'open', waiting: 'open', called: 'open', skipped: 'open', paused: 'paused', close: 'open', closed: 'closed', ended: 'ended' };
  for (const [name, state] of Object.entries(want)) {
    const b = m.validPocket(raw(name));
    assert.ok(b, name);
    assert.equal(b.kit, 'queue');
    assert.equal(b.queue.state, state, name);
  }
  assert.equal(m.kitLabel('queue'), 'Queue ticket');
});

test('validQueue: missing fields fall back', () => {
  const q = m.validQueue({ state: 'open' }, "Tine's Bakery", 'live');
  assert.ok(q);
  assert.equal(q.spec.title, "Tine's Bakery");
  assert.equal(q.spec.call_minutes, 3);
  assert.equal(q.spec.ask_name, false);
  assert.equal(q.next_number, 1);
  assert.deepEqual(q.tickets, []);
  assert.equal(q.summary.waiting, 0);
  assert.equal(q.summary.avg_minutes, null);
  assert.equal(q.join, null);
  assert.equal(q.now_serving, null);
  // Set-up: the spec alone, block state "writing".
  const s = m.validQueue({ spec: { title: 'Bakery', line: 'Side counter' } }, undefined, 'writing');
  assert.equal(s.state, 'setup');
  assert.equal(s.spec.line, 'Side counter');
  // An ended block is ended whatever the view says.
  assert.equal(m.validQueue({ state: 'open' }, 'X', 'ended').state, 'ended');
  // Tickets are sorted oldest first; a first name is plain text (kept as typed, drawn as text).
  const t = m.validQueue({ state: 'open', tickets: [{ number: 9, state: 'waiting', name: '<b>Ana</b>' }, { number: 3, state: 'waiting' }] }, 'X', 'live');
  assert.deepEqual(t.tickets.map((x) => x.number), [3, 9]);
  assert.equal(t.tickets[1].name, '<b>Ana</b>');
});

test('validQueue: junk is rejected (the reply shows as markdown)', () => {
  bad(withQ('waiting', { state: 'busy' }), 'unknown state');
  bad(withQ('waiting', { tickets: 'many' }), 'tickets not a list');
  bad(withQ('waiting', { tickets: [{ number: -1, state: 'waiting' }] }), 'negative number');
  bad(withQ('waiting', { tickets: [{ number: 1, state: 'served' }] }), 'served tickets are not in the host view');
  bad(withQ('waiting', { tickets: [{ number: 1, state: 'waiting' }, { number: 1, state: 'waiting' }] }), 'duplicate numbers');
  bad(withQ('waiting', { tickets: [{ number: 1, state: 'called' }, { number: 2, state: 'called' }] }), 'two called at once');
  bad(withQ('waiting', { spec: { ...raw('waiting').queue.spec, call_minutes: 30 } }), 'call time out of range');
  bad(withQ('waiting', { hours_until: '25:00' }), 'not a time of day');
  bad(withQ('waiting', { now: 'yesterday' }), 'now not a time');
  bad(withQ('waiting', { summary: { waiting: -2 } }), 'negative count');
  bad(withQ('waiting', { pause_text: 'x'.repeat(61) }), 'pause text too long');
  bad(withQ('waiting', { join: { mode: 'anywhere', anywhere_url: 'https://a.trycloudflare.com/p/x', anywhere_kind: 'https' } }), 'quick tunnel');
  bad({ ...raw('waiting'), kit: 'desk' }, 'kit and part disagree');
  bad({ ...raw('waiting'), join: { mode: 'wifi', wifi_url: 'http://x/p' } }, 'top-level join');
  ok(raw('waiting'));
});

test('queueFromRead lays a host read over the card; queueMood follows the state', () => {
  const q = m.validPocket(raw('waiting')).queue;
  const next = m.queueFromRead(q, { pocket: { state: 'live' }, host_view: { ...raw('called').queue, join: undefined }, join: raw('called').queue.join });
  assert.equal(next.tickets[0].state, 'called');
  assert.equal(m.queueFromRead(q, { nope: 1 }), null);
  assert.equal(m.queueMood(q), 'working');
  assert.equal(m.queueMood(m.validPocket(raw('empty')).queue), 'idle');
  assert.equal(m.queueMood(m.validPocket(raw('paused')).queue), 'thinking');
  assert.equal(m.queueMood(m.validPocket(raw('closed')).queue), 'finished');
  assert.equal(m.hourLabel('13:00'), '1 PM');
  assert.equal(m.hourLabel('09:30'), '9:30 AM');
});

// ---- the states draw

test('states draw their own chip, block and actions; never holo', async () => {
  const want = {
    setup: ['Set up · not open yet', 'Open the queue'],
    empty: ['Open · nobody waiting', 'Nobody waiting yet. The first ticket is #1.', 'Customers scan this to take a ticket', 'Print counter sign'],
    waiting: ['Open · 5 waiting', 'Call next · #24', 'Next 3 · 5 waiting', '+ 2 more: #27, #28', 'Served', 'Average wait'],
    called: ['Open · #24 called', 'Served · call #25', 'Skip #24', 'to reach the counter', '4 waiting after'],
    skipped: ["#24 didn't come in 3 minutes", 'Bring back #24', 'until 10:44'],
    paused: ['Paused · 5 keep their place', 'Back in 10 minutes', 'Resume · call #24 next', 'Paused since 10:40'],
    closed: ['Closed for today · 1:00 PM', 'Closed by itself at 1 PM, as set. 2 people were still waiting', 'Open again · starts at #1', 'End this queue', '4 · 5'],
    ended: ['Ended · Dec 19', 'Set up a new queue like this', '174', 'The link and the counter sign no longer work.'],
  };
  for (const [name, texts] of Object.entries(want)) {
    stub(raw(name).queue);
    const v = await card(name);
    const text = v.host.textContent;
    for (const t of texts) assert.ok(text.includes(t), `${name}: "${t}" in ${text.slice(0, 400)}`);
    assert.equal(v.host.querySelectorAll('[data-holo], .kw-holo, .sticker-holo').length, 0, `${name}: no holo`);
    v.done();
  }
});

test('nobody waiting: Call next is off, and N does nothing', async () => {
  stub(raw('empty').queue);
  const v = await card('empty');
  const btn = v.$('[data-action="call-next"]');
  assert.equal(btn.getAttribute('aria-disabled'), 'true');
  await click(btn);
  await key(v.$('section'), 'n');
  assert.deepEqual(actions(), []);
});

// ---- calling

test('Call next sends call_next; with a number called it becomes Served · call next', async () => {
  const called = raw('called').queue;
  stub(raw('waiting').queue, (name) => (name === 'call_next' ? called : raw('waiting').queue));
  const v = await card('waiting');
  await click(v.$('[data-action="call-next"]'));
  assert.deepEqual(actions(), [{ name: 'call_next' }]);
  assert.match(v.host.textContent, /Served · call #25/);
  assert.equal(v.$('[data-action="served-call-next"]').getAttribute('aria-keyshortcuts'), 'N');

  stub(called, () => raw('waiting').queue);
  await click(v.$('[data-action="served-call-next"]'));
  assert.deepEqual(actions(), [{ name: 'call_next', body: { served: 24 } }]);
});

test('Served alone sends serve; a refused action says why in words', async () => {
  stub(raw('called').queue, () => [409, 'paused']);
  const v = await card('called');
  await click(v.$('[data-action="serve"]'));
  assert.deepEqual(actions(), [{ name: 'serve', body: { ticket: 24 } }]);
  assert.match(v.$('[data-done="error"]').textContent, /paused\. Resume it first/);
});

test('Skip sends skip, and Bring back sends recall', async () => {
  stub(raw('called').queue, (name) => (name === 'skip' ? raw('skipped').queue : raw('called').queue));
  const v = await card('called');
  await click(v.$('[data-action="skip"]'));
  assert.deepEqual(actions()[0], { name: 'skip', body: { ticket: 24 } });
  // Skipped by hand: the callout says so rather than "didn't come".
  assert.match(v.host.textContent, /You skipped #24/);
  assert.ok(v.$('[data-skipped="24"]'));

  stub(raw('skipped').queue, () => raw('called').queue);
  await click(v.$('[data-action="recall"]'));
  assert.deepEqual(actions(), [{ name: 'recall', body: { ticket: 24 } }]);
  assert.equal(v.$('[data-skipped]'), null);
});

// ---- Remove with Undo

test('Remove hides the row at once, Undo brings it back and nothing is sent', async () => {
  stub(raw('waiting').queue);
  const v = await card('waiting');
  const x = v.$('[data-ticket="25"] [data-action="remove"]');
  assert.equal(x.getAttribute('aria-label'), 'Remove #25 from the queue');
  await click(x);
  assert.equal(v.$('[data-ticket="25"]'), null);
  assert.match(v.$('[data-undo="25"]').textContent, /#25 removed/);
  await click(v.$('[data-action="undo"]'));
  assert.ok(v.$('[data-ticket="25"]'));
  assert.equal(v.$('[data-undo]'), null);
  await wait(50);
  assert.deepEqual(actions(), []);
});

test(`Remove reaches the Mac after ${5} s without Undo; a second remove sends the first at once`, async () => {
  const gone = (n) => ({ ...raw('waiting').queue, tickets: raw('waiting').queue.tickets.filter((t) => t.number !== n) });
  stub(raw('waiting').queue, (name, body) => gone(body.ticket));
  const v = await card('waiting');
  await click(v.$('[data-ticket="25"] [data-action="remove"]'));
  await click(v.$('[data-ticket="26"] [data-action="remove"]'));
  assert.deepEqual(actions(), [{ name: 'remove', body: { ticket: 25 } }]);
  assert.equal(v.$('[data-ticket="26"]'), null);
  await wait(m.QUEUE_UNDO_MS + 150);
  await settle();
  assert.deepEqual(actions(), [
    { name: 'remove', body: { ticket: 25 } },
    { name: 'remove', body: { ticket: 26 } },
  ]);
  assert.equal(v.$('[data-undo]'), null);
});

// ---- N

test('N calls the next number only while the card has focus, never from a text field', async () => {
  stub(raw('waiting').queue, () => raw('called').queue);
  const v = await card('waiting');
  // Outside the card: nothing.
  document.body.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: 'n', bubbles: true }));
  await settle();
  assert.deepEqual(actions(), []);
  // With a modifier: nothing.
  v.$('section').dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: 'n', metaKey: true, bubbles: true }));
  await settle();
  assert.deepEqual(actions(), []);
  // On the card (or a button in it): call_next.
  await key(v.$('[data-ticket="25"] [data-action="remove"]'), 'N');
  assert.deepEqual(actions(), [{ name: 'call_next' }]);
});

test('holding N calls once: key repeats do nothing', async () => {
  stub(raw('waiting').queue, () => raw('waiting').queue);
  const v = await card('waiting');
  const el = v.$('section');
  el.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: 'n', bubbles: true }));
  await settle();
  for (let i = 0; i < 5; i++) el.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: 'n', repeat: true, bubbles: true }));
  await settle();
  assert.deepEqual(actions(), [{ name: 'call_next' }]);
});

test('N from the pause text field types, not calls', async () => {
  stub(raw('waiting').queue);
  const v = await card('waiting');
  await click(v.$('[data-action="pause"]'));
  await click(v.$('[data-preset="own"]'));
  const input = v.$('input[aria-label="Your pause message"]');
  assert.ok(input);
  await key(input, 'n');
  assert.deepEqual(actions(), []);
});

// ---- pause, close, closed

test('Pause presets send pause with the text; while paused another preset sends resume then pause', async () => {
  stub(raw('waiting').queue, (name, body) => (name === 'pause' ? { ...raw('paused').queue, pause_text: body.text } : raw('waiting').queue));
  const v = await card('waiting');
  await click(v.$('[data-action="pause"]'));
  await click(v.$('[data-preset="Back after lunch"]'));
  assert.deepEqual(actions(), [{ name: 'pause', body: { text: 'Back after lunch' } }]);
  assert.match(v.$('[data-pause-text]').textContent, /Back after lunch/);

  stub(raw('paused').queue, (name, body) => (name === 'pause' ? { ...raw('paused').queue, pause_text: body.text } : raw('waiting').queue));
  await click(v.$('[data-preset="Back in 10 minutes"]'));
  assert.deepEqual(actions(), [{ name: 'resume' }, { name: 'pause', body: { text: 'Back in 10 minutes' } }]);
});

test('Resume · call next sends resume, then call_next', async () => {
  stub(raw('paused').queue, (name) => (name === 'resume' ? raw('waiting').queue : raw('called').queue));
  const v = await card('paused');
  await click(v.$('[data-action="resume"]'));
  assert.deepEqual(actions(), [{ name: 'resume' }, { name: 'call_next' }]);
});

test('Close for today is an inline confirm with the message, no modal', async () => {
  stub(raw('waiting').queue, () => raw('closed').queue);
  const v = await card('waiting');
  await click(v.$('[data-action="close"]'));
  const box = v.$('[data-confirm="close"]');
  assert.ok(box);
  assert.equal(document.querySelector('[role="dialog"]'), null);
  assert.match(box.textContent, /5 people are still waiting\. Their page shows your message and their tickets are deleted\./);
  assert.equal(box.querySelector('textarea').value, m.QUEUE_CLOSE_DEFAULT);
  await click(v.$('[data-action="confirm-close"]'));
  assert.deepEqual(actions(), [{ name: 'close', body: { text: m.QUEUE_CLOSE_DEFAULT } }]);
  assert.equal(v.$('section').dataset.queue, 'closed');
});

test('pause and close text go out on one line: the server refuses an Enter', async () => {
  assert.equal(m.oneLine('a\nb'), 'a b');
  assert.equal(m.oneLine('  Back\r\n at\t2 PM  '), 'Back at 2 PM');
  stub(raw('waiting').queue, () => raw('closed').queue);
  const v = await card('waiting');
  await click(v.$('[data-action="close"]'));
  const ta = v.$('[data-confirm="close"] textarea');
  Object.getOwnPropertyDescriptor(dom.window.HTMLTextAreaElement.prototype, 'value').set.call(ta, 'a\nb');
  ta.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
  await settle();
  await click(v.$('[data-action="confirm-close"]'));
  assert.deepEqual(actions(), [{ name: 'close', body: { text: 'a b' } }]);
});

/** Track the card's poll timers (setInterval at QUEUE_POLL_MS) so a test can fire them by hand. */
function pollTimers() {
  const live = new Map();
  const set = window.setInterval;
  const clear = window.clearInterval;
  let next = 1e6;
  window.setInterval = (fn, ms, ...rest) => {
    if (ms !== m.QUEUE_POLL_MS) return set.call(window, fn, ms, ...rest);
    const id = next++;
    live.set(id, fn);
    return id;
  };
  window.clearInterval = (id) => (live.has(id) ? live.delete(id) : clear.call(window, id));
  return { live, fire: () => [...live.values()].forEach((fn) => fn()), restore: () => ((window.setInterval = set), (window.clearInterval = clear)) };
}

test('a 404 stops the polling; other errors back off', async () => {
  const timers = pollTimers();
  try {
    calls.length = 0;
    let status = 500;
    g.fetch = async (url, init = {}) => {
      calls.push([init.method ?? 'GET', String(url), null]);
      return new Response(JSON.stringify({ error: 'x', message: 'x' }), { status, headers: { 'content-type': 'application/json' } });
    };
    await card('waiting');
    assert.equal(timers.live.size, 1, 'polls while open');
    const gets = () => calls.filter((c) => c[0] === 'GET').length;
    const before = gets(); // the read on show failed once: the next poll waits a beat
    timers.fire();
    await settle();
    assert.equal(gets(), before, 'skips a beat after a failed read');
    timers.fire();
    await settle();
    assert.equal(gets(), before + 1);
    status = 404;
    timers.fire(), timers.fire(), timers.fire(), timers.fire();
    await settle();
    assert.equal(timers.live.size, 0, 'a gone pocket is not polled again');
  } finally {
    timers.restore();
  }
});

test('a read that started before an action does not paint over it', async () => {
  let release;
  const slow = new Promise((r) => (release = r));
  stub(raw('waiting').queue, () => raw('called').queue);
  const inner = g.fetch;
  let first = true;
  g.fetch = async (url, init = {}) => {
    if ((init.method ?? 'GET') === 'GET' && first) {
      first = false;
      await slow;
    }
    return inner(url, init);
  };
  const v = await card('waiting');
  await click(v.$('[data-action="call-next"]'));
  assert.ok(v.$('[data-action="served-call-next"]'), 'called after the action');
  release();
  await settle();
  assert.ok(v.$('[data-action="served-call-next"]'), 'the late read of the waiting queue is ignored');
});

test('Closed: Open again sends reopen; End asks first, then ends the pocket', async () => {
  stub(raw('closed').queue, () => raw('empty').queue);
  const v = await card('closed');
  await click(v.$('[data-action="reopen"]'));
  assert.deepEqual(actions(), [{ name: 'reopen' }]);
  v.done();

  stub(raw('closed').queue);
  const w = await card('closed');
  await click(w.$('[data-action="end"]'));
  assert.match(w.$('[data-confirm="end"]').textContent, /stop working for good/);
  await click(w.$('[data-action="confirm-end"]'));
  assert.ok(calls.some(([meth, url]) => meth === 'POST' && url.endsWith('/api/pockets/bakery/end')));
  assert.equal(w.$('section').dataset.queue, 'ended');
});

test('Change hours sends set_hours with the time, or null for "until I close it"', async () => {
  stub(raw('closed').queue);
  const v = await card('closed');
  assert.match(v.$('[data-action="reopen"]').className, /kw-btn-primary/);
  await click(v.$('[data-action="change-hours"]'));
  assert.equal(v.$('[data-action="change-hours"]'), null, 'Change hours hides while the editor is open');
  assert.doesNotMatch(v.$('[data-action="reopen"]').className, /kw-btn-primary/, 'only Save hours stays violet');
  const radios = v.host.querySelectorAll('[data-hours-edit] input[type="radio"]');
  await click(radios[1]);
  await click(v.$('[data-action="save-hours"]'));
  assert.deepEqual(actions(), [{ name: 'set_hours', body: { until: null } }]);
});

test('Set-up: Open the queue asks the bot with the choices; Ask for a first name is off by default', async () => {
  stub(raw('setup').queue);
  const v = await card('setup');
  const sw = v.$('[role="switch"]');
  assert.equal(sw.getAttribute('aria-checked'), 'false');
  assert.equal(v.$('[data-setup] input').getAttribute('maxlength'), '40', 'the server takes a name of 1-40');
  await click(v.$('[data-step="up"]'));
  await click(sw);
  await click(v.$('[data-action="open-queue"]'));
  assert.equal(v.asked.length, 1);
  assert.match(v.asked[0], /^Open Tine's Bakery with the line "Pick up at the side counter", open until 1 PM today, 4 minutes to reach the counter, asking for a first name\.$/);
  assert.deepEqual(actions(), []);
});
