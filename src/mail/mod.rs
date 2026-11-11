//! Email for bots (0.10.0): a sign-up address per use, sign-in codes and links, and questions to
//! the person by email. The behaviour and safety rules are ported from ClankerInbox (MIT, Danny
//! Postma, github.com/ClankerInbox/clankerinbox); see docs/bot-email.md.
//!
//! Mail is untrusted, like a web page. Only a code or a link leaves a sign-up address; an answer
//! counts only from the send list with DMARC passed aligned; mail goes only to the send list.
//!
//! A [`MailProvider`] only moves mail: it hands out addresses, hands back raw RFC 5322 bytes and
//! sends a finished message. Every decision about what is safe is made here in the core
//! ([`parse`], [`flow`], [`safety`], and the caps in the server), the same for every provider.

pub mod auth_results;
pub mod config;
pub mod flow;
pub mod html;
pub mod imap_smtp;
pub mod inbox;
pub mod outgoing;
pub mod parse;
pub mod safety;

use chrono::{DateTime, Utc};
use std::collections::HashSet;

/// One received mail as the provider has it: raw bytes, never parsed or judged by the provider.
#[derive(Debug, Clone)]
pub struct RawMail {
    /// Stable across reads (`INBOX:<uidvalidity>:<uid>` for IMAP), so a mail is handled once.
    pub key: String,
    pub bytes: Vec<u8>,
    pub received: Option<DateTime<Utc>>,
}

/// What to fetch: mail since `since`, sent to one of `addresses` when given (a provider that
/// can search by recipient should, so the person's other mail is never read), minus `skip`.
#[derive(Debug, Clone, Default)]
pub struct FetchQuery {
    pub since: Option<DateTime<Utc>>,
    pub addresses: Option<Vec<String>>,
    pub skip: HashSet<String>,
}

/// A finished message the core built and checked (send list, caps, headers). The provider only
/// delivers it.
#[derive(Debug, Clone)]
pub struct OutgoingMail {
    /// The envelope sender (the bot mailbox) and the recipients, for the SMTP envelope.
    pub envelope_from: String,
    pub envelope_to: Vec<String>,
    /// RFC 5322 bytes with every header, `Auto-Submitted` and `X-Loop` included.
    pub bytes: Vec<u8>,
}

/// A provider's failure, with a code the setup screen maps to its words (spec E1–E12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailError {
    /// `wrong_password`, `app_password_required`, `unreachable`, `send_failed`, `receive_failed`,
    /// `config`.
    pub code: &'static str,
    /// The server's text, cut and cleaned: untrusted, shown as plain text.
    pub message: String,
}

impl MailError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        let m: String = safety::clean_line(&message.into());
        MailError {
            code,
            message: m.chars().take(160).collect(),
        }
    }
}

impl std::fmt::Display for MailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

/// A mailbox the bots use. IMAP+SMTP in phase 1; a hosted provider later (addresses assigned by
/// the service, a token instead of a password, fetching over HTTPS) fits the same calls.
#[async_trait::async_trait]
pub trait MailProvider: Send + Sync {
    /// The provider id in [`config::PROVIDERS`].
    fn kind(&self) -> &'static str;
    /// The mailbox's own addresses: mail from them is never an answer.
    fn own_addresses(&self) -> Vec<String>;
    /// Whether each new address is its own (plus-addresses, assigned addresses), or every use
    /// shares one address (iCloud): then codes are matched by the sender's domain.
    fn tagged(&self) -> bool;
    /// The authserv-ids whose `Authentication-Results` the core may trust in this provider's
    /// mail (the receiving server's own). The core still parses and judges them.
    fn trusted_authserv(&self) -> Vec<String>;
    /// A new full address for one use. `tag_hint` is kwant's tag (`kwant-<bot>-<5>`); a
    /// provider may use it (IMAP: `local+tag@domain`) or assign its own address.
    async fn new_address(&self, tag_hint: &str) -> Result<String, MailError>;
    /// Raw mail matching `q`, read without changing anything on the server.
    async fn fetch(&self, q: &FetchQuery) -> Result<Vec<RawMail>, MailError>;
    /// Delivers a finished message.
    async fn send(&self, m: &OutgoingMail) -> Result<(), MailError>;
    /// Signs in to receive and to send, and signs out.
    async fn check(&self) -> Result<(), MailError>;
}
