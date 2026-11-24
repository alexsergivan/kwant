// 36a one-status-steps (autonomy/one-status-steps/brief.md; Mira's simplify-review rank 2): a failed step the bot went on
// past reads a grey "Retried", not a rose "Failed"; rose only when the turn ends on it. The summary says "· 1 retried".
// A shell step is titled in words ("Running a command on 203.0.113.66") and its raw command sits behind a real
// "Show command" button. A screen reader hears the same words the eye sees.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'KeyboardEvent', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
g.matchMedia = g.window.matchMedia;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { LiveActivityList, SavedActivityList } from './src/components/ActivityList';
  export { LiveActivityList, SavedActivityList };
  export * from './src/activity';
  export { plainShellTitle, shellCommandText, sshHost, shellCommandOf, stepsWho } from './src/toolLabel';
  export const live = (props) => renderToStaticMarkup(React.createElement(LiveActivityList, props));
  export const saved = (props) => renderToStaticMarkup(React.createElement(SavedActivityList, props));
`);

const SSH = "ssh -o ConnectTimeout=30 root@203.0.113.66 'cd /opt/tradebot && cp -p data/trend-state.json data/trend-state.bak'";
const titleOf = (cmd) => `Running \`${cmd.length > 80 ? `${cmd.slice(0, 80)}…` : cmd}\``;
const at = (sec) => `2026-11-24T16:50:${String(sec).padStart(2, '0')}Z`;
const sshFailed = { id: 's1', kind: 'tool', tool: 'Bash', status: 'failed', title: titleOf(SSH), detail: 'ssh: connect to host 203.0.113.66 port 22: Connection timed out', started_at: at(0), duration_ms: 4300 };
const sshRunning = { id: 's2', kind: 'tool', tool: 'Bash', status: 'running', title: titleOf(SSH), detail: SSH, started_at: at(5) };
const sshOk = { ...sshRunning, status: 'ok', duration_ms: 2100 };
const thought = { id: 's3', kind: 'thinking', status: 'done', title: 'Thought', text: 'It timed out; I will tell them.', started_at: at(9), duration_ms: 800 };
const text = (html) => html.replace(/<[^>]+>/g, '');

test('retriedSteps: live, every failed step; saved, only one the bot went on past (a later tool, approval, human step or question)', () => {
  assert.deepEqual([...m.retriedSteps([sshFailed, sshRunning], true)], ['s1']);
  assert.deepEqual([...m.retriedSteps([sshFailed], true)], ['s1'], 'the turn still runs, so it has not ended on it');
  assert.deepEqual([...m.retriedSteps([sshFailed, sshOk], false)], ['s1']);
  assert.deepEqual([...m.retriedSteps([sshFailed], false)], [], 'ended on it');
  assert.deepEqual([...m.retriedSteps([sshFailed, thought], false)], [], 'thinking after it is not carrying on');
  const note = { id: 'n', kind: 'note', status: 'ok', title: 'This engine hides its thinking', started_at: at(8) };
  assert.deepEqual([...m.retriedSteps([sshFailed, note], false)], [], 'nor is a note');
  const second = { ...sshFailed, id: 's9' };
  assert.deepEqual([...m.retriedSteps([sshFailed, second], false)], ['s1'], 'two failures: the last one is where it ended');
  assert.equal(m.statusLabel(sshFailed, true), 'Retried');
  assert.equal(m.statusLabel(sshFailed), 'Failed');
  assert.equal(m.shownStatus(sshFailed, true), 'retried');
  assert.equal(m.shownStatus(sshOk, true), 'ok', 'only a failed step can be retried');
});

test('(a) live: the recovered step shows a grey "Retried" chip and icon label, no rose, and the title says what happened', () => {
  const html = m.live({ live: { turnId: 't', steps: [sshFailed, sshRunning], thinking: {} }, idBase: 'l', who: 'Quant' });
  assert.match(html, /data-step-mark="retried"[^>]*>Retried</);
  assert.match(html, /data-step-mark="retried" class="[^"]*text-zinc-300 border-hairline-strong"/, 'Mira: zinc-300, hairline-strong, no fill');
  assert.doesNotMatch(html, /data-step-mark="retried" class="[^"]*\bbg-/, 'no colour fill');
  assert.match(html, /role="img" aria-label="Retried"/, 'a screen reader hears "Retried"');
  assert.doesNotMatch(html, />Failed<|aria-label="Failed"|text-rose|border-rose|bg-rose/, 'nothing rose while the bot is fine');
  assert.doesNotMatch(html, /holo/i, 'never holo');
  assert.match(text(html), /A command on 203\.0\.113\.66 didn’t connect, so Quant tried again/);
  assert.match(text(html), /Working ?· 2 steps · 1 retried/);
});

