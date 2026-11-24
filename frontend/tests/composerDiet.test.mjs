// composer-diet (0.9.50, Mira's simplify review rank 1): one row under the message box, not four.
// a. the Ask row folds into the Answers chip ("Answers: <team> · <lead> leads ▾"), which opens the member list;
// b. the hint line shows only until the first sent message; c. Suggested hides while a reply streams;
// d. the desktop placeholder is "Message {title}", fitted like the phone's. (e) is in firstScreen.test.mjs.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
dom.window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
globalThis.fetch = dom.window.fetch = async () => new Response('[]', { status: 200, headers: { 'Content-Type': 'application/json' } });
globalThis.IS_REACT_ACT_ENVIRONMENT = false;
dom.window.Element.prototype.scrollIntoView = dom.window.Element.prototype.scrollIntoView || function () {};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export * from './src/composer';
  export { ChatView } from './src/components/ChatView';
`);
const h = m.React.createElement;
const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');

const noop = () => {};
const members = [
  { id: 'k1', name: 'Quant', title: 'Lead', avatar_sprite: 'researcher' },
  { id: 'k2', name: 'Backtester', title: 'Tests', avatar_sprite: 'coder' },
  { id: 'k3', name: 'Builder', title: 'Builds', avatar_sprite: 'builder' },
];
const propsOf = (over = {}) =>
  new Proxy(
    {
      title: '@trading-lab',
      subtitle: '',
      messages: [],
      pendingApprovals: [],
      isStreaming: false,
      kwants: members,
      teams: [],
      teamMembers: members,
      teamLeadId: 'k1',
      teamName: 'trading-lab',
      home: { kind: 'team', team: { name: 'trading-lab', purpose: 'Builds and runs the trading bot.', lead: 'Quant', members } },
      canRun: true,
      ...over,
    },
    { get: (t, k) => (k in t ? t[k] : typeof k === 'string' && k.startsWith('on') ? noop : undefined) },
  );
const html = (over) => m.renderToStaticMarkup(h(m.ChatView, propsOf(over)));
const text = (s) => s.replace(/<[^>]+>/g, '').replace(/\s+/g, ' ');

// ------------------------------------------------------------------ a: one "who answers" control

test('a: no Ask row; one Answers chip says who answers and who leads, and is a button that opens a list', () => {
  localStorage.clear();
  const out = html();
  assert.doesNotMatch(out, /aria-label="Ask a member"|>Ask:</, 'the Ask row is gone');
  assert.doesNotMatch(chat, /aria-label="Ask a member"|data-team-chip/);
  const at = out.indexOf('data-answers-chip=""');
  assert.ok(at > 0, 'the chip renders');
  const chip = out.slice(out.lastIndexOf('<button', at), out.indexOf('</button>', at));
  assert.match(chip, /aria-haspopup="listbox"/);
  assert.match(chip, /aria-expanded="false"/);
  assert.match(chip, /aria-label="Answers: trading-lab team, Quant leads\. Choose who answers"/);
  assert.match(text(chip), /Answers:\s*trading-lab team\s*Quant leads/);
  assert.match(chip, /data-answer-lead=""/, 'the lead rides in the chip');
  // The lead's pixel avatar, not a generic icon.
  assert.match(chip.slice(chip.indexOf('data-answer-lead')), /<(img|canvas|svg|span|div)[^>]*(data-avatar|kw-avatar|aria-hidden)/);
});

test('a: no lead known, or an @mention typed: plain "Answers: <who>" without "leads"', () => {
  localStorage.clear();
  const noLead = html({ teamLeadId: null });
  assert.doesNotMatch(noLead, /data-answer-lead/);
  assert.match(noLead, /aria-label="Answers: trading-lab team\. Choose who answers"/);
  assert.equal(m.answersChip({ who: 'Quant', lead: null }).name, 'Answers: Quant. Choose who answers');
  assert.equal(m.answersChip({ who: 'trading-lab team', lead: '  Quant ' }).lead, 'Quant');
});

test('a: the chip keeps a 2 px sky-400 focus ring with a 2 px offset, 44 px to tap on a phone, and no motion under reduced motion', () => {
  const block = chat.slice(chat.indexOf('data-answers-chip=""'), chat.indexOf('</button>', chat.indexOf('data-answers-chip=""')));
  const cls = chat.slice(chat.lastIndexOf('className={`kw-chip', chat.indexOf('data-answers-chip=""')), chat.indexOf('data-answers-chip=""'));
  assert.match(cls, /focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400/);
  assert.match(cls, /\$\{phone \? 'kws-hit h-8' : ''\}/, 'drawn 32 px on a phone, kws-hit makes it 48 to the touch');
  assert.match(block, /transition-transform duration-150 motion-reduce:transition-none/);
  assert.doesNotMatch(cls, /#[0-9a-fA-F]{3,6}/, 'no hard-coded hex');
});

