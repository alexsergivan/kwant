// 0.9.0 H5/H1 follow-ups (Pixel): S1 "Needs you" oldest ask first whatever its kind (qa-494.md S1);
// N2 H4 `stuck` reaches rank 2 and rank 5 (working) runs longest first; N1 the Dev desk repo rollup and order;
// N3 the held-send line offers Try again after `stalled_gave_up` (qa-499.md N3).
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test, { afterEach, beforeEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'HTMLSelectElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'DOMParser']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });

const m = await loadBundled(`
  export { needsYouRows, attentionRows } from './src/chatsTab';
  export * from './src/h4Status';
  export { HeldLine } from './src/components/HeldLine';
  export { rowRank, repoRollup, repoRollupParts, sortRepos, sortWorktrees, worktreeStore } from './src/dev/worktrees';
  export * as fx from './src/dev/worktreeFixtures';
  export { WorktreesPanel } from './src/components/dev/WorktreesTab';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
`);
const { fx } = m;
const h = m.createElement;
const NOW = Date.parse('2026-11-10T12:00:00Z');
const at = (min) => new Date(NOW - min * 60_000).toISOString();
const bot = (id, name) => ({ id, name, title: 'Role', persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '' });
const kwants = [bot('lin', 'Lin'), bot('pix', 'Pixel'), bot('jun', 'Juno'), bot('iri', 'Iris'), bot('rig', 'Rigel'), bot('veg', 'Vega')];
const all = new Set(kwants.map((k) => k.id));
const item = (kind, kwant_id, over = {}) => ({ kind, id: `${kind}-${kwant_id}-${over.timestamp ?? ''}`, kwant_id, kwant_name: kwant_id, title: '', summary: '',
  status: 'waiting', timestamp: at(5), link: { thread_id: null, channel_id: null, direct_kwant_id: kwant_id }, ...over });
const st = (kwant_id, status, over = {}) => ({ kwant_id, status, reason_code: '', held: [], ...over });
const order = (rs) => rs.map((r) => `${r.rank}:${r.kwant.name}`);

// ---------------------------------------------------------------------------------------------------
// S1

test('S1: "Needs you" puts the oldest ask first, whatever its kind', () => {
  // /api/away lists approvals first; Juno's question is older than Pixel's approval.
  const waiting = [item('approval', 'pix', { timestamp: at(2) }), item('question', 'jun', { timestamp: at(30) })];
  const rows = m.needsYouRows(waiting, {}, all);
  assert.deepEqual(rows.map((r) => r.kwant_id), ['jun', 'pix']);
  const att = m.attentionRows({ kwants, needs: rows, failed: [], finished: [], busy: {}, status: {}, now: NOW });
  assert.deepEqual(order(att).slice(0, 2), ['1:Juno', '1:Pixel']);
});

test('S1: one bot with two asks keeps the strongest line and waits since its oldest ask', () => {
  const waiting = [item('approval', 'lin', { timestamp: at(3), title: 'Run the migration' }), item('question', 'lin', { timestamp: at(40) }), item('question', 'pix', { timestamp: at(20) })];
  const rows = m.needsYouRows(waiting, {}, all);
  assert.deepEqual(rows.map((r) => r.kwant_id), ['lin', 'pix']);
  assert.equal(rows[0].at, NOW - 40 * 60_000);
  assert.match(rows[0].line, /OK|approv|Run the migration/i, 'the approval line wins over the question line');
});

test('S1: an ask with no time (a browser step) goes last', () => {
  const rows = m.needsYouRows([item('question', 'pix', { timestamp: at(1) })], { jun: 1 }, all);
  assert.deepEqual(rows.map((r) => r.kwant_id), ['pix', 'jun']);
  assert.equal(rows[1].line, 'Needs you in the browser');
});

// ---------------------------------------------------------------------------------------------------
// N2

test('N2: an H4-stuck bot with no Inbox item reaches rank 2 with the stuck line', () => {
  const h4 = { veg: st('veg', 'stuck', { reason_code: 'stalled_retrying', since: at(8), waits_on: { kind: 'ai', label: 'Claude' } }) };
  const att = m.attentionRows({ kwants, needs: [], failed: [], finished: [], busy: {}, status: {}, h4, now: NOW });
  const v = att.find((r) => r.kwant.id === 'veg');
  assert.equal(v.rank, 2);
  assert.equal(v.line, 'Stuck: Claude stopped answering');
  assert.equal(att[0].kwant.id, 'veg');
});

test('N2: gave up reads "stopped after 2 tries"; needs-you still beats stuck; stuck oldest first', () => {
  const h4 = {
    veg: st('veg', 'stuck', { reason_code: 'stalled_gave_up', since: at(5), stuck: { gave_up_at: '2026-11-10T11:55:00Z', tries: 2 } }),
    rig: st('rig', 'stuck', { reason_code: 'stalled_retrying', since: at(50) }),
    lin: st('lin', 'stuck', { reason_code: 'stalled_retrying', since: at(60) }),
  };
  const needs = m.needsYouRows([item('question', 'lin', { timestamp: at(1) })], {}, all);
  const att = m.attentionRows({ kwants, needs, failed: [], finished: [], busy: {}, status: {}, h4, now: NOW });
  assert.deepEqual(order(att).slice(0, 3), ['1:Lin', '2:Rigel', '2:Vega']);
  assert.equal(att.find((r) => r.kwant.id === 'veg').line, 'Stuck: stopped after 2 tries · 11:55 AM');
  assert.equal(att.find((r) => r.kwant.id === 'rig').line, 'Stuck: the AI stopped answering');
});

test('N2: rank 5 (working) runs longest first by the H4 turn start', () => {
  const h4 = { iri: st('iri', 'working', { turn: { started_at: at(3) } }), rig: st('rig', 'working', { turn: { started_at: at(45) } }) };
  const att = m.attentionRows({ kwants, needs: [], failed: [], finished: [], busy: { iri: 'Working', rig: 'Working' }, status: { iri: 'working', rig: 'working' }, h4, now: NOW });
  assert.deepEqual(order(att).filter((x) => x.startsWith('5:')), ['5:Rigel', '5:Iris']);
});

test('N2: status list and bot_status events parse; an older server (404) gives null', async () => {
  const body = { bots: [st('lin', 'stuck', { reason_code: 'stalled_retrying' }), { kwant_id: 'x', status: 'weird' }, { nope: 1 }], now: at(0) };
  const map = m.parseH4List(body);
  assert.deepEqual(Object.keys(map), ['lin']);
  const next = m.applyBotStatusEvent(map, { kwant_id: 'pix', status: st('pix', 'working') });
  assert.equal(next.pix.status, 'working');
  assert.equal(m.applyBotStatusEvent(map, { kwant_id: 'pix', status: st('lin', 'working') }), map, 'mismatched ids are ignored');
  g.fetch = async () => new Response('{}', { status: 404 });
  assert.equal(await m.h4Api.list('sp1'), null);
  g.fetch = async () => new Response(JSON.stringify(body), { status: 200 });
  assert.equal((await m.h4Api.list('sp1')).lin.reason_code, 'stalled_retrying');
});

// ---------------------------------------------------------------------------------------------------
// N3

const reply = (reason, extra = {}) => ({ status: 'held', user_message_id: 'm-9', held: { kwant_id: 'jun', kwant_name: 'Juno', reason_code: reason, since: at(2), ...extra } });

test('N3: a held reply becomes a held line; only gave-up offers Try again', () => {
  assert.equal(m.heldFromReply({ status: 'started' }, 't1'), null);
  const hs = m.heldFromReply(reply('stuck'), 't1');
  assert.deepEqual([hs.tempId, hs.messageId, hs.kwantId, hs.name], ['t1', 'm-9', 'jun', 'Juno']);

  const retrying = m.heldLine(hs, st('jun', 'stuck', { reason_code: 'stalled_retrying', waits_on: { kind: 'ai', label: 'Claude' } }));
  assert.equal(retrying.tryAgain, false);
  assert.equal(retrying.text, 'Waiting for Juno: Juno is stuck (Claude stopped answering). Your message waits until Juno runs again.');

  const gaveUp = m.heldLine(hs, st('jun', 'stuck', { reason_code: 'stalled_gave_up', stuck: { gave_up_at: '2026-11-10T11:50:00Z', tries: 2 } }));
  assert.equal(gaveUp.tryAgain, true);
  assert.equal(gaveUp.text, 'Waiting for Juno: Juno stopped after 2 tries at 11:50 AM. Your message is kept and goes through when Juno runs again.');

  const q = m.heldLine(m.heldFromReply(reply('waits_on_you', { question: { asked_at: '2026-11-10T14:08:00Z' } }), 't2'));
  assert.equal(q.tryAgain, false);
  assert.equal(q.text, 'Queued for Juno: Juno is waiting for your answer to its question (2:08 PM). It goes through when you answer.');
  assert.equal(m.heldLine(m.heldFromReply(reply('paused'), 't3')).tryAgain, false);
});

test('N3: the line goes once the status no longer holds the message', () => {
  const hs = m.heldFromReply(reply('stuck'), 't1');
  assert.equal(m.stillHeld(hs, undefined), true);
  assert.equal(m.stillHeld(hs, st('jun', 'stuck', { held: [{ kind: 'message', id: 'm-9' }] })), true);
  assert.equal(m.stillHeld(hs, st('jun', 'working')), false);
});

const mounted = [];
afterEach(() => {
  for (const u of mounted.splice(0)) u();
  document.body.replaceChildren();
});
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const render = async (el) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(el);
  await settle();
  mounted.push(() => { root.unmount(); host.remove(); });
  return host;
};
const click = async (el) => {
  assert.ok(el, 'element to click exists');
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};

