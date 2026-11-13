//! The public MCP directory for people ("Find a tool", mcp-finder): search kwnt's local copy of
//! the MCP Registry and add an entry in one step. Agents have their own `mcp_search` and
//! `mcp_suggest`; both routes here refuse agents' tokens.
//! Contract: docs/mcp-api.md ("Public directory").

use super::mcp_routes::view;
use super::mcp_suggest::{create_and_test, refuse_agent_tokens, InstallFailed, Installed};
use super::spaces::{ensure_active, Scope};
use super::AppState;
use crate::db::models::McpServer;
use crate::mcp::directory::{DirEntry, Source};
use crate::mcp::registry::{valid_registry_name, MAX_QUERY_CHARS};
use crate::mcp::SECRET_MASK;
use axum::{
    extract::{rejection::JsonRejection, Query, State},
    http::{header::HeaderName, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Instant;

const REFUSAL: &str = "only the person can search the directory and add tools from it";
/// Most env values plus headers one install takes.
const MAX_VALUES: usize = 50;

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

#[derive(Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub q: String,
}

/// An address for comparing: lower case, no trailing slash.
fn same_address(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// `https://mcp.stripe.com/` → `https://mcp.stripe.com`: the address as people write it.
fn shown_url(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(u) if u.path() == "/" && u.query().is_none() && u.fragment().is_none() => url.trim_end_matches('/').to_string(),
        _ => url.to_string(),
    }
}

/// `npx -y @scope/pkg@1.0.0`: the command line installing it would run (a preview only).
fn command_line(command: &str, args: &[String]) -> String {
    std::iter::once(command.to_string())
        .chain(args.iter().map(|a| if a.is_empty() || a.chars().any(char::is_whitespace) { format!("'{}'", a.replace('\'', "'\\''")) } else { a.clone() }))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a server from this entry already exists in the space: one installed from it through
/// a suggestion, or one with the same address (remote) or the same command line (stdio).
fn already_added(e: &DirEntry, servers: &[McpServer], suggested: &[String]) -> bool {
    if suggested.iter().any(|n| n == &e.entry.name) {
        return true;
    }
    let Ok(plan) = &e.entry.plan else { return false };
    servers.iter().any(|s| match (&plan.url, &s.url) {
        (Some(a), Some(b)) if plan.transport == "http" => same_address(a) == same_address(b),
        _ => plan.transport == "stdio" && s.transport == "stdio" && plan.command == s.command && plan.args == s.args,
    })
}

/// One row as Find a tool reads it (`DirectoryEntry` in frontend/src/connections.ts).
fn entry_json(e: &DirEntry, added: bool) -> Value {
    let Ok(plan) = &e.entry.plan else { return Value::Null };
    let remote = plan.transport == "http";
    json!({
        "registry_name": e.entry.name,
        "version": e.entry.version,
        "title": e.title,
        "description": e.entry.description,
        "publisher": e.publisher,
        "kind": if remote { "remote" } else { "stdio" },
        "url": if remote { plan.url.as_deref().map(shown_url) } else { None },
        "transport": if remote { Some("http") } else { None },
        "command": if remote { None } else { plan.command.as_deref().map(|c| command_line(c, &plan.args)) },
        "inputs": plan.inputs.iter().map(|i| json!({
            "name": i.name,
            "description": i.description,
            "required": i.required,
            "secret": i.secret,
        })).collect::<Vec<_>>(),
        "already_added": added,
    })
}

/// `GET /api/mcp/registry/search?q=&space=`: up to 12 one-click entries for what the person typed,
/// from kwnt's copy of the registry (from the registry's own search until the first sync is done).
/// 200 `{entries, unavailable, source}`; `unavailable: true` (and no entries) when there's no copy
/// yet and the registry can't be reached. Fewer than 2 letters answer no entries.
pub async fn search(State(state): State<AppState>, headers: HeaderMap, Scope(space): Scope, Query(q): Query<SearchQuery>) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let started = Instant::now();
    let query: String = q.q.chars().filter(|c| !c.is_control()).take(MAX_QUERY_CHARS).collect();
    let answer = state.mcp_directory.search(&state.mcp_registry, &query).await;
    let (servers, suggested) = if answer.entries.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        let servers = state.db.list_mcp_servers(&space).await.unwrap_or_default();
        let suggested = state.db.installed_mcp_suggestions(&space).await.unwrap_or_default().into_iter().map(|(n, _)| n).collect();
        (servers, suggested)
    };
    let entries: Vec<Value> = answer.entries.iter().map(|e| entry_json(e, already_added(e, &servers, &suggested))).collect();
    let source = match answer.source {
        Source::Copy => "copy",
        Source::Live => "live",
    };
    let body = json!({ "entries": entries, "unavailable": answer.unavailable, "source": source });
    let timing = format!("directory;dur={:.1}", started.elapsed().as_secs_f64() * 1000.0);
    let mut resp = Json(body).into_response();
    if let Ok(v) = HeaderValue::from_str(&timing) {
        resp.headers_mut().insert(HeaderName::from_static("server-timing"), v);
    }
    resp
}

