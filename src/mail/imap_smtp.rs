//! "A mailbox for your bots": IMAP to read, SMTP to send. Reading follows src/zombie/imap.rs
//! exactly: INBOX is opened with EXAMINE (read-only, the server refuses any change), messages
//! are read with BODY.PEEK (no \Seen flag), and nothing ever sends STORE, COPY, MOVE, EXPUNGE
//! or APPEND, so the mailbox's other mail keeps its flags. Sending is lettre over rustls.
//!
//! This provider only moves mail. Whether a mail is safe, whose it is and what may leave it
//! is decided in the core (super::flow, super::safety).

use super::config::{provider, MailConfig};
use super::safety::{base_address, domain_of, with_tag};
use super::{FetchQuery, MailError, MailProvider, OutgoingMail, RawMail};
use crate::zombie::imap::{connect, imap_error, uid_set, Security, MAX_FULL_BYTES};
use chrono::{Duration, Utc};
use futures::StreamExt;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use std::collections::BTreeSet;
use zeroize::Zeroizing;

/// Most mails read in one fetch (the newest).
pub const MAX_PER_FETCH: usize = 200;
/// A whole fetch, and the SMTP conversation, give up after this.
const FETCH_TIMEOUT_SECS: u64 = 60;
const SMTP_TIMEOUT_SECS: u64 = 15;

pub struct ImapSmtp {
    cfg: MailConfig,
    password: Zeroizing<String>,
}

impl ImapSmtp {
    pub fn new(cfg: MailConfig, password: &str) -> Self {
        let password = Zeroizing::new(cfg.clean_password(password));
        ImapSmtp { cfg, password }
    }

    pub fn config(&self) -> &MailConfig {
        &self.cfg
    }

    fn is_gmail(&self) -> bool {
        self.cfg.provider == "gmail"
    }

    /// Gmail app passwords are 16 letters; anything else is likely the account password.
    fn looks_like_account_password(&self) -> bool {
        self.is_gmail()
            && !(self.password.len() == 16
                && self.password.chars().all(|c| c.is_ascii_alphabetic()))
    }

    async fn session(
        &self,
    ) -> Result<async_imap::Session<Box<dyn crate::zombie::imap::Io>>, MailError> {
        connect(
            &self.cfg.imap.host,
            self.cfg.imap.port,
            self.cfg.imap.security,
            self.cfg.accept_invalid_certs,
            self.cfg.login(),
            &self.password,
        )
        .await
        .map_err(|e| imap_sign_in_error(&e, self.looks_like_account_password()))
    }

    async fn read(&self, q: &FetchQuery) -> Result<Vec<RawMail>, MailError> {
        let mut session = self.session().await?;
        let inbox = session
            .examine("INBOX")
            .await
            .map_err(|e| MailError::new("receive_failed", imap_error("EXAMINE INBOX", &e)))?;
        let validity = inbox.uid_validity.unwrap_or(0);
        let since = q.since.unwrap_or_else(|| Utc::now() - Duration::days(1)) - Duration::days(1);
        let since = since.format("%-d-%b-%Y").to_string();
        let mut queries = Vec::new();
        match &q.addresses {
            // Only mail to kwant's own addresses: the person's other mail is never fetched
            Some(list) => {
                for a in list {
                    let a = a.trim();
                    if !search_safe(a) {
                        continue;
                    }
                    queries.push(format!(
                        "SINCE {since} OR TO \"{a}\" HEADER Delivered-To \"{a}\""
                    ));
                }
            }
            None => queries.push(format!("SINCE {since}")),
        }
        let mut uids = BTreeSet::new();
        for query in &queries {
            let found = session
                .uid_search(query)
                .await
                .map_err(|e| MailError::new("receive_failed", imap_error("UID SEARCH", &e)))?;
            uids.extend(found);
        }
        let key = |uid: u32| format!("INBOX:{validity}:{uid}");
        let uids: Vec<u32> = uids
            .into_iter()
            .rev()
            .filter(|u| !q.skip.contains(&key(*u)))
            .take(MAX_PER_FETCH)
            .collect();
        let mut out = Vec::new();
        if uids.is_empty() {
            let _ = session.logout().await;
            return Ok(out);
        }
        let mut small = Vec::new();
        for chunk in uids.chunks(100) {
            let mut stream = session
                .uid_fetch(uid_set(chunk), "(UID RFC822.SIZE)")
                .await
                .map_err(|e| MailError::new("receive_failed", imap_error("UID FETCH", &e)))?;
            while let Some(f) = stream.next().await {
                let f =
                    f.map_err(|e| MailError::new("receive_failed", imap_error("UID FETCH", &e)))?;
                // Larger mail is attachments, never a code or an answer: not read at all
                if let (Some(uid), Some(size)) = (f.uid, f.size) {
                    if size <= MAX_FULL_BYTES {
                        small.push(uid);
                    }
                }
            }
        }
        for chunk in small.chunks(25) {
            let mut stream = session
                .uid_fetch(uid_set(chunk), "(UID INTERNALDATE BODY.PEEK[])")
                .await
                .map_err(|e| MailError::new("receive_failed", imap_error("UID FETCH", &e)))?;
            while let Some(f) = stream.next().await {
                let f =
                    f.map_err(|e| MailError::new("receive_failed", imap_error("UID FETCH", &e)))?;
                if let (Some(uid), Some(body)) = (f.uid, f.body()) {
                    out.push(RawMail {
                        key: key(uid),
                        bytes: body.to_vec(),
                        received: f.internal_date().map(|d| d.with_timezone(&Utc)),
                    });
                }
            }
        }
        let _ = session.logout().await;
        Ok(out)
    }

