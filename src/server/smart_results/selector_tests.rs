use super::*;
use crate::server::smart_results::{compute, validate, Block, ResultSet, MAX_PAYLOAD, PICK_REASON};

fn first(text: &str) -> Option<Block> {
    compute(text).map(|set| set.blocks.into_iter().next().expect("one block"))
}

fn comparison_of(text: &str) -> Comparison {
    match first(text) {
        Some(Block::Comparison(c)) => c,
        other => panic!("expected a comparison, got {other:?}"),
    }
}

fn steps_of(text: &str) -> Steps {
    match first(text) {
        Some(Block::Steps(s)) => s,
        other => panic!("expected steps, got {other:?}"),
    }
}

const PLANS: &str = "| Plan | Price | Seats |\n|---|---|---|\n| Basic | EUR 12 | 1 |\n| Team | EUR 18 | 5 |\n| Business | EUR 26 | 20 |\n";

fn with_prose(prose: &str) -> String {
    format!("Here are the plans.\n\n{PLANS}\n{prose}\n")
}

fn highlight(prose: &str) -> Option<String> {
    comparison_of(&with_prose(prose)).highlight_row
}

#[test]
fn a_pipe_table_is_a_comparison_with_cells_copied_verbatim() {
    let c = comparison_of(PLANS);
    assert_eq!(c.id, "b0");
    assert_eq!((c.source.kind.as_str(), c.source.index), ("markdown_table", 0));
    assert_eq!(c.provenance.kind, "reply_text");
    let cols: Vec<(&str, &str, bool, Option<&str>)> =
        c.columns.iter().map(|k| (k.key.as_str(), k.label.as_str(), k.numeric, k.unit.as_deref())).collect();
    assert_eq!(cols, [("c0", "Plan", false, None), ("c1", "Price", true, Some("EUR")), ("c2", "Seats", true, None)]);
    assert_eq!(c.rows[1].id, "r1");
    assert_eq!(c.rows[1].cells, ["Team", "EUR 18", "5"]);
    assert_eq!((c.highlight_row, c.highlight_reason), (None, None));
}

#[test]
fn small_ragged_wide_or_repeated_tables_stay_text() {
    let two_rows = "| Plan | Price |\n|---|---|\n| Basic | 12 |\n| Team | 18 |\n";
    assert_eq!(first(two_rows), None);
    let ragged = "| Plan | Price |\n|---|---|\n| Basic | 12 |\n| Team |\n| Business | 26 |\n";
    assert_eq!(first(ragged), None);
    let one_column = "| Plan |\n|---|\n| Basic |\n| Team |\n| Business |\n";
    assert_eq!(first(one_column), None);
    let header = (0..9).map(|i| format!("h{i}")).collect::<Vec<_>>().join(" | ");
    let rows = (0..3).map(|r| (0..9).map(|i| format!("r{r}c{i}")).collect::<Vec<_>>().join(" | ")).collect::<Vec<_>>();
    let nine = format!("| {header} |\n|{}\n| {} |\n| {} |\n| {} |\n", "---|".repeat(9), rows[0], rows[1], rows[2]);
    assert_eq!(first(&nine), None);
    for labels in [["A", "A", "B"], ["Alpha", "alpha", "Beta"], ["**Basic**", "Basic", "Team"], ["A ", "A", "B"]] {
        let t = format!("| Plan | Price |\n|---|---|\n| {} | 1 |\n| {} | 2 |\n| {} | 3 |\n", labels[0], labels[1], labels[2]);
        assert_eq!(first(&t), None, "{labels:?}");
    }
}

#[test]
fn row_and_cell_limits() {
    let rows = |n: usize| (0..n).map(|i| format!("| item {i} | {i} |\n")).collect::<String>();
    let fifty = format!("| Item | N |\n|---|---|\n{}", rows(50));
    assert_eq!(comparison_of(&fifty).rows.len(), 50);
    let fifty_one = format!("| Item | N |\n|---|---|\n{}", rows(51));
    assert_eq!(first(&fifty_one), None);
    let cell = |n: usize| format!("| Plan | Note |\n|---|---|\n| Basic | {} |\n| Team | b |\n| Business | c |\n", "x".repeat(n));
    assert!(first(&cell(200)).is_some());
    assert_eq!(first(&cell(201)), None);
}

#[test]
fn html_in_a_cell_or_step_stays_text() {
    let t = "| Plan | Price |\n|---|---|\n| <b>Basic</b> | 12 |\n| Team | 18 |\n| Business | 26 |\n";
    assert_eq!(first(t), None);
    let s = "Steps:\n1. Open <a href=x>this</a>\n2. Two\n3. Three\n";
    assert_eq!(first(s), None);
    // `a < b` isn't a tag
    let t = "| Plan | Rule |\n|---|---|\n| Basic | a < b |\n| Team | b < c |\n| Business | c < d |\n";
    assert!(first(t).is_some());
}

