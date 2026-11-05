// 0.6.13 slice C (+ A3): the cards and the live line on a phone. Each test here fails on 3ca2bf7.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

// The sound effects module reads its mute flag on load.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ModernApprovalCard } from './src/components/ModernApprovalCard';
  import { WorkingStrip } from './src/components/WorkingStrip';
  import { LiveActivityList } from './src/components/ActivityList';
  import { MarkdownRenderer } from './src/components/MarkdownRenderer';
  import { QuestionCard } from './src/components/QuestionCard';
  import { EngineChip } from './src/components/Signature';
  import { ReplyMeta } from './src/components/ReplyMeta';
  export * from './src/toolLabel';
  export { keyFacts } from './src/components/ModernApprovalCard';
  export { parseQuestionMeta } from './src/questions';
  export { awayTitle } from './src/approvals';
  import { AwayView } from './src/components/AwayView';
  const r = (C) => (props) => renderToStaticMarkup(React.createElement(C, props));
  export const card = r(ModernApprovalCard);
  export const strip = r(WorkingStrip);
  export const steps = r(LiveActivityList);
  export const md = (content) => renderToStaticMarkup(React.createElement(MarkdownRenderer, { content }));
  export const question = r(QuestionCard);
  export const chip = r(EngineChip);
  export const meta = r(ReplyMeta);
  export const away = r(AwayView);
