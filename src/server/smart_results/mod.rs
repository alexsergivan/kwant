//! Smart Results (0.6.5): a table or a list in a bot's final reply, kept as one typed block the
//! chat can draw as a chart, a comparison, metric tiles, findings, cards, a checklist, a timeline,
//! steps, facts or a recipe. Contract: docs/smart-results.md.
//!
//! - The reply's text is never changed. The block set lives beside it in `message_results`,
//!   written when the reply is saved ([`crate::db::Database::insert_message_with`]), and every
//!   message object carries it as `results` (`null` when there is none).
//! - It always runs; there is no setting. The release fallback is [`ENABLED_TYPES`].
//! - The selectors are deterministic (no model calls) and conservative: when one isn't sure the
//!   chat would render the same table or list, it abstains and the reply stays markdown.
//! - Every value is copied from the reply. The only numbers a block adds are the cells' own
//!   values, parsed (`chart` points, `metrics` items); nothing is summed, averaged or rounded.
//! - Nothing here may cost the reply: a panic, an invalid block or a failed write is logged and
//!   the message is saved as it always was.

mod chart;
mod items;
mod compose;
mod files;
mod present_result;
mod recipe;
mod selector;
mod validate;
mod verification;

pub use compose::compute_for_message;
pub use present_result::{tools as present_result_tools, handles as present_result_handles, call as present_result_call};
pub use selector::{candidates, rank, PRIORITY};
pub use validate::validate;

use serde::{Deserialize, Serialize};

/// The block types this build stores and serves. Taking a type out (a one-line release fallback)
/// stops new blocks of it and hides stored ones (`results: null`); the chat then shows the
/// reply's markdown.
pub const ENABLED_TYPES: &[&str] = &["chart", "comparison", "metrics", "findings", "cards", "steps", "facts", "verification", "files", "recipe"];
/// The payload version this build writes and reads.
pub const VERSION: u32 = 1;
/// Largest stored payload, in bytes of JSON.
pub const MAX_PAYLOAD: usize = 64 * 1024;
/// Replies longer than this stay text.
pub const MAX_REPLY: usize = 128 * 1024;
/// Why a row is highlighted; the only reason v1 gives.
pub const PICK_REASON: &str = "named as the pick in this reply";
/// The one line of the bot system prompt that asks for structure the selectors can read (plan
/// §A.4). It is in every bot's system prompt once (`drivers::TurnInput::system_prompt_parts`).
///
/// Two options count too: "Aldi vs Lidl" came back as bullets when the line said "3 or more", and
/// the transposed shape (options as columns, a row per point) is what [`selector`] reads as a
/// comparison. Every header cell must be filled, or the table stays text.
pub const PROMPT_LINE: &str = "When you compare options use a table with every header cell filled (one row per option; for \
    just two, a column each and a row per point), for steps use a numbered list, for a recipe an 'Ingredients' list then \
    numbered steps, and for key numbers use '**Label:** value' lines; kwant shows them as cards and charts.";

/// The checked block set for a reply: the first candidate, in priority order ([`rank`]), whose
/// type is enabled and that passes [`validate`]; `None` when there isn't one. A selector bug must never
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

/// One block; every type has `id`, `title`, `source` and `provenance` (plan §A.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Chart(Chart),
    Comparison(Comparison),
    Metrics(Metrics),
    Findings(Findings),
    Cards(Cards),
    Steps(Steps),
    Facts(Facts),
    Verification(Verification),
    Files(Files),
    Recipe(Recipe),
}

impl Block {
    /// The block's `type`.
    pub fn kind(&self) -> &'static str {
        match self {
            Block::Chart(_) => "chart",
            Block::Comparison(_) => "comparison",
            Block::Metrics(_) => "metrics",
            Block::Findings(_) => "findings",
            Block::Cards(_) => "cards",
            Block::Steps(_) => "steps",
            Block::Facts(_) => "facts",
            Block::Verification(_) => "verification",
            Block::Files(_) => "files",
            Block::Recipe(_) => "recipe",
        }
    }

    pub fn id(&self) -> &str {
        common!(self, b => &b.id)
    }

    pub fn title(&self) -> Option<&str> {
        common!(self, b => b.title.as_deref())
    }

    pub fn source(&self) -> &Source {
        common!(self, b => &b.source)
    }

    pub fn provenance(&self) -> &Provenance {
        common!(self, b => &b.provenance)
    }
}

/// `$e` for whichever block type `$block` is, bound as `$b`: the fields every type has.
macro_rules! common {
    ($block:expr, $b:ident => $e:expr) => {
        match $block {
            Block::Chart($b) => $e,
            Block::Comparison($b) => $e,
            Block::Metrics($b) => $e,
            Block::Findings($b) => $e,
            Block::Cards($b) => $e,
            Block::Steps($b) => $e,
            Block::Facts($b) => $e,
            Block::Verification($b) => $e,
            Block::Files($b) => $e,
            Block::Recipe($b) => $e,
        }
    };
}
use common;

