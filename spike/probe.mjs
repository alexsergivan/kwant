// Browser spike probe (0.8.0 section C, Vega). Starts ONE browser, measures start time and memory,
// replays the DevTools calls kwant's sandbox/browser.rs actions make (same methods and params),
// checks each one's effect, tries sign-in pages and screenshots, then stops the browser.
//   node probe.mjs <name> <port> <outdir> <exe> [args...]
// Linux: memory is PSS summed over the browser's whole process group (/proc/*/smaps_rollup).
// Mac/Windows: RSS summed over the process tree (ps / PowerShell), marked as RSS in the output.
import { spawn, execFileSync } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';

const [name, portArg, outdir, exe, ...args] = process.argv.slice(2);
const port = Number(portArg);
const SITES = (process.env.SPIKE_SITES ?? 'on') !== 'off';
fs.mkdirSync(outdir, { recursive: true });
const out = { name, exe: path.basename(exe), args, platform: process.platform, arch: process.arch, results: [], sites: [], signin: [], mem: {} };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const now = () => Number(process.hrtime.bigint() / 1000n) / 1000; // ms

// ---------- memory ----------
function linuxGroup(pgid) {
  const pids = [];
  for (const d of fs.readdirSync('/proc')) {
    if (!/^\d+$/.test(d)) continue;
    try {
      const stat = fs.readFileSync(`/proc/${d}/stat`, 'utf8');
      const f = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
      if (Number(f[2]) === pgid) pids.push(Number(d));
    } catch {}
  }
  return pids;
}
function memNow(child) {
  try {
    if (process.platform === 'linux') {
      let pss = 0, rss = 0, threads = 0;
      const pids = linuxGroup(child.pid);
      for (const p of pids) {
        try {
          const r = fs.readFileSync(`/proc/${p}/smaps_rollup`, 'utf8');
          pss += Number(/^Pss:\s+(\d+)/m.exec(r)?.[1] ?? 0);
          rss += Number(/^Rss:\s+(\d+)/m.exec(r)?.[1] ?? 0);
          threads += Number(/^Threads:\s+(\d+)/m.exec(fs.readFileSync(`/proc/${p}/status`, 'utf8'))?.[1] ?? 0);
        } catch {}
      }
      return { kind: 'PSS', mib: pss / 1024, rss_mib: rss / 1024, procs: pids.length, threads };
    }
    if (process.platform === 'darwin') {
      const txt = execFileSync('ps', ['-axo', 'pid=,ppid=,rss=']).toString();
      const rows = txt.trim().split('\n').map((l) => l.trim().split(/\s+/).map(Number));
      const set = new Set([child.pid]);
      let grew = true;
      while (grew) { grew = false; for (const [p, pp] of rows) if (set.has(pp) && !set.has(p)) { set.add(p); grew = true; } }
      const rss = rows.filter(([p]) => set.has(p)).reduce((a, [, , r]) => a + r, 0);
      return { kind: 'RSS', mib: rss / 1024, procs: set.size };
    }
    if (process.platform === 'win32') {
      const ps = `$all=Get-CimInstance Win32_Process|Select ProcessId,ParentProcessId,WorkingSetSize,PrivatePageCount;$s=@{${child.pid}=1};do{$n=$s.Count;foreach($p in $all){if($s.ContainsKey([int]$p.ParentProcessId) -and -not $s.ContainsKey([int]$p.ProcessId)){$s[[int]$p.ProcessId]=1}}}while($s.Count -ne $n);$ws=0;$pv=0;foreach($p in $all){if($s.ContainsKey([int]$p.ProcessId)){$ws+=$p.WorkingSetSize;$pv+=$p.PrivatePageCount}};"$ws $pv $($s.Count)"`;
      const [ws, pv, n] = execFileSync('powershell', ['-NoProfile', '-Command', ps]).toString().trim().split(' ').map(Number);
      return { kind: 'WorkingSet', mib: ws / 1048576, private_mib: pv / 1048576, procs: n };
    }
  } catch (e) { return { error: String(e).slice(0, 200) }; }
  return {};
}
let peak = 0, sampler = null, child = null;
function startSampler() {
  sampler = setInterval(() => { const m = memNow(child); if (m.mib > peak) peak = m.mib; }, process.platform === 'linux' ? 200 : 1000);
}

