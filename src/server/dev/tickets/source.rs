//! Where a ticket's words come from (jira-tickets api.md §1.2): the read trait the brief turn's
//! tools call ([`TicketSource`]) and the one write ([`TicketWriter`]), with host-neutral types at
//! the boundary, like `dev/source.rs`. Jira Cloud and Data Center both come out as these types
//! (`dev::atlassian`); Linear or GitHub Issues could later.
//!
//! A source is built for one site and scoped to the projects and Confluence spaces the person
//! chose: a key, page or space outside them is [`SourceError::OutOfScope`], whatever a bot asks.
//! Every string in these types is untrusted text from the tracker: quote it, never obey it.

use chrono::{DateTime, Utc};

#[async_trait::async_trait]
pub trait TicketSource: Send + Sync {
    /// One ticket. `OutOfScope` when the key's project isn't one of the site's projects.
    async fn issue(&self, key: &str) -> Result<Issue, SourceError>;
    /// Oldest first; at most the newest 100; each body cut at 8,000 chars (`truncated` set).
    async fn comments(&self, key: &str) -> Result<Vec<Comment>, SourceError>;
    /// Confluence pages in the site's chosen spaces only. `query.limit` is clamped to 1..=25;
    /// Vega's own cap (5) applies on top. `NoSpaces` when Confluence is off or no space is chosen.
    async fn search_pages(&self, query: &PageQuery) -> Result<Vec<PageHit>, SourceError>;
    /// One page as plain text. `OutOfScope` when the page's space isn't chosen. Hard ceiling 200,000
    /// chars (`truncated` set); Vega cuts to 20k.
    async fn page(&self, id: &str) -> Result<Page, SourceError>;
}

/// Writes. A separate trait so the bots' read tools can never hold a writer. Only person-only routes
/// (Ask on Jira, the J9 work-started / PR-ready comments) get one.
#[async_trait::async_trait]
pub trait TicketWriter: Send + Sync {
    /// Posts one plain-text comment (ADF paragraphs on Cloud, wiki text on DC). Returns it with its id.
    async fn add_comment(&self, key: &str, body: &str) -> Result<Comment, SourceError>;
    /// Who an Ask on Jira comment mentions (design §5): the reporter, else the assignee; `None`
    /// when the ticket has neither.
    async fn mention_target(&self, key: &str) -> Result<Option<Mention>, SourceError>;
    /// Posts an Ask on Jira comment: `Hi <mention>, ` then `rest`. The mention is a real one (ADF
    /// `mention` node with the accountId on Cloud, `[~username]` on DC); with `to: None` it's
    /// `Hi, `. Never retried. The text is [`ask_text`] of the same arguments.
    async fn ask(&self, key: &str, to: Option<&Mention>, rest: &str) -> Result<Posted, SourceError>;
    /// The link to one comment: `<site>/browse/KEY?focusedCommentId=<id>`.
    fn comment_url(&self, key: &str, comment_id: &str) -> String;
}

/// The person an Ask on Jira comment is addressed to.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub struct Mention {
    /// What the mention carries: Cloud `accountId`; DC username (`name`, what `[~…]` takes).
    pub mention_id: String,
    /// [`Person::id`] of the same user (Cloud accountId, DC user key): how a reply is matched.
    pub person_id: String,
    pub display_name: String,
    /// `reporter` or `assignee`.
    pub role: &'static str,
}

/// A posted comment and its link.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Posted {
    pub comment: Comment,
    pub url: String,
}

/// The exact text an Ask on Jira comment shows (the preview the person approves), with the
/// mention as `@Display Name`.
pub fn ask_text(to: Option<&Mention>, rest: &str) -> String {
    match to {
        Some(m) => format!("Hi @{}, {}", m.display_name.trim(), rest.trim()),
        None => format!("Hi, {}", rest.trim()),
    }
}

