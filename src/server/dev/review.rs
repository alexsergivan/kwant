//! The review card (plan §3.4 `review`, design §5): an open PR head, read by a bot of the space that
//! isn't the author (D7). Rules give the risk, the test-tamper flags and the security tiles; the
//! reviewer's one turn gives the intent check and the findings.

use super::facts::{Author, DevFacts};
use super::generate::{ask_json, cut_of, diff_text, json_object, text_field, Model, DATA_RULES};
use super::risk::{self, Inputs, Risk};
use super::tamper::{self, Flag};
use crate::db::dev_mode::{FAILED, READY};
use crate::db::missions::Task;
use crate::db::models::Kwant;
use crate::server::AppState;
use serde_json::{json, Map, Value};

/// Copy for a space with no bot to review (D7).
pub const NO_REVIEWER: &str = "No reviewer: add a second bot to this space";
const MAX_ROWS: usize = 8;
const MAX_FINDINGS: usize = 12;
const BODY_CHARS: usize = 4_000;

/// The reviewer and how its engine compares with the author's.
#[derive(Debug, Clone)]
pub struct Picked {
    pub bot: Kwant,
    /// `claude`, `codex`, …; none when the bot is on auto.
    pub engine: Option<String>,
    /// Both engines are known and the same.
    pub same_engine: bool,
}

impl Picked {
    pub fn json(&self) -> Value {
        let pin = crate::router::parse_pin(&self.bot.preferred_model);
        json!({"name": self.bot.name, "engine": self.engine.as_deref().unwrap_or("auto"), "model": pin.model.as_deref().unwrap_or("auto")})
    }
}

/// The reviewer of a PR by `author` in `space_id`: the Swarm Auditor's ranking (never the author,
/// nor the author's mission or delegation lead), then the first bot on another engine than the
/// author's, else the first on auto, else the first.
pub async fn pick_reviewer(state: &AppState, space_id: &str, author: &Author) -> Option<Picked> {
    let task = stand_in_task(space_id, author);
    let ranked = crate::server::missions::auditor::ranked(state, &task, true).await;
    choose(ranked.into_iter().map(|(k, _)| k).collect(), author)
}

/// The ranked bots' pick (see [`pick_reviewer`]). The author's own bot never reviews.
pub fn choose(ranked: Vec<Kwant>, author: &Author) -> Option<Picked> {
    let author_id = author.bot.as_ref().map(|b| b.id.as_str());
    let author_engine = author.bot.as_ref().map(|b| b.engine.trim().to_ascii_lowercase()).filter(|e| !e.is_empty() && e != "auto");
    let cands: Vec<(Kwant, Option<String>)> = ranked
        .into_iter()
        .filter(|k| Some(k.id.as_str()) != author_id)
        .map(|k| {
            let engine = crate::router::parse_pin(&k.preferred_model).family;
            (k, engine)
        })
        .collect();
    let other = |e: &Option<String>| match (e, &author_engine) {
        (Some(e), Some(a)) => e != a,
        // A person's PR: any bot is another engine
        (Some(_), None) => true,
        (None, _) => false,
    };
    let at = cands.iter().position(|(_, e)| other(e)).or_else(|| cands.iter().position(|(_, e)| e.is_none())).or(if cands.is_empty() { None } else { Some(0) })?;
    let (bot, engine) = cands.into_iter().nth(at)?;
    let same_engine = matches!((&engine, &author_engine), (Some(e), Some(a)) if e == a);
    Some(Picked { bot, engine, same_engine })
}

/// The auditor picks for a task; a PR has none, so this stands in: owned by the author's bot, in
/// the card's space, with no mission or delegation (so only the author is left out).
fn stand_in_task(space_id: &str, author: &Author) -> Task {
    let now = chrono::Utc::now();
    Task {
        id: "dev_review".into(),
        space_id: space_id.into(),
        mission_id: None,
        title: String::new(),
        acceptance: String::new(),
        state: "review".into(),
        priority: 0,
        owner_kwant_id: author.bot.as_ref().map(|b| b.id.clone()),
        suggested_kwant_id: None,
        budget_tokens: 0,
        spent_tokens: 0,
        attempts: 0,
        max_attempts: 0,
        blocked_reason: None,
        source_kind: "dev".into(),
        source_id: None,
        channel_id: None,
        direct_kwant_id: None,
        thread_id: None,
        created_by: "kwant".into(),
        created_at: now,
        updated_at: now,
        finished_at: None,
        forked_from: None,
        held_at: None,
        recovery_spent_tokens: 0,
    }
}

