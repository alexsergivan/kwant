//! A bot's own folder (`bot-workspaces/<kwant id>`, see [`crate::tools::bot_workspace`]) as two
//! read-only tools, `read_file` and `list_dir`, for engines that have no file tools of their own:
//! API engines (Custom API, API keys). CLI engines (Claude, Codex, Grok) read with their own tools
//! and are never offered these (GitHub issue #715, reported with this patch shape by calintro).
//!
//! Read-only, for the same reason as `space_docs`: a bot shouldn't write the words it then quotes.
//! Every path stays inside the folder: `..`, absolute paths, and symlinks (to a file or a folder)
//! that lead out of it are refused with a plain sentence, never file contents. A path must exist
//! to be read; nothing here creates anything.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};

pub const READ_FILE: &str = "read_file";
pub const LIST_DIR: &str = "list_dir";

/// Lines a `read_file` page has when the bot gives no `limit`.
pub const DEFAULT_LINES: usize = 600;
/// Most lines one `read_file` page has.
pub const MAX_LINES: usize = 2_000;
/// A longer line is cut, so one huge line (minified JSON, a data URI) can't fill the turn.
pub const MAX_LINE_CHARS: usize = 2_000;
/// Most bytes of text one page carries; past it the page ends early and says where to go on.
pub const MAX_PAGE_BYTES: usize = 100_000;
/// Biggest file `read_file` opens.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Most entries one `list_dir` answer names.
pub const MAX_ENTRIES: usize = 500;

/// What a path leading out of the folder gets.
pub const OUTSIDE: &str = "That path is outside your folder, so I can't open it. Use a path inside your folder, like notes.md or docs/plan.md.";

/// The tools, in the self server's schema shape.
pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": READ_FILE,
            "description": format!(
                "Read a text file in your own folder (the files the person put there for you). Give a path inside the folder, like notes.md or docs/plan.md. \
                Long files come in pages of {DEFAULT_LINES} lines (at most {MAX_LINES}): the answer says how many lines the file has in total and the offset to read on from. \
                Quote only what you read here, never from memory."
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "The file's path inside your folder, e.g. notes.md"},
                    "offset": {"type": "integer", "description": "Lines to skip from the start (0 = the first line). The answer gives the next offset."},
                    "limit": {"type": "integer", "description": format!("How many lines to read ({DEFAULT_LINES} if left out, at most {MAX_LINES}).")},
                },
                "required": ["path"],
            },
        }),
        json!({
            "name": LIST_DIR,
            "description": "List the files and folders in your own folder, or in a folder inside it. Leave path out for the folder itself.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "A folder inside your folder, e.g. docs. Leave out for the folder itself."},
                },
            },
        }),
    ]
}

pub fn is_tool(name: &str) -> bool {
    matches!(name, READ_FILE | LIST_DIR)
}

/// Runs `name` in `workspace`: `Ok` with the text the bot reads, `Err` with a plain sentence.
pub fn run(workspace: &Path, name: &str, args: &Value) -> Result<String, String> {
    let path = args["path"].as_str().unwrap_or("");
    match name {
        READ_FILE => {
            if path.trim().is_empty() {
                return Err("path is required: the file's path inside your folder, like notes.md.".into());
            }
            let offset = count_arg(&args["offset"]).unwrap_or(0);
            let limit = count_arg(&args["limit"]).unwrap_or(DEFAULT_LINES).clamp(1, MAX_LINES);
            read_file(workspace, path, offset, limit)
        }
        LIST_DIR => list_dir(workspace, path),
        _ => Err(format!("{name} isn't a folder tool.")),
    }
}

/// A non-negative count from a JSON number or numeric string; negatives count as 0.
fn count_arg(v: &Value) -> Option<usize> {
    if let Some(n) = v.as_u64() {
        return Some(usize::try_from(n).unwrap_or(usize::MAX));
    }
    if let Some(n) = v.as_i64() {
        return Some(n.max(0) as usize);
    }
    if let Some(f) = v.as_f64() {
        return Some(if f.is_finite() && f > 0.0 { f as usize } else { 0 });
    }
    v.as_str().and_then(|s| s.trim().parse::<usize>().ok())
}

