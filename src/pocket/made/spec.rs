//! The `made/1` spec (pa12 plan section 1): what a bot writes for a pocket it makes. Every error
//! is a plain sentence the bot can act on, naming the collection and field it is about.
//!
//! L2 checks the parts it uses (title, retention, the reveal date, players, words, secrets and
//! the collections). `actions`, `steps` and `screens` are only type-checked here: their contents
//! belong to L4 and V2, and no view in L2 reads them.

use super::super::kits::spec_date;
use chrono::{Duration, Utc};
use super::super::words;
use serde_json::{Map, Value};

pub const FORMAT: &str = "made/1";
/// The biggest `made/1` spec (plan section 5), well under the core's 256 KB.
pub const MAX_SPEC_BYTES: usize = 64 * 1024;
pub const TITLE_MAX: usize = 80;
/// `reveal_at` is at most this many days ahead: with `after_date` the data stays until a week after it.
pub const REVEAL_MAX_DAYS: i64 = 366;
pub const MAX_PLAYERS: u64 = 60;
pub const MAX_COLLECTIONS: usize = 12;
pub const MAX_FIELDS: usize = 12;
pub const MAX_PER_GUEST: u64 = 50;
pub const TEXT_MAX: u64 = 500;
pub const MAX_CHOICES: usize = 20;
pub const CHOICE_MAX: usize = 80;
pub const MAX_WORDS: usize = 20;
/// A `words` entry is a short line on a screen (a button, a thank-you).
pub const WORD_MAX: usize = 80;

pub const RETENTIONS: &[&str] = &["after_end", "at_end", "after_date"];
pub const READS: &[&str] = &["mine", "host", "everyone", "after_reveal"];
pub const WRITES: &[&str] = &["guest_one", "guest_many", "host", "server"];
pub const TYPES: &[&str] = &["text", "int", "choice", "bool", "time", "row"];

const TOP_KEYS: &[&str] = &["format", "title", "retention", "reveal_at", "max_players", "collections", "secrets", "actions", "steps", "screens", "words"];
const COLLECTION_KEYS: &[&str] = &["read", "write", "max_per_guest", "fields"];

/// A collection or field name: a lower-case letter, then up to 23 lower-case letters, digits or `_`.
pub fn valid_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty() && b.len() <= 24 && b[0].is_ascii_lowercase() && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

fn list(items: &[&str]) -> String {
    items.join(", ")
}

/// Checks a `made/1` spec. `Err` is one plain sentence.
pub fn validate(spec: &Value) -> Result<(), String> {
    let o = spec.as_object().ok_or("The spec must be a JSON object.")?;
    if o.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(format!("The spec needs \"format\": \"{FORMAT}\"."));
    }
    if let Some(k) = o.keys().find(|k| !TOP_KEYS.contains(&k.as_str())) {
        return Err(format!("The spec has a key {:?} that {FORMAT} doesn't know. Known keys: {}.", short(k), list(TOP_KEYS)));
    }
    if serde_json::to_string(spec).map_or(usize::MAX, |s| s.len()) > MAX_SPEC_BYTES {
        return Err("The spec is bigger than 64 KB.".into());
    }
    // The title is shown to guests (info, the join page, the view), so it is cleaned like any
    // text guests read: invisible and direction characters and links come out (QA N2)
    let title = o.get("title").and_then(Value::as_str).unwrap_or("");
    match words::clean_text(title, TITLE_MAX) {
        Ok(_) => {}
        Err(words::TextError::Empty) => return Err("The spec needs a title.".into()),
        Err(words::TextError::TooLong(_)) => return Err(format!("The title is at most {TITLE_MAX} characters.")),
        Err(words::TextError::NotAllowed) => return Err("The title needs other words.".into()),
    }
    let reveal_at = match o.get("reveal_at") {
        None | Some(Value::Null) => None,
        Some(v) => Some(spec_date(v).ok_or("reveal_at must be a date (2026-12-18) or a time (2026-12-18T19:00:00Z).")?),
    };
    // QA N1: a far date would keep the data for ever (and once overflowed the expiry)
    if reveal_at.is_some_and(|t| t > Utc::now() + Duration::days(REVEAL_MAX_DAYS)) {
        return Err("reveal_at is at most a year from now.".into());
    }
    match o.get("retention") {
        None | Some(Value::Null) => {}
        Some(v) => match v.as_str() {
            Some("after_date") if reveal_at.is_none() => return Err("retention \"after_date\" needs a reveal_at date.".into()),
            Some(r) if RETENTIONS.contains(&r) => {}
            _ => return Err(format!("retention must be one of {}.", list(RETENTIONS))),
        },
    }
    match o.get("max_players") {
        None | Some(Value::Null) => {}
        Some(v) => {
            if !v.as_u64().is_some_and(|n| (1..=MAX_PLAYERS).contains(&n)) {
                return Err(format!("max_players must be a whole number from 1 to {MAX_PLAYERS}."));
            }
        }
    }
    if let Some(w) = o.get("words").filter(|v| !v.is_null()) {
        check_words(w)?;
    }
    if let Some(s) = o.get("secrets").filter(|v| !v.is_null()) {
        if !s.is_object() {
            return Err("secrets must be an object.".into());
        }
    }
    for (key, ok, what) in [("actions", Value::is_object as fn(&Value) -> bool, "an object"), ("steps", Value::is_array, "a list"), ("screens", Value::is_object, "an object")] {
        if let Some(v) = o.get(key).filter(|v| !v.is_null()) {
            if !ok(v) {
                return Err(format!("{key} must be {what}."));
            }
        }
    }
    check_collections(o.get("collections"))
}

