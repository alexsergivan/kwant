//! The menu-bar (tray) menu as data: a pure function from the places and what each says to the
//! menu's items (spec P4, Mira's design/menu.html). main.rs turns the entries into Tauri menu
//! items and only rebuilds the menu when its shape changes.

use crate::places::{Place, Places, THIS_MAC_LABEL};

/// What one place's row says after the "·".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// This Mac: kwant hasn't answered yet.
    Starting,
    /// This Mac: kwant didn't start (the window says why).
    DidntStart,
    /// The count from that place's kwant.
    Waiting(usize),
    /// A server the app hasn't heard back from yet.
    Connecting,
    /// A server that answers but not to this app's cookie (none yet, or signed out).
    SignIn,
    /// A server that didn't answer.
    Unreachable,
}

impl Status {
    pub fn text(self) -> String {
        match self {
            Status::Starting => "Starting…".into(),
            Status::DidntStart => "Didn't start".into(),
            Status::Waiting(0) => "Nothing waiting".into(),
            Status::Waiting(n) => format!("{n} waiting"),
            Status::Connecting => "Connecting…".into(),
            Status::SignIn => "Sign in to see what's waiting".into(),
            Status::Unreachable => "Can't reach it".into(),
        }
    }

    pub fn count(self) -> usize {
        match self {
            Status::Waiting(n) => n,
            _ => 0,
        }
    }
}

/// One menu item. Ids: `place:<key>`, `remove:<origin>`, `open`, `status`, `add`, `this_mac`,
/// `nudges`, `update`, `quit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Item { id: String, text: String, enabled: bool },
    Check { id: String, text: String, checked: bool },
    Submenu { text: String, items: Vec<(String, String)> },
    Separator,
}

impl Entry {
    fn item(id: &str, text: impl Into<String>, enabled: bool) -> Entry {
        Entry::Item { id: id.into(), text: text.into(), enabled }
    }

    /// What must match for an existing menu to be updated in place (texts and ticks change; ids
    /// and kinds don't).
    pub fn shape(&self) -> String {
        match self {
            Entry::Item { id, .. } => format!("i:{id}"),
            Entry::Check { id, .. } => format!("c:{id}"),
            Entry::Submenu { items, .. } => {
                let ids: Vec<&str> = items.iter().map(|(id, _)| id.as_str()).collect();
                format!("s:{}", ids.join(","))
            }
            Entry::Separator => "-".into(),
        }
    }
}

/// "Run kwant on this Mac" (this computer on Linux).
pub fn run_here_text() -> String {
    crate::places::RUN_HERE.to_string()
}

/// A long host is cut in the middle so the row stays readable: "kwant-on-a-very-long…example.com".
pub fn short_host(host: &str) -> String {
    let chars: Vec<char> = host.chars().collect();
    if chars.len() <= 32 {
        return host.to_string();
    }
    let head: String = chars[..20].iter().collect();
    let tail: String = chars[chars.len() - 11..].iter().collect();
    format!("{head}…{tail}")
}

/// A place's row: "This Mac · 2 waiting", "kwant.jumpin.games · Can't reach it".
pub fn row(place: &Place, status: Status) -> String {
    let label = match place {
        Place::ThisMac => THIS_MAC_LABEL.to_string(),
        Place::Server(o) => short_host(&o.host()),
    };
    format!("{label} · {}", status.text())
}

/// Everything the menu shows.
pub struct Input<'a> {
    pub places: &'a Places,
    /// What each running place says, by `Place::key`; a place not in it is still starting or
    /// connecting.
    pub status: &'a dyn Fn(&Place) -> Status,
    /// With no servers: today's status line ("Running on port 8799", "2 approvals waiting for
    /// you", "Starting…").
    pub local_line: String,
    pub show_nudges: bool,
    /// "Update to x.y.z…" and whether it can be clicked.
    pub update: Option<(String, bool)>,
}

