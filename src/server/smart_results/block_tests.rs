//! One test group per v2 block type (plan §A.4) and the priority order, with Rigel's traps
//! (qa/test-plan.md 5.4 and 5.14).
use super::*;
use crate::server::smart_results::{compute, validate, AxisKind, Block, ChartKind, Severity, Variant};

fn first(text: &str) -> Option<Block> {
    compute(text).map(|set| set.blocks.into_iter().next().expect("one block"))
}

fn kind(text: &str) -> Option<&'static str> {
    first(text).map(|b| b.kind())
}

fn table(header: &str, rows: &[String]) -> String {
    let n = header.matches('|').count() - 1;
    format!("{header}\n|{}\n{}\n", "---|".repeat(n), rows.join("\n"))
}

fn rows(n: usize, f: impl Fn(usize) -> String) -> Vec<String> {
    (0..n).map(f).collect()
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

// --- chart: line

#[test]
fn a_time_series_of_four_rows_or_more_is_a_line_chart() {
    let t = table("| Month | Visits |", &rows(4, |i| format!("| 2025-0{} | {} |", i + 1, 100 + i * 10)));
    let Some(Block::Chart(c)) = first(&t) else { panic!("{:?}", first(&t)) };
    assert_eq!((c.chart_kind, c.x.kind, c.series.len(), c.points.len()), (ChartKind::Line, AxisKind::Time, 1, 4));
    assert_eq!((c.source.kind.as_str(), c.source.index), ("markdown_table", Some(0)));
    assert_eq!((c.points[1].x.as_str(), c.points[1].cells[0].as_str(), c.points[1].values[0]), ("2025-02", "110", Some(110.0)));
    assert!(c.summary.is_none());
    // Four series is still a line; five isn't (never four of five silently)
    let wide = |s: usize| {
        let head = format!("| Month |{}", (0..s).map(|k| format!(" S{k} |")).collect::<String>());
        table(&head, &rows(5, |i| format!("| {} 2025 |{}", MONTHS[i], (0..s).map(|k| format!(" {} |", i + k)).collect::<String>())))
    };
    let Some(Block::Chart(c)) = first(&wide(4)) else { panic!() };
    assert_eq!(c.series.len(), 4);
    assert_ne!(kind(&wide(5)), Some("chart"));
}

#[test]
fn three_dated_rows_are_not_a_line() {
    let t = table("| Quarter | Revenue |", &rows(3, |i| format!("| Q{} 2025 | {} |", i + 1, 10 + i)));
    assert_ne!(kind(&t), Some("chart"));
}

#[test]
fn gaps_are_null_never_zero_and_mixed_units_never_chart() {
    let t = table("| Week | Sales |", &rows(5, |i| if i == 2 { "| Week 3 | n/a |".into() } else { format!("| Week {} | 4{i} |", i + 1) }));
    let Some(Block::Chart(c)) = first(&t) else { panic!("{:?}", first(&t)) };
    assert_eq!(c.points[2].values, [None]);
    let mixed = table("| Day | Price |", &rows(5, |i| format!("| Day {} | {} 1{i} |", i + 1, if i == 0 { "USD" } else { "EUR" })));
    assert_ne!(kind(&mixed), Some("chart"));
}

#[test]
fn series_in_different_units_never_share_one_axis() {
    // SR-R-13: bare numbers whose headers name two units stay off one Y axis
    let line = |h: &str| table(h, &rows(5, |i| format!("| {} 2025 | 1{i}0 | {}.{i} |", MONTHS[i], 3 + i % 2)));
    assert_ne!(kind(&line("| Month | Price (EUR) | Rating (1-5) |")), Some("chart"));
    assert_ne!(kind(&line("| Month | Price [EUR] | Rating |")), Some("chart"));
    assert_ne!(kind(&line("| Month | Price (€) | Cost (EUR) |")), Some("chart"));
    // A cell unit against a header unit that differs
    let t = table("| Month | Price | Rating (1-5) |", &rows(5, |i| format!("| {} 2025 | €1{i} | 4.{i} |", MONTHS[i])));
    assert_ne!(kind(&t), Some("chart"));
    let bar = table("| Shop | Price (EUR) | Rating (1-5) |", &rows(5, |i| format!("| Shop{} | 1{i}0 | 4.{i} |", MONTHS[i])));
    assert_ne!(kind(&bar), Some("chart"));
    // One unit, named in the headers or the cells, still charts
    for h in ["| Month | Revenue (EUR) | Cost (eur) |", "| Month | Signups | Paid |"] {
        let Some(Block::Chart(c)) = first(&line(h)) else { panic!("{h}: {:?}", first(&line(h))) };
        assert_eq!(c.series.len(), 2, "{h}");
    }
    let t = table("| Month | Revenue (€) | Cost |", &rows(5, |i| format!("| {} 2025 | 1{i}0 | €2{i} |", MONTHS[i])));
    let Some(Block::Chart(c)) = first(&t) else { panic!("{:?}", first(&t)) };
    assert_eq!(c.series.len(), 2);
}

#[test]
fn ambiguous_locale_numbers_abstain() {
    // `1,299` alone is 1299 or 1.299: no chart, and never a misread scale
    let t = table("| Month | Cost |", &rows(5, |i| format!("| {} 2025 | 1,2{i}9 |", MONTHS[i])));
    assert_ne!(kind(&t), Some("chart"));
    // With `1,299.00` beside it the column has one reading
    let t = table("| Month | Cost |", &rows(5, |i| format!("| {} 2025 | 1,2{i}9.50 |", MONTHS[i])));
    let Some(Block::Chart(c)) = first(&t) else { panic!("{:?}", first(&t)) };
    assert_eq!(c.points[0].values, [Some(1209.5)]);
    let t = table("| Month | Cost |", &rows(5, |i| format!("| {} 2025 | 1.2{i}9,50 |", MONTHS[i])));
    let Some(Block::Chart(c)) = first(&t) else { panic!("{:?}", first(&t)) };
    assert_eq!(c.points[0].values, [Some(1209.5)]);
}

// --- chart: bar

fn ranking(n: usize, cols: usize) -> String {
    let head = format!("| Language |{}", (0..cols).map(|k| format!(" Score {k} |")).collect::<String>());
    table(&head, &rows(n, |i| format!("| Lang{} |{}", (b'A' + i as u8) as char, (0..cols).map(|k| format!(" {} |", 90 - i - k)).collect::<String>())))
}

#[test]
fn five_to_twenty_category_rows_are_a_bar_chart() {
    for n in [5, 20] {
        let Some(Block::Chart(c)) = first(&ranking(n, 1)) else { panic!("{n}: {:?}", first(&ranking(n, 1))) };
        assert_eq!((c.chart_kind, c.x.kind, c.points.len()), (ChartKind::Bar, AxisKind::Category, n));
    }
    let Some(Block::Chart(c)) = first(&ranking(6, 2)) else { panic!() };
    assert_eq!(c.series.len(), 2);
}

#[test]
fn small_long_or_wide_rankings_are_comparisons() {
    // Ada's A.4 decision: 2–4 rows stay a comparison (Rigel SR-T-06)
    for n in [3, 4] {
        assert_eq!(kind(&ranking(n, 1)), Some("comparison"), "{n} rows");
    }
    assert_ne!(kind(&ranking(21, 1)), Some("chart"));
    assert_eq!(kind(&ranking(6, 3)), Some("comparison"));
}

#[test]
fn a_named_pick_keeps_the_table_a_comparison_with_its_highlight() {
    let text = format!("{}\nI'd go with LangB for this.\n", ranking(6, 1));
    let Some(Block::Comparison(c)) = first(&text) else { panic!("{:?}", first(&text)) };
    assert_eq!(c.highlight_row.as_deref(), Some("r1"));
}

// --- metrics

#[test]
fn bold_label_number_lines_are_metrics_with_value_text_verbatim() {
    let text = "Key numbers:\n- **Revenue:** $4.2M, up from last year\n- **Latency:** 12 ms\n- **Uptime:** 98%\n- **Tests:** 3 of 5\n";
    let Some(Block::Metrics(m)) = first(text) else { panic!("{:?}", first(text)) };
    let got: Vec<_> = m.items.iter().map(|i| (i.label.as_str(), i.value_text.as_str(), i.value, i.unit.as_deref())).collect();
    assert_eq!(got[0], ("Revenue", "$4.2M", 4.2, None));
    assert_eq!(got[1], ("Latency", "12 ms", 12.0, Some("ms")));
    assert_eq!(got[2], ("Uptime", "98%", 98.0, Some("%")));
    assert_eq!(m.items[0].note.as_deref(), Some("up from last year"));
    assert_eq!((m.source.kind.as_str(), m.source.index, m.source.ordered), ("markdown_list", Some(0), Some(false)));
    // One item or seven: not metrics
    assert_ne!(kind("- **Revenue:** $4.2M\n"), Some("metrics"));
    let seven: String = (0..7).map(|i| format!("- **K{}:** {i}%\n", MONTHS[i])).collect();
    assert_ne!(kind(&seven), Some("metrics"));
}

#[test]
fn a_two_column_table_is_metrics_when_its_units_differ_and_a_bar_from_five_rows_when_they_dont() {
    let mixed = table("| Metric | Value |", &["| Revenue | $4.2M |".into(), "| Latency | 12 ms |".into(), "| Uptime | 98% |".into()]);
    assert_eq!(kind(&mixed), Some("metrics"));
    let one_unit = |n: usize| table("| Team | Share |", &rows(n, |i| format!("| Team{} | {}% |", MONTHS[i], 10 + i)));
    for n in [3, 4] {
        assert_eq!(kind(&one_unit(n)), Some("comparison"), "{n} rows");
    }
    for n in [5, 6] {
        let Some(Block::Chart(c)) = first(&one_unit(n)) else { panic!("{n}") };
        assert_eq!(c.chart_kind, ChartKind::Bar);
    }
}

#[test]
fn recipes_packing_lists_and_schedules_are_not_metrics() {
    for text in [
        "Ingredients:\n- **Flour:** 2 cups\n- **Sugar:** 1 cup\n- **Eggs:** 3\n",
        "What to pack:\n- **Socks:** 5 pairs\n- **Shirts:** 3\n",
        "- **Mon:** 9 am\n- **Tue:** 10 am\n",
        "- **Note:** 5 minutes of rest\n",
    ] {
        assert_ne!(kind(text), Some("metrics"), "{text}");
    }
}

// --- the weather trap (Rigel SR-T-21)

#[test]
fn a_weather_reply_is_never_findings() {
    for text in [
        "- High: 18C\n- Low: 9C\n",
        "High: 18C / Low: 9C\n",
        "Forecast risks:\n- High: 18C\n- Low: 9C\n",
        "Review:\n- Low prices: cheaper than last year\n- Major benefit: it is quiet\n",
        "Issues:\n- High: 5 files fail to build\n- Low: one typo\n",
    ] {
        assert_ne!(kind(text), Some("findings"), "{text}");
    }
    // The bare high/low list reads as two key numbers, which is what it says
    assert_eq!(kind("- High: 18C\n- Low: 9C\n"), Some("metrics"));
    assert_eq!(kind("High: 18C / Low: 9C\n"), None);
}

// --- findings

#[test]
fn severity_lists_need_a_second_signal() {
    let list = "- **High:** SQL built from user input in `src/db.rs:42`\n- **Low:** typo in the log message\n";
    let Some(Block::Findings(f)) = first(list) else { panic!("{:?}", first(list)) };
    assert_eq!((f.items[0].severity, f.items[0].severity_text.as_str()), (Severity::High, "High"));
    assert_eq!(f.items[0].refs.len(), 1);
    assert_eq!((f.items[0].refs[0].path.as_str(), f.items[0].refs[0].line), ("src/db.rs", Some(42)));
    // Without the ref, only a review lead makes it findings
    let bare = "- **High:** SQL built from user input\n- **Low:** typo in the log message\n";
    assert_ne!(kind(bare), Some("findings"));
    assert_eq!(kind(&format!("Code review findings:\n\n{bare}")), Some("findings"));
    assert_eq!(kind(&format!("## Audit\n\n{bare}")), Some("findings"));
    // A port or a URL isn't a ref
    let port = "- **High:** the API listens on `localhost:8080`\n- **Low:** see `http://x:80`\n";
    assert_ne!(kind(port), Some("findings"));
}

#[test]
fn look_alike_severity_words_are_not_findings() {
    for text in [
        "Review:\n- High-quality build\n- Lowering the bar\n",
        "Review:\n- P4: someday\n- P5: never\n",
        "Review:\n- **High:** one thing\n",
    ] {
        assert_ne!(kind(text), Some("findings"), "{text}");
    }
}

// --- cards

#[test]
fn https_links_with_descriptions_are_cards() {
    let text = "Resources:\n- [Rust Book](https://doc.rust-lang.org/book/) — the official guide to the language\n- [Tokio](https://tokio.rs) — an async runtime for network services\n- [Serde](https://serde.rs) — serialization for Rust data structures\n";
    let Some(Block::Cards(c)) = first(text) else { panic!("{:?}", first(text)) };
    assert_eq!(c.items.len(), 3);
    assert_eq!((c.items[1].title.as_str(), c.items[1].url.as_deref()), ("Tokio", Some("https://tokio.rs")));
    for bad in ["http://tokio.rs", "javascript:alert(1)", "/docs", "mailto:a@b.c"] {
        let t = text.replace("https://tokio.rs", bad);
        assert_ne!(kind(&t), Some("cards"), "{bad}");
    }
    // Link text that looks like a URL elsewhere is a spoof
    let spoof = text.replace("[Tokio](https://tokio.rs)", "[https://bank.com](https://evil.test)");
    assert_ne!(kind(&spoof), Some("cards"));
    // A description under 10 characters is too weak
    assert_ne!(kind(&text.replace("an async runtime for network services", "a runtime")), Some("cards"));
}

#[test]
fn bold_titles_with_a_dash_are_cards_and_with_a_colon_are_facts() {
    let dash = "- **Alpha** — a description of fifteen characters\n- **Beta** — another long enough description\n- **Gamma** — and a third long description\n";
    assert_eq!(kind(dash), Some("cards"));
    let colon = dash.replace("** —", ":**");
    assert_eq!(kind(&colon), Some("facts"));
}

// --- facts

#[test]
fn bold_label_lines_that_are_mostly_words_are_facts() {
    let text = "Specs:\n- **Material:** aluminium\n- **Colour:** space grey\n- **Weight:** 1.2 kg\n- **Warranty:** two years\n";
    let Some(Block::Facts(f)) = first(text) else { panic!("{:?}", first(text)) };
    assert_eq!((f.items[2].label.as_str(), f.items[2].value.as_str()), ("Weight", "1.2 kg"));
    // Two items, or half numeric: not facts
    assert_ne!(kind("- **Material:** aluminium\n- **Colour:** grey\n"), Some("facts"));
    let half = "- **A:** 1 kg\n- **B:** 2 kg\n- **C:** grey\n- **D:** blue\n";
    assert_ne!(kind(half), Some("facts"));
}

// --- steps variants

#[test]
fn task_lists_are_checklists_with_the_reply_s_boxes() {
    let text = "Launch:\n- [x] Write notes\n- [ ] Tag the release\n- [ ] Announce it (owner: Sam)\n";
    let Some(Block::Steps(s)) = first(text) else { panic!("{:?}", first(text)) };
    assert_eq!((s.variant, s.source.ordered), (Variant::Checklist, Some(false)));
    assert_eq!(s.items.iter().map(|i| i.done).collect::<Vec<_>>(), [Some(true), Some(false), Some(false)]);
    assert_eq!(s.items[2].owner.as_deref(), Some("Sam"));
    // Two items, or plain items mixed in: markdown
    assert_eq!(kind("- [x] a\n- [ ] b\n"), None);
    assert_eq!(kind("- [x] a\n- b\n- [ ] c\n"), None);
}

#[test]
fn owners_come_only_from_the_literal_forms() {
    let owner = |tail: &str| {
        let text = format!("Steps:\n1. Build it {tail}\n2. Test it\n3. Ship it\n");
        match first(&text) {
            Some(Block::Steps(s)) => s.items[0].owner.clone(),
            other => panic!("{tail}: {other:?}"),
        }
    };
    assert_eq!(owner("(owner: Sam)").as_deref(), Some("Sam"));
    assert_eq!(owner("Owner: Sam").as_deref(), Some("Sam"));
    for tail in ["owner - Sam", "assigned to Sam", "Sam will do it", "OWNER: Sam"] {
        assert_eq!(owner(tail), None, "{tail}");
    }
}

#[test]
fn time_labelled_lists_are_timelines() {
    let text = "Release plan:\n- 2025-01-05 — branch cut\n- Jan 12: beta\n- Q2: general availability\n";
    let Some(Block::Steps(s)) = first(text) else { panic!("{:?}", first(text)) };
    assert_eq!(s.variant, Variant::Timeline);
    assert_eq!(s.items[1].when.as_deref(), Some("Jan 12"));
    assert_eq!(s.items[1].text, "beta");
    let day = "Agenda:\n- 9:00 – Intro\n- 10:30 – Demo\n- Mon: kickoff\n";
    assert_eq!(first(day).map(|b| rank(&b)), Some((PRIORITY.iter().position(|t| *t == "steps").unwrap(), 0)));
    // One label that isn't a time: not a timeline
    assert_ne!(kind("- 9:00 – Intro\n- Note: bring a laptop\n- 11:00 – Lunch\n"), Some("steps"));
}

// --- priority: one block per reply

#[test]
fn one_block_per_reply_by_priority_whatever_the_order() {
    let comparison = "| Plan | Price | Seats |\n|---|---|---|\n| Basic | EUR 12 | 1 |\n| Team | EUR 18 | 5 |\n| Business | EUR 26 | 20 |\n";
    let checklist = "- [x] a task\n- [ ] b task\n- [ ] c task\n";
    let metrics = "Key numbers:\n- **Revenue:** $4.2M\n- **Uptime:** 98%\n";
    let parts = [checklist, metrics, comparison];
    for order in [[0, 1, 2], [2, 1, 0], [1, 0, 2]] {
        let text: String = order.iter().map(|&i| format!("{}\n", parts[i])).collect();
        let set = compute(&text).expect("a block");
        assert_eq!(set.blocks.len(), 1);
        assert_eq!(set.blocks[0].kind(), "comparison", "{order:?}");
        validate(&set, Some(&text)).expect("valid");
    }
    // Without the table, metrics beats the checklist
    assert_eq!(kind(&format!("{checklist}\n{metrics}")), Some("metrics"));
}

#[test]
fn a_tie_goes_to_the_earliest_and_later_blocks_keep_their_indexes() {
    let a = "Steps:\n1. One\n2. Two\n3. Three\n";
    let b = "Steps:\n1. Uno\n2. Dos\n3. Tres\n";
    let Some(Block::Steps(s)) = first(&format!("{a}\n{b}")) else { panic!() };
    assert_eq!((s.items[0].text.as_str(), s.source.index), ("One", Some(0)));
    let all = candidates(&format!("{a}\n{b}"));
    assert_eq!(all[1].source().index, Some(1));
}

#[test]
fn ordered_and_unordered_lists_keep_separate_counters() {
    let text = "Notes:\n- a\n- b\n\nSteps:\n1. One\n2. Two\n3. Three\n\nLaunch:\n- [ ] x task\n- [ ] y task\n- [ ] z task\n";
    let all = candidates(text);
    let got: Vec<_> = all.iter().map(|b| (b.kind(), b.source().index, b.source().ordered)).collect();
    assert!(got.contains(&("steps", Some(0), Some(true))), "{got:?}");
    assert!(got.contains(&("steps", Some(1), Some(false))), "{got:?}");
    // A marker change splits an unordered list
    let text = "- a\n- b\n* [ ] x task\n* [ ] y task\n* [ ] z task\n";
    let Some(Block::Steps(s)) = first(text) else { panic!("{:?}", first(text)) };
    assert_eq!(s.source.index, Some(1));
}

#[test]
fn every_new_type_passes_its_checks() {
    for text in [
        table("| Month | Visits |", &rows(6, |i| format!("| {} 2025 | {} |", MONTHS[i], 100 + i))),
        ranking(7, 2),
        "- **Revenue:** $4.2M\n- **Uptime:** 98%\n".to_string(),
        "Specs:\n- **Material:** aluminium\n- **Colour:** grey\n- **Size:** large\n".to_string(),
        "Bugs:\n- [High] crash on start\n- Low: typo\n".to_string(),
        "- [Tokio](https://tokio.rs) — an async runtime\n- [Serde](https://serde.rs) — serialization crate\n- [Axum](https://docs.rs/axum) — a web framework\n".to_string(),
    ] {
        let set = compute(&text).unwrap_or_else(|| panic!("no block: {text}"));
        validate(&set, Some(&text)).unwrap_or_else(|e| panic!("{e:?}: {text}"));
    }
}

/// Ada's call (2026-11-02): a bare high/low weather list is two key-number tiles, never findings.
/// Pinned so a later change can't quietly turn it into findings or drop a tile.
#[test]
fn a_bare_weather_list_is_two_metric_tiles() {
    let text = "- High: 18C\n- Low: 9C\n";
    let Some(Block::Metrics(m)) = first(text) else { panic!("{:?}", first(text)) };
    let tiles: Vec<_> = m.items.iter().map(|i| (i.label.as_str(), i.value_text.as_str(), i.value, i.unit.as_deref())).collect();
    assert_eq!(tiles, [("High", "18C", 18.0, Some("C")), ("Low", "9C", 9.0, Some("C"))]);
    assert!(candidates(text).iter().all(|b| b.kind() != "findings"));
    let set = compute(text).unwrap();
    validate(&set, Some(text)).unwrap();
}
