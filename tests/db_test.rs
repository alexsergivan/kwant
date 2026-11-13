use kwnt::db::{Database, models::*};
use tempfile::tempdir;

#[tokio::test]
async fn test_db_init_and_crud() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();

    let new_kwant = NewKwant {
        name: "PixelMage".into(),
        title: "Arcane Coder".into(),
        persona_prompt: "You are a master 8-bit wizard.".into(),
        soul: String::new(),
        avatar_sprite: "cyber_mage".into(),
        driver_type: "api".into(),
        preferred_model: "auto".into(),
    };

    let kwant = db.create_kwant(&kwnt::db::spaces::SpaceId::default_space(), &new_kwant).await.unwrap();
    assert_eq!(kwant.name, "PixelMage");

    let list = db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    // Default seeded kwants (3) + 1 newly created = 4
    assert!(list.iter().any(|k| k.name == "PixelMage"));
    assert!(list.len() >= 4);

    let fetched = db.get_kwant(&kwant.id).await.unwrap();
    assert!(fetched.is_some());
    assert_eq!(fetched.unwrap().name, "PixelMage");

    // Channels
    let new_chan = NewChannel {
        name: "tavern".into(),
        description: Some("Guild Tavern".into()),
        lead_kwant_id: kwant.id.clone(),
        member_kwants: vec![kwant.id.clone()],
        routing_mode: "auto".into(),
    };
    let chan = db.create_channel(&kwnt::db::spaces::SpaceId::default_space(), &new_chan).await.unwrap();
    assert_eq!(chan.name, "tavern");

    let channels = db.list_channels(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    assert!(!channels.is_empty());

    // Messages
    let new_msg = NewMessage {
        channel_id: Some(chan.id.clone()),
        direct_kwant_id: None,
        sender_role: "user".into(),
        sender_id: "player1".into(),
        text: "Hello tavern!".into(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 12,
    };
    let msg = db.insert_message(&new_msg).await.unwrap();
    assert_eq!(msg.text, "Hello tavern!");

    let msgs = db.list_messages(&chan.id, 10).await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].text, "Hello tavern!");

    // A limit keeps the latest messages, still in chronological order
    for text in ["second", "third"] {
        db.insert_message(&NewMessage { text: text.into(), ..new_msg.clone() }).await.unwrap();
    }
    let latest: Vec<String> = db.list_messages(&chan.id, 2).await.unwrap().into_iter().map(|m| m.text).collect();
    assert_eq!(latest, vec!["second", "third"]);

    // Editing a kwant renames its past messages too
    db.insert_message(&NewMessage {
        sender_role: "kwant".into(),
        sender_id: kwant.name.clone(),
        text: "bot reply".into(),
        ..new_msg.clone()
    })
    .await
    .unwrap();
    let edited = db
        .update_kwant(
            &kwant.id,
            &UpdateKwant {
                name: "Renamed".into(),
                title: "New Title".into(),
                persona_prompt: "New persona".into(),
                soul: "Cheerful and blunt.".into(),
                avatar_sprite: "security".into(),
                preferred_model: "claude".into(),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!((edited.name.as_str(), edited.title.as_str(), edited.preferred_model.as_str()), ("Renamed", "New Title", "claude"));
    assert_eq!(edited.soul, "Cheerful and blunt.");
    let msgs = db.list_messages(&chan.id, 10).await.unwrap();
    assert_eq!(msgs.last().unwrap().sender_id, "Renamed");
    assert!(db.update_kwant("missing", &UpdateKwant {
        name: "x".into(), title: "".into(), persona_prompt: "".into(), soul: String::new(), avatar_sprite: "".into(), preferred_model: "auto".into(),
    }).await.unwrap().is_none());

    // Approvals
    let new_app = NewApproval {
        turn_id: "turn-123".into(),
        kwant_id: kwant.id.clone(),
        tool_name: "execute_shell".into(),
        parameters: serde_json::json!({"command": "ls -la"}),
    };
    let app = db.save_approval(&new_app).await.unwrap();
    assert_eq!(app.status, "pending");

    db.update_approval_status(&app.id, "allowed").await.unwrap();
    let updated = db.get_approval(&app.id).await.unwrap().unwrap();
    assert_eq!(updated.status, "allowed");
}

#[tokio::test]
async fn test_team_crud_and_seed() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db = Database::init(temp_dir.path()).await.unwrap();

    let teams = db.list_teams(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    assert!(!teams.is_empty(), "Default seed team should exist");
    assert_eq!(teams[0].name, "helpers");

    let kwants = db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    let new_team = NewTeam {
        name: "infra-sec".into(),
        description: Some("Security and cloud infrastructure".into()),
        icon: Some("shield".into()),
        lead_kwant_id: kwants[0].id.clone(),
        member_kwants: vec![kwants[0].id.clone(), kwants[1].id.clone()],
        workspace_path: None,
        repo_url: None,
    };
    let created = db.create_team(&kwnt::db::spaces::SpaceId::default_space(), &new_team).await.unwrap();
    assert_eq!(created.name, "infra-sec");

    let fetched = db.get_team_by_name(&kwnt::db::spaces::SpaceId::default_space(), "infra-sec").await.unwrap();
    assert!(fetched.is_some());
    assert_eq!(fetched.unwrap().id, created.id);

    let by_id = db.get_team(&created.id).await.unwrap();
    assert!(by_id.is_some());
    assert_eq!(by_id.unwrap().name, "infra-sec");

    let channels = db.list_channels(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    assert!(channels.iter().any(|c| c.name == "team-infra-sec"));
    assert!(channels.iter().any(|c| c.name == "team-helpers"));

    let deleted = db.delete_team(&created.id).await.unwrap();
    assert!(deleted);
    let after_delete = db.get_team(&created.id).await.unwrap();
    assert!(after_delete.is_none());
}

#[tokio::test]
async fn test_team_with_workspace_and_repo() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db = Database::init(temp_dir.path()).await.unwrap();

    let kwants = db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    let new_team = NewTeam {
        name: "dev-squad".into(),
        description: Some("Frontend and backend app development".into()),
        icon: Some("code".into()),
        lead_kwant_id: kwants[0].id.clone(),
        member_kwants: vec![kwants[0].id.clone()],
        workspace_path: Some("/Users/dev/my-project".into()),
        repo_url: Some("https://github.com/my-org/my-project.git".into()),
    };
    let created = db.create_team(&kwnt::db::spaces::SpaceId::default_space(), &new_team).await.unwrap();
    assert_eq!(created.workspace_path.as_deref(), Some("/Users/dev/my-project"));
    assert_eq!(created.repo_url.as_deref(), Some("https://github.com/my-org/my-project.git"));

    // Update team workspace and repo
    let update_team = NewTeam {
        name: "dev-squad".into(),
        description: Some("Updated description".into()),
        icon: Some("code".into()),
        lead_kwant_id: kwants[0].id.clone(),
        member_kwants: vec![kwants[0].id.clone()],
        workspace_path: Some("/Users/dev/new-location".into()),
        repo_url: None,
    };
    let updated = db.update_team(&created.id, &update_team).await.unwrap().unwrap();
    assert_eq!(updated.workspace_path.as_deref(), Some("/Users/dev/new-location"));
    assert_eq!(updated.repo_url, None);
}



/// The three bots releases before 0.9.8 seeded, as they seeded them: (name, title, persona, soul, avatar).
const LEGACY: [(&str, &str, &str, &str, &str); 3] = [
    ("Architect", "Systems Architect", "You are the Architect, an expert in deep systems architecture, software design, and complex multi-file engineering. You give structured, thorough, and highly insightful technical advice.", "Calm, deliberate, and a little philosophical about software. You think in diagrams and long time horizons, value simplicity that survives change, and distrust cleverness. You speak in measured, well-structured sentences, often naming the trade-off before the answer, and you are quietly delighted by a clean abstraction.", "architect"),
    ("TechLead", "Staff Engineer & Prototyper", "You are the Tech Lead, a sharp, pragmatic engineer who excels in fast reasoning, rapid prototyping, and clear technical communication.", "Energetic, direct, and allergic to over-engineering. You would rather ship a small working thing today than a perfect plan next month. You keep replies short, use plain words, and are generous with credit to teammates. Your catchphrase-level instinct is \"what is the smallest step that proves it?\"", "tech_lead"),
    ("DevOps", "Systems & Security Engineer", "You are the DevOps Engineer, specializing in shell commands, server infrastructure, containerization, security policies, and automation.", "Steady, careful, and unflappable under pressure, like someone who has been paged at 3 a.m. and lived. You value reliability, least privilege, and being able to undo anything. Dry sense of humor; you always say what a command will do before running it and never pretend something is safe when it is not.", "devops"),
];

async fn run_sql(path: &std::path::Path, sql: &str) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}?mode=rwc", path.join("kwnt.db").display()))
        .await
        .unwrap();
    for stmt in sql.split(";\n").map(str::trim).filter(|s| !s.is_empty()) {
        sqlx::query(stmt).execute(&pool).await.unwrap_or_else(|e| panic!("{stmt}: {e}"));
    }
    pool.close().await;
}

