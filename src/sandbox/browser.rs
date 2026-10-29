use futures::{SinkExt, StreamExt};
use super::credentials::CredentialStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, RwLock, OwnedRwLockWriteGuard};
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// The DevTools WebSocket request for `url`. Chrome refuses a DevTools connection whose Host is a
/// name other than localhost (500 on the upgrade), so a plain-ws name such as the compose service
/// `browser` gets `Host: localhost`, as the `/json` requests do. IPs and wss (a hosted service
/// that may need its own Host) are left alone.
pub(crate) fn cdp_ws_request(url: &str) -> Result<tokio_tungstenite::tungstenite::handshake::client::Request, BrowserError> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut req = url.into_client_request().map_err(|e| BrowserError::Ws(e.to_string()))?;
    let named_host = req.uri().scheme_str() == Some("ws")
        && req.uri().host().is_some_and(|h| h != "localhost" && h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_err());
    if named_host {
        req.headers_mut().insert(reqwest::header::HOST, reqwest::header::HeaderValue::from_static("localhost"));
    }
    Ok(req)
}

/// Opens a DevTools WebSocket (see [`cdp_ws_request`]).
pub(crate) async fn connect_cdp(url: &str) -> Result<WsStream, BrowserError> {
    let (ws, _) = connect_async(cdp_ws_request(url)?).await.map_err(|e| BrowserError::Ws(e.to_string()))?;
    Ok(ws)
}

/// `function(needle)` evaluated in the page. Collects visible interactive elements (incl. open
/// shadow roots and same-origin iframes), topmost first. With a needle, picks the best label match
/// and either returns its viewport center (when nothing covers it) or clicks it directly.
const CLICKABLES_JS: &str = r#"function (needle) {
  var SEL = 'button,a[href],[role=button],[role=link],[role=menuitem],[role=tab],[role=checkbox],[role=option],input,textarea,select,summary,label,[onclick]';
  var items = [];
  function clean(s) { return (s || '').replace(/\s+/g, ' ').trim().slice(0, 80); }
  function labelOf(el) {
    var tag = el.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') {
      var t = (el.type || '').toLowerCase();
      if (t === 'submit' || t === 'button') return clean(el.value);
      if (t === 'hidden') return '';
      var name = clean(el.getAttribute('aria-label') || el.placeholder || el.name || el.id);
      return name ? name + ' (' + (t === 'checkbox' || t === 'radio' ? t : 'field') + ')' : '';
    }
    return clean(el.innerText || el.getAttribute('aria-label') || el.title || el.value);
  }
  function walk(root, ox, oy) {
    var all;
    try { all = root.querySelectorAll('*'); } catch (e) { return; }
    for (var i = 0; i < all.length; i++) {
      var el = all[i];
      if (el.shadowRoot) walk(el.shadowRoot, ox, oy);
      if (el.tagName === 'IFRAME') {
        try { var d = el.contentDocument; if (d) { var fr = el.getBoundingClientRect(); walk(d, ox + fr.left, oy + fr.top); } } catch (e) {}
        continue;
      }
      if (!el.matches(SEL)) continue;
      var r = el.getBoundingClientRect();
      if (r.width < 2 || r.height < 2) continue;
      var cs = getComputedStyle(el);
      if (cs.visibility === 'hidden' || cs.display === 'none' || Number(cs.opacity) === 0) continue;
      var label = labelOf(el);
      if (!label) continue;
      var lx = r.left + r.width / 2, ly = r.top + r.height / 2;
      var x = ox + lx, y = oy + ly;
      var inView = x >= 0 && y >= 0 && x <= innerWidth && y <= innerHeight;
      var onTop = false;
      if (inView) {
        var hit = root.elementFromPoint ? root.elementFromPoint(lx, ly) : null;
        onTop = !hit || hit === el || el.contains(hit) || hit.contains(el);
      }
      items.push({ label: label, x: Math.round(x), y: Math.round(y), inView: inView, onTop: onTop, el: el });
    }
  }
  walk(document, 0, 0);
  items.sort(function (a, b) { return (b.onTop - a.onTop) || (b.inView - a.inView); });
  var match = null;
  if (needle) {
    var n = clean(needle).toLowerCase(), best = 0;
    for (var j = 0; j < items.length; j++) {
      var l = items[j].label.toLowerCase();
      var s = l === n ? 3 : l.indexOf(n) === 0 ? 2 : l.indexOf(n) >= 0 ? 1 : 0;
      if (s > best) { best = s; match = items[j]; }
    }
    if (match) {
      var m = match.el;
      if (!match.onTop) {
        m.scrollIntoView({ block: 'center' });
        if (typeof m.focus === 'function') m.focus();
        m.click();
        match = { label: match.label, clicked: true };
      } else {
        match = { label: match.label, x: match.x, y: match.y, clicked: false };
      }
    }
  }
  return { items: items.map(function (i) { return { label: i.label, onTop: i.onTop }; }), match: match };
}"#;

#[derive(Error, Debug)]
pub enum BrowserError {
    #[error("HTTP error connecting to CDP: {0}")]
    Http(String),
    #[error("WebSocket connection to browser failed: {0}")]
    Ws(String),
    #[error("Protocol error: {0}")]
    Protocol(String),
    #[error("Operation timed out")]
    Timeout,
    #[error("Browser is controlled by a person. Retry after control is returned.")]
    HumanControl,
    /// The browser works, but the page itself could not be loaded (DNS, TLS, refused, ...).
    #[error("Page could not be loaded: {0}")]
    Navigation(String),
    /// A file path (or file:// address) handed to the browser; see [`refuse_file_path`].
    #[error("{0}")]
    NotWebAddress(String),
}

/// Refuses an address that is a file on the computer before it reaches Chrome, so it is never
/// read as a page on the site already open. file:// is refused in every mode: in Local mode it
/// would show the bot any file on the person's computer, past the folders and approvals its
/// file tools keep to.
pub fn refuse_file_path(target: &str) -> Result<(), BrowserError> {
    let t = target.trim();
    if t.get(..5).is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:")) {
        return Err(BrowserError::NotWebAddress(format!(
            "{t} is a file:// address, which the browser doesn't open. Read local files with your file tools \
             (Read, or cat in the shell); the browser opens http:// and https:// pages."
        )));
    }
    let b = t.as_bytes();
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/');
    let path = drive || ["/", "~", "\\", "./", "../", ".\\", "..\\"].iter().any(|p| t.starts_with(p));
    if path {
        return Err(BrowserError::NotWebAddress(format!(
            "{t} is a file path on the computer, not a web address, so the browser didn't open it. \
             Read local files with your file tools (Read, or cat in the shell); the browser opens \
             http:// and https:// pages."
        )));
    }
    Ok(())
}