/// `written_by` / `author`: the matched bot, else the person.
pub fn author_json(a: &Author) -> Value {
    match &a.bot {
        Some(b) => json!({"kind": "bot", "name": b.name, "engine": b.engine, "model": b.model}),
        None => {
            let name = if a.name.trim().is_empty() { a.login.clone().unwrap_or_else(|| "Someone".into()) } else { a.name.clone() };
            json!({"kind": "person", "name": name})
        }
    }
}

/// What the reviewer's reply must hold.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub asked: String,
    pub rows: Vec<(String, String)>,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub severity: String,
    pub title: String,
    pub file: String,
    pub line: u32,
    pub detail: String,
    pub fix: String,
}

/// Reads the reviewer's reply strictly: every row and finding must be well formed, and each
/// finding must point at a file of this diff.
pub fn parse_reply(reply: &str, paths: &[&str]) -> Result<Reply, String> {
    let m = json_object(reply)?;
    let intent = m.get("intent").and_then(Value::as_object).ok_or("`intent` is missing or not an object")?;
    let asked = text_field(intent, "asked", 400)?;
    let rows_v = intent.get("rows").and_then(Value::as_array).ok_or("`intent.rows` is missing or not a list")?;
    if rows_v.is_empty() || rows_v.len() > MAX_ROWS {
        return Err(format!("`intent.rows` must have 1 to {MAX_ROWS} rows"));
    }
    let mut rows = Vec::new();
    for (i, r) in rows_v.iter().enumerate() {
        let r = r.as_object().ok_or(format!("`intent.rows[{i}]` is not an object"))?;
        let state = r.get("state").and_then(Value::as_str).unwrap_or("");
        if !matches!(state, "ok" | "missing" | "extra") {
            return Err(format!("`intent.rows[{i}].state` must be ok, missing or extra"));
        }
        rows.push((state.to_string(), one_line(&text_field(r, "text", 200).map_err(|e| format!("intent.rows[{i}]: {e}"))?)));
    }
    let fs = m.get("findings").and_then(Value::as_array).ok_or("`findings` is missing or not a list")?;
    if fs.len() > MAX_FINDINGS {
        return Err(format!("more than {MAX_FINDINGS} findings"));
    }
    let mut findings = Vec::new();
    for (i, f) in fs.iter().enumerate() {
        let f = f.as_object().ok_or(format!("`findings[{i}]` is not an object"))?;
        let at = |e: String| format!("findings[{i}]: {e}");
        let severity = f.get("severity").and_then(Value::as_str).unwrap_or("");
        if !matches!(severity, "critical" | "high" | "medium" | "low") {
            return Err(at("`severity` must be critical, high, medium or low".into()));
        }
        let file = text_field(f, "file", 300).map_err(at)?;
        if !paths.contains(&file.as_str()) {
            return Err(at(format!("`file` {file:?} isn't a file of this PR")));
        }
        let line = f.get("line").and_then(Value::as_u64).filter(|l| (1..=1_000_000).contains(l)).ok_or_else(|| at("`line` must be a line number from 1".into()))? as u32;
        findings.push(Finding {
            severity: severity.into(),
            title: one_line(&text_field(f, "title", 120).map_err(at)?),
            file,
            line,
            detail: text_field(f, "detail", 800).map_err(at)?,
            fix: f.get("fix").and_then(Value::as_str).map(str::trim).unwrap_or("").chars().take(600).collect(),
        });
    }
    Ok(Reply { asked: one_line(&asked), rows, findings })
}

