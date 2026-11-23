// Secret Santa Pocket card (pa10a, autonomy/pocket-apps/pa10a/design.md §1, card-brief.md): validSanta for a good
// block per phase, missing fields falling back, junk rejected, anything match-like (a match, the loop, a
// question's words, a wish list) rejected anywhere in the block, All ready only when every gift is ticked (the
// one holo), People and PINs with Reset PIN and Remove before the draw, the join box with no quick tunnel,
// buttons with no santa action yet saying why, and the Santa avatar.
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
const clipboard = { text: null, fail: false, writeText: async (t) => {
  if (clipboard.fail) throw new Error('denied');
  clipboard.text = t;
} };
Object.defineProperty(g.navigator, 'clipboard', { value: clipboard, configurable: true });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { MarkdownRenderer } from './src/components/MarkdownRenderer';
  export { COMPOSE_PREFILL_EVENT } from './src/components/results/SmartResult';
  export { validPocket } from './src/components/results/Pocket';
  export { validSanta, findMatchLike, earnedAllReady, MATCH_KEYS, SANTA_LATER, howToSteps, expiryText } from './src/components/results/PocketSanta';
  export { SANTA_FIXTURES, SANTA_RESET_CODE, POCKET_FIXTURES } from './src/components/results/fixtures';
  export { pocketApi, PocketApiError } from './src/pocketApi';
  export { pickCreature, AVATAR_STYLES, guestCreature, GUEST_CREATURES } from './src/components/ModernAvatar';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mounted = new Set();
const realFetch = g.fetch;
afterEach(() => {
  for (const v of mounted) v.done();
  mounted.clear();
  localStorage.clear();
  clipboard.text = null;
  clipboard.fail = false;
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
  el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const key = async (el, k) => {
  el.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: k, bubbles: true }));
  await settle();
};
const F = m.SANTA_FIXTURES;
const raw = (name) => structuredClone(F[name].results.blocks[0]);
const withSanta = (name, over) => {
  const b = raw(name);
  b.santa = { ...b.santa, ...over };
  return b;
};
const card = (name, block = raw(name)) => mount(h(m.MarkdownRenderer, { content: F[name].reply, results: { version: 1, blocks: [block] }, messageId: `m-${name}` }));
const button = (v, text) => [...v.host.querySelectorAll('button, a')].find((b) => b.textContent.trim() === text);
const holos = (v) => v.host.querySelectorAll('.kw-sticker-holo, [data-tone="holo"]').length;
const ok = (b) => assert.ok(m.validPocket(b), 'should validate');
const bad = (b, why) => assert.equal(m.validPocket(b), null, why);

// ---------------------------------------------------------------------------------------------------
// The validator

test('a good block per phase: each validates and renders the santa card in its phase', async () => {
  const phases = { collecting: 'collecting', noAddress: 'collecting', drawn: 'drawn', revealDay: 'reveal_day', revealDayNotAll: 'reveal_day', ended: 'ended', people: 'drawn' };
  for (const [name, phase] of Object.entries(phases)) {
    const b = m.validPocket(raw(name));
    assert.ok(b, `${name} validates`);
    assert.equal(b.santa.phase, phase);
    const v = await card(name);
    const el = v.host.querySelector('[data-santa]');
    assert.ok(el, `${name} renders the santa card`);
    assert.equal(el.dataset.santa, phase);
    assert.match(el.textContent, /Pocket app · Secret Santa/);
    assert.match(el.textContent, /About 3 bot calls to set this up, about \$0\.01\. The draw and everything after it use none\./);
    v.done();
  }
});

test('missing optional fields fall back to plain words', async () => {
  const b = raw('drawn');
  for (const k of ['expected', 'waiting_for', 'people', 'keep_apart', 'draw_rule', 'drawn_at', 'reveal', 'reveal_at', 'deletes_on', 'looked', 'gifts_ready', 'asks_asked', 'asks_answered', 'can_draw', 'blocked', 'join']) delete b.santa[k];
  const s = m.validPocket(b).santa;
  assert.deepEqual([s.people, s.keep_apart, s.waiting_for, s.reveal, s.join, s.can_draw], [[], [], [], 'scheduled', null, false]);
  const v = await card('drawn', b);
  const t = v.host.textContent;
  assert.match(t, /Drawn · everyone has their match/);
  assert.match(t, /Rules kept: nobody drew themselves\. Each person sees only their own match\./);
  assert.match(t, /Kept until 7 days after the gift day, then deleted\./);
  assert.equal(v.host.querySelector('[data-join]'), null, 'no link, no join box');
  assert.ok(button(v, 'Open my Santa page').getAttribute('aria-disabled'), 'no link: my page says why');
});

