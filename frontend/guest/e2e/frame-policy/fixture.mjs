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
// app code runs, and gives the app the only two calls style-guide.md section 3 allows across the frame boundary
// (Mira's K1 ask): kwant.onState(fn) wraps the message listener and checks type==='state'; kwant.act(name, body)
// posts {type:"act",name,body} to the parent. With these in place, K2's lint can ban `postMessage` and `parent` as
// banned tokens outright in the app's own script (style-guide.md section 7, J1), since the app never needs them.
const PRELUDE_JS = `(()=>{
for(const k of ['RTCPeerConnection','webkitRTCPeerConnection','mozRTCPeerConnection','RTCDataChannel']){try{delete window[k]}catch(e){}}
window.__kwantPreludeRan=true;
window.kwant={
  onState:function(fn){
    window.addEventListener('message',function(e){
      if(e.source!==window.parent)return;
      var d=e.data;
      if(d&&d.type==='state')fn(d);
    });
  },
  act:function(name,body){
    window.parent.postMessage({type:'act',name:name,body:body},'*');
  }
};
})();`;

// kwant's pocket.css (a constant <style>): a stand-in for the real one, just enough to prove the style-src hash.
const POCKET_CSS = `:root{color-scheme:dark}body{margin:0;font:14px system-ui,sans-serif;background:#120e26;color:#f4f1ff}`;

// The app's own style (hash-pinned alongside pocket.css, per section 3: "style-src 'sha256-<pocket.css>' 'sha256-<app css>'").
const APP_CSS = `.k1{padding:8px;color:#8fd14f}`;

// The app's own body markup (not hash-pinned: markup isn't script or style, so CSP doesn't gate it).
const APP_BODY = `<div class="k1" id="k1">pa12 K1 probe</div>`;

