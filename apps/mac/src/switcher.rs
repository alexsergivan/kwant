//! Switching places from inside the window (desktop-island/build-brief.md, Part 1): what the
//! place chip and the picker may ask, and the rules Rust holds them to. Pure: no Tauri, no I/O,
//! so every rule here has a unit test (`rustc --test src/switcher.rs` runs them without WebKit).
//!
//! - A place's page asks for the picker by navigating to `kwant-app://places?at=x,y` (or with no
//!   `at`: the window's top centre). Any place window may ask (it only opens our own bundled
//!   page), at most once a second per window, and x,y are clamped to the window.
//! - The picker (bundled `loading/places.html`) answers with `kwant-places://…` navigations that
//!   name a row by its index in the list Rust gave it, never by a key it made up.
//! - Only This Mac's window (`main`, showing the kwant on 127.0.0.1) learns how many things wait
//!   in other places (`window.__KWANT_ELSEWHERE__`). A server's page never learns anything about
//!   any other place.
//! - Perch settings (desktop-island/build-brief.md, Part 2): only This Mac's page learns them
//!   (`window.__KWANT_ISLAND__`) and only it may change them (`kwant-app://island?mode=…&shortcut=…`,
//!   whitelisted values, at most once a second). The same ask from any other window is ignored.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The label of This Mac's window (as `crate::MAIN`).
pub const MAIN_WINDOW: &str = "main";
/// The picker window's label.
pub const PICKER_WINDOW: &str = "places";
/// How often one window may open the picker.
pub const ASK_EVERY: Duration = Duration::from_secs(1);
/// Longest `kwant-app://` address read at all.
const MAX_URL: usize = 256;

/// The picker card's width (design.md 9.2) and the transparent margin around it for its shadow.
pub const PICKER_W: f64 = 300.0;
pub const PICKER_MARGIN: f64 = 16.0;
/// The heights the picker page may ask for (with its margins).
pub const PICKER_MIN_H: f64 = 120.0;
pub const PICKER_MAX_H: f64 = 560.0;
/// The gap between the chip's bottom edge and the picker card.
const UNDER_CHIP: f64 = 4.0;
/// From the window's top edge when opened by the shortcut.
const FROM_TOP: f64 = 8.0;

/// What a place's page asked for through `kwant-app://`.
#[derive(Debug, Clone, PartialEq)]
pub enum AppAsk {
    /// Open the picker; `at` is the chip's bottom-left in window (CSS) coordinates, unclamped.
    Places { at: Option<(f64, f64)> },
    /// Perch settings from This Mac's Settings page (both values are required).
    Island { mode: PerchMode, shortcut: PerchShortcut },
}

/// When the perch shows (design.md §8): "Only when something is happening", "Always", "Off".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerchMode {
    Busy,
    Always,
    Off,
}

impl PerchMode {
    pub fn as_str(self) -> &'static str {
        match self {
            PerchMode::Busy => "busy",
            PerchMode::Always => "always",
            PerchMode::Off => "off",
        }
    }
    /// Exactly one of the three words; anything else is `None`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "busy" => Some(PerchMode::Busy),
            "always" => Some(PerchMode::Always),
            "off" => Some(PerchMode::Off),
            _ => None,
        }
    }
}

/// The perch's global shortcut: ⌥⌘K / Ctrl+Alt+K, ⌃⌥Space / Ctrl+Alt+Space, or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerchShortcut {
    K,
    Space,
    Off,
}

impl PerchShortcut {
    pub fn as_str(self) -> &'static str {
        match self {
            PerchShortcut::K => "k",
            PerchShortcut::Space => "space",
            PerchShortcut::Off => "off",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "k" => Some(PerchShortcut::K),
            "space" => Some(PerchShortcut::Space),
            "off" => Some(PerchShortcut::Off),
            _ => None,
        }
    }
}

/// The rate-limit key for perch settings from a window (its own budget, apart from the picker's).
pub fn island_key(label: &str) -> String {
    format!("{label}#island")
}

