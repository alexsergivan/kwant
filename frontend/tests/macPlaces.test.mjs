// 0.7.5 side by side (next-075/side-by-side/spec.md P3, P5): the Mac app's bundled pages.
// apps/mac/loading/connect.html is the "Add a server" window; index.html is each place's
// loading / unreachable page. Loaded as they ship, with a fake command bridge, no network.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';

const dir = new URL('../../apps/mac/loading/', import.meta.url);
const page = (name) => readFileSync(new URL(name, dir), 'utf8');
const tick = (ms = 0) => new Promise((r) => setTimeout(r, ms));

// answers: command -> value | (args) => value; a thrown value rejects like a Tauri error.
function load(name, answers = {}, before) {
  const calls = [];
  const dom = new JSDOM(page(name), {
    runScripts: 'dangerously',
    pretendToBeVisual: true,
    beforeParse(win) {
      win.__TAURI_INTERNALS__ = {
        invoke(cmd, args) {
          calls.push([cmd, args]);
          const a = answers[cmd];
          if (a === undefined) return Promise.reject({ code: 'unknown_command' });
          try {
            const v = typeof a === 'function' ? a(args) : a;
            return v && typeof v.then === 'function' ? v : Promise.resolve(v);
          } catch (e) {
            return Promise.reject(e);
          }
        },
      };
      if (before) before(win);
    },
  });
  const doc = dom.window.document;
  return { dom, win: dom.window, doc, calls, $: (id) => doc.getElementById(id) };
}

function type(p, text) {
  const f = p.$('address');
  f.value = text;
  f.dispatchEvent(new p.win.Event('input', { bubbles: true }));
}
function submit(p) {
  p.$('form').dispatchEvent(new p.win.Event('submit', { bubbles: true, cancelable: true }));
}
const statusText = (p) => p.$('status').textContent;
const names = (p) => p.calls.map((c) => c[0]);

const PLACES = { this_mac: true, servers: [], last_origin: null };

test('Add a server: title, Connect button, no restart or switch-back words', async () => {
  const p = load('connect.html', { places_state: PLACES });
  await tick();
  assert.equal(p.doc.title, 'Add a server – kwant');
  assert.equal(p.doc.querySelector('h1').textContent, 'Add a server');
  assert.equal(p.$('go').textContent, 'Connect');
  const text = p.doc.body.textContent;
  for (const gone of ['restart', 'Restart', 'Use kwant on this computer', 'switch back', 'pause']) {
    assert.ok(!text.includes(gone), `no "${gone}"`);
  }
  assert.ok(text.includes('opens in its own window'));
  assert.ok(!p.$('notice-local').hidden, 'This Mac on: says it keeps running');
});

test('the notice drops the This Mac sentence when This Mac is off', async () => {
  const p = load('connect.html', { places_state: { ...PLACES, this_mac: false } });
  await tick();
  assert.ok(p.$('notice-local').hidden);
});

test('local checks answer without a call: https only, no name before @', async () => {
  const p = load('connect.html', { places_state: PLACES });
  await tick();
  type(p, 'http://kwant.example.com');
  submit(p);
  assert.match(statusText(p), /Use an https:\/\/ address/);
  type(p, 'sam@kwant.example.com');
  submit(p);
  assert.match(statusText(p), /Leave out the part before @/);
  assert.deepEqual(names(p).filter((n) => n === 'check_server' || n === 'add_server'), []);
});

test('Connect: check, add, then close the window', async () => {
  const p = load('connect.html', {
    places_state: PLACES,
    check_server: { origin: 'https://kwant.example.com', host: 'kwant.example.com' },
    add_server: { added: true, origin: 'https://kwant.example.com', host: 'kwant.example.com' },
    close_connect: null,
  });
  await tick();
  type(p, 'kwant.example.com');
  submit(p);
  assert.equal(p.$('go').textContent, 'Checking…');
  await tick();
  assert.match(statusText(p), /Added kwant\.example\.com/);
  assert.match(statusText(p), /Opening it in its own window/);
  const add = p.calls.find((c) => c[0] === 'add_server');
  assert.deepEqual({ ...add[1] }, { address: 'kwant.example.com' });
  assert.ok(!names(p).includes('close_connect'), 'holds a moment first');
  await tick(700);
  assert.ok(names(p).includes('close_connect'));
});

