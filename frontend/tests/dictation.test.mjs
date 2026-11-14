// Dictation mic (Mira, autonomy/dictation/design.md; Juno's #610 routes). The pure helpers, then the mic,
// the dictation line and the "isn't set up" popover inside the real ChatView, against a fake
// /transcribe, a fake microphone and a fake MediaRecorder. Made-up words only; no network.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'HTMLTextAreaElement', 'Element', 'Node', 'Event', 'KeyboardEvent', 'MouseEvent', 'FocusEvent', 'MutationObserver', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'CSS']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
const store = new Map();
const ls = { getItem: (k) => (store.has(k) ? store.get(k) : null), setItem: (k, v) => void store.set(k, String(v)), removeItem: (k) => void store.delete(k) };
Object.defineProperty(globalThis, 'localStorage', { value: ls, configurable: true, writable: true });
Object.defineProperty(dom.window, 'localStorage', { value: ls, configurable: true });
const media = { phone: false, coarse: false };
dom.window.matchMedia = (q) => ({
  matches: q.includes('pointer: coarse') ? media.coarse : q.includes('max-width: 767px') || q.includes('max-width: 639px') ? media.phone : false,
  media: q,
  addEventListener() {},
  removeEventListener() {},
  addListener() {},
  removeListener() {},
});
globalThis.matchMedia = dom.window.matchMedia;
globalThis.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} };
dom.window.ResizeObserver ??= globalThis.ResizeObserver;
globalThis.IntersectionObserver ??= class { observe() {} unobserve() {} disconnect() {} };
dom.window.HTMLElement.prototype.scrollIntoView ??= function () {};
dom.window.HTMLElement.prototype.scrollTo ??= function () {};
globalThis.IS_REACT_ACT_ENVIRONMENT = false;

// The fake microphone and recorder.
const mic = { refuse: null, streams: 0, stopped: 0 };
class FakeRecorder {
  constructor(stream) {
    this.stream = stream;
    this.state = 'inactive';
    this.mimeType = 'audio/webm;codecs=opus';
  }
  start() {
    this.state = 'recording';
  }
  stop() {
    this.state = 'inactive';
    this.ondataavailable?.({ data: new Blob(['made-up audio'], { type: this.mimeType }) });
    this.onstop?.();
  }
}
dom.window.MediaRecorder = FakeRecorder;
globalThis.MediaRecorder = FakeRecorder;
Object.defineProperty(dom.window, 'isSecureContext', { value: true, configurable: true, writable: true });
Object.defineProperty(dom.window.navigator, 'mediaDevices', {
  configurable: true,
  value: {
    getUserMedia: async () => {
      if (mic.refuse) throw Object.assign(new Error('no'), { name: mic.refuse });
      mic.streams++;
      const track = { stop: () => mic.stopped++, addEventListener() {} };
      return { getTracks: () => [track], getAudioTracks: () => [track] };
    },
  },
});

// The fake server: GET answers per space, POST answers from a queue (or waits for `release`).
const server = { get: {}, posts: [], answers: [], hold: null };
globalThis.fetch = async (url, init = {}) => {
  const u = String(url);
  const m1 = u.match(/^\/api\/spaces\/([^/]+)\/transcribe(\?.*)?$/);
  if (!m1) return { ok: true, status: 200, json: async () => [] };
  const space = decodeURIComponent(m1[1]);
  if ((init.method ?? 'GET') === 'GET') {
    const a = server.get[space];
    if (a === 'down') throw new TypeError('fetch failed');
    if (!a) return { ok: false, status: 404, json: async () => ({ error: 'no such space', code: 'no_space' }) };
    return { ok: true, status: 200, json: async () => a };
  }
  server.posts.push({ space, url: u, type: init.headers?.['Content-Type'], body: init.body });
  if (server.hold) await server.hold;
  const [status, body] = server.answers.shift() ?? [200, { text: 'made-up words', source: 'openai' }];
  if (status === 0) throw new TypeError('fetch failed');
  return { ok: status >= 200 && status < 300, status, json: async () => body };
};

const ui = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { createRoot } from 'react-dom/client';
  import { ChatView } from './src/components/ChatView';
  export * from './src/dictation';
  export const mount = (p) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    const render = (q) => flushSync(() => root.render(React.createElement(ChatView, q)));
    render(p);
    return { host, render, unmount: () => { root.unmount(); host.remove(); } };
  };
  export const act = (fn) => flushSync(fn);
