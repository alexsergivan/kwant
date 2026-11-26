// santa-gone-page (autonomy/santa-gone-page/brief.md): a deleted, expired or unknown /p/<code> gets the guest bundle with a
// boot of exactly {"gone":true}. The entry (guest/src/main.ts) draws one static, kit-neutral ended screen and stops:
// no /info fetch, no socket, no wake listeners.
import './pinLocale.mjs'; // en-US default, whatever the machine's locale
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { build } from 'esbuild';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));

/** guest/src/main.ts as the page runs it (its css import dropped), as importable source. */
async function mainSource() {
  const out = await build({
    entryPoints: [`${root}guest/src/main.ts`],
    bundle: true,
    write: false,
    format: 'esm',
    platform: 'browser',
    logLevel: 'silent',
    loader: { '.css': 'empty' },
  });
  assert.equal(out.outputFiles.length, 1);
  return out.outputFiles[0].text;
}

const src = await mainSource();
const settle = async () => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};

/** Boots main.ts in a fresh page whose boot slot holds `bootText`; records every fetch, socket and window listener. */
async function bootMain(bootText, url = 'http://localhost/p/abcdef') {
  const dom = new JSDOM(
    `<!doctype html><html><head><title>Join the quiz</title></head><body><div id="app"></div><script type="application/json" id="pocket-boot">${bootText}</script></body></html>`,
    { url, pretendToBeVisual: true },
  );
  const w = dom.window;
  const seen = { fetch: [], sockets: [], listeners: [] };
  w.fetch = async (u) => {
    seen.fetch.push(String(u));
    return { ok: false, status: 404, json: async () => ({}) };
  };
  w.WebSocket = class {
    constructor(u) {
      seen.sockets.push(String(u));
    }
    close() {}
  };
  const addW = w.addEventListener.bind(w);
  w.addEventListener = (type, ...rest) => (seen.listeners.push(`window:${type}`), addW(type, ...rest));
  const addD = w.document.addEventListener.bind(w.document);
  w.document.addEventListener = (type, ...rest) => (seen.listeners.push(`document:${type}`), addD(type, ...rest));
  const g = globalThis;
  for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'SVGElement', 'localStorage', 'location']) {
    Object.defineProperty(g, k, { value: k === 'window' ? w : w[k], configurable: true, writable: true });
  }
  // A fresh module instance per boot (main.ts runs on import)
  await import(`data:text/javascript;base64,${Buffer.from(`${src}\n// ${Math.random()}`).toString('base64')}`);
  await settle();
  return { w, seen, app: w.document.getElementById('app') };
}

test('a gone boot draws the ended screen, kit-neutral, and asks nothing', async () => {
  const { w, seen, app } = await bootMain('{"gone":true}');
  const main = app.querySelector('main');
  assert.equal(main?.getAttribute('data-screen'), 'gone');
  assert.equal(app.querySelector('h2')?.textContent, 'This page has ended');
  assert.equal(app.querySelector('p.lead')?.textContent, 'Everything in it was deleted. Thanks for taking\u00a0part!', 'a no-break space before "part!"');
  assert.equal(app.querySelector('.body.mid p.small')?.textContent, 'Expected something here? Check the link with whoever sent it.');
  assert.equal(w.document.title, 'This page has ended', 'not "Join the quiz"');
  assert.ok(app.querySelector('.body.mid .qmwrap [role="img"]'), 'the creature, like the other message screens');
  const made = app.querySelector('footer.madewith a');
  assert.equal(made?.getAttribute('href'), 'https://kwant.si');
  assert.doesNotMatch(app.textContent, /pocket|quiz|santa|wall|desk|queue|gather/i, 'no kit, no pocket-app word');
  assert.deepEqual(seen.fetch, [], 'no /info or state fetch');
  assert.deepEqual(seen.sockets, [], 'no socket');
  assert.deepEqual(seen.listeners.filter((l) => /visibilitychange|online|pageshow|resize/.test(l)), [], 'no wake listeners');
  assert.equal(app.querySelectorAll('button, input, form').length, 0, 'nothing to press');
});

test('a gone boot wins over a screen or remote link, and still wipes its token', async () => {
  for (const hash of ['#screen=abc123', '#remote=abc123']) {
    const { w, seen, app } = await bootMain('{"gone":true}', `http://localhost/p/abcdef${hash}`);
    assert.equal(app.querySelector('main')?.getAttribute('data-screen'), 'gone', hash);
    assert.equal(w.location.hash, '', `${hash}: wiped from the address bar`);
    assert.deepEqual([seen.fetch, seen.sockets], [[], []], hash);
  }
});

test('only exactly {"gone": true} is the gone page', async () => {
  for (const boot of ['{"gone":"true"}', '{"gone":1}', 'null']) {
    const { app, seen } = await bootMain(boot);
    assert.notEqual(app.querySelector('main')?.getAttribute('data-screen'), 'gone', boot);
    assert.ok(seen.fetch.length > 0, `${boot}: the page goes on to ask the box`);
  }
});
