//! The perch (desktop-island/design.md, build-brief.md Part 2): pure rules. No Tauri and no I/O
//! apart from the settings file, so every rule here has a unit test.
//!
//! - **What it shows** comes from each place's `GET /api/glance` (or `/api/away` on an older
//!   server). Every string is cleaned and capped here before the page sees it, and the page puts
//!   it in with `textContent`.
//! - **What it may ask** comes back as `kwant-island://…` navigations. A thing is named by its
//!   place index and its id, and Rust acts only on an id that is in the latest snapshot: Allow
//!   once / Deny only for an approval whose `can_allow_inline` is true, a chip only for one of
//!   the first 3 choices, a Quick ask only to one of the listed targets.
//! - **Where it hangs** is worked out from the screen's notch metrics, which are checked first.

use crate::nudge;
use crate::switcher::{PerchMode, PerchShortcut};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

/// The perch window's label. It isn't in `capabilities/connect.json`: it can call no command.
pub const WINDOW: &str = "perch";
/// Settings keys in `desktop-app.json` (next to `show_nudges`).
const PREF_MODE: &str = "perch_mode";
const PREF_SHORTCUT: &str = "perch_shortcut";

/// Lists the page gets at most (the server caps them too: impl-server.md).
const MAX_NEEDS: usize = 20;
const MAX_WORKING: usize = 12;
const MAX_FINISHED: usize = 5;
const MAX_ASK: usize = 8;
const MAX_CHOICES: usize = 6;
/// Chips shown for a question (design.md §7.1).
pub const CHIPS: usize = 3;
/// Longest Quick ask (the server's own cap).
pub const ASK_MAX: usize = 8000;
/// Longest `kwant-island://` address read at all (an 8,000-character ask, percent-encoded).
const MAX_URL: usize = 100_000;
/// How long a peek or celebrate stays in the updates the page gets (it shows each one once).
pub const EVENT_CARRY: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------------------------
// Settings

/// The perch settings (Settings → App settings → Perch).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prefs {
    pub mode: PerchMode,
    pub shortcut: PerchShortcut,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs { mode: PerchMode::Busy, shortcut: PerchShortcut::K }
    }
}

impl Prefs {
    /// The saved settings; a missing, unreadable or odd value means the default.
    pub fn load(data_dir: &Path) -> Prefs {
        let saved = std::fs::read_to_string(nudge::prefs_path(data_dir))
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .unwrap_or(Value::Null);
        Prefs::from_json(&saved)
    }

    pub fn from_json(v: &Value) -> Prefs {
        let d = Prefs::default();
        Prefs {
            mode: v[PREF_MODE].as_str().and_then(PerchMode::parse).unwrap_or(d.mode),
            shortcut: v[PREF_SHORTCUT].as_str().and_then(PerchShortcut::parse).unwrap_or(d.shortcut),
        }
    }

    /// Saves the settings, keeping every other key the file holds (`show_nudges`).
    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        let path = nudge::prefs_path(data_dir);
        let mut v = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .filter(|v| v.is_object())
            .unwrap_or_else(|| json!({}));
        v[PREF_MODE] = Value::from(self.mode.as_str());
        v[PREF_SHORTCUT] = Value::from(self.shortcut.as_str());
        std::fs::create_dir_all(data_dir)?;
        let tmp = path.with_extension("json.perch.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&v).unwrap_or_default())?;
        std::fs::rename(tmp, path)
    }

    /// Whether the perch may show at all (and the Mac pill steps aside).
    pub fn on(&self, available: bool) -> bool {
        available && self.mode != PerchMode::Off
    }

    /// The shortcut to register (none while the mode is Off: Settings disables it then).
    pub fn active_shortcut(&self, available: bool) -> PerchShortcut {
        if self.on(available) {
            self.shortcut
        } else {
            PerchShortcut::Off
        }
    }
}

/// Whether this computer can show a perch: not on Wayland (design.md §8), unless the app runs
/// through XWayland (`GDK_BACKEND=x11`). `os` is `std::env::consts::OS`.
pub fn available(os: &str, wayland_display: Option<&str>, session_type: Option<&str>, gdk_backend: Option<&str>) -> bool {
    if matches!(os, "macos" | "windows") {
        return true;
    }
    let set = |v: Option<&str>| v.is_some_and(|s| !s.trim().is_empty());
    if gdk_backend.is_some_and(|b| b.split(',').next().map(str::trim) == Some("x11")) {
        return true;
    }
    let wayland = session_type.is_some_and(|s| s.trim().eq_ignore_ascii_case("wayland")) || set(wayland_display);
    !wayland
}

/// The global shortcut to register, in the plugin's accelerator words.
pub fn accelerator(shortcut: PerchShortcut, mac: bool) -> Option<&'static str> {
    match (shortcut, mac) {
        (PerchShortcut::K, true) => Some("Alt+Super+K"),
        (PerchShortcut::K, false) => Some("Ctrl+Alt+K"),
        (PerchShortcut::Space, _) => Some("Ctrl+Alt+Space"),
        (PerchShortcut::Off, _) => None,
    }
}

/// The shortcut as the page shows it ("⌥⌘K opens and closes this").
pub fn key_label(shortcut: PerchShortcut, mac: bool) -> Option<&'static str> {
    match (shortcut, mac) {
        (PerchShortcut::K, true) => Some("⌥⌘K"),
        (PerchShortcut::K, false) => Some("Ctrl+Alt+K"),
        (PerchShortcut::Space, true) => Some("⌃⌥Space"),
        (PerchShortcut::Space, false) => Some("Ctrl+Alt+Space"),
        (PerchShortcut::Off, _) => None,
    }
}

// ---------------------------------------------------------------------------------------------
// What a place's kwant says (GET /api/glance)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeedKind {
    Approval,
    Question,
    HumanStep,
    Watch,
    Stopped,
}

impl NeedKind {
    fn parse(s: &str) -> Option<NeedKind> {
        Some(match s {
            "approval" => NeedKind::Approval,
            "question" => NeedKind::Question,
            "human_step" => NeedKind::HumanStep,
            "watch" => NeedKind::Watch,
            "stopped" => NeedKind::Stopped,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            NeedKind::Approval => "approval",
            NeedKind::Question => "question",
            NeedKind::HumanStep => "human_step",
            NeedKind::Watch => "watch",
            NeedKind::Stopped => "stopped",
        }
    }
}

/// One thing that needs the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Need {
    pub kind: NeedKind,
    pub id: String,
    pub space_id: String,
    pub space_name: String,
    pub kwant_name: String,
    pub avatar: Option<String>,
    pub title: String,
    pub since: String,
    pub path: String,
    /// An approval the server lets the perch allow or deny in place.
    pub inline: bool,
    pub why_open: String,
    /// A question's choices (at most 6) and how many it has in all.
    pub question: Option<Question>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub choices: Vec<String>,
    pub total: usize,
    pub allow_text: bool,
}

/// A turn that is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Work {
    pub id: String,
    pub kwant_name: String,
    pub avatar: Option<String>,
    pub step: String,
    pub since: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Verified,
    NotChecked,
    Failed,
}

/// Something that just finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    pub id: String,
    pub kwant_name: String,
    pub avatar: Option<String>,
    pub title: String,
    pub unseen: bool,
    pub check: Option<Check>,
    pub path: String,
}

/// A bot the Quick ask may write to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskTarget {
    pub kwant_id: String,
    pub name: String,
    pub avatar: Option<String>,
    pub space_id: String,
}

/// One place's glance, cleaned.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Glance {
    pub needs_count: usize,
    pub working_count: usize,
    pub needs: Vec<Need>,
    pub working: Vec<Work>,
    pub finished: Vec<Done>,
    pub ask: Vec<AskTarget>,
}

