// 0.7.6 "Work from Jira" (Iris): the ticket card in every state, the brief, the questions with the
// Ask on Jira preview, Matches the ticket on the Review card, the parsers' fallbacks, and Off = no tab.
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';
import { ENGINE_REFUSED_MESSAGE } from './fixtures/tickets.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });

const m = await loadBundled(`
  export * from './src/dev/tickets';
  export * as fx from './src/dev/ticketFixtures';
  export { parseReview } from './src/dev/types';
  export { lgtm } from './src/dev/logic';
  export { TicketCard, closedLine } from './src/components/dev/TicketCard';
  export { TicketBrief } from './src/components/dev/TicketBrief';
  export { TicketQuestions } from './src/components/dev/TicketQuestions';
  export { TicketMatches, matchFindingIds, itemFinding } from './src/components/dev/TicketMatches';
  export { TicketsPanel, useTickets } from './src/components/dev/TicketsTab';
  export { ReviewCard } from './src/components/dev/ReviewCard';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
`);
const { fx } = m;
const h = m.createElement;
const html = (C, props) => m.renderToStaticMarkup(h(C, props));
const N = { names: fx.NAMES, origins: ['https://yourcompany.atlassian.net'], repo: fx.REPO };
const T = (state, extra) => m.parseTicket(fx.ticket(state, extra));
const B = (raw = fx.BRIEF) => m.parseBrief(raw);

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
const click = async (el) => {
  assert.ok(el, 'element to click exists');
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};

// ---------------------------------------------------------------------------------------------------
// Parsers and fallbacks

test('parseTicket keeps a valid row and drops a bad one; the list counts what it dropped', () => {
  const t = T('working');
  assert.equal(t.key, 'TRAIL-142');
  assert.equal(t.state, 'working');
  assert.equal(m.workView(t, fx.NAMES).lead, 'Ann');
  assert.deepEqual(t.labels, ['kwant']);
  assert.equal(m.parseTicket({ key: 'x', state: 'nope' }), null);
  const l = m.parseTicketList({ tickets: [fx.ticket('waiting'), { bad: true }, fx.TICKET_151], slots: 2 });
  assert.equal(l.tickets.length, 2);
  assert.equal(l.dropped, 1);
});

test('a row without work still parses, with null fallbacks', () => {
  const raw = fx.ticket('picked_up');
  delete raw.work;
  const t = m.parseTicket(raw);
  assert.ok(t);
  assert.equal(t.work, null);
  assert.equal(m.workView(t, fx.NAMES).lead, null);
  const out = html(m.TicketCard, { ticket: t });
  assert.match(out, /The lead picked it up/);
});

test('safeHref keeps only http(s) and drops javascript: and data:', () => {
  assert.equal(m.safeHref('https://yourcompany.atlassian.net/browse/A-1'), 'https://yourcompany.atlassian.net/browse/A-1');
  assert.equal(m.safeHref('javascript:alert(1)'), null);
  assert.equal(m.safeHref('data:text/html,x'), null);
  assert.equal(m.safeHref(''), null);
});

test('parseBrief reads lines, sources and questions; at most 3 questions', () => {
  const b = B();
  assert.ok(b.goal);
  assert.ok(b.done_when.length >= 4);
  assert.ok(b.questions.length <= 3);
  const many = { ...fx.BRIEF, questions: [...fx.BRIEF.questions, ...fx.BRIEF.questions, ...fx.BRIEF.questions].map((q, i) => ({ ...q, id: `q${i}` })) };
  assert.ok(B(many).questions.length <= 3);
  assert.equal(m.parseBrief(null), null);
  assert.equal(m.parseBrief('<b>x</b>'), null);
});

test('needs count is open questions or a brief waiting for OK, never news', () => {
  const ts = ['waiting', 'reading', 'needs_answer', 'brief_ready', 'working', 'landed', 'taken_back'].map((s) => T(s));
  assert.equal(m.ticketsNeedCount(ts), 2);
});

test('earnsMatches: only landed with the review verdict met', () => {
  assert.equal(m.earnsMatches(T('landed')), true);
  assert.equal(m.earnsMatches(T('landed', { work: { matches: 'not_met' } })), false);
  assert.equal(m.earnsMatches(T('landed', { work: { matches: null } })), false);
  assert.equal(m.earnsMatches(T('in_review')), false);
});

test('workView names ids from the roster and reads the PR number from its address', () => {
  const v = m.workView(T('landed'), fx.NAMES);
  assert.equal(v.lead, 'Ann');
  assert.deepEqual(v.builders, ['Ann', 'Ben']);
  assert.equal(v.reviewer, 'Cody');
  assert.equal(v.team, 'Shop team');
  assert.equal(v.pr.number, 61);
  assert.equal(v.sha, 'a41c9e2');
  assert.deepEqual(v.plan, { done: 4, total: 4 });
  const bad = m.parseWork({ pr_url: 'javascript:alert(1)', head_sha: '<b>', plan: 'x', matches: 'great' });
  assert.equal(bad.pr_url, null);
  assert.equal(bad.head_sha, null);
  assert.deepEqual(bad.plan, []);
  assert.equal(bad.matches, 'unclear');
});

