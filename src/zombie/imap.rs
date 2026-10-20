//! A mailbox over IMAP, read-only: folders are opened with EXAMINE (the server refuses any
//! change in that mode), messages are read with BODY.PEEK (no \Seen flag), and nothing ever
//! sends STORE, COPY, MOVE, EXPUNGE or APPEND. Works with any provider that has IMAP: Gmail
//! (app password), Fastmail, iCloud, Outlook, Proton Bridge on 127.0.0.1.
//!
//! The password comes from the password vault at scan time and is never logged, put in an
//! error, or written anywhere else. Message text is never logged either: errors name the
//! folder and the IMAP step, nothing more.

use super::{clip, squash, ConvKind, Conversation, MissionSource, Msg, SourceKind, MAX_MESSAGE_CHARS};
use chrono::{DateTime, Duration, TimeZone, Utc};
use futures::StreamExt;
use mail_parser::{Address, HeaderValue, MessageParser};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use zeroize::Zeroizing;

/// Most messages read per folder per scan (the newest ones).
pub const MAX_PER_FOLDER: usize = 500;
/// Larger messages are read by their headers only: their text is likely attachments.
pub const MAX_FULL_BYTES: u32 = 512 * 1024;
const CONNECT_TIMEOUT_SECS: u64 = 20;
const SCAN_TIMEOUT_SECS: u64 = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Security {
    /// TLS from the first byte (port 993).
    #[default]
    Tls,
    /// Plain connection upgraded with STARTTLS (port 143).
    Starttls,
    /// No encryption: only to this computer (Proton Bridge, a test server).
    Plain,
}

/// A mailbox's settings as stored: never the password.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImapConfig {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub security: Security,
    pub username: String,
    /// Folders to read; empty reads INBOX and the Sent folder the server marks (or names) as such.
    #[serde(default)]
    pub folders: Vec<String>,
    /// Accept a self-signed certificate: only for a server on this computer (Proton Bridge).
    #[serde(default)]
    pub accept_invalid_certs: bool,
}

fn default_port() -> u16 {
    993
}

