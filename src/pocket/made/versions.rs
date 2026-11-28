//! pa12 L5: versions of a bot-made app and the app document (plan.md section 4,
//! security-review.md sections 3 and 5, L5/brief.md).
//!
//! A version is a `made/1` spec plus, per surface (`phone`, `screen`), the bot's body markup, one
//! stylesheet and one script. When a draft is made, each surface becomes one whole document
//! ([`document`]): kwant's prelude, kwant's pocket.css, the app's style, the app's markup and the
//! app's script, all inline. The CSP hashes are taken over exactly those strings at that moment and
//! stored with the version, and the guest port serves the stored document byte for byte, so the
//! hashes always match the bytes served (a later kwant with another prelude or pocket.css changes
//! only versions made after it).
//!
//! States: `draft` → `checking` → `passed` | `failed`; only `passed` goes live, under the pocket's
//! lock, and the live one before it becomes `retired`. Rollback makes the newest earlier version
//! that passed live again; the rolled-back one becomes `failed` ("Rolled back by the person."), so
//! no later rollback picks it, and its document answers 404 like any version that isn't live. A pocket keeps at most [`MAX_APP_VERSIONS`].
//!
//! The additive rule: once a pocket has guests, a new version may add collections and fields, but
//! not remove a collection that has rows, change the type of one of its fields, or change who may
//! read it ("start a new pocket for that"). So an older version can always read the newer state,
//! which keeps rollback safe.

use super::super::kits::{self, Kit};
use super::super::{snapshot, PResult, PocketError};
use super::{collections, Made};
use crate::db::pocket::{AppVersionInfo, NewAppVersion, ENDED};
pub use crate::db::pocket::MAX_APP_VERSIONS;
use crate::server::AppState;
use axum::http::StatusCode;
use base64::Engine;
use chrono::Utc;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Version states, as stored.
pub const DRAFT: &str = "draft";
pub const CHECKING: &str = "checking";
pub const PASSED: &str = "passed";
pub const FAILED: &str = "failed";
pub const LIVE: &str = "live";
pub const RETIRED: &str = "retired";

/// The report a rolled-back version gets (its state becomes `failed`).
pub const ROLLED_BACK: &str = "Rolled back by the person.";

/// The app's html, css and js together, per surface (plan.md section 5).
pub const MAX_SCREEN_BYTES: usize = 120 * 1024;

/// kwant's pocket.css (Mira's style guide section 2), copied byte for byte from the mockup kit's
/// build (13,240 bytes). Never edit it by hand: rebuild it there and repin [`POCKET_CSS_SHA256`].
pub const POCKET_CSS: &str = include_str!("pocket.css");
/// The pin for [`POCKET_CSS`] (CSP form of sha256 c1160680…0487).
pub const POCKET_CSS_SHA256: &str = "wRYGgOsZBVhyqVo9OvtlSLECNiMk7CQ4NF1fPNzeBIc=";

/// kwant's prelude, the first script in every app document (style guide section 3, L5/inputs.md).
/// - It removes WebRTC's constructors before any app code runs (CSP doesn't cover WebRTC).
/// - Opened outside a frame (T15), it hides the page and gives the app no `kwant`.
/// - `kwant.onState(fn)` takes only `{type:"state"}` messages from the parent page. The prelude
///   keeps the last state: `fn` registered after it came is called with it at once, and every
///   listener gets each later state once.
/// - `kwant.act(name, body)` posts `{type:"act", name, body}` to the parent.
/// - A CSP violation posts `{type:"violation", what}`, so the page can drop the frame.
///
/// There is no timer and nothing is posted again: the page posts each state once.
pub const PRELUDE: &str = r#"(function(){"use strict";var w=window,P=w.parent,last,has=false,fns=[];
["RTCPeerConnection","webkitRTCPeerConnection","mozRTCPeerConnection","RTCDataChannel"].forEach(function(k){try{delete w[k]}catch(e){}});
if(w.top===w){document.documentElement.hidden=true;return}
w.addEventListener("message",function(e){var d=e.data;if(e.source!==P||!d||d.type!=="state")return;last=d;has=true;
for(var i=0;i<fns.length;i++)fns[i](d)});
w.addEventListener("securitypolicyviolation",function(e){P.postMessage({type:"violation",what:String(e.effectiveDirective||"")},"*")});
Object.defineProperty(w,"kwant",{value:Object.freeze({
onState:function(fn){if(typeof fn!=="function")return;fns.push(fn);if(has)fn(last)},
act:function(name,body){P.postMessage({type:"act",name:String(name),body:body===undefined?null:body},"*")}})});
})();"#;

