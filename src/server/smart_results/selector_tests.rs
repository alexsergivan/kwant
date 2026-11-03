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
    assert_eq!((c.source.kind.as_str(), c.source.index), ("markdown_table", Some(0)));
    assert_eq!(c.provenance.kind, "reply_text");
    let cols: Vec<(&str, &str, bool, Option<&str>)> =
        c.columns.iter().map(|k| (k.key.as_str(), k.label.as_str(), k.numeric, k.unit.as_deref())).collect();
    assert_eq!(cols, [("c0", "Plan", false, None), ("c1", "Price", true, Some("EUR")), ("c2", "Seats", true, None)]);
    assert_eq!(c.rows[1].id, "r1");
    assert_eq!(c.rows[1].cells, ["Team", "EUR 18", "5"]);
    assert_eq!((c.highlight_row, c.highlight_reason), (None, None));
}

/// The prompt line asks for this shape when a reply compares just two options ("Compare Aldi with
/// Lidl" came back as bullets and stayed text): the options as columns, a row per point.
#[test]
fn a_two_option_table_with_a_row_per_point_is_a_comparison() {
    let text = "They're close on price; it depends on what you buy.\n\n\
        | Point | Aldi Süd | Lidl |\n|---|---|---|\n\
        | Prices | Basket €42.07, about 4% less | Basket €44.02 |\n\
        | Deals | Tighter range | More non-food deals each week |\n\
        | App | Nothing comparable | Lidl Plus coupons |\n\
        | Store feel | Quicker to shop | Bigger, more variety |\n\n\
        If you mostly buy staples, Aldi Süd is the slightly cheaper default.\n";
    let c = comparison_of(text);
    let labels: Vec<&str> = c.columns.iter().map(|k| k.label.as_str()).collect();
    assert_eq!(labels, ["Point", "Aldi Süd", "Lidl"]);
    let points: Vec<&str> = c.rows.iter().map(|r| r.cells[0].as_str()).collect();
    assert_eq!(points, ["Prices", "Deals", "App", "Store feel"]);
    assert_eq!(c.rows[0].cells, ["Prices", "Basket €42.07, about 4% less", "Basket €44.02"]);
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
        assert_eq!(c.source.index, Some(0), "{fence}");
        assert_eq!(c.rows[0].cells[0], "Basic");
    }
    let indented = format!("Code:\n\n    | a | b |\n    |---|---|\n    | 1 | 2 |\n\n{PLANS}");
    assert_eq!(comparison_of(&indented).source.index, Some(0));
    // An unclosed fence hides the rest of the reply
    assert_eq!(first(&format!("```\n{PLANS}")), None);
}

#[test]
fn the_first_qualifying_table_wins_and_keeps_its_index() {
    let small = "| a | b |\n|---|---|\n| 1 | 2 |\n\n";
    let c = comparison_of(&format!("{small}{PLANS}"));
    assert_eq!(c.source.index, Some(1));
    // A table beats a list, wherever the list is
    let text = format!("Steps:\n1. One\n2. Two\n3. Three\n\n{PLANS}");
    assert_eq!(comparison_of(&text).source.index, Some(0));
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
    assert_eq!(comparison_of(&before).source.index, Some(0));
}