/// `raw` inside `workspace`, as a real path that exists and doesn't lead out of it.
///
/// Stricter than `tools::file_ops::resolve_safe_path`: only plain names (and `.`) are allowed, so
/// `..`, `/abs` and `C:\` never reach the file system; the path must exist; and its canonical form
/// (every symlink followed) must still be under the folder's canonical form.
pub fn resolve(workspace: &Path, raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    let rel = Path::new(raw);
    if rel.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Err(OUTSIDE.into());
    }
    let root = workspace.canonicalize().map_err(|_| "I couldn't open your folder.".to_string())?;
    let real = match root.join(rel).canonicalize() {
        Ok(real) => real,
        Err(_) => {
            // A link whose target is gone: say it leads out when it does, never "missing" for it
            if root.join(rel).symlink_metadata().is_ok() {
                return Err(OUTSIDE.into());
            }
            return Err(format!("There's no {} in your folder.", shown(raw)));
        }
    };
    if !real.starts_with(&root) {
        return Err(OUTSIDE.into());
    }
    Ok(real)
}

/// `raw` as the bot named it, `your folder` for the folder itself.
fn shown(raw: &str) -> String {
    match raw.trim() {
        "" | "." | "./" => "your folder".into(),
        s => s.to_string(),
    }
}

fn read_file(workspace: &Path, raw: &str, offset: usize, limit: usize) -> Result<String, String> {
    let path = resolve(workspace, raw)?;
    let name = shown(raw);
    let meta = std::fs::metadata(&path).map_err(|_| format!("I couldn't open {name}."))?;
    if meta.is_dir() {
        return Err(format!("{name} is a folder: list it with list_dir."));
    }
    if !meta.is_file() {
        return Err(format!("{name} isn't a text file, so I can't read it here."));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("{name} is too big to read here ({} MB; the limit is {} MB).", meta.len() / (1024 * 1024), MAX_FILE_BYTES / (1024 * 1024)));
    }
    let file = std::fs::File::open(&path).map_err(|_| format!("I couldn't open {name}."))?;
    // The whole file is checked (and its lines counted), so a binary tail never reaches the page
    let mut reader = BufReader::new(file.take(MAX_FILE_BYTES + 1));
    let not_text = || format!("{name} isn't a text file, so I can't read it here.");
    let mut buf = Vec::new();
    let mut total = 0usize;
    let mut page = String::new();
    let mut shown_lines = 0usize;
    let mut full = false;
    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf).map_err(|_| format!("I couldn't read {name}."))?;
        if n == 0 {
            break;
        }
        if buf.contains(&0) {
            return Err(not_text());
        }
        let line = std::str::from_utf8(&buf).map_err(|_| not_text())?;
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        let i = total;
        total += 1;
        if i < offset || shown_lines >= limit || full {
            continue;
        }
        let cut = cut_line(line);
        if !page.is_empty() && page.len() + cut.len() + 1 > MAX_PAGE_BYTES {
            full = true;
            continue;
        }
        page.push_str(&cut);
        page.push('\n');
        shown_lines += 1;
    }
    let lines = |n: usize| if n == 1 { "1 line".to_string() } else { format!("{n} lines") };
    if total == 0 {
        return Ok(format!("{name}: 0 lines total (the file is empty)."));
    }
    if shown_lines == 0 {
        return Ok(format!("{name}: {} total. Nothing from offset {offset} on: the file ends at line {total}.", lines(total)));
    }
    let first = offset + 1;
    let last = offset + shown_lines;
    let next = if last < total {
        format!(" To read on, call read_file with offset {last}.")
    } else {
        " That's the end of the file.".to_string()
    };
    Ok(format!("{name}: {} total. Lines {first}–{last}:{next}\n\n{page}", lines(total)))
}

