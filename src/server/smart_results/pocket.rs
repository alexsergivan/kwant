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
    let id = draft_named(input)?;
    let on = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?").bind(qt::SETTING).fetch_optional(&mut *conn).await.ok()?;
    if on.as_deref() != Some("on") {
        return None;
    }
    let row = sqlx::query_as::<_, QuizDraftRow>("SELECT * FROM quiz_drafts WHERE id = ?").bind(id.as_str()).fetch_optional(&mut *conn).await.ok()??;
    if !belongs(&mut *conn, &row, input, extra).await {
        return None;
    }
    let tz = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
        .bind(crate::server::bot_actions::TIMEZONE_SETTING)
        .fetch_optional(&mut *conn)
        .await
        .ok()
        .flatten();
    Some(Block::Pocket(block(&row, &qt::card_of(&row), tz.as_deref())))
}

/// The draft id the turn's last Quizmaster call that answered with a card named: `arguments.id`,
/// or for `pocket_create` (which has none) the id its card text starts with. A call that worked is
/// "ok"; a draft still being written comes back as an "error" whose answer is that draft's own card
/// (so the bot fixes it), and counts too. Any other failure doesn't. Shape-checked only; whose draft
/// it is gets checked against the stored row.
fn draft_named(input: &NewMessage) -> Option<String> {
    let calls = input.tool_calls.as_ref()?.as_array()?;
    calls.iter().rev().find_map(|c| {
        if c.get("server").and_then(Value::as_str) != Some(SELF_SERVER_NAME) {
            return None;
        }
        let tool = c.get("tool").and_then(Value::as_str).filter(|t| qt::TOOLS.contains(t))?;
        let card_id = c.get("result").and_then(Value::as_str).and_then(card_text_id);
        let id = match c.get("arguments").and_then(|a| a.get("id")).and_then(Value::as_str) {
            Some(id) => id.trim().to_string(),
            None if tool == "pocket_create" => card_id.clone()?,
            None => return None,
        };
        let answered = match c.get("status").and_then(Value::as_str)? {
            "ok" => true,
            "error" => card_id.as_deref() == Some(id.as_str()),
            _ => false,
        };
        (answered && valid_id(&id)).then_some(id)
    })
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
        join: None,
        ended: None,
        budget: None,
        results: None,
        draft: Some(card.id.clone()),
        yours: Some(card.yours as u64),
    }
}

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
        }
    }

    type Change = Box<dyn Fn(&mut Pocket)>;

    fn check(p: Pocket) -> Result<(), String> {
        validate(&ResultSet { version: VERSION, blocks: vec![Block::Pocket(p)] }, None)
    }

    #[test]
    fn a_ready_card_passes_and_serializes_draft_and_yours() {
        let p = sample();
        assert_eq!(check(p.clone()), Ok(()));
        let v = serde_json::to_value(Block::Pocket(p)).unwrap();
        assert_eq!(v["type"], "pocket");
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
        draft_named(&input)
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

    #[test]
    fn ids_and_codes_have_their_shape() {
        assert!(valid_id("qd_0123456789ab"));
        assert!(!valid_id("") && !valid_id("qd 1") && !valid_id(&"a".repeat(65)) && !valid_id("qd/1"));
        assert!(valid_code("PQ7XK2") && !valid_code("PQ") && !valid_code(&"a".repeat(33)));
    }
}
