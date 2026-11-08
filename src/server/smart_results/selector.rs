//! The reader and the selectors: every table and top-level list of a reply that could be shown as
//! a block, in priority order.
//!
//! A line-by-line reading of CommonMark + GFM that is exact for the shapes bots write (top-level
//! pipe tables, numbered and bullet lists, fenced code) and abstains on the rest. `index` must name
//! the same table or list the chat's remark-gfm renders (smart-results-goals/index-rule.md), so
//! anything that could make a count uncertain (a table or list inside a quote or an item, raw
//! HTML, tabs, a table run straight into a paragraph or another block, footnotes) marks that count
//! uncertain from that line on, and no block of that kind is offered at or after it.

use std::cell::OnceCell;
use std::collections::HashSet;

use super::validate::{cell_fits, has_html};
use super::{chart, items};
use super::{Block, Column, Comparison, Provenance, Role, Row, Source, Steps, Variant, MAX_REPLY, PICK_REASON};

/// The order types win in when a reply has more than one candidate (plan §A.2), best first.
/// `present_result`, `verification` and `files` come from typed sources, not from these selectors.
pub const PRIORITY: &[&str] = &[
    "present_result",
    "verification",
    "recipe",
    "chart",
    "comparison",
    "metrics",
    "findings",
    "cards",
    "steps",
    "facts",
    "files",
];

/// Where `block` stands in [`PRIORITY`] (lower wins); steps rank timeline, then checklist, then
/// plain steps.
pub fn rank(block: &Block) -> (usize, u8) {
    let at = PRIORITY.iter().position(|t| *t == block.kind()).unwrap_or(PRIORITY.len());
    let sub = match block {
        Block::Steps(s) => match s.variant {
            Variant::Timeline => 0,
            Variant::Checklist => 1,
            Variant::Steps => 2,
        },
        _ => 0,
    };
    (at, sub)
}

/// Every block `text` could show, unchecked (see [`super::compute`]): each qualifying table and
/// top-level list as the one type it reads as, sorted by [`rank`], earliest in the text on a tie.
pub fn candidates(text: &str) -> Vec<Block> {
    if text.len() > MAX_REPLY {
        return Vec::new();
    }
    let doc = parse(text);
    // Read the prose for a pick once, and only when a table needs it
    let said = OnceCell::new();
    let said = || said.get_or_init(|| recommending(&doc.prose)).as_deref();
    let mut found: Vec<(usize, Block)> = Vec::new();
    if let Some((start, r)) = super::recipe::recipe(&doc) {
        found.push((start, Block::Recipe(r)));
    }
    let tables = doc.tables.iter().filter(|t| t.clean && t.start < doc.tables_unsure_at).take(MAX_TRIES);
    for t in tables {
        if let Some(b) = table_block(t, &said) {
            found.push((t.start, b));
        }
    }
    let lists = doc.lists.iter().filter(|l| l.clean && l.start < doc.list_unsure_at(l.ordered)).take(MAX_TRIES);
    for l in lists {
        if let Some(b) = list_block(l) {
            found.push((l.start, b));
        }
    }
    found.sort_by_key(|(start, b)| (rank(b), *start));
    found.into_iter().map(|(_, b)| b).collect()
}

/// A table of the reply as the chat counts it: its `index`, header and rows, verbatim.
pub struct ReplyTable {
    pub index: usize,
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// An ordered list of the reply as the chat counts it: its `index` and item texts, verbatim.
pub struct ReplyList {
    pub index: usize,
    pub items: Vec<String>,
}

/// Every table whose index is certain (none at or after the first line that could make the count
/// differ from the chat's), for checking a `present_result` call's `source` against the reply.
pub fn tables_of(text: &str) -> Vec<ReplyTable> {
    if text.len() > MAX_REPLY {
        return Vec::new();
    }
    let doc = parse(text);
    doc.tables
        .iter()
        .filter(|t| t.clean && t.start < doc.tables_unsure_at)
        .map(|t| ReplyTable {
            index: t.index,
            header: t.header.iter().map(|c| c.to_string()).collect(),
            rows: t.rows.iter().map(|r| r.iter().map(|c| c.to_string()).collect()).collect(),
        })
        .collect()
}

/// Every ordered list whose index is certain; see [`tables_of`].
pub fn lists_of(text: &str) -> Vec<ReplyList> {
    if text.len() > MAX_REPLY {
        return Vec::new();
    }
    let doc = parse(text);
    doc.lists
        .iter()
        .filter(|l| l.ordered && l.clean && l.start < doc.ol_unsure_at)
        .map(|l| ReplyList { index: l.index, items: l.items.iter().map(|c| c.to_string()).collect() })
        .collect()
}

/// A table as the one type it reads as: a line chart, a bar chart, metrics, else a comparison.
fn table_block<'s>(t: &Table, said: &impl Fn() -> Option<&'s [String]>) -> Option<Block> {
    if let Some(c) = chart::line(t) {
        return Some(Block::Chart(c));
    }
    if let Some(c) = chart::bar(t) {
        // A named pick makes it a comparison
        let labels: Vec<&str> = t.rows.iter().map(|r| r[0]).collect();
        if said().and_then(|said| pick(&labels, said)).is_none() {
            return Some(Block::Chart(c));
        }
    }
    if let Some(m) = chart::metrics_table(t) {
        return Some(Block::Metrics(m));
    }
    comparison(t, said()).map(Block::Comparison)
}

/// A top-level list as the one type it reads as, in priority order.
fn list_block(l: &List) -> Option<Block> {
    items::metrics(l)
        .map(Block::Metrics)
        .or_else(|| items::findings(l).map(Block::Findings))
        .or_else(|| items::cards(l).map(Block::Cards))
        .or_else(|| items::timeline(l).map(Block::Steps))
        .or_else(|| items::checklist(l).map(Block::Steps))
        .or_else(|| steps(l).map(Block::Steps))
        .or_else(|| items::facts(l).map(Block::Facts))
}

