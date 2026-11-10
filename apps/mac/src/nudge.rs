//! "Needs you" nudges: what the menu-bar/tray icon, the macOS pill and the Linux notification
//! know about the things waiting on the person. Plain std + serde_json, so every rule here is
//! tested without a window; the platform parts live in `nudge_mac.rs` and `nudge_linux.rs`.
//!
//! What counts is everything `GET /api/away` lists as `waiting`: open questions and pending
//! approvals, in every space. Finished and failed work never counts.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A replay of the icon animation while something still waits.
pub const REPLAY_EVERY: Duration = Duration::from_secs(15 * 60);
/// The app's own settings file in the data dir.
pub const PREFS_FILE: &str = "desktop-app.json";
/// `since` far in the future: `/api/away` then lists no finished or failed work, only what
/// waits (which it lists whatever `since` is), so the 5-second poll stays small.
const SINCE_NOTHING: &str = "2999-01-01T00:00:00Z";
/// Seen ids kept before the ones no longer waiting are dropped.
const SEEN_CAP: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Question,
    Approval,
}

/// One thing waiting on the person, as `/api/away` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    pub id: String,
    pub space_id: Option<String>,
    pub kwant_id: String,
    pub kwant_name: String,
    /// Already clipped and secret-masked by the server.
    pub title: String,
    pub thread_id: Option<String>,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    /// The place it waits in (`places::Place::key`: `this_mac` or a server's origin); a click
    /// opens that place's window.
    pub place: String,
    /// What the nudge calls the place ("This Mac", the host), or empty while only one place
    /// runs (no prefix then: P7).
    pub place_label: String,
}

impl Item {
    /// The same item in a seen-set: ids are unique per kind on one server; two places can
    /// reuse an id.
    pub fn key(&self) -> String {
        let kind = match self.kind {
            Kind::Question => "q",
            Kind::Approval => "a",
        };
        format!("{}|{kind}:{}", self.place, self.id)
    }
}

/// What waits in one space: the listed items and the server's count (which can pass the
/// 50 items a list holds).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Waiting {
    pub items: Vec<Item>,
    pub count: usize,
}

impl Waiting {
    pub fn add(&mut self, other: Waiting) {
        self.items.extend(other.items);
        self.count += other.count;
    }

    /// Marks every item with the place it waits in (`label` empty: no prefix).
    pub fn in_place(mut self, place: &str, label: &str) -> Waiting {
        for i in &mut self.items {
            i.place = place.to_string();
            i.place_label = label.to_string();
        }
        self
    }
}

/// The second line of a nudge: "kwant.jumpin.games · Use Bash (…)" once more than one place
/// runs (Mira's nudge.html), the text alone before that.
pub fn with_place(label: &str, text: &str) -> String {
    if label.is_empty() {
        text.to_string()
    } else {
        format!("{label} · {text}")
    }
}

/// The path of `/api/away` for a space (`None`: the default space).
pub fn away_path(space_id: Option<&str>) -> String {
    match space_id {
        Some(s) => format!("/api/away?since={SINCE_NOTHING}&space={}", encode(s)),
        None => format!("/api/away?since={SINCE_NOTHING}"),
    }
}

/// The ids of the spaces `GET /api/spaces` lists (archived ones aren't listed).
pub fn parse_spaces(body: &str) -> Option<Vec<String>> {
    let rows: Vec<serde_json::Value> = serde_json::from_str(body).ok()?;
    Some(rows.iter().filter_map(|r| r["id"].as_str().map(str::to_string)).collect())
}

/// A `GET /api/away` body: its `waiting` questions and approvals and `unseen.waiting`.
pub fn parse_away(body: &str, space_id: Option<&str>) -> Option<Waiting> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let rows = v["waiting"].as_array()?;
    let text = |r: &serde_json::Value, k: &str| r[k].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    let items: Vec<Item> = rows
        .iter()
        .filter_map(|r| {
            let kind = match r["kind"].as_str()? {
                "question" => Kind::Question,
                "approval" => Kind::Approval,
                _ => return None,
            };
            let link = &r["link"];
            Some(Item {
                kind,
                id: text(r, "id")?,
                space_id: space_id.map(str::to_string),
                kwant_id: text(r, "kwant_id").unwrap_or_default(),
                kwant_name: text(r, "kwant_name").unwrap_or_else(|| "A bot".to_string()),
                title: text(r, "title").unwrap_or_default(),
                thread_id: text(link, "thread_id"),
                channel_id: text(link, "channel_id"),
                direct_kwant_id: text(link, "direct_kwant_id"),
                place: String::new(),
                place_label: String::new(),
            })
        })
        .collect();
    let count = v["unseen"]["waiting"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(items.len())
        .max(items.len());
    Some(Waiting { items, count })
}