/// A name from the spec, cut short for an error sentence.
fn short(s: &str) -> String {
    s.chars().take(40).collect()
}

fn check_words(w: &Value) -> Result<(), String> {
    let w = w.as_object().ok_or("words must be an object of short texts.")?;
    if w.len() > MAX_WORDS {
        return Err(format!("words has at most {MAX_WORDS} entries."));
    }
    for (k, v) in w {
        if !valid_name(k) {
            return Err(format!("Word {:?}: a name is lower-case letters, digits and _, starting with a letter (at most 24).", short(k)));
        }
        let text = v.as_str().ok_or_else(|| format!("Word {k:?} must be a text."))?;
        words::clean_text(text, WORD_MAX).map_err(|e| format!("Word {k:?}: {}", e.message()))?;
    }
    Ok(())
}

fn check_collections(c: Option<&Value>) -> Result<(), String> {
    let c = c.and_then(Value::as_object).ok_or("The spec needs collections: an object of 1 to 12 named collections.")?;
    if c.is_empty() || c.len() > MAX_COLLECTIONS {
        return Err(format!("The spec has {} collections; it needs 1 to {MAX_COLLECTIONS}.", c.len()));
    }
    for (name, coll) in c {
        if !valid_name(name) {
            return Err(format!(
                "Collection {:?}: a name is lower-case letters, digits and _, starting with a letter (at most 24).",
                short(name)
            ));
        }
        check_collection(name, coll, c)?;
    }
    Ok(())
}

fn check_collection(name: &str, coll: &Value, all: &Map<String, Value>) -> Result<(), String> {
    let o = coll.as_object().ok_or_else(|| format!("Collection {name:?} must be an object with read, write and fields."))?;
    if let Some(k) = o.keys().find(|k| !COLLECTION_KEYS.contains(&k.as_str())) {
        return Err(format!("Collection {name:?} has a key {:?} it doesn't know. Known keys: {}.", short(k), list(COLLECTION_KEYS)));
    }
    let read = o.get("read").and_then(Value::as_str).unwrap_or("");
    if !READS.contains(&read) {
        return Err(format!("Collection {name:?}: read must be one of {}.", list(READS)));
    }
    let write = o.get("write").and_then(Value::as_str).unwrap_or("");
    if !WRITES.contains(&write) {
        return Err(format!("Collection {name:?}: write must be one of {}.", list(WRITES)));
    }
    if let Some(m) = o.get("max_per_guest") {
        if write != "guest_many" {
            return Err(format!("Collection {name:?}: max_per_guest goes only with write \"guest_many\"."));
        }
        if !m.as_u64().is_some_and(|n| (1..=MAX_PER_GUEST).contains(&n)) {
            return Err(format!("Collection {name:?}: max_per_guest must be a whole number from 1 to {MAX_PER_GUEST}."));
        }
    }
    let fields = o.get("fields").and_then(Value::as_object).ok_or_else(|| format!("Collection {name:?} needs fields: an object of 1 to {MAX_FIELDS} named fields."))?;
    if fields.is_empty() || fields.len() > MAX_FIELDS {
        return Err(format!("Collection {name:?} has {} fields; it needs 1 to {MAX_FIELDS}.", fields.len()));
    }
    for (f, def) in fields {
        if !valid_name(f) {
            return Err(format!(
                "Collection {name:?}: field {:?}: a name is lower-case letters, digits and _, starting with a letter (at most 24).",
                short(f)
            ));
        }
        check_field(name, def, all).map_err(|m| format!("Collection {name:?}: field {f:?} {m}"))?;
    }
    Ok(())
}

