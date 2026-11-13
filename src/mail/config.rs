//! Bot email settings as stored (never the password), the providers setup offers, and every
//! rule a setting must pass. The provider list is code, so a later provider (kwant.si) is one
//! more entry, not a new screen.

use super::safety::{base_address, domain_of, has_control_chars, parse_addresses};
use crate::zombie::imap::{is_loopback, Security};
use serde::{Deserialize, Serialize};

/// The vault owner the bot mailbox password is saved under.
pub const VAULT_OWNER: &str = "kwnt-bot-email";

pub const DEFAULT_SENDS_PER_HOUR: u32 = 30;
pub const DEFAULT_RECEIVED_PER_SENDER_PER_HOUR: u32 = 20;
/// Minutes an ask waits unanswered in the app before it goes by email (spec ENG-1).
pub const ASK_DELAYS: &[u32] = &[0, 10, 60];
pub const DEFAULT_ASK_DELAY: u32 = 10;
/// Default and longest `mail_wait_code` wait, in seconds (spec ENG-16).
pub const DEFAULT_WAIT_SECS: u64 = 300;
pub const MAX_WAIT_SECS: u64 = 900;
pub const MAX_SEND_LIST: usize = 10;
pub const MAX_AUTHSERV: usize = 10;
/// The validation error for a send list holding the bot mailbox (setup maps it to `same_mailbox`).
pub const SAME_MAILBOX: &str = "where bots reach you must be another address than the bot mailbox";

/// One field the setup screen shows for a provider.
#[derive(Debug, Clone, Serialize)]
pub struct Field {
    pub name: &'static str,
    pub label: &'static str,
    /// `email`, `password`, `host`, `port`, `security`, `list`.
    pub kind: &'static str,
}

/// A provider setup offers, with its server settings filled in.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderInfo {
    pub id: &'static str,
    pub label: &'static str,
    /// IMAP and SMTP `(host, port, security)`; empty host: the person enters them.
    pub imap: (&'static str, u16, Security),
    pub smtp: (&'static str, u16, Security),
    /// Accept the server's own certificate (Proton Bridge, on this computer only).
    pub accept_invalid_certs: bool,
    /// Each use gets its own `+tag` address. Without: one shared address, codes matched by the
    /// sender's domain, and only a mailbox just for the bots.
    pub plus_addresses: bool,
    /// The receiving server's authserv-ids, whose DMARC results count. Unverified until QA
    /// with a real mailbox (spec §8).
    pub authserv: &'static [&'static str],
    /// Remove spaces from the password (Gmail shows app passwords in groups of four).
    pub strip_password_spaces: bool,
    pub password_label: &'static str,
    pub password_help: &'static str,
    /// Runs on the person's own computer: only offered in the desktop app (spec E10).
    pub local_only: bool,
    pub fields: &'static [Field],
}

const SIGN_IN: &[Field] = &[
    Field {
        name: "address",
        label: "Bot mailbox address",
        kind: "email",
    },
    Field {
        name: "password",
        label: "App password",
        kind: "password",
    },
    Field {
        name: "send_to",
        label: "Where bots reach you",
        kind: "list",
    },
];
const SIGN_IN_OTHER: &[Field] = &[
    Field {
        name: "address",
        label: "Bot mailbox address",
        kind: "email",
    },
    Field {
        name: "username",
        label: "User name (if not the address)",
        kind: "email",
    },
    Field {
        name: "password",
        label: "Password",
        kind: "password",
    },
    Field {
        name: "imap_host",
        label: "Incoming (IMAP) server",
        kind: "host",
    },
    Field {
        name: "imap_port",
        label: "IMAP port",
        kind: "port",
    },
    Field {
        name: "imap_security",
        label: "IMAP security",
        kind: "security",
    },
    Field {
        name: "smtp_host",
        label: "Outgoing (SMTP) server",
        kind: "host",
    },
    Field {
        name: "smtp_port",
        label: "SMTP port",
        kind: "port",
    },
    Field {
        name: "smtp_security",
        label: "SMTP security",
        kind: "security",
    },
    Field {
        name: "authserv",
        label: "Trusted Authentication-Results server",
        kind: "list",
    },
    Field {
        name: "send_to",
        label: "Where bots reach you",
        kind: "list",
    },
];

pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        id: "gmail",
        label: "Gmail",
        imap: ("imap.gmail.com", 993, Security::Tls),
        smtp: ("smtp.gmail.com", 465, Security::Tls),
        accept_invalid_certs: false,
        plus_addresses: true,
        authserv: &["mx.google.com"],
        strip_password_spaces: true,
        password_label: "App password",
        password_help: "https://myaccount.google.com/apppasswords",
        local_only: false,
        fields: SIGN_IN,
    },
    ProviderInfo {
        id: "fastmail",
        label: "Fastmail",
        imap: ("imap.fastmail.com", 993, Security::Tls),
        smtp: ("smtp.fastmail.com", 465, Security::Tls),
        accept_invalid_certs: false,
        plus_addresses: true,
        authserv: &["*.messagingengine.com"],
        strip_password_spaces: true,
        password_label: "App password",
        password_help: "https://app.fastmail.com/settings/security/apps",
        local_only: false,
        fields: SIGN_IN,
    },
    ProviderInfo {
        id: "icloud",
        label: "iCloud",
        imap: ("imap.mail.me.com", 993, Security::Tls),
        smtp: ("smtp.mail.me.com", 587, Security::Starttls),
        accept_invalid_certs: false,
        plus_addresses: false,
        authserv: &["*.icloud.com", "*.me.com"],
        strip_password_spaces: false,
        password_label: "App password",
        password_help: "https://account.apple.com",
        local_only: false,
        fields: SIGN_IN,
    },
    ProviderInfo {
        id: "proton_bridge",
        label: "Proton Bridge",
        imap: ("127.0.0.1", 1143, Security::Starttls),
        smtp: ("127.0.0.1", 1025, Security::Starttls),
        accept_invalid_certs: true,
        plus_addresses: true,
        authserv: &["mail.protonmail.ch", "*.protonmail.ch"],
        strip_password_spaces: false,
        password_label: "Bridge password",
        password_help: "",
        local_only: true,
        fields: SIGN_IN,
    },
    ProviderInfo {
        id: "other",
        label: "Other",
        imap: ("", 993, Security::Tls),
        smtp: ("", 465, Security::Tls),
        accept_invalid_certs: false,
        plus_addresses: true,
        authserv: &[],
        strip_password_spaces: false,
        password_label: "Password",
        password_help: "",
        local_only: false,
        fields: SIGN_IN_OTHER,
    },
];

pub fn provider(id: &str) -> Option<&'static ProviderInfo> {
    PROVIDERS.iter().find(|p| p.id == id)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Server {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub security: Security,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mailbox {
    /// A mailbox just for the bots (recommended): kwant may read all its new mail.
    #[default]
    Bots,
    /// The person's own mailbox: kwant reads only mail sent to its `+kwant-` addresses, and
    /// email asks are off (a reply from the same mailbox can't be proven, spec E9).
    Own,
}

/// The bot mailbox as stored. Never the password: that is in the vault.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MailConfig {
    pub provider: String,
    #[serde(default)]
    pub mailbox: Mailbox,
    /// The bot mailbox's address.
    pub address: String,
    /// The sign-in name when it isn't the address.
    #[serde(default)]
    pub username: String,
    pub imap: Server,
    pub smtp: Server,
    #[serde(default)]
    pub accept_invalid_certs: bool,
    #[serde(default = "yes")]
    pub plus_addresses: bool,
    /// The person said this mailbox is only for the bots (required without plus-addresses).
    #[serde(default)]
    pub only_for_bots: bool,
    /// Where bots may send mail, and whose replies count.
    pub send_to: Vec<String>,
    /// authserv-ids whose DMARC results count (`*.example.com` matches subdomains).
    #[serde(default)]
    pub authserv: Vec<String>,
    /// Questions also go by email (needs the reply check).
    #[serde(default)]
    pub asks: bool,
    #[serde(default = "default_delay")]
    pub ask_delay_minutes: u32,
    /// The setup's reply check passed: a reply from the send list proved itself (DMARC).
    #[serde(default)]
    pub reply_checked: bool,
    /// When asks were last turned on: only questions asked since then go by email, so turning
    /// asks on never mails a backlog.
    #[serde(default)]
    pub asks_since: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default = "default_sends")]
    pub sends_per_hour: u32,
    #[serde(default = "default_received")]
    pub received_per_sender_per_hour: u32,
}