/// Reads a `kwant-app://` address. `None`: not one we know (it is refused and ignored).
pub fn parse_app(url: &str) -> Option<AppAsk> {
    if url.len() > MAX_URL {
        return None;
    }
    let rest = url.strip_prefix("kwant-app://")?;
    let (host, query) = split_query(rest);
    match host.trim_end_matches('/') {
        "places" => {
            let at = query.and_then(|q| param(q, "at")).and_then(parse_at);
            Some(AppAsk::Places { at })
        }
        "island" => {
            let q = query?;
            let mode = PerchMode::parse(param(q, "mode")?)?;
            let shortcut = PerchShortcut::parse(param(q, "shortcut")?)?;
            Some(AppAsk::Island { mode, shortcut })
        }
        _ => None,
    }
}

/// What the picker page asked for through `kwant-places://`.
#[derive(Debug, Clone, PartialEq)]
pub enum PickerAsk {
    /// The page loaded and can take its rows.
    Ready,
    /// Switch to row `index` of the list Rust gave it; `side`: open side by side instead.
    Switch { index: usize, side: bool },
    /// "Add a server…".
    Add,
    /// "Run kwant on this Mac".
    RunHere,
    /// Esc.
    Close,
    /// The card's height (CSS px, without the margins).
    Size { h: f64 },
}

/// Reads a `kwant-places://` address. `None`: not one we know.
pub fn parse_picker(url: &str) -> Option<PickerAsk> {
    if url.len() > MAX_URL {
        return None;
    }
    let rest = url.strip_prefix("kwant-places://")?;
    let (host, query) = split_query(rest);
    let q = query.unwrap_or("");
    match host.trim_end_matches('/') {
        "ready" => Some(PickerAsk::Ready),
        "switch" => {
            let index = param(q, "i")?;
            // digits only: no sign, no spaces, no hex; small enough for any list
            if index.is_empty() || index.len() > 2 || !index.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let side = match param(q, "side") {
                None | Some("0") => false,
                Some("1") => true,
                Some(_) => return None,
            };
            Some(PickerAsk::Switch { index: index.parse().ok()?, side })
        }
        "add" => Some(PickerAsk::Add),
        "run-here" => Some(PickerAsk::RunHere),
        "close" => Some(PickerAsk::Close),
        "size" => {
            let h = param(q, "h")?;
            if h.is_empty() || h.len() > 4 || !h.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            Some(PickerAsk::Size { h: h.parse::<f64>().ok()? })
        }
        _ => None,
    }
}

fn split_query(rest: &str) -> (&str, Option<&str>) {
    let rest = rest.split('#').next().unwrap_or("");
    match rest.split_once('?') {
        Some((h, q)) => (h, Some(q)),
        None => (rest, None),
    }
}

/// The first value of `name` in a query (no percent-decoding: none of our values need it).
fn param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == name).then_some(v)
    })
}

/// "x,y" as two finite numbers (a comma may come percent-encoded as %2C). `None` for anything else.
pub fn parse_at(text: &str) -> Option<(f64, f64)> {
    if text.len() > 40 {
        return None;
    }
    let text = text.replace("%2C", ",").replace("%2c", ",");
    let (x, y) = text.split_once(',')?;
    let num = |s: &str| -> Option<f64> {
        // plain decimals only: no "inf", "NaN", exponents or signs other than a leading minus
        let ok = !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b'-');
        if !ok {
            return None;
        }
        s.parse::<f64>().ok().filter(|v| v.is_finite())
    };
    Some((num(x)?, num(y)?))
}

/// Keeps a point inside a window of `size` (CSS px).
pub fn clamp_at(at: (f64, f64), size: (f64, f64)) -> (f64, f64) {
    let (w, h) = (size.0.max(0.0), size.1.max(0.0));
    (at.0.clamp(0.0, w), at.1.clamp(0.0, h))
}

/// At most one ask per `every` per key (a window label). A refused ask doesn't restart the clock.
#[derive(Debug, Default)]
pub struct RateLimit {
    last: HashMap<String, Instant>,
}

impl RateLimit {
    pub fn allow(&mut self, key: &str, now: Instant, every: Duration) -> bool {
        match self.last.get(key) {
            Some(t) if now.saturating_duration_since(*t) < every => false,
            _ => {
                self.last.insert(key.to_string(), now);
                true
            }
        }
    }
}

