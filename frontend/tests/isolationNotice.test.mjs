// Chat notices for "Run Claude inside the VM": every refusal code gets a row (design/ui.md
// notices 1–7, design/ui-check-446.md fix 4), the unknown code shows the server's own words,
// and the request limit gets Continue.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export * from './src/isolationNotice';
  export { retryTargets } from './src/interruptions';
  export { IsolationNotice, RequestLimitNotice } from './src/components/IsolationNotice';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);

// Shaped like IsolationFailed::to_meta (src/sandbox/agent_vm/notice.rs).
const notice = (id, reason, extra = {}, text = 'saved text') => ({
  id,
  sender_role: 'kwant',
  sender_id: 'Ada',
  text,
  meta: JSON.stringify({ kind: 'turn_isolation_failed', kwant_name: 'Ada', engine: 'claude', reason, phase: 'start', progress_pct: null, code: null, ...extra }),
});
const limitNote = (id, limit = 200) => ({
  id,
  sender_role: 'kwant',
  sender_id: 'Ada',
  text: `Ada paused after ${limit} requests to Claude in this reply.`,
  meta: JSON.stringify({ kind: 'turn_request_limit', kwant_name: 'Ada', engine: 'claude', limit }),
});
const user = (id, text) => ({ id, sender_role: 'user', sender_id: 'me', text, meta: null });
const card = (reason, extra, text) => m.isolationCard(m.parseIsolationFailed(notice('n', reason, extra, text)), text);
const all = { onRetry: () => {}, onOpenComputerSettings: () => {}, onOpenAccounts: () => {}, onChangeTeamFolder: () => {}, onEditBot: () => {} };
const render = (msg, props = {}) =>
  m.renderToStaticMarkup(
    m.createElement(m.IsolationNotice, { messageId: msg.id, meta: m.parseIsolationFailed(msg), text: msg.text, retry: { text: 'go', attachments: [] }, ...all, ...props }),
  );
const plain = (c) => c.reason.map((s) => s.text).join('');

test('parses the meta and ignores other messages', () => {
  const p = m.parseIsolationFailed(notice('n', 'unsafe_folder', { folder: '/home/oleks', folder_from: 'team', team_id: 't1', team_name: 'Checkout', unsafe_kind: 'home' }));
  assert.equal(p.botName, 'Ada');
  assert.equal(p.folderFrom, 'team');
  assert.equal(p.unsafeKind, 'home');
  assert.equal(m.parseIsolationFailed(user('u', 'hi')), null);
  assert.equal(m.parseIsolationFailed({ ...notice('n', 'x'), meta: JSON.stringify({ kind: 'turn_stalled' }) }), null);
  assert.equal(m.parseRequestLimit(limitNote('l')).limit, 200);
});

// The table: every code the server sends, with first line, reason, safety and buttons.
const ROWS = [
  ['docker_unavailable', /Claude’s sealed box couldn’t start\.$/, /^Docker isn’t running on this computer\. Start Docker, then Retry\.$/, 'standard', ['retry', 'computer']],
  ['preparing', /sealed box isn’t ready yet\.$/, /^The one-time setup is still running\. Retry when it’s done\.$/, 'standard', ['retry', 'computer']],
  ['download_failed', /sealed box isn’t ready yet\.$/, /^The one-time setup didn’t finish\. Try it again in Computer settings, then Retry\.$/, 'standard', ['retry', 'computer']],
  ['download_needed', /sealed box isn’t ready yet\.$/, /^The one-time setup didn’t finish\./, 'standard', ['retry', 'computer']],
  ['relay_down', /Claude’s sealed box couldn’t start\.$/, /^kwant couldn’t connect Claude to your sign-in\. Restart kwant, then Retry\.$/, 'standard', ['retry', 'computer']],
  ['sign_in_expired', /your Claude sign-in has expired\.$/, /^Sign in again in Settings › This space › Apps & accounts, then Retry\.$/, null, ['retry', 'accounts']],
  ['no_credential', /there’s no Claude sign-in for it on this computer\.$/, /^Sign in to Claude in Settings › This space › Apps & accounts, then Retry\.$/, null, ['retry', 'accounts']],
  ['host_is_root', /Claude’s sealed box couldn’t start\.$/, /^kwant is running as root, so Claude’s sealed box can’t run as you\./, 'standard', ['retry', 'computer']],
  ['folder_name_unsupported', /Claude can’t take this folder into its sealed box\.$/, /^The folder’s name has characters kwant can’t pass to Claude’s sealed box, like a line break\. Rename the folder, then Retry\.$/, 'nothing_ran', ['retry']],
  ['folder_record_failed', /Claude’s sealed box couldn’t start\.$/, /^kwant couldn’t note that this folder is used by Claude’s sealed box, so it didn’t start Claude\./, 'standard', ['retry', 'computer']],
  ['container_failed', /Claude’s sealed box couldn’t start\.$/, /^Docker couldn’t start it\. Retry; if it keeps happening, restart Docker\.$/, 'standard', ['retry', 'computer']],
  ['unsupported_os', /Claude’s sealed box couldn’t start\.$/, /^Isolation is only on Linux for now\. Turn off key protection to run Claude on this computer\.$/, 'standard', ['computer']],
];