/// An id as kwant makes them: a UUID or `spc_default`.
pub fn id_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Text for the page: no control or direction-override characters, trimmed, at most `max`
/// characters (cut with "…").
pub fn clean(s: &str, max: usize) -> String {
    let kept: String = s
        .chars()
        .map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_control() && !matches!(*c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{FEFF}'))
        .collect();
    let t = kept.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let mut out: String = t.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// A sprite name (`architect`, `fox`), else `None`: the page then draws the name's creature.
pub fn avatar_ok(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    let ok = !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    ok.then(|| s.to_string())
}

/// An in-app path (`/s/spc_default/inbox`): one leading slash, no backslash, no spaces or
/// control characters. Anything else is `None` (the row opens the Inbox instead).
pub fn path_ok(s: &str) -> Option<String> {
    let ok = s.starts_with('/')
        && !s.starts_with("//")
        && s.len() <= 512
        && !s.contains('\\')
        && s.chars().all(|c| c.is_ascii_graphic());
    ok.then(|| s.to_string())
}

/// A timestamp as the server writes them (RFC 3339), else empty.
fn time_ok(v: &Value) -> String {
    let s = v.as_str().unwrap_or("");
    let ok = s.len() <= 40 && s.bytes().all(|b| b.is_ascii_digit() || b"-:.TZ+".contains(&b));
    if ok {
        s.to_string()
    } else {
        String::new()
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn count(v: &Value) -> usize {
    v.as_u64().map(|n| n.min(1_000_000) as usize).unwrap_or(0)
}

fn space_or_default(v: &Value) -> String {
    let id = s(v, "space_id");
    if id_ok(id) {
        id.to_string()
    } else {
        "spc_default".to_string()
    }
}

/// Reads a `/api/glance` answer. `None`: not a glance (the place is shown as before).
pub fn parse_glance(body: &str) -> Option<Glance> {
    let v: Value = serde_json::from_str(body).ok()?;
    if !v.is_object() || !v["counts"].is_object() {
        return None;
    }
    let list = |k: &str| v[k].as_array().cloned().unwrap_or_default();
    let needs: Vec<Need> = list("needs_you")
        .iter()
        .filter_map(|n| {
            let kind = NeedKind::parse(s(n, "kind"))?;
            let id = s(n, "id");
            if !id_ok(id) {
                return None;
            }
            let space_id = space_or_default(n);
            let approval = &n["approval"];
            let inline = kind == NeedKind::Approval && approval["can_allow_inline"].as_bool() == Some(true);
            let question = (kind == NeedKind::Question && n["question"].is_object()).then(|| {
                let q = &n["question"];
                let choices: Vec<String> = q["choices"]
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_str).map(|c| clean(c, 60)).take(MAX_CHOICES).collect())
                    .unwrap_or_default();
                let total = q["choices_total"].as_u64().map(|t| t as usize).unwrap_or(choices.len());
                Question { total: total.max(choices.len()).min(1000), choices, allow_text: q["allow_text"].as_bool() == Some(true) }
            });
            Some(Need {
                kind,
                id: id.to_string(),
                path: path_ok(s(n, "path")).unwrap_or_else(|| nudge::inbox_path(Some(&space_id))),
                space_id,
                space_name: clean(s(n, "space_name"), 60),
                kwant_name: clean(s(n, "kwant_name"), 60),
                avatar: avatar_ok(&n["avatar"]),
                title: clean(s(n, "title"), 120),
                since: time_ok(&n["since"]),
                inline,
                why_open: clean(approval["why_open"].as_str().unwrap_or(""), 120),
                question,
            })
        })
        .take(MAX_NEEDS)
        .collect();
    let working: Vec<Work> = list("working")
        .iter()
        .filter_map(|w| {
            let id = s(w, "turn_id");
            if !id_ok(id) {
                return None;
            }
            let space_id = space_or_default(w);
            let since = Some(time_ok(&w["step_started_at"])).filter(|t| !t.is_empty()).unwrap_or_else(|| time_ok(&w["started_at"]));
            Some(Work {
                id: id.to_string(),
                kwant_name: clean(s(w, "kwant_name"), 60),
                avatar: avatar_ok(&w["avatar"]),
                step: clean(s(w, "step"), 80),
                since,
                path: path_ok(s(w, "path")).unwrap_or_else(|| nudge::inbox_path(Some(&space_id))),
            })
        })
        .take(MAX_WORKING)
        .collect();
    let finished: Vec<Done> = list("finished")
        .iter()
        .filter_map(|f| {
            let id = s(f, "id");
            if !id_ok(id) {
                return None;
            }
            let space_id = space_or_default(f);
            let check = match f["check"]["status"].as_str() {
                Some("verified") => Some(Check::Verified),
                Some("not_checked") => Some(Check::NotChecked),
                Some("failed") => Some(Check::Failed),
                _ => None,
            };
            Some(Done {
                id: id.to_string(),
                kwant_name: clean(s(f, "kwant_name"), 60),
                avatar: avatar_ok(&f["avatar"]),
                title: clean(s(f, "title"), 120),
                unseen: f["unseen"].as_bool() == Some(true),
                check,
                path: path_ok(s(f, "path")).unwrap_or_else(|| nudge::inbox_path(Some(&space_id))),
            })
        })
        .take(MAX_FINISHED)
        .collect();
    let ask: Vec<AskTarget> = list("ask_targets")
        .iter()
        .filter_map(|a| {
            let id = s(a, "kwant_id");
            let space = s(a, "space_id");
            let name = clean(s(a, "name"), 60);
            (id_ok(id) && id_ok(space) && !name.is_empty()).then(|| AskTarget {
                kwant_id: id.to_string(),
                name,
                avatar: avatar_ok(&a["avatar"]),
                space_id: space.to_string(),
            })
        })
        .take(MAX_ASK)
        .collect();
    let c = &v["counts"];
    Some(Glance {
        needs_count: count(&c["needs_you"]).max(needs.len()),
        working_count: count(&c["working"]).max(working.len()),
        needs,
        working,
        finished,
        ask,
    })
}

/// What an older server (no `/api/glance`) still tells: what waits, from `/api/away`. Nothing
/// in it can be settled in the perch: every item is "Open in kwant".
pub fn from_waiting(w: &nudge::Waiting) -> Glance {
    let needs: Vec<Need> = w
        .items
        .iter()
        .filter(|i| id_ok(&i.id))
        .map(|i| {
            let space_id = i.space_id.clone().filter(|s| id_ok(s)).unwrap_or_else(|| "spc_default".into());
            Need {
                kind: match i.kind {
                    nudge::Kind::Approval => NeedKind::Approval,
                    nudge::Kind::Question => NeedKind::Question,
                },
                id: i.id.clone(),
                path: path_ok(&nudge::target_path(i)).unwrap_or_else(|| nudge::inbox_path(Some(&space_id))),
                space_id,
                space_name: String::new(),
                kwant_name: clean(&i.kwant_name, 60),
                avatar: None,
                title: clean(&i.title, 120),
                since: String::new(),
                inline: false,
                why_open: String::new(),
                question: None,
            }
        })
        .take(MAX_NEEDS)
        .collect();
    Glance { needs_count: w.count.max(needs.len()), needs, ..Glance::default() }
}

// ---------------------------------------------------------------------------------------------
// Every place at once

/// One place as the perch sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceView {
    /// `this_mac` or a server's origin (as `places::Place::key`).
    pub key: String,
    /// "This Mac" / "This computer", or the host.
    pub label: String,
    pub this_mac: bool,
    /// The place didn't answer (or is signed out).
    pub off: bool,
    pub current: bool,
    pub glance: Glance,
}

/// What every place said at the last poll. Asks from the page are checked against this.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub places: Vec<PlaceView>,
}

impl Snapshot {
    /// Everything that needs the person, newest first, as (place index, need).
    pub fn needs(&self) -> Vec<(usize, &Need)> {
        let mut all: Vec<(usize, &Need)> =
            self.places.iter().enumerate().flat_map(|(p, v)| v.glance.needs.iter().map(move |n| (p, n))).collect();
        // newest first; items with no time keep their order after the dated ones
        all.sort_by(|a, b| b.1.since.cmp(&a.1.since));
        all
    }

    pub fn working(&self) -> Vec<(usize, &Work)> {
        self.places.iter().enumerate().flat_map(|(p, v)| v.glance.working.iter().map(move |w| (p, w))).collect()
    }

    /// Unseen first, then the rest.
    pub fn finished(&self) -> Vec<(usize, &Done)> {
        let mut all: Vec<(usize, &Done)> =
            self.places.iter().enumerate().flat_map(|(p, v)| v.glance.finished.iter().map(move |f| (p, f))).collect();
        all.sort_by_key(|(_, f)| !f.unseen);
        all
    }

    /// The Quick ask list: This Mac's bots first, then each place's, at most 8.
    pub fn ask_targets(&self) -> Vec<(usize, &AskTarget)> {
        self.places
            .iter()
            .enumerate()
            .flat_map(|(p, v)| v.glance.ask.iter().map(move |a| (p, a)))
            .take(MAX_ASK)
            .collect()
    }

    pub fn needs_count(&self) -> usize {
        self.places.iter().map(|p| p.glance.needs_count).sum()
    }

    pub fn working_count(&self) -> usize {
        self.places.iter().map(|p| p.glance.working_count).sum()
    }

    /// Nothing needs the person and nobody is working.
    pub fn quiet(&self) -> bool {
        self.needs_count() == 0 && self.working_count() == 0
    }

    fn need(&self, p: usize, id: &str) -> Option<&Need> {
        self.places.get(p)?.glance.needs.iter().find(|n| n.id == id)
    }
}