/// A screen rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Where the picker window goes (physical px, top-left). `window`: the asking window's content
/// area; `at`: the chip's bottom-left in that window (CSS px, already clamped), or `None` for the
/// window's top centre; `scale`: the window's scale factor; `height`: the picker window's height
/// in CSS px with margins; `screen`: the work area of the window's monitor. The card's top-left
/// lands under the chip; the whole window stays on screen.
pub fn picker_spot(window: Rect, at: Option<(f64, f64)>, scale: f64, height: f64, screen: Rect) -> (f64, f64) {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let (card_x, card_y) = match at {
        Some((x, y)) => (window.x + x * scale, window.y + (y + UNDER_CHIP) * scale),
        None => (window.x + window.w / 2.0 - PICKER_W / 2.0 * scale, window.y + FROM_TOP * scale),
    };
    let (x, y) = (card_x - PICKER_MARGIN * scale, card_y - PICKER_MARGIN * scale);
    let (w, h) = ((PICKER_W + 2.0 * PICKER_MARGIN) * scale, height * scale);
    let x = x.min(screen.x + screen.w - w).max(screen.x);
    let y = y.min(screen.y + screen.h - h).max(screen.y);
    (x.round(), y.round())
}

/// The picker window's height for the card height the page reported (CSS px).
pub fn picker_height(card_h: f64) -> f64 {
    (card_h + 2.0 * PICKER_MARGIN).clamp(PICKER_MIN_H, PICKER_MAX_H)
}

/// The picker's row order (design.md 9.2): the current place first, then the others in saved
/// order. `keys`: the places in saved order; returns indexes into it.
pub fn row_order(keys: &[String], current: Option<&str>) -> Vec<usize> {
    let cur = current.and_then(|c| keys.iter().position(|k| k == c));
    cur.into_iter().chain((0..keys.len()).filter(|i| Some(*i) != cur)).collect()
}

/// The card's height before the page measures itself (CSS px, without margins), from
/// places.html's CSS: padding 6+6, header 30, rows 44, divider 13, actions 40, hint 25.
pub fn card_height(rows: usize, actions: usize) -> f64 {
    12.0 + 30.0 + 44.0 * rows as f64 + 13.0 + 40.0 * actions as f64 + 25.0
}

/// What a switch does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Show (and focus) the target; nothing else moves.
    Show,
    /// The target takes `from`'s frame and shows; `from` hides (and stays connected).
    TakeFrame { from: String },
    /// The target opens in its own window, as before (⌥↩).
    SideBySide,
}

/// How to get to `target` (a window label) from the window the person was in (`from`: its label
/// while it shows, else `None`). `target_shows`: the target's window is already on screen (a
/// side-by-side window stays where it is). `side`: ⌥↩.
pub fn plan(from: Option<&str>, target: &str, target_shows: bool, side: bool) -> Plan {
    if side {
        return Plan::SideBySide;
    }
    match from {
        Some(f) if f != target && !target_shows && f != PICKER_WINDOW => Plan::TakeFrame { from: f.to_string() },
        _ => Plan::Show,
    }
}

/// Whether `url` is the page of the kwant on this Mac (`port`): http on 127.0.0.1 or localhost.
pub fn is_local_page(url: &str, port: Option<u16>) -> bool {
    let Some(port) = port.filter(|p| *p != 0) else {
        return false;
    };
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return false;
    }
    authority == format!("127.0.0.1:{port}") || authority == format!("localhost:{port}")
}

/// The script that tells This Mac's page how many things wait in other places, or `None` when
/// the window isn't This Mac's or doesn't show This Mac's kwant (a server window, the picker, the
/// loading page): nothing about other places is ever given to them.
pub fn elsewhere_script(label: &str, page_is_local: bool, waiting_elsewhere: usize) -> Option<String> {
    if label != MAIN_WINDOW || !page_is_local {
        return None;
    }
    let n = waiting_elsewhere.min(9999);
    Some(format!(
        "window.__KWANT_ELSEWHERE__={n};try{{window.dispatchEvent(new Event('kwant-elsewhere'))}}catch(e){{}}"
    ))
}