`);

const text = (html) => html.replace(/<[^>]+>/g, '').replace(/&amp;/g, '&').replace(/&#x27;/g, "'").replace(/\s+/g, ' ').trim();
const buttons = (html) => [...html.matchAll(/<button([^>]*)>([\s\S]*?)<\/button>/g)].map(([, attrs, inner]) => ({ attrs, label: text(inner) }));

// --- C2: tool names in words ----------------------------------------------------------------

test('toolLabel: the spec mappings, an MCP-prefixed name, and the fallback', () => {
  assert.equal(m.toolLabel('create_pull_request', 'github'), 'opening a pull request on GitHub');
  assert.equal(m.toolLabel('mcp__github__create_pull_request'), 'opening a pull request on GitHub');
  assert.equal(m.toolLabel('browser_navigate'), 'opening a page');
  assert.equal(m.toolLabel('ask_user', 'kwant'), 'asking you a question');
  assert.equal(m.toolLabel('web_search'), 'searching the web');
  assert.equal(m.toolLabel('list_open_tickets', 'linear'), 'using list open tickets (linear)');
  assert.equal(m.toolLabel('list_open_tickets'), 'using list open tickets');
  assert.equal(m.toolAction('create_pull_request', 'github'), 'open a pull request on GitHub');
  assert.equal(m.toolAction('list_open_tickets', 'linear'), 'use list open tickets (linear)');
});

test('step titles from the server lose their underscores', () => {
  assert.equal(m.readableStepTitle('Using `web_search`'), 'Searching the web');
  assert.equal(m.readableStepTitle('Using create_pull_request (github)'), 'Opening a pull request on GitHub');
  assert.equal(m.readableStepTitle('Using list_open_tickets (linear)'), 'Using list open tickets (linear)');
  assert.equal(m.readableStepTitle('Waiting for your approval: Using create_pull_request (github)'), 'Waiting for your OK to open a pull request on GitHub');
  assert.equal(m.readableStepTitle('Running `cargo test`'), 'Running `cargo test`', 'titles already in words stay');
  for (const t of ['Using a_b_c (x_y)', 'Using `send_email`', 'Waiting for your approval: Using push_files (github)']) {
    assert.doesNotMatch(m.readableStepTitle(t).replace(/\([^)]*\)$/, ''), /_/, t);
  }
});

// --- C1: the approval card ------------------------------------------------------------------

const pr = (over = {}) => ({
  id: 'ap1',
  tool_name: 'mcp__github__create_pull_request',
  details: {
    source: 'mcp',
    server: 'github',
    tool: 'create_pull_request',
    kwant_name: 'TechLead',
    arguments: { repo: 'acme/site', title: 'Behind the seat: timed checkout', body: 'Behind the seat, timed.', head: 'tl-checkout', base: 'main' },
  },
  ...over,
});
const noop = () => {};

test('approval card on a phone: header, one sentence, two key facts, details closed, Allow once alone on top', () => {
  const html = m.card({ approval: pr(), onResolve: noop, phone: true });
  assert.match(html, /<h3[^>]*>Approval needed<\/h3>/);
  assert.doesNotMatch(html, /Action Approval Required|Allow Once|Always Allow/, 'sentence case only');
  assert.match(text(html), /TechLead wants to open a pull request on GitHub/);
  assert.match(html, /<strong[^>]*>GitHub<\/strong>/);
  // title comes before repo in the spec's order; body/head/base stay in the details.
  assert.deepEqual(m.keyFacts(pr().details.arguments), [['title', 'Behind the seat: timed checkout'], ['repo', 'acme/site']]);
  assert.match(html, /data-approval-facts/);
  // Details start closed: the JSON is not in the markup, the toggle says so.
  assert.doesNotMatch(html, /data-approval-details=""/);
  assert.doesNotMatch(html, /&quot;head&quot;/);
  const b = buttons(html);
  assert.deepEqual(b.map((x) => x.label), ['Show details', 'Allow once', 'Deny', 'Always allow']);
  assert.match(b[0].attrs, /aria-expanded="false"/);
  assert.match(b[0].attrs, /min-h-\[44px\]/);
  assert.match(b[1].attrs, /w-full/);
  assert.match(b[1].attrs, /min-h-\[48px\]/);
  assert.match(b[1].attrs, /bg-\[#059669\]/);
  for (const x of b.slice(2)) assert.match(x.attrs, /min-h-\[44px\]/, x.label);
  // Allow once is the only child of its row; Deny and Always allow share the next one.
  assert.match(html, /data-approval-actions[^>]*>\s*<button[^>]*data-approval-allow[\s\S]*?<\/button>\s*<div class="grid gap-2 sm:flex grid-cols-2">/);
});

test('approval card on desktop: details open, the JSON wraps between words, never break-all', () => {
  const html = m.card({ approval: pr(), onResolve: noop, phone: false });
  assert.match(html, /aria-expanded="true"[^>]*>Hide details/);
  const pre = /<pre class="([^"]*)"[^>]*>([\s\S]*?)<\/pre>/.exec(html);
  assert.ok(pre, 'the arguments are shown');
  assert.match(pre[2], /Behind the seat/);
  assert.doesNotMatch(pre[1], /break-all/);
  assert.match(pre[1], /whitespace-pre-wrap/);
  assert.match(pre[1], /\[overflow-wrap:anywhere\]/);
  assert.match(pre[1], /max-h-\[240px\]/);
  assert.match(pre[1], /text-xs leading-\[18px\]/);
});

test('a payment card offers no Always allow on a phone either: its second row is just Deny', () => {
  const payment = pr({ details: { ...pr().details, payment_reason: 'stripe create payment is a money action' } });
  const html = m.card({ approval: payment, onResolve: noop, phone: true });
  assert.deepEqual(buttons(html).map((x) => x.label), ['Show details', 'Allow once', 'Deny']);
  assert.match(html, /<div class="grid gap-2 sm:flex grid-cols-1">/);
  assert.doesNotMatch(html, /Always/);
});

test('a still-open card keeps its note and puts Allow and continue on the first row', () => {
  const html = m.card({ approval: pr({ waiting: true }), onResolve: noop, phone: true });
  assert.match(html, /stopped waiting for an answer/);
  const b = buttons(html).map((x) => x.label);
  assert.deepEqual(b, ['Show details', 'Allow and continue', 'Deny']);
});

test('the header shows when the bot asked, when known', () => {
  const html = m.card({ approval: pr({ asked_at: new Date().toISOString() }), onResolve: noop, phone: true });
  assert.match(html, /<time[^>]*text-\[#a39dcc\][^>]*>[^<]+<\/time>/);
  assert.doesNotMatch(m.card({ approval: pr(), onResolve: noop, phone: true }), /<time/);
});

// --- C5: inline code ------------------------------------------------------------------------

test('inline code wraps anywhere and keeps its box on every line', () => {
  const html = m.md('Set `KWANT_SUPERCALIFRAGILISTIC_EXPIALIDOCIOUS_TIMEOUT_SECONDS` to 30.');
  const code = /<code class="([^"]*)"[^>]*>KWANT_SUPER/.exec(html);
  assert.ok(code, 'inline code rendered');
  for (const c of ['[overflow-wrap:anywhere]', '[box-decoration-break:clone]', '[-webkit-box-decoration-break:clone]']) assert.ok(code[1].includes(c), c);
});

// --- A3 + C2: one live line -----------------------------------------------------------------

const live = (steps) => ({ turnId: 't1', thinking: {}, steps });
const step = (over = {}) => ({ id: 's1', kind: 'tool', status: 'running', title: 'Using `web_search`', started_at: '2026-11-05T10:00:00Z', ...over });

test('on a phone the strip is the one live line: 44 px, 14 px text, words not tool names', () => {
  const html = m.strip({ active: true, name: 'Architect', title: 'Architect', live: live([step()]), phone: true });
  assert.match(html, /min-h-\[44px\]/);
  assert.match(html, /text-sm leading-5/);
  assert.match(html, /tabular-nums[^"]*text-\[#a39dcc\]|text-\[#a39dcc\][^"]*tabular-nums/);
  assert.match(text(html), /Architect is searching the web/);
  assert.doesNotMatch(text(html), /_/);
});

test('waiting on a question, the strip says so and offers Answer, which goes to the card', () => {
  const calls = [];
  const html = m.strip({ active: true, name: 'QA', title: 'QA', awaitingAnswer: true, phone: true, onAnswer: (on) => calls.push(on) });
  assert.match(text(html), /QA is waiting for your answer/);
  assert.match(html, /data-waiting-on="question"/);
  const answer = buttons(html).find((b) => b.label === 'Answer');
  assert.ok(answer, 'Answer button');
  assert.match(answer.attrs, /min-h-\[44px\]/);
  assert.match(answer.attrs, /data-strip-answer/);
  // Desktop keeps its old strip: no Answer button there.
  assert.ok(!buttons(m.strip({ active: true, name: 'QA', title: 'QA', awaitingAnswer: true, phone: false, onAnswer: noop })).some((b) => b.label === 'Answer'));
});

test('on a phone the steps panel is one 44 px row, the engine note behind an info button', () => {
  const note = step({ id: 'n', kind: 'note', status: 'ok', title: 'This engine doesn’t share its thinking.' });
  const html = m.steps({ live: live([step(), note]), idBase: 'x', compact: true });
  assert.match(html, /data-steps-compact/);
  assert.doesNotMatch(text(html), /doesn’t share its thinking/, 'the disclaimer is not on screen');
  assert.doesNotMatch(text(html), /Working ·/);
  const b = buttons(html);
  assert.match(b[0].label, /^Steps · 1/);
  assert.match(b[0].attrs, /min-h-\[44px\]|h-11/);
  assert.match(b[0].attrs, /aria-expanded="false"/);
  const info = b.find((x) => /aria-label="About this engine/.test(x.attrs));
  assert.ok(info, 'info button');
  assert.match(info.attrs, /w-11 h-11|h-11 w-11/);
});

// --- C3 and C4 ------------------------------------------------------------------------------

test('question card: a short placeholder on one line, a 16 px field and 44 px targets', () => {
  const meta = m.parseQuestionMeta({ kind: 'ask_user', question_id: 'q1', question: 'Which hotel?', choices: ['Rifugio', 'Hotel'], status: 'open', answer: null });
  const html = m.question({ meta, messageId: 'm1', kwantName: 'QA', waiting: true, onAnswer: async () => {}, onDismiss: async () => {} });
  const field = /<textarea([^>]*)>/.exec(html)[1];
  assert.match(field, /placeholder="Your own answer…"/);
  assert.match(field, /text-base/);
  assert.match(field, /placeholder:whitespace-nowrap/);
  const b = buttons(html);
  for (const c of b.filter((x) => /aria-pressed/.test(x.attrs))) {
    assert.match(c.attrs, /w-full/, c.label);
    assert.match(c.attrs, /min-h-\[48px\]/, c.label);
    assert.match(c.attrs, /text-base leading-\[22px\]/, c.label);
  }
  const skip = b.find((x) => x.label === 'Skip question');
  assert.match(skip.attrs, /min-h-\[44px\]/);
  assert.match(skip.attrs, /text-sm/);
  assert.match(skip.attrs, /text-\[#a39dcc\]/);
  assert.match(html, /w-11 h-11/, 'a 44 px send button');
  // The strip already says the bot waits: on a phone the card does not say it again (C2: at most twice).
  assert.match(html, /<span class="[^"]*max-md:hidden[^"]*" data-question-waiting="">/);
});

test('engine chip: the model alone on a phone, the full label from sm up and on hover', () => {
  const html = m.chip({ label: 'Claude · claude-opus-5-5 · Booktuning', short: 'claude-opus-5-5' });
  assert.match(html, /title="Claude · claude-opus-5-5 · Booktuning"/);
  assert.match(html, /<span class="[^"]*sm:hidden[^"]*max-w-\[160px\][^"]*"[^>]*>claude-opus-5-5<\/span>/);
  assert.match(html, /<span class="[^"]*hidden sm:inline[^"]*">Claude · claude-opus-5-5 · Booktuning<\/span>/);
});

test('reply header on a phone: one row, the time never shrinks, the chip gives way', () => {
  const html = m.meta({ name: 'Architect', at: '2026-11-02T11:42:00Z', now: Date.parse('2026-11-02T12:00:00Z'), ranOn: 'Custom API · claude-opus-5-5', account: 'Anthropic API' });
  assert.match(html, /<div class="[^"]*flex-nowrap sm:flex-wrap[^"]*">/);
  assert.match(html, /<span class="[^"]*shrink-0 whitespace-nowrap[^"]*" data-reply-time="">/);
  assert.match(html, /<span class="[^"]*sm:hidden[^"]*max-w-\[160px\][^"]*"[^>]*>claude-opus-5-5<\/span>/);
});

// --- Rigel's #309 HOLD on b48683c ------------------------------------------------------------

import { readFileSync } from 'node:fs';
const src = (p) => readFileSync(new URL(`../${p}`, import.meta.url), 'utf8');

test('C1.5d, C3.1c: Allow once and the choices beat the 44 px coarse-pointer rule with an important 48 px', () => {
  // modern.css's phone rule is (0,2,0) and stays as it is; a plain .min-h-[48px] (0,1,0) loses to it.
  assert.match(src('src/styles/modern.css'), /\(pointer: coarse\)[\s\S]*?:is\(button[\s\S]*?min-height: 44px/);
  const allow = buttons(m.card({ approval: pr(), onResolve: noop, phone: true })).find((x) => x.label === 'Allow once');
  assert.match(allow.attrs, /max-sm:!min-h-\[48px\]/);
  assert.match(allow.attrs, /sm:min-h-\[36px\]/, 'desktop keeps its compact button');
  const meta = m.parseQuestionMeta({ kind: 'ask_user', question_id: 'q1', question: 'Which hotel?', choices: ['Rifugio', 'Hotel'], status: 'open', answer: null });
  const choices = buttons(m.question({ meta, messageId: 'm1', kwantName: 'QA', waiting: true, onAnswer: async () => {}, onDismiss: async () => {} })).filter((x) => /aria-pressed/.test(x.attrs));
  assert.equal(choices.length, 2);
  for (const c of choices) assert.match(c.attrs, /max-sm:!min-h-\[48px\]/, c.label);
});

const awayApproval = (over = {}) => ({
  kind: 'approval', id: 'ap1', kwant_id: 'k1', kwant_name: 'TechLead', title: 'Use create_pull_request (github)',
  summary: 'acme/site', status: 'pending', timestamp: '2026-11-05T08:00:00Z',
  link: { thread_id: 't1', channel_id: null, direct_kwant_id: null },
  tool_name: 'mcp__github__create_pull_request', details: pr().details, ...over,
});

test('C2.3: an Inbox approval is titled like its card, never by the raw tool name', () => {
  assert.equal(m.awayTitle(awayApproval()), 'Open a pull request on GitHub');
  // An older server: only the title and a summary.
  assert.equal(m.awayTitle(awayApproval({ tool_name: undefined, details: undefined, title: 'Use Bash' })), 'Run a command');
  const q = { ...awayApproval(), kind: 'question', title: 'Which hotel?' };
  assert.equal(m.awayTitle(q), 'Which hotel?', 'other items keep their title');
  const data = { since: '2026-11-05T07:00:00Z', last_seen: null, now: '2026-11-05T08:05:00Z', stuck_after_minutes: 30, waiting: [awayApproval()], finished: [], failed: [], unseen: { waiting: 1, finished: 0, failed: 0, total: 1 } };
  const html = m.away({ data, loading: false, error: null, now: Date.parse('2026-11-05T08:05:00Z'), onRefresh: noop, onOpen: noop, onAnswer: noop, onResolve: noop });
  // The item's title line and its accessible name; the card's own "Show details" may still show the raw call.
  const titles = [...html.matchAll(/<p class="mt-0\.5 [^"]*">([^<]*)<\/p>/g)].map((x) => x[1]);
  assert.deepEqual(titles, ['Open a pull request on GitHub']);
  assert.match(html, /aria-label="Open a pull request on GitHub, /);
  assert.doesNotMatch(html, /Use create_pull_request/);
  // The card says the same words.
  assert.match(text(m.card({ approval: pr(), onResolve: noop, phone: true })), /wants to open a pull request on GitHub/);
});

test('ChatView keeps one phone flag after the rebase on #307: useMedia(PHONE_QUERY)', () => {
  const chat = src('src/components/ChatView.tsx');
  assert.deepEqual([...chat.matchAll(/const phone\b[^;\n]*/g)].map((x) => x[0]), ['const phone = useMedia(PHONE_QUERY)']);
  assert.doesNotMatch(chat, /usePhoneLayout/);
});
