// House (26e) must read as a house at 22 px, not a party hat on a head (Oleks, Nov 16: v1 did).
// Guards the v3 shape: eaves wider than the walls, a chimney on the left (mood badges sit top-right),
// and nothing of the roof over the face (y 10+), so blink and asleep lids still show.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const s = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ModernAvatar } from './src/components/ModernAvatar';
  export const svg = (p) => renderToStaticMarkup(React.createElement(ModernAvatar, { size: 22, ...p }));
`);

const rects = (html) => [...html.matchAll(/<rect x="(\d+)" y="(\d+)" width="(\d+)" height="(\d+)" fill="([^"]+)"/g)]
  .map(([, x, y, w, h, fill]) => ({ x: +x, y: +y, w: +w, h: +h, fill }));

test('House has eaves past its walls, a left chimney, and a face the roof never covers', () => {
  const r = rects(s.svg({ role: 'home', name: 'House' }));
  const eave = r.find((e) => e.fill === '#a8323a' && e.h === 1 && e.w >= 16);
  assert.ok(eave, 'an eave at least 16 px wide');
  assert.ok(eave.x <= 5 && eave.x + eave.w >= 21, 'eaves reach 2 px past both walls (x 7-18)');
  assert.ok(eave.y <= 8, 'the roof ends at or above y 8');
  const chimney = r.find((e) => e.fill === '#9a5a46');
  assert.ok(chimney && chimney.x + chimney.w <= 13, 'chimney on the left half');
  const roofish = new Set(['#a8323a', '#c23a42', '#ff8f8f', '#9a5a46', '#c98a6e']);
  assert.ok(!r.some((e) => roofish.has(e.fill) && e.y + e.h > 9), 'no roof pixel at y 9 or below');
});

test('every mood still draws House the same shape', () => {
  for (const mood of ['working', 'done', 'error', 'needs_you', 'thinking']) {
    const html = s.svg({ role: 'home', name: 'House', mood });
    assert.ok(rects(html).some((e) => e.fill === '#a8323a' && e.w >= 16), `${mood}: eave present`);
  }
});
