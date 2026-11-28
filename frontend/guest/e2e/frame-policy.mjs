// pa12 K1: the app-document frame policy (security-review.md section 3-4), proved in real browsers, not just read
// off the header text. Runs every check in frame-policy/fixture.mjs's probe against Chromium always, and against
// WebKit when playwright-core's webkit is installed on this machine; if it isn't, says so and moves on (the K1
// brief: "if it doesn't, say so and list the exact Safari-on-Mac steps for the Mac CI or Oleks's Mac, don't block
// on it"). Writes <out>/frame-policy-results.json and fails (non-zero exit, listing every problem) if a check that
// should hold didn't, in any engine that ran.
// Usage: PW_CORE=<playwright-core/index.mjs> CHROMIUM=<chrome> node guest/e2e/frame-policy.mjs <out dir>
import fs from 'node:fs';
import path from 'node:path';
import { startFramePolicyFixture, TODAY_CSP } from './frame-policy/fixture.mjs';

const pw = await import(process.env.PW_CORE || 'playwright-core');
const out = path.resolve(process.argv[2] || 'guest-shots');
fs.mkdirSync(out, { recursive: true });

const problems = [];
const engineResults = {};
// Checks where "pass" must be true for K1 to say the policy holds. (trusted-types-enforced is judged against
// whatever the engine supports: see the per-engine note below, not a flat pass/fail here.)
const REQUIRED = [
  'opaque-origin', 'no-parent-dom', 'no-cookie-write', 'no-local-storage', 'no-session-storage',
  'unlisted-script-blocked', 'eval-blocked', 'function-ctor-blocked',
  'fetch-outside-blocked', 'fetch-same-origin-blocked', 'websocket-outside-blocked', 'websocket-same-origin-blocked',
  'sendbeacon-outside-blocked', 'sendbeacon-same-origin-blocked', 'img-outside-blocked', 'img-same-origin-blocked',
  'rtc-deleted', 'rtc-fresh-realm-blocked',
];

