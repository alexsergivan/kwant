// 0.6.4 follow-ups on the bot editor: "Good for" / "Not for" on the create form (POST /api/kwants
// takes covers / avoids, router-concierge/contract.md §5), and a Browser sites section in bot
// settings for the sites a bot's own browser may open (browser-borrowed/contract.md). These render
// the whole space app in a DOM, so what's checked is the body that reaches the server.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_default', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
class FakeEvents {
  addEventListener() {}
  removeEventListener() {}
  close() {}
}
g.EventSource = g.window.EventSource = FakeEvents;
g.window.HTMLElement.prototype.scrollIntoView ??= () => {};

const bot = (id, name, extra = {}) => ({ id, name, title: 'Role', persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '', covers: [], avoids: [], ...extra });
const sites = (allowed = [], blocked = [], priv = false) => ({ browser_allowed_sites: allowed, browser_blocked_sites: blocked, browser_block_private_network: priv });
let kwants;
const space = { id: 'spc_default', name: 'Default', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: 'ch1', counts: { bots: 2, ready_accounts: 1, unread: 0 } };

// Every write the app sent, and what the next write to a path answers (default: 200 with the bot).
let writes;
let answer;
const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'Content-Type': 'application/json' } });
g.fetch = g.window.fetch = async (url, init = {}) => {
  const path = String(url).split('?')[0];
  const method = init.method ?? 'GET';
  if (method !== 'GET') {
    const body = init.body ? JSON.parse(init.body) : null;
    writes.push({ method, path, body });
    const a = answer[`${method} ${path}`];
    if (a) return json(a.body, a.status);
    if (method === 'POST' && path === '/api/kwants') return json({ ...bot('k_new', body.name), ...body, id: 'k_new' });
    if (method === 'PUT' && path.startsWith('/api/kwants/') && !path.endsWith('/mcp-servers')) {
      const id = path.split('/')[3];
      return json({ ...kwants.find((k) => k.id === id), ...body });
    }
    return json({});
  }
  const routes = {
    '/api/kwants': kwants,
    '/api/channels': [{ id: 'ch1', name: 'general', kwant_ids: kwants.map((k) => k.id), created_at: '' }],
    '/api/teams': [],
    '/api/engines': [{ id: 'claude', kind: 'cli', enabled: true, status: 'ready', ready: true }],
    '/api/automation-discovery': { setting: 'off', daily_cap: 3, blocked_by: [], last_scan_at: null, suggestions: [], stats: {} },
  };
  return json(path in routes ? routes[path] : []);
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { SpaceApp } from './src/App';
  export * from './src/browserSites';
  export { addTags } from './src/tagList';
`);
const { React } = m;
const noop = () => {};
const settle = async () => {
  for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0));
};
const click = async (el) => {
  assert.ok(el, 'the element to click is there');
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};
const key = async (k, el = document.activeElement ?? document.body) => {
  el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));
  await settle();
};
const type = async (el, text) => {
  el.focus();
  const proto = el.tagName === 'TEXTAREA' ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, text);
  el.dispatchEvent(new Event('input', { bubbles: true }));
  await settle();
};
/** Types a tag and presses Enter, the way a person adds one. */
const addTag = async (input, text) => {
  await type(input, text);
  await key('Enter', input);
};
const paste = async (input, text) => {
  input.focus();
  const e = new Event('paste', { bubbles: true, cancelable: true });
  e.clipboardData = { getData: () => text };
  input.dispatchEvent(e);
  await settle();
};

const mounted = new Set();
afterEach(() => {
  for (const unmount of mounted) unmount();
  document.body.replaceChildren();
});
const mountSpace = async (over = {}) => {
  writes = [];
  answer = {};
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(React.createElement(m.SpaceApp, {
    space, spaces: [space], initialTarget: null, onSwitchSpace: noop, onCreateSpace: noop, onOpenArchived: noop,
    onSpacesChanged: noop, onSpaceArchived: noop, onChatChange: noop, ...over,
  }));
  await settle();
  const unmount = () => {
    if (!mounted.delete(unmount)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(unmount);
  return { unmount };
};
const q = (sel) => document.querySelector(sel);
const button = (label) => [...document.querySelectorAll('button')].find((b) => b.getAttribute('aria-label') === label || b.textContent.trim() === label);
const editor = () => q('[role="dialog"][aria-labelledby="agent-modal-title"]');
const inputFor = (label) => {
  const l = [...editor().querySelectorAll('label')].find((x) => x.textContent.trim().startsWith(label));
  assert.ok(l, `a field labelled ${label}`);
  return document.getElementById(l.htmlFor);
};
const errorOf = (input) => document.getElementById(input.getAttribute('aria-describedby') ?? '')?.textContent ?? null;
const submit = async () => {
  editor().querySelector('form').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  await settle();
};
const openCreate = async () => {
  await mountSpace();
  await click(button('New bot') ?? button('New bot in Default'));
  assert.ok(editor(), 'the create form is open');
  await type(q('#agent-name'), 'Foodie');
};
const openEdit = async (name = 'Ada') => {
  await mountSpace({ initialView: { kind: 'settings', scope: 'space', tab: 'bots' } });
  await click(button(`Edit ${name}`));
  assert.ok(editor(), 'the bot editor is open');
};
const sitesSection = () => editor().querySelector('[data-browser-sites]');
const postBody = () => writes.find((w) => w.method === 'POST' && w.path === '/api/kwants')?.body;
const putBody = (id = 'k1') => writes.find((w) => w.method === 'PUT' && w.path === `/api/kwants/${id}`)?.body;

const reset = () => {
  kwants = [bot('k1', 'Ada', sites(['wikipedia.org'], [], false)), bot('k2', 'Lin')];
};

// ---------------------------------------------------------------------------
// Helpers

test('sitesOf reads the three fields, and is null when the server has none', () => {
  reset();
  assert.equal(m.sitesOf({ name: 'Old' }), null);
  assert.deepEqual(m.sitesOf(kwants[0]), sites(['wikipedia.org']));
  assert.deepEqual(m.sitesOf({ browser_allowed_sites: [], browser_blocked_sites: [] }), sites(), 'a missing toggle reads as off');
});

test('changedSites sends only what changed, so a profile edit leaves the rules alone', () => {
  const before = sites(['a.com'], ['b.com'], false);
  assert.deepEqual(m.changedSites(before, sites(['a.com'], ['b.com'], false)), {});
  assert.deepEqual(m.changedSites(before, sites(['a.com', 'c.com'], ['b.com'], false)), { browser_allowed_sites: ['a.com', 'c.com'] });
  assert.deepEqual(m.changedSites(before, sites(['a.com'], [], true)), { browser_blocked_sites: [], browser_block_private_network: true });
});

test('sitesSummary says what the closed section holds', () => {
  assert.equal(m.sitesSummary(sites()), 'Any site');
  assert.equal(m.sitesSummary(sites(['wikipedia.org'])), 'Only wikipedia.org');
  assert.equal(m.sitesSummary(sites(['a.com', 'b.com', 'c.com'], ['evil.com'], true)), 'Only 3 sites · never evil.com · no local network');
  assert.equal(m.sitesSummary(sites([], ['x.com', 'y.com'])), 'Never 2 sites');
  assert.equal(m.sitesSummary(sites([], [], true)), 'No local network');
});

test('fieldError finds the list a 400 names; other errors stay whole', () => {
  assert.deepEqual(m.fieldError('browser_allowed_sites: "a*b.com" isn\'t a site: use a domain such as example.com or *.example.com'), {
    field: 'browser_allowed_sites',
    text: '"a*b.com" isn\'t a site: use a domain such as example.com or *.example.com',
  });
  assert.deepEqual(m.fieldError('covers: at most 20 entries'), { field: 'covers', text: 'At most 20 entries' });
  assert.deepEqual(m.fieldError('Browser_blocked_sites: "*" isn\'t a site'), { field: 'browser_blocked_sites', text: '"*" isn\'t a site' }, 'as the app capitalizes it');
  assert.deepEqual(m.fieldError('avoids: each entry is at most 60 characters'), { field: 'avoids', text: 'Each entry is at most 60 characters' });
  assert.equal(m.fieldError('only the person sets which sites a bot\'s browser may open'), null);
  assert.equal(m.fieldError('Another bot is already named "Ada".'), null);
});

test('site lists split on spaces, commas and lines, keep up to 200, and leave the length to the server', () => {
  const r = m.addTags([], 'wikipedia.org, en.wikipedia.org\n*.example.com  WIKIPEDIA.org', m.SITE_LIMITS);
  assert.deepEqual(r.list, ['wikipedia.org', 'en.wikipedia.org', '*.example.com']);
  assert.equal(r.error, null);
  const long = `${'a'.repeat(70)}.com`;
  assert.deepEqual(m.addTags([], long, m.SITE_LIMITS).list, [long]);
  const full = Array.from({ length: 200 }, (_, i) => `s${i}.com`);
  assert.equal(m.addTags(full, 'one-more.com', m.SITE_LIMITS).error, 'Up to 200. Remove a site to add another.');
  assert.equal(m.addTags([], 'a'.repeat(61)).error, 'Keep each one under 60 characters.', 'Good for keeps its 60-character rule');
});

// ---------------------------------------------------------------------------
// Good for / Not for on create

test('create: Good for and Not for are on the form and reach POST /api/kwants', async () => {
  reset();
  await openCreate();
  await addTag(inputFor('Good for'), 'restaurants');
  await addTag(inputFor('Good for'), 'street food');
  await addTag(inputFor('Not for'), 'legal advice');
  assert.ok(button('Remove street food') && button('Remove legal advice'), 'the phrases show as chips');
  assert.ok(!sitesSection(), 'the Browser sites section is only in bot settings');
  await submit();
  const body = postBody();
  assert.ok(body, 'the bot was created');
  assert.deepEqual(body.covers, ['restaurants', 'street food']);
  assert.deepEqual(body.avoids, ['legal advice']);
  assert.ok(!('browser_allowed_sites' in body), 'no site rules on create');
  assert.ok(!editor(), 'the form closed');
});

test('create: with no phrases the request has no covers / avoids, as before', async () => {
  reset();
  await openCreate();
  await submit();
  const body = postBody();
  assert.ok(body);
  assert.ok(!('covers' in body) && !('avoids' in body));
});

test('create: a 400 naming covers shows next to Good for, takes focus there, and keeps the form open', async () => {
  reset();
  await openCreate();
  await addTag(inputFor('Good for'), 'food');
  answer['POST /api/kwants'] = { status: 400, body: { error: 'covers: at most 20 entries' } };
  await submit();
  const good = inputFor('Good for');
  assert.ok(editor(), 'the form stays open');
  assert.equal(errorOf(good), 'At most 20 entries');
  assert.equal(good.getAttribute('aria-invalid'), 'true');
  assert.ok(document.activeElement === good, 'focus is on Good for');
  assert.match(editor().textContent, /Nothing was saved\. Fix the entry marked above/);
  await click(button('Remove food'));
  assert.equal(errorOf(inputFor('Good for')), null, 'changing the list clears the server message');
});

test('create: typing a phrase and pressing Escape keeps the form open (it counts as unsaved)', async () => {
  reset();
  await mountSpace();
  await click(button('New bot') ?? button('New bot in Default'));
  await addTag(inputFor('Good for'), 'taxes');
  inputFor('Good for').blur();
  await key('Escape', editor());
  assert.ok(editor(), 'a Good for phrase keeps it open');
});

// ---------------------------------------------------------------------------
// Browser sites in bot settings

test('edit: Browser sites is closed with a summary, opens to two lists and the private-network toggle', async () => {
  reset();
  await openEdit();
  const s = sitesSection();
  assert.ok(s, 'the section is there');
  assert.equal(s.open, false, 'closed by default');
  assert.match(s.querySelector('summary').textContent, /Browser sites.*Only wikipedia\.org/);
  s.open = true;
  s.dispatchEvent(new Event('toggle'));
  await settle();
  assert.ok(button('Remove wikipedia.org'), 'the stored allowed site is a chip');
  assert.ok(inputFor('Blocked sites'));
  const toggle = [...s.querySelectorAll('input[type="checkbox"]')].find((c) => c.closest('label').textContent.includes('Block local and private network'));
  assert.ok(toggle && !toggle.checked, 'the toggle shows the stored value');
});

test('edit: a bot from a server without site rules has no Browser sites section', async () => {
  reset();
  kwants[1] = { ...kwants[1] };
  await openEdit('Lin');
  assert.ok(!sitesSection());
});

test('edit: a profile-only change sends no browser_* fields', async () => {
  reset();
  await openEdit();
  await type(q('#agent-name'), 'Ada2');
  await submit();
  const body = putBody();
  assert.ok(body, 'saved');
  assert.equal(body.name, 'Ada2');
  for (const k of ['browser_allowed_sites', 'browser_blocked_sites', 'browser_block_private_network']) assert.ok(!(k in body), `${k} left out`);
});

test('edit: added sites, a removed one and the toggle are sent; untouched lists are left out', async () => {
  reset();
  await openEdit();
  const s = sitesSection();
  s.open = true;
  s.dispatchEvent(new Event('toggle'));
  await settle();
  await paste(inputFor('Allowed sites'), 'en.wikipedia.org\nhttps://docs.rs/serde');
  await click(button('Remove wikipedia.org'));
  const toggle = [...s.querySelectorAll('input[type="checkbox"]')].find((c) => c.closest('label').textContent.includes('Block local'));
  await click(toggle);
  assert.match(s.querySelector('summary').textContent, /Only 2 sites · no local network/, 'the summary follows the edit');
  await submit();
  const body = putBody();
  assert.deepEqual(body.browser_allowed_sites, ['en.wikipedia.org', 'https://docs.rs/serde'], 'sent as typed; the server keeps the host');
  assert.equal(body.browser_block_private_network, true);
  assert.ok(!('browser_blocked_sites' in body), 'the untouched blocked list is left out');
});

test('edit: a 400 naming a bad site opens the section, shows the message under that list and focuses it', async () => {
  reset();
  await openEdit();
  const s = sitesSection();
  s.open = true;
  s.dispatchEvent(new Event('toggle'));
  await settle();
  await addTag(inputFor('Blocked sites'), 'a*b.com');
  s.open = false;
  s.dispatchEvent(new Event('toggle'));
  await settle();
  answer['PUT /api/kwants/k1'] = {
    status: 400,
    body: { error: 'browser_blocked_sites: "a*b.com" isn\'t a site: use a domain such as example.com or *.example.com' },
  };
  await submit();
  assert.ok(editor(), 'the editor stays open');
  assert.equal(sitesSection().open, true, 'the section opened to show it');
  const blocked = inputFor('Blocked sites');
  assert.equal(errorOf(blocked), '"a*b.com" isn\'t a site: use a domain such as example.com or *.example.com');
  assert.equal(errorOf(inputFor('Allowed sites')), null, 'only the named list');
  assert.ok(document.activeElement === blocked, 'focus is on Blocked sites');
  assert.match(editor().textContent, /Nothing was saved/);
  await click(button('Remove a*b.com'));
  assert.equal(errorOf(inputFor('Blocked sites')), null);
});

test('edit: the 403 for a bot\'s token shows as the form error', async () => {
  reset();
  await openEdit();
  const s = sitesSection();
  s.open = true;
  s.dispatchEvent(new Event('toggle'));
  await settle();
  await addTag(inputFor('Allowed sites'), 'example.com');
  answer['PUT /api/kwants/k1'] = { status: 403, body: { error: 'only the person sets which sites a bot\'s browser may open' } };
  await submit();
  assert.ok(editor());
  assert.match(editor().textContent, /Only the person sets which sites a bot's browser may open/);
  assert.ok(!sitesSection().querySelector('[role="alert"]'), 'not pinned on a list: it is about who may set them');
});

test('edit: a changed site list keeps the editor open on Escape', async () => {
  reset();
  await openEdit();
  await key('Escape', editor());
  assert.ok(!editor(), 'untouched, Escape closes it');
  await click(button('Edit Ada'));
  const s = sitesSection();
  s.open = true;
  s.dispatchEvent(new Event('toggle'));
  await settle();
  await click(button('Remove wikipedia.org'));
  await key('Escape', editor());
  assert.ok(editor(), 'a removed site keeps it open');
});
