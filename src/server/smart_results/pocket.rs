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
use crate::db::pocket::PocketRow;
use crate::pocket::kits::Player;
use crate::server::{desk_tool, queue_tool, santa_tool};
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
    // The turn's last call that names a card wins: a Secret Santa made after a quiz call shows
    // the Santa, and a quiz call after `santa_create` shows the quiz.
    let called = last_named(input)?;
    // pocket-switch-santa-alone: the card's own kit must be on (`on`, or a list naming it)
    let on = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?").bind(qt::SETTING).fetch_optional(&mut *conn).await.ok()?;
    let switch = crate::pocket::Switch::parse(on.as_deref().unwrap_or(""));
    let kit = match &called {
        Called::Quiz(_) => crate::kits::quiz::KIT_ID,
        Called::Santa(_) => crate::pocket::santa::ID,
        Called::Desk(_) => crate::pocket::desk::ID,
        Called::Queue(_) => crate::pocket::queue::ID,
    };
    if !switch.allows(kit) {
        return None;
    }
    let tz = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
        .bind(crate::server::bot_actions::TIMEZONE_SETTING)
        .fetch_optional(&mut *conn)
        .await
        .ok()
        .flatten();
    let named = match called {
        Called::Quiz(named) => named,
        Called::Santa(Named::Id(code)) => return santa_candidate(&mut *conn, &code, input, extra, tz.as_deref()).await,
        Called::Santa(Named::MadeThisTurn) => {
            let code = santa_made_this_turn(&mut *conn, input, extra).await?;
            return santa_candidate(&mut *conn, &code, input, extra, tz.as_deref()).await;
        }
        Called::Desk(Named::Id(code)) => return desk_candidate(&mut *conn, &code, input, extra).await,
        Called::Desk(Named::MadeThisTurn) => {
            let code = kit_made_this_turn(&mut *conn, crate::pocket::desk::ID, input, extra).await?;
            return desk_candidate(&mut *conn, &code, input, extra).await;
        }
        Called::Queue(Named::Id(code)) => return queue_candidate(&mut *conn, &code, input, extra).await,
        Called::Queue(Named::MadeThisTurn) => {
            let code = kit_made_this_turn(&mut *conn, crate::pocket::queue::ID, input, extra).await?;
            return queue_candidate(&mut *conn, &code, input, extra).await;
        }
    };
    let row = match named {
        Named::Id(id) => sqlx::query_as::<_, QuizDraftRow>("SELECT * FROM quiz_drafts WHERE id = ?").bind(id.as_str()).fetch_optional(&mut *conn).await.ok()??,
        Named::MadeThisTurn => made_this_turn(&mut *conn, input, extra).await?,
    };
    if !belongs(&mut *conn, &row.kwant_id, &row.space_id, input, extra).await {
        return None;
    }
    Some(Block::Pocket(Box::new(block(&row, &qt::card_of(&row), tz.as_deref()))))
}

/// What the turn's last card-naming call named.
#[derive(Debug, PartialEq)]
enum Called {
    /// A Quizmaster draft.
    Quiz(Named),
    /// The Secret Santa `santa_create` made: its code ([`Named::Id`]), or the one this bot made
    /// this turn when the call's answer wasn't kept ([`Named::MadeThisTurn`]).
    Santa(Named),
    /// The front desk `desk_create` made (desk-create-from-chat), named the same two ways.
    Desk(Named),
    /// The take-a-number queue `queue_create` made (queue-card-in-chat), named the same two ways.
    Queue(Named),
}

/// The turn's last call that names a card, of any kind.
fn last_named(input: &NewMessage) -> Option<Called> {
    let quiz = draft_named_at(input).map(|(i, n)| (i, Called::Quiz(n)));
    let santa = santa_named_at(input).map(|(i, n)| (i, Called::Santa(n)));
    let desk = desk_named_at(input).map(|(i, n)| (i, Called::Desk(n)));
    let queue = queue_named_at(input).map(|(i, n)| (i, Called::Queue(n)));
    // No two calls share a place in the turn
    [quiz, santa, desk, queue].into_iter().flatten().max_by_key(|(i, _)| *i).map(|(_, c)| c)
}

/// The Secret Santa the turn's last `santa_create` call that made one names, and that call's place
/// in the turn: the code its answer starts with (`Pocket app · Secret Santa [<code>]`), or, for a
/// call the Claude CLI engine let through ("allowed") with a spec and no answer kept,
/// [`Named::MadeThisTurn`] (santa-create-claude-engine-card, as #1230 did for the quiz). A failed
/// or denied call, a call that only asked for the rules (no spec, or a null one), or another
/// server's tool of the same name names nothing. Shape-checked only; whose pocket it is gets
/// checked against the stored row.
fn santa_named_at(input: &NewMessage) -> Option<(usize, Named)> {
    let calls = input.tool_calls.as_ref()?.as_array()?;
    calls.iter().enumerate().rev().find_map(|(i, c)| {
        if c.get("server").and_then(Value::as_str) != Some(SELF_SERVER_NAME) || c.get("tool").and_then(Value::as_str) != Some(santa_tool::TOOL) {
            return None;
        }
        let status = c.get("status").and_then(Value::as_str);
        if !matches!(status, Some("ok" | "allowed")) {
            return None;
        }
        let Some(result) = c.get("result").and_then(Value::as_str) else {
            // A create with a spec that ran, with no answer kept: the Santa it made
            let spec = c.get("arguments").and_then(|a| a.get("spec")).filter(|s| !s.is_null());
            return (status == Some("allowed") && spec.is_some()).then_some((i, Named::MadeThisTurn));
        };
        let rest = result.trim_start().strip_prefix(santa_tool::CARD_TEXT_HEAD)?;
        let code = &rest[..rest.find(']')?];
        valid_code(code).then(|| (i, Named::Id(code.to_string())))
    })
}

