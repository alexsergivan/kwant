//! Composio's app catalogue (API v3.1): search its toolkits and start a sign-in for one, with the
//! person's own Composio API key. kwnt only calls this once the person has added Composio as a
//! connector, and only from person-only routes (`server::connections`).
//!
//! Sources: <https://docs.composio.dev/reference/api-reference/toolkits/getToolkits>,
//! <https://docs.composio.dev/reference/api-reference/tool-router/postToolRouterSession> and
//! <https://docs.composio.dev/reference/api-reference/tool-router/postToolRouterSessionBySessionIdLink>.

use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;

pub const DEFAULT_API_URL: &str = "https://backend.composio.dev";
/// Points kwnt at another Composio API address (tests).
pub const API_URL_ENV: &str = "KWNT_COMPOSIO_API_URL";
const TIMEOUT: Duration = Duration::from_secs(10);
/// Largest Composio answer kwnt reads.
const MAX_BODY: usize = 2 * 1024 * 1024;
pub const MAX_QUERY_CHARS: usize = 60;
/// How many apps one search returns.
const PAGE: usize = 20;

/// One app (Composio calls it a toolkit), as kwnt shows it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ComposioApp {
    pub slug: String,
    pub name: String,
    pub description: String,
    /// An https logo address, or none.
    pub logo: Option<String>,
    pub tools_count: Option<u64>,
    /// The app works without signing in to it.
    pub no_auth: bool,
}

/// Why a Composio call failed, in words for the person.
#[derive(Debug, Clone, PartialEq)]
pub enum ComposioError {
    /// Composio refused the API key (401/403).
    KeyRefused,
    /// Composio answered with an error; carries a short message.
    Upstream(u16, String),
    Unreachable,
}

impl ComposioError {
    pub fn message(&self) -> String {
        match self {
            Self::KeyRefused => "Composio refused the API key saved with the Composio connector. Check it under its MCP server.".into(),
            Self::Upstream(status, m) if m.is_empty() => format!("Composio answered HTTP {status}; try again later."),
            Self::Upstream(status, m) => format!("Composio answered HTTP {status}: {m}"),
            Self::Unreachable => "Composio couldn't be reached; try again later.".into(),
        }
    }
}

pub struct ComposioApi {
    base_url: String,
    client: reqwest::Client,
}

/// Whether `slug` looks like a Composio toolkit slug (`gmail`, `google_drive`, `x-twitter`).
pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 64
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// The Composio user an MCP URL acts for: its `user_id` query value.
pub fn user_id_of(mcp_url: &str) -> Option<String> {
    let url = url::Url::parse(mcp_url).ok()?;
    let id = url
        .query_pairs()
        .find(|(k, _)| k == "user_id")
        .map(|(_, v)| v.trim().to_string())?;
    (!id.is_empty() && id.len() <= 256).then_some(id)
}

fn https_or_none(v: &Value) -> Option<String> {
    v.as_str()
        .filter(|s| s.starts_with("https://") && s.len() <= 2048)
        .map(str::to_string)
}

fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_string(),
    }
}

