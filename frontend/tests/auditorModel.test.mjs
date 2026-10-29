// Settings → Audit model (docs/mission-control.md §18): next to the auditor switch, a picker for
// `auditor_model` (fast by default, balanced, deep, or the bot's own model). It is saved through
// /api/settings with the rest, a 400 about it shows next to the picker, and a server without the
// setting shows no picker and gets nothing new in the save. Also: task classes read as words.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}

const a = await loadBundled(`export * from './src/auditor';`);
const fs = await loadBundled(`export { classLabel } from './src/flightSchool'; export { classText } from './src/performance';`);
const ui = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { AuditorModelSetting } from './src/components/AuditorModelSetting';
  export { createRoot } from 'react-dom/client';
  export { React, flushSync, AuditorModelSetting };
`);

const mount = (props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = ui.createRoot(host);
  ui.flushSync(() => root.render(ui.React.createElement(ui.AuditorModelSetting, props)));
  return { host, root };
};

test('settings: auditor_model is read when the server sends it, and left out when it does not', () => {
  assert.deepEqual(a.parseAuditorSettings({ auditor: 'on', auditor_daily_cap: 5, auditor_model: 'fast' }), { auditor: 'on', auditor_daily_cap: 5, auditor_model: 'fast' });
  assert.deepEqual(a.parseAuditorSettings({ auditor: 'off', auditor_daily_cap: 5, auditor_model: 'bot' }).auditor_model, 'bot');
  assert.equal('auditor_model' in a.parseAuditorSettings({ auditor: 'off', auditor_daily_cap: 5 }), false, 'older server: nothing new is saved');
  assert.equal(a.parseAuditorSettings({ auditor: 'on', auditor_model: 'turbo' }).auditor_model, 'turbo', 'an unknown value is kept, not silently changed');
});

test('the picker lists fast, balanced, deep and "same as the bot", with the plain hint', () => {
  const { host, root } = mount({ value: 'fast', error: null, onChange: () => {} });
  const sel = host.querySelector('select#auditor-model');
  assert.ok(sel);
  assert.equal(host.querySelector('label[for="auditor-model"]').textContent, 'Audit model');
  assert.deepEqual([...sel.options].map((o) => [o.value, o.textContent]), [
    ['fast', 'Fast'],
    ['balanced', 'Balanced'],
    ['deep', 'Deep'],
    ['bot', 'Same as the bot'],
  ]);
  assert.equal(sel.value, 'fast');
  assert.equal(a.MODEL_HINT, 'Fast is cheapest; audits never mark work passed or failed.');
  assert.match(host.textContent, /Fast is cheapest; audits never mark work passed or failed\./);
  assert.equal(sel.getAttribute('aria-describedby'), 'auditor-model-hint');
  assert.equal(host.querySelector('[role="alert"]'), null);
  root.unmount();
});

test('choosing a model reports it; an unknown saved value still shows as the current choice', () => {
  const picked = [];
  const { host, root } = mount({ value: 'fast', error: null, onChange: (m) => picked.push(m) });
  const sel = host.querySelector('select');
  sel.value = 'deep';
  sel.dispatchEvent(new window.Event('change', { bubbles: true }));
  assert.deepEqual(picked, ['deep']);
  root.unmount();
  const odd = mount({ value: 'turbo', error: null, onChange: () => {} });
  const s2 = odd.host.querySelector('select');
  assert.equal(s2.value, 'turbo');
  assert.equal(s2.options[0].textContent, 'turbo');
  assert.equal(s2.options.length, 5);
  odd.root.unmount();
});

test('a 400 about auditor_model shows inline, next to the picker', () => {
  const msg = 'auditor_model must be fast, balanced, deep or bot (got turbo)';
  assert.equal(a.isAuditorModelError(msg), true);
  assert.equal(a.isAuditorModelError('auditor_daily_cap must be 1–50'), false);
  assert.equal(a.isAuditorModelError(null), false);
  assert.equal(a.MODEL_ERROR_TEXT, 'The server didn’t accept that audit model. Pick Fast, Balanced, Deep or Same as the bot.');
  const { host, root } = mount({ value: 'turbo', error: a.MODEL_ERROR_TEXT, onChange: () => {} });
  const alert = host.querySelector('[role="alert"]#auditor-model-error');
  assert.ok(alert);
  assert.equal(alert.textContent, a.MODEL_ERROR_TEXT, 'plain words, not the server’s field name');
  const sel = host.querySelector('select');
  assert.equal(sel.getAttribute('aria-invalid'), 'true');
  assert.equal(sel.getAttribute('aria-describedby'), 'auditor-model-hint auditor-model-error');
  root.unmount();
});

test('Settings wires the picker into the auditor switch and the save', () => {
  const src = readFileSync(new URL('../src/components/SettingsModal.tsx', import.meta.url), 'utf8');
  assert.match(src, /const modelError = isAuditorModelError\(saveError\)/);
  assert.match(src, /\(auditor\.auditor === 'on' \|\| modelError\) && auditor\.auditor_model !== undefined && \(\s*<AuditorModelSetting/);
  assert.match(src, /error=\{modelError \? AUDITOR_MODEL_ERROR : null\}/);
  assert.match(src, /\.\.\.\(auditor \?\? \{\}\)/, 'auditor_model goes out with the other auditor settings');
});

test('task classes read as words: code-review → Code review', () => {
  assert.equal(fs.classLabel('code-review'), 'Code review');
  assert.equal(fs.classLabel('data_cleanup'), 'Data cleanup');
  assert.equal(fs.classLabel('research'), 'Research');
  assert.equal(fs.classLabel('general'), 'General');
  assert.equal(fs.classLabel('*'), 'All work');
  assert.equal(fs.classLabel('--'), '--', 'nothing readable left: keep the raw value');
  assert.equal(fs.classText('code-review'), 'Code review', 'the Performance section uses the same label');
});