fn q(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A data dir from before 0.9.8: an older release's schema holding the legacy starters (souls as
/// `soul`), #general, `extra` rows, and, with `team`, the core-engineering team and its chat.
async fn old_install(path: &std::path::Path, soul: impl Fn(&str) -> String, general: &str, team: bool, extra: &str) {
    run_sql(path, include_str!("fixtures/schema_c1e8b3a.sql")).await;
    let mut sql = String::new();
    for (i, (name, title, persona, _, avatar)) in LEGACY.iter().enumerate() {
        sql += &format!(
            "INSERT INTO kwants (id, name, title, persona_prompt, soul, avatar_sprite, created_at) VALUES ('k{i}', {}, {}, {}, {}, {}, '2026-01-0{} 10:00:00');\n",
            q(name), q(title), q(persona), q(&soul(name)), q(avatar), i + 1
        );
    }
    sql += &format!("INSERT INTO channels (id, name, description, lead_kwant_id, member_kwants) VALUES ('c-general', 'general', {}, 'k0', '[\"k0\",\"k1\",\"k2\"]');\n", q(general));
    if team {
        sql += "INSERT INTO teams (id, name, description, icon, lead_kwant_id, member_kwants) VALUES ('t-core', 'core-engineering', 'Core architectural, engineering, and infrastructure team', 'users', 'k0', '[\"k0\",\"k1\",\"k2\"]');\n";
        sql += "INSERT INTO channels (id, name, description, lead_kwant_id, member_kwants) VALUES ('c-team', 'team-core-engineering', 'Team chat', 'k0', '[\"k0\",\"k1\",\"k2\"]');\n";
    }
    sql += extra;
    run_sql(path, &sql).await;
}

/// Everything the seed could touch in the default space, as text.
async fn snapshot(db: &Database) -> String {
    let space = kwnt::db::spaces::SpaceId::default_space();
    let kwants = db.list_kwants(&space).await.unwrap();
    let mut scopes = Vec::new();
    for k in &kwants {
        scopes.push((k.name.clone(), db.get_scope(&k.id).await.unwrap()));
    }
    let teams = db.list_teams(&space).await.unwrap();
    let mut instructions = Vec::new();
    for t in &teams {
        instructions.push(db.team_instructions(&t.id).await.unwrap());
    }
    let channels = db.list_channels(&space).await.unwrap();
    format!("{kwants:#?}\n{scopes:#?}\n{teams:#?}\n{instructions:#?}\n{channels:#?}")
}

#[tokio::test]
async fn a_fresh_install_gets_the_helpers_team() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let space = kwnt::db::spaces::SpaceId::default_space();

    // first-screen design.md B: Assistant (lead), Researcher and Writer, from the helpers template
    let kwants = db.list_kwants(&space).await.unwrap();
    let got: Vec<_> = kwants.iter().map(|k| (k.name.as_str(), k.title.as_str(), k.avatar_sprite.as_str())).collect();
    assert_eq!(
        got,
        [("Assistant", "Personal Assistant", "product"), ("Researcher", "Research Analyst", "research"), ("Writer", "Writer & Editor", "writer")]
    );
    let template = kwnt::teams::templates::get("helpers").unwrap();
    for (k, b) in kwants.iter().zip(&template.bots) {
        assert_eq!((&k.persona_prompt, &k.soul), (&b.persona, &b.soul), "{}", k.name);
        let scope = db.get_scope(&k.id).await.unwrap();
        assert_eq!((scope.covers, scope.avoids), (b.good_for.clone(), b.not_for.clone()), "{}", k.name);
    }
    let assistant = &kwants[0];

    // The one team: @helpers, led by Assistant, with all three, the card line and the instructions
    let teams = db.list_teams(&space).await.unwrap();
    assert_eq!(teams.len(), 1);
    let helpers = &teams[0];
    assert_eq!((helpers.name.as_str(), helpers.icon.as_str()), ("helpers", "users"));
    assert_eq!(helpers.description.as_deref(), Some("Everyday help: plans, looks things up and writes."));
    assert_eq!(helpers.lead_kwant_id, assistant.id);
    let members: Vec<String> = serde_json::from_str(&helpers.member_kwants).unwrap();
    assert_eq!(members, kwants.iter().map(|k| k.id.clone()).collect::<Vec<_>>());
    assert_eq!(db.team_instructions(&helpers.id).await.unwrap().as_deref(), Some(template.instructions.trim()));
    assert!(db.get_team_by_name(&space, "core-engineering").await.unwrap().is_none());

    // #general (new wording, F) led by Assistant with everyone, and the team's own chat
    let channels = db.list_channels(&space).await.unwrap();
    let general = channels.iter().find(|c| c.name == "general").unwrap();
    assert_eq!(general.description.as_deref(), Some("Everyone in this space. The best fit answers."));
    assert_eq!(general.lead_kwant_id, assistant.id);
    let members: Vec<String> = serde_json::from_str(&general.member_kwants).unwrap();
    assert_eq!(members.len(), 3);
    assert!(channels.iter().any(|c| c.name == "team-helpers"));

    // Get started knows Helpers is already here; the other templates are not
    assert!(db.template_used_in_space(&space, "helpers").await.unwrap());
    assert!(!db.template_used_in_space(&space, "inbox").await.unwrap());

    // Starting again changes nothing
    let before = snapshot(&db).await;
    drop(db);
    let db = Database::init(tmp.path()).await.unwrap();
    assert_eq!(snapshot(&db).await, before);
}

