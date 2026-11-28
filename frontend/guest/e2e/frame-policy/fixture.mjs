// pa12 K1: a static fixture that serves exactly the app-document headers from
// autonomy/pocket-apps/pa12/security-review.md section 3 (the ones L5 will serve) and a parent page that adds
// section 3's `frame-src 'self'; object-src 'none'; worker-src 'self'` to today's pocket CSP (src/pocket/guest.rs CSP
// const, kept here as TODAY_CSP so a change to the real constant doesn't silently drift). No src/ code runs here: this
// is fixtures and evidence only, proving the header set holds in a real browser (frame-policy.mjs drives it).
//
// Two origins, both loopback, so the "outside" checks need no real network: `main` serves the parent and the app
// document; `foreign` stands in for a different site (fetch/img/navigate targets use it, never a real host).
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';

// Today's pocket CSP (src/pocket/guest.rs `CSP`, frontend/guest/fake/views.mjs `CSP`). Copied, not imported: K1 must
// notice if the two drift, not silently inherit whichever one changed.
export const TODAY_CSP =
  "default-src 'self'; connect-src 'self'; img-src 'self' data: blob:; style-src 'self'; frame-ancestors 'none'; form-action 'self'; base-uri 'none'";

const sha256 = (s) => createHash('sha256').update(s, 'utf8').digest('base64');

// kwant's prelude (a constant <script>, security-review.md section 3): removes WebRTC's constructors before any
// app code runs. A real L5 prelude will do more (the postMessage bridge, P1); K1 only needs the one thing the
// review calls out as CSP-proof: "K1 must prove it, because CSP alone doesn't cover WebRTC."
const PRELUDE_JS = `(()=>{for(const k of ['RTCPeerConnection','webkitRTCPeerConnection','mozRTCPeerConnection','RTCDataChannel']){try{delete window[k]}catch(e){}}window.__kwantPreludeRan=true})();`;

// kwant's pocket.css (a constant <style>): a stand-in for the real one, just enough to prove the style-src hash.
const POCKET_CSS = `:root{color-scheme:dark}body{margin:0;font:14px system-ui,sans-serif;background:#120e26;color:#f4f1ff}`;

// The app's own style (hash-pinned alongside pocket.css, per section 3: "style-src 'sha256-<pocket.css>' 'sha256-<app css>'").
const APP_CSS = `.k1{padding:8px;color:#8fd14f}`;

// The app's own body markup (not hash-pinned: markup isn't script or style, so CSP doesn't gate it).
const APP_BODY = `<div class="k1" id="k1">pa12 K1 probe</div>`;

/**
 * The app's own script: every check security-review.md section 3/4 and the K1 brief ask for, run as genuine in-page
 * code (never from Playwright's privileged evaluate, which Chromium's devtools-style Runtime.evaluate exempts from
 * `unsafe-eval` and would make the eval/script checks lie). Results land in `window.__k1`, which frame-policy.mjs
 * only *reads* afterwards. `foreign` is baked in at build time (fixture.mjs starts the foreign server first).
 */
