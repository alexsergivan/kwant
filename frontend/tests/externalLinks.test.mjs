import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

const source = await readFile(new URL('../src/externalLinks.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 },
});
const { OAUTH_RETURN_KEY, DESKTOP_APPS, inDesktopApp, leaveForSignIn, signInChanged, pollUntil, mcpSignIn } = await import(
  `data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`
);

/** A window with sessionStorage and a location that records where it was sent. */
const fakeWindow = (app) => {
  const store = new Map();
  const w = {
    assigned: [],
    sessionStorage: { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, String(v)) },
    location: { assign: (url) => w.assigned.push(url) },
    store,
  };
  if (app !== undefined) w.__KWANT_APP__ = app;
  return w;
};

test('the callback page and the app agree on the return mark', async () => {
  const rust = await readFile(new URL('../../src/server/mcp_oauth.rs', import.meta.url), 'utf8');
  assert.match(rust, new RegExp(`RETURN_KEY: &str = "${OAUTH_RETURN_KEY}"`));
});

test('only the desktop app’s marks count as the desktop app', () => {
  assert.equal(inDesktopApp(fakeWindow('mac')), true);
  assert.equal(inDesktopApp(fakeWindow('linux')), true);
  assert.equal(inDesktopApp(fakeWindow()), false);
  assert.equal(inDesktopApp(fakeWindow('windows')), false);
  assert.equal(inDesktopApp(fakeWindow(true)), false);
  assert.equal(inDesktopApp(fakeWindow('includes')), false);
  assert.equal(inDesktopApp(undefined), false);
});

test('every mark the desktop app sets is one the page knows', async () => {
  const rust = await readFile(new URL('../../apps/mac/src/main.rs', import.meta.url), 'utf8');
  const marks = [...rust.matchAll(/__KWANT_APP__ = '(\w+)'/g)].map((m) => m[1]);
  assert.deepEqual(marks.sort(), ['linux', 'mac']);
  for (const mark of marks) assert.ok(DESKTOP_APPS.includes(mark), mark);
});

test('a browser tab that leaves to sign in is marked so the callback brings it back', () => {
  const w = fakeWindow();
  leaveForSignIn('https://mcp.example/authorize?x=1', w);
  assert.deepEqual(w.assigned, ['https://mcp.example/authorize?x=1']);
  assert.equal(w.store.get(OAUTH_RETURN_KEY), '1');
});

test('in the Mac app the window isn’t marked: the sign-in finishes in the browser, which has no opener', () => {
  const w = fakeWindow('mac');
  leaveForSignIn('https://dash.cloudflare.com/oauth2/auth', w);
  // the app turns this navigation into the default browser; nothing should bring a browser tab to kwant
  assert.deepEqual(w.assigned, ['https://dash.cloudflare.com/oauth2/auth']);
  assert.equal(w.store.has(OAUTH_RETURN_KEY), false);
});

test('storage being off doesn’t stop the sign-in', () => {
  const w = fakeWindow();
  w.sessionStorage.setItem = () => {
    throw new Error('SecurityError');
  };
  leaveForSignIn('https://x.example/', w);
  assert.deepEqual(w.assigned, ['https://x.example/']);
});

test('a sign-in counts as finished only when it is new', () => {
  const off = { connected: false, expires_at: null };
  const on = { connected: true, expires_at: '2026-10-28T10:00:00Z' };
  assert.equal(signInChanged(undefined, on), true);
  assert.equal(signInChanged(off, on), true);
  assert.equal(signInChanged(undefined, undefined), false);
  assert.equal(signInChanged(off, off), false);
  // signing in again over an existing sign-in: same expiry means nothing happened yet
  assert.equal(signInChanged(on, on), false);
  assert.equal(signInChanged(on, { connected: true, expires_at: '2026-10-28T11:00:00Z' }), true);
  assert.equal(signInChanged(on, off), false);
});

