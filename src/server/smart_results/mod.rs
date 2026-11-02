//! Smart Results (0.6.5): a table or a numbered list in a bot's final reply, kept as a typed
//! block the chat can draw as a comparison card or a checklist. Contract: docs/smart-results.md.
//!
//! - The reply's text is never changed. The block set lives beside it in `message_results`,
//!   written when the reply is saved ([`crate::db::Database::insert_message_with`]), and every
//!   message object carries it as `results` (`null` when there is none).
//! - It always runs; there is no setting. The release fallback is [`ENABLED_TYPES`].
//! - The selectors are deterministic (no model calls) and conservative: when one isn't sure the
//!   chat would render the same table or list, it abstains and the reply stays markdown.
//! - Nothing here may cost the reply: a panic, an invalid block or a failed write is logged and
//!   the message is saved as it always was.

mod selector;
mod validate;

pub use selector::candidates;
pub use validate::validate;

use serde::{Deserialize, Serialize};

/// The block types this build stores and serves. Taking a type out (a one-line release fallback)
/// stops new blocks of it and hides stored ones (`results: null`); the chat then shows the
/// reply's markdown.
pub const ENABLED_TYPES: &[&str] = &["comparison", "steps"];
/// The payload version this build writes and reads.
pub const VERSION: u32 = 1;
/// Largest stored payload, in bytes of JSON.
pub const MAX_PAYLOAD: usize = 64 * 1024;
/// Replies longer than this stay text.
pub const MAX_REPLY: usize = 128 * 1024;
/// Why a row is highlighted; the only reason v1 gives.
pub const PICK_REASON: &str = "named as the pick in this reply";

/// The checked block set for a reply: the first candidate, in priority order, whose type is
/// enabled and that passes [`validate`]; `None` when there isn't one. A selector bug must never
/// cost the reply, so a panic is caught and logged.
pub fn compute(text: &str) -> Option<ResultSet> {
    let run = std::panic::catch_unwind(|| {
        for block in candidates(text) {
            if !ENABLED_TYPES.contains(&block.kind()) {
                continue;
            }
            let set = ResultSet { version: VERSION, blocks: vec![block] };
            match validate(&set, Some(text)) {
                Ok(()) => return Some(set),
                Err(why) => tracing::warn!("smart results: dropped a block that failed its checks: {why}"),
            }
        }
        None
    });
    run.unwrap_or_else(|_| {
        tracing::warn!("smart results: a selector panicked; the reply stays text");
        None
    })
}

/// `results` on a message: payload v1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResultSet {
    pub version: u32,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Comparison(Comparison),
    Steps(Steps),
}

impl Block {
    /// The block's `type`.
    pub fn kind(&self) -> &'static str {
        match self {
            Block::Comparison(_) => "comparison",
            Block::Steps(_) => "steps",
        }
    }
}

/// Which table (`markdown_table`) or ordered list (`markdown_list`) of the reply a block stands
/// for: `index` counts them in the order the chat renders them, from 0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub kind: String,
    pub index: usize,
}

/// Where the values came from; v1 only copies them from the reply (`reply_text`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub key: String,
    pub label: String,
    pub numeric: bool,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub id: String,
    pub cells: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    pub id: String,
    pub source: Source,
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
    pub highlight_row: Option<String>,
    pub highlight_reason: Option<String>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Steps {
    pub id: String,
    pub source: Source,
    pub items: Vec<Step>,
    pub provenance: Provenance,
}