/// Where an app document is shown: a guest's phone or the big screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Phone,
    Screen,
}

impl Surface {
    pub fn parse(s: &str) -> Option<Surface> {
        match s {
            "phone" => Some(Surface::Phone),
            "screen" => Some(Surface::Screen),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Surface::Phone => "phone",
            Surface::Screen => "screen",
        }
    }
}

/// What the bot wrote for one surface.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Screen {
    pub html: String,
    pub css: String,
    pub js: String,
}

/// A new version: the spec and the screens (a surface with none has no document).
#[derive(Debug, Clone, Default)]
pub struct Draft {
    pub spec: Value,
    pub phone: Option<Screen>,
    pub screen: Option<Screen>,
    /// The bot that made it (its kwant id), for the person's list.
    pub made_by: Option<String>,
}

/// The CSP form of a string's sha256 (base64, no prefix).
pub fn sha256_b64(s: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(Sha256::digest(s.as_bytes()))
}

/// The four hashes of one surface's document, in document order.
#[derive(Debug, Clone, PartialEq)]
pub struct Hashes {
    pub prelude: String,
    pub pocket_css: String,
    pub app_css: String,
    pub app_js: String,
}

impl Hashes {
    fn to_json(&self) -> Value {
        json!({ "script": [self.prelude, self.app_js], "style": [self.pocket_css, self.app_css] })
    }

    fn from_json(v: &Value) -> Option<Hashes> {
        let s = |k: &str, i: usize| v[k][i].as_str().map(str::to_string);
        Some(Hashes { prelude: s("script", 0)?, app_js: s("script", 1)?, pocket_css: s("style", 0)?, app_css: s("style", 1)? })
    }
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    hay.to_ascii_lowercase().contains(needle)
}

/// The app document for one surface and the hashes of its inline script and style bodies, taken
/// over exactly the strings put in. `Err` is one plain sentence for the bot: a part that would
/// end its element early (or start another) is refused, so each inline body is exactly the string
/// hashed. Everything else in the code is K2's lint.
pub fn document(surface: Surface, s: &Screen) -> Result<(String, Hashes), String> {
    let name = surface.as_str();
    if s.html.len() + s.css.len() + s.js.len() > MAX_SCREEN_BYTES {
        return Err(format!("The {name} screen is bigger than 120 KB of html, css and js together."));
    }
    if contains_ci(&s.html, "<script") || contains_ci(&s.html, "<style") {
        return Err(format!("The {name} screen's html can't have a <script> or <style>: put them in js and css."));
    }
    if contains_ci(&s.css, "</style") {
        return Err(format!("The {name} screen's css can't contain \"</style\"."));
    }
    if contains_ci(&s.js, "</script") || s.js.contains("<!--") {
        return Err(format!("The {name} screen's js can't contain \"</script\" or \"<!--\"."));
    }
    let doc = format!(
        "<!doctype html><html data-surface=\"{name}\"><head><meta charset=\"utf-8\"><script>{PRELUDE}</script><style>{POCKET_CSS}</style><style>{}</style></head><body>{}<script>{}</script></body></html>",
        s.css, s.html, s.js
    );
    let hashes = Hashes { prelude: sha256_b64(PRELUDE), pocket_css: sha256_b64(POCKET_CSS), app_css: sha256_b64(&s.css), app_js: sha256_b64(&s.js) };
    Ok((doc, hashes))
}