// Poll, not a fixed wait (Rigel F1): the list opens a frame after the click; under load that frame can be late.
const until = async (ok, ms = 1000) => {
  for (const end = Date.now() + ms; Date.now() < end; await new Promise((r) => setTimeout(r, 10))) if (ok()) return true;
  return ok();
};
const listOpen = async (host) => assert.ok(await until(() => !!host.querySelector('#kw-mention-list')), 'the member list opens');
const listClosed = async (host) => assert.ok(await until(() => !host.querySelector('#kw-mention-list')), 'the member list closes');

test('a: clicking the chip opens the member list (the same one @ opens), keyboard-reachable from the field', async () => {
  localStorage.clear();
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(h(m.ChatView, propsOf())));
  const chip = host.querySelector('[data-answers-chip]');
  assert.ok(chip, 'chip mounted');
  chip.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await listOpen(host);
  const field = host.querySelector('textarea[data-composer]');
  assert.equal(field.value, '@', 'the field gets the @ the list filters on');
  assert.equal(document.activeElement, field, 'focus goes to the field, where arrows and Enter pick a member');
  const list = host.querySelector('#kw-mention-list');
  assert.ok(list, 'the member list is open');
  for (const n of ['Quant', 'Backtester', 'Builder']) assert.match(list.textContent, new RegExp(n));
  assert.equal(field.getAttribute('aria-expanded'), 'true');
  assert.equal(host.querySelector('[data-answers-chip]').getAttribute('aria-expanded'), 'true');
  root.unmount();
  host.remove();
});

// ------------------------------------------------------------------ b: the hint line until the first send

test('b: the hint line shows until the first sent message, then never again (localStorage flag)', () => {
  const store = new Map();
  const s = { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v) };
  assert.equal(m.hintLearned(s), false);
  assert.equal(m.hintLineShows({ phone: false, touch: false, learned: false }), true);
  m.markHintLearned(s);
  assert.equal(store.get(m.HINT_KEY), '1');
  assert.equal(m.HINT_KEY, 'kwnt:composer-hint-done');
  assert.equal(m.hintLearned(s), true);
  assert.equal(m.hintLineShows({ phone: false, touch: false, learned: true }), false);
  assert.equal(m.hintLineShows({ phone: true, touch: true, learned: false }), false, 'never on a touch phone');
  // Storage that throws counts as "not yet" and never breaks the composer.
  const bad = { getItem() { throw new Error('denied'); }, setItem() { throw new Error('denied'); } };
  assert.equal(m.hintLearned(bad), false);
  assert.doesNotThrow(() => m.markHintLearned(bad));
});

test('b: the real composer shows the hint before the first send and hides it once the key is set', () => {
  localStorage.clear();
  assert.match(text(html()), /mention a bot · Enter send/);
  localStorage.setItem('kwnt:composer-hint-done', '1');
  assert.doesNotMatch(text(html()), /mention a bot · Enter send/);
  assert.match(html(), /data-answers-chip/, 'the Answers chip stays');
  localStorage.clear();
});

test('b: sending a message sets the key and the hint goes in the same render', async () => {
  localStorage.clear();
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  let sent = 0;
  m.flushSync(() => root.render(h(m.ChatView, propsOf({ onSendMessage: () => { sent++; return true; } }))));
  assert.match(host.textContent, /mention a bot · Enter send/);
  const field = host.querySelector('textarea[data-composer]');
  const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set;
  setter.call(field, 'hello');
  field.dispatchEvent(new Event('input', { bubbles: true }));
  await new Promise((r) => setTimeout(r, 20));
  field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(sent, 1, 'the message went');
  assert.equal(localStorage.getItem('kwnt:composer-hint-done'), '1');
  assert.doesNotMatch(host.textContent, /mention a bot · Enter send/);
  root.unmount();
  host.remove();
  localStorage.clear();
});

// ------------------------------------------------------------------ c: Suggested while streaming

test('c: Suggested hides while a reply streams', () => {
  const base = { messages: 1, suggestions: 3, showHome: false, canRun: true, streaming: false };
  assert.equal(m.suggestedShows(base), true);
  assert.equal(m.suggestedShows({ ...base, streaming: true }), false);
  assert.equal(m.suggestedShows({ ...base, canRun: false }), false);
  assert.equal(m.suggestedShows({ ...base, canRun: null }), true, 'not known yet still shows');
  assert.equal(m.suggestedShows({ ...base, messages: 2 }), false);
  const msg = [{ id: 'm1', sender_role: 'user', sender_id: 'u', text: 'hi', created_at: new Date().toISOString() }];
  const over = { messages: msg, suggestions: ['Write a test plan', 'Draft the API'], home: null };
  assert.match(html(over), /aria-label="Suggested prompts"/, 'shown when idle');
  assert.doesNotMatch(html({ ...over, isStreaming: true, streamingSpeakerName: 'Quant' }), /aria-label="Suggested prompts"/, 'gone while streaming');
});

// ------------------------------------------------------------------ d: the desktop placeholder