/// `line`, cut to [`MAX_LINE_CHARS`] characters with a note when longer.
fn cut_line(line: &str) -> String {
    match line.char_indices().nth(MAX_LINE_CHARS) {
        Some((at, _)) => format!("{}… [line cut: {} characters]", &line[..at], line.chars().count()),
        None => line.to_string(),
    }
}

fn list_dir(workspace: &Path, raw: &str) -> Result<String, String> {
    let path = resolve(workspace, raw)?;
    let name = shown(raw);
    if !path.is_dir() {
        return Err(format!("{name} is a file: read it with read_file."));
    }
    let read = std::fs::read_dir(&path).map_err(|_| format!("I couldn't list {name}."))?;
    let mut entries: Vec<(String, String)> = Vec::new();
    for entry in read.flatten() {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        // A link is shown as one, not followed: read_file says whether it stays inside the folder
        let kind = match entry.file_type() {
            Ok(t) if t.is_symlink() => format!("{file_name} (link)"),
            Ok(t) if t.is_dir() => format!("{file_name}/"),
            _ => {
                let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                format!("{file_name} ({})", size(bytes))
            }
        };
        entries.push((file_name, kind));
    }
    entries.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()).then(a.0.cmp(&b.0)));
    if entries.is_empty() {
        return Ok(format!("{name} is empty."));
    }
    let total = entries.len();
    let mut out = format!("{name}: {} {}.\n", total, if total == 1 { "entry" } else { "entries" });
    for (_, line) in entries.iter().take(MAX_ENTRIES) {
        out.push_str(line);
        out.push('\n');
    }
    if total > MAX_ENTRIES {
        out.push_str(&format!("… and {} more not listed.\n", total - MAX_ENTRIES));
    }
    Ok(out)
}

