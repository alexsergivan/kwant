// House (26e) must read as a house at 22 px, not a party hat on a smiley (Oleks, Nov 16: v1 and v3 did).
// v4 (Mira): the body IS the house. Guards the shape: the eave (y 9) is the widest row and overhangs the
// walls, a door sits at the bottom centre, the chimney is on the left (mood badges sit top-right), no
// 'roof' accessory is drawn on a round body, and House comes from its role only, never from a name.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const s = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ModernAvatar } from './src/components/ModernAvatar';
  export const svg = (p) => renderToStaticMarkup(React.createElement(ModernAvatar, { size: 22, ...p }));
`);

const FRAME = new Set(['#4a3f8a', '#15112e']);
const rects = (html) => [...html.matchAll(/<rect x="(\d+)" y="(\d+)" width="(\d+)" height="(\d+)" fill="([^"]+)"/g)]
  .map(([, x, y, w, h, fill]) => ({ x: +x, y: +y, w: +w, h: +h, fill }))
  .filter((r) => !FRAME.has(r.fill));
const DOOR = '#8f5b4a';
const isHouse = (html) => rects(html).some((r) => r.fill === DOOR && r.w === 4 && r.h === 4 && r.x === 11 && r.y === 16);

test('House is a house: a door, the eave at y 9 is the widest row, a chimney on the left', () => {
  const r = rects(s.svg({ role: 'home', name: 'House' }));
  assert.ok(isHouse(s.svg({ role: 'home', name: 'House' })), 'a 4x4 door at the bottom centre (x 11, y 16)');
  assert.ok(r.some((e) => e.fill === '#ffe066' && e.x === 13 && e.y === 17), 'the door has its knob');
  // Width of each sprite row y 2..19 (sprite rects only: the ground shadow at y 20 is left out).
  const width = (y) => {
    const xs = r.filter((e) => e.y <= y && y < e.y + e.h && e.y < 20).flatMap((e) => [e.x, e.x + e.w]);
    return xs.length ? Math.max(...xs) - Math.min(...xs) : 0;
  };
  const eave = width(9);
  assert.equal(eave, 16, 'the eave spans x 5-20');
  for (let y = 2; y < 20; y++) if (y !== 9) assert.ok(width(y) < eave, `row ${y} (${width(y)}) is narrower than the eave`);
  const walls = r.find((e) => e.y === 10 && e.h === 10 && e.w === 12);
  assert.ok(walls && walls.x === 7, 'square 12-wide walls at x 7-18, 2 px inside each eave end');
  const chimney = r.find((e) => e.x === 8 && e.y === 2 && e.h === 4);
  assert.ok(chimney, 'chimney on the left slope (x 8-9)');
});

test('no v3 roof is drawn: nothing in front of the face, no round body under a hat', () => {
  const html = s.svg({ role: 'home', name: 'House' });
  // v3 drew its eave in #a8323a 16 wide at y 8 over the round body; v4's eave is the accent at y 9.
  assert.ok(!rects(html).some((e) => e.y === 8 && e.w === 16), 'no v3 eave at y 8');
  // The round creature body starts with a 8-wide row at (9, 7); a house has none.
  assert.ok(!rects(html).some((e) => e.x === 9 && e.y === 7 && e.w === 8 && e.h === 1), 'no round body');
});

test('every mood still draws the same house', () => {
  for (const mood of ['working', 'done', 'error', 'needs_you', 'thinking']) {
    assert.ok(isHouse(s.svg({ role: 'home', name: 'House', mood })), `${mood}: still a house`);
  }
});

test('House comes from its role only: a name or title with "home" or "house" is not a house', () => {
  for (const p of [
    { role: 'writer', name: 'Homework helper' },
    { role: 'Homework helper', name: 'Homework helper' },
    { role: 'Home office', name: 'Home office' },
    { role: 'Housekeeping', name: 'Housekeeping' },
    { name: 'Home office' },
  ]) {
    assert.ok(!isHouse(s.svg(p)), `${JSON.stringify(p)} must not be a house`);
  }
});