pub fn is_loopback(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost") || h.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

impl ImapConfig {
    pub fn validate(&self) -> Result<(), String> {
        let host = self.host.trim();
        if host.is_empty() || host.len() > 253 || !host.chars().all(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(c)) {
            return Err("host must be a host name like imap.gmail.com".into());
        }
        if self.port == 0 {
            return Err("port must be between 1 and 65535".into());
        }
        if self.username.trim().is_empty() || self.username.len() > 254 || self.username.chars().any(char::is_control) {
            return Err("username is required".into());
        }
        if (self.security == Security::Plain || self.accept_invalid_certs) && !is_loopback(host) {
            return Err("an unencrypted or unverified connection is only allowed to this computer (127.0.0.1)".into());
        }
        if self.folders.len() > 10 || self.folders.iter().any(|f| f.is_empty() || f.len() > 200 || f.chars().any(|c| c.is_control() || c == '"' || c == '\\')) {
            return Err("folders must be up to 10 folder names".into());
        }
        Ok(())
    }

    /// The vault origin the password is saved under (the vault keys logins by origin).
    pub fn vault_origin(&self) -> String {
        format!("https://{}:{}", self.host.trim().to_ascii_lowercase(), self.port)
    }
}

/// What a connection test found.
#[derive(Debug, Clone, Serialize)]
pub struct TestReport {
    pub folders: Vec<String>,
    /// The folders a scan would read.
    pub scanning: Vec<String>,
    pub inbox_messages: u32,
}

pub struct ImapSource {
    id: String,
    cfg: ImapConfig,
    password: Zeroizing<String>,
    /// The person's addresses: the login and any the settings add, lowercased.
    me: Vec<String>,
}

impl ImapSource {
    pub fn new(id: &str, cfg: ImapConfig, password: Zeroizing<String>, extra_addresses: &[String]) -> Self {
        let mut me: Vec<String> = extra_addresses.iter().map(|a| a.trim().to_lowercase()).collect();
        if cfg.username.contains('@') {
            me.push(cfg.username.trim().to_lowercase());
        }
        Self { id: id.into(), cfg, password, me }
    }

    pub async fn test(&self) -> Result<TestReport, String> {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            let mut session = self.connect().await?;
            let (all, scanning) = folders_to_scan(&mut session, &self.cfg.folders).await?;
            let inbox = session.examine("INBOX").await.map_err(|e| imap_error("EXAMINE INBOX", &e))?;
            let _ = session.logout().await;
            Ok(TestReport { folders: all, scanning: scanning.into_iter().map(|(f, _)| f).collect(), inbox_messages: inbox.exists })
        })
        .await
        .map_err(|_| "the mail server did not answer in time".to_string())?
    }

    async fn connect(&self) -> Result<async_imap::Session<Box<dyn Io>>, String> {
        let host = self.cfg.host.trim().to_string();
        let tcp = tokio::time::timeout(
            std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS),
            tokio::net::TcpStream::connect((host.as_str(), self.cfg.port)),
        )
        .await
        .map_err(|_| format!("could not reach {host}:{}: timed out", self.cfg.port))?
        .map_err(|e| format!("could not reach {host}:{}: {e}", self.cfg.port))?;
        let stream: Box<dyn Io> = match self.cfg.security {
            Security::Tls => Box::new(tls(&host, tcp, self.cfg.accept_invalid_certs).await?),
            Security::Plain => Box::new(tcp),
            Security::Starttls => {
                let mut client = async_imap::Client::new(tcp);
                client.read_response().await.map_err(|e| format!("the server sent no greeting: {e}"))?;
                client.run_command_and_check_ok("STARTTLS", None).await.map_err(|e| imap_error("STARTTLS", &e))?;
                Box::new(tls(&host, client.into_inner(), self.cfg.accept_invalid_certs).await?)
            }
        };
        let mut client = async_imap::Client::new(stream);
        if self.cfg.security != Security::Starttls {
            client.read_response().await.map_err(|e| format!("the server sent no greeting: {e}"))?;
        }
        client
            .login(self.cfg.username.trim(), self.password.as_str())
            .await
            // The server's answer, never the password
            .map_err(|(e, _)| format!("sign-in to {host} was refused: {}", short(&e.to_string())))
    }

    async fn read_all(&self, since: DateTime<Utc>) -> Result<Vec<RawMail>, String> {
        let mut session = self.connect().await?;
        let (_, folders) = folders_to_scan(&mut session, &self.cfg.folders).await?;
        let mut out = Vec::new();
        for (folder, sent) in folders {
            // EXAMINE: the folder opens read-only, so not even \Recent changes
            if let Err(e) = session.examine(&folder).await {
                tracing::warn!("zombie: imap source {}: EXAMINE failed for one folder: {}", self.id, short(&e.to_string()));
                continue;
            }
            let query = format!("SINCE {}", since.format("%-d-%b-%Y"));
            let mut uids: Vec<u32> = session.uid_search(&query).await.map_err(|e| imap_error("UID SEARCH", &e))?.into_iter().collect();
            uids.sort_unstable();
            let uids: Vec<u32> = uids.into_iter().rev().take(MAX_PER_FOLDER).collect();
            if uids.is_empty() {
                continue;
            }
            let mut sizes: Vec<(u32, u32)> = Vec::new();
            for chunk in uids.chunks(100) {
                let set = uid_set(chunk);
                let mut stream = session.uid_fetch(&set, "(UID RFC822.SIZE)").await.map_err(|e| imap_error("UID FETCH", &e))?;
                while let Some(f) = stream.next().await {
                    let f = f.map_err(|e| imap_error("UID FETCH", &e))?;
                    if let Some(uid) = f.uid {
                        sizes.push((uid, f.size.unwrap_or(0)));
                    }
                }
            }
            let (small, large): (Vec<_>, Vec<_>) = sizes.into_iter().partition(|(_, size)| *size <= MAX_FULL_BYTES);
            for (uids, query) in [
                (small.iter().map(|(u, _)| *u).collect::<Vec<_>>(), "(UID INTERNALDATE BODY.PEEK[])"),
                (large.iter().map(|(u, _)| *u).collect::<Vec<_>>(), "(UID INTERNALDATE BODY.PEEK[HEADER])"),
            ] {
                for chunk in uids.chunks(25) {
                    let set = uid_set(chunk);
                    let mut stream = session.uid_fetch(&set, query).await.map_err(|e| imap_error("UID FETCH", &e))?;
                    while let Some(f) = stream.next().await {
                        let f = f.map_err(|e| imap_error("UID FETCH", &e))?;
                        let bytes = f.body().or_else(|| f.header()).map(<[u8]>::to_vec);
                        if let Some(bytes) = bytes {
                            out.push(RawMail { bytes, in_sent: sent, received: f.internal_date().map(|d| d.with_timezone(&Utc)) });
                        }
                    }
                }
            }
        }
        let _ = session.logout().await;
        Ok(out)
    }
}

