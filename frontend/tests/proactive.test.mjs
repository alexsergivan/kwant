import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

// The markdown renderer pulls in the UI sounds, which read localStorage when loaded.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const p = await loadBundled(`export * from './src/decideFirst';`);
const notify = await loadBundled(`export * from './src/continuity';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { DecisionChips, DecidingLine, WorkAheadBody } from './src/components/Proactive';
  import { QuestionCard } from './src/components/QuestionCard';
  const r = (C) => (props) => renderToStaticMarkup(React.createElement(C, props));
  export const chips = r(DecisionChips);
  export const deciding = r(DecidingLine);
  export const workAhead = r(WorkAheadBody);
  export const question = r(QuestionCard);
`);
const src = (path) => readFileSync(new URL(`../src/${path}`, import.meta.url), 'utf8');

// The example from contract.md §1
const DESIGN = {
  index: 0,
  question: 'Which design direction?',
  options: ['A: Classic', 'B: Wax seal', 'C: Green desk'],
  picked: 'B: Wax seal',
  why: 'Matches the wax-seal hero you asked for and stays easy to read.',
  status: 'decided',
  changed_to: null,
};
const meta = (over = {}) =>
  JSON.stringify({ decisions: [DESIGN], decide_first: { consulted: ['Designer'], held_turn_id: 't1', kwant_id: 'lead' }, ...over });

test('decisions parse from the contract shape, as a string or an object', () => {
  const [d] = p.parseDecisions(meta());
  assert.deepEqual(d, DESIGN);
  assert.equal(p.parseDecisions(JSON.parse(meta())).length, 1);
  assert.deepEqual(p.consultedNames(meta()), ['Designer']);
});

test('junk, other metas and decisions without a pick give nothing', () => {
  for (const raw of [null, '', 'not json', '{"kind":"ask_user"}', '{"decisions":"x"}', JSON.stringify({ decisions: [{ question: 'Q', options: ['a'] }, 7, null] })])
    assert.deepEqual(p.parseDecisions(raw), []);
  assert.deepEqual(p.consultedNames('{"decisions":[]}'), []);
  // A missing index falls back to the position
  assert.equal(p.parseDecisions(JSON.stringify({ decisions: [{ ...DESIGN, index: undefined }] }))[0].index, 0);
});

test('a changed decision shows the person’s pick and offers no second Change', () => {
  const [d] = p.parseDecisions(meta({ decisions: [{ ...DESIGN, status: 'changed', changed_to: 'A: Classic' }] }));
  assert.equal(p.decisionLabel(d), 'You chose: A: Classic');
  assert.deepEqual(p.changeOptions(d), []);
  // "changed" without changed_to is treated as still decided
  const [odd] = p.parseDecisions(meta({ decisions: [{ ...DESIGN, status: 'changed' }] }));
  assert.equal(odd.status, 'decided');
  assert.equal(p.decisionLabel(DESIGN), 'Decided: B: Wax seal');
  assert.deepEqual(p.changeOptions(DESIGN), ['A: Classic', 'C: Green desk']);
});

test('consulted line reads naturally', () => {
  assert.equal(p.consultedLine([]), '');
  assert.equal(p.consultedLine(['Designer']), 'Asked Designer first.');
  assert.equal(p.consultedLine(['Designer', 'Writer']), 'Asked Designer and Writer first.');
  assert.equal(p.decidingDetail('Mira', ['Designer']), 'Mira is checking with Designer and making the call, instead of asking you.');
  assert.match(p.decidingDetail('Mira', []), /^Mira is running the checks it listed/);
});

test('Change posts the new pick to the contract route', async () => {
  const calls = [];
  const prev = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    calls.push([init.method, url, JSON.parse(init.body)]);
    return { ok: true, status: 200, json: async () => ({ message: { id: 'm 1' }, prompt: '@Mira Change of decision' }) };
  };
  try {
    const res = await p.decisionsApi.change('m 1', 0, 'A: Classic');
    assert.equal(res.prompt, '@Mira Change of decision');
    assert.deepEqual(calls, [['POST', '/api/messages/m%201/decisions/0/change', { picked: 'A: Classic' }]]);
    // 409 surfaces the server's words
    globalThis.fetch = async () => ({ ok: false, status: 409, json: async () => ({ error: 'Already changed.' }) });
    await assert.rejects(p.decisionsApi.change('m1', 0, 'A: Classic'), /Already changed\./);
  } finally {
    globalThis.fetch = prev;
  }
});

