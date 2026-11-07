//! Jira reads (search, issue, comments, remote links, projects, users) and the one write (a
//! comment), for Cloud (v3, ADF) and Data Center (v2, wiki markup; text read from `renderedFields`).

use reqwest::Method;
use serde_json::{json, Value};

use super::client::{Api, AtlassianClient, Kind};
use super::text::{adf_to_text, html_to_text, text_to_adf, text_to_wiki};
use crate::server::dev::tickets::source::*;

/// The fields pickup reads per ticket (search) and `issue` reads in full.
pub const SEARCH_FIELDS: &[&str] = &["summary", "status", "priority", "assignee", "reporter", "labels", "created", "updated", "project", "issuetype"];
const ISSUE_FIELDS: &str = "summary,description,status,priority,assignee,reporter,labels,created,updated,project,issuetype,parent,issuelinks,attachment";
/// Most tickets one poll reads from a search (5 pages of 100).
pub const MAX_SEARCH: usize = 500;
/// Cloud's `reconcileIssues` takes at most 50 ids.
pub const MAX_RECONCILE: usize = 50;

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// Jira's times: `2026-11-07T09:58:00.000+0000` (no colon in the offset), or RFC 3339.
pub fn jira_date(v: &Value) -> Option<chrono::DateTime<chrono::Utc>> {
    let t = v.as_str()?;
    chrono::DateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S%.f%z")
        .or_else(|_| chrono::DateTime::parse_from_rfc3339(t))
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
}

/// Cloud `accountId`; DC `key`, else `name`.
pub fn person(v: &Value) -> Option<Person> {
    if !v.is_object() {
        return None;
    }
    let id = v["accountId"].as_str().or(v["key"].as_str()).or(v["name"].as_str())?;
    Some(Person { id: id.to_string(), display_name: s(&v["displayName"]) })
}

fn linked(v: &Value) -> LinkedIssue {
    LinkedIssue {
        key: s(&v["key"]),
        summary: s(&v["fields"]["summary"]),
        status: s(&v["fields"]["status"]["name"]),
        status_category: StatusCategory::from_key(v["fields"]["status"]["statusCategory"]["key"].as_str().unwrap_or("new")),
    }
}

/// One issue from a search hit or `GET /issue`. `rendered` is DC's `renderedFields` (HTML).
pub fn parse_issue(v: &Value, browse_base: &str, kind: Kind, epic_field: Option<&str>) -> Result<Issue, SourceError> {
    let key = s(&v["key"]);
    if !valid_key(&key) {
        return Err(SourceError::Invalid("Jira's answer had no issue key".into()));
    }
    let f = &v["fields"];
    let description = match kind {
        Kind::Cloud => adf_to_text(&f["description"]),
        Kind::Dc => match v["renderedFields"]["description"].as_str() {
            Some(html) => html_to_text(html),
            None => s(&f["description"]),
        },
    };
    let priority = f["priority"]["name"].as_str().map(str::to_string);
    let parent = if f["parent"].is_object() {
        Some(linked(&f["parent"]))
    } else {
        epic_field.and_then(|id| f[id].as_str()).filter(|k| valid_key(k)).map(|k| LinkedIssue { key: k.to_string(), ..Default::default() })
    };
    let links = f["issuelinks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| {
            if l["outwardIssue"].is_object() {
                Some(IssueLink { relation: s(&l["type"]["outward"]), issue: linked(&l["outwardIssue"]) })
            } else if l["inwardIssue"].is_object() {
                Some(IssueLink { relation: s(&l["type"]["inward"]), issue: linked(&l["inwardIssue"]) })
            } else {
                None
            }
        })
        .collect();
    Ok(Issue {
        url: format!("{browse_base}/browse/{key}"),
        project_key: f["project"]["key"].as_str().map(str::to_string).or_else(|| project_of(&key).map(str::to_string)).unwrap_or_default(),
        issue_type: s(&f["issuetype"]["name"]),
        summary: s(&f["summary"]),
        description,
        status: s(&f["status"]["name"]),
        status_category: StatusCategory::from_key(f["status"]["statusCategory"]["key"].as_str().unwrap_or("new")),
        priority_rank: priority.as_deref().and_then(|p| super::priority_rank(p, f["priority"]["id"].as_str())),
        priority,
        reporter: person(&f["reporter"]),
        assignee: person(&f["assignee"]),
        labels: f["labels"].as_array().into_iter().flatten().filter_map(|l| l.as_str().map(str::to_string)).collect(),
        parent,
        links,
        remote_links: Vec::new(),
        attachments: f["attachment"].as_array().into_iter().flatten().filter_map(|a| a["filename"].as_str().map(str::to_string)).collect(),
        created: jira_date(&f["created"]),
        updated: jira_date(&f["updated"]),
        key,
    })
}

