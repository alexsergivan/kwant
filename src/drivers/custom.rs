//! Custom API accounts: any server speaking the OpenAI Chat Completions or the Anthropic Messages
//! protocol (Agent Maestro, OpenRouter, LM Studio, vLLM, …). Here: the base URL rules, the URLs a
//! turn and a check use, and the check that says whether the server answers and which models it
//! lists. Turns run on [`super::ApiDriver::for_custom`].

use super::engines::{tier_from_name, EngineModel};
use serde::Serialize;
use std::time::Duration;

/// How long a check waits for the server.
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Most models a check keeps (OpenRouter lists hundreds).
const MAX_MODELS: usize = 500;
/// Longest base URL or model id accepted.
const MAX_URL_LEN: usize = 2048;
const MAX_MODEL_LEN: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Protocol {
    #[serde(rename = "openai-chat")]
    OpenaiChat,
    #[serde(rename = "anthropic-messages")]
    AnthropicMessages,
}

impl Protocol {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "openai-chat" => Some(Protocol::OpenaiChat),
            "anthropic-messages" => Some(Protocol::AnthropicMessages),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::OpenaiChat => "openai-chat",
            Protocol::AnthropicMessages => "anthropic-messages",
        }
    }

    /// Whether turns on this protocol get the bot's MCP tools. Both send them as `tools` and
    /// kwnt runs the calls; kept for clients that still show a "no tools" badge.
    pub fn supports_tools(self) -> bool {
        true
    }
}

/// Checks a base URL and brings it to the API root that `/chat/completions`, `/messages` and
/// `/models` hang off: http(s) only, no credentials, query or fragment (keys go in the API key
/// field, never the URL), and a pasted endpoint URL cut back to its root. Anthropic roots end
/// in `/v1`, added when missing ("http://127.0.0.1:23333/api/anthropic" works as is). OpenAI
/// roots are kept as given, since not every server puts its API under `/v1`.
pub fn normalize_base(raw: &str, protocol: Protocol) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("base_url is required".into());
    }
    if raw.len() > MAX_URL_LEN || raw.chars().any(char::is_control) {
        return Err("base_url is not a valid URL".into());
    }
    let url = url::Url::parse(raw).map_err(|_| "base_url is not a valid URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("base_url must start with http:// or https://".into());
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("base_url needs a host".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("base_url must not contain a user name or password; put the key in api_key".into());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("base_url must not contain ?query or #fragment; put the key in api_key".into());
    }
    let mut path = url.path().trim_end_matches('/').to_string();
    for suffix in ["/chat/completions", "/completions", "/messages", "/models", "/responses"] {
        if let Some(root) = path.strip_suffix(suffix) {
            path = root.trim_end_matches('/').to_string();
            break;
        }
    }
    if protocol == Protocol::AnthropicMessages && !path.ends_with("/v1") {
        path.push_str("/v1");
    }
    let mut root = url.clone();
    root.set_path(&path);
    Ok(root.as_str().trim_end_matches('/').to_string())
}

/// Validates a hand-entered model id: 1 to 200 printable characters, no spaces.
pub fn clean_model(raw: &str) -> Result<Option<String>, String> {
    let model = raw.trim();
    if model.is_empty() {
        return Ok(None);
    }
    if model.chars().count() > MAX_MODEL_LEN || model.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("model must be up to 200 characters with no spaces".into());
    }
    Ok(Some(model.to_string()))
}

/// Where turns go: `{base}/chat/completions` or `{base}/messages`. The Anthropic one ends in
/// `/v1/messages`, which is how [`super::ApiDriver`] tells the protocols apart.
pub fn chat_url(base: &str, protocol: Protocol) -> String {
    match protocol {
        Protocol::OpenaiChat => format!("{base}/chat/completions"),
        Protocol::AnthropicMessages => format!("{base}/messages"),
    }
}

pub fn models_url(base: &str) -> String {
    format!("{base}/models")
}

/// The message for a server that doesn't answer at all. It names Agent Maestro only for its
/// address (localhost:23333 or its `/api/openai` / `/api/anthropic` paths).
pub fn not_answering(base: &str) -> String {
    let server = if is_agent_maestro(base) { "Agent Maestro" } else { "the server" };
    format!("Nothing is answering at {base}. Is {server} running?")
}

fn is_agent_maestro(base: &str) -> bool {
    let lower = base.to_ascii_lowercase();
    let rest = lower.split_once("://").map_or(lower.as_str(), |(_, r)| r);
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let local = ["localhost:23333", "127.0.0.1:23333", "[::1]:23333"].contains(&authority);
    let path = format!("/{path}");
    local || path.starts_with("/api/openai") || path.starts_with("/api/anthropic")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// The server answered: turns can run.
    Ready,
    /// Connection refused, DNS failure or timeout.
    Unreachable,
    /// 401/403: the key is missing or refused.
    KeyRejected,
    /// Anything else (unexpected status, no models and none entered).
    Error,
}