test('decide_updated: deciding adds the chat, any done removes it', () => {
  const ev = (data) => ({ type: 'decide_updated', data: { kwant_id: 'lead', context_id: 'ch1', thread_id: null, held_turn_id: 't1', ...data } });
  let m = p.applyDecideEvent({}, ev({ state: 'deciding', consulted: [] }));
  assert.deepEqual(m, { ch1: { kwantId: 'lead', heldTurnId: 't1', consulted: [] } });
  m = p.applyDecideEvent(m, ev({ state: 'deciding', consulted: ['Designer'] }));
  assert.deepEqual(m.ch1.consulted, ['Designer']);
  for (const outcome of ['decided', 'posted_original', 'cancelled']) assert.deepEqual(p.applyDecideEvent(m, ev({ state: 'done', outcome })), {});
  // Unrelated events and a done for a chat not deciding keep the same map
  assert.equal(p.applyDecideEvent(m, { type: 'turn_started', data: {} }), m);
  const empty = {};
  assert.equal(p.applyDecideEvent(empty, ev({ state: 'done' })), empty);
});

test('after a reload, turns with purpose "decide" bring the line back', () => {
  const m = p.hydrateDeciding([
    { turn_id: 'a', kwant_id: 'designer', context_id: 'ch1', purpose: 'decide' },
    { turn_id: 'b', kwant_id: 'lead', context_id: 'ch1', purpose: 'decide' },
    { turn_id: 'c', kwant_id: 'writer', context_id: 'ch2' },
  ]);
  assert.deepEqual(Object.keys(m), ['ch1']);
  assert.deepEqual(p.hydrateDeciding(undefined), {});
});

test('work-ahead meta and its one-line summary', () => {
  assert.deepEqual(p.parseWorkAhead('{"kind":"work_ahead","date":"2026-10-26","run_id":"r1"}'), { kind: 'work_ahead', date: '2026-10-26', run_id: 'r1' });
  assert.equal(p.parseWorkAhead('{"kind":"ask_user"}'), null);
  assert.equal(p.workAheadSummary('\n## While you were away I **ran** the contrast check.\n\n- A passes'), 'While you were away I ran the contrast check.');
  assert.equal(p.workAheadSummary('x'.repeat(200)).length, 140);
});

test('settings: parsed only from a server that has them, cap kept in 1–10', () => {
  assert.equal(p.parseProactiveSettings({ agent_tools: true }), null);
  assert.deepEqual(p.parseProactiveSettings({ decide_first: false, work_ahead: true, work_ahead_daily_cap: 5 }), {
    decide_first: false,
    work_ahead: true,
    work_ahead_daily_cap: 5,
  });
  assert.deepEqual(p.parseProactiveSettings({ decide_first: true }), { decide_first: true, work_ahead: true, work_ahead_daily_cap: 3 });
  assert.equal(p.parseProactiveSettings({ decide_first: true, work_ahead_daily_cap: 40 }).work_ahead_daily_cap, 10);
});

test('work_ahead notifications: listed and saved only where the server has the setting', () => {
  const ids = (a, w) => notify.notifyKindsFor(a, w).map((k) => k.id);
  assert.ok(!ids(false, false).includes('work_ahead'));
  assert.ok(ids(false, true).includes('work_ahead'));
  assert.equal(ids(true, true).at(-1), 'app_update');
  assert.deepEqual(notify.notifyOnToSave(['turn_finished', 'work_ahead'], false, false), ['turn_finished']);
  assert.deepEqual(notify.notifyOnToSave(['turn_finished', 'work_ahead'], false, true), ['turn_finished', 'work_ahead']);
  assert.equal(notify.shouldShowOsNotification({ enabled: true, permission: 'granted', hidden: true, kind: 'work_ahead' }), true);
});

