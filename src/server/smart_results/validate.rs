//! Checks every block set before it is stored, and again when a stored one is read back.

use std::collections::HashSet;

use super::items::severity_of;
use super::selector::plain;
use super::recipe::{
    GLYPHS, GROUP_MAX, INGREDIENTS, INGREDIENT_MAX, MAX_LEAD, MAX_NOTES, MAX_TIMES, NOTE_MAX, QTY_MAX, SERVES, STEPS, STEP_MAX,
    TIME_LABEL_MAX, TIME_TEXT_MAX,
};
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
        // `present_result` may declare a comparison, steps or recipe block itself (`bot_declared`,
        // grounded against the reply by its call); the typed task and file blocks come from kwant's data.
        let prov = block.provenance().kind.as_str();
        let prov_ok = match block {
            Block::Comparison(_) | Block::Steps(_) | Block::Recipe(_) => matches!(prov, "reply_text" | "bot_declared"),
            Block::Verification(_) | Block::Files(_) => prov == "kwant_data",
            _ => prov == "reply_text",
        };
        if !prov_ok {
            return Err(format!("{} provenance", block.kind()));
        }
        // Only text copied from the reply is checked against it.
        let strict = prov == "reply_text";
        match block {
            Block::Recipe(r) => {
                if let Some(n) = r.serves {
                    if !(SERVES.0..=SERVES.1).contains(&n) || reply.is_some_and(|reply| !reply.contains(&n.to_string())) {
                        return Err(format!("serves {n}"));
                    }
                }
                if r.times.len() > MAX_TIMES {
                    return Err(format!("{} times", r.times.len()));
                }
                for t in &r.times {
                    check_text(&t.label, TIME_LABEL_MAX, "time label")?;
                    check_text(&t.text, TIME_TEXT_MAX, "time")?;
                    texts.push(&t.label);
                    texts.push(&t.text);
                }
                if !(INGREDIENTS.0..=INGREDIENTS.1).contains(&r.ingredients.len()) {
                    return Err(format!("{} ingredients", r.ingredients.len()));
                }
                for (k, i) in r.ingredients.iter().enumerate() {
                    if i.id != format!("i{k}") {
                        return Err(format!("ingredient id {:?}", i.id));
                    }
                    check_text(&i.text, INGREDIENT_MAX, "ingredient")?;
                    if let Some(q) = &i.qty {
                        check_text(q, QTY_MAX, "qty")?;
                        if !q.starts_with(|c: char| c.is_ascii_digit() || GLYPHS.contains(&c)) {
                            return Err(format!("qty {q:?}"));
                        }
                    }
                    if let Some(g) = &i.group {
                        check_text(g, GROUP_MAX, "group")?;
                        texts.push(g);
                    }
                    // qty + text is the line as the reply wrote it
                    let line = i.line();
                    if reply.is_some_and(|reply| !reply.contains(&line)) {
                        return Err(format!("{line:?} is not text of the reply"));
                    }
                }
                if !(STEPS.0..=STEPS.1).contains(&r.steps.len()) {
                    return Err(format!("{} steps", r.steps.len()));
                }
                for (k, step) in r.steps.iter().enumerate() {
                    if step.id != format!("s{k}") {
                        return Err(format!("step id {:?}", step.id));
                    }
                    check_text(&step.text, STEP_MAX, "step")?;
                    texts.push(&step.text);
                }
                if r.notes.len() > MAX_NOTES {
                    return Err(format!("{} notes", r.notes.len()));
                }
                for n in &r.notes {
                    check_text(n, NOTE_MAX, "note")?;
                    texts.push(n);
                }
            }
            Block::Comparison(c) => {
                if !(2..=8).contains(&c.columns.len()) {
                    return Err(format!("{} columns", c.columns.len()));
                }
                for (k, col) in c.columns.iter().enumerate() {
                    if col.key != format!("c{k}") {
                        return Err(format!("column key {:?}", col.key));
                    }
                    check_label(&col.label, "column label")?;
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
                    check_label(&row.cells[0], "row label")?;
                    if !labels.insert(label_key(&row.cells[0])) {
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
                check_label(&c.x.label, "axis label")?;
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
                    check_label(&s.label, "series label")?;
                    texts.push(&s.label);
                    if let Some(unit) = &s.unit {
                        check_text(unit, 10, "unit")?;
                        texts.push(unit);
                    }
                }
                // One shared axis means one unit; the client draws no chart whose series differ
                if c.series.iter().any(|s| s.unit != c.series[0].unit) {
                    return Err("series in more than one unit".into());
                }
                if !(3..=60).contains(&c.points.len()) || c.chart_kind == ChartKind::Bar && c.points.len() < 5 {
                    return Err(format!("{} points", c.points.len()));
                }
                let mut xs = HashSet::new();
                for (k, p) in c.points.iter().enumerate() {
                    if p.id != format!("p{k}") {
                        return Err(format!("point id {:?}", p.id));
                    }
                    check_label(&p.x, "point label")?;
                    if !xs.insert(p.x.as_str()) {
                        return Err(format!("point {:?} repeats", p.x));
                    }
                    texts.push(&p.x);
                    if p.cells.len() != c.series.len() || p.values.len() != c.series.len() {
                        return Err(format!("point {k} has {} cells, {} values", p.cells.len(), p.values.len()));
                    }
                    if p.values.iter().flatten().any(|v| !in_range(*v)) {
                        return Err("a value that isn't a number or is out of range".into());
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
                    if !in_range(x.value) {
                        return Err("a value that isn't a number or is out of range".into());
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
                            && !url.contains(|c: char| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '"' | '\'' | '`'))
                            && has_plain_host(url);
                        if !safe {
                            return Err("a card link that isn't a plain https link".into());
                        }
                        texts.push(url);
                    }
                    if x.meta.len() > 4 {
                        return Err("card meta".into());
                    }
                    for m in &x.meta {
                        check_text(m, 40, "card meta")?;
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
    let section_only = s.last_index.is_none() && s.last_ordered.is_none() && s.lead.is_none();
    let ok = match s.kind.as_str() {
        "markdown_table" => table_ok && section_only && s.ordered.is_none() && s.index.is_some_and(|i| i <= 10_000),
        "markdown_list" => list_ok && section_only && s.index.is_some_and(|i| i <= 10_000),
        "markdown_section" => {
            kind == "recipe"
                && s.ordered.is_some()
                && s.last_ordered.is_some()
                && s.lead.is_some_and(|l| l <= MAX_LEAD)
                && match (s.index, s.last_index) {
                    // The section can't end before it starts
                    (Some(a), Some(b)) => a <= 10_000 && b <= 10_000 && (s.ordered != s.last_ordered || a <= b),
                    _ => false,
                }
        }
        "slot" => matches!(kind, "verification" | "files") && section_only && s.index.is_none() && s.ordered.is_none(),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!("{kind} source"))
    }
}

/// A shown cell: no HTML, within [`cell_fits`]; it may be empty (a gap).
fn check_cell(cell: &str) -> Result<(), String> {
    if !cell_fits(cell) {
        return Err(format!("a cell over {MAX_CELL} characters"));
    }
    if has_html(cell) {
        return Err("HTML in a cell".into());
    }
    Ok(())
}

/// A table label (a header or a row's first cell): a cell that isn't blank.
fn check_label(s: &str, what: &str) -> Result<(), String> {
    if s.trim().is_empty() {
        return Err(format!("an empty {what}"));
    }
    check_cell(s).map_err(|e| e.replace("cell", what))
}

/// The most a table cell may show, in characters.
pub(super) const MAX_CELL: usize = 200;
/// The most a table cell may hold raw, URLs included, so a long link can't bloat a stored block.
pub(super) const MAX_CELL_RAW: usize = 1000;

/// A table cell short enough to store: at most [`MAX_CELL`] shown characters ([`shown_len`]) and
/// [`MAX_CELL_RAW`] raw ones. Lengths are UTF-16 units, as the client counts (`isCell`).
pub(super) fn cell_fits(cell: &str) -> bool {
    cell.encode_utf16().count() <= MAX_CELL_RAW && shown_len(cell) <= MAX_CELL
}

/// A cell's length as the reader sees it: a markdown link `[text](url)` (or image `![text](url)`)
/// counts as its text, since a source link's URL isn't shown. A bare URL is shown, so it counts.
/// The client's `shownLength` does the same (`/!?\[([^\]]*)\]\([^)]*\)/g` → `$1`).
pub(super) fn shown_len(s: &str) -> usize {
    let mut n = 0;
    let mut rest = s;
    while !rest.is_empty() {
        let at = rest.find('[').unwrap_or(rest.len());
        let (before, from) = rest.split_at(at);
        // The `!` of an image goes with its link
        let link = link_text(from);
        let bang = link.is_some() && before.ends_with('!');
        n += before.encode_utf16().count() - usize::from(bang);
        match link {
            Some((text, len)) => {
                n += text.encode_utf16().count();
                rest = &from[len..];
            }
            None if from.is_empty() => break,
            None => {
                n += 1;
                rest = &from[1..];
            }
        }
    }
    n
}

/// `[text](url)` at the start of `s`: the text and the link's length in bytes.
fn link_text(s: &str) -> Option<(&str, usize)> {
    let body = s.strip_prefix('[')?;
    let close = body.find(']')?;
    let after = body[close + 1..].strip_prefix('(')?;
    let end = after.find(')')?;
    Some((&body[..close], 1 + close + 2 + end + 1))
}

fn check_text(s: &str, max: usize, what: &str) -> Result<(), String> {
    if s.trim().is_empty() {
        return Err(format!("an empty {what}"));
    }
    if utf16_len(s) > max {
        return Err(format!("a {what} over {max} characters"));
    }
    if has_html(s) {
        return Err(format!("HTML in a {what}"));
    }
    Ok(())
}

/// A text's length as the client counts it (UTF-16 units: an emoji is two), so a text the server
/// stores is one the client's own limit lets through.
pub(super) fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// The largest number a value may be: what an SVG coordinate carries without turning every scaled
/// position into NaN (the client's own bound, `MAX_VALUE` in `Chart.tsx`); below
/// `Number.MAX_SAFE_INTEGER`.
const MAX_VALUE: f64 = 1e15;

fn in_range(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_VALUE
}

/// A row label as the client compares them: markdown out, whitespace collapsed, lowercase.
fn label_key(label: &str) -> String {
    plain(label).split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A URL (already `https://`) with a host of its own and no login in front of it
/// (`https://user:pw@host/`, `https://host@evil/`): an `@` after the host is part of the path.
pub(super) fn has_plain_host(url: &str) -> bool {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(url);
    let authority = rest.split(['/', '?', '#', '\\']).next().unwrap_or("");
    let host = authority.split(':').next().unwrap_or("");
    !authority.contains('@') && !host.is_empty() && !url.contains('\\')
}

/// Anything that could open an HTML tag, comment or declaration (`<b`, `</`, `<!`, `<?`).
pub(super) fn has_html(s: &str) -> bool {
    s.char_indices().any(|(i, c)| {
        c == '<' && s[i + 1..].chars().next().is_some_and(|n| n.is_ascii_alphabetic() || matches!(n, '/' | '!' | '?'))
    })
}
