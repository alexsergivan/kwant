// Audit F-10 (2026-10-20): with approvals off, a bot that can run commands can read every password
// saved on this server. The server refuses that combination; these screens say so plainly and offer
// the way through: move the passwords to Bitwarden first, or turn approvals back on.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SpaceApprovalsSetting, BotApprovalsSetting, OffConfirm } from './src/components/ApprovalsSetting';
  import { ExposedBanner, MoveToBitwardenFirst, AutofillGapNote } from './src/components/PasswordStorePanel';
  import { SavedPasswordsPanel } from './src/components/SavedPasswordsPanel';
  export * from './src/passwordStore';
  export { approvalModeApi, NeedsBitwardenError, APPROVALS_CHANGED_EVENT, offConsequences } from './src/approvalMode';
  export { passwordsApi, PasswordsError } from './src/savedPasswords';
  const r = (C, p) => renderToStaticMarkup(React.createElement(C, p));
  export const storeState = (status) => ({ status, problem: null, reload() {}, set() {} });
  export const confirm = (status, p) => r(OffConfirm, { title: 'Turn off approvals for every bot in Work?', busy: false, onCancel() {}, onConfirm() {}, passwords: status === undefined ? undefined : storeState(status), ...p });
  export const spaceSetting = (p) => r(SpaceApprovalsSetting, { spaceId: 'spc_1', spaceName: 'Work', value: 'ask', onChanged() {}, ...p });
  export const botSetting = (p) => r(BotApprovalsSetting, { kwantId: 'k1', kwantName: 'Dev', ...p });
  export const banner = (status) => r(ExposedBanner, { store: storeState(status) });
  export const moveFirst = (status, p) => r(MoveToBitwardenFirst, { store: storeState(status), label: 'Move 3 saved passwords to Bitwarden first', ...p });
  export const gap = (status) => r(AutofillGapNote, { status });
  export const panel = (status, p) =>
    r(SavedPasswordsPanel, { sandboxId: 'k1', state: { logins: [], pending: null, loading: false, problem: null, reload() {} }, pageUrl: '', store: storeState(status), ...p });
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/’/g, "'").replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const buttons = (html) => [...html.matchAll(/<button[^>]*>([\s\S]*?)<\/button>/g)].map((x) => text(x[1]));
const bw = (p) => ({ server_url: 'https://vault.bitwarden.com', email: 'dummy@example.com', login: 'password', client_id: null, unlocked: true, unlocked_at: 't', item_count: 4, last_sync: 't', ...p });
const st = (p) => ({ store: 'local', locked: false, local_count: 0, bitwarden: null, approvals_off: false, exposed: false, ...p });
const LOCAL3 = st({ local_count: 3 });
const LOCAL0 = st();
const BW = st({ store: 'bitwarden', bitwarden: bw() });
const BW_LEFTOVER = st({ store: 'bitwarden', bitwarden: bw(), local_count: 2 });
const BW_LOCKED = st({ store: 'bitwarden', locked: true, bitwarden: bw({ unlocked: false }), local_count: 3 });
const EXPOSED = st({ local_count: 3, approvals_off: true, exposed: true });
const LOCAL_NOTE = "With approvals off, any bot that can run commands could read passwords saved on this server, so that's blocked: use Bitwarden.";
const BW_NOTE = 'While a password is being filled into a page, a bot that can run commands can read it.';

// ---------------------------------------------------------------- the model

test('store status reads approvals_off and exposed; older servers mean neither', () => {
  const s = m.parseStoreStatus({ store: 'local', locked: false, local_count: 3, bitwarden: null, approvals_off: true, exposed: true });
  assert.equal(s.approvals_off, true);
  assert.equal(s.exposed, true);
  const old = m.parseStoreStatus({ store: 'local', local_count: 3 });
  assert.equal(old.approvals_off, false);
  assert.equal(old.exposed, false);
  assert.equal(m.parseStoreStatus({ exposed: 'yes' }).exposed, false, 'only true counts');
});

test('the note fits the store: Bitwarden (empty here) gets the fill gap, anything else the block', () => {
  assert.equal(text(m.autofillGapNote(BW)), BW_NOTE);
  assert.equal(text(m.autofillGapNote(LOCAL0)), LOCAL_NOTE);
  assert.equal(text(m.autofillGapNote(LOCAL3)), LOCAL_NOTE);
  assert.equal(text(m.autofillGapNote(BW_LEFTOVER)), LOCAL_NOTE, 'leftovers here open with the built-in key');
  assert.equal(text(m.autofillGapNote(null)), LOCAL_NOTE, 'unknown: the stricter one');
  for (const n of Object.values(m.AUTOFILL_GAP_NOTE)) assert.equal(n.split(/[.!?](\s|$)/).filter((x) => x && x.trim()).length, 1, `one sentence: ${n}`);
  assert.deepEqual(m.offConsequences(BW).at(-1), m.AUTOFILL_GAP_NOTE.bitwarden);
  assert.deepEqual(m.offConsequences(LOCAL0).at(-1), m.AUTOFILL_GAP_NOTE.local);
});