test('pollUntil stops when the check says done, and survives failed checks', async () => {
  let calls = 0;
  const done = await new Promise((resolve) => {
    pollUntil(
      async () => {
        calls += 1;
        if (calls === 1) throw new Error('offline');
        return calls >= 3;
      },
      () => resolve('done'),
      { everyMs: 1, forMs: 5000, onTimeout: () => resolve('timeout') },
    );
  });
  assert.equal(done, 'done');
  assert.equal(calls, 3);
});

test('pollUntil gives up after its time and can be stopped', async () => {
  const outcome = await new Promise((resolve) => {
    pollUntil(async () => false, () => resolve('done'), { everyMs: 1, forMs: 10, onTimeout: () => resolve('timeout') });
  });
  assert.equal(outcome, 'timeout');

  let calls = 0;
  const stop = pollUntil(
    async () => {
      calls += 1;
      return false;
    },
    () => assert.fail('stopped'),
    { everyMs: 1 },
  );
  stop();
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(calls, 0);
});

test('mcpSignIn reads one server’s sign-in from the list', async () => {
  const list = [
    { id: 'a', oauth: { connected: false, expires_at: null } },
    { id: 'b', oauth: { connected: true, expires_at: 'x' } },
    { id: 'c' },
  ];
  const fetcher = async (url) => {
    assert.equal(url, '/api/mcp/servers');
    return { ok: true, json: async () => list };
  };
  assert.deepEqual(await mcpSignIn('b', fetcher), { connected: true, expires_at: 'x' });
  assert.equal(await mcpSignIn('c', fetcher), undefined);
  assert.equal(await mcpSignIn('zzz', fetcher), undefined);
  await assert.rejects(mcpSignIn('a', async () => ({ ok: false, status: 503 })), /503/);
});

/** Runs kwnt's callback-page script against a fake browser, the way the page would. */
const runCallback = async (win) => {
  const rust = await readFile(new URL('../../src/server/mcp_oauth.rs', import.meta.url), 'utf8');
  // the script as format! writes it: {{ }} → { }, \" → ", {} slots → the message and the key
  const tpl = rust.match(/<script>(\(function[\s\S]*?)<\/script>/)[1];
  const js = tpl
    .split('\\\n')
    .map((l) => l.trim())
    .join('')
    .replace(/\{\}/, JSON.stringify({ type: 'kwnt-mcp-oauth', server_id: 's1', ok: true }))
    .replace(/\{\}/, JSON.stringify(OAUTH_RETURN_KEY))
    .replaceAll('{{', '{')
    .replaceAll('}}', '}')
    .replaceAll('\\"', '"');
  new Function('window', 'location', 'sessionStorage', js)(win, win.location, win.sessionStorage);
};

const callbackWindow = ({ opener = null, marked = false } = {}) => {
  const store = new Map(marked ? [[OAUTH_RETURN_KEY, '1']] : []);
  const w = {
    opener,
    closed: false,
    close: () => (w.closed = true),
    replaced: [],
    location: { origin: 'http://127.0.0.1:8799', replace: (u) => w.replaced.push(u) },
    sessionStorage: {
      getItem: (k) => store.get(k) ?? null,
      removeItem: (k) => store.delete(k),
    },
    store,
  };
  return w;
};

test('callback page: a popup reports to its opener and closes', async () => {
  const sent = [];
  const w = callbackWindow({ opener: { postMessage: (m, o) => sent.push([m, o]) } });
  await runCallback(w);
  assert.equal(sent.length, 1);
  assert.equal(sent[0][0].ok, true);
  assert.equal(sent[0][1], 'http://127.0.0.1:8799');
  assert.equal(w.closed, true);
  assert.deepEqual(w.replaced, []);
});

test('callback page: a tab that left kwant goes back to it', async () => {
  const w = callbackWindow({ marked: true });
  await runCallback(w);
  assert.deepEqual(w.replaced, ['/']);
  assert.equal(w.store.has(OAUTH_RETURN_KEY), false);
});

test('callback page: the Mac app’s browser tab stays put and doesn’t load kwant', async () => {
  const w = callbackWindow();
  await runCallback(w);
  assert.deepEqual(w.replaced, []);
  assert.equal(w.closed, false);
});
