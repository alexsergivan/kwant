//! The production floor in the approval path (plan §3.5): builds a space's [`DevCtx`], runs the
//! pure classifier in `crate::mcp::prod_floor`, and gives the card its `prod_floor` payload. The
//! approvers treat a hit as `forced` (it asks whatever the approvals say) with no "always allow";
//! [`allow_refusal`] is the server-side check that refuses **Allow** while a step needs a backup
//! and none is fresh.

use crate::db::Database;
use crate::mcp::prod_floor::{self, DevCtx, Floor};
use crate::mcp::ToolApprovalRequest;
use crate::server::gitfile::{self, plain_ref};
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};

/// The Dev mode context of a bot's space, or `None` when Dev mode is off there (no floor).
/// With Dev mode off in every space it reads nothing.
pub async fn ctx_for_kwant(db: &Database, kwant_id: &str, cwd: Option<&Path>) -> Option<(String, DevCtx)> {
    if !db.any_dev_mode_on() {
        return None;
    }
    let kwant = db.get_kwant(kwant_id).await.ok().flatten()?;
    let mode = db.dev_mode(&kwant.space_id).await.ok().flatten().filter(|m| m.on)?;
    // Every watched repo's branch is protected (T9: the floor stays space-wide). The first
    // repo's is the main one; with none, the branch a team folder named at switch-on.
    let repos = db.dev_repos(&kwant.space_id).await.unwrap_or_default();
    let mut branches: Vec<String> = Vec::new();
    for b in repos.iter().map(|r| r.branch.trim()).filter(|b| !b.is_empty()) {
        if !branches.iter().any(|x| x == b) {
            branches.push(b.to_string());
        }
    }
    let branch = if branches.is_empty() { mode.branch.clone() } else { branches.remove(0) };
    let deploy_hint = repos.iter().find_map(|r| r.deploy_hint.clone()).or(mode.deploy_hint.clone());
    let marked = db.prod_connections(&kwant.space_id).await.unwrap_or_default();
    let servers = db.list_mcp_servers(&crate::db::spaces::SpaceId::new(&kwant.space_id)).await.unwrap_or_default();
    let prod_connections = prod_names(&marked, servers.iter().map(|s| (s.id.as_str(), s.name.as_str())));
    let ctx = DevCtx {
        branch,
        protected: branches,
        deploy_hint,
        prod_connections,
        head_branch: cwd.and_then(head_branch),
        migration: None,
    };
    Some((kwant.space_id, ctx))
}

/// Marked connection ids as the classifier names them: `mcp:<id>` becomes the server's name,
/// `composio:<slug>` the toolkit slug. A mark whose server is gone is dropped.
pub fn prod_names<'a>(marked: &[String], servers: impl Iterator<Item = (&'a str, &'a str)>) -> Vec<String> {
    let servers: Vec<(&str, &str)> = servers.collect();
    marked
        .iter()
        .filter_map(|m| {
            if let Some(id) = m.strip_prefix("mcp:") {
                servers.iter().find(|(sid, _)| *sid == id).map(|(_, name)| name.to_string())
            } else {
                m.strip_prefix("composio:").map(str::to_string)
            }
        })
        .collect()
}

/// The branch checked out in `cwd` (walking up to its `.git`), from `HEAD` alone: no git process.
/// `None` for a detached head or no repo.
///
/// `cwd` can be a bot's folder, so its `.git` is untrusted: every file is read through
/// [`gitfile::read`] (no symlink, FIFO, hard link or big file, so this never blocks), a `.git`
/// that is a symlink is no repo, a worktree's `gitdir:` is followed only as [`worktree_head`]
/// allows, and the branch must be a plain ref.
pub fn head_branch(cwd: &Path) -> Option<String> {
    let mut dir = Some(cwd);
    for _ in 0..12 {
        let d = dir?;
        let head = match std::fs::symlink_metadata(d.join(".git")) {
            Err(_) => {
                dir = d.parent();
                continue;
            }
            Ok(m) if m.is_dir() => gitfile::read(&d.join(".git"), "HEAD", gitfile::REF_MAX)?,
            Ok(m) if m.is_file() => worktree_head(d)?,
            Ok(_) => return None,
        };
        let r = head.trim().strip_prefix("ref: ")?.trim();
        return r.strip_prefix("refs/heads/").filter(|_| plain_ref(r)).map(str::to_string);
    }
    None
}

