//! The production floor through the real approval route (plan §3.5): in a Dev mode space a step
//! that needs a backup gets a card with `prod_floor`, and **Allow** on it is refused by the server
//! (409, the card stays waiting) until the space has an ok backup at most 10 minutes old. Deny is
//! always accepted.

mod common;

use common::*;
use kwnt::db::dev_mode::DevBackupRow;
use kwnt::mcp::{ApprovalOutcome, ToolApprovalRequest, ToolApprover};
use kwnt::server::mcp_runtime::BrokerApprover;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

fn approver(env: &Env) -> Arc<BrokerApprover> {
    Arc::new(BrokerApprover {
        db: env.state.db.clone(),
        data_dir: env.state.config.data_dir.clone(),
        broker: env.state.broker.clone(),
        events: env.state.event_sender.clone(),
        timeout: Duration::from_millis(200),
        conversation: env.a.clone(),
        chat: Some(json!({"direct_kwant_id": env.a})),
        always_ask: false,
        strict_ask: false,
        started_by: None,
    })
}

fn request(env: &Env, command: &str) -> ToolApprovalRequest {
    ToolApprovalRequest {
        kwant_id: env.a.clone(),
        kwant_name: "A".into(),
        turn_id: "t-floor".into(),
        tool_name: "Bash".into(),
        server: String::new(),
        tool: "Bash".into(),
        arguments: json!({"command": command, "description": "demo"}),
        builtin: None,
        smart: None,
        payment: None,
    }
}

/// The card stores the command JSON-escaped, so look for it the way it is stored.
fn stored(command: &str) -> String {
    let s = serde_json::to_string(command).unwrap();
    s[1..s.len() - 1].to_string()
}

/// Runs one call nobody answers and returns its waiting card.
async fn unanswered(env: &Env, command: &str) -> (String, Value) {
    assert_eq!(approver(env).decide(request(env, command)).await, ApprovalOutcome::Unanswered);
    let rows = env.state.db.list_approvals(&kwnt::db::spaces::SpaceId::default_space(), Some("waiting"), Some(&env.a)).await.unwrap();
    let row = rows.into_iter().find(|r| r.parameters.contains(&stored(command))).expect("a waiting card");
    let details: Value = serde_json::from_str(&row.parameters).unwrap();
    (row.id, details)
}

async fn space_of_a(env: &Env) -> String {
    env.state.db.get_kwant(&env.a).await.unwrap().unwrap().space_id
}

fn backup(space: &str, ok: bool, age_s: i64) -> DevBackupRow {
    DevBackupRow {
        id: format!("dvb_{}", uuid::Uuid::new_v4().simple()),
        space_id: space.into(),
        kwant_id: None,
        path: Some("/tmp/demo.sql".into()),
        bytes: Some(10),
        ok,
        error: (!ok).then(|| "exit 1".into()),
        taken_at: chrono::Utc::now() - chrono::Duration::seconds(age_s),
    }
}