    fn transport(&self) -> Result<AsyncSmtpTransport<Tokio1Executor>, MailError> {
        let host = self.cfg.smtp.host.trim();
        let tls = || {
            TlsParameters::builder(host.to_string())
                .dangerous_accept_invalid_certs(
                    self.cfg.accept_invalid_certs && crate::zombie::imap::is_loopback(host),
                )
                .build_rustls()
                .map_err(|e| MailError::new("config", format!("TLS setup failed: {e}")))
        };
        let tls = match self.cfg.smtp.security {
            Security::Tls => Tls::Wrapper(tls()?),
            Security::Starttls => Tls::Required(tls()?),
            // Validated: only to this computer
            Security::Plain if crate::zombie::imap::is_loopback(host) => Tls::None,
            Security::Plain => {
                return Err(MailError::new(
                    "config",
                    "an unencrypted connection is only allowed to this computer",
                ))
            }
        };
        Ok(
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
                .port(self.cfg.smtp.port)
                .credentials(Credentials::new(
                    self.cfg.login().trim().to_string(),
                    self.password.to_string(),
                ))
                .timeout(Some(std::time::Duration::from_secs(SMTP_TIMEOUT_SECS)))
                .tls(tls)
                .build(),
        )
    }

    fn smtp_error(&self, e: &lettre::transport::smtp::Error) -> MailError {
        smtp_error(e, self.looks_like_account_password())
    }
}

/// An address that can go in a quoted IMAP SEARCH string as it is.
fn search_safe(a: &str) -> bool {
    !a.is_empty()
        && a.len() <= 254
        && a.chars()
            .all(|c| c.is_ascii_graphic() && c != '"' && c != '\\')
}

/// Maps a sign-in failure from src/zombie/imap.rs's `connect` to a setup error code.
fn imap_sign_in_error(e: &str, account_password: bool) -> MailError {
    let lower = e.to_ascii_lowercase();
    if lower.starts_with("could not reach")
        || lower.contains("tls")
        || lower.contains("no greeting")
    {
        return MailError::new("unreachable", e);
    }
    if lower.contains("application-specific password")
        || lower.contains("app password")
        || lower.contains("web login required")
        || (account_password && lower.contains("refused"))
    {
        return MailError::new("app_password_required", e);
    }
    if lower.contains("refused") {
        return MailError::new("wrong_password", e);
    }
    MailError::new("receive_failed", e)
}

/// Maps an SMTP failure to a setup error code (spec ENG-3).
fn smtp_error(e: &lettre::transport::smtp::Error, account_password: bool) -> MailError {
    let text = e.to_string();
    let code = e.status().map(|c| c.to_string()).unwrap_or_default();
    if code == "534" || text.contains("5.7.9") {
        return MailError::new("app_password_required", text);
    }
    if code == "535" {
        return MailError::new(
            if account_password {
                "app_password_required"
            } else {
                "wrong_password"
            },
            text,
        );
    }
    if e.is_timeout()
        || (!e.is_response()
            && (text.starts_with("Connection") || text.starts_with("network") || e.is_tls()))
    {
        return MailError::new("unreachable", text);
    }
    MailError::new("send_failed", text)
}

