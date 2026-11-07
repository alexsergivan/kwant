//! 0.7.6 Jira tickets (jira-tickets spec J2–J5, J9, J13–J16; design amendments): the site check,
//! saving a site, pickup, the stop rules, the slot cap, back-off and Ask on Jira, against the mock
//! Atlassian (Cloud and DC shapes) on 127.0.0.1. No real Atlassian site is ever contacted.
// Each test holds GATEWAY (a std Mutex) for its whole run on purpose: it serialises the
// process-wide KWANT_ATLASSIAN_GATEWAY_URL so no test can reach the real gateway.
#![allow(clippy::await_holding_lock)]

mod common;
#[path = "support/mock_atlassian.rs"]
mod mock_atlassian;

use common::*;
use kwnt::db::dev_jira::{self, TicketRow};
use kwnt::db::spaces::SpaceId;
use kwnt::server::continuity::{SelfScope, TurnOrigin};
use kwnt::server::dev::tickets::{pickup, store};
use kwnt::server::mcp_runtime;
use mock_atlassian::{Fault, Flavor, MockAtlassian};
use serde_json::{json, Value};

/// The scoped-token gateway is read from the environment, which every test shares: tests that
/// talk to a mock hold this and point it at their own mock (never at api.atlassian.com).
static GATEWAY: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gateway_to(m: &MockAtlassian) -> std::sync::MutexGuard<'static, ()> {
    let g = GATEWAY.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("KWANT_ATLASSIAN_GATEWAY_URL", &m.base);
    g
}

/// A server whose space asks before running commands: the built-in password store only takes the
/// Jira token then (passwords::refuse_local_save), as on a fresh install.
async fn jira_setup() -> Env {
    let env = setup().await;
    env.state.db.set_space_approvals(&SpaceId::default_space(), "ask").await.unwrap();
    env
}

fn space() -> String {
    SpaceId::default_space().as_str().to_string()
}

fn uri(rest: &str) -> String {
    format!("/api/spaces/{}/dev/{rest}", space())
}

async fn turn_token(env: &Env) -> String {
    let bot = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { direct_kwant_id: Some(bot.id.clone()), ..Default::default() }),
        cancel: Some(tokio_util::sync::CancellationToken::new()),
        turn_id: Some("turn-dev-jira".into()),
        sandbox_target: Some(bot.id.clone()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&env.state, &bot, Some("turn-dev-jira".into()), None, Some(scope)).await.unwrap().1
}

