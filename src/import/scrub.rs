//! Keys never travel: anything shaped like an API key, token or password is removed from an
//! import before anyone sees it, and fields named like secrets are dropped from source JSON.

use super::Bundle;
use serde_json::Value;

pub const REDACTED: &str = "[removed key]";

/// Words that, in a JSON key, mean its value is a secret.
const SECRET_KEY_WORDS: [&str; 10] =
    ["apikey", "api_key", "api-key", "token", "secret", "password", "passwd", "authorization", "credential", "private_key"];

/// Labels that, written just before a long value, mean the value is a secret (`password: hunter2…`).
const SECRET_LABELS: [&str; 9] =
    ["api_key", "api-key", "apikey", "api key", "token", "secret", "password", "bearer", "authorization"];

/// Removes, at any depth, object fields whose names say they hold a secret (and `env` and
/// `headers`, which carry them in tool configs).
pub fn drop_secret_fields(v: &mut Value) {
    match v {
        Value::Object(map) => {
            map.retain(|k, _| {
                let k = k.to_lowercase();
                k != "env" && k != "headers" && !SECRET_KEY_WORDS.iter().any(|w| k.contains(w))
            });
            map.values_mut().for_each(drop_secret_fields);
        }
        Value::Array(items) => items.iter_mut().for_each(drop_secret_fields),
        _ => {}
    }
}

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')
}

