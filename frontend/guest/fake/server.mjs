// A fake pa1 guest port for the built guest page: serves dist/pocket exactly as pa5/impl.md asks Lin to, with the
// pocket CSP, and answers /api/pocket/<code>/* from views.mjs. The code picks the scenario: /p/lobby, /p/away, ...
// Also a minimal WebSocket (RFC 6455 text frames, server to client only) that pushes the current view.
// Usage: node guest/fake/server.mjs [port]   (default 8731; serves ../../dist/pocket)
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import { readFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join, extname } from 'node:path';
import { scenarios, CSP, KIWI_SVG } from './views.mjs';

const DIST = fileURLToPath(new URL('../../dist/pocket/', import.meta.url));
const TYPES = { '.js': 'text/javascript', '.css': 'text/css', '.woff2': 'font/woff2', '.html': 'text/html; charset=utf-8' };

const fill = (v, code) => (v && typeof v === 'object' ? JSON.parse(JSON.stringify(v).replaceAll('/CODE/', `/${code}/`)) : v);

export function startFake({ port = 8731, dist = DIST, actionDelayMs = 400, boot = false } = {}) {
  const seats = new Map(); // code -> view after join
  const sockets = new Map(); // code -> Set<socket>
  const log = [];

  const viewFor = (code, what) => {
    const s = scenarios[code];
    if (!s) return 404;
    if (what === 'rejoin' && seats.has(code)) return seats.get(code);
    if (what === 'state' && seats.has(code)) return seats.get(code);
    const v = s[what] ?? (what === 'state' ? s.rejoin : undefined);
    return fill(v, code);
  };

  const send = (res, status, body, type = 'application/json', extra = {}) => {
    res.writeHead(status, { 'Content-Type': type, 'Cache-Control': 'no-store', ...extra });
    res.end(typeof body === 'string' || Buffer.isBuffer(body) ? body : JSON.stringify(body));
  };
  const reply = (res, v) => (typeof v === 'number' ? send(res, v, { error: v === 410 ? 'removed' : v === 401 ? 'no_seat' : 'unavailable' }) : send(res, 200, { view: v }));

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
        const v = viewFor(m[1], 'state');
        if (typeof v === 'object') html = html.replace('>null</script>', `>${JSON.stringify(v).replaceAll('<', '\\u003c')}</script>`);
      }
      return send(res, 200, html, TYPES['.html'], { 'Content-Security-Policy': CSP, 'X-Content-Type-Options': 'nosniff', 'Referrer-Policy': 'no-referrer' });
    }
    m = p.match(/^\/api\/pocket\/([^/]+)\/(state|rejoin|join|action|media\/kiwi\.svg)$/);
    if (!m) return send(res, 404, { error: 'not_found' });
    const [, code, what] = m;
    if (what === 'media/kiwi.svg') return send(res, 200, KIWI_SVG, 'image/svg+xml', { 'Cache-Control': 'public, max-age=3600' });
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      if (what === 'state' || what === 'rejoin') return reply(res, viewFor(code, what));
      if (what === 'join') {
        const v = fill(scenarios[code]?.join, code);
        if (!v) return send(res, 409, { error: 'ended' });
        const b = JSON.parse(body || '{}');
        const seated = { ...v, me: { ...v.me, name: b.name ?? v.me.name, avatar: b.avatar ?? v.me.avatar } };
        seats.set(code, seated);
        return send(res, 200, { view: seated });
      }
      // action: answer
      const v = fill(scenarios[code]?.action, code);
      setTimeout(() => {
        if (!v) return send(res, 200, {});
        seats.set(code, v);
        send(res, 200, { view: v });
      }, actionDelayMs);
    });
  });

  server.on('upgrade', (req, socket) => {
    const m = new URL(req.url, 'http://x').pathname.match(/^\/api\/pocket\/([^/]+)\/ws$/);
    const key = req.headers['sec-websocket-key'];
    if (!m || !key) return socket.destroy();
    const code = m[1];
    const v = viewFor(code, 'state');
    if (typeof v === 'number') return socket.destroy();
    const accept = createHash('sha1').update(key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    log.push(`WS ${code}`);
    if (!sockets.has(code)) sockets.set(code, new Set());
    sockets.get(code).add(socket);
    socket.on('close', () => sockets.get(code)?.delete(socket));
    socket.on('error', () => sockets.get(code)?.delete(socket));
    socket.on('data', () => {}); // client frames (pings, close) are ignored
  });

  /** Push a view to every socket of a code (what pa1 does on any change). */
  const push = (code, view) => {
    seats.set(code, view);
    const data = Buffer.from(JSON.stringify({ view }));
    const head = data.length < 126 ? Buffer.from([0x81, data.length]) : Buffer.from([0x81, 126, data.length >> 8, data.length & 255]);
    for (const s of sockets.get(code) ?? []) s.write(Buffer.concat([head, data]));
  };

  return new Promise((resolve) => server.listen(port, '127.0.0.1', () => resolve({ server, port: server.address().port, push, log, close: () => new Promise((r) => { for (const set of sockets.values()) for (const s of set) s.destroy(); server.close(r); }) })));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const f = await startFake({ port: Number(process.argv[2] || 8731) });
  console.log(`fake guest port on http://127.0.0.1:${f.port}/p/join`);
}
