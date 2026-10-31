// Mission board, live: who is busy right now (chats, rooms and routines too, not just tasks), lanes by
// team or bot, repeated routine runs folded into one card, one-tap Review actions, and Verified
// showing only the last day unless the person asks for older.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const store = new Map();
globalThis.localStorage = { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, String(v)), removeItem: (k) => store.delete(k) };

const m = await loadBundled(`export * from './src/missions';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { MissionBoardView, TaskCard, Board, LiveStrip } from './src/components/MissionBoardView';
  const r = (c, p) => renderToStaticMarkup(React.createElement(c, p));
  export const view = (p) => r(MissionBoardView, p);
  export const board = (p) => r(Board, p);
  export const live = (p) => r(LiveStrip, p);
  export const card = (p) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(TaskCard, p)));
`);

const NOW = Date.parse('2026-10-31T16:00:00Z');
const H = 3_600_000;
const iso = (ms) => new Date(ms).toISOString();
const noop = () => {};
const names = { k_ada: 'Ada', k_lin: 'Lin', k_pixel: 'Pixel', k_tw: 'TechWriter', k_solo: 'Solo' };
const teams = [
  { id: 'tm_ship', name: 'Ship Software Safely', lead_kwant_id: 'k_ada', member_kwants: ['k_lin', 'k_pixel'] },
  { id: 'tm_docs', name: 'Docs', lead_kwant_id: 'k_tw', member_kwants: ['k_lin'] },
];

let n = 0;
const task = (over = {}) => ({
  id: `t${++n}`, space_id: 's1', mission_id: null, mission_title: null, title: 'Write the pricing copy', acceptance: '',
  state: 'ready', priority: 1, owner_kwant_id: 'k_lin', owner_name: 'Lin', suggested_kwant_id: null, budget_tokens: 0,
  spent_tokens: 0, attempts: 0, max_attempts: 3, blocked_reason: null, source_kind: 'person', source_id: null,
  channel_id: null, direct_kwant_id: 'k_lin', thread_id: null, created_by: 'person', created_at: iso(NOW - 2 * H),
  updated_at: iso(NOW - H), finished_at: null, depends_on: [], waiting_on: [], claim: null, needs_you: [],
  checks_summary: { status: 'not_checked', passed: 0, failed: 0, total: 0 },
  ...over,
});
const routineRun = (hoursAgo, over = {}) =>
  task({ title: 'Routine: Morning overview', source_kind: 'routine', source_id: `run_${hoursAgo}`, state: 'review', owner_kwant_id: 'k_tw', owner_name: 'TechWriter', updated_at: iso(NOW - hoursAgo * H), finished_at: iso(NOW - hoursAgo * H), ...over });

// ---------- Folding repeated runs ----------

test('repeated runs of one routine fold into one card, newest on top, older ones newest first', () => {
  const runs = [routineRun(50), routineRun(2), routineRun(26), routineRun(74)];
  const other = task({ state: 'review', title: 'Fix the login page' });
  const items = m.collapseRepeats([...runs, other]);
  assert.equal(items.length, 2);
  const folded = items.find((i) => i.task.title.startsWith('Routine'));
  assert.equal(folded.task.source_id, 'run_2', 'the newest run is the card');
  assert.deepEqual(folded.older.map((t) => t.source_id), ['run_26', 'run_50', 'run_74']);
  assert.deepEqual(items.find((i) => i.task === other).older, []);
});

test('a routine card folds by routine name and bot, not by its run id (each run has its own)', () => {
  const a = routineRun(1);
  const b = routineRun(3);
  assert.notEqual(a.source_id, b.source_id);
  assert.equal(m.repeatKey(a), m.repeatKey(b));
  assert.notEqual(m.repeatKey(a), m.repeatKey(routineRun(1, { owner_kwant_id: 'k_ada' })), 'another bot’s routine of the same name stays apart');
  assert.notEqual(m.repeatKey(a), m.repeatKey(routineRun(1, { title: 'Routine: Evening wrap-up' })));
});