/// One check of a Custom API account.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CustomCheck {
    pub status: CheckStatus,
    /// Readable, never contains the key.
    pub detail: String,
    /// What the server listed; empty when it lists none.
    pub models: Vec<EngineModel>,
    pub checked_at: chrono::DateTime<chrono::Utc>,
}

impl CustomCheck {
    pub fn new(status: CheckStatus, detail: impl Into<String>, models: Vec<EngineModel>) -> Self {
        Self { status, detail: detail.into(), models, checked_at: chrono::Utc::now() }
    }

    pub fn ready(&self) -> bool {
        self.status == CheckStatus::Ready
    }
}

/// The models in a `GET /models` body: OpenAI's and Anthropic's `{"data": [{"id": …}]}`, with
/// `display_name` (Anthropic) or `name` (OpenRouter) as the label. `model`, the account's own
/// model, is marked as the default when listed. `None` when the body isn't a model list.
pub fn parse_models(body: &serde_json::Value, model: Option<&str>) -> Option<Vec<EngineModel>> {
    let data = body.get("data").or_else(|| body.get("models"))?.as_array()?;
    let mut models: Vec<EngineModel> = data
        .iter()
        .filter_map(|m| {
            let id = m["id"].as_str().or_else(|| m.as_str())?.trim();
            if id.is_empty() || id.len() > MAX_MODEL_LEN {
                return None;
            }
            let label = m["display_name"].as_str().or_else(|| m["name"].as_str()).unwrap_or(id);
            Some(EngineModel {
                id: id.to_string(),
                label: crate::profiles::classify::redact_detail(label, 120),
                default: model.is_some_and(|d| d.eq_ignore_ascii_case(id)),
                tier: tier_from_name(id),
            })
        })
        .take(MAX_MODELS)
        .collect();
    let mut seen = std::collections::HashSet::new();
    models.retain(|m| seen.insert(m.id.clone()));
    Some(models)
}

