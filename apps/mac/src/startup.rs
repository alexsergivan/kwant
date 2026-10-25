//! What the app works out before kwant starts: the PATH a Finder launch lacks, and which port
//! to use. Plain std, so it is tested without a window.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Where the claude, codex and grok CLIs usually live, besides the login shell's PATH.
const EXTRA_DIRS: [&str; 3] = ["/opt/homebrew/bin", "/usr/local/bin", "~/.local/bin"];

/// The PATH to run with: the login shell's PATH, then the current one, then [`EXTRA_DIRS`], each
/// directory once, in that order. `login_path` is `None` when the shell gave nothing in time.
pub fn merged_path(login_path: Option<&str>, current: &str, home: &Path) -> String {
    let mut out: Vec<String> = Vec::new();
    let extras = EXTRA_DIRS.iter().map(|d| match d.strip_prefix("~/") {
        Some(rest) => home.join(rest).display().to_string(),
        None => d.to_string(),
    });
    let dirs = login_path
        .unwrap_or("")
        .split(':')
        .chain(current.split(':'))
        .map(str::to_string)
        .chain(extras);
    for dir in dirs {
        let dir = dir.trim().to_string();
        if !dir.is_empty() && !out.contains(&dir) {
            out.push(dir);
        }
    }
    out.join(":")
}

/// The PATH `$SHELL -lc` sees (so .zprofile/.zshrc additions count), or `None` after `timeout`.
/// The shell's own output is noise: only the line between the markers is read.
pub fn login_shell_path(shell: &str, timeout: Duration) -> Option<String> {
    let mut child = Command::new(shell)
        .args(["-ilc", "printf '\\n__KWANT_PATH__%s__KWANT_END__\\n' \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let start = out.find("__KWANT_PATH__")? + "__KWANT_PATH__".len();
    let end = start + out[start..].find("__KWANT_END__")?;
    Some(out[start..end].to_string()).filter(|p| !p.is_empty())
}

/// What to do about the preferred port.
#[derive(Debug, PartialEq, Eq)]
pub enum PortChoice {
    /// kwant already answers there: open a window on it, start nothing.
    Attach(u16),
    /// Start kwant here.
    Serve(u16),
}

/// kwant on `preferred` → attach; `preferred` free → serve there; taken by something else →
/// serve on a free loopback port.
pub fn choose_port(preferred: u16) -> std::io::Result<PortChoice> {
    if is_kwant(preferred) {
        return Ok(PortChoice::Attach(preferred));
    }
    if TcpListener::bind(("127.0.0.1", preferred)).is_ok() {
        return Ok(PortChoice::Serve(preferred));
    }
    free_port().map(PortChoice::Serve)
}

/// A loopback port nobody is listening on right now.
pub fn free_port() -> std::io::Result<u16> {
    Ok(TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

/// Whether `127.0.0.1:port/api/health` answers as kwant.
pub fn is_kwant(port: u16) -> bool {
    health(port).is_some()
}

/// The `/api/health` body when kwant answers on `port` with `"app":"kwnt"`.
pub fn health(port: u16) -> Option<String> {
    let (status, body) = http_get(port, "/api/health", Duration::from_millis(800))?;
    let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    (status == 200 && compact.contains("\"app\":\"kwnt\"")).then_some(body)
}

/// Waits up to `timeout` for kwant to answer on `port`.
pub fn wait_for_health(port: u16, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(body) = health(port) {
            return Some(body);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// A plain HTTP/1.0 GET to 127.0.0.1: status and body. Loopback only, no TLS, no redirects.
pub fn http_get(port: u16, path: &str, timeout: Duration) -> Option<(u16, String)> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, timeout).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    // One write: `write!` on a bare TcpStream sends each piece separately, and a server that answers after the first
    // piece and closes makes macOS reset the connection, losing the answer.
    let request = format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\n\r\n");
    stream.write_all(request.as_bytes()).ok()?;
    let mut raw = Vec::new();
    stream.take(4 * 1024 * 1024).read_to_end(&mut raw).ok()?;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n")?;
    let status = head.split_whitespace().nth(1)?.parse().ok()?;
    Some((status, body.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    // The port tests run in parallel: a port free_port() just released can be handed to
    // another test's listener before choose_port() looks at it. Run them one at a time.
    static PORTS: Mutex<()> = Mutex::new(());

    fn ports() -> std::sync::MutexGuard<'static, ()> {
        PORTS.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn login_path_comes_first_then_current_then_extras_without_repeats() {
        let home = PathBuf::from("/Users/o");
        let merged = merged_path(Some("/Users/o/.cargo/bin:/usr/bin:/opt/homebrew/bin"), "/usr/bin:/bin", &home);
        assert_eq!(
            merged,
            "/Users/o/.cargo/bin:/usr/bin:/opt/homebrew/bin:/bin:/usr/local/bin:/Users/o/.local/bin"
        );
    }

    #[test]
    fn finder_launch_without_a_shell_answer_still_gets_homebrew_and_local_bin() {
        let merged = merged_path(None, "/usr/bin:/bin:/usr/sbin:/sbin", Path::new("/Users/o"));
        assert_eq!(
            merged,
            "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin:/Users/o/.local/bin"
        );
    }

    #[test]
    fn login_shell_path_reads_only_the_marked_line() {
        let path = login_shell_path("/bin/sh", Duration::from_secs(5)).expect("sh answers");
        assert!(path.contains('/'), "{path}");
        assert!(!path.contains("__KWANT"), "{path}");
    }

    #[test]
    fn a_shell_that_hangs_is_given_up_on() {
        let dir = std::env::temp_dir().join(format!("kwant-hang-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let shell = dir.join("slowsh");
        std::fs::write(&shell, "#!/bin/sh\nsleep 30\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let started = Instant::now();
        assert_eq!(login_shell_path(shell.to_str().unwrap(), Duration::from_millis(300)), None);
        assert!(started.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn serve_once(body: &'static str) -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                // read the whole request first, so closing doesn't reset the connection under the client
                let mut request = Vec::new();
                let mut buf = [0u8; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buf[..n]),
                    }
                }
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{body}");
                let _ = stream.write_all(response.as_bytes());
            }
        });
        port
    }

    #[test]
    fn kwant_on_the_port_is_attached_to() {
        let _ports = ports();
        let port = serve_once(r#"{"status":"ok","app":"kwnt","version":"0.1.0"}"#);
        assert_eq!(choose_port(port).unwrap(), PortChoice::Attach(port));
    }

    #[test]
    fn something_else_on_the_port_moves_kwant_to_a_free_one() {
        let _ports = ports();
        let port = serve_once(r#"{"status":"ok","app":"other"}"#);
        match choose_port(port).unwrap() {
            PortChoice::Serve(p) => assert_ne!(p, port),
            other => panic!("expected a new port, got {other:?}"),
        }
    }

    #[test]
    fn a_free_port_is_used_as_it_is() {
        let _ports = ports();
        let port = free_port().unwrap();
        assert_eq!(choose_port(port).unwrap(), PortChoice::Serve(port));
    }
}