#[async_trait::async_trait]
impl MissionSource for ImapSource {
    fn kind(&self) -> SourceKind {
        SourceKind::Imap
    }

    fn id(&self) -> &str {
        &self.id
    }

    async fn conversations(&self, since: DateTime<Utc>) -> Result<Vec<Conversation>, String> {
        let raw = tokio::time::timeout(std::time::Duration::from_secs(SCAN_TIMEOUT_SECS), self.read_all(since))
            .await
            .map_err(|_| "reading the mailbox took too long".to_string())??;
        Ok(conversations_from_raw(&self.id, &raw, &self.me))
    }
}

/// The stream an IMAP session runs over: TLS or, to this computer only, plain TCP.
pub trait Io: AsyncRead + AsyncWrite + Unpin + Send + std::fmt::Debug {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + std::fmt::Debug> Io for T {}

async fn tls<S: AsyncRead + AsyncWrite + Unpin>(host: &str, stream: S, accept_invalid: bool) -> Result<tokio_rustls::client::TlsStream<S>, String> {
    use tokio_rustls::rustls;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS setup failed: {e}"))?;
    let config = if accept_invalid && is_loopback(host) {
        builder.dangerous().with_custom_certificate_verifier(Arc::new(NoVerify(provider))).with_no_client_auth()
    } else {
        let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        builder.with_root_certificates(roots).with_no_client_auth()
    };
    let name = rustls::pki_types::ServerName::try_from(host.trim_start_matches('[').trim_end_matches(']').to_string())
        .map_err(|_| format!("'{host}' is not a valid server name"))?;
    tokio::time::timeout(
        std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS),
        tokio_rustls::TlsConnector::from(Arc::new(config)).connect(name, stream),
    )
    .await
    .map_err(|_| "the TLS handshake timed out".to_string())?
    .map_err(|e| format!("the TLS connection failed: {e}"))
}

/// Accepts any certificate. Only ever used for a server on 127.0.0.1 (Proton Bridge's own
/// self-signed one), where there is no network to intercept.
#[derive(Debug)]
struct NoVerify(Arc<tokio_rustls::rustls::crypto::CryptoProvider>);

