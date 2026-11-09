// 0.8.0 P4 (app half): pictures a bot shares show as that bot's, under its reply, without repeats.
// The chat replays Oleks's 2026-11-09 thread: share_file rows stored with the bot's uuid, posted before
// the reply, the same file three times. The chat's title is the space ("@booktun.ing"), never a sender.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'CSS']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
if (!g.CSS) g.CSS = { escape: (s) => s };
g.window.matchMedia = g.window.matchMedia || ((q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} }));
g.ResizeObserver = g.window.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
g.IntersectionObserver = g.window.IntersectionObserver = class { observe() {} unobserve() {} disconnect() {} };
g.window.HTMLElement.prototype.scrollIntoView = function () {};
g.fetch = g.window.fetch = async () => new Response('[]', { status: 200, headers: { 'Content-Type': 'application/json' } });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export * from './src/sharedFiles';
  export { ChatView } from './src/components/ChatView';
`);
const { React, createRoot, flushSync } = m;
const h = React.createElement;
const settle = () => new Promise((r) => setImmediate(r));

const FRONTEND = '7b2657fd-54d1-465b-ab37-5269eb1ed456';
const GROWTH = '0e7bf95d-cb8b-4396-a0d7-bcf14a38ab47';
const GONE = '4c6677dc-533e-4706-8028-4afa492d95a4';
const bots = [
  { id: FRONTEND, name: 'Frontend', title: 'Frontend engineer', avatar_sprite: 'dev' },
  { id: GROWTH, name: 'Growth', title: 'Growth', avatar_sprite: 'marketer' },
];
let n = 0;
const fileId = () => `00000000-0000-4000-8000-${String(++n).padStart(12, '0')}`;
const at = (hm, s = 0) => `2026-11-09T${hm}:${String(s).padStart(2, '0')}Z`;
const you = (id, text, when) => ({ id, sender_role: 'user', sender_id: 'Oleks', text, created_at: when });
const reply = (id, sender, text, when) => ({ id, sender_role: 'kwant', sender_id: sender, text, created_at: when });
// Real, differing sizes: each name has its own size, so only a true re-share of the same bytes matches.
const sizeOf = (name) => 40000 + [...name].reduce((t, c, i) => t + c.charCodeAt(0) * (i + 1), 0);
const share = (id, sender, name, when, size = sizeOf(name)) => ({
  id,
  sender_role: 'kwant',
  sender_id: sender,
  text: `Shared ${name}`,
  created_at: when,
  meta: JSON.stringify({ kind: 'file', attachments: [{ id: fileId(), name, mime: 'image/jpeg', size }] }),
});

// ---- names --------------------------------------------------------------------------------------

test('a uuid sender resolves to its bot; an unknown one is "A bot", never the chat title', () => {
  assert.equal(m.botDisplayName(FRONTEND, bots), 'Frontend');
  assert.equal(m.botDisplayName('Frontend', bots), 'Frontend', 'replies store the name');
  assert.equal(m.botDisplayName(GONE, bots), 'A bot');
  assert.equal(m.botDisplayName(GONE, []), 'A bot', 'list not loaded yet');
  assert.equal(m.botDisplayName('', bots), 'A bot');
  assert.equal(m.botDisplayName('Laya', bots), 'Laya', 'a short stored name stays');
  assert.equal(m.botOf(FRONTEND, bots).id, FRONTEND);
  assert.equal(m.botOf('Growth', bots).id, GROWTH);
  assert.equal(m.botOf(GONE, bots), null);
});

// ---- placement ----------------------------------------------------------------------------------

test("a turn's shares move right under that bot's reply; other bots' and later turns stay put", () => {
  const msgs = [
    you('u1', 'Show mw screenshot in chat!!', at('10:25')),
    share('f1', FRONTEND, 'find-your-book-bar.jpg', at('10:26', 5)),
    reply('g1', 'Growth', 'Not mine.', at('10:26', 10)),
    reply('r1', 'Frontend', 'I took a fresh screenshot and shared it in this chat', at('10:26', 30)),
    you('u2', 'why it not aligned?', at('10:27')),
    share('f2', FRONTEND, 'lonely.png', at('10:27', 30)),
  ];
  const a = m.arrangeSharedFiles(msgs, bots);
  assert.deepEqual(a.messages.map((x) => x.id), ['u1', 'g1', 'r1', 'f1', 'u2', 'f2']);
  assert.equal(a.attached.has('f1'), true);
  assert.equal(a.attached.has('f2'), false, 'no reply after it in its turn: it keeps its own header');
  assert.equal(a.earlier.size, 0);
});

test('a share saved after the reply is attached in place; a share never jumps into another turn', () => {
  const msgs = [
    you('u1', 'go', at('10:00')),
    reply('r1', 'Frontend', 'Here it is', at('10:00', 20)),
    share('f1', FRONTEND, 'a.png', at('10:00', 21)),
    you('u2', 'thanks', at('10:01')),
    reply('r2', 'Frontend', 'ok', at('10:01', 5)),
  ];
  const a = m.arrangeSharedFiles(msgs, bots);
  assert.deepEqual(a.messages.map((x) => x.id), ['u1', 'r1', 'f1', 'u2', 'r2']);
  assert.equal(a.attached.has('f1'), true);
});

// ---- repeats ------------------------------------------------------------------------------------

test('identical repeats collapse into the newest; a different file in between does not stop it', () => {
  const msgs = [
    you('u1', 'the button is not aligned!!!!', at('10:27')),
    share('f1', FRONTEND, 'find-your-book-bar.jpg', at('10:28', 0)),
    share('f2', FRONTEND, 'find-your-book-bar-dark.jpg', at('10:28', 10)),
    share('f3', FRONTEND, 'find-your-book-bar.jpg', at('10:28', 20)),
    share('f4', FRONTEND, 'find-your-book-bar.jpg', at('10:28', 40)),
    reply('r1', 'Frontend', 'Restyled from styles.css', at('10:28', 50)),
  ];
  const a = m.arrangeSharedFiles(msgs, bots);
  assert.deepEqual(a.messages.map((x) => x.id), ['u1', 'r1', 'f2', 'f4']);
  assert.deepEqual(a.earlier.get('f4').map((x) => x.id), ['f1', 'f3'], 'oldest first');
  assert.equal(a.attached.has('f2') && a.attached.has('f4'), true);
});

test('the same bytes fold; the same name with an edited picture does not (Mira 1)', () => {
  const msgs = [
    you('u1', 'Fix it and re-shoot', at('10:40')),
    share('s1', FRONTEND, 'screenshot-example.com.jpg', at('10:40', 5), 51234),
    share('s2', FRONTEND, 'screenshot-example.com.jpg', at('10:40', 15), 51234),
    share('s3', FRONTEND, 'screenshot-example.com.jpg', at('10:41', 0), 49870),
    reply('r1', 'Frontend', 'Fixed, here is before and after', at('10:41', 10)),
  ];
  const a = m.arrangeSharedFiles(msgs, bots);
  assert.deepEqual(a.messages.map((x) => x.id), ['u1', 'r1', 's2', 's3'], 'the edited picture stays its own tile');
  assert.deepEqual(a.earlier.get('s2').map((x) => x.id), ['s1'], 'the identical copy folds');
  assert.equal(a.earlier.has('s3'), false);
});

test('repeats need the same bot, the same name and a few minutes', () => {
  const msgs = [
    you('u1', 'go', at('10:00')),
    share('a1', FRONTEND, 'x.png', at('10:00', 0)),
    share('a2', GROWTH, 'x.png', at('10:00', 30)),
    share('a3', FRONTEND, 'x.png', at('10:20', 0)),
  ];
  const a = m.arrangeSharedFiles(msgs, bots);
  assert.deepEqual(a.messages.map((x) => x.id), ['u1', 'a1', 'a2', 'a3'], 'another bot breaks the run; 20 min apart is not a repeat');
  assert.equal(a.earlier.size, 0);
});

test('an id row and a name row of the same bot count as the same sender', () => {
  const msgs = [you('u1', 'go', at('10:00')), share('a1', FRONTEND, 'x.png', at('10:00', 0)), share('a2', 'Frontend', 'x.png', at('10:00', 9))];
  const a = m.arrangeSharedFiles(msgs, bots);
  assert.deepEqual(a.messages.map((x) => x.id), ['u1', 'a2']);
  assert.deepEqual(a.earlier.get('a2').map((x) => x.id), ['a1']);
});

// ---- ChatView -----------------------------------------------------------------------------------

function mount(messages, extra = {}) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  const props = {
    title: '@booktun.ing',
    messages,
    streamingText: '',
    streamingThinking: '',
    pendingApprovals: [],
    isStreaming: false,
    onSendMessage: () => true,
    onResolveApproval: () => {},
    contextKey: 'space-1',
    kwants: bots,
    ...extra,
  };
  flushSync(() => root.render(h(m.ChatView, props)));
  return { host, unmount: () => { flushSync(() => root.unmount()); host.remove(); } };
}
const rowIds = (host) => [...host.querySelectorAll('[role="log"] [data-message-id]')].map((e) => e.getAttribute('data-message-id'));

test("ChatView: the old uuid rows show the bot's name, never the chat title, and sit under its reply", async () => {
  const msgs = [
    you('u1', 'Show mw screenshot in chat!!', at('10:25')),
    share('f1', FRONTEND, 'find-your-book-bar.jpg', at('10:26', 5)),
    share('f9', GONE, 'mystery.png', at('10:26', 6)),
    reply('r1', 'Frontend', 'I took a fresh screenshot and shared it in this chat', at('10:26', 30)),
  ];
  const v = mount(msgs);
  await settle();
  const log = v.host.querySelector('[role="log"]');
  assert.deepEqual(rowIds(v.host), ['u1', 'f9', 'r1', 'f1'], "the Frontend picture follows Frontend's reply; the unknown one stays");
  const f1 = log.querySelector('[data-message-id="f1"]');
  assert.equal(f1.hasAttribute('data-shared-under'), true);
  assert.match(f1.textContent, /Shared by Frontend/);
  assert.ok(f1.querySelector('img[alt="find-your-book-bar.jpg, shared by Frontend"]'), "the image names its bot in the alt text");
  const f9 = log.querySelector('[data-message-id="f9"]');
  assert.match(f9.textContent, /A bot/);
  assert.doesNotMatch(log.textContent, /@booktun\.ing/, 'the chat title never names a sender');
  v.unmount();
});

test('ChatView: three copies show once, with "Shared 3 times · Show 2 earlier" opening the older two', async () => {
  const msgs = [
    you('u1', 'Fix differences!', at('10:29')),
    share('f1', FRONTEND, 'find-your-book-bar.jpg', at('10:29', 10)),
    share('f2', FRONTEND, 'find-your-book-bar.jpg', at('10:29', 40)),
    share('f3', FRONTEND, 'find-your-book-bar.jpg', at('10:30', 0)),
    reply('r1', 'Frontend', 'Added the pixel edge and the card shadows.', at('10:30', 20)),
  ];
  const v = mount(msgs);
  await settle();
  assert.deepEqual(rowIds(v.host), ['u1', 'r1', 'f3']);
  assert.equal(v.host.querySelectorAll('[role="log"] img[alt="find-your-book-bar.jpg, shared by Frontend"]').length, 1);
  const toggle = v.host.querySelector('[data-earlier-copies] button');
  assert.equal(toggle.textContent, 'Shared 3 times · Show 2 earlier');
  assert.equal(toggle.getAttribute('aria-label'), 'Show 2 earlier copies of find-your-book-bar.jpg');
  assert.equal(toggle.getAttribute('aria-expanded'), 'false');
  flushSync(() => toggle.click());
  assert.equal(toggle.getAttribute('aria-expanded'), 'true');
  assert.equal(toggle.textContent, 'Hide earlier');
  assert.deepEqual([...v.host.querySelectorAll('[data-earlier-copy]')].map((e) => e.getAttribute('data-earlier-copy')), ['f1', 'f2']);
  assert.equal(v.host.querySelectorAll('[role="log"] img[alt="find-your-book-bar.jpg, shared by Frontend"]').length, 3);
  v.unmount();
});

test('ChatView: a click opens the picture full size; Esc closes it and focus returns', async () => {
  const msgs = [you('u1', 'show me', at('10:25')), reply('r1', 'Frontend', 'Here', at('10:26')), share('f1', FRONTEND, 'bar.jpg', at('10:26', 2))];
  const v = mount(msgs);
  await settle();
  const thumb = v.host.querySelector('[data-image-thumb]');
  assert.equal(thumb.getAttribute('aria-label'), 'View bar.jpg full size');
  flushSync(() => thumb.dispatchEvent(new window.MouseEvent('click', { bubbles: true, cancelable: true, button: 0 })));
  const dlg = document.querySelector('[data-image-viewer]');
  assert.ok(dlg, 'viewer open');
  assert.equal(dlg.getAttribute('role'), 'dialog');
  assert.equal(dlg.getAttribute('aria-modal'), 'true');
  assert.ok(dlg.querySelector('img[alt="bar.jpg, shared by Frontend"]'));
  assert.match(dlg.className, /bg-black\/85/, 'scrim is black at 85%');
  assert.equal(dlg.style.height, '100dvh');
  assert.equal(dlg.querySelector('img').style.touchAction, 'pinch-zoom', 'pinch-zoom left to the browser');
  assert.match(dlg.textContent, /bar\.jpg · Frontend · \S/, 'bottom bar: file, bot, time');
  const orig = [...dlg.querySelectorAll('a')].find((a) => a.textContent.includes('Open original'));
  assert.equal(orig.getAttribute('target'), '_blank');
  assert.equal(orig.getAttribute('rel'), 'noopener');
  assert.equal(document.body.style.overflow, 'hidden', 'page behind does not scroll');
  assert.equal(document.activeElement.getAttribute('aria-label'), 'Close');
  assert.match(document.activeElement.className, /w-11 h-11/, 'Close is 44 px');
  flushSync(() => document.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
  assert.equal(document.querySelector('[data-image-viewer]'), null);
  assert.equal(document.body.style.overflow, '', 'scroll lock released');
  assert.equal(document.activeElement, thumb);
  // a tap on the scrim closes it too
  flushSync(() => thumb.dispatchEvent(new window.MouseEvent('click', { bubbles: true, cancelable: true, button: 0 })));
  const dlg2 = document.querySelector('[data-image-viewer]');
  flushSync(() => dlg2.dispatchEvent(new window.MouseEvent('click', { bubbles: true })));
  assert.equal(document.querySelector('[data-image-viewer]'), null);
  // Cmd/Ctrl click keeps the browser's new tab
  const ev = new window.MouseEvent('click', { bubbles: true, cancelable: true, button: 0, ctrlKey: true });
  thumb.addEventListener('click', (e) => e.preventDefault(), { once: true });
  flushSync(() => thumb.dispatchEvent(ev));
  assert.equal(document.querySelector('[data-image-viewer]'), null);
  v.unmount();
});

test("ChatView: in a bot's own chat an id row resolves to that bot even before the list loads", async () => {
  const home = { kind: 'bot', bot: { id: FRONTEND, name: 'Frontend', title: 'Frontend engineer', avatar_sprite: 'dev' } };
  const v = mount([you('u1', 'x', at('10:00')), share('f1', FRONTEND, 'a.png', at('10:00', 5))], { kwants: [], home, title: 'Frontend' });
  await settle();
  assert.match(v.host.querySelector('[data-message-id="f1"]').textContent, /Frontend/);
  assert.doesNotMatch(v.host.querySelector('[data-message-id="f1"]').textContent, /A bot/);
  v.unmount();
});

// ---- the server's new shape (vega-080-share-screens) --------------------------------------------

const newShare = (id, bot, name, when, auto = false) => ({
  id,
  sender_role: 'kwant',
  sender_id: bot.name,
  text: `Shared a file: ${name}`,
  created_at: when,
  meta: JSON.stringify({ kind: 'file', attachments: [{ id: fileId(), name, mime: 'image/jpeg', size: sizeOf(name) }], kwant_id: bot.id, kwant_name: bot.name, turn_id: 't1', auto }),
});

test("shareInfo reads kwant_id and auto; a new-shape share follows its bot's reply", () => {
  const s = newShare('n1', bots[0], 'screenshot-booktun.ing.jpg', at('11:00'), true);
  assert.deepEqual(m.shareInfo(s.meta), { kwantId: FRONTEND, auto: true });
  assert.deepEqual(m.shareInfo(null), { kwantId: null, auto: false });
  assert.deepEqual(m.shareInfo('{"kind":"file","auto":"yes"}'), { kwantId: null, auto: false }, 'only a real true counts');
  // A share named after its bot, with the id in meta, follows that bot's reply (by id).
  const msgs = [you('u1', 'show me', at('11:00')), newShare('n2', bots[0], 'screenshot.jpg', at('11:00', 5)), reply('r1', FRONTEND, 'Here it is', at('11:00', 9))];
  assert.deepEqual(m.arrangeSharedFiles(msgs, bots).messages.map((x) => x.id), ['u1', 'r1', 'n2']);
});

test('ChatView: a new-shape share shows its bot, and an automatic one says "Shared automatically"', async () => {
  const msgs = [
    you('u1', 'Show me the page', at('11:10')),
    reply('r1', 'Growth', 'Here is the page I opened.', at('11:10', 20)),
    newShare('n1', bots[1], 'screenshot-booktun.ing.jpg', at('11:10', 21), true),
    you('u2', 'and the other', at('11:11')),
    newShare('n2', bots[1], 'screenshot.jpg', at('11:11', 5)),
    reply('r2', 'Growth', 'Shared above.', at('11:11', 9)),
  ];
  const v = mount(msgs);
  await settle();
  const log = v.host.querySelector('[role="log"]');
  assert.deepEqual(rowIds(v.host), ['u1', 'r1', 'n1', 'u2', 'r2', 'n2']);
  const n1 = log.querySelector('[data-message-id="n1"]');
  assert.equal(n1.hasAttribute('data-shared-under'), true);
  assert.match(n1.textContent, /Shared by Growth/);
  assert.ok(n1.querySelector('[data-shared-auto]'), 'auto share has the hint');
  assert.match(n1.querySelector('[data-shared-auto]').textContent, /^Shared automatically$/);
  assert.equal(log.querySelector('[data-message-id="n2"] [data-shared-auto]'), null, 'a share the bot made itself has no hint');
  v.unmount();
});

// ---- the running turn (Mira: no jump) -----------------------------------------------------------

test("arrange: while a bot's turn runs, its loose shares go to the live row; others' stay", () => {
  const msgs = [
    you('u0', 'earlier', at('10:00')),
    share('old', FRONTEND, 'old.jpg', at('10:00', 5)),
    you('u1', 'Show it', at('10:25')),
    share('f1', FRONTEND, 'bar.jpg', at('10:26', 5)),
    share('g1', GROWTH, 'chart.png', at('10:26', 6)),
  ];
  const a = m.arrangeSharedFiles(msgs, bots, 'Frontend');
  assert.deepEqual(a.live.map((x) => x.id), ['f1']);
  assert.deepEqual(a.messages.map((x) => x.id), ['u0', 'old', 'u1', 'g1'], 'an earlier turn never moves');
  assert.deepEqual(m.arrangeSharedFiles(msgs, bots).live, []);
});

test("ChatView: a share during the turn shows under the bot's live row, then stays put when the reply lands", async () => {
  const base = [you('u1', 'Show mw screenshot in chat!!', at('10:25')), share('f1', FRONTEND, 'bar.jpg', at('10:26', 5))];
  const v = mount(base, { isStreaming: true, streamingSpeakerName: 'Frontend', streamingText: 'Taking a screenshot' });
  await settle();
  const ids = rowIds(v.host);
  assert.deepEqual(ids, ['u1', 'f1']);
  const f1 = v.host.querySelector('[data-message-id="f1"]');
  assert.equal(f1.hasAttribute('data-shared-under'), true, 'drawn as part of the live row, no second header');
  // it sits after the live bubble, not before it
  const live = [...v.host.querySelectorAll('[role="log"] > *')];
  const liveIdx = live.findIndex((e) => e.textContent.includes('Taking a screenshot'));
  assert.ok(liveIdx >= 0 && live.indexOf(f1) > liveIdx, 'after the streaming bubble');
  v.unmount();
  const done = mount([...base, reply('r1', 'Frontend', 'Shared it', at('10:26', 30))]);
  await settle();
  assert.deepEqual(rowIds(done.host), ['u1', 'r1', 'f1']);
  assert.equal(done.host.querySelector('[data-message-id="f1"]').hasAttribute('data-shared-under'), true);
  done.unmount();
});

// ---- rework after review (Ada A, Mira 2-5, 7, 8; two bots) --------------------------------------

test('ChatView: the open viewer keeps focus where the person put it while a reply streams (Ada A)', async () => {
  const msgs = [you('u1', 'show me', at('10:25')), reply('r1', 'Frontend', 'Here', at('10:26')), share('f1', FRONTEND, 'bar.jpg', at('10:26', 2))];
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  const props = (text) => ({
    title: '@booktun.ing', messages: msgs, streamingText: text, streamingThinking: '', pendingApprovals: [], isStreaming: true,
    streamingSpeakerName: 'Growth', onSendMessage: () => true, onResolveApproval: () => {}, contextKey: 'space-1', kwants: bots,
  });
  flushSync(() => root.render(h(m.ChatView, props('Look'))));
  await settle();
  const thumb = host.querySelector('[data-image-thumb]');
  flushSync(() => thumb.dispatchEvent(new window.MouseEvent('click', { bubbles: true, cancelable: true, button: 0 })));
  const dlg = document.querySelector('[data-image-viewer]');
  const orig = [...dlg.querySelectorAll('a')].find((a) => a.textContent.includes('Open original'));
  orig.focus();
  assert.equal(document.activeElement, orig);
  for (const t of ['Looking', 'Looking at', 'Looking at the page']) flushSync(() => root.render(h(m.ChatView, props(t))));
  await settle();
  assert.ok(document.querySelector('[data-image-viewer]'), 'still open');
  assert.equal(document.activeElement, orig, 'new tokens do not pull focus back to Close');
  flushSync(() => document.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
  assert.equal(document.querySelector('[data-image-viewer]'), null, 'Esc still closes after re-renders');
  assert.equal(document.activeElement, host.querySelector('[data-image-thumb]'));
  flushSync(() => root.unmount());
  host.remove();
});

test('ChatView: tap targets and focus ring on shared tiles (Mira 2, 3, 4, 5, 7, 8)', async () => {
  const msgs = [
    you('u1', 'go', at('10:29')),
    share('f1', FRONTEND, 'find-your-book-bar.jpg', at('10:29', 10)),
    share('f2', FRONTEND, 'find-your-book-bar.jpg', at('10:29', 40)),
    reply('r1', 'Frontend', 'Done', at('10:30', 20)),
  ];
  const v = mount(msgs);
  await settle();
  const toggle = v.host.querySelector('[data-earlier-copies] button');
  assert.match(toggle.className, /(^|\s)py-3\.5(\s|$)/, '44 px tall on phone (16 px line + 2 x 14 px)');
  assert.match(toggle.className, /sm:py-1/);
  const tile = v.host.querySelector('[data-message-id="f2"] figure');
  const dl = tile.querySelector('[data-download]');
  assert.match(dl.className, /w-8 h-8/, 'icon-only 32 px chip on a shared tile');
  assert.match(dl.className, /before:-inset-1\.5/, '32 + 2 x 6 = 44 px hit area');
  assert.equal(dl.getAttribute('aria-label'), 'Download find-your-book-bar.jpg');
  assert.equal(dl.getAttribute('title'), 'Download');
  assert.match(dl.querySelector('span').className, /sr-only/);
  const thumb = tile.querySelector('[data-image-thumb]');
  assert.match(thumb.className, /focus-visible:outline-offset-\[-2px\]/, 'ring drawn inside the clipped figure');
  assert.match(thumb.className, /focus-visible:outline-violet-400/);
  assert.match(tile.parentElement.className, /w-full sm:w-auto/, 'full bubble width on phone');
  flushSync(() => toggle.click());
  const small = v.host.querySelector('[data-earlier-copy="f1"] figure');
  assert.match(small.querySelector('img').className, /w-full h-24/, 'small copy fills its tile');
  assert.doesNotMatch(small.querySelector('img').className, /max-h-24|sm:w-auto/);
  assert.match(small.querySelector('figcaption .whitespace-nowrap').textContent, /JPEG|JPG|Image/i);
  v.unmount();
  // a person's own upload keeps the labelled button, 44 px on phone
  const mine = mount([{ ...you('u9', 'here', at('10:31')), meta: JSON.stringify({ attachments: [{ id: fileId(), name: 'me.png', mime: 'image/png', size: 1200 }] }) }]);
  await settle();
  const own = mine.host.querySelector('[data-download]');
  if (own) assert.match(own.className, /max-sm:min-w-11 max-sm:min-h-11/);
  mine.unmount();
});

test("ChatView: in a team chat with two bots, each bot's share sits under its own reply", async () => {
  const msgs = [
    you('u1', 'Both of you, show me', at('10:50')),
    share('fa', FRONTEND, 'header.png', at('10:50', 5)),
    share('ga', GROWTH, 'funnel.png', at('10:50', 6)),
    reply('gr', 'Growth', 'Funnel is above.', at('10:50', 20)),
    reply('fr', 'Frontend', 'Header is fixed.', at('10:50', 30)),
  ];
  const v = mount(msgs, { title: 'Ship team' });
  await settle();
  assert.deepEqual(rowIds(v.host), ['u1', 'gr', 'ga', 'fr', 'fa'], "Frontend's picture does not land under Growth's reply");
  assert.match(v.host.querySelector('[data-message-id="ga"]').textContent, /Shared by Growth/);
  assert.match(v.host.querySelector('[data-message-id="fa"]').textContent, /Shared by Frontend/);
  assert.ok(v.host.querySelector('img[alt="header.png, shared by Frontend"]'));
  assert.ok(v.host.querySelector('img[alt="funnel.png, shared by Growth"]'));
  v.unmount();
});