test('d: desktop placeholder is "Message {title}" when it fits, "Message" when not; never " in {space}"', () => {
  localStorage.clear();
  const out = html({ spaceName: 'Binance desk', title: 'Quant', home: null });
  assert.match(out, /placeholder="Message"|placeholder="Message Quant"/);
  assert.doesNotMatch(out, / in Binance desk"/);
  assert.doesNotMatch(chat, /Message \$\{title\} in \$\{spaceName\}/);
  const measure = (t) => t.length * 9;
  assert.equal(m.phonePlaceholder('@trading-lab', 600, measure), 'Message @trading-lab');
  assert.equal(m.phonePlaceholder('@a-very-long-team-name-that-will-not-fit', 200, measure), 'Message');
  // Desktop measures the field too (it used to measure on phones only).
  assert.match(chat, /useEffect\(\(\) => \{\n    const el = textareaRef\.current;\n    if \(!el\) return;\n    const measure = \(\) => setFieldWidth\(el\.clientWidth\);/);
});

test('a: closing the chip\'s list with no pick takes its "@" back; a second tap closes the list (Mira F1, Rigel F2)', async () => {
  localStorage.clear();
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(h(m.ChatView, propsOf())));
  const field = () => host.querySelector('textarea[data-composer]');
  const chip = () => host.querySelector('[data-answers-chip]');
  // Escape with nothing picked: the field is empty again, not "@".
  chip().dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await listOpen(host);
  assert.equal(field().value, '@');
  field().dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await listClosed(host);
  assert.ok(await until(() => field().value === ''), `the stray @ goes (got ${JSON.stringify(field().value)})`);
  // A second tap on the chip closes the list instead of adding "@ @", and the "@" goes too.
  chip().dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await listOpen(host);
  chip().dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await listClosed(host);
  assert.ok(await until(() => field().value === ''), `no "@ @" (got ${JSON.stringify(field().value)})`);
  assert.equal(chip().getAttribute('aria-expanded'), 'false');
  // A pick keeps the name.
  chip().dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await listOpen(host);
  field().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
  await listClosed(host);
  assert.ok(await until(() => /^@Quant $/.test(field().value)), `the pick stays (got ${JSON.stringify(field().value)})`);
  root.unmount();
  host.remove();
});

test('a: the chip\'s 14 px lead face rests (no working ring running into "leads")', () => {
  const src = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  const lead = src.slice(src.indexOf('data-answer-lead'), src.indexOf('data-answer-lead') + 400);
  assert.match(lead, /size=\{14\} kwantId=\{lead\.id\} mood="idle"/);
});

test('a: F1b, the chip\'s "@" goes on a tap outside and when typed words close the list; typed names and the @ button stay', async () => {
  localStorage.clear();
  const host = document.createElement('div');
  document.body.appendChild(host);
  const outside = document.createElement('button');
  document.body.appendChild(outside);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(h(m.ChatView, propsOf())));
  const field = () => host.querySelector('textarea[data-composer]');
  const chip = () => host.querySelector('[data-answers-chip]');
  const PE = window.PointerEvent ?? window.MouseEvent;
  const tapOutside = () => outside.dispatchEvent(new PE('pointerdown', { bubbles: true }));
  const setField = (v) => {
    const el = field();
    Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set.call(el, v);
    el.setSelectionRange(v.length, v.length);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  };
  const is = async (want, why) => assert.ok(await until(() => field().value === want), `${why}: want ${JSON.stringify(want)}, got ${JSON.stringify(field().value)}`);

  // "" → chip → tap outside → "", list closed.
  chip().dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await listOpen(host);
  tapOutside();
  await listClosed(host);
  await is('', 'tap outside');
  // "hello" → chip → tap outside → "hello".
  setField('hello');
  await is('hello', 'typed');
  chip().dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await listOpen(host);
  await is('hello @', 'chip');
  tapOutside();
  await listClosed(host);
  await is('hello', 'tap outside keeps the words');
  // "" → chip → type "Back up" → "Back up" (Back matches Backtester; the space closes the list).
  setField('');
  await is('', 'cleared');
  chip().dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await listOpen(host);
  setField('@Back');
  setField('@Back up');
  await listClosed(host);
  await is('Back up', 'words that are not a name drop only the chip\'s @');
  // "" → chip → type "zzz" (nobody) → "zzz".
  setField('');
  chip().dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await listOpen(host);
  setField('@zzz');
  await listClosed(host);
  await is('zzz', 'no match');
  // "" → chip → type a whole name and a space → the mention stays.
  setField('');
  chip().dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await listOpen(host);
  setField('@Quant ');
  await listClosed(host);
  await is('@Quant ', 'a typed name is kept');
  // "" → chip → "Qu" → Escape → "@Qu" kept.
  setField('');
  chip().dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await listOpen(host);
  setField('@Qu');
  field().dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await listClosed(host);
  await is('@Qu', 'Escape after typing keeps the typed name');
  // A hand-typed "@" → tap outside: unchanged from today (list stays, "@" stays).
  setField('@');
  await listOpen(host);
  tapOutside();
  await new Promise((r) => setTimeout(r, 50));
  await is('@', 'hand-typed @ untouched');
  root.unmount();
  host.remove();
  outside.remove();
});
