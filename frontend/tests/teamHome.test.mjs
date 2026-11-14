// 13d: an empty team chat opens on a team home (name, purpose or who is in it, who answers, members, three asks),
// not a blank pane. The bot home and #general are unchanged.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'KeyboardEvent', 'MutationObserver', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'CSS']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
const store = new Map();
const ls = { getItem: (k) => (store.has(k) ? store.get(k) : null), setItem: (k, v) => void store.set(k, String(v)), removeItem: (k) => void store.delete(k) };
Object.defineProperty(globalThis, 'localStorage', { value: ls, configurable: true, writable: true });
Object.defineProperty(dom.window, 'localStorage', { value: ls, configurable: true });
dom.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
globalThis.matchMedia = dom.window.matchMedia;
globalThis.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} };
dom.window.ResizeObserver ??= globalThis.ResizeObserver;
globalThis.IntersectionObserver ??= class { observe() {} unobserve() {} disconnect() {} };
dom.window.HTMLElement.prototype.scrollIntoView ??= function () {};
dom.window.HTMLElement.prototype.scrollTo ??= function () {};
globalThis.fetch = dom.window.fetch = async () => new Response('{}', { status: 404, headers: { 'Content-Type': 'application/json' } });

const m = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { createRoot } from 'react-dom/client';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ChatView } from './src/components/ChatView';
  import { BotHome } from './src/components/BotHome';
  export * from './src/botHome';
  export const botHome = (p) => renderToStaticMarkup(React.createElement(BotHome, p));
  export const mount = (p) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    flushSync(() => root.render(React.createElement(ChatView, p)));
    return { host, unmount: () => (root.unmount(), host.remove()) };
  };