fn parse_comment(v: &Value, kind: Kind) -> Comment {
    let text = match kind {
        Kind::Cloud => adf_to_text(&v["body"]),
        Kind::Dc => v["renderedBody"].as_str().map(html_to_text).unwrap_or_else(|| s(&v["body"])),
    };
    let (body, truncated) = cut(&text, COMMENT_CHARS);
    Comment { id: s(&v["id"]), author: person(&v["author"]), created: jira_date(&v["created"]), updated: jira_date(&v["updated"]), body, truncated }
}

fn enc(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

/// A user for the assignee picker: `id` is what issues report ([`person`]); `jql` is what JQL
/// takes (Cloud: both the accountId; DC: the username, while issues report the user key).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UserInfo {
    pub id: String,
    pub jql: String,
    pub display_name: String,
}

pub fn user_info(v: &Value) -> Option<UserInfo> {
    let p = person(v)?;
    let jql = v["accountId"].as_str().or(v["name"].as_str()).unwrap_or(&p.id).to_string();
    Some(UserInfo { id: p.id, jql, display_name: p.display_name })
}

/// A project the token can see.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ProjectInfo {
    pub key: String,
    pub name: String,
}

/// A DC username fit for `[~username]`: no wiki brackets, braces, pipes, tildes or line breaks.
fn wiki_username(name: &str) -> Option<&str> {
    let n = name.trim();
    (!n.is_empty() && n.len() <= 255 && !n.chars().any(|c| matches!(c, '[' | ']' | '{' | '}' | '|' | '~' | '!') || c.is_control())).then_some(n)
}

/// The ADF body of an Ask on Jira comment: `Hi `, a `mention` node, `, ` then `rest`'s first
/// paragraph, and the rest of `rest` as is.
pub fn ask_adf(to: Option<&Mention>, rest: &str) -> Value {
    let mut doc = text_to_adf(rest.trim());
    let mut lead = match to {
        Some(m) => vec![
            json!({"type": "text", "text": "Hi "}),
            json!({"type": "mention", "attrs": {"id": m.mention_id, "text": format!("@{}", m.display_name.trim())}}),
            json!({"type": "text", "text": ", "}),
        ],
        None => vec![json!({"type": "text", "text": "Hi, "})],
    };
    let content = doc["content"].as_array_mut().expect("text_to_adf makes a doc");
    match content.first_mut() {
        Some(p) if p["type"] == "paragraph" => {
            let inner = p["content"].as_array_mut().expect("a paragraph has content");
            lead.append(inner);
            *inner = lead;
        }
        _ => content.insert(0, json!({"type": "paragraph", "content": lead})),
    }
    doc
}

/// The wiki body of an Ask on Jira comment on DC: `Hi [~username], ` then `rest` escaped. A
/// username that can't sit in `[~…]` falls back to the display name as plain (escaped) text.
pub fn ask_wiki(to: Option<&Mention>, rest: &str) -> String {
    let hi = match to {
        Some(m) => match wiki_username(&m.mention_id) {
            Some(u) => format!("Hi [~{u}], "),
            None => format!("Hi {}, ", text_to_wiki(m.display_name.trim())),
        },
        None => "Hi, ".to_string(),
    };
    format!("{hi}{}", text_to_wiki(rest.trim()))
}