/// The code of the Secret Santa the reply's bot made since the turn began: its newest pocket of
/// kit `santa` created no earlier than the chat's newest message from anyone but this bot (the
/// same anchor as [`made_this_turn`]). `None` without such a message or pocket. Whose pocket it
/// is still goes through [`belongs`] in [`santa_candidate`].
async fn santa_made_this_turn(conn: &mut SqliteConnection, input: &NewMessage, extra: &MessageExtra) -> Option<String> {
    kit_made_this_turn(conn, crate::pocket::santa::ID, input, extra).await
}

/// [`santa_made_this_turn`] for any kit: the newest pocket of `kit` the reply's bot made since
/// the turn began.
async fn kit_made_this_turn(conn: &mut SqliteConnection, kit: &str, input: &NewMessage, extra: &MessageExtra) -> Option<String> {
    let (kwant, asked) = turn_began(&mut *conn, input, extra).await?;
    let sql = format!("SELECT {} FROM pocket_apps WHERE kit = ? AND owner_kwant_id = ? ORDER BY created_at DESC, rowid DESC LIMIT 1", crate::db::pocket::POCKET_COLS);
    let row = sqlx::query_as::<_, PocketRow>(&sql).bind(kit).bind(&kwant).fetch_optional(&mut *conn).await.ok()??;
    // Message times keep whole seconds; pockets keep fractions
    (row.created_at.timestamp_millis() >= asked.timestamp() * 1000).then_some(row.code)
}

/// A Secret Santa's card: the stored pocket (kit `santa`, open or ended, owned by the reply's
/// sender, in the reply's space) through [`santa_tool::card`], never the call's text.
async fn santa_candidate(conn: &mut SqliteConnection, code: &str, input: &NewMessage, extra: &MessageExtra, tz: Option<&str>) -> Option<Block> {
    let sql = format!("SELECT {} FROM pocket_apps WHERE code = ?", crate::db::pocket::POCKET_COLS);
    let row = sqlx::query_as::<_, PocketRow>(&sql).bind(code).fetch_optional(&mut *conn).await.ok()??;
    if row.kit != crate::pocket::santa::ID {
        return None;
    }
    let owner = row.owner_kwant_id.clone()?;
    let space = sqlx::query_scalar::<_, String>("SELECT space_id FROM kwants WHERE id = ?").bind(&owner).fetch_optional(&mut *conn).await.ok()??;
    if !belongs(&mut *conn, &owner, &space, input, extra).await {
        return None;
    }
    let players: Vec<Player> = sqlx::query_as::<_, (String, String, String, Option<String>, DateTime<Utc>)>(
        "SELECT id, name, avatar, team, joined_at FROM pocket_players WHERE pocket_code = ? AND removed_at IS NULL ORDER BY joined_at, id",
    )
    .bind(code)
    .fetch_all(&mut *conn)
    .await
    .ok()?
    .into_iter()
    .map(|(id, name, avatar, team, joined_at)| Player { id, name, avatar, team, joined_at })
    .collect();
    let spec: Value = serde_json::from_str(&row.spec).ok()?;
    let kit_state: Value = serde_json::from_str(&row.kit_state).unwrap_or(Value::Null);
    let c = santa_tool::card(&santa_tool::CardInput {
        title: &row.title,
        spec: &spec,
        kit_state: &kit_state,
        players: &players,
        ended: row.state != crate::db::pocket::OPEN,
        ended_at: row.ended_at,
        expires_at: row.expires_at,
        tz,
        now: Utc::now(),
        join: santa_tool::join_for(code),
    });
    Some(Block::Pocket(Box::new(santa_block(&row.code, c))))
}

/// The `pocket` block for a Secret Santa's card: kit `santa`, its `santa` part, and none of the
/// quiz's parts.
fn santa_block(code: &str, c: santa_tool::Card) -> Pocket {
    Pocket {
        id: "b0".into(),
        title: None,
        source: Source::slot(),
        provenance: Provenance::kwant_data(format!("pocket:{code}")),
        code: code.to_string(),
        kit: crate::pocket::santa::ID.into(),
        name: fit(&c.name, 80),
        when: c.when.map(|w| fit(&w, 160)),
        state: c.state.into(),
        paused: false,
        progress: None,
        rounds: Vec::new(),
        checks: None,
        dry_run: None,
        eta: None,
        opens_line: None,
        live: None,
        join: None,
        ended: None,
        budget: None,
        results: None,
        draft: None,
        yours: None,
        notice: None,
        santa: Some(c.santa),
        desk: None,
        queue: None,
    }
}

/// The front desk the turn's last `desk_create` call that made one names, and that call's place:
/// the code its answer starts with (`Pocket app · Front desk [<code>]`), or, for a call the Claude
/// CLI engine let through ("allowed") holding a desk and with no answer kept,
/// [`Named::MadeThisTurn`] (as #1280 did for the Santa). A failed or denied call, a call that only
/// asked for the rules (no `title`), or another server's tool of the same name names nothing.
fn desk_named_at(input: &NewMessage) -> Option<(usize, Named)> {
    let calls = input.tool_calls.as_ref()?.as_array()?;
    calls.iter().enumerate().rev().find_map(|(i, c)| {
        if c.get("server").and_then(Value::as_str) != Some(SELF_SERVER_NAME) || c.get("tool").and_then(Value::as_str) != Some(desk_tool::TOOL) {
            return None;
        }
        let status = c.get("status").and_then(Value::as_str);
        if !matches!(status, Some("ok" | "allowed")) {
            return None;
        }
        let Some(result) = c.get("result").and_then(Value::as_str) else {
            return (status == Some("allowed") && desk_tool::holds_desk(c.get("arguments"))).then_some((i, Named::MadeThisTurn));
        };
        let rest = result.trim_start().strip_prefix(desk_tool::CARD_TEXT_HEAD)?;
        let code = &rest[..rest.find(']')?];
        valid_code(code).then(|| (i, Named::Id(code.to_string())))
    })
}

