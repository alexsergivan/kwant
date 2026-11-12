// A team from your own words (team-from-words brief, "Frontend (Iris)"): the box in the Get ready card,
// the sidebar's "Describe your own" chip and New team; Add an AI account first; submit, loading and the
// sheet; an error keeps the words; Try again sends the same need; Create sends suggestion_id + edits;
// Escape cancels. Against the bakery fixture in src/teams/suggestFixtures.ts. Example data only.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_demo', pretendToBeVisual: true });
const g = globalThis;
for (const k of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLSelectElement',
  'HTMLTextAreaElement',
  'HTMLButtonElement',
  'Element',
  'Node',
  'Event',
  'MouseEvent',
  'KeyboardEvent',
  'FocusEvent',
  'CustomEvent',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
  'localStorage',
  'sessionStorage',
]) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
let phone = false;
window.matchMedia = (q) => ({ matches: q.includes('max-width: 767px') ? phone : false, media: q, addEventListener() {}, removeEventListener() {} });
g.fetch = g.window.fetch = async () => new Response('{}', { status: 200, headers: { 'Content-Type': 'application/json' } });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * from './src/teams/teamTemplates';
  export * as fx from './src/teams/suggestFixtures';
  export { TeamPreviewSheet, listWords } from './src/components/TeamPreviewSheet';
  export { ReadyTeamsProvider } from './src/components/ReadyTeams';
  export { ReadyTeamsSidebarBox } from './src/components/ReadyTeamsEntry';
  export { TeamNeedBox } from './src/components/TeamNeedBox';
  export { SpaceReadyCard } from './src/components/SpaceReadyCard';
  export { NewTeamModal } from './src/components/NewTeamModal';
