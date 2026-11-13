import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

const load = async (path) => {
  const source = await readFile(new URL(path, import.meta.url), 'utf8');
  const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } });
  return import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
};

const { choosePage, shownPage, stepPage, pageLabel, pageOwner, pageWhere, parsePageList, pageTeam, notifyBrowserPages, onBrowserPagesChanged } =
  await load('../src/browserPages.ts');
const { browserStreamUrl } = await load('../src/browserViewport.ts');

const TEAM = 'team-1';
const ARCHITECT = 'team-1#architect#chat-a';
const COMMUNITY = 'team-1#community#chat-c';
const all = [COMMUNITY, ARCHITECT, TEAM];

test('choosePage: the page the person picked wins', () => {
  assert.equal(choosePage({ picked: COMMUNITY, stepPage: ARCHITECT, pages: all }), COMMUNITY);
});

test('choosePage: with no pick, the open step\'s page', () => {
  assert.equal(choosePage({ picked: null, stepPage: ARCHITECT, pages: all }), ARCHITECT);
});

test('choosePage: with no pick and no step, the server default (no ?page=)', () => {
  assert.equal(choosePage({ picked: null, stepPage: null, pages: all }), null);
});

test('choosePage: a picked page that closed falls back to the step page, then the default', () => {
  const left = [ARCHITECT, TEAM];
  assert.equal(choosePage({ picked: COMMUNITY, stepPage: ARCHITECT, pages: left }), ARCHITECT);
  assert.equal(choosePage({ picked: COMMUNITY, stepPage: null, pages: left }), null);
  // a step on a page that is gone is no reason to ask for it either
  assert.equal(choosePage({ picked: null, stepPage: COMMUNITY, pages: left }), null);
});

test('choosePage: a new step never pulls the view off a picked page', () => {
  const before = choosePage({ picked: COMMUNITY, stepPage: null, pages: all });
  const after = choosePage({ picked: COMMUNITY, stepPage: ARCHITECT, pages: all });
  assert.equal(before, COMMUNITY);
  assert.equal(after, COMMUNITY);
});

test('choosePage: before the first list loads, a pick or step page is trusted as is', () => {
  assert.equal(choosePage({ picked: ARCHITECT, stepPage: null, pages: null }), ARCHITECT);
  assert.equal(choosePage({ picked: null, stepPage: ARCHITECT, pages: null }), ARCHITECT);
  assert.equal(choosePage({ picked: null, stepPage: null, pages: null }), null);
});

test('shownPage: the chosen page, else the server\'s live page, else the team page', () => {
  assert.equal(shownPage(ARCHITECT, { live: COMMUNITY }, TEAM), ARCHITECT);
  assert.equal(shownPage(null, { live: COMMUNITY }, TEAM), COMMUNITY);
  assert.equal(shownPage(null, null, TEAM), TEAM);
});

test('stepPage: its own page, else (an older step) the team page', () => {
  assert.equal(stepPage({ page_key: ARCHITECT, sandbox_target: TEAM }), ARCHITECT);
  assert.equal(stepPage({ page_key: null, sandbox_target: TEAM }), TEAM);
  assert.equal(stepPage({ sandbox_target: TEAM }), TEAM);
  assert.equal(stepPage(null), null);
  assert.equal(pageTeam(ARCHITECT), TEAM);
  assert.equal(pageTeam(TEAM), TEAM);
});

test('tab words: bot and site, "Shared" for the team page, and the waiting mark', () => {
  const names = { architect: 'Architect' };
  const name = (id) => names[id];
  const held = { page: ARCHITECT, team_id: TEAM, kwant_id: 'architect', chat_id: 'chat-a', job: true, url: 'https://www.tiktok.com/login', title: 'Log in | TikTok', held: true, idle_seconds: 3 };
  assert.equal(pageLabel(held, name), "Architect's page: tiktok.com (needs you)");
  const team = { ...held, page: TEAM, kwant_id: null, chat_id: null, job: false, url: 'about:blank', title: '', held: false };
  assert.equal(pageOwner(team, name), 'Shared');
  assert.equal(pageLabel(team, name), 'Shared page: nothing open');
  assert.equal(pageOwner({ job: true, kwant_id: 'gone' }, name), 'A bot');
  assert.equal(pageWhere({ url: null, title: 'Flow' }), 'Flow');
  assert.equal(pageWhere({ url: 'about:blank', title: 'about:blank' }), 'nothing open');
});