/// A front desk's card: the stored pocket (kit `desk`, owned by the reply's sender, in the
/// reply's space) through [`desk_tool::card`], never the call's text.
async fn desk_candidate(conn: &mut SqliteConnection, code: &str, input: &NewMessage, extra: &MessageExtra) -> Option<Block> {
    let sql = format!("SELECT {} FROM pocket_apps WHERE code = ?", crate::db::pocket::POCKET_COLS);
    let row = sqlx::query_as::<_, PocketRow>(&sql).bind(code).fetch_optional(&mut *conn).await.ok()??;
    if row.kit != crate::pocket::desk::ID {
        return None;
    }
    let owner = row.owner_kwant_id.clone()?;
    let space = sqlx::query_scalar::<_, String>("SELECT space_id FROM kwants WHERE id = ?").bind(&owner).fetch_optional(&mut *conn).await.ok()??;
    if !belongs(&mut *conn, &owner, &space, input, extra).await {
        return None;
    }
    let players: Vec<Player> = sqlx::query_as::<_, (String, String, String, Option<String>, DateTime<Utc>)>(
        "SELECT id, name, avatar, team, joined_at FROM pocket_players WHERE pocket_code = ? AND removed_at IS NULL ORDER BY joined_at, id",
    )
    .bind(code)
    .fetch_all(&mut *conn)
    .await
    .ok()?
    .into_iter()
    .map(|(id, name, avatar, team, joined_at)| Player { id, name, avatar, team, joined_at })
    .collect();
    let spec: Value = serde_json::from_str(&row.spec).ok()?;
    let kit_state: Value = serde_json::from_str(&row.kit_state).unwrap_or(Value::Null);
    // The answering bot's model, for the card's cost line (host_api's read does the same)
    let bot = spec["answer_bot"].as_str().unwrap_or("");
    let model = sqlx::query_scalar::<_, String>("SELECT preferred_model FROM kwants WHERE id = ?")
        .bind(bot)
        .fetch_optional(&mut *conn)
        .await
        .ok()
        .flatten()
        .filter(|m| !m.trim().is_empty());
    let ended = row.state != crate::db::pocket::OPEN;
    let d = desk_tool::card(&desk_tool::CardInput {
        spec: &spec,
        kit_state: &kit_state,
        players: &players,
        now: Utc::now(),
        model,
        join: if ended { None } else { santa_tool::join_for(code) },
    });
    Some(Block::Pocket(Box::new(desk_block(&row.code, &row.title, ended, d))))
}

/// The `pocket` block for a front desk's card: kit `desk`, its `desk` part (which carries its own
/// join), and none of the quiz's parts.
fn desk_block(code: &str, title: &str, ended: bool, d: Value) -> Pocket {
    Pocket {
        id: "b0".into(),
        title: None,
        source: Source::slot(),
        provenance: Provenance::kwant_data(format!("pocket:{code}")),
        code: code.to_string(),
        kit: crate::pocket::desk::ID.into(),
        name: fit(title, 80),
        when: None,
        state: if ended { "ended" } else { "live" }.into(),
        paused: false,
        progress: None,
        rounds: Vec::new(),
        checks: None,
        dry_run: None,
        eta: None,
        opens_line: None,
        live: None,
        join: None,
        ended: None,
        budget: None,
        results: None,
        draft: None,
        yours: None,
        notice: None,
        santa: None,
        desk: Some(d),
        queue: None,
    }
}

/// The take-a-number queue the turn's last `queue_create` call that made one names, and that
/// call's place: the code its answer starts with (`Pocket app · Queue [<code>]`), or, for a call
/// the Claude CLI engine let through ("allowed") holding a queue and with no answer kept,
/// [`Named::MadeThisTurn`]. A failed or denied call, a call that only asked for the rules (no
/// `title`), a refusal ("Nothing was made: …"), or another server's tool of the same name names
/// nothing.
fn queue_named_at(input: &NewMessage) -> Option<(usize, Named)> {
    let calls = input.tool_calls.as_ref()?.as_array()?;
    calls.iter().enumerate().rev().find_map(|(i, c)| {
        if c.get("server").and_then(Value::as_str) != Some(SELF_SERVER_NAME) || c.get("tool").and_then(Value::as_str) != Some(queue_tool::TOOL) {
            return None;
        }
        let status = c.get("status").and_then(Value::as_str);
        if !matches!(status, Some("ok" | "allowed")) {
            return None;
        }
        let Some(result) = c.get("result").and_then(Value::as_str) else {
            return (status == Some("allowed") && queue_tool::holds_queue(c.get("arguments"))).then_some((i, Named::MadeThisTurn));
        };
        let rest = result.trim_start().strip_prefix(queue_tool::CARD_TEXT_HEAD)?;
        let code = &rest[..rest.find(']')?];
        valid_code(code).then(|| (i, Named::Id(code.to_string())))
    })
}

