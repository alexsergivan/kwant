// pa5 guest page (autonomy/pocket-apps/pa5/brief.md): every phone screen from the fake server, reconnect and backoff,
// first tap counts, the static half of the CSP rules, token drift, and the 80 KB gzipped budget.
// The live half of the CSP test (the built page in Chrome under exactly pa1's policy) is guest/e2e/csp.mjs.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { readFileSync, readdirSync, existsSync, statSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import { loadBundled } from './bundle.mjs';
import { scenarios, order, CSP, FakeBox, envelope } from '../guest/fake/views.mjs';

const dom = new JSDOM('<!doctype html><html><body><div id="app"></div></body></html>', { url: 'http://localhost/p/join', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'MouseEvent', 'Image', 'SVGElement']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { GuestApp, privacyLine, day } from './guest/src/app';
  export { BACKOFF, backoff, codeFrom, fromState, retentionFrom } from './guest/src/api';
`);
const settle = () => new Promise((r) => setTimeout(r, 0));
const flush = async () => {
  for (let i = 0; i < 6; i++) await settle();
};
const clone = (v) => (v && typeof v === 'object' ? JSON.parse(JSON.stringify(v).replaceAll('/CODE/', '/t/')) : v);

/** A fake box (pa1's guest API, from views.mjs) and a hand-driven clock. */
function harness(name, { ws = false, overrides = {} } = {}) {
  const calls = [];
  let now = 1_000_000;
  let timers = [];
  let nextId = 1;
  const box = new FakeBox(name, { code: 't', overrides });
  box.up = true;
  box.actionReply = null;
  const fetch = async (url, init = {}) => {
    const path = url.split('/api/pocket/t/')[1] ?? '';
    const what = path.split('?')[0];
    calls.push({ at: now, what, path, body: init.body ? JSON.parse(init.body) : undefined });
    if (!box.up) throw new TypeError('Failed to fetch');
    if (what === 'action' && box.actionReply) return box.actionReply();
    const r = box.handle(path, init.body ? JSON.parse(init.body) : {});
    return { ok: r.status < 400, status: r.status, json: async () => (r.body === null ? Promise.reject(new SyntaxError('empty')) : r.body) };
  };
  const sockets = [];
  class FakeWS {
    constructor(url) {
      this.url = url;
      sockets.push(this);
    }
    close() {
      this.closed = true;
    }
  }
  const env = {
    fetch,
    WebSocket: ws ? FakeWS : undefined,
    setTimeout: (f, ms) => {
      timers.push({ id: nextId, at: now + ms, f });
      return nextId++;
    },
    clearTimeout: (id) => (timers = timers.filter((t) => t.id !== id)),
    setInterval: (f, ms) => {
      timers.push({ id: nextId, at: now + ms, f, every: ms });
      return nextId++;
    },
    clearInterval: (id) => (timers = timers.filter((t) => t.id !== id)),
    now: () => now,
    location: { pathname: '/p/t', protocol: 'http:', host: 'localhost' },
  };
  const root = document.getElementById('app');
  root.replaceChildren();
  document.querySelectorAll('.toast').forEach((t) => t.remove());
  const app = new m.GuestApp(root, env, 't');
  const advance = async (ms) => {
    const end = now + ms;
    for (;;) {
      timers.sort((a, b) => a.at - b.at);
      const t = timers[0];
      if (!t || t.at > end) break;
      now = t.at;
      if (t.every) t.at += t.every;
      else timers.shift();
      t.f();
      await flush();
    }
    now = end;
    await flush();
  };
  return { app, root, calls, box, sockets, advance, env, screen: () => root.querySelector('[data-screen]')?.getAttribute('data-screen'), text: () => root.textContent.replace(/\s+/g, ' ') };
}

/** A ws push as pa1 sends it: {type:"state", ...v1 state}. */
const frame = (flat, version = 50) => ({ data: JSON.stringify({ type: 'state', ...envelope(clone(flat), { code: 't', version }) }) });
/** An HTTP reply with a v1 state body. */
const stateReply = (flat, status = 200) => ({ ok: true, status, json: async () => envelope(clone(flat), { code: 't', version: 60 }) });

async function open(name, opts) {
  const t = harness(name, opts);
  await t.app.start(null);
  await flush();
  return t;
}

const click = (el) => el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
const type = (el, v) => {
  el.value = v;
  el.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
};
const submit = (form) => form.dispatchEvent(new dom.window.Event('submit', { bubbles: true, cancelable: true }));

// ---------- every screen ----------

const expect = {
  join: ['join', ['Friday Pub Quiz', 'Hosted by Dana · 4 rounds · teams of up to 4', 'Your name', 'Pick your creature', 'Quizzly Bears', 'Full', 'Start a new team', 'Join the quiz', 'Your name and answers are deleted 7 days after the quiz.', 'made with kwant']],
  'join-solo': ['lobby', ["You're in, Ana!"]],
  lobby: ['lobby', ["You're in, Mia!", 'Quizzly Bears · 3 of 4', 'Grab a seat. When the TV shows question 1, your answers appear right here.', '14 players · 5 teams', 'Jonas', 'Keep this page open.', 'Starts 8:00 PM']],
  rejoin: ['question', ['1,240 pts', 'Round 2 · 90s music', 'Question 8 of 10', 'Tap one. Your first tap counts.']],
  full: ['full', ['This quiz is full', '60 players have joined, the most this quiz takes. Ask the host if a team has room for one more.']],
  ended: ['ended', ['This quiz has ended', 'Quizzly Bears won with 3,420 points. Thanks for playing!', 'Your name and answers are deleted on Fri, Dec 4.']],
  choice: ['question', ['1994', '1996', '1998', '2000', 'Tap one. Your first tap counts.']],
  text: ['question', ['What is the capital city of Australia?', "Spelling doesn't have to be perfect. Quizmaster settles close ones.", 'Send answer']],
  number: ['question', ['How many bones', 'Closest answer wins. Exact gets a 50 point bonus.', 'Send 212']],
  picture: ['question', ['Which fruit is this, cut in half?', 'Picture: drawn by Quizmaster', 'Kiwi']],
  locked: ['question', ['Locked in', '1996', 'Waiting for 5 more players. Eyes on the TV!']],
  right: ['reveal', ['Right!', '+120', '100 + 20 for speed', '62% got it', 'Source: Wikipedia', 'Quizzly Bears', '1,960', '1,360 pts']],
  near: ['reveal', ['Close one!', 'You wrote "Canbera". One letter off, so it counts.', '+100', 'Canberra']],
  overturned: ['reveal', ['Not counted', `The host didn't count "Canbra".`, '+0']],
  wrong: ['reveal', ['Not this time', 'You said 212. The answer is 206.', '+40', 'for being close']],
  between: ['between', ['After round 2 of 4', 'Round 3 in 0:40', 'Quizzly Bears jump to first', 'Fact Hunters', '▲ 2', '▼ 1', 'Your team: up 2 places this round']],
  paused: ['paused', ['Back in a moment', 'The host paused the quiz. Your answers and score are safe: 1,240.']],
  missing: ['missing', ['Nothing here', "This link doesn't open a pocket app any more."]],
  removed: ['removed', ["You've been removed from this quiz", "If that's a mistake, have a word with the host.", 'Your name and answers are deleted with the quiz.']],
};

