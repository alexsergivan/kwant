// 0.7.5 side by side (next-075/side-by-side/spec.md P3, P5; design.md §3, §6): the Mac app's bundled pages.
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
function paste(p, text) {
  const ev = new p.win.Event('paste', { bubbles: true, cancelable: true });
  ev.clipboardData = { getData: () => text };
  p.$('address').dispatchEvent(ev);
}

const SERVER = { origin: 'https://kwant.example.com', host: 'kwant.example.com', open: true, reach: 'connected' };
// places_state as Lin's e35143d answers it (apps/mac/src/connect.rs)
const PLACES = { this_mac: true, servers: [], last_origin: null, place: null, local_kwant: 'app', started_kwant: true, app: 'mac', version: '0.7.5' };
const LISTED = { ...PLACES, servers: [SERVER], last_origin: SERVER.origin };
const SERVER_WINDOW = { ...LISTED, place: { key: SERVER.origin, kind: 'server', host: SERVER.host } };
const MAC_WINDOW = { ...LISTED, place: { key: 'this_mac', kind: 'this_mac', host: null } };

// ---- connect.html, the Add a server window (design.md §3) ----

test('Add a server: Mira §3.2 words, Connect button, no notice and no restart or switch-back words', async () => {
  const p = load('connect.html', { places_state: PLACES });
  await tick();
  assert.equal(p.doc.title, 'Add a server – kwant');
  assert.equal(p.doc.querySelector('h1').textContent, 'Add a server');
  assert.equal(p.doc.querySelector('.lead').textContent, 'Open a kwant that runs on another machine, such as your VPS. It gets a window of its own.');
  assert.equal(p.doc.querySelector('label').textContent, 'Server address');
  assert.equal(p.$('address').placeholder, 'kwant.example.com');
  assert.equal(p.$('help1').textContent, 'The address you open kwant at in a browser.');
  assert.equal(p.$('help2').textContent, "Or paste a one-time sign-in link (in kwant: Settings → Signed-in devices → Add a phone or browser). It signs this app in once and isn't kept.");
  assert.equal(p.$('go').textContent, 'Connect');
  assert.equal(p.$('cancel').textContent, 'Cancel');
  assert.equal(p.doc.querySelector('.notice'), null, 'the notice card is gone');
  const text = p.doc.querySelector('main').textContent;
  for (const gone of ['restart', 'Restart', 'Use kwant on this computer', 'switch back', 'pause']) {
    assert.ok(!text.includes(gone), `no "${gone}"`);
  }
  assert.ok(!names(p).includes('connect_state'), 'connect_state is gone');
});

test('local checks answer without a call: https only, no name before @', async () => {
  const p = load('connect.html', { places_state: PLACES });
  await tick();
  type(p, 'http://kwant.example.com');
  submit(p);
  assert.match(statusText(p), /Use an https:\/\/ address/);
  assert.match(statusText(p), /http:\/\/ works only for this computer \(localhost or 127\.0\.0\.1\), for example through an SSH tunnel\./);
  type(p, 'sam@kwant.example.com');
  submit(p);
  assert.match(statusText(p), /Leave out the part before @/);
  assert.deepEqual(names(p).filter((n) => n === 'check_server' || n === 'add_server'), []);
});

test('Connect: check, add, then close the window at once (no hold)', async () => {
  const p = load('connect.html', {
    places_state: PLACES,
    check_server: { origin: SERVER.origin, host: SERVER.host },
    add_server: { added: true, origin: SERVER.origin, host: SERVER.host },
    close_connect: null,
  });
  await tick();
  type(p, 'kwant.example.com');
  assert.match(p.$('preview').textContent, /Connects to https:\/\/kwant\.example\.com/);
  submit(p);
  assert.equal(p.$('go').textContent, 'Checking…');
  assert.match(statusText(p), /Checking kwant\.example\.com…/);
  assert.equal(p.$('cancel').textContent, 'Stop');
  await tick();
  const add = p.calls.find((c) => c[0] === 'add_server');
  assert.deepEqual({ ...add[1] }, { address: 'kwant.example.com' });
  assert.ok(names(p).includes('close_connect'), 'closes straight after add_server');
  assert.ok(!/Restarting|Added/.test(p.doc.querySelector('main').textContent));
});