/// Which table or list of the reply a block stands for, or `slot` (no source in the text: the
/// chat draws it after the reply; `verification`, `files` and a `present_result` call use it and
/// leave `index` out). `index` counts in the order the chat renders them, from 0: tables in one
/// count, numbered lists in another, bullet lists (`ordered: false`) in a third.
///
/// `markdown_section` (`recipe` only) is a run of the reply's top-level blocks: from `lead` blocks
/// before the list `index`/`ordered` to the list `last_index`/`last_ordered`, both ends included.
/// The chat draws the block where the run starts and none of the run's own markdown.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Source {
    /// `markdown_table`, `markdown_list`, `markdown_section` or `slot`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    /// Lists only; absent means `true` (v1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordered: Option<bool>,
    /// `markdown_section` only: its last list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ordered: Option<bool>,
    /// `markdown_section` only: top-level blocks just before its first list that belong to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead: Option<usize>,
}

impl Source {
    pub fn table(index: usize) -> Self {
        Source { kind: "markdown_table".into(), index: Some(index), ..Source::default() }
    }
    pub fn list(index: usize, ordered: bool) -> Self {
        Source { kind: "markdown_list".into(), index: Some(index), ordered: Some(ordered), ..Source::default() }
    }
    /// The run from `lead` blocks before list `first` to list `last` (index, ordered).
    pub fn section(first: (usize, bool), last: (usize, bool), lead: usize) -> Self {
        Source {
            kind: "markdown_section".into(),
            index: Some(first.0),
            ordered: Some(first.1),
            last_index: Some(last.0),
            last_ordered: Some(last.1),
            lead: Some(lead),
        }
    }
    pub fn slot() -> Self {
        Source { kind: "slot".into(), ..Source::default() }
    }
}

/// Where the values came from: copied from the reply (`reply_text`), declared by the bot with
/// `present_result` and grounded against the reply (`bot_declared`, with `tool_call_id`), or read
/// from kwant's own stored data (`kwant_data`, with `ref` "task:<id>" or "attachment:<id>").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

impl Provenance {
    pub fn reply_text() -> Self {
        Provenance { kind: "reply_text".into(), tool_call_id: None, reference: None }
    }
    pub fn kwant_data(reference: String) -> Self {
        Provenance { kind: "kwant_data".into(), tool_call_id: None, reference: Some(reference) }
    }
    pub fn bot_declared(tool_call_id: String) -> Self {
        Provenance { kind: "bot_declared".into(), tool_call_id: Some(tool_call_id), reference: None }
    }
}

/// What a comparison column holds; absent (v1) means `label` for the first column and `value`
/// for the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Label,
    Value,
    Pros,
    Cons,
    Note,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub key: String,
    pub label: String,
    pub numeric: bool,
    pub unit: Option<String>,
    #[serde(default)]
    pub role: Option<Role>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub id: String,
    pub cells: Vec<String>,
}

/// A table: 2–8 columns, 3–50 rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub source: Source,
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
    pub highlight_row: Option<String>,
    pub highlight_reason: Option<String>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    Line,
    Bar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisKind {
    Time,
    Category,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Axis {
    pub label: String,
    pub kind: AxisKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub key: String,
    pub label: String,
    pub unit: Option<String>,
}

/// One table row: `x` is its first cell, `cells` the others (one per series), verbatim, and
/// `values` the same cells as numbers (`null` where a cell has none: a gap, never a zero).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub id: String,
    pub x: String,
    pub cells: Vec<String>,
    pub values: Vec<Option<f64>>,
}

/// A table of numbers: 1–4 series, 3–60 points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chart {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub source: Source,
    pub chart_kind: ChartKind,
    pub x: Axis,
    pub series: Vec<Series>,
    pub points: Vec<Point>,
    /// Quotes cells only, never a computed number; the selectors leave it `null`.
    pub summary: Option<String>,
    pub provenance: Provenance,
}

/// `value_text` is what the reply says ("$4.2M") and what the chat shows; `value` is its number as
/// written (4.2), `unit` its one affix when it has exactly one, `note` the rest of the item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub id: String,
    pub label: String,
    pub value_text: String,
    pub value: f64,
    pub unit: Option<String>,
    pub note: Option<String>,
}

/// Key numbers: 2–6 items.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub source: Source,
    pub items: Vec<Metric>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub id: String,
    pub label: String,
    pub value: String,
}

/// Labelled values: 3–12 items.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Facts {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub source: Source,
    pub items: Vec<Fact>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    #[default]
    Steps,
    Checklist,
    Timeline,
}