test('N3: Try again calls back, shows "Trying…", and says so when it fails', async () => {
  let release;
  let calls = 0;
  const host = await render(h(m.HeldLine, { text: 'Waiting for Juno', onTryAgain: () => { calls++; return new Promise((r) => { release = r; }); } }));
  const btn = host.querySelector('[data-held-try-again]');
  assert.equal(btn.textContent, 'Try again');
  assert.equal(host.querySelector('[role=status]').textContent, 'Waiting for Juno');
  await click(btn);
  assert.equal(calls, 1);
  assert.equal(btn.textContent, 'Trying…');
  assert.equal(btn.disabled, true);
  release('Couldn’t try again. Check your connection, then press it again.');
  await settle();
  assert.equal(btn.textContent, 'Try again');
  assert.match(host.querySelector('[data-held-error][role=alert]').textContent, /Couldn’t try again/);

  const plain = await render(h(m.HeldLine, { text: 'Queued for Lin' }));
  assert.equal(plain.querySelector('[data-held-try-again]'), null, 'no button unless the bot gave up');
});

test('N3: retry posts to the person-only route', async () => {
  const seen = [];
  g.fetch = async (url, init = {}) => {
    seen.push(`${init.method ?? 'GET'} ${url}`);
    return new Response(JSON.stringify({ released: 1, status: st('jun', 'working') }), { status: 200 });
  };
  const r = await m.h4Api.retry('jun');
  assert.equal(r.released, 1);
  assert.equal(r.status.status, 'working');
  assert.ok(seen.some((s) => s.startsWith('POST /api/kwants/jun/status/retry')));
});