pub(super) fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The secrets and missing-package counts of a `security` object whose scan hit; Juno's scanners
/// fill it, and either count makes the card High.
pub fn security_hits(security: &Value) -> (u32, u32) {
    let hits = |k: &str| {
        let t = &security[k];
        if t["state"] == "hit" {
            t["count"].as_u64().unwrap_or(1).max(1) as u32
        } else {
            0
        }
    };
    (hits("secrets"), hits("packages"))
}

/// The card's `security` before the scanners land (plan §3.4: `not_run`, OSV and Semgrep later).
pub fn security_not_run() -> Value {
    json!({"secrets": {"state": "not_run", "count": 0}, "packages": {"state": "not_run", "count": 0}, "osv": {"state": "later"}, "semgrep": {"state": "later"}})
}

fn tamper_finding(n: usize, f: &Flag) -> Value {
    let (severity, title, fix) = match f.kind {
        tamper::Kind::DeletedAssert => ("medium", "A test check was deleted", "Put the check back, or say in the PR why it no longer holds."),
        tamper::Kind::Skip => ("high", "A test was switched off", "Run the test again, or link the issue that tracks turning it back on."),
        tamper::Kind::Loosened => ("medium", "A test expects less than before", "Keep the exact expectation unless the behaviour really changed."),
        tamper::Kind::Snapshot => ("low", "A snapshot was rewritten", "Check the new snapshot shows what the change meant to do."),
        tamper::Kind::CoverageDrop => ("medium", "Tests were removed while the code grew", "Add tests for the new code, or keep the ones that were removed."),
    };
    json!({"id": format!("t{n}"), "severity": severity, "title": title, "file": f.file, "line": f.line, "detail": f.why, "fix": fix,
           "source": "tamper", "state": "open", "sent_at": null, "dismissed": null})
}

fn prompt(facts: &DevFacts, reviewer: &Kwant, rules: &Risk, flags: &[Flag]) -> (String, Option<String>) {
    let (diff, left_out) = diff_text(&facts.files);
    let cut = cut_of(facts, left_out);
    let body: String = facts.body.chars().take(BODY_CHARS).collect();
    let task = match &facts.task {
        Some(t) => format!("The task this PR came from:\n<task>\n{}\n</task>", t.chars().take(BODY_CHARS).collect::<String>()),
        None => "There is no task text: compare the change with the PR's own title and description.".into(),
    };
    let flags = if flags.is_empty() { "none".to_string() } else { flags.iter().map(|f| format!("{:?} at {}:{}", f.kind, f.file, f.line)).collect::<Vec<_>>().join("; ") };
    let p = format!(
        "You are {name}, reviewing a pull request in {repo} that another bot or person wrote. Read it as a careful reviewer.\n\n\
{task}\n\n\
<pr>\n#{pr} {title}\nBranch {branch} into {base}\n\n{body}\n\nThe diff{cut_note}:\n{diff}</pr>\n\n\
kwant's rules already found: risk {level:?} ({reasons}); test-tamper flags: {flags}. Don't repeat those as findings.\n\n\
Reply with this JSON object:\n\
{{\"intent\": {{\"asked\": \"what the task (or the PR text) asked for, one sentence\", \
\"rows\": [{{\"state\": \"ok | missing | extra\", \"text\": \"one thing asked for and whether the diff does it (ok), \
something asked for that the diff doesn't do (missing), or something the diff does that wasn't asked (extra)\"}}]}}, \
\"findings\": [{{\"severity\": \"critical | high | medium | low\", \"title\": \"short\", \"file\": \"a path from the diff\", \
\"line\": 12, \"detail\": \"what is wrong and why it matters\", \"fix\": \"the change you suggest\"}}]}}\n\
1 to {MAX_ROWS} intent rows; at most {MAX_FINDINGS} findings, only real problems (bugs, missing checks, unsafe input, \
broken behaviour), none for style; an empty list is fine. `line` is a line number in the PR's version of `file`.\n\n{DATA_RULES}",
        name = reviewer.name,
        repo = facts.repo,
        pr = facts.pr.unwrap_or(0),
        title = facts.title,
        branch = facts.head_branch.as_deref().unwrap_or("?"),
        base = facts.base_branch,
        cut_note = cut.as_ref().map(|c| format!(" ({c})")).unwrap_or_default(),
        level = rules.level,
        reasons = rules.reasons.join(", "),
    );
    (p, cut)
}

