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
    /// What the page answers for `location.href`; empty means https://example.com.
    page_url: Arc<Mutex<String>>,
    /// Reading the page's address fails, the way Chromium answers between two documents.
    between_documents: Arc<std::sync::atomic::AtomicBool>,
    /// Every open DevTools socket, to push page events the way Chromium does.
    sockets: Arc<Mutex<Vec<tokio::sync::mpsc::UnboundedSender<String>>>>,
    /// Console lines the page logged at load, replayed when Runtime or Log is turned on.
    console_lines: Arc<AtomicUsize>,
    /// How the page answers the live view's input check: 0 not at all, 1 a live tab, 2 a tab
    /// behind Chrome's "Open app?" prompt (scripts run, input never arrives).
    input: Arc<std::sync::atomic::AtomicU8>,
}

/// Sends a page event on every open DevTools socket.
async fn push_event(mock: &Mock, method: &str, params: Value) {
    let event = json!({"method":method,"params":params}).to_string();
    for socket in mock.sockets.lock().await.iter() { let _ = socket.send(event.clone()); }
}

async fn cdp(State(mock): State<Mock>, Path(_id): Path<String>, ws: WebSocketUpgrade) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |mut socket| async move {
        let (push, mut pushed) = tokio::sync::mpsc::unbounded_channel::<String>();
        mock.sockets.lock().await.push(push);
        loop {
            let text = tokio::select! {
                Some(event) = pushed.recv() => { if socket.send(Message::Text(event)).await.is_err() { break; } continue; }
                message = socket.next() => match message { Some(Ok(Message::Text(text))) => text, _ => break },
            };
            let cmd: Value = serde_json::from_str(&text).unwrap();
            mock.commands.lock().await.push(cmd.clone());
            let method = cmd["method"].as_str().unwrap();
            if method == "Runtime.evaluate" && mock.between_documents.load(Ordering::SeqCst)
                && cmd["params"]["expression"].as_str().unwrap_or_default().contains("location.href") {
                let error = json!({"id":cmd["id"],"error":{"code":-32000,"message":"Cannot find context with specified id"}});
                if socket.send(Message::Text(error.to_string())).await.is_err() { break; }
                continue;
            }
            // Chromium replays what the page already logged before it answers
            let replay = |event: &str, n: usize| (0..n).map(|i| match event {
                "Runtime.consoleAPICalled" => json!({"method":event,"params":{"type":"log","args":[{"type":"string","value":format!("line {i}")}],"executionContextId":1,"timestamp":i}}),
                _ => json!({"method":event,"params":{"entry":{"source":"javascript","level":"info","text":format!("line {i}"),"timestamp":i}}}),
            }).collect::<Vec<_>>();
            let lines = mock.console_lines.load(Ordering::SeqCst);
            let replayed = match method {
                "Runtime.enable" => [replay("Runtime.consoleAPICalled", lines), (1..=3).map(|i| json!({"method":"Runtime.executionContextCreated","params":{"context":{"id":i}}})).collect()].concat(),
                "Log.enable" => replay("Log.entryAdded", lines),
                _ => vec![],
            };
            let mut gone = false;
            for event in replayed { if socket.send(Message::Text(event.to_string())).await.is_err() { gone = true; break; } }
            if gone { break; }
            let probe = cmd["params"]["expression"].as_str().is_some_and(|e| e.contains("kwnt.inputProbe")) && mock.input.load(Ordering::SeqCst) != 0;
            let result = match method {
                "Runtime.evaluate" if probe => {
                    let stuck = mock.input.load(Ordering::SeqCst) == 2;
                    let arming = cmd["params"]["expression"].as_str().unwrap().contains("elementFromPoint");
                    json!({"result":{"value": if arming { json!([8, 8]) } else if stuck { json!(0) } else { json!(2) }}})
                }
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
                // the page has a link at (300, 300); everything else has the default cursor
                "Runtime.evaluate" if cmd["params"]["expression"].as_str().unwrap_or_default().contains("getComputedStyle") => {
                    let link = cmd["params"]["expression"].as_str().unwrap().contains("elementFromPoint(300, 300)");
                    json!({"result":{"value": if link {"pointer"} else {"auto"}}})
                }
                "Runtime.evaluate" => {
                    let url = mock.page_url.lock().await.clone();
                    let url = if url.is_empty() { "https://example.com".to_string() } else { url };
                    json!({"result":{"value":{"url":url,"title":"Example"}}})
                }
                "Page.captureScreenshot" => json!({"data":"fresh-frame"}),
                // the tab is on a sign-in page; it steps to about:blank when told to
                "Page.getNavigationHistory" => {
                    let blank = mock.commands.lock().await.iter().any(|c| c["method"] == "Page.navigate" && c["params"]["url"] == "about:blank");
                    let mut entries = vec![json!({"id":1,"url":"https://example.com/"}), json!({"id":2,"url":"https://www.tiktok.com/login/qrcode"})];
                    if blank { entries.push(json!({"id":3,"url":"about:blank"})); }
                    json!({"currentIndex": entries.len() - 1, "entries": entries})
                }
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
    let id=db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap()[0].id.clone();
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
        let ack=receive(&mut ws,"ack").await;
        assert_eq!(ack["id"],i);
        // a tap reports whether it focused a text field (the mock page has none); typing doesn't
        if i==0 { assert_eq!(ack["editable"],false); } else { assert!(ack.get("editable").is_none()); }
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

async fn mouse_events(mock: &Mock) -> Vec<(String, i64, String)> {
    mock.commands.lock().await.iter().filter(|c| c["method"] == "Input.dispatchMouseEvent").map(|c| (
        c["params"]["type"].as_str().unwrap().to_string(),
        c["params"]["buttons"].as_i64().unwrap_or(-1),
        c["params"]["button"].as_str().unwrap_or("").to_string(),
    )).collect()
}

#[tokio::test]
async fn pointer_drags_press_move_release_in_order_and_let_go_on_release_or_disconnect() {
    let (mock, cdp_task) = mock().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = Config {data_dir:tmp.path().into(),browser_cdp_url:Some(format!("http://127.0.0.1:{}",mock.port)),..Config::default()};
    let db = Database::init(tmp.path()).await.unwrap();
    let id=db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap()[0].id.clone();
    let state=AppState::new(config.clone(),db,ToolRunner::new(tmp.path().join("workspace")),PermissionBroker::new(),LayaRouter::new(config.laya_url));
    let ctrl=kwnt::server::routes::get_or_create_browser_controller(&state,&id).await.unwrap();
    let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port=listener.local_addr().unwrap().port();
    let app=create_router(state);
    let app_task=tokio::spawn(async move {axum::serve(listener,app).await.unwrap();});
    let connect = || async {
        let mut request=format!("ws://127.0.0.1:{port}/api/sandbox/{id}/stream").into_client_request().unwrap();
        request.headers_mut().insert("Origin",format!("http://127.0.0.1:{port}").parse().unwrap());
        let (mut ws,_)=connect_async(request).await.unwrap();
        receive(&mut ws,"frame").await;
        ws.send(WsMessage::Text(json!({"action":"take_control"}).to_string())).await.unwrap();
        assert_eq!(receive(&mut ws,"control").await["active"],true);
        ws
    };
    let mut ws = connect().await;
    let send = |payload: Value| WsMessage::Text(payload.to_string());

    // pointer input is bounds-checked like a click
    mock.commands.lock().await.clear();
    ws.send(send(json!({"action":"pointer_down","x":10,"y":10,"id":"out"}))).await.unwrap();
    assert_eq!(receive(&mut ws,"ack").await["id"],"out");
    ws.send(send(json!({"action":"pointer_down","x":1280,"y":10,"id":"edge"}))).await.unwrap();
    assert!(receive(&mut ws,"error").await["error"].as_str().unwrap().contains("outside"), "same bounds as click");
    ws.send(send(json!({"action":"pointer_up","x":10,"y":10,"id":"reset"}))).await.unwrap();
    receive(&mut ws,"ack").await;
    mock.commands.lock().await.clear();

    // a slider drag: press, two moves while held, release
    for (i, (action, x)) in [("pointer_down",100),("pointer_move",150),("pointer_move",220),("pointer_up",220),("pointer_move",230),("pointer_up",230)].into_iter().enumerate() {
        ws.send(send(json!({"action":action,"x":x,"y":400,"id":i}))).await.unwrap();
        assert_eq!(receive(&mut ws,"ack").await["id"],i);
    }
    assert_eq!(mouse_events(&mock).await, vec![
        ("mousePressed".into(),1,"left".into()),
        ("mouseMoved".into(),1,"left".into()),
        ("mouseMoved".into(),1,"left".into()),
        ("mouseReleased".into(),0,"left".into()),
        // a hover after the drag is a plain move, and a stray pointer_up sends nothing
        ("mouseMoved".into(),0,"none".into()),
    ]);
    assert!(ctrl.pointer_held().is_none());

    // pressed, then control released: the button is let go where it was
    mock.commands.lock().await.clear();
    ws.send(send(json!({"action":"pointer_down","x":300,"y":400,"id":"d1"}))).await.unwrap();
    receive(&mut ws,"ack").await;
    ws.send(send(json!({"action":"pointer_move","x":340,"y":405,"id":"m1"}))).await.unwrap();
    receive(&mut ws,"ack").await;
    ws.send(send(json!({"action":"release_control"}))).await.unwrap();
    assert_eq!(receive(&mut ws,"control").await["active"],false);
    let events = mouse_events(&mock).await;
    assert_eq!(events.last().unwrap(), &("mouseReleased".into(),0,"left".into()));
    let released = mock.commands.lock().await.iter().rev().find(|c| c["params"]["type"]=="mouseReleased").cloned().unwrap();
    assert_eq!((released["params"]["x"].as_f64(), released["params"]["y"].as_f64()), (Some(340.0), Some(405.0)));
    ws.close(None).await.unwrap();

    // pressed, then the socket drops mid-drag: released too, and bot actions come back
    let mut ws = connect().await;
    mock.commands.lock().await.clear();
    ws.send(send(json!({"action":"pointer_down","x":500,"y":420,"id":"d2"}))).await.unwrap();
    receive(&mut ws,"ack").await;
    drop(ws);
    timeout(Duration::from_secs(5),async {loop {
        if ctrl.control.try_read().is_ok() && ctrl.pointer_held().is_none() {break;}
        tokio::time::sleep(Duration::from_millis(20)).await;
    }}).await.unwrap();
    assert_eq!(mouse_events(&mock).await, vec![("mousePressed".into(),1,"left".into()),("mouseReleased".into(),0,"left".into())]);
    app_task.abort(); cdp_task.abort();
}

/// Messages up to and including the ack or error for `id`, skipping frames, pings and locations.
async fn until_reply(ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, id: &str) -> Vec<Value> {
    timeout(Duration::from_secs(5), async {
        let mut seen = vec![];
        loop {
            if let WsMessage::Text(text) = ws.next().await.unwrap().unwrap() {
                let value: Value = serde_json::from_str(&text).unwrap();
                match value["type"].as_str().unwrap_or_default() {
                    "ping" => ws.send(WsMessage::Text(json!({"action":"pong"}).to_string())).await.unwrap(),
                    "frame" => ws.send(WsMessage::Text(json!({"action":"frame_ack","id":value["id"]}).to_string())).await.unwrap(),
                    "location" => {}
                    _ => {
                        let done = value["id"] == id && matches!(value["type"].as_str(), Some("ack" | "error"));
                        seen.push(value);
                        if done { return seen; }
                    }
                }
            }
        }
    }).await.unwrap()
}

#[tokio::test]
async fn protocol_v2_mouse_keys_wheel_and_resize_reach_the_page_in_order_and_restore() {
    let (mock, cdp_task) = mock().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = Config {data_dir:tmp.path().into(),browser_cdp_url:Some(format!("http://127.0.0.1:{}",mock.port)),..Config::default()};
    let db = Database::init(tmp.path()).await.unwrap();
    let id=db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap()[0].id.clone();
    let state=AppState::new(config.clone(),db,ToolRunner::new(tmp.path().join("workspace")),PermissionBroker::new(),LayaRouter::new(config.laya_url));
    let ctrl=kwnt::server::routes::get_or_create_browser_controller(&state,&id).await.unwrap();
    let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port=listener.local_addr().unwrap().port();
    let app_task=tokio::spawn(async move {axum::serve(listener,create_router(state)).await.unwrap();});
    let mut request=format!("ws://127.0.0.1:{port}/api/sandbox/{id}/stream").into_client_request().unwrap();
    request.headers_mut().insert("Origin",format!("http://127.0.0.1:{port}").parse().unwrap());
    let (mut ws,_)=connect_async(request).await.unwrap();
    let frame = receive(&mut ws,"frame").await;
    assert_eq!((frame["width"].as_f64(), frame["height"].as_f64()), (Some(1280.0), Some(800.0)));
    let send = |payload: Value| WsMessage::Text(payload.to_string());

    // input needs control first
    ws.send(send(json!({"action":"mouse","type":"down","x":5,"y":5,"button":"left","id":"early"}))).await.unwrap();
    assert_eq!(until_reply(&mut ws,"early").await.last().unwrap()["error"], "Take control of the browser first.");
    ws.send(send(json!({"action":"take_control"}))).await.unwrap();
    let control = receive(&mut ws,"control").await;
    assert_eq!((control["active"].clone(), control["viewport"]["width"].as_f64()), (json!(true), Some(1280.0)));
    mock.commands.lock().await.clear();

    // a hover move is never acked; a right press with Shift and its release are, in order
    ws.send(send(json!({"action":"mouse","type":"move","x":10,"y":20,"id":"m"}))).await.unwrap();
    ws.send(send(json!({"action":"mouse","type":"down","x":10,"y":20,"button":"right","modifiers":8,"id":"d"}))).await.unwrap();
    ws.send(send(json!({"action":"mouse","type":"up","x":10,"y":20,"button":"right","modifiers":8,"id":"u"}))).await.unwrap();
    let seen = until_reply(&mut ws,"u").await;
    assert!(!seen.iter().any(|m| m["id"]=="m"), "moves are fire-and-forget: {seen:?}");
    // a double click: the second press carries clickCount 2
    for (i, (kind, count)) in [("down",1),("up",1),("down",2),("up",2)].into_iter().enumerate() {
        ws.send(send(json!({"action":"mouse","type":kind,"x":30,"y":40,"button":"left","click_count":count,"id":format!("c{i}")}))).await.unwrap();
    }
    until_reply(&mut ws,"c3").await;
    let mouse: Vec<_> = mock.commands.lock().await.iter().filter(|c| c["method"]=="Input.dispatchMouseEvent").map(|c| c["params"].clone()).collect();
    let shape: Vec<_> = mouse.iter().map(|p| (p["type"].as_str().unwrap().to_string(), p["button"].as_str().unwrap().to_string(), p["buttons"].as_i64().unwrap(), p["clickCount"].as_i64().unwrap_or(0), p["modifiers"].as_i64().unwrap())).collect();
    assert_eq!(shape, vec![
        ("mouseMoved".into(),"none".into(),0,0,0),
        ("mousePressed".into(),"right".into(),2,1,8),
        ("mouseReleased".into(),"right".into(),0,1,8),
        ("mousePressed".into(),"left".into(),1,1,0),
        ("mouseReleased".into(),"left".into(),0,1,0),
        ("mousePressed".into(),"left".into(),1,2,0),
        ("mouseReleased".into(),"left".into(),0,2,0),
    ]);
    mock.commands.lock().await.clear();

    // keys: Ctrl+A is a raw key down with the selectAll command, a letter types its text,
    // Shift+Tab is a raw key with Shift, an unknown code is refused
    for (i, (key, code, modifiers)) in [("a","KeyA",2),("b","KeyB",0),("Tab","Tab",8)].into_iter().enumerate() {
        ws.send(send(json!({"action":"key","type":"down","key":key,"code":code,"modifiers":modifiers,"id":format!("k{i}")}))).await.unwrap();
        assert_eq!(until_reply(&mut ws,&format!("k{i}")).await.last().unwrap()["type"],"ack");
    }
    ws.send(send(json!({"action":"key","type":"down","key":"x","code":"Bogus","id":"bad"}))).await.unwrap();
    assert!(until_reply(&mut ws,"bad").await.last().unwrap()["error"].as_str().unwrap().contains("Unsupported key code"));
    let keys: Vec<_> = mock.commands.lock().await.iter().filter(|c| c["method"]=="Input.dispatchKeyEvent").map(|c| c["params"].clone()).collect();
    assert_eq!(keys.len(), 3);
    assert_eq!((keys[0]["type"].clone(), keys[0]["windowsVirtualKeyCode"].clone(), keys[0]["commands"].clone(), keys[0].get("text")), (json!("rawKeyDown"), json!(65), json!(["selectAll"]), None));
    assert_eq!((keys[1]["type"].clone(), keys[1]["text"].clone()), (json!("keyDown"), json!("b")));
    assert_eq!((keys[2]["type"].clone(), keys[2]["windowsVirtualKeyCode"].clone(), keys[2]["modifiers"].clone()), (json!("rawKeyDown"), json!(9), json!(8)));
    mock.commands.lock().await.clear();

    // wheel carries modifiers and is not acked
    ws.send(send(json!({"action":"wheel","x":100,"y":100,"delta_y":120,"modifiers":2,"id":"w"}))).await.unwrap();
    ws.send(send(json!({"action":"type","text":"z","id":"t"}))).await.unwrap();
    assert!(!until_reply(&mut ws,"t").await.iter().any(|m| m["id"]=="w"));
    let wheel = mock.commands.lock().await.iter().find(|c| c["params"]["type"]=="mouseWheel").cloned().unwrap();
    assert_eq!((wheel["params"]["deltaY"].as_f64(), wheel["params"]["modifiers"].as_i64()), (Some(120.0), Some(2)));

    // resize: applied, screencast restarted at device pixels, bounds follow the new viewport
    mock.commands.lock().await.clear();
    ws.send(send(json!({"action":"resize","width":900,"height":5000,"dpr":2,"id":"r"}))).await.unwrap();
    let resized = until_reply(&mut ws,"r").await.pop().unwrap();
    assert_eq!(resized["viewport"], json!({"width":900,"height":1200,"dpr":2.0}), "height clamps to 1200");
    let commands = mock.commands.lock().await.clone();
    let metrics = commands.iter().find(|c| c["method"]=="Emulation.setDeviceMetricsOverride").unwrap();
    assert_eq!((metrics["params"]["width"].clone(), metrics["params"]["height"].clone(), metrics["params"]["deviceScaleFactor"].clone()), (json!(900), json!(1200), json!(2.0)));
    let cast = commands.iter().find(|c| c["method"]=="Page.startScreencast").unwrap();
    assert_eq!((cast["params"]["maxWidth"].clone(), cast["params"]["maxHeight"].clone()), (json!(1800), json!(2400)));
    ws.send(send(json!({"action":"mouse","type":"down","x":950,"y":10,"button":"left","id":"o"}))).await.unwrap();
    assert!(until_reply(&mut ws,"o").await.last().unwrap()["error"].as_str().unwrap().contains("outside"));
    ws.send(send(json!({"action":"mouse","type":"down","x":850,"y":1100,"button":"left","id":"in"}))).await.unwrap();
    assert_eq!(until_reply(&mut ws,"in").await.last().unwrap()["type"],"ack");

    // release: the held button is let go and the bot's 1280x800 comes back
    mock.commands.lock().await.clear();
    ws.send(send(json!({"action":"release_control"}))).await.unwrap();
    let released = receive(&mut ws,"control").await;
    assert_eq!((released["active"].clone(), released["viewport"]["width"].as_f64()), (json!(false), Some(1280.0)));
    let commands = mock.commands.lock().await.clone();
    assert!(commands.iter().any(|c| c["params"]["type"]=="mouseReleased" && c["params"]["button"]=="left"));
    let metrics = commands.iter().find(|c| c["method"]=="Emulation.setDeviceMetricsOverride").unwrap();
    assert_eq!((metrics["params"]["width"].clone(), metrics["params"]["height"].clone(), metrics["params"]["deviceScaleFactor"].clone()), (json!(1280), json!(800), json!(1)));
    assert_eq!(commands.iter().find(|c| c["method"]=="Page.startScreencast").unwrap()["params"]["maxWidth"], 1280);
    assert!(ctrl.pointer_held().is_none());

    // a resize then a dropped socket: restored as well
    ws.send(send(json!({"action":"take_control"}))).await.unwrap();
    receive(&mut ws,"control").await;
    ws.send(send(json!({"action":"resize","width":700,"height":500,"id":"r2"}))).await.unwrap();
    until_reply(&mut ws,"r2").await;
    mock.commands.lock().await.clear();
    drop(ws);
    timeout(Duration::from_secs(5),async {loop {
        let restored = mock.commands.lock().await.iter().any(|c| c["method"]=="Emulation.setDeviceMetricsOverride" && c["params"]["width"]==1280);
        if restored && ctrl.control.try_read().is_ok() {break;}
        tokio::time::sleep(Duration::from_millis(20)).await;
    }}).await.unwrap();
    app_task.abort(); cdp_task.abort();
}

type Client = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A kwant server on a mock browser with one live view open on it.
async fn open_stream(mock: &Mock, tmp: &std::path::Path) -> (Client, kwnt::sandbox::BrowserController, tokio::task::JoinHandle<()>) {
    let config = Config {data_dir:tmp.into(),browser_cdp_url:Some(format!("http://127.0.0.1:{}",mock.port)),..Config::default()};
    let db = Database::init(tmp).await.unwrap();
    let id=db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap()[0].id.clone();
    let state=AppState::new(config.clone(),db,ToolRunner::new(tmp.join("workspace")),PermissionBroker::new(),LayaRouter::new(config.laya_url));
    let ctrl=kwnt::server::routes::get_or_create_browser_controller(&state,&id).await.unwrap();
    let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port=listener.local_addr().unwrap().port();
    let app_task=tokio::spawn(async move {axum::serve(listener,create_router(state)).await.unwrap();});
    let mut request=format!("ws://127.0.0.1:{port}/api/sandbox/{id}/stream").into_client_request().unwrap();
    request.headers_mut().insert("Origin",format!("http://127.0.0.1:{port}").parse().unwrap());
    let (ws,_)=connect_async(request).await.unwrap();
    (ws, ctrl, app_task)
}

/// The next `location` whose url is `url`, acking frames on the way.
async fn location_for(ws: &mut Client, url: &str, within: Duration) -> Option<Value> {
    timeout(within, async {
        loop {
            if let WsMessage::Text(text) = ws.next().await.unwrap().unwrap() {
                let value: Value = serde_json::from_str(&text).unwrap();
                match value["type"].as_str().unwrap_or_default() {
                    "ping" => ws.send(WsMessage::Text(json!({"action":"pong"}).to_string())).await.unwrap(),
                    "frame" => ws.send(WsMessage::Text(json!({"action":"frame_ack","id":value["id"]}).to_string())).await.unwrap(),
                    "location" if value["url"] == url => return value,
                    _ => {}
                }
            }
        }
    }).await.ok()
}

#[tokio::test]
async fn the_address_follows_navigation_at_once_and_the_cursor_follows_where_the_pointer_rests() {
    let (mock, cdp_task) = mock().await;
    let tmp = tempfile::tempdir().unwrap();
    let (mut ws, _ctrl, app_task) = open_stream(&mock, tmp.path()).await;
    // the first location comes from the 2 s tick, which fires at once; the next tick is 2 s away
    location_for(&mut ws, "https://example.com", Duration::from_secs(5)).await.expect("first location");
    let tick = tokio::time::Instant::now();

    // a top-level navigation (a link, back/forward/reload or a bot's navigate) is sent at once
    *mock.page_url.lock().await = "https://example.org/next".into();
    push_event(&mock, "Page.frameNavigated", json!({"frame":{"id":"main","url":"https://example.org/next"},"type":"Navigation"})).await;
    let next = location_for(&mut ws, "https://example.org/next", Duration::from_millis(1500)).await;
    assert!(next.is_some() && tick.elapsed() < Duration::from_millis(1900), "the address must not wait for the 2 s tick");
    // pushState and #fragment changes too
    *mock.page_url.lock().await = "https://example.org/next#b".into();
    push_event(&mock, "Page.navigatedWithinDocument", json!({"frameId":"main","url":"https://example.org/next#b"})).await;
    assert!(location_for(&mut ws, "https://example.org/next#b", Duration::from_millis(1500)).await.is_some());

    // the title: a new document clears the old one at once, even while the page can't be read
    // yet, and a failed read never puts the previous page's address or title back
    mock.between_documents.store(true, Ordering::SeqCst);
    *mock.page_url.lock().await = "https://example.net/".into();
    push_event(&mock, "Page.frameNavigated", json!({"frame":{"id":"main","url":"https://example.net/"},"type":"Navigation"})).await;
    let cleared = location_for(&mut ws, "https://example.net/", Duration::from_millis(1500)).await.expect("the new address at once");
    assert_eq!(cleared["title"], "", "the previous page's title must not linger");
    assert!(location_for(&mut ws, "https://example.org/next#b", Duration::from_millis(2500)).await.is_none(), "a failed read resent the previous page");
    // once the document is parsed its title arrives
    mock.between_documents.store(false, Ordering::SeqCst);
    push_event(&mock, "Page.domContentEventFired", json!({"timestamp":1.0})).await;
    let titled = location_for(&mut ws, "https://example.net/", Duration::from_millis(1500)).await.expect("the new title");
    assert_eq!(titled["title"], "Example");

    // D3: a move onto a link right after one that was looked up still gets its cursor
    ws.send(WsMessage::Text(json!({"action":"take_control"}).to_string())).await.unwrap();
    receive(&mut ws,"control").await;
    ws.send(WsMessage::Text(json!({"action":"mouse","type":"move","x":10,"y":10}).to_string())).await.unwrap();
    ws.send(WsMessage::Text(json!({"action":"mouse","type":"move","x":300,"y":300}).to_string())).await.unwrap();
    let cursor = timeout(Duration::from_secs(2), receive(&mut ws,"cursor")).await.expect("a trailing lookup at the resting point");
    assert_eq!(cursor["cursor"], "pointer");
    let probes: Vec<_> = mock.commands.lock().await.iter()
        .filter(|c| c["params"]["expression"].as_str().is_some_and(|e| e.contains("getComputedStyle")))
        .map(|c| c["params"]["expression"].as_str().unwrap().contains("elementFromPoint(300, 300)")).collect();
    assert_eq!(probes, vec![false, true], "one lookup at once, one trailing at the newest point");
    app_task.abort(); cdp_task.abort();
}

#[tokio::test]
async fn a_watcher_is_told_to_take_control_and_a_lost_frame_ack_does_not_drop_the_stream() {
    let (mock, cdp_task) = mock().await;
    let tmp = tempfile::tempdir().unwrap();
    let (mut ws, ctrl, app_task) = open_stream(&mock, tmp.path()).await;
    // the frame is never acked, like an identical picture that fires no load event
    receive(&mut ws,"frame").await;

    // D4: history without control gets the same refusal as any other input
    for (id, payload) in [("h", json!({"action":"history","direction":"back"})), ("c", json!({"action":"click","x":5,"y":5}))] {
        let mut payload = payload; payload["id"] = json!(id);
        ws.send(WsMessage::Text(payload.to_string())).await.unwrap();
        assert_eq!(receive(&mut ws,"error").await, json!({"type":"error","id":id,"error":"Take control of the browser first."}));
    }
    // another viewer holds control: then it is theirs
    let other = ctrl.take_control().await.unwrap();
    ws.send(WsMessage::Text(json!({"action":"history","direction":"reload","id":"o"}).to_string())).await.unwrap();
    assert!(receive(&mut ws,"error").await["error"].as_str().unwrap().contains("controlled by a person"));
    drop(other);

    // the server stops waiting for the lost ack and keeps the stream (and control) alive
    timeout(Duration::from_secs(9), async { loop {
        if mock.commands.lock().await.iter().any(|c| c["method"]=="Page.screencastFrameAck") { break; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }}).await.expect("the stalled frame is let go");
    ws.send(WsMessage::Text(json!({"action":"take_control"}).to_string())).await.unwrap();
    assert_eq!(receive(&mut ws,"control").await["active"], true, "the stream is still up");
    app_task.abort(); cdp_task.abort();
}

/// The next frame, answering pings on the way; `None` when none comes `within`.
async fn next_frame(ws: &mut Client, within: Duration) -> Option<Value> {
    timeout(within, async {
        loop {
            if let WsMessage::Text(text) = ws.next().await.unwrap().unwrap() {
                let value: Value = serde_json::from_str(&text).unwrap();
                match value["type"].as_str().unwrap_or_default() {
                    "ping" => ws.send(WsMessage::Text(json!({"action":"pong"}).to_string())).await.unwrap(),
                    "frame" => return value,
                    _ => {}
                }
            }
        }
    }).await.ok()
}

#[tokio::test]
async fn a_picture_drawn_while_the_client_was_busy_is_sent_next_even_on_a_page_that_stays_still() {
    let (mock, cdp_task) = mock().await;
    let tmp = tempfile::tempdir().unwrap();
    let (mut ws, _ctrl, app_task) = open_stream(&mock, tmp.path()).await;
    let red = next_frame(&mut ws, Duration::from_secs(5)).await.expect("the first picture");
    assert_eq!(red["image"], "streamed-frame");

    // N1: the ack of the red picture is lost and the page turns blue 0.6 s later, then stays
    // still, so Chrome never draws it again. After the 4-6 s stall the client must get the blue
    // picture instead of showing red until the page next changes.
    tokio::time::sleep(Duration::from_millis(600)).await;
    push_event(&mock, "Page.screencastFrame", json!({"sessionId":8,"data":"blue"})).await;
    let blue = next_frame(&mut ws, Duration::from_secs(8)).await.expect("the page's current picture after the stall");
    assert_eq!(blue["image"], "blue");
    assert_ne!(blue["id"], red["id"]);

    // the same when the client is only slow: the page changes while it decodes, and its ack
    // brings the newer picture at once
    ws.send(WsMessage::Text(json!({"action":"frame_ack","id":blue["id"]}).to_string())).await.unwrap();
    push_event(&mock, "Page.screencastFrame", json!({"sessionId":9,"data":"white"})).await;
    let white = next_frame(&mut ws, Duration::from_secs(1)).await.expect("a fresh picture");
    push_event(&mock, "Page.screencastFrame", json!({"sessionId":10,"data":"green"})).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    ws.send(WsMessage::Text(json!({"action":"frame_ack","id":white["id"]}).to_string())).await.unwrap();
    let green = next_frame(&mut ws, Duration::from_secs(1)).await.expect("the picture drawn while the client was busy");
    assert_eq!(green["image"], "green");

    // Chrome had its ack for every picture, so its screencast never stalls either
    let acked = || async { mock.commands.lock().await.iter()
        .filter(|c| c["method"] == "Page.screencastFrameAck").map(|c| c["params"]["sessionId"].clone()).collect::<Vec<_>>() };
    let all = timeout(Duration::from_secs(2), async { loop {
        let seen = acked().await;
        if [7, 8, 9, 10].iter().all(|s| seen.contains(&json!(s))) { break; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }}).await;
    assert!(all.is_ok(), "every session acked: {:?}", acked().await);
    app_task.abort(); cdp_task.abort();
}

/// The next banner about a link to another app, acking frames on the way.
async fn app_link_notice(ws: &mut Client, within: Duration) -> Option<String> {
    timeout(within, async {
        loop {
            if let WsMessage::Text(text) = ws.next().await.unwrap().unwrap() {
                let value: Value = serde_json::from_str(&text).unwrap();
                match value["type"].as_str().unwrap_or_default() {
                    "ping" => ws.send(WsMessage::Text(json!({"action":"pong"}).to_string())).await.unwrap(),
                    "frame" => ws.send(WsMessage::Text(json!({"action":"frame_ack","id":value["id"]}).to_string())).await.unwrap(),
                    "error" if value["notice"] == "app_link" => {
                        assert!(value.get("id").is_none(), "a banner, not the answer to an action");
                        return value["error"].as_str().unwrap().to_string();
                    }
                    _ => {}
                }
            }
        }
    }).await.ok()
}

#[tokio::test]
async fn a_link_to_another_app_is_stopped_or_its_prompt_closed_so_the_tab_keeps_taking_input() {
    let (mock, cdp_task) = mock().await;
    let tmp = tempfile::tempdir().unwrap();
    let (mut ws, _ctrl, app_task) = open_stream(&mock, tmp.path()).await;
    next_frame(&mut ws, Duration::from_secs(5)).await.expect("the stream is up");

    // Chrome answers a link like snssdk1233:// with an "Open app?" prompt that a headless tab
    // never shows and that makes it ignore every click and key, even after a reload. The live
    // view cancels such links in every document, in a world the page can't reach.
    let commands = mock.commands.lock().await.clone();
    let guard = commands.iter().find(|c| c["method"] == "Page.addScriptToEvaluateOnNewDocument" && c["params"]["worldName"] == "kwnt-applinks")
        .expect("the live view guards against links to other apps");
    assert_eq!(guard["params"]["runImmediately"], true, "the page already open is guarded too");
    assert!(guard["params"]["source"].as_str().unwrap().contains("preventDefault"));
    assert!(commands.iter().any(|c| c["method"] == "Runtime.addBinding" && c["params"]["executionContextName"] == "kwnt-applinks"));

    // the guard stopped one: the person learns why their tap did nothing
    push_event(&mock, "Runtime.bindingCalled", json!({"name":"kwntAppLink","payload":"snssdk1233","executionContextId":9})).await;
    let stopped = app_link_notice(&mut ws, Duration::from_secs(2)).await.expect("told the link was stopped");
    assert!(stopped.contains("(snssdk1233:)"), "{stopped}");
    // the page's own script can't put words on the banner
    push_event(&mock, "Runtime.bindingCalled", json!({"name":"kwntAppLink","payload":"<img src=x>","executionContextId":9})).await;

    // Chrome refused one outright (no gesture): no prompt, so nothing to undo
    push_event(&mock, "Page.frameStartedNavigating", json!({"frameId":"F1","url":"snssdk1233://login?a=1","loaderId":"L1","navigationType":"differentDocument"})).await;
    push_event(&mock, "Log.entryAdded", json!({"entry":{"source":"other","level":"error","text":"Not allowed to launch 'snssdk1233://login?a=1' because a user gesture is required."}})).await;
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(!mock.commands.lock().await.iter().any(|c| c["method"] == "Page.navigate"), "the page was left alone");

    // one got past the guard (a frame whose first address is the link): Chrome is asking, so the
    // tab steps off to about:blank and straight back to the sign-in page, which closes the prompt
    push_event(&mock, "Page.frameStartedNavigating", json!({"frameId":"F2","url":"snssdk1233://login?b=1","loaderId":"L2","navigationType":"differentDocument"})).await;
    let recovered = app_link_notice(&mut ws, Duration::from_secs(5)).await.expect("told the page was reloaded");
    assert!(recovered.contains("reloaded"), "{recovered}");
    let steps: Vec<Value> = mock.commands.lock().await.iter()
        .filter(|c| matches!(c["method"].as_str(), Some("Page.navigate" | "Page.navigateToHistoryEntry"))).map(|c| c["params"].clone()).collect();
    assert_eq!(steps, vec![json!({"url":"about:blank"}), json!({"entryId":2})]);
    app_task.abort(); cdp_task.abort();
}

/// Opt in after building docker/browser.Dockerfile. Uses disposable data and a local HTML page.
#[tokio::test]
#[ignore = "requires Docker and the kwnt-browser:v2 image"]
async fn live_chromium_stream_and_isolated_sessions() {
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::WARN).try_init();
    let tmp = tempfile::tempdir().unwrap();
    let _container = Container(kwnt::sandbox::service::managed_container_name(tmp.path()));
    let config = Config { data_dir: tmp.path().into(), ..Config::default() };
    let db = Database::init(tmp.path()).await.unwrap();
    let bots = db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
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
        let ack = receive(&mut ws, "ack").await;
        assert_eq!(ack["id"], index);
        // the first tap lands in the text field, the last on a button
        match index { 0 => assert_eq!(ack["editable"], true), 2 => assert_eq!(ack["editable"], false), _ => {} }
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

/// Removes a test's browser container when the test ends: kwant starts it with
/// `--restart unless-stopped` and never removes it, so each disposable data folder would leave
/// one running.
struct Container(String);

impl Drop for Container {
    fn drop(&mut self) {
        let _ = std::process::Command::new("docker").args(["rm", "-f", &self.0]).output();
    }
}

/// Opt in after building docker/browser.Dockerfile as kwnt-browser:v2, with kwnt-browser:v1 kept:
/// a browser container made from an older image is replaced, so an image change reaches it.
#[tokio::test]
#[ignore = "requires Docker and the kwnt-browser:v1 and :v2 images"]
async fn a_browser_container_from_an_older_image_is_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let name = kwnt::sandbox::service::managed_container_name(tmp.path());
    let _container = Container(name.clone());
    let made = std::process::Command::new("docker")
        .args(["run", "-d", "--name", &name, "--init", "-p", "127.0.0.1::9222", "kwnt-browser:v1"])
        .output()
        .unwrap();
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    let image = || {
        let out = std::process::Command::new("docker").args(["inspect", "--format", "{{.Config.Image}}", &name]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    assert_eq!(image(), "kwnt-browser:v1");

    let service = BrowserService::new(tmp.path().into(), None);
    service.set_mode(kwnt::sandbox::computer::ComputerMode::Vm).await;
    let ctrl = service.session("bot", tmp.path()).await.expect("a browser from the new image");
    assert_eq!(image(), kwnt::sandbox::service::BROWSER_IMAGE);
    let agent = ctrl.send_cdp_command("Runtime.evaluate", json!({"expression": "navigator.userAgent", "returnByValue": true})).await.unwrap();
    assert!(agent.to_string().contains("Chrome/") && !agent.to_string().contains("Headless"), "{agent}");
}

/// F1: turning on Runtime and Log makes Chromium replay everything the page logged. A page
/// that logged 200 lines at load (the TikTok QR page logs ~70) must still start the live view.
#[tokio::test]
async fn a_page_that_logged_hundreds_of_lines_still_starts_the_live_view() {
    let (mock, cdp_task) = mock().await;
    mock.console_lines.store(200, Ordering::SeqCst);
    let tmp = tempfile::tempdir().unwrap();
    let (mut ws, _ctrl, app_task) = open_stream(&mock, tmp.path()).await;
    let first = next_frame(&mut ws, Duration::from_secs(5)).await.expect("the first picture of a noisy page");
    assert_eq!(first["image"], "streamed-frame");
    let commands = mock.commands.lock().await.clone();
    assert!(commands.iter().any(|c| c["method"] == "Runtime.enable") && commands.iter().any(|c| c["method"] == "Log.enable"), "the guard is still on");
    // and the stream is live: the guard's notices and Chrome's refusals still get through
    push_event(&mock, "Runtime.bindingCalled", json!({"name":"kwntAppLink","payload":"snssdk1233","executionContextId":9})).await;
    assert!(app_link_notice(&mut ws, Duration::from_secs(2)).await.expect("told the link was stopped").contains("(snssdk1233:)"));
    app_task.abort(); cdp_task.abort();
}

/// F2: the link fired while no live view was open (the bot opened the sign-in page; the person
/// takes over later). Opening the view finds the tab ignoring input and steps it off and back.
#[tokio::test]
async fn a_tab_frozen_before_the_live_view_opened_is_recovered_when_it_opens() {
    let (mock, cdp_task) = mock().await;
    mock.input.store(2, Ordering::SeqCst);
    let tmp = tempfile::tempdir().unwrap();
    let (mut ws, _ctrl, app_task) = open_stream(&mock, tmp.path()).await;
    let recovered = app_link_notice(&mut ws, Duration::from_secs(5)).await.expect("told the page was reloaded");
    assert!(recovered.contains("reloaded the page"), "{recovered}");
    let commands = mock.commands.lock().await.clone();
    let moves: Vec<&Value> = commands.iter().filter(|c| c["method"] == "Input.dispatchMouseEvent").collect();
    assert!(!moves.is_empty() && moves.iter().all(|c| c["params"]["type"] == "mouseMoved" && c["params"]["buttons"] == 0), "the check only moves the pointer: {moves:?}");
    let steps: Vec<Value> = commands.iter()
        .filter(|c| matches!(c["method"].as_str(), Some("Page.navigate" | "Page.navigateToHistoryEntry"))).map(|c| c["params"].clone()).collect();
    assert_eq!(steps, vec![json!({"url":"about:blank"}), json!({"entryId":2})]);
    app_task.abort(); cdp_task.abort();
}

/// F2: a tab that takes input is left alone when the live view opens: no reload, no banner.
#[tokio::test]
async fn a_live_tab_is_left_alone_when_the_live_view_opens() {
    let (mock, cdp_task) = mock().await;
    mock.input.store(1, Ordering::SeqCst);
    let tmp = tempfile::tempdir().unwrap();
    let (mut ws, _ctrl, app_task) = open_stream(&mock, tmp.path()).await;
    next_frame(&mut ws, Duration::from_secs(5)).await.expect("the stream is up");
    let checked = timeout(Duration::from_secs(3), async { loop {
        if mock.commands.lock().await.iter().filter(|c| c["params"]["expression"].as_str().is_some_and(|e| e.contains("kwnt.inputProbe"))).count() >= 2 { break; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }}).await;
    assert!(checked.is_ok(), "the view checked the tab");
    assert_eq!(app_link_notice(&mut ws, Duration::from_millis(1500)).await, None, "no banner");
    assert!(!mock.commands.lock().await.iter().any(|c| c["method"] == "Page.navigate"), "no reload");
    app_task.abort(); cdp_task.abort();
}

/// F3: a bot's own navigate to a link that opens another app never reaches Chromium, whose
/// "Open app?" prompt would leave the tab ignoring every click and key. The bot is told why.
#[tokio::test]
async fn a_bot_navigating_to_another_app_is_refused_before_the_browser_sees_it() {
    let (mock, cdp_task) = mock().await;
    let tmp = tempfile::tempdir().unwrap();
    let (_ws, ctrl, app_task) = open_stream(&mock, tmp.path()).await;
    for url in ["tiktok://open", "snssdk1233://login", "intent://scan/#Intent;scheme=zxing;end"] {
        let error = ctrl.navigate(url).await.expect_err(url);
        assert!(matches!(error, kwnt::sandbox::BrowserError::NotWebAddress(_)), "{url}: {error:?}");
        let text = error.to_string();
        assert!(text.contains("opens another app") && text.contains("only opens web pages"), "{text}");
    }
    let navigated: Vec<Value> = mock.commands.lock().await.iter().filter(|c| c["method"] == "Page.navigate").map(|c| c["params"]["url"].clone()).collect();
    assert!(navigated.is_empty(), "nothing reached the browser: {navigated:?}");
    // the bot carries on with web pages
    ctrl.navigate("https://www.tiktok.com/login").await.expect("a web page still opens");
    app_task.abort(); cdp_task.abort();
}
