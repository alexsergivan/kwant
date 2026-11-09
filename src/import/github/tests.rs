use super::*;
use std::io::Write as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn link(owner: &str, repo: &str, r: Option<&str>, path: Option<&str>) -> RepoLink {
    RepoLink { owner: owner.into(), repo: repo.into(), ref_: r.map(Into::into), path: path.map(Into::into) }
}

#[test]
fn accepted_link_shapes() {
    let cases = [
        ("https://github.com/coreyhaines31/marketingskills", link("coreyhaines31", "marketingskills", None, None)),
        ("github.com/o/r", link("o", "r", None, None)),
        ("www.github.com/o/r.git/", link("o", "r", None, None)),
        ("https://GitHub.com/o/r/tree/v1.2", link("o", "r", Some("v1.2"), None)),
        ("https://github.com/o/r/tree/main/skills/seo", link("o", "r", Some("main"), Some("skills/seo"))),
        ("https://github.com/o/r/blob/main/skills/seo/SKILL.md", link("o", "r", Some("main"), Some("skills/seo/SKILL.md"))),
        ("https://github.com/o/r/blob/main/AGENTS.md?plain=1#L3", link("o", "r", Some("main"), Some("AGENTS.md"))),
    ];
    for (raw, want) in cases {
        assert_eq!(parse_link(raw), Ok(want), "{raw}");
    }
    assert_eq!(zip_url(CODELOAD, &link("o", "r", None, None)), "https://codeload.github.com/o/r/zip/HEAD");
    assert_eq!(zip_url(CODELOAD, &link("o", "r", Some("v2"), Some("x"))), "https://codeload.github.com/o/r/zip/v2");
}

#[test]
fn other_hosts_and_shapes_are_refused() {
    for raw in [
        "https://gitlab.com/o/r",
        "https://github.com.evil.example/o/r",
        "https://evil.example/github.com/o/r",
        "https://raw.githubusercontent.com/o/r/main/SKILL.md",
        "https://codeload.github.com/o/r/zip/HEAD",
        "https://api.github.com/repos/o/r",
        "https://gist.github.com/o/abc",
        "http://github.com/o/r",
        "ftp://github.com/o/r",
        "file:///etc/passwd",
        "https://user:pw@github.com/o/r",
        "https://token@github.com/o/r",
        "https://github.com:8443/o/r",
        "https://github.com./o/r",
        "https://127.0.0.1/o/r",
        "https://github.com/o",
        "https://github.com/o/r/issues/1",
        "https://github.com/o/r/tree/feature/x%2F..%2F..",
        "https://github.com/o/r/tree/main/a/%2e%2e/b",
        "https://github.com/o/r/blob/main",
        "https://github.com/o/r/archive/refs/heads/main.zip",
        "https://github.com/o%2Fx/r",
        "https://github.com/../r",
        "https://github.com/o/r/tree/main/./x",
        "https://github.com/o/r/tree/main/a\\..\\b",
        "",
        "github.com/o/r with space",
    ] {
        assert!(matches!(parse_link(raw), Err(GithubError::NotGithub(_))), "{raw} was accepted: {:?}", parse_link(raw));
    }
}

#[test]
fn licence_guesses() {
    let mit = "MIT License\n\nCopyright (c) 2025\n\nPermission is hereby granted, free of charge, to any person obtaining a copy";
    assert_eq!(guess_spdx(mit), Some(("MIT", "MIT License")));
    assert_eq!(guess_spdx("Apache License\n Version 2.0, January 2004"), Some(("Apache-2.0", "Apache License 2.0")));
    assert_eq!(guess_spdx("GNU AFFERO GENERAL PUBLIC LICENSE Version 3"), Some(("AGPL-3.0", "GNU Affero General Public License v3.0")));
    assert_eq!(guess_spdx("GNU GENERAL PUBLIC LICENSE\n Version 3, 29 June 2007").map(|g| g.0), Some("GPL-3.0"));
    assert_eq!(guess_spdx("Redistribution and use in source and binary forms ... Neither the name of").map(|g| g.0), Some("BSD-3-Clause"));
    assert_eq!(guess_spdx("All rights reserved. Do not copy."), None);
    assert_eq!(licence_of(Some(("COPYING".into(), "whatever".into()))), Some(Licence { spdx: None, name: "Unknown licence".into(), file: "COPYING".into() }));
    assert_eq!(licence_of(None), None);
}