test('work the person asked for never folds, even with the same title', () => {
  const items = m.collapseRepeats([task({ state: 'review' }), task({ state: 'review' })]);
  assert.equal(items.length, 2);
  assert.equal(m.repeatKey(task()), null);
  assert.equal(m.repeatKey(task({ source_kind: 'delegation' })), null);
});

test('routine runs fold by bot and title with case and spaces ignored', () => {
  const a = routineRun(3);
  const b = routineRun(5, { title: ' routine: MORNING overview ' });
  assert.equal(m.collapseRepeats([a, b]).length, 1);
});

test('worked-ahead and loose-end cards never fold, even with identical titles: they can be different work', () => {
  const ahead = (over) => task({ source_kind: 'work_ahead', title: 'Worked ahead: API Error 529', state: 'review', owner_kwant_id: 'k_tw', ...over });
  const loose = (over) => task({ source_kind: 'zombie', title: 'Loose end: reply to Maya', state: 'review', owner_kwant_id: 'k_tw', ...over });
  const cards = [ahead({ source_id: 'w1' }), ahead({ source_id: 'w2' }), loose({ source_id: 'z1' }), loose({ source_id: 'z2' })];
  const items = m.collapseRepeats(cards);
  assert.equal(items.length, 4);
  assert.ok(items.every((i) => i.older.length === 0), 'nothing folded under them, so "Dismiss N older" never covers them');
  for (const c of cards) assert.equal(m.repeatKey(c), null);
  const html = ui.card({ task: cards[0], names, now: NOW, older: m.collapseRepeats(cards)[0].older, onQuick: noop, onDismissMany: noop });
  assert.doesNotMatch(html, /older/i);
});

// ---------- Verified fades out ----------

test('Verified keeps the last 24 hours; older cards wait behind Show older, none deleted', () => {
  const fresh = task({ state: 'verified', finished_at: iso(NOW - 3 * H) });
  const edge = task({ state: 'verified', finished_at: iso(NOW - 24 * H) });
  const old = task({ state: 'verified', finished_at: iso(NOW - 25 * H) });
  const noFinish = task({ state: 'verified', finished_at: null, updated_at: iso(NOW - 48 * H) });
  const { recent, older } = m.splitVerified([fresh, edge, old, noFinish], NOW);
  assert.deepEqual(recent, [fresh, edge]);
  assert.deepEqual(older, [old, noFinish], 'falls back to updated_at');
  assert.equal(recent.length + older.length, 4);
});

test('the Verified header count shows both numbers only when some are hidden', () => {
  assert.equal(m.verifiedCountText(5, 37), '5 · 37 all');
  assert.equal(m.verifiedCountText(4, 4), '4');
});

test('the board renders only recent Verified cards with a Show older link and both counts', () => {
  const fresh = task({ state: 'verified', title: 'Fresh one', finished_at: iso(NOW - H) });
  const olds = [30, 40, 50].map((h) => task({ state: 'verified', title: `Old ${h}`, finished_at: iso(NOW - h * H) }));
  const html = ui.board({ tasks: [fresh, ...olds], names, column: 'verified', now: NOW, onColumn: noop, onOpen: noop });
  assert.match(html, /Fresh one/);
  assert.doesNotMatch(html, /Old 30/);
  assert.match(html, /Show older \(3\)/);
  assert.match(html, /1 · 4 all/);
});

test('with nothing verified today, Verified says so instead of looking empty forever', () => {
  const html = ui.board({ tasks: [task({ state: 'verified', finished_at: iso(NOW - 72 * H) })], names, column: 'verified', now: NOW, onColumn: noop, onOpen: noop });
  assert.match(html, /Nothing verified in the last 24 hours\./);
  assert.match(html, /Show older \(1\)/);
});

// ---------- Old automatic reviews ----------

test('Clear old reviews picks only automatic Review cards older than 7 days', () => {
  const oldRoutine = routineRun(8 * 24);
  const oldAhead = task({ source_kind: 'work_ahead', state: 'failed', updated_at: iso(NOW - 10 * 24 * H) });
  const newRoutine = routineRun(6 * 24);
  const oldPerson = task({ state: 'review', updated_at: iso(NOW - 30 * 24 * H) });
  const oldVerified = routineRun(9 * 24, { state: 'verified' });
  const oldButNeedsYou = routineRun(9 * 24, { needs_you: [{ kind: 'check_approval', text: 'Allow?' }] });
  const picked = m.oldAutoReviews([oldRoutine, oldAhead, newRoutine, oldPerson, oldVerified, oldButNeedsYou], NOW);
  assert.deepEqual(picked, [oldRoutine, oldAhead]);
});