/// security-review.md section 3's Content-Security-Policy for a document with these hashes. No
/// `'self'` anywhere: in an opaque-origin document engines differ on what it matches.
pub fn csp(h: &Hashes) -> String {
    format!(
        "sandbox allow-scripts; default-src 'none'; script-src 'sha256-{}' 'sha256-{}'; style-src 'sha256-{}' 'sha256-{}'; \
         img-src data: blob:; font-src data:; connect-src 'none'; media-src 'none'; frame-src 'none'; worker-src 'none'; \
         object-src 'none'; form-action 'none'; base-uri 'none'; frame-ancestors 'self'; require-trusted-types-for 'script'",
        h.prelude, h.app_js, h.pocket_css, h.app_css
    )
}

/// security-review.md section 3's Permissions-Policy.
pub const PERMISSIONS_POLICY: &str = "camera=(), microphone=(), geolocation=(), payment=(), usb=(), serial=(), bluetooth=(), hid=(), \
     publickey-credentials-get=(), publickey-credentials-create=(), clipboard-read=(), display-capture=(), screen-wake-lock=(), \
     web-share=(), fullscreen=()";

/// The additive rule (plan.md section 4). `live` is the pocket's spec now, `next` the new
/// version's, `state` the kit state; `has_guests` once anyone has joined. `Err` is one sentence.
pub fn additive(live: &Value, next: &Value, state: &Value, has_guests: bool) -> Result<(), String> {
    if !has_guests {
        return Ok(());
    }
    for old in collections::all(live) {
        if collections::rows(state, old.name).is_empty() {
            continue;
        }
        let refuse = |what: &str| Err(format!("\"{}\" has rows, so a new version can't {what} while guests are in: start a new pocket for that.", old.name));
        let Some(new) = collections::find(next, old.name) else { return refuse("remove it") };
        if new.read != old.read {
            return refuse("change who can read it");
        }
        let empty = serde_json::Map::new();
        let new_fields = new.fields.unwrap_or(&empty);
        for (field, def) in old.fields.unwrap_or(&empty) {
            let same = new_fields.get(field).is_some_and(|d| d["type"] == def["type"] && same_of(def, d));
            if !same {
                return refuse(&format!("remove or retype its field \"{field}\""));
            }
        }
    }
    Ok(())
}

/// A row field points at the same collection; a choice keeps every option it had (it may add some).
fn same_of(old: &Value, new: &Value) -> bool {
    match (old["of"].as_array(), new["of"].as_array()) {
        (Some(was), Some(now)) => was.iter().all(|o| now.contains(o)),
        _ => old["of"] == new["of"],
    }
}

fn bad_screen(m: String) -> PocketError {
    PocketError::new(StatusCode::BAD_REQUEST, "bad_screen", m)
}

fn not_additive(m: String) -> PocketError {
    PocketError::new(StatusCode::CONFLICT, "not_additive", m)
}

fn no_such_version() -> PocketError {
    PocketError::new(StatusCode::NOT_FOUND, "no_such_version", "That version isn't in this pocket.")
}

fn ended() -> PocketError {
    PocketError::new(StatusCode::GONE, "ended", "This pocket app has ended.")
}

/// The pocket, if it's a `made` one (anything else is a missing code).
async fn made_pocket(state: &AppState, code: &str) -> PResult<std::sync::Arc<super::super::Snapshot>> {
    let s = snapshot(state, code).await?.ok_or_else(PocketError::not_found)?;
    if s.row.kit != kits::MADE_ID {
        return Err(PocketError::not_found());
    }
    Ok(s)
}

