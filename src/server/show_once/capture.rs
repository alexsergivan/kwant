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
/// Run in the recorder's world at a stop: the pending edits' payloads (a list of strings), or an
/// empty list where the recorder isn't installed.
pub const FLUSH: &str =
    "typeof globalThis.__kwntRecorderFlush === 'function' ? globalThis.__kwntRecorderFlush() : []";
/// Called on the select the person picked in kwnt's drawn option list, resolved into the
/// recorder's world: the recorder posts that pick as a `select` step through its binding. `false`
/// where the recorder isn't installed.
pub const PICKED_FN: &str = "function (indexes) { var f = globalThis.__kwntRecorderPicked; \
    return typeof f === 'function' ? f(this, indexes) : false; }";

/// Records a pick the person made through kwnt's drawn option list (`select_option` on the
/// stream). kwnt sets that select itself, so the page sees an untrusted change the recorder
/// ignores; this hands the pick to the recorder in its own world instead, which posts it as a
/// `select` step through the binding like any other (same field facts, same mask). `select` is
/// the select's remote object id in the page's world; `call` sends one CDP command. True when the
/// recorder took it. Only kwnt's server reaches this path: a page script can neither call the
/// recorder's world nor send `select_option`.
pub async fn record_pick<F, Fut>(call: F, select: &str, indexes: &[i64]) -> bool
where
    F: Fn(&'static str, serde_json::Value) -> Fut,
    Fut: std::future::Future<Output = Option<serde_json::Value>>,
{
    use serde_json::json;
    let Some(node) = call("DOM.describeNode", json!({"objectId": select})).await else { return false };
    let Some(backend) = node["node"]["backendNodeId"].as_i64() else { return false };
    let Some(tree) = call("Page.getFrameTree", json!({})).await else { return false };
    let Some(frame) = tree["frameTree"]["frame"]["id"].as_str() else { return false };
    let world = json!({"frameId": frame, "worldName": WORLD});
    let Some(world) = call("Page.createIsolatedWorld", world).await else { return false };
    let Some(context) = world["executionContextId"].as_i64() else { return false };
    let resolve = json!({"backendNodeId": backend, "executionContextId": context});
    let Some(resolved) = call("DOM.resolveNode", resolve).await else { return false };
    let Some(object) = resolved["object"]["objectId"].as_str().map(str::to_string) else { return false };
    let picked = json!({"objectId": object, "functionDeclaration": PICKED_FN, "arguments": [{"value": indexes}], "returnByValue": true});
    let took = call("Runtime.callFunctionOn", picked).await.is_some_and(|r| r["result"]["value"] == true);
    let _ = call("Runtime.releaseObject", json!({"objectId": object})).await;
    took
}

/// Run in the recorder's world before the person's own navigation: the pending edits are posted
/// through the binding as steps. `false` where the recorder isn't installed.
pub const COMMIT: &str =
    "typeof globalThis.__kwntRecorderCommit === 'function' ? (globalThis.__kwntRecorderCommit(), true) : false";

/// The person is leaving the page through kwnt's address bar or back/forward (S-F5). Neither
/// moves focus in the page, so a field typed into and never left would otherwise leave no step.
/// The recorder posts those edits through its binding first, as `type` steps with the field's
/// mask, so a password or code is recorded masked and its value never leaves the page. `call`
/// sends one CDP command. True when the recorder took the commit.
pub async fn commit_pending<F, Fut>(call: F) -> bool
where
    F: Fn(&'static str, serde_json::Value) -> Fut,
    Fut: std::future::Future<Output = Option<serde_json::Value>>,
{
    use serde_json::json;
    let Some(tree) = call("Page.getFrameTree", json!({})).await else { return false };
    let Some(frame) = tree["frameTree"]["frame"]["id"].as_str() else { return false };
    let world = json!({"frameId": frame, "worldName": WORLD});
    let Some(world) = call("Page.createIsolatedWorld", world).await else { return false };
    let Some(context) = world["executionContextId"].as_i64() else { return false };
    let eval = json!({"expression": COMMIT, "contextId": context, "returnByValue": true});
    call("Runtime.evaluate", eval).await.is_some_and(|r| r["result"]["value"] == true)
}

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

    /// Canned CDP replies per method (`None` fails that call).
    type Replies = Vec<(&'static str, Option<serde_json::Value>)>;

    /// `record_pick` against a fake CDP: what it returned and every call it made.
    fn pick_with(replies: Replies) -> (bool, Vec<(String, serde_json::Value)>) {
        let map: std::collections::HashMap<_, _> = replies.into_iter().collect();
        let log = std::sync::Mutex::new(Vec::new());
        let call = |m: &'static str, p: serde_json::Value| {
            log.lock().unwrap().push((m.to_string(), p));
            let r = map.get(m).cloned().flatten();
            async move { r }
        };
        let took = tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(record_pick(call, "page-obj", &[1]));
        (took, log.into_inner().unwrap())
    }

    fn all_ok(took: bool) -> Replies {
        vec![
            ("DOM.describeNode", Some(json!({"node": {"backendNodeId": 42}}))),
            ("Page.getFrameTree", Some(json!({"frameTree": {"frame": {"id": "F1"}}}))),
            ("Page.createIsolatedWorld", Some(json!({"executionContextId": 7}))),
            ("DOM.resolveNode", Some(json!({"object": {"objectId": "rec-obj"}}))),
            ("Runtime.callFunctionOn", Some(json!({"result": {"value": took}}))),
            ("Runtime.releaseObject", Some(json!({}))),
        ]
    }

    #[test]
    fn a_pick_goes_to_the_recorders_world_on_the_same_select() {
        let (took, log) = pick_with(all_ok(true));
        assert!(took);
        let methods: Vec<&str> = log.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(methods, ["DOM.describeNode", "Page.getFrameTree", "Page.createIsolatedWorld", "DOM.resolveNode", "Runtime.callFunctionOn", "Runtime.releaseObject"]);
        assert_eq!(log[0].1, json!({"objectId": "page-obj"}));
        assert_eq!(log[2].1, json!({"frameId": "F1", "worldName": WORLD}), "the recorder's own world");
        assert_eq!(log[3].1, json!({"backendNodeId": 42, "executionContextId": 7}));
        assert_eq!(log[4].1["objectId"], "rec-obj", "called on the select as seen from the recorder's world");
        assert_eq!(log[4].1["functionDeclaration"], PICKED_FN);
        assert_eq!(log[4].1["arguments"], json!([{"value": [1]}]));
        assert_eq!(log[5].1, json!({"objectId": "rec-obj"}));
    }

    #[test]
    fn a_pick_is_not_recorded_without_the_recorder_or_when_the_page_cant_be_reached() {
        // the recorder isn't installed: its function answers false
        assert!(!pick_with(all_ok(false)).0);
        // any step failing stops there
        for failing in ["DOM.describeNode", "Page.getFrameTree", "Page.createIsolatedWorld", "DOM.resolveNode", "Runtime.callFunctionOn"] {
            let replies = all_ok(true).into_iter().map(|(m, r)| (m, if m == failing { None } else { r })).collect();
            let (took, log) = pick_with(replies);
            assert!(!took, "{failing}");
            assert!(!log.iter().any(|(m, _)| m == "Runtime.callFunctionOn") || failing == "Runtime.callFunctionOn", "{failing}");
        }
        assert!(PICKED_FN.contains("typeof f === 'function'"));
    }

    /// `commit_pending` against a fake CDP: what it returned and every call it made.
    fn commit_with(replies: Replies) -> (bool, Vec<(String, serde_json::Value)>) {
        let map: std::collections::HashMap<_, _> = replies.into_iter().collect();
        let log = std::sync::Mutex::new(Vec::new());
        let call = |m: &'static str, p: serde_json::Value| {
            log.lock().unwrap().push((m.to_string(), p));
            let r = map.get(m).cloned().flatten();
            async move { r }
        };
        let took = tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(commit_pending(call));
        (took, log.into_inner().unwrap())
    }

    fn commit_ok(took: bool) -> Replies {
        vec![
            ("Page.getFrameTree", Some(json!({"frameTree": {"frame": {"id": "F1"}}}))),
            ("Page.createIsolatedWorld", Some(json!({"executionContextId": 7}))),
            ("Runtime.evaluate", Some(json!({"result": {"value": took}}))),
        ]
    }

    #[test]
    fn leaving_by_the_address_bar_commits_in_the_recorders_world_only() {
        let (took, log) = commit_with(commit_ok(true));
        assert!(took);
        let methods: Vec<&str> = log.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(methods, ["Page.getFrameTree", "Page.createIsolatedWorld", "Runtime.evaluate"]);
        assert_eq!(log[1].1, json!({"frameId": "F1", "worldName": WORLD}), "the recorder's own world");
        assert_eq!(log[2].1, json!({"expression": COMMIT, "contextId": 7, "returnByValue": true}));
        // no recorder there, or any step failing: nothing taken, and nothing past the failure
        assert!(!commit_with(commit_ok(false)).0);
        for failing in ["Page.getFrameTree", "Page.createIsolatedWorld", "Runtime.evaluate"] {
            let replies = commit_ok(true).into_iter().map(|(m, r)| (m, if m == failing { None } else { r })).collect();
            let (took, log) = commit_with(replies);
            assert!(!took, "{failing}");
            assert_eq!(log.last().unwrap().0, failing);
        }
        assert!(COMMIT.contains("typeof globalThis.__kwntRecorderCommit === 'function'"));
        let js = recorder_js();
        assert!(js.contains("Object.defineProperty(globalThis, '__kwntRecorderCommit'"), "not enumerable, not writable");
    }

    #[test]
    fn the_recorder_names_a_wrapped_field_by_the_labels_own_words() {
        let js = recorder_js();
        assert!(js.contains("function labelText(lab)"));
        assert!(js.contains("out.push(cut(labelText(el.labels[i]), 200))"));
        assert!(!js.contains("el.labels[i].textContent"), "a label's whole text includes its select's options");
        assert!(js.contains("Object.defineProperty(globalThis, '__kwntRecorderPicked'"));
    }

    #[test]
    fn the_script_carries_the_binding_and_the_mask_words() {
        let js = recorder_js();
        assert!(js.contains("globalThis.kwntRecord"));
        assert!(js.contains("[\"passw\",\"password\"]"));
        assert!(!js.contains("__MASK_WORDS__") && !js.contains("__BINDING__"));
    }
}