#[tokio::test]
async fn an_old_install_keeps_its_starters_team_and_chats() {
    let tmp = tempdir().unwrap();
    // A 0.9.7-era default space: the legacy starters with souls, #general's 0.9.7 text, core-engineering,
    // and the person's own Writer and Assistant (names the new starters use, with their own personas)
    old_install(
        tmp.path(),
        |name| LEGACY.iter().find(|l| l.0 == name).unwrap().3.to_string(),
        "Everyone in this space. Laya picks who answers.",
        true,
        "INSERT INTO kwants (id, name, title, persona_prompt, avatar_sprite, created_at) VALUES ('k-own', 'Writer', 'My writer', 'You write my newsletter.', 'writer', '2026-02-01 10:00:00');\n\
         INSERT INTO kwants (id, name, title, persona_prompt, avatar_sprite, created_at) VALUES ('k-own2', 'Assistant', 'My assistant', 'You keep my calendar.', 'product', '2026-02-02 10:00:00');\n",
    )
    .await;
    let db = Database::init(tmp.path()).await.unwrap();
    let space = kwnt::db::spaces::SpaceId::default_space();

    let kwants = db.list_kwants(&space).await.unwrap();
    let got: Vec<_> = kwants.iter().map(|k| (k.id.as_str(), k.name.as_str(), k.title.as_str(), k.persona_prompt.as_str(), k.soul.as_str(), k.avatar_sprite.as_str())).collect();
    let mut want: Vec<_> = LEGACY.iter().enumerate().map(|(i, l)| (["k0", "k1", "k2"][i], l.0, l.1, l.2, l.3, l.4)).collect();
    want.push(("k-own", "Writer", "My writer", "You write my newsletter.", "", "writer"));
    want.push(("k-own2", "Assistant", "My assistant", "You keep my calendar.", "", "product"));
    assert_eq!(got, want);
    for k in &kwants {
        assert_eq!(db.get_scope(&k.id).await.unwrap(), Default::default(), "{} got a scope", k.name);
    }

    let teams = db.list_teams(&space).await.unwrap();
    let got: Vec<_> = teams.iter().map(|t| (t.id.as_str(), t.name.as_str(), t.description.as_deref(), t.lead_kwant_id.as_str(), t.member_kwants.as_str())).collect();
    assert_eq!(got, [("t-core", "core-engineering", Some("Core architectural, engineering, and infrastructure team"), "k0", "[\"k0\",\"k1\",\"k2\"]")]);
    assert_eq!(db.team_instructions("t-core").await.unwrap(), None);

    let channels = db.list_channels(&space).await.unwrap();
    let mut got: Vec<_> = channels.iter().map(|c| (c.id.as_str(), c.name.as_str(), c.description.as_deref(), c.lead_kwant_id.as_str())).collect();
    got.sort();
    assert_eq!(
        got,
        [("c-general", "general", Some("Everyone in this space. Laya picks who answers."), "k0"), ("c-team", "team-core-engineering", Some("Team chat"), "k0")]
    );
    // The person's own Assistant leads no team, so Helpers is still offered
    assert!(!db.template_used_in_space(&space, "helpers").await.unwrap());

    // seed_defaults again (a restart): byte for byte the same
    let before = snapshot(&db).await;
    drop(db);
    let db = Database::init(tmp.path()).await.unwrap();
    assert_eq!(snapshot(&db).await, before);
}

