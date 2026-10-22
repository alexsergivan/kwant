//! The IMAP source against a real IMAP server: the synthetic mailbox is uploaded, scanned, and
//! every message's flags are compared before and after (the scan must change nothing). Runs
//! when `KWNT_TEST_IMAP` names a throwaway server that accepts any login, e.g. GreenMail:
//!
//! ```sh
//! docker run -d --rm --name kwnt-imaptest -p 127.0.0.1:18943:3143 \
//!   -e GREENMAIL_OPTS='-Dgreenmail.setup.test.all -Dgreenmail.hostname=0.0.0.0 -Dgreenmail.auth.disabled' \
//!   greenmail/standalone:2.1.2
//! KWNT_TEST_IMAP=127.0.0.1:18943 cargo test --test zombie_imap_e2e -- --nocapture
//! docker rm -f kwnt-imaptest
//! ```
//!
//! Without it the test passes without doing anything (and says so).

use chrono::{Duration, Utc};
use futures::StreamExt;
use kwnt::zombie::{
    detect::{detect, Rules},
    imap::{ImapConfig, ImapSource, Security},
    MissionSource,
};
use std::collections::BTreeMap;
use std::path::Path;
use zeroize::Zeroizing;

const USER: &str = "alex@example.org";
const PASSWORD: &str = "throwaway-test-only";

type Session = async_imap::Session<tokio::net::TcpStream>;

async fn login(addr: &str) -> Session {
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut client = async_imap::Client::new(tcp);
    client.read_response().await.unwrap();
    client.login(USER, PASSWORD).await.map_err(|(e, _)| e).unwrap()
}

/// Every message's flags in `folder`, by UID, read with EXAMINE so reading changes nothing.
async fn flags(s: &mut Session, folder: &str) -> BTreeMap<u32, Vec<String>> {
    s.examine(folder).await.unwrap();
    let mut out = BTreeMap::new();
    let mut stream = s.uid_fetch("1:*", "(UID FLAGS)").await.unwrap();
    while let Some(f) = stream.next().await {
        let f = f.unwrap();
        let mut fl: Vec<String> = f.flags().map(|x| format!("{x:?}")).filter(|x| !x.contains("Recent")).collect();
        fl.sort();
        out.insert(f.uid.unwrap(), fl);
    }
    out
}

#[tokio::test]
async fn the_imap_source_reads_without_changing_anything() {
    let Ok(addr) = std::env::var("KWNT_TEST_IMAP") else {
        eprintln!("KWNT_TEST_IMAP is not set: skipping the IMAP end-to-end test");
        return;
    };
    let (host, port) = addr.rsplit_once(':').unwrap();
    let now = Utc::now();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zombie_mailbox");

    // Upload the synthetic mailbox: the inbox unread, the sent folder read
    let mut s = login(&addr).await;
    let _ = s.create("Sent").await;
    let mut uploaded = 0;
    for (folder, imap_folder, flags) in [("inbox", "INBOX", None), ("sent", "Sent", Some("(\\Seen)"))] {
        let mut files: Vec<_> = std::fs::read_dir(dir.join(folder)).unwrap().flatten().map(|e| e.path()).collect();
        files.sort();
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            let start = text.find("{{DATE:").unwrap();
            let end = start + text[start..].find("}}").unwrap();
            let days: i64 = text[start + 7..end].parse().unwrap();
            let text = format!("{}{}{}", &text[..start], (now - Duration::days(days)).to_rfc2822(), &text[end + 2..]);
            s.append(imap_folder, flags, None, text.as_bytes()).await.unwrap();
            uploaded += 1;
        }
    }
    let before = (flags(&mut s, "INBOX").await, flags(&mut s, "Sent").await);
    assert!(before.0.values().all(|f| f.is_empty()), "inbox starts unread: {:?}", before.0);

    let cfg = ImapConfig {
        host: host.into(),
        port: port.parse().unwrap(),
        security: Security::Plain,
        username: USER.into(),
        folders: Vec::new(),
        accept_invalid_certs: false,
    };
    cfg.validate().unwrap();
    let source = ImapSource::new("src-e2e", cfg, Zeroizing::new(PASSWORD.into()), &[]);
    let test = source.test().await.unwrap();
    println!("test: {test:?}");
    assert!(test.scanning.iter().any(|f| f == "Sent"), "the Sent folder is found: {test:?}");

    let convs = source.conversations(now - Duration::days(120)).await.unwrap();
    let rules = Rules { now, stale_days: 14, kwant_stale_days: 3 };
    let flagged: Vec<_> = convs.iter().filter_map(|c| detect(c, &rules)).collect();
    println!("{uploaded} messages uploaded, {} conversations read, {} flagged:", convs.len(), flagged.len());
    for f in &flagged {
        println!("  {:15} {}", f.kind.as_str(), f.title);
    }
    let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    let threads = manifest["threads"].as_array().unwrap();
    let expected = threads.iter().filter(|t| t["expect"].is_string()).count();
    assert_eq!(convs.len(), threads.len(), "every thread once over IMAP too");
    assert_eq!(flagged.len(), expected, "the same cases as the offline run");

    // Nothing was marked read, flagged or moved
    let after = (flags(&mut s, "INBOX").await, flags(&mut s, "Sent").await);
    assert_eq!(before, after, "the scan changed flags on the server");
    let _ = s.logout().await;
}
