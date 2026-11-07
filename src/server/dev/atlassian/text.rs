//! Atlassian text to plain text and back: Atlassian Document Format (Jira Cloud v3), Confluence
//! storage format and Jira DC's rendered HTML, and wiki markup for a DC comment. What comes out is
//! plain text for a brief, never HTML: nothing here is rendered, so a hostile ticket can't inject
//! markup into the desk.

use serde_json::{json, Value};

/// How deep an ADF tree is walked; deeper nodes are dropped (a hostile document can't recurse us).
const MAX_DEPTH: usize = 48;

/// ADF (`{"type":"doc","content":[…]}`) as plain text: paragraphs and headings separated by a
/// blank line, list items as `- ` / `1. `, a mention as its display text, a link card as its URL,
/// media as `[attachment]`, table cells joined with ` | `.
pub fn adf_to_text(doc: &Value) -> String {
    let mut out = String::new();
    walk(doc, &mut out, 0, &mut Vec::new());
    tidy(&out)
}

fn walk(n: &Value, out: &mut String, depth: usize, lists: &mut Vec<(bool, usize)>) {
    if depth > MAX_DEPTH {
        return;
    }
    let kids = |out: &mut String, lists: &mut Vec<(bool, usize)>| {
        for c in n["content"].as_array().into_iter().flatten() {
            walk(c, out, depth + 1, lists);
        }
    };
    match n["type"].as_str().unwrap_or_default() {
        "text" => out.push_str(n["text"].as_str().unwrap_or_default()),
        "hardBreak" => out.push('\n'),
        "mention" => out.push_str(n["attrs"]["text"].as_str().unwrap_or("@someone")),
        "emoji" => out.push_str(n["attrs"]["text"].as_str().or(n["attrs"]["shortName"].as_str()).unwrap_or_default()),
        "date" => out.push_str(n["attrs"]["timestamp"].as_str().unwrap_or_default()),
        "status" => out.push_str(n["attrs"]["text"].as_str().unwrap_or_default()),
        "inlineCard" | "blockCard" | "embedCard" => {
            out.push_str(n["attrs"]["url"].as_str().unwrap_or_default());
            if n["type"] != "inlineCard" {
                out.push_str("\n\n");
            }
        }
        "media" | "mediaInline" => out.push_str("[attachment]"),
        "rule" => out.push_str("\n---\n\n"),
        "paragraph" | "heading" | "codeBlock" | "blockquote" | "panel" | "mediaSingle" | "mediaGroup" | "expand" | "nestedExpand" => {
            if lists.is_empty() {
                block_start(out);
            }
            kids(out, lists);
            if lists.is_empty() {
                out.push_str("\n\n");
            }
        }
        "bulletList" | "orderedList" => {
            lists.push((n["type"] == "orderedList", 0));
            kids(out, lists);
            lists.pop();
            if lists.is_empty() {
                out.push('\n');
            }
        }
        "listItem" | "taskItem" | "decisionItem" => {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            let indent = "  ".repeat(lists.len().saturating_sub(1));
            let marker = match lists.last_mut() {
                Some((true, k)) => {
                    *k += 1;
                    format!("{k}. ")
                }
                _ => "- ".to_string(),
            };
            out.push_str(&indent);
            out.push_str(&marker);
            kids(out, lists);
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
        "table" => {
            block_start(out);
            kids(out, lists);
            out.push('\n');
        }
        "tableRow" => {
            let mut cells = Vec::new();
            for c in n["content"].as_array().into_iter().flatten() {
                let mut cell = String::new();
                walk(c, &mut cell, depth + 1, &mut vec![(false, 0)]);
                cells.push(cell.split_whitespace().collect::<Vec<_>>().join(" "));
            }
            out.push_str(&cells.join(" | "));
            out.push('\n');
        }
        _ => kids(out, lists),
    }
}

fn block_start(out: &mut String) {
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push_str(if out.ends_with('\n') { "\n" } else { "\n\n" });
    }
}

