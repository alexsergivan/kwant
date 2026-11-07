//! The app's log never holds a sign-in token, a cookie value or the fragment of an address.
//! The app itself never logs them; this writer is the backstop for lines from the libraries
//! underneath (a webview that logs the address it failed to open, for one).

use std::borrow::Cow;
use std::io::Write;

const MASK: &str = "[redacted]";

/// The cookie's name, plain and with its `_` percent-encoded.
const NAMES: [&str; 2] = ["kwnt_token", "kwnt%5ftoken"];
/// What can stand between the name and its value: `=`, `:`, quotes and blanks, plain or
/// percent-encoded.
const ENCODED_SEPARATORS: [&str; 3] = ["%3d", "%3a", "%22"];
/// kwant's own token prefixes (src/server/signin.rs): sign-in link, session, device.
const TOKEN_PREFIXES: [&str; 3] = ["kl1.", "ks1.", "kd1."];

/// `kwnt_token=<value>` anywhere (a fragment, a cookie header, a query), everything after `#`
/// in a web address, and a bare kwant token (`kl1.…`, `ks1.…`, `kd1.…`) become `[redacted]`.
pub fn redact(line: &str) -> Cow<'_, str> {
    match named(line) {
        Cow::Borrowed(_) => bare_tokens(line),
        Cow::Owned(out) => Cow::Owned(bare_tokens(&out).into_owned()),
    }
}

/// The name rule and the address rule.
fn named(line: &str) -> Cow<'_, str> {
    if find_name(line).is_none() && !line.contains('#') {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while !rest.is_empty() {
        let token_at = find_name(rest);
        let url_at = ["http://", "https://", "tauri://"].iter().filter_map(|s| find_ci(rest, s)).min();
        match (token_at, url_at) {
            (Some((t, len)), u) if u.is_none_or(|u| t < u) => {
                let after = t + len;
                out.push_str(&rest[..after]);
                rest = &rest[after..];
                // `kwnt_token=value`, `kwnt_token: value`, `"kwnt_token":"value"`, `kwnt_token\tvalue`
                let sep = separator_len(rest, |c| matches!(c, '=' | ':' | '"') || c.is_whitespace());
                if sep > 0 {
                    out.push_str(&rest[..sep]);
                    rest = &rest[sep..];
                    let end = rest.find(is_value_end).unwrap_or(rest.len());
                    if end > 0 {
                        out.push_str(MASK);
                    }
                    rest = &rest[end..];
                }
            }
            (_, Some(u)) => {
                out.push_str(&rest[..u]);
                rest = &rest[u..];
                let end = rest.find(is_url_end).unwrap_or(rest.len());
                let url = &rest[..end];
                match url.find('#') {
                    Some(h) => {
                        out.push_str(&redact_plain(&url[..h]));
                        out.push('#');
                        out.push_str(MASK);
                    }
                    None => out.push_str(&redact_plain(url)),
                }
                rest = &rest[end..];
            }
            (None, None) => {
                out.push_str(rest);
                rest = "";
            }
            (Some(_), None) => unreachable!(),
        }
    }
    Cow::Owned(out)
}

/// The token rule alone (inside an address, before its fragment).
fn redact_plain(text: &str) -> String {
    if find_name(text).is_none() {
        return text.to_string();
    }
    // no address can start inside `text` again, so this ends
    let mut out = String::new();
    let mut rest = text;
    while let Some((t, len)) = find_name(rest) {
        let after = t + len;
        out.push_str(&rest[..after]);
        rest = &rest[after..];
        let sep = separator_len(rest, |c| c == '=');
        if sep > 0 {
            out.push_str(&rest[..sep]);
            let value = &rest[sep..];
            let end = value.find(['&', ';', '#']).unwrap_or(value.len());
            if end > 0 {
                out.push_str(MASK);
            }
            rest = &value[end..];
        }
    }
    out.push_str(rest);
    out
}

/// The length of the separator run at the start of `text`: characters `plain` accepts and
/// percent-encoded separators.
fn separator_len(text: &str, plain: impl Fn(char) -> bool) -> usize {
    let mut at = 0;
    loop {
        let rest = &text[at..];
        if let Some(c) = rest.chars().next().filter(|&c| plain(c)) {
            at += c.len_utf8();
        } else if let Some(e) = ENCODED_SEPARATORS.iter().find(|e| rest.get(..e.len()).is_some_and(|r| r.eq_ignore_ascii_case(e))) {
            at += e.len();
        } else {
            return at;
        }
    }
}

