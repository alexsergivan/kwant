//! `present_result`: a self tool a bot may call during a turn to declare the block its final reply
//! deserves. Nothing it declares is shown unless it is grounded in the final reply, checked when
//! the message is stored (after any re-prompt), never when the tool is called:
//!
//! - a `comparison` must name the table of the reply it stands for (`source`), and every row it
//!   declares must be a row of that table: each cell found in the same row, so a value moved under
//!   another label is refused (a call without `source` is refused);
//! - `steps` must name the numbered list of the reply they stand for (`source`), and every item must
//!   quote an item of that list;
//! - every number equals a number in the reply, every URL is a link in the reply (never plain text,
//!   never with a login in it), and nothing is HTML.
//!
//! Only the last call that was valid when made counts. If it doesn't ground, the block is dropped
//! (logged) and the deterministic selectors run on the reply as if the tool had never been called.
//! A bot can never set a block's provenance or the `verification` and `files` types: those come
//! from kwant's own data only.

use serde_json::{json, Value};

use crate::db::models::NewMessage;
use crate::mcp::SELF_SERVER_NAME;
use crate::server::continuity::SelfScope;
use crate::server::AppState;

use super::selector::{lists_of, tables_of};
use super::{Block, Column, Comparison, Provenance, ResultSet, Row, Source, Step, Steps, VERSION};

const NAME: &str = "present_result";
/// Largest call, in bytes of JSON; a bigger one is refused at the call.
const MAX_ARGS: usize = 64 * 1024;

pub fn handles(name: &str) -> bool {
    name == NAME
}

pub fn tools() -> Vec<Value> {
    vec![json!({
        "name": NAME,
        "description": "Optional. Declare the one block kwant should draw for your final reply (a comparison table, a numbered-steps list), only when the answer's data reads better that way than as plain text; never for a simple answer, and never instead of writing the reply. The data must match your reply text exactly: every label, number and link is checked against it, and a call that doesn't match is dropped and the plain reply is shown. For a comparison, `source` is required: the index (from 0) of the table in your reply that it describes. Call it once, just before your final reply. Types today: comparison and steps; kwant draws charts, metrics, facts, cards and findings from your reply's own structure (use '**Label:** value' lines for key numbers).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "type": {"type": "string", "enum": ["comparison", "steps"]},
                "title": {"type": "string", "description": "Optional short title."},
                "source": {
                    "type": "object",
                    "description": "Which table or numbered list of your reply the block replaces: {kind: 'markdown_table' | 'markdown_list', index: number counted from 0 in the order they appear}. Required for both types.",
                    "properties": {"kind": {"type": "string", "enum": ["markdown_table", "markdown_list"]}, "index": {"type": "integer", "minimum": 0}},
                    "required": ["kind", "index"],
                },
                "data": {
                    "type": "object",
                    "description": "comparison: {columns: [{label, numeric?, unit?}], rows: [{cells: [string, ...]}]}, 3-50 rows and 2-8 columns, cells exactly as in your table. steps: {items: [string, ...]}, 3-15 items, each quoting an item of that list.",
                },
            },
            "required": ["type", "data"],
        },
    })]
}

/// Checks the call's shape and says plainly what is wrong; grounding comes later, against the
/// final reply.
pub async fn call(_state: &AppState, _scope: &SelfScope, name: &str, args: &Value) -> (String, bool) {
    if !handles(name) {
        return (format!("Unknown tool {name}."), true);
    }
    match declared(args) {
        Ok(_) => ("Noted. kwant draws it only if your final reply contains the same labels, numbers and links; write your reply as usual.".into(), false),
        Err(why) => (format!("Not accepted: {why}. Your reply will show as written."), true),
    }
}

/// A call's block before grounding, with where it points.
#[derive(Debug)]
struct Declared {
    block: Block,
    /// `markdown_table` / `markdown_list` index, or `None` for a block drawn after the reply.
    source: Option<(bool, usize)>,
}

