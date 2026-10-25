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
}

/// A stand-in `kwnt serve` that answers one `POST /api/signin/links` and hands back the request
/// it got, so the test can check the CLI asked with the token.
fn fake_server() -> (u16, std::thread::JoinHandle<String>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        sock.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
        let mut req = Vec::new();
        let mut buf = [0u8; 4096];
        while !req.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = sock.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            req.extend_from_slice(&buf[..n]);
        }
        let body = r#"{"code":"kl1.onetimecode","url":null,"expires_in":900}"#;
        let resp = format!(
            "HTTP/1.1 201 Created\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        sock.write_all(resp.as_bytes()).unwrap();
        String::from_utf8_lossy(&req).into_owned()
    });
    (port, handle)
}

fn kwnt_token_link(data_dir: &std::path::Path, port: u16) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_kwnt"))
        .arg("token")
        .env("KWNT_DATA_DIR", data_dir)
        .env_remove("KWNT_API_TOKEN")
        .env("KWNT_HOST", "127.0.0.1")
        .env("KWNT_PORT", port.to_string())
        .env_remove("KWNT_ALLOWED_ORIGINS")
        .output()
        .expect("run kwnt token")
}

#[test]
fn token_prints_a_one_time_link_never_the_token() {
    let dir = tempfile::tempdir().unwrap();
    let tok = "abcdefghijklmnopqrstuvwxyz012345";
    std::fs::write(dir.path().join("api-token"), format!("{tok}\n")).unwrap();
    let (port, server) = fake_server();
    let out = kwnt_token_link(dir.path(), port);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains(&format!("http://127.0.0.1:{port}/#kwnt_token=kl1.onetimecode")), "{stdout}");
    assert!(stdout.contains("works once"), "{stdout}");
    assert!(!stdout.contains(tok) && !stderr.contains(tok), "the API token must not be printed:\n{stdout}\n{stderr}");
    // It points at the token for typing by hand, by where it lives, not by value
    assert!(stdout.contains(&dir.path().join("api-token").display().to_string()), "{stdout}");
    let req = server.join().unwrap();
    assert!(req.starts_with("POST /api/signin/links "), "{req}");
    assert!(req.to_ascii_lowercase().contains(&format!("authorization: bearer {tok}")), "{req}");
}

#[test]
fn token_without_a_running_server_fails_without_printing_the_token() {
    let dir = tempfile::tempdir().unwrap();
    let tok = "abcdefghijklmnopqrstuvwxyz012345";
    std::fs::write(dir.path().join("api-token"), format!("{tok}\n")).unwrap();
    // A port nobody listens on
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let out = kwnt_token_link(dir.path(), port);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(!stdout.contains(tok) && !stderr.contains(tok), "{stdout}\n{stderr}");
    assert!(stderr.contains("Is `kwnt serve` running?"), "{stderr}");
    assert!(stderr.contains("kwnt token --raw"), "{stderr}");
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
