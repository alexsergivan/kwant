//! Local mode with a fake Chrome (a shell script named by KWNT_CHROME_PATH): one that prints an
//! error and exits fails the start within 2 s with its last line, instead of waiting out the port
//! wait; one that is only slow to open its port still starts.
#![cfg(unix)]

use kwnt::sandbox::computer::{self, CHROME_PATH_ENV};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

/// Both tests set the same environment variable.
static ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn fake_chrome(dir: &Path, body: &str) -> std::path::PathBuf {
    let path = dir.join("fake-chrome");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[tokio::test]
async fn a_chrome_that_exits_at_once_fails_fast_with_its_last_line() {
    let _env = ENV.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let chrome = fake_chrome(tmp.path(), "echo '[1:1:ERROR:sandbox_linux.cc] starting up' >&2\n\
         echo 'chrome: error while loading shared libraries: libnss3.so: cannot open shared object file' >&2\n\
         exit 1");
    std::env::set_var(CHROME_PATH_ENV, &chrome);
    let profile = tmp.path().join("local-chrome");
    let started = Instant::now();
    let error = computer::ensure_local_chrome(&profile, true).await.unwrap_err();
    let took = started.elapsed();
    std::env::remove_var(CHROME_PATH_ENV);
    assert!(took < Duration::from_secs(2), "took {took:?}: {error}");
    assert!(
        error.starts_with("kwant's Chrome closed right after starting: chrome: error while loading shared libraries: libnss3.so"),
        "{error}"
    );
    assert!(error.contains("(exit code 1)"), "{error}");
    assert!(error.contains("starting up"), "the lines before it come too: {error}");
    // its stderr went to a file in the profile, not to kwnt
    let log = std::fs::read_to_string(profile.join(computer::STDERR_FILE)).unwrap();
    assert!(log.contains("libnss3.so"), "{log}");
}

#[tokio::test]
async fn a_chrome_that_is_only_slow_still_starts() {
    let _env = ENV.lock().await;
    if std::process::Command::new("python3").arg("--version").output().is_err() {
        eprintln!("skipped: no python3 for the slow fake Chrome");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    // takes 2 s, then answers /json/version on a port it writes to DevToolsActivePort, as Chrome
    // does; it leaves by itself after 60 s, and the test stops it sooner
    let server = r#"
import http.server, json, os, sys, threading
d = sys.argv[1]
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps({"User-Agent": "Mozilla/5.0 HeadlessChrome/154.0.0.0"}).encode()
        self.send_response(200); self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def log_message(self, *a): pass
s = http.server.HTTPServer(("127.0.0.1", 0), H)
open(os.path.join(d, "fake.pid"), "w").write(str(os.getpid()))
open(os.path.join(d, "DevToolsActivePort"), "w").write("%d\n/devtools/browser/fake\n" % s.server_address[1])
threading.Timer(60, os._exit, [0]).start()
s.serve_forever()
"#;
    std::fs::write(tmp.path().join("server.py"), server).unwrap();
    let chrome = fake_chrome(tmp.path(), &format!(
        "for a in \"$@\"; do case \"$a\" in --user-data-dir=*) dir=\"${{a#--user-data-dir=}}\";; esac; done\n\
         echo 'starting slowly' >&2\n\
         sleep 2\n\
         exec python3 {} \"$dir\"",
        tmp.path().join("server.py").display()
    ));
    std::env::set_var(CHROME_PATH_ENV, &chrome);
    let profile = tmp.path().join("local-chrome");
    let started = Instant::now();
    let result = computer::ensure_local_chrome(&profile, true).await;
    let took = started.elapsed();
    std::env::remove_var(CHROME_PATH_ENV);
    if let Ok(pid) = std::fs::read_to_string(profile.join("fake.pid")) {
        if let Ok(pid) = pid.trim().parse::<libc::pid_t>() {
            // SAFETY: kill(2) on the fake server this test started
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
    let (endpoint, headless) = result.expect("the slow Chrome starts");
    assert!(endpoint.starts_with("http://127.0.0.1:"), "{endpoint}");
    assert!(headless);
    assert!(took >= Duration::from_secs(2), "it waited for the slow one: {took:?}");
}