/// Recognises CAPTCHA / anti-bot interstitials that replace the requested content.
pub fn detect_bot_wall(url: &str, title: &str, text: &str) -> Option<&'static str> {
    let url = url.to_lowercase();
    let title = title.to_lowercase();
    let text = text.to_lowercase();
    if url.contains("google.") && url.contains("/sorry/") || text.contains("unusual traffic from your computer network") {
        return Some("Google \"unusual traffic\" CAPTCHA");
    }
    if title.starts_with("just a moment") || text.contains("checking your browser") || text.contains("checking if the site connection is secure") {
        return Some("Cloudflare bot check");
    }
    let captcha_markers = ["i'm not a robot", "i am not a robot", "are you a robot", "verify you are human", "verifying you are human", "prove you're human", "complete the captcha"];
    if captcha_markers.iter().any(|m| text.contains(m) || title.contains(m)) || url.contains("captcha") {
        return Some("CAPTCHA / human verification");
    }
    None
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?; }
    }
    let tmp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    let mut file = options.open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)
}

const FOCUSED_EDITABLE_JS: &str = r#"(() => {
  let el = document.activeElement;
  for (let i = 0; el && i < 8; i++) {
    if (el.shadowRoot && el.shadowRoot.activeElement) { el = el.shadowRoot.activeElement; continue; }
    if (el.tagName === 'IFRAME' || el.tagName === 'FRAME') {
      try { const inner = el.contentDocument && el.contentDocument.activeElement; if (inner) { el = inner; continue; } } catch (e) {}
      return false;
    }
    break;
  }
  if (!el) return false;
  if (el.isContentEditable) return true;
  if (el.tagName === 'TEXTAREA') return !el.readOnly && !el.disabled;
  if (el.tagName !== 'INPUT') return false;
  const t = (el.type || 'text').toLowerCase();
  return !el.readOnly && !el.disabled && ['text','search','email','url','tel','password','number','date','datetime-local','month','time','week'].includes(t);
})()"#;

/// Finds a visible CAPTCHA widget in the page (document, open shadow roots and same-origin
/// frames) and returns `{kind, x, y, width, height}` in viewport pixels, or null. Never scrolls:
/// a widget below the fold comes back with `offscreen: true` and no usable box.
///
/// Look-alikes are skipped: a reCAPTCHA v2 checkbox only counts while its challenge (bframe) is
/// open or the page is otherwise a wall (no field to fill, little text), and phrases like
/// "press and hold" only count inside an element whose id or class looks like a CAPTCHA.
pub const CAPTCHA_PROBE_JS: &str = r##"(function () {
  var VENDORS = [
    ['recaptcha', /(^|\.)google\.com\/recaptcha\/|recaptcha\.net\/recaptcha\//i],
    ['hcaptcha', /(^|\/\/|\.)hcaptcha\.com\//i],
    ['turnstile', /challenges\.cloudflare\.com\//i],
    ['arkose', /arkoselabs\.com|funcaptcha\.com/i],
    ['datadome', /captcha-delivery\.com/i],
    ['geetest', /geetest\.com/i]
  ];
  var SELECTORS = [
    ['geetest', '.geetest_panel, .geetest_box, .geetest_holder, .geetest_widget, .geetest_captcha'],
    ['arkose', '#FunCaptcha, #arkose, [data-e2e="enforcement-frame"]'],
    ['perimeterx', '#px-captcha'],
    ['datadome', '#ddv1-captcha-container, .captcha__human'],
    ['tiktok', '#captcha_container, #captcha-verify-container-main-page, .captcha_verify_container, .captcha-verify-container, [class*="captcha_verify"], [class*="captcha-verify"], [id^="tiktok-verify"]']
  ];
  var PHRASES = /drag the slider|drag the puzzle piece|slide to complete the puzzle|drag the puzzle|slide to verify|select all images with|press (and|&) hold/i;
  // ids/classes a real challenge box carries; a cookie or newsletter dialog doesn't
  var CAPTCHA_LIKE = /captcha|challenge|verif(y|ication)|secsdk|geetest|(^|[\s_-])(px|dd)[-_]/;
  var vw = window.innerWidth, vh = window.innerHeight;
  function visible(el, rect) {
    if (rect.width < 30 || rect.height < 30) return false;
    var st = getComputedStyle(el);
    return st.display !== 'none' && st.visibility !== 'hidden' && parseFloat(st.opacity || '1') > 0.05;
  }
  function frameOffset(win) {
    var x = 0, y = 0;
    try {
      while (win && win !== window && win.frameElement) {
        var r = win.frameElement.getBoundingClientRect();
        x += r.left; y += r.top; win = win.parent;
      }
    } catch (e) {}
    return [x, y];
  }
  function box(kind, el) {
    var r = el.getBoundingClientRect();
    if (!visible(el, r)) return null;
    var off = frameOffset(el.ownerDocument.defaultView);
    var left = r.left + off[0], top = r.top + off[1];
    var x = Math.max(0, left), y = Math.max(0, top);
    var w = Math.min(vw, left + r.width) - x, h = Math.min(vh, top + r.height) - y;
    if (w < 10 || h < 10) return {kind: kind, offscreen: true, x: 0, y: 0, width: 0, height: 0};
    return {kind: kind, x: Math.round(x), y: Math.round(y), width: Math.round(w), height: Math.round(h)};
  }
  function captchaLike(n) {
    var tag = ((n.id || '') + ' ' + (typeof n.className === 'string' ? n.className : '')).toLowerCase();
    return CAPTCHA_LIKE.test(tag);
  }
  function container(el) {
    for (var n = el, i = 0; n && i < 12; i++) {
      if (n.nodeType === 1 && captchaLike(n)) return n;
      n = n.parentNode || n.host;
    }
    return null;
  }
  // A page that is nothing but the check (Google's "sorry" page, a bare verify form): no field a
  // person would fill in and little text. A contact form or a page with a footer form isn't.
  function wallPage() {
    var fields = document.querySelectorAll('input, textarea, select');
    for (var i = 0; i < fields.length; i++) {
      var t = (fields[i].getAttribute('type') || 'text').toLowerCase();
      if (fields[i].tagName === 'INPUT' && /^(hidden|submit|button|image|reset|checkbox|radio)$/.test(t)) continue;
      var r = fields[i].getBoundingClientRect();
      if (r.width > 0 && r.height > 0) return false;
    }
    return ((document.body && document.body.innerText) || '').trim().length < 1000;
  }
  var roots = [];
  function collect(root, depth) {
    if (!root || depth > 6) return;
    roots.push(root);
    var all = root.querySelectorAll('*');
    for (var i = 0; i < all.length; i++) {
      var el = all[i];
      if (el.shadowRoot) collect(el.shadowRoot, depth + 1);
      if (el.tagName === 'IFRAME' || el.tagName === 'FRAME') {
        try { if (el.contentDocument) collect(el.contentDocument, depth + 1); } catch (e) {}
      }
    }
  }
  collect(document, 0);
  var found = null, anchor = null;
  for (var ri = 0; ri < roots.length && !found; ri++) {
    var frames = roots[ri].querySelectorAll('iframe');
    for (var f = 0; f < frames.length && !found; f++) {
      var src = frames[f].getAttribute('src') || '';
      if (frames[f].closest && frames[f].closest('.grecaptcha-badge')) continue;
      if (/recaptcha\/(api2|enterprise)\/anchor.*[?&]size=invisible/i.test(src)) continue;
      for (var v = 0; v < VENDORS.length; v++) {
        if (!VENDORS[v][1].test(src)) continue;
        if (VENDORS[v][0] === 'recaptcha') {
          if (/recaptcha\/(api2|enterprise)\/bframe/i.test(src)) {
            // Google parks a closed challenge at top:-10000px and hides it
            var b = box('recaptcha', frames[f]);
            if (b && frames[f].getBoundingClientRect().top + window.scrollY > -1000) found = b;
          } else if (!anchor) {
            anchor = frames[f];
          }
        } else {
          found = box(VENDORS[v][0], frames[f]);
        }
        break;
      }
    }
  }
  if (!found && anchor && wallPage()) found = box('recaptcha', anchor);
  for (var ri2 = 0; ri2 < roots.length && !found; ri2++) {
    for (var s = 0; s < SELECTORS.length && !found; s++) {
      var els = roots[ri2].querySelectorAll(SELECTORS[s][1]);
      for (var e = 0; e < els.length && !found; e++) found = box(SELECTORS[s][0], els[e]);
    }
  }
  for (var ri3 = 0; ri3 < roots.length && !found; ri3++) {
    var root = roots[ri3];
    var walker = (root.ownerDocument || root).createTreeWalker(root, NodeFilter.SHOW_TEXT);
    var node;
    while (!found && (node = walker.nextNode())) {
      if (!PHRASES.test(node.nodeValue || '')) continue;
      var c = container(node.parentNode);
      if (c) found = box(/tiktok/i.test(location.hostname) ? 'tiktok' : 'puzzle', c);
    }
  }
  return found;
})()"##;

pub const CAPTCHA_KINDS: &[&str] = &["recaptcha", "hcaptcha", "turnstile", "arkose", "geetest", "datadome", "perimeterx", "tiktok", "puzzle"];

/// Where a CAPTCHA widget sits in the viewport, so the person's view can point at it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptchaHit {
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// The widget is on the page but outside the viewport (the probe never scrolls), so the box
    /// is empty and shouldn't be pointed at.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub offscreen: bool,
}