impl tokio_rustls::rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _: &[tokio_rustls::rustls::pki_types::CertificateDer<'_>],
        _: &tokio_rustls::rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: tokio_rustls::rustls::pki_types::UnixTime,
    ) -> Result<tokio_rustls::rustls::client::danger::ServerCertVerified, tokio_rustls::rustls::Error> {
        Ok(tokio_rustls::rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        tokio_rustls::rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        tokio_rustls::rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn short(s: &str) -> String {
    clip(&squash(s), 160)
}

fn imap_error(step: &str, e: &async_imap::error::Error) -> String {
    format!("the mail server refused {step}: {}", short(&e.to_string()))
}

fn uid_set(uids: &[u32]) -> String {
    uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

/// Every folder, and the ones to scan with whether each is a Sent folder.
async fn folders_to_scan(session: &mut async_imap::Session<Box<dyn Io>>, chosen: &[String]) -> Result<(Vec<String>, Vec<(String, bool)>), String> {
    use async_imap::imap_proto::NameAttribute;
    let mut all = Vec::new();
    let mut marked_sent = None;
    {
        let mut names = session.list(Some(""), Some("*")).await.map_err(|e| imap_error("LIST", &e))?;
        while let Some(n) = names.next().await {
            let n = n.map_err(|e| imap_error("LIST", &e))?;
            if n.attributes().iter().any(|a| matches!(a, NameAttribute::NoSelect)) {
                continue;
            }
            if marked_sent.is_none() && n.attributes().iter().any(|a| matches!(a, NameAttribute::Sent)) {
                marked_sent = Some(n.name().to_string());
            }
            all.push(n.name().to_string());
        }
    }
    let sent = marked_sent.or_else(|| {
        const NAMES: [&str; 6] = ["Sent", "Sent Items", "Sent Messages", "Sent Mail", "[Gmail]/Sent Mail", "INBOX.Sent"];
        NAMES.iter().find_map(|want| all.iter().find(|f| f.eq_ignore_ascii_case(want)).cloned())
    });
    let scan: Vec<(String, bool)> = if chosen.is_empty() {
        let mut v = vec![("INBOX".to_string(), false)];
        v.extend(sent.clone().map(|s| (s, true)));
        v
    } else {
        chosen.iter().map(|f| (f.clone(), sent.as_deref() == Some(f.as_str()) || f.to_lowercase().contains("sent"))).collect()
    };
    Ok((all, scan))
}

/// One message as fetched.
#[derive(Debug, Clone)]
pub struct RawMail {
    pub bytes: Vec<u8>,
    /// Read from the Sent folder: the person wrote it.
    pub in_sent: bool,
    /// The server's arrival time, when the Date header is missing.
    pub received: Option<DateTime<Utc>>,
}

/// A parsed message, before threading.
#[derive(Debug, Clone)]
struct Mail {
    message_id: Option<String>,
    parents: Vec<String>,
    subject: String,
    from: String,
    from_name: Option<String>,
    to: Option<String>,
    to_name: Option<String>,
    at: DateTime<Utc>,
    text: String,
    outbound: bool,
    auto: bool,
    bulk: bool,
}

fn ids(v: &HeaderValue) -> Vec<String> {
    match v {
        HeaderValue::Text(t) => vec![t.to_string()],
        HeaderValue::TextList(l) => l.iter().map(|t| t.to_string()).collect(),
        _ => Vec::new(),
    }
}

fn first_addr(a: Option<&Address>) -> (Option<String>, Option<String>) {
    match a.and_then(|a| a.first()) {
        Some(addr) => (addr.address().map(|s| s.trim().to_lowercase()), addr.name().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())),
        None => (None, None),
    }
}

/// A sender no person reads: newsletters, notifications, bounces.
pub fn is_noreply(addr: &str) -> bool {
    let local = addr.split('@').next().unwrap_or("").to_lowercase();
    ["noreply", "no-reply", "no_reply", "donotreply", "do-not-reply", "do_not_reply", "mailer-daemon", "postmaster", "newsletter", "notifications", "notification", "bounce", "news"]
        .iter()
        .any(|p| local == *p || local.starts_with(&format!("{p}-")) || local.starts_with(&format!("{p}+")) || local.starts_with(&format!("{p}.")) || local.contains("noreply") || local.contains("no-reply"))
}

fn parse(raw: &RawMail, me: &[String]) -> Option<Mail> {
    let m = MessageParser::default().parse(&raw.bytes)?;
    let header_text = |name: &str| m.header_raw(name).map(|v| v.trim().to_ascii_lowercase());
    let (from, from_name) = first_addr(m.from());
    let from = from.unwrap_or_default();
    let (to, to_name) = first_addr(m.to());
    let subject = m.subject().unwrap_or("").trim().to_string();
    let lower_subject = subject.to_lowercase();
    let auto_submitted = header_text("Auto-Submitted").is_some_and(|v| v != "no");
    let precedence = header_text("Precedence").unwrap_or_default();
    let auto = auto_submitted
        || m.header_raw("X-Autoreply").is_some()
        || m.header_raw("X-Autorespond").is_some()
        || precedence == "auto_reply"
        || from.starts_with("mailer-daemon@")
        || from.starts_with("postmaster@")
        || ["out of office", "automatic reply", "auto:", "autoreply", "auto-reply", "abwesenheitsnotiz", "undeliverable", "delivery status notification"]
            .iter()
            .any(|p| lower_subject.starts_with(p) || lower_subject.contains(&format!(" {p}")));
    let bulk = m.header_raw("List-Id").is_some()
        || m.header_raw("List-Unsubscribe").is_some()
        || matches!(precedence.as_str(), "bulk" | "list" | "junk")
        || is_noreply(&from);
    let outbound = raw.in_sent || (!from.is_empty() && me.contains(&from));
    let at = m
        .date()
        .and_then(|d| Utc.timestamp_opt(d.to_timestamp(), 0).single())
        .or(raw.received)
        .unwrap_or_else(Utc::now);
    let body = m.body_text(0).map(|b| b.into_owned()).unwrap_or_default();
    let mut parents = ids(m.references());
    for id in ids(m.in_reply_to()) {
        if !parents.contains(&id) {
            parents.push(id);
        }
    }
    Some(Mail {
        message_id: m.message_id().map(str::to_string),
        parents,
        subject,
        from,
        from_name,
        to,
        to_name,
        at,
        text: clip(&strip_quoted(&body), MAX_MESSAGE_CHARS),
        outbound,
        auto,
        bulk,
    })
}

/// The new text of a reply: quoted earlier messages and the signature cut off.
pub fn strip_quoted(body: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let lines: Vec<&str> = body.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t == "--" || t == "-- " || line.starts_with("-- ") && t.len() == 2 {
            break;
        }
        if t.starts_with("-----Original Message-----") || t.starts_with("________________________________") || t.starts_with("-------- Forwarded") {
            break;
        }
        // "On Tue, 3 Sep 2026 at 10:00, Dana <d@x> wrote:", possibly wrapped over two lines
        let joined = match lines.get(i + 1) {
            Some(next) => format!("{t} {}", next.trim()),
            None => t.to_string(),
        };
        if (t.starts_with("On ") || t.starts_with("Am ") || t.starts_with("Le ")) && (t.ends_with("wrote:") || t.ends_with("schrieb:") || t.ends_with("écrit :") || joined.ends_with("wrote:")) {
            break;
        }
        // Outlook's quoted header block
        if t.starts_with("From:") && lines.get(i + 1).is_some_and(|n| n.trim().starts_with("Sent:") || n.trim().starts_with("Date:")) {
            break;
        }
        if t.starts_with('>') {
            continue;
        }
        out.push(line);
    }
    out.join("\n").trim().to_string()
}

/// "Re: Fwd: Claim 4471" → "claim 4471".
fn base_subject(s: &str) -> String {
    let mut s = s.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        let cut = ["re:", "aw:", "sv:", "fw:", "fwd:", "wg:", "tr:"].iter().find(|p| lower.starts_with(**p)).map(|p| p.len());
        match cut {
            Some(n) => s = s[n..].trim_start(),
            None => break,
        }
    }
    squash(s).to_lowercase()
}

