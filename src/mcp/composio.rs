//! Composio's app catalogue (API v3.1): search its toolkits and start a sign-in for one, with the
//! person's own Composio API key. kwnt only calls this once the person has added Composio as a
//! connector, and only from person-only routes (`server::connections`).
//!
//! Sources: <https://docs.composio.dev/reference/api-reference/toolkits/getToolkits>,
//! <https://docs.composio.dev/reference/api-reference/tool-router/postToolRouterSession> and
//! <https://docs.composio.dev/reference/api-reference/tool-router/postToolRouterSessionBySessionIdLink> and
//! <https://docs.composio.dev/reference/api-reference/connected-accounts/getConnectedAccounts>.

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
/// The whole-catalogue listing: pages of this many apps, at most this many pages.
const LIST_PAGE: usize = 500;
const MAX_LIST_PAGES: usize = 20;

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

/// Where a person copies a Composio project API key (checked 2026-11-12: it sends a signed-out
/// visitor to Composio's sign-in, then back here).
pub const KEY_PAGE: &str = "https://dashboard.composio.dev/~/project/settings/api-keys";

/// Why a Composio call failed, in words for the person.
#[derive(Debug, Clone, PartialEq)]
pub enum ComposioError {
    /// Composio refused the API key (401).
    KeyRefused,
    /// The key is valid but not allowed to do this (403: a scoped project key).
    KeyNoAccess,
    /// Composio answered with an error; carries a short message.
    Upstream(u16, String),
    Unreachable,
}

impl ComposioError {
    pub fn message(&self) -> String {
        match self {
            Self::KeyRefused => "Composio didn't accept this key. Copy the API key from your Composio dashboard, \
                under Project settings › API keys. It starts with ak_."
                .into(),
            Self::KeyNoAccess => "This Composio key isn't allowed to do this. Copy a full-access API key from your \
                Composio dashboard, under Project settings › API keys."
                .into(),
            Self::Upstream(status, _) if *status >= 500 => "Composio couldn't be reached; try again in a minute.".into(),
            Self::Upstream(status, m) if m.is_empty() => format!("Composio answered HTTP {status}; try again later."),
            Self::Upstream(status, m) => format!("Composio answered HTTP {status}: {m}"),
            Self::Unreachable => "Composio couldn't be reached; try again in a minute.".into(),
        }
    }

    /// The one thing the person can do about it: `replace_key` or `try_again`.
    pub fn action(&self) -> &'static str {
        match self {
            Self::KeyRefused | Self::KeyNoAccess => "replace_key",
            _ => "try_again",
        }
    }

    /// The key is the problem (refused or not allowed).
    pub fn is_key(&self) -> bool {
        matches!(self, Self::KeyRefused | Self::KeyNoAccess)
    }
}

/// A Tool Router session: one Composio user's MCP server with Composio's search, connect and
/// execute tools, over every app in the catalogue.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub session_id: String,
    /// The session's MCP address, always https on composio.dev.
    pub mcp_url: String,
    pub user_id: String,
    /// The tools a bot gets (`COMPOSIO_SEARCH_TOOLS`, `COMPOSIO_MANAGE_CONNECTIONS`…).
    pub tools: Vec<String>,
}

/// The Composio user kwant picks for a space when nothing older names one.
pub fn space_user_id(space: &str) -> String {
    format!("kwant-{space}")
}

/// Whether `url` is an https address on composio.dev, the only place a Composio key may go.
pub fn is_composio_https(url: &str) -> bool {
    url::Url::parse(url).ok().is_some_and(|u| {
        u.scheme() == "https" && u.host_str().is_some_and(|h| h == "composio.dev" || h.ends_with(".composio.dev"))
    })
}

fn valid_session_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A session from a `POST /api/v3.1/tool_router/session` answer. The MCP address must be https on
/// composio.dev: kwant sends the key there with every bot call.
pub fn parse_session(body: &Value, user_id: &str) -> Result<Session, ComposioError> {
    let session_id = body["session_id"]
        .as_str()
        .filter(|s| valid_session_id(s))
        .ok_or_else(|| ComposioError::Upstream(201, "no session_id in the answer".into()))?;
    let mcp_url = body["mcp"]["url"]
        .as_str()
        .filter(|u| u.len() <= 2048 && is_composio_https(u))
        .ok_or_else(|| ComposioError::Upstream(201, "no https composio.dev MCP address in the answer".into()))?;
    let tools = body["tool_router_tools"]
        .as_array()
        .map(|t| t.iter().filter_map(|n| n.as_str()).filter(|n| n.len() <= 128).take(64).map(str::to_string).collect())
        .unwrap_or_default();
    Ok(Session { session_id: session_id.into(), mcp_url: mcp_url.into(), user_id: user_id.into(), tools })
}