/// What the page asked for through `kwant-island://`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PerchAsk {
    /// The page loaded and can take updates.
    Ready,
    /// What the page needs: its state and content size (CSS px, without the margin).
    Size { state: PageState, w: u32, h: u32 },
    /// Allow once / Deny an approval.
    Allow { p: usize, id: String },
    Deny { p: usize, id: String },
    /// A question's chip.
    Answer { p: usize, id: String, choice: usize },
    /// Open a thing (a need, a turn, a finish) in its place's window.
    Open { p: usize, id: String },
    /// Open one of the fixed pages.
    OpenPage(Page),
    /// Send a Quick ask to target `t` (an index into the snapshot's ask list).
    Ask { t: usize, text: String, cid: String },
    /// Switch to place `i`; `Places`: the picker (the "+N" chip).
    Place { i: usize },
    Places,
    /// Quick ask was clicked: the panel may take keys now.
    Focus,
    /// Esc, a click outside, the time ran out.
    Collapse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// "and N more · Open Inbox"
    Inbox,
    /// "and N more · Open Live now"
    Live,
    /// "and N more · Open Activity"
    Activity,
    /// "Open kwant"
    App,
}

/// The page's state, as it reports it with its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageState {
    /// Nothing to show: the window hides.
    Hidden,
    Rest,
    Celebrate,
    Peek,
    Expanded,
}

impl PageState {
    fn parse(s: &str) -> Option<PageState> {
        Some(match s {
            "hidden" => PageState::Hidden,
            "rest" => PageState::Rest,
            "celebrate" => PageState::Celebrate,
            "peek" => PageState::Peek,
            "expanded" => PageState::Expanded,
            _ => return None,
        })
    }
    /// Open states get the 24 px shadow margin and may take clicks into Quick ask.
    pub fn open(self) -> bool {
        matches!(self, PageState::Peek | PageState::Expanded)
    }
}

fn split_query(rest: &str) -> (&str, &str) {
    let rest = rest.split('#').next().unwrap_or("");
    rest.split_once('?').unwrap_or((rest, ""))
}

fn param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == name).then_some(v)
    })
}

/// Digits only, at most `len` of them.
fn small_number(s: &str, len: usize) -> Option<usize> {
    if s.is_empty() || s.len() > len || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Percent-decodes a query value (`+` is a space) into UTF-8; `None` for broken escapes or bytes.
pub fn decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let hex = |c: u8| (c as char).to_digit(16);
                let hi = hex(*b.get(i + 1)?)?;
                let lo = hex(*b.get(i + 2)?)?;
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Reads a `kwant-island://` address. `None`: not one we know (refused and ignored).
pub fn parse_perch(url: &str) -> Option<PerchAsk> {
    if url.len() > MAX_URL {
        return None;
    }
    let rest = url.strip_prefix("kwant-island://")?;
    let (host, q) = split_query(rest);
    let thing = || -> Option<(usize, String)> {
        let p = small_number(param(q, "p")?, 2)?;
        let id = param(q, "id")?;
        id_ok(id).then(|| (p, id.to_string()))
    };
    Some(match host.trim_end_matches('/') {
        "ready" => PerchAsk::Ready,
        "size" => {
            let state = PageState::parse(param(q, "s")?)?;
            let w = small_number(param(q, "w")?, 4)? as u32;
            let h = small_number(param(q, "h")?, 4)? as u32;
            PerchAsk::Size { state, w, h }
        }
        "allow" => {
            let (p, id) = thing()?;
            PerchAsk::Allow { p, id }
        }
        "deny" => {
            let (p, id) = thing()?;
            PerchAsk::Deny { p, id }
        }
        "answer" => {
            let (p, id) = thing()?;
            let choice = small_number(param(q, "c")?, 1)?;
            PerchAsk::Answer { p, id, choice }
        }
        "open" => match param(q, "what") {
            Some("inbox") => PerchAsk::OpenPage(Page::Inbox),
            Some("live") => PerchAsk::OpenPage(Page::Live),
            Some("activity") => PerchAsk::OpenPage(Page::Activity),
            Some("app") => PerchAsk::OpenPage(Page::App),
            Some(_) => return None,
            None => {
                let (p, id) = thing()?;
                PerchAsk::Open { p, id }
            }
        },
        "ask" => {
            let t = small_number(param(q, "t")?, 1)?;
            let text = decode(param(q, "text")?)?;
            let text = text.trim();
            if text.is_empty() || text.chars().count() > ASK_MAX {
                return None;
            }
            let cid = param(q, "cid")?;
            let tail = cid.strip_prefix("island-")?;
            if tail.len() < 8 || tail.len() > 64 || !tail.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                return None;
            }
            PerchAsk::Ask { t, text: text.to_string(), cid: cid.to_string() }
        }
        "place" => PerchAsk::Place { i: small_number(param(q, "i")?, 2)? },
        "places" => PerchAsk::Places,
        "focus" => PerchAsk::Focus,
        "collapse" => PerchAsk::Collapse,
        _ => return None,
    })
}

/// A call to make on a place's kwant, after the ask was checked against the snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    /// The place index in the snapshot (whose window and sign-in it uses).
    pub p: usize,
    pub path: String,
    pub body: Value,
    /// What the page is told when it comes back (`kwantPerch.done({op, id, ok})`).
    pub op: &'static str,
    pub id: String,
}

/// Why an ask was refused (the page shows "Didn't go through. Open it in kwant.").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// Not in the latest snapshot (gone, never there, or another place's id).
    NotThere,
    /// In the snapshot, but not something the perch may settle (needs the full card).
    NotInline,
}

/// The call an Allow once / Deny / chip / Quick ask makes, only for a thing in `snap`.
pub fn call_for(snap: &Snapshot, ask: &PerchAsk) -> Result<Call, Refused> {
    match ask {
        PerchAsk::Allow { p, id } | PerchAsk::Deny { p, id } => {
            let need = snap.need(*p, id).ok_or(Refused::NotThere)?;
            if need.kind != NeedKind::Approval || !need.inline {
                return Err(Refused::NotInline);
            }
            let allow = matches!(ask, PerchAsk::Allow { .. });
            Ok(Call {
                p: *p,
                path: format!("/api/approvals/{}/resolve", nudge::encode(id)),
                // never always_allow from the perch
                body: json!({ "decision": if allow { "allow_once" } else { "deny" } }),
                op: if allow { "allow" } else { "deny" },
                id: id.clone(),
            })
        }
        PerchAsk::Answer { p, id, choice } => {
            let need = snap.need(*p, id).ok_or(Refused::NotThere)?;
            let q = need.question.as_ref().filter(|_| need.kind == NeedKind::Question).ok_or(Refused::NotInline)?;
            if *choice >= CHIPS.min(q.choices.len()) || *choice >= q.total {
                return Err(Refused::NotInline);
            }
            Ok(Call {
                p: *p,
                path: format!("/api/questions/{}/answer", nudge::encode(id)),
                body: json!({ "choice": choice }),
                op: "answer",
                id: id.clone(),
            })
        }
        PerchAsk::Ask { t, text, cid } => {
            let targets = snap.ask_targets();
            let (p, target) = targets.get(*t).ok_or(Refused::NotThere)?;
            Ok(Call {
                p: *p,
                path: "/api/glance/ask".to_string(),
                body: json!({ "space_id": target.space_id, "kwant_id": target.kwant_id, "text": text, "client_message_id": cid }),
                op: "ask",
                id: cid.clone(),
            })
        }
        _ => Err(Refused::NotThere),
    }
}

/// Where Open goes: (place index, in-app path). Only for things in `snap`; a page link uses
/// the first place (This Mac when it runs).
pub fn open_target(snap: &Snapshot, ask: &PerchAsk) -> Option<(usize, String)> {
    match ask {
        PerchAsk::Open { p, id } => {
            let g = &snap.places.get(*p)?.glance;
            g.needs
                .iter()
                .find(|n| &n.id == id)
                .map(|n| n.path.clone())
                .or_else(|| g.working.iter().find(|w| &w.id == id).map(|w| w.path.clone()))
                .or_else(|| g.finished.iter().find(|f| &f.id == id).map(|f| f.path.clone()))
                .map(|path| (*p, path))
        }
        PerchAsk::OpenPage(page) => {
            let first = |p: usize, space: Option<&str>| Some((p, nudge::inbox_path(space)));
            match page {
                Page::App => (!snap.places.is_empty()).then(|| (0, "/".to_string())),
                Page::Inbox | Page::Activity => {
                    let needs = snap.needs();
                    match needs.first() {
                        Some((p, n)) => first(*p, Some(&n.space_id)),
                        None => (!snap.places.is_empty()).then(|| (0, nudge::inbox_path(None))),
                    }
                }
                // there is no Live now page yet: the 4th bot at work's chat (the first not
                // listed), else the Inbox
                Page::Live => match snap.working().get(3) {
                    Some((p, w)) => Some((*p, w.path.clone())),
                    None => (!snap.places.is_empty()).then(|| (0, nudge::inbox_path(None))),
                },
            }
        }
        _ => None,
    }
}