/// The display name on the newest changelog entry that changed one of `fields` (J14: "Dana took
/// it back in Jira"). Untrusted text. Histories are compared by `created`, whatever their order.
pub fn last_changed_by(changelog: &Value, fields: &[&str]) -> Option<String> {
    changelog["histories"]
        .as_array()?
        .iter()
        .filter(|h| h["items"].as_array().is_some_and(|items| items.iter().any(|i| i["field"].as_str().is_some_and(|f| fields.iter().any(|x| x.eq_ignore_ascii_case(f))))))
        .filter_map(|h| Some((jira_date(&h["created"])?, h["author"]["displayName"].as_str()?.trim().to_string())))
        .filter(|(_, n)| !n.is_empty())
        .max_by_key(|(t, _)| *t)
        .map(|(_, n)| cut(&n, 120).0)
}

impl AtlassianClient {
    fn in_scope(&self, key: &str) -> Result<(), SourceError> {
        let p = project_of(key).ok_or_else(|| SourceError::Invalid("that isn't a Jira issue key (like TRAIL-101)".into()))?;
        if self.site.projects.iter().any(|x| x == p) {
            Ok(())
        } else {
            Err(SourceError::OutOfScope)
        }
    }

    /// The signed-in user (`/myself`).
    pub async fn myself(&self) -> Result<Person, SourceError> {
        let v = self.call(Api::Jira, Method::GET, &format!("{}/myself", self.jira_api()), None).await?;
        person(&v).ok_or_else(|| SourceError::Invalid("Jira didn't say who the token belongs to".into()))
    }

    /// Projects the token can browse (first 200).
    pub async fn projects(&self) -> Result<Vec<ProjectInfo>, SourceError> {
        let to = |v: &Value| ProjectInfo { key: s(&v["key"]), name: s(&v["name"]) };
        let list: Vec<ProjectInfo> = match self.site.kind {
            Kind::Cloud => {
                let mut out = Vec::new();
                let mut start = 0;
                loop {
                    let v = self.call(Api::Jira, Method::GET, &format!("{}/project/search?maxResults=100&startAt={start}&orderBy=key", self.jira_api()), None).await?;
                    let page: Vec<ProjectInfo> = v["values"].as_array().into_iter().flatten().map(to).collect();
                    let n = page.len();
                    out.extend(page);
                    if v["isLast"].as_bool().unwrap_or(true) || n == 0 || out.len() >= 200 {
                        break;
                    }
                    start += n;
                }
                out
            }
            Kind::Dc => self.call(Api::Jira, Method::GET, &format!("{}/project", self.jira_api()), None).await?.as_array().into_iter().flatten().map(to).collect(),
        };
        Ok(list.into_iter().filter(|p| valid_project(&p.key)).take(200).collect())
    }

    /// Jira users who can be assigned in `projects`, matching `q` (for "Which Jira user stands for
    /// your bots?"). At most 20.
    pub async fn assignable_users(&self, projects: &[String], q: &str) -> Result<Vec<UserInfo>, SourceError> {
        let keys = projects.iter().filter(|p| valid_project(p)).cloned().collect::<Vec<_>>().join(",");
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let param = if self.site.kind == Kind::Cloud { "query" } else { "username" };
        let url = format!("{}/user/assignable/multiProjectSearch?projectKeys={}&{param}={}&maxResults=20", self.jira_api(), enc(&keys), enc(q.trim()));
        let v = self.call(Api::Jira, Method::GET, &url, None).await?;
        Ok(v.as_array().into_iter().flatten().filter_map(user_info).take(20).collect())
    }