/// A kwant token on its own (`token kl1.abc`): the prefix stays, the secret goes.
fn bare_tokens(line: &str) -> Cow<'_, str> {
    if !TOKEN_PREFIXES.iter().any(|p| line.contains(p)) {
        return Cow::Borrowed(line);
    }
    let is_secret = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '+' | '/' | '%');
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    let mut prev: Option<char> = None;
    while let Some((at, prefix)) = TOKEN_PREFIXES.iter().filter_map(|p| rest.find(p).map(|i| (i, *p))).min() {
        let before = rest[..at].chars().next_back().or(prev);
        let starts_word = before.is_none_or(|c| !c.is_ascii_alphanumeric());
        let after = &rest[at + prefix.len()..];
        let secret = after.find(|c: char| !is_secret(c)).unwrap_or(after.len());
        out.push_str(&rest[..at + prefix.len()]);
        if starts_word && secret > 0 && !after.starts_with(MASK) {
            out.push_str(MASK);
            rest = &after[secret..];
        } else {
            rest = after;
        }
        prev = out.chars().next_back();
    }
    out.push_str(rest);
    Cow::Owned(out)
}

fn find_name(hay: &str) -> Option<(usize, usize)> {
    NAMES.iter().filter_map(|n| find_ci(hay, n).map(|i| (i, n.len()))).min()
}

fn is_value_end(c: char) -> bool {
    c.is_whitespace() || matches!(c, '&' | ';' | '"' | '\'' | ',' | ')' | '}' | ']' | '>')
}