/// A place-switch ask: the place's key, only for a listed place.
pub fn place_target<'a>(snap: &'a Snapshot, ask: &PerchAsk) -> Option<&'a str> {
    match ask {
        PerchAsk::Place { i } => snap.places.get(*i).map(|p| p.key.as_str()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Peek and celebrate

/// Notices what is new between polls: a new need peeks; a newly seen, unseen finish with a
/// verified check celebrates. A place seen for the first time (app start, a server added) is
/// taken as it is, with no events.
#[derive(Debug, Default)]
pub struct Tracker {
    seen: HashMap<String, (HashSet<String>, HashSet<String>)>,
    peek: Option<(String, String, Instant)>,
    celebrate: Option<(String, String, Instant)>,
}

/// The events the page gets with an update: (place index, id).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Events {
    pub peek: Option<(usize, String)>,
    pub celebrate: Option<(usize, String)>,
}

impl Tracker {
    /// Takes in a snapshot. `failed`: places whose poll failed this time (their old content
    /// stays in the snapshot; nothing about them is new).
    pub fn update(&mut self, snap: &Snapshot, now: Instant) {
        let keys: HashSet<&str> = snap.places.iter().map(|p| p.key.as_str()).collect();
        self.seen.retain(|k, _| keys.contains(k.as_str()));
        let mut new_need: Option<(String, String, String)> = None; // (since, place, id)
        for place in &snap.places {
            let needs: HashSet<String> = place.glance.needs.iter().map(|n| n.id.clone()).collect();
            let done: HashSet<String> = place.glance.finished.iter().map(|f| f.id.clone()).collect();
            if let Some((old_needs, old_done)) = self.seen.get(&place.key) {
                for n in &place.glance.needs {
                    if !old_needs.contains(&n.id) && new_need.as_ref().is_none_or(|(since, _, _)| n.since > *since) {
                        new_need = Some((n.since.clone(), place.key.clone(), n.id.clone()));
                    }
                }
                for f in &place.glance.finished {
                    if !old_done.contains(&f.id) && f.unseen && f.check == Some(Check::Verified) {
                        self.celebrate = Some((place.key.clone(), f.id.clone(), now));
                    }
                }
            }
            self.seen.insert(place.key.clone(), (needs, done));
        }
        if let Some((_, place, id)) = new_need {
            self.peek = Some((place, id, now));
        }
    }

    /// The events still worth showing (at most `EVENT_CARRY` old, and still in `snap`).
    pub fn events(&self, snap: &Snapshot, now: Instant) -> Events {
        let index = |key: &str| snap.places.iter().position(|p| p.key == key);
        let fresh = |t: &Instant| now.saturating_duration_since(*t) < EVENT_CARRY;
        let peek = self.peek.as_ref().filter(|(_, _, t)| fresh(t)).and_then(|(k, id, _)| {
            let p = index(k)?;
            snap.need(p, id).map(|_| (p, id.clone()))
        });
        let celebrate = self.celebrate.as_ref().filter(|(_, _, t)| fresh(t)).and_then(|(k, id, _)| {
            let p = index(k)?;
            snap.places[p].glance.finished.iter().any(|f| &f.id == id).then(|| (p, id.clone()))
        });
        Events { peek, celebrate }
    }
}

// ---------------------------------------------------------------------------------------------
// What the page gets

/// How the perch hangs on this screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// Around the notch: notch width and height (pt).
    Notch { w: f64, h: f64 },
    /// A floating tab at the top of the work area.
    Float,
}

/// Everything about the page that isn't the places' content.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub shape: Shape,
    pub mac: bool,
    pub prefs: Prefs,
    /// The shortcut opened it (it stays until Esc, the shortcut again or a click outside).
    pub open_by_key: bool,
    /// Toggle count from the shortcut: each press flips the page between open and closed.
    pub toggles: u64,
}

fn face(name: &str, avatar: &Option<String>) -> Value {
    json!({ "name": name, "avatar": avatar })
}

/// The update for `kwantPerch.update(…)`. Only cleaned values go in, and the page names each
/// thing back by `p` (place index) and `id`.
pub fn page_json(snap: &Snapshot, view: &View, events: &Events) -> Value {
    let multi_place = snap.places.len() > 1;
    let spaces: HashSet<&str> = snap.needs().iter().map(|(_, n)| n.space_id.as_str()).collect();
    let place_label = |p: usize| if multi_place { snap.places[p].label.clone() } else { String::new() };
    let needs: Vec<Value> = snap
        .needs()
        .into_iter()
        .take(MAX_NEEDS)
        .map(|(p, n)| {
            let q = n.question.as_ref();
            json!({
                "p": p, "id": n.id, "kind": n.kind.as_str(),
                "who": face(&n.kwant_name, &n.avatar),
                "title": n.title, "space": n.space_name, "place": place_label(p), "since": n.since,
                "inline": n.kind == NeedKind::Approval && n.inline,
                "why": n.why_open,
                "choices": q.map(|q| q.choices.iter().take(CHIPS.min(q.total)).cloned().collect::<Vec<_>>()).unwrap_or_default(),
                "reply": q.is_some_and(|q| q.total > CHIPS || q.allow_text),
            })
        })
        .collect();
    let working: Vec<Value> = snap
        .working()
        .into_iter()
        .map(|(p, w)| json!({ "p": p, "id": w.id, "who": face(&w.kwant_name, &w.avatar), "step": w.step, "since": w.since }))
        .collect();
    let finished: Vec<Value> = snap
        .finished()
        .into_iter()
        .map(|(p, f)| {
            let check = match f.check {
                Some(Check::Verified) => "verified",
                Some(Check::Failed) => "failed",
                _ => "not_checked",
            };
            json!({ "p": p, "id": f.id, "who": face(&f.kwant_name, &f.avatar), "title": f.title, "check": check, "unseen": f.unseen })
        })
        .collect();
    let ask: Vec<Value> = snap
        .ask_targets()
        .into_iter()
        .enumerate()
        .map(|(t, (p, a))| json!({ "t": t, "p": p, "who": face(&a.name, &a.avatar), "place": place_label(p) }))
        .collect();
    let places: Vec<Value> = if multi_place {
        snap.places
            .iter()
            .enumerate()
            .map(|(i, p)| {
                json!({ "i": i, "kind": if p.this_mac { "mac" } else { "server" }, "name": p.label,
                        "current": p.current, "waiting": p.glance.needs_count, "off": p.off })
            })
            .collect()
    } else {
        Vec::new()
    };
    let (shape, notch) = match view.shape {
        Shape::Notch { w, h } => ("notch", json!({ "w": w.round(), "h": h.round() })),
        Shape::Float => ("float", Value::Null),
    };
    json!({
        "shape": shape, "notch": notch, "mac": view.mac,
        "mode": view.prefs.mode.as_str(),
        "key": key_label(view.prefs.active_shortcut(true), view.mac),
        "byKey": view.open_by_key, "toggles": view.toggles,
        "counts": { "needs": snap.needs_count(), "working": snap.working_count() },
        "needs": needs, "working": working, "finished": finished, "ask": ask, "places": places,
        "multiPlace": multi_place, "multiSpace": spaces.len() > 1,
        "peek": events.peek.as_ref().map(|(p, id)| json!({ "p": p, "id": id })),
        "celebrate": events.celebrate.as_ref().map(|(p, id)| json!({ "p": p, "id": id })),
    })
}

/// Whether the window should be on screen, before the page has said anything: hidden in Busy
/// mode when nothing is going on, always hidden when the perch is off.
pub fn wants_window(prefs: &Prefs, available: bool, snap: &Snapshot, open_by_key: bool) -> bool {
    if !prefs.on(available) {
        return false;
    }
    open_by_key || prefs.mode == PerchMode::Always || !snap.quiet()
}

// ---------------------------------------------------------------------------------------------
// Where it hangs

/// A screen's notch as AppKit reports it (points; `None` when the screen has no notch).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NotchMetrics {
    /// The screen's width.
    pub screen_w: f64,
    /// `safeAreaInsets.top`.
    pub inset_top: f64,
    /// `auxiliaryTopLeftArea.width` (the menu bar left of the notch).
    pub left_w: f64,
    /// `auxiliaryTopRightArea.width`.
    pub right_w: f64,
}