for (const [reason, first, why, safety, actions] of ROWS) {
  test(`row: ${reason}`, () => {
    const c = card(reason);
    assert.equal(c.row, reason);
    assert.match(c.first, /^Ada /);
    assert.match(c.first, first);
    assert.match(plain(c), why);
    assert.equal(c.safety, safety);
    assert.deepEqual(c.actions, actions);
    assert.doesNotMatch(plain(c), /Something went wrong/);
    const html = render(notice('n1', reason));
    assert.match(html, new RegExp(`data-isolation-notice="${reason}"`));
    for (const a of actions) assert.match(html, new RegExp(`data-isolation-action="${a}"`));
    if (safety) assert.match(html, new RegExp(`data-isolation-safety="${safety}"`));
    else assert.doesNotMatch(html, /data-isolation-safety/);
  });
}

test('preparing uses the percent when the server sends it', () => {
  assert.match(plain(card('preparing', { progress_pct: 42 })), /at 42%/);
});

test('the standard safety line bolds the setting name', () => {
  const html = render(notice('n', 'docker_unavailable'));
  assert.match(html, /While <span class="font-medium text-zinc-300">key protection<\/span> is on, Claude never runs outside its sealed box, so nothing ran on this computer\. To run Claude here instead, turn off key protection in Computer settings\./);
});

test('mid-turn (notice 3): any reason, its own first line', () => {
  const c = card('container_failed', { phase: 'mid_turn' });
  assert.equal(c.row, 'mid_turn');
  assert.equal(c.first, 'Ada’s reply stopped partway: Claude’s sealed box lost its connection.');
  assert.match(plain(c), /Undo still works/);
  assert.equal(c.safety, 'standard');
});

test('unknown code: the server’s own words win over "Something went wrong"', () => {
  // A detail in the meta
  assert.match(plain(card('new_code', { detail: 'The VM ran out of disk.' })), /^The VM ran out of disk\. Retry, or turn off key protection to run Claude on this computer\.$/);
  // Human text the server saved beyond its template
  const saved = "Ada didn't reply: Claude couldn't start inside the VM. The image is from a newer kwant. While Run Claude inside the VM is on, Claude never runs outside it, so nothing ran on this computer.";
  assert.match(plain(card('new_code', {}, saved)), /^The image is from a newer kwant\. Retry/);
  // Only the template: the code, as a last resort
  const tpl = "Ada didn't reply: Claude couldn't start inside the VM. Something went wrong (new_code). Retry, or turn off Run Claude inside the VM to run Claude on this computer. While Run Claude inside the VM is on, Claude never runs outside it, so nothing ran on this computer.";
  assert.equal(m.serverReason(tpl, 'Ada'), null);
  // 0.8.0's server words (key protection, vm-naming.md) are stripped as well as 0.7.7's
  const tpl080 = "Ada didn't reply: Claude's sealed box couldn't start. Something went wrong (new_code). Retry, or turn off key protection to run Claude on this computer. While key protection is on, Claude never runs outside its sealed box, so nothing ran on this computer.";
  assert.equal(m.serverReason(tpl080, 'Ada'), null);
  const saved080 = "Ada didn't reply: Claude's sealed box couldn't start. The image is from a newer kwant. While key protection is on, Claude never runs outside its sealed box, so nothing ran on this computer.";
  assert.equal(m.serverReason(saved080, 'Ada'), 'The image is from a newer kwant.');
  assert.match(plain(card('new_code', {}, tpl)), /^Something went wrong \(new_code\)\./);
  assert.equal(card('new_code').row, 'unknown');
  assert.deepEqual(card('new_code').actions, ['retry', 'computer']);
});

