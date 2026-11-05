//! The landed card (plan §3.4 `landed`, design §4): one merge to the watched branch, told in plain
//! words. Rules give the risk, the tests line and `verified`; one turn gives the area, headline,
//! "what changes for you" and "under the hood".

use super::facts::{CiState, DevFacts};
use super::generate::{ask_json, cut_of, diff_text, json_object, text_field, Model, DATA_RULES};
use super::review::{author_json, one_line};
use super::risk::{self, Inputs};
use super::tamper;
use crate::db::dev_mode::{FAILED, READY};
use serde_json::{json, Map, Value};

pub const AREA_CHARS: usize = 24;
pub const HEADLINE_CHARS: usize = 90;
const BODY_CHARS: usize = 4_000;

#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub area: String,
    pub headline: String,
    pub for_you: String,
    pub under_hood: String,
}

/// Reads the writer's reply strictly: four non-empty strings, the area and headline on one line.
pub fn parse_reply(reply: &str) -> Result<Text, String> {
    let m = json_object(reply)?;
    let area = text_field(&m, "area", AREA_CHARS)?;
    let headline = text_field(&m, "headline", HEADLINE_CHARS)?;
    for (k, v) in [("area", &area), ("headline", &headline)] {
        if v.contains('\n') {
            return Err(format!("`{k}` must be one line"));
        }
    }
    let under = match m.get("under_hood") {
        // `{"summary": …}` as the card has it is fine too
        Some(Value::Object(o)) => text_field(o, "summary", 800)?,
        _ => text_field(&m, "under_hood", 800)?,
    };
    Ok(Text { area: one_line(&area), headline: one_line(&headline), for_you: text_field(&m, "for_you", 600)?, under_hood: under })
}

/// The tests line from CI (the poller has check runs, not test counts).
pub fn tests_json(facts: &DevFacts) -> Value {
    let n = facts.checks.len();
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let (state, summary) = match facts.ci {
        CiState::Green => ("passed", format!("{n} check{} passed", plural(n))),
        CiState::Red => {
            let bad: Vec<&str> = facts
                .checks
                .iter()
                .filter(|c| matches!(c.conclusion.as_deref(), Some("failure" | "timed_out" | "cancelled" | "action_required" | "startup_failure")))
                .map(|c| c.name.as_str())
                .collect();
            ("failed", format!("{} of {n} check{} failed: {}", bad.len(), plural(n), bad.join(", ")))
        }
        CiState::Pending => ("unknown", "Checks still running".to_string()),
        CiState::None => ("none", "No CI checks".to_string()),
    };
    json!({"state": state, "summary": summary})
}

/// `reviewed_by` and the open follow-ups, from the newest ready review card of the same PR.
pub fn from_review(review: Option<&Value>, pr: Option<u64>) -> (Option<String>, Vec<Value>) {
    let Some(r) = review else { return (None, Vec::new()) };
    let by = r["reviewer"]["name"].as_str().map(str::to_string);
    let follow = r["findings"]
        .as_array()
        .map(|fs| {
            fs.iter()
                .filter(|f| f["state"] == "open")
                .map(|f| json!({"text": f["title"], "ref": format!("#{} {}", pr.map_or("?".into(), |p| p.to_string()), f["id"].as_str().unwrap_or("?"))}))
                .collect()
        })
        .unwrap_or_default();
    (by, follow)
}

/// Verified: CI green, reviewed by someone other than the author, and nothing left open.
pub fn verified(ci: CiState, reviewed_by: Option<&str>, author: &str, follow_ups: &[Value]) -> bool {
    ci == CiState::Green && reviewed_by.is_some_and(|r| !r.eq_ignore_ascii_case(author)) && follow_ups.is_empty()
}

