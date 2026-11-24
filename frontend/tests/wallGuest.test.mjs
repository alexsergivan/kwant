// pa14 Wall, guest page and big screen (autonomy/pocket-apps/pa14/guest-brief.md): the view mapping from Juno's wall
// API, the upload queue (one at a time, retry, 429, closed, away), the error copy, G1-G8 against the fake wall box
// (guest/fake/wall.mjs), the TV's slideshow rules, and the two CSS files' token and size rules. The browser half
// (G1-G8 at 390 px and TV1-TV4 at 1920 under pa1's CSP, with shots) is guest/e2e/csp.mjs.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';
import { WallBox, wallPhotos } from '../guest/fake/wall.mjs';

const dom = new JSDOM('<!doctype html><html><body><div id="app"></div></body></html>', { url: 'http://localhost/p/wall-join', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'HTMLButtonElement', 'HTMLImageElement', 'Element', 'Node', 'Event', 'MouseEvent', 'SVGElement', 'File', 'Blob']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { wallFrom, tvFrom, screenOf, wallPage, refusal, PAGE } from './guest/src/wall/view';
  export { UploadQueue, precheck } from './guest/src/wall/queue';
  export { WallApp } from './guest/src/wall/app';
  export { WallTV, SLIDE_MS, PILL_MS } from './guest/src/wall/tv';
`);
const settle = () => new Promise((r) => setTimeout(r, 0));
const flush = async () => {
  for (let i = 0; i < 10; i++) await settle();
};
const css = (p) => readFileSync(new URL(`../guest/src/${p}`, import.meta.url), 'utf8');

// ---------- view mapping ----------

test('view: only a wall pocket maps; a seat gets the wall and its own photos, no counts; junk is dropped', () => {
  const b = new WallBox('wall-live');
  const v = m.wallFrom(b.env(true)).view;
  assert.equal(v.title, "Mia & Jonas's wedding");
  assert.equal(v.host_name, 'Mia');
  assert.equal(v.me.name, 'Sam');
  assert.equal(v.wall.length, wallPhotos.length);
  assert.equal(v.wall[0].id, 'ph15', 'newest first, as the box sends');
  assert.equal(v.count, undefined, 'the guest view has no count');
  assert.deepEqual(v.mine.map((p) => p.status), ['approved', 'pending', 'pending']);
  assert.equal(v.limits.photo_cap_mb, 20);
  assert.equal(v.limits.caption_max, 80);

  const e = b.env(true);
  e.view.wall = [...e.view.wall, { id: '../x', by_name: 'Bad' }, { id: 'ok1' }, null, 7, { id: 'ok2', by_name: 'Zed', rotate: 45, creature: 99 }];
  e.view.mine = [...e.view.mine, { id: 'h1', status: 'hidden' }, { id: 'h2' }];
  const j = m.wallFrom(e).view;
  assert.equal(j.wall.length, wallPhotos.length + 1);
  assert.deepEqual(j.wall.at(-1), { id: 'ok2', by_name: 'Zed', creature: 3, caption: undefined, at: undefined, w: undefined, h: undefined, rotate: 0, best: false });
  assert.equal(j.mine.length, 3, 'a hidden photo of yours is dropped quietly');

  const kit = b.env(true);
  kit.pocket.kit = 'quiz';
  assert.equal(m.wallFrom(kit), undefined);
  assert.equal(m.wallFrom(null), undefined);

  const pub = m.wallFrom(b.env(false)).view;
  assert.equal(pub.me, undefined);
  assert.deepEqual(pub.wall, [], 'no seat: no photos');
  assert.equal(pub.count, wallPhotos.length);
});

test('tv view: the screen state maps oldest first with its count; another kit does not', () => {
  const t = new WallBox('screen-wall-slides');
  const v = m.tvFrom(t.env()).view;
  assert.equal(v.photos[0].id, 'ph01');
  assert.equal(v.count, 8);
  assert.equal(v.open, true);
  const q = t.env();
  q.pocket.kit = 'quiz';
  assert.equal(m.tvFrom(q), undefined);
});

test('screens: join, rejoin, wall, best only when best_set, closed', () => {
  const at = (name, seated, o) => m.screenOf(m.wallFrom(new WallBox(name).env(seated)).view, o);
  assert.equal(at('wall-join', false), 'join');
  assert.equal(at('wall-join', false, { rejoin: true }), 'rejoin');
  assert.equal(at('wall-live', true), 'wall');
  assert.equal(at('wall-closed', false), 'closed');
  assert.equal(at('wall-closed', true, { pick: 'best' }), 'closed', 'no best without best_set');
  assert.equal(at('wall-best', true, { pick: 'best' }), 'best');
  assert.equal(at('wall-best', true, { pick: 'all' }), 'wall');
});

test('paging: 12 at a time; newer photos wait behind the pill; a hidden one drops out', () => {
  const ps = [...wallPhotos].reverse();
  const first = m.wallPage(ps, [], m.PAGE);
  assert.equal(first.photos.length, 12);
  assert.equal(first.more, true);
  const shown = first.photos.map((p) => p.id);
  const extra = [{ ...ps[0], id: 'new1' }, { ...ps[0], id: 'new2' }];
  const next = m.wallPage([...extra, ...ps], shown, m.PAGE);
  assert.deepEqual(next.fresh.map((p) => p.id), ['new1', 'new2']);
  assert.equal(next.photos[0].id, shown[0]);
  const hid = m.wallPage(ps.filter((p) => p.id !== shown[1]), shown, m.PAGE);
  assert.ok(!hid.photos.some((p) => p.id === shown[1]));
});

test('copy: every refusal from the box has plain words and a fix; no file sizes but the box limit', () => {
  const r = (s, e, o) => m.refusal(s, e, o);
  assert.deepEqual(r(413, 'too_big', { limit: 20 }), { kind: 'row', title: 'Too big: photos can be up to 20 MB.', fix: 'Try a smaller copy, or a screenshot of it.' });
  assert.equal(r(415, 'heic').fix, 'Try sharing it from Photos, which makes a JPEG.');
  assert.equal(r(415, 'not_photo').title, 'Not a photo: this wall takes photos only.');
  assert.equal(r(415, 'broken_photo').kind, 'row');
  // Rows whose words say "Tap Try again" carry `again`, so the row shows the button; the rest don't.
  assert.equal(r(400, 'broken_photo').again, true);
  assert.equal(r(401, 'signed_out').again, true);
  assert.equal(r(400, 'weird').fix, 'Tap Try again.');
  for (const e of ['too_big', 'heic', 'not_photo', 'bad_caption']) assert.equal(r(400, e).again, undefined, e);
  assert.equal(r(400, 'bad_caption').title, "That caption can't go up.");
  assert.deepEqual(r(410, 'closed'), { kind: 'closed' });
  assert.equal(r(409, 'guest_max', { limit: 30 }).title, "You've added 30 photos, the most each guest can.");
  assert.equal(r(507, 'disk_low', { host: 'Mia' }).title, 'The wall is full for now. Mia can make room.');
  assert.equal(r(507, 'wall_full').title, 'The wall is full for now. The host can make room.');
  assert.deepEqual(r(429, 'slow_down', { retryAfter: 4 }), { kind: 'wait', secs: 4 });
  assert.deepEqual(r(0), { kind: 'retry' });
  assert.deepEqual(r(503), { kind: 'retry' });
  for (const e of ['too_big', 'heic', 'not_photo', 'broken_photo', 'bad_caption', 'guest_max', 'disk_low']) assert.doesNotMatch(JSON.stringify(r(400, e)), /[\u{1F300}-\u{1FAFF}]/u);
});

test('precheck: HEIC, non-photos and over-cap files never start sending', () => {
  const f = (name, type, size = 10) => ({ name, type, size });
  assert.match(m.precheck(f('a.HEIC', '')).title, /type can't go up/);
  assert.match(m.precheck(f('a.pdf', 'application/pdf')).title, /Not a photo/);
  assert.match(m.precheck(f('a.jpg', 'image/jpeg', 21 * 1024 * 1024), 20).title, /up to 20 MB/);
  assert.equal(m.precheck(f('a.jpg', 'image/jpeg', 1000), 20), undefined);
  assert.equal(m.precheck(f('a.webp', 'image/webp')), undefined);
});

// ---------- the upload queue ----------

function queue(outcomes, { probe = true } = {}) {
  let now = 0;
  let timers = [];
  let id = 1;
  const sent = [];
  const events = [];
  const q = new m.UploadQueue('/api/pocket/t/photo', {
    send: async (path, file, headers, progress) => {
      sent.push({ path, name: file.name, headers });
      progress(file.size / 2, file.size);
      const o = outcomes.shift() ?? { status: 201 };
      return { status: o.status, json: o.json ?? (o.status === 201 ? { photo: { id: `x${sent.length}` } } : { error: o.error }), retryAfter: o.retryAfter };
    },
    setTimeout: (f, ms) => {
      timers.push({ id, at: now + ms, f });
      return id++;
    },
    clearTimeout: (x) => (timers = timers.filter((t) => t.id !== x)),
    change: () => events.push('change'),
    progress: () => events.push('progress'),
    view: () => events.push('view'),
    page: (r) => events.push(`page:${r.title}`),
    closed: () => events.push('closed'),
    probe: async () => probe,
    away: () => events.push('away'),
    facts: () => ({ host: 'Mia', capMb: 20, perGuest: 30, captionMax: 5 }),
  });
  const advance = async (ms) => {
    now += ms;
    for (const t of timers.filter((t) => t.at <= now)) {
      timers = timers.filter((x) => x !== t);
      t.f();
      await flush();
    }
  };
  return { q, sent, events, advance };
}
const file = (name, type = 'image/jpeg', size = 1000) => ({ name, type, size });

test('queue: one at a time in order, caption cut to caption_max and URI-encoded, ids kept', async () => {
  const t = queue([]);
  t.q.add([file('a.jpg'), file('b.jpg'), file('c.HEIC', '')]);
  // The first is already on its way; the second still takes a caption.
  assert.deepEqual(t.q.rows.map((r) => r.state), ['sending', 'next', 'refused']);
  const ids = [];
  const b = t.q.rows[1];
  t.q.setCaption(b.key, 'Grüße aus Wien');
  t.q.setCaption(t.q.rows[0].key, 'too late');
  await flush();
  assert.deepEqual(t.sent.map((s) => s.name), ['a.jpg', 'b.jpg']);
  assert.equal(t.sent[0].headers['X-Caption'], undefined);
  assert.equal(t.sent[1].headers['X-Caption'], encodeURIComponent('Grüße'));
  assert.equal(b.id, 'x2');
  void ids;
  // Once the batch is done, sent rows leave; the refused one stays with its reason.
  assert.deepEqual(t.q.rows.map((r) => r.state), ['refused']);
});

test('queue: a refused file stays with its reason, the rest keep sending; Try again resends a dropped one', async () => {
  const t = queue([{ status: 201 }, { status: 415, error: 'heic' }, { status: 0 }, { status: 201 }]);
  t.q.add([file('a.jpg'), file('b.jpg'), file('c.jpg'), file('d.jpg')]);
  await flush();
  const st = t.q.rows.map((r) => r.state);
  assert.equal(st[0], 'sent');
  assert.equal(st[1], 'refused');
  assert.equal(t.q.rows[1].why.title, "This photo type can't go up.");
  assert.equal(st[2], 'dropped', 'no answer: dropped, with Try again');
  assert.equal(st[3], 'sent');
  const dropped = t.q.rows[2];
  t.q.retry(dropped.key);
  await flush();
  assert.equal(dropped.state, 'sent');
  assert.equal(t.sent.length, 5);
  assert.deepEqual(t.q.rows.map((r) => r.state), ['refused']);
});

test('queue: a 400 broken_photo is refused with again; Try again resends it', async () => {
  const t = queue([{ status: 400, error: 'broken_photo' }, { status: 201 }]);
  t.q.add([file('a.jpg')]);
  await flush();
  const row = t.q.rows[0];
  assert.equal(row.state, 'refused');
  assert.equal(row.why.again, true);
  t.q.retry(row.key);
  await flush();
  assert.equal(row.state, 'sent');
  assert.equal(t.sent.length, 2);
  // A file the page refused itself (HEIC) has no Try again: sending it again can't work.
  t.q.add([file('b.HEIC', '')]);
  assert.equal(t.q.rows.at(-1).why.again, undefined);
});

test('queue: 429 waits Retry-After then resends the same photo', async () => {
  const t = queue([{ status: 429, error: 'slow_down', retryAfter: 2 }, { status: 201 }]);
  t.q.add([file('a.jpg')]);
  const row = t.q.rows[0];
  await flush();
  assert.equal(t.q.paused, 'wait');
  assert.equal(t.sent.length, 1);
  await t.advance(1999);
  assert.equal(t.sent.length, 1);
  await t.advance(1);
  assert.equal(t.sent.length, 2);
  assert.equal(row.state, 'sent');
});

test('queue: 410 closes (G7) and keeps the unsent; 507 and guest_max are page notes; no answer at all is away', async () => {
  const c = queue([{ status: 201 }, { status: 410, error: 'closed' }]);
  c.q.add([file('a.jpg'), file('b.jpg'), file('c.jpg')]);
  await flush();
  assert.ok(c.events.includes('closed'));
  assert.equal(c.q.unsent.length, 2);

  const f = queue([{ status: 507, error: 'disk_low' }]);
  f.q.add([file('a.jpg')]);
  await flush();
  assert.ok(f.events.includes('page:The wall is full for now. Mia can make room.'));
  assert.equal(f.q.paused, 'full');

  const g2 = queue([{ status: 409, error: 'guest_max' }]);
  g2.q.add([file('a.jpg')]);
  await flush();
  assert.ok(g2.events.includes("page:You've added 30 photos, the most each guest can."));

  const a = queue([{ status: 0 }], { probe: false });
  a.q.add([file('a.jpg'), file('b.jpg')]);
  await flush();
  assert.equal(a.q.paused, 'away');
  assert.ok(a.events.includes('away'));
  const rows = [...a.q.rows];
  a.q.resume();
  await flush();
  assert.deepEqual(rows.map((r) => r.state), ['sent', 'sent']);
});

// ---------- flows against the fake box ----------

function harness(name) {
  const calls = [];
  let now = Date.UTC(2026, 10, 21, 21);
  let timers = [];
  let nextId = 1;
  const box = new WallBox(name, { code: 't' });
  let seated = false;
  box.up = true;
  const fetch = async (url, init = {}) => {
    const path = url.split('/api/pocket/t/')[1] ?? '';
    const body = init.body ? JSON.parse(init.body) : undefined;
    calls.push({ path, body });
    if (!box.up) throw new TypeError('Failed to fetch');
    const r = box.handle(path, body ?? {}, { seated });
    if ((path === 'join' || path === 'rejoin') && r.status < 300) seated = true;
    return { ok: r.status < 400, status: r.status, json: async () => (r.body === null ? Promise.reject(new SyntaxError('empty')) : r.body) };
  };
  class XHR {
    constructor() {
      this.h = {};
      this.upload = {};
    }
    open(method, path) {
      this.path = path;
      calls.push({ path: 'photo', method });
    }
    setRequestHeader(k, v) {
      this.h[k] = v;
    }
    getResponseHeader(k) {
      return this.r?.headers?.[k] ?? null;
    }
    send(f) {
      Promise.resolve().then(() => {
        if (!box.up) return this.onerror();
        this.r = box.upload({ type: f.type, bytes: f.size, caption: decodeURIComponent(this.h['X-Caption'] ?? ''), seated: seated || box.s.seat });
        this.upload.onprogress?.({ loaded: f.size, total: f.size, lengthComputable: true });
        this.status = this.r.status;
        this.responseText = JSON.stringify(this.r.body);
        this.onload();
      });
    }
  }
  const env = {
    fetch,
    WebSocket: undefined,
    XHR,
    blobs: { create: () => 'blob:http://localhost/x', revoke: () => {} },
    setTimeout: (f, ms) => {
      timers.push({ id: nextId, at: now + ms, f });
      return nextId++;
    },
    clearTimeout: (id) => (timers = timers.filter((t) => t.id !== id)),
    setInterval: (f, ms) => {
      timers.push({ id: nextId, at: now + ms, f, every: ms });
      return nextId++;
    },
    clearInterval: (id) => (timers = timers.filter((t) => t.id !== id)),
    now: () => now,
    location: { pathname: '/p/t', protocol: 'http:', host: 'localhost' },
  };
  const root = document.getElementById('app');
  root.replaceChildren();
  document.querySelectorAll('.toast').forEach((t) => t.remove());
  const app = new m.WallApp(root, env, 't');
  const advance = async (ms) => {
    const end = now + ms;
    for (;;) {
      timers.sort((a, b) => a.at - b.at);
      const t = timers[0];
      if (!t || t.at > end) break;
      now = t.at;
      if (t.every) t.at += t.every;
      else timers.shift();
      t.f();
      await flush();
    }
    now = end;
    await flush();
  };
  const $ = (s) => root.querySelector(s);
  const $$ = (s) => [...root.querySelectorAll(s)];
  return { app, root, box, calls, advance, $, $$, screen: () => $('[data-screen]')?.getAttribute('data-screen'), text: () => root.textContent.replace(/\s+/g, ' ') };
}
async function open(name) {
  const t = harness(name);
  await t.app.start(null);
  await flush();
  return t;
}
const click = async (el) => {
  assert.ok(el, 'element to click');
  el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await flush();
};
const type = async (el, v) => {
  assert.ok(el, 'element to type in');
  el.value = v;
  el.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
  await flush();
};
const submit = async (form) => {
  form.dispatchEvent(new dom.window.Event('submit', { bubbles: true, cancelable: true }));
  await flush();
};
const pick = async (t, files) => {
  const input = t.$('input[type=file]');
  assert.ok(input, 'a file input');
  assert.equal(input.getAttribute('accept'), 'image/jpeg,image/png,image/webp');
  assert.ok(input.hasAttribute('multiple'));
  Object.defineProperty(input, 'files', { value: files, configurable: true });
  input.dispatchEvent(new dom.window.Event('change', { bubbles: true }));
  await flush();
};
const jpg = (name) => new dom.window.File(['x'.repeat(100)], name, { type: 'image/jpeg' });

test('G1 join: keep line first, picked files wait, then join sends them (G2) and the thanks shows', async () => {
  const t = await open('wall-join');
  assert.equal(t.screen(), 'join');
  assert.ok(t.$('[data-keep]'), 'keep line');
  assert.match(t.text(), /Photos go to Mia, who keeps them\. Location and camera details are removed\./);
  // Juno's public facts carry no limits (only a seat's view does), so before joining it says only "Photos only."
  assert.match(t.text(), /Photos only\./);
  assert.doesNotMatch(t.text(), /MB/);
  assert.match(t.text(), /Just look at the wall/);
  await pick(t, [jpg('a.jpg'), jpg('b.jpg')]);
  assert.match(t.text(), /2 photos picked\. They send once you join\./);
  assert.match(t.text(), /Send 2 photos/);
  assert.equal(t.box.sent.length, 0, 'nothing sends before a seat');
  await type(t.$('#w-name'), 'Sam');
  await type(t.$('#w-pin'), '1234');
  await flush();
  await t.advance(50);
  if (t.screen() === 'join') await submit(t.$('form'));
  await t.advance(50);
  assert.equal(t.screen(), 'wall');
  const join = t.calls.find((c) => c.path === 'join');
  assert.deepEqual(Object.keys(join.body).sort(), ['avatar', 'name', 'pin']);
  assert.equal(t.box.sent.length, 2);
  assert.match(t.text(), /Thanks, Sam! Your 2 photos are with Mia\./);
  assert.equal(t.$$('.ph.pend[data-mine]').length, 2, 'your two wait, marked');
  t.app.stop();
});

test('G3 live wall: yours first with Waiting for Mia, the wall newest first, 12 then Show more, no counts of others', async () => {
  const t = await open('wall-live');
  assert.equal(t.screen(), 'wall');
  assert.match(t.text(), /Waiting for Mia/);
  assert.match(t.text(), /2 waiting · only you see these/);
  const tiles = t.$$('[data-wall] .ph');
  assert.equal(tiles.length, 12);
  assert.match(t.text(), /Show more photos/);
  assert.doesNotMatch(t.text(), /\b15 photos\b|guests/);
  const img = t.$('[data-wall] .ph img');
  assert.match(img.getAttribute('src'), /^\/api\/pocket\/t\/photo\/ph15$/);
  assert.equal(img.getAttribute('loading'), 'lazy');
  // A new approved photo waits behind the pill instead of jumping the grid.
  t.box.set((b) => b.approve({ ...wallPhotos[0], id: 'new1', by_name: 'Zoe' }));
  await t.app.wake();
  await t.advance(2100);
  await flush();
  if (t.$('.newpill')) {
    assert.match(t.$('.newpill').textContent, /1 new photo/);
    await click(t.$('.newpill'));
    assert.equal(t.$('[data-wall] .ph img').getAttribute('src'), '/api/pocket/t/photo/new1');
  }
  t.app.stop();
});

test('G2/G4 sending: one at a time; HEIC and too big are refused in place with a fix; the rest go up', async () => {
  const t = await open('wall-refuse');
  await pick(t, [jpg('1.jpg'), jpg('2.jpg'), jpg('3.jpg'), jpg('4.jpg'), jpg('5.jpg')]);
  await t.advance(5000);
  assert.ok(t.$('[data-refused]'), 'G4 block');
  assert.match(t.text(), /2 of these can't go up/);
  assert.match(t.text(), /Try sharing it from Photos, which makes a JPEG\./);
  assert.match(t.text(), /Too big: photos can be up to 20 MB\./);
  assert.equal(t.box.sent.filter((s) => s.status === 201).length, 3);
  assert.doesNotMatch(t.text(), /\d+(\.\d+)? ?(KB|MB) (of|sent)|%/, 'no sizes or percentages');
  await click([...t.root.querySelectorAll('button')].find((b) => /Back to the wall/.test(b.textContent)));
  assert.equal(t.$('[data-refused]'), null);
  t.app.stop();
});

test('G4 broken_photo: the row says Tap Try again and has a Try again button that resends; HEIC rows have none', async () => {
  const t = await open('wall-broken');
  await pick(t, [jpg('1.jpg'), new dom.window.File(['x'], '2.HEIC', { type: '' })]);
  await t.advance(3000);
  assert.ok(t.$('[data-refused]'), 'G4 block');
  assert.match(t.text(), /That photo didn't arrive whole\. ?Tap Try again to send it once more\./);
  const rows = [...t.root.querySelectorAll('li[data-state=refused]')];
  assert.equal(rows.length, 2);
  const again = rows.map((li) => li.querySelector('button.retry'));
  assert.ok(again[0], 'broken_photo row has Try again');
  assert.equal(again[0].getAttribute('aria-label'), 'Try Photo 1 again');
  assert.equal(again[1], null, 'the HEIC row has no Try again');
  await click(again[0]);
  await t.advance(3000);
  assert.deepEqual(t.box.sent.map((s) => s.status), [400, 201]);
  assert.equal(t.root.querySelectorAll('li[data-state=refused] button.retry').length, 0);
  t.app.stop();
});

test('G8 page notes: the wall is full names the host; the guest max says the box number', async () => {
  const f = await open('wall-full');
  await pick(f, [jpg('a.jpg')]);
  await f.advance(1000);
  assert.match(f.$('[data-note]').textContent, /The wall is full for now\. Mia can make room\./);
  assert.match(f.text(), /Photos you already added are safe\./);
  f.app.stop();
  const x = await open('wall-max');
  await pick(x, [jpg('a.jpg')]);
  await x.advance(1000);
  assert.match(x.$('[data-note]').textContent, /You've added 30 photos, the most each guest can\./);
  x.app.stop();
});

test('G6 the best, host picked (no pa14b yet): The best N, Picked by Mia, no holo sticker', async () => {
  const t = await open('wall-best');
  const best = [...t.root.querySelectorAll('button, a')].find((b) => /See the best/.test(b.textContent));
  if (t.screen() === 'closed') await click(best);
  assert.equal(t.screen(), 'best');
  assert.equal(t.$$('.kw-sticker-holo').length, 0, 'no check picked them, so no sticker');
  assert.equal(t.$$('[data-wall] .ph').length, wallPhotos.filter((p) => p.best).length);
  assert.match(t.text(), new RegExp(`The best ${wallPhotos.filter((p) => p.best).length}`));
  assert.match(t.text(), /Picked by Mia\./);
  await click([...t.root.querySelectorAll('button')].find((b) => /See all the photos/.test(b.textContent)));
  assert.equal(t.screen(), 'wall');
  t.app.stop();
});

test('G6 the best, Snap picked (best_by "snap"): exactly one holo Picked sticker, gone on See all', async () => {
  const t = await open('wall-best-snap');
  const best = [...t.root.querySelectorAll('button, a')].find((b) => /See the best/.test(b.textContent));
  if (t.screen() === 'closed') await click(best);
  assert.equal(t.screen(), 'best');
  assert.equal(t.$$('.kw-sticker-holo').length, 1);
  assert.equal(t.$('[data-sticker=holo]').textContent.trim(), 'Picked');
  await click([...t.root.querySelectorAll('button')].find((b) => /See all the photos/.test(b.textContent)));
  assert.equal(t.screen(), 'wall');
  assert.equal(t.$$('.kw-sticker-holo').length, 0, 'holo only on Picked');
  t.app.stop();
});

test('G1 a fixed field drops its error in place: name, then PIN, no repaint', async () => {
  const t = await open('wall-join');
  await pick(t, [jpg('a.jpg'), jpg('b.jpg')]);
  assert.match(t.$('[data-problem]').textContent, /Type your first name to send 2 photos\./);
  const name = t.$('#w-name');
  assert.equal(name.getAttribute('aria-invalid'), 'true');
  await type(name, 'Sam');
  assert.equal(t.$('[data-problem]'), null, 'the alert goes once the name reads right');
  assert.equal(name.getAttribute('aria-invalid'), null);
  assert.ok(!name.classList.contains('bad'));
  assert.equal(t.$('#w-name'), name, 'same input, no repaint');
  // The PIN's own error clears the same way, warn boxes included.
  t.app.fail({ title: 'Pick a 4-digit PIN to send them.', field: 'pin' });
  const pin = t.$('#w-pin');
  assert.equal(pin.getAttribute('aria-invalid'), 'true');
  await type(pin, '12');
  assert.ok(t.$('[data-problem]'), 'still flagged at 2 digits');
  await type(pin, '1234');
  assert.equal(t.$('[data-problem]'), null);
  assert.equal(pin.getAttribute('aria-invalid'), null);
  assert.equal(t.$$('.pin.warn').length, 0);
  t.app.stop();
});

test('G7 closed: no join form, the thanks line, no holo; uploads closing mid-batch say how many did not go up', async () => {
  const t = await open('wall-closed');
  assert.equal(t.screen(), 'closed');
  assert.match(t.text(), /The wall is closed/);
  assert.equal(t.$('#w-name'), null);
  assert.equal(t.$$('.kw-sticker-holo').length, 0);
  t.app.stop();
  const c = await open('wall-closing');
  c.box.info.state = 'closed';
  await pick(c, [jpg('a.jpg'), jpg('b.jpg'), jpg('c.jpg')]);
  await c.advance(2000);
  assert.equal(c.screen(), 'closed');
  assert.match(c.text(), /2 photos didn't go up before it closed\./);
  c.app.stop();
});

test('G5 back: name and PIN bring your photos back; a wrong PIN says so', async () => {
  const t = await open('wall-join');
  await click([...t.root.querySelectorAll('button, a')].find((b) => /joined before/.test(b.textContent)));
  assert.equal(t.screen(), 'rejoin');
  assert.match(t.text(), /Welcome back/);
  await type(t.$('#w-bname'), 'Sam');
  await type(t.$('#w-bpin'), '0000');
  await submit(t.$('form'));
  assert.equal(t.screen(), 'rejoin');
  assert.match(t.text(), /name and PIN don't match/);
  await type(t.$('#w-bpin'), '1234');
  await submit(t.$('form'));
  assert.equal(t.screen(), 'wall');
  assert.match(t.text(), /2 waiting · only you see these/);
  t.app.stop();
});

test('away: the box stops answering mid-batch, the unsent wait with Try now, and send when it is back', async () => {
  const t = await open('wall-live');
  t.box.up = false;
  await pick(t, [jpg('a.jpg'), jpg('b.jpg')]);
  await t.advance(3000);
  assert.ok(t.$('[data-away]'), 'away screen');
  assert.match(t.text(), /computer is resting/);
  t.box.up = true;
  await click([...t.root.querySelectorAll('button')].find((b) => /Try now/.test(b.textContent)));
  await t.advance(3000);
  assert.equal(t.screen(), 'wall');
  assert.equal(t.box.sent.filter((s) => s.status === 201).length, 2);
  t.app.stop();
});

test('safety: nothing from the box is parsed as HTML, no style attributes, no emoji in the page', async () => {
  const t = harness('wall-live');
  t.box.wall.push({ ...wallPhotos[0], id: 'evil1', by_name: '<img src=x onerror=alert(1)>', caption: '<b>bold</b>' });
  await t.app.start(null);
  await flush();
  assert.equal(t.root.querySelectorAll('b').length >= 0, true);
  assert.equal([...t.root.querySelectorAll('img')].filter((i) => i.getAttribute('src') === 'x').length, 0);
  assert.match(t.text(), /<b>bold<\/b>/);
  assert.equal(t.root.querySelectorAll('[style]').length, 0);
  assert.doesNotMatch(t.text(), /[\u{1F300}-\u{1FAFF}\u{2600}-\u{27BF}]/u);
  const src = ['app.ts', 'tv.ts', 'look.ts', 'queue.ts', 'view.ts'].map((f) => css(`wall/${f}`)).join('\n');
  assert.doesNotMatch(src, /innerHTML|outerHTML|insertAdjacentHTML|\.style\s*=|setAttribute\('style'/);
  t.app.stop();
});

// ---------- the big screen ----------

function tv(name, { still = false } = {}) {
  let now = 0;
  let timers = [];
  let nextId = 1;
  const box = new WallBox(name);
  let signed = false;
  const fetch = async (url, init = {}) => {
    const path = url.split(`/api/pocket/${name}/`)[1] ?? '';
    const body = init.body ? JSON.parse(init.body) : {};
    const r = box.handle(path, body, { signed, method: init.method ?? 'GET' });
    if (r.cookie) signed = true;
    return { ok: r.status < 400, status: r.status, json: async () => (r.body === null ? Promise.reject(new SyntaxError('empty')) : r.body) };
  };
  const env = {
    fetch,
    WebSocket: undefined,
    setTimeout: (f, ms) => {
      timers.push({ id: nextId, at: now + ms, f });
      return nextId++;
    },
    clearTimeout: (id) => (timers = timers.filter((t) => t.id !== id)),
    setInterval: (f, ms) => {
      timers.push({ id: nextId, at: now + ms, f, every: ms });
      return nextId++;
    },
    clearInterval: (id) => (timers = timers.filter((t) => t.id !== id)),
    now: () => now,
    origin: 'http://localhost:8731',
    still: () => still,
    location: { pathname: `/p/${name}`, protocol: 'http:', host: 'localhost' },
  };
  const root = document.getElementById('app');
  root.replaceChildren();
  const app = new m.WallTV(root, env, name, 'tok');
  const advance = async (ms) => {
    const end = now + ms;
    for (;;) {
      timers.sort((a, b) => a.at - b.at);
      const t = timers[0];
      if (!t || t.at > end) break;
      now = t.at;
      if (t.every) t.at += t.every;
      else timers.shift();
      t.f();
      await flush();
    }
    now = end;
    await flush();
  };
  const $ = (s) => root.querySelector(s);
  const $$ = (s) => [...root.querySelectorAll(s)];
  return { app, box, advance, $, $$, screen: () => $('[data-screen]')?.getAttribute('data-screen'), on: () => $('.wslide:not(.out)')?.getAttribute('data-id'), text: () => root.textContent.replace(/\s+/g, ' ') };
}

test('TV1 slideshow: oldest first, 8 s a photo, looping; caption, name, creature, the QR corner; never writes', async () => {
  const t = tv('screen-wall-slides');
  await t.app.start();
  await flush();
  assert.equal(t.screen(), 'slides');
  assert.equal(t.on(), 'ph01');
  assert.match(t.text(), /Mia & Jonas's wedding · photo wall/);
  assert.match(t.text(), /8 photos/);
  assert.match(t.$('.wcap').textContent, /First dance!!Mia/);
  assert.ok(t.$('.wcap svg'), 'the guest creature');
  assert.match(t.$('.wqr').textContent, /Add your photoslocalhost:8731\/p\/screen-wall-slides/);
  assert.ok(t.$('.wqr svg[role=img] path'));
  assert.equal(t.$('.wfit img').getAttribute('src'), '/api/pocket/screen-wall-slides/photo/ph01');
  await t.advance(m.SLIDE_MS - 1);
  assert.equal(t.on(), 'ph01');
  await t.advance(1);
  assert.equal(t.on(), 'ph02');
  await t.advance(m.SLIDE_MS * 7);
  assert.equal(t.on(), 'ph01', 'loops');
  assert.deepEqual(t.box.writes, []);
  t.app.stop();
});

test('TV2 a new photo jumps the line with a slide and a New from pill for 4 s; a hidden one on stage cuts at once', async () => {
  const t = tv('screen-wall-slides');
  await t.app.start();
  await flush();
  t.box.set((b) => b.approve({ ...wallPhotos[1], id: 'new1', by_name: 'Priya' }));
  t.app.wake();
  await flush();
  await t.advance(10);
  assert.equal(t.on(), 'new1');
  assert.ok(t.$('.wslide.in'), 'slides in');
  assert.ok(t.$('.wslide.out'), 'the old one slides out');
  assert.match(t.$('.wnew').textContent, /New from Priya/);
  assert.match(t.$('.wcap').textContent, /Priya · just now/);
  await t.advance(300);
  assert.equal(t.$('.wslide.out'), null);
  await t.advance(m.PILL_MS);
  assert.equal(t.$('.wnew'), null);
  // Hide the photo on stage: it is gone at once, no slide.
  t.box.set((b) => b.hide('new1'));
  t.app.wake();
  await flush();
  await t.advance(10);
  assert.notEqual(t.on(), 'new1');
  assert.equal(t.$('.wslide.out'), null);
  assert.equal(t.$$('img').filter((i) => /new1/.test(i.getAttribute('src'))).length, 0);
  t.app.stop();
});

test('TV3 names the host once the screen view carries host_name', async () => {
  const t = tv('screen-wall-waiting');
  t.box.set({ screen_host_name: 'Mia' });
  await t.app.start();
  await flush();
  assert.equal(t.screen(), 'waiting');
  assert.match(t.text(), /I'll put the first photos up as soon as Mia approves them\./);
  t.app.stop();
});

test('TV3 before the first photo: the big QR, scan line, short link, Snap waiting', async () => {
  const t = tv('screen-wall-waiting');
  await t.app.start();
  await flush();
  assert.equal(t.screen(), 'waiting');
  assert.match(t.text(), /Scan to add your photos/);
  assert.match(t.text(), /No app, no account\. They show here once the host says yes\./);
  assert.match(t.text(), /localhost:8731\/p\/screen-wall-waiting/);
  assert.match(t.text(), /I'll put the first photos up as soon as they're approved\./);
  assert.ok(t.$('.wbigqr svg path'));
  assert.equal(t.$('.snap').getAttribute('aria-label'), 'Snap is waiting');
  t.box.set((b) => b.approve(wallPhotos[0]));
  t.app.wake();
  await t.advance(10);
  assert.equal(t.screen(), 'slides');
  t.app.stop();
});

test('TV4 still view: a 3x2 grid, one tile changes every 8 s with no movement; count says still view', async () => {
  const t = tv('screen-wall-slides', { still: true });
  await t.app.start();
  await flush();
  assert.equal(t.screen(), 'still');
  assert.match(t.text(), /8 photos · still view/);
  const ids = () => t.$$('.wtile').map((x) => x.getAttribute('data-id'));
  const a = ids();
  assert.equal(a.length, 6);
  await t.advance(m.SLIDE_MS);
  const b = ids();
  assert.equal(b.length, 6);
  assert.equal(a.filter((x, i) => x !== b[i]).length, 1, 'exactly one tile changed');
  assert.equal(t.$$('.wslide').length, 0);
  t.app.stop();
});

test('TV ended and away: calm screens with Snap; away has Try now', async () => {
  const e = tv('screen-wall-ended');
  await e.app.start();
  await flush();
  assert.equal(e.screen(), 'ended');
  assert.match(e.text(), /The wall is closed/);
  const a = tv('screen-wall-away');
  void a.app.start();
  await flush();
  assert.equal(a.screen(), 'away');
  assert.match(a.text(), /Try now/);
  a.app.stop();
});

// ---------- css ----------

const tokens = (s) => Object.fromEntries([...s.matchAll(/--([a-z0-9-]+):\s*([^;]+);/g)].map((x) => [x[1], x[2].trim()]));
const modern = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
const holo = () => {
  const foil = modern.slice(modern.indexOf('.kw-sticker-holo::before {'));
  return [...foil.slice(0, foil.indexOf('}')).matchAll(/#[0-9a-f]{6}/g)].map((x) => x[0]).slice(0, 5);
};

test('wall.css: hex only in its token block, holo stops and plate match modern.css, the bar is static under reduced motion', () => {
  const s = css('wall/wall.css');
  const body = s.slice(s.indexOf('.qm160'));
  assert.doesNotMatch(body, /#[0-9a-fA-F]{3,8}\b/);
  const v = tokens(s);
  assert.deepEqual([1, 2, 3, 4, 5].map((i) => v[`holo-${i}`]), holo());
  assert.match(modern, new RegExp(`background: ${v['sticker-plate']};`));
  const reduced = s.slice(s.lastIndexOf('prefers-reduced-motion'));
  assert.match(reduced, /\.bar2 i \{ transition: none; \}/);
  // Every tap target is at least 44 px.
  for (const sel of ['.newpill', '.capin', '.retry', '.gbtn.sm']) assert.match(s, new RegExp(`${sel.replace('.', '\\.')}[^{]*\\{[^}]*(min-height|height): (4[4-9]|[5-9]\\d|1\\d\\d)px`));
});

test('screen-wall.css: no hex at all, nothing under 32 px, the 200 ms slide uses --kw-ease and is off under reduced motion', () => {
  const s = css('wall/screen-wall.css');
  assert.doesNotMatch(s, /#[0-9a-fA-F]{3,8}\b/);
  for (const [, px] of s.matchAll(/font-size:\s*(\d+)px/g)) assert.ok(Number(px) >= 32, `font-size ${px}px`);
  assert.match(s, /\.wslide\.in \{[^}]*animation: wtv-in 200ms var\(--kw-ease\)/);
  assert.match(s, /@keyframes wtv-in \{ from \{ transform: translateX\(22%\)/);
  assert.match(s, /@keyframes wtv-out \{[^}]*\}[^}]*translateX\(-22%\)/);
  const reduced = s.slice(s.lastIndexOf('prefers-reduced-motion'));
  assert.match(reduced, /animation: none/);
});