test('chips: question, pick, why, who was asked, and a Change button', () => {
  const html = ui.chips({ messageId: 'm1', decisions: [DESIGN], consulted: ['Designer'], onChange: async () => {} });
  assert.match(html, /aria-label="Decided by the team"/);
  assert.match(html, /Which design direction\?/);
  assert.match(html, /Decided: B: Wax seal/);
  assert.match(html, /Why: Matches the wax-seal hero/);
  assert.match(html, /Asked Designer first\./);
  assert.match(html, /aria-expanded="false"[^>]*>Change</);
  // Without a handler, or once changed, there's nothing to press
  assert.doesNotMatch(ui.chips({ messageId: 'm1', decisions: [DESIGN] }), />Change</);
  const changed = ui.chips({ messageId: 'm1', decisions: [{ ...DESIGN, status: 'changed', changed_to: 'A: Classic' }], onChange: async () => {} });
  assert.match(changed, /You chose: A: Classic/);
  assert.doesNotMatch(changed, />Change</);
  assert.equal(ui.chips({ messageId: 'm1', decisions: [] }), '');
});

test('"Team is deciding…" is a polite status with plain words', () => {
  const html = ui.deciding({ botName: 'Mira', consulted: ['Designer'] });
  assert.match(html, /role="status"/);
  assert.match(html, /aria-live="polite"/);
  assert.match(html, /Team is deciding…/);
  assert.match(html, /Mira is checking with Designer/);
});

test('"While you were away" starts collapsed to one line', () => {
  const html = ui.workAhead({ messageId: 'm9', text: 'While you were away I ran the contrast check.\n\nNext I expect you’ll want the CSS.' });
  assert.match(html, /While you were away<\/span>/);
  assert.match(html, /aria-expanded="false"/);
  assert.match(html, /aria-controls="wa-m9"/);
  assert.match(html, /id="wa-m9" hidden=""/);
  assert.doesNotMatch(html, /Next I expect/);
});

test('question cards say why only the person can answer, when the bot said', () => {
  const base = { kind: 'ask_user', question_id: 'q1', question: 'Send the email to Ana?', choices: ['Send', 'Not yet'], status: 'open', answer: null };
  const props = { messageId: 'm1', kwantName: 'Mira', waiting: true, onAnswer: async () => {}, onDismiss: async () => {} };
  const html = ui.question({ ...props, meta: { ...base, why_you: 'outside', checked: 'Drafted it and checked the address.' } });
  assert.match(html, /Only you can answer: It sends something or contacts someone\./);
  assert.match(html, /Already tried: Drafted it and checked the address\./);
  const old = ui.question({ ...props, meta: base });
  assert.doesNotMatch(old, /Only you can answer|Already tried/);
  assert.equal(p.whyYouText('nonsense'), null);
});

test('wiring: chat view, app and settings', () => {
  const chat = src('components/ChatView.tsx');
  // The line stands in for the streamed draft while deciding
  assert.match(chat, /\{deciding && <DecidingLine/);
  assert.match(chat, /\{isStreaming && !deciding && routingState\?\.phase !== 'deciding'/);
  assert.match(chat, /<DecisionChips/);
  assert.match(chat, /<WorkAheadBody messageId=\{msg\.id\}/);

  const app = src('App.tsx');
  assert.match(app, /case 'decide_updated':/);
  assert.match(app, /setDeciding\(hydrateDeciding\(list\)\)/);
  // message_added with a known id replaces it (a changed decision)
  assert.match(app, /if \(list\.some\(\(x\) => x\.id === m\.id\)\) \{\s*const replaced = list\.map/);
  // Change: record, then send the server's prompt to the same chat
  assert.match(app, /decisionsApi\.change<ChatMessage>\(msg\.id, index, picked\)/);
  assert.match(app, /await handleSendMessage\(prompt\)/);

  const settings = src('components/SettingsModal.tsx');
  assert.match(settings, /Bots decide reversible choices themselves/);
  assert.match(settings, /Bots work ahead while I’m away/);
  assert.match(settings, /\.\.\.\(proactive \?\? \{\}\)/);
});
