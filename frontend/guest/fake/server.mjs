// A fake pa1 guest port for the built guest page: serves dist/pocket exactly as pa5/impl.md asks Lin to, with the
// pocket CSP, and answers /api/pocket/<code>/* from views.mjs. The code picks the scenario: /p/lobby, /p/away, ...
// Also a minimal WebSocket (RFC 6455 text frames, server to client only) that pushes the current view.
// Usage: node guest/fake/server.mjs [port]   (default 8731; serves ../../dist/pocket)
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import { readFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join, extname } from 'node:path';
import { FakeBox, CSP, KIWI_SVG } from './views.mjs';
import { SantaBox } from './santa.mjs';
import { DeskBox } from './desk.mjs';
import { GatherBox } from './gather.mjs';
import { RemoteBox } from './remote.mjs';
import { ScreenBox } from './screen.mjs';
import { WallBox } from './wall.mjs';
import { QueueBox } from './queue.mjs';

const DIST = fileURLToPath(new URL('../../dist/pocket/', import.meta.url));
const TYPES = { '.js': 'text/javascript', '.css': 'text/css', '.woff2': 'font/woff2', '.html': 'text/html; charset=utf-8' };

export function startFake({ port = 8731, dist = DIST, actionDelayMs = 400, boot = false } = {}) {
  const boxes = new Map(); // code -> FakeBox (one per scenario code, like one pocket)
  const sockets = new Map(); // code -> Set<socket>
  const log = [];
  const box = (code) => {
    // Codes starting with santa- are Secret Santa pockets (santa.mjs), gather- the pa10b gather kit (gather.mjs), screen- the
    // big screen (screen.mjs), remote- the host remote (remote.mjs), wall- and screen-wall- the Wall and its big screen
    // (wall.mjs), desk- the front desk (desk.mjs), queue- and screen-queue- the Queue ticket and its big screen (queue.mjs); the rest are quiz scenarios.
    if (!boxes.has(code))
      boxes.set(code, code.startsWith('queue-') || code.startsWith('screen-queue-') ? new QueueBox(code) : code.startsWith('wall-') || code.startsWith('screen-wall-') ? new WallBox(code) : code.startsWith('santa-') ? new SantaBox(code) : code.startsWith('desk-') ? new DeskBox(code) : code.startsWith('gather-') ? new GatherBox(code) : code.startsWith('screen-') ? new ScreenBox(code) : code.startsWith('remote-') ? new RemoteBox(code) : new FakeBox(code));
    return boxes.get(code);
  };

  const send = (res, status, body, type = 'application/json', extra = {}) => {
    res.writeHead(status, { 'Content-Type': type, 'Cache-Control': 'no-store', ...extra });
    res.end(typeof body === 'string' || Buffer.isBuffer(body) ? body : JSON.stringify(body));
  };

  const server = createServer((req, res) => {
    const url = new URL(req.url, 'http://x');
    const p = url.pathname;
    log.push(`${req.method} ${p}`);
    if (p.startsWith('/p/_a/')) {
      const f = p.slice(6);
      if (!f || f.includes('/') || f.includes('..') || f === 'index.html' || !existsSync(join(dist, f))) return send(res, 404, 'not found', 'text/plain');
      return send(res, 200, readFileSync(join(dist, f)), TYPES[extname(f)] ?? 'application/octet-stream', { 'Cache-Control': 'public, max-age=31536000, immutable' });
    }
    let m = p.match(/^\/p\/([^/]+)$/);
    if (m) {
      let html = readFileSync(join(dist, 'index.html'), 'utf8');
      if (boot) {
        // As pa1 does (pa1/impl.md "Boot JSON"): the seat cookie never reaches /p/<code>, so the page always carries the
        // public facts (the `info` JSON), `<` escaped. A removed guest's or an away box's page still names the pocket.
        const r = box(m[1]).handle('info', {}, { seated: false });
        if (r.status === 200) html = html.replace('>null</script>', `>${JSON.stringify(r.body).replaceAll('<', '\\u003c')}</script>`);
      }
      return send(res, 200, html, TYPES['.html'], { 'Content-Security-Policy': CSP, 'X-Content-Type-Options': 'nosniff', 'Referrer-Policy': 'no-referrer' });
    }
    m = p.match(/^\/api\/pocket\/((?:screen|remote)-[^/]+)\/(host\/[\w/-]+)$/);
    if (m) {
      const [, code, what] = m;
      let body = '';
      req.on('data', (c) => (body += c));
      req.on('end', () => {
        let json = {};
        try {
          json = JSON.parse(body || '{}');
        } catch {
          json = {};
        }
        // The remote's box tells a host session from a screen one by the cookie's value (pa1b: a screen gets screen_only).
        const kph = (req.headers.cookie ?? '').match(new RegExp(`kph_${code}=(\\w+)`))?.[1];
        const signed = code.startsWith('remote-') ? (kph ?? false) : !!kph;
        const r = box(code).handle(`${what}${url.search}`, json, { signed, method: req.method });
        // Like pa1's host/signin: the session is an HttpOnly cookie scoped to the pocket's API.
        if (r.cookie) res.setHeader('Set-Cookie', `kph_${code}=${r.cookie === true ? 'fake' : r.cookie}; Path=/api/pocket/${code}; HttpOnly; SameSite=Strict`);
        if (r.status === 204) {
          res.writeHead(204, { 'Cache-Control': 'no-store' });
          return res.end();
        }
        send(res, r.status, r.body);
      });
      return;
    }
    // The Wall (pa14): GET photo/<id> gives the photo's bytes (an SVG stand-in), POST photo takes one raw file.
    m = p.match(/^\/api\/pocket\/([^/]+)\/photo(?:\/([\w-]+))?$/);
    if (m && box(m[1]) instanceof WallBox) {
      const b = box(m[1]);
      if (req.method === 'GET' && m[2]) {
        const svg = b.photo(m[2]);
        return svg ? send(res, 200, svg, 'image/svg+xml', { 'Cache-Control': 'private, max-age=3600' }) : send(res, 404, { error: 'not_found' });
      }
      if (req.method !== 'POST' || m[2]) return send(res, 405, { error: 'method' });
      let bytes = 0;
      req.on('data', (c) => (bytes += c.length));
      req.on('end', () => {
        const seated = (req.headers.cookie ?? '').includes(`kp_${m[1]}=`);
        let caption = '';
        try {
          caption = decodeURIComponent(String(req.headers['x-caption'] ?? ''));
        } catch {
          caption = '';
        }
        setTimeout(() => {
          const r = b.upload({ type: req.headers['content-type'] ?? '', bytes, caption, seated });
          send(res, r.status, r.body, 'application/json', r.headers ?? {});
        }, b.s?.delayMs ?? 250);
      });
      return;
    }
    m = p.match(/^\/api\/pocket\/([^/]+)\/(state|info|rejoin|join|action|notices\/seen|media\/kiwi\.svg)$/);
    if (!m) return send(res, 404, { error: 'not_found' });
    const [, code, what] = m;
    if (what === 'media/kiwi.svg') return send(res, 200, KIWI_SVG, 'image/svg+xml', { 'Cache-Control': 'public, max-age=3600' });
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      const answer = () => {
        const seated = (req.headers.cookie ?? '').includes(`kp_${code}=`);
        const r = box(code).handle(`${what}${url.search}`, JSON.parse(body || '{}'), { seated });
        // Like pa1: join and rejoin hand this phone its seat as an HttpOnly cookie scoped to the pocket's API.
        if ((what === 'join' || what === 'rejoin') && r.status < 300) res.setHeader('Set-Cookie', `kp_${code}=fake; Path=/api/pocket/${code}; HttpOnly; SameSite=Strict`);
        if (r.status === 204) {
          res.writeHead(204, { 'Cache-Control': 'no-store' });
          return res.end();
        }
        send(res, r.status, r.body);
      };
      if (what === 'action') setTimeout(answer, actionDelayMs);
      else answer();
    });
  });

  server.on('upgrade', (req, socket) => {
    const m = new URL(req.url, 'http://x').pathname.match(/^\/api\/pocket\/([^/]+)\/ws$/);
    const key = req.headers['sec-websocket-key'];
    if (!m || !key) return socket.destroy();
    const code = m[1];
    if (code.startsWith('screen-')) {
      const signed = (req.headers.cookie ?? '').includes(`kph_${code}=`);
      if (new URL(req.url, 'http://x').searchParams.get('view') !== 'screen' || box(code).handle('host/state?view=screen', {}, { signed }).status !== 200) return socket.destroy();
    } else if (code.startsWith('remote-')) {
      const kph = (req.headers.cookie ?? '').match(new RegExp(`kph_${code}=(\\w+)`))?.[1] ?? false;
      if (new URL(req.url, 'http://x').searchParams.get('view') !== 'host' || box(code).handle('host/state', {}, { signed: kph }).status !== 200) return socket.destroy();
    } else if (box(code).handle('state', {}, { seated: (req.headers.cookie ?? '').includes(`kp_${code}=`) }).status !== 200) return socket.destroy();
    const accept = createHash('sha1').update(key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    log.push(`WS ${code}`);
    if (!sockets.has(code)) sockets.set(code, new Set());
    sockets.get(code).add(socket);
    socket.on('close', () => sockets.get(code)?.delete(socket));
    socket.on('error', () => sockets.get(code)?.delete(socket));
    socket.on('data', () => {}); // client frames (pings, close) are ignored
  });

  /** Push a flat view to every socket of a code as a v1 `{type:"state", ...}` frame (what pa1 does on any change). */
  const push = (code, view) => {
    const b = box(code);
    if (b instanceof RemoteBox) {
      // The remote's box changes itself (host writes); a push with a function edits it first, e.g. a new near miss.
      if (typeof view === 'function') view(b);
      b.version++;
    } else b.set(view);
    const data = Buffer.from(JSON.stringify({ type: 'state', ...(b instanceof ScreenBox || b instanceof RemoteBox || b instanceof WallBox || b instanceof QueueBox ? b.env() : b.env(view)) }));
    const head = data.length < 126 ? Buffer.from([0x81, data.length]) : Buffer.from([0x81, 126, data.length >> 8, data.length & 255]);
    for (const s of sockets.get(code) ?? []) s.write(Buffer.concat([head, data]));
  };

  return new Promise((resolve) => server.listen(port, '127.0.0.1', () => resolve({ server, port: server.address().port, push, log, box, close: () => new Promise((r) => { for (const set of sockets.values()) for (const s of set) s.destroy(); server.close(r); }) })));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const f = await startFake({ port: Number(process.argv[2] || 8731) });
  console.log(`fake guest port on http://127.0.0.1:${f.port}/p/join`);
}
