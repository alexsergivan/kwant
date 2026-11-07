//! Turns what the recorder in the page sends into a step. Everything in a payload is untrusted
//! (the page chose the labels and the text): it's size-checked, cleaned and capped here, the
//! address goes through [`scrub`], and the mask is decided again by [`mask_for`], so a value the
//! page sent for a masked field is dropped, never stored.

use super::mask::{mask_for, Field, Mask, MASK_WORDS};
use super::scrub::{scrub, PageAt};
use crate::db::show_once::NewStep;
use serde::Deserialize;

/// The binding the recorder calls, and the isolated world it lives in.
pub const BINDING: &str = "kwntRecord";
pub const WORLD: &str = "kwnt-recorder";
/// Largest payload read; anything bigger is dropped.
pub const MAX_PAYLOAD: usize = 16 * 1024;
pub const MAX_NAME: usize = 120;
pub const MAX_VALUE: usize = 500;
pub const MAX_NOTE: usize = 1000;
/// Keys recorded as their own step.
pub const KEYS: [&str; 3] = ["Enter", "Tab", "Escape"];

/// The recorder's script, with the binding and the mask words filled in.
pub fn recorder_js() -> String {
    let words: Vec<(&str, &str)> = MASK_WORDS.iter().map(|(w, m)| (*w, m.as_str())).collect();
    include_str!("recorder.js")
        .replace("__BINDING__", BINDING)
        .replace("__MASK_WORDS__", &serde_json::to_string(&words).unwrap_or_else(|_| "[]".into()))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FieldIn {
    #[serde(rename = "type")]
    input_type: String,
    ever_password: bool,
    autocomplete: String,
    name: String,
    id: String,
    labels: Vec<String>,
    script_filled: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Payload {
    kind: String,
    trusted: bool,
    url: String,
    role: Option<String>,
    name: Option<String>,
    label: Option<String>,
    value: Option<String>,
    masked: Option<String>,
    field: Option<FieldIn>,
}

/// A recorded action and the page it happened on.
#[derive(Debug, Clone, PartialEq)]
pub struct Captured {
    pub page: PageAt,
    pub step: NewStep,
}

/// The step in a recorder payload, or `None` for anything that isn't one: too big, not JSON,
/// not from the person (`trusted` false), an unknown kind, or a page that isn't recorded.
pub fn parse(payload: &str) -> Option<Captured> {
    if payload.len() > MAX_PAYLOAD {
        return None;
    }
    let p: Payload = serde_json::from_str(payload).ok()?;
    if !p.trusted {
        return None;
    }
    let page = scrub(&p.url)?;
    let mut step = NewStep { kind: p.kind.clone(), origin: Some(page.origin.clone()), path: Some(page.path.clone()), ..Default::default() };
    match p.kind.as_str() {
        "click" => {
            step.role = Some(clean(p.role.as_deref().unwrap_or("generic"), 40)).filter(|r| !r.is_empty());
            step.name = Some(clean(p.name.as_deref().unwrap_or_default(), MAX_NAME));
        }
        "key" => {
            let key = p.value.as_deref().filter(|k| KEYS.contains(k))?;
            step.value = Some(key.to_string());
        }
        "type" | "select" | "upload" => {
            let f = p.field.unwrap_or_default();
            let labels: Vec<&str> = f.labels.iter().map(String::as_str).collect();
            let field = Field {
                input_type: &f.input_type,
                ever_password: f.ever_password,
                autocomplete: &f.autocomplete,
                name: &f.name,
                id: &f.id,
                labels: &labels,
                script_filled: f.script_filled,
            };
            let label = p.label.as_deref().filter(|l| !l.trim().is_empty()).or(labels.first().copied()).unwrap_or(&f.name);
            step.label = Some(clean(label, MAX_NAME));
            // upload: the label only, never the file's name (S-C13)
            if p.kind != "upload" {
                let mask = first(mask_for(&field), p.masked.as_deref().and_then(Mask::parse));
                match mask {
                    Some(m) => step.masked = Some(m.as_str().to_string()),
                    None => step.value = Some(clean(p.value.as_deref().unwrap_or_default(), MAX_VALUE)),
                }
            }
        }
        _ => return None,
    }
    Some(Captured { page, step })
}

/// The earlier of two masks in precedence order (the server's and the page's own verdict).
fn first(a: Option<Mask>, b: Option<Mask>) -> Option<Mask> {
    let rank = |m: Mask| [Mask::Password, Mask::Code, Mask::Payment, Mask::Secret, Mask::SavedLogin].iter().position(|x| *x == m);
    match (a, b) {
        (Some(a), Some(b)) => Some(if rank(b) < rank(a) { b } else { a }),
        (a, b) => a.or(b),
    }
}

/// Untrusted text as a step keeps it: control characters out, whitespace runs collapsed, cut to
/// `max` characters with "…" (S-C17, S-L6).
pub fn clean(s: &str, max: usize) -> String {
    let collapsed = s
        .chars()
        .map(|c| if c.is_control() || c.is_whitespace() { ' ' } else { c })
        .collect::<String>()
        .split(' ')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.chars().count() <= max {
        collapsed
    } else {
        let mut out: String = collapsed.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn typed(field: serde_json::Value, value: &str) -> Captured {
        parse(&json!({"kind":"type","trusted":true,"url":"https://shop.example.com/login?next=/x#f","field":field,"value":value}).to_string()).unwrap()
    }

    #[test]
    fn a_plain_field_keeps_its_value_and_a_clean_address() {
        let c = typed(json!({"type":"email","name":"email","labels":["Email"]}), "sam@example.com");
        assert_eq!(c.step.value.as_deref(), Some("sam@example.com"));
        assert_eq!(c.step.label.as_deref(), Some("Email"));
        assert_eq!(c.step.masked, None);
        assert_eq!((c.page.origin.as_str(), c.page.path.as_str()), ("https://shop.example.com", "/login"));
    }

    #[test]
    fn the_server_masks_even_when_the_page_sent_a_value() {
        for (field, kind) in [
            (json!({"type":"password","name":"pw"}), "password"),
            (json!({"type":"text","ever_password":true}), "password"),
            (json!({"type":"text","autocomplete":"one-time-code"}), "code"),
            (json!({"type":"text","autocomplete":"cc-number"}), "payment"),
            (json!({"type":"text","labels":["API token"]}), "secret"),
            (json!({"type":"text","name":"username","script_filled":true}), "saved_login"),
        ] {
            let c = typed(field.clone(), "hunter2");
            assert_eq!(c.step.masked.as_deref(), Some(kind), "{field}");
            assert_eq!(c.step.value, None, "{field}: a masked value is never kept");
        }
    }

    #[test]
    fn the_pages_own_mask_verdict_is_kept_and_the_earlier_rule_wins() {
        // the page saw something the server's facts don't show: still masked
        let c = parse(&json!({"kind":"type","trusted":true,"url":"https://a.example/","field":{"name":"q"},"masked":"saved_login","value":"x"}).to_string()).unwrap();
        assert_eq!(c.step.masked.as_deref(), Some("saved_login"));
        assert_eq!(c.step.value, None);
        // both: password beats saved_login
        let c = parse(&json!({"kind":"type","trusted":true,"url":"https://a.example/","field":{"type":"password"},"masked":"saved_login","value":"x"}).to_string()).unwrap();
        assert_eq!(c.step.masked.as_deref(), Some("password"));
        // a made-up mask word from the page is ignored, not trusted as "no mask"
        let c = parse(&json!({"kind":"type","trusted":true,"url":"https://a.example/","field":{"name":"pin"},"masked":"none","value":"1234"}).to_string()).unwrap();
        assert_eq!(c.step.masked.as_deref(), Some("code"));
    }

    #[test]
    fn untrusted_oversized_and_unknown_payloads_are_dropped() {
        assert!(parse(&json!({"kind":"click","trusted":false,"url":"https://a.example/"}).to_string()).is_none());
        assert!(parse(&json!({"kind":"click","url":"https://a.example/"}).to_string()).is_none());
        assert!(parse(&json!({"kind":"exec","trusted":true,"url":"https://a.example/"}).to_string()).is_none());
        assert!(parse(&json!({"kind":"click","trusted":true,"url":"data:text/html,hi"}).to_string()).is_none());
        assert!(parse("not json").is_none());
        let big = json!({"kind":"click","trusted":true,"url":"https://a.example/","name":"x".repeat(MAX_PAYLOAD)}).to_string();
        assert!(parse(&big).is_none());
        assert!(parse(&json!({"kind":"key","trusted":true,"url":"https://a.example/","value":"a"}).to_string()).is_none(), "only Enter, Tab and Escape");
    }

    #[test]
    fn uploads_keep_the_label_only_and_selects_the_option_label() {
        let c = parse(&json!({"kind":"upload","trusted":true,"url":"https://a.example/up","field":{"labels":["Invoice"]},"value":"C:/secret/plan.pdf"}).to_string()).unwrap();
        assert_eq!((c.step.label.as_deref(), c.step.value.as_deref()), (Some("Invoice"), None));
        let c = parse(&json!({"kind":"select","trusted":true,"url":"https://a.example/","field":{"labels":["Range"]},"value":"Last 7 days"}).to_string()).unwrap();
        assert_eq!(c.step.value.as_deref(), Some("Last 7 days"));
    }

    #[test]
    fn text_is_cleaned_and_capped() {
        assert_eq!(clean("  a\u{0}\n\tb   c ", 100), "a b c");
        assert_eq!(clean(&"x".repeat(130), MAX_NAME).chars().count(), MAX_NAME);
        assert!(clean(&"x".repeat(130), MAX_NAME).ends_with('…'));
        let c = parse(&json!({"kind":"click","trusted":true,"url":"https://a.example/","role":"button","name":"y".repeat(400)}).to_string()).unwrap();
        assert_eq!(c.step.name.unwrap().chars().count(), MAX_NAME);
        let c = typed(json!({"name":"note"}), &"z".repeat(900));
        assert_eq!(c.step.value.unwrap().chars().count(), MAX_VALUE);
    }

    #[test]
    fn the_script_carries_the_binding_and_the_mask_words() {
        let js = recorder_js();
        assert!(js.contains("globalThis.kwntRecord"));
        assert!(js.contains("[\"passw\",\"password\"]"));
        assert!(!js.contains("__MASK_WORDS__") && !js.contains("__BINDING__"));
    }
}