// ---------- local test pages (no network needed) ----------
const PAGES = {
  '/form': `<!doctype html><html><head><title>form</title><meta name=viewport content="width=device-width"></head><body style="margin:0;font:16px sans-serif">
<button id=b style="position:absolute;left:0;top:0;width:400px;height:200px" onclick="document.title='clicked '+innerWidth+'x'+innerHeight">press</button>
<form id=f method=post action=/submit style="position:absolute;left:0;top:240px">
<input id=u name=u autocomplete=username style="width:300px;height:40px"><br>
<input id=p name=p type=password autocomplete=current-password style="width:300px;height:40px"><br>
<button id=s type=submit>Sign in</button></form>
<a href="/long" id=lnk style="position:absolute;left:0;top:420px">Long page</a>
<iframe src="/frame" style="position:absolute;left:500px;top:0;width:300px;height:200px"></iframe>
<button id=al style="position:absolute;left:0;top:460px" onclick="alert('hi');document.title='after alert'">alert</button>
</body></html>`,
  '/frame': `<!doctype html><title>frame</title><p>inner frame text</p>`,
  '/long': `<!doctype html><title>long</title><body style="margin:0">${Array.from({ length: 200 }, (_, i) => `<p style="height:40px;margin:0">row ${i}</p>`).join('')}</body>`,
};
let server, base;
async function startServer() {
  server = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      const p = req.url.split('?')[0];
      if (p === '/submit') {
        res.writeHead(200, { 'content-type': 'text/html', 'set-cookie': 'after=submit; Path=/' });
        res.end(`<!doctype html><title>submitted</title><pre id=r>${body.replace(/</g, '&lt;')}|cookie=${(req.headers.cookie ?? '').replace(/</g, '&lt;')}</pre>`);
        return;
      }
      if (p === '/echo') {
        res.writeHead(200, { 'content-type': 'text/html' });
        res.end(`<!doctype html><title>echo</title><pre id=r>cookie=${(req.headers.cookie ?? '').replace(/</g, '&lt;')}|ua=${(req.headers['user-agent'] ?? '').replace(/</g, '&lt;')}</pre>`);
        return;
      }
      const page = PAGES[p];
      res.writeHead(page ? 200 : 404, { 'content-type': 'text/html', 'set-cookie': 'srv=1; Path=/' });
      res.end(page ?? 'nope');
    });
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  base = `http://127.0.0.1:${server.address().port}`;
}

// ---------- CDP ----------
class Cdp {
  constructor(url) { this.url = url; this.id = 0; this.wait = new Map(); this.events = []; this.listeners = []; }
  open() {
    return new Promise((res, rej) => {
      this.ws = new WebSocket(this.url);
      this.ws.onopen = () => res(this);
      this.ws.onerror = (e) => rej(new Error('ws error ' + (e.message ?? '')));
      this.ws.onmessage = (m) => {
        const msg = JSON.parse(m.data);
        if (msg.id && this.wait.has(msg.id)) { const w = this.wait.get(msg.id); this.wait.delete(msg.id); msg.error ? w.rej(new Error(JSON.stringify(msg.error))) : w.res(msg.result); }
        else if (msg.method) { this.events.push(msg); for (const l of this.listeners) l(msg); }
      };
    });
  }
  call(method, params = {}, sessionId, ms = 15000) {
    const id = ++this.id;
    const msg = { id, method, params }; if (sessionId) msg.sessionId = sessionId;
    this.ws.send(JSON.stringify(msg));
    return new Promise((res, rej) => {
      this.wait.set(id, { res, rej });
      setTimeout(() => { if (this.wait.has(id)) { this.wait.delete(id); rej(new Error('timeout ' + method)); } }, ms);
    });
  }
  once(method, ms = 10000, pred = () => true) {
    return new Promise((res) => {
      const t = setTimeout(() => { this.listeners = this.listeners.filter((x) => x !== l); res(null); }, ms);
      const l = (m) => { if (m.method === method && pred(m)) { clearTimeout(t); this.listeners = this.listeners.filter((x) => x !== l); res(m); } };
      this.listeners.push(l);
    });
  }
  close() { try { this.ws.close(); } catch {} }
}
const getJson = async (p, method = 'GET') => (await fetch(`http://127.0.0.1:${port}${p}`, { method, signal: AbortSignal.timeout(3000) })).json();