#[tokio::test]
async fn an_old_install_still_gets_its_pre_0_9_8_upgrades() {
    // The paths old installs take are unchanged: #general's oldest text becomes the 0.9.7 text (not
    // the new one), and a default space with bots but no team gets core-engineering led by Architect,
    // even when the person has their own bot called Assistant
    let tmp = tempdir().unwrap();
    old_install(
        tmp.path(),
        |name| LEGACY.iter().find(|l| l.0 == name).unwrap().3.to_string(),
        "Main collaboration hub for all agents",
        false,
        "INSERT INTO kwants (id, name, title, persona_prompt, avatar_sprite, created_at) VALUES ('k-own', 'Assistant', 'My assistant', 'You keep my calendar.', 'product', '2026-02-01 10:00:00');\n",
    )
    .await;
    let db = Database::init(tmp.path()).await.unwrap();
    let space = kwnt::db::spaces::SpaceId::default_space();
    let general = db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap();
    assert_eq!(general.description.as_deref(), Some("Everyone in this space. Laya picks who answers."));
    let teams = db.list_teams(&space).await.unwrap();
    assert_eq!(teams.len(), 1);
    assert_eq!((teams[0].name.as_str(), teams[0].lead_kwant_id.as_str()), ("core-engineering", "k0"));
    assert_eq!(teams[0].description.as_deref(), Some("Core architectural, engineering, and infrastructure team"));
    assert_eq!(serde_json::from_str::<Vec<String>>(&teams[0].member_kwants).unwrap(), ["k0", "k1", "k2", "k-own"]);
    assert_eq!(db.list_kwants(&space).await.unwrap().len(), 4, "no new bots");
}