test('move-first and exposed wording, one and many', () => {
  assert.equal(m.moveFirstLabel(1), 'Move 1 saved password to Bitwarden first');
  assert.equal(m.moveFirstLabel(3), 'Move 3 saved passwords to Bitwarden first');
  assert.equal(
    text(m.moveFirstText(3)),
    "3 saved passwords are kept on this server. With approvals off, any bot that can run commands could read them, so move them to Bitwarden first.",
  );
  assert.match(text(m.moveFirstText(1)), /^1 saved password is kept .* read it, so move it to Bitwarden first\.$/);
  assert.equal(text(m.exposedText(3)), 'Approvals are off and 3 saved passwords are still on this server, so any bot that can run commands can read them.');
  assert.match(text(m.exposedText(1)), /1 saved password is still on this server, .* read it\.$/);
});

test('a 409 with needs_bitwarden: the server\'s sentence, capitalised, and the count; anything else is not it', () => {
  const body = { error: "approvals can't be switched off while passwords are saved on this server: a bot that can run commands could read them. Move them to Bitwarden first.", needs_bitwarden: true, local_count: 2 };
  const n = m.needsBitwardenFrom(409, body);
  assert.equal(n.local_count, 2);
  assert.match(n.message, /^Approvals can't be switched off/);
  assert.equal(m.needsBitwardenFrom(409, { error: 'x', needs_bitwarden: true }).local_count, null);
  assert.match(m.needsBitwardenFrom(409, { needs_bitwarden: true }).message, /Bitwarden/);
  assert.equal(m.needsBitwardenFrom(409, { error: 'this space is archived; restore it first' }), null);
  assert.equal(m.needsBitwardenFrom(400, body), null);
});

// ---------------------------------------------------------------- the API

async function withFetch(handler, fn) {
  const prevFetch = globalThis.fetch;
  const prevWindow = globalThis.window;
  globalThis.window = new EventTarget();
  const calls = [];
  globalThis.fetch = async (url, init = {}) => {
    calls.push({ url, method: init.method ?? 'GET', body: init.body });
    return handler(url, init);
  };
  try {
    return await fn(calls);
  } finally {
    globalThis.fetch = prevFetch;
    globalThis.window = prevWindow;
  }
}
const json = (b, status = 200) => new Response(JSON.stringify(b), { status });

test('approvals PUT refused for passwords here → NeedsBitwardenError with the count; archived 409 stays as before', async () => {
  const refusal = { error: "approvals can't be switched off while passwords are saved on this server: a bot that can run commands could read them. Move them to Bitwarden first.", needs_bitwarden: true, local_count: 3 };
  await withFetch(
    () => json(refusal, 409),
    async () => {
      await assert.rejects(m.approvalModeApi.setSpace('spc_1', 'off'), (e) => e instanceof m.NeedsBitwardenError && e.localCount === 3 && /^Approvals can't be switched off/.test(e.message));
      await assert.rejects(m.approvalModeApi.setBot('k1', 'inherit'), (e) => e instanceof m.NeedsBitwardenError && e.localCount === 3);
    },
  );
  await withFetch(
    () => json({ error: 'this space is archived; restore it first' }, 409),
    async () => {
      await assert.rejects(m.approvalModeApi.setSpace('spc_1', 'off'), (e) => !(e instanceof m.NeedsBitwardenError) && /archived/.test(e.message));
    },
  );
});

test('saving here while approvals are off: the 409 reason, not a generic error (Add and a caught login)', async () => {
  const refusal = { error: "passwords can't be saved on this server while approvals are off: a bot that can run commands could read them. Use Bitwarden, or turn approvals back on.", needs_bitwarden: true };
  await withFetch(
    () => json(refusal, 409),
    async (calls) => {
      for (const p of [m.passwordsApi.add('k1', 'https://example.com', 'alice', 'DUMMY-PW'), m.passwordsApi.savePending('k1')]) {
        await assert.rejects(p, (e) => e instanceof m.PasswordsError && e.needsBitwarden === true && e.status === 409 && /^Passwords can't be saved on this server while approvals are off/.test(e.message) && /Use Bitwarden, or turn approvals back on\.$/.test(e.message));
      }
      assert.deepEqual(calls.map((c) => `${c.method} ${c.url}`), ['POST /api/sandbox/k1/passwords', 'POST /api/sandbox/k1/passwords/pending']);
    },
  );
  // An ordinary failure is unchanged.
  await withFetch(
    () => json({ error: 'bad origin' }, 400),
    async () => {
      await assert.rejects(m.passwordsApi.add('k1', 'x', 'a', 'b'), (e) => e.needsBitwarden === false && e.message === 'bad origin');
    },
  );
  // The typed password isn't kept around after that refusal.
  const src = readFileSync(new URL('../src/components/SavedPasswordsPanel.tsx', import.meta.url), 'utf8');
  assert.match(src, /err\.needsBitwarden\) setPassword\(''\)/);
});

test('"Turn approvals back on": every active space that is off and every bot set to off → ask; archived counts as not changed', async () => {
  let changedEvents = 0;
  await withFetch(
    (url, init) => {
      // Like the server: the plain list is active spaces only, ?archived=1 only archived ones.
      if (url === '/api/spaces')
        return json([
          { id: 'spc_1', approvals: 'off', archived_at: null },
          { id: 'spc_2', approvals: 'ask', archived_at: null },
        ]);
      if (url === '/api/spaces?archived=1') return json([{ id: 'spc_old', approvals: 'off', archived_at: '2026-01-01' }]);
      if (url === '/api/kwants?space=spc_1') return json([{ id: 'k1', approvals: 'inherit' }, { id: 'k2', approvals: 'off' }]);
      if (url === '/api/kwants?space=spc_2') return json([{ id: 'k3', approvals: 'off' }, { id: 'k4', approvals: 'ask' }]);
      if (init.method === 'PUT' && url.startsWith('/api/spaces/')) return json({ approvals: 'ask' });
      if (init.method === 'PUT') return json({ kwant_id: url.split('/')[3], approvals: 'ask', approvals_effective: 'ask', space_approvals: 'ask' });
      return json({}, 404);
    },
    async (calls) => {
      window.addEventListener(m.APPROVALS_CHANGED_EVENT, () => changedEvents++);
      assert.deepEqual(await m.approvalModeApi.turnAllOn(), { changed: 3, failed: 1 });
      const puts = calls.filter((c) => c.method === 'PUT');
      assert.deepEqual(
        puts.map((c) => `${c.url} ${c.body}`),
        ['/api/spaces/spc_1/approvals {"approvals":"ask"}', '/api/kwants/k2/approvals?space=spc_1 {"approvals":"ask"}', '/api/kwants/k3/approvals?space=spc_2 {"approvals":"ask"}'],
      );
      assert.ok(!calls.some((c) => c.url.includes('spc_old')), 'an archived space is not touched');
    },
  );
  assert.equal(changedEvents, 1, 'the roster and markers load again');
});

// ---------------------------------------------------------------- the confirm

test('confirm with passwords here: "off" isn\'t offered; it says why and offers to move them first', () => {
  const html = m.confirm(LOCAL3);
  const t = text(html);
  assert.match(html, /data-off-needs-bitwarden/);
  assert.match(t, /3 saved passwords are kept on this server\. With approvals off, any bot that can run commands could read them, so move them to Bitwarden first\./);
  assert.deepEqual(buttons(html), ['Move 3 saved passwords to Bitwarden first', 'Keep asking']);
  assert.doesNotMatch(t, /Turn approvals off/);
  // The paragraph says it; the note isn't repeated above it.
  assert.ok(!t.includes(LOCAL_NOTE));
  // Bitwarden with leftovers here, and Bitwarden locked with some here: the same.
  assert.deepEqual(buttons(m.confirm(BW_LEFTOVER)), ['Move 2 saved passwords to Bitwarden first', 'Keep asking']);
  assert.deepEqual(buttons(m.confirm(BW_LOCKED)), ['Move 3 saved passwords to Bitwarden first', 'Keep asking']);
});

test('confirm with nothing here: "off" is offered, with the sentence for that store', () => {
  const b = m.confirm(BW);
  assert.deepEqual(buttons(b), ['Keep asking', 'Turn approvals off']);
  assert.ok(text(b).includes(BW_NOTE));
  assert.ok(!text(b).includes(LOCAL_NOTE));
  const l = m.confirm(LOCAL0);
  assert.deepEqual(buttons(l), ['Keep asking', 'Turn approvals off']);
  assert.ok(text(l).includes(LOCAL_NOTE), 'nothing saved yet, and saving here will be refused');
});

test('confirm while the store is still loading: "off" waits; with no store info (older screens) it is offered and the server decides', () => {
  const html = m.confirm(null);
  assert.match(text(html), /Checking saved passwords…|Checking saved passwords\.\.\./);
  assert.match(html, /<button[^>]*disabled=""[^>]*>[\s\S]*Checking saved passwords/);
  assert.deepEqual(buttons(m.confirm(undefined)), ['Keep asking', 'Turn approvals off']);
  const failed = m.confirm(undefined, { passwords: { status: null, problem: 'error', reload() {}, set() {} } });
  assert.deepEqual(buttons(failed), ['Keep asking', 'Turn approvals off'], 'store unreadable: the server\'s 409 is the backstop');
});

test('space and bot settings use it: move first with passwords here, off with none', () => {
  assert.match(text(m.spaceSetting({ initialConfirm: true, initialPasswords: LOCAL3 })), /Move 3 saved passwords to Bitwarden first/);
  assert.doesNotMatch(text(m.spaceSetting({ initialConfirm: true, initialPasswords: LOCAL3 })), /Turn approvals off/);
  const state = { kwant_id: 'k1', approvals: 'inherit', approvals_effective: 'ask', space_approvals: 'ask' };
  const bot = m.botSetting({ initial: state, initialPending: 'off', initialPasswords: LOCAL3 });
  assert.match(text(bot), /Let Dev run everything without approval\?/);
  assert.match(text(bot), /Move 3 saved passwords to Bitwarden first/);
  assert.doesNotMatch(text(bot), /Turn approvals off/);
  assert.match(text(m.botSetting({ initial: state, initialPending: 'off', initialPasswords: BW })), /Turn approvals off/);
});

test('move first: connect when there is no Bitwarden, unlock when it is locked, both in place', () => {
  assert.match(m.moveFirst(LOCAL3, { initialStep: 'connect' }), /data-bitwarden-form="connect"/);
  assert.match(text(m.moveFirst(LOCAL3, { initialStep: 'connect' })), /the passwords move as soon as it's connected/);
  assert.match(m.moveFirst(BW_LOCKED, { initialStep: 'unlock' }), /data-bitwarden-form="unlock"/);
  assert.match(text(m.moveFirst(BW_LOCKED, { initialStep: 'unlock' })), /dummy@example\.com/);
  assert.deepEqual(buttons(m.moveFirst(LOCAL3)), ['Move 3 saved passwords to Bitwarden first']);
  assert.equal(m.moveFirst(null), '', 'no store, nothing');
  // The master password stays in its input, never in state (same rule as the Bitwarden screens).
  const src = readFileSync(new URL('../src/components/PasswordStorePanel.tsx', import.meta.url), 'utf8');
  assert.doesNotMatch(src, /useState[^;\n]*(master|secret)/i);
});

// ---------------------------------------------------------------- old installs

test('exposed: a plain banner with both ways out, next to saved passwords and in approvals settings', () => {
  const html = m.banner(EXPOSED);
  assert.match(html, /data-exposed/);
  assert.match(html, /role="region"/);
  assert.equal(text(html).split(' Move to Bitwarden')[0], 'Approvals are off and 3 saved passwords are still on this server, so any bot that can run commands can read them.');
  assert.deepEqual(buttons(html), ['Move to Bitwarden', 'Turn approvals back on']);
  assert.equal(m.banner({ ...EXPOSED, exposed: false }), '', 'nothing when not exposed');
  assert.equal(m.banner(null), '');

  assert.match(m.panel(EXPOSED, { approvalsOff: true }), /data-exposed/);
  assert.doesNotMatch(m.panel(EXPOSED, { approvalsOff: true }), /data-autofill-gap/, 'the banner says it; no note repeating it');
  assert.doesNotMatch(m.panel(LOCAL3), /data-exposed/);
  assert.match(m.spaceSetting({ value: 'off', initialPasswords: EXPOSED }), /data-exposed/);
  assert.match(m.botSetting({ initial: { kwant_id: 'k1', approvals: 'off', approvals_effective: 'off', space_approvals: 'ask' }, initialPasswords: EXPOSED }), /data-exposed/);
  assert.doesNotMatch(m.spaceSetting({ value: 'ask', initialPasswords: LOCAL3 }), /data-exposed/);
});

test('next to saved passwords: the note follows the store, and shows when any bot runs without asking', () => {
  assert.ok(text(m.panel(st({ store: 'bitwarden', bitwarden: bw(), approvals_off: true }))).includes(BW_NOTE), 'the server says approvals are off somewhere');
  assert.ok(text(m.panel(st({ approvals_off: true }))).includes(LOCAL_NOTE));
  assert.ok(!text(m.panel(BW)).includes(BW_NOTE));
  assert.match(m.gap(BW), /data-autofill-gap="bitwarden"/);
  assert.match(m.gap(LOCAL0), /data-autofill-gap="local"/);
  for (const s of [BW, LOCAL0]) assert.doesNotMatch(m.gap(s), /rose|red-|amber/, 'no scare styling');
  assert.equal(text(m.gap(BW)), `Approvals are off. ${BW_NOTE}`);
  assert.equal(text(m.gap(LOCAL0)), LOCAL_NOTE, 'no "Approvals are off." before "With approvals off"');
});
