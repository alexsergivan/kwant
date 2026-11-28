//! pa4c-server: Inbox › Done shows each pocket whose results were posted (pa7
//! `pocket_schedule.results_posted_at`) for [`inbox_done::DONE_DAYS`] days, as one finished item
//! in the same list and the same unseen count as the other Done items. Names and scores only:
//! the title names the winning team, never a guest. A deleted or expired pocket drops out.
//! Nothing shows while the `pocket_apps` switch is off.
//!
//! Item: `kind: "pocket"`, `id: "pocket:<code>"`, title `"<title>: <winner> win"` (pa7's
//! "Nobody joined." for no players, "<title> has ended" when nobody scored), `meta {players,
//! ended_at}` (queue-end-summary: a kit with its own end line, the queue, has `meta {summary,
//! ended_at}`, the title "<title> has ended" and `checked: null`), `winner {name, points}` (every team tied on the top score, null at 0 points),
//! `source {kind: "pocket", code, kwant_id, message_id, channel_id, thread_id}`, and the results
//! post's link and excerpt through [`inbox_done::apply`].
//!
//! desk-inbox-rows-server (m3): a front desk that ended while guests still waited says so:
//! title `"<title>: Ended with N unanswered"`, `ended_unanswered: N`, and the item opens the
//! desk's answering bot's chat (like its Needs-you rows) while that bot exists. One item per desk.

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

/// The winners `{name, points}`, from the kit's results ([`winner_of`]). A vote (the gather kit)
/// lists its decided winner first, with final-round counts: a tie the host broke still shows
/// equal counts, so it keeps the first team only.
fn winner(s: &Snapshot) -> Value {
    let results = s.kit().map(|k| k.results(&KitCtx::new(&s.spec, &s.players, Utc::now()), &s.kit_state)).unwrap_or(Value::Null);
    winner_of(&results, s.row.kit != pocket::gather::ID)
}

