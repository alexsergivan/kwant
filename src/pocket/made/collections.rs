//! A made pocket's collections (pa12 plan section 1): rows `{id, owner, values, at}` in the kit
//! state under `rows.<collection>`, the write rules `put` and `take_back` check, and the read
//! rules every view goes through.
//!
//! The read rules are the privacy boundary of a bot-made app. A guest never gets another guest's
//! `mine` or `host` rows, the big screen never gets a `host` collection at all, and nothing here
//! ever reads `secrets`. A row's owner is a player id (or [`HOST_OWNER`]); names come from the
//! player list, never from a row.

use super::super::kits::{spec_date, KitCtx, KitError, Viewer};
use super::super::words;
use super::spec::MAX_PER_GUEST;
use chrono::{DateTime, SecondsFormat, Utc};
use ring::rand::SecureRandom;
use serde_json::{json, Map, Value};

/// The owner of a row the host put.
pub const HOST_OWNER: &str = "host";

/// One collection of the spec, as the rules read it.
pub struct Coll<'a> {
    pub name: &'a str,
    pub read: &'a str,
    pub write: &'a str,
    pub max_per_guest: usize,
    pub fields: Option<&'a Map<String, Value>>,
}

impl<'a> Coll<'a> {
    fn from(name: &'a str, def: &'a Value) -> Self {
        Coll {
            name,
            read: def["read"].as_str().unwrap_or(""),
            write: def["write"].as_str().unwrap_or(""),
            max_per_guest: def["max_per_guest"].as_u64().unwrap_or(MAX_PER_GUEST).min(MAX_PER_GUEST) as usize,
            fields: def["fields"].as_object(),
        }
    }
}

/// The spec's collections in name order. A spec the validator passed has 1 to 12; anything else
/// (a row put in by hand) gives what it can and never panics.
pub fn all(spec: &Value) -> Vec<Coll<'_>> {
    spec["collections"].as_object().map(|m| m.iter().map(|(n, d)| Coll::from(n, d)).collect()).unwrap_or_default()
}

pub fn find<'a>(spec: &'a Value, name: &str) -> Option<Coll<'a>> {
    spec["collections"].as_object()?.get_key_value(name).map(|(n, d)| Coll::from(n, d))
}

/// The rows of one collection in the state (oldest first).
pub fn rows<'s>(state: &'s Value, name: &str) -> &'s [Value] {
    state["rows"][name].as_array().map_or(&[], Vec::as_slice)
}

fn owner(row: &Value) -> &str {
    row["owner"].as_str().unwrap_or("")
}

pub fn locked(state: &Value, name: &str) -> bool {
    state["locked"][name].as_bool().unwrap_or(false)
}

/// Revealed: the `revealed` flag is set (the host's `reveal`, or `tick` once `reveal_at` passed), or
/// `reveal_at` has passed already (a poll may come before the next tick).
pub fn revealed(spec: &Value, state: &Value, now: DateTime<Utc>) -> bool {
    state["revealed"].as_bool().unwrap_or(false) || spec.get("reveal_at").and_then(spec_date).is_some_and(|t| t <= now)
}

/// What one viewer may see of one collection (plan section 1's table).
#[derive(Debug, PartialEq)]
pub enum Part<'s> {
    /// These rows, and `count` (all rows when the rule shows the total, else `rows.len()`).
    Rows { rows: Vec<&'s Value>, count: usize },
    /// The number of rows and nothing else.
    Count(usize),
    /// Not even that: the collection is left out of the view.
    Hidden,
}

/// The read rule. `mine`: the owner gets their rows, other guests nothing, the screen and the
/// host a count. `host`: the owner gets their rows, the host all, the screen nothing.
/// `everyone`: all rows to everyone. `after_reveal`: before the reveal like `mine`, except other
/// guests get the count; after it, all rows to everyone.
pub fn part<'s>(c: &Coll, state: &'s Value, viewer: &Viewer, revealed: bool) -> Part<'s> {
    let all = rows(state, c.name);
    let every = || Part::Rows { rows: all.iter().collect(), count: all.len() };
    let own = |id: &str, count: Option<usize>| {
        let rows: Vec<&Value> = all.iter().filter(|r| owner(r) == id).collect();
        let count = count.unwrap_or(rows.len());
        Part::Rows { rows, count }
    };
    match (c.read, viewer) {
        ("everyone", _) => every(),
        ("after_reveal", _) if revealed => every(),
        ("mine", Viewer::Guest(id)) => own(id.as_str(), None),
        ("mine", _) => Part::Count(all.len()),
        ("host", Viewer::Guest(id)) => own(id.as_str(), None),
        ("host", Viewer::Host) => every(),
        ("host", Viewer::BigScreen) => Part::Hidden,
        ("after_reveal", Viewer::Guest(id)) => own(id.as_str(), Some(all.len())),
        ("after_reveal", _) => Part::Count(all.len()),
        // A rule the validator wouldn't pass: show nothing
        _ => Part::Hidden,
    }
}