const appJs = (foreign) => `(() => {
  // Baked in at build time as a real browser-side constant (not a bare reference to the outer Node.js template
  // parameter, which a plain "foreign" token below would otherwise be - the generated script runs in the browser,
  // long after fixture.mjs's own "foreign" variable is out of scope).
  const foreign = ${JSON.stringify(foreign)};
  const r = { checks: {}, violations: [], done: false };
  window.__k1 = r;
  document.addEventListener('securitypolicyviolation', (e) => {
    r.violations.push(\`\${e.violatedDirective} \${e.blockedURI || ''}\`.trim());
  });
  const pass = (name, ok, detail) => { r.checks[name] = { pass: !!ok, detail: detail ?? '' }; };
  // For a check where throwing IS the desired, blocked outcome (eval, the Function constructor, WebSocket to a
  // connect-src 'none' target): pass only if it threw.
  const mustThrow = (name, fn) => {
    try {
      const v = fn();
      pass(name, false, 'did not throw' + (v !== undefined ? \`: \${String(v)}\` : ''));
    } catch (e) {
      pass(name, true, String(e && e.message || e));
    }
  };
  // For a check where the resource is blocked either by a thrown exception (sandboxed opaque origin, no
  // allow-same-origin: Chromium throws SecurityError just reading the property) or by the value silently never
  // persisting (another engine might not throw, but must still not let the write stick): either counts as blocked.
  // fn() returns true if, having not thrown, it confirmed the write did NOT persist; false if it did persist.
  const blockedEitherWay = (name, fn) => {
    try {
      const notPersisted = fn();
      pass(name, notPersisted, notPersisted ? 'no exception, and nothing persisted' : 'no exception, and the value persisted');
    } catch (e) {
      pass(name, true, \`threw: \${String(e && e.message || e)}\`);
    }
  };

  async function run() {
    // 1. Opaque origin: no access to the parent's DOM, cookies or storage.
    pass('opaque-origin', self.origin === 'null', self.origin);
    mustThrow('no-parent-dom', () => window.parent.document);
    blockedEitherWay('no-cookie-write', () => { document.cookie = 'k1=1'; return !document.cookie.includes('k1=1'); });
    blockedEitherWay('no-local-storage', () => { localStorage.setItem('k1', '1'); return localStorage.getItem('k1') !== '1'; });
    blockedEitherWay('no-session-storage', () => { sessionStorage.setItem('k1', '1'); return sessionStorage.getItem('k1') !== '1'; });

    // 2. Inline script allow-list: the hash-pinned scripts above already ran (prelude + this one). An unlisted
    // inline script, and eval, must not run. Building the unlisted <script> itself can be blocked two ways: Trusted
    // Types (require-trusted-types-for 'script' makes HTMLScriptElement.textContent a sink that throws on a raw
    // string) stops it before it is even a parsed script; where TT isn't enforced, script-src's hash allow-list
    // stops it from running once it's in the document. Either is a pass.
    window.__unlistedScriptRan = false;
    try {
      const s = document.createElement('script');
      s.textContent = 'window.__unlistedScriptRan = true;';
      document.head.appendChild(s);
      await new Promise((res) => setTimeout(res, 50));
      pass('unlisted-script-blocked', window.__unlistedScriptRan !== true, \`ran=\${window.__unlistedScriptRan}\`);
    } catch (e) {
      pass('unlisted-script-blocked', true, \`blocked building the element: \${String(e && e.message || e)}\`);
    }
    mustThrow('eval-blocked', () => eval('1+1'));
    mustThrow('function-ctor-blocked', () => new Function('return 1')());

    // 3. fetch, WebSocket, sendBeacon and <img> blocked to an outside URL and to this same origin (connect-src
    // 'none' / img-src data: blob: don't carve out an exception for same-origin).
    await (async () => { try { await fetch(foreign + '/foreign-echo'); pass('fetch-outside-blocked', false, 'resolved'); } catch (e) { pass('fetch-outside-blocked', true, String(e)); } })();
    await (async () => { try { await fetch(location.origin + '/main-echo'); pass('fetch-same-origin-blocked', false, 'resolved'); } catch (e) { pass('fetch-same-origin-blocked', true, String(e)); } })();
    // WebSocket under connect-src 'none': some engines throw synchronously at construction, others let the socket
    // open and then fail the connection asynchronously (never firing onopen). Either is "blocked"; only an actual
    // open event is a failure.
    const connectionBlocked = (name, mk) => new Promise((resolve) => {
      let ws;
      try { ws = mk(); } catch (e) { pass(name, true, \`threw constructing: \${String(e && e.message || e)}\`); resolve(); return; }
      let settled = false;
      const finish = (ok, detail) => { if (settled) return; settled = true; pass(name, ok, detail); resolve(); };
      ws.onopen = () => finish(false, 'opened');
      ws.onerror = () => finish(true, 'error event, never opened');
      ws.onclose = () => finish(true, 'close event, never opened');
      setTimeout(() => finish(true, 'timed out without opening'), 1500);
    });
    await connectionBlocked('websocket-outside-blocked', () => new WebSocket(foreign.replace('http', 'ws') + '/'));
    await connectionBlocked('websocket-same-origin-blocked', () => new WebSocket(location.origin.replace('http', 'ws') + '/'));
    // sendBeacon's return value isn't a reliable signal: connect-src 'none' lets the call return true and then
    // silently drops the request at the network layer, same as it does for a blocked fetch that never dials out. So
    // this only records that the call didn't throw (on a unique path per target); frame-policy.mjs decides pass/fail
    // from the mock servers' own access logs, the only place a dropped-vs-sent request is actually visible.
    try { navigator.sendBeacon(foreign + '/beacon-outside', 'x'); r.sendBeaconOutsideErr = null; } catch (e) { r.sendBeaconOutsideErr = String(e && e.message || e); }
    try { navigator.sendBeacon(location.origin + '/beacon-same', 'x'); r.sendBeaconSameErr = null; } catch (e) { r.sendBeaconSameErr = String(e && e.message || e); }
    // <img>: a blocked load fires 'error' in every engine tried so far, but a timeout is treated the same as an
    // 'error' (the resource never loaded, which is the property under test), so a check is never left unset even if
    // some engine neither fires an event nor loads the image.
    await new Promise((resolve) => {
      let left = 2;
      const done = () => { if (--left <= 0) resolve(); };
      const settleImg = (name, ok, detail) => { if (!r.checks[name]) { pass(name, ok, detail); done(); } };
      const imgOut = new Image(); imgOut.onload = () => settleImg('img-outside-blocked', false, 'loaded'); imgOut.onerror = () => settleImg('img-outside-blocked', true, 'onerror'); imgOut.src = foreign + '/foreign.png';
      const imgSame = new Image(); imgSame.onload = () => settleImg('img-same-origin-blocked', false, 'loaded'); imgSame.onerror = () => settleImg('img-same-origin-blocked', true, 'onerror'); imgSame.src = location.origin + '/main.png';
      setTimeout(() => settleImg('img-outside-blocked', true, 'timed out without loading'), 2000);
      setTimeout(() => settleImg('img-same-origin-blocked', true, 'timed out without loading'), 2000);
    });

    // 5. Trusted Types, where the engine has it (require-trusted-types-for 'script' is in the CSP; an engine
    // without Trusted Types just ignores the directive, which is noted, not failed).
    const ttSupported = typeof window.trustedTypes !== 'undefined';
    pass('trusted-types-supported', ttSupported, '');
    if (ttSupported) {
      mustThrow('trusted-types-enforced', () => { document.getElementById('k1').innerHTML = '<b>x</b>'; });
    } else {
      pass('trusted-types-enforced', true, 'not applicable: this engine has no Trusted Types support to enforce');
    }

    // 6. The prelude deleted RTCPeerConnection; a fresh realm (an about:blank child frame) must not get it back.
    pass('rtc-deleted', typeof window.RTCPeerConnection === 'undefined' && typeof window.webkitRTCPeerConnection === 'undefined', '');
    await new Promise((resolve) => {
      const f = document.createElement('iframe');
      let settled = false;
      const settle = (ok, detail) => { if (settled) return; settled = true; pass('rtc-fresh-realm-blocked', ok, detail); resolve(); };
      f.onload = () => {
        try {
          const has = typeof f.contentWindow.RTCPeerConnection !== 'undefined';
          settle(!has, has ? 'RTCPeerConnection reachable in nested frame' : 'nested frame has no RTCPeerConnection');
        } catch (e) { settle(true, \`nested frame inaccessible: \${e}\`); }
      };
      document.body.appendChild(f);
      setTimeout(() => settle(true, 'nested frame never loaded (frame-src none held)'), 1500);
    });

    r.done = true;
  }
  run().catch((e) => { r.error = String(e && e.stack || e); r.done = true; });
})();`;