test('parsePageList: keeps the server\'s list and drops what it cannot read', () => {
  assert.equal(parsePageList(null), null);
  assert.equal(parsePageList({ error: 'Not found' }), null);
  const list = parsePageList({
    team_id: TEAM,
    live: ARCHITECT,
    max_job_pages: 4,
    pages: [{ page: ARCHITECT, kwant_id: 'architect', chat_id: 'chat-a', job: true, url: 'https://tiktok.com', title: 'TikTok', held: true, idle_seconds: 1 }, { nope: 1 }, { page: TEAM, kwant_id: null, job: false }],
  });
  assert.equal(list.live, ARCHITECT);
  assert.deepEqual(list.pages.map((p) => [p.page, p.held, p.job]), [[ARCHITECT, true, true], [TEAM, false, false]]);
});

test('notifyBrowserPages: a page key wakes its team\'s strip', () => {
  const heard = [];
  const off = onBrowserPagesChanged((t) => heard.push(t));
  notifyBrowserPages(ARCHITECT);
  notifyBrowserPages(TEAM);
  notifyBrowserPages(null);
  off();
  notifyBrowserPages(TEAM);
  assert.deepEqual(heard, [TEAM, TEAM]);
});

test('browserStreamUrl: ?page= with the # of a job page encoded', () => {
  const loc = { protocol: 'https:', host: 'bots.example.com' };
  assert.equal(browserStreamUrl(loc, TEAM), 'wss://bots.example.com/api/sandbox/team-1/stream');
  assert.equal(browserStreamUrl(loc, TEAM, null), 'wss://bots.example.com/api/sandbox/team-1/stream');
  assert.equal(browserStreamUrl(loc, TEAM, ARCHITECT), 'wss://bots.example.com/api/sandbox/team-1/stream?page=team-1%23architect%23chat-a');
});

test('the pane picks the page by choosePage and streams it; the card\'s Sign in / Solve sends the step\'s page', async () => {
  const pane = await readFile(new URL('../src/components/VirtualMachinePane.tsx', import.meta.url), 'utf8');
  assert.match(pane, /choosePage\(\{ picked: picked\?\.page \?\? null, stepPage: stepPage\(openStep\), pages: knownPages \}\)/);
  assert.match(pane, /useBrowserStream\([^)]*, chosenPage\)/);
  assert.match(pane, /stream\.page !== chosenPage/, 'Solve takes control only once the chosen page is on screen');
  const card = await readFile(new URL('../src/components/HumanStepCard.tsx', import.meta.url), 'utf8');
  assert.equal((card.match(/onOpenBrowser\(view\.sandbox_target, true, view\.page_key \?\? view\.sandbox_target\)/g) || []).length, 2);
  const hook = await readFile(new URL('../src/useBrowserStream.ts', import.meta.url), 'utf8');
  assert.match(hook, /\}, \[id, visible, page\]\);/, 'a new page reconnects the stream');
});

// ---------- 0.9.8 browser panel: the chip and the page trail ----------
const bp = await load('../src/browserPages.ts');
const step = (tool, detail, at, extra = {}) => ({ kind: 'tool', status: 'done', tool, detail, started_at: at, ...extra });

test('pagesOfReply: counts the bot browser\'s own open/navigate tools, MCP-prefixed too', () => {
  bp.clearTitles();
  const pages = bp.pagesOfReply([
    step('mcp__kwnt__browser_open', 'https://www.roastco.example.com/shop/coffee/1kg', '2026-11-13T10:44:00Z'),
    step('browser_navigate', 'https://roastco.example.com/cart', '2026-11-13T10:45:00Z'),
    step('mcp__kwnt__open_url', 'https://beans.example.org/', '2026-11-13T10:46:00Z'),
  ]);
  assert.deepEqual(pages.map((p) => p.url), ['https://beans.example.org/', 'https://roastco.example.com/cart', 'https://www.roastco.example.com/shop/coffee/1kg']);
  assert.deepEqual(pages.map((p) => p.host), ['beans.example.org', 'roastco.example.com', 'roastco.example.com']);
});

test('pagesOfReply: WebFetch, fetch, failed steps, non-tool steps and non-http details don\'t count', () => {
  assert.deepEqual(
    bp.pagesOfReply([
      step('WebFetch', 'https://a.example.com/', 't1'),
      step('mcp__kwnt__webfetch', 'https://a.example.com/', 't1'),
      step('fetch', 'https://a.example.com/', 't1'),
      step('browser_open', 'https://a.example.com/', 't1', { status: 'failed' }),
      step('browser_open', 'Timed out loading the page', 't1'),
      step('browser_open', 'file:///etc/hosts', 't1'),
      { kind: 'thinking', detail: 'https://a.example.com/', started_at: 't1' },
      step('browser_click', 'https://a.example.com/', 't1'),
    ]),
    [],
  );
  assert.deepEqual(bp.pagesOfReply(null), []);
});

