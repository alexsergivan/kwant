//! pa4c-server: Inbox › Done shows each pocket whose results were posted (pa7
//! `pocket_schedule.results_posted_at`) for [`inbox_done::DONE_DAYS`] days, as one finished item
//! in the same list and the same unseen count as the other Done items. Names and scores only:
//! the title names the winning team, never a guest. A deleted or expired pocket drops out.
//! Nothing shows while the `pocket_apps` switch is off.
//!
//! Item: `kind: "pocket"`, `id: "pocket:<code>"`, title `"<title>: <winner> win"` (pa7's
//! "Nobody joined." for no players), `meta {players, ended_at}`, `winner {name, points}`,
//! `source {kind: "pocket", code, kwant_id, message_id, channel_id, thread_id}`, and the results
//! post's link and excerpt through [`inbox_done::apply`].

use super::away::rfc3339;
use super::inbox::{sort_key, Group};
use super::inbox_done::{self, DoneFacts};
use super::AppState;
use crate::db::spaces::SpaceId;
use crate::db::DbError;
use crate::pocket::{self, kits::KitCtx, schedule::Origin, Snapshot};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

/// A posted pocket in a space, with what the item needs.
struct Posted {
    snap: std::sync::Arc<Snapshot>,
    at: DateTime<Utc>,
    origin: Option<Origin>,
    kwant_id: Option<String>,
}

/// The space's posted pockets since `since`, newest first (none while pocket apps are off).
async fn posted(state: &AppState, space: &SpaceId, since: DateTime<Utc>) -> Result<Vec<Posted>, DbError> {
    if !pocket::enabled(&state.db).await {
        return Ok(Vec::new());
    }
    let now = Utc::now();
    let mut out = Vec::new();
    for (code, at) in state.db.pocket_results_posted().await? {
        if at < since {
            continue;
        }
        let Ok(Some(snap)) = pocket::snapshot(state, &code).await else { continue };
        if snap.row.expires_at.is_some_and(|e| e <= now) {
            continue;
        }
        let origin: Option<Origin> = state.db.pocket_schedule(&code).await?.and_then(|s| s.origin).and_then(|o| serde_json::from_str(&o).ok());
        let kwant_id = origin.as_ref().map(|o| o.kwant_id.clone()).or_else(|| snap.row.owner_kwant_id.clone());
        let of = match &kwant_id {
            Some(k) => state.db.space_of_kwant(k).await?.unwrap_or_else(SpaceId::default_space),
            None => SpaceId::default_space(),
        };
        if &of == space {
            out.push(Posted { snap, at, origin, kwant_id });
        }
    }
    Ok(out)
}

/// The winning team `{name, points}`, from the kit's results (teams[0], as pa7's results card).
fn winner(s: &Snapshot) -> Value {
    let results = s.kit().map(|k| k.results(&KitCtx::new(&s.spec, &s.players, Utc::now()), &s.kit_state)).unwrap_or(Value::Null);
    results["teams"].as_array().and_then(|t| t.first()).filter(|t| t["name"].is_string()).map(|t| json!({ "name": t["name"], "points": t["points"] })).unwrap_or(Value::Null)
}

/// The item's title: who won, or pa7's words for nobody.
pub fn title(pocket_title: &str, players: usize, winner: &Value) -> String {
    let t = pocket_title.trim();
    let t = if t.is_empty() { "Pocket app" } else { t };
    match (players, winner["name"].as_str()) {
        (0, _) => format!("{t}: Nobody joined."),
        (_, Some(w)) => format!("{t}: {w} win"),
        _ => format!("{t} has ended"),
    }
}

/// The space's Done items for posted pockets since `since`; `unread` against `seen`.
pub async fn items(state: &AppState, space: &SpaceId, since: DateTime<Utc>, seen: Option<DateTime<Utc>>) -> Result<Vec<Value>, DbError> {
    let mut out = Vec::new();
    for p in posted(state, space, since).await? {
        let s = &p.snap;
        let code = s.row.code.clone();
        let players = s.players.len();
        let win = winner(s);
        let kwant = match &p.kwant_id {
            Some(k) => state.db.get_kwant(k).await?,
            None => None,
        };
        let kwant_name = kwant.as_ref().map(|k| k.name.clone()).unwrap_or_default();
        let message = match state.db.pocket_result_post_of(&code).await? {
            Some(id) => state.db.get_message(&id).await?,
            None => None,
        };
        let unread = seen.is_none_or(|s| p.at > s);
        let id = format!("pocket:{code}");
        let title = title(&s.row.title, players, &win);
        let link = match (&message, &p.origin) {
            (Some(m), _) => json!({ "thread_id": m.thread_id, "channel_id": m.channel_id, "direct_kwant_id": m.direct_kwant_id }),
            (None, Some(o)) => json!({ "thread_id": o.thread_id, "channel_id": o.channel_id, "direct_kwant_id": o.channel_id.is_none().then(|| o.kwant_id.clone()) }),
            (None, None) => Value::Null,
        };
        let mut v = json!({
            "kind": "pocket",
            "id": id,
            "kwant_id": p.kwant_id,
            "kwant_name": kwant_name,
            "title": title,
            "summary": title,
            "status": "completed",
            "timestamp": rfc3339(p.at),
            "link": link,
            "group": Group::Finished.key(),
            "sort_key": sort_key(Group::Finished, None, p.at.timestamp_millis(), unread, &kwant_name, &id),
            "meta": { "players": players, "ended_at": s.row.ended_at.map(rfc3339) },
            "winner": win,
        });
        let source = json!({
            "kind": "pocket",
            "code": code,
            "kwant_id": p.kwant_id,
            "message_id": message.as_ref().map(|m| m.id.clone()),
            "channel_id": message.as_ref().and_then(|m| m.channel_id.clone()).or_else(|| p.origin.as_ref().and_then(|o| o.channel_id.clone())),
            "thread_id": message.as_ref().and_then(|m| m.thread_id.clone()).or_else(|| p.origin.as_ref().and_then(|o| o.thread_id.clone())),
        });
        inbox_done::apply(&mut v, &DoneFacts { message, source, ..Default::default() }, unread);
        out.push(v);
    }
    Ok(out)
}

/// How many of the space's posted pockets are newer than `seen_since` (the badge's count, as
/// `away_counts` counts finished work since the last-seen marker).
pub async fn unread(state: &AppState, space: &SpaceId, seen_since: DateTime<Utc>) -> Result<i64, DbError> {
    let since = inbox_done::done_since(Utc::now(), Some(seen_since));
    Ok(posted(state, space, since).await?.iter().filter(|p| p.at > seen_since).count() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_name_the_team_or_say_nobody_joined() {
        assert_eq!(title("Pub quiz", 4, &json!({ "name": "Team 1", "points": 3 })), "Pub quiz: Team 1 win");
        assert_eq!(title("Pub quiz", 0, &Value::Null), "Pub quiz: Nobody joined.");
        assert_eq!(title("  ", 2, &Value::Null), "Pocket app has ended");
    }
}