/// `{collection: part}` for one viewer: `{rows: [{id, owner, mine, values}], count, locked}` or
/// `{count, locked}`; a hidden collection isn't a key at all.
pub fn view(spec: &Value, state: &Value, viewer: &Viewer, now: DateTime<Utc>) -> Value {
    let revealed = revealed(spec, state, now);
    let me = match viewer {
        Viewer::Guest(id) => Some(id.as_str()),
        Viewer::Host => Some(HOST_OWNER),
        Viewer::BigScreen => None,
    };
    let mut out = Map::new();
    for c in all(spec) {
        let lock = super::actions::coll_locked(state, c.name, now);
        let v = match part(&c, state, viewer, revealed) {
            Part::Rows { rows, count } => {
                let rows: Vec<Value> = rows
                    .into_iter()
                    .map(|r| json!({ "id": r["id"], "owner": r["owner"], "mine": me.is_some_and(|m| owner(r) == m), "values": r["values"] }))
                    .collect();
                json!({ "rows": rows, "count": count, "locked": lock })
            }
            Part::Count(n) => json!({ "count": n, "locked": lock }),
            Part::Hidden => continue,
        };
        out.insert(c.name.to_string(), v);
    }
    Value::Object(out)
}

/// The number of rows in each collection the rule `keep` picks (the lobby: `everyone` only; the
/// host's summary: all).
pub fn counts(spec: &Value, state: &Value, keep: impl Fn(&Coll) -> bool) -> Value {
    let mut out = Map::new();
    for c in all(spec).iter().filter(|c| keep(c)) {
        out.insert(c.name.to_string(), json!(rows(state, c.name).len()));
    }
    Value::Object(out)
}

pub fn new_row_id() -> String {
    let mut b = [0u8; 8];
    ring::rand::SystemRandom::new().fill(&mut b).expect("the OS random generator works");
    format!("r_{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>())
}

fn collection<'a>(spec: &'a Value, body: &Value) -> Result<Coll<'a>, KitError> {
    let name = body.get("collection").and_then(Value::as_str).ok_or_else(|| KitError::BadInput("Say which collection (collection).".into()))?;
    find(spec, name).ok_or_else(|| KitError::BadInput(format!("There's no collection called {:?}.", name.chars().take(40).collect::<String>())))
}

/// `put {collection, values}`: one row under the collection's write rule, every value checked
/// against its field. `guest_one` overwrites the writer's one row (same id), `guest_many` adds
/// up to `max_per_guest`, `host` is the host's alone, and `server` refuses every `put`. The
/// host may also put into `guest_one`/`guest_many` collections, as [`HOST_OWNER`].
pub fn put(ctx: &KitCtx, state: &mut Value, actor: &Viewer, body: &Value) -> Result<(), KitError> {
    let c = collection(ctx.spec, body)?;
    let who = match actor {
        Viewer::Guest(id) => id.as_str(),
        Viewer::Host => HOST_OWNER,
        Viewer::BigScreen => return Err(KitError::BadInput("The big screen can't put anything.".into())),
    };
    match (c.write, actor) {
        ("server", _) => return Err(KitError::NotNow(format!("Nobody puts into {:?}: only the app itself writes it.", c.name))),
        ("host", Viewer::Guest(_)) => return Err(KitError::NotNow(format!("Only the host puts into {:?}.", c.name))),
        ("guest_one" | "guest_many" | "host", _) => {}
        _ => return Err(KitError::NotNow(format!("Nobody can put into {:?}.", c.name))),
    }
    // L4: `lock` (or a timer that ran out) closes a collection to guests; the host may still put
    if matches!(actor, Viewer::Guest(_)) && super::actions::coll_locked(state, c.name, ctx.now) {
        return Err(KitError::NotNow(format!("{:?} is closed right now.", c.name)));
    }
    let values = check_values(ctx, state, &c, actor, body.get("values"))?;
    let at = ctx.now.to_rfc3339_opts(SecondsFormat::Secs, true);
    if !state["rows"].is_object() {
        state["rows"] = json!({});
    }
    if !state["rows"][c.name].is_array() {
        state["rows"][c.name] = json!([]);
    }
    let list = state["rows"][c.name].as_array_mut().expect("just made a list");
    if c.write == "guest_one" {
        if let Some(row) = list.iter_mut().find(|r| owner(r) == who) {
            row["values"] = values;
            row["at"] = json!(at);
            return Ok(());
        }
    } else if c.write == "guest_many" {
        let mine = list.iter().filter(|r| owner(r) == who).count();
        if mine >= c.max_per_guest {
            return Err(KitError::NotNow(format!("You can add at most {} to {:?}. Take one back first.", c.max_per_guest, c.name)));
        }
    }
    list.push(json!({ "id": new_row_id(), "owner": who, "values": values, "at": at }));
    Ok(())
}

