// 31y (autonomy/approval-clears-needs-you-at-once/brief.md; Mira's site-clips design checks): answering an approval
// on its chat card clears its "Needs you" strip and its sidebar NEEDS YOU row in the same update that drops the card,
// not on the 300 ms refetch or the server's close event. A failed send clears nothing, the needs_backup 409 keeps
// both, and another ask still waiting in that chat keeps the strip.
//
// The test runs App.tsx's own handleResolveApproval (its source, transpiled) with a fake fetch, and builds the strip
// and the sidebar rows the way App does (awayList minus clearedAsks -> asksHere / needsYouRows). No timers are
// advanced and no server event is sent. Red at main 51ff5594.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { transform } from 'esbuild';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'DOMParser', 'localStorage', 'sessionStorage', 'getComputedStyle']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { default as React } from 'react';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export * as away from './src/away';
  export { asksHere } from './src/askHere';
  export { needsYouRows } from './src/chatsTab';
  export { NeedsYouStrip } from './src/components/NeedsYouStrip';
`);

// App.tsx's handleResolveApproval, as written, made callable with App's pieces passed in.
const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
const start = app.indexOf('const handleResolveApproval = async (');
assert.ok(start > 0, 'App.tsx has handleResolveApproval');
const end = app.indexOf('\n  };\n', start) + '\n  };\n'.length;
const { code: handlerJs } = await transform(`${app.slice(start, end)}\n`, { loader: 'ts' });
const DEPS = ['haptic', 'fetch', 'resolveBody', 'pushToast', 'pendingApprovals', 'dropApproval', 'lateAnswerText', 'refreshAwayCount', 'setClearedAsks', 'clearAnswered', 'console'];
const makeHandler = new Function(...DEPS, `${handlerJs}\nreturn handleResolveApproval;`);

const NOW = Date.parse('2026-11-21T03:30:00Z');
const at = (min) => new Date(NOW - min * 60_000).toISOString();
const link = { thread_id: null, channel_id: null, direct_kwant_id: 'tl' };
const approval = (id, min = 2) => ({ kind: 'approval', id, kwant_id: 'tl', kwant_name: 'TechLead', title: 'Run a command', summary: 'git push',
  status: 'pending', timestamp: at(min), link, tool_name: 'Bash', details: { command: 'git push' }, action: 'decide', dismissable: false });
const question = (id) => ({ kind: 'question', id, kwant_id: 'tl', kwant_name: 'TechLead', title: 'Which branch?', summary: 'Waiting for your answer',
  status: 'pending', timestamp: at(5), link, choices: ['main', 'dev'], action: 'answer', dismissable: true });
const card = (id) => ({ id, waiting: false, details: { kwant_name: 'TechLead', tool_name: 'Bash' } });
const HERE = { direct: 'tl' };

/** App's state for one chat: the away list, the cleared set, the cards; and the handler wired to it. */
function rig(awayList, respond) {
  const s = { awayList, cleared: new Set(), cards: awayList.filter((i) => i.kind === 'approval').map((i) => card(i.id)), toasts: [], refetches: [] };
  const setClearedAsks = (fn) => { s.cleared = typeof fn === 'function' ? fn(s.cleared) : fn; };
  const fetch = async () => respond();
  const handler = makeHandler(() => {}, fetch, m.away.resolveBody, (t, b) => s.toasts.push([t, b]), s.cards,
    (id) => { s.cards = s.cards.filter((c) => c.id !== id); }, () => null, (ms) => s.refetches.push(ms), setClearedAsks, m.away.clearAnswered,
    { error: () => {} });
  // App.tsx: awayItems = awayList minus clearedAsks; the strip is asksHere(awayItems, here); the sidebar rows needsYouRows(awayItems, ...).
  const items = () => s.awayList.filter((i) => !s.cleared.has(i.id));
  s.asks = () => m.asksHere(items(), HERE);
  s.rows = () => m.needsYouRows(items(), {}, new Set(['tl']));
  s.strip = () => {
    const html = m.renderToStaticMarkup(m.React.createElement(m.NeedsYouStrip, { asks: s.asks(), onAction: () => {}, onDismiss: async () => {}, now: NOW }));
    return new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html').querySelector('[data-needs-you-strip]');
  };
  s.answer = (decision = 'allow_once', opts) => handler('ap1', decision, opts);
  return s;
}
const json = (status, body) => new Response(body === undefined ? null : JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

test('31y: before answering, the strip and the sidebar row point at the approval (the setup is real)', () => {
  const s = rig([approval('ap1')], () => json(200, {}));
  assert.equal(s.asks().length, 1);
  assert.ok(s.strip(), 'the strip shows');
  assert.equal(s.rows().length, 1, 'one NEEDS YOU row');
});

for (const decision of ['allow_once', 'always_allow', 'deny']) {
  test(`31y: ${decision} on the card clears its strip and sidebar row at once, with the card`, async () => {
    const s = rig([approval('ap1')], () => json(200, { status: 'resolved' }));
    await s.answer(decision);
    assert.deepEqual(s.cards, [], 'the card is gone');
    assert.deepEqual(s.asks(), [], 'no ask left for the strip');
    assert.equal(s.strip(), null, 'no "Go to the approval" strip');
    assert.equal(s.rows().length, 0, 'the sidebar NEEDS YOU row is gone');
    assert.deepEqual(s.refetches, [300], 'the list still refetches, as before');
  });
}

for (const status of [409, 404]) {
  test(`31y: a lapsed card (${status}) leaves with its strip and row`, async () => {
    const s = rig([approval('ap1')], () => json(status, { error: 'gone' }));
    await s.answer();
    assert.deepEqual(s.cards, []);
    assert.equal(s.strip(), null);
    assert.equal(s.rows().length, 0);
    assert.equal(s.toasts[0][0], 'This request expired');
  });
}

test('31y: a second ask still waiting in the chat keeps the strip, for that ask only', async () => {
  const s = rig([approval('ap1'), question('q1')], () => json(200, {}));
  assert.equal(s.asks().length, 2);
  await s.answer();
  assert.deepEqual(s.asks().map((a) => a.id), ['q1'], 'only the answered approval left');
  assert.ok(s.strip(), 'the strip stays');
  assert.doesNotMatch(s.strip().textContent, /Go to the approval/);
  const rows = s.rows();
  assert.equal(rows.length, 1, 'the bot still needs you');
  assert.equal(rows[0].item.id, 'q1', 'its row now names the question');
  assert.equal(rows[0].line, 'Asked you a question');
});

test('31y: a second waiting approval from the same bot stays', async () => {
  const s = rig([approval('ap1', 2), approval('ap2', 1)], () => json(200, {}));
  await s.answer();
  assert.deepEqual(s.asks().map((a) => a.id), ['ap2']);
  assert.equal(s.rows()[0].item.id, 'ap2');
});

const keeps = async (s, why) => {
  await s.answer();
  assert.equal(s.cards.length, 1, `${why}: the card stays`);
  assert.equal(s.asks().length, 1, `${why}: the strip keeps its ask`);
  assert.ok(s.strip(), `${why}: the strip shows`);
  assert.equal(s.rows().length, 1, `${why}: the sidebar row stays`);
  assert.equal(s.cleared.size, 0, `${why}: nothing cleared`);
};

test('31y: a failed send clears nothing (5xx, 401 token dialog, network error)', async () => {
  await keeps(rig([approval('ap1')], () => json(500, {})), '500');
  await keeps(rig([approval('ap1')], () => json(401, { error: 'token' })), '401');
  await keeps(rig([approval('ap1')], () => { throw new TypeError('Failed to fetch'); }), 'network');
});

test('31y: the needs_backup 409 keeps the card waiting, and so its strip and row', async () => {
  await keeps(rig([approval('ap1')], () => json(409, { status: 'needs_backup', code: 'backup_required', error: 'Take a backup first.' })), 'needs_backup');
});

test('31y: clearAnswered adds the ticket once and keeps the set when it is already there', () => {
  const a = m.away.clearAnswered(new Set(), 'ap1');
  assert.deepEqual([...a], ['ap1']);
  assert.equal(m.away.clearAnswered(a, 'ap1'), a, 'same set back: no extra render');
});

test('31y: the clear is in the same update as dropApproval, before any await in that branch', () => {
  const fn = app.slice(start, end);
  const drop = fn.indexOf('dropApproval(ticketId);');
  const clear = fn.indexOf('setClearedAsks((prev) => clearAnswered(prev, ticketId));');
  assert.ok(drop > 0 && clear > drop, 'cleared right after the card is dropped');
  assert.doesNotMatch(fn.slice(drop, clear), /await/, 'no await between them');
  assert.equal(fn.split('clearAnswered(').length - 1, 1, 'only the answered/lapsed branch clears');
});