/// Tables (lists) looked at per reply: a reply with more doesn't cost more.
const MAX_TRIES: usize = 8;

// Reading the markdown

pub(super) struct Table<'a> {
    pub index: usize,
    pub start: usize,
    pub clean: bool,
    pub header: Vec<&'a str>,
    pub rows: Vec<Vec<&'a str>>,
}

pub(super) struct List<'a> {
    /// Counted among lists of its own kind (numbered or bullet).
    pub index: usize,
    pub ordered: bool,
    pub start: usize,
    /// One line per item, no item empty, nothing nested.
    pub clean: bool,
    /// `.` or `)` for a numbered list, `-`, `*` or `+` for a bullet list.
    pub delim: u8,
    /// The first item's number; 0 for a bullet list.
    pub number: u64,
    pub items: Vec<&'a str>,
    /// The non-blank line just before the list, and the latest heading above it.
    pub intro: Option<&'a str>,
    pub heading: Option<&'a str>,
}

/// A top-level block of the reply, in the chat's order of root children.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Top<'a> {
    /// A one-line heading (ATX, or setext over one line), its text.
    Heading(&'a str),
    /// A paragraph: its first and last line.
    Para { start: usize, end: usize },
    /// `Doc::lists[n]`.
    List(usize),
    /// Code, a table, a quote, HTML, a rule or a heading over several lines.
    Other,
}

#[derive(Default)]
pub(super) struct Doc<'a> {
    pub lines: Vec<&'a str>,
    pub blocks: Vec<Top<'a>>,
    /// The line each of `blocks` starts on.
    pub starts: Vec<usize>,
    pub tables: Vec<Table<'a>>,
    pub lists: Vec<List<'a>>,
    /// Lines outside code, HTML and tables, where a pick can be named.
    prose: Vec<&'a str>,
    /// The first line from which table (numbered list, bullet list) counts may differ from the
    /// chat's.
    tables_unsure_at: usize,
    ol_unsure_at: usize,
    ul_unsure_at: usize,
    ol_count: usize,
    ul_count: usize,
}

impl<'a> Doc<'a> {
    fn push(&mut self, line: usize, top: Top<'a>) {
        self.blocks.push(top);
        self.starts.push(line);
    }
}

impl Doc<'_> {
    fn unsure_tables(&mut self, line: usize) {
        self.tables_unsure_at = self.tables_unsure_at.min(line);
    }
    fn unsure_ol(&mut self, line: usize) {
        self.ol_unsure_at = self.ol_unsure_at.min(line);
    }
    fn unsure_ul(&mut self, line: usize) {
        self.ul_unsure_at = self.ul_unsure_at.min(line);
    }
    fn unsure(&mut self, line: usize) {
        self.unsure_tables(line);
        self.unsure_ol(line);
        self.unsure_ul(line);
    }
    /// Nested lists of either kind start in `s` (after quote and list markers).
    fn unsure_nested(&mut self, line: usize, s: &str) {
        if has_ordered_marker(s) {
            self.unsure_ol(line);
        }
        if has_bullet_marker(s) {
            self.unsure_ul(line);
        }
    }
    /// The first line from which any count may differ from the chat's.
    pub(super) fn unsure_at(&self) -> usize {
        self.tables_unsure_at.min(self.ol_unsure_at).min(self.ul_unsure_at)
    }
    pub(super) fn list_unsure_at(&self, ordered: bool) -> usize {
        if ordered {
            self.ol_unsure_at
        } else {
            self.ul_unsure_at
        }
    }
    fn new_list<'a>(&mut self, line: usize, ordered: bool, delim: u8, number: u64, first: &'a str) -> List<'a> {
        let count = if ordered { &mut self.ol_count } else { &mut self.ul_count };
        let index = *count;
        *count += 1;
        List { index, ordered, start: line, clean: !first.is_empty(), delim, number, items: vec![first], intro: None, heading: None }
    }
}

#[derive(Clone, Copy)]
enum State {
    None,
    Para,
    Table(usize),
    /// A top-level ordered list; `offset` is the current item's content column.
    List { list: usize, offset: usize, blank: bool },
    /// A top-level bullet list.
    Bullet { list: usize, offset: usize, blank: bool },
    Quote,
    Html,
    Code,
}

struct Fence {
    ch: u8,
    len: usize,
    /// The content column of the list item holding it; `None` at the top level.
    inside: Option<usize>,
}

