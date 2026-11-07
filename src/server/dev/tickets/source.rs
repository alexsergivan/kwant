//! The read trait and neutral types of a ticket source, verbatim from jira-tickets api.md 1.2
//! (owned by Juno; at merge, take Juno's file, which also adds `fixture::FixtureSource`).

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
    pub key: String,                    // "TRAIL-102"
    pub url: String,                    // browse URL, for citations
    pub project_key: String,
    pub issue_type: String,             // "Story", "Bug", …
    pub summary: String,
    pub description: String,            // plain text: ADF (Cloud v3) or wiki markup (DC v2) flattened
    pub status: String,                 // the site's own name, "In Progress"
    pub status_category: StatusCategory,
    pub priority: Option<String>,       // "High"; None when the site has no priorities
    pub priority_rank: Option<u32>,     // lower = more urgent, from the site's priority order (J13 queue)
    pub reporter: Option<Person>,
    pub assignee: Option<Person>,
    pub labels: Vec<String>,
    /// Parent on Cloud (sub-task parent or epic); on DC the sub-task parent, else the Epic Link field.
    pub parent: Option<LinkedIssue>,
    pub links: Vec<IssueLink>,
    pub remote_links: Vec<RemoteLink>,
    pub attachments: Vec<String>,       // names only (spec: out of scope beyond names)
    pub created: Option<DateTime<Utc>>,
    pub updated: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusCategory { #[default] ToDo, InProgress, Done }

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
    pub relation: String,               // as Jira words it from this side: "blocks", "is blocked by", "relates to"
    pub issue: LinkedIssue,
}

/// Title and URL are untrusted text: quote them, never fetch them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RemoteLink { pub title: String, pub url: String }

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Comment {
    pub id: String,
    pub author: Option<Person>,
    pub created: Option<DateTime<Utc>>,
    pub updated: Option<DateTime<Utc>>,
    pub body: String,                   // plain text
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PageHit {
    pub id: String,
    pub title: String,
    pub space_key: String,
    pub version: u32,
    pub url: String,
    pub excerpt: String,                // plain text, highlight markers removed
    pub updated: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Page {
    pub id: String,
    pub title: String,
    pub space_key: String,
    pub version: u32,
    pub url: String,
    pub body: String,                   // plain text from storage format (Cloud and DC alike)
    pub truncated: bool,
}

/// `Display` is safe to show a bot or the person: never a token, header or credentialed URL.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SourceError {
    #[error("not found")] NotFound,
    #[error("outside this site's projects or spaces")] OutOfScope,
    #[error("Confluence search is off for this site")] NoSpaces,
    #[error("the token was rejected")] Unauthorized,      // 401
    #[error("the token can't see this")] Forbidden,       // 403
    #[error("rate limited")] RateLimited { retry_after: Option<std::time::Duration> }, // 429
    #[error("{0}")] Unavailable(String),                   // network or 5xx, short reason
    #[error("{0}")] Invalid(String),                       // bad key, empty query, unreadable reply
}