/// A take-a-number queue's card: the stored pocket (kit `queue`, owned by the reply's sender, in
/// the reply's space), as `GET /api/pockets/:code` gives it to the card: the kit's host view, the
/// join `queue_create` worked out (while it's fresh, and never once the queue ended) and
/// `ended_at`. Never the call's text.
async fn queue_candidate(conn: &mut SqliteConnection, code: &str, input: &NewMessage, extra: &MessageExtra) -> Option<Block> {
    let sql = format!("SELECT {} FROM pocket_apps WHERE code = ?", crate::db::pocket::POCKET_COLS);
    let row = sqlx::query_as::<_, PocketRow>(&sql).bind(code).fetch_optional(&mut *conn).await.ok()??;
    if row.kit != crate::pocket::queue::ID {
        return None;
    }
    let owner = row.owner_kwant_id.clone()?;
    let space = sqlx::query_scalar::<_, String>("SELECT space_id FROM kwants WHERE id = ?").bind(&owner).fetch_optional(&mut *conn).await.ok()??;
    if !belongs(&mut *conn, &owner, &space, input, extra).await {
        return None;
    }
    let players: Vec<Player> = sqlx::query_as::<_, (String, String, String, Option<String>, DateTime<Utc>)>(
        "SELECT id, name, avatar, team, joined_at FROM pocket_players WHERE pocket_code = ? AND removed_at IS NULL ORDER BY joined_at, id",
    )
    .bind(code)
    .fetch_all(&mut *conn)
    .await
    .ok()?
    .into_iter()
    .map(|(id, name, avatar, team, joined_at)| Player { id, name, avatar, team, joined_at })
    .collect();
    let spec: Value = serde_json::from_str(&row.spec).ok()?;
    let kit_state: Value = serde_json::from_str(&row.kit_state).unwrap_or(Value::Null);
    let ended = row.state != crate::db::pocket::OPEN;
    let join = if ended { None } else { santa_tool::join_for(code) };
    let q = queue_part(&spec, &kit_state, &players, Utc::now(), join, row.ended_at);
    Some(Block::Pocket(Box::new(queue_block(&row.code, &row.title, ended, q))))
}

/// The `queue` part: the kit's host view (what the card's live read gets as `host_view`) with the
/// pocket's own `join` and `ended_at` laid over it, the way the card's `queueFromRead` does.
fn queue_part(spec: &Value, kit_state: &Value, players: &[Player], now: DateTime<Utc>, join: Option<Value>, ended_at: Option<DateTime<Utc>>) -> Value {
    use crate::pocket::kits::{KitCtx, Viewer};
    let kit = crate::pocket::kits::kit(crate::pocket::queue::ID).expect("the queue kit is built in");
    let ctx = KitCtx::new(spec, players, now);
    let mut q = kit.view(&ctx, kit_state, &Viewer::Host);
    if let Some(o) = q.as_object_mut() {
        o.insert("join".into(), join.unwrap_or(Value::Null));
        o.insert("ended_at".into(), serde_json::to_value(ended_at).unwrap_or(Value::Null));
    }
    q
}

/// The `pocket` block for a queue's card: kit `queue`, its `queue` part (which carries its own
/// join), and none of the quiz's parts.
fn queue_block(code: &str, title: &str, ended: bool, q: Value) -> Pocket {
    Pocket {
        id: "b0".into(),
        title: None,
        source: Source::slot(),
        provenance: Provenance::kwant_data(format!("pocket:{code}")),
        code: code.to_string(),
        kit: crate::pocket::queue::ID.into(),
        name: fit(title, 80),
        when: None,
        state: if ended { "ended" } else { "live" }.into(),
        paused: false,
        progress: None,
        rounds: Vec::new(),
        checks: None,
        dry_run: None,
        eta: None,
        opens_line: None,
        live: None,
        join: None,
        ended: None,
        budget: None,
        results: None,
        draft: None,
        yours: None,
        notice: None,
        santa: None,
        desk: None,
        queue: Some(q),
    }
}

/// Which draft a turn's Quizmaster calls name (or, in [`Called::Santa`], which Secret Santa).
#[derive(Debug, PartialEq)]
enum Named {
    /// `arguments.id`, or, when a call has none, the id its card's text starts with (a Santa's
    /// code).
    Id(String),
    /// A `pocket_create` that ran with a spec but whose answer wasn't recorded: the Claude CLI
    /// engine records kwnt_tools calls as "allowed" with no `result` (quiz-card-and-results). The
    /// draft it made is this bot's newest one created since the turn began ([`made_this_turn`]).
    MadeThisTurn,
}

/// The draft the turn's last Quizmaster call that answered with a card named: `arguments.id`, or
/// for a call without one (`pocket_create`, or "open it" with no id, which acts on the bot's only
/// quiz) the id its card text starts with. A call that worked is
/// "ok", or "allowed" on the Claude CLI engine, which records the call when it is let through and
/// never its answer; a draft still being written comes back as an "error" whose answer is that
/// draft's own card (so the bot fixes it), and counts too. Any other failure doesn't. Shape-checked
/// only; whose draft it is gets checked against the stored row.
#[cfg(test)]
fn draft_named(input: &NewMessage) -> Option<Named> {
    draft_named_at(input).map(|(_, named)| named)
}