#[async_trait::async_trait]
impl MailProvider for ImapSmtp {
    fn kind(&self) -> &'static str {
        provider(&self.cfg.provider)
            .map(|p| p.id)
            .unwrap_or("other")
    }

    fn own_addresses(&self) -> Vec<String> {
        let mut v = vec![base_address(&self.cfg.address)];
        if self.cfg.username.contains('@') {
            v.push(base_address(&self.cfg.username));
        }
        v
    }

    fn tagged(&self) -> bool {
        self.cfg.plus_addresses
    }

    fn trusted_authserv(&self) -> Vec<String> {
        self.cfg.authserv.clone()
    }

    async fn new_address(&self, tag_hint: &str) -> Result<String, MailError> {
        if !self.cfg.plus_addresses {
            return Ok(self.cfg.address.trim().to_string());
        }
        if domain_of(&self.cfg.address).is_empty() {
            return Err(MailError::new(
                "config",
                "the bot mailbox address isn't an email address",
            ));
        }
        Ok(with_tag(&base_address(&self.cfg.address), tag_hint))
    }

    async fn fetch(&self, q: &FetchQuery) -> Result<Vec<RawMail>, MailError> {
        tokio::time::timeout(
            std::time::Duration::from_secs(FETCH_TIMEOUT_SECS),
            self.read(q),
        )
        .await
        .map_err(|_| MailError::new("unreachable", "the mail server did not answer in time"))?
    }

    async fn send(&self, m: &OutgoingMail) -> Result<(), MailError> {
        let from: Address = m
            .envelope_from
            .parse()
            .map_err(|_| MailError::new("config", "the sender is not an email address"))?;
        let to: Vec<Address> = m
            .envelope_to
            .iter()
            .filter_map(|a| a.parse().ok())
            .collect();
        if to.is_empty() || to.len() != m.envelope_to.len() {
            return Err(MailError::new(
                "send_failed",
                "a recipient is not an email address",
            ));
        }
        let envelope = lettre::address::Envelope::new(Some(from), to)
            .map_err(|e| MailError::new("send_failed", e.to_string()))?;
        self.transport()?
            .send_raw(&envelope, &m.bytes)
            .await
            .map(|_| ())
            .map_err(|e| self.smtp_error(&e))
    }

    async fn check(&self) -> Result<(), MailError> {
        let work = async {
            let mut session = self.session().await?;
            session
                .examine("INBOX")
                .await
                .map_err(|e| MailError::new("receive_failed", imap_error("EXAMINE INBOX", &e)))?;
            let _ = session.logout().await;
            match self.transport()?.test_connection().await {
                Ok(true) => Ok(()),
                Ok(false) => Err(MailError::new(
                    "unreachable",
                    "the outgoing server did not answer",
                )),
                Err(e) => Err(self.smtp_error(&e)),
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS), work)
            .await
            .map_err(|_| MailError::new("unreachable", "the mail server did not answer in time"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::config::provider;

    fn gmail(pw: &str) -> ImapSmtp {
        ImapSmtp::new(
            MailConfig::preset(
                provider("gmail").unwrap(),
                "Sam.Bots@gmail.com",
                vec!["sam@example.com".into()],
            ),
            pw,
        )
    }

    #[tokio::test]
    async fn addresses_are_plus_tags_or_the_one_shared_address() {
        let g = gmail("abcd efgh ijkl mnop");
        assert_eq!(
            g.new_address("kwant-drafter-k7q2m").await.unwrap(),
            "sam.bots+kwant-drafter-k7q2m@gmail.com"
        );
        assert!(g.tagged());
        assert_eq!(g.own_addresses(), vec!["sam.bots@gmail.com".to_string()]);
        assert!(!g.looks_like_account_password(), "spaces are stripped");
        assert!(gmail("hunter2").looks_like_account_password());
        let mut c = MailConfig::preset(
            provider("icloud").unwrap(),
            "sam.bots@icloud.com",
            vec!["sam@example.com".into()],
        );
        c.only_for_bots = true;
        let i = ImapSmtp::new(c, "x");
        assert_eq!(
            i.new_address("kwant-drafter-k7q2m").await.unwrap(),
            "sam.bots@icloud.com"
        );
        assert!(!i.tagged());
    }

    #[test]
    fn sign_in_errors_map_to_setup_codes() {
        assert_eq!(
            imap_sign_in_error("could not reach imap.x:993: timed out", false).code,
            "unreachable"
        );
        assert_eq!(
            imap_sign_in_error(
                "sign-in to imap.gmail.com was refused: [AUTHENTICATIONFAILED] Invalid credentials",
                false
            )
            .code,
            "wrong_password"
        );
        assert_eq!(
            imap_sign_in_error(
                "sign-in to imap.gmail.com was refused: [AUTHENTICATIONFAILED] Invalid credentials",
                true
            )
            .code,
            "app_password_required"
        );
        assert_eq!(imap_sign_in_error("sign-in to imap.gmail.com was refused: [ALERT] Application-specific password required", false).code, "app_password_required");
        let long = format!("sign-in to x was refused: {}\u{7}", "a".repeat(400));
        let e = imap_sign_in_error(&long, false);
        assert!(e.message.chars().count() <= 160 && !e.message.contains('\u{7}'));
    }

    #[test]
    fn only_plain_addresses_go_into_a_search() {
        assert!(search_safe("bots+kwant-a-b2c3d@example.org"));
        assert!(!search_safe("a\" OR ALL \"@x"));
        assert!(!search_safe("a\r\nb@x"));
        assert!(!search_safe(""));
    }
}
