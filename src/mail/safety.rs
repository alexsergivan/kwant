//! Every rule that decides which mail kwant sends and which mail counts. Ported from
//! ClankerInbox's `src/safety.ts` (MIT, Danny Postma, 8c8cf3be), behaviour for behaviour; the
//! parsers are hand-written single passes (no regex), so hostile input costs linear time.
//!
//! Mail content never authorizes an action: these checks only decide whether a mail is passed
//! on as an answer or a code, never what a bot does with it.

use unicode_normalization::UnicodeNormalization;

/// Longer address headers are not read at all: no real From needs this much, and a hostile one
/// should cost nothing to refuse.
pub const MAX_ADDRESS_HEADER: usize = 2048;
/// The `X-Loop` value on every mail kwant sends.
pub const LOOP_MARK: &str = "kwant";
/// The marks on every mail kwant sends, so other robots (and kwant itself) leave it alone.
pub const LOOP_HEADERS: [(&str, &str); 2] =
    [("Auto-Submitted", "auto-generated"), ("X-Loop", LOOP_MARK)];
/// The kind of mail another kwant (or this one) sends: its only marks are [`LOOP_HEADERS`].
pub const AGENT_MAIL: &str = "agent";

/// One mail as the rules see it. Headers keep their order (topmost first) and their raw,
/// unfolded value; a header that comes more than once is read as its values joined by `, `.
#[derive(Clone, Debug, Default)]
pub struct Mail {
    /// (lowercase name, value)
    pub headers: Vec<(String, String)>,
    /// The sender the provider saw, when it reports one apart from the header (IMAP doesn't).
    pub from: Option<String>,
    /// The envelope sender: `None` when unknown, `Some("")` or `Some("<>")` for a bounce.
    pub mail_from: Option<String>,
    /// The DMARC verdict of a server we trust (see [`super::auth_results::trusted_dmarc`]).
    pub dmarc: Option<Dmarc>,
    pub subject: String,
    pub text: String,
    pub html: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dmarc {
    pub result: String,
    pub aligned: bool,
}

impl Mail {
    /// The header's value, its occurrences joined by `, `; empty when it isn't there.
    pub fn header(&self, name: &str) -> String {
        let name = name.to_ascii_lowercase();
        let values: Vec<&str> = self
            .headers
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
            .collect();
        values.join(", ")
    }

    /// Every value of a header, topmost first.
    pub fn header_all(&self, name: &str) -> Vec<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

/// Whether a mail counts, and the one From address it was read with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub ok: bool,
    pub reason: Option<String>,
    pub from: Option<String>,
}

impl Verdict {
    fn pass(from: Option<String>) -> Self {
        Self {
            ok: true,
            reason: None,
            from,
        }
    }
    fn fail(reason: impl Into<String>, from: Option<String>) -> Self {
        Self {
            ok: false,
            reason: Some(reason.into()),
            from,
        }
    }
}

/// Who may write in: everyone (a codes tag, where DMARC still has to pass) or a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Allow {
    Anyone,
    List(Vec<String>),
}

// --- Addresses ---------------------------------------------------------------------------------

/// `^[^@]+@[^@.][^@]*\.[^@]+$`
fn address_shape(w: &str) -> bool {
    let Some((local, domain)) = w.split_once('@') else {
        return false;
    };
    if local.is_empty() || domain.contains('@') || domain.is_empty() || domain.starts_with('.') {
        return false;
    }
    let d: Vec<char> = domain.chars().collect();
    (1..d.len().saturating_sub(1)).any(|i| d[i] == '.')
}

/// The addresses in an address-list header, read left to right the way RFC 5322 reads it: a
/// comment runs to its own `)`, a quoted string to its own `"`, and both are dropped, so
/// `"a@b.com" <c@d.com>` is one address. `None` when the value can be read more than one way
/// (an unbalanced quote, comment or angle bracket, a quote inside a comment, a backslash escape,
/// a quoted local part, a word with an @ that is no plain address), since another parser could
/// then see another sender. Lowercase, without repeats.
pub fn address_list(value: &str) -> Option<Vec<String>> {
    let v: Vec<char> = value.chars().collect();
    if v.len() > MAX_ADDRESS_HEADER || value.contains('\\') {
        return None;
    }
    let mut out = String::new();
    let mut angle = false;
    let mut i = 0;
    while i < v.len() {
        let c = v[i];
        if c == '"' {
            let j = (i + 1..v.len()).find(|&j| v[j] == '"')?;
            if v.get(j + 1) == Some(&'@') {
                return None;
            }
            out.push(' ');
            i = j + 1;
            continue;
        }
        if c == '(' {
            let mut depth = 0i32;
            let mut j = i;
            while j < v.len() {
                match v[j] {
                    '"' => return None,
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if depth != 0 {
                return None;
            }
            out.push(' ');
            i = j + 1;
            continue;
        }
        if c == ')' {
            return None;
        }
        if c == '<' {
            if angle {
                return None;
            }
            angle = true;
        }
        if c == '>' {
            if !angle {
                return None;
            }
            angle = false;
        }
        out.push(c);
        i += 1;
    }
    if angle {
        return None;
    }
    let mut found: Vec<String> = Vec::new();
    for word in out.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | ',' | ';' | ':')) {
        if !word.contains('@') {
            continue;
        }
        if !address_shape(word) {
            return None;
        }
        let w = word.to_lowercase();
        if !found.contains(&w) {
            found.push(w);
        }
    }
    Some(found)
}

/// All addresses in a header value; none when it can't be read one way only.
pub fn parse_addresses(value: &str) -> Vec<String> {
    if value.is_empty() {
        return Vec::new();
    }
    address_list(value).unwrap_or_default()
}

/// A plus tag addresses the same mailbox, so `owner+x@` and `owner@` are one sender.
pub fn base_address(address: &str) -> String {
    match address.rfind('@') {
        None => address.to_lowercase(),
        Some(at) => {
            let local = address[..at].to_lowercase();
            let domain = address[at + 1..].to_lowercase();
            format!("{}@{domain}", local.split('+').next().unwrap_or(""))
        }
    }
}

pub fn plus_tag(address: &str) -> Option<String> {
    let local = &address[..address.rfind('@').unwrap_or(address.len())];
    local.find('+').map(|i| local[i + 1..].to_string())
}

pub fn with_tag(address: &str, tag: &str) -> String {
    let at = address.rfind('@').unwrap_or(address.len());
    let local = address[..at].split('+').next().unwrap_or("");
    format!("{local}+{tag}@{}", address.get(at + 1..).unwrap_or(""))
}

