//! The Quizmaster's card in chat (pa3c, `autonomy/pocket-apps/pa3c/brief.md`): a bot reply whose
//! turn called one of kwant's own Quizmaster tools (`pocket_create` … `pocket_close`) gets a
//! `pocket` block, built from the quiz draft kwant stored, never from the call's text. It carries
//! `draft` (the quiz_drafts id the card's Edit questions opens) and `yours` (the questions the
//! person wrote or changed themselves).
//!
//! The call only names a draft. That draft is used only when it is this bot's own, in the space
//! the reply is in: the reply's sender is the draft's writer (by name and space), a direct chat is
//! that bot's, and a channel is in the draft's space. Anything else (another space's or another
//! bot's draft, no such draft, a call that failed, another server's tool of the same name, Pocket
//! apps switched off) gives no block.
//!
//! The message kwnt posts when a pocket ends gets the results card from the block it carries in
//! its `meta` ([`from_meta`]).

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::SqliteConnection;

use crate::db::models::{MessageExtra, NewMessage};
use crate::db::quiz_drafts::QuizDraftRow;
use crate::mcp::SELF_SERVER_NAME;
use crate::quizmaster::card::{Card, State};
use crate::server::quizmaster_tools as qt;

use super::validate::utf16_len;
use super::verification::safe;
use super::{Block, Pocket, PocketChecks, PocketDryRun, PocketProgress, Provenance, Source};

/// What `pocket_create` answers with: `Card::text()` starts with this, then the draft id and `]`.
const CARD_TEXT_HEAD: &str = "Pocket app · Quiz [";

/// The `pocket` block for a bot reply, or `None` (see the module docs).
pub async fn candidate(conn: &mut SqliteConnection, input: &NewMessage, extra: &MessageExtra) -> Option<Block> {
    if input.sender_role != "kwant" {
        return None;
    }
    let named = draft_named(input)?;
    let on = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?").bind(qt::SETTING).fetch_optional(&mut *conn).await.ok()?;
    if on.as_deref() != Some("on") {
        return None;
    }
    let row = match named {
        Named::Id(id) => sqlx::query_as::<_, QuizDraftRow>("SELECT * FROM quiz_drafts WHERE id = ?").bind(id.as_str()).fetch_optional(&mut *conn).await.ok()??,
        Named::MadeThisTurn => made_this_turn(&mut *conn, input, extra).await?,
    };
    if !belongs(&mut *conn, &row, input, extra).await {
        return None;
    }
    let tz = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
        .bind(crate::server::bot_actions::TIMEZONE_SETTING)
        .fetch_optional(&mut *conn)
        .await
        .ok()
        .flatten();
    Some(Block::Pocket(Box::new(block(&row, &qt::card_of(&row), tz.as_deref()))))
}

/// Which draft a turn's Quizmaster calls name.
#[derive(Debug, PartialEq)]
enum Named {
    /// `arguments.id`, or the id a `pocket_create` card's text starts with.
    Id(String),
    /// A `pocket_create` that ran with a spec but whose answer wasn't recorded: the Claude CLI
    /// engine records kwnt_tools calls as "allowed" with no `result` (quiz-card-and-results). The
    /// draft it made is this bot's newest one created since the turn began ([`made_this_turn`]).
    MadeThisTurn,
}

/// The draft the turn's last Quizmaster call that answered with a card named: `arguments.id`, or
/// for `pocket_create` (which has none) the id its card text starts with. A call that worked is
/// "ok", or "allowed" on the Claude CLI engine, which records the call when it is let through and
/// never its answer; a draft still being written comes back as an "error" whose answer is that
/// draft's own card (so the bot fixes it), and counts too. Any other failure doesn't. Shape-checked
/// only; whose draft it is gets checked against the stored row.
fn draft_named(input: &NewMessage) -> Option<Named> {
    let calls = input.tool_calls.as_ref()?.as_array()?;
    calls.iter().rev().find_map(|c| {
        if c.get("server").and_then(Value::as_str) != Some(SELF_SERVER_NAME) {
            return None;
        }
        let tool = c.get("tool").and_then(Value::as_str).filter(|t| qt::TOOLS.contains(t))?;
        let status = c.get("status").and_then(Value::as_str)?;
        let result = c.get("result").and_then(Value::as_str);
        let card_id = result.and_then(card_text_id);
        let id = match c.get("arguments").and_then(|a| a.get("id")).and_then(Value::as_str) {
            Some(id) => id.trim().to_string(),
            // A create with a spec that ran, with no answer kept: the draft it made
            None if tool == "pocket_create" && status == "allowed" && result.is_none() => {
                let spec = c.get("arguments").and_then(|a| a.get("spec")).filter(|s| !s.is_null());
                return spec.map(|_| Named::MadeThisTurn);
            }
            None if tool == "pocket_create" => card_id.clone()?,
            None => return None,
        };
        let answered = match status {
            "ok" | "allowed" => true,
            "error" => card_id.as_deref() == Some(id.as_str()),
            _ => false,
        };
        (answered && valid_id(&id)).then_some(Named::Id(id))
    })
}