#[derive(Deserialize)]
pub struct InstallBody {
    pub registry_name: String,
    /// The Name on the form; used when it is a valid server name (else the entry's own name).
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

/// Every string in `v` with each secret replaced by the mask.
fn scrub(v: Value, secrets: &[String]) -> Value {
    match v {
        Value::String(s) => Value::String(secrets.iter().fold(s, |s, secret| s.replace(secret.as_str(), SECRET_MASK))),
        Value::Array(a) => Value::Array(a.into_iter().map(|x| scrub(x, secrets)).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, x)| (k, scrub(x, secrets))).collect()),
        other => other,
    }
}

/// `POST /api/mcp/registry/install?space= {registry_name, name?, env?, headers?}`: the person adds an
/// entry from the directory. Its latest version is fetched again and planned from the registry's
/// data only; `env` and `headers` fill its named inputs. The server is added like one the person
/// typed in (for every bot), tested, and kept if it starts or only needs sign-in
/// (`oauth_required: true`). It is named `name` when that is a valid server name, else after the
/// entry, with a suffix on a clash. The values typed are never echoed back or logged.
pub async fn install(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
    body: Result<Json<InstallBody>, JsonRejection>,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    // A JSON error can quote what was sent, so it isn't passed on.
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, "send {registry_name, name, env, headers} as JSON, with text values");
    };
    let secrets: Vec<String> = body.env.values().chain(body.headers.values()).map(|v| v.trim().to_string()).filter(|v| v.len() >= 6).collect();
    let resp = install_inner(&state, &space, body).await;
    let (status, value) = resp;
    (status, Json(scrub(value, &secrets))).into_response()
}

async fn install_inner(state: &AppState, space: &crate::db::spaces::SpaceId, body: InstallBody) -> (StatusCode, Value) {
    let err = |status: StatusCode, msg: &str| (status, json!({ "error": msg }));
    if let Err(r) = ensure_active(state, space).await {
        return (r.status(), json!({ "error": "this space can't be changed" }));
    }
    let name = body.registry_name.trim();
    if !valid_registry_name(name) {
        return err(StatusCode::BAD_REQUEST, "registry_name must be a registry name like com.example/server");
    }
    if body.env.len() + body.headers.len() > MAX_VALUES {
        return err(StatusCode::BAD_REQUEST, "too many values");
    }
    let entry = match state.mcp_registry.get_latest(name).await {
        Ok(Some(e)) => e,
        Ok(None) => return err(StatusCode::NOT_FOUND, "This server isn't in the MCP Registry any more."),
        Err(e) => return (StatusCode::BAD_GATEWAY, json!({ "error": e, "unavailable": true })),
    };
    let plan = match &entry.plan {
        Ok(p) => p,
        Err(why) => return err(StatusCode::UNPROCESSABLE_ENTITY, &format!("This server can't be added in one step: {why}.")),
    };
    let (env, header_values) = match plan.resolve(&body.env, &body.headers) {
        Ok(v) => v,
        Err(e) => return err(StatusCode::BAD_REQUEST, &e),
    };
    match create_and_test(state, space, name, body.name.as_deref(), plan, env, header_values, None).await {
        Ok(Installed { server, test, oauth_required }) => {
            tracing::info!(registry_name = %name, server_id = %server.id, oauth_required, "MCP server added from the directory");
            (
                StatusCode::OK,
                json!({
                    "registry_name": name,
                    "version": entry.version,
                    "server": view(state, &server).await,
                    "test": test,
                    "oauth_required": oauth_required,
                }),
            )
        }
        Err(InstallFailed { status, error, test }) => {
            tracing::info!(registry_name = %name, status = status.as_u16(), "adding an MCP server from the directory failed");
            let mut v = json!({ "error": error });
            if let Some(t) = test {
                v["test"] = t;
            }
            (status, v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_command_lines_and_scrubbing() {
        assert_eq!(shown_url("https://mcp.stripe.com/"), "https://mcp.stripe.com");
        assert_eq!(shown_url("https://mcp.notion.com/mcp"), "https://mcp.notion.com/mcp");
        assert_eq!(shown_url("https://x.example/?a=1"), "https://x.example/?a=1");
        assert_eq!(same_address("https://MCP.stripe.com/"), same_address("https://mcp.stripe.com"));
        assert_eq!(command_line("npx", &["-y".into(), "a b".into(), "x".into()]), "npx -y 'a b' x");
        let v = scrub(json!({"error": "bad key sk_live_123", "n": [1, "Bearer sk_live_123"]}), &["sk_live_123".into()]);
        assert_eq!(v, json!({"error": format!("bad key {SECRET_MASK}"), "n": [1, format!("Bearer {SECRET_MASK}")]}));
    }
}
