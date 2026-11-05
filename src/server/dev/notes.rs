//! The release-notes draft (plan §3.4 `release_notes`): the landed cards since the last tag, grouped
//! by area, as markdown, plus a one-line "what's new". Built from the cards' text, with no model
//! call of its own, so it costs nothing against the daily cap.

use serde_json::{json, Value};

/// Headlines named in the "what's new" line before "and N more".
const NAMED: usize = 3;

/// Markdown-safe one line: control characters dropped, newlines folded, and the characters that
/// would make a link, an image, HTML or emphasis escaped (the text came from a model reading a PR).
pub fn inline(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_control() {
            continue;
        }
        if matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' | '|' | '!' | '~') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The `release_notes` card from the landed `cards` (oldest first; their card JSON, or null when
/// a card has no text yet).
pub fn card(since_tag: Option<&str>, cards: &[Value]) -> Value {
    let mut areas: Vec<(String, Vec<String>)> = Vec::new();
    let mut headlines: Vec<String> = Vec::new();
    for c in cards {
        let Some(h) = c["headline"].as_str().map(str::trim).filter(|h| !h.is_empty()) else { continue };
        let area = c["area"].as_str().map(str::trim).filter(|a| !a.is_empty()).unwrap_or("Other").to_string();
        let pr = c["pr"].as_u64().map(|p| format!(" (#{p})")).unwrap_or_default();
        let line = format!("- {}{pr}", inline(h));
        match areas.iter_mut().find(|(a, _)| a.eq_ignore_ascii_case(&area)) {
            Some((_, lines)) => lines.push(line),
            None => areas.push((area, vec![line])),
        }
        headlines.push(h.split_whitespace().collect::<Vec<_>>().join(" "));
    }
    let since = since_tag.map_or_else(|| "the start".to_string(), str::to_string);
    let mut md = format!("# What's new since {}\n", inline(&since));
    if areas.is_empty() {
        md.push_str("\nNothing has landed yet.\n");
    }
    for (area, lines) in &areas {
        md.push_str(&format!("\n## {}\n", inline(area)));
        for l in lines {
            md.push_str(l);
            md.push('\n');
        }
    }
    let n = headlines.len();
    let whats_new = match n {
        0 => format!("Nothing new since {since}."),
        _ => {
            let named = headlines.iter().take(NAMED).cloned().collect::<Vec<_>>().join("; ");
            let more = if n > NAMED { format!("; and {} more", n - NAMED) } else { String::new() };
            format!("{n} change{} since {since}: {named}{more}.", if n == 1 { "" } else { "s" })
        }
    };
    json!({"kind": "release_notes", "since_tag": since_tag, "markdown": md, "whats_new": whats_new})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn landed(pr: u64, area: &str, headline: &str) -> Value {
        json!({"kind": "landed", "pr": pr, "area": area, "headline": headline})
    }

    #[test]
    fn cards_group_by_area_in_landing_order() {
        let cards = [
            landed(290, "Sidebar", "Sidebar rows stop jumping on hover"),
            landed(301, "Sidebar", "Long names wrap between words"),
            landed(303, "Chat", "A 👍 reaches the bot once"),
            Value::Null,
            landed(304, "Results", "Cards get their icons back"),
        ];
        let c = card(Some("mac-v0.6.12"), &cards);
        assert_eq!(
            c["markdown"],
            "# What's new since mac-v0.6.12\n\n## Sidebar\n- Sidebar rows stop jumping on hover (#290)\n- Long names wrap between words (#301)\n\n## Chat\n- A 👍 reaches the bot once (#303)\n\n## Results\n- Cards get their icons back (#304)\n"
        );
        assert_eq!(c["whats_new"], "4 changes since mac-v0.6.12: Sidebar rows stop jumping on hover; Long names wrap between words; A 👍 reaches the bot once; and 1 more.");
        assert_eq!(c["since_tag"], "mac-v0.6.12");
    }

    #[test]
    fn nothing_landed_and_no_tag() {
        let c = card(None, &[]);
        assert_eq!(c["markdown"], "# What's new since the start\n\nNothing has landed yet.\n");
        assert_eq!(c["whats_new"], "Nothing new since the start.");
        assert_eq!(c["since_tag"], Value::Null);
        let c = card(None, &[json!({"pr": 1, "headline": "One thing"})]);
        assert_eq!(c["whats_new"], "1 change since the start: One thing.");
        assert!(c["markdown"].as_str().unwrap().contains("## Other\n- One thing (#1)"));
    }

    #[test]
    fn model_text_cant_inject_markdown() {
        let c = card(Some("v1"), &[landed(9, "Chat\n# Owned", "Click [here](https://example.com) <img src=x>\n## Fake")]);
        let md = c["markdown"].as_str().unwrap();
        assert!(md.contains("## Chat \\# Owned\n"), "{md}");
        assert!(md.contains("- Click \\[here\\](https://example.com) \\<img src=x\\> \\#\\# Fake (#9)"), "{md}");
        assert_eq!(md.lines().filter(|l| l.starts_with('#')).count(), 2);
    }
}
