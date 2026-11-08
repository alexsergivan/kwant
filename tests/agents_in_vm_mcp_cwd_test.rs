//! M2 (D4, C8), and should-fix 9: in a folder an isolated bot used (equal, a child, or one holding
//! it), stdio MCP servers start from a fresh, empty, kwnt-owned folder, so a file the bot planted
//! there never runs on this computer: not through kwnt's own client, not through a CLI agent that
//! starts the servers from its config in its own folder, and not through the Test button. A server
//! that names a file in the folder isn't started at all.
#![cfg(unix)]
mod common;

use common::*;
use kwnt::db::{models::McpServer, spaces::SpaceId};
use kwnt::mcp::McpClient;
use kwnt::server::mcp_runtime;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const NODE_BIN: &str = "/opt/openmausbot/node-v24.21.0-linux-arm64/bin";

fn npx() -> Option<PathBuf> {
    let path = std::env::var("PATH").unwrap_or_default();
    std::env::split_paths(&path).chain([PathBuf::from(NODE_BIN)]).map(|d| d.join("npx")).find(|p| p.is_file())
}

fn stdio(name: &str, command: &str, args: &[&str], home: &Path) -> McpServer {
    let path_var = format!("{}:/usr/local/bin:/usr/bin:/bin", npx().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default().display());
    McpServer {
        id: String::new(),
        name: name.into(),
        transport: "stdio".into(),
        command: Some(command.into()),
        args: args.iter().map(|a| a.to_string()).collect(),
        url: None,
        // npx must never fetch a package: without the planted bin it fails offline instead
        env: BTreeMap::from([
            ("PATH".into(), path_var),
            ("HOME".into(), home.display().to_string()),
            ("npm_config_yes".into(), "false".into()),
            ("npm_config_offline".into(), "true".into()),
        ]),
        headers: BTreeMap::new(),
        enabled: true,
        timeout_secs: 5,
        created_at: None,
    }
}

