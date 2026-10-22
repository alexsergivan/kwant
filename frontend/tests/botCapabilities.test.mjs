// Alex's Mac findings (2026-10-19): Delete bot, team hires on one card, readable approval cards
// for Claude CLI built-in tools, a short tool list in Import, and an honest routing line.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ModernApprovalCard } from './src/components/ModernApprovalCard';
  import { HireCard } from './src/components/HireCard';
  import { DeleteBotDialog } from './src/components/DeleteBotDialog';
  import { PreviewView } from './src/components/ImportDialog';
  export { builtinView } from './src/components/ModernApprovalCard';
  export { parseHireMeta, hireStateText, hiredBotGone, isTeamHire } from './src/botActions';
  export { lossLines, parseUsage, deleteBlockedReason, leadsNote, botDeleteApi } from './src/botDelete';
  export { toolNote } from './src/imports';
  export { routeReasonText } from './src/routeReason';
  export { routingLine } from './src/spaces';
  const r = (C, p) => renderToStaticMarkup(React.createElement(C, p));
  export const approval = (a) => r(ModernApprovalCard, { approval: a, onResolve: () => {} });
  export const hire = (p) => r(HireCard, p);
  export const del = (p) => r(DeleteBotDialog, { onClose: () => {}, onDeleted: () => {}, ...p });
  export const preview = (p) => r(PreviewView, { skip: new Set(), onToggle: () => {}, ...p });
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/’/g, "'").replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const buttons = (html) => [...html.matchAll(/<button[^>]*>([\s\S]*?)<\/button>/g)].map((x) => text(x[1]));

// ---------------------------------------------------------------- built-in tool approvals

const bash = { id: 'b1', tool_name: 'Bash', details: { source: 'claude_builtin', tool: 'Bash', kwant_name: 'Frontend', cwd: '/Users/alex/trader', arguments: { command: 'go test ./internal/strategy/', description: 'Run the strategy tests' } } };

test('a Bash approval shows who, the command and the folder, with the three choices, and no raw JSON', () => {
  const html = m.approval(bash);
  const t = text(html);
  assert.match(t, /Frontend wants to run a command\./);
  assert.match(t, /Run the strategy tests/);
  assert.match(html, /<pre[^>]*>go test \.\/internal\/strategy\/<\/pre>/);
  assert.match(t, /In folder: \/Users\/alex\/trader/);
  assert.deepEqual(buttons(html), ['Deny', 'Allow Once', 'Always this session']);
  assert.doesNotMatch(html, /&quot;source&quot;|claude_builtin|kwant_name/);
});

test('a Write outside its folder names the file, the size and its own folder', () => {
  const html = m.approval({ id: 'w1', tool_name: 'Write', details: { source: 'claude_builtin', tool: 'Write', kwant_name: 'Frontend', cwd: '/Users/alex/bots/frontend', outside_folder: true, arguments: { file_path: '/Users/alex/trader/internal/strategy/smacross.go', content: 'package strategy\n\nfunc x() {}\n' } } });
  const t = text(html);
  assert.match(t, /Frontend wants to write a file outside its own folder\./);
  assert.match(t, /\/Users\/alex\/trader\/internal\/strategy\/smacross\.go/);
  assert.match(t, /Write 4 lines/);
  assert.match(t, /Its folder: \/Users\/alex\/bots\/frontend/);
  assert.doesNotMatch(t, /package strategy/, 'file contents are not dumped on the card');
});

test('built-ins are recognised by tool name alone, with the CLI input field', () => {
  const v = m.builtinView({ id: 'e', tool_name: 'Edit', details: { input: { file_path: '/a.go', old_string: 'a', new_string: 'b' } } });
  assert.equal(v.kind, 'file');
  assert.equal(v.path, '/a.go');
  assert.equal(v.change, 'Replace text');
  assert.equal(m.builtinView({ id: 'x', tool_name: 'write_file', details: {} }), null, 'unknown tools keep the old card');
  assert.equal(m.builtinView({ id: 'x', tool_name: 'Bash', details: null }).command, '');
});

// ---------------------------------------------------------------- team hires