#[tokio::test]
async fn allow_on_a_step_that_needs_a_backup_is_refused_until_a_fresh_one() {
    let env = setup().await;
    let space = space_of_a(&env).await;

    // Dev mode off: no floor on the card
    let (_, off) = unanswered(&env, "sqlite3 demo.db 'DROP TABLE demo_off'").await;
    assert!(off.get("prod_floor").is_none(), "{off}");

    env.state.db.put_dev_mode(&space, true, "demo-org/demo-app", "main", None, Some("pg_dump demo > /tmp/demo.sql")).await.unwrap();
    let (ticket, details) = unanswered(&env, "sqlite3 demo.db 'DROP TABLE demo_on'").await;
    let floor = &details["prod_floor"];
    assert_eq!(floor["kind"], "destructive", "{details}");
    assert_eq!(floor["needs_backup"], true);
    assert_eq!(floor["space_id"], space.as_str());
    assert_eq!(floor["backup"]["state"], "none");

    // No backup, then an 11-minute-old one: Allow and Always allow are refused, the card waits
    env.state.db.insert_dev_backup(&backup(&space, true, 660)).await.unwrap();
    for decision in ["allow_once", "always_allow"] {
        let (s, v) = call(&env.app, "POST", &format!("/api/approvals/{ticket}/resolve"), Some(json!({"decision": decision}))).await;
        assert_eq!(s.as_u16(), 409, "{v}");
        assert_eq!(v["status"], "needs_backup");
        assert_eq!(v["resolved"], false);
        assert_eq!(v["ticket_id"], ticket.as_str());
        assert_eq!(v["backup"]["state"], "old");
        assert!(v["error"].as_str().unwrap().starts_with("Back up first"));
    }
    assert_eq!(env.state.db.get_approval(&ticket).await.unwrap().unwrap().status, "waiting");

    // A fresh ok backup, taken while the card waits, lets Allow through
    env.state.db.insert_dev_backup(&backup(&space, true, 20)).await.unwrap();
    let (s, v) = call(&env.app, "POST", &format!("/api/approvals/{ticket}/resolve"), Some(json!({"decision": "allow_once"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(env.state.db.get_approval(&ticket).await.unwrap().unwrap().status, "allowed_once");
    wait_for("the resumed turn to end", || async { env.state.turns.active_turns().is_empty() }).await;
}

#[tokio::test]
async fn deny_is_always_accepted_on_a_floor_card() {
    let env = setup().await;
    let space = space_of_a(&env).await;
    env.state.db.put_dev_mode(&space, true, "demo-org/demo-app", "main", None, None).await.unwrap();
    let (ticket, details) = unanswered(&env, "psql postgres://db.example.com/demo -c 'TRUNCATE demo'").await;
    assert_eq!(details["prod_floor"]["needs_backup"], true, "{details}");
    assert_eq!(details["prod_floor"]["backup"]["state"], "no_command");
    let (s, v) = call(&env.app, "POST", &format!("/api/approvals/{ticket}/resolve"), Some(json!({"decision": "deny"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(env.state.db.get_approval(&ticket).await.unwrap().unwrap().status, "denied");
}

/// The floor skips its reads while no space has Dev mode on (an in-memory flag). Turning it on
/// anywhere applies the floor to the very next call, and turning it off in one space leaves the
/// others' floor in place: the flag is never stale.
#[tokio::test]
async fn turning_dev_mode_on_applies_the_floor_at_once() {
    let env = setup().await;
    let space = space_of_a(&env).await;
    let db = &env.state.db;
    let floor_of = |n: u32| {
        let env = &env;
        async move { unanswered(env, &format!("sqlite3 demo.db 'DROP TABLE demo_{n}'")).await.1.get("prod_floor").cloned() }
    };
    assert!(!db.any_dev_mode_on());
    assert!(floor_of(1).await.is_none());

    // On in another space only: the flag is up, but A's space still has no floor
    db.put_dev_mode("spc_other", true, "demo-org/other", "main", None, None).await.unwrap();
    assert!(db.any_dev_mode_on());
    assert!(floor_of(2).await.is_none());

    // On in A's space: the next call gets the floor, with no wait
    db.put_dev_mode(&space, true, "demo-org/demo-app", "main", None, None).await.unwrap();
    assert_eq!(floor_of(3).await.expect("the floor applies at once")["kind"], "destructive");

    // Off in the other space: A's floor stays
    db.put_dev_mode("spc_other", false, "demo-org/other", "main", None, None).await.unwrap();
    assert!(db.any_dev_mode_on());
    assert!(floor_of(4).await.is_some());

    // Off everywhere: no floor, flag down; on again (a clone shares the flag), floor back
    db.put_dev_mode(&space, false, "demo-org/demo-app", "main", None, None).await.unwrap();
    assert!(!db.any_dev_mode_on());
    assert!(floor_of(5).await.is_none());
    db.clone().put_dev_mode(&space, true, "demo-org/demo-app", "main", None, None).await.unwrap();
    assert!(db.any_dev_mode_on());
    assert!(floor_of(6).await.is_some());

    // Deleting the space's Dev mode rows takes the flag down too
    db.delete_dev_mode_of_space(&space).await.unwrap();
    assert!(!db.any_dev_mode_on());
    assert!(floor_of(7).await.is_none());
}

/// Approvals off now wins in a turn kwant started on its own (0.9.7), but not over the floor:
/// a push to the watched branch still asks there, while an ordinary command runs.
#[tokio::test]
async fn the_floor_still_asks_with_approvals_off_in_a_kwant_started_turn() {
    let env = setup().await;
    let space = space_of_a(&env).await;
    env.state.db.set_kwant_approvals(&env.a, "off").await.unwrap();
    env.state.db.put_dev_mode(&space, true, "demo-org/demo-app", "main", None, None).await.unwrap();
    let started = BrokerApprover {
        db: env.state.db.clone(),
        data_dir: env.state.config.data_dir.clone(),
        broker: env.state.broker.clone(),
        events: env.state.event_sender.clone(),
        timeout: Duration::from_millis(200),
        conversation: env.a.clone(),
        chat: Some(json!({"direct_kwant_id": env.a})),
        always_ask: true,
        strict_ask: false,
        started_by: Some("Loose ends".into()),
    };

    assert_eq!(started.decide(request(&env, "touch notes.txt")).await, ApprovalOutcome::Allowed);
    for command in ["git push origin main", "sqlite3 demo.db 'DROP TABLE demo_auto'"] {
        assert_eq!(started.decide(request(&env, command)).await, ApprovalOutcome::Unanswered, "{command}");
        let rows = env.state.db.list_approvals(&kwnt::db::spaces::SpaceId::default_space(), Some("waiting"), Some(&env.a)).await.unwrap();
        let row = rows.into_iter().find(|r| r.parameters.contains(&stored(command))).expect("a waiting card");
        let details: Value = serde_json::from_str(&row.parameters).unwrap();
        assert!(details["prod_floor"].is_object(), "{command}: {details}");
        assert_eq!(details["started_by"], "Loose ends");
    }
}

/// A bot's own API token for a turn of A (what a bot's `curl` to kwant would carry).
async fn bot_token(env: &Env) -> String {
    use kwnt::server::continuity::{SelfScope, TurnOrigin};
    use kwnt::server::mcp_runtime;
    let bot = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { direct_kwant_id: Some(bot.id.clone()), ..Default::default() }),
        cancel: Some(tokio_util::sync::CancellationToken::new()),
        turn_id: Some("turn-floor-bot".into()),
        sandbox_target: Some(bot.id.clone()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&env.state, &bot, Some("turn-floor-bot".into()), None, Some(scope)).await.unwrap().1
}

/// 22f (Oleks, Nov 15): a deploy kwant only guesses changes the database is never a dead end. With
/// no backup command, the person can allow it without a backup (recorded on the card); a bot
/// can't; and a destructive step or a known migration still can't go without a fresh backup, even
/// through a hand-made API call.
#[tokio::test]
async fn a_guessed_migration_can_be_allowed_without_a_backup_by_the_person_only() {
    let env = setup().await;
    let space = space_of_a(&env).await;
    env.state.db.put_dev_mode(&space, true, "demo-org/demo-app", "main", None, None).await.unwrap();
    let resolve = |t: &str| format!("/api/approvals/{t}/resolve");

    // The deploy: a guess, with no backup command
    let (ticket, details) = unanswered(&env, "ssh deploy@app.example.com 'cd /srv/app && docker compose up -d'").await;
    let floor = &details["prod_floor"];
    assert_eq!(floor["kind"], "deploy_migration", "{details}");
    assert_eq!(floor["migration"], "guess");
    assert_eq!(floor["backup"]["state"], "no_command");

    // A plain Allow is refused, and says the way forward exists
    let (s, v) = call(&env.app, "POST", &resolve(&ticket), Some(json!({"decision": "allow_once"}))).await;
    assert_eq!(s.as_u16(), 409, "{v}");
    assert_eq!((v["status"].as_str(), v["code"].as_str()), (Some("needs_backup"), Some("needs_backup")));
    assert_eq!(v["can_allow_without_backup"], true);

    // A bot can't send it: the route is the person's
    let token = bot_token(&env).await;
    let auth = format!("Bearer {token}");
    let (s, v) = call_with(&env.app, "POST", &resolve(&ticket), Some(json!({"decision": "allow_once", "without_backup": true})), &[("authorization", auth.as_str())]).await;
    assert_eq!(s.as_u16(), 403, "{v}");
    assert_eq!(env.state.db.get_approval(&ticket).await.unwrap().unwrap().status, "waiting");

    // The backup state reads the same as the card's, for the person only
    let backup_uri = format!("/api/spaces/{space}/dev/backup");
    let (s, v) = call(&env.app, "GET", &backup_uri, None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v, floor["backup"]);
    let (s, _) = call_with(&env.app, "GET", &backup_uri, None, &[("authorization", auth.as_str())]).await;
    assert_eq!(s.as_u16(), 403);
    // Setting a command (the same PUT as Dev mode settings) moves it on
    let (s, v) = call(&env.app, "PUT", &format!("/api/spaces/{space}/dev_mode"), Some(json!({"backup_command": "pg_dump demo > /tmp/demo.sql"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(call(&env.app, "GET", &backup_uri, None).await.1["state"], "none");

    // The person's Allow without a backup goes through, and is recorded on the card
    let (s, v) = call(&env.app, "POST", &resolve(&ticket), Some(json!({"decision": "allow_once", "without_backup": true}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["without_backup"], true);
    let row = env.state.db.get_approval(&ticket).await.unwrap().unwrap();
    assert_eq!(row.status, "allowed_once");
    let saved: Value = serde_json::from_str(&row.parameters).unwrap();
    let rec = &saved["prod_floor"]["allowed_without_backup"];
    assert_eq!(rec["line"], "Allowed without a backup by you", "{saved}");
    assert_eq!((rec["by"].as_str(), rec["via"].as_str()), (Some("person"), Some("app")));
    assert!(rec["at"].is_string());
    wait_for("the resumed turn to end", || async { env.state.turns.active_turns().is_empty() }).await;

    // A destructive step and a known migration: a hand-made call with without_backup is refused
    for cmd in ["ssh deploy@app.example.com 'rm -rf /srv/app/data'", "RAILS_ENV=production bin/rails db:migrate"] {
        let (ticket, details) = unanswered(&env, cmd).await;
        assert_eq!(details["prod_floor"]["needs_backup"], true, "{details}");
        assert_ne!(details["prod_floor"]["migration"], "guess", "{details}");
        for decision in ["allow_once", "always_allow"] {
            let (s, v) = call(&env.app, "POST", &resolve(&ticket), Some(json!({"decision": decision, "without_backup": true}))).await;
            assert_eq!(s.as_u16(), 409, "{cmd}: {v}");
            assert_eq!((v["status"].as_str(), v["code"].as_str()), (Some("needs_backup"), Some("backup_required")), "{cmd}");
            assert_eq!(v["can_allow_without_backup"], false);
        }
        let row = env.state.db.get_approval(&ticket).await.unwrap().unwrap();
        assert_eq!(row.status, "waiting", "{cmd}");
        assert!(!row.parameters.contains("allowed_without_backup"));
    }
}

/// 22f: Oleks's own deploy, verbatim: the card names the server `$R` stands for.
#[tokio::test]
async fn where_names_the_server_of_olekss_deploy() {
    let env = setup().await;
    let space = space_of_a(&env).await;
    env.state.db.put_dev_mode(&space, true, "demo-org/demo-app", "main", None, None).await.unwrap();
    let cmd = r#"set -e; R=root@164.90.207.66; D=/opt/tradebot; timeout 120 ssh -o BatchMode=yes -o ConnectTimeout=15 $R "cd $D && chmod +x catalyst-linux.new && docker compose up -d --no-deps --force-recreate catalyst 2>&1 && docker compose ps catalyst"; echo "exit=$?""#;
    let (_, details) = unanswered(&env, cmd).await;
    let blast = details["prod_floor"]["blast"].as_array().unwrap().clone();
    assert!(blast.contains(&json!(["Where", "root@164.90.207.66"])), "{blast:?}");
    assert_eq!(details["prod_floor"]["migration"], "guess");
}
