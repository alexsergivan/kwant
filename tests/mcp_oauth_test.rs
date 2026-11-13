//! OAuth sign-in for http MCP servers, end to end against the mock OAuth + MCP server:
//! 401 → start → (browser) → callback → test ok, refresh, revocation and disconnect.

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    mcp::{cli_config, oauth_mock},
    router::LayaRouter,
    server::{create_router, mcp_runtime, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const ORIGIN: &str = "https://kwnt.example.test";
const HOST: &str = "kwnt.example.test";
/// kwnt is addressed by a public name here, so requests need the API token.
const TOKEN: &str = "test-token-0123456789abcdef";

struct Env {
    app: Router,
    state: AppState,
    mock: Arc<oauth_mock::MockState>,
    _tmp: TempDir,
}

async fn env(options: oauth_mock::MockOptions) -> Env {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (mock_app, mock) = oauth_mock::router(&base, options);
    tokio::spawn(async move { axum::serve(listener, mock_app).await.unwrap() });

    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let mut state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    state.api_token = Arc::new(kwnt::server::auth::ApiToken::for_tests(
        TOKEN,
        kwnt::server::auth::Require::Auto,
        config.port,
    ));
    Env {
        app: create_router(state.clone()),
        state,
        mock,
        _tmp: tmp,
    }
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    host: &str,
) -> (StatusCode, String) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", host)
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let (status, text) = send(app, method, uri, body, HOST).await;
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn create_http_server(e: &Env) -> String {
    let (status, created) = call(
        &e.app,
        "POST",
        "/api/mcp/servers",
        Some(json!({"name": "notionish", "transport": "http", "url": format!("{}/mcp", e.mock.base_url)})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["oauth"], json!({"connected": false, "expires_at": null}));
    created["id"].as_str().unwrap().to_string()
}

/// Start → follow the mock's Allow link → callback. Returns the callback page.
async fn sign_in(e: &Env, id: &str) -> String {
    sign_in_at(e, id, ORIGIN, HOST).await
}

/// [`sign_in`] from another address of kwnt (`origin` as the browser sees it, `host` as sent).
async fn sign_in_at(e: &Env, id: &str, origin: &str, host: &str) -> String {
    let authorize_url = start_at(e, id, origin, host).await;
    let resp = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(&authorize_url)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "{}", resp.text().await.unwrap_or_default());
    let redirect = resp.headers()["x-mock-redirect"].to_str().unwrap().to_string();
    let redirect = url::Url::parse(&redirect).unwrap();
    assert_eq!(redirect.origin().ascii_serialization(), origin);
    let path_and_query = format!("{}?{}", redirect.path(), redirect.query().unwrap());
    let (status, page) = send(&e.app, "GET", &path_and_query, None, host).await;
    assert_eq!(status, StatusCode::OK);

    // The state is single use
    let (_, again) = send(&e.app, "GET", &path_and_query, None, host).await;
    assert!(again.contains("\"ok\":false") && again.contains("expired or was already used"), "{again}");
    page
}

/// Starts a sign-in and checks the authorize URL; returns it.
async fn start_at(e: &Env, id: &str, origin: &str, host: &str) -> String {
    let (status, text) =
        send(&e.app, "POST", &format!("/api/mcp/servers/{id}/oauth/start"), Some(json!({"origin": origin})), host).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let started: Value = serde_json::from_str(&text).unwrap();
    let authorize_url = started["authorize_url"].as_str().unwrap().to_string();
    let parsed = url::Url::parse(&authorize_url).unwrap();
    let q: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["redirect_uri"], format!("{origin}/api/mcp/oauth/callback"));
    assert_eq!(q["resource"], format!("{}/mcp", e.mock.base_url));
    assert_eq!(q["scope"], "mcp");
    assert!(!q.contains_key("code_verifier"));
    authorize_url
}

fn client_id_of(authorize_url: &str) -> String {
    url::Url::parse(authorize_url).unwrap().query_pairs().find(|(k, _)| k == "client_id").unwrap().1.into_owned()
}

