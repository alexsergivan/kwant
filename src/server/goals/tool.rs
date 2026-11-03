//! `goal_propose`: a bot drafts a goal when the person asks for one. The draft does nothing until
//! the person approves it on the board; at most [`MAX_DRAFTS`] wait per space, and one nobody
//! approves is deleted silently after a week. A bot can't make a goal fix things on its own: a
//! proposed `fix` becomes `suggest`, and the person raises it.

use super::{build_goal, emit_goal, predicate, CheckInput, GoalInput};
use crate::db::goals::{Created, MAX_DRAFTS};
use crate::db::missions::now_ms;
use crate::server::continuity::SelfScope;
use crate::server::AppState;
use serde_json::{json, Value};

pub const TOOL: &str = "goal_propose";

/// The `metric` and `operator` of the schema, from what predicate validation accepts, so the two
/// can't drift apart.
fn metric_help() -> String {
    format!(
        "http: {}; command: {}. Leave out for the usual (http status 200, command exit code 0). Say what the target is compared with in `target`.",
        predicate::metrics("http").join(", "),
        predicate::metrics("command").join(", ")
    )
}

pub fn tool() -> Value {
    let metrics: Vec<&str> = ["http", "command"]
        .iter()
        .flat_map(|k| predicate::metrics(k).iter().copied())
        .collect();
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
                            "spec": {"type": "object", "description": "command: {command, timeout_secs?}; http: {url}, and for the json_value metric also {pointer}, a JSON pointer such as /status or /items/0/name (empty for the whole document); file_exists: {path}; file_contains: {path, text}; file_sha256: {path, sha256}. Paths are relative to your folder."},
                            "metric": {"type": "string", "enum": metrics, "description": metric_help()},
                            "operator": {"type": "string", "enum": predicate::ALL_OPERATORS, "description": "How the metric is compared with the target. status and exit_code take eq, ne; latency_ms lt, lte; stdout_number and json_value eq, ne, lt, lte, gt, gte; json_value also contains (text in a string, or an item of an array). body_contains and stdout_contains take no operator."},
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

#[cfg(test)]
mod tests {
    use super::*;
    use predicate::normalize;

    fn schema_list(path: &[&str]) -> Vec<String> {
        let mut v = tool();
        for k in ["inputSchema", "properties", "checks", "items", "properties"]
            .iter()
            .chain(path)
        {
            v = v[*k].clone();
        }
        v["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    }

    /// A target every metric takes, whatever its type.
    fn target(metric: &str, op: Option<&str>) -> &'static str {
        match (metric, op) {
            ("status", _) => "200",
            ("exit_code", _) => "0",
            ("latency_ms", _) => "500",
            ("stdout_number", _) => "5",
            ("json_value", Some("lt" | "lte" | "gt" | "gte")) => "5",
            _ => "ok",
        }
    }

    fn accepts(kind: &str, metric: &str, op: Option<&str>) -> bool {
        normalize(
            kind,
            &json!({"url": "https://example.com/health", "pointer": "/status"}),
            Some(metric),
            op,
            Some(target(metric, op)),
        )
        .is_ok()
    }

    const KINDS: [&str; 2] = ["http", "command"];
    /// Names a bot might try, valid or not (`json_path` is the one the schema once advertised).
    const TRIED_METRICS: [&str; 10] = [
        "status",
        "latency_ms",
        "body_contains",
        "json_value",
        "json_path",
        "exit_code",
        "stdout_contains",
        "stdout_number",
        "body_matches",
        "stdout",
    ];
    const TRIED_OPERATORS: [&str; 11] = [
        "eq", "ne", "lt", "lte", "gt", "gte", "contains", "like", "matches", "in", "between",
    ];

    #[test]
    fn every_metric_the_schema_lists_is_accepted() {
        let listed = schema_list(&["metric"]);
        assert!(!listed.is_empty());
        for m in &listed {
            assert!(
                KINDS.iter().any(|k| accepts(k, m, None)),
                "the schema lists metric {m}, which validation refuses"
            );
        }
    }

    #[test]
    fn every_metric_validation_accepts_is_listed() {
        let listed = schema_list(&["metric"]);
        for kind in KINDS {
            for m in TRIED_METRICS {
                if accepts(kind, m, None) {
                    assert!(
                        listed.iter().any(|l| l == m),
                        "{kind} accepts metric {m}, which the schema doesn't list"
                    );
                }
            }
            // And what the kind says it offers
            for m in predicate::metrics(kind) {
                assert!(
                    listed.iter().any(|l| l == m),
                    "{kind} offers {m}, which the schema doesn't list"
                );
            }
        }
        assert!(
            !listed.iter().any(|l| l == "json_path"),
            "json_path isn't a metric"
        );
    }

    #[test]
    fn every_operator_the_schema_lists_is_accepted() {
        let listed = schema_list(&["operator"]);
        for op in &listed {
            let some_metric_takes_it = KINDS.iter().any(|k| {
                predicate::metrics(k)
                    .iter()
                    .any(|m| accepts(k, m, Some(op)))
            });
            assert!(
                some_metric_takes_it,
                "the schema lists operator {op}, which no metric accepts"
            );
        }
    }

    #[test]
    fn every_operator_validation_accepts_is_listed() {
        let listed = schema_list(&["operator"]);
        for kind in KINDS {
            for m in predicate::metrics(kind) {
                for op in TRIED_OPERATORS {
                    if accepts(kind, m, Some(op)) {
                        assert!(
                            listed.iter().any(|l| l == op),
                            "{m} accepts operator {op}, which the schema doesn't list"
                        );
                    }
                }
            }
        }
        // contains is the one the schema lacked
        assert!(listed.iter().any(|l| l == "contains"));
    }

    #[test]
    fn a_json_check_written_from_the_schema_is_accepted() {
        // The spec description says json_value needs a pointer
        let v = tool();
        let spec = v["inputSchema"]["properties"]["checks"]["items"]["properties"]["spec"]
            ["description"]
            .as_str()
            .unwrap();
        assert!(
            spec.contains("json_value") && spec.contains("pointer"),
            "{spec}"
        );
        let spec = json!({"url": "https://example.com/health", "pointer": "/status"});
        for (op, t) in [
            ("eq", "ok"),
            ("ne", "down"),
            ("contains", "ok"),
            ("gte", "3"),
        ] {
            let p = normalize("http", &spec, Some("json_value"), Some(op), Some(t))
                .unwrap()
                .unwrap();
            assert_eq!(
                (p.metric.as_str(), p.operator.as_deref()),
                ("json_value", Some(op))
            );
        }
        // A bad pointer is refused in words
        assert!(normalize(
            "http",
            &json!({"url": "https://x.test", "pointer": "status"}),
            Some("json_value"),
            Some("eq"),
            Some("ok")
        )
        .is_err());
    }
}