async fn dev_mode_on(env: &Env) {
    let (s, v) = call(&env.app, "PUT", &format!("/api/spaces/{}/dev_mode", space()), Some(json!({"on": true}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
}

/// The sign-in part of a check or save body for this mock.
fn sign_in(m: &MockAtlassian) -> Value {
    match m.flavor {
        Flavor::Cloud => json!({"jira_url": m.base, "kind": "cloud", "email": m.email(), "token": m.token()}),
        Flavor::Dc => json!({"jira_url": m.base, "token": m.token()}),
    }
}

fn with(mut a: Value, b: Value) -> Value {
    for (k, v) in b.as_object().unwrap() {
        a[k] = v.clone();
    }
    a
}

async fn connect(env: &Env, m: &MockAtlassian, extra: Value) -> Value {
    let body = with(with(sign_in(m), json!({"projects": ["TRAIL"], "handover": "label", "label": "kwant"})), extra);
    let (s, v) = call(&env.app, "PUT", &uri("jira"), Some(body)).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert!(!v.to_string().contains(&m.token()), "a save never answers the token: {v}");
    v
}

async fn poll(env: &Env) -> Value {
    let (s, v) = call(&env.app, "POST", &uri("jira/poll"), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    v
}

async fn ticket(env: &Env, key: &str) -> TicketRow {
    env.state.db.dev_ticket(&space(), key).await.unwrap().unwrap_or_else(|| panic!("no row for {key}"))
}

async fn events(env: &Env, key: &str) -> Vec<String> {
    env.state.db.dev_ticket_events(&space(), key, 100).await.unwrap().into_iter().map(|e| format!("{}:{}", e.kind, e.detail.unwrap_or_default())).collect()
}

/// Nothing kwant kept (rows, events, settings answers) carries a token.
async fn no_token_anywhere(env: &Env, m: &MockAtlassian) {
    let (_, site) = call(&env.app, "GET", &uri("jira"), None).await;
    let (_, list) = call(&env.app, "GET", &uri("tickets"), None).await;
    let mut all = format!("{site}{list}");
    for t in env.state.db.dev_tickets(&space()).await.unwrap() {
        all.push_str(&format!("{:?}", env.state.db.dev_ticket_events(&space(), &t.key, 100).await.unwrap()));
    }
    for tok in [m.token(), m.scoped_token(), m.confluence_token()] {
        assert!(!all.contains(&tok), "a token leaked into kwant's answers or rows");
    }
}

// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn with_no_jira_site_nothing_changes() {
    let env = jira_setup().await;
    dev_mode_on(&env).await;
    // api-decisions 9: not connected is 200 {site: null, tickets: []}
    let (s, v) = call(&env.app, "GET", &uri("jira"), None).await;
    assert_eq!((s.as_u16(), v.clone()), (200, json!({"site": null, "tickets": []})), "{v}");
    let (s, v) = call(&env.app, "GET", &uri("tickets"), None).await;
    assert_eq!((s.as_u16(), v["tickets"].clone()), (200, json!([])), "{v}");
    let (s, v) = call(&env.app, "POST", &uri("jira/poll"), None).await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (404, Some("no_site")), "{v}");
    let r = pickup::poll_site(&env.state, &space()).await;
    assert_eq!(r.skipped, Some("no_site"));
    assert!(env.state.db.dev_jira_sites_on().await.unwrap().is_empty());
    // Dev mode itself answers as before
    let (s, v) = call(&env.app, "GET", &format!("/api/spaces/{}/dev_mode", space()), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert!(v.to_string().find("jira").is_none(), "Dev mode's answer is unchanged with no site: {v}");
}

#[tokio::test]
async fn a_bot_can_not_read_or_change_the_jira_connection() {
    let env = jira_setup().await;
    let m = MockAtlassian::cloud().await;
    let _g = gateway_to(&m);
    m.add_project("TRAIL", "Trail app");
    let token = turn_token(&env).await;
    let auth = format!("Bearer {token}");
    let h = [("authorization", auth.as_str())];
    for (method, path, body) in [
        ("GET", uri("jira"), None),
        ("PUT", uri("jira"), Some(with(sign_in(&m), json!({"projects": ["TRAIL"]})))),
        ("POST", uri("jira/check"), Some(sign_in(&m))),
        ("POST", uri("jira/poll"), None),
        ("DELETE", uri("jira"), None),
        ("GET", uri("tickets"), None),
        ("POST", uri("tickets/TRAIL-101/start"), None),
        ("POST", uri("tickets/TRAIL-101/stop"), None),
        ("POST", uri("tickets/TRAIL-101/skip"), None),
        ("POST", uri("tickets/TRAIL-101/close"), None),
        ("POST", uri("tickets/TRAIL-101/retry"), None),
        ("POST", uri("jira/check"), Some(json!({"address": m.base, "detect": true}))),
        ("POST", uri("tickets/TRAIL-101/ask/preview"), Some(json!({"questions": [{"text": "a?"}]}))),
        ("POST", uri("tickets/TRAIL-101/ask"), Some(json!({"questions": [{"text": "a?"}], "digest": "x"}))),
    ] {
        let (s, v) = call_with(&env.app, method, &path, body, &h).await;
        assert_eq!((s.as_u16(), v["code"].as_str()), (403, Some("person_only")), "{method} {path}: {v}");
    }
    assert!(m.hits().is_empty(), "a refused bot never reached Jira");
    assert!(env.state.db.dev_jira_site(&space()).await.unwrap().is_none());
}

#[tokio::test]
async fn the_check_answers_found_auth_failed_not_jira_unreachable_no_projects_and_rate_limited() {
    let env = jira_setup().await;
    let m = MockAtlassian::cloud().await;
    let _g = gateway_to(&m);
    let check = |body: Value| {
        let app = env.app.clone();
        async move {
            let (s, v) = call(&app, "POST", &uri("jira/check"), Some(body)).await;
            assert_eq!(s.as_u16(), 200, "{v}");
            v
        }
    };

    // No projects visible is its own answer, not "auth failed"
    let v = check(sign_in(&m)).await;
    assert_eq!(v["result"], "no_projects", "{v}");
    assert!(v["message"].as_str().unwrap().contains("can't see any projects"), "{v}");

    m.add_project("TRAIL", "Trail app").add_project("SHOP", "Shop");
    let v = check(sign_in(&m)).await;
    assert_eq!((v["result"].as_str(), v["kind"].as_str(), v["auth"].as_str()), (Some("found"), Some("cloud"), Some("basic")), "{v}");
    assert_eq!(v["me"]["display_name"], "kwant bot");
    assert_eq!(v["projects"].as_array().unwrap().len(), 2);
    assert_eq!(v["reads"], json!(["jira", "confluence"]), "{v}");
    assert_eq!(v["scope_note"], Value::Null, "a classic token has no scope line");
    assert!(!v.to_string().contains(&m.token()));

    // The suggested project comes from Dev mode's branch names; none → nothing pre-ticked
    assert_eq!(v["suggested_project"], Value::Null, "{v}");
    for (i, b) in ["SHOP-7-cart", "feature/SHOP-9", "TRAIL-2-x"].iter().enumerate() {
        env.state.db.insert_dev_card(&space(), kwnt::db::dev_mode::REVIEW, &format!("k{i}"), &format!("20261105T03000{i}.000"), &json!({"head_branch": b}).to_string()).await.unwrap();
    }
    let v = check(sign_in(&m)).await;
    assert_eq!(v["suggested_project"]["key"], "SHOP", "{v}");

    // A count and a sample for the rule
    m.add_issue("TRAIL", "Add dark mode", &["kwant"], None);
    m.add_issue("TRAIL", "Not for kwant", &[], None);
    let v = check(with(sign_in(&m), json!({"projects": ["TRAIL"], "handover": "label", "label": "kwant"}))).await;
    assert_eq!((v["result"].as_str(), v["count"].as_u64()), (Some("found"), Some(1)), "{v}");
    assert!(v["jql"].as_str().unwrap().contains("labels = \"kwant\""), "{v}");
    // "Either" is Advanced JQL only (design amendment J4): not a hand-over word (api-decisions 2)
    let (s, v) = call(&env.app, "POST", &uri("jira/check"), Some(with(sign_in(&m), json!({"projects": ["TRAIL"], "handover": "either"})))).await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (400, Some("bad_request")), "{v}");

    // A wrong token
    let v = check(with(sign_in(&m), json!({"token": "mock-wrong-not-a-real-token"}))).await;
    assert_eq!(v["result"], "auth_failed", "{v}");
    assert!(v["message"].as_str().unwrap().contains(&m.email()), "{v}");

    // Rate limited, with the time to try again
    m.fault("/myself", Fault::TooMany { retry_after: Some(120) });
    let v = check(sign_in(&m)).await;
    assert_eq!(v["result"], "rate_limited", "{v}");
    let at = chrono::DateTime::parse_from_rfc3339(v["retry_at"].as_str().unwrap()).unwrap();
    let wait = at.with_timezone(&chrono::Utc) - chrono::Utc::now();
    assert!(wait > chrono::Duration::seconds(100) && wait <= chrono::Duration::seconds(121), "{v}");
    m.heal("/myself");

    // Nothing at the address
    let v = check(json!({"jira_url": "http://127.0.0.1:9", "kind": "cloud", "email": m.email(), "token": m.token()})).await;
    assert_eq!(v["result"], "unreachable", "{v}");
    // Something that isn't Jira (type detection by serverInfo fails)
    let v = check(json!({"jira_url": format!("{}/not-jira", m.base), "token": m.token()})).await;
    assert_eq!(v["result"], "not_jira", "{v}");
    assert!(env.state.db.dev_jira_site(&space()).await.unwrap().is_none(), "a check saves nothing");
}

#[tokio::test]
async fn a_scoped_cloud_token_goes_through_the_gateway_and_says_what_it_reads() {
    let env = jira_setup().await;
    let m = MockAtlassian::cloud().await;
    let _g = gateway_to(&m);
    m.add_project("TRAIL", "Trail app");
    m.add_space("ENG", "Engineering");
    let body = with(sign_in(&m), json!({"token": m.scoped_token()}));
    let (s, v) = call(&env.app, "POST", &uri("jira/check"), Some(body.clone())).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!((v["result"].as_str(), v["auth"].as_str()), (Some("found"), Some("scoped")), "{v}");
    assert_eq!(v["cloud_id"].as_str(), Some(m.cloud_id().as_str()));
    assert_eq!(v["scope_note"], "scoped: reads Jira and Confluence", "{v}");
    assert!(m.hits().iter().any(|h| h.gateway && h.path.contains(&format!("/ex/jira/{}/rest/api/3/myself", m.cloud_id()))));

    // Saved and polled through the gateway too
    dev_mode_on(&env).await;
    let v = connect(&env, &m, json!({"token": m.scoped_token(), "spaces": ["ENG"]})).await;
    assert_eq!(v["site"]["auth"], "scoped");
    // api-decisions 6: the non-secret scope line is kept and shown again
    assert_eq!(v["site"]["scope_note"], "scoped: reads Jira and Confluence", "{v}");
    let (_, g) = call(&env.app, "GET", &uri("jira"), None).await;
    assert_eq!((g["site"]["scope_note"].as_str(), g["site"]["spaces"].clone()), (Some("scoped: reads Jira and Confluence"), json!(["ENG"])), "{g}");
    m.clear_hits();
    m.add_issue("TRAIL", "Add dark mode", &["kwant"], None);
    poll(&env).await;
    assert!(m.hits().iter().filter(|h| h.path.contains("search")).all(|h| h.gateway), "{:?}", m.hits());
    no_token_anywhere(&env, &m).await;
}

#[tokio::test]
async fn a_dc_site_is_detected_and_its_confluence_has_its_own_address_and_pat() {
    let env = jira_setup().await;
    let m = MockAtlassian::dc().await;
    let _g = gateway_to(&m);
    m.add_project("TRAIL", "Trail app");
    m.add_space("ENG", "Engineering");
    let (_, v) = call(&env.app, "POST", &uri("jira/check"), Some(with(sign_in(&m), json!({"confluence_url": m.confluence_url()})))).await;
    assert_eq!((v["result"].as_str(), v["kind"].as_str(), v["auth"].as_str()), (Some("found"), Some("dc"), Some("pat")), "{v}");
    assert_eq!(v["confluence"]["result"], "needs_token", "{v}");
    let body = with(sign_in(&m), json!({"confluence_url": m.confluence_url(), "confluence_token": m.confluence_token()}));
    let (_, v) = call(&env.app, "POST", &uri("jira/check"), Some(body)).await;
    assert_eq!(v["confluence"]["result"], "ok", "{v}");
    assert_eq!(v["reads"], json!(["jira", "confluence"]));
    assert_eq!(v["scope_note"], Value::Null, "a PAT has no scope line");
    assert!(m.hits().iter().all(|h| h.auth != Some("basic")), "DC is bearer only");

    dev_mode_on(&env).await;
    let v = connect(&env, &m, json!({"confluence_url": m.confluence_url(), "confluence_token": m.confluence_token(), "spaces": ["ENG"]})).await;
    assert_eq!((v["site"]["kind"].as_str(), v["site"]["confluence_url"].as_str()), (Some("dc"), Some(m.confluence_url().as_str())));
    assert_eq!(v["site"]["scope_note"], Value::Null, "a PAT has no scope line");
    // Saving again without retyping the tokens keeps them
    let v = connect(&env, &m, json!({"token": null, "confluence_url": m.confluence_url(), "spaces": ["ENG"], "poll_secs": 600})).await;
    assert_eq!(v["site"]["poll_secs"], 600);
    no_token_anywhere(&env, &m).await;
}

#[tokio::test]
async fn pickup_backfills_picks_up_stops_and_names_who_took_it_back() {
    let env = jira_setup().await;
    let m = MockAtlassian::cloud().await;
    let _g = gateway_to(&m);
    dev_mode_on(&env).await;
    m.add_project("TRAIL", "Trail app");
    let dana = m.add_user("dana", "Dana Park");
    let old = m.add_issue("TRAIL", "Labelled before connecting", &["kwant"], Some("dana"));
    m.add_issue("TRAIL", "Not handed over", &[], None);

    // "Either" can't be saved without Advanced JQL
    let body = with(sign_in(&m), json!({"projects": ["TRAIL"], "handover": "either"}));
    let (s, v) = call(&env.app, "PUT", &uri("jira"), Some(body)).await;
    assert!(s.is_client_error(), "{v}");

    connect(&env, &m, json!({})).await;
    let r = poll(&env).await;
    assert_eq!((r["matched"].as_u64(), r["waiting"].as_u64(), r["started"].as_u64()), (Some(1), Some(1), Some(0)), "{r}");
    let t = ticket(&env, &old).await;
    assert_eq!((t.state.as_str(), t.waiting_since_connect), (dev_jira::WAITING, true), "backfill waits for Start");
    assert_eq!(t.handed_over_by.as_deref(), Some("label"));
    assert_eq!(t.assignee_id.as_deref(), Some(dana.account_id.as_str()), "the label rule keeps the human assignee");

    // A new hand-over after connecting is picked up and started (a slot is free)
    let new = m.add_issue("TRAIL", "Add dark mode", &["kwant", "ui"], None);
    let r = poll(&env).await;
    assert_eq!((r["new"].as_u64(), r["started"].as_u64()), (Some(1), Some(1)), "{r}");
    let t = ticket(&env, &new).await;
    assert_eq!((t.state.as_str(), t.waiting_since_connect, t.start_called_at.is_some()), (dev_jira::PICKED_UP, false, true));
    assert!(events(&env, &new).await.contains(&"hook:start".to_string()));

    // Start on the Waiting card
    let (s, v) = call(&env.app, "POST", &uri(&format!("tickets/{old}/start")), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!((v["ticket"]["state"].as_str(), v["started"].as_bool()), (Some(dev_jira::PICKED_UP), Some(true)), "{v}");
    let (s, _) = call(&env.app, "POST", &uri(&format!("tickets/{old}/start")), None).await;
    assert_eq!(s.as_u16(), 409, "Start twice is refused");

    // Dana removes the label: taken back, and the card names her
    m.act_as("dana");
    m.set_labels(&new, &["ui"]);
    let r = poll(&env).await;
    assert_eq!(r["taken_back"].as_u64(), Some(1), "{r}");
    let t = ticket(&env, &new).await;
    assert_eq!(t.state, dev_jira::TAKEN_BACK);
    assert_eq!(t.taken_back_by.as_deref(), Some("Dana Park"));
    assert!(t.taken_back_at.is_some());
    assert!(events(&env, &new).await.contains(&"hook:stopped: taken_back".to_string()));
    // Handed over again: picked up again
    m.set_labels(&new, &["ui", "kwant"]);
    poll(&env).await;
    assert_eq!(ticket(&env, &new).await.state, dev_jira::PICKED_UP);

    // Done closes it, with the resolution; deleted closes too
    m.transition(&old, "Done", "done");
    m.delete(&new);
    let r = poll(&env).await;
    assert_eq!(r["closed"].as_u64(), Some(2), "{r}");
    let t = ticket(&env, &old).await;
    assert_eq!(t.state, dev_jira::CLOSED);
    assert!(t.closed_resolution.as_deref().unwrap_or_default().starts_with("done"), "{:?}", t.closed_resolution);
    assert_eq!(ticket(&env, &new).await.closed_resolution.as_deref(), Some("deleted"));

    // Only reads and no writes went to Jira
    assert!(m.writes().is_empty() && m.refused().is_empty(), "pickup never writes to Jira");
    no_token_anywhere(&env, &m).await;
}

#[tokio::test]
async fn a_search_lag_is_reconciled_and_never_reads_as_taken_back() {
    let env = jira_setup().await;
    let m = MockAtlassian::cloud().await;
    let _g = gateway_to(&m);
    dev_mode_on(&env).await;
    m.add_project("TRAIL", "Trail app");
    connect(&env, &m, json!({})).await;
    poll(&env).await;
    let key = m.add_issue("TRAIL", "Lagging", &["kwant"], None);
    poll(&env).await;
    assert_eq!(ticket(&env, &key).await.state, dev_jira::PICKED_UP);
    m.hide(&key, true);
    poll(&env).await;
    assert_eq!(ticket(&env, &key).await.state, dev_jira::PICKED_UP, "still handed over in Jira");
    let id = ticket(&env, &key).await.issue_id.unwrap().parse::<u64>().unwrap();
    assert!(m.reconciled().iter().any(|r| r.contains(&id)), "{:?}", m.reconciled());
}

#[tokio::test]
async fn at_most_three_tickets_run_and_the_queue_goes_by_priority() {
    let env = jira_setup().await;
    let m = MockAtlassian::cloud().await;
    let _g = gateway_to(&m);
    dev_mode_on(&env).await;
    m.add_project("TRAIL", "Trail app");
    connect(&env, &m, json!({})).await;
    poll(&env).await;
    let keys: Vec<String> = (0..5).map(|i| m.add_issue("TRAIL", &format!("T{i}"), &["kwant"], None)).collect();
    m.set_priority(&keys[4], "Highest", "1");
    let r = poll(&env).await;
    assert_eq!((r["new"].as_u64(), r["started"].as_u64()), (Some(5), Some(pickup::SLOTS as u64)), "{r}");
    let started: Vec<String> = env.state.db.dev_tickets(&space()).await.unwrap().into_iter().filter(|t| t.start_called_at.is_some()).map(|t| t.key).collect();
    assert!(started.contains(&keys[4]), "Highest first: {started:?}");
    assert!(started.contains(&keys[0]) && started.contains(&keys[1]), "then oldest: {started:?}");
    // A later poll starts nothing more while all slots are taken
    let r = poll(&env).await;
    assert_eq!(r["started"].as_u64(), Some(0), "{r}");
}

#[tokio::test]
async fn a_429_backs_off_until_retry_after_and_a_revoked_token_is_named() {
    let env = jira_setup().await;
    let m = MockAtlassian::cloud().await;
    let _g = gateway_to(&m);
    dev_mode_on(&env).await;
    m.add_project("TRAIL", "Trail app");
    connect(&env, &m, json!({})).await;
    m.fault("/search/jql", Fault::TooMany { retry_after: Some(600) });
    let r = poll(&env).await;
    assert!(r["error"].as_str().unwrap_or_default().contains("slow down"), "{r}");
    let site = env.state.db.dev_jira_site(&space()).await.unwrap().unwrap();
    let wait = site.backoff_until.unwrap() - chrono::Utc::now();
    assert!(wait > chrono::Duration::seconds(590) && wait <= chrono::Duration::seconds(601), "{wait:?}");
    assert!(!pickup::due(&site, chrono::Utc::now() + chrono::Duration::seconds(400)));
    let searches = m.searches();
    let r = poll(&env).await;
    assert_eq!(r["skipped"], "backing_off", "{r}");
    assert_eq!(m.searches(), searches, "no request while backing off");

    // Saving the site again clears the back-off; a revoked token is named, not retried
    m.heal("/search/jql");
    connect(&env, &m, json!({})).await;
    m.revoke();
    poll(&env).await;
    let site = env.state.db.dev_jira_site(&space()).await.unwrap().unwrap();
    assert!(site.token_rejected_at.is_some(), "{site:?}");
}

#[tokio::test]
async fn turning_pickup_off_keeps_the_cards_and_delete_forgets_the_token() {
    let env = jira_setup().await;
    let m = MockAtlassian::cloud().await;
    let _g = gateway_to(&m);
    dev_mode_on(&env).await;
    m.add_project("TRAIL", "Trail app");
    connect(&env, &m, json!({})).await;
    poll(&env).await;
    let key = m.add_issue("TRAIL", "A", &["kwant"], None);
    poll(&env).await;
    let (s, v) = call(&env.app, "POST", &uri("jira/on"), Some(json!({"on": false}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let searches = m.searches();
    assert_eq!(poll(&env).await["skipped"], "off");
    assert_eq!(m.searches(), searches);
    assert!(env.state.db.dev_ticket(&space(), &key).await.unwrap().is_some(), "cards stay");

    let site_id = env.state.db.dev_jira_site(&space()).await.unwrap().unwrap().id;
    let (s, v) = call(&env.app, "DELETE", &uri("jira"), None).await;
    assert_eq!((s.as_u16(), v["deleted"].as_bool()), (200, Some(true)), "{v}");
    assert!(env.state.db.dev_jira_site(&space()).await.unwrap().is_none());
    let store = env.state.credentials.local();
    assert!(kwnt::zombie::hunt::source_secret(store, &site_id, &m.base).await.unwrap().is_none(), "the token is forgotten");
}

/// Design §5 and amendment J9: the exact comment is shown first, mentions the reporter (an ADF
/// mention on Cloud, `[~username]` on DC) and is posted once, at the person's word.
#[tokio::test]
async fn ask_on_jira_shows_the_exact_comment_and_posts_it_once_with_a_mention() {
    for flavor in [Flavor::Cloud, Flavor::Dc] {
        let env = jira_setup().await;
        let m = MockAtlassian::start(flavor).await;
        let _g = gateway_to(&m);
        dev_mode_on(&env).await;
        m.add_project("TRAIL", "Trail app");
        connect(&env, &m, json!({})).await;
        poll(&env).await;
        let key = m.add_issue("TRAIL", "Export orders", &["kwant"], None);
        poll(&env).await;
        let questions = json!({"questions": [{"text": "Which file should the export make?", "choices": ["CSV", "XLSX"]}], "team": "Shop team"});

        // Only while the card is waiting on questions
        let (s, v) = call(&env.app, "POST", &uri(&format!("tickets/{key}/ask/preview")), Some(questions.clone())).await;
        assert_eq!(s.as_u16(), 409, "{v}");
        assert!(store::set_state(&env.state, &space(), &key, dev_jira::READING, None).await.unwrap());
        assert!(store::set_state(&env.state, &space(), &key, dev_jira::NEEDS_ANSWER, None).await.unwrap());

        let (s, p) = call(&env.app, "POST", &uri(&format!("tickets/{key}/ask/preview")), Some(questions.clone())).await;
        assert_eq!(s.as_u16(), 200, "{p}");
        assert_eq!((p["to"]["display_name"].as_str(), p["to"]["role"].as_str()), (Some("Ana Reporter"), Some("reporter")), "{p}");
        let text = p["text"].as_str().unwrap();
        assert!(text.starts_with(&format!("Hi @Ana Reporter, before we start {key} we have a question:")), "{text}");
        assert!(text.ends_with("— kwant, for the Shop team"), "{text}");
        assert_eq!(p["as"]["display_name"], "kwant bot");
        assert!(m.writes().is_empty(), "a preview posts nothing");

        // A digest that doesn't match what was shown is refused
        let (s, v) = call(&env.app, "POST", &uri(&format!("tickets/{key}/ask")), Some(with(questions.clone(), json!({"digest": "0".repeat(64)})))).await;
        assert_eq!((s.as_u16(), v["code"].as_str()), (409, Some("changed")), "{v}");
        let changed = with(questions.clone(), json!({"team": "Other team", "digest": p["digest"]}));
        let (s, _) = call(&env.app, "POST", &uri(&format!("tickets/{key}/ask")), Some(changed)).await;
        assert_eq!(s.as_u16(), 409);
        assert!(m.writes().is_empty());

        let (s, v) = call(&env.app, "POST", &uri(&format!("tickets/{key}/ask")), Some(with(questions.clone(), json!({"digest": p["digest"]})))).await;
        assert_eq!(s.as_u16(), 200, "{v}");
        let id = v["comment_id"].as_str().unwrap().to_string();
        assert_eq!(v["url"].as_str().unwrap(), format!("{}/browse/{key}?focusedCommentId={id}", m.base));
        let writes = m.writes();
        assert_eq!(writes.len(), 1, "one post");
        let ana = m.user("ana").unwrap();
        match flavor {
            Flavor::Cloud => {
                let body: Value = serde_json::from_str(&writes[0].body).unwrap();
                let first = &body["body"]["content"][0]["content"];
                assert_eq!(first[1], json!({"type": "mention", "attrs": {"id": ana.account_id, "text": "@Ana Reporter"}}), "{body}");
            }
            Flavor::Dc => assert!(writes[0].body.contains("Hi [~ana], before we start"), "{}", writes[0].body),
        }
        assert_eq!(ticket(&env, &key).await.asked_comment_id.as_deref(), Some(id.as_str()));
        // Asked once: a second post is refused
        let (s, v) = call(&env.app, "POST", &uri(&format!("tickets/{key}/ask")), Some(with(questions.clone(), json!({"digest": p["digest"]})))).await;
        assert_eq!((s.as_u16(), v["code"].as_str()), (409, Some("already_asked")), "{v}");
        assert_eq!(m.writes().len(), 1);

        // kwant's own comment isn't an answer; the reporter's reply is
        m.comment_as(&key, "kwant-bot", "a note");
        let reply = m.comment_as(&key, "ana", "CSV please");
        let r = poll(&env).await;
        assert_eq!(r["replies"].as_u64(), Some(1), "{flavor:?}: {r}");
        assert!(events(&env, &key).await.iter().any(|e| e.starts_with("hook:reporter_replied") && e.contains(&reply)));
        assert_eq!(ticket(&env, &key).await.last_reply_id.as_deref(), Some(reply.as_str()));
        let r = poll(&env).await;
        assert_eq!(r["replies"].as_u64(), Some(0), "once per reply");
        no_token_anywhere(&env, &m).await;
    }
}

#[tokio::test]
async fn detect_names_the_jira_kind_with_no_token() {
    let env = jira_setup().await;
    let dc = MockAtlassian::dc().await;
    let _g = gateway_to(&dc);
    let cloud = MockAtlassian::cloud().await;
    let detect = |address: String| {
        let app = env.app.clone();
        async move { call(&app, "POST", &uri("jira/check"), Some(json!({"address": address, "detect": true}))).await }
    };
    let (s, v) = detect(dc.base.clone()).await;
    assert_eq!((s.as_u16(), v.clone()), (200, json!({"result": "detected", "kind": "dc"})), "{v}");
    let (s, v) = detect(cloud.base.clone()).await;
    assert_eq!((s.as_u16(), v.clone()), (200, json!({"result": "detected", "kind": "cloud"})), "{v}");
    let (_, v) = detect(format!("{}/not-jira", dc.base)).await;
    assert_eq!(v, json!({"result": "not_jira"}));
    let (_, v) = detect("http://127.0.0.1:9".into()).await;
    assert_eq!(v, json!({"result": "unreachable"}));
    dc.fault("/serverInfo", Fault::TooMany { retry_after: Some(90) });
    let (_, v) = detect(dc.base.clone()).await;
    assert_eq!(v["result"], "rate_limited", "{v}");
    assert!(v["retry_at"].is_string(), "{v}");
    dc.heal("/serverInfo");
    // An address that can't be used is a coded 400
    for bad in ["", "ftp://jira.example.com", "javascript:alert(1)"] {
        let (s, v) = detect(bad.into()).await;
        assert_eq!((s.as_u16(), v["code"].as_str()), (400, Some("bad_address")), "{bad}: {v}");
    }
    // No token was read, sent or needed; nothing was saved
    assert!(dc.hits().iter().chain(cloud.hits().iter()).all(|h| h.auth.is_none()), "detect is anonymous");
    assert!(env.state.db.dev_jira_site(&space()).await.unwrap().is_none());
}

#[tokio::test]
async fn the_site_answer_has_arrays_on_and_puts_hand_over_words_and_every_4xx_is_coded() {
    let env = jira_setup().await;
    let m = MockAtlassian::cloud().await;
    let _g = gateway_to(&m);
    dev_mode_on(&env).await;
    m.add_project("TRAIL", "Trail app");
    m.add_issue("TRAIL", "Add dark mode", &["kwant"], None);

    let v = connect(&env, &m, json!({"handover": "label"})).await;
    for site in [v["site"].clone(), call(&env.app, "GET", &uri("jira"), None).await.1["site"].clone()] {
        assert_eq!(site["projects"], json!(["TRAIL"]), "api-decisions 1: {site}");
        assert_eq!(site["spaces"], json!([]), "{site}");
        assert_eq!((site["on"].as_bool(), site.get("on_")), (Some(true), None), "api-decisions 3: {site}");
        assert_eq!(site["handover"], "label", "{site}");
    }
    poll(&env).await;
    let (_, g) = call(&env.app, "GET", &uri("jira"), None).await;
    let tickets = g["tickets"].as_array().expect("tickets is the rows");
    assert_eq!(tickets.len(), 1, "{g}");
    assert_eq!((tickets[0]["labels"].clone(), tickets[0]["work"].clone()), (json!(["kwant"]), Value::Null), "{g}");

    // Assignee is stored and answered in PUT's words; "either" never is
    let dana = m.add_user("dana", "Dana Park");
    let v = connect(&env, &m, json!({"handover": "assignee", "assignee_id": dana.account_id})).await;
    assert_eq!(v["site"]["handover"], "assignee");
    let (s, v) = call(&env.app, "PUT", &uri("jira"), Some(with(sign_in(&m), json!({"projects": ["TRAIL"], "handover": "either"})))).await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (400, Some("bad_request")), "{v}");
    let (s, v) = call(&env.app, "PUT", &uri("jira"), Some(with(sign_in(&m), json!({"projects": ["TRAIL"], "handover": "jql"})))).await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (400, Some("bad_request")), "jql needs the search: {v}");

    // api-decisions 10: every 4xx is {error, code}
    let (s, v) = call(&env.app, "GET", "/api/spaces/sp_nope/dev/jira", None).await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (404, Some("no_space")), "{v}");
    assert!(v["error"].is_string());
    let (s, v) = call(&env.app, "POST", &uri("jira/on"), Some(json!({"on": "yes"}))).await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (400, Some("bad_request")), "{v}");
    let (s, v) = call(&env.app, "POST", &uri("tickets/TRAIL-999/ask/preview"), Some(json!({"questions": "not a list"}))).await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (400, Some("bad_request")), "a body of the wrong shape: {v}");
    assert!(v["error"].is_string(), "{v}");
    let (s, v) = call(&env.app, "GET", &uri("tickets/TRAIL-999"), None).await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (404, Some("not_found")), "{v}");
    let key = tickets[0]["key"].as_str().unwrap().to_string();
    let (s, v) = call(&env.app, "POST", &uri(&format!("tickets/{key}/ask/preview")), Some(json!({"questions": [{"text": "a?"}]}))).await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (409, Some("wrong_state")), "{v}");
    let (s, _) = call(&env.app, "POST", &uri(&format!("tickets/{key}/close")), None).await;
    assert_eq!(s.as_u16(), 200);
    let (s, v) = call(&env.app, "POST", &uri(&format!("tickets/{key}/start")), None).await;
    assert_eq!((s.as_u16(), v["code"].as_str(), v["state"].as_str()), (409, Some("wrong_state"), Some(dev_jira::CLOSED)), "{v}");
    no_token_anywhere(&env, &m).await;
}

