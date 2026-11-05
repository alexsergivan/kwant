//! The production floor (0.7 Dev mode, plan §3.5, D9, D10): in a space with Dev mode on, tool
//! calls that push to the watched branch, force-push or delete a protected ref, deploy, destroy
//! data, or go through a connection marked production always get an approval card, whatever the
//! bot's approvals are and whatever "always allow" was given. Like the payment floor it sits
//! next to, [`reason`] and [`shell_reason`] are pure: they read the call and a [`DevCtx`] the
//! caller builds from the space's Dev mode row, never anything the bot says about the call.
//!
//! The lists err on the side of asking: an extra card costs a tap, a missed deploy can cost a
//! site. What the floor can't see is written down in docs/dev-mode.md.

use super::ToolApprovalRequest;
use serde::Serialize;
use serde_json::Value;

/// What kind of production step a call is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FloorKind {
    PushMain,
    ForcePush,
    DeleteRef,
    Deploy,
    DeployMigration,
    Destructive,
    ProdKey,
}

impl FloorKind {
    /// Destructive steps need an `ok` backup from the last 10 minutes before Allow (D9).
    pub fn needs_backup(self) -> bool {
        matches!(self, FloorKind::Destructive | FloorKind::DeployMigration)
    }

    /// How sure the floor is that this matters most: the stronger kind wins when one command
    /// does several things.
    fn rank(self) -> u8 {
        match self {
            FloorKind::Destructive => 7,
            FloorKind::DeployMigration => 6,
            FloorKind::ForcePush => 5,
            FloorKind::DeleteRef => 4,
            FloorKind::Deploy => 3,
            FloorKind::PushMain => 2,
            FloorKind::ProdKey => 1,
        }
    }
}

/// Why a call hits the floor, with the blast-radius rows the card shows (only what's known).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Floor {
    pub kind: FloorKind,
    pub why: String,
    /// `(label, value)`, labels from: Where, What changes, Downtime, Data, Key used, Migration,
    /// If it fails. Serialised as `[label, value]` pairs.
    pub blast: Vec<(String, String)>,
    pub needs_backup: bool,
}

impl Floor {
    fn new(kind: FloorKind, why: impl Into<String>) -> Self {
        Floor { kind, why: why.into(), blast: Vec::new(), needs_backup: kind.needs_backup() }
    }

    fn row(mut self, label: &str, value: impl Into<String>) -> Self {
        let value = value.into();
        if !value.is_empty() && !self.blast.iter().any(|(l, _)| l == label) {
            self.blast.push((label.to_string(), value));
        }
        self
    }

    /// The stronger of two findings; rows of both are kept.
    fn merge(self, other: Floor) -> Floor {
        let (mut keep, extra) = if other.kind.rank() > self.kind.rank() { (other, self) } else { (self, other) };
        for (l, v) in extra.blast {
            if !keep.blast.iter().any(|(k, _)| *k == l) {
                keep.blast.push((l, v));
            }
        }
        keep.needs_backup = keep.kind.needs_backup();
        keep
    }
}

/// What the floor knows about the space, built by the caller from `dev_mode_spaces`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DevCtx {
    /// The watched branch (`main`).
    pub branch: String,
    /// More branches to protect besides [`PROTECTED`] and the watched one.
    pub protected: Vec<String>,
    /// `docker`, `cloudflare_pages`, `fly` or `None`.
    pub deploy_hint: Option<String>,
    /// MCP server names and Composio toolkit slugs marked production (D10).
    pub prod_connections: Vec<String>,
    /// The branch checked out in the bot's folder, when the caller could read it; a bare
    /// `git push` pushes that branch. `None` counts as protected (it can't be told).
    pub head_branch: Option<String>,
    /// Whether the code a deploy ships changes the database. `None`: kwant can't tell, so a
    /// deploy counts as having a migration (D9), except where nothing holds data.
    pub migration: Option<bool>,
}

impl DevCtx {
    fn is_protected(&self, branch: &str) -> bool {
        let b = branch.strip_prefix("refs/heads/").unwrap_or(branch);
        b == self.branch || PROTECTED.contains(&b) || self.protected.iter().any(|p| p == b) || b.starts_with("release/")
    }

    /// The target keeps no data (a static Pages site): nothing to back up.
    pub fn no_data(&self) -> bool {
        self.deploy_hint.as_deref() == Some("cloudflare_pages")
    }

    /// A deploy, upgraded to a migration deploy unless the site holds no data or the caller
    /// knows there's no database change.
    fn deploy(&self, why: String) -> Floor {
        let migration = !self.no_data() && self.migration != Some(false);
        if migration {
            let known = self.migration == Some(true);
            Floor::new(FloorKind::DeployMigration, why).row(
                "Migration",
                if known { "This deploy changes the database." } else { "kwant can't tell whether this deploy changes the database, so it counts as one." },
            )
            .row("If it fails", "The app goes back on its own. The database goes back only from a backup.")
        } else {
            let f = Floor::new(FloorKind::Deploy, why);
            if self.no_data() {
                f.row("Data", "Nothing to back up: this site has no database or uploads.")
            } else {
                f
            }
        }
    }
}

/// Branches always protected, besides the watched one.
pub const PROTECTED: &[&str] = &["main", "master", "production", "prod", "live", "stable", "trunk", "gh-pages"];

/// Git remotes whose push is itself a deploy.
const DEPLOY_REMOTES: &[&str] = &["heroku", "dokku", "production", "prod", "live", "deploy", "fly"];

/// Words in a script, make target, npm script or workflow name that mean shipping.
const SHIP_WORDS: &[&str] = &["deploy", "redeploy", "ship", "rollout", "release", "promote"];

/// Infrastructure apps (MCP server names, Composio toolkits) whose tools change real systems.
const INFRA_APPS: &[&str] = &[
    "cloudflare", "digitalocean", "aws", "gcp", "gcloud", "google_cloud", "azure", "vercel", "netlify", "fly", "flyio", "render",
    "railway", "heroku", "supabase", "neon", "planetscale", "turso", "kubernetes", "k8s", "docker", "hetzner", "linode", "vultr",
    "firebase", "mongodb", "atlas", "redis", "upstash", "postgres", "mysql", "sqlite", "database",
];

/// Tool words that destroy something on an infrastructure app.
const DESTROY_WORDS: &[&str] = &[
    "delete", "destroy", "drop", "purge", "wipe", "truncate", "rebuild", "reset", "restore", "remove", "rm", "terminate", "erase",
    "flush", "shutdown", "poweroff", "uninstall",
];

/// Tool words that ship.
const DEPLOY_TOOL_WORDS: &[&str] = &["deploy", "redeploy", "rollout", "promote", "publish", "release"];

/// Path parts that hold data a person would miss.
const DATA_WORDS: &[&str] = &[
    "data", "db", "database", "databases", "uploads", "upload", "media", "volumes", "volume", "backups", "backup", "storage",
    "prod", "production", "postgres", "postgresql", "mysql", "redis", "var",
];

/// Why this call hits the production floor, or `None`. Only called when the bot's space has
/// Dev mode on.
pub fn reason(req: &ToolApprovalRequest, ctx: &DevCtx) -> Option<Floor> {
    let shell = |cmd: Option<&str>| cmd.and_then(|c| shell_reason(c, ctx));
    let found = if req.builtin.is_some() {
        match req.tool.as_str() {
            "Bash" => shell(req.arguments["command"].as_str()),
            _ => None,
        }
    } else if req.tool == "execute_shell" {
        shell(req.arguments["command"].as_str())
    } else if req.server == super::SELF_SERVER_NAME {
        None
    } else {
        mcp_reason(&req.server, &req.tool, &req.arguments, ctx)
    };
    // A production connection is checked last: what the call does names the card first
    match (found, prod_key(&req.server, &req.tool, &req.arguments, ctx)) {
        (Some(f), Some(k)) => Some(f.merge(k)),
        (f, k) => f.or(k),
    }
}

/// The connection a call goes through, when it's marked production.
fn prod_key(server: &str, tool: &str, args: &Value, ctx: &DevCtx) -> Option<Floor> {
    if ctx.prod_connections.is_empty() || server.is_empty() || server == super::SELF_SERVER_NAME {
        return None;
    }
    let s = server.to_lowercase();
    let mut names = vec![s.clone()];
    // Composio: the toolkit is the first word of the tool slug (`GITHUB_CREATE_ISSUE`)
    let mut slugs = Vec::new();
    collect_slugs(args, &mut slugs, 0);
    slugs.push(tool.to_string());
    for slug in slugs {
        if let Some(first) = slug.to_lowercase().split(['_', '-']).find(|w| !w.is_empty() && *w != "composio") {
            names.push(first.to_string());
        }
    }
    let hit = ctx.prod_connections.iter().find(|p| names.iter().any(|n| n == &p.to_lowercase()))?;
    Some(Floor::new(FloorKind::ProdKey, format!("{tool} goes through {hit}, a connection marked production")).row("Key used", format!("{hit} · production")))
}