const SHA: &str = "2cdd34e1f0d7bfa5b9dfadecf3d72b525fbb1577";

fn repo_zip(files: &[(&str, &str)]) -> Vec<u8> {
    repo_zip_in("skills-main", files)
}

/// A codeload-shaped zip: every file under `<top>/`.
fn repo_zip_in(top: &str, files: &[(&str, &str)]) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut out);
        let opts = zip::write::SimpleFileOptions::default();
        z.add_directory(format!("{top}/"), opts).unwrap();
        for (name, text) in files {
            z.start_file(format!("{top}/{name}"), opts).unwrap();
            z.write_all(text.as_bytes()).unwrap();
        }
        z.set_comment(SHA.to_string()).unwrap();
        z.finish().unwrap();
    }
    out.into_inner()
}

fn skill(name: &str) -> String {
    format!("---\nname: {name}\ndescription: Does {name}.\n---\n\nSteps.\n")
}

#[test]
fn reads_a_repo_with_licence_commit_and_path() {
    let zip = repo_zip(&[
        ("LICENSE", "Permission is hereby granted, free of charge, to any person"),
        ("README.md", "# Skills"),
        ("seo/SKILL.md", &skill("seo")),
        ("copy/SKILL.md", &skill("copy")),
        ("src/main.rs", "fn main() {}"),
    ]);
    let (b, info) = read(&link("o", "skills", None, None), &zip).unwrap();
    assert_eq!(b.skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["seo", "copy"]);
    assert_eq!(b.source.label.as_deref(), Some("github.com/o/skills"));
    assert!(b.omitted.contains(&"From github.com/o/skills (MIT)".to_string()), "{:?}", b.omitted);
    assert_eq!(info.commit.as_deref(), Some(SHA));
    assert_eq!(info.ref_, "HEAD");
    assert_eq!(info.licence, Some(Licence { spdx: Some("MIT".into()), name: "MIT License".into(), file: "LICENSE".into() }));
    let json = serde_json::to_value(&info).unwrap();
    assert_eq!(json["ref"], "HEAD");

    let (b, info) = read(&link("o", "skills", Some("main"), Some("seo")), &zip).unwrap();
    assert_eq!(b.skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["seo"]);
    assert_eq!(b.source.label.as_deref(), Some("github.com/o/skills/seo"));
    assert_eq!(info.path.as_deref(), Some("seo"));

    let (b, _) = read(&link("o", "skills", Some("main"), Some("copy/SKILL.md")), &zip).unwrap();
    assert_eq!(b.skills.len(), 1);

    assert!(matches!(read(&link("o", "skills", None, Some("nope")), &zip), Err(GithubError::Import(ImportError::Unsupported(_)))));
}