    /// Issues matching `jql` (built by `jql.rs`), at most `max` (≤ [`MAX_SEARCH`]), with the pickup
    /// fields. `reconcile`: Cloud issue ids whose latest edits must be reflected (read-after-write).
    /// Each hit comes with Jira's numeric issue id (for `reconcile` on the next poll).
    pub async fn search(&self, jql: &str, reconcile: &[String], max: usize) -> Result<Vec<(String, Issue)>, SourceError> {
        let max = max.clamp(1, MAX_SEARCH);
        let per = max.min(100);
        let mut out = Vec::new();
        match self.site.kind {
            Kind::Cloud => {
                let ids: Vec<Value> = reconcile.iter().filter_map(|i| i.parse::<u64>().ok()).take(MAX_RECONCILE).map(Value::from).collect();
                let mut token: Option<String> = None;
                loop {
                    let mut body = json!({"jql": jql, "fields": SEARCH_FIELDS, "maxResults": per});
                    if !ids.is_empty() {
                        body["reconcileIssues"] = Value::Array(ids.clone());
                    }
                    if let Some(t) = &token {
                        body["nextPageToken"] = Value::String(t.clone());
                    }
                    let v = self.call(Api::Jira, Method::POST, &format!("{}/search/jql", self.jira_api()), Some(&body)).await?;
                    for i in v["issues"].as_array().into_iter().flatten() {
                        out.push((s(&i["id"]), parse_issue(i, &self.site.jira_url, Kind::Cloud, None)?));
                    }
                    token = v["nextPageToken"].as_str().map(str::to_string);
                    if token.is_none() || v["isLast"].as_bool() == Some(true) || out.len() >= max {
                        break;
                    }
                }
            }
            Kind::Dc => loop {
                let body = json!({"jql": jql, "fields": SEARCH_FIELDS, "maxResults": per, "startAt": out.len()});
                let v = self.call(Api::Jira, Method::POST, &format!("{}/search", self.jira_api()), Some(&body)).await?;
                let page = v["issues"].as_array().cloned().unwrap_or_default();
                for i in &page {
                    out.push((s(&i["id"]), parse_issue(i, &self.site.jira_url, Kind::Dc, None)?));
                }
                let total = v["total"].as_u64().unwrap_or(0) as usize;
                if page.is_empty() || out.len() >= total || out.len() >= max {
                    break;
                }
            },
        }
        out.truncate(max);
        Ok(out)
    }

    /// How many issues match: Cloud's approximate count, DC's `total`.
    pub async fn count(&self, jql: &str) -> Result<u64, SourceError> {
        match self.site.kind {
            Kind::Cloud => {
                let v = self.call(Api::Jira, Method::POST, &format!("{}/search/approximate-count", self.jira_api()), Some(&json!({"jql": jql}))).await?;
                Ok(v["count"].as_u64().unwrap_or(0))
            }
            Kind::Dc => {
                let v = self.call(Api::Jira, Method::POST, &format!("{}/search", self.jira_api()), Some(&json!({"jql": jql, "maxResults": 0, "fields": ["key"]}))).await?;
                Ok(v["total"].as_u64().unwrap_or(0))
            }
        }
    }

    /// DC's "Epic Link" custom field id, looked up by name once (Cloud uses `parent`).
    async fn epic_field(&self) -> Option<String> {
        if self.site.kind != Kind::Dc {
            return None;
        }
        self.epic_field
            .get_or_init(|| async {
                let v = self.call(Api::Jira, Method::GET, &format!("{}/field", self.jira_api()), None).await.ok()?;
                v.as_array()?.iter().find(|f| f["name"] == "Epic Link").and_then(|f| f["id"].as_str()).filter(|id| id.starts_with("customfield_")).map(str::to_string)
            })
            .await
            .clone()
    }

    /// One issue by key with no scope check (pickup uses this for a ticket it already holds).
    pub async fn issue_unscoped(&self, key: &str) -> Result<Issue, SourceError> {
        if !valid_key(key) {
            return Err(SourceError::Invalid("that isn't a Jira issue key (like TRAIL-101)".into()));
        }
        let epic = self.epic_field().await;
        let mut fields = ISSUE_FIELDS.to_string();
        if let Some(e) = &epic {
            fields.push(',');
            fields.push_str(e);
        }
        let expand = if self.site.kind == Kind::Dc { "&expand=renderedFields" } else { "" };
        let v = self.call(Api::Jira, Method::GET, &format!("{}/issue/{key}?fields={fields}{expand}", self.jira_api()), None).await?;
        let mut issue = parse_issue(&v, &self.site.jira_url, self.site.kind, epic.as_deref())?;
        if issue.key != key {
            // A moved ticket answers with its new key; the caller keeps tracking the old one.
            issue.key = key.to_string();
        }
        Ok(issue)
    }