/// The reply's bot's newest draft created since the turn began: no earlier than the chat's newest
/// message from anyone but this bot (the ask the turn answers). `None` without such a message or
/// draft. Whose draft it is still goes through [`belongs`].
async fn made_this_turn(conn: &mut SqliteConnection, input: &NewMessage, extra: &MessageExtra) -> Option<QuizDraftRow> {
    let (chat, by_channel) = match (&input.direct_kwant_id, &input.channel_id) {
        (Some(k), _) => (k.as_str(), false),
        (None, Some(c)) => (c.as_str(), true),
        (None, None) => return None,
    };
    let column = if by_channel { "channel_id" } else { "direct_kwant_id" };
    let asked: DateTime<Utc> = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(&format!(
        "SELECT created_at FROM messages WHERE {column} = ? AND NOT (sender_role = 'kwant' AND sender_id = ?) ORDER BY created_at DESC, rowid DESC LIMIT 1"
    ))
    .bind(chat)
    .bind(&input.sender_id)
    .fetch_optional(&mut *conn)
    .await
    .ok()???;
    // The bot: a direct chat's own, else the one the card names, else the sender by name in the channel's space
    let kwant = match (&input.direct_kwant_id, extra.meta.as_ref().and_then(|m| m.get("kwant_id")).and_then(Value::as_str)) {
        (Some(k), _) => k.clone(),
        (None, Some(k)) => k.to_string(),
        (None, None) => sqlx::query_scalar::<_, String>("SELECT k.id FROM kwants k JOIN channels c ON c.space_id = k.space_id WHERE c.id = ? AND k.name = ? LIMIT 1")
            .bind(chat)
            .bind(&input.sender_id)
            .fetch_optional(&mut *conn)
            .await
            .ok()??,
    };
    // Message times keep whole seconds; drafts keep milliseconds
    let since = asked.timestamp() * 1000;
    sqlx::query_as::<_, QuizDraftRow>("SELECT * FROM quiz_drafts WHERE kwant_id = ? AND created_at >= ? ORDER BY created_at DESC, rowid DESC LIMIT 1")
        .bind(kwant)
        .bind(since)
        .fetch_optional(&mut *conn)
        .await
        .ok()?
}

/// The draft id a card's text starts with (`Card::text()`: `Pocket app · Quiz [<id>] …`).
fn card_text_id(text: &str) -> Option<String> {
    let rest = text.trim_start().strip_prefix(CARD_TEXT_HEAD)?;
    Some(rest[..rest.find(']')?].to_string())
}

/// A draft id's shape (`qd_` and 12 hex digits today; the route's own rule is looser).
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The draft is the reply sender's own, in the reply's space.
async fn belongs(conn: &mut SqliteConnection, row: &QuizDraftRow, input: &NewMessage, extra: &MessageExtra) -> bool {
    let writer = match sqlx::query_as::<_, (String, String)>("SELECT name, space_id FROM kwants WHERE id = ?").bind(&row.kwant_id).fetch_optional(&mut *conn).await {
        Ok(w) => w,
        Err(_) => return false,
    };
    let Some((name, space)) = writer else { return false };
    if name != input.sender_id || space != row.space_id {
        return false;
    }
    // A card that names its bot must name this one
    if let Some(k) = extra.meta.as_ref().and_then(|m| m.get("kwant_id")) {
        if k.as_str() != Some(row.kwant_id.as_str()) {
            return false;
        }
    }
    match (&input.direct_kwant_id, &input.channel_id) {
        (Some(direct), _) => *direct == row.kwant_id,
        (None, Some(channel)) => {
            let space =
                sqlx::query_scalar::<_, String>("SELECT space_id FROM channels WHERE id = ?").bind(channel).fetch_optional(&mut *conn).await.ok().flatten();
            space.as_deref() == Some(row.space_id.as_str())
        }
        (None, None) => false,
    }
}

/// Kits whose ended pocket the chat draws as the quiz results card. The other kits carry parts of
/// their own (`santa`, `wall`, `gather`, `desk`) that this block doesn't, and the chat refuses one
/// without them. `test` exists only in debug builds.
const RESULT_KITS: &[&str] = &[crate::kits::quiz::KIT_ID, "test"];

/// The most player names a results card's team shows (the chat's `validTeam` and
/// [`super::validate`] refuse more).
const MAX_CARD_MEMBERS: usize = 12;

/// The results card of a message kwnt posted when a pocket ended (`meta.kind` "pocket",
/// `reason` "results", `src/pocket/schedule.rs` `post_results`), read from the `meta.block` it
/// built from the pocket's own data (quiz-card-and-results, Mira's ux-pass-0952 fix #2). `None`
/// for any other pocket message (a reminder, which has no block), another kit, a block whose code
/// isn't the message's, or one that doesn't fit. Guest-typed texts (team and player names) are
/// cleaned and bounded the way [`fit`] does; the block then still has to pass [`super::validate`].
pub fn from_meta(extra: &MessageExtra) -> Option<Block> {
    let meta = extra.meta.as_ref()?;
    if meta.get("reason").and_then(Value::as_str) != Some("results") {
        return None;
    }
    let code = meta.get("code").and_then(Value::as_str).filter(|c| valid_code(c))?;
    let raw = meta.get("block").filter(|b| b.is_object())?;
    let mut p: Pocket = serde_json::from_value(raw.clone()).ok()?;
    let reference = format!("pocket:{code}");
    if p.state != "ended" || p.code != code || p.provenance.reference.as_deref() != Some(reference.as_str()) || !RESULT_KITS.contains(&p.kit.as_str()) {
        return None;
    }
    // One block per message, so its id is the set's own
    p.id = "b0".into();
    p.provenance = Provenance::kwant_data(reference);
    p.name = fit(p.name.trim(), 80);
    let clean = |v: &mut Value, max: usize| {
        if let Some(t) = v.as_str() {
            *v = Value::String(fit(t.trim(), max));
        }
    };
    if let Some(w) = p.ended.as_mut().and_then(|e| e.get_mut("winner")).and_then(|w| w.get_mut("name")) {
        clean(w, 40);
    }
    // Results posted before the card learned its times kept raw timestamps; the card prints these as they
    // are, so they read as the card's words do, in UTC (the message doesn't keep the person's zone)
    if let Some(e) = p.ended.as_mut().and_then(Value::as_object_mut) {
        let raw = |k: &str| e.get(k).and_then(Value::as_str).and_then(|t| DateTime::parse_from_rfc3339(t.trim()).ok()).map(|t| t.naive_utc());
        let (at, deletes) = (raw("at"), raw("deletes_on"));
        if let Some(t) = at {
            e.insert("at".into(), Value::String(format!("{} UTC", crate::pocket::schedule::clock(t))));
        }
        if let Some(t) = deletes {
            e.insert("deletes_on".into(), Value::String(t.format("%a, %b %-d (UTC)").to_string()));
        }
    }
    if let Some(teams) = p.results.as_mut().and_then(|r| r.get_mut("teams")).and_then(Value::as_array_mut) {
        for team in teams.iter_mut() {
            if let Some(n) = team.get_mut("name") {
                clean(n, 40);
            }
            // The kit lists every player of a team; the card names at most 12 (`players` keeps the
            // real count), and a name that cleans to nothing is left out rather than refusing the card
            if let Some(members) = team.get_mut("members").and_then(Value::as_array_mut) {
                members.iter_mut().for_each(|m| clean(m, 24));
                members.retain(|m| m.as_str().is_none_or(|t| !t.trim().is_empty()));
                members.truncate(MAX_CARD_MEMBERS);
            }
        }
    }
    Some(Block::Pocket(Box::new(p)))
}

