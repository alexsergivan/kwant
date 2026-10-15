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
        avatar_sprite: "cyber_mage".into(),
        driver_type: "api".into(),
        preferred_model: "auto".into(),
    };

    let kwant = db.create_kwant(&new_kwant).await.unwrap();
    assert_eq!(kwant.name, "PixelMage");

    let list = db.list_kwants().await.unwrap();
    // Default seeded kwants (3) + 1 newly created = 4
    assert!(list.iter().any(|k| k.name == "PixelMage"));
    assert!(list.len() >= 4);

    let fetched = db.get_kwant(&kwant.id).await.unwrap();
    assert!(fetched.is_some());
    assert_eq!(fetched.unwrap().name, "PixelMage");

    // Channels
    let new_chan = NewChannel {
        name: "general".into(),
        description: Some("Guild Tavern".into()),
        lead_kwant_id: kwant.id.clone(),
        member_kwants: vec![kwant.id.clone()],
        routing_mode: "auto".into(),
    };
    let chan = db.create_channel(&new_chan).await.unwrap();
    assert_eq!(chan.name, "general");

    let channels = db.list_channels().await.unwrap();
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

    let teams = db.list_teams().await.unwrap();
    assert!(!teams.is_empty(), "Default seed team should exist");
    assert_eq!(teams[0].name, "core-engineering");

    let kwants = db.list_kwants().await.unwrap();
    let new_team = NewTeam {
        name: "infra-sec".into(),
        description: Some("Security and cloud infrastructure".into()),
        icon: Some("shield".into()),
        lead_kwant_id: kwants[0].id.clone(),
        member_kwants: vec![kwants[0].id.clone(), kwants[1].id.clone()],
        workspace_path: None,
        repo_url: None,
    };
    let created = db.create_team(&new_team).await.unwrap();
    assert_eq!(created.name, "infra-sec");

    let fetched = db.get_team_by_name("infra-sec").await.unwrap();
    assert!(fetched.is_some());
    assert_eq!(fetched.unwrap().id, created.id);

    let by_id = db.get_team(&created.id).await.unwrap();
    assert!(by_id.is_some());
    assert_eq!(by_id.unwrap().name, "infra-sec");

    let channels = db.list_channels().await.unwrap();
    assert!(channels.iter().any(|c| c.name == "team-infra-sec"));
    assert!(channels.iter().any(|c| c.name == "team-core-engineering"));

    let deleted = db.delete_team(&created.id).await.unwrap();
    assert!(deleted);
    let after_delete = db.get_team(&created.id).await.unwrap();
    assert!(after_delete.is_none());
}

#[tokio::test]
async fn test_team_with_workspace_and_repo() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db = Database::init(temp_dir.path()).await.unwrap();

    let kwants = db.list_kwants().await.unwrap();
    let new_team = NewTeam {
        name: "dev-squad".into(),
        description: Some("Frontend and backend app development".into()),
        icon: Some("code".into()),
        lead_kwant_id: kwants[0].id.clone(),
        member_kwants: vec![kwants[0].id.clone()],
        workspace_path: Some("/Users/dev/my-project".into()),
        repo_url: Some("https://github.com/my-org/my-project.git".into()),
    };
    let created = db.create_team(&new_team).await.unwrap();
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