    /// Who last changed one of `fields` (e.g. `assignee`, `labels`) on a ticket kwant holds, from
    /// the issue's changelog. `None` when the changelog doesn't say (or is too long to have it).
    pub async fn last_changed_by(&self, key: &str, fields: &[&str]) -> Result<Option<String>, SourceError> {
        if !valid_key(key) {
            return Err(SourceError::Invalid("that isn't a Jira issue key (like TRAIL-101)".into()));
        }
        let v = self.call(Api::Jira, Method::GET, &format!("{}/issue/{key}?fields=created&expand=changelog", self.jira_api()), None).await?;
        Ok(last_changed_by(&v["changelog"], fields))
    }

    async fn remote_links(&self, key: &str) -> Vec<RemoteLink> {
        let Ok(v) = self.call(Api::Jira, Method::GET, &format!("{}/issue/{key}/remotelink", self.jira_api()), None).await else { return Vec::new() };
        v.as_array().into_iter().flatten().take(50).map(|l| RemoteLink { title: s(&l["object"]["title"]), url: s(&l["object"]["url"]) }).collect()
    }
}

#[async_trait::async_trait]
impl TicketSource for AtlassianClient {
    async fn issue(&self, key: &str) -> Result<Issue, SourceError> {
        self.in_scope(key)?;
        let mut issue = self.issue_unscoped(key).await?;
        issue.remote_links = self.remote_links(key).await;
        Ok(issue)
    }

    async fn comments(&self, key: &str) -> Result<Vec<Comment>, SourceError> {
        self.in_scope(key)?;
        let expand = if self.site.kind == Kind::Dc { "&expand=renderedBody" } else { "" };
        let url = format!("{}/issue/{key}/comment?orderBy=-created&maxResults={MAX_COMMENTS}{expand}", self.jira_api());
        let v = self.call(Api::Jira, Method::GET, &url, None).await?;
        let mut out: Vec<Comment> = v["comments"].as_array().into_iter().flatten().map(|c| parse_comment(c, self.site.kind)).collect();
        // Oldest first, whatever order the server honoured.
        out.sort_by(|a, b| a.created.cmp(&b.created).then_with(|| a.id.len().cmp(&b.id.len())).then_with(|| a.id.cmp(&b.id)));
        let skip = out.len().saturating_sub(MAX_COMMENTS);
        Ok(out.split_off(skip))
    }

    async fn search_pages(&self, query: &PageQuery) -> Result<Vec<PageHit>, SourceError> {
        self.confluence_search(query).await
    }

    async fn page(&self, id: &str) -> Result<Page, SourceError> {
        self.confluence_page(id).await
    }
}

#[async_trait::async_trait]
impl TicketWriter for AtlassianClient {
    async fn add_comment(&self, key: &str, body: &str) -> Result<Comment, SourceError> {
        self.in_scope(key)?;
        let text = body.trim();
        if text.is_empty() || text.chars().count() > WRITE_CHARS {
            return Err(SourceError::Invalid("a comment is 1 to 10,000 characters".into()));
        }
        let payload = match self.site.kind {
            Kind::Cloud => json!({"body": text_to_adf(text)}),
            Kind::Dc => json!({"body": text_to_wiki(text)}),
        };
        // Never retried: a timeout may still have posted it.
        let v = self.call(Api::Jira, Method::POST, &format!("{}/issue/{key}/comment", self.jira_api()), Some(&payload)).await?;
        let mut c = parse_comment(&v, self.site.kind);
        if c.body.is_empty() {
            c.body = text.to_string();
        }
        if c.id.is_empty() {
            return Err(SourceError::Invalid("Jira didn't return the comment".into()));
        }
        Ok(c)
    }