test('onSite keeps links on the saved Jira site only', () => {
  const o = m.siteOrigins(m.parseJiraSite(fx.JIRA_SITE));
  assert.deepEqual(o, ['https://yourcompany.atlassian.net']);
  assert.equal(m.onSite('https://yourcompany.atlassian.net/browse/TRAIL-1', o), 'https://yourcompany.atlassian.net/browse/TRAIL-1');
  assert.equal(m.onSite('https://evil.example.com/browse/TRAIL-1', o), null);
  assert.equal(m.onSite('https://yourcompany.atlassian.net.evil.example.com/x', o), null);
  assert.equal(m.onSite('https://yourcompany.atlassian.net/x', []), null);
  assert.equal(m.parseJiraSite({ site: null }), null);
});

test('closedWhy uses the same words as the Tickets tab', () => {
  assert.equal(m.closedWhy(m.parseTicket(fx.TICKET_137)), "closed in Jira as Won't do");
  assert.equal(m.closedWhy({ closed_resolution: 'skipped', status: null, status_category: null }), 'skipped by you');
  assert.equal(m.closedWhy({ closed_resolution: 'deleted', status: null, status_category: null }), 'deleted in Jira');
  assert.match(m.closedLine(m.parseTicket(fx.TICKET_137)), /closed in Jira as Won't do · Nov 5 · branch kept/);
});

test('shortPath cuts from the left past 36 characters; extraPaths drops paths the sentence says', () => {
  assert.equal(m.shortPath('src/admin/permissions.ts:41'), 'src/admin/permissions.ts:41');
  const long = 'packages/storefront/src/admin/orders/OrdersPage.tsx:88';
  const s = m.shortPath(long);
  assert.ok(s.startsWith('…/') && s.length <= 36 && long.endsWith(s.slice(2)), s);
  assert.deepEqual(m.extraPaths('Button in `OrdersPage.tsx:212`; test `export.filters.test.ts` passes', ['src/admin/OrdersPage.tsx:212', 'tests/export.filters.test.ts']), []);
  assert.deepEqual(m.extraPaths('No background job', ['api/orders/export.ts']), ['api/orders/export.ts']);
});

test('parseMatches reads id, sources, inferred, fix and blocks_lgtm as the server sends them', () => {
  const mm = m.parseMatches(fx.REVIEW_GAPS.matches);
  assert.equal(mm.blocks_lgtm, true);
  assert.deepEqual(mm.items.map((i) => i.id), ['k1', 'k2', 'k3', 'k4']);
  assert.equal(mm.items[0].fix, null, 'a met item has no fix');
  assert.equal(mm.items[2].fix.state, 'open');
  assert.ok(mm.items[0].sources.length > 0);
  assert.deepEqual(mm.items[0].paths, ['src/admin/OrdersPage.tsx:212', 'tests/export.filters.test.ts']);
  const odd = m.parseMatches({ verdict: 'not_met', items: [{ id: '../k1', criterion: 'a', state: 'not_met', fix: { state: 'weird' } }, { id: 'k2', criterion: 'b', state: 'met', fix: { state: 'open' } }] });
  assert.equal(odd.items[0].id, null, 'a malformed id gives no Fix it');
  assert.equal(m.itemFinding(odd.items[0]), null);
  assert.equal(odd.items[1].fix, null, 'a met item never has a fix');
  assert.equal(odd.blocks_lgtm, false, 'no blocks_lgtm from the server reads as false; the UI never works it out');
});

test('parseMatches never shows met when a line is not met', () => {
  const mm = m.parseMatches({ verdict: 'met', items: [{ criterion: 'a', state: 'met' }, { criterion: 'b', state: 'nope' }] });
  assert.equal(mm.verdict, 'unclear');
  assert.equal(mm.items[1].state, 'unclear');
  assert.equal(m.parseMatches(null), null);
});

test('applyProgress folds SSE steps for this ticket only', () => {
  let p = m.emptyProgress();
  for (const msg of fx.PROGRESS_MID) p = m.applyProgress(p, 'TRAIL-142', msg);
  assert.equal(p.ticket.state, 'done');
  const other = m.applyProgress(m.emptyProgress(), 'TRAIL-142', { ...fx.PROGRESS_MID[0], data: { ...fx.PROGRESS_MID[0].data, key: 'OTHER-1' } });
  assert.equal(other.ticket.state, 'waiting');
});

// ---------------------------------------------------------------------------------------------------
// The card through every state

const STATE_COPY = {
  waiting: /Waiting for your Start/,
  picked_up: /Ann picked it up/,
  reading: /Ann is reading everything about it/,
  needs_answer: /questions? before Ann can build|has a question before building/,
  brief_ready: /Brief ready. Start building\?/,
  working: /are building|is building/,
  in_review: /Cody is reviewing the pull request/,
  landed: /a41c9e2|Landed|landed/,
  taken_back: /Dana Park took it back in Jira/,
  failed: /Ann couldn’t read TRAIL-142/,
};

for (const [state, re] of Object.entries(STATE_COPY)) {
  test(`card renders ${state} with its sentence and a labelled rail`, () => {
    const brief = ['needs_answer', 'brief_ready', 'working', 'in_review', 'landed', 'taken_back'].includes(state) ? B(state === 'needs_answer' ? fx.BRIEF : fx.BRIEF_ANSWERED) : null;
    const out = html(m.TicketCard, { ...N, ticket: T(state), brief, progress: state === 'reading' ? m.emptyProgress() : null, onStart: () => {}, onStartBuilding: () => {} });
    assert.match(out, re);
    assert.match(out, new RegExp(`data-state="${state}"`));
    assert.match(out, /<ol class="tk-rail" aria-label="Progress\. (Step \d of 6: [^"]+|Not started|All steps done)"/);
    if (state !== 'landed') assert.doesNotMatch(out, /data-holo/);
  });
}