/// Plain text as an ADF document: blank-line-separated paragraphs (single newlines become hard
/// breaks), and a run of lines starting `- ` or `* ` as a bullet list. No marks, links or macros:
/// what kwant posts reads exactly as written.
pub fn text_to_adf(text: &str) -> Value {
    let mut content = Vec::new();
    for block in text.replace("\r\n", "\n").split("\n\n").map(|b| b.trim_matches('\n')).filter(|b| !b.trim().is_empty()) {
        let lines: Vec<&str> = block.lines().collect();
        let bullet = |l: &str| l.trim_start().strip_prefix("- ").or_else(|| l.trim_start().strip_prefix("* ")).map(str::to_string);
        if lines.iter().all(|l| bullet(l).is_some()) {
            let items: Vec<Value> = lines
                .iter()
                .map(|l| json!({"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": bullet(l).unwrap_or_default()}]}]}))
                .collect();
            content.push(json!({"type": "bulletList", "content": items}));
            continue;
        }
        let mut inline = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            if i > 0 {
                inline.push(json!({"type": "hardBreak"}));
            }
            if !l.is_empty() {
                inline.push(json!({"type": "text", "text": l}));
            }
        }
        content.push(json!({"type": "paragraph", "content": inline}));
    }
    json!({"type": "doc", "version": 1, "content": content})
}

/// Plain text for a Jira DC comment (wiki markup): the characters that open a macro, link, image
/// or table (`{ } [ ] ! |`) are escaped with `\`, so the text reads as written and can't pull in
/// a macro. Bullets (`- `) and line breaks stay as they are.
pub fn text_to_wiki(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    for c in text.replace("\r\n", "\n").chars() {
        if matches!(c, '{' | '}' | '[' | ']' | '!' | '|') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// HTML (Confluence storage format, Jira DC's rendered fields, Confluence excerpts) as plain text.
/// Block tags end a line, `<li>` starts `- `, `<br>` breaks, cells are joined with ` | `;
/// `<script>`, `<style>` and macro parameters are dropped; CDATA (code macros) is kept; entities
/// are decoded. A small tag scanner, not a parser: it never fails, it only loses formatting.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut rest = html;
    let mut skip_until: Option<&'static str> = None;
    while !rest.is_empty() {
        if let Some(end) = skip_until {
            match find_ci(rest, end) {
                Some(i) => {
                    rest = &rest[i + end.len()..];
                    if let Some(j) = rest.find('>') {
                        rest = &rest[j + 1..];
                    }
                    skip_until = None;
                }
                None => break,
            }
            continue;
        }
        if let Some(cdata) = rest.strip_prefix("<![CDATA[") {
            let end = cdata.find("]]>").unwrap_or(cdata.len());
            out.push_str(&cdata[..end]);
            rest = cdata.get(end + 3..).unwrap_or("");
            continue;
        }
        if let Some(c) = rest.strip_prefix("<!--") {
            rest = c.find("-->").map_or("", |i| &c[i + 3..]);
            continue;
        }
        let Some(lt) = rest.find('<') else {
            out.push_str(&decode(rest));
            break;
        };
        out.push_str(&decode(&rest[..lt]));
        rest = &rest[lt..];
        let Some(gt) = rest.find('>') else {
            out.push_str(&decode(rest));
            break;
        };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        let closing = tag.starts_with('/');
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric() || *c == ':' || *c == '-').collect::<String>().to_ascii_lowercase();
        match name.as_str() {
            "script" | "style" if !closing => skip_until = Some(if name == "script" { "</script" } else { "</style" }),
            "ac:parameter" if !closing && !tag.ends_with('/') => skip_until = Some("</ac:parameter"),
            "br" => out.push('\n'),
            "li" if !closing => {
                if !out.ends_with('\n') && !out.is_empty() {
                    out.push('\n');
                }
                out.push_str("- ");
            }
            "td" | "th" if closing => out.push_str(" | "),
            "tr" if closing => {
                let t = out.trim_end_matches([' ', '|']).len();
                out.truncate(t);
                out.push('\n');
            }
            "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "table" | "pre" | "blockquote" | "hr" | "ac:structured-macro" | "ac:rich-text-body" | "ac:plain-text-body" => {
                if !out.ends_with("\n\n") && !out.is_empty() {
                    out.push_str(if out.ends_with('\n') { "\n" } else { "\n\n" });
                }
            }
            "li" => out.push('\n'),
            "ri:page" | "ri:attachment" | "ri:url" => {
                if let Some(v) = attr(tag, "ri:content-title").or_else(|| attr(tag, "ri:filename")).or_else(|| attr(tag, "ri:value")) {
                    out.push_str(&decode(&v));
                }
            }
            _ => {}
        }
    }
    tidy(&out)
}

fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    hay.to_ascii_lowercase().find(needle)
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let i = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let end = tag[i..].find('"')?;
    Some(tag[i..i + end].to_string())
}