// ---------- Lanes by team or bot ----------

test('a bot belongs to the team it leads first, then the first team it is in', () => {
  const of = m.teamOfBots(teams);
  assert.equal(of.k_ada.id, 'tm_ship');
  assert.equal(of.k_tw.id, 'tm_docs');
  assert.equal(of.k_lin.id, 'tm_ship', 'Lin is in both; the first team wins');
  assert.equal(of.k_solo, undefined);
});

test('lanes by team keep the person’s team order, carry per-column cards and a Needs you count; No team is last', () => {
  const tasks = [
    task({ owner_kwant_id: 'k_solo', state: 'ready' }),
    task({ owner_kwant_id: 'k_tw', state: 'working' }),
    task({ owner_kwant_id: 'k_lin', state: 'needs_you' }),
    task({ owner_kwant_id: 'k_pixel', state: 'review' }),
    task({ owner_kwant_id: null, claim: { kwant_id: 'k_tw', claimed_at: '', expires_at: '' }, state: 'working' }),
    task({ owner_kwant_id: null, state: 'cancelled' }),
  ];
  const lanes = m.groupIntoLanes(tasks, 'team', teams, names);
  assert.deepEqual(lanes.map((l) => l.label), ['Ship Software Safely', 'Docs', 'No team']);
  const ship = lanes[0];
  assert.equal(ship.total, 2);
  assert.equal(ship.needsYou, 1);
  assert.equal(ship.columns.needs_you.length, 1);
  assert.equal(ship.columns.review.length, 1);
  assert.equal(lanes[1].columns.working.length, 2, 'an unowned card goes with the bot holding it');
  assert.equal(lanes[2].key, m.NO_TEAM);
});

test('lanes by bot go alphabetically with No owner last', () => {
  const lanes = m.groupIntoLanes([task({ owner_kwant_id: 'k_pixel' }), task({ owner_kwant_id: null }), task({ owner_kwant_id: 'k_ada' })], 'bot', teams, names);
  assert.deepEqual(lanes.map((l) => l.label), ['Ada', 'Pixel', 'No owner yet']);
});

test('team chips filter cards; none picked shows all; No team picks unteamed cards', () => {
  const a = task({ owner_kwant_id: 'k_ada' });
  const b = task({ owner_kwant_id: 'k_tw' });
  const c = task({ owner_kwant_id: 'k_solo' });
  assert.deepEqual(m.filterByTeams([a, b, c], [], teams), [a, b, c]);
  assert.deepEqual(m.filterByTeams([a, b, c], ['tm_docs'], teams), [b]);
  assert.deepEqual(m.filterByTeams([a, b, c], ['tm_ship', m.NO_TEAM], teams), [a, c]);
});

test('Group by is remembered on this device and falls back to None', () => {
  store.clear();
  assert.equal(m.readGroupBy(), 'none');
  m.saveGroupBy('team');
  assert.equal(m.readGroupBy(), 'team');
  store.set(m.GROUP_BY_KEY, 'nonsense');
  assert.equal(m.readGroupBy(), 'none');
});

test('the board grouped by team renders one row per team with the column headers once', () => {
  const html = ui.board({
    tasks: [task({ owner_kwant_id: 'k_tw', state: 'working' }), task({ owner_kwant_id: 'k_lin', state: 'needs_you' })],
    names, column: 'ready', now: NOW, onColumn: noop, onOpen: noop, groupBy: 'team', teams,
  });
  assert.match(html, /data-board="lanes"/);
  assert.match(html, /data-lane="tm_ship"/);
  assert.match(html, /data-lane="tm_docs"/);
  assert.match(html, /data-lane-needs-you="1"/);
  assert.doesNotMatch(html, /data-board="columns"/);
});

// ---------- Live now ----------

