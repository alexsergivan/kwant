//! pa4c-server: Inbox › Done shows each pocket whose results were posted (pa7
//! `pocket_schedule.results_posted_at`) for [`inbox_done::DONE_DAYS`] days, as one finished item
//! in the same list and the same unseen count as the other Done items. Names and scores only:
//! the title names the winning team, never a guest. A deleted or expired pocket drops out.
//! Nothing shows while the `pocket_apps` switch is off.
//!
//! Item: `kind: "pocket"`, `id: "pocket:<code>"`, title `"<title>: <winner> win"` ("wins" for a vote; pa7's
//! "Nobody joined." for no players, "<title> has ended" when nobody scored), `meta {players,
//! ended_at}`, `winner {name, points}` (every team tied on the top score, null at 0 points),
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
    title_as(pocket_title, players, winner, "win")
}

/// [`title`] with the kit's verb: teams "win", a vote's one option "wins" (m1: "Party theme:
/// Winter wonderland wins").
pub fn title_as(pocket_title: &str, players: usize, winner: &Value, verb: &str) -> String {
    let t = pocket_title.trim();
    let t = if t.is_empty() { "Pocket app" } else { t };
    match (players, winner["name"].as_str()) {
        (0, _) => format!("{t}: Nobody joined."),
        (_, Some(w)) => format!("{t}: {w} {verb}"),
        _ => format!("{t} has ended"),
    }
}

/// The space's Done items for posted pockets since `since`; `unread` against `seen`.
pub async fn items(state: &AppState, space: &SpaceId, since: DateTime<Utc>, seen: Option<DateTime<Utc>>) -> Result<Vec<Value>, DbError> {
    let mut out = Vec::new();
    for p in posted(state, space, since).await? {
        let s = &p.snap;
        let code = s.row.code.clone();
        let players = s.player_count();
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
        let title = title_as(&s.row.title, players, &win, if s.row.kit == pocket::gather::ID { "wins" } else { "win" });
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
        // m15: the server counted these results itself; there is nothing for a check to verify, so
        // the row and the chat's result line show a plain Done, never "Not checked"
        v["checked"] = Value::Null;
        out.push(v);
    }
    Ok(out)
}

/// party-card-and-results (pa14 design :96): an open Photo Wall with photos waiting for the host
/// is one Needs-you row, never one per photo: "1 photo waiting on Office holiday party",
/// "3 photos waiting on …". It names the next photo (`next_photo {id, by_name}`, oldest
/// first) so the row can show it with Approve / Hide, which go through the wall's own `approve` /
/// `hide` host actions; the row itself opens the chat the wall was made in, where its card is.
/// Id `wall:<code>`; the time is the newest waiting photo's. Only walls of the space's own bots,
/// as [`posted`] decides it (the origin bot, else the owner bot).
pub async fn wall_needs_you(state: &AppState, space: &SpaceId) -> Result<Vec<(DateTime<Utc>, Value)>, DbError> {
    let mut out = Vec::new();
    if !pocket::enabled_for(&state.db, pocket::wall::ID).await {
        return Ok(out);
    }
    let now = Utc::now();
    for code in state.db.pocket_codes_open_of_kit(pocket::wall::ID).await? {
        let Ok(Some(s)) = pocket::snapshot(state, &code).await else { continue };
        let waiting = pocket::wall::waiting_photos(&KitCtx::new(&s.spec, &s.players, now), &s.kit_state);
        let Some((next_id, next_by, _)) = waiting.first().cloned() else { continue };
        let origin: Option<Origin> = state.db.pocket_schedule(&code).await?.and_then(|r| r.origin).and_then(|o| serde_json::from_str(&o).ok());
        let Some(kwant_id) = origin.as_ref().map(|o| o.kwant_id.clone()).or_else(|| s.row.owner_kwant_id.clone()) else { continue };
        if &state.db.space_of_kwant(&kwant_id).await?.unwrap_or_else(SpaceId::default_space) != space {
            continue;
        }
        let kwant_name = state.db.get_kwant(&kwant_id).await?.map(|k| k.name).unwrap_or_default();
        let at = waiting.iter().filter_map(|w| w.2).max().unwrap_or(s.row.created_at);
        let link = match &origin {
            Some(o) => json!({ "thread_id": o.thread_id, "channel_id": o.channel_id, "direct_kwant_id": o.channel_id.is_none().then(|| o.kwant_id.clone()) }),
            None => json!({ "thread_id": null, "channel_id": null, "direct_kwant_id": kwant_id }),
        };
        out.push((at, json!({
            "kind": "pocket_wall",
            "id": format!("wall:{code}"),
            "pocket_code": code,
            "pocket_title": s.row.title,
            "kwant_id": kwant_id,
            "kwant_name": kwant_name,
            "title": wall_needs_title(&s.row.title, waiting.len()),
            "summary": next_by.as_deref().map(|n| format!("Next: a photo from {n}")).unwrap_or_else(|| "Next: a photo".to_string()),
            "waiting": waiting.len(),
            "next_photo": { "id": next_id, "by_name": next_by },
            "link": link,
        })));
    }
    Ok(out)
}

/// Count first, so a narrow Inbox list that truncates still shows it (pa14 design :96, Mira's
/// design check fix 2): "1 photo waiting on Office holiday party" / "3 photos waiting on …". The
/// wall's own title, trimmed; "your photo wall" when it has none. The "Photos to check" label
/// already says it is photos, so no " wall" is added.
pub fn wall_needs_title(pocket_title: &str, waiting: usize) -> String {
    let t = pocket_title.trim();
    let name = if t.is_empty() { "your photo wall" } else { t };
    match waiting {
        1 => format!("1 photo waiting on {name}"),
        n => format!("{n} photos waiting on {name}"),
    }
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
        assert_eq!(title_as("Club dinner", 7, &w, "wins"), "Club dinner: Pizza wins");
        // a decided winner missing from the last round is listed with 0 and still wins
        assert_eq!(winner_of(&results(&[("Pizza", 0), ("Tacos", 2)]), false), json!({ "name": "Pizza", "points": 0 }));
    }

    #[test]
    fn a_wall_row_counts_its_waiting_photos_in_one_line() {
        assert_eq!(wall_needs_title("Office holiday party", 1), "1 photo waiting on Office holiday party");
        assert_eq!(wall_needs_title(" Office holiday party ", 3), "3 photos waiting on Office holiday party");
        assert_eq!(wall_needs_title("Mia & Jonas's wedding", 9), "9 photos waiting on Mia & Jonas's wedding");
        assert_eq!(wall_needs_title("  ", 2), "2 photos waiting on your photo wall");
        assert_eq!(wall_needs_title("", 1), "1 photo waiting on your photo wall");
    }

    #[test]
    fn no_results_no_winner() {
        assert!(winner_of(&Value::Null, true).is_null());
        assert!(winner_of(&json!({ "teams": [] }), true).is_null());
    }
}
