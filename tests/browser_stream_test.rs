use axum::{extract::{State, Path, WebSocketUpgrade, ws::Message}, routing::get, Json, Router};
use futures::{SinkExt, StreamExt};
use kwnt::{config::Config, db::Database, tools::ToolRunner, broker::PermissionBroker, router::LayaRouter, server::{AppState, create_router}, sandbox::service::BrowserService};
use serde_json::{json, Value};
use std::{sync::{Arc, atomic::{AtomicUsize, Ordering}}, time::Duration};
use tokio::{net::TcpListener, sync::Mutex, time::timeout};
use tokio_tungstenite::{connect_async, tungstenite::{client::IntoClientRequest, Message as WsMessage}};

#[derive(Clone, Default)]
struct Mock {
    port: u16,
    targets: Arc<Mutex<Vec<Value>>>,
    commands: Arc<Mutex<Vec<Value>>>,
    contexts: Arc<AtomicUsize>,
}

async fn cdp(State(mock): State<Mock>, Path(_id): Path<String>, ws: WebSocketUpgrade) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |mut socket| async move {
        while let Some(Ok(Message::Text(text))) = socket.next().await {
            let cmd: Value = serde_json::from_str(&text).unwrap();
            mock.commands.lock().await.push(cmd.clone());
            let method = cmd["method"].as_str().unwrap();
            let result = match method {
                "Target.createBrowserContext" => json!({"browserContextId":format!("context-{}",mock.contexts.fetch_add(1,Ordering::SeqCst))}),
                "Target.createTarget" => {
                    assert!(cmd["params"].get("width").is_none(), "viewport dimensions belong in Emulation, not createTarget without newWindow");
                    assert!(cmd["params"].get("height").is_none());
                    let mut targets = mock.targets.lock().await;
                    let id = format!("page-{}",mock.contexts.load(Ordering::SeqCst));
                    targets.push(json!({"targetId":id,"browserContextId":cmd["params"]["browserContextId"]}));
                    json!({"targetId":id})
                }
                "Target.getTargets" => json!({"targetInfos":mock.targets.lock().await.clone()}),
                "Target.disposeBrowserContext" => {
                    mock.targets.lock().await.retain(|target| target["browserContextId"] != cmd["params"]["browserContextId"]);
                    json!({})
                }
                "Runtime.evaluate" => json!({"result":{"value":{"url":"https://example.com","title":"Example"}}}),
                "Page.captureScreenshot" => json!({"data":"fresh-frame"}),
                _ => json!({}),
            };
            if socket.send(Message::Text(json!({"id":cmd["id"],"result":result}).to_string())).await.is_err() { break; }
            if method == "Page.startScreencast" {
                let _ = socket.send(Message::Text(json!({"method":"Page.screencastFrame","params":{"sessionId":7,"data":"streamed-frame"}}).to_string())).await;
            }
        }
    })
}

async fn mock() -> (Mock, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock = Mock { port: listener.local_addr().unwrap().port(), ..Mock::default() };
    let app = Router::new()
        .route("/json/version",get(|headers: axum::http::HeaderMap| async move {
            assert_eq!(headers["host"], "localhost", "Chromium requires localhost or an IP Host, even when reached by Docker DNS");
            Json(json!({"webSocketDebuggerUrl":"ws://unreachable-container:9223/devtools/browser/root"}))
        }))
        .route("/json/list",get(|State(m):State<Mock>| async move {
            Json(m.targets.lock().await.iter().map(|t| json!({"type":"page","id":t["targetId"],
                "webSocketDebuggerUrl":format!("ws://unreachable-container:9223/devtools/page/{}",t["targetId"].as_str().unwrap())})).collect::<Vec<_>>())
        }))
        .route("/devtools/browser/:id", get(cdp)).route("/devtools/page/:id",get(cdp)).with_state(mock.clone());
    let task = tokio::spawn(async move { axum::serve(listener,app).await.unwrap(); });
    (mock,task)
}

#[tokio::test]
async fn sessions_are_isolated_reused_and_recover_after_browser_restart() {
    let (mock, task) = mock().await;
    let tmp = tempfile::tempdir().unwrap();
    let endpoint = format!("http://127.0.0.1:{}",mock.port);
    let service = BrowserService::new(tmp.path().into(),Some(endpoint.clone()));
    let a = service.session("bot-a",tmp.path()).await.unwrap();
    let b = service.session("bot-b",tmp.path()).await.unwrap();
    assert_ne!(a.get_ws_url().await.unwrap(),b.get_ws_url().await.unwrap());
    assert_eq!(mock.contexts.load(Ordering::SeqCst),2);
    drop(service);
    let service = BrowserService::new(tmp.path().into(),Some(endpoint));
    assert_eq!(service.session("bot-a",tmp.path()).await.unwrap().get_ws_url().await.unwrap(),a.get_ws_url().await.unwrap());
    assert_eq!(mock.contexts.load(Ordering::SeqCst),2,"server restart reuses the context");
    mock.targets.lock().await.clear();
    service.session("bot-a",tmp.path()).await.unwrap();
    assert_eq!(mock.contexts.load(Ordering::SeqCst),3,"browser restart recreates missing context");
    task.abort();
}

