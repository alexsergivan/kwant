//! quiz-guest-phone-plays: the guest phone's quiz fakes come from the engine itself.
//!
//! One short teams quiz is played through `QuizGame`, and the guest view (`QuizGame::view` for a
//! guest) is kept at every phase the phone draws, with the public lobby (`QuizGame::lobby`) that
//! `info` sends. The result must equal `frontend/guest/fake/quiz-views.json`, which the guest
//! page's tests play a quiz on (tests/guestPage.test.mjs, "quiz on the engine's own views"). When
//! the engine's view changes, this test fails until the file is written again:
//!
//!     KWNT_WRITE_GUEST_FAKES=1 cargo test --test quiz_guest_fakes_test
//!
//! so the phone's fakes can't drift from what a real box sends.

use chrono::{DateTime, Duration, Utc};
use kwnt::kits::quiz::engine::QuizGame;
use kwnt::kits::quiz::validate;
use kwnt::kits::{Actor, Viewer};
use serde_json::{json, Map, Value};

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/frontend/guest/fake/quiz-views.json");

fn at(s: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000, 0).unwrap() + Duration::seconds(s)
}

fn src(url: &str, quote: &str) -> Value {
    json!({"url": url, "quote": quote})
}

/// Two rounds, one question of each kind, teams of up to 4 made by the guests.
fn spec() -> Value {
    json!({
        "version": 1, "kit": "quiz", "title": "Friday Pub Quiz", "when": "2026-11-27T20:00:00+00:00",
        "teams": {"mode": "teams", "size": 4},
        "settings": {"shuffle_options": false, "lock_when_all_answered": false},
        "lines": {"intro": "Evening, quizzers! Phones out, names in.", "reveal": ["Let's see who knew that one."], "final": "That's the lot! Give it up for our winners."},
        "rounds": [
            {"title": "General knowledge", "questions": [
                {"kind": "choice", "text": "Which band released \"Definitely Maybe\" in 1994?", "options": ["Blur", "Oasis", "Pulp", "Suede"],
                 "answer": "Oasis", "points": 2, "time_limit_s": 20,
                 "source": src("https://en.wikipedia.org/wiki/Definitely_Maybe", "the debut studio album by English rock band Oasis")},
                {"kind": "text", "text": "What is the capital city of Australia?", "answer": "Canberra", "points": 2, "time_limit_s": 30,
                 "source": src("https://en.wikipedia.org/wiki/Canberra", "Canberra is the capital city of Australia")}
            ]},
            {"title": "Numbers", "questions": [
                {"kind": "number", "text": "How many bones are in an adult human body?", "answer": 206, "points": 3, "time_limit_s": 30,
                 "source": src("https://en.wikipedia.org/wiki/Human_skeleton", "The adult human skeleton consists of 206 bones")}
            ]}
        ]
    })
}

fn guest(g: &QuizGame, id: &str, now: DateTime<Utc>) -> Value {
    serde_json::to_value(g.view(&Viewer::Guest(id.to_string()), now)).unwrap()
}

fn act(g: &mut QuizGame, who: Actor, name: &str, body: Value, now: DateTime<Utc>) {
    g.act(&who, name, &body, now).unwrap_or_else(|e| panic!("{name} {body}: {e:?}"));
}

fn answer(g: &mut QuizGame, id: &str, q: usize, value: Value, now: DateTime<Utc>) {
    act(g, Actor::Guest(id.into()), "answer", json!({"q": q, "value": value}), now);
}

fn next(g: &mut QuizGame, now: DateTime<Utc>) {
    act(g, Actor::Host, "next", json!({}), now);
}