#[tokio::test]
async fn the_person_stops_skips_closes_and_retries_without_writing_to_jira() {
    let env = jira_setup().await;
    let m = MockAtlassian::cloud().await;
    let _g = gateway_to(&m);
    dev_mode_on(&env).await;
    m.add_project("TRAIL", "Trail app");
    let old = m.add_issue("TRAIL", "Labelled before connecting", &["kwant"], None);
    connect(&env, &m, json!({})).await;
    poll(&env).await;
    let new = m.add_issue("TRAIL", "Add dark mode", &["kwant"], None);
    poll(&env).await;
    assert_eq!(ticket(&env, &new).await.state, dev_jira::PICKED_UP);
    let mv = |key: String, what: &'static str| {
        let app = env.app.clone();
        async move { call(&app, "POST", &uri(&format!("tickets/{key}/{what}")), None).await }
    };

    // Stop: taken back, held; the poller doesn't pick it up again while it still matches
    let (s, v) = mv(new.clone(), "stop").await;
    assert_eq!((s.as_u16(), v["ticket"]["state"].as_str(), v["ticket"]["held_by_person"].as_bool()), (200, Some(dev_jira::TAKEN_BACK), Some(true)), "{v}");
    assert!(v["ticket"].get("work").is_some(), "the answer is the row with work: {v}");
    let e = events(&env, &new).await;
    assert!(e.contains(&"state:stopped by you".to_string()) && e.contains(&"hook:stopped: taken_back".to_string()), "{e:?}");
    poll(&env).await;
    assert_eq!(ticket(&env, &new).await.state, dev_jira::TAKEN_BACK, "a stopped ticket stays stopped");
    let (s, v) = mv(new.clone(), "stop").await;
    assert_eq!((s.as_u16(), v["code"].as_str(), v["state"].as_str()), (409, Some("wrong_state"), Some(dev_jira::TAKEN_BACK)), "{v}");

    // Retry: back in the queue (and started, a slot is free), no longer held
    let (s, v) = mv(new.clone(), "retry").await;
    assert_eq!((s.as_u16(), v["ticket"]["state"].as_str(), v["ticket"]["held_by_person"].as_bool(), v["started"].as_bool()), (200, Some(dev_jira::PICKED_UP), Some(false), Some(true)), "{v}");

    // Skip: closed as skipped, never picked up again even though it still matches
    let (s, v) = mv(new.clone(), "skip").await;
    assert_eq!((s.as_u16(), v["ticket"]["state"].as_str(), v["ticket"]["closed_resolution"].as_str()), (200, Some(dev_jira::CLOSED), Some("skipped")), "{v}");
    poll(&env).await;
    assert_eq!(ticket(&env, &new).await.state, dev_jira::CLOSED);
    let (s, _) = mv(new.clone(), "skip").await;
    assert_eq!(s.as_u16(), 409);
    // Retry from closed
    let (s, v) = mv(new.clone(), "retry").await;
    assert_eq!((s.as_u16(), v["ticket"]["state"].as_str(), v["ticket"]["closed_resolution"].clone()), (200, Some(dev_jira::PICKED_UP), Value::Null), "{v}");

    // Close: done in kwant only
    let (s, v) = mv(new.clone(), "close").await;
    assert_eq!((s.as_u16(), v["ticket"]["state"].as_str(), v["ticket"]["closed_resolution"].as_str()), (200, Some(dev_jira::CLOSED), Some("done")), "{v}");
    let (s, v) = mv(new.clone(), "close").await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (409, Some("wrong_state")), "{v}");
    assert_eq!(m.issue(&new).unwrap().status, "To Do", "Close changes nothing in Jira");

    // Skip works on a Waiting card; Stop doesn't (nothing is running)
    assert_eq!(ticket(&env, &old).await.state, dev_jira::WAITING);
    let (s, v) = mv(old.clone(), "stop").await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (409, Some("wrong_state")), "{v}");
    let (s, v) = mv(old.clone(), "skip").await;
    assert_eq!((s.as_u16(), v["ticket"]["closed_resolution"].as_str()), (200, Some("skipped")), "{v}");
    let (s, v) = mv("TRAIL-999".into(), "close").await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (404, Some("not_found")), "{v}");

    assert!(m.writes().is_empty() && m.refused().is_empty(), "no lifecycle button writes to Jira");
    no_token_anywhere(&env, &m).await;
}