/// The menu's status line and the number next to the icon (`None`: nothing waits).
/// Approvals keep their wording when only approvals wait; questions get theirs; a mix is "things".
pub fn status_line(waiting: &Waiting, port: u16) -> (String, Option<String>) {
    let n = waiting.count;
    if n == 0 {
        return (format!("Running on port {port}"), None);
    }
    let questions = waiting.items.iter().filter(|i| i.kind == Kind::Question).count();
    let approvals = waiting.items.len() - questions;
    let listed_all = waiting.items.len() == n;
    let noun = match (questions, approvals) {
        (0, _) if listed_all => ("approval", "approvals"),
        (_, 0) if listed_all => ("question", "questions"),
        _ => ("thing", "things"),
    };
    let line = match n {
        1 => format!("1 {} waiting for you", noun.0),
        _ => format!("{n} {} waiting for you", noun.1),
    };
    (line, Some(n.to_string()))
}


/// What one poll changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Change {
    /// Items not seen before, newest first as the server lists them.
    pub arrived: Vec<Item>,
    /// Play the icon animation (something arrived, or the 15-minute replay is due).
    pub play: bool,
    pub count: usize,
    /// The keys of everything still waiting, so a pill can drop what was answered meanwhile.
    pub live: HashSet<String>,
}

/// Remembers which waiting items the person was already nudged about.
#[derive(Debug, Default)]
pub struct Tracker {
    seen: HashSet<String>,
    last_play: Option<Instant>,
}

impl Tracker {
    pub fn update(&mut self, waiting: &Waiting, now: Instant) -> Change {
        let arrived: Vec<Item> = waiting.items.iter().filter(|i| !self.seen.contains(&i.key())).cloned().collect();
        self.seen.extend(arrived.iter().map(Item::key));
        let live: HashSet<String> = waiting.items.iter().map(Item::key).collect();
        if self.seen.len() > SEEN_CAP {
            // Answered items never come back with the same id; keep only what still waits
            self.seen.retain(|k| live.contains(k));
        }
        let due = waiting.count > 0 && self.last_play.is_some_and(|t| now.duration_since(t) >= REPLAY_EVERY);
        let play = !arrived.is_empty() || due;
        if play {
            self.last_play = Some(now);
        }
        if waiting.count == 0 {
            self.last_play = None;
        }
        Change { arrived, play, count: waiting.count, live }
    }
}

impl Kind {
    /// The name `/api/away` (and the pill page) use.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // only the macOS pill page takes it
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Approval => "approval",
            Kind::Question => "question",
        }
    }
}

/// The first line of a nudge for one item.
pub fn headline(item: &Item) -> String {
    match item.kind {
        Kind::Approval => format!("{} needs your OK", item.kwant_name),
        Kind::Question => format!("{} asked you something", item.kwant_name),
    }
}

/// What the pill (or a notification) says, and where a click goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pill {
    pub headline: String,
    pub detail: String,
    /// The avatar's letter; empty for several items.
    pub initial: String,
    /// The kwant page to open, a path such as `/s/spc_default/c/ch1`.
    pub target: String,
    /// The place whose window opens (`places::Place::key`).
    pub place: String,
}

/// The items one pill gathers: a pill only gathers items from one place (Mira's nudge.html,
/// "Several items from one place"), the place of the first one.
pub fn one_place(items: &[Item]) -> Vec<Item> {
    match items.first() {
        Some(first) => items.iter().filter(|i| i.place == first.place).cloned().collect(),
        None => Vec::new(),
    }
}

