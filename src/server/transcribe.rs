//! Dictation (roadmap Next 12b): the message box's mic button sends its recording here and gets
//! the spoken words back as text, to put in the text box. The person reads and edits them there;
//! nothing reaches a bot before they press Send.
//!
//! The words come from the space's own OpenAI account, in the order hybrid recall uses for
//! embeddings (`memory_recall::source_for_space`), minus Ollama, which has no speech-to-text:
//! 1. an enabled OpenAI API-key account of the space (an `api_key` profile on Codex) with a key;
//! 2. the host's OpenAI key (Settings, else `OPENAI_API_KEY`), unless This machine is off for
//!    Codex in the space;
//!
//! and neither while the OpenAI API engine is turned off. Local Whisper is a follow-up.
//!
//! Nothing is stored: the audio stays in memory for the one call (at most [`MAX_AUDIO_BYTES`]),
//! no attachment row, no file, no chat message, no memory. Only the person transcribes: a bot's
//! turn token is refused, so no bot can spend the space's OpenAI money through here. At most
//! [`MAX_AT_ONCE`] recordings are sent at a time; the next waits up to [`QUEUE_WAIT`]. The key never appears in an answer or a log
//! line, and neither does OpenAI's own error text (its 401 quotes part of the key).

use super::AppState;
use crate::db::spaces::SpaceId;
use crate::drivers::engines::EngineId;
use crate::profiles::Engine;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::LazyLock;
use std::time::Duration;
use tokio::sync::Semaphore;

/// OpenAI's own limit for one recording.
pub const MAX_AUDIO_BYTES: u64 = 25 * 1024 * 1024;
/// Longest wait for OpenAI's answer, per model tried.
pub const TIMEOUT: Duration = Duration::from_secs(60);
/// The model asked first, and the one tried when OpenAI says the first doesn't exist.
pub const MODEL: &str = "gpt-4o-mini-transcribe";
pub const FALLBACK_MODEL: &str = "whisper-1";
/// Recordings sent to OpenAI at the same time, across all spaces. One more waits up to
/// [`QUEUE_WAIT`] for its turn, then gets 429 `busy`.
pub const MAX_AT_ONCE: usize = 2;
pub const QUEUE_WAIT: Duration = Duration::from_secs(30);
/// Test seam: another OpenAI address (only with the `test-seams` feature, like Telegram's).
pub const API_BASE_ENV: &str = "KWNT_OPENAI_API_BASE";
const OPENAI_API: &str = "https://api.openai.com";

pub const WHY_NO_KEY: &str = "Add an OpenAI API key in Apps & accounts to dictate.";
pub const WHY_HOST_OFF: &str =
    "This machine is turned off for OpenAI in this space, so its key isn't used. Add an OpenAI API key to this space in Apps & accounts to dictate.";
pub const WHY_ENGINE_OFF: &str = "The OpenAI API is turned off in Apps & accounts. Turn it on to dictate.";
pub const WHY_KEY_REFUSED: &str = "The OpenAI key was refused.";

/// The recording types the mic button can send (`MediaRecorder` gives webm in Chrome and
/// Firefox, mp4 in Safari), with the file name OpenAI is told. Parameters such as
/// `;codecs=opus` are ignored.
const AUDIO_TYPES: [(&str, &str); 10] = [
    ("audio/webm", "audio.webm"),
    ("audio/mp4", "audio.mp4"),
    ("audio/x-m4a", "audio.m4a"),
    ("audio/m4a", "audio.m4a"),
    ("audio/mpeg", "audio.mp3"),
    ("audio/mp3", "audio.mp3"),
    ("audio/wav", "audio.wav"),
    ("audio/x-wav", "audio.wav"),
    ("audio/wave", "audio.wav"),
    ("audio/ogg", "audio.ogg"),
];

/// Where a space's dictation goes.
#[derive(Clone, PartialEq)]
pub enum TranscribeSource {
    /// `base` is the API root without `/v1`.
    OpenAi { base: String, key: String },
}

impl std::fmt::Debug for TranscribeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranscribeSource::OpenAi { base, .. } => write!(f, "OpenAi({base}, key <redacted>)"),
        }
    }
}

impl TranscribeSource {
    /// The `source` the routes answer with.
    pub fn name(&self) -> &'static str {
        match self {
            TranscribeSource::OpenAi { .. } => "openai",
        }
    }
}