impl CaptchaHit {
    /// Validates what the page returned: a known kind and a finite box inside the viewport, or an
    /// off-screen widget without a box.
    pub fn parse(value: &Value) -> Option<Self> {
        let kind = value.get("kind")?.as_str()?;
        if !CAPTCHA_KINDS.contains(&kind) { return None; }
        if value.get("offscreen").and_then(Value::as_bool) == Some(true) {
            return Some(Self { kind: kind.to_string(), x: 0.0, y: 0.0, width: 0.0, height: 0.0, offscreen: true });
        }
        let num = |k: &str| value.get(k).and_then(Value::as_f64).filter(|n| n.is_finite());
        let (x, y) = (num("x")?.clamp(0.0, 1280.0), num("y")?.clamp(0.0, 800.0));
        let width = num("width")?.clamp(0.0, 1280.0 - x);
        let height = num("height")?.clamp(0.0, 800.0 - y);
        if width < 1.0 || height < 1.0 { return None; }
        Some(Self { kind: kind.to_string(), x, y, width, height, offscreen: false })
    }

    /// The box, when it's on screen and worth pointing the person's view at.
    pub fn on_screen(self) -> Option<Self> {
        (!self.offscreen).then_some(self)
    }

    pub fn label(&self) -> &'static str {
        match self.kind.as_str() {
            "recaptcha" => "reCAPTCHA",
            "hcaptcha" => "hCaptcha",
            "turnstile" => "Cloudflare Turnstile check",
            "arkose" => "Arkose (FunCaptcha) puzzle",
            "geetest" => "GeeTest puzzle",
            "datadome" => "DataDome check",
            "perimeterx" => "PerimeterX press-and-hold check",
            "tiktok" => "TikTok slider puzzle",
            _ => "slider / puzzle CAPTCHA",
        }
    }
}