pub(super) fn parse(text: &str) -> Doc<'_> {
    let lines: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    let mut doc = Doc { tables_unsure_at: usize::MAX, ol_unsure_at: usize::MAX, ul_unsure_at: usize::MAX, ..Doc::default() };
    // The chat takes the last `[DECISIONS]` block still in a reply out of it, code or not, and
    // trims and rejoins the rest (frontend/src/decideFirst.ts takeDecisionsBlock): no line of it
    // is sure to read the same there
    if lines.iter().any(|l| l.to_ascii_lowercase().contains("[decisions]")) {
        doc.unsure(0);
    }
    let mut state = State::None;
    let mut fence: Option<Fence> = None;
    let mut heading: Option<&str> = None;
    let mut prev: Option<&str> = None;
    let mut delimiter_row: Option<usize> = None;
    for (i, &line) in lines.iter().enumerate() {
        let (indent, rest) = lead(line);
        let blank = rest.trim().is_empty();
        if let Some(f) = &fence {
            match f.inside {
                Some(offset) if !blank && indent < offset => {
                    // The item ends and closes the fence with it; whether the chat agrees is
                    // more than this reader knows.
                    doc.unsure(i);
                    fence = None;
                }
                Some(_) => {
                    if fence_closes(rest, f) {
                        fence = None;
                    }
                    if !blank {
                        prev = Some(line);
                    }
                    continue;
                }
                None => {
                    if indent <= 3 && fence_closes(rest, f) {
                        fence = None;
                    }
                    if !blank {
                        prev = Some(line);
                    }
                    continue;
                }
            }
        }
        if delimiter_row == Some(i) {
            delimiter_row = None;
            prev = Some(line);
            continue;
        }
        if line.contains('\t') {
            doc.unsure(i);
        }
        let mut prose = !blank;
        let mut in_table = false;
        let mut code = false;
        loop {
            match state {
                State::Table(t) => {
                    if blank {
                        state = State::None;
                    } else if indent >= 4 || block_start(rest) {
                        // GFM ends the table here, or takes the line as a row: not sure which
                        doc.unsure(i);
                        doc.tables[t].clean = false;
                        state = State::None;
                        continue;
                    } else {
                        let cells = split_cells(rest);
                        let table = &mut doc.tables[t];
                        if !has_pipe(rest) || cells.len() != table.header.len() {
                            table.clean = false;
                        }
                        table.rows.push(cells);
                        prose = false;
                        in_table = true;
                    }
                }
                State::List { list, offset, blank: after_blank } | State::Bullet { list, offset, blank: after_blank } => {
                    let ordered = matches!(state, State::List { .. });
                    let at = |list, offset, blank| {
                        if ordered {
                            State::List { list, offset, blank }
                        } else {
                            State::Bullet { list, offset, blank }
                        }
                    };
                    if blank {
                        state = at(list, offset, true);
                        break;
                    }
                    if indent >= offset {
                        // More of the current item: a second paragraph, nested blocks or code
                        doc.lists[list].clean = false;
                        let inner = after_cols(line, offset);
                        doc.unsure_nested(i, inner);
                        if let Some((ch, len)) = fence_open(inner.trim_start()) {
                            fence = Some(Fence { ch, len, inside: Some(offset) });
                        }
                        state = at(list, offset, false);
                        break;
                    }
                    if indent <= 3 {
                        if thematic_break(rest) {
                            state = State::None;
                            continue;
                        }
                        let item = if ordered {
                            ordered_marker(rest, indent).map(|m| (m.delim, m.content, m.offset))
                        } else {
                            bullet_marker(rest, indent).map(|b| (rest.as_bytes()[0], b.content, b.offset))
                        };
                        if let Some((delim, content, item_offset)) = item {
                            if delim == doc.lists[list].delim {
                                let l = &mut doc.lists[list];
                                l.items.push(content);
                                if content.is_empty() {
                                    l.clean = false;
                                }
                                doc.unsure_nested(i, content);
                                fence = item_fence(content, item_offset);
                                if fence.is_some() {
                                    doc.lists[list].clean = false;
                                }
                                state = at(list, item_offset, false);
                                break;
                            }
                            // `1.` then `1)`, or `-` then `*`: a new list
                            state = State::None;
                            continue;
                        }
                        if !ordered {
                            if let Some(m) = ordered_marker(rest, indent) {
                                if after_blank || (m.number == 1 && !m.content.is_empty()) {
                                    state = State::None;
                                    continue;
                                }
                                // Lazy text, or a new list that ends this one: CommonMark readers differ
                                doc.unsure_ol(i);
                                doc.unsure_ul(i);
                                doc.lists[list].clean = false;
                                state = at(list, offset, false);
                                break;
                            }
                        }
                        if (ordered && bullet_marker(rest, indent).is_some())
                            || atx_heading(rest).is_some()
                            || fence_open(rest).is_some()
                            || rest.starts_with('>')
                            || html_start(rest)
                        {
                            state = State::None;
                            continue;
                        }
                    }
                    if after_blank {
                        state = State::None;
                        continue;
                    }
                    // A lazy continuation line of the item's paragraph
                    doc.lists[list].clean = false;
                    doc.unsure_nested(i, rest);
                    state = at(list, offset, false);
                }
                State::Quote => {
                    if blank {
                        state = State::None;
                    } else if indent <= 3 && rest.starts_with('>') {
                        doc.unsure_nested(i, rest);
                    } else if indent <= 3 && block_start(rest) {
                        if let Some(m) = ordered_marker(rest, indent) {
                            if m.number != 1 || m.content.is_empty() {
                                doc.unsure_ol(i);
                                break;
                            }
                        }
                        if bullet_marker(rest, indent).is_some_and(|b| b.content.is_empty()) {
                            doc.unsure_ul(i);
                            break;
                        }
                        state = State::None;
                        continue;
                    }
                }
                State::Html => {
                    prose = false;
                    if blank {
                        state = State::None;
                    }
                }
                State::Code => {
                    if blank {
                        break;
                    }
                    if indent < 4 {
                        state = State::None;
                        continue;
                    }
                    prose = false;
                    code = true;
                    doc.unsure_nested(i, rest);
                }
                State::Para => {
                    if blank {
                        state = State::None;
                        break;
                    }
                    if indent <= 3 {
                        if setext_underline(rest) {
                            if rest.contains('-') && prev.is_some_and(has_pipe) {
                                // `| a |` over `---`: a one-column table or a heading
                                doc.unsure_tables(i);
                            }
                            heading = prev.map(str::trim);
                            if let Some(top) = doc.blocks.last_mut() {
                                if let Top::Para { start, end } = *top {
                                    *top = match heading {
                                        Some(h) if start == end => Top::Heading(h),
                                        _ => Top::Other,
                                    };
                                }
                            }
                            state = State::None;
                            break;
                        }
                        if let Some(m) = ordered_marker(rest, indent) {
                            if m.number == 1 && !m.content.is_empty() {
                                state = State::None;
                                continue;
                            }
                            // Anything else continues the paragraph
                            break;
                        }
                        if thematic_break(rest) || atx_heading(rest).is_some() || fence_open(rest).is_some() || rest.starts_with('>') {
                            state = State::None;
                            continue;
                        }
                        if bullet_marker(rest, indent).is_some_and(|b| !b.content.is_empty()) {
                            state = State::None;
                            continue;
                        }
                        if html_start(rest) {
                            doc.unsure(i);
                            state = State::None;
                            continue;
                        }
                        if table_start(&lines, i).is_some() {
                            // A table straight after a paragraph line: GFM readers differ
                            doc.unsure(i);
                            state = State::None;
                            continue;
                        }
                    }
                }
                State::None => {
                    if blank {
                        break;
                    }
                    if indent >= 4 {
                        doc.push(i, Top::Other);
                        state = State::Code;
                        prose = false;
                        code = true;
                        doc.unsure_nested(i, rest);
                        if fence_open(rest).is_some() {
                            doc.unsure_ol(i);
                            doc.unsure_ul(i);
                        }
                        break;
                    }
                    if let Some((ch, len)) = fence_open(rest) {
                        doc.push(i, Top::Other);
                        fence = Some(Fence { ch, len, inside: None });
                        prose = false;
                        break;
                    }
                    if let Some(h) = atx_heading(rest) {
                        doc.push(i, Top::Heading(h));
                        heading = Some(h);
                        break;
                    }
                    if thematic_break(rest) {
                        doc.push(i, Top::Other);
                        break;
                    }
                    if rest.starts_with('>') {
                        doc.push(i, Top::Other);
                        doc.unsure_nested(i, rest);
                        state = State::Quote;
                        break;
                    }
                    if html_start(rest) {
                        doc.push(i, Top::Other);
                        doc.unsure(i);
                        prose = false;
                        state = State::Html;
                        break;
                    }
                    if let Some(m) = ordered_marker(rest, indent) {
                        doc.unsure_nested(i, m.content);
                        let mut l = doc.new_list(i, true, m.delim, m.number, m.content);
                        (l.intro, l.heading) = (prev, heading);
                        if fence_open(m.content).is_some() {
                            l.clean = false;
                        }
                        doc.push(i, Top::List(doc.lists.len()));
                        doc.lists.push(l);
                        fence = item_fence(m.content, m.offset);
                        state = State::List { list: doc.lists.len() - 1, offset: m.offset, blank: false };
                        break;
                    }
                    if let Some(b) = bullet_marker(rest, indent) {
                        doc.unsure_nested(i, b.content);
                        let mut l = doc.new_list(i, false, rest.as_bytes()[0], 0, b.content);
                        (l.intro, l.heading) = (prev, heading);
                        if fence_open(b.content).is_some() {
                            l.clean = false;
                        }
                        doc.push(i, Top::List(doc.lists.len()));
                        doc.lists.push(l);
                        fence = item_fence(b.content, b.offset);
                        state = State::Bullet { list: doc.lists.len() - 1, offset: b.offset, blank: false };
                        break;
                    }
                    if let Some(header) = table_start(&lines, i) {
                        let index = doc.tables.len();
                        doc.push(i, Top::Other);
                        doc.tables.push(Table { index, start: i, clean: true, header, rows: Vec::new() });
                        delimiter_row = Some(i + 1);
                        state = State::Table(index);
                        prose = false;
                        in_table = true;
                        break;
                    }
                    doc.push(i, Top::Para { start: i, end: i });
                    state = State::Para;
                }
            }
            break;
        }
        if matches!(state, State::Para) {
            if let Some(Top::Para { end, .. }) = doc.blocks.last_mut() {
                *end = i;
            }
        }
        if !in_table && !code && delimiter_like(line) {
            // A table this reader didn't take as top-level (in a quote or an item, or orphaned)
            doc.unsure_tables(i);
        }
        if !code && footnote_like(line) {
            doc.unsure(i);
        }
        if prose && !has_pipe(line) {
            doc.prose.push(line);
        }
        if !blank {
            prev = Some(line);
        }
    }
    doc.lines = lines;
    doc
}

