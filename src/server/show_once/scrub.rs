//! What a recorded step keeps of a page address (decisions.md Q-S1): the origin and the path,
//! without the query, the fragment or any user:password, and with every path segment that looks
//! like an id or a token (a run of 20 or more base64url characters, which covers hex) replaced
//! by "…". A long run that is a readable slug, lowercase words joined by hyphens such as
//! `how-to-export-your-orders`, is kept (S-F6): see [`is_slug`].

/// Where a step happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageAt {
    /// Scheme and host, with the port when it isn't the default: `https://shop.example.com`.
    pub origin: String,
    pub path: String,
}

/// A run this long of `[A-Za-z0-9_-]` inside a path segment hides the segment.
pub const TOKEN_RUN: usize = 20;
/// Longest kept path, in characters; longer is cut with "…".
pub const MAX_PATH_CHARS: usize = 300;

/// The cleaned address, or `None` for a page that isn't recorded (anything but http and https:
/// `about:`, `data:`, `blob:`, `chrome:`, `file:`...) or that isn't an address at all.
pub fn scrub(raw: &str) -> Option<PageAt> {
    let url = url::Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    let origin = url.origin().ascii_serialization();
    let path = url.path().split('/').map(scrub_segment).collect::<Vec<_>>().join("/");
    let path = if path.is_empty() { "/".to_string() } else { path };
    Some(PageAt { origin, path: cut(&path, MAX_PATH_CHARS) })
}

/// A segment, or "…" when it has a token-like run: a run of [`TOKEN_RUN`] or more
/// `[A-Za-z0-9_-]` characters that isn't a readable slug ([`is_slug`]).
pub fn scrub_segment(seg: &str) -> String {
    let token = seg
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .any(|run| run.len() >= TOKEN_RUN && !is_slug(run));
    if token {
        "…".into()
    } else {
        seg.to_string()
    }
}

/// Longest word in a readable slug.
pub const SLUG_WORD: usize = 16;
/// Most consonants in a row in a slug's word (`strengths` has five and isn't kept: safe).
pub const SLUG_CONSONANTS: usize = 4;
/// Longest number in a readable slug (a year, an issue number).
pub const SLUG_NUMBER: usize = 4;

/// Whether a run reads as words joined by hyphens (`quarterly-report-for-2025`) rather than an
/// id or a token: two or more parts split by single hyphens, each either lowercase letters (at
/// most [`SLUG_WORD`], and once longer than three with a vowel and no more than
/// [`SLUG_CONSONANTS`] consonants in a row) or digits (at most
/// [`SLUG_NUMBER`]), and not only numbers. Anything with an uppercase letter, an underscore,
/// letters and digits in one part, a long number or a long consonant run is not: that covers
/// base64url, hex, uuids and most random ids. A token made of real words (`correct-horse-…`)
/// reads as a slug and is kept; it's indistinguishable from one by its look.
pub fn is_slug(run: &str) -> bool {
    let parts: Vec<&str> = run.split('-').collect();
    if parts.len() < 2 {
        return false;
    }
    let mut words = 0;
    for part in &parts {
        let letters = !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase());
        let digits = !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
        if letters {
            if part.len() > SLUG_WORD || (part.len() > 3 && !reads_as_a_word(part)) {
                return false;
            }
            words += 1;
        } else if !(digits && part.len() <= SLUG_NUMBER) {
            return false;
        }
    }
    words > 0
}

