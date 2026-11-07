// 0.7.6 J12 (Vega): "Matches the ticket" on the review card, when the PR works on a Jira ticket with a brief.
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export * from './src/dev/types';
  export * from './src/dev/logic';
  export * as fx from './src/dev/fixtures';
  export { ReviewCard } from './src/components/dev/ReviewCard';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);
const { fx } = m;
const html = (card) => m.renderToStaticMarkup(m.createElement(m.ReviewCard, { card, onFinding: async () => card, forge: 'github', now: fx.FIXTURE_NOW }));
const flat = (s) => s.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ');

const ticket = (rows) =>
  m.parseReview({
    ...fx.REVIEW_294,
    intent: { source: 'ticket', key: 'TRAIL-101', url: 'https://yourcompany.atlassian.net/browse/TRAIL-101', asked: '', none: false, rows },
  });
const row = (state, text, extra = {}) => ({ done_when_index: 0, state, text, evidence_text: '', evidence_paths: [], ...extra });

test('the ticket states parse, and an unknown state is never shown as done', () => {
  const c = ticket([
    row('met', 'The link is in the email', { evidence_text: 'templates/order.html adds it', evidence_paths: ['templates/order.html'] }),
    row('not_met', 'It opens the PDF'),
    row('unclear', 'It works on mobile'),
    row('done!', 'Something odd'),
  ]);
  assert.deepEqual(c.intent.rows.map((r) => r.state), ['met', 'not_met', 'unclear', 'unclear']);
  assert.equal(c.intent.key, 'TRAIL-101');
  assert.deepEqual(c.intent.rows[0].evidence_paths, ['templates/order.html']);
});

test('LGTM needs every done-when line met', () => {
  const clean = (rows) => ({
    ...ticket(rows),
    risk: { level: 'low', reasons: [] },
    tamper: { flags: [], unread: [] },
    findings: [],
  });
  assert.equal(m.lgtm(clean([row('met', 'a', { evidence_paths: ['x'] })])), true);
  assert.equal(m.lgtm(clean([row('met', 'a'), row('not_met', 'b')])), false, 'not met');
  assert.equal(m.lgtm(clean([row('met', 'a'), row('unclear', 'b')])), false, 'unclear');
});

test('the card says Matches the ticket, with its states and evidence', () => {
  const out = flat(html(ticket([row('met', 'The link is in the email', { evidence_text: 'templates/order.html adds it' }), row('not_met', 'It opens the PDF'), row('unclear', 'It works on mobile')])));
  assert.match(out, /Matches the ticket TRAIL-101/);
  assert.doesNotMatch(out, /Did it do what was asked/);
  assert.match(out, /Not met/);
  assert.match(out, /Unclear/);
  assert.match(out, /templates\/order\.html adds it/);
});