fn is_url_end(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`')
}

/// Case-insensitive find for an ASCII needle.
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    if n.len() > h.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| hay.is_char_boundary(i) && h[i..i + n.len()].eq_ignore_ascii_case(n))
}

/// The most a line may grow while it waits for its end; past this it is redacted as it is.
const LINE_LIMIT: usize = 64 * 1024;

/// Wraps the log file (or stderr) and redacts whole lines. The log formatter hands over one
/// line per write, but a line that comes in pieces (a token split across two writes) is held
/// until its newline, a flush or the end of the writer, and redacted in one go.
pub struct Redacting<W: Write> {
    inner: W,
    pending: Vec<u8>,
}

impl<W: Write> Redacting<W> {
    pub fn new(inner: W) -> Self {
        Redacting { inner, pending: Vec::new() }
    }

    /// Redacts and writes the first `upto` bytes held.
    fn emit(&mut self, upto: usize) -> std::io::Result<()> {
        if upto == 0 {
            return Ok(());
        }
        let line: Vec<u8> = self.pending.drain(..upto).collect();
        self.inner.write_all(redact(&String::from_utf8_lossy(&line)).as_bytes())
    }
}

impl<W: Write> Write for Redacting<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(buf);
        if let Some(last) = self.pending.iter().rposition(|&b| b == b'\n') {
            self.emit(last + 1)?;
        }
        if self.pending.len() > LINE_LIMIT {
            self.emit(self.pending.len())?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.emit(self.pending.len())?;
        self.inner.flush()
    }
}

impl<W: Write> Drop for Redacting<W> {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_links_lose_their_fragment() {
        assert_eq!(
            redact("failed to navigate to url https://kwant.example.com/#kwnt_token=kl1.TESTONLY: oops"),
            "failed to navigate to url https://kwant.example.com/#[redacted] oops"
        );
        assert_eq!(
            redact(r#"navigating to "https://kwant.example.com/x#/missions" now"#),
            r#"navigating to "https://kwant.example.com/x#[redacted]" now"#
        );
        assert_eq!(redact("tauri://localhost/connect.html#x"), "tauri://localhost/connect.html#[redacted]");
    }

    #[test]
    fn token_values_are_masked_anywhere() {
        assert_eq!(redact("Cookie: kwnt_token=kd1.TESTONLY; other=1"), "Cookie: kwnt_token=[redacted]; other=1");
        assert_eq!(redact(r#"{"kwnt_token":"kd1.TESTONLY"}"#), r#"{"kwnt_token":"[redacted]"}"#);
        assert_eq!(redact("KWNT_TOKEN=kd1.TESTONLY"), "KWNT_TOKEN=[redacted]");
        assert_eq!(redact("https://kwant.example.com/?kwnt_token=kd1.TESTONLY&a=1"), "https://kwant.example.com/?kwnt_token=[redacted]&a=1");
        assert_eq!(redact("set kwnt_token kd1.TESTONLY"), "set kwnt_token [redacted]");
    }

    #[test]
    fn other_lines_are_left_alone() {
        let line = "2026-11-07T10:00:00Z  INFO connected host=kwant.example.com status=200";
        assert!(matches!(redact(line), Cow::Borrowed(_)));
        assert_eq!(redact("issue #42 on https://kwant.example.com/"), "issue #42 on https://kwant.example.com/");
        assert_eq!(redact("kwnt_token"), "kwnt_token");
        assert_eq!(redact("ünïcødé # kwnt_token=é"), "ünïcødé # kwnt_token=[redacted]");
    }

    #[test]
    fn the_writer_redacts_what_it_writes() {
        let mut buf = Vec::new();
        writeln!(Redacting::new(&mut buf), "opened https://kwant.example.com/#kwnt_token=kl1.TESTONLY").unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(!out.contains("TESTONLY"), "{out}");
        assert_eq!(out, "opened https://kwant.example.com/#[redacted]\n");
    }

    /// Rigel's R1 battery: a tab, a percent-encoded name or separator, a bare token.
    #[test]
    fn odd_separators_encoded_names_and_bare_tokens_are_masked() {
        let t = "kl1.TESTONLY";
        for line in [
            format!("kwnt_token\t{t}"),
            format!("kwnt_token \t {t}"),
            format!("kwnt%5Ftoken={t}"),
            format!("KWNT%5fTOKEN={t}"),
            format!("kwnt_token%3D{t}"),
            format!("https://kwant.example.com/?kwnt%5Ftoken%3D{t}&a=1"),
            format!("https://kwant.example.com/%23kwnt_token={t}"),
            format!("token {t}"),
            t.to_string(),
            "(ks1.TESTONLY)".to_string(),
            "device=kd1.TESTONLY-x_y".to_string(),
            format!("two {t} and ks1.TESTONLY"),
            format!("failed to open https://kwant.example.com/#kwnt_token={t}"),
            format!("Set-Cookie: kwnt_token={t}; Path=/; HttpOnly"),
            format!("url=https://h/?kwnt_token={t}&x=1"),
            format!("kwnt_token = {t}"),
        ] {
            let out = redact(&line);
            assert!(!out.contains("TESTONLY"), "{line:?} => {out:?}");
            assert!(out.contains(MASK), "{line:?} => {out:?}");
        }
        assert_eq!(redact("token kl1.TESTONLY ok"), "token kl1.[redacted] ok");
        assert_eq!(redact("kwnt%5Ftoken=kl1.TESTONLY;x"), "kwnt%5Ftoken=[redacted];x");
        assert_eq!(redact("kwnt_token\tkl1.TESTONLY"), "kwnt_token\t[redacted]");
        // look-alikes inside other words are left alone, and masking is idempotent
        assert_eq!(redact("model gkl1.5 and v2ks1.0"), "model gkl1.5 and v2ks1.0");
        assert_eq!(redact("kl1. on its own"), "kl1. on its own");
        let once = redact("token kl1.TESTONLY").into_owned();
        assert_eq!(redact(&once), once);
        assert!(matches!(redact("2026-11-07 INFO connected host=kwant.example.com"), Cow::Borrowed(_)));
    }

    /// Rigel's R1: a line that reaches the writer in pieces is redacted as one line.
    #[test]
    fn a_token_split_across_writes_is_masked() {
        let line = "x https://kwant.example.com/#kwnt_token=kl1.TESTONLY\nnext kwnt_token=kd1.TESTONLY\n";
        for cut in 1..line.len() {
            let mut buf = Vec::new();
            {
                let mut w = Redacting::new(&mut buf);
                let (a, b) = line.split_at(cut);
                w.write_all(a.as_bytes()).unwrap();
                w.write_all(b.as_bytes()).unwrap();
            }
            let out = String::from_utf8(buf).unwrap();
            assert_eq!(out, "x https://kwant.example.com/#[redacted]\nnext kwnt_token=[redacted]\n", "cut at {cut}");
        }
        // a last line with no newline is written (redacted) when the writer ends
        let mut buf = Vec::new();
        {
            let mut w = Redacting::new(&mut buf);
            w.write_all(b"tail kwnt_tok").unwrap();
            w.write_all(b"en=kl1.TESTONLY").unwrap();
        }
        assert_eq!(String::from_utf8(buf).unwrap(), "tail kwnt_token=[redacted]");
    }
}