pub fn domain_of(address: &str) -> String {
    address[address.rfind('@').map(|i| i + 1).unwrap_or(0)..].to_lowercase()
}

// --- Senders -----------------------------------------------------------------------------------

/// One From address, the same one the provider saw, or nothing. A From that is there must be
/// exactly one readable address: an unreadable header is never filled in from the provider's field.
pub fn single_from(msg: &Mail) -> Verdict {
    let raw_header = msg.header("from");
    let raw_header = raw_header.trim();
    let raw_field = msg.from.as_deref().unwrap_or("").trim();
    let from_header = if raw_header.is_empty() {
        Some(Vec::new())
    } else {
        address_list(raw_header)
    };
    let from_field = if raw_field.is_empty() {
        Some(Vec::new())
    } else {
        address_list(raw_field)
    };
    let (Some(from_header), Some(from_field)) = (from_header, from_field) else {
        return Verdict::fail("From is not one readable address", None);
    };
    if (!raw_header.is_empty() && from_header.is_empty())
        || (!raw_field.is_empty() && from_field.is_empty())
    {
        return Verdict::fail("From is not one readable address", None);
    }
    if from_header.is_empty() && from_field.is_empty() {
        return Verdict::fail("no From address", None);
    }
    if from_header.len() > 1 || from_field.len() > 1 {
        return Verdict::fail("more than one From address", None);
    }
    let from = from_field.first().or(from_header.first()).cloned();
    if let (Some(h), Some(f)) = (from_header.first(), from_field.first()) {
        if h != f {
            return Verdict::fail("From header and sender differ", None);
        }
    }
    Verdict::pass(from)
}

/// The DMARC result a server we trust vouches for; a header the sender could write is never
/// trusted for this.
pub fn dmarc_aligned(msg: &Mail) -> bool {
    msg.dmarc
        .as_ref()
        .is_some_and(|d| d.result == "pass" && d.aligned)
}

/// Default deny: a mail counts only when an allow list exists, its single From is on it, and
/// DMARC passed aligned for that From.
pub fn check_sender(msg: &Mail, allow: &Allow) -> Verdict {
    if let Allow::List(list) = allow {
        if list.is_empty() {
            return Verdict::fail("allowlist is empty", None);
        }
    }
    let single = single_from(msg);
    if !single.ok {
        return single;
    }
    let from = single.from.clone().unwrap_or_default();
    if let Allow::List(list) = allow {
        let me = base_address(&from);
        if !list.iter().any(|a| base_address(a) == me) {
            return Verdict::fail("sender not on allowlist", Some(from));
        }
    }
    if !dmarc_aligned(msg) {
        return Verdict::fail("DMARC did not pass aligned", Some(from));
    }
    Verdict::pass(Some(from))
}

pub fn is_bounce(msg: &Mail) -> bool {
    let from = msg
        .from
        .clone()
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| msg.header("from"))
        .to_lowercase();
    if from.starts_with("mailer-daemon@")
        || from.starts_with("postmaster@")
        || from.contains("mailer-daemon")
    {
        return true;
    }
    if matches!(msg.mail_from.as_deref(), Some("") | Some("<>")) {
        return true;
    }
    let ct = msg.header("content-type").to_lowercase();
    if ct.contains("multipart/report") || ct.contains("delivery-status") {
        return true;
    }
    !msg.header("x-failed-recipients").is_empty()
}

/// The values of a header that may come more than once (`a, b`), each without its parameters
/// (`auto-generated; x=y` is `auto-generated`), in lowercase.
fn tokens(msg: &Mail, name: &str) -> Vec<String> {
    msg.header(name)
        .split(',')
        .map(|v| v.split(';').next().unwrap_or("").trim().to_lowercase())
        .filter(|v| !v.is_empty())
        .collect()
}

/// What kind of robot sent a mail. `None`: no mark at all (a person). [`AGENT_MAIL`]: its only
/// marks are `Auto-Submitted: auto-generated` and `X-Loop: kwant`. Else the reason to skip it:
/// an auto-reply, bulk or list mail, or another tool's loop mark. Answering any of those could
/// start a loop with a robot that never stops by itself.
pub fn automation(msg: &Mail) -> Option<String> {
    if tokens(msg, "precedence")
        .iter()
        .any(|p| matches!(p.as_str(), "bulk" | "junk" | "list" | "auto_reply"))
    {
        return Some("automated mail (Precedence: bulk, list or junk)".into());
    }
    if !msg.header("x-autoreply").is_empty() || !msg.header("x-autorespond").is_empty() {
        return Some("automated mail (an auto-reply)".into());
    }
    if !msg.header("list-id").is_empty() || !msg.header("list-unsubscribe").is_empty() {
        return Some("automated mail (a mailing list)".into());
    }
    let auto: Vec<String> = tokens(msg, "auto-submitted")
        .into_iter()
        .filter(|v| v != "no")
        .collect();
    if auto.iter().any(|v| v != "auto-generated") {
        return Some("automated mail (an auto-reply)".into());
    }
    // An X-Loop that is there but empty or unreadable is another tool's too
    let lp = tokens(msg, "x-loop");
    let has_loop = msg.headers.iter().any(|(n, _)| n == "x-loop");
    if (has_loop && lp.is_empty()) || lp.iter().any(|v| v != LOOP_MARK) {
        return Some("automated mail (the X-Loop of another tool)".into());
    }
    if !auto.is_empty() || !lp.is_empty() {
        Some(AGENT_MAIL.into())
    } else {
        None
    }
}

/// Send list: each `to` must be on the send list or be one of kwant's own addresses, which
/// reaches nobody else. Plus tags address the same mailbox, so they don't widen who can be reached.
pub fn check_recipients(to: &[String], send: &[String], own: &[String]) -> Verdict {
    if to.is_empty() {
        return Verdict::fail("no recipient", None);
    }
    let allowed: Vec<String> = send
        .iter()
        .chain(own.iter())
        .map(|a| base_address(a))
        .collect();
    for raw in to {
        let parsed = parse_addresses(raw);
        if parsed.len() != 1 {
            return Verdict::fail(format!("not one address: {raw}"), None);
        }
        if !allowed.contains(&base_address(&parsed[0])) {
            return Verdict::fail(
                format!("recipient not on send allowlist: {}", parsed[0]),
                None,
            );
        }
    }
    Verdict::pass(None)
}

// --- Text --------------------------------------------------------------------------------------