/// Serves `router` on a loopback port; returns its base URL.
async fn serve_local(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

/// Stands in for kwant.si: serves the committed site/oauth/client.json.
async fn serve_kwant_si() -> String {
    let file = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("site/oauth/client.json")).unwrap();
    serve_local(Router::new().route(
        "/oauth/client.json",
        axum::routing::get(move || async move { ([("content-type", "application/json")], file) }),
    ))
    .await
}

/// A mock that has no dynamic registration but takes client ID metadata documents (ElevenLabs).
async fn document_only(loopback_any_port: bool) -> oauth_mock::MockOptions {
    oauth_mock::MockOptions {
        registration: false,
        client_documents: true,
        loopback_any_port,
        document_hosts: [("https://kwant.si".to_string(), serve_kwant_si().await)].into(),
        ..Default::default()
    }
}

async fn stored_token(e: &Env, id: &str) -> Option<kwnt::db::models::McpOAuthToken> {
    e.state.db.get_mcp_oauth_token(id).await.unwrap()
}

#[tokio::test]
async fn sign_in_flow_end_to_end() {
    let e = env(oauth_mock::MockOptions::default()).await;
    let id = create_http_server(&e).await;

    // Before sign-in: the 401 becomes auth_required
    let (_, tested) = call(&e.app, "POST", &format!("/api/mcp/servers/{id}/test"), None).await;
    assert_eq!(tested["ok"], false);
    assert_eq!(tested["auth_required"], true);
    assert_eq!(tested["error"], "Sign-in required");

    // Origin must match the request host
    for bad in ["https://evil.test", "javascript:alert(1)", "https://kwnt.example.test/path"] {
        let (status, body) =
            call(&e.app, "POST", &format!("/api/mcp/servers/{id}/oauth/start"), Some(json!({"origin": bad}))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }
    let (status, _) = call(&e.app, "POST", "/api/mcp/servers/nope/oauth/start", Some(json!({"origin": ORIGIN}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let page = sign_in(&e, &id).await;
    assert!(page.contains(&format!("\"server_id\":\"{id}\"")), "{page}");
    assert!(page.contains("\"ok\":true"), "{page}");
    assert!(page.contains("kwnt-mcp-oauth") && page.contains("location.origin"));
    let token = stored_token(&e, &id).await.expect("token stored");
    assert!(token.refresh_token.is_some());
    assert_eq!(e.mock.registrations.load(Ordering::SeqCst), 1);
    assert_eq!(e.mock.code_exchanges.load(Ordering::SeqCst), 1);

    // Test now connects with the token
    let (_, tested) = call(&e.app, "POST", &format!("/api/mcp/servers/{id}/test"), None).await;
    assert_eq!(tested["ok"], true, "{tested}");
    assert_eq!(tested["tools"][0]["name"], "whoami");

    // List and get show the status, never the tokens
    let (_, list) = call(&e.app, "GET", "/api/mcp/servers", None).await;
    assert_eq!(list[0]["oauth"]["connected"], true);
    assert!(list[0]["oauth"]["expires_at"].is_string());
    let (_, one) = call(&e.app, "GET", &format!("/api/mcp/servers/{id}"), None).await;
    assert_eq!(one["oauth"]["connected"], true);
    for body in [list.to_string(), one.to_string()] {
        assert!(!body.contains(&token.access_token) && !body.contains(token.refresh_token.as_deref().unwrap()));
    }
    assert!(!format!("{token:?}").contains(&token.access_token));

    // Turns get the Bearer header; the claude CLI config carries it in its private file
    let kwant = e.state.db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap().into_iter().next().unwrap();
    e.state.db.set_kwant_mcp_servers(&kwant.id, std::slice::from_ref(&id)).await.unwrap();
    let (ctx, turn) = mcp_runtime::prepare_turn(&e.state, &kwant, None).await.unwrap();
    let bearer = format!("Bearer {}", token.access_token);
    assert_eq!(ctx.servers[0].headers["Authorization"], bearer);
    assert!(ctx.secret_values().contains(&bearer), "token is redacted from tool output");
    let config = cli_config::claude_config(&ctx);
    assert_eq!(config["mcpServers"]["notionish"]["headers"]["Authorization"], bearer);
    mcp_runtime::finish_turn(&e.state, &turn).await;

    // A second sign-in reuses the client registration
    sign_in(&e, &id).await;
    assert_eq!(e.mock.registrations.load(Ordering::SeqCst), 1);

    // Disconnect drops the tokens
    let (status, body) = call(&e.app, "DELETE", &format!("/api/mcp/servers/{id}/oauth"), None).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"status": "disconnected"})));
    assert!(stored_token(&e, &id).await.is_none());
    let (_, tested) = call(&e.app, "POST", &format!("/api/mcp/servers/{id}/test"), None).await;
    assert_eq!(tested["auth_required"], true);

    // Deleting the server deletes its tokens and registrations
    sign_in(&e, &id).await;
    assert!(stored_token(&e, &id).await.is_some());
    call(&e.app, "DELETE", &format!("/api/mcp/servers/{id}"), None).await;
    assert!(stored_token(&e, &id).await.is_none());
    assert!(e.state.db.get_mcp_oauth_client(&id, &format!("{ORIGIN}/api/mcp/oauth/callback"), &e.mock.base_url).await.unwrap().is_none());
}