/// Makes a draft version: checks the spec and the additive rule against the pocket now, builds
/// each surface's document and hashes, stores them, and drops the oldest versions past the cap.
/// Returns the new version number. Guests stay on the live version.
pub async fn draft(state: &AppState, code: &str, d: Draft) -> PResult<i64> {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let s = made_pocket(state, code).await?;
    if s.row.state == ENDED {
        return Err(ended());
    }
    Made.validate_spec(&d.spec).map_err(|m| PocketError::new(StatusCode::BAD_REQUEST, "bad_spec", m))?;
    additive(&s.spec, &d.spec, &s.kit_state, !s.rows.is_empty()).map_err(not_additive)?;
    let mut docs: [Option<String>; 2] = [None, None];
    let mut hashes = serde_json::Map::new();
    for (i, (surface, screen)) in [(Surface::Phone, &d.phone), (Surface::Screen, &d.screen)].into_iter().enumerate() {
        if let Some(screen) = screen {
            let (doc, h) = document(surface, screen).map_err(bad_screen)?;
            docs[i] = Some(doc);
            hashes.insert(surface.as_str().into(), h.to_json());
        }
    }
    let spec_text = serde_json::to_string(&d.spec).unwrap_or_default();
    let hashes_text = Value::Object(hashes).to_string();
    let version = state
        .db
        .pocket_app_version_insert(NewAppVersion {
            code,
            spec: &spec_text,
            phone_doc: docs[0].as_deref(),
            screen_doc: docs[1].as_deref(),
            hashes: &hashes_text,
            made_by: d.made_by.as_deref(),
            now: Utc::now(),
        })
        .await?;
    Ok(version)
}

/// The check's progress on a draft: `checking`, then `passed` or `failed` with its report.
/// 409 `version_moved_on` for a version that is already passed, failed, live or retired.
pub async fn set_check(state: &AppState, code: &str, version: i64, to: &str, report: Option<&str>) -> PResult<()> {
    if ![CHECKING, PASSED, FAILED].contains(&to) {
        return Err(PocketError::new(StatusCode::BAD_REQUEST, "bad_version_state", "A check sets checking, passed or failed."));
    }
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    made_pocket(state, code).await?;
    state.db.pocket_app_version(code, version).await?.ok_or_else(no_such_version)?;
    if !state.db.pocket_app_version_set_state(code, version, to, report).await? {
        return Err(PocketError::new(StatusCode::CONFLICT, "version_moved_on", "That version's check is over."));
    }
    Ok(())
}

/// Makes `version` live if `from` allows its state; re-checks the additive rule against the pocket
/// now (guests may have joined since the draft). Caller holds the lock.
async fn make_live(state: &AppState, s: &super::super::Snapshot, version: i64, from: &[&str], old: (&str, Option<&str>)) -> PResult<()> {
    let code = &s.row.code;
    let row = state.db.pocket_app_version(code, version).await?.ok_or_else(no_such_version)?;
    if !from.contains(&row.state.as_str()) {
        return Err(PocketError::new(StatusCode::CONFLICT, "not_passed", "Only a version that passed its check can go live."));
    }
    let spec: Value = serde_json::from_str(&row.spec).unwrap_or(Value::Null);
    // A rollback goes back to an older version, which reads the newer state (that's the rule's point)
    if from == [PASSED] {
        additive(&s.spec, &spec, &s.kit_state, !s.rows.is_empty()).map_err(not_additive)?;
    }
    let title = Made.title(&spec);
    if !state.db.pocket_app_make_live(code, version, from, old, &row.spec, &title).await? {
        return Err(PocketError::new(StatusCode::CONFLICT, "not_passed", "Only a version that passed its check can go live."));
    }
    super::super::changed(state, code).await
}

/// Makes a version that passed its check the live one. Guests' views name it as `app.version`
/// from their next state; the version before it answers 404 from then on.
pub async fn go_live(state: &AppState, code: &str, version: i64) -> PResult<()> {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let s = made_pocket(state, code).await?;
    if s.row.state == ENDED {
        return Err(ended());
    }
    make_live(state, &s, version, &[PASSED], (RETIRED, None)).await
}

