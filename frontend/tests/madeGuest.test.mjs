// pa12 P2: the ready-made blocks of a "Make your own" pocket (autonomy/pocket-apps/pa12/P2/brief.md, blocks.md). The
// view mapping (junk dropped, a count stays a count), every block on the phone, the big screen and the host remote from
// the fake box's two section-1 examples, hostile guest text drawn as text, the actions each block sends, and that no
// view the fake box gives a phone or a screen carries a secret before the reveal. The browser half is guest/e2e/made.mjs.
import './pinLocale.mjs';
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { readFileSync, readdirSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';
import { MadeBox, madeScenes } from '../guest/fake/made.mjs';

const dom = new JSDOM('<!doctype html><html><body><div id="app"></div></body></html>', { url: 'http://localhost/p/made-shelf', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'SVGElement']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { madeFrom, blockFrom, secondsLeft, clock, MAX_BLOCKS } from './guest/src/made/view';
  export { drawBlocks, stepBar, newUi } from './guest/src/made/blocks';
  export { MadeApp } from './guest/src/made/app';
`);

const SURFACE = { '': 'phone', 'screen-': 'screen', 'remote-': 'remote' };
const env = (pre, scene) => new MadeBox(`${pre}made-${scene}`).env(true);
const viewOf = (pre, scene) => m.madeFrom(env(pre, scene)).view;

/** Draws one surface of a scene into a fresh section; `sent` collects what the blocks send. */
function draw(pre, scene, { now, reply = null } = {}) {
  const v = viewOf(pre, scene);
  now ??= Date.now(); // after the view: MadeBox stamps ends_at with Date.now(), so an earlier now could read one second more
  const sent = [];
  const ui = m.newUi(() => {});
  const c = { v, surface: SURFACE[pre], send: async (name, body) => (sent.push({ name, body }), reply), ui, now, skew: 0 };
  const root = document.createElement('div');
  root.append(...(pre === 'remote-' ? [m.stepBar(c)].filter(Boolean) : []), ...m.drawBlocks(c));
  return { root, v, sent, ui, text: root.textContent.replace(/\s+/g, ' ') };
}
const types = (root) => [...root.querySelectorAll('[data-block]')].map((e) => e.dataset.block);
const settle = () => new Promise((r) => setTimeout(r, 0));

test('view: a made state maps; another kit, junk and unknown blocks are dropped', () => {
  assert.equal(m.madeFrom(null), undefined);
  assert.equal(m.madeFrom({ ...env('', 'shelf'), pocket: { kit: 'quiz' } }), undefined);
  const e = env('', 'shelf');
  e.view.app.blocks = [{ type: 'say', text: 'Hi' }, { type: 'script', src: 'x.js' }, { type: 'form', into: 'Bad Name', fields: [{ name: 'a', type: 'text' }] }, { type: 'board', from: 'shelf' }, null, 7];
  e.view.collections.junk = { rows: 'no' };
  e.view.collections.Bad = { count: 3 };
  e.view.collections.neg = { count: -1 };
  const v = m.madeFrom(e).view;
  assert.deepEqual(v.blocks.map((b) => b.type), ['say']);
  assert.deepEqual(Object.keys(v.collections).sort(), ['picks', 'shelf'], 'rows that are not a list, a bad name and a negative count are dropped');
  assert.equal(m.madeFrom(env('', 'shelf')).version, 1);
  assert.equal(m.blockFrom({ type: 'form', into: 'x', fields: [{ name: 'c', type: 'choice', of: ['only one'] }] }), undefined, 'a choice needs two options');
  const many = env('', 'shelf');
  many.view.app.blocks = Array.from({ length: 40 }, () => ({ type: 'turn' }));
  assert.equal(m.madeFrom(many).view.blocks.length, m.MAX_BLOCKS);
});

test('view: a `mine` collection reaches the screen and the remote as a count only, the phone as its own rows', () => {
  const phone = viewOf('', 'opening');
  assert.deepEqual(phone.collections.guesses.rows.map((r) => r.values.text), ['Moby-Dick']);
  assert.ok(phone.collections.guesses.rows.every((r) => r.mine));
  for (const pre of ['screen-', 'remote-']) {
    const v = viewOf(pre, 'opening');
    assert.equal(v.collections.guesses.rows, undefined, pre);
    assert.equal(v.collections.guesses.count, 3, pre);
  }
  // The fake box never puts another guest's `mine` row, or a secret before the reveal, in any serialized view.
  for (const pre of ['', 'screen-', 'remote-']) {
    const s = JSON.stringify(env(pre, 'opening'));
    assert.doesNotMatch(s, /Robinson Crusoe|Moby Dick"|Ishmael|thirteen/i, pre);
  }
  assert.match(JSON.stringify(env('screen-', 'opening-revealed')), /Call me Ishmael/);
});

test('timer: seconds left on the box clock; a stopped timer shows its stored seconds', () => {
  const t = { ends_at: 100_000, seconds: 30, running: true };
  assert.equal(m.secondsLeft(t, 78_000), 22);
  assert.equal(m.secondsLeft(t, 78_000, 2_000), 20);
  assert.equal(m.secondsLeft(t, 200_000), 0);
  assert.equal(m.secondsLeft({ ends_at: null, seconds: 12, running: false }, 0), 12);
  assert.equal(m.secondsLeft(null, 0), null);
  assert.deepEqual([0, 9, 61, 600].map(m.clock), ['0:00', '0:09', '1:01', '10:00']);
});

test('phone: the shelf draws say, form and board; a vote and an add send the right actions', async () => {
  const d = draw('', 'shelf');
  assert.deepEqual(types(d.root), ['say', 'form', 'board']);
  assert.match(d.text, /Add a book you would swap/);
  // The tally is the box's: 3 for s1, 1 for s3, and the bar beside each.
  const rows = [...d.root.querySelectorAll('.mopt-row')];
  assert.equal(rows.length, 4);
  assert.deepEqual(rows.map((r) => r.querySelector('.mn').textContent), ['3', '0', '1', '0']);
  assert.equal(rows[0].querySelector('button').getAttribute('aria-pressed'), 'true', 'my pick is pressed');
  assert.equal(d.root.querySelectorAll('svg.mbar').length, 4);
  rows[2].querySelector('button').click();
  await settle();
  assert.deepEqual(d.sent.at(-1), { name: 'vote', body: { collection: 'picks', value: 's3' } });
  // The form: typed drafts go out as values; an empty form asks first and sends nothing.
  const form = d.root.querySelector('form');
  form.dispatchEvent(new dom.window.Event('submit', { cancelable: true }));
  assert.equal(d.sent.length, 1);
  assert.equal(d.ui.errors.get('b1'), 'Fill in at least one box first.');
  d.ui.drafts.set('b1-title', '  Middlemarch ');
  d.ui.drafts.set('b1-genre', 'novel');
  form.dispatchEvent(new dom.window.Event('submit', { cancelable: true }));
  await settle();
  assert.deepEqual(d.sent.at(-1), { name: 'put', body: { collection: 'shelf', values: { title: 'Middlemarch', genre: 'novel' } } });
  assert.equal(d.ui.drafts.has('b1-title'), false, 'a sent draft is cleared');
  // Every input has a label; the choice is a fieldset with a legend.
  for (const i of d.root.querySelectorAll('input:not([type=radio])')) assert.ok(d.root.querySelector(`label[for="${i.id}"]`), i.id);
  assert.equal(d.root.querySelector('fieldset legend').textContent, 'Kind of book');
});

test('phone: a refusal is shown by its block in the box’s words', async () => {
  const d = draw('', 'shelf', { reply: 'This is closed for now.' });
  d.root.querySelector('.mopt-row button').click();
  await settle();
  assert.equal(d.ui.errors.get('b2'), 'This is closed for now.');
});

test('phone: locked lists close the form and the vote; the empty shelf says so', () => {
  const l = draw('', 'shelf-locked');
  assert.equal(l.root.querySelector('form button[type=submit]').textContent, 'Closed for now');
  assert.ok(l.root.querySelector('form button[type=submit]').disabled);
  assert.equal(l.root.querySelectorAll('button.mvote').length, 0, 'no vote buttons while locked');
  const e = draw('', 'shelf-empty');
  assert.match(e.text, /Nothing to vote on yet\./);
});

test('phone: the opening-line game draws every block and sends `answer`', async () => {
  const d = draw('', 'opening');
  assert.deepEqual(types(d.root), ['say', 'timer', 'turn', 'form', 'scores', 'reveal']);
  assert.match(d.text, /Round 1: which book opens like this\?/);
  assert.match(d.root.querySelector('[role=timer]').textContent, /^0:2[12]$/);
  assert.match(d.text, /Your turn/);
  assert.match(d.text, /Then Ana/);
  assert.match(d.text, /Your guess: Moby-Dick/);
  assert.deepEqual([...d.root.querySelectorAll('.mscores li')].map((li) => li.textContent), ['1Ana2', '2You1', '3Ben1', '4Cleo0']);
  assert.match(d.text, /The answers show after the last round\./);
  assert.equal(d.root.querySelectorAll('button').length, 1, 'only the form’s own button: the host’s controls are not on a phone');
  d.ui.drafts.set('b3-text', 'Moby-Dick');
  d.root.querySelector('form').dispatchEvent(new dom.window.Event('submit', { cancelable: true }));
  await settle();
  assert.deepEqual(d.sent, [{ name: 'answer', body: { collection: 'guesses', values: { text: 'Moby-Dick' } } }]);
  const r = draw('', 'opening-revealed');
  assert.deepEqual([...r.root.querySelectorAll('.mreveal li')].map((li) => li.textContent), ['Call me Ishmael.', 'It was a bright cold day in April, and the clocks were striking thirteen.']);
  assert.match(r.text, /Ben's turn/);
});

test('screen: blocks draw with no buttons and no inputs; counts for `mine`', () => {
  for (const scene of ['shelf', 'opening', 'opening-revealed']) {
    const d = draw('screen-', scene);
    assert.equal(d.root.querySelectorAll('button, input, form').length, 0, scene);
  }
  const s = draw('screen-', 'shelf');
  assert.deepEqual(types(s.root), ['say', 'board', 'list']);
  assert.match(s.text, /The Left Hand of Darkness · novel/);
  const o = draw('screen-', 'opening');
  assert.deepEqual(types(o.root), ['say', 'timer', 'turn', 'list', 'scores', 'reveal']);
  assert.match(o.text, /3 guesses in/);
  assert.match(o.text, /Mo's turn/);
  assert.doesNotMatch(o.text, /Your turn|You\b/);
});

test('remote: the host’s buttons follow `can`, and write with host actions', async () => {
  const d = draw('remote-', 'opening');
  assert.deepEqual(types(d.root), ['step', 'timer', 'turn', 'form', 'scores', 'reveal']);
  assert.match(d.text, /Step 2 of 4/);
  const labels = [...d.root.querySelectorAll('button')].map((b) => b.textContent);
  assert.deepEqual(labels, ['Next step', 'Stop', 'Next turn', 'Skip', 'Lock', 'Reveal']);
  assert.match(d.text, /3 guesses in/, 'the form block names its count with its own noun');
  for (const b of d.root.querySelectorAll('button')) b.click();
  await settle();
  assert.deepEqual(d.sent.map((s) => s.name), ['step', 'timer_stop', 'turn_next', 'turn_skip', 'lock', 'reveal']);
  assert.deepEqual(d.sent[4].body, { collection: 'guesses' });
  const r = draw('remote-', 'opening-revealed');
  assert.deepEqual([...r.root.querySelectorAll('button')].map((b) => b.textContent), ['Start', 'Next turn', 'Skip', 'Unlock']);
  assert.equal(r.root.querySelectorAll('button.mvote').length, 0);
});

test('headings are the author’s labels, never a collection key; nouns, hidden counts, count-only, own rows', async () => {
  const d = draw('', 'swap', { reply: 'You can add up to 1 here.' });
  const heads = [...d.root.querySelectorAll('.mbh')].map((e) => e.textContent);
  assert.deepEqual(heads, ['On the shelf', 'Vote for the next read', 'Wish list']);
  assert.doesNotMatch(d.text, /\b(shelf|picks|wishes)\b(?! )/);
  // Own row: "You" and Take back; others' rows have neither.
  const mine = d.root.querySelector('.mrow.me');
  assert.match(mine.textContent, /A Wizard of Earthsea.*You.*Take back/);
  assert.equal(d.root.querySelectorAll('.mrow:not(.me) button').length, 0);
  // No tally from the box: no numbers, the counts stay hidden.
  assert.equal(d.root.querySelectorAll('.mn, .mbar').length, 0);
  assert.match(d.text, /The counts are hidden for now\./);
  assert.doesNotMatch(d.text, /votes? in/, 'a phone holds only its own ballot, so it never says how many are in');
  assert.match(draw('screen-', 'swap').text, /4 votes in\. The counts are hidden for now\./);
  assert.match(d.text, /3 wishes in/);
  // A refusal shows under its block, in the box's words.
  d.root.querySelector('[data-block=form] input').value = 'x';
  d.ui.drafts.set('b1-title', 'Ulysses');
  d.root.querySelector('[data-block=form] form').dispatchEvent(new dom.window.Event('submit', { cancelable: true }));
  await settle();
  assert.equal(d.ui.errors.get('b1'), 'You can add up to 1 here.');
  // The vote line at 0 and 1, and a closed vote on the phone.
  const v = viewOf('', 'swap');
  const line = (n, locked = false) => {
    const c = { v: { ...v, collections: { ...v.collections, picks: { count: n, locked } } }, surface: locked ? 'phone' : 'screen', send: async () => null, ui: m.newUi(() => {}), now: 0, skew: 0 };
    return m.drawBlocks(c).find((e) => e.dataset.block === 'board').textContent;
  };
  assert.match(line(0), /No votes yet\./);
  assert.match(line(1), /1 vote in\.$|1 vote in\.(?!.*hidden)/);
  assert.match(line(2, true), /Voting is closed\./);
  // The remote's reveal without a `hidden` line speaks to the host.
  assert.match(draw('remote-', 'opening').text, /Hidden until you press Reveal\./);
  // A box `rank` lets equal points share a place.
  const e = env('', 'opening-revealed');
  e.view.scores = [{ player: 'g1', points: 2, rank: 1 }, { player: 'g2', points: 2, rank: 1 }, { player: 'g3', points: 1, rank: 3 }];
  const c = { v: m.madeFrom(e).view, surface: 'phone', send: async () => null, ui: m.newUi(() => {}), now: 0, skew: 0 };
  const sc = m.drawBlocks(c).find((x) => x.dataset.block === 'scores');
  assert.deepEqual([...sc.querySelectorAll('.mrank')].map((x) => x.textContent), ['1', '1', '3']);
});

test('hostile guest text is drawn as text, never as markup', () => {
  for (const pre of ['', 'screen-']) {
    const d = draw(pre, 'hostile');
    assert.equal(d.root.querySelectorAll('img, script, svg:not(.px):not(.mbar)').length, 0, pre);
    assert.match(d.text, /<img src=x onerror=alert\(1\)>/);
  }
});

test('app: an unseated phone joins, then draws; an ended pocket says so; a spent screen link says so', async () => {
  const { startFake } = await import('../guest/fake/server.mjs');
  const f = await startFake({ port: 0, actionDelayMs: 0 });
  const base = `http://127.0.0.1:${f.port}`;
  const appEnv = { fetch: (u, o) => fetch(base + u, o), setTimeout, clearTimeout, setInterval, clearInterval, now: () => Date.now(), location: { pathname: '/p/x', protocol: 'http:', host: `127.0.0.1:${f.port}` } };
  try {
    const root = document.createElement('div');
    const a = new m.MadeApp(root, appEnv, 'made-shelf-join', 'phone');
    await a.start(null);
    assert.equal(root.querySelector('main').dataset.screen, 'join');
    assert.match(root.textContent, /Join the book swap/);
    root.querySelector('#mname').value = 'Mo';
    root.querySelector('#mname').dispatchEvent(new dom.window.Event('input'));
    root.querySelector('form').dispatchEvent(new dom.window.Event('submit', { cancelable: true }));
    for (let i = 0; i < 20 && root.querySelector('main').dataset.screen !== 'live'; i++) await new Promise((r) => setTimeout(r, 20));
    assert.equal(root.querySelector('main').dataset.screen, 'live');
    assert.ok(root.querySelector('[data-block=board]'));
    a.live.stop();
    const e = document.createElement('div');
    const b = new m.MadeApp(e, appEnv, 'made-ended', 'phone');
    await b.start(null);
    assert.equal(e.querySelector('main').dataset.screen, 'ended');
    assert.match(e.textContent, /Thanks for swapping!/);
    b.live.stop();
    const s = document.createElement('div');
    const tv = new m.MadeApp(s, appEnv, 'screen-made-shelf', 'screen', 'used');
    await tv.start(null);
    assert.equal(s.querySelector('main').dataset.screen, 'signed-out');
    assert.match(s.textContent, /This big-screen link was already used/);
    assert.match(s.textContent, /press Big screen on its card in kwant\.\s*Links also stop working 10 minutes after they're made\./);
    const rr = document.createElement('div');
    const rm = new m.MadeApp(rr, appEnv, 'remote-made-shelf', 'remote', 'used');
    await rm.start(null);
    assert.match(rr.textContent, /This link was already used or is too old/);
    assert.match(rr.textContent, /press Open the remote again\. Each link works once, for 10 minutes\./);
  } finally {
    await f.close();
  }
});

test('source: the made chunk keeps the guest rules (text nodes only, no style attributes, no hex)', () => {
  const dir = new URL('../guest/src/made/', import.meta.url);
  for (const f of readdirSync(dir)) {
    const s = readFileSync(new URL(f, dir), 'utf8');
    assert.doesNotMatch(s, /innerHTML|outerHTML|insertAdjacentHTML|\.style\b|setAttribute\(\s*['"]style/, f);
    if (f.endsWith('.css')) assert.doesNotMatch(s, /#[0-9a-fA-F]{3,8}\b/, f);
  }
  assert.ok(Object.keys(madeScenes).length >= 6);
});