/// `dir` gets a `planted` Python module and an npm bin `srv`, each writing `<out>/<what>-<tag>`.
fn plant(dir: &Path, out: &Path, tag: &str) {
    std::fs::create_dir_all(dir.join("node_modules/.bin")).unwrap();
    std::fs::write(dir.join("planted.py"), format!("open({:?}, 'w').write('ran')\n", out.join(format!("py-{tag}")).display().to_string())).unwrap();
    let bin = dir.join("node_modules/.bin/srv");
    std::fs::write(&bin, format!("#!/bin/sh\necho ran > '{}'\n", out.join(format!("npx-{tag}")).display())).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Starts every server the way kwnt does (its client, in `cwd`) and the way a CLI agent does
/// (the config's command and args, run in `cwd`), each to completion.
async fn start_all(servers: &[McpServer], cwd: &Path) {
    for s in servers.iter().filter(|s| s.transport == "stdio") {
        let _ = McpClient::connect(s, Some(cwd)).await;
        let status = tokio::process::Command::new(s.command.as_deref().unwrap())
            .args(&s.args)
            .env_clear()
            .envs(&s.env)
            .current_dir(cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .status();
        let _ = tokio::time::timeout(Duration::from_secs(30), status).await;
    }
}

/// The turn's servers this test added (a fresh database may bring its own).
fn ours(servers: &[McpServer]) -> Vec<McpServer> {
    servers.iter().filter(|s| ["py", "node", "file"].contains(&s.name.as_str())).cloned().collect()
}

fn ran(out: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[tokio::test]
async fn stdio_servers_in_an_isolated_bots_folder_start_from_an_empty_one_and_planted_files_never_run() {
    let env = setup().await;
    let space = SpaceId::default_space();
    let kwant = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    let work = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(work.path().join("rec")).unwrap();
    let rec = work.path().join("rec").canonicalize().unwrap();
    let child = rec.join("child");
    let free = work.path().canonicalize().unwrap().join("free");
    for (dir, tag) in [(&rec, "rec"), (&child, "child"), (&free, "free")] {
        plant(dir, out.path(), tag);
    }

    let py = stdio("py", "python3", &["-m", "planted"], home.path());
    let mut servers = vec![py];
    let has_npx = npx().is_some();
    if has_npx {
        servers.push(stdio("node", "npx", &["srv"], home.path()));
    } else {
        eprintln!("npx isn't installed here: only the Python case runs");
    }
    // Names a file in the folder: never started there
    servers.push(stdio("file", "python3", &["./planted.py"], home.path()));
    for s in &servers {
        env.state.db.create_mcp_server(&space, s).await.unwrap();
    }

    // Control: in a folder no isolated bot used, nothing changes and the planted files do run, so
    // the checks below would see a run
    let (ctx, token) = mcp_runtime::prepare_turn(&env.state, &kwant, Some(free.clone())).await.unwrap();
    let mine = ours(&ctx.servers);
    assert_eq!(mine.len(), servers.len());
    assert!(mine.iter().all(|s| s.command.as_deref() != Some("/bin/sh")), "untouched outside a recorded folder");
    start_all(&mine, &free).await;
    mcp_runtime::finish_turn(&env.state, &token).await;
    let want: Vec<String> = if has_npx { vec!["npx-free".into(), "py-free".into()] } else { vec!["py-free".into()] };
    assert_eq!(ran(out.path()), want, "the control turn runs the planted files");
    for f in &want {
        std::fs::remove_file(out.path().join(f)).unwrap();
    }

    env.state.db.record_isolated_folder(&rec, "isolated-bot").await.unwrap();
    // The recorded folder itself, a child of it, and a folder holding it
    for cwd in [rec.clone(), child.clone(), work.path().canonicalize().unwrap()] {
        let (ctx, token) = mcp_runtime::prepare_turn(&env.state, &kwant, Some(cwd.clone())).await.unwrap();
        let empty = mcp_runtime::mcp_cwd(&env.state.config.data_dir, &token);
        let mine = ours(&ctx.servers);
        let names: Vec<&str> = mine.iter().map(|s| s.name.as_str()).collect();
        assert!(!names.contains(&"file"), "{cwd:?}: a server naming a file in the folder isn't started ({names:?})");
        assert_eq!(names.len(), servers.len() - 1, "{cwd:?}: {names:?}");
        for s in ctx.servers.iter().filter(|s| s.transport == "stdio") {
            assert_eq!(s.command.as_deref(), Some("/bin/sh"), "{cwd:?}: {s:?}");
            assert_eq!(s.args[2], empty.display().to_string(), "{cwd:?}: starts from the turn's empty folder");
        }
        assert!(empty.is_dir() && std::fs::read_dir(&empty).unwrap().next().is_none(), "{cwd:?}: a fresh, empty folder");
        assert_eq!(std::fs::metadata(&empty).unwrap().permissions().mode() & 0o777, 0o700);
        start_all(&mine, &cwd).await;
        mcp_runtime::finish_turn(&env.state, &token).await;
        assert!(!empty.exists(), "{cwd:?}: the empty folder goes with the turn");
        assert_eq!(ran(out.path()), Vec::<String>::new(), "{cwd:?}: no planted file ran");
    }

    // A turn with no folder whose own folder isn't near a recorded one is left alone
    let (ctx, token) = mcp_runtime::prepare_turn(&env.state, &kwant, None).await.unwrap();
    assert_eq!(ours(&ctx.servers).len(), servers.len(), "kwnt's own folder isn't a recorded one here");
    mcp_runtime::finish_turn(&env.state, &token).await;

    // The Test button: from <data>/workspace, once a bot used it, the same rule
    let ws = env.state.config.data_dir.join("workspace");
    std::fs::create_dir_all(&ws).unwrap();
    plant(&ws, out.path(), "ws");
    env.state.db.record_isolated_folder(&ws.canonicalize().unwrap(), "isolated-bot").await.unwrap();
    let list = ours(&env.state.db.list_mcp_servers(&space).await.unwrap());
    for s in &list {
        let started = Instant::now();
        let (status, v) = call(&env.app, "POST", &format!("/api/mcp/servers/{}/test", s.id), None).await;
        assert_eq!(status.as_u16(), 200, "{v}");
        assert_eq!(v["ok"], false, "{}: nothing real to list ({v})", s.name);
        if s.name == "file" {
            assert!(v["error"].as_str().unwrap().starts_with("Not started"), "{v}");
        }
        assert!(started.elapsed() < Duration::from_secs(30), "{}: no hang", s.name);
    }
    assert_eq!(ran(out.path()), Vec::<String>::new(), "the Test button ran no planted file");
    let left = env.state.config.data_dir.join("run").join("mcp-cwd");
    assert!(std::fs::read_dir(&left).map(|mut d| d.next().is_none()).unwrap_or(true), "no empty folder is left behind");
}
