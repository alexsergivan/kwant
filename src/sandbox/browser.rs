use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigationResult {
    pub url: String,
    pub title: String,
    pub status_code: u16,
}

#[derive(Clone)]
pub struct BrowserController {
    pub cdp_port: u16,
    pub current_url: Arc<Mutex<Option<String>>>,
    pub current_title: Arc<Mutex<Option<String>>>,
    pub last_screenshot: Arc<Mutex<Option<String>>>,
    msg_id: Arc<AtomicU64>,
    ws_conn: Arc<Mutex<Option<WsStream>>>,
}

impl BrowserController {
    pub fn new(cdp_port: u16) -> Self {
        Self {
            cdp_port,
            current_url: Arc::new(Mutex::new(None)),
            current_title: Arc::new(Mutex::new(None)),
            last_screenshot: Arc::new(Mutex::new(None)),
            msg_id: Arc::new(AtomicU64::new(1)),
            ws_conn: Arc::new(Mutex::new(None)),
        }
    }

    async fn get_ws_url(&self) -> Result<String, BrowserError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| BrowserError::Http(e.to_string()))?;

        let url = format!("http://127.0.0.1:{}/json/version", self.cdp_port);
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| BrowserError::Http(e.to_string()))?;

        let json: Value = resp.json().await.map_err(|e| BrowserError::Http(e.to_string()))?;
        json["webSocketDebuggerUrl"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| BrowserError::Protocol("Missing webSocketDebuggerUrl".into()))
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

    async fn send_cdp_command(
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
        if ws.send(Message::Text(request.to_string().into())).await.is_err() {
            // Reconnect and retry send once if disconnected
            *guard = None;
            self.ensure_connected(&mut guard).await?;
            let ws = guard.as_mut().unwrap();
            ws.send(Message::Text(request.to_string().into()))
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

    pub async fn navigate(&self, target_url: &str) -> Result<NavigationResult, BrowserError> {
        let _ = self.send_cdp_command("Page.enable", json!({})).await;
        let _res = self
            .send_cdp_command("Page.navigate", json!({ "url": target_url }))
            .await?;

        // Extract title
        let eval_res = self
            .send_cdp_command(
                "Runtime.evaluate",
                json!({ "expression": "document.title", "returnByValue": true }),
            )
            .await
            .ok();

        let title = eval_res
            .as_ref()
            .and_then(|r| r["result"]["value"].as_str())
            .unwrap_or(target_url)
            .to_string();

        *self.current_url.lock().await = Some(target_url.to_string());
        *self.current_title.lock().await = Some(title.clone());

        // Automatically capture fresh screenshot on navigation
        let _ = self.capture_screenshot().await;

        Ok(NavigationResult {
            url: target_url.to_string(),
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
        let _ = self.capture_screenshot().await;
        Ok(())
    }

    pub async fn type_text(&self, selector: &str, text: &str) -> Result<(), BrowserError> {
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
}