/// What decides a space's source, read from the database and Settings.
#[derive(Debug, Default, Clone)]
pub struct KeyFacts {
    /// The OpenAI API engine is on (Settings).
    pub openai_on: bool,
    /// The key of the space's first enabled Codex `api_key` account that has one.
    pub space_key: Option<String>,
    /// This machine is off for Codex in the space.
    pub host_off: bool,
    /// The host's key (Settings, else `OPENAI_API_KEY`).
    pub host_key: Option<String>,
}

/// The key to use, or why there is none (a sentence for the person). Pure: the order lives here.
pub fn pick(facts: &KeyFacts) -> Result<String, &'static str> {
    let usable = |k: &Option<String>| k.as_deref().map(str::trim).filter(|k| !k.is_empty()).map(str::to_string);
    if !facts.openai_on {
        return Err(if usable(&facts.space_key).is_some() || usable(&facts.host_key).is_some() { WHY_ENGINE_OFF } else { WHY_NO_KEY });
    }
    if let Some(k) = usable(&facts.space_key) {
        return Ok(k);
    }
    match usable(&facts.host_key) {
        Some(_) if facts.host_off => Err(WHY_HOST_OFF),
        Some(k) => Ok(k),
        None => Err(WHY_NO_KEY),
    }
}

/// Reads [`KeyFacts`] for `space`: only the database and the environment, never a network call.
pub async fn key_facts(state: &AppState, space: &SpaceId) -> KeyFacts {
    let db = &state.db;
    let profiles = db.list_profiles(space).await.unwrap_or_default();
    let mut space_key = None;
    for p in profiles.iter().filter(|p| p.engine == "codex" && p.kind == "api_key" && p.enabled) {
        if let Ok(Some(key)) = db.get_profile_key(&p.id).await {
            if !key.0.trim().is_empty() {
                space_key = Some(key.0);
                break;
            }
        }
    }
    let host_off = profiles.iter().any(|p| p.kind == "system" && !p.enabled && Engine::parse(&p.engine) == Some(Engine::Codex));
    let host_key = db.get_setting("openai_api_key").await.ok().flatten().or_else(|| std::env::var("OPENAI_API_KEY").ok()).filter(|k| !k.trim().is_empty());
    KeyFacts { openai_on: state.engines.is_enabled(EngineId::OpenaiApi), space_key, host_off, host_key }
}

/// The space's dictation source, or why there is none.
pub async fn transcribe_source_for_space(state: &AppState, space: &SpaceId) -> Result<TranscribeSource, &'static str> {
    pick(&key_facts(state, space).await).map(|key| TranscribeSource::OpenAi { base: api_base(), key })
}

fn api_base() -> String {
    base_for(crate::clock::TEST_SEAMS, std::env::var(API_BASE_ENV).ok())
}

/// The override counts only with the test seams built in, and only as an http(s) address, so a
/// shipped build always sends the key to api.openai.com.
fn base_for(seams: bool, override_base: Option<String>) -> String {
    if seams {
        if let Some(b) = override_base {
            let b = b.trim().trim_end_matches('/');
            if b.starts_with("http://") || b.starts_with("https://") {
                return b.to_string();
            }
        }
    }
    OPENAI_API.to_string()
}

/// The file name for a `Content-Type`, or `None` when it isn't a recording type we send.
pub fn audio_file_name(content_type: Option<&str>) -> Option<&'static str> {
    let ct = content_type?.split(';').next()?.trim().to_ascii_lowercase();
    AUDIO_TYPES.iter().find(|(t, _)| *t == ct).map(|(_, name)| *name)
}

/// An ISO-639-1 hint: two ASCII letters, lower-cased.
fn clean_lang(lang: Option<&str>) -> Result<Option<String>, ()> {
    match lang.map(str::trim).filter(|l| !l.is_empty()) {
        None => Ok(None),
        Some(l) if l.len() == 2 && l.bytes().all(|b| b.is_ascii_alphabetic()) => Ok(Some(l.to_ascii_lowercase())),
        Some(_) => Err(()),
    }
}

fn refusal(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(json!({"error": message, "code": code}))).into_response()
}

fn no_transcriber(why: &str) -> Response {
    (StatusCode::CONFLICT, Json(json!({"error": why, "code": "no_transcriber", "why": why}))).into_response()
}

