//! Routing in a space of unrelated bots: Alex's "create a team to build a new startup…" went to
//! FitnessCoach because the message shared one everyday word with it and it led the space.
use kwnt::db::models::{Kwant, Team};
use kwnt::router::team::{is_team_building_request, pick_member, LayaGuess};
use kwnt::router::{heuristic_team_decision, LayaRouter, TeamDecision};

const ALEX: &str = "create a team to build a new startup. Analize what bots we need and do everythign needed";

fn bot(id: &str, name: &str, title: &str, persona: &str) -> Kwant {
    Kwant {
        space_id: kwnt::db::spaces::DEFAULT_SPACE_ID.into(),
        id: id.into(),
        name: name.into(),
        title: title.into(),
        persona_prompt: persona.into(),
        soul: String::new(),
        avatar_sprite: "s".into(),
        driver_type: "claude".into(),
        preferred_model: "auto".into(),
        temperature: 0.7,
        max_tokens: 4096,
        level: 1,
        turns_completed: 0,
        created_at: None,
    }
}

/// The space from the screenshot, FitnessCoach leading. Personas use everyday words the way real
/// ones do ("build strength", "builds APIs", "new habits").
fn space() -> Vec<Kwant> {
    vec![
        bot("fit", "FitnessCoach", "Fitness Trainer",
            "You are FitnessCoach. You build workout plans, help people build strength and start new habits they need."),
        bot("travel", "TravelPlanner", "Travel Planner",
            "You are TravelPlanner. You plan trips, compare flights and build itineraries for a new city."),
        bot("be", "Backend", "Backend Engineer",
            "You are Backend. You build APIs, services and databases and do code reviews."),
        bot("sre", "SRE", "Site Reliability Engineer",
            "You are SRE. You keep services up, build pipelines and handle deploys."),
        bot("fe", "Frontend", "Frontend Engineer",
            "You are Frontend. You build pages and components for a new web app."),
        bot("qa", "QA", "QA Engineer", "You are QA. You write tests and find bugs before a release."),
    ]
}

fn team() -> Team {
    Team {
        space_id: kwnt::db::spaces::DEFAULT_SPACE_ID.into(),
        id: "t".into(),
        name: "general".into(),
        description: None,
        workspace_path: None,
        repo_url: None,
        icon: "users".into(),
        lead_kwant_id: "fit".into(),
        member_kwants: "[]".into(),
        created_at: None,
        instructions: None,
    }
}

fn unsure_frontend(members: &[Kwant]) -> Option<LayaGuess<'_>> {
    Some(LayaGuess { kwant: &members[4], confident: false })
}

#[test]
fn alexs_message_is_a_team_building_request_for_the_lead() {
    let members = space();
    for laya in [None, unsure_frontend(&members)] {
        let pick = pick_member(ALEX, "fit", &members, laya, "channel lead").unwrap();
        assert_eq!((pick.kwant.id.as_str(), pick.basis), ("fit", "team"));
        assert!(pick.reason.starts_with("This asks to put a team of bots together"), "{}", pick.reason);
        assert!(pick.reason.contains("hire card"), "{}", pick.reason);
        assert!(!pick.reason.contains("shares words"), "{}", pick.reason);
    }
}

#[test]
fn a_team_request_settles_without_a_discussion() {
    let members = space();
    match heuristic_team_decision(ALEX, &team(), &members) {
        TeamDecision::Direct { kwant_id, basis, .. } => assert_eq!((kwant_id.as_str(), basis.as_str()), ("fit", "team")),
        other => panic!("expected the lead to take it, got {other:?}"),
    }
}

#[tokio::test]
async fn channel_route_for_alexs_message_states_the_intent() {
    let members = space();
    let laya = LayaRouter::new("http://127.0.0.1:9".into());
    let r = laya.route_channel_query_detailed(ALEX, "general", "fit", &members).await;
    assert_eq!((r.kwant_id.as_str(), r.basis.as_str()), ("fit", "team"));
    assert!(r.reason.contains("team of bots"), "{}", r.reason);
}

#[test]
fn team_building_requests_are_recognised() {
    for msg in [
        ALEX,
        "Create a team for my podcast",
        "hire a designer bot",
        "Hire a copywriter",
        "please recruit some agents to run marketing",
        "put together a small team of agents for SEO",
        "spin up 3 agents to research competitors",
        "assemble a crew to plan the launch",
        "what bots do we need for a newsletter?",
        "Build me a team that can ship a mobile app",
    ] {
        assert!(is_team_building_request(msg), "{msg}");
    }
}