// ---------------------------------------------------------------------------------------------------
// N1

const rankOf = (ranks) => (id) => (id ? ranks[id] : undefined);

test('N1: row ranks, the repo rollup in words, and the order of rows and repos', () => {
  const r = rankOf({ 'k-lin': 1, 'k-rigel': 6 });
  const shop = fx.LIST.repos[0];
  assert.deepEqual(shop.worktrees.map((w) => m.rowRank(w, r)), [5, 1, 7, 7], 'Pixel works (5), Lin needs you (1), Rigel is waiting (7), mine (7)');
  assert.deepEqual(m.sortWorktrees(shop.worktrees, r).map((w) => w.id), ['w2', 'w1', 'w3', 'w4']);
  const c = m.repoRollup(shop, r);
  assert.deepEqual(c, { needs: 1, stuck: 0, review: 0, finished: 0, working: 1 });
  assert.deepEqual(m.repoRollupParts(c).map((p) => p.text), ['1 needs you', '1 working']);
  assert.deepEqual(m.repoRollupParts({ needs: 2, stuck: 1, review: 1, finished: 3, working: 0 }).map((p) => p.text), ['2 need you', '1 stuck', '1 to review', '3 finished']);
  assert.equal(m.rowRank({ ...shop.worktrees[0], removed_at: at(1) }, r), 7);
  // A repo with a stuck bot goes above one that is only working; ties by name.
  const a = { ...shop, repo: 'zed', worktrees: [{ ...fx.WORKING, kwant_id: 'k-x' }] };
  const b = { ...shop, repo: 'alpha', worktrees: [{ ...fx.MINE }] };
  const z = { ...shop, repo: 'mid', worktrees: [{ ...fx.FINISHED_DUE, kwant_id: 'k-y' }] };
  assert.deepEqual(m.sortRepos([b, a, z], rankOf({ 'k-y': 2 })).map((x) => x.repo), ['mid', 'zed', 'alpha']);
});

