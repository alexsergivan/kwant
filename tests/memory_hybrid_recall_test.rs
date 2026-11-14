//! Hybrid memory recall (roadmap Next #11): the bots' `memory_search` finds notes by meaning
//! through the space's own embedding provider, merged with the FTS5 keyword search; with no
//! provider it is exactly the keyword search it was, with no HTTP call.
//!
//! The provider here is a fake Ollama (and a fake OpenAI endpoint) whose "embeddings" put words
//! of one concept (car/vehicle, refund/money-back, …) on one axis, so paraphrases land close.

use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{HeaderMap, Request},
    routing::{get, post},
    Json, Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::Kwant, spaces::SpaceId, Database},
    drivers::engines::{EngineId, ProbeEnv},
    router::LayaRouter,
    server::{
        continuity::SelfScope,
        create_router, mcp_runtime,
        memory_recall::{self, EmbedSource},
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const DIM: usize = 16;

/// A concept-axis "embedding": words of one concept share an axis; any other word lands, lightly,
/// on one of the remaining axes.
fn fake_embedding(text: &str) -> Vec<f32> {
    const CONCEPTS: [&[&str]; 4] = [
        &["car", "cars", "vehicle", "vehicles", "auto", "automobile"],
        &["refund", "refunds", "money", "back", "reimburse", "returned"],
        &["rules", "rule", "policy", "policies"],
        &["customer", "customers", "client", "clients"],
    ];
    let mut v = vec![0f32; DIM];
    for word in text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        match CONCEPTS.iter().position(|c| c.contains(&word)) {
            Some(axis) => v[axis] += 1.0,
            None => v[4 + word.bytes().map(usize::from).sum::<usize>() % (DIM - 4)] += 0.3,
        }
    }
    v
}

#[derive(Clone, Default)]
struct Fake {
    /// Every request the server got, of any kind.
    calls: Arc<AtomicUsize>,
    /// Embedding requests only, and how many texts they carried.
    embed_calls: Arc<AtomicUsize>,
    embedded_texts: Arc<AtomicUsize>,
    /// Milliseconds to wait before answering a one-text (query) embedding.
    query_delay_ms: Arc<AtomicU64>,
    /// Ollama's pulled models.
    models: Arc<std::sync::Mutex<Vec<String>>>,
}

async fn tags(State(f): State<Fake>) -> Json<Value> {
    f.calls.fetch_add(1, Ordering::SeqCst);
    let models = f.models.lock().unwrap().clone();
    Json(json!({"models": models.iter().map(|m| json!({"name": m})).collect::<Vec<_>>()}))
}

async fn version(State(f): State<Fake>) -> Json<Value> {
    f.calls.fetch_add(1, Ordering::SeqCst);
    Json(json!({"version": "0.0.0-fake"}))
}

async fn inputs(f: &Fake, body: &Value) -> Vec<Vec<f32>> {
    f.calls.fetch_add(1, Ordering::SeqCst);
    f.embed_calls.fetch_add(1, Ordering::SeqCst);
    let texts: Vec<String> = body["input"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect();
    f.embedded_texts.fetch_add(texts.len(), Ordering::SeqCst);
    let delay = f.query_delay_ms.load(Ordering::SeqCst);
    if texts.len() == 1 && delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    texts.iter().map(|t| fake_embedding(t)).collect()
}

async fn ollama_embed(State(f): State<Fake>, Json(body): Json<Value>) -> Json<Value> {
    Json(json!({"model": body["model"], "embeddings": inputs(&f, &body).await}))
}

async fn openai_embed(State(f): State<Fake>, headers: HeaderMap, Json(body): Json<Value>) -> axum::response::Response {
    use axum::response::IntoResponse;
    if headers.get("authorization").and_then(|h| h.to_str().ok()) != Some("Bearer sk-test") {
        f.calls.fetch_add(1, Ordering::SeqCst);
        return (axum::http::StatusCode::UNAUTHORIZED, Json(json!({"error": "bad key"}))).into_response();
    }
    assert_eq!(body["model"], "text-embedding-3-small");
    let vectors = inputs(&f, &body).await;
    // Answer out of order: the client must sort by index
    let data: Vec<Value> = vectors.iter().enumerate().rev().map(|(i, v)| json!({"index": i, "embedding": v})).collect();
    Json(json!({"data": data})).into_response()
}

async fn fake_server(models: &[&str]) -> (String, Fake) {
    let fake = Fake::default();
    *fake.models.lock().unwrap() = models.iter().map(|m| m.to_string()).collect();
    let app = Router::new()
        .route("/api/tags", get(tags))
        .route("/api/version", get(version))
        .route("/api/embed", post(ollama_embed))
        .route("/v1/embeddings", post(openai_embed))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, fake)
}

