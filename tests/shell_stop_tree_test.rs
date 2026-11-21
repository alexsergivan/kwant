//! Stop ends a bot command's whole process tree (Windows plan decision 3: a Job Object per turn;
//! on Mac and Linux, the process group). The command starts a child bash that starts a grandchild;
//! both keep appending to a file until they are gone. Runs on every OS; on windows-latest it is the
//! check that the Job Object holds the tree. The `killed_kwnt` test at the end is Windows only.
use kwnt::tools::shell::execute_shell;
use serde_json::json;
use std::path::Path;
use std::time::Duration;
use tempfile::tempdir;

fn size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

#[tokio::test]
async fn stop_ends_the_child_and_the_grandchild() {
    let tmp = tempdir().unwrap();
    // Relative names: the command runs in the workspace, and Git bash on Windows would read the
    // backslashes of an absolute path as escapes
    let args = json!({
        "command": "bash -c '(while :; do echo g >> grandchild.log; sleep 0.2; done) & while :; do echo c >> child.log; sleep 0.2; done'",
        "timeout_secs": 120
    });
    let (child, grandchild) = (tmp.path().join("child.log"), tmp.path().join("grandchild.log"));
    let dir = tmp.path().to_path_buf();
    let task = tokio::spawn(async move { execute_shell(&dir, &args).await });

    // Both are running: each file grows
    let started = std::time::Instant::now();
    while (size(&child) == 0 || size(&grandchild) == 0) && started.elapsed() < Duration::from_secs(30) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(size(&child) > 0, "the child never started");
    assert!(size(&grandchild) > 0, "the grandchild never started");

    // Stop: the turn is cancelled, which drops the command
    task.abort();
    let _ = task.await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let (c, g) = (size(&child), size(&grandchild));
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(size(&child), c, "the child kept running after Stop");
    assert_eq!(size(&grandchild), g, "the grandchild kept running after Stop");
}

/// Windows only: kwnt.exe killed outright (Task Manager, `Stop-Process -Force`, a crash) takes its
/// own helper commands with it. Here the helper is the `docker ps` the browser-container reaper
/// runs at startup (serve.rs), the docker.exe that outlived kwnt.exe in the 0.9.33 release smoke.
/// A fake `docker.exe` keeps appending to a file for two minutes; once kwnt.exe is terminated, the
/// file must stop growing. No real Docker is needed.
///
/// The fake sits next to a copy of kwnt.exe, and that copy is the one started. Windows looks for a
/// program in the starting program's own folder first, then in System32, and only then on PATH.
/// GitHub's Windows runners put the real docker.exe in System32, so a fake that is only first on
/// PATH is never reached: kwnt.exe runs the real docker instead and the fake never writes.
#[cfg(windows)]
mod killed_kwnt {
    use super::size;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    const FAKE_DOCKER: &str = r#"
use std::io::Write;
fn main() {
    let log = std::env::var_os("KWNT_TEST_FAKE_DOCKER_LOG").expect("KWNT_TEST_FAKE_DOCKER_LOG");
    let args: Vec<String> = std::env::args().skip(1).collect();
    for _ in 0..1200 {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
            let _ = writeln!(f, "{} {}", std::process::id(), args.join(" "));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
"#;

    #[test]
    fn a_killed_kwnt_takes_its_docker_helper_with_it() {
        let tmp = tempdir().unwrap();
        let (bin, data) = (tmp.path().join("bin"), tmp.path().join("data"));
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let src = tmp.path().join("fake_docker.rs");
        std::fs::write(&src, FAKE_DOCKER).unwrap();
        let built = Command::new("rustc")
            .args(["--edition", "2021", "-o"])
            .arg(bin.join("docker.exe"))
            .arg(&src)
            .output()
            .expect("rustc runs");
        assert!(built.status.success(), "couldn't build the fake docker.exe: {}", String::from_utf8_lossy(&built.stderr));

        // kwnt.exe's own folder is searched before System32 (see above): run a copy from `bin`
        let exe = bin.join("kwnt.exe");
        std::fs::copy(env!("CARGO_BIN_EXE_kwnt"), &exe).expect("copy kwnt.exe next to the fake docker.exe");

        let log = tmp.path().join("docker.log");
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let path = std::env::join_paths(
            std::iter::once(bin.clone()).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())),
        )
        .unwrap();
        let (out, err) = (tmp.path().join("kwnt.out"), tmp.path().join("kwnt.err"));
        let mut kwnt = Command::new(&exe)
            .args(["serve", "--host", "127.0.0.1", "--port", &port.to_string(), "--no-laya"])
            .env("KWNT_DATA_DIR", &data)
            .env("PATH", &path)
            .env("KWNT_TEST_FAKE_DOCKER_LOG", &log)
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&out).unwrap())
            .stderr(std::fs::File::create(&err).unwrap())
            .spawn()
            .unwrap();

        // kwnt.exe is up and its docker helper is running: the file grows
        let started = Instant::now();
        while size(&log) == 0 && started.elapsed() < Duration::from_secs(90) {
            if let Ok(Some(status)) = kwnt.try_wait() {
                panic!("kwnt.exe exited ({status}) before it ran docker: {}", std::fs::read_to_string(&err).unwrap_or_default());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if size(&log) == 0 {
            let _ = kwnt.kill();
            panic!(
                "kwnt.exe never ran the fake docker in 90 s.\nstdout: {}\nstderr: {}",
                std::fs::read_to_string(&out).unwrap_or_default(),
                std::fs::read_to_string(&err).unwrap_or_default()
            );
        }

        // Killed outright: TerminateProcess, so none of kwnt's own cleanup runs
        kwnt.kill().unwrap();
        kwnt.wait().unwrap();
        std::thread::sleep(Duration::from_secs(1));
        let before = size(&log);
        std::thread::sleep(Duration::from_secs(2));
        let after = size(&log);
        if after != before {
            // don't leave it running on the machine
            let lines = std::fs::read_to_string(&log).unwrap_or_default();
            let mut pids: Vec<&str> = lines.lines().filter_map(|l| l.split(' ').next()).collect();
            pids.sort_unstable();
            pids.dedup();
            for pid in &pids {
                let _ = Command::new("taskkill").args(["/F", "/PID", pid]).stdout(Stdio::null()).stderr(Stdio::null()).status();
            }
            panic!(
                "kwnt.exe's docker helper (pid {}) kept running after kwnt.exe was killed; it ran: {}",
                pids.join(", "),
                lines.lines().last().unwrap_or("")
            );
        }
    }
}
