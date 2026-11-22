// 33m stopped-row-names-the-job (Juno): a stopped job's Inbox row is titled with the person's request (server,
// `title_is_request`), so its subline says why it stopped: "Stopped · Couldn't finish this reply: …". A row still
// titled with the reason (no request found) shows only "Stopped", never the reason twice.
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/inbox', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'HTMLTextAreaElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'DOMParser', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.IS_REACT_ACT_ENVIRONMENT = false;
g.fetch = async () => ({ ok: true, status: 200, json: async () => ({}) });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * as fx from './src/doneAwayFixture';
  export * as store from './src/inboxStore';
  export { InboxNew } from './src/components/InboxNew';
`);
const { React, createRoot, fx, store: s } = m;
const h = React.createElement;
const settle = async (n = 20) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};

const REASON = "Couldn't finish this reply: its model stopped answering. Retry in the chat sends your message again.";
const stopped = (id, title, extra = {}) => ({
  kind: 'stopped',
  id,
  kwant_id: 'k-ben',
  kwant_name: 'Ben',
  title,
  summary: REASON,
  status: 'open',
  timestamp: '2026-11-11T23:58:00Z',
  message_id: `notice-${id}`,
  retry: null,
  link: { thread_id: null, channel_id: null, direct_kwant_id: 'k-ben' },
  group: 'waiting_on_you',
  sort_key: [1, 0, 0, 'Ben', id],
  unread: true,
  ...extra,
});

let root = null;
afterEach(() => {
  root?.unmount();
  root = null;
  document.body.innerHTML = '';
});

async function mount(items, wide) {
  const data = { ...JSON.parse(JSON.stringify(fx.AWAY_INBOX)), items };
  document.body.innerHTML = '';
  sessionStorage.clear();
  window.history.replaceState(null, '', '/inbox');
  s.inboxStore.reset();
  s.inboxSource.get = async () => data;
  s.inboxStore.patch({ data });
  const el = document.createElement('div');
  document.body.appendChild(el);
  root = createRoot(el);
  root.render(h(m.InboxNew, { onOpen: () => {}, noLoad: true, wide, now: Date.parse(data.now), placeName: fx.awayPlaceName }));
  await settle();
  return el;
}

const row = (el, id) => {
  const r = el.querySelector(`[data-row="${id}"]`);
  assert.ok(r, `the Inbox shows row ${id}`);
  return r;
};
const text = (n) => n.textContent.replace(/\s+/g, ' ').trim();

for (const wide of [true, false]) {
  const where = wide ? 'wide' : 'phone';

  test(`33m (${where}): a stopped row titled with the request says why it stopped underneath`, async () => {
    const el = await mount([stopped('w-req', 'Draft the Q3 board update', { title_is_request: true })], wide);
    const r = row(el, 'w-req');
    assert.match(text(r), /Draft the Q3 board update/);
    const why = r.querySelector('[data-stopped-why]');
    assert.ok(why, 'the reason is in the subline');
    assert.equal(text(why), `· ${REASON}`);
    assert.ok(why.classList.contains('truncate'), 'one truncated line, like the other sublines');
    assert.equal(text(why.parentElement.firstElementChild), 'Stopped');
  });

  test(`33m (${where}): a stopped row titled with its reason doesn't say it twice`, async () => {
    const el = await mount([stopped('w-old', REASON)], wide);
    const r = row(el, 'w-old');
    assert.equal(r.querySelector('[data-stopped-why]'), null);
    assert.equal(text(r).split(REASON).length - 1, 1, `the reason once: ${text(r)}`);
  });
}

test('33m: a long-reason stopped row whose title shortens the reason still shows it once', async () => {
  // The server's fallback title is the reason shortened ("…"), so it differs from summary: only the flag decides.
  const long = `${REASON.slice(0, 40)}… Retry in the chat sends your message again.`;
  const el = await mount([stopped('w-long', long)], true);
  assert.equal(row(el, 'w-long').querySelector('[data-stopped-why]'), null);
});