/// `GET /api/spaces/:id/transcribe` → `{available, source: "openai"|null, why: string|null}`.
pub async fn status(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let space = SpaceId::new(id.trim());
    match state.db.get_space(&space).await {
        Ok(Some(_)) => {}
        Ok(None) => return refusal(StatusCode::NOT_FOUND, "no_space", "no such space"),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
    Json(status_json(transcribe_source_for_space(&state, &space).await)).into_response()
}

pub fn status_json(source: Result<TranscribeSource, &'static str>) -> Value {
    match source {
        Ok(s) => json!({"available": true, "source": s.name(), "why": null}),
        Err(why) => json!({"available": false, "source": null, "why": why}),
    }
}

#[derive(Deserialize)]
pub struct TranscribeQuery {
    pub lang: Option<String>,
}

static AT_ONCE: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(MAX_AT_ONCE));
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

/// The pooled client for api.openai.com, or a new one for any other base (only a test-seams
/// build has one: a stub). Each `#[tokio::test]` runs on its own runtime, and a connection pooled
/// during one test dies with that test's runtime, so a shared pool let a test's upload ride a
/// connection that was cut halfway ("OpenAI couldn't be reached.", #610 CI at 06:02).
fn client_for(base: &str) -> reqwest::Client {
    if base == OPENAI_API {
        CLIENT.clone()
    } else {
        reqwest::Client::new()
    }
}

/// `POST /api/spaces/:id/transcribe[?lang=xx]` with the recording as the raw body → 200
/// `{text, source}`. Refusals come before any byte is read where they can: 403 for a bot's
/// token, 404 no space, 409 archived space, 413 a declared length over 25 MB, 415 not a
/// recording type, 400 a bad `lang`, 409 `no_transcriber`; then 413 at the byte that passes
/// 25 MB, 400 `empty_audio`, 429 `busy`, 502 `transcriber_failed` (see docs/dictation.md).
pub async fn transcribe(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<TranscribeQuery>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if let Err(r) = super::mcp_suggest::refuse_agent_tokens(&state, &headers, "only the person can dictate").await {
        return r;
    }
    let space = match super::spaces::active_space(&state, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let declared = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > MAX_AUDIO_BYTES) {
        return too_large();
    }
    let Some(file_name) = audio_file_name(headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok())) else {
        return refusal(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "bad_audio_type",
            "Send the recording as audio/webm, audio/mp4, audio/mpeg, audio/wav or audio/ogg.",
        );
    };
    let Ok(lang) = clean_lang(q.lang.as_deref()) else {
        return refusal(StatusCode::BAD_REQUEST, "bad_lang", "lang must be a two-letter language code such as en.");
    };
    let source = match transcribe_source_for_space(&state, &space).await {
        Ok(s) => s,
        Err(why) => return no_transcriber(why),
    };
    let mut audio = Vec::with_capacity(declared.unwrap_or(0).min(MAX_AUDIO_BYTES) as usize);
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return refusal(StatusCode::BAD_REQUEST, "interrupted", "The recording was interrupted.");
        };
        if audio.len() as u64 + chunk.len() as u64 > MAX_AUDIO_BYTES {
            return too_large();
        }
        audio.extend_from_slice(&chunk);
    }
    if audio.is_empty() {
        return refusal(StatusCode::BAD_REQUEST, "empty_audio", "The recording is empty.");
    }
    let Ok(Ok(_slot)) = tokio::time::timeout(QUEUE_WAIT, AT_ONCE.acquire()).await else {
        return refusal(StatusCode::TOO_MANY_REQUESTS, "busy", "Other recordings are being turned into text. Try again in a moment.");
    };
    let size = audio.len();
    match send(&source, audio, file_name, lang.as_deref()).await {
        Ok((text, model)) => {
            tracing::info!(space = %space, bytes = size, model, "dictation transcribed");
            Json(json!({"text": text, "source": source.name()})).into_response()
        }
        Err(why) => {
            tracing::warn!(space = %space, bytes = size, why, "dictation failed");
            refusal(StatusCode::BAD_GATEWAY, "transcriber_failed", why)
        }
    }
}

fn too_large() -> Response {
    refusal(StatusCode::PAYLOAD_TOO_LARGE, "too_large", "The recording is over 25 MB. Record a shorter message.")
}