/// Review #455 item 1: a `/tree/<ref>/<path>` link reads only `<path>`. 501 `.json` files outside
/// it (over the 500 readable-file cap) are listed by name, never inflated or counted.
#[test]
fn a_subpath_link_counts_only_files_inside_it() {
    let outside: Vec<(String, String)> = (0..501).map(|i| (format!("i18n/{i:03}.json"), "{\"hello\": \"hi\"}".to_string())).collect();
    let mut files: Vec<(&str, &str)> = outside.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
    let seo = skill("seo");
    files.push(("skills/seo/SKILL.md", &seo));
    files.push(("LICENSE", "Permission is hereby granted, free of charge, to any person"));
    let zip = repo_zip_in("r-main", &files);

    let (b, info) = read(&link("o", "r", Some("main"), Some("skills")), &zip).unwrap();
    assert_eq!(b.skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["seo"]);
    assert_eq!(info.licence.and_then(|l| l.spdx).as_deref(), Some("MIT"), "the repo's top licence is still read");

    // At the reader: only the folder (and the top licence) is inflated; the rest is named only.
    let un = crate::import::archive::read_zip_as(&zip, crate::import::archive::ZipKind::Repo { subpath: Some("skills") }).unwrap();
    assert_eq!(un.entries.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["skills/seo/SKILL.md", "LICENSE"]);
    assert_eq!(un.skipped.iter().filter(|p| p.starts_with("i18n/")).count(), 501);

    // The whole repo has 501 readable .json files: over the cap.
    assert!(matches!(read(&link("o", "r", Some("main"), None), &zip), Err(GithubError::Import(ImportError::TooLarge(_)))));
}

/// Review #455 item 2: the hidden-folder rule runs after `<repo>-<ref>/` is stripped, as for an
/// upload, so a repo's `.claude/agents` and `.claude/skills` are read; `skills/.system` stays skipped.
#[test]
fn a_repos_claude_folder_is_read_and_hidden_subfolders_are_not() {
    let zip = repo_zip_in(
        "r-main",
        &[
            (".claude/agents/a.md", "---\nname: reviewer\ndescription: Reviews code.\n---\n\nReview the diff.\n"),
            (".claude/skills/s/SKILL.md", &skill("s")),
            ("skills/.system/x/SKILL.md", &skill("x")),
        ],
    );
    let un = crate::import::archive::read_zip_as(&zip, crate::import::archive::ZipKind::Repo { subpath: None }).unwrap();
    assert_eq!(un.entries.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), [".claude/agents/a.md", ".claude/skills/s/SKILL.md"]);
    assert!(un.skipped.contains(&"skills/.system/x/SKILL.md".to_string()), "{:?}", un.skipped);

    // One bot and one skill; as for an upload, a skill next to a single agent is that agent's.
    let (b, _) = read(&link("o", "r", Some("main"), None), &zip).unwrap();
    assert_eq!(b.bots.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["reviewer"]);
    assert_eq!(b.bots[0].skills.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["s"]);
    assert!(b.skills.is_empty(), "{:?}", b.skills);
}

/// Review #455 item 2: a link straight to `/tree/main/.claude/agents` previews the bot.
#[tokio::test]
async fn a_link_to_a_repos_claude_agents_folder_previews_the_bot() {
    let zip = repo_zip_in(
        "r-main",
        &[
            (".claude/agents/a.md", "---\nname: reviewer\ndescription: Reviews code.\n---\n\nReview the diff.\n"),
            (".claude/skills/s/SKILL.md", &skill("s")),
        ],
    );
    let (base, seen) = mock(http("200 OK", &format!("Content-Length: {}\r\n", zip.len()), &zip)).await;
    let (b, info) = preview(&base, "https://github.com/o/r/tree/main/.claude/agents").await.unwrap();
    assert_eq!(b.bots.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["reviewer"]);
    assert!(b.skills.is_empty(), "{:?}", b.skills);
    assert_eq!(info.path.as_deref(), Some(".claude/agents"));
    assert_eq!(seen.lock().unwrap().as_slice(), ["GET /o/r/zip/main HTTP/1.1"]);
}

#[test]
fn no_licence_is_named_and_not_a_zip_is_invalid() {
    let zip = repo_zip(&[("seo/SKILL.md", &skill("seo"))]);
    let (b, info) = read(&link("o", "r", None, None), &zip).unwrap();
    assert_eq!(info.licence, None);
    assert!(b.omitted.contains(&"From github.com/o/r (no licence file)".to_string()));
    assert!(matches!(read(&link("o", "r", None, None), b"<html>rate limited</html>"), Err(GithubError::Import(ImportError::Invalid(_)))));
}

