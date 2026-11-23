// pa4b host remote (autonomy/pocket-apps/pa4b/brief.md): every remote state from the fake box (guest/fake/remote.mjs,
// the real HostView shapes), the link's token read and wiped before the first request, sign-in errors, the big Next
// button per phase (one tap reveals from an open question), near-miss judge bodies, ±100 adjusts with Undo, Remove and
// End asking first, focus kept in sheets, and nothing written while the box is away.
// The live half (the built page in Chrome under pa1's CSP, at 390) is guest/e2e/remote.mjs.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';
import { RemoteBox } from '../guest/fake/remote.mjs';

const dom = new JSDOM('<!doctype html><html><body><div id="app"></div></body></html>', { url: 'http://localhost/p/remote-lobby', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'SVGElement']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { RemoteApp, LATER } from './guest/src/remote/app';
  export { remoteFrom, nextStep } from './guest/src/remote/view';
  export { remoteToken } from './guest/src/remote/link';
`);
const settle = () => new Promise((r) => setTimeout(r, 0));
const flush = async () => {
  for (let i = 0; i < 8; i++) await settle();
};

/** The fake box behind a fetch, a hand-driven clock and a fake WebSocket. The kph cookie is the sign-in's role. */
function harness(scene, { token = 'tok-1', ws = true } = {}) {
  const code = `remote-${scene}`;
  const calls = [];
  let now = 1_000_000;
  let timers = [];
  let nextId = 1;
  let signed = false;
  const box = new RemoteBox(code);
  const fetch = async (url, init = {}) => {
    const path = url.split(`/api/pocket/${code}/`)[1] ?? '';
    const body = init.body ? JSON.parse(init.body) : undefined;
    calls.push({ method: init.method ?? 'GET', path, body, url });
    if (box.down) throw new TypeError('Failed to fetch');
    const r = box.handle(path, body ?? {}, { signed, method: init.method ?? 'GET' });
    if (r.cookie) signed = r.cookie;
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
  };
  const root = document.getElementById('app');
  root.replaceChildren();
  const app = new m.RemoteApp(root, env, code, token ?? undefined);
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
  /** A ws push as pa1 sends it: the box's state as a v1 `{type:"state", ...}` frame, after `edit` changes it. */
  const push = (edit) => {
    if (edit) edit(box);
    box.version++;
    sockets.at(-1).onmessage({ data: JSON.stringify({ type: 'state', ...box.env() }) });
  };
  const $ = (s) => root.querySelector(s);
  const $$ = (s) => [...root.querySelectorAll(s)];
  const click = async (s) => {
    const el = typeof s === 'string' ? $(s) : s;
    assert.ok(el, `no ${s}`);
    el.click();
    await flush();
  };
  const writes = () => calls.filter((c) => c.method === 'POST' && c.path !== 'host/signin').map((c) => [c.path, c.body]);
  return { app, root, calls, box, sockets, advance, push, env, $, $$, click, writes, screen: () => $('[data-screen]')?.getAttribute('data-screen'), text: () => root.textContent.replace(/\s+/g, ' ') };
}

async function open(scene, opts) {
  const t = harness(scene, opts);
  void t.app.start();
  await flush();
  return t;
}
const tab = (t, name) => t.click(`[data-k="tab-${name}"]`);

// ---------- the link ----------

test('link: #remote=<token> is read, wiped from the address bar before any request, and a reload knows it was a remote', () => {
  const store = new Map();
  const replaced = [];
  const w = (hash) => ({
    location: { hash, pathname: '/p/QZ7K', search: '' },
    history: { replaceState: (_a, _b, url) => replaced.push(url) },
    sessionStorage: { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v) },
  });
  assert.equal(m.remoteToken(w('#remote=abc%2Bdef'), 'QZ7K'), 'abc+def');
  assert.deepEqual(replaced, ['/p/QZ7K']);
  // The reload: no token in the address any more, but this tab was a remote, so it reads host/state on its cookie.
  assert.equal(m.remoteToken(w(''), 'QZ7K'), '');
  assert.equal(m.remoteToken(w(''), 'OTHER'), undefined);
  assert.equal(m.remoteToken(w('#screen=x'), 'NEW'), undefined);
});

test('sign-in: one POST host/signin {token} (never a query string), then host/state and ws?view=host on the cookie', async () => {
  const t = await open('question', { token: 'tok-9' });
  assert.equal(t.calls[0].method, 'POST');
  assert.equal(t.calls[0].path, 'host/signin');
  assert.deepEqual(t.calls[0].body, { token: 'tok-9' });
  assert.ok(t.calls.every((c) => !c.url.includes('tok-9')), 'the token is never in a URL');
  assert.match(t.sockets[0].url, /\/api\/pocket\/remote-question\/ws\?view=host$/);
  assert.equal(t.screen(), 'live');
  assert.ok(t.text().includes('Remote · Friday Pub Quiz'));
  assert.ok(t.text().includes('Only you see this'));
  assert.ok(t.$('.qm48'), 'the Quizmaster is in the header');
  t.app.gone(401);
});

test('sign-in: a used or old link, signed out, a screen session, a deleted quiz and slow_down each get their own screen', async () => {
  const a = await open('expired');
  assert.equal(a.screen(), 'expired');
  assert.ok(a.text().includes('This link was already used or is too old'));
  assert.ok(a.text().includes('Open a new one from the card'));
  const b = await open('lobby', { token: null });
  assert.equal(b.screen(), 'signed-out');
  assert.ok(b.text().includes('Open the remote again from the card'));
  const c = await open('lobby', { token: 'screen' });
  assert.equal(c.screen(), 'screen-only');
  const d = harness('lobby');
  d.box.handle = () => ({ status: 404, body: { error: 'not_found' } });
  void d.app.start();
  await flush();
  assert.equal(d.screen(), 'gone');
  const e = harness('lobby');
  const real = e.box.handle.bind(e.box);
  let slow = true;
  e.box.handle = (...a) => (slow ? { status: 429, body: { error: 'slow_down', message: 'Too many tries from here. Wait a moment.' } } : real(...a));
  void e.app.start();
  await flush();
  assert.equal(e.screen(), 'slow');
  slow = false;
  await e.click('[data-k="retry"]');
  assert.equal(e.screen(), 'live');
  e.app.gone(401);
});

test('sign-in: while the box is away the remote waits with backoff and signs in once it answers', async () => {
  const t = harness('lobby');
  t.box.down = true;
  void t.app.start();
  await flush();
  assert.equal(t.screen(), 'loading');
  assert.ok(t.text().includes("Can't reach your quiz box"));
  t.box.down = false;
  await t.advance(30_000);
  assert.equal(t.screen(), 'live');
  assert.equal(t.calls.filter((c) => c.path === 'host/signin').length >= 2, true);
  t.app.gone(401);
});

test('socket: signed_out (a host-key rotation) goes to "Open the remote again"; gone to the deleted screen', async () => {
  const t = await open('question');
  t.sockets[0].onmessage({ data: JSON.stringify({ type: 'signed_out' }) });
  await flush();
  assert.equal(t.screen(), 'signed-out');
  const u = await open('question');
  u.sockets[0].onmessage({ data: JSON.stringify({ type: 'gone' }) });
  await flush();
  assert.equal(u.screen(), 'gone');
});

// ---------- the Game tab ----------

const nexts = {
  lobby: ['Start', 'question 1'],
  question: ['Reveal now', 'closes answers, shows the answer'],
  locked: ['Reveal now', 'shows the answer'],
  reveal: ['Next question', 'question 5, or scores if the round is over'],
  leaderboard: ['Next round', 'round 3'],
  paused: ['Resume', ''],
  last: ['Show winners', ''],
};
for (const [scene, [label, sub]] of Object.entries(nexts)) {
  test(`game: ${scene} — the big button says ${label}`, async () => {
    const t = await open(scene);
    assert.equal(t.$('[data-k="next"] b').textContent, label);
    assert.equal(t.$('[data-k="next"] small')?.textContent ?? '', sub);
    assert.equal(t.$('[data-k="next"]').getAttribute('aria-disabled'), null);
    t.app.gone(401);
  });
}

test('game: question shows round, number, the answer only the host sees, the count and a ticking clock', async () => {
  const t = await open('question');
  const s = t.text();
  for (const l of ['Round 2 · question 4 of 10', 'In which year did the Spice Girls', '18 of 23 answered', 'answer: B, 1996', '0:14']) assert.ok(s.includes(l), `"${l}" in ${s}`);
  await t.advance(3000);
  assert.equal(t.$('.clock').textContent, '0:11');
  assert.equal(t.$('.rbar i').style.getPropertyValue('--f'), String(11 / 30));
  assert.equal(t.$('.rchip').textContent, 'Live');
  t.app.gone(401);
});

test('game: one tap of Reveal now from an open question sends next twice (lock, then reveal)', async () => {
  const t = await open('question');
  await t.click('[data-k="next"]');
  assert.deepEqual(t.writes(), [
    ['host/action', { name: 'next', body: {} }],
    ['host/action', { name: 'next', body: {} }],
  ]);
  assert.equal(t.box.phase, 'reveal');
  assert.equal(t.$('[data-k="next"] b').textContent, 'Next question');
  t.app.gone(401);
});

test('game: Start, then Pause and Resume; paused shows the stopped clock and no Pause tile', async () => {
  const t = await open('lobby');
  assert.ok(t.text().includes('14 players are in.'));
  assert.equal(t.$('[data-k="pause"]'), null, 'no controls in the lobby');
  await t.click('[data-k="next"]');
  assert.deepEqual(t.writes().at(-1), ['host/action', { name: 'start', body: {} }]);
  assert.equal(t.$('.rnow .meta').textContent, 'Round 1 · question 1 of 10');
  await t.click('[data-k="pause"]');
  assert.deepEqual(t.writes().at(-1), ['host/action', { name: 'pause', body: {} }]);
  assert.equal(t.$('.rchip').textContent, 'Paused');
  assert.ok(t.text().includes('The timer stopped at 0:30.'));
  assert.equal(t.$('[data-k="pause"]'), null);
  await t.click('[data-k="next"]');
  assert.deepEqual(t.writes().at(-1), ['host/action', { name: 'resume', body: {} }]);
  assert.equal(t.$('.rchip').textContent, 'Live');
  t.app.gone(401);
});

test('game: nobody joined yet: Start is disabled and says why', async () => {
  const t = await open('empty');
  assert.ok(t.text().includes('Nobody has joined yet.'));
  // Words only: the kit allows Start with nobody, so the copy never promises it is locked (Mira round 1, fix 3).
  assert.ok(t.text().includes('Players show up here as they scan the code. Start when everyone is in.'));
  assert.ok(!t.text().includes('first player joins'));
  // host_can says start; the engine refuses with nobody: the box's words show.
  await t.click('[data-k="next"]');
  assert.ok(t.text().includes('Nobody has joined yet.'));
  t.app.gone(401);
});

test('game: Skip question and Voice are drawn but disabled with the reason (not in the quiz kit yet)', async () => {
  const t = await open('question');
  for (const k of ['skip', 'voice']) {
    const b = t.$(`[data-k="${k}"]`);
    assert.equal(b.getAttribute('aria-disabled'), 'true');
    assert.equal(b.getAttribute('data-later'), m.LATER[k]);
  }
  // Touch has no hover: the reason is also a visible line under the row.
  assert.equal(t.$('[data-k="later-line"]').textContent, 'Skip and Voice come in a later update.');
  await t.click('[data-k="skip"]');
  assert.deepEqual(t.writes(), []);
  t.app.gone(401);
});

test('Quizmaster mood: no badge in lobby, live and paused; thinking only while the box is away; finished when ended', async () => {
  const badge = (t) => t.$('.rhead .mood')?.getAttribute('class') ?? null;
  const lobby = await open('lobby');
  assert.equal(badge(lobby), null, 'lobby');
  assert.equal(lobby.$('[data-k="later-line"]'), null, 'no later line in the lobby');
  lobby.app.gone(401);
  for (const scene of ['question', 'paused']) {
    const t = await open(scene);
    assert.equal(badge(t), null, scene);
    t.app.gone(401);
  }
  const a = await open('question', { ws: false });
  a.box.down = true;
  await a.advance(3000);
  assert.ok(a.$('.raway'));
  assert.match(badge(a), /mood-thinking/);
  a.app.gone(401);
  const f = await open('final');
  assert.match(badge(f), /mood-finished/);
  f.app.gone(401);
});

test('near miss: Needs you box; Accept sends judge yes with q and player, Not right sends no', async () => {
  const t = await open('miss');
  const s = t.text();
  for (const l of ['Needs you · 1 near miss', 'Priya (Table Four) wrote "nineteen ninety six"', "Answer: 1996 · also accepted: '96", 'Accept +100', 'Not right', "If you don't answer before the reveal, Quizmaster's call stands."]) assert.ok(s.includes(l), `"${l}" in ${s}`);
  await t.click('[data-k="yes-3-p4"]');
  assert.deepEqual(t.writes(), [['host/action', { name: 'judge', body: { q: 3, player: 'p4', verdict: 'yes' } }]]);
  assert.equal(t.$('.rmiss'), null);
  assert.ok(t.text().includes("Accepted Priya's answer."));
  const u = await open('miss');
  await u.click('[data-k="no-3-p4"]');
  assert.deepEqual(u.writes(), [['host/action', { name: 'judge', body: { q: 3, player: 'p4', verdict: 'no' } }]]);
  t.app.gone(401);
  u.app.gone(401);
});

test('near miss: a new one arrives over the socket while the host watches', async () => {
  const t = await open('question');
  assert.equal(t.$('.rmiss'), null);
  t.push((b) => b.pending.push({ q: 3, player: 'p1', question: 'x', answer: '1996', accept: ['1996'], guess: '96' }));
  await flush();
  assert.ok(t.$('.rmiss'));
  t.app.gone(401);
});

// ---------- Teams and Players ----------

test('teams: ± sends adjust {team, points: ±100}; Undo sends the opposite; the tab keeps focus on the button', async () => {
  const t = await open('question');
  await tab(t, 'teams');
  assert.equal(t.$$('.rlist li').length, 6);
  assert.ok(t.text().includes('Quizzly Bears1,960'));
  const plus = t.$('[data-k="plus-Fact Hunters"]');
  plus.focus();
  await t.click(plus);
  assert.deepEqual(t.writes(), [['host/action', { name: 'adjust', body: { team: 'Fact Hunters', points: 100 } }]]);
  assert.ok(t.text().includes('Fact Hunters2,020'));
  assert.equal(document.activeElement.getAttribute('data-k'), 'plus-Fact Hunters');
  assert.ok(t.text().includes('Last change: +100 to Fact Hunters.'));
  await t.click('[data-k="undo"]');
  assert.deepEqual(t.writes().at(-1), ['host/action', { name: 'adjust', body: { team: 'Fact Hunters', points: -100 } }]);
  assert.ok(t.text().includes('Fact Hunters1,920'));
  assert.equal(t.$('[data-k="undo"]'), null);
  t.app.gone(401);
});

test('teams: a solo quiz names the tab Scores and adjusts players by id', async () => {
  const t = await open('solo');
  assert.equal(t.$('[data-k="tab-teams"]').textContent, 'Scores');
  await tab(t, 'teams');
  await t.click('[data-k="minus-p2"]');
  assert.deepEqual(t.writes(), [['host/action', { name: 'adjust', body: { player: 'p2', points: -100 } }]]);
  t.app.gone(401);
});

test('players: status per player; Remove asks first, Cancel sends nothing, Remove sends host/remove {player_id}', async () => {
  const t = await open('question');
  await tab(t, 'players');
  assert.equal(t.$$('.rlist li').length, 23);
  const st = Object.fromEntries(t.$$('.rlist li').map((li) => [li.querySelector('.who b').textContent, li.querySelector('.st').textContent]));
  assert.equal(st.Mia, 'answered');
  assert.equal(st.Hugo, '—');
  assert.equal(st.Nia, 'asleep');
  const more = t.$('[data-k="more-p9"]');
  more.focus();
  await t.click(more);
  const sheet = t.$('[role="dialog"]');
  assert.equal(sheet.getAttribute('aria-modal'), 'true');
  assert.ok(sheet.textContent.includes('Remove Omar?'));
  assert.equal(document.activeElement.getAttribute('data-k'), 'sheet-remove');
  assert.equal(t.$('[data-k="sheet-rename"]').getAttribute('aria-disabled'), 'true');
  assert.deepEqual(t.writes(), []);
  await t.click('[data-k="sheet-cancel"]');
  assert.equal(t.$('[role="dialog"]'), null);
  assert.equal(document.activeElement.getAttribute('data-k'), 'more-p9', 'focus back on the opener');
  assert.deepEqual(t.writes(), []);
  await t.click('[data-k="more-p9"]');
  await t.click('[data-k="sheet-remove"]');
  assert.deepEqual(t.writes(), [['host/remove', { player_id: 'p9' }]]);
  assert.equal(t.$('[role="dialog"]'), null);
  assert.ok(t.text().includes('Omar was removed.'));
  const omar = t.$$('.rlist li').find((li) => li.textContent.includes('Omar'));
  assert.equal(omar.querySelector('.st').textContent, 'removed');
  assert.equal(omar.querySelector('[data-k^="more-"]'), null);
  t.app.gone(401);
});

test('sheets: Escape closes and gives focus back; Tab stays inside the sheet', async () => {
  const t = await open('question');
  await t.click('[data-k="end"]');
  assert.ok(t.$('[data-sheet="end"]'));
  const items = t.$$('.rsheet button:not([disabled])');
  items.at(-1).focus();
  t.root.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }));
  assert.equal(document.activeElement, t.$$('.rsheet button')[0]);
  t.root.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await flush();
  assert.equal(t.$('[role="dialog"]'), null);
  assert.equal(document.activeElement.getAttribute('data-k'), 'end');
  t.app.gone(401);
});

test('end: asks first and names the leader; Keep playing sends nothing; End and show winners sends host/end', async () => {
  const t = await open('question');
  await t.click('[data-k="end"]');
  const s = t.$('[data-sheet="end"]').textContent;
  assert.ok(s.includes('End the quiz now?'));
  assert.ok(s.includes('Quizzly Bears win with 1,960'), s);
  await t.click('[data-k="sheet-keep"]');
  assert.deepEqual(t.writes(), []);
  await t.click('[data-k="end"]');
  await t.click('[data-k="sheet-end"]');
  assert.deepEqual(t.writes(), [['host/end', {}]]);
  assert.equal(t.$('.rchip').textContent, 'Ended');
  assert.ok(t.text().includes('The quiz has ended'));
  assert.equal(t.$('[data-k="next"]'), null);
  assert.equal(t.$('[data-k="end"]'), null);
  t.app.gone(401);
});

test('tabs: arrow keys move between Game, Teams and Players; aria-selected and tabindex follow', async () => {
  const t = await open('question');
  t.$('[data-k="tab-game"]').focus();
  t.$('[data-k="tab-game"]').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
  assert.equal(document.activeElement.getAttribute('data-k'), 'tab-teams');
  assert.equal(t.$('[data-k="tab-teams"]').getAttribute('aria-selected'), 'true');
  assert.equal(t.$('[data-k="tab-game"]').getAttribute('tabindex'), '-1');
  document.activeElement.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true }));
  document.activeElement.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true }));
  assert.equal(document.activeElement.getAttribute('data-k'), 'tab-players');
  t.app.gone(401);
});

test('away: buttons wait (aria-disabled), nothing is written, and the remote comes back by itself', async () => {
  const t = await open('question', { ws: false });
  t.box.down = true;
  await t.advance(3000);
  assert.ok(t.$('.raway'));
  assert.equal(t.$('[data-k="next"]').getAttribute('aria-disabled'), 'true');
  const before = t.writes().length;
  await t.click('[data-k="next"]');
  assert.equal(t.writes().length, before);
  t.box.down = false;
  await t.advance(30_000);
  assert.equal(t.$('.raway'), null);
  t.app.gone(401);
});

test('errors: a refused write shows the box words in the status line and keeps the remote', async () => {
  const t = await open('question');
  await tab(t, 'teams');
  const real = t.box.handle.bind(t.box);
  t.box.handle = (p, b, o) => (p === 'host/action' ? { status: 409, body: { error: 'not_now', message: "That can't be done right now." } } : real(p, b, o));
  await t.click('[data-k="plus-Table Four"]');
  assert.equal(t.$('.rnote').textContent, "That can't be done right now.");
  assert.ok(t.$('.rnote').classList.contains('bad'));
  assert.equal(t.screen(), 'live');
  await t.advance(5000);
  assert.equal(t.$('.rnote').textContent, '');
  t.app.gone(401);
});

// ---------- source rules ----------

test('source: the remote is plain TS + DOM (no React, no lucide), sets no inline style, and has no hex colour', () => {
  for (const f of ['app.ts', 'view.ts', 'link.ts', 'icons.ts', 'remote.ts']) {
    const s = readFileSync(new URL(`../guest/src/remote/${f}`, import.meta.url), 'utf8');
    assert.doesNotMatch(s, /from ['"](react|lucide-react|preact)/, f);
    assert.doesNotMatch(s, /innerHTML|\.style\s*=|setAttribute\(['"]style/, f);
    assert.doesNotMatch(s, /fetch\(['"]https?:/, f);
  }
  const css = readFileSync(new URL('../guest/src/remote/remote.css', import.meta.url), 'utf8');
  assert.doesNotMatch(css, /#[0-9a-fA-F]{3,8}\b/);
  assert.doesNotMatch(css, /holo/, 'no holo on the remote');
  const main = readFileSync(new URL('../guest/src/main.ts', import.meta.url), 'utf8');
  assert.match(main, /await import\('\.\/remote\/remote'\)/, 'the remote is a lazy chunk');
});