#[tokio::test]
async fn a_ticket_at_work_shows_its_branch_pr_and_lead() {
    let env = jira_setup().await;
    let m = MockAtlassian::cloud().await;
    let _g = gateway_to(&m);
    dev_mode_on(&env).await;
    m.add_project("TRAIL", "Trail app");
    connect(&env, &m, json!({})).await;
    poll(&env).await;
    let key = m.add_issue("TRAIL", "Add dark mode", &["kwant"], None);
    poll(&env).await;
    let (_, v) = call(&env.app, "GET", &uri("tickets"), None).await;
    assert_eq!(v["tickets"][0]["work"], Value::Null, "no work before work starts: {v}");

    for to in [dev_jira::READING, dev_jira::BRIEF_READY, dev_jira::WORKING] {
        assert!(env.state.db.set_dev_ticket_state(&space(), &key, to).await.unwrap(), "{to}");
    }
    let facts = json!({"head_branch": format!("feat/{key}-dark-mode"), "title": "Dark mode", "links": {"pr": "https://github.com/acme/trail/pull/7"}});
    env.state.db.insert_dev_card(&space(), kwnt::db::dev_mode::REVIEW, "pr/7", "abc1234", &facts.to_string()).await.unwrap();
    let (s, v) = call(&env.app, "GET", &uri(&format!("tickets/{key}")), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let w = &v["ticket"]["work"];
    assert_eq!(w["branch"].as_str(), Some(format!("feat/{key}-dark-mode").as_str()), "{v}");
    assert_eq!((w["pr_url"].as_str(), w["head_sha"].as_str()), (Some("https://github.com/acme/trail/pull/7"), Some("abc1234")), "{v}");
    assert_eq!((w["builder_ids"].clone(), w["plan"].clone(), w["matches"].clone()), (json!([]), json!([]), Value::Null), "the stub seam has no mission: {v}");
    let (_, list) = call(&env.app, "GET", &uri("tickets"), None).await;
    assert_eq!(list["tickets"][0]["work"], *w, "the list and the ticket agree");
}
