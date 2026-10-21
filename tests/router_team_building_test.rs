//! Routing in a space of unrelated bots: Oleks's "create a team to build a new startup…" went to
//! FitnessCoach because the message shared one everyday word with it and it led the space.
use kwnt::db::models::{Kwant, Team};
use kwnt::router::team::{is_team_building_request, pick_member, LayaGuess};
use kwnt::router::{heuristic_team_decision, LayaRouter, TeamDecision};

const OLEKS: &str = "create a team to build a new startup. Analize what bots we need and do everythign needed";

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
fn olekss_message_is_a_team_building_request_for_the_lead() {
    let members = space();
    for laya in [None, unsure_frontend(&members)] {
        let pick = pick_member(OLEKS, "fit", &members, laya, "channel lead").unwrap();
        assert_eq!((pick.kwant.id.as_str(), pick.basis), ("fit", "team"));
        assert!(pick.reason.starts_with("This asks to put a team of bots together"), "{}", pick.reason);
        assert!(pick.reason.contains("hire card"), "{}", pick.reason);
        assert!(!pick.reason.contains("shares words"), "{}", pick.reason);
    }
}

#[test]
fn a_team_request_settles_without_a_discussion() {
    let members = space();
    match heuristic_team_decision(OLEKS, &team(), &members) {
        TeamDecision::Direct { kwant_id, basis, .. } => assert_eq!((kwant_id.as_str(), basis.as_str()), ("fit", "team")),
        other => panic!("expected the lead to take it, got {other:?}"),
    }
}

#[tokio::test]
async fn channel_route_for_olekss_message_states_the_intent() {
    let members = space();
    let laya = LayaRouter::new("http://127.0.0.1:9".into());
    let r = laya.route_channel_query_detailed(OLEKS, "general", "fit", &members).await;
    assert_eq!((r.kwant_id.as_str(), r.basis.as_str()), ("fit", "team"));
    assert!(r.reason.contains("team of bots"), "{}", r.reason);
}

#[test]
fn team_building_requests_are_recognised() {
    for msg in [
        OLEKS,
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
    // with nothing in the words, a clear Laya pick still wins, and an unsure one does not
    let msg = "do everything needed to build something new";
    let sure = Some(LayaGuess { kwant: &members[2], confident: true });
    assert_eq!(pick_member(msg, "fit", &members, sure, "channel lead").unwrap().kwant.id, "be");
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
