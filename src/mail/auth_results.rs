//! DMARC from Authentication-Results headers (RFC 8601). Ported from ClankerInbox's
//! `src/auth-results.ts` (MIT). Anyone can write such a header into the mail they send, so only a
//! header from a server we trust counts, and only the topmost one from it: the receiving server
//! adds its header above everything the sender wrote.

use super::safety::{address_list, domain_of, Dmarc, Mail};

/// Splits a header into its `;` parts and each part into words, in one pass from the left: a
/// comment (which may nest) is dropped, a quoted string is part of its word, and spaces around
/// `=` join. The sender writes parts of this header (the envelope address, say), so whatever two
/// parsers could read two ways gives `None`: an unclosed quote or comment, a stray `)`, a quote
/// inside a comment, a parenthesis inside a quoted string.
fn split_parts(value: &str) -> Option<Vec<Vec<String>>> {
    let v: Vec<char> = value.chars().collect();
    let mut parts: Vec<Vec<String>> = vec![Vec::new()];
    let mut word = String::new();
    let mut gap = false;
    let mut depth = 0u32;
    let mut quoted = false;
    fn end_word(parts: &mut [Vec<String>], word: &mut String, gap: &mut bool) {
        if !word.is_empty() {
            parts
                .last_mut()
                .expect("one part")
                .push(std::mem::take(word));
        }
        *gap = false;
    }
    let mut i = 0;
    while i < v.len() {
        let c = v[i];
        if depth > 0 {
            match c {
                '\\' => i += 1,
                '"' => return None,
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            i += 1;
            continue;
        }
        if quoted {
            match c {
                '\\' => {
                    i += 1;
                    if let Some(n) = v.get(i) {
                        word.push(*n);
                    }
                }
                '"' => quoted = false,
                '(' | ')' => return None,
                _ => word.push(c),
            }
            i += 1;
            continue;
        }
        match c {
            '(' => {
                depth = 1;
                gap = true;
            }
            ')' => return None,
            ';' => {
                end_word(&mut parts, &mut word, &mut gap);
                parts.push(Vec::new());
            }
            ' ' | '\t' | '\r' | '\n' => gap = true,
            _ => {
                // add(c): a gap ends the word, unless `=` joins across it
                let joins = c == '=' || word.ends_with('=');
                if gap && !joins {
                    end_word(&mut parts, &mut word, &mut gap);
                }
                gap = false;
                if c == '"' {
                    quoted = true;
                } else {
                    word.push(c);
                }
            }
        }
        i += 1;
    }
    if quoted || depth > 0 {
        return None;
    }
    end_word(&mut parts, &mut word, &mut gap);
    Some(parts)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Method {
    pub method: String,
    pub result: String,
    pub props: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthResults {
    pub authserv_id: String,
    pub methods: Vec<Method>,
}

fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// `^([a-z0-9_-]+)(?:/[\d.]+)?=([a-z0-9_-]+)$`, without regard to case.
fn method_head(t: &str) -> Option<(String, String)> {
    let (left, result) = t.split_once('=')?;
    let (name, version) = match left.split_once('/') {
        Some((n, v)) => (n, Some(v)),
        None => (left, None),
    };
    if name.is_empty() || !name.chars().all(name_char) {
        return None;
    }
    if let Some(v) = version {
        if v.is_empty() || !v.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return None;
        }
    }
    if result.is_empty() || !result.chars().all(name_char) {
        return None;
    }
    Some((name.to_ascii_lowercase(), result.to_ascii_lowercase()))
}

pub fn parse_auth_results(value: &str) -> Option<AuthResults> {
    let parts = split_parts(value)?;
    let authserv_id = parts[0].first()?.to_lowercase();
    if authserv_id.is_empty() {
        return None;
    }
    let mut methods = Vec::new();
    for tokens in &parts[1..] {
        let first = tokens.first().map(String::as_str).unwrap_or("");
        let Some((method, result)) = method_head(first) else {
            // Kept, so a result we cannot read still counts when results are counted
            let name: String = first.chars().take_while(|c| name_char(*c)).collect();
            if !name.is_empty() {
                methods.push(Method {
                    method: name.to_ascii_lowercase(),
                    result: String::new(),
                    props: Vec::new(),
                });
            }
            continue;
        };
        let mut props = Vec::new();
        for t in &tokens[1..] {
            // ^([a-z0-9_.-]+)=(.*)$
            if let Some((k, v)) = t.split_once('=') {
                if !k.is_empty() && k.chars().all(|c| name_char(c) || c == '.') && !v.contains('\n')
                {
                    props.push((k.to_ascii_lowercase(), v.to_string()));
                }
            }
        }
        methods.push(Method {
            method,
            result,
            props,
        });
    }
    Some(AuthResults {
        authserv_id,
        methods,
    })
}

/// The authserv-id a header names: `^\s*([^\s;(]+)`.
fn header_id(v: &str) -> String {
    v.trim_start()
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ';' && *c != '(')
        .collect::<String>()
        .to_lowercase()
}

/// Whether an authserv-id is one we trust: an exact id, or under a `*.domain` entry (a provider
/// whose MX hosts are numbered, such as `*.messagingengine.com`).
pub fn trusted_id(id: &str, trusted: &[String]) -> bool {
    trusted
        .iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .any(|t| match t.strip_prefix("*.") {
            Some(domain) => id.ends_with(&format!(".{domain}")),
            None => t == id,
        })
}

/// The DMARC verdict of the trusted server for this mail, or `None` when it gave none. `aligned`
/// also checks the domain the verdict was for (`header.from`) against the From header, so a pass
/// for another domain doesn't count; a verdict that names no header.from is never aligned.
pub fn trusted_dmarc(msg: &Mail, trusted: &[String]) -> Option<Dmarc> {
    // The topmost header from the trusted server, readable or not: one we can't read is never
    // passed over for a readable copy below it
    let raw = msg
        .header_all("authentication-results")
        .into_iter()
        .find(|v| trusted_id(&header_id(v), trusted))?;
    let ours = parse_auth_results(raw)?;
    // Two DMARC results in one header means part of it wasn't written by the server
    let results: Vec<&Method> = ours
        .methods
        .iter()
        .filter(|m| m.method == "dmarc")
        .collect();
    if results.len() != 1 || results[0].result.is_empty() {
        return None;
    }
    let dmarc = results[0];
    let from = address_list(&msg.header_all("from").join(", ")).unwrap_or_default();
    let from_domain = (from.len() == 1).then(|| domain_of(&from[0]));
    let checked = dmarc
        .props
        .iter()
        .rev()
        .find(|(k, _)| k == "header.from")
        .map(|(_, v)| v.to_lowercase())
        .unwrap_or_default();
    let aligned =
        dmarc.result == "pass" && from_domain.is_some_and(|d| !d.is_empty() && d == checked);
    Some(Dmarc {
        result: dmarc.result.clone(),
        aligned,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(ar: &[&str], from: &str) -> Mail {
        let mut headers: Vec<(String, String)> = ar
            .iter()
            .map(|a| ("authentication-results".to_string(), a.to_string()))
            .collect();
        headers.push(("from".into(), from.into()));
        Mail {
            headers,
            ..Default::default()
        }
    }
    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }
    fn d(result: &str, aligned: bool) -> Option<Dmarc> {
        Some(Dmarc {
            result: result.into(),
            aligned,
        })
    }
    const OWNER: &str = "Owner <owner@example.org>";

    #[test]
    fn only_the_trusted_authserv_id_counts_a_forged_header_is_ignored() {
        let mx = ids(&["mx.example.org"]);
        let forged = mail(
            &[
                "evil.example.net; dmarc=pass header.from=example.org",
                "mx.example.org; dmarc=fail (p=REJECT) header.from=example.org",
            ],
            OWNER,
        );
        assert_eq!(trusted_dmarc(&forged, &mx), d("fail", false));
        assert_eq!(
            trusted_dmarc(
                &mail(
                    &["evil.example.net; dmarc=pass header.from=example.org"],
                    OWNER
                ),
                &mx
            ),
            None
        );
        assert_eq!(trusted_dmarc(&mail(&[], OWNER), &mx), None);
        assert_eq!(
            trusted_dmarc(
                &mail(
                    &["mx.example.org; dmarc=pass header.from=example.org"],
                    OWNER
                ),
                &[]
            ),
            None
        );
    }

    #[test]
    fn the_topmost_trusted_header_wins_over_a_copy_below_it() {
        let m = mail(
            &[
                "mx.example.org; dmarc=fail header.from=example.org",
                "mx.example.org; dmarc=pass header.from=example.org",
            ],
            OWNER,
        );
        assert_eq!(
            trusted_dmarc(&m, &ids(&["mx.example.org"])),
            d("fail", false)
        );
    }

    #[test]
    fn a_pass_for_another_domain_is_not_aligned_comments_and_case_are_handled() {
        let mx = ids(&["mx.example.org"]);
        assert_eq!(
            trusted_dmarc(
                &mail(&["mx.example.org; dmarc=pass header.from=other.org"], OWNER),
                &mx
            ),
            d("pass", false)
        );
        let fancy = mail(&["MX.Example.ORG 1; (comment; dmarc=fail) DMARC=Pass (p=NONE sp=NONE dis=NONE) header.from=Example.org"], OWNER);
        assert_eq!(
            trusted_dmarc(&fancy, &ids(&["mx2.example.org", "mx.example.org"])),
            d("pass", true)
        );
        // No header.from: never aligned
        assert_eq!(
            trusted_dmarc(&mail(&["mx.example.org; dmarc=pass"], OWNER), &mx),
            d("pass", false)
        );
        // Two From addresses: never aligned
        assert_eq!(
            trusted_dmarc(
                &mail(
                    &["mx.example.org; dmarc=pass header.from=example.org"],
                    "owner@example.org, eve@example.org"
                ),
                &mx
            ),
            d("pass", false)
        );
    }

    #[test]
    fn a_semicolon_inside_quotes_or_comments_splits_nothing_unbalanced_or_doubled_results_are_untrusted(
    ) {
        let verdict = |ar: &str| trusted_dmarc(&mail(&[ar], OWNER), &ids(&["mx.cloudflare.net"]));
        assert_eq!(verdict("mx.cloudflare.net; spf=pass smtp.mailfrom=\"x; dmarc=pass y\"@evil.example; dmarc=fail header.from=example.org"), d("fail", false));
        assert_eq!(
            verdict("mx.cloudflare.net; dmarc=fail (dmarc=pass; x) header.from=example.org"),
            d("fail", false)
        );
        assert_eq!(
            verdict(
                "mx.cloudflare.net; spf=pass (domain of \"a) ; dmarc=pass header.from=example.org"
            ),
            None
        );
        assert_eq!(
            verdict(
                "mx.cloudflare.net; spf=pass smtp.mailfrom=\"a; dmarc=pass header.from=example.org"
            ),
            None
        );
        assert_eq!(
            verdict("mx.cloudflare.net; spf=pass (open; dmarc=pass header.from=example.org"),
            None
        );
        assert_eq!(verdict("mx.cloudflare.net; dmarc=fail header.from=example.org; dmarc=pass header.from=example.org"), None);
        assert_eq!(
            verdict("mx.cloudflare.net; dmarc=pass header.from=example.org; DMARC = fail"),
            None
        );
        assert_eq!(
            verdict("mx.cloudflare.net; spf=pass) dmarc=pass header.from=example.org"),
            None
        );
        // An unreadable trusted header is not skipped for a readable one below it
        let two = mail(
            &[
                "mx.cloudflare.net; spf=pass (x",
                "mx.cloudflare.net; dmarc=pass header.from=example.org",
            ],
            "owner@example.org",
        );
        assert_eq!(trusted_dmarc(&two, &ids(&["mx.cloudflare.net"])), None);
        assert_eq!(
            verdict("mx.cloudflare.net; dmarc=pass header.from=example.org"),
            d("pass", true)
        );
    }

    #[test]
    fn a_wildcard_entry_trusts_numbered_mx_hosts_of_that_domain_only() {
        let fm = ids(&["*.messagingengine.com"]);
        assert_eq!(
            trusted_dmarc(
                &mail(
                    &["mx3.messagingengine.com; dmarc=pass header.from=example.org"],
                    OWNER
                ),
                &fm
            ),
            d("pass", true)
        );
        assert_eq!(
            trusted_dmarc(
                &mail(
                    &["messagingengine.com.evil.example; dmarc=pass header.from=example.org"],
                    OWNER
                ),
                &fm
            ),
            None
        );
        assert_eq!(
            trusted_dmarc(
                &mail(
                    &["evilmessagingengine.com; dmarc=pass header.from=example.org"],
                    OWNER
                ),
                &fm
            ),
            None
        );
    }

    #[test]
    fn methods_versions_and_props_are_read() {
        let r =
            parse_auth_results("mx.example.org; dkim/1=pass header.d=example.org; weird").unwrap();
        assert_eq!(r.authserv_id, "mx.example.org");
        assert_eq!(r.methods[0].method, "dkim");
        assert_eq!(
            r.methods[0].props,
            vec![("header.d".to_string(), "example.org".to_string())]
        );
        assert_eq!(
            r.methods[1],
            Method {
                method: "weird".into(),
                result: String::new(),
                props: vec![]
            }
        );
        assert_eq!(parse_auth_results(""), None);
        assert_eq!(parse_auth_results("; dmarc=pass"), None);
    }
}
