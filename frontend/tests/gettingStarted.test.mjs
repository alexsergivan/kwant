// 0.7.6 O2: the Get started card (design next-076/onboarding). Step logic (already signed in → steps
// 1–2 done, 3 open), the install tracker (409 wait, 422 admin step, failures), device-code sign-in
// with polling and cancel, the Terminal fallback before O3, failure rows, the banner and its strings.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = g.window.matchMedia || ((q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {} }));
g.fetch = g.window.fetch = async () => new Response('{}', { status: 404, headers: { 'Content-Type': 'application/json' } });
const copied = [];
Object.defineProperty(g.navigator, 'clipboard', { value: { writeText: async (t) => void copied.push(t) }, configurable: true });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export * from './src/gettingStarted';
  export * from './src/gettingStartedInstall';
  export { GetStarted, GetStartedBanner, bannerPhase, hostOf } from './src/components/GetStarted';
`);
const { React, createRoot, flushSync } = m;
const h = React.createElement;
const settle = async (n = 6) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const engine = (id, over = {}) => ({
  id, name: id, kind: id.endsWith('_api') ? 'api' : 'cli', status: 'not_installed', detail: null, version: null, models: [], checked_at: null, enabled: true, ready: false, uses: null, ...over,
});
const ready = (id) => engine(id, { status: 'ready', ready: true });
const signedOut = (id) => engine(id, { status: 'signed_out' });

function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  flushSync(() => root.render(el));
  return { host, root, done: () => (root.unmount(), host.remove()) };
}

// ------------------------------------------------------------------ step logic

test('steps: already signed in → steps 1 and 2 done, step 3 open (D2)', () => {
  const v = m.stepsView({ engines: [ready('claude'), engine('codex')], chosen: null });
  assert.deepEqual(v, { open: 3, done1: true, done2: true, choice: 'claude', foundReady: true });
  assert.equal(m.step1Value(v, [ready('claude')], 'on this Mac'), 'Claude, already signed in on this Mac');
  assert.deepEqual(m.heroText(v, { mac: true }), { title: 'Claude is ready on this Mac', sub: 'You were already signed in. Pick a team and say hello.' });
  assert.equal(m.heroText(v, { host: 'vps.example.net' }).title, 'Claude is ready on vps.example.net');
});

test('steps: nothing chosen → 1; chosen, not signed in → 2; F8 → 3 early; Change → 1; never backwards', () => {
  assert.equal(m.stepsView({ engines: [engine('claude')], chosen: null }).open, 1);
  const two = m.stepsView({ engines: [engine('claude')], chosen: 'claude' });
  assert.deepEqual([two.open, two.done1, two.done2], [2, true, false]);
  assert.equal(m.step1Value(two, [engine('claude')], 'on this Mac'), 'Claude');
  assert.equal(m.step1Value(m.stepsView({ engines: [signedOut('codex')], chosen: 'codex' }), [signedOut('codex')], ''), 'ChatGPT, installed');
  assert.equal(m.stepsView({ engines: [engine('claude')], chosen: 'claude', teamEarly: true }).open, 3);
  assert.equal(m.stepsView({ engines: [ready('claude')], chosen: 'claude', changing: true }).open, 1);
  // Codex chosen while Claude works: the card stays on the chosen AI.
  assert.equal(m.stepsView({ engines: [ready('claude'), signedOut('codex')], chosen: 'codex' }).open, 2);
  // An API key that works finishes step 2.
  assert.equal(m.stepsView({ engines: [ready('anthropic_api')], chosen: 'key' }).open, 3);
  assert.equal(m.readyChoice([ready('xai_api'), ready('codex')]), 'codex', 'engine order: CLIs first');
});

test('the card shows for a fresh space, never for an older one with history, never once dismissed', () => {
  const base = { dismissed: false, canRun: true, generalMessages: 0, teams: 0, started: false };
  assert.equal(m.showCard(base), true);
  assert.equal(m.showCard({ ...base, generalMessages: 4 }), false);
  assert.equal(m.showCard({ ...base, teams: 1 }), true, 'the starter team alone is still a fresh install');
  assert.equal(m.showCard({ ...base, teams: 2 }), false);
  assert.equal(m.showCard({ ...base, teams: 2, started: true }), true, 'C4: stays until the first message');
  assert.equal(m.showCard({ ...base, canRun: false, generalMessages: 9 }), true);
  assert.equal(m.showCard({ ...base, canRun: null }), false, 'not while the engines load');
  assert.equal(m.showCard({ ...base, canRun: false, dismissed: true }), false);
});

test('dismiss is kept per space and clears the stored choice', () => {
  localStorage.clear();
  m.storeChoice('s1', 'grok');
  m.markStarted('s1');
  assert.equal(m.storedChoice('s1'), 'grok');
  assert.equal(m.wasStarted('s1'), true);
  m.markStarted('s1', 'kept');
  assert.equal(m.isKept('s1'), true);
  m.dismiss('s1');
  assert.equal(m.isDismissed('s1'), true);
  assert.equal(m.storedChoice('s1'), null);
  m.markStarted('s1');
  assert.equal(m.isDismissed('s1'), true, 'never comes back');
  assert.equal(m.isDismissed('s2'), false);
});

// ------------------------------------------------------------------ failures and strings

test('install failures map to four kinds; the raw log never reaches the text', () => {
  assert.equal(m.installFailKind('curl: (6) Could not resolve host', true), 'network');
  assert.equal(m.installFailKind('write failed: No space left on device', true), 'disk');
  assert.equal(m.installFailKind('npm ERR! code EACCES', true), 'permission');
  assert.equal(m.installFailKind('exit status 3', true), 'other');
  assert.equal(m.installFailKind('anything', false), 'offline');
  const off = m.installFailView('offline', 'Claude', 'on this Mac');
  assert.equal(off.tone, 'warn');
  assert.equal(off.title, 'You’re offline, so Claude can’t download yet.');
  assert.equal(m.installFailView('disk', 'Claude', 'on this computer', 'vps.example.net').title, 'Couldn’t get Claude ready on vps.example.net.');
  assert.equal(m.installFailView('other', 'Codex', '').title, 'Something went wrong while getting Codex ready.');
});

test('sign-in endings: cancelled is neutral, expired asks for a new code, a rejected account is rose (F3, F4, F6)', () => {
  assert.deepEqual(
    [m.signInEnd({ state: 'cancelled', detail: null }, 'codex').tone, m.signInEnd({ state: 'cancelled', detail: null }, 'codex').title],
    ['neutral', 'Sign-in stopped. Nothing was saved.'],
  );
  assert.equal(m.signInEnd({ state: 'expired', detail: null }, 'grok').action, 'Get a new code');
  assert.equal(m.signInEnd({ state: 'expired', detail: null }, 'claude').title, 'That sign-in timed out.');
  const f6 = m.signInEnd({ state: 'failed', detail: "This account can't use this account type" }, 'codex');
  assert.equal(f6.tone, 'bad');
  assert.equal(f6.title, 'Signed in, but Codex can’t use this account.');
  assert.equal(m.signInEnd({ state: 'waiting_for_approval', detail: null }, 'codex'), null);
});

test('device code: read out letter by letter, only https sign-in pages open, the label drops https', () => {
  assert.equal(m.codeAriaLabel('K7QF-2MXD'), 'Code K 7 Q F, 2 M X D');
  assert.equal(m.deviceLabel('https://chatgpt.com/device?x=1'), 'chatgpt.com/device');
  assert.equal(m.isSignInUrl('codex', 'https://auth.openai.com/codex/device'), true);
  assert.equal(m.isSignInUrl('codex', 'http://chatgpt.com/device'), false);
  assert.equal(m.isSignInUrl('codex', 'https://evil.example/device'), false);
  assert.equal(m.isSignInUrl('claude', 'javascript:alert(1)'), false);
  assert.equal(m.minutesLeft(new Date(Date.now() + 14.2 * 60000).toISOString(), Date.now()), 15);
  assert.equal(m.minutesLeft(null, Date.now()), null);
});

test('banner strings outside #general (§7.1)', () => {
  assert.equal(m.bannerView({ kind: 'choose' }, null).title, 'Getting started · Choose your AI');
  const inst = m.bannerView({ kind: 'installing', seconds: 9 }, 'claude');
  assert.deepEqual([inst.title, inst.short, inst.spinner], ['Getting started · Getting Claude ready… 9 s', 'Getting Claude ready… 9 s', true]);
  const sign = m.bannerView({ kind: 'signin' }, 'codex');
  assert.deepEqual([sign.title, sign.button], ['Getting started · Sign in to Codex', 'Continue']);
  assert.equal(m.bannerView({ kind: 'failed' }, 'claude').sub, null);
  assert.equal(m.bannerPhase([ready('claude')], null, null), null, 'never once step 2 is done');
  assert.deepEqual(m.bannerPhase([engine('claude')], null, null), { kind: 'choose' });
  assert.deepEqual(m.bannerPhase([signedOut('claude')], 'claude', null), { kind: 'signin' });
});

// ------------------------------------------------------------------ the install tracker

const fast = { sleep: async () => {}, online: () => true };
function setupFake({ busy = 0, start = 202, polls = 2, fail = null, manualReadyAfter = 1 } = {}) {
  const calls = [];
  let reads = 0;
  let statusReads = 0;
  return {
    calls,
    async status() {
      calls.push('status');
      statusReads++;
      return [{ id: 'claude', label: 'Claude Code', state: statusReads >= manualReadyAfter ? 'ready' : 'missing', can_install: true }];
    },
    async install(item) {
      calls.push(`install:${item}`);
      if (busy-- > 0) return { status: 409, body: { error: 'another install is running', job_id: 'setup-0' } };
      if (start === 422) return { status: 422, body: { error: 'this part needs a step only you can do', item, manual: [{ label: 'Install', command: 'sudo apt install -y bubblewrap' }] } };
      if (start === 200) return { status: 200, body: { already_installed: true } };
      return { status: 202, body: { job_id: 'setup-1', item } };
    },
    async job(id, after) {
      calls.push(`job:${after}`);
      reads++;
      if (reads < polls) return { id, state: 'running', error: null, lines: [`line ${reads}`], next: after + 1 };
      if (fail) return { id, state: 'failed', error: fail, lines: ['last'], next: after + 1 };
      return { id, state: 'done', error: null, lines: [], next: after + 1 };
    },
  };
}

test('install: waits out 409, follows the job, then done; the card and banner read the same tracker', async () => {
  const api = setupFake({ busy: 1 });
  let done = 0;
  const t = await m.startInstall('i1', 'claude', { ...fast, api, onDone: () => done++ });
  assert.equal(t.state, 'done');
  assert.deepEqual(api.calls, ['install:claude', 'install:claude', 'job:0', 'job:1']);
  assert.equal(done, 1);
  assert.equal(m.getTrack('i1').state, 'done');
  m.forgetInstall('i1');
  assert.equal(m.getTrack('i1'), null);
});

test('install: a failure keeps its kind and the log only for Copy details', async () => {
  const t = await m.startInstall('i2', 'claude', { ...fast, api: setupFake({ fail: 'EACCES: permission denied' }) });
  assert.deepEqual([t.state, t.kind], ['failed', 'permission']);
  assert.match(m.installDetails(t), /EACCES/);
  const off = await m.startInstall('i3', 'claude', { ...fast, api: setupFake(), online: () => false });
  assert.deepEqual([off.state, off.kind], ['failed', 'offline']);
});

test('install: 422 shows the admin command, then moves on by itself once the part is there (F2c)', async () => {
  const api = setupFake({ start: 422, manualReadyAfter: 2 });
  const seen = [];
  const un = m.subscribeTracks(() => seen.push(m.getTrack('i4')?.state));
  const t = await m.startInstall('i4', 'claude', { ...fast, api });
  un();
  assert.equal(t.state, 'done');
  assert.ok(seen.includes('needs_admin'));
  assert.deepEqual(api.calls, ['install:claude', 'status', 'status']);
});

// ------------------------------------------------------------------ device-code sign-in

function loginFake({ startError = null, polls = 2, end = 'done' } = {}) {
  const calls = [];
  let reads = 0;
  const base = { session_id: 'L1', engine: 'codex', kind: 'device_code', detail: null, url: 'https://chatgpt.com/device', user_code: 'K7QF-2MXD', expires_at: new Date(Date.now() + 15 * 60000).toISOString() };
  return {
    calls,
    async start(e) {
      calls.push(`start:${e}`);
      if (startError) throw startError;
      return { ...base, engine: e, state: 'waiting_for_approval' };
    },
    async get(e, sid) {
      calls.push(`get:${sid}`);
      reads++;
      return { ...base, engine: e, state: reads >= polls ? end : 'waiting_for_approval' };
    },
    async cancel(e, sid) {
      calls.push(`cancel:${sid}`);
    },
    async submitCode() {
      throw new Error('not for device codes');
    },
  };
}

test('startOrResume picks up the sign-in already running (409 with its id)', async () => {
  const api = loginFake();
  api.start = async () => {
    throw new m.LoginError('a sign-in is already running', 409, 'L9');
  };
  const s = await m.startOrResume(api, 'codex');
  assert.equal(api.calls[0], 'get:L9');
  assert.equal(s.state, 'waiting_for_approval');
});

test('card: ChatGPT signed out → device code, copy-then-open, polls until done and reads engines again', async () => {
  localStorage.clear();
  m.storeChoice('d1', 'codex');
  const api = loginFake({ polls: 1 });
  let reloads = 0;
  const opened = [];
  window.open = (u) => (opened.push(u), null);
  const r = mount(h(m.GetStarted, { space: 'd1', bots: [], engines: [signedOut('codex')], reloadEngines: () => reloads++, onDismissed() {}, loginApiOverride: api, hostOverride: null }));
  await settle();
  const card = r.host.querySelector('[data-get-started]');
  assert.equal(card.getAttribute('data-open-step'), '2');
  const code = r.host.querySelector('[data-user-code]');
  assert.equal(code.textContent, 'K7QF-2MXD');
  assert.equal(code.getAttribute('aria-label'), 'Code K 7 Q F, 2 M X D');
  assert.match(r.host.textContent, /Open chatgpt\.com\/device and enter this code/);
  assert.match(r.host.textContent, /Waiting for you to finish on chatgpt\.com…/);
  assert.match(r.host.textContent, /The code works for 15 minutes\./);
  assert.match(r.host.textContent, /ChatGPT, installed/);
  copied.length = 0;
  r.host.querySelector('[data-open-device]').click();
  await settle();
  assert.deepEqual(copied, ['K7QF-2MXD'], 'C6: the code is copied first');
  assert.deepEqual(opened, ['https://chatgpt.com/device']);
  await sleep(1700);
  await settle();
  assert.ok(api.calls.includes('get:L1'));
  assert.ok(reloads >= 1, 'done → engines read again (step 3 opens from them)');
  r.done();
});

test('card: Cancel stops the sign-in and shows the neutral F3 row with Start again', async () => {
  localStorage.clear();
  m.storeChoice('d2', 'grok');
  const api = loginFake({ polls: 99 });
  const r = mount(h(m.GetStarted, { space: 'd2', bots: [], engines: [signedOut('grok')], reloadEngines() {}, onDismissed() {}, loginApiOverride: api, hostOverride: null }));
  await settle();
  const cancel = [...r.host.querySelectorAll('button')].find((b) => b.textContent === 'Cancel');
  cancel.click();
  await settle();
  assert.ok(api.calls.includes('cancel:L1'));
  assert.match(r.host.textContent, /Sign-in stopped\. Nothing was saved\./);
  assert.equal(r.host.querySelector('[data-fail-row]').getAttribute('data-fail-row'), 'neutral');
  const again = [...r.host.querySelectorAll('button')].find((b) => b.textContent === 'Start again');
  assert.ok(again);
  assert.ok([...r.host.querySelectorAll('button')].some((b) => b.textContent === 'Choose another AI'));
  again.click();
  await settle();
  assert.equal(api.calls.filter((c) => c === 'start:grok').length, 2);
  r.done();
});

test('card: no device route on this server (before O3) → "codex login" in Terminal with Copy', async () => {
  localStorage.clear();
  m.storeChoice('d3', 'codex');
  const api = loginFake({ startError: new m.LoginError('HTTP 404', 404) });
  const r = mount(h(m.GetStarted, { space: 'd3', bots: [], engines: [signedOut('codex')], reloadEngines() {}, onDismissed() {}, loginApiOverride: api, hostOverride: null }));
  await settle();
  assert.ok(r.host.querySelector('[data-terminal-signin]'));
  assert.match(r.host.textContent, /codex login/);
  assert.doesNotMatch(r.host.textContent, /Re-check/);
  r.done();
});

// ------------------------------------------------------------------ the card's other states

test('card: already signed in → hero D2, steps 1–2 done with Change, step 3 open with Keep the starter bots', async () => {
  localStorage.clear();
  const r = mount(h(m.GetStarted, { space: 'r1', bots: [{ id: 'a', name: 'Architect' }], engines: [ready('claude')], reloadEngines() {}, onDismissed() {}, hostOverride: null }));
  await settle();
  const card = r.host.querySelector('[data-get-started]');
  assert.equal(card.getAttribute('data-open-step'), '3');
  assert.match(r.host.querySelector('h2').textContent, /^Claude is ready on this (Mac|computer)$/);
  const heads = [...r.host.querySelectorAll('h3')].map((x) => x.textContent);
  assert.match(heads[0], /^Step 1 of 3, Choose your AI, doneClaude, already signed in on this (Mac|computer)$/);
  assert.equal(heads[1], 'Step 2 of 3, Sign in, doneNothing to do');
  assert.equal(heads[2], 'Step 3 of 3, Pick a team, open');
  assert.ok([...r.host.querySelectorAll('button')].some((b) => b.getAttribute('aria-label') === 'Change your AI'));
  let dismissed = 0;
  r.done();
  const r2 = mount(h(m.GetStarted, { space: 'r1', bots: [], engines: [ready('claude')], reloadEngines() {}, onDismissed: () => dismissed++, hostOverride: null }));
  r2.host.querySelector('[data-keep-starter]').click();
  await settle();
  assert.equal(dismissed, 1);
  assert.equal(m.isDismissed('r1'), true);
  r2.done();
});

test('card: step 1 tiles are a radiogroup; arrows move, a tap starts at once; server shows the host', async () => {
  localStorage.clear();
  const r = mount(h(m.GetStarted, { space: 't1', bots: [], engines: [engine('claude'), signedOut('codex')], reloadEngines() {}, onDismissed() {}, hostOverride: 'vps.example.net' }));
  await settle();
  const group = r.host.querySelector('[role="radiogroup"]');
  assert.equal(group.getAttribute('aria-label'), 'Choose your AI');
  const tiles = [...group.querySelectorAll('[role="radio"]')];
  assert.deepEqual(tiles.map((t) => t.getAttribute('data-ai-tile')), ['claude', 'codex', 'grok', 'key']);
  assert.deepEqual(tiles.map((t) => t.tabIndex), [0, -1, -1, -1]);
  assert.match(tiles[1].textContent, /Installed · sign in next/);
  tiles[0].focus();
  tiles[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
  assert.equal(document.activeElement, tiles[1]);
  assert.match(r.host.textContent, /Setting up on vps\.example\.net/);
  assert.match(r.host.textContent, /Tap one\. It gets ready on vps\.example\.net by itself\./);
  assert.match(r.host.textContent, /It installs on your server, not on this phone\./);
  flushSync(() => tiles[3].click());
  await settle();
  assert.equal(m.storedChoice('t1'), 'key');
  assert.equal(r.host.querySelector('[data-get-started]').getAttribute('data-open-step'), '2');
  assert.match(r.host.textContent, /Add your key/);
  r.done();
});

test('card: a failed install shows F2 in step 2 with Try again, Copy details and Choose another AI; no log', async () => {
  localStorage.clear();
  m.storeChoice('f1', 'claude');
  await m.startInstall('f1', 'claude', { ...fast, api: setupFake({ fail: 'curl: (56) Recv failure: Connection reset by peer' }) });
  const r = mount(h(m.GetStarted, { space: 'f1', bots: [], engines: [engine('claude')], reloadEngines() {}, onDismissed() {}, hostOverride: null }));
  await settle();
  const row = r.host.querySelector('[data-fail-row]');
  assert.equal(row.getAttribute('data-fail-row'), 'bad');
  assert.equal(row.getAttribute('role'), 'alert');
  assert.match(row.textContent, /Couldn’t get Claude ready\./);
  assert.match(row.textContent, /The download stopped partway/);
  const labels = [...row.querySelectorAll('button')].map((b) => b.textContent);
  assert.deepEqual(labels, ['Try again', 'Copy details', 'Choose another AI']);
  assert.doesNotMatch(r.host.textContent, /Recv failure|line 1/);
  m.forgetInstall('f1');
  r.done();
});

test('card: the install line counts seconds but only announces the words', async () => {
  localStorage.clear();
  m.storeChoice('f2', 'claude');
  let release;
  const api = setupFake();
  api.install = () => new Promise((res) => (release = () => res({ status: 202, body: { job_id: 'j' } })));
  void m.startInstall('f2', 'claude', { ...fast, api });
  const r = mount(h(m.GetStarted, { space: 'f2', bots: [], engines: [engine('claude')], reloadEngines() {}, onDismissed() {}, hostOverride: null }));
  await settle();
  const live = r.host.querySelector('[data-install-line] [role="status"]');
  assert.equal(live.textContent, 'Getting Claude ready');
  assert.match(r.host.querySelector('[data-install-line]').textContent, /Getting Claude ready… 0 s/);
  assert.match(r.host.textContent, /Next you sign in with your Claude account\./);
  assert.match(r.host.querySelectorAll('h3')[1].textContent, /Sign in, in progress$/);
  m.forgetInstall('f2');
  release();
  r.done();
});

test('banner: outside #general it says where getting started is, and Open goes to the card', async () => {
  localStorage.clear();
  let opened = 0;
  const r = mount(h(m.GetStartedBanner, { space: 'b1', engines: [engine('claude')], onOpen: () => opened++ }));
  assert.match(r.host.textContent, /Getting started · Choose your AI/);
  assert.match(r.host.textContent, /One tap installs it\./);
  r.host.querySelector('button').click();
  assert.equal(opened, 1);
  r.done();
  const r2 = mount(h(m.GetStartedBanner, { space: 'b1', engines: [ready('claude')], onOpen() {} }));
  assert.equal(r2.host.textContent, '', 'never once an account works');
  r2.done();
});

test('host: only a real address counts as a server', () => {
  assert.equal(m.hostOf({ location: { hostname: 'localhost' } }), null);
  assert.equal(m.hostOf({ location: { hostname: '127.0.0.1' } }), null);
  assert.equal(m.hostOf({ location: { hostname: 'vps.example.net' } }), 'vps.example.net');
});
