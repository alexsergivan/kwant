//! The official MCP Registry (`registry.modelcontextprotocol.io`, API `v0.1`): search with a
//! short cache, fetch one entry, and turn an entry into exactly what kwnt would register for it
//! (an [`InstallPlan`]). Agents only ever see and suggest plans; the person installs them.
//!
//! A plan is built only from the registry's data, never from free-form text: a remote
//! streamable-HTTP address (https, or http on loopback), or a stdio package started with `npx`
//! (npm), `uvx` (PyPI) or `docker` (OCI), with arguments taken from the entry. Anything that
//! needs more than values for named env variables or headers isn't one-click and is refused.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// Env variables or headers, by name.
pub type Values = BTreeMap<String, String>;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub const DEFAULT_REGISTRY_URL: &str = "https://registry.modelcontextprotocol.io";
/// Points kwnt at another registry with the same API (tests, mirrors).
pub const REGISTRY_URL_ENV: &str = "KWNT_MCP_REGISTRY_URL";
const TIMEOUT: Duration = Duration::from_secs(8);
const CACHE_TTL: Duration = Duration::from_secs(600);
const CACHE_MAX: usize = 64;
/// Largest registry answer kwnt reads.
const MAX_BODY: usize = 2 * 1024 * 1024;
/// How many entries one registry page asks for.
const PAGE: usize = 30;
pub const MAX_QUERY_CHARS: usize = 60;
const MAX_INPUT_VALUE: usize = 4096;

/// A value the person types at install time: an env variable (stdio) or a header (http).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InputSpec {
    /// "env" or "header".
    pub kind: String,
    pub name: String,
    pub description: String,
    pub required: bool,
    pub secret: bool,
    /// The registry's value with `{placeholders}`, e.g. `Bearer {api_key}`: a typed value without
    /// the text before the placeholder is put in its place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

/// Exactly what installing an entry registers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstallPlan {
    /// "http" or "stdio", as in kwnt's server registry.
    pub transport: String,
    pub url: Option<String>,
    pub command: Option<String>,
    pub args: Vec<String>,
    /// Non-secret values the registry fixes; stored like any other env/header.
    pub fixed_env: BTreeMap<String, String>,
    pub fixed_headers: BTreeMap<String, String>,
    pub inputs: Vec<InputSpec>,
    /// A stdio server: a program runs on the machine kwnt runs on.
    pub runs_on_your_computer: bool,
    /// e.g. `npm @scope/pkg@1.2.3`; `None` for remote servers.
    pub package: Option<String>,
}

/// One registry entry, as kwnt shows it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RegistryEntry {
    pub name: String,
    pub title: String,
    pub description: String,
    pub version: String,
    pub repository_url: Option<String>,
    pub website_url: Option<String>,
    /// The plan, or why the entry can't be set up in one click.
    pub plan: Result<InstallPlan, String>,
}

// Raw registry JSON (only the fields kwnt uses)

#[derive(Debug, Deserialize)]
struct RawList {
    #[serde(default)]
    servers: Option<Vec<RawResponse>>,
}

#[derive(Debug, Deserialize)]
struct RawResponse {
    server: RawServer,
    #[serde(default, rename = "_meta")]
    meta: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawServer {
    name: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    repository: Option<RawRepository>,
    #[serde(default)]
    website_url: Option<String>,
    #[serde(default)]
    packages: Option<Vec<RawPackage>>,
    #[serde(default)]
    remotes: Option<Vec<RawTransport>>,
}

#[derive(Debug, Deserialize)]
struct RawRepository {
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPackage {
    registry_type: String,
    identifier: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    registry_base_url: Option<String>,
    #[serde(default)]
    runtime_hint: Option<String>,
    transport: RawTransport,
    #[serde(default)]
    runtime_arguments: Option<Vec<RawArgument>>,
    #[serde(default)]
    package_arguments: Option<Vec<RawArgument>>,
    #[serde(default)]
    environment_variables: Option<Vec<RawKeyValue>>,
}

#[derive(Debug, Deserialize)]
struct RawTransport {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    headers: Option<Vec<RawKeyValue>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawKeyValue {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    is_required: bool,
    #[serde(default)]
    is_secret: bool,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    placeholder: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawArgument {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    is_required: bool,
    #[serde(default)]
    is_secret: bool,
    #[serde(default)]
    value_hint: Option<String>,
}

// Plans

fn clip(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut out: String = flat.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn templated(v: &str) -> bool {
    v.contains('{')
}

fn plain(v: &str) -> bool {
    !v.chars().any(|c| c.is_control())
}

/// https anywhere, plain http only on this machine; no credentials or templates in the address.
fn check_remote_url(raw: &str) -> Result<String, String> {
    if templated(raw) {
        return Err("its address needs values filled in by hand".into());
    }
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| "its address is not a valid URL".to_string())?;
    let loopback = match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => return Err("its address has no host".into()),
    };
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => return Err("its address is not https".into()),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("its address contains a user name or password".into());
    }
    Ok(url.to_string())
}

fn valid_header_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c))
}