/// Asks the server for its models: ready when it answers, else why not. A server without a
/// model list (Agent Maestro's OpenAI side) is ready when the account names a model.
pub async fn check(protocol: Protocol, base: &str, key: Option<&str>, model: Option<&str>) -> CustomCheck {
    let Ok(client) = reqwest::Client::builder()
        .timeout(CHECK_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
    else {
        return CustomCheck::new(CheckStatus::Error, "Couldn't start the check", Vec::new());
    };
    let mut req = client.get(models_url(base));
    if protocol == Protocol::AnthropicMessages {
        req = req.header("anthropic-version", super::api::ANTHROPIC_VERSION);
    }
    if let Some(key) = key {
        req = match protocol {
            Protocol::AnthropicMessages => req.header("x-api-key", key),
            Protocol::OpenaiChat => req.bearer_auth(key),
        };
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) if e.is_timeout() => {
            return CustomCheck::new(
                CheckStatus::Unreachable,
                format!("{base} didn't answer within {}s. Is the server running and reachable from this machine?", CHECK_TIMEOUT.as_secs()),
                Vec::new(),
            )
        }
        Err(_) => return CustomCheck::new(CheckStatus::Unreachable, not_answering(base), Vec::new()),
    };
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let listed = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| parse_models(&v, model));
    let using = |why: &str| {
        model.map(|m| CustomCheck::new(CheckStatus::Ready, format!("{why}; using {m}"), Vec::new()))
    };
    match status.as_u16() {
        200..=299 => match listed {
            Some(models) if !models.is_empty() => {
                let n = models.len();
                let detail = format!("Connected · {n} model{}", if n == 1 { "" } else { "s" });
                CustomCheck::new(CheckStatus::Ready, detail, models)
            }
            _ => using("Connected; the server lists no models").unwrap_or_else(|| {
                CustomCheck::new(
                    CheckStatus::Error,
                    "Connected, but the server lists no models. Enter a model id for this account.",
                    Vec::new(),
                )
            }),
        },
        401 | 403 => CustomCheck::new(
            CheckStatus::KeyRejected,
            match key {
                Some(_) => format!("The server refused the API key (HTTP {}). Check the key.", status.as_u16()),
                None => format!("The server needs an API key (HTTP {}). Add one to this account.", status.as_u16()),
            },
            Vec::new(),
        ),
        404 | 405 | 501 => using("Connected; this server doesn't list its models").unwrap_or_else(|| {
            let hint = if base.ends_with("/v1") {
                ""
            } else {
                " Most servers' base URL ends in /v1."
            };
            CustomCheck::new(
                CheckStatus::Error,
                format!(
                    "{} has no model list (HTTP {}). Enter a model id for this account (Agent Maestro's OpenAI side needs one).{hint}",
                    models_url(base),
                    status.as_u16()
                ),
                Vec::new(),
            )
        }),
        429 => CustomCheck::new(
            CheckStatus::Error,
            "The server is rate-limiting requests (HTTP 429). Try again in a minute.",
            Vec::new(),
        ),
        code => CustomCheck::new(
            CheckStatus::Error,
            format!("The server answered HTTP {code}: {}", super::api::error_message(&body)),
            Vec::new(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_urls_normalize_to_the_api_root() {
        let oa = Protocol::OpenaiChat;
        let an = Protocol::AnthropicMessages;
        assert_eq!(normalize_base("http://localhost:1234/v1/", oa).unwrap(), "http://localhost:1234/v1");
        assert_eq!(
            normalize_base(" https://openrouter.ai/api/v1/chat/completions ", oa).unwrap(),
            "https://openrouter.ai/api/v1"
        );
        assert_eq!(
            normalize_base("http://127.0.0.1:23333/api/openai/v1", oa).unwrap(),
            "http://127.0.0.1:23333/api/openai/v1"
        );
        // OpenAI roots aren't guessed: Gemini's is /v1beta/openai
        assert_eq!(
            normalize_base("https://generativelanguage.googleapis.com/v1beta/openai/", oa).unwrap(),
            "https://generativelanguage.googleapis.com/v1beta/openai"
        );
        assert_eq!(
            normalize_base("http://127.0.0.1:23333/api/anthropic", an).unwrap(),
            "http://127.0.0.1:23333/api/anthropic/v1"
        );
        assert_eq!(
            normalize_base("http://127.0.0.1:23333/api/anthropic/v1/messages", an).unwrap(),
            "http://127.0.0.1:23333/api/anthropic/v1"
        );
        assert_eq!(normalize_base("http://localhost:8080", oa).unwrap(), "http://localhost:8080");
    }

    #[test]
    fn base_urls_with_secrets_or_odd_schemes_are_refused() {
        let oa = Protocol::OpenaiChat;
        for bad in [
            "",
            "localhost:1234",
            "ftp://host/v1",
            "file:///etc/passwd",
            "http://user:sk-secret@host/v1",
            "http://host/v1?key=sk-secret",
            "http://host/v1#frag",
        ] {
            let err = normalize_base(bad, oa).unwrap_err();
            assert!(!err.contains("sk-secret"), "{bad}: {err}");
        }
    }

    #[test]
    fn only_agent_maestro_addresses_name_agent_maestro() {
        for base in [
            "http://localhost:23333/api/openai/v1",
            "http://127.0.0.1:23333/api/anthropic/v1",
            "http://localhost:23333/v1",
            "http://192.168.1.5:9000/api/openai/v1",
        ] {
            assert_eq!(not_answering(base), format!("Nothing is answering at {base}. Is Agent Maestro running?"));
        }
        for base in ["http://localhost:1234/v1", "https://openrouter.ai/api/v1", "http://localhost:233330/v1"] {
            assert_eq!(not_answering(base), format!("Nothing is answering at {base}. Is the server running?"));
        }
    }

    #[test]
    fn model_ids_are_checked() {
        assert_eq!(clean_model("  ").unwrap(), None);
        assert_eq!(clean_model(" gpt-5.5 ").unwrap().as_deref(), Some("gpt-5.5"));
        assert!(clean_model("two words").is_err());
        assert!(clean_model(&"x".repeat(201)).is_err());
    }

    #[test]
    fn model_lists_parse_openai_anthropic_and_openrouter_shapes() {
        let body = serde_json::json!({"data": [
            {"id": "claude-sonnet-4.6", "display_name": "Claude Sonnet 4.6"},
            {"id": "gpt-5.5", "object": "model"},
            {"id": "openai/gpt-4o-mini", "name": "OpenAI: GPT-4o mini"},
            {"id": "gpt-5.5"},
            {"id": ""}
        ]});
        let models = parse_models(&body, Some("GPT-5.5")).unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["claude-sonnet-4.6", "gpt-5.5", "openai/gpt-4o-mini"]);
        assert_eq!(models[0].label, "Claude Sonnet 4.6");
        assert_eq!(models[2].label, "OpenAI: GPT-4o mini");
        assert!(models[1].default && !models[0].default);
        assert_eq!(models[0].tier, Some(super::super::engines::Tier::Balanced));
        assert_eq!(models[2].tier, Some(super::super::engines::Tier::Fast));
        assert!(parse_models(&serde_json::json!({"error": "nope"}), None).is_none());
    }
}