/// The pill for `items` (newest first), all arriving within one pill's time: one item names its
/// bot and opens its chat; several say how many and open the Inbox.
pub fn pill(items: &[Item]) -> Option<Pill> {
    match items {
        [] => None,
        [one] => Some(Pill {
            headline: headline(one),
            detail: with_place(&one.place_label, &one.title),
            initial: one.kwant_name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default(),
            target: target_path(one),
            place: one.place.clone(),
        }),
        [first, ..] => {
            let mut names: Vec<&str> = Vec::new();
            for i in items {
                if !names.contains(&i.kwant_name.as_str()) {
                    names.push(&i.kwant_name);
                }
            }
            let detail = match names.as_slice() {
                [a] => format!("All from {a}"),
                [a, b] => format!("{a} and {b}"),
                [a, b, c] => format!("{a}, {b} and {c}"),
                [a, b, rest @ ..] => format!("{a}, {b} and {} more", rest.len()),
                [] => String::new(),
            };
            Some(Pill {
                headline: format!("{} things need you", items.len()),
                detail: with_place(&first.place_label, &detail),
                initial: String::new(),
                target: inbox_path(first.space_id.as_deref()),
                place: first.place.clone(),
            })
        }
    }
}

fn space_base(space_id: Option<&str>) -> String {
    format!("/s/{}", encode(space_id.unwrap_or("spc_default")))
}

/// The Inbox of a space.
pub fn inbox_path(space_id: Option<&str>) -> String {
    format!("{}/inbox", space_base(space_id))
}

/// Where an item opens in the kwant window: its channel (a team chat is one), its bot's
/// conversation (`/b/<bot>/t/<thread>`, 0.8.1 F6), else its bot's chat. An approval adds
/// `?card=<id>` so the page shows that card. With no bot to open, the Inbox.
pub fn target_path(item: &Item) -> String {
    let base = space_base(item.space_id.as_deref());
    let card = match item.kind {
        Kind::Approval if !item.id.is_empty() => format!("?card={}", encode(&item.id)),
        _ => String::new(),
    };
    if let Some(c) = &item.channel_id {
        return format!("{base}/c/{}{card}", encode(c));
    }
    let bot = item.direct_kwant_id.as_deref().filter(|b| !b.is_empty()).unwrap_or(&item.kwant_id);
    if bot.is_empty() {
        return format!("{base}/inbox");
    }
    if let Some(t) = item.thread_id.as_deref().filter(|t| !t.is_empty()) {
        return format!("{base}/b/{}/t/{}{card}", encode(bot), encode(t));
    }
    format!("{base}/b/{}{card}", encode(bot))
}

/// Percent-encodes everything but RFC 3986 unreserved characters (a path segment or query value).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Two places of the pill window (points) count as the same within half a point.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // only the macOS pill is placed
pub fn same_spot(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 0.5 && (a.1 - b.1).abs() < 0.5
}

/// Waits for the menu-bar icon to stop moving before a fresh pill shows: reads where the pill
/// would go, then again every `every`, until `need` reads in a row agree or `cap` has passed on
/// `elapsed` (the wall clock since the wait began, so slow reads count too; the last read can end
/// up to one read past it). The 0 -> 1 nudge sets the icon's count and AppKit lays the bar out
/// again a moment later, sometimes in two steps (width first, then x), so the first read alone
/// would put the pill ~11 pt off, and two early reads can agree on a spot that still moves.
/// Returns the last read (`None`: the screen couldn't be read) and how many reads it took.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn settle(
    mut read: impl FnMut() -> Option<(f64, f64)>,
    every: Duration,
    need: usize,
    cap: Duration,
    mut elapsed: impl FnMut() -> Duration,
    mut sleep: impl FnMut(Duration),
) -> (Option<(f64, f64)>, usize) {
    let mut last = read();
    let (mut reads, mut same) = (1, 1);
    while same < need && elapsed() + every <= cap {
        sleep(every);
        let now = read();
        reads += 1;
        let agree = match (last, now) {
            (Some(a), Some(b)) => same_spot(a, b),
            (None, None) => true,
            _ => false,
        };
        same = if agree { same + 1 } else { 1 };
        last = now;
    }
    (last, reads)
}