for (const name of order) {
  if (name === 'away') continue;
  test(`screen: ${name}`, async () => {
    const t = await open(name);
    if (name === 'join-solo') {
      // Solo quiz: no team section; joining goes straight to the lobby.
      assert.equal(t.screen(), 'join');
      assert.equal(t.root.querySelector('[data-teams]'), null);
      for (const s of ['Hosted by Dana · 4 rounds', 'Join the quiz', 'Your name and answers are deleted when the quiz ends.']) assert.ok(t.text().includes(s), s);
      type(t.root.querySelector('#name'), 'Ana');
      submit(t.root.querySelector('form'));
      await flush();
      assert.equal(t.calls.at(-1).body.team, undefined);
    }
    const [scr, strings] = expect[name];
    assert.equal(t.screen(), scr);
    for (const s of strings) assert.ok(t.text().includes(s) || document.body.textContent.includes(s), `${name}: "${s}" in ${t.text()}`);
    // Never a style attribute in markup; the timer bar's element.style is a CSSOM write the CSP allows.
    for (const el of t.root.querySelectorAll('[style]')) assert.ok(el.parentElement?.matches('[data-timer]'), `style on ${el.outerHTML.slice(0, 80)}`);
    // The Quizmaster is the subject of the bar on every screen.
    assert.ok(t.root.querySelector('.bar .qm[role="img"][aria-label^="Quizmaster"]'));
    t.app.stop();
  });
}

test('join: name, creature and team go to join; errors are plain words', async () => {
  const t = await open('join');
  const form = t.root.querySelector('form');
  submit(form);
  await flush();
  assert.equal(t.root.querySelector('[data-err]').textContent, 'Type your name first.');
  assert.equal(t.root.querySelector('[data-err]').getAttribute('role'), 'alert');
  type(t.root.querySelector('#name'), 'Mia');
  submit(form);
  await flush();
  assert.equal(t.root.querySelector('[data-err]').textContent, 'Pick a team, or start a new one.');
  assert.ok(!t.calls.some((c) => c.what === 'join'));
  // The full team can't be picked.
  const full = [...t.root.querySelectorAll('.team')].find((r) => r.textContent.includes('Table Four'));
  assert.ok(full.classList.contains('full'));
  assert.equal(full.querySelector('input').disabled, true);
  // Pick the cyan visor and Quizzly Bears.
  const visor = t.root.querySelector('input[value="visor"]');
  visor.checked = true;
  visor.dispatchEvent(new dom.window.Event('change', { bubbles: true }));
  const bears = t.root.querySelector('input[value="t1"]');
  bears.checked = true;
  bears.dispatchEvent(new dom.window.Event('change', { bubbles: true }));
  assert.ok(t.root.querySelector('input[value="t1"]').closest('.team').classList.contains('on'));
  t.box.joinError = 'team_full';
  submit(t.root.querySelector('form'));
  await flush();
  assert.equal(t.root.querySelector('[data-err]').textContent, 'That team just filled up. Pick another one.');
  t.box.joinError = 'name_taken';
  submit(t.root.querySelector('form'));
  await flush();
  assert.match(t.root.querySelector('[data-err]').textContent, /already has that name/);
  t.box.joinError = null;
  submit(t.root.querySelector('form'));
  await flush();
  const join = t.calls.filter((c) => c.what === 'join').at(-1);
  assert.deepEqual(join.body, { name: 'Mia', avatar: 'visor', team: 'Quizzly Bears' }, 'v1 takes the team by name');
  assert.equal(join.path, 'join');
  assert.equal(t.screen(), 'lobby');
  assert.ok(t.text().includes("You're in, Mia!"));
  t.app.stop();
});