#[test]
fn code_and_everyday_requests_that_mention_teams_are_not() {
    for msg in [
        "build the team page",
        "add a filter to the team members list",
        "create a team settings tab",
        "make the team picker accessible",
        "build a dashboard for the team",
        "fix the hire card layout",
        "set the team lead to Backend",
        "our team needs a faster deploy pipeline",
        "create a new endpoint for bots",
        "hello",
    ] {
        assert!(!is_team_building_request(msg), "{msg}");
    }
}

/// One everyday word shared with a persona is no reason to pick a bot, even the lead.
#[test]
fn a_weak_overlap_does_not_pick_a_bot() {
    let members = space();
    for msg in ["do everything needed to build something new", "can you start on this now", "build it"] {
        let pick = pick_member(msg, "fit", &members, None, "channel lead").unwrap();
        assert_eq!(pick.basis, "lead", "{msg}: {}", pick.reason);
        assert!(pick.reason.starts_with("No one's role matched this"), "{msg}: {}", pick.reason);
    }
    // with nothing in the words, even a clear Laya pick doesn't pick on its own: Laya sees only
    // titles (it sent "Best place to eat corn dogs in Munich tomorrow?" to a TikTok scriptwriter)
    let msg = "do everything needed to build something new";
    let sure = Some(LayaGuess { kwant: &members[2], confident: true });
    let pick = pick_member(msg, "fit", &members, sure, "channel lead").unwrap();
    assert_eq!((pick.kwant.id.as_str(), pick.basis, pick.fit), ("fit", "lead", 0.0));
    assert_eq!(pick_member(msg, "fit", &members, unsure_frontend(&members), "channel lead").unwrap().basis, "lead");
}

/// Real matches still route: a title word, or more than one persona word.
#[test]
fn clear_matches_still_route_to_the_specialist() {
    let members = space();
    let cases = [
        ("plan my trips to Rome and compare flights", "travel"),
        ("a workout to build strength", "fit"),
        ("the api endpoint returns 500", "be"),
        ("FitnessCoach, what should I eat today?", "fit"),
        ("Backend, create a team of agents", "be"), // named wins over the intent
    ];
    for (msg, want) in cases {
        let pick = pick_member(msg, "fit", &members, None, "channel lead").unwrap();
        assert_eq!(pick.kwant.id, want, "{msg}: {}", pick.reason);
    }
}

/// A tie between bots is no pick either: it is not settled by who leads.
#[test]
fn a_tie_is_not_settled_by_who_leads() {
    let members = space();
    // "new" is generic now; "itineraries"/"components" are one persona word each
    let pick = pick_member("itineraries and components", "fit", &members, None, "channel lead").unwrap();
    assert_ne!(pick.reason, "FitnessCoach (Fitness Trainer) shares words with your message and leads here");
    assert!(!pick.reason.contains("leads here"), "{}", pick.reason);
}

/// Rigel's regressions (report §3): with Backend leading, a plain request for one specialist
/// fell to the lead after generic words stopped counting. One word only that bot has still wins.
#[test]
fn one_distinctive_word_reaches_the_specialist_whoever_leads() {
    let members = space();
    let cases = [
        ("log my meals and workouts", "fit"),
        ("create a workout schedule for the week", "fit"),
        ("start a new habit tracker", "fit"),
        ("book a flight and hotel", "travel"),
        ("create a bot that tracks flights", "travel"),
        ("plan a trip to Rome", "travel"),
    ];
    for lead in ["be", "fit", "fe"] {
        for (msg, want) in cases {
            let pick = pick_member(msg, lead, &members, None, "channel lead").unwrap();
            assert_eq!(pick.kwant.id, want, "lead {lead}, {msg}: {}", pick.reason);
            assert!(!is_team_building_request(msg), "{msg}");
        }
    }
}

/// "plan" is in Travel Planner's title and in FitnessCoach's "workout plans", so it says nothing
/// about which of them it is for; "workout" does.
#[test]
fn a_word_two_bots_share_does_not_beat_one_only_one_has() {
    let members = space();
    for lead in ["be", "fit", "travel"] {
        for msg in ["need a new workout plan", "create a new workout plan", "plan my workout routine"] {
            let pick = pick_member(msg, lead, &members, None, "channel lead").unwrap();
            assert_eq!(pick.kwant.id, "fit", "lead {lead}, {msg}: {}", pick.reason);
        }
    }
}

/// put/make/start go with a team in everyday use; without a bot word they are no hire request.
#[test]
fn everyday_team_phrases_get_no_hire_card() {
    let members = space();
    for (msg, want) in [
        ("put the team on the dashboard", Some("fe")),
        ("make the team happy", None),
        ("start the team standup notes", None),
    ] {
        assert!(!is_team_building_request(msg), "{msg}");
        let pick = pick_member(msg, "be", &members, None, "channel lead").unwrap();
        assert_ne!(pick.basis, "team", "{msg}: {}", pick.reason);
        if let Some(want) = want {
            assert_eq!(pick.kwant.id, want, "{msg}: {}", pick.reason);
        }
    }
    // with a bot word they still are
    for msg in ["put together a team of agents for SEO", "make a team of bots for my shop", "start a new team of agents"] {
        assert!(is_team_building_request(msg), "{msg}");
    }
}