fn valid_env_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Sorts key/values into fixed values and inputs the person types.
fn key_values(
    list: &[RawKeyValue],
    kind: &str,
    fixed: &mut BTreeMap<String, String>,
    inputs: &mut Vec<InputSpec>,
) -> Result<(), String> {
    for kv in list {
        let name = kv.name.trim();
        let ok = if kind == "header" { valid_header_name(name) } else { valid_env_name(name) };
        if !ok {
            return Err(format!("it uses an invalid {kind} name"));
        }
        match kv.value.as_deref().filter(|v| !v.is_empty()) {
            Some(v) if !templated(v) && !kv.is_secret && plain(v) => {
                fixed.insert(name.to_string(), v.to_string());
            }
            value => inputs.push(InputSpec {
                kind: kind.into(),
                name: name.to_string(),
                description: clip(kv.description.as_deref().unwrap_or(""), 300),
                required: kv.is_required || value.is_some_and(templated),
                secret: kv.is_secret || value.is_some_and(templated),
                template: value.filter(|v| templated(v) && plain(v)).map(str::to_string),
                placeholder: kv.placeholder.as_deref().or(kv.default.as_deref()).map(|p| clip(p, 120)),
            }),
        }
    }
    Ok(())
}

fn remote_plan(t: &RawTransport) -> Result<InstallPlan, String> {
    if !matches!(t.kind.as_str(), "streamable-http" | "http") {
        return Err(format!("its remote uses the {} transport, which kwnt doesn't speak", clip(&t.kind, 20)));
    }
    let url = check_remote_url(t.url.as_deref().unwrap_or(""))?;
    let (mut fixed_headers, mut inputs) = (BTreeMap::new(), Vec::new());
    key_values(t.headers.as_deref().unwrap_or(&[]), "header", &mut fixed_headers, &mut inputs)?;
    Ok(InstallPlan {
        transport: "http".into(),
        url: Some(url),
        command: None,
        args: Vec::new(),
        fixed_env: BTreeMap::new(),
        fixed_headers,
        inputs,
        runs_on_your_computer: false,
        package: None,
    })
}

/// An exact version: no ranges, tags with spaces or wildcards.
fn exact_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 64
        && v.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && v.chars().all(|c| c.is_ascii_alphanumeric() || ".-+_".contains(c))
        && !v.contains(".x")
        && v != "latest"
}

fn valid_identifier(kind: &str, id: &str) -> bool {
    let chars = |extra: &str| id.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c));
    let first = id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || (kind == "npm" && c == '@'));
    first
        && id.len() <= 214
        && match kind {
            "npm" => chars("@/._-~") && id.matches('/').count() <= 1 && (id.starts_with('@') == id.contains('/')),
            "pypi" => chars("._-"),
            "oci" => chars("./_:@-"),
            _ => false,
        }
}

/// docker flags that would give the container the host: never set up in one click.
const DOCKER_HOST_ACCESS: [&str; 14] = [
    "--privileged", "-v", "--volume", "--mount", "--network", "--net", "--pid", "--ipc", "--cap-add",
    "--device", "--security-opt", "--userns", "--uts", "--volumes-from",
];

