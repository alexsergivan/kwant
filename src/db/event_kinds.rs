//! The one list of task event kinds the server writes to `task_events` (roadmap 17d).
//!
//! Activity words each kind in plain language (`eventText` in frontend/src/missions.ts). This list
//! reaches the frontend as frontend/src/generated/taskEventKinds.json, and its guard test
//! (frontend/tests/auditLineEverywhere.test.mjs) fails when a kind here has no wording. Two checks
//! keep the list complete: tests/task_event_kinds_test.rs scans the emitters for the kinds they
//! write, and the writes themselves (`log_task_event`, `insert_event`) `debug_assert!` that a kind
//! built at run time is listed.
//!
//! Adding a kind: add it here (sorted), then regenerate the JSON with
//! `KWANT_WRITE_TASK_EVENT_KINDS=1 cargo test --test task_event_kinds_test`.

/// Every `task_events.kind` the server writes, sorted byte-wise, no duplicates.
pub const TASK_EVENT_KINDS: &[&str] = &[
    "audit_challenge", "audit_done", "audit_fresh_eyes", "audit_rebuttal", "audit_skipped", "audit_started",
    "autofix", "autofix_finished", "autofix_held", "autofix_scheduled", "autofix_started", "budget", "check_added",
    "check_result", "check_wait_more", "checkpoint", "checkpoint_failed", "checkpoint_pruned", "claimed",
    "class_changed", "contract_updated", "created", "edited", "evidence", "failure", "flag_resolved", "flagged",
    "forked", "handoff_ended", "held", "lead_review_asked", "lease_expired", "lessons_held_out", "lessons_shown",
    "note", "released", "restored", "review", "risk_resolved", "self_checked", "skill_drafted_from", "skipped",
    "spec", "state", "tier_stepped_down", "verdict_overturned", "verifier_done", "verifier_skipped",
    "verifier_started", "verifier_turn", "watch", "workflow_change_applied", "workflow_change_decided",
    "workflow_change_proposed",
];

/// Whether the server writes `kind` as a task event kind.
pub fn is_task_event_kind(kind: &str) -> bool {
    TASK_EVENT_KINDS.binary_search(&kind).is_ok()
}