/// A code fence opened on a list item's marker line, closed by the item's end or its own fence.
fn item_fence(content: &str, offset: usize) -> Option<Fence> {
    fence_open(content).map(|(ch, len)| Fence { ch, len, inside: Some(offset) })
}

/// The indentation in columns (a tab counts to the next multiple of 4) and the rest of the line.
fn lead(line: &str) -> (usize, &str) {
    let mut cols = 0;
    for (i, c) in line.char_indices() {
        match c {
            ' ' => cols += 1,
            '\t' => cols += 4 - cols % 4,
            _ => return (cols, &line[i..]),
        }
    }
    (cols, "")
}

/// `line` after its first `cols` columns of indentation.
fn after_cols(line: &str, cols: usize) -> &str {
    let mut seen = 0;
    for (i, c) in line.char_indices() {
        if seen >= cols {
            return &line[i..];
        }
        match c {
            ' ' => seen += 1,
            '\t' => seen += 4 - seen % 4,
            _ => return &line[i..],
        }
    }
    ""
}

/// A line that starts a block of another kind (given indentation of at most 3).
fn block_start(rest: &str) -> bool {
    rest.starts_with('>')
        || atx_heading(rest).is_some()
        || fence_open(rest).is_some()
        || thematic_break(rest)
        || bullet_marker(rest, 0).is_some()
        || ordered_marker(rest, 0).is_some()
        || html_start(rest)
}

fn atx_heading(rest: &str) -> Option<&str> {
    let hashes = rest.bytes().take_while(|b| *b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let after = &rest[hashes..];
    if !after.is_empty() && !after.starts_with([' ', '\t']) {
        return None;
    }
    Some(after.trim().trim_end_matches('#').trim_end())
}

fn thematic_break(rest: &str) -> bool {
    let mut marks = rest.chars().filter(|c| !matches!(c, ' ' | '\t'));
    let Some(first) = marks.next() else { return false };
    if !matches!(first, '-' | '*' | '_') {
        return false;
    }
    let mut n = 1;
    for c in marks {
        if c != first {
            return false;
        }
        n += 1;
    }
    n >= 3
}

fn setext_underline(rest: &str) -> bool {
    let t = rest.trim_end();
    !t.is_empty() && (t.bytes().all(|b| b == b'=') || t.bytes().all(|b| b == b'-'))
}

fn fence_open(rest: &str) -> Option<(u8, usize)> {
    let ch = *rest.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let len = rest.bytes().take_while(|b| *b == ch).count();
    if len < 3 || (ch == b'`' && rest[len..].contains('`')) {
        return None;
    }
    Some((ch, len))
}

fn fence_closes(rest: &str, fence: &Fence) -> bool {
    let len = rest.bytes().take_while(|b| *b == fence.ch).count();
    len >= fence.len && rest[len..].trim().is_empty()
}

fn html_start(rest: &str) -> bool {
    let mut chars = rest.chars();
    chars.next() == Some('<') && chars.next().is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?'))
}

struct Ordered<'a> {
    number: u64,
    delim: u8,
    /// The item's first line, trimmed; empty for an empty item.
    content: &'a str,
    /// The column its content starts at.
    offset: usize,
}