fn prompt(facts: &DevFacts, writer: &str) -> (String, Option<String>) {
    let (diff, left_out) = diff_text(&facts.files);
    let cut = cut_of(facts, left_out);
    let body: String = facts.body.chars().take(BODY_CHARS).collect();
    let p = format!(
        "You are {writer}. A change just landed on {base} in {repo}. Tell the person who runs this app what it means for them, \
in plain words, as a short card. No PR numbers, branch names or code jargon in the headline or \"for_you\".\n\n\
<pr>\n{pr}{title}\n\n{body}\n\nThe diff{cut_note}:\n{diff}</pr>\n\n\
Reply with this JSON object:\n\
{{\"area\": \"the part of the product it touches, 1 to 3 words (Sidebar, Chat, Release, Dev mode)\", \
\"headline\": \"what changes, at most {HEADLINE_CHARS} characters\", \
\"for_you\": \"1 to 3 sentences on what the person notices; if nothing, say so\", \
\"under_hood\": \"1 to 3 sentences on what changed in the code\"}}\n\n{DATA_RULES}",
        base = facts.base_branch,
        repo = facts.repo,
        pr = facts.pr.map(|p| format!("#{p} ")).unwrap_or_default(),
        title = facts.title,
        cut_note = cut.as_ref().map(|c| format!(" ({c})")).unwrap_or_default(),
    );
    (p, cut)
}

/// Builds the landed card. With no bot to write the text it is `ready` with the PR title as the
/// headline; when the text fails twice or the cap is used up it is `failed` with the same
/// fallback and `error`.
pub(super) async fn card<M: Model>(facts: &DevFacts, review: Option<&Value>, model: Option<&mut M>) -> (&'static str, Value) {
    let flags = tamper::check(&facts.files);
    let risk = risk::assess(&Inputs { files: &facts.files, first_edit_files: &facts.first_edit_files, tamper: &flags, ..Default::default() });
    let (reviewed_by, follow_ups) = from_review(review, facts.pr);
    let author = author_json(&facts.author);
    let author_name = author["name"].as_str().unwrap_or("").to_string();
    let verified = verified(facts.ci, reviewed_by.as_deref(), &author_name, &follow_ups);

    let (mut status, mut error, mut text, mut cut) = (READY, None, None, facts.cut.clone());
    if let Some(model) = model {
        let (p, c) = prompt(facts, &model.bot().name.clone());
        cut = c;
        match ask_json(model, &p, parse_reply).await {
            Ok(t) => text = Some(t),
            Err(e) => (status, error) = (FAILED, Some(e)),
        }
    }
    let files: Vec<Value> = facts.files.iter().map(|f| json!({"path": f.path, "add": f.additions, "del": f.deletions})).collect();
    let mut card = Map::new();
    card.insert("kind".into(), json!("landed"));
    card.insert("pr".into(), json!(facts.pr));
    card.insert("sha".into(), json!(facts.head_sha.chars().take(7).collect::<String>()));
    card.insert("merged_at".into(), json!(facts.merged_at));
    card.insert("merged_by".into(), json!(facts.merged_by));
    card.insert("written_by".into(), author);
    card.insert("area".into(), json!(text.as_ref().map(|t| t.area.clone())));
    card.insert("headline".into(), json!(text.as_ref().map_or_else(|| one_line(&facts.title), |t| t.headline.clone())));
    card.insert("for_you".into(), json!(text.as_ref().map(|t| t.for_you.clone())));
    card.insert("under_hood".into(), json!({"summary": text.as_ref().map(|t| t.under_hood.clone()), "files": files}));
    card.insert("risk".into(), json!(risk));
    card.insert("tests".into(), tests_json(facts));
    card.insert("reviewed_by".into(), json!(reviewed_by));
    card.insert("follow_ups".into(), Value::Array(follow_ups));
    card.insert("verified".into(), json!(verified));
    card.insert("links".into(), json!(facts.links));
    card.insert("cut".into(), json!(cut));
    if let Some(e) = error {
        card.insert("error".into(), json!(e));
    }
    (status, Value::Object(card))
}

#[cfg(test)]
mod tests {
    use super::super::facts::Check;
    use super::super::generate::stub::Stub;
    use super::*;