function appCsp(prelude, appJsSrc) {
  return [
    "sandbox allow-scripts",
    "default-src 'none'",
    `script-src 'sha256-${sha256(prelude)}' 'sha256-${sha256(appJsSrc)}'`,
    `style-src 'sha256-${sha256(POCKET_CSS)}' 'sha256-${sha256(APP_CSS)}'`,
    "img-src data: blob:",
    "font-src data:",
    "connect-src 'none'",
    "media-src 'none'",
    "frame-src 'none'",
    "worker-src 'none'",
    "object-src 'none'",
    "form-action 'none'",
    "base-uri 'none'",
    "frame-ancestors 'self'",
    "require-trusted-types-for 'script'",
  ].join('; ');
}

const appHeaders = (csp) => ({
  'Content-Security-Policy': csp,
  'Permissions-Policy': "camera=(), microphone=(), geolocation=(), payment=(), usb=(), serial=(), bluetooth=(), hid=(), publickey-credentials-get=(), publickey-credentials-create=(), clipboard-read=(), display-capture=(), screen-wake-lock=(), web-share=(), fullscreen=()",
  'X-Content-Type-Options': 'nosniff',
  'Referrer-Policy': 'no-referrer',
  'X-DNS-Prefetch-Control': 'off',
  'Cross-Origin-Resource-Policy': 'same-origin',
  'Cache-Control': 'private, no-cache',
  'Content-Type': 'text/html; charset=utf-8',
});

