import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

const load = async (path) => {
  const source = await readFile(new URL(path, import.meta.url), 'utf8');
  const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } });
  return import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
};

const c = await load('../src/continuity.ts');

/** A fake server holding the `timezone` setting, with the server's own no-overwrite rule. */
function server(initial, { getStatus = 200, field = true } = {}) {
  const s = { timezone: initial, posts: [] };
  s.fetch = async (url, init) => {
    assert.equal(url, '/api/settings');
    if (!init || !init.method || init.method === 'GET') {
      const body = field ? { timezone: s.timezone, location: '' } : { agent_tools: true };
      return new Response(JSON.stringify(body), { status: getStatus });
    }
    const body = JSON.parse(init.body);
    s.posts.push(body);
    if (s.timezone.trim() === '' && body.timezone_default) s.timezone = body.timezone_default;
    return new Response('{"status":"saved"}', { status: 200 });
  };
  return s;
}

test('an empty time zone is filled from the browser, through timezone_default', async () => {
  const s = server('');
  assert.equal(await c.fillTimeZone(s.fetch, 'Europe/Ljubljana'), 'saved');
  assert.deepEqual(s.posts, [{ timezone_default: 'Europe/Ljubljana' }], 'never the plain timezone field, which overwrites');
  assert.equal(s.timezone, 'Europe/Ljubljana');
});

test('a time zone that is already set is never overwritten', async () => {
  for (const set of ['Europe/Berlin', 'UTC']) {
    const s = server(set);
    assert.equal(await c.fillTimeZone(s.fetch, 'America/New_York'), 'kept');
    assert.deepEqual(s.posts, [], `no write at all when "${set}" is set`);
    assert.equal(s.timezone, set);
  }
});

test('blank counts as unset; failures and old servers write nothing and never throw', async () => {
  const blank = server('   ');
  assert.equal(await c.fillTimeZone(blank.fetch, 'Asia/Tokyo'), 'saved');

  const old = server('', { field: false });
  assert.equal(await c.fillTimeZone(old.fetch, 'Asia/Tokyo'), 'skipped');
  assert.deepEqual(old.posts, []);

  const locked = server('', { getStatus: 401 });
  assert.equal(await c.fillTimeZone(locked.fetch, 'Asia/Tokyo'), 'skipped');
  assert.deepEqual(locked.posts, []);

  const offline = async () => {
    throw new TypeError('Failed to fetch');
  };
  assert.equal(await c.fillTimeZone(offline, 'Asia/Tokyo'), 'skipped');
});

test('the default zone is the browser one', async () => {
  const s = server('');
  await c.fillTimeZone(s.fetch);
  assert.equal(s.posts[0].timezone_default, c.defaultTimeZone());
});
