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
        || prefixed(&["hf_"], 30)
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

/// `text` with key-like words replaced by [`REDACTED`], and how many there were.
pub fn scrub_text(text: &str) -> (String, usize) {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut count = 0;
    let mut last = 0;
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
        let end = start + word.len();
        let mut from = start.saturating_sub(40);
        while !text.is_char_boundary(from) {
            from += 1;
        }
        let before = &text[from..start];
        if looks_like_key(word) || labelled_secret(before, word) {
            out.push_str(&text[last..start]);
            out.push_str(REDACTED);
            last = end;
            count += 1;
        }
    }
    out.push_str(&text[last..]);
    (out, count)
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
    fn secret_fields_are_dropped() {
        let mut v = serde_json::json!({"name": "x", "apiKey": "abc", "nested": [{"env": {"A": "b"}, "Authorization": "c", "ok": 1}]});
        drop_secret_fields(&mut v);
        assert_eq!(v, serde_json::json!({"name": "x", "nested": [{"ok": 1}]}));
    }
}