/// `HEAD` of the worktree whose `.git` file is in `d`. The file must say
/// `gitdir: <repo>/.git/worktrees/<name>` (relative to `d` unless absolute). `.` and `..` are
/// resolved on the text, so only a path that was checked is opened, and every folder on it that
/// `d`'s own path doesn't already name (always `.git`, `worktrees` and `<name>`) must be a real
/// folder, not a symlink. So a common sibling worktree reads, and a planted link can't redirect it.
fn worktree_head(d: &Path) -> Option<String> {
    let text = gitfile::read(d, ".git", gitfile::REF_MAX)?;
    let gd = text.trim().strip_prefix("gitdir:")?.trim();
    let here = lexical(d)?;
    let target = lexical(&d.join(gd))?;
    let parts: Vec<Component> = target.components().collect();
    let [.., Component::Normal(g), Component::Normal(w), Component::Normal(_)] = parts[..] else {
        return None;
    };
    if g.to_str() != Some(".git") || w.to_str() != Some("worktrees") {
        return None;
    }
    let shared = parts.iter().zip(here.components()).take_while(|(a, b)| **a == *b).count().min(parts.len() - 3);
    let mut at: PathBuf = parts[..shared].iter().collect();
    for c in &parts[shared..] {
        at.push(c);
        if !std::fs::symlink_metadata(&at).ok()?.file_type().is_dir() {
            return None;
        }
    }
    gitfile::read(&target, "HEAD", gitfile::REF_MAX)
}

/// `p` with `.` dropped and `..` taken off the text (never past its start), not by the kernel.
fn lexical(p: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            c => out.push(c),
        }
    }
    Some(out)
}

/// The card's `prod_floor` object: `{kind, why, blast, needs_backup, space_id, backup}`.
pub async fn payload(db: &Database, space_id: &str, floor: &Floor, ctx: &DevCtx) -> Value {
    let mut v = serde_json::to_value(floor).unwrap_or_else(|_| json!({}));
    v["space_id"] = json!(space_id);
    v["backup"] = super::backup::state_of(db, space_id, ctx.no_data()).await;
    v
}

/// The floor for a tool call (an MCP tool, or one of claude's own), with its payload.
pub async fn for_call(db: &Database, req: &ToolApprovalRequest) -> Option<(Floor, Value)> {
    let cwd = req.builtin.as_ref().and_then(|b| b.cwd.as_deref()).map(Path::new);
    let (space_id, ctx) = ctx_for_kwant(db, &req.kwant_id, cwd).await?;
    let floor = prod_floor::reason(req, &ctx)?;
    let p = payload(db, &space_id, &floor, &ctx).await;
    Some((floor, p))
}

/// The floor for a command kwant's own shell runs for a bot (`execute_shell`).
pub async fn for_shell(db: &Database, kwant_id: &str, command: &str, cwd: &Path) -> Option<(Floor, Value)> {
    let (space_id, ctx) = ctx_for_kwant(db, kwant_id, Some(cwd)).await?;
    let floor = prod_floor::shell_reason(command, &ctx)?;
    let p = payload(db, &space_id, &floor, &ctx).await;
    Some((floor, p))
}