fn yes() -> bool {
    true
}
fn default_delay() -> u32 {
    DEFAULT_ASK_DELAY
}
fn default_sends() -> u32 {
    DEFAULT_SENDS_PER_HOUR
}
fn default_received() -> u32 {
    DEFAULT_RECEIVED_PER_SENDER_PER_HOUR
}

fn valid_address(a: &str) -> bool {
    let a = a.trim();
    !a.is_empty()
        && a.len() <= 254
        && !has_control_chars(a)
        && !a.contains(['"', '\\', ' ', '(', ')'])
        && parse_addresses(a).len() == 1
        && {
            let d = domain_of(a);
            !d.is_empty() && d.contains('.') && a.find('@').is_some_and(|i| i > 0)
        }
}

fn valid_host(h: &str) -> bool {
    let h = h.trim();
    !h.is_empty()
        && h.len() <= 253
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(c))
}

fn valid_authserv(a: &str) -> bool {
    let a = a.strip_prefix("*.").unwrap_or(a);
    !a.is_empty()
        && a.len() <= 253
        && a.contains('.')
        && a.chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
}

impl MailConfig {
    /// A config from a provider's preset, before the person's own fields.
    pub fn preset(p: &ProviderInfo, address: &str, send_to: Vec<String>) -> Self {
        MailConfig {
            provider: p.id.into(),
            mailbox: Mailbox::Bots,
            address: address.trim().to_string(),
            username: String::new(),
            imap: Server {
                host: p.imap.0.into(),
                port: p.imap.1,
                security: p.imap.2,
            },
            smtp: Server {
                host: p.smtp.0.into(),
                port: p.smtp.1,
                security: p.smtp.2,
            },
            accept_invalid_certs: p.accept_invalid_certs,
            plus_addresses: p.plus_addresses,
            only_for_bots: false,
            send_to,
            authserv: p.authserv.iter().map(|s| s.to_string()).collect(),
            asks: false,
            ask_delay_minutes: DEFAULT_ASK_DELAY,
            reply_checked: false,
            asks_since: None,
            sends_per_hour: DEFAULT_SENDS_PER_HOUR,
            received_per_sender_per_hour: DEFAULT_RECEIVED_PER_SENDER_PER_HOUR,
        }
    }

    /// Settles a config from the setup screen: a preset's server settings win over the ones
    /// sent, so only "Other" has hand-entered servers; then [`validate`](Self::validate).
    pub fn settle(mut self) -> Result<Self, String> {
        let p = provider(&self.provider).ok_or("unknown provider")?;
        if p.id != "other" {
            let send_to = std::mem::take(&mut self.send_to);
            let keep = MailConfig::preset(p, &self.address, send_to);
            self = MailConfig {
                mailbox: self.mailbox,
                username: self.username,
                only_for_bots: self.only_for_bots,
                asks: self.asks,
                ask_delay_minutes: self.ask_delay_minutes,
                reply_checked: self.reply_checked,
                asks_since: self.asks_since,
                sends_per_hour: self.sends_per_hour,
                received_per_sender_per_hour: self.received_per_sender_per_hour,
                // Proton Bridge's ports can differ from its defaults; its host can't
                imap: if p.local_only {
                    Server {
                        port: self.imap.port,
                        ..keep.imap.clone()
                    }
                } else {
                    keep.imap.clone()
                },
                smtp: if p.local_only {
                    Server {
                        port: self.smtp.port,
                        ..keep.smtp.clone()
                    }
                } else {
                    keep.smtp.clone()
                },
                ..keep
            };
        }
        self.address = self.address.trim().to_string();
        self.username = self.username.trim().to_string();
        self.send_to = self
            .send_to
            .iter()
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect();
        self.authserv = self
            .authserv
            .iter()
            .map(|a| a.trim().to_ascii_lowercase())
            .filter(|a| !a.is_empty())
            .collect();
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), String> {
        let p = provider(&self.provider).ok_or("unknown provider")?;
        if !valid_address(&self.address) {
            return Err("the bot mailbox address isn't an email address".into());
        }
        if !self.username.is_empty()
            && (self.username.len() > 254 || has_control_chars(&self.username))
        {
            return Err("the user name is not valid".into());
        }
        for (what, s) in [("IMAP", &self.imap), ("SMTP", &self.smtp)] {
            if !valid_host(&s.host) {
                return Err(format!(
                    "the {what} server must be a host name like imap.example.org"
                ));
            }
            if s.port == 0 {
                return Err(format!("the {what} port must be between 1 and 65535"));
            }
            if (s.security == Security::Plain || self.accept_invalid_certs) && !is_loopback(&s.host)
            {
                return Err("an unencrypted or unverified connection is only allowed to this computer (127.0.0.1)".into());
            }
        }
        if self.send_to.is_empty() {
            return Err("add at least one address where bots reach you".into());
        }
        if self.send_to.len() > MAX_SEND_LIST || self.send_to.iter().any(|a| !valid_address(a)) {
            return Err(format!(
                "where bots reach you: up to {MAX_SEND_LIST} email addresses"
            ));
        }
        // A reply from the bot mailbox itself proves nothing (spec E9)
        if self.mailbox == Mailbox::Bots
            && self
                .send_to
                .iter()
                .any(|a| base_address(a) == base_address(&self.address))
        {
            return Err(SAME_MAILBOX.into());
        }
        if self.authserv.len() > MAX_AUTHSERV || self.authserv.iter().any(|a| !valid_authserv(a)) {
            return Err(
                "trusted servers must be host names like mx.example.org or *.example.org".into(),
            );
        }
        if !self.plus_addresses && self.mailbox == Mailbox::Own {
            return Err(format!("{} has no +addresses, so kwant would read all your new mail: use a mailbox just for the bots", p.label));
        }
        if !self.plus_addresses && !self.only_for_bots {
            return Err("confirm that this mailbox is only for the bots".into());
        }
        if !ASK_DELAYS.contains(&self.ask_delay_minutes) {
            return Err("send asks right away, after 10 minutes or after 1 hour".into());
        }
        if self.asks && (self.mailbox == Mailbox::Own || !self.reply_checked) {
            return Err(
                "email asks turn on after the reply check passes, with a mailbox just for the bots"
                    .into(),
            );
        }
        if !(1..=200).contains(&self.sends_per_hour)
            || !(1..=200).contains(&self.received_per_sender_per_hour)
        {
            return Err("hourly limits must be between 1 and 200".into());
        }
        Ok(())
    }