/// Clicks only "reject" / "decline optional" in known cookie banners (OneTrust, Cookiebot,
/// Didomi, Usercentrics, Quantcast, Google, TikTok) and in containers that are clearly a
/// consent banner. Never accepts. Returns the clicked label or null.
pub const CONSENT_REJECT_JS: &str = r##"(function () {
  var KNOWN = ['#onetrust-reject-all-handler', '#CybotCookiebotDialogBodyButtonDecline',
    '#didomi-notice-disagree-button', '[data-testid="uc-deny-all-button"]',
    '.qc-cmp2-summary-buttons button[mode="secondary"]', '#truste-consent-required',
    '.cmpboxbtnno', '#cmpwelcomebtnno a', '.fc-cta-do-not-consent', 'button#W0wltc'];
  var REJECT = /^\s*(reject( all| optional)?( cookies)?|decline( all| optional)?( cookies)?|deny( all)?|refuse( all)?|only (necessary|essential|required)( cookies)?|use necessary cookies only|necessary cookies only|continue without accepting|alle ablehnen|ablehnen|tout refuser|refuser|rechazar( todo)?|rifiuta( tutto)?|weiger(en)?|odrzuć|відхилити|отклонить)\s*$/i;
  var ACCEPT = /accept|allow|agree|consent|got it|^\s*ok\s*$|akzept|accepter|aceptar|accetta|zustimmen/i;
  var NEGATED = /\b(not|don't|reject|decline|deny|refuse|necessary|essential|required)\b|ablehnen|refuser|rechazar|rifiuta/i;
  var BANNER = /cookie|consent|gdpr|cmp|privacy|onetrust|didomi|usercentrics|cookiebot|qc-cmp|truste/i;
  function label(el) { return (el.innerText || el.textContent || el.getAttribute('aria-label') || el.value || '').replace(/\s+/g, ' ').trim(); }
  function shown(el) { var r = el.getBoundingClientRect(); var st = getComputedStyle(el); return r.width > 0 && r.height > 0 && st.visibility !== 'hidden' && st.display !== 'none'; }
  function click(el) { var l = label(el); if (ACCEPT.test(l) && !NEGATED.test(l)) return null; el.click(); return l || 'reject'; }
  var roots = [];
  (function collect(root, d) { if (!root || d > 4) return; roots.push(root);
    var all = root.querySelectorAll('*');
    for (var i = 0; i < all.length; i++) { if (all[i].shadowRoot) collect(all[i].shadowRoot, d + 1);
      if (all[i].tagName === 'IFRAME') { try { if (all[i].contentDocument) collect(all[i].contentDocument, d + 1); } catch (e) {} } }
  })(document, 0);
  for (var r = 0; r < roots.length; r++) {
    for (var k = 0; k < KNOWN.length; k++) {
      var el = roots[r].querySelector(KNOWN[k]);
      if (el && shown(el)) { var c = click(el); if (c) return c; }
    }
  }
  for (var r2 = 0; r2 < roots.length; r2++) {
    var host = roots[r2].host;
    var buttons = roots[r2].querySelectorAll('button, [role="button"], a[href="#"], input[type="button"]');
    for (var b = 0; b < buttons.length; b++) {
      var btn = buttons[b], text = label(btn);
      if (!REJECT.test(text) || !shown(btn)) continue;
      var inBanner = false;
      for (var n = btn, i = 0; n && i < 10; i++, n = n.parentNode || n.host) {
        if (n.nodeType !== 1) continue;
        var tag = (n.id || '') + ' ' + (typeof n.className === 'string' ? n.className : '') + ' ' + (n.getAttribute('aria-label') || '') + ' ' + n.tagName;
        if (BANNER.test(tag)) { inBanner = true; break; }
      }
      if (!inBanner && host) inBanner = BANNER.test(host.tagName + ' ' + (host.id || '') + ' ' + (host.className || ''));
      if (inBanner) { btn.click(); return text; }
    }
  }
  return null;
})()"##;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigationResult {
    pub url: String,
    pub title: String,
    pub status_code: u16,
}

/// A password the person typed into a page, waiting for them to confirm saving it. The origin is
/// the frame's real URL as Chrome reports it, never anything the page says about itself.
#[derive(Clone, Serialize, PartialEq)]
pub struct PendingCapture {
    pub origin: String,
    pub username: String,
    #[serde(skip)]
    pub password: String,
    pub captured_at: String,
}

impl std::fmt::Debug for PendingCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingCapture")
            .field("origin", &self.origin)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

/// The store and owner (team, channel or bot id) a browser fills passwords from.
#[derive(Clone)]
pub struct Vault {
    pub store: Arc<dyn CredentialStore>,
    pub owner: String,
}

/// Name of the isolated world kwnt's autofill and capture run in: the page's own scripts share
/// the DOM but not its JavaScript globals, so they can't read or forge the capture.
const KWNT_WORLD: &str = "kwnt-autofill";

/// Runs in [`KWNT_WORLD`]; `window` here is that world's own global, not the page's.
const AUTOFILL_SCRIPT: &str = r#"
(function(creds) {
    var USER = 'input[type="text"], input[type="email"], input:not([type]), input[name*="user"], input[name*="login"], input[name*="email"], input[id*="user"], input[id*="login"]';
    function visible(el) {
        var r = el.getBoundingClientRect();
        var s = window.getComputedStyle(el);
        return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none';
    }
    function postsHere(input) {
        var f = input.form;
        if (!f || !f.getAttribute('action')) return true;
        try { return new URL(f.action, location.href).origin === location.origin; } catch (e) { return false; }
    }
    function set(input, value) {
        input.value = value;
        input.dispatchEvent(new Event('input', { bubbles: true }));
        input.dispatchEvent(new Event('change', { bubbles: true }));
    }
    function fill() {
        if (!creds || location.origin !== creds.origin) return;
        var pass = Array.prototype.find.call(document.querySelectorAll('input[type="password"]'), function(p) {
            return visible(p) && postsHere(p);
        });
        if (!pass || pass.value) return;
        var scope = pass.form || document;
        var user = Array.prototype.find.call(scope.querySelectorAll(USER), visible);
        if (user && !user.value && creds.username) set(user, creds.username);
        set(pass, creds.password);
    }
    fill();
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', fill);
    setTimeout(fill, 600);
    setTimeout(fill, 1800);

    if (window.__kwntCaptureInstalled) return;
    window.__kwntCaptureInstalled = true;
    function record(scope) {
        if (!scope) return;
        var pass = scope.querySelector('input[type="password"]');
        if (!pass || !pass.value) return;
        var user = scope.querySelector(USER);
        window.__kwntCaptured = { username: user ? user.value : '', password: pass.value };
    }
    document.addEventListener('submit', function(e) { record(e.target); }, true);
    document.addEventListener('click', function(e) {
        var btn = e.target.closest && e.target.closest('button, input[type="submit"], [role="button"]');
        if (btn) record(btn.closest('form') || document);
    }, true);
})(__KWNT_CREDS__);
"#;

const COLLECT_SCRIPT: &str = r#"
(function() {
    var c = window.__kwntCaptured;
    window.__kwntCaptured = null;
    return c ? JSON.stringify(c) : null;
})()
"#;

#[derive(Clone)]
pub struct BrowserController {
    pub cdp_port: u16,
    pub current_url: Arc<Mutex<Option<String>>>,
    pub current_title: Arc<Mutex<Option<String>>>,
    pub last_screenshot: Arc<Mutex<Option<String>>>,
    vault: Arc<Mutex<Option<Vault>>>,
    /// A password captured on the page, waiting for the person's OK.
    pending_capture: Arc<Mutex<Option<PendingCapture>>>,
    /// kwnt's isolated world in the current document.
    capture_world: Arc<Mutex<Option<i64>>>,
    msg_id: Arc<AtomicU64>,
    ws_conn: Arc<Mutex<Option<WsStream>>>,
    endpoint: String,
    target_id: Option<String>,
    browser_level: bool,
    pub control: Arc<RwLock<()>>,
    /// Held while a human step (sign in, CAPTCHA) waits on the person: bot actions get
    /// `HumanControl` and the person's live-view input goes through without taking control.
    held_for_person: Arc<std::sync::Mutex<Option<OwnedRwLockWriteGuard<()>>>>,
    /// Where the left button is held down from live-view pointer input, if it is.
    pointer_held: Arc<std::sync::Mutex<Option<(f64, f64)>>>,
    /// VM mode: this bot's cookies, saved outside the throwaway browser context.
    cookie_jar: Arc<std::sync::Mutex<Option<std::path::PathBuf>>>,
}

