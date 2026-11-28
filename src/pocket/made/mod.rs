//! The `made` kit (pa12 "Make your own", L2): a pocket a bot made from a `made/1` spec
//! ([`spec`]). Plan and security review: autonomy/pocket-apps/pa12/plan.md section 1 and
//! security-review.md. It runs only while `pocket_make` is on as well as `pocket_apps`
//! (`super::Switch`, pa12 L1).
//!
//! L2 is the data part: collections of rows with one read rule and one write rule each
//! ([`collections`]), the guest and host action `put`, and the guest action `take_back`. The
//! other server actions (lock, answer, vote, draw, timer, turns, reveal, step, adjust) come in
//! L4; the app document and its versions in L5.
//!
//! Every viewer gets only its own view, built on the box from the read rules. `secrets` is in the
//! spec only: no view, lobby or summary reads it, and the kit state never copies it.

pub mod collections;
pub mod spec;

use super::kits::{spec_date, Kit, KitCtx, KitError, KitRules, Player, Retention, Viewer, DEFAULT_MAX_PLAYERS, MADE_ID};
use super::words;
use serde_json::{json, Map, Value};

/// `after_date` keeps the data until 7 days after `reveal_at`.
const DATE_FIELDS: &[&str] = &["reveal_at"];
const RETENTION_DAYS: i64 = 7;

pub struct Made;

/// The spec's `words`, each cleaned as the validator checked it; one that no longer passes is left out.
fn clean_words(spec: &Value) -> Value {
    let mut out = Map::new();
    if let Some(w) = spec["words"].as_object() {
        for (k, v) in w {
            if let Some(t) = v.as_str().and_then(|s| words::clean_text(s, spec::WORD_MAX).ok()) {
                out.insert(k.clone(), json!(t));
            }
        }
    }
    Value::Object(out)
}