#[tokio::test]
async fn short_lived_tokens_are_refreshed_before_use() {
    let e = env(oauth_mock::MockOptions { expires_in: 30, ..Default::default() }).await;
    let id = create_http_server(&e).await;
    sign_in(&e, &id).await;
    let first = stored_token(&e, &id).await.unwrap();

    let (_, tested) = call(&e.app, "POST", &format!("/api/mcp/servers/{id}/test"), None).await;
    assert_eq!(tested["ok"], true, "{tested}");
    assert_eq!(e.mock.refreshes.load(Ordering::SeqCst), 1);
    let second = stored_token(&e, &id).await.unwrap();
    assert_ne!(first.access_token, second.access_token);
    assert_ne!(first.refresh_token, second.refresh_token, "rotated refresh token is stored");

    // Concurrent turns refresh once each in turn, never with a spent refresh token
    let kwant = e.state.db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap().into_iter().next().unwrap();
    e.state.db.set_kwant_mcp_servers(&kwant.id, std::slice::from_ref(&id)).await.unwrap();
    let (a, b) = tokio::join!(mcp_runtime::prepare_turn(&e.state, &kwant, None), mcp_runtime::prepare_turn(&e.state, &kwant, None));
    for (ctx, _) in [a.unwrap(), b.unwrap()] {
        assert!(ctx.servers[0].headers["Authorization"].starts_with("Bearer at-"));
    }
    assert!(stored_token(&e, &id).await.is_some(), "no refresh was rejected");

    // Refresh rejected (the user revoked kwnt): token cleared, sign-in needed
    e.mock.revoke_all();
    let (_, tested) = call(&e.app, "POST", &format!("/api/mcp/servers/{id}/test"), None).await;
    assert_eq!(tested["auth_required"], true, "{tested}");
    assert!(stored_token(&e, &id).await.is_none());
    let (_, one) = call(&e.app, "GET", &format!("/api/mcp/servers/{id}"), None).await;
    assert_eq!(one["oauth"]["connected"], false);
}

#[tokio::test]
async fn rejected_token_is_refreshed_once_then_cleared() {
    let e = env(oauth_mock::MockOptions::default()).await;
    let id = create_http_server(&e).await;
    sign_in(&e, &id).await;

    // Access token revoked early, refresh token still good: one refresh, then ok
    e.mock.revoke_access_tokens();
    let (_, tested) = call(&e.app, "POST", &format!("/api/mcp/servers/{id}/test"), None).await;
    assert_eq!(tested["ok"], true, "{tested}");
    assert_eq!(e.mock.refreshes.load(Ordering::SeqCst), 1);

    e.mock.revoke_all();
    let (_, tested) = call(&e.app, "POST", &format!("/api/mcp/servers/{id}/test"), None).await;
    assert_eq!(tested["auth_required"], true);
    assert!(stored_token(&e, &id).await.is_none());
}