    pub(crate) fn fixture(n: u32) -> DevFacts {
        serde_json::from_str(&std::fs::read_to_string(format!("{}/src/server/dev/fixtures/pr{n}.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap()
    }

    #[test]
    fn the_reply_is_read_strictly() {
        let ok = r#"{"area": "Sidebar", "headline": "Rows stop jumping", "for_you": "Names stay put.", "under_hood": "CSS only."}"#;
        assert_eq!(parse_reply(ok).unwrap().area, "Sidebar");
        let nested = r#"{"area": "Sidebar", "headline": "x", "for_you": "y", "under_hood": {"summary": "z"}}"#;
        assert_eq!(parse_reply(nested).unwrap().under_hood, "z");
        assert!(parse_reply(&ok.replace("Sidebar", "The whole left-hand sidebar area")).unwrap_err().contains("`area` is longer"));
        assert!(parse_reply(&ok.replace("Rows stop jumping", &"a".repeat(91))).unwrap_err().contains("`headline` is longer than 90"));
        assert!(parse_reply(&ok.replace("Rows stop", "Rows\\nstop")).unwrap_err().contains("one line"));
        assert!(parse_reply(r#"{"area": "Sidebar", "headline": "x", "under_hood": "z"}"#).unwrap_err().contains("`for_you`"));
    }

    #[test]
    fn verified_needs_green_ci_another_reviewer_and_nothing_open() {
        let open = [json!({"text": "N3", "ref": "#288 f1"})];
        assert!(verified(CiState::Green, Some("Rigel"), "Pixel", &[]));
        assert!(!verified(CiState::Green, None, "Pixel", &[]));
        assert!(!verified(CiState::Green, Some("Pixel"), "Pixel", &[]));
        assert!(!verified(CiState::Green, Some("Rigel"), "Pixel", &open));
        assert!(!verified(CiState::Pending, Some("Rigel"), "Pixel", &[]));
        assert!(!verified(CiState::Red, Some("Rigel"), "Pixel", &[]));
    }

    #[test]
    fn follow_ups_are_the_reviews_open_findings() {
        let review = json!({"reviewer": {"name": "Rigel"}, "findings": [
            {"id": "f1", "title": "Told too early", "state": "open"},
            {"id": "f2", "title": "Typo", "state": "dismissed"},
            {"id": "t1", "title": "A test was switched off", "state": "sent"}]});
        let (by, f) = from_review(Some(&review), Some(303));
        assert_eq!(by.as_deref(), Some("Rigel"));
        assert_eq!(f, vec![json!({"text": "Told too early", "ref": "#303 f1"})]);
        assert_eq!(from_review(None, Some(303)), (None, vec![]));
    }

    #[test]
    fn the_tests_line_follows_ci() {
        let mut f = fixture(290);
        assert_eq!(tests_json(&f), json!({"state": "passed", "summary": "3 checks passed"}));
        f.checks[1] = Check { name: "frontend".into(), status: "completed".into(), conclusion: Some("failure".into()), url: None };
        f.ci = CiState::of(&f.checks);
        assert_eq!(tests_json(&f), json!({"state": "failed", "summary": "1 of 3 checks failed: frontend"}));
        f.checks.clear();
        f.ci = CiState::None;
        assert_eq!(tests_json(&f)["state"], "none");
    }

    #[tokio::test]
    async fn a_landed_card_with_text() {
        let f = fixture(290);
        let mut m = Stub::new(vec![Ok(r#"{"area": "Sidebar", "headline": "Sidebar rows stop jumping on hover", "for_you": "Hovering a bot keeps its name on one line.", "under_hood": "Only the sidebar's CSS changed."}"#.into())]);
        let review = json!({"reviewer": {"name": "Rigel"}, "findings": []});
        let (status, c) = card(&f, Some(&review), Some(&mut m)).await;
        assert_eq!(status, READY);
        assert_eq!(c["headline"], "Sidebar rows stop jumping on hover");
        assert_eq!(c["sha"], f.head_sha[..7]);
        assert_eq!(c["verified"], true);
        assert_eq!(c["written_by"]["kind"], "bot");
        assert_eq!(c["under_hood"]["files"].as_array().unwrap().len(), 2);
        assert_eq!(c["links"]["pr"], "https://github.com/alexsergivan/kwant/pull/290");
    }

    #[tokio::test]
    async fn without_text_the_title_stands_in() {
        let f = fixture(301);
        let (status, c) = card::<Stub>(&f, None, None).await;
        assert_eq!(status, READY);
        assert_eq!(c["headline"], f.title);
        assert_eq!(c["area"], Value::Null);
        // Not reviewed: never verified
        assert_eq!(c["verified"], false);
        let mut m = Stub::new(vec![Ok("{}".into()), Ok("{}".into())]);
        let (status, c) = card(&f, None, Some(&mut m)).await;
        assert_eq!(status, FAILED);
        assert!(c["error"].as_str().unwrap().contains("`area` is missing"));
    }
}
