use futures::{SinkExt, StreamExt};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigationResult {
    pub url: String,
    pub title: String,
    pub status_code: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SavedCredential {
    pub id: String,
    pub domain: String,
    pub origin: String,
    pub username: String,
    pub password: String,
    pub last_used: String,
}

#[derive(Clone)]
pub struct BrowserController {
    pub cdp_port: u16,
    pub current_url: Arc<Mutex<Option<String>>>,
    pub current_title: Arc<Mutex<Option<String>>>,
    pub last_screenshot: Arc<Mutex<Option<String>>>,
    pub saved_credentials: Arc<Mutex<Vec<SavedCredential>>>,
    pub workspace_dir: Arc<Mutex<Option<std::path::PathBuf>>>,
    msg_id: Arc<AtomicU64>,
    ws_conn: Arc<Mutex<Option<WsStream>>>,
    endpoint: String,
    target_id: Option<String>,
    browser_level: bool,
    pub control: Arc<RwLock<()>>,
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
            saved_credentials: Arc::new(Mutex::new(Vec::new())),
            workspace_dir: Arc::new(Mutex::new(None)),
            msg_id: Arc::new(AtomicU64::new(1)),
            ws_conn: Arc::new(Mutex::new(None)),
            endpoint: format!("http://127.0.0.1:{cdp_port}"),
            target_id: None,
            browser_level: false,
            control: Arc::new(RwLock::new(())),
        }
    }

    pub fn with_workspace(cdp_port: u16, workspace_dir: std::path::PathBuf) -> Self {
        let mut loaded = Vec::new();
        let creds_file = workspace_dir.join(".saved_passwords.json");
        if creds_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&creds_file) {
                if let Ok(creds) = serde_json::from_str::<Vec<SavedCredential>>(&content) {
                    loaded = creds;
                }
            }
        }

        Self {
            cdp_port,
            current_url: Arc::new(Mutex::new(None)),
            current_title: Arc::new(Mutex::new(None)),
            last_screenshot: Arc::new(Mutex::new(None)),
            saved_credentials: Arc::new(Mutex::new(loaded)),
            workspace_dir: Arc::new(Mutex::new(Some(workspace_dir))),
            msg_id: Arc::new(AtomicU64::new(1)),
            ws_conn: Arc::new(Mutex::new(None)),
            endpoint: format!("http://127.0.0.1:{cdp_port}"),
            target_id: None,
            browser_level: false,
            control: Arc::new(RwLock::new(())),
        }
    }

    pub async fn set_workspace(&self, ws: std::path::PathBuf) {
        let creds_file = ws.join(".saved_passwords.json");
        if creds_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&creds_file) {
                if let Ok(creds) = serde_json::from_str::<Vec<SavedCredential>>(&content) {
                    let mut lock = self.saved_credentials.lock().await;
                    *lock = creds;
                }
            }
        }
        *self.workspace_dir.lock().await = Some(ws);
    }

    pub fn at_endpoint(endpoint: &str, target_id: Option<String>, workspace: Option<std::path::PathBuf>, browser_level: bool) -> Self {
        let mut ctrl = workspace.map(|ws| Self::with_workspace(0, ws)).unwrap_or_else(|| Self::new(0));
        ctrl.endpoint = endpoint.trim_end_matches('/').to_string();
        ctrl.target_id = target_id;
        ctrl.browser_level = browser_level;
        ctrl
    }

    pub async fn take_control(&self) -> Result<OwnedRwLockWriteGuard<()>, BrowserError> {
        self.control.clone().try_write_owned().map_err(|_| BrowserError::HumanControl)
    }

    pub fn is_managed_session(&self) -> bool { self.target_id.is_some() }

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
            let (ws, _) = connect_async(&ws_url)
                .await
                .map_err(|e| BrowserError::Ws(e.to_string()))?;
            **guard = Some(ws);
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

    pub async fn list_saved_credentials(&self) -> Vec<SavedCredential> {
        self.saved_credentials.lock().await.clone()
    }

    pub async fn save_credential(
        &self,
        domain: &str,
        origin: &str,
        username: &str,
        password: &str,
    ) -> SavedCredential {
        let mut lock = self.saved_credentials.lock().await;
        let now = chrono::Utc::now().to_rfc3339();

        let clean_domain = if domain.is_empty() {
            if let Ok(parsed) = reqwest::Url::parse(origin) {
                parsed.host_str().unwrap_or(origin).to_string()
            } else {
                origin.to_string()
            }
        } else {
            domain.to_string()
        };

        // Check if matching credential exists
        if let Some(existing) = lock.iter_mut().find(|c| {
            (c.domain.eq_ignore_ascii_case(&clean_domain) || (!origin.is_empty() && c.origin == origin))
                && c.username == username
        }) {
            existing.password = password.to_string();
            existing.last_used = now;
            let updated = existing.clone();
            drop(lock);
            self.persist_credentials_to_disk().await;
            return updated;
        }

        let new_cred = SavedCredential {
            id: uuid::Uuid::new_v4().to_string(),
            domain: clean_domain,
            origin: origin.to_string(),
            username: username.to_string(),
            password: password.to_string(),
            last_used: now,
        };

        lock.push(new_cred.clone());
        drop(lock);
        self.persist_credentials_to_disk().await;
        new_cred
    }

    pub async fn delete_saved_credential(&self, id: &str) -> bool {
        let mut lock = self.saved_credentials.lock().await;
        let initial_len = lock.len();
        lock.retain(|c| c.id != id);
        let removed = lock.len() < initial_len;
        drop(lock);
        if removed {
            self.persist_credentials_to_disk().await;
        }
        removed
    }

    pub async fn get_credential_for_url(&self, target_url: &str) -> Option<SavedCredential> {
        let parsed = reqwest::Url::parse(target_url).ok();
        let target_host = parsed.as_ref().and_then(|u| u.host_str()).unwrap_or(target_url);
        let target_origin = parsed.as_ref().map(|u| u.origin().ascii_serialization()).unwrap_or_default();

        let lock = self.saved_credentials.lock().await;
        lock.iter().find(|c| {
            c.domain.eq_ignore_ascii_case(target_host)
                || (!c.origin.is_empty() && c.origin == target_origin)
                || target_host.ends_with(&format!(".{}", c.domain))
        }).cloned()
    }

    pub async fn persist_credentials_to_disk(&self) {
        let ws_opt = self.workspace_dir.lock().await.clone();
        if let Some(ws) = ws_opt {
            if tokio::fs::create_dir_all(&ws).await.is_err() { return; }
            let creds = self.saved_credentials.lock().await.clone();
            let creds_file = ws.join(".saved_passwords.json");
            if let Ok(json_str) = serde_json::to_string_pretty(&creds) {
                let _ = std::fs::write(&creds_file, json_str);
            }
        }
    }

    pub async fn inject_autofill_and_capture(&self, target_url: &str) -> Result<(), BrowserError> {
        let cred_opt = self.get_credential_for_url(target_url).await;
        let cred_json = serde_json::to_string(&cred_opt).unwrap_or_else(|_| "null".into());

        let script = format!(
            r#"
            (function() {{
                var creds = {};

                function fillCredentials() {{
                    if (!creds || !creds.password) return;
                    var passInput = document.querySelector('input[type="password"]');
                    var userInput = document.querySelector('input[type="text"], input[type="email"], input[name*="user"], input[name*="login"], input[name*="email"], input[id*="user"], input[id*="login"]');
                    if (passInput) {{
                        if (!passInput.value) {{
                            passInput.value = creds.password;
                            passInput.dispatchEvent(new Event('input', {{ bubbles: true }}));
                            passInput.dispatchEvent(new Event('change', {{ bubbles: true }}));
                        }}
                    }}
                    if (userInput) {{
                        if (!userInput.value && creds.username) {{
                            userInput.value = creds.username;
                            userInput.dispatchEvent(new Event('input', {{ bubbles: true }}));
                            userInput.dispatchEvent(new Event('change', {{ bubbles: true }}));
                        }}
                    }}
                }}

                fillCredentials();
                if (document.readyState === 'loading') {{
                    document.addEventListener('DOMContentLoaded', fillCredentials);
                }}
                setTimeout(fillCredentials, 600);
                setTimeout(fillCredentials, 1800);

                if (!window.__kwnt_capture_installed) {{
                    window.__kwnt_capture_installed = true;

                    function recordForm(form) {{
                        if (!form) return;
                        var passInput = form.querySelector('input[type="password"]');
                        if (!passInput || !passInput.value) return;
                        var userInput = form.querySelector('input[type="text"], input[type="email"], input[name*="user"], input[name*="login"], input[name*="email"], input[id*="user"], input[id*="login"]');
                        var username = userInput ? userInput.value : '';
                        var password = passInput.value;
                        if (password) {{
                            window.__kwnt_captured_creds = {{
                                domain: window.location.hostname || window.location.host,
                                origin: window.location.origin,
                                username: username,
                                password: password
                            }};
                        }}
                    }}

                    document.addEventListener('submit', function(e) {{
                        recordForm(e.target);
                    }}, true);

                    document.addEventListener('click', function(e) {{
                        var btn = e.target.closest('button, input[type="submit"], [role="button"]');
                        if (!btn) return;
                        var form = btn.closest('form') || document;
                        recordForm(form);
                    }}, true);
                }}
            }})();
            "#,
            cred_json
        );

        let _ = self
            .send_cdp_command(
                "Runtime.evaluate",
                json!({ "expression": script, "returnByValue": false }),
            )
            .await;

        Ok(())
    }

    pub async fn collect_and_save_captured_credentials(&self) -> Result<Option<SavedCredential>, BrowserError> {
        let check_script = r#"
            (function() {
                if (window.__kwnt_captured_creds) {
                    var c = window.__kwnt_captured_creds;
                    window.__kwnt_captured_creds = null;
                    return JSON.stringify(c);
                }
                return null;
            })()
        "#;

        let eval_res = self
            .send_cdp_command(
                "Runtime.evaluate",
                json!({ "expression": check_script, "returnByValue": true }),
            )
            .await
            .ok();

        if let Some(r) = eval_res {
            if let Some(raw_json) = r["result"]["value"].as_str() {
                if let Ok(val) = serde_json::from_str::<Value>(raw_json) {
                    let domain = val["domain"].as_str().unwrap_or("");
                    let origin = val["origin"].as_str().unwrap_or("");
                    let username = val["username"].as_str().unwrap_or("");
                    let password = val["password"].as_str().unwrap_or("");

                    if !password.is_empty() {
                        let saved = self.save_credential(domain, origin, username, password).await;
                        return Ok(Some(saved));
                    }
                }
            }
        }

        Ok(None)
    }

    pub async fn navigate(&self, target_url: &str) -> Result<NavigationResult, BrowserError> {
        let _permit = self.control.try_read().map_err(|_| BrowserError::HumanControl)?;
        self.navigate_unlocked(target_url).await
    }

    pub async fn navigate_unlocked(&self, target_url: &str) -> Result<NavigationResult, BrowserError> {
        let _ = self.collect_and_save_captured_credentials().await;
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
        let _ = self.inject_autofill_and_capture(&url).await;

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
        let _ = self.collect_and_save_captured_credentials().await;
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
        let _ = self.collect_and_save_captured_credentials().await;
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
        let _ = self.collect_and_save_captured_credentials().await;
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
        let _ = self.collect_and_save_captured_credentials().await;
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