impl Kit for Made {
    fn id(&self) -> &'static str {
        MADE_ID
    }

    fn rules(&self, spec: &Value) -> KitRules {
        let retention = match spec["retention"].as_str() {
            Some("at_end") => Retention::AtEnd,
            Some("after_date") => Retention::AfterDate { fields: DATE_FIELDS, days: RETENTION_DAYS },
            _ => Retention::AfterEnd { days: RETENTION_DAYS },
        };
        let max_players = spec["max_players"].as_u64().map_or(DEFAULT_MAX_PLAYERS, |n| (n as usize).clamp(1, DEFAULT_MAX_PLAYERS));
        KitRules { retention, needs_pin: false, max_players, guest_actions: &["put", "take_back"], host_actions: &["put"] }
    }

    fn validate_spec(&self, spec: &Value) -> Result<(), String> {
        spec::validate(spec)
    }

    fn init(&self, _spec: &Value) -> Value {
        json!({ "rows": {}, "locked": {}, "revealed": false })
    }

    fn on_remove(&self, _ctx: &mut KitCtx, state: &mut Value, player_id: &str) {
        collections::forget(state, player_id);
    }

    fn act(&self, ctx: &mut KitCtx, state: &mut Value, actor: &Viewer, name: &str, body: &Value) -> Result<(), KitError> {
        match name {
            "put" => collections::put(ctx, state, actor, body),
            "take_back" => collections::take_back(ctx, state, actor, body),
            _ => Err(KitError::BadInput(format!("unknown action {name}"))),
        }
    }

    /// Flips `revealed` once `reveal_at` passes, so open sockets get the revealed rows.
    fn tick(&self, ctx: &mut KitCtx, state: &mut Value) -> bool {
        let due = ctx.spec.get("reveal_at").and_then(spec_date).is_some_and(|t| t <= ctx.now);
        if due && !state["revealed"].as_bool().unwrap_or(false) {
            state["revealed"] = json!(true);
            return true;
        }
        false
    }

    fn view(&self, ctx: &KitCtx, state: &Value, viewer: &Viewer) -> Value {
        let players: Vec<Value> = ctx.players.iter().map(|p: &Player| json!({ "id": p.id, "name": p.name, "avatar": p.avatar })).collect();
        json!({
            "title": self.title(ctx.spec),
            "words": clean_words(ctx.spec),
            "collections": collections::view(ctx.spec, state, viewer, ctx.now),
            // L4's `reveal` copies named secret items here; until then nothing is revealed
            "revealed": {},
            "step": null,
            "timer": null,
            "turn": null,
            "scores": null,
            "players": players,
        })
    }

    /// Before joining: the title, the words and how many rows each `everyone` collection has.
    fn lobby(&self, ctx: &KitCtx, state: &Value) -> Value {
        json!({
            "title": self.title(ctx.spec),
            "words": clean_words(ctx.spec),
            "counts": collections::counts(ctx.spec, state, |c| c.read == "everyone"),
        })
    }

    /// The person's card: how many rows each collection has, nothing else.
    fn host_summary(&self, ctx: &KitCtx, state: &Value) -> Value {
        json!({ "counts": collections::counts(ctx.spec, state, |_| true) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn spec() -> Value {
        json!({
            "format": "made/1",
            "title": "Book swap",
            "reveal_at": "2030-01-01T00:00:00Z",
            "collections": {
                "shelf": { "read": "everyone", "write": "guest_many", "max_per_guest": 2,
                           "fields": { "title": {"type":"text","max":20} } },
                "guesses": { "read": "mine", "write": "guest_one", "fields": { "text": {"type":"text","max":50} } },
                "notes": { "read": "host", "write": "guest_one", "fields": { "line": {"type":"text","max":50} } },
                "picks": { "read": "after_reveal", "write": "guest_one", "fields": { "pick": {"type":"row","of":"shelf"} } },
                "board": { "read": "everyone", "write": "host", "fields": { "n": {"type":"int","min":0,"max":9}, "on": {"type":"bool"}, "at": {"type":"time"} } },
                "tally": { "read": "everyone", "write": "server", "fields": { "n": {"type":"int","min":0,"max":9} } }
            },
            "secrets": { "answers": ["moby-dick-secret"] }
        })
    }

    fn players() -> Vec<Player> {
        ["a", "b"].iter().map(|id| Player { id: id.to_string(), name: id.to_uppercase(), avatar: "cat".into(), team: None, joined_at: Utc::now() }).collect()
    }

    fn put(ctx: &mut KitCtx, st: &mut Value, who: &Viewer, coll: &str, values: Value) -> Result<(), KitError> {
        Made.act(ctx, st, who, "put", &json!({ "collection": coll, "values": values }))
    }

    #[test]
    fn put_follows_each_write_rule_and_field_type() {
        let (spec, ps) = (spec(), players());
        assert_eq!(Made.validate_spec(&spec), Ok(()));
        let mut ctx = KitCtx::new(&spec, &ps, Utc::now());
        let mut st = Made.init(&spec);
        let (a, b, host) = (Viewer::Guest("a".into()), Viewer::Guest("b".into()), Viewer::Host);
        // guest_one overwrites, keeping the row id
        put(&mut ctx, &mut st, &a, "guesses", json!({ "text": "first" })).unwrap();
        let id = st["rows"]["guesses"][0]["id"].clone();
        put(&mut ctx, &mut st, &a, "guesses", json!({ "text": "  second   try " })).unwrap();
        assert_eq!(st["rows"]["guesses"].as_array().unwrap().len(), 1);
        assert_eq!((st["rows"]["guesses"][0]["id"].clone(), st["rows"]["guesses"][0]["values"]["text"].clone()), (id, json!("second try")));
        // guest_many caps at max_per_guest, per guest
        put(&mut ctx, &mut st, &a, "shelf", json!({ "title": "Emma" })).unwrap();
        put(&mut ctx, &mut st, &a, "shelf", json!({ "title": "Ulysses" })).unwrap();
        assert!(matches!(put(&mut ctx, &mut st, &a, "shelf", json!({ "title": "Third" })), Err(KitError::NotNow(m)) if m.contains("at most 2")));
        put(&mut ctx, &mut st, &b, "shelf", json!({ "title": "Beloved" })).unwrap();
        // host is the host's only; server refuses everyone
        assert!(matches!(put(&mut ctx, &mut st, &a, "board", json!({ "n": 1, "on": true, "at": "2026-12-18" })), Err(KitError::NotNow(_))));
        put(&mut ctx, &mut st, &host, "board", json!({ "n": 1, "on": true, "at": "2026-12-18" })).unwrap();
        assert_eq!(st["rows"]["board"][0]["values"]["at"], "2026-12-18T23:59:59Z");
        assert_eq!(st["rows"]["board"][0]["owner"], collections::HOST_OWNER);
        for who in [&a, &host] {
            assert!(matches!(put(&mut ctx, &mut st, who, "tally", json!({ "n": 1 })), Err(KitError::NotNow(m)) if m.contains("only the app itself")));
        }
        // field types
        for (coll, values, want) in [
            ("board", json!({ "n": 10, "on": true, "at": "2026-12-18" }), "n: send a whole number from 0 to 9."),
            ("board", json!({ "n": 1.5, "on": true, "at": "2026-12-18" }), "n: send a whole number from 0 to 9."),
            ("board", json!({ "n": 1, "on": "yes", "at": "2026-12-18" }), "on: send true or false."),
            ("board", json!({ "n": 1, "on": true, "at": "soon" }), "at: send a date"),
            ("board", json!({ "n": 1, "on": true }), "at: fill this in."),
            ("board", json!({ "n": 1, "on": true, "at": "2026-12-18", "x": 1 }), "\"board\" has no field called \"x\"."),
            ("nope", json!({}), "There's no collection called \"nope\"."),
        ] {
            match put(&mut ctx, &mut st, &host, coll, values) {
                Err(KitError::BadInput(m)) => assert!(m.starts_with(want), "want {want:?}, got {m:?}"),
                other => panic!("{coll}: {other:?}"),
            }
        }
        assert!(matches!(put(&mut ctx, &mut st, &a, "shelf", json!({ "title": "x".repeat(21) })), Err(KitError::BadInput(m)) if m.contains("at most 20")));
        // a row field: an existing visible row only
        let shelf_id = st["rows"]["shelf"][2]["id"].as_str().unwrap().to_string();
        put(&mut ctx, &mut st, &a, "picks", json!({ "pick": shelf_id })).unwrap();
        assert!(matches!(put(&mut ctx, &mut st, &a, "picks", json!({ "pick": "r_0000000000000000" })), Err(KitError::BadInput(m)) if m.contains("no such row")));
    }

    #[test]
    fn take_back_deletes_only_the_guests_own_row_and_says_the_same_otherwise() {
        let (spec, ps) = (spec(), players());
        let mut ctx = KitCtx::new(&spec, &ps, Utc::now());
        let mut st = Made.init(&spec);
        let (a, b) = (Viewer::Guest("a".into()), Viewer::Guest("b".into()));
        put(&mut ctx, &mut st, &a, "guesses", json!({ "text": "mine" })).unwrap();
        let id = st["rows"]["guesses"][0]["id"].as_str().unwrap().to_string();
        fn back(ctx: &mut KitCtx, st: &mut Value, who: &Viewer, id: &str) -> Result<(), KitError> {
            Made.act(ctx, st, who, "take_back", &json!({ "collection": "guesses", "id": id }))
        }
        let theirs = back(&mut ctx, &mut st, &b, &id).unwrap_err();
        let missing = back(&mut ctx, &mut st, &b, "r_nope").unwrap_err();
        assert_eq!(theirs, missing, "another guest's row answers like a missing one");
        back(&mut ctx, &mut st, &a, &id).unwrap();
        assert_eq!(st["rows"]["guesses"], json!([]));
    }

    #[test]
    fn the_reveal_flips_at_reveal_at_and_removed_guests_rows_go() {
        let (spec, ps) = (spec(), players());
        let before = chrono::DateTime::parse_from_rfc3339("2029-12-31T23:59:00Z").unwrap().with_timezone(&Utc);
        let mut ctx = KitCtx::new(&spec, &ps, before);
        let mut st = Made.init(&spec);
        assert!(!Made.tick(&mut ctx, &mut st));
        let mut later = KitCtx::new(&spec, &ps, before + Duration::minutes(2));
        assert!(Made.tick(&mut later, &mut st) && !Made.tick(&mut later, &mut st), "once");
        assert_eq!(st["revealed"], true);
        put(&mut later, &mut st, &Viewer::Guest("a".into()), "guesses", json!({ "text": "bye" })).unwrap();
        Made.on_remove(&mut later, &mut st, "a");
        assert_eq!(st["rows"]["guesses"], json!([]));
    }

    #[test]
    fn a_hand_made_row_with_odd_state_never_panics_and_shows_nothing_private() {
        let ps = players();
        for spec in [json!({}), json!({ "collections": 3 }), spec()] {
            for st in [json!({}), json!(null), json!({ "rows": [] }), json!({ "rows": { "guesses": "x" } })] {
                let ctx = KitCtx::new(&spec, &ps, Utc::now());
                for v in [Viewer::Guest("a".into()), Viewer::Host, Viewer::BigScreen] {
                    let out = Made.view(&ctx, &st, &v).to_string();
                    assert!(!out.contains("moby-dick-secret"), "{out}");
                }
                assert!(!Made.lobby(&ctx, &st).to_string().contains("moby"));
                assert!(!Made.host_summary(&ctx, &st).to_string().contains("moby"));
            }
        }
    }
}