test('join: a new team goes by its name; a full quiz shows the full screen', async () => {
  const t = await open('join');
  type(t.root.querySelector('#name'), 'Leo');
  click([...t.root.querySelectorAll('button.team')].find((b) => b.textContent.includes('Start a new team')));
  submit(t.root.querySelector('form'));
  await flush();
  assert.equal(t.root.querySelector('[data-err]').textContent, 'Give your new team a name.');
  type(t.root.querySelector('.team.new input'), 'Brain Freeze');
  t.box.joinError = 'full';
  submit(t.root.querySelector('form'));
  await flush();
  assert.deepEqual(t.calls.filter((c) => c.what === 'join').at(-1).body, { name: 'Leo', avatar: 'antenna', team: 'Brain Freeze' });
  assert.equal(t.screen(), 'full');
  t.app.stop();
});

test('join: no seat means no socket; info every 10 s, and a name typed survives it (patch, not redraw)', async () => {
  const t = await open('join', { ws: true });
  assert.deepEqual(t.calls.map((c) => c.what), ['state', 'info'], 'state answers 401 signed_out, then the public info');
  assert.equal(t.sockets.length, 0, "pa1's ws needs a seat");
  const name = t.root.querySelector('#name');
  type(name, 'Pri');
  name.focus();
  const v = clone(scenarios.join.info);
  v.teams[0].count = 3;
  t.box.s.info = v;
  await t.advance(10000);
  assert.equal(t.calls.at(-1).what, 'info');
  assert.equal(t.root.querySelector('#name'), name, 'same input element');
  assert.equal(name.value, 'Pri');
  assert.equal(document.activeElement, name);
  assert.ok(t.text().includes('3 of 4'));
  t.app.stop();
});

// ---------- Pocket API v1 (pa1/impl.md) ----------

test('v1: the state envelope maps to the page view; pa1 owns pocket and me facts, nothing is computed', () => {
  const env = envelope(clone(scenarios.right.state), { code: 't', version: 7 });
  assert.equal(env.api, 'pocket/1');
  assert.equal(env.me.team, 'Quizzly Bears', 'v1 me.team is the team name');
  const { view, version } = m.fromState(env);
  assert.equal(version, 7);
  assert.equal(view.phase, 'reveal');
  assert.equal(view.title, 'Friday Pub Quiz');
  assert.equal(view.max_players, 60);
  assert.deepEqual(view.retention, { kind: 'after_end', days: 7 });
  assert.equal(view.me.name, 'Mia');
  assert.equal(view.me.team_name, 'Quizzly Bears');
  assert.equal(view.me.score, 1360, 'score comes from the kit view');
  assert.equal(view.reveal.points, 120);
  // An ended pocket is the ended screen whatever the kit says; expires_at gives the deletion day.
  const ended = m.fromState({ ...env, pocket: { ...env.pocket, state: 'ended', expires_at: '2026-12-04T22:10:00Z' } }).view;
  assert.equal(ended.phase, 'ended');
  assert.equal(ended.deletes_on, '2026-12-04');
  // describe(): after_date keeps its days, unknown kinds are dropped, not guessed.
  assert.deepEqual(m.retentionFrom({ kind: 'after_date', fields: ['date'], days: 3 }), { kind: 'after_date', days: 3 });
  assert.equal(m.retentionFrom({ kind: 'forever' }), undefined);
  assert.equal(m.retentionFrom(null), undefined);
  // Not an envelope: ignored.
  assert.equal(m.fromState({ phase: 'lobby' }), undefined);
  assert.equal(m.fromState('x'), undefined);
});

test('v1: info with no room left (players = max_players) shows the full screen', async () => {
  const t = await open('full');
  assert.equal(t.screen(), 'full');
  assert.ok(t.text().includes('60 players have joined'));
  t.app.stop();
});