const turn = (over = {}) => ({ turn_id: `u${++n}`, kwant_id: 'k_tw', context_id: 'k_tw', thread_id: null, state: 'running', since: iso(NOW - 5 * 60_000), ...over });

test('a bot chatting (no task) is live: the board no longer says nobody is working', () => {
  const bots = m.liveBots([turn()], { names, teams, tasks: [] });
  assert.equal(bots.length, 1);
  assert.equal(bots[0].name, 'TechWriter');
  assert.equal(bots[0].kind, 'chat');
  assert.equal(bots[0].teamName, 'Docs');
  assert.deepEqual(bots[0].place, { channelId: null, kwantId: 'k_tw', threadId: null });
  assert.equal(m.liveSummary(bots, 0), '1 bot busy · 0 on board tasks, 1 in chats, rooms or routines');
});

test('a live turn says what it is: the task it runs, the room, the thread, or a decide round', () => {
  const working = task({ state: 'working', owner_kwant_id: 'k_tw', title: 'Routine: Morning overview', channel_id: null, direct_kwant_id: 'k_tw', source_kind: 'routine' });
  const ctx = { names, teams, rooms: { c_eng: 'Engineering Room' }, threadTitles: { th1: 'Release notes draft' }, tasks: [working] };
  assert.deepEqual(m.describeTurn(turn(), ctx).what, 'Routine: Morning overview');
  assert.equal(m.describeTurn(turn(), ctx).taskId, working.id);
  const room = m.describeTurn(turn({ kwant_id: 'k_ada', context_id: 'c_eng' }), ctx);
  assert.equal(room.what, 'In Engineering Room');
  assert.deepEqual(room.place, { channelId: 'c_eng', kwantId: null, threadId: null });
  assert.equal(m.describeTurn(turn({ kwant_id: 'k_lin', context_id: 'th1', thread_id: 'th1' }), ctx).what, 'Release notes draft');
  assert.equal(m.describeTurn(turn({ kwant_id: 'k_lin', context_id: 'th9', thread_id: 'th9' }), ctx).what, 'In a side conversation');
  assert.equal(m.describeTurn(turn({ kwant_id: 'k_lin', purpose: 'decide' }), ctx).kind, 'deciding');
});

test('one row per bot: a task turn wins, the rest count as +N; queued turns are not live', () => {
  const working = task({ state: 'working', owner_kwant_id: 'k_lin', thread_id: 'th2', title: 'Fix the flaky test' });
  const bots = m.liveBots(
    [
      turn({ kwant_id: 'k_lin', since: iso(NOW - 30 * 60_000) }),
      turn({ kwant_id: 'k_lin', context_id: 'th2', thread_id: 'th2', since: iso(NOW - 2 * 60_000) }),
      turn({ kwant_id: 'k_ada', state: 'queued' }),
    ],
    { names, teams, tasks: [working] },
  );
  assert.equal(bots.length, 1);
  assert.equal(bots[0].what, 'Fix the flaky test');
  assert.equal(bots[0].more, 1);
  assert.equal(bots[0].since, iso(NOW - 30 * 60_000), 'busy since its first running turn');
});

test('live bots group by team in the person’s order with No team last', () => {
  const bots = m.liveBots([turn({ kwant_id: 'k_solo' }), turn({ kwant_id: 'k_tw' }), turn({ kwant_id: 'k_ada' })], { names, teams });
  assert.deepEqual(m.liveByTeam(bots, teams).map((g) => g.teamName), ['Ship Software Safely', 'Docs', 'No team']);
});

test('elapsed time reads short', () => {
  assert.equal(m.elapsedText(iso(NOW - 45_000), NOW), '45s');
  assert.equal(m.elapsedText(iso(NOW - 12 * 60_000), NOW), '12m');
  assert.equal(m.elapsedText(iso(NOW - 65 * 60_000), NOW), '1h 05m');
  assert.equal(m.elapsedText(iso(NOW - 51 * H), NOW), '2d 3h');
  assert.equal(m.elapsedText(undefined, NOW), '');
});

