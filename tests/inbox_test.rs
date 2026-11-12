//! 0.9.0 V1 Review inbox: `GET /api/inbox` joins `GET /api/away`'s items with tasks in review.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        missions::{NewCheck, NewEvidence, NewTask, TaskChange},
        models::{NewChannel, NewMessage},
        questions::QuestionPlace,
        spaces::SpaceId,
        threads::NewDelegation,
        verifier::NewVerdict,
        Database,
    },
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

async fn setup() -> (Router, AppState, TempDir) {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    // These chains predate keep-going turns; a stalled lead isn't nudged here.
    db.set_setting("keep_going_turns", "0").await.unwrap();
    let config = Config {
        data_dir: tmp.path().to_path_buf(),
        ..Config::default()
    };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    (create_router(state.clone()), state, tmp)
}

async fn raw(app: &Router, method: &str, uri: &str) -> (StatusCode, String) {
    let req = Request::builder().method(method).uri(uri).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn get(app: &Router, uri: &str) -> Value {
    let (s, body) = raw(app, "GET", uri).await;
    assert_eq!(s, StatusCode::OK, "{uri}: {body}");
    serde_json::from_str(&body).unwrap()
}

/// `GET /api/away`'s bytes with the one field that moves on its own (`now`) blanked.
async fn away_bytes(app: &Router) -> String {
    let (s, body) = raw(app, "GET", "/api/away").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    // `now` and `since` (now minus the lookback) move with the clock; everything else must match
    let mut body = body;
    for key in ["\"now\":\"", "\"since\":\""] {
        let start = body.find(key).unwrap_or_else(|| panic!("{key} in {body}")) + key.len();
        let end = start + body[start..].find('"').unwrap();
        body.replace_range(start..end, "");
    }
    body
}

async fn delegation(state: &AppState, lead: &str, spec: &str, title: &str, status: &str, text: &str) -> String {
    let (_, d) = state
        .db
        .create_delegation(&NewDelegation {
            lead_kwant_id: lead.into(),
            specialist_kwant_id: spec.into(),
            origin_channel_id: None,
            origin_kwant_id: Some(lead.into()),
            origin_thread_id: None,
            team_id: None,
            task: format!("do {title}"),
            title: title.into(),
            continue_lead: false,
            depth: 1,
            follow_up_at: None,
        })
        .await
        .unwrap();
    assert!(state.db.mark_delegation_running(&d.id).await.unwrap());
    if status == "completed" {
        let m = state
            .db
            .insert_message(&NewMessage {
                channel_id: None,
                direct_kwant_id: Some(spec.into()),
                sender_role: "kwant".into(),
                sender_id: spec.into(),
                text: text.into(),
                reasoning_content: None,
                tool_calls: None,
                tokens_used: 1,
            })
            .await
            .unwrap();
        assert!(state.db.finish_delegation(&d.id, "completed", None, Some(&m.id)).await.unwrap());
    } else {
        assert!(state.db.finish_delegation(&d.id, status, Some(text), None).await.unwrap());
    }
    d.id
}

fn change(actor_reason: Option<&str>) -> TaskChange {
    TaskChange { kind: "state".into(), reason: actor_reason.map(String::from), ..Default::default() }
}

/// A task of `owner` taken through `working → review`, sent back `backs` times on the way.
async fn review_task(state: &AppState, owner: &str, title: &str, acceptance: &str, channel: &str, backs: usize) -> String {
    let t = state
        .db
        .create_task(&SpaceId::default_space(), &NewTask {
            title: title.into(),
            acceptance: acceptance.into(),
            state: "ready".into(),
            owner_kwant_id: Some(owner.into()),
            max_attempts: 10,
            source_kind: "person".into(),
            channel_id: Some(channel.into()),
            created_by: "person".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let db = &state.db;
    // `working` needs a live lease, so each round starts with a claim
    db.claim_task(&t.id, owner, None, None, 600_000, false, owner).await.unwrap().unwrap();
    for n in 0..backs {
        assert!(db.move_task(&t.id, "working", "review", owner, &change(Some(&format!("done, round {}", n + 1)))).await.unwrap());
        assert!(db.move_task(&t.id, "review", "ready", "person", &change(Some(&format!("Also handle case {}", n + 1)))).await.unwrap());
        db.claim_task(&t.id, owner, None, None, 600_000, false, owner).await.unwrap().unwrap();
    }
    assert!(db.move_task(&t.id, "working", "review", owner, &change(Some("Handled the empty input too"))).await.unwrap());
    t.id
}

fn find<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["items"].as_array().unwrap().iter().find(|i| i["id"] == id).unwrap_or_else(|| panic!("{id} missing from {v}"))
}

fn has(v: &Value, id: &str) -> bool {
    v["items"].as_array().unwrap().iter().any(|i| i["id"] == id)
}

#[tokio::test]
async fn inbox_joins_away_and_review_tasks_with_fallbacks() {
    let (app, state, _tmp) = setup().await;
    let space = SpaceId::default_space();
    let kwants = state.db.list_kwants(&space).await.unwrap();
    let (lead, spec) = (kwants[0].clone(), kwants[1].clone());
    let chan = state
        .db
        .create_channel(&space, &NewChannel {
            name: "ops".into(),
            description: None,
            lead_kwant_id: lead.id.clone(),
            member_kwants: vec![lead.id.clone(), spec.id.clone()],
            routing_mode: "auto".into(),
        })
        .await
        .unwrap();

    // Empty
    let v = get(&app, "/api/inbox").await;
    assert_eq!(v["items"], json!([]));
    assert_eq!(v["batch"], Value::Null);
    assert_eq!(v["counts"], json!({"waiting_on_you": 0, "failed_or_stuck": 0, "to_review": 0, "finished": 0, "badge": 0}));
    assert_eq!(v["last_seen"], Value::Null);

    // Today's away items: a question, a finished and a failed delegation
    let place = QuestionPlace { kwant_id: spec.id.clone(), channel_id: Some(chan.id.clone()), ..Default::default() };
    let q = state.db.create_question(&place, "Ship tonight?", &["Yes".into(), "No".into()]).await.unwrap();
    let done = delegation(&state, &lead.id, &spec.id, "Write the brief", "completed", "Brief drafted.").await;
    let failed = delegation(&state, &lead.id, &spec.id, "Deploy", "failed", "401 from the host").await;
    let away_before = away_bytes(&app).await;

    // Review tasks: one checked by a teammate with its PR, one back from a send-back, one sent
    // back 3 times, and one still working (not listed)
    let t1 = review_task(&state, &spec.id, "Limit sign-in attempts", "- Five tries then a wait\n- No new dependencies", &chan.id, 0).await;
    let check = state
        .db
        .add_check(&NewCheck {
            task_id: t1.clone(),
            kind: "command".into(),
            spec: json!({"command": "cargo test login"}),
            required: true,
            approved: true,
            requested_by: "person".into(),
            approved_by: Some("person".into()),
        })
        .await
        .unwrap();
    state.db.finish_check(&check.id, "passed", Some("running 4 tests\ntest result: ok. 4 passed"), &lead.id, None).await.unwrap();
    state
        .db
        .add_evidence(&NewEvidence {
            task_id: t1.clone(),
            kind: "url".into(),
            label: "PR".into(),
            uri: Some("https://github.com/acme/app/pull/512".into()),
            captured_by: spec.id.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    let t2 = review_task(&state, &lead.id, "Nightly export to CSV", "1. Runs at 2am", &chan.id, 1).await;
    let t3 = review_task(&state, &spec.id, "Legend under the plot", "", &chan.id, 3).await;
    let working = state
        .db
        .create_task(&space, &NewTask { title: "Still going".into(), state: "ready".into(), owner_kwant_id: Some(lead.id.clone()), source_kind: "person".into(), created_by: "person".into(), ..Default::default() })
        .await
        .unwrap();
    state.db.claim_task(&working.id, &lead.id, None, None, 600_000, false, &lead.id).await.unwrap().unwrap();

    let v = get(&app, "/api/inbox").await;
    let items = v["items"].as_array().unwrap();
    assert!(!has(&v, &working.id), "a working task isn't listed");
    // Sorted across the groups; the 3rd send-back waits on the person
    let groups: Vec<&str> = items.iter().map(|i| i["group"].as_str().unwrap()).collect();
    assert_eq!(groups, ["waiting_on_you", "waiting_on_you", "failed_or_stuck", "to_review", "to_review", "finished"], "{v}");
    assert_eq!(find(&v, &q.id)["group"], "waiting_on_you");
    assert_eq!(find(&v, &t3)["group"], "waiting_on_you");
    assert_eq!(find(&v, &t3)["send_backs"], 3);
    assert_eq!(find(&v, &failed)["group"], "failed_or_stuck");
    assert_eq!(find(&v, &done)["group"], "finished");
    // Same group and risk: oldest in review first
    let review_ids: Vec<&str> = items.iter().filter(|i| i["group"] == "to_review").map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(review_ids, [t1.as_str(), t2.as_str()]);

    // Mira's fallbacks until the verifier lands
    let r1 = find(&v, &t1);
    assert_eq!(r1["kind"], "review");
    assert_eq!(r1["task_id"], t1.as_str());
    assert_eq!(r1["kwant_name"], spec.name.as_str());
    assert_eq!(r1["verdict"]["result"], "none");
    assert_eq!(r1["verdict"]["id"], Value::Null);
    assert_eq!(r1["verdict"]["line"], "1 of 1 required checks passed.");
    assert_eq!(r1["summary"], "1 of 1 required checks passed.");
    assert_eq!(r1["round"], json!({"n": 1, "max": 2}));
    assert_eq!(r1["risk"], json!({"level": null, "reasons": []}));
    assert_eq!(r1["criteria"].as_array().unwrap().len(), 2);
    assert_eq!(r1["criteria"][0], json!({"index": 1, "text": "Five tries then a wait", "status": "not_checked", "source": "task", "evidence": null, "note": null, "round_added": 1}));
    assert_eq!(r1["criteria_summary"], json!({"met": 0, "total": 2, "not_met": 0, "not_checked": 2}));
    assert_eq!(r1["couldnt_verify"], json!([]));
    assert_eq!(r1["decisions"], json!({"total": 0, "top": []}));
    assert_eq!(r1["evidence"]["checks"][0]["result"], "pass");
    assert_eq!(r1["evidence"]["checks"][0]["ran_by"], "checker");
    assert_eq!(r1["evidence"]["checks"][0]["command"], "cargo test login");
    assert_eq!(r1["evidence"]["pr"]["repo"], "acme/app");
    assert_eq!(r1["evidence"]["pr"]["number"], 512);
    assert_eq!(r1["held_actions"], json!([]));
    assert_eq!(r1["previous_round"], Value::Null);
    assert_eq!(r1["stale"], Value::Null);
    assert_eq!(r1["approve_error"], Value::Null);
    // No verdict: never one tap or batchable; no before_run checkpoint: no roll back
    assert_eq!((r1["one_tap"].clone(), r1["batchable"].clone(), r1["rollback_ok"].clone()), (json!(false), json!(false), json!(false)));
    assert_eq!(r1["link"]["channel_id"], chan.id.as_str());
    let key = r1["sort_key"].as_array().unwrap();
    assert_eq!(key.len(), 5);
    assert_eq!((key[0].clone(), key[1].clone(), key[3].clone(), key[4].clone()), (json!(3), json!(1), json!(spec.name), json!(t1)));

    // Back from a send-back: the person's note and the bot's summary, no flips or diff yet
    let r2 = find(&v, &t2);
    assert_eq!(r2["previous_round"]["note"], "Also handle case 1");
    assert_eq!(r2["previous_round"]["bot_summary"], "Handled the empty input too");
    assert_eq!(r2["previous_round"]["flips"], json!([]));
    assert_eq!(r2["previous_round"]["diff"], json!({"files": 0, "add": 0, "del": 0}));
    assert_eq!(r2["criteria"][0]["text"], "Runs at 2am");
    assert_eq!(r2["verdict"]["line"], "No required checks yet.");

    // Away items come through unchanged, with their group and key added
    let away = get(&app, "/api/away").await;
    for section in ["waiting", "finished", "failed"] {
        for a in away[section].as_array().unwrap() {
            let mut i = find(&v, a["id"].as_str().unwrap()).clone();
            let o = i.as_object_mut().unwrap();
            assert!(o.remove("group").is_some() && o.remove("sort_key").is_some());
            assert_eq!(&i, a);
        }
    }
    // Badge: today's unseen.total plus the three open review items
    assert_eq!(away["unseen"]["total"], 3);
    assert_eq!(v["counts"], json!({"waiting_on_you": 2, "failed_or_stuck": 1, "to_review": 2, "finished": 1, "badge": 6}));
    assert_eq!(v["batch"], Value::Null);

    // GET /api/away is byte for byte what it was before review tasks and the inbox existed
    assert_eq!(away_bytes(&app).await, away_before);

    // Marking seen clears finished and failed, never review items
    let (s, _) = raw(&app, "POST", "/api/away/seen").await;
    assert_eq!(s, StatusCode::OK);
    let v = get(&app, "/api/inbox").await;
    assert!(v["last_seen"].is_string());
    for id in [&t1, &t2, &t3, &q.id] {
        assert!(has(&v, id), "{id} cleared by seen: {v}");
    }
    assert!(!has(&v, &done) && !has(&v, &failed), "{v}");
    assert_eq!(v["counts"]["badge"], 4, "1 question + 3 review items: {v}");

    // since must parse
    let (s, body) = raw(&app, "GET", "/api/inbox?since=yesterday").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
}

/// The verifier's hold on a task (v2-v3-contract §3.4), as #502 adds it at the move to review.
async fn hold(state: &AppState, task_id: &str, requested_by: &str, spec: Value) -> String {
    state
        .db
        .add_check(&NewCheck {
            task_id: task_id.into(),
            kind: "peer_review".into(),
            spec,
            required: true,
            approved: true,
            requested_by: requested_by.into(),
            approved_by: Some("kwant".into()),
        })
        .await
        .unwrap()
        .id
}

/// D7 "Being checked": a task in review whose verifier hold is pending, with no verdict, has
/// `being_checked: true`; its verdict stays `none` and it is never one tap or batchable. A task
/// with no hold, a hold from an earlier review, a bot's own peer review or a settled hold has
/// `being_checked: false`.
#[tokio::test]
async fn being_checked_follows_the_verifiers_pending_hold() {
    let (app, state, _tmp) = setup().await;
    let space = SpaceId::default_space();
    let kwants = state.db.list_kwants(&space).await.unwrap();
    let (lead, spec) = (kwants[0].clone(), kwants[1].clone());
    let chan = state
        .db
        .create_channel(&space, &NewChannel {
            name: "ops".into(),
            description: None,
            lead_kwant_id: lead.id.clone(),
            member_kwants: vec![lead.id.clone(), spec.id.clone()],
            routing_mode: "auto".into(),
        })
        .await
        .unwrap();

    let held = review_task(&state, &spec.id, "Held by the verifier", "- Works", &chan.id, 0).await;
    hold(&state, &held, "verifier", json!({"verifier": true})).await;
    let plain = review_task(&state, &spec.id, "No hold", "- Works", &chan.id, 0).await;
    let bots = review_task(&state, &spec.id, "A bot's peer review", "- Works", &chan.id, 0).await;
    hold(&state, &bots, &lead.id, json!({})).await;
    let settled = review_task(&state, &spec.id, "Hold settled", "- Works", &chan.id, 0).await;
    let s = hold(&state, &settled, "verifier", json!({"verifier": true})).await;
    state.db.finish_check(&s, "passed", Some("verified"), "kwant", None).await.unwrap();
    // A hold from the first review; the task was sent back and is in review again
    let again = review_task(&state, &lead.id, "In review again", "- Works", &chan.id, 0).await;
    hold(&state, &again, "verifier", json!({"verifier": true})).await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert!(state.db.move_task(&again, "review", "ready", "person", &change(Some("Also the edge case"))).await.unwrap());
    state.db.claim_task(&again, &lead.id, None, None, 600_000, false, &lead.id).await.unwrap().unwrap();
    assert!(state.db.move_task(&again, "working", "review", &lead.id, &change(Some("Edge case done"))).await.unwrap());

    let v = get(&app, "/api/inbox").await;
    let h = find(&v, &held);
    assert_eq!(h["being_checked"], json!(true), "{h}");
    assert_eq!(h["verdict"]["result"], "none");
    assert_eq!((h["one_tap"].clone(), h["batchable"].clone()), (json!(false), json!(false)));
    for id in [&plain, &bots, &again] {
        assert_eq!(find(&v, id)["being_checked"], json!(false), "{id}: {}", find(&v, id));
    }
    // A passed hold may verify the task at once; if it is still listed, it isn't being checked
    if has(&v, &settled) {
        assert_eq!(find(&v, &settled)["being_checked"], json!(false));
    }
    // Every review item carries the field; away items don't
    for i in v["items"].as_array().unwrap() {
        assert_eq!(i["being_checked"].is_boolean(), i["kind"] == "review", "{i}");
    }
    assert_eq!(v["batch"], Value::Null, "nothing without a verdict is batched");
}

/// The verifier's verdict on `task_id`, as `verifier::drive` saves it.
async fn verdict(state: &AppState, task_id: &str, result: &str, confidence: &str, risk: &str, check_id: Option<&str>) -> String {
    let next = match result {
        "pass" => "counted",
        "fail" => "sent_back",
        _ => "held_for_you",
    };
    state
        .db
        .add_verdict(&NewVerdict {
            task_id: task_id.into(),
            attempt: 1,
            round: 1,
            verdict: result.into(),
            confidence: confidence.into(),
            risk_tier: risk.into(),
            risk_reasons: json!([format!("{risk} because of the test")]),
            steps: json!([{"status": "passed"}, {"status": "skipped"}]),
            findings: json!([]),
            couldnt_verify: json!([]),
            model: Some("gpt-5.5".into()),
            difference: "model".into(),
            check_id: check_id.map(String::from),
            next: next.into(),
            summary: format!("The verifier says {result}."),
            ..Default::default()
        })
        .await
        .unwrap()
        .id
}

/// M1: `/api/inbox` reads the verifier's verdict of this review round. A pass gives its result,
/// risk and one tap; a held needs_you waits on the person and isn't being checked; a pending hold
/// with no verdict still is; a pass from before a send-back is the last round's and doesn't count.
#[tokio::test]
async fn review_items_read_the_verifiers_verdict_of_this_round() {
    let (app, state, _tmp) = setup().await;
    let space = SpaceId::default_space();
    let kwants = state.db.list_kwants(&space).await.unwrap();
    let (lead, spec) = (kwants[0].clone(), kwants[1].clone());
    let chan = state
        .db
        .create_channel(&space, &NewChannel {
            name: "ops".into(),
            description: None,
            lead_kwant_id: lead.id.clone(),
            member_kwants: vec![lead.id.clone(), spec.id.clone()],
            routing_mode: "auto".into(),
        })
        .await
        .unwrap();

    let low = review_task(&state, &spec.id, "Low-risk pass", "- Works", &chan.id, 0).await;
    let low_v = verdict(&state, &low, "pass", "high", "low", None).await;
    let high = review_task(&state, &spec.id, "High-risk pass", "- Works", &chan.id, 0).await;
    verdict(&state, &high, "pass", "high", "high", None).await;
    let held = review_task(&state, &spec.id, "Held for you", "- Works", &chan.id, 0).await;
    let h = hold(&state, &held, "verifier", json!({"verifier": true})).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let held_v = verdict(&state, &held, "needs_you", "high", "high", Some(&h)).await;
    let checking = review_task(&state, &spec.id, "Being checked", "- Works", &chan.id, 0).await;
    hold(&state, &checking, "verifier", json!({"verifier": true})).await;
    // Passed in round 1, sent back by the person, in review again with no new verdict
    let again = review_task(&state, &lead.id, "Back in review", "- Works", &chan.id, 0).await;
    verdict(&state, &again, "pass", "high", "low", None).await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert!(state.db.move_task(&again, "review", "ready", "person", &change(Some("Also the edge case"))).await.unwrap());
    state.db.claim_task(&again, &lead.id, None, None, 600_000, false, &lead.id).await.unwrap().unwrap();
    assert!(state.db.move_task(&again, "working", "review", &lead.id, &change(Some("Edge case done"))).await.unwrap());

    let v = get(&app, "/api/inbox").await;
    let l = find(&v, &low);
    assert_eq!((l["group"].as_str(), l["verdict"]["result"].as_str(), l["verdict"]["id"].as_str()), (Some("to_review"), Some("pass"), Some(low_v.as_str())), "{l}");
    assert_eq!((l["verdict"]["confidence"].as_str(), l["risk"]["level"].as_str()), (Some("high"), Some("low")));
    assert_eq!(l["risk"]["reasons"], json!(["low because of the test"]));
    assert_eq!(l["verdict"]["line"], "The verifier says pass.");
    assert_eq!((l["verdict"]["steps_run"].as_i64(), l["verdict"]["steps_total"].as_i64()), (Some(1), Some(2)));
    assert_eq!(l["verdict"]["checker"]["engine"], "Codex");
    assert!(l["verdict"]["checked_at"].is_string());
    assert_eq!((l["one_tap"].clone(), l["being_checked"].clone()), (json!(true), json!(false)), "{l}");
    // Batching also needs every required check passed; this task has none
    assert_eq!(l["batchable"], json!(false));
    let hi = find(&v, &high);
    assert_eq!((hi["risk"]["level"].as_str(), hi["one_tap"].clone(), hi["batchable"].clone()), (Some("high"), json!(false), json!(false)), "{hi}");

    let n = find(&v, &held);
    assert_eq!((n["group"].as_str(), n["verdict"]["result"].as_str(), n["verdict"]["id"].as_str()), (Some("waiting_on_you"), Some("needs_you"), Some(held_v.as_str())), "{n}");
    assert_eq!((n["risk"]["level"].as_str(), n["being_checked"].clone()), (Some("high"), json!(false)), "{n}");
    assert_eq!((n["one_tap"].clone(), n["batchable"].clone()), (json!(false), json!(false)));
    assert!(v["counts"]["waiting_on_you"].as_i64().unwrap() >= 1, "{}", v["counts"]);

    let c = find(&v, &checking);
    assert_eq!((c["verdict"]["result"].as_str(), c["being_checked"].clone(), c["group"].as_str()), (Some("none"), json!(true), Some("to_review")), "{c}");

    let a = find(&v, &again);
    assert_eq!((a["verdict"]["result"].as_str(), a["verdict"]["id"].clone(), a["one_tap"].clone()), (Some("none"), Value::Null, json!(false)), "an earlier round's pass: {a}");
    assert_eq!(a["risk"]["level"], Value::Null);
}

/// M3: "What done looks like" reads the task's contract and this round's verdict. Pixel's shot had
/// "Pass · high confidence" over "0 of 3 met · 3 not checked": every line was hardcoded not checked.
#[tokio::test]
async fn review_criteria_read_the_contract_and_the_verdict() {
    let (app, state, _tmp) = setup().await;
    let space = SpaceId::default_space();
    let kwants = state.db.list_kwants(&space).await.unwrap();
    let (lead, spec) = (kwants[0].clone(), kwants[1].clone());
    let chan = state
        .db
        .create_channel(&space, &NewChannel {
            name: "ops".into(),
            description: None,
            lead_kwant_id: lead.id.clone(),
            member_kwants: vec![lead.id.clone(), spec.id.clone()],
            routing_mode: "auto".into(),
        })
        .await
        .unwrap();

    let t = review_task(&state, &spec.id, "Form errors", "- Error under the field\n- Works at 390px\n- Empty input", &chan.id, 0).await;
    let ev = state
        .db
        .add_evidence(&NewEvidence {
            task_id: t.clone(),
            kind: "screenshot".into(),
            label: "Form at 390px".into(),
            uri: Some("https://example.com/form.png".into()),
            captured_by: spec.id.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    let body = json!({"criteria": [
        {"id": "c1", "text": "Error under the field", "status": "met", "evidence": [ev.id], "by": "acceptance", "round_added": 1},
        {"id": "c2", "text": "Works at 390px", "status": "met", "evidence": [], "by": "acceptance", "round_added": 1},
        {"id": "c3", "text": "Empty input", "status": "not_met", "evidence": [], "by": "acceptance", "round_added": 1},
    ]});
    state.db.put_contract(&t, &body, &spec.id, &json!({"by": "owner"})).await.unwrap();
    state
        .db
        .add_verdict(&NewVerdict {
            task_id: t.clone(),
            attempt: 1,
            round: 1,
            verdict: "pass".into(),
            confidence: "high".into(),
            risk_tier: "low".into(),
            steps: json!([{"status": "passed"}]),
            next: "counted".into(),
            summary: "Two of three met.".into(),
            model: Some("gpt-5.5".into()),
            difference: "model".into(),
            criteria_status: json!([{"id": "c1", "status": "met", "claim": false}, {"id": "c2", "status": "met", "claim": true}, {"id": "c3", "status": "not_met", "claim": false}]),
            ..Default::default()
        })
        .await
        .unwrap();
    // No contract: the acceptance lines stay the fallback, not checked
    let plain = review_task(&state, &spec.id, "No contract", "- One\n- Two", &chan.id, 0).await;

    let v = get(&app, "/api/inbox").await;
    let r = find(&v, &t);
    assert_eq!(r["verdict"]["result"], "pass", "{r}");
    let got: Vec<(&str, &str, &str)> =
        r["criteria"].as_array().unwrap().iter().map(|c| (c["text"].as_str().unwrap(), c["status"].as_str().unwrap(), c["source"].as_str().unwrap())).collect();
    assert_eq!(got, [("Error under the field", "met", "task"), ("Works at 390px", "claim_only", "task"), ("Empty input", "not_met", "task")]);
    assert_eq!(r["criteria"][0]["evidence"], json!({"kind": "screenshot", "ref": "https://example.com/form.png", "label": "Form at 390px"}));
    assert_eq!(r["criteria"][1]["evidence"], Value::Null);
    assert_eq!(r["criteria_summary"], json!({"met": 2, "total": 3, "not_met": 1, "not_checked": 0}));

    let p = find(&v, &plain);
    assert_eq!(p["criteria"][1], json!({"index": 2, "text": "Two", "status": "not_checked", "source": "task", "evidence": null, "note": null, "round_added": 1}));
    assert_eq!(p["criteria_summary"], json!({"met": 0, "total": 2, "not_met": 0, "not_checked": 2}));
}