test('v1: boot JSON draws at once; a seated one skips the first round trip, a public one still asks state', async () => {
  const t = harness('lobby', { ws: true });
  await t.app.start(envelope(clone(scenarios.lobby.state), { code: 't', version: 3 }));
  await flush();
  assert.equal(t.screen(), 'lobby');
  assert.equal(t.calls.length, 0, 'no state call before the socket');
  assert.equal(t.sockets.length, 1);
  t.app.stop();
  const j = harness('join', { ws: true });
  await j.app.start(envelope(clone(scenarios.join.info), { code: 't', version: 3 }));
  await flush();
  assert.equal(j.screen(), 'join');
  assert.deepEqual(j.calls.map((c) => c.what), ['state'], 'the seat cookie is not sent with the page, so ask once');
  assert.equal(j.sockets.length, 0);
  // A phone that does have a seat gets it back even though the page came with the public info.
  const k = harness('rejoin', { ws: true });
  await k.app.start(envelope(clone(scenarios.join.info), { code: 't', version: 3 }));
  await flush();
  assert.equal(k.screen(), 'question');
  assert.match(document.querySelector('.toast').textContent, /^You're back, Mia\./);
  k.app.stop();
  j.app.stop();
});

test("v1: name_taken offers That's me, which rejoins by name and says so", async () => {
  const t = await open('join');
  const mine = () => t.root.querySelector('[data-mine]');
  assert.equal(mine().hidden, true);
  type(t.root.querySelector('#name'), 'Mia');
  const bears = t.root.querySelector('input[value="t1"]');
  bears.checked = true;
  bears.dispatchEvent(new dom.window.Event('change', { bubbles: true }));
  t.box.joinError = 'name_taken';
  submit(t.root.querySelector('form'));
  await flush();
  assert.match(t.root.querySelector('[data-err]').textContent, /already has that name/);
  assert.equal(mine().hidden, false);
  click(mine());
  await flush();
  assert.deepEqual(t.calls.at(-1), { at: t.calls.at(-1).at, what: 'rejoin', path: 'rejoin', body: { name: 'Mia' } });
  assert.equal(t.screen(), 'question');
  assert.match(document.querySelector('.toast').textContent, /^You're back, Mia\./);
  t.app.stop();
});

test('v1: join errors from pa1 are plain words', async () => {
  const t = await open('join-solo');
  type(t.root.querySelector('#name'), 'Ana');
  const cases = {
    not_open_yet: "This quiz isn't open yet. Try again in a minute.",
    name_too_long: 'Names are 1-20 letters.',
    name_not_allowed: "That name can't be used here. Try another one.",
    slow_down: 'Too many tries just now. Wait a few seconds and tap Join again.',
    pin_required: 'This name needs its PIN. Ask the host to help you back in.',
  };
  for (const [code, line] of Object.entries(cases)) {
    t.box.joinError = code;
    submit(t.root.querySelector('form'));
    await flush();
    assert.equal(t.root.querySelector('[data-err]').textContent, line, code);
  }
  t.app.stop();
});

test('v1: ws signed_out goes back to the join screen; gone ends the page; 410 is the ended screen', async () => {
  const a = await open('lobby', { ws: true });
  a.sockets[0].onopen();
  a.box.s = { ...a.box.s, state: 401, info: scenarios.join.info };
  a.sockets[0].onmessage({ data: JSON.stringify({ type: 'signed_out' }) });
  await flush();
  assert.equal(a.screen(), 'join');
  assert.equal(a.calls.at(-1).what, 'info');
  a.app.stop();

  const b = await open('lobby', { ws: true });
  b.sockets[0].onopen();
  b.sockets[0].onmessage({ data: JSON.stringify({ type: 'gone' }) });
  await flush();
  assert.equal(b.screen(), 'missing');
  const n = b.calls.length;
  await b.advance(60000);
  assert.equal(b.calls.length, n, 'no traffic after gone');
  b.app.stop();

  const c = await open('lobby');
  c.box.seat = null;
  c.box.s = { ...c.box.s, state: 410 };
  await c.advance(2000);
  assert.equal(c.screen(), 'ended');
  c.app.stop();
});

test('v1: a ws state frame updates the page; an older-style frame is ignored', async () => {
  const t = await open('lobby', { ws: true });
  t.sockets[0].onopen();
  t.sockets[0].onmessage({ data: JSON.stringify({ view: { ...scenarios.choice.state } }) });
  await flush();
  assert.equal(t.screen(), 'lobby');
  t.sockets[0].onmessage(frame(scenarios.choice.state));
  await flush();
  assert.equal(t.screen(), 'question');
  t.app.stop();
});

// ---------- 2c rejoin toast ----------

test('2c: a restored seat mid-game says so for 4 s, then goes', async () => {
  const t = await open('rejoin');
  const toast = document.querySelector('.toast');
  assert.ok(toast);
  assert.equal(toast.getAttribute('role'), 'status');
  assert.equal(toast.textContent, "You're back, Mia. Same seat, same team. Your score is safe: 1,240.");
  await t.advance(4300);
  assert.equal(document.querySelector('.toast'), null);
  t.app.stop();
});

test('2b: a restored seat in the lobby gets no toast (nothing was missed)', async () => {
  const t = await open('lobby');
  assert.equal(document.querySelector('.toast'), null);
  t.app.stop();
});

// ---------- 2d away, backoff, back ----------

test('2d: box away retries on 2, 5, 10, 20, then every 20 s; Try now; back with a toast', async () => {
  assert.deepEqual(m.BACKOFF, [2, 5, 10, 20]);
  const t = await open('choice', { ws: true });
  assert.equal(t.screen(), 'question');
  t.box.up = false;
  t.sockets[0].onopen();
  t.sockets[0].onclose(); // the box went away
  await flush();
  assert.equal(t.screen(), 'away');
  assert.ok(t.text().includes("The host's box is away"));
  assert.ok(t.text().includes('Your score is safe: 1,240.'));
  assert.ok(t.text().includes('Trying again in 2 s · tried 1 time'));
  const states = () => t.calls.filter((c) => c.what === 'state').map((c) => c.at);
  const start = states().at(-1);
  await t.advance(2000 + 5000 + 10000 + 20000 + 20000 + 20000);
  const gaps = states().slice(-6).map((a, i, all) => (i ? (a - all[i - 1]) / 1000 : (a - start) / 1000));
  assert.deepEqual(gaps, [2, 5, 10, 20, 20, 20]);
  assert.ok(t.text().includes('tried 7 times'));
  // The countdown line ticks down each second.
  await t.advance(3000);
  assert.ok(t.text().includes('Trying again in 17 s'), t.text());
  // Try now asks at once.
  const before = states().length;
  click([...t.root.querySelectorAll('button')].find((b) => b.textContent === 'Try now'));
  await flush();
  assert.equal(states().length, before + 1);
  // Back.
  t.box.up = true;
  await t.advance(20000);
  assert.equal(t.screen(), 'question');
  assert.match(document.querySelector('.toast').textContent, /^You're back, Mia\./);
  t.app.stop();
});

test('2d: no WebSocket: polls state every 2 s, and away after the first failed poll', async () => {
  const t = await open('lobby');
  await t.advance(4000);
  const polls = t.calls.filter((c) => c.what === 'state');
  assert.equal(polls.length, 3);
  assert.deepEqual(polls.map((c) => c.path), ['state', 'state?since=1', 'state?since=1'], 'polls ask only for what changed (204)');
  assert.equal(t.screen(), 'lobby');
  t.box.up = false;
  await t.advance(2000);
  assert.equal(t.screen(), 'away');
  t.app.stop();
});

test('3j: removed (403) stops all traffic and never offers rejoin', async () => {
  const t = await open('choice', { ws: true });
  t.box.seat = null;
  t.box.s = { ...t.box.s, state: 403 };
  t.sockets[0].onopen();
  t.sockets[0].onclose();
  await flush();
  assert.equal(t.screen(), 'removed');
  const n = t.calls.length;
  await t.advance(60000);
  assert.equal(t.calls.length, n);
  assert.ok(!t.text().includes('Join'));
  t.app.stop();
});

// ---------- play ----------

test('3a: first tap counts; later taps are ignored; then locked in', async () => {
  const t = await open('choice');
  let release;
  t.box.actionReply = () => new Promise((r) => (release = () => r({ ok: true, status: 200, json: async () => envelope(clone(scenarios.choice.action), { code: 't', version: 9 }) })));
  const btn = (k) => t.root.querySelector(`button[data-answer="${k}"]`);
  assert.equal(btn('B').getAttribute('aria-label'), 'B: 1996');
  click(btn('B'));
  click(btn('C'));
  click(btn('B'));
  await flush();
  const sent = t.calls.filter((c) => c.what === 'action');
  assert.equal(sent.length, 1);
  assert.deepEqual(sent[0].body, { name: 'answer', body: { q: 'q8', choice: 'B' } });
  assert.ok(btn('B').classList.contains('pick'));
  assert.equal(btn('B').getAttribute('aria-pressed'), 'true');
  assert.equal(btn('C').disabled, true);
  assert.ok(t.text().includes('Sending…'), 'pending until the box takes it');
  assert.ok(!t.text().includes('Sent.'));
  release();
  await flush();
  assert.ok(t.text().includes('Locked in'));
  assert.ok(t.text().includes('Waiting for 5 more players. Eyes on the TV!'));
  t.app.stop();
});

test('3a: an answer sent while the box blinks is retried, and a 409 keeps the first', async () => {
  const t = await open('choice');
  let n = 0;
  t.box.actionReply = async () => (++n < 3 ? { ok: false, status: 503, json: async () => ({}) } : { ok: false, status: 409, json: async () => ({ error: 'already_answered' }) });
  click(t.root.querySelector('button[data-answer="A"]'));
  await flush();
  await t.advance(4500);
  assert.equal(n, 3);
  assert.deepEqual(t.calls.filter((c) => c.what === 'action').map((c) => c.body.body), [{ q: 'q8', choice: 'A' }, { q: 'q8', choice: 'A' }, { q: 'q8', choice: 'A' }]);
  t.app.stop();
});

test('D1: a lost answer is never shown as sent; it says so after 6 tries and is sent once the network is back', async () => {
  const t = await open('choice');
  let down = true;
  t.box.actionReply = () => (down ? Promise.reject(new TypeError('Failed to fetch')) : null);
  const reach = () => t.calls.filter((c) => c.what === 'action');
  click(t.root.querySelector('button[data-answer="B"]'));
  await flush();
  assert.ok(t.text().includes('Sending…'));
  await t.advance(10_500);
  assert.equal(reach().length, 6);
  assert.ok(t.text().includes("Couldn't send. Trying again."), t.text());
  assert.ok(!t.text().includes('Sent.'));
  assert.equal(t.root.querySelector('[data-note]').getAttribute('role'), 'status');
  // The network comes back: the same answer goes again without a tap, and the box takes it.
  down = false;
  t.box.actionReply = null;
  await t.advance(5000);
  assert.deepEqual(reach().at(-1).body, { name: 'answer', body: { q: 'q8', choice: 'B' } });
  assert.ok(t.text().includes('Locked in'), t.text());
  const n = reach().length;
  await t.advance(20_000);
  assert.equal(reach().length, n, 'nothing more once the box has it');
  t.app.stop();
});

test('D1: an answer lost while the box was away is sent again when it is back', async () => {
  const t = await open('choice', { ws: true });
  t.sockets[0].onopen();
  t.box.up = false;
  click(t.root.querySelector('button[data-answer="C"]'));
  await flush();
  t.sockets[0].onclose();
  await flush();
  assert.equal(t.screen(), 'away');
  await t.advance(15_000);
  t.box.up = true;
  await t.advance(20_000);
  assert.equal(t.screen(), 'question');
  assert.ok(t.text().includes('Locked in'), t.text());
  const ok = t.calls.filter((c) => c.what === 'action');
  assert.deepEqual(ok.at(-1).body.body, { q: 'q8', choice: 'C' });
  t.app.stop();
});

test('D1: a refused answer (time is up) frees the pick and is not retried', async () => {
  const t = await open('choice');
  t.box.actionReply = async () => ({ ok: false, status: 422, json: async () => ({ error: 'closed' }) });
  click(t.root.querySelector('button[data-answer="A"]'));
  await flush();
  await t.advance(15_000);
  assert.equal(t.calls.filter((c) => c.what === 'action').length, 1);
  assert.ok(!t.text().includes('Sending…'));
  assert.ok(!t.text().includes('Sent.'));
  t.app.stop();
});

test('R1: a ws removed frame shows the removed screen at once and stops all traffic', async () => {
  const t = await open('lobby', { ws: true });
  t.sockets[0].onopen();
  t.sockets[0].onmessage({ data: JSON.stringify({ type: 'removed' }) });
  await flush();
  assert.equal(t.screen(), 'removed');
  assert.ok(t.sockets[0].closed);
  const n = t.calls.length;
  await t.advance(60_000);
  assert.equal(t.calls.length, n);
  t.app.stop();
});

test('R2: waking the phone asks state now and swaps a quiet socket for a fresh one; no seat, no traffic', async () => {
  const t = await open('lobby', { ws: true });
  t.sockets[0].onopen();
  const n = t.calls.length;
  t.app.wake();
  await flush();
  assert.equal(t.calls.length, n + 1);
  assert.equal(t.calls.at(-1).what, 'state');
  assert.ok(t.sockets[0].closed);
  assert.equal(t.sockets.length, 2);
  t.app.stop();

  const j = await open('join');
  const k = j.calls.length;
  j.app.wake();
  await flush();
  assert.equal(j.calls.length, k);
  j.app.stop();
});

test('R3: with no seat the kit phase never decides the screen', () => {
  const v = m.fromState({ version: 3, pocket: { kit: 'quiz', state: 'live', title: 'Q' }, view: { phase: 'question' } }).view;
  assert.equal(v.phase, 'join');
  const seated = m.fromState({ version: 3, pocket: { kit: 'quiz', state: 'live' }, me: { name: 'Mia', avatar: 'a' }, view: { phase: 'question' } }).view;
  assert.equal(seated.phase, 'question');
});

test('3b: text answer sends once and keeps typing across pushes', async () => {
  const t = await open('text', { ws: true });
  const input = t.root.querySelector('input[data-answer]');
  type(input, 'Canbera');
  t.sockets[0].onopen();
  t.sockets[0].onmessage(frame({ ...scenarios.text.state, question: { ...scenarios.text.state.question, secs_left: 20 } }));
  await flush();
  assert.equal(t.root.querySelector('input[data-answer]'), input);
  assert.equal(input.value, 'Canbera');
  submit(t.root.querySelector('form'));
  submit(t.root.querySelector('form'));
  await flush();
  const sent = t.calls.filter((c) => c.what === 'action');
  assert.equal(sent.length, 1);
  assert.deepEqual(sent[0].body, { name: 'answer', body: { q: 'q3', text: 'Canbera' } });
  assert.ok(t.text().includes('Locked in'));
  assert.ok(t.text().includes('Canbera'));
  t.app.stop();
});

test('3c: number answer steps within bounds and sends the number', async () => {
  const t = await open('number');
  const plus = t.root.querySelector('[aria-label="Plus 1"]');
  click(plus);
  click(plus);
  click(t.root.querySelector('[aria-label="Minus 1"]'));
  assert.equal(t.root.querySelector('input.num').value, '213');
  assert.ok(t.text().includes('Send 213'));
  submit(t.root.querySelector('form'));
  await flush();
  assert.deepEqual(t.calls.filter((c) => c.what === 'action')[0].body.body, { q: 'q2', number: 213 });
  t.app.stop();
});

test('3d: picture options sit 2 by 2; the picture has alt text', async () => {
  const t = await open('picture');
  assert.ok(t.root.querySelector('.answers.two'));
  const img = t.root.querySelector('.pic img');
  assert.equal(img.getAttribute('src'), '/api/pocket/t/media/kiwi.svg');
  assert.match(img.getAttribute('alt'), /green fruit/);
  t.app.stop();
});

test('timer: counts down locally, amber under 10 s, answers close at 0', async () => {
  const t = await open('choice', { ws: true });
  t.sockets[0].onopen();
  const bar = t.root.querySelector('[data-timer]');
  assert.equal(bar.getAttribute('role'), 'timer');
  assert.equal(bar.getAttribute('aria-label'), '14 seconds left');
  assert.equal(bar.firstChild.style.transform, 'scaleX(0.7)');
  assert.ok(!bar.classList.contains('low'));
  await t.advance(5000);
  assert.equal(bar.getAttribute('aria-label'), '9 seconds left');
  assert.ok(bar.classList.contains('low'));
  await t.advance(9000);
  assert.equal(bar.getAttribute('aria-label'), '0 seconds left');
  assert.ok([...t.root.querySelectorAll('button[data-answer]')].every((b) => b.disabled));
  assert.ok(t.text().includes("Time's up."));
  click(t.root.querySelector('button[data-answer="A"]'));
  await flush();
  assert.equal(t.calls.filter((c) => c.what === 'action').length, 0);
  t.app.stop();
});

test('lobby preloads pictures one at a time, same origin only', async () => {
  const made = [];
  const Real = g.Image;
  g.Image = class {
    set src(v) {
      made.push(v);
      this._src = v;
    }
  };
  try {
    const t = await open('lobby', { overrides: { state: { ...scenarios.lobby.state, preload: ['/a.png', 'https://evil.example/x.png', '/b.png'] } } });
    assert.deepEqual(made, ['/a.png']);
    t.app.stop();
  } finally {
    g.Image = Real;
  }
});

test('privacy line follows the kit retention', () => {
  const v = { title: 'x', phase: 'lobby', v: 1, kit: 'quiz' };
  assert.equal(m.privacyLine({ ...v, retention: { kind: 'after_end', days: 7 } }), 'Your name and answers are deleted 7 days after the quiz.');
  assert.equal(m.privacyLine({ ...v, retention: { kind: 'after_end', days: 1 } }), 'Your name and answers are deleted 1 day after the quiz.');
  assert.equal(m.privacyLine({ ...v, retention: { kind: 'at_end' } }), 'Your name and answers are deleted when the quiz ends.');
  assert.equal(m.privacyLine({ ...v, retention: { kind: 'after_date', date: '2026-12-04' } }), 'Your name and answers are deleted on Fri, Dec 4.');
  assert.equal(m.privacyLine({ ...v, noun: 'game', retention: { kind: 'after_end', days: 3 } }), 'Your name and answers are deleted 3 days after the game.');
  assert.equal(m.codeFrom('/p/AbC-12'), 'AbC-12');
});

// ---------- static rules ----------

const guestSrc = new URL('../guest/src/', import.meta.url);
const srcFiles = readdirSync(guestSrc).map((f) => [f, readFileSync(new URL(f, guestSrc), 'utf8')]);

test('source: never HTML from data, no style attributes, no third-party URLs, no emoji', () => {
  for (const [f, s] of srcFiles) {
    assert.doesNotMatch(s, /\.innerHTML|\.outerHTML|insertAdjacentHTML|document\.write|DOMParser|createContextualFragment/, f);
    assert.doesNotMatch(s, /setAttribute\(\s*['"]style['"]/, f);
    assert.doesNotMatch(s, /\beval\(|new Function\(/, f);
    assert.doesNotMatch(s, /\p{Extended_Pictographic}/u, `${f} has an emoji`);
    for (const url of s.match(/https?:\/\/[^\s'"`)]+/g) ?? []) assert.ok(url === 'https://kwant.si' || url.startsWith('http://www.w3.org/2000/svg'), `${f}: ${url}`);
  }
  assert.doesNotMatch(readFileSync(new URL('../guest/index.html', import.meta.url), 'utf8'), /style=|<style/);
});

test('tokens: guest.css matches tailwind.config.js and modern.css', () => {
  const css = readFileSync(new URL('guest.css', guestSrc), 'utf8');
  const tw = readFileSync(new URL('../tailwind.config.js', import.meta.url), 'utf8');
  const modern = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
  const vars = Object.fromEntries([...css.matchAll(/--([a-z0-9-]+):\s*([^;]+);/g)].map((x) => [x[1], x[2].trim()]));
  let checked = 0;
  for (const scale of ['zinc', 'sky']) {
    const block = tw.slice(tw.indexOf(`${scale}: {`), tw.indexOf('}', tw.indexOf(`${scale}: {`)));
    for (const [, step, hex] of block.matchAll(/(\d+): '(#[0-9a-f]{6})'/g)) {
      if (!vars[`${scale}-${step}`]) continue;
      assert.equal(vars[`${scale}-${step}`], hex, `${scale}-${step}`);
      checked++;
    }
  }
  const norm = (s) => s.replace(/\s+/g, '').replace(/0\.(\d)/g, '.$1');
  for (const [k, v] of Object.entries(vars).filter(([k]) => k.startsWith('kw-'))) {
    const mm = modern.match(new RegExp(`--${k}:\\s*([^;]+);`));
    assert.ok(mm, `--${k} in modern.css`);
    assert.equal(norm(v), norm(mm[1]), `--${k}`);
    checked++;
  }
  assert.ok(checked >= 29, `checked ${checked}`);
  // Hex colours only in the token block.
  const body = css.slice(css.indexOf('@font-face'));
  assert.doesNotMatch(body, /#[0-9a-fA-F]{3,8}\b/);
});

// ---------- the built bundle (CI builds before testing; locally only when dist/pocket exists) ----------

const dist = new URL('../dist/pocket/', import.meta.url);
const built = existsSync(new URL('index.html', dist));
const needBuild = { skip: !built && !process.env.CI ? 'dist/pocket not built here (npm run build:guest)' : false };

test('built: at most 80 KB gzipped (JS + CSS + HTML + font)', needBuild, () => {
  const files = readdirSync(dist).filter((f) => statSync(new URL(f, dist)).isFile());
  let total = 0;
  const rows = [];
  for (const f of files) {
    const buf = readFileSync(new URL(f, dist));
    const gz = gzipSync(buf, { level: 9 }).length;
    total += gz;
    rows.push(`${f} ${buf.length} B raw, ${gz} B gz`);
  }
  console.log(`guest bundle: ${total} B gzipped\n  ${rows.join('\n  ')}`);
  assert.ok(total <= 80 * 1024, `guest page is ${total} B gzipped, budget 81920`);
  assert.ok(files.some((f) => f.endsWith('.woff2')), 'the font ships as a file');
  assert.ok(files.every((f) => /^(index\.html|g-[\w-]+\.(js|css)|inter-guest-[\w-]+\.woff2)$/.test(f)), files.join(' '));
});

test('built: index.html fits the CSP with no inline script or style; the app is not in it', needBuild, () => {
  const html = readFileSync(new URL('index.html', dist), 'utf8');
  // pa1's fill_boot (src/pocket/guest.rs BOOT_SLOT) swaps null for the boot state only on this exact text.
  assert.ok(html.includes('<script type="application/json" id="pocket-boot">null</script>'), 'boot slot exactly as pa1 looks for it');
  const d = new JSDOM(html).window.document;
  for (const s of d.querySelectorAll('script')) {
    if (s.src) assert.match(s.getAttribute('src'), /^\/p\/_a\/g-[\w-]+\.js$/);
    else assert.equal(s.type, 'application/json', 'only the inert boot JSON is inline');
  }
  assert.equal(d.querySelectorAll('style, [style]').length, 0);
  for (const l of d.querySelectorAll('link[href]')) assert.match(l.getAttribute('href'), /^\/p\/_a\//);
  const js = readdirSync(dist).filter((f) => f.endsWith('.js')).map((f) => readFileSync(new URL(f, dist), 'utf8')).join('\n');
  assert.doesNotMatch(js, /react|innerHTML/i);
  assert.ok(CSP.includes("style-src 'self'"));
});

test('built: the guest files are not in the app service-worker shell', { skip: !existsSync(new URL('../dist/sw.js', import.meta.url)) && !process.env.CI ? 'app not built here' : false }, () => {
  const top = new URL('../dist/', import.meta.url);
  for (const f of readdirSync(top).filter((n) => /^(sw|service-worker).*\.js$/.test(n))) assert.doesNotMatch(readFileSync(new URL(f, top), 'utf8'), /pocket|\/p\/_a\//, f);
});