async fn receive(ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, kind: &str) -> Value {
    timeout(Duration::from_secs(5),async {
        loop {
            let message = ws.next().await.unwrap().unwrap();
            if let WsMessage::Text(text) = message {
                let value:Value=serde_json::from_str(&text).unwrap();
                if value["type"] == kind { return value; }
                if value["type"] == "ping" { ws.send(WsMessage::Text(json!({"action":"pong"}).to_string())).await.unwrap(); }
            }
        }
    }).await.unwrap()
}

#[tokio::test]
async fn stream_input_control_lease_and_origin_protection() {
    let (mock, cdp_task) = mock().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = Config {data_dir:tmp.path().into(),browser_cdp_url:Some(format!("http://127.0.0.1:{}",mock.port)),..Config::default()};
    let db = Database::init(tmp.path()).await.unwrap();
    let id=db.list_kwants().await.unwrap()[0].id.clone();
    let state=AppState::new(config.clone(),db,ToolRunner::new(tmp.path().join("workspace")),PermissionBroker::new(),LayaRouter::new(config.laya_url));
    let ctrl=kwnt::server::routes::get_or_create_browser_controller(&state,&id).await.unwrap();
    assert!(state.sandbox_manager.get_sandbox(&id).await.is_none(),"browser must not start an execution container");
    let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port=listener.local_addr().unwrap().port();
    let app=create_router(state);
    let app_task=tokio::spawn(async move {axum::serve(listener,app).await.unwrap();});
    let url=format!("ws://127.0.0.1:{port}/api/sandbox/{id}/stream");
    let mut request=url.clone().into_client_request().unwrap();
    request.headers_mut().insert("Origin","https://other-site.example".parse().unwrap());
    assert!(connect_async(request).await.is_err());
    let mut request=url.into_client_request().unwrap();
    request.headers_mut().insert("Origin",format!("http://127.0.0.1:{port}").parse().unwrap());
    let (mut ws,_)=connect_async(request).await.unwrap();
    let frame=receive(&mut ws,"frame").await;
    assert_eq!(frame["image"],"streamed-frame");
    assert!(!mock.commands.lock().await.iter().any(|c|c["method"]=="Page.screencastFrameAck"));
    ws.send(WsMessage::Text(json!({"action":"frame_ack","id":frame["id"]}).to_string())).await.unwrap();
    ws.send(WsMessage::Text(json!({"action":"take_control"}).to_string())).await.unwrap();
    assert_eq!(receive(&mut ws,"control").await["active"],true);
    assert!(ctrl.navigate("https://example.com").await.unwrap_err().to_string().contains("controlled"));
    let response = reqwest::Client::new().post(format!("http://127.0.0.1:{port}/api/sandbox/{id}/navigate"))
        .json(&json!({"url":"https://example.com"})).send().await.unwrap();
    assert_eq!(response.status(),reqwest::StatusCode::CONFLICT);
    assert!(ctrl.take_control().await.is_err(),"second viewer cannot take control");
    mock.commands.lock().await.clear();
    for (i,payload) in [json!({"action":"click","x":50,"y":60}),json!({"action":"type","text":"abc"}),json!({"action":"key","key":"Enter","code":"Enter"})].into_iter().enumerate() {
        let mut payload=payload; payload["id"]=json!(i);
        ws.send(WsMessage::Text(payload.to_string())).await.unwrap();
        assert_eq!(receive(&mut ws,"ack").await["id"],i);
    }
    let commands=mock.commands.lock().await.clone();
    assert!(!commands.iter().any(|c|c["method"]=="Page.captureScreenshot"),"input must not wait for screenshots");
    let input:Vec<_>=commands.iter().filter(|c|c["method"].as_str().unwrap().starts_with("Input.")).collect();
    assert_eq!(input[0]["params"]["type"],"mousePressed");
    assert_eq!(input[1]["params"]["type"],"mouseReleased");
    assert_eq!(input[2]["params"]["text"],"abc");
    ws.send(WsMessage::Text(json!({"action":"release_control"}).to_string())).await.unwrap();
    assert_eq!(receive(&mut ws,"control").await["active"],false);
    assert!(ctrl.control.try_read().is_ok());
    ws.send(WsMessage::Text(json!({"action":"take_control"}).to_string())).await.unwrap();
    assert_eq!(receive(&mut ws,"control").await["active"],true);
    ws.close(None).await.unwrap();
    timeout(Duration::from_secs(2),async {loop {if ctrl.control.try_read().is_ok(){break;} tokio::task::yield_now().await;}}).await.unwrap();
    app_task.abort(); cdp_task.abort();
}

