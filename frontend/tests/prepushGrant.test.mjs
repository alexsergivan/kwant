// 0.9.7 prepush-grant (Pixel): the card after retry/override, one Needs-you item per branch, superseded.
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'CustomEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  export * from './src/dev/prepush';
  export * from './src/dev/freshEyes';
  export * as fx from './src/dev/prepushFixtures';
  export { parseReview } from './src/dev/types';
  export * as dfx from './src/dev/fixtures';
  export { PrepushCard } from './src/components/dev/PrepushCard';
  export { PrepushNeedsYou, PrepushThread, threadCards, usePrepushCards } from './src/components/dev/PrepushThread';
  export { FreshEyesChip } from './src/components/dev/FreshEyesChip';
  export { DevModeError } from './src/devMode';
  export { ReviewCard, semgrepTile } from './src/components/dev/ReviewCard';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
`);
const { fx } = m;
const h = m.createElement;
const html = (C, props) => m.renderToStaticMarkup(h(C, props));
const card = (raw) => {
  const c = m.parsePrepush(raw);
  assert.ok(c, 'fixture parses');
  return c;
};

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
  return { host, rerender: async (p) => { root.render(h(C, p)); await settle(); } };
};
const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const button = (host, t) => [...host.querySelectorAll('button')].find((b) => text(b).startsWith(t));
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const type = async (el, value) => {
  const set = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set;
  set.call(el, value);
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
};

// ---------------------------------------------------------------------------------------------------
// 0.9.7 prepush-grant (autonomy/prepush-retry/brief.md, Frontend): the press is the approval.

const HM = (ms) => new Date(ms).toISOString().slice(11, 16);
const all = (...ks) => ks.map((k) => card(fx[k]));

test('grant and superseded parse; pushed is a known status now; a junk grant is dropped', () => {
  const r = card(fx.PP_RETRIED_GRANT);
  assert.deepEqual(r.grant, { kind: 'retry', at: fx.PP_RETRIED_GRANT.grant.at, expires_at: fx.PP_RETRIED_GRANT.grant.expires_at, used_at: null, card_id: 'pp_rg', next_card_id: null });
  assert.equal(card({ ...fx.PP_RETRIED_GRANT, grant: { ...fx.PP_RETRIED_GRANT.grant, next_card_id: 'pp_next' } }).grant.next_card_id, 'pp_next');
  assert.equal(card({ ...fx.PP_RETRIED_GRANT, grant: { kind: 'always', at: 1 } }).grant, null, 'only retry or override');
  assert.equal(card({ ...fx.PP_RETRIED_GRANT, grant: null }).grant, null);
  assert.equal(card(fx.PP_CLEAR).grant, null, 'old servers send no grant');
  assert.equal(card(fx.PP_CLEAR).folder, null);
  assert.equal(card({ ...fx.PP_CLEAR, folder: 'kwant' }).folder, 'kwant');
  const s = card(fx.PP_SUPERSEDED);
  assert.equal(s.status, 'superseded');
  assert.equal(m.phaseOf(s), 'SS');
  assert.equal(card({ ...fx.PP_SUPERSEDED, needs_you: true }).needs_you, false, 'a superseded card never needs you');
  assert.equal(m.canRetry(s), false);
  assert.equal(m.statusChip(s), null);
  assert.equal(s.superseded_by, null);
  assert.equal(card({ ...fx.PP_SUPERSEDED, superseded_by: 'pp_new' }).superseded_by, 'pp_new');
  assert.equal(card({ ...fx.PP_BLOCKED, superseded_by: 'pp_new' }).superseded_by, null, 'only a superseded card names its replacement');
  const p = card(fx.PP_PUSHED);
  assert.equal(p.status, 'pushed');
  assert.equal(m.phaseOf(p), 'P6');
  assert.equal(m.statusChip(p).text, 'Pushed');
});

test('superseded renders folded: one grey line, no checks, no buttons', () => {
  const out = html(m.PrepushCard, { card: card(fx.PP_SUPERSEDED), bot: 'Pixel', onRetry: async () => {}, onOverride: async () => {} });
  assert.match(out, /data-prepush-phase="SS"/);
  assert.match(out, /Replaced by a newer push of <code[^>]*>pixel\/inbox-empty-state<\/code>/);
  assert.doesNotMatch(out, /<button/);
  assert.doesNotMatch(out, /data-check=/);
  const linked = html(m.PrepushCard, { card: card({ ...fx.PP_SUPERSEDED, superseded_by: 'pp_new' }), bot: 'Pixel', onOpenCard: () => {} });
  assert.match(linked, /data-superseded-by="pp_new"[^>]*>See the newer push/, 'the server names the newer card: link to it');
});

test('needsYou: one item per bot, folder and branch, the newest; an older card on a pushed-again branch never shows', () => {
  const older = card({ ...fx.PP_FAILED_NEEDS_YOU, id: 'a', created_at: 1, updated_at: 1 });
  const mid = card({ ...fx.PP_NEEDS_YOU, id: 'b', created_at: 2, updated_at: 2 });
  const newest = card({ ...fx.PP_NEEDS_YOU, id: 'c', created_at: 3, updated_at: 3 });
  assert.deepEqual(m.needsYou([older, mid, newest]).map((c) => c.id), ['c'], "Oleks's four items become one");
  const clearAfter = card({ ...fx.PP_CLEAR, id: 'd', created_at: 4, updated_at: 4 });
  assert.deepEqual(m.needsYou([older, mid, newest, clearAfter]), [], 'the bot pushed again and it went clear: nothing waits');
  const otherBot = card({ ...fx.PP_NEEDS_YOU, id: 'e', kwant_id: 'k_iris', created_at: 1, updated_at: 5 });
  const otherFolder = card({ ...fx.PP_NEEDS_YOU, id: 'f', folder: 'site', created_at: 1, updated_at: 6 });
  const otherBranch = card({ ...fx.PP_NEEDS_YOU, id: 'g', branch: 'pixel/x', created_at: 1, updated_at: 7 });
  assert.deepEqual(m.needsYou([newest, otherBot, otherFolder, otherBranch]).map((c) => c.id), ['g', 'f', 'e', 'c'], 'separate targets each keep one, newest update first');
  const sup = card({ ...fx.PP_SUPERSEDED, id: 'h', created_at: 2 });
  assert.deepEqual(m.needsYou([sup, newest]).map((c) => c.id), ['c']);
  const revived = card({ ...fx.PP_NEEDS_YOU, id: 'old', created_at: 1, updated_at: 9 });
  const newer = card({ ...fx.PP_SUPERSEDED, id: 'new', created_at: 5, updated_at: 8, branch: revived.branch, kwant_id: revived.kwant_id, space_id: revived.space_id, folder: revived.folder });
  assert.deepEqual(m.needsYou([revived, newer]).map((c) => c.id), ['old'], 'a revived older card waits even though a newer card exists, when that newer one is superseded');
});

test('needsYou: a card the person just pressed leaves at once, and comes back if the press failed', async () => {
  const c = card(fx.PP_NEEDS_YOU);
  assert.deepEqual(m.needsYou([c], new Set(['pp_needs'])), []);
  const { host, rerender } = await mount(m.PrepushNeedsYou, { cards: [c], names: { k_pixel: 'Pixel' }, onOpen: () => {}, acted: new Set() });
  assert.equal(host.querySelectorAll('[data-needs-prepush-item]').length, 1);
  // The branch tells two items apart, so it is there on a phone too (under the line), not only beside it.
  const phoneBranch = host.querySelector('[data-needs-branch-phone]');
  assert.equal(phoneBranch?.textContent, c.branch);
  assert.match(phoneBranch.className, /sm:hidden/);
  assert.doesNotMatch(phoneBranch.className, /max-sm:hidden/);
  await rerender({ cards: [c], names: { k_pixel: 'Pixel' }, onOpen: () => {}, acted: new Set(['pp_needs']) });
  assert.equal(host.querySelector('[data-needs-prepush]'), null, 'gone before the server answers');
  await rerender({ cards: [c], names: { k_pixel: 'Pixel' }, onOpen: () => {}, acted: new Set() });
  assert.equal(host.querySelectorAll('[data-needs-prepush-item]').length, 1, 'back when the call failed');
});

test('usePrepushCards: setActed marks and unmarks a card', async () => {
  let state;
  const api = { list: async () => [card(fx.PP_NEEDS_YOU)], get: async () => card(fx.PP_NEEDS_YOU) };
  const Probe = () => {
    state = m.usePrepushCards('s_demo', true, api);
    return null;
  };
  await mount(Probe, {});
  assert.equal(state.acted.size, 0);
  state.setActed('pp_needs', true);
  await settle();
  assert.ok(state.acted.has('pp_needs'));
  state.setActed('pp_needs', false);
  await settle();
  assert.equal(state.acted.size, 0);
});

test('followUp: the new card the retry led to, by the grant\'s card id, else the newest later card of the same target', () => {
  const r = card(fx.PP_RETRIED_GRANT);
  assert.equal(m.followUp(r, [r]), null);
  assert.equal(m.followUp(r, all('PP_RETRIED_GRANT', 'PP_NEXT_CHECKING', 'PP_NEEDS_YOU')).id, 'pp_next');
  const noId = card({ ...fx.PP_NEXT_CHECKING, grant: null });
  assert.equal(m.followUp(r, [r, noId]).id, 'pp_next', 'old server: matched by branch and time');
  assert.equal(m.followUp(r, [r, card({ ...fx.PP_NEXT_CHECKING, branch: 'other' })]), null, 'another branch is another story');
  const named = card({ ...fx.PP_RETRIED_GRANT, grant: { ...fx.PP_RETRIED_GRANT.grant, next_card_id: 'pp_named' } });
  const newer = card({ ...fx.PP_NEXT_CHECKING, id: 'pp_newer', created_at: fx.PP_NEXT_CHECKING.created_at + 60000 });
  assert.equal(m.followUp(named, [named, newer, card({ ...fx.PP_NEXT_CLEAR, id: 'pp_named' })]).id, 'pp_named', "the server's next_card_id wins");
  assert.equal(m.followUp(r, [r, card({ ...fx.PP_NEXT_CHECKING, grant: { ...fx.PP_NEXT_CHECKING.grant, card_id: 'someone_else' } })]), null);
  assert.equal(m.pressedBefore(card(fx.PP_NEXT_CHECKING), all('PP_RETRIED_GRANT', 'PP_NEXT_CHECKING')).id, 'pp_rg');
  assert.equal(m.pressedBefore(card(fx.PP_CLEAR), [card(fx.PP_CLEAR)]), null);
});

test('story after a retry: fixing → checking the new push → Pushed · HH:MM, or blocked again', () => {
  const r = card(fx.PP_RETRIED_GRANT);
  const at = fx.PP_RETRIED_GRANT.grant.at;
  const s0 = m.storyOf(r, [r], 'Quant', null, at + 1000);
  assert.equal(s0.step, 'fixing');
  assert.equal(s0.lead, 'You asked Quant to fix this and push again. Its next push of pixel/inbox-empty-state goes without asking you again, once the checks pass.');
  assert.equal(s0.now, 'Quant is fixing it…');
  const s1 = m.storyOf(r, all('PP_RETRIED_GRANT', 'PP_NEXT_CHECKING'), 'Quant', null, at);
  assert.equal(s1.step, 'checking');
  assert.equal(s1.now, "Checking Quant's new push…");
  assert.equal(s1.next.id, 'pp_next');
  assert.equal(m.storyOf(r, all('PP_RETRIED_GRANT', 'PP_NEXT_CLEAR'), 'Quant', null, at).step, 'pushing');
  const s3 = m.storyOf(r, all('PP_RETRIED_GRANT', 'PP_NEXT_PUSHED'), 'Quant', null, at);
  assert.equal(s3.step, 'pushed');
  assert.equal(s3.now, `Pushed · ${HM(fx.PP_NEXT_PUSHED.grant.used_at)}`);
  const s4 = m.storyOf(r, all('PP_RETRIED_GRANT', 'PP_NEXT_BLOCKED'), 'Quant', null, at);
  assert.equal(s4.step, 'blocked');
  assert.equal(s4.now, 'Blocked again: 1 finding. Nothing was pushed.');
  assert.equal(m.storyOf(r, [r], 'Quant', null, fx.PP_RETRIED_GRANT.grant.expires_at + 1).step, 'expired', 'a window that ran out says so');
  const old = card({ ...fx.PP_RETRIED_GRANT, grant: null });
  assert.match(m.storyOf(old, [old], 'Quant').lead, /is checked from try 1\.$/, 'without a grant the card promises nothing it can\'t keep');
  assert.equal(m.storyOf(card(fx.PP_NEEDS_YOU), [], 'Quant'), null, 'only pressed cards tell the story');
});

test('story after Push anyway: pushing → Pushed · HH:MM', () => {
  const o = card(fx.PP_OVERRIDDEN_GRANT);
  const s = m.storyOf(o, [o], 'Quant');
  assert.equal(s.step, 'pushing');
  assert.match(s.lead, /goes without asking you again/);
  const p = card(fx.PP_PUSHED);
  const sp = m.storyOf(p, [p], 'Quant');
  assert.equal(sp.step, 'pushed');
  assert.equal(sp.now, `Pushed · ${HM(fx.PP_PUSHED.grant.used_at)}`);
});

test('a floor or payment approval the push still needs gets one line with a link; an ordinary approval does not', () => {
  const r = card(fx.PP_RETRIED_GRANT);
  const after = new Date(fx.PP_RETRIED_GRANT.grant.at + 60_000).toISOString();
  const floor = { id: 'ap1', tool_name: 'Bash', kwant_id: 'k_pixel', asked_at: after, details: { prod_floor: { kind: 'push_main', why: 'it pushes to main, a watched branch.' } } };
  const pay = { id: 'ap2', tool_name: 'stripe', kwant_id: 'k_pixel', asked_at: after, details: { payment_reason: 'it charges a card' } };
  const plain = { id: 'ap3', tool_name: 'Bash', kwant_id: 'k_pixel', asked_at: after, details: { command: 'git push' } };
  const otherBot = { ...floor, id: 'ap4', kwant_id: 'k_iris' };
  const before = { ...floor, id: 'ap5', asked_at: new Date(fx.PP_RETRIED_GRANT.grant.at - 600_000).toISOString() };
  assert.deepEqual(m.pushApproval(r, [plain, otherBot, before, floor]), { id: 'ap1', reason: 'it pushes to main, a watched branch' });
  assert.deepEqual(m.pushApproval(r, [pay]), { id: 'ap2', reason: 'it charges a card' });
  assert.equal(m.pushApproval(r, [plain]), null);
  assert.equal(m.pushApproval(card(fx.PP_NEEDS_YOU), [floor]), null, 'only after a press');
  assert.equal(m.pushApproval(card(fx.PP_PUSHED), [floor]), null, 'a push that went needs nothing');
  const s = m.storyOf(r, all('PP_RETRIED_GRANT', 'PP_NEXT_CLEAR'), 'Quant', { id: 'ap1', reason: 'it pushes to main' });
  assert.equal(s.step, 'asks');
  assert.equal(s.now, "Quant's push needs your OK: it pushes to main");
});

test('the retried card shows the new state at once and links to the new card', async () => {
  const r = card(fx.PP_RETRIED_GRANT);
  const out0 = html(m.PrepushCard, { card: r, bot: 'Quant', story: m.storyOf(r, [r], 'Quant', null, fx.PP_RETRIED_GRANT.grant.at) });
  assert.match(out0, /data-prepush-phase="RT"/);
  assert.match(out0, /You asked Quant to fix this and push again\. Its next push of <code[^>]*>pixel\/inbox-empty-state<\/code> goes without asking you again, once the checks pass\./);
  assert.match(out0, /data-story="fixing"/);
  assert.match(out0, /role="status"[^>]*>.*Quant is fixing it…/);
  const opened = [];
  const cards = all('PP_RETRIED_GRANT', 'PP_NEXT_BLOCKED');
  const { host } = await mount(m.PrepushCard, { card: r, bot: 'Quant', story: m.storyOf(r, cards, 'Quant'), onOpenCard: (id) => opened.push(id) });
  assert.match(text(host), /Blocked again: 1 finding\. Nothing was pushed\./);
  await click(button(host, 'See what it found'));
  assert.deepEqual(opened, ['pp_next']);
});

test('the story line follows the new push live, and a floor approval line opens that approval', async () => {
  const r = card(fx.PP_RETRIED_GRANT);
  const at = fx.PP_RETRIED_GRANT.grant.at;
  const props = (cards, approval = null) => ({ card: r, bot: 'Quant', story: m.storyOf(r, cards, 'Quant', approval, at), onOpenCard: () => {}, onOpenApproval: (id) => openedA.push(id) });
  const openedA = [];
  const { host, rerender } = await mount(m.PrepushCard, props([r]));
  const now = () => text(host.querySelector('[data-story-now]'));
  assert.equal(now(), 'Quant is fixing it…');
  await rerender(props(all('PP_RETRIED_GRANT', 'PP_NEXT_CHECKING')));
  assert.equal(now(), "Checking Quant's new push…");
  assert.ok(host.querySelector('[data-story-next="pp_next"]'), 'links to the new card');
  await rerender(props(all('PP_RETRIED_GRANT', 'PP_NEXT_CLEAR'), { id: 'ap1', reason: 'it pushes to main' }));
  const line = host.querySelector('[data-story-approval="ap1"]');
  assert.ok(line);
  assert.match(text(line), /Quant's push needs your OK: it pushes to main/);
  // Said once: the amber line is the only status line, not a second "needs your OK." above it.
  assert.equal((text(host).match(/needs your OK/g) ?? []).length, 1);
  assert.equal(host.querySelectorAll('[role="status"]').length, 1);
  assert.equal(line.getAttribute('role'), 'status');
  await click(button(line, 'Open it'));
  assert.deepEqual(openedA, ['ap1']);
  await rerender(props(all('PP_RETRIED_GRANT', 'PP_NEXT_PUSHED')));
  assert.equal(now(), `Pushed · ${HM(fx.PP_NEXT_PUSHED.grant.used_at)}`);
  assert.equal(host.querySelector('[data-story-approval]'), null);
});

test('the new card links back to the request, and once its push went it folds to "Pushed · HH:MM"', async () => {
  const cards = all('PP_RETRIED_GRANT', 'PP_NEXT_CHECKING');
  const next = cards[1];
  const opened = [];
  const { host } = await mount(m.PrepushCard, { card: next, bot: 'Quant', after: cards[0], onOpenCard: (id) => opened.push(id) });
  assert.match(text(host.querySelector('[data-after-retry="pp_rg"]')), new RegExp(`After you asked for a fix at ${HM(fx.PP_RETRIED_GRANT.grant.at)}\\.`));
  await click(button(host, 'See your request'));
  assert.deepEqual(opened, ['pp_rg']);
  const out = html(m.PrepushCard, { card: card(fx.PP_NEXT_PUSHED), bot: 'Quant', after: cards[0] });
  assert.match(out, /data-prepush-phase="P2b"/);
  assert.match(out, new RegExp(`Pushed · ${HM(fx.PP_NEXT_PUSHED.grant.used_at)}`));
  assert.match(out, /with no second OK: you asked for the fix\./);
});

test('Push anyway: the card follows its one push to "Pushed · HH:MM"', () => {
  const o = card(fx.PP_OVERRIDDEN_GRANT);
  const out = html(m.PrepushCard, { card: o, bot: 'Quant', story: m.storyOf(o, [o], 'Quant') });
  assert.match(out, /data-story="pushing"/);
  assert.match(out, /Quant can push it now\./);
  const p = card(fx.PP_PUSHED);
  const outP = html(m.PrepushCard, { card: p, bot: 'Quant', story: m.storyOf(p, [p], 'Quant') });
  assert.match(outP, /data-story="pushed"/);
  assert.match(outP, new RegExp(`Pushed · ${HM(fx.PP_PUSHED.grant.used_at)}`));
  assert.doesNotMatch(outP, /<button[^>]*>(Push anyway…|Ask Quant)/);
});

test('PrepushThread wires the story: the retried card follows the new card from the whole list', async () => {
  const every = all('PP_RETRIED_GRANT', 'PP_NEXT_CHECKING');
  const { host } = await mount(m.PrepushThread, { cards: every, all: every, bot: 'Quant', live: new Set(), approvals: [] });
  assert.equal(text(host.querySelector('#prepush-pp_rg [data-story-now]')), "Checking Quant's new push…");
  assert.ok(host.querySelector('#prepush-pp_next [data-after-retry="pp_rg"]'));
});