/// [`draft_named`], with that call's place in the turn.
fn draft_named_at(input: &NewMessage) -> Option<(usize, Named)> {
    let calls = input.tool_calls.as_ref()?.as_array()?;
    calls.iter().enumerate().rev().find_map(|(i, c)| {
        if c.get("server").and_then(Value::as_str) != Some(SELF_SERVER_NAME) {
            return None;
        }
        let tool = c.get("tool").and_then(Value::as_str).filter(|t| qt::TOOLS.contains(t))?;
        let status = c.get("status").and_then(Value::as_str)?;
        let result = c.get("result").and_then(Value::as_str);
        let card_id = result.and_then(card_text_id);
        // A left-out id (absent, null or blank) is the tool's own "your only quiz" (quiz-open-without-id)
        let given = c.get("arguments").and_then(|a| a.get("id")).and_then(Value::as_str).map(str::trim).filter(|id| !id.is_empty());
        let id = match given {
            Some(id) => id.to_string(),
            // A create with a spec that ran, with no answer kept: the draft it made
            None if tool == "pocket_create" && status == "allowed" && result.is_none() => {
                let spec = c.get("arguments").and_then(|a| a.get("spec")).filter(|s| !s.is_null());
                return spec.map(|_| (i, Named::MadeThisTurn));
            }
            // Any other call without an id names the draft its card answered with
            None => card_id.clone()?,
        };
        let answered = match status {
            "ok" | "allowed" => true,
            "error" => card_id.as_deref() == Some(id.as_str()),
            _ => false,
        };
        (answered && valid_id(&id)).then_some((i, Named::Id(id)))
    })
}

/// The reply's bot's newest draft created since the turn began: no earlier than the chat's newest
/// message from anyone but this bot (the ask the turn answers). `None` without such a message or
/// draft. Whose draft it is still goes through [`belongs`].
async fn made_this_turn(conn: &mut SqliteConnection, input: &NewMessage, extra: &MessageExtra) -> Option<QuizDraftRow> {
    let (kwant, asked) = turn_began(&mut *conn, input, extra).await?;
    // Message times keep whole seconds; drafts keep milliseconds
    let since = asked.timestamp() * 1000;
    sqlx::query_as::<_, QuizDraftRow>("SELECT * FROM quiz_drafts WHERE kwant_id = ? AND created_at >= ? ORDER BY created_at DESC, rowid DESC LIMIT 1")
        .bind(kwant)
        .bind(since)
        .fetch_optional(&mut *conn)
        .await
        .ok()?
}