/// Sends the recording with [`MODEL`], then [`FALLBACK_MODEL`] if OpenAI says the first doesn't
/// exist. `Ok((text, model))`, or a plain reason that never has the key or OpenAI's own words.
pub async fn send(source: &TranscribeSource, audio: Vec<u8>, file_name: &str, lang: Option<&str>) -> Result<(String, &'static str), &'static str> {
    match send_one(source, &audio, file_name, lang, MODEL).await {
        Err(Upstream::ModelMissing) => match send_one(source, &audio, file_name, lang, FALLBACK_MODEL).await {
            Ok(text) => Ok((text, FALLBACK_MODEL)),
            Err(e) => Err(e.why()),
        },
        Ok(text) => Ok((text, MODEL)),
        Err(e) => Err(e.why()),
    }
}

#[derive(Debug, PartialEq)]
enum Upstream {
    ModelMissing,
    Status(u16),
    Timeout,
    Unreachable,
    Unreadable,
}

impl Upstream {
    fn why(&self) -> &'static str {
        match self {
            Upstream::Status(401 | 403) => WHY_KEY_REFUSED,
            Upstream::Status(429) => "OpenAI is limiting this key, or its quota is used up. Try again in a minute.",
            Upstream::Status(413) => "The recording is too large for OpenAI.",
            Upstream::Status(400 | 404 | 415 | 422) | Upstream::ModelMissing => "OpenAI couldn't read this recording.",
            Upstream::Status(_) => "OpenAI had a problem. Try again in a minute.",
            Upstream::Timeout => "OpenAI didn't answer within 60 seconds.",
            Upstream::Unreachable => "OpenAI couldn't be reached.",
            Upstream::Unreadable => "OpenAI's answer couldn't be read.",
        }
    }
}

/// One `multipart/form-data` body for `/v1/audio/transcriptions`.
fn multipart(boundary: &str, audio: &[u8], file_name: &str, lang: Option<&str>, model: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(audio.len() + 512);
    let mut field = |name: &str, value: &str| {
        out.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    };
    field("model", model);
    field("response_format", "json");
    if let Some(l) = lang {
        field("language", l);
    }
    out.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n")
            .as_bytes(),
    );
    out.extend_from_slice(audio);
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    out
}

async fn send_one(source: &TranscribeSource, audio: &[u8], file_name: &str, lang: Option<&str>, model: &str) -> Result<String, Upstream> {
    let TranscribeSource::OpenAi { base, key } = source;
    let boundary = format!("kwnt-{}", uuid::Uuid::new_v4().simple());
    let resp = client_for(base)
        .post(format!("{}/v1/audio/transcriptions", base.trim_end_matches('/')))
        .bearer_auth(key)
        .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
        .body(multipart(&boundary, audio, file_name, lang, model))
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                return Upstream::Timeout;
            }
            // The error without its address; the key is a header and is never in it.
            tracing::warn!(error = %e.without_url(), "dictation: OpenAI couldn't be reached");
            Upstream::Unreachable
        })?;
    let status = resp.status().as_u16();
    if status != 200 {
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        return Err(if model_missing(status, &body) { Upstream::ModelMissing } else { Upstream::Status(status) });
    }
    let body: Value = resp.json().await.map_err(|e| if e.is_timeout() { Upstream::Timeout } else { Upstream::Unreadable })?;
    body["text"].as_str().map(|t| t.trim().to_string()).ok_or(Upstream::Unreadable)
}

