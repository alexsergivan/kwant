// The Composio tile (composio-simple/design.md §3): one key adds it, status chip and facts, Replace
// key, Turn off, Remove asks inline with Keep it focused, the old link mode offers the switch, a
// refused key opens the field with the fix, and the main path never says MCP or header.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.window.HTMLElement.prototype.scrollIntoView ??= () => {};

const m = await loadBundled(`
  export { ComposioTile, shortList, removeQuestion } from './src/components/ComposioTile';
  export { ConnectionsError } from './src/connections';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
`);
const h = m.createElement;
const PLUMBING = /\b(MCP|header|user_id|toolkit|server URL|connector)\b/i;

const entry = (over = {}) => ({
  id: 'composio', kind: 'mcp', name: 'Composio', description: '', optional: true, added: false, connection_id: null, enabled: false,
  logins_stored: 'composio', warning: '', add: '/api/connections/composio', key_url: 'https://dashboard.composio.dev', ...over,
});
const added = (over = {}) =>
  entry({ added: true, enabled: true, connection_id: 'mcp:cmp1', mode: 'connect', apps_connected: [{ slug: 'gmail', name: 'Gmail' }, { slug: 'slack', name: 'Slack' }], ...over });

const stubApi = (over = {}) => {
  const calls = [];
  const api = {
    addKey: async (k) => void calls.push(['addKey', k]),
    replaceKey: async (k) => void calls.push(['replaceKey', k]),
    switchTo: async (k) => void calls.push(['switchTo', k ?? null]),
    setEnabled: async (id, on) => void calls.push(['setEnabled', id, on]),
    remove: async (id) => void calls.push(['remove', id]),
    ...over,
  };
  return { api, calls };
};

const mounted = new Set();
afterEach(() => {
  for (const u of mounted) u();
  document.body.replaceChildren();
});
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mount = async (props) => {
  const host = document.createElement('ul');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(h(m.ComposioTile, { usedBy: [], onAddLink: async () => null, ...props }));
  await settle();
  const u = () => {
    if (!mounted.delete(u)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(u);
  return host;
};
const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true, cancelable: true }));
  await settle();
};
const fill = async (el, value) => {
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(el, value);
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
};
const submit = async (form) => {
  form.dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
};

test('names and lists stay short', () => {
  assert.equal(m.shortList(['Ann']), 'Ann');
  assert.equal(m.shortList(['Ann', 'Ben']), 'Ann and Ben');
  assert.equal(m.shortList(['Ann', 'Ben', 'Cy', 'Di']), 'Ann, Ben and 2 more');
  assert.equal(m.shortList(['Gmail', 'Slack'], 'or'), 'Gmail or Slack');
  assert.equal(m.removeQuestion(['Ada'], ['Gmail', 'Slack']), 'Remove Composio? Ada loses Gmail and Slack. Your sign-ins stay on Composio, so you can add it back later.');
  assert.equal(m.removeQuestion([], []), 'Remove Composio? Your sign-ins stay on Composio, so you can add it back later.');
});

test('not added: one key field adds it, and the link form hides under advanced', async () => {
  const { api, calls } = stubApi();
  let changed = 0;
  const host = await mount({ entry: entry(), api, onChanged: () => changed++ });
  assert.equal(host.querySelector('[data-composio-tile]').dataset.composioTile, 'new');
  assert.ok(!PLUMBING.test(text(host)), 'no plumbing words');
  await click(host.querySelector('[data-composio-add]'));
  const field = host.querySelector('input[type="password"]');
  assert.ok(field, 'the key field opens');
  assert.ok(!PLUMBING.test(text(host)), 'no plumbing words with the field open');
  assert.ok(!host.querySelector('[data-composio-link-form]'));
  await fill(field, 'ck_test');
  await submit(field.form);
  assert.deepEqual(calls, [['addKey', 'ck_test']]);
  assert.equal(changed, 1);
  assert.match(text(host.querySelector('[data-composio-notice]')), /Composio is added\. No bot has it until you choose bots\./);
});

