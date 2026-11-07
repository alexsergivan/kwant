//! The app's log never holds a sign-in token, a cookie value or the fragment of an address.
//! The app itself never logs them; this writer is the backstop for lines from the libraries
//! underneath (a webview that logs the address it failed to open, for one).

use std::borrow::Cow;
use std::io::Write;

const MASK: &str = "[redacted]";

/// `kwnt_token=<value>` anywhere (a fragment, a cookie header, a query), and everything after
/// `#` in a web address, become `[redacted]`.
pub fn redact(line: &str) -> Cow<'_, str> {
    if find_ci(line, "kwnt_token").is_none() && !line.contains('#') {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while !rest.is_empty() {
        let token_at = find_ci(rest, "kwnt_token");
        let url_at = ["http://", "https://", "tauri://"].iter().filter_map(|s| find_ci(rest, s)).min();
        match (token_at, url_at) {
            (Some(t), u) if u.is_none_or(|u| t < u) => {
                let after = t + "kwnt_token".len();
                out.push_str(&rest[..after]);
                rest = &rest[after..];
                // `kwnt_token=value`, `kwnt_token: value` or `"kwnt_token":"value"`
                let sep = rest.len() - rest.trim_start_matches(['=', ':', '"', ' ']).len();
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
    if find_ci(text, "kwnt_token").is_none() {
        return text.to_string();
    }
    // no address can start inside `text` again, so this ends
    let mut out = String::new();
    let mut rest = text;
    while let Some(t) = find_ci(rest, "kwnt_token") {
        let after = t + "kwnt_token".len();
        out.push_str(&rest[..after]);
        rest = &rest[after..];
        if let Some(stripped) = rest.strip_prefix('=') {
            out.push('=');
            let end = stripped.find(['&', ';', '#']).unwrap_or(stripped.len());
            if end > 0 {
                out.push_str(MASK);
            }
            rest = &stripped[end..];
        }
    }
    out.push_str(rest);
    out
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

/// Wraps the log file (or stderr) and redacts each write. The log formatter hands over one
/// whole line per write.
pub struct Redacting<W: Write>(pub W);

impl<W: Write> Write for Redacting<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        self.0.write_all(redact(&text).as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
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
        let mut w = Redacting(Vec::new());
        write!(w, "opened https://kwant.example.com/#kwnt_token=kl1.TESTONLY\n").unwrap();
        let out = String::from_utf8(w.0).unwrap();
        assert!(!out.contains("TESTONLY"), "{out}");
        assert_eq!(out, "opened https://kwant.example.com/#[redacted]\n");
    }
}
