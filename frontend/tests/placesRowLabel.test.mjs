// The place picker (apps/mac/loading/places.html): what a screen reader hears for each row.
// design-check-switcher F10 (server rows) and Rigel's fix 3 in qa-switcher.md (This Mac rows): a
// row's aria-label is its whole name, so it says the place, its status, the count when that isn't
// the status already, and "You are here", and never the keycap digit.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM, VirtualConsole } from 'jsdom';

const html = readFileSync(new URL('../../apps/mac/loading/places.html', import.meta.url), 'utf8');

function picker(rows) {
  const vc = new VirtualConsole();   // the page answers Rust by navigating to kwant-places://, which jsdom can't
  const errors = [];
  vc.on('jsdomError', (e) => { if (!/navigation/i.test(String(e.message))) errors.push(e); });
  const dom = new JSDOM(html, { runScripts: 'dangerously', pretendToBeVisual: true, virtualConsole: vc });
  dom.window.kwantPlaces.show({ mac: true, rows, actions: [] });
  const opts = [...dom.window.document.querySelectorAll('#rows [role="option"]')];
  return { opts, labels: opts.map((o) => o.getAttribute('aria-label')), errors };
}

const mac = 'This Mac';

test('every row says its place, status, count once and "You are here", server or This Mac', () => {
  const { opts, labels, errors } = picker([
    { kind: 'mac', name: mac, sub: 'Nothing waiting', n: 0, cur: true },
    { kind: 'server', name: 'kwant.jumpin.games', sub: '2 waiting', n: 2, cur: false },
    { kind: 'server', name: 'vps2.example.com', sub: "Can't reach it", bad: true, n: 0, cur: false },
    { kind: 'server', name: 'kwant-on-a-very-long-host-name.example.com', sub: "Sign in to see what's waiting", n: 0, cur: false },
  ]);
  assert.deepEqual(errors, []);
  assert.equal(opts.length, 4);
  assert.deepEqual(labels, [
    `${mac}, Nothing waiting, You are here`,
    'kwant.jumpin.games, 2 waiting',
    "vps2.example.com, Can't reach it",
    "kwant-on-a-very-long-host-name.example.com, Sign in to see what's waiting",
  ]);
});

test('a This Mac row keeps "You are here" when its count takes the ✓\'s place, and says the count once', () => {
  // Rigel's jsdom readings before the fix: "This Mac Running You are here 1" and
  // "This Mac 3 waiting 3 waiting 9" (keycap digit read, count twice, no "You are here").
  const { labels } = picker([
    { kind: 'server', name: 'kwant.jumpin.games', sub: 'Nothing waiting', n: 0, cur: false },
    { kind: 'mac', name: mac, sub: '3 waiting', n: 3, cur: true },
    { kind: 'mac', name: mac, sub: 'Not running', n: 0, cur: false },
    { kind: 'server', name: 'busy.example.com', sub: '12 waiting', n: 12, cur: true },
  ]);
  assert.deepEqual(labels, [
    'kwant.jumpin.games, Nothing waiting',
    `${mac}, 3 waiting, You are here`,
    `${mac}, Not running`,
    'busy.example.com, 12 waiting, You are here',
  ]);
  for (const l of labels) assert.doesNotMatch(l, /\d$/, `no keycap digit at the end: ${l}`);
});

test('a count that is not the status is still said', () => {
  const { labels } = picker([{ kind: 'mac', name: mac, sub: 'Starting…', n: 2, cur: true }]);
  assert.deepEqual(labels, [`${mac}, Starting…, 2 waiting, You are here`]);
});
