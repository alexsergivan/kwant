import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

// The sound effects module reads its mute flag on load.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export { isBlankPage, browserFooterLabel } from './src/computerMode';
  export { VirtualMachinePane } from './src/components/VirtualMachinePane';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);

const settings = (mode, visible = false) => ({
  mode,
  available: { local: true, vm: true },
  reasons: { local: null, vm: null },
  window: { visible, available: true, reason: null },
});
const pane = (props) =>
  m.renderToStaticMarkup(m.createElement(m.VirtualMachinePane, { isOpen: true, onClose: () => {}, teamId: 'k1', screenshotBase64: 'AAAA', ...props }));

test('an empty or about:blank tab counts as not opened yet', () => {
  for (const url of ['', '  ', 'about:blank', null, undefined]) assert.equal(m.isBlankPage(url), true, String(url));
  for (const url of ['https://maps.google.com/', 'about:blank#x', 'http://127.0.0.1:8080/']) assert.equal(m.isBlankPage(url), false, url);
});

test('before the bot opens a page the panel says "Browser idle" instead of the blank frame', () => {
  for (const activeUrl of ['', 'about:blank']) {
    const html = pane({ activeUrl });
    assert.match(html, /Browser idle/);
    assert.doesNotMatch(html, /alt="Browser Viewport"/);
  }
});

test('once the bot has opened a page the live frame shows', () => {
  const html = pane({ activeUrl: 'https://www.google.com/maps' });
  assert.match(html, /alt="Browser Viewport"/);
  assert.match(html, /data:image\/jpeg;base64,AAAA/);
  assert.doesNotMatch(html, /Browser idle/);
});

test('the footer says where Chrome runs and where you see it', () => {
  assert.equal(m.browserFooterLabel(settings('local'), true), 'Chrome shown here (no window)');
  assert.equal(m.browserFooterLabel(settings('local', true), true), 'Chrome window on your computer');
  assert.equal(m.browserFooterLabel(settings('local'), false), 'Chrome offline');
  assert.equal(m.browserFooterLabel(settings('vm'), true), 'Browser Ready');
  assert.equal(m.browserFooterLabel(settings('vm'), false), 'Browser Offline');
  // Settings not loaded yet: no claim about the user's computer.
  assert.equal(m.browserFooterLabel(null, true), 'Browser Ready');
});