`);
const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;/g, "'").replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();

const bot = (id, name, title) => ({ id, name, title, avatar_sprite: title.toLowerCase() });
const ada = bot('a', 'Ada', 'Tech Lead');
const lin = bot('l', 'Lin', 'Backend Engineer');
const pix = bot('p', 'Pixel', 'Frontend Engineer');

test('team words: the purpose when there is one, trimmed', () => {
  assert.equal(m.teamHomeLine({ purpose: '  Ships   software safely. ', lead: 'Ada', members: [ada, lin] }), 'Ships software safely.');
});

test('team words: no purpose says who is in it and who leads', () => {
  assert.equal(m.teamHomeLine({ purpose: '', lead: 'Ada', members: [ada, lin, pix] }), '3 bots who work together. Ada leads.');
  assert.equal(m.teamHomeLine({ lead: 'Ada', members: [ada] }), '1 bot in this team. Ada leads.');
});

test('team words: no lead drops the second sentence; no members says so', () => {
  assert.equal(m.teamHomeLine({ purpose: '   ', members: [lin, pix] }), '2 bots who work together.');
  assert.equal(m.teamHomeLine({ lead: 'Ada', members: [] }), 'No bots in this team yet.');
});

test('members: lead first, each once', () => {
  assert.deepEqual(m.teamMembersLeadFirst([lin, pix, ada, lin], 'a').map((b) => b.name), ['Ada', 'Lin', 'Pixel']);
  assert.deepEqual(m.teamMembersLeadFirst([lin, pix], null).map((b) => b.name), ['Lin', 'Pixel']);
});

test('who answers: the team home reuses the composer pill words', () => {
  const t = m.answerTarget({ kind: 'team', text: '', teamName: 'engineering', names: [] });
  assert.equal(t.detail, '@engineering team answers. Type @ and a name to ask one member.');
});

const teamHome = (purpose) => ({ kind: 'team', team: { name: 'engineering', purpose, lead: 'Ada', members: [ada, lin, pix] } });
const prompts = ['review the open pull requests', 'plan the next release', 'what changed this week?', 'a fourth one'];

test('BotHome for a team: name, purpose, answer line, member chips with titles and the lead, three asks', () => {
  const html = m.botHome({ home: teamHome('Ships software safely.'), team: teamHome().team.members, prompts, canRun: true, onPick() {}, answerLine: 'ANSWERS' });
  assert.match(html, /data-bot-home="team"/);
  const t = text(html);
  assert.match(t, /engineering/);
  assert.match(t, /Ships software safely\./);
  assert.match(t, /ANSWERS/);
  assert.match(html, /aria-label="Who is in engineering"/);
  assert.match(html, /title="Tech Lead, leads the team"/);
  assert.match(html, /title="Backend Engineer"/);
  assert.equal((html.match(/data-lead/g) || []).length, 1);
  assert.match(t, /Review the open pull requests/);
  assert.match(t, /What changed this week\?/);
  assert.doesNotMatch(t, /A fourth one/, 'three asks, not four');
  assert.doesNotMatch(t, /What do you want done\?/);
});

test('BotHome for a team without a description, and while nothing can run', () => {
  const t = text(m.botHome({ home: teamHome(undefined), team: [ada, lin, pix], prompts, canRun: true, onPick() {} }));
  assert.match(t, /3 bots who work together\. Ada leads\./);
  const asleep = m.botHome({ home: teamHome('x'), team: [ada, lin, pix], prompts, canRun: false, onPick() {}, accountCard: true });
  assert.match(asleep, /data-account-card/);
  assert.doesNotMatch(text(asleep), /Review the open pull requests/);
});

test('bot home and #general words are unchanged', () => {
  const g = text(m.botHome({ home: { kind: 'general' }, team: [ada], prompts: prompts.slice(0, 3), canRun: true, onPick() {} }));
  assert.match(g, /What do you want done\?/);
  assert.match(g, /Type @ to ask one bot yourself\./);
  const b = m.botHome({ home: { kind: 'bot', bot: ada }, prompts, canRun: true, onPick() {} });
  assert.match(text(b), /Ada Tech Lead/);
  assert.match(text(b), /A fourth one/, 'a bot keeps four asks');
  assert.doesNotMatch(b, /data-lead|data-answer-line=""/);
});

const props = (messages, extra = {}) => ({
  title: '@engineering', subtitle: '', messages, streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], onSendMessage() {}, onResolveApproval() {}, onCancelDiscussion() {},
  kwants: [ada, lin, pix], teams: [], suggestions: prompts.slice(0, 3), canRun: true,
  teamName: 'engineering', teamMembers: [lin, pix], home: teamHome('Ships software safely.'), ...extra,
});

test('ChatView: an empty team chat shows the team home; with one message it does not', () => {
  const v = m.mount(props([]));
  try {
    const home = v.host.querySelector('[data-bot-home="team"]');
    assert.ok(home, 'team home shows');
    assert.equal(home.querySelector('[data-team-home-line]').textContent, 'Ships software safely.');
    assert.equal(home.querySelector('[data-answer-line]').textContent, '@engineering team answers. Type @ and a name to ask one member.');
    assert.equal(v.host.querySelector('[aria-label="Suggested prompts"] > span'), null, 'no "Suggested:" chip row under the composer');
  } finally {
    v.unmount();
  }
  const msg = { id: 'm1', sender_role: 'user', sender_id: 'u', text: 'hello team', created_at: '2026-11-14T10:00:00Z' };
  const w = m.mount(props([msg]));
  try {
    assert.equal(w.host.querySelector('[data-bot-home]'), null, 'no home once a message is in');
  } finally {
    w.unmount();
  }
});

test('ChatView: an empty team chat while nothing can run shows the account card inside the home, once', () => {
  const v = m.mount(props([], { canRun: false }));
  try {
    assert.ok(v.host.querySelector('[data-bot-home="team"] [data-account-card]'));
    assert.equal(v.host.querySelectorAll('[data-account-card]').length, 1);
  } finally {
    v.unmount();
  }
});
