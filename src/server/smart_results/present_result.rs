//! `present_result`: a self tool a bot may call during a turn to declare the block its final reply
//! deserves. Nothing it declares is shown unless it is grounded in the final reply, checked when
//! the message is stored (after any re-prompt), never when the tool is called:
//!
//! - a `comparison` must name the table of the reply it stands for (`source`) and be all of it:
//!   the same columns and rows, every header and cell the same text at the same position (markdown,
//!   case and punctuation aside), so a value moved under another label is refused (a call without
//!   `source` is refused);
//! - `steps` must name the numbered list of the reply they stand for (`source`) and be all of it:
//!   the same items in the same order (a task list is never declared as steps);
//! - a `recipe` needs no `source`: the reply must hold the one recipe the recipe selector reads,
//!   with the same ingredient lines and steps in the same order, and the block stored is the one
//!   read from the reply (so its amounts, groups, servings and times are the reply's own);
//! - every URL is a link in the reply (never plain text, never with a login in it), and nothing is
//!   HTML.
//!
//! That is what the client checks before it draws a declared block in place of the reply's own, so
//! a block that grounds here is one that is drawn.
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

use super::recipe::recipe_of;
use super::selector::{is_task, lists_of, plain, tables_of};
use super::{Block, Column, Comparison, Ingredient, Provenance, Recipe, RecipeStep, ResultSet, Row, Source, Step, Steps, VERSION};

const NAME: &str = "present_result";
/// Largest call, in bytes of JSON; a bigger one is refused at the call.
const MAX_ARGS: usize = 64 * 1024;

pub fn handles(name: &str) -> bool {
    name == NAME
}

