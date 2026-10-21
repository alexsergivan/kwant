//! Zombie Mission Hunter: finds things the person started and never finished (an email that got
//! no reply, a promise nobody kept, an issue that went quiet, a bot task that failed) and turns
//! each into a case with verbatim evidence and a drafted next action.
//!
//! - [`MissionSource`]s read conversations, read-only: kwant's own threads, delegations and
//!   questions ([`kwant_source`]), a mailbox over IMAP ([`imap`]), GitHub issues and pull
//!   requests ([`github`]). Their logins live in the password vault, never in config or logs.
//! - [`detect`] applies plain rules (you wrote last and nobody answered for N days, a promise
//!   that lapsed, a question you owe an answer to). Every case quotes its evidence word for word.
//! - [`classify`] is an optional model pass that can only drop rule candidates, and only keeps
//!   one whose quote is found verbatim in the source text. It runs without any tools.
//! - [`draft`] writes the follow-up. Nothing is ever sent: the person copies it or opens it in
//!   their mail app.
//! - [`hunt`] runs a scan, stores cases (`db::zombie`), and the weekly "Loose ends" review.
//!
//! See docs/zombie-hunter.md.

pub mod classify;
pub mod detect;
pub mod draft;
pub mod github;
pub mod hunt;
pub mod imap;
pub mod kwant_source;
pub mod settings;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The vault owner the sources' logins are saved under. No team, channel or bot has this id, so
/// no browser ever fills them into a page.
pub const VAULT_OWNER: &str = "kwnt-zombie-hunter";

/// Longest text kept of one message, after quoted replies are cut off.
pub const MAX_MESSAGE_CHARS: usize = 4000;
/// Longest evidence quote.
pub const MAX_QUOTE_CHARS: usize = 280;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Kwant,
    Imap,
    Github,
    /// A letter the person photographed or uploaded (`crate::documents`).
    Document,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Kwant => "kwant",
            SourceKind::Imap => "imap",
            SourceKind::Github => "github",
            SourceKind::Document => "document",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "kwant" => Some(SourceKind::Kwant),
            "imap" => Some(SourceKind::Imap),
            "github" => Some(SourceKind::Github),
            "document" => Some(SourceKind::Document),
            _ => None,
        }
    }
}

/// What a conversation is, which decides the rules and the draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConvKind {
    Email,
    Issue,
    PullRequest,
    /// A chat with a bot: the person's messages are outbound, the bot's inbound.
    KwantThread,
    /// A bot's `ask_user` question still open.
    KwantQuestion,
    /// A task handed to a bot that failed or was cut short.
    KwantDelegation,
    /// A letter on paper or in a PDF.
    Document,
}

/// One message as the detector sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Msg {
    /// Address, login or name of who wrote it.
    pub from: String,
    /// Written by the person.
    pub outbound: bool,
    pub at: DateTime<Utc>,
    /// Plain text, quoted earlier replies removed, at most [`MAX_MESSAGE_CHARS`].
    pub text: String,
    /// An auto-reply (out of office, "we received your request"): never counts as an answer.
    pub auto: bool,
    /// A newsletter, mailing list, no-reply sender or bot account.
    pub bulk: bool,
    /// Message-ID, for a reply that threads under it.
    pub message_id: Option<String>,
}

/// A thread, issue or bot task as a [`MissionSource`] hands it over.
#[derive(Debug, Clone)]
pub struct Conversation {
    pub source: SourceKind,
    /// The source config's id, or `"kwant"`.
    pub source_id: String,
    /// Stable within the source: the thread root, the issue URL, the kwant thread id.
    pub key: String,
    pub kind: ConvKind,
    pub subject: String,
    pub link: Option<String>,
    /// Who a follow-up goes to: an address, a login, a bot's name.
    pub counterpart: Option<String>,
    pub counterpart_name: Option<String>,
    /// Oldest first.
    pub messages: Vec<Msg>,
    /// Issues and pull requests: still open. Always true for the rest.
    pub open: bool,
    /// The work is the person's: an issue or pull request they opened or are assigned to.
    pub mine: bool,
    /// A reference the source knows for sure (`owner/repo#12`); otherwise one is looked for in
    /// the text.
    pub reference: Option<String>,
    /// A bot task that failed: the error shown as evidence.
    pub failed: bool,
    /// The kwant space it belongs to, for kwant's own conversations.
    pub space_id: Option<String>,
    /// The bot whose chat it is in, for kwant's own conversations.
    pub kwant_id: Option<String>,
}