async fn setup() -> (Router, AppState, TempDir) {
    // The host's own OpenAI key would be a provider; these tests choose theirs
    std::env::remove_var("OPENAI_API_KEY");
    std::env::remove_var("OLLAMA_URL");
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    db.set_setting("mission_board", "false").await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    (create_router(state.clone()), state, tmp)
}

/// Points the space at the fake Ollama and runs its engine check there, as the app does.
async fn use_ollama(state: &AppState, url: &str) {
    state.db.set_setting("ollama_url", url).await.unwrap();
    let env = ProbeEnv { ollama_root: url.to_string(), ..ProbeEnv::default() };
    state.engines.recheck_one(EngineId::Ollama, &env).await;
}

async fn first_kwant(state: &AppState) -> Kwant {
    state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0)
}

/// Calls the kwant's own `memory_search` tool the way a turn does.
async fn memory_search(app: &Router, state: &AppState, k: &Kwant, query: &str) -> String {
    let (_ctx, token) = mcp_runtime::prepare_turn_with_self(
        state,
        k,
        None,
        None,
        Some(SelfScope { kwant_id: k.id.clone(), kwant_name: k.name.clone(), ..Default::default() }),
    )
    .await
    .unwrap();
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "memory_search", "arguments": {"query": query}}});
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mcp/self")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(v["result"]["isError"], false, "{v}");
    v["result"]["content"][0]["text"].as_str().unwrap().to_string()
}

const VEHICLE: &str = "Oleks parks the vehicle in the garage on Elm street";
const MONEY_BACK: &str = "Money-back policy: clients get their money returned within 30 days";

async fn seed(state: &AppState, k: &Kwant) -> (String, String) {
    let db = &state.db;
    let v = db.add_memory(&k.id, VEHICLE, "user").await.unwrap().id;
    let m = db.add_memory(&k.id, MONEY_BACK, "agent").await.unwrap().id;
    for text in ["Daughter's birthday is 3 March", "Prefers short answers in the morning", "The office wifi password lives in the vault"] {
        db.add_memory(&k.id, text, "agent").await.unwrap();
    }
    (v, m)
}

