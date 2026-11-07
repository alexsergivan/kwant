//! The setup sheet's site check (design §2, step 1 to the live count): which Jira this is, whether
//! the token works and how (J3), which projects and Confluence spaces it sees, and how many open
//! tickets the pickup rule matches right now. Read-only; nothing is saved here. Every message is
//! kwant's own sentence; Jira's text only appears as the JQL error Jira returned.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::Serialize;
use serde_json::Value;
use zeroize::Zeroizing;

use super::client::{self, normalize_base, AtlassianClient, Auth, Kind, Secrets, Site};
use super::confluence::SpaceInfo;
use super::jira::ProjectInfo;
use super::jql::{self, Handover, Rule};
use crate::server::dev::{forge, git::Resolver};
use crate::server::dev::tickets::source::{valid_project, SourceError};

/// What the person typed (tokens already taken out of the request body).
#[derive(Default)]
pub struct CheckInput {
    pub jira_url: String,
    /// `cloud` or `dc` when the person pressed "Not right? Change"; else detected.
    pub kind: Option<String>,
    pub email: Option<String>,
    pub token: Option<Zeroizing<String>>,
    pub confluence_url: Option<String>,
    pub confluence_token: Option<Zeroizing<String>>,
    /// Empty: no count yet (step 1 only).
    pub projects: Vec<String>,
    pub handover: Option<String>,
    pub assignee_id: Option<String>,
    pub label: Option<String>,
    pub jql: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Me {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Sample {
    pub key: String,
    /// Untrusted text from Jira: show it as text.
    pub summary: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConfluenceCheck {
    /// `ok`, `auth_failed` (the token can't read Confluence), `unreachable`, `needs_token` (DC
    /// Confluence without its own PAT) or `off` (DC with no Confluence address).
    pub result: &'static str,
    pub spaces: Vec<SpaceInfo>,
}

/// The answer. `result`: `found`, `auth_failed`, `not_jira`, `unreachable`, `rate_limited`,
/// `no_projects` or `invalid` (the address or a field can't be used; `message` says which).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CheckOutcome {
    pub result: &'static str,
    pub message: Option<String>,
    pub kind: Option<&'static str>,
    pub jira_url: Option<String>,
    pub confluence_url: Option<String>,
    /// `basic` (classic Cloud token), `scoped` (Cloud token through api.atlassian.com) or `pat`.
    pub auth: Option<&'static str>,
    pub cloud_id: Option<String>,
    pub me: Option<Me>,
    pub projects: Vec<ProjectInfo>,
    pub confluence: Option<ConfluenceCheck>,
    /// The JQL pickup runs (the person's Advanced JQL scoped to the projects, or built).
    pub jql: Option<String>,
    /// Why that JQL can't be used (ours, or Jira's own error text).
    pub jql_error: Option<String>,
    pub count: Option<u64>,
    pub sample: Vec<Sample>,
    /// `rate_limited`: when to try again (Retry-After), for "Trying again at 10:09".
    pub retry_at: Option<DateTime<Utc>>,
    /// What this token was seen to read: `jira` (signed in, projects listed), then `confluence`
    /// (spaces listed). kwant never test-writes, so a write scope can't show here.
    pub reads: Vec<&'static str>,
    /// The sheet's token line for a scoped token: "scoped: reads Jira and Confluence" (or "reads
    /// Jira"); `None` for a classic token or a PAT, which can do whatever their user can.
    pub scope_note: Option<String>,
    /// The project whose key is in the most branch names of this space's Dev mode cards
    /// (pre-ticked, "· in your branch names"); `None` = no suggestion, nothing pre-ticked.
    pub suggested_project: Option<SuggestedProject>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SuggestedProject {
    pub key: String,
    /// How many branch names carry one of its keys.
    pub branches: usize,
}

impl CheckOutcome {
    fn new(result: &'static str) -> Self {
        CheckOutcome {
            result,
            message: None,
            kind: None,
            jira_url: None,
            confluence_url: None,
            auth: None,
            cloud_id: None,
            me: None,
            projects: Vec::new(),
            confluence: None,
            jql: None,
            jql_error: None,
            count: None,
            sample: Vec::new(),
            retry_at: None,
            reads: Vec::new(),
            scope_note: None,
            suggested_project: None,
        }
    }

    fn invalid(msg: impl Into<String>) -> Self {
        let mut o = Self::new("invalid");
        o.message = Some(msg.into());
        o
    }

    /// Sets `result`, `message` and `retry_at` from an error.
    fn failed(mut self, e: &SourceError, email: Option<&str>) -> Self {
        let (result, message) = match e {
            SourceError::Unauthorized | SourceError::Forbidden => (
                "auth_failed",
                match email {
                    Some(m) => format!("Jira didn't accept this token. Check it was copied whole and belongs to {m}. Nothing was saved."),
                    None => "Jira didn't accept this token. Check it was copied whole. Nothing was saved.".into(),
                },
            ),
            SourceError::RateLimited { retry_after } => {
                let wait = retry_after.unwrap_or(std::time::Duration::from_secs(60));
                self.retry_at = Some(Utc::now() + chrono::Duration::from_std(wait).unwrap_or(chrono::Duration::seconds(60)));
                ("rate_limited", "Jira asked us to slow down.".into())
            }
            SourceError::Unavailable(_) => ("unreachable", "No Jira answered at this address. Check the spelling, or ask your Jira admin for it.".into()),
            _ => ("not_jira", "No Jira answered at this address. Check the spelling, or ask your Jira admin for it.".into()),
        };
        self.result = result;
        self.message = Some(message);
        self
    }

    /// What the site row needs from a passing check.
    pub fn passed(&self) -> bool {
        self.result == "found"
    }
}

/// Which Jira lives at `base`: Cloud by its address, else by anonymous `serverInfo`
/// (`deploymentType` is `Cloud` or `Server`/`DataCenter`). 401/403 there still means a Jira.
async fn detect(http: &reqwest::Client, base: &str) -> Result<Kind, SourceError> {
    if url::Url::parse(base).ok().and_then(|u| u.host_str().map(client::is_cloud_host)).unwrap_or(false) {
        return Ok(Kind::Cloud);
    }
    match client::send(http, Method::GET, &format!("{base}/rest/api/2/serverInfo"), None, None).await {
        Ok(v) if v["deploymentType"].as_str() == Some("Cloud") => Ok(Kind::Cloud),
        Ok(v) if v["deploymentType"].is_string() || (v["version"].is_string() && v["baseUrl"].is_string()) => Ok(Kind::Dc),
        Ok(_) => Err(SourceError::NotFound),
        Err(SourceError::Unauthorized | SourceError::Forbidden) => Ok(Kind::Dc),
        Err(e) => Err(e),
    }
}

/// The answer to `POST jira/check {address, detect: true}` (api-decisions 8): `detected` with
/// `kind`, or `not_jira`, `unreachable` or `rate_limited` (with `retry_at`).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Detected {
    pub result: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<DateTime<Utc>>,
}

/// Which Jira lives at `address`, with no token and nothing else: only the anonymous detection,
/// through the same client as [`check`] (pinned resolver, no local addresses unless the address
/// itself is one, no redirects). Err: the address can't be used (a sentence for the person).
pub async fn probe(address: &str, resolver: Arc<dyn Resolver>) -> Result<Detected, String> {
    let base = normalize_base(address)?;
    let http = forge::client(resolver, forge::loopback_allowed(&base), client::TIMEOUT);
    Ok(match detect(&http, &base).await {
        Ok(k) => Detected { result: "detected", kind: Some(k.as_str()), retry_at: None },
        Err(e) => {
            let f = CheckOutcome::new("found").failed(&e, None);
            let result = if matches!(f.result, "unreachable" | "rate_limited") { f.result } else { "not_jira" };
            Detected { result, kind: None, retry_at: f.retry_at }
        }
    })
}

/// Cloud's site id (for scoped tokens), from the anonymous `/_edge/tenant_info`.
async fn cloud_id(http: &reqwest::Client, base: &str) -> Option<String> {
    let v: Value = client::send(http, Method::GET, &format!("{base}/_edge/tenant_info"), None, None).await.ok()?;
    let id = v["cloudId"].as_str()?;
    (id.len() <= 64 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')).then(|| id.to_string())
}

fn trimmed(s: &Option<String>) -> Option<String> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// Runs the check. `resolver` is the forge's pinned resolver (the system's in production).
pub async fn check(mut input: CheckInput, resolver: Arc<dyn Resolver>) -> CheckOutcome {
    let base = match normalize_base(&input.jira_url) {
        Ok(b) => b,
        Err(m) => return CheckOutcome::invalid(m),
    };
    let http = forge::client(resolver.clone(), forge::loopback_allowed(&base), client::TIMEOUT);
    let mut out = CheckOutcome::new("found");
    out.jira_url = Some(base.clone());
    let kind = match input.kind.as_deref().and_then(Kind::parse) {
        Some(k) => k,
        None => match detect(&http, &base).await {
            Ok(k) => k,
            Err(e) => return out.failed(&e, None),
        },
    };
    out.kind = Some(kind.as_str());
    let email = trimmed(&input.email);
    if kind == Kind::Cloud && email.as_deref().is_none_or(|m| !m.contains('@') || m.len() > 254) {
        return CheckOutcome { kind: out.kind, jira_url: out.jira_url, ..CheckOutcome::invalid("type the Atlassian email the token belongs to") };
    }
    let Some(token) = input.token.take().filter(|t| !t.trim().is_empty()) else {
        return CheckOutcome { kind: out.kind, jira_url: out.jira_url, ..CheckOutcome::invalid("paste the token") };
    };
    let token = Zeroizing::new(token.trim().to_string());
    let confluence_url = match (kind, trimmed(&input.confluence_url)) {
        (Kind::Dc, Some(u)) => match normalize_base(&u) {
            Ok(u) => Some(u),
            Err(m) => return CheckOutcome { kind: out.kind, jira_url: out.jira_url, ..CheckOutcome::invalid(format!("Confluence address: {m}")) },
        },
        _ => None,
    };
    out.confluence_url = confluence_url.clone();
    let mut site = Site {
        kind,
        jira_url: base.clone(),
        confluence_url: confluence_url.clone(),
        email: if kind == Kind::Cloud { email.clone() } else { None },
        auth: if kind == Kind::Cloud { Auth::Basic } else { Auth::Pat },
        cloud_id: None,
        projects: Vec::new(),
        spaces: Vec::new(),
    };
    let secrets = Secrets { jira: token.clone(), confluence: None };
    let mut c = match AtlassianClient::new(site.clone(), &secrets, resolver.clone()) {
        Ok(c) => c,
        Err(e) => return out.failed(&e, email.as_deref()),
    };
    // J3: a classic Cloud token signs in at the site; a scoped one only through the gateway.
    let me = match c.myself().await {
        Ok(me) => me,
        Err(SourceError::Unauthorized | SourceError::Forbidden) if kind == Kind::Cloud => {
            let Some(id) = cloud_id(&http, &base).await else {
                return out.failed(&SourceError::Unauthorized, email.as_deref());
            };
            site.auth = Auth::Scoped;
            site.cloud_id = Some(id);
            c = match AtlassianClient::new(site.clone(), &secrets, resolver.clone()) {
                Ok(c) => c,
                Err(e) => return out.failed(&e, email.as_deref()),
            };
            match c.myself().await {
                Ok(me) => me,
                Err(e) => return out.failed(&e, email.as_deref()),
            }
        }
        Err(e) => return out.failed(&e, email.as_deref()),
    };
    out.auth = Some(site.auth.as_str());
    out.cloud_id = site.cloud_id.clone();
    out.me = Some(Me { id: me.id, display_name: me.display_name });

    out.projects = match c.projects().await {
        Ok(p) => p,
        Err(SourceError::Forbidden | SourceError::NotFound) => Vec::new(),
        Err(e) => return out.failed(&e, email.as_deref()),
    };
    if out.projects.is_empty() {
        out.result = "no_projects";
        out.message = Some(match &email {
            Some(m) => format!("This token can't see any projects. Ask a Jira admin to give {m} access to the project, then tap Check now."),
            None => "This token can't see any projects. Ask a Jira admin for access to the project, then tap Check now.".into(),
        });
        return out;
    }

    out.reads.push("jira");
    let conf = confluence(&site, &secrets, input.confluence_token.take(), &resolver).await;
    if conf.result == "ok" {
        out.reads.push("confluence");
    }
    out.confluence = Some(conf);
    if site.auth == Auth::Scoped {
        out.scope_note = Some(scope_note(&out.reads));
    }

    if !input.projects.is_empty() {
        rule_count(&mut out, &c, &input).await;
    }
    out
}

/// "scoped: reads Jira and Confluence".
pub fn scope_note(reads: &[&str]) -> String {
    let names: Vec<&str> = reads.iter().map(|r| if *r == "confluence" { "Confluence" } else { "Jira" }).collect();
    format!("scoped: reads {}", names.join(" and "))
}

/// The project to pre-tick: the one (of `visible`) whose key is in the most `branches`; ties go to
/// the earlier one in `visible`. `None` when no branch carries a visible project's key.
pub fn suggest_project(visible: &[ProjectInfo], branches: &[String]) -> Option<SuggestedProject> {
    let mut best: Option<SuggestedProject> = None;
    for p in visible {
        let n = branches
            .iter()
            .filter(|b| crate::server::dev::tickets::store::keys_in(Some(b), "").iter().any(|k| crate::server::dev::tickets::source::project_of(k) == Some(p.key.as_str())))
            .count();
        if n > 0 && best.as_ref().is_none_or(|b| n > b.branches) {
            best = Some(SuggestedProject { key: p.key.clone(), branches: n });
        }
    }
    best
}

/// Confluence's answer for the sheet's step 3.
async fn confluence(site: &Site, secrets: &Secrets, conf_token: Option<Zeroizing<String>>, resolver: &Arc<dyn Resolver>) -> ConfluenceCheck {
    let mut s = site.clone();
    let secrets = match site.kind {
        Kind::Cloud => Secrets { jira: secrets.jira.clone(), confluence: None },
        Kind::Dc => {
            if site.confluence_url.is_none() {
                return ConfluenceCheck { result: "off", spaces: Vec::new() };
            }
            match conf_token.filter(|t| !t.trim().is_empty()) {
                Some(t) => Secrets { jira: secrets.jira.clone(), confluence: Some(Zeroizing::new(t.trim().to_string())) },
                None => return ConfluenceCheck { result: "needs_token", spaces: Vec::new() },
            }
        }
    };
    s.spaces = Vec::new();
    let Ok(c) = AtlassianClient::new(s, &secrets, resolver.clone()) else {
        return ConfluenceCheck { result: "auth_failed", spaces: Vec::new() };
    };
    match c.spaces().await {
        Ok(spaces) => ConfluenceCheck { result: "ok", spaces },
        Err(SourceError::Unauthorized | SourceError::Forbidden | SourceError::NotFound) => ConfluenceCheck { result: "auth_failed", spaces: Vec::new() },
        Err(_) => ConfluenceCheck { result: "unreachable", spaces: Vec::new() },
    }
}

/// The rule from the sheet's choices.
pub fn rule_of(projects: &[String], handover: Option<&str>, assignee: Option<&str>, label: Option<&str>) -> Result<Rule, String> {
    let handover = match handover {
        None => Handover::Label,
        Some("either") => return Err("one rule only: pick the label or the assignee. For either, edit the search under Advanced".into()),
        Some(h) => Handover::parse(h).ok_or("hand over by label or assignee")?,
    };
    let assignee = assignee.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let label = match label.map(str::trim).filter(|s| !s.is_empty()) {
        Some(l) => Some(l.to_string()),
        None if handover == Handover::Label => Some("kwant".to_string()),
        None => None,
    };
    let rule = Rule { projects: projects.to_vec(), handover, assignee, label };
    rule.validate()?;
    Ok(rule)
}

/// The JQL pickup runs for these choices: Advanced JQL scoped to the projects, else built.
pub fn pickup_jql(projects: &[String], handover: Option<&str>, assignee: Option<&str>, label: Option<&str>, custom: Option<&str>) -> Result<String, String> {
    match custom.map(str::trim).filter(|s| !s.is_empty()) {
        Some(custom) => {
            if projects.is_empty() || projects.len() > jql::MAX_PROJECTS || projects.iter().any(|p| !valid_project(p)) {
                return Err("pick between 1 and 20 Jira projects".into());
            }
            jql::scoped_custom(projects, custom)
        }
        None => Ok(rule_of(projects, handover, assignee, label)?.jql()),
    }
}

async fn rule_count(out: &mut CheckOutcome, c: &AtlassianClient, input: &CheckInput) {
    if let Some(p) = input.projects.iter().find(|p| !out.projects.iter().any(|v| &v.key == *p)) {
        out.jql_error = Some(format!("this token can't see the project {p}"));
        return;
    }
    let jql = match pickup_jql(&input.projects, input.handover.as_deref(), input.assignee_id.as_deref(), input.label.as_deref(), input.jql.as_deref()) {
        Ok(j) => j,
        Err(m) => {
            out.jql_error = Some(m);
            return;
        }
    };
    out.jql = Some(jql.clone());
    match c.count(&jql).await {
        Ok(n) => out.count = Some(n),
        Err(SourceError::Invalid(m)) => {
            out.jql_error = Some(crate::server::dev::tickets::source::cut(&m, 300).0);
            return;
        }
        Err(e) => {
            *out = out.clone().failed(&e, None);
            return;
        }
    }
    if out.count.unwrap_or(0) > 0 {
        if let Ok(hits) = c.search(&jql, &[], 3).await {
            out.sample = hits.into_iter().map(|(_, i)| Sample { key: i.key, summary: crate::server::dev::tickets::source::cut(&i.summary, 200).0, url: i.url }).collect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_defaults_to_the_kwant_label() {
        let r = rule_of(&["TRAIL".into()], None, None, None).unwrap();
        assert_eq!(r.jql(), "project in (\"TRAIL\") AND labels = \"kwant\" AND statusCategory != Done ORDER BY created ASC");
        assert!(rule_of(&["TRAIL".into()], Some("assignee"), None, None).is_err());
        assert!(rule_of(&[], None, None, None).is_err());
        assert!(rule_of(&["TRAIL".into()], Some("someone"), None, None).is_err());
        assert!(rule_of(&["TRAIL".into()], Some("either"), Some("u"), Some("kwant")).unwrap_err().contains("Advanced"));
        let r = rule_of(&["TRAIL".into()], Some("assignee"), Some("712020:x"), Some("kwant")).unwrap();
        assert_eq!(r.jql(), "project in (\"TRAIL\") AND assignee = \"712020:x\" AND statusCategory != Done ORDER BY created ASC");
    }

    #[test]
    fn the_suggested_project_is_the_one_in_most_branch_names() {
        let p = |k: &str| ProjectInfo { key: k.into(), name: k.into() };
        let visible = vec![p("SHOP"), p("TRAIL")];
        let branches: Vec<String> = ["TRAIL-12-export", "feature/TRAIL-14", "SHOP-3-x", "main", "UTF-8-fix"].iter().map(|s| s.to_string()).collect();
        assert_eq!(suggest_project(&visible, &branches), Some(SuggestedProject { key: "TRAIL".into(), branches: 2 }));
        assert_eq!(suggest_project(&visible, &["main".into(), "HR-1".into()]), None, "no visible key: nothing pre-ticked");
        assert_eq!(scope_note(&["jira", "confluence"]), "scoped: reads Jira and Confluence");
        assert_eq!(scope_note(&["jira"]), "scoped: reads Jira");
    }

    #[test]
    fn advanced_jql_stays_in_the_projects() {
        let j = pickup_jql(&["TRAIL".into()], None, None, None, Some("labels = kwant OR project = HR")).unwrap();
        assert!(j.starts_with("project in (\"TRAIL\") AND ("), "{j}");
        assert!(pickup_jql(&[], None, None, None, Some("labels = kwant")).is_err());
    }
}