/// `done` is set for a checklist only, `when` for a timeline (every item), `owner` only from a
/// literal "Owner: X" or "(owner: X)". `text` is the item as written, after any `[ ]` box or time
/// label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub done: Option<bool>,
    #[serde(default)]
    pub when: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
}

/// A list to follow: 3–20 items.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Steps {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub source: Source,
    #[serde(default)]
    pub variant: Variant,
    pub items: Vec<Step>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    pub title: String,
    /// An http(s) link of the reply, or `null`.
    pub url: Option<String>,
    pub description: String,
    pub meta: Vec<String>,
}

/// Links or named options with a description: 3–12 items.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cards {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub source: Source,
    pub items: Vec<Card>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ref {
    pub path: String,
    pub line: Option<u32>,
}

/// `severity_text` is the reply's own token ("Blocker", "P1", "Nit"); `detail` may be empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub severity: Severity,
    pub severity_text: String,
    pub title: String,
    pub detail: String,
    pub refs: Vec<Ref>,
}

/// Review or audit results: 2–30 items, in the reply's order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Findings {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub source: Source,
    pub items: Vec<Finding>,
    pub provenance: Provenance,
}

/// The task a `verification` block is about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationTarget {
    pub kind: String,
    pub id: String,
    pub title: String,
}

/// One outcome check's stored result. `status` is `passed`, `failed`, `error` or `pending` (a
/// check that has not finished: waiting for approval, queued or running). `label` and `excerpt`
/// are redacted like the check's own stored output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub status: String,
    pub excerpt: Option<String>,
    pub checked_at: Option<String>,
}

/// A mission task's outcome checks, attached to its "Verified: ..." / "Failed: ..." report line.
/// A snapshot taken when the report was posted; each check says when it was checked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verification {
    pub id: String,
    pub title: Option<String>,
    pub source: Source,
    pub provenance: Provenance,
    pub target: VerificationTarget,
    pub checks: Vec<Check>,
    pub summary: String,
}

/// What a file's preview shows: a CSV/TSV table (at most 20 rows by 8 columns), the first lines of
/// a text file, or the image itself (drawn from the authorized attachment route). `None` is a file
/// row with no preview.
///
/// A preview that was cut says so, so a 20-row preview is never read as the whole file. The extra
/// fields are all optional and left out of the JSON for a complete preview (old fixtures stay
/// valid): `truncated: true` is set whenever the file has more than the preview shows, and
/// `total_rows` (records after the header) and `total_cols` (CSV/TSV) or `total_lines` (text) are
/// added only when kwant counted them exactly (never below what the preview shows); a file cut
/// at the 1 MB read cap gets `truncated` and no totals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FilePreview {
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        truncated: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total_rows: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total_cols: Option<u64>,
    },
    Text {
        excerpt: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        truncated: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total_lines: Option<u64>,
    },
    Image,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileItem {
    pub id: String,
    pub attachment_id: String,
    pub name: String,
    pub mime: String,
    pub size_bytes: i64,
    pub preview: Option<FilePreview>,
}

/// The files a bot shared with one message, and only those: each one is a stored attachment of
/// this chat, shared by this bot. Bytes are served by the existing attachment routes, never from
/// here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Files {
    pub id: String,
    pub title: Option<String>,
    pub source: Source,
    pub provenance: Provenance,
    pub items: Vec<FileItem>,
}

/// A time of a recipe in the reply's own words: "Prep" and "15 min".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeTime {
    pub label: String,
    pub text: String,
}

/// One ingredient line. `qty` is its leading amount exactly as written ("2 cups", "½", "1–2") and
/// `text` the rest, so the line is `text` alone without a `qty`, `qty + text` when `text` starts
/// with ",", and `qty + " " + text` otherwise. `group` is the sub-heading above it ("For the sauce").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ingredient {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub qty: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
}

impl Ingredient {
    /// The line as the reply wrote it.
    pub fn line(&self) -> String {
        match &self.qty {
            None => self.text.clone(),
            Some(q) if self.text.starts_with(',') => format!("{q}{}", self.text),
            Some(q) => format!("{q} {}", self.text),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeStep {
    pub id: String,
    pub text: String,
}

/// One recipe: 2–40 ingredients and 2–30 steps, with servings, up to 3 times and up to 6 notes
/// when the reply gives them. `title` is the dish name. Its source is the `markdown_section` the
/// recipe fills, from the dish heading to its last list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub source: Source,
    #[serde(default)]
    pub serves: Option<u32>,
    #[serde(default)]
    pub times: Vec<RecipeTime>,
    pub ingredients: Vec<Ingredient>,
    pub steps: Vec<RecipeStep>,
    #[serde(default)]
    pub notes: Vec<String>,
    pub provenance: Provenance,
}
