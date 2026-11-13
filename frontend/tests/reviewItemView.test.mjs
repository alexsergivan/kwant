// 0.9.0 V1 ReviewItemView (Mira, release-090/designs/v1-review-inbox.md; Ada's D2, D3): the copy helpers,
// and the component against Pixel's fixtures with an injected fake api. Demo data only; no network.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/inbox', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'HTMLTextAreaElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * from './src/reviewItem';
  export { ReviewItemView } from './src/components/ReviewItemView';
  export { FIXTURE_REVIEW_ITEMS, InboxError } from './src/api/inbox';
  export { MissionError } from './src/missions';
`);
const { React, createRoot, ReviewItemView, FIXTURE_REVIEW_ITEMS: FX, InboxError, MissionError } = m;
const h = React.createElement;
const settle = async (n = 12) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};

function fakeApi(over = {}) {
  const calls = [];
  const rec = (name, ret) => async (...a) => {
    calls.push([name, ...a]);
    return typeof ret === 'function' ? ret(...a) : ret;
  };
  const cps = [
    { id: 'cp-before', kind: 'before_run', attempt: 1, created_at: '2026-11-10T14:02:00Z', created_by: 'kwant', files: 3, bytes: 10, skipped: 0, git_head: null, error: null, restorable: true, effects_after: { workspace: 1, external: [] } },
  ];
  const preview = {
    checkpoint_id: 'cp-before', preview_sha256: 'sha-1',
    changes: [{ path: 'src/auth/limit.rs', action: 'overwrite' }, { path: 'src/auth/new.rs', action: 'delete' }],
    conflicts: [], skipped: [], external_not_undone: [], not_undone_note: 'kwant can’t undo these. Check them yourself.',
    effects_known: true, task_state_after: 'ready', refused: null, via: 'restore', step_down: null,
  };
  const api = {
    verify: rec('verify', {}),
    reject: rec('reject', {}),
    runChecks: rec('runChecks', {}),
    resolveHeld: rec('resolveHeld', (id, d) => ({ result_line: d === 'allow_once' ? 'Merged #512 into main.' : null })),
    checkpoints: rec('checkpoints', cps),
    restorePreview: rec('restorePreview', preview),
    restore: rec('restore', { restored: ['src/auth/limit.rs'], deleted: ['src/auth/new.rs'], failed: [], before_restore_checkpoint_id: 'cp-safety', task: { held_at: '2026-11-10T15:30:00Z' } }),
    forkFromCheckpoint: rec('forkFromCheckpoint', { id: 't-copy', stepped_down: { kwant_id: 'k-lin', class: 'auth changes', from: 'trusted', to: 'sandbox' } }),
    ...over,
  };
  return { api, calls, preview };
}

async function mount(item, api, extra = {}) {
  document.body.innerHTML = '';
  sessionStorage.clear();
  const el = document.createElement('div');
  document.body.appendChild(el);
  const done = [];
  const root = createRoot(el);
  root.render(h(ReviewItemView, { item, entry: 'inbox', onDone: (r) => done.push(r), api, ...extra }));
  await settle();
  return { el, done, root, q: (s) => document.querySelector(s), qa: (s) => [...document.querySelectorAll(s)] };
}
const click = async (b) => {
  b.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};
const key = async (k, target = document.body) => {
  target.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));
  await settle();
};
const type = async (ta, text) => {
  const set = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set;
  set.call(ta, text);
  ta.dispatchEvent(new Event('input', { bubbles: true }));
  await settle();
};

// ---- helpers ----

test('helpers: summaries come from the server numbers, tail keeps 12 lines', () => {
  assert.equal(m.criteriaSummaryText({ met: 1, total: 4, not_met: 1, not_checked: 2 }), '1 of 4 met · 1 not met · 2 not checked');
  assert.equal(m.decisionsSummaryText(FX.needsYou.decisions), '2 for you, of 4');
  assert.equal(m.evidenceSummaryText(FX.needsYou.evidence), '1 check · 1 screenshot · shop #512');
  const out = Array.from({ length: 20 }, (_, i) => `line ${i + 1}`).join('\n');
  const t = m.tailLines(out);
  assert.equal(t.more, true);
  assert.equal(t.lines.split('\n').length, 12);
  assert.equal(t.lines.split('\n')[0], 'line 9');
  assert.equal(m.nextCriterionIndex(FX.failRoundLimit.criteria), 7);
  assert.equal(m.notUndoneTitle({ effects_known: false, external_not_undone: [] }), 'Not undone: unknown');
  assert.equal(m.notUndoneTitle({ effects_known: true, external_not_undone: [] }), null);
  assert.equal(m.sendBackSteps(FX.needsYou, 1)[0], 'The held merge of #512 is denied first, so nothing runs.');
  assert.equal(m.sendBackSteps(FX.heldDenied, 0).some((s) => /denied first/.test(s)), false);
  const withChecks = m.sendBackSteps(FX.heldDenied, 0);
  const noChecks = m.sendBackSteps(FX.heldDenied, 0, true);
  assert.equal(noChecks.length, withChecks.length - 1, 'no-checks drops the checker line');
  assert.equal(noChecks.some((s) => /checker/.test(s)), false);
  assert.match(noChecks[0], /starts round \d+ with your note and the work so far\.$/);
  assert.equal(noChecks.at(-1), withChecks.at(-1), 'the round line stays');
});

test('invalid data falls back to the plain card with the title and summary as text', async () => {
  const { api } = fakeApi();
  const bad = { ...FX.needsYou, verdict: null, summary: '<b>not html</b>' };
  const { q } = await mount(bad, api);
  assert.ok(q('[data-review-fallback]'));
  assert.equal(q('[data-review-fallback] b'), null);
  assert.match(q('[data-review-fallback]').textContent, /<b>not html<\/b>/);
  const { q: q2 } = await mount({ kind: 'question', id: 'q1', title: 'Ship tonight?', summary: 'Choices', group: 'waiting_on_you', sort_key: [] }, api);
  assert.ok(q2('[data-review-fallback]'));
});

test('every fixture renders without throwing, with a verdict box', async () => {
  for (const [name, item] of Object.entries(FX)) {
    const { api } = fakeApi();
    const { q } = await mount(item, api);
    assert.ok(q(`[data-verdict="${item.being_checked && item.verdict.result === 'none' ? 'being_checked' : item.verdict.result}"]`), name);
  }
});

test('held action: Approve is aria-disabled with the reason; A focuses Deny and never approves or allows', async () => {
  const { api, calls } = fakeApi();
  const { q, done } = await mount(FX.needsYou, api);
  const ap = q('[data-approve]');
  assert.equal(ap.getAttribute('aria-disabled'), 'true');
  assert.equal(q('[data-approve-blocked]').textContent.trim(), 'Allow or deny the held action first');
  assert.equal(ap.getAttribute('aria-keyshortcuts'), 'A');
  await key('a');
  assert.ok(document.activeElement === q('[data-held-deny]'), 'focus');
  await click(ap);
  assert.ok(document.activeElement === q('[data-held-deny]'), 'focus');
  assert.deepEqual(calls, []);
  assert.equal(done.length, 0);
  // Allow once: the held box shows what ran, Approve comes back.
  await click(q('[data-held-allow]'));
  assert.deepEqual(calls[0], ['resolveHeld', 'a-merge', 'allow_once']);
  assert.equal(q('[data-held]').getAttribute('data-held-state'), 'allowed');
  assert.match(q('[data-held]').textContent, /Merged #512 into main\./);
  assert.equal(q('[data-approve]').getAttribute('aria-disabled'), null);
  await key('a');
  assert.deepEqual(calls[1], ['verify', 't-r-needs', 'v-needs']);
  assert.equal(done[0].kind, 'approved');
  assert.equal(done[0].task_id, 't-r-needs');
});

test('resolved held actions from the server keep their result and do not block Approve', async () => {
  const { api } = fakeApi();
  const { q, qa } = await mount(FX.heldDenied, api);
  assert.deepEqual(qa('[data-held]').map((e) => e.getAttribute('data-held-state')), ['denied', 'expired']);
  assert.equal(qa('[data-held-deny]').length, 0);
  assert.equal(q('[data-approve]').getAttribute('aria-disabled'), null);
  assert.match(qa('[data-held]')[1].textContent, /Expired before anyone answered/);
});

test('a held action that expired under us (409) is shown expired, not as an error', async () => {
  const { api } = fakeApi({ resolveHeld: async () => { throw new InboxError(409, 'gone'); } });
  const { q } = await mount(FX.passHeld, api);
  await click(q('[data-held-deny]'));
  assert.equal(q('[data-held]').getAttribute('data-held-state'), 'expired');
  assert.equal(q('[data-approve]').getAttribute('aria-disabled'), null);
});

test('stale 409 on approve: onDone stale with the paths, nothing approved', async () => {
  const { api } = fakeApi({ verify: async () => { throw new InboxError(409, 'stale', 'stale', ['README.md']); } });
  const { q, done } = await mount(FX.oneTap, api);
  await click(q('[data-approve]'));
  assert.equal(done[0].kind, 'stale');
  assert.deepEqual(done[0].paths, ['README.md']);
  assert.match(done[0].text, /back in Failed or stuck, and nothing was approved\./);
  // V1-4: "changed after its check" is said once: in the stale box, not again in a notice or in the text the Inbox shows.
  assert.doesNotMatch(done[0].text, /changed after its check/);
  assert.match(q('[data-stale]').textContent, /README\.md changed after its check\./);
  assert.equal(document.body.textContent.split('changed after its check').length - 1, 1);
  assert.equal(q('[data-notice]'), null);
  // When the Inbox hands back the item with `stale` set, it is still drawn once.
  const { q: q2 } = await mount({ ...FX.oneTap, stale: { paths: ['README.md'], at: '2026-11-10T15:00:00Z' } }, api);
  assert.equal(document.body.textContent.split('changed after its check').length - 1, 1);
  assert.ok(q2('[data-check-again]'));
});

test('D7 Being checked: the word and line, Approve stays enabled and A sends verify with no verdict id', async () => {
  const { api, calls } = fakeApi();
  const { q, done } = await mount(FX.beingChecked, api);
  assert.ok(q('[data-verdict="being_checked"]'));
  assert.match(q('[data-verdict]').textContent, /Being checked/);
  assert.doesNotMatch(q('[data-verdict]').textContent, /Not checked yet/);
  assert.equal(q('[data-verdict-line]').textContent, 'The verifier is checking this now. You can still approve it yourself.');
  assert.equal(q('[data-approve]').getAttribute('aria-disabled'), null);
  assert.equal(q('[data-approve-blocked]'), null);
  await key('a');
  assert.deepEqual(calls, [['verify', 't-r-being', null]]);
  assert.equal(done[0].kind, 'approved');
});

test('D7: no hold (field missing or false) stays "Not checked yet"; a verdict wins over the flag', async () => {
  const { api } = fakeApi();
  assert.equal(FX.notChecked.being_checked, undefined, 'the fixture has no field: missing reads as false');
  const { q } = await mount(FX.notChecked, api);
  assert.ok(q('[data-verdict="none"]'));
  assert.match(q('[data-verdict]').textContent, /Not checked yet/);
  assert.equal(q('[data-verdict-line]').textContent, FX.notChecked.verdict.line);
  const { q: q2 } = await mount({ ...FX.beingChecked, being_checked: false }, api);
  assert.match(q2('[data-verdict]').textContent, /Not checked yet/);
  const { q: q3 } = await mount({ ...FX.oneTap, being_checked: true }, api);
  assert.ok(q3('[data-verdict="pass"]'));
  assert.equal(m.verdictWord('none'), 'Not checked yet');
});

test('V1-3: denying, allowing or finding a held action expired tells the list (onHeldResolved)', async () => {
  const seen = [];
  const { api } = fakeApi();
  const { q } = await mount(FX.passHeld, api, { onHeldResolved: (id, st) => seen.push([id, st]) });
  await click(q('[data-held-deny]'));
  assert.deepEqual(seen, [['a-refund', 'denied']]);
  const { api: gone } = fakeApi({ resolveHeld: async () => { throw new InboxError(404, 'gone'); } });
  const { q: q2 } = await mount(FX.needsYou, gone, { onHeldResolved: (id, st) => seen.push([id, st]) });
  await click(q2('[data-held-deny]'));
  assert.deepEqual(seen[1], ['a-merge', 'expired']);
  const { api: bad } = fakeApi({ resolveHeld: async () => { throw new InboxError(500, 'HTTP 500'); } });
  const { q: q3 } = await mount(FX.needsYou, bad, { onHeldResolved: (id, st) => seen.push([id, st]) });
  await click(q3('[data-held-deny]'));
  assert.equal(seen.length, 2, 'a failed deny is not reported');
});

test('approve error that is not stale stays in the view; nothing reported done', async () => {
  const { api } = fakeApi({ verify: async () => { throw new InboxError(500, 'HTTP 500'); } });
  const { q, done } = await mount(FX.oneTap, api);
  await click(q('[data-approve]'));
  assert.equal(done.length, 0);
  assert.equal(q('[data-notice]').getAttribute('data-notice'), 'bad');
});

test('send back denies the held action first, then rejects with the note; Becomes criterion N', async () => {
  const { api, calls } = fakeApi();
  const { q, done } = await mount(FX.needsYou, api);
  await key('s');
  const ta = q('[data-send-note]');
  assert.ok(ta);
  assert.equal(document.activeElement, ta);
  assert.equal(q('[data-send-submit]').disabled, true);
  await type(ta, 'Count per account too');
  assert.match(q('[data-becomes]').textContent, /Becomes criterion 5:/);
  assert.match(q('[data-send-form]').textContent, /The held merge of #512 is denied first/);
  await click(q('[data-send-submit]'));
  assert.deepEqual(calls.map((c) => c[0]), ['resolveHeld', 'reject']);
  assert.deepEqual(calls[0], ['resolveHeld', 'a-merge', 'deny']);
  assert.deepEqual(calls[1], ['reject', 't-r-needs', 'Count per account too']);
  assert.equal(done[0].kind, 'sent_back');
  assert.equal(done[0].text, 'Sent back to Lin. Round 2 starts now.');
});

test('send back stops when the deny fails: no reject, the error shows in the panel', async () => {
  const { api, calls } = fakeApi({ resolveHeld: async () => { throw new InboxError(500, 'HTTP 500'); } });
  const { q, done } = await mount(FX.needsYou, api);
  await click(q('[data-send-back]'));
  await type(q('[data-send-note]'), 'Again');
  await click(q('[data-send-submit]'));
  assert.equal(calls.some((c) => c[0] === 'reject'), false);
  assert.equal(done.length, 0);
  assert.match(q('[data-send-form] [role="alert"]').textContent, /nothing was sent/);
});

test('decision Change prefills the send back note', async () => {
  const { api } = fakeApi();
  const { q } = await mount(FX.needsYou, api);
  await click(q('[data-fold="Decisions"] button'));
  await click(q('[data-decision-change]'));
  assert.equal(q('[data-send-note]').value, 'Change: Count failed sign-ins per IP or per account? → ');
});

test('folds: closed by default, aria-expanded and controls, remembered for the session', async () => {
  const { api } = fakeApi();
  const { q } = await mount(FX.needsYou, api);
  const b = q('[data-fold="Evidence"] button');
  assert.equal(b.getAttribute('aria-expanded'), 'false');
  assert.ok(document.getElementById(b.getAttribute('aria-controls')).hidden);
  await click(b);
  assert.equal(b.getAttribute('aria-expanded'), 'true');
  assert.ok(q('[data-pr-card]'));
  assert.ok(q('[data-shot]'));
  assert.ok(q('[data-check="unit"] pre'));
  assert.equal(JSON.parse(sessionStorage.getItem('kw-review-folds')).evidence, true);
  // Couldn't check is open when it has anything.
  assert.match(q('[data-couldnt-check]').textContent, /Safari login/);
});

test('round 2 box: your note, the flips and the diff, from the server', async () => {
  const { api } = fakeApi();
  const { q } = await mount(FX.failRoundLimit, api);
  const t = q('[data-round-box]').textContent;
  assert.match(t, /Round 2 · what changed since round 1/);
  assert.match(t, /Your note is now criterion 5, and it’s Met\./);
  assert.match(t, /Criterion 2 went Not met → Met\./);
  assert.match(t, /\+14 −6 in 2 files since round 1/);
  assert.ok(q('[data-last-round]'));
  assert.ok(q('[data-round-chip]'));
});

test('roll back: preview rows, restore with the preview sha and an idempotency key, then Undo', async () => {
  const { api, calls } = fakeApi();
  const { q, done } = await mount(FX.oneTap, api);
  await click(q('[data-roll-back]'));
  assert.ok(q('[role="dialog"]'));
  assert.equal(q('[data-rollback-confirm]').textContent.trim(), 'Roll back 2 files');
  assert.equal(q('[data-not-undone]').getAttribute('data-not-undone'), 'none');
  assert.ok(document.activeElement === q('[data-keep-work]'), 'focus');
  await click(q('[data-rollback-confirm]'));
  const r = calls.find((c) => c[0] === 'restore');
  assert.equal(r[1], 't-r-onetap');
  assert.equal(r[2], 'cp-before');
  assert.equal(r[3].preview_sha256, 'sha-1');
  assert.ok(typeof r[4] === 'string' && r[4].length > 8);
  assert.equal(done[0].kind, 'rolled_back');
  assert.equal(done[0].undo_checkpoint_id, 'cp-safety');
  assert.match(done[0].text, /1 file put back, 1 removed\. The task is on hold until you press Run\./);
  assert.ok(q('[data-undo-rollback]'));
});

test('roll back shows "Not undone: unknown" when effects_known is false', async () => {
  const base = fakeApi();
  const { api } = fakeApi({ restorePreview: async () => ({ ...base.preview, effects_known: false }) });
  const { q } = await mount(FX.oneTap, api);
  await click(q('[data-roll-back]'));
  const nu = q('[data-not-undone]');
  assert.equal(nu.getAttribute('data-not-undone'), 'unknown');
  assert.match(nu.textContent, /Not undone: unknown/);
});

test('roll back with a held action denies it first; a stale preview reloads', async () => {
  let n = 0;
  const base = fakeApi();
  const { api, calls } = fakeApi({
    restore: async () => {
      n++;
      throw new MissionError('stale', 409, 'stale_preview');
    },
  });
  const { q } = await mount(FX.needsYou, api);
  await click(q('[data-roll-back]'));
  assert.match(q('[data-review-dialog]').textContent, /The held action is denied first\./);
  await click(q('[data-rollback-confirm]'));
  assert.deepEqual(calls.find((c) => c[0] === 'resolveHeld'), ['resolveHeld', 'a-merge', 'deny']);
  assert.equal(n, 1);
  assert.match(q('[data-rollback-error]').textContent, /Files changed since this preview/);
  assert.equal(calls.filter((c) => c[0] === 'restorePreview').length, 2);
  void base;
});

test('roll back after approval (via fork): after-approval lines, step-down, fork call, copy', async () => {
  const base = fakeApi();
  const { api, calls } = fakeApi({
    // As missionsApi.restorePreview returns it: a fork-path preview carries refused 'fork_instead'.
    restorePreview: async () => ({ ...base.preview, via: 'fork', refused: 'fork_instead', verified_at: '2026-11-10T14:50:00Z', step_down: { kwant_id: 'k-mira', class: 'copy changes', from: 'trusted', to: 'sandbox' } }),
  });
  const opened = [];
  const { q, done } = await mount(FX.oneTap, api, { onOpenTask: (id) => opened.push(id) });
  await click(q('[data-roll-back]'));
  assert.ok(q('[data-after-approval]'));
  assert.equal(q('[data-rollback-refused]'), null, 'the copy path is the roll back: no "isn\u2019t restored" warning');
  assert.equal(q('[data-rollback-confirm]').disabled, false);
  assert.match(q('[data-step-down]').textContent, /Mira moves down one Flight School tier for copy changes/);
  await click(q('[data-rollback-confirm]'));
  const f = calls.find((c) => c[0] === 'forkFromCheckpoint');
  assert.equal(f[1], 't-r-onetap');
  assert.equal(f[2].from_checkpoint, 'cp-before');
  assert.equal(f[2].preview_sha256, 'sha-1');
  assert.ok(f[2].reason.length > 0 && f[2].reason.length <= 500);
  assert.equal(calls.some((c) => c[0] === 'restore'), false);
  assert.equal(done[0].copy_task_id, 't-copy');
  await click(q('[data-open-copy]'));
  assert.deepEqual(opened, ['t-copy']);
});

test('roll back by restore (in review, person check passed) still shows the step-down; none when step_down is null', async () => {
  const base = fakeApi();
  const { api, calls } = fakeApi({
    restorePreview: async () => ({ ...base.preview, via: 'restore', step_down: { kwant_id: 'k-mira', class: 'copy changes', from: 'trusted', to: 'sandbox' } }),
  });
  const { q } = await mount(FX.oneTap, api);
  await click(q('[data-roll-back]'));
  assert.equal(q('[data-after-approval]'), null);
  assert.match(q('[data-step-down]').textContent, /Mira moves down one Flight School tier for copy changes: trusted → sandbox/i);
  await click(q('[data-rollback-confirm]'));
  assert.ok(calls.some((c) => c[0] === 'restore'));
  assert.equal(calls.some((c) => c[0] === 'forkFromCheckpoint'), false);
  const { api: api2 } = fakeApi();
  const { q: q2 } = await mount(FX.oneTap, api2);
  await click(q2('[data-roll-back]'));
  assert.equal(q2('[data-step-down]'), null);
  assert.equal(q2('[data-after-approval]'), null);
});

test('roll back by restore after a person check passed shows "This was approved at <time>"', async () => {
  const base = fakeApi();
  const { api, calls } = fakeApi({
    restorePreview: async () => ({ ...base.preview, via: 'restore', verified_at: '2026-11-10T14:50:00Z', step_down: null }),
  });
  const { q } = await mount(FX.oneTap, api);
  await click(q('[data-roll-back]'));
  const line = q('[data-after-approval]');
  assert.ok(line, 'approved-at line in the restore branch');
  assert.match(line.textContent, /^This was approved at \S.*\. Rolling it back undoes that approval\.$/);
  assert.ok(line.querySelector('strong').textContent.trim().length > 0, 'the time is shown');
  assert.match(q('[data-rollback-body]').textContent, /Puts this task’s files back to how they were before/);
  await click(q('[data-rollback-confirm]'));
  assert.ok(calls.some((c) => c[0] === 'restore'));
  assert.equal(calls.some((c) => c[0] === 'forkFromCheckpoint'), false);
});

test('no roll back button when rollback_ok is false; Escape closes the dialog', async () => {
  const { api } = fakeApi();
  const { q } = await mount(FX.noRollback, api);
  assert.equal(q('[data-roll-back]'), null);
  const { q: q2 } = await mount(FX.oneTap, api);
  await click(q2('[data-roll-back]'));
  await key('Escape', q2('[role="dialog"]'));
  assert.equal(q2('[role="dialog"]'), null);
});

test('stale item: Check again runs the checks', async () => {
  const { api, calls } = fakeApi();
  const { q } = await mount(FX.stale, api);
  assert.match(q('[data-stale]').textContent, /README\.md changed after its check/);
  await click(q('[data-check-again]'));
  assert.deepEqual(calls[0], ['runChecks', 't-r-stale']);
});

test('copy has no emoji and no HTML from data', async () => {
  const { api } = fakeApi();
  for (const item of Object.values(FX)) {
    const { el } = await mount(item, api);
    assert.doesNotMatch(el.textContent, /\p{Extended_Pictographic}/u);
  }
});