test('(a) saved: "Retried" when the bot went on, rose "Failed" when the turn ended on the step', () => {
  const went = m.saved({ activity: { steps: [sshFailed, sshOk], dropped: 0 }, idBase: 'r', defaultOpen: true, who: 'Quant' });
  assert.match(went, /data-step-mark="retried"[^>]*>Retried</);
  assert.doesNotMatch(went, />Failed<|text-rose/);
  assert.match(text(went), /2 steps · 1 retried/);
  const ended = m.saved({ activity: { steps: [sshFailed], dropped: 0 }, idBase: 'r', defaultOpen: true, who: 'Quant' });
  assert.match(ended, /data-step-mark="failed"[^>]*>Failed</);
  assert.match(ended, /role="img" aria-label="Failed"/);
  assert.match(ended, /lucide-list-tree[^"]*text-rose-300/, 'the summary icon is rose only here');
  assert.match(text(ended), /1 step · 1 failed/);
  assert.doesNotMatch(text(ended), /retried|tried again/);
  assert.match(text(ended), /Running a command on 203\.0\.113\.66/);
  assert.equal(m.activitySummary({ steps: [sshFailed, sshOk, { ...sshFailed, id: 's4' }], dropped: 0 }), 'Worked 0:07 · 3 steps · 1 failed · 1 retried');
});

test('(b) the phone row reads "Steps · 2 · 1 retried", with no red mark', () => {
  const html = m.live({ live: { turnId: 't', steps: [sshFailed, sshRunning], thinking: {} }, idBase: 'p', compact: true, who: 'Quant' });
  assert.match(text(html), /Steps · 2 · 1 retried/);
  assert.doesNotMatch(html, /rose/);
  const clean = m.live({ live: { turnId: 't', steps: [sshRunning], thinking: {} }, idBase: 'p', compact: true });
  assert.doesNotMatch(text(clean), /retried/);
});

test('(c) shell titles in words: ssh names its host; other commands say "Running a command"; non-shell steps unchanged', () => {
  assert.equal(m.sshHost(SSH), '203.0.113.66');
  assert.equal(m.sshHost('ssh -p 2222 -i ~/.ssh/k deploy@box.example.com uptime'), 'box.example.com');
  assert.equal(m.sshHost('ssh -tt ops@[2001:db8::1]:22 ls'), '2001:db8::1');
  assert.equal(m.sshHost('/usr/bin/ssh host-1 true'), 'host-1');
  assert.equal(m.sshHost('ssh -o ConnectTimeout=30'), null, 'no host');
  assert.equal(m.sshHost('scp a b'), null);
  assert.equal(m.sshHost('ssh "$HOST" ls'), null, 'not a plain host: words only');
  assert.deepEqual(m.plainShellTitle({ title: titleOf(SSH) }), { text: 'Running a command on 203.0.113.66', host: '203.0.113.66' });
  assert.deepEqual(m.plainShellTitle({ title: 'Running `cargo test -q`' }), { text: 'Running a command', host: null });
  assert.equal(m.plainShellTitle({ title: 'Running `npm test`', detail: 'exit 1' }, true).text, 'A command didn’t work, so the bot tried again');
  assert.equal(m.plainShellTitle({ title: 'Opening github.com' }), null);
  const html = m.saved({ activity: { steps: [{ id: 'w', kind: 'tool', tool: 'WebFetch', status: 'ok', title: 'Opening github.com', started_at: at(0) }], dropped: 0 }, idBase: 'x', defaultOpen: true });
  assert.match(text(html), /Opening github\.com/);
  assert.doesNotMatch(html, /Show command/);
});

test('(c) the raw command is not in the title; it is behind "Show command": a button, collapsed, keyboard reachable, 44 px on a touch phone', () => {
  const html = m.saved({ activity: { steps: [sshOk], dropped: 0 }, idBase: 'c', defaultOpen: true });
  assert.doesNotMatch(html, /ConnectTimeout/, 'the command is not drawn until asked');
  assert.match(html, /<button type="button" aria-expanded="false"[^>]*>Show command<\/button>/);
  assert.match(html, /<button type="button" aria-expanded="false" class="[^"]*\[@media\(max-width:767px\)_and_\(pointer:coarse\)\]:min-h-\[44px\]/);
  assert.match(html, /<button type="button" aria-expanded="false" class="[^"]*focus-visible:outline-sky-400/);
  assert.match(html, /<strong[^>]*>203\.0\.113\.66<\/strong>/, 'the host is set in bold');
  assert.doesNotMatch(html, /<details/, 'a detail that is only the command is not drawn twice');
  assert.equal(m.shellCommandText(sshOk), SSH, 'the whole command from the detail');
  assert.equal(m.shellCommandText(sshFailed), titleOf(SSH).slice(9, -1), 'a failed step: the title’s (its detail is the error)');
});