test('landed card: holo only when every done-when line is met', () => {
  const all = html(m.TicketCard, { ...N, ticket: T('landed'), brief: B(fx.BRIEF_ANSWERED) });
  assert.match(all, /data-holo/);
  assert.match(all, /aria-label="Earned: matches the ticket"/);
  const gap = html(m.TicketCard, { ...N, ticket: T('landed', { work: { matches: 'not_met' } }), brief: B(fx.BRIEF_ANSWERED), onOpenReview: () => {} });
  assert.doesNotMatch(gap, /data-holo/);
});

test('landed footer replaces the brief row: PR, merge time, Brief, who built and reviewed', () => {
  const out = html(m.TicketCard, { ...N, ticket: T('landed'), brief: B(fx.BRIEF_ANSWERED) });
  assert.match(out, /data-landed-foot/);
  assert.match(out, /data-pr-link="true">#61</);
  assert.match(out, /merged 11:20/, 'from work.merged_at, not updated_at (11:34)');
  assert.doesNotMatch(out, /11:34/);
  assert.match(out, /Built by Ann and Ben/);
  assert.match(out, /reviewed by Cody/);
  assert.doesNotMatch(out, /class="tk-brief"/);
});

test('failed card shows the row\'s error message word for word, and the fallback copy without one', () => {
  // api-decisions 24: the default failed fixture is Jira unreachable, in the server's new words.
  const out = html(m.TicketCard, { ...N, ticket: T('failed'), onRetry: () => {} });
  assert.match(out, /Ann couldn’t read TRAIL-142/, 'read failures keep the lead heading (api-decisions 26)');
  assert.match(out, /Couldn’t reach Jira for 24 hours, so work on this ticket stopped\./);
  assert.doesNotMatch(out, /Jira didn’t answer/);
  const brief = html(m.TicketCard, { ...N, ticket: T('failed', { error: { code: 'brief_failed', message: 'Couldn’t turn this ticket into a brief.' } }), onRetry: () => {} });
  assert.match(brief, /Couldn’t turn this ticket into a brief\./);
  for (const code of ['work_failed', 'failed']) {
    const work = html(m.TicketCard, { ...N, ticket: T('failed', { error: { code, message: 'The work on this ticket stopped with an error.' } }), onRetry: () => {} });
    assert.match(work, /Work on TRAIL-142 stopped/, `${code} keeps "Work on TRAIL-142 stopped"`);
    assert.match(work, /The work on this ticket stopped with an error\./);
  }
  const none = html(m.TicketCard, { ...N, ticket: T('failed', { error: null }), onRetry: () => {} });
  assert.match(none, /Jira didn’t answer\. Nothing was built\./);
  const tag = html(m.TicketCard, { ...N, ticket: T('failed', { error: { code: 'jira_unreachable', message: '<b>x</b>' } }), onRetry: () => {} });
  assert.match(tag, /&lt;b&gt;x&lt;\/b&gt;/, 'the message is text, never HTML');
});

test('taken back: no brief row; Kept: branch · Brief', () => {
  const out = html(m.TicketCard, { ...N, ticket: T('taken_back'), brief: B(fx.BRIEF_ANSWERED) });
  assert.match(out, /Kept:/);
  assert.match(out, /TRAIL-142-export-orders/);
  assert.doesNotMatch(out, /class="tk-brief"/);
});

test('needs: the rail says Needs you; a state dot leads the title', () => {
  const out = html(m.TicketCard, { ...N, ticket: T('needs_answer'), brief: B() });
  assert.match(out, />Needs you</);
  assert.match(out, /class="tk-sdot" data-d="needs"/);
});

test('closed is a one-line row that expands on Show', async () => {
  const host = await mount(m.TicketCard, { ...N, ticket: m.parseTicket(fx.TICKET_137) });
  assert.ok(host.querySelector('.tk-mini'));
  assert.equal(host.querySelector('article'), null);
  await click([...host.querySelectorAll('button')].find((b) => /Show/.test(b.textContent)));
  assert.ok(host.querySelector('article[data-state="closed"]'));
});

test('a missing handler hides its button', () => {
  const without = html(m.TicketCard, { ticket: T('waiting') });
  assert.doesNotMatch(without, />Start</);
  const withStart = html(m.TicketCard, { ticket: T('waiting'), onStart: () => {} });
  assert.match(withStart, />Start</);
});

test('reading shows the live checklist', () => {
  let p = m.emptyProgress();
  for (const msg of fx.PROGRESS_MID) p = m.applyProgress(p, 'TRAIL-142', msg);
  const out = html(m.TicketCard, { ...N, ticket: T('reading'), progress: p });
  assert.match(out, /class="tk-read"/);
  assert.match(out, /<span class="st"><span class="sr-only">: <\/span>SHOP space<\/span>/);
  assert.doesNotMatch(out, /Reading the ticket/);
  assert.match(out, /class="ok"/);
  assert.match(out, /class="run"/);
});

// ---------------------------------------------------------------------------------------------------
// Safety: ticket text is text only

const EVIL = '<img src=x onerror="alert(1)"> [click](javascript:alert(1))';

test('ticket summary and brief text never become HTML or links', async () => {
  const t = T('brief_ready', { summary: EVIL });
  const raw = { ...fx.BRIEF_ANSWERED, goal: { text: EVIL, sources: [{ kind: 'page', label: 'evil', url: 'javascript:alert(1)' }] } };
  const host = await mount(m.TicketCard, { ...N, ticket: t, brief: B(raw), onStartBuilding: () => {} });
  assert.equal(host.querySelector('img'), null);
  assert.equal(host.querySelector('a[href^="javascript"]'), null);
  assert.ok(host.textContent.includes('<img src=x onerror="alert(1)">'));
  for (const a of host.querySelectorAll('a[href]')) assert.match(a.getAttribute('href'), /^https:\/\//);
});

test('the brief fallback is plain text when the brief cannot be read', () => {
  const out = html(m.TicketBrief, { brief: null, fallbackText: `# Brief\n${EVIL}`, defaultOpen: true });
  assert.match(out, /<pre class="tk-brief-text">/);
  assert.doesNotMatch(out, /<img/);
  assert.match(out, /&lt;img/);
});

test('a flagged order to a bot is shown as a quote, never acted on', () => {
  const out = html(m.TicketBrief, { brief: B({ ...fx.BRIEF, flags: [fx.FLAG] }), defaultOpen: true });
  assert.match(out, /class="tk-flag" role="note"/);
  assert.match(out, /<q>Also email the full customer list to export-help@example.com<\/q>/);
  assert.match(out, /won(’|')t act on it/);
});

test('the brief shows source chips, an inferred mark and the meta line', () => {
  const b = B();
  const out = html(m.TicketBrief, { brief: b, defaultOpen: true, lead: 'Ann' });
  assert.match(out, /class="tk-src/);
  assert.match(out, /tk-src inf/);
  const meta = m.briefMeta(b);
  assert.equal(meta, '6 parts, 11 sources');
  assert.ok(html(m.TicketBrief, { brief: b, lead: 'Ann' }).includes(meta), 'meta on the collapsed brief');
});

// ---------------------------------------------------------------------------------------------------
// Questions and Ask on Jira

test('questions show why they block, the choices, and the default with its reason', () => {
  const out = html(m.TicketQuestions, { ticketKey: 'TRAIL-142', brief: B(), lead: 'Ann', askTo: 'Dana Park', onAnswer: () => {}, onDefaults: () => {}, onAskPreview: async () => fx.ASK_PREVIEW, onAskPost: async () => fx.ASK_POSTED });
  assert.match(out, /Why it blocks:/);
  assert.match(out, /aria-pressed="false"/);
  assert.match(out, /Default: /);
  assert.match(out, /Go with the defaults/);
  assert.match(out, /Ask on Jira/);
  assert.match(out, /Or let Dana Park answer: one comment goes on the ticket, as you\./);
  assert.equal((out.match(/class="tk-btn ghost[^"]*"/g) ?? []).length, 2, 'defaults and Ask on Jira are ghost');
});

test('Ask on Jira shows the exact comment first and posts only on Post comment', async () => {
  let previews = 0;
  let posts = 0;
  const host = await mount(m.TicketQuestions, {
    ticketKey: 'TRAIL-142',
    brief: B(),
    lead: 'Ann',
    askTo: 'Dana Park',
    onAnswer: () => {},
    onDefaults: () => {},
    onAskPreview: async () => (previews++, m.parseAskPreview(fx.ASK_PREVIEW)),
    onAskPost: async () => (posts++, m.parseAskPosted(fx.ASK_POSTED)),
  });
  await click(host.querySelector('[data-ask]'));
  assert.equal(previews, 1);
  assert.equal(posts, 0, 'nothing is posted by the preview');
  const prev = host.querySelector('.tk-jprev');
  assert.ok(prev);
  assert.ok(prev.textContent.includes('before we start TRAIL-142'));
  assert.match(prev.textContent, /sam@example\.com/);
  assert.match(prev.textContent, /Posted once\. It’s never edited or deleted\./);
  assert.match(prev.textContent, /— Ann, a kwant bot on the Shop team/);
  await click([...prev.querySelectorAll('button')].find((b) => /Cancel/.test(b.textContent)));
  assert.equal(host.querySelector('.tk-jprev'), null);
  assert.equal(posts, 0);
  await click(host.querySelector('[data-ask]'));
  await click(host.querySelector('[data-post]'));
  assert.equal(posts, 1);
});

test('picking a choice answers that question', async () => {
  const got = [];
  const host = await mount(m.TicketQuestions, { ticketKey: 'TRAIL-142', brief: B(), lead: 'Ann', askTo: 'Dana Park', onAnswer: async (id, v) => void got.push([id, v]), onDefaults: () => {} });
  await click(host.querySelector('.tk-ch'));
  assert.equal(got.length, 1);
  assert.equal(got[0][0], B().questions[0].id);
});

test('once asked, the card says it waits for the answer on Jira and offers See the comment', () => {
  const out = html(m.TicketCard, { ...N, ticket: T('needs_answer'), brief: B(fx.BRIEF_ASKED), onAskPreview: async () => fx.ASK_PREVIEW });
  assert.match(out, /Asked Dana Park on Jira at 10:04\. You can still answer here\./);
  assert.match(out, /Ann checks the Jira comment every 5 minutes\. The first answer wins, here or there\./);
  assert.doesNotMatch(out, /data-ask=/);
});

// ---------------------------------------------------------------------------------------------------
// Matches the ticket on the Review card

test('Matches replaces "Did it do what was asked?" and hides ticket findings from the list', () => {
  const card = m.parseReview(fx.REVIEW_GAPS);
  assert.ok(card, 'review fixture parses');
  assert.ok(card.matches);
  assert.deepEqual([...m.matchFindingIds(card.matches)], ['k1', 'k2', 'k3', 'k4']);
  const out = html(m.ReviewCard, { card, now: fx.TICKET_NOW, onFinding: async () => {} });
  assert.match(out, /aria-label="Matches the ticket"/);
  assert.doesNotMatch(out, /Did it do what was asked/);
  assert.match(out, /2 of 4 met/);
  assert.equal((out.match(/data-fix="/g) ?? []).length, 2, 'Fix it on the not met and unclear rows only');
  assert.equal((out.match(/data-m="ok"/g) ?? []).length, 2);
  assert.doesNotMatch(out, /TRAIL-142 says the work is done when/, 'row findings are not repeated in Findings');
  assert.match(out, /class="srcs"/, 'rows bring back their source chips');
  assert.doesNotMatch(out, /<code>tests\/export\.filters\.test\.ts<\/code>/, 'paths the sentence says are not listed again');
});

test('all met: n of n met, no Fix it', () => {
  const card = m.parseReview(fx.REVIEW_ALL_MET);
  const out = html(m.ReviewCard, { card, now: fx.TICKET_NOW, onFinding: async () => {} });
  assert.match(out, /4 of 4 met/);
  assert.doesNotMatch(out, /data-fix="/);
});

test('Fix it posts the item\'s own server id, and its state comes from the item\'s fix', async () => {
  const card = m.parseReview(fx.REVIEW_GAPS);
  const sent = [];
  const after = m.parseReview(fx.REVIEW_GAPS_SENT);
  const host = await mount(m.ReviewCard, { card, now: fx.TICKET_NOW, onFinding: async (_c, id, a) => (sent.push([id, a.action]), after) });
  await click(host.querySelector('[data-fix="k3"]'));
  assert.deepEqual(sent[0], ['k3', 'fix']);
  const row = host.querySelector('[data-item="k3"]');
  assert.ok(row.querySelector('[data-sent]'), 'the row reads Sent from matches.items[i].fix');
  assert.match(row.textContent, /Sent to Ben/);
  assert.equal(row.querySelector('[data-fix]'), null);
  assert.ok(host.querySelector('[data-fix="k4"]'), 'the unclear row still offers Fix it');
});

test('a Fix it that fails keeps its error on the item and offers Try again', async () => {
  const card = m.parseReview(fx.REVIEW_GAPS);
  const host = await mount(m.ReviewCard, { card, now: fx.TICKET_NOW, onFinding: async () => { throw new Error('the lead is offline'); } });
  await click(host.querySelector('[data-fix="k3"]'));
  const row = host.querySelector('[data-item="k3"]');
  assert.match(row.textContent, /Couldn’t send it: the lead is offline/);
  assert.match(row.querySelector('[data-fix="k3"]').textContent, /Try again/);
});

test('Dismiss on a matches row posts {action:"dismiss", reason, note} to the item\'s k{n}', async () => {
  const card = m.parseReview(fx.REVIEW_GAPS);
  const sent = [];
  const host = await mount(m.ReviewCard, { card, now: fx.TICKET_NOW, onFinding: async (_c, id, a) => (sent.push([id, a]), m.parseReview(fx.REVIEW_GAPS_DISMISSED)) });
  await click(host.querySelector('[data-dismiss="k3"]'));
  const form = host.querySelector('#dismiss-k3');
  assert.ok(form, 'the findings\' own Dismiss form opens on the row');
  assert.match(form.textContent, /Why dismiss\? Rigel learns from this\./);
  assert.equal(form.querySelectorAll('[role="radio"]').length, 4, 'the same four reasons as findings');
  assert.equal(form.querySelector('button[type="submit"]').disabled, true, 'a reason is required');
  await click(form.querySelector('[data-reason="fine_here"]'));
  const note = form.querySelector('textarea');
  const set = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set;
  set.call(note, '  Small enough for now.  ');
  note.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
  await click(form.querySelector('button[type="submit"]'));
  assert.deepEqual(sent, [['k3', { action: 'dismiss', reason: 'fine_here', note: 'Small enough for now.' }]]);
});

test('after the server answers, the dismissed row shows its Dismissed · reason line and LGTM comes back', async () => {
  const card = m.parseReview(fx.REVIEW_GAPS);
  const host = await mount(m.ReviewCard, { card, now: fx.TICKET_NOW, onFinding: async () => m.parseReview(fx.REVIEW_GAPS_DISMISSED) });
  assert.equal(host.querySelector('[data-lgtm]'), null, 'blocks_lgtm true: no LGTM yet');
  await click(host.querySelector('[data-dismiss="k3"]'));
  await click(host.querySelector('#dismiss-k3 [data-reason="fine_here"]'));
  await click(host.querySelector('#dismiss-k3 button[type="submit"]'));
  const row = host.querySelector('[data-item="k3"]');
  assert.equal(row.querySelector('[data-dismissed]').textContent, 'Dismissed · Fine in this PR · you', 'read from matches.items[i].fix');
  assert.equal(row.querySelector('[data-fix]'), null);
  assert.equal(row.querySelector('[data-dismiss]'), null);
  assert.equal(host.querySelector('#dismiss-k3'), null, 'the form closes');
  assert.ok(host.querySelector('[data-lgtm]'), 'blocks_lgtm flipped to false: LGTM is back');
  assert.ok(host.querySelector('[data-dismiss="k4"]'), 'the unclear row still offers Dismiss');
});

test('a Dismiss the server refuses keeps the form open with the error', async () => {
  const card = m.parseReview(fx.REVIEW_GAPS);
  const host = await mount(m.ReviewCard, { card, now: fx.TICKET_NOW, onFinding: async () => { throw new Error('the review moved on'); } });
  await click(host.querySelector('[data-dismiss="k4"]'));
  await click(host.querySelector('#dismiss-k4 [data-reason="wrong"]'));
  await click(host.querySelector('#dismiss-k4 button[type="submit"]'));
  assert.match(host.querySelector('#dismiss-k4 [role="alert"]').textContent, /the review moved on/);
  assert.equal(host.querySelector('[data-item="k4"] [data-dismissed]'), null);
});

test('Dismiss shows only on open not_met and unclear rows: never on met, sent or dismissed ones', () => {
  const gaps = html(m.TicketMatches, { matches: m.parseReview(fx.REVIEW_GAPS).matches, now: fx.TICKET_NOW, act: async () => {} });
  assert.deepEqual([...gaps.matchAll(/data-dismiss="(k\d+)"/g)].map((x) => x[1]), ['k3', 'k4']);
  const met = html(m.TicketMatches, { matches: m.parseReview(fx.REVIEW_ALL_MET).matches, now: fx.TICKET_NOW, act: async () => {} });
  assert.doesNotMatch(met, /data-dismiss=/, 'a met row has no Dismiss');
  const sent = html(m.TicketMatches, { matches: m.parseReview(fx.REVIEW_GAPS_SENT).matches, now: fx.TICKET_NOW, act: async () => {} });
  assert.deepEqual([...sent.matchAll(/data-dismiss="(k\d+)"/g)].map((x) => x[1]), ['k4']);
  const dis = html(m.TicketMatches, { matches: m.parseReview(fx.REVIEW_GAPS_DISMISSED).matches, now: fx.TICKET_NOW, act: async () => {} });
  assert.deepEqual([...dis.matchAll(/data-dismiss="(k\d+)"/g)].map((x) => x[1]), ['k4']);
  assert.match(dis, /Dismissed · Fine in this PR · you/);
});

test('LGTM follows blocks_lgtm: an unclear item never disables it, an open not_met does', () => {
  const ok = { verdict: 'unclear', key: 'TRAIL-142', url: null, blocks_lgtm: false };
  // Only an unclear item left open: the server says it doesn't block, so the stamp shows.
  const unclear = m.parseReview({ ...fx.REVIEW_GAPS, matches: { ...ok, items: fx.REVIEW_GAPS.matches.items.map((it) => (it.id === 'k3' ? { ...it, state: 'met', fix: null } : it)) } });
  assert.equal(unclear.matches.items.find((i) => i.id === 'k4').fix.state, 'open');
  assert.equal(m.lgtm(unclear), true);
  assert.match(html(m.ReviewCard, { card: unclear, now: fx.TICKET_NOW, onFinding: async () => {} }), /data-lgtm/);
  // An open not_met item: the server says it blocks, so no stamp.
  const notMet = m.parseReview(fx.REVIEW_GAPS);
  assert.equal(notMet.matches.blocks_lgtm, true);
  assert.equal(m.lgtm(notMet), false);
  assert.doesNotMatch(html(m.ReviewCard, { card: notMet, now: fx.TICKET_NOW, onFinding: async () => {} }), /data-lgtm/);
  // Once that fix is sent the server stops blocking; the card follows it and works nothing out itself.
  const sent = m.parseReview(fx.REVIEW_GAPS_SENT);
  assert.equal(m.lgtm(sent), true);
  // The card never derives it: a not_met item with blocks_lgtm false doesn't block, and true always does.
  assert.equal(m.lgtm({ ...notMet, matches: { ...notMet.matches, blocks_lgtm: false } }), true);
  assert.equal(m.lgtm({ ...unclear, matches: { ...unclear.matches, blocks_lgtm: true } }), false);
});

test('a review without a ticket keeps the old intent section', () => {
  const raw = { ...fx.REVIEW_GAPS, matches: null, findings: [], intent: { source: 'pr', asked: 'Export orders', none: false, rows: [{ state: 'met', text: 'Export button' }] } };
  const card = m.parseReview(raw);
  const out = html(m.ReviewCard, { card, now: fx.TICKET_NOW, onFinding: async () => {} });
  assert.doesNotMatch(out, /aria-label="Matches the ticket"/);
});

// ---------------------------------------------------------------------------------------------------
// Off means unchanged

test('Jira off (404): useTickets reports no site, so the Dev desk shows no Tickets tab', async () => {
  const calls = [];
  g.fetch = async (url) => {
    calls.push(String(url));
    return new Response(JSON.stringify({ error: 'not_found' }), { status: 404, headers: { 'Content-Type': 'application/json' } });
  };
  let seen = null;
  const Probe = () => {
    seen = m.useTickets('sp_demo', true);
    return null;
  };
  await mount(Probe, {});
  assert.equal(seen.site, null);
  assert.equal(seen.tickets.length, 0);
  assert.equal(seen.needs, 0);
  assert.deepEqual(calls, ['/api/spaces/sp_demo/dev/jira'], 'no ticket list is read when Jira is off');
});

test('Dev mode off: nothing is read at all', async () => {
  const calls = [];
  g.fetch = async (url) => (calls.push(url), new Response('{}'));
  const Probe = () => (m.useTickets('sp_demo', false), null);
  await mount(Probe, {});
  assert.equal(calls.length, 0);
});

test('TicketsPanel groups active, waiting and closed', () => {
  const tickets = [T('working'), m.parseTicket(fx.TICKET_151), m.parseTicket(fx.TICKET_148), m.parseTicket(fx.TICKET_137), m.parseTicket(fx.TICKET_139)].filter(Boolean);
  const data = { site: m.parseJiraSite(fx.JIRA_SITE), tickets, dropped: 0, loading: false, error: null, needs: m.ticketsNeedCount(tickets), reload() {} };
  const out = html(m.TicketsPanel, { space: 'sp_demo', data, names: fx.NAMES });
  assert.match(out, /From yourcompany\.atlassian\.net · TRAIL · label <code>kwant<\/code> · @Shop team/);
  assert.match(out, /closed in Jira as Won&#x27;t do · Nov 5 · branch kept/);
  assert.match(out, /data-tickets/);
});

// ---------------------------------------------------------------------------------------------------
// Rigel's round 2 (D7, D8, Off)

test('D8: every card button names its ticket, so a list of cards does not read "Stop, Stop, Skip"', async () => {
  const noop = () => {};
  const hs = { onStart: noop, onSkip: noop, onStop: noop, onRetry: noop, onCloseCard: noop, onStartBuilding: noop, onChangeBrief: noop, onOpenChat: noop, onOpenReview: noop };
  const named = (state, extra) => {
    const out = html(m.TicketCard, { ...N, ...hs, ticket: T(state, extra), brief: B(fx.BRIEF_ANSWERED) });
    return [...out.matchAll(/<button type="button" class="tk-btn[^"]*"[^>]*aria-label="([^"]+)"/g)].map((x) => x[1]);
  };
  assert.deepEqual(named('waiting'), ['Skip TRAIL-142', 'Start TRAIL-142']);
  assert.deepEqual(named('picked_up'), ['Stop TRAIL-142']);
  assert.deepEqual(named('reading'), ['Open Ann’s chat TRAIL-142', 'Stop TRAIL-142']);
  assert.deepEqual(named('failed'), ['Try again TRAIL-142']);
  assert.deepEqual(named('taken_back'), ['Close card TRAIL-142']);
  // The visible words stay as they were.
  assert.match(html(m.TicketCard, { ...N, ...hs, ticket: T('picked_up') }), />Stop<\/button>/);
  // The Brief link names the ticket too, and says which way it goes.
  const host = await mount(m.TicketCard, { ...N, ticket: T('landed'), brief: B(fx.BRIEF_ANSWERED) });
  const link = host.querySelector('[data-brief-link]');
  assert.equal(link.getAttribute('aria-label'), 'Show brief TRAIL-142');
  await click(link);
  assert.equal(host.querySelector('[data-brief-link]').getAttribute('aria-label'), 'Hide brief TRAIL-142');
});

test('D7: a ticket that waited from before says "before Work from Jira was on", not "before you connected"', () => {
  const out = html(m.TicketCard, { ...N, ticket: T('waiting', { waiting_since_connect: true, jira_created: '2026-11-03T09:00:00Z' }), onStart: () => {} });
  assert.match(out, /It was open before Work from Jira was turned on\. New tickets start on their own\./);
  assert.match(out, /open since Nov 3, before Work from Jira was on/);
  assert.doesNotMatch(out, /connected/);
});

test('Off: the site reads on:false, and the cards hide Start and Retry but keep Skip and Stop', () => {
  assert.equal(m.parseJiraSite(fx.JIRA_SITE).on, true, 'a site without `on` reads as on');
  assert.equal(m.parseJiraSite({ site: { ...fx.JIRA_SITE.site, on: false } }).on, false);
  const noop = () => {};
  const w = html(m.TicketCard, { ...N, ticket: T('waiting'), jiraOff: true, onStart: noop, onSkip: noop });
  assert.doesNotMatch(w, />Start</);
  assert.match(w, />Skip</);
  const f = html(m.TicketCard, { ...N, ticket: T('failed'), jiraOff: true, onRetry: noop });
  assert.doesNotMatch(f, />Try again</);
  const r = html(m.TicketCard, { ...N, ticket: T('reading'), jiraOff: true, onStop: noop });
  assert.match(r, />Stop</);
  // TicketsPanel passes Off down from the site.
  const tickets = [T('waiting'), T('failed', { key: 'TRAIL-150' })].filter(Boolean);
  const data = { site: m.parseJiraSite({ site: { ...fx.JIRA_SITE.site, on: false } }), tickets, dropped: 0, loading: false, error: null, needs: 0, reload() {} };
  const panel = html(m.TicketsPanel, { space: 'sp_demo', data, names: fx.NAMES });
  assert.doesNotMatch(panel, />Start</);
  assert.doesNotMatch(panel, />Try again</);
  const onData = { ...data, site: m.parseJiraSite(fx.JIRA_SITE) };
  assert.match(html(m.TicketsPanel, { space: 'sp_demo', data: onData, names: fx.NAMES }), />Start</);
});

test('Off: a 409 jira_off from Start shows the plain sentence on the card, never the raw error', async () => {
  const calls = [];
  g.fetch = async (url, init) => {
    calls.push(`${init?.method ?? 'GET'} ${url}`);
    if (String(url).endsWith('/start'))
      return new Response(JSON.stringify({ error: 'jira_off', code: 'jira_off' }), { status: 409, headers: { 'Content-Type': 'application/json' } });
    return new Response(JSON.stringify({ brief: null }), { status: 404, headers: { 'Content-Type': 'application/json' } });
  };
  // The site still read on when the list loaded; Off came in between.
  const data = { site: m.parseJiraSite(fx.JIRA_SITE), tickets: [T('waiting')], dropped: 0, loading: false, error: null, needs: 0, reload() {} };
  const host = await mount(m.TicketsPanel, { space: 'sp_demo', data, names: fx.NAMES });
  await click(host.querySelector('button[aria-label="Start TRAIL-142"]'));
  assert.ok(calls.includes('POST /api/spaces/sp_demo/dev/tickets/TRAIL-142/start'));
  const n = host.querySelector('[data-notice]');
  assert.ok(n, 'the card shows a notice');
  assert.equal(n.textContent, 'Work from Jira is off. Turn it on in Settings to start this ticket.');
  assert.equal(n.getAttribute('role'), 'alert');
  assert.doesNotMatch(host.textContent, /jira_off|HTTP 409/);
});

// ---------------------------------------------------------------------------------------------------
// 0.7.6 follow-ups: D8's last two names, D6's engine_refused

test('D8: the brief’s own toggle and "See the review" name their ticket too', async () => {
  const host = await mount(m.TicketCard, { ...N, ticket: T('brief_ready'), brief: B(fx.BRIEF_ANSWERED) });
  const toggle = host.querySelector('.tk-brief header button');
  assert.ok(toggle, 'the brief toggle');
  const was = toggle.getAttribute('aria-label');
  assert.match(was, /^(Show|Hide) brief TRAIL-142$/);
  await click(toggle);
  const now = host.querySelector('.tk-brief header button').getAttribute('aria-label');
  assert.notEqual(now, was, 'it says which way it goes');
  assert.match(now, /^(Show|Hide) brief TRAIL-142$/);
  const review = html(m.TicketCard, { ...N, ticket: T('landed', { work: { matches: 'not_met' } }), onOpenReview: () => {} });
  assert.match(review, /aria-label="See the review TRAIL-142"[^>]*>\s*See the review/);
  // Without a key (older callers) the toggle keeps its visible words as its name.
  assert.doesNotMatch(html(m.TicketBrief, { brief: B(fx.BRIEF_ANSWERED) }), /aria-label="(Show|Hide) brief/);
});

test('D6: an engine_refused card shows the server’s sentence word for word, with Try again', () => {
  const t = m.parseTicket(fx.TICKET_ENGINE_REFUSED(ENGINE_REFUSED_MESSAGE));
  assert.equal(t.error.code, 'engine_refused');
  const out = html(m.TicketCard, { ...N, ticket: t, onRetry: () => {} });
  const plain = out.replace(/<[^>]+>/g, '').replace(/&#x27;/g, "'");
  assert.ok(plain.includes(ENGINE_REFUSED_MESSAGE), 'the message, unchanged');
  assert.equal(t.error.message, ENGINE_REFUSED_MESSAGE, 'under the 300-character cap, so not cut');
  assert.match(out, /aria-label="Try again TRAIL-142"/);
  assert.doesNotMatch(plain, /Jira didn’t answer|Work on TRAIL-142 stopped/);
  // Any code the card doesn't know still shows its message as is.
  const other = m.parseTicket(fx.ticket('failed', { error: { code: 'something_new', message: 'A sentence kwant wrote.' } }));
  assert.match(html(m.TicketCard, { ...N, ticket: other }), /A sentence kwant wrote\./);
});