/// Apps from a `GET /api/v3.1/toolkits` answer; entries without a usable slug are left out.
pub fn parse_apps(body: &Value) -> Vec<ComposioApp> {
    body["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|t| {
                    let slug = t["slug"].as_str().filter(|s| valid_slug(s))?.to_string();
                    let name = t["name"]
                        .as_str()
                        .map(|n| clip(n, 80))
                        .unwrap_or_else(|| slug.clone());
                    Some(ComposioApp {
                        description: clip(
                            t["meta"]["description"].as_str().unwrap_or_default(),
                            240,
                        ),
                        logo: https_or_none(&t["meta"]["logo"]),
                        tools_count: t["meta"]["tools_count"].as_u64(),
                        no_auth: t["no_auth"] == json!(true),
                        slug,
                        name,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl ComposioApi {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::builder()
                .timeout(TIMEOUT)
                // the key goes in a header: never follow a redirect to another host with it
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!("kwnt/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_default(),
        }
    }

    /// [`API_URL_ENV`], else Composio's own API.
    pub fn from_env() -> Self {
        let url = std::env::var(API_URL_ENV)
            .ok()
            .filter(|u| u.starts_with("http"));
        Self::new(url.as_deref().unwrap_or(DEFAULT_API_URL))
    }

    fn url(&self, segments: &[&str]) -> Result<reqwest::Url, ComposioError> {
        let mut url =
            reqwest::Url::parse(&self.base_url).map_err(|_| ComposioError::Unreachable)?;
        url.path_segments_mut()
            .map_err(|_| ComposioError::Unreachable)?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    async fn send(
        &self,
        req: reqwest::RequestBuilder,
        api_key: &str,
    ) -> Result<Value, ComposioError> {
        let res = req
            .header("x-api-key", api_key)
            .send()
            .await
            .map_err(|_| ComposioError::Unreachable)?;
        let status = res.status();
        if res.content_length().is_some_and(|n| n as usize > MAX_BODY) {
            return Err(ComposioError::Upstream(
                status.as_u16(),
                "the answer was too large".into(),
            ));
        }
        let body = res.bytes().await.map_err(|_| ComposioError::Unreachable)?;
        if body.len() > MAX_BODY {
            return Err(ComposioError::Upstream(
                status.as_u16(),
                "the answer was too large".into(),
            ));
        }
        let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(ComposioError::KeyRefused);
        }
        if !status.is_success() {
            let message = value["error"]["message"]
                .as_str()
                .or_else(|| value["message"].as_str())
                .unwrap_or_default();
            return Err(ComposioError::Upstream(status.as_u16(), clip(message, 200)));
        }
        Ok(value)
    }

    /// Apps whose name, slug or description match `query`.
    pub async fn search(
        &self,
        api_key: &str,
        query: &str,
    ) -> Result<Vec<ComposioApp>, ComposioError> {
        let mut url = self.url(&["api", "v3.1", "toolkits"])?;
        url.query_pairs_mut()
            .append_pair("search", query)
            .append_pair("sort_by", "usage")
            .append_pair("limit", &PAGE.to_string());
        let body = self.send(self.client.get(url), api_key).await?;
        Ok(parse_apps(&body))
    }

    /// Starts signing `user_id` in to the app `slug`: a session for the user, then a sign-in link
    /// for the app. Returns the https address the person opens.
    pub async fn connect_link(
        &self,
        api_key: &str,
        user_id: &str,
        slug: &str,
    ) -> Result<String, ComposioError> {
        let session = self
            .send(
                self.client
                    .post(self.url(&["api", "v3.1", "tool_router", "session"])?)
                    .json(&json!({"user_id": user_id})),
                api_key,
            )
            .await?;
        let session_id = session["session_id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or_else(|| ComposioError::Upstream(200, "no session_id in the answer".into()))?;
        let link = self
            .send(
                self.client
                    .post(self.url(&[
                        "api",
                        "v3.1",
                        "tool_router",
                        "session",
                        session_id,
                        "link",
                    ])?)
                    .json(&json!({"toolkit": slug})),
                api_key,
            )
            .await?;
        https_or_none(&link["redirect_url"]).ok_or_else(|| {
            ComposioError::Upstream(201, "no https redirect_url in the answer".into())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_plain_lowercase_words() {
        assert!(valid_slug("gmail") && valid_slug("google_drive") && valid_slug("x-twitter"));
        for bad in ["", "Gmail", "../x", "a/b", "a b", "a?b", &"a".repeat(65)] {
            assert!(!valid_slug(bad), "{bad}");
        }
    }

    #[test]
    fn user_id_comes_from_the_mcp_url() {
        assert_eq!(
            user_id_of("https://backend.composio.dev/v3/mcp/abc/mcp?user_id=u-1").as_deref(),
            Some("u-1")
        );
        assert_eq!(
            user_id_of("https://backend.composio.dev/v3/mcp/abc/mcp?user_id=me%40x.dev&a=1")
                .as_deref(),
            Some("me@x.dev")
        );
        assert_eq!(
            user_id_of("https://backend.composio.dev/v3/mcp/abc/mcp"),
            None
        );
        assert_eq!(
            user_id_of("https://backend.composio.dev/v3/mcp/abc/mcp?user_id="),
            None
        );
    }

    #[test]
    fn apps_keep_only_safe_fields() {
        let body = json!({"items": [
            {"slug": "gmail", "name": "Gmail", "no_auth": false,
             "meta": {"description": "Mail", "logo": "https://logos.composio.dev/gmail.png", "tools_count": 23}},
            {"slug": "hack", "name": "Hack", "meta": {"logo": "javascript:alert(1)"}},
            {"slug": "../etc", "name": "Bad"},
            {"name": "No slug"}
        ]});
        let apps = parse_apps(&body);
        assert_eq!(apps.len(), 2);
        assert_eq!(
            apps[0].logo.as_deref(),
            Some("https://logos.composio.dev/gmail.png")
        );
        assert_eq!(apps[0].tools_count, Some(23));
        assert_eq!(apps[1].logo, None, "only https logos");
        assert!(parse_apps(&Value::Null).is_empty());
    }
}
