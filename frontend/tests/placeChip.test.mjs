// Oleks (2026-11-14): an easier way to switch between This Mac and a server in the desktop app, plus settings
// for the Perch. desktop-island/build-brief.md: the chip only in the app; the waiting-elsewhere count and the
// Perch section only where Rust sets __KWANT_ELSEWHERE__ / __KWANT_ISLAND__ (the This Mac window).
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { PlaceChipRow, PerchSettingsCard } from './src/components/PlaceChip';
  import { Sidebar } from './src/components/Sidebar';
  export * from './src/placeChip';
  const r = (C, p) => renderToStaticMarkup(React.createElement(C, p));
  export const chip = (p) => r(PlaceChipRow, p);
  export const perch = (p) => r(PerchSettingsCard, p);
  const engines = { engines: [], loading: false, error: null, refresh: () => {} };
  export const sidebar = (p) =>
    r(Sidebar, {
      spaces: [], onSwitchSpace() {}, onCreateSpace() {}, onOpenSpaceSettings() {}, onOpenArchived() {}, onOpenSearch() {}, onOpenRoutines() {},
      channels: [], teams: [], selectedId: '', isChannel: false, onSelectKwant() {}, onSelectChannel() {}, onSelectTeam() {}, onSummonKwant() {}, onCreateTeam() {},
      engines, kwants: [], ...p,
    });