/// Opt in after building docker/browser.Dockerfile. Uses disposable data and a local HTML page.
#[tokio::test]
#[ignore = "requires Docker and the kwnt-browser:v1 image"]
async fn live_chromium_stream_and_isolated_sessions() {
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::WARN).try_init();
    let tmp = tempfile::tempdir().unwrap();
    let config = Config { data_dir: tmp.path().into(), ..Config::default() };
    let db = Database::init(tmp.path()).await.unwrap();
    let bots = db.list_kwants().await.unwrap();
    let id = bots[0].id.clone();
    let state = AppState::new(config.clone(), db, ToolRunner::new(tmp.path().join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url));
    let ctrl = kwnt::server::routes::get_or_create_browser_controller(&state, &id).await.expect("real Chromium session");
    let html = "<title>Kwnt live check</title><style>body{font:30px sans-serif}input,button{font:30px sans-serif;position:absolute;left:40px}input{top:40px;width:400px;height:50px}button{top:120px;width:220px;height:60px}</style><input id=field><button onclick=\"document.title='Clicked'\">Click target</button>";
    let encoded: String = html.bytes().map(|byte| format!("%{byte:02X}")).collect();
    ctrl.navigate(&format!("data:text/html,{encoded}")).await.unwrap();
    let second = kwnt::server::routes::get_or_create_browser_controller(&state, &bots[1].id).await.unwrap();
    assert_ne!(ctrl.get_ws_url().await.unwrap(), second.get_ws_url().await.unwrap());
    assert_eq!(*second.current_url.lock().await, Some("about:blank".into()));
    assert!(state.sandbox_manager.get_sandbox(&id).await.is_none());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = create_router(state.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    let mut request = format!("ws://127.0.0.1:{port}/api/sandbox/{id}/stream").into_client_request().unwrap();
    request.headers_mut().insert("Origin", format!("http://127.0.0.1:{port}").parse().unwrap());
    let (mut ws, _) = connect_async(request).await.unwrap();
    let first_frame = receive(&mut ws, "frame").await;
    assert!(first_frame["image"].as_str().unwrap().len() > 1000);
    ws.send(WsMessage::Text(json!({"action":"frame_ack","id":first_frame["id"]}).to_string())).await.unwrap();
    ws.send(WsMessage::Text(json!({"action":"take_control"}).to_string())).await.unwrap();
    assert_eq!(receive(&mut ws, "control").await["active"], true);
    assert!(matches!(ctrl.navigate("about:blank").await, Err(kwnt::sandbox::BrowserError::HumanControl)));

    let inputs = [json!({"action":"click","x":80,"y":60}), json!({"action":"type","text":"streaming works"}), json!({"action":"click","x":100,"y":150})];
    let start = std::time::Instant::now();
    for (index, mut input) in inputs.into_iter().enumerate() {
        input["id"] = json!(index);
        ws.send(WsMessage::Text(input.to_string())).await.unwrap();
        assert_eq!(receive(&mut ws, "ack").await["id"], index);
    }
    let elapsed = start.elapsed();
    let result = ctrl.send_cdp_command("Runtime.evaluate", json!({"expression":"({value:document.querySelector('#field').value,title:document.title})","returnByValue":true})).await.unwrap();
    assert_eq!(result["result"]["value"]["value"], "streaming works");
    assert_eq!(result["result"]["value"]["title"], "Clicked");
    eprintln!("live Chromium: click + type + click acknowledgements in {} ms", elapsed.as_millis());
    ws.close(None).await.unwrap();
    timeout(Duration::from_secs(3), async { loop { if ctrl.control.try_read().is_ok() { break; } tokio::task::yield_now().await; } }).await.unwrap();
    let execution = kwnt::server::routes::execute_tool(&state, Some(&id), "execute_shell", &json!({
        "command": "python3 -c 'print(6 * 7)'; if command -v chromium; then exit 1; fi"
    })).await.unwrap();
    assert_eq!(execution["success"], true);
    assert_eq!(execution["stdout"], "42\n");
    state.sandbox_manager.stop_sandbox(&id).await.unwrap();
    state.browser_service.reset(&id).await.unwrap();
    state.browser_service.reset(&bots[1].id).await.unwrap();
    task.abort();
}