/// The card as the chat draws it. Every text is kwant's own, cleaned and bounded the way
/// [`super::validate`] checks it.
fn block(row: &QuizDraftRow, card: &Card, tz: Option<&str>) -> Pocket {
    let state = match card.state {
        State::Open => "live",
        State::Closed => "ended",
        s => s.as_str(),
    };
    let code = row.pocket_code.as_deref().filter(|c| valid_code(c)).unwrap_or(card.id.as_str()).to_string();
    let name = match fit(card.title.trim(), 80) {
        n if n.trim().is_empty() => "Untitled quiz".to_string(),
        n => n,
    };
    let checked = card.checked.min(card.total) as u64;
    let total = card.total as u64;
    let progress = (card.state == State::Checking && total > 0).then_some(PocketProgress { done: checked, total });
    let checks = (card.state != State::Writing && total > 0).then(|| PocketChecks {
        matched: checked,
        total,
        swapped: card.failed.iter().take(20).map(|f| format!("Q{}", f.q + 1)).collect(),
    });
    let dry_run = match card.state {
        State::Ready | State::Open | State::Closed => Some(PocketDryRun {
            players: crate::quizmaster::DRY_RUN_PLAYERS as u64,
            passed: true,
            stopped_at: None,
            what_happened: None,
            what_im_doing: None,
        }),
        State::DryRunStopped => {
            let line = card.dry_run.as_ref();
            let happened = card.problems.first().or_else(|| line.and_then(|d| d.problems.first())).or(line.map(|d| &d.summary));
            Some(PocketDryRun {
                players: crate::quizmaster::DRY_RUN_PLAYERS as u64,
                passed: false,
                stopped_at: None,
                what_happened: happened.map(|t| fit(t.trim(), 400)).filter(|t| !t.trim().is_empty()),
                what_im_doing: Some("Fixing what the dry run found, then running it again.".into()),
            })
        }
        State::Writing | State::Checking => None,
    };
    let reference = format!("pocket:{}", card.id);
    let (notice, join) = class_parts(row, state);
    Pocket {
        id: "b0".into(),
        title: None,
        source: Source::slot(),
        provenance: Provenance::kwant_data(reference),
        code,
        kit: "quiz".into(),
        name,
        when: when(card, tz),
        state: state.into(),
        paused: false,
        progress,
        rounds: Vec::new(),
        checks,
        dry_run,
        eta: None,
        opens_line: None,
        live: None,
        join,
        ended: None,
        budget: None,
        results: None,
        draft: Some(card.id.clone()),
        yours: Some(card.yours as u64),
        notice,
    }
}

/// pa11 qa F6: from the draft's spec, the kit's notice (a class quiz's line; not once it ended,
/// where the deletion line says it) and, on a ready or live card, a `join` saying there is no
/// quick link when the pocket core would refuse one for this spec (`pocket::tunnel::spec_refusal`,
/// the 409's own rule). The join URLs stay null: the card doesn't carry them today.
fn class_parts(row: &QuizDraftRow, state: &str) -> (Option<String>, Option<Value>) {
    let Ok(spec) = serde_json::from_str::<Value>(&row.spec) else { return (None, None) };
    let kit = crate::pocket::kits::kit(crate::kits::quiz::KIT_ID);
    let notice = (state != "ended").then(|| kit.and_then(|k| k.notice(&spec))).flatten().map(|n| fit(n, NOTICE_MAX));
    let no_quick = matches!(crate::pocket::tunnel::spec_refusal(kit, &spec), Some(("needs_stable_address", _)));
    let join = (no_quick && matches!(state, "ready" | "live"))
        .then(|| serde_json::json!({ "mode": "anywhere", "wifi_url": null, "anywhere_url": null, "quick_tunnel": false }));
    (notice, join)
}

/// The frontend's limit on `notice` (`validPocket`'s `optText(b.notice, 300)`).
pub(crate) const NOTICE_MAX: usize = 300;

/// "Fri, Nov 27 · 8:00 PM · 4 rounds · 40 questions", in the person's time zone (the pocket
/// scheduler's own wording); `None` without a time the spec states as RFC 3339.
fn when(card: &Card, tz: Option<&str>) -> Option<String> {
    let at = DateTime::parse_from_rfc3339(card.when.as_deref()?.trim()).ok()?;
    let zone = crate::pocket::schedule::zone_for(tz, Some(at));
    let mut line = crate::pocket::schedule::when_line(&zone, at.with_timezone(&Utc));
    if card.questions > 0 {
        line.push_str(&format!(" · {} rounds · {} questions", card.rounds, card.questions));
    }
    Some(fit(&line, 160))
}

