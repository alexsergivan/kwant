import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

const source = await readFile(new URL('../src/browserViewport.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext },
});
const { mapBrowserPoint, browserStreamUrl } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);

test('maps a click near the top of a vertically letterboxed screenshot', () => {
  // A 1280 x 720 screenshot occupies y=320..500 in this tall pane.
  const bounds = { left: 100, top: 200, width: 320, height: 420 };
  assert.deepEqual(mapBrowserPoint(125, 330, bounds, 1280, 720), { x: 100, y: 40 });
  assert.deepEqual(mapBrowserPoint(395, 490, bounds, 1280, 720), { x: 1180, y: 680 });
  assert.equal(mapBrowserPoint(125, 250, bounds, 1280, 720), null);
  assert.equal(mapBrowserPoint(125, 550, bounds, 1280, 720), null);
});

test('maps horizontal letterboxing and rejects its empty margins', () => {
  const bounds = { left: 10, top: 20, width: 600, height: 200 };
  assert.deepEqual(mapBrowserPoint(170, 30, bounds, 1280, 800), { x: 80, y: 40 });
  assert.equal(mapBrowserPoint(100, 30, bounds, 1280, 800), null);
  assert.equal(mapBrowserPoint(500, 30, bounds, 1280, 800), null);
});

test('uses the actual screenshot size and stays within its edges', () => {
  const bounds = { left: 10, top: 20, width: 500, height: 300 };
  assert.deepEqual(mapBrowserPoint(260, 170, bounds, 1000, 600), { x: 500, y: 300 });
  assert.deepEqual(mapBrowserPoint(509.99, 319.99, bounds, 1000, 600), { x: 999, y: 599 });
  assert.equal(mapBrowserPoint(510, 320, bounds, 1000, 600), null);
});

test('ignores unloaded images and collapsed panes', () => {
  const bounds = { left: 0, top: 0, width: 320, height: 420 };
  assert.equal(mapBrowserPoint(50, 50, bounds, 0, 0), null);
  assert.equal(mapBrowserPoint(0, 0, { ...bounds, width: 0 }, 1280, 800), null);
});

test('streams through the VPS origin with TLS and encodes session IDs', () => {
  assert.equal(browserStreamUrl({ protocol: 'https:', host: 'bots.example.com' }, 'team/a'), 'wss://bots.example.com/api/sandbox/team%2Fa/stream');
  assert.equal(browserStreamUrl({ protocol: 'http:', host: '127.0.0.1:8899' }, 'bot'), 'ws://127.0.0.1:8899/api/sandbox/bot/stream');
});
