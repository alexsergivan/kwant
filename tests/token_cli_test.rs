//! `kwnt token` only reads the token; it must never create one (a token made by the wrong user
//! or for the wrong data dir is one the server rejects).

use std::process::Command;

fn kwnt_token(data_dir: &std::path::Path, raw: bool) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kwnt"));
    cmd.arg("token").env("KWNT_DATA_DIR", data_dir).env_remove("KWNT_API_TOKEN");
    if raw {
        cmd.arg("--raw");
    }
    cmd.output().expect("run kwnt token")
}

#[test]
fn token_fails_without_a_token_file_and_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = kwnt_token(dir.path(), true);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "nothing printed to stdout on failure");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no token file at"), "{err}");
    assert!(err.contains("as the user that runs `kwnt serve`"), "{err}");
    assert!(err.contains("docker compose exec kwnt kwnt token --raw"), "{err}");
    assert!(!dir.path().join("api-token").exists());
}

#[test]
fn token_prints_the_existing_token() {
    let dir = tempfile::tempdir().unwrap();
    let tok = "abcdefghijklmnopqrstuvwxyz012345";
    std::fs::write(dir.path().join("api-token"), format!("{tok}\n")).unwrap();
    let out = kwnt_token(dir.path(), true);
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), tok);
    let link = kwnt_token(dir.path(), false);
    assert!(String::from_utf8_lossy(&link.stdout).contains(&format!("#kwnt_token={tok}")));
}

#[test]
fn token_prefers_kwnt_api_token() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_kwnt"))
        .args(["token", "--raw"])
        .env("KWNT_DATA_DIR", dir.path())
        .env("KWNT_API_TOKEN", "from-env-0123456789abcdef")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "from-env-0123456789abcdef");
    assert!(!dir.path().join("api-token").exists());
}
