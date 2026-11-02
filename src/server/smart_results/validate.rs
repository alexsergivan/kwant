//! Checks every block set before it is stored, and again when a stored one is read back.

use std::collections::HashSet;

use super::{Block, ResultSet, ENABLED_TYPES, MAX_PAYLOAD, PICK_REASON, VERSION};

/// `Ok` when `set` is a block set this build may store and serve. With `reply`, every label, unit,
/// cell and item must also be text of that reply (values are copied, never made up).
pub fn validate(set: &ResultSet, reply: Option<&str>) -> Result<(), String> {
    if set.version != VERSION {
        return Err(format!("unknown version {}", set.version));
    }
    if set.blocks.len() != 1 {
        return Err(format!("{} blocks; v1 has exactly one", set.blocks.len()));
    }
    if let Some(off) = set.blocks.iter().find(|b| !ENABLED_TYPES.contains(&b.kind())) {
        return Err(format!("type {} is not enabled", off.kind()));
    }
    let mut texts: Vec<&str> = Vec::new();
    for (i, block) in set.blocks.iter().enumerate() {
        let want = format!("b{i}");
        match block {
            Block::Comparison(c) => {
                if c.id != want {
                    return Err(format!("block id {:?}", c.id));
                }
                if c.source.kind != "markdown_table" || c.source.index > 10_000 {
                    return Err("comparison source".into());
                }
                if c.provenance.kind != "reply_text" {
                    return Err("comparison provenance".into());
                }
                if !(2..=8).contains(&c.columns.len()) {
                    return Err(format!("{} columns", c.columns.len()));
                }
                for (k, col) in c.columns.iter().enumerate() {
                    if col.key != format!("c{k}") {
                        return Err(format!("column key {:?}", col.key));
                    }
                    check_text(&col.label, 200, "column label")?;
                    texts.push(&col.label);
                    match &col.unit {
                        Some(_) if !col.numeric => return Err("a unit on a text column".into()),
                        Some(unit) => {
                            check_text(unit, 10, "unit")?;
                            texts.push(unit);
                        }
                        None => {}
                    }
                }
                if !(3..=50).contains(&c.rows.len()) {
                    return Err(format!("{} rows", c.rows.len()));
                }
                let mut labels = HashSet::new();
                for (r, row) in c.rows.iter().enumerate() {
                    if row.id != format!("r{r}") {
                        return Err(format!("row id {:?}", row.id));
                    }
                    if row.cells.len() != c.columns.len() {
                        return Err(format!("row {r} has {} cells", row.cells.len()));
                    }
                    check_text(&row.cells[0], 200, "row label")?;
                    if !labels.insert(row.cells[0].as_str()) {
                        return Err(format!("row label {:?} repeats", row.cells[0]));
                    }
                    for cell in &row.cells {
                        if cell.chars().count() > 200 {
                            return Err("a cell over 200 characters".into());
                        }
                        if has_html(cell) {
                            return Err("HTML in a cell".into());
                        }
                        texts.push(cell);
                    }
                }
                match (&c.highlight_row, &c.highlight_reason) {
                    (None, None) => {}
                    (Some(row), Some(reason)) => {
                        if !c.rows.iter().any(|r| &r.id == row) {
                            return Err(format!("highlight_row {row:?} is not a row"));
                        }
                        if reason != PICK_REASON {
                            return Err("highlight_reason".into());
                        }
                    }
                    _ => return Err("highlight_row and highlight_reason come together".into()),
                }
            }
            Block::Steps(s) => {
                if s.id != want {
                    return Err(format!("block id {:?}", s.id));
                }
                if s.source.kind != "markdown_list" || s.source.index > 10_000 {
                    return Err("steps source".into());
                }
                if s.provenance.kind != "reply_text" {
                    return Err("steps provenance".into());
                }
                if !(3..=15).contains(&s.items.len()) {
                    return Err(format!("{} steps", s.items.len()));
                }
                for (k, item) in s.items.iter().enumerate() {
                    if item.id != format!("s{k}") {
                        return Err(format!("step id {:?}", item.id));
                    }
                    check_text(&item.text, 300, "step")?;
                    texts.push(&item.text);
                }
            }
        }
    }
    if let Some(reply) = reply {
        if let Some(t) = texts.iter().find(|t| !reply.contains(**t)) {
            return Err(format!("{t:?} is not text of the reply"));
        }
    }
    let size = serde_json::to_vec(set).map_err(|e| e.to_string())?.len();
    if size > MAX_PAYLOAD {
        return Err(format!("payload of {size} bytes"));
    }
    Ok(())
}

fn check_text(s: &str, max: usize, what: &str) -> Result<(), String> {
    if s.trim().is_empty() {
        return Err(format!("an empty {what}"));
    }
    if s.chars().count() > max {
        return Err(format!("a {what} over {max} characters"));
    }
    if has_html(s) {
        return Err(format!("HTML in a {what}"));
    }
    Ok(())
}

/// Anything that could open an HTML tag, comment or declaration (`<b`, `</`, `<!`, `<?`).
pub(super) fn has_html(s: &str) -> bool {
    s.char_indices().any(|(i, c)| {
        c == '<' && s[i + 1..].chars().next().is_some_and(|n| n.is_ascii_alphabetic() || matches!(n, '/' | '!' | '?'))
    })
}