    pub fn login(&self) -> &str {
        if self.username.is_empty() {
            &self.address
        } else {
            &self.username
        }
    }

    /// The vault origin the password is saved under (the username is the space id).
    pub fn vault_origin(&self) -> String {
        format!(
            "https://{}:{}",
            self.imap.host.trim().to_ascii_lowercase(),
            self.imap.port
        )
    }

    /// The password as the provider wants it (Gmail: no spaces).
    pub fn clean_password(&self, password: &str) -> String {
        match provider(&self.provider) {
            Some(p) if p.strip_password_spaces => {
                password.chars().filter(|c| !c.is_whitespace()).collect()
            }
            _ => password.trim_end_matches(['\r', '\n']).to_string(),
        }
    }

    /// Whether `address` is one of the send list (by base address).
    pub fn on_send_list(&self, address: &str) -> bool {
        let b = base_address(address);
        self.send_to.iter().any(|s| base_address(s) == b)
    }
}

/// A bot's tag part: its name in lowercase letters and digits, up to 16 (spec ENG-5).
pub fn bot_slug(name: &str) -> String {
    let mut s = String::new();
    let mut dash = false;
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            s.push(c);
            dash = false;
        } else if !s.is_empty() && !dash {
            s.push('-');
            dash = true;
        }
        if s.len() >= 16 {
            break;
        }
    }
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() {
        "bot".into()
    } else {
        s
    }
}

/// `kwant-<slug>-<5 random lowercase letters and digits>`.
pub fn new_tag(slug: &str) -> String {
    const ALPHABET: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
    let mut bytes = [0u8; 5];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .unwrap_or_default();
    let r: String = bytes
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect();
    format!("kwant-{slug}-{r}")
}

