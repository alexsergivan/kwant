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