fn ordered_marker(rest: &str, indent: usize) -> Option<Ordered<'_>> {
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    let delim = *rest.as_bytes().get(digits)?;
    if delim != b'.' && delim != b')' {
        return None;
    }
    let after = &rest[digits + 1..];
    let number = rest[..digits].parse().ok()?;
    let marker = digits + 1;
    if after.trim().is_empty() {
        return Some(Ordered { number, delim, content: "", offset: indent + marker + 1 });
    }
    if !after.starts_with(' ') {
        return None;
    }
    let spaces = after.bytes().take_while(|b| *b == b' ').count();
    let offset = if spaces >= 5 { indent + marker + 1 } else { indent + marker + spaces };
    Some(Ordered { number, delim, content: after.trim(), offset })
}

struct Bullet<'a> {
    content: &'a str,
    offset: usize,
}

fn bullet_marker(rest: &str, indent: usize) -> Option<Bullet<'_>> {
    if !rest.starts_with(['-', '+', '*']) {
        return None;
    }
    let after = &rest[1..];
    if after.trim().is_empty() {
        return Some(Bullet { content: "", offset: indent + 2 });
    }
    if !after.starts_with(' ') {
        return None;
    }
    let spaces = after.bytes().take_while(|b| *b == b' ').count();
    let offset = if spaces >= 5 { indent + 2 } else { indent + 1 + spaces };
    Some(Bullet { content: after.trim(), offset })
}

/// Whether a numbered list starts in `s`, after any quote and bullet markers: one this reader
/// doesn't count, so the chat's count may differ from here on.
fn has_ordered_marker(s: &str) -> bool {
    let mut s = s.trim_start();
    for _ in 0..32 {
        if let Some(r) = s.strip_prefix('>') {
            s = r.trim_start();
        } else if let Some(b) = bullet_marker(s, 0).filter(|b| !b.content.is_empty()) {
            s = b.content;
        } else {
            return ordered_marker(s, 0).is_some();
        }
    }
    true
}

/// Whether a bullet list starts in `s`, after any quote and numbered-list markers.
fn has_bullet_marker(s: &str) -> bool {
    let mut s = s.trim_start();
    for _ in 0..32 {
        if let Some(r) = s.strip_prefix('>') {
            s = r.trim_start();
        } else if let Some(m) = ordered_marker(s, 0).filter(|m| !m.content.is_empty()) {
            s = m.content;
        } else {
            return !thematic_break(s) && bullet_marker(s, 0).is_some();
        }
    }
    true
}

fn has_pipe(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'|' => return true,
            _ => i += 1,
        }
    }
    false
}

/// The cells of a table row: split on unescaped pipes, outer pipes dropped, each trimmed. Cells
/// are verbatim markdown (an escaped `\|` stays escaped).
fn split_cells(line: &str) -> Vec<&str> {
    let s = line.trim();
    let s = s.strip_prefix('|').unwrap_or(s);
    let s = match s.strip_suffix('|') {
        Some(inner) if !inner.ends_with('\\') => inner,
        _ => s,
    };
    let bytes = s.as_bytes();
    let mut cells = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'|' => {
                cells.push(s[start..i].trim());
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    cells.push(s[start.min(s.len())..].trim());
    cells
}

/// A GFM delimiter row (`| --- | :-: |`): its cell count.
fn delimiter_cells(line: &str) -> Option<usize> {
    if !has_pipe(line) {
        return None;
    }
    let cells = split_cells(line);
    let ok = cells.iter().all(|c| {
        let c = c.strip_prefix(':').unwrap_or(c);
        let c = c.strip_suffix(':').unwrap_or(c);
        !c.is_empty() && c.bytes().all(|b| b == b'-')
    });
    ok.then_some(cells.len())
}

/// A line that looks like a delimiter row once indentation and quote or list markers are gone.
fn delimiter_like(line: &str) -> bool {
    let mut s = line.trim_start();
    for _ in 0..32 {
        if let Some(r) = s.strip_prefix('>') {
            s = r.trim_start();
        } else if let Some(r) = s.strip_prefix("- ").or_else(|| s.strip_prefix("* ")).or_else(|| s.strip_prefix("+ ")) {
            s = r.trim_start();
        } else if let Some(m) = ordered_marker(s, 0).filter(|m| !m.content.is_empty()) {
            s = m.content;
        } else {
            break;
        }
    }
    if !s.contains('|') {
        // A one-column table needs no pipe when its delimiter has a colon (`Pro` over `--:`);
        // `---` under a line is a heading
        let cell = s.trim();
        let dashes = cell.strip_prefix(':').unwrap_or(cell);
        let dashes = dashes.strip_suffix(':').unwrap_or(dashes);
        return cell.contains(':') && !dashes.is_empty() && dashes.bytes().all(|b| b == b'-');
    }
    s.contains('-') && s.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

/// A footnote definition (`[^1]: ...`) once indentation and quote or list markers are gone. The
/// client numbers what is inside it where it stands, but renders it at the end of the reply.
fn footnote_like(line: &str) -> bool {
    let mut s = line.trim_start();
    for _ in 0..32 {
        if let Some(r) = s.strip_prefix('>') {
            s = r.trim_start();
        } else if let Some(r) = s.strip_prefix("- ").or_else(|| s.strip_prefix("* ")).or_else(|| s.strip_prefix("+ ")) {
            s = r.trim_start();
        } else if let Some(m) = ordered_marker(s, 0).filter(|m| !m.content.is_empty()) {
            s = m.content;
        } else {
            break;
        }
    }
    s.strip_prefix("[^").is_some_and(|r| r.contains("]:"))
}

/// The header cells when lines `i` and `i + 1` open a GFM table.
fn table_start<'a>(lines: &[&'a str], i: usize) -> Option<Vec<&'a str>> {
    let header = lines[i];
    let delimiter = *lines.get(i + 1)?;
    let (h_indent, h_rest) = lead(header);
    let (d_indent, d_rest) = lead(delimiter);
    if h_indent > 3 || d_indent > 3 || !has_pipe(h_rest) {
        return None;
    }
    // `- | -` is a bullet item to the client, not a delimiter row
    if d_rest.starts_with("- ") || d_rest.starts_with("-\t") {
        return None;
    }
    let cells = split_cells(h_rest);
    (delimiter_cells(delimiter)? == cells.len()).then_some(cells)
}

