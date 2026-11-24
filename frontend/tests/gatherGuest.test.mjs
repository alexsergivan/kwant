// pa10b gather guest page (autonomy/pocket-apps/pa10b/guest-brief.md): the view mapping from Lin's gather API
// (names-so-far, guest-impl.md), the screen for each face and phase, and the G1-G11 flows against the fake gather box
// (guest/fake/gather.mjs). The browser half (G1-G11 in Chrome under pa1's CSP, 390 px shots) is guest/e2e/csp.mjs.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';
import { GatherBox, gatherScenarios, gatherEnvelope } from '../guest/fake/gather.mjs';

const dom = new JSDOM('<!doctype html><html><body><div id="app"></div></body></html>', { url: 'http://localhost/p/gather-rsvp', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'HTMLButtonElement', 'HTMLTextAreaElement', 'Element', 'Node', 'Event', 'MouseEvent', 'SVGElement']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { gatherFrom, screenOf, whoSees, nth, move, FOOD_LABELS } from './guest/src/gather/view';
  export { GatherApp, avatarFor } from './guest/src/gather/app';
`);
const settle = () => new Promise((r) => setTimeout(r, 0));
const flush = async () => {
  for (let i = 0; i < 8; i++) await settle();
};

const MIA = { id: 'p1', name: 'Mia', avatar: 'visor', notices: [] };
/** A v1 state for the mapping tests. */
const env1 = (view, { me = MIA, kit = 'gather', ended = false } = {}) => {
  const e = gatherEnvelope({ code: 't', version: 3, me: me ?? undefined, view, ended });
  e.pocket.kit = kit;
  return e;
};
const S = gatherScenarios;
const viewOf = (name) => (S[name].state ?? S[name].join).view;
const map = (view, o) => m.gatherFrom(env1(view, o))?.view;

// ---------- view mapping ----------

test('view: only a gather pocket with a known face and phase maps; junk is dropped', () => {
  const v = viewOf('gather-rsvp-in');
  assert.equal(m.gatherFrom(env1(v, { kit: 'santa' })), undefined);
  assert.equal(m.gatherFrom(env1({ ...v, face: 'party' })), undefined);
  assert.equal(m.gatherFrom(env1({ ...v, phase: 'later' })), undefined);
  assert.equal(m.gatherFrom(null), undefined);
  const x = map({ ...v, joined: -1, price: '  ', food_choices: ['vegan', 'pizza', 3], people: [v.people[0], { id: 'x' }, 7, { ...v.people[1], answer: 'perhaps' }] });
  assert.equal(x.facts.joined, undefined);
  assert.equal(x.facts.price, undefined);
  assert.deepEqual(x.facts.food_choices, ['vegan']);
  assert.deepEqual(x.people.map((p) => p.name), ['Sam']);
  assert.equal(m.gatherFrom(env1(v)).version, 3);
  assert.equal(x.keep_days, 7);
});

test('view: fields the host did not choose stay absent, whatever the box sends', () => {
  const v = viewOf('gather-rsvp-in');
  const x = map({ ...v, fields: ['name', 'answer'] });
  assert.deepEqual(x.facts.food_choices, []);
  assert.equal(x.facts.max_plus_ones, 0);
  assert.equal(x.mine.plus_ones, undefined);
  assert.deepEqual(x.mine.food, []);
  assert.equal(x.mine.answer, 'yes');
});

test('view: names only with show_names === true (RSVP people, slot people and waiting people)', () => {
  assert.equal(map({ ...viewOf('gather-rsvp-in'), show_names: 'yes' }).people, null);
  const s = map({ ...viewOf('gather-waiting'), show_names: false });
  assert.ok(s.facts.slots.every((x) => x.people === null && x.waiting_people === null));
  assert.ok(map(viewOf('gather-waiting')).facts.slots[2].waiting_people.length === 2);
});

test('view: paid only on an RSVP with a paid.at; live only seated and open', () => {
  assert.deepEqual(map(viewOf('gather-rsvp-in')).mine.paid, { at: '2026-12-04T09:00:00Z' });
  assert.equal(map({ ...viewOf('gather-rsvp-in'), me: { ...viewOf('gather-rsvp-in').me, paid: { at: 5 } } }).mine.paid, undefined);
  assert.equal(map({ ...viewOf('gather-rsvp-in'), me: { ...viewOf('gather-rsvp-in').me, paid: true } }).mine.paid, undefined);
  assert.equal(map(viewOf('gather-voted')).live.length, 4);
  assert.equal(map(viewOf('gather-voted'), { me: null }).live, null);
  assert.equal(map({ ...viewOf('gather-voted'), phase: 'closed' }).live, null);
});

test('view: waiting position, moved-up and ranked order are read, unknown slots and options dropped', () => {
  const w = map(viewOf('gather-waiting'));
  assert.deepEqual(w.mine.waiting, { slot: 2, position: 1 });
  assert.equal(map({ ...viewOf('gather-waiting'), me: { waiting: { slot: 9, position: 1 } } }).mine.waiting, undefined);
  assert.equal(map({ ...viewOf('gather-waiting'), me: { waiting: { slot: 2, position: 0 } } }).mine.waiting, undefined);
  const mv = map(viewOf('gather-moved'));
  assert.deepEqual(mv.mine.moved_up, { slot: 2, at: '2026-12-05T12:00:00Z', new: true });
  const v = map({ ...viewOf('gather-voted'), me: { voted: true, ballot: [2, 99, 'x', 0] } });
  assert.deepEqual(v.mine.ballot, [2, 0]);
  const r = map(viewOf('gather-result')).result;
  assert.equal(r.outcome, 'winner');
  assert.equal(r.rounds.length, 3);
  assert.deepEqual(r.rounds[0].dropped, [{ option: 3, name: 'Pizza Nonna' }]);
  assert.equal(map({ ...viewOf('gather-result'), result: { outcome: 'winner', winner: null } }).result, null, 'a winner with no winner is junk');
});

test('view: each face and phase gives its screen', () => {
  const at = (name, o = {}, opts) => m.screenOf(map((S[name].state ?? S[name].join ?? S[name]).view ?? S[name].info, opts), o);
  assert.equal(m.screenOf(map(S['gather-rsvp'].info, { me: null })), 'join');
  assert.equal(m.screenOf(map(S['gather-rsvp'].info, { me: null }), { rejoin: true }), 'rejoin');
  assert.equal(at('gather-rsvp'), 'join', 'seated with no answer yet');
  assert.equal(at('gather-rsvp-in'), 'in');
  assert.equal(at('gather-rsvp-in', { edit: true }), 'join');
  assert.equal(at('gather-signup'), 'join');
  assert.equal(at('gather-signup-in'), 'in');
  assert.equal(at('gather-waiting'), 'waiting');
  assert.equal(at('gather-moved'), 'moved');
  assert.equal(at('gather-vote'), 'join');
  assert.equal(at('gather-voted'), 'voted');
  assert.equal(at('gather-result'), 'result');
  assert.equal(at('gather-tie'), 'result');
  assert.equal(m.screenOf(map(S['gather-closed'].info, { me: null })), 'closed');
});

test('view: whoSees follows the fields and show_names; nth and move', () => {
  const said = (f) => m.whoSees(f).map(([a, b]) => a + b).join(' ');
  assert.match(said(map(viewOf('gather-rsvp-in')).facts), /Dana/);
  assert.equal(m.nth(1), '1st');
  assert.equal(m.nth(2), '2nd');
  assert.equal(m.nth(3), '3rd');
  assert.equal(m.nth(11), '11th');
  assert.equal(m.nth(22), '22nd');
  assert.deepEqual(m.move([0, 1, 2], 1, 'up'), [1, 0, 2]);
  assert.deepEqual(m.move([0, 1, 2], 1, 'down'), [0, 2, 1]);
  assert.deepEqual(m.move([0, 1, 2], 2, 'down'), [0, 1], 'moving the last down leaves it out');
  const o = [0, 1, 2];
  assert.equal(m.move(o, 0, 'up'), o, 'top edge: unchanged');
});

// ---------- the app against the fake gather box ----------

function harness(name) {
  const calls = [];
  let now = Date.parse('2026-12-02T12:00:00Z');
  let timers = [];
  let nextId = 1;
  const box = new GatherBox(name, { code: 't' });
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
  };
  const root = document.getElementById('app');
  root.replaceChildren();
  document.querySelectorAll('.toast').forEach((t) => t.remove());
  const app = new m.GatherApp(root, env, 't');
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
  return { app, root, box, calls, advance, $, screen: () => $('[data-screen]')?.getAttribute('data-screen'), text: () => root.textContent.replace(/\s+/g, ' '), actions: () => calls.filter((c) => c.path === 'action').map((c) => c.body.name), acts: () => calls.filter((c) => c.path === 'action').map((c) => c.body) };
}
async function open(name) {
  const t = harness(name);
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
const pick = async (el) => {
  assert.ok(el, 'input to pick');
  el.checked = !el.checked || el.type === 'radio';
  el.dispatchEvent(new dom.window.Event('change', { bubbles: true }));
  await flush();
};
const submit = async (form) => {
  form.dispatchEvent(new dom.window.Event('submit', { bubbles: true, cancelable: true }));
  await flush();
};
const holo = (t) => t.root.querySelectorAll('.kw-sticker-holo, [data-sticker="holo"]').length;

test('G1 RSVP join: keep line first, facts, plus-ones and food only for Yes, sends only chosen fields, then G2', async () => {
  const t = await open('gather-rsvp');
  assert.equal(t.screen(), 'join');
  const form = t.$('form');
  assert.ok(form.firstElementChild.matches('[data-keep]'), 'keep line first under the bar');
  assert.match(t.text(), /Kept on Dana's computer until .*Dec 19.* \(7 days after the event\), then deleted\./);
  assert.match(t.text(), /Club winter dinner/);
  assert.match(t.text(), /14 answered/);
  assert.match(t.text(), /Each\s*€15/);
  assert.ok(t.$('[data-whosees]'));
  assert.equal(t.$('[data-send]').disabled, true);
  assert.equal(t.$('.stepper'), null, 'no plus-ones before Yes');
  await type(t.$('#g-name'), 'Ana');
  await type(t.$('#g-pin'), '1234');
  assert.equal(t.$('[data-send]').disabled, true, 'nothing picked yet');
  await pick(t.$('input[name="answer"][value="yes"]'));
  assert.equal(t.$('[data-send]').disabled, false);
  assert.ok(t.$('.opt.on input[value="yes"]'));
  assert.equal(t.$('[data-k="plus-less"]').getAttribute('aria-label'), 'One fewer');
  assert.equal(t.$('[data-k="plus-less"]').getAttribute('aria-disabled'), 'true');
  await click(t.$('[data-k="plus-more"]'));
  assert.match(t.$('[data-plus]').textContent, /^1plus-one$/);
  assert.equal(t.$('[data-plus]').getAttribute('aria-live'), 'polite');
  await pick(t.$('[data-k="food-vegetarian"]'));
  assert.ok(t.$('.tchips label.on [data-k="food-vegetarian"]'));
  await submit(t.$('form'));
  assert.equal(t.screen(), 'in');
  const join = t.calls.find((c) => c.path === 'join');
  assert.deepEqual(Object.keys(join.body).sort(), ['avatar', 'name', 'pin']);
  assert.deepEqual(t.acts(), [{ name: 'answer', body: { answer: 'yes', plus_ones: 1, food: ['vegetarian'] } }]);
  assert.match(t.text(), /You're in, Ana!/);
  assert.match(t.$('[data-summary]').textContent, /Yes · \+1 · vegetarian/);
  assert.equal(t.$('[data-paid]'), null, 'never paid until the host ticks it');
  t.app.stop();
});

test('G1 a No sends no plus-ones or food', async () => {
  const t = await open('gather-rsvp');
  await type(t.$('#g-name'), 'Ana');
  await type(t.$('#g-pin'), '1234');
  await pick(t.$('input[name="answer"][value="yes"]'));
  await click(t.$('[data-k="plus-more"]'));
  await pick(t.$('input[name="answer"][value="no"]'));
  assert.equal(t.$('.stepper'), null);
  await submit(t.$('form'));
  assert.deepEqual(t.acts(), [{ name: 'answer', body: { answer: 'no' } }]);
  assert.match(t.text(), /Thanks, Ana/);
  t.app.stop();
});

test('G2 in: summary, paid line only for them, who is coming, change my answer starts from what was sent', async () => {
  const t = await open('gather-rsvp-in');
  assert.equal(t.screen(), 'in');
  assert.match(t.text(), /You're in, Mia!/);
  assert.match(t.$('[data-paid]').textContent, /Dana marked you as paid · Dec 4 · only you see this/);
  assert.match(t.text(), /Who's coming 12 with plus-ones/);
  assert.equal(t.root.querySelectorAll('.who-grid li').length, 4);
  assert.match(t.text(), /Maybe: Ana/);
  assert.equal(holo(t), 0);
  await click(t.$('[data-change]'));
  assert.equal(t.screen(), 'join');
  assert.ok(t.$('input[name="answer"][value="yes"]').checked);
  assert.match(t.$('[data-plus]').textContent, /^1/);
  assert.equal(t.$('#g-pin'), null, 'seated: no PIN again');
  await click(t.$('[data-cancel]'));
  assert.equal(t.screen(), 'in');
  t.app.stop();
});

test('G2 without names: counts only', async () => {
  const t = await open('gather-rsvp-counts');
  assert.equal(t.$('.who-grid'), null);
  assert.match(t.text(), /Yes\s*9.*Maybe\s*3.*No\s*2/);
  assert.match(t.text(), /Got it, Mia/);
  t.app.stop();
});

test('G3 sign-up: slot cards, full one is dashed with the waiting line, button says waiting list', async () => {
  const t = await open('gather-signup');
  assert.equal(t.screen(), 'join');
  const cards = t.root.querySelectorAll('.gslot');
  assert.equal(cards.length, 4);
  assert.ok(cards[2].classList.contains('full'));
  assert.match(cards[2].textContent, /Full: you'd be 1st in line/);
  assert.match(cards[3].textContent, /Nobody yet/);
  assert.match(cards[0].textContent, /2 of 4/);
  assert.doesNotMatch(t.text(), /spots? left/i, 'no computed spots left');
  await type(t.$('#g-name'), 'Ana');
  await type(t.$('#g-pin'), '1234');
  await pick(t.$('input[name="slot"][value="2"]'));
  assert.equal(t.$('[data-send]').textContent, 'Join the waiting list');
  await pick(t.$('input[name="slot"][value="3"]'));
  assert.equal(t.$('[data-send]').textContent, 'Sign me up');
  await type(t.$('#g-note'), 'lemonade');
  await submit(t.$('form'));
  assert.deepEqual(t.acts(), [{ name: 'take', body: { slot: 3, note: 'lemonade' } }]);
  assert.equal(t.screen(), 'in');
  assert.match(t.text(), /You're in: Drinks/);
  t.app.stop();
});

test('sign-up in: let the spot go asks twice', async () => {
  const t = await open('gather-signup-in');
  assert.equal(t.screen(), 'in');
  await click(t.$('[data-let-go]'));
  assert.equal(t.actions().length, 0);
  assert.equal(document.activeElement, t.$('[data-let-go-yes]'));
  await click(t.$('[data-keep-it]'));
  assert.equal(t.actions().length, 0);
  await click(t.$('[data-let-go]'));
  await click(t.$('[data-let-go-yes]'));
  assert.deepEqual(t.actions(), ['let_go']);
  assert.equal(t.screen(), 'join');
  t.app.stop();
});

test('G4 waiting: the server position, the list, leave sends leave', async () => {
  const t = await open('gather-waiting');
  assert.equal(t.screen(), 'waiting');
  assert.equal(t.$('[data-position]').textContent, '1st');
  assert.match(t.text(), /You're on the Desserts waiting list/);
  assert.match(t.text(), /in line · Desserts is 2 of 2/);
  assert.match(t.$('.gi.me').textContent, /you · waiting 1st/);
  assert.match(t.text(), /Mains, Salads and Drinks still have room\./);
  await click(t.$('[data-let-go]'));
  await click(t.$('[data-let-go-yes]'));
  assert.deepEqual(t.actions(), ['leave']);
  t.app.stop();
});

test('G5 moved up: ink New, never holo, seen sent once', async () => {
  const t = await open('gather-moved');
  assert.equal(t.screen(), 'moved');
  assert.ok(t.$('[data-sticker="ink"]'));
  assert.equal(holo(t), 0);
  assert.match(t.text(), /You're in: Desserts/);
  assert.match(t.text(), /You moved up from the waiting list/);
  assert.match(t.text(), /Let the spot go, and Tom moves up\./);
  assert.deepEqual(t.actions(), ['seen']);
  assert.equal(t.box.seat.view.me.moved_up.new, false);
  t.app.paint(true);
  assert.ok(t.$('[data-sticker="ink"]'), 'still New on this page load');
  t.app.stop();
});

test('G6 one pick: radio rows, sends one', async () => {
  const t = await open('gather-vote');
  assert.equal(t.screen(), 'join');
  assert.equal(t.root.querySelectorAll('input[name="pick"]').length, 4);
  await type(t.$('#g-name'), 'Ana');
  await type(t.$('#g-pin'), '1234');
  await pick(t.$('input[name="pick"][value="1"]'));
  assert.ok(t.$('.ans.pick .radio'));
  await submit(t.$('form'));
  assert.deepEqual(t.acts(), [{ name: 'vote', body: { ranking: [1] } }]);
  assert.equal(t.screen(), 'voted');
  t.app.stop();
});

test('G7 ranked: add, move with focus staying on the row, leave the last out', async () => {
  const t = await open('gather-ranked');
  assert.equal(t.root.querySelectorAll('.rk.un').length, 4);
  assert.match(t.$('[data-k="add-0"]').textContent, /Tap to rank it 1st/);
  await click(t.$('[data-k="add-0"]'));
  await click(t.$('[data-k="add-1"]'));
  await click(t.$('[data-k="add-2"]'));
  assert.match(t.$('[data-k="add-3"]').textContent, /Tap to rank it 4th, or leave it out/);
  const order = () => [...t.root.querySelectorAll('.ranks .rk[data-opt]')].map((r) => Number(r.dataset.opt));
  assert.deepEqual(order(), [0, 1, 2]);
  const up0 = t.$('[data-k="up-0"]');
  assert.equal(up0.getAttribute('aria-disabled'), 'true');
  assert.equal(up0.getAttribute('aria-label'), 'Move The Old Mill up');
  assert.equal(t.$('[data-k="down-2"]').getAttribute('aria-label'), 'Leave Sushi Bar Ko out');
  // Move Casa Verde up: focus stays on its up button, on its new row.
  t.$('[data-k="up-1"]').focus();
  await click(t.$('[data-k="up-1"]'));
  assert.deepEqual(order(), [1, 0, 2]);
  assert.equal(document.activeElement, t.$('[data-k="up-1"]'));
  assert.ok(document.activeElement.closest('.rk').classList.contains('slide-up'));
  // At the top the up button does nothing and keeps focus.
  await click(t.$('[data-k="up-1"]'));
  assert.deepEqual(order(), [1, 0, 2]);
  assert.equal(document.activeElement, t.$('[data-k="up-1"]'));
  // The last one down leaves it out; focus goes to its add row.
  await click(t.$('[data-k="down-2"]'));
  assert.deepEqual(order(), [1, 0]);
  assert.equal(document.activeElement, t.$('[data-k="add-2"]'));
  await type(t.$('#g-name'), 'Ana');
  await type(t.$('#g-pin'), '1234');
  await submit(t.$('form'));
  assert.deepEqual(t.acts(), [{ name: 'vote', body: { ranking: [1, 0] } }]);
  assert.equal(t.screen(), 'voted');
  t.app.stop();
});

test('G8 voted: own order, live bars from the server counts as width classes, no holo', async () => {
  const t = await open('gather-voted');
  assert.equal(t.screen(), 'voted');
  assert.match(t.$('[data-ballot]').textContent, /1 The Old Mill · 2 Casa Verde · 3 Sushi Bar Ko/);
  const bars = t.root.querySelectorAll('.vb');
  assert.equal(bars.length, 4);
  assert.ok(bars[0].classList.contains('lead'));
  assert.equal(bars[0].querySelector('.tr').getAttribute('aria-label'), 'The Old Mill: 5 of 11');
  assert.ok(bars[0].querySelector('.tr i').classList.contains('w9'));
  assert.ok(bars[3].querySelector('.tr i').classList.contains('w0'));
  assert.equal(t.root.querySelectorAll('[style]').length, 0, 'no style attributes (CSP)');
  assert.equal(holo(t), 0);
  assert.ok(t.$('[data-change]'));
  t.app.stop();
});

test('G9 result: the winner earns the one holo; rounds read from the server', async () => {
  const t = await open('gather-result');
  assert.equal(t.screen(), 'result');
  assert.equal(holo(t), 1);
  assert.match(t.$('.matchcard').textContent, /The vote picked.*The Old Mill.*7 of 11 in the final round.*Picked/);
  assert.match(t.text(), /Round 1: The Old Mill 5 · Casa Verde 4 · Sushi Bar Ko 2 · Pizza Nonna 1\. Pizza Nonna drops out\./);
  assert.match(t.text(), /Round 3: .* 1 ballot had no choice left\./);
  assert.match(t.text(), /Ballots are deleted on .*Dec 19/);
  t.app.stop();
});

test('G9 tie: plain notice, no holo', async () => {
  const t = await open('gather-tie');
  assert.equal(t.screen(), 'result');
  assert.equal(holo(t), 0);
  assert.match(t.text(), /Tie: Dana picks/);
  assert.match(t.text(), /The Old Mill and Casa Verde are level\./);
  t.app.stop();
});

test('G10 rejoin: a wrong PIN says the server tries left and empties the PIN', async () => {
  const t = await open('gather-wrongpin');
  await click(t.$('[data-to-rejoin]'));
  assert.equal(t.screen(), 'rejoin');
  assert.match(t.text(), /Welcome back/);
  assert.match(t.$('[data-rejoin]').textContent, /Open my answer/);
  await type(t.$('#g-bname'), 'Mia');
  await type(t.$('#g-bpin'), '0000');
  await submit(t.$('form'));
  assert.equal(t.screen(), 'rejoin');
  assert.match(t.$('[data-problem]').textContent, /That name and PIN don't match\./);
  assert.match(t.$('[data-problem]').textContent, /3/);
  t.app.stop();
});

test('G10 rejoin opens the answer', async () => {
  const t = await open('gather-rsvp');
  await click(t.$('[data-to-rejoin]'));
  await type(t.$('#g-bname'), 'Mia');
  await type(t.$('#g-bpin'), '1234');
  await submit(t.$('form'));
  assert.equal(t.screen(), 'in');
  assert.ok(t.$('[data-paid]'));
  t.app.stop();
});

test('G11 closed: Tally, the server count, the deletion day', async () => {
  const t = await open('gather-closed');
  assert.equal(t.screen(), 'closed');
  assert.match(t.text(), /RSVPs are closed/);
  assert.match(t.text(), /12 are coming on .*Need to change something\? Ask Dana\./);
  assert.match(t.text(), /Answers and food needs are deleted on .*Dec 19/);
  t.app.stop();
});

test('host asleep: Tally, try now, the answer is safe; comes back by itself', async () => {
  const t = await open('gather-rsvp-in');
  t.box.up = false;
  t.app.wake();
  await t.advance(3000);
  assert.equal(t.screen(), 'away');
  assert.match(t.text(), /Your answer is safe\./);
  assert.match(t.text(), /Dana's computer is asleep/);
  assert.ok(t.$('[data-try]'));
  assert.match(t.$('[data-away]').textContent, /Trying again in \d+ s · tried \d+ time/);
  t.box.up = true;
  await t.advance(60_000);
  assert.equal(t.screen(), 'in');
  t.app.stop();
});

test('avatar follows the name, from the eight', () => {
  assert.equal(m.avatarFor('Ana'), m.avatarFor(' ana '));
  assert.ok(['antenna', 'ears', 'visor', 'spikes', 'beret', 'goggles', 'bulb', 'horns'].includes(m.avatarFor('Zoë')));
});

// ---------- css ----------

test('css: gather motion as specified and static under reduced motion; bar buckets; holo tokens match modern.css', () => {
  const css = readFileSync(new URL('../guest/src/guest.css', import.meta.url), 'utf8');
  const ga = css.slice(css.indexOf('pa10b gather'), css.indexOf('pa10a Secret Santa'));
  assert.match(ga, /\.vb \.tr i \{[^}]*transition: transform 160ms var\(--kw-ease\)/);
  assert.match(ga, /@keyframes ga-pick \{ from \{ transform: scale\(0\.98\); \} \}/);
  assert.match(ga, /\.opt\.on, \.gslot\.on, \.ans\.pick \{ animation: ga-pick 120ms/);
  assert.match(ga, /@keyframes ga-up \{ from \{ transform: translateY\(64px\); \} \}/);
  assert.match(ga, /\.rk\.slide-up \{ animation: ga-up 160ms/);
  const reduced = ga.slice(ga.lastIndexOf('prefers-reduced-motion'));
  for (const s of ['.vb .tr i', '.opt.on', '.rk.slide-up', '.rk.slide-down', '.kw-sticker-holo::before']) assert.ok(reduced.includes(s), s);
  for (let i = 0; i <= 20; i++) assert.match(ga, new RegExp(`\\.ga \\.vb \\.tr i\\.w${i} \\{ transform: scaleX\\(`));
  assert.doesNotMatch(ga, /#[0-9a-fA-F]{3,8}\b/, 'no hex outside :root');
  const modern = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
  const foil = modern.slice(modern.indexOf('.kw-sticker-holo::before {'));
  const stops = [...foil.slice(0, foil.indexOf('}')).matchAll(/#[0-9a-f]{6}/g)].map((x) => x[0]).slice(0, 5);
  const v = (k) => css.match(new RegExp(`--${k}:\\s*([^;]+);`))[1].trim();
  assert.deepEqual([1, 2, 3, 4, 5].map((i) => v(`holo-${i}`)), stops);
});