/// `take_back {collection, id}`: deletes one of the guest's own rows. A row that isn't theirs
/// gets the same answer as one that doesn't exist, so nobody can probe for other guests' rows.
pub fn take_back(ctx: &KitCtx, state: &mut Value, actor: &Viewer, body: &Value) -> Result<(), KitError> {
    let c = collection(ctx.spec, body)?;
    let Viewer::Guest(who) = actor else {
        return Err(KitError::BadInput("Only a guest takes back their own rows.".into()));
    };
    let id = body.get("id").and_then(Value::as_str).ok_or_else(|| KitError::BadInput("Say which row (id).".into()))?;
    if super::actions::coll_locked(state, c.name, ctx.now) {
        return Err(KitError::NotNow(format!("{:?} is closed right now.", c.name)));
    }
    let list = state["rows"][c.name].as_array_mut();
    let at = list.as_ref().and_then(|l| l.iter().position(|r| r["id"].as_str() == Some(id) && owner(r) == who));
    match (list, at) {
        (Some(list), Some(i)) => {
            list.remove(i);
            Ok(())
        }
        _ => Err(KitError::BadInput(format!("You have nothing with that id in {:?}.", c.name))),
    }
}

/// Removes every row a player owns (the host removed them).
pub fn forget(state: &mut Value, player_id: &str) {
    if let Some(m) = state["rows"].as_object_mut() {
        for list in m.values_mut().filter_map(Value::as_array_mut) {
            list.retain(|r| owner(r) != player_id);
        }
    }
}

/// The values of a `put`: exactly the collection's fields, each one checked; returns them
/// cleaned (text through [`words::clean_text`], a time as RFC 3339).
fn check_values(ctx: &KitCtx, state: &Value, c: &Coll, actor: &Viewer, values: Option<&Value>) -> Result<Value, KitError> {
    let bad = |m: String| KitError::BadInput(m);
    let values = values.and_then(Value::as_object).ok_or_else(|| bad("Send values: one for each field.".into()))?;
    let fields = c.fields.ok_or_else(|| bad(format!("{:?} has no fields.", c.name)))?;
    if let Some(k) = values.keys().find(|k| !fields.contains_key(k.as_str())) {
        return Err(bad(format!("{:?} has no field called {:?}.", c.name, k.chars().take(40).collect::<String>())));
    }
    let mut out = Map::new();
    for (f, def) in fields {
        let v = values.get(f).filter(|v| !v.is_null()).ok_or_else(|| bad(format!("{f}: fill this in.")))?;
        let clean = match def["type"].as_str().unwrap_or("") {
            "text" => {
                let max = def["max"].as_u64().unwrap_or(1) as usize;
                let s = v.as_str().ok_or_else(|| bad(format!("{f}: send a text.")))?;
                json!(words::clean_text(s, max).map_err(|e| bad(format!("{f}: {}", e.message())))?)
            }
            "int" => {
                let (min, max) = (def["min"].as_i64().unwrap_or(0), def["max"].as_i64().unwrap_or(0));
                match v.as_i64() {
                    Some(n) if (min..=max).contains(&n) => json!(n),
                    _ => return Err(bad(format!("{f}: send a whole number from {min} to {max}."))),
                }
            }
            "choice" => {
                let of = def["of"].as_array().map_or(&[][..], Vec::as_slice);
                match v.as_str().map(str::trim) {
                    Some(s) if of.iter().any(|o| o.as_str().map(str::trim) == Some(s)) => json!(s),
                    _ => return Err(bad(format!("{f}: pick one of the options."))),
                }
            }
            "bool" => json!(v.as_bool().ok_or_else(|| bad(format!("{f}: send true or false.")))?),
            "time" => {
                let t = spec_date(v).ok_or_else(|| bad(format!("{f}: send a date (2026-12-18) or a time (2026-12-18T19:00:00Z).")))?;
                json!(t.to_rfc3339_opts(SecondsFormat::Secs, true))
            }
            "row" => {
                let id = v.as_str().ok_or_else(|| bad(format!("{f}: send the id of a row.")))?;
                let of = def["of"].as_str().and_then(|n| find(ctx.spec, n)).ok_or_else(|| bad(format!("{f}: there's nothing to point at.")))?;
                // Only a row this writer can see counts, so a row field never tells anyone that
                // a row they may not see exists
                let seen = match part(&of, state, actor, revealed(ctx.spec, state, ctx.now)) {
                    Part::Rows { rows, .. } => rows.iter().any(|r| r["id"].as_str() == Some(id)),
                    _ => false,
                };
                if !seen {
                    return Err(bad(format!("{f}: there's no such row in {:?}.", of.name)));
                }
                json!(id)
            }
            _ => return Err(bad(format!("{f}: this field can't be filled in."))),
        };
        out.insert(f.clone(), clean);
    }
    Ok(Value::Object(out))
}
