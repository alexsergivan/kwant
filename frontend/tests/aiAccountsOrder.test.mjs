// AI accounts order (autonomy/apps-accounts/ai-accounts-order.md, from Oleks's "this not very intuitive"):
// rows titled by the AI, grouped Used first / If it runs out / Not used, "Use first" instead of grip,
// number and arrows, Off accounts outside the order with Turn on, and a no-backup line. Display only:
// the order is still each account's priority, saved with PATCH /api/profiles/:id.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'CSS']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.CSS ??= { escape: (s) => String(s).replace(/["\\]/g, '\\$&') };
g.window.CSS ??= g.CSS;

const m = await loadBundled(`
  export { AccountPriority } from './src/components/AccountPriority';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
  export { act } from 'react';
`);
const h = m.createElement;
const NOW = Date.parse('2026-11-12T20:00:00');

const profile = (over) => ({
  id: 'p', engine: 'claude', name: 'This machine', kind: 'system', enabled: true, is_default: false, priority: 0,
  home_dir: null, has_key: false, signed_in: true, account_label: null, limited_until: null, last_error: null, ...over,
});
const engine = (id, over = {}) => ({ id, name: id, kind: 'cli', status: 'ready', detail: null, version: '1', models: [], checked_at: null, enabled: true, ready: true, uses: null, ...over });

// Oleks's shot: 1 Personal/Claude Ready, 2 This machine/Grok Ready, 3 This machine/Claude Off, 4 This machine/Codex Off.
const oleks = () => [
  profile({ id: 'personal', name: 'Personal', kind: 'cli_home', priority: 10 }),
  profile({ id: 'sys-grok', engine: 'grok', priority: 20 }),
  profile({ id: 'sys-claude', priority: 30, enabled: false }),
  profile({ id: 'sys-codex', engine: 'codex', priority: 40 }),
];
const oleksEngines = [engine('claude'), engine('grok'), engine('codex', { enabled: false, status: 'disabled' })];

// ── A tiny server: GET is the list, PATCH changes one account ─────────────────────────────────
const server = (profiles) => {
  const calls = [];
  g.fetch = async (url, init = {}) => {
    const method = init.method ?? 'GET';
    const body = init.body ? JSON.parse(init.body) : undefined;
    calls.push([method, url, body]);
    const id = decodeURIComponent(url.split('/').pop());
    const p = profiles.find((x) => x.id === id);
    if (method === 'PATCH' && p) Object.assign(p, body);
    return { ok: true, status: 200, json: async () => p ?? profiles };
  };
  return calls;
};

const mounted = [];
afterEach(() => {
  for (const r of mounted.splice(0)) r.unmount();
  document.body.replaceChildren();
});
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
/** Mounts the list; onChanged re-renders it from the fake server, like the page's reload does. */
const mount = async (profiles, engines, extra = {}) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  mounted.push(root);
  const engineOn = [];
  const props = () => ({
    profiles: profiles.map((p) => ({ ...p })),
    engines,
    spaceName: 'Binan',
    now: NOW,
    onChanged: async () => root.render(h(m.AccountPriority, props())),
    onEngineOn: async (id) => {
      engineOn.push(id);
      const e = engines.find((x) => x.id === id);
      if (e) Object.assign(e, { enabled: true, status: 'ready' });
    },
    onAddAccount: () => {},
    ...extra,
  });
  root.render(h(m.AccountPriority, props()));
  await settle();
  return { host, engineOn };
};
const text = (el) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const titles = (el) => [...(el?.querySelectorAll('[data-account-title]') ?? [])].map(text);
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};