fn is_control(c: char) -> bool {
    matches!(c as u32, 0x00..=0x1f | 0x7f..=0x9f | 0x2028 | 0x2029)
}

/// C0 and C1 controls (line breaks and tabs among them), DEL, and the Unicode line and paragraph
/// separators: none belongs in a header value.
pub fn has_control_chars(value: &str) -> bool {
    value.chars().any(is_control)
}

/// Mail text shown to a bot or a person: escape sequences (CSI, OSC such as hyperlinks, any other
/// ESC pair), other C0 and C1 controls, DEL and bidi overrides are dropped, so a mail can't move
/// a cursor, rewrite a line or hide text. Line breaks and tabs stay.
pub fn clean_text(value: &str) -> String {
    const ESC: char = '\u{1b}';
    // CSI: (ESC [ | 0x9b) [0-?]* [ -/]* [@-~]?
    let v: Vec<char> = value.chars().collect();
    let mut a = Vec::with_capacity(v.len());
    let mut i = 0;
    while i < v.len() {
        let start = if v[i] == ESC && v.get(i + 1) == Some(&'[') {
            Some(i + 2)
        } else if v[i] == '\u{9b}' {
            Some(i + 1)
        } else {
            None
        };
        if let Some(mut j) = start {
            while j < v.len() && ('0'..='?').contains(&v[j]) {
                j += 1;
            }
            while j < v.len() && (' '..='/').contains(&v[j]) {
                j += 1;
            }
            if j < v.len() && ('@'..='~').contains(&v[j]) {
                j += 1;
            }
            i = j;
            continue;
        }
        a.push(v[i]);
        i += 1;
    }
    // OSC: (ESC ] | 0x9d) [^BEL ESC 0x9c]* (BEL | ESC \ | 0x9c)?
    let mut b = Vec::with_capacity(a.len());
    let mut i = 0;
    while i < a.len() {
        let start = if a[i] == ESC && a.get(i + 1) == Some(&']') {
            Some(i + 2)
        } else if a[i] == '\u{9d}' {
            Some(i + 1)
        } else {
            None
        };
        if let Some(mut j) = start {
            while j < a.len() && !matches!(a[j], '\u{7}' | ESC | '\u{9c}') {
                j += 1;
            }
            if j < a.len() {
                if a[j] == '\u{7}' || a[j] == '\u{9c}' {
                    j += 1;
                } else if a[j] == ESC && a.get(j + 1) == Some(&'\\') {
                    j += 2;
                }
            }
            i = j;
            continue;
        }
        b.push(a[i]);
        i += 1;
    }
    // Any other ESC and the character after it
    let mut c = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == ESC {
            i += 2;
            continue;
        }
        c.push(b[i]);
        i += 1;
    }
    // CRLF to LF, a lone CR dropped, then the remaining controls and bidi marks
    let mut out = String::with_capacity(c.len());
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch == '\r' {
            if c.get(i + 1) == Some(&'\n') {
                out.push('\n');
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        let drop = matches!(ch as u32, 0x00..=0x08 | 0x0b..=0x1f | 0x7f..=0x9f | 0x200e | 0x200f | 0x202a..=0x202e | 0x2066..=0x2069);
        if !drop {
            out.push(ch);
        }
        i += 1;
    }
    out
}