const parentCsp = `${TODAY_CSP}; frame-src 'self'; object-src 'none'; worker-src 'self'`;

const parentHtml = (appUrl) => `<!doctype html><html><head><meta charset="utf-8"><title>K1 parent</title></head>
<body><div id="parent">parent</div>
<iframe id="app" sandbox="allow-scripts" allow="" referrerpolicy="no-referrer" src="${appUrl}"></iframe>
</body></html>`;

function appHtml(foreign) {
  const appJsSrc = appJs(foreign);
  const csp = appCsp(PRELUDE_JS, appJsSrc);
  const html = `<!doctype html><html><head><meta charset="utf-8">
<style>${POCKET_CSS}</style><style>${APP_CSS}</style>
</head><body>${APP_BODY}
<script>${PRELUDE_JS}</script>
<script>${appJsSrc}</script>
</body></html>`;
  return { html, csp };
}

// 4. A separate, one-shot app document: it tries to navigate itself away the instant it loads, nothing else. Kept
// apart from the main probe so a gap here (the frame actually leaving) can't also cost every other check its result.
const appNavJs = (foreign) => `(() => {
  window.__navAttempted = true;
  try { location.href = '${foreign}/nav-landed'; } catch (e) { window.__navError = String(e); }
})();`;

function appNavHtml(foreign) {
  const appJsSrc = appNavJs(foreign);
  const csp = appCsp(PRELUDE_JS, appJsSrc);
  const html = `<!doctype html><html><head><meta charset="utf-8">
<style>${POCKET_CSS}</style></head><body>${APP_BODY}
<script>${PRELUDE_JS}</script>
<script>${appJsSrc}</script>
</body></html>`;
  return { html, csp };
}

export async function startFramePolicyFixture() {
  const log = [];
  // The foreign origin first: the app document's script needs its address baked in before its hash is computed.
  const foreignServer = createServer((req, res) => {
    log.push(`foreign ${req.method} ${req.url}`);
    if (req.url === '/foreign-echo') return send(res, 200, 'ok', 'text/plain');
    if (req.url === '/foreign.png') return send(res, 200, Buffer.from([0x89, 0x50, 0x4e, 0x47]), 'image/png');
    if (req.url === '/nav-landed') return send(res, 200, '<!doctype html><title>landed</title>landed', 'text/html; charset=utf-8');
    send(res, 404, 'not found', 'text/plain');
  });
  await new Promise((resolve) => foreignServer.listen(0, '127.0.0.1', resolve));
  const foreignPort = foreignServer.address().port;
  const foreign = `http://127.0.0.1:${foreignPort}`;

  const { html: appDoc, csp: appHeaderCsp } = appHtml(foreign);
  const { html: appNavDoc, csp: appNavCsp } = appNavHtml(foreign);

  const mainServer = createServer((req, res) => {
    log.push(`main ${req.method} ${req.url}`);
    const path = req.url.split('?')[0];
    if (path === '/parent') return send(res, 200, parentHtml('/app'), 'text/html; charset=utf-8', { 'Content-Security-Policy': parentCsp });
    if (path === '/parent-nav') return send(res, 200, parentHtml('/app-nav'), 'text/html; charset=utf-8', { 'Content-Security-Policy': parentCsp });
    if (path === '/app') return send(res, 200, appDoc, undefined, appHeaders(appHeaderCsp));
    if (path === '/app-nav') return send(res, 200, appNavDoc, undefined, appHeaders(appNavCsp));
    if (path === '/main-echo') return send(res, 200, 'ok', 'text/plain');
    if (path === '/main.png') return send(res, 200, Buffer.from([0x89, 0x50, 0x4e, 0x47]), 'image/png');
    send(res, 404, 'not found', 'text/plain');
  });
  await new Promise((resolve) => mainServer.listen(0, '127.0.0.1', resolve));
  const mainPort = mainServer.address().port;

  function send(res, status, body, type, extra = {}) {
    const headers = { ...(type ? { 'Content-Type': type } : {}), ...extra };
    res.writeHead(status, headers);
    res.end(body);
  }

  return {
    log,
    origin: `http://127.0.0.1:${mainPort}`,
    foreign,
    appCsp: appHeaderCsp,
    parentCsp,
    close: async () => {
      await new Promise((r) => mainServer.close(r));
      await new Promise((r) => foreignServer.close(r));
    },
  };
}