/// Composio-style slugs in the arguments (`tool_slug`, `tools: [{tool_slug}]`).
fn collect_slugs(v: &Value, out: &mut Vec<String>, depth: usize) {
    if depth > 4 || out.len() > 20 {
        return;
    }
    match v {
        Value::Object(m) => {
            for (k, val) in m {
                let k = k.to_lowercase();
                if matches!(k.as_str(), "tool_slug" | "slug" | "action" | "tool") {
                    if let Some(s) = val.as_str() {
                        out.push(s.to_string());
                    }
                }
                collect_slugs(val, out, depth + 1);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_slugs(x, out, depth + 1)),
        _ => {}
    }
}

/// An MCP or Composio tool by its name and arguments.
fn mcp_reason(server: &str, tool: &str, args: &Value, ctx: &DevCtx) -> Option<Floor> {
    let mut slugs = vec![tool.to_string()];
    collect_slugs(args, &mut slugs, 0);
    let app_words = words(server);
    let mut best: Option<Floor> = None;
    let mut add = |f: Floor| best = Some(match best.take() { Some(b) => b.merge(f), None => f });
    for slug in &slugs {
        let w = words(slug);
        let infra = app_words.iter().chain(w.iter()).any(|x| INFRA_APPS.contains(&x.as_str()));
        if w.first().is_some_and(|f| READ_VERBS.contains(&f.as_str())) {
            continue;
        }
        if w.iter().any(|x| DEPLOY_TOOL_WORDS.contains(&x.as_str())) && (infra || w.iter().any(|x| x == "deploy" || x == "redeploy" || x == "rollout")) {
            add(ctx.deploy(format!("{slug} deploys")).row("Where", server.to_string()));
        }
        if infra && w.iter().any(|x| DESTROY_WORDS.contains(&x.as_str())) {
            add(Floor::new(FloorKind::Destructive, format!("{slug} deletes or rebuilds something on {server}")).row("Data", "Can't be undone without a backup."));
        }
    }
    // SQL a database tool runs
    let mut texts = Vec::new();
    collect_strings(args, &mut texts, 0);
    if let Some(why) = texts.iter().find_map(|t| destructive_sql(t)) {
        add(Floor::new(FloorKind::Destructive, format!("{tool} runs {why}")).row("Data", "Can't be undone without a backup."));
    }
    // A shell an MCP server runs (a remote bash tool) is checked like Bash
    let tw = words(tool);
    if tw.iter().any(|x| matches!(x.as_str(), "bash" | "shell" | "exec" | "execute" | "command" | "terminal" | "run" | "ssh")) {
        for key in ["command", "cmd", "script"] {
            if let Some(f) = args[key].as_str().and_then(|c| shell_reason(c, ctx)) {
                add(f);
            }
        }
    }
    best
}

fn collect_strings(v: &Value, out: &mut Vec<String>, depth: usize) {
    if depth > 4 || out.len() > 50 {
        return;
    }
    match v {
        Value::String(s) => out.push(s.chars().take(20_000).collect()),
        Value::Object(m) => m.values().for_each(|x| collect_strings(x, out, depth + 1)),
        Value::Array(a) => a.iter().for_each(|x| collect_strings(x, out, depth + 1)),
        _ => {}
    }
}

/// A tool whose first action word is one of these only reads.
const READ_VERBS: &[&str] = &[
    "get", "list", "retrieve", "search", "find", "read", "fetch", "describe", "view", "show", "count", "lookup", "query", "inspect",
    "status", "info", "logs", "log", "whoami", "docs",
];

/// `snake_case`, `kebab-case` and `camelCase` words, lower-cased.
fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// SQL that drops, empties or rewrites data: `DROP TABLE|DATABASE|SCHEMA`, `TRUNCATE`,
/// `DELETE FROM` or `UPDATE … SET` with no `WHERE`, `ALTER TABLE … DROP`, and the Redis/Mongo
/// equivalents.
pub fn destructive_sql(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let flat: String = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    for (pat, what) in [
        ("drop table", "DROP TABLE"),
        ("drop database", "DROP DATABASE"),
        ("drop schema", "DROP SCHEMA"),
        ("drop collection", "DROP COLLECTION"),
        ("truncate table", "TRUNCATE"),
        ("flushall", "FLUSHALL"),
        ("flushdb", "FLUSHDB"),
        ("dropdatabase(", "dropDatabase()"),
        (".drop(", "a collection drop"),
        ("deletemany({})", "deleteMany({})"),
    ] {
        if flat.contains(pat) {
            return Some(what.to_string());
        }
    }
    if flat.contains("alter table") && flat.contains(" drop ") {
        return Some("ALTER TABLE … DROP".into());
    }
    if starts_sql(&flat, "truncate ") {
        return Some("TRUNCATE".into());
    }
    // DELETE FROM x and UPDATE x SET with no WHERE, statement by statement
    for stmt in flat.split(';') {
        if let Some(i) = stmt.find("delete from ") {
            if !stmt[i..].contains(" where ") {
                return Some("DELETE FROM with no WHERE".into());
            }
        }
        if update_without_where(stmt) {
            return Some("UPDATE with no WHERE".into());
        }
    }
    None
}

/// `UPDATE t SET …` with no `WHERE`. `UPDATE` must start the statement or a quoted argument
/// (`psql -c 'UPDATE t SET x=1'`) and `SET` follow the table, so prose such as "update the docs
/// and set a date" in a message doesn't count.
fn update_without_where(stmt: &str) -> bool {
    stmt.match_indices("update ").any(|(i, _)| {
        let before = stmt[..i].trim_end();
        let starts = before.is_empty() || matches!(before.as_bytes().last(), Some(b'\'' | b'"' | b'(' | b'`'));
        let mut rest = stmt[i + 7..].split_whitespace();
        let table = match rest.next() {
            Some("only") => rest.next(),
            t => t,
        };
        starts && table.is_some() && rest.next() == Some("set") && !stmt[i..].contains(" where ")
    })
}

fn starts_sql(flat: &str, kw: &str) -> bool {
    // A statement starts the text, follows a `;`, or opens a quoted argument (`-c 'TRUNCATE x'`)
    flat.split([';', '\'', '"']).any(|s| s.trim_start().starts_with(kw))
}

/// Why a shell command hits the floor, or `None`.
pub fn shell_reason(command: &str, ctx: &DevCtx) -> Option<Floor> {
    shell_reason_at(command, ctx, 0, false)
}

/// `over_ssh`: the command runs on another machine (`ssh host '…'`), where a compose `up` is a
/// deploy whatever the space's hint says.
fn shell_reason_at(command: &str, ctx: &DevCtx, depth: usize, over_ssh: bool) -> Option<Floor> {
    if depth > 3 {
        return None;
    }
    let mut best: Option<Floor> = None;
    let mut add = |f: Floor| best = Some(match best.take() { Some(b) => b.merge(f), None => f });
    let lower = command.to_lowercase();
    // SQL through a database client anywhere in the command
    if DB_CLIENTS.iter().any(|c| lower.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')).any(|w| w == *c)) {
        if let Some(why) = destructive_sql(command) {
            add(Floor::new(FloorKind::Destructive, format!("a database command that runs {why}")).row("Data", "Can't be undone without a backup."));
        }
    }
    let mut ssh_host: Option<String> = None;
    // A `git checkout`/`git switch` earlier in the chain changes what a bare `git push` pushes
    let mut cur = ctx.clone();
    // `ssh host 'cd /srv && docker compose up -d'`: the quoted remote command spans segments;
    // the quote still open after `ssh`, until it closes
    let mut ssh_quote: Option<char> = None;
    for segment in command.split([';', '&', '|', '\n', '(', ')', '`']) {
        let remote = ssh_quote.is_some();
        if let Some(q) = ssh_quote {
            if segment.matches(q).count() % 2 == 1 {
                ssh_quote = None;
            }
        }
        let over_ssh = over_ssh || remote;
        let words: Vec<String> = segment
            .split_whitespace()
            .map(|w| w.trim_matches(|c| c == '\'' || c == '"').to_string())
            .filter(|w| !w.is_empty())
            .collect();
        // Env assignments, and wrappers that run the rest of the line (with their own flags)
        let mut i = 0;
        let mut env_prod = false;
        while i < words.len() {
            let w = words[i].as_str();
            if w.contains('=') && !w.starts_with('-') {
                env_prod |= prod_env(w);
                i += 1;
            } else if let Some(n) = wrapper_len(w.trim_start_matches('\\'), &words[i + 1..]) {
                i = (i + n).min(words.len());
            } else {
                break;
            }
        }
        let Some((program, args)) = words[i..].split_first() else { continue };
        // `\git` skips aliases and still runs git
        let program = program.trim_start_matches('\\');
        let program = program.rsplit('/').next().unwrap_or(program).to_lowercase();
        let args: Vec<String> = args.to_vec();
        let a: Vec<&str> = args.iter().map(String::as_str).collect();
        // Wrappers that run another command: classify what they run
        match program.as_str() {
            "ssh" => {
                if let Some((host, rest)) = ssh_target(&a) {
                    ssh_host = Some(host.clone());
                    ssh_quote = ['\'', '"'].into_iter().find(|q| segment.matches(*q).count() % 2 == 1);
                    if !rest.is_empty() {
                        if let Some(f) = shell_reason_at(&rest.join(" "), ctx, depth + 1, true) {
                            add(f.row("Where", host));
                        }
                    }
                }
                continue;
            }
            "bash" | "sh" | "zsh" | "dash" => {
                if let Some(p) = a.iter().position(|x| *x == "-c" || *x == "-lc") {
                    if let Some(f) = shell_reason_at(&a[p + 1..].join(" "), &cur, depth + 1, over_ssh) {
                        add(f);
                    }
                } else if let Some(script) = first_operand(&a, &["-o", "+o", "-O", "+O"]) {
                    if let Some(f) = script_reason(script, &cur) {
                        add(f);
                    }
                }
                continue;
            }
            "git" => {
                if let Some(head) = git_switches_to(&a, &cur) {
                    cur.head_branch = head;
                    continue;
                }
            }
            _ => {}
        }
        if let Some(f) = program_reason(&program, &a, &cur, env_prod, over_ssh) {
            add(f);
        }
    }
    best.map(|f| match ssh_host {
        Some(h) => f.row("Where", h),
        None => f,
    })
}

/// How many words a wrapper that runs the rest of the line takes (itself, its flags and their
/// values, and `timeout`'s duration), or `None` when `w` isn't one: `timeout 60 git push`,
/// `nice -n 10 …`, `sudo -u deploy …`, `watch -n1 '…'`, `eval '…'`.
fn wrapper_len(w: &str, rest: &[String]) -> Option<usize> {
    // Flags that take the next word as their value
    let takes: &[&str] = match w {
        "sudo" => &["-u", "-g", "-h", "-p", "-C", "-U", "-r", "-t", "-D", "--user", "--group", "--host", "--prompt", "--chdir"],
        "doas" => &["-u", "-C"],
        "env" => &["-u", "-C", "-S", "--unset", "--chdir", "--split-string"],
        "timeout" => &["-s", "-k", "--signal", "--kill-after"],
        "nice" => &["-n", "--adjustment"],
        "ionice" => &["-c", "-n", "-t", "--class", "--classdata"],
        "stdbuf" => &["-i", "-o", "-e", "--input", "--output", "--error"],
        "watch" => &["-n", "-q", "--interval", "--equexit"],
        "xargs" => &["-I", "-i", "-n", "-P", "-L", "-l", "-d", "-E", "-e", "-a", "-s", "--max-args", "--max-procs", "--max-lines", "--delimiter", "--arg-file", "--max-chars", "--replace"],
        "npx" | "pnpx" | "bunx" => &["-p", "--package", "-c", "--call"],
        "exec" => &["-a"],
        "time" => &["-f", "-o", "--format", "--output"],
        "setsid" | "nohup" | "command" | "eval" => &[],
        _ => return None,
    };
    let mut n = 1;
    while let Some(x) = rest.get(n - 1) {
        if !x.starts_with('-') || x == "-" {
            break;
        }
        n += if takes.contains(&x.as_str()) { 2 } else { 1 };
        if x == "--" {
            break;
        }
    }
    // `timeout [flags] DURATION command`
    if w == "timeout" && rest.get(n - 1).is_some_and(|d| d.starts_with(|c: char| c.is_ascii_digit() || c == '.')) {
        n += 1;
    }
    Some(n)
}

/// The first word that isn't a flag or a flag's value.
fn first_operand<'a>(a: &[&'a str], takes: &[&str]) -> Option<&'a str> {
    let mut i = 0;
    while let Some(x) = a.get(i) {
        if takes.contains(x) {
            i += 2;
        } else if x.starts_with('-') || x.starts_with('+') {
            i += 1;
        } else {
            return Some(x);
        }
    }
    None
}