async function runEngine(engineName, launcher) {
  let browser;
  try {
    browser = await launcher();
  } catch (e) {
    engineResults[engineName] = { skipped: true, reason: String(e && e.message || e) };
    return;
  }
  const fixture = await startFramePolicyFixture();
  const result = { csp: { parent: fixture.parentCsp, app: fixture.appCsp }, checks: null, violations: null, navigation: null, canary: null };
  try {
    const ctx = await browser.newContext();
    const parentViolations = [];
    await ctx.addInitScript(() => {
      window.__parentViolations = [];
      document.addEventListener('securitypolicyviolation', (e) => window.__parentViolations.push(`${e.violatedDirective} ${e.blockedURI || ''}`.trim()));
    });
    const page = await ctx.newPage();
    const consoleLog = [];
    page.on('console', (m) => { if (/Content.Security.Policy|Refused to|Trusted Types|require-trusted-types/i.test(m.text())) consoleLog.push(m.text()); });
    const thirdParty = [];
    page.on('request', (r) => {
      const u = r.url();
      if (!u.startsWith('http://127.0.0.1:') && u !== 'about:blank') thirdParty.push(u);
    });

    // Canary: the detector itself. If an inline style we never hash-pinned doesn't get reported as a violation on
    // the APP document, the rest of this run proves nothing (same discipline as guest/e2e/csp.mjs's own canary).
    // Through the real parent, so the app only ever runs the way it will in production: inside the sandboxed iframe.
    await page.goto(`${fixture.origin}/parent`);
    const appHandle = await page.waitForSelector('#app');
    const appFrame = await appHandle.contentFrame();
    await appFrame.waitForFunction(() => window.__k1 && window.__k1.done === true, null, { timeout: 15000 });
    await appFrame.evaluate(() => {
      const s = document.createElement('style');
      s.textContent = 'body{outline:1px solid red}';
      document.head.appendChild(s);
    });
    await page.waitForTimeout(150);
    const canaryViolations = await appFrame.evaluate(() => window.__k1.violations.slice());
    result.canary = { sawStyleViolation: canaryViolations.some((v) => v.startsWith('style-src')) };
    if (!result.canary.sawStyleViolation) problems.push(`${engineName}: canary: an inline style we never hashed was not reported, the CSP check is not working`);

    const checks = await appFrame.evaluate(() => window.__k1.checks);
    const violations = await appFrame.evaluate(() => window.__k1.violations.slice());
    const appOrigin = await appFrame.evaluate(() => self.origin);
    const sendBeaconOutsideErr = await appFrame.evaluate(() => window.__k1.sendBeaconOutsideErr);
    const sendBeaconSameErr = await appFrame.evaluate(() => window.__k1.sendBeaconSameErr);
    // sendBeacon's own return value lies (connect-src 'none' still reports success to the caller), so the real
    // signal is whether the mock server the request targeted ever logged it.
    const beaconOutsideHits = fixture.log.filter((l) => l.startsWith('foreign') && l.includes('/beacon-outside')).length;
    const beaconSameHits = fixture.log.filter((l) => l.startsWith('main') && l.includes('/beacon-same')).length;
    checks['sendbeacon-outside-blocked'] = { pass: beaconOutsideHits === 0, detail: `server hits=${beaconOutsideHits}${sendBeaconOutsideErr ? ` (threw: ${sendBeaconOutsideErr})` : ''}` };
    checks['sendbeacon-same-origin-blocked'] = { pass: beaconSameHits === 0, detail: `server hits=${beaconSameHits}${sendBeaconSameErr ? ` (threw: ${sendBeaconSameErr})` : ''}` };
    result.checks = checks;
    result.violations = violations;
    result.appOrigin = appOrigin;
    result.consoleLog = consoleLog;
    result.thirdPartyRequests = thirdParty.filter((u) => !u.startsWith(fixture.origin) && !u.startsWith(fixture.foreign));

    for (const name of REQUIRED) {
      const c = checks[name];
      if (!c) { problems.push(`${engineName}: missing check ${name}`); continue; }
      if (!c.pass) problems.push(`${engineName}: ${name} failed: ${c.detail}`);
    }
    if (result.thirdPartyRequests.length) problems.push(`${engineName}: a request left the two loopback origins: ${result.thirdPartyRequests.join(', ')}`);

    const tt = checks['trusted-types-supported'];
    const ttEnforced = checks['trusted-types-enforced'];
    result.trustedTypes = tt?.pass ? (ttEnforced?.pass ? 'supported and enforced' : 'supported but NOT enforced (gap)') : 'not supported by this engine';
    if (tt?.pass && !ttEnforced?.pass) problems.push(`${engineName}: Trusted Types is supported but require-trusted-types-for did not block the innerHTML write`);

    await ctx.close();

    // Navigation: a separate one-shot document, so a failure here can't cost the main probe its results.
    const navCtx = await browser.newContext();
    await navCtx.addInitScript(() => {
      window.__parentViolations = [];
      document.addEventListener('securitypolicyviolation', (e) => window.__parentViolations.push(`${e.violatedDirective} ${e.blockedURI || ''}`.trim()));
    });
    const navPage = await navCtx.newPage();
    await navPage.goto(`${fixture.origin}/parent-nav`);
    const navHandle = await navPage.waitForSelector('#app');
    await navPage.waitForTimeout(500);
    const navFrame = await navHandle.contentFrame();
    const navFrameUrl = navFrame ? navFrame.url() : null;
    const navParentViolations = await navPage.evaluate(() => window.__parentViolations.slice());
    const landedOnForeign = fixture.log.filter((l) => l.startsWith('foreign') && l.includes('/nav-landed')).length;
    result.navigation = { navFrameUrl, navParentViolations, landedOnForeign };
    // The authoritative signal is the foreign server's own access log: did the navigation ever reach it. Where the
    // frame's document ends up varies by engine - staying on /app-nav, or landing on an internal network-error page
    // (e.g. Chromium's chrome-error://chromewebdata/) because the request was blocked before it left the browser -
    // and both are a pass as long as the foreign host never saw the request.
    const stayedPut = landedOnForeign === 0;
    if (!stayedPut) problems.push(`${engineName}: self-navigation-blocked failed: frame ended at ${navFrameUrl}, foreign saw ${landedOnForeign} request(s) for /nav-landed`);
    result.checks['self-navigation-blocked'] = { pass: stayedPut, detail: `frame=${navFrameUrl} foreignHits=${landedOnForeign} frame-src violations=${navParentViolations.filter((v) => v.startsWith('frame-src')).length}` };
    await navCtx.close();
  } finally {
    await fixture.close();
    await browser.close();
  }
  engineResults[engineName] = result;
}

const chromiumExec = process.env.CHROMIUM || undefined;
await runEngine('chromium', () => pw.chromium.launch({ executablePath: chromiumExec, args: ['--no-sandbox'] }));

const webkitExec = process.env.WEBKIT || undefined;
await runEngine('webkit', () => pw.webkit.launch({ executablePath: webkitExec }));

fs.writeFileSync(path.join(out, 'frame-policy-results.json'), JSON.stringify({ today_csp: TODAY_CSP, engines: engineResults }, null, 2));

if (engineResults.webkit?.skipped) {
  console.log(`webkit: not run here (${engineResults.webkit.reason}). On a Mac (CI's mac job, or Oleks's Mac): open Safari, enable the Develop menu (Safari > Settings > Advanced > Show features for web developers), open the two fixture URLs by running this script's fixture.mjs separately and visiting /parent and /parent-nav, and read the Console for CSP violation lines and the Storage/Network tabs for the same checks frame-policy.mjs runs; frame-policy.md records the result by hand in that case.`);
}

console.log(JSON.stringify({ problems: problems.length, engines: Object.fromEntries(Object.entries(engineResults).map(([k, v]) => [k, v.skipped ? 'skipped' : (problems.some((p) => p.startsWith(k)) ? 'FAIL' : 'pass')])) }, null, 2));

if (problems.length) {
  console.error('\nfailures:');
  for (const p of problems) console.error(`  ${p}`);
  process.exit(1);
}