/// One tap on the card: the newest version older than the live one that passed its check (and
/// may have been live since, `retired`) becomes live; the live one becomes `failed`. Returns its number; 409 `nothing_to_roll_back` when
/// there's none.
pub async fn rollback(state: &AppState, code: &str) -> PResult<i64> {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let s = made_pocket(state, code).await?;
    if s.row.state == ENDED {
        return Err(ended());
    }
    let nothing = || PocketError::new(StatusCode::CONFLICT, "nothing_to_roll_back", "There's no earlier version that passed its check.");
    let live = state.db.pocket_app_live_version(code).await?.ok_or_else(nothing)?;
    let target = state
        .db
        .pocket_app_versions(code)
        .await?
        .into_iter()
        .find(|v| v.version < live && (v.state == PASSED || v.state == RETIRED))
        .ok_or_else(nothing)?;
    // The one rolled back is the person's "no": it never goes live again, nor back to by a later rollback
    make_live(state, &s, target.version, &[PASSED, RETIRED], (FAILED, Some(ROLLED_BACK))).await?;
    Ok(target.version)
}

/// The person's list, newest first, and the live version's number.
pub async fn list(state: &AppState, code: &str) -> PResult<(Vec<AppVersionInfo>, Option<i64>)> {
    made_pocket(state, code).await?;
    let versions = state.db.pocket_app_versions(code).await?;
    let live = versions.iter().find(|v| v.state == LIVE).map(|v| v.version);
    Ok((versions, live))
}