test('the summary explains idle bots and Working cards so the strip and column agree', () => {
  assert.equal(m.liveSummary([], 0), 'All bots idle');
  assert.match(m.liveSummary([], 2), /^All bots idle · 2 tasks wait in Working/);
  const onTask = { kind: 'task' };
  assert.equal(m.liveSummary([onTask, onTask], 2), '2 bots busy, all on board tasks');
});

test('the Live strip renders each busy bot under its team, with elapsed time; idle shows All bots idle', () => {
  const bots = m.liveBots([turn()], { names, teams });
  const html = ui.live({ bots, teams, workingCards: 0, now: NOW, onOpen: noop });
  assert.match(html, /aria-label="Live now"/);
  assert.match(html, /data-live-team="tm_docs"/);
  assert.match(html, /TechWriter/);
  assert.match(html, /5m/);
  assert.match(ui.live({ bots: [], teams, workingCards: 0, now: NOW }), /All bots idle/);
});

// ---------- Review card actions ----------

test('a Review card carries Mark verified, Send back and Dismiss; a failed one Try again and Dismiss', () => {
  assert.deepEqual(m.quickActionsFor({ state: 'review' }), ['verify', 'reject', 'cancel']);
  assert.deepEqual(m.quickActionsFor({ state: 'failed' }), ['retry', 'cancel']);
  assert.deepEqual(m.quickActionsFor({ state: 'working' }), []);
  const html = ui.card({ task: task({ state: 'review' }), names, now: NOW, onOpen: noop, onQuick: noop });
  assert.match(html, /data-card-action="verify"[^>]*>.*Mark verified/);
  assert.match(html, /data-card-action="reject"/);
  assert.match(html, /data-card-action="cancel"[^>]*>.*Dismiss/);
  assert.doesNotMatch(ui.card({ task: task({ state: 'review' }), names, now: NOW, onOpen: noop }), /data-card-actions/, 'no actions without a handler');
});

test('a Review card waiting on the person (Needs you column) gets no quick actions', () => {
  const t = task({ state: 'review', needs_you: [{ kind: 'check_approval', text: 'Allow the check?' }] });
  assert.doesNotMatch(ui.card({ task: t, names, now: NOW, onOpen: noop, onQuick: noop }), /data-card-actions/);
});

test('a folded card shows ×N and offers to dismiss the older runs together', () => {
  const html = ui.card({ task: routineRun(1), older: [routineRun(25), routineRun(49), routineRun(73)], names, now: NOW, onOpen: noop, onQuick: noop, onDismissMany: noop });
  assert.match(html, /×4/);
  assert.match(html, /Dismiss 3 older/);
});

test('the Review column folds four Morning overview runs into one card on the board', () => {
  const html = ui.board({ tasks: [routineRun(1), routineRun(25), routineRun(49), routineRun(73)], names, column: 'review', now: NOW, onColumn: noop, onOpen: noop, onQuick: noop });
  assert.equal((html.match(/data-repeats="4"/g) ?? []).length, 2, 'one folded card in the phone tab and one on desktop');
});

test('undo text names the card or counts them', () => {
  assert.equal(m.quickDoneText('verify', 1, 'Fix login'), 'Marked “Fix login” verified.');
  assert.equal(m.quickDoneText('cancel', 12), 'Dismissed 12 cards.');
  assert.equal(m.quickDoneText('reject', 1, 'X'), 'Sent “X” back to its bot.');
  assert.ok(m.DEFAULT_SEND_BACK.length > 0, 'the server needs a reason, so an empty note still sends one');
});

// ---------- The page ----------

test('the board view shows Live now, the Group control and Clear old reviews with its count', () => {
  const tasks = [routineRun(9 * 24), routineRun(10 * 24, { title: 'Routine: Weekly digest' }), task({ owner_kwant_id: 'k_ada' })];
  const html = ui.view({
    tasks, missions: [], insights: null, names, missionId: null, column: 'review', loading: false, error: null, now: NOW,
    onMission: noop, onColumn: noop, onOpenTask: noop, onApproveMission: noop, onCancelMission: noop, onRefresh: noop,
    live: m.liveBots([turn()], { names, teams }), teams, groupBy: 'none', onGroupBy: noop, onTeamFilter: noop, onQuick: noop, onClearOld: noop,
  });
  assert.match(html, /Live now/);
  assert.match(html, /data-group-by="none"/);
  assert.match(html, /Clear old reviews \(2\)/);
  assert.match(html, /data-team-chip="tm_docs"/);
  assert.match(html, /data-team-chip="tm_ship"/);
});

