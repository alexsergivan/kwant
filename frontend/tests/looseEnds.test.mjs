// Loose ends (Zombie Hunter, plan item 6.6): the panel shows stalled things with their evidence and a
// draft the person sends themselves. Nothing in it sends. Adding a mailbox while approvals are off
// leads to "Move to Bitwarden", not an error.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`export * from './src/looseEnds';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { LooseEndsView, LooseCaseCard, LooseSources } from './src/components/LooseEndsView';
  export const view = (props) => renderToStaticMarkup(React.createElement(LooseEndsView, props));
  export const card = (props) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(LooseCaseCard, props)));
  export const sources = (props) => renderToStaticMarkup(React.createElement(LooseSources, props));
`);

// Shaped like CaseView in src/zombie/api.rs.
const mailCase = (over = {}) => ({
  id: 'c1', status: 'open', kind: 'awaiting_reply', conv_kind: 'email', source: 'imap', source_id: 's-mail',
  title: 'Invoice 4411 for the October print run', counterpart: 'billing@printer.example', counterpart_name: 'Dana at Printer Co',
  reference: 'INV-4411', link: 'https://mail.example/thread/1',
  why: 'You asked for a corrected invoice 12 days ago and nobody answered.',
  evidence: [{ quote: 'Could you resend it with our VAT number?', from: 'you@example.com', at: '2026-10-08T09:00:00Z', outbound: true }],
  draft: {
    channel: 'email', to: 'billing@printer.example', subject: 'Re: Invoice 4411', body: 'Hi Dana,\njust checking in on the corrected invoice.',
    in_reply_to: '<m1@example>', mailto: 'mailto:billing@printer.example?subject=Re%3A%20Invoice%204411', open_url: null,
    kwant_thread_id: null, generated_by: 'template', created_at: '2026-10-20T08:00:00Z',
  },
  age_days: 12, last_activity_at: '2026-10-08T09:00:00Z', first_seen: '2026-10-19T08:00:00Z', status_changed_at: '2026-10-19T08:00:00Z',
  snoozed_until: null, waiting_since: null, follow_ups: 0, deadline: '2026-10-25', resolution: null, dismiss_reason: null, space_id: null,
  ...over,
});
const noop = () => {};
const viewProps = (over = {}) => ({
  cases: [mailCase()], loading: false, running: false, error: null, tab: 'open', now: Date.parse('2026-10-20T12:00:00Z'),
  sources: null, onTab: noop, onRefresh: noop, onScan: noop, onAction: noop, ...over,
});

test('each status lands on one tab and gets only the actions that make sense there', () => {
  assert.equal(m.tabOf('open'), 'open');
  assert.equal(m.tabOf('waiting_reply'), 'waiting');
  assert.equal(m.tabOf('snoozed'), 'snoozed');
  assert.equal(m.tabOf('closed'), 'done');
  assert.equal(m.tabOf('dismissed'), 'done');
  assert.deepEqual(m.actionsFor(mailCase()), ['draft', 'followed-up', 'snooze', 'dismiss', 'done']);
  assert.deepEqual(m.actionsFor(mailCase({ draft: null })), ['followed-up', 'snooze', 'dismiss', 'done'], 'no "Draft again" without a draft (the API answers 422)');
  assert.deepEqual(m.actionsFor(mailCase({ status: 'snoozed' })), ['reopen', 'done', 'dismiss']);
  assert.deepEqual(m.actionsFor(mailCase({ status: 'closed' })), ['reopen']);
  for (const a of ['draft', 'snooze', 'dismiss', 'done', 'followed-up', 'reopen']) assert.ok(m.ACTION_LABELS[a]);
});

test('a card shows who, why, the deadline, quoted evidence with its source, and a draft that is never sent', () => {
  const html = ui.card({ c: mailCase(), busy: false, now: Date.parse('2026-10-20T12:00:00Z'), onAction: noop });
  for (const text of ['Email', 'Invoice 4411', 'Dana at Printer Co (billing@printer.example)', 'nobody answered', 'INV-4411', 'Could you resend it with our VAT number?', 'Open the original', 'Copy draft', 'Open in my mail app', 'Draft again', 'I followed up', 'Snooze', 'Dismiss', 'Mark done']) {
    assert.ok(html.includes(text), `card shows "${text}"`);
  }
  assert.match(html, /Draft only — copy it and send it yourself/);
  assert.match(html, /<blockquote/);
  assert.match(html, /href="https:\/\/mail\.example\/thread\/1"/);
  assert.match(html, /href="mailto:billing@printer\.example\?subject=/);
  assert.doesNotMatch(html, />Send</, 'there is no Send button');
});

test('the card is generic: an unknown source and kind still render with readable labels', () => {
  const html = ui.card({ c: mailCase({ source: 'camera', kind: 'letter_unanswered', link: null, draft: null, deadline: null }), busy: false, onAction: noop });
  assert.match(html, /Camera/);
  assert.match(html, /Letter unanswered/);
  assert.doesNotMatch(html, /Open the original/);
});

test('links only open https or mailto, never a script or a kwant:// address', () => {
  assert.equal(m.safeHref('https://github.com/a/b/issues/1'), 'https://github.com/a/b/issues/1');
  assert.equal(m.safeHref('javascript:alert(1)'), null);
  assert.equal(m.safeHref('kwant://thread/t1'), null);
  assert.equal(m.safeHref('http://plain.example'), null);
  assert.equal(m.safeHref('mailto:a@b.example', 'mailto'), 'mailto:a@b.example');
  const gh = m.draftOpen({ channel: 'github', body: 'x', open_url: 'https://github.com/a/b/issues/1#new_comment_field', mailto: null });
  assert.deepEqual(gh, { href: 'https://github.com/a/b/issues/1#new_comment_field', label: 'Open on GitHub' });
  assert.equal(m.draftOpen({ channel: 'email', body: 'x', mailto: 'javascript:x', open_url: null }), null);
});

test('a bot-chat case opens its chat in the app when it names the bot, and shows no chat link without it', () => {
  const chatCase = (over = {}) => mailCase({
    source: 'kwant', conv_kind: 'kwant_thread', kind: 'unanswered_question', counterpart: null, counterpart_name: 'Ada',
    link: 'kwant://thread/t-9', kwant_id: 'k-ada', draft: null, ...over,
  });
  assert.deepEqual(m.caseChat(chatCase()), { kwantId: 'k-ada', threadId: 't-9' });
  assert.deepEqual(m.caseChat(chatCase({ link: null, draft: { ...mailCase().draft, channel: 'kwant', kwant_thread_id: 't-3' } })), { kwantId: 'k-ada', threadId: 't-3' });
  // Cases found before kwant_id existed, and mail cases, have no chat to open.
  assert.equal(m.caseChat(chatCase({ kwant_id: null })), null);
  assert.equal(m.caseChat(chatCase({ link: null })), null);
  assert.equal(m.caseChat(mailCase()), null);

  const opened = [];
  const withChat = ui.card({ c: chatCase(), busy: false, onAction: noop, onOpenChat: (k, t) => opened.push([k, t]) });
  assert.match(withChat, /Open chat/);
  assert.doesNotMatch(withChat, /kwant:\/\//, 'the kwant:// address is never rendered as a link');
  assert.doesNotMatch(ui.card({ c: chatCase({ kwant_id: null }), busy: false, onAction: noop, onOpenChat: noop }), /Open chat/);
  assert.doesNotMatch(ui.card({ c: chatCase(), busy: false, onAction: noop }), /Open chat/, 'no handler, no link');
  assert.doesNotMatch(ui.card({ c: mailCase(), busy: false, onAction: noop, onOpenChat: noop }), /Open chat/);
  assert.match(ui.view(viewProps({ cases: [chatCase()], onOpenChat: noop })), /Open chat/);
});

test('no cases shows the empty state, other tabs say what goes there, and a scan in progress is shown', () => {
  const empty = ui.view(viewProps({ cases: [] }));
  assert.match(empty, /No loose ends/);
  assert.match(empty, /data-empty/);
  assert.match(empty, /never send anything/);
  const snoozed = ui.view(viewProps({ tab: 'snoozed' }));
  assert.match(snoozed, /Nothing snoozed/);
  assert.match(snoozed, /Open <span[^>]*>1</);
  assert.match(ui.view(viewProps({ running: true })), /Scanning…/);
  assert.match(ui.view(viewProps({ cases: null, loading: true })), /Loading…/);
  assert.match(ui.view(viewProps({ cases: null, error: 'Server down.' })), /Couldn’t load loose ends: Server down\./);
  const token = ui.view(viewProps({ cases: null, needsToken: true }));
  assert.match(token, /Confirm it’s you/);
});

test('the IMAP form refuses plain text to another computer and only sends self-signed for this one', () => {
  const base = { label: '', host: 'imap.example.com', port: '', security: 'tls', username: 'me@example.com', folders: '', accept_invalid_certs: true };
  const tls = m.imapSource(base, 'pw');
  assert.equal(tls.kind, 'imap');
  assert.equal(tls.config.port, 993);
  assert.equal(tls.config.security, 'tls');
  assert.equal(tls.config.accept_invalid_certs, false, 'self-signed only for a server on this computer');
  assert.ok('error' in m.imapSource({ ...base, security: 'plain' }, 'pw'));
  const bridge = m.imapSource({ ...base, host: '127.0.0.1', port: '1143', security: 'starttls' }, 'pw');
  assert.equal(bridge.config.port, 1143);
  assert.equal(bridge.config.accept_invalid_certs, true);
  assert.ok('error' in m.imapSource({ ...base, host: '' }, 'pw'));
  assert.ok('error' in m.imapSource(base, ''));
  assert.ok('error' in m.githubSource('', '', ''));
  assert.equal(m.githubSource('', '', 'tok').kind, 'github');
});

test('a 409 needs_bitwarden becomes a Bitwarden step, a 423 an unlock, and other errors read as sentences', async () => {
  const real = globalThis.fetch;
  const answer = (status, body) => (globalThis.fetch = async () => new Response(JSON.stringify(body), { status }));
  try {
    answer(409, { needs_bitwarden: true, error: 'a space or bot is on "Ask only when risky": connect Bitwarden first' });
    await assert.rejects(m.looseApi.addSource({ kind: 'imap', label: 'x', config: {}, secret: 's' }), (e) => e.needsBitwarden && e.status === 409);
    answer(423, { locked: true, error: 'the vault is locked' });
    await assert.rejects(m.looseApi.addSource({ kind: 'imap', label: 'x', config: {}, secret: 's' }), (e) => e.locked && !e.needsBitwarden);
    answer(400, { error: 'connection test failed: login refused' });
    await assert.rejects(m.looseApi.addSource({ kind: 'imap', label: 'x', config: {}, secret: 's' }), /^Error: Connection test failed: login refused\.$/);
    globalThis.fetch = async () => new Response('not found', { status: 404 });
    await assert.rejects(m.looseApi.cases(), /doesn’t have loose ends yet/);
  } finally {
    globalThis.fetch = real;
  }
});

test('the built-in kwant source cannot be removed; added ones can be tested, paused and removed', () => {
  const html = ui.sources({
    sources: [
      { id: 'kwant', kind: 'kwant', label: 'kwant conversations', enabled: true, builtin: true, config: {}, has_secret: false, last_scan_at: null, last_error: null, created_at: '2026-10-19T00:00:00Z' },
      { id: 's1', kind: 'imap', label: 'Work mail', enabled: true, builtin: false, config: {}, has_secret: true, last_scan_at: null, last_error: null, created_at: '2026-10-19T00:00:00Z' },
    ],
    error: null, onChanged: noop,
  });
  assert.equal((html.match(/ Remove</g) ?? []).length, 1);
  assert.match(html, /Test/);
  assert.match(html, /Pause/);
  assert.match(html, /built in/);
  const form = ui.sources({ sources: [], error: null, onChanged: noop, initialAdding: true });
  assert.match(form, /type="password"/);
  assert.match(form, /Test and add/);
  assert.match(form, /disabled="">Not encrypted/, 'plain text is off until the server is this computer');
});