/// Serves one canned HTTP response per connection on 127.0.0.1 and records the request lines.
async fn mock(response: Vec<u8>) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or_default().to_string();
            seen2.lock().unwrap().push(head);
            let _ = sock.write_all(&response).await;
            let _ = sock.shutdown().await;
        }
    });
    (base, seen)
}

fn http(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut r = format!("HTTP/1.1 {status}\r\n{headers}Connection: close\r\n\r\n").into_bytes();
    r.extend_from_slice(body);
    r
}

#[tokio::test]
async fn fetches_the_zip() {
    let zip = repo_zip(&[("LICENSE", "Apache License Version 2.0"), ("seo/SKILL.md", &skill("seo"))]);
    let (base, seen) = mock(http("200 OK", &format!("Content-Type: application/zip\r\nContent-Length: {}\r\n", zip.len()), &zip)).await;
    let (b, info) = preview(&base, "https://github.com/o/skills/tree/main").await.unwrap();
    assert_eq!(b.skills.len(), 1);
    assert_eq!(info.licence.and_then(|l| l.spdx).as_deref(), Some("Apache-2.0"));
    assert_eq!(seen.lock().unwrap().as_slice(), ["GET /o/skills/zip/main HTTP/1.1"]);
}

#[tokio::test]
async fn redirects_are_refused_not_followed() {
    let (target, target_seen) = mock(http("200 OK", "Content-Length: 2\r\n", b"PK")).await;
    let (base, seen) = mock(http("302 Found", &format!("Location: {target}/elsewhere\r\nContent-Length: 0\r\n"), b"")).await;
    assert_eq!(preview(&base, "github.com/o/r").await.unwrap_err(), GithubError::Redirect);
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(target_seen.lock().unwrap().is_empty(), "the redirect was followed");
}

#[tokio::test]
async fn oversize_zips_are_refused() {
    // Refused from the length alone, before the body
    // Review #455 item 3: no advice to link a folder (codeload always sends the whole repo).
    const TOO_LARGE: &str = "This repository is larger than 20 MB, so kwant can't fetch it. Download the folder you want, zip it and import the zip instead.";
    let (base, _) = mock(http("200 OK", &format!("Content-Length: {}\r\n", MAX_ZIP_BYTES + 1), b"PK")).await;
    assert_eq!(preview(&base, "github.com/o/r/tree/main/skills").await.unwrap_err(), GithubError::Import(ImportError::TooLarge(TOO_LARGE.into())));
    // No length: counted while streaming
    let big = vec![b'x'; MAX_ZIP_BYTES + 1024];
    let (base, _) = mock(http("200 OK", "", &big)).await;
    assert_eq!(preview(&base, "github.com/o/r").await.unwrap_err(), GithubError::Import(ImportError::TooLarge(TOO_LARGE.into())));
}

#[tokio::test]
async fn statuses_map_to_errors() {
    let (base, _) = mock(http("404 Not Found", "Content-Length: 0\r\n", b"")).await;
    assert_eq!(preview(&base, "github.com/o/private").await.unwrap_err(), GithubError::NotFound);
    let (base, _) = mock(http("429 Too Many Requests", "Content-Length: 0\r\n", b"")).await;
    assert!(matches!(preview(&base, "github.com/o/r").await, Err(GithubError::FetchFailed(_))));
    // Nothing listening
    let closed = {
        let l = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        format!("http://{}", l.local_addr().unwrap())
    };
    assert!(matches!(preview(&closed, "github.com/o/r").await, Err(GithubError::FetchFailed(_))));
    // A link to another host never reaches the network
    let (base, seen) = mock(http("200 OK", "Content-Length: 0\r\n", b"")).await;
    assert!(matches!(preview(&base, "https://gitlab.com/o/r").await, Err(GithubError::NotGithub(_))));
    assert!(seen.lock().unwrap().is_empty());
}