test('no row is titled "This machine": titles are the AI, and the built-in account says where it is', () => {
  const html = m.renderToStaticMarkup(h(m.AccountPriority, { profiles: oleks(), engines: oleksEngines, onChanged: async () => {}, now: NOW, spaceName: 'Binan' }));
  assert.doesNotMatch(html, /This machine/);
  assert.match(html, /Your bots in Binan use the first account that’s ready\. If it runs out, they switch to the next one by themselves\./);
  assert.match(html, /Signed in on this computer/);
  assert.doesNotMatch(html, /Order to use|draggable|aria-label="Move /, 'no grip, numbers or arrows');
});

test('Oleks’s four accounts: Claude (Personal) first, Grok if it runs out, Claude and Codex on this computer not used, each with Turn on', async () => {
  server(oleks());
  const { host } = await mount(oleks(), oleksEngines.map((e) => ({ ...e })));
  const first = host.querySelector('[data-group="first"]');
  assert.deepEqual(titles(first), ['Claude']);
  assert.match(text(first), /Personal/);
  assert.deepEqual(titles(host.querySelector('[data-group="backup"]')), ['Grok']);
  const notUsed = host.querySelector('[data-group="not-used"]');
  assert.deepEqual(titles(notUsed), ['Claude', 'Codex']);
  assert.deepEqual([...notUsed.querySelectorAll('[data-not-used]')].map((li) => li.getAttribute('data-not-used')), ['off', 'off']);
  assert.equal(notUsed.querySelectorAll('[data-turn-on]').length, 2);
  assert.match(text(notUsed), /On this computer · Turned off/);
  assert.equal(host.querySelector('[data-no-backup]'), null, 'Grok is the backup');
  assert.ok(host.querySelector('[data-group="backup"] [data-use-first]'), 'the backup row has Use first');
  assert.equal(host.querySelector('[data-group="first"] [data-use-first]'), null);
});

test('Turn on saves enabled with a place at the end of "If it runs out", and the row moves there', async () => {
  const profs = oleks();
  const calls = server(profs);
  const engines = oleksEngines.map((e) => ({ ...e }));
  const { host, engineOn } = await mount(profs, engines);
  // Claude on this computer: the account itself is off.
  await click(host.querySelector('[data-account="sys-claude"] [data-turn-on]'));
  const patch = calls.find(([meth, url]) => meth === 'PATCH' && url.endsWith('/sys-claude'));
  assert.equal(patch[2].enabled, true);
  // Its old number (30) already sorts after Grok (20), so it keeps it; any new one must too.
  assert.ok((patch[2].priority ?? profs.find((p) => p.id === 'sys-claude').priority) > 20, `after Grok (${patch[2].priority})`);
  assert.deepEqual(titles(host.querySelector('[data-group="backup"]')), ['Grok', 'Claude']);
  assert.equal(host.querySelector('[data-group="backup"] li:last-child').getAttribute('data-account'), 'sys-claude');
  // Codex: the whole engine is off, so its own switch is turned on too.
  await click(host.querySelector('[data-account="sys-codex"] [data-turn-on]'));
  assert.deepEqual(engineOn, ['codex']);
  assert.deepEqual([...host.querySelectorAll('[data-group="backup"] li')].map((li) => li.getAttribute('data-account')), ['sys-grok', 'sys-claude', 'sys-codex']);
  assert.equal(host.querySelector('[data-group="not-used"]'), null);
});

test('Use first saves the new priorities and puts that account in "Used first"', async () => {
  const profs = oleks();
  const calls = server(profs);
  const { host } = await mount(profs, oleksEngines.map((e) => ({ ...e })));
  await click(host.querySelector('[data-account="sys-grok"] [data-use-first]'));
  const patches = Object.fromEntries(calls.filter(([meth]) => meth === 'PATCH').map(([, url, b]) => [url.split('/').pop(), b.priority]));
  assert.ok(patches['sys-grok'] < (patches.personal ?? 10), `Grok ranks above Personal: ${JSON.stringify(patches)}`);
  assert.deepEqual(titles(host.querySelector('[data-group="first"]')), ['Grok']);
  assert.deepEqual(titles(host.querySelector('[data-group="backup"]')), ['Claude']);
  assert.equal(document.activeElement?.getAttribute('data-focus-key'), 'sys-grok:more', 'focus stays with the row that moved');
});

test('one ready account: "No backup yet" with Turn on for an off one, or Add an account when none is off', async () => {
  const one = [profile({ id: 'personal', name: 'Personal', kind: 'cli_home', priority: 10 }), profile({ id: 'sys-codex', engine: 'codex', priority: 20, enabled: false })];
  server(one);
  const { host } = await mount(one, [engine('claude'), engine('codex')]);
  const line = host.querySelector('[data-no-backup]');
  assert.match(text(line), /^No backup yet\. If Claude runs out, your bots wait until it resets\./);
  assert.equal(text(line.querySelector('[data-no-backup-action]')), 'Turn on Codex');
  assert.equal(host.querySelectorAll('[data-group="backup"] li').length, 0, 'no backup rows');
  assert.ok(host.querySelector('[data-group="backup"] [data-no-backup]'), 'F14: the line sits under If it runs out');
  const alone = m.renderToStaticMarkup(h(m.AccountPriority, { profiles: [one[0]], engines: [engine('claude')], onChanged: async () => {}, onAddAccount: () => {}, now: NOW }));
  assert.match(alone, /No backup yet\./);
  assert.match(alone, /data-no-backup-action="true">Add an account</);
});

test('an account that is out keeps its place and names the account bots use until then', () => {
  const profs = oleks();
  profs[0].limited_until = '2026-11-13T03:00:00';
  const html = m.renderToStaticMarkup(h(m.AccountPriority, { profiles: profs, engines: oleksEngines, onChanged: async () => {}, now: NOW }));
  const first = html.slice(html.indexOf('data-group="first"'), html.indexOf('data-group="backup"'));
  assert.match(first, /Claude/);
  assert.match(first, /Out until/);
  assert.match(first, /data-step-in="true">Bots use Grok until then\./);
});

test('the More menu holds Move up, Move down and Turn off; arrows move in it and Escape returns focus', async () => {
  server(oleks());
  const { host } = await mount(oleks(), oleksEngines.map((e) => ({ ...e })));
  const btn = host.querySelector('[data-account="sys-grok"] [data-row-menu]');
  assert.equal(btn.getAttribute('aria-haspopup'), 'menu');
  assert.equal(btn.getAttribute('aria-label'), 'More for Grok (on this computer)');
  await click(btn);
  const items = [...host.querySelectorAll('[data-row-menu-list] [role="menuitem"]')];
  assert.deepEqual(items.map(text), ['Move up', 'Move down', 'Turn off']);
  assert.ok(items[1].disabled, 'the last used row can’t go down');
  assert.equal(document.activeElement, items[0]);
  items[0].dispatchEvent(new window.KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
  await settle();
  assert.equal(document.activeElement, items[2], 'skips the disabled item');
  document.activeElement.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  await settle();
  assert.equal(host.querySelector('[data-row-menu-list]'), null);
  assert.equal(document.activeElement, btn);
});

// ── Round 3 (design-check-r2 F12–F15) ────────────────────────────────────────────────────────────
const render = (profs, extra = {}) => m.renderToStaticMarkup(h(m.AccountPriority, { profiles: profs, engines: oleksEngines, onChanged: async () => {}, now: NOW, ...extra }));
const group = (html, name) => html.split(`data-group="${name}"`)[1]?.split('data-group=')[0] ?? '';

test('F12: a bot pinned to a backup account "uses it first"; the first row keeps "Used by"', () => {
  const users = { personal: ['Ann', 'Ben'], 'sys-grok': ['Cody'] };
  let html = render(oleks(), { usedBy: (id) => users[id] ?? [] });
  assert.match(group(html, 'first'), /Used by Ann and Ben/);
  assert.match(group(html, 'backup'), /Cody uses it first/);
  assert.doesNotMatch(group(html, 'backup'), /Used by/);
  users['sys-grok'] = ['Cody', 'Dee'];
  assert.match(group(render(oleks(), { usedBy: (id) => users[id] ?? [] }), 'backup'), /Cody and Dee use it first/);
  users['sys-grok'] = ['Cody', 'Dee', 'Eve', 'Fay'];
  assert.match(group(render(oleks(), { usedBy: (id) => users[id] ?? [] }), 'backup'), /Cody, Dee and 2 more use it first/);
  // Not used, signed out: "Used by" explains why Sign in is amber.
  const profs = oleks();
  profs[3] = profile({ id: 'sys-codex', engine: 'codex', priority: 40, signed_in: false });
  html = m.renderToStaticMarkup(h(m.AccountPriority, { profiles: profs, engines: [engine('claude'), engine('grok'), engine('codex', { status: 'signed_out', ready: false })], onChanged: async () => {}, now: NOW, usedBy: (id) => (id === 'sys-codex' ? ['Cody'] : []) }));
  assert.match(group(html, 'not-used'), /Used by Cody/);
});

test('F13: Use first, Turn on, ⋯, links and menu items are 44 px on phones; menu items show keyboard focus', async () => {
  const html = render(oleks(), { onAddAccount: () => {} });
  const cls = (re) => [...html.matchAll(re)].map((x) => x[1]);
  for (const c of cls(/<button[^>]*class="([^"]*)"[^>]*data-use-first/g)) assert.match(c, /min-h-\[44px\] sm:min-h-\[28px\]/);
  for (const c of cls(/<button[^>]*class="([^"]*)"[^>]*data-turn-on/g)) assert.match(c, /min-h-\[44px\] sm:min-h-\[28px\]/);
  const more = cls(/<button[^>]*class="([^"]*)"[^>]*data-row-menu/g);
  assert.ok(more.length > 0);
  for (const c of more) assert.match(c, /w-11 h-11 sm:w-7 sm:h-7/);
  const alone = m.renderToStaticMarkup(h(m.AccountPriority, { profiles: [oleks()[0]], engines: [engine('claude')], onChanged: async () => {}, onAddAccount: () => {}, now: NOW }));
  assert.match(alone, /class="inline-flex items-center min-h-\[44px\] sm:min-h-0[^"]*"[^>]*data-no-backup-action="true">Add an account/);
  server(oleks());
  const { host } = await mount(oleks(), oleksEngines.map((e) => ({ ...e })));
  await click(host.querySelector('[data-account="sys-grok"] [data-row-menu]'));
  for (const it of host.querySelectorAll('[role="menuitem"]')) {
    assert.match(it.className, /min-h-\[44px\] sm:min-h-0/);
    assert.match(it.className, /focus-visible:outline-2/);
    assert.match(it.className, /focus-visible:outline-sky-400/);
  }
});

test('F14: headings are text-xs semibold (Not used quieter), and one ready account still shows "If it runs out"', () => {
  const html = render(oleks());
  assert.match(html, /<h4 id="account-group-first" class="text-xs font-semibold text-zinc-200">Used first/);
  assert.match(html, /<h4 id="account-group-backup" class="text-xs font-semibold text-zinc-200">If it runs out/);
  assert.match(html, /<h4 id="account-group-off" class="text-xs font-semibold text-zinc-400">Not used/);
  const one = [profile({ id: 'personal', name: 'Personal', kind: 'cli_home', priority: 10 }), profile({ id: 'sys-grok', engine: 'grok', priority: 20, enabled: false })];
  const solo = m.renderToStaticMarkup(h(m.AccountPriority, { profiles: one, engines: [engine('claude'), engine('grok')], onChanged: async () => {}, now: NOW }));
  const backup = group(solo, 'backup');
  assert.match(backup, /If it runs out<\/h4>/);
  assert.match(backup, /data-no-backup="true"[^>]*>|class="mt-1\.5[^"]*"[^>]*data-no-backup/);
  assert.match(backup, /No backup yet\. If Claude runs out, your bots wait until it resets\./);
  const at = (t) => solo.indexOf(`>${t}</h4>`);
  assert.ok(at('Used first') > 0 && at('Used first') < at('If it runs out') && at('If it runs out') < at('Not used'), 'Used first → If it runs out → Not used');
});

test('F15: "Bots use Grok until then." is zinc-300; "Bots wait until then." stays amber', () => {
  const profs = oleks();
  profs[0].limited_until = '2026-11-13T03:00:00';
  assert.match(render(profs), /class="mt-0\.5 text-\[11px\] text-zinc-300" data-step-in="true">Bots use Grok until then\./);
  const alone = [{ ...profs[0] }];
  assert.match(m.renderToStaticMarkup(h(m.AccountPriority, { profiles: alone, engines: [engine('claude')], onChanged: async () => {}, now: NOW })), /class="mt-0\.5 text-\[11px\] text-amber-200" data-step-in="true">Bots wait until then\./);
});

// ── Round 5 (Rigel's qa.md item 1): Turn off on an engine's main (default) account ─────────────
const statusText = (host) => text(host.querySelector('[role="status"]'));
const turnOffRow = async (host, id) => {
  await click(host.querySelector(`[data-account="${id}"] [data-row-menu]`));
  const item = [...host.querySelectorAll('[data-row-menu-list] [role="menuitem"]')].find((x) => text(x) === 'Turn off');
  await click(item);
};

test('Turn off, not the main account: only enabled:false is saved', async () => {
  const profs = oleks();
  const calls = server(profs);
  const { host } = await mount(profs, oleksEngines.map((e) => ({ ...e })));
  await turnOffRow(host, 'sys-grok');
  assert.deepEqual(calls.filter((c) => c[0] === 'PATCH').map((c) => [c[1], c[2]]), [['/api/profiles/sys-grok', { enabled: false }]]);
  assert.match(statusText(host), /Grok \(on this computer\) is off/);
  assert.equal(host.querySelector('[data-order-error]'), null);
});

test('Turn off, the main account with another on: the next one becomes main first, then this one goes off', async () => {
  const profs = oleks();
  profs[0].is_default = true; // Personal is Claude's main account
  profs[2].enabled = true; // Claude on this computer is on, further down the order
  const calls = server(profs);
  const { host } = await mount(profs, oleksEngines.map((e) => ({ ...e })));
  await turnOffRow(host, 'personal');
  assert.deepEqual(
    calls.filter((c) => c[0] === 'PATCH').map((c) => [c[1], c[2]]),
    [
      ['/api/profiles/sys-claude', { is_default: true }],
      ['/api/profiles/personal', { enabled: false }],
    ],
  );
  assert.equal(statusText(host), 'Claude (Personal) is off. The sign-in on this computer is now Claude’s main account.');
  assert.equal(host.querySelector('[data-order-error]'), null);
  assert.ok(host.querySelector('[data-group="not-used"] [data-account="personal"]'), 'Personal moved to Not used');
});

test('Turn off, the only account its engine has on: the engine’s own switch, no PATCH', async () => {
  const profs = oleks();
  profs[1].is_default = true; // Grok on this computer: Grok's main and only account
  const calls = server(profs);
  const engines = oleksEngines.map((e) => ({ ...e }));
  const engineOff = [];
  const { host } = await mount(profs, engines, {
    onEngineOff: async (id) => {
      engineOff.push(id);
      Object.assign(engines.find((x) => x.id === id), { enabled: false, status: 'disabled' });
    },
  });
  await turnOffRow(host, 'sys-grok');
  assert.deepEqual(engineOff, ['grok']);
  assert.equal(calls.filter((c) => c[0] === 'PATCH').length, 0);
  assert.ok(host.querySelector('[data-group="not-used"] [data-account="sys-grok"]'), 'the row lands in Not used');
  assert.equal(host.querySelector('[data-order-error]'), null);
});

test('Turn off that fails says so in plain words, never the server’s text, and focus goes back to More', async () => {
  const profs = oleks();
  profs[1].is_default = true;
  profs.push(profile({ id: 'grok-work', engine: 'grok', name: 'Work', kind: 'cli_home', priority: 25 }));
  const calls = server(profs);
  const ok = g.fetch;
  g.fetch = async (url, init = {}) => {
    if (init.method === 'PATCH' && url.endsWith('/sys-grok')) {
      calls.push(['PATCH', url, JSON.parse(init.body)]);
      return { ok: false, status: 400, json: async () => ({ error: "the default profile can't be disabled" }) };
    }
    return ok(url, init);
  };
  const { host } = await mount(profs, oleksEngines.map((e) => ({ ...e })));
  await turnOffRow(host, 'sys-grok');
  const err = text(host.querySelector('[data-order-error]'));
  assert.equal(err, 'Couldn’t turn off Grok (on this computer). It is back as it was.');
  assert.equal(host.innerHTML.includes('default profile'), false, 'no server text anywhere');
  // The main account moved to Work first, so it is put back on this one.
  assert.deepEqual(
    calls.filter((c) => c[0] === 'PATCH').map((c) => [c[1], c[2]]),
    [
      ['/api/profiles/grok-work', { is_default: true }],
      ['/api/profiles/sys-grok', { enabled: false }],
      ['/api/profiles/sys-grok', { is_default: true }],
    ],
  );
  const more = host.querySelector('[data-account="sys-grok"] [data-row-menu]');
  assert.equal(document.activeElement === more, true, 'focus is on the row’s More button');
});

// ── Round 6 (qa.md round 5, item 1 last row): the engine's own switch fails ─────────────────────
test('Turn off whose engine switch fails: "Couldn’t turn off …", no "is off", focus on the row’s More', async () => {
  const profs = oleks();
  profs[1].is_default = true; // Grok on this computer: Grok's main and only account
  const calls = server(profs);
  const engines = oleksEngines.map((e) => ({ ...e }));
  const engineOff = [];
  const { host } = await mount(profs, engines, {
    // Like useEngines.setEnabled on a failed PUT: it puts the switch back and returns false.
    onEngineOff: async (id) => {
      engineOff.push(id);
      return false;
    },
  });
  await turnOffRow(host, 'sys-grok');
  assert.deepEqual(engineOff, ['grok']);
  assert.equal(text(host.querySelector('[data-order-error]')), 'Couldn’t turn off Grok (on this computer). It is back as it was.');
  assert.doesNotMatch(statusText(host), /is off/);
  assert.doesNotMatch(host.textContent, /is off\./, 'no success words anywhere');
  assert.equal(calls.filter((c) => c[0] === 'PATCH').length, 0);
  assert.ok(host.querySelector('[data-group="backup"] [data-account="sys-grok"]'), 'the row stays where it was');
  const more = host.querySelector('[data-account="sys-grok"] [data-row-menu]');
  assert.equal(more.getAttribute('data-focus-key'), 'sys-grok:more');
  assert.equal(document.activeElement === more, true, 'focus is on the row’s More button');
});

test('Turn on whose engine switch fails: "Couldn’t turn on …", no "is on", focus back on the row’s Turn on', async () => {
  const profs = oleks();
  const calls = server(profs);
  const engines = oleksEngines.map((e) => ({ ...e }));
  const engineOn = [];
  const { host } = await mount(profs, engines, {
    onEngineOn: async (id) => {
      engineOn.push(id);
      return false;
    },
  });
  await click(host.querySelector('[data-account="sys-codex"] [data-turn-on]'));
  assert.deepEqual(engineOn, ['codex']);
  assert.equal(text(host.querySelector('[data-order-error]')), 'Couldn’t turn on Codex (on this computer). It is back as it was.');
  assert.doesNotMatch(statusText(host), /is on/);
  assert.doesNotMatch(host.textContent, /is on\. Your bots/, 'no success words anywhere');
  assert.equal(calls.filter((c) => c[0] === 'PATCH').length, 0, 'the account is not turned on without its engine');
  const on = host.querySelector('[data-account="sys-codex"] [data-turn-on]');
  assert.ok(host.querySelector('[data-group="not-used"] [data-account="sys-codex"]'), 'the row stays in Not used');
  assert.equal(document.activeElement === on, true, 'focus is back on the row’s Turn on (Not used rows have no More)');
});

// ── Round 5 (qa.md item 3): bots on auto are named only on "Used first" ─────────────────────────
test('auto bots only on Used first: backup and Not used rows name pinned bots (usedBy pinnedOnly)', () => {
  // Rigel's space: Ann and Ben on auto, Cody pinned to Codex, Gus pinned to Grok; Grok first.
  const profs = [
    profile({ id: 'grok', engine: 'grok', is_default: true, priority: 10 }),
    profile({ id: 'claude', engine: 'claude', is_default: true, priority: 20 }),
    profile({ id: 'codex', engine: 'codex', is_default: true, priority: 30 }),
  ];
  const server = { grok: ['Ann', 'Ben', 'Gus'], claude: ['Ann', 'Ben'], codex: ['Ann', 'Ben', 'Cody'] };
  const auto = new Set(['Ann', 'Ben']);
  const usedBy = (id, pinnedOnly) => (server[id] ?? []).filter((n) => !pinnedOnly || !auto.has(n));
  const engines = [engine('claude'), engine('grok'), engine('codex')];
  const html = m.renderToStaticMarkup(h(m.AccountPriority, { profiles: profs, engines, onChanged: async () => {}, now: NOW, usedBy }));
  assert.match(group(html, 'first'), /Used by Ann, Ben and Gus/);
  const backup = group(html, 'backup');
  assert.match(backup, /Cody uses it first/);
  assert.doesNotMatch(backup, /Ann|Ben/);
  assert.doesNotMatch(backup, /Ann and Ben use it first/);
});
