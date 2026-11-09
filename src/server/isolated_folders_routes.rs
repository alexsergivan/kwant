//! Folders isolated Claude turns used, and forgetting one (#446 item 6, 0.8.0 section F).
//!
//! Every folder a reply that ran Claude inside the VM used is recorded (`db::isolated_folders`),
//! and kwnt then treats it as untrusted on this computer: Codex and Grok don't start there, host
//! commands are refused, Claude starts without the folder's own settings. Only the person can end
//! that, after looking at what a bot may have left behind. Contract:
//! `release-080/forget-folder-contract.md`.
//!
//! - `GET /api/isolated-folders`: the recorded folders, with the files to check first.
//! - `POST /api/isolated-folders/forget` `{ "path": … }`: forgets that one record.
//!
//! Person-only, every method (`auth::person_only`): a bot that could forget a folder could plant
//! a hook in it and then have a host CLI load it.

use super::AppState;
use crate::db::isolated_folders::IsolatedFolder;
use crate::sandbox::safe_fs::{Dir, Kind};
use axum::{
    extract::{rejection::JsonRejection, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Hook files listed per folder, at most (a folder full of them still shows the first ones).
const MAX_HOOKS: usize = 20;

/// Files a program kwnt or the person starts in the folder would load or run, by folder.
const LOADED_FILES: &[(&str, &[&str])] = &[
    ("", &[".mcp.json", ".envrc"]),
    (".claude", &["settings.json", "settings.local.json"]),
    (".codex", &["config.toml"]),
    (".grok", &["settings.json", "config.toml"]),
    (".git", &["config"]),
];

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(json!({ "error": message, "code": code }))).into_response()
}

