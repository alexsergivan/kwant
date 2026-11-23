// pa5b big screen (autonomy/pocket-apps/pa5b/brief.md): every TV state from the fake box (guest/fake/screen.mjs, the
// real QuizView shapes), the screen never calling a host write, the link's token wiped before the first request,
// the phone's backoff while the box is away, reduced motion, and no text under 32 px at 1920.
// The live half (the built page in Chrome under pa1's CSP, at 1920x1080) is guest/e2e/csp.mjs.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';
import { ScreenBox, scenes } from '../guest/fake/screen.mjs';

const dom = new JSDOM('<!doctype html><html><body><div id="app"></div></body></html>', { url: 'http://localhost/p/screen-lobby', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'SVGElement']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { ScreenApp } from './guest/src/screen/app';
  export { screenFrom } from './guest/src/screen/view';
  export { screenToken } from './guest/src/screen/link';
`);
const settle = () => new Promise((r) => setTimeout(r, 0));
const flush = async () => {
  for (let i = 0; i < 6; i++) await settle();
};

/** The fake box behind a fetch, a hand-driven clock and a fake WebSocket. The kph cookie is a flag set by sign-in. */
function harness(scene, { ws = true, still = false, token = 'tok-1' } = {}) {
  const code = `screen-${scene}`;
  const calls = [];
  let now = 1_000_000;
  let timers = [];
  let nextId = 1;
  let signed = false;
  const box = new ScreenBox(code);
  box.up = true;
  const fetch = async (url, init = {}) => {
    const path = url.split(`/api/pocket/${code}/`)[1] ?? '';
    calls.push({ at: now, method: init.method ?? 'GET', path, body: init.body ? JSON.parse(init.body) : undefined });
    if (!box.up) throw new TypeError('Failed to fetch');
    const r = box.handle(path, init.body ? JSON.parse(init.body) : {}, { signed, method: init.method ?? 'GET' });
    if (r.cookie) signed = true;
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
    location: { pathname: `/p/${code}`, protocol: 'http:', host: 'localhost:8790' },
    origin: 'http://localhost:8790',
    still: () => still,
  };
  const root = document.getElementById('app');
  root.replaceChildren();
  const app = new m.ScreenApp(root, env, code, token ?? undefined);
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
  /** A ws push as pa1 sends it: the box's new view as a v1 `{type:"state", ...}` frame. */
  const push = (view) => {
    box.set(view);
    sockets.at(-1).onmessage({ data: JSON.stringify({ type: 'state', ...box.env() }) });
  };
  const $ = (s) => root.querySelector(s);
  const $$ = (s) => [...root.querySelectorAll(s)];
  return { app, root, calls, box, sockets, advance, push, env, $, $$, screen: () => $('[data-screen]')?.getAttribute('data-screen'), text: () => root.textContent.replace(/\s+/g, ' ') };
}

async function open(scene, opts) {
  const t = harness(scene, opts);
  void t.app.start();
  await flush();
  return t;
}

// ---------- every state ----------

const expect = {
  lobby: ['lobby', ['Friday Pub Quiz', 'Join at localhost:8790/p/screen-lobby', 'Scan to play', 'No app, no account', '14 players', 'Mia', 'Noor', "Grab a seat and pick a team name you'd be happy to hear me read out loud.", 'made with kwant']],
  question: ['question', ['Round 2 · 90s music · question 8 of 10', '0:14', 'In which year did the Spice Girls release their first single, "Wannabe"?', 'A1994', 'B1996', 'C1998', 'D2000', '18 of 23answered']],
  locked: ['question', ["Time's up", 'In which year did the Spice Girls', '23 of 23']],
  reveal: ['reveal', ['Answer', '1996 (the right answer)14 picks', '19942 picks', 'Source: en.wikipedia.org: released 7 July 1996 in the UK.', 'Fastest right answer: Mia, Quizzly Bears, in 2.1 seconds.']],
  leaderboard: ['leaderboard', ['After round 2 of 4', 'Quizzly Bears', '1,960', 'Know-It-Owls', '1,090', 'Quizzly Bears jump to first, just 40 ahead.']],
  winners: ['winners', ['Friday Pub Quiz · final scores', 'Winners', 'Quizzly Bears win!', 'Fact Hunters', '3,210 points', '3,420 points', 'The Late Answers', '2,980 points']],
  paused: ['paused', ['Back in a moment', 'The quiz is paused. The timer stopped at 0:12.']],
  ended: ['winners', ['Quizzly Bears win!', '3,420 points']],
  solo: ['leaderboard', ['Mia', '900', 'Leo', '620']],
};

for (const [scene, [screen, lines]] of Object.entries(expect)) {
  test(`state: ${scene}`, async () => {
    const t = await open(scene);
    assert.equal(t.screen(), screen);
    for (const l of lines) assert.ok(t.text().includes(l), `${scene}: "${l}" in: ${t.text()}`);
    t.app.gone(401); // stop timers
  });
}

test('lobby: a 500 px QR of the join link, one creature per player, nobody drops in on the first draw', async () => {
  const t = await open('lobby');
  const qr = t.$('svg.tvqr');
  assert.match(qr.getAttribute('aria-label'), /http:\/\/localhost:8790\/p\/screen-lobby$/);
  assert.ok(qr.querySelector('path').getAttribute('d').length > 200);
  assert.equal(t.$$('.tvplayers .player').length, 14);
  assert.equal(t.$$('.drop-in').length, 0);
  assert.ok(t.$('.tvqm'), 'Quizmaster is there');
  // A new player drops in; the ones already there don't.
  t.push({ ...scenes.lobby, lobby: [...scenes.lobby.lobby, { name: 'Eli', team: 'Brain Freeze' }], players: 15 });
  assert.equal(t.$$('.drop-in').length, 1);
  assert.equal(t.$('.drop-in .nm').textContent, 'Eli');
  assert.ok(t.text().includes('15 players'));
});

test('lobby: past 18 players the creatures get smaller; past 36 only the newest 36 are drawn, the count has all', async () => {
  const t = await open('lobby');
  const many = Array.from({ length: 40 }, (_, i) => ({ name: `P${i}`, team: null }));
  t.push({ ...scenes.lobby, lobby: many.slice(0, 20), players: 20 });
  assert.ok(t.$('.tvplayers').classList.contains('many'));
  t.push({ ...scenes.lobby, lobby: many, players: 40 });
  assert.equal(t.$$('.tvplayers .player').length, 36);
  assert.equal(t.$('.tvplayers .player .nm').textContent, 'P4');
  assert.ok(t.text().includes('40 players'));
});

test('question: the countdown ticks from the box remaining_ms, the bar is a CSSOM transform, low under 10 s; no names', async () => {
  const t = await open('question');
  const bar = t.$('.tvtimer i');
  assert.equal(bar.style.transform, `scaleX(${14 / 30})`);
  assert.equal(t.$('.tvclock').textContent, '0:14');
  await t.advance(1000);
  assert.equal(t.$('.tvclock').textContent, '0:13');
  assert.ok(!t.$('.tvtimer').classList.contains('low'));
  await t.advance(5000);
  assert.equal(t.$('.tvclock').textContent, '0:08');
  assert.ok(t.$('.tvtimer').classList.contains('low'));
  await t.advance(20_000);
  assert.equal(t.$('.tvclock').textContent, '0:00');
  // 18 answered creatures, 5 waiting (dimmed), and the answered row carries no names.
  assert.equal(t.$$('.who svg').length, 23);
  assert.equal(t.$$('.who svg.wait').length, 5);
  assert.equal(t.$('.tvanswered').textContent.includes('Mia'), false);
  assert.equal(t.$$('.tvans .ans').length, 4);
  t.app.gone(401);
});

test('reveal: wrong answers dimmed, the right one marked ok; picks shown as the box counted them, pcts when sent', async () => {
  const t = await open('reveal');
  assert.equal(t.$$('.ans.right').length, 1);
  assert.equal(t.$$('.ans.dim').length, 3);
  assert.equal(t.$('.ans.right .o').textContent, '1996');
  t.push({ ...scenes.reveal, reveal: { ...scenes.reveal.reveal, pcts: [9, 62, 22, 7], line: null } });
  assert.deepEqual(t.$$('.ans .pct').map((p) => p.textContent), ['9%', '62%', '22%', '7%']);
  // No host line from the kit: the box's own counts, as words.
  assert.ok(t.text().includes('14 of 23 got it right.'));
});

test('leaderboard: 6 rows, bars scaled to the leader, arrows only when the box sends a move, 200 ms reorder', async () => {
  const t = await open('leaderboard');
  const rows = t.$$('.lbt .row');
  assert.equal(rows.length, 6);
  assert.ok(rows[0].classList.contains('first'));
  assert.equal(rows[0].querySelector('.bar i').style.transform, 'scaleX(1)');
  assert.equal(rows[5].querySelector('.bar i').style.transform, `scaleX(${1090 / 1960})`);
  assert.equal(t.$$('.lbt .d.up, .lbt .d.down').length, 0);
  // The next board: Fact Hunters jump to first.
  const b = scenes.leaderboard.team_board;
  const next = [{ ...b[1], rank: 1, score: 2100, move: 1 }, { ...b[0], rank: 2, score: 2000, move: -1 }, ...b.slice(2)];
  t.push({ ...scenes.leaderboard, team_board: next });
  const moved = t.$$('.lbt .row');
  assert.equal(moved[0].getAttribute('data-name'), 'Fact Hunters');
  assert.ok(moved[0].classList.contains('moving'));
  assert.ok(!moved[2].classList.contains('moving'));
  assert.ok(moved[0].querySelector('.d').textContent.startsWith('▲ 1'));
  assert.ok(moved[1].querySelector('.d').textContent.startsWith('▼ 1'));
  // Team colours stay with the team (join order), not with the place.
  assert.equal(moved[0].querySelector('.sw').className, rows[1].querySelector('.sw').className);
});

test('winners: one holo sticker, podium 2-1-3, five stars; ended stays on the winners', async () => {
  const t = await open('winners');
  assert.equal(t.$$('.kw-sticker-holo').length, 1);
  assert.deepEqual(t.$$('.podium .pod').map((p) => p.className), ['pod p2', 'pod p1', 'pod p3']);
  assert.equal(t.$$('.tvstars svg').length, 5);
  assert.ok(t.$('.tvstars').classList.contains('twinkle'));
  // The box ends the pocket: the screen keeps the winners up.
  t.push({ ...scenes.winners, ended: true });
  assert.equal(t.screen(), 'winners');
  t.app.gone(410, 'ended');
  assert.equal(t.screen(), 'winners');
});

test('reduced motion: no drop-in, no twinkle, no reorder slide', async () => {
  const t = await open('lobby', { still: true });
  t.push({ ...scenes.lobby, lobby: [...scenes.lobby.lobby, { name: 'Eli', team: null }] });
  assert.equal(t.$$('.drop-in').length, 0);
  t.push(scenes.leaderboard);
  const b = scenes.leaderboard.team_board;
  t.push({ ...scenes.leaderboard, team_board: [b[1], b[0], ...b.slice(2)].map((r, i) => ({ ...r, rank: i + 1 })) });
  assert.equal(t.$$('.moving').length, 0);
  assert.ok(t.$$('.lbt .row').every((r) => !r.style.transform));
  t.push(scenes.winners);
  assert.ok(!t.$('.tvstars').classList.contains('twinkle'));
  const css = readFileSync(new URL('../guest/src/screen/screen.css', import.meta.url), 'utf8');
  const rm = css.slice(css.indexOf('@media (prefers-reduced-motion: reduce)'));
  assert.match(rm, /drop-in[^}]*animation: none/);
  assert.match(rm, /tvtimer i[^}]*transition: none/);
});

// ---------- read-only, sign-in and the link ----------

test('the screen signs in once with its token, then only reads host/state?view=screen and ws?view=screen; never a host write', async () => {
  const t = await open('question');
  assert.deepEqual(t.calls[0], { at: t.calls[0].at, method: 'POST', path: 'host/signin', body: { key: 'tok-1' } });
  assert.equal(t.sockets[0].url, 'ws://localhost:8790/api/pocket/screen-question/ws?view=screen');
  t.sockets[0].onopen();
  // The socket drops: the screen asks state, gets it, opens again.
  t.sockets[0].onclose();
  await flush();
  // The box goes away and comes back; the viewer taps "Try now"; time passes.
  t.box.up = false;
  t.app.wake();
  await flush();
  assert.equal(t.screen(), 'away');
  t.$('.tvbtn').dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await flush();
  t.box.up = true;
  await t.advance(30_000);
  assert.equal(t.screen(), 'question');
  for (const c of t.calls.slice(1)) assert.match(c.path, /^host\/state\?view=screen(&since=\d+)?$/, c.path);
  assert.equal(t.calls.filter((c) => c.path === 'host/signin').length, 1);
  assert.ok(t.calls.every((c) => c.method === 'GET' || c.path === 'host/signin'));
  assert.deepEqual(t.box.writes, []);
  t.app.gone(401);
});

test('no WebSocket: the screen polls host/state?view=screen&since=', async () => {
  const t = await open('leaderboard', { ws: false });
  await t.advance(4100);
  const polls = t.calls.filter((c) => c.path.startsWith('host/state'));
  assert.ok(polls.length >= 2);
  assert.ok(polls.every((c) => /^host\/state\?view=screen&since=\d+$/.test(c.path)), polls.map((c) => c.path).join(' '));
  assert.deepEqual(t.box.writes, []);
  t.app.gone(401);
});

test('the fake box refuses any host write from a screen session (403 screen_only) and records it', () => {
  const box = new ScreenBox('screen-question');
  assert.equal(box.handle('host/signin', { key: 'x' }, { method: 'POST' }).status, 200);
  for (const w of ['host/action', 'host/next', 'host/pause', 'host/end']) assert.equal(box.handle(w, {}, { signed: true, method: 'POST' }).status, 403);
  assert.equal(box.writes.length, 4);
});

test('an expired or used link says so; a reload with the cookie reads state without signing in', async () => {
  const t = await open('expired');
  assert.equal(t.screen(), 'signed-out');
  assert.ok(t.text().includes('This screen link has expired'));
  const r = harness('lobby', { token: null });
  void r.app.start();
  await flush();
  assert.equal(r.calls[0].path, 'host/state?view=screen');
  assert.equal(r.screen(), 'signed-out', 'no cookie in this fake session, so 401');
});

test('away: the phone backoff (2, 5, 10, 20 s, then every 20 s) and a calm full-screen line', async () => {
  const t = harness('away');
  void t.app.start();
  await flush();
  assert.equal(t.screen(), 'away');
  assert.ok(t.text().includes("The host's box is away"));
  assert.ok(t.text().includes('Trying again in 2 s'));
  await t.advance(60_000);
  const at = t.calls.map((c) => c.at);
  const gaps = at.slice(1).map((x, i) => (x - at[i]) / 1000);
  assert.deepEqual(gaps.slice(0, 5), [2, 5, 10, 20, 20]);
  assert.ok(t.calls.every((c) => c.path === 'host/signin'), 'keeps trying the sign-in until the box answers');
});

test('link: the token is read and wiped from the address bar, a reload stays a screen, a phone is a phone', () => {
  const store = new Map();
  const seen = [];
  const w = (hash) => ({
    location: { hash, pathname: '/p/AbC', search: '' },
    history: { replaceState: (_d, _u, url) => seen.push(url) },
    sessionStorage: { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v) },
  });
  assert.equal(m.screenToken(w(''), 'AbC'), undefined);
  assert.equal(m.screenToken(w('#screen=t%2Fk-9'), 'AbC'), 't/k-9');
  assert.deepEqual(seen, ['/p/AbC']);
  assert.equal(store.get('kw-screen-AbC'), '1');
  assert.equal(m.screenToken(w(''), 'AbC'), '');
  assert.equal(m.screenToken(w(''), 'Other'), undefined);
});

test('main.ts wipes the fragment at module start, before any fetch, and loads the screen lazily', () => {
  const src = readFileSync(new URL('../guest/src/main.ts', import.meta.url), 'utf8');
  const wipe = src.indexOf('const screen = screenToken(w, code);');
  assert.ok(wipe > 0);
  for (const call of ['w.fetch(', 'app.start(', 'await kit(']) assert.ok(src.indexOf(call) > wipe, call);
  assert.match(src, /await import\('\.\/screen\/screen'\)/);
  assert.doesNotMatch(src, /^import .*screen\/(app|screen|qr)/m, 'the screen code is never in the phone entry');
});

test('fit: the 1920x1080 stage scales by the smaller ratio (letterbox)', async () => {
  const t = await open('lobby');
  t.app.fit(1280, 720);
  assert.equal(t.$('.tv').style.getPropertyValue('--s'), String(1280 / 1920));
  t.app.fit(1024, 768);
  assert.equal(t.$('.tv').style.getPropertyValue('--s'), String(1024 / 1920));
  t.app.fit(3440, 1440);
  assert.equal(t.$('.tv').style.getPropertyValue('--s'), String(1440 / 1080));
});

test('view: reads the real QuizView screen shapes, computes nothing, ignores what it does not know', () => {
  const box = new ScreenBox('screen-reveal');
  const { view } = m.screenFrom(box.env());
  assert.equal(view.reveal.right, 14);
  assert.deepEqual(view.reveal.picks, [2, 14, 5, 2]);
  assert.equal(view.reveal.pcts, undefined);
  assert.equal(m.screenFrom({ api: 'pocket/1', pocket: {}, view: { phase: 'dance' } }), undefined);
  const solo = m.screenFrom(new ScreenBox('screen-solo').env()).view;
  assert.equal(solo.board.length, 10);
  assert.equal(solo.board[0].name, 'Mia');
});

test('screen.css: nothing under 32 px at 1920 (the sticker plate is drawn at 3.2x), hex only in its token block, holo stops match modern.css', () => {
  const css = readFileSync(new URL('../guest/src/screen/screen.css', import.meta.url), 'utf8');
  for (const [, px] of css.matchAll(/font-size:\s*(\d+)px/g)) assert.ok(Number(px) >= 32, `font-size ${px}px`);
  const body = css.slice(css.indexOf('/* ---- stage'));
  assert.doesNotMatch(body, /#[0-9a-fA-F]{3,8}\b/);
  const modern = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
  const foil = modern.slice(modern.indexOf('.kw-sticker-holo::before {'));
  const stops = [...foil.slice(0, foil.indexOf('}')).matchAll(/#[0-9a-f]{6}/g)].map((x) => x[0]).slice(0, 5);
  const vars = Object.fromEntries([...css.matchAll(/--([a-z0-9-]+):\s*([^;]+);/g)].map((x) => [x[1], x[2].trim()]));
  assert.deepEqual([1, 2, 3, 4, 5].map((i) => vars[`holo-${i}`]), stops);
  assert.match(modern, new RegExp(`background: ${vars['sticker-plate']};`));
  // The guest sticker is 12 px; at scale(3.2) that is 38.4 px on the TV.
  assert.match(css, /\.bigsticker \{[^}]*scale\(3\.2\)/);
});

test('solo board: no team swatch, players have no team colour; the team board keeps one per row', async () => {
  const s = await open('solo');
  assert.equal(s.$$('.lbt .sw').length, 0);
  s.app.gone(401);
  const t = await open('leaderboard');
  assert.equal(t.$$('.lbt .sw').length, 6);
  t.app.gone(401);
});

// ---------- pa5b round 2 (Mira's design check, Rigel's D1 and notes, Lin's v1.1 notes) ----------

test('question: the clock sits in the header row; the answered row is in the flow after the answers, never pinned over them', async () => {
  const t = await open('question');
  assert.ok(t.$('.tvtop').classList.contains('withclock'));
  assert.equal(t.$('.tvanswered').parentElement, t.$('.tvq'));
  assert.equal(t.$('.tvq').lastElementChild, t.$('.tvanswered'));
  const css = readFileSync(new URL('../guest/src/screen/screen.css', import.meta.url), 'utf8');
  assert.match(css, /\.tv \.tvclock \{[^}]*top: 22px; font-size: 96px;/);
  assert.match(css, /\.tv \.tvq \{[^}]*top: 170px; bottom: 44px;/);
  assert.doesNotMatch(css, /\.tvanswered \{[^}]*position: absolute/);
  t.push({ ...scenes.locked });
  assert.equal(t.$('.tvclock'), null);
  assert.equal(t.$('.tvtop').classList.contains('withclock'), false);
  t.app.gone(401);
});

test('question: 5 or 6 answers or a long question get the compact size; the validator limits get the tight one', async () => {
  const t = await open('question');
  const q = scenes.question.question;
  assert.equal(t.$('.tvq').className, 'tvq');
  t.push({ ...scenes.question, question: { ...q, options: ['A1', 'B2', 'C3', 'D4', 'E5', 'F6'] } });
  assert.equal(t.$('.tvq').className, 'tvq c1');
  assert.equal(t.$$('.ans').length, 6);
  t.push({ ...scenes.question, question: { ...q, text: 'x'.repeat(140) } });
  assert.equal(t.$('.tvq').className, 'tvq c1');
  t.push({ ...scenes.question, question: { ...q, text: 'x '.repeat(150), options: Array.from({ length: 6 }, () => 'y'.repeat(80)) } });
  assert.equal(t.$('.tvq').className, 'tvq c2');
  t.app.gone(401);
});

test('reveal: dimmed answers keep readable text (the card is dimmed, not the text); the Quizmaster is in the flow', async () => {
  const t = await open('reveal');
  assert.ok(t.$('.tvq.rv > .tvhost'));
  assert.equal(t.$('.tvfoot'), null);
  const css = readFileSync(new URL('../guest/src/screen/screen.css', import.meta.url), 'utf8');
  assert.doesNotMatch(css, /\.ans\.dim \{[^}]*opacity/);
  assert.match(css, /\.tv \.ans\.dim \{[^}]*color: var\(--zinc-500\)/);
  assert.match(css, /\.tv \.ans\.dim \.pct \{ color: var\(--zinc-400\); \}/);
  t.app.gone(401);
});

test('a reveal frame without reveal falls back to the locked question instead of throwing; an empty final board is "ended"', async () => {
  const t = await open('reveal');
  t.push({ ...scenes.reveal, reveal: null });
  assert.equal(t.screen(), 'question');
  assert.match(t.text(), /Time's up/);
  t.push({ ...scenes.winners, team_board: [], leaderboard: [] });
  assert.equal(t.screen(), 'ended');
  assert.doesNotMatch(t.text(), /(^|\s)win!/);
  t.app.gone(401);
});

test('winners: the podium is as wide as its name (380 to 560 px), the blocks stay 380', () => {
  const css = readFileSync(new URL('../guest/src/screen/screen.css', import.meta.url), 'utf8');
  assert.match(css, /\.tv \.pod \{[^}]*width: auto; min-width: 380px; max-width: 560px;/);
  assert.match(css, /\.tv \.pod \.blk \{ width: 380px;/);
});

test('leaderboard: no move column when the box sends no moves; the solo board shows each player creature from the lobby', async () => {
  const s = harness('solo');
  s.box.set({ ...scenes.lobby, teams: false, lobby: scenes.solo.leaderboard.slice(0, 5).map((r) => ({ name: r.name, team: null })) });
  void s.app.start();
  await flush();
  assert.equal(s.screen(), 'lobby');
  s.push({ ...scenes.solo, lobby: null }); // the real kit sends lobby only in the lobby phase (view.rs)
  assert.equal(s.screen(), 'leaderboard');
  assert.ok(s.$('.lbt').classList.contains('nomove'));
  assert.equal(s.$$('.lbt .d').length, 0);
  assert.equal(s.$$('.lbt svg.sc').length, 5, s.$$('.lbt .row').map((r) => r.getAttribute('data-name') + ':' + r.querySelectorAll('svg.sc').length).join(' '));
  s.app.gone(401);
});

test('expired and gone: the Quizmaster rests (no thinking dots); paused and away keep thinking', async () => {
  const t = await open('expired');
  assert.equal(t.screen(), 'signed-out');
  assert.match(t.text(), /This screen link has expired/);
  const rest = t.$('.tvqm').outerHTML;
  const p = await open('paused');
  assert.notEqual(p.$('.tvqm').outerHTML, rest);
  p.app.gone(401);
});

test('sign-in 429 slow_down retries on the backoff with the same token instead of showing signed-out', async () => {
  const t = harness('lobby');
  const real = t.box.handle.bind(t.box);
  let n = 0;
  t.box.handle = (path, body, o) => (path === 'host/signin' && n++ < 2 ? { status: 429, body: { error: 'slow_down', message: 'Slow down.' } } : real(path, body, o));
  void t.app.start();
  await flush();
  assert.equal(t.screen(), 'away');
  await t.advance(2_000);
  await t.advance(5_000);
  assert.equal(t.screen(), 'lobby');
  const ins = t.calls.filter((c) => c.path === 'host/signin');
  assert.equal(ins.length, 3);
  assert.ok(ins.every((c) => c.body.key === 'tok-1'));
  t.app.gone(401);
});

test('the fake uses the real v1.1 messages for a used link and a screen-only refusal', () => {
  const b = new ScreenBox('screen-expired');
  assert.equal(b.handle('host/signin', { key: 'x' }, { method: 'POST' }).body.message, 'This link was already used or is too old. Open a new one from the card.');
  const c = new ScreenBox('screen-lobby');
  assert.equal(c.handle('host/next', {}, { method: 'POST', signed: true }).body.message, 'This screen can only show the game.');
});

// ---------- pa5b round 3 (Rigel D2 and D3): measure, then step the density up ----------

test('the question column steps up its density (c1, c2, then c2 c3 c4) while it is taller than its box, and never changes what is shown', async () => {
  const P = dom.window.HTMLElement.prototype;
  const sh = Object.getOwnPropertyDescriptor(P, 'scrollHeight') ?? Object.getOwnPropertyDescriptor(dom.window.Element.prototype, 'scrollHeight');
  const ch = Object.getOwnPropertyDescriptor(P, 'clientHeight') ?? Object.getOwnPropertyDescriptor(dom.window.Element.prototype, 'clientHeight');
  // A column that only fits once it is at c2: before that it is 100 px taller than its box.
  let fitsAt = 'c2';
  Object.defineProperty(P, 'clientHeight', { configurable: true, get() { return this.classList.contains('tvq') ? 866 : 0; } });
  Object.defineProperty(P, 'scrollHeight', { configurable: true, get() { return this.classList.contains('tvq') && !this.classList.contains(fitsAt) ? 966 : 866; } });
  try {
    const t = await open('question');
    assert.equal(t.$('.tvq').className, 'tvq c2', 'short question, but it overflowed: none to c1 to c2');
    const text = t.text();
    fitsAt = 'c3';
    t.push({ ...scenes.reveal });
    assert.equal(t.$('.tvq').className, 'tvq rv c2 c3', 'a reveal with a quote that still overflows at c2 gets c3 on top');
    fitsAt = 'never';
    t.push({ ...scenes.question });
    assert.equal(t.$('.tvq').className, 'tvq c2 c3 c4', 'stops at the last step');
    assert.equal(t.text(), text, 'layout only: the same words are shown');
    fitsAt = 'tvq';
    t.push({ ...scenes.question });
    assert.equal(t.$('.tvq').className, 'tvq', 'a column that fits keeps the size density() chose');
    t.app.gone(401);
  } finally {
    if (sh) Object.defineProperty(dom.window.Element.prototype, 'scrollHeight', sh);
    if (ch) Object.defineProperty(dom.window.Element.prototype, 'clientHeight', ch);
    delete P.scrollHeight;
    delete P.clientHeight;
  }
});

test('the tight steps keep every font at 32 px or more', () => {
  const css = readFileSync(new URL('../guest/src/screen/screen.css', import.meta.url), 'utf8');
  const rules = css.match(/\.tvq\.c[34][^{]*\{[^}]*\}/g) ?? [];
  assert.ok(rules.length >= 5);
  for (const r of rules) for (const m of r.matchAll(/font-size: (\d+)px/g)) assert.ok(Number(m[1]) >= 32, r);
  assert.match(css, /\.tv \.tvq\.c4 \.tvans \{ grid-template-columns: 1fr;/);
});
