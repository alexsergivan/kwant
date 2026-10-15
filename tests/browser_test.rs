use futures::{SinkExt, StreamExt};
use kwnt::sandbox::browser::{BrowserController, BrowserError, NavigationResult};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

#[test]
fn test_browser_controller_serialization_and_helpers() {
    let nav = NavigationResult {
        url: "https://example.com".into(),
        title: "Example Domain".into(),
        status_code: 200,
    };

    let json = serde_json::to_string(&nav).unwrap();
    assert!(json.contains("Example Domain"));

    let deserialized: NavigationResult = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.url, "https://example.com");
    assert_eq!(deserialized.title, "Example Domain");
    assert_eq!(deserialized.status_code, 200);

    let controller = BrowserController::new(19222);
    assert_eq!(controller.cdp_port, 19222);
}

#[tokio::test]
async fn test_browser_controller_connection_failure() {
    // Port 65530 is unlikely to be listening, should fail with Http error
    let controller = BrowserController::new(65530);
    let res = controller.navigate("https://example.com").await;
    assert!(res.is_err());
    match res.unwrap_err() {
        BrowserError::Http(_) => {}
        other => panic!("Expected BrowserError::Http, got {:?}", other),
    }
}

#[tokio::test]
async fn test_mock_cdp_controller_lifecycle() {
    let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_port = http_listener.local_addr().unwrap().port();

    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();

    // Spawn HTTP listener for /json/version
    let http_handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = http_listener.accept().await {
            let mut buf = vec![0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);

            if req.contains("/json/version") {
                let ws_url = format!("ws://127.0.0.1:{}/devtools/page/test", ws_port);
                let body = json!({
                    "Browser": "Chrome/120.0.0.0",
                    "webSocketDebuggerUrl": ws_url
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.flush().await;
            }
        }
    });

    // Spawn WebSocket listener for CDP commands
    let ws_handle = tokio::spawn(async move {
        while let Ok((stream, _)) = ws_listener.accept().await {
            tokio::spawn(async move {
                if let Ok(mut ws) = accept_async(stream).await {
                    while let Some(Ok(msg)) = ws.next().await {
                        if let tokio_tungstenite::tungstenite::Message::Text(txt) = msg {
                            let parsed: Value = serde_json::from_str(&txt).unwrap();
                            let id = parsed["id"].as_u64().unwrap();
                            let method = parsed["method"].as_str().unwrap();

                            let result_val = match method {
                                "Page.enable" => json!({}),
                                "Page.navigate" => json!({ "frameId": "F1", "loaderId": "L1" }),
                                "Page.captureScreenshot" => json!({ "data": "dGVzdC1zY3JlZW5zaG90" }),
                                "Runtime.evaluate" => {
                                    let expr = parsed["params"]["expression"].as_str().unwrap();
                                    if expr == "document.title" {
                                        json!({ "result": { "type": "string", "value": "Kwants Sandbox Test" } })
                                    } else if expr.contains("clone.innerText") {
                                        json!({ "result": { "type": "string", "value": "Extracted DOM Content" } })
                                    } else if expr.contains("el.click()") {
                                        json!({ "result": { "type": "boolean", "value": true } })
                                    } else if expr.contains("el.value =") {
                                        json!({ "result": { "type": "boolean", "value": true } })
                                    } else {
                                        json!({ "result": { "type": "undefined" } })
                                    }
                                }
                                _ => json!({}),
                            };

                            let resp = json!({
                                "id": id,
                                "result": result_val
                            });

                            let _ = ws
                                .send(tokio_tungstenite::tungstenite::Message::Text(
                                    resp.to_string().into(),
                                ))
                                .await;
                        }
                    }
                }
            });
        }
    });

    let controller = BrowserController::new(http_port);

    // Test navigate
    let nav_res = controller.navigate("https://kwants.internal/app").await.unwrap();
    assert_eq!(nav_res.url, "https://kwants.internal/app");
    assert_eq!(nav_res.title, "Kwants Sandbox Test");
    assert_eq!(nav_res.status_code, 200);

    // Verify internal state updated
    assert_eq!(controller.current_url.lock().await.as_deref(), Some("https://kwants.internal/app"));
    assert_eq!(controller.current_title.lock().await.as_deref(), Some("Kwants Sandbox Test"));
    assert_eq!(controller.last_screenshot.lock().await.as_deref(), Some("dGVzdC1zY3JlZW5zaG90"));

    // Test capture_screenshot directly
    let shot = controller.capture_screenshot().await.unwrap();
    assert_eq!(shot, "dGVzdC1zY3JlZW5zaG90");

    // Test extract_content
    let text = controller.extract_content().await.unwrap();
    assert_eq!(text, "Extracted DOM Content");

    // Test click
    controller.click("#submit-button").await.unwrap();

    // Test type_text
    controller.type_text("input[name=query]", "hello world").await.unwrap();

    http_handle.abort();
    ws_handle.abort();
}

#[tokio::test]
async fn test_mock_cdp_missing_ws_url() {
    let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_port = http_listener.local_addr().unwrap().port();

    let http_handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = http_listener.accept().await {
            let mut buf = vec![0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let body = json!({ "Browser": "Chrome/120.0.0.0" }).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;
        }
    });

    let controller = BrowserController::new(http_port);
    let res = controller.navigate("https://kwants.internal").await;
    match res {
        Err(BrowserError::Protocol(msg)) => assert!(msg.contains("Missing webSocketDebuggerUrl")),
        other => panic!("Expected BrowserError::Protocol, got {:?}", other),
    }

    http_handle.abort();
}

#[tokio::test]
async fn test_mock_cdp_protocol_error_response() {
    let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_port = http_listener.local_addr().unwrap().port();

    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();

    let http_handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = http_listener.accept().await {
            let mut buf = vec![0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let ws_url = format!("ws://127.0.0.1:{}/devtools/page/test", ws_port);
            let body = json!({
                "Browser": "Chrome/120.0.0.0",
                "webSocketDebuggerUrl": ws_url
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;
        }
    });

    let ws_handle = tokio::spawn(async move {
        while let Ok((stream, _)) = ws_listener.accept().await {
            tokio::spawn(async move {
                if let Ok(mut ws) = accept_async(stream).await {
                    while let Some(Ok(msg)) = ws.next().await {
                        if let tokio_tungstenite::tungstenite::Message::Text(txt) = msg {
                            let parsed: Value = serde_json::from_str(&txt).unwrap();
                            let id = parsed["id"].as_u64().unwrap();
                            let resp = json!({
                                "id": id,
                                "error": { "code": -32000, "message": "Failed to take screenshot" }
                            });
                            let _ = ws
                                .send(tokio_tungstenite::tungstenite::Message::Text(
                                    resp.to_string().into(),
                                ))
                                .await;
                        }
                    }
                }
            });
        }
    });

    let controller = BrowserController::new(http_port);
    let res = controller.capture_screenshot().await;
    match res {
        Err(BrowserError::Protocol(msg)) => assert!(msg.contains("Failed to take screenshot")),
        other => panic!("Expected BrowserError::Protocol, got {:?}", other),
    }

    http_handle.abort();
    ws_handle.abort();
}