/// Whether one word looks like a key from a known provider, or a JWT.
fn looks_like_key(t: &str) -> bool {
    let long = |min: usize| t.len() >= min;
    let prefixed = |prefixes: &[&str], min: usize| prefixes.iter().any(|p| t.starts_with(p)) && long(min);
    if prefixed(&["sk-", "xai-", "rk_live_", "sk_live_", "sk_test_"], 20)
        || prefixed(&["AIza"], 30)
        || prefixed(&["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "glpat-"], 24)
        || prefixed(&["xoxb-", "xoxp-", "xoxa-", "xoxr-", "xoxs-"], 20)
        || prefixed(&["hf_", "npm_", "ya29."], 30)
    {
        return true;
    }
    // AWS access key id
    if t.len() == 20 && (t.starts_with("AKIA") || t.starts_with("ASIA")) && t.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()) {
        return true;
    }
    // JWT: three base64url parts, the header starting {"
    let parts: Vec<&str> = t.split('.').collect();
    parts.len() == 3 && t.starts_with("eyJ") && long(30) && parts.iter().all(|p| p.len() >= 4)
}

/// A long value with letters and digits right after a secret label.
fn labelled_secret(before: &str, t: &str) -> bool {
    if t.len() < 12 || !t.bytes().any(|b| b.is_ascii_digit()) || !t.bytes().any(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    let mut label = before.trim_end_matches([' ', ':', '=', '"', '\'', '\t']).to_lowercase();
    let mut changed = label.len() != before.len();
    // `password is …`, `token was …`
    for filler in [" is", " was", " are"] {
        if let Some(rest) = label.strip_suffix(filler) {
            label = rest.trim_end().to_string();
            changed = true;
        }
    }
    (changed || label.ends_with("bearer")) && SECRET_LABELS.iter().any(|l| label.ends_with(l))
}

/// `text` with the byte ranges in `spans` (sorted, not overlapping) replaced by [`REDACTED`].
fn redact(text: &str, spans: &[(usize, usize)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for &(a, b) in spans {
        out.push_str(&text[last..a]);
        out.push_str(REDACTED);
        last = b;
    }
    out.push_str(&text[last..]);
    out
}

/// Zero-width and direction characters inside a run of ASCII, where they can only be there to
/// split a key (`sk-abc\u{200B}def`), removed. Elsewhere (emoji joiners) they stay.
fn join_hidden(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let hidden = |c: char| super::is_hidden(c) && !c.is_control();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        if hidden(c) {
            let prev = out.chars().last();
            let next = chars[i + 1..].iter().copied().find(|&n| !hidden(n));
            if prev.is_some_and(|p| p.is_ascii_graphic()) && next.is_some_and(|n| n.is_ascii_graphic()) {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// PEM private key blocks, `-----BEGIN … PRIVATE KEY-----` to the matching END line (or to the
/// end of the text when the END is missing).
fn pem_blocks(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find("-----BEGIN ") {
        let start = from + i;
        let header_end = text[start + 11..].find("-----").map(|j| start + 11 + j + 5);
        let Some(header_end) = header_end.filter(|&h| text[start..h].contains("PRIVATE KEY") && !text[start..h].contains('\n')) else {
            from = start + 11;
            continue;
        };
        let end = text[header_end..]
            .find("-----END ")
            .and_then(|j| {
                let e = header_end + j + 9;
                text[e..].find("-----").map(|k| e + k + 5)
            })
            .unwrap_or(text.len());
        spans.push((start, end));
        from = end;
    }
    spans
}

/// The `user:password` part of `scheme://user:password@host`; the host stays.
fn url_credentials(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find("://") {
        let start = from + i + 3;
        let len = text[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '/' | '?' | '#' | '"' | '\'' | '<' | '>' | ')' | ']' | '`'))
            .unwrap_or(text.len() - start);
        let authority = &text[start..start + len];
        if let Some(at) = authority.rfind('@') {
            let userinfo = &authority[..at];
            if userinfo.split_once(':').is_some_and(|(_, pass)| !pass.is_empty()) {
                spans.push((start, start + at));
            }
        }
        from = start + len;
    }
    spans
}

/// Labels that, followed by `:` or `=`, mean the value after them is a secret.
const VALUE_LABELS: [&str; 10] =
    ["password", "passwd", "pwd", "passphrase", "secret", "token", "api_key", "api-key", "apikey", "api key"];

/// Whether a value written after a secret label is a secret: long enough, with a letter and a
/// digit or symbol, and not a path, address, template or something already removed.
fn secret_value(v: &str) -> bool {
    v.chars().count() >= 8
        && v.chars().any(|c| c.is_alphabetic())
        && v.chars().any(|c| !c.is_alphabetic() && c != ' ')
        && !["[removed key]", "http", "/", "<", "{", "$", "~"].iter().any(|p| v.starts_with(p))
}

/// Values after secret labels, symbols and all: `password: p@ssw0rd!`, `"secret" = "Tr0ub&4"`,
/// `aws_secret_access_key=wJal…`. Quoted values run to the closing quote, others to whitespace.
fn labelled_values(text: &str) -> Vec<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    let b = lower.as_bytes();
    let n = b.len();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < n {
        let at_word = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        let Some(label) = VALUE_LABELS.iter().find(|l| at_word && b[i..].starts_with(l.as_bytes())) else {
            i += 1;
            continue;
        };
        let mut j = i + label.len();
        // the rest of a longer key name: `secret_access_key`, `token_v2`
        let name_end = (j + 30).min(n);
        while j < name_end && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'_' | b'-')) {
            j += 1;
        }
        if j < n && matches!(b[j], b'"' | b'\'') {
            j += 1;
        }
        while j < n && matches!(b[j], b' ' | b'\t') {
            j += 1;
        }
        if j >= n || !matches!(b[j], b':' | b'=') {
            i += 1;
            continue;
        }
        j += 1;
        while j < n && matches!(b[j], b' ' | b'\t' | b'=' | b'>') {
            j += 1;
        }
        let (start, end) = if j < n && matches!(b[j], b'"' | b'\'' | b'`') {
            let q = b[j];
            match b[j + 1..].iter().take(300).position(|&c| c == q || c == b'\n') {
                Some(k) if b[j + 1 + k] == q => (j + 1, j + 1 + k),
                _ => {
                    i += 1;
                    continue;
                }
            }
        } else {
            let end = b[j..].iter().position(|c| c.is_ascii_whitespace()).map_or(n, |k| j + k);
            let value = text[j..end].trim_end_matches(['.', ',', ';', ':', ')', ']', '}']);
            (j, j + value.len())
        };
        if start < end && secret_value(&text[start..end]) {
            spans.push((start, end));
            i = end;
        } else {
            i += 1;
        }
    }
    spans
}

/// A bare 40-character AWS secret access key, only where the 80 characters before it mention
/// AWS or a secret, so ordinary long words and paths stay.
fn aws_secrets(text: &str) -> Vec<(usize, usize)> {
    let b = text.as_bytes();
    let is_b64 = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'/' | b'+');
    let mut spans = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !is_b64(b[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && is_b64(b[i]) {
            i += 1;
        }
        let run = &b[start..i];
        let mixed = run.iter().any(u8::is_ascii_uppercase) && run.iter().any(u8::is_ascii_lowercase) && run.iter().any(u8::is_ascii_digit);
        if run.len() == 40 && mixed {
            let mut from = start.saturating_sub(80);
            while !text.is_char_boundary(from) {
                from += 1;
            }
            let context = text[from..start].to_ascii_lowercase();
            if context.contains("aws") || context.contains("secret") {
                spans.push((start, i));
            }
        }
    }
    spans
}

/// Key-shaped words: provider prefixes, AWS key ids, JWTs, and long values after a label.
fn key_words(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !is_token_byte(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_token_byte(bytes[i]) {
            i += 1;
        }
        // Token bytes are ASCII, so these are char boundaries.
        let word = text[start..i].trim_end_matches(['.', '-']);
        let mut from = start.saturating_sub(40);
        while !text.is_char_boundary(from) {
            from += 1;
        }
        if looks_like_key(word) || labelled_secret(&text[from..start], word) {
            spans.push((start, start + word.len()));
        }
    }
    spans
}

/// One kind of secret: the byte ranges in a text that hold it.
type Pass = fn(&str) -> Vec<(usize, usize)>;

/// `text` with secrets replaced by [`REDACTED`], and how many there were. Text with none comes
/// back exactly as it was.
pub fn scrub_text(text: &str) -> (String, usize) {
    let passes: [Pass; 5] = [pem_blocks, url_credentials, labelled_values, aws_secrets, key_words];
    let mut out = join_hidden(text);
    let mut count = 0;
    for pass in passes {
        let spans = pass(&out);
        if !spans.is_empty() {
            count += spans.len();
            out = redact(&out, &spans);
        }
    }
    if count == 0 {
        (text.to_string(), 0)
    } else {
        (out, count)
    }
}

fn scrub_in_place(s: &mut String) -> usize {
    let (clean, n) = scrub_text(s);
    if n > 0 {
        *s = clean;
    }
    n
}

/// Scrubs every text in the bundle. Returns, per bot ref (`None` = teams, shared skills and
/// the rest), how many keys were removed; only places with removals are listed.
pub fn scrub_bundle(b: &mut Bundle) -> Vec<(Option<String>, usize)> {
    let mut found = Vec::new();
    for bot in &mut b.bots {
        let mut n = 0;
        for s in [&mut bot.name, &mut bot.title, &mut bot.persona_prompt, &mut bot.soul] {
            n += scrub_in_place(s);
        }
        for s in bot.starters.iter_mut().chain(&mut bot.memories).chain(&mut bot.tool_hints).chain(&mut bot.knowledge_files) {
            n += scrub_in_place(s);
        }
        for s in bot.avatar_hint.iter_mut().chain(bot.model_hint.iter_mut()) {
            n += scrub_in_place(s);
        }
        for sk in &mut bot.skills {
            n += scrub_in_place(&mut sk.name) + scrub_in_place(&mut sk.description) + scrub_in_place(&mut sk.body);
        }
        for r in &mut bot.routines {
            n += scrub_in_place(&mut r.name) + scrub_in_place(&mut r.instructions);
            if let Some(t) = &mut r.schedule_text {
                n += scrub_in_place(t);
            }
        }
        if n > 0 {
            found.push((Some(bot.ref_.clone()), n));
        }
    }
    let mut n = 0;
    for t in &mut b.teams {
        n += scrub_in_place(&mut t.name);
        if let Some(i) = &mut t.instructions {
            n += scrub_in_place(i);
        }
    }
    for sk in &mut b.skills {
        n += scrub_in_place(&mut sk.name) + scrub_in_place(&mut sk.description) + scrub_in_place(&mut sk.body);
    }
    for s in b.omitted.iter_mut() {
        n += scrub_in_place(s);
    }
    if let Some(l) = &mut b.source.label {
        n += scrub_in_place(l);
    }
    if n > 0 {
        found.push((None, n));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_keys_are_removed() {
        let text = "Use sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAA and xai-BBBBBBBBBBBBBBBBBBBBBBBB, \
                    AIzaSyCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC or ghp_DDDDDDDDDDDDDDDDDDDDDDDDDDDD.";
        let (out, n) = scrub_text(text);
        assert_eq!(n, 4, "{out}");
        assert!(!out.contains("sk-ant") && !out.contains("xai-B") && !out.contains("AIza") && !out.contains("ghp_"));
        assert!(out.ends_with("[removed key]."));
    }

    #[test]
    fn jwts_and_labelled_secrets_are_removed() {
        let (out, n) = scrub_text("token: eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abcdefghijkl and password = hunter2hunter2X9");
        assert_eq!(n, 2, "{out}");
        assert!(!out.contains("eyJ") && !out.contains("hunter2"));
    }

    #[test]
    fn ordinary_words_stay() {
        let text = "Ask about the sk-learn task; the tokenizer is fine. My password policy is strict.";
        assert_eq!(scrub_text(text), (text.to_string(), 0));
    }

    #[test]
    fn rigels_shapes_are_removed() {
        // IMPORT-QA.md F1: each of these survived the first scrubber.
        let cases = [
            ("pem", "Key:\n-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASC\nabcdEFGH1234\n-----END PRIVATE KEY-----\nThanks.", "MIIE"),
            ("rsa pem", "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA7\n-----END RSA PRIVATE KEY-----", "MIIEp"),
            ("pem without end", "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA", "b3Bl"),
            ("password symbols", "password: p@ssw0rd!Secret#99 then more", "p@ssw0rd"),
            ("quoted password", "password=\"Tr0ub4dor&3xyz\"", "Tr0ub4dor"),
            ("passwd", "passwd = s3cr3t!pass", "s3cr3t"),
            ("pwd", "pwd: Hunter#2024x", "Hunter#"),
            ("secret label", "secret: 9f8e7d6c5b4a!", "9f8e7d"),
            ("json token", "{\"token\": \"abc123/def456+ghi\"}", "abc123"),
            ("url creds", "Clone https://user:SuperSecret123@git.example.com/x.git", "SuperSecret"),
            ("aws secret", "aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "wJalr"),
            ("bare aws secret", "My AWS keys: AKIAIOSFODNN7EXAMPLE and wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "wJalr"),
            ("npm", "npm_aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789 is the publish token", "npm_aB"),
            ("google oauth", "Use ya29.a0AfH6SMBx-EXAMPLE_token_value_1234567890 for Drive", "ya29"),
            ("zero-width split", "sk-proj-abcdefgh\u{200B}ijklmnopqrstuvwxyz0123", "ijklmnop"),
        ];
        for (what, text, secret) in cases {
            let (out, n) = scrub_text(text);
            assert!(n >= 1 && !out.contains(secret), "{what}: {out}");
        }
        let (out, _) = scrub_text("Clone https://user:SuperSecret123@git.example.com/x.git");
        assert_eq!(out, "Clone https://[removed key]@git.example.com/x.git");
        let (out, n) = scrub_text("Key:\n-----BEGIN PRIVATE KEY-----\nMIIE\n-----END PRIVATE KEY-----\nThanks.");
        assert_eq!((out.as_str(), n), ("Key:\n[removed key]\nThanks.", 1));
    }

    #[test]
    fn ordinary_prose_survives_unchanged() {
        let text = "You are a friendly tutor. Answer in the \"format\" the student asks for.\n\
                    Password rules: at least 12 characters. The secret: keep answers short.\n\
                    Token limits: 4096 tokens. \"max_tokens\": 4096, \"password_min_length\": 12.\n\
                    Docs live at https://docs.example.com/guide?x=1 and mail me at help@example.com.\n\
                    pwd: /home/maus/work. Use ${API_KEY} from the environment; token = <your token>.\n\
                    AWS regions: us-east-1. A long path: /usr/share/doc/packages/something/really/long/x.\n\
                    Family emoji 👨\u{200D}👩\u{200D}👧 stays, and so does the sk-learn library.\n\
                    -----BEGIN CERTIFICATE----- is public, fine to keep.";
        assert_eq!(scrub_text(text), (text.to_string(), 0));
    }

    #[test]
    fn multibyte_text_around_labels_never_panics() {
        for lead in ["", "é", "👨", "ж", "\u{200B}", "\u{202E}"] {
            for body in ["password: ", "token=\"", "https://u:", "aws ", "-----BEGIN PRIVATE KEY-----", "sk-"] {
                for tail in ["", "é", "Пароль123!", "👨\u{200D}👩", "\u{200B}abc"] {
                    let text = format!("{lead}{body}{lead}{tail}{lead}");
                    let _ = scrub_text(&text);
                }
            }
        }
    }

    #[test]
    fn secret_fields_are_dropped() {
        let mut v = serde_json::json!({"name": "x", "apiKey": "abc", "nested": [{"env": {"A": "b"}, "Authorization": "c", "ok": 1}]});
        drop_secret_fields(&mut v);
        assert_eq!(v, serde_json::json!({"name": "x", "nested": [{"ok": 1}]}));
    }
}