pub fn model(input: &Input) -> Vec<Entry> {
    let places = input.places;
    let mut out = Vec::new();
    if let Some((text, enabled)) = &input.update {
        // with a server listed, "Update" alone could read as updating the server
        let text = if places.servers.is_empty() { text.clone() } else { text.replacen("Update to ", "Update this app to ", 1) };
        out.push(Entry::item("update", text, *enabled));
        out.push(Entry::Separator);
    }
    let nudges = Entry::Check { id: "nudges".into(), text: "Show nudges".into(), checked: input.show_nudges };
    let add = Entry::item("add", "Add a server…", true);
    let quit = Entry::item("quit", "Quit kwant", true);
    if places.servers.is_empty() && places.this_mac {
        // A: today's Local menu, with "Add a server…"
        out.extend([
            Entry::item("open", "Open kwant", true),
            Entry::item("status", input.local_line.clone(), false),
            Entry::Separator,
            nudges,
            Entry::Separator,
            add,
            Entry::Separator,
            quit,
        ]);
        return out;
    }
    for place in places.all() {
        out.push(Entry::item(&format!("place:{}", place.key()), row(&place, (input.status)(&place)), true));
    }
    if !out.is_empty() && !matches!(out.last(), Some(Entry::Separator)) {
        out.push(Entry::Separator);
    }
    out.push(add);
    if !places.servers.is_empty() {
        out.push(Entry::Submenu {
            text: "Remove a server".into(),
            items: places.servers.iter().map(|o| (format!("remove:{}", o.as_str()), format!("{}…", short_host(&o.host())))).collect(),
        });
    }
    out.push(Entry::Check { id: "this_mac".into(), text: run_here_text(), checked: places.this_mac });
    out.push(nudges);
    out.push(Entry::Separator);
    out.push(quit);
    out
}