/// Whether a window may read or change perch settings: This Mac's window while it shows This
/// Mac's own kwant. Server windows, the picker, the perch and the loading page may not.
pub fn island_allowed(label: &str, page_is_local: bool) -> bool {
    label == MAIN_WINDOW && page_is_local
}

/// The script that gives This Mac's Settings page the perch settings (`available`: false on
/// Wayland), or `None` for any other window or page.
pub fn island_script(label: &str, page_is_local: bool, mode: PerchMode, shortcut: PerchShortcut, available: bool) -> Option<String> {
    if !island_allowed(label, page_is_local) {
        return None;
    }
    Some(format!(
        "window.__KWANT_ISLAND__=Object.freeze({{mode:\"{}\",shortcut:\"{}\",available:{}}});try{{window.dispatchEvent(new Event('kwant-island'))}}catch(e){{}}",
        mode.as_str(),
        shortcut.as_str(),
        available
    ))
}

/// What waits in the places other than This Mac (`counts`: (is This Mac, count) per place).
pub fn waiting_elsewhere(counts: &[(bool, usize)]) -> usize {
    counts.iter().filter(|(this_mac, _)| !this_mac).map(|(_, n)| *n).sum()
}

/// Shortcut for Windows and Linux, where the app has no menu bar: Ctrl+Shift+O in a place's
/// window asks for the picker at the window's top centre. (macOS has the Window menu's
/// "Switch place…" ⌘⇧O instead.) Runs in the main frame of each place window.
pub const SHORTCUT_SCRIPT: &str = "window.addEventListener('keydown',function(e){if(e.ctrlKey&&e.shiftKey&&!e.altKey&&!e.metaKey&&(e.key==='O'||e.key==='o'||e.code==='KeyO')){e.preventDefault();e.stopPropagation();window.location.href='kwant-app://places';}},true);";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chip_s_ask_is_read_with_its_point() {
        assert_eq!(parse_app("kwant-app://places?at=76,44"), Some(AppAsk::Places { at: Some((76.0, 44.0)) }));
        assert_eq!(parse_app("kwant-app://places?at=76.5%2C44"), Some(AppAsk::Places { at: Some((76.5, 44.0)) }));
        assert_eq!(parse_app("kwant-app://places/?at=1,2"), Some(AppAsk::Places { at: Some((1.0, 2.0)) }));
        // the shortcut sends no point: the window's top centre
        assert_eq!(parse_app("kwant-app://places"), Some(AppAsk::Places { at: None }));
    }

    #[test]
    fn an_odd_point_is_dropped_not_trusted() {
        for at in ["NaN,1", "inf,2", "1e309,1", "1,", ",1", "a,b", "1;2", "1,2,3", "+1,2", " 1,2", "0x10,2"] {
            assert_eq!(parse_app(&format!("kwant-app://places?at={at}")), Some(AppAsk::Places { at: None }), "{at}");
        }
        let long = format!("kwant-app://places?at={},1", "9".repeat(300));
        assert_eq!(parse_app(&long), None, "an overlong address is ignored");
    }

    #[test]
    fn unknown_app_asks_are_ignored() {
        for url in ["kwant-app://", "kwant-app://placesx", "kwant-app://islandx?mode=off&shortcut=k", "kwant-nudge://open", "https://kwant-app/places"] {
            assert_eq!(parse_app(url), None, "{url}");
        }
    }

    #[test]
    fn perch_settings_take_only_whitelisted_values() {
        assert_eq!(
            parse_app("kwant-app://island?mode=busy&shortcut=k"),
            Some(AppAsk::Island { mode: PerchMode::Busy, shortcut: PerchShortcut::K })
        );
        assert_eq!(
            parse_app("kwant-app://island?shortcut=space&mode=always"),
            Some(AppAsk::Island { mode: PerchMode::Always, shortcut: PerchShortcut::Space })
        );
        assert_eq!(
            parse_app("kwant-app://island/?mode=off&shortcut=off"),
            Some(AppAsk::Island { mode: PerchMode::Off, shortcut: PerchShortcut::Off })
        );
        for url in [
            "kwant-app://island",
            "kwant-app://island?mode=off",
            "kwant-app://island?shortcut=k",
            "kwant-app://island?mode=Busy&shortcut=k",
            "kwant-app://island?mode=busy&shortcut=K",
            "kwant-app://island?mode=busy%20&shortcut=k",
            "kwant-app://island?mode=busy&shortcut=cmd+q",
            "kwant-app://island?mode=&shortcut=",
            "kwant-app://island?mode=always;alert(1)&shortcut=k",
            "kwant-app://island?mode=\"},x:{\"&shortcut=k",
        ] {
            assert_eq!(parse_app(url), None, "{url}");
        }
        for m in [PerchMode::Busy, PerchMode::Always, PerchMode::Off] {
            assert_eq!(PerchMode::parse(m.as_str()), Some(m));
        }
        for k in [PerchShortcut::K, PerchShortcut::Space, PerchShortcut::Off] {
            assert_eq!(PerchShortcut::parse(k.as_str()), Some(k));
        }
    }

    #[test]
    fn only_this_mac_s_page_reads_or_changes_perch_settings() {
        for label in ["server-68747470733a2f2f6b77616e742e6a756d70696e2e67616d6573", "server-", "connect", PICKER_WINDOW, "perch", "kwant-nudge-pill"] {
            assert!(!island_allowed(label, true), "{label}");
            assert_eq!(island_script(label, true, PerchMode::Busy, PerchShortcut::K, true), None, "{label}");
        }
        assert!(!island_allowed(MAIN_WINDOW, false), "main on the loading page or a remote page");
        assert_eq!(island_script(MAIN_WINDOW, false, PerchMode::Busy, PerchShortcut::K, true), None);
        let js = island_script(MAIN_WINDOW, true, PerchMode::Always, PerchShortcut::Space, false).unwrap();
        assert!(js.starts_with("window.__KWANT_ISLAND__=Object.freeze({mode:\"always\",shortcut:\"space\",available:false});"), "{js}");
        assert!(js.contains("new Event('kwant-island')"));
        assert!(!js.contains("ELSEWHERE"));
    }

    #[test]
    fn perch_settings_have_their_own_once_a_second_budget() {
        let mut limit = RateLimit::default();
        let t0 = Instant::now();
        assert!(limit.allow(MAIN_WINDOW, t0, ASK_EVERY), "the picker");
        assert!(limit.allow(&island_key(MAIN_WINDOW), t0, ASK_EVERY), "settings don't share the picker's budget");
        let saved = (1..200).filter(|i| limit.allow(&island_key(MAIN_WINDOW), t0 + Duration::from_millis(*i * 4), ASK_EVERY)).count();
        assert_eq!(saved, 0, "200 asks within 0.8 s save nothing more");
        assert!(limit.allow(&island_key(MAIN_WINDOW), t0 + Duration::from_millis(1000), ASK_EVERY));
    }

    #[test]
    fn points_are_clamped_to_the_window() {
        assert_eq!(clamp_at((-50.0, -1.0), (1280.0, 820.0)), (0.0, 0.0));
        assert_eq!(clamp_at((99999.0, 99999.0), (1280.0, 820.0)), (1280.0, 820.0));
        assert_eq!(clamp_at((76.0, 44.0), (1280.0, 820.0)), (76.0, 44.0));
        assert_eq!(clamp_at((5.0, 5.0), (-3.0, f64::NAN.max(0.0))), (0.0, 0.0));
    }

    #[test]
    fn a_window_opens_the_picker_at_most_once_a_second() {
        let mut limit = RateLimit::default();
        let t0 = Instant::now();
        assert!(limit.allow("server-ab", t0, ASK_EVERY));
        assert!(!limit.allow("server-ab", t0 + Duration::from_millis(10), ASK_EVERY));
        assert!(!limit.allow("server-ab", t0 + Duration::from_millis(999), ASK_EVERY));
        // another window has its own allowance
        assert!(limit.allow("main", t0 + Duration::from_millis(10), ASK_EVERY));
        assert!(limit.allow("server-ab", t0 + Duration::from_millis(1000), ASK_EVERY));
        // 100 asks in a burst open it once
        let t1 = t0 + Duration::from_secs(10);
        let opened = (0..100).filter(|i| limit.allow("server-ab", t1 + Duration::from_millis(*i), ASK_EVERY)).count();
        assert_eq!(opened, 1);
    }

    #[test]
    fn the_picker_names_rows_by_index_only() {
        assert_eq!(parse_picker("kwant-places://switch?i=2"), Some(PickerAsk::Switch { index: 2, side: false }));
        assert_eq!(parse_picker("kwant-places://switch?i=0&side=1"), Some(PickerAsk::Switch { index: 0, side: true }));
        for url in [
            "kwant-places://switch",
            "kwant-places://switch?i=",
            "kwant-places://switch?i=-1",
            "kwant-places://switch?i=+1",
            "kwant-places://switch?i=999",
            "kwant-places://switch?i=1&side=yes",
            "kwant-places://switch?key=https://evil.example",
            "kwant-places://open?i=1",
        ] {
            assert_eq!(parse_picker(url), None, "{url}");
        }
        assert_eq!(parse_picker("kwant-places://ready"), Some(PickerAsk::Ready));
        assert_eq!(parse_picker("kwant-places://add"), Some(PickerAsk::Add));
        assert_eq!(parse_picker("kwant-places://run-here"), Some(PickerAsk::RunHere));
        assert_eq!(parse_picker("kwant-places://close"), Some(PickerAsk::Close));
        assert_eq!(parse_picker("kwant-places://size?h=244"), Some(PickerAsk::Size { h: 244.0 }));
        assert_eq!(parse_picker("kwant-places://size?h=1e9"), None);
        assert_eq!(parse_picker("kwant-places://size?h=99999"), None);
    }

    #[test]
    fn the_picker_height_is_clamped() {
        assert_eq!(picker_height(10.0), PICKER_MIN_H);
        assert_eq!(picker_height(9000.0), PICKER_MAX_H);
        assert_eq!(picker_height(244.0), 244.0 + 2.0 * PICKER_MARGIN);
    }

    #[test]
    fn the_picker_lands_under_the_chip_and_stays_on_screen() {
        let screen = Rect { x: 0.0, y: 0.0, w: 2880.0, h: 1800.0 };
        let window = Rect { x: 200.0, y: 100.0, w: 2560.0, h: 1640.0 };
        // chip bottom-left at 76,44 CSS px on a 2x screen: card at 200+152, 100+(48*2)
        let (x, y) = picker_spot(window, Some((76.0, 44.0)), 2.0, 300.0, screen);
        assert_eq!((x, y), (200.0 + 152.0 - 32.0, 100.0 + 96.0 - 32.0));
        // the shortcut: centred on the window, near its top
        let (x, _) = picker_spot(window, None, 2.0, 300.0, screen);
        assert_eq!(x, (200.0 + 1280.0 - 300.0 - 32.0_f64).round());
        // a window at the screen's right edge: the picker is pulled back on screen
        let edge = Rect { x: 2700.0, y: 1700.0, w: 900.0, h: 600.0 };
        let (x, y) = picker_spot(edge, Some((800.0, 500.0)), 1.0, 300.0, screen);
        assert_eq!((x, y), (2880.0 - 332.0, 1800.0 - 300.0));
        // a window off the left/top of a second screen
        let other = Rect { x: -1920.0, y: 0.0, w: 1920.0, h: 1080.0 };
        let left = Rect { x: -1950.0, y: -40.0, w: 800.0, h: 600.0 };
        let (x, y) = picker_spot(left, Some((0.0, 0.0)), 1.0, 300.0, other);
        assert_eq!((x, y), (-1920.0, 0.0));
        // a bad scale factor never yields NaN
        let (x, y) = picker_spot(window, Some((1.0, 1.0)), f64::NAN, 300.0, screen);
        assert!(x.is_finite() && y.is_finite());
    }

    #[test]
    fn the_current_place_comes_first_then_saved_order() {
        let keys: Vec<String> = ["this_mac", "https://a", "https://b"].iter().map(|s| s.to_string()).collect();
        assert_eq!(row_order(&keys, Some("https://b")), vec![2, 0, 1]);
        assert_eq!(row_order(&keys, Some("this_mac")), vec![0, 1, 2]);
        assert_eq!(row_order(&keys, None), vec![0, 1, 2]);
        assert_eq!(row_order(&keys, Some("https://gone")), vec![0, 1, 2]);
        assert_eq!(row_order(&[], Some("x")), Vec::<usize>::new());
        assert!(picker_height(card_height(3, 1)) < PICKER_MAX_H);
    }

    #[test]
    fn switching_takes_the_frame_unless_asked_otherwise() {
        let s1 = "server-6874";
        assert_eq!(plan(Some("main"), s1, false, false), Plan::TakeFrame { from: "main".into() });
        assert_eq!(plan(Some("main"), s1, false, true), Plan::SideBySide, "⌥↩ keeps today's side by side");
        assert_eq!(plan(Some("main"), "main", false, false), Plan::Show, "the current place just comes front");
        assert_eq!(plan(None, s1, false, false), Plan::Show, "no window showing: just show the target");
        assert_eq!(plan(Some("main"), s1, true, false), Plan::Show, "a window already beside it stays, nothing hides");
        assert_eq!(plan(Some(PICKER_WINDOW), s1, false, false), Plan::Show, "the picker is never a place to hide");
    }

    #[test]
    fn only_this_mac_s_page_is_local() {
        assert!(is_local_page("http://127.0.0.1:8799/", Some(8799)));
        assert!(is_local_page("http://localhost:8799/spaces/x?y#z", Some(8799)));
        assert!(!is_local_page("http://127.0.0.1:8799/", Some(8800)), "another port");
        assert!(!is_local_page("http://127.0.0.1:8799/", None), "This Mac's kwant isn't running");
        assert!(!is_local_page("http://127.0.0.1:0/", Some(0)));
        assert!(!is_local_page("https://kwant.jumpin.games/", Some(8799)));
        assert!(!is_local_page("http://127.0.0.1:8799@evil.example/", Some(8799)));
        assert!(!is_local_page("http://evil.example/?127.0.0.1:8799", Some(8799)));
        assert!(!is_local_page("http://127.0.0.1:87990/", Some(8799)));
        assert!(!is_local_page("tauri://localhost/index.html", Some(8799)), "the loading page");
    }

    #[test]
    fn a_server_window_never_learns_about_other_places() {
        for label in ["server-68747470733a2f2f6b77616e742e6a756d70696e2e67616d6573", "server-", "connect", PICKER_WINDOW, "kwant-nudge-pill", "perch"] {
            assert_eq!(elsewhere_script(label, true, 3), None, "{label}");
            assert_eq!(elsewhere_script(label, false, 3), None, "{label}");
        }
        assert_eq!(elsewhere_script(MAIN_WINDOW, false, 3), None, "This Mac's window off its own kwant (loading page)");
        let js = elsewhere_script(MAIN_WINDOW, true, 3).expect("This Mac's page gets it");
        assert!(js.starts_with("window.__KWANT_ELSEWHERE__=3;"));
        assert!(js.contains("new Event('kwant-elsewhere')"));
        assert!(elsewhere_script(MAIN_WINDOW, true, 0).unwrap().contains("=0;"), "0 hides the badge");
    }

    #[test]
    fn elsewhere_counts_only_the_other_places() {
        assert_eq!(waiting_elsewhere(&[(true, 5), (false, 2), (false, 1)]), 3);
        assert_eq!(waiting_elsewhere(&[(true, 5)]), 0);
        assert_eq!(waiting_elsewhere(&[]), 0);
    }

    #[test]
    fn the_shortcut_script_only_asks_for_the_picker() {
        assert!(SHORTCUT_SCRIPT.contains("'kwant-app://places'"));
        assert_eq!(parse_app("kwant-app://places"), Some(AppAsk::Places { at: None }));
        assert!(!SHORTCUT_SCRIPT.contains("__KWANT"), "it gives the page nothing");
    }
}