test('notice 6: unsafe folder, by kind, with ~ and Change folder for a team', () => {
  const c = card('unsafe_folder', { folder: '/home/oleks', folder_from: 'team', team_id: 't1', team_name: 'Checkout', unsafe_kind: 'home' });
  assert.equal(c.first, 'Ada didn’t reply: Claude can’t take this folder into its sealed box.');
  assert.equal(plain(c), '~ is your home folder. In its sealed box, Claude would see all your files and sign-ins. Pick a project folder for @Checkout instead, then Retry.');
  assert.equal(c.reason[0].folder, true);
  assert.equal(c.safety, 'unsafe');
  assert.deepEqual(c.actions, ['change_folder', 'retry']);
  assert.equal((plain(c).match(/Pick a project folder/g) || []).length, 1);
  assert.doesNotMatch(plain(c), /the computer/);

  const kinds = {
    contains_home: /holds your home folder/,
    root: /holds your home folder/,
    sign_ins: /holds sign-ins, like your SSH keys/,
    kwant_data: /holds kwant’s own data/,
    docker: /Docker’s control socket/,
    mystery: /^This folder holds your home folder or sign-ins/,
  };
  for (const [k, re] of Object.entries(kinds)) {
    const x = card('unsafe_folder', { folder: '/srv/x', folder_from: 'bot', unsafe_kind: k });
    assert.match(plain(x), re, k);
    assert.match(plain(x), /Pick a project folder instead, then Retry\.$/);
    assert.deepEqual(x.actions, ['retry'], `${k}: no Change folder for a bot folder`);
  }
  assert.match(plain(card('unsafe_folder', { folder: '/x', folder_from: 'goal', unsafe_kind: 'home' })), /for this goal instead/);
  // No folder at all: still reads, no path
  assert.match(plain(card('unsafe_folder', { unsafe_kind: 'home' })), /^This folder is your home folder/);
  const html = render(notice('n', 'unsafe_folder', { folder: '/Users/oleks/code', folder_from: 'team', team_id: 't1', team_name: 'Checkout', unsafe_kind: 'sign_ins' }));
  assert.match(html, /data-isolation-folder[^>]*>~\/code</);
  assert.match(html, /data-isolation-action="change_folder"[\s\S]*data-isolation-action="retry"/);
});

test('notice 6b: the shared folder has no path and asks for the team’s own folder', () => {
  const c = card('unsafe_folder', { folder: '/var/lib/kwant/workspace', folder_from: 'team', team_id: 't1', team_name: 'Checkout', unsafe_kind: 'shared_folder' });
  assert.equal(c.row, 'unsafe_folder:shared_folder');
  assert.doesNotMatch(plain(c), /\/var\/lib/);
  assert.match(plain(c), /Give @Checkout its own folder, then Retry\.$/);
  assert.equal(c.safety, 'unsafe_shared');
  assert.deepEqual(c.actions, ['change_folder', 'retry']);
  assert.match(plain(card('unsafe_folder', { unsafe_kind: 'shared_folder' })), /Give this team its own folder, then Retry\.$/);
});