/// Why **Allow** is refused on this card now, or `None`: the card carries a `prod_floor` that
/// needs a backup, and the space has no `ok` backup at most 10 minutes old. Checked when the
/// person answers, not when the card was made, so a backup taken in between counts and one that
/// went old doesn't.
pub async fn allow_refusal(db: &Database, details: &Value) -> Option<Value> {
    let f = details.get("prod_floor")?;
    if f["needs_backup"] != true {
        return None;
    }
    let space_id = f["space_id"].as_str()?;
    let (_, ok) = db.latest_dev_backups(space_id).await.unwrap_or((None, None));
    if super::backup::is_fresh(ok.as_ref(), super::now()) {
        return None;
    }
    let backup = super::backup::state_of(db, space_id, false).await;
    Some(json!({
        "error": "Back up first: this step needs a backup from the last 10 minutes",
        "backup": backup,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_become_server_names_and_slugs() {
        let marked = vec!["mcp:srv_1".to_string(), "mcp:gone".into(), "composio:github".into(), "account:p1".into()];
        let names = prod_names(&marked, [("srv_1", "cloudflare"), ("srv_2", "notion")].into_iter());
        assert_eq!(names, ["cloudflare", "github"]);
    }

    #[test]
    fn the_checked_out_branch_is_read_from_head() {
        let root = std::env::temp_dir().join(format!("kwnt-dev-head-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/lin-dev-core\n").unwrap();
        assert_eq!(head_branch(&root.join("src/deep")).as_deref(), Some("lin-dev-core"));
        std::fs::write(root.join(".git/HEAD"), "3ca2bf7aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n").unwrap();
        assert_eq!(head_branch(&root), None, "a detached head can't be told");
        // A worktree's `.git` file
        let wt = root.join("wt");
        std::fs::create_dir_all(root.join(".git/worktrees/wt")).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", root.join(".git/worktrees/wt").display())).unwrap();
        std::fs::write(root.join(".git/worktrees/wt/HEAD"), "ref: refs/heads/release/0.7\n").unwrap();
        assert_eq!(head_branch(&wt).as_deref(), Some("release/0.7"));
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(head_branch(Path::new("/nonexistent/kwnt-dev")), None);
    }

    /// A repo with `.git/HEAD` on `lin-dev-core` in a temp folder.
    fn head_repo() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("repo");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/lin-dev-core\n").unwrap();
        (d, root)
    }

    /// `head_branch` on another thread, failing (not hanging) after 5 s.
    fn head_within_5s(cwd: &Path) -> Option<String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let cwd = cwd.to_path_buf();
        std::thread::spawn(move || tx.send(head_branch(&cwd)).unwrap());
        rx.recv_timeout(std::time::Duration::from_secs(5)).expect("head_branch blocked")
    }

    #[cfg(unix)]
    fn mkfifo(p: &Path) {
        let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
    }

    /// F-1: a planted FIFO as HEAD, as a worktree's `.git` file or as its HEAD never blocks the
    /// approval check.
    #[cfg(unix)]
    #[test]
    fn head_branch_does_not_block_on_a_fifo() {
        let (d, root) = head_repo();
        std::fs::remove_file(root.join(".git/HEAD")).unwrap();
        mkfifo(&root.join(".git/HEAD"));
        assert_eq!(head_within_5s(&root), None, "HEAD FIFO");
        let wt = d.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        mkfifo(&wt.join(".git"));
        assert_eq!(head_within_5s(&wt), None, ".git FIFO");
        std::fs::remove_file(wt.join(".git")).unwrap();
        std::fs::create_dir_all(root.join(".git/worktrees/wt")).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", root.join(".git/worktrees/wt").display())).unwrap();
        mkfifo(&root.join(".git/worktrees/wt/HEAD"));
        assert_eq!(head_within_5s(&wt), None, "worktree HEAD FIFO");
    }

    /// F-2: a HEAD that is another name for some other file isn't read.
    #[cfg(unix)]
    #[test]
    fn head_branch_never_reads_a_hard_link() {
        let (d, root) = head_repo();
        let host = d.path().join("host-file");
        std::fs::write(&host, "ref: refs/heads/LEAKED-FROM-OUTSIDE\n").unwrap();
        std::fs::remove_file(root.join(".git/HEAD")).unwrap();
        std::fs::hard_link(&host, root.join(".git/HEAD")).unwrap();
        assert_eq!(head_within_5s(&root), None);
    }

    #[cfg(unix)]
    #[test]
    fn head_branch_never_follows_a_symlink() {
        use std::os::unix::fs::symlink;
        let (d, root) = head_repo();
        let host = d.path().join("host-file");
        std::fs::write(&host, "ref: refs/heads/LEAKED-FROM-OUTSIDE\n").unwrap();
        std::fs::remove_file(root.join(".git/HEAD")).unwrap();
        symlink(&host, root.join(".git/HEAD")).unwrap();
        assert_eq!(head_branch(&root), None, "HEAD symlink");
        // `.git` as a symlink to another repo, or to a `gitdir:` file
        let (_d2, other) = head_repo();
        let linked = d.path().join("linked");
        std::fs::create_dir_all(&linked).unwrap();
        symlink(other.join(".git"), linked.join(".git")).unwrap();
        assert_eq!(head_branch(&linked), None, ".git symlink to a folder");
        assert_eq!(head_branch(&linked.join("sub")), None, "and not from below it either");
        std::fs::remove_file(linked.join(".git")).unwrap();
        std::fs::write(d.path().join("gitfile"), format!("gitdir: {}\n", other.join(".git").display())).unwrap();
        symlink(d.path().join("gitfile"), linked.join(".git")).unwrap();
        assert_eq!(head_branch(&linked), None, ".git symlink to a file");
        // A worktree whose gitdir goes through a planted folder link
        let wt = d.path().join("wt");
        std::fs::create_dir_all(other.join(".git/worktrees/wt")).unwrap();
        std::fs::write(other.join(".git/worktrees/wt/HEAD"), "ref: refs/heads/LEAKED-FROM-OUTSIDE\n").unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        symlink(other.parent().unwrap(), wt.join("hop")).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: hop/repo/.git/worktrees/wt\n").unwrap();
        assert_eq!(head_branch(&wt), None, "gitdir through a symlinked folder");
    }

    #[test]
    fn head_branch_only_follows_a_worktree_gitdir() {
        let (d, root) = head_repo();
        let wt = d.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        for bad in [
            format!("gitdir: {}", root.join(".git").display()),
            format!("gitdir: {}", root.display()),
            "gitdir: ../repo/.git/worktrees".into(),
            "gitdir: ../../../../../../../../../../../../../../../../../../.git/worktrees/x".into(),
            "worktree pointer".into(),
        ] {
            std::fs::create_dir_all(root.join(".git/worktrees/x")).unwrap();
            std::fs::write(root.join(".git/worktrees/x/HEAD"), "ref: refs/heads/x\n").unwrap();
            std::fs::write(wt.join(".git"), &bad).unwrap();
            assert_eq!(head_branch(&wt), None, "{bad}");
        }
        // A relative gitdir (what `worktree.useRelativePaths` writes) reads
        std::fs::write(wt.join(".git"), "gitdir: ../repo/.git/worktrees/x\n").unwrap();
        assert_eq!(head_branch(&wt).as_deref(), Some("x"));
        // The branch must be a plain ref
        for head in ["ref: refs/heads/../../x", "ref: refs/heads/a b", "ref: refs/tags/v1", "ref: refs/heads/"] {
            std::fs::write(root.join(".git/HEAD"), head).unwrap();
            assert_eq!(head_branch(&root), None, "{head}");
        }
    }

    /// A worktree beside its repo, made by git itself, still reads its own branch.
    #[test]
    fn head_branch_reads_a_real_sibling_worktree() {
        let d = tempfile::tempdir().unwrap();
        let git = |dir: &Path, args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"])
                .args(args)
                .current_dir(dir)
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        };
        let main = d.path().join("main");
        std::fs::create_dir_all(&main).unwrap();
        git(&main, &["init", "-q"]);
        git(&main, &["commit", "-q", "--allow-empty", "-m", "x"]);
        let wt = d.path().join("main-wt");
        git(&main, &["worktree", "add", "-q", "-b", "juno/side-1", wt.to_str().unwrap()]);
        assert_eq!(head_branch(&main).as_deref(), Some("main"));
        assert_eq!(head_branch(&wt).as_deref(), Some("juno/side-1"));
        std::fs::create_dir_all(wt.join("src")).unwrap();
        assert_eq!(head_branch(&wt.join("src")).as_deref(), Some("juno/side-1"));
    }

    fn backup(space: &str, ok: bool, age_s: i64) -> crate::db::dev_mode::DevBackupRow {
        crate::db::dev_mode::DevBackupRow {
            id: format!("dvb_{}", uuid::Uuid::new_v4().simple()),
            space_id: space.into(),
            kwant_id: None,
            path: Some("/tmp/demo.sql".into()),
            bytes: Some(10),
            ok,
            error: (!ok).then(|| "exit 1".into()),
            taken_at: chrono::Utc::now() - chrono::Duration::seconds(age_s),
        }
    }

    #[tokio::test]
    async fn allow_is_refused_until_a_fresh_ok_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let card = json!({"prod_floor": {"kind": "migration", "needs_backup": true, "space_id": "s1"}});
        // No backup at all, and then no backup command either
        let r = allow_refusal(&db, &card).await.expect("refused with no backup");
        assert!(r["error"].as_str().unwrap().starts_with("Back up first"));
        assert_eq!(r["backup"]["state"], "no_command");
        db.put_dev_mode("s1", true, "demo-org/demo-app", "main", None, Some("pg_dump demo > /tmp/demo.sql")).await.unwrap();
        assert_eq!(allow_refusal(&db, &card).await.unwrap()["backup"]["state"], "none");
        // Only a failed one, and an ok one 11 minutes old
        db.insert_dev_backup(&backup("s1", false, 5)).await.unwrap();
        db.insert_dev_backup(&backup("s1", true, 660)).await.unwrap();
        assert!(allow_refusal(&db, &card).await.is_some(), "a failed or old backup doesn't count");
        // Another space's fresh backup doesn't count either
        db.insert_dev_backup(&backup("s2", true, 5)).await.unwrap();
        assert!(allow_refusal(&db, &card).await.is_some());
        // A fresh ok one in this space does
        db.insert_dev_backup(&backup("s1", true, 30)).await.unwrap();
        assert!(allow_refusal(&db, &card).await.is_none());
        // A step that needs no backup, or a card that isn't the floor's, is never refused here
        assert!(allow_refusal(&db, &json!({"prod_floor": {"needs_backup": false, "space_id": "s3"}})).await.is_none());
        assert!(allow_refusal(&db, &json!({"source": "mcp"})).await.is_none());
    }

    #[tokio::test]
    async fn production_marks_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        db.set_prod_connection("s1", "composio:github", true).await.unwrap();
        db.set_prod_connection("s1", "composio:github", true).await.unwrap();
        db.set_prod_connection("s1", "mcp:srv_1", true).await.unwrap();
        db.set_prod_connection("s2", "composio:notion", true).await.unwrap();
        let mut marked = db.prod_connections("s1").await.unwrap();
        marked.sort();
        assert_eq!(marked, ["composio:github", "mcp:srv_1"]);
        db.set_prod_connection("s1", "composio:github", false).await.unwrap();
        assert_eq!(db.prod_connections("s1").await.unwrap(), ["mcp:srv_1"]);
        // Off: no context, so no floor
        assert!(ctx_for_kwant(&db, "no-such-bot", None).await.is_none());
    }

    #[tokio::test]
    async fn the_dev_mode_flag_is_read_at_startup() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        assert!(!db.any_dev_mode_on());
        db.put_dev_mode("s1", true, "demo-org/demo-app", "main", None, None).await.unwrap();
        drop(db);
        let db = Database::init(tmp.path()).await.unwrap();
        assert!(db.any_dev_mode_on(), "a restart with a space on starts with the floor on");
        db.put_dev_mode("s1", false, "demo-org/demo-app", "main", None, None).await.unwrap();
        drop(db);
        assert!(!Database::init(tmp.path()).await.unwrap().any_dev_mode_on());
    }
}