/// Arguments from the registry: fixed values only. A required argument without one isn't one-click.
fn arguments(list: &[RawArgument]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for a in list {
        let label = a.name.as_deref().or(a.value_hint.as_deref()).unwrap_or("an argument");
        let value = a.value.as_deref().or(if a.is_required { a.default.as_deref() } else { None });
        let value = match value {
            Some(v) if templated(v) || a.is_secret => {
                if a.is_required {
                    return Err(format!("it needs a value for {} typed into its command line", clip(label, 40)));
                }
                continue;
            }
            Some(v) => Some(v),
            None if a.is_required && a.kind != "named" => {
                return Err(format!("it needs a value for {}", clip(label, 40)));
            }
            None => None,
        };
        if value.is_some_and(|v| !plain(v)) {
            return Err("an argument holds control characters".into());
        }
        match (a.kind.as_str(), a.name.as_deref().map(str::trim).filter(|n| !n.is_empty()), value) {
            ("named", Some(name), v) => {
                if !plain(name) || !name.starts_with('-') {
                    return Err("it has a malformed named argument".into());
                }
                if v.is_none() && !a.is_required {
                    continue;
                }
                out.push(name.to_string());
                out.extend(v.map(str::to_string));
            }
            (_, _, Some(v)) => out.push(v.to_string()),
            _ => {}
        }
    }
    Ok(out)
}

fn package_plan(p: &RawPackage) -> Result<InstallPlan, String> {
    if p.transport.kind != "stdio" {
        return Err(format!("its package runs a {} server, not a stdio one", clip(&p.transport.kind, 20)));
    }
    let kind = p.registry_type.as_str();
    let runtime = match kind {
        "npm" => "npx",
        "pypi" => "uvx",
        "oci" => "docker",
        other => return Err(format!("its package comes from {}, which kwnt doesn't install", clip(other, 20))),
    };
    if p.runtime_hint.as_deref().is_some_and(|h| h != runtime) {
        return Err(format!("its package wants to be started with {}", clip(p.runtime_hint.as_deref().unwrap_or(""), 20)));
    }
    let official = match kind {
        "npm" => Some("https://registry.npmjs.org"),
        "pypi" => Some("https://pypi.org"),
        _ => None,
    };
    if let (Some(base), Some(official)) = (p.registry_base_url.as_deref(), official) {
        if base.trim_end_matches('/') != official {
            return Err("its package comes from a non-default package registry".into());
        }
    }
    let id = p.identifier.trim();
    if !valid_identifier(kind, id) {
        return Err("its package name is malformed".into());
    }
    let version = p.version.as_deref().map(str::trim).unwrap_or("");
    let image_tagged = kind == "oci" && (id.rsplit('/').next().is_some_and(|last| last.contains(':')) || id.contains('@'));
    if !image_tagged && !exact_version(version) {
        return Err("its package has no exact version".into());
    }

    let runtime_args = arguments(p.runtime_arguments.as_deref().unwrap_or(&[]))?;
    let package_args = arguments(p.package_arguments.as_deref().unwrap_or(&[]))?;
    let (mut fixed_env, mut inputs) = (BTreeMap::new(), Vec::new());
    key_values(p.environment_variables.as_deref().unwrap_or(&[]), "env", &mut fixed_env, &mut inputs)?;

    let (spec, mut args) = match kind {
        "npm" => (format!("{id}@{version}"), vec!["-y".to_string()]),
        "pypi" => (format!("{id}=={version}"), Vec::new()),
        _ => (if image_tagged { id.to_string() } else { format!("{id}:{version}") }, vec!["run".into(), "-i".into(), "--rm".into()]),
    };
    if kind == "oci" {
        for a in &runtime_args {
            let flag = a.split('=').next().unwrap_or("");
            if DOCKER_HOST_ACCESS.contains(&flag) || (flag.starts_with('-') && !flag.starts_with("--") && flag.len() > 2 && flag.contains('v')) {
                return Err("it asks docker for access to this computer".into());
            }
        }
        args.extend(runtime_args.into_iter().filter(|a| !["run", "-i", "--rm", "--interactive"].contains(&a.as_str())));
        // Pass the env variables through to the container
        for name in fixed_env.keys().chain(inputs.iter().map(|i| &i.name)) {
            args.push("-e".into());
            args.push(name.clone());
        }
    } else if runtime_args.iter().any(|a| !["-y", "--yes", "-q", "--quiet"].contains(&a.as_str())) {
        return Err(format!("it passes its own options to {runtime}"));
    }
    args.push(spec.clone());
    args.extend(package_args);
    Ok(InstallPlan {
        transport: "stdio".into(),
        url: None,
        command: Some(runtime.into()),
        args,
        fixed_env,
        fixed_headers: BTreeMap::new(),
        inputs,
        runs_on_your_computer: true,
        package: Some(format!("{kind} {spec}")),
    })
}