/// The branch `git checkout`/`git switch` leaves checked out: `Some(Some(b))` when that's
/// certain, `Some(None)` when it can't be told (a bare `git push` then counts as protected), and
/// `None` when the command doesn't change branches. `git checkout x` may restore a file `x`, so it
/// only counts as a switch to `x` when `x` is protected; otherwise the branch becomes unknown.
fn git_switches_to(a: &[&str], ctx: &DevCtx) -> Option<Option<String>> {
    let mut i = 0;
    while i < a.len() && a[i].starts_with('-') {
        i += if matches!(a[i], "-C" | "-c" | "--git-dir" | "--work-tree") { 2 } else { 1 };
    }
    let sub = *a.get(i)?;
    if sub != "checkout" && sub != "switch" {
        return None;
    }
    let args = &a[i + 1..];
    let create: &[&str] = if sub == "checkout" { &["-b", "-B", "--orphan"] } else { &["-c", "-C", "--create", "--force-create", "--orphan"] };
    let mut pos: Vec<&str> = Vec::new();
    let mut j = 0;
    while let Some(x) = args.get(j) {
        if create.contains(x) {
            return Some(args.get(j + 1).map(|b| b.to_string()));
        }
        match *x {
            "--" => return Some(if sub == "checkout" { ctx.head_branch.clone() } else { None }),
            "--detach" | "-d" if sub == "switch" || *x == "--detach" => return Some(None),
            "-" => return Some(None),
            _ if x.starts_with('-') => {}
            _ => pos.push(x),
        }
        j += 1;
    }
    match (sub, pos.as_slice()) {
        // `git checkout`, `git switch` alone change nothing
        (_, []) => None,
        ("switch", [b, ..]) => Some(Some(b.to_string())),
        // `git checkout .` restores files
        ("checkout", ["."]) => None,
        ("checkout", [b]) if ctx.is_protected(b) => Some(Some(b.to_string())),
        // `git checkout x` (a branch or a file) or `git checkout ref -- paths`
        _ => Some(None),
    }
}

/// Database clients whose arguments may hold SQL.
const DB_CLIENTS: &[&str] = &["sqlite3", "sqlite", "psql", "mysql", "mariadb", "mongosh", "mongo", "redis-cli", "d1", "turso", "pgcli", "duckdb"];

/// `RAILS_ENV=production`, `NODE_ENV=production`, `DATABASE_URL=postgres://db.example.com/…`.
fn prod_env(assign: &str) -> bool {
    let Some((k, v)) = assign.split_once('=') else { return false };
    let v = v.trim_matches(|c| c == '\'' || c == '"').to_lowercase();
    let k = k.to_uppercase();
    if k.ends_with("_ENV") || k == "ENV" || k == "ENVIRONMENT" || k == "STAGE" {
        return matches!(v.as_str(), "production" | "prod" | "live");
    }
    if k.ends_with("DATABASE_URL") || k.ends_with("DB_URL") {
        return remote_url(&v);
    }
    false
}

/// A database URL that isn't on this machine.
fn remote_url(v: &str) -> bool {
    match url::Url::parse(v) {
        // A file database is on this machine
        Ok(u) if matches!(u.scheme(), "sqlite" | "file" | "sqlite3" | "duckdb") => false,
        Ok(u) => match u.host_str() {
            Some(h) => !matches!(h, "localhost" | "127.0.0.1" | "::1" | "[::1]" | "0.0.0.0") && !h.ends_with(".local"),
            None => false,
        },
        Err(_) => false,
    }
}

/// `ssh [options] host [command…]` → host and command words.
fn ssh_target<'a>(args: &[&'a str]) -> Option<(String, Vec<&'a str>)> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        if a.starts_with('-') {
            // Options that take a value
            let takes = matches!(a, "-p" | "-i" | "-l" | "-o" | "-F" | "-J" | "-L" | "-R" | "-D" | "-b" | "-c" | "-E" | "-e" | "-m" | "-O" | "-S" | "-W" | "-w" | "-B" | "-Q");
            i += if takes { 2 } else { 1 };
            continue;
        }
        let host = a.rsplit('@').next().unwrap_or(a).to_string();
        return Some((host, args[i + 1..].to_vec()));
    }
    None
}

/// A script file by its name: `./deploy.sh`, `scripts/release-prod.sh`.
fn script_reason(script: &str, ctx: &DevCtx) -> Option<Floor> {
    let name = script.rsplit('/').next().unwrap_or(script).to_lowercase();
    let ws = words(&name);
    ws.iter().any(|w| SHIP_WORDS.contains(&w.as_str())).then(|| ctx.deploy(format!("runs {script}, a deploy script")))
}

fn has(args: &[&str], x: &str) -> bool {
    args.contains(&x)
}

/// Positional arguments (not starting with `-`).
fn positional<'a>(args: &[&'a str]) -> Vec<&'a str> {
    args.iter().copied().filter(|a| !a.starts_with('-')).collect()
}