test('(c) Show command opens and closes the command as text, and a failed step keeps its error in the row', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  const evil = { ...sshOk, id: 'e', title: 'Running `echo <img src=x onerror=alert(1)>`', detail: 'echo <img src=x onerror=alert(1)>' };
  m.flushSync(() => root.render(m.React.createElement(m.SavedActivityList, { activity: { steps: [sshFailed, sshOk, evil], dropped: 0 }, idBase: 'd', defaultOpen: true, who: 'Quant' })));
  const buttons = [...host.querySelectorAll('[data-show-command] button')];
  assert.equal(buttons.length, 3);
  const b = buttons[1];
  assert.equal(b.getAttribute('aria-expanded'), 'false');
  b.focus();
  assert.equal(document.activeElement, b, 'reachable by keyboard focus');
  m.flushSync(() => b.click());
  assert.equal(b.getAttribute('aria-expanded'), 'true');
  assert.equal(b.textContent, 'Hide command');
  const pre = document.getElementById(b.getAttribute('aria-controls'));
  assert.equal(pre.textContent, SSH);
  assert.equal(pre.getAttribute('tabindex'), '0', 'a long command scrolls by keyboard');
  m.flushSync(() => b.click());
  assert.equal(b.getAttribute('aria-expanded'), 'false');
  assert.equal(b.textContent, 'Show command');
  assert.equal(document.getElementById('d-steps-s2-cmd'), null);
  // A failed step: the error stays in its own box, neutral (retried), and the command is behind its own button.
  const failedRow = host.querySelector('#d-steps-s1');
  assert.match(failedRow.querySelector('pre').textContent, /Connection timed out/);
  assert.doesNotMatch(failedRow.querySelector('pre').className, /rose/);
  assert.equal(failedRow.querySelector('summary').textContent.includes('so Quant tried again'), true);
  // Never HTML from data.
  m.flushSync(() => buttons[2].click());
  assert.equal(host.querySelector('img'), null);
  assert.match(document.getElementById('d-steps-e-cmd').textContent, /<img src=x/);
  m.flushSync(() => root.unmount());
  host.remove();
});

test('fix 1: a team or room chat never names the chat as the bot; a discussion names the current speaker', () => {
  // team chat, nobody streaming yet: "the bot", never "trading-lab"
  const who = m.stepsWho({ discussing: false, currentSpeaker: null, streamingSpeakerName: undefined, title: 'trading-lab', direct: false });
  assert.equal(who, undefined);
  const html = m.live({ live: { turnId: 't1', steps: [sshFailed, sshRunning], thinking: {} }, idBase: 'live-t1', who });
  assert.match(text(html), /didn’t connect, so the bot tried again/);
  assert.doesNotMatch(html, /trading-lab/, 'the chat title is never the bot');
  // discussion in progress: the current speaker
  const lead = m.stepsWho({ discussing: true, currentSpeaker: 'Quant', streamingSpeakerName: 'Lead', title: 'trading-lab', direct: false });
  assert.equal(lead, 'Quant');
  assert.match(text(m.live({ live: { turnId: 't2', steps: [sshFailed, sshRunning], thinking: {} }, idBase: 'live-t2', who: lead })), /so Quant tried again/);
  // the replying bot when one streams; the title only in a direct chat (it is the bot's own name)
  assert.equal(m.stepsWho({ streamingSpeakerName: 'Lead', title: 'trading-lab', direct: false }), 'Lead');
  assert.equal(m.stepsWho({ title: 'Quant', direct: true }), 'Quant');
  // ChatView hands the steps stepsWho's answer, not liveSpeaker (BrowsedChip stays for 36d)
  const src = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  assert.match(src, /who=\{stepsName\}/);
  assert.match(src, /direct: home\?\.kind === 'bot'/);
});

test('fix 2: the Show command ring has room (px-1 -ml-1 outline-offset-2) and keeps 44 px on a touch phone', () => {
  const html = m.live({ live: { turnId: 't3', steps: [sshOk], thinking: {} }, idBase: 'live-t3', who: 'Quant' });
  const btn = html.match(/<button[^>]*>Show command<\/button>/)?.[0] ?? '';
  for (const c of ['px-1', '-ml-1', 'focus-visible:outline-offset-2', 'min-h-[44px]']) assert.ok(btn.includes(c), c);
});