// Comparison

/// `said`: the reply's recommending sentences ([`recommending`]); `None` means no row is
/// highlighted.
fn comparison(t: &Table, said: Option<&[String]>) -> Option<Comparison> {
    let cols = t.header.len();
    if !t.clean || !(2..=8).contains(&cols) || !(3..=50).contains(&t.rows.len()) {
        return None;
    }
    // A cell is measured by what it shows: a source link counts as its text (`cell_fits`)
    if t.header.iter().any(|h| h.is_empty() || !cell_fits(h)) {
        return None;
    }
    if t.rows.iter().any(|r| r.len() != cols || r.iter().any(|c| !cell_fits(c))) {
        return None;
    }
    // Cells are shown as text, so a table with HTML in it stays markdown
    if t.header.iter().chain(t.rows.iter().flatten()).any(|c| has_html(c)) {
        return None;
    }
    let mut seen = HashSet::new();
    for row in &t.rows {
        let key: String = plain(row[0]).trim().chars().map(fold).collect();
        if key.is_empty() || !seen.insert(key) {
            return None;
        }
    }
    let columns = (0..cols)
        .map(|c| {
            let (numeric, unit) = numeric_column(t.rows.iter().map(|r| r[c]));
            let role = if c == 0 { Role::Label } else { header_role(t.header[c]) };
            Column { key: format!("c{c}"), label: t.header[c].to_string(), numeric, unit, role: Some(role) }
        })
        .collect();
    let rows = t
        .rows
        .iter()
        .enumerate()
        .map(|(i, r)| Row { id: format!("r{i}"), cells: r.iter().map(|c| c.to_string()).collect() })
        .collect();
    let labels: Vec<&str> = t.rows.iter().map(|r| r[0]).collect();
    let pick = said.and_then(|said| pick(&labels, said));
    Some(Comparison {
        id: "b0".into(),
        title: None,
        source: Source::table(t.index),
        columns,
        rows,
        highlight_row: pick.map(|r| format!("r{r}")),
        highlight_reason: pick.map(|_| PICK_REASON.to_string()),
        provenance: Provenance::reply_text(),
    })
}

/// Pros / Cons / Advantages / Drawbacks / Notes headers name what a column holds.
fn header_role(header: &str) -> Role {
    match plain(header).trim().to_lowercase().as_str() {
        "pros" | "advantages" => Role::Pros,
        "cons" | "drawbacks" | "disadvantages" => Role::Cons,
        "notes" | "note" => Role::Note,
        _ => Role::Value,
    }
}

/// Numeric when at least 80% of the cells are a number with the same prefix or suffix (that
/// affix is the unit; none counts as an affix too).
pub(super) fn numeric_column<'a>(cells: impl Iterator<Item = &'a str>) -> (bool, Option<String>) {
    let mut counts: Vec<(Option<&str>, usize)> = Vec::new();
    let mut total = 0;
    for cell in cells {
        total += 1;
        let Some(affix) = number_affix(cell) else { continue };
        match counts.iter_mut().find(|(a, _)| *a == affix) {
            Some((_, n)) => *n += 1,
            None => counts.push((affix, 1)),
        }
    }
    let mut best: Option<(Option<&str>, usize)> = None;
    for (affix, n) in counts {
        if best.is_none_or(|(_, b)| n > b) {
            best = Some((affix, n));
        }
    }
    match best {
        Some((affix, n)) if total > 0 && n * 5 >= total * 4 => (true, affix.map(str::to_string)),
        _ => (false, None),
    }
}

/// `Some(affix)` when `cell` is a number with at most one affix: a prefix of up to 4 letters or
/// currency signs (`EUR 12`, `$12`) or a suffix of up to 10 (`12 EUR`, `-4%`, `4.5/5`).
pub(super) fn number_affix(cell: &str) -> Option<Option<&str>> {
    number_parts(cell).map(|(affix, _)| affix)
}