test('pagesOfReply: distinct addresses with the hash stripped; a later visit moves the page up', () => {
  const pages = bp.pagesOfReply([
    step('browser_open', 'https://a.example.com/x#top', '2026-11-13T10:00:00Z'),
    step('browser_open', 'https://a.example.com/y', '2026-11-13T10:01:00Z'),
    step('browser_open', 'https://a.example.com/x#reviews', '2026-11-13T10:02:00Z'),
  ]);
  assert.deepEqual(pages.map((p) => [p.url, p.at]), [
    ['https://a.example.com/x', '2026-11-13T10:02:00Z'],
    ['https://a.example.com/y', '2026-11-13T10:01:00Z'],
  ]);
});

test('page titles: the step\'s page_title, then one seen live, then the path; "/" reads as the host', () => {
  bp.clearTitles();
  bp.noteTitle('https://roastco.example.com/cart#x', 'Your cart · RoastCo');
  const pages = bp.pagesOfReply([
    step('browser_open', 'https://roastco.example.com/', 't1'),
    step('browser_open', 'https://roastco.example.com/shop/coffee/1kg', 't2'),
    step('browser_open', 'https://roastco.example.com/cart', 't3'),
    step('browser_open', 'https://roastco.example.com/about', 't4', { page_title: 'About RoastCo' }),
  ]);
  assert.deepEqual(pages.map((p) => bp.pageTitleOf(p.url, p.stepTitle)), ['About RoastCo', 'Your cart · RoastCo', '/shop/coffee/1kg', 'roastco.example.com']);
});

test('a title noted after pagesOfReply ran still shows through pageTitleOf (memoized trails)', () => {
  bp.clearTitles();
  const pages = bp.pagesOfReply([step('browser_open', 'https://roastco.example.com/cart', 't1')]);
  assert.equal(pages[0].stepTitle, null);
  assert.equal(bp.pageTitleOf(pages[0].url, pages[0].stepTitle), '/cart');
  bp.noteTitle('https://roastco.example.com/cart', 'Your cart · RoastCo');
  assert.equal(bp.pageTitleOf(pages[0].url, pages[0].stepTitle), 'Your cart · RoastCo');
  // A step's own page_title still wins over a live one
  const own = bp.pagesOfReply([step('browser_open', 'https://roastco.example.com/cart', 't2', { page_title: 'Cart' })]);
  assert.equal(bp.pageTitleOf(own[0].url, own[0].stepTitle), 'Cart');
});

test('noteTitle keeps the newest 200 addresses and ignores blank titles', () => {
  bp.clearTitles();
  for (let i = 0; i < 205; i++) bp.noteTitle(`https://a.example.com/${i}`, `Page ${i}`);
  assert.equal(bp.titleFor('https://a.example.com/0'), null);
  assert.equal(bp.titleFor('https://a.example.com/204'), 'Page 204');
  bp.noteTitle('https://b.example.com/', '  ');
  bp.noteTitle('about:blank', 'Blank');
  assert.equal(bp.titleFor('https://b.example.com/'), null);
});

test('the chip\'s words: "1 page" / "2 pages" after, the host and a count from 2 while browsing', () => {
  const one = bp.pagesOfReply([step('browser_open', 'https://roastco.example.com/', 't1')]);
  const two = bp.pagesOfReply([step('browser_open', 'https://roastco.example.com/', 't1'), step('browser_open', 'https://www.beans.example.org/a', 't2')]);
  assert.equal(bp.browsedChipText('Ben', one, false).lead, 'Ben opened 1 page');
  assert.equal(bp.browsedChipText('Ben', one, false).label, 'Show the page Ben opened');
  assert.equal(bp.browsedChipText('Ben', two, false).lead, 'Ben opened 2 pages');
  assert.equal(bp.browsedChipText('Ben', two, false).label, 'Show the 2 pages Ben opened');
  assert.deepEqual(bp.browsedChipText('Ben', one, true), { lead: 'Ben is on', host: 'roastco.example.com', count: null, label: "Watch Ben's browser" });
  assert.deepEqual(bp.browsedChipText('Ben', two, true), { lead: 'Ben is on', host: 'beans.example.org', count: '2 pages', label: "Watch Ben's browser" });
});