/// Builds the review card: `ready` with the reviewer's text, `ready` with rules only when there's no
/// reviewer, `failed` (rules plus `error`) when the reviewer's reply wouldn't do twice or the cap
/// is used up.
pub(super) async fn card<M: Model>(facts: &DevFacts, picked: Option<&Picked>, model: Option<&mut M>) -> (&'static str, Value) {
    let flags = tamper::check(&facts.files);
    let security = security_not_run();
    let (secrets, missing_packages) = security_hits(&security);
    let inputs = Inputs { files: &facts.files, first_edit_files: &facts.first_edit_files, tamper: &flags, outside_task: false, secrets, missing_packages };
    let rules = risk::assess(&inputs);
    let paths: Vec<&str> = facts.files.iter().map(|f| f.path.as_str()).collect();

    let mut status = READY;
    let mut error = None;
    let mut reply = None;
    let mut cut = facts.cut.clone();
    if let (Some(p), Some(model)) = (picked, model) {
        let (text, c) = prompt(facts, &p.bot, &rules, &flags);
        cut = c;
        match ask_json(model, &text, |r| parse_reply(r, &paths)).await {
            Ok(r) => reply = Some(r),
            Err(e) => (status, error) = (FAILED, Some(e)),
        }
    }

    let outside_task = reply.as_ref().is_some_and(|r| r.rows.iter().any(|(s, _)| s == "extra"));
    let risk = if outside_task { risk::assess(&Inputs { outside_task, ..inputs }) } else { rules };
    let mut findings: Vec<Value> = Vec::new();
    if let Some(r) = &reply {
        for (i, f) in r.findings.iter().enumerate() {
            findings.push(json!({"id": format!("f{}", i + 1), "severity": f.severity, "title": f.title, "file": f.file, "line": f.line,
                                 "detail": f.detail, "fix": f.fix, "source": "review", "state": "open", "sent_at": null, "dismissed": null}));
        }
    }
    findings.extend(flags.iter().enumerate().map(|(i, f)| tamper_finding(i + 1, f)));

    let mut card = Map::new();
    card.insert("kind".into(), json!("review"));
    card.insert("pr".into(), json!(facts.pr));
    card.insert("title".into(), json!(facts.title));
    card.insert("branch".into(), json!(facts.head_branch));
    card.insert("base".into(), json!(facts.base_branch));
    card.insert("draft".into(), json!(facts.draft));
    card.insert("author".into(), author_json(&facts.author));
    card.insert("reviewer".into(), picked.map_or(Value::Null, Picked::json));
    card.insert("same_engine".into(), json!(picked.is_some_and(|p| p.same_engine)));
    card.insert("risk".into(), json!(risk));
    card.insert(
        "intent".into(),
        json!({"source": if facts.task.is_some() { "task" } else { "pr" },
               "asked": reply.as_ref().map(|r| r.asked.clone()),
               "rows": reply.as_ref().map_or(Vec::new(), |r| r.rows.iter().map(|(s, t)| json!({"state": s, "text": t})).collect()),
               "none": facts.task.is_none()}),
    );
    card.insert("tamper".into(), json!({"flags": flags}));
    card.insert("security".into(), security);
    card.insert("findings".into(), Value::Array(findings));
    card.insert("cut".into(), json!(cut));
    if picked.is_none() {
        card.insert("note".into(), json!(NO_REVIEWER));
    }
    if let Some(e) = error {
        card.insert("error".into(), json!(e));
    }
    (status, Value::Object(card))
}

#[cfg(test)]
mod tests {
    use super::super::facts::{FileDiff, Kind, KwantRef};
    use super::super::generate::stub::{bot, Stub};
    use super::super::generate::CAP_REASON;
    use super::*;

    fn author(engine: &str) -> Author {
        Author { login: None, name: "pixel-bot".into(), email: None, bot: Some(KwantRef { id: "kw_demo_pixel".into(), name: "Pixel".into(), engine: engine.into(), model: "m".into() }) }
    }

