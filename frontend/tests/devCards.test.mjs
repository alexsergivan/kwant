// Dev mode renderers (0.7, Iris): the Verified sticker rules, the production-floor backup state machine
// (all 7 cases), the 500 ms Allow guard and the findings Dismiss flow, plus parse and render checks.
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });

const m = await loadBundled(`
  export * from './src/dev/types';
  export * from './src/dev/logic';
  export * as fx from './src/dev/fixtures';
  export { LandedCard, LandedList } from './src/components/dev/LandedCard';
  export { Rollup, rollupTitle, rollupTiles } from './src/components/dev/Rollup';
  export { ReviewCard } from './src/components/dev/ReviewCard';
  export { ReleaseNotesSheet } from './src/components/dev/ReleaseNotesSheet';
  export { ProdFloorCard } from './src/components/dev/ProdFloorCard';
  export { ModernApprovalCard } from './src/components/ModernApprovalCard';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
`);
const { fx } = m;
const h = m.createElement;
const html = (C, props) => m.renderToStaticMarkup(h(C, props));
const NOW = fx.FIXTURE_NOW;

const landed = (raw, over = {}) => ({ ...m.parseLanded(raw), ...over });
const review = (raw, over = {}) => ({ ...m.parseReview(raw), ...over });

// ---------------------------------------------------------------------------------------------------
// DOM helpers