pub fn tools() -> Vec<Value> {
    vec![json!({
        "name": NAME,
        "description": "Optional. Declare the one block kwant should draw for your final reply (a comparison table, a numbered-steps list), only when the answer's data reads better that way than as plain text; never for a simple answer, and never instead of writing the reply. The data must match your reply text exactly: every label, cell, item and link is checked against it, and a call that doesn't match is dropped and the plain reply is shown. For a comparison, `source` is required: the index (from 0) of the table in your reply that it describes. For a recipe, no `source`: your reply must hold the recipe as an 'Ingredients' list then numbered steps. Call it once, just before your final reply. Types today: comparison, steps and recipe; kwant draws charts, metrics, facts, cards and findings from your reply's own structure (use '**Label:** value' lines for key numbers).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "type": {"type": "string", "enum": ["comparison", "steps", "recipe"]},
                "title": {"type": "string", "description": "Optional short title."},
                "source": {
                    "type": "object",
                    "description": "Which table or numbered list of your reply the block replaces: {kind: 'markdown_table' | 'markdown_list', index: number counted from 0 in the order they appear}. Required for comparison and steps; leave it out for a recipe.",
                    "properties": {"kind": {"type": "string", "enum": ["markdown_table", "markdown_list"]}, "index": {"type": "integer", "minimum": 0}},
                    "required": ["kind", "index"],
                },
                "data": {
                    "type": "object",
                    "description": "comparison: {columns: [{label, numeric?, unit?}], rows: [{cells: [string, ...]}]}, 3-50 rows and 2-8 columns, every header and cell exactly as in your table: all of its rows and columns, in its order. steps: {items: [string, ...]}, 3-15 items: all the items of that list, in its order. recipe: {ingredients: [string, ...], steps: [string, ...]}, 2-40 ingredient lines and 2-30 steps, each exactly as in your reply, in its order.",
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
    if kind == "recipe" {
        // Where it goes is read from the reply itself, never from the call
        let block = recipe(data)?;
        let set = ResultSet { version: VERSION, blocks: vec![block.clone()] };
        super::validate(&set, None).map_err(|why| format!("the block doesn't fit ({why})"))?;
        return Ok(Declared { block, source: None });
    }
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
        source: Source { kind: "markdown_table".into(), index: source.map(|s| s.1), ..Source::default() },
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
    let source = Source { kind: "markdown_list".into(), index: source.map(|s| s.1), ordered: Some(true), ..Source::default() };
    Ok(Block::Steps(Steps { id: "b0".into(), title: None, variant: super::Variant::Steps, source, items, provenance: Provenance::reply_text() }))
}

/// A declared recipe: its ingredient lines (held whole in `text`) and steps. The block that is
/// stored is the reply's own (see [`ground`]); this one only carries what to match.
fn recipe(data: &Value) -> Result<Block, String> {
    let strings = |key: &str| -> Result<Vec<String>, String> {
        data.get(key)
            .and_then(Value::as_array)
            .ok_or(format!("`data.{key}` is missing"))?
            .iter()
            .map(|v| v.as_str().map(str::to_string).ok_or(format!("every one of `data.{key}` must be a string")))
            .collect()
    };
    let ingredients = strings("ingredients")?
        .into_iter()
        .enumerate()
        .map(|(k, text)| Ingredient { id: format!("i{k}"), text, qty: None, group: None })
        .collect();
    let steps = strings("steps")?.into_iter().enumerate().map(|(k, text)| RecipeStep { id: format!("s{k}"), text }).collect();
    Ok(Block::Recipe(Recipe {
        id: "b0".into(),
        title: None,
        source: Source::section((0, false), (0, true), 0),
        serves: None,
        times: Vec::new(),
        ingredients,
        steps,
        notes: Vec::new(),
        provenance: Provenance::reply_text(),
    }))
}

/// `declared` held to `reply`, as a block with `bot_declared` provenance, or the reason it isn't.
///
/// A declaration is the whole table (or numbered list) it names, in the same order, because that
/// is what the client checks before it draws the block in place of the reply's own: same number of
/// columns, rows and items, every header, cell and item the same text (markdown, case and
/// punctuation aside) with the same numbers in the same order, so punctuation inside a number is
/// never aside: "26.50" is not "2650" and "-3" is not "3". A declaration the client would refuse
/// is refused here, so a stored block is always one that is drawn.
fn ground(declared: Declared, reply: &str, tool_call_id: String) -> Result<Block, String> {
    let Declared { mut block, source } = declared;
    let links = reply_links(reply);
    match &mut block {
        Block::Comparison(c) => {
            let (_, index) = source.ok_or("a comparison without a source")?;
            let table = tables_of(reply).into_iter().find(|t| t.index == index).ok_or("no table at that index")?;
            if c.columns.len() != table.header.len() {
                return Err("the columns aren't all of that table's columns".into());
            }
            if c.rows.len() != table.rows.len() {
                return Err("the rows aren't all of that table's rows".into());
            }
            if c.columns.iter().zip(&table.header).any(|(col, h)| !same_cell(&col.label, h)) {
                return Err("a column label isn't the table's header at that position".into());
            }
            for col in &c.columns {
                if col.unit.as_deref().is_some_and(|u| !normalize(reply).contains(&normalize(u))) {
                    return Err("a unit isn't in the reply".into());
                }
            }
            for (row, table_row) in c.rows.iter().zip(&table.rows) {
                for cell in &row.cells {
                    check_urls(cell, &links)?;
                }
                if row.cells.len() != table_row.len() || row.cells.iter().zip(table_row).any(|(a, b)| !same_cell(a, b)) {
                    return Err("a row doesn't match the table's row at that position".into());
                }
            }
            c.provenance = Provenance::bot_declared(tool_call_id);
        }
        Block::Steps(s) => {
            let (_, index) = source.ok_or("steps without a source")?;
            let list = lists_of(reply).into_iter().find(|l| l.index == index).ok_or("no list at that index")?;
            if s.items.len() != list.items.len() {
                return Err("the steps aren't all of that list's items".into());
            }
            // A task list is drawn as one (with its done marks) or not at all, never as plain steps
            if list.items.iter().any(|t| is_task(t)) {
                return Err("that list is a task list".into());
            }
            for (item, listed) in s.items.iter().zip(&list.items) {
                check_urls(&item.text, &links)?;
                if !same_cell(&item.text, listed) {
                    return Err("a step isn't the list's item at that position".into());
                }
            }
            s.provenance = Provenance::bot_declared(tool_call_id);
        }
        Block::Recipe(declared) => {
            let mut read = recipe_of(reply).ok_or("the reply has no recipe kwant can read")?;
            if declared.ingredients.len() != read.ingredients.len() || declared.steps.len() != read.steps.len() {
                return Err("the ingredients and steps aren't all of the reply's recipe".into());
            }
            for (d, r) in declared.ingredients.iter().zip(&read.ingredients) {
                check_urls(&d.text, &links)?;
                if !same_cell(&d.text, &r.line()) {
                    return Err("an ingredient isn't the recipe's line at that position".into());
                }
            }
            for (d, r) in declared.steps.iter().zip(&read.steps) {
                check_urls(&d.text, &links)?;
                if !same_cell(&d.text, &r.text) {
                    return Err("a step isn't the recipe's step at that position".into());
                }
            }
            read.provenance = Provenance::bot_declared(tool_call_id);
            return Ok(Block::Recipe(read));
        }
        _ => return Err("not declarable".into()),
    }
    Ok(block)
}

/// The text the client compares by: markdown out, lowercase, letters and digits only.
fn match_key(s: &str) -> String {
    plain(s).to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// `a` and `b` read as the same text, and not as nothing.
fn same_text(a: &str, b: &str) -> bool {
    let key = match_key(a);
    !key.is_empty() && key == match_key(b)
}

/// `a` and `b` are the same text (letters and digits, as the client compares) and carry the same
/// numbers in the same order: same values, same signs, same decimal separators ("1.5" is not "15",
/// "1,5" is not "1.5", "-3" is not "3"). A thousands comma is not a difference ("1,800" is 1800).
fn same_cell(a: &str, b: &str) -> bool {
    same_text(a, b) && same_numbers(&string_numbers(&plain(a)), &string_numbers(&plain(b)))
}

fn same_numbers(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= x.abs().max(y.abs()) * 1e-9 + 1e-9)
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
        let groups: Vec<&str> = token.split(',').collect();
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
        let word = word.trim_end_matches(['.', ',', ';', ':', '!', '?']);
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

#[cfg(test)]
#[path = "present_result_tests.rs"]
mod tests;