#[test]
fn tables_in_code_are_neither_offered_nor_counted() {
    for fence in ["```", "~~~", "````"] {
        let text = format!("{fence}\n| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n| 5 | 6 |\n{fence}\n\n{PLANS}");
        let c = comparison_of(&text);
        assert_eq!(c.source.index, 0, "{fence}");
        assert_eq!(c.rows[0].cells[0], "Basic");
    }
    let indented = format!("Code:\n\n    | a | b |\n    |---|---|\n    | 1 | 2 |\n\n{PLANS}");
    assert_eq!(comparison_of(&indented).source.index, 0);
    // An unclosed fence hides the rest of the reply
    assert_eq!(first(&format!("```\n{PLANS}")), None);
}

#[test]
fn the_first_qualifying_table_wins_and_keeps_its_index() {
    let small = "| a | b |\n|---|---|\n| 1 | 2 |\n\n";
    let c = comparison_of(&format!("{small}{PLANS}"));
    assert_eq!(c.source.index, 1);
    // A table beats a list, wherever the list is
    let text = format!("Steps:\n1. One\n2. Two\n3. Three\n\n{PLANS}");
    assert_eq!(comparison_of(&text).source.index, 0);
}

#[test]
fn uncertain_structure_before_a_candidate_abstains() {
    // A table inside a bullet item: remark-gfm counts it, this reader doesn't try
    let nested = format!("- item\n\n  | a | b |\n  |---|---|\n  | 1 | 2 |\n  | 3 | 4 |\n  | 5 | 6 |\n\n{PLANS}");
    assert_eq!(first(&nested), None);
    // A table in a quote
    let quoted = format!("> | a | b |\n> |---|---|\n> | 1 | 2 |\n\n{PLANS}");
    assert_eq!(first(&quoted), None);
    // A numbered list in a quote, then steps
    let quote_list = "> 1. a\n> 2. b\n\nSteps:\n1. One\n2. Two\n3. Three\n";
    assert_eq!(first(quote_list), None);
    // A numbered list nested in a bullet, then steps
    let nested_list = "- top\n  1. a\n  2. b\n\nSteps:\n1. One\n2. Two\n3. Three\n";
    assert_eq!(first(nested_list), None);
    // Raw HTML
    assert_eq!(first(&format!("<div>\nhi\n</div>\n\n{PLANS}")), None);
    // A table straight after a paragraph line
    assert_eq!(first(&format!("Here:\n{PLANS}")), None);
    // Tabs
    assert_eq!(first(&format!("a\tb\n\n{PLANS}")), None);
    // What comes after the uncertain part is out; what comes before is still offered
    let before = format!("{PLANS}\n> | a | b |\n> |---|---|\n> | 1 | 2 |\n");
    assert_eq!(comparison_of(&before).source.index, 0);
}

#[test]
fn a_pick_named_once_in_a_recommending_sentence_is_highlighted() {
    assert_eq!(highlight("Team is the best fit for five people."), Some("r1".into()));
    assert_eq!(highlight("I recommend **Business** for larger teams."), Some("r2".into()));
    assert_eq!(highlight("Go with Basic."), Some("r0".into()));
    let c = comparison_of(&with_prose("My pick: Team."));
    assert_eq!(c.highlight_reason.as_deref(), Some(PICK_REASON));
}

#[test]
fn no_highlight_when_unsure() {
    // Two rows named
    assert_eq!(highlight("Basic and Team are both the best value."), None);
    assert_eq!(highlight("Team is the best. Basic is the best for one person."), None);
    // No recommending word
    assert_eq!(highlight("Team has five seats."), None);
    // Negation, hedges and questions
    assert_eq!(highlight("I would not recommend Team."), None);
    assert_eq!(highlight("Don't pick Basic."), None);
    assert_eq!(highlight("If you grow, Business is the best."), None);
    assert_eq!(highlight("Is Team the best?"), None);
    assert_eq!(highlight("Team is the best, but check the seats."), None);
    // A recommending sentence inside the table doesn't count
    let t = "| Plan | Note |\n|---|---|\n| Basic | the best for one |\n| Team | b |\n| Business | c |\n";
    assert_eq!(comparison_of(t).highlight_row, None);
}

