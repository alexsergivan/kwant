//! `goal_propose`: a bot drafts a goal when the person asks for one. The draft does nothing until
//! the person approves it on the board; at most [`MAX_DRAFTS`] wait per space, and one nobody
//! approves is deleted silently after a week. A bot can't make a goal fix things on its own: a
//! proposed `fix` becomes `suggest`, and the person raises it.

use super::{build_goal, emit_goal, CheckInput, GoalInput};
use crate::db::goals::{Created, MAX_DRAFTS};
use crate::db::missions::now_ms;
use crate::server::continuity::SelfScope;
use crate::server::AppState;
use serde_json::{json, Value};

pub const TOOL: &str = "goal_propose";

pub fn tool() -> Value {
    json!({
        "name": TOOL,
        "description": "Propose a goal: something that should stay true (a site answers, a file exists, a number stays under a limit), checked on a schedule. It is a draft until the person approves it on the board; nothing is checked before that. Use it only when the person asks you to watch, keep an eye on or make sure of something, never on your own.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": {"type": "string", "description": "What should stay true, in the person's words."},
                "description": {"type": "string"},
                "interval_minutes": {"type": "integer", "minimum": super::INTERVAL_RANGE.0, "maximum": super::INTERVAL_RANGE.1},
                "mode": {"type": "string", "enum": ["watch", "suggest"], "description": "watch: only tell the person; suggest (default): also offer a fix for them to start."},
                "checks": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": super::MAX_CHECKS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": {"type": "string"},
                            "kind": {"type": "string", "enum": ["command", "http", "file_exists", "file_contains", "file_sha256"]},
                            "spec": {"type": "object", "description": "command: {command, timeout_secs?}; http: {url}; file_exists: {path}; file_contains: {path, text}; file_sha256: {path, sha256}. Paths are relative to your folder."},
                            "metric": {"type": "string", "description": "http: status, latency_ms, body_contains, json_path; command: exit_code, stdout_contains, stdout_number. Leave out for the usual (status 200, exit code 0)."},
                            "operator": {"type": "string", "enum": ["eq", "ne", "lt", "lte", "gt", "gte"]},
                            "target": {"description": "What the metric is compared with: a number or text."},
                            "severity": {"type": "string", "enum": ["required", "advisory"], "description": "Only required checks decide the goal."},
                        },
                        "required": ["kind", "spec"],
                    },
                },
            },
            "required": ["title", "checks"],
        },
    })
}

pub async fn call(state: &AppState, scope: &SelfScope, args: &Value) -> (String, bool) {
    if !crate::server::missions::enabled(state).await {
        return ("The task board is off in Settings.".into(), true);
    }
    let space = match state.db.space_of_kwant(&scope.kwant_id).await {
        Ok(Some(s)) => s,
        _ => return ("Could not find your space.".into(), true),
    };
    let checks: Vec<CheckInput> = match serde_json::from_value(args["checks"].clone()) {
        Ok(c) => c,
        Err(e) => return (format!("checks: {e}"), true),
    };
    let mode = match args["mode"].as_str().map(str::trim) {
        Some("fix") | None => "suggest",
        Some(m) => m,
    };
    let input = GoalInput {
        title: args["title"].as_str().unwrap_or("").into(),
        description: args["description"].as_str().unwrap_or("").into(),
        kwant_id: Some(scope.kwant_id.clone()),
        mode: Some(mode.into()),
        interval_minutes: args["interval_minutes"].as_i64(),
        confirm_failures: None,
        budget_tokens: None,
        max_fixes_per_day: None,
        priority: None,
        checks,
        from_task: None,
    };
    let new = match build_goal(state, &space, &input, &scope.kwant_id, true).await {
        Ok(n) => n,
        Err(e) => return (e, true),
    };
    match state.db.create_goal(&new, now_ms()).await {
        Ok(Created::Goal(g)) => {
            emit_goal(state, &g);
            let out = json!({
                "goal_id": g.id,
                "status": "draft",
                "note": "Proposed. The person approves it on the Mission board; nothing is checked before that.",
            });
            (serde_json::to_string_pretty(&out).unwrap_or_default(), false)
        }
        Ok(Created::TooManyDrafts) => (
            format!("There are already {MAX_DRAFTS} goals waiting for the person's approval in this space. Ask them to approve or delete one first."),
            true,
        ),
        Err(e) => {
            tracing::warn!(error = %e, "goal_propose");
            ("Could not save the goal.".into(), true)
        }
    }
}
