use futures::{SinkExt, StreamExt};
use kwnt::sandbox::browser::{BrowserController, BrowserError, NavigationResult};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
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
                                    if expr == "[location.href, document.title]" {
                                        json!({ "result": { "type": "object", "value": ["https://kwants.internal/app", "Kwants Sandbox Test"] } })
                                    } else if expr == "document.title" {
                                        json!({ "result": { "type": "string", "value": "Kwants Sandbox Test" } })
                                    } else if expr.contains("clone.innerText") {
                                        json!({ "result": { "type": "string", "value": "Extracted DOM Content" } })
                                    } else if expr.contains("el.click()") || expr.contains("el.value =") {
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
                                    resp.to_string(),
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
async fn test_mock_cdp_persistent_connection_reuse() {
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

    let ws_connect_count = Arc::new(AtomicUsize::new(0));
    let ws_counter_clone = ws_connect_count.clone();

    let ws_handle = tokio::spawn(async move {
        while let Ok((stream, _)) = ws_listener.accept().await {
            ws_counter_clone.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                if let Ok(mut ws) = accept_async(stream).await {
                    while let Some(Ok(msg)) = ws.next().await {
                        if let tokio_tungstenite::tungstenite::Message::Text(txt) = msg {
                            let parsed: Value = serde_json::from_str(&txt).unwrap();
                            let id = parsed["id"].as_u64().unwrap();
                            let method = parsed["method"].as_str().unwrap();

                            let result_val = match method {
                                "Page.enable" => json!({}),
                                "Page.navigate" => json!({ "frameId": "F1" }),
                                "Page.captureScreenshot" => json!({ "data": "c2NyZWVuc2hvdA==" }),
                                "Runtime.evaluate" => {
                                    json!({ "result": { "type": "boolean", "value": true } })
                                }
                                _ => json!({}),
                            };

                            let resp = json!({
                                "id": id,
                                "result": result_val
                            });
                            let _ = ws
                                .send(tokio_tungstenite::tungstenite::Message::Text(
                                    resp.to_string(),
                                ))
                                .await;
                        }
                    }
                }
            });
        }
    });

    let controller = BrowserController::new(http_port);

    // Perform multiple operations sequentially
    controller.navigate("https://kwants.internal").await.unwrap();
    controller.capture_screenshot().await.unwrap();
    controller.extract_content().await.unwrap();
    controller.click("#btn").await.unwrap();
    controller.type_text("#input", "value").await.unwrap();

    // Verify only 1 WebSocket connection was established across all operations
    assert_eq!(
        ws_connect_count.load(Ordering::SeqCst),
        1,
        "WebSocket connection must be reused persistently across commands"
    );

    http_handle.abort();
    ws_handle.abort();
}