test('the advanced link form is the only place that says MCP', async () => {
  const links = [];
  const host = await mount({ entry: entry(), initialOpen: 'add', onAddLink: async (u, k, on) => (links.push([u, k, on]), 'Composio already added.') });
  const toggle = host.querySelector('[data-composio-link-toggle]');
  assert.equal(text(toggle), 'Use a Composio link instead (advanced)');
  assert.equal(toggle.getAttribute('aria-expanded'), 'false');
  await click(toggle);
  const form = host.querySelector('[data-composio-link-form]');
  assert.ok(form && text(form).includes('Composio MCP URL'));
  await fill(form.querySelector('input[type="url"]'), ' https://backend.composio.dev/v3/mcp/x ');
  await submit(form);
  assert.deepEqual(links, [['https://backend.composio.dev/v3/mcp/x', '', false]]);
  assert.equal(text(form.querySelector('[role="alert"]')), 'Composio already added.');
});

test('on: chip, facts, and Choose bots · Replace key · Turn off · Remove', async () => {
  const host = await mount({ entry: added(), usedBy: ['Ada', 'Lin'], onChooseBots() {}, api: stubApi().api });
  assert.equal(text(host.querySelector('[data-composio-chip]')), 'Added · on');
  assert.equal(text(host.querySelector('[data-fact="apps"] dd')), 'Gmail and Slack');
  assert.equal(text(host.querySelector('[data-fact="used-by"] dd')), 'Ada and Lin');
  const buttons = [...host.querySelectorAll('button')].map(text);
  assert.deepEqual(buttons, ['Choose bots', 'Replace key', 'Turn off', 'Remove']);
  assert.ok(text(host).includes('Gmail and hundreds more apps. Your bots find the app they need by themselves.'));
  assert.ok(!PLUMBING.test(text(host)));
});

test('off: the primary action is Choose bots to turn it on, and no Turn off', async () => {
  const host = await mount({ entry: added({ enabled: false, apps_connected: [] }), onChooseBots() {}, api: stubApi().api });
  assert.equal(text(host.querySelector('[data-composio-chip]')), 'Added · off');
  assert.deepEqual([...host.querySelectorAll('button')].map(text), ['Choose bots to turn it on', 'Replace key', 'Remove']);
  assert.equal(text(host.querySelector('[data-fact="apps"] dd')), 'None yet. Find one with Find a tool above.');
  assert.equal(text(host.querySelector('[data-fact="used-by"] dd')), 'No bot yet');
});

test('Turn off and Replace key call the server with the right thing', async () => {
  const { api, calls } = stubApi();
  const host = await mount({ entry: added(), api });
  await click(host.querySelector('[data-composio-off]'));
  assert.deepEqual(calls.at(-1), ['setEnabled', 'cmp1', false]);
  await click(host.querySelector('[data-composio-replace]'));
  const field = host.querySelector('input[type="password"]');
  assert.ok(text(host).includes('New Composio API key'));
  assert.ok(text(host).includes('A key from another Composio project starts fresh there, so you’d connect Gmail and Slack again.'));
  await fill(field, 'ck_new');
  await submit(field.form);
  assert.deepEqual(calls.at(-1), ['replaceKey', 'ck_new']);
});

test('Remove asks inline, focuses Keep it, and only removes on Remove Composio', async () => {
  const { api, calls } = stubApi();
  const host = await mount({ entry: added(), usedBy: ['Ada'], api });
  await click(host.querySelector('[data-composio-remove]'));
  const ask = host.querySelector('[data-composio-remove-ask]');
  assert.ok(text(ask).startsWith('Remove Composio? Ada loses Gmail and Slack. Your sign-ins stay on Composio, so you can add it back later.'));
  assert.equal(document.activeElement, host.querySelector('[data-composio-keep]'));
  await click(host.querySelector('[data-composio-keep]'));
  assert.ok(!host.querySelector('[data-composio-remove-ask]'));
  assert.equal(calls.length, 0);
  await click(host.querySelector('[data-composio-remove]'));
  await click(host.querySelector('[data-composio-remove-yes]'));
  assert.deepEqual(calls, [['remove', 'cmp1']]);
});