/// [`clean_text`] for one line: line breaks and tabs become spaces.
pub fn clean_line(value: &str) -> String {
    clean_text(value)
        .chars()
        .map(|c| {
            if matches!(c, '\n' | '\t' | '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

/// An attachment name as the sender wrote it is only ever a label: no path, no controls.
pub fn safe_filename(value: &str) -> String {
    let name: String = clean_line(value).replace(['/', '\\'], "_");
    let name: String = name.trim().chars().take(255).collect();
    if name.is_empty() || name == "." || name == ".." {
        "attachment".into()
    } else {
        name
    }
}

// --- Replies -----------------------------------------------------------------------------------

/// Header lines are short; a longer line is the person's text, and is never matched against the
/// patterns below, which keeps them cheap on hostile input.
const MAX_HEADER_LINE: usize = 300;

fn lc(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn lower_chars(s: &str) -> Vec<char> {
    s.chars().map(lc).collect()
}

fn starts_ci(hay: &[char], needle: &str) -> bool {
    let n: Vec<char> = lower_chars(needle);
    hay.len() >= n.len() && hay[..n.len()] == n[..]
}

fn is_letter(c: char) -> bool {
    c.is_alphabetic()
}

/// `^(word)(?!\p{L})` for any of `words`, without regard to case.
fn starts_with_word(line: &str, words: &[&str]) -> bool {
    let hay = lower_chars(line);
    words.iter().any(|w| {
        let n = w.chars().count();
        starts_ci(&hay, w) && !hay.get(n).is_some_and(|c| is_letter(*c))
    })
}

/// `(?<!\p{L})(word)(?!\p{L})` anywhere, for any of `words`, without regard to case.
fn has_word(line: &str, words: &[&str]) -> bool {
    let hay = lower_chars(line);
    words.iter().any(|w| {
        let n: Vec<char> = lower_chars(w);
        (0..hay.len()).any(|i| {
            hay.len() >= i + n.len()
                && hay[i..i + n.len()] == n[..]
                && (i == 0 || !is_letter(hay[i - 1]))
                && !hay.get(i + n.len()).is_some_and(|c| is_letter(*c))
        })
    })
}

const WROTE: &[&str] = &[
    "wrote",
    "schrieb",
    "écrit",
    "escribió",
    "schreef",
    "scritto",
    "escreveu",
    "skrev",
    "kirjoitti",
    "napisał",
    "napsal",
];
const ATTRIBUTION_START: &[&str] = &[
    "On", "Am", "Le", "El", "Op", "Il", "Em", "Den", "På", "W dniu", "Dne",
];

/// `^On(?!\p{L})` and `(?<!\p{L})wrote\s*:$`
fn english_attribution(joined: &str) -> bool {
    if !starts_with_word(joined, &["on"]) {
        return false;
    }
    let Some(rest) = joined.strip_suffix(':') else {
        return false;
    };
    let rest: Vec<char> = lower_chars(rest.trim_end());
    let w: Vec<char> = "wrote".chars().collect();
    rest.len() >= w.len()
        && rest[rest.len() - w.len()..] == w[..]
        && (rest.len() == w.len() || !is_letter(rest[rest.len() - w.len() - 1]))
}

/// The number of lines (1 to 3, since clients wrap it) of an attribution line such as
/// "On Tue, … Bot <bot@…> wrote:" that starts at line `i`, or 0.
fn attribution(lines: &[&str], i: usize) -> usize {
    let mut joined = String::new();
    for k in 0..3 {
        let Some(line) = lines.get(i + k) else { break };
        let line = line.trim();
        if line.is_empty() || line.chars().count() > MAX_HEADER_LINE {
            return 0;
        }
        if k > 0 {
            joined.push(' ');
        }
        joined.push_str(line);
        if !joined.ends_with(':') {
            continue;
        }
        if english_attribution(&joined) {
            return k + 1;
        }
        if has_word(&joined, WROTE)
            && (starts_with_word(&joined, ATTRIBUTION_START) || joined.contains('@'))
        {
            return k + 1;
        }
    }
    0
}

/// `^(label)\s?:` for any of `labels`, without regard to case.
fn label_line(line: &str, labels: &[&str]) -> bool {
    let hay = lower_chars(line);
    labels.iter().any(|l| {
        let n = l.chars().count();
        if !starts_ci(&hay, l) {
            return false;
        }
        match (hay.get(n), hay.get(n + 1)) {
            (Some(':'), _) => true,
            (Some(c), Some(':')) => c.is_whitespace(),
            _ => false,
        }
    })
}

const FROM_LABEL: &[&str] = &[
    "From",
    "Von",
    "Van",
    "De",
    "Da",
    "Från",
    "Fra",
    "Od",
    "Lähettäjä",
];
const DATE_LABEL: &[&str] = &[
    "Sent",
    "Date",
    "Gesendet",
    "Datum",
    "Verzonden",
    "Envoyé",
    "Enviado",
    "Inviato",
    "Data",
    "Skickat",
    "Sendt",
    "Odesláno",
    "Wysłano",
    "Lähetetty",
];
const OTHER_LABEL: &[&str] = &[
    "To",
    "Subject",
    "Cc",
    "An",
    "Betreff",
    "Aan",
    "Onderwerp",
    "À",
    "Objet",
    "Para",
    "Asunto",
    "A",
    "Oggetto",
    "Till",
    "Ämne",
    "Til",
    "Emne",
    "Komu",
    "Předmět",
    "Do",
    "Temat",
    "Vastaanottaja",
    "Aihe",
];

/// Outlook's quoted header block, in the languages it writes it in: a From line followed closely
/// by a date line and a To or Subject line.
fn outlook_block(lines: &[&str], i: usize) -> bool {
    if !label_line(lines[i].trim(), FROM_LABEL) {
        return false;
    }
    let next: Vec<&str> = lines.iter().skip(i + 1).take(5).map(|l| l.trim()).collect();
    next.iter().any(|l| label_line(l, DATE_LABEL))
        && next.iter().any(|l| label_line(l, OTHER_LABEL))
}

/// `prefix` + `min..=max` chars that each pass `class` + `suffix`, the whole of `s`, without
/// regard to case.
fn shaped(
    s: &[char],
    prefix: &str,
    class: fn(char) -> bool,
    min: usize,
    max: usize,
    suffix: &str,
) -> bool {
    let p = lower_chars(prefix);
    let x = lower_chars(suffix);
    if s.len() < p.len() + x.len() || s[..p.len()] != p[..] || s[s.len() - x.len()..] != x[..] {
        return false;
    }
    let mid = &s[p.len()..s.len() - x.len()];
    (min..=max).contains(&mid.len()) && mid.iter().all(|c| class(*c))
}

fn letter_digit_space_dash(c: char) -> bool {
    c.is_alphabetic() || c.is_numeric() || c == ' ' || c == '-'
}
fn letter_space_dash(c: char) -> bool {
    c.is_alphabetic() || c == ' ' || c == '-'
}
fn letter_space(c: char) -> bool {
    c.is_alphabetic() || c == ' '
}

fn footer_core(s: &[char]) -> bool {
    let is = |t: &str| s == &lower_chars(t)[..];
    if shaped(s, "sent from my ", letter_digit_space_dash, 1, 40, "") {
        return true;
    }
    for app in [
        "Outlook",
        "Mail for Windows",
        "Yahoo Mail",
        "Proton Mail",
        "ProtonMail",
    ] {
        let base = format!("sent from {}", app.to_lowercase());
        if is(&base) || shaped(s, &format!("{base} for "), letter_space, 1, 20, "") {
            return true;
        }
    }
    if is("get outlook for ios") || is("get outlook for android") {
        return true;
    }
    if is("sent with proton mail")
        || is("sent with proton mail secure email")
        || is("sent from proton mail mobile")
    {
        return true;
    }
    shaped(s, "envoyé de mon ", letter_space_dash, 1, 30, "")
        || shaped(s, "von meinem ", letter_space_dash, 1, 30, " gesendet")
        || shaped(s, "verzonden vanaf mijn ", letter_space_dash, 1, 30, "")
        || shaped(s, "verzonden met mijn ", letter_space_dash, 1, 30, "")
        || shaped(s, "enviado desde mi ", letter_space_dash, 1, 30, "")
        || shaped(s, "enviado do meu ", letter_space_dash, 1, 30, "")
        || shaped(s, "inviato da ", letter_space_dash, 1, 30, "")
        || shaped(s, "skickat från min ", letter_space_dash, 1, 30, "")
}

/// A whole line that mail apps add under what the person wrote (`Sent from my iPhone`), with an
/// optional closing `.` and `<address>`. Kept to the exact shape, so "Sent from my laptop because
/// the phone died: 2" stays an answer.
fn footer(line: &str) -> bool {
    let s = lower_chars(line);
    let core = |s: &[char]| {
        let s = if s.last() == Some(&'.') {
            &s[..s.len() - 1]
        } else {
            s
        };
        footer_core(s) || footer_core_dotless(s)
    };
    if core(&s) {
        return true;
    }
    // (\s*<[^<>]{0,200}>)?$
    if s.last() == Some(&'>') {
        if let Some(lt) = s.iter().rposition(|c| *c == '<') {
            let inner = &s[lt + 1..s.len() - 1];
            if inner.len() <= 200 && !inner.iter().any(|c| *c == '<' || *c == '>') {
                let mut end = lt;
                while end > 0 && s[end - 1].is_whitespace() {
                    end -= 1;
                }
                return core(&s[..end]);
            }
        }
    }
    false
}

/// The class parts can end in spaces the regex hands to `\s*`: the core with its own trailing
/// spaces trimmed matches too.
fn footer_core_dotless(s: &[char]) -> bool {
    let mut end = s.len();
    while end > 0 && s[end - 1] == ' ' {
        end -= 1;
    }
    end != s.len() && footer_core(&s[..end])
}

fn dashed_separator(line: &str) -> bool {
    // ^-{2,}[^-]{3,60}-{2,}$
    let c: Vec<char> = line.chars().collect();
    let lead = c.iter().take_while(|x| **x == '-').count();
    let trail = c.iter().rev().take_while(|x| **x == '-').count();
    if lead < 2 || trail < 2 || lead + trail >= c.len() {
        return false;
    }
    let mid = &c[lead..c.len() - trail];
    (3..=60).contains(&mid.len()) && !mid.contains(&'-')
}

/// Where the person's own text ends for good: a signature, a forward or separator line, an
/// Outlook header block, a mobile or Proton footer.
fn end_of_text(lines: &[&str], i: usize) -> bool {
    let line = lines[i].trim();
    if lines[i] == "-- " || line == "--" {
        return true;
    }
    if line.chars().count() > MAX_HEADER_LINE {
        return false;
    }
    dashed_separator(line)
        || (line.chars().count() >= 10 && line.chars().all(|c| c == '_'))
        || footer(line)
        || outlook_block(lines, i)
}

/// `^\s*>(?:\s|>|$)`
fn quote_marked(line: &str) -> bool {
    let t = line.trim_start();
    t.strip_prefix('>')
        .is_some_and(|r| r.is_empty() || r.starts_with(|c: char| c.is_whitespace() || c == '>'))
}

fn split_lines(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(str::to_string)
        .collect()
}

/// Where the person's own text ends: the line of the first quote, attribution, signature, footer
/// or header block. `quote`: it ends at a quote (an answer may follow it).
fn quote_start(lines: &[&str]) -> (usize, bool) {
    let spaced = lines.iter().any(|l| quote_marked(l));
    for i in 0..lines.len() {
        let line = lines[i].trim();
        if line.starts_with('>') && (spaced || quote_marked(line)) {
            return (i, true);
        }
        if attribution(lines, i) > 0 {
            return (i, true);
        }
        if end_of_text(lines, i) {
            return (i, false);
        }
    }
    (lines.len(), false)
}

fn squeeze_blank_lines(s: &str) -> String {
    // \n{3,} -> \n\n
    let mut out = String::with_capacity(s.len());
    let mut run = 0;
    for c in s.chars() {
        if c == '\n' {
            run += 1;
            if run <= 2 {
                out.push(c);
            }
        } else {
            run = 0;
            out.push(c);
        }
    }
    out
}

/// Answers arrive as replies that quote the question; the quote must not be read as the answer,
/// since the quoted options would look like a choice. Text above the quote is the answer. With
/// nothing above it (bottom-posted or inline replies), the answer is the text between and after
/// the quoted lines. A line starting with `>` is a quote only in a text that quotes the usual way
/// (`> `), or after an attribution line: alone, `>2 weeks is too long` is the person's text.
pub fn strip_quoted(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let owned = split_lines(text);
    let lines: Vec<&str> = owned.iter().map(String::as_str).collect();
    let (cut, quote) = quote_start(&lines);
    let above = lines[..cut].join("\n").trim().to_string();
    if !above.is_empty() || !quote {
        return above;
    }
    let mut own: Vec<&str> = Vec::new();
    let mut i = cut;
    while i < lines.len() {
        if lines[i].trim().starts_with('>') {
            i += 1;
            continue;
        }
        let header = attribution(&lines, i);
        if header > 0 {
            i += header;
            continue;
        }
        if end_of_text(&lines, i) {
            break;
        }
        own.push(lines[i].trim_end());
        i += 1;
    }
    squeeze_blank_lines(&own.join("\n")).trim().to_string()
}

const HTML_CUT_CLASSES: &[&str] = &[
    "gmail_signature",
    "gmail_quote",
    "gmail_extra",
    "moz-signature",
    "moz-cite-prefix",
    "yahoo_quoted",
    "protonmail_signature_block",
    "protonmail_quote",
];
const HTML_CUT_IDS: &[&str] = &[
    "applemailsignature",
    "signature",
    "divrplyfwdmsg",
    "appendonsend",
    "ms-outlook-mobile-signature",
];
const MAX_TAG: usize = 2000;

fn word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `\bname\b` at `i` of `s` (lowercase chars).
fn word_at(s: &[char], i: usize, name: &str) -> bool {
    let n: Vec<char> = name.chars().collect();
    s.len() >= i + n.len()
        && s[i..i + n.len()] == n[..]
        && (i == 0 || !word_char(s[i - 1]) || !word_char(n[0]))
        && (i + n.len() == s.len() || !word_char(s[i + n.len()]) || !word_char(n[n.len() - 1]))
}

fn skip_ws(s: &[char], mut j: usize) -> usize {
    while j < s.len() && s[j].is_whitespace() {
        j += 1;
    }
    j
}

/// `\bclass\s*=\s*["'][^"']*\b(?:names)\b`
fn tag_cut_class(s: &[char]) -> bool {
    (0..s.len()).any(|i| {
        if !word_at_start(s, i, "class") {
            return false;
        }
        let mut j = skip_ws(s, i + 5);
        if s.get(j) != Some(&'=') {
            return false;
        }
        j = skip_ws(s, j + 1);
        if !matches!(s.get(j), Some('"') | Some('\'')) {
            return false;
        }
        let start = j + 1;
        let end = (start..s.len())
            .find(|&k| s[k] == '"' || s[k] == '\'')
            .unwrap_or(s.len());
        (start..end).any(|k| HTML_CUT_CLASSES.iter().any(|n| word_at(&s[..end], k, n)))
    })
}

/// `\bname` at `i` (a word boundary before it only).
fn word_at_start(s: &[char], i: usize, name: &str) -> bool {
    let n: Vec<char> = name.chars().collect();
    s.len() >= i + n.len() && s[i..i + n.len()] == n[..] && (i == 0 || !word_char(s[i - 1]))
}

/// `\bid\s*=\s*["']?(?:names)\b`
fn tag_cut_id(s: &[char]) -> bool {
    (0..s.len()).any(|i| {
        if !word_at_start(s, i, "id") {
            return false;
        }
        let mut j = skip_ws(s, i + 2);
        if s.get(j) != Some(&'=') {
            return false;
        }
        j = skip_ws(s, j + 1);
        if matches!(s.get(j), Some('"') | Some('\'')) {
            j += 1;
        }
        HTML_CUT_IDS.iter().any(|n| {
            let m: Vec<char> = n.chars().collect();
            s.len() >= j + m.len()
                && s[j..j + m.len()] == m[..]
                && (j + m.len() == s.len() || !word_char(s[j + m.len()]))
        })
    })
}

/// `^blockquote\b[^]*\btype\s*=\s*["']?cite\b`
fn tag_cut_cite(s: &[char]) -> bool {
    let b: Vec<char> = "blockquote".chars().collect();
    if s.len() < b.len() || s[..b.len()] != b[..] || s.get(b.len()).is_some_and(|c| word_char(*c)) {
        return false;
    }
    (b.len()..s.len()).any(|i| {
        if !word_at_start(s, i, "type") {
            return false;
        }
        let mut j = skip_ws(s, i + 4);
        if s.get(j) != Some(&'=') {
            return false;
        }
        j = skip_ws(s, j + 1);
        if matches!(s.get(j), Some('"') | Some('\'')) {
            j += 1;
        }
        let c: Vec<char> = "cite".chars().collect();
        s.len() >= j + 4 && s[j..j + 4] == c[..] && (j + 4 == s.len() || !word_char(s[j + 4]))
    })
}

/// The person's own words from the HTML part of a reply: what comes before the first signature or
/// quote that a mail app marked, without any quote the text still has. `None` when the HTML has no
/// such mark, or nothing is before it (a bottom-posted reply): then the text part decides. One
/// pass from the left, as [`super::html::html_to_text`] reads it: anyone can send HTML.
pub fn html_answer(html: Option<&str>) -> Option<String> {
    let html = html.filter(|h| !h.is_empty())?;
    let mut i = 0;
    while i < html.len() {
        let lt = i + html[i..].find('<')?;
        let gt = lt + 1 + html[lt + 1..].find('>')?;
        let tag: Vec<char> = html[lt + 1..gt].chars().take(MAX_TAG).map(lc).collect();
        let first = tag.first().copied();
        if first != Some('/')
            && first != Some('!')
            && (tag_cut_class(&tag) || tag_cut_id(&tag) || tag_cut_cite(&tag))
        {
            let own = strip_quoted(&super::html::html_to_text(&html[..lt]));
            return if own.is_empty() { None } else { Some(own) };
        }
        i = gt + 1;
    }
    None
}

/// The last paragraph of an answer, when the quoted part of the same mail has the same lines (at
/// least two, one after the other): a signature that the sender's earlier mail in the quote has
/// too. Text only; a paragraph the quote doesn't repeat stays, since it may be the answer.
fn repeated_signature(own: &str, lines: &[&str], cut: usize) -> String {
    let paragraphs = split_paragraphs(own);
    if paragraphs.len() < 2 {
        return own.to_string();
    }
    let last: Vec<&str> = paragraphs[paragraphs.len() - 1]
        .split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if last.len() < 2 {
        return own.to_string();
    }
    let quoted: Vec<String> = lines[cut..]
        .iter()
        .map(|l| strip_quote_marks(l).trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let mut k = 0;
    while k + last.len() <= quoted.len() {
        if last.iter().enumerate().all(|(j, l)| quoted[k + j] == *l) {
            return paragraphs[..paragraphs.len() - 1]
                .join("\n\n")
                .trim()
                .to_string();
        }
        k += 1;
    }
    own.to_string()
}

/// `^\s*(?:>\s?)*`
fn strip_quote_marks(line: &str) -> &str {
    let mut s = line.trim_start();
    while let Some(r) = s.strip_prefix('>') {
        s = r.strip_prefix(|c: char| c.is_whitespace()).unwrap_or(r);
    }
    s
}

/// `own.split(/\n\s*\n/)`: a line break, white space, and the last line break in it.
fn split_paragraphs(own: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while let Some(k) = own[i..].find('\n') {
        let nl = i + k;
        let ws_end = own[nl + 1..]
            .find(|c: char| !c.is_whitespace())
            .map(|e| nl + 1 + e)
            .unwrap_or(own.len());
        match own[nl + 1..ws_end].rfind('\n') {
            Some(last) => {
                out.push(own[start..nl].to_string());
                start = nl + 1 + last + 1;
                i = start;
            }
            None => i = nl + 1,
        }
    }
    out.push(own[start..].to_string());
    out
}

/// The answer in a reply: from the HTML part when a mail app marked its signature or quote there,
/// else from the text without quotes, footers and a signature the quote repeats.
pub fn answer_text(text: &str, html: Option<&str>) -> String {
    if let Some(a) = html_answer(html) {
        return a;
    }
    if text.is_empty() {
        return String::new();
    }
    let own = strip_quoted(text);
    let owned = split_lines(text);
    let lines: Vec<&str> = owned.iter().map(String::as_str).collect();
    let (cut, quote) = quote_start(&lines);
    if !own.is_empty() && quote && cut < lines.len() && !lines[..cut].join("\n").trim().is_empty() {
        repeated_signature(&own, &lines, cut)
    } else {
        own
    }
}

// --- Choices -----------------------------------------------------------------------------------

const WRAPPING: &[char] = &[
    '"', '\'', '*', '_', '`', '\u{201c}', '\u{201d}', '\u{2018}', '\u{2019}',
];

/// One form for a reply line and an option alike: width and case folded, zero-width characters
/// gone, wrapping quotes or emphasis and a closing `.` or `!` dropped. A `?` stays: "Cancel?" is
/// a question back, not a choice.
fn normalize_choice(s: &str) -> String {
    let v: String = s
        .nfkc()
        .filter(|c| !matches!(*c as u32, 0x200b..=0x200d | 0x2060 | 0xfeff))
        .collect();
    let v: Vec<char> = v.trim().chars().collect();
    let mut a = 0;
    let mut b = v.len();
    while a < b && WRAPPING.contains(&v[a]) {
        a += 1;
    }
    while b > a && (WRAPPING.contains(&v[b - 1]) || v[b - 1] == '.' || v[b - 1] == '!') {
        b -= 1;
    }
    let mid: String = v[a..b].iter().collect();
    mid.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `^(?:option ?#? ?|#)?(\d{1,3})\)?$`: `2`, `2)`, `#2`, `option 2`; plain digits only, so
/// `0x2`, `+2` or `1e0` are no index.
fn option_index(v: &str) -> Option<usize> {
    let mut s = v;
    if let Some(r) = s.strip_prefix("option") {
        s = r.strip_prefix(' ').unwrap_or(r);
        s = s.strip_prefix('#').unwrap_or(s);
        s = s.strip_prefix(' ').unwrap_or(s);
    } else if let Some(r) = s.strip_prefix('#') {
        s = r;
    }
    let s = s.strip_suffix(')').unwrap_or(s);
    if s.is_empty() || s.len() > 3 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `^(hi|hello|hey|dear|thanks|thank you|thx|cheers)( \p{L}+){0,2}[,!.]*$`, without regard to case.
fn greeting(line: &str) -> bool {
    let l = line.to_lowercase();
    [
        "hi",
        "hello",
        "hey",
        "dear",
        "thanks",
        "thank you",
        "thx",
        "cheers",
    ]
    .iter()
    .any(|g| {
        let Some(rest) = l.strip_prefix(g) else {
            return false;
        };
        let rest = rest.trim_end_matches([',', '!', '.']);
        if rest.is_empty() {
            return true;
        }
        let Some(words) = rest.strip_prefix(' ') else {
            return false;
        };
        let words: Vec<&str> = words.split(' ').collect();
        words.len() <= 2
            && words
                .iter()
                .all(|w| !w.is_empty() && w.chars().all(char::is_alphabetic))
    })
}

fn match_line(line: &str, options: &[String], normalized: &[String]) -> Option<String> {
    let v = normalize_choice(line);
    if v.is_empty() {
        return None;
    }
    // The text of an option wins over a number, so the reply `1` to options `4, 2, 1` is `1`
    if let Some(i) = normalized.iter().position(|o| *o == v) {
        return Some(options[i].clone());
    }
    if let Some(n) = option_index(&v) {
        return (n >= 1 && n <= options.len()).then(|| options[n - 1].clone());
    }
    // The start of exactly one option, ending where a word ends: "yes" for "Yes, ship it"
    let vn = v.chars().count();
    let starts: Vec<usize> = normalized
        .iter()
        .enumerate()
        .filter(|(_, o)| {
            o.starts_with(&v) && !o.chars().nth(vn).is_none_or(|c| c.is_alphanumeric())
        })
        .map(|(i, _)| i)
        .collect();
    (starts.len() == 1).then(|| options[starts[0]].clone())
}

/// Reads a reply against the offered options: a number or an option's text (or the start of
/// exactly one option) picks that option; anything else is free text (`None`). A first line that
/// is only a greeting ("Thanks!") is passed over for the next one.
pub fn match_option(answer: &str, options: &[String]) -> Option<String> {
    if options.is_empty() {
        return None;
    }
    let lines: Vec<&str> = answer
        .split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let first_line = lines.first()?;
    let normalized: Vec<String> = options.iter().map(|o| normalize_choice(o)).collect();
    let first = match_line(first_line, options, &normalized);
    if first.is_some() || !greeting(first_line) || lines.len() < 2 {
        return first;
    }
    match_line(lines[1], options, &normalized)
}

// --- Codes -------------------------------------------------------------------------------------

const CODE_WORDS: &[&str] = &[
    "code",
    "otp",
    "pin",
    "passcode",
    "verification",
    "verify",
    "confirm",
    "one-time",
    "one time",
    "security",
    "token",
    "login",
    "sign-in",
    "sign in",
    "signin",
];
const LINK_WORDS: &[&str] = &[
    "verify",
    "verification",
    "confirm",
    "magic",
    "login",
    "log-in",
    "signin",
    "sign-in",
    "sign_in",
    "auth",
    "token",
    "activate",
    "validate",
    "callback",
    "session",
];

fn contains_any_ci(hay: &str, words: &[&str]) -> bool {
    let h = hay.to_lowercase();
    words.iter().any(|w| h.contains(w))
}

/// A code-shaped number in a text, located by char index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub code: String,
    pub at: usize,
    pub len: usize,
}

/// Each code-shaped number in a text: 4–8 digits alone, or two groups of three (`482-913`), not
/// glued to a word, a path, a time or a sign. A run of digit groups joined by single spaces,
/// dashes or dots (`\+?\d+(?:[ .-]\d+)*`) is read whole: a phone number, a date or a long
/// reference never gives up a piece of itself as a code.
pub fn code_candidates(src: &str) -> Vec<Candidate> {
    let s: Vec<char> = src.chars().collect();
    let digit = |i: usize| s.get(i).is_some_and(|c| c.is_ascii_digit());
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let start = i;
        let mut j = i;
        if s[j] == '+' && digit(j + 1) {
            j += 1;
        } else if !digit(j) {
            i += 1;
            continue;
        }
        while digit(j) {
            j += 1;
        }
        while j < s.len() && matches!(s[j], ' ' | '.' | '-') && digit(j + 1) {
            j += 1;
            while digit(j) {
                j += 1;
            }
        }
        let run: String = s[start..j].iter().collect();
        let before = if start > 0 { Some(s[start - 1]) } else { None };
        let after = s.get(j).copied();
        let glued_before =
            before.is_some_and(|c| word_char(c) || matches!(c, '.' | '/' | ':' | '+' | '-'));
        let glued_after = after.is_some_and(|c| word_char(c) || matches!(c, '/' | '-'));
        if !glued_before && !glued_after {
            let plain = (4..=8).contains(&run.len()) && run.bytes().all(|b| b.is_ascii_digit());
            let b = run.as_bytes();
            let pair = b.len() == 7
                && b[..3].iter().all(u8::is_ascii_digit)
                && matches!(b[3], b'-' | b' ')
                && b[4..].iter().all(u8::is_ascii_digit);
            if plain || pair {
                out.push(Candidate {
                    code: run.replace(['-', ' '], ""),
                    at: start,
                    len: j - start,
                });
            }
        }
        i = j;
    }
    out
}

/// Hosts no sign-in link of a real service points to: IP literals, single labels and names that
/// only resolve inside a network.
const PRIVATE_TLDS: &[&str] = &[
    "localhost",
    "local",
    "internal",
    "lan",
    "arpa",
    "intranet",
    "corp",
    "private",
    "home",
];

fn public_host(host: &str) -> bool {
    if !host.contains('.')
        || host.starts_with('[')
        || host.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        return false;
    }
    !PRIVATE_TLDS.contains(&&host[host.rfind('.').map(|i| i + 1).unwrap_or(0)..])
}

/// A link from a mail, as a normalized https URL to a public host (on `domain` or under it when
/// given), or `None`.
fn safe_link(raw: &str, domain: Option<&str>) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let host = url.host_str()?.to_lowercase();
    let host = host.trim_end_matches('.');
    if !public_host(host) {
        return None;
    }
    if let Some(d) = domain {
        if host != d && !host.ends_with(&format!(".{d}")) {
            return None;
        }
    }
    Some(url.to_string())
}

/// Sentence punctuation after a link isn't part of it.
fn trim_punctuation(s: &str) -> &str {
    s.trim_end_matches(['.', ',', ';'])
}

/// `https://[^\s<>"'`)\]\u0000-\u001f\u007f-\u009f]+`, each match with its byte offset.
fn https_links(src: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(k) = src[from..].find("https://") {
        let start = from + k;
        let body = start + "https://".len();
        let end = src[body..]
            .char_indices()
            .find(|(_, c)| {
                c.is_whitespace()
                    || matches!(*c, '<' | '>' | '"' | '\'' | '`' | ')' | ']')
                    || matches!(*c as u32, 0..=0x1f | 0x7f..=0x9f)
            })
            .map(|(i, _)| body + i)
            .unwrap_or(src.len());
        if end > body {
            out.push(&src[start..end]);
            from = end;
        } else {
            from = body;
        }
    }
    out
}

/// A DNS label: `^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$`
fn dns_label(l: &str) -> bool {
    let b = l.as_bytes();
    let ok = |c: &u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    !b.is_empty()
        && b.len() <= 63
        && ok(&b[0])
        && ok(&b[b.len() - 1])
        && b.iter().all(|c| ok(c) || *c == b'-')
}

/// The domain a code must come from, given as a domain, an address or a URL, in any case and with
/// or without a trailing dot.
pub fn sender_domain(value: &str) -> Result<String, String> {
    let fail =
        || format!("invalid from: {value}. Give the sender's domain, for example github.com");
    let mut v = value.trim().to_lowercase();
    let scheme_end = v.find("://");
    let has_scheme = scheme_end.is_some_and(|e| {
        let s = &v[..e];
        s.starts_with(|c: char| c.is_ascii_lowercase())
            && s.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '.' | '-')
            })
    });
    if has_scheme {
        v = url::Url::parse(&v)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .ok_or_else(fail)?;
    } else if v.contains('@') {
        let parts: Vec<&str> = v.split('@').collect();
        if parts.len() != 2 {
            return Err(fail());
        }
        v = parts[1].to_string();
    }
    let v = v.strip_suffix('.').unwrap_or(&v).to_string();
    let host = url::Url::parse(&format!("https://{v}/"))
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .ok_or_else(fail)?;
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 || !labels.iter().all(|l| dns_label(l)) || !public_host(&host) {
        return Err(fail());
    }
    Ok(host)
}

/// What a codes tag hands a bot: a code, a sign-in link, or nothing. Never the mail.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub code: Option<String>,
    pub link: Option<String>,
}