/// The notch as an offset from the screen's left edge, its width and its height, only when the
/// numbers make sense: finite, a 16–64 pt inset, an 80–400 pt notch, and areas that fit.
pub fn notch(m: &NotchMetrics) -> Option<(f64, f64, f64)> {
    let all = [m.screen_w, m.inset_top, m.left_w, m.right_w];
    if all.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return None;
    }
    if !(16.0..=64.0).contains(&m.inset_top) || m.left_w + m.right_w >= m.screen_w {
        return None;
    }
    let left = m.left_w;
    let w = m.screen_w - m.right_w - m.left_w;
    if !(80.0..=400.0).contains(&w) {
        return None;
    }
    Some((left, w, m.inset_top))
}

/// Ears either side of the notch at rest (design.md §2.1: two 60 pt ears).
pub const EAR: f64 = 60.0;
/// The floating tab's height.
pub const FLOAT_H: f64 = 30.0;
/// The open width, and the shadow margin left, right and below in peek and expanded.
pub const OPEN_W: f64 = 460.0;
pub const MARGIN: f64 = 24.0;
/// The open height the page may ask for.
pub const MIN_H: f64 = 30.0;
pub const MAX_H: f64 = 720.0;
/// The floating tab's widest rest (3 faces, a count, padding).
pub const MAX_REST_W: f64 = 200.0;

/// The perch window's frame in points, relative to the screen's top-left (`screen_w`: the
/// screen's width; `top`: where it hangs, 0 on a notched screen or the work area's top).
/// `content`: the size the page reported for its state.
pub fn frame(shape: Shape, screen_w: f64, notch_left: f64, top: f64, state: PageState, content: (f64, f64)) -> (f64, f64, f64, f64) {
    let ok = |v: f64, lo: f64, hi: f64, d: f64| if v.is_finite() { v.clamp(lo, hi) } else { d };
    let (cw, ch) = content;
    let (centre, w, h) = match (shape, state.open()) {
        (Shape::Notch { w, h }, false) => (notch_left + w / 2.0, w + 2.0 * EAR, h),
        (Shape::Float, false) => (screen_w / 2.0, ok(cw, 40.0, MAX_REST_W, 70.0), FLOAT_H),
        (Shape::Notch { w, h: nh }, true) => (notch_left + w / 2.0, OPEN_W, ok(ch, nh, MAX_H, 120.0)),
        (Shape::Float, true) => (screen_w / 2.0, OPEN_W, ok(ch, MIN_H, MAX_H, 108.0)),
    };
    let margin = if state.open() { MARGIN } else { 0.0 };
    let (fw, fh) = (w + 2.0 * margin, h + margin);
    let x = (centre - fw / 2.0).clamp(0.0, (screen_w - fw).max(0.0));
    (x.round(), top.round(), fw.round(), fh.round())
}

// ---------------------------------------------------------------------------------------------
// One place's glance between polls

/// What a place's glance poll keeps: the last good glance, its ETag, and whether the place
/// answered this time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlanceCache {
    pub etag: Option<String>,
    pub glance: Glance,
    /// It didn't answer (or is signed out, or answered with something unreadable). The last
    /// glance stays, so the perch doesn't flicker over one missed poll.
    pub off: bool,
    /// Polls in a row that came back 304, and rounds skipped since the last poll (the backoff).
    pub unchanged: u32,
    pub skipped: u32,
}

/// The longest backoff: a place that keeps answering 304 is asked every 3rd round (15 s).
pub const BACKOFF_MAX: u32 = 3;
/// 304s in a row before each step of the backoff.
pub const BACKOFF_AFTER: u32 = 3;

impl GlanceCache {
    /// Whether to ask this round. Every round while anything changes; after 3 unchanged answers
    /// every 2nd round, after 6 every 3rd. `eager` (the perch is open, or the person just acted
    /// on it) and a place with no ETag or no good answer are always asked.
    pub fn due(&mut self, eager: bool) -> bool {
        let every = if eager || self.etag.is_none() || self.off {
            1
        } else {
            (1 + self.unchanged / BACKOFF_AFTER).min(BACKOFF_MAX)
        };
        if self.skipped + 1 >= every {
            self.skipped = 0;
            true
        } else {
            self.skipped += 1;
            false
        }
    }

    /// Takes in one answer. `away`: what `/api/away` said this round, used when the place is a
    /// kwant from before the glance (404).
    pub fn take(&mut self, answer: crate::remote::Glance, away: Option<&nudge::Waiting>) {
        use crate::remote::Glance as A;
        match answer {
            A::Fresh { etag, body } => match parse_glance(&body) {
                Some(g) => {
                    self.glance = g;
                    self.etag = etag;
                    self.off = false;
                    self.unchanged = 0;
                }
                None => {
                    self.etag = None;
                    self.off = true;
                }
            },
            A::NotModified if self.etag.is_some() => {
                self.off = false;
                self.unchanged = self.unchanged.saturating_add(1);
            }
            // a 304 to no ETag of ours: odd, so nothing is trusted
            A::NotModified => self.off = true,
            A::Missing => {
                self.etag = None;
                match away {
                    Some(w) => {
                        self.glance = from_waiting(w);
                        self.off = false;
                    }
                    None => self.off = true,
                }
            }
            A::SignedOut | A::Failed => self.off = true,
        }
    }
}