fn size(bytes: u64) -> String {
    match bytes {
        1 => "1 byte".into(),
        b if b < 1024 => format!("{b} bytes"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("space").join("bot-workspaces").join("bot_a");
        std::fs::create_dir_all(&ws).unwrap();
        (tmp, ws)
    }

    fn read(ws: &Path, args: Value) -> Result<String, String> {
        run(ws, READ_FILE, &args)
    }

    #[test]
    fn reads_a_file_and_says_how_many_lines_it_has() {
        let (_t, ws) = folder();
        std::fs::write(ws.join("notes.md"), "one\ntwo\r\nthree").unwrap();
        let text = read(&ws, json!({"path": "notes.md"})).unwrap();
        assert!(text.starts_with("notes.md: 3 lines total. Lines 1–3: That's the end of the file.\n\none\ntwo\nthree\n"), "{text}");
        // ./ and nested paths are fine
        std::fs::create_dir_all(ws.join("docs")).unwrap();
        std::fs::write(ws.join("docs/plan.md"), "plan\n").unwrap();
        assert!(read(&ws, json!({"path": "./docs/plan.md"})).unwrap().contains("1 line total"));
    }

    #[test]
    fn a_long_file_reads_in_pages_and_an_offset_past_the_end_says_so() {
        let (_t, ws) = folder();
        let body: String = (1..=7_000).map(|i| format!("line {i}\n")).collect();
        std::fs::write(ws.join("big.txt"), body).unwrap();
        let first = read(&ws, json!({"path": "big.txt"})).unwrap();
        assert!(first.starts_with("big.txt: 7000 lines total. Lines 1–600: To read on, call read_file with offset 600."), "{first}");
        assert!(first.ends_with("line 600\n") && !first.contains("line 601\n"));
        // The bot pages on with the offset it was given
        let mut offset = 600;
        let mut pages = 1;
        loop {
            let page = read(&ws, json!({"path": "big.txt", "offset": offset, "limit": 2000})).unwrap();
            pages += 1;
            if page.contains("That's the end of the file.") {
                assert!(page.ends_with("line 7000\n"), "{}", &page[..200]);
                break;
            }
            let next = page.split("offset ").nth(1).unwrap().split('.').next().unwrap().parse::<usize>().unwrap();
            assert_eq!(next, offset + 2000);
            offset = next;
        }
        assert_eq!(pages, 5, "600 + 2000 + 2000 + 2000 + 400");
        // limit is capped at 2,000 and at least 1
        let capped = read(&ws, json!({"path": "big.txt", "limit": 50_000})).unwrap();
        assert!(capped.contains("Lines 1–2000:"), "{}", &capped[..120]);
        let one = read(&ws, json!({"path": "big.txt", "limit": 0, "offset": "9"})).unwrap();
        assert!(one.contains("Lines 10–10:"), "{}", &one[..120]);
        // Past the end: an empty page and the total
        let past = read(&ws, json!({"path": "big.txt", "offset": 8000})).unwrap();
        assert_eq!(past, "big.txt: 7000 lines total. Nothing from offset 8000 on: the file ends at line 7000.");
        let neg = read(&ws, json!({"path": "big.txt", "offset": -5, "limit": 1})).unwrap();
        assert!(neg.contains("Lines 1–1:"), "{neg}");
    }

    #[test]
    fn a_huge_line_is_cut_and_a_page_stays_small() {
        let (_t, ws) = folder();
        let long = "x".repeat(50_000);
        std::fs::write(ws.join("min.json"), format!("{long}\nshort\n")).unwrap();
        let text = read(&ws, json!({"path": "min.json"})).unwrap();
        assert!(text.contains("… [line cut: 50000 characters]"), "{}", &text[..200]);
        assert!(text.contains("\nshort\n"));
        assert!(text.len() < MAX_LINE_CHARS + 300, "{}", text.len());
        // Many long lines: the page ends early at its byte budget and says where to go on
        let wide: String = (0..2_000).map(|i| format!("{i:04}{}\n", "y".repeat(1_500))).collect();
        std::fs::write(ws.join("wide.txt"), wide).unwrap();
        let page = read(&ws, json!({"path": "wide.txt", "limit": 2000})).unwrap();
        assert!(page.len() <= MAX_PAGE_BYTES + 200, "{}", page.len());
        let shown = page.lines().filter(|l| l.contains("yyy")).count();
        assert!(shown > 0 && shown < 2_000, "{shown}");
        assert!(page.contains(&format!("To read on, call read_file with offset {shown}.")), "{}", &page[..200]);
    }

    #[test]
    fn binary_and_non_utf8_files_are_not_text() {
        let (_t, ws) = folder();
        std::fs::write(ws.join("img.png"), b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR").unwrap();
        std::fs::write(ws.join("latin1.txt"), b"caf\xe9 au lait\n").unwrap();
        // Text first, a binary tail later: still refused, never a page of the text part
        let mut tail = "fine line\n".repeat(700).into_bytes();
        tail.extend_from_slice(b"\0\x01\x02");
        std::fs::write(ws.join("mixed.log"), tail).unwrap();
        for f in ["img.png", "latin1.txt", "mixed.log"] {
            let e = read(&ws, json!({"path": f})).unwrap_err();
            assert_eq!(e, format!("{f} isn't a text file, so I can't read it here."));
        }
        assert_eq!(read(&ws, json!({"path": "empty.txt"})).unwrap_err(), "There's no empty.txt in your folder.");
        std::fs::write(ws.join("empty.txt"), "").unwrap();
        assert_eq!(read(&ws, json!({"path": "empty.txt"})).unwrap(), "empty.txt: 0 lines total (the file is empty).");
    }

    #[test]
    fn paths_out_of_the_folder_are_refused_with_no_contents() {
        let (tmp, ws) = folder();
        let secret = tmp.path().join("secret.txt");
        std::fs::write(&secret, "TOP SECRET\n").unwrap();
        // A teammate's folder in the same space
        let other = ws.parent().unwrap().join("bot_b");
        std::fs::create_dir_all(&other).unwrap();
        let theirs = other.join("notes.md");
        std::fs::write(&theirs, "B'S NOTES\n").unwrap();
        let bad = [
            "../secret.txt",
            "../../secret.txt",
            "../bot_b/notes.md",
            "docs/../../bot_b/notes.md",
            "..",
            secret.to_str().unwrap(),
            theirs.to_str().unwrap(),
            "/etc/passwd",
            "/",
        ];
        for p in bad {
            for tool in [READ_FILE, LIST_DIR] {
                let e = run(&ws, tool, &json!({"path": p})).unwrap_err();
                assert_eq!(e, OUTSIDE, "{tool} {p}");
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(&secret, ws.join("link.txt")).unwrap();
            symlink(tmp.path(), ws.join("up")).unwrap();
            symlink(&other, ws.join("teammate")).unwrap();
            symlink(tmp.path().join("gone.txt"), ws.join("dangling.txt")).unwrap();
            // A link into a folder that doesn't exist yet: resolve_safe_path's "new file" branch
            // would let `dangling_dir/x` through; here it is refused
            symlink(tmp.path().join("later"), ws.join("dangling_dir")).unwrap();
            for p in ["link.txt", "up/secret.txt", "up", "teammate/notes.md", "teammate", "dangling.txt"] {
                for tool in [READ_FILE, LIST_DIR] {
                    let e = run(&ws, tool, &json!({"path": p})).unwrap_err();
                    assert_eq!(e, OUTSIDE, "{tool} {p}");
                }
            }
            let e = read(&ws, json!({"path": "dangling_dir/x.txt"})).unwrap_err();
            assert!(!e.contains("SECRET") && e.starts_with("There's no"), "{e}");
            std::fs::create_dir_all(tmp.path().join("later")).unwrap();
            std::fs::write(tmp.path().join("later/x.txt"), "LATER SECRET\n").unwrap();
            assert_eq!(read(&ws, json!({"path": "dangling_dir/x.txt"})).unwrap_err(), OUTSIDE);
            // A link that stays inside the folder is fine
            std::fs::write(ws.join("real.md"), "inside\n").unwrap();
            symlink(ws.join("real.md"), ws.join("alias.md")).unwrap();
            assert!(read(&ws, json!({"path": "alias.md"})).unwrap().contains("inside"));
            // The listing names links without following them
            let listed = run(&ws, LIST_DIR, &json!({})).unwrap();
            assert!(listed.contains("link.txt (link)") && listed.contains("up (link)"), "{listed}");
            assert!(!listed.contains("secret.txt"), "{listed}");
        }
    }

    #[test]
    fn lists_the_folder_sorted_with_kinds() {
        let (_t, ws) = folder();
        std::fs::write(ws.join("b.txt"), "12345").unwrap();
        std::fs::write(ws.join("A.md"), "").unwrap();
        std::fs::create_dir_all(ws.join("docs")).unwrap();
        std::fs::write(ws.join("docs/plan.md"), "p").unwrap();
        let text = run(&ws, LIST_DIR, &json!({})).unwrap();
        assert_eq!(text, "your folder: 3 entries.\nA.md (0 bytes)\nb.txt (5 bytes)\ndocs/\n");
        assert_eq!(run(&ws, LIST_DIR, &json!({"path": "docs"})).unwrap(), "docs: 1 entry.\nplan.md (1 byte)\n");
        assert_eq!(run(&ws, LIST_DIR, &json!({"path": "b.txt"})).unwrap_err(), "b.txt is a file: read it with read_file.");
        assert_eq!(read(&ws, json!({"path": "docs"})).unwrap_err(), "docs is a folder: list it with list_dir.");
        assert_eq!(read(&ws, json!({})).unwrap_err(), "path is required: the file's path inside your folder, like notes.md.");
        std::fs::create_dir_all(ws.join("empty")).unwrap();
        assert_eq!(run(&ws, LIST_DIR, &json!({"path": "empty"})).unwrap(), "empty is empty.");
    }

    #[test]
    fn the_schemas_offer_exactly_the_two_read_tools() {
        let names: Vec<String> = tools().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, [READ_FILE, LIST_DIR]);
        assert!(is_tool("read_file") && is_tool("list_dir") && !is_tool("write_file"));
        assert_eq!(tools()[0]["inputSchema"]["required"], json!(["path"]));
    }
}
