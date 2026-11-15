// 0.7.6 "Work from Jira", wave 2 (Pixel): the setup sheet, the settings row, the Dev desk hint and the
// badge rule. Example hosts and fake tokens only; nothing leaves this process.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
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
// Anything not routed through the Jira transport (the password store) answers "open".
g.fetch = g.window.fetch = async (url) =>
  new Response(JSON.stringify(String(url).includes('/api/passwords/store') ? { store: 'bitwarden', unlocked: true } : {}), { status: 200, headers: { 'Content-Type': 'application/json' } });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * from './src/dev/jiraSetup';
  export * as fx from './src/dev/jiraFixtures';
  export { JiraSetupForm, JiraSetupHost, openJiraSetup, suggestTeam } from './src/components/JiraSetup';
  export { JiraHint, JiraSettings } from './src/components/JiraSettings';
  export { devTeamRefs } from './src/devMode';
  export { popoverPlace } from './src/components/GitHubConnect';
`);
const { React, createRoot, fx } = m;
const h = React.createElement;
const settle = async (n = 10) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const typeIn = (el, value) => {
  const proto = el.tagName === 'TEXTAREA' ? window.HTMLTextAreaElement.prototype : el.tagName === 'SELECT' ? window.HTMLSelectElement.prototype : window.HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, value);
  el.dispatchEvent(new window.Event(el.tagName === 'SELECT' ? 'change' : 'input', { bubbles: true }));
};
const blur = async (el) => {
  el.dispatchEvent(new window.FocusEvent('focusout', { bubbles: true }));
  el.dispatchEvent(new window.FocusEvent('blur'));
  await settle();
};
const mount = async (el) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  return { host, root, unmount: () => (root.unmount(), host.remove()) };
};
const FAKE = 'demo-not-a-token-1234';
const ANN = { id: 'k_ann', name: 'Ann', avatar: 'frontend' };
const TEAMS = [
  { id: 'team_shop', name: 'Storefront team', lead: ANN, members: [ANN, { id: 'k_ben', name: 'Ben', avatar: 'backend' }] },
  { id: 'team_ops', name: 'Ops' },
];
const form = (sc, calls = []) => {
  m.setJiraTransport(fx.fxTransport(sc, calls));
  return calls;
};

// ---------------------------------------------------------------------------------------------------
// Pure rules

test('the address: Cloud from the host, anything else asked of the server', () => {
  assert.equal(m.normaliseJiraUrl('yourcompany.atlassian.net'), 'https://yourcompany.atlassian.net');
  assert.equal(m.normaliseJiraUrl('https://jira.example.com/secure/Dashboard.jspa'), 'https://jira.example.com');
  assert.equal(m.normaliseJiraUrl('https://yourcompany.atlassian.net/browse/TRAIL-151'), 'https://yourcompany.atlassian.net');
  assert.equal(m.normaliseJiraUrl('https://www.example.com/jira/browse/TRAIL-1'), 'https://www.example.com/jira');
  assert.equal(m.kindFromHost('https://yourcompany.atlassian.net'), 'cloud');
  assert.equal(m.kindFromHost('https://jira.example.com'), null);
  assert.equal(m.normaliseJiraUrl('not a url at all'), null);
});

test('step 1 errors: token, address, the server words for invalid; no_projects and rate limits are not step-1 errors', () => {
  const c = (o) => m.parseCheck({ ...fx.fxFound('cloud'), ...o });
  assert.deepEqual(m.step1Error(c({ result: 'auth_failed' }), 'cloud', 'oleks@example.com'), {
    field: 'token',
    text: 'Jira didn’t accept this token. Check it was copied whole and belongs to oleks@example.com. Nothing was saved.',
  });
  assert.equal(m.step1Error(c({ result: 'auth_failed' }), 'dc', 'x@example.com').text, 'Jira didn’t accept this token. Check it was copied whole. Nothing was saved.');
  assert.equal(m.step1Error(c({ result: 'not_jira' }), 'cloud', null).field, 'address');
  assert.equal(m.step1Error(c({ result: 'unreachable' }), 'cloud', null).text, m.JIRA_COPY.notJira);
  assert.equal(m.step1Error(c({ result: 'invalid', message: 'email is required for Cloud' }), 'cloud', null).field, 'email');
  assert.equal(m.step1Error(c({ result: 'no_projects' }), 'cloud', null), null);
  assert.equal(m.step1Error(c({ result: 'rate_limited' }), 'cloud', null), null);
  assert.match(m.noProjectsText('oleks@example.com'), /can’t see any projects.*oleks@example\.com/);
});

test('parse: the suggested project must be one of the projects; the row lists are JSON strings', () => {
  const c = m.parseCheck({ ...fx.fxFound('cloud'), suggested_project: { key: 'NOPE', branches: 3 } });
  assert.equal(c.suggested_project, null);
  assert.equal(m.parseCheck(fx.fxFound('cloud')).suggested_project.key, 'TRAIL');
  const s = m.parseSite(fx.fxSiteRow({ handover: 'either', on_: false }));
  assert.deepEqual(s.projects, ['TRAIL']);
  assert.deepEqual(s.spaces, ['SHOP', 'ENG']);
  assert.equal(s.handover, 'jql');
  assert.equal(s.on, false);
  assert.equal(m.parseSite({ site: null }), null);
});

test('the JQL under Advanced is built from one rule', () => {
  assert.equal(m.builtJql({ projects: ['TRAIL'], handover: 'label', label: 'kwant', assignee_id: null }), 'project = TRAIL AND labels = kwant AND statusCategory != Done ORDER BY created ASC');
  assert.equal(
    m.builtJql({ projects: ['TRAIL', 'SUP'], handover: 'assignee', label: 'kwant', assignee_id: 'abc' }),
    'project in (TRAIL, SUP) AND assignee = "abc" AND statusCategory != Done ORDER BY created ASC',
  );
  assert.match(m.builtJql({ projects: ['A'], handover: 'label', label: 'my label', assignee_id: null }), /labels = "my label"/);
});

test('the badge counts needs, not news: backfill Waiting never counts; amber only when a ticket waits on you', () => {
  const T = (state) => ({ state });
  // never connected = the 0.7.5 badge, word for word
  assert.deepEqual(m.deskBadge(0, null), { count: 0, tickets: 0, reviews: 0, tone: 'accent', label: undefined, lead: null });
  assert.equal(m.deskBadge(2, null).label, 'Dev desk, 2 new reviews');
  assert.equal(m.deskBadge(1, null).label, 'Dev desk, 1 new review');
  // waiting (backfill), picked up, reading, working: not counted
  const quiet = m.deskBadge(0, ['waiting', 'waiting', 'picked_up', 'reading', 'working'].map(T));
  assert.equal(quiet.count, 0);
  assert.equal(quiet.tone, 'accent');
  const b = m.deskBadge(1, ['needs_answer', 'working', 'waiting', 'waiting'].map(T));
  assert.equal(b.count, 2);
  assert.equal(b.tone, 'needs');
  assert.equal(b.lead, '1 ticket needs you');
  assert.equal(b.label, 'Dev desk, 1 ticket needs you, 1 review open');
  assert.equal(m.deskBadge(0, ['brief_ready', 'needs_answer'].map(T)).label, 'Dev desk, 2 tickets need you');
  assert.equal(m.ticketSummary(fx.fxTickets().tickets), '1 ticket needs you, 1 being built, 2 waiting for your Start');
});

test('the team suggestion matches a word of the project name, else the only team, else none', () => {
  assert.equal(m.suggestTeam(TEAMS, 'Storefront').team.id, 'team_shop');
  assert.match(m.suggestTeam(TEAMS, 'Storefront').why, /@Storefront team works on storefront/);
  assert.equal(m.suggestTeam(TEAMS, 'Payroll'), null);
  assert.equal(m.suggestTeam([TEAMS[1]], 'Payroll').why, null);
});

test('popoverPlace takes the Jira width and still clamps to the window', () => {
  assert.equal(m.popoverPlace(null, 1440, 900, 400, 448).width, 448);
  assert.equal(m.popoverPlace(null, 390, 844, 400, 448).width, 366);
  assert.equal(m.popoverPlace(null, 1440, 900, 400).width, 380);
});

// ---------------------------------------------------------------------------------------------------
// Off means unchanged

test('never connected: no hint or row until GET jira answers; an older server (404) shows nothing at all', async () => {
  m.jiraStore._reset();
  form({ site: 'missing' });
  let r = await mount(h('div', null, h(m.JiraHint, { space: 'spc_old' }), h(m.JiraSettings, { space: 'spc_old' })));
  await settle();
  assert.equal(r.host.textContent, '', 'a server without the Jira routes changes nothing');
  assert.equal(r.host.querySelector('[data-jira-hint],[data-jira-settings]'), null);
  r.unmount();

  m.jiraStore._reset();
  const calls = form({ site: null });
  r = await mount(h('div', null, h(m.JiraHint, { space: 'spc_demo' }), h(m.JiraSettings, { space: 'spc_demo' })));
  await settle();
  assert.match(r.host.textContent, /Use Jira\? Your bots can take tickets you hand them\./);
  assert.ok(r.host.querySelector('[data-jira-hint-open]').textContent.includes('Take work from Jira'));
  const row = r.host.querySelector('[data-jira-settings="off"]');
  assert.ok(row);
  assert.match(row.textContent, /Work from Jira\s*Off\s*Set up/);
  // one GET jira, and no tickets call while never connected
  assert.deepEqual(
    calls.map((c) => `${c.method} ${c.path.replace(/.*\/dev/, '')}`),
    ['GET /jira'],
  );
  r.unmount();
});

test('the hint hides per space and stays hidden; the settings row stays the way back', async () => {
  m.jiraStore._reset();
  localStorage.clear();
  form({ site: null });
  let r = await mount(h('div', null, h(m.JiraHint, { space: 'spc_a' }), h(m.JiraSettings, { space: 'spc_a' })));
  await settle();
  const hide = r.host.querySelector('[data-jira-hint-hide]');
  assert.equal(hide.textContent, 'Hide');
  assert.equal(hide.getAttribute('aria-label'), 'Hide the Jira hint in this space');
  assert.match(hide.className, /px-1\.5 py-0\.5 -mr-1\.5 rounded-md/);
  await click(hide);
  assert.equal(r.host.querySelector('[data-jira-hint]'), null);
  assert.equal(localStorage.getItem('kwnt:jira-hint-hidden:spc_a'), '1');
  assert.ok(r.host.querySelector('[data-jira-settings="off"]'), 'Settings still offers Set up');
  r.unmount();
  // a remount in the same space keeps it hidden; another space still shows it
  r = await mount(h('div', null, h(m.JiraHint, { space: 'spc_a' }), h(m.JiraHint, { space: 'spc_b' })));
  await settle();
  assert.equal(r.host.querySelectorAll('[data-jira-hint]').length, 1);
  r.unmount();
  localStorage.clear();
});

// ---------------------------------------------------------------------------------------------------
// The sheet

const openSheet = async (sc, props = {}) => {
  const calls = form(sc);
  const r = await mount(
    h(m.JiraSetupForm, { space: 'spc_demo', teams: TEAMS, site: null, phone: false, onCancel() {}, onSaved: props.onSaved ?? (() => {}), initialGate: 'open', recountMs: 5, ...props }),
  );
  return { ...r, calls, $: (s) => r.host.querySelector(s) };
};
const step1 = async (s, { address = 'yourcompany.atlassian.net', email = 'oleks@example.com', token = FAKE } = {}) => {
  typeIn(s.$('[data-jira-address]'), address);
  await settle();
  await blur(s.$('[data-jira-address]'));
  await settle();
  if (s.$('[data-jira-email]') && email) typeIn(s.$('[data-jira-email]'), email);
  if (s.$('[data-jira-token]') && token) typeIn(s.$('[data-jira-token]'), token);
  await settle();
};

test('Cloud: found from the host, then the check opens step 2 with the suggested project ticked', async () => {
  const s = await openSheet({});
  await step1(s);
  assert.equal(s.$('[data-jira-kind]').dataset.jiraKind, 'cloud');
  assert.match(s.$('[data-jira-kind]').textContent, /Found Jira Cloud\./);
  assert.ok(!s.calls.some((c) => c.path.endsWith('/jira/check')), 'a Cloud host needs no probe');
  assert.match(s.host.textContent, /Nothing is saved until the check passes\./);
  await click(s.$('[data-jira-check]'));
  await settle(20);
  assert.equal(s.$('[data-jira-setup]').dataset.stage, '2');
  assert.match(s.$('[data-jira-done]').textContent, /yourcompany\.atlassian\.net · oleks@example\.com/);
  assert.equal(s.$('[data-jira-pick="TRAIL"]').getAttribute('aria-checked'), 'true');
  assert.match(s.$('[data-jira-pick="TRAIL"]').textContent, /in your branch names/);
  assert.equal(s.$('[data-jira-pick="SUP"]').getAttribute('aria-checked'), 'false');
  assert.match(s.$('[data-jira-more]').textContent, /\+4 more/);
  // one rule, label kwant by default
  assert.equal(s.$('[data-jira-handover]').dataset.jiraHandover, 'label');
  assert.equal(s.host.querySelectorAll('[data-jira-handover] [role="radio"]').length, 2);
  // both "on its own" choices start off
  assert.equal(s.$('[data-jira-toggle="wait"]').getAttribute('aria-checked'), 'false');
  assert.equal(s.$('[data-jira-toggle="comment"]').getAttribute('aria-checked'), 'false');
  // the team is suggested from the project name
  assert.equal(s.$('[data-jira-team]').value, 'team_shop');
  assert.match(s.$('[data-jira-team-why]').textContent, /Suggested/);
  // the live result counts after 400 ms (5 in tests)
  await wait(30);
  await settle();
  assert.match(s.$('[data-jira-live]').textContent, /3 open tickets match right now/);
  assert.match(s.$('[data-jira-live]').textContent, /TRAIL-151/);
  assert.equal(s.$('[data-jira-live]').getAttribute('role'), 'status');
  s.unmount();
});

test('Data Center: asked of the server ("Looks like" until it answers), then the Confluence address and token', async () => {
  const s = await openSheet({});
  typeIn(s.$('[data-jira-address]'), 'jira.example.com');
  await settle();
  await blur(s.$('[data-jira-address]'));
  await settle(20);
  const probe = s.calls.find((c) => c.path.endsWith('/jira/check'));
  assert.ok(probe, 'a non-Cloud host is probed');
  assert.equal(probe.body.token, undefined, 'the probe sends no token');
  assert.equal(probe.body.detect, true, 'the probe asks for detection only (api-decisions §8)');
  assert.match(s.$('[data-jira-kind]').textContent, /Found Jira Data Center \(your company runs it\)\./);
  assert.equal(s.$('[data-jira-email]'), null, 'Data Center signs in with a personal access token only');
  assert.match(s.$('label[for$="-tok"]').textContent, /Personal access token/);
  typeIn(s.$('[data-jira-token]'), FAKE);
  await click(s.$('[data-jira-check]'));
  await settle(20);
  assert.ok(s.$('[data-jira-confluence-dc]'), 'DC asks for the Confluence address and token');
  assert.match(s.$('[data-jira-confluence-dc]').textContent, /Data Center keeps Confluence separate/);
  s.unmount();
});

test('Not right? Change flips the kind', async () => {
  const s = await openSheet({});
  await step1(s, { token: null });
  await click(s.$('[data-jira-kind-change]'));
  assert.equal(s.$('[data-jira-kind]').dataset.jiraKind, 'dc');
  assert.match(s.$('[data-jira-kind]').textContent, /Looks like Jira Data Center/);
  s.unmount();
});

// D5 (Rigel, #395 QA): the address check ran again when the address lost focus to a click into
// Email, put the Data Center guess back first, and so unmounted Email under the click.
const press = async (el, key) => {
  el.dispatchEvent(new window.KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
  await settle();
};
const clickInto = async (from, to) => {
  // A mouse click: the old field blurs, the new one gets focus (no settle between, as in a browser).
  from.dispatchEvent(new window.FocusEvent('focusout', { bubbles: true, relatedTarget: to }));
  from.dispatchEvent(new window.FocusEvent('blur', { relatedTarget: to }));
  to.focus();
  to.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
};

test('D5: Enter runs the address check, then a click into Email keeps what you type there', async () => {
  const s = await openSheet({ detectKind: 'cloud', delayMs: 15 });
  const addr = s.$('[data-jira-address]');
  addr.focus();
  typeIn(addr, 'jira.example.com');
  await settle();
  await press(addr, 'Enter');
  await wait(40);
  await settle();
  assert.equal(s.$('[data-jira-kind]').dataset.jiraKind, 'cloud', 'the server said Cloud');
  const probes = () => s.calls.filter((c) => c.path.endsWith('/jira/check')).length;
  assert.equal(probes(), 1);
  const mail = s.$('[data-jira-email]');
  assert.ok(mail, 'Email shows once the check resolved');
  await clickInto(addr, mail);
  typeIn(mail, 'oleks@example.com');
  await wait(40);
  await settle();
  assert.equal(s.$('[data-jira-email]')?.value, 'oleks@example.com', 'what you typed into Email is still there');
  assert.equal(s.$('[data-jira-email]'), mail, 'Email stays the same field');
  assert.equal(document.activeElement, mail, 'focus stays in Email');
  assert.equal(probes(), 1, 'the same address is not checked twice');
  // The token field too: type it, go back to the address and out again, nothing moves or empties.
  const tok = s.$('[data-jira-token]');
  await clickInto(mail, tok);
  typeIn(tok, FAKE);
  await clickInto(tok, addr);
  await clickInto(addr, tok);
  await wait(40);
  await settle();
  assert.equal(s.$('[data-jira-token]'), tok);
  assert.equal(tok.value, FAKE);
  assert.equal(s.$('[data-jira-email]').value, 'oleks@example.com');
  assert.equal(probes(), 1);
  // A changed address is checked again.
  typeIn(addr, 'jira2.example.com');
  await settle();
  await press(addr, 'Enter');
  await wait(40);
  await settle();
  assert.equal(probes(), 2);
  s.unmount();
});

test('D5: an answer for an address that changed meanwhile is dropped', async () => {
  const s = await openSheet({ delayMs: 150 });
  const addr = s.$('[data-jira-address]');
  typeIn(addr, 'jira.example.com');
  await settle();
  await press(addr, 'Enter');
  typeIn(addr, 'yourcompany.atlassian.net');
  await settle();
  await press(addr, 'Enter');
  assert.equal(s.$('[data-jira-kind]').dataset.jiraKind, 'cloud');
  await wait(250);
  await settle();
  assert.equal(s.$('[data-jira-kind]').dataset.jiraKind, 'cloud', 'the late Data Center answer does not win');
  assert.ok(s.$('[data-jira-email]'));
  s.unmount();
});

test('errors: a rejected token, an address with no Jira, no projects visible (its own error), slow down', async () => {
  let s = await openSheet({ check: 'auth_failed' });
  await step1(s);
  await click(s.$('[data-jira-check]'));
  await settle(20);
  assert.match(s.$('[data-jira-error]').textContent, /Jira didn’t accept this token\. Check it was copied whole and belongs to oleks@example\.com\. Nothing was saved\./);
  assert.equal(s.$('[data-jira-token]').getAttribute('aria-invalid'), 'true');
  assert.equal(s.$('[data-jira-setup]').dataset.stage, '1');
  s.unmount();

  s = await openSheet({});
  await step1(s, { address: 'yourcompny.atlassian.net' });
  await click(s.$('[data-jira-check]'));
  await settle(20);
  assert.match(s.$('[data-jira-error]').textContent, /No Jira answered at this address/);
  assert.equal(s.$('[data-jira-address]').getAttribute('aria-invalid'), 'true');
  s.unmount();

  s = await openSheet({ check: 'no_projects' });
  await step1(s);
  await click(s.$('[data-jira-check]'));
  await settle(20);
  assert.ok(s.$('[data-jira-no-projects]'));
  assert.match(s.$('[data-jira-no-projects]').textContent, /This token can’t see any projects\. Ask a Jira admin to give oleks@example\.com access/);
  assert.equal(s.$('[data-jira-error]'), null, 'not the token error');
  s.unmount();

  s = await openSheet({ check: 'rate_limited', retryAt: new Date(2026, 10, 7, 10, 9).toISOString() });
  await step1(s);
  await click(s.$('[data-jira-check]'));
  await settle(20);
  assert.match(s.$('[data-jira-slow]').textContent, /Jira asked us to slow down\. Trying again at 10:09 AM\./);
  s.unmount();
});

test('the Checking state holds the button and says so', async () => {
  const s = await openSheet({ delayMs: 80 });
  await step1(s);
  await click(s.$('[data-jira-check]'));
  assert.equal(s.$('[data-jira-check]').disabled, true);
  assert.match(s.$('[data-jira-check]').textContent, /Checking/);
  assert.match(s.$('[data-jira-foot]').textContent, /Checking with Jira…/);
  await wait(120);
  await settle(20);
  assert.equal(s.$('[data-jira-setup]').dataset.stage, '2');
  s.unmount();
});

test('the store card comes first, with the Jira sentence', async () => {
  form({});
  const r = await mount(h(m.JiraSetupForm, { space: 'spc_demo', teams: [], site: null, phone: false, onCancel() {}, onSaved() {}, initialGate: 'store' }));
  typeIn(r.host.querySelector('[data-jira-address]'), 'yourcompany.atlassian.net');
  await blur(r.host.querySelector('[data-jira-address]'));
  const why = r.host.querySelector('[data-save-first-why]');
  assert.ok(why);
  assert.match(why.textContent, /A scoped token that can only read Jira and Confluence keeps that risk small\./);
  assert.equal(r.host.querySelector('[data-jira-token]'), null, 'the token field waits for a way on');
  await click(r.host.querySelector('[data-save-first-save]'));
  assert.ok(r.host.querySelector('[data-jira-token]'));
  r.unmount();
});

test('Advanced: editing the search sets "Set by your search"; assignee or label is possible only there', async () => {
  const s = await openSheet({});
  await step1(s);
  await click(s.$('[data-jira-check]'));
  await settle(20);
  const jql = s.$('[data-jira-jql]');
  assert.equal(jql.value, 'project = TRAIL AND labels = kwant AND statusCategory != Done ORDER BY created ASC');
  assert.match(jql.className, /font-variant-ligatures:none/);
  typeIn(jql, 'project = TRAIL AND (labels = kwant OR assignee = currentUser()) AND statusCategory != Done');
  await settle();
  assert.ok(s.$('[data-jira-set-by-search]'));
  assert.equal(s.$('[data-jira-handover]'), null);
  assert.equal(s.$('[data-jira-poll]').value, '300');
  s.unmount();
});

test('Start taking tickets saves once; the token is sent, never kept in the DOM or the store', async () => {
  m.jiraStore._reset();
  let saved = 0;
  const s = await openSheet({}, { onSaved: () => saved++ });
  await step1(s);
  await click(s.$('[data-jira-check]'));
  await settle(20);
  await wait(30);
  await click(s.$('[data-jira-save]'));
  await settle(20);
  assert.equal(saved, 1);
  const put = s.calls.find((c) => c.method === 'PUT');
  assert.equal(put.body.token, '[redacted]', 'the token went to the server');
  assert.deepEqual(put.body.projects, ['TRAIL']);
  assert.equal(put.body.handover, 'label');
  assert.equal(put.body.label, 'kwant');
  assert.equal(put.body.comment_progress, false);
  assert.equal(put.body.wait_for_ok, false);
  assert.equal(put.body.team_id, 'team_shop');
  assert.equal(put.body.poll_secs, 300);
  assert.ok(!s.host.innerHTML.includes(FAKE), 'no token in the DOM');
  assert.ok(!JSON.stringify(m.jiraStore.get('spc_demo') ?? {}).includes(FAKE), 'no token in the store');
  assert.ok(!localStorage.length || !JSON.stringify({ ...localStorage }).includes(FAKE), 'no token in storage');
  s.unmount();
});

test('a 422 on save goes back to step 1 with the reason', async () => {
  const s = await openSheet({});
  await step1(s);
  await click(s.$('[data-jira-check]'));
  await settle(20);
  m.setJiraTransport(fx.fxTransport({ check: 'auth_failed' }, s.calls));
  await click(s.$('[data-jira-save]'));
  await settle(20);
  assert.equal(s.$('[data-jira-setup]').dataset.stage, '1');
  assert.match(s.$('[data-jira-error]').textContent, /didn’t accept this token/);
  s.unmount();
});

test('phone: the hand-over choice stacks and the picks are 44 px', async () => {
  const s = await openSheet({}, { phone: true });
  await step1(s);
  await click(s.$('[data-jira-check]'));
  await settle(20);
  assert.match(s.$('[data-jira-handover]').className, /grid-flow-row/);
  assert.match(s.$('[data-jira-pick="TRAIL"]').className, /min-h-\[44px\]/);
  s.unmount();
});

test('connected: the settings box shows the status line from GET tickets and Turn off asks first', async () => {
  m.jiraStore._reset();
  const calls = form({ site: fx.fxSiteRow() });
  const r = await mount(h(m.JiraSettings, { space: 'spc_demo', teams: [{ id: 'team_shop', name: 'Shop team' }] }));
  await settle(20);
  const box = r.host.querySelector('[data-jira-settings="on"]');
  assert.ok(box);
  assert.match(box.querySelector('[data-jira-status]').textContent, /1 ticket needs you, 1 being built, 2 waiting for your Start/);
  assert.match(box.textContent, /@Shop team/);
  await click(box.querySelector('[data-jira-turn-off]'));
  assert.ok(r.host.querySelector('[data-jira-off-confirm]'));
  await click(r.host.querySelector('[data-jira-off-yes]'));
  await settle(20);
  assert.ok(calls.some((c) => c.method === 'DELETE'));
  assert.ok(r.host.querySelector('[data-jira-settings="off"]'));
  r.unmount();
});

// ---------------------------------------------------------------------------------------------------
// Mira's shots review (A2, 8, 18, 19)

test('A2: hiding the hint says so politely, and nothing visual is left', async () => {
  m.jiraStore._reset();
  localStorage.clear();
  form({ site: null });
  const r = await mount(h(m.JiraHint, { space: 'spc_live', teams: TEAMS }));
  await settle(20);
  await click(r.host.querySelector('[data-jira-hint-hide]'));
  assert.equal(r.host.querySelector('[data-jira-hint]'), null);
  const live = r.host.querySelector('[data-jira-hint-live]');
  assert.equal(live.getAttribute('aria-live'), 'polite');
  assert.match(live.className, /sr-only/);
  assert.equal(live.textContent, 'Hidden. Set it up any time in Settings › Dev mode.');
  r.unmount();
});

test('23: a fresh browser shows project and space names straight from GET jira, with no setup check first', async () => {
  m.jiraStore._reset();
  localStorage.clear();
  const calls = form({ site: fx.fxSiteRow() });
  const r = await mount(h(m.JiraSettings, { space: 'spc_demo', teams: [] }));
  await settle(20);
  const box = r.host.querySelector('[data-jira-settings="on"]');
  assert.match(box.querySelector('[data-jira-projects]').textContent, /TRAIL\s*Storefront/);
  assert.match(box.textContent, /SHOP Shop handbook, ENG Engineering/);
  assert.ok(!calls.some((c) => c.path.endsWith('/dev/jira/check')), 'no check was needed for the names');
  assert.equal(Object.keys(localStorage).filter((k) => /jira-names/.test(k)).length, 0, 'nothing remembered in the browser');
  r.unmount();
});

test('23: a key with no name shows alone; names are parsed as untrusted text', async () => {
  const site = m.parseSite(fx.fxSiteRow({ projects: '["TRAIL","SUP"]', project_names: { TRAIL: 'Storefront', 'bad key': 'x', SUP: 7 }, space_names: null }));
  assert.deepEqual(site.project_names, { TRAIL: 'Storefront' });
  assert.deepEqual(site.space_names, {});
  m.jiraStore._reset();
  form({ site: fx.fxSiteRow({ projects: '["TRAIL","SUP"]', project_names: { TRAIL: 'Storefront' }, space_names: {} }) });
  const r = await mount(h(m.JiraSettings, { space: 'spc_demo', teams: [] }));
  await settle(20);
  const box = r.host.querySelector('[data-jira-settings="on"]');
  assert.match(box.querySelector('[data-jira-projects]').textContent, /^TRAIL\s*StorefrontSUP$/);
  assert.match(box.textContent, /ConfluenceSHOP, ENG/);
  r.unmount();
});

test('23: PUT jira never sends project_names or space_names back', async () => {
  const calls = form({});
  const f = { jira_url: 'https://yourcompany.atlassian.net', kind: 'cloud', email: 'oleks@example.com', token: 't', projects: ['TRAIL'], handover: 'label', label: 'kwant' };
  await m.jiraApi.save('spc_demo', f, { spaces: [], team_id: null, comment_progress: false, wait_for_ok: false, poll_secs: 300, project_names: { TRAIL: 'x' }, space_names: { SHOP: 'y' } });
  const put = calls.find((c) => c.method === 'PUT');
  assert.ok(put);
  assert.ok(!('project_names' in put.body) && !('space_names' in put.body));
});

test('8: the settings box names where the token lives, its scope, projects, spaces and the lead; Check now runs a pass', async () => {
  m.jiraStore._reset();
  localStorage.clear();
  const calls = form({ site: fx.fxSiteRow() });
  const r = await mount(h(m.JiraSettings, { space: 'spc_demo', teams: [{ id: 'team_shop', name: 'Shop team', lead: ANN, members: [ANN] }] }));
  await settle(20);
  const box = r.host.querySelector('[data-jira-settings="on"]');
  assert.equal(box.querySelector('[data-jira-token-line]').textContent, 'Saved in Bitwarden · scoped: reads Jira and Confluence');
  assert.match(box.querySelector('[data-jira-projects]').textContent, /TRAIL\s*Storefront/);
  assert.match(box.textContent, /SHOP Shop handbook, ENG Engineering/);
  const who = box.querySelector('[data-jira-who]');
  assert.ok(who.querySelector('[data-team-faces]'));
  assert.match(who.textContent, /@Shop team· Ann leads/);
  const btns = [...box.querySelectorAll('button')].map((b) => b.textContent.trim());
  assert.deepEqual(btns.slice(0, 3), ['Edit', 'Check now', 'Turn off']);
  await click(box.querySelector('[data-jira-check-now]'));
  await settle(20);
  assert.ok(calls.some((c) => c.path.endsWith('/dev/jira/poll') && c.method === 'POST'));
  r.unmount();
});

test('Check now: each skipped code reads as a sentence, never the raw code', async () => {
  const want = {
    no_site: 'Jira isn’t set up for this space yet.',
    off: 'Work from Jira is off. Turn it on to check for tickets.',
    dev_mode_off: 'Dev mode is off for this space, so bots don’t check Jira.',
    no_team: 'Choose the team that works on these tickets, then check again.',
    backing_off: 'Jira asked for a slower pace. The next check runs on its own in a few minutes.',
    busy: 'A check is already running. Try again in a moment.',
    something_new: 'Jira wasn’t checked this time.',
    constructor: 'Jira wasn’t checked this time.',
  };
  for (const [code, text] of Object.entries(want)) {
    m.setJiraTransport(async () => new Response(JSON.stringify({ matched: 0, skipped: code, error: null }), { status: 200, headers: { 'Content-Type': 'application/json' } }));
    const got = await m.jiraApi.poll('spc_demo');
    assert.equal(got, text, code);
    assert.doesNotMatch(got, /Skipped:|_/, code);
  }
  m.setJiraTransport(async () => new Response(JSON.stringify({ matched: 4, skipped: null, error: null }), { status: 200, headers: { 'Content-Type': 'application/json' } }));
  assert.equal(await m.jiraApi.poll('spc_demo'), null, 'a pass that ran has no note');
  m.setJiraTransport(async () => new Response(JSON.stringify({ skipped: 'no_team', error: 'Jira answered 500.' }), { status: 200, headers: { 'Content-Type': 'application/json' } }));
  assert.equal(await m.jiraApi.poll('spc_demo'), 'Jira answered 500.', 'the server’s error wins');
});

test('Check now: the settings box shows the no_team sentence', async () => {
  m.jiraStore._reset();
  localStorage.clear();
  const inner = fx.fxTransport({ site: fx.fxSiteRow() });
  m.setJiraTransport(async (url, init) =>
    url.split('?')[0].endsWith('/dev/jira/poll') ? new Response(JSON.stringify({ matched: 0, skipped: 'no_team', error: null }), { status: 200, headers: { 'Content-Type': 'application/json' } }) : inner(url, init),
  );
  const r = await mount(h(m.JiraSettings, { space: 'spc_demo', teams: [{ id: 'team_shop', name: 'Shop team', lead: ANN, members: [ANN] }] }));
  await settle(20);
  await click(r.host.querySelector('[data-jira-check-now]'));
  await settle(20);
  assert.equal(r.host.querySelector('[data-jira-poll-note]').textContent, 'Choose the team that works on these tickets, then check again.');
  r.unmount();
});

test('8: no scope note, no scope; a local store reads "Saved on this server"', () => {
  assert.equal(m.tokenLine('local', null), 'Saved on this server');
  assert.equal(m.tokenLine(null, 'scoped: reads Jira and Confluence'), 'Saved on this server · scoped: reads Jira and Confluence');
  assert.equal(m.tokenLine('bitwarden', null), 'Saved in Bitwarden');
  assert.equal(m.parseSite(fx.fxSiteRow({ scope_note: null })).scope_note, null);
});

test('devTeamRefs: the lead first, then the members, from the roster', () => {
  const [t] = m.devTeamRefs(
    [{ id: 't', name: 'Shop team', lead_kwant_id: 'a', member_kwants: ['b', 'a', 'gone'] }],
    [
      { id: 'a', name: 'Ann', avatar_sprite: 'frontend' },
      { id: 'b', name: 'Ben', avatar_sprite: 'backend' },
    ],
  );
  assert.equal(t.lead.name, 'Ann');
  assert.deepEqual(
    t.members.map((k) => k.name),
    ['Ann', 'Ben'],
  );
});

test('8 + 19: step 2 shows the team faces and "Ann leads and reads every ticket"; Off names the lead', async () => {
  const s = await openSheet({});
  await step1(s);
  await click(s.$('[data-jira-check]'));
  await settle(20);
  const face = s.$('[data-jira-team-face]');
  assert.equal(face.querySelectorAll('[data-team-faces] > *').length, 2);
  assert.match(face.textContent, /@Storefront team· Ann leads and reads every ticket/);
  assert.match(s.host.textContent, /Off: Ann writes the brief, asks if something is unclear, then builds\./);
  assert.equal(m.waitForOkText(null), 'Off: the lead writes the brief, asks if something is unclear, then builds. On: you OK every brief first.');
  s.unmount();
});

test('18: the phone footer has Cancel and the primary at 44 px and no "Change any of this later" note', async () => {
  const s = await openSheet({}, { phone: true });
  await step1(s);
  await click(s.$('[data-jira-check]'));
  await settle(20);
  const foot = s.$('[data-jira-foot]');
  assert.ok(!foot.textContent.includes('Change any of this later'));
  assert.match(foot.querySelector('[data-jira-cancel]').className, /\bh-11\b/);
  assert.match(foot.querySelector('[data-jira-save]').className, /\bh-11\b/);
  s.unmount();
});