#[tokio::test]
async fn test_mock_cdp_string_escaping_multiline_and_quotes() {
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

    let received_expression = Arc::new(tokio::sync::Mutex::new(String::new()));
    let expr_clone = received_expression.clone();

    let ws_handle = tokio::spawn(async move {
        while let Ok((stream, _)) = ws_listener.accept().await {
            let expr_receiver = expr_clone.clone();
            tokio::spawn(async move {
                if let Ok(mut ws) = accept_async(stream).await {
                    while let Some(Ok(msg)) = ws.next().await {
                        if let tokio_tungstenite::tungstenite::Message::Text(txt) = msg {
                            let parsed: Value = serde_json::from_str(&txt).unwrap();
                            let id = parsed["id"].as_u64().unwrap();
                            let method = parsed["method"].as_str().unwrap();

                            if method == "Runtime.evaluate" {
                                let expr = parsed["params"]["expression"].as_str().unwrap();
                                *expr_receiver.lock().await = expr.to_string();
                            }

                            let resp = json!({
                                "id": id,
                                "result": {
                                    "result": { "type": "boolean", "value": true },
                                    "data": "c2NyZWVuc2hvdA=="
                                }
                            });
                            let _ = ws
                                .send(tokio_tungstenite::tungstenite::Message::Text(
                                    resp.to_string(),
                                ))
                                .await;
                        }
                    }
                }
            });
        }
    });

    let controller = BrowserController::new(http_port);

    // Multiline string with double quotes, single quotes, backslashes, and special characters
    let complex_text = "Line 1\nLine 2 with \"double quotes\", 'single quotes', and backslashes \\path\\to\\dir.";
    let complex_selector = "textarea[data-test='user\"input']";

    controller.type_text(complex_selector, complex_text).await.unwrap();

    let recorded_expr = received_expression.lock().await.clone();
    // Verify JSON-escaped string literal is present in JS expression
    assert!(
        recorded_expr.contains(r#""textarea[data-test='user\"input']""#),
        "Selector should be safely JSON encoded in JS expression: {}",
        recorded_expr
    );
    assert!(
        recorded_expr.contains(r#""Line 1\nLine 2 with \"double quotes\", 'single quotes', and backslashes \\path\\to\\dir.""#),
        "Text should be safely JSON encoded in JS expression: {}",
        recorded_expr
    );

    http_handle.abort();
    ws_handle.abort();
}

#[tokio::test]
async fn test_mock_cdp_click_element_not_found() {
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
                            // Return false to simulate element not found in DOM
                            let resp = json!({
                                "id": id,
                                "result": {
                                    "result": { "type": "boolean", "value": false }
                                }
                            });
                            let _ = ws
                                .send(tokio_tungstenite::tungstenite::Message::Text(
                                    resp.to_string(),
                                ))
                                .await;
                        }
                    }
                }
            });
        }
    });

    let controller = BrowserController::new(http_port);
    let res = controller.click("#nonexistent-button").await;
    match res {
        Err(BrowserError::Protocol(msg)) => {
            assert!(msg.contains("Element not found for selector"));
        }
        other => panic!("Expected Element not found Protocol error, got {:?}", other),
    }

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
                                    resp.to_string(),
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

#[tokio::test]
async fn test_browser_credential_vault_lifecycle() {
    let temp_dir = tempfile::tempdir().unwrap();
    let ws_path = temp_dir.path().to_path_buf();

    let controller = BrowserController::with_workspace(19222, ws_path.clone());
    assert_eq!(controller.list_saved_credentials().await.len(), 0);

    // Save first credential
    let cred1 = controller
        .save_credential(
            "github.com",
            "https://github.com",
            "dev_user",
            "secret_pass_123",
        )
        .await;

    assert_eq!(cred1.domain, "github.com");
    assert_eq!(cred1.username, "dev_user");
    assert_eq!(cred1.password, "secret_pass_123");

    // List credentials
    let list = controller.list_saved_credentials().await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, cred1.id);

    // Get for URL
    let found = controller
        .get_credential_for_url("https://github.com/login")
        .await;
    assert!(found.is_some());
    assert_eq!(found.unwrap().username, "dev_user");

    // File was written to disk
    let creds_file = ws_path.join(".saved_passwords.json");
    assert!(creds_file.exists());
    let file_content = std::fs::read_to_string(&creds_file).unwrap();
    assert!(file_content.contains("dev_user"));
    assert!(file_content.contains("secret_pass_123"));

    // Reloading a new controller on same workspace loads saved passwords
    let controller2 = BrowserController::with_workspace(19223, ws_path.clone());
    let reloaded = controller2.list_saved_credentials().await;
    assert_eq!(reloaded.len(), 1);
    assert_eq!(reloaded[0].username, "dev_user");

    // Delete credential
    let deleted = controller2.delete_saved_credential(&cred1.id).await;
    assert!(deleted);
    assert_eq!(controller2.list_saved_credentials().await.len(), 0);

    // File was updated
    let file_content_after = std::fs::read_to_string(&creds_file).unwrap();
    assert!(!file_content_after.contains("dev_user"));
}

