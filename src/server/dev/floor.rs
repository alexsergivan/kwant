//! The production floor in the approval path (plan §3.5): builds a space's [`DevCtx`], runs the
//! pure classifier in `crate::mcp::prod_floor`, and gives the card its `prod_floor` payload. The
//! approvers treat a hit as `forced` (it asks whatever the approvals say) with no "always allow";
//! [`allow_refusal`] is the server-side check that refuses **Allow** while a step needs a backup
//! and none is fresh.

use crate::db::Database;
use crate::mcp::prod_floor::{self, DevCtx, Floor};
use crate::mcp::ToolApprovalRequest;
use serde_json::{json, Value};
use std::path::Path;

/// The Dev mode context of a bot's space, or `None` when Dev mode is off there (no floor).
pub async fn ctx_for_kwant(db: &Database, kwant_id: &str, cwd: Option<&Path>) -> Option<(String, DevCtx)> {
    let kwant = db.get_kwant(kwant_id).await.ok().flatten()?;
    let mode = db.dev_mode(&kwant.space_id).await.ok().flatten().filter(|m| m.on)?;
    let marked = db.prod_connections(&kwant.space_id).await.unwrap_or_default();
    let servers = db.list_mcp_servers(&crate::db::spaces::SpaceId::new(&kwant.space_id)).await.unwrap_or_default();
    let prod_connections = prod_names(&marked, servers.iter().map(|s| (s.id.as_str(), s.name.as_str())));
    let ctx = DevCtx {
        branch: mode.branch.clone(),
        protected: Vec::new(),
        deploy_hint: mode.deploy_hint.clone(),
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
pub fn head_branch(cwd: &Path) -> Option<String> {
    let mut dir = Some(cwd);
    for _ in 0..12 {
        let d = dir?;
        let git = d.join(".git");
        let head_file = if git.is_dir() {
            git.join("HEAD")
        } else if git.is_file() {
            // A worktree: `.git` says `gitdir: <path>`
            let text = std::fs::read_to_string(&git).ok()?;
            let gd = text.trim().strip_prefix("gitdir:")?.trim().to_string();
            let gd = if Path::new(&gd).is_absolute() { Path::new(&gd).to_path_buf() } else { d.join(gd) };
            gd.join("HEAD")
        } else {
            dir = d.parent();
            continue;
        };
        let head = std::fs::read_to_string(head_file).ok()?;
        return head.trim().strip_prefix("ref: refs/heads/").map(str::to_string);
    }
    None
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
    if super::backup::is_fresh(ok.as_ref(), chrono::Utc::now()) {
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
}