test('a listed server: "already in this app" as you type, with no call, and the button reads Show it', async () => {
  const p = load('connect.html', {
    places_state: LISTED,
    check_server: () => { throw new Error('must not be called'); },
    add_server: { added: false, origin: SERVER.origin, host: SERVER.host },
    close_connect: null,
  });
  await tick();
  type(p, 'https://kwant.example.com/');
  assert.equal(statusText(p), 'kwant.example.com is already in this app.');
  assert.ok(p.doc.querySelector('#status .status.info'));
  assert.equal(p.$('go').textContent, 'Show it');
  assert.deepEqual(names(p), ['places_state'], 'decided locally');
  type(p, 'other.example.com');
  assert.equal(statusText(p), '', 'the line goes when the address changes');
  assert.equal(p.$('go').textContent, 'Connect');
  type(p, 'kwant.example.com');
  submit(p);
  await tick();
  assert.ok(!names(p).includes('check_server'));
  assert.deepEqual({ ...p.calls.find((c) => c[0] === 'add_server')[1] }, { address: 'kwant.example.com' });
  assert.ok(names(p).includes('close_connect'));
});

test('add_server answering added:false after a check closes like any add', async () => {
  const p = load('connect.html', {
    places_state: PLACES,
    check_server: { host: SERVER.host },
    add_server: { added: false, origin: SERVER.origin, host: SERVER.host },
    close_connect: null,
  });
  await tick();
  type(p, 'kwant.example.com');
  submit(p);
  await tick();
  assert.ok(names(p).includes('close_connect'));
});

test('a pasted sign-in link: field shows only the origin, the link goes to check and add, nowhere else', async () => {
  const LINK = 'https://kwant.example.com/#kwnt_token=FAKE-not-a-token';
  const p = load('connect.html', {
    places_state: PLACES,
    check_server: { host: SERVER.host },
    add_server: { added: true, origin: SERVER.origin, host: SERVER.host },
    close_connect: null,
  });
  await tick();
  paste(p, LINK);
  assert.equal(p.$('address').value, 'https://kwant.example.com');
  assert.ok(!p.$('chip').hidden);
  assert.equal(p.$('chip-text').textContent, "Sign-in link added. It signs this app in once when you connect, then it's dropped. It isn't saved.");
  assert.ok(!p.doc.documentElement.outerHTML.includes('FAKE-not-a-token'));
  submit(p);
  await tick();
  assert.deepEqual({ ...p.calls.find((c) => c[0] === 'check_server')[1] }, { address: LINK });
  assert.deepEqual({ ...p.calls.find((c) => c[0] === 'add_server')[1] }, { address: LINK });
  assert.ok(!p.doc.documentElement.outerHTML.includes('FAKE-not-a-token'));
});

test('a listed server with a pasted link: already-link line, button stays Connect, the link is sent', async () => {
  const LINK = 'https://kwant.example.com/#kwnt_token=FAKE-not-a-token';
  const p = load('connect.html', {
    places_state: LISTED,
    check_server: { host: SERVER.host },
    add_server: { added: false, origin: SERVER.origin, host: SERVER.host },
    close_connect: null,
  });
  await tick();
  paste(p, LINK);
  assert.equal(statusText(p), 'kwant.example.com is already in this app.Connect signs its window in again with the link.');
  assert.equal(p.$('go').textContent, 'Connect');
  submit(p);
  await tick();
  assert.deepEqual({ ...p.calls.find((c) => c[0] === 'add_server')[1] }, { address: LINK });
});