fn program_reason(program: &str, a: &[&str], ctx: &DevCtx, env_prod: bool, over_ssh: bool) -> Option<Floor> {
    let pos = positional(a);
    let sub = pos.first().copied().unwrap_or("");
    let sub2 = pos.get(1).copied().unwrap_or("");
    let destroy = |why: String| Floor::new(FloorKind::Destructive, why).row("Data", "Can't be undone without a backup.");
    // A script run by path
    if program.ends_with(".sh") || program.ends_with(".py") || program.ends_with(".rb") || program.ends_with(".js") {
        return script_reason(program, ctx);
    }
    // A script run by an interpreter is read by its name too: `python release.py`, `node deploy.js`
    if let Some(f) = interpreter_script(program, a).and_then(|s| script_reason(s, ctx)) {
        return Some(f);
    }
    match program {
        "git" => git_reason(a, ctx),
        "gh" => match (sub, sub2) {
            ("pr", "merge") => Some(
                Floor::new(FloorKind::PushMain, "gh pr merge lands a PR on its base branch").row("What changes", "The PR's commits land on the base branch."),
            ),
            ("workflow", "run") => {
                let name = pos.get(2).copied().unwrap_or("");
                ship_name(name).then(|| ctx.deploy(format!("starts the {name} workflow")))
            }
            ("release", "create") => Some(ctx.deploy("gh release create makes a release, which can start a deploy".into())),
            ("release", "delete") | ("repo", "delete") => Some(destroy(format!("gh {sub} delete removes it from GitHub"))),
            _ => None,
        },
        "fly" | "flyctl" => {
            let app = flag_value(a, &["-a", "--app"]);
            let f = match (sub, sub2) {
                ("deploy", _) | ("releases", "rollback") => Some(ctx.deploy(format!("{program} deploy ships the app"))),
                ("apps", "destroy") | ("volumes", "destroy") | ("volumes", "delete") | ("destroy", _) | ("postgres", "destroy") => {
                    Some(destroy(format!("{program} {sub} {sub2} destroys it")))
                }
                ("machine", "destroy") | ("machines", "destroy") => Some(destroy(format!("{program} {sub} destroy removes a machine"))),
                ("secrets", "set") | ("secrets", "unset") => Some(Floor::new(FloorKind::Deploy, format!("{program} secrets {sub2} restarts the app"))),
                _ => None,
            }?;
            Some(match app {
                Some(app) => f.row("Where", format!("Fly app {app}")),
                None => f,
            })
        }
        "wrangler" => {
            let pages = sub == "pages";
            match (sub, sub2) {
                ("deploy", _) | ("publish", _) | ("pages", "deploy") | ("pages", "publish") | ("versions", "deploy") | ("rollback", _) => {
                    // A Pages site holds no data; a Worker might (D1, KV, R2)
                    let mut c = ctx.clone();
                    if pages {
                        c.deploy_hint = Some("cloudflare_pages".into());
                    }
                    Some(c.deploy(format!("wrangler {sub} ships to Cloudflare")))
                }
                ("d1", "execute") => {
                    let sql = flag_value(a, &["--command"]).unwrap_or("");
                    destructive_sql(sql).map(|w| destroy(format!("wrangler d1 execute runs {w}")))
                }
                ("d1", "delete") | ("r2", _) | ("kv", _) | ("delete", _) => {
                    let removes = sub == "delete" || sub2 == "delete" || a.iter().any(|x| *x == "delete" || *x == "rm");
                    removes.then(|| destroy(format!("wrangler {sub} {sub2} deletes data on Cloudflare")))
                }
                _ => None,
            }
        }
        "docker" | "docker-compose" | "podman" => docker_reason(program, a, ctx, over_ssh),
        "kubectl" | "oc" => match sub {
            "apply" | "rollout" | "set" | "replace" | "patch" | "scale" | "create" => Some(ctx.deploy(format!("{program} {sub} changes the cluster"))),
            "delete" | "drain" => Some(destroy(format!("{program} {sub} removes it from the cluster"))),
            _ => None,
        },
        "helm" => match sub {
            "install" | "upgrade" | "rollback" => Some(ctx.deploy(format!("helm {sub} changes the cluster"))),
            "uninstall" | "delete" => Some(destroy("helm uninstall removes a release".into())),
            _ => None,
        },
        "terraform" | "tofu" | "terragrunt" => match sub {
            "apply" => Some(if has(a, "-destroy") { destroy(format!("{program} apply -destroy removes infrastructure")) } else { ctx.deploy(format!("{program} apply changes infrastructure")) }),
            "destroy" => Some(destroy(format!("{program} destroy removes infrastructure"))),
            _ => None,
        },
        "pulumi" => match sub {
            "up" | "update" => Some(ctx.deploy("pulumi up changes infrastructure".into())),
            "destroy" => Some(destroy("pulumi destroy removes infrastructure".into())),
            _ => None,
        },
        "vercel" => (has(a, "--prod") || sub == "promote" || sub == "rollback" || (sub == "deploy" && has(a, "--prod"))).then(|| ctx.deploy("vercel ships to production".into())),
        "netlify" => (sub == "deploy" && has(a, "--prod")).then(|| ctx.deploy("netlify deploy --prod ships the site".into())),
        "firebase" => (sub == "deploy").then(|| ctx.deploy("firebase deploy ships the project".into())),
        "railway" => (sub == "up" || sub == "redeploy").then(|| ctx.deploy(format!("railway {sub} ships the app"))),
        "kamal" | "cap" | "mina" => (a.contains(&"deploy") || sub == "rollback").then(|| ctx.deploy(format!("{program} deploy ships the app"))),
        "serverless" | "sls" | "sst" | "cdk" | "amplify" | "eb" | "zappa" => (a.contains(&"deploy") || a.contains(&"publish")).then(|| ctx.deploy(format!("{program} deploy ships the app"))),
        "heroku" => match sub {
            "releases:rollback" | "container:release" | "restart" => Some(ctx.deploy(format!("heroku {sub} changes the running app"))),
            "pg:reset" | "apps:destroy" | "destroy" => Some(destroy(format!("heroku {sub} deletes data"))),
            _ => None,
        },
        "gcloud" => (a.contains(&"deploy")).then(|| ctx.deploy("gcloud … deploy ships the service".into())),
        "aws" => {
            if a.contains(&"deploy") || a.contains(&"update-service") || a.contains(&"update-function-code") {
                Some(ctx.deploy("aws … changes a running service".into()))
            } else if (sub == "s3" && (sub2 == "rb" || (sub2 == "rm" && has(a, "--recursive")))) || a.iter().any(|x| x.starts_with("delete-") || x.starts_with("terminate-")) {
                Some(destroy("an aws command that deletes resources or data".into()))
            } else {
                None
            }
        }
        "gsutil" => ((sub == "rm" && (has(a, "-r") || has(a, "-rf"))) || sub == "rb").then(|| destroy("gsutil deletes a bucket or its objects".into())),
        "make" | "just" | "task" | "mage" | "rake" => pos.iter().find(|t| ship_name(t)).map(|t| ctx.deploy(format!("{program} {t}"))),
        "npm" | "yarn" | "pnpm" | "bun" => {
            let script = if sub == "run" || sub == "run-script" { sub2 } else { sub };
            ship_name(script).then(|| ctx.deploy(format!("{program} runs the {script} script")))
        }
        "rm" | "shred" | "unlink" => rm_reason(program, a),
        "truncate" => a.iter().any(|x| is_db_file(x)).then(|| destroy("truncate empties a database file".into())),
        "dd" => a.iter().any(|x| x.starts_with("of=/dev/")).then(|| destroy("dd writes over a disk".into())),
        p if p.starts_with("mkfs") => Some(destroy(format!("{p} formats a disk"))),
        "dropdb" | "dropuser" => Some(destroy(format!("{program} deletes a database"))),
        "redis-cli" => a.iter().any(|x| x.eq_ignore_ascii_case("flushall") || x.eq_ignore_ascii_case("flushdb")).then(|| destroy("redis-cli empties the store".into())),
        // Migrations run against production
        _ if migrate_cmd(program, a) => (env_prod || a.iter().any(|x| is_prod_flag(x) || prod_env(x)) || flag_value(a, &["--settings"]).is_some_and(prod_settings))
            .then(|| Floor::new(FloorKind::DeployMigration, format!("{program} migrates a production database")).row("Migration", "Runs database migrations on production.")),
        _ => None,
    }
}

/// The script `python|node|ruby [flags] script.py …` runs (`python -m pkg` runs `pkg`); `None`
/// for `-c`/`-e` code or another program.
fn interpreter_script<'a>(program: &str, a: &[&'a str]) -> Option<&'a str> {
    let (takes, code): (&[&str], &[&str]) = if program.starts_with("python") && !program.contains('-') {
        (&["-W", "-X", "-Q"], &["-c"])
    } else if matches!(program, "node" | "nodejs") {
        (&["-r", "--require", "--import", "--loader", "--experimental-loader", "--env-file", "--conditions", "-C"], &["-e", "--eval", "-p", "--print"])
    } else if program == "ruby" {
        (&["-r", "-I", "-C", "-E", "--encoding"], &["-e"])
    } else {
        return None;
    };
    let mut i = 0;
    while let Some(x) = a.get(i) {
        if code.contains(x) {
            return None;
        }
        if *x == "-m" {
            return a.get(i + 1).copied();
        }
        if takes.contains(x) {
            i += 2;
        } else if x.starts_with('-') {
            i += 1;
        } else {
            return Some(x);
        }
    }
    None
}