#[tokio::test]
async fn paraphrases_are_found_with_embeddings_and_missed_without() {
    let (app, state, _tmp) = setup().await;
    let k = first_kwant(&state).await;
    let (vehicle, money_back) = seed(&state, &k).await;

    // Embeddings off: keyword search alone misses both paraphrases
    assert_eq!(memory_search(&app, &state, &k, "car").await, "No matching notes.");
    assert_eq!(memory_search(&app, &state, &k, "customer refund rules").await, "No matching notes.");

    let (url, fake) = fake_server(&["llama3.2:latest", "nomic-embed-text:latest"]).await;
    use_ollama(&state, &url).await;
    assert_eq!(
        memory_recall::source_for_kwant(&state, &k.id).await,
        Some(EmbedSource::Ollama { root: url.clone(), model: "nomic-embed-text:latest".into() })
    );
    assert_eq!(memory_recall::fill_now(&state, &k.id, memory_recall::FILL_ROUNDS).await, 5);
    assert_eq!(fake.embed_calls.load(Ordering::SeqCst), 1, "five notes in one batch");
    assert_eq!(memory_recall::fill_now(&state, &k.id, memory_recall::FILL_ROUNDS).await, 0, "nothing left to fill");

    // Embeddings on: found by meaning, through the bots' own tool
    let found = memory_search(&app, &state, &k, "car").await;
    assert!(found.contains(&vehicle) && found.contains(VEHICLE), "{found}");
    assert!(!found.contains(&money_back), "unrelated notes stay out: {found}");
    let found = memory_search(&app, &state, &k, "customer refund rules").await;
    assert!(found.contains(&money_back) && found.contains("Money-back policy"), "{found}");
    assert!(!found.contains(&vehicle), "{found}");
    // Keywords still work, and come first when both agree
    let found = memory_search(&app, &state, &k, "birthday").await;
    assert!(found.contains("Daughter's birthday"), "{found}");
    let found = memory_search(&app, &state, &k, "vehicle garage").await;
    assert!(found.find(VEHICLE).is_some(), "{found}");
    // One query embedding per search, nothing more
    let before = fake.embed_calls.load(Ordering::SeqCst);
    memory_search(&app, &state, &k, "car").await;
    assert_eq!(fake.embed_calls.load(Ordering::SeqCst), before + 1);
}

#[tokio::test]
async fn without_a_provider_search_is_todays_keyword_search_with_no_http_call() {
    let (app, state, _tmp) = setup().await;
    let k = first_kwant(&state).await;
    seed(&state, &k).await;
    for i in 0..12 {
        state.db.add_memory(&k.id, &format!("garage note {i} about the garage door and the garage key"), "agent").await.unwrap();
    }
    // An Ollama server is configured but never checked ready, and there is no OpenAI key
    let (url, fake) = fake_server(&["nomic-embed-text"]).await;
    state.db.set_setting("ollama_url", &url).await.unwrap();
    assert_eq!(memory_recall::source_for_kwant(&state, &k.id).await, None);

    for q in ["garage", "garage door", "the", "car", "birthday March"] {
        let today = state.db.search_memories(&k.id, q, 10).await.unwrap();
        let now = memory_recall::search(&state, &k.id, q, 10).await.unwrap();
        assert_eq!(now, today, "same notes in the same order for {q:?}");
    }
    assert_eq!(memory_recall::fill_now(&state, &k.id, memory_recall::FILL_ROUNDS).await, 0);
    let text = memory_search(&app, &state, &k, "garage").await;
    assert!(text.contains("garage note"), "{text}");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0, "no HTTP call of any kind");

    // A ready Ollama with chat models only is no embedding provider either
    let (url, fake) = fake_server(&["llama3.2", "qwen3"]).await;
    use_ollama(&state, &url).await;
    let probed = fake.calls.load(Ordering::SeqCst);
    assert_eq!(memory_recall::source_for_kwant(&state, &k.id).await, None);
    assert_eq!(memory_recall::search(&state, &k.id, "garage", 10).await.unwrap(), state.db.search_memories(&k.id, "garage", 10).await.unwrap());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fake.calls.load(Ordering::SeqCst), probed, "only the engine check itself");
}

