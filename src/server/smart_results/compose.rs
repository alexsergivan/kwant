//! Orchestrates one message's block set from every candidate source, in priority order (plan.md
//! §A.2). A bot reply: a grounded `present_result` call first (the bot's own declared pick), then
//! the deterministic reply-text selectors. A task report card (`meta.kind` "mission" with a
//! `task_id`) gets its `verification` block, an agent's shared-file card its `files` block, and no
//! other card gets anything; each block must be enabled and pass [`super::validate`].

use std::path::Path;

use sqlx::SqliteConnection;

use crate::db::models::{MessageExtra, NewMessage};

use super::{files, present_result, verification, Block, ResultSet, ENABLED_TYPES, VERSION};

/// The checked block set for a message being stored on `conn`, or `None` when nothing qualifies.
/// `root` is the attachments directory, for file previews. Never fails: every step is best-effort,
/// and a panic in any one of them is caught and logged, falling through to the next candidate
/// exactly as if that source had found nothing.
pub async fn compute_for_message(
    conn: &mut SqliteConnection,
    input: &NewMessage,
    extra: &MessageExtra,
    root: Option<&Path>,
) -> Option<ResultSet> {
    let kind = extra.meta.as_ref().and_then(|m| m.get("kind")).and_then(|k| k.as_str());
    match kind {
        // A card kwant built from its own data: only its own block, never selectors on its text.
        Some("mission") => return from_block(verification::candidate(&mut *conn, extra).await),
        Some("file") => return from_block(files::candidate(&mut *conn, input, extra, root).await),
        Some(k) if k != "delegation_result" => return None,
        _ => {}
    }
    if let Some(set) = from_block(present_result::grounded_candidate(input)) {
        return Some(set);
    }
    super::compute(&input.text)
}

/// `block`, wrapped as a one-block [`ResultSet`], when its type is enabled and it passes
/// [`super::validate`]. None of these three sources are `reply_text` provenance, so the strict
/// reply-containment check never applies to them; `present_result`'s own grounding check already
/// ran, and `verification`/`files` are kwant's own stored data.
fn from_block(block: Option<Block>) -> Option<ResultSet> {
    let block = block?;
    if !ENABLED_TYPES.contains(&block.kind()) {
        return None;
    }
    let set = ResultSet { version: VERSION, blocks: vec![block] };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| super::validate(&set, None))) {
        Ok(Ok(())) => Some(set),
        Ok(Err(why)) => {
            tracing::warn!("smart results: dropped a block that failed its checks: {why}");
            None
        }
        Err(_) => {
            tracing::warn!("smart results: a candidate panicked; falling through");
            None
        }
    }
}