/// The app's own settings, kept in [`PREFS_FILE`] in the data dir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prefs {
    /// Icon animation, pill and notifications. Off keeps the count.
    pub show_nudges: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { show_nudges: true }
    }
}

pub fn prefs_path(data_dir: &Path) -> PathBuf {
    data_dir.join(PREFS_FILE)
}

impl Prefs {
    /// The saved settings; a missing or unreadable file means the defaults.
    pub fn load(data_dir: &Path) -> Prefs {
        let saved = std::fs::read_to_string(prefs_path(data_dir))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
        Prefs {
            show_nudges: saved.and_then(|v| v["show_nudges"].as_bool()).unwrap_or(true),
        }
    }

    /// Saves the settings, keeping any other keys the file holds.
    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        let path = prefs_path(data_dir);
        let mut v = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .filter(|v| v.is_object())
            .unwrap_or_else(|| serde_json::json!({}));
        v["show_nudges"] = serde_json::Value::Bool(self.show_nudges);
        std::fs::create_dir_all(data_dir)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&v).unwrap_or_default())?;
        std::fs::rename(tmp, path)
    }
}

/// `--demo-nudge`: made-up waiting items that arrive on a timeline, so CI can screenshot a
/// single pill, a second one, two arriving together (coalesced), and one under Reduce Motion.
pub mod demo {
    use super::{Item, Kind, Waiting};
    use std::time::Duration;

    /// When each step's items appear, counted from the first poll.
    pub const STEPS: [Duration; 4] =
        [Duration::from_secs(6), Duration::from_secs(30), Duration::from_secs(54), Duration::from_secs(78)];
    /// The step whose nudge plays as under Reduce Motion (the runner's setting can't be changed).
    pub const REDUCE_MOTION_STEP: usize = 4;
    /// The demo quits on its own this long after start.
    pub const RUNS_FOR: Duration = Duration::from_secs(100);

    fn item(kind: Kind, id: &str, name: &str, title: &str) -> Item {
        Item {
            kind,
            id: id.to_string(),
            space_id: Some("spc_default".to_string()),
            kwant_id: format!("bot-{}", name.to_lowercase()),
            kwant_name: name.to_string(),
            title: title.to_string(),
            thread_id: None,
            channel_id: None,
            direct_kwant_id: None,
            place: crate::places::THIS_MAC.to_string(),
            place_label: String::new(),
        }
    }

    /// How many steps have happened `elapsed` after start.
    pub fn step(elapsed: Duration) -> usize {
        STEPS.iter().filter(|t| elapsed >= **t).count()
    }

    /// What waits after `step` steps.
    pub fn waiting(step: usize) -> Waiting {
        let all = [
            (1, item(Kind::Approval, "demo-a1", "Juno", "Use Bash (deploy the site to kwant.si)")),
            (2, item(Kind::Question, "demo-q1", "Mira", "Which colour should the sticker badge use?")),
            (3, item(Kind::Approval, "demo-a2", "Rigel", "Use Bash (run the release checks)")),
            (3, item(Kind::Question, "demo-q2", "Pixel", "Ship the sidebar change today?")),
            (4, item(Kind::Question, "demo-q3", "Vega", "Keep the old results view for a week?")),
        ];
        // newest first, as /api/away lists them
        let items: Vec<Item> = all.into_iter().rev().filter(|(s, _)| *s <= step).map(|(_, i)| i).collect();
        Waiting { count: items.len(), items }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: Kind, id: &str, name: &str) -> Item {
        Item {
            kind,
            id: id.into(),
            space_id: Some("spc_default".into()),
            kwant_id: format!("k-{id}"),
            kwant_name: name.into(),
            title: format!("title {id}"),
            thread_id: None,
            channel_id: None,
            direct_kwant_id: None,
            place: String::new(),
            place_label: String::new(),
        }
    }

    fn waiting(items: Vec<Item>) -> Waiting {
        Waiting { count: items.len(), items }
    }