`);
const { React, createRoot, fx } = m;
const h = React.createElement;
const settle = async (n = 12) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const click = async (el) => {
  assert.ok(el, 'element to click exists');
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const typeArea = async (el, value) => {
  Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set.call(el, value);
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle(2);
};
const key = async (el, k, extra = {}) => {
  el.dispatchEvent(new window.KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...extra }));
  await settle();
};
const mount = async (el) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  return { host, root, rerender: async (e) => (root.render(e), await settle()), unmount: () => (root.unmount(), host.remove()) };
};
const byText = (root, sel, re) => [...root.querySelectorAll(sel)].find((el) => re.test(el.textContent ?? ''));
const use = (opts) => {
  const calls = [];
  m.setTeamsTransport(fx.fixtureTransport(opts, calls));
  return calls;
};
const asks = (calls) => calls.filter((c) => c.url.startsWith('/api/teams/suggest') && c.method !== 'DELETE');
const stops = (calls) => calls.filter((c) => c.url.startsWith('/api/teams/suggest') && c.method === 'DELETE');
const creates = (calls) => calls.filter((c) => c.url.startsWith('/api/teams/from-template'));
const NEED = fx.BAKERY_NEED;

/** The Get ready card inside the provider, as App renders it on an empty space. */
const card = (props = {}, provider = {}) => {
  const out = { accounts: 0, newBot: 0, opened: [], undone: 0 };
  const el = h(
    m.ReadyTeamsProvider,
    {
      space: 'spc_demo',
      bots: [],
      teams: [],
      onChanged() {},
      onOpenTeam: (t, ch) => out.opened.push({ t, ch }),
      onUndone: () => out.undone++,
      readyAccounts: 1,
      onOpenAccounts: () => out.accounts++,
      ...provider,
    },
    h('div', { id: 'card' }, h(m.SpaceReadyCard, { spaceName: 'Bakery', tile: null, readyAccounts: provider.readyAccounts ?? 1, botCount: 0, onAddAccount() {}, onNewBot: () => out.newBot++, ...props })),
    h('div', { id: 'side' }, h(m.ReadyTeamsSidebarBox, { teamCount: 0 })),
  );
  return { el, out };
};
const box = () => document.querySelector('#card [data-need-box] textarea');
const submitBtn = () => document.querySelector('#card [data-need-submit]');
const theSheet = () => document.querySelector('[data-team-sheet="suggest"]');

// ---------------------------------------------------------------------------------------------------

test('client rules: 10–1000 characters after trimming; the error copy per server code', () => {
  assert.equal(m.needProblem('   '), 'Say what your bots should do first.');
  assert.equal(m.needProblem('  bakery  '), 'Add a few more words so the team fits.');
  assert.equal(m.needProblem('a'.repeat(10)), null);
  assert.equal(m.needProblem(` ${'a'.repeat(1000)} `), null);
  assert.match(m.needProblem('a'.repeat(1003)), /3 characters over the 1,000 limit/);
  const E = (status, body) => new m.TeamsError(body.error ?? 'x', status, body.code ?? null, null, null, null, body);
  const failed = 'Couldn’t suggest a team this time. Try again, or start from a ready-made team.';
  assert.equal(m.needError(E(502, { error: 'The draft came back unusable twice.', code: 'draft_failed' })).head, failed);
  assert.equal(m.needError(E(502, { error: 'The draft came back unusable twice.', code: 'draft_failed' })).reason, 'The draft came back unusable twice.');
  const na = m.needError(E(409, { error: 'No account in this space can run right now.', code: 'no_ready_account' }));
  assert.equal(na.noAccount, true);
  assert.equal(na.head, 'Add an AI account first.');
  assert.equal(m.needError(E(409, { code: 'busy' })).head, 'Already suggesting a team here.');
  const lim = m.needError(E(429, { code: 'limited', account: 'Claude', until: '2026-11-12T21:10:00Z' }));
  assert.equal(lim.head, failed);
  assert.match(lim.reason, /Claude/);
  assert.equal(m.needError(new m.TeamsError('x', 0, 'offline')).head, 'You’re offline.');
  assert.equal(m.needError(new Error('boom')).head, failed);
  assert.equal(m.listWords(['customer emails', 'reply drafts']), 'customer emails and reply drafts');
  assert.equal(m.listWords(['a', 'b', 'c']), 'a, b and c');
});

test('fixture: the bakery answer has the server contract’s shape (brief "Server contract")', () => {
  const s = fx.BAKERY_SUGGESTION;
  assert.equal(s.bots.length, 4);
  assert.equal(s.bots.filter((b) => b.lead).length, 1);
  assert.deepEqual(s.facts, []);
  assert.deepEqual(s.read, { files: 0, manifests: 0, ms: 0 });
  assert.equal(s.partial, null);
  assert.equal(s.stack_line, null);
  assert.equal(s.fallback, false);
  assert.equal(s.need, NEED);
  for (const b of s.bots) {
    assert.ok(b.owns.length > 0, `${b.name} owns something`);
    assert.ok(b.reason.length > 0, `${b.name} has a reason`);
    assert.ok(b.instructions && b.instructions.length > 20, `${b.name} has instructions`);
  }
});

test('Get ready card: step 2 is "Add your bots" with the box, the ready-made line and add one bot', async () => {
  use({});
  const { el, out } = card();
  const v = await mount(el);
  const c = document.querySelector('#card');
  assert.match(c.textContent, /Add your bots/);
  const ta = box();
  assert.ok(ta, 'the box is there');
  assert.equal(ta.labels[0].textContent, 'What should your bots do?');
  assert.equal(ta.placeholder, 'e.g. I run a small bakery: answer customer emails, post on Instagram, keep track of orders');
  assert.equal(submitBtn().textContent, 'Suggest a team');
  const line = c.querySelector('[data-ready-made-line]');
  assert.match(line.textContent, /^Or start from a ready-made team: Inbox · Sales · Coding · or add one bot$/);
  assert.equal(byText(c, 'button', /^New bot$/), undefined, 'New bot moved into the line');
  await click(byText(line, 'button', /^add one bot$/));
  assert.equal(out.newBot, 1);
  await click(line.querySelector('[data-template="inbox"]'));
  assert.ok(document.querySelector('[data-team-sheet="inbox"]'), 'Inbox opens its preview');
  await key(document.querySelector('[data-team-sheet="inbox"]'), 'Escape');
  assert.match(c.textContent, /Message #general/, 'step 3 stays');
  v.unmount();
});

test('submit: Suggesting… locks the box, then the sheet opens on the bakery team; nothing is created yet', async () => {
  const calls = use({});
  const { el } = card();
  const v = await mount(el);
  await typeArea(box(), `  ${NEED}  `);
  submitBtn().dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await new Promise((r) => setTimeout(r, 0));
  const ask = asks(calls).at(-1);
  assert.equal(ask.url, '/api/teams/suggest?space=spc_demo');
  assert.deepEqual(ask.body, { need: NEED }, 'only { need }, trimmed');
  await settle();
  const s = theSheet();
  assert.ok(s, 'the sheet opens');
  assert.equal(s.querySelector('h3').textContent, 'Suggested team');
  assert.match(s.querySelector('[data-asked]').textContent, /I run a small bakery/);
  const rows = [...s.querySelectorAll('[data-row]')];
  assert.equal(rows.length, 4);
  assert.match(s.textContent, /ShopLead\s*Leads/);
  assert.match(rows[1].querySelector('[data-takes-care]').textContent, /^Takes care of customer emails and reply drafts$/);
  // Hidden for a team from words: the read line, partial notes, the stack line, and Coding's role picker.
  assert.doesNotMatch(s.textContent, /What it found|In every persona|Use the Coding roster|Owns /);
  assert.equal(s.querySelector('[aria-haspopup="listbox"]'), null);
  assert.equal(creates(calls).length, 0, 'nothing is created before Create');
  // Instructions: an expander per bot, closed at first, read-only text.
  const exp = byText(rows[1], 'button', /^Instructions/);
  assert.equal(exp.getAttribute('aria-expanded'), 'false');
  await click(exp);
  assert.equal(exp.getAttribute('aria-expanded'), 'true');
  const pane = document.getElementById(exp.getAttribute('aria-controls'));
  assert.match(pane.querySelector('pre').textContent, /Never send without the owner’s OK/);
  assert.equal(pane.querySelector('textarea, input'), null, 'instructions are read-only');
  v.unmount();
});

test('Enter submits, Shift+Enter is a new line; too few words stay in the box with a note', async () => {
  const calls = use({});
  const { el } = card();
  const v = await mount(el);
  await typeArea(box(), 'bakery');
  await key(box(), 'Enter');
  assert.equal(asks(calls).length, 0, 'too short is not sent');
  assert.match(document.querySelector('#card').textContent, /Add a few more words so the team fits\./);
  assert.equal(box().getAttribute('aria-invalid'), 'true');
  await typeArea(box(), NEED);
  await key(box(), 'Enter', { shiftKey: true });
  assert.equal(asks(calls).length, 0, 'Shift+Enter does not submit');
  await key(box(), 'Enter');
  assert.equal(asks(calls).length, 1);
  assert.ok(theSheet());
  v.unmount();
});

test('no ready account: the box stays, the button reads "Add an AI account first" and opens Accounts', async () => {
  const calls = use({});
  const { el, out } = card({}, { readyAccounts: 0 });
  const v = await mount(el);
  assert.ok(box(), 'the box stays');
  assert.equal(box().readOnly, false, 'you can still type');
  assert.equal(submitBtn().textContent, 'Add an AI account first');
  await typeArea(box(), NEED);
  await key(box(), 'Enter');
  assert.equal(asks(calls).length, 0, 'Enter sends nothing without an account');
  await click(submitBtn());
  assert.equal(out.accounts, 1);
  assert.equal(asks(calls).length, 0);
  v.unmount();

  // The server says so (409 no_ready_account): same button, words kept.
  use({ need: 'no_ready_account' });
  const b = card();
  const v2 = await mount(b.el);
  await typeArea(box(), NEED);
  await click(submitBtn());
  assert.match(document.querySelector('[data-need-error]').textContent, /Add an AI account first\./);
  assert.equal(submitBtn().textContent, 'Add an AI account first');
  assert.equal(box().value, NEED);
  await click(submitBtn());
  assert.equal(b.out.accounts, 1);
  v2.unmount();
});

test('errors keep the words and show the server’s reason (502, 429, busy, offline)', async () => {
  for (const [sc, re] of [
    ['draft_failed', /Couldn’t suggest a team this time\. Try again, or start from a ready-made team\.The draft came back unusable twice\./],
    ['limit', /Couldn’t suggest a team this time\..*Claude is out until 21:10\./],
    ['busy', /Already suggesting a team here\./],
    ['offline', /You’re offline\./],
  ]) {
    use({ need: sc });
    const { el } = card();
    const v = await mount(el);
    await typeArea(box(), NEED);
    await click(submitBtn());
    const err = document.querySelector('[data-need-error]');
    assert.ok(err, `${sc}: an error shows`);
    assert.equal(err.getAttribute('role'), 'alert');
    assert.match(err.textContent, re, sc);
    assert.equal(box().value, NEED, `${sc}: the words are kept`);
    assert.equal(box().readOnly, false, `${sc}: the box unlocks`);
    assert.equal(theSheet(), null, `${sc}: no sheet`);
    assert.equal(submitBtn().textContent, 'Suggest a team');
    v.unmount();
  }
});

test('Escape or Cancel stops Suggesting…; the words stay; after 8 s it adds "Usually under a minute."', async () => {
  const realST = window.setTimeout;
  // The 8-second note: run that one timer at once.
  window.setTimeout = (fn, ms, ...a) => realST(fn, ms === 8000 ? 0 : ms, ...a);
  try {
    const calls = use({ need: 'slow' });
    const { el } = card();
    const v = await mount(el);
    await typeArea(box(), NEED);
    await click(submitBtn());
    assert.equal(stops(calls).length, 0);
    assert.equal(submitBtn().textContent, 'Suggesting…');
    assert.equal(submitBtn().getAttribute('aria-disabled'), 'true');
    assert.equal(box().readOnly, true, 'the box locks');
    assert.match(document.querySelector('#card').textContent, /Usually under a minute\./);
    await key(box(), 'Escape');
    assert.deepEqual(stops(calls).map((c) => c.url), ['/api/teams/suggest?space=spc_demo'], 'Escape stops the server too; its 404 is ignored');
    assert.equal(document.querySelector('#card [role="alert"]'), null, 'no error for the 404');
    assert.equal(submitBtn().textContent, 'Suggest a team');
    assert.equal(box().readOnly, false);
    assert.equal(box().value, NEED);
    assert.equal(theSheet(), null);
    // Cancel does the same.
    await click(submitBtn());
    await click(byText(document.querySelector('#card'), 'button', /^Cancel$/));
    assert.equal(stops(calls).length, 2, 'Cancel too');
    assert.equal(submitBtn().textContent, 'Suggest a team');
    assert.equal(box().value, NEED);
    v.unmount();
  } finally {
    window.setTimeout = realST;
  }
});

test('Try again sends the same words again and shows the new team; Escape while asking keeps the last one', async () => {
  const calls = use({ need: 'again' });
  const { el } = card();
  const v = await mount(el);
  await typeArea(box(), NEED);
  await click(submitBtn());
  let s = theSheet();
  assert.equal(s.querySelectorAll('[data-row]').length, 4);
  await click(byText(s, 'button', /^Try again$/));
  const sent = asks(calls).map((c) => c.body);
  assert.equal(sent.length, 2);
  assert.deepEqual(sent[1], sent[0], 'the same need');
  s = theSheet();
  assert.equal(s.querySelectorAll('[data-row]').length, 3, 'the second draft');
  assert.match(s.textContent, /Baker/);
  v.unmount();

  // A slow second ask: Escape stops it and the first team stays.
  const realST = window.setTimeout;
  let calls2 = [];
  let n = 0;
  const base = fx.fixtureTransport({}, calls2);
  m.setTeamsTransport(async (url, method, body, headers) => {
    if (url.startsWith('/api/teams/suggest') && method !== 'DELETE' && ++n > 1) {
      calls2.push({ url, method, body });
      return new Promise(() => {});
    }
    return base(url, method, body, headers);
  });
  const b = card();
  const v2 = await mount(b.el);
  await typeArea(box(), NEED);
  await click(submitBtn());
  await click(byText(theSheet(), 'button', /^Try again$/));
  assert.match(theSheet().textContent, /Suggesting…/);
  assert.equal(stops(calls2).length, 0);
  await key(theSheet(), 'Escape');
  assert.equal(stops(calls2).length, 1, 'Escape during Try again stops the server’s ask');
  assert.ok(theSheet(), 'the sheet stays');
  assert.equal(theSheet().querySelectorAll('[data-row]').length, 4, 'the last team is back');
  await key(theSheet(), 'Escape');
  assert.equal(stops(calls2).length, 1, 'closing a ready sheet sends no stop');
  assert.equal(theSheet(), null, 'a second Escape closes it');
  assert.equal(box().value, NEED, 'the words are still in the box');
  window.setTimeout = realST;
  v2.unmount();
});

test('Create sends suggestion_id + edits (untick, rename) to from-template; the toast offers Undo', async () => {
  const calls = use({});
  const realFetch = g.fetch;
  const undos = [];
  g.fetch = g.window.fetch = async (url, init) => {
    if (String(url).includes('/api/bot-actions/')) {
      undos.push(String(url));
      return new Response(JSON.stringify({ id: 'act_demo', status: 'undone', can_undo: false }), { status: 200, headers: { 'Content-Type': 'application/json' } });
    }
    return new Response('{}', { status: 200, headers: { 'Content-Type': 'application/json' } });
  };
  try {
    const { el, out } = card();
    const v = await mount(el);
    await typeArea(box(), NEED);
    await click(submitBtn());
    const s = theSheet();
    // decisions-1112 §4: no Team handle field for a team from words; the project suggest flows keep it.
    assert.equal(s.querySelector('[aria-label="Team handle"]'), null);
    assert.equal(/Team handle/.test(s.textContent), false);
    // Untick the Instagram bot, rename the order keeper.
    await click(s.querySelector('[data-row="social"] [role="checkbox"]'));
    await click(s.querySelector('[aria-label="Rename OrderKeeper"]'));
    const input = s.querySelector('[aria-label^="Name for"]');
    Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(input, 'Orders');
    input.dispatchEvent(new window.Event('input', { bubbles: true }));
    await settle(2);
    await key(input, 'Enter');
    const create = byText(s, 'button', /^Create team · 3 bots$/);
    assert.ok(create, 'Create team · 3 bots');
    await click(create);
    const c = creates(calls);
    assert.equal(c.length, 1);
    assert.equal(c[0].url, '/api/teams/from-template?space=spc_demo');
    assert.deepEqual(c[0].body, {
      suggestion_id: 'sug_demo_bakery',
      edits: {
        rows: { lead: { on: true, name: 'ShopLead' }, inbox: { on: true, name: 'CustomerMail' }, social: { on: false }, orders: { on: true, name: 'Orders' } },
      },
    }, 'no handle: the server picks a free one (decisions-1112 §4)');
    assert.ok(c[0].headers?.['Idempotency-Key'], 'one idempotency key per open');
    assert.equal(theSheet(), null);
    assert.equal(out.opened.length, 1);
    const toast = byText(document.body, '[role="status"], div', /Created @bakery: 3 bots\./);
    assert.ok(toast, 'Created toast');
    await click(byText(document.body, 'button', /^\s*Undo$/));
    assert.deepEqual(undos, ['/api/bot-actions/act_demo/undo']);
    assert.ok(byText(document.body, 'div', /Removed @bakery\./));
    assert.equal(out.undone, 1);
    assert.equal(box().value, '', 'the words go after a create');
    v.unmount();
  } finally {
    g.fetch = g.window.fetch = realFetch;
  }
});

test('sidebar chip "Describe your own": focuses the card’s box; without the card it opens the box in a small sheet', async () => {
  use({});
  const { el } = card();
  const v = await mount(el);
  const chip = document.querySelector('#side [data-describe-own]');
  assert.ok(chip, 'the first chip');
  assert.equal(document.querySelector('#side [data-template-sidebar] button[data-describe-own]'), document.querySelector('#side [data-template-sidebar] .flex button'), 'it comes first');
  assert.equal(chip.textContent, 'Describe your own');
  await click(chip);
  assert.equal(document.activeElement, box(), 'focuses the card’s box');
  assert.equal(document.querySelector('[data-need-sheet]'), null);
  v.unmount();

  // No card on screen: a small sheet with the same box; Cancel in the preview brings it back.
  const calls = use({});
  const v2 = await mount(h(m.ReadyTeamsProvider, { space: 'spc_demo', bots: [], teams: [], onChanged() {}, onOpenTeam() {}, readyAccounts: 1 }, h('div', { id: 'side' }, h(m.ReadyTeamsSidebarBox, { teamCount: 0 }))));
  await click(document.querySelector('#side [data-describe-own]'));
  const ns = document.querySelector('[data-need-sheet]');
  assert.ok(ns, 'the small sheet opens');
  assert.equal(ns.getAttribute('role'), 'dialog');
  assert.match(ns.textContent, /Describe your own team/);
  const ta = ns.querySelector('textarea');
  await typeArea(ta, NEED);
  await key(ta, 'Enter');
  assert.equal(asks(calls).length, 1);
  assert.equal(document.querySelector('[data-need-sheet]'), null, 'it steps aside for the preview');
  assert.ok(theSheet());
  await key(theSheet(), 'Escape');
  const back = document.querySelector('[data-need-sheet]');
  assert.ok(back, 'Cancel brings it back');
  assert.equal(back.querySelector('textarea').value, NEED, 'with the words');
  await key(back, 'Escape');
  assert.equal(document.querySelector('[data-need-sheet]'), null);
  v2.unmount();
});

test('New team: "Describe what this team should do" above the form; Suggest members from the code stays', async () => {
  const calls = use({});
  let closed = 0;
  const v = await mount(
    h(m.ReadyTeamsProvider, { space: 'spc_demo', bots: [], teams: [], onChanged() {}, onOpenTeam() {}, readyAccounts: 1 }, h(m.NewTeamModal, { isOpen: true, onClose: () => closed++, kwants: [] })),
  );
  const ta = [...document.querySelectorAll('textarea')].find((t) => t.labels?.[0]?.textContent === 'Describe what this team should do');
  assert.ok(ta, 'the box with its label');
  const form = document.querySelector('form');
  assert.ok(ta.compareDocumentPosition(form) & 4, 'above the manual form');
  await typeArea(ta, NEED);
  await key(ta, 'Enter');
  assert.equal(asks(calls).length, 1);
  const dlg = document.querySelector('.kw-dialog');
  assert.equal(dlg.hidden, true, 'New team hides while the preview is open');
  await key(theSheet(), 'Escape');
  assert.equal(dlg.hidden, false, 'and comes back on Cancel');
  assert.equal(ta.value, NEED);
  await key(ta, 'Enter');
  await click(byText(theSheet(), 'button', /^Create team · 4 bots$/));
  assert.equal(creates(calls).length, 1);
  assert.equal(closed, 1, 'New team closes after a create');
  assert.match(readFileSync(new URL('../src/components/NewTeamModal.tsx', import.meta.url), 'utf8'), /<SuggestMembersButton/);
  v.unmount();
});

test('phone (390 px): the preview is a bottom sheet; the box’s buttons are 44 px tall', async () => {
  phone = true;
  try {
    use({});
    const { el } = card();
    const v = await mount(el);
    assert.match(submitBtn().className, /\bh-11\b/);
    await typeArea(box(), NEED);
    await click(submitBtn());
    const s = theSheet();
    assert.match(s.className, /bottom-0/);
    assert.match(s.className, /rounded-t-2xl/);
    v.unmount();
  } finally {
    phone = false;
  }
});

test('never renders HTML from the server: instructions and owns are text', async () => {
  const calls = [];
  const base = fx.fixtureTransport({}, calls);
  m.setTeamsTransport(async (url, method, body, headers) => {
    const r = await base(url, method, body, headers);
    if (url.startsWith('/api/teams/suggest')) {
      const b = structuredClone(r.body);
      b.bots[1].instructions = '<img src=x onerror="window.__pwned=1">Hi';
      b.bots[1].owns = ['<b>emails</b>'];
      return { ...r, body: b };
    }
    return r;
  });
  const { el } = card();
  const v = await mount(el);
  await typeArea(box(), NEED);
  await click(submitBtn());
  const row = theSheet().querySelector('[data-row="inbox"]');
  await click(byText(row, 'button', /^Instructions/));
  assert.equal(row.querySelector('img, b'), null);
  assert.match(row.textContent, /<img src=x/);
  assert.equal(window.__pwned, undefined);
  v.unmount();
});
