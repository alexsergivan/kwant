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
use crate::sandbox::safe_fs::Dir;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// The Dev mode context of a bot's space, or `None` when Dev mode is off there (no floor).
/// With Dev mode off in every space it reads nothing. `at` is `(folder, cwd)`: the bot's folder
/// for the turn and the working folder of the call, which [`head_branch`] keeps inside it.
pub async fn ctx_for_kwant(db: &Database, kwant_id: &str, at: Option<(&Path, &Path)>) -> Option<(String, DevCtx)> {
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
        head_branch: at.and_then(|(folder, cwd)| head_branch(folder, cwd)),
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

/// The branch checked out in `cwd` (walking up to its `.git`, but never above `folder`), from
/// `HEAD` alone: no git process. `None` for a detached head, no repo, or anything doubtful (a bare
/// push then counts as one to a protected branch, so `None` asks).
///
/// `folder` is the bot's folder for the turn, the host's own path; `cwd` and everything under
/// `folder` are untrusted. The rules (0.7.7, S-8):
/// - **Q3:** a `cwd` that isn't `folder` or below it (by its text: no `..`) is refused before
///   any `.git` is read, and no `.git` above `folder` is read.
/// - Every folder and file is reached by one openat walk from `folder` ([`Dir`]): no symlink is
///   followed anywhere below `folder` (a planted link makes this `None`), no FIFO blocks it, and
///   every file goes through [`gitfile::read`] (no hard link, no big file).
/// - **P1:** a `.git` folder counts as a repo only with `objects/` and `refs/` folders in it, as
///   git requires. Without them git uses the repo above, and so does this.
/// - A worktree's `.git` file is followed only as [`worktree_head`] allows (**A4**: never out of
///   `folder`), and the branch must be a plain ref.
pub fn head_branch(folder: &Path, cwd: &Path) -> Option<String> {
    let mut parts = rel_inside(folder, cwd)?;
    let root = Dir::open(folder).ok()?;
    loop {
        let d = root.open_dir(&joined(&parts)).ok()?;
        let head = match d.open_dir(Path::new(".git")) {
            Ok(git) => match repo_dirs(&git)? {
                true => gitfile::read(&git, "HEAD", gitfile::REF_MAX)?,
                false => {
                    parts.pop()?;
                    continue;
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                parts.pop()?;
                continue;
            }
            // A file (a worktree's pointer), or a symlink, FIFO or anything else: never followed
            Err(_) => worktree_head(&root, folder, &parts, &gitfile::read(&d, ".git", gitfile::REF_MAX)?)?,
        };
        let r = head.trim().strip_prefix("ref: ")?.trim();
        return r.strip_prefix("refs/heads/").filter(|_| plain_ref(r)).map(str::to_string);
    }
}

/// P1: whether an open `.git` folder is a repo to git: `Some(true)` with real `objects/` and
/// `refs/` folders, `Some(false)` when either is missing (git then looks above), `None` when one
/// is there but not a real folder (a symlink git would follow: doubtful, so no answer).
fn repo_dirs(git: &Dir) -> Option<bool> {
    for sub in ["objects", "refs"] {
        match git.open_dir(Path::new(sub)) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(false),
            Err(_) => return None,
        }
    }
    Some(true)
}

/// Q3: `cwd`'s parts below `folder`, when `cwd` names `folder` (as given, or with the person's
/// own symlinks resolved) or a folder below it with plain names only.
fn rel_inside(folder: &Path, cwd: &Path) -> Option<Vec<OsString>> {
    let canon = folder.canonicalize().ok();
    let rest = cwd.strip_prefix(folder).ok().or_else(|| cwd.strip_prefix(canon.as_deref()?).ok())?;
    rest.components()
        .filter(|c| !matches!(c, Component::CurDir))
        .map(|c| match c {
            Component::Normal(p) => Some(p.to_os_string()),
            _ => None,
        })
        .collect()
}

fn joined(parts: &[OsString]) -> PathBuf {
    parts.iter().collect()
}

/// `HEAD` of the worktree whose `.git` file (holding `text`) is in `folder/<at>`. The file must
/// say `gitdir: <repo>/.git/worktrees/<name>`, relative to that folder or absolute.
///
/// **A4:** the gitdir must stay inside `folder`: an absolute one must start with `folder`, and
/// a relative one may not climb above it. It is resolved on the text, but a `..` only takes off
/// a part that opens as a real folder: the kernel and git take `link/..` to the parent of the
/// link's *target*, so a `..` over a symlink makes this `None` rather than read a different
/// `HEAD` than git would (F-3). The result is then opened by the same openat walk from `folder`,
/// so every part of it, `.git`, `worktrees` and `<name>` included, is a real folder.
fn worktree_head(root: &Dir, folder: &Path, at: &[OsString], text: &str) -> Option<String> {
    let gd = Path::new(text.trim().strip_prefix("gitdir:")?.trim());
    let (mut stack, rest): (Vec<OsString>, &Path) = if gd.is_absolute() {
        let canon = folder.canonicalize().ok();
        (Vec::new(), gd.strip_prefix(folder).ok().or_else(|| gd.strip_prefix(canon.as_deref()?).ok())?)
    } else {
        (at.to_vec(), gd)
    };
    for c in rest.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                // Above `folder` (A4), or over a part that isn't a real folder (F-3)
                if stack.is_empty() || root.open_dir(&joined(&stack)).is_err() {
                    return None;
                }
                stack.pop();
            }
            Component::Normal(p) => stack.push(p.to_os_string()),
            _ => return None,
        }
    }
    let [.., g, w, _] = &stack[..] else { return None };
    if g != ".git" || w != "worktrees" {
        return None;
    }
    gitfile::read(&root.open_dir(&joined(&stack)).ok()?, "HEAD", gitfile::REF_MAX)
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
    // The call's folder is the turn's own folder, set by the host (`PermissionSession::cwd`),
    // never one the bot names, so it is the bot folder too
    let cwd = req.builtin.as_ref().and_then(|b| b.cwd.as_deref()).map(Path::new);
    let (space_id, ctx) = ctx_for_kwant(db, &req.kwant_id, cwd.map(|c| (c, c))).await?;
    let floor = prod_floor::reason(req, &ctx)?;
    let p = payload(db, &space_id, &floor, &ctx).await;
    Some((floor, p))
}