/// The frontend's (and pocketApi's) rule for a pocket code.
pub(crate) fn valid_code(code: &str) -> bool {
    (3..=32).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// [`safe`], and within `max` as the client counts length (UTF-16 units: an emoji is two).
fn fit(text: &str, max: usize) -> String {
    let out = safe(text, max);
    if utf16_len(&out) <= max {
        return out;
    }
    safe(text, max / 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::smart_results::{validate, ResultSet, VERSION};

    fn sample() -> Pocket {
        Pocket {
            id: "b0".into(),
            title: None,
            source: Source::slot(),
            provenance: Provenance::kwant_data("pocket:qd_0123456789ab".into()),
            code: "qd_0123456789ab".into(),
            kit: "quiz".into(),
            name: "Friday Pub Quiz".into(),
            when: Some("Fri, Nov 27 · 8:00 PM · 4 rounds · 40 questions".into()),
            state: "ready".into(),
            paused: false,
            progress: None,
            rounds: Vec::new(),
            checks: Some(PocketChecks { matched: 40, total: 40, swapped: Vec::new() }),
            dry_run: Some(PocketDryRun { players: 30, passed: true, stopped_at: None, what_happened: None, what_im_doing: None }),
            eta: None,
            opens_line: None,
            live: None,
            join: None,
            ended: None,
            budget: None,
            results: None,
            draft: Some("qd_0123456789ab".into()),
            yours: Some(1),
            notice: None,
        }
    }

    type Change = Box<dyn Fn(&mut Pocket)>;

    fn check(p: Pocket) -> Result<(), String> {
        validate(&ResultSet { version: VERSION, blocks: vec![Block::Pocket(Box::new(p))] }, None)
    }

    #[test]
    fn a_ready_card_passes_and_serializes_draft_and_yours() {
        let p = sample();
        assert_eq!(check(p.clone()), Ok(()));
        let block = Block::Pocket(Box::new(p.clone()));
        let v = serde_json::to_value(&block).unwrap();
        assert_eq!(v["type"], "pocket");
        // The Box adds nothing to the wire: the block is exactly the card's own fields plus `type`,
        // and it reads back to the same block.
        let mut want = serde_json::to_value(&p).unwrap();
        want.as_object_mut().unwrap().insert("type".into(), "pocket".into());
        assert_eq!(v, want);
        assert_eq!(serde_json::from_str::<Block>(&serde_json::to_string(&block).unwrap()).unwrap(), block);
        assert_eq!(v["draft"], "qd_0123456789ab");
        assert_eq!(v["yours"], 1);
        assert!(v["results"].is_null() && v["join"].is_null());
    }

    #[test]
    fn each_rule_refuses_its_block() {
        let bad: Vec<(&str, Change)> = vec![
            ("reply_text provenance", Box::new(|p: &mut Pocket| p.provenance = Provenance::reply_text())),
            ("a task ref", Box::new(|p: &mut Pocket| p.provenance = Provenance::kwant_data("task:t1".into()))),
            ("a ref with a slash", Box::new(|p: &mut Pocket| p.provenance = Provenance::kwant_data("pocket:../x".into()))),
            ("a table source", Box::new(|p: &mut Pocket| p.source = Source::table(0))),
            ("a title", Box::new(|p: &mut Pocket| p.title = Some("Quiz".into()))),
            ("a short code", Box::new(|p: &mut Pocket| p.code = "ab".into())),
            ("a code with a space", Box::new(|p: &mut Pocket| p.code = "ab cd".into())),
            ("a kit in capitals", Box::new(|p: &mut Pocket| p.kit = "Quiz".into())),
            ("a blank name", Box::new(|p: &mut Pocket| p.name = " ".into())),
            ("HTML in the name", Box::new(|p: &mut Pocket| p.name = "<b>Quiz</b>".into())),
            ("a long name", Box::new(|p: &mut Pocket| p.name = "x".repeat(81))),
            ("an unknown state", Box::new(|p: &mut Pocket| p.state = "open".into())),
            ("a long time line", Box::new(|p: &mut Pocket| p.when = Some("x".repeat(161)))),
            ("paused but not live", Box::new(|p: &mut Pocket| p.paused = true)),
            ("progress over its total", Box::new(|p: &mut Pocket| p.progress = Some(PocketProgress { done: 3, total: 2 }))),
            ("progress of nothing", Box::new(|p: &mut Pocket| p.progress = Some(PocketProgress { done: 0, total: 0 }))),
            ("checks over the total", Box::new(|p: &mut Pocket| p.checks = Some(PocketChecks { matched: 41, total: 40, swapped: Vec::new() }))),
            ("a long swapped label", Box::new(|p: &mut Pocket| p.checks = Some(PocketChecks { matched: 1, total: 2, swapped: vec!["x".repeat(13)] }))),
            ("ready without a dry run", Box::new(|p: &mut Pocket| p.dry_run = None)),
            ("ready with a failed dry run", Box::new(|p: &mut Pocket| p.dry_run.as_mut().unwrap().passed = false)),
            ("stopped with a passed dry run", Box::new(|p: &mut Pocket| p.state = "dry_run_stopped".into())),
            ("a long dry run line", Box::new(|p: &mut Pocket| p.dry_run.as_mut().unwrap().what_happened = Some("x".repeat(401)))),
            ("rounds", Box::new(|p: &mut Pocket| p.rounds = vec![serde_json::json!({"label": "Round 1"})])),
            ("a join", Box::new(|p: &mut Pocket| p.join = Some(serde_json::json!({"mode": "wifi"})))),
            ("results", Box::new(|p: &mut Pocket| p.results = Some(serde_json::json!({"teams": []})))),
            ("a draft id with a slash", Box::new(|p: &mut Pocket| p.draft = Some("qd/../x".into()))),
            ("an empty draft id", Box::new(|p: &mut Pocket| p.draft = Some(String::new()))),
            ("too many of yours", Box::new(|p: &mut Pocket| p.yours = Some(10_001))),
        ];
        for (what, change) in bad {
            let mut p = sample();
            change(&mut p);
            assert!(check(p).is_err(), "{what} passed");
        }
    }

    #[test]
    fn a_stopped_card_with_a_failed_dry_run_passes() {
        let mut p = sample();
        p.state = "dry_run_stopped".into();
        p.dry_run = Some(PocketDryRun {
            players: 30,
            passed: false,
            stopped_at: None,
            what_happened: Some("Q3 has no answer".into()),
            what_im_doing: Some("Fixing it.".into()),
        });
        assert_eq!(check(p), Ok(()));
    }

    #[test]
    fn names_and_lines_fit_the_clients_length() {
        // An emoji is two UTF-16 units: 80 of them would be 160 to the client
        let name = fit(&"🎉".repeat(80), 80);
        assert!(utf16_len(&name) <= 80, "{}", utf16_len(&name));
        assert_eq!(fit("<b>Quiz</b>", 80), "\u{2039}b\u{203a}Quiz\u{2039}/b\u{203a}");
    }

    fn named(calls: Vec<serde_json::Value>) -> Option<String> {
        let input = NewMessage {
            channel_id: None,
            direct_kwant_id: Some("k1".into()),
            sender_role: "kwant".into(),
            sender_id: "Quizmaster".into(),
            text: "Done.".into(),
            reasoning_content: None,
            tool_calls: Some(serde_json::Value::Array(calls)),
            tokens_used: 0,
        };
        draft_named(&input).map(|n| match n {
            Named::Id(id) => id,
            Named::MadeThisTurn => "<made this turn>".into(),
        })
    }

    fn call(tool: &str, args: serde_json::Value, status: &str, result: &str) -> serde_json::Value {
        serde_json::json!({"server": SELF_SERVER_NAME, "tool": tool, "arguments": args, "status": status, "result": result})
    }

    #[test]
    fn the_draft_comes_from_the_last_call_that_answered_with_a_card() {
        let card = "Pocket app · Quiz [qd_0123456789ab] \"Friday\" · Writing\ncard: {}";
        let id = Some("qd_0123456789ab".to_string());
        assert_eq!(named(vec![call("pocket_create", serde_json::json!({"spec": {}}), "ok", card)]), id);
        // A draft still being written answers with its card as an error
        assert_eq!(named(vec![call("pocket_create", serde_json::json!({"spec": {}}), "error", card)]), id);
        assert_eq!(named(vec![call("pocket_update", serde_json::json!({"id": "qd_0123456789ab"}), "error", card)]), id);
        // Any other failure, or a card naming another draft, doesn't count; the call before it does
        let failed = call("pocket_update", serde_json::json!({"id": "qd_0123456789ab"}), "error", "Couldn't save the quiz.");
        assert_eq!(named(vec![failed.clone()]), None);
        assert_eq!(named(vec![call("pocket_update", serde_json::json!({"id": "qd_other000001"}), "error", card)]), None);
        let earlier = call("pocket_open", serde_json::json!({"id": "qd_aaaaaaaaaaaa"}), "ok", "");
        assert_eq!(named(vec![earlier, failed]), Some("qd_aaaaaaaaaaaa".into()));
        // Not a Quizmaster tool, not kwant's own server, denied, a create without a card, a bad id
        assert_eq!(named(vec![call("present_result", serde_json::json!({"id": "qd_0123456789ab"}), "ok", "")]), None);
        let mut foreign = call("pocket_update", serde_json::json!({"id": "qd_0123456789ab"}), "ok", "");
        foreign["server"] = serde_json::json!("someones_mcp");
        assert_eq!(named(vec![foreign]), None);
        assert_eq!(named(vec![call("pocket_update", serde_json::json!({"id": "qd_0123456789ab"}), "denied", "")]), None);
        assert_eq!(named(vec![call("pocket_create", serde_json::json!({}), "ok", "Template: ...")]), None);
        assert_eq!(named(vec![call("pocket_update", serde_json::json!({"id": "qd/../x"}), "ok", "")]), None);
        assert_eq!(named(vec![]), None);
    }

    /// quiz-card-and-results: the Claude CLI engine records kwnt_tools calls as "allowed", never
    /// with their answer.
    #[test]
    fn a_claude_engine_call_names_its_draft_without_an_answer() {
        let allowed = |tool: &str, args: serde_json::Value| serde_json::json!({"server": SELF_SERVER_NAME, "tool": tool, "arguments": args, "status": "allowed"});
        let made = Some("<made this turn>".to_string());
        assert_eq!(named(vec![allowed("pocket_create", serde_json::json!({"spec": {"version": 1}}))]), made);
        assert_eq!(named(vec![allowed("pocket_dry_run", serde_json::json!({"id": "qd_0123456789ab"}))]), Some("qd_0123456789ab".into()));
        // The last call wins: a create then a dry run names the dry run's draft
        let both = vec![allowed("pocket_create", serde_json::json!({"spec": {}})), allowed("pocket_dry_run", serde_json::json!({"id": "qd_0123456789ab"}))];
        assert_eq!(named(both), Some("qd_0123456789ab".into()));
        // The template request (no spec, or a null one) made nothing; a bad id is still refused
        assert_eq!(named(vec![allowed("pocket_create", serde_json::json!({}))]), None);
        assert_eq!(named(vec![allowed("pocket_create", serde_json::json!({"spec": null}))]), None);
        assert_eq!(named(vec![allowed("pocket_update", serde_json::json!({"id": "qd/../x"}))]), None);
        // An "allowed" create that did keep its answer reads the id from it, as "ok" does
        let card = "Pocket app · Quiz [qd_0123456789ab] \"Friday\" · Ready\ncard: {}";
        assert_eq!(named(vec![call("pocket_create", serde_json::json!({"spec": {}}), "allowed", card)]), Some("qd_0123456789ab".into()));
        // Another server's tool of the same name doesn't count
        let mut foreign = allowed("pocket_create", serde_json::json!({"spec": {}}));
        foreign["server"] = serde_json::json!("someones_mcp");
        assert_eq!(named(vec![foreign]), None);
    }

    /// What `schedule::ended_block` stores in the results message's `meta.block`.
    fn ended_meta(code: &str, kit: &str) -> serde_json::Value {
        serde_json::json!({
            "kind": "pocket", "reason": "results", "code": code, "kwant_id": "k1",
            "block": {
                "id": format!("pocket-{code}-ended"), "type": "pocket", "title": null,
                "source": {"kind": "slot"}, "provenance": {"kind": "kwant_data", "ref": format!("pocket:{code}")},
                "code": code, "kit": kit, "name": "Friday Pub Quiz", "when": null, "state": "ended",
                "progress": null, "rounds": [], "checks": null, "dry_run": null, "eta": null, "opens_line": null,
                "live": null, "join": null,
                "ended": {"at": "9:41 PM", "winner": {"name": "Quizzly Bears", "points": 31}, "stats": null, "deletes_on": "Fri, Dec 25"},
                "budget": null,
                "results": {
                    "teams": [
                        {"name": "Quizzly Bears", "players": 4, "points": 31, "palette": "yellow", "members": ["Mia", "Tom"]},
                        {"name": "Les Quizérables", "players": 3, "points": 27.5, "palette": "teal", "members": []}
                    ],
                    "missed": [{"label": "R2 Q4", "question": "Capital of Australia?", "answer": "Canberra", "pct_right": 22}],
                    "margin": 3, "near_misses": 2, "near_misses_accepted": 1
                }
            }
        })
    }

    fn results_of(meta: serde_json::Value) -> Option<Block> {
        from_meta(&MessageExtra { meta: Some(meta), ..Default::default() })
    }

    /// quiz-card-and-results (Mira's ux-pass-0952 fix #2): the results message gets the card.
    #[test]
    fn a_results_message_gets_the_ended_card_from_its_meta() {
        let Some(Block::Pocket(p)) = results_of(ended_meta("PQ7XK2", "quiz")) else { panic!("no card") };
        assert_eq!((p.id.as_str(), p.state.as_str(), p.code.as_str(), p.kit.as_str()), ("b0", "ended", "PQ7XK2", "quiz"));
        assert_eq!(p.provenance, Provenance::kwant_data("pocket:PQ7XK2".into()));
        assert_eq!(p.results.as_ref().unwrap()["teams"][1]["name"], "Les Quizérables");
        assert_eq!(check(*p), Ok(()));
        // A reminder (no block), another reason, another kit, a block for another code, a card not
        // ended, a ref to another pocket, a bad code: no card
        let mut reminder = ended_meta("PQ7XK2", "quiz");
        (reminder["reason"], reminder["block"]) = (serde_json::json!("reminder"), serde_json::Value::Null);
        assert!(results_of(reminder).is_none());
        let mut other_reason = ended_meta("PQ7XK2", "quiz");
        other_reason["reason"] = serde_json::json!("deleted");
        assert!(results_of(other_reason).is_none());
        for kit in ["santa", "wall", "gather", "desk", "Quiz"] {
            assert!(results_of(ended_meta("PQ7XK2", kit)).is_none(), "{kit}");
        }
        let mut other_code = ended_meta("PQ7XK2", "quiz");
        other_code["code"] = serde_json::json!("ZZ9ZZ9");
        assert!(results_of(other_code).is_none());
        let mut live = ended_meta("PQ7XK2", "quiz");
        live["block"]["state"] = serde_json::json!("live");
        assert!(results_of(live).is_none());
        let mut other_ref = ended_meta("PQ7XK2", "quiz");
        other_ref["block"]["provenance"]["ref"] = serde_json::json!("pocket:ZZ9ZZ9");
        assert!(results_of(other_ref).is_none());
        assert!(results_of(ended_meta("P Q", "quiz")).is_none());
        let mut junk = ended_meta("PQ7XK2", "quiz");
        junk["block"] = serde_json::json!("not a block");
        assert!(results_of(junk).is_none());
        assert!(results_of(serde_json::json!(null)).is_none());
    }

    /// A results message stored before the card learned its times still reads in words.
    #[test]
    fn raw_end_times_from_older_results_read_as_words() {
        let mut meta = ended_meta("PQ7XK2", "quiz");
        meta["block"]["ended"]["at"] = serde_json::json!("2026-11-27T21:12:40.123456789Z");
        meta["block"]["ended"]["deletes_on"] = serde_json::json!("2026-12-04T21:12:40+00:00");
        let Some(Block::Pocket(p)) = results_of(meta) else { panic!("no card") };
        let e = p.ended.as_ref().unwrap();
        assert_eq!((e["at"].as_str(), e["deletes_on"].as_str()), (Some("9:12 PM UTC"), Some("Fri, Dec 4 (UTC)")));
        assert_eq!(check(*p), Ok(()));
        // Words are kept as they are
        let Some(Block::Pocket(p)) = results_of(ended_meta("PQ7XK2", "quiz")) else { panic!("no card") };
        assert_eq!((p.ended.as_ref().unwrap()["at"].as_str(), p.ended.as_ref().unwrap()["deletes_on"].as_str()), (Some("9:41 PM"), Some("Fri, Dec 25")));
    }

    /// Guest-typed team and player names are cleaned and bounded, so the card still passes.
    #[test]
    fn guest_names_on_the_results_card_are_cleaned_and_bounded() {
        let mut meta = ended_meta("PQ7XK2", "quiz");
        meta["block"]["results"]["teams"][0]["name"] = serde_json::json!(format!("<img src=x>{}", "🎉".repeat(40)));
        meta["block"]["results"]["teams"][0]["members"] = serde_json::json!(["<b>Mia</b>", "x".repeat(60)]);
        meta["block"]["ended"]["winner"]["name"] = serde_json::json!("<script>".repeat(10));
        meta["block"]["name"] = serde_json::json!("y".repeat(200));
        let Some(Block::Pocket(p)) = results_of(meta) else { panic!("no card") };
        let r = p.results.as_ref().unwrap();
        let name = r["teams"][0]["name"].as_str().unwrap();
        assert!(!name.contains('<') && utf16_len(name) <= 40, "{name}");
        assert!(r["teams"][0]["members"].as_array().unwrap().iter().all(|m| !m.as_str().unwrap().contains('<') && utf16_len(m.as_str().unwrap()) <= 24));
        assert!(!p.ended.as_ref().unwrap()["winner"]["name"].as_str().unwrap().contains('<'));
        assert!(utf16_len(&p.name) <= 80);
        assert_eq!(check(*p), Ok(()));
    }

    /// Rigel's QA F1: a team of 13+ players (a class quiz, a big pub team) still gets its card. The
    /// card names the first 12 and keeps the real count; a name that cleans to nothing is left out.
    #[test]
    fn a_big_team_still_gets_its_results_card() {
        let mut meta = ended_meta("PQ7XK2", "quiz");
        let mut names: Vec<String> = (1..=13).map(|i| format!("Kid {i}")).collect();
        names.insert(1, " \u{7} ".into());
        meta["block"]["results"]["teams"][0]["members"] = serde_json::json!(names);
        meta["block"]["results"]["teams"][0]["players"] = serde_json::json!(13);
        let Some(Block::Pocket(p)) = results_of(meta) else { panic!("no card") };
        let team = &p.results.as_ref().unwrap()["teams"][0];
        let members: Vec<&str> = team["members"].as_array().unwrap().iter().map(|m| m.as_str().unwrap()).collect();
        assert_eq!(members, (1..=12).map(|i| format!("Kid {i}")).collect::<Vec<_>>());
        assert_eq!(team["players"], serde_json::json!(13));
        assert_eq!(check(*p), Ok(()));
    }

    /// The server checks an ended card's `ended` and `results` the way the chat's `validPocket` does.
    #[test]
    fn each_results_rule_refuses_its_block() {
        let ended = || match results_of(ended_meta("PQ7XK2", "quiz")) {
            Some(Block::Pocket(p)) => *p,
            _ => panic!("no card"),
        };
        assert_eq!(check(ended()), Ok(()));
        let mut no_winner = ended();
        no_winner.ended.as_mut().unwrap()["winner"] = serde_json::Value::Null;
        assert_eq!(check(no_winner), Ok(()), "a winner can be null");
        let set = |path: &[&str], v: serde_json::Value| -> Change {
            let path: Vec<String> = path.iter().map(|s| s.to_string()).collect();
            Box::new(move |p: &mut Pocket| {
                let (root, rest) = path.split_first().unwrap();
                let mut at = if root == "ended" { p.ended.as_mut().unwrap() } else { p.results.as_mut().unwrap() };
                for k in rest {
                    at = match k.parse::<usize>() {
                        Ok(i) => &mut at[i],
                        Err(_) => &mut at[k.as_str()],
                    };
                }
                *at = v.clone();
            })
        };
        let bad: Vec<(&str, Change)> = vec![
            ("results before the end", Box::new(|p: &mut Pocket| (p.state, p.ended) = ("writing".into(), None))),
            ("ended before the end", Box::new(|p: &mut Pocket| (p.state, p.results) = ("writing".into(), None))),
            ("ended not an object", Box::new(|p: &mut Pocket| p.ended = Some(serde_json::json!("done")))),
            ("results not an object", Box::new(|p: &mut Pocket| p.results = Some(serde_json::json!([])))),
            ("a long winner", set(&["ended", "winner", "name"], serde_json::json!("x".repeat(41)))),
            ("HTML in the winner", set(&["ended", "winner", "name"], serde_json::json!("<b>A</b>"))),
            ("winner points not a number", set(&["ended", "winner", "points"], serde_json::json!("31"))),
            ("a long ended time", set(&["ended", "at"], serde_json::json!("x".repeat(41)))),
            ("long stats", set(&["ended", "stats"], serde_json::json!("x".repeat(201)))),
            ("no teams", set(&["results", "teams"], serde_json::json!([]))),
            ("13 teams", set(&["results", "teams"], serde_json::json!(vec![serde_json::json!({"name": "A", "players": 1, "points": 1, "palette": null, "members": []}); 13]))),
            ("a blank team", set(&["results", "teams", "0", "name"], serde_json::json!(" "))),
            ("a long team", set(&["results", "teams", "0", "name"], serde_json::json!("x".repeat(41)))),
            ("negative players", set(&["results", "teams", "0", "players"], serde_json::json!(-1))),
            ("points out of range", set(&["results", "teams", "0", "points"], serde_json::json!(1e8))),
            ("an unknown palette", set(&["results", "teams", "0", "palette"], serde_json::json!("url(x)"))),
            ("13 members", set(&["results", "teams", "0", "members"], serde_json::json!(vec!["A"; 13]))),
            ("a long member", set(&["results", "teams", "0", "members", "0"], serde_json::json!("x".repeat(25)))),
            ("4 missed", set(&["results", "missed"], serde_json::json!(vec![serde_json::json!({"label": "Q", "question": "Q?", "answer": "A", "pct_right": 1}); 4]))),
            ("a long missed label", set(&["results", "missed", "0", "label"], serde_json::json!("x".repeat(13)))),
            ("a long missed question", set(&["results", "missed", "0", "question"], serde_json::json!("x".repeat(201)))),
            ("a long missed answer", set(&["results", "missed", "0", "answer"], serde_json::json!("x".repeat(81)))),
            ("over 100 percent", set(&["results", "missed", "0", "pct_right"], serde_json::json!(101))),
            ("a fractional margin", set(&["results", "margin"], serde_json::json!(1.5))),
            ("more accepted than near", set(&["results", "near_misses_accepted"], serde_json::json!(3))),
        ];
        for (what, change) in bad {
            let mut p = ended();
            change(&mut p);
            assert!(check(p).is_err(), "{what} passed");
        }
    }

    fn draft_with(spec: serde_json::Value) -> QuizDraftRow {
        QuizDraftRow {
            id: "qd_0123456789ab".into(),
            space_id: "s1".into(),
            kwant_id: "k1".into(),
            title: "Year 8 Science".into(),
            spec: spec.to_string(),
            state: "ready".into(),
            problems: "[]".into(),
            sources: "[]".into(),
            dry_run: None,
            budget: "{}".into(),
            pocket_code: None,
            created_at: 1,
            updated_at: 1,
            own_keys: "[]".into(),
        }
    }

    /// pa11 qa F6: the card gets the class line, and "no quick link" over 60, from the draft's spec.
    #[test]
    fn a_class_quiz_card_carries_its_notice_and_no_quick_link_over_60() {
        let line = crate::pocket::quiz_kit::CLASS_NOTICE;
        let no_quick = serde_json::json!({ "mode": "anywhere", "wifi_url": null, "anywhere_url": null, "quick_tunnel": false });
        let big = draft_with(serde_json::json!({ "settings": { "class": true, "max_players": 70 } }));
        for state in ["ready", "live"] {
            let (notice, join) = class_parts(&big, state);
            assert_eq!((notice.as_deref(), join.as_ref()), (Some(line), Some(&no_quick)), "{state}");
            let mut p = sample();
            (p.state, p.notice, p.join) = (state.into(), notice, join);
            assert_eq!(check(p.clone()), Ok(()), "{state}");
            let v = serde_json::to_value(Block::Pocket(Box::new(p))).unwrap();
            assert_eq!((v["notice"].as_str(), &v["join"]), (Some(line), &no_quick));
        }
        // Before the dry run the card has no join box: the notice only
        assert_eq!(class_parts(&big, "writing"), (Some(line.to_string()), None));
        // Ended: the deletion line says it; nothing more
        assert_eq!(class_parts(&big, "ended"), (None, None));
        // A class quiz of 60 or fewer keeps the quick link
        let small = draft_with(serde_json::json!({ "settings": { "class": true, "max_players": 60 } }));
        assert_eq!(class_parts(&small, "ready"), (Some(line.to_string()), None));
        // A normal quiz: nothing, and no `notice` key on the wire
        for spec in [serde_json::json!({}), serde_json::json!({ "settings": { "max_players": 70 } }), serde_json::json!("not a spec")] {
            assert_eq!(class_parts(&draft_with(spec.clone()), "ready"), (None, None), "{spec}");
        }
        assert_eq!(class_parts(&QuizDraftRow { spec: "{".into(), ..big.clone() }, "ready"), (None, None), "a spec that doesn't parse");
        let v = serde_json::to_value(Block::Pocket(Box::new(sample()))).unwrap();
        assert!(v.get("notice").is_none(), "{v}");
    }

    #[test]
    fn the_validator_takes_only_kwants_own_join_and_a_bounded_notice() {
        let no_quick = serde_json::json!({ "mode": "anywhere", "wifi_url": null, "anywhere_url": null, "quick_tunnel": false });
        let with = |state: &str, join: Option<serde_json::Value>, notice: Option<String>| {
            let mut p = sample();
            (p.state, p.join, p.notice) = (state.into(), join, notice);
            if state == "live" || state == "ready" {
                p.dry_run = Some(PocketDryRun { players: 30, passed: true, stopped_at: None, what_happened: None, what_im_doing: None });
            }
            check(p)
        };
        assert_eq!(with("live", Some(no_quick.clone()), None), Ok(()));
        let url = serde_json::json!({ "mode": "wifi", "wifi_url": "http://x", "anywhere_url": null, "quick_tunnel": false });
        let quick = serde_json::json!({ "mode": "anywhere", "wifi_url": null, "anywhere_url": null, "quick_tunnel": true });
        for bad in [url, quick] {
            assert!(with("ready", Some(bad), None).is_err());
        }
        assert!(with("writing", Some(no_quick), None).is_err(), "no join box before the dry run");
        assert_eq!(with("ready", None, Some("a".repeat(NOTICE_MAX))), Ok(()));
        assert!(with("ready", None, Some("a".repeat(NOTICE_MAX + 1))).is_err());
        assert!(with("ready", None, Some("  ".into())).is_err());
    }

    #[test]
    fn ids_and_codes_have_their_shape() {
        assert!(valid_id("qd_0123456789ab"));
        assert!(!valid_id("") && !valid_id("qd 1") && !valid_id(&"a".repeat(65)) && !valid_id("qd/1"));
        assert!(valid_code("PQ7XK2") && !valid_code("PQ") && !valid_code(&"a".repeat(33)));
    }
}