/// The block of the turn's last valid `present_result` call, when it is grounded in `input.text`
/// (the final reply) and passes the block checks; `None` otherwise (nothing declared, an invalid
/// call, or an ungrounded one, which is logged).
pub fn grounded_candidate(input: &NewMessage) -> Option<Block> {
    let calls = input.tool_calls.as_ref()?.as_array()?;
    let (id, declared) = calls.iter().rev().find_map(|c| {
        let ran = matches!(c.get("status")?.as_str()?, "ok" | "allowed");
        if !ran || c.get("server")?.as_str()? != SELF_SERVER_NAME || c.get("tool")?.as_str()? != NAME {
            return None;
        }
        let id = c.get("id")?.as_str()?.chars().take(200).collect::<String>();
        Some((id, declared(c.get("arguments")?).ok()?))
    })?;
    match ground(declared, &input.text, id) {
        Ok(block) => Some(block),
        Err(why) => {
            tracing::info!("smart results: dropped a present_result block: {why}");
            None
        }
    }
}

/// Reads and checks a call's arguments. Only `type`, `source` and `data` are read: a bot's own
/// `provenance`, `id` or `tool_call_id` is never looked at.
fn declared(args: &Value) -> Result<Declared, String> {
    if args.to_string().len() > MAX_ARGS {
        return Err("the call is over 64 KB".into());
    }
    let kind = args.get("type").and_then(Value::as_str).ok_or("`type` is missing")?;
    let data = args.get("data").filter(|d| d.is_object()).ok_or("`data` is missing")?;
    let source = match args.get("source") {
        None | Some(Value::Null) => None,
        Some(s) => {
            let kind = s.get("kind").and_then(Value::as_str).ok_or("`source.kind` is missing")?;
            let index = s.get("index").and_then(Value::as_u64).filter(|i| *i <= 10_000).ok_or("`source.index` must be a whole number")?;
            match kind {
                "markdown_table" => Some((true, index as usize)),
                "markdown_list" => Some((false, index as usize)),
                _ => return Err("`source.kind` must be markdown_table or markdown_list".into()),
            }
        }
    };
    let block = match kind {
        "comparison" => {
            if !matches!(source, Some((true, _))) {
                return Err("a comparison needs `source` pointing at the table of your reply it describes (kind markdown_table)".into());
            }
            comparison(data, source)?
        }
        "steps" => {
            if !matches!(source, Some((false, _))) {
                return Err("steps need `source` pointing at the numbered list of your reply they describe (kind markdown_list)".into());
            }
            steps(data, source)?
        }
        "chart" | "metrics" | "facts" | "cards" | "findings" => {
            return Err(format!("{kind} can't be declared yet; kwant draws it from your reply's own table or '**Label:** value' lines"));
        }
        "verification" | "files" => return Err(format!("{kind} comes only from kwant's own data")),
        other => return Err(format!("unknown type {other:?}")),
    };
    let set = ResultSet { version: VERSION, blocks: vec![block.clone()] };
    super::validate(&set, None).map_err(|why| format!("the block doesn't fit ({why})"))?;
    Ok(Declared { block, source })
}

fn comparison(data: &Value, source: Option<(bool, usize)>) -> Result<Block, String> {
    let raw_columns = data.get("columns").and_then(Value::as_array).ok_or("`data.columns` is missing")?;
    let columns = raw_columns
        .iter()
        .enumerate()
        .map(|(k, c)| {
            let label = c.get("label").and_then(Value::as_str).ok_or("a column needs a `label`")?;
            Ok(Column {
                key: format!("c{k}"),
                label: label.to_string(),
                numeric: c.get("numeric").and_then(Value::as_bool).unwrap_or(false),
                unit: c.get("unit").and_then(Value::as_str).map(str::to_string),
                role: None,
            })
        })
        .collect::<Result<Vec<_>, &str>>()?;
    let raw_rows = data.get("rows").and_then(Value::as_array).ok_or("`data.rows` is missing")?;
    let rows = raw_rows
        .iter()
        .enumerate()
        .map(|(r, row)| {
            let cells = row
                .get("cells")
                .and_then(Value::as_array)
                .ok_or("a row needs `cells`")?
                .iter()
                .map(|c| c.as_str().map(str::to_string).ok_or("every cell must be a string"))
                .collect::<Result<Vec<_>, _>>()?;
            if cells.len() != columns.len() {
                return Err("a row has a different number of cells than there are columns");
            }
            Ok(Row { id: format!("r{r}"), cells })
        })
        .collect::<Result<Vec<_>, &str>>()?;
    Ok(Block::Comparison(Comparison {
        id: "b0".into(),
        title: None,
        source: Source { kind: "markdown_table".into(), index: source.map(|s| s.1), ordered: None },
        columns,
        rows,
        highlight_row: None,
        highlight_reason: None,
        provenance: Provenance::reply_text(),
    }))
}