/// Threads `raw` messages into conversations, newest activity in each.
pub fn conversations_from_raw(source_id: &str, raw: &[RawMail], me: &[String]) -> Vec<Conversation> {
    let mut mails: Vec<Mail> = raw.iter().filter_map(|r| parse(r, me)).collect();
    // The same message read twice (INBOX and Sent when the person Cc'd themselves)
    mails.sort_by_key(|m| m.at);
    let mut seen = std::collections::HashSet::new();
    mails.retain(|m| m.message_id.as_ref().is_none_or(|id| seen.insert(id.clone())));

    // Union-find over Message-IDs, then by subject + counterpart for clients that drop them
    let mut parent: Vec<usize> = (0..mails.len()).collect();
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while p[r] != r {
            r = p[r];
        }
        let mut i = i;
        while p[i] != r {
            let next = p[i];
            p[i] = r;
            i = next;
        }
        r
    }
    fn union(p: &mut [usize], a: usize, b: usize) {
        let (ra, rb) = (find(p, a), find(p, b));
        if ra != rb {
            p[rb] = ra;
        }
    }
    let mut by_id: HashMap<String, usize> = HashMap::new();
    for (i, m) in mails.iter().enumerate() {
        for id in m.message_id.iter().chain(m.parents.iter()) {
            match by_id.get(id) {
                Some(&j) => union(&mut parent, j, i),
                None => {
                    by_id.insert(id.clone(), i);
                }
            }
        }
    }
    let other = |m: &Mail| if m.outbound { m.to.clone().unwrap_or_default() } else { m.from.clone() };
    let mut by_subject: HashMap<(String, String), usize> = HashMap::new();
    for (i, m) in mails.iter().enumerate() {
        let subject = base_subject(&m.subject);
        if subject.len() < 4 {
            continue;
        }
        match by_subject.get(&(subject.clone(), other(m))) {
            Some(&j) => union(&mut parent, j, i),
            None => {
                by_subject.insert((subject, other(m)), i);
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..mails.len() {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }

    let mut out = Vec::new();
    for members in groups.into_values() {
        let thread: Vec<&Mail> = members.iter().map(|&i| &mails[i]).collect(); // sorted by date already
        let first = thread[0];
        // Stable across scans even when the first message falls out of the window: the root the
        // replies point at
        let root = first.parents.first().or(first.message_id.as_ref()).cloned().unwrap_or_else(|| format!("subject:{}|{}", base_subject(&first.subject), other(first)));
        let key = hex(&ring::digest::digest(&ring::digest::SHA256, root.as_bytes()).as_ref()[..12]);
        let subject = thread.iter().rev().find(|m| !m.auto && !m.subject.is_empty()).map(|m| m.subject.clone()).unwrap_or_else(|| first.subject.clone());
        let mut c = Conversation::new(SourceKind::Imap, source_id, &key, ConvKind::Email, &subject);
        // Who a follow-up goes to: the last real person on the other side
        let them = thread.iter().rev().find(|m| !m.outbound && !m.auto && !m.bulk).map(|m| (m.from.clone(), m.from_name.clone()));
        let (to, name) = match them {
            Some(t) => (Some(t.0), t.1),
            None => thread.iter().rev().find(|m| m.outbound).map(|m| (m.to.clone(), m.to_name.clone())).unwrap_or((None, None)),
        };
        c.counterpart = to.filter(|t| !t.is_empty());
        c.counterpart_name = name;
        c.messages = thread
            .iter()
            .map(|m| Msg { from: m.from.clone(), outbound: m.outbound, at: m.at, text: m.text.clone(), auto: m.auto, bulk: m.bulk && !m.outbound, message_id: m.message_id.as_ref().map(|id| format!("<{id}>")) })
            .collect();
        out.push(c);
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The `since` a scan of `lookback_days` reads from.
pub fn since(now: DateTime<Utc>, lookback_days: i64) -> DateTime<Utc> {
    now - Duration::days(lookback_days)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(s: &str, in_sent: bool) -> RawMail {
        RawMail { bytes: s.replace('\n', "\r\n").into_bytes(), in_sent, received: None }
    }

    #[test]
    fn plain_and_unverified_connections_only_to_this_computer() {
        let mut c = ImapConfig { host: "imap.example.com".into(), port: 993, security: Security::Tls, username: "me@example.com".into(), folders: vec![], accept_invalid_certs: false };
        c.validate().unwrap();
        c.security = Security::Plain;
        assert!(c.validate().is_err());
        c.host = "127.0.0.1".into();
        c.validate().unwrap();
        c.security = Security::Tls;
        c.accept_invalid_certs = true;
        c.validate().unwrap();
        c.host = "evil.example".into();
        assert!(c.validate().is_err());
        c.host = "imap.example.com\r\nA1 STORE".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn quoted_text_and_signatures_are_cut() {
        let body = "Any news?\n\nOn Tue, 3 Sep 2026 at 10:00, Dana <d@x.example> wrote:\n> We will get back to you.\n";
        assert_eq!(strip_quoted(body), "Any news?");
        assert_eq!(strip_quoted("Thanks\n-- \nDana\nCEO"), "Thanks");
        assert_eq!(strip_quoted("Yes.\n> quoted\nMore"), "Yes.\nMore");
        assert_eq!(strip_quoted("Ok\nFrom: A\nSent: Monday\nTo: B"), "Ok");
    }

    #[test]
    fn threads_by_message_id_and_marks_direction() {
        let a = raw("Message-ID: <1@me.example>\nFrom: Me <me@me.example>\nTo: Dana <dana@x.example>\nSubject: Claim 4471\nDate: Mon, 7 Sep 2026 10:00:00 +0000\n\nCould you check claim 4471?\n", true);
        let b = raw("Message-ID: <2@x.example>\nIn-Reply-To: <1@me.example>\nReferences: <1@me.example>\nFrom: Dana <dana@x.example>\nTo: me@me.example\nSubject: Re: Claim 4471\nDate: Tue, 8 Sep 2026 10:00:00 +0000\n\nWe will get back to you within 5 days.\n\nOn Mon, 7 Sep 2026, Me wrote:\n> Could you check claim 4471?\n", false);
        let n = raw("Message-ID: <3@news.example>\nFrom: News <newsletter@news.example>\nList-Unsubscribe: <mailto:u@news.example>\nTo: me@me.example\nSubject: Weekly\nDate: Tue, 8 Sep 2026 10:00:00 +0000\n\nHello\n", false);
        let convs = conversations_from_raw("s1", &[b, n, a], &["me@me.example".into()]);
        assert_eq!(convs.len(), 2);
        let claim = convs.iter().find(|c| c.messages.len() == 2).unwrap();
        assert_eq!(claim.subject, "Re: Claim 4471");
        assert!(claim.messages[0].outbound && !claim.messages[1].outbound);
        assert_eq!(claim.messages[1].text, "We will get back to you within 5 days.");
        assert_eq!(claim.counterpart.as_deref(), Some("dana@x.example"));
        assert_eq!(claim.counterpart_name.as_deref(), Some("Dana"));
        assert_eq!(claim.messages[1].message_id.as_deref(), Some("<2@x.example>"));
        let news = convs.iter().find(|c| c.messages.len() == 1).unwrap();
        assert!(news.messages[0].bulk);
    }

    #[test]
    fn auto_replies_are_marked() {
        let a = raw("Message-ID: <9@x.example>\nFrom: support@x.example\nAuto-Submitted: auto-replied\nSubject: Automatic reply: Refund\nDate: Tue, 8 Sep 2026 10:00:00 +0000\n\nWe received it.\n", false);
        let c = conversations_from_raw("s1", &[a], &[]);
        assert!(c[0].messages[0].auto);
    }

    #[test]
    fn the_thread_key_survives_the_first_message_leaving_the_window() {
        let a = raw("Message-ID: <1@me.example>\nFrom: me@me.example\nTo: d@x.example\nSubject: Hi there\nDate: Mon, 7 Sep 2026 10:00:00 +0000\n\nQ?\n", true);
        let b = raw("Message-ID: <2@x.example>\nReferences: <1@me.example>\nFrom: d@x.example\nTo: me@me.example\nSubject: Re: Hi there\nDate: Tue, 8 Sep 2026 10:00:00 +0000\n\nA.\n", false);
        let both = conversations_from_raw("s1", &[a, b.clone()], &[]);
        let later = conversations_from_raw("s1", &[b], &[]);
        assert_eq!(both[0].key, later[0].key);
    }
}