#[test]
fn highlight_names_whole_labels_only() {
    let abc = "| Option | Cost |\n|---|---|\n| A | 1 |\n| B | 2 |\n| C | 3 |\n";
    let hl = |prose: &str| comparison_of(&format!("{abc}\n{prose}\n")).highlight_row;
    // A lone "A" opening a sentence is the word
    assert_eq!(hl("A clear winner is B."), Some("r1".into()));
    // Lowercase "a" and words containing "a" aren't label A
    assert_eq!(hl("Overall a great pick is C, also cheap."), Some("r2".into()));
    assert_eq!(hl("Pick A."), Some("r0".into()));

    let pro = "| Model | Price |\n|---|---|\n| Pro | 999 |\n| Pro Max | 1199 |\n| Air | 799 |\n";
    let hl = |prose: &str| comparison_of(&format!("{pro}\n{prose}\n")).highlight_row;
    assert_eq!(hl("The Pro Max is the best camera."), Some("r1".into()));
    assert_eq!(hl("The Pro is the best value."), Some("r0".into()));

    let langs = "| Language | Speed |\n|---|---|\n| C | 10 |\n| C++ | 9 |\n| Rust | 9 |\n";
    let hl = |prose: &str| comparison_of(&format!("{langs}\n{prose}\n")).highlight_row;
    assert_eq!(hl("I recommend C++ here."), Some("r1".into()));
    assert_eq!(hl("I recommend C here."), Some("r0".into()));

    let beta = "| Tool | Cost |\n|---|---|\n| Claude | 1 |\n| Claude (beta) | 2 |\n| Gemini | 3 |\n";
    let hl = |prose: &str| comparison_of(&format!("{beta}\n{prose}\n")).highlight_row;
    assert_eq!(hl("Claude (beta) is my pick."), Some("r1".into()));

    let js = "| Runtime | Speed |\n|---|---|\n| Node | 1 |\n| Deno | 2 |\n| Bun | 3 |\n";
    let hl = |prose: &str| comparison_of(&format!("{js}\n{prose}\n")).highlight_row;
    assert_eq!(hl("Node.js is the best known."), None);
    assert_eq!(hl("Bun is the best. "), Some("r2".into()));
}

#[test]
fn numeric_columns_and_units() {
    assert_eq!(number_affix("12"), Some(None));
    assert_eq!(number_affix("EUR 12"), Some(Some("EUR")));
    assert_eq!(number_affix("12 EUR"), Some(Some("EUR")));
    assert_eq!(number_affix("$1,299.00"), Some(Some("$")));
    assert_eq!(number_affix("1.299,00 EUR"), Some(Some("EUR")));
    assert_eq!(number_affix("-4%"), Some(Some("%")));
    assert_eq!(number_affix("−4%"), Some(Some("%")));
    assert_eq!(number_affix("4.5/5"), Some(Some("/5")));
    assert_eq!(number_affix("**12 ms**"), Some(Some("ms")));
    assert_eq!(number_affix("N/A"), None);
    assert_eq!(number_affix("about 12"), None);
    assert_eq!(number_affix("12 to 14"), None);
    assert_eq!(number_affix("EUR 12/mo"), None);
    assert_eq!(number_affix("1,2,3"), None);
    assert_eq!(number_affix("1234567890123456"), None);

    let col = |cells: &[&str]| numeric_column(cells.iter().copied());
    assert_eq!(col(&["1", "2", "3", "4", "n/a"]), (true, None));
    assert_eq!(col(&["1", "2", "3", "n/a", "n/a"]), (false, None));
    assert_eq!(col(&["EUR 1", "EUR 2", "EUR 3", "EUR 4", "5"]), (true, Some("EUR".into())));
    assert_eq!(col(&["EUR 1", "EUR 2", "3 USD", "4 USD", "5"]), (false, None));
}

#[test]
fn a_numbered_list_introduced_as_steps_is_steps() {
    let s = steps_of("Steps:\n1. Open settings\n2. Pick a bot\n3. Save\n");
    assert_eq!((s.source.kind.as_str(), s.source.index), ("markdown_list", 0));
    let items: Vec<(&str, &str)> = s.items.iter().map(|i| (i.id.as_str(), i.text.as_str())).collect();
    assert_eq!(items, [("s0", "Open settings"), ("s1", "Pick a bot"), ("s2", "Save")]);
    assert!(first("Here's how to set it up:\n1. One\n2. Two\n3. Three\n").is_some());
    assert!(first("A quick checklist:\n1) One\n2) Two\n3) Three\n").is_some());
    assert!(first("## How to deploy\n\n1. One\n2. Two\n3. Three\n").is_some());
    assert!(first("## Deploy plan\n\nDo this:\n\n1. One\n2. Two\n3. Three\n").is_some());
    assert!(first("1. **Step 1:** Open\n2. **Step 2:** Pick\n3. Step 3 - Save\n").is_some());
}