/// The apps (toolkit slugs) with a working account of `user_id` in a `GET connected_accounts`
/// answer, checked the same way as [`has_active_account`].
pub fn active_apps(body: &Value, user_id: &str) -> std::collections::BTreeSet<String> {
    body["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|a| {
                    a["status"] == "ACTIVE"
                        && a["is_disabled"] != json!(true)
                        && a["user_id"].as_str().is_none_or(|u| u == user_id)
                })
                .filter_map(|a| a["toolkit"]["slug"].as_str().filter(|s| valid_slug(s)).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
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

/// Whether a `GET /api/v3.1/connected_accounts` answer holds a working account of `user_id` for the
/// app `slug`. Composio filters by user, app and status already; this checks again, so a wider
/// answer never reads as connected. Composio is dropping `user_id` from the answer, so a missing
/// one is fine but a different one isn't.
pub fn has_active_account(body: &Value, user_id: &str, slug: &str) -> bool {
    body["items"].as_array().is_some_and(|items| {
        items.iter().any(|a| {
            a["status"] == "ACTIVE"
                && a["toolkit"]["slug"] == slug
                && a["is_disabled"] != json!(true)
                && a["user_id"].as_str().is_none_or(|u| u == user_id)
        })
    })
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
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ComposioError::KeyRefused);
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(ComposioError::KeyNoAccess);
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
    /// Composio's whole app catalogue, most used first, paged through with `cursor` (at most
    /// `MAX_LIST_PAGES` pages of `LIST_PAGE`). Duplicate or unusable slugs are left out.
    pub async fn all_apps(&self, api_key: &str) -> Result<Vec<ComposioApp>, ComposioError> {
        let mut apps: Vec<ComposioApp> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let mut url = self.url(&["api", "v3.1", "toolkits"])?;
            {
                let mut q = url.query_pairs_mut();
                q.append_pair("sort_by", "usage").append_pair("limit", &LIST_PAGE.to_string());
                if let Some(c) = &cursor {
                    q.append_pair("cursor", c);
                }
            }
            let body = self.send(self.client.get(url), api_key).await?;
            for app in parse_apps(&body) {
                if seen.insert(app.slug.clone()) {
                    apps.push(app);
                }
            }
            cursor = body["next_cursor"].as_str().filter(|c| !c.is_empty() && c.len() <= 512).map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(apps)
    }

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

    /// Creates a Tool Router session for `user_id` over every app (no toolkit list), with
    /// connection management on, so a bot can find any app's tools and ask to connect one.
    /// This is also the key check: a refused key fails here and nothing is kept.
    pub async fn create_session(&self, api_key: &str, user_id: &str) -> Result<Session, ComposioError> {
        let body = self
            .send(
                self.client
                    .post(self.url(&["api", "v3.1", "tool_router", "session"])?)
                    .json(&json!({"user_id": user_id, "manage_connections": {"enable": true}})),
                api_key,
            )
            .await?;
        parse_session(&body, user_id)
    }

    /// The user of session `session_id` as this key's project sees it; `None` when the project
    /// has no such session (another project's key, or a deleted session).
    pub async fn session_user(&self, api_key: &str, session_id: &str) -> Result<Option<String>, ComposioError> {
        if !valid_session_id(session_id) {
            return Ok(None);
        }
        match self.send(self.client.get(self.url(&["api", "v3.1", "tool_router", "session", session_id])?), api_key).await {
            Ok(body) => Ok(body["config"]["user_id"].as_str().or_else(|| body["user_id"].as_str()).map(str::to_string)),
            Err(ComposioError::Upstream(400 | 404, _)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// One cheap authenticated call: `Ok` when Composio takes the key.
    pub async fn check_key(&self, api_key: &str) -> Result<(), ComposioError> {
        let mut url = self.url(&["api", "v3.1", "connected_accounts"])?;
        url.query_pairs_mut().append_pair("limit", "1");
        self.send(self.client.get(url), api_key).await.map(|_| ())
    }

    /// A sign-in link for the app `slug` in an existing session.
    pub async fn session_link(&self, api_key: &str, session_id: &str, slug: &str) -> Result<String, ComposioError> {
        if !valid_session_id(session_id) {
            return Err(ComposioError::Upstream(0, "the saved session id isn't valid".into()));
        }
        let link = self
            .send(
                self.client
                    .post(self.url(&["api", "v3.1", "tool_router", "session", session_id, "link"])?)
                    .json(&json!({"toolkit": slug})),
                api_key,
            )
            .await?;
        https_or_none(&link["redirect_url"])
            .ok_or_else(|| ComposioError::Upstream(201, "no https redirect_url in the answer".into()))
    }

    /// Starts signing `user_id` in to the app `slug`: a session for the user, then a sign-in link
    /// for the app. Returns the https address the person opens. (Setups made from an MCP URL.)
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
        self.session_link(api_key, session_id, slug).await
    }

    /// Every app `user_id` has a working account for, in one call.
    pub async fn connected_apps(
        &self,
        api_key: &str,
        user_id: &str,
    ) -> Result<std::collections::BTreeSet<String>, ComposioError> {
        let mut url = self.url(&["api", "v3.1", "connected_accounts"])?;
        url.query_pairs_mut()
            .append_pair("user_ids", user_id)
            .append_pair("statuses", "ACTIVE")
            .append_pair("limit", "200");
        let body = self.send(self.client.get(url), api_key).await?;
        Ok(active_apps(&body, user_id))
    }

    /// Whether `user_id` has a working (ACTIVE) connection to the app `slug` on Composio.
    pub async fn is_connected(
        &self,
        api_key: &str,
        user_id: &str,
        slug: &str,
    ) -> Result<bool, ComposioError> {
        let mut url = self.url(&["api", "v3.1", "connected_accounts"])?;
        url.query_pairs_mut()
            .append_pair("user_ids", user_id)
            .append_pair("toolkit_slugs", slug)
            .append_pair("statuses", "ACTIVE")
            .append_pair("limit", &PAGE.to_string());
        let body = self.send(self.client.get(url), api_key).await?;
        Ok(has_active_account(&body, user_id, slug))
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

    #[test]
    fn a_session_must_point_its_mcp_server_at_composio_over_https() {
        let ok = json!({"session_id": "trs_1", "mcp": {"type": "http", "url": "https://backend.composio.dev/tool_router/trs_1/mcp"},
            "tool_router_tools": ["COMPOSIO_SEARCH_TOOLS", "COMPOSIO_MANAGE_CONNECTIONS"]});
        let s = parse_session(&ok, "kwant-spc_default").unwrap();
        assert_eq!(s.mcp_url, "https://backend.composio.dev/tool_router/trs_1/mcp");
        assert_eq!(s.tools, ["COMPOSIO_SEARCH_TOOLS", "COMPOSIO_MANAGE_CONNECTIONS"]);
        for url in ["http://backend.composio.dev/x", "https://evil.example/mcp", "https://composio.dev.evil.example/mcp"] {
            let mut bad = ok.clone();
            bad["mcp"]["url"] = json!(url);
            assert!(parse_session(&bad, "u").is_err(), "{url}");
        }
        let mut bad = ok.clone();
        bad["session_id"] = json!("../x");
        assert!(parse_session(&bad, "u").is_err());
    }

    #[test]
    fn active_apps_are_the_working_accounts_of_this_user() {
        let body = json!({"items": [
            {"status": "ACTIVE", "toolkit": {"slug": "gmail"}},
            {"status": "ACTIVE", "toolkit": {"slug": "slack"}, "user_id": "someone-else"},
            {"status": "INITIATED", "toolkit": {"slug": "notion"}},
            {"status": "ACTIVE", "toolkit": {"slug": "github"}, "is_disabled": true},
            {"status": "ACTIVE", "toolkit": {"slug": "../x"}}
        ]});
        assert_eq!(active_apps(&body, "u-1").into_iter().collect::<Vec<_>>(), ["gmail"]);
        assert_eq!(space_user_id("spc_default"), "kwant-spc_default");
    }

    #[test]
    fn errors_say_what_to_do() {
        assert!(ComposioError::KeyRefused.message().contains("It starts with ak_"));
        assert_eq!(ComposioError::KeyRefused.action(), "replace_key");
        assert_eq!(ComposioError::KeyNoAccess.action(), "replace_key");
        assert_eq!(ComposioError::Unreachable.message(), "Composio couldn't be reached; try again in a minute.");
        assert_eq!(ComposioError::Upstream(500, "boom".into()).action(), "try_again");
        assert!(!ComposioError::KeyRefused.message().contains("MCP server"));
    }

    #[test]
    fn only_an_active_enabled_account_of_this_user_and_app_counts() {
        let account = |status: &str, slug: &str, extra: Value| {
            let mut a = json!({"id": "ca_1", "status": status, "toolkit": {"slug": slug}});
            for (k, v) in extra.as_object().unwrap() {
                a[k] = v.clone();
            }
            json!({"items": [a], "next_cursor": null})
        };
        assert!(has_active_account(&account("ACTIVE", "gmail", json!({})), "u-1", "gmail"));
        assert!(has_active_account(
            &account("ACTIVE", "gmail", json!({"user_id": "u-1", "is_disabled": false})),
            "u-1",
            "gmail"
        ));
        for (body, why) in [
            (account("INITIATED", "gmail", json!({})), "not finished"),
            (account("EXPIRED", "gmail", json!({})), "expired"),
            (account("ACTIVE", "slack", json!({})), "another app"),
            (account("ACTIVE", "gmail", json!({"user_id": "u-2"})), "another user"),
            (account("ACTIVE", "gmail", json!({"is_disabled": true})), "disabled"),
            (json!({"items": []}), "none"),
            (Value::Null, "no answer"),
        ] {
            assert!(!has_active_account(&body, "u-1", "gmail"), "{why}");
        }
    }
}