    async fn mention_target(&self, key: &str) -> Result<Option<Mention>, SourceError> {
        self.in_scope(key)?;
        let v = self.call(Api::Jira, Method::GET, &format!("{}/issue/{key}?fields=reporter,assignee", self.jira_api()), None).await?;
        for role in ["reporter", "assignee"] {
            let u = &v["fields"][role];
            let Some(p) = person(u) else { continue };
            // Cloud mentions take the accountId; DC's `[~…]` takes the username.
            let mention_id = match self.site.kind {
                Kind::Cloud => u["accountId"].as_str(),
                Kind::Dc => u["name"].as_str(),
            };
            if let Some(id) = mention_id.filter(|i| !i.trim().is_empty()) {
                return Ok(Some(Mention { mention_id: id.to_string(), person_id: p.id, display_name: cut(&p.display_name, 120).0, role }));
            }
        }
        Ok(None)
    }

    async fn ask(&self, key: &str, to: Option<&Mention>, rest: &str) -> Result<Posted, SourceError> {
        self.in_scope(key)?;
        let text = ask_text(to, rest);
        if rest.trim().is_empty() || text.chars().count() > WRITE_CHARS {
            return Err(SourceError::Invalid("a comment is 1 to 10,000 characters".into()));
        }
        let payload = match self.site.kind {
            Kind::Cloud => json!({"body": ask_adf(to, rest)}),
            Kind::Dc => json!({"body": ask_wiki(to, rest)}),
        };
        // Never retried: a timeout may still have posted it.
        let v = self.call(Api::Jira, Method::POST, &format!("{}/issue/{key}/comment", self.jira_api()), Some(&payload)).await?;
        let mut c = parse_comment(&v, self.site.kind);
        if c.id.is_empty() {
            return Err(SourceError::Invalid("Jira didn't return the comment".into()));
        }
        if c.body.is_empty() {
            c.body = text;
        }
        Ok(Posted { url: self.comment_url(key, &c.id), comment: c })
    }