impl std::fmt::Debug for BrowserController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserController")
            .field("cdp_port", &self.cdp_port)
            .finish()
    }
}

impl BrowserController {
    pub fn new(cdp_port: u16) -> Self {
        Self {
            cdp_port,
            current_url: Arc::new(Mutex::new(None)),
            current_title: Arc::new(Mutex::new(None)),
            last_screenshot: Arc::new(Mutex::new(None)),
            vault: Arc::new(Mutex::new(None)),
            pending_capture: Arc::new(Mutex::new(None)),
            capture_world: Arc::new(Mutex::new(None)),
            msg_id: Arc::new(AtomicU64::new(1)),
            ws_conn: Arc::new(Mutex::new(None)),
            endpoint: format!("http://127.0.0.1:{cdp_port}"),
            target_id: None,
            browser_level: false,
            control: Arc::new(RwLock::new(())),
            held_for_person: Arc::default(),
            pointer_held: Arc::default(),
            cookie_jar: Arc::default(),
        }
    }

    /// Fills and captures passwords for `owner` from `store`.
    pub async fn attach_vault(&self, store: Arc<dyn CredentialStore>, owner: &str) {
        *self.vault.lock().await = Some(Vault { store, owner: owner.to_string() });
    }

    pub fn at_endpoint(endpoint: &str, target_id: Option<String>, browser_level: bool) -> Self {
        let mut ctrl = Self::new(0);
        ctrl.endpoint = endpoint.trim_end_matches('/').to_string();
        ctrl.target_id = target_id;
        ctrl.browser_level = browser_level;
        ctrl
    }

    pub async fn take_control(&self) -> Result<OwnedRwLockWriteGuard<()>, BrowserError> {
        self.control.clone().try_write_owned().map_err(|_| BrowserError::HumanControl)
    }

    /// Locks the browser to the person until [`Self::release_person_hold`]. Waits briefly for a
    /// bot action in flight; `false` if it couldn't (the person already has control, say).
    pub async fn hold_for_person(&self) -> bool {
        if self.is_held_for_person() {
            return true;
        }
        let Ok(guard) = timeout(std::time::Duration::from_secs(5), self.control.clone().write_owned()).await else {
            return false;
        };
        *self.held_for_person.lock().unwrap_or_else(|e| e.into_inner()) = Some(guard);
        true
    }

    pub fn release_person_hold(&self) {
        self.held_for_person.lock().unwrap_or_else(|e| e.into_inner()).take();
    }

    pub fn is_held_for_person(&self) -> bool {
        self.held_for_person.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    pub fn is_managed_session(&self) -> bool { self.target_id.is_some() }

    pub(crate) fn pointer_held_slot(&self) -> &std::sync::Mutex<Option<(f64, f64)>> { &self.pointer_held }

    pub async fn refresh_location(&self) -> Result<(), BrowserError> {
        let result = self.send_cdp_command("Runtime.evaluate", json!({
            "expression": "({url: location.href, title: document.title})", "returnByValue": true
        })).await?;
        if let Some(url) = result["result"]["value"]["url"].as_str() {
            *self.current_url.lock().await = Some(url.to_string());
        }
        if let Some(title) = result["result"]["value"]["title"].as_str() {
            *self.current_title.lock().await = Some(title.to_string());
        }
        Ok(())
    }

    // Chromium advertises its own hostname; the configured endpoint may be a Docker service or proxy.
    fn reachable_ws_url(&self, advertised: &str) -> Result<String, BrowserError> {
        if self.cdp_port != 0 { return Ok(advertised.to_string()); } // legacy/test direct endpoints
        let mut url = reqwest::Url::parse(&self.endpoint).map_err(|e| BrowserError::Protocol(e.to_string()))?;
        let target = reqwest::Url::parse(advertised).map_err(|e| BrowserError::Protocol(e.to_string()))?;
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme).map_err(|_| BrowserError::Protocol("Invalid CDP scheme".into()))?;
        url.set_path(target.path());
        url.set_query(target.query());
        Ok(url.to_string())
    }