/// The floor for a command kwant's own shell runs for a bot (`execute_shell`).
pub async fn for_shell(db: &Database, kwant_id: &str, command: &str, cwd: &Path) -> Option<(Floor, Value)> {
    // kwant's shell runs in the bot's own folder (`execute_tool` picks it), so it is both
    let (space_id, ctx) = ctx_for_kwant(db, kwant_id, Some((cwd, cwd))).await?;
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
        std::fs::create_dir_all(root.join(".git/objects")).unwrap();
        std::fs::create_dir_all(root.join(".git/refs")).unwrap();
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/lin-dev-core\n").unwrap();
        assert_eq!(in_tmp(&root.join("src/deep")).as_deref(), Some("lin-dev-core"));
        std::fs::write(root.join(".git/HEAD"), "3ca2bf7aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n").unwrap();
        assert_eq!(in_tmp(&root), None, "a detached head can't be told");
        // A worktree's `.git` file
        let wt = root.join("wt");
        std::fs::create_dir_all(root.join(".git/worktrees/wt")).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", root.join(".git/worktrees/wt").display())).unwrap();
        std::fs::write(root.join(".git/worktrees/wt/HEAD"), "ref: refs/heads/release/0.7\n").unwrap();
        assert_eq!(in_tmp(&wt).as_deref(), Some("release/0.7"));
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(in_tmp(Path::new("/nonexistent/kwnt-dev")), None);
    }

    /// [`head_branch`] with the whole temp folder as the bot folder, so these cases only test the
    /// walk inside it (Q3 and A4 have their own tests below).
    fn in_tmp(cwd: &Path) -> Option<String> {
        head_branch(&std::env::temp_dir(), cwd)
    }

    /// A repo with `.git/HEAD` on `lin-dev-core` in a temp folder.
    fn head_repo() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("repo");
        std::fs::create_dir_all(root.join(".git/objects")).unwrap();
        std::fs::create_dir_all(root.join(".git/refs")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/lin-dev-core\n").unwrap();
        (d, root)
    }

    /// `head_branch` on another thread, failing (not hanging) after 5 s.
    fn head_within_5s(cwd: &Path) -> Option<String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let cwd = cwd.to_path_buf();
        std::thread::spawn(move || tx.send(in_tmp(&cwd)).unwrap());
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
        assert_eq!(in_tmp(&root), None, "HEAD symlink");
        // `.git` as a symlink to another repo, or to a `gitdir:` file
        let (_d2, other) = head_repo();
        let linked = d.path().join("linked");
        std::fs::create_dir_all(&linked).unwrap();
        symlink(other.join(".git"), linked.join(".git")).unwrap();
        assert_eq!(in_tmp(&linked), None, ".git symlink to a folder");
        assert_eq!(in_tmp(&linked.join("sub")), None, "and not from below it either");
        std::fs::remove_file(linked.join(".git")).unwrap();
        std::fs::write(d.path().join("gitfile"), format!("gitdir: {}\n", other.join(".git").display())).unwrap();
        symlink(d.path().join("gitfile"), linked.join(".git")).unwrap();
        assert_eq!(in_tmp(&linked), None, ".git symlink to a file");
        // A worktree whose gitdir goes through a planted folder link
        let wt = d.path().join("wt");
        std::fs::create_dir_all(other.join(".git/worktrees/wt")).unwrap();
        std::fs::write(other.join(".git/worktrees/wt/HEAD"), "ref: refs/heads/LEAKED-FROM-OUTSIDE\n").unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        symlink(other.parent().unwrap(), wt.join("hop")).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: hop/repo/.git/worktrees/wt\n").unwrap();
        assert_eq!(in_tmp(&wt), None, "gitdir through a symlinked folder");
    }

    /// F-3, M1: `link/..` in the gitdir is the link target's parent to the kernel and git, not
    /// the folder the text names. A decoy on the text path says `feature`, the real gitdir `main`.
    #[cfg(unix)]
    #[test]
    fn head_branch_refuses_dotdot_after_a_symlink_in_the_gitdir() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        let wt = p.join("wt");
        std::fs::create_dir_all(p.join("phys/sub")).unwrap();
        std::fs::create_dir_all(p.join("phys/decoy/.git/worktrees/x")).unwrap();
        std::fs::write(p.join("phys/decoy/.git/worktrees/x/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::create_dir_all(wt.join("decoy/.git/worktrees/x")).unwrap();
        std::fs::write(wt.join("decoy/.git/worktrees/x/HEAD"), "ref: refs/heads/feature\n").unwrap();
        symlink(p.join("phys/sub"), wt.join("link")).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: link/../decoy/.git/worktrees/x\n").unwrap();
        // What git and the kernel open: the physical copy, on `main`
        let kernel = std::fs::read_to_string(wt.join("link/../decoy/.git/worktrees/x/HEAD")).unwrap();
        assert_eq!(kernel, "ref: refs/heads/main\n");
        assert_eq!(in_tmp(&wt), None, "M1: must not read the text path's `feature`");
        // `..` after a real folder still reads
        std::fs::create_dir_all(wt.join("real")).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: real/../decoy/.git/worktrees/x\n").unwrap();
        assert_eq!(in_tmp(&wt).as_deref(), Some("feature"));
    }

    /// F-3, M2: the start folder sits under a symlink, and the gitdir's leading `..` climb over it.
    /// The text lands beside the link; the kernel lands beside the link's target.
    #[cfg(unix)]
    #[test]
    fn head_branch_refuses_dotdot_over_a_symlink_in_the_start_folder() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        let real = p.join("deep/er/real");
        std::fs::create_dir_all(real.join("wt")).unwrap();
        symlink(&real, p.join("lnk")).unwrap();
        std::fs::create_dir_all(p.join("deep/er/decoy2/.git/worktrees/x")).unwrap();
        std::fs::write(p.join("deep/er/decoy2/.git/worktrees/x/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::create_dir_all(p.join("decoy2/.git/worktrees/x")).unwrap();
        std::fs::write(p.join("decoy2/.git/worktrees/x/HEAD"), "ref: refs/heads/feature\n").unwrap();
        std::fs::write(real.join("wt/.git"), "gitdir: ../../decoy2/.git/worktrees/x\n").unwrap();
        let start = p.join("lnk/wt");
        let kernel = std::fs::read_to_string(start.join("../../decoy2/.git/worktrees/x/HEAD")).unwrap();
        assert_eq!(kernel, "ref: refs/heads/main\n");
        assert_eq!(in_tmp(&start), None, "M2: must not read the text path's `feature`");
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
            assert_eq!(in_tmp(&wt), None, "{bad}");
        }
        // A relative gitdir (what `worktree.useRelativePaths` writes) reads
        std::fs::write(wt.join(".git"), "gitdir: ../repo/.git/worktrees/x\n").unwrap();
        assert_eq!(in_tmp(&wt).as_deref(), Some("x"));
        // The branch must be a plain ref
        for head in ["ref: refs/heads/../../x", "ref: refs/heads/a b", "ref: refs/tags/v1", "ref: refs/heads/"] {
            std::fs::write(root.join(".git/HEAD"), head).unwrap();
            assert_eq!(in_tmp(&root), None, "{head}");
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
        assert_eq!(in_tmp(&main).as_deref(), Some("main"));
        assert_eq!(in_tmp(&wt).as_deref(), Some("juno/side-1"));
        std::fs::create_dir_all(wt.join("src")).unwrap();
        assert_eq!(in_tmp(&wt.join("src")).as_deref(), Some("juno/side-1"));
    }

    /// P1: a `.git` folder without `objects/` and `refs/` isn't a repo to git, which uses the repo
    /// above it. A planted `sub/.git/HEAD` must not hide the real branch (old code: "decoy").
    #[test]
    fn head_branch_needs_objects_and_refs_like_git() {
        let (d, root) = head_repo();
        std::fs::create_dir_all(root.join("sub/.git")).unwrap();
        std::fs::write(root.join("sub/.git/HEAD"), "ref: refs/heads/decoy\n").unwrap();
        assert_eq!(head_branch(d.path(), &root.join("sub")).as_deref(), Some("lin-dev-core"), "HEAD only");
        std::fs::create_dir_all(root.join("sub/.git/objects")).unwrap();
        assert_eq!(head_branch(d.path(), &root.join("sub")).as_deref(), Some("lin-dev-core"), "no refs/");
        std::fs::create_dir_all(root.join("sub/.git/refs")).unwrap();
        assert_eq!(head_branch(d.path(), &root.join("sub")).as_deref(), Some("decoy"), "a real nested repo");
        // A symlinked `refs` git would follow: no answer rather than the wrong repo's
        #[cfg(unix)]
        {
            std::fs::remove_dir(root.join("sub/.git/refs")).unwrap();
            std::os::unix::fs::symlink(root.join(".git/refs"), root.join("sub/.git/refs")).unwrap();
            assert_eq!(head_branch(d.path(), &root.join("sub")), None);
        }
    }

    /// A4: a worktree's gitdir may not leave the bot folder, absolute (old code read the other
    /// repo's branch) or relative.
    #[test]
    fn head_branch_keeps_a_worktree_gitdir_in_the_bot_folder() {
        let (_o, other) = head_repo();
        std::fs::create_dir_all(other.join(".git/worktrees/x")).unwrap();
        std::fs::write(other.join(".git/worktrees/x/HEAD"), "ref: refs/heads/theirs\n").unwrap();
        let d = tempfile::tempdir().unwrap();
        let bot = d.path().join("bot");
        std::fs::create_dir_all(&bot).unwrap();
        std::fs::write(bot.join(".git"), format!("gitdir: {}\n", other.join(".git/worktrees/x").display())).unwrap();
        assert_eq!(head_branch(&bot, &bot), None, "absolute gitdir into another repo");
        let rel = format!("gitdir: ../../{}/.git/worktrees/x\n", other.strip_prefix(d.path().parent().unwrap()).unwrap().display());
        std::fs::write(bot.join(".git"), &rel).unwrap();
        assert_eq!(head_branch(&bot, &bot), None, "relative gitdir climbing out: {rel}");
        // The same gitdir reads once the bot folder holds both
        assert_eq!(head_branch(d.path().parent().unwrap(), &bot).as_deref(), Some("theirs"));
        // An absolute gitdir inside the bot folder reads
        std::fs::create_dir_all(bot.join("repo/.git/worktrees/w")).unwrap();
        std::fs::write(bot.join("repo/.git/worktrees/w/HEAD"), "ref: refs/heads/mine\n").unwrap();
        std::fs::create_dir_all(bot.join("wt")).unwrap();
        std::fs::write(bot.join("wt/.git"), format!("gitdir: {}\n", bot.join("repo/.git/worktrees/w").display())).unwrap();
        assert_eq!(head_branch(&bot, &bot.join("wt")).as_deref(), Some("mine"));
    }

    /// Q3: a working folder outside the bot folder is refused before any `.git` is read (old
    /// code read the other repo's branch), and so is one that climbs out with `..`.
    #[test]
    fn head_branch_refuses_a_working_folder_outside_the_bot_folder() {
        let (_o, other) = head_repo();
        let (_b, bot) = head_repo();
        assert_eq!(head_branch(&bot, &other), None, "another repo");
        assert_eq!(head_branch(&bot, &bot.join("..")), None, "`..` out of the folder");
        assert_eq!(head_branch(&bot, &bot.join("sub/../..")), None);
        assert_eq!(head_branch(&bot, Path::new("repo")), None, "a relative cwd");
        // And no `.git` above the bot folder: a bot folder inside a repo gets no answer
        std::fs::create_dir_all(bot.join("sub")).unwrap();
        assert_eq!(head_branch(&bot.join("sub"), &bot.join("sub")), None);
        // Inside, it reads
        assert_eq!(head_branch(&bot, &bot).as_deref(), Some("lin-dev-core"));
        assert_eq!(head_branch(&bot, &bot.join("sub")).as_deref(), Some("lin-dev-core"));
        assert_eq!(head_branch(&bot, &bot.join("./sub")).as_deref(), Some("lin-dev-core"));
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