/// Only a 4–8 digit code or a sign-in link ever leaves a codes tag. A number next to code words
/// wins; a bare number (a ZIP in a footer, say) loses to a sign-in link. A link must point at
/// `domain` (or under it) when one is given.
pub fn extract_code(subject: &str, text: &str, domain: Option<&str>) -> Found {
    struct Scored {
        code: String,
        score: i32,
        order: usize,
    }
    let sources = [subject, text];
    let mut candidates: Vec<Scored> = Vec::new();
    for src in sources {
        let chars: Vec<char> = src.chars().collect();
        for c in code_candidates(src) {
            let around: String = chars
                [c.at.saturating_sub(60)..(c.at + c.len + 20).min(chars.len())]
                .iter()
                .collect();
            let mut score = if contains_any_ci(&around, CODE_WORDS) {
                2
            } else {
                0
            };
            let year = c.code.len() == 4 && (c.code.starts_with("19") || c.code.starts_with("20"));
            if year {
                score -= 2;
            }
            let order = candidates.len();
            candidates.push(Scored {
                code: c.code,
                score,
                order,
            });
        }
    }
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then(a.order.cmp(&b.order)));
    let best = candidates.first();
    if let Some(b) = best.filter(|b| b.score > 0) {
        return Found {
            code: Some(b.code.clone()),
            link: None,
        };
    }
    for src in sources {
        for m in https_links(src) {
            let raw = trim_punctuation(m);
            if !contains_any_ci(raw, LINK_WORDS) {
                continue;
            }
            if let Some(link) = safe_link(raw, domain) {
                return Found {
                    code: None,
                    link: Some(link),
                };
            }
        }
    }
    match best {
        Some(b) if b.score >= 0 => Found {
            code: Some(b.code.clone()),
            link: None,
        },
        _ => Found::default(),
    }
}