test('a failed Remove says so next to the Remove button', async () => {
  const { api } = stubApi({ remove: async () => { throw new Error('Couldn’t remove it. Try again.'); } });
  const host = await mount({ entry: added(), api });
  await click(host.querySelector('[data-composio-remove]'));
  await click(host.querySelector('[data-composio-remove-yes]'));
  const err = host.querySelector('[data-composio-remove-ask] [data-tile-error="remove"]');
  assert.ok(err && err.getAttribute('role') === 'alert');
});

test('a refused saved key: rose line, the key field already open with no Cancel, then Turn off · Remove', async () => {
  const host = await mount({
    entry: added({ problem: { message: 'Composio refused the key.', action: 'replace_key' } }),
    usedBy: ['Ada'],
    api: stubApi().api,
  });
  assert.equal(text(host.querySelector('[data-composio-chip]')), 'Key not accepted');
  assert.equal(text(host.querySelector('[data-composio-refused]')), 'Composio didn’t accept the saved key, so Ada can’t use Gmail or Slack right now. Paste a new key to fix it.');
  assert.ok(host.querySelector('input[type="password"]'));
  const buttons = [...host.querySelectorAll('button')].map(text);
  assert.ok(!buttons.includes('Cancel'));
  assert.ok(buttons.includes('Save key') && buttons.includes('Turn off') && buttons.includes('Remove'));
  assert.ok(text(host).includes('your Composio dashboard') || text(host).includes('From your Composio dashboard'));
});

test('Composio down while listing apps: says so with Try again in the facts', async () => {
  let changed = 0;
  const host = await mount({ entry: added({ problem: { message: 'down', action: 'try_again' }, apps_connected: [] }), api: stubApi().api, onChanged: () => changed++ });
  assert.ok(text(host.querySelector('[data-fact="apps"]')).includes('Couldn’t reach Composio to list them.'));
  await click(host.querySelector('[data-composio-retry]'));
  assert.equal(changed, 1);
});

test('the old link setup offers the switch; with no saved key it asks for one right there', async () => {
  const { api, calls } = stubApi({
    switchTo: async (k) => {
      calls.push(['switchTo', k ?? null]);
      if (!k) throw new m.ConnectionsError('needs a key', 400);
    },
  });
  const host = await mount({ entry: added({ mode: 'url', can_switch: true }), api });
  assert.equal(text(host.querySelector('[data-fact="setup"] dd')), 'a Composio link');
  assert.ok(text(host.querySelector('[data-composio-switch]')).startsWith('Switch to the one-key setup to connect apps right here.'));
  assert.ok(!host.querySelector('[data-composio-replace]'), 'no Replace key in link mode');
  await click(host.querySelector('[data-composio-switch-go]'));
  assert.ok(host.querySelector('[data-composio-switch-needs-key]'));
  const field = host.querySelector('input[type="password"]');
  await fill(field, 'ck_k');
  await submit(field.form);
  assert.deepEqual(calls, [['switchTo', null], ['switchTo', 'ck_k']]);
  assert.equal(text(host.querySelector('[data-composio-notice]')), 'Switched to the one-key setup. Gmail and Slack are still connected.');
});

test('the footer always says where sign-ins live', async () => {
  for (const e of [entry(), added(), added({ enabled: false })]) {
    const host = await mount({ entry: e, api: stubApi().api });
    assert.equal(text(host.querySelector('[data-composio-footer]')), 'Your app sign-ins stay on Composio’s servers, not in kwant’s vault.');
  }
});