#[test]
fn text_the_chat_reads_differently_abstains() {
    // `- | -` under a pipe line is a bullet item to the chat, not a delimiter row
    assert_eq!(first(&format!("a | b\n- | -\n1 | 2\n\n{PLANS}")), None);
    // A one-column table needs no pipe when its delimiter has a colon
    assert_eq!(first(&format!("Pro\n--:\nrow\n\n{PLANS}")), None);
    // ... but `---` under a line is a heading, and the table after it is still table 0
    assert_eq!(comparison_of(&format!("Pro\n---\n\n{PLANS}")).source.index, Some(0));
    // A footnote definition is rendered at the end, with any table in it
    let note = format!("Text[^1]\n\n[^1]: Note\n\n    | k | v |\n    |---|---|\n    | a | 1 |\n\n{PLANS}");
    assert_eq!(first(&note), None);
    // A [DECISIONS] tag left in the text: the chat hides it and what follows, and trims the rest
    assert_eq!(first(&format!("Intro [DECISIONS]\n\n{PLANS}")), None);
    let trimmed = "    1. a\n    2. b\n\nSteps:\n1. One\n2. Two\n3. Three\n\n[DECISIONS] none\n";
    assert_eq!(first(trimmed), None);
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
    assert_eq!((s.source.kind.as_str(), s.source.index), ("markdown_list", Some(0)));
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
fn a_task_list_is_a_checklist_and_counts_among_numbered_lists() {
    let text = "Plan:\n1. [ ] a\n2. [x] b\n3. [ ] c\n\nSteps:\n1. One\n2. Two\n3. Three\n";
    let c = steps_of(text);
    assert_eq!((c.variant, c.source.index, c.source.ordered), (Variant::Checklist, Some(0), Some(true)));
    let done: Vec<_> = c.items.iter().map(|i| (i.text.as_str(), i.done)).collect();
    assert_eq!(done, [("a", Some(false)), ("b", Some(true)), ("c", Some(false))]);
    let all = candidates(text);
    match &all[1] {
        Block::Steps(s) => assert_eq!((s.variant, s.source.index), (Variant::Steps, Some(1))),
        other => panic!("{other:?}"),
    }
}

#[test]
fn lists_are_counted_in_order_and_by_delimiter() {
    // `1.` then `1)` is two lists
    let text = "Steps:\n1. a\n2. b\n1) c\n2) d\n3) e\n\nSteps:\n1. One\n2. Two\n3. Three\n";
    assert_eq!(steps_of(text).source.index, Some(2));
    // A list in a fence isn't counted
    let text = "```\n1. a\n2. b\n3. c\n```\n\nSteps:\n1. One\n2. Two\n3. Three\n";
    assert_eq!(steps_of(text).source.index, Some(0));
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
    // Labels found all over the prose but never as whole names, recommending prose over the scan
    // cap (100 lines) and just under it (8 lines), link brackets that never close
    let labels = (0..50).map(|i| format!("| {} | {i} |\n", "a".repeat(150 + i))).collect::<String>();
    for lines in [100, 8] {
        let prose = format!("{} best.\n", "a".repeat(990)).repeat(lines);
        let text = format!("| Name | N |\n|---|---|\n{labels}\n{prose}{}", "](".repeat(2_000));
        assert!(text.len() < MAX_REPLY);
        let start = std::time::Instant::now();
        let c = comparison_of(&text);
        assert!(start.elapsed() < std::time::Duration::from_secs(2), "{lines}: {:?}", start.elapsed());
        assert_eq!(c.highlight_row, None);
    }
    // Hundreds of tables cost no more than a few
    let many = format!("{PLANS}\n").repeat(1_000) + "We recommend Team.\n";
    assert!(many.len() < MAX_REPLY);
    let start = std::time::Instant::now();
    assert_eq!(candidates(&many).len(), MAX_TRIES);
    assert!(start.elapsed() < std::time::Duration::from_secs(2), "{:?}", start.elapsed());
}

#[test]
fn huge_ragged_tables_nested_lists_and_pipe_walls_stay_fast() {
    // The block is worked out inside the message insert, so no reply under the size cap may be slow
    let header = "| a | b | c | d | e | f | g | h |\n|---|---|---|---|---|---|---|---|\n";
    let ragged = (0..2_500).map(|i| "| x ".repeat(7 + i % 3) + "|\n").collect::<String>();
    let nested =
        (0..5_000).map(|i| format!("{}{} x\n", "   ".repeat(i % 12), if i % 2 == 0 { "1." } else { "-" })).collect::<String>();
    let cases = [
        ("ragged table", format!("{header}{ragged}")),
        ("nested lists", format!("Steps:\n\n{nested}")),
        ("pipe wall", "|".repeat(MAX_REPLY - 1)),
        ("pipe lines", format!("{header}{}", "|||||||||\n".repeat(12_000))),
        ("tiny tables", "| a |\n|---|\n\n".repeat(9_000)),
        ("escaped pipes", format!("{header}{}", "| \\| ".repeat(25_000))),
    ];
    for (what, text) in cases {
        assert!(text.len() < MAX_REPLY, "{what}: {} bytes", text.len());
        let start = std::time::Instant::now();
        let set = compute(&text);
        let took = start.elapsed();
        eprintln!("{what}: {took:?}");
        assert!(took < std::time::Duration::from_secs(1), "{what}: {took:?}");
        assert_eq!(set, None, "{what}");
    }
}

#[test]
fn a_long_run_of_recommending_prose_names_no_pick() {
    let some = format!("I recommend Team. {}", "We recommend reading the docs. ".repeat(100));
    assert_eq!(highlight(&some).as_deref(), Some("r1"));
    let too_much = format!("I recommend Team. {}", "We recommend reading the docs. ".repeat(300));
    assert_eq!(highlight(&too_much), None);
}

/// Found by the bullet-list cross-check (index-check, Z129 and K20).
#[test]
fn bullet_lists_end_where_the_chat_ends_them() {
    // `3.` after a bullet item: lazy text or a new numbered list; either way no later bullet
    // list is sure of its number
    let text = "- a\n- b\n3. c\n4. d\n\n- [ ] x task\n- [ ] y task\n- [ ] z task\n";
    assert!(candidates(text).iter().all(|b| b.source().ordered != Some(false)), "{:?}", candidates(text));
    // A fence opened on the marker line belongs to the item, and the list isn't offered
    let text = "- ```\n  [ ] a\n  ```\n\nLaunch:\n\n- [ ] x task\n- [ ] y task\n- [ ] z task\n";
    let Some(Block::Steps(s)) = first(text) else { panic!("{:?}", first(text)) };
    assert_eq!((s.variant, s.source.index), (Variant::Checklist, Some(1)));
    assert_eq!(first("Steps:\n1. ```\n2. b\n3. c\n"), None);
}