/// A Django settings module for production: `app.settings.prod`, `settings_production`.
fn prod_settings(v: &str) -> bool {
    let last = v.rsplit(['.', '_', '-']).next().unwrap_or("").to_lowercase();
    matches!(last.as_str(), "prod" | "production" | "live")
}

fn ship_name(name: &str) -> bool {
    let ws = words(name);
    ws.iter().any(|w| SHIP_WORDS.contains(&w.as_str()))
}

fn flag_value<'a>(a: &[&'a str], names: &[&str]) -> Option<&'a str> {
    for (i, x) in a.iter().enumerate() {
        for n in names {
            if x == n {
                return a.get(i + 1).copied();
            }
            if let Some(v) = x.strip_prefix(&format!("{n}=")) {
                return Some(v);
            }
        }
    }
    None
}

fn is_prod_flag(x: &str) -> bool {
    let l = x.to_lowercase();
    matches!(l.as_str(), "--prod" | "--production" | "production" | "prod" | "-e=production" | "--env=production" | "--env=prod" | "--environment=production")
        || remote_url(&l)
}

fn migrate_cmd(program: &str, a: &[&str]) -> bool {
    let joined = a.join(" ").to_lowercase();
    match program {
        "sqlx" | "diesel" | "refinery" | "goose" | "migrate" | "dbmate" | "flyway" | "liquibase" | "atlas" => joined.contains("migrat") || joined.contains("up") || joined.contains("apply"),
        "rails" | "rake" | "bin/rails" => joined.contains("db:migrate") || joined.contains("db:schema:load"),
        "prisma" => joined.contains("migrate deploy") || joined.contains("db push"),
        "alembic" => joined.contains("upgrade") || joined.contains("downgrade"),
        "knex" => joined.contains("migrate:"),
        "python" | "python3" => joined.contains("manage.py migrate"),
        "php" => joined.contains("artisan migrate"),
        _ => false,
    }
}

fn is_db_file(x: &str) -> bool {
    let l = x.to_lowercase();
    l.ends_with(".db") || l.ends_with(".sqlite") || l.ends_with(".sqlite3") || l.ends_with(".db-wal")
}

/// `rm -r` of a place that holds data, or of a database file.
fn rm_reason(program: &str, a: &[&str]) -> Option<Floor> {
    let recursive = a.iter().any(|x| {
        x.starts_with('-') && !x.starts_with("--") && (x.contains('r') || x.contains('R')) || *x == "--recursive"
    });
    let targets = positional(a);
    let destroy = |what: &str| Some(Floor::new(FloorKind::Destructive, format!("{program} deletes {what}")).row("Data", "Can't be undone without a backup."));
    if let Some(t) = targets.iter().find(|t| is_db_file(t)) {
        return destroy(&format!("the database file {t}"));
    }
    if !recursive && program == "rm" {
        return None;
    }
    for t in &targets {
        let t = t.trim_end_matches('/');
        if matches!(t, "" | "/" | "/*" | "~" | "~/*" | "$HOME" | "${HOME}" | "*" | "." | "..") {
            return destroy(&format!("everything in {}", if t.is_empty() { "/" } else { t }));
        }
        let parts = words(t);
        if parts.iter().any(|p| DATA_WORDS.contains(&p.as_str())) && !t.contains("node_modules") && !t.starts_with("/tmp") && !t.contains("target") {
            return destroy(&format!("{t}, which looks like data"));
        }
    }
    None
}

/// `over_ssh`: run on another machine through `ssh`.
fn docker_reason(program: &str, a: &[&str], ctx: &DevCtx, over_ssh: bool) -> Option<Floor> {
    // docker's and compose's options that take a value: `--context prod`, `-f compose.prod.yml`
    const TAKES: &[&str] = &[
        "-H", "--host", "--context", "-c", "--config", "-l", "--log-level", "-f", "--file", "-p", "--project-name", "--profile", "--env-file",
        "--project-directory", "--ansi",
    ];
    let mut pos: Vec<&str> = Vec::new();
    let mut k = 0;
    while let Some(x) = a.get(k) {
        if TAKES.contains(x) {
            k += 2;
            continue;
        }
        if !x.starts_with('-') {
            pos.push(x);
        }
        k += 1;
    }
    let compose = program == "docker-compose" || pos.first() == Some(&"compose");
    let cmd: Vec<&str> = if compose && program != "docker-compose" { pos[1..].to_vec() } else { pos.clone() };
    let sub = cmd.first().copied().unwrap_or("");
    let destroy = |why: String| Floor::new(FloorKind::Destructive, why).row("Data", "Can't be undone without a backup.");
    let remote = flag_value(a, &["-H", "--host", "--context", "-c"]);
    let f = if compose {
        match sub {
            "down" if has(a, "-v") || has(a, "--volumes") => Some(destroy("docker compose down -v deletes the volumes".into())),
            // On a laptop compose is local dev: it ships only where the space deploys with Docker,
            // on another host (`-H`, `--context`) or over ssh (plan §3). down, stop, pull, rm and
            // create don't ship anything.
            "up" | "restart" if ctx.deploy_hint.as_deref() == Some("docker") || remote.is_some() || over_ssh => {
                Some(ctx.deploy(format!("docker compose {sub} changes running containers")).row("Downtime", "While the containers restart."))
            }
            _ => None,
        }
    } else {
        match (sub, cmd.get(1).copied().unwrap_or("")) {
            ("volume", "rm") | ("volume", "prune") => Some(destroy(format!("{program} volume {} deletes volumes", cmd[1]))),
            ("system", "prune") if has(a, "--volumes") => Some(destroy(format!("{program} system prune --volumes deletes volumes"))),
            ("rm", _) if has(a, "-v") || has(a, "--volumes") || has(a, "-fv") || has(a, "-vf") => Some(destroy(format!("{program} rm -v deletes a container's volumes"))),
            ("stack", "deploy") | ("service", "update") | ("service", "create") | ("service", "rollback") => Some(ctx.deploy(format!("{program} {sub} {} changes the swarm", cmd[1]))),
            ("stack", "rm") | ("service", "rm") => Some(destroy(format!("{program} {sub} rm removes it from the swarm"))),
            // Replacing a running container: rm -f, then run with a name or a restart policy
            ("run", _) if (flag_value(a, &["--name"]).is_some() && (flag_value(a, &["--restart"]).is_some() || has(a, "-d"))) || remote.is_some() => {
                Some(ctx.deploy(format!("{program} run starts a long-running container")).row("Downtime", "While the container restarts."))
            }
            ("rm", _) | ("stop", _) | ("restart", _) | ("kill", _) if remote.is_some() || has(a, "-f") || sub == "restart" => {
                Some(ctx.deploy(format!("{program} {sub} stops or replaces a running container")).row("Downtime", "While the container restarts."))
            }
            _ => None,
        }
    }?;
    Some(match remote {
        Some(r) => f.row("Where", r.to_string()),
        None => f,
    })
}

/// `git push` to a protected branch, a force-push, a ref deletion, a tag or a deploy remote.
fn git_reason(a: &[&str], ctx: &DevCtx) -> Option<Floor> {
    // Global options before the subcommand: -C dir, -c k=v
    let mut i = 0;
    while i < a.len() && a[i].starts_with('-') {
        i += if matches!(a[i], "-C" | "-c" | "--git-dir" | "--work-tree") { 2 } else { 1 };
    }
    if a.get(i) != Some(&"push") {
        return None;
    }
    let args = &a[i + 1..];
    let mut force = false;
    let mut delete = false;
    let mut all = false;
    let mut tags = false;
    let mut pos: Vec<&str> = Vec::new();
    let mut j = 0;
    while j < args.len() {
        let x = args[j];
        match x {
            "-f" | "--force" | "--force-if-includes" => force = true,
            _ if x.starts_with("--force-with-lease") => force = true,
            "-d" | "--delete" => delete = true,
            "--all" | "--branches" => all = true,
            "--mirror" => {
                all = true;
                force = true;
                delete = true;
            }
            "--tags" => tags = true,
            "-o" | "--push-option" | "--repo" | "--receive-pack" | "--exec" => j += 1,
            _ if x.starts_with('-') => {
                // Combined short flags: -uf
                if !x.starts_with("--") && x.contains('f') {
                    force = true;
                }
                if !x.starts_with("--") && x.contains('d') {
                    delete = true;
                }
            }
            _ => pos.push(x),
        }
        j += 1;
    }
    let remote = pos.first().copied();
    let refspecs: Vec<&str> = pos.iter().skip(1).copied().collect();
    let mut best: Option<Floor> = None;
    let mut add = |f: Floor| best = Some(match best.take() { Some(b) => b.merge(f), None => f });
    if let Some(r) = remote {
        if DEPLOY_REMOTES.contains(&r.to_lowercase().as_str()) {
            add(ctx.deploy(format!("git push {r} deploys the app")).row("Where", format!("the {r} remote")));
        }
    }
    if tags {
        add(ctx.deploy("git push --tags pushes tags, which can start a release".into()));
    }
    if all {
        add(Floor::new(if force { FloorKind::ForcePush } else { FloorKind::PushMain }, "git push --all pushes every branch, the protected ones too"));
    }
    // The branches this push writes, and how
    let mut targets: Vec<(String, bool, bool)> = Vec::new(); // (dst, force, delete)
    if refspecs.is_empty() && !all && !tags {
        // A bare push writes the checked-out branch; unknown counts as protected
        let head = ctx.head_branch.clone().unwrap_or_default();
        targets.push((head, force, delete));
    }
    for spec in refspecs {
        let plus = spec.starts_with('+');
        let spec = spec.trim_start_matches('+');
        let (src, dst) = match spec.split_once(':') {
            Some((s, d)) => (s, if d.is_empty() { s } else { d }),
            None => (spec, spec),
        };
        let del = delete || src.is_empty();
        let dst = if dst == "HEAD" || dst == "@" { ctx.head_branch.clone().unwrap_or_default() } else { dst.to_string() };
        targets.push((dst, force || plus, del));
    }
    for (dst, force, del) in targets {
        let tag = dst.starts_with("refs/tags/") || looks_like_version(&dst);
        let protected = dst.is_empty() || ctx.is_protected(&dst);
        let shown = if dst.is_empty() { "the checked-out branch (kwant can't tell which)".to_string() } else { dst.trim_start_matches("refs/heads/").to_string() };
        if del && (protected || tag) {
            add(Floor::new(FloorKind::DeleteRef, format!("git push deletes {shown} on the remote")).row("What changes", format!("{shown} is deleted on GitHub.")));
        } else if tag {
            add(ctx.deploy(format!("git push sends the tag {shown}, which can start a release")));
        } else if protected && force {
            add(Floor::new(FloorKind::ForcePush, format!("a force-push to {shown}")).row("What changes", format!("{shown} is rewritten; commits others have can be lost.")));
        } else if protected {
            add(Floor::new(FloorKind::PushMain, format!("git push to {shown}")).row("What changes", format!("New commits land on {shown}.")));
        }
    }
    best
}