/// The chat a Quick ask went to (`{"path": "/s/…/b/…"}`), if the answer names a usable one.
pub fn asked_path(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    path_ok(v["path"].as_str()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_glance_poll_keeps_the_last_good_glance() {
        use crate::remote::Glance as A;
        let mut c = GlanceCache::default();
        c.take(A::Fresh { etag: Some("W/\"g-1\"".into()), body: glance_body() }, None);
        assert!(!c.off);
        assert_eq!(c.etag.as_deref(), Some("W/\"g-1\""));
        assert_eq!(c.glance.needs.len(), 3);
        let good = c.glance.clone();
        c.take(A::NotModified, None);
        assert!(!c.off);
        assert_eq!(c.glance, good);
        // a missed poll: off, the glance stays
        c.take(A::Failed, None);
        assert!(c.off);
        assert_eq!(c.glance, good);
        c.take(A::SignedOut, None);
        assert!(c.off);
        // something that isn't a glance: off, and its ETag is dropped
        c.take(A::Fresh { etag: Some("W/\"g-2\"".into()), body: "<html>".into() }, None);
        assert!(c.off);
        assert_eq!(c.etag, None);
        assert_eq!(c.glance, good);
        // a 304 to no ETag of ours isn't trusted
        c.take(A::NotModified, None);
        assert!(c.off);
    }

    #[test]
    fn an_unchanged_place_is_asked_less_often_until_it_changes() {
        use crate::remote::Glance as A;
        let mut c = GlanceCache::default();
        let fresh = || A::Fresh { etag: Some("\"g1\"".into()), body: glance_body() };
        // no ETag yet: every round
        assert!(c.due(false));
        c.take(fresh(), None);
        let mut asked = Vec::new();
        for _ in 0..14 {
            let due = c.due(false);
            asked.push(due);
            if due {
                c.take(A::NotModified, None);
            }
        }
        // 3 polls every round, then 3 every 2nd round, then every 3rd (never slower)
        let pattern: String = asked.iter().map(|d| if *d { 'x' } else { '.' }).collect();
        assert_eq!(pattern, "xxx.x.x.x..x..", "{pattern}");
        // open perch (or just acted): every round again
        assert!(c.due(true));
        assert!(c.due(true));
        // a change resets it
        c.take(fresh(), None);
        assert_eq!(c.unchanged, 0);
        assert!(c.due(false));
        assert!(c.due(false));
        // a failed place is asked every round
        c.unchanged = 50;
        c.take(A::Failed, None);
        assert!(c.due(false) && c.due(false));
    }

    #[test]
    fn an_older_kwant_falls_back_to_what_away_said() {
        use crate::remote::Glance as A;
        let mut c = GlanceCache { etag: Some("W/\"g-1\"".into()), ..Default::default() };
        c.take(A::Missing, None);
        assert!(c.off);
        assert_eq!(c.etag, None);
        let w = nudge::Waiting::default();
        c.take(A::Missing, Some(&w));
        assert!(!c.off);
        assert_eq!(c.glance, from_waiting(&w));
    }

    #[test]
    fn a_quick_ask_names_its_chat_only_when_the_path_is_ours() {
        assert_eq!(asked_path(r#"{"path":"/s/spc_default/b/k1","deduped":false}"#).as_deref(), Some("/s/spc_default/b/k1"));
        for body in [r#"{"path":"//evil.example/x"}"#, r#"{"path":"https://evil.example/"}"#, r#"{"path":7}"#, "nope", "{}"] {
            assert_eq!(asked_path(body), None, "{body}");
        }
    }

    fn glance_body() -> String {
        json!({
            "counts": { "needs_you": 3, "working": 2, "finished_unseen": 1 },
            "needs_you": [
                { "kind": "approval", "id": "a1", "space_id": "spc_default", "space_name": "My space", "kwant_id": "k1",
                  "kwant_name": "Ben", "avatar": "fox", "title": "Send the invoice", "since": "2026-11-14T21:35:27.000Z",
                  "path": "/s/spc_default/b/k1?card=a1", "approval": { "risk": "medium", "can_allow_inline": true, "why_open": null }, "question": null },
                { "kind": "approval", "id": "a2", "space_id": "spc_default", "kwant_name": "Otto", "avatar": null,
                  "title": "Pay $420", "since": "2026-11-14T21:30:00.000Z", "path": "/s/spc_default/b/k2?card=a2",
                  "approval": { "risk": "high", "can_allow_inline": false, "why_open": "This one involves a payment." } },
                { "kind": "question", "id": "q1", "space_id": "spc_books", "kwant_name": "Ivy", "title": "Which cover?",
                  "since": "2026-11-14T21:36:00.000Z", "path": "/s/spc_books/c/ch1?card=q1",
                  "question": { "choices": ["Blue", "Sunset", "Neither", "Four", "Five", "Six"], "choices_total": 8, "allow_text": false } }
            ],
            "working": [
                { "turn_id": "t1", "kwant_name": "Nora", "avatar": "architect", "space_id": "spc_default", "step": "Reading acme.com",
                  "step_started_at": "2026-11-14T21:30:00Z", "started_at": "2026-11-14T21:20:00Z", "path": "/s/spc_default/b/k3" },
                { "turn_id": "t2", "kwant_name": "Kai", "space_id": "spc_default", "step": "Writing", "path": "//evil.example/x" }
            ],
            "finished": [
                { "kind": "delegation", "id": "f1", "kwant_name": "Kai", "title": "Weekly report", "unseen": true,
                  "check": { "status": "verified", "line": "3 of 3" }, "path": "/s/spc_default/b/k4" }
            ],
            "ask_targets": [ { "kwant_id": "k1", "name": "Ben", "avatar": "fox", "space_id": "spc_default" } ]
        })
        .to_string()
    }

    fn snap_one() -> Snapshot {
        Snapshot {
            places: vec![PlaceView {
                key: "this_mac".into(),
                label: "This Mac".into(),
                this_mac: true,
                off: false,
                current: true,
                glance: parse_glance(&glance_body()).unwrap(),
            }],
        }
    }

    #[test]
    fn settings_default_to_busy_and_k_and_keep_other_keys() {
        let dir = std::env::temp_dir().join(format!("perch-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Prefs::load(&dir), Prefs::default());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(nudge::prefs_path(&dir), r#"{"show_nudges":false,"perch_mode":"weird"}"#).unwrap();
        assert_eq!(Prefs::load(&dir), Prefs::default(), "an odd value is the default");
        let p = Prefs { mode: PerchMode::Always, shortcut: PerchShortcut::Space };
        p.save(&dir).unwrap();
        assert_eq!(Prefs::load(&dir), p);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(nudge::prefs_path(&dir)).unwrap()).unwrap();
        assert_eq!(v["show_nudges"], json!(false), "the nudge setting stays");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn off_means_no_perch_and_no_shortcut() {
        let off = Prefs { mode: PerchMode::Off, shortcut: PerchShortcut::K };
        assert!(!off.on(true));
        assert_eq!(off.active_shortcut(true), PerchShortcut::Off);
        let busy = Prefs::default();
        assert!(busy.on(true));
        assert!(!busy.on(false), "Wayland");
        assert_eq!(busy.active_shortcut(false), PerchShortcut::Off);
        assert_eq!(busy.active_shortcut(true), PerchShortcut::K);
    }

    #[test]
    fn there_is_no_perch_on_wayland() {
        assert!(available("macos", None, None, None));
        assert!(available("windows", Some("wayland-0"), None, None));
        assert!(available("linux", None, Some("x11"), None));
        assert!(!available("linux", Some("wayland-0"), None, None));
        assert!(!available("linux", None, Some("wayland"), None));
        assert!(!available("linux", Some(""), Some("Wayland"), None));
        assert!(available("linux", Some("wayland-0"), Some("wayland"), Some("x11")), "XWayland");
        assert!(available("linux", Some(""), None, None), "an empty WAYLAND_DISPLAY is not Wayland");
    }

    #[test]
    fn shortcuts_have_one_accelerator_and_label_each() {
        assert_eq!(accelerator(PerchShortcut::K, true), Some("Alt+Super+K"));
        assert_eq!(accelerator(PerchShortcut::K, false), Some("Ctrl+Alt+K"));
        assert_eq!(accelerator(PerchShortcut::Space, true), Some("Ctrl+Alt+Space"));
        assert_eq!(accelerator(PerchShortcut::Off, true), None);
        assert_eq!(key_label(PerchShortcut::K, true), Some("⌥⌘K"));
        assert_eq!(key_label(PerchShortcut::Space, true), Some("⌃⌥Space"));
        assert_eq!(key_label(PerchShortcut::K, false), Some("Ctrl+Alt+K"));
        assert_eq!(key_label(PerchShortcut::Off, false), None);
    }

    #[test]
    fn a_glance_is_read_and_cleaned() {
        let g = parse_glance(&glance_body()).unwrap();
        assert_eq!((g.needs_count, g.working_count), (3, 2));
        assert_eq!(g.needs.len(), 3);
        assert!(g.needs[0].inline);
        assert!(!g.needs[1].inline);
        assert_eq!(g.needs[1].why_open, "This one involves a payment.");
        assert_eq!(g.needs[1].avatar, None);
        let q = g.needs[2].question.as_ref().unwrap();
        assert_eq!((q.choices.len(), q.total, q.allow_text), (6, 8, false));
        assert_eq!(g.working[0].since, "2026-11-14T21:30:00Z", "the step's start");
        assert_eq!(g.working[1].path, "/s/spc_default/inbox", "a path to another origin is dropped");
        assert_eq!(g.finished[0].check, Some(Check::Verified));
        assert_eq!(g.ask[0].name, "Ben");
        assert_eq!(parse_glance("not json"), None);
        assert_eq!(parse_glance(r#"{"error":"x"}"#), None);
        assert_eq!(parse_glance(r#"[1,2]"#), None);
    }

    #[test]
    fn hostile_glance_values_are_dropped_or_cut() {
        let body = json!({
            "counts": { "needs_you": -5, "working": "lots" },
            "needs_you": [
                { "kind": "approval", "id": "../../etc", "title": "x" },
                { "kind": "approval", "id": "ok1", "title": "a\u{202E}gnp.exe\u{0007}\nnext", "kwant_name": "<img src=x onerror=alert(1)>",
                  "avatar": "<svg onload=alert(1)>", "path": "javascript:alert(1)", "since": "<b>",
                  "approval": { "can_allow_inline": "true" } },
                { "kind": "always_allow", "id": "x1" },
                { "kind": "question", "id": "q2", "title": "t".repeat(500), "question": { "choices": [1, "ok", "c".repeat(200)], "choices_total": 1 } }
            ],
            "working": [ { "turn_id": "t 1" } ],
            "ask_targets": [ { "kwant_id": "k1", "name": "", "space_id": "s" }, { "kwant_id": "k2", "name": "Nora", "space_id": "s p" } ]
        })
        .to_string();
        let g = parse_glance(&body).unwrap();
        assert_eq!(g.needs.len(), 2, "a bad id or an unknown kind is skipped");
        let n = &g.needs[0];
        assert_eq!(n.title, "agnp.exe next", "no control or direction override characters");
        assert_eq!(n.kwant_name, "<img src=x onerror=alert(1)>", "kept as text: the page uses textContent");
        assert_eq!(n.avatar, None);
        assert_eq!(n.path, "/s/spc_default/inbox");
        assert_eq!(n.since, "");
        assert!(!n.inline, "only a real true allows inline");
        let q = &g.needs[1];
        assert_eq!(q.title.chars().count(), 120);
        assert!(q.title.ends_with('…'));
        let qq = q.question.as_ref().unwrap();
        assert_eq!(qq.choices.len(), 2, "a non-string choice is skipped");
        assert_eq!(qq.choices[1].chars().count(), 60);
        assert_eq!(qq.total, 2, "the total is never below what is listed");
        assert!(g.working.is_empty());
        assert!(g.ask.is_empty());
        assert_eq!((g.needs_count, g.working_count), (2, 0));
    }

    #[test]
    fn lists_are_capped() {
        let needs: Vec<Value> = (0..50).map(|i| json!({ "kind": "watch", "id": format!("w{i}"), "title": "x" })).collect();
        let body = json!({ "counts": { "needs_you": 50 }, "needs_you": needs }).to_string();
        let g = parse_glance(&body).unwrap();
        assert_eq!(g.needs.len(), MAX_NEEDS);
        assert_eq!(g.needs_count, 50, "the count still says how many");
    }

    #[test]
    fn an_older_server_s_items_can_only_be_opened() {
        let item = nudge::Item {
            kind: nudge::Kind::Approval,
            id: "a9".into(),
            space_id: Some("spc_default".into()),
            kwant_id: "k1".into(),
            kwant_name: "Ben".into(),
            title: "Use Bash".into(),
            thread_id: None,
            channel_id: None,
            direct_kwant_id: None,
            place: "this_mac".into(),
            place_label: String::new(),
        };
        let g = from_waiting(&nudge::Waiting { items: vec![item], count: 4 });
        assert_eq!(g.needs_count, 4);
        assert!(!g.needs[0].inline);
        assert_eq!(g.needs[0].path, "/s/spc_default/b/k1?card=a9");
        let snap = Snapshot {
            places: vec![PlaceView { key: "this_mac".into(), label: "This Mac".into(), this_mac: true, off: false, current: true, glance: g }],
        };
        assert_eq!(call_for(&snap, &PerchAsk::Allow { p: 0, id: "a9".into() }), Err(Refused::NotInline));
        assert_eq!(open_target(&snap, &PerchAsk::Open { p: 0, id: "a9".into() }), Some((0, "/s/spc_default/b/k1?card=a9".into())));
    }

    #[test]
    fn the_page_s_asks_are_parsed_strictly() {
        assert_eq!(parse_perch("kwant-island://ready"), Some(PerchAsk::Ready));
        assert_eq!(parse_perch("kwant-island://allow?p=0&id=a1"), Some(PerchAsk::Allow { p: 0, id: "a1".into() }));
        assert_eq!(parse_perch("kwant-island://deny?p=1&id=a1"), Some(PerchAsk::Deny { p: 1, id: "a1".into() }));
        assert_eq!(parse_perch("kwant-island://answer?p=0&id=q1&c=2"), Some(PerchAsk::Answer { p: 0, id: "q1".into(), choice: 2 }));
        assert_eq!(parse_perch("kwant-island://open?p=0&id=t1"), Some(PerchAsk::Open { p: 0, id: "t1".into() }));
        assert_eq!(parse_perch("kwant-island://open?what=inbox"), Some(PerchAsk::OpenPage(Page::Inbox)));
        assert_eq!(parse_perch("kwant-island://size?s=peek&w=460&h=119"), Some(PerchAsk::Size { state: PageState::Peek, w: 460, h: 119 }));
        assert_eq!(parse_perch("kwant-island://place?i=1"), Some(PerchAsk::Place { i: 1 }));
        assert_eq!(
            parse_perch("kwant-island://ask?t=0&text=Hi%20there%2C%20%F0%9F%91%8B&cid=island-0f8c2a1e-7d"),
            Some(PerchAsk::Ask { t: 0, text: "Hi there, 👋".into(), cid: "island-0f8c2a1e-7d".into() })
        );
        for url in [
            "kwant-island://allow?p=0",
            "kwant-island://allow?id=a1",
            "kwant-island://allow?p=-1&id=a1",
            "kwant-island://allow?p=0&id=a1%27",
            "kwant-island://allow?p=0&id=https://evil",
            "kwant-island://always_allow?p=0&id=a1",
            "kwant-island://answer?p=0&id=q1&c=10",
            "kwant-island://answer?p=0&id=q1",
            "kwant-island://open?what=https://evil.example",
            "kwant-island://open?path=/s/x",
            "kwant-island://size?s=peek&w=460&h=1e3",
            "kwant-island://size?s=huge&w=1&h=1",
            "kwant-island://ask?t=0&text=&cid=island-12345678",
            "kwant-island://ask?t=0&text=%20%20&cid=island-12345678",
            "kwant-island://ask?t=0&text=hi&cid=evil-12345678",
            "kwant-island://ask?t=0&text=hi&cid=island-1",
            "kwant-island://ask?t=0&text=%FF&cid=island-12345678",
            "kwant-island://ask?t=0&text=%2&cid=island-12345678",
            "kwant-island://ask?t=10&text=hi&cid=island-12345678",
            "kwant-places://ready",
            "kwant-app://island?mode=busy&shortcut=k",
        ] {
            assert_eq!(parse_perch(url), None, "{url}");
        }
        let long = format!("kwant-island://ask?t=0&text={}&cid=island-12345678", "a".repeat(ASK_MAX + 1));
        assert_eq!(parse_perch(&long), None, "over 8,000 characters");
        let max = format!("kwant-island://ask?t=0&text={}&cid=island-12345678", "%C3%A9".repeat(ASK_MAX));
        assert!(matches!(parse_perch(&max), Some(PerchAsk::Ask { .. })), "8,000 two-byte characters fit");
    }

    #[test]
    fn allow_once_only_for_an_inline_approval_in_the_snapshot() {
        let snap = snap_one();
        let call = call_for(&snap, &PerchAsk::Allow { p: 0, id: "a1".into() }).unwrap();
        assert_eq!(call.path, "/api/approvals/a1/resolve");
        assert_eq!(call.body, json!({ "decision": "allow_once" }));
        let deny = call_for(&snap, &PerchAsk::Deny { p: 0, id: "a1".into() }).unwrap();
        assert_eq!(deny.body, json!({ "decision": "deny" }));
        // money / risky: the full card only, both ways
        assert_eq!(call_for(&snap, &PerchAsk::Allow { p: 0, id: "a2".into() }), Err(Refused::NotInline));
        assert_eq!(call_for(&snap, &PerchAsk::Deny { p: 0, id: "a2".into() }), Err(Refused::NotInline));
        // not in the snapshot, another place, a question
        assert_eq!(call_for(&snap, &PerchAsk::Allow { p: 0, id: "zzz".into() }), Err(Refused::NotThere));
        assert_eq!(call_for(&snap, &PerchAsk::Allow { p: 1, id: "a1".into() }), Err(Refused::NotThere));
        assert_eq!(call_for(&snap, &PerchAsk::Allow { p: 0, id: "q1".into() }), Err(Refused::NotInline));
        // nothing the page sends can become always_allow
        for ask in [PerchAsk::Allow { p: 0, id: "a1".into() }, PerchAsk::Deny { p: 0, id: "a1".into() }] {
            assert!(!call_for(&snap, &ask).unwrap().body.to_string().contains("always"));
        }
    }

    #[test]
    fn a_chip_answers_only_one_of_the_first_three_choices() {
        let snap = snap_one();
        let call = call_for(&snap, &PerchAsk::Answer { p: 0, id: "q1".into(), choice: 2 }).unwrap();
        assert_eq!(call.path, "/api/questions/q1/answer");
        assert_eq!(call.body, json!({ "choice": 2 }));
        assert_eq!(call_for(&snap, &PerchAsk::Answer { p: 0, id: "q1".into(), choice: 3 }), Err(Refused::NotInline));
        assert_eq!(call_for(&snap, &PerchAsk::Answer { p: 0, id: "a1".into(), choice: 0 }), Err(Refused::NotInline));
        assert_eq!(call_for(&snap, &PerchAsk::Answer { p: 0, id: "q9".into(), choice: 0 }), Err(Refused::NotThere));
    }

    #[test]
    fn a_quick_ask_goes_only_to_a_listed_bot() {
        let snap = snap_one();
        let ask = PerchAsk::Ask { t: 0, text: "Hi".into(), cid: "island-12345678".into() };
        let call = call_for(&snap, &ask).unwrap();
        assert_eq!(call.path, "/api/glance/ask");
        assert_eq!(call.body, json!({ "space_id": "spc_default", "kwant_id": "k1", "text": "Hi", "client_message_id": "island-12345678" }));
        assert_eq!(call_for(&snap, &PerchAsk::Ask { t: 1, text: "Hi".into(), cid: "island-12345678".into() }), Err(Refused::NotThere));
        assert_eq!(call_for(&Snapshot::default(), &ask), Err(Refused::NotThere));
    }

    #[test]
    fn open_uses_the_snapshot_s_path_only() {
        let snap = snap_one();
        assert_eq!(open_target(&snap, &PerchAsk::Open { p: 0, id: "q1".into() }), Some((0, "/s/spc_books/c/ch1?card=q1".into())));
        assert_eq!(open_target(&snap, &PerchAsk::Open { p: 0, id: "t1".into() }), Some((0, "/s/spc_default/b/k3".into())));
        assert_eq!(open_target(&snap, &PerchAsk::Open { p: 0, id: "f1".into() }), Some((0, "/s/spc_default/b/k4".into())));
        assert_eq!(open_target(&snap, &PerchAsk::Open { p: 0, id: "nope".into() }), None);
        assert_eq!(open_target(&snap, &PerchAsk::Open { p: 3, id: "q1".into() }), None);
        assert_eq!(open_target(&snap, &PerchAsk::OpenPage(Page::App)), Some((0, "/".into())));
        // Inbox opens the newest need's space
        assert_eq!(open_target(&snap, &PerchAsk::OpenPage(Page::Inbox)), Some((0, "/s/spc_books/inbox".into())));
        assert_eq!(open_target(&Snapshot::default(), &PerchAsk::OpenPage(Page::App)), None);
        assert_eq!(place_target(&snap, &PerchAsk::Place { i: 0 }), Some("this_mac"));
        assert_eq!(place_target(&snap, &PerchAsk::Place { i: 1 }), None);
    }

    #[test]
    fn a_new_need_peeks_and_a_verified_finish_celebrates_once_primed() {
        let t0 = Instant::now();
        let mut tr = Tracker::default();
        let mut snap = snap_one();
        tr.update(&snap, t0);
        assert_eq!(tr.events(&snap, t0), Events::default(), "what was there at start is not news");
        // a new need and a new verified finish
        let g = &mut snap.places[0].glance;
        let mut n = g.needs[0].clone();
        n.id = "a3".into();
        n.since = "2026-11-14T22:00:00Z".into();
        g.needs.push(n);
        let mut f = g.finished[0].clone();
        f.id = "f2".into();
        g.finished.push(f.clone());
        let mut f3 = f.clone();
        f3.id = "f3".into();
        f3.check = Some(Check::NotChecked);
        g.finished.push(f3);
        tr.update(&snap, t0 + Duration::from_secs(5));
        let ev = tr.events(&snap, t0 + Duration::from_secs(5));
        assert_eq!(ev.peek, Some((0, "a3".into())));
        assert_eq!(ev.celebrate, Some((0, "f2".into())), "a not-checked finish never celebrates");
        // carried for a while, then gone
        assert_eq!(tr.events(&snap, t0 + Duration::from_secs(5) + EVENT_CARRY), Events::default());
        // settled elsewhere: no peek for a thing that left
        tr.update(&snap, t0 + Duration::from_secs(6));
        snap.places[0].glance.needs.retain(|n| n.id != "a3");
        assert_eq!(tr.events(&snap, t0 + Duration::from_secs(6)).peek, None);
    }

    #[test]
    fn a_place_seen_first_brings_no_events() {
        let t0 = Instant::now();
        let mut tr = Tracker::default();
        let mut snap = snap_one();
        tr.update(&snap, t0);
        let mut other = snap.places[0].clone();
        other.key = "https://kwant.example.com".into();
        other.this_mac = false;
        snap.places.push(other);
        tr.update(&snap, t0 + Duration::from_secs(5));
        assert_eq!(tr.events(&snap, t0 + Duration::from_secs(5)), Events::default());
    }

    #[test]
    fn the_page_gets_places_only_with_two_or_more() {
        let view = View { shape: Shape::Float, mac: false, prefs: Prefs::default(), open_by_key: false, toggles: 0 };
        let snap = snap_one();
        let j = page_json(&snap, &view, &Events::default());
        assert_eq!(j["places"], json!([]));
        assert_eq!(j["needs"][0]["place"], json!(""), "no place named with one place");
        assert_eq!(j["needs"][0]["id"], json!("q1"), "newest first");
        assert_eq!(j["needs"][0]["choices"], json!(["Blue", "Sunset", "Neither"]));
        assert_eq!(j["needs"][0]["reply"], json!(true));
        assert_eq!(j["key"], json!("Ctrl+Alt+K"));
        assert_eq!(j["counts"], json!({ "needs": 3, "working": 2 }));
        assert_eq!(j["multiSpace"], json!(true));
        let mut two = snap.clone();
        let mut s = two.places[0].clone();
        s.key = "https://kwant.jumpin.games".into();
        s.label = "kwant.jumpin.games".into();
        s.this_mac = false;
        s.current = false;
        two.places.push(s);
        let j = page_json(&two, &view, &Events { peek: Some((1, "a1".into())), celebrate: None });
        assert_eq!(j["places"].as_array().unwrap().len(), 2);
        assert_eq!(j["places"][1]["kind"], json!("server"));
        assert!(j["needs"].as_array().unwrap().iter().any(|n| n["place"] == json!("kwant.jumpin.games")));
        assert_eq!(j["peek"], json!({ "p": 1, "id": "a1" }));
        assert_eq!(j["ask"].as_array().unwrap().len(), 2);
        // the shortcut label follows the settings; none while Off
        let off = View { prefs: Prefs { mode: PerchMode::Off, shortcut: PerchShortcut::K }, ..view };
        assert_eq!(page_json(&snap, &off, &Events::default())["key"], Value::Null);
    }

    #[test]
    fn busy_mode_hides_when_quiet_and_always_stays() {
        let quiet = Snapshot::default();
        let busy = Prefs::default();
        let always = Prefs { mode: PerchMode::Always, ..busy };
        let off = Prefs { mode: PerchMode::Off, ..busy };
        assert!(!wants_window(&busy, true, &quiet, false));
        assert!(wants_window(&busy, true, &quiet, true), "the shortcut opens it anyway");
        assert!(wants_window(&always, true, &quiet, false));
        assert!(wants_window(&busy, true, &snap_one(), false));
        assert!(!wants_window(&off, true, &snap_one(), true));
        assert!(!wants_window(&always, false, &snap_one(), true), "Wayland");
    }

    #[test]
    fn the_notch_is_read_only_when_it_makes_sense() {
        // MacBook Pro 14": 1512 pt wide, 185 pt notch, 37 pt tall
        let m = NotchMetrics { screen_w: 1512.0, inset_top: 37.0, left_w: 663.5, right_w: 663.5 };
        assert_eq!(notch(&m), Some((663.5, 185.0, 37.0)));
        for bad in [
            NotchMetrics { inset_top: 0.0, ..m },
            NotchMetrics { inset_top: 24.0, left_w: 0.0, right_w: 0.0, ..m },
            NotchMetrics { inset_top: f64::NAN, ..m },
            NotchMetrics { left_w: f64::INFINITY, ..m },
            NotchMetrics { left_w: -1.0, ..m },
            NotchMetrics { left_w: 800.0, right_w: 800.0, ..m },
            NotchMetrics { left_w: 300.0, right_w: 300.0, ..m },
            NotchMetrics { inset_top: 100.0, ..m },
        ] {
            assert_eq!(notch(&bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_frame_centres_on_the_notch_and_stays_on_screen() {
        let shape = Shape::Notch { w: 185.0, h: 37.0 };
        // rest: notch + two 60 pt ears, no margin, centred on the notch's centre
        assert_eq!(frame(shape, 1512.0, 663.5, 0.0, PageState::Rest, (0.0, 0.0)), (604.0, 0.0, 305.0, 37.0));
        // an off-centre notch is followed
        assert_eq!(frame(shape, 1512.0, 700.0, 0.0, PageState::Rest, (0.0, 0.0)).0, (700.0_f64 + 92.5 - 152.5).round());
        // peek: 460 wide + 24 px margins, height from the page, clamped
        assert_eq!(frame(shape, 1512.0, 663.5, 0.0, PageState::Peek, (460.0, 119.0)), (502.0, 0.0, 508.0, 143.0));
        assert_eq!(frame(shape, 1512.0, 663.5, 0.0, PageState::Expanded, (460.0, 9999.0)).3, MAX_H + MARGIN);
        assert_eq!(frame(shape, 1512.0, 663.5, 0.0, PageState::Expanded, (460.0, 1.0)).3, 37.0 + MARGIN, "never shorter than the notch");
        // floating: centred on the screen, under the menu bar / panel
        assert_eq!(frame(Shape::Float, 1440.0, 0.0, 24.0, PageState::Rest, (106.0, 30.0)), (667.0, 24.0, 106.0, 30.0));
        assert_eq!(frame(Shape::Float, 1440.0, 0.0, 24.0, PageState::Rest, (5000.0, 30.0)).2, MAX_REST_W);
        assert_eq!(frame(Shape::Float, 1440.0, 0.0, 24.0, PageState::Peek, (460.0, 108.0)), (466.0, 24.0, 508.0, 132.0));
        // a tiny screen: pulled back on screen; a NaN size never makes a NaN frame
        let f = frame(Shape::Float, 300.0, 0.0, 0.0, PageState::Expanded, (f64::NAN, f64::NAN));
        assert_eq!(f.0, 0.0);
        assert!(f.3.is_finite());
    }
}