const mounted = new Set();
afterEach(() => {
  for (const u of mounted) u();
  document.body.replaceChildren();
});
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mount = async (C, props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(h(C, props));
  await settle();
  const u = () => {
    if (!mounted.delete(u)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(u);
  return host;
};
const button = (host, text) => [...host.querySelectorAll('button')].find((b) => b.textContent.replace(/\s+/g, ' ').trim().startsWith(text));
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};

// ---------------------------------------------------------------------------------------------------
// Parsing and null handling (§3.6 items 10, 12, 13)

test('parsers keep the §3.6 nulls and reject other shapes', () => {
  const c = m.parseLanded(fx.LANDED_NO_TEXT);
  assert.equal(c.headline, 'Bump vite to 6.4.4', 'headline falls back to the PR title');
  assert.equal(c.area, null);
  assert.equal(c.for_you, null);
  assert.equal(c.under_hood.summary, null);
  assert.equal(c.status, 'failed');
  assert.match(c.error, /daily cap/);
  assert.equal(c.cut, 'diff cut, 2 files not read');
  const r = m.parseReview(fx.REVIEW_NO_REVIEWER);
  assert.equal(r.reviewer, null);
  assert.match(r.note, /No reviewer/);
  assert.equal(m.parseLanded({ kind: 'review', pr: 1 }), null);
  assert.equal(m.parseLanded({ kind: 'landed' }), null, 'no PR number');
  assert.equal(m.parseReview(null), null);
  const odd = m.parseLanded({ ...fx.LANDED_289, links: { pr: 'javascript:alert(1)', commit: 'http://x', ci: null } });
  assert.equal(odd.links.pr, null, 'only https links');
  assert.equal(odd.links.commit, null);
  const f = m.parseReview(fx.REVIEW_294).findings.find((x) => x.id === 'f2');
  assert.equal(f.sent_to, 'Pixel');
  assert.equal(f.task_id, null);
});

test('the floor parser forces needs_backup for destructive and migration deploys and drops bad blast rows', () => {
  const d = m.parseProdFloor({ prod_floor: { kind: 'destructive', needs_backup: false, blast: [['Where', 'x'], ['bad'], [1, 2]], backup: {} } });
  assert.equal(d.needs_backup, true);
  assert.deepEqual(d.blast, [['Where', 'x']]);
  assert.equal(d.backup.state, 'none');
  assert.equal(m.parseProdFloor({ prod_floor: { kind: 'deploy_migration', backup: {} } }).needs_backup, true);
  assert.equal(m.parseProdFloor({ prod_floor: { kind: 'launch_rockets' } }), null);
  assert.equal(m.parseProdFloor({ source: 'local_shell' }), null);
});

// ---------------------------------------------------------------------------------------------------
// Verified sticker (§3.6 item 14)

test('Verified: CI green + reviewed by someone else + nothing open', () => {
  const ok = landed(fx.LANDED_289);
  assert.equal(m.showVerified(ok), true);
  assert.equal(m.showVerified({ ...ok, verified: false }), false, 'the server has to agree');
  assert.equal(m.showVerified({ ...ok, tests: { state: 'failed', summary: null } }), false, 'red CI');
  assert.equal(m.showVerified({ ...ok, tests: { state: 'none', summary: null } }), false, 'no CI');
  assert.equal(m.showVerified({ ...ok, reviewed_by: null }), false, 'no review');
  assert.equal(m.showVerified({ ...ok, reviewed_by: 'ada ' }), false, 'reviewed by its own author (any case)');
  assert.equal(m.showVerified(landed(fx.LANDED_288, { verified: true })), false, 'an open follow-up');
  assert.equal(m.showVerified({ ...ok, status: 'stale' }), false);
  assert.equal(m.showVerified({ ...ok, status: 'pending' }), false);
});

test('Verified: the PR review card can only take it away', () => {
  const ok = landed(fx.LANDED_289);
  const rv = review(fx.REVIEW_289);
  assert.equal(m.showVerified(ok, [rv]), true);
  const open = { ...rv, findings: [{ ...review(fx.REVIEW_294).findings[0] }] };
  assert.equal(m.showVerified(ok, [open]), false, 'an open finding');
  assert.equal(m.showVerified(ok, [{ ...open, findings: open.findings.map((f) => ({ ...f, state: 'dismissed' })) }]), true, 'dismissed is closed');
  assert.equal(m.showVerified(ok, [{ ...open, findings: open.findings.map((f) => ({ ...f, state: 'sent' })) }]), true, 'sent as a task is closed');
  assert.equal(m.showVerified(ok, [{ ...rv, reviewer: null }]), false, 'no reviewer');
  assert.equal(m.showVerified(ok, [{ ...rv, reviewer: { name: 'Ada', engine: null, model: null } }]), false, 'reviewer is the author');
  assert.equal(m.showVerified(ok, [{ ...rv, pr: 999, reviewer: null }]), true, 'another PR’s review does not count');
});

test('the landed card shows the holo sticker only when Verified', () => {
  const yes = html(m.LandedCard, { card: landed(fx.LANDED_289) });
  assert.match(yes, /data-verified="true"/);
  assert.match(yes, /kw-sticker[^"]*holo|holo/);
  assert.match(yes, />Verified</);
  const no = html(m.LandedCard, { card: landed(fx.LANDED_288) });
  assert.doesNotMatch(no, /data-verified/);
  assert.doesNotMatch(no, />Verified</);
  assert.match(no, /1 follow-up open:/);
  assert.match(no, /#288 N3/);
});

test('the landed card: plain words, the details fold, a failed card and day separators', () => {
  const c = html(m.LandedCard, { card: landed(fx.LANDED_288) });
  assert.match(c, /What changes for you/);
  assert.match(c, /<details/);
  assert.match(c, /Under the hood · 4 files, \+249 −15/);
  assert.match(c, /Merged 18:48 by/);
  assert.match(c, /Pixel<\/span><\/span><span>\(bot · Claude Sonnet\)/);
  const f = html(m.LandedCard, { card: landed(fx.LANDED_NO_TEXT) });
  assert.match(f, /Bump vite to 6\.4\.4/);
  assert.match(f, /No plain-words text for this change/);
  assert.doesNotMatch(f, /What changes for you/);
  assert.match(f, /diff cut, 2 files not read/);
  assert.doesNotMatch(f, /class="kw-chip">/, 'no area chip when area is null');
  const list = html(m.LandedList, { cards: [landed(fx.LANDED_303), landed(fx.LANDED_288), landed(fx.LANDED_NO_TEXT)], reviews: [], now: NOW });
  const days = [...list.matchAll(/<section aria-label="([^"]+)"/g)].map((x) => x[1]);
  assert.deepEqual(days, ['Today', 'Yesterday', 'Tue 3 Nov']);
});

test('roll-up: title, needs-you tiles, +N more, the empty state, no routine link', () => {
  assert.equal(m.rollupTitle(3, 0, 'main'), '3 changes landed on main. Nothing needs you.');
  assert.equal(m.rollupTitle(1, 1, 'main'), '1 change landed on main. 1 needs you.');
  assert.deepEqual(m.rollupTiles([1, 2, 3, 4]), { shown: [1, 2, 3, 4], more: 0 });
  assert.deepEqual(m.rollupTiles([1, 2, 3, 4, 5, 6]), { shown: [1, 2], more: 4 });
  const base = { last_tag: 'mac-v0.6.10', last_merge: null, branch: 'main', onDraftNotes() {}, onMarkSeen() {} };
  const r = html(m.Rollup, { ...base, rollup: fx.LANDED_ROUTE.rollup, needsYouAreas: ['Live activity'] });
  assert.match(r, /data-dev-rollup="needs-you"/);
  assert.match(r, /data-area="Live activity" data-needs-you="true"/);
  assert.match(r, /main is green/);
  assert.doesNotMatch(r, /every morning/i);
  assert.match(html(m.Rollup, { ...base, rollup: fx.ROLLUP_MANY }), /\+4 more areas/);
  assert.match(html(m.Rollup, { ...base, rollup: fx.ROLLUP_MANY }), /main is red/);
  const e = html(m.Rollup, { ...base, rollup: fx.ROLLUP_EMPTY, last_merge: { pr: 290, merged_at: '2026-11-04T19:26:00Z' } });
  assert.match(e, /Nothing landed since you last looked\. Last merge: #290, 19:26\./);
  assert.deepEqual(m.needsYouAreas(fx.LANDED_ROUTE.cards.map((c) => m.parseLanded(c))), ['Live activity']);

  const { branch: _b, ...noBranch } = base;
  assert.match(html(m.Rollup, { ...noBranch, rollup: fx.ROLLUP_QUIET }), /landed on main\./, 'branch defaults to main');
  const waiting = html(m.Rollup, { ...base, rollup: fx.ROLLUP_QUIET, marking: true });
  assert.match(waiting, /<button[^>]*disabled=""[^>]*aria-busy="true"[^>]*>Marking…<\/button>/, 'Mark all seen waits while marking');
  assert.match(html(m.Rollup, { ...base, rollup: fx.ROLLUP_QUIET }), />Mark all seen<\/button>/);
});

// ---------------------------------------------------------------------------------------------------
// Production floor: the backup state machine (design §6, all 7 cases)

const floorOf = (approval) => m.parseProdFloor(approval.details);
const viewOf = (approval, ui) => {
  const f = floorOf(approval);
  return m.floorView(f, ui ?? m.initialFloorUi(f), NOW, NOW);
};

test('backup case 1: a fresh backup lets Allow through', () => {
  const v = viewOf(fx.FLOOR_DEPLOY_FRESH);
  assert.equal(v.case, 'fresh');
  assert.equal(v.main, 'allow');
  assert.equal(v.secondaryBackup, false);
  assert.equal(v.lead, 'Backup taken 14 s ago');
  assert.equal(v.text, ' · 412 MB');
  assert.equal(v.file, 'backup-2026-11-04-pre-4049325.tgz');
});

test('backup case 2: a plain deploy with an old backup offers Allow and Back up first', () => {
  const v = viewOf(fx.FLOOR_DEPLOY_OLD);
  assert.equal(v.case, 'old_plain');
  assert.equal(v.tone, 'warn');
  assert.equal(v.main, 'allow');
  assert.equal(v.secondaryBackup, true);
  assert.equal(`${v.lead}${v.text}`, 'Last backup 3 h ago. No data changes, so you can still allow it.');
});

test('backup case 3: just taken via Back up first, then it ages and goes old after 10 minutes', () => {
  const f = floorOf(fx.FLOOR_MIGRATION);
  let ui = m.floorReducer(m.initialFloorUi(f), { type: 'backup_start' });
  ui = m.floorReducer(ui, { type: 'backup_done', result: { id: 'b', ok: true, path: '/x/b.tgz', bytes: 412_000_000, taken_at: null, error: null }, now: NOW });
  let v = m.floorView(f, ui, NOW + 3000, NOW);
  assert.equal(v.case, 'just_taken');
  assert.equal(v.lead, 'Backup taken 3 s ago');
  assert.equal(v.main, 'allow');
  v = m.floorView(f, ui, NOW + 11 * 60_000, NOW);
  assert.equal(v.case, 'blocked', 'stale again after 10 minutes');
  assert.equal(v.main, 'backup');
});

test('backup case 4: a site with no data has nothing to back up', () => {
  const v = viewOf(fx.FLOOR_PAGES);
  assert.equal(v.case, 'no_data');
  assert.equal(v.tone, 'neutral');
  assert.equal(v.main, 'allow');
  assert.equal(v.text, 'Nothing to back up: this site has no database or uploads.');
});

test('backup case 5: destructive or migration with no fresh backup replaces Allow with Back up first', () => {
  for (const [ap, text] of [[fx.FLOOR_DESTRUCTIVE, ' The last one is 3 h old.'], [fx.FLOOR_MIGRATION, ' The last one is 41 min old.']]) {
    const v = viewOf(ap);
    assert.equal(v.case, 'blocked');
    assert.equal(v.tone, 'bad');
    assert.equal(v.main, 'backup');
    assert.equal(v.lead, 'No fresh backup.');
    assert.equal(v.text, text);
    assert.equal(v.note, 'Nothing runs without a fresh backup.');
  }
  const never = m.floorView({ ...floorOf(fx.FLOOR_DESTRUCTIVE), backup: { state: 'none', age_s: null, bytes: null, path: null, error: null } }, { phase: 'idle', taken: null, error: null, armedAt: null }, NOW);
  assert.equal(never.lead, 'No backup yet.');
  const nc = viewOf(fx.FLOOR_NO_COMMAND);
  assert.equal(nc.case, 'no_command');
  assert.equal(nc.main, 'none', 'nothing to run, so no main button');
});

test('backup case 6: taking disables every button and shows a percent only when the server sends one', () => {
  const f = floorOf(fx.FLOOR_DESTRUCTIVE);
  const ui = m.floorReducer(m.initialFloorUi(f), { type: 'backup_start' });
  assert.equal(m.floorReducer(ui, { type: 'backup_start' }), ui, 'a second press changes nothing');
  let v = m.floorView(f, ui, NOW);
  assert.equal(v.case, 'taking');
  assert.equal(v.busy, true);
  assert.equal(v.text, 'Taking a backup…');
  v = m.floorView({ ...f, backup: { ...f.backup, state: 'taking', percent: 38 } }, ui, NOW);
  assert.equal(v.text, 'Taking a backup… 38 %');
});

test('backup case 7: a failed backup never shows Allow, only Try the backup again', () => {
  const v = viewOf(fx.FLOOR_FAILED);
  assert.equal(v.case, 'failed');
  assert.equal(`${v.lead}${v.text}`, 'Backup failed: disk full.');
  assert.equal(v.main, 'retry');
  const f = floorOf(fx.FLOOR_DESTRUCTIVE);
  let ui = m.floorReducer(m.initialFloorUi(f), { type: 'backup_start' });
  ui = m.floorReducer(ui, { type: 'backup_done', result: { id: null, ok: false, path: null, bytes: null, taken_at: null, error: 'exit 1: no space' }, now: NOW });
  assert.equal(ui.phase, 'failed');
  assert.equal(ui.armedAt, null);
  assert.equal(m.floorView(f, ui, NOW).main, 'retry');
  ui = m.floorReducer(m.floorReducer(m.initialFloorUi(f), { type: 'backup_start' }), { type: 'backup_error', error: 'A backup is running already' });
  assert.equal(m.floorView(f, ui, NOW).text, ' A backup is running already.');
});

test('the floor card never offers "always allow", in any case', () => {
  for (const ap of [fx.FLOOR_DEPLOY_FRESH, fx.FLOOR_DESTRUCTIVE, fx.FLOOR_MIGRATION, fx.FLOOR_DEPLOY_OLD, fx.FLOOR_PAGES, fx.FLOOR_NO_COMMAND, fx.FLOOR_FAILED]) {
    const out = html(m.ModernApprovalCard, { approval: ap, onResolve() {} });
    assert.match(out, /data-prod-floor=/);
    assert.doesNotMatch(out, /Always Allow|Always this session/);
    assert.match(out, /Production floor: /);
  }
  const fresh = html(m.ModernApprovalCard, { approval: fx.FLOOR_DEPLOY_FRESH, onResolve() {} });
  assert.match(fresh, /Production floor: Pixel wants to deploy/);
  assert.match(fresh, /Allow this deploy/);
  assert.match(fresh, /<dt[^>]*>Downtime<\/dt>/);
  assert.match(fresh, /aria-live="polite"/);
  const mig = html(m.ModernApprovalCard, { approval: fx.FLOOR_MIGRATION, onResolve() {} });
  assert.match(mig, /wants to deploy a database change/);
  assert.doesNotMatch(mig, /Allow this deploy/, 'blocked: Allow is replaced');
  assert.match(mig, /Back up first/);
});

// ---------------------------------------------------------------------------------------------------
// The 500 ms Allow guard

test('allowReady: the Allow that replaced Back up first ignores its first 500 ms', () => {
  const f = floorOf(fx.FLOOR_DESTRUCTIVE);
  assert.equal(m.allowReady(m.initialFloorUi(f), NOW), true, 'an Allow that was there from the start is not guarded');
  let ui = m.floorReducer(m.initialFloorUi(f), { type: 'backup_start' });
  ui = m.floorReducer(ui, { type: 'backup_done', result: { id: 'b', ok: true, path: null, bytes: 1, taken_at: null, error: null }, now: NOW });
  assert.equal(ui.armedAt, NOW);
  assert.equal(m.allowReady(ui, NOW), false);
  assert.equal(m.allowReady(ui, NOW + 499), false);
  assert.equal(m.allowReady(ui, NOW + 500), true);
});

test('Back up first turns into Allow in place; a double press cannot run the command', async () => {
  const realNow = Date.now;
  let clock = NOW;
  Date.now = () => clock;
  try {
    const calls = [];
    let release;
    const backupDone = new Promise((r) => (release = r));
    const host = await mount(m.ModernApprovalCard, {
      approval: fx.FLOOR_DESTRUCTIVE,
      onResolve: (id, d) => calls.push([id, d]),
      onBackup: async () => {
        await backupDone;
        return { id: 'b1', ok: true, path: '/home/demo/backups/b1.tgz', bytes: 412_000_000, taken_at: null, error: null };
      },
    });
    const main = host.querySelector('[data-floor-main]');
    assert.equal(main.dataset.floorMain, 'backup');
    assert.equal(main.textContent.trim(), 'Back up first');
    await click(main);
    assert.equal(host.querySelector('[data-backup-row]').dataset.backupRow, 'taking');
    assert.ok([...host.querySelectorAll('button')].every((b) => b.disabled), 'every button waits');
    release();
    await settle();
    const after = host.querySelector('[data-floor-main]');
    assert.equal(after, main, 'the same element, changed in place');
    assert.equal(after.dataset.floorMain, 'allow');
    assert.equal(after.textContent.trim(), 'Allow');
    assert.equal(host.querySelector('[data-backup-row]').dataset.backupRow, 'just_taken');
    await click(after);
    clock += 300;
    await click(after);
    assert.deepEqual(calls, [], 'presses in the first 500 ms are ignored');
    clock += 250;
    await click(after);
    assert.deepEqual(calls, [['ap_2', 'allow_once']]);
  } finally {
    Date.now = realNow;
  }
});

test('a failed backup from the card shows the error and Try the backup again', async () => {
  const host = await mount(m.ModernApprovalCard, {
    approval: fx.FLOOR_MIGRATION,
    onResolve() {},
    onBackup: async () => {
      throw new Error('Set a backup command in Dev mode settings');
    },
  });
  await click(host.querySelector('[data-floor-main]'));
  assert.equal(host.querySelector('[data-backup-row]').dataset.backupRow, 'failed');
  assert.match(host.querySelector('[data-backup-row]').textContent, /Backup failed: Set a backup command in Dev mode settings\./);
  assert.equal(host.querySelector('[data-floor-main]').textContent.trim(), 'Try the backup again');
  assert.ok(!button(host, 'Allow'), 'Allow never appears after a failure');
});

// ---------------------------------------------------------------------------------------------------
// Findings: Fix it, Undo and the Dismiss flow

test('dismissReducer: a reason is required, a double submit is ignored, cancel resets', () => {
  let d = m.dismissReducer(m.closedDraft, { type: 'open' });
  assert.equal(d.open, true);
  assert.equal(m.dismissReducer(d, { type: 'submit' }), d, 'no reason, no submit');
  d = m.dismissReducer(d, { type: 'reason', reason: 'fine_here' });
  d = m.dismissReducer(d, { type: 'note', note: 'x'.repeat(600) });
  assert.equal(d.note.length, 500);
  d = m.dismissReducer(d, { type: 'submit' });
  assert.equal(d.sending, true);
  assert.equal(m.dismissReducer(d, { type: 'submit' }), d, 'second submit ignored');
  assert.equal(m.dismissReducer(d, { type: 'reason', reason: 'wrong' }), d, 'locked while sending');
  d = m.dismissReducer(d, { type: 'failed', error: 'offline' });
  assert.equal(d.sending, false);
  assert.equal(d.error, 'offline');
  assert.equal(d.reason, 'fine_here', 'the choice survives a failure');
  assert.deepEqual(m.dismissReducer(d, { type: 'cancel' }), m.closedDraft);
});

test('dismissedLine and the comment count', () => {
  const f = { ...review(fx.REVIEW_294).findings[0], state: 'dismissed', dismissed: { reason: 'fine_here', note: '', by: 'person', at: null } };
  assert.equal(m.dismissedLine(f), 'Dismissed · Fine in this PR · you');
  assert.equal(m.commentCount(review(fx.REVIEW_294).findings), 2, 'f1 and f3 open; f2 was sent');
  assert.equal(m.findingsSummary(review(fx.REVIEW_294).findings), '1 high, 1 medium, 1 low');
});

test('undoLeft: 10 s after a Fix it, and none once a task exists', () => {
  const f2 = review(fx.REVIEW_294).findings[1];
  const at = Date.parse(f2.sent_at);
  assert.equal(m.undoLeft(f2, at + 1000), 9000);
  assert.equal(m.undoLeft(f2, at + 10_000), 0);
  assert.equal(m.undoLeft({ ...f2, task_id: 't1' }, at + 1000), 0);
  assert.equal(m.undoLeft({ ...f2, state: 'open' }, at), 0);
});

test('the Dismiss flow in the review card: reason, submit, collapsed line', async () => {
  const card = review(fx.REVIEW_294);
  const sent = [];
  const onFinding = async (cardId, fid, action) => {
    sent.push([cardId, fid, action]);
    return { ...card, findings: card.findings.map((f) => (f.id === fid ? { ...f, state: 'dismissed', dismissed: { reason: action.reason, note: action.note ?? '', by: 'person', at: null } } : f)) };
  };
  const host = await mount(m.ReviewCard, { card, onFinding, sample: true, now: NOW });
  const row = () => host.querySelector('[data-finding="f3"]');
  await click(button(row(), 'Dismiss'));
  const form = row().querySelector('form');
  assert.ok(form, 'the inline form opens');
  assert.match(form.textContent, /Why dismiss\? Rigel learns from this\./);
  const submit = button(form, 'Dismiss finding');
  assert.equal(submit.disabled, true, 'needs a reason');
  await click(button(form, 'Fine in this PR'));
  assert.equal(button(form, 'Fine in this PR').getAttribute('aria-checked'), 'true');
  assert.equal(button(form, 'Dismiss finding').disabled, false);
  form.dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  assert.deepEqual(sent, [['dvr_294', 'f3', { action: 'dismiss', reason: 'fine_here' }]]);
  assert.equal(row().dataset.state, 'dismissed');
  assert.match(row().textContent, /Dismissed · Fine in this PR · you/);
  assert.match(host.textContent, /Post to GitHub · 1 comment/);
});

test('Dismiss can be cancelled, and a failed dismiss keeps the form with the error', async () => {
  const card = review(fx.REVIEW_294);
  const host = await mount(m.ReviewCard, { card, now: NOW, onFinding: async () => { throw new Error('kwant is offline'); } });
  const row = () => host.querySelector('[data-finding="f1"]');
  await click(button(row(), 'Dismiss'));
  await click(button(row(), 'Cancel'));
  assert.equal(row().querySelector('form'), null);
  await click(button(row(), 'Dismiss'));
  await click(button(row(), 'Wrong'));
  row().querySelector('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  assert.match(row().querySelector('[role="alert"]').textContent, /kwant is offline/);
  assert.equal(row().dataset.state, 'open');
});

test('Fix it sends the finding and offers Undo for 10 s', async () => {
  const card = review(fx.REVIEW_294);
  const actions = [];
  const host = await mount(m.ReviewCard, {
    card,
    now: Date.parse('2026-11-05T01:00:02Z'),
    onFinding: async (_c, fid, a) => {
      actions.push([fid, a.action]);
      return { ...card, findings: card.findings.map((f) => (f.id === fid ? { ...f, state: a.action === 'fix' ? 'sent' : 'open', sent_to: 'Pixel', sent_at: '2026-11-05T01:00:00Z' } : f)) };
    },
  });
  await click(button(host.querySelector('[data-finding="f1"]'), 'Fix it'));
  const row = host.querySelector('[data-finding="f1"]');
  assert.match(row.textContent, /Sent to Pixel as a task · 01:00/);
  assert.match(button(row, 'Undo').textContent, /8 s/);
  await click(button(row, 'Undo'));
  assert.deepEqual(actions, [['f1', 'fix'], ['f1', 'undo_fix']]);
});

test('the review card: sample chip, tamper, security tiles in 0.7.x, Post to GitHub disabled', () => {
  const out = html(m.ReviewCard, { card: review(fx.REVIEW_294), onFinding: async () => review(fx.REVIEW_294), sample: true, now: NOW });
  assert.match(out, /Sample PR/);
  assert.match(out, /A test was made easier to pass/);
  assert.match(out, /Different engine from the author/);
  assert.match(out, /data-sec="OSV" data-state="later"/);
  assert.match(out, /data-sec="Semgrep" data-state="later"/);
  assert.equal((out.match(/in 0\.7\.x/g) ?? []).length >= 2, true);
  assert.match(out, /<span title="Posting to GitHub comes in 0\.7\.x"[^>]*><button type="button" disabled=""/);
  assert.match(out, /Post to GitHub · 2 comments/);
  assert.match(out, /Did less than asked/);
  assert.match(out, /Did more than asked/);
  const none = html(m.ReviewCard, { card: review(fx.REVIEW_NO_REVIEWER), onFinding: async () => review(fx.REVIEW_NO_REVIEWER) });
  assert.match(none, /No reviewer: add a second bot to this space/);
  assert.doesNotMatch(none, /Different engine/);
  assert.match(none, /Draft/);
});

test('release notes: a pending state, then Copy and Discard; Save to Notion only when connected', () => {
  const pend = html(m.ReleaseNotesSheet, { card: m.parseReleaseNotes(fx.RELEASE_PENDING), onDiscard() {}, onClose() {} });
  assert.match(pend, /Writing the notes from what landed/);
  assert.doesNotMatch(pend, /Save to Notion/);
  const ready = html(m.ReleaseNotesSheet, { card: m.parseReleaseNotes(fx.RELEASE_READY), onDiscard() {}, onClose() {} });
  assert.match(ready, /What’s new/);
  assert.match(ready, />Copy</);
  assert.match(ready, /Discard/);
  assert.doesNotMatch(ready, /Save to Notion/);
  assert.match(html(m.ReleaseNotesSheet, { card: m.parseReleaseNotes(fx.RELEASE_READY), onSaveToNotion: async () => {}, onDiscard() {}, onClose() {} }), /Save to Notion/);
});
