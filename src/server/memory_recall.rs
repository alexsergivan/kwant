//! Hybrid memory recall (roadmap Next #11): a bot's `memory_search` finds notes by meaning as well
//! as by words. Each memory gets an embedding from the space's own provider (its OpenAI account,
//! else a local Ollama with an embedding model pulled), kept in `kwant_memory_vectors`; a search
//! ranks the kwant's notes by cosine similarity and by FTS5 bm25 and merges the two lists with
//! Reciprocal Rank Fusion.
//!
//! With no embedding provider the search is exactly the keyword search it always was: the
//! provider lookup reads only the database and the cached engine checks, so it makes no HTTP call
//! and adds no wait. Memory text only ever goes to a provider the space already set up. Claude has
//! no embeddings API, so a Claude-only space stays keyword-only.
//!
//! Caps: one query embedding per search with a [`QUERY_TIMEOUT`]; vectors are filled in the
//! background, at most [`FILL_BATCH`] × [`FILL_ROUNDS`] per fill and one fill per kwant at a time;
//! after a provider error that provider is left alone for [`COOLDOWN`].

use super::AppState;
use crate::db::continuity::{Memory, MAX_MEMORY_SEARCH_LIMIT};
use crate::db::spaces::SpaceId;
use crate::drivers::engines::{EngineId, EngineStatus};
use crate::profiles::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// The OpenAI embedding model.
pub const OPENAI_MODEL: &str = "text-embedding-3-small";
/// Ollama embedding models, best first; else any pulled model whose name has `embed` in it.
const OLLAMA_PREFERRED: [&str; 2] = ["nomic-embed-text", "mxbai-embed-large"];
/// Longest wait for the query's embedding; past it the search is keyword-only.
pub const QUERY_TIMEOUT: Duration = Duration::from_millis(1500);
/// Longest wait for one batch of memory embeddings (in the background).
const FILL_TIMEOUT: Duration = Duration::from_secs(20);
/// Memories embedded per provider call.
pub const FILL_BATCH: usize = 32;
/// Calls per background fill: 4 × 32 covers a kwant's 100 notes.
pub const FILL_ROUNDS: usize = 4;
/// After a provider error, how long that provider isn't asked again.
pub const COOLDOWN: Duration = Duration::from_secs(60);
/// Candidates taken from each ranking before the merge.
const CANDIDATES: usize = MAX_MEMORY_SEARCH_LIMIT as usize;
/// Reciprocal Rank Fusion constant: score = Σ 1/(RRF_K + rank).
const RRF_K: f64 = 60.0;
/// Below this cosine a note isn't related enough to be a by-meaning candidate.
const MIN_COSINE: f32 = 0.2;

/// Where a space's embeddings come from.
#[derive(Clone, PartialEq)]
pub enum EmbedSource {
    /// `base` is the API root without `/v1` (`https://api.openai.com`).
    OpenAi { base: String, key: String },
    /// `root` is the Ollama server root without `/v1`.
    Ollama { root: String, model: String },
}

impl std::fmt::Debug for EmbedSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbedSource::OpenAi { base, .. } => write!(f, "OpenAi({base}, key <redacted>)"),
            EmbedSource::Ollama { root, model } => write!(f, "Ollama({root}, {model})"),
        }
    }
}

impl EmbedSource {
    /// What a stored vector was made with; a vector from another model is stale.
    pub fn model_id(&self) -> String {
        match self {
            EmbedSource::OpenAi { .. } => format!("openai/{OPENAI_MODEL}"),
            EmbedSource::Ollama { model, .. } => format!("ollama/{model}"),
        }
    }

    /// Cooldown key: one per provider endpoint, never containing the key.
    fn cooldown_key(&self) -> String {
        match self {
            EmbedSource::OpenAi { base, .. } => format!("openai {base}"),
            EmbedSource::Ollama { root, model } => format!("ollama {root} {model}"),
        }
    }
}