fn reads_as_a_word(word: &str) -> bool {
    let vowel = |b: u8| b"aeiouy".contains(&b);
    let mut run = 0;
    for b in word.bytes() {
        run = if vowel(b) { 0 } else { run + 1 };
        if run > SLUG_CONSONANTS {
            return false;
        }
    }
    word.bytes().any(vowel)
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(raw: &str) -> Option<(String, String)> {
        scrub(raw).map(|p| (p.origin, p.path))
    }

    fn pair(o: &str, p: &str) -> Option<(String, String)> {
        Some((o.to_string(), p.to_string()))
    }

    #[test]
    fn query_fragment_and_userinfo_are_dropped() {
        assert_eq!(
            at("https://shop.example.com/admin/orders?status=paid&token=abc#top"),
            pair("https://shop.example.com", "/admin/orders")
        );
        assert_eq!(at("https://sam:hunter2@shop.example.com/x"), pair("https://shop.example.com", "/x"));
        assert_eq!(at("https://shop.example.com"), pair("https://shop.example.com", "/"));
        assert_eq!(at("https://shop.example.com/?q=1"), pair("https://shop.example.com", "/"));
    }

    #[test]
    fn the_port_stays_only_when_it_isnt_the_default() {
        assert_eq!(at("http://127.0.0.1:9491/form.html"), pair("http://127.0.0.1:9491", "/form.html"));
        assert_eq!(at("https://example.com:443/a"), pair("https://example.com", "/a"));
        assert_eq!(at("http://example.com:80/a"), pair("http://example.com", "/a"));
        assert_eq!(at("HTTPS://Example.COM/A"), pair("https://example.com", "/A"));
    }

    #[test]
    fn token_like_segments_become_an_ellipsis() {
        // exactly 20 base64url characters
        assert_eq!(at("https://a.example/orders/AbCdEfGhIjKlMnOpQrSt"), pair("https://a.example", "/orders/…"));
        // 19 are kept
        assert_eq!(at("https://a.example/orders/AbCdEfGhIjKlMnOpQrS"), pair("https://a.example", "/orders/AbCdEfGhIjKlMnOpQrS"));
        // hex, a uuid's long tail, _ and - inside the run
        assert_eq!(at("https://a.example/s/0123456789abcdef0123/edit"), pair("https://a.example", "/s/…/edit"));
        assert_eq!(at("https://a.example/r/ab_cd-ef_gh-ij_kl-mn_op/x"), pair("https://a.example", "/r/…/x"));
        // a uuid's parts are dash-joined, so the run is the whole uuid
        assert_eq!(
            at("https://a.example/i/123e4567-e89b-12d3-a456-426614174000"),
            pair("https://a.example", "/i/…")
        );
        // a run inside a longer segment hides all of it
        assert_eq!(at("https://a.example/reset.AbCdEfGhIjKlMnOpQrStUv.html"), pair("https://a.example", "/…"));
    }

    #[test]
    fn readable_slugs_are_kept_but_tokens_are_not() {
        // S-F6: words joined by hyphens stay readable, however long
        for slug in [
            "how-to-export-your-orders",
            "quarterly-report-for-2025",
            "annual-sales-summary",
            "help/articles/reset-your-two-factor-settings",
        ] {
            assert_eq!(at(&format!("https://a.example/{slug}")), pair("https://a.example", &format!("/{slug}")), "{slug}");
        }
        // with a readable slug next to a token in the same address, only the token goes
        assert_eq!(
            at("https://a.example/blog/how-to-export-your-orders/AbCdEfGhIjKlMnOpQrSt"),
            pair("https://a.example", "/blog/how-to-export-your-orders/…")
        );
        // Rigel's reset link (Q-S1(f)): the token is still scrubbed
        for token in [
            "Zx9fK2LmQp4RtV8wYb3NcD7e",
            "0f3a9c27e1b84d6f9a2c5e7b",
            "9c1f6e2a-4b7d-4e3a-8f5c-0d2b6a9e1c74",
            "k3j4h5g6-f7d8s9a0-q1w2e3r4",
            "abcdefgh-ijklmnop-qrstuvwx",
            "xkcd-qwrtzp-plmnbv-zxcvbn",
            "reset-token-0f3a9c27e1b84d6f9a2c",
            "Reset-Your-Password-Today",
            "reset_your_password_today",
            "abc--defghijklmnopqrstu",
            "2024-11-07-1234-5678",
        ] {
            assert_eq!(at(&format!("https://a.example/account/reset/{token}")), pair("https://a.example", "/account/reset/…"), "{token}");
        }
    }

    #[test]
    fn what_counts_as_a_slug() {
        assert!(is_slug("how-to-export-your-orders"));
        assert!(is_slug("report-2025"));
        assert!(is_slug("q-and-a"));
        assert!(!is_slug("orders"), "one word has no hyphen");
        assert!(!is_slug("2024-11-07"), "only numbers");
        assert!(!is_slug("report-20251107"), "a long number");
        assert!(!is_slug("report-v2"), "letters and digits in one part");
        assert!(!is_slug("verylongwordthatkeepsgoing-on"), "a word over 16 letters");
        assert!(!is_slug("bcdf-report"), "no vowel");
        assert!(!is_slug("ijklmnop-report"), "five consonants in a row");
        assert!(!is_slug("-report-x"), "an empty part");
    }

    #[test]
    fn dotted_segments_are_kept_while_each_part_is_short() {
        assert_eq!(at("https://a.example/files/report.csv"), pair("https://a.example", "/files/report.csv"));
        // quarterly_report_v2 is 19 characters: kept; one more and the segment is hidden
        assert_eq!(
            at("https://a.example/files/quarterly_report_v2.final.pdf"),
            pair("https://a.example", "/files/quarterly_report_v2.final.pdf")
        );
        assert_eq!(at("https://a.example/files/quarterly_report_v22.final.pdf"), pair("https://a.example", "/files/…"));
    }

    #[test]
    fn pages_that_arent_recorded() {
        for raw in [
            "about:blank",
            "data:text/html,<p>hi</p>",
            "blob:https://a.example/123",
            "chrome://settings",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "chrome-error://chromewebdata/",
            "not a url",
            "",
        ] {
            assert_eq!(at(raw), None, "{raw}");
        }
    }

    #[test]
    fn a_long_path_is_cut() {
        let long = format!("https://a.example/{}", "ab/".repeat(200));
        let (_, path) = at(&long).unwrap();
        assert_eq!(path.chars().count(), MAX_PATH_CHARS);
        assert!(path.ends_with('…'));
    }
}