#[tokio::test]
async fn a_new_install_that_lost_its_team_gets_helpers_back_not_core_engineering() {
    // A fresh install whose first start stopped after the bots (or whose person deleted the team):
    // the seeded lead still has the template's persona, so the team made is @helpers
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let space = kwnt::db::spaces::SpaceId::default_space();
    let team = db.get_team_by_name(&space, "helpers").await.unwrap().unwrap();
    assert!(db.delete_team(&team.id).await.unwrap());
    drop(db);
    let db = Database::init(tmp.path()).await.unwrap();
    let teams = db.list_teams(&space).await.unwrap();
    assert_eq!(teams.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["helpers"]);
    let lead = db.get_kwant(&teams[0].lead_kwant_id).await.unwrap().unwrap();
    assert_eq!(lead.name, "Assistant");
}

#[tokio::test]
async fn legacy_starter_souls_are_backfilled_and_user_edits_survive_restart() {
    // A pre-soul install: the legacy starters with no soul, TechLead's persona rewritten by the person,
    // and the person's own bots called Assistant and Researcher with no soul
    let tmp = tempdir().unwrap();
    old_install(
        tmp.path(),
        |_| String::new(),
        "Everyone in this space. Laya picks who answers.",
        true,
        "UPDATE kwants SET persona_prompt = 'My own persona' WHERE name = 'TechLead';\n\
         INSERT INTO kwants (id, name, title, persona_prompt, avatar_sprite) VALUES ('k-a', 'Assistant', 'Personal Assistant', 'Mine.', 'product');\n\
         INSERT INTO kwants (id, name, title, persona_prompt, avatar_sprite) VALUES ('k-r', 'Researcher', 'Research Analyst', 'Also mine.', 'research');\n",
    )
    .await;
    let db = Database::init(tmp.path()).await.unwrap();
    let after = db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    let soul = |name: &str| after.iter().find(|k| k.name == name).unwrap().soul.clone();
    // An untouched legacy starter without a soul (as in pre-soul databases) gets one back
    assert_eq!(soul("Architect"), LEGACY[0].3);
    assert_eq!(soul("DevOps"), LEGACY[2].3);
    // A starter whose persona the user rewrote is left alone
    assert!(soul("TechLead").is_empty());
    // A person's own bot named like a new starter is never touched
    assert!(soul("Assistant").is_empty() && soul("Researcher").is_empty());
    assert_eq!(after.len(), 5);
}

#[tokio::test]
async fn starter_souls_are_seeded_and_user_edits_survive_restart() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let starters = db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    assert_eq!(starters.len(), 3);
    assert!(starters.iter().all(|k| !k.soul.trim().is_empty()), "every starter has a soul");

    // The person clears Assistant's soul: a restart doesn't put it back (no backfill for new starters)
    let assistant = starters.iter().find(|k| k.name == "Assistant").unwrap();
    db.update_kwant(&assistant.id, &UpdateKwant {
        name: assistant.name.clone(),
        title: assistant.title.clone(),
        persona_prompt: assistant.persona_prompt.clone(),
        soul: String::new(),
        avatar_sprite: assistant.avatar_sprite.clone(),
        preferred_model: assistant.preferred_model.clone(),
    }).await.unwrap().unwrap();
    drop(db);

    let db = Database::init(tmp.path()).await.unwrap();
    let after = db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    assert!(after.iter().find(|k| k.name == "Assistant").unwrap().soul.is_empty());
    assert!(!after.iter().find(|k| k.name == "Writer").unwrap().soul.is_empty());
}