`);

const space = { id: 'spc_1', name: 'Work', icon: '💼', color: null, sort: 0, is_default: false, archived_at: null, created_at: '', general_channel_id: null, counts: { bots: 0, ready_accounts: 1, unread: 0 }, approvals: 'ask' };

/** Renders with `globalThis.window` set to a fake window for the duration of `fn`. */
function inWindow(props, fn) {
  const assigned = [];
  const listeners = {};
  const w = {
    location: { host: props.host ?? '127.0.0.1:8799', assign: (u) => assigned.push(u) },
    addEventListener: (n, f) => (listeners[n] = f),
    removeEventListener: () => {},
    setTimeout: () => 0,
    navigator: { userAgent: "test" },
    matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }),
    ...props.marks,
  };
  const before = globalThis.window;
  globalThis.window = w;
  try {
    return fn(w, assigned);
  } finally {
    if (before === undefined) delete globalThis.window;
    else globalThis.window = before;
  }
}

test('web build: no chip, in the chip row or in the whole Sidebar', () => {
  inWindow({ marks: {} }, () => {
    assert.equal(m.chip({}), '');
    assert.equal(m.chip({ phone: true }), '');
    const html = m.sidebar({ space });
    assert.doesNotMatch(html, /data-place-row|data-place-chip/);
  });
  // A window that claims some other mark is still the web build.
  inWindow({ marks: { __KWANT_APP__: 'windows' } }, () => assert.equal(m.chip({}), ''));
  inWindow({ marks: { __KWANT_APP__: true } }, () => assert.equal(m.chip({}), ''));
});

test('desktop app: the chip heads the Sidebar, above the space header, even with one place', () => {
  inWindow({ marks: { __KWANT_APP__: 'mac' } }, () => {
    const html = m.sidebar({ space });
    const row = html.indexOf('data-place-row');
    assert.ok(row > 0, 'chip row present');
    assert.ok(html.indexOf('>Work<') > row, 'above the space name');
    assert.ok(html.indexOf('<aside') < row, 'inside the aside');
    assert.match(html, /This Mac/);
    const phone = m.sidebar({ space, phone: true });
    assert.match(phone, /data-phone="" data-place-row/);
  });
});

test('no badge without __KWANT_ELSEWHERE__, and none for zero or junk', () => {
  inWindow({ marks: { __KWANT_APP__: 'mac' } }, () => {
    const html = m.chip({});
    assert.doesNotMatch(html, /data-place-elsewhere/);
    assert.match(html, /aria-label="Place: This Mac\. Switch place"/);
  });
  for (const junk of [0, -2, 1.5, '3', null, {}, NaN]) {
    inWindow({ marks: { __KWANT_APP__: 'mac', __KWANT_ELSEWHERE__: junk } }, () => {
      assert.doesNotMatch(m.chip({}), /data-place-elsewhere/, `junk ${String(junk)}`);
    });
  }
});

test('the badge shows the count Rust gave, with the label saying it', () => {
  inWindow({ marks: { __KWANT_APP__: 'mac', __KWANT_ELSEWHERE__: 2 } }, () => {
    const html = m.chip({});
    assert.match(html, /data-place-elsewhere="true">2</);
    assert.match(html, /aria-label="Place: This Mac, 2 waiting in other places\. Switch place"/);
  });
  inWindow({ marks: { __KWANT_APP__: 'mac', __KWANT_ELSEWHERE__: 14 } }, () => {
    assert.match(m.chip({}), /data-place-elsewhere="true">9\+</);
  });
});

test('a server window shows its own host, middle-cut at 28, and never a count it was not given', () => {
  inWindow({ host: 'kwant.jumpin.games', marks: { __KWANT_APP__: 'mac' } }, () => {
    const html = m.chip({});
    assert.match(html, /data-place-chip="server"/);
    assert.match(html, />kwant\.jumpin\.games</);
    assert.doesNotMatch(html, /data-place-elsewhere/);
  });
  const long = 'very-long-subdomain.kwant.example.com';
  assert.equal(m.middleCut(long).length, 28);
  assert.equal(m.middleCut(long), 'very-long-subd…t.example.com');
  assert.equal(m.middleCut('short.host'), 'short.host');
  assert.equal(m.placeOf({ location: { host: 'srv.example.com:443' } }).full, 'srv.example.com');
});

test('This Mac on a Mac, This computer on Linux; localhost counts as this computer', () => {
  assert.equal(m.placeOf({ __KWANT_APP__: 'mac', location: { host: '127.0.0.1:8799' } }).name, 'This Mac');
  assert.equal(m.placeOf({ __KWANT_APP__: 'linux', location: { host: '127.0.0.1:8799' } }).name, 'This computer');
  assert.equal(m.placeOf({ __KWANT_APP__: 'mac', location: { host: 'localhost:8799' } }).kind, 'local');
  // Another port on this computer is not This Mac's own server.
  assert.equal(m.placeOf({ __KWANT_APP__: 'mac', location: { host: '127.0.0.1:8800' } }).kind, 'server');
});

test('the picker URL is whole, non-negative window pixels, and clicks pass at most once a second', () => {
  assert.equal(m.placesUrl(12.4, 44.6), 'kwant-app://places?at=12,45');
  assert.equal(m.placesUrl(-3, NaN), 'kwant-app://places?at=0,0');
  let t = 0;
  const gate = m.onceEvery(1000, () => t);
  assert.equal(gate(), true);
  t = 400;
  assert.equal(gate(), false);
  t = 999;
  assert.equal(gate(), false);
  t = 1000;
  assert.equal(gate(), true);
});

test('no Perch section without __KWANT_ISLAND__, nor in the web build', () => {
  inWindow({ marks: { __KWANT_APP__: 'mac' } }, () => assert.equal(m.perch({}), ''));
  // A server page that somehow had the variable but no app mark still shows nothing.
  inWindow({ marks: { __KWANT_ISLAND__: { mode: 'busy', shortcut: 'k', available: true } } }, () => assert.equal(m.perch({}), ''));
  assert.equal(m.perchSettings({ __KWANT_APP__: 'mac', __KWANT_ISLAND__: 'busy' }), null);
});

test('Perch section: three modes, the line for the chosen one, the shortcut in this OS’s words', () => {
  inWindow({ marks: { __KWANT_APP__: 'mac', __KWANT_ISLAND__: { mode: 'always', shortcut: 'k', available: true } } }, () => {
    const html = m.perch({});
    assert.match(html, /data-perch-settings/);
    for (const label of ['When something’s happening', 'Always', 'Off']) assert.ok(html.includes(label), label);
    assert.match(html, /aria-checked="true"[^>]*data-perch-mode="always"/);
    assert.match(html, /Stays at the top of the screen/);
    assert.match(html, /⌥⌘K/);
    assert.doesNotMatch(html, /data-perch-wayland/);
  });
  inWindow({ marks: { __KWANT_APP__: 'linux', __KWANT_ISLAND__: { mode: 'off', shortcut: 'space', available: true } } }, () => {
    const html = m.perch({});
    assert.match(html, /Ctrl\+Alt\+K/);
    assert.match(html, /<select[^>]*disabled/, 'shortcut disabled while Off');
  });
});

test('Wayland: the section is disabled and says the perch is not available there', () => {
  inWindow({ marks: { __KWANT_APP__: 'linux', __KWANT_ISLAND__: { mode: 'off', shortcut: 'k', available: false } } }, () => {
    const html = m.perch({});
    assert.match(html, /data-perch-wayland/);
    assert.match(html, /isn’t available on Wayland yet/);
    assert.equal((html.match(/role="radio"[^>]*disabled/g) || []).length, 3);
  });
});

test('settings URLs carry whitelisted values only', () => {
  assert.equal(m.islandUrl('busy', 'k'), 'kwant-app://island?mode=busy&shortcut=k');
  assert.equal(m.islandUrl('off', 'off'), 'kwant-app://island?mode=off&shortcut=off');
  assert.equal(m.islandUrl('always&x=1', 'k'), null);
  assert.equal(m.islandUrl('busy', '⌥⌘K'), null);
  assert.equal(m.islandUrl(undefined, 'k'), null);
  // Unknown values from Rust fall back to the defaults instead of showing nothing chosen.
  assert.deepEqual(m.perchSettings({ __KWANT_APP__: 'mac', __KWANT_ISLAND__: { mode: 'weird', shortcut: 1 } }), { mode: 'busy', shortcut: 'k', available: true });
});