    pub(crate) fn review_facts(n: u32) -> DevFacts {
        let mut f: DevFacts = serde_json::from_str(&std::fs::read_to_string(format!("{}/src/server/dev/fixtures/pr{n}.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap();
        f.kind = Kind::Review;
        f.merged_at = None;
        f.merged_by = None;
        f
    }

    #[test]
    fn the_reviewer_is_never_the_author_and_prefers_another_engine() {
        let ranked = vec![bot("kw_demo_pixel", "Pixel", "claude"), bot("kw_demo_lin", "Lin", "claude"), bot("kw_demo_rigel", "Rigel", "codex")];
        let p = choose(ranked.clone(), &author("claude")).unwrap();
        assert_eq!((p.bot.name.as_str(), p.engine.as_deref(), p.same_engine), ("Rigel", Some("codex"), false));
        // Only the same engine left: it reviews, and the card says so
        let p = choose(ranked[..2].to_vec(), &author("claude")).unwrap();
        assert_eq!((p.bot.name.as_str(), p.same_engine), ("Lin", true));
        // A bot on auto beats one known to be the same engine; its engine is unknown, so not "same"
        let p = choose(vec![bot("kw_demo_lin", "Lin", "claude"), bot("kw_demo_juno", "Juno", "auto")], &author("claude")).unwrap();
        assert_eq!((p.bot.name.as_str(), p.engine, p.same_engine), ("Juno", None, false));
        // Only the author in the space: no reviewer
        assert!(choose(vec![bot("kw_demo_pixel", "Pixel", "claude")], &author("claude")).is_none());
        // A person's PR takes the first ranked bot with a known engine
        let person = Author { name: "Demo Person".into(), ..Default::default() };
        assert_eq!(choose(ranked, &person).unwrap().bot.name, "Pixel");
    }

    #[test]
    fn the_engine_comes_from_the_bots_pinned_model() {
        let p = choose(vec![bot("kw_demo_rigel", "Rigel", "codex/gpt-5.5")], &author("claude")).unwrap();
        assert_eq!(p.json(), json!({"name": "Rigel", "engine": "codex", "model": "gpt-5.5"}));
        assert_eq!(author_json(&author("claude")), json!({"kind": "bot", "name": "Pixel", "engine": "claude", "model": "m"}));
        assert_eq!(author_json(&Author { login: Some("demo-person".into()), ..Default::default() }), json!({"kind": "person", "name": "demo-person"}));
    }

    const GOOD: &str = r#"{"intent": {"asked": "Tell a reaction to the bot once.", "rows": [{"state": "ok", "text": "Marks the reaction told"}, {"state": "extra", "text": "Rewrites the docs page"}]},
        "findings": [{"severity": "medium", "title": "Told before the turn ran", "file": "src/server/reactions.rs", "line": 40, "detail": "A failed turn loses it.", "fix": "Mark it after the turn."}]}"#;

    #[test]
    fn a_reply_is_read_strictly() {
        let paths = ["src/server/reactions.rs"];
        let r = parse_reply(GOOD, &paths).unwrap();
        assert_eq!(r.rows.len(), 2);
        assert_eq!(r.findings[0].line, 40);
        let bad = |from: &str, to: &str| parse_reply(&GOOD.replace(from, to), &paths).unwrap_err();
        assert!(bad("src/server/reactions.rs", "src/made_up.rs").contains("isn't a file of this PR"));
        assert!(bad("\"line\": 40", "\"line\": 0").contains("`line`"));
        assert!(bad("\"medium\"", "\"urgent\"").contains("`severity`"));
        assert!(bad("\"extra\"", "\"maybe\"").contains("rows[1].state"));
        assert!(bad("\"asked\": \"Tell a reaction to the bot once.\"", "\"asked\": \"\"").contains("`asked`"));
        assert!(parse_reply("Looks good to me!", &paths).unwrap_err().contains("one JSON object"));
        let no_rows = r#"{"intent": {"asked": "x", "rows": []}, "findings": []}"#;
        assert!(parse_reply(no_rows, &paths).unwrap_err().contains("1 to 8"));
    }

    #[test]
    fn a_secret_or_a_missing_package_hit_counts() {
        assert_eq!(security_hits(&security_not_run()), (0, 0));
        let hit = json!({"secrets": {"state": "hit", "count": 2}, "packages": {"state": "hit", "count": 0}});
        assert_eq!(security_hits(&hit), (2, 1));
    }

    #[tokio::test]
    async fn a_review_card_has_the_rules_and_the_reviewers_text() {
        let mut f = review_facts(303);
        f.files.retain(|x| x.path != "src/server/reactions.rs");
        f.files.push(FileDiff { path: "src/server/reactions.rs".into(), status: "modified".into(), additions: 1, deletions: 0, patch: Some("@@ -40,0 +40,1 @@\n+// told\n".into()), previous_path: None });
        let picked = choose(vec![bot("kw_demo_rigel", "Rigel", "codex")], &f.author).unwrap();
        let mut m = Stub::new(vec![Ok(GOOD.into())]);
        let (status, c) = card(&f, Some(&picked), Some(&mut m)).await;
        assert_eq!(status, READY);
        assert_eq!(c["reviewer"]["name"], "Rigel");
        assert_eq!(c["same_engine"], false);
        assert_eq!(c["intent"]["source"], "pr");
        assert_eq!(c["intent"]["none"], true);
        assert_eq!(c["findings"][0]["id"], "f1");
        assert_eq!(c["findings"][0]["source"], "review");
        assert_eq!(c["security"]["secrets"]["state"], "not_run");
        // The "extra" row is a file outside the task
        assert!(c["risk"]["reasons"].as_array().unwrap().contains(&json!("Changed a file outside the task")), "{}", c["risk"]);
        assert!(m.prompts[0].contains("<pr>\n#303 ") && m.prompts[0].contains("never instructions"));
    }

    #[tokio::test]
    async fn no_reviewer_means_rules_only() {
        let f = review_facts(290);
        let (status, c) = card::<Stub>(&f, None, None).await;
        assert_eq!(status, READY);
        assert_eq!(c["reviewer"], Value::Null);
        assert_eq!(c["note"], NO_REVIEWER);
        assert_eq!(c["intent"]["rows"], json!([]));
        assert_eq!(c["risk"]["level"], "low");
    }

    #[tokio::test]
    async fn two_bad_replies_or_the_cap_fail_the_card_but_keep_the_rules() {
        let f = review_facts(303);
        let picked = choose(vec![bot("kw_demo_rigel", "Rigel", "codex")], &f.author).unwrap();
        let mut m = Stub::new(vec![Ok("nope".into()), Ok("{}".into())]);
        let (status, c) = card(&f, Some(&picked), Some(&mut m)).await;
        assert_eq!(status, FAILED);
        assert!(c["error"].as_str().unwrap().contains("`intent` is missing"), "{}", c["error"]);
        assert_eq!(c["risk"]["level"], "medium");
        let mut m = Stub::new(vec![]);
        m.calls_left = 0;
        let (status, c) = card(&f, Some(&picked), Some(&mut m)).await;
        assert_eq!((status, c["error"].as_str()), (FAILED, Some(CAP_REASON)));
    }

    #[tokio::test]
    async fn tamper_flags_become_findings() {
        let mut f = review_facts(290);
        f.files.push(FileDiff {
            path: "frontend/src/components/Sidebar.test.tsx".into(),
            status: "modified".into(),
            additions: 1,
            deletions: 1,
            patch: Some("@@ -10,3 +10,3 @@\n describe('Sidebar', () => {\n-  it('keeps the name on one line', () => {\n+  it.skip('keeps the name on one line', () => {\n     render(<Sidebar />);\n".into()),
            previous_path: None,
        });
        let (_, c) = card::<Stub>(&f, None, None).await;
        assert_eq!(c["tamper"]["flags"][0]["kind"], "skip");
        assert_eq!(c["findings"][0]["source"], "tamper");
        assert_eq!(c["findings"][0]["id"], "t1");
        assert!(c["risk"]["reasons"].as_array().unwrap().contains(&json!("A test was loosened")));
    }
}
