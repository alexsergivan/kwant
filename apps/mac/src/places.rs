//! The places the app shows: the kwant on this Mac and every server the person added, each in
//! its own window, all running at once (next-075/side-by-side/spec.md P1). Saved in the data dir
//! as `desktop-places.json`:
//!
//! ```json
//! {"version":1,"this_mac":true,"servers":["https://kwant.jumpin.games"],"open":["this_mac","https://kwant.jumpin.games"]}
//! ```
//!
//! Only origins are ever saved, built by `target::parse_address` with its rules: never a token, a
//! path, a query, a fragment or a user name. A missing file is migrated from the old
//! `desktop-target.json` (the 0.7.5 test builds' "one place, relaunch to switch"); an odd or
//! unreadable file means This Mac only.

use crate::target::{parse_address, target_path, Origin, Saved};
use std::path::{Path, PathBuf};

pub const PLACES_FILE: &str = "desktop-places.json";
const VERSION: u64 = 1;
/// The `open` and window key of the kwant on this Mac.
pub const THIS_MAC: &str = "this_mac";

/// One place: the kwant on this Mac, or a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    ThisMac,
    Server(Origin),
}

impl Place {
    /// The key used in the file's `open` list and in nudges: `this_mac` or the origin.
    pub fn key(&self) -> String {
        match self {
            Place::ThisMac => THIS_MAC.to_string(),
            Place::Server(o) => o.as_str(),
        }
    }

    /// What people read: "This Mac" or the server's host.
    pub fn label(&self) -> String {
        match self {
            Place::ThisMac => THIS_MAC_LABEL.to_string(),
            Place::Server(o) => o.host(),
        }
    }

    /// The window's label. Tauri labels allow `a-zA-Z0-9-/:_`, so a server's origin goes in as
    /// hex: unique, and nothing in an address can reach another window's label.
    pub fn window_label(&self) -> String {
        match self {
            Place::ThisMac => crate::MAIN.to_string(),
            Place::Server(o) => {
                let hex: String = o.as_str().bytes().map(|b| format!("{b:02x}")).collect();
                format!("{SERVER_WINDOW_PREFIX}{hex}")
            }
        }
    }
}

/// Window labels of servers start with this (capabilities/connect.json lists `server-*`).
pub const SERVER_WINDOW_PREFIX: &str = "server-";

/// What the app calls the kwant on this machine (Mira's design/menu.html: "This computer" on
/// Linux), and its item in the menu.
#[cfg(target_os = "macos")]
pub const THIS_MAC_LABEL: &str = "This Mac";
#[cfg(not(target_os = "macos"))]
pub const THIS_MAC_LABEL: &str = "This computer";
#[cfg(target_os = "macos")]
pub const RUN_HERE: &str = "Run kwant on this Mac";
#[cfg(not(target_os = "macos"))]
pub const RUN_HERE: &str = "Run kwant on this computer";