test('without live data the view stays as before: no strip, no toolbar', () => {
  const html = ui.view({
    tasks: [task()], missions: [], insights: null, names, missionId: null, column: 'ready', loading: false, error: null, now: NOW,
    onMission: noop, onColumn: noop, onOpenTask: noop, onApproveMission: noop, onCancelMission: noop, onRefresh: noop,
  });
  assert.doesNotMatch(html, /Live now/);
  assert.doesNotMatch(html, /data-board-toolbar/);
});

// ---------- A held action is sent exactly once ----------

const held = (sent, over = {}) => ({ id: 1, tasks: [task()], timer: 7, send: (leaving) => (sent.push(leaving), Promise.resolve()), ...over });

test('a held action is sent once, however many flushes race for it', async () => {
  const sent = [];
  const cleared = [];
  const slot = new m.HeldActionSlot((t) => cleared.push(t));
  slot.hold(held(sent));
  const first = slot.commit(true);
  assert.ok(first);
  await first.sent;
  assert.equal(slot.commit(true), null, 'pagehide after visibilitychange finds nothing');
  assert.equal(slot.commit(), null, 'the Undo timer firing late finds nothing');
  assert.equal(slot.take(), null, 'Undo after it went finds nothing');
  assert.deepEqual(sent, [true]);
  assert.deepEqual(cleared, [7], 'its timer is stopped');
});

test('Undo drops the held action: nothing is sent', () => {
  const sent = [];
  const slot = new m.HeldActionSlot(noop);
  slot.hold(held(sent));
  assert.equal(slot.take()?.id, 1);
  assert.equal(slot.commit(true), null);
  assert.deepEqual(sent, []);
});

const fakePage = () => {
  const win = new EventTarget();
  const doc = new EventTarget();
  doc.visibilityState = 'visible';
  return { win, doc };
};

test('closing the page sends the held action once, with keepalive', () => {
  const sent = [];
  const slot = new m.HeldActionSlot(noop);
  const { win, doc } = fakePage();
  const stop = m.flushOnPageLeave(() => slot.commit(true), win, doc);
  slot.hold(held(sent));
  // A phone closing a tab: hidden first, then pagehide.
  doc.visibilityState = 'hidden';
  doc.dispatchEvent(new Event('visibilitychange'));
  win.dispatchEvent(new Event('pagehide'));
  assert.deepEqual(sent, [true]);
  stop();
});

test('pagehide alone sends it; becoming visible again does not', () => {
  const sent = [];
  const slot = new m.HeldActionSlot(noop);
  const { win, doc } = fakePage();
  const stop = m.flushOnPageLeave(() => slot.commit(true), win, doc);
  slot.hold(held(sent));
  doc.dispatchEvent(new Event('visibilitychange'));
  assert.deepEqual(sent, [], 'still visible: keep waiting');
  win.dispatchEvent(new Event('pagehide'));
  assert.deepEqual(sent, [true]);
  stop();
  slot.hold(held(sent, { id: 2 }));
  win.dispatchEvent(new Event('pagehide'));
  assert.deepEqual(sent, [true], 'after cleanup the page no longer flushes');
});

test('card actions can ask for keepalive, so they finish after the page closes', async () => {
  const seen = [];
  const real = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    seen.push({ url, keepalive: init?.keepalive === true });
    return new Response(JSON.stringify(task({ state: 'verified' })), { status: 200 });
  };
  try {
    await m.missionsApi.act('t1', 'verify', undefined, true);
    await m.missionsApi.act('t1', 'cancel', undefined, true);
    await m.missionsApi.act('t1', 'reject', 'nope');
  } finally {
    globalThis.fetch = real;
  }
  assert.deepEqual(seen, [
    { url: '/api/tasks/t1/verify', keepalive: true },
    { url: '/api/tasks/t1/cancel', keepalive: true },
    { url: '/api/tasks/t1/reject', keepalive: false },
  ]);
});