test('junk and contradictions are rejected, so the reply shows as markdown', async () => {
  bad(withSanta('collecting', { phase: 'shuffling' }), 'unknown phase');
  bad(withSanta('collecting', { joined: -1 }), 'negative count');
  bad(withSanta('collecting', { joined: '7' }), 'count as text');
  bad(withSanta('drawn', { gifts_ready: 12 }), 'more gifts than people');
  bad(withSanta('drawn', { looked: 10 }), 'more looked than people');
  bad(withSanta('drawn', { asks_answered: 9 }), 'more answers than questions');
  bad(withSanta('drawn', { reveal: 'maybe' }), 'unknown reveal');
  bad(withSanta('collecting', { keep_apart: [['Mia']] }), 'a pair of one');
  bad(withSanta('collecting', { waiting_for: [42] }), 'a name that is not text');
  bad(withSanta('collecting', { people: [{ id: 'p1', name: '' }] }), 'a person with no name');
  bad(withSanta('collecting', { people: [{ id: 'p1', name: 'A', you: true }, { id: 'p2', name: 'B', you: true }] }), 'two people are "you"');
  bad(withSanta('collecting', { people: [{ id: 'p1', name: 'A', avatar: '<script>' }] }), 'a creature id that is not an id');
  bad(withSanta('collecting', { join: { mode: 'anywhere', anywhere_url: null, wifi_url: 'http://192.168.1.20:7480/p/x' } }), 'from anywhere with no address');
  bad(withSanta('collecting', { join: { mode: 'anywhere', anywhere_kind: 'https', anywhere_url: 'https://fuzzy-otter.trycloudflare.com/p/x', wifi_url: null } }), 'a quick tunnel is never offered');
  bad(withSanta('collecting', { join: { mode: 'wifi', wifi_url: 'http://x/p', quick_tunnel: true } }), 'quick_tunnel on');
  bad(withSanta('collecting', { join: { mode: 'wifi', wifi_url: 'javascript:alert(1)' } }), 'a link that is not http');
  bad({ ...raw('collecting'), state: 'ended' }, 'ended state with a live phase');
  bad({ ...raw('ended'), state: 'live' }, 'live state with the ended phase');
  bad({ ...raw('collecting'), santa: null }, 'kit santa without its santa part');
  bad({ ...structuredClone(m.POCKET_FIXTURES.live.results.blocks[0]), santa: raw('collecting').santa }, 'a quiz carrying a santa part');
  bad({ ...raw('ended'), results: structuredClone(m.POCKET_FIXTURES.results.results.blocks[0].results) }, 'quiz results on a santa');
  const v = await card('collecting', withSanta('collecting', { phase: 'shuffling' }));
  assert.equal(v.host.querySelector('[data-santa]'), null, 'no card');
});

test('anything match-like is rejected, at any depth, the whole block included', () => {
  const leaks = {
    match: { match: 'Sam' },
    matches: { matches: [{ from: 'Mia', to: 'Sam' }] },
    pairs: { pairs: [['Mia', 'Sam']] },
    gives_to: { people: [{ id: 'p1', name: 'Oleks', gives_to: 'Mia' }] },
    receiver: { people: [{ id: 'p1', name: 'Oleks', receiver: 'p2' }] },
    loop: { loop: ['p1', 'p2', 'p3'] },
    reveal_loop: { reveal_loop: ['p1', 'p2'] },
    my_santa: { my_santa: 'Kai' },
    questions: { questions: [{ q: 'Does Mia like tea?' }] },
    asks: { asks: ['Socks or a scarf?'] },
    question_text: { stats: { text: 'Socks or a scarf?' } },
    wish_list: { people: [{ id: 'p2', name: 'Mia', wish_list: 'Books' }] },
    deep: { join: { mode: 'wifi', wifi_url: 'http://192.168.1.20:7480/p/x', extra: { a: { b: [{ assignment: 'p2' }] } } } },
  };
  for (const [what, over] of Object.entries(leaks)) bad(withSanta('drawn', over), `a ${what} leak`);
  // Outside the santa part too: a top-level field or one inside ended/live.
  bad({ ...raw('drawn'), matches: [['Mia', 'Sam']] }, 'a top-level matches');
  bad({ ...raw('ended'), ended: { deletes_on: 'Fri, Dec 25', loop: ['p1'] } }, 'a loop inside ended');
  assert.equal(m.findMatchLike({ keep_apart: [['Mia', 'Sam']], counts: 3 }), null, 'keep_apart is the host’s own rule, allowed');
  assert.equal(m.findMatchLike({ x: { MATCH: 1 } }), '.x.MATCH', 'case does not hide it');
  assert.ok(m.MATCH_KEYS.includes('gives_to') && m.MATCH_KEYS.includes('questions'));
  // The quiz's own `answer` fields stay fine: the scan is for santa blocks only.
  assert.ok(m.validPocket(raw('drawn')));
  assert.ok(m.validPocket(structuredClone(m.POCKET_FIXTURES.results.results.blocks[0])));
});