`);

const settle = async (n = 15) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const props = (extra = {}) => ({
  title: 'Wren',
  subtitle: 'Newsletters and social posts',
  messages: [],
  streamingText: '',
  streamingThinking: '',
  isStreaming: false,
  pendingApprovals: [],
  onSendMessage: () => {},
  onResolveApproval: () => {},
  onCancelDiscussion: () => {},
  kwants: [],
  teams: [],
  contextKey: 'k:wren',
  ...extra,
});
const $ = (h, s) => h.querySelector(s);
const micBtn = (h) => $(h, '[data-mic]');
const sendBtn = (h) => $(h, 'form[data-composer-form] button[type="submit"]');
const line = (h) => $(h, '[data-dictation-line]');
const click = (el) => ui.act(() => el.dispatchEvent(new window.MouseEvent('click', { bubbles: true })));
const typeInto = (ta, value) =>
  ui.act(() => {
    Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set.call(ta, value);
    ta.dispatchEvent(new window.Event('input', { bubbles: true }));
  });
const reset = () => {
  media.phone = false;
  media.coarse = false;
  mic.refuse = null;
  server.answers = [];
  server.posts = [];
  server.hold = null;
};

// ---------------------------------------------------------------------------------------------------
// 1. insertDictated: spacing

test('insertDictated: start, middle, end, before punctuation, and a caret past the end is clamped', () => {
  // empty box
  assert.deepEqual(ui.insertDictated('', 0, '  Book a table.  '), { value: 'Book a table.', from: 0, to: 13, piece: 'Book a table.', caret: 13 });
  // start of typed text: a space after, none before
  let r = ui.insertDictated('for four', 0, 'Book a table');
  assert.equal(r.value, 'Book a table for four');
  assert.equal(r.piece, 'Book a table ');
  assert.equal(r.caret, 12);
  // middle, between two words with no space at the caret
  r = ui.insertDictated('Wren,plan', 5, 'please');
  assert.equal(r.value, 'Wren, please plan');
  // after a space: no extra space before
  r = ui.insertDictated('Wren, ', 6, 'plan it');
  assert.equal(r.value, 'Wren, plan it');
  assert.equal(r.piece, 'plan it');
  // end of text with no trailing space
  r = ui.insertDictated('Wren, plan the spring newsletter.', 33, 'Use three quotes.');
  assert.equal(r.value, 'Wren, plan the spring newsletter. Use three quotes.');
  assert.equal(r.caret, r.value.length);
  // before punctuation: no space after
  r = ui.insertDictated('Call Ana.', 8, 'tomorrow');
  assert.equal(r.value, 'Call Ana tomorrow.');
  r = ui.insertDictated('Call Ana, then', 8, 'and Ben');
  assert.equal(r.value, 'Call Ana and Ben, then');
  // typed while recording: the saved caret is past the (shorter) text now
  r = ui.insertDictated('Hi', 40, 'there');
  assert.equal(r.value, 'Hi there');
  assert.equal(r.from, 2);
  r = ui.insertDictated('Hi', -3, 'there');
  assert.equal(r.value, 'there Hi');
});

// ---------------------------------------------------------------------------------------------------
// 2. Undo only when unchanged

test('undoDictated: takes back exactly the inserted piece, only while it is unchanged', () => {
  const r = ui.insertDictated('Wren, plan it.', 10, 'the spring newsletter');
  assert.equal(r.value, 'Wren, plan the spring newsletter it.');
  assert.equal(ui.undoDictated(r.value, r), 'Wren, plan it.');
  assert.equal(ui.undoDictated(r.value.replace('spring', 'autumn'), r), null, 'edited words stay');
  assert.equal(ui.undoDictated(`X${r.value}`, r), null, 'moved words stay');
  assert.equal(ui.undoDictated('', r), null);
});

// ---------------------------------------------------------------------------------------------------
// 3. micVisible: every row of §2

test('micVisible: ready, muted, hidden for every row of the availability table', () => {
  const yes = { available: true, source: 'openai', why: null };
  const no = { available: false, source: null, why: 'Add an OpenAI API key in Apps & accounts to dictate.' };
  const v = (o) => ui.micVisible({ supported: true, secure: true, availability: yes, hidden: false, ...o });
  assert.equal(v({}), 'ready');
  assert.equal(v({ availability: no }), 'muted');
  assert.equal(v({ availability: no, hidden: true }), 'hidden', 'Hide the mic');
  assert.equal(v({ hidden: true }), 'ready', 'a key brings the mic back even if it was hidden');
  assert.equal(v({ availability: 'error' }), 'hidden', 'GET failed or 404');
  assert.equal(v({ availability: undefined }), 'hidden', 'not asked yet');
  assert.equal(v({ supported: false }), 'hidden', 'no MediaRecorder');
  assert.equal(v({ secure: false }), 'muted', 'plain http: muted, popover explains');
  assert.equal(v({ secure: false, hidden: true }), 'hidden');
  // what "supported" means
  const gum = { mediaDevices: { getUserMedia() {} } };
  assert.equal(ui.browserSupports({ MediaRecorder: FakeRecorder, isSecureContext: true, navigator: gum }), true);
  assert.equal(ui.browserSupports({ isSecureContext: true, navigator: gum }), false);
  assert.equal(ui.browserSupports({ MediaRecorder: FakeRecorder, isSecureContext: true, navigator: {} }), false);
  assert.equal(ui.browserSupports({ MediaRecorder: FakeRecorder, isSecureContext: false, navigator: {} }), true, 'http hides getUserMedia; the mic shows muted');
  assert.equal(ui.langHint('en-GB'), 'en');
  assert.equal(ui.langHint('PT'), 'pt');
  assert.equal(ui.langHint('x'), null);
  assert.equal(ui.langHint(''), null);
  assert.equal(ui.wordCount('  one  two\nthree '), 3);
  assert.equal(ui.wordsLabel(1), '1 word');
  assert.equal(ui.wordsLabel(23), '23 words');
  assert.equal(ui.clock(12), '0:12');
  assert.equal(ui.clock(600), '10:00');
});

// ---------------------------------------------------------------------------------------------------
// 4. Browser errors

test('micErrorText: each getUserMedia error, desktop and phone', () => {
  const blockedD = 'The microphone is blocked for this site. Allow it from the icon at the left of the address bar, then press the mic again.';
  const blockedP = 'The microphone is blocked for this site. Allow it in your browser’s site settings, then press the mic again.';
  for (const n of ['NotAllowedError', 'SecurityError']) {
    assert.equal(ui.micErrorText(n, false), blockedD);
    assert.equal(ui.micErrorText(n, true), blockedP);
  }
  for (const n of ['NotFoundError', 'OverconstrainedError']) {
    assert.equal(ui.micErrorText(n, false), 'No microphone found. Plug one in or pick one in your system’s sound settings, then press the mic again.');
    assert.equal(ui.micErrorText(n, true), 'No microphone found. Check your phone’s microphone settings for this browser, then press the mic again.');
  }
  for (const n of ['NotReadableError', 'AbortError'])
    for (const p of [false, true]) assert.equal(ui.micErrorText(n, p), 'The microphone is in use by another app. Close it there, then press the mic again.');
  assert.equal(ui.micErrorText('TypeError', false), 'Couldn’t start the microphone. Try again, or reload the page.');
  assert.equal(ui.micErrorText(undefined, true), 'Couldn’t start the microphone. Try again, or reload the page.');
});

// ---------------------------------------------------------------------------------------------------
// 5. Retry only for 429/502/network

test('canRetry: busy, OpenAI failed and no network keep the recording; the rest need a new one', () => {
  assert.equal(ui.canRetry(429, 'busy'), true);
  assert.equal(ui.canRetry(502, 'transcriber_failed'), true);
  assert.equal(ui.canRetry(0, 'network'), true);
  for (const [s, c] of [[413, 'too_large'], [400, 'empty_audio'], [400, 'interrupted'], [415, 'bad_audio_type'], [400, 'bad_lang'], [403, null], [404, null], [409, null]])
    assert.equal(ui.canRetry(s, c), false, `${s} ${c}`);
});

test('transcribe: the raw recording as the body with its type, lang only when given; refusals as sent', async () => {
  reset();
  const blob = new Blob(['x'], { type: 'audio/webm;codecs=opus' });
  server.answers.push([200, { text: 'Book a table for four on Friday at seven.', source: 'openai' }]);
  assert.deepEqual(await ui.transcribe('spc_t', blob, 'en'), { ok: true, text: 'Book a table for four on Friday at seven.' });
  assert.equal(server.posts[0].url, '/api/spaces/spc_t/transcribe?lang=en');
  assert.equal(server.posts[0].type, 'audio/webm;codecs=opus');
  assert.equal(server.posts[0].body, blob);
  server.answers.push([429, { error: 'Other recordings are being turned into text. Try again in a moment.', code: 'busy' }]);
  const r = await ui.transcribe('spc_t', blob, null);
  assert.equal(server.posts[1].url, '/api/spaces/spc_t/transcribe');
  assert.deepEqual(r, { ok: false, status: 429, code: 'busy', error: 'Other recordings are being turned into text. Try again in a moment.', why: null });
  server.answers.push([0]);
  assert.deepEqual(await ui.transcribe('spc_t', blob, null), { ok: false, status: 0, code: 'network', error: ui.NETWORK_ERROR });
});

// ---------------------------------------------------------------------------------------------------
// 6–8. In the real ChatView

test('Desktop: the mic sits between @ and Send, records, and Send waits for dictation; the words land at the caret', async () => {
  reset();
  server.get.spc_a = { available: true, source: 'openai', why: null };
  const sent = [];
  const v = ui.mount(props({ spaceId: 'spc_a', onSendMessage: (t) => sent.push(t) }));
  await settle();
  const tools = [...v.host.querySelectorAll('[data-composer-tools] > button')].map((b) => b.getAttribute('aria-label'));
  assert.deepEqual(tools, ['Attach files', 'Mention a bot', 'Dictate', 'Send']);
  assert.equal(micBtn(v.host).getAttribute('aria-pressed'), 'false');
  assert.equal(micBtn(v.host).title, 'Dictate: speak, then edit before you send');

  const ta = $(v.host, 'textarea[data-composer]');
  ta.focus();
  typeInto(ta, 'Wren, plan the spring newsletter.');
  assert.equal(sendBtn(v.host).disabled, false);

  let release;
  server.hold = new Promise((r) => (release = r));
  server.answers.push([200, { text: 'Use the three customer quotes from last week.', source: 'openai' }]);
  click(micBtn(v.host));
  await settle();
  assert.equal(micBtn(v.host).dataset.mic, 'rec');
  assert.equal(micBtn(v.host).getAttribute('aria-pressed'), 'true');
  assert.equal(micBtn(v.host).title, 'Stop and turn it into text (Esc)');
  assert.equal(line(v.host).dataset.dictationLine, 'recording');
  assert.match(line(v.host).textContent, /Listening0:0\d.*nothing goes to Wren until you press Send/);
  assert.equal(sendBtn(v.host).disabled, true, 'Send is disabled while recording');
  assert.equal(sendBtn(v.host).getAttribute('aria-label'), 'Send (waiting for dictation)');
  assert.equal(sendBtn(v.host).title, 'Waiting for dictation to finish');
  // Enter does not send while dictating
  ui.act(() => ta.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));
  assert.equal(sent.length, 0);

  click(micBtn(v.host)); // stop
  await settle(3);
  assert.equal(mic.stopped > 0, true, 'the microphone is released at once');
  assert.equal(micBtn(v.host).dataset.mic, 'busy');
  assert.equal(micBtn(v.host).getAttribute('aria-disabled'), 'true');
  assert.equal(line(v.host).dataset.dictationLine, 'transcribing');
  assert.match(line(v.host).textContent, /Turning speech into text…0:00 recorded/);
  assert.equal(sendBtn(v.host).disabled, true, 'Send is disabled while turning into text');
  assert.equal(server.posts.at(-1).type, 'audio/webm;codecs=opus');

  release();
  await settle();
  assert.equal(ta.value, 'Wren, plan the spring newsletter. Use the three customer quotes from last week.');
  assert.equal(line(v.host).dataset.dictationLine, 'added');
  assert.match(line(v.host).textContent, /Added 8 words\. Read them over, then press Send\./);
  assert.ok($(v.host, '[data-dictation-line] button'), 'Undo shows');
  assert.equal(sendBtn(v.host).disabled, false);
  assert.equal(sendBtn(v.host).getAttribute('aria-label'), 'Send');
  assert.equal(document.activeElement, ta, 'desktop: focus goes to the text, caret after the words');
  assert.equal(ta.selectionStart, ta.value.length);
  assert.match($(v.host, '[data-dictation-announce]').textContent, /Added 8 words to the message\./);
  assert.equal(sent.length, 0, 'nothing went to a bot');

  // Undo removes exactly what was added
  click($(v.host, '[data-dictation-line] button'));
  await settle(3);
  assert.equal(ta.value, 'Wren, plan the spring newsletter.');
  assert.equal(line(v.host), null);
  v.unmount();
});

test('Undo hides once the words were edited, and the line goes on the next keystroke', async () => {
  reset();
  server.get.spc_u = { available: true, source: 'openai', why: null };
  const v = ui.mount(props({ spaceId: 'spc_u' }));
  await settle();
  server.answers.push([200, { text: 'hello there', source: 'openai' }]);
  click(micBtn(v.host));
  await settle();
  click(micBtn(v.host));
  await settle();
  const ta = $(v.host, 'textarea[data-composer]');
  assert.equal(ta.value, 'hello there');
  assert.equal(line(v.host).dataset.dictationLine, 'added');
  v.render(props({ spaceId: 'spc_u' }));
  typeInto(ta, 'hello where');
  assert.equal(line(v.host), null, 'a keystroke clears the Added line');
  v.unmount();
});

test('Esc stops and transcribes; Cancel throws the recording away and says so', async () => {
  reset();
  server.get.spc_e = { available: true, source: 'openai', why: null };
  const v = ui.mount(props({ spaceId: 'spc_e' }));
  await settle();
  server.answers.push([200, { text: 'by Esc', source: 'openai' }]);
  click(micBtn(v.host));
  await settle();
  ui.act(() => document.body.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
  await settle();
  assert.equal($(v.host, 'textarea[data-composer]').value, 'by Esc');

  const posts = server.posts.length;
  click(micBtn(v.host));
  await settle();
  assert.equal(line(v.host).dataset.dictationLine, 'recording');
  click([...v.host.querySelectorAll('[data-dictation-line] button')].find((b) => b.textContent === 'Cancel'));
  await settle();
  assert.equal(line(v.host), null);
  assert.equal(server.posts.length, posts, 'nothing was sent');
  assert.equal(micBtn(v.host).dataset.mic, 'ready');
  assert.equal($(v.host, '[data-dictation-announce]').textContent, 'Dictation cancelled.');
  assert.equal(document.activeElement, micBtn(v.host));
  v.unmount();
});

test('Errors: 429 keeps the recording for Try again; 413 has only ✕; a blocked microphone says what fixes it', async () => {
  reset();
  server.get.spc_r = { available: true, source: 'openai', why: null };
  const v = ui.mount(props({ spaceId: 'spc_r' }));
  await settle();
  const ta = $(v.host, 'textarea[data-composer]');
  typeInto(ta, 'Typed first.');
  server.answers.push([429, { error: 'Other recordings are being turned into text. Try again in a moment.', code: 'busy' }]);
  click(micBtn(v.host));
  await settle();
  click(micBtn(v.host));
  await settle();
  let l = line(v.host);
  assert.equal(l.dataset.dictationLine, 'bad');
  assert.equal(l.getAttribute('role'), 'alert');
  assert.match(l.textContent, /Other recordings are being turned into text\. Try again in a moment\./);
  assert.equal(ta.value, 'Typed first.', 'what was typed is kept');
  const tryAgain = [...l.querySelectorAll('button')].find((b) => b.textContent.includes('Try again'));
  assert.ok(tryAgain);
  assert.equal(document.activeElement, micBtn(v.host), 'focus returns to the mic');
  const first = server.posts.at(-1).body;
  server.answers.push([200, { text: 'second time lucky', source: 'openai' }]);
  click(tryAgain);
  await settle();
  assert.equal(server.posts.at(-1).body, first, 'Try again sends the same recording');
  assert.equal(ta.value, 'Typed first. second time lucky');

  server.answers.push([413, { error: 'The recording is over 25 MB. Record a shorter message.', code: 'too_large' }]);
  click(micBtn(v.host));
  await settle();
  click(micBtn(v.host));
  await settle();
  l = line(v.host);
  assert.deepEqual([...l.querySelectorAll('button')].map((b) => b.getAttribute('aria-label') ?? b.textContent), ['Dismiss']);
  click(l.querySelector('button'));
  assert.equal(line(v.host), null);

  server.answers.push([200, { text: '   ', source: 'openai' }]);
  click(micBtn(v.host));
  await settle();
  click(micBtn(v.host));
  await settle();
  assert.equal(line(v.host).dataset.dictationLine, 'warn');
  assert.match(line(v.host).textContent, /Didn’t catch any words/);

  mic.refuse = 'NotAllowedError';
  click(micBtn(v.host));
  await settle();
  l = line(v.host);
  assert.equal(l.dataset.dictationLine, 'warn');
  assert.match(l.textContent, /Allow it from the icon at the left of the address bar/);
  assert.equal(micBtn(v.host).dataset.mic, 'ready');
  v.unmount();
});

test('409 no_transcriber: the mic turns muted and the popover opens with the server’s why; typing is untouched', async () => {
  reset();
  server.get.spc_n = { available: true, source: 'openai', why: null };
  let opened = 0;
  const v = ui.mount(props({ spaceId: 'spc_n', onOpenAccounts: () => opened++ }));
  await settle();
  const ta = $(v.host, 'textarea[data-composer]');
  typeInto(ta, 'Keep me.');
  const why = 'The OpenAI API is turned off in Apps & accounts. Turn it on to dictate.';
  server.answers.push([409, { error: why, code: 'no_transcriber', why }]);
  click(micBtn(v.host));
  await settle();
  click(micBtn(v.host));
  await settle();
  assert.equal(ta.value, 'Keep me.');
  assert.equal(line(v.host), null, 'no line: the popover says it');
  assert.equal(micBtn(v.host).dataset.mic, 'muted');
  assert.equal(micBtn(v.host).getAttribute('aria-label'), 'Dictate (not set up)');
  assert.equal(micBtn(v.host).getAttribute('aria-expanded'), 'true');
  const pop = $(v.host, '[data-dictation-popover]');
  assert.ok(pop);
  assert.equal(pop.getAttribute('role'), 'dialog');
  assert.match(pop.textContent, /Dictation isn’t set up in this space/);
  assert.ok(pop.textContent.includes(why));
  assert.equal(document.activeElement.textContent, 'Open Apps & accounts');
  click([...pop.querySelectorAll('button')].find((b) => b.textContent === 'Open Apps & accounts'));
  assert.equal(opened, 1);
  assert.equal($(v.host, '[data-dictation-popover]'), null);
  v.unmount();
});

test('Not set up: the muted mic opens the popover; Esc closes it to the mic; Hide the mic hides it until a key arrives', async () => {
  reset();
  server.get.spc_off = { available: false, source: null, why: 'Add an OpenAI API key in Apps & accounts to dictate.' };
  const v = ui.mount(props({ spaceId: 'spc_off', onOpenAccounts: () => {} }));
  await settle();
  assert.equal(micBtn(v.host).dataset.mic, 'muted');
  assert.equal(micBtn(v.host).disabled, false, 'focusable and pressable');
  assert.equal(micBtn(v.host).getAttribute('aria-haspopup'), 'dialog');
  click(micBtn(v.host));
  await settle(3);
  assert.ok($(v.host, '[data-dictation-popover]'));
  assert.equal(mic.streams, mic.streams, 'no microphone asked for');
  ui.act(() => document.body.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
  assert.equal($(v.host, '[data-dictation-popover]'), null);
  assert.equal(document.activeElement, micBtn(v.host));
  click(micBtn(v.host));
  await settle(3);
  click([...$(v.host, '[data-dictation-popover]').querySelectorAll('button')].find((b) => b.textContent === 'Hide the mic'));
  await settle(3);
  assert.equal(micBtn(v.host), null);
  assert.equal(localStorage.getItem('kwnt:dictation-hidden:spc_off'), '1');
  assert.equal($(v.host, '[data-dictation-announce]').textContent, 'Mic hidden. It comes back when dictation is set up.');
  // a key gets added: the next GET says yes, and the mic is back
  server.get.spc_off = { available: true, source: 'openai', why: null };
  ui.refreshDictation();
  await settle();
  assert.equal(micBtn(v.host)?.dataset.mic, 'ready');
  assert.equal(localStorage.getItem('kwnt:dictation-hidden:spc_off'), null);
  v.unmount();
});

test('No mic when the GET fails or 404s, or the browser can’t record', async () => {
  reset();
  server.get.spc_down = 'down';
  let v = ui.mount(props({ spaceId: 'spc_down' }));
  await settle();
  assert.equal(micBtn(v.host), null);
  assert.equal(line(v.host), null, 'no error line: nothing the person did');
  v.unmount();
  v = ui.mount(props({ spaceId: 'spc_missing' }));
  await settle();
  assert.equal(micBtn(v.host), null);
  v.unmount();
  server.get.spc_norec = { available: true, source: 'openai', why: null };
  const keep = dom.window.MediaRecorder;
  delete dom.window.MediaRecorder;
  v = ui.mount(props({ spaceId: 'spc_norec' }));
  await settle();
  assert.equal(micBtn(v.host), null);
  v.unmount();
  dom.window.MediaRecorder = keep;
});

test('Phone at rest: the mic takes Send’s slot only while it shows; stacked brings Send back', async () => {
  reset();
  media.phone = true;
  media.coarse = true;
  server.get.spc_p = { available: true, source: 'openai', why: null };
  let v = ui.mount(props({ spaceId: 'spc_p' }));
  await settle();
  const form = $(v.host, 'form[data-composer-form]');
  assert.equal(form.dataset.stacked, 'false');
  assert.ok(micBtn(v.host));
  assert.equal(sendBtn(v.host), null, 'at rest: no Send, the mic is in its slot');
  const ta = $(v.host, 'textarea[data-composer]');
  typeInto(ta, 'Hi');
  assert.equal(form.dataset.stacked, 'true');
  const tools = [...v.host.querySelectorAll('[data-composer-tools] > button')].map((b) => b.getAttribute('aria-label'));
  assert.deepEqual(tools, ['Attach files', 'Mention a bot', 'Dictate', 'Send']);
  v.unmount();
  // No mic (GET failed): Send keeps its slot at rest
  server.get.spc_p2 = 'down';
  v = ui.mount(props({ spaceId: 'spc_p2' }));
  await settle();
  assert.equal(micBtn(v.host), null);
  assert.ok(sendBtn(v.host), 'Send stays when there is no mic');
  v.unmount();
});

test('Phone: dictating does not focus the text (no keyboard); the words land and focus stays on the mic', async () => {
  reset();
  media.phone = true;
  media.coarse = true;
  server.get.spc_ph = { available: true, source: 'openai', why: null };
  const v = ui.mount(props({ spaceId: 'spc_ph' }));
  await settle();
  server.answers.push([200, { text: 'from the phone', source: 'openai' }]);
  micBtn(v.host).focus();
  click(micBtn(v.host));
  await settle();
  assert.equal($(v.host, 'textarea[data-composer]').placeholder, 'Speak now…');
  assert.notEqual(document.activeElement, $(v.host, 'textarea[data-composer]'));
  assert.ok(!/nothing goes to/.test(line(v.host).textContent), 'the long note is desktop only');
  click(micBtn(v.host));
  await settle();
  assert.equal($(v.host, 'textarea[data-composer]').value, 'from the phone');
  assert.equal(document.activeElement, micBtn(v.host));
  assert.ok(sendBtn(v.host), 'it has words now, so it stacks and Send is back');
  v.unmount();
});

test('Leaving the chat while recording stops it, discards it and says so', async () => {
  reset();
  server.get.spc_l = { available: true, source: 'openai', why: null };
  const toasts = [];
  const v = ui.mount(props({ spaceId: 'spc_l', onToast: (t) => toasts.push(t) }));
  await settle();
  click(micBtn(v.host));
  await settle();
  const posts = server.posts.length;
  const stopped = mic.stopped;
  v.render(props({ spaceId: 'spc_l', contextKey: 'k:ola', onToast: (t) => toasts.push(t) }));
  await settle();
  assert.deepEqual(toasts, ['Dictation stopped: you left the chat.']);
  assert.equal(server.posts.length, posts);
  assert.ok(mic.stopped > stopped);
  assert.equal(line(v.host), null);
  v.unmount();
});

test('Wiring: App passes the real space id and a toast, and asks again when Space settings close', () => {
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  const at = app.indexOf('const chat = (');
  const site = app.slice(at, app.indexOf('/>', app.indexOf('<ChatView', at)));
  assert.match(site, /spaceId=\{space\.id\}/);
  assert.match(site, /onToast=\{pushToast\}/);
  const close = app.slice(app.indexOf('const closeSpaceSettings'), app.indexOf('const closeSpaceSettings') + 500);
  assert.match(close, /refreshDictation\(\)/);
});
