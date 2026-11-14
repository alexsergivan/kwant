// Space documents (space-docs/design.md, Build split 5): the kwant-doc:// address, the re-check cache, and the
// checks a file gets in the browser before it is sent. Table style.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export * from './src/spaceDocs';
  export { problemText } from './src/components/SpaceDocsTab';
`);

test('parseDocUrl: name, page and quote, as the cite route writes them', () => {
  for (const [href, want, why] of [
    ['kwant-doc://spc_default/faq.md', { space: 'spc_default', file: 'faq.md', page: null, quote: null }, 'plain'],
    ['kwant-doc://shop/plan.pdf#page=2:~:text=Budget%20is%2042%20euros', { space: 'shop', file: 'plan.pdf', page: 2, quote: 'Budget is 42 euros' }, 'PDF page and quote'],
    ['kwant-doc://shop/Returns%20Policy.pdf#page=1', { space: 'shop', file: 'Returns Policy.pdf', page: 1, quote: null }, 'percent-encoded name'],
    ['kwant-doc://spc_default/faq.md#:~:text=return%20any%20item%20within%2030%20days', { space: 'spc_default', file: 'faq.md', page: null, quote: 'return any item within 30 days' }, '.md quote, no page'],
    ['kwant-doc://s/faq.md#page=3:~:text=x', { space: 's', file: 'faq.md', page: null, quote: 'x' }, 'no page for .md, even if the link has one'],
    ['kwant-doc://s/caf%C3%A9%20menu.txt#:~:text=%E2%80%9Cfree%E2%80%9D%2C%2050%25%20off', { space: 's', file: 'café menu.txt', page: null, quote: '“free”, 50% off' }, 'UTF-8 and punctuation'],
    ['kwant-doc://s/notes.markdown', { space: 's', file: 'notes.markdown', page: null, quote: null }, '.markdown'],
  ])
    assert.deepEqual(m.parseDocUrl(href), want, why);
});

test('parseDocUrl: anything else is not a document address', () => {
  for (const [href, why] of [
    ['https://example.com/faq.md', 'another scheme'],
    ['kwant-doc://faq.md', 'no space'],
    ['kwant-doc:///faq.md', 'empty space'],
    ['kwant-doc://s/../kwnt.db', 'a path out'],
    ['kwant-doc://s/sub/faq.md', 'a folder'],
    ['kwant-doc://s/.env.md', 'leading dot'],
    ['kwant-doc://s/photo.png', 'not a kind kwant reads'],
    ['kwant-doc://s/%E0%A4%A.md', 'broken percent-encoding'],
    ['', 'empty'],
    [null, 'none'],
  ])
    assert.equal(m.parseDocUrl(href), null, why);
});

test('citeKey: one key per space, file and quote', () => {
  const k = m.citeKey('shop', 'faq.md', 'thirty days');
  assert.equal(k, m.citeKey('shop', 'faq.md', 'thirty days'));
  for (const other of [m.citeKey('shop2', 'faq.md', 'thirty days'), m.citeKey('shop', 'faq.txt', 'thirty days'), m.citeKey('shop', 'faq.md', 'thirty day')])
    assert.notEqual(k, other);
  // No separator a name or quote could fake.
  assert.notEqual(m.citeKey('a|b', 'c.md', 'd'), m.citeKey('a', 'b|c.md', 'd'));
});

test('recheckQuote: one call per quote, shared; at most 4 at a time; the answer mapped to the chip', async () => {
  m._resetCiteCache();
  const calls = [];
  let running = 0;
  let peak = 0;
  const send = (url, init) => {
    calls.push([url, JSON.parse(init.body)]);
    running++;
    peak = Math.max(peak, running);
    const { quote } = JSON.parse(init.body);
    const status = quote.startsWith('ok') ? 200 : quote.startsWith('gone') ? 404 : quote.startsWith('big') ? 413 : 422;
    return new Promise((r) => setTimeout(() => (running--, r({ status })), 5));
  };
  const a = m.recheckQuote('shop', 'faq.md', 'ok one', send);
  const b = m.recheckQuote('shop', 'faq.md', 'ok one', send);
  assert.equal(a, b, 'the same promise');
  const many = ['ok 2', 'no 3', 'gone 4', 'big 5', 'ok 6', 'ok 7'].map((q) => m.recheckQuote('shop', 'faq.md', q, send));
  const all = await Promise.all([a, ...many]);
  assert.deepEqual(all, ['ok', 'ok', 'missing', 'gone', 'unknown', 'ok', 'ok']);
  assert.equal(calls.length, 7);
  assert.ok(peak <= 4, `peak ${peak}`);
  assert.equal(calls[0][0], '/api/spaces/shop/docs/cite');
  assert.deepEqual(calls[0][1], { file: 'faq.md', quote: 'ok one' });
  await m.recheckQuote('shop', 'faq.md', 'ok one', send);
  assert.equal(calls.length, 7, 'cached for the session');
});

test('recheckQuote: a network error shows no mark and may try again later', async () => {
  m._resetCiteCache();
  let n = 0;
  const send = () => (n++ === 0 ? Promise.reject(new Error('offline')) : Promise.resolve({ status: 200 }));
  assert.equal(await m.recheckQuote('s', 'a.md', 'q', send), 'unknown');
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(await m.recheckQuote('s', 'a.md', 'q', send), 'ok');
});

test('checkFile: name, then kind, then size, before anything is sent (QA F2 included)', () => {
  const MB = 1024 * 1024;
  for (const [name, size, want, why] of [
    ['faq.md', 10, null, 'fine'],
    ['Returns policy.PDF', MB, null, 'upper-case extension'],
    ['notes.markdown', 10, null, '.markdown'],
    ['prices.txt', 2 * MB, null, 'exactly 2 MB'],
    ['big.pdf', 2 * MB + 1, 'too_large', 'over 2 MB'],
    ['Staff handbook.docx', 10, 'wrong_kind', 'docx'],
    ['README', 10, 'wrong_kind', 'no extension'],
    ['photo.png', 10, 'wrong_kind', 'png'],
    ['.env.md', 10, 'bad_name', 'leading dot'],
    ['..md', 10, 'bad_name', '".." (F2)'],
    ['a..b.md', 10, 'bad_name', '".." inside (F2)'],
    ['../secret.md', 10, 'bad_name', 'path out'],
    ['sub/faq.md', 10, 'bad_name', 'slash'],
    ['sub\\faq.md', 10, 'bad_name', 'backslash'],
    [' faq.md', 10, 'bad_name', 'padded'],
    ['.hidden.docx', 9 * MB, 'bad_name', 'name is checked first'],
    ['huge.docx', 9 * MB, 'wrong_kind', 'kind before size'],
  ])
    assert.equal(m.checkFile({ name, size })?.kind ?? null, want, why);
});

test('words: sizes, meta line, marks, folder and problem rows', () => {
  const now = Date.parse('2026-11-13T12:00:00Z');
  assert.equal(m.formatSize(64), '1 KB');
  assert.equal(m.formatSize(312 * 1024), '312 KB');
  assert.equal(m.formatSize(6.4 * 1024 * 1024), '6.4 MB');
  assert.equal(m.docMeta({ name: 'Returns policy.pdf', size: 312 * 1024, modified: '2026-11-10T09:00:00Z', pages: 4 }, now), '4 pages · 312 KB · updated 3 days ago');
  assert.equal(m.docMeta({ name: 'faq.md', size: 18 * 1024, modified: '2026-11-13T08:00:00Z' }, now), '18 KB · updated today');
  assert.equal(m.docMeta({ name: 'one.pdf', size: 2048, modified: '2026-11-12T08:00:00Z', pages: 1 }, now), '1 page · 2 KB · updated yesterday');
  assert.equal(m.updatedWhen('2026-10-28T08:00:00Z', now), 'Oct 28');
  assert.deepEqual(['a.pdf', 'b.MD', 'c.markdown', 'd.txt', 'e.docx', 'f'].map(m.markOf), ['PDF', 'MD', 'MD', 'TXT', 'DOCX', 'FILE']);
  assert.equal(m.shortFolder('/home/me/.kwnt/docs'), '~/.kwnt/docs');
  assert.equal(m.shortFolder('/Users/ana/Library/kwnt/spaces/7f3a9c2e1b4d/docs'), '~/Library/kwnt/spaces/7f3a9c2e…/docs');
  assert.equal(m.shortFolder('/home/me/.kwnt/spaces/spc_7f3a9c2e1b4d/docs'), '~/.kwnt/spaces/spc_7f3a…/docs', 'a real space id keeps 8 characters');
  assert.equal(m.shortFolder('/srv/kwnt/spaces/shop/docs'), '/srv/kwnt/spaces/shop/docs', 'a short id stays');
  assert.deepEqual(
    m.sortDocs([
      { name: 'old.md', modified: '2026-11-01T00:00:00Z' },
      { name: 'new.md', modified: '2026-11-13T00:00:00Z' },
    ]).map((d) => d.name),
    ['new.md', 'old.md'],
    'newest first',
  );
  assert.equal(m.problemText('too_large', { name: 'big.pdf', size: 6.4 * 1024 * 1024 }), 'It’s 6.4 MB, over the 2 MB bots can read. Save a smaller copy or split it.');
  assert.match(m.problemText('bad_name', { name: '..md', size: 1 }), /can’t start with a dot or contain “\.\.”/);
  assert.equal(m.copyQuote('Budget is 42 euros', 'plan.pdf', 2), 'Budget is 42 euros — plan.pdf, p. 2');
  assert.equal(m.copyQuote('thirty days', 'faq.md', null), 'thirty days — faq.md');
});

test('add: the error code decides the row, never the status or the words', () => {
  for (const [status, code, error, kind] of [
    [400, 'bad_name', 'names can’t start with a dot', 'bad_name'],
    [415, 'wrong_kind', 'wrong kind', 'wrong_kind'],
    [413, 'too_large', 'kwant reads documents up to 2 MB', 'too_large'],
    [413, '', '', 'too_large'],
    [422, 'no_text', 'PDF has no text layer', 'no_text'],
    [422, 'unreadable', 'faq.md isn’t UTF-8 text', 'unreadable'],
    [409, 'exists', 'almost.md is already in this space’s documents', 'exists'],
    [409, 'exists', 'faq200.md is already in this space’s documents; the most', 'exists'],
    [409, 'full', 'this space has 200 documents, the most kwant reads', 'limit'],
    [409, 'docs_link', 'the space’s docs isn’t a plain folder', 'docs_link'],
    [409, '', 'this space is archived; restore it first', 'archived'],
    [403, 'person_only', 'only the person', 'person_only'],
    [400, 'interrupted', 'The upload was interrupted.', 'failed'],
    [422, 'something_new', 'already exists', 'failed'],
    [500, '', '', 'failed'],
  ])
    assert.equal(m.addResultOf(status, code, error).kind, kind, `${status} ${code} ${error}`);
  for (const kind of ['bad_name', 'wrong_kind', 'no_text', 'unreadable', 'docs_link', 'limit', 'archived'])
    assert.notEqual(m.problemText(kind, { name: 'a.md', size: 1 }), m.problemText('failed', { name: 'a.md', size: 1 }), kind);
});

test('add: the file is the raw body and its name is in the query', async () => {
  const real = globalThis.fetch;
  const calls = [];
  const reply = (status, body) => async (url, init) => {
    calls.push({ url, init });
    return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
  };
  try {
    const file = new File(['return any item within 30 days'], 'Returns & FAQ #2.md', { type: 'text/markdown' });
    globalThis.fetch = reply(201, { name: 'Returns & FAQ #2.md', size: 30, modified: '2026-11-14T00:00:00Z' });
    const ok = await m.spaceDocsApi.add('spc 1', file);
    assert.equal(ok.ok, true);
    assert.equal(calls[0].url, '/api/spaces/spc%201/docs?name=Returns%20%26%20FAQ%20%232.md');
    assert.equal(calls[0].init.method, 'POST');
    assert.equal(calls[0].init.body, file, 'the File itself, not FormData');
    assert.ok(!(calls[0].init.body instanceof FormData));
    assert.equal(calls[0].init.headers, undefined, 'no Content-Type override');

    globalThis.fetch = reply(201, { name: 'faq.md', size: 1, modified: '2026-11-14T00:00:00Z' });
    await m.spaceDocsApi.add('s', new File(['x'], 'faq.md'), true);
    assert.equal(calls[1].url, '/api/spaces/s/docs?name=faq.md&replace=1');

    globalThis.fetch = reply(409, { error: 'almost.md is already in this space’s documents', code: 'exists' });
    assert.equal((await m.spaceDocsApi.add('s', new File(['x'], 'almost.md'))).kind, 'exists');
    globalThis.fetch = reply(409, { error: 'dir.md is a folder', code: 'exists' });
    assert.equal((await m.spaceDocsApi.add('s', new File(['x'], 'dir.md'), true)).kind, 'failed', 'replace refused: no second clash row');
    globalThis.fetch = reply(422, { error: 'x isn’t UTF-8 text', code: 'unreadable' });
    assert.equal((await m.spaceDocsApi.add('s', new File(['x'], 'x.txt'))).kind, 'unreadable');
  } finally {
    globalThis.fetch = real;
  }
});

test('routes probe and page text: a missing route is told apart from a missing file', async () => {
  const real = globalThis.fetch;
  const answer = (status, body) => async () => new Response(body === undefined ? 'Not Found' : JSON.stringify(body), { status, headers: body === undefined ? {} : { 'Content-Type': 'application/json' } });
  try {
    for (const [status, body, routes, page, why] of [
      [400, { error: 'file is required' }, true, 'failed', 'route there, empty name refused'],
      [404, undefined, false, 'no_route', 'no route yet: plain 404'],
      [405, undefined, false, 'no_route', 'no route yet: 405'],
      [404, { error: 'no such document' }, true, 'gone', 'route there, file gone'],
      [500, { error: 'boom' }, true, 'failed', 'server error'],
    ]) {
      globalThis.fetch = answer(status, body);
      assert.equal(await m.spaceDocsApi.hasRoutes('s'), routes, `hasRoutes: ${why}`);
      assert.equal(await m.spaceDocsApi.page('s', 'faq.md', 1, 'x'), page, `page: ${why}`);
    }
    const doc = { file: 'a.pdf', page: 2, pages: 4, text_before: 'a ', match: 'b', text_after: ' c', cut_before: false, cut_after: false };
    globalThis.fetch = answer(200, doc);
    assert.deepEqual(await m.spaceDocsApi.page('s', 'a.pdf', 2, 'b'), doc);
    globalThis.fetch = async () => { throw new TypeError('offline'); };
    assert.equal(await m.spaceDocsApi.hasRoutes('s'), false);
    assert.equal(await m.spaceDocsApi.page('s', 'a.pdf', 2), 'failed');
  } finally {
    globalThis.fetch = real;
  }
});