/// The Ollama embedding model to use from the pulled `models`, if any.
pub fn pick_ollama_model<'a>(models: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let models: Vec<&str> = models.into_iter().collect();
    let base = |m: &str| m.split(':').next().unwrap_or(m).to_string();
    OLLAMA_PREFERRED
        .iter()
        .find_map(|p| models.iter().find(|m| base(m) == *p))
        .or_else(|| models.iter().find(|m| m.to_ascii_lowercase().contains("embed")))
        .map(|m| m.to_string())
}

/// The space's embedding provider, or `None`. Reads only the database, the environment and the
/// cached engine checks: never a network call.
///
/// 1. An enabled OpenAI API-key account of the space (an `api_key` profile on Codex) with a key.
/// 2. The host's OpenAI key (Settings, else `OPENAI_API_KEY`), unless This machine is off for
///    Codex in the space or the OpenAI API engine is turned off.
/// 3. A local Ollama whose last check was ready and that has an embedding model pulled.
pub async fn source_for_space(state: &AppState, space: &SpaceId) -> Option<EmbedSource> {
    let db = &state.db;
    let openai_on = state.engines.is_enabled(EngineId::OpenaiApi);
    let profiles = db.list_profiles(space).await.unwrap_or_default();
    if openai_on {
        for p in profiles.iter().filter(|p| p.engine == "codex" && p.kind == "api_key" && p.enabled) {
            if let Ok(Some(key)) = db.get_profile_key(&p.id).await {
                if !key.0.trim().is_empty() {
                    return Some(EmbedSource::OpenAi { base: "https://api.openai.com".into(), key: key.0.trim().to_string() });
                }
            }
        }
        let host_off = profiles.iter().any(|p| p.kind == "system" && !p.enabled && Engine::parse(&p.engine) == Some(Engine::Codex));
        if !host_off {
            let key = db
                .get_setting("openai_api_key")
                .await
                .ok()
                .flatten()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .filter(|k| !k.trim().is_empty());
            if let Some(key) = key {
                return Some(EmbedSource::OpenAi { base: "https://api.openai.com".into(), key: key.trim().to_string() });
            }
        }
    }
    let ollama = state.engines.report(EngineId::Ollama);
    if ollama.status != EngineStatus::Ready {
        return None;
    }
    let model = pick_ollama_model(ollama.models.iter().map(|m| m.id.as_str()))?;
    let url = db.get_setting("ollama_url").await.ok().flatten();
    Some(EmbedSource::Ollama { root: crate::drivers::ollama_root(url.as_deref()), model })
}

/// The kwant's space's provider (see [`source_for_space`]).
pub async fn source_for_kwant(state: &AppState, kwant_id: &str) -> Option<EmbedSource> {
    let space = state.db.space_of_kwant(kwant_id).await.ok().flatten()?;
    source_for_space(state, &space).await
}

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

/// Embeds `texts` in one call, in order. `Err` with a short reason that never contains the key.
pub async fn embed(source: &EmbedSource, texts: &[String], timeout: Duration) -> Result<Vec<Vec<f32>>, String> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let req = match source {
        EmbedSource::OpenAi { base, key } => CLIENT
            .post(format!("{}/v1/embeddings", base.trim_end_matches('/')))
            .bearer_auth(key)
            .json(&json!({"model": OPENAI_MODEL, "input": texts})),
        EmbedSource::Ollama { root, model } => CLIENT
            .post(format!("{}/api/embed", root.trim_end_matches('/')))
            .json(&json!({"model": model, "input": texts})),
    };
    let resp = req.timeout(timeout).send().await.map_err(|e| if e.is_timeout() { "timed out".to_string() } else { "not reachable".to_string() })?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    let body: Value = resp.json().await.map_err(|_| "unreadable answer".to_string())?;
    let vectors = parse_embeddings(source, &body).ok_or_else(|| "unexpected answer".to_string())?;
    if vectors.len() != texts.len() || vectors.iter().any(|v| v.is_empty() || v.len() != vectors[0].len()) {
        return Err("wrong number or size of vectors".into());
    }
    Ok(vectors)
}