impl Conversation {
    pub fn new(source: SourceKind, source_id: &str, key: &str, kind: ConvKind, subject: &str) -> Self {
        Self {
            source,
            source_id: source_id.into(),
            key: key.into(),
            kind,
            subject: subject.into(),
            link: None,
            counterpart: None,
            counterpart_name: None,
            messages: Vec::new(),
            open: true,
            mine: false,
            reference: None,
            failed: false,
            space_id: None,
            kwant_id: None,
        }
    }

    /// The key a case for this conversation is stored under.
    pub fn dedupe_key(&self) -> String {
        format!("{}:{}:{}", self.source.as_str(), self.source_id, self.key)
    }

    /// Every text the evidence may quote from.
    pub fn texts(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.subject.as_str()).chain(self.messages.iter().map(|m| m.text.as_str()))
    }
}

/// What kind of loose end a case is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseKind {
    /// The person wrote last, expecting an answer, and none came.
    AwaitingReply,
    /// The other side promised to get back, and didn't.
    PromiseLapsed,
    /// The person promised something and nothing followed.
    YouPromised,
    /// The other side asked the person something in an ongoing exchange and got no answer.
    YouOweReply,
    /// An issue or pull request of the person's with no activity.
    StaleIssue,
    /// A bot task that failed or was interrupted and was never retried.
    FailedTask,
    /// A bot asked the person a question that is still open.
    UnansweredQuestion,
    /// A letter asks the person to do something.
    DocumentRequest,
}

impl CaseKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CaseKind::AwaitingReply => "awaiting_reply",
            CaseKind::PromiseLapsed => "promise_lapsed",
            CaseKind::YouPromised => "you_promised",
            CaseKind::YouOweReply => "you_owe_reply",
            CaseKind::StaleIssue => "stale_issue",
            CaseKind::FailedTask => "failed_task",
            CaseKind::UnansweredQuestion => "unanswered_question",
            CaseKind::DocumentRequest => "document_request",
        }
    }

    /// Ranking weight in a review: what someone else is waiting on comes first.
    pub fn weight(self) -> i64 {
        match self {
            CaseKind::PromiseLapsed | CaseKind::YouOweReply | CaseKind::YouPromised | CaseKind::DocumentRequest => 3,
            CaseKind::AwaitingReply | CaseKind::UnansweredQuestion => 2,
            CaseKind::FailedTask | CaseKind::StaleIssue => 1,
        }
    }
}

/// A verbatim quote backing a case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub quote: String,
    pub from: String,
    pub at: DateTime<Utc>,
    pub outbound: bool,
}

/// What the rules found in one conversation, before it becomes a stored case.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub kind: CaseKind,
    pub dedupe_key: String,
    pub source: SourceKind,
    pub source_id: String,
    pub source_ref: String,
    pub conv_kind: ConvKind,
    pub title: String,
    pub counterpart: Option<String>,
    pub counterpart_name: Option<String>,
    pub reference: Option<String>,
    pub link: Option<String>,
    /// Why it matters, in one sentence.
    pub why: String,
    pub evidence: Vec<Evidence>,
    /// When the last real message was written: the case's age counts from here.
    pub last_activity_at: DateTime<Utc>,
    /// The deadline a promise named, when it named one that has passed.
    pub deadline: Option<chrono::NaiveDate>,
    /// Message-ID of the last message, for a reply that threads under it.
    pub reply_to_message_id: Option<String>,
    pub space_id: Option<String>,
    pub kwant_id: Option<String>,
}

impl Candidate {
    pub fn age_days(&self, now: DateTime<Utc>) -> i64 {
        (now - self.last_activity_at).num_days().max(0)
    }
}

/// A place conversations come from. Read-only: no implementation may change anything there.
#[async_trait::async_trait]
pub trait MissionSource: Send + Sync {
    fn kind(&self) -> SourceKind;
    fn id(&self) -> &str;
    /// Conversations with activity since `since`. `Err` is a short reason for the person, never
    /// message text.
    async fn conversations(&self, since: DateTime<Utc>) -> Result<Vec<Conversation>, String>;
}

/// Cuts `s` to at most `max` characters on a character boundary.
pub fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// Whitespace collapsed to single spaces, for comparing a quote with its source.
pub fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `quote` appears in `text` word for word (whitespace and case of letters aside).
pub fn quote_in(text: &str, quote: &str) -> bool {
    let q = squash(quote).to_lowercase();
    !q.is_empty() && squash(text).to_lowercase().contains(&q)
}
