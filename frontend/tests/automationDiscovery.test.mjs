// Automation Discovery screens (Mission Control D4, docs/mission-control.md §20): the API client
// (one Idempotency-Key per Accept intent, the 409 in plain words), the words for every blocked and
// skipped reason, and the Suggested routines section rendered in a DOM against a stubbed server.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'KeyboardEvent', 'MouseEvent', 'SubmitEvent', 'HTMLFormElement', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
g.matchMedia = g.window.matchMedia;
g.window.HTMLElement.prototype.scrollIntoView ??= () => {};
g.window.HTMLMediaElement && (g.window.HTMLMediaElement.prototype.play = () => Promise.resolve());

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * from './src/automationDiscovery';
  export { SuggestedRoutines, SuggestionCard } from './src/components/SuggestedRoutines';
  export { RoutineRow } from './src/components/RoutinesPanel';
  export { notifyKindsFor, notifyOnToSave, shouldShowOsNotification, TOAST_KINDS } from './src/continuity';
`);
const { React, createRoot } = m;

const settle = async () => {
  for (let i = 0; i < 25; i++) await new Promise((r) => setTimeout(r, 2));
};
const act = async (fn) => {
  await fn();
  await settle();
};

const KWANTS = [
  { id: 'k_ada', name: 'Ada', title: 'Lead', persona_prompt: '', avatar_sprite: 'architect', driver_type: 'claude', preferred_model: 'auto' },
  { id: 'k_lin', name: 'Lin', title: 'Backend', persona_prompt: '', avatar_sprite: 'engineer', driver_type: 'claude', preferred_model: 'auto' },
];
const CHANNELS = [{ id: 'c_eng', name: 'engineering', lead_kwant_id: 'k_ada', member_kwants: ['k_ada'], routing_mode: 'lead' }];

const SCHEDULE = { type: 'weekly', time: '09:00', weekdays: ['mon'], tz: 'Europe/Berlin' };
const offer = (over = {}) => ({
  id: 'sg_1',
  space_id: 'sp_1',
  cluster_key: 'weekly-report',
  kwant_id: 'k_ada',
  name: 'Weekly status report',
  instructions: 'Write the weekly status report.',
  schedule: SCHEDULE,
  schedule_text: 'weekly on Mon at 09:00 Europe/Berlin',
  first_run_at: '2026-11-02T08:00:00Z',
  first_run_text: 'Mon 2 Nov at 09:00',
  delivery: { channel_id: 'c_eng' },
  task_class: 'report',
  acceptance: 'The report is in reports/',
  checks: [
    { kind: 'file_exists', spec: { path: 'reports/week.md' }, required: true },
    { kind: 'command', spec: { command: 'curl -H "Authorization: ••••••••" https://api.example.com' }, required: false },
  ],
  left_out_checks: [{ kind: 'person', spec: {}, reason: 'needs a person or a reviewer' }],
  chain: [],
  source_task_ids: ['t1', 't2', 't3'],
  sources: [
    { task_id: 't1', title: 'Weekly status report', local_date: '2026-10-12', local_time: '09:05', state: 'verified' },
    { task_id: 't2', title: 'Weekly status report', local_date: '2026-10-19', local_time: '09:10', state: 'verified' },
    { task_id: 't3', title: 'Status report for the week', local_date: '2026-10-26', local_time: '09:02', state: 'verified' },
  ],
  time_spread: '09:02–09:10',
  evidence: { pattern: 'weekly', matched: 3, expected: 3, coverage: 1, precision: 1, verified: 3, settled: 3, total: 3, time_spread: '09:02–09:10', time_varies: false, last_ask_at: '2026-10-26T08:02:00Z' },
  permissions: {
    flight_school: true,
    tier: 'sandbox',
    tier_from: 'class',
    tier_label: 'Sandbox: commands that can change things ask you first',
    approval_mode: 'smart',
    payments_ask: true,
    past: { external: 0, workspace: 3, needed_ok: 1 },
  },
  status: 'offered',
  routine_id: null,
  cooldown_until: null,
  offered_at: '2026-10-31T10:00:00Z',
  decided_at: null,
  ...over,
});
const STATS = { offered_30d: 2, accepted_30d: 1, dismissed_30d: 0, retained: 1, dropped: 0 };
const listing = (over = {}) => ({ setting: 'on', daily_cap: 2, blocked_by: [], last_scan_at: '2026-10-31T10:00:00Z', suggestions: [offer()], stats: STATS, ...over });

const json = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

/** A stub server: `routes` answers by "METHOD path" prefix; every call is recorded. */
function stubServer(routes) {
  const calls = [];
  g.fetch = async (url, init = {}) => {
    const u = String(url);
    const method = init.method ?? 'GET';
    const headers = Object.fromEntries(Object.entries(init.headers ?? {}).map(([k, v]) => [k.toLowerCase(), v]));
    calls.push({ method, url: u, headers, body: init.body === undefined ? undefined : JSON.parse(init.body) });
    for (const [key, answer] of Object.entries(routes)) {
      const [rm, rp] = key.split(' ');
      if (rm !== method || !u.startsWith(rp)) continue;
      if (typeof answer === 'function') return answer({ url: u, headers, init, calls });
      return answer instanceof Response ? answer.clone() : json(answer);
    }
    return json([]);
  };
  g.window.fetch = g.fetch;
  return calls;
}

function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  return {
    host,
    render: (e) => act(async () => root.render(e)),
    done: () => {
      root.unmount();
      host.remove();
    },
    button: (label) => [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === label || b.getAttribute('aria-label') === label),
    text: () => host.textContent.replace(/\s+/g, ' '),
  };
}

const setValue = (el, v) => {
  const proto = el.tagName === 'TEXTAREA' ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, v);
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
};

async function renderSection(routes, extra = {}) {
  const calls = stubServer(routes);
  const v = mount();
  const accepted = { n: 0 };
  await v.render(
    React.createElement(m.SuggestedRoutines, { kwants: KWANTS, channels: CHANNELS, onAccepted: () => accepted.n++, onOpenSettings: () => {}, ...extra }),
  );
  await settle();
  return { ...v, calls, accepted };
}

// ---------------------------------------------------------------------------
// API client

test('accept sends the Idempotency-Key header, and a body only when something changed', async () => {
  const calls = stubServer({ 'POST /api/automation-discovery/sg_1/accept': json({ suggestion: offer({ status: 'accepted' }), routine: { id: 'r1', name: 'x' }, first_run_text: 'Mon 2 Nov at 09:00' }, 201) });
  await m.discoveryApi.accept('sg_1', 'key-1');
  await m.discoveryApi.accept('sg_1', 'key-2', {});
  await m.discoveryApi.accept('sg_1', 'key-3', { name: 'Renamed' });
  assert.deepEqual(
    calls.map((c) => [c.method, c.url, c.headers['idempotency-key'], c.body]),
    [
      ['POST', '/api/automation-discovery/sg_1/accept', 'key-1', undefined],
      ['POST', '/api/automation-discovery/sg_1/accept', 'key-2', undefined],
      ['POST', '/api/automation-discovery/sg_1/accept', 'key-3', { name: 'Renamed' }],
    ],
  );
});

test('a 409 becomes a DiscoveryError with what already happened, in plain words', async () => {
  stubServer({ 'POST /api/automation-discovery/sg_1/dismiss': () => json({ error: 'this suggestion was already accepted', status: 'accepted', routine_id: 'r9' }, 409) });
  await assert.rejects(m.discoveryApi.dismiss('sg_1'), (e) => {
    assert.ok(e instanceof m.DiscoveryError);
    assert.equal(e.status, 409);
    assert.deepEqual(e.current, { status: 'accepted', routine_id: 'r9' });
    assert.equal(e.message, 'Already added. It’s in your routines below.');
    return true;
  });
  for (const s of ['dismissed', 'expired', 'superseded', 'weird']) assert.ok(m.alreadyHandledText(s).length > 10, s);
});

test('a dropped connection is a status-0 error, not a crash', async () => {
  g.fetch = async () => {
    throw new TypeError('Failed to fetch');
  };
  g.window.fetch = g.fetch;
  await assert.rejects(m.discoveryApi.list(), (e) => e instanceof m.DiscoveryError && e.status === 0 && /connection/.test(e.message));
});

test('AcceptIntent keeps one key across retries that may have gone through, and starts a new one after a refusal', () => {
  let n = 0;
  const intent = new m.AcceptIntent(() => `k${++n}`);
  assert.equal(intent.current(), 'k1');
  assert.equal(intent.current(), 'k1', 'a double press sends the same key');
  intent.settle(new m.DiscoveryError('offline', 0));
  assert.equal(intent.current(), 'k1', 'after a dropped connection the retry replays');
  intent.settle(new m.DiscoveryError('boom', 502));
  assert.equal(intent.current(), 'k1', 'after a server error the retry replays');
  intent.settle(new TypeError('x'));
  assert.equal(intent.current(), 'k1');
  intent.settle(new m.DiscoveryError('bad schedule', 400));
  assert.equal(intent.current(), 'k2', 'a refusal the person fixes is a new intent');
  intent.settle(new m.DiscoveryError('bad index', 422));
  assert.equal(intent.current(), 'k3');
});

test('acceptEdits sends only what changed; checks can only be dropped', () => {
  const s = offer();
  const body = { kwant_id: 'k_ada', channel_id: 'c_eng', name: 'Weekly status report', instructions: 'Write the weekly status report.', schedule: { tz: 'Europe/Berlin', weekdays: ['mon'], time: '09:00', type: 'weekly' }, enabled: true };
  assert.deepEqual(m.acceptEdits(s, body, false, [true, true]), {}, 'the same schedule in another key order is no change');
  assert.deepEqual(m.acceptEdits(s, { ...body, name: ' Monday report ' }, false, [true, true]), { name: 'Monday report' });
  assert.deepEqual(m.acceptEdits(s, { ...body, schedule: { ...SCHEDULE, time: '08:30' } }, false, [true, true]), { schedule: { ...SCHEDULE, time: '08:30' } });
  assert.deepEqual(m.acceptEdits(s, { ...body, channel_id: null }, true, [true, true]), { channel_id: null }, 'moving to direct chat sends null');
  assert.deepEqual(m.acceptEdits(s, { ...body, kwant_id: 'k_lin' }, false, [true, true]), { kwant_id: 'k_lin' });
  assert.deepEqual(m.acceptEdits(s, body, false, [false, true]), { keep_checks: [1] });
  assert.deepEqual(m.acceptEdits(s, body, false, [false, false]), { keep_checks: [] });
  assert.ok(!('checks' in m.acceptEdits(s, body, false, [false, true])), 'a masked spec is never sent back');
});

// ---------------------------------------------------------------------------
// Words

test('every blocked reason and every skip reason has plain words', () => {
  for (const b of ['off', 'board_off', 'no_timezone']) {
    const t = m.blockText(b);
    assert.ok(t.title && t.fix && /Settings/.test(t.fix), b);
  }
  const reasons = ['too_short', 'not_regular', 'stopped', 'not_verified', 'mixed_owners', 'bot_unavailable', 'covered', 'cooling_off', 'already_offered', 'in_chain', 'capped'];
  for (const r of reasons) {
    const t = m.skipText(r);
    assert.ok(!t.includes('_') && t !== r && /^[A-Z]/.test(t), `${r} → ${t}`);
  }
  assert.equal(m.skipText('something_new'), 'something new', 'an unknown reason still reads as words');
});

test('the habit, permissions and stats lines read naturally', () => {
  assert.equal(m.habitLine(offer().evidence), 'You asked for this 3 times, every week, usually between 09:02 and 09:10.');
  assert.equal(m.habitLine({ ...offer().evidence, pattern: 'daily', time_spread: '18:00', matched: 7, time_varies: true }), 'You asked for this 7 times, every day, usually around 18:00 (the time varies).');
  assert.equal(m.pastText({ external: 0, workspace: 3, needed_ok: 1 }), 'In past runs: 3 workspace changes; needed your OK once.');
  assert.equal(m.pastText({ external: 0, workspace: 0, needed_ok: 0 }), 'In past runs it changed nothing outside its chat, and never needed your OK.');
  assert.equal(m.approvalText('smart'), 'Asks you only when an action is risky');
  assert.equal(m.statsLine(STATS), 'Last 30 days: 2 suggested, 1 added, 0 dismissed. Still on 14 days after adding: 1 of 1.');
  assert.equal(m.statsLine({ ...STATS, retained: 0 }), 'Last 30 days: 2 suggested, 1 added, 0 dismissed.');
  assert.equal(m.checkLine(offer().checks[0]), 'File exists: reports/week.md');
  assert.match(m.checkLine(offer().checks[1]), /^Command: curl -H "Authorization: ••••••••"/);
  assert.match(m.leftOutText('needs a person or a reviewer'), /can’t run on a schedule/);
});

test('settings parse: null on an older server, the cap kept within 0–10', () => {
  assert.equal(m.parseDiscoverySettings({}), null);
  assert.deepEqual(m.parseDiscoverySettings({ automation_discovery: 'off', automation_discovery_daily_cap: 2 }), { automation_discovery: 'off', automation_discovery_daily_cap: 2 });
  assert.deepEqual(m.parseDiscoverySettings({ automation_discovery: 'on', automation_discovery_daily_cap: 40 }), { automation_discovery: 'on', automation_discovery_daily_cap: 10 });
  assert.deepEqual(m.parseDiscoverySettings({ automation_discovery: 'on' }), { automation_discovery: 'on', automation_discovery_daily_cap: 2 });
  assert.deepEqual(m.DISCOVERY_CAPS, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
});

test('the automation_suggested notification: a toggle where the server has it, a toast, never a Needs-you item', () => {
  const ids = (d) => m.notifyKindsFor(false, false, d).map((k) => k.id);
  assert.ok(ids(true).includes('automation_suggested'));
  assert.ok(!ids(false).includes('automation_suggested'), 'not offered to a server that would refuse it');
  assert.equal(m.notifyKindsFor(false, false, true).find((k) => k.id === 'automation_suggested').label, 'A routine was suggested');
  assert.deepEqual(m.notifyOnToSave(['turn_finished', 'automation_suggested'], false, false, false), ['turn_finished']);
  assert.deepEqual(m.notifyOnToSave(['turn_finished', 'automation_suggested'], false, false, true), ['turn_finished', 'automation_suggested']);
  assert.ok(m.TOAST_KINDS.includes('automation_suggested'));
  assert.equal(m.shouldShowOsNotification({ enabled: true, permission: 'granted', hidden: true, kind: 'automation_suggested' }), true);
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /n\.kind === 'automation_suggested'\) setIsRoutinesOpen\(true\)/, 'a tapped notification opens Routines');
  assert.doesNotMatch(readFileSync(new URL('../src/missions.ts', import.meta.url), 'utf8'), /automation_suggested/, 'not on the Needs-you list');
  const settings = readFileSync(new URL('../src/components/SettingsModal.tsx', import.meta.url), 'utf8');
  assert.match(settings, /notifyKindsFor\(appUpdates, !!proactive, !!discovery[,)]/);
  assert.match(settings, /notifyOnToSave\(notifyOn, appUpdates, !!proactive, !!discovery[,)]/);
  assert.match(settings, /\.\.\.\(discovery \?\? \{\}\)/, 'the switch and cap are saved');
});

// ---------------------------------------------------------------------------
// The section in a DOM

test('an offer card shows the name, bot, rhythm, first run, why, permissions and checks', async () => {
  const v = await renderSection({ 'GET /api/automation-discovery': listing() });
  const card = v.host.querySelector('[data-testid="suggestion-card"]');
  assert.ok(card);
  const t = card.textContent.replace(/\s+/g, ' ');
  for (const want of ['Weekly status report', 'Ada', '#engineering', 'First run: Mon 2 Nov at 09:00', 'You asked for this 3 times, every week', '3 of 3 finished tasks passed their checks', 'Sandbox: commands that can change things ask you first', 'Payments always ask you', 'File exists: reports/week.md', '(optional)', 'Left out:', 'can’t run on a schedule']) {
    assert.ok(t.includes(want), `card shows “${want}”: ${t}`);
  }
  assert.equal(card.getAttribute('aria-labelledby'), card.querySelector('h4').id, 'the card is named by its title');
  assert.ok(v.button('Accept Weekly status report: review and add it'));
  assert.ok(v.button('Dismiss Weekly status report'));
  // Why: the sources, with their days
  await act(async () => v.button('Why: the 3 tasks it learned from').click());
  assert.match(v.text(), /Mon, Oct 12 09:05 ?Weekly status report/);
  assert.match(v.text(), /Last 30 days: 2 suggested, 1 added/);
  v.done();
});

test('Accept → edit → Add routine sends one key, only the edits, and shows the first run it returned', async () => {
  let answered = 0;
  const v = await renderSection({
    'GET /api/automation-discovery': () => json(listing({ suggestions: answered ? [] : [offer()] })),
    'POST /api/automation-discovery/sg_1/accept': ({ init }) => {
      answered++;
      const body = JSON.parse(init.body);
      return json({ suggestion: offer({ status: 'accepted' }), routine: { id: 'r1', name: body.name, origin: 'discovery' }, first_run_text: 'Mon 2 Nov at 09:00' }, 201);
    },
  });
  await act(async () => v.button('Accept Weekly status report: review and add it').click());
  const form = v.host.querySelector('form[aria-label="Accept Weekly status report"]');
  assert.ok(form, 'Accept opens the edit step');
  assert.equal(form.querySelector('input[type="checkbox"]:not([role])')?.closest('label').textContent.includes('Enabled'), false, 'no Enabled box: it starts on');
  // Masked specs are text, never an input that could be edited back into a secret
  for (const input of form.querySelectorAll('input, textarea')) assert.ok(!String(input.value).includes('••••'), 'no masked spec in an input');
  const name = form.querySelector('input[placeholder="Morning digest"]');
  assert.equal(name.value, 'Weekly status report');
  await act(async () => setValue(name, 'Monday report'));
  const keepBoxes = [...form.querySelectorAll('fieldset input[type="checkbox"]')];
  assert.equal(keepBoxes.length, 2);
  await act(async () => keepBoxes[1].click());
  await act(async () => [...form.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Add routine').click());
  const posts = v.calls.filter((c) => c.method === 'POST');
  assert.equal(posts.length, 1);
  assert.ok(posts[0].headers['idempotency-key'], 'sent with an Idempotency-Key');
  assert.deepEqual(posts[0].body, { name: 'Monday report', keep_checks: [0] });
  assert.match(v.text(), /Added “Monday report” for Ada\. First run: Mon 2 Nov at 09:00\./);
  assert.equal(v.accepted.n, 1, 'the routine list reloads');
  assert.equal(v.host.querySelectorAll('[data-testid="suggestion-card"]').length, 0);
  assert.equal(document.activeElement?.textContent.includes('Added'), true, 'focus moves to the result line');
  v.done();
});

test('a failed Accept retried after a dropped connection reuses the key; a fixed 400 gets a new one', async () => {
  let n = 0;
  const v = await renderSection({
    'GET /api/automation-discovery': listing(),
    'POST /api/automation-discovery/sg_1/accept': () => {
      n++;
      if (n === 1) throw new TypeError('Failed to fetch');
      if (n === 2) return json({ error: 'that schedule is in the past' }, 400);
      return json({ suggestion: offer({ status: 'accepted' }), routine: { id: 'r1', name: 'Weekly status report' }, first_run_text: 'Mon 2 Nov at 09:00' }, 201);
    },
  });
  await act(async () => v.button('Accept Weekly status report: review and add it').click());
  const add = () => [...v.host.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Add routine');
  await act(async () => add().click());
  assert.match(v.host.querySelector('[role="alert"]').textContent, /Couldn’t reach the server/);
  await act(async () => add().click());
  assert.match(v.host.querySelector('[role="alert"]').textContent, /That schedule is in the past\./);
  await act(async () => add().click());
  const keys = v.calls.filter((c) => c.method === 'POST').map((c) => c.headers['idempotency-key']);
  assert.equal(keys.length, 3);
  assert.equal(keys[0], keys[1], 'the retry after a dropped connection replays the same intent');
  assert.notEqual(keys[2], keys[1], 'after a refusal, a new intent');
  assert.match(v.text(), /Added “Weekly status report”/);
  v.done();
});

test('Dismiss says the suggestion is hidden for 30 days', async () => {
  let dismissed = false;
  const v = await renderSection({
    'GET /api/automation-discovery': () => json(listing({ suggestions: dismissed ? [] : [offer()] })),
    'POST /api/automation-discovery/sg_1/dismiss': () => {
      dismissed = true;
      return json({ suggestion: offer({ status: 'dismissed', cooldown_until: '2026-11-30T10:00:00Z' }) });
    },
  });
  await act(async () => v.button('Dismiss Weekly status report').click());
  assert.match(v.text(), /Dismissed “Weekly status report”\. It won’t be suggested again for 30 days \(until Nov 30\)\./);
  assert.equal(v.host.querySelectorAll('[data-testid="suggestion-card"]').length, 0);
  assert.equal(v.accepted.n, 0);
  v.done();
});

test('a 409 refreshes the list and says it was already handled', async () => {
  let gets = 0;
  const v = await renderSection({
    'GET /api/automation-discovery': () => json(listing({ suggestions: gets++ ? [] : [offer()] })),
    'POST /api/automation-discovery/sg_1/dismiss': () => json({ error: 'this suggestion was already accepted', status: 'accepted', routine_id: 'r1' }, 409),
  });
  await act(async () => v.button('Dismiss Weekly status report').click());
  assert.match(v.text(), /Already added\. It’s in your routines below\./);
  assert.ok(gets >= 2, 'the list was fetched again');
  assert.equal(v.host.querySelectorAll('[data-testid="suggestion-card"]').length, 0);
  assert.equal(v.accepted.n, 1, 'the routine list reloads to show the routine someone else added');
  assert.equal(v.host.querySelector('[role="alert"]'), null, 'not shown as an error');
  v.done();
});

test('a 409 on Accept in the edit step does the same', async () => {
  const v = await renderSection({
    'GET /api/automation-discovery': listing(),
    'POST /api/automation-discovery/sg_1/accept': () => json({ error: 'this suggestion was already dismissed', status: 'dismissed', routine_id: null }, 409),
  });
  await act(async () => v.button('Accept Weekly status report: review and add it').click());
  await act(async () => [...v.host.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Add routine').click());
  assert.match(v.text(), /Already dismissed\./);
  assert.equal(v.accepted.n, 0);
  v.done();
});

test('blocked: off is one quiet line; board off and no time zone each show their fix', async () => {
  let v = await renderSection({ 'GET /api/automation-discovery': listing({ setting: 'off', blocked_by: ['off'], suggestions: [], stats: { ...STATS, offered_30d: 0 } }) });
  let box = v.host.querySelector('[data-testid="discovery-blocked"]');
  assert.match(box.textContent, /Suggestions are off\..*Turn on “Suggest routines from repeated tasks” in Settings/);
  assert.ok(v.button('Open Settings'));
  assert.equal(v.button('Scan now'), undefined, 'no scan while blocked');
  v.done();

  v = await renderSection({ 'GET /api/automation-discovery': listing({ blocked_by: ['board_off', 'no_timezone'], suggestions: [] }) });
  box = v.host.querySelector('[data-testid="discovery-blocked"]');
  assert.equal(box.getAttribute('role'), 'status');
  assert.match(box.textContent, /the board is off\..*Turn on “Show the Mission board” in Settings/);
  assert.match(box.textContent, /time zone isn’t set.*Set your time zone in Settings, under Where you are/);
  assert.ok(v.button('Check again'));
  v.done();
});

test('empty, loading and error states', async () => {
  let v = await renderSection({ 'GET /api/automation-discovery': listing({ suggestions: [] }) });
  assert.match(v.host.querySelector('[data-testid="discovery-empty"]').textContent, /No suggestions right now\..*at most 2 new suggestions a day/);
  v.done();

  g.fetch = () => new Promise(() => {});
  g.window.fetch = g.fetch;
  v = mount();
  await v.render(React.createElement(m.SuggestedRoutines, { kwants: KWANTS, channels: CHANNELS, onAccepted: () => {} }));
  assert.match(v.host.querySelector('[role="status"]').textContent, /Checking for suggestions/);
  v.done();

  v = await renderSection({ 'GET /api/automation-discovery': () => json({ error: 'database is locked' }, 500) });
  assert.match(v.host.querySelector('[role="alert"]').textContent, /Database is locked\./);
  assert.ok(v.button('Retry'));
  v.done();

  v = await renderSection({ 'GET /api/automation-discovery': () => new Response('Not Found', { status: 404 }) });
  assert.equal(v.host.querySelector('[data-testid="suggested-routines"]'), null, 'a server without it shows nothing');
  v.done();
});

test('Scan now and Preview show what was found and why the rest was skipped', async () => {
  const skipped = [
    { cluster_key: 'a', title: 'Fix the build', asks: 2, reason: 'too_short' },
    { cluster_key: 'b', title: 'Daily standup notes', asks: 9, reason: 'covered' },
  ];
  const v = await renderSection({
    'GET /api/automation-discovery': listing({ suggestions: [] }),
    'POST /api/automation-discovery/scan?preview=1': json({ blocked_by: null, preview: true, timezone: 'Europe/Berlin', offered: [{ cluster_key: 'c', kwant_id: 'k_ada', name: 'Invoice run', schedule: { type: 'monthly', time: '10:00', days: [1], tz: 'Europe/Berlin' }, first_run_text: 'Sun 1 Nov at 10:00', evidence: { ...offer().evidence, pattern: 'monthly', matched: 4 } }], skipped }),
    'POST /api/automation-discovery/scan': json({ blocked_by: null, preview: false, timezone: 'Europe/Berlin', offered: [], skipped, expired: [], superseded: [] }),
  });
  await act(async () => v.button('Preview').click());
  let report = v.host.querySelector('[data-testid="scan-report"]');
  assert.match(report.textContent, /Preview: 1 routine would be suggested\. Nothing was saved\./);
  assert.match(report.textContent, /Invoice run/);
  await act(async () => v.button('Not suggested (2)').click());
  report = v.host.querySelector('[data-testid="scan-report"]');
  assert.match(report.textContent, /Fix the build \(asked 2 times\): Not enough asks yet to see a rhythm\./);
  assert.match(report.textContent, /Daily standup notes \(asked 9 times\): A routine already does this\./);

  await act(async () => v.button('Scan now').click());
  assert.match(v.host.querySelector('[data-testid="scan-report"]').textContent, /Looked just now: nothing new to suggest\./);
  assert.deepEqual(
    v.calls.filter((c) => c.method === 'POST').map((c) => c.url),
    ['/api/automation-discovery/scan?preview=1', '/api/automation-discovery/scan'],
  );
  v.done();
});

test('a routine added from a suggestion carries the “Suggested for you” badge', async () => {
  stubServer({});
  const base = { id: 'r1', kwant_id: 'k_ada', channel_id: null, name: 'Weekly status report', instructions: 'x', schedule: SCHEDULE, enabled: true, next_run_at: null, last_run_at: null, last_status: null, created_at: '2026-10-31T10:00:00Z' };
  const v = mount();
  await v.render(React.createElement(m.RoutineRow, { routine: { ...base, origin: 'discovery', suggestion_id: 'sg_1' }, placeLabel: 'direct chat', tick: 0, onEdit() {}, onChanged() {} }));
  assert.match(v.text(), /Suggested for you/);
  await v.render(React.createElement(m.RoutineRow, { routine: { ...base, origin: 'person' }, placeLabel: 'direct chat', tick: 0, onEdit() {}, onChanged() {} }));
  assert.doesNotMatch(v.text(), /Suggested for you/);
  v.done();
});