test('check errors keep today\'s words and leave the window open', async () => {
  const p = load('connect.html', {
    places_state: PLACES,
    check_server: () => { throw { code: 'dns', host: SERVER.host }; },
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

test('save_failed from add_server reads Mira\'s sentence and keeps the window', async () => {
  const p = load('connect.html', {
    places_state: PLACES,
    check_server: { host: SERVER.host },
    add_server: () => { throw { code: 'save_failed', host: SERVER.host, message: 'from the app' }; },
  });
  await tick();
  type(p, 'kwant.example.com');
  submit(p);
  await tick();
  assert.equal(statusText(p), "kwant couldn't save the server. Check that ~/.kwnt can be written.");
  assert.ok(!names(p).includes('close_connect'));
});

test('any other add_server code reads "Couldn\'t add the server."', async () => {
  const p = load('connect.html', {
    places_state: PLACES,
    check_server: { host: SERVER.host },
    add_server: () => { throw { code: 'not_allowed', message: 'x' }; },
  });
  await tick();
  type(p, 'kwant.example.com');
  submit(p);
  await tick();
  assert.match(statusText(p), /^Couldn't add the server\./);
});

test('a dns failure from add_server keeps its own words', async () => {
  const p = load('connect.html', {
    places_state: PLACES,
    check_server: { host: SERVER.host },
    add_server: () => { throw { code: 'dns', host: SERVER.host }; },
  });
  await tick();
  type(p, 'kwant.example.com');
  submit(p);
  await tick();
  assert.match(statusText(p), /Couldn't find kwant\.example\.com/);
});

test('if close_connect fails the form comes back empty', async () => {
  const p = load('connect.html', {
    places_state: PLACES,
    check_server: { host: SERVER.host },
    add_server: { added: true, origin: SERVER.origin, host: SERVER.host },
    close_connect: () => { throw { code: 'not_allowed' }; },
  });
  await tick();
  type(p, 'kwant.example.com');
  submit(p);
  await tick();
  await tick();
  assert.equal(p.$('address').value, '');
  assert.equal(p.$('address').readOnly, false);
  assert.equal(p.$('cancel').hidden, false);
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
  release({ host: SERVER.host });
  await tick();
  assert.ok(!names(p).includes('add_server'), 'a stopped check never adds');
  assert.equal(p.$('cancel').textContent, 'Cancel');
  p.doc.dispatchEvent(new p.win.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  await tick();
  assert.ok(names(p).includes('close_connect'));
});

// ---- index.html, one per window (design.md §6) ----

test('the failure page: Try again then Remove this server…, nothing removed by design', async () => {
  const p = load('index.html', { places_state: SERVER_WINDOW });
  await tick();
  assert.equal(p.$('change'), null);
  assert.equal(p.$('local'), null);
  const text = [...p.doc.body.children].filter((e) => e.tagName !== 'SCRIPT').map((e) => e.textContent).join(' ');
  for (const gone of ['Change server', 'Use kwant on this computer', 'paused', 'Your data on this computer']) {
    assert.ok(!text.includes(gone), `no "${gone}"`);
  }
  const buttons = [...p.doc.querySelectorAll('.unreachable .actions button')];
  assert.deepEqual(buttons.map((b) => b.textContent), ['Try again', 'Remove this server…']);
  assert.ok(buttons[0].classList.contains('primary'));
  assert.ok(buttons[1].classList.contains('secondary'));
  assert.match(p.doc.querySelector('.unreachable .logline').textContent, /Details are in ~\/\.kwnt\/mac-app\.log/);
});

test('a server window: connecting, then unreachable with that host, Try again is focused and re-checks this window', async () => {
  const p = load('index.html', { places_state: SERVER_WINDOW, retry_start: null });
  await tick();
  assert.equal(p.doc.body.dataset.state, 'connecting');
  assert.match(p.doc.querySelector('.connecting').textContent, /Connecting to kwant\.example\.com…/);
  p.doc.body.dataset.host = 'kwant.example.com';
  p.doc.body.dataset.code = 'refused';
  p.doc.body.dataset.state = 'unreachable';
  await tick();
  assert.equal(p.$('title').textContent, "Couldn't reach kwant.example.com");
  assert.equal(p.$('why').textContent, "The machine is there, but kwant isn't answering. It may be restarting or stopped.");
  assert.equal(p.doc.activeElement, p.$('retry'));
  p.$('retry').click();
  assert.equal(p.doc.body.dataset.state, 'connecting');
  assert.equal(p.calls.filter((c) => c[0] === 'retry_start').length, 1);
});

test('Remove this server… calls remove_server with this window\'s origin; cancelled comes back', async () => {
  let answer = { removed: false };
  const p = load('index.html', { places_state: SERVER_WINDOW, remove_server: () => answer });
  await tick();
  p.doc.body.dataset.host = 'kwant.example.com';
  p.doc.body.dataset.code = 'timeout';
  p.doc.body.dataset.state = 'unreachable';
  await tick();
  assert.equal(p.$('remove').hidden, false);
  p.$('remove').click();
  assert.equal(p.$('retry').disabled, true, 'busy while the app asks');
  await tick();
  assert.deepEqual({ ...p.calls.find((c) => c[0] === 'remove_server')[1] }, { origin: 'https://kwant.example.com' });
  assert.equal(p.$('retry').disabled, false);
  assert.equal(p.$('remove').disabled, false);
  answer = { removed: true };
  p.$('remove').click();
  await tick();
  assert.equal(p.$('remove').disabled, true, 'removed: the app closes this window');
});

test('Remove this server… stays hidden when the window has no server place', async () => {
  const p = load('index.html', { places_state: MAC_WINDOW });
  await tick();
  assert.equal(p.$('remove').hidden, true);
  assert.equal(p.doc.body.dataset.state ?? '', '');
});

test('the host the app set on the window wins over places_state', async () => {
  const p = load('index.html', { places_state: { ...SERVER_WINDOW, place: { ...SERVER_WINDOW.place, host: 'other.example.com' } } });
  p.doc.body.dataset.host = 'kwant.example.com';
  await tick();
  assert.match(p.doc.querySelector('.connecting').textContent, /kwant\.example\.com/);
});

test('Linux: the log path reads desktop-app.log', async () => {
  const p = load('index.html', { places_state: { ...SERVER_WINDOW, app: 'linux' } });
  await tick();
  assert.match(p.doc.querySelector('.unreachable .logline').textContent, /~\/\.kwnt\/desktop-app\.log/);
});

test('This Mac\'s window: today\'s two lines; failed has Try again for this window only', async () => {
  const p = load('index.html', { places_state: MAC_WINDOW, retry_start: null });
  await tick();
  assert.equal(p.doc.querySelector('.starting').textContent, 'Starting kwant…');
  p.doc.body.dataset.state = 'failed';
  await tick();
  assert.match(p.doc.querySelector('.failed p').textContent, /^kwant didn't start\.Details are in ~\/\.kwnt\/mac-app\.log$/);
  p.$('retry-local').click();
  assert.equal(p.doc.body.dataset.state, '');
  assert.equal(p.calls.filter((c) => c[0] === 'retry_start').length, 1);
});

test('a failed retry goes back to the failure state', async () => {
  const p = load('index.html', { places_state: MAC_WINDOW }); // retry_start rejects
  await tick();
  p.doc.body.dataset.state = 'failed';
  await tick();
  p.$('retry-local').click();
  await tick();
  assert.equal(p.doc.body.dataset.state, 'failed');
  assert.equal(p.$('retry-local').disabled, false);
});

test('the pages call only Lin\'s commands; set_this_mac (last_place, save_failed) stays menu-only', () => {
  const allowed = new Set(['places_state', 'check_server', 'add_server', 'close_connect', 'remove_server', 'retry_start']);
  for (const name of ['connect.html', 'index.html']) {
    const used = [...page(name).matchAll(/invoke\('([a-z_]+)'/g)].map((m) => m[1]);
    for (const cmd of used) assert.ok(allowed.has(cmd), `${name} calls ${cmd}`);
    assert.ok(!used.includes('connect_state') && !used.includes('set_this_mac') && !used.includes('use_this_computer') && !used.includes('connect_server'));
  }
});

test('the pages put nothing in as markup and use no network', () => {
  for (const name of ['connect.html', 'index.html']) {
    const src = page(name);
    assert.ok(!/innerHTML|outerHTML\s*=|insertAdjacentHTML|document\.write/.test(src), `${name}: no markup from data`);
    assert.ok(!/fetch\(|XMLHttpRequest|WebSocket|localStorage|sessionStorage/.test(src), `${name}: no network or storage`);
    assert.ok(!/location\.(hash|search)/.test(src), `${name}: never reads its own URL`);
  }
});
