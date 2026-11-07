//! What a recorded step keeps of a page address (decisions.md Q-S1): the origin and the path,
//! without the query, the fragment or any user:password, and with every path segment that looks
//! like an id or a token (a run of 20 or more base64url characters, which covers hex) replaced
//! by "…".

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

/// A segment, or "…" when it has a token-like run.
pub fn scrub_segment(seg: &str) -> String {
    let mut run = 0usize;
    for c in seg.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            run += 1;
            if run >= TOKEN_RUN {
                return "…".into();
            }
        } else {
            run = 0;
        }
    }
    seg.to_string()
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
