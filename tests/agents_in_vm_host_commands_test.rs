//! Test plan #20 (D4, C9c): the Dev backup command runs bash on this computer as kwnt, so in a
//! folder an isolated Claude turn used it never runs. Its own file: the one-backup-per-space
//! claim is process-wide.
mod common;

use common::*;
use kwnt::db::spaces::SpaceId;
use kwnt::server::dev::backup;

const URI: &str = "/api/spaces/spc_default/dev/backup";

#[tokio::test]
async fn a_backup_command_never_runs_in_a_folder_an_isolated_bot_used() {
    let env = setup().await;
    let space = SpaceId::default_space();
    env.state.db.put_dev_mode(space.as_str(), true, "demo-org/demo-app", "main", None, Some("echo backup > ran.txt; echo \"Backup saved to ran.txt\"")).await.unwrap();
    let dir = backup::folder(&env.state, &space, None);
    env.state.db.record_isolated_folder(&dir.canonicalize().unwrap(), "some-bot").await.unwrap();

    let (s, v) = call(&env.app, "POST", URI, None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["ok"], false, "{v}");
    assert!(v["error"].as_str().unwrap().starts_with("folder_used_by_isolated_bot"), "{v}");
    assert!(!dir.join("ran.txt").exists(), "nothing ran");
    let (last, ok) = env.state.db.latest_dev_backups(space.as_str()).await.unwrap();
    assert!(!last.unwrap().ok, "the refusal is recorded like a failed backup");
    assert!(ok.is_none());

    // A folder nobody isolated used runs it
    assert!(env.state.db.forget_isolated_folder(&dir).await.unwrap());
    let (s, v) = call(&env.app, "POST", URI, None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["ok"], true, "{v}");
    assert!(dir.join("ran.txt").exists());
}