#[tokio::test]
async fn stale_vectors_are_refilled_and_deleted_with_their_memory() {
    let (_app, state, tmp) = setup().await;
    let k = first_kwant(&state).await;
    let (vehicle, money_back) = seed(&state, &k).await;
    let (url, fake) = fake_server(&["mxbai-embed-large"]).await;
    use_ollama(&state, &url).await;
    assert_eq!(memory_recall::fill_now(&state, &k.id, 4).await, 5);

    // The person edits a note: its old vector no longer counts, a search starts a refill
    state.db.update_memory(&k.id, &money_back, "Clients may get a refund within 30 days").await.unwrap();
    let hits = memory_recall::search(&state, &k.id, "reimburse", 10).await.unwrap();
    assert!(hits.iter().all(|m| m.id != money_back), "the stale vector isn't used");
    let mut waited = 0;
    while fake.embedded_texts.load(Ordering::SeqCst) < 7 && waited < 50 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        waited += 1;
    }
    // 5 notes + 1 query + the 1 edited note, filled in the background
    assert_eq!(fake.embedded_texts.load(Ordering::SeqCst), 7);
    let hits = memory_recall::search(&state, &k.id, "reimburse", 10).await.unwrap();
    assert_eq!(hits.first().map(|m| m.id.as_str()), Some(money_back.as_str()));

    // A vector made by another model is stale too
    assert!(state.db.put_memory_vector(&money_back, "ollama/other-model", &[1.0; 16], &memory_recall::text_hash("Clients may get a refund within 30 days")).await.unwrap());
    let row = state.db.memory_vector_rows(&k.id).await.unwrap().into_iter().find(|r| r.id == money_back).unwrap();
    assert!(row.fresh_vector("ollama/other-model", &memory_recall::text_hash(&row.text)).is_some());
    assert!(row.fresh_vector("ollama/mxbai-embed-large", &memory_recall::text_hash(&row.text)).is_none());
    assert_eq!(memory_recall::fill_now(&state, &k.id, 4).await, 1, "refilled with the space's model");
    let count = |sql: &'static str| {
        let path = tmp.path().join("kwnt.db");
        async move {
            let pool = sqlx::sqlite::SqlitePoolOptions::new().connect(&format!("sqlite://{}", path.display())).await.unwrap();
            let (n,): (i64,) = sqlx::query_as(sql).fetch_one(&pool).await.unwrap();
            pool.close().await;
            n
        }
    };
    assert_eq!(count("SELECT COUNT(*) FROM kwant_memory_vectors").await, 5);

    // Deleting a memory deletes its vector
    assert!(state.db.delete_memory(&k.id, &vehicle).await.unwrap());
    assert_eq!(count("SELECT COUNT(*) FROM kwant_memory_vectors").await, 4);
    assert!(!state.db.put_memory_vector(&vehicle, "ollama/x", &[1.0], "h").await.unwrap(), "no vector for a deleted note");
    assert_eq!(count("SELECT COUNT(*) FROM kwant_memory_vectors WHERE dim <> 16 OR model <> 'ollama/mxbai-embed-large'").await, 0);
}

#[tokio::test]
async fn a_slow_provider_falls_back_to_keywords_and_cools_down() {
    let (_app, state, _tmp) = setup().await;
    let k = first_kwant(&state).await;
    seed(&state, &k).await;
    let (url, fake) = fake_server(&["nomic-embed-text"]).await;
    use_ollama(&state, &url).await;
    memory_recall::fill_now(&state, &k.id, 4).await;
    fake.query_delay_ms.store(3000, Ordering::SeqCst);

    let started = Instant::now();
    let hits = memory_recall::search(&state, &k.id, "car", 10).await.unwrap();
    let took = started.elapsed();
    assert!(hits.is_empty(), "keyword-only result");
    assert!(took < Duration::from_millis(2500), "gave up at the query timeout: {took:?}");
    // In the cooldown no call is made at all
    let calls = fake.embed_calls.load(Ordering::SeqCst);
    let started = Instant::now();
    assert!(memory_recall::search(&state, &k.id, "car", 10).await.unwrap().is_empty());
    assert!(started.elapsed() < Duration::from_millis(300));
    assert_eq!(fake.embed_calls.load(Ordering::SeqCst), calls);
}