const hireMeta = (over = {}) => ({
  kind: 'hire_proposal', hire_id: 'h1', kwant_id: 'k1', proposer_name: 'Backend', name: 'Frontend', role: 'Frontend Engineer',
  instructions: 'Own the UI.', model: 'auto', tools: [], team: null, reason: 'You asked for a professional engineering team.',
  evidence: null, requested_by_person: true, status: 'pending', auto: false, choices: ['deny', 'accept_once', 'always_accept'],
  hired_kwant_id: null, action_id: null, can_undo: false, policy: 'ask', note: null, created_at: '2026-10-19T10:00:00Z', resolved_at: null,
  members: [
    { name: 'Frontend', role: 'Frontend Engineer', model: 'auto', tools: [], instructions: 'Own the UI.' },
    { name: 'QA', role: 'QA / Test Engineer', model: 'claude-sonnet-5-5', tools: [{ id: 's', name: 'Browser' }], instructions: 'Test it.' },
    { name: 'Lead', role: 'Tech Lead', model: null, tools: [], instructions: '' },
  ],
  new_team: { name: 'Engineering' },
  ...over,
});
const hireProps = (meta) => ({ meta: m.parseHireMeta(meta), messageId: 'm1', kwantName: 'Backend', kwantId: 'k1', onDecide: async () => {} });

test('a team proposal lists every member with role and model, and asks once', () => {
  const html = m.hire(hireProps(hireMeta()));
  const t = text(html);
  assert.match(t, /Backend proposes hiring a team of 3/);
  assert.match(t, /Engineering/);
  for (const s of ['Frontend Lead Frontend Engineer Picked automatically', 'QA QA \/ Test Engineer claude-sonnet-5-5 · Tools: Browser', 'Lead Tech Lead Picked automatically']) assert.match(t, new RegExp(s));
  assert.match(t, /New team: Engineering/);
  assert.deepEqual(buttons(html), ['Show their instructions', 'Approve all 3', 'Always accept', 'Decline']);
});

test('a single hire still reads as before; old metas without members parse as one', () => {
  const one = m.parseHireMeta(hireMeta({ members: undefined, new_team: undefined }));
  assert.equal(one.members.length, 1);
  assert.equal(m.isTeamHire(one), false);
  const t = text(m.hire(hireProps(hireMeta({ members: undefined, new_team: undefined }))));
  assert.match(t, /proposes hiring a new bot/);
  assert.match(t, /Accept once/);
});

test('team state lines name everyone; gone only when every hired bot is gone', () => {
  const h = m.parseHireMeta(hireMeta({ status: 'accepted', hired_kwant_ids: ['a', 'b', 'c'] }));
  assert.equal(m.hireStateText(h).label, 'Hired. Frontend, QA and Lead joined Engineering.');
  assert.equal(m.hiredBotGone(h, new Set(['b'])), false);
  assert.equal(m.hiredBotGone(h, new Set()), true);
});

// ---------------------------------------------------------------- delete bot