#[tokio::test]
async fn test_mock_cdp_interactive_control() {
    let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_port = http_listener.local_addr().unwrap().port();

    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();

    let recorded_events = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<(String, Value)>::new()));
    let rec_clone = recorded_events.clone();

    // Spawn HTTP listener for /json/version
    let _http_handle = tokio::spawn(async move {
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
    let _ws_handle = tokio::spawn(async move {
        while let Ok((stream, _)) = ws_listener.accept().await {
            let rec = rec_clone.clone();
            tokio::spawn(async move {
                if let Ok(mut ws) = accept_async(stream).await {
                    while let Some(Ok(msg)) = ws.next().await {
                        if let tokio_tungstenite::tungstenite::Message::Text(txt) = msg {
                            let parsed: Value = serde_json::from_str(&txt).unwrap();
                            let id = parsed["id"].as_u64().unwrap();
                            let method = parsed["method"].as_str().unwrap().to_string();
                            let params = parsed.get("params").cloned().unwrap_or(Value::Null);

                            {
                                let mut events = rec.lock().await;
                                events.push((method.clone(), params));
                            }

                            let result_val = match method.as_str() {
                                "Page.captureScreenshot" => json!({ "data": "dGVzdC1pbnRlcmFjdGl2ZS1zY3JlZW4=" }),
                                _ => json!({}),
                            };

                            let resp = json!({
                                "id": id,
                                "result": result_val
                            });

                            let _ = ws
                                .send(tokio_tungstenite::tungstenite::Message::Text(
                                    resp.to_string(),
                                ))
                                .await;
                        }
                    }
                }
            });
        }
    });

    let controller = BrowserController::new(http_port);

    // 1. Test click interaction
    let shot1 = controller.interact_click(450, 220).await.unwrap();
    assert_eq!(shot1, "dGVzdC1pbnRlcmFjdGl2ZS1zY3JlZW4=");

    // 2. Test typing interaction
    let shot2 = controller.interact_type("hello kwnt").await.unwrap();
    assert_eq!(shot2, "dGVzdC1pbnRlcmFjdGl2ZS1zY3JlZW4=");

    // 3. Test key press interaction
    let shot3 = controller.interact_key("Enter", Some("Enter")).await.unwrap();
    assert_eq!(shot3, "dGVzdC1pbnRlcmFjdGl2ZS1zY3JlZW4=");

    // 4. Test scroll interaction
    let shot4 = controller.interact_scroll(450, 220, 0.0, 100.0).await.unwrap();
    assert_eq!(shot4, "dGVzdC1pbnRlcmFjdGl2ZS1zY3JlZW4=");

    // Verify recorded commands
    let events = recorded_events.lock().await;
    let methods: Vec<&str> = events.iter().map(|(m, _)| m.as_str()).collect();

    assert!(methods.contains(&"Input.dispatchMouseEvent"));
    assert!(methods.contains(&"Input.insertText"));
    assert!(methods.contains(&"Input.dispatchKeyEvent"));
    assert!(methods.contains(&"Page.captureScreenshot"));
}


#[test]
fn detects_captcha_and_bot_check_pages() {
    use kwnt::sandbox::detect_bot_wall;
    assert!(detect_bot_wall(
        "https://www.google.com/sorry/index?continue=https://www.google.com/search%3Fq%3Dcat",
        "https://www.google.com/search?q=cat",
        "About this page Our systems have detected unusual traffic from your computer network.",
    )
    .is_some());
    assert!(detect_bot_wall("https://example.org/", "Just a moment...", "").is_some());
    assert!(detect_bot_wall("https://shop.example/", "Shop", "Please verify you are human to continue").is_some());
    assert!(detect_bot_wall("https://www.bing.com/images/search?q=cat", "cat - Bing images", "Cat pictures, kittens").is_none());
}