fn steps(data: &Value, source: Option<(bool, usize)>) -> Result<Block, String> {
    let items = data
        .get("items")
        .and_then(Value::as_array)
        .ok_or("`data.items` is missing")?
        .iter()
        .enumerate()
        .map(|(k, it)| it.as_str().map(|t| Step { id: format!("s{k}"), text: t.to_string(), done: None, when: None, owner: None }).ok_or("every step must be a string"))
        .collect::<Result<Vec<_>, _>>()?;
    let source = Source { kind: "markdown_list".into(), index: source.map(|s| s.1), ordered: Some(true) };
    Ok(Block::Steps(Steps { id: "b0".into(), title: None, variant: super::Variant::Steps, source, items, provenance: Provenance::reply_text() }))
}

/// `declared` held to `reply`, as a block with `bot_declared` provenance, or the reason it isn't.
fn ground(declared: Declared, reply: &str, tool_call_id: String) -> Result<Block, String> {
    let Declared { mut block, source } = declared;
    let links = reply_links(reply);
    let numbers = reply_numbers(reply);
    match &mut block {
        Block::Comparison(c) => {
            let (_, index) = source.ok_or("a comparison without a source")?;
            let table = tables_of(reply).into_iter().find(|t| t.index == index).ok_or("no table at that index")?;
            if c.columns.iter().any(|col| !has_label(&table.header, &col.label)) {
                return Err("a column label isn't in that table's header".into());
            }
            for col in &c.columns {
                if col.unit.as_deref().is_some_and(|u| !normalize(reply).contains(&normalize(u))) {
                    return Err("a unit isn't in the reply".into());
                }
            }
            for row in &c.rows {
                for cell in &row.cells {
                    check_urls(cell, &links)?;
                }
                if !table.rows.iter().any(|t| row.cells.iter().all(|cell| table_row_has(t, cell))) {
                    return Err("a row doesn't match any row of that table".into());
                }
            }
            c.provenance = Provenance::bot_declared(tool_call_id);
        }
        Block::Steps(s) => {
            let (_, index) = source.ok_or("steps without a source")?;
            let list = lists_of(reply).into_iter().find(|l| l.index == index).ok_or("no list at that index")?;
            for item in &s.items {
                check_urls(&item.text, &links)?;
                let wanted = normalize(&item.text);
                if wanted.is_empty() || !list.items.iter().any(|t| normalize(t).contains(&wanted)) {
                    return Err("a step isn't an item of that list".into());
                }
                if !string_numbers(&item.text).iter().all(|n| numbers.iter().any(|r| same(*r, *n))) {
                    return Err("a number isn't in the reply".into());
                }
            }
            s.provenance = Provenance::bot_declared(tool_call_id);
        }
        _ => return Err("not declarable".into()),
    }
    Ok(block)
}

fn has_label(header: &[String], label: &str) -> bool {
    let want = normalize(label);
    !want.is_empty() && header.iter().any(|h| normalize(h).contains(&want))
}

/// `cell` is found in one cell of the reply's table row `row`: every word of it is in that table
/// cell (markdown and case aside), and a number in it equals a number there, however the reply
/// formats it ("1800" is "1,800", "26.5" is "26.50"; "5" is never "15").
fn table_row_has(row: &[String], cell: &str) -> bool {
    let words: Vec<String> = normalize(cell).split_whitespace().map(str::to_string).collect();
    if words.is_empty() {
        return false;
    }
    row.iter().any(|c| {
        let text = normalize(c);
        let numbers = string_numbers(&text);
        words.iter().all(|w| match parse_number(w) {
            Some(n) => numbers.iter().any(|r| same(*r, n)),
            None => text.contains(w.as_str()),
        })
    })
}