    fn comment_url(&self, key: &str, comment_id: &str) -> String {
        focused_comment_url(&self.site.jira_url, key, comment_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cloud_issue_parses() {
        let v = json!({
            "key": "TRAIL-101", "id": "10101",
            "fields": {
                "summary": "Add a link", "project": {"key": "TRAIL"}, "issuetype": {"name": "Story"},
                "status": {"name": "To Do", "statusCategory": {"key": "new"}},
                "priority": {"name": "High", "id": "2"},
                "reporter": {"accountId": "u-ann", "displayName": "Ann Lee", "emailAddress": "ann@example.com"},
                "assignee": null, "labels": ["kwant"],
                "description": {"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Hello"}]}]},
                "parent": {"key": "TRAIL-100", "fields": {"summary": "Epic", "status": {"name": "In Progress", "statusCategory": {"key": "indeterminate"}}}},
                "issuelinks": [{"type": {"inward": "is blocked by", "outward": "blocks"}, "inwardIssue": {"key": "TRAIL-99", "fields": {"summary": "x", "status": {"name": "Done", "statusCategory": {"key": "done"}}}}}],
                "attachment": [{"filename": "a.png"}],
                "created": "2026-11-07T09:58:00.000+0000"
            }
        });
        let i = parse_issue(&v, "https://yourcompany.atlassian.net", Kind::Cloud, None).unwrap();
        assert_eq!(i.url, "https://yourcompany.atlassian.net/browse/TRAIL-101");
        assert_eq!(i.description, "Hello");
        assert_eq!(i.priority_rank, Some(2));
        assert_eq!(i.reporter.unwrap(), Person { id: "u-ann".into(), display_name: "Ann Lee".into() });
        assert_eq!(i.parent.unwrap().status_category, StatusCategory::InProgress);
        assert_eq!(i.links[0].relation, "is blocked by");
        assert_eq!(i.links[0].issue.status_category, StatusCategory::Done);
        assert_eq!(i.attachments, vec!["a.png"]);
        assert_eq!(i.created.unwrap().to_rfc3339(), "2026-11-07T09:58:00+00:00");
    }

    #[test]
    fn a_dc_issue_reads_rendered_text_and_the_epic_link() {
        let v = json!({
            "key": "TRAIL-102",
            "fields": {"summary": "Export", "status": {"name": "Open", "statusCategory": {"key": "new"}}, "description": "h1. raw", "customfield_10008": "TRAIL-100",
                       "reporter": {"key": "JIRAUSER10100", "name": "ann.lee", "displayName": "Ann Lee"}},
            "renderedFields": {"description": "<p>Make a <b>CSV</b></p>"}
        });
        let i = parse_issue(&v, "https://jira.example.com", Kind::Dc, Some("customfield_10008")).unwrap();
        assert_eq!(i.description, "Make a CSV");
        assert_eq!(i.parent.unwrap().key, "TRAIL-100");
        assert_eq!(i.reporter.unwrap().id, "JIRAUSER10100");
        assert!(parse_issue(&json!({"key": "nope"}), "x", Kind::Dc, None).is_err());
    }

    fn dana(id: &str) -> Mention {
        Mention { mention_id: id.into(), person_id: id.into(), display_name: "Dana Park".into(), role: "reporter" }
    }

    #[test]
    fn a_cloud_ask_mentions_by_account_id_in_the_first_paragraph() {
        let d = dana("5b10ac8d82e05b22cc7d4ef5");
        let doc = ask_adf(Some(&d), "before we start TRAIL-142 we have a question:\n\n1. CSV or XLSX?");
        let first = &doc["content"][0]["content"];
        assert_eq!(first[0], json!({"type": "text", "text": "Hi "}));
        assert_eq!(first[1], json!({"type": "mention", "attrs": {"id": "5b10ac8d82e05b22cc7d4ef5", "text": "@Dana Park"}}));
        assert_eq!(first[2], json!({"type": "text", "text": ", "}));
        assert_eq!(first[3]["text"], "before we start TRAIL-142 we have a question:");
        assert!(adf_to_text(&doc).starts_with("Hi @Dana Park, before we start TRAIL-142 we have a question:"));
        let none = ask_adf(None, "x");
        assert_eq!(none["content"][0]["content"][0]["text"], "Hi, ");
    }

    #[test]
    fn a_dc_ask_mentions_by_username_and_escapes_the_rest() {
        assert_eq!(ask_wiki(Some(&dana("dana.park")), "pick [one] {a|b}"), "Hi [~dana.park], pick \\[one\\] \\{a\\|b\\}");
        // A username that would break out of [~…] is not used as a mention.
        assert_eq!(ask_wiki(Some(&dana("x]y")), "q"), "Hi Dana Park, q");
        assert_eq!(ask_wiki(None, "q"), "Hi, q");
        assert_eq!(ask_text(Some(&dana("dana.park")), " q "), "Hi @Dana Park, q");
    }

    #[test]
    fn the_newest_assignee_or_label_change_names_who_took_it_back() {
        let log = json!({"histories": [
            {"created": "2026-11-07T10:05:00.000+0000", "author": {"displayName": "Dana Park"}, "items": [{"field": "labels"}]},
            {"created": "2026-11-07T09:00:00.000+0000", "author": {"displayName": "Raj Patel"}, "items": [{"field": "assignee"}]},
            {"created": "2026-11-07T11:00:00.000+0000", "author": {"displayName": "Mo Chen"}, "items": [{"field": "status"}]}
        ]});
        assert_eq!(last_changed_by(&log, &["assignee", "labels"]).as_deref(), Some("Dana Park"));
        assert_eq!(last_changed_by(&log, &["assignee"]).as_deref(), Some("Raj Patel"));
        assert_eq!(last_changed_by(&log, &["priority"]), None);
        assert_eq!(last_changed_by(&json!({}), &["assignee"]), None);
        assert_eq!(focused_comment_url("https://jira.example.com/", "TRAIL-1", "10042"), "https://jira.example.com/browse/TRAIL-1?focusedCommentId=10042");
    }
}
