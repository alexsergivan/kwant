//! Test helpers: a tiny HTTP mock on 127.0.0.1 (Juno's ports 9475–9478 only), a fake gitleaks
//! release built in memory, and fake keys assembled at runtime so no token prefix is written in
//! the source. Nothing here reaches a real registry or GitHub.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The registry and download mock.
pub const MOCK_PORT: u16 = 9475;
/// A listener the fake gitleaks tries to reach; it must hear nothing.
pub const LEAK_PORT: u16 = 9477;

/// Tests that bind a port take this first, so they never share one.
pub async fn port_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static L: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    L.lock().await
}

#[derive(Debug, Clone)]
pub struct Req {
    pub method: String,
    pub path: String,
    /// The request line and headers as received.
    pub head: String,
    pub body: Vec<u8>,
}

/// One answer: status, body and how long to wait before sending it.
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub delay: Duration,
}

impl Reply {
    pub fn json(status: u16, v: serde_json::Value) -> Reply {
        Reply { status, body: v.to_string().into_bytes(), delay: Duration::ZERO }
    }
    pub fn bytes(status: u16, body: Vec<u8>) -> Reply {
        Reply { status, body, delay: Duration::ZERO }
    }
    pub fn slow(mut self, d: Duration) -> Reply {
        self.delay = d;
        self
    }
}

pub struct Mock {
    pub base: String,
    pub seen: Arc<Mutex<Vec<Req>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Mock {
    pub fn requests(&self) -> Vec<Req> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Serves `handler` on `127.0.0.1:port` until dropped.
pub async fn serve<F>(port: u16, handler: F) -> Mock
where
    F: Fn(&Req) -> Reply + Send + Sync + 'static,
{
    assert!((9475..=9478).contains(&port), "Juno's tests use ports 9475-9478 only");
    // A mock dropped a moment ago (here or in the test before) frees its port once its task is
    // cancelled, which is not instant
    let mut tries = 0;
    let listener = loop {
        match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => break l,
            Err(e) if tries < 150 => {
                let _ = e;
                tries += 1;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(e) => panic!("mock port {port} not free: {e}"),
        }
    };
    let seen: Arc<Mutex<Vec<Req>>> = Arc::default();
    let handler = Arc::new(handler);
    let seen2 = seen.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let (seen, handler) = (seen2.clone(), handler.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let head_end = loop {
                    let Ok(n) = sock.read(&mut chunk).await else { return };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                    if buf.len() > 1 << 16 {
                        return;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
                let len = head
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                    .unwrap_or(0);
                while buf.len() < head_end + len {
                    let Ok(n) = sock.read(&mut chunk).await else { return };
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let mut first = head.lines().next().unwrap_or("").split(' ');
                let req = Req {
                    method: first.next().unwrap_or("").into(),
                    path: first.next().unwrap_or("").into(),
                    head: head.clone(),
                    body: buf[head_end..].to_vec(),
                };
                seen.lock().unwrap().push(req.clone());
                let r = handler(&req);
                tokio::time::sleep(r.delay).await;
                let mut out = format!("HTTP/1.1 {} X\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n", r.status, r.body.len())
                    .into_bytes();
                out.extend_from_slice(&r.body);
                let _ = sock.write_all(&out).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    Mock { base: format!("http://127.0.0.1:{port}"), seen, task }
}

/// A ustar archive of `(name, mode, bytes)` entries.
pub fn tar(entries: &[(&str, u32, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, mode, data) in entries {
        let mut h = [0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        let put = |h: &mut [u8; 512], at: usize, width: usize, v: u64| {
            let s = format!("{v:0w$o}\0", w = width - 1);
            h[at..at + width].copy_from_slice(s.as_bytes());
        };
        put(&mut h, 100, 8, u64::from(*mode));
        put(&mut h, 108, 8, 0);
        put(&mut h, 116, 8, 0);
        put(&mut h, 124, 12, data.len() as u64);
        put(&mut h, 136, 12, 0);
        h[156] = b'0';
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
        h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    out
}

pub fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(bytes).unwrap();
    e.finish().unwrap()
}

/// A fake gitleaks: a shell script that reports every `FAKE_SECRET_<digits>` in the tree it is
/// given, at its file and line, after running `extra` (a sleep, a crash, a network attempt).
pub fn fake_gitleaks(extra: &str) -> Vec<u8> {
    format!(
        r#"#!/bin/sh
out=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--report-path" ]; then out="$a"; fi
  prev="$a"
done
{extra}
printf '[' > "$out"
grep -rnE 'FAKE_SECRET_[0-9]+' tree | {{
  sep=""
  while IFS=: read -r f n rest; do
    printf '%s{{"RuleID":"fake-rule","File":"%s","StartLine":%s,"EndLine":%s,"Secret":"REDACTED","Match":"REDACTED"}}' "$sep" "$f" "$n" "$n" >> "$out"
    sep=","
  done
}}
printf ']' >> "$out"
exit 0
"#
    )
    .into_bytes()
}

/// A release archive holding `script` as `gitleaks`, with the archive's and the binary's sha256.
pub fn fake_release(script: &[u8]) -> (Vec<u8>, String, String) {
    let archive = gzip(&tar(&[("LICENSE", 0o644, b"demo"), ("README.md", 0o644, b"# demo"), ("gitleaks", 0o755, script)]));
    let a = super::gitleaks::sha256_hex(&archive);
    let b = super::gitleaks::sha256_hex(script);
    (archive, a, b)
}

/// Letters from a fixed seed, so fake keys look random but tests are repeatable.
fn letters(seed: u64, n: usize, alphabet: &[u8]) -> String {
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            alphabet[((x >> 33) as usize) % alphabet.len()] as char
        })
        .collect()
}

const UPPER32: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
const ALNUM: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Fake keys in the shape of real ones, joined at runtime. None of them is a real credential.
pub fn fake_aws_key() -> String {
    format!("{}{}{}", "AK", "IA", letters(1, 16, UPPER32))
}
pub fn fake_github_token() -> String {
    format!("{}{}{}", "gh", "p_", letters(2, 36, ALNUM))
}
pub fn fake_stripe_key() -> String {
    format!("{}{}{}", "sk", "_live_", letters(3, 32, ALNUM))
}
pub fn fake_private_key() -> String {
    let body: Vec<String> = (0..6).map(|i| letters(10 + i, 64, B64)).collect();
    let kind = ["PRIVATE", "KEY"].join(" ");
    format!("-----{} {kind}-----\n{}\n-----{} {kind}-----", "BEGIN", body.join("\n"), "END")
}

/// A unified diff hunk adding `lines` at line 1 of a new file.
pub fn added_patch(lines: &[&str]) -> String {
    let mut p = format!("@@ -0,0 +1,{} @@", lines.len());
    for l in lines {
        p.push_str("\n+");
        p.push_str(l);
    }
    p
}