/// The first remote kwnt can use, else the first package; or why none works.
fn plan_for(s: &RawServer) -> Result<InstallPlan, String> {
    let mut why = None;
    for r in s.remotes.as_deref().unwrap_or(&[]) {
        match remote_plan(r) {
            Ok(plan) => return Ok(plan),
            Err(e) => why = why.or(Some(e)),
        }
    }
    for p in s.packages.as_deref().unwrap_or(&[]) {
        match package_plan(p) {
            Ok(plan) => return Ok(plan),
            Err(e) => why = why.or(Some(e)),
        }
    }
    Err(why.unwrap_or_else(|| "it lists no address or package".into()))
}

fn entry(r: RawResponse) -> Option<RegistryEntry> {
    let status = r.meta["io.modelcontextprotocol.registry/official"]["status"].as_str().unwrap_or("active");
    if status != "active" {
        return None;
    }
    let s = r.server;
    let plan = plan_for(&s);
    Some(RegistryEntry {
        title: clip(s.title.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or(&s.name), 100),
        description: clip(&s.description, 300),
        version: clip(&s.version, 64),
        repository_url: s.repository.and_then(|r| r.url).filter(|u| u.starts_with("https://")).map(|u| clip(&u, 300)),
        website_url: s.website_url.filter(|u| u.starts_with("https://")).map(|u| clip(&u, 300)),
        name: s.name,
        plan,
    })
}

/// Parses a `GET /v0.1/servers` answer.
pub fn parse_list(body: &[u8]) -> Result<Vec<RegistryEntry>, String> {
    let list: RawList = serde_json::from_slice(body).map_err(|_| "the registry sent an answer kwnt can't read".to_string())?;
    Ok(list.servers.unwrap_or_default().into_iter().filter_map(entry).collect())
}

/// Parses a `GET /v0.1/servers/{name}/versions/latest` answer.
pub fn parse_one(body: &[u8]) -> Result<Option<RegistryEntry>, String> {
    let one: RawResponse = serde_json::from_slice(body).map_err(|_| "the registry sent an answer kwnt can't read".to_string())?;
    Ok(entry(one))
}