test('a server already listed skips the check and says its window is shown', async () => {
  const p = load('connect.html', {
    places_state: { ...PLACES, servers: [{ origin: 'https://kwant.example.com', host: 'kwant.example.com', open: true, reach: 'ok' }] },
    check_server: () => { throw new Error('must not be called'); },
    add_server: { added: false, origin: 'https://kwant.example.com', host: 'kwant.example.com' },
    close_connect: null,
  });
  await tick();
  type(p, 'https://kwant.example.com/');
  submit(p);
  await tick();
  assert.ok(!names(p).includes('check_server'));
  assert.match(statusText(p), /kwant\.example\.com is already in this app/);
  assert.match(statusText(p), /Showing its window/);
  await tick(700);
  assert.ok(!names(p).includes('close_connect'), 'the "already" note stays longer');
  await tick(900);
  assert.ok(names(p).includes('close_connect'));
});

test('add_server answering added:false after a check still reads as already listed', async () => {
  const p = load('connect.html', {
    places_state: PLACES,
    check_server: { host: 'kwant.example.com' },
    add_server: { added: false, origin: 'https://kwant.example.com', host: 'kwant.example.com' },
    close_connect: null,
  });
  await tick();
  type(p, 'kwant.example.com');
  submit(p);
  await tick();
  assert.match(statusText(p), /already in this app/);
});

test('a pasted sign-in link: field shows only the origin, the link goes to check and add, nowhere else', async () => {
  const LINK = 'https://kwant.example.com/#kwnt_token=FAKE-not-a-token';
  const p = load('connect.html', {
    places_state: { ...PLACES, servers: [{ origin: 'https://kwant.example.com', host: 'kwant.example.com' }] },
    check_server: { host: 'kwant.example.com' },
    add_server: { added: false, origin: 'https://kwant.example.com', host: 'kwant.example.com' },
    close_connect: null,
  });
  await tick();
  const ev = new p.win.Event('paste', { bubbles: true, cancelable: true });
  ev.clipboardData = { getData: () => LINK };
  p.$('address').dispatchEvent(ev);
  assert.equal(p.$('address').value, 'https://kwant.example.com');
  assert.ok(!p.$('chip').hidden);
  assert.ok(!p.doc.documentElement.outerHTML.includes('FAKE-not-a-token'));
  submit(p);
  await tick();
  // A listed server with a link still checks, so the window can open the link once.
  assert.deepEqual({ ...p.calls.find((c) => c[0] === 'check_server')[1] }, { address: LINK });
  assert.deepEqual({ ...p.calls.find((c) => c[0] === 'add_server')[1] }, { address: LINK });
  assert.ok(!p.doc.documentElement.outerHTML.includes('FAKE-not-a-token'));
});