/// OpenAI: `data[].embedding` (by `index`); Ollama: `embeddings[]`.
fn parse_embeddings(source: &EmbedSource, body: &Value) -> Option<Vec<Vec<f32>>> {
    let floats = |v: &Value| v.as_array().map(|a| a.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect::<Vec<f32>>());
    match source {
        EmbedSource::OpenAi { .. } => {
            let mut items: Vec<(i64, Vec<f32>)> = body["data"]
                .as_array()?
                .iter()
                .enumerate()
                .map(|(i, d)| Some((d["index"].as_i64().unwrap_or(i as i64), floats(&d["embedding"])?)))
                .collect::<Option<_>>()?;
            items.sort_by_key(|(i, _)| *i);
            Some(items.into_iter().map(|(_, v)| v).collect())
        }
        EmbedSource::Ollama { .. } => body["embeddings"].as_array()?.iter().map(floats).collect(),
    }
}

/// Hash of a memory's text, to tell a stale vector from a fresh one.
pub fn text_hash(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().take(16).map(|b| format!("{b:02x}")).collect()
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0f32, 0f32, 0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// Reciprocal Rank Fusion of ranked id lists (best first): score = Σ 1/(60 + rank), rank from 1.
/// Ties keep the order of the first list, then the second.
pub fn rrf(lists: &[&[String]]) -> Vec<String> {
    let mut score: HashMap<&str, f64> = HashMap::new();
    let mut first_seen: Vec<&str> = Vec::new();
    for list in lists {
        for (i, id) in list.iter().enumerate() {
            let s = score.entry(id.as_str()).or_insert_with(|| {
                first_seen.push(id.as_str());
                0.0
            });
            *s += 1.0 / (RRF_K + (i + 1) as f64);
        }
    }
    let mut ids: Vec<(usize, &str)> = first_seen.into_iter().enumerate().collect();
    ids.sort_by(|(ia, a), (ib, b)| score[b].partial_cmp(&score[a]).unwrap_or(std::cmp::Ordering::Equal).then(ia.cmp(ib)));
    ids.into_iter().map(|(_, id)| id.to_string()).collect()
}

/// Providers in their cooldown after an error, until when.
static COOLING: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// Kwants with a fill running.
static FILLING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

fn cooling(source: &EmbedSource) -> bool {
    let mut map = COOLING.lock().unwrap();
    let key = source.cooldown_key();
    match map.get(&key) {
        Some(until) if *until > Instant::now() => true,
        Some(_) => {
            map.remove(&key);
            false
        }
        None => false,
    }
}

fn cool_down(source: &EmbedSource, why: &str) {
    tracing::warn!(provider = ?source, reason = why, "memory embeddings failed; keyword search only for a minute");
    COOLING.lock().unwrap().insert(source.cooldown_key(), Instant::now() + COOLDOWN);
}

/// Forgets every cooldown (tests).
pub fn reset_cooldowns() {
    COOLING.lock().unwrap().clear();
}

/// The kwant's notes for `q`, best first, at most `limit` (1..=20). With no embedding provider,
/// or no stored vectors yet, or when the query can't be embedded in time, this is exactly
/// [`crate::db::Database::search_memories`]. Missing or stale vectors are filled in the
/// background; the reply never waits for that.
pub async fn search(state: &AppState, kwant_id: &str, q: &str, limit: i64) -> Result<Vec<Memory>, crate::db::DbError> {
    let db = &state.db;
    let Some(source) = source_for_kwant(state, kwant_id).await else {
        return db.search_memories(kwant_id, q, limit).await;
    };
    let limit = limit.clamp(1, MAX_MEMORY_SEARCH_LIMIT) as usize;
    let model = source.model_id();
    let rows = db.memory_vector_rows(kwant_id).await?;
    let mut missing = false;
    let vectors: Vec<(String, Vec<f32>)> = rows
        .iter()
        .filter_map(|r| {
            let v = r.fresh_vector(&model, &text_hash(&r.text));
            missing |= v.is_none();
            v.map(|v| (r.id.clone(), v))
        })
        .collect();
    if missing {
        fill_soon(state, kwant_id);
    }
    if vectors.is_empty() || cooling(&source) {
        return db.search_memories(kwant_id, q, limit as i64).await;
    }
    let query_vec = match embed(&source, &[q.to_string()], QUERY_TIMEOUT).await {
        Ok(mut v) => v.remove(0),
        Err(why) => {
            cool_down(&source, &why);
            return db.search_memories(kwant_id, q, limit as i64).await;
        }
    };
    let by_words: Vec<Memory> = db.search_memories(kwant_id, q, CANDIDATES as i64).await?;
    let word_ids: Vec<String> = by_words.iter().map(|m| m.id.clone()).collect();
    let mut by_meaning: Vec<(f32, &String)> = vectors
        .iter()
        .map(|(id, v)| (cosine(&query_vec, v), id))
        .filter(|(c, _)| *c >= MIN_COSINE)
        .collect();
    by_meaning.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let meaning_ids: Vec<String> = by_meaning.into_iter().take(CANDIDATES).map(|(_, id)| id.clone()).collect();
    let merged = rrf(&[&word_ids, &meaning_ids]);
    let mut by_id: HashMap<String, Memory> = by_words.into_iter().map(|m| (m.id.clone(), m)).collect();
    if merged.iter().any(|id| !by_id.contains_key(id)) {
        for m in db.list_memories(kwant_id).await? {
            by_id.entry(m.id.clone()).or_insert(m);
        }
    }
    Ok(merged.into_iter().filter_map(|id| by_id.remove(&id)).take(limit).collect())
}

/// Starts a background fill of the kwant's missing or stale vectors, unless one is running.
pub fn fill_soon(state: &AppState, kwant_id: &str) {
    let (state, kwant_id) = (state.clone(), kwant_id.to_string());
    tokio::spawn(async move {
        fill_now(&state, &kwant_id, FILL_ROUNDS).await;
    });
}

/// Embeds up to `rounds` × [`FILL_BATCH`] of the kwant's notes whose vector is missing or stale,
/// and returns how many it stored. 0 without a provider (no call made), during a cooldown, or
/// when another fill for the kwant is running.
pub async fn fill_now(state: &AppState, kwant_id: &str, rounds: usize) -> usize {
    if !FILLING.lock().unwrap().insert(kwant_id.to_string()) {
        return 0;
    }
    let stored = fill_rounds(state, kwant_id, rounds).await;
    FILLING.lock().unwrap().remove(kwant_id);
    stored
}

async fn fill_rounds(state: &AppState, kwant_id: &str, rounds: usize) -> usize {
    let Some(source) = source_for_kwant(state, kwant_id).await else {
        return 0;
    };
    let model = source.model_id();
    let mut stored = 0;
    for _ in 0..rounds {
        if cooling(&source) {
            break;
        }
        let Ok(rows) = state.db.memory_vector_rows(kwant_id).await else {
            break;
        };
        let todo: Vec<(String, String, String)> = rows
            .into_iter()
            .filter_map(|r| {
                let hash = text_hash(&r.text);
                r.fresh_vector(&model, &hash).is_none().then_some((r.id, r.text, hash))
            })
            .take(FILL_BATCH)
            .collect();
        if todo.is_empty() {
            break;
        }
        let texts: Vec<String> = todo.iter().map(|(_, t, _)| t.clone()).collect();
        match embed(&source, &texts, FILL_TIMEOUT).await {
            Ok(vectors) => {
                for ((id, _, hash), v) in todo.iter().zip(vectors) {
                    if let Ok(true) = state.db.put_memory_vector(id, &model, &v, hash).await {
                        stored += 1;
                    }
                }
            }
            Err(why) => {
                cool_down(&source, &why);
                break;
            }
        }
        if todo.len() < FILL_BATCH {
            break;
        }
    }
    stored
}

/// Fills a kwant's vectors after each memory write (bot or person, add or edit), until shutdown.
/// Imports and anything this misses are filled lazily by the next search.
pub fn spawn(state: AppState, shutdown: tokio_util::sync::CancellationToken) {
    let mut rx = state.event_sender.subscribe();
    tokio::spawn(async move {
        loop {
            let ev = tokio::select! {
                _ = shutdown.cancelled() => return,
                ev = rx.recv() => ev,
            };
            match ev {
                Ok(crate::drivers::events::RuntimeEvent::MemoryChanged { kwant_id }) => fill_soon(&state, &kwant_id),
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_rewards_notes_both_lists_agree_on() {
        let words: Vec<String> = ["a", "b", "c"].map(String::from).into();
        let meaning: Vec<String> = ["c", "d", "a"].map(String::from).into();
        // a: 1/61 + 1/63, c: 1/63 + 1/61 (tie, a first from the first list), b: 1/62, d: 1/62
        assert_eq!(rrf(&[&words, &meaning]), ["a", "c", "b", "d"]);
        assert_eq!(rrf(&[&words, &[]]), ["a", "b", "c"], "one list alone keeps its order");
        assert!(rrf(&[&[], &[]]).is_empty());
    }

    #[test]
    fn cosine_handles_zero_and_mismatched_vectors() {
        assert!((cosine(&[1.0, 0.0], &[2.0, 0.0]) - 1.0).abs() < 1e-6);
        assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 0.0]), 0.0);
        assert_eq!(cosine(&[1.0], &[1.0, 0.0]), 0.0);
    }

    #[test]
    fn picks_an_embedding_model_only() {
        assert_eq!(pick_ollama_model(["llama3.2:latest", "nomic-embed-text:latest"]).as_deref(), Some("nomic-embed-text:latest"));
        assert_eq!(pick_ollama_model(["mxbai-embed-large", "nomic-embed-text"]).as_deref(), Some("nomic-embed-text"));
        assert_eq!(pick_ollama_model(["llama3.2", "snowflake-arctic-embed:s"]).as_deref(), Some("snowflake-arctic-embed:s"));
        assert_eq!(pick_ollama_model(["llama3.2", "qwen3"]), None);
    }

    #[test]
    fn parses_both_answer_shapes_and_hides_the_key() {
        let openai = EmbedSource::OpenAi { base: "https://x".into(), key: "sk-secret".into() };
        let body = json!({"data": [{"index": 1, "embedding": [0.0, 1.0]}, {"index": 0, "embedding": [1.0, 0.0]}]});
        assert_eq!(parse_embeddings(&openai, &body).unwrap(), vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
        let ollama = EmbedSource::Ollama { root: "http://o".into(), model: "nomic-embed-text".into() };
        assert_eq!(parse_embeddings(&ollama, &json!({"embeddings": [[0.5, 0.5]]})).unwrap(), vec![vec![0.5, 0.5]]);
        assert!(parse_embeddings(&ollama, &json!({"error": "no"})).is_none());
        assert!(!format!("{openai:?}").contains("sk-secret"));
        assert_eq!(openai.model_id(), "openai/text-embedding-3-small");
        assert_eq!(ollama.model_id(), "ollama/nomic-embed-text");
    }

    #[test]
    fn text_hash_tells_texts_apart() {
        assert_eq!(text_hash("a"), text_hash("a"));
        assert_ne!(text_hash("a"), text_hash("b"));
        assert_eq!(text_hash("a").len(), 32);
    }
}