/// Whether `name` looks like a registry name (`namespace/server`).
pub fn valid_registry_name(name: &str) -> bool {
    let Some((ns, server)) = name.split_once('/') else {
        return false;
    };
    (3..=200).contains(&name.len())
        && !ns.is_empty()
        && !server.is_empty()
        && ns.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
        && server.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// A kwnt server name for a registry entry: its part after the slash, cleaned up.
pub fn server_name_for(registry_name: &str) -> String {
    let base = registry_name.rsplit('/').next().unwrap_or(registry_name);
    let mut name = String::new();
    for c in base.chars() {
        let c = if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' };
        if !(c == '-' && name.ends_with('-')) {
            name.push(c);
        }
    }
    let name: String = name.trim_matches('-').chars().take(40).collect();
    let name = name.trim_matches('-').to_lowercase();
    if name.is_empty() || name == super::RESERVED_SERVER_NAME || name == super::SELF_SERVER_NAME {
        "mcp-server".into()
    } else {
        name
    }
}

impl InstallPlan {
    /// The env variables and headers to store, from the fixed values and the person's input.
    /// Unknown names and missing required values are refused.
    pub fn resolve(
        &self,
        env: &BTreeMap<String, String>,
        headers: &BTreeMap<String, String>,
    ) -> Result<(Values, Values), String> {
        let mut out_env = self.fixed_env.clone();
        let mut out_headers = self.fixed_headers.clone();
        for (kind, given) in [("env", env), ("header", headers)] {
            if let Some(unknown) = given.keys().find(|k| !self.inputs.iter().any(|i| i.kind == kind && &i.name == *k)) {
                return Err(format!("this server takes no {kind} value '{}'", clip(unknown, 60)));
            }
        }
        for input in &self.inputs {
            let given = if input.kind == "env" { env } else { headers };
            let value = given.get(&input.name).map(|v| v.trim()).filter(|v| !v.is_empty());
            let Some(value) = value else {
                if input.required {
                    return Err(format!("{} is required", input.name));
                }
                continue;
            };
            if value.len() > MAX_INPUT_VALUE || !plain(value) {
                return Err(format!("{} must be one line of at most {MAX_INPUT_VALUE} characters", input.name));
            }
            let value = match &input.template {
                Some(t) => fill_template(t, value),
                None => value.to_string(),
            };
            let target = if input.kind == "env" { &mut out_env } else { &mut out_headers };
            target.insert(input.name.clone(), value);
        }
        Ok((out_env, out_headers))
    }
}

/// `Bearer {api_key}` + `abc` → `Bearer abc`; a value that already starts with the literal part
/// (`Bearer abc`), or a template with several placeholders, is taken as typed.
pub fn fill_template(template: &str, value: &str) -> String {
    let (Some(open), Some(close)) = (template.find('{'), template.find('}')) else {
        return value.to_string();
    };
    if close < open || template[close + 1..].contains('{') {
        return value.to_string();
    }
    let prefix = &template[..open];
    if !prefix.is_empty() && value.starts_with(prefix) {
        return value.to_string();
    }
    format!("{prefix}{value}{}", &template[close + 1..])
}

// Client

/// Talks to the registry, with a small per-query cache.
pub struct McpRegistry {
    base_url: String,
    client: reqwest::Client,
    cache: Mutex<HashMap<String, (Instant, Vec<RegistryEntry>)>>,
}

impl McpRegistry {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::builder()
                .timeout(TIMEOUT)
                .user_agent(concat!("kwnt/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_default(),
            cache: Mutex::default(),
        }
    }

    /// [`REGISTRY_URL_ENV`], else the official registry.
    pub fn from_env() -> Self {
        let url = std::env::var(REGISTRY_URL_ENV).ok().filter(|u| u.starts_with("http"));
        Self::new(url.as_deref().unwrap_or(DEFAULT_REGISTRY_URL))
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    async fn get(&self, url: reqwest::Url) -> Result<Option<Vec<u8>>, String> {
        let unreachable = |_| "The MCP Registry couldn't be reached; try again later.".to_string();
        let res = self.client.get(url).send().await.map_err(unreachable)?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !res.status().is_success() {
            return Err(format!("The MCP Registry answered HTTP {}; try again later.", res.status().as_u16()));
        }
        if res.content_length().is_some_and(|n| n as usize > MAX_BODY) {
            return Err("The MCP Registry's answer was too large.".into());
        }
        let body = res.bytes().await.map_err(unreachable)?;
        if body.len() > MAX_BODY {
            return Err("The MCP Registry's answer was too large.".into());
        }
        Ok(Some(body.to_vec()))
    }

    fn url(&self, segments: &[&str]) -> Result<reqwest::Url, String> {
        let mut url = reqwest::Url::parse(&self.base_url).map_err(|_| "The MCP Registry address is invalid.".to_string())?;
        url.path_segments_mut().map_err(|_| "The MCP Registry address is invalid.".to_string())?.pop_if_empty().extend(segments);
        Ok(url)
    }

    async fn search_one(&self, query: &str) -> Result<Vec<RegistryEntry>, String> {
        let key = query.to_lowercase();
        if let Some((at, hits)) = self.cache.lock().await.get(&key) {
            if at.elapsed() < CACHE_TTL {
                return Ok(hits.clone());
            }
        }
        let mut url = self.url(&["v0.1", "servers"])?;
        url.query_pairs_mut()
            .append_pair("search", query)
            .append_pair("version", "latest")
            .append_pair("limit", &PAGE.to_string());
        let hits = match self.get(url).await? {
            Some(body) => parse_list(&body)?,
            None => Vec::new(),
        };
        let mut cache = self.cache.lock().await;
        if cache.len() >= CACHE_MAX {
            cache.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
            if cache.len() >= CACHE_MAX {
                cache.clear();
            }
        }
        cache.insert(key, (Instant::now(), hits.clone()));
        Ok(hits)
    }

    /// Entries whose name contains the query (the registry matches names only). With no hit for
    /// a several-word query, each of its first three words is tried.
    pub async fn search(&self, query: &str) -> Result<Vec<RegistryEntry>, String> {
        let query: String = query.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_QUERY_CHARS).collect();
        if query.is_empty() {
            return Err("query is required".into());
        }
        let mut hits = self.search_one(&query).await?;
        if hits.is_empty() && query.contains(' ') {
            for word in query.split(' ').filter(|w| w.len() >= 3).take(3) {
                for e in self.search_one(word).await? {
                    if !hits.iter().any(|h| h.name == e.name) {
                        hits.push(e);
                    }
                }
            }
        }
        Ok(hits)
    }

    /// The latest version of one entry; `None` when the registry doesn't know it.
    pub async fn get_latest(&self, name: &str) -> Result<Option<RegistryEntry>, String> {
        if !valid_registry_name(name) {
            return Ok(None);
        }
        let url = self.url(&["v0.1", "servers", name, "versions", "latest"])?;
        match self.get(url).await? {
            Some(body) => parse_one(&body),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn one(server: Value) -> RegistryEntry {
        parse_one(json!({"server": server, "_meta": {}}).to_string().as_bytes()).unwrap().unwrap()
    }

    #[test]
    fn remote_with_templated_header() {
        let e = one(json!({
            "name": "ai.example/maps", "description": "Maps", "version": "1.0.0",
            "remotes": [{"type": "sse", "url": "https://x.example/sse"},
                        {"type": "streamable-http", "url": "https://x.example/mcp",
                         "headers": [{"name": "Authorization", "value": "Bearer {api_key}", "isSecret": true, "description": "Your key"},
                                     {"name": "X-Client", "value": "kwnt"}]}],
        }));
        let plan = e.plan.unwrap();
        assert_eq!(plan.transport, "http");
        assert_eq!(plan.url.as_deref(), Some("https://x.example/mcp"));
        assert_eq!(plan.fixed_headers.get("X-Client").map(String::as_str), Some("kwnt"));
        assert_eq!(plan.inputs.len(), 1);
        assert!(plan.inputs[0].required && plan.inputs[0].secret);
        assert!(!plan.runs_on_your_computer);
        let none = BTreeMap::new();
        assert!(plan.resolve(&none, &none).unwrap_err().contains("Authorization is required"));
        let typed: BTreeMap<_, _> = [("Authorization".to_string(), "abc".to_string())].into();
        let (_, h) = plan.resolve(&none, &typed).unwrap();
        assert_eq!(h["Authorization"], "Bearer abc");
        let full: BTreeMap<_, _> = [("Authorization".to_string(), "Bearer xyz".to_string())].into();
        assert_eq!(plan.resolve(&none, &full).unwrap().1["Authorization"], "Bearer xyz");
        let unknown: BTreeMap<_, _> = [("Cookie".to_string(), "x".to_string())].into();
        assert!(plan.resolve(&none, &unknown).is_err());
        let newline: BTreeMap<_, _> = [("Authorization".to_string(), "a\nb".to_string())].into();
        assert!(plan.resolve(&none, &newline).is_err());
    }

    #[test]
    fn remote_addresses_must_be_safe() {
        assert!(check_remote_url("http://example.com/mcp").is_err());
        assert!(check_remote_url("http://127.0.0.1:9/mcp").is_ok());
        assert!(check_remote_url("http://localhost:9/mcp").is_ok());
        assert!(check_remote_url("https://u:p@example.com/mcp").is_err());
        assert!(check_remote_url("https://{tenant}.example.com/mcp").is_err());
        assert!(check_remote_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn npm_package_becomes_npx() {
        let e = one(json!({
            "name": "io.github.someone/weather", "description": "Weather", "version": "2.0.0",
            "packages": [{"registryType": "npm", "identifier": "@someone/weather-mcp", "version": "2.0.1",
                          "runtimeHint": "npx", "transport": {"type": "stdio"},
                          "packageArguments": [{"type": "named", "name": "--units", "value": "metric"},
                                               {"type": "positional", "value": "serve"},
                                               {"type": "named", "name": "--debug"}],
                          "environmentVariables": [{"name": "WEATHER_KEY", "isRequired": true, "isSecret": true, "description": "API key"},
                                                   {"name": "LOG_LEVEL", "value": "info"}]}],
        }));
        let plan = e.plan.unwrap();
        assert_eq!(plan.command.as_deref(), Some("npx"));
        assert_eq!(plan.args, ["-y", "@someone/weather-mcp@2.0.1", "--units", "metric", "serve"]);
        assert_eq!(plan.fixed_env["LOG_LEVEL"], "info");
        assert_eq!(plan.inputs[0].name, "WEATHER_KEY");
        assert!(plan.runs_on_your_computer);
        assert_eq!(plan.package.as_deref(), Some("npm @someone/weather-mcp@2.0.1"));
    }

    #[test]
    fn pypi_and_docker() {
        let py = one(json!({"name": "io.github.a/b", "description": "d", "version": "1",
            "packages": [{"registryType": "pypi", "identifier": "mcp-time", "version": "0.3.1", "transport": {"type": "stdio"}}]}));
        let plan = py.plan.unwrap();
        assert_eq!((plan.command.as_deref(), plan.args.as_slice()), (Some("uvx"), &["mcp-time==0.3.1".to_string()][..]));

        let oci = one(json!({"name": "io.github.a/c", "description": "d", "version": "1",
            "packages": [{"registryType": "oci", "identifier": "ghcr.io/a/c", "version": "1.2.0", "transport": {"type": "stdio"},
                          "environmentVariables": [{"name": "TOKEN", "isSecret": true}]}]}));
        let plan = oci.plan.unwrap();
        assert_eq!(plan.command.as_deref(), Some("docker"));
        assert_eq!(plan.args, ["run", "-i", "--rm", "-e", "TOKEN", "ghcr.io/a/c:1.2.0"]);

        let mounts = one(json!({"name": "io.github.a/d", "description": "d", "version": "1",
            "packages": [{"registryType": "oci", "identifier": "a/d", "version": "1", "transport": {"type": "stdio"},
                          "runtimeArguments": [{"type": "named", "name": "-v", "value": "/:/host"}]}]}));
        assert!(mounts.plan.unwrap_err().contains("access to this computer"));
    }

    #[test]
    fn not_one_click() {
        let range = one(json!({"name": "a.b/c", "description": "d", "version": "1",
            "packages": [{"registryType": "npm", "identifier": "x", "version": "^1.0.0", "transport": {"type": "stdio"}}]}));
        assert!(range.plan.unwrap_err().contains("exact version"));
        let cargo = one(json!({"name": "a.b/c", "description": "d", "version": "1",
            "packages": [{"registryType": "cargo", "identifier": "x", "version": "1.0.0", "transport": {"type": "stdio"}}]}));
        assert!(cargo.plan.is_err());
        let needs_arg = one(json!({"name": "a.b/c", "description": "d", "version": "1",
            "packages": [{"registryType": "npm", "identifier": "x", "version": "1.0.0", "transport": {"type": "stdio"},
                          "packageArguments": [{"type": "positional", "valueHint": "directory", "isRequired": true}]}]}));
        assert!(needs_arg.plan.unwrap_err().contains("directory"));
        let own_registry = one(json!({"name": "a.b/c", "description": "d", "version": "1",
            "packages": [{"registryType": "npm", "registryBaseUrl": "https://evil.example", "identifier": "x", "version": "1.0.0", "transport": {"type": "stdio"}}]}));
        assert!(own_registry.plan.is_err());
        let flag_as_name = one(json!({"name": "a.b/c", "description": "d", "version": "1",
            "packages": [{"registryType": "npm", "identifier": "--registry=x", "version": "1.0.0", "transport": {"type": "stdio"}}]}));
        assert!(flag_as_name.plan.is_err());
        let npx_opts = one(json!({"name": "a.b/c", "description": "d", "version": "1",
            "packages": [{"registryType": "npm", "identifier": "x", "version": "1.0.0", "transport": {"type": "stdio"},
                          "runtimeArguments": [{"type": "named", "name": "--registry", "value": "https://evil.example"}]}]}));
        assert!(npx_opts.plan.is_err());
    }

    #[test]
    fn inactive_entries_are_dropped_and_names_cleaned() {
        let body = json!({"servers": [
            {"server": {"name": "a.b/old", "description": "d", "version": "1"}, "_meta": {"io.modelcontextprotocol.registry/official": {"status": "deprecated"}}},
            {"server": {"name": "a.b/new", "description": "d", "version": "1"}, "_meta": {}},
        ], "metadata": {"count": 2}});
        let list = parse_list(body.to_string().as_bytes()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(server_name_for("io.github.Foo/Echo.Server_v2"), "echo-server-v2");
        assert_eq!(server_name_for("a.b/kwnt"), "mcp-server");
        assert!(valid_registry_name("io.github.x/echo"));
        assert!(!valid_registry_name("../etc/passwd"));
        assert!(!valid_registry_name("noslash"));
        assert_eq!(fill_template("token={k}&x=1", "abc"), "token=abc&x=1");
        assert_eq!(fill_template("{a}{b}", "abc"), "abc");
    }
}