// one row per kwant action / DevTools method: works = the call is accepted; effect = it did what kwant needs
async function check(area, method, fn) {
  const t = now();
  try {
    const effect = await fn();
    out.results.push({ area, method, works: true, effect: effect === true ? 'yes' : effect === false ? 'NO' : String(effect), ms: Math.round(now() - t) });
  } catch (e) {
    out.results.push({ area, method, works: false, effect: 'error: ' + String(e.message ?? e).slice(0, 160), ms: Math.round(now() - t) });
  }
}
const evalv = async (cdp, expression, extra = {}) => (await cdp.call('Runtime.evaluate', { expression, returnByValue: true, ...extra })).result?.value;
async function navLoad(cdp, url, ms = 20000) {
  const loaded = cdp.once('Page.loadEventFired', ms);
  const r = await cdp.call('Page.navigate', { url }, undefined, ms);
  if (r.errorText) throw new Error('navigate: ' + r.errorText);
  await loaded;
  return r;
}
async function personEnter() {
  await cdp.call('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', text: '\r', windowsVirtualKeyCode: 13, modifiers: 0, location: 0, autoRepeat: false });
  await cdp.call('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, modifiers: 0, location: 0, autoRepeat: false });
}
function jpegSize(b64) {
  const b = Buffer.from(b64, 'base64');
  if (b[0] === 0x89) return [b.readUInt32BE(16), b.readUInt32BE(20)]; // PNG
  for (let i = 2; i < b.length - 9;) {
    if (b[i] !== 0xff) { i++; continue; }
    const m = b[i + 1];
    if (m >= 0xc0 && m <= 0xc3) return [b.readUInt16BE(i + 7), b.readUInt16BE(i + 5)];
    i += 2 + b.readUInt16BE(i + 2);
  }
  return [0, 0];
}
const save = (file, b64) => fs.writeFileSync(path.join(outdir, file), Buffer.from(b64, 'base64'));

// ---------- run ----------
await startServer();
const profile = fs.mkdtempSync(path.join(outdir, 'profile-'));
const realArgs = args.map((a) => a.replace('{profile}', profile).replace('{port}', String(port)));
const t0 = now();
child = spawn(exe, realArgs, { detached: true, stdio: ['ignore', 'ignore', fs.openSync(path.join(outdir, 'stderr.log'), 'w')] });
startSampler();
let version = null;
for (let i = 0; i < 3000 && !version; i++) { try { version = await getJson('/json/version'); } catch { await sleep(5); } }
out.start_ms_json_version = Math.round(now() - t0);
out.browser = version?.Browser; out.ua = version?.['User-Agent'];
let page = null;
for (let i = 0; i < 400 && !page; i++) { try { page = (await getJson('/json/list')).find((t) => t.type === 'page'); } catch {} if (!page) await sleep(10); }
out.start_ms_page_target = Math.round(now() - t0);
if (!page) { try { page = await getJson('/json/new?about:blank', 'PUT'); out.note_page = 'no page at start; made one with PUT /json/new'; } catch (e) { out.note_page = 'no page target: ' + e; } }
await sleep(1500);
out.mem.idle = memNow(child);

const cdp = await new Cdp(page.webSocketDebuggerUrl).open();
const t1 = now();
await check('navigate', 'Page.enable', async () => { await cdp.call('Page.enable'); return true; });
await check('navigate', 'Page.navigate (+loadEventFired)', async () => { await navLoad(cdp, `${base}/form`); return (await evalv(cdp, 'document.title')) === 'form'; });
out.first_local_page_ms = Math.round(now() - t1);
out.start_to_first_page_ms = Math.round(now() - t0);
out.mem.one_local_page = memNow(child);