/// HTML entities: the named few Atlassian writes, and numeric ones.
pub fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(semi) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "hellip" => Some('…'),
            "rsquo" | "lsquo" => Some('\''),
            "rdquo" | "ldquo" => Some('"'),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Confluence search's highlight markers removed (`@@@hl@@@word@@@endhl@@@`).
pub fn strip_highlight(s: &str) -> String {
    s.replace("@@@hl@@@", "").replace("@@@endhl@@@", "")
}

/// Trailing spaces off each line, at most one blank line in a row, no blank lines at the ends.
fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0;
    for line in s.lines().map(str::trim_end) {
        if line.trim().is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        blank = 0;
        out.push_str(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adf_reads_as_plain_text() {
        let doc = json!({"type": "doc", "version": 1, "content": [
            {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Done when"}]},
            {"type": "bulletList", "content": [
                {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a link", "marks": [{"type": "strong"}]}]}]},
                {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "ask "}, {"type": "mention", "attrs": {"id": "u-ann", "text": "@Ann Lee"}}]}]}
            ]},
            {"type": "paragraph", "content": [{"type": "text", "text": "line one"}, {"type": "hardBreak"}, {"type": "text", "text": "line two"}]},
            {"type": "orderedList", "content": [
                {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "first"}]}]},
                {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "second"}]}]}
            ]},
            {"type": "mediaSingle", "content": [{"type": "media", "attrs": {"id": "x"}}]},
            {"type": "paragraph", "content": [{"type": "inlineCard", "attrs": {"url": "https://example.com/x"}}]},
            {"type": "table", "content": [{"type": "tableRow", "content": [
                {"type": "tableCell", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a"}]}]},
                {"type": "tableCell", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "b"}]}]}
            ]}]}
        ]});
        assert_eq!(
            adf_to_text(&doc),
            "Done when\n\n- a link\n- ask @Ann Lee\n\nline one\nline two\n\n1. first\n2. second\n\n[attachment]\n\nhttps://example.com/x\n\na | b"
        );
    }

    #[test]
    fn deep_adf_is_cut_not_followed() {
        let mut n = json!({"type": "text", "text": "bottom"});
        for _ in 0..500 {
            n = json!({"type": "blockquote", "content": [n]});
        }
        assert_eq!(adf_to_text(&json!({"type": "doc", "content": [n]})), "");
    }

    #[test]
    fn text_goes_to_adf_and_back() {
        let text = "Two questions before we start:\n\n- CSV or XLSX?\n- Which month first?\n\nReply here.\nThanks";
        let adf = text_to_adf(text);
        assert_eq!(adf["content"][1]["type"], "bulletList");
        assert_eq!(adf["content"][2]["content"][1]["type"], "hardBreak");
        assert_eq!(adf_to_text(&adf), text);
    }

    #[test]
    fn wiki_text_cannot_open_a_macro() {
        assert_eq!(text_to_wiki("see {code}rm{code} [x|http://e] !img.png!"), "see \\{code\\}rm\\{code\\} \\[x\\|http://e\\] \\!img.png\\!");
        assert_eq!(text_to_wiki("- one\n- two"), "- one\n- two");
    }

    #[test]
    fn storage_format_reads_as_plain_text() {
        let html = "<h1>Orders export</h1><p>An <strong>XLSX</strong> file &amp; more&nbsp;here.</p><ul><li>Dates</li><li>Totals</li></ul>\
            <ac:structured-macro ac:name=\"code\"><ac:parameter ac:name=\"language\">sh</ac:parameter><ac:plain-text-body><![CDATA[echo <hi>]]></ac:plain-text-body></ac:structured-macro>\
            <script>alert(1)</script><table><tr><th>a</th><th>b</th></tr><tr><td>1</td><td>2</td></tr></table>\
            <p><ac:link><ri:page ri:content-title=\"Order emails\" /></ac:link> &#8212; &#x41;</p>";
        assert_eq!(html_to_text(html), "Orders export\n\nAn XLSX file & more here.\n\n- Dates\n- Totals\n\necho <hi>\n\na | b\n1 | 2\n\nOrder emails — A");
    }

    #[test]
    fn broken_html_never_panics() {
        for s in ["<p", "a < b", "&", "&#xZZ;", "<script>never closed", "<![CDATA[x", "<!-- x", "é<b>ü</b>&amp"] {
            let _ = html_to_text(s);
        }
        assert_eq!(html_to_text("a < b"), "a < b");
        assert_eq!(strip_highlight("@@@hl@@@XLSX@@@endhl@@@ workbook"), "XLSX workbook");
    }
}