/// The end of a sentence that starts `Collection "x": field "y" `.
fn check_field(coll: &str, def: &Value, all: &Map<String, Value>) -> Result<(), String> {
    let d = def.as_object().ok_or("must be an object with a type.")?;
    let ty = d.get("type").and_then(Value::as_str).unwrap_or("");
    let keys: &[&str] = match ty {
        "text" => &["type", "max"],
        "int" => &["type", "min", "max"],
        "choice" | "row" => &["type", "of"],
        "bool" | "time" => &["type"],
        _ => return Err(format!("needs a type: one of {}.", list(TYPES))),
    };
    if let Some(k) = d.keys().find(|k| !keys.contains(&k.as_str())) {
        return Err(format!("is a {ty} and doesn't take {:?}.", short(k)));
    }
    match ty {
        "text" => {
            if !d.get("max").and_then(Value::as_u64).is_some_and(|n| (1..=TEXT_MAX).contains(&n)) {
                return Err(format!("is a text and needs max: a whole number from 1 to {TEXT_MAX}."));
            }
        }
        "int" => {
            let (min, max) = (d.get("min").and_then(Value::as_i64), d.get("max").and_then(Value::as_i64));
            match (min, max) {
                (Some(a), Some(b)) if a <= b => {}
                (Some(_), Some(_)) => return Err("is an int whose min is above its max.".into()),
                _ => return Err("is an int and needs min and max (whole numbers).".into()),
            }
        }
        "choice" => {
            let of = d.get("of").and_then(Value::as_array).filter(|a| !a.is_empty()).ok_or("is a choice with no options.")?;
            if of.len() > MAX_CHOICES {
                return Err(format!("is a choice with {} options; it may have at most {MAX_CHOICES}.", of.len()));
            }
            let mut seen = Vec::new();
            for o in of {
                let s = o.as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or("is a choice whose options must be non-empty texts.")?;
                if s.chars().count() > CHOICE_MAX {
                    return Err(format!("is a choice with an option over {CHOICE_MAX} characters."));
                }
                if seen.contains(&s) {
                    return Err(format!("is a choice that lists {:?} twice.", short(s)));
                }
                seen.push(s);
            }
        }
        "row" => {
            let of = d.get("of").and_then(Value::as_str).ok_or("is a row and needs of: the name of another collection.")?;
            if of == coll {
                return Err("is a row and must point at another collection, not its own.".into());
            }
            if !all.contains_key(of) {
                return Err(format!("is a row of {:?}, but there's no collection called that.", short(of)));
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn good() -> Value {
        json!({
            "format": "made/1",
            "title": "Book swap",
            "collections": {
                "shelf": { "read": "everyone", "write": "guest_many", "max_per_guest": 5,
                           "fields": { "title": {"type":"text","max":80}, "genre": {"type":"choice","of":["novel","poetry"]} } },
                "picks": { "read": "mine", "write": "guest_one", "fields": { "pick": {"type":"row","of":"shelf"} } }
            },
            "words": { "join": "Join the swap" },
            "secrets": { "answers": [] },
            "actions": {}, "steps": [], "screens": {}
        })
    }

    fn with(path: &[&str], v: Value) -> Value {
        let mut s = good();
        let mut at = &mut s;
        for p in &path[..path.len() - 1] {
            at = &mut at[*p];
        }
        at[path[path.len() - 1]] = v;
        s
    }

    #[test]
    fn a_good_spec_passes_and_each_mistake_is_one_plain_sentence() {
        assert_eq!(validate(&good()), Ok(()));
        let cases = [
            (with(&["format"], json!("made/2")), "The spec needs \"format\": \"made/1\"."),
            (with(&["title"], json!("  ")), "The spec needs a title."),
            (with(&["when"], json!("2026-12-01")), "The spec has a key \"when\" that made/1 doesn't know."),
            (with(&["retention"], json!("after_date")), "retention \"after_date\" needs a reveal_at date."),
            (with(&["retention"], json!("forever")), "retention must be one of after_end, at_end, after_date."),
            (with(&["reveal_at"], json!("next tuesday")), "reveal_at must be a date"),
            (with(&["max_players"], json!(61)), "max_players must be a whole number from 1 to 60."),
            (with(&["secrets"], json!([])), "secrets must be an object."),
            (with(&["steps"], json!({})), "steps must be a list."),
            (with(&["collections"], json!({})), "The spec has 0 collections; it needs 1 to 12."),
            (with(&["collections", "Shelf"], json!({})), "Collection \"Shelf\": a name is lower-case"),
            (with(&["collections", "shelf", "read"], json!("all")), "Collection \"shelf\": read must be one of mine, host, everyone, after_reveal."),
            (with(&["collections", "shelf", "write"], json!("anyone")), "Collection \"shelf\": write must be one of"),
            (with(&["collections", "picks", "max_per_guest"], json!(3)), "Collection \"picks\": max_per_guest goes only with write \"guest_many\"."),
            (with(&["collections", "shelf", "max_per_guest"], json!(51)), "Collection \"shelf\": max_per_guest must be a whole number from 1 to 50."),
            (with(&["collections", "shelf", "fields", "genre", "of"], json!([])), "Collection \"shelf\": field \"genre\" is a choice with no options."),
            (with(&["collections", "shelf", "fields", "genre", "of"], json!(["a", "a"])), "Collection \"shelf\": field \"genre\" is a choice that lists \"a\" twice."),
            (with(&["collections", "shelf", "fields", "title", "max"], json!(501)), "Collection \"shelf\": field \"title\" is a text and needs max"),
            (with(&["collections", "shelf", "fields", "n"], json!({"type":"int","min":5,"max":1})), "Collection \"shelf\": field \"n\" is an int whose min is above its max."),
            (with(&["collections", "shelf", "fields", "n"], json!({"type":"photo"})), "Collection \"shelf\": field \"n\" needs a type: one of text, int, choice, bool, time, row."),
            (with(&["collections", "picks", "fields", "pick", "of"], json!("books")), "Collection \"picks\": field \"pick\" is a row of \"books\", but there's no collection called that."),
            (with(&["collections", "picks", "fields", "pick", "of"], json!("picks")), "must point at another collection, not its own."),
            (with(&["collections", "shelf", "fields", "title", "label"], json!("Title")), "Collection \"shelf\": field \"title\" is a text and doesn't take \"label\"."),
            (with(&["words", "join"], json!("a".repeat(81))), "Word \"join\": That's too long: at most 80 characters."),
            (with(&["reveal_at"], json!("+262142-12-31")), "reveal_at is at most a year from now."),
            (with(&["reveal_at"], json!((Utc::now() + Duration::days(REVEAL_MAX_DAYS + 2)).to_rfc3339())), "reveal_at is at most a year from now."),
            (with(&["title"], json!("https://example.com \u{200B}")), "The spec needs a title."),
            (with(&["title"], json!("x".repeat(81))), "The title is at most 80 characters."),
        ];
        for (spec, want) in cases {
            let got = validate(&spec).expect_err(want);
            assert!(got.starts_with(want) || got.contains(want), "want {want:?}, got {got:?}");
            assert!(got.ends_with('.') && !got.contains('\n'), "one plain sentence: {got:?}");
        }
        // after_date with a reveal date passes; 13 collections don't
        assert_eq!(validate(&with(&["reveal_at"], json!("2026-12-18"))), Ok(()));
        let soon = (Utc::now() + Duration::days(REVEAL_MAX_DAYS - 1)).to_rfc3339();
        assert_eq!(validate(&with(&["reveal_at"], json!(soon))), Ok(()));
        let mut s = with(&["retention"], json!("after_date"));
        s["reveal_at"] = json!("2026-12-18T19:00:00Z");
        assert_eq!(validate(&s), Ok(()));
        let mut many = good();
        for i in 0..12 {
            many["collections"][format!("c{i}")] = json!({ "read": "everyone", "write": "host", "fields": { "x": {"type":"bool"} } });
        }
        assert_eq!(validate(&many), Err("The spec has 14 collections; it needs 1 to 12.".into()));
        let mut big = good();
        big["secrets"]["pad"] = json!("x".repeat(MAX_SPEC_BYTES));
        assert_eq!(validate(&big), Err("The spec is bigger than 64 KB.".into()));
    }
}