// set_viewport / reapply_viewport / capture_for_agent
await check('set_viewport', 'Emulation.setDeviceMetricsOverride 1280x800', async () => {
  await cdp.call('Emulation.setDeviceMetricsOverride', { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });
  return JSON.stringify(await evalv(cdp, '[innerWidth, innerHeight, devicePixelRatio]')) === '[1280,800,1]' || JSON.stringify(await evalv(cdp, '[innerWidth, innerHeight, devicePixelRatio]'));
});
await check('refresh_location / sync_location', 'Runtime.evaluate returnByValue', async () => JSON.stringify(await evalv(cdp, '[location.href, document.title]')) === JSON.stringify([`${base}/form`, 'form']));
await check('main_frame', 'Page.getFrameTree', async () => { const t = await cdp.call('Page.getFrameTree'); return !!t.frameTree?.frame?.id && (t.frameTree.childFrames?.length ?? 0) >= 1 ? true : `frame ok, child frames: ${t.frameTree?.childFrames?.length ?? 0}`; });
let ctxId = null;
await check('inject_autofill_and_capture', 'Page.createIsolatedWorld', async () => {
  const fid = (await cdp.call('Page.getFrameTree')).frameTree.frame.id;
  ctxId = (await cdp.call('Page.createIsolatedWorld', { frameId: fid, worldName: 'kwnt-spike' })).executionContextId;
  return typeof ctxId === 'number';
});
await check('inject_autofill_and_capture', 'Runtime.evaluate in isolated world (contextId)', async () => {
  await cdp.call('Runtime.evaluate', { expression: 'window.__kw = 1; document.getElementById("u").value="from-world"', contextId: ctxId, returnByValue: false });
  const pageSees = await evalv(cdp, 'typeof window.__kw');
  const domShared = await evalv(cdp, 'document.getElementById("u").value');
  await evalv(cdp, 'document.getElementById("u").value=""');
  return pageSees === 'undefined' && domShared === 'from-world' ? true : `page sees world global: ${pageSees !== 'undefined'}, DOM shared: ${domShared === 'from-world'}`;
});
await check('guard_app_links', 'Page.addScriptToEvaluateOnNewDocument (worldName, runImmediately)', async () => {
  const r = await cdp.call('Page.addScriptToEvaluateOnNewDocument', { source: 'document.addEventListener("DOMContentLoaded", () => { document.documentElement.dataset.kwg = "1" })', worldName: 'kwnt-guard', runImmediately: true });
  await navLoad(cdp, `${base}/form`);
  return !!r.identifier && (await evalv(cdp, 'document.documentElement.dataset.kwg')) === '1' ? true : `id=${r.identifier} ran on new doc: ${await evalv(cdp, 'document.documentElement.dataset.kwg')}`;
});
await check('capture_screenshot', 'Page.captureScreenshot jpeg q75', async () => { const r = await cdp.call('Page.captureScreenshot', { format: 'jpeg', quality: 75 }); save('local-form.jpg', r.data); const [w, h] = jpegSize(r.data); return w === 1280 && h === 800 ? true : `screenshot ${w}x${h}`; });
await check('capture_for_agent', 'setDeviceMetricsOverride 390 mobile + jpeg q70 + restore', async () => {
  await cdp.call('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true });
  await sleep(300);
  const w = await evalv(cdp, 'innerWidth');
  const r = await cdp.call('Page.captureScreenshot', { format: 'jpeg', quality: 70 });
  save('local-form-390.jpg', r.data);
  await cdp.call('Emulation.setDeviceMetricsOverride', { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });
  const [jw, jh] = jpegSize(r.data);
  return jw === 390 && jh === 844 ? true : `screenshot ${jw}x${jh}, innerWidth=${w}`;
});
await check('capture_for_agent at 390 without mobile flag (workaround probe)', 'setDeviceMetricsOverride 390 mobile:false + jpeg', async () => {
  await cdp.call('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: false });
  await sleep(300);
  const r = await cdp.call('Page.captureScreenshot', { format: 'jpeg', quality: 70 });
  await cdp.call('Emulation.setDeviceMetricsOverride', { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });
  const [jw, jh] = jpegSize(r.data);
  return jw === 390 && jh === 844 ? true : `screenshot ${jw}x${jh}`;
});
await check('capture_for_agent (no fixed viewport)', 'Emulation.clearDeviceMetricsOverride', async () => { await cdp.call('Emulation.clearDeviceMetricsOverride'); await cdp.call('Emulation.setDeviceMetricsOverride', { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false }); return true; });
await check('interact_click', 'Input.dispatchMouseEvent pressed/released at (200,100)', async () => {
  for (const type of ['mousePressed', 'mouseReleased']) await cdp.call('Input.dispatchMouseEvent', { type, x: 200, y: 100, button: 'left', clickCount: 1 });
  await sleep(150);
  const t = await evalv(cdp, 'document.title');
  return t === 'clicked 1280x800' ? true : `title=${t}`;
});
await check('interact_type', 'click field + Input.insertText', async () => {
  for (const type of ['mousePressed', 'mouseReleased']) await cdp.call('Input.dispatchMouseEvent', { type, x: 100, y: 260, button: 'left', clickCount: 1 });
  await cdp.call('Input.insertText', { text: 'octo-spike' });
  await sleep(150);
  const v = await evalv(cdp, 'document.getElementById("u").value');
  return v === 'octo-spike' ? true : `value=${JSON.stringify(v)} focused=${await evalv(cdp, 'document.activeElement && document.activeElement.id')}`;
});
await check('interact_key', 'Input.dispatchKeyEvent rawKeyDown/keyUp Tab', async () => {
  for (const type of ['rawKeyDown', 'keyUp']) await cdp.call('Input.dispatchKeyEvent', { type, key: 'Tab', code: 'Tab' });
  await sleep(150);
  const f = await evalv(cdp, 'document.activeElement && document.activeElement.id');
  return f === 'p' ? true : `focus after Tab=${f} (Chrome moves it to #p)`;
});
await check('interact_type (password)', 'Input.insertText into password', async () => { await cdp.call('Input.insertText', { text: 'not-a-real-pass' }); await sleep(100); return (await evalv(cdp, 'document.getElementById("p").value')) === 'not-a-real-pass'; });
await check('interact_key (bot, exact kwant params)', 'Input.dispatchKeyEvent rawKeyDown/keyUp Enter (key+code only)', async () => {
  const loaded = cdp.once('Page.loadEventFired', 3000);
  for (const type of ['rawKeyDown', 'keyUp']) await cdp.call('Input.dispatchKeyEvent', { type, key: 'Enter', code: 'Enter' });
  const l = await loaded; await sleep(200);
  return `form ${l ? 'submitted' : 'not submitted'} (title=${await evalv(cdp, 'document.title')})`;
});
if ((await evalv(cdp, 'document.title')) === 'submitted') { await navLoad(cdp, `${base}/form`); await evalv(cdp, 'document.getElementById("u").value="octo-spike";document.getElementById("p").value="not-a-real-pass";document.getElementById("p").focus()'); }
await check('live view Enter (browser_input.rs key_params)', 'Input.dispatchKeyEvent keyDown text \\r vk13 + keyUp submits the form', async () => {
  const loaded = cdp.once('Page.loadEventFired', 8000);
  await personEnter();
  await loaded; await sleep(200);
  const t = await evalv(cdp, 'document.title');
  const r = await evalv(cdp, 'document.getElementById("r") && document.getElementById("r").textContent');
  return t === 'submitted' && /u=octo-spike&p=not-a-real-pass/.test(r ?? '') ? true : `title=${t} body=${JSON.stringify(r)}`;
});
await check('navigate (session cookies)', 'server Set-Cookie kept and sent back', async () => { const r = await evalv(cdp, 'document.getElementById("r").textContent'); return /srv=1/.test(r ?? '') ? true : `cookie header=${r}`; });
await check('persist_cookies', 'Network.getAllCookies', async () => { const c = (await cdp.call('Network.getAllCookies')).cookies ?? []; return c.some((x) => x.name === 'srv') && c.some((x) => x.name === 'after') ? true : `got ${c.map((x) => x.name).join(',')}`; });
await check('restore_cookies', 'Network.setCookies then sent to the site', async () => {
  await cdp.call('Network.setCookies', { cookies: [{ name: 'restored', value: 'yes', domain: '127.0.0.1', path: '/', secure: false, httpOnly: true, sameSite: 'Lax', expires: Math.floor(Date.now() / 1000) + 3600 }] });
  await navLoad(cdp, `${base}/echo`);
  const r = await evalv(cdp, 'document.getElementById("r").textContent');
  return /restored=yes/.test(r ?? '') ? true : `cookie header=${r}`;
});
await check('leave_refused_page', 'Page.getNavigationHistory + navigateToHistoryEntry', async () => {
  const h = await cdp.call('Page.getNavigationHistory');
  const prev = h.entries[h.currentIndex - 1];
  const loaded = cdp.once('Page.loadEventFired', 8000);
  await cdp.call('Page.navigateToHistoryEntry', { entryId: prev.id });
  await loaded; await sleep(200);
  const t = await evalv(cdp, 'document.title');
  return t === prev.title || t === 'submitted' ? true : `entries=${h.entries.length} went to title=${t}, wanted ${prev.title}`;
});
await navLoad(cdp, `${base}/long`);
await check('interact_scroll', 'Input.dispatchMouseEvent mouseWheel deltaY 600', async () => {
  await cdp.call('Input.dispatchMouseEvent', { type: 'mouseWheel', x: 400, y: 400, deltaX: 0, deltaY: 600 });
  await sleep(400);
  const y = await evalv(cdp, 'scrollY');
  return y > 0 ? true : `scrollY=${y}`;
});
await check('extract_content / snapshot / list_clickables (JS reads)', 'Runtime.evaluate innerText + getBoundingClientRect', async () => {
  await navLoad(cdp, `${base}/form`);
  const r = await evalv(cdp, `(() => { const b = document.getElementById('b').getBoundingClientRect(); return [document.body.innerText.includes('press'), Math.round(b.width), Math.round(b.height), document.elementFromPoint(200,100)?.id]; })()`);
  return JSON.stringify(r) === JSON.stringify([true, 400, 200, 'b']) ? true : `innerText/rect/hit-test=${JSON.stringify(r)}`;
});
await check('click(selector) / click_ref (JS click)', 'Runtime.evaluate element.click()', async () => { await evalv(cdp, 'document.getElementById("b").click()'); return (await evalv(cdp, 'document.title')).startsWith('clicked'); });
await check('frame_texts', 'Runtime.evaluate reading same-origin iframe text', async () => (await evalv(cdp, 'document.querySelector("iframe").contentDocument?.body?.innerText ?? null')) === 'inner frame text');
await check('page errors', 'Page.navigate to unreachable host → errorText', async () => { const r = await cdp.call('Page.navigate', { url: 'http://127.0.0.1:1/' }); await sleep(300); return r.errorText ? true : `no errorText; result=${JSON.stringify(r).slice(0, 120)}`; });

// ---- outside browser.rs but on the same bots'-browser path (live view, site rules, sessions, dialogs) ----
await navLoad(cdp, `${base}/form`);
await check('live view (browser_stream.rs)', 'Page.startScreencast + frames + ack + stop', async () => {
  let frames = 0;
  const l = (m) => { if (m.method === 'Page.screencastFrame') { frames++; if (frames === 1) save('screencast-first.jpg', m.params.data); cdp.call('Page.screencastFrameAck', { sessionId: m.params.sessionId }).catch(() => {}); } };
  cdp.listeners.push(l);
  await cdp.call('Page.startScreencast', { format: 'jpeg', quality: 65, maxWidth: 1280, maxHeight: 800, everyNthFrame: 1 });
  for (let i = 0; i < 6; i++) { await evalv(cdp, `document.body.style.background = '${i % 2 ? '#fff' : '#eef'}'`); await sleep(500); }
  await cdp.call('Page.stopScreencast');
  cdp.listeners = cdp.listeners.filter((x) => x !== l);
  return frames >= 3 ? `yes (${frames} frames in 3 s)` : `${frames} frames in 3 s`;
});
await check('dialogs (browser_stream.rs)', 'Page.javascriptDialogOpening + handleJavaScriptDialog', async () => {
  const opened = cdp.once('Page.javascriptDialogOpening', 4000);
  cdp.call('Runtime.evaluate', { expression: 'setTimeout(() => document.getElementById("al").click(), 10)' }).catch(() => {});
  const d = await opened;
  if (!d) return 'no dialog event';
  await cdp.call('Page.handleJavaScriptDialog', { accept: true });
  await sleep(200);
  return (await evalv(cdp, 'document.title')) === 'after alert';
});
await check('bindings (progress/app_links)', 'Runtime.addBinding + bindingCalled', async () => {
  await cdp.call('Runtime.enable');
  await cdp.call('Runtime.addBinding', { name: 'kwSpike' });
  const got = cdp.once('Runtime.bindingCalled', 3000);
  await evalv(cdp, 'kwSpike("ping")');
  return (await got)?.params?.payload === 'ping';
});
await check('console (Log)', 'Log.enable', async () => { await cdp.call('Log.enable'); return true; });
await check('focus', 'Emulation.setFocusEmulationEnabled', async () => { await cdp.call('Emulation.setFocusEmulationEnabled', { enabled: true }); return true; });
await check('DOM nodes (browser_stream.rs)', 'DOM.describeNode/resolveNode + Runtime.callFunctionOn', async () => {
  const obj = (await cdp.call('Runtime.evaluate', { expression: 'document.getElementById("u")' })).result;
  const d = await cdp.call('DOM.describeNode', { objectId: obj.objectId });
  const r = await cdp.call('DOM.resolveNode', { backendNodeId: d.node.backendNodeId });
  const v = await cdp.call('Runtime.callFunctionOn', { objectId: r.object.objectId, functionDeclaration: 'function(){return this.id}', returnByValue: true });
  await cdp.call('Runtime.releaseObject', { objectId: obj.objectId });
  return v.result.value === 'u';
});

const bw = await new Cdp(version.webSocketDebuggerUrl).open();
await check('browser endpoint', 'Browser.getVersion', async () => !!(await bw.call('Browser.getVersion')).product);
await check('bots\' own sessions (service.rs)', 'Target.createBrowserContext/createTarget/closeTarget/disposeBrowserContext', async () => {
  const c = (await bw.call('Target.createBrowserContext', { disposeOnDetach: false })).browserContextId;
  const t = (await bw.call('Target.createTarget', { url: 'about:blank', browserContextId: c })).targetId;
  const list = await getJson('/json/list');
  const listed = list.some((x) => x.id === t);
  await bw.call('Target.closeTarget', { targetId: t });
  await bw.call('Target.disposeBrowserContext', { browserContextId: c });
  return listed ? true : 'target not in /json/list';
});
await check('bots\' own sessions (service.rs)', 'Target.createTarget newWindow', async () => { const t = (await bw.call('Target.createTarget', { url: 'about:blank', newWindow: true })).targetId; await bw.call('Target.closeTarget', { targetId: t }); return !!t; });
await check('site rules (site_guard.rs)', 'Target.attachToTarget flatten + Fetch.enable + requestPaused + failRequest/continueRequest + setAutoAttach', async () => {
  const s = (await bw.call('Target.attachToTarget', { targetId: page.id, flatten: true })).sessionId;
  await bw.call('Fetch.enable', { patterns: [{ urlPattern: '*', resourceType: 'Document', requestStage: 'Request' }] }, s);
  await bw.call('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: true, flatten: true });
  const seen = [];
  const l = (m) => { if (m.method === 'Fetch.requestPaused' && m.sessionId === s) { seen.push(m.params.request.url); const block = m.params.request.url.includes('/echo'); bw.call(block ? 'Fetch.failRequest' : 'Fetch.continueRequest', block ? { requestId: m.params.requestId, errorReason: 'BlockedByClient' } : { requestId: m.params.requestId }, s).catch(() => {}); } };
  bw.listeners.push(l);
  const blocked = await cdp.call('Page.navigate', { url: `${base}/echo` }).catch((e) => ({ errorText: 'CDP error reply: ' + e.message }));
  await sleep(300);
  const allowed = await cdp.call('Page.navigate', { url: `${base}/long` });
  await sleep(500);
  bw.listeners = bw.listeners.filter((x) => x !== l);
  await bw.call('Fetch.disable', {}, s).catch(() => {});
  await bw.call('Target.setAutoAttach', { autoAttach: false, waitForDebuggerOnStart: false, flatten: true }).catch(() => {});
  await bw.call('Target.detachFromTarget', { sessionId: s }).catch(() => {});
  const t = await evalv(cdp, 'document.title');
  if (!(seen.length >= 2 && !!blocked.errorText && t === 'long')) return `paused=${seen.length} blockedError=${blocked.errorText} title after allowed=${t}`;
  return blocked.errorText.startsWith('CDP error reply') ? `blocked, but as ${blocked.errorText} (Chrome: result.errorText)` : true;
});
out.mem.after_local_suite = memNow(child);

// ---------- real sites: load, read, screenshot ----------
const SITE_LIST = [
  ['example', 'https://example.com/'],
  ['wikipedia', 'https://en.wikipedia.org/wiki/Rust_(programming_language)'],
  ['hn', 'https://news.ycombinator.com/'],
  ['github-login', 'https://github.com/login'],
  ['google-signin', 'https://accounts.google.com/signin'],
  ['microsoft-login', 'https://login.microsoftonline.com/'],
  ['amazon-signin', 'https://www.amazon.com/ap/signin?openid.pape.max_auth_age=0&openid.return_to=https%3A%2F%2Fwww.amazon.com%2F&openid.identity=http%3A%2F%2Fspecs.openid.net%2Fauth%2F2.0%2Fidentifier_select&openid.assoc_handle=usflex&openid.mode=checkid_setup&openid.claimed_id=http%3A%2F%2Fspecs.openid.net%2Fauth%2F2.0%2Fidentifier_select&openid.ns=http%3A%2F%2Fspecs.openid.net%2Fauth%2F2.0'],
  ['x-login', 'https://x.com/i/flow/login'],
];
const READ = `(() => { const vis = (e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
  const inputs = [...document.querySelectorAll('input')].filter((i) => !['hidden'].includes(i.type));
  return { url: location.href, title: document.title, text: (document.body?.innerText ?? '').length,
    inputs: inputs.length, visibleInputs: inputs.filter(vis).length,
    password: inputs.some((i) => i.type === 'password'), email: inputs.some((i) => i.type === 'email' || /user|email|login|identifier/i.test(i.name + i.id + i.autocomplete)),
    buttons: document.querySelectorAll('button,[role=button],input[type=submit]').length }; })()`;
if (SITES) for (const [key, url] of SITE_LIST) {
  const t = now();
  const row = { key, url };
  try {
    await cdp.call('Page.enable');
    const loaded = cdp.once('Page.loadEventFired', 20000);
    const r = await cdp.call('Page.navigate', { url }, undefined, 25000);
    if (r.errorText) row.error = r.errorText;
    row.load_event = !!(await loaded);
    row.load_ms = Math.round(now() - t);
    await sleep(2500);
    Object.assign(row, await evalv(cdp, READ));
    const shot = await cdp.call('Page.captureScreenshot', { format: 'jpeg', quality: 70 }, undefined, 20000);
    save(`site-${key}.jpg`, shot.data);
    row.shot_bytes = Math.round(shot.data.length * 0.75);
  } catch (e) { row.error = String(e.message ?? e).slice(0, 160); }
  row.mem = memNow(child);
  out.sites.push(row);
}

// ---------- sign-in flows without real accounts (person-style input: click, type, Enter) ----------
async function clickSel(sel) {
  const p = await evalv(cdp, `(() => { const e = document.querySelector(${JSON.stringify(sel)}); if (!e) return null; e.scrollIntoView({block:'center'}); const r = e.getBoundingClientRect(); return [r.x + r.width/2, r.y + r.height/2]; })()`);
  if (!p) throw new Error('no element ' + sel);
  for (const type of ['mousePressed', 'mouseReleased']) await cdp.call('Input.dispatchMouseEvent', { type, x: p[0], y: p[1], button: 'left', clickCount: 1 });
}
const enter = () => personEnter();
async function flow(key, url, steps) {
  const row = { key };
  try {
    await navLoad(cdp, url, 25000).catch(() => {});
    await sleep(2500);
    await steps(row);
    await sleep(4000);
    const shot = await cdp.call('Page.captureScreenshot', { format: 'jpeg', quality: 70 }, undefined, 20000);
    save(`signin-${key}.jpg`, shot.data);
    row.after = await evalv(cdp, `({url: location.href, title: document.title, text: (document.body?.innerText ?? '').replace(/\\s+/g,' ').slice(0, 400)})`);
  } catch (e) { row.error = String(e.message ?? e).slice(0, 200); try { const shot = await cdp.call('Page.captureScreenshot', { format: 'jpeg', quality: 70 }); save(`signin-${key}.jpg`, shot.data); } catch {} }
  out.signin.push(row);
}
if (SITES) {
  // GitHub: wrong password must come back as GitHub's own "Incorrect username or password."
  await flow('github-wrong-password', 'https://github.com/login', async () => {
    await clickSel('#login_field'); await cdp.call('Input.insertText', { text: 'kwant-spike-nonexistent-user-4417' });
    await clickSel('#password'); await cdp.call('Input.insertText', { text: 'definitely-wrong-password' });
    await enter();
  });
  // Google: an email that does not exist must reach Google's "Couldn't find your Google Account" (not a browser refusal)
  await flow('google-unknown-email', 'https://accounts.google.com/signin', async () => {
    await clickSel('#identifierId, input[type=email], input[name=identifier], input[autocomplete=username]'); await cdp.call('Input.insertText', { text: 'kwant.spike.no.such.account.4417@gmail.com' });
    await enter();
  });
  // Microsoft: unknown account must reach "This username may be incorrect" / "We couldn't find an account"
  await flow('microsoft-unknown-email', 'https://login.microsoftonline.com/', async () => {
    await clickSel('input[type=email]'); await cdp.call('Input.insertText', { text: 'kwant.spike.no.such.account.4417@outlook.com' });
    await enter();
  });
}
out.mem.after_sites = memNow(child);
out.mem.peak_mib = Math.round(peak);

// ---------- stop ----------
await check('stop', 'Browser.close', async () => { await bw.call('Browser.close').catch(() => {}); await sleep(1500); return child.exitCode !== null || child.signalCode !== null ? true : 'still running (killed)'; });
clearInterval(sampler);
cdp.close(); bw.close();
try { process.kill(-child.pid, 'SIGKILL'); } catch {}
try { if (process.platform === 'win32') execFileSync('taskkill', ['/F', '/T', '/PID', String(child.pid)]); } catch {}
server.close();
fs.rmSync(profile, { recursive: true, force: true });
fs.writeFileSync(path.join(outdir, 'result.json'), JSON.stringify(out, null, 2));
const pass = out.results.filter((r) => r.works && r.effect.startsWith('yes')).length;
console.log(`${name}: start ${out.start_ms_json_version} ms (DevTools up), ${out.start_to_first_page_ms} ms to first page; idle ${Math.round(out.mem.idle.mib)} MiB ${out.mem.idle.kind}, peak ${out.mem.peak_mib} MiB; ${pass}/${out.results.length} checks fully OK`);
for (const r of out.results) console.log(`  ${r.works ? (r.effect.startsWith('yes') ? 'OK  ' : 'PART') : 'FAIL'} ${r.area} | ${r.method} | ${r.effect}`);
for (const s of out.sites) console.log(`  site ${s.key}: ${s.error ?? ''} load=${s.load_ms}ms title=${JSON.stringify(s.title)} text=${s.text} inputs=${s.visibleInputs}/${s.inputs} pw=${s.password} mem=${Math.round(s.mem?.mib ?? 0)}`);
for (const s of out.signin) console.log(`  signin ${s.key}: ${s.error ?? ''} ${JSON.stringify(s.after)}`);
process.exit(0);