/// A comment link with Jira's focused-comment anchor.
pub fn focused_comment_url(browse_base: &str, key: &str, comment_id: &str) -> String {
    format!("{}/browse/{key}?focusedCommentId={}", browse_base.trim_end_matches('/'), percent_encoding::utf8_percent_encode(comment_id, percent_encoding::NON_ALPHANUMERIC))
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PageQuery {
    /// Words to find. Quoted and escaped by the client: `text ~ "<words>"`.
    pub text: String,
    /// Match the title only (`title ~`), for "is there a page called …".
    pub title_only: bool,
    pub limit: usize,
}

/// A Jira user. Never an email: Cloud hides it and we don't want it in a brief.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Person {
    /// Cloud `accountId`; DC `key` (falls back to `name`). Used to tell the reporter's reply apart.
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Issue {
    pub key: String,
    pub url: String,
    pub project_key: String,
    pub issue_type: String,
    pub summary: String,
    /// Plain text: ADF (Cloud v3) or the rendered wiki markup (DC v2) flattened.
    pub description: String,
    pub status: String,
    pub status_category: StatusCategory,
    pub priority: Option<String>,
    /// Lower = more urgent (J13 queue). From the priority's name, else its id (see `atlassian::priority_rank`).
    pub priority_rank: Option<u32>,
    pub reporter: Option<Person>,
    pub assignee: Option<Person>,
    pub labels: Vec<String>,
    /// Parent on Cloud (sub-task parent or epic); on DC the sub-task parent, else the Epic Link field.
    pub parent: Option<LinkedIssue>,
    pub links: Vec<IssueLink>,
    pub remote_links: Vec<RemoteLink>,
    /// Names only (spec: out of scope beyond names).
    pub attachments: Vec<String>,
    pub created: Option<DateTime<Utc>>,
    pub updated: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusCategory {
    #[default]
    ToDo,
    InProgress,
    Done,
}

impl StatusCategory {
    /// From Jira's `statusCategory.key`: `new`, `indeterminate`, `done` (`undefined` reads as to do).
    pub fn from_key(key: &str) -> Self {
        match key {
            "done" => Self::Done,
            "indeterminate" => Self::InProgress,
            _ => Self::ToDo,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToDo => "todo",
            Self::InProgress => "in_progress",
            Self::Done => "done",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinkedIssue {
    pub key: String,
    pub summary: String,
    pub status: String,
    pub status_category: StatusCategory,
}

/// A link carries the other ticket's summary and status inline, so reading it needs no second call
/// (and works when the linked ticket is in a project outside the site's scope).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IssueLink {
    /// As Jira words it from this side: "blocks", "is blocked by", "relates to".
    pub relation: String,
    pub issue: LinkedIssue,
}

/// Title and URL are untrusted text: quote them, never fetch them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RemoteLink {
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Comment {
    pub id: String,
    pub author: Option<Person>,
    pub created: Option<DateTime<Utc>>,
    pub updated: Option<DateTime<Utc>>,
    /// Plain text.
    pub body: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PageHit {
    pub id: String,
    pub title: String,
    pub space_key: String,
    pub version: u32,
    pub url: String,
    /// Plain text, highlight markers removed.
    pub excerpt: String,
    pub updated: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Page {
    pub id: String,
    pub title: String,
    pub space_key: String,
    pub version: u32,
    pub url: String,
    /// Plain text from storage format (Cloud and DC alike).
    pub body: String,
    pub truncated: bool,
}

/// `Display` is safe to show a bot or the person: never a token, header or credentialed URL.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SourceError {
    #[error("not found")]
    NotFound,
    #[error("outside this site's projects or spaces")]
    OutOfScope,
    #[error("Confluence search is off for this site")]
    NoSpaces,
    #[error("the token was rejected")]
    Unauthorized,
    #[error("the token can't see this")]
    Forbidden,
    #[error("rate limited")]
    RateLimited { retry_after: Option<std::time::Duration> },
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Invalid(String),
}

/// Most comments [`TicketSource::comments`] returns (the newest).
pub const MAX_COMMENTS: usize = 100;
/// Each comment body is cut here.
pub const COMMENT_CHARS: usize = 8_000;
/// Each page body is cut here (Vega cuts further, to 20k).
pub const PAGE_CHARS: usize = 200_000;
/// [`PageQuery::limit`] is clamped to `1..=MAX_PAGE_HITS`.
pub const MAX_PAGE_HITS: usize = 25;
/// The longest comment [`TicketWriter::add_comment`] posts.
pub const WRITE_CHARS: usize = 10_000;

/// `s` cut at `max` chars (not bytes), and whether it was cut.
pub fn cut(s: &str, max: usize) -> (String, bool) {
    match s.char_indices().nth(max) {
        Some((i, _)) => (s[..i].to_string(), true),
        None => (s.to_string(), false),
    }
}

/// A Jira issue key: `PROJ-123`, project part `[A-Z][A-Z0-9_]+` (1–30 more chars).
pub fn valid_key(key: &str) -> bool {
    let Some((project, n)) = key.split_once('-') else { return false };
    valid_project(project) && !n.is_empty() && n.len() <= 12 && n.bytes().all(|b| b.is_ascii_digit()) && !n.starts_with('0')
}

/// A Jira project key: an upper-case letter, then 1–30 upper-case letters, digits or `_`.
pub fn valid_project(p: &str) -> bool {
    let b = p.as_bytes();
    (2..=31).contains(&b.len()) && b[0].is_ascii_uppercase() && b.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'_')
}

/// The project part of a valid key.
pub fn project_of(key: &str) -> Option<&str> {
    valid_key(key).then(|| key.split_once('-').map(|(p, _)| p)).flatten()
}

pub mod fixture {
    //! [`FixtureSource`]: both traits over the Trailhead Outfitters fixture the mock Atlassian
    //! server serves (`dev/fixtures/atlassian/trail.json`), with no HTTP. It records every
    //! `add_comment`, so a test can assert "exactly one comment".

    use super::*;
    use serde_json::Value;
    use std::sync::Mutex;

    /// The fixture's JSON, shared with `atlassian::mock` and the media kit's Python mock.
    pub const TRAIL_JSON: &str = include_str!("../fixtures/atlassian/trail.json");

    pub struct FixtureSource {
        data: Value,
        projects: Vec<String>,
        spaces: Vec<String>,
        base: String,
        /// Every comment posted, as `(key, body)`.
        pub written: Mutex<Vec<(String, String)>>,
    }

    impl FixtureSource {
        /// Scoped like a site that chose project TRAIL and Confluence space OPS (never HR).
        pub fn trail() -> Self {
            Self::scoped(&["TRAIL"], &["OPS"])
        }

        pub fn scoped(projects: &[&str], spaces: &[&str]) -> Self {
            Self {
                data: serde_json::from_str(TRAIL_JSON).expect("the fixture is valid JSON"),
                projects: projects.iter().map(|s| s.to_string()).collect(),
                spaces: spaces.iter().map(|s| s.to_string()).collect(),
                base: "https://yourcompany.atlassian.net".into(),
                written: Mutex::new(Vec::new()),
            }
        }

        fn raw_issue(&self, key: &str) -> Result<&Value, SourceError> {
            let p = project_of(key).ok_or_else(|| SourceError::Invalid("not an issue key".into()))?;
            if !self.projects.iter().any(|x| x == p) {
                return Err(SourceError::OutOfScope);
            }
            self.data["issues"].as_array().and_then(|a| a.iter().find(|i| i["key"] == key)).ok_or(SourceError::NotFound)
        }

        fn person(&self, id: &Value) -> Option<Person> {
            let id = id.as_str()?;
            let u = self.data["users"].as_array()?.iter().find(|u| u["id"] == id)?;
            Some(Person { id: id.to_string(), display_name: u["display_name"].as_str().unwrap_or_default().to_string() })
        }

        fn linked(&self, key: &str) -> LinkedIssue {
            let i = self.data["issues"].as_array().and_then(|a| a.iter().find(|i| i["key"] == key));
            LinkedIssue {
                key: key.to_string(),
                summary: i.and_then(|i| i["summary"].as_str()).unwrap_or_default().to_string(),
                status: i.and_then(|i| i["status"].as_str()).unwrap_or_default().to_string(),
                status_category: StatusCategory::from_key(i.and_then(|i| i["category"].as_str()).unwrap_or("new")),
            }
        }
    }

    fn s(v: &Value) -> String {
        v.as_str().unwrap_or_default().to_string()
    }

    fn date(v: &Value) -> Option<DateTime<Utc>> {
        v.as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|d| d.with_timezone(&Utc))
    }

    #[async_trait::async_trait]
    impl TicketSource for FixtureSource {
        async fn issue(&self, key: &str) -> Result<Issue, SourceError> {
            let i = self.raw_issue(key)?;
            let strings = |v: &Value| v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
            Ok(Issue {
                key: key.to_string(),
                url: format!("{}/browse/{key}", self.base),
                project_key: project_of(key).unwrap_or_default().to_string(),
                issue_type: s(&i["type"]),
                summary: s(&i["summary"]),
                description: s(&i["description"]),
                status: s(&i["status"]),
                status_category: StatusCategory::from_key(i["category"].as_str().unwrap_or("new")),
                priority: i["priority"].as_str().map(str::to_string),
                priority_rank: i["priority"].as_str().and_then(|p| crate::server::dev::atlassian::priority_rank(p, None)),
                reporter: self.person(&i["reporter"]),
                assignee: self.person(&i["assignee"]),
                labels: strings(&i["labels"]),
                parent: i["parent"].as_str().map(|k| self.linked(k)),
                links: i["links"]
                    .as_array()
                    .map(|a| a.iter().map(|l| IssueLink { relation: s(&l["relation"]), issue: self.linked(l["key"].as_str().unwrap_or_default()) }).collect())
                    .unwrap_or_default(),
                remote_links: i["remote_links"].as_array().map(|a| a.iter().map(|l| RemoteLink { title: s(&l["title"]), url: s(&l["url"]) }).collect()).unwrap_or_default(),
                attachments: strings(&i["attachments"]),
                created: date(&i["created"]),
                updated: date(&i["updated"]),
            })
        }

        async fn comments(&self, key: &str) -> Result<Vec<Comment>, SourceError> {
            let i = self.raw_issue(key)?;
            let mut out: Vec<Comment> = i["comments"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|c| {
                            let (body, truncated) = cut(c["body"].as_str().unwrap_or_default(), COMMENT_CHARS);
                            Comment { id: s(&c["id"]), author: self.person(&c["author"]), created: date(&c["created"]), updated: date(&c["created"]), body, truncated }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let written = self.written.lock().unwrap().clone();
            out.extend(written.iter().enumerate().filter(|(_, (k, _))| k == key).map(|(n, (_, b))| Comment {
                id: format!("fixture-{}", n + 1),
                author: self.person(&self.data["me"]["id"]),
                body: b.clone(),
                ..Default::default()
            }));
            let skip = out.len().saturating_sub(MAX_COMMENTS);
            Ok(out.split_off(skip))
        }

        async fn search_pages(&self, query: &PageQuery) -> Result<Vec<PageHit>, SourceError> {
            if self.spaces.is_empty() {
                return Err(SourceError::NoSpaces);
            }
            let words = query.text.trim().to_lowercase();
            if words.is_empty() {
                return Err(SourceError::Invalid("say what to search for".into()));
            }
            let limit = query.limit.clamp(1, MAX_PAGE_HITS);
            Ok(self.data["pages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|p| self.spaces.iter().any(|sp| p["space"] == sp.as_str()))
                .filter(|p| {
                    let title = p["title"].as_str().unwrap_or_default().to_lowercase();
                    let body = p["body"].as_str().unwrap_or_default().to_lowercase();
                    words.split_whitespace().any(|w| title.contains(w) || (!query.title_only && body.contains(w)))
                })
                .take(limit)
                .map(|p| PageHit {
                    id: s(&p["id"]),
                    title: s(&p["title"]),
                    space_key: s(&p["space"]),
                    version: p["version"].as_u64().unwrap_or(1) as u32,
                    url: format!("{}/wiki/spaces/{}/pages/{}", self.base, s(&p["space"]), s(&p["id"])),
                    excerpt: cut(&crate::server::dev::atlassian::text::html_to_text(p["body"].as_str().unwrap_or_default()), 200).0,
                    updated: date(&p["updated"]),
                })
                .collect())
        }

        async fn page(&self, id: &str) -> Result<Page, SourceError> {
            let p = self.data["pages"].as_array().and_then(|a| a.iter().find(|p| p["id"] == id)).ok_or(SourceError::NotFound)?;
            if !self.spaces.iter().any(|sp| p["space"] == sp.as_str()) {
                return Err(SourceError::OutOfScope);
            }
            let (body, truncated) = cut(&crate::server::dev::atlassian::text::html_to_text(p["body"].as_str().unwrap_or_default()), PAGE_CHARS);
            Ok(Page {
                id: id.to_string(),
                title: s(&p["title"]),
                space_key: s(&p["space"]),
                version: p["version"].as_u64().unwrap_or(1) as u32,
                url: format!("{}/wiki/spaces/{}/pages/{id}", self.base, s(&p["space"])),
                body,
                truncated,
            })
        }
    }

    #[async_trait::async_trait]
    impl TicketWriter for FixtureSource {
        async fn add_comment(&self, key: &str, body: &str) -> Result<Comment, SourceError> {
            self.raw_issue(key)?;
            let body = body.trim();
            if body.is_empty() || body.chars().count() > WRITE_CHARS {
                return Err(SourceError::Invalid("a comment is 1 to 10,000 characters".into()));
            }
            let mut w = self.written.lock().unwrap();
            w.push((key.to_string(), body.to_string()));
            Ok(Comment { id: format!("fixture-{}", w.len()), author: self.person(&self.data["me"]["id"]), body: body.to_string(), ..Default::default() })
        }

        async fn mention_target(&self, key: &str) -> Result<Option<Mention>, SourceError> {
            let i = self.raw_issue(key)?;
            for role in ["reporter", "assignee"] {
                if let Some(id) = i[role].as_str() {
                    if let Some(p) = self.person(&Value::String(id.to_string())) {
                        return Ok(Some(Mention { mention_id: p.id.clone(), person_id: p.id, display_name: p.display_name, role }));
                    }
                }
            }
            Ok(None)
        }

        async fn ask(&self, key: &str, to: Option<&Mention>, rest: &str) -> Result<Posted, SourceError> {
            let c = self.add_comment(key, &ask_text(to, rest)).await?;
            Ok(Posted { url: self.comment_url(key, &c.id), comment: c })
        }

        fn comment_url(&self, key: &str, comment_id: &str) -> String {
            focused_comment_url(&self.base, key, comment_id)
        }
    }
}