/// The number by the icon (and on the Dock): what waits in every place together.
pub fn badge(places: &Places, status: &dyn Fn(&Place) -> Status) -> Option<String> {
    let n: usize = places.all().iter().map(|p| status(p).count()).sum();
    (n > 0).then(|| n.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::parse_address;

    fn o(s: &str) -> crate::target::Origin {
        parse_address(s).unwrap().origin
    }

    fn texts(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match e {
                Entry::Item { text, enabled: true, .. } => text.clone(),
                Entry::Item { text, enabled: false, .. } => format!("({text})"),
                Entry::Check { text, checked, .. } => format!("{text}{}", if *checked { " ✓" } else { "" }),
                Entry::Submenu { text, items } => {
                    format!("{text} ▸ {}", items.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(" | "))
                }
                Entry::Separator => "—".into(),
            })
            .collect()
    }

    fn input<'a>(places: &'a Places, status: &'a dyn Fn(&Place) -> Status) -> Input<'a> {
        Input { places, status, local_line: "2 approvals waiting for you".into(), show_nudges: true, update: None }
    }

    #[test]
    fn with_no_servers_it_is_today_s_local_menu_with_add_a_server() {
        let p = Places::default();
        let st = |_: &Place| Status::Waiting(2);
        assert_eq!(
            texts(&model(&input(&p, &st))),
            ["Open kwant", "(2 approvals waiting for you)", "—", "Show nudges ✓", "—", "Add a server…", "—", "Quit kwant"]
        );
        assert_eq!(badge(&p, &st), Some("2".into()));
    }

    #[test]
    fn this_mac_and_a_server_list_places_first() {
        let mut p = Places::default();
        let a = o("https://kwant.jumpin.games");
        p.add_server(a.clone());
        let st = |pl: &Place| match pl {
            Place::ThisMac => Status::Waiting(2),
            Place::Server(_) => Status::Waiting(5),
        };
        let m = model(&input(&p, &st));
        let here = THIS_MAC_LABEL;
        assert_eq!(
            texts(&m),
            [
                format!("{here} · 2 waiting"),
                "kwant.jumpin.games · 5 waiting".into(),
                "—".into(),
                "Add a server…".into(),
                "Remove a server ▸ kwant.jumpin.games…".into(),
                format!("{} ✓", run_here_text()),
                "Show nudges ✓".into(),
                "—".into(),
                "Quit kwant".into(),
            ]
        );
        assert_eq!(badge(&p, &st), Some("7".into()));
        let ids: Vec<String> = m.iter().map(Entry::shape).collect();
        assert_eq!(ids[0], "i:place:this_mac");
        assert_eq!(ids[1], "i:place:https://kwant.jumpin.games");
        assert_eq!(ids[4], "s:remove:https://kwant.jumpin.games");
    }

    #[test]
    fn every_row_state_reads_as_mira_wrote_it() {
        let a = Place::Server(o("https://kwant.jumpin.games"));
        assert_eq!(row(&Place::ThisMac, Status::Starting), format!("{THIS_MAC_LABEL} · Starting…"));
        assert_eq!(row(&Place::ThisMac, Status::DidntStart), format!("{THIS_MAC_LABEL} · Didn't start"));
        assert_eq!(row(&Place::ThisMac, Status::Waiting(0)), format!("{THIS_MAC_LABEL} · Nothing waiting"));
        assert_eq!(row(&a, Status::Connecting), "kwant.jumpin.games · Connecting…");
        assert_eq!(row(&a, Status::Waiting(1)), "kwant.jumpin.games · 1 waiting");
        assert_eq!(row(&a, Status::SignIn), "kwant.jumpin.games · Sign in to see what's waiting");
        assert_eq!(row(&a, Status::Unreachable), "kwant.jumpin.games · Can't reach it");
        let long = Place::Server(o("https://kwant-on-a-very-long-name.example.com"));
        assert_eq!(row(&long, Status::Waiting(3)), "kwant-on-a-very-long…example.com · 3 waiting");
        assert_eq!(short_host("kwant.jumpin.games"), "kwant.jumpin.games");
    }

    #[test]
    fn only_a_server_has_no_this_mac_row_and_an_unticked_run_here() {
        let mut p = Places::default();
        p.add_server(o("https://kwant.jumpin.games"));
        p.add_server(o("https://vps2.example.com"));
        p.set_this_mac(false);
        let st = |_: &Place| Status::Unreachable;
        let m = model(&input(&p, &st));
        assert_eq!(
            texts(&m),
            [
                "kwant.jumpin.games · Can't reach it".to_string(),
                "vps2.example.com · Can't reach it".into(),
                "—".into(),
                "Add a server…".into(),
                "Remove a server ▸ kwant.jumpin.games… | vps2.example.com…".into(),
                run_here_text(),
                "Show nudges ✓".into(),
                "—".into(),
                "Quit kwant".into(),
            ]
        );
        assert_eq!(badge(&p, &st), None);
    }

    #[test]
    fn with_a_server_listed_the_update_says_this_app() {
        let mut p = Places::default();
        let st = |_: &Place| Status::Starting;
        let mut i = input(&p, &st);
        i.update = Some(("Update to 0.7.6…".into(), true));
        assert_eq!(texts(&model(&i))[..2], ["Update to 0.7.6…".to_string(), "—".into()]);
        p.add_server(o("https://kwant.jumpin.games"));
        let mut i = input(&p, &st);
        i.update = Some(("Update to 0.7.6…".into(), false));
        let m = model(&i);
        assert_eq!(m[0], Entry::Item { id: "update".into(), text: "Update this app to 0.7.6…".into(), enabled: false });
        assert_eq!(texts(&m)[2], format!("{THIS_MAC_LABEL} · Starting…"));
    }

    #[test]
    fn a_count_change_keeps_the_shape_and_a_new_server_changes_it() {
        let mut p = Places::default();
        p.add_server(o("https://kwant.jumpin.games"));
        let shape = |m: &[Entry]| m.iter().map(Entry::shape).collect::<Vec<_>>();
        let one = |_: &Place| Status::Waiting(1);
        let two = |_: &Place| Status::SignIn;
        let before = model(&input(&p, &one));
        assert_eq!(shape(&before), shape(&model(&input(&p, &two))));
        assert_ne!(before, model(&input(&p, &two)));
        let mut more = p.clone();
        more.add_server(o("https://vps2.example.com"));
        assert_ne!(shape(&before), shape(&model(&input(&more, &one))));
        let mut off = p.clone();
        off.set_this_mac(false);
        assert_ne!(shape(&before), shape(&model(&input(&off, &one))));
    }

    #[test]
    fn the_run_here_item_names_this_machine() {
        #[cfg(target_os = "macos")]
        assert_eq!(run_here_text(), "Run kwant on this Mac");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(run_here_text(), "Run kwant on this computer");
    }
}
