// Show it once (0.7.9, design §S): the words for each step and its masks, the drafted skill in its
// sections (and the SKILL.md as it is when it doesn't split), the store fed by the stage's frames,
// the start dialog, the red Recording bar, the review (delete with Undo, notes, Discard, Write and
// its failure, Writing and Written), and the draft in Skills (badge, Try it once, Delete steps now).
// Every number shown is the server's.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';
import { rec, STEPS, SKILL_BODY, shown, skill } from './fixtures/showOnce.mjs';

const dom = new JSDOM('<!doctype html><div id="root"></div>', { url: 'https://kwant.example/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'localStorage', 'sessionStorage', 'KeyboardEvent']) {
  Object.defineProperty(globalThis, k, { value: k === 'window' ? dom.window : dom.window[k], configurable: true, writable: true });
}
globalThis.requestAnimationFrame = (cb) => setTimeout(cb, 0);
globalThis.cancelAnimationFrame = (id) => clearTimeout(id);
globalThis.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  import React from 'react';
  export * from './src/showOnce';
  export { emitContinuity } from './src/continuity';
  export { ShowOnceStart, RecordingStrip, StepReview, RecordStepsButton } from './src/components/ShowOnce';
  export { DraftSections, TryItOnce, KeepLine, ShownLine, shownOf } from './src/components/SkillDraft';
  export { PendingSkill, ActiveSkill, SkillsPanel } from './src/components/SkillsPanel';
  export { React };
  export { createRoot } from 'react-dom/client';
`);
const h = m.React.createElement;
const src = (f) => readFileSync(new URL(`../src/${f}`, import.meta.url), 'utf8');
const flush = () => new Promise((r) => setTimeout(r, 0));
const settle = async () => {
  for (let i = 0; i < 8; i++) await flush();
};

/** A fetch that answers by "METHOD url" ([status, body]) and records the calls. */
function fake(routes) {
  const calls = [];
  globalThis.fetch = async (url, init = {}) => {
    const method = init.method ?? 'GET';
    calls.push({ url, method, body: init.body ? JSON.parse(init.body) : undefined });
    const r = routes[`${method} ${url}`];
    if (!r) return { ok: false, status: 404, json: async () => ({ error: 'not_found', message: `unexpected ${method} ${url}` }) };
    const [status, body] = typeof r === 'function' ? r() : r;
    return { ok: status >= 200 && status < 300, status, json: async () => body };
  };
  return calls;
}

async function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(el);
  await settle();
  return { host, unmount: () => (root.unmount(), host.remove()) };
}
const byText = (host, sel, text) => [...host.querySelectorAll(sel)].find((e) => e.textContent.trim() === text || e.textContent.includes(text));
const click = async (el) => {
  assert.ok(el, 'element to click');
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const type = async (el, value) => {
  const proto = el.tagName === 'TEXTAREA' ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, value);
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
};
const key = async (k, opts = {}, target = window) => {
  target.dispatchEvent(new window.KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...opts }));
  await settle();
};
const R = '/api/show-once/recordings';
const session = (over = {}) => ({
  kwantId: 'k_rook', botName: 'Rook', stage: 'k_rook', rec: rec(), steps: STEPS, phase: 'recording',
  pending: null, handBackAfter: false, syncedAt: Date.now(), error: null, ...over,
});
const reset = () => m.showOnceStore.set(null);

// ---------------------------------------------------------------- words

test('each step reads as the design says it, and a masked one never shows a value', () => {
  const [page, email, pass, code, signIn, , range, , note, , upload, , , enter, link] = STEPS;
  assert.equal(m.stepSentence(page), 'Opened shop.example.com/admin/login');
  assert.equal(m.stepSentence(email), 'Typed “ops@example.com” in “Email”');
  assert.equal(m.stepSentence(pass), 'Typed in “Password”, kept as password not kept, uses the saved login');
  assert.equal(m.stepSentence(pass, { short: true }), 'Password kept as password not kept, uses the saved login');
  assert.deepEqual(m.stepParts(pass, { short: true })[1], { t: 'mask', text: '[your password]', label: 'password not kept, uses the saved login' });
  assert.deepEqual(m.stepParts(code, { short: true })[1], { t: 'mask', text: '[code]', label: 'code not kept' });
  assert.equal(m.stepSentence(signIn), 'Clicked button “Sign in”');
  assert.equal(m.stepSentence(range), 'Chose “Last 7 days” in “Date range”');
  assert.equal(m.stepSentence(note), 'Note: Only paid orders. Skip refunds.');
  assert.equal(m.stepSentence(upload), 'Uploaded a file in “Orders file”');
  assert.equal(m.stepSentence(enter), 'Pressed Enter');
  assert.equal(m.stepSentence(link), 'Clicked link “View import”');
  // Payment and the others keep the server's placeholder in the bar too
  const card = { id: 'x', n: 1, kind: 'type', label: 'Card number', masked: 'payment', placeholder: '[card details]' };
  assert.deepEqual(m.stepParts(card, { short: true }).at(-1), { t: 'mask', text: '[card details]', label: 'card details not kept' });
});

test('steps and recordings off-shape are left out', () => {
  assert.equal(m.isStep({ id: 1, n: 1, kind: 'page', origin: 'https://a.b' }), false, 'page needs a path');
  assert.equal(m.isStep({ id: 1, n: 1, kind: 'type', label: 'Password', masked: 'password' }), false, 'a mask needs its placeholder');
  assert.equal(m.isStep({ id: 1, n: 1, kind: 'type', label: 'Email' }), false, 'unmasked typing needs its text');
  assert.equal(m.isStep({ id: 1, n: 1, kind: 'note', value: '  ' }), false);
  assert.equal(m.isStep({ id: 1, n: 1, kind: 'script', value: '<b>x</b>' }), false);
  assert.equal(m.cleanSteps([STEPS[0], { id: 9 }, null]).length, 1);
  assert.equal(m.isRecording(rec()), true);
  assert.equal(m.isRecording({ ...rec(), state: 'paused' }), false);
  assert.equal(m.isSkillShown({ shown_on: 'Nov 7, 2026' }), false);
});

test('the clock, durations, the near-limit line and the review subtitle', () => {
  assert.equal(m.clock(168_000), '2:48');
  assert.equal(m.clock(-5), '0:00');
  assert.equal(m.duration(220_000), '3 min 40 s');
  assert.equal(m.duration(120_000), '2 min');
  assert.equal(m.duration(9_000), '9 s');
  assert.equal(m.stepsWord(1), '1 step');
  const L = m.DEFAULT_LIMITS;
  assert.equal(m.nearLimit(14, 168_000, L), null);
  assert.equal(m.nearLimit(192, 168_000, L), '28 min or 8 steps left');
  assert.equal(m.nearLimit(14, 1_680_000, L), '2 min or 186 steps left');
  assert.equal(m.nearLimit(14, 168_000, L, true), '28 min or 186 steps left', 'the server’s near flag wins');
  assert.equal(m.reviewSub(rec({ elapsed_secs: 220 }), 15), '15 steps · 3 min 40 s · shop.example.com, crm.example.com');
  assert.equal(m.reviewSub(rec({ elapsed_secs: 1800, stop_reason: 'limit_time' }), 120), '120 steps · 30:00 · stopped at the limit');
  assert.deepEqual(m.stopBanner(rec({ stop_reason: 'limit_time' }), 120), {
    lead: 'Stopped at 30 minutes,', rest: ' the most one recording can take. All 120 steps up to there are kept.',
  });
  assert.equal(m.stopBanner(rec({ stop_reason: 'stop' }), 3), null);
  assert.match(m.stopBanner(rec({ stop_reason: 'restart' }), 3).lead, /the app restarted/);
});

test('Write the skill says why it can’t yet', () => {
  assert.equal(m.writeBlocker(rec(), STEPS, 'Rook'), null);
  assert.equal(m.writeBlocker(rec({ engine: null }), STEPS, 'Rook'), 'Rook has no engine to write the skill with.');
  const onlyNotes = [STEPS[8], { ...STEPS[0], deleted: true }];
  assert.match(m.writeBlocker(rec(), onlyNotes, 'Rook'), /no steps left/);
});

// ---------------------------------------------------------------- the drafted SKILL.md

test('the draft splits into Goal, When to use, Inputs, Steps, Checks, What can go wrong and Sites', () => {
  const d = m.parseSkillSections(SKILL_BODY);
  assert.match(d.goal, /^Export last week/);
  assert.match(d.whenToUse, /Every Monday/);
  assert.deepEqual(d.inputs, [{ name: 'date_range', example: 'Last 7 days' }, { name: 'import_type', example: 'Orders' }]);
  assert.equal(d.steps.length, 5);
  assert.match(d.checks, /row count/);
  assert.match(d.wrong, /expires/);
  assert.deepEqual(d.sites, ['shop.example.com', 'crm.example.com']);
  // Placeholders become chips; `code` stays code
  const parts = m.inlineParts(d.steps[0]);
  assert.ok(parts.some((p) => p.t === 'code' && p.v === 'ops@example.com'));
  assert.ok(parts.some((p) => p.t === 'mask' && p.label === 'password not kept, uses the saved login'));
});

test('a draft that doesn’t split is shown as the SKILL.md it is, never as HTML', async () => {
  assert.equal(m.parseSkillSections('Just do the thing <img src=x onerror=alert(1)>'), null);
  assert.equal(m.parseSkillSections('## Goal\nSomething\n'), null, 'no steps');
  const body = 'Just do the thing <img src=x onerror=alert(1)>';
  const { host, unmount } = await mount(h(m.DraftSections, { body, shown: shown(), label: 'Draft' }));
  assert.equal(host.querySelector('pre').textContent, body);
  assert.equal(host.querySelector('img'), null);
  unmount();
});

test('the draft card: inputs with the run’s examples, the site rule and “Shown by you on …”', async () => {
  const { host, unmount } = await mount(h(m.DraftSections, { body: SKILL_BODY, shown: shown(), label: 'Draft export-weekly-orders' }));
  const region = host.querySelector('[role="region"]');
  assert.equal(region.getAttribute('aria-label'), 'Draft export-weekly-orders');
  assert.deepEqual([...host.querySelectorAll('h6')].map((x) => x.textContent), ['Goal', 'When to use', 'Inputs', 'Steps', 'Checks', 'What can go wrong', 'Sites']);
  assert.equal(host.querySelectorAll('tbody tr').length, 2);
  assert.equal(host.querySelector('th[scope="col"]').textContent, 'Name');
  assert.match(host.textContent, /Your Browser site rules still apply\./);
  assert.match(host.textContent, /Shown by you on Nov 7, 2026 · from 14 steps/);
  assert.equal(host.querySelector('[role="img"]').getAttribute('aria-label'), 'password not kept, uses the saved login');
  unmount();
});

// ---------------------------------------------------------------- the store

test('frames from the stage land in the session; another recording’s are ignored', async () => {
  fake({ [`GET ${R}/rec_1`]: [200, { recording: rec({ step_count: 15, state: 'stopped', stop_reason: 'stop' }), steps: STEPS }] });
  m.showOnceStore.set(session({ steps: STEPS.slice(0, 2), rec: rec({ step_count: 2 }) }));
  m.onRecordingFrame({ type: 'skill.recording.step', recording_id: 'rec_1', step: STEPS[2] });
  assert.equal(m.showOnceStore.get().steps.length, 3);
  assert.equal(m.showOnceStore.get().rec.step_count, 3);
  m.onRecordingFrame({ type: 'skill.recording.step', recording_id: 'rec_1', step: STEPS[2] });
  assert.equal(m.showOnceStore.get().rec.step_count, 3, 'a repeat frame isn’t counted twice');
  m.onRecordingFrame({ type: 'skill.recording.step', recording_id: 'rec_other', step: STEPS[3] });
  assert.equal(m.showOnceStore.get().steps.length, 3);
  m.onRecordingFrame({ type: 'skill.recording.step', recording_id: 'rec_1', step: { id: 'bad', n: 4, kind: 'type', label: 'x' } });
  assert.equal(m.showOnceStore.get().steps.length, 3, 'a malformed step is left out');
  m.onRecordingFrame({ type: 'skill.recording.near', recording_id: 'rec_1' });
  assert.equal(m.showOnceStore.get().rec.near, true);
  m.onRecordingFrame({ type: 'skill.recording.stopped', recording_id: 'rec_1' });
  assert.equal(m.showOnceStore.get().phase, 'review');
  await settle();
  assert.equal(m.showOnceStore.get().steps.length, 15, 'the review reads the server’s list');
  assert.equal(m.showOnceStore.get().rec.step_count, 15);
  reset();
});

test('the elapsed clock is the server’s figure plus the time since, only while recording', () => {
  const s = session({ rec: rec({ elapsed_secs: 100 }), syncedAt: 1000 });
  assert.equal(m.elapsedNow(s, 4000), 103_000);
  assert.equal(m.elapsedNow({ ...s, phase: 'review' }, 4000), 100_000);
});

// ---------------------------------------------------------------- A26 start dialog

test('Start recording: Start at prefilled, the optional line focused, Esc cancels, Start sends both', async () => {
  let started = null;
  let cancelled = 0;
  const { host, unmount } = await mount(
    h(m.ShowOnceStart, { botName: 'Rook', open: true, defaultUrl: 'https://shop.example.com/admin', onCancel: () => cancelled++, onStart: (u, n) => (started = [u, n]) }),
  );
  const dlg = host.querySelector('[role="dialog"]');
  assert.equal(dlg.getAttribute('aria-modal'), 'true');
  assert.equal(document.getElementById(dlg.getAttribute('aria-labelledby')).textContent, 'Show Rook once');
  const [url, what] = host.querySelectorAll('input');
  assert.equal(url.value, 'https://shop.example.com/admin');
  assert.equal(document.activeElement, what);
  assert.equal(host.querySelector(`label[for="${url.id}"]`).textContent, 'Start at');
  assert.match(host.textContent, /Up to 200 steps or 30 minutes\./);
  assert.match(host.textContent, /No screenshots\./);
  await key('Escape');
  assert.equal(cancelled, 1);
  await type(what, '  Export last week’s orders  ');
  host.querySelector('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  assert.deepEqual(started, ['https://shop.example.com/admin', 'Export last week’s orders']);
  unmount();
});

test('an empty Start at and note send nulls (Start uses the page the browser is on)', async () => {
  let started = null;
  const { host, unmount } = await mount(h(m.ShowOnceStart, { botName: 'Rook', open: true, onCancel: () => {}, onStart: (u, n) => (started = [u, n]) }));
  assert.match(host.textContent, /that you uploaded a file \(never its name\)/, 'api.md §3: an upload keeps the label, never the file name');
  await click(byText(host, 'button', 'Start recording'));
  assert.deepEqual(started, [null, null]);
  unmount();
});

test('begin, then Start once Take over holds: start_url and first_note go once; a failure says so with Try again', async () => {
  let fail = true;
  const calls = fake({
    [`POST ${R}`]: () => (fail ? [409, { error: 'stage_busy' }] : [201, { recording: rec({ step_count: 0, elapsed_secs: 0 }) }]),
    [`GET ${R}/rec_1`]: [200, { recording: rec({ step_count: 0, elapsed_secs: 0 }), steps: [] }],
  });
  m.showOnce.begin('k_rook', 'Rook', 'k_rook', 'https://shop.example.com', 'Orders');
  const { host, unmount } = await mount(h(m.RecordingStrip, { stage: 'k_rook' }));
  assert.match(host.textContent, /Taking over the browser…/);
  assert.equal(host.querySelector('[role="status"]').getAttribute('aria-live'), 'polite');
  await m.showOnce.startNow();
  await settle();
  assert.match(host.textContent, /The recording didn’t start\./);
  assert.match(host.textContent, /Another bot is recording in this browser\./);
  assert.deepEqual(calls[0].body, { bot: 'k_rook', stage: 'k_rook', start_url: 'https://shop.example.com', first_note: 'Orders' });
  fail = false;
  await click(byText(host, 'button', 'Try again'));
  assert.equal(m.showOnceStore.get().phase, 'recording');
  assert.match(host.textContent, /Waiting for your first step…/);
  assert.match(host.textContent, /0 steps · 0:0/);
  unmount();
  reset();
});

test('already_open brings back the open recording instead of failing', async () => {
  fake({
    [`POST ${R}`]: [409, { error: 'already_open', recording: rec({ state: 'stopped', stop_reason: 'restart' }) }],
    [`GET ${R}/rec_1`]: [200, { recording: rec({ state: 'stopped', stop_reason: 'restart' }), steps: STEPS }],
  });
  assert.equal(await m.showOnce.record('k_rook', 'Rook', 'k_rook'), null);
  await settle();
  assert.equal(m.showOnceStore.get().phase, 'review');
  assert.equal(m.showOnceStore.get().steps.length, 15);
  reset();
});

test('a recording left open comes back when the stage opens (S-B5)', async () => {
  fake({
    [`GET ${R}?bot=k_rook`]: [200, { recording: rec() }],
    [`GET ${R}/rec_1`]: [200, { recording: rec(), steps: STEPS }],
  });
  await m.showOnce.resumeOpen('k_rook', 'Rook', 'k_rook');
  assert.equal(m.showOnceStore.get().phase, 'recording');
  assert.equal(m.showOnceStore.get().steps.length, 15);
  reset();
  fake({ [`GET ${R}?bot=k_rook`]: [200, { recording: null }] });
  await m.showOnce.resumeOpen('k_rook', 'Rook', 'k_rook');
  assert.equal(m.showOnceStore.get(), null);
});

// ---------------------------------------------------------------- A28 the bar

test('the bar: Recording, the server’s count and the clock, the last step short, Stop and Add a note by name and key', async () => {
  const calls = fake({
    [`POST ${R}/rec_1/stop`]: [200, { recording: rec({ state: 'stopped', stop_reason: 'stop' }) }],
    [`GET ${R}/rec_1`]: [200, { recording: rec({ state: 'stopped', stop_reason: 'stop' }), steps: STEPS }],
  });
  m.showOnceStore.set(session({ steps: STEPS.slice(0, 3) }));
  const { host, unmount } = await mount(h(m.RecordingStrip, { stage: 'k_rook' }));
  const bar = host.querySelector('[role="region"]');
  assert.equal(bar.getAttribute('aria-label'), 'Recording');
  assert.match(bar.textContent, /14 steps · 2:4[89]/);
  const last = host.querySelector('.so-last');
  assert.equal(last.getAttribute('aria-live'), 'polite');
  assert.equal(last.getAttribute('aria-label'), 'Last step: Password kept as password not kept, uses the saved login');
  assert.match(last.textContent, /Password kept as \[your password\]/);
  assert.doesNotMatch(host.innerHTML, /ops@example|hunter/);
  const stop = byText(host, 'button', 'Stop');
  assert.match(stop.getAttribute('aria-keyshortcuts'), /Shift\+S/);
  assert.match(byText(host, 'button', 'Add a note').getAttribute('aria-keyshortcuts'), /Shift\+N/);
  // Ctrl+Shift+N opens the note, Esc closes it and gives focus back
  await key('N', { ctrlKey: true, shiftKey: true });
  const pop = host.querySelector('[role="dialog"]');
  assert.equal(pop.getAttribute('aria-label'), 'Add a note');
  assert.equal(document.activeElement, pop.querySelector('textarea'));
  assert.match(pop.textContent, /Saved as the next step, as you wrote it\. Esc closes\./);
  await key('Escape', {}, pop.querySelector('textarea'));
  assert.equal(host.querySelector('[role="dialog"]'), null);
  assert.equal(document.activeElement, byText(host, 'button', 'Add a note'));
  // Ctrl+Shift+S stops
  await key('S', { ctrlKey: true, shiftKey: true });
  assert.ok(calls.some((c) => c.method === 'POST' && c.url === `${R}/rec_1/stop`));
  assert.equal(m.showOnceStore.get().phase, 'review');
  assert.equal(host.textContent, '', 'the bar goes; the review takes over');
  unmount();
  reset();
});

test('near the limit the bar says what’s left (A34); the phone bar has the same parts', async () => {
  m.showOnceStore.set(session({ rec: rec({ step_count: 193 }) }));
  const { host, unmount } = await mount(h(m.RecordingStrip, { stage: 'k_rook', phone: true }));
  assert.ok(host.querySelector('.so-bar.ph'));
  assert.match(host.querySelector('.so-near').textContent, /28 min or 7 steps left/);
  assert.ok(byText(host, 'button', 'Stop'));
  assert.ok(byText(host, 'button', 'Add a note'));
  unmount();
  reset();
});

test('a note from the bar is saved as written and the bar shows it as the last step', async () => {
  const noteStep = { id: 16, n: 16, kind: 'note', value: 'Then check the total.', masked: null, placeholder: null };
  const calls = fake({
    [`POST ${R}/rec_1/notes`]: [201, { step: noteStep }],
    [`GET ${R}/rec_1`]: [200, { recording: rec({ note_count: 2 }), steps: [...STEPS, noteStep] }],
  });
  m.showOnceStore.set(session());
  const { host, unmount } = await mount(h(m.RecordingStrip, { stage: 'k_rook' }));
  await click(byText(host, 'button', 'Add a note'));
  const add = byText(host, 'button', 'Add note');
  assert.equal(add.disabled, true);
  await type(host.querySelector('textarea'), 'Then check the total.');
  await click(byText(host, 'button', 'Add note'));
  assert.deepEqual(calls.find((c) => c.url.endsWith('/notes')).body, { text: 'Then check the total.' });
  assert.equal(host.querySelector('[role="dialog"]'), null);
  assert.match(host.querySelector('.so-last').textContent, /Then check the total\./);
  unmount();
  reset();
});

test('a recording on another stage shows nothing here', async () => {
  m.showOnceStore.set(session({ stage: 'k_other' }));
  const { host, unmount } = await mount(h(m.RecordingStrip, { stage: 'k_rook' }));
  assert.equal(host.textContent, '');
  unmount();
  reset();
});

// ---------------------------------------------------------------- S4 the review

const reviewing = (over = {}) => session({ phase: 'review', rec: rec({ state: 'stopped', stop_reason: 'stop', elapsed_secs: 220, ...over }) });

test('Check the steps: every step numbered and named, masks as chips, the engine line, focus on the heading', async () => {
  m.showOnceStore.set(reviewing());
  const { host, unmount } = await mount(h(m.StepReview));
  const dlg = host.querySelector('[role="dialog"]');
  assert.ok(dlg, 'a dialog');
  assert.equal(document.activeElement?.tagName, 'H3');
  assert.match(host.textContent, /15 steps · 3 min 40 s · shop\.example\.com, crm\.example\.com/);
  const rows = [...host.querySelectorAll('li.so-step')];
  assert.equal(rows.length, 15);
  assert.equal(rows[2].getAttribute('aria-label'), 'Step 3: Typed in “Password”, kept as password not kept, uses the saved login');
  assert.ok(rows[2].classList.contains('secret'));
  assert.ok(host.querySelector('button[aria-label="Delete step 3"]'));
  assert.ok(host.querySelector('button[aria-label="Add a note after step 3"]'));
  assert.ok(host.querySelector('button[aria-label="Step 3: add a note after or delete"]'));
  assert.match(host.textContent, /Only Rook's own engine \(Claude\) reads these steps\./);
  assert.match(host.textContent, /deleted 7 days after you approve the skill, and at once if you reject it\./);
  assert.ok(byText(host, 'button', 'Write the skill'));
  assert.ok(byText(host, 'button', 'Keep recording'));
  unmount();
  reset();
});

test('delete a step: “Step n deleted.” with Undo focused; Undo puts it back', async () => {
  const gone = { ...STEPS[4], deleted: true, undo_until: new Date(Date.now() + 60_000).toISOString() };
  const calls = fake({
    [`DELETE ${R}/rec_1/steps/5`]: [200, { step: gone }],
    [`POST ${R}/rec_1/steps/5/restore`]: [200, { step: STEPS[4] }],
  });
  m.showOnceStore.set(reviewing());
  const { host, unmount } = await mount(h(m.StepReview));
  await click(host.querySelector('button[aria-label="Delete step 5"]'));
  const row = byText(host, 'li', 'Step 5 deleted.');
  assert.ok(row);
  const undo = host.querySelector('button[aria-label="Undo deleting step 5"]');
  assert.equal(document.activeElement, undo);
  assert.match(host.textContent, /14 steps · /, 'the subtitle counts what is still listed');
  await click(undo);
  assert.ok(calls.some((c) => c.method === 'POST' && c.url.endsWith('/steps/5/restore')));
  assert.equal(byText(host, 'li', 'Step 5 deleted.'), undefined);
  assert.ok(host.querySelector('button[aria-label="Delete step 5"]'));
  unmount();
  reset();
});

test('a note after a step goes in with `after`', async () => {
  const noteStep = { id: 'n2', n: 8, kind: 'note', value: 'Wait for the file.', masked: null, placeholder: null };
  const calls = fake({
    [`POST ${R}/rec_1/notes`]: [201, { step: noteStep }],
    [`GET ${R}/rec_1`]: [200, { recording: rec({ state: 'stopped', stop_reason: 'stop' }), steps: [...STEPS.slice(0, 7), noteStep, ...STEPS.slice(7)] }],
  });
  m.showOnceStore.set(reviewing());
  const { host, unmount } = await mount(h(m.StepReview));
  await click(host.querySelector('button[aria-label="Add a note after step 7"]'));
  const ta = host.querySelector('.so-addnote textarea');
  assert.equal(document.activeElement, ta);
  await type(ta, 'Wait for the file.');
  await click(byText(host, '.so-addnote button', 'Add'));
  assert.deepEqual(calls.find((c) => c.url.endsWith('/notes')).body, { text: 'Wait for the file.', after: 7 });
  assert.equal(host.querySelector('.so-addnote'), null);
  assert.match(host.textContent, /Wait for the file\./);
  unmount();
  reset();
});

test('Escape asks before discarding; Keep them keeps; Discard deletes and closes', async () => {
  const calls = fake({ [`DELETE ${R}/rec_1`]: [204, null] });
  m.showOnceStore.set(reviewing());
  const ended = [];
  const off = m.showOnceStore.onEnd((e) => ended.push(e));
  const { host, unmount } = await mount(h(m.StepReview));
  await key('Escape');
  const conf = host.querySelector('[role="alertdialog"]');
  assert.match(conf.textContent, /Discard all 15 steps\? Nothing is written, and the steps are deleted now\./);
  assert.equal(document.activeElement, byText(host, 'button', 'Keep them'));
  await click(byText(host, 'button', 'Keep them'));
  assert.equal(host.querySelector('[role="alertdialog"]'), null);
  await click(byText(host, 'button', 'Discard'));
  await click(byText(host, '[role="alertdialog"] button', 'Discard'));
  assert.ok(calls.some((c) => c.method === 'DELETE' && c.url === `${R}/rec_1`));
  assert.equal(m.showOnceStore.get(), null);
  assert.equal(host.textContent, '');
  assert.deepEqual(ended, [{ stage: 'k_rook', handBack: false }]);
  off();
  unmount();
});

test('Keep recording goes back to the stage with the same list', async () => {
  fake({ [`POST ${R}/rec_1/resume`]: [200, { recording: rec() }] });
  m.showOnceStore.set(reviewing());
  const { host, unmount } = await mount(h(m.StepReview));
  await click(byText(host, 'button', 'Keep recording'));
  assert.equal(m.showOnceStore.get().phase, 'recording');
  assert.equal(m.showOnceStore.get().steps.length, 15);
  assert.equal(host.textContent, '');
  unmount();
  reset();
});

test('Write: “is writing the skill from 15 steps”, then “wrote the skill”; Close hands the browser back', async () => {
  fake({ [`POST ${R}/rec_1/write`]: [202, { recording: rec({ state: 'writing' }) }] });
  m.showOnceStore.set(reviewing());
  const ended = [];
  const off = m.showOnceStore.onEnd((e) => ended.push(e));
  const { host, unmount } = await mount(h(m.StepReview));
  await click(byText(host, 'button', 'Write the skill'));
  assert.equal(m.showOnceStore.get().phase, 'writing');
  assert.match(host.textContent, /Rook is writing the skill from 15 steps\./);
  assert.match(host.textContent, /You can close this\./);
  assert.equal(host.querySelector('button[aria-label="Delete step 1"]')?.disabled ?? true, true, 'no edits while writing');
  m.showOnceStore.update((s) => m.applyRecording(s, rec({ state: 'written', skill_id: 'sk_1' })));
  await settle();
  assert.match(host.textContent, /Rook wrote the skill\. It's in Skills to review\./);
  await click(byText(host, 'button', 'Close'));
  assert.deepEqual(ended, [{ stage: 'k_rook', handBack: true }]);
  off();
  unmount();
});

test('a failed write comes back to the review with the reason and Try again', async () => {
  m.showOnceStore.set(reviewing({ write_error: 'The engine stopped before it finished.' }));
  fake({ [`POST ${R}/rec_1/write`]: [202, { recording: rec({ state: 'writing' }) }] });
  const { host, unmount } = await mount(h(m.StepReview));
  const alert = host.querySelector('.so-banner.bad, [role="alert"]');
  assert.match(alert.textContent, /The engine stopped before it finished\./);
  assert.ok(byText(host, 'button', 'Try again'));
  unmount();
  reset();
});

test('a stop at the limit says so in a banner; no engine blocks Write with the reason', async () => {
  m.showOnceStore.set(reviewing({ stop_reason: 'limit_steps', step_count: 200, engine: null }));
  const { host, unmount } = await mount(h(m.StepReview));
  assert.match(host.textContent, /Stopped at 200 steps, the most one recording can take\./);
  const w = byText(host, 'button', 'Write the skill');
  assert.equal(w.disabled, true);
  assert.equal(document.getElementById(w.getAttribute('aria-describedby')).textContent, 'Rook has no engine to write the skill with.');
  assert.match(host.textContent, /Only Rook's own engine reads/, 'no engine name to give');
  unmount();
  reset();
});

test('Hand back while recording stops first, and hands back when the review closes (A27)', async () => {
  const calls = fake({
    [`POST ${R}/rec_1/stop`]: [200, { recording: rec({ state: 'stopped', stop_reason: 'handed_back' }) }],
    [`GET ${R}/rec_1`]: [200, { recording: rec({ state: 'stopped', stop_reason: 'handed_back' }), steps: STEPS }],
    [`DELETE ${R}/rec_1`]: [204, null],
  });
  m.showOnceStore.set(session());
  const ended = [];
  const off = m.showOnceStore.onEnd((e) => ended.push(e));
  await m.showOnce.stop(true);
  assert.equal(m.showOnceStore.get().handBackAfter, true);
  assert.deepEqual(calls.find((c) => c.url === `${R}/rec_1/stop`).body, { reason: 'handed_back' }, '§10: stop says why');
  await m.showOnce.discard();
  assert.deepEqual(ended, [{ stage: 'k_rook', handBack: true }]);
  off();
});

// ---------------------------------------------------------------- S5 the draft in Skills

test('Skills: “Show it once” beside “Write a skill” with the help line; Start opens the stage', async () => {
  fake({ ['GET /api/skills?kwant_id=k_rook']: [200, []] });
  let opened = 0;
  const { host, unmount } = await mount(h(m.SkillsPanel, { kwantId: 'k_rook', kwantName: 'Rook', onShowOnceStarted: () => opened++, browserUrl: 'https://shop.example.com/' }));
  const entry = host.querySelector('.so-entry');
  assert.deepEqual([...entry.querySelectorAll('button')].map((b) => b.textContent.trim()), ['Show it once', 'Write a skill']);
  assert.match(host.querySelector('.so-entry-help').textContent, /Passwords and codes are never kept\./);
  await click(byText(entry, 'button', 'Show it once'));
  assert.equal(document.querySelector('[role="dialog"] input').value, 'https://shop.example.com/');
  await click(byText(document.body, 'button', 'Start recording'));
  assert.equal(opened, 1);
  assert.equal(m.showOnceStore.get().phase, 'taking');
  assert.deepEqual(m.showOnceStore.get().pending, { startUrl: 'https://shop.example.com/', firstNote: null });
  unmount();
  reset();
});

test('without a browser to show in, Skills keeps only Write a skill', async () => {
  fake({ ['GET /api/skills?kwant_id=k_rook']: [200, []] });
  const { host, unmount } = await mount(h(m.SkillsPanel, { kwantId: 'k_rook', kwantName: 'Rook' }));
  assert.equal(byText(host, 'button', 'Show it once'), undefined);
  assert.ok(byText(host, 'button', 'Write a skill'));
  unmount();
});

test('a shown draft: the badge, its sections, Approve / Edit then approve / Reject, Try it once, when its steps go', async () => {
  const { host, unmount } = await mount(h(m.PendingSkill, { skill: skill(), botName: 'Rook' }));
  assert.match(host.textContent, /Shown by you/);
  assert.match(host.textContent, /Shown by you on Nov 7, 2026 · from 14 steps/);
  for (const b of ['Approve', 'Edit, then approve', 'Reject', 'Try it once']) assert.ok(byText(host, 'button', b), b);
  assert.match(host.textContent, /The recorded steps are kept until Nov 14\. They're deleted 7 days after you approve, and at once if you reject\./);
  unmount();
});

test('Try it once: inputs prefilled with the run’s examples, a plain warning, Start sends the values', async () => {
  const calls = fake({ ['POST /api/show-once/skills/sk_1/try']: [202, { kwant_id: 'k_rook', skill_id: 'sk_1', message_id: 'm1' }] });
  let startedIn = null;
  const { host, unmount } = await mount(h(m.TryItOnce, { skill: skill(), shown: shown(), botName: 'Rook', onClose: () => {}, onStarted: (k) => (startedIn = k) }));
  const inputs = [...host.querySelectorAll('input')];
  assert.deepEqual(inputs.map((i) => i.value), ['Last 7 days', 'Orders']);
  assert.match(host.textContent, /It runs for real: Export last week's paid orders from the shop and import them into the CRM\. Approvals still ask\. You watch it in Rook's chat\./, 'A29: what it does is the GOAL');
  await type(inputs[0], 'Last 14 days');
  await click(byText(host, 'button', 'Start the trial'));
  assert.deepEqual(calls[0].body, { inputs: { date_range: 'Last 14 days', import_type: 'Orders' } });
  assert.equal(startedIn, 'k_rook');
  assert.match(host.querySelector('[role="status"]').textContent, /Trial started in Rook's chat\./);
  unmount();
});

test('Try it once on a skill that is no longer waiting says why', async () => {
  fake({ ['POST /api/show-once/skills/sk_1/try']: [409, { error: 'not_proposed' }] });
  const { host, unmount } = await mount(h(m.TryItOnce, { skill: skill(), shown: shown(), botName: 'Rook', onClose: () => {} }));
  await click(byText(host, 'button', 'Start the trial'));
  assert.equal(host.querySelector('[role="alert"]').textContent, 'Only a skill waiting for review can be tried.');
  unmount();
});

test('in use: “Shown by you on Nov 7 · recorded steps kept until Nov 14 · Delete steps now”', async () => {
  const calls = fake({ ['DELETE /api/show-once/skills/sk_1/steps']: [200, { recording: rec({ steps_deleted_at: '2026-11-07T11:00:00Z' }) }] });
  const s = skill({ status: 'enabled' });
  const { host, unmount } = await mount(h(m.ShownLine, { skill: s, shown: s.shown }));
  assert.match(host.textContent, /Shown by you\s*on Nov 7\s*·\s*recorded steps kept until Nov 14\s*·\s*Delete steps now/);
  await click(host.querySelector('button[aria-label="Delete the recorded steps of export-weekly-orders now"]'));
  assert.equal(calls[0].method, 'DELETE');
  assert.match(host.querySelector('[role="status"]').textContent, /recorded steps deleted/);
  assert.equal(byText(host, 'button', 'Delete steps now'), undefined);
  unmount();
});

test('an off-shape `shown` makes it an ordinary skill card', () => {
  assert.equal(m.shownOf(skill({ shown: { step_count: 'x' } })), null);
  assert.equal(m.shownOf(skill({ source: 'agent' })), null);
  assert.ok(m.shownOf(skill()));
});

// ---------------------------------------------------------------- wiring, reduced motion, no HTML

test('the stage gets Record steps on desktop only, the bar under its own bar, and Hand back that stops first', () => {
  const vmp = src('components/VirtualMachinePane.tsx');
  assert.match(vmp, /extraAction=\{recordSteps\}/);
  assert.match(vmp, /onHandBack=\{handBackOrStop\}/);
  assert.match(vmp, /interceptHandBack=/);
  assert.match(vmp, /<RecordingStrip stage=\{teamId\} phone \/>/);
  const overlay = src('components/BrowserControlOverlay.tsx');
  assert.doesNotMatch(overlay, /RecordStepsButton/, 'phone starts from Skills');
  assert.match(overlay, /\{belowBar\}/);
  assert.match(src('App.tsx'), /<StepReview \/>/);
});

test('no HTML from data, spinners stop for reduced motion, the dot stops pulsing', () => {
  for (const f of ['components/ShowOnce.tsx', 'components/SkillDraft.tsx', 'showOnce.ts']) {
    const s = src(f);
    assert.doesNotMatch(s, /dangerouslySetInnerHTML|innerHTML/, f);
    for (const spin of s.match(/animate-spin[^"]*/g) ?? []) assert.match(spin, /motion-reduce:animate-none/, f);
  }
  const css = src('styles/showOnce.css');
  assert.match(css, /prefers-reduced-motion:\s*reduce/);
});

test('a Discard elsewhere (SSE with deleted: true, api.md §10.1) closes the review here', async () => {
  m.showOnceStore.set(reviewing());
  m.emitContinuity({ type: 'recording_updated', data: { id: 'rec_other', kwant_id: 'k_rook', space_id: 'sp1', deleted: true } });
  assert.ok(m.showOnceStore.get(), 'another recording: nothing happens');
  m.emitContinuity({ type: 'recording_updated', data: { id: 'rec_1', kwant_id: 'k_rook', space_id: 'sp1', deleted: true } });
  await settle();
  assert.equal(m.showOnceStore.get(), null);
  reset();
});

test('a plain Stop sends no reason', async () => {
  const calls = fake({
    [`POST ${R}/rec_1/stop`]: [200, { recording: rec({ state: 'stopped', stop_reason: 'stop' }) }],
    [`GET ${R}/rec_1`]: [200, { recording: rec({ state: 'stopped', stop_reason: 'stop' }), steps: STEPS }],
  });
  m.showOnceStore.set(session());
  await m.showOnce.stop(false);
  assert.equal(calls.find((c) => c.url === `${R}/rec_1/stop`).body, undefined);
  reset();
});

test('Try it once without a GOAL names the sites; with neither it still reads', async () => {
  fake({});
  const noGoal = SKILL_BODY.replace(/## Goal\n[^\n]*\n/, '');
  let v = await mount(h(m.TryItOnce, { skill: skill({ body: noGoal }), shown: shown(), botName: 'Rook', onClose: () => {} }));
  assert.match(v.host.textContent, /It runs for real on shop\.example\.com and crm\.example\.com\. Approvals still ask\./);
  v.unmount();
  v = await mount(h(m.TryItOnce, { skill: skill({ body: 'Just do it.' }), shown: shown(), botName: 'Rook', onClose: () => {} }));
  assert.match(v.host.textContent, /It runs for real\. Approvals still ask\. You watch it in Rook's chat\./);
  v.unmount();
  assert.deepEqual(m.tryFacts('## Goal\nExport **paid** `orders`.\n'), { goal: 'Export paid orders', sites: [] });
});

test('Start at is prefilled with the bot’s last page (last_url, §10); null keeps the placeholder', async () => {
  fake({ ['GET /api/skills?kwant_id=k_rook']: [200, []], [`GET ${R}?bot=k_rook`]: [200, { recording: null, last_url: 'https://shop.example.com/admin/orders' }] });
  let v = await mount(h(m.SkillsPanel, { kwantId: 'k_rook', kwantName: 'Rook', onShowOnceStarted: () => {} }));
  await click(byText(v.host.querySelector('.so-entry'), 'button', 'Show it once'));
  assert.equal(document.querySelector('[role="dialog"] input').value, 'https://shop.example.com/admin/orders');
  await click(byText(document.body, 'button', 'Cancel'));
  v.unmount();
  fake({ ['GET /api/skills?kwant_id=k_rook']: [200, []], [`GET ${R}?bot=k_rook`]: [200, { recording: null, last_url: null }] });
  v = await mount(h(m.SkillsPanel, { kwantId: 'k_rook', kwantName: 'Rook', onShowOnceStarted: () => {} }));
  await click(byText(v.host.querySelector('.so-entry'), 'button', 'Show it once'));
  const input = document.querySelector('[role="dialog"] input');
  assert.equal(input.value, '');
  assert.equal(input.placeholder, 'The page the browser is on');
  await click(byText(document.body, 'button', 'Cancel'));
  v.unmount();
});

test('trial messages: the marker on the message or in meta; the card goes after the last one once it ended', () => {
  const tr = { skill_id: 'sk_1', recording_id: 'rec_1' };
  assert.deepEqual(m.trialOf({ trial: tr }), tr);
  assert.deepEqual(m.trialOf({ meta: JSON.stringify({ trial: tr }) }), tr);
  assert.equal(m.trialOf({ meta: '{"trial": 1}' }), null);
  assert.equal(m.trialOf({ meta: 'not json "trial"' }), null);
  assert.equal(m.trialOf({ trial: { skill_id: '', recording_id: 'r' } }), null);
  const msgs = [
    { id: 'u1', sender_role: 'user' },
    { id: 's1', sender_role: 'system', trial: tr },
    { id: 'k1', sender_role: 'kwant', trial: tr },
    { id: 'k2', sender_role: 'kwant' },
  ];
  assert.deepEqual([...m.trialCardsAfter(msgs, new Set())], [['k1', tr]], 'after a reload: the bot’s result ends it');
  assert.deepEqual([...m.trialCardsAfter(msgs.slice(0, 2), new Set())], [], 'only the sysline: still running');
  assert.deepEqual([...m.trialCardsAfter(msgs.slice(0, 2), new Set(['sk_1']))], [['s1', tr]], 'the SSE says it ended');
});