// A real, decodable 1x1 PNG (QA fix 1, Rigel's K1 qa.md): the previous 4 raw bytes weren't a loadable image in
// any engine, so onerror fired whether or not CSP actually blocked the request - the img checks passed either
// way. With a real image, a blocked load and a successful decode are distinguishable again.
const PNG_1x1 = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=',
  'base64',
);

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

    // 2b. QA (Rigel, pa12 K1 round 1): the three checks above can't tell script-src's hash allow-list apart from
    // Trusted Types - TT throws on the raw string before script-src is ever consulted (this file's own comment
    // above: "before CSP's hash check even applies"). Mint a TrustedScript through a policy CSP does allow (there's
    // no \`trusted-types\` directive restricting policy names, so createPolicy succeeds): Trusted Types raises no
    // objection to a TrustedScript, so whatever stops this from running can only be CSP.
    const ttAvailable = typeof window.trustedTypes !== 'undefined';
    if (ttAvailable) {
      try {
        const pol = trustedTypes.createPolicy('qa-csp-proof', { createScript: (s) => s });
        window.__qaUnlistedRan = false;
        try {
          const s2 = document.createElement('script');
          s2.textContent = pol.createScript('window.__qaUnlistedRan = true;');
          document.head.appendChild(s2);
          await new Promise((res) => setTimeout(res, 50));
          pass('unlisted-script-blocked-independent-of-tt', window.__qaUnlistedRan !== true, \`ran=\${window.__qaUnlistedRan}\`);
        } catch (e) {
          pass('unlisted-script-blocked-independent-of-tt', true, \`blocked building the element: \${String(e && e.message || e)}\`);
        }
        mustThrow('eval-blocked-independent-of-tt', () => eval(pol.createScript('1+1')));
      } catch (e) {
        pass('unlisted-script-blocked-independent-of-tt', false, \`trustedTypes.createPolicy itself failed: \${String(e && e.message || e)}\`);
        pass('eval-blocked-independent-of-tt', false, \`trustedTypes.createPolicy itself failed: \${String(e && e.message || e)}\`);
      }
    } else {
      pass('unlisted-script-blocked-independent-of-tt', true, 'not applicable: this engine has no Trusted Types to bypass, so the base check above is already CSP-only');
      pass('eval-blocked-independent-of-tt', true, 'not applicable: this engine has no Trusted Types to bypass, so the base check above is already CSP-only');
    }
    // (The Function-constructor form isn't repeated here: with a TT-minted script, Chromium's
    // \`new Function(trustedScript)\` doesn't reliably fail even when script-src is weakened - see frame-policy.md's
    // mutation table - so it can't be trusted as an independent proof; eval is kept as the one CSP-only check for
    // the "execute via a constructor, not a <script> element" path.)

    // 3. fetch, WebSocket, sendBeacon and <img> blocked to an outside URL and to this same origin (connect-src
    // 'none' / img-src data: blob: don't carve out an exception for same-origin). QA (Rigel): a cross-origin fetch
    // or WebSocket to a server that sends no CORS header / never completes the upgrade handshake fails anyway, with
    // or without connect-src blocking it - so these must not be judged by whether the call threw or rejected. Like
    // sendBeacon, frame-policy.mjs decides pass/fail from whether the mock servers' own access/upgrade logs ever saw
    // the request actually reach them - the one place a request dropped before it left the browser is
    // distinguishable from one that merely failed after arriving.
    try { fetch(foreign + '/fetch-outside').catch(() => {}); } catch (e) { /* the log decides, not this catch */ }
    try { fetch(location.origin + '/fetch-same').catch(() => {}); } catch (e) { /* the log decides */ }
    const tryWebSocket = (mk) => { try { const ws = mk(); ws.onerror = () => {}; } catch (e) { /* the log decides */ } };
    tryWebSocket(() => new WebSocket(foreign.replace('http', 'ws') + '/ws-outside'));
    tryWebSocket(() => new WebSocket(location.origin.replace('http', 'ws') + '/ws-same'));
    // Give the fetches and the WebSocket handshake attempts time to either reach the mock servers or be dropped by
    // CSP before frame-policy.mjs reads the servers' logs.
    await new Promise((res) => setTimeout(res, 300));
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

    // 7. The bridge (style-guide.md section 3, Mira's K1 ask): kwant.onState/kwant.act are now the only two calls
    // across the frame (the prelude's job), so this proves they actually carry a state down and an act up through
    // this exact sandbox+CSP header set - not just that the two functions exist. The parent (parent.js) posts a
    // {type:"state"} message repeatedly until this check settles, since this probe reaches the bridge section at an
    // unpredictable time relative to the parent's own 'load' event.
    let bridgeState = null;
    window.kwant.onState((s) => { bridgeState = s; });
    window.kwant.act('qa-bridge-act', { ping: 1 });
    await new Promise((resolve) => {
      let waited = 0;
      const iv = setInterval(() => {
        waited += 50;
        if (bridgeState || waited >= 3000) { clearInterval(iv); resolve(); }
      }, 50);
    });
    pass(
      'bridge-onstate-received',
      !!(bridgeState && bridgeState.type === 'state' && bridgeState.marker === 'k1-state'),
      JSON.stringify(bridgeState),
    );

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

// The parent's bridge side (style-guide.md section 3): posts one `{type:"state"}` message down to the frame and
// records any `{type:"act"}` message the frame posts back. Today's pocket CSP (TODAY_CSP) has no script-src of its
// own, so default-src 'self' governs it - an inline <script> would violate it; a same-origin external file doesn't,
// matching how the real guest page is built. Sends repeatedly, not once: the app runs its checks sequentially with
// several awaited timeouts, so kwant.onState's listener can register well after this script's own 'load' handler
// would have already fired a single, one-shot send.
const PARENT_JS = `window.__receivedActs = [];
window.addEventListener('message', function (e) {
  var f = document.getElementById('app');
  if (!f || e.source !== f.contentWindow) return;
  window.__receivedActs.push(e.data);
});
window.addEventListener('load', function () {
  var f = document.getElementById('app');
  var n = 0;
  var iv = setInterval(function () {
    n += 1;
    f.contentWindow.postMessage({ type: 'state', marker: 'k1-state', view: {} }, '*');
    if (n >= 60) clearInterval(iv);
  }, 50);
});`;

const parentHtml = (appUrl) => `<!doctype html><html><head><meta charset="utf-8"><title>K1 parent</title></head>
<body><div id="parent">parent</div>
<iframe id="app" sandbox="allow-scripts" allow="" referrerpolicy="no-referrer" src="${appUrl}"></iframe>
<script src="/parent.js"></script>
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
    if (req.url === '/foreign.png') return send(res, 200, PNG_1x1, 'image/png');
    if (req.url === '/nav-landed') return send(res, 200, '<!doctype html><title>landed</title>landed', 'text/html; charset=utf-8');
    send(res, 404, 'not found', 'text/plain');
  });
  // A plain http.Server with no 'upgrade' listener silently destroys a WebSocket handshake request without
  // emitting anything - so without this, a handshake CSP let through and one CSP blocked both end up with zero
  // log entries, and the "server logs decide" check (QA fix 2, Rigel's K1 qa.md) would pass either way. Logging
  // here and then destroying the socket keeps the server a mock (never a real echo target) while still recording
  // that the handshake actually reached it.
  foreignServer.on('upgrade', (req, socket) => { log.push(`foreign UPGRADE ${req.url}`); socket.destroy(); });
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
    if (path === '/main.png') return send(res, 200, PNG_1x1, 'image/png');
    if (path === '/parent.js') return send(res, 200, PARENT_JS, 'text/javascript; charset=utf-8');
    send(res, 404, 'not found', 'text/plain');
  });
  mainServer.on('upgrade', (req, socket) => { log.push(`main UPGRADE ${req.url}`); socket.destroy(); });
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