    pub async fn get_ws_url(&self) -> Result<String, BrowserError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| BrowserError::Http(e.to_string()))?;

        // 1. Try /json/list to get active Page target
        let list_url = format!("{}/json/list", self.endpoint);
        if !self.browser_level {
        if let Ok(resp) = client.get(&list_url).header(reqwest::header::HOST, "localhost").send().await {
            if let Ok(targets) = resp.json::<Vec<Value>>().await {
                if let Some(page) = targets.into_iter().find(|t| t["type"] == "page" &&
                    self.target_id.as_ref().is_none_or(|id| t["id"] == *id)) {
                    if let Some(ws) = page["webSocketDebuggerUrl"].as_str() {
                        return self.reachable_ws_url(ws);
                    }
                }
            }
        }
        if self.target_id.is_some() { return Err(BrowserError::Protocol("Browser session no longer exists".into())); }
        }

        // 2. Fallback to /json/version
        let url = format!("{}/json/version", self.endpoint);
        let resp = client
            .get(&url)
            .header(reqwest::header::HOST, "localhost")
            .send()
            .await
            .map_err(|e| BrowserError::Http(e.to_string()))?;

        let json: Value = resp.json().await.map_err(|e| BrowserError::Http(e.to_string()))?;
        let advertised = json["webSocketDebuggerUrl"]
            .as_str()
            .ok_or_else(|| BrowserError::Protocol("Missing webSocketDebuggerUrl".into()))?;
        self.reachable_ws_url(advertised)
    }

    async fn ensure_connected(
        &self,
        guard: &mut tokio::sync::MutexGuard<'_, Option<WsStream>>,
    ) -> Result<(), BrowserError> {
        if guard.is_none() {
            let ws_url = self.get_ws_url().await?;
            **guard = Some(connect_cdp(&ws_url).await?);
        }
        Ok(())
    }

    pub async fn send_cdp_command(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, BrowserError> {
        let mut guard = self.ws_conn.lock().await;
        self.ensure_connected(&mut guard).await?;

        let id = self.msg_id.fetch_add(1, Ordering::SeqCst);
        let request = json!({
            "id": id,
            "method": method,
            "params": params
        });

        let ws = guard.as_mut().unwrap();
        if ws.send(Message::Text(request.to_string())).await.is_err() {
            // Reconnect and retry send once if disconnected
            *guard = None;
            self.ensure_connected(&mut guard).await?;
            let ws = guard.as_mut().unwrap();
            ws.send(Message::Text(request.to_string()))
                .await
                .map_err(|e| BrowserError::Ws(e.to_string()))?;
        }

        let deadline = Duration::from_secs(15);
        let ws = guard.as_mut().unwrap();
        let timeout_res = timeout(deadline, async {
            while let Some(msg) = ws.next().await {
                match msg {
                    Ok(Message::Text(txt)) => {
                        if let Ok(parsed) = serde_json::from_str::<Value>(&txt) {
                            if parsed.get("id").and_then(|v| v.as_u64()) == Some(id) {
                                if let Some(err) = parsed.get("error") {
                                    return Err(BrowserError::Protocol(err.to_string()));
                                }
                                return Ok(parsed.get("result").cloned().unwrap_or(Value::Null));
                            }
                        }
                    }
                    Ok(Message::Close(_)) => {
                        return Err(BrowserError::Protocol("Connection closed before response".into()));
                    }
                    Err(e) => {
                        return Err(BrowserError::Ws(e.to_string()));
                    }
                    _ => continue,
                }
            }
            Err(BrowserError::Protocol("Connection closed before response".into()))
        })
        .await;

        match timeout_res {
            Ok(Ok(val)) => Ok(val),
            Ok(Err(e)) => {
                *guard = None;
                Err(e)
            }
            Err(_) => {
                *guard = None;
                Err(BrowserError::Timeout)
            }
        }
    }

    async fn vault(&self) -> Option<Vault> {
        self.vault.lock().await.clone()
    }

    /// The main frame's id and real URL, as Chrome reports them.
    async fn main_frame(&self) -> Option<(String, String)> {
        let tree = self.send_cdp_command("Page.getFrameTree", json!({})).await.ok()?;
        let frame = &tree["frameTree"]["frame"];
        Some((frame["id"].as_str()?.to_string(), frame["url"].as_str()?.to_string()))
    }

    fn web_origin(url: &str) -> Option<String> {
        let u = url::Url::parse(url).ok()?;
        matches!(u.scheme(), "http" | "https").then(|| u.origin().ascii_serialization())
    }

    /// Sets up autofill and capture in kwnt's isolated world of the current document. The saved
    /// password goes only to a page whose real origin (scheme, host and port) is exactly the
    /// saved one, into a visible password field whose form posts to the same origin.
    pub async fn inject_autofill_and_capture(&self) -> Result<(), BrowserError> {
        *self.capture_world.lock().await = None;
        let Some(vault) = self.vault().await else { return Ok(()) };
        let Some((frame_id, url)) = self.main_frame().await else { return Ok(()) };
        let Some(origin) = Self::web_origin(&url) else { return Ok(()) };
        let world = self
            .send_cdp_command("Page.createIsolatedWorld", json!({"frameId": frame_id, "worldName": KWNT_WORLD}))
            .await?;
        let Some(context) = world["executionContextId"].as_i64() else { return Ok(()) };
        *self.capture_world.lock().await = Some(context);
        let fill = match vault.store.for_origin(&vault.owner, &origin).await {
            Ok(Some(c)) => json!({"origin": origin, "username": c.meta.username, "password": c.password}),
            _ => Value::Null,
        };
        let script = AUTOFILL_SCRIPT.replace("__KWNT_CREDS__", &fill.to_string());
        let _ = self
            .send_cdp_command(
                "Runtime.evaluate",
                json!({ "expression": script, "contextId": context, "returnByValue": false }),
            )
            .await;
        Ok(())
    }

    /// Takes a password the person submitted on the page into [`Self::pending_capture`], to be
    /// saved only when they confirm. The origin comes from the frame's URL, read before and
    /// after, so a capture is never filed under another site. A capture that equals the saved
    /// credential (the autofill itself) is dropped.
    pub async fn collect_captured_credentials(&self) -> Option<PendingCapture> {
        let context = (*self.capture_world.lock().await)?;
        let (_, before) = self.main_frame().await?;
        let r = self
            .send_cdp_command(
                "Runtime.evaluate",
                json!({ "expression": COLLECT_SCRIPT, "contextId": context, "returnByValue": true }),
            )
            .await
            .ok()?;
        let raw = r["result"]["value"].as_str()?;
        let (_, after) = self.main_frame().await?;
        let origin = Self::web_origin(&before)?;
        if Self::web_origin(&after).as_deref() != Some(origin.as_str()) {
            return None;
        }
        let val: Value = serde_json::from_str(raw).ok()?;
        let username = val["username"].as_str().unwrap_or("").to_string();
        let password = val["password"].as_str().unwrap_or("").to_string();
        if password.is_empty() {
            return None;
        }
        if let Some(vault) = self.vault().await {
            if let Ok(Some(saved)) = vault.store.for_origin(&vault.owner, &origin).await {
                if saved.meta.username == username && saved.password == password {
                    return None;
                }
            }
        }
        let capture = PendingCapture { origin, username, password, captured_at: chrono::Utc::now().to_rfc3339() };
        self.hold_for_confirmation(capture.clone()).await;
        Some(capture)
    }

    /// Holds a captured password until the person saves or discards it (replacing an older one).
    pub async fn hold_for_confirmation(&self, capture: PendingCapture) {
        *self.pending_capture.lock().await = Some(capture);
    }

    /// The capture waiting for the person, if any.
    pub async fn pending(&self) -> Option<PendingCapture> {
        self.pending_capture.lock().await.clone()
    }

    pub async fn take_pending(&self) -> Option<PendingCapture> {
        self.pending_capture.lock().await.take()
    }

    pub async fn navigate(&self, target_url: &str) -> Result<NavigationResult, BrowserError> {
        let _permit = self.control.try_read().map_err(|_| BrowserError::HumanControl)?;
        let result = self.navigate_unlocked(target_url).await?;
        // Bot navigation only: the person decides banners themselves in the live view
        if let Some(clicked) = self.reject_consent().await {
            tracing::debug!("Declined cookie banner via \"{}\"", clicked);
            let _ = self.capture_screenshot().await;
        }
        self.persist_cookies().await;
        Ok(result)
    }

    pub async fn navigate_unlocked(&self, target_url: &str) -> Result<NavigationResult, BrowserError> {
        refuse_file_path(target_url)?;
        let _ = self.collect_captured_credentials().await;
        let _ = self.send_cdp_command("Page.enable", json!({})).await;
        let res = self
            .send_cdp_command("Page.navigate", json!({ "url": target_url }))
            .await?;
        if let Some(error) = res["errorText"].as_str() {
            // Chrome shows its own error page; reflect it in the viewer
            tokio::time::sleep(Duration::from_millis(200)).await;
            let _ = self.sync_location().await;
            let _ = self.capture_screenshot().await;
            return Err(BrowserError::Navigation(error.to_string()));
        }
        *self.last_screenshot.lock().await = None;

        // Report where the browser actually ended up (redirects, e.g. to a CAPTCHA page)
        let (url, title) = match self.sync_location().await {
            Some(loc) => loc,
            None => {
                *self.current_url.lock().await = Some(target_url.to_string());
                *self.current_title.lock().await = Some(target_url.to_string());
                (target_url.to_string(), target_url.to_string())
            }
        };

        // Automatically inject autofill and capture hooks
        let _ = self.inject_autofill_and_capture().await;

        // Automatically capture fresh screenshot on navigation
        self.capture_screenshot().await?;

        Ok(NavigationResult {
            url,
            title,
            status_code: 200,
        })
    }

    pub async fn capture_screenshot(&self) -> Result<String, BrowserError> {
        let res = self
            .send_cdp_command(
                "Page.captureScreenshot",
                json!({ "format": "jpeg", "quality": 75 }),
            )
            .await?;

        let b64 = res["data"]
            .as_str()
            .ok_or_else(|| BrowserError::Protocol("Screenshot data missing".into()))?
            .to_string();

        *self.last_screenshot.lock().await = Some(b64.clone());
        Ok(b64)
    }

    pub async fn extract_content(&self) -> Result<String, BrowserError> {
        let expression = r#"
            (function() {
                if (!document.body) return '';
                var clone = document.body.cloneNode(true);
                var scripts = clone.getElementsByTagName('script');
                while(scripts.length > 0) { scripts[0].parentNode.removeChild(scripts[0]); }
                var styles = clone.getElementsByTagName('style');
                while(styles.length > 0) { styles[0].parentNode.removeChild(styles[0]); }
                return clone.innerText.substring(0, 10000);
            })()
        "#;
        let res = self
            .send_cdp_command(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await?;

        let text = res["result"]["value"].as_str().unwrap_or("").to_string();
        Ok(text)
    }

    pub async fn click(&self, selector: &str) -> Result<(), BrowserError> {
        let _permit = self.control.try_read().map_err(|_| BrowserError::HumanControl)?;
        let sel_json = serde_json::to_string(selector)
            .map_err(|e| BrowserError::Protocol(e.to_string()))?;
        let expression = format!(
            "var el = document.querySelector({}); if (el) {{ el.click(); true; }} else {{ false; }}",
            sel_json
        );
        let res = self
            .send_cdp_command(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await?;
        if res["result"]["value"].as_bool() != Some(true) {
            return Err(BrowserError::Protocol("Element not found for selector".into()));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let _ = self.collect_captured_credentials().await;
        let _ = self.capture_screenshot().await;
        Ok(())
    }

    pub async fn type_text(&self, selector: &str, text: &str) -> Result<(), BrowserError> {
        let _permit = self.control.try_read().map_err(|_| BrowserError::HumanControl)?;
        let sel_json = serde_json::to_string(selector)
            .map_err(|e| BrowserError::Protocol(e.to_string()))?;
        let text_json = serde_json::to_string(text)
            .map_err(|e| BrowserError::Protocol(e.to_string()))?;
        let expression = format!(
            "var el = document.querySelector({}); if (el) {{ el.value = {}; el.dispatchEvent(new Event('input', {{ bubbles: true }})); true; }} else {{ false; }}",
            sel_json,
            text_json
        );
        let res = self
            .send_cdp_command(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await?;
        if res["result"]["value"].as_bool() != Some(true) {
            return Err(BrowserError::Protocol("Element not found for selector".into()));
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _ = self.capture_screenshot().await;
        Ok(())
    }

    /// Whether the page's focused element takes typing (a text field or editable area), followed
    /// into open shadow roots and same-origin frames. The live view opens the phone keyboard on it.
    pub async fn focused_editable(&self) -> bool {
        let Ok(res) = self.send_cdp_command("Runtime.evaluate", json!({
            "expression": FOCUSED_EDITABLE_JS, "returnByValue": true
        })).await else { return false };
        res["result"]["value"].as_bool().unwrap_or(false)
    }

    /// A visible CAPTCHA widget on the page, if any. `Err` when the page couldn't be read
    /// (mid-navigation, say): callers treat that as unknown, never as "gone".
    pub async fn probe_captcha(&self) -> Result<Option<CaptchaHit>, BrowserError> {
        let res = self.send_cdp_command("Runtime.evaluate", json!({
            "expression": CAPTCHA_PROBE_JS, "returnByValue": true
        })).await?;
        if res.get("exceptionDetails").is_some() {
            return Err(BrowserError::Protocol("CAPTCHA probe failed".into()));
        }
        Ok(CaptchaHit::parse(&res["result"]["value"]))
    }

    /// Declines optional cookies on a known consent banner. Never accepts.
    pub async fn reject_consent(&self) -> Option<String> {
        // Consent managers often render a moment after load: look twice
        for attempt in 0..2 {
            if attempt > 0 { tokio::time::sleep(Duration::from_millis(700)).await; }
            let Ok(res) = self.send_cdp_command("Runtime.evaluate", json!({
                "expression": CONSENT_REJECT_JS, "returnByValue": true
            })).await else { return None };
            if let Some(label) = res["result"]["value"].as_str() {
                tokio::time::sleep(Duration::from_millis(300)).await;
                return Some(label.chars().take(80).collect());
            }
        }
        None
    }

    /// VM mode keeps cookies in a file because the browser's profile lives on tmpfs.
    pub fn set_cookie_jar(&self, path: Option<std::path::PathBuf>) {
        *self.cookie_jar.lock().unwrap_or_else(|e| e.into_inner()) = path;
    }

    fn cookie_jar_path(&self) -> Option<std::path::PathBuf> {
        self.cookie_jar.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Saves this page's browser-context cookies to the jar (owner-only file, written atomically).
    pub async fn persist_cookies(&self) {
        let Some(path) = self.cookie_jar_path() else { return };
        let Ok(res) = self.send_cdp_command("Network.getAllCookies", json!({})).await else { return };
        let Some(cookies) = res["cookies"].as_array() else { return };
        // session cookies die with the browser anyway; keep only persistent ones
        let kept: Vec<&Value> = cookies.iter().filter(|c| c["session"].as_bool() != Some(true)).collect();
        if let Err(error) = write_private(&path, &serde_json::to_vec(&kept).unwrap_or_default()) {
            tracing::warn!("Could not save browser cookies: {}", error);
        }
    }

    /// Loads the jar into a fresh browser context. Returns how many cookies were restored.
    pub async fn restore_cookies(&self) -> usize {
        let Some(path) = self.cookie_jar_path() else { return 0 };
        let Ok(bytes) = std::fs::read(&path) else { return 0 };
        let Ok(Value::Array(cookies)) = serde_json::from_slice::<Value>(&bytes) else { return 0 };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
        let params: Vec<Value> = cookies.iter().filter(|c| c["expires"].as_f64().is_some_and(|e| e > now)).map(|c| {
            let mut p = json!({});
            for key in ["name", "value", "domain", "path", "secure", "httpOnly", "sameSite", "expires", "priority", "sameParty", "sourceScheme", "partitionKey"] {
                if let Some(v) = c.get(key) { if !v.is_null() { p[key] = v.clone(); } }
            }
            p
        }).collect();
        if params.is_empty() { return 0; }
        let count = params.len();
        match self.send_cdp_command("Network.setCookies", json!({"cookies": params})).await {
            Ok(_) => count,
            Err(error) => { tracing::warn!("Could not restore browser cookies: {}", error); 0 }
        }
    }

    /// Re-reads the page's real URL and title, since user clicks/keys can navigate
    /// without going through `navigate()`.
    pub async fn sync_location(&self) -> Option<(String, String)> {
        let res = self
            .send_cdp_command(
                "Runtime.evaluate",
                json!({ "expression": "[location.href, document.title]", "returnByValue": true }),
            )
            .await
            .ok()?;
        let vals = res["result"]["value"].as_array()?;
        let url = vals.first()?.as_str()?.to_string();
        let title = vals.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string();
        if url.is_empty() {
            return None;
        }
        *self.current_url.lock().await = Some(url.clone());
        *self.current_title.lock().await = Some(title.clone());
        Some((url, title))
    }

    /// Labels of visible interactive elements, topmost (e.g. modal buttons) first.
    pub async fn list_clickables(&self, limit: usize) -> Vec<String> {
        let expression = format!("({})(null)", CLICKABLES_JS);
        let Ok(res) = self
            .send_cdp_command(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await
        else {
            return Vec::new();
        };
        let mut labels: Vec<String> = Vec::new();
        if let Some(items) = res["result"]["value"]["items"].as_array() {
            for item in items {
                if let Some(label) = item["label"].as_str() {
                    if !labels.iter().any(|l| l == label) {
                        labels.push(label.to_string());
                    }
                }
                if labels.len() >= limit {
                    break;
                }
            }
        }
        labels
    }

    /// Clicks the visible element whose label best matches `text` (exact > prefix > contains,
    /// topmost first). Returns the clicked label, or an error listing the available ones.
    pub async fn click_text(&self, text: &str) -> Result<String, BrowserError> {
        let needle = serde_json::to_string(text).map_err(|e| BrowserError::Protocol(e.to_string()))?;
        let expression = format!("({})({})", CLICKABLES_JS, needle);
        let res = self
            .send_cdp_command(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await?;
        let value = &res["result"]["value"];
        let Some(label) = value["match"]["label"].as_str().map(str::to_string) else {
            let options: Vec<&str> = value["items"]
                .as_array()
                .map(|a| a.iter().filter_map(|i| i["label"].as_str()).take(25).collect())
                .unwrap_or_default();
            return Err(BrowserError::Protocol(format!(
                "No clickable element matching \"{}\". Visible options: {}",
                text,
                options.join(" | ")
            )));
        };
        if value["match"]["clicked"].as_bool() == Some(true) {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let _ = self.sync_location().await;
            let _ = self.capture_screenshot().await;
        } else {
            let x = value["match"]["x"].as_i64().unwrap_or(0);
            let y = value["match"]["y"].as_i64().unwrap_or(0);
            self.interact_click(x, y).await?;
            // Give navigations/animations triggered by the click a moment to settle
            tokio::time::sleep(Duration::from_millis(250)).await;
            let _ = self.sync_location().await;
            let _ = self.capture_screenshot().await;
        }
        Ok(label)
    }

    pub async fn interact_click(&self, x: i64, y: i64) -> Result<String, BrowserError> {
        self.send_cdp_command(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mousePressed",
                "x": x,
                "y": y,
                "button": "left",
                "clickCount": 1
            }),
        )
        .await?;

        self.send_cdp_command(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseReleased",
                "x": x,
                "y": y,
                "button": "left",
                "clickCount": 1
            }),
        )
        .await?;

        tokio::time::sleep(Duration::from_millis(150)).await;
        let _ = self.collect_captured_credentials().await;
        let _ = self.sync_location().await;
        self.capture_screenshot().await
    }

    pub async fn interact_type(&self, text: &str) -> Result<String, BrowserError> {
        self.send_cdp_command(
            "Input.insertText",
            json!({ "text": text }),
        )
        .await?;

        tokio::time::sleep(Duration::from_millis(150)).await;
        let _ = self.collect_captured_credentials().await;
        let _ = self.sync_location().await;
        self.capture_screenshot().await
    }

    pub async fn interact_key(&self, key: &str, code: Option<&str>) -> Result<String, BrowserError> {
        let mut down_params = json!({
            "type": "rawKeyDown",
            "key": key
        });
        if let Some(c) = code {
            down_params["code"] = json!(c);
        }
        self.send_cdp_command("Input.dispatchKeyEvent", down_params).await?;

        let mut up_params = json!({
            "type": "keyUp",
            "key": key
        });
        if let Some(c) = code {
            up_params["code"] = json!(c);
        }
        self.send_cdp_command("Input.dispatchKeyEvent", up_params).await?;

        tokio::time::sleep(Duration::from_millis(150)).await;
        let _ = self.collect_captured_credentials().await;
        let _ = self.sync_location().await;
        self.capture_screenshot().await
    }

    pub async fn interact_scroll(&self, x: i64, y: i64, delta_x: f64, delta_y: f64) -> Result<String, BrowserError> {
        self.send_cdp_command(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseWheel",
                "x": x,
                "y": y,
                "deltaX": delta_x,
                "deltaY": delta_y
            }),
        )
        .await?;

        tokio::time::sleep(Duration::from_millis(150)).await;
        let _ = self.sync_location().await;
        self.capture_screenshot().await
    }
}


#[cfg(test)]
mod tests {
    use super::cdp_ws_request;

    fn host(url: &str) -> String {
        cdp_ws_request(url).unwrap().headers()["host"].to_str().unwrap().to_string()
    }

    #[test]
    fn cdp_ws_host_is_localhost_for_service_names() {
        // Chrome answers 500 to a DevTools upgrade with Host: browser:9222
        assert_eq!(host("ws://browser:9222/devtools/browser/x"), "localhost");
        assert_eq!(host("ws://127.0.0.1:9223/devtools/page/x"), "127.0.0.1:9223");
        assert_eq!(host("ws://[::1]:9222/devtools/page/x"), "[::1]:9222");
        assert_eq!(host("ws://localhost:9222/devtools/page/x"), "localhost:9222");
        assert_eq!(host("wss://cdp.example.com/devtools/browser/x"), "cdp.example.com");
    }
}