fn team_led_by(lead: &str) -> Team {
    Team { lead_kwant_id: lead.into(), ..team() }
}

fn direct_to(decision: &TeamDecision) -> Option<&str> {
    match decision {
        TeamDecision::Direct { kwant_id, .. } => Some(kwant_id),
        _ => None,
    }
}

/// Rigel's R1 (gate2): "plan" made these a team discussion in which TravelPlanner spoke before
/// FitnessCoach. The decision itself, not only `pick_member`, now goes straight to the specialist.
#[test]
fn one_clear_specialist_skips_the_discussion() {
    let members = space();
    let cases = [
        ("need a new workout plan", "fit"),
        ("plan my workout routine", "fit"),
        ("I need a workout and meal plan", "fit"),
        ("plan a trip to Rome", "travel"),
    ];
    for lead in ["be", "fit"] {
        for (msg, want) in cases {
            let decision = heuristic_team_decision(msg, &team_led_by(lead), &members);
            assert_eq!(direct_to(&decision), Some(want), "lead {lead}, {msg}: {decision:?}");
        }
    }
}

/// Naming two bots still asks for their views together.
#[test]
fn naming_two_bots_still_starts_a_discussion() {
    let members = space();
    for lead in ["be", "fit"] {
        let msg = "FitnessCoach and TravelPlanner, plan my workout routine for the trip";
        match heuristic_team_decision(msg, &team_led_by(lead), &members) {
            TeamDecision::Discuss { speakers, .. } => {
                assert!(speakers.iter().any(|s| s == "fit") && speakers.iter().any(|s| s == "travel"), "lead {lead}: {speakers:?}");
            }
            other => panic!("lead {lead}: {other:?}"),
        }
    }
}

/// Rigel's R2: a discussion request nothing in the message matches goes to the lead alone instead
/// of pulling in whoever is listed first; a bot that shares no words never speaks.
#[test]
fn no_unrelated_bot_joins_a_discussion() {
    let members = space();
    for lead in ["be", "fit"] {
        let decision = heuristic_team_decision("review my pull request", &team_led_by(lead), &members);
        if let TeamDecision::Discuss { speakers, .. } = &decision {
            for s in speakers.iter().skip(1) {
                let m = members.iter().find(|m| &m.id == s).unwrap();
                assert!(!["fit", "travel"].contains(&m.id.as_str()), "lead {lead}: {} has nothing to add: {decision:?}", m.name);
            }
        }
        if lead == "fit" {
            // Backend's persona mentions code reviews; a fitness lead with no word in common answers or hands on
            assert!(!matches!(&decision, TeamDecision::Discuss { speakers, .. } if speakers.len() > 1 && speakers[1] == "fit"), "{decision:?}");
        }
        // With no shared words, only an engineering-role bot may be added
        let decision = heuristic_team_decision("what are the tradeoffs here", &team_led_by(lead), &members);
        if let TeamDecision::Discuss { speakers, .. } = &decision {
            assert!(!speakers.iter().skip(1).any(|s| s == "fit" || s == "travel"), "lead {lead}: {decision:?}");
        }
    }
    // A space with no engineering roles: the lead answers alone instead of pulling in a stranger
    let lifestyle: Vec<Kwant> = members.iter().filter(|m| m.id == "fit" || m.id == "travel").cloned().collect();
    for lead in ["fit", "travel"] {
        let decision = heuristic_team_decision("what are the tradeoffs here", &team_led_by(lead), &lifestyle);
        assert_eq!(direct_to(&decision), Some(lead), "lead {lead}: {decision:?}");
    }
}

/// Rigel's R2: "find" is an everyday verb, so QA's "find bugs" no longer ties with TravelPlanner's
/// "flights".
#[test]
fn find_cheap_flights_goes_to_the_travel_planner() {
    let members = space();
    for lead in ["be", "fit", "qa"] {
        let pick = pick_member("find cheap flights to Rome", lead, &members, None, "channel lead").unwrap();
        assert_eq!(pick.kwant.id, "travel", "lead {lead}: {}", pick.reason);
        let decision = heuristic_team_decision("find cheap flights to Rome", &team_led_by(lead), &members);
        assert_eq!(direct_to(&decision), Some("travel"), "lead {lead}: {decision:?}");
    }
    // QA still gets its own work
    let pick = pick_member("find the flaky tests", "be", &members, None, "channel lead").unwrap();
    assert_eq!(pick.kwant.id, "qa", "{}", pick.reason);
}