/// The reply's bot and when its turn began: the time of the chat's newest message from anyone
/// but this bot (the ask the turn answers). `None` without such a message.
async fn turn_began(conn: &mut SqliteConnection, input: &NewMessage, extra: &MessageExtra) -> Option<(String, DateTime<Utc>)> {
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
    Some((kwant, asked))
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

/// The draft (or pocket) of `kwant_id` in `space_id` is the reply sender's own, in the reply's space.
async fn belongs(conn: &mut SqliteConnection, kwant_id: &str, space_id: &str, input: &NewMessage, extra: &MessageExtra) -> bool {
    let writer = match sqlx::query_as::<_, (String, String)>("SELECT name, space_id FROM kwants WHERE id = ?").bind(kwant_id).fetch_optional(&mut *conn).await {
        Ok(w) => w,
        Err(_) => return false,
    };
    let Some((name, space)) = writer else { return false };
    if name != input.sender_id || space != space_id {
        return false;
    }
    // A card that names its bot must name this one
    if let Some(k) = extra.meta.as_ref().and_then(|m| m.get("kwant_id")) {
        if k.as_str() != Some(kwant_id) {
            return false;
        }
    }
    match (&input.direct_kwant_id, &input.channel_id) {
        (Some(direct), _) => direct == kwant_id,
        (None, Some(channel)) => {
            let space =
                sqlx::query_scalar::<_, String>("SELECT space_id FROM channels WHERE id = ?").bind(channel).fetch_optional(&mut *conn).await.ok().flatten();
            space.as_deref() == Some(space_id)
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
        santa: None,
        desk: None,
        queue: None,
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
            santa: None,
            desk: None,
            queue: None,
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

    /// quiz-open-card-without-id (Rigel's quiz-open-without-id F1): "open it" with no id acts on
    /// the bot's only quiz and answers with its card, so the reply gets that card too.
    #[test]
    fn a_call_without_an_id_names_the_draft_its_card_answered_with() {
        let card = "Pocket app · Quiz [qd_0123456789ab] \"Friday\" · Live\ncard: {}";
        let id = Some("qd_0123456789ab".to_string());
        for tool in ["pocket_open", "pocket_update", "pocket_dry_run", "pocket_close"] {
            for args in [serde_json::json!({}), serde_json::json!({"id": null}), serde_json::json!({"id": "  "})] {
                assert_eq!(named(vec![call(tool, args.clone(), "ok", card)]), id, "{tool} {args}");
                assert_eq!(named(vec![call(tool, args.clone(), "allowed", card)]), id, "{tool} {args}");
            }
            // No answer, or one that isn't a card (the pick list, "no quiz to open"): no draft
            let bare = serde_json::json!({"server": SELF_SERVER_NAME, "tool": tool, "arguments": {}, "status": "ok"});
            assert_eq!(named(vec![bare]), None, "{tool}");
            assert_eq!(named(vec![call(tool, serde_json::json!({}), "allowed", "")]), None, "{tool}");
            let picks = "You have 2 quizzes you could open, so nothing changed. Call pocket_open again with `id`, one of:\n  qd_0123456789ab · Friday · Ready · updated just now\n";
            assert_eq!(named(vec![call(tool, serde_json::json!({}), "error", picks)]), None, "{tool}");
            assert_eq!(named(vec![call(tool, serde_json::json!({}), "error", "You have no quiz draft to open.")]), None, "{tool}");
            // A failure whose card names another draft than the id given still doesn't count
            assert_eq!(named(vec![call(tool, serde_json::json!({"id": "qd_other000001"}), "error", card)]), None, "{tool}");
            // Denied, another server's tool, or a card with a bad id: none
            assert_eq!(named(vec![call(tool, serde_json::json!({}), "denied", card)]), None, "{tool}");
            let mut foreign = call(tool, serde_json::json!({}), "ok", card);
            foreign["server"] = serde_json::json!("someones_mcp");
            assert_eq!(named(vec![foreign]), None, "{tool}");
            assert_eq!(named(vec![call(tool, serde_json::json!({}), "ok", "Pocket app · Quiz [qd/../x] · Live")]), None, "{tool}");
        }
        // A draft still being written answers with its own card as an error, and counts as before
        let writing = "Pocket app · Quiz [qd_0123456789ab] \"Friday\" · Writing\ncard: {}";
        assert_eq!(named(vec![call("pocket_dry_run", serde_json::json!({}), "error", writing)]), id);
        // The last call that answered wins: an open without an id after an update of another draft
        let update = call("pocket_update", serde_json::json!({"id": "qd_aaaaaaaaaaaa"}), "ok", "");
        assert_eq!(named(vec![update.clone(), call("pocket_open", serde_json::json!({}), "ok", card)]), id);
        assert_eq!(named(vec![update, call("pocket_open", serde_json::json!({}), "error", "You have no quiz draft to open.")]), Some("qd_aaaaaaaaaaaa".into()));
        // The Claude CLI create that ran with no answer kept is still the draft made this turn
        let made = serde_json::json!({"server": SELF_SERVER_NAME, "tool": "pocket_create", "arguments": {"spec": {"version": 1}}, "status": "allowed"});
        assert_eq!(named(vec![made]), Some("<made this turn>".into()));
    }

    fn reply_with(calls: Vec<serde_json::Value>) -> NewMessage {
        NewMessage {
            channel_id: None,
            direct_kwant_id: Some("k1".into()),
            sender_role: "kwant".into(),
            sender_id: "Quizmaster".into(),
            text: "Done.".into(),
            reasoning_content: None,
            tool_calls: Some(serde_json::Value::Array(calls)),
            tokens_used: 0,
        }
    }

    /// santa-create-claude-engine-card: the Claude CLI engine records `santa_create` as "allowed"
    /// with no answer, so a call that made a Santa names the one this bot made this turn.
    #[test]
    fn a_claude_engine_santa_create_names_the_santa_made_this_turn() {
        let santa = |calls| santa_named_at(&reply_with(calls)).map(|(_, n)| n);
        let allowed = |args: serde_json::Value| serde_json::json!({"server": SELF_SERVER_NAME, "tool": santa_tool::TOOL, "arguments": args, "status": "allowed"});
        assert_eq!(santa(vec![allowed(serde_json::json!({"spec": {"title": "Team Secret Santa"}}))]), Some(Named::MadeThisTurn));
        // A spec sent as text still made one
        assert_eq!(santa(vec![allowed(serde_json::json!({"spec": "{\"title\": \"x\"}"}))]), Some(Named::MadeThisTurn));
        // The rules (no spec, or a null one), no arguments at all: nothing made
        assert_eq!(santa(vec![allowed(serde_json::json!({}))]), None);
        assert_eq!(santa(vec![allowed(serde_json::json!({"spec": null}))]), None);
        assert_eq!(santa(vec![serde_json::json!({"server": SELF_SERVER_NAME, "tool": santa_tool::TOOL, "status": "allowed"})]), None);
        // Denied, an error, or "ok" with no answer kept ran nothing this card can name
        for status in ["denied", "error", "ok"] {
            let mut c = allowed(serde_json::json!({"spec": {}}));
            c["status"] = serde_json::json!(status);
            assert_eq!(santa(vec![c]), None, "{status}");
        }
        // Another server's tool of the same name
        let mut foreign = allowed(serde_json::json!({"spec": {}}));
        foreign["server"] = serde_json::json!("someones_mcp");
        assert_eq!(santa(vec![foreign]), None);
        // An answer that was kept is read as before, "ok" or "allowed"; one that isn't a made Santa names nothing
        let made = "Pocket app · Secret Santa [PQ7XK2] \"Team Secret Santa\" · collecting";
        assert_eq!(santa(vec![call(santa_tool::TOOL, serde_json::json!({"spec": {}}), "ok", made)]), Some(Named::Id("PQ7XK2".into())));
        assert_eq!(santa(vec![call(santa_tool::TOOL, serde_json::json!({"spec": {}}), "allowed", made)]), Some(Named::Id("PQ7XK2".into())));
        assert_eq!(santa(vec![call(santa_tool::TOOL, serde_json::json!({"spec": {}}), "allowed", "Nothing was made: gift_on is missing.")]), None);
        assert_eq!(santa(vec![call(santa_tool::TOOL, serde_json::json!({}), "allowed", "")]), None);
    }

    /// queue-card-in-chat: `queue_create`'s answer names its queue by the code after its head; the
    /// Claude CLI engine's "allowed" call with a queue and no answer names the one made this turn.
    #[test]
    fn a_queue_create_names_its_queue_and_only_a_made_one() {
        let queue = |calls| queue_named_at(&reply_with(calls)).map(|(_, n)| n);
        let bakery = serde_json::json!({"title": "Tine's Bakery", "until": "12:00"});
        let made = "Pocket app · Queue [PQ7XK2] \"Tine's Bakery\" is made. It's open now and closes by itself at 12:00.";
        assert_eq!(queue(vec![call(queue_tool::TOOL, bakery.clone(), "ok", made)]), Some(Named::Id("PQ7XK2".into())));
        assert_eq!(queue(vec![call(queue_tool::TOOL, bakery.clone(), "allowed", made)]), Some(Named::Id("PQ7XK2".into())));
        // A refusal or the rules made nothing
        assert_eq!(queue(vec![call(queue_tool::TOOL, bakery.clone(), "ok", "Nothing was made: until must be a time written HH:MM.")]), None);
        assert_eq!(queue(vec![call(queue_tool::TOOL, serde_json::json!({}), "ok", queue_tool::RULES)]), None);
        // The Claude CLI engine: a queue in the arguments made one; the rules, a blank title or a denied call didn't
        let allowed = |args: serde_json::Value| serde_json::json!({"server": SELF_SERVER_NAME, "tool": queue_tool::TOOL, "arguments": args, "status": "allowed"});
        assert_eq!(queue(vec![allowed(bakery.clone())]), Some(Named::MadeThisTurn));
        assert_eq!(queue(vec![allowed(serde_json::json!({"spec": {"title": "Q"}}))]), Some(Named::MadeThisTurn));
        for args in [serde_json::json!({}), serde_json::json!({"spec": null}), serde_json::json!({"title": "  "})] {
            assert_eq!(queue(vec![allowed(args.clone())]), None, "{args}");
        }
        for status in ["denied", "error", "ok"] {
            let mut c = allowed(bakery.clone());
            c["status"] = serde_json::json!(status);
            assert_eq!(queue(vec![c]), None, "{status}");
        }
        // Another server's tool of the same name, or a code of the wrong shape
        let mut foreign = call(queue_tool::TOOL, bakery.clone(), "ok", made);
        foreign["server"] = serde_json::json!("someones_mcp");
        assert_eq!(queue(vec![foreign]), None);
        assert_eq!(queue(vec![call(queue_tool::TOOL, bakery.clone(), "ok", "Pocket app · Queue [a b] made")]), None);
        // The turn's last card call wins: a queue after a desk shows the queue, a desk after it the desk
        let desk = call(desk_tool::TOOL, serde_json::json!({"title": "D"}), "ok", "Pocket app · Front desk [DESK01] \"D\" is open.");
        let q = call(queue_tool::TOOL, bakery.clone(), "ok", made);
        assert_eq!(last_named(&reply_with(vec![desk.clone(), q.clone()])), Some(Called::Queue(Named::Id("PQ7XK2".into()))));
        assert_eq!(last_named(&reply_with(vec![q, desk])), Some(Called::Desk(Named::Id("DESK01".into()))));
    }

    /// The queue part is the kit's host view (what the card's live read gets) with its own join and
    /// `ended_at`, and the block passes the validator; first names show, a guest's id never does.
    #[test]
    fn a_queue_block_carries_the_host_view_and_its_join_and_passes() {
        use crate::pocket::kits::{KitCtx, Viewer};
        let kit = crate::pocket::kits::kit(crate::pocket::queue::ID).unwrap();
        let spec = serde_json::json!({"title": "Tine's Bakery", "line": "Pick up at the side counter", "hours": {"until": "12:00"}, "call_minutes": 3, "ask_name": true});
        let now = Utc::now();
        let players: Vec<Player> = ["g-ana-1", "g-ben-2"].iter().map(|id| Player { id: id.to_string(), name: String::new(), avatar: String::new(), team: None, joined_at: now }).collect();
        let mut st = kit.init(&spec);
        for (g, name) in [("g-ana-1", "Ana"), ("g-ben-2", "Ben")] {
            let mut ctx = KitCtx::new(&spec, &players, now);
            kit.act(&mut ctx, &mut st, &Viewer::Guest(g.into()), "take", &serde_json::json!({"name": name})).unwrap();
        }
        let mut ctx = KitCtx::new(&spec, &players, now);
        kit.act(&mut ctx, &mut st, &Viewer::Host, "call_next", &serde_json::json!({})).unwrap();
        let join = serde_json::json!({"mode": "wifi", "anywhere_kind": null, "anywhere_url": null, "wifi_url": "http://192.168.1.5:8787/j/PQ7XK2", "wifi_note": null});
        let q = queue_part(&spec, &st, &players, now, Some(join.clone()), None);
        let p = queue_block("PQ7XK2", "Tine's Bakery", false, q.clone());
        assert_eq!(check(p.clone()), Ok(()));
        assert_eq!((p.kit.as_str(), p.state.as_str(), p.name.as_str()), ("queue", "live", "Tine's Bakery"));
        assert!(p.join.is_none() && p.desk.is_none() && p.santa.is_none() && p.draft.is_none());
        // The same view GET /api/pockets/:code reads as host_view, plus join and ended_at
        let mut want = kit.view(&KitCtx::new(&spec, &players, now), &st, &Viewer::Host);
        want["join"] = join;
        want["ended_at"] = serde_json::Value::Null;
        assert_eq!(q, want);
        assert_eq!((q["state"].clone(), q["now_serving"].clone(), q["next_number"].clone()), (serde_json::json!("open"), serde_json::json!(1), serde_json::json!(3)), "{q}");
        assert_eq!(q["spec"]["hours"], serde_json::json!({"until": "12:00"}), "{q}");
        let tickets = q["tickets"].as_array().unwrap();
        assert_eq!(tickets.iter().map(|t| (t["number"].as_u64().unwrap(), t["state"].as_str().unwrap(), t["name"].as_str().unwrap())).collect::<Vec<_>>(), vec![(1, "called", "Ana"), (2, "waiting", "Ben")]);
        assert_eq!(q["summary"]["waiting"], serde_json::json!(1), "{q}");
        let wire = serde_json::to_string(&Block::Pocket(Box::new(p))).unwrap();
        assert!(!wire.contains("g-ana-1") && !wire.contains("g-ben-2"), "a guest's id on the card: {wire}");
        // An ended queue: state ended, no join, its end time
        let ended_at = now;
        let q = queue_part(&spec, &st, &players, now, None, Some(ended_at));
        let p = queue_block("PQ7XK2", "Tine's Bakery", true, q.clone());
        assert_eq!(check(p.clone()), Ok(()));
        assert_eq!(p.state, "ended");
        assert!(q["join"].is_null() && q["ended_at"].is_string(), "{q}");
    }

    #[test]
    fn each_queue_rule_refuses_its_block() {
        let kit = crate::pocket::kits::kit(crate::pocket::queue::ID).unwrap();
        let spec = serde_json::json!({"title": "Tine's Bakery"});
        let good = || queue_block("PQ7XK2", "Tine's Bakery", false, queue_part(&spec, &kit.init(&spec), &[], Utc::now(), None, None));
        assert_eq!(check(good()), Ok(()));
        let ticket = |n: u64, state: &str, name: serde_json::Value| serde_json::json!({"number": n, "state": state, "name": name});
        // What a guest or the host types is plain text to the card: a `<` doesn't refuse it
        let mut p = good();
        p.queue.as_mut().unwrap()["tickets"] = serde_json::json!([ticket(1, "waiting", serde_json::json!("<b>Ana"))]);
        p.queue.as_mut().unwrap()["pause_text"] = serde_json::json!("Back <3 in 10");
        assert_eq!(check(p), Ok(()));
        let cases: Vec<(&str, Change)> = vec![
            ("no part on kit queue", Box::new(|p| p.queue = None)),
            ("a part on another kit", Box::new(|p| p.kit = "desk".into())),
            ("a top-level join", Box::new(|p| p.join = Some(serde_json::json!({"mode": "anywhere", "wifi_url": null, "anywhere_url": null, "quick_tunnel": false})))),
            ("a ready queue", Box::new(|p| p.state = "ready".into())),
            ("not an object", Box::new(|p| p.queue = Some(serde_json::json!("open")))),
            ("an unknown state", Box::new(|p| p.queue.as_mut().unwrap()["state"] = serde_json::json!("busy"))),
            ("a guest id", Box::new(move |p| p.queue.as_mut().unwrap()["tickets"] = serde_json::json!([{"number": 1, "state": "waiting", "guest": "g-1"}]))),
            ("a key", Box::new(|p| p.queue.as_mut().unwrap()["host_key"] = serde_json::json!("k"))),
            ("two called", Box::new(move |p| p.queue.as_mut().unwrap()["tickets"] = serde_json::json!([ticket(1, "called", serde_json::Value::Null), ticket(2, "called", serde_json::Value::Null)]))),
            ("the same number twice", Box::new(move |p| p.queue.as_mut().unwrap()["tickets"] = serde_json::json!([ticket(1, "waiting", serde_json::Value::Null), ticket(1, "skipped", serde_json::Value::Null)]))),
            ("a served ticket", Box::new(move |p| p.queue.as_mut().unwrap()["tickets"] = serde_json::json!([ticket(1, "served", serde_json::Value::Null)]))),
            ("number 0", Box::new(move |p| p.queue.as_mut().unwrap()["tickets"] = serde_json::json!([ticket(0, "waiting", serde_json::Value::Null)]))),
            ("a long name", Box::new(move |p| p.queue.as_mut().unwrap()["tickets"] = serde_json::json!([ticket(1, "waiting", serde_json::json!("x".repeat(41)))]))),
            ("a control character", Box::new(|p| p.queue.as_mut().unwrap()["close_text"] = serde_json::json!("Closed\u{7}"))),
            ("261 tickets", Box::new(move |p| p.queue.as_mut().unwrap()["tickets"] = serde_json::Value::Array((1..=261).map(|n| ticket(n, "waiting", serde_json::Value::Null)).collect()))),
            ("a negative count", Box::new(|p| p.queue.as_mut().unwrap()["summary"]["served_today"] = serde_json::json!(-1))),
            ("call minutes 11", Box::new(|p| p.queue.as_mut().unwrap()["spec"]["call_minutes"] = serde_json::json!(11))),
            ("HTML in the title", Box::new(|p| p.queue.as_mut().unwrap()["spec"]["title"] = serde_json::json!("<b>Bakery</b>"))),
            ("a quick tunnel", Box::new(|p| p.queue.as_mut().unwrap()["join"] = serde_json::json!({"mode": "anywhere", "anywhere_kind": "https", "anywhere_url": "https://x.trycloudflare.com/j/PQ7XK2"}))),
        ];
        for (what, change) in cases {
            let mut p = good();
            change(&mut p);
            assert!(check(p).is_err(), "{what}");
        }
        // A quiz block never carries a queue part either
        let mut d = good();
        d.kit = "quiz".into();
        assert!(check(d).is_err());
    }

    /// The turn's last card call wins, with Claude engine calls as with any other.
    #[test]
    fn quiz_then_santa_shows_the_santa_and_the_reverse_the_quiz() {
        let allowed = |tool: &str| serde_json::json!({"server": SELF_SERVER_NAME, "tool": tool, "arguments": {"spec": {"version": 1}}, "status": "allowed"});
        let quiz = allowed("pocket_create");
        let santa = allowed(santa_tool::TOOL);
        assert_eq!(last_named(&reply_with(vec![quiz.clone(), santa.clone()])), Some(Called::Santa(Named::MadeThisTurn)));
        assert_eq!(last_named(&reply_with(vec![santa.clone(), quiz.clone()])), Some(Called::Quiz(Named::MadeThisTurn)));
        // A rules-only Santa call after the quiz names no Santa, so the quiz stays
        let rules = serde_json::json!({"server": SELF_SERVER_NAME, "tool": santa_tool::TOOL, "arguments": {}, "status": "allowed"});
        assert_eq!(last_named(&reply_with(vec![quiz, rules])), Some(Called::Quiz(Named::MadeThisTurn)));
        assert_eq!(last_named(&reply_with(vec![santa])), Some(Called::Santa(Named::MadeThisTurn)));
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
