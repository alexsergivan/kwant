// pa10a Secret Santa guest page (autonomy/pocket-apps/pa10a/guest-brief.md): the view mapping from Lin's santa API,
// the screen for each phase, the PIN boxes and their copy, and the G1-G11 flows against the fake santa box
// (guest/fake/santa.mjs). The browser half (G1-G11 in Chrome under pa1's CSP, shots) is guest/e2e/csp.mjs.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';
import { SantaBox, santaScenarios, santaEnvelope, secretMatch, people } from '../guest/fake/santa.mjs';

const dom = new JSDOM('<!doctype html><html><body><div id="app"></div></body></html>', { url: 'http://localhost/p/santa-join', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'HTMLButtonElement', 'HTMLTextAreaElement', 'Element', 'Node', 'Event', 'MouseEvent', 'SVGElement']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { santaFrom, screenOf, kitOf } from './guest/src/santa/view';
  export { PinInput, pinCopy, clean } from './guest/src/santa/pin';
  export { SantaApp, daysLeft, shortDay } from './guest/src/santa/app';
`);
const settle = () => new Promise((r) => setTimeout(r, 0));
const flush = async () => {
  for (let i = 0; i < 8; i++) await settle();
};

/** A v1 state for the mapping tests. */
const env1 = (view, { me = { id: 'p1', name: 'Mia', avatar: 'visor', notices: [] }, kit = 'santa', ended = false } = {}) => {
  const e = santaEnvelope({}, { code: 't', version: 3, me, view, ended });
  e.pocket.kit = kit;
  return e;
};
const facts = santaScenarios['santa-join'].info;
const seatView = (over = {}) => ({ ...facts, people, my_wish: null, in_draw: true, can_open: false, opened: false, match: null, gift_ready: false, notices: [], asks: { sent: [], received: [], left_today: 10, max_chars: 200, open: false }, my_santa: null, reveal_loop: null, ...over });
const loop = [
  { giver: people[0], receiver: people[2] },
  { giver: people[2], receiver: people[0] },
];

// ---------- view mapping ----------

test('view: only a santa pocket with valid facts maps; junk is dropped', () => {
  assert.equal(m.santaFrom(env1(seatView(), { kit: 'quiz' })), undefined);
  assert.equal(m.santaFrom(env1({ ...seatView(), phase: 'party' })), undefined);
  assert.equal(m.santaFrom(null), undefined);
  assert.equal(m.santaFrom('x'), undefined);
  assert.equal(m.kitOf(env1(seatView())), 'santa');
  const s = m.santaFrom(env1(seatView({ people: [people[0], { id: 'x' }, 7, null, { name: 'no id' }], joined: -2, expected: 'eight', budget: '   ' })));
  assert.deepEqual(s.view.people.map((p) => p.name), ['Mia']);
  assert.equal(s.view.facts.joined, undefined);
  assert.equal(s.view.facts.expected, undefined);
  assert.equal(s.view.facts.budget, undefined);
  assert.equal(s.version, 3);
  assert.equal(s.view.keep_days, 7);
  assert.equal(s.view.expires_at, '2026-12-25T18:00:00Z');
});

test('view: each phase gives its screen', () => {
  const at = (view, o, opts) => m.screenOf(m.santaFrom(env1(view, opts)).view, o);
  // No seat
  assert.equal(at(facts, {}, { me: null }), 'join');
  assert.equal(at(facts, { rejoin: true }, { me: null }), 'rejoin');
  assert.equal(at({ ...facts, phase: 'drawn' }, {}, { me: null }), 'late');
  assert.equal(at({ ...facts, phase: 'drawn' }, { joinLate: true }, { me: null }), 'join');
  assert.equal(at({ ...facts, phase: 'revealed' }, {}, { me: null }), 'ended');
  assert.equal(at({ ...facts, phase: 'revealed' }, {}, { me: null, ended: true }), 'ended');
  // Seated
  assert.equal(at(seatView()), 'waiting');
  assert.equal(at(seatView({ phase: 'drawn', in_draw: false })), 'waiting');
  assert.equal(at(seatView({ phase: 'drawn', can_open: true })), 'unwrap');
  assert.equal(at(seatView({ phase: 'drawn', opened: true, match: secretMatch })), 'unwrap', 'match on a device that has not unwrapped: the box first');
  assert.equal(at(seatView({ phase: 'drawn', opened: true, match: secretMatch }), { unwrapped: true }), 'match');
  assert.equal(at(seatView({ phase: 'revealed', match: secretMatch, my_santa: people[3], reveal_loop: loop }), { unwrapped: true }), 'reveal');
  assert.equal(at(seatView({ phase: 'drawn' }), {}, { ended: true }), 'ended');
});

test('view: match is null until opened; match_changed closes it', () => {
  const before = m.santaFrom(env1(seatView({ phase: 'drawn', can_open: true }))).view;
  assert.equal(before.match, null);
  const changed = m.santaFrom(env1(seatView({ phase: 'drawn', can_open: true, opened: true, match: secretMatch, notices: [{ kind: 'match_changed', at: 'x' }] }))).view;
  assert.equal(changed.match_changed, true);
  assert.equal(m.screenOf(changed, { unwrapped: true }), 'unwrap');
  // Without a seat a match is never read, whatever the box sends.
  assert.equal(m.santaFrom(env1(seatView({ phase: 'drawn', match: secretMatch }), { me: null })).view.match, null);
});

test('view: the reveal only with the server reveal_loop, and only once revealed', () => {
  const noLoop = m.santaFrom(env1(seatView({ phase: 'revealed', my_santa: people[3], reveal_loop: null }))).view;
  assert.equal(noLoop.reveal_loop, null);
  assert.notEqual(m.screenOf(noLoop), 'reveal');
  const early = m.santaFrom(env1(seatView({ phase: 'drawn', opened: true, match: secretMatch, my_santa: people[3], reveal_loop: loop }))).view;
  assert.equal(early.reveal_loop, null);
  assert.equal(early.my_santa, null);
  const junk = m.santaFrom(env1(seatView({ phase: 'revealed', reveal_loop: [{ giver: 1 }, 'x', { giver: people[0] }] }))).view;
  assert.equal(junk.reveal_loop, null);
  const ok = m.santaFrom(env1(seatView({ phase: 'revealed', my_santa: people[3], reveal_loop: loop }))).view;
  assert.deepEqual(ok.reveal_loop.map((l) => `${l.giver.name}>${l.receiver.name}`), ['Mia>Jonas', 'Jonas>Mia']);
  assert.equal(ok.my_santa.name, 'Priya');
});

test('view: asks keep the new flags; questions never carry who asked', () => {
  const v = m.santaFrom(env1(seatView({ phase: 'drawn', opened: true, match: secretMatch, asks: santaScenarios['santa-unwrap'].open.asks }))).view;
  assert.equal(v.asks.open, true);
  assert.equal(v.asks.left_today, 9);
  assert.equal(v.asks.sent[0].new, true, 'new_answer');
  assert.equal(v.asks.received[0].new, true);
  assert.deepEqual(Object.keys(v.asks.received[0]).sort(), ['a', 'answered_at', 'at', 'id', 'new', 'q']);
});

// ---------- PIN ----------

test('pin: digits only, at most 4, shown as dots never digits', () => {
  assert.equal(m.clean('12a3 45'), '1234');
  const seen = [];
  const p = new m.PinInput('pin', 'Pick a 4-digit PIN', (v) => seen.push(v));
  document.body.append(p.el);
  assert.equal(p.input.getAttribute('inputmode'), 'numeric');
  assert.equal(p.input.getAttribute('autocomplete'), 'off');
  assert.equal(p.input.getAttribute('maxlength'), '4');
  assert.equal(p.el.querySelectorAll('.pin').length, 4);
  assert.equal(p.el.querySelector('.pins').getAttribute('aria-hidden'), 'true');
  p.input.value = '9x87';
  p.input.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
  assert.equal(p.value, '987');
  assert.deepEqual(seen, ['987']);
  const boxes = [...p.el.querySelectorAll('.pin')].map((b) => b.textContent);
  assert.deepEqual(boxes, ['•', '•', '•', '']);
  assert.doesNotMatch(p.el.querySelector('.pins').textContent, /\d/);
  p.el.remove();
});

test('pin: wrong-PIN copy counts down from the server tries_left', () => {
  const c = (r) => m.pinCopy({ ok: false, status: 401, ...r }, 'Dana', false);
  assert.equal(c({ error: 'wrong_pin', tries_left: 3 }).sub, '3 more tries, then a 10-minute wait. Check the name is spelled the way you joined.');
  assert.equal(c({ error: 'wrong_pin', tries_left: 1 }).sub, '1 more try, then a 10-minute wait. Check the name is spelled the way you joined.');
  assert.equal(c({ error: 'wrong_pin', tries_left: 0 }).sub, 'Next comes a 10-minute wait. Check the name is spelled the way you joined.');
  assert.equal(c({ error: 'wrong_pin' }).title, "That name and PIN don't match.");
  assert.match(c({ error: 'locked', status: 423 }).sub, /ask Dana for a reset code/);
  assert.equal(c({ error: 'hard_locked', status: 423 }).title, 'This name is locked.');
  assert.equal(c({ error: 'reset_pending', status: 409 }).title, 'Dana made you a reset code.');
  assert.equal(c({ error: 'whatever', status: 400, message: 'Plain server words.' }).title, 'Plain server words.');
  assert.match(m.pinCopy({ ok: false, status: 409, error: 'name_taken' }, 'Dana', true).sub, /I've joined before/);
});

test('days left: from gift_on on this device', () => {
  const now = Date.UTC(2026, 11, 4, 12); // Dec 4 noon
  assert.equal(m.daysLeft('2026-12-18', now), 14);
  assert.equal(m.daysLeft('2026-12-04', now), 0);
  assert.equal(m.daysLeft('2026-12-01T23:59:59Z', now), -3);
  assert.equal(m.daysLeft('junk', now), undefined);
  assert.equal(m.shortDay('2026-12-18'), 'Dec 18');
});

// ---------- flows against the fake box ----------

function harness(name, { store = new Map(), boxOpts = {} } = {}) {
  const calls = [];
  let now = Date.UTC(2026, 11, 4, 12);
  let timers = [];
  let nextId = 1;
  const box = new SantaBox(name, { code: 't', ...boxOpts });
  box.up = true;
  const fetch = async (url, init = {}) => {
    const path = url.split('/api/pocket/t/')[1] ?? '';
    const body = init.body ? JSON.parse(init.body) : undefined;
    calls.push({ path, body });
    if (!box.up) throw new TypeError('Failed to fetch');
    const r = box.handle(path, body ?? {});
    return { ok: r.status < 400, status: r.status, json: async () => (r.body === null ? Promise.reject(new SyntaxError('empty')) : r.body) };
  };
  const env = {
    fetch,
    WebSocket: undefined,
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
    store: { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v) },
  };
  const root = document.getElementById('app');
  root.replaceChildren();
  document.querySelectorAll('.toast').forEach((t) => t.remove());
  const app = new m.SantaApp(root, env, 't');
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
  const $ = (s) => root.querySelector(s);
  return { app, root, box, calls, store, advance, $, screen: () => $('[data-screen]')?.getAttribute('data-screen'), text: () => root.textContent.replace(/\s+/g, ' '), actions: () => calls.filter((c) => c.path === 'action').map((c) => c.body.name) };
}
async function open(name, opts) {
  const t = harness(name, opts);
  await t.app.start(null);
  await flush();
  return t;
}
const click = async (el) => {
  assert.ok(el, 'element to click');
  el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await flush();
};
const type = async (el, v) => {
  assert.ok(el, 'element to type in');
  el.value = v;
  el.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
  await flush();
};
const submit = async (form) => {
  form.dispatchEvent(new dom.window.Event('submit', { bubbles: true, cancelable: true }));
  await flush();
};

test('G1 join: keep line first, PIN needed, then G2 waiting with the wish saved', async () => {
  const t = await open('santa-join');
  assert.equal(t.screen(), 'join');
  const form = t.$('form');
  assert.ok(form.firstElementChild.matches('[data-keep]'), 'keep line first under the bar');
  assert.match(t.text(), /Kept on Dana's computer until .*Dec 25.* \(7 days after the reveal\), then deleted\./);
  assert.match(t.text(), /Office Secret Santa/);
  assert.match(t.text(), /5 of 8 joined/);
  assert.equal(t.$('[data-join]').disabled, true);
  await type(t.$('#s-name'), 'Ana');
  await type(t.$('#s-pin'), '12');
  assert.equal(t.$('[data-join]').disabled, true, 'two digits are not a PIN');
  await type(t.$('#s-pin'), '1234');
  await type(t.$('#s-wish'), 'Tea');
  assert.equal(t.$('[data-join]').disabled, false);
  await submit(t.$('form'));
  assert.equal(t.screen(), 'waiting');
  assert.match(t.text(), /You're in, Ana!/);
  const join = t.calls.find((c) => c.path === 'join');
  assert.deepEqual(Object.keys(join.body).sort(), ['avatar', 'name', 'pin']);
  assert.deepEqual(t.actions(), ['set_wish']);
  assert.equal(t.box.seat.view.my_wish, 'Tea');
  assert.match(t.text(), /Edit my wish list/);
  t.app.stop();
});

test('G3/G4 rejoin: a wrong PIN says the server tries left and empties the boxes', async () => {
  const t = await open('santa-wrongpin');
  await click(t.$('[data-to-rejoin]'));
  assert.equal(t.screen(), 'rejoin');
  assert.match(t.text(), /Welcome back/);
  await type(t.$('#s-bname'), 'Mia');
  await type(t.$('#s-bpin'), '0000');
  await submit(t.$('form'));
  assert.equal(t.screen(), 'rejoin');
  assert.match(t.$('[data-problem]').textContent, /That name and PIN don't match\..*3 more tries, then a 10-minute wait\./);
  assert.equal(t.$('#s-bpin').value, '');
  assert.equal(t.$('#s-bpin').getAttribute('aria-invalid'), 'true');
  // Reset-code mode
  await click(t.$('[data-use-code]'));
  assert.ok(t.$('#s-code') && t.$('#s-newpin'));
  t.app.stop();
});

test('G3 rejoin with the right PIN opens the page; the box shows first on a new device', async () => {
  const t = await open('santa-join');
  await click(t.$('[data-to-rejoin]'));
  await type(t.$('#s-bname'), 'Mia');
  await type(t.$('#s-bpin'), '1234');
  await submit(t.$('form'));
  assert.equal(t.screen(), 'unwrap', 'opened elsewhere: this device unwraps once');
  await click(t.$('[data-open]'));
  await t.advance(250);
  assert.equal(t.screen(), 'match');
  assert.deepEqual(t.actions().filter((a) => a === 'open_match'), [], 'the match was already in the view');
  t.app.stop();
});

test('G5/G6 unwrap: Open it asks once, the lid lifts, the device remembers', async () => {
  const t = await open('santa-unwrap');
  assert.equal(t.screen(), 'unwrap');
  assert.doesNotMatch(t.text(), /Jonas/, 'no match before Open it');
  await click(t.$('[data-open]'));
  assert.ok(t.$('.giftbox.open'), 'lid lifting');
  assert.equal(t.screen(), 'unwrap');
  await t.advance(250);
  assert.equal(t.screen(), 'match');
  assert.ok(t.$('.matchcard.pop'));
  assert.match(t.text(), /You're buying for.*Jonas/);
  assert.match(t.text(), /A good paperback, any thriller/);
  assert.match(t.text(), /Days left\s*14/);
  assert.equal(t.store.get('kp-santa-open:t:p1'), '1');
  assert.equal(t.actions().filter((a) => a === 'open_match').length, 1);
  // A reload on this device goes straight to the match.
  const again = await open('santa-unwrap', { store: t.store });
  again.box.seat = t.box.seat;
  await again.app.start(null);
  await flush();
  assert.equal(again.screen(), 'match');
  assert.ok(!again.$('.matchcard.pop'), 'no pop on reload');
  again.app.stop();
  t.app.stop();
});

test('G5 match_changed: closed until Open it again', async () => {
  const t = await open('santa-changed', { store: new Map([['kp-santa-open:t:p1', '1']]) });
  assert.equal(t.screen(), 'unwrap');
  assert.ok(t.$('[data-changed]'));
  assert.equal(t.actions().length, 0, 'never reopened by itself');
  assert.match(t.$('[data-open]').textContent, /Open it again/);
  await click(t.$('[data-open]'));
  await t.advance(250);
  assert.equal(t.screen(), 'match');
  assert.match(t.text(), /Priya/);
  t.app.stop();
});

test('G6 gift ready toggles with aria-pressed', async () => {
  const t = await open('santa-unwrap', { store: new Map([['kp-santa-open:t:p1', '1']]) });
  await flush();
  assert.equal(t.screen(), 'match');
  const b = t.$('[data-gift]');
  assert.equal(b.getAttribute('aria-pressed'), 'false');
  await click(b);
  assert.equal(t.$('[data-gift]').getAttribute('aria-pressed'), 'true');
  assert.equal(t.box.seat.view.gift_ready, true);
  t.app.stop();
});

test('G7 Santa asks: chips, count, ink New, seen_asks, never who asked', async () => {
  const t = await open('santa-unwrap', { store: new Map([['kp-santa-open:t:p1', '1']]) });
  await flush();
  assert.equal(t.screen(), 'match');
  assert.equal(t.root.querySelectorAll('[data-chip]').length, 3);
  assert.ok(t.$('[data-jump]'), 'jump notice for a new question');
  assert.ok(t.root.querySelectorAll('[data-sticker="ink"]').length >= 1);
  assert.equal(t.root.querySelectorAll('.kw-sticker-holo, [data-sticker="holo"]').length, 0, 'no holo on guest screens');
  assert.ok(t.actions().includes('seen_asks'));
  // Seen on the box, still marked New on this page load.
  assert.equal(t.box.seat.view.asks.received[0].new, false);
  assert.ok(t.$('[data-sticker="ink"]'));
  assert.match(t.text(), /Santa asks/);
  assert.doesNotMatch(t.$('[data-received]').textContent, /Mia|Sam|Priya|Leo/);
  await click(t.root.querySelector('[data-chip]'));
  assert.equal(t.$('#s-ask').value, 'Shoe size?');
  assert.match(t.$('#ask-count').textContent, /10 of 200 · 9 questions left today/);
  await submit(t.$('[data-ask-form]'));
  assert.equal(t.box.seat.view.asks.sent.at(-1).q, 'Shoe size?');
  assert.match(t.$('[data-sent]').textContent, /Waiting for Jonas's answer/);
  assert.equal(t.$('#s-ask').value, '');
  // Answer the Santa's question.
  await type(t.$('[data-k="reply-r1"]'), 'Yes, very dark.');
  await submit(t.$('form.reply'));
  assert.match(t.$('[data-received]').textContent, /You: Yes, very dark\./);
  t.app.stop();
});

test('G7 Santa asks: the server message is shown as is; none left today disables sending', async () => {
  const t = await open('santa-unwrap', { store: new Map([['kp-santa-open:t:p1', '1']]) });
  await flush();
  t.box.seat.view.asks.left_today = 0;
  t.box.seat.view.asks = { ...t.box.seat.view.asks };
  await type(t.$('#s-ask'), 'One more?');
  await submit(t.$('[data-ask-form]'));
  assert.equal(t.$('#ask-err').textContent, "That's 10 for today. You can ask again tomorrow.");
  assert.equal(t.$('#s-ask').value, 'One more?', 'typed text kept');
  t.app.stop();
});

test('G8 reveal: the server loop in its order, my two links marked', async () => {
  const t = await open('santa-reveal');
  assert.equal(t.screen(), 'reveal');
  const links = [...t.root.querySelectorAll('ol.chain li')];
  assert.deepEqual(links.map((l) => l.getAttribute('aria-label')), ['Mia gave to Jonas', 'Jonas gave to Sam', 'Sam gave to Leo', 'Leo gave to Priya', 'Priya gave to Mia']);
  assert.deepEqual(links.map((l) => l.classList.contains('me')), [true, false, false, false, true]);
  assert.match(t.text(), /Your Santa was.*Priya/);
  t.app.stop();
});

test('G9 away keeps typed text; G2 comes back', async () => {
  const t = await open('santa-waiting');
  assert.equal(t.screen(), 'waiting');
  await click(t.$('[data-edit-wish]'));
  await type(t.$('#s-mywish'), 'Warm socks and tea');
  t.box.up = false;
  t.app.wake();
  await t.advance(3000);
  assert.equal(t.screen(), 'away');
  assert.match(t.text(), /Dana's computer is asleep/);
  t.box.up = true;
  await t.advance(60_000);
  assert.equal(t.screen(), 'waiting');
  assert.equal(t.$('#s-mywish').value, 'Warm socks and tea');
  t.app.stop();
});

test('G10 late: drawn before typing, and 409 not_now with the server reason', async () => {
  const t = await open('santa-late');
  assert.equal(t.screen(), 'late');
  assert.ok(t.$('[data-to-rejoin]'));
  assert.match(t.$('[data-to-rejoin]').textContent, /I've joined before: open my page/);
  await click(t.$('[data-join-late]'));
  assert.equal(t.screen(), 'join');
  await type(t.$('#s-name'), 'Zoe');
  await type(t.$('#s-pin'), '4321');
  await submit(t.$('form'));
  assert.equal(t.screen(), 'late');
  assert.match(t.text(), /The names are already drawn\. Ask Dana to let you in\./);
  t.app.stop();
});

test('G11 ended', async () => {
  const t = await open('santa-ended');
  assert.equal(t.screen(), 'ended');
  assert.match(t.text(), /This Secret Santa has ended/);
  assert.match(t.text(), /Names, wish lists and questions are deleted on/);
  assert.equal(t.$('[data-to-rejoin]'), null);
  t.app.stop();
});

test('notices: a wrong-PIN try on this seat shows once and is cleared', async () => {
  const t = harness('santa-waiting');
  t.box.seat.me.notices = [{ kind: 'wrong_pin', at: '2026-12-03T10:00:00Z' }];
  await t.app.start(null);
  await flush();
  assert.match(document.querySelector('.toast').textContent, /wrong PIN/);
  assert.ok(t.calls.some((c) => c.path === 'notices/seen'));
  assert.equal(t.box.seat.me.notices.length, 0);
  t.app.stop();
});

test('css: lid and card motion as specified, static under reduced motion; sticker tokens match modern.css', () => {
  const css = readFileSync(new URL('../guest/src/guest.css', import.meta.url), 'utf8');
  assert.match(css, /\.giftbox\.open \.lid\s*\{[^}]*transform:\s*translateY\(-6px\) rotate\(-8deg\)[^}]*opacity:\s*0/);
  assert.match(css, /\.lid\s*\{[^}]*180ms/);
  assert.match(css, /@keyframes sa-pop\s*\{\s*from\s*\{[^}]*scale\(0?\.96\)/);
  assert.match(css, /\.pop\s*\{[^}]*160ms/);
  const reduced = css.slice(css.lastIndexOf('prefers-reduced-motion'));
  assert.match(reduced, /\.lid/);
  assert.match(reduced, /\.pop/);
  const modern = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
  const v = (k) => css.match(new RegExp(`--${k}:\\s*([^;]+);`))[1].trim();
  const sticker = modern.slice(modern.indexOf('.kw-sticker {'), modern.indexOf('}', modern.indexOf('.kw-sticker {')));
  assert.equal(v('sticker-edge'), sticker.match(/border: 2px solid (#[0-9a-f]+)/)[1]);
  assert.equal(v('sticker-text'), sticker.match(/color: (#[0-9a-f]+)/)[1]);
  assert.equal(v('sticker-ink'), modern.match(/\.kw-sticker-ink \{ background: (#[0-9a-f]+); \}/)[1]);
});