/// Mail text kept anywhere is masked, so it never holds a usable secret: each number
/// [`extract_code`] could take as a code, and the path and query of each link (a token can sit
/// in either). Other numbers stay readable.
pub fn mask_secrets(text: &str) -> String {
    // https?://[^\s<>"'`)\]]*
    let mut links = String::with_capacity(text.len());
    let mut from = 0;
    loop {
        let next = [text[from..].find("http://"), text[from..].find("https://")]
            .into_iter()
            .flatten()
            .min();
        let Some(k) = next else {
            links.push_str(&text[from..]);
            break;
        };
        let start = from + k;
        let body = start
            + if text[start..].starts_with("https://") {
                8
            } else {
                7
            };
        let end = text[body..]
            .char_indices()
            .find(|(_, c)| {
                c.is_whitespace() || matches!(*c, '<' | '>' | '"' | '\'' | '`' | ')' | ']')
            })
            .map(|(i, _)| body + i)
            .unwrap_or(text.len());
        links.push_str(&text[from..start]);
        let raw = &text[start..end];
        let url = trim_punctuation(raw);
        let host_start = url.find("//").map(|i| i + 2).unwrap_or(0);
        let path = url[host_start..]
            .find(['/', '?', '#'])
            .map(|i| host_start + i)
            .unwrap_or(url.len());
        let rest = &url[path..];
        if rest.is_empty() || rest == "/" {
            links.push_str(raw);
        } else {
            links.push_str(&url[..path]);
            links.push_str("/…");
            links.push_str(&raw[url.len()..]);
        }
        from = end;
    }
    let chars: Vec<char> = links.chars().collect();
    let mut out = String::with_capacity(links.len());
    let mut at = 0;
    for c in code_candidates(&links) {
        out.extend(&chars[at..c.at]);
        out.extend(chars[c.at..c.at + c.len].iter().map(|ch| {
            if ch.is_ascii_digit() {
                '•'
            } else {
                *ch
            }
        }));
        at = c.at + c.len;
    }
    out.extend(&chars[at..]);
    out
}

#[cfg(test)]
#[path = "safety_tests.rs"]
mod tests;