fn internal(e: impl std::fmt::Display) -> Response {
    tracing::warn!(error = %e, "isolated folders: database error");
    error(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Couldn't read the list of folders. Try again.")
}

#[derive(Serialize, Debug, PartialEq)]
pub struct FolderView {
    pub path: String,
    pub first_bot_id: String,
    /// `null` when that bot was deleted since
    pub first_bot_name: Option<String>,
    pub first_used_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: chrono::DateTime<chrono::Utc>,
    /// `false` when the folder was deleted or moved: it can still be forgotten
    pub exists: bool,
    /// Paths inside the folder (relative, `/`-separated) that a CLI, git or the person's shell
    /// started there would load. A bot in the VM could have written them; look before forgetting.
    /// A symlink is listed with ` -> link` (never followed). A `.git` or `.git/hooks` that isn't a
    /// real folder is listed too (`.git (file)`, `.git/hooks -> link`, …): git then loads hooks
    /// from somewhere kwnt doesn't look.
    pub check_first: Vec<String>,
}

/// What [`LOADED_FILES`] and `.git/hooks` hold in `folder`. Read without following any link
/// below the folder (a bot made them), names only, never contents.
pub fn check_first(folder: &Path) -> (bool, Vec<String>) {
    let Ok(root) = Dir::open(folder) else {
        return (false, Vec::new());
    };
    let mut out = Vec::new();
    let show = |dir: &str, name: &str, kind: Kind| {
        let rel = if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") };
        if kind == Kind::Symlink {
            format!("{rel} -> link")
        } else {
            rel
        }
    };
    let entries = |dir: &str, max: usize| -> Vec<(String, Kind)> {
        root.open_dir(Path::new(dir))
            .and_then(|d| d.list_at_most(max))
            .map(|(es, _)| es.into_iter().map(|e| (e.name.to_string_lossy().into_owned(), e.kind)).collect())
            .unwrap_or_default()
    };
    // A folder may hold many entries; only the named ones matter, so read each (bounded)
    let top = entries("", 10_000);
    for (dir, names) in LOADED_FILES {
        let found = if dir.is_empty() { top.clone() } else { entries(dir, 10_000) };
        for name in *names {
            if let Some((_, kind)) = found.iter().find(|(n, _)| n == name) {
                out.push(show(dir, name, *kind));
            }
        }
        // `.claude`, `.codex`, … as a link: everything in it comes from somewhere else
        if !dir.is_empty() {
            if let Some((_, Kind::Symlink)) = top.iter().find(|(n, _)| n == dir) {
                out.push(show("", dir, Kind::Symlink));
            }
        }
    }
    // #462 review must-fix 2: a `.git` file (`gitdir: .x`, the "rename `.git`" way around the
    // read-only hooks) or a `hooks` that isn't a real folder sends git to hooks this list can't
    // see, so say so instead of showing nothing
    let not_a_folder = |rel: &str, kind: Kind| match kind {
        Kind::Dir => None,
        Kind::Symlink => Some(format!("{rel} -> link")),
        Kind::File => Some(format!("{rel} (file)")),
        Kind::Other => Some(format!("{rel} (not a folder)")),
    };
    match top.iter().find(|(n, _)| n == ".git").map(|(_, k)| *k) {
        Some(Kind::Dir) => {
            if let Some((_, kind)) = entries(".git", 10_000).into_iter().find(|(n, _)| n == "hooks") {
                out.extend(not_a_folder(".git/hooks", kind));
            }
        }
        // A linked `.git` is already listed as `.git -> link` by the loop above
        Some(Kind::Symlink) | None => {}
        Some(kind) => out.extend(not_a_folder(".git", kind)),
    }
    let mut hooks: Vec<(String, Kind)> = entries(".git/hooks", 1_000).into_iter().filter(|(n, _)| !n.ends_with(".sample")).collect();
    hooks.sort_by(|a, b| a.0.cmp(&b.0));
    let more = hooks.len().saturating_sub(MAX_HOOKS);
    out.extend(hooks.into_iter().take(MAX_HOOKS).map(|(n, k)| show(".git/hooks", &n, k)));
    if more > 0 {
        out.push(format!(".git/hooks: {more} more"));
    }
    out.dedup();
    (true, out)
}

async fn views(state: &AppState, records: Vec<IsolatedFolder>) -> Vec<FolderView> {
    let mut names: HashMap<String, Option<String>> = HashMap::new();
    let mut out = Vec::with_capacity(records.len());
    for r in records {
        if !names.contains_key(&r.first_bot_id) {
            let name = state.db.get_kwant(&r.first_bot_id).await.ok().flatten().map(|k| k.name);
            names.insert(r.first_bot_id.clone(), name);
        }
        let path = r.path.clone();
        let (exists, check) = tokio::task::spawn_blocking(move || check_first(&path)).await.unwrap_or((false, Vec::new()));
        out.push(FolderView {
            path: r.path.to_string_lossy().into_owned(),
            first_bot_name: names.get(&r.first_bot_id).cloned().flatten(),
            first_bot_id: r.first_bot_id,
            first_used_at: r.first_used_at,
            last_used_at: r.last_used_at,
            exists,
            check_first: check,
        });
    }
    out
}

/// `GET /api/isolated-folders`
pub async fn list(State(state): State<AppState>) -> Response {
    match state.db.isolated_folder_records().await {
        Ok(records) => Json(json!({ "folders": views(&state, records).await })).into_response(),
        Err(e) => internal(e),
    }
}

/// `POST /api/isolated-folders/forget` `{ "path": "<as listed>" }`
pub async fn forget(State(state): State<AppState>, body: Result<Json<Value>, JsonRejection>) -> Response {
    let bad = || error(StatusCode::BAD_REQUEST, "invalid", "Send the folder's path, as the list shows it.");
    let Ok(Json(body)) = body else { return bad() };
    let Some(path) = body.get("path").and_then(Value::as_str) else { return bad() };
    if path.is_empty() || path.len() > 4096 || path.contains('\0') || !Path::new(path).is_absolute() {
        return bad();
    }
    let path = PathBuf::from(path);
    match state.db.forget_isolated_folder(&path).await {
        Ok(true) => {
            tracing::info!(folder = %path.display(), "the person forgot a folder an isolated bot used");
            let rest = match state.db.isolated_folder_records().await {
                Ok(r) => views(&state, r).await,
                Err(e) => return internal(e),
            };
            Json(json!({ "forgotten": path.to_string_lossy(), "folders": rest })).into_response()
        }
        Ok(false) => error(StatusCode::NOT_FOUND, "not_recorded", "That folder isn't on the list (it may have been forgotten already)."),
        Err(e) => internal(e),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn check_first_names_what_a_cli_or_git_would_load_without_following_links() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path();
        assert_eq!(check_first(f), (true, vec![]), "an empty folder has nothing to check");
        std::fs::create_dir_all(f.join(".claude")).unwrap();
        std::fs::write(f.join(".claude/settings.json"), "{}").unwrap();
        std::fs::write(f.join(".mcp.json"), "{}").unwrap();
        std::fs::create_dir_all(f.join(".git/hooks")).unwrap();
        std::fs::write(f.join(".git/hooks/pre-commit.sample"), "").unwrap();
        std::fs::write(f.join(".git/hooks/post-checkout"), "#!/bin/sh").unwrap();
        std::fs::write(f.join(".git/config"), "").unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::write(elsewhere.path().join("config.toml"), "").unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), f.join(".codex")).unwrap();
        let (exists, got) = check_first(f);
        assert!(exists);
        assert_eq!(got, vec![".mcp.json", ".claude/settings.json", ".codex -> link", ".git/config", ".git/hooks/post-checkout"]);
        // A folder that's gone
        assert_eq!(check_first(&f.join("nope")), (false, vec![]));
    }

    // #462 review must-fix 2: when git is sent elsewhere, the list says so instead of looking empty
    #[test]
    fn check_first_names_a_git_or_hooks_that_isnt_a_real_folder() {
        let d = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        // `.git` is a file (`gitdir: .x`, the renamed-.git bypass)
        let f = d.path().join("gitfile");
        std::fs::create_dir_all(f.join(".x/hooks")).unwrap();
        std::fs::write(f.join(".x/hooks/pre-commit"), "#!/bin/sh").unwrap();
        std::fs::write(f.join(".git"), "gitdir: .x\n").unwrap();
        assert_eq!(check_first(&f), (true, vec![".git (file)".to_string()]));
        // `.git` is a link: named once
        let f = d.path().join("gitlink");
        std::fs::create_dir_all(&f).unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), f.join(".git")).unwrap();
        assert_eq!(check_first(&f).1, vec![".git -> link"]);
        // `.git/hooks` is a link
        let f = d.path().join("hookslink");
        std::fs::create_dir_all(f.join(".git")).unwrap();
        std::fs::write(f.join(".git/config"), "").unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), f.join(".git/hooks")).unwrap();
        assert_eq!(check_first(&f).1, vec![".git/config", ".git/hooks -> link"]);
        // `.git/hooks` is a file
        let f = d.path().join("hooksfile");
        std::fs::create_dir_all(f.join(".git")).unwrap();
        std::fs::write(f.join(".git/hooks"), "").unwrap();
        assert_eq!(check_first(&f).1, vec![".git/hooks (file)"]);
        // A real `.git/hooks` with only samples still shows nothing
        let f = d.path().join("plain");
        std::fs::create_dir_all(f.join(".git/hooks")).unwrap();
        std::fs::write(f.join(".git/hooks/pre-commit.sample"), "").unwrap();
        assert_eq!(check_first(&f).1, Vec::<String>::new());
    }

    #[test]
    fn many_hooks_are_cut_with_a_count() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".git/hooks")).unwrap();
        for i in 0..(MAX_HOOKS + 3) {
            std::fs::write(d.path().join(format!(".git/hooks/h{i:02}")), "").unwrap();
        }
        let (_, got) = check_first(d.path());
        assert_eq!(got.len(), MAX_HOOKS + 1);
        assert_eq!(got.last().unwrap(), ".git/hooks: 3 more");
    }
}