test('the delete confirm names what goes, offers Export, and starts safe', () => {
  const html = m.del({ kwant: { id: 'k1', name: 'Frontend' } });
  const t = text(html);
  assert.match(html, /role="alertdialog"/);
  assert.match(t, /Delete Frontend\?/);
  assert.match(t, /can't be undone/);
  for (const s of ['conversations', 'memory', 'skills', 'routines']) assert.match(t, new RegExp(s));
  assert.deepEqual(buttons(html), ['Export Frontend', 'Cancel', 'Delete Frontend']);
});

test('Delete is off with a reason while the bot works', () => {
  const html = m.del({ kwant: { id: 'k1', name: 'Frontend' }, busy: 'Working on #general' });
  assert.match(text(html), /Frontend is working right now\. Stop it or wait until it finishes, then delete it\./);
  assert.match(html, /<button[^>]*disabled=""[^>]*>[\s\S]*?Delete Frontend<\/button>/);
  assert.equal(m.deleteBlockedReason('X', null, m.parseUsage({ running: true })), 'X is working right now. Stop it or wait until it finishes, then delete it.');
  assert.equal(m.deleteBlockedReason('X', null, null), null);
});

test('loss lines use counts when the server gives them', () => {
  assert.deepEqual(m.lossLines(m.parseUsage({ conversations: 12, memories: 1, skills: 0, routines: 2 })), ['12 conversations', '1 memory', '0 skills', '2 routines']);
  assert.equal(m.parseUsage(null), null);
  assert.equal(m.lossLines(null).length, 4);
});

// ---------------------------------------------------------------- import tool list

const LONG = 'It used tools (mcp__agents__coordinate_bots, mcp__agents__memory_update, mcp__agents__session_search, mcp__browser__agent_browser_open, mcp__browser__agent_browser_click, mcp__claude_ai_Notion__notion-search, mcp__claude_ai_Gmail__send, mcp__composio__COMPOSIO_SEARCH_TOOLS, mcp__claude_ai_Canva__create-design, WebSearch, Bash). Imported bots start with no tools; connect the ones it needs.';

test('a long tool notice groups by app', () => {
  const n = m.toolNote(LONG);
  assert.deepEqual(n.apps.map((a) => a.name), ['Agents', 'Browser', 'Notion', 'Gmail', 'Composio', 'Canva', 'WebSearch', 'Bash']);
  assert.deepEqual(n.apps[0].tools.length, 3);
  assert.equal(n.tail, 'Imported bots start with no tools; connect the ones it needs.');
  assert.equal(m.toolNote('Routine X: paused.'), null);
});

test('the preview shows five app names and "Show all", not every tool', () => {
  const p = {
    bundle: { format: 'kwant.bundle/1', source: { kind: 'openmausbot' }, bots: [{ ref: 'b1', name: 'Personal assistant', persona_prompt: 'Helps.' }] },
    warnings: [{ bot: 'b1', message: LONG }], dropped: [], problems: [], renames: [], can_apply: true,
  };
  const html = m.preview({ preview: p });
  const t = text(html);
  assert.match(t, /It used tools from 8 apps: Agents, Browser, Notion, Gmail, Composio and 3 more\./);
  assert.match(html, /aria-expanded="false"[^>]*>Show all</);
  assert.doesNotMatch(t, /coordinate_bots/);
});

test('a one-app tool note reads as a sentence', () => {
  assert.match(text(m.preview({ preview: { bundle: { format: 'kwant.bundle/1', source: { kind: 'openmausbot' }, bots: [{ ref: 'b1', name: 'A', persona_prompt: '' }] }, warnings: [{ bot: 'b1', message: 'It used tools (mcp__claude_ai_Notion__notion-search). Connect it.' }], dropped: [], problems: [], renames: [], can_apply: true } })), /It used tools from Notion\. Connect it\./);
});

// ---------------------------------------------------------------- routing

test('the routing line says when the team lead took it because no role matched', () => {
  assert.equal(m.routeReasonText({ kwantName: 'Ada', basis: 'lead', reason: 'Laya picked Ada (Tech Lead) as the best fit for this question' }), 'No one’s role matched this, so it went to the team lead, Ada.');
  assert.equal(m.routeReasonText({ kwantName: 'Lin', reason: 'Laya picked Lin (Backend) as the best fit' }), 'Laya picked Lin (Backend) as the best fit');
  assert.equal(m.routeReasonText({ basis: 'keywords', matchedTerms: ['strategy', 'go'] }), 'Picked by matching words: strategy, go.');
  assert.equal(m.routeReasonText({ kwantName: 'FitnessCoach', basis: 'team' }), 'This asks to put a team of bots together, so it went to the lead, FitnessCoach, to propose them for you to approve.');
  assert.match(m.routingLine({ kwant_name: 'Ada', basis: 'lead', candidate_count: 4 }, 'Work'), /^No one’s role matched, so the team lead Ada took it from 4 bots in Work$/);
});

// ---------------------------------------------------------------- shapes Lin's server sends (bot-capabilities)

test('a team proposal as the server sends it: members once, no Always accept, and the lead marked', () => {
  const html = m.hire(hireProps(hireMeta({ choices: ['deny', 'accept_once'], new_team: { name: 'Trading', lead: 'QA' } })));
  const t = text(html);
  assert.equal((t.match(/Frontend Engineer/g) ?? []).length, 1, 'the top-level bot is members[0], not an extra row');
  assert.deepEqual(buttons(html), ['Show their instructions', 'Approve all 3', 'Decline']);
  assert.match(t, /QA Lead QA \/ Test Engineer/);
  assert.match(t, /New team: Trading, led by QA/);
  assert.match(text(m.hire(hireProps(hireMeta({ new_team: { name: 'Trading', lead: null } })))), /led by Frontend/, 'no lead named: the first member leads');
});

test('the routing line and card use the server’s own reason when it sends one', () => {
  const several = { kwant_name: 'Gopher', basis: 'several', fallback: true, candidate_count: 3, matched_terms: ['endpoint', 'page'],
    reason: 'This touches several roles (Gopher and Pixel), so it went to the team lead, Gopher (Go backend), to answer or hand parts on. Laya is offline, so this was picked by keywords' };
  const line = m.routingLine(several, 'Trading');
  assert.match(line, /^This touches several roles \(Gopher and Pixel\).*picked by keywords\. Chosen from 3 bots in Trading\.$/);
  assert.doesNotMatch(line, /Laya picked/);
  const named = { kwantName: 'Probe', basis: 'keywords', matchedTerms: [], reason: 'Probe (QA) was named in your message' };
  assert.equal(m.routeReasonText(named), 'Probe (QA) was named in your message.');
  assert.match(m.routingLine({ kwant_name: 'Probe', basis: 'keywords', matched_terms: [], reason: named.reason }, 'Trading'), /^Probe \(QA\) was named in your message\. Chosen in Trading\.$/);
  assert.match(m.routingLine({ kwant_name: 'Lin', reason: 'Laya picked Lin (Backend) as the best fit', candidate_count: 2 }, 'Work'), /^Laya picked Lin from 2 bots in Work$/, 'older servers keep the old line');
});

// One row per routing basis, with the sentence src/router/team.rs (and routes.rs for discuss) actually sends.
const OFFLINE = 'Laya is offline, so this was a keyword match';
const ROUTES = [
  { case: 'named', basis: 'keywords', name: 'Probe', terms: ['probe'], reason: 'Probe (QA) was named in your message' },
  { case: 'keywords', basis: 'keywords', name: 'Gopher', terms: ['strategy', 'go'], reason: `Gopher (Go backend) does this kind of work: your message mentions strategy, go. ${OFFLINE}` },
  { case: 'lead-unmatched', basis: 'lead', name: 'Ada', terms: [], reason: "No one's role matched this, so it went to the channel lead, Ada (Tech Lead)" },
  { case: 'lead-multi-role', basis: 'several', name: 'Gopher', terms: ['endpoint', 'page'], reason: `This touches several roles (Gopher and Pixel), so it went to the channel lead, Gopher (Go backend), to answer or hand parts on. ${OFFLINE}` },
  { case: 'only-bot', basis: 'only', name: 'Lin', terms: [], reason: 'Lin is the only bot here that can answer' },
  { case: 'laya', basis: 'skills', name: 'Lin', terms: ['migration'], reason: 'Lin (Backend) does this kind of work: your message mentions migration; Laya picked the same bot' },
  { case: 'laya-titles', basis: 'skills', name: 'Lin', terms: [], reason: "Laya matched this to Lin's role (Backend); no words in your message named a role" },
  { case: 'team-building', basis: 'team', name: 'FitnessCoach', terms: [], reason: 'This asks to put a team of bots together, so it went to the channel lead, FitnessCoach (Fitness Trainer), to propose them on one hire card for you to approve' },
  { case: 'discuss', basis: 'discuss', name: 'Ada', terms: [], reason: 'This needs several perspectives — starting a 4-turn team discussion led by Ada' },
];

for (const r of ROUTES) {
  test(`routing basis ${r.case}: card and line show the server’s reason`, () => {
    const sentence = /[.!?]$/.test(r.reason) ? r.reason : `${r.reason}.`;
    assert.equal(m.routeReasonText({ kwantName: r.name, basis: r.basis, matchedTerms: r.terms, reason: r.reason }), sentence);
    const line = m.routingLine({ kwant_name: r.name, basis: r.basis, matched_terms: r.terms, reason: r.reason, candidate_count: 3, fallback: r.reason.includes(OFFLINE) }, 'Trading');
    assert.equal(line, `${sentence} Chosen from 3 bots in Trading.`);
    assert.doesNotMatch(line, /^Laya picked|Picked by keywords \(Laya is offline\)/, 'no local wording over the server’s');
  });
}

test('delete dialog reads Lin’s real usage shape and both 409 codes', async () => {
  const usage = m.parseUsage({ conversations: 3, messages: 41, memories: 2, skills: 1, routines: 0, leads: 1, running: false });
  assert.deepEqual(m.lossLines(usage), ['3 conversations (41 messages)', '2 memories', '1 skill', '0 routines']);
  assert.equal(m.leadsNote(usage), 'It leads 1 team or channel; the next member takes over as lead.');
  assert.equal(m.leadsNote(m.parseUsage({ leads: 0 })), null);
  const real = globalThis.fetch;
  const reply = (status, body) => { globalThis.fetch = async () => new Response(JSON.stringify(body), { status }); };
  try {
    reply(409, { error: 'last_bot', message: 'Probe is the only bot in this space; a space needs at least one.' });
    assert.equal(await m.botDeleteApi.remove('k1', 'Probe'), 'Probe is the only bot in this space; a space needs at least one.');
    reply(409, { error: 'last_bot' });
    assert.match(await m.botDeleteApi.remove('k1', 'Probe'), /only bot in this space/);
    reply(409, { error: 'running' });
    assert.match(await m.botDeleteApi.remove('k1', 'Probe'), /working right now/);
    reply(200, { status: 'deleted', deleted: 'k1', name: 'Probe', removed: {}, kept_folder: null });
    assert.equal(await m.botDeleteApi.remove('k1', 'Probe'), null);
    reply(404, { error: 'bot not found' });
    assert.equal(await m.botDeleteApi.remove('k1', 'Probe'), null, 'already gone counts as deleted');
  } finally {
    globalThis.fetch = real;
  }
});