test('All ready (the one holo) only when every gift is ticked, on reveal day and after', async () => {
  const ready = (o) => m.earnedAllReady({ phase: 'reveal_day', joined: 9, gifts_ready: 9, ...o });
  assert.equal(ready({}), true);
  assert.equal(ready({ phase: 'ended' }), true);
  assert.equal(ready({ gifts_ready: 8 }), false, 'one gift short');
  assert.equal(ready({ phase: 'drawn' }), false, 'not before reveal day');
  assert.equal(ready({ joined: 0, gifts_ready: 0 }), false, 'nobody in');
  assert.equal(ready({ gifts_ready: null }), false, 'unknown');
  for (const [name, n] of Object.entries({ collecting: 0, noAddress: 0, drawn: 0, people: 0, revealDayNotAll: 0, revealDay: 1, ended: 1 })) {
    const v = await card(name);
    assert.equal(holos(v), n, `${name}: ${n} holo`);
    if (n) assert.match(v.host.textContent, /All ready/);
    v.done();
  }
  // Drawn with every gift ticked still earns nothing yet; errors stay plain.
  const v = await card('drawn', withSanta('drawn', { gifts_ready: 9 }));
  assert.equal(holos(v), 0);
  const w = await card('collecting', withSanta('collecting', { blocked: 'Mia and Sam are kept apart from everyone else, so no draw works.' }));
  assert.equal(holos(w), 0);
  assert.match(w.host.textContent, /Santa can't draw yet\. Mia and Sam are kept apart/);
  const chip = w.host.querySelector('.kw-chip');
  assert.match(chip.textContent, /Collecting names/);
});

// ---------------------------------------------------------------------------------------------------
// The card

test('collecting: counts and first names only, the draw rule, kept-apart pairs, Add a pair prefills', async () => {
  const v = await card('collecting');
  const t = v.host.textContent;
  assert.match(t, /7 of 9 joined · waiting for Leo and Zoe/);
  assert.match(t, /The draw: when all 9 are in, or Sun, Dec 6 at 6 PM, whichever comes first\./);
  assert.match(t, /Kept apart:/);
  assert.equal(v.host.querySelectorAll('[data-kept-apart] .rounded-full').length, 2);
  assert.match(t, /Nobody sees anyone else's match, you included\./);
  assert.equal(v.host.querySelectorAll('[data-stack] [data-creature]').length, 7, 'one creature per person');
  let sent = null;
  const on = (e) => (sent = e.detail);
  window.addEventListener(m.COMPOSE_PREFILL_EVENT, on);
  await click(button(v, 'Add a pair'));
  window.removeEventListener(m.COMPOSE_PREFILL_EVENT, on);
  assert.ok(sent, 'Add a pair prefills the composer, never sends');
});

test('join box: Funnel first, Same Wi-Fi second, never a quick link; 1b says why with the How-to', async () => {
  const v = await card('collecting');
  const box = v.host.querySelector('[data-join="full"]');
  assert.equal(box.dataset.joinMode, 'anywhere');
  const radios = [...box.querySelectorAll('[role=radio]')];
  assert.deepEqual(radios.map((r) => r.textContent.trim()), ['From anywhere', 'Same Wi-Fi']);
  assert.match(box.textContent, /Tailscale Funnel address\. It stays the same/);
  assert.equal(box.querySelector('input').value, 'https://oleks-mac.tail4b2e.ts.net/p/sn4tq9');
  await key(radios[0], 'ArrowRight');
  assert.equal(box.querySelector('input').value, 'http://192.168.1.20:7480/p/sn4tq9', 'arrow keys move to Same Wi-Fi and the link follows');
  assert.equal(document.activeElement, radios[1]);
  assert.doesNotMatch(v.host.innerHTML, /trycloudflare/);

  const b = await card('noAddress');
  const nb = b.host.querySelector('[data-join="full"]');
  assert.equal(nb.dataset.joinMode, 'wifi');
  assert.equal(nb.querySelectorAll('[role=radio]').length, 0, 'no From anywhere to pick');
  assert.match(nb.textContent, /Same Wi-Fi only, for now\./);
  assert.match(nb.textContent, /No address that stays the same is set up on this Mac\. To let people join from anywhere for weeks, turn on Tailscale Funnel: free, about 5 minutes\./);
  assert.match(nb.textContent, /A quick link isn't offered: it changes when the Mac restarts/);
  const how = button(b, 'How to');
  assert.equal(how.getAttribute('aria-expanded'), 'false');
  await click(how);
  assert.equal(how.getAttribute('aria-expanded'), 'true');
  const steps = [...nb.querySelectorAll('[data-how-to] li')].map((li) => li.textContent);
  assert.equal(steps.length, 5);
  assert.match(steps[2], /tailscale funnel --bg 7480/, 'the port comes from the Wi-Fi link');
  assert.deepEqual(m.howToSteps(null)[2].includes('<the port'), true);
});

test('Copy invite copies the invite with the link that stays the same', async () => {
  const v = await card('collecting');
  await click(button(v, 'Copy invite'));
  assert.match(clipboard.text, /Family Secret Santa/);
  assert.match(clipboard.text, /https:\/\/oleks-mac\.tail4b2e\.ts\.net\/p\/sn4tq9$/);
  assert.match(v.host.textContent, /Invite copied/);
  const join = button(v, 'Join as Oleks');
  assert.equal(join.tagName, 'A', 'the host joins on their phone like everyone else');
});

test('drawn: looked, gifts and questions as counts, never words; the compact join box', async () => {
  const v = await card('drawn');
  const t = v.host.textContent;
  assert.match(t, /Drawn Sun, Dec 6 at 6:00 PM · everyone has their match/);
  assert.match(t, /Rules kept: nobody drew themselves, both pairs apart\. Each person sees only their own match\./);
  assert.match(t, /8 of 9 have looked at their match · Leo hasn't yet/);
  assert.match(t, /5 of 9 gifts ready/);
  assert.match(t, /6 Santa questions asked, 5 answered\. You see the count, never the words\./);
  assert.match(t, /Kept until Fri, Dec 25 \(7 days after the reveal\), then deleted\./);
  assert.match(t, /Same link for the whole month\. Lost it\? Scan again, then type your name and PIN\./);
  assert.ok(v.host.querySelector('[data-join="compact"]'));
  assert.equal(v.host.querySelector('[data-join="full"]'), null);
  assert.ok(v.host.querySelector('.kw-chip-ok'));
});

test('reveal day and ended: the reveal line, the chip, the deletion line', async () => {
  const r = await card('revealDay');
  assert.match(r.host.textContent, /9 of 9 gifts ready: every Santa ticked it/);
  assert.match(r.host.textContent, /At 7:00 PM everyone's page shows who was whose Santa\. Until then it stays a secret\./);
  assert.match(r.host.textContent, /11 Santa questions asked, all answered/);
  assert.ok(r.host.querySelector('[data-chip="reveal-day"]'));
  const e = await card('ended');
  assert.match(e.host.textContent, /Ended Dec 18/);
  assert.match(e.host.textContent, /Revealed Fri, Dec 18 at 7:00 PM · 9 people · 9 gifts/);
  assert.match(e.host.textContent, /The link is closed\. Names, wish lists and questions are deleted on Fri, Dec 25\./);
  assert.equal(e.host.querySelector('[data-join]'), null);
  assert.equal(button(e, 'People'), undefined, 'no People after the end');
});

test('buttons with no santa action yet are disabled and say why; Draw now says why it can’t', async () => {
  const d = await card('drawn');
  const dates = button(d, 'Change dates');
  assert.equal(dates.getAttribute('aria-disabled'), 'true');
  assert.equal(dates.getAttribute('title'), m.SANTA_LATER.dates);
  const c = await card('collecting');
  const draw = button(c, 'Draw now');
  assert.equal(draw.getAttribute('aria-disabled'), 'true');
  assert.ok(draw.getAttribute('title'));
  const few = await card('collecting', withSanta('collecting', { joined: 2, people: [] , waiting_for: []}));
  assert.equal(button(few, 'Draw now').getAttribute('title'), 'A draw needs at least 3 people');
  const can = await card('collecting', withSanta('collecting', { can_draw: true }));
  assert.equal(button(can, 'Draw now').getAttribute('aria-disabled'), null);
});

test('Draw now, Show the reveal now and Skip ask first and call the box’s named santa actions', async () => {
  const calls = [];
  g.fetch = async (url, init) => (calls.push([init.method, url, init.body ?? null]), new Response(JSON.stringify({ ok: true }), { status: 200 }));
  const c = await card('collecting', withSanta('collecting', { can_draw: true }));
  await click(button(c, 'Draw now'));
  assert.match(c.host.querySelector('[data-confirm="draw"]').textContent, /Only 7 of 9 are in/);
  assert.equal(document.activeElement, button(c, 'Draw names'), 'focus goes to the answer');
  await click(button(c, 'Draw names'));
  assert.deepEqual(calls.pop(), ['POST', '/api/pockets/sn4tq9/host/action', JSON.stringify({ name: 'draw_now' })]);
  assert.match(c.host.textContent, /Drawn\. Everyone can open their own match now\./);

  const r = await card('revealDay');
  await click(button(r, 'Skip the reveal'));
  await key(r.host.querySelector('[data-confirm="skip"]'), 'Escape');
  assert.equal(r.host.querySelector('[data-confirm]'), null, 'Escape keeps it');
  await click(button(r, 'Show the reveal now'));
  await click(button(r, 'Show it now'));
  assert.deepEqual(calls.pop(), ['POST', '/api/pockets/sn4tq9/host/action', JSON.stringify({ name: 'reveal_now' })]);

  g.fetch = async () => new Response(JSON.stringify({ error: 'already_revealed', message: 'The reveal already happened.' }), { status: 409 });
  const s = await card('revealDay');
  await click(button(s, 'Skip the reveal'));
  await click(button(s, 'Skip the reveal'));
  assert.match(s.host.querySelector('[data-done="error"]').textContent, /The reveal already happened\./);
  assert.equal(holos(s), 1, 'an error stays plain; the earned holo stays the only one');
});

test('Stop and Delete it all ask first and use the pocket routes', async () => {
  const calls = [];
  g.fetch = async (url, init) => (calls.push([init.method, url]), new Response(JSON.stringify({ pocket: { code: 'sn4tq9' } }), { status: 200 }));
  const d = await card('drawn');
  await click(button(d, 'Stop'));
  await click(button(d, 'Stop it'));
  assert.deepEqual(calls.pop(), ['POST', '/api/pockets/sn4tq9/end']);
  assert.equal(d.host.querySelector('[data-join]'), null, 'the link is gone once stopped');
  const e = await card('ended');
  await click(button(e, 'Delete it all now'));
  assert.match(e.host.textContent, /This can't be undone\./);
  await click(button(e, 'Delete it all'));
  assert.deepEqual(calls.pop(), ['DELETE', '/api/pockets/sn4tq9']);
});

test('People and PINs: inside the card, names and status only, Remove before the draw only', async () => {
  const v = await card('people');
  await click(button(v, 'People'));
  const panel = v.host.querySelector('[data-people]');
  assert.ok(panel && !panel.closest('[role=dialog]'), 'a panel in the card, not a modal');
  assert.match(panel.textContent, /People and PINs · 9/);
  assert.match(panel.textContent, /Matches never show here/);
  const status = (n) => panel.querySelector(`[data-person="${n}"] [data-status]`).textContent;
  assert.equal(status('Oleks'), 'you · seen · gift ready');
  assert.equal(status('Leo'), 'not looked yet');
  assert.equal(status('Ana'), 'locked: needs a reset code');
  assert.equal(status('Kai'), 'seen');
  await click(panel.querySelector('[data-person-menu="p2"]'));
  const items = [...panel.querySelectorAll('[role=menuitem]')];
  assert.deepEqual(items.map((i) => i.textContent.trim()), ['Reset PIN', 'Remove (before the draw only)']);
  assert.equal(items[1].getAttribute('aria-disabled'), 'true');
  assert.match(items[1].getAttribute('title'), /would change a match/);
  assert.equal(document.activeElement, items[0]);
  await key(items[0], 'ArrowDown');
  assert.equal(document.activeElement, items[1]);
  await key(items[1], 'Escape');
  assert.equal(panel.querySelector('[role=menu]'), null);

  const calls = [];
  g.fetch = async (url, init) => (calls.push([init.method, url, init.body]), new Response(JSON.stringify({ ok: true }), { status: 200 }));
  const c = await card('collecting');
  await click(button(c, 'People'));
  const cp = c.host.querySelector('[data-people]');
  assert.match(cp.textContent, /People and PINs · 7/);
  await click(cp.querySelector('[data-person-menu="p3"]'));
  await click(button(c, 'Remove'));
  assert.match(cp.querySelector('[data-confirm="remove"]').textContent, /Nothing is drawn yet, so nobody's match changes\./);
  await click(button(c, 'Remove Sam'));
  assert.deepEqual(calls.pop(), ['POST', '/api/pockets/sn4tq9/host/remove', JSON.stringify({ player_id: 'p3' })]);
  assert.match(cp.textContent, /People and PINs · 6/);
  assert.equal(cp.querySelector('[data-person="Sam"]'), null);
});

test('Reset PIN: asks first, then shows only the one-time code (mono 28px) and its expiry', async () => {
  const calls = [];
  g.fetch = async (url, init) => (calls.push([url, init.body]), new Response(JSON.stringify({ ...m.SANTA_RESET_CODE, player: { name: 'Mia', match: 'Sam' } }), { status: 200 }));
  const v = await card('people');
  await click(button(v, 'People'));
  const panel = v.host.querySelector('[data-people]');
  await click(panel.querySelector('[data-person-menu="p2"]'));
  await click(button(v, 'Reset PIN'));
  const ask = panel.querySelector('[data-confirm="reset"]');
  assert.match(ask.textContent, /Reset Mia's PIN\?/);
  assert.match(ask.textContent, /Mia's match, wish list and questions stay as they are\. You won't see any of them\./);
  assert.doesNotMatch(ask.textContent, /\b(Her|her|His|his|She|she|He|he)\b/, 'no guessed pronouns');
  await click(button(v, 'Reset and show the code'));
  assert.deepEqual(calls.pop(), ['/api/pockets/sn4tq9/host/reset-pin', JSON.stringify({ player_id: 'p2' })]);
  const box = panel.querySelector('[data-reset-code]');
  assert.match(box.textContent, /Mia's one-time code/);
  const code = box.querySelector('[data-code]');
  assert.equal(code.textContent, 'K7QX');
  assert.match(code.className, /font-mono/);
  assert.match(code.className, /text-\[28px\]/);
  assert.match(box.textContent, /It works once, until \w{3}, Dec 8 at \d{1,2}:\d{2}/);
  assert.doesNotMatch(box.textContent, /Sam/, 'whatever else the box sends is never shown');
  await click(button(v, 'Copy code'));
  assert.equal(clipboard.text, 'K7QX');
  assert.equal(holos(v), 0);
  await click(button(v, 'Done'));
  assert.equal(panel.querySelector('[data-reset-code]'), null);
  assert.equal(document.activeElement, panel.querySelector('[data-person-menu="p2"]'), 'focus goes back to the row');

  g.fetch = async () => new Response(JSON.stringify({ code: 'not a code!', expires_at: 'x' }), { status: 200 });
  const w = await card('people');
  await click(button(w, 'People'));
  await click(w.host.querySelector('[data-person-menu="p2"]'));
  await click(button(w, 'Reset PIN'));
  await click(button(w, 'Reset and show the code'));
  assert.match(w.host.querySelector('[data-confirm="reset"] [role=alert]').textContent, /Couldn't reset it: The box sent something unexpected\./);
  assert.equal(w.host.querySelector('[data-reset-code]'), null);
});

test('expiry is said in the person’s own words', () => {
  assert.match(m.expiryText('2026-12-08T21:41:00Z'), /^until \w{3}, Dec \d at \d{1,2}:\d{2}\s?[AP]M$/);
  assert.equal(m.expiryText('soon'), 'for 24 hours');
});

test('Santa avatar: a picker style, green with the knit hat; guests get the creature they picked', () => {
  assert.ok(m.AVATAR_STYLES.some((s) => s.id === 'santa' && s.label === 'Santa'));
  const c = m.pickCreature('santa', 'Anything');
  assert.equal(c.accessory, 'knithat');
  assert.equal(m.pickCreature('default', 'Secret Santa helper').accessory, 'knithat');
  assert.equal(Object.keys(m.GUEST_CREATURES).length, 8);
  assert.equal(m.guestCreature('horns').accessory, 'horns');
  assert.equal(m.guestCreature('nope').accessory, m.guestCreature('antenna').accessory, 'unknown falls back to antenna');
});