#[tokio::test]
async fn user_authorization_header_wins_over_sign_in() {
    let e = env(oauth_mock::MockOptions::default()).await;
    let id = create_http_server(&e).await;
    sign_in(&e, &id).await;
    let token = stored_token(&e, &id).await.unwrap();

    let url = format!("{}/mcp", e.mock.base_url);
    let (status, _) = call(
        &e.app,
        "PUT",
        &format!("/api/mcp/servers/{id}"),
        Some(json!({"name": "notionish", "transport": "http", "url": url, "headers": {"Authorization": "Bearer user-own-key"}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, tested) = call(&e.app, "POST", &format!("/api/mcp/servers/{id}/test"), None).await;
    // The user's (wrong) key was used, so this is a plain error, not a sign-in prompt
    assert_eq!(tested["ok"], false);
    assert!(tested.get("auth_required").is_none(), "{tested}");
    assert!(tested["error"].as_str().unwrap().contains("401"));
    assert!(!tested.to_string().contains("user-own-key") && !tested.to_string().contains(&token.access_token));
    assert!(stored_token(&e, &id).await.is_some(), "kwnt's sign-in is kept");

    // Changing the url forgets the sign-in
    let (status, updated) = call(
        &e.app,
        "PUT",
        &format!("/api/mcp/servers/{id}"),
        Some(json!({"name": "notionish", "transport": "http", "url": format!("{url}/v2")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["oauth"]["connected"], false);
    assert!(stored_token(&e, &id).await.is_none());
}

#[tokio::test]
async fn callback_reports_provider_errors_and_bad_state() {
    let e = env(oauth_mock::MockOptions::default()).await;
    let id = create_http_server(&e).await;
    let (_, page) = send(&e.app, "GET", "/api/mcp/oauth/callback?code=x&state=forged", None, HOST).await;
    assert!(page.contains("\"ok\":false") && page.contains("\"server_id\":null"), "{page}");

    let (_, started) =
        call(&e.app, "POST", &format!("/api/mcp/servers/{id}/oauth/start"), Some(json!({"origin": ORIGIN}))).await;
    let authorize = url::Url::parse(started["authorize_url"].as_str().unwrap()).unwrap();
    let state = authorize.query_pairs().find(|(k, _)| k == "state").unwrap().1.into_owned();
    let (_, page) = send(
        &e.app,
        "GET",
        &format!("/api/mcp/oauth/callback?error=access_denied&error_description=%3Cscript%3Ex&state={state}"),
        None,
        HOST,
    )
    .await;
    assert!(page.contains("\"ok\":false") && page.contains("access_denied"), "{page}");
    assert!(!page.contains("<script>x"), "provider text is escaped: {page}");
    assert!(stored_token(&e, &id).await.is_none());

    // Behind a proxy the public host comes in X-Forwarded-Host
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/mcp/servers/{id}/oauth/start"))
        .header("host", "127.0.0.1:7777")
        .header("x-forwarded-host", HOST)
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"origin": ORIGIN}).to_string()))
        .unwrap();
    assert_eq!(e.app.clone().oneshot(req).await.unwrap().status(), StatusCode::OK);
}

/// The Mac app (kwant on 127.0.0.1:8799) signs in to a server without dynamic registration with
/// the document on kwant.si, which the server fetches and checks the redirect against.
#[tokio::test]
async fn client_document_sign_in_on_this_computer() {
    let e = env(document_only(false).await).await;
    let id = create_http_server(&e).await;
    let (origin, host) = ("http://127.0.0.1:8799", "127.0.0.1:8799");

    let page = sign_in_at(&e, &id, origin, host).await;
    assert!(page.contains("\"ok\":true"), "{page}");
    assert_eq!(e.mock.registrations.load(Ordering::SeqCst), 0);
    assert_eq!(e.mock.document_fetches.load(Ordering::SeqCst), 1);
    assert_eq!(e.mock.code_exchanges.load(Ordering::SeqCst), 1);
    let token = stored_token(&e, &id).await.expect("token stored");
    assert_eq!(token.client_id, "https://kwant.si/oauth/client.json");
    assert_eq!(token.auth_method, "none");
    assert_eq!(token.client_secret, None);
    let client = e
        .state
        .db
        .get_mcp_oauth_client(&id, &format!("{origin}/api/mcp/oauth/callback"), &e.mock.base_url)
        .await
        .unwrap()
        .expect("the document client is stored like a registration");
    assert_eq!(client.client_id, "https://kwant.si/oauth/client.json");

    // The token works, and a second sign-in reuses the stored client
    let (_, tested) = call(&e.app, "POST", &format!("/api/mcp/servers/{id}/test"), None).await;
    assert_eq!(tested["ok"], true, "{tested}");
    sign_in_at(&e, &id, origin, host).await;
    assert_eq!(e.mock.registrations.load(Ordering::SeqCst), 0);

    // Refresh sends the document client_id too
    assert!(kwnt::server::mcp_oauth::retry_after_unauthorized(&e.state, &e.state.db.get_mcp_server(&id).await.unwrap().unwrap())
        .await
        .is_some());
    assert_eq!(e.mock.refreshes.load(Ordering::SeqCst), 1);
}

/// kwant on another loopback port: fine where the server accepts any loopback port (RFC 8252),
/// refused by a server that matches redirects exactly (the document lists port 8799 only).
#[tokio::test]
async fn client_document_loopback_ports() {
    let (origin, host) = ("http://127.0.0.1:51234", "127.0.0.1:51234");
    let e = env(document_only(true).await).await;
    let id = create_http_server(&e).await;
    assert!(sign_in_at(&e, &id, origin, host).await.contains("\"ok\":true"));

    let e = env(document_only(false).await).await;
    let id = create_http_server(&e).await;
    let authorize_url = start_at(&e, &id, origin, host).await;
    assert_eq!(client_id_of(&authorize_url), "https://kwant.si/oauth/client.json");
    let resp = reqwest::get(&authorize_url).await.unwrap();
    assert_eq!(resp.status(), 400);
    assert_eq!(resp.text().await.unwrap(), "unknown client or redirect_uri");
}

/// A self-hosted kwant on a public https name serves its own document, without a login, and
/// the server fetches it from there.
#[tokio::test]
async fn client_document_sign_in_self_hosted() {
    // The mock reaches "https://kwnt.example.test" at a real listener running the same app
    let mut e = env(oauth_mock::MockOptions::default()).await;
    let kwnt_base = serve_local(e.app.clone()).await;
    let options = oauth_mock::MockOptions {
        registration: false,
        client_documents: true,
        document_hosts: [(ORIGIN.to_string(), kwnt_base)].into(),
        ..Default::default()
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (mock_app, mock) = oauth_mock::router(&base, options);
    tokio::spawn(async move { axum::serve(listener, mock_app).await.unwrap() });
    e.mock = mock;

    let id = create_http_server(&e).await;
    let authorize_url = start_at(&e, &id, ORIGIN, HOST).await;
    assert_eq!(client_id_of(&authorize_url), format!("{ORIGIN}/api/mcp/oauth/client.json"));
    assert!(sign_in(&e, &id).await.contains("\"ok\":true"));
    assert_eq!(e.mock.document_fetches.load(Ordering::SeqCst), 2);
    assert_eq!(e.mock.registrations.load(Ordering::SeqCst), 0);
    assert_eq!(stored_token(&e, &id).await.unwrap().client_id, format!("{ORIGIN}/api/mcp/oauth/client.json"));
}

#[tokio::test]
async fn client_document_endpoint() {
    let e = env(oauth_mock::MockOptions::default()).await;
    let get = |host: &'static str, forwarded: Option<&'static str>| {
        let app = e.app.clone();
        async move {
            // No API token: authorization servers fetch it anonymously
            let mut req = Request::builder().uri("/api/mcp/oauth/client.json").header("host", host);
            if let Some(f) = forwarded {
                req = req.header("x-forwarded-host", f);
            }
            let resp = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
            let status = resp.status();
            let ctype = resp.headers().get("content-type").map(|v| v.to_str().unwrap().to_string());
            let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            (status, ctype, serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null))
        }
    };
    let (status, ctype, doc) = get(HOST, None).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(ctype.as_deref(), Some("application/json"));
    assert_eq!(doc["client_id"], format!("{ORIGIN}/api/mcp/oauth/client.json"));
    assert_eq!(doc["redirect_uris"], json!([format!("{ORIGIN}/api/mcp/oauth/callback")]));

    // Behind a proxy the public name wins
    let (_, _, doc) = get("127.0.0.1:8799", Some("kwant.example.com, inner.proxy")).await;
    assert_eq!(doc["client_id"], "https://kwant.example.com/api/mcp/oauth/client.json");

    // Loopback uses kwant.si's document; odd hosts get nothing
    for (host, fwd) in [("127.0.0.1:8799", None), ("localhost:8799", None), (HOST, Some("evil.test/x")), (HOST, Some("u@evil.test"))] {
        let (status, _, body) = get(host, fwd).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{host} {fwd:?}: {body}");
    }
}

/// Servers that offer both keep using dynamic registration; if it is refused, kwant falls back
/// to its document.
#[tokio::test]
async fn registration_first_then_client_document() {
    let mut options = document_only(false).await;
    options.registration = true;
    let e = env(options.clone()).await;
    let id = create_http_server(&e).await;
    sign_in_at(&e, &id, "http://127.0.0.1:8799", "127.0.0.1:8799").await;
    assert_eq!(e.mock.registrations.load(Ordering::SeqCst), 1);
    assert_eq!(e.mock.document_fetches.load(Ordering::SeqCst), 0);
    assert_ne!(stored_token(&e, &id).await.unwrap().client_id, "https://kwant.si/oauth/client.json");

    options.refuse_registration = true;
    let e = env(options).await;
    let id = create_http_server(&e).await;
    sign_in_at(&e, &id, "http://127.0.0.1:8799", "127.0.0.1:8799").await;
    assert_eq!(e.mock.document_fetches.load(Ordering::SeqCst), 1);
    assert_eq!(stored_token(&e, &id).await.unwrap().client_id, "https://kwant.si/oauth/client.json");
}

/// Dead ends say so with a code (the card shows what to do, not Retry) and store nothing.
#[tokio::test]
async fn no_way_to_register_is_a_clear_error() {
    let e = env(oauth_mock::MockOptions { registration: false, ..Default::default() }).await;
    let id = create_http_server(&e).await;
    let (status, body) = call(&e.app, "POST", &format!("/api/mcp/servers/{id}/oauth/start"), Some(json!({"origin": ORIGIN}))).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["code"], "no_client_registration", "{body}");
    assert!(body["error"].as_str().unwrap().contains("server's own settings"), "{body}");

    // Documents need https or loopback: kwant on a LAN address can't use them
    let e = env(document_only(false).await).await;
    let id = create_http_server(&e).await;
    let (status, text) = send(
        &e.app,
        "POST",
        &format!("/api/mcp/servers/{id}/oauth/start"),
        Some(json!({"origin": "http://192.168.1.5:8799"})),
        "192.168.1.5:8799",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let body: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(body["code"], "cimd_needs_https", "{body}");
    assert!(e
        .state
        .db
        .get_mcp_oauth_client(&id, "http://192.168.1.5:8799/api/mcp/oauth/callback", &e.mock.base_url)
        .await
        .unwrap()
        .is_none());

    // Other failures carry no code (Retry may help)
    let e = env(oauth_mock::MockOptions { refuse_registration: true, ..Default::default() }).await;
    let id = create_http_server(&e).await;
    let (status, body) = call(&e.app, "POST", &format!("/api/mcp/servers/{id}/oauth/start"), Some(json!({"origin": ORIGIN}))).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(body.get("code").is_none(), "{body}");
}