test('check errors keep today\'s words and leave the window open', async () => {
  const p = load('connect.html', {
    places_state: PLACES,
    check_server: () => { throw { code: 'dns', host: 'kwant.example.com' }; },
    add_server: () => { throw new Error('must not be called'); },
  });
  await tick();
  type(p, 'kwant.example.com');
  submit(p);
  await tick();
  assert.match(statusText(p), /Couldn't find kwant\.example\.com/);
  assert.equal(p.$('address').getAttribute('aria-invalid'), 'true');
  assert.equal(p.$('go').textContent, 'Connect');
  assert.ok(!names(p).includes('add_server'));
});

test('an add_server failure shows as an error, not as added', async () => {
  const p = load('connect.html', {
    places_state: PLACES,
    check_server: { host: 'kwant.example.com' },
    add_server: () => { throw { code: 'save_failed' }; },
  });
  await tick();
  type(p, 'kwant.example.com');
  submit(p);
  await tick();
  assert.match(statusText(p), /Couldn't add the server/);
  assert.ok(!names(p).includes('close_connect'));
});

test('Cancel and Escape close the Add a server window; Stop ends a check', async () => {
  let release;
  const p = load('connect.html', {
    places_state: PLACES,
    check_server: () => new Promise((r) => { release = r; }),
    close_connect: null,
  });
  await tick();
  type(p, 'kwant.example.com');
  submit(p);
  assert.equal(p.$('cancel').textContent, 'Stop');
  p.$('cancel').click();
  release({ host: 'kwant.example.com' });
  await tick();
  assert.ok(!names(p).includes('add_server'), 'a stopped check never adds');
  assert.equal(p.$('cancel').textContent, 'Cancel');
  p.doc.dispatchEvent(new p.win.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  await tick();
  assert.ok(names(p).includes('close_connect'));
});

test('add_server with an unknown code (open_failed) reads "Couldn\'t add the server."', async () => {
  const p = load('connect.html', {
    places_state: PLACES,
    check_server: { host: 'kwant.example.com' },
    add_server: () => { throw { code: 'open_failed', message: 'x' }; },
  });
  await tick();
  type(p, 'kwant.example.com');
  submit(p);
  await tick();
  assert.match(statusText(p), /Couldn't add the server\./);
  assert.ok(!/didn't answer/.test(statusText(p)));
});

test('a dns failure from add_server keeps its own words', async () => {
  const p = load('connect.html', {
    places_state: PLACES,
    check_server: { host: 'kwant.example.com' },
    add_server: () => { throw { code: 'dns', host: 'kwant.example.com' }; },
  });
  await tick();
  type(p, 'kwant.example.com');
  submit(p);
  await tick();
  assert.match(statusText(p), /Couldn't find kwant\.example\.com/);
});

test('places_state app linux says tray', async () => {
  const p = load('connect.html', { places_state: { ...PLACES, app: 'linux' } });
  await tick();
  assert.equal(p.$('where').textContent, 'tray');
});

test('the Linux build says tray', async () => {
  const p = load('connect.html', { places_state: PLACES, connect_state: { app: 'linux' } });
  await tick();
  assert.equal(p.$('where').textContent, 'tray');
});

// ---- index.html, one per window ----

test('a place window has no switch or change-server buttons', () => {
  const p = load('index.html', {});
  assert.equal(p.$('change'), null);
  assert.equal(p.$('local'), null);
  assert.ok(!p.doc.body.textContent.includes('Use kwant on this computer'));
  assert.ok(!p.doc.body.textContent.includes('paused'));
});

test('a server window: connecting, then unreachable with that host, Try again re-checks this window', async () => {
  const p = load('index.html', {
    connect_state: { mode: 'remote', host: 'kwant.example.com' },
    retry_start: null,
  });
  await tick();
  assert.equal(p.doc.body.dataset.state, 'connecting');
  assert.match(p.doc.querySelector('.connecting').textContent, /Connecting to kwant\.example\.com/);
  p.doc.body.dataset.host = 'kwant.example.com';
  p.doc.body.dataset.code = 'refused';
  p.doc.body.dataset.state = 'unreachable';
  await tick();
  assert.equal(p.$('title').textContent, "Couldn't reach kwant.example.com");
  assert.equal(p.doc.activeElement, p.$('retry'));
  p.$('retry').click();
  assert.equal(p.doc.body.dataset.state, 'connecting');
  assert.deepEqual(p.calls.filter((c) => c[0] === 'retry_start').length, 1);
});

test('the host the app set on the window wins over connect_state', async () => {
  const p = load('index.html', { connect_state: { mode: 'remote', host: 'other.example.com' } });
  p.doc.body.dataset.host = 'kwant.example.com';
  await tick();
  assert.match(p.doc.querySelector('.connecting').textContent, /kwant\.example\.com/);
});

test('This Mac\'s window: failed has Try again, which retries this window only', async () => {
  const p = load('index.html', { connect_state: { mode: 'local' }, retry_start: null });
  await tick();
  assert.equal(p.doc.body.dataset.state ?? '', '');
  p.doc.body.dataset.state = 'failed';
  await tick();
  p.$('retry-local').click();
  assert.equal(p.doc.body.dataset.state, '');
  assert.equal(p.calls.filter((c) => c[0] === 'retry_start').length, 1);
});

test('a failed retry goes back to the failure state', async () => {
  const p = load('index.html', { connect_state: { mode: 'local' } }); // retry_start rejects
  await tick();
  p.doc.body.dataset.state = 'failed';
  await tick();
  p.$('retry-local').click();
  await tick();
  assert.equal(p.doc.body.dataset.state, 'failed');
  assert.equal(p.$('retry-local').disabled, false);
});

test('the pages put nothing in as markup and use no network', () => {
  for (const name of ['connect.html', 'index.html']) {
    const src = page(name);
    assert.ok(!/innerHTML|outerHTML\s*=|insertAdjacentHTML|document\.write/.test(src), `${name}: no markup from data`);
    assert.ok(!/fetch\(|XMLHttpRequest|WebSocket|localStorage|sessionStorage/.test(src), `${name}: no network or storage`);
    assert.ok(!/location\.(hash|search)/.test(src), `${name}: never reads its own URL`);
  }
});
