//! Checks every block set before it is stored, and again when a stored one is read back.

use std::collections::HashSet;

use super::items::severity_of;
use super::{AxisKind, Block, ChartKind, ResultSet, Role, Variant, ENABLED_TYPES, MAX_PAYLOAD, PICK_REASON, VERSION};

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
        if block.id() != format!("b{i}") {
            return Err(format!("block id {:?}", block.id()));
        }
        if let Some(title) = block.title() {
            check_text(title, 300, "title")?;
            texts.push(title);
        }
        check_source(block)?;
        // `present_result` may declare a comparison or steps block itself (`bot_declared`, grounded
        // against the reply by its call); the typed task and file blocks come from kwant's data.
        let prov = block.provenance().kind.as_str();
        let prov_ok = match block {
            Block::Comparison(_) | Block::Steps(_) => matches!(prov, "reply_text" | "bot_declared"),
            Block::Verification(_) | Block::Files(_) => prov == "kwant_data",
            _ => prov == "reply_text",
        };
        if !prov_ok {
            return Err(format!("{} provenance", block.kind()));
        }
        // Only text copied from the reply is checked against it.
        let strict = prov == "reply_text";
        match block {
            Block::Comparison(c) => {
                if !(2..=8).contains(&c.columns.len()) {
                    return Err(format!("{} columns", c.columns.len()));
                }
                for (k, col) in c.columns.iter().enumerate() {
                    if col.key != format!("c{k}") {
                        return Err(format!("column key {:?}", col.key));
                    }
                    check_text(&col.label, 200, "column label")?;
                    if strict {
                        texts.push(&col.label);
                    }
                    match &col.unit {
                        Some(_) if !col.numeric => return Err("a unit on a text column".into()),
                        Some(unit) => {
                            check_text(unit, 10, "unit")?;
                            if strict {
                                texts.push(unit);
                            }
                        }
                        None => {}
                    }
                    if k == 0 && col.role.is_some_and(|r| r != Role::Label) || k > 0 && col.role == Some(Role::Label) {
                        return Err(format!("column {k} role"));
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
                        check_cell(cell)?;
                        if strict {
                            texts.push(cell);
                        }
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
            Block::Chart(c) => {
                check_text(&c.x.label, 200, "axis label")?;
                texts.push(&c.x.label);
                let axis_ok = matches!((c.chart_kind, c.x.kind), (ChartKind::Line, AxisKind::Time) | (ChartKind::Bar, AxisKind::Category));
                if !axis_ok {
                    return Err("chart axis".into());
                }
                if !(1..=4).contains(&c.series.len()) {
                    return Err(format!("{} series", c.series.len()));
                }
                for (k, s) in c.series.iter().enumerate() {
                    if s.key != format!("s{k}") {
                        return Err(format!("series key {:?}", s.key));
                    }
                    check_text(&s.label, 200, "series label")?;
                    texts.push(&s.label);
                    if let Some(unit) = &s.unit {
                        check_text(unit, 10, "unit")?;
                        texts.push(unit);
                    }
                }
                if !(3..=60).contains(&c.points.len()) {
                    return Err(format!("{} points", c.points.len()));
                }
                let mut xs = HashSet::new();
                for (k, p) in c.points.iter().enumerate() {
                    if p.id != format!("p{k}") {
                        return Err(format!("point id {:?}", p.id));
                    }
                    check_text(&p.x, 200, "point label")?;
                    if !xs.insert(p.x.as_str()) {
                        return Err(format!("point {:?} repeats", p.x));
                    }
                    texts.push(&p.x);
                    if p.cells.len() != c.series.len() || p.values.len() != c.series.len() {
                        return Err(format!("point {k} has {} cells, {} values", p.cells.len(), p.values.len()));
                    }
                    if p.values.iter().flatten().any(|v| !v.is_finite()) {
                        return Err("a value that isn't a number".into());
                    }
                    for cell in &p.cells {
                        check_cell(cell)?;
                        if !cell.trim().is_empty() {
                            texts.push(cell);
                        }
                    }
                }
                if c.points.iter().all(|p| p.values.iter().all(Option::is_none)) {
                    return Err("a chart with no values".into());
                }
                if let Some(summary) = &c.summary {
                    check_text(summary, 300, "summary")?;
                    texts.push(summary);
                }
            }
            Block::Metrics(m) => {
                if !(2..=6).contains(&m.items.len()) {
                    return Err(format!("{} metrics", m.items.len()));
                }
                for (k, x) in m.items.iter().enumerate() {
                    if x.id != format!("m{k}") {
                        return Err(format!("metric id {:?}", x.id));
                    }
                    check_text(&x.label, 200, "metric label")?;
                    check_text(&x.value_text, 100, "metric value")?;
                    if !x.value.is_finite() {
                        return Err("a value that isn't a number".into());
                    }
                    texts.extend([x.label.as_str(), x.value_text.as_str()]);
                    if let Some(unit) = &x.unit {
                        check_text(unit, 10, "unit")?;
                        if !x.value_text.contains(unit.as_str()) {
                            return Err(format!("unit {unit:?} is not in its value"));
                        }
                    }
                    if let Some(note) = &x.note {
                        check_text(note, 300, "note")?;
                        texts.push(note);
                    }
                }
            }
            Block::Facts(f) => {
                if !(3..=12).contains(&f.items.len()) {
                    return Err(format!("{} facts", f.items.len()));
                }
                for (k, x) in f.items.iter().enumerate() {
                    if x.id != format!("f{k}") {
                        return Err(format!("fact id {:?}", x.id));
                    }
                    check_text(&x.label, 200, "fact label")?;
                    check_text(&x.value, 300, "fact value")?;
                    texts.extend([x.label.as_str(), x.value.as_str()]);
                }
            }
            Block::Steps(s) => {
                if !(3..=20).contains(&s.items.len()) {
                    return Err(format!("{} steps", s.items.len()));
                }
                for (k, item) in s.items.iter().enumerate() {
                    if item.id != format!("s{k}") {
                        return Err(format!("step id {:?}", item.id));
                    }
                    check_text(&item.text, 300, "step")?;
                    if strict {
                        texts.push(&item.text);
                    }
                    let shape = match s.variant {
                        Variant::Steps => item.done.is_none() && item.when.is_none(),
                        Variant::Checklist => item.done.is_some() && item.when.is_none(),
                        Variant::Timeline => item.done.is_none() && item.when.is_some(),
                    };
                    if !shape {
                        return Err(format!("step {k} doesn't fit its variant"));
                    }
                    for extra in [&item.when, &item.owner].into_iter().flatten() {
                        check_text(extra, 60, "step label")?;
                        if strict {
                            texts.push(extra);
                        }
                    }
                }
            }
            Block::Cards(c) => {
                if !(3..=12).contains(&c.items.len()) {
                    return Err(format!("{} cards", c.items.len()));
                }
                for (k, x) in c.items.iter().enumerate() {
                    if x.id != format!("k{k}") {
                        return Err(format!("card id {:?}", x.id));
                    }
                    check_text(&x.title, 200, "card title")?;
                    check_text(&x.description, 300, "card description")?;
                    texts.extend([x.title.as_str(), x.description.as_str()]);
                    if let Some(url) = &x.url {
                        let safe = url.starts_with("https://")
                            && url.len() <= 2048
                            && !url.contains(|c: char| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '"' | '\'' | '`'));
                        if !safe {
                            return Err("a card link that isn't a plain https link".into());
                        }
                        texts.push(url);
                    }
                    if x.meta.len() > 4 {
                        return Err("card meta".into());
                    }
                    for m in &x.meta {
                        check_text(m, 100, "card meta")?;
                        texts.push(m);
                    }
                }
            }
            Block::Findings(f) => {
                if !(2..=30).contains(&f.items.len()) {
                    return Err(format!("{} findings", f.items.len()));
                }
                for (k, x) in f.items.iter().enumerate() {
                    if x.id != format!("x{k}") {
                        return Err(format!("finding id {:?}", x.id));
                    }
                    if severity_of(&x.severity_text) != Some(x.severity) {
                        return Err(format!("severity {:?} for {:?}", x.severity, x.severity_text));
                    }
                    check_text(&x.title, 300, "finding title")?;
                    texts.extend([x.severity_text.as_str(), x.title.as_str()]);
                    if !x.detail.is_empty() {
                        check_text(&x.detail, 300, "finding detail")?;
                        texts.push(&x.detail);
                    }
                    if x.refs.len() > 10 {
                        return Err("too many refs".into());
                    }
                    for r in &x.refs {
                        check_text(&r.path, 300, "ref")?;
                        if r.path.contains("://") || r.path.contains(char::is_whitespace) {
                            return Err("a ref that isn't a path".into());
                        }
                        texts.push(&r.path);
                    }
                }
            }
            Block::Verification(v) => {
                let task = v.provenance.reference.as_deref().and_then(|r| r.strip_prefix("task:"));
                if v.provenance.kind != "kwant_data" || task.is_none_or(|t| t.is_empty() || t != v.target.id) {
                    return Err("verification provenance".into());
                }
                if v.target.kind != "task" {
                    return Err("verification target".into());
                }
                check_text(&v.target.id, 200, "verification target id")?;
                check_text(&v.target.title, 300, "verification target title")?;
                check_text(&v.summary, 300, "verification summary")?;
                if let Some(title) = &v.title {
                    check_text(title, 300, "verification title")?;
                }
                if v.checks.is_empty() || v.checks.len() > 20 {
                    return Err(format!("{} checks", v.checks.len()));
                }
                for check in &v.checks {
                    if !matches!(check.status.as_str(), "passed" | "failed" | "error" | "pending") {
                        return Err(format!("check status {:?}", check.status));
                    }
                    check_text(&check.id, 200, "check id")?;
                    check_text(&check.label, 300, "check label")?;
                    check_text(&check.kind, 100, "check kind")?;
                    if let Some(excerpt) = &check.excerpt {
                        check_text(excerpt, 300, "check excerpt")?;
                    }
                    if let Some(at) = &check.checked_at {
                        check_text(at, 100, "check time")?;
                    }
                }
            }
            Block::Files(files) => {
                if files.provenance.kind != "kwant_data"
                    || files.provenance.reference.as_deref().is_none_or(|r| !r.starts_with("attachment:") || r.len() > 200)
                {
                    return Err("files provenance".into());
                }
                if let Some(title) = &files.title {
                    check_text(title, 300, "files title")?;
                }
                if files.items.is_empty() || files.items.len() > 10 {
                    return Err(format!("{} files", files.items.len()));
                }
                for item in &files.items {
                    check_text(&item.id, 200, "file id")?;
                    check_text(&item.attachment_id, 200, "attachment id")?;
                    check_text(&item.name, 300, "file name")?;
                    check_text(&item.mime, 200, "file type")?;
                    if item.size_bytes < 0 {
                        return Err("a negative file size".into());
                    }
                    match &item.preview {
                        None | Some(super::FilePreview::Image) => {}
                        Some(super::FilePreview::Table { columns, rows, truncated, total_rows, total_cols }) => {
                            // A total only exists on a cut preview and is never below what is shown
                            if (total_rows.is_some() || total_cols.is_some()) && !*truncated {
                                return Err("file totals on a complete preview".into());
                            }
                            if total_rows.is_some_and(|n| n < rows.len() as u64) || total_cols.is_some_and(|n| n < columns.len() as u64) {
                                return Err("a file total below the preview".into());
                            }
                            if columns.is_empty() || columns.len() > 8 {
                                return Err(format!("{} file columns", columns.len()));
                            }
                            if rows.len() > 20 {
                                return Err(format!("{} file rows", rows.len()));
                            }
                            for c in columns {
                                if c.chars().count() > 300 || has_html(c) {
                                    return Err("a file column label".into());
                                }
                            }
                            for row in rows {
                                if row.len() != columns.len() {
                                    return Err("file row width".into());
                                }
                                for cell in row {
                                    if cell.chars().count() > 300 || has_html(cell) {
                                        return Err("a file cell".into());
                                    }
                                }
                            }
                        }
                        Some(super::FilePreview::Text { excerpt, truncated, total_lines }) => {
                            if let Some(n) = total_lines {
                                if !*truncated || *n < excerpt.lines().count() as u64 {
                                    return Err("a file line total".into());
                                }
                            }
                            if excerpt.chars().count() > 4000 || has_html(excerpt) {
                                return Err("a file excerpt".into());
                            }
                        }
                    }
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

/// Tables (lists) are indexed in their own count; `slot` has no index. A list's `ordered` absent
/// means numbered (v1).
fn check_source(block: &Block) -> Result<(), String> {
    let s = block.source();
    let kind = block.kind();
    let table_ok = matches!(kind, "comparison" | "chart" | "metrics");
    let list_ok = matches!(kind, "steps" | "metrics" | "facts" | "cards" | "findings");
    let ok = match s.kind.as_str() {
        "markdown_table" => table_ok && s.ordered.is_none() && s.index.is_some_and(|i| i <= 10_000),
        "markdown_list" => list_ok && s.index.is_some_and(|i| i <= 10_000),
        "slot" => matches!(kind, "verification" | "files") && s.index.is_none() && s.ordered.is_none(),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!("{kind} source"))
    }
}

/// A shown cell: at most 200 characters, no HTML; it may be empty (a gap).
fn check_cell(cell: &str) -> Result<(), String> {
    if cell.chars().count() > 200 {
        return Err("a cell over 200 characters".into());
    }
    if has_html(cell) {
        return Err("HTML in a cell".into());
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