    const AWAY: &str = r#"{
      "since": "2999-01-01T00:00:00.000Z",
      "waiting": [
        {"kind":"approval","id":"ap1","kwant_id":"k1","kwant_name":"Juno","title":"Use Bash","summary":"command: ls",
         "status":"pending","timestamp":"2026-11-03T21:00:00.000Z",
         "link":{"thread_id":null,"channel_id":"ch 1","direct_kwant_id":null},"tool_name":"Bash","details":{}},
        {"kind":"question","id":"q1","kwant_id":"k2","kwant_name":"Mira","title":"Which colour?","summary":"Waiting for your answer",
         "status":"open","timestamp":"2026-11-03T20:00:00.000Z",
         "link":{"thread_id":"t1","channel_id":null,"direct_kwant_id":null},"choices":[]},
        {"kind":"delegation","id":"d1","kwant_id":"k3","title":"stuck","link":{}}
      ],
      "finished": [], "failed": [],
      "unseen": {"waiting": 2, "finished": 0, "failed": 0, "total": 2}
    }"#;

    #[test]
    fn away_lists_questions_and_approvals_with_their_links() {
        let w = parse_away(AWAY, Some("sp1")).unwrap();
        assert_eq!(w.count, 2);
        assert_eq!(w.items.len(), 2, "only questions and approvals count");
        assert_eq!(w.items[0].kind, Kind::Approval);
        assert_eq!(w.items[0].channel_id.as_deref(), Some("ch 1"));
        assert_eq!(w.items[0].space_id.as_deref(), Some("sp1"));
        assert_eq!(w.items[1].kind, Kind::Question);
        assert_eq!(w.items[1].thread_id.as_deref(), Some("t1"));
        assert!(parse_away("not json", None).is_none());
        assert!(parse_away(r#"{"error":"no such space"}"#, None).is_none());
    }

    #[test]
    fn the_count_is_the_server_s_even_past_the_listed_items() {
        let body = r#"{"waiting":[{"kind":"question","id":"q","kwant_name":"A","title":"x","link":{}}],"unseen":{"waiting":57}}"#;
        let w = parse_away(body, None).unwrap();
        assert_eq!((w.count, w.items.len()), (57, 1));
        // a mixed or partly listed count is "things"
        assert_eq!(status_line(&w, 8799).0, "57 things waiting for you");
    }

    #[test]
    fn the_poll_asks_for_waiting_items_only_in_each_space() {
        assert_eq!(away_path(None), "/api/away?since=2999-01-01T00:00:00Z");
        assert_eq!(away_path(Some("sp 1&x")), "/api/away?since=2999-01-01T00:00:00Z&space=sp%201%26x");
        assert_eq!(parse_spaces(r#"[{"id":"spc_default","name":"Home"},{"id":"s2"}]"#).unwrap(), vec!["spc_default", "s2"]);
        assert!(parse_spaces("{}").is_none());
    }

    #[test]
    fn status_line_words_follow_what_waits() {
        assert_eq!(status_line(&Waiting::default(), 8799), ("Running on port 8799".into(), None));
        let one_a = waiting(vec![item(Kind::Approval, "a", "J")]);
        assert_eq!(status_line(&one_a, 1), ("1 approval waiting for you".into(), Some("1".into())));
        let two_a = waiting(vec![item(Kind::Approval, "a", "J"), item(Kind::Approval, "b", "J")]);
        assert_eq!(status_line(&two_a, 1).0, "2 approvals waiting for you");
        let one_q = waiting(vec![item(Kind::Question, "q", "J")]);
        assert_eq!(status_line(&one_q, 1).0, "1 question waiting for you");
        let mix = waiting(vec![item(Kind::Question, "q", "J"), item(Kind::Approval, "a", "J"), item(Kind::Approval, "b", "J")]);
        assert_eq!(status_line(&mix, 1), ("3 things waiting for you".into(), Some("3".into())));
    }

    /// P7: once more than one place runs, a nudge names its place, a pill gathers one place's
    /// items only, and a click opens that place.
    #[test]
    fn nudges_name_their_place_once_two_places_run() {
        let server = "https://kwant.jumpin.games";
        let one = waiting(vec![item(Kind::Approval, "a", "Juno")]).in_place("this_mac", "");
        let p = pill(&one.items).unwrap();
        assert_eq!((p.headline.as_str(), p.detail.as_str(), p.place.as_str()), ("Juno needs your OK", "title a", "this_mac"));

        let remote = waiting(vec![item(Kind::Approval, "a", "Juno")]).in_place(server, "kwant.jumpin.games");
        let p = pill(&remote.items).unwrap();
        assert_eq!(p.detail, "kwant.jumpin.games · title a");
        assert_eq!(p.place, server);
        assert_eq!(p.target, "/s/spc_default/b/k-a?card=a");

        let three = waiting(vec![item(Kind::Question, "q", "Juno"), item(Kind::Approval, "b", "Mira"), item(Kind::Approval, "c", "Iris")])
            .in_place(server, "kwant.jumpin.games");
        assert_eq!(pill(&three.items).unwrap().detail, "kwant.jumpin.games · Juno, Mira and Iris");

        // the same id on two places is two items
        let local = waiting(vec![item(Kind::Approval, "a", "Juno")]).in_place("this_mac", "This Mac");
        assert_ne!(local.items[0].key(), remote.items[0].key());
        let mut both = remote.clone();
        both.add(local.clone());
        let mut t = Tracker::default();
        let c = t.update(&both, Instant::now());
        assert_eq!(c.arrived.len(), 2);
        assert_eq!(c.count, 2);
        // one pill, one place: the first arrival's
        let gathered = one_place(&c.arrived);
        assert_eq!(gathered.len(), 1);
        assert_eq!(gathered[0].place, server);
        assert_eq!(pill(&gathered).unwrap().detail, "kwant.jumpin.games · title a");
        assert_eq!(one_place(&[]), Vec::<Item>::new());
        assert_eq!(with_place("", "x"), "x");
        assert_eq!(with_place("This Mac", "x"), "This Mac · x");
    }

    #[test]
    fn a_new_item_plays_once_and_a_seen_one_never_again() {
        let mut t = Tracker::default();
        let now = Instant::now();
        let w = waiting(vec![item(Kind::Approval, "a", "Juno")]);
        let c = t.update(&w, now);
        assert!(c.play);
        assert_eq!(c.arrived.len(), 1);
        // the same item on the next polls: nothing new, no animation
        for s in 1..10 {
            let c = t.update(&w, now + Duration::from_secs(5 * s));
            assert!(!c.play && c.arrived.is_empty());
            assert_eq!(c.count, 1);
        }
        // a second one arrives: only it is new
        let w2 = waiting(vec![item(Kind::Question, "q", "Mira"), item(Kind::Approval, "a", "Juno")]);
        let c = t.update(&w2, now + Duration::from_secs(60));
        assert!(c.play);
        assert_eq!(c.arrived.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["q"]);
    }

    #[test]
    fn a_question_and_an_approval_with_the_same_id_are_two_items() {
        let mut t = Tracker::default();
        let now = Instant::now();
        t.update(&waiting(vec![item(Kind::Approval, "x", "J")]), now);
        let c = t.update(&waiting(vec![item(Kind::Question, "x", "J"), item(Kind::Approval, "x", "J")]), now);
        assert_eq!(c.arrived.len(), 1);
        assert_eq!(c.arrived[0].kind, Kind::Question);
    }

    #[test]
    fn the_icon_replays_every_15_minutes_while_something_waits_and_stops_at_zero() {
        let mut t = Tracker::default();
        let start = Instant::now();
        let w = waiting(vec![item(Kind::Approval, "a", "Juno")]);
        assert!(t.update(&w, start).play);
        assert!(!t.update(&w, start + Duration::from_secs(14 * 60 + 59)).play);
        let c = t.update(&w, start + REPLAY_EVERY);
        assert!(c.play && c.arrived.is_empty(), "a replay brings no pill");
        assert!(!t.update(&w, start + REPLAY_EVERY + Duration::from_secs(5)).play);
        // answered: nothing waits, no replays
        assert!(!t.update(&Waiting::default(), start + REPLAY_EVERY * 2).play);
        assert!(!t.update(&Waiting::default(), start + REPLAY_EVERY * 4).play);
    }

    #[test]
    fn the_seen_set_stays_bounded() {
        let mut t = Tracker::default();
        let now = Instant::now();
        for i in 0..(SEEN_CAP + 5) {
            t.update(&waiting(vec![item(Kind::Question, &i.to_string(), "J")]), now);
        }
        assert!(t.seen.len() <= SEEN_CAP);
        // what still waits stays seen
        let last = (SEEN_CAP + 4).to_string();
        assert!(t.update(&waiting(vec![item(Kind::Question, &last, "J")]), now).arrived.is_empty());
    }

    #[test]
    fn one_item_s_pill_names_its_bot_and_opens_its_chat() {
        let mut a = item(Kind::Approval, "a", "juno");
        a.channel_id = Some("ch/1".into());
        let p = pill(std::slice::from_ref(&a)).unwrap();
        assert_eq!(p.headline, "juno needs your OK");
        assert_eq!(p.detail, "title a");
        assert_eq!(p.initial, "J");
        assert_eq!(p.target, "/s/spc_default/c/ch%2F1?card=a");
        let q = item(Kind::Question, "q", "Mira");
        let p = pill(&[q]).unwrap();
        assert_eq!(p.headline, "Mira asked you something");
        assert_eq!(p.target, "/s/spc_default/b/k-q");
        assert!(pill(&[]).is_none());
    }

    #[test]
    fn several_arrivals_coalesce_into_one_pill_that_opens_the_inbox() {
        let items = vec![item(Kind::Approval, "a", "Juno"), item(Kind::Question, "q", "Mira"), item(Kind::Question, "r", "Juno")];
        let p = pill(&items).unwrap();
        assert_eq!(p.headline, "3 things need you");
        assert_eq!(p.detail, "Juno and Mira");
        assert_eq!(p.initial, "");
        assert_eq!(p.target, "/s/spc_default/inbox");
        let same = vec![item(Kind::Approval, "a", "Juno"), item(Kind::Approval, "b", "Juno")];
        assert_eq!(pill(&same).unwrap().detail, "All from Juno");
        let three: Vec<Item> = ["Juno", "Iris", "Lin"].iter().map(|n| item(Kind::Question, n, n)).collect();
        assert_eq!(pill(&three).unwrap().detail, "Juno, Iris and Lin");
        let many: Vec<Item> = ["A", "B", "C", "D", "E"].iter().map(|n| item(Kind::Question, n, n)).collect();
        assert_eq!(pill(&many).unwrap().detail, "A, B and 3 more");
    }

    #[test]
    fn targets_follow_the_item_s_link() {
        let mut i = item(Kind::Question, "q", "J");
        i.space_id = Some("my space".into());
        i.direct_kwant_id = Some("k9".into());
        assert_eq!(target_path(&i), "/s/my%20space/b/k9");
        i.thread_id = Some("t1".into());
        assert_eq!(target_path(&i), "/s/my%20space/b/k9/t/t1", "a thread opens itself (0.8.1 F6)");
        let mut in_thread = item(Kind::Question, "q", "J");
        in_thread.space_id = Some("s".into());
        in_thread.kwant_id = "k 1".into();
        in_thread.thread_id = Some("t/2".into());
        assert_eq!(target_path(&in_thread), "/s/s/b/k%201/t/t%2F2", "a thread item without a direct link names its bot");
        let mut approval = item(Kind::Approval, "ap 1", "J");
        approval.space_id = Some("s".into());
        approval.kwant_id = "k1".into();
        approval.thread_id = Some("t1".into());
        assert_eq!(target_path(&approval), "/s/s/b/k1/t/t1?card=ap%201", "an approval names its card");
        approval.thread_id = None;
        approval.channel_id = Some("team-c".into());
        assert_eq!(target_path(&approval), "/s/s/c/team-c?card=ap%201", "a team chat approval opens the team chat");
        approval.channel_id = None;
        assert_eq!(target_path(&approval), "/s/s/b/k1?card=ap%201");
        i.channel_id = Some("c1".into());
        assert_eq!(target_path(&i), "/s/my%20space/c/c1");
        let mut none = item(Kind::Question, "q", "J");
        none.space_id = None;
        none.kwant_id = String::new();
        assert_eq!(target_path(&none), "/s/spc_default/inbox");
        // nothing a title can carry reaches the path
        assert_eq!(encode("../../x?y#z"), "..%2F..%2Fx%3Fy%23z");
    }

    #[test]
    fn a_fresh_pill_waits_until_the_icon_reads_the_same_three_times() {
        let every = Duration::from_millis(30);
        let cap = Duration::from_millis(250);
        // each read costs `cost` ms of wall clock and each sleep its length; (spot, reads, ms)
        let run = |reads: Vec<Option<(f64, f64)>>, cost: u64| {
            let mut it = reads.into_iter();
            let clock = std::cell::Cell::new(Duration::ZERO);
            let read = || {
                clock.set(clock.get() + Duration::from_millis(cost));
                it.next().flatten()
            };
            let (spot, n) = settle(read, every, 3, cap, || clock.get(), |d| clock.set(clock.get() + d));
            (spot, n, clock.get().as_millis())
        };
        let at = |x: f64| Some((x, 26.0));
        // already settled: three reads, 60 ms, never sooner
        assert_eq!(run(vec![at(785.0); 9], 0), (at(785.0), 3, 60));
        // the 0 -> 1 case: the stale rect, then the settled one three times
        assert_eq!(run(vec![at(795.5), at(784.0), at(784.2), at(784.1)], 0), (at(784.1), 4, 90));
        // step 3 on CI: two early reads agree, then macOS moves the item 2 pt; two equal reads
        // are not enough, so it doesn't settle at 30 or 60 ms but on the moved spot at 120 ms
        let late = vec![at(616.0), at(616.0), at(614.0), at(614.0), at(614.0), at(612.0)];
        assert_eq!(run(late, 0), (at(614.0), 5, 120));
        // never settles: gives up at the cap (240 ms, 9 reads) with the newest place
        let drifting: Vec<_> = (0..20).map(|i| at(700.0 + i as f64)).collect();
        assert_eq!(run(drifting.clone(), 0), (at(708.0), 9, 240));
        // slow reads (40 ms each: a read waits for the main thread) count against the cap too
        assert_eq!(run(drifting, 40), (at(703.0), 4, 250));
        // no screen to read: settles on None after three reads
        assert_eq!(run(vec![None; 3], 0), (None, 3, 60));
        assert!(same_spot((1.0, 2.0), (1.4, 1.6)) && !same_spot((1.0, 2.0), (1.5, 2.0)));
    }

    #[test]
    fn show_nudges_is_on_by_default_and_saved_in_the_data_dir() {
        let dir = std::env::temp_dir().join(format!("kwant-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(Prefs::load(&dir).show_nudges, "on when nothing is saved");
        Prefs { show_nudges: false }.save(&dir).unwrap();
        assert!(!Prefs::load(&dir).show_nudges);
        // other keys in the file survive a save
        std::fs::write(prefs_path(&dir), r#"{"show_nudges":false,"other":1}"#).unwrap();
        Prefs { show_nudges: true }.save(&dir).unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(prefs_path(&dir)).unwrap()).unwrap();
        assert_eq!(v["other"], 1);
        assert!(Prefs::load(&dir).show_nudges);
        // a broken file is the defaults
        std::fs::write(prefs_path(&dir), "{oops").unwrap();
        assert!(Prefs::load(&dir).show_nudges);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_demo_brings_one_then_one_then_two_together() {
        assert_eq!(demo::step(Duration::from_secs(1)), 0);
        assert_eq!(demo::waiting(0).count, 0);
        let mut t = Tracker::default();
        let now = Instant::now();
        let arrived: Vec<usize> = (1..=4).map(|s| t.update(&demo::waiting(s), now).arrived.len()).collect();
        assert_eq!(arrived, vec![1, 1, 2, 1]);
        assert_eq!(pill(&t.update(&demo::waiting(4), now).arrived), None);
        assert_eq!(demo::step(Duration::from_secs(60)), 3);
        assert_eq!(demo::step(demo::RUNS_FOR), 4);
        assert_eq!(pill(&demo::waiting(3).items[..2]).unwrap().headline, "2 things need you");
    }
}