fn looks_like_version(r: &str) -> bool {
    // `v0.7.0`, `0.7.0`; a branch like `release/0.7` isn't a tag
    if r.contains('/') {
        return false;
    }
    let r = r.strip_prefix('v').unwrap_or(r);
    let mut parts = r.split('.');
    parts.next().is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())) && parts.next().is_some_and(|p| p.chars().next().is_some_and(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> DevCtx {
        DevCtx { branch: "main".into(), head_branch: Some("lin-dev-core".into()), deploy_hint: Some("docker".into()), ..Default::default() }
    }

    fn kind(cmd: &str, c: &DevCtx) -> Option<FloorKind> {
        shell_reason(cmd, c).map(|f| f.kind)
    }

    fn req(server: &str, tool: &str, args: Value) -> ToolApprovalRequest {
        ToolApprovalRequest {
            kwant_id: "k".into(),
            kwant_name: "Demo".into(),
            turn_id: "t".into(),
            tool_name: format!("mcp__{server}__{tool}"),
            server: server.into(),
            tool: tool.into(),
            arguments: args,
            builtin: None,
            smart: None,
            payment: None,
        }
    }

    #[test]
    fn pushes_to_protected_branches_ask() {
        use FloorKind::*;
        let c = ctx();
        let table: &[(&str, Option<FloorKind>)] = &[
            ("git push origin main", Some(PushMain)),
            ("git push origin HEAD:main", Some(PushMain)),
            ("git push origin feature:refs/heads/main", Some(PushMain)),
            ("git push -u origin master", Some(PushMain)),
            ("git -C /srv/app push origin main", Some(PushMain)),
            ("cd app && git push origin main", Some(PushMain)),
            ("git push origin release/0.7", Some(PushMain)),
            ("git push origin gh-pages", Some(PushMain)),
            ("git push --all origin", Some(PushMain)),
            ("gh pr merge 301 --squash", Some(PushMain)),
            // Feature branches don't
            ("git push origin lin-dev-core", None),
            ("git push -u origin pixel-mobile-room", None),
            ("git push", None),
            ("git push origin", None),
            ("git push origin HEAD", None),
            ("git push origin mainline-fix", None),
            ("git status && git log -1", None),
            ("git pull origin main", None),
            ("git fetch origin main", None),
        ];
        for (cmd, want) in table {
            assert_eq!(kind(cmd, &c), *want, "{cmd}");
        }
    }

    #[test]
    fn a_bare_push_from_main_or_an_unknown_branch_asks() {
        let mut c = ctx();
        c.head_branch = Some("main".into());
        assert_eq!(kind("git push", &c), Some(FloorKind::PushMain));
        assert_eq!(kind("git push origin HEAD", &c), Some(FloorKind::PushMain));
        c.head_branch = None;
        assert_eq!(kind("git push", &c), Some(FloorKind::PushMain), "unknown counts as protected");
        let f = shell_reason("git push", &c).unwrap();
        assert!(f.why.contains("can't tell"), "{}", f.why);
    }

    #[test]
    fn the_watched_branch_and_extra_protected_ones_count() {
        let c = DevCtx { branch: "develop".into(), protected: vec!["staging".into()], head_branch: Some("x".into()), ..Default::default() };
        assert_eq!(kind("git push origin develop", &c), Some(FloorKind::PushMain));
        assert_eq!(kind("git push origin staging", &c), Some(FloorKind::PushMain));
        assert_eq!(kind("git push origin main", &c), Some(FloorKind::PushMain), "main is always protected");
    }

    #[test]
    fn force_pushes_and_deletions_of_protected_refs_ask() {
        use FloorKind::*;
        let c = ctx();
        let table: &[(&str, Option<FloorKind>)] = &[
            ("git push -f origin main", Some(ForcePush)),
            ("git push --force origin main", Some(ForcePush)),
            ("git push --force-with-lease origin main", Some(ForcePush)),
            ("git push --force-with-lease=main:abc origin main", Some(ForcePush)),
            ("git push origin +main", Some(ForcePush)),
            ("git push origin +HEAD:main", Some(ForcePush)),
            ("git push -uf origin main", Some(ForcePush)),
            ("git push --mirror", Some(ForcePush)),
            ("git push origin --delete main", Some(DeleteRef)),
            ("git push origin :main", Some(DeleteRef)),
            ("git push origin :refs/tags/mac-v0.6.12", Some(DeleteRef)),
            ("git push -d origin v0.6.12", Some(DeleteRef)),
            // A force-push or deletion of a feature branch is routine
            ("git push -f origin lin-dev-core", None),
            ("git push --force-with-lease origin pixel-dev-switch", None),
            ("git push origin --delete old-feature", None),
            ("git push origin :old-feature", None),
        ];
        for (cmd, want) in table {
            assert_eq!(kind(cmd, &c), *want, "{cmd}");
        }
    }

    #[test]
    fn deploys_ask_and_count_as_migrations_unless_known_otherwise() {
        use FloorKind::*;
        let c = ctx();
        let table: &[(&str, Option<FloorKind>)] = &[
            ("fly deploy", Some(DeployMigration)),
            ("flyctl deploy -a demo-app", Some(DeployMigration)),
            ("npx wrangler deploy", Some(DeployMigration)),
            ("wrangler pages deploy dist --project-name demo", Some(Deploy)),
            ("docker compose up -d", Some(DeployMigration)),
            ("docker compose -f compose.prod.yml pull && docker compose up -d --force-recreate", Some(DeployMigration)),
            ("docker-compose up -d", Some(DeployMigration)),
            ("docker stack deploy -c stack.yml demo", Some(DeployMigration)),
            ("docker run -d --name kwant-demo --restart unless-stopped demo:1", Some(DeployMigration)),
            ("docker rm -f kwant-demo", Some(DeployMigration)),
            ("kubectl apply -f k8s/", Some(DeployMigration)),
            ("helm upgrade demo ./chart", Some(DeployMigration)),
            ("terraform apply -auto-approve", Some(DeployMigration)),
            ("vercel --prod", Some(DeployMigration)),
            ("netlify deploy --prod", Some(DeployMigration)),
            ("firebase deploy", Some(DeployMigration)),
            ("gcloud run deploy demo --image x", Some(DeployMigration)),
            ("git push heroku main", Some(DeployMigration)),
            ("git push origin v0.7.0", Some(DeployMigration)),
            ("git push --tags", Some(DeployMigration)),
            ("gh workflow run deploy.yml", Some(DeployMigration)),
            ("gh release create v0.7.0", Some(DeployMigration)),
            ("make deploy", Some(DeployMigration)),
            ("npm run deploy:prod", Some(DeployMigration)),
            ("pnpm release", Some(DeployMigration)),
            ("./scripts/deploy.sh", Some(DeployMigration)),
            ("bash scripts/deploy-prod.sh", Some(DeployMigration)),
            ("bash -c 'fly deploy'", Some(DeployMigration)),
            ("sudo docker compose up -d", Some(DeployMigration)),
            // Building and testing isn't deploying
            ("docker build -t demo .", None),
            ("docker compose build", None),
            ("docker compose logs -f", None),
            ("docker ps", None),
            ("docker run --rm demo cargo test", None),
            ("npm run build", None),
            ("npm test", None),
            ("make test", None),
            ("vercel", None),
            ("netlify deploy", None),
            ("wrangler dev", None),
            ("kubectl get pods", None),
            ("terraform plan", None),
            ("fly status", None),
            ("fly logs -a demo-app", None),
            ("cargo build --release", None),
            ("gh pr create --fill", None),
            ("gh workflow run ci.yml", None),
        ];
        for (cmd, want) in table {
            assert_eq!(kind(cmd, &c), *want, "{cmd}");
        }
    }

    #[test]
    fn a_deploy_with_no_data_or_no_migration_is_plain() {
        let pages = DevCtx { deploy_hint: Some("cloudflare_pages".into()), ..ctx() };
        let f = shell_reason("npx wrangler deploy", &pages).unwrap();
        assert_eq!(f.kind, FloorKind::Deploy);
        assert!(!f.needs_backup);
        assert!(f.blast.iter().any(|(l, v)| l == "Data" && v.contains("Nothing to back up")));
        let known = DevCtx { migration: Some(false), ..ctx() };
        let f = shell_reason("fly deploy", &known).unwrap();
        assert_eq!(f.kind, FloorKind::Deploy);
        assert!(!f.needs_backup);
        let unknown = shell_reason("fly deploy", &ctx()).unwrap();
        assert!(unknown.needs_backup);
        assert!(unknown.blast.iter().any(|(l, v)| l == "Migration" && v.contains("can't tell")));
    }

    #[test]
    fn destructive_commands_need_a_backup() {
        use FloorKind::*;
        let c = ctx();
        let table: &[(&str, Option<FloorKind>)] = &[
            ("rm -rf /", Some(Destructive)),
            ("rm -rf ~", Some(Destructive)),
            ("rm -rf *", Some(Destructive)),
            ("rm -rf /srv/app/data", Some(Destructive)),
            ("rm -rf ./uploads/", Some(Destructive)),
            ("rm -r /var/lib/postgresql", Some(Destructive)),
            ("rm kwnt.db", Some(Destructive)),
            ("rm -f data/app.sqlite3", Some(Destructive)),
            ("docker volume rm kwant-data", Some(Destructive)),
            ("docker volume prune -f", Some(Destructive)),
            ("docker compose down -v", Some(Destructive)),
            ("docker system prune -a --volumes", Some(Destructive)),
            ("sqlite3 kwnt.db 'DROP TABLE tasks'", Some(Destructive)),
            ("sqlite3 kwnt.db \"delete from tasks\"", Some(Destructive)),
            ("psql $DATABASE_URL -c 'TRUNCATE users'", Some(Destructive)),
            ("docker exec db psql -c 'drop database app'", Some(Destructive)),
            ("mysql -e 'ALTER TABLE users DROP COLUMN email'", Some(Destructive)),
            ("redis-cli FLUSHALL", Some(Destructive)),
            ("dropdb app", Some(Destructive)),
            ("kubectl delete namespace prod", Some(Destructive)),
            ("terraform destroy", Some(Destructive)),
            ("fly apps destroy demo-app", Some(Destructive)),
            ("fly volumes destroy vol_123", Some(Destructive)),
            ("aws s3 rm s3://demo-bucket --recursive", Some(Destructive)),
            ("gh repo delete demo-org/demo", Some(Destructive)),
            ("ssh deploy@kwant.example.com 'rm -rf /srv/kwant/data'", Some(Destructive)),
            ("dd if=/dev/zero of=/dev/sda", Some(Destructive)),
            ("RAILS_ENV=production rails db:migrate", Some(DeployMigration)),
            ("DATABASE_URL=postgres://db.example.com/app sqlx migrate run", Some(DeployMigration)),
            // Everyday clean-ups and reads don't
            ("rm -rf target", None),
            ("rm -rf node_modules dist", None),
            ("rm -rf /tmp/build-data", None),
            ("rm notes.txt", None),
            ("rm -f src/old.rs", None),
            ("sqlite3 kwnt.db 'select count(*) from tasks'", None),
            ("sqlite3 kwnt.db \"delete from tasks where id = 'x'\"", None),
            ("grep -rn 'DROP TABLE' src", None),
            ("sqlx migrate run", None),
            ("DATABASE_URL=sqlite://./dev.db sqlx migrate run", None),
            ("cargo test -j 3 db::dev_mode", None),
            // F8: down only stops local containers
            ("docker compose down", None),
        ];
        for (cmd, want) in table {
            assert_eq!(kind(cmd, &c), *want, "{cmd}");
        }
        assert!(shell_reason("rm -rf /srv/app/data", &c).unwrap().needs_backup);
    }

    #[test]
    fn the_strongest_kind_wins_and_rows_are_kept() {
        let f = shell_reason("git push origin main && fly deploy -a demo-app", &ctx()).unwrap();
        assert_eq!(f.kind, FloorKind::DeployMigration);
        assert!(f.blast.iter().any(|(l, v)| l == "Where" && v == "Fly app demo-app"));
        assert!(f.blast.iter().any(|(l, _)| l == "What changes"));
        let f = shell_reason("ssh deploy@kwant.example.com docker compose up -d", &ctx()).unwrap();
        assert!(f.blast.iter().any(|(l, v)| l == "Where" && v == "kwant.example.com"), "{:?}", f.blast);
    }

    #[test]
    fn claude_bash_and_local_shell_go_through_the_shell_rules() {
        let c = ctx();
        let mut r = req("claude", "Bash", json!({"command": "fly deploy"}));
        r.builtin = Some(crate::mcp::BuiltinCall { cwd: None, outside_folder: false });
        assert_eq!(reason(&r, &c).map(|f| f.kind), Some(FloorKind::DeployMigration));
        let r = req("local", "execute_shell", json!({"command": "git push -f origin main"}));
        assert_eq!(reason(&r, &c).map(|f| f.kind), Some(FloorKind::ForcePush));
        let mut r = req("claude", "Write", json!({"file_path": "deploy.sh", "content": "fly deploy"}));
        r.builtin = Some(crate::mcp::BuiltinCall { cwd: None, outside_folder: false });
        assert_eq!(reason(&r, &c), None, "writing a script isn't running it");
    }

    #[test]
    fn mcp_deploy_and_destroy_tools_ask() {
        let c = ctx();
        let k = |server: &str, tool: &str, args: Value| reason(&req(server, tool, args), &c).map(|f| f.kind);
        assert_eq!(k("cloudflare", "workers_deploy", json!({})), Some(FloorKind::DeployMigration));
        assert_eq!(k("vercel", "deploy_to_vercel", json!({})), Some(FloorKind::DeployMigration));
        assert_eq!(k("digitalocean-droplets", "droplet-delete", json!({"id": 1})), Some(FloorKind::Destructive));
        assert_eq!(k("digitalocean-droplets", "rebuild-droplet", json!({"id": 1})), Some(FloorKind::Destructive));
        assert_eq!(k("supabase", "execute_sql", json!({"query": "DROP TABLE users"})), Some(FloorKind::Destructive));
        assert_eq!(k("composio", "COMPOSIO_REMOTE_BASH_TOOL", json!({"command": "fly deploy"})), Some(FloorKind::DeployMigration));
        // Reads and unrelated writes don't
        assert_eq!(k("digitalocean-droplets", "droplet-list", json!({})), None);
        assert_eq!(k("cloudflare", "search", json!({"query": "delete worker"})), None);
        assert_eq!(k("supabase", "execute_sql", json!({"query": "select * from users"})), None);
        assert_eq!(k("slack", "delete_message", json!({})), None);
        assert_eq!(k("github", "create_issue", json!({})), None);
        assert_eq!(k("notion", "publish_page", json!({})), None);
    }

    #[test]
    fn production_connections_always_ask() {
        let c = DevCtx { prod_connections: vec!["digitalocean-droplets".into(), "stripe".into()], ..ctx() };
        let f = reason(&req("digitalocean-droplets", "droplet-list", json!({})), &c).unwrap();
        assert_eq!(f.kind, FloorKind::ProdKey);
        assert!(!f.needs_backup);
        assert!(f.blast.iter().any(|(l, v)| l == "Key used" && v == "digitalocean-droplets · production"));
        // Composio: the toolkit is the slug's first word
        let f = reason(&req("composio", "COMPOSIO_MULTI_EXECUTE_TOOL", json!({"tools": [{"tool_slug": "STRIPE_CREATE_REFUND"}]})), &c).unwrap();
        assert_eq!(f.kind, FloorKind::ProdKey);
        // A destructive call through a production key keeps the stronger kind and the key row
        let f = reason(&req("digitalocean-droplets", "droplet-delete", json!({})), &c).unwrap();
        assert_eq!(f.kind, FloorKind::Destructive);
        assert!(f.blast.iter().any(|(l, _)| l == "Key used"));
        // Not marked: no card
        assert_eq!(reason(&req("github", "list_issues", json!({})), &c), None);
    }

    #[test]
    fn the_payload_serialises_as_the_contract_says() {
        let f = shell_reason("rm -rf /srv/app/data", &ctx()).unwrap();
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["kind"], "destructive");
        assert_eq!(v["needs_backup"], true);
        assert_eq!(v["blast"][0][0], "Data");
        for k in [FloorKind::PushMain, FloorKind::ForcePush, FloorKind::DeleteRef, FloorKind::Deploy, FloorKind::DeployMigration, FloorKind::ProdKey] {
            let s = serde_json::to_value(k).unwrap();
            assert!(s.as_str().unwrap().chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        }
        assert!(FloorKind::Destructive.needs_backup() && FloorKind::DeployMigration.needs_backup());
        assert!(!FloorKind::Deploy.needs_backup() && !FloorKind::PushMain.needs_backup() && !FloorKind::ProdKey.needs_backup());
    }

    #[test]
    fn destructive_sql_reads_the_statement() {
        assert!(destructive_sql("DROP TABLE x").is_some());
        assert!(destructive_sql("drop   table x").is_some());
        assert!(destructive_sql("TRUNCATE users").is_some());
        assert!(destructive_sql("delete from t; select 1").is_some());
        assert!(destructive_sql("DELETE FROM t WHERE id=1").is_none());
        assert!(destructive_sql("select * from drops").is_none());
        assert!(destructive_sql("db.users.deleteMany({})").is_some());
        assert!(destructive_sql("insert into t values (1)").is_none());
    }

    /// F4 (Rigel, #313): `UPDATE … SET` with no `WHERE` rewrites every row.
    #[test]
    fn update_with_no_where_is_destructive() {
        use FloorKind::*;
        let c = ctx();
        for (cmd, want) in [
            ("psql -c 'UPDATE users SET admin=1'", Some(Destructive)),
            ("sqlite3 app.db \"UPDATE t SET x=1\"", Some(Destructive)),
            ("psql -c 'BEGIN; update only accounts set balance = 0; COMMIT'", Some(Destructive)),
            ("mysql -e \"UPDATE users SET admin=1 WHERE id=7\"", None),
            ("sqlite3 app.db \"update t set x='a b' where id = 1\"", None),
            ("psql -c 'select last_update from t'", None),
        ] {
            assert_eq!(kind(cmd, &c), want, "{cmd}");
        }
        assert_eq!(destructive_sql("UPDATE users SET plan = 'free'").as_deref(), Some("UPDATE with no WHERE"));
        // Prose in a message an MCP tool sends isn't SQL
        assert!(destructive_sql("Please update the docs and set a date for the release").is_none());
        assert!(destructive_sql("We'll update everything set up last week").is_none());
    }

    /// F5: a `git checkout`/`git switch` earlier in the chain changes what a bare push pushes.
    #[test]
    fn a_checkout_earlier_in_the_chain_moves_the_head() {
        use FloorKind::*;
        let c = DevCtx { head_branch: Some("feature/x".into()), ..ctx() };
        for (cmd, want) in [
            ("git checkout main && git merge feature/x && git push", Some(PushMain)),
            ("git switch main; git push", Some(PushMain)),
            ("git -C . checkout release/1.2 && git push origin", Some(PushMain)),
            ("git checkout -b feature/deploy && git push", None),
            ("git switch -c feature/y && git push", None),
            ("git checkout -- src/main.rs && git push", None),
            ("git checkout . && git push", None),
            // A branch or a file: can't be told, so the push counts as protected
            ("git checkout other && git push", Some(PushMain)),
            ("git switch --detach v1.0 && git push", Some(PushMain)),
            ("git checkout - && git push", Some(PushMain)),
            ("git push && git checkout main", None),
        ] {
            assert_eq!(kind(cmd, &c), want, "{cmd}");
        }
    }

    /// F6: wrappers that run the rest of the line, with their own flags and values.
    #[test]
    fn wrappers_with_flags_are_read_through() {
        use FloorKind::*;
        let c = DevCtx { head_branch: Some("feature/x".into()), ..ctx() };
        for cmd in [
            "timeout 60 git push origin main",
            "timeout 10 git push origin main",
            "timeout -k 5 -s TERM 1m git push origin main",
            "nice git push origin main",
            "nice -n 10 git push origin main",
            "ionice -c3 git push origin main",
            "ionice -c 2 -n 7 git push origin main",
            "stdbuf -oL git push origin main",
            "setsid git push origin main",
            "watch -n1 'git push origin main'",
            "xargs -n 1 git push origin main",
            "eval 'git push origin main'",
            "eval \"git push origin main\"",
            "sudo -u deploy -E git push origin main",
            "env -u GIT_DIR git push origin main",
            "\\git push origin main",
            "nohup nice -n 5 timeout 30 git push origin main",
        ] {
            assert_eq!(kind(cmd, &c), Some(PushMain), "{cmd}");
        }
        assert_eq!(kind("timeout 60 fly deploy", &c), Some(DeployMigration));
        assert_eq!(kind("timeout 60 git push origin feature/x", &c), None);
        assert_eq!(kind("nice -n 10 cargo -j 2 test", &c), None);
        // A wrapper with nothing after it
        assert_eq!(kind("sudo -u", &c), None);
        assert_eq!(kind("timeout", &c), None);
    }

    /// F7: a script an interpreter runs is read by its name, like `./deploy.sh`.
    #[test]
    fn interpreter_scripts_are_read_by_name() {
        use FloorKind::*;
        let c = ctx();
        for (cmd, want) in [
            ("python release.py", Some(DeployMigration)),
            ("python3 deploy.py --env prod", Some(DeployMigration)),
            ("python3 -u scripts/ship_it.py", Some(DeployMigration)),
            ("node deploy.js", Some(DeployMigration)),
            ("node --require dotenv/config scripts/release.mjs", Some(DeployMigration)),
            ("ruby deploy.rb", Some(DeployMigration)),
            ("bash -x deploy.sh", Some(DeployMigration)),
            ("sh -o errexit scripts/release.sh", Some(DeployMigration)),
            ("python3 script.py", None),
            ("node build.js", None),
            ("python -c 'import sys' deploy.py", None),
            ("node -e 'console.log(1)'", None),
            ("python3 -m http.server 9481", None),
            ("python3 manage.py runserver", None),
            ("bash -x build.sh", None),
        ] {
            assert_eq!(kind(cmd, &c), want, "{cmd}");
        }
        // Not a deploy script: the migration rules still read it
        assert_eq!(kind("python manage.py migrate --settings=app.prod", &c), Some(DeployMigration));
    }

    /// F8: compose ships only where the space deploys with Docker, on another host, or over ssh.
    #[test]
    fn compose_ships_only_where_the_space_deploys_with_docker() {
        use FloorKind::*;
        let docker = ctx();
        let none = DevCtx { deploy_hint: None, ..ctx() };
        let fly = DevCtx { deploy_hint: Some("fly".into()), ..ctx() };
        for sub in ["down", "stop", "pull", "rm -f", "create", "build", "logs -f", "ps"] {
            for c in [&docker, &none] {
                assert_eq!(kind(&format!("docker compose {sub}"), c), None, "compose {sub} {:?}", c.deploy_hint);
                assert_eq!(kind(&format!("docker-compose {sub}"), c), None, "docker-compose {sub}");
            }
        }
        for cmd in ["docker compose up -d", "docker-compose up -d", "docker compose restart web"] {
            assert_eq!(kind(cmd, &docker), Some(DeployMigration), "{cmd}");
            assert_eq!(kind(cmd, &none), None, "{cmd}");
            assert_eq!(kind(cmd, &fly), None, "{cmd}");
        }
        // Another host or over ssh ships whatever the hint
        assert_eq!(kind("docker --context prod compose up -d", &none), Some(DeployMigration));
        assert_eq!(kind("ssh prod 'docker compose up -d'", &none), Some(DeployMigration));
        assert_eq!(kind("ssh deploy@app.example.com 'cd /srv/app && docker compose up -d'", &none), Some(DeployMigration));
        // Deleting volumes is destructive anywhere
        for c in [&docker, &none] {
            assert_eq!(kind("docker compose down -v", c), Some(Destructive));
            assert_eq!(kind("docker-compose down --volumes", c), Some(Destructive));
        }
    }

    /// F9: `RAILS_ENV=production` and `--settings=…prod` after the command.
    #[test]
    fn trailing_production_env_and_settings_migrate_production() {
        use FloorKind::*;
        let c = ctx();
        for (cmd, want) in [
            ("rails db:migrate RAILS_ENV=production", Some(DeployMigration)),
            ("bin/rails db:migrate RAILS_ENV=production", Some(DeployMigration)),
            ("RAILS_ENV=production rails db:migrate", Some(DeployMigration)),
            ("python manage.py migrate --settings=app.prod", Some(DeployMigration)),
            ("python3 manage.py migrate --settings app.settings.production", Some(DeployMigration)),
            ("python manage.py migrate --settings=config.settings_production", Some(DeployMigration)),
            ("rails db:migrate", None),
            ("rails db:migrate RAILS_ENV=development", None),
            ("python manage.py migrate", None),
            ("python manage.py migrate --settings=app.dev", None),
            ("python manage.py migrate --settings=app.products", None),
        ] {
            assert_eq!(kind(cmd, &c), want, "{cmd}");
        }
    }
}