/// Refuses a URL that isn't a link in the reply (a markdown link or `<autolink>`; the same URL as
/// plain text doesn't count), has a login in it, or isn't http(s).
fn check_urls(text: &str, links: &[String]) -> Result<(), String> {
    let lower = text.to_ascii_lowercase();
    for scheme in ["javascript:", "data:", "vbscript:", "file:"] {
        if lower.split_whitespace().any(|w| w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != ':').starts_with(scheme)) {
            return Err(format!("a {scheme} URL"));
        }
    }
    for word in text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']')) {
        let word = word.trim_end_matches(|c| matches!(c, '.' | ',' | ';' | ':' | '!' | '?'));
        if !(word.starts_with("http://") || word.starts_with("https://")) {
            continue;
        }
        let authority = word.split("://").nth(1).unwrap_or("").split(['/', '?', '#']).next().unwrap_or("");
        if authority.contains('@') {
            return Err("a URL with a login in it".into());
        }
        if !links.iter().any(|l| l == word) {
            return Err(format!("{word} isn't a link in the reply"));
        }
    }
    Ok(())
}

/// The targets of the reply's markdown links `[text](url)` and `<url>` autolinks.
fn reply_links(reply: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = reply;
    while let Some(i) = rest.find("](") {
        let after = &rest[i + 2..];
        let end = after.find(|c: char| c == ')' || c.is_whitespace()).unwrap_or(after.len());
        out.push(after[..end].trim_matches(['<', '>']).to_string());
        rest = &after[end..];
    }
    let mut rest = reply;
    while let Some(i) = rest.find('<') {
        let after = &rest[i + 1..];
        let Some(end) = after.find('>') else { break };
        let inner = &after[..end];
        if (inner.starts_with("http://") || inner.starts_with("https://")) && !inner.contains(char::is_whitespace) {
            out.push(inner.to_string());
        }
        rest = &after[end..];
    }
    out
}

/// Lowercased, markdown emphasis and backticks dropped, whitespace collapsed to single spaces.
fn normalize(s: &str) -> String {
    let stripped: String = s.chars().filter(|c| !matches!(c, '*' | '_' | '`')).collect();
    stripped.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= a.abs().max(b.abs()) * 1e-9 + 1e-9
}

/// `s` as a number when all of it is one ("12.50", "1,000", "$9", "-3"); a currency or percent
/// sign around it is allowed.
fn parse_number(s: &str) -> Option<f64> {
    let t = s.trim().trim_matches(|c: char| matches!(c, '$' | '€' | '£' | '%' | ' '));
    let nums = string_numbers(t);
    let digits = t.chars().filter(char::is_ascii_digit).count();
    let shaped = t.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-'));
    (nums.len() == 1 && digits > 0 && shaped).then(|| nums[0])
}

/// Every number in `s`: digits with an optional leading `-` and a decimal point; `1,234,567` with
/// thousands separators is one number, `1,5` is two.
fn string_numbers(s: &str) -> Vec<f64> {
    let mut nums = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, nums: &mut Vec<f64>| {
        let negative = cur.starts_with('-');
        let token = cur.trim_matches(|c| matches!(c, '.' | ',' | '-')).to_string();
        cur.clear();
        if token.is_empty() || !token.chars().any(|c| c.is_ascii_digit()) {
            return;
        }
        let body = token.as_str();
        let groups: Vec<&str> = body.split(',').collect();
        let thousands = groups.len() > 1 && !groups[0].is_empty() && groups[0].len() <= 3 && groups[1..].iter().all(|g| g.len() >= 3 && g.chars().take(3).all(|c| c.is_ascii_digit()));
        let parts: Vec<String> = if thousands { vec![groups.join("")] } else { groups.iter().map(|g| g.to_string()).collect() };
        for p in parts {
            if let Ok(n) = p.parse::<f64>() {
                nums.push(if negative { -n } else { n });
            }
        }
    };
    for c in s.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() || matches!(c, '.' | ',') || (c == '-' && cur.is_empty()) {
            cur.push(c);
        } else {
            flush(&mut cur, &mut nums);
        }
    }
    nums
}

fn reply_numbers(reply: &str) -> Vec<f64> {
    string_numbers(reply)
}

#[cfg(test)]
#[path = "present_result_tests.rs"]
mod tests;