/// quiz-tie-names-all: with `by_score` (kits whose results rank by points, best first, with no
/// rank), every team sharing the top score, named the TV's way ("A", "A and B", "A, B and C"),
/// with that score; null when nobody scored (a top score of 0 or less), so a 0-0 quiz crowns no
/// one and the title says it ended. Without it (a decided vote), the first team, as before.
pub fn winner_of(results: &Value, by_score: bool) -> Value {
    let Some(teams) = results["teams"].as_array() else { return Value::Null };
    let Some(top) = teams.first().filter(|t| t["name"].is_string()) else { return Value::Null };
    if !by_score {
        return json!({ "name": top["name"], "points": top["points"] });
    }
    let points = top["points"].as_f64().unwrap_or(0.0);
    if points <= 0.0 {
        return Value::Null;
    }
    let names: Vec<&str> = teams.iter().filter(|t| t["points"].as_f64() == Some(points)).filter_map(|t| t["name"].as_str()).collect();
    let name = match names.as_slice() {
        [] => return Value::Null,
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    json!({ "name": name, "points": top["points"] })
}

/// The item's title: who won, or pa7's words for nobody.
pub fn title(pocket_title: &str, players: usize, winner: &Value) -> String {
    let t = pocket_title.trim();
    let t = if t.is_empty() { "Pocket app" } else { t };
    match (players, winner["name"].as_str()) {
        (0, _) => format!("{t}: Nobody joined."),
        (_, Some(w)) => format!("{t}: {w} win"),
        _ => ended_title(pocket_title),
    }
}

/// "<title> has ended" (a kit with its own end line, or nobody scored).
pub fn ended_title(pocket_title: &str) -> String {
    let t = pocket_title.trim();
    let t = if t.is_empty() { "Pocket app" } else { t };
    format!("{t} has ended")
}

/// m3: a desk that ended with `n` guests still waiting for the host.
pub fn ended_unanswered_title(pocket_title: &str, n: u64) -> String {
    let t = pocket_title.trim();
    let t = if t.is_empty() { "Front desk" } else { t };
    format!("{t}: Ended with {n} unanswered")
}

/// The space's Done items for posted pockets since `since`; `unread` against `seen`.
pub async fn items(state: &AppState, space: &SpaceId, since: DateTime<Utc>, seen: Option<DateTime<Utc>>) -> Result<Vec<Value>, DbError> {
    let mut out = Vec::new();
    for p in posted(state, space, since).await? {
        let s = &p.snap;
        let code = s.row.code.clone();
        let players = s.player_count();
        let win = winner(s);
        // queue-end-summary: a kit's own line (the queue's "1 day · 3 served · average 1 min")
        let summary = s.kit().and_then(|k| k.end_summary(&s.kit_state)).filter(|l| !l.trim().is_empty());
        // m3: a desk left with guests waiting opens its answering bot's chat
        let unanswered = if s.row.kit == pocket::desk::ID { pocket::desk::ended_unanswered(&s.kit_state) } else { 0 };
        let desk_bot = match s.spec["answer_bot"].as_str().filter(|b| unanswered > 0 && !b.is_empty()) {
            Some(b) => state.db.get_kwant(b).await?,
            None => None,
        };
        let kwant = match (&desk_bot, &p.kwant_id) {
            (Some(b), _) => Some(b.clone()),
            (None, Some(k)) => state.db.get_kwant(k).await?,
            (None, None) => None,
        };
        let kwant_id = kwant.as_ref().filter(|_| desk_bot.is_some()).map(|k| k.id.clone()).or_else(|| p.kwant_id.clone());
        let kwant_name = kwant.as_ref().map(|k| k.name.clone()).unwrap_or_default();
        let message = match state.db.pocket_result_post_of(&code).await? {
            Some(id) => state.db.get_message(&id).await?,
            None => None,
        };
        let unread = seen.is_none_or(|s| p.at > s);
        let id = format!("pocket:{code}");
        let title = if unanswered > 0 {
            ended_unanswered_title(&s.row.title, unanswered)
        } else if summary.is_some() {
            ended_title(&s.row.title)
        } else {
            title(&s.row.title, players, &win)
        };
        let link = match (&desk_bot, &message, &p.origin) {
            (Some(b), _, _) => json!({ "thread_id": null, "channel_id": null, "direct_kwant_id": b.id }),
            (None, Some(m), _) => json!({ "thread_id": m.thread_id, "channel_id": m.channel_id, "direct_kwant_id": m.direct_kwant_id }),
            (None, None, Some(o)) => json!({ "thread_id": o.thread_id, "channel_id": o.channel_id, "direct_kwant_id": o.channel_id.is_none().then(|| o.kwant_id.clone()) }),
            (None, None, None) => Value::Null,
        };
        let mut v = json!({
            "kind": "pocket",
            "id": id,
            "kwant_id": kwant_id,
            "kwant_name": kwant_name,
            "title": title,
            "summary": title,
            "status": "completed",
            "timestamp": rfc3339(p.at),
            "link": link,
            "group": Group::Finished.key(),
            "sort_key": sort_key(Group::Finished, None, p.at.timestamp_millis(), unread, &kwant_name, &id),
            "meta": match &summary {
                Some(l) => json!({ "summary": l, "ended_at": s.row.ended_at.map(rfc3339) }),
                None => json!({ "players": players, "ended_at": s.row.ended_at.map(rfc3339) }),
            },
            "winner": win,
        });
        if unanswered > 0 {
            v["ended_unanswered"] = json!(unanswered);
        }
        let source = json!({
            "kind": "pocket",
            "code": code,
            "kwant_id": p.kwant_id,
            "message_id": message.as_ref().map(|m| m.id.clone()),
            "channel_id": message.as_ref().and_then(|m| m.channel_id.clone()).or_else(|| p.origin.as_ref().and_then(|o| o.channel_id.clone())),
            "thread_id": message.as_ref().and_then(|m| m.thread_id.clone()).or_else(|| p.origin.as_ref().and_then(|o| o.thread_id.clone())),
        });
        inbox_done::apply(&mut v, &DoneFacts { message, source, ..Default::default() }, unread);
        if summary.is_some() {
            // A count, not a result to verify: no "Not checked" strip on the post or the row
            v["checked"] = Value::Null;
        }
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

    #[test]
    fn a_desk_left_with_guests_waiting_says_how_many() {
        assert_eq!(ended_unanswered_title("Seaview desk", 1), "Seaview desk: Ended with 1 unanswered");
        assert_eq!(ended_unanswered_title(" ", 3), "Front desk: Ended with 3 unanswered");
    }

    fn results(scores: &[(&str, i64)]) -> Value {
        let teams: Vec<Value> = scores.iter().map(|(n, p)| json!({ "name": n, "players": 1, "points": p, "palette": null, "members": [] })).collect();
        json!({ "teams": teams, "margin": null })
    }

    #[test]
    fn a_clear_win_names_one_team() {
        let w = winner_of(&results(&[("Team 1", 3), ("Team 2", 1)]), true);
        assert_eq!(w, json!({ "name": "Team 1", "points": 3 }));
        assert_eq!(title("Pub quiz", 2, &w), "Pub quiz: Team 1 win");
    }

    #[test]
    fn a_two_way_tie_names_both_teams() {
        let w = winner_of(&results(&[("Quizzy Rascals", 7), ("Brainy Bunch", 7), ("Late Comers", 2)]), true);
        assert_eq!(w, json!({ "name": "Quizzy Rascals and Brainy Bunch", "points": 7 }));
        assert_eq!(title("Friday Pub Quiz", 6, &w), "Friday Pub Quiz: Quizzy Rascals and Brainy Bunch win");
    }

    #[test]
    fn a_three_way_tie_reads_like_the_tv() {
        let w = winner_of(&results(&[("A", 4), ("B", 4), ("C", 4), ("D", 3)]), true);
        assert_eq!(w, json!({ "name": "A, B and C", "points": 4 }));
    }

    #[test]
    fn a_nil_nil_quiz_crowns_nobody_and_says_it_ended() {
        let w = winner_of(&results(&[("Quizzy Rascals", 0), ("Brainy Bunch", 0)]), true);
        assert!(w.is_null(), "no holo Winners sticker for 0 points: {w}");
        assert_eq!(title("Friday Pub Quiz", 4, &w), "Friday Pub Quiz has ended");
        assert!(winner_of(&results(&[("Solo", 0)]), true).is_null());
    }

    #[test]
    fn solo_ties_name_the_players() {
        let w = winner_of(&results(&[("Player 1", 5), ("Player 2", 5)]), true);
        assert_eq!(title("Quiz", 2, &w), "Quiz: Player 1 and Player 2 win");
    }

    #[test]
    fn a_vote_the_host_broke_names_only_the_pick() {
        // gather lists the host's pick first; the tied options keep equal final-round counts
        let w = winner_of(&results(&[("Pizza", 3), ("Tacos", 3), ("Sushi", 1)]), false);
        assert_eq!(w, json!({ "name": "Pizza", "points": 3 }));
        assert_eq!(title("Club dinner", 7, &w), "Club dinner: Pizza win");
        // a decided winner missing from the last round is listed with 0 and still wins
        assert_eq!(winner_of(&results(&[("Pizza", 0), ("Tacos", 2)]), false), json!({ "name": "Pizza", "points": 0 }));
    }

    #[test]
    fn no_results_no_winner() {
        assert!(winner_of(&Value::Null, true).is_null());
        assert!(winner_of(&json!({ "teams": [] }), true).is_null());
    }
}
