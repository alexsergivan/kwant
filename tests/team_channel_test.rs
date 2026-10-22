//! A team's chat is the channel `team-<team name>`; these keep the two linked.
use kwnt::db::spaces::SpaceId;
use kwnt::db::{models::*, Database, DbError};
use tempfile::tempdir;

fn team(name: &str, lead: &str, members: &[&str]) -> NewTeam {
    NewTeam {
        name: name.into(),
        description: Some("Hired together".into()),
        icon: None,
        lead_kwant_id: lead.into(),
        member_kwants: members.iter().map(|m| m.to_string()).collect(),
        workspace_path: None,
        repo_url: None,
    }
}

fn said(channel_id: &str, text: &str) -> NewMessage {
    NewMessage {
        channel_id: Some(channel_id.into()),
        direct_kwant_id: None,
        sender_role: "user".into(),
        sender_id: "person".into(),
        text: text.into(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    }
}

async fn channel_named(db: &Database, name: &str) -> Option<Channel> {
    db.list_channels(&SpaceId::default_space()).await.unwrap().into_iter().find(|c| c.name == name)
}

#[tokio::test]
async fn renaming_a_team_renames_its_chat_and_keeps_history() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let space = SpaceId::default_space();
    let ks = db.list_kwants(&space).await.unwrap();
    let (a, b) = (ks[0].id.as_str(), ks[1].id.as_str());

    let t = db.create_team(&space, &team("Startup", a, &[a])).await.unwrap();
    let chat = channel_named(&db, "team-Startup").await.unwrap();
    db.insert_message(&said(&chat.id, "before the rename")).await.unwrap();

    // Startup -> startup is what Manage Team's save did to a hired team's name.
    db.update_team(&t.id, &team("startup", b, &[a, b])).await.unwrap().unwrap();
    assert!(channel_named(&db, "team-Startup").await.is_none(), "no chat left under the old name");
    let renamed = channel_named(&db, "team-startup").await.expect("the chat follows the team");
    assert_eq!(renamed.id, chat.id, "the same channel, renamed");
    assert_eq!(renamed.lead_kwant_id, b, "members and lead still sync after a rename");
    let history: Vec<String> = db.list_messages(&chat.id, 10).await.unwrap().into_iter().map(|m| m.text).collect();
    assert_eq!(history, vec!["before the rename"]);

    // A later edit, under the new name, still reaches the chat.
    db.update_team(&t.id, &team("Launch crew", a, &[a])).await.unwrap().unwrap();
    assert_eq!(channel_named(&db, "team-Launch crew").await.unwrap().id, chat.id);
}

#[tokio::test]
async fn a_rename_onto_another_channels_name_is_refused_whole() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let space = SpaceId::default_space();
    let a = db.list_kwants(&space).await.unwrap()[0].id.clone();

    let t = db.create_team(&space, &team("alpha", &a, &[&a])).await.unwrap();
    let other = db
        .create_channel(
            &space,
            &NewChannel { name: "team-beta".into(), description: None, lead_kwant_id: a.clone(), member_kwants: vec![], routing_mode: "auto".into() },
        )
        .await
        .unwrap();

    let err = db.update_team(&t.id, &team("beta", &a, &[&a])).await.unwrap_err();
    assert!(matches!(err, DbError::Conflict(ref m) if m.contains("#team-beta")), "{err}");
    assert_eq!(db.get_team(&t.id).await.unwrap().unwrap().name, "alpha", "the team isn't renamed either");
    assert!(channel_named(&db, "team-alpha").await.is_some());
    assert_eq!(channel_named(&db, "team-beta").await.unwrap().id, other.id, "the other channel is untouched");
}

/// What an old rename left behind: the team renamed, its chat still under the old name.
async fn rename_team_only(dir: &std::path::Path, from: &str, to: &str, members: &str) {
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", dir.join("kwnt.db").display())).await.unwrap();
    sqlx::query("UPDATE teams SET name = ?, member_kwants = ? WHERE name = ?").bind(to).bind(members).bind(from).execute(&pool).await.unwrap();
    pool.close().await;
}

#[tokio::test]
async fn startup_relinks_a_team_to_its_chat_and_keeps_history() {
    let tmp = tempdir().unwrap();
    let space = SpaceId::default_space();
    let (chat_id, a, b) = {
        let db = Database::init(tmp.path()).await.unwrap();
        let ks = db.list_kwants(&space).await.unwrap();
        let (a, b) = (ks[0].id.clone(), ks[1].id.clone());
        db.create_team(&space, &team("Startup", &a, &[&a])).await.unwrap();
        let chat = channel_named(&db, "team-Startup").await.unwrap();
        db.insert_message(&said(&chat.id, "one")).await.unwrap();
        db.insert_message(&said(&chat.id, "two")).await.unwrap();
        (chat.id, a, b)
    };
    rename_team_only(tmp.path(), "Startup", "startup", &serde_json::to_string(&[&a, &b]).unwrap()).await;

    for _ in 0..2 {
        // The second start finds nothing left to repair.
        let db = Database::init(tmp.path()).await.unwrap();
        assert!(channel_named(&db, "team-Startup").await.is_none());
        let chat = channel_named(&db, "team-startup").await.expect("relinked on start");
        assert_eq!(chat.id, chat_id, "renamed in place, not a new channel");
        assert_eq!(serde_json::from_str::<Vec<String>>(&chat.member_kwants).unwrap(), vec![a.clone(), b.clone()], "members that changed meanwhile catch up");
        let history: Vec<String> = db.list_messages(&chat_id, 10).await.unwrap().into_iter().map(|m| m.text).collect();
        assert_eq!(history, vec!["one", "two"]);
        let teams = db.list_teams(&space).await.unwrap();
        assert_eq!(db.list_channels(&space).await.unwrap().iter().filter(|c| c.name.starts_with("team-")).count(), teams.len());
    }
}

#[tokio::test]
async fn startup_leaves_ambiguous_chats_alone() {
    let tmp = tempdir().unwrap();
    let space = SpaceId::default_space();
    let a = {
        let db = Database::init(tmp.path()).await.unwrap();
        let a = db.list_kwants(&space).await.unwrap()[0].id.clone();
        db.create_team(&space, &team("Dup", &a, &[&a])).await.unwrap();
        db.create_channel(
            &space,
            &NewChannel { name: "team-DUP".into(), description: None, lead_kwant_id: a.clone(), member_kwants: vec![], routing_mode: "auto".into() },
        )
        .await
        .unwrap();
        a
    };
    rename_team_only(tmp.path(), "Dup", "dup", &serde_json::to_string(&[&a]).unwrap()).await;

    let db = Database::init(tmp.path()).await.unwrap();
    assert!(channel_named(&db, "team-Dup").await.is_some());
    assert!(channel_named(&db, "team-DUP").await.is_some());
    assert!(channel_named(&db, "team-dup").await.is_none(), "two candidates: neither is guessed at");
}
