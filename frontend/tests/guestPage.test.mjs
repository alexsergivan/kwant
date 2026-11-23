// pa5 guest page (autonomy/pocket-apps/pa5/brief.md): every phone screen from the fake server, reconnect and backoff,
// first tap counts, the static half of the CSP rules, token drift, and the 80 KB gzipped budget.
// The live half of the CSP test (the built page in Chrome under exactly pa1's policy) is guest/e2e/csp.mjs.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { readFileSync, readdirSync, existsSync, statSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import { loadBundled } from './bundle.mjs';
import { scenarios, order, CSP } from '../guest/fake/views.mjs';

const dom = new JSDOM('<!doctype html><html><body><div id="app"></div></body></html>', { url: 'http://localhost/p/join', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'MouseEvent', 'Image', 'SVGElement']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { GuestApp, privacyLine, day } from './guest/src/app';
  export { BACKOFF, backoff, codeFrom } from './guest/src/api';
`);
const settle = () => new Promise((r) => setTimeout(r, 0));
const flush = async () => {
  for (let i = 0; i < 6; i++) await settle();
};
const clone = (v) => (v && typeof v === 'object' ? JSON.parse(JSON.stringify(v).replaceAll('/CODE/', '/t/')) : v);

/** A fake box (pa1's guest API, from views.mjs) and a hand-driven clock. */
function harness(name, { ws = false, overrides = {} } = {}) {
  const s = { ...scenarios[name], ...overrides };
  const calls = [];
  let now = 1_000_000;
  let timers = [];
  let nextId = 1;
  let seat = null;
  const box = { up: true, s, actionReply: null };
  const json = (status, body) => ({ ok: status < 400, status, json: async () => body });
  const answer = (v) => (typeof v === 'number' ? json(v, { error: v === 410 ? 'removed' : 'x' }) : json(200, { view: clone(v) }));
  const fetch = async (url, init = {}) => {
    const what = url.split('/').pop();
    calls.push({ at: now, what, body: init.body ? JSON.parse(init.body) : undefined });
    if (!box.up) throw new TypeError('Failed to fetch');
    if (what === 'state') return answer(seat ?? box.s.state ?? box.s.rejoin);
    if (what === 'rejoin') return answer(seat ?? box.s.rejoin);
    if (what === 'join') {
      if (box.joinError) return json(409, { error: box.joinError });
      seat = clone(box.s.join);
      return json(200, { view: seat });
    }
    if (what === 'action') {
      if (box.actionReply) return box.actionReply();
      return box.s.action ? answer(box.s.action) : json(200, {});
    }
    return json(404, {});
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
  'join-solo': ['lobby', ["You're in, Mia!"]],
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
  assert.deepEqual(join.body, { name: 'Mia', avatar: 'visor', team: 't1' });
  assert.equal(t.screen(), 'lobby');
  assert.ok(t.text().includes("You're in, Mia!"));
  t.app.stop();
});

test('join: a new team sends new_team; a full quiz shows the full screen', async () => {
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
  assert.deepEqual(t.calls.filter((c) => c.what === 'join').at(-1).body, { name: 'Leo', avatar: 'antenna', new_team: 'Brain Freeze' });
  assert.equal(t.screen(), 'full');
  t.app.stop();
});

test('join: a name typed survives a lobby push (patch, not redraw)', async () => {
  const t = await open('join', { ws: true });
  const name = t.root.querySelector('#name');
  type(name, 'Pri');
  name.focus();
  const ws = t.sockets[0];
  ws.onopen();
  const v = clone(scenarios.join.state);
  v.teams[0].count = 3;
  ws.onmessage({ data: JSON.stringify({ view: v }) });
  await flush();
  assert.equal(t.root.querySelector('#name'), name, 'same input element');
  assert.equal(name.value, 'Pri');
  assert.equal(document.activeElement, name);
  assert.ok(t.text().includes('3 of 4'));
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
  assert.equal(t.calls.filter((c) => c.what === 'state').length, 2);
  t.box.up = false;
  await t.advance(2000);
  assert.equal(t.screen(), 'away');
  t.app.stop();
});

test('3j: removed stops all traffic and never offers rejoin', async () => {
  const t = await open('choice', { ws: true });
  t.box.s = { ...t.box.s, state: 410, rejoin: 410 };
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
  t.box.actionReply = () => new Promise((r) => (release = () => r({ ok: true, status: 200, json: async () => ({ view: clone(scenarios.choice.action) }) })));
  const btn = (k) => t.root.querySelector(`button[data-answer="${k}"]`);
  assert.equal(btn('B').getAttribute('aria-label'), 'B: 1996');
  click(btn('B'));
  click(btn('C'));
  click(btn('B'));
  await flush();
  const sent = t.calls.filter((c) => c.what === 'action');
  assert.equal(sent.length, 1);
  assert.deepEqual(sent[0].body, { name: 'answer', q: 'q8', body: { choice: 'B' } });
  assert.ok(btn('B').classList.contains('pick'));
  assert.equal(btn('B').getAttribute('aria-pressed'), 'true');
  assert.equal(btn('C').disabled, true);
  assert.ok(t.text().includes('Sent. Waiting for the others.'));
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
  assert.deepEqual(t.calls.filter((c) => c.what === 'action').map((c) => c.body.body), [{ choice: 'A' }, { choice: 'A' }, { choice: 'A' }]);
  t.app.stop();
});

test('3b: text answer sends once and keeps typing across pushes', async () => {
  const t = await open('text', { ws: true });
  const input = t.root.querySelector('input[data-answer]');
  type(input, 'Canbera');
  t.sockets[0].onopen();
  t.sockets[0].onmessage({ data: JSON.stringify({ view: { ...clone(scenarios.text.rejoin), question: { ...scenarios.text.rejoin.question, secs_left: 20 } } }) });
  await flush();
  assert.equal(t.root.querySelector('input[data-answer]'), input);
  assert.equal(input.value, 'Canbera');
  submit(t.root.querySelector('form'));
  submit(t.root.querySelector('form'));
  await flush();
  const sent = t.calls.filter((c) => c.what === 'action');
  assert.equal(sent.length, 1);
  assert.deepEqual(sent[0].body.body, { text: 'Canbera' });
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
  assert.deepEqual(t.calls.filter((c) => c.what === 'action')[0].body.body, { number: 213 });
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
    const t = await open('lobby', { overrides: { rejoin: { ...scenarios.lobby.rejoin, preload: ['/a.png', 'https://evil.example/x.png', '/b.png'] } } });
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
