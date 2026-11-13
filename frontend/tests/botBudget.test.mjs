// Per-bot monthly budgets (0.9.5): the "Monthly budget" section in a bot's settings, the 80% meter and
// Needs-you item, the 100% pause ("Paused: monthly budget reached") with Raise the limit / Continue this
// month in the chat, the sidebar row and Needs you, and the overview in Settings › Usage. Every number
// shown is the server's (src/server/bot_budget.rs).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><div id="root"></div>', { url: 'https://kwant.example/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'localStorage', 'sessionStorage']) {
  Object.defineProperty(globalThis, k, { value: k === 'window' ? dom.window : dom.window[k], configurable: true, writable: true });
}
globalThis.requestAnimationFrame = (cb) => setTimeout(cb, 0);
globalThis.cancelAnimationFrame = (id) => clearTimeout(id);
globalThis.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  import React from 'react';
  export * from './src/botBudget';
  export { BotBudgetSection, BudgetMeter, BudgetNotice, BudgetPausedBanner, BudgetNeedsYou, BudgetOverview } from './src/components/BotBudget';
  export { React };
  export { createRoot } from 'react-dom/client';
`);

const src = (f) => readFileSync(new URL(`../src/${f}`, import.meta.url), 'utf8');
const flush = () => new Promise((r) => setTimeout(r, 0));
const settle = async () => {
  for (let i = 0; i < 8; i++) await flush();
};
const h = (...a) => m.React.createElement(...a);

/** A fetch that answers by "METHOD url" ([status, body] or a function of the call) and records the calls. */
function fake(routes) {
  const calls = [];
  const send = async (url, init = {}) => {
    const method = init.method ?? 'GET';
    calls.push({ url, method, body: init.body ? JSON.parse(init.body) : undefined });
    const r = routes[`${method} ${url}`];
    if (!r) throw new Error(`unexpected ${method} ${url}`);
    const [status, body] = typeof r === 'function' ? r(init) : r;
    return { ok: status >= 200 && status < 300, status, json: async () => body };
  };
  return { send, calls };
}

const B = (over = {}) => ({
  kwant_id: 'k1',
  kwant_name: 'Juno',
  month: '2026-11',
  timezone: 'UTC',
  used_tokens: 1_600_000,
  used_usd: 0,
  today_tokens: 0,
  budget_tokens: 2_000_000,
  budget_usd: 0,
  percent: 80,
  over: null,
  state: 'warn',
  paused_since: null,
  continued_month: null,
  resets_at: '2026-12-01T00:00:00+00:00',
  usage_reported: true,
  some_unreported: false,
  cost_reported: true,
  estimated: false,
  engine: 'claude',
  ...over,
});
const PAUSED = B({ used_tokens: 2_100_000, percent: 105, over: 'tokens', state: 'paused', paused_since: '2026-11-12T10:00:00Z' });
const OFF = B({ kwant_id: 'k2', kwant_name: 'Lin', budget_tokens: 0, percent: 0, state: 'off', used_tokens: 3000 });
const LIST = (bots) => ({ month: '2026-11', timezone: 'UTC', resets_at: '2026-12-01T00:00:00+00:00', bots });

async function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(el);
  await settle();
  return { host, root, unmount: () => (root.unmount(), host.remove()) };
}
const click = async (el) => {
  el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await settle();
};
/** Types into a React-controlled input. */
const type = async (input, value) => {
  const set = Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype, 'value').set;
  set.call(input, value);
  input.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
  await settle();
};

// ---------------------------------------------------------------- the words

test('lines: used / limit and when it starts over, in plain words', () => {
  const b = m.parseBudget(B());
  assert.equal(m.usedLine(b), '1.6M of 2M fresh tokens in November');
  assert.equal(m.resetDay(b), 'December 1');
  assert.equal(m.stateLine(b), '80% used. At 100% it finishes what it’s doing and starts nothing new until December 1.');
  assert.equal(m.stateLine(m.parseBudget(B({ state: 'under', percent: 30 }))), 'The count starts over on December 1.');
  const p = m.parseBudget(PAUSED);
  assert.equal(m.meterPercent(p), 100, 'the meter stops at full');
  assert.match(m.stateLine(p), /^Paused: monthly budget reached\. Juno starts nothing new until you raise the limit, continue this month, or December 1\.$/);
  assert.equal(m.usedLine(m.parseBudget(B({ budget_tokens: 0, budget_usd: 20, used_usd: 16.4, percent: 82 }))), '$16.40 of $20.00 in November');
  assert.equal(m.usedLine(m.parseBudget({ ...OFF })), '3k fresh tokens in November');
  assert.match(m.usedLine(m.parseBudget(B({ estimated: true }))), /^about /);
  assert.equal(m.stateLine(m.parseBudget(OFF)), 'No budget: Lin can use as much as it needs.');
  assert.equal(m.parseBudget(B({ cost_reported: false })).cost_unreported, true);
  assert.equal(m.parseBudget({ kwant_id: 'x', state: 'nope' }), null);
});

test('fields: empty is off, 2M and $2.50 parse, bad input says how', () => {
  assert.deepEqual(m.budgetBody({ tokens: '', usd: '' }), { budget_tokens: null, budget_usd: null });
  assert.deepEqual(m.budgetBody({ tokens: '2M', usd: '2.50' }), { budget_tokens: 2_000_000, budget_usd: 2.5 });
  assert.deepEqual(m.budgetBody({ tokens: '0', usd: '' }), { budget_tokens: null, budget_usd: null });
  assert.equal(m.budgetBody({ tokens: 'lots', usd: '' }), null);
  assert.match(m.budgetTokensError('lots'), /like 500k or 2M/);
  assert.match(m.budgetTokensError('20000000000'), /10 billion/);
  assert.match(m.budgetUsdError('2.505'), /like 20 or 2.50/);
  assert.match(m.budgetUsdError('200000'), /\$100,000/);
  assert.equal(m.fieldsChanged({ tokens: '2000000', usd: '' }, B()), false);
  assert.equal(m.fieldsChanged({ tokens: '1M', usd: '' }, B()), true);
  assert.deepEqual(m.fieldsFromBudget(m.parseBudget(OFF)), { tokens: '', usd: '' });
});

test('chat lines: meta as a string or an object; Needs you puts paused first', () => {
  const meta = { kind: 'bot_budget_stopped', kwant_id: 'k1', kwant_name: 'Juno', month: '2026-11', percent: 105 };
  assert.equal(m.parseBudgetNotice(JSON.stringify(meta)).kind, 'bot_budget_stopped');
  assert.equal(m.parseBudgetNotice(meta).kwant_id, 'k1');
  assert.equal(m.parseBudgetNotice({ kind: 'spend_stopped' }), null);
  assert.equal(m.isBudgetMessage({ meta: { ...meta, kind: 'bot_budget_resumed' } }), true);
  assert.equal(m.isBudgetMessage({ meta: 'not json' }), false);
  const order = m.budgetsNeedingYou([B({ kwant_id: 'a', percent: 85 }), m.parseBudget(OFF), PAUSED, B({ kwant_id: 'b', percent: 95 })]).map((b) => b.kwant_id);
  assert.deepEqual(order, ['k1', 'b', 'a']);
  assert.equal(m.takeBudgetFocus('k1'), false);
  m.requestBudgetFocus('k1');
  assert.equal(m.takeBudgetFocus('k2'), false);
  assert.equal(m.takeBudgetFocus('k1'), true);
  assert.equal(m.takeBudgetFocus('k1'), false, 'focus is taken once');
});

test('api: errors in plain words; the server’s own 400/409 text is shown', async () => {
  const api = m.botBudgetApi(
    fake({
      'POST /api/kwants/k1/budget': [400, { error: 'At most $100,000 a month.' }],
      'POST /api/kwants/k1/budget/continue': [403, { error: 'forbidden' }],
      'GET /api/usage/budgets': [500, {}],
    }).send,
  );
  await assert.rejects(api.set('k1', { budget_tokens: null, budget_usd: 1e9 }), /At most \$100,000 a month\./);
  await assert.rejects(api.continueMonth('k1'), /Only you can change a bot’s budget/);
  await assert.rejects(api.list(), /Couldn’t load the budgets\./);
  const down = m.botBudgetApi(async () => {
    throw new Error('offline');
  });
  await assert.rejects(down.get('k1'), /Couldn’t reach the server/);
});

// ---------------------------------------------------------------- bot settings › Usage

test('Monthly budget: loading, then off by default; set a limit and save', async () => {
  let saved = null;
  const { send, calls } = fake({
    'GET /api/usage/budgets': [200, LIST([OFF])],
    'GET /api/kwants/k2/budget': [200, OFF],
    'POST /api/kwants/k2/budget': (init) => {
      saved = JSON.parse(init.body);
      return [200, { ...OFF, budget_tokens: 1000, percent: 300, over: 'tokens', state: 'paused' }];
    },
  });
  m.budgetStore.reset(send);
  const { host, unmount } = await mount(h(m.BotBudgetSection, { kwantId: 'k2', kwantName: 'Lin', send }));
  assert.equal(host.querySelector('h3').textContent, 'Monthly budget');
  assert.equal(host.querySelector('[data-budget-state]').dataset.budgetState, 'off');
  assert.equal(host.querySelector('[role=meter]'), null, 'no meter with no budget');
  const tokens = host.querySelector('#bot-budget-tokens');
  assert.equal(tokens.value, '');
  assert.equal(tokens.placeholder, 'No limit');
  assert.equal(host.querySelector('label[for=bot-budget-tokens]').textContent, 'Tokens per month');
  const save = host.querySelector('[data-budget-save]');
  assert.equal(save.disabled, true, 'nothing to save yet');

  await type(tokens, 'abc');
  assert.equal(tokens.getAttribute('aria-invalid'), 'true');
  assert.match(host.querySelector('#bot-budget-tokens-hint').textContent, /like 500k or 2M/);
  assert.equal(save.disabled, true);

  await type(tokens, '1k');
  assert.equal(save.disabled, false);
  host.querySelector('form').dispatchEvent(new dom.window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  assert.deepEqual(saved, { budget_tokens: 1000, budget_usd: null });
  assert.equal(host.querySelector('[data-budget-state]').dataset.budgetState, 'paused');
  assert.match(host.textContent, /still over this limit, so it stays paused/);
  assert.ok(host.querySelector('[data-budget-continue]'), 'paused: Continue this month is right there');
  assert.equal(m.budgetStore.get().byBot.k2.state, 'paused', 'the shared list has it');
  assert.ok(calls.some((c) => c.method === 'POST'));
  unmount();
});

test('Monthly budget: a load error says so with Retry; Raise the limit focuses the first field', async () => {
  let fail = true;
  const { send } = fake({
    'GET /api/usage/budgets': [200, LIST([])],
    'GET /api/kwants/k1/budget': () => (fail ? [500, {}] : [200, PAUSED]),
  });
  m.budgetStore.reset(send);
  m.requestBudgetFocus('k1');
  const { host, unmount } = await mount(h(m.BotBudgetSection, { kwantId: 'k1', kwantName: 'Juno', send }));
  const alert = host.querySelector('[data-budget-load-error]');
  assert.match(alert.textContent, /Couldn’t load the budget\./);
  fail = false;
  await click(alert.querySelector('button'));
  assert.equal(host.querySelector('[data-budget-state]').dataset.budgetState, 'paused');
  assert.equal(document.activeElement, host.querySelector('#bot-budget-tokens'), 'Raise the limit lands on the limit');
  assert.equal(host.querySelector('#bot-budget-tokens').value, '2000000');
  unmount();
});

test('Monthly budget: continued shows Pause at the budget again, which undoes it', async () => {
  const CONT = { ...PAUSED, state: 'continued', continued_month: '2026-11' };
  const { send, calls } = fake({
    'GET /api/usage/budgets': [200, LIST([CONT])],
    'GET /api/kwants/k1/budget': [200, CONT],
    'DELETE /api/kwants/k1/budget/continue': [200, PAUSED],
  });
  m.budgetStore.reset(send);
  const { host, unmount } = await mount(h(m.BotBudgetSection, { kwantId: 'k1', kwantName: 'Juno', send }));
  assert.match(host.textContent, /Continued for November: the limit is ignored until December 1\./);
  await click(host.querySelector('[data-budget-pause-again]'));
  assert.ok(calls.some((c) => c.method === 'DELETE'));
  assert.equal(host.querySelector('[data-budget-state]').dataset.budgetState, 'paused');
  unmount();
});

// ---------------------------------------------------------------- the pause, everywhere

test('paused banner: the line, used / limit, Raise the limit and Continue this month (one post)', async () => {
  let posts = 0;
  const { send } = fake({
    'POST /api/kwants/k1/budget/continue': () => {
      posts++;
      return [200, { ...PAUSED, state: 'continued' }];
    },
  });
  m.budgetStore.reset(send);
  const raised = [];
  const { host, unmount } = await mount(h(m.BudgetPausedBanner, { budget: m.parseBudget(PAUSED), onRaise: (id) => raised.push(id), send }));
  assert.match(host.textContent, /Paused: monthly budget reached/);
  assert.match(host.textContent, /2\.1M of 2M fresh tokens in November/);
  await click(host.querySelector('[data-budget-raise]'));
  assert.deepEqual(raised, ['k1']);
  const go = host.querySelector('[data-budget-continue]');
  go.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  go.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await settle();
  assert.equal(posts, 1, 'a double click posts once');
  assert.match(go.textContent, /Continued this month/);
  assert.equal(go.getAttribute('aria-disabled'), 'true');
  assert.equal(go.disabled, false, 'aria-disabled, never disabled, so focus stays on it (F4)');
  assert.equal(m.budgetStore.get().byBot.k1.state, 'continued');
  unmount();
});

test('chat notice: the stop line’s buttons follow the bot now; an old month’s line is only a record', async () => {
  const { send } = fake({ 'GET /api/usage/budgets': [200, LIST([PAUSED])] });
  m.budgetStore.reset(send);
  await m.budgetStore.refresh();
  const meta = m.parseBudgetNotice({ kind: 'bot_budget_stopped', kwant_id: 'k1', kwant_name: 'Juno', month: '2026-11', percent: 105 });
  const old = { ...meta, month: '2026-10' };
  const { host, unmount } = await mount(
    h('div', null, h(m.BudgetNotice, { messageId: 'a', text: 'Juno reached its monthly budget.', meta, onRaise: () => {} }), h(m.BudgetNotice, { messageId: 'b', text: 'old', meta: old, onRaise: () => {} })),
  );
  const [now, then] = host.querySelectorAll('[data-budget-notice]');
  assert.ok(now.querySelector('[data-budget-continue]'));
  assert.ok(now.querySelector('[data-budget-raise]'));
  assert.equal(then.querySelector('button'), null);
  m.budgetStore.put(m.parseBudget(B()));
  await settle();
  assert.equal(now.querySelector('button'), null, 'raised above the limit: nothing left to do here');
  unmount();
});

test('Needs you: paused first, then 80%; nothing when no bot needs you', async () => {
  const { send } = fake({ 'GET /api/usage/budgets': [200, LIST([PAUSED, B({ kwant_id: 'k3', kwant_name: 'Vega', percent: 86 }), OFF])] });
  m.budgetStore.reset(send);
  await m.budgetStore.refresh();
  const { host, unmount } = await mount(h(m.BudgetNeedsYou, { onRaise: () => {}, send }));
  const items = [...host.querySelectorAll('[data-needs-budget-item]')];
  assert.deepEqual(
    items.map((i) => i.dataset.needsBudgetItem),
    ['paused', 'warn'],
  );
  assert.match(items[0].textContent, /Juno · Paused: monthly budget reached/);
  assert.match(items[1].textContent, /Vega has used 86% of its monthly budget\./);
  assert.equal(items[1].querySelector('[role=meter]').getAttribute('aria-valuenow'), '86');
  m.budgetStore.reset(send);
  await settle();
  assert.equal(host.querySelector('[data-needs-budget]'), null);
  unmount();
});

test('meter: amber from 80%, labelled for screen readers', async () => {
  const { host, unmount } = await mount(h('div', null, h(m.BudgetMeter, { budget: m.parseBudget(B({ percent: 40, state: 'under' })) }), h(m.BudgetMeter, { budget: m.parseBudget(B()), size: 'row' })));
  const [low, hot] = host.querySelectorAll('[role=meter]');
  assert.doesNotMatch(low.innerHTML, /amber/);
  assert.match(hot.innerHTML, /bg-amber-400/);
  assert.equal(hot.getAttribute('aria-label'), 'Juno has used 80% of its monthly budget');
  assert.match(hot.className, /w-7/);
  unmount();
});

// ---------------------------------------------------------------- Settings › Usage

test('overview: loading, error with Retry, empty, and the list with Set / Change', async () => {
  let answer = [500, {}];
  const { send } = fake({ 'GET /api/usage/budgets': () => answer });
  m.budgetStore.reset(send);
  const opened = [];
  const { host, unmount } = await mount(h(m.BudgetOverview, { open: true, onOpenBot: (id) => opened.push(id) }));
  assert.match(host.querySelector('[role=alert]').textContent, /Couldn’t load the budgets\./);
  answer = [200, LIST([])];
  await click(host.querySelector('[role=alert] button'));
  assert.match(host.textContent, /No bots in this space yet\./);
  answer = [200, LIST([OFF])];
  await m.budgetStore.refresh();
  await settle();
  assert.ok(host.querySelector('[data-budget-none]'), 'no bot has a budget yet');
  answer = [200, LIST([PAUSED, OFF])];
  await m.budgetStore.refresh();
  await settle();
  const rows = [...host.querySelectorAll('[data-budget-row]')];
  assert.deepEqual(rows.map((r) => r.dataset.budgetRow), ['paused', 'off']);
  assert.match(rows[0].textContent, /Paused/);
  assert.equal(rows[0].querySelector('button').getAttribute('aria-label'), 'Change Juno’s budget');
  assert.equal(rows[1].querySelector('button').getAttribute('aria-label'), 'Set Lin’s budget');
  await click(rows[1].querySelector('button'));
  assert.deepEqual(opened, ['k2']);
  unmount();
});

// ---------------------------------------------------------------- wiring

test('wiring: bot settings, chat, sidebar, Inbox, Settings › Usage and live refresh', () => {
  const modal = src('components/NewAgentModal.tsx');
  assert.match(modal, /<BotBudgetSection kwantId=\{kwantId\}/);
  const chat = src('components/ChatView.tsx');
  assert.match(chat, /<BudgetNotice/);
  assert.match(chat, /\{budgetBanner\}\s*\{workingStrip\}/);
  const side = src('components/Sidebar.tsx');
  assert.match(side, /budgetPaused && !busyKwants\[k\.id\]\s*\? PAUSED_ROW_LINE/);
  // F7: the row meter shows only at 80%, centred on the name line
  assert.match(side, /\{budget\?\.state === 'warn' && <BudgetMeter budget=\{budget!\} size="row" className="ml-0\.5 self-center" \/>\}/);
  assert.equal(m.PAUSED_ROW_LINE, 'Paused: budget reached');
  // F1: in a paused 1:1 the bar is the one place to act; the stop note is a record
  assert.match(chat, /canAct=\{!\(budgetBotId === budgetNote\.kwant_id && chatBudget\?\.state === 'paused'\)\}/);
  // F2: the composer's "New" sticker hides while the bar shows, at both call sites
  assert.equal(chat.match(/\$\{layerPhase \|\| budgetBanner \? 'hidden' : ''\}/g)?.length, 2);
  assert.doesNotMatch(chat, /\$\{layerPhase \? 'hidden' : ''\}/);
  const app = src('App.tsx');
  assert.match(app, /<BudgetNeedsYou onRaise=\{raiseBudget\} \/>/);
  assert.match(app, /isBudgetMessage\(m\)\) void budgetStore\.refresh\(\)/);
  assert.match(app, /requestBudgetFocus\(kwantId\);\s*openAgentSettings\(k, 'usage'\)/);
  assert.match(app, /onOpenBotBudget=\{raiseBudget\}/);
  assert.match(src('components/SettingsModal.tsx'), /<BudgetOverview open=\{isOpen\} onOpenBot=\{onOpenBotBudget\} \/>/);
});

test('Inbox: the all-clear line hides while a Needs you block sits above it', () => {
  assert.match(src('components/InboxNew.tsx'), /\[\[data-needs-you\]~&\]:hidden[^>]*data-all-clear/);
  for (const f of ['components/BotBudget.tsx', 'components/SpendStop.tsx', 'components/dev/PrepushThread.tsx']) assert.match(src(f), /data-needs-you(>|=\{)/, f);
});

// ---------------------------------------------------------------- Mira's fixes (#536 F3, F4, F6)

test('F4 bar: after Continue this month, focus goes to the composer', async () => {
  const { send } = fake({ 'POST /api/kwants/k1/budget/continue': [200, { ...PAUSED, state: 'continued' }] });
  m.budgetStore.reset(send);
  const form = document.createElement('form');
  form.setAttribute('data-composer-form', '');
  const box = document.createElement('textarea');
  form.appendChild(box);
  document.body.appendChild(form);
  const { host, unmount } = await mount(h(m.BudgetPausedBanner, { budget: m.parseBudget(PAUSED), send }));
  const go = host.querySelector('[data-budget-continue]');
  go.focus();
  await click(go);
  assert.equal(document.activeElement, box, 'the person can type straight away');
  unmount();
  form.remove();
});

test('F4 Inbox: a continued row stays, says so, and keeps focus on an aria-disabled button', async () => {
  const { send } = fake({
    'GET /api/usage/budgets': [200, LIST([PAUSED])],
    'POST /api/kwants/k1/budget/continue': [200, { ...PAUSED, state: 'continued', continued_month: '2026-11' }],
  });
  m.budgetStore.reset(send);
  await m.budgetStore.refresh();
  const { host, unmount } = await mount(h(m.BudgetNeedsYou, { onRaise: () => {}, send }));
  const go = host.querySelector('[data-budget-continue]');
  go.focus();
  await click(go);
  const row = host.querySelector('[data-needs-budget-item]');
  assert.equal(row.dataset.needsBudgetItem, 'continued');
  assert.match(row.textContent, /Juno can start new work again\./);
  const done = row.querySelector('[data-budget-continue]');
  assert.equal(done, go, 'the same button, still mounted');
  assert.match(done.textContent, /Continued this month/);
  assert.equal(done.getAttribute('aria-disabled'), 'true');
  assert.equal(done.disabled, false);
  assert.equal(document.activeElement, done, 'focus stays on it');
  assert.equal(row.querySelector('[data-budget-raise]'), null);
  assert.equal(host.querySelector('[data-needs-budget]').hasAttribute('data-needs-you'), false, 'all done: the all-clear can come back');
  assert.match(host.querySelector('[role=status]').textContent, /Continued\. Juno can start new work again\./);
  unmount();
});

test('F4 settings: after Continue, focus lands on Pause at the budget again', async () => {
  const CONT = { ...PAUSED, state: 'continued', continued_month: '2026-11' };
  const { send } = fake({
    'GET /api/usage/budgets': [200, LIST([PAUSED])],
    'GET /api/kwants/k1/budget': [200, PAUSED],
    'POST /api/kwants/k1/budget/continue': [200, CONT],
  });
  m.budgetStore.reset(send);
  const { host, unmount } = await mount(h(m.BotBudgetSection, { kwantId: 'k1', kwantName: 'Juno', send }));
  await click(host.querySelector('[data-budget-continue]'));
  await settle();
  assert.equal(document.activeElement, host.querySelector('[data-budget-pause-again]'));
  unmount();
});

test('F6 settings: a raise that un-pauses says the bot can start again; state lines are zinc-400', async () => {
  const { send } = fake({
    'GET /api/usage/budgets': [200, LIST([PAUSED])],
    'GET /api/kwants/k1/budget': [200, PAUSED],
    'POST /api/kwants/k1/budget': [200, B({ budget_tokens: 5_000_000, used_tokens: 2_100_000, percent: 42, state: 'under' })],
  });
  m.budgetStore.reset(send);
  const { host, unmount } = await mount(h(m.BotBudgetSection, { kwantId: 'k1', kwantName: 'Juno', send }));
  await type(host.querySelector('#bot-budget-tokens'), '5M');
  host.querySelector('form').dispatchEvent(new dom.window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  assert.match(host.textContent, /Saved\. Juno can start new work again\./);
  const line = [...host.querySelectorAll('[data-budget-state] p')].find((p) => /starts over/.test(p.textContent));
  assert.match(line.className, /text-zinc-400/);
  unmount();
});

test('F3 and F6 overview: 44 px phone targets, "N% used" chip, readable no-budget note', async () => {
  const { send } = fake({ 'GET /api/usage/budgets': [200, LIST([B({ percent: 80 }), OFF])] });
  m.budgetStore.reset(send);
  const { host, unmount } = await mount(h(m.BudgetOverview, { open: true, onOpenBot: () => {} }));
  const [warn] = host.querySelectorAll('[data-budget-row]');
  assert.match(warn.textContent, /80% used/);
  assert.doesNotMatch(warn.textContent, /Over 80%/);
  assert.match(warn.querySelector('button').className, /min-h-\[44px\] sm:min-h-\[32px\] px-3 sm:px-2/);
  unmount();
  const src2 = src('components/BotBudget.tsx');
  assert.equal(src2.match(/min-h-\[44px\] sm:min-h-\[36px\] px-3/g)?.length, 3, 'all three button styles');
  assert.equal(src2.match(/min-h-\[44px\] sm:min-h-0 px-2 -mx-2/g)?.length, 2, 'both Retry buttons');
  assert.match(src2, /text-\[11px\] text-zinc-400" data-budget-none/);
});