test('notice 7: Codex or Grok in a folder Claude used', () => {
  const team = card('folder_used_by_isolated_bot', { engine: 'codex', folder: '/home/oleks/shop', folder_from: 'team', team_id: 't1', team_name: 'Checkout' });
  assert.equal(team.first, 'Ada didn’t reply: Codex won’t run in a folder Claude used in its sealed box.');
  assert.match(plain(team), /left settings in ~\/shop that Codex would load/);
  assert.match(plain(team), /Switch Ada to Claude, or pick another folder for @Checkout, then Retry\.$/);
  assert.deepEqual(team.actions, ['edit_bot', 'change_folder']);
  assert.equal(team.safety, 'folder_used');

  const bot = card('folder_used_by_isolated_bot', { engine: 'grok', folder: '/x', folder_from: 'bot' });
  assert.match(plain(bot), /^Ada used this folder as Claude in its sealed box.*Grok would load.*Switch Ada back to Claude to keep working here\.$/);
  assert.deepEqual(bot.actions, ['edit_bot', 'retry']);

  const other = card('folder_used_by_isolated_bot', { engine: 'codex', folder_from: 'goal' });
  assert.match(plain(other), /Switch Ada to Claude, then Retry\.$/);
  const html = render(notice('n', 'folder_used_by_isolated_bot', { engine: 'codex', folder_from: 'bot' }));
  assert.match(html, /Edit Ada/);
});

test('buttons: Retry only on the newest notice; the first shown button is primary', () => {
  const noRetry = render(notice('n', 'docker_unavailable'), { retry: null });
  assert.doesNotMatch(noRetry, /data-isolation-action="retry"/);
  assert.match(noRetry, /data-isolation-action="computer"[^>]*class="[^"]*bg-amber-500\/15/);
  const both = render(notice('n', 'docker_unavailable'));
  assert.match(both, /data-isolation-action="retry"[^>]*class="[^"]*bg-amber-500\/15/);
  assert.match(both, /data-isolation-action="computer"[^>]*class="[^"]*border-zinc-700/);
});

test('retryTargets counts isolation and limit notices', () => {
  const t = m.retryTargets([user('u1', 'build it'), notice('n1', 'docker_unavailable')]);
  assert.equal(t.get('n1').text, 'build it');
  assert.equal(m.retryTargets([user('u1', 'a'), notice('n1', 'preparing'), user('u2', 'b')]).size, 0);
  assert.equal(m.retryTargets([user('u1', 'a'), limitNote('l1')]).has('l1'), true);
  // Only the newest one
  const two = m.retryTargets([user('u1', 'a'), notice('n1', 'preparing'), notice('n2', 'docker_unavailable')]);
  assert.deepEqual([...two.keys()], ['n2']);
});

test('notice 5: the request limit, with Continue only when allowed', () => {
  const meta = m.parseRequestLimit(limitNote('l1', 200));
  const copy = m.requestLimitText(meta);
  assert.equal(copy.first, 'Ada paused after 200 requests to Claude in this reply.');
  assert.match(copy.body, /bot stuck in a loop/);
  const html = m.renderToStaticMarkup(m.createElement(m.RequestLimitNotice, { messageId: 'l1', meta, canContinue: true, onSend: () => {} }));
  assert.match(html, /data-request-limit-action="continue"/);
  assert.match(html, />Continue</);
  const none = m.renderToStaticMarkup(m.createElement(m.RequestLimitNotice, { messageId: 'l1', meta, canContinue: false, onSend: () => {} }));
  assert.doesNotMatch(none, /data-request-limit-action/);
  assert.equal(m.CONTINUE_TEXT, 'Continue where you left off.');
});

test('shortPath turns the home folder into ~', () => {
  assert.equal(m.shortPath('/home/oleks'), '~');
  assert.equal(m.shortPath('/home/oleks/shop'), '~/shop');
  assert.equal(m.shortPath('/Users/o/x'), '~/x');
  assert.equal(m.shortPath('/srv/home/x'), '/srv/home/x');
});
