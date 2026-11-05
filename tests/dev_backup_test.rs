//! **Back up first** through `POST /api/spaces/:id/dev/backup` (D9, Rigel's F1–F3 on #313): a
//! request that goes away mid-backup doesn't strand the space, and only a file this run wrote
//! counts as a backup.

mod common;

use common::*;
use kwnt::db::spaces::SpaceId;
use kwnt::server::dev::backup;
use std::time::Duration;

const URI: &str = "/api/spaces/spc_default/dev/backup";

async fn set_command(env: &Env, command: &str) {
    env.state.db.put_dev_mode(SpaceId::default_space().as_str(), true, "demo-org/demo-app", "main", None, Some(command)).await.unwrap();
}

#[tokio::test]
async fn a_dropped_request_frees_the_space_and_only_a_file_this_run_wrote_counts() {
    let env = setup().await;
    let space = SpaceId::default_space();
    set_command(&env, "sleep 1; mkdir -p bk && head -c 300 /dev/zero > bk/app.sql && echo \"Backup saved to bk/app.sql\"").await;

    // The browser goes away while the command runs: the handler's future is dropped
    let gone = tokio::time::timeout(Duration::from_millis(300), call(&env.app, "POST", URI, None)).await;
    assert!(gone.is_err(), "the request should still be running at 300 ms");
    assert!(backup::is_running(space.as_str()), "the backup keeps running without its request");
    let (s, v) = call(&env.app, "POST", URI, None).await;
    assert_eq!(s.as_u16(), 409, "a second press while it runs is refused: {v}");

    // It finishes on its own, is recorded, and frees the space
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while backup::is_running(space.as_str()) {
        assert!(tokio::time::Instant::now() < deadline, "the space was never freed");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (last, ok) = env.state.db.latest_dev_backups(space.as_str()).await.unwrap();
    let last = last.expect("the dropped run was recorded");
    assert!(last.ok, "{:?}", last.error);
    assert!(last.path.as_deref().unwrap().ends_with("bk/app.sql"));
    assert_eq!(last.bytes, Some(300));
    assert_eq!(ok.map(|b| b.id), Some(last.id));

    // The next press runs again instead of a 409 until restart
    let (s, v) = call(&env.app, "POST", URI, None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["ok"], true, "{v}");
    assert!(!backup::is_running(space.as_str()));

    // Only a file this run wrote counts. (One test, not two: the one-run-per-space claim is
    // process-wide, so two tests on the default space would refuse each other.)
    // Exit 0 and nothing named: not a backup (F2)
    set_command(&env, "echo done").await;
    let (s, v) = call(&env.app, "POST", URI, None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["ok"], false, "{v}");
    assert_eq!(v["error"], backup::NOTHING_NAMED);

    // An existing file from an hour ago, named but not written by this run (F3)
    set_command(&env, "mkdir -p bk && head -c 300 /dev/zero > bk/old.sql && touch -d '1 hour ago' bk/old.sql && echo \"Backup saved to bk/old.sql\"").await;
    let (s, v) = call(&env.app, "POST", URI, None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["ok"], false, "{v}");
    assert!(v["error"].as_str().unwrap().contains("older than this backup run"), "{v}");

    // A bare folder name in the run's folder, written now (F3)
    set_command(&env, "mkdir -p snapshots && head -c 64 /dev/zero > snapshots/app.dump && echo \"Backup written to snapshots\"").await;
    let (s, v) = call(&env.app, "POST", URI, None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["ok"], true, "{v}");
    assert!(v["path"].as_str().unwrap().ends_with("/snapshots"), "{v}");
    assert_eq!(v["bytes"], 64);

    // The Allow gate reads the newest ok backup: the failed ones above don't count
    let (last, ok) = env.state.db.latest_dev_backups(SpaceId::default_space().as_str()).await.unwrap();
    assert!(last.unwrap().ok);
    assert!(ok.unwrap().path.unwrap().ends_with("/snapshots"));
}