/// OpenAI's answer for a model this key can't use: 404 (or 400) with `error.code`
/// `model_not_found`, or an error about the `model` parameter.
fn model_missing(status: u16, body: &Value) -> bool {
    let e = &body["error"];
    matches!(status, 400 | 404) && (e["code"] == "model_not_found" || e["param"] == "model")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(openai_on: bool, space_key: Option<&str>, host_off: bool, host_key: Option<&str>) -> KeyFacts {
        KeyFacts { openai_on, space_key: space_key.map(Into::into), host_off, host_key: host_key.map(Into::into) }
    }

    #[test]
    fn the_space_key_beats_the_host_key() {
        assert_eq!(pick(&facts(true, Some("sk-space"), false, Some("sk-host"))), Ok("sk-space".into()));
        assert_eq!(pick(&facts(true, Some(" sk-space \n"), true, None)), Ok("sk-space".into()));
    }

    #[test]
    fn the_host_key_is_used_unless_this_machine_is_off() {
        assert_eq!(pick(&facts(true, None, false, Some("sk-host"))), Ok("sk-host".into()));
        assert_eq!(pick(&facts(true, None, true, Some("sk-host"))), Err(WHY_HOST_OFF));
        assert_eq!(pick(&facts(true, Some("  "), true, Some("sk-host"))), Err(WHY_HOST_OFF), "a blank space key is no key");
    }

    #[test]
    fn no_key_or_the_engine_off_gives_the_why() {
        assert_eq!(pick(&facts(true, None, false, None)), Err(WHY_NO_KEY));
        assert_eq!(pick(&facts(true, Some(""), false, Some(" "))), Err(WHY_NO_KEY));
        assert_eq!(pick(&facts(false, Some("sk-space"), false, Some("sk-host"))), Err(WHY_ENGINE_OFF));
        assert_eq!(pick(&facts(false, None, false, None)), Err(WHY_NO_KEY));
        for why in [WHY_NO_KEY, WHY_HOST_OFF, WHY_ENGINE_OFF] {
            assert!(!why.contains("sk-"));
        }
    }

    #[test]
    fn the_source_never_prints_its_key() {
        let s = TranscribeSource::OpenAi { base: OPENAI_API.into(), key: "sk-secret".into() };
        assert!(!format!("{s:?}").contains("sk-secret"));
        assert_eq!(s.name(), "openai");
        let v = status_json(Ok(s));
        assert_eq!(v, json!({"available": true, "source": "openai", "why": null}));
        assert_eq!(status_json(Err(WHY_NO_KEY)), json!({"available": false, "source": null, "why": WHY_NO_KEY}));
    }

    #[test]
    fn recording_types_and_language_hints() {
        assert_eq!(audio_file_name(Some("audio/webm;codecs=opus")), Some("audio.webm"));
        assert_eq!(audio_file_name(Some("Audio/MP4")), Some("audio.mp4"));
        assert_eq!(audio_file_name(Some("audio/mpeg")), Some("audio.mp3"));
        assert_eq!(audio_file_name(Some("audio/wav")), Some("audio.wav"));
        assert_eq!(audio_file_name(Some("audio/ogg; codecs=opus")), Some("audio.ogg"));
        for bad in [Some("text/plain"), Some("video/webm"), Some("application/octet-stream"), Some(""), None] {
            assert_eq!(audio_file_name(bad), None, "{bad:?}");
        }
        assert_eq!(clean_lang(None), Ok(None));
        assert_eq!(clean_lang(Some("")), Ok(None));
        assert_eq!(clean_lang(Some("EN")), Ok(Some("en".into())));
        for bad in ["eng", "e", "e1", "en-US", "ü1"] {
            assert_eq!(clean_lang(Some(bad)), Err(()), "{bad}");
        }
    }

    #[test]
    fn only_test_builds_may_point_elsewhere() {
        assert_eq!(base_for(false, Some("http://127.0.0.1:9".into())), OPENAI_API);
        assert_eq!(base_for(true, Some("http://127.0.0.1:9/".into())), "http://127.0.0.1:9");
        assert_eq!(base_for(true, Some("file:///etc".into())), OPENAI_API);
        assert_eq!(base_for(true, None), OPENAI_API);
    }

    #[test]
    fn a_missing_model_is_told_apart_from_other_errors() {
        assert!(model_missing(404, &json!({"error": {"code": "model_not_found"}})));
        assert!(model_missing(400, &json!({"error": {"param": "model", "message": "invalid model"}})));
        assert!(!model_missing(400, &json!({"error": {"param": "file"}})));
        assert!(!model_missing(401, &json!({"error": {"code": "model_not_found"}})));
        assert_eq!(Upstream::Status(401).why(), WHY_KEY_REFUSED);
        assert_eq!(Upstream::Timeout.why(), "OpenAI didn't answer within 60 seconds.");
    }

    #[test]
    fn the_multipart_body_has_the_fields_and_the_bytes() {
        let body = multipart("B", b"\x00\x01RIFF", "audio.wav", Some("de"), MODEL);
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with("--B\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\ngpt-4o-mini-transcribe\r\n"), "{text}");
        assert!(text.contains("name=\"language\"\r\n\r\nde\r\n"));
        assert!(text.contains("name=\"response_format\"\r\n\r\njson\r\n"));
        assert!(text.contains("filename=\"audio.wav\""));
        assert!(body.windows(6).any(|w| w == b"\x00\x01RIFF"));
        assert!(text.ends_with("\r\n--B--\r\n"));
        assert!(!String::from_utf8_lossy(&multipart("B", b"x", "audio.webm", None, MODEL)).contains("language"));
    }
}