test('N1: the Worktrees tab shows the rollup on the repo header and orders repos by their best row', async () => {
  const list = { repos: [fx.LIST.repos[1], { ...fx.LIST.repos[0] }] }; // site first in the reply
  g.fetch = async (url) => {
    const path = String(url).split('?')[0];
    if (path === '/api/spaces/sp1/dev/worktrees') return new Response(JSON.stringify(list), { status: 200 });
    if (path === '/api/spaces/sp1/dev/worktrees/branches') return new Response(JSON.stringify(fx.BRANCHES), { status: 200 });
    return new Response('{}', { status: 404 });
  };
  m.worktreeStore._reset();
  await m.worktreeStore.load('sp1');
  const bots = fx.BOTS.map((b) => ({ ...b, rank: b.id === 'k-lin' ? 1 : b.id === 'k-pixel' ? 5 : undefined }));
  const host = await render(h(m.WorktreesPanel, { space: 'sp1', data: m.worktreeStore.get('sp1'), bots, now: fx.NOW, home: '/home/ann' }));
  const roll = host.querySelector('[data-wt-rollup="shop"]');
  assert.ok(roll, 'shop has a rollup');
  assert.equal(roll.textContent.replace(/\s+/g, ' ').trim(), '1 needs you, 1 working', 'read as a list, with a hidden comma');
  assert.deepEqual([...roll.querySelectorAll('[data-wt-rollup-rank]')].map((e) => e.getAttribute('data-wt-rollup-rank')), ['1', '5']);
  assert.equal(host.querySelector('[data-wt-rollup="site"]'), null, 'no rollup on a repo with nothing going on');
  const ids = [...host.querySelectorAll('[data-wt-row]')].map((e) => e.getAttribute('data-wt-row'));
  assert.deepEqual(ids.slice(0, 2), ['w2', 'w1'], 'Lin needs you, so its worktree is first; Pixel works next');
  const repos = [...host.querySelectorAll('section[data-wt-repo]')].map((e) => e.getAttribute('data-wt-repo'));
  assert.deepEqual(repos, ['shop', 'site'], 'the repo with the ask comes before the quiet one');
});

// ---------------------------------------------------------------------------------------------------
// Union with 0.9 V1 (#506 "To review") on the integrated tree

const rv = (kwant_id, group, min, title = 'PR') => ({ ...item('review', kwant_id, { timestamp: at(min), title }), group, sort_key: [] });

test('Union: plain asks first in needsYouRows order, then reviews that need you oldest first', () => {
  const needs = m.needsYouRows([item('question', 'pix', { timestamp: at(2) }), item('approval', 'jun', { timestamp: at(10) })], { iri: 1 }, all);
  const reviews = [rv('lin', 'waiting_on_you', 90), rv('veg', 'waiting_on_you', 120)];
  const att = m.attentionRows({ kwants, needs, failed: [], finished: [], reviews, busy: {}, status: {}, now: NOW });
  assert.deepEqual(order(att).slice(0, 5), ['1:Juno', '1:Pixel', '1:Iris', '1:Vega', '1:Lin'], 'reviews come after every plain ask, even older ones; a browser step stays last among asks');
});

test('Union: stuck from an Inbox failure, a live H4 stuck or a review at rank 2; to review oldest; working longest first', () => {
  const h4 = {
    lin: st('lin', 'stuck', { reason_code: 'stalled_retrying', since: at(5), waits_on: { kind: 'ai', label: 'Claude' } }),
    rig: st('rig', 'working', { turn: { started_at: at(20) } }),
    iri: st('iri', 'working', {}),
  };
  const failed = [item('turn', 'lin', { status: 'failed', timestamp: at(30), title: 'Deploy' }), item('turn', 'pix', { status: 'failed', timestamp: at(10), title: 'Build' })];
  const reviews = [rv('jun', 'failed_or_stuck', 15, 'Couldn’t'), rv('lin', 'failed_or_stuck', 60), rv('veg', 'to_review', 40, 'Old'), rv('pix', 'to_review', 5)];
  const att = m.attentionRows({ kwants, needs: [], failed, finished: [], reviews, busy: { rig: 'Working', iri: 'Working' }, status: { rig: 'working', iri: 'working' }, h4, now: NOW });
  assert.deepEqual(order(att), ['2:Lin', '2:Juno', '2:Pixel', '3:Vega', '5:Rigel', '5:Iris']);
  const lin = att[0];
  assert.equal(lin.at, NOW - 60 * 60_000, 'the oldest of failure, stuck and review orders the group');
  assert.equal(lin.line, 'Stuck: Claude stopped answering', 'the live H4 line wins');
  assert.equal(lin.review?.kwant_id, 'lin', 'the older review gives the item it opens');
  assert.equal(att[1].line, 'Couldn’t approve: Couldn’t');
});