/// The stored document and its CSP, only for the live version and a surface it has.
pub async fn live_document(state: &AppState, code: &str, version: i64, surface: Surface) -> PResult<Option<(String, String)>> {
    let Some(row) = state.db.pocket_app_version(code, version).await? else { return Ok(None) };
    if row.state != LIVE {
        return Ok(None);
    }
    let doc = match surface {
        Surface::Phone => row.phone_doc,
        Surface::Screen => row.screen_doc,
    };
    let hashes: Value = serde_json::from_str(&row.hashes).unwrap_or(Value::Null);
    Ok(doc.zip(Hashes::from_json(&hashes[surface.as_str()])).map(|(doc, h)| (doc, csp(&h))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pocket_css_is_the_pinned_build() {
        assert_eq!(POCKET_CSS.len(), 13_240);
        assert_eq!(sha256_b64(POCKET_CSS), POCKET_CSS_SHA256);
        assert!(!contains_ci(POCKET_CSS, "</style"));
    }

    #[test]
    fn the_prelude_buffers_has_no_timer_and_ends_no_element() {
        for banned in ["setTimeout", "setInterval", "requestAnimationFrame", "</script", "<!--", "fetch", "XMLHttpRequest", "WebSocket"] {
            assert!(!PRELUDE.contains(banned), "{banned}");
        }
        // (a) a late listener is called with the held state at once; (b) each state reaches every listener once
        assert!(PRELUDE.contains("fns.push(fn);if(has)fn(last)"));
        assert!(PRELUDE.contains("last=d;has=true;\nfor(var i=0;i<fns.length;i++)fns[i](d)"));
        assert!(PRELUDE.contains("d.type!==\"state\"") && PRELUDE.contains("e.source!==P"));
        assert!(PRELUDE.contains("{type:\"act\",name:String(name)"));
        assert!(PRELUDE.contains("delete w[k]") && PRELUDE.contains("\"RTCPeerConnection\""));
    }

    fn screen() -> Screen {
        Screen { html: "<main class=\"stack\"><h1>Hi</h1></main>".into(), css: ".x{color:red}".into(), js: "kwant.onState(function(s){})".into() }
    }

    #[test]
    fn the_document_is_in_order_and_hashed_over_the_exact_strings() {
        let (doc, h) = document(Surface::Screen, &screen()).unwrap();
        assert!(doc.starts_with("<!doctype html><html data-surface=\"screen\"><head><meta charset=\"utf-8\"><script>"));
        let at = |part: &str| doc.find(part).unwrap();
        assert!(at(PRELUDE) < at(POCKET_CSS) && at(POCKET_CSS) < at(".x{color:red}") && at(".x{color:red}") < at("<main") && at("<main") < at("kwant.onState(function(s)"));
        assert_eq!(h.app_js, sha256_b64("kwant.onState(function(s){})"));
        assert_eq!(h.pocket_css, POCKET_CSS_SHA256);
        let c = csp(&h);
        assert_eq!(c.matches("'self'").count(), 1, "only frame-ancestors names 'self': {c}");
        assert!(c.contains("frame-ancestors 'self'") && !c.contains("font-src 'self'"));
        assert!(c.starts_with("sandbox allow-scripts; default-src 'none'; script-src 'sha256-"));
    }

    #[test]
    fn a_part_that_would_end_its_element_or_is_too_big_is_refused() {
        for (html, css, js) in [
            ("<SCRIPT>x</SCRIPT>", "", ""),
            ("<style>x</style>", "", ""),
            ("", "a{}</Style><script>x</script>", ""),
            ("", "", "var a='</script><script>x';"),
            ("", "", "<!--"),
        ] {
            let s = Screen { html: html.into(), css: css.into(), js: js.into() };
            assert!(document(Surface::Phone, &s).is_err(), "{html} {css} {js}");
        }
        let big = Screen { html: "a".repeat(MAX_SCREEN_BYTES), css: "b".into(), js: String::new() };
        assert!(document(Surface::Phone, &big).unwrap_err().contains("120 KB"));
    }

    #[test]
    fn the_additive_rule_guards_collections_with_rows_once_guests_are_in() {
        let live = json!({ "collections": {
            "shelf": { "read": "everyone", "write": "guest_many", "fields": { "title": { "type": "text" }, "book": { "type": "row", "of": "shelf" }, "genre": { "type": "choice", "of": ["novel", "poetry"] } } },
            "empty": { "read": "mine", "write": "guest_one", "fields": { "t": { "type": "text" } } }
        } });
        let state = json!({ "rows": { "shelf": [{ "id": "r_1", "owner": "a", "values": { "title": "Emma" } }] } });
        let ok = json!({ "collections": {
            "shelf": { "read": "everyone", "write": "guest_one", "fields": { "title": { "type": "text", "max": 9 }, "book": { "type": "row", "of": "shelf" }, "genre": { "type": "choice", "of": ["poetry", "novel"] }, "new": { "type": "int" } } },
            "more": { "read": "mine", "write": "guest_one", "fields": { "t": { "type": "text" } } }
        } });
        assert_eq!(additive(&live, &ok, &state, true), Ok(()), "adding, and dropping an empty collection, is fine");
        let mut gone = ok.clone();
        gone["collections"].as_object_mut().unwrap().remove("shelf");
        let mut retyped = ok.clone();
        retyped["collections"]["shelf"]["fields"]["title"]["type"] = json!("int");
        let mut repointed = ok.clone();
        repointed["collections"]["shelf"]["fields"]["book"]["of"] = json!("more");
        let mut no_field = ok.clone();
        no_field["collections"]["shelf"]["fields"].as_object_mut().unwrap().remove("title");
        let mut fewer = ok.clone();
        fewer["collections"]["shelf"]["fields"]["genre"] = json!({ "type": "choice", "of": ["novel"] });
        let mut more = ok.clone();
        more["collections"]["shelf"]["fields"]["genre"] = json!({ "type": "choice", "of": ["novel", "poetry", "essay"] });
        assert_eq!(additive(&live, &more, &state, true), Ok(()), "a choice may gain options");
        let mut wider = ok.clone();
        wider["collections"]["shelf"]["read"] = json!("host");
        for next in [&gone, &retyped, &repointed, &no_field, &fewer, &wider] {
            let m = additive(&live, next, &state, true).unwrap_err();
            assert!(m.starts_with("\"shelf\" has rows") && m.ends_with("start a new pocket for that."), "{m}");
            assert_eq!(additive(&live, next, &state, false), Ok(()), "before any guest anything goes");
        }
    }
}