/// Whether a `+tag` is one of kwant's.
pub fn is_kwant_tag(tag: &str) -> bool {
    let Some(rest) = tag.strip_prefix("kwant-") else {
        return false;
    };
    rest.len() >= 7
        && rest.len() <= 23
        && rest
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gmail() -> MailConfig {
        MailConfig::preset(
            provider("gmail").unwrap(),
            "sam.bots@gmail.com",
            vec!["sam@example.com".into()],
        )
    }

    #[test]
    fn presets_fill_the_servers_and_the_trusted_results() {
        let c = gmail().settle().unwrap();
        assert_eq!(
            c.imap,
            Server {
                host: "imap.gmail.com".into(),
                port: 993,
                security: Security::Tls
            }
        );
        assert_eq!(c.smtp.port, 465);
        assert_eq!(c.authserv, vec!["mx.google.com".to_string()]);
        // A preset's servers can't be changed from the setup screen
        let mut c = gmail();
        c.imap.host = "evil.example".into();
        c.authserv = vec!["evil.example".into()];
        let c = c.settle().unwrap();
        assert_eq!(c.imap.host, "imap.gmail.com");
        assert_eq!(c.authserv, vec!["mx.google.com".to_string()]);
        assert!(PROVIDERS.iter().all(|p| !p.fields.is_empty()));
    }

    #[test]
    fn plain_or_unverified_only_to_this_computer() {
        let mut c = MailConfig::preset(
            provider("other").unwrap(),
            "bots@example.org",
            vec!["me@example.org".into()],
        );
        c.imap.host = "mail.example.org".into();
        c.smtp.host = "mail.example.org".into();
        assert!(c.clone().settle().is_ok());
        c.accept_invalid_certs = true;
        assert!(c.clone().settle().is_err());
        c.accept_invalid_certs = false;
        c.smtp.security = Security::Plain;
        assert!(c.clone().settle().is_err());
        let p = MailConfig::preset(
            provider("proton_bridge").unwrap(),
            "me@proton.me",
            vec!["me@example.org".into()],
        );
        let p = p.settle().unwrap();
        assert!(p.accept_invalid_certs && p.imap.host == "127.0.0.1");
    }

    #[test]
    fn icloud_needs_a_mailbox_just_for_the_bots() {
        let mut c = MailConfig::preset(
            provider("icloud").unwrap(),
            "sam.bots@icloud.com",
            vec!["sam@example.com".into()],
        );
        assert!(c
            .clone()
            .settle()
            .unwrap_err()
            .contains("only for the bots"));
        c.only_for_bots = true;
        assert!(c.clone().settle().is_ok());
        c.mailbox = Mailbox::Own;
        assert!(c.settle().unwrap_err().contains("+addresses"));
    }

    #[test]
    fn the_send_list_and_the_asks_rules() {
        let mut c = gmail();
        c.send_to = vec![];
        assert!(c.clone().settle().is_err());
        c.send_to = vec!["a@b.example, c@d.example".into()];
        assert!(c.clone().settle().is_err());
        c.send_to = vec!["Sam <sam@example.com>".into()];
        assert!(c.clone().settle().is_err(), "a bare address only");
        c.send_to = vec!["sam@example.com".into()];
        c.asks = true;
        assert!(c.clone().settle().is_err(), "asks need the reply check");
        c.reply_checked = true;
        assert!(c.clone().settle().is_ok());
        c.mailbox = Mailbox::Own;
        assert!(
            c.clone().settle().is_err(),
            "asks need a mailbox just for the bots"
        );
        let mut c = gmail();
        c.ask_delay_minutes = 5;
        assert!(c.settle().is_err());
        let mut c = gmail();
        c.send_to = vec!["Sam.Bots+x@gmail.com".into()];
        assert_eq!(
            c.clone().settle().unwrap_err(),
            SAME_MAILBOX,
            "a reply from the bot mailbox proves nothing (E9)"
        );
        c.mailbox = Mailbox::Own;
        assert!(
            c.settle().is_ok(),
            "the own-mailbox mode sends codes only, so it may mail itself"
        );
        assert!(gmail().on_send_list("SAM+x@Example.com"));
        assert!(!gmail().on_send_list("bob@example.com"));
    }

    #[test]
    fn passwords_and_tags() {
        assert_eq!(
            gmail().clean_password("abcd efgh ijkl mnop"),
            "abcdefghijklmnop"
        );
        assert_eq!(bot_slug("Drafter"), "drafter");
        assert_eq!(bot_slug("Book Creator for KDP!"), "book-creator-for");
        assert_eq!(bot_slug("✨"), "bot");
        let t = new_tag("drafter");
        assert!(
            t.starts_with("kwant-drafter-") && t.len() == "kwant-drafter-".len() + 5,
            "{t}"
        );
        assert!(is_kwant_tag(&t));
        assert_ne!(new_tag("x"), new_tag("x"));
        assert!(!is_kwant_tag("kwant-"));
        assert!(!is_kwant_tag("news"));
        assert!(!is_kwant_tag("kwant-A\"b"));
    }
}