/// [`number_affix`], and the number as written (`1,299.00`, `-4`).
pub(super) fn number_parts(cell: &str) -> Option<(Option<&str>, &str)> {
    let mut s = cell.trim();
    loop {
        let before = s;
        for wrap in ["**", "__", "*", "_", "`"] {
            if s.len() > 2 * wrap.len() && s.starts_with(wrap) && s.ends_with(wrap) {
                s = s[wrap.len()..s.len() - wrap.len()].trim();
            }
        }
        if s == before {
            break;
        }
    }
    let start = s.char_indices().find_map(|(i, c)| {
        let signed = matches!(c, '+' | '-' | '−') && s[i + c.len_utf8()..].starts_with(|d: char| d.is_ascii_digit());
        (c.is_ascii_digit() || signed).then_some(i)
    })?;
    let mut end = start + s[start..].chars().next()?.len_utf8();
    end += s[end..].bytes().take_while(|b| b.is_ascii_digit() || *b == b'.' || *b == b',').count();
    let number = s[start..end].trim_end_matches(['.', ',']);
    let end = start + number.len();
    if !is_number(number) {
        return None;
    }
    let (before, after) = (&s[..start], &s[end..]);
    let prefix = before.trim_end();
    let suffix = after.trim_start();
    if before.len() - prefix.len() > 1 || after.len() - suffix.len() > 1 {
        return None;
    }
    match (prefix.is_empty(), suffix.is_empty()) {
        (true, true) => Some((None, number)),
        (false, true) => {
            let ok = prefix.chars().count() <= 4
                && prefix.chars().all(|c| c.is_alphabetic() || currency(c) || c == '.')
                && prefix.chars().any(|c| c != '.');
            ok.then_some((Some(prefix), number))
        }
        (true, false) => {
            let out_of = suffix.strip_prefix('/').is_some_and(|d| (1..=3).contains(&d.len()) && d.bytes().all(|b| b.is_ascii_digit()));
            let unit = suffix.chars().all(|c| c.is_alphabetic() || currency(c) || "%‰°/²³µ'\".".contains(c))
                && suffix.chars().any(|c| c != '.');
            (suffix.chars().count() <= 10 && (out_of || unit)).then_some((Some(suffix), number))
        }
        (false, false) => None,
    }
}

pub(super) fn currency(c: char) -> bool {
    matches!(c, '$' | '¢' | '£' | '¤' | '¥' | '֏' | '؋' | '৳' | '฿' | '៛' | '﷼') || ('\u{20A0}'..='\u{20CF}').contains(&c)
}

/// `1299`, `12.5`, `12,5`, `1,299.00` or `1.299,00`, with an optional sign; at most 15 digits.
pub(super) fn is_number(n: &str) -> bool {
    let n = n.strip_prefix(['+', '-', '−']).unwrap_or(n);
    let digits = n.bytes().filter(u8::is_ascii_digit).count();
    if digits == 0 || digits > 15 {
        return false;
    }
    let all_digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let grouped = |group: char, decimal: char| {
        let (int, frac) = match n.split_once(decimal) {
            Some((int, frac)) => (int, Some(frac)),
            None => (n, None),
        };
        let mut parts = int.split(group);
        let first = parts.next().unwrap_or("");
        let rest: Vec<&str> = parts.collect();
        frac.is_none_or(all_digits)
            && (1..=3).contains(&first.len())
            && all_digits(first)
            && !rest.is_empty()
            && rest.iter().all(|p| p.len() == 3 && all_digits(p))
    };
    let simple = match n.split_once(['.', ',']) {
        Some((int, frac)) => all_digits(int) && all_digits(frac),
        None => all_digits(n),
    };
    simple || grouped(',', '.') || grouped('.', ',')
}

// The pick

const PICK_WORDS: [&str; 6] = ["best", "pick", "picks", "picked", "choose", "winner"];
/// Words that make a sentence too hedged or negative to name a pick.
const HEDGES: [&str; 23] = [
    "not", "never", "avoid", "no", "nor", "none", "without", "unless", "instead", "rather", "except", "skip", "worst",
    "worse", "if", "otherwise", "depends", "depending", "cannot", "second", "runner", "alternatively", "but",
];

/// More recommending text than this in a reply (in characters), or more label hits than
/// [`MAX_HITS`] while looking for a table's pick, and no row is highlighted: it keeps the search
/// cheap and the pick plain.
const MAX_SCANNED: usize = 8_000;
const MAX_HITS: usize = 1_000;

/// The sentences of the reply's prose that recommend something ([`recommends`]), without
/// emphasis, code marks or link targets; `None` when they run past [`MAX_SCANNED`] characters.
fn recommending(prose: &[&str]) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut total = 0;
    for line in prose {
        let line = plain(line);
        for sentence in sentences(&line) {
            if !recommends(sentence) {
                continue;
            }
            total += sentence.chars().count();
            if total > MAX_SCANNED {
                return None;
            }
            out.push(sentence.to_string());
        }
    }
    Some(out)
}

/// The one row whose first-column label a recommending sentence (`said`, outside the table)
/// names; `None` when no row or more than one is named.
fn pick(labels: &[&str], said: &[String]) -> Option<usize> {
    // Short labels match case exactly, longer ones ignore case
    let labels: Vec<(usize, String, bool)> = labels
        .iter()
        .enumerate()
        .filter_map(|(r, l)| {
            let l = plain(l);
            let l = l.trim();
            let folded: String = l.chars().map(fold).collect();
            let usable = l.chars().any(char::is_alphabetic)
                && !PICK_WORDS.contains(&folded.as_str())
                && !folded.starts_with("recommend")
                && folded != "go with";
            let exact = l.chars().count() < 4;
            usable.then(|| (r, if exact { l.to_string() } else { folded }, exact))
        })
        .collect();
    let mut named = HashSet::new();
    let mut hits = 0;
    for sentence in said {
        let folded: String = sentence.chars().map(fold).collect();
        let (at, folded_at) = (char_positions(sentence), char_positions(&folded));
        let mut found: Vec<(usize, usize, usize)> = Vec::new();
        for (r, label, exact) in &labels {
            let (h, pos) = if *exact { (sentence.as_str(), &at) } else { (folded.as_str(), &folded_at) };
            for (start, end) in occurrences(h, label, *exact, &mut hits)? {
                found.push((pos[start], pos[end], *r));
            }
        }
        // `Pro` inside `Pro Max` names `Pro Max`
        for &(start, end, r) in &found {
            let inside = found.iter().any(|&(s2, e2, _)| s2 <= start && end <= e2 && e2 - s2 > end - start);
            if !inside {
                named.insert(r);
            }
        }
    }
    (named.len() == 1).then(|| named.into_iter().next()).flatten()
}