#[tokio::test]
async fn an_openai_account_embeds_with_its_key() {
    let (url, fake) = fake_server(&[]).await;
    let source = EmbedSource::OpenAi { base: url.clone(), key: "sk-test".into() };
    let texts = vec!["car".to_string(), MONEY_BACK.to_string()];
    let vectors = memory_recall::embed(&source, &texts, Duration::from_secs(2)).await.unwrap();
    assert_eq!(vectors, vec![fake_embedding("car"), fake_embedding(MONEY_BACK)], "sorted back by index");
    let wrong = EmbedSource::OpenAi { base: url, key: "sk-wrong".into() };
    let err = memory_recall::embed(&wrong, &texts, Duration::from_secs(2)).await.unwrap_err();
    assert_eq!(err, "HTTP 401");
    assert_eq!(fake.embed_calls.load(Ordering::SeqCst), 1);

    // The space's own OpenAI key (Settings) is chosen before a local Ollama
    let (_app, state, _tmp) = setup().await;
    let k = first_kwant(&state).await;
    let (ollama, _) = fake_server(&["nomic-embed-text"]).await;
    use_ollama(&state, &ollama).await;
    state.db.set_setting("openai_api_key", "sk-space").await.unwrap();
    assert_eq!(
        memory_recall::source_for_kwant(&state, &k.id).await,
        Some(EmbedSource::OpenAi { base: "https://api.openai.com".into(), key: "sk-space".into() })
    );
    // …unless the OpenAI API engine is turned off
    state.engines.set_enabled(EngineId::OpenaiApi, false);
    assert!(matches!(memory_recall::source_for_kwant(&state, &k.id).await, Some(EmbedSource::Ollama { .. })));
}

/// The latency of one search over ~500 notes, with and without embeddings (printed; run with
/// `--nocapture`). The cap of 100 notes per bot is bypassed with direct inserts.
#[tokio::test]
async fn search_latency_with_500_memories() {
    let (_app, state, tmp) = setup().await;
    let k = first_kwant(&state).await;
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect(&format!("sqlite://{}", tmp.path().join("kwnt.db").display()))
        .await
        .unwrap();
    let words = ["garage", "vehicle", "refund", "policy", "birthday", "meeting", "invoice", "travel", "school", "doctor"];
    for i in 0..500 {
        let text = format!("note {i}: {} {} and {} details number {}", words[i % 10], words[(i / 10) % 10], words[(i / 7) % 10], i * 37);
        sqlx::query("INSERT INTO kwant_memories (id, kwant_id, text, source, created_at, updated_at) VALUES (?, ?, ?, 'agent', ?, ?)")
            .bind(format!("bench-{i}"))
            .bind(&k.id)
            .bind(text)
            .bind(chrono::Utc::now())
            .bind(chrono::Utc::now())
            .execute(&pool)
            .await
            .unwrap();
    }
    pool.close().await;

    let time = |label: &'static str, hybrid: bool| {
        let (state, k) = (state.clone(), k.id.clone());
        async move {
            let runs = 20;
            let started = Instant::now();
            let mut n = 0;
            for q in ["car", "refund rules", "garage", "doctor visit", "travel invoice"].iter().cycle().take(runs) {
                n += if hybrid {
                    memory_recall::search(&state, &k, q, 10).await.unwrap().len()
                } else {
                    state.db.search_memories(&k, q, 10).await.unwrap().len()
                };
            }
            let per = started.elapsed() / runs as u32;
            println!("{label}: {per:?} per search ({n} hits over {runs} searches)");
            per
        }
    };
    let keywords = time("keyword only (today, no provider)", false).await;
    let none = time("memory_recall::search with no provider", true).await;

    let (url, _fake) = fake_server(&["nomic-embed-text"]).await;
    use_ollama(&state, &url).await;
    let started = Instant::now();
    assert_eq!(memory_recall::fill_now(&state, &k.id, 100).await, 500);
    println!("embedding 500 notes in batches of {}: {:?}", memory_recall::FILL_BATCH, started.elapsed());
    let hybrid = time("hybrid (local fake embedding server)", true).await;
    assert!(keywords < Duration::from_millis(200) && none < Duration::from_millis(200), "{keywords:?} {none:?}");
    assert!(hybrid < Duration::from_millis(500), "{hybrid:?}");
}