/// The window title: "kwant – This Mac", "kwant – kwant.jumpin.games"; "kwant" while This Mac is
/// the only place there is.
pub fn window_title(place: &Place, places: &Places) -> String {
    match place {
        Place::ThisMac if places.servers.is_empty() => "kwant".to_string(),
        _ => format!("kwant – {}", place.label()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Places {
    /// Run (and show) the kwant on this Mac.
    pub this_mac: bool,
    /// The servers, in the order they were added.
    pub servers: Vec<Origin>,
    /// The places whose window was open at Quit (they open again at launch).
    pub open: Vec<Place>,
}

impl Default for Places {
    /// This Mac only, with its window open: how the app has always started.
    fn default() -> Places {
        Places { this_mac: true, servers: Vec::new(), open: vec![Place::ThisMac] }
    }
}

/// A saved origin, accepted only in its exact saved form (as `desktop-target.json`'s).
fn saved_origin(value: &serde_json::Value) -> Option<Origin> {
    let text = value.as_str()?;
    let address = parse_address(text).ok()?;
    (address.sign_in.is_none() && address.origin.as_str() == text).then_some(address.origin)
}

pub fn places_path(data_dir: &Path) -> PathBuf {
    data_dir.join(PLACES_FILE)
}

impl Places {
    /// Every place, This Mac first (while it runs), then the servers.
    pub fn all(&self) -> Vec<Place> {
        let mut all = Vec::with_capacity(self.servers.len() + 1);
        if self.this_mac {
            all.push(Place::ThisMac);
        }
        all.extend(self.servers.iter().cloned().map(Place::Server));
        all
    }

    pub fn has(&self, place: &Place) -> bool {
        match place {
            Place::ThisMac => self.this_mac,
            Place::Server(o) => self.servers.contains(o),
        }
    }

    /// The place behind a window label.
    pub fn by_window(&self, label: &str) -> Option<Place> {
        self.all().into_iter().find(|p| p.window_label() == label)
    }

    /// Reads the file's text. `None`: not a places file this app understands.
    pub fn parse(text: &str) -> Option<Places> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        if v["version"].as_u64() != Some(VERSION) {
            return None;
        }
        let this_mac = v["this_mac"].as_bool()?;
        let mut servers: Vec<Origin> = Vec::new();
        for s in v["servers"].as_array()? {
            // one odd entry is dropped, not the whole file: the rest still opens
            if let Some(o) = saved_origin(s) {
                if !servers.contains(&o) {
                    servers.push(o);
                }
            }
        }
        let mut places = Places { this_mac, servers, open: Vec::new() };
        if let Some(open) = v["open"].as_array() {
            for key in open {
                let place = match key.as_str() {
                    Some(THIS_MAC) => Some(Place::ThisMac),
                    _ => saved_origin(key).map(Place::Server),
                };
                if let Some(p) = place.filter(|p| places.has(p)) {
                    places.set_open(&p, true);
                }
            }
        }
        Some(places)
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "version": VERSION,
            "this_mac": self.this_mac,
            "servers": self.servers.iter().map(Origin::as_str).collect::<Vec<_>>(),
            "open": self.open.iter().map(Place::key).collect::<Vec<_>>(),
        })
    }

    /// The saved places; with no file, the old `desktop-target.json` moved over (and deleted once
    /// the new file is written). Missing, unreadable or odd means This Mac only.
    pub fn load(data_dir: &Path) -> Places {
        match std::fs::read_to_string(places_path(data_dir)) {
            Ok(text) => Places::parse(&text).unwrap_or_else(|| {
                tracing::warn!("desktop-places.json isn't readable; opening This Mac only");
                Places::default()
            }),
            Err(_) => Places::migrate(data_dir),
        }
    }

    /// `Remote(o)` → servers [o], open [o]. `Local{last:Some(o)}` → servers [o], open [This Mac].
    /// `Local{last:None}` (or no old file) → This Mac only.
    fn migrate(data_dir: &Path) -> Places {
        let old = target_path(data_dir);
        if !old.exists() {
            return Places::default();
        }
        let places = match Saved::load(data_dir) {
            Saved::Remote(o) => Places { this_mac: true, servers: vec![o.clone()], open: vec![Place::Server(o)] },
            Saved::Local { last: Some(o) } => Places { this_mac: true, servers: vec![o], open: vec![Place::ThisMac] },
            Saved::Local { last: None } => Places::default(),
        };
        match places.save(data_dir) {
            Ok(()) => {
                if let Err(e) = std::fs::remove_file(&old) {
                    tracing::warn!(error = %e, "couldn't delete the old desktop-target.json");
                }
                tracing::info!(servers = places.servers.len(), "moved the saved server to desktop-places.json");
            }
            Err(e) => tracing::warn!(error = %e, "couldn't write desktop-places.json; keeping desktop-target.json"),
        }
        places
    }

    /// Written whole to a temporary file (0600) and renamed over the old one.
    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(data_dir)?;
        let path = places_path(data_dir);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&self.to_json()).unwrap_or_default())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(tmp, path)
    }

    /// Adds a server and marks its window open. `false`: it was already listed.
    pub fn add_server(&mut self, origin: Origin) -> bool {
        let place = Place::Server(origin.clone());
        let added = !self.servers.contains(&origin);
        if added {
            self.servers.push(origin);
        }
        self.set_open(&place, true);
        added
    }

    /// Takes a server out, with its window. `false`: it wasn't listed.
    pub fn remove_server(&mut self, origin: &Origin) -> bool {
        let before = self.servers.len();
        self.servers.retain(|o| o != origin);
        self.open.retain(|p| p != &Place::Server(origin.clone()));
        self.servers.len() != before
    }

    /// "Run kwant on this Mac": on opens its window too; off closes it.
    pub fn set_this_mac(&mut self, on: bool) {
        self.this_mac = on;
        self.set_open(&Place::ThisMac, on);
    }

    pub fn set_open(&mut self, place: &Place, open: bool) {
        let at = self.open.iter().position(|p| p == place);
        match (open, at) {
            (true, None) if self.has(place) => {
                self.open.push(place.clone());
                // the file lists This Mac first, then the servers in their order
                let order = self.all();
                self.open.sort_by_key(|p| order.iter().position(|q| q == p));
            }
            (false, Some(i)) => {
                self.open.remove(i);
            }
            _ => {}
        }
    }

    /// The places to open at launch: the ones open at Quit, else the first place there is (a
    /// person who closed every window still gets one).
    pub fn to_open(&self) -> Vec<Place> {
        if !self.open.is_empty() {
            return self.open.clone();
        }
        self.all().into_iter().take(1).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::TARGET_FILE;

    fn o(s: &str) -> Origin {
        parse_address(s).unwrap().origin
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kwant-places-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const SPEC_EXAMPLE: &str =
        r#"{"version":1,"this_mac":true,"servers":["https://kwant.jumpin.games"],"open":["this_mac","https://kwant.jumpin.games"]}"#;

    #[test]
    fn the_spec_s_example_parses_and_writes_back_the_same() {
        let p = Places::parse(SPEC_EXAMPLE).expect("parses");
        let server = o("https://kwant.jumpin.games");
        assert!(p.this_mac);
        assert_eq!(p.servers, vec![server.clone()]);
        assert_eq!(p.open, vec![Place::ThisMac, Place::Server(server)]);
        let back: serde_json::Value = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        assert_eq!(p.to_json(), back);
    }

    #[test]
    fn no_file_and_no_old_file_means_this_mac_only() {
        let d = dir("none");
        assert_eq!(Places::load(&d), Places::default());
        assert_eq!(Places::default().all(), vec![Place::ThisMac]);
        assert!(!places_path(&d).exists(), "nothing to migrate, nothing written");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_odd_or_unreadable_file_means_this_mac_only() {
        let d = dir("odd");
        for text in [
            "not json",
            "[]",
            r#"{"this_mac":true,"servers":[]}"#,
            r#"{"version":2,"this_mac":true,"servers":[]}"#,
            r#"{"version":1,"this_mac":"yes","servers":[]}"#,
            r#"{"version":1,"this_mac":true,"servers":"https://kwant.example.com"}"#,
        ] {
            std::fs::write(places_path(&d), text).unwrap();
            assert_eq!(Places::load(&d), Places::default(), "{text}");
        }
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn origins_follow_the_address_rules_and_odd_entries_are_dropped() {
        let text = r#"{"version":1,"this_mac":false,"servers":[
            "https://kwant.example.com",
            "https://kwant.example.com",
            "http://kwant.example.com",
            "https://kwant.example.com/path",
            "https://kwant.example.com/",
            "https://a@kwant.example.com",
            "https://other.example.com/#kwnt_token=kl1.TESTONLY",
            "http://127.0.0.1:2222",
            42
        ],"open":["https://kwant.example.com","this_mac","https://evil.example","http://127.0.0.1:2222"]}"#;
        let p = Places::parse(text).unwrap();
        assert_eq!(p.servers, vec![o("https://kwant.example.com"), o("http://127.0.0.1:2222")]);
        // This Mac is off, and evil.example isn't listed: neither is open
        assert_eq!(p.open, vec![Place::Server(o("https://kwant.example.com")), Place::Server(o("http://127.0.0.1:2222"))]);
        assert_eq!(p.all().len(), 2);
    }

    #[test]
    fn the_file_holds_origins_only_never_a_token() {
        let d = dir("token");
        let a = parse_address("https://kwant.example.com/s/x?q=1#kwnt_token=kl1.TESTONLY").unwrap();
        assert!(a.sign_in.is_some());
        let mut p = Places::default();
        assert!(p.add_server(a.origin));
        p.save(&d).unwrap();
        let text = std::fs::read_to_string(places_path(&d)).unwrap();
        for bad in ["TESTONLY", "kwnt_token", "kl1.", "/s/x", "q=1", "#"] {
            assert!(!text.contains(bad), "{bad} in {text}");
        }
        assert_eq!(Places::load(&d), p);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(places_path(&d)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(!d.join("desktop-places.json.tmp").exists());
        let _ = std::fs::remove_dir_all(d);
    }

    fn migrated(old: Saved) -> (Places, PathBuf) {
        let d = dir(&format!("mig-{}", matches!(old, Saved::Remote(_))));
        old.save(&d).unwrap();
        assert!(d.join(TARGET_FILE).exists());
        (Places::load(&d), d)
    }

    #[test]
    fn a_remote_test_build_migrates_to_its_server_open() {
        let server = o("https://kwant.jumpin.games");
        let (p, d) = migrated(Saved::Remote(server.clone()));
        assert_eq!(p, Places { this_mac: true, servers: vec![server.clone()], open: vec![Place::Server(server)] });
        assert!(!d.join(TARGET_FILE).exists(), "the old file is gone once the new one is written");
        assert_eq!(Places::load(&d), p, "the new file reads back the same");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_local_test_build_keeps_its_last_server_listed() {
        let server = o("https://kwant.jumpin.games");
        let d = dir("mig-local-last");
        Saved::Local { last: Some(server.clone()) }.save(&d).unwrap();
        let p = Places::load(&d);
        assert_eq!(p, Places { this_mac: true, servers: vec![server], open: vec![Place::ThisMac] });
        assert!(!d.join(TARGET_FILE).exists());
        let _ = std::fs::remove_dir_all(d);

        let d = dir("mig-local-none");
        Saved::Local { last: None }.save(&d).unwrap();
        assert_eq!(Places::load(&d), Places::default());
        assert!(!d.join(TARGET_FILE).exists());
        assert!(places_path(&d).exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_odd_old_file_migrates_to_this_mac_only() {
        let d = dir("mig-odd");
        std::fs::write(d.join(TARGET_FILE), r#"{"mode":"remote","origin":"https://kwant.example.com/#kwnt_token=kl1.TESTONLY"}"#).unwrap();
        assert_eq!(Places::load(&d), Places::default());
        let text = std::fs::read_to_string(places_path(&d)).unwrap();
        assert!(!text.contains("TESTONLY"), "{text}");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn adding_a_server_lists_it_once_and_opens_it() {
        let mut p = Places::default();
        let a = o("https://a.example.com");
        let b = o("https://b.example.com");
        assert!(p.add_server(a.clone()));
        assert!(p.add_server(b.clone()));
        assert!(!p.add_server(a.clone()), "already listed");
        assert_eq!(p.servers, vec![a.clone(), b.clone()]);
        assert_eq!(p.all(), vec![Place::ThisMac, Place::Server(a.clone()), Place::Server(b.clone())]);
        assert_eq!(p.open, vec![Place::ThisMac, Place::Server(a.clone()), Place::Server(b.clone())]);
        // re-adding a listed server whose window was closed opens it again
        p.set_open(&Place::Server(a.clone()), false);
        assert!(!p.add_server(a.clone()));
        assert_eq!(p.open, vec![Place::ThisMac, Place::Server(a), Place::Server(b)], "kept in place order");
    }

    #[test]
    fn removing_a_server_drops_it_and_its_window() {
        let mut p = Places::default();
        let a = o("https://a.example.com");
        p.add_server(a.clone());
        assert!(p.remove_server(&a));
        assert!(!p.remove_server(&a), "already gone");
        assert_eq!(p, Places::default());
    }

    #[test]
    fn this_mac_off_and_on() {
        let mut p = Places::default();
        let a = o("https://a.example.com");
        p.add_server(a.clone());
        p.set_this_mac(false);
        assert_eq!(p.all(), vec![Place::Server(a.clone())]);
        assert_eq!(p.open, vec![Place::Server(a.clone())]);
        // a closed This Mac can't be marked open
        p.set_open(&Place::ThisMac, true);
        assert_eq!(p.open, vec![Place::Server(a.clone())]);
        p.set_this_mac(true);
        assert_eq!(p.open, vec![Place::ThisMac, Place::Server(a)]);
    }

    #[test]
    fn with_every_window_closed_launch_opens_the_first_place() {
        let mut p = Places::default();
        p.set_open(&Place::ThisMac, false);
        assert_eq!(p.to_open(), vec![Place::ThisMac]);
        let a = o("https://a.example.com");
        p.add_server(a.clone());
        p.set_this_mac(false);
        p.set_open(&Place::Server(a.clone()), false);
        assert_eq!(p.to_open(), vec![Place::Server(a)]);
        p.servers.clear();
        assert!(p.to_open().is_empty(), "nothing to open: the menu offers Add a server");
    }

    #[test]
    fn window_labels_are_unique_and_tauri_safe() {
        let p = Places {
            this_mac: true,
            servers: vec![o("https://a-b.example.com"), o("https://a.b.example.com"), o("http://[::1]:2222"), o("https://kwänt.example")],
            open: vec![],
        };
        let labels: Vec<String> = p.all().iter().map(Place::window_label).collect();
        assert_eq!(labels[0], "main");
        for l in &labels {
            assert!(l.chars().all(|c| c.is_ascii_alphanumeric() || "-/:_".contains(c)), "{l}");
        }
        let mut unique = labels.clone();
        unique.dedup();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), labels.len());
        for place in p.all() {
            assert_eq!(p.by_window(&place.window_label()), Some(place));
        }
        assert_eq!(p.by_window("connect"), None);
    }

    #[test]
    fn titles_name_the_place_once_there_is_more_than_this_mac() {
        let mut p = Places::default();
        assert_eq!(window_title(&Place::ThisMac, &p), "kwant");
        let a = o("https://kwant.jumpin.games");
        p.add_server(a.clone());
        assert_eq!(window_title(&Place::ThisMac, &p), format!("kwant – {THIS_MAC_LABEL}"));
        assert_eq!(window_title(&Place::Server(a.clone()), &p), "kwant – kwant.jumpin.games");
        p.set_this_mac(false);
        assert_eq!(window_title(&Place::Server(a), &p), "kwant – kwant.jumpin.games");
    }
}