/// Every view the phone draws, by name: `{viewer, view}` (the viewer is a player id from `players`).
fn play() -> Value {
    let spec = validate::parse(&spec().to_string()).expect("the spec is valid");
    let mut g = QuizGame::new(spec, 7, at(0));
    let players = json!([
        {"id": "mia", "name": "Mia", "avatar": "visor", "team": "Quizzly Bears"},
        {"id": "sam", "name": "Sam", "avatar": "antenna", "team": "Quizzly Bears"},
        {"id": "ana", "name": "Ana", "avatar": "bulb", "team": "Fact Hunters"}
    ]);
    for p in players.as_array().unwrap() {
        let id = p["id"].as_str().unwrap();
        g.join(id, p["name"].as_str().unwrap(), at(0)).unwrap();
        act(&mut g, Actor::Guest(id.into()), "team", json!({"name": p["team"]}), at(0));
    }
    let mut views = Map::new();
    let mut keep = |name: &str, viewer: &str, view: Value| {
        views.insert(name.into(), json!({"viewer": viewer, "view": view}));
    };
    keep("lobby", "mia", guest(&g, "mia", at(1)));

    // Round 1, question 1 (choice): Mia right, Ana wrong, Sam no answer.
    next(&mut g, at(10));
    keep("question_choice", "mia", guest(&g, "mia", at(14)));
    answer(&mut g, "mia", 0, json!(1), at(15));
    keep("answered_choice", "mia", guest(&g, "mia", at(15)));
    answer(&mut g, "ana", 0, json!(0), at(16));
    next(&mut g, at(20));
    keep("locked", "mia", guest(&g, "mia", at(20)));
    next(&mut g, at(21));
    keep("reveal_right", "mia", guest(&g, "mia", at(21)));
    keep("reveal_wrong", "ana", guest(&g, "ana", at(21)));
    keep("reveal_no_answer", "sam", guest(&g, "sam", at(21)));

    // Question 2 (text): Mia's near miss waits for the judge, then counts.
    next(&mut g, at(30));
    keep("question_text", "mia", guest(&g, "mia", at(33)));
    answer(&mut g, "mia", 1, json!("Canbera"), at(35));
    answer(&mut g, "sam", 1, json!("Canberra"), at(36));
    answer(&mut g, "ana", 1, json!("Sydney"), at(37));
    keep("answered_text", "mia", guest(&g, "mia", at(37)));
    next(&mut g, at(40));
    next(&mut g, at(41));
    keep("reveal_judging", "mia", guest(&g, "mia", at(41)));
    act(&mut g, Actor::Host, "judge", json!({"q": 1, "player": "mia", "verdict": "yes"}), at(42));
    keep("reveal_judged", "mia", guest(&g, "mia", at(42)));

    // End of round 1: the scores.
    next(&mut g, at(45));
    keep("leaderboard", "mia", guest(&g, "mia", at(45)));

    // Round 2, question 3 (number).
    next(&mut g, at(50));
    keep("question_number", "mia", guest(&g, "mia", at(52)));
    answer(&mut g, "mia", 2, json!(206), at(55));
    answer(&mut g, "ana", 2, json!(200), at(55));
    next(&mut g, at(60));
    next(&mut g, at(61));
    keep("reveal_number", "mia", guest(&g, "mia", at(61)));
    while g.phase() != kwnt::kits::quiz::engine::Phase::Final {
        next(&mut g, at(70));
    }
    keep("final", "mia", guest(&g, "mia", at(70)));

    json!({
        "about": "Written by tests/quiz_guest_fakes_test.rs from QuizGame::view (guest) and QuizGame::lobby. Do not edit by hand: KWNT_WRITE_GUEST_FAKES=1 cargo test --test quiz_guest_fakes_test",
        "code": "quiz",
        "title": "Friday Pub Quiz",
        "players": players,
        "info": serde_json::to_value(g_lobby_before_start()).unwrap(),
        "views": views,
    })
}

/// The public lobby `info` sends before anyone starts (teams with their colours).
fn g_lobby_before_start() -> Value {
    let spec = validate::parse(&spec().to_string()).unwrap();
    let mut g = QuizGame::new(spec, 7, at(0));
    for (id, name, team) in [("mia", "Mia", "Quizzly Bears"), ("sam", "Sam", "Quizzly Bears"), ("ana", "Ana", "Fact Hunters")] {
        g.join(id, name, at(0)).unwrap();
        act(&mut g, Actor::Guest(id.into()), "team", json!({"name": team}), at(0));
    }
    serde_json::to_value(g.lobby()).unwrap()
}

#[test]
fn the_guest_phone_fakes_are_the_engine_views() {
    let got = play();
    // The play went the way the fakes say.
    let v = &got["views"];
    assert_eq!(v["reveal_right"]["view"]["me"]["result"]["verdict"], "correct");
    assert_eq!(v["reveal_wrong"]["view"]["me"]["result"]["verdict"], "wrong");
    assert_eq!(v["reveal_no_answer"]["view"]["me"]["result"]["verdict"], "no_answer");
    assert_eq!(v["reveal_judging"]["view"]["me"]["result"]["verdict"], "waiting_for_judge");
    assert_eq!(v["reveal_judged"]["view"]["me"]["result"]["verdict"], "correct");
    assert_eq!(v["leaderboard"]["view"]["phase"], "leaderboard");
    assert_eq!(v["final"]["view"]["phase"], "final");

    let text = serde_json::to_string_pretty(&got).unwrap() + "\n";
    if std::env::var_os("KWNT_WRITE_GUEST_FAKES").is_some() {
        std::fs::write(FILE, &text).expect("write quiz-views.json");
        return;
    }
    let have: Value = serde_json::from_str(&std::fs::read_to_string(FILE).expect("quiz-views.json is there")).expect("quiz-views.json is JSON");
    if have != got {
        // CI prints the file the engine wants, so it can be copied without a local build.
        eprintln!("----- quiz-views.json (engine) -----\n{text}----- end -----");
    }
    assert!(
        have == got,
        "frontend/guest/fake/quiz-views.json is not what the engine sends any more. Write it again with\n  KWNT_WRITE_GUEST_FAKES=1 cargo test --test quiz_guest_fakes_test\nthen check the guest page's tests still pass."
    );
}
