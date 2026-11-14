import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';
import { loadBundled } from './bundle.mjs';

// 0.9.8 browser panel (design browser-panel/design.md D): the panel closes itself only when a bot opened it.
const load = async (path) => {
  const source = await readFile(new URL(path, import.meta.url), 'utf8');
  const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } });
  return import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
};
const { shouldAutoClose, shouldAutoOpen, paneStatus, AUTO_CLOSE_DELAY_MS } = await loadBundled("export * from './src/browserPanel.ts';") // imports clock.ts, so it's bundled;

const ended = { by: 'bot', touched: false, control: false, humanStep: false, pointPending: false, running: false };

test('a panel the bot opened closes once its turn ends and nobody used it', () => {
  assert.equal(shouldAutoClose(ended), true);
  assert.equal(AUTO_CLOSE_DELAY_MS, 600);
});

test('it stays open while the turn runs', () => {
  assert.equal(shouldAutoClose({ ...ended, running: true }), false);
});

test('a panel the person opened never closes itself', () => {
  assert.equal(shouldAutoClose({ ...ended, by: 'person' }), false);
  assert.equal(shouldAutoClose({ ...ended, by: null }), false);
});

test('it stays open while the person uses it: touched, in control, a step waiting on them, Point picks pending', () => {
  for (const k of ['touched', 'control', 'humanStep', 'pointPending']) assert.equal(shouldAutoClose({ ...ended, [k]: true }), false, k);
});

test('a browser event never opens the panel on a phone, nor reopens an open one', () => {
  assert.equal(shouldAutoOpen({ open: false, narrow: false }), true);
  assert.equal(shouldAutoOpen({ open: false, narrow: true }), false);
  assert.equal(shouldAutoOpen({ open: true, narrow: false }), false);
});

test('the header status: Browsing now, Finished with the time, Idle', () => {
  assert.deepEqual(paneStatus({ running: true, finishedAt: '2026-11-13T22:46:00Z' }), { kind: 'browsing', text: 'Browsing now' });
  const f = paneStatus({ running: false, finishedAt: '2026-11-13T22:46:00Z', locale: 'en-US' });
  assert.equal(f.kind, 'finished');
  assert.match(f.text, /^Finished \d{1,2}:46/);
  assert.deepEqual(paneStatus({ running: false, finishedAt: null }), { kind: 'idle', text: 'Idle' });
});