#[test]
fn other_numbered_lists_stay_text() {
    assert_eq!(first("3 reasons why:\n1. One\n2. Two\n3. Three\n"), None);
    assert_eq!(first("Here are some ideas:\n1. One\n2. Two\n3. Three\n"), None);
    assert_eq!(first("Steps to compare pricing:\n1. One\n2. Two\n3. Three\n"), None);
    assert_eq!(first("Some things:\n1. One\n2. Two\n3. Three\n"), None);
    // Two items, or more than 15
    assert_eq!(first("Steps:\n1. One\n2. Two\n"), None);
    let many = (1..=16).map(|i| format!("{i}. item {i}\n")).collect::<String>();
    assert_eq!(first(&format!("Steps:\n{many}")), None);
    // Not starting at 1, or items with more than one line
    assert_eq!(first("Steps:\n5. One\n6. Two\n7. Three\n"), None);
    assert_eq!(first("Steps:\n1. One\n   more\n2. Two\n3. Three\n"), None);
    assert_eq!(first("Steps:\n1. One\n   - sub\n2. Two\n3. Three\n"), None);
    // 301 characters
    assert_eq!(first(&format!("Steps:\n1. {}\n2. Two\n3. Three\n", "x".repeat(301))), None);
}

#[test]
fn a_task_list_is_counted_but_not_offered() {
    let text = "Plan:\n1. [ ] a\n2. [x] b\n3. [ ] c\n\nSteps:\n1. One\n2. Two\n3. Three\n";
    assert_eq!(steps_of(text).source.index, 1);
}

#[test]
fn lists_are_counted_in_order_and_by_delimiter() {
    // `1.` then `1)` is two lists
    let text = "Steps:\n1. a\n2. b\n1) c\n2) d\n3) e\n\nSteps:\n1. One\n2. Two\n3. Three\n";
    assert_eq!(steps_of(text).source.index, 2);
    // A list in a fence isn't counted
    let text = "```\n1. a\n2. b\n3. c\n```\n\nSteps:\n1. One\n2. Two\n3. Three\n";
    assert_eq!(steps_of(text).source.index, 0);
}

#[test]
fn every_value_is_text_of_the_reply_and_checks_pass() {
    let text = with_prose("Team is the best pick.");
    let set = compute(&text).unwrap();
    assert_eq!(validate(&set, Some(&text)), Ok(()));
    let mut changed = set.clone();
    if let Block::Comparison(c) = &mut changed.blocks[0] {
        c.rows[0].cells[1] = "EUR 99".into();
    }
    assert!(validate(&changed, Some(&text)).is_err());
    // An unknown version is never served
    let v2 = ResultSet { version: 2, ..set.clone() };
    assert!(validate(&v2, None).is_err());
    let none = ResultSet { blocks: Vec::new(), ..set };
    assert!(validate(&none, None).is_err());
}

#[test]
fn a_too_big_block_is_dropped() {
    let cell = "x".repeat(200);
    let rows = (0..50).map(|i| format!("| item {i} | {cell} | {cell} | {cell} | {cell} | {cell} | {cell} | {cell} |\n")).collect::<String>();
    let text = format!("| A | B | C | D | E | F | G | H |\n|---|---|---|---|---|---|---|---|\n{rows}");
    assert!(text.len() > MAX_PAYLOAD);
    assert_eq!(candidates(&text).len(), 1);
    assert_eq!(compute(&text), None);
    // Over the reply limit, nothing is read at all
    let long = format!("{PLANS}\n{}", "word ".repeat(30_000));
    assert!(candidates(&long).is_empty());
}

#[test]
fn odd_input_never_panics() {
    // A small deterministic generator over markdown-ish pieces, multibyte text included
    let pieces = [
        "|", " | ", "---", ":-:", "\n", "\n\n", "1. ", "2) ", "- ", "* ", "> ", "    ", "\t", "```", "~~~", "#", "## ",
        "Steps:", "best", "pick ", "Pro", "A", "€", "日本", "é", "💡", "\\|", "<b>", "**", "[x](y)", "](", "12", "1,299.00",
        "-4%", "[ ]", "Step 1", "\r\n", "=", "  ", ".",
    ];
    let mut seed: u64 = 0x5eed;
    for _ in 0..3_000 {
        let mut text = String::new();
        for _ in 0..(seed % 60) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            text.push_str(pieces[(seed % pieces.len() as u64) as usize]);
        }
        let found = candidates(&text);
        if let Some(set) = compute(&text) {
            assert_eq!(validate(&set, Some(&text)), Ok(()), "{text:?}");
        }
        drop(found);
    }
}

#[test]
fn pathological_text_stays_fast() {
    let labels = (0..50).map(|i| format!("| {} | {i} |\n", "a".repeat(150 + i))).collect::<String>();
    let prose = format!("{} best.\n", "a".repeat(990)).repeat(120);
    let text = format!("| Name | N |\n|---|---|\n{labels}\n{prose}{}", "](".repeat(5_000));
    let start = std::time::Instant::now();
    let _ = compute(&text);
    assert!(start.elapsed() < std::time::Duration::from_secs(2), "{:?}", start.elapsed());
}