/// The character index of each character's first byte in `s`, and of its end.
fn char_positions(s: &str) -> Vec<usize> {
    let mut out = vec![0; s.len() + 1];
    let mut n = 0;
    for (i, _) in s.char_indices() {
        out[i] = n;
        n += 1;
    }
    out[s.len()] = n;
    out
}

fn sentences(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = line.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?') && chars.peek().is_none_or(|(_, n)| n.is_whitespace()) {
            let end = i + c.len_utf8();
            out.push(&line[start..end]);
            start = end;
        }
    }
    out.push(&line[start..]);
    out.into_iter().map(str::trim).filter(|s| !s.is_empty()).collect()
}

/// A sentence that recommends something, and isn't a question or hedged.
fn recommends(sentence: &str) -> bool {
    if sentence.ends_with('?') {
        return false;
    }
    let lower: String = sentence.chars().map(fold).collect();
    let words: Vec<&str> = lower.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’')).filter(|w| !w.is_empty()).collect();
    if words.iter().any(|w| HEDGES.contains(w) || w.ends_with("n't") || w.ends_with("n’t")) {
        return false;
    }
    words.iter().any(|w| PICK_WORDS.contains(w) || w.starts_with("recommend")) || words.windows(2).any(|w| w == ["go", "with"])
}

/// Where `label` appears in `h` as a whole name, as byte ranges: neighbours may not be letters,
/// digits or `_ + # -`, and a following `.` must end the sentence (so `Node` doesn't match
/// `Node.js`). With `exact` (a short label matched case for case), a lone `A` or `I` opening the
/// sentence is the word, not the label. Every place `label` is found counts toward `hits`, whole
/// name or not; `None` past [`MAX_HITS`].
fn occurrences(h: &str, label: &str, exact: bool, hits: &mut usize) -> Option<Vec<(usize, usize)>> {
    let joins = |c: char| c.is_alphanumeric() || matches!(c, '_' | '+' | '#' | '-');
    let mut out = Vec::new();
    if label.is_empty() {
        return Some(out);
    }
    let mut from = 0;
    while let Some(at) = h[from..].find(label) {
        *hits += 1;
        if *hits > MAX_HITS {
            return None;
        }
        let start = from + at;
        let end = start + label.len();
        from = start + h[start..].chars().next().map_or(1, char::len_utf8);
        if h[..start].chars().next_back().is_some_and(joins) {
            continue;
        }
        let mut after = h[end..].chars();
        if let Some(next) = after.next() {
            if joins(next) || (next == '.' && after.next().is_some_and(|c| !c.is_whitespace())) {
                continue;
            }
        }
        if exact && matches!(label, "A" | "I") && h[..start].chars().all(|c| !c.is_alphanumeric()) {
            continue;
        }
        out.push((start, end));
    }
    Some(out)
}

/// Text without emphasis or code marks, links reduced to their text: for matching only.
pub(super) fn plain(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    // Once no `)` follows, none will: don't look again (keeps this linear)
    let mut closes = true;
    while i < chars.len() {
        let c = chars[i];
        if closes && c == ']' && chars.get(i + 1) == Some(&'(') {
            match chars[i + 2..].iter().position(|&x| x == ')') {
                Some(close) => {
                    i += close + 3;
                    continue;
                }
                None => closes = false,
            }
        }
        if !matches!(c, '*' | '`' | '[' | ']') {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// One lowercase character for one character, so positions line up.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

// Steps

const STEP_HINTS: [&str; 5] = ["step", "steps", "checklist", "plan", "todo"];
/// Intro words that make a numbered list something other than steps to follow.
const NOT_STEPS: [&str; 24] = [
    "reason", "reasons", "why", "benefit", "benefits", "advantage", "advantages", "tip", "tips", "example", "examples",
    "idea", "ideas", "pros", "cons", "option", "options", "comparison", "compare", "pricing", "price", "prices", "tier",
    "tiers",
];

fn steps(l: &List) -> Option<Steps> {
    if !l.clean || !l.ordered || l.number != 1 || !(3..=15).contains(&l.items.len()) {
        return None;
    }
    if l.items.iter().any(|t| t.is_empty() || t.chars().count() > 300 || is_task(t) || has_html(t)) {
        return None;
    }
    let numbered = l.items.iter().all(|t| starts_step_n(t));
    let intro = l.intro.map(words).unwrap_or_default();
    let heading = l.heading.map(words).unwrap_or_default();
    let hinted = |w: &[String]| {
        w.iter().any(|x| STEP_HINTS.contains(&x.as_str()))
            || w.windows(2).any(|p| p == ["how", "to"] || p == ["to", "do"])
    };
    let hedged = |w: &[String]| w.iter().any(|x| NOT_STEPS.contains(&x.as_str()));
    let introduced = !hedged(&intro) && (hinted(&intro) || (hinted(&heading) && !hedged(&heading)));
    if !(numbered || introduced) {
        return None;
    }
    let items = l.items.iter().map(|t| (t.to_string(), None, None)).collect();
    Some(items::steps_block(l, Variant::Steps, items))
}

pub(super) fn words(s: &str) -> Vec<String> {
    let lower: String = s.chars().map(fold).collect();
    lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

pub(super) fn is_task(item: &str) -> bool {
    ["[ ]", "[x]", "[X]"].iter().any(|t| item.strip_prefix(t).is_some_and(|r| r.is_empty() || r.starts_with(' ')))
}

/// "Step 1", "**Step 2:**" and the like.
fn starts_step_n(item: &str) -> bool {
    let s = item.trim_start_matches(['*', '_', ' ']);
    let Some(head) = s.get(..4) else { return false };
    if !head.eq_ignore_ascii_case("step") {
        return false;
    }
    let digits = s[4..].trim_start();
    digits.starts_with(|c: char| c.is_ascii_digit())
}

#[cfg(test)]
#[path = "selector_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "block_tests.rs"]
mod block_tests;
