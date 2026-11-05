//! Dev mode's test seams (QA plan §2.2), in their own binary because both are process-wide:
//! `KWANT_DEV_POLL_SECS` / `KWANT_DEV_CALL_CAP` are read once, and the clock skew moves
//! `kwnt::server::dev::now()` for everything in the process.

use chrono::{Duration, Utc};
use kwnt::db::dev_mode::DevBackupRow;
use kwnt::db::Database;
use kwnt::server::dev::{self, floor};
use serde_json::json;

/// The two tests change process-wide state: one at a time.
static ONE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn the_clock_seam_ages_a_backup_past_ten_minutes() {
    let _one = ONE.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let card = json!({"prod_floor": {"kind": "destructive", "needs_backup": true, "space_id": "s1"}});
    db.put_dev_mode("s1", true, "demo-org/demo-app", "main", None, Some("pg_dump demo > /tmp/demo.sql")).await.unwrap();

    db.insert_dev_backup(&DevBackupRow {
        id: "dvb_clock".into(),
        space_id: "s1".into(),
        kwant_id: None,
        path: Some("/tmp/demo.sql".into()),
        bytes: Some(10),
        ok: true,
        error: None,
        taken_at: dev::now(),
    })
    .await
    .unwrap();
    assert_eq!(floor::allow_refusal(&db, &card).await, None, "a backup taken just now is fresh");

    dev::set_clock_skew(9 * 60);
    assert_eq!(floor::allow_refusal(&db, &card).await, None, "9 minutes on, still fresh");
    dev::set_clock_skew(11 * 60);
    assert!((dev::now() - Utc::now() - Duration::minutes(11)).num_seconds().abs() <= 1);
    let refusal = floor::allow_refusal(&db, &card).await.expect("11 minutes on, Allow is refused");
    assert_eq!(refusal["backup"]["state"], "old", "{refusal}");
    dev::set_clock_skew(0);
    assert_eq!(floor::allow_refusal(&db, &card).await, None);
}

#[tokio::test]
async fn the_env_seams_set_the_poll_and_the_cap() {
    let _one = ONE.lock().await;
    // Read once per process: set before the first read, as a test server would at launch
    std::env::set_var("KWANT_DEV_POLL_SECS", "7");
    std::env::set_var("KWANT_DEV_CALL_CAP", "2");
    assert_eq!(dev::poll_every(), std::time::Duration::from_secs(7));
    assert_eq!(dev::call_cap(), 2);

    let tmp = tempfile::tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    assert!(db.take_dev_call("s1").await.unwrap());
    assert!(db.take_dev_call("s1").await.unwrap());
    assert!(!db.take_dev_call("s1").await.unwrap(), "the third call today is over the cap of 2");
    assert_eq!(db.dev_calls_today("s1").await.unwrap(), 2);
}
