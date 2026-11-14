//! The perch's window and its talk with the places (desktop-island/design.md, build-brief.md
//! Part 2). The rules are in `perch.rs`; this is the Tauri side.
//!
//! - **The window** `perch` is bundled `loading/perch.html`: no decorations, transparent, always
//!   on top, on every Space, never in the taskbar, and never key unless the person clicks into
//!   Quick ask or opens it with the shortcut. On a Mac it floats at the status-bar level around
//!   the notch (or under the menu bar without one); elsewhere it is a tab at the top of the work
//!   area. It is off on Wayland (`perch::available`).
//! - **Its data** comes from the menu's poll (`main.rs` `build_tray`): each place's
//!   `GET /api/glance`, with `/api/away`'s answer when a place is too old for it. Only cleaned
//!   values reach the page, which puts every string in with `textContent`.
//! - **The page talks back** only by navigating to `kwant-island://…`, which is refused as a
//!   navigation and acted on here, and only for things in the latest snapshot (`perch::call_for`).
//!   The label `perch` is in no capability file, so the page can call no command.
//! - **Settings** (`kwant-app://island?…`) come only from This Mac's page in `main`
//!   (`picker::on_app_url`), and `__KWANT_ISLAND__` goes only there (`island_for`).

use crate::perch::{self, GlanceCache, PageState, PerchAsk, PlaceView, Prefs, Shape, Snapshot, Tracker, View};
use crate::places::{self, Place};
use crate::switcher::{self, PerchMode, PerchShortcut};
use crate::{current, nudge, remote, Goto, Shared, DEMO};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Runtime, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// Two shortcut presses this close are one (a key held down repeats).
const TOGGLE_DEBOUNCE: Duration = Duration::from_millis(300);
/// The same item can't be sent again until this long after its last call ended (a double click,
/// or a retry right after a slow failure).
const SAME_ITEM_EVERY: Duration = Duration::from_secs(1);
/// At most this many of the perch's calls in flight at once.
const MAX_IN_FLIGHT: usize = 4;
/// Quick asks remembered for "Open chat".
const ASKED_KEEP: usize = 8;

/// The perch's state, in `Shared`.
pub struct PerchState {
    pub prefs: Prefs,
    /// Whether this computer can show it (not on Wayland).
    pub available: bool,
    /// The page loaded and said `ready`.
    ready: bool,
    snap: Snapshot,
    tracker: Tracker,
    /// What the page reported last: its state and content size (CSS px).
    page: (PageState, f64, f64),
    open_by_key: bool,
    toggles: u64,
    last_toggle: Option<Instant>,
    /// The accelerator registered now.
    shortcut: Option<&'static str>,
    /// The last update the page got (not sent again unchanged).
    last_sent: Option<String>,
    /// Quick asks that went through: (client message id, place index, chat path).
    asked: VecDeque<(String, usize, String)>,
    /// Calls in flight, and when each item was last sent.
    in_flight: usize,
    sent: perch::Busy,
    /// When the person last acted on it (no polling backoff for a while after).
    last_call: Option<Instant>,
}

impl PerchState {
    pub fn new(data_dir: &std::path::Path) -> PerchState {
        let env = |k: &str| std::env::var(k).ok();
        let available = perch::available(
            std::env::consts::OS,
            env("WAYLAND_DISPLAY").as_deref(),
            env("XDG_SESSION_TYPE").as_deref(),
            env("GDK_BACKEND").as_deref(),
        );
        PerchState {
            prefs: Prefs::load(data_dir),
            available,
            ready: false,
            snap: Snapshot::default(),
            tracker: Tracker::default(),
            page: (PageState::Hidden, 0.0, 0.0),
            open_by_key: false,
            toggles: 0,
            last_toggle: None,
            shortcut: None,
            last_sent: None,
            asked: VecDeque::new(),
            in_flight: 0,
            sent: perch::Busy::default(),
            last_call: None,
        }
    }
}

/// Whether the global-shortcut plugin can start here: never on Wayland, and on Linux only with
/// an X display (the plugin's setup fails without one, and that would stop the app).
pub fn shortcut_plugin_ok() -> bool {
    let env = |k: &str| std::env::var(k).ok();
    let available = perch::available(
        std::env::consts::OS,
        env("WAYLAND_DISPLAY").as_deref(),
        env("XDG_SESSION_TYPE").as_deref(),
        env("GDK_BACKEND").as_deref(),
    );
    available && (!cfg!(target_os = "linux") || env("DISPLAY").is_some_and(|d| !d.trim().is_empty()))
}

/// Whether the perch is on (and the Mac pill steps aside). Never in the demo, whose pill CI
/// screenshots.
pub fn on<R: Runtime>(app: &AppHandle<R>) -> bool {
    if DEMO.get().is_some() {
        return false;
    }
    let shared = app.state::<Shared>();
    let st = shared.perch.lock().unwrap();
    st.prefs.on(st.available)
}

/// The Mac pill stays away while the perch shows what needs you.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn pill_off<R: Runtime>(app: &AppHandle<R>) -> bool {
    on(app)
}

// ---------------------------------------------------------------------------------------------
// Settings

/// `__KWANT_ISLAND__` for a page that just loaded in `label`'s window, if it may have it.
pub fn island_for<R: Runtime>(app: &AppHandle<R>, label: &str, url: &Url) -> Option<String> {
    let port = app.state::<Shared>().local.lock().unwrap().port;
    let (prefs, available) = {
        let shared = app.state::<Shared>();
        let st = shared.perch.lock().unwrap();
        (st.prefs, st.available)
    };
    switcher::island_script(label, switcher::is_local_page(url.as_str(), port), prefs.mode, prefs.shortcut, available)
}

/// This Mac's page changed the Perch settings (already checked: `main`, This Mac's own page,
/// once a second). Saves them, tells the page, and applies them.
pub fn set_prefs<R: Runtime>(app: &AppHandle<R>, mode: PerchMode, shortcut: PerchShortcut) {
    let prefs = Prefs { mode, shortcut };
    let data_dir = app.state::<Shared>().data_dir.clone();
    {
        let shared = app.state::<Shared>();
        let mut st = shared.perch.lock().unwrap();
        if st.prefs == prefs {
            return;
        }
        st.prefs = prefs;
        if !prefs.on(st.available) {
            st.open_by_key = false;
        }
    }
    if let Err(e) = prefs.save(&data_dir) {
        tracing::warn!(error = %e, "couldn't save the Perch settings");
    }
    if let Some(main) = app.get_webview_window(crate::MAIN) {
        if let Some(js) = main.url().ok().and_then(|url| island_for(app, crate::MAIN, &url)) {
            let _ = main.eval(js.as_str());
        }
    }
    apply_shortcut(app);
    refresh(app);
    crate::wake(app);
}

/// Registers the shortcut the settings name (and drops the old one). Without the plugin
/// (Wayland, no display, tests) nothing happens.
pub fn apply_shortcut<R: Runtime>(app: &AppHandle<R>) {
    let want = {
        let shared = app.state::<Shared>();
        let st = shared.perch.lock().unwrap();
        if DEMO.get().is_some() {
            None
        } else {
            perch::accelerator(st.prefs.active_shortcut(st.available), cfg!(target_os = "macos"))
        }
    };
    let had = app.state::<Shared>().perch.lock().unwrap().shortcut;
    if had == want {
        return;
    }
    let Some(gs) = app.try_state::<tauri_plugin_global_shortcut::GlobalShortcut<R>>() else {
        return;
    };
    if let Some(old) = had {
        if let Err(e) = gs.unregister(old) {
            tracing::info!(error = %e, "couldn't drop the old Perch shortcut");
        }
    }
    let mut now = None;
    if let Some(accel) = want {
        let registered = gs.on_shortcut(accel, |app, _, event| {
            if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                let app = app.clone();
                std::thread::spawn(move || toggle(&app));
            }
        });
        match registered {
            Ok(()) => now = Some(accel),
            // another app holds it: the perch still works by pointer
            Err(e) => tracing::info!(error = %e, shortcut = accel, "couldn't register the Perch shortcut"),
        }
    }
    app.state::<Shared>().perch.lock().unwrap().shortcut = now;
}

/// The shortcut (or the Window menu's "Open the perch"): opens it expanded with the keyboard,
/// or closes it.
pub fn toggle<R: Runtime>(app: &AppHandle<R>) {
    let open = {
        let shared = app.state::<Shared>();
        let mut st = shared.perch.lock().unwrap();
        if !st.prefs.on(st.available) || DEMO.get().is_some() {
            return;
        }
        let now = Instant::now();
        if st.last_toggle.is_some_and(|t| now.saturating_duration_since(t) < TOGGLE_DEBOUNCE) {
            return;
        }
        st.last_toggle = Some(now);
        st.open_by_key = !(st.open_by_key || st.page.0.open());
        st.toggles += 1;
        st.open_by_key
    };
    refresh(app);
    if let Some(w) = app.get_webview_window(perch::WINDOW) {
        if open {
            let _ = w.set_focusable(true);
            let _ = w.set_focus();
        } else {
            let _ = w.set_focusable(false);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Polling

/// Whether the menu's poll should fetch glances this round.
pub fn polling<R: Runtime>(app: &AppHandle<R>) -> bool {
    on(app)
}

/// How long after an Allow once, Deny, chip or Quick ask every place is asked each round.
const EAGER_AFTER_CALL: Duration = Duration::from_secs(15);

/// No backoff while the perch is open or the person just acted on it.
pub fn eager<R: Runtime>(app: &AppHandle<R>) -> bool {
    let shared = app.state::<Shared>();
    let st = shared.perch.lock().unwrap();
    st.page.0.open() || st.last_call.is_some_and(|t| t.elapsed() < EAGER_AFTER_CALL)
}

/// This Mac's kwant's glance (loopback: no token).
pub fn local_glance(client: &reqwest::Client, port: u16, etag: Option<&str>) -> remote::Glance {
    let Ok(address) = crate::target::parse_address(&format!("http://127.0.0.1:{port}")) else {
        return remote::Glance::Failed;
    };
    tauri::async_runtime::block_on(remote::get_glance(client, &address.origin, None, etag))
}

/// The place's row in the snapshot, from its glance cache.
pub fn view(place: &Place, cache: &GlanceCache) -> PlaceView {
    let (label, this_mac) = match place {
        Place::ThisMac => (places::THIS_MAC_LABEL.to_string(), true),
        Place::Server(origin) => (origin.host(), false),
    };
    PlaceView { key: place.key(), label, this_mac, off: cache.off, current: false, glance: cache.glance.clone() }
}

/// The menu's poll has every place's glance: the perch takes the new snapshot.
pub fn on_poll<R: Runtime>(app: &AppHandle<R>, mut snap: Snapshot) {
    let here = crate::picker::last_place_window(app)
        .and_then(|l| current(app).by_window(&l).map(|p| p.key()));
    for p in &mut snap.places {
        p.current = here.as_deref() == Some(p.key.as_str());
    }
    {
        let shared = app.state::<Shared>();
        let mut st = shared.perch.lock().unwrap();
        st.tracker.update(&snap, Instant::now());
        st.snap = snap;
    }
    refresh(app);
}

/// Brings the window in line with the state: made when it is wanted, hidden when it isn't,
/// and the page told what changed.
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
    if DEMO.get().is_some() {
        return;
    }
    let (wants, update) = {
        let shared = app.state::<Shared>();
        let mut st = shared.perch.lock().unwrap();
        let wants = perch::wants_window(&st.prefs, st.available, &st.snap, st.open_by_key);
        let view = View {
            shape: shape_guess(),
            mac: cfg!(target_os = "macos"),
            prefs: st.prefs,
            open_by_key: st.open_by_key,
            toggles: st.toggles,
        };
        let events = st.tracker.events(&st.snap, Instant::now());
        let json = perch::page_json(&st.snap, &view, &events).to_string();
        let update = (st.ready && wants && st.last_sent.as_deref() != Some(json.as_str())).then(|| {
            st.last_sent = Some(json.clone());
            json
        });
        if !wants {
            // the next time it shows, the page gets everything again
            st.last_sent = None;
        }
        (wants, update)
    };
    if !wants {
        if let Some(w) = app.get_webview_window(perch::WINDOW) {
            let _ = w.hide();
        }
        return;
    }
    let Some(w) = window(app) else {
        return;
    };
    if let Some(json) = update {
        let _ = w.eval(format!("window.kwantPerch&&kwantPerch.update({json})"));
    }
}

/// The shape the page draws before the window is placed: a notch on a Mac (the real one is
/// measured when it is placed), a tab elsewhere.
fn shape_guess() -> Shape {
    #[cfg(all(target_os = "macos", not(test)))]
    if let Some(s) = mac::last_shape() {
        return s;
    }
    Shape::Float
}

// ---------------------------------------------------------------------------------------------
// The window

/// The perch's window (made hidden the first time).
fn window<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    if let Some(w) = app.get_webview_window(perch::WINDOW) {
        return Some(w);
    }
    let nav_app = app.clone();
    let built = WebviewWindowBuilder::new(app, perch::WINDOW, WebviewUrl::App("perch.html".into()))
        .title("Perch")
        .inner_size(perch::OPEN_W + 2.0 * perch::MARGIN, perch::FLOAT_H)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        // never key on its own: it doesn't take the keyboard from the app being typed in
        .focusable(false)
        .focused(false)
        .accept_first_mouse(true)
        .visible(false)
        .on_navigation(move |url| {
            if url.scheme() == "kwant-island" {
                on_perch_url(&nav_app, url);
                return false;
            }
            // only the bundled perch page itself
            crate::links::is_bundled(url)
        })
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .build();
    match built {
        Ok(w) => {
            // never on a test's mock window: its ns_window isn't a real NSWindow
            #[cfg(all(target_os = "macos", not(test)))]
            mac::float_above(&w);
            Some(w)
        }
        Err(e) => {
            tracing::error!(error = %e, "couldn't make the perch");
            None
        }
    }
}

/// The page navigated to `kwant-island://…` (never loaded).
fn on_perch_url<R: Runtime>(app: &AppHandle<R>, url: &Url) {
    let Some(ask) = perch::parse_perch(url.as_str()) else {
        tracing::info!("ignored a perch address the app doesn't know");
        return;
    };
    let app = app.clone();
    // never touch a window inside the webview's own callback
    std::thread::spawn(move || {
        act(&app, ask);
    });
}

/// How a call went, as the page hears it: (op, id, ok, code).
pub type Outcome = (&'static str, String, bool, &'static str);

/// Does what the page asked. A call (Allow once, Deny, a chip, a Quick ask) returns how it went.
fn act<R: Runtime>(app: &AppHandle<R>, ask: PerchAsk) -> Option<Outcome> {
    match ask {
        PerchAsk::Ready => {
            {
                let shared = app.state::<Shared>();
                let mut st = shared.perch.lock().unwrap();
                st.ready = true;
                st.last_sent = None;
            }
            refresh(app);
        }
        PerchAsk::Size { state, w, h } => {
            app.state::<Shared>().perch.lock().unwrap().page = (state, w as f64, h as f64);
            place(app, state, (w as f64, h as f64));
        }
        PerchAsk::Focus => {
            if let Some(w) = app.get_webview_window(perch::WINDOW) {
                let _ = w.set_focusable(true);
                let _ = w.set_focus();
            }
        }
        PerchAsk::Collapse => {
            app.state::<Shared>().perch.lock().unwrap().open_by_key = false;
            if let Some(w) = app.get_webview_window(perch::WINDOW) {
                let _ = w.set_focusable(false);
            }
            refresh(app);
        }
        PerchAsk::Open { .. } | PerchAsk::OpenPage(_) => open(app, &ask),
        PerchAsk::Place { .. } => {
            let key = {
                let shared = app.state::<Shared>();
                let st = shared.perch.lock().unwrap();
                perch::place_target(&st.snap, &ask).map(str::to_string)
            };
            if let Some(place) = key.and_then(|k| crate::place_by_key(app, &k)) {
                collapse(app);
                crate::picker::switch_from_last(app, &place);
            }
        }
        PerchAsk::Places => {
            collapse(app);
            let from = crate::picker::last_place_window(app);
            crate::picker::open(app, from, None);
        }
        PerchAsk::Allow { .. } | PerchAsk::Deny { .. } | PerchAsk::Answer { .. } | PerchAsk::Ask { .. } => {
            return Some(call(app, &ask));
        }
    }
    None
}

/// The perch page's `kwant-island://` address, handled here and now (tests; the window's own
/// handler does the same on a thread).
#[cfg(test)]
pub fn handle<R: Runtime>(app: &AppHandle<R>, url: &Url) -> Option<Outcome> {
    act(app, perch::parse_perch(url.as_str())?)
}

/// Sets what the last poll saw (tests).
#[cfg(test)]
pub fn set_snapshot<R: Runtime>(app: &AppHandle<R>, snap: Snapshot) {
    app.state::<Shared>().perch.lock().unwrap().snap = snap;
}

/// (ready, calls in flight, acted lately, opened by the shortcut) (tests).
#[cfg(test)]
pub fn probe<R: Runtime>(app: &AppHandle<R>) -> (bool, usize, bool, bool) {
    let shared = app.state::<Shared>();
    let st = shared.perch.lock().unwrap();
    (st.ready, st.in_flight, st.last_call.is_some(), st.open_by_key)
}

/// Closes the page (after it sent the person somewhere else).
fn collapse<R: Runtime>(app: &AppHandle<R>) {
    app.state::<Shared>().perch.lock().unwrap().open_by_key = false;
    if let Some(w) = app.get_webview_window(perch::WINDOW) {
        let _ = w.set_focusable(false);
        let _ = w.eval("window.kwantPerch&&kwantPerch.collapse()");
    }
}

/// The perch lost the keyboard: the page closes what the shortcut opened.
pub fn blurred<R: Runtime>(app: &AppHandle<R>) {
    let by_key = std::mem::take(&mut app.state::<Shared>().perch.lock().unwrap().open_by_key);
    if let Some(w) = app.get_webview_window(perch::WINDOW) {
        let _ = w.set_focusable(false);
        if by_key {
            let _ = w.eval("window.kwantPerch&&kwantPerch.collapse()");
        }
    }
}

/// Opens a thing in its place's window, at its page.
fn open<R: Runtime>(app: &AppHandle<R>, ask: &PerchAsk) {
    let target = {
        let shared = app.state::<Shared>();
        let st = shared.perch.lock().unwrap();
        let found = perch::open_target(&st.snap, ask).map(|(p, path)| (st.snap.places[p].key.clone(), path));
        // "Open chat" after a Quick ask
        found.or_else(|| match ask {
            PerchAsk::Open { p, id } => st
                .asked
                .iter()
                .find(|(cid, ap, _)| cid == id && ap == p)
                .and_then(|(_, p, path)| st.snap.places.get(*p).map(|v| (v.key.clone(), path.clone()))),
            _ => None,
        })
    };
    let Some((key, path)) = target else {
        tracing::info!("the perch asked to open something that isn't there any more");
        return;
    };
    collapse(app);
    if let Some(place) = crate::place_by_key(app, &key) {
        crate::open_place(app, &place, Goto::Path(path));
    }
}

/// Allow once / Deny / a chip / a Quick ask, for a thing in the latest snapshot only.
fn call<R: Runtime>(app: &AppHandle<R>, ask: &PerchAsk) -> Outcome {
    let checked = {
        let shared = app.state::<Shared>();
        let mut st = shared.perch.lock().unwrap();
        match perch::call_for(&st.snap, ask) {
            Ok(call) => {
                let key = format!("{}#{}", call.p, call.id);
                if st.in_flight >= MAX_IN_FLIGHT || !st.sent.start(&key, Instant::now(), SAME_ITEM_EVERY) {
                    Err((call.op, call.id, "busy"))
                } else {
                    st.in_flight += 1;
                    st.last_call = Some(Instant::now());
                    let place = st.snap.places.get(call.p).map(|v| v.key.clone());
                    Ok((call, place, key))
                }
            }
            Err(refused) => {
                let (op, id) = op_of(ask);
                Err((op, id, if refused == perch::Refused::NotInline { "not_inline" } else { "gone" }))
            }
        }
    };
    let (call, key, busy_key) = match checked {
        Ok(c) => c,
        Err((op, id, code)) => {
            tracing::info!(op, code, "the perch's ask was refused");
            done(app, op, &id, false, code);
            return (op, id, false, code);
        }
    };
    let posted = key.and_then(|k| crate::place_by_key(app, &k)).map(|place| post(app, &place, &call.path, &call.body));
    let (ok, code) = match &posted {
        Some(remote::Posted::Ok(body)) => {
            if call.op == "ask" {
                if let Some(path) = perch::asked_path(body) {
                    let shared = app.state::<Shared>();
                    let mut st = shared.perch.lock().unwrap();
                    st.asked.push_back((call.id.clone(), call.p, path));
                    while st.asked.len() > ASKED_KEEP {
                        st.asked.pop_front();
                    }
                }
            }
            (true, "ok")
        }
        Some(remote::Posted::Status(409)) => (false, "already"),
        Some(remote::Posted::Status(_)) => (false, "refused"),
        Some(remote::Posted::SignedOut) => (false, "signed_out"),
        Some(remote::Posted::Failed) | None => (false, "failed"),
    };
    {
        let shared = app.state::<Shared>();
        let mut st = shared.perch.lock().unwrap();
        st.in_flight = st.in_flight.saturating_sub(1);
        // busy for a second from now, the call's end, not its start
        st.sent.end(&busy_key, Instant::now());
        st.last_call = Some(Instant::now());
    }
    done(app, call.op, &call.id, ok, code);
    // the next poll shows it answered (or gone)
    crate::wake(app);
    (call.op, call.id, ok, code)
}

fn op_of(ask: &PerchAsk) -> (&'static str, String) {
    match ask {
        PerchAsk::Allow { id, .. } => ("allow", id.clone()),
        PerchAsk::Deny { id, .. } => ("deny", id.clone()),
        PerchAsk::Answer { id, .. } => ("answer", id.clone()),
        PerchAsk::Ask { cid, .. } => ("ask", cid.clone()),
        _ => ("", String::new()),
    }
}

/// Tells the page how a call went. Only our own words and an id the page sent go in.
fn done<R: Runtime>(app: &AppHandle<R>, op: &str, id: &str, ok: bool, code: &str) {
    if let Some(w) = app.get_webview_window(perch::WINDOW) {
        let msg = serde_json::json!({ "op": op, "id": id, "ok": ok, "code": code });
        let _ = w.eval(format!("window.kwantPerch&&kwantPerch.done({msg})"));
    }
}

/// POSTs to a place's kwant: This Mac on loopback (no token), a server with the windows' cookie.
fn post<R: Runtime>(app: &AppHandle<R>, place: &Place, path: &str, body: &serde_json::Value) -> remote::Posted {
    let client = remote::client();
    match place {
        Place::ThisMac => {
            let Some(port) = app.state::<Shared>().local.lock().unwrap().port else {
                return remote::Posted::Failed;
            };
            let Ok(address) = crate::target::parse_address(&format!("http://127.0.0.1:{port}")) else {
                return remote::Posted::Failed;
            };
            tauri::async_runtime::block_on(remote::post_json(&client, &address.origin, None, path, body))
        }
        Place::Server(origin) => {
            let Some(token) = crate::read_token(app, origin) else {
                return remote::Posted::SignedOut;
            };
            tauri::async_runtime::block_on(remote::post_json(&client, origin, Some(&token), path, body))
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Where it hangs

/// Moves and sizes the window for the page's state, and shows or hides it.
fn place<R: Runtime>(app: &AppHandle<R>, state: PageState, content: (f64, f64)) {
    let Some(w) = app.get_webview_window(perch::WINDOW) else {
        return;
    };
    let wants = {
        let shared = app.state::<Shared>();
        let st = shared.perch.lock().unwrap();
        perch::wants_window(&st.prefs, st.available, &st.snap, st.open_by_key)
    };
    if state == PageState::Hidden || !wants {
        let _ = w.hide();
        return;
    }
    let Some(screen) = screen(app, &w) else {
        return;
    };
    let (x, y, fw, fh) = perch::frame(screen.shape, screen.w, screen.notch_left, screen.top, state, content);
    let _ = w.set_size(LogicalSize::new(fw, fh));
    let _ = w.set_position(LogicalPosition::new(screen.x + x, screen.y + y));
    // shown, not focused: the person keeps typing where they were
    let _ = w.show();
}

/// The screen the perch hangs on, in logical points from the primary screen's top-left.
struct Screen {
    shape: Shape,
    /// The screen's (or work area's) left and top edges, and width.
    x: f64,
    y: f64,
    w: f64,
    notch_left: f64,
    /// Where it hangs below `y`.
    top: f64,
}

fn screen<R: Runtime>(app: &AppHandle<R>, perch_window: &WebviewWindow<R>) -> Option<Screen> {
    #[cfg(all(target_os = "macos", not(test)))]
    if let Some(s) = mac::screen(app, perch_window) {
        return Some(s);
    }
    // the work area of the screen the person is on: their last place window's, else the primary
    let monitor = crate::picker::last_place_window(app)
        .and_then(|l| app.get_webview_window(&l))
        .and_then(|w| w.current_monitor().ok().flatten())
        .or_else(|| perch_window.primary_monitor().ok().flatten())?;
    let scale = monitor.scale_factor();
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let area = monitor.work_area();
    Some(Screen {
        shape: Shape::Float,
        x: area.position.x as f64 / scale,
        y: area.position.y as f64 / scale,
        w: area.size.width as f64 / scale,
        notch_left: 0.0,
        top: 0.0,
    })
}

/// AppKit: the notch, the status-bar level, every Space. Left out of test builds: the mock
/// runtime's windows aren't NSWindows, and touching one segfaults the test binary.
#[cfg(all(target_os = "macos", not(test)))]
mod mac {
    use super::Screen;
    use crate::perch::{self, NotchMetrics, Shape};
    use objc2::runtime::NSObjectProtocol;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSScreen, NSWindow, NSWindowCollectionBehavior};
    use std::sync::Mutex;
    use tauri::{AppHandle, Runtime, WebviewWindow};

    /// NSStatusWindowLevel: above normal and floating windows, like the menu bar's own items.
    const STATUS_LEVEL: isize = 25;

    static LAST: Mutex<Option<Shape>> = Mutex::new(None);

    pub fn last_shape() -> Option<Shape> {
        *LAST.lock().unwrap()
    }

    /// Floats the window at the status-bar level, on every Space and over full-screen apps,
    /// out of ⌘` cycling.
    pub fn float_above<R: Runtime>(w: &WebviewWindow<R>) {
        let Ok(ptr) = w.ns_window() else {
            return;
        };
        let ptr = ptr as usize;
        let _ = w.run_on_main_thread(move || {
            if ptr == 0 || MainThreadMarker::new().is_none() {
                return;
            }
            // SAFETY: Tauri gives the window's live NSWindow, used on the main thread only.
            let window: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
            window.setLevel(STATUS_LEVEL);
            window.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::Stationary
                    | NSWindowCollectionBehavior::IgnoresCycle
                    | NSWindowCollectionBehavior::FullScreenAuxiliary,
            );
            window.setHidesOnDeactivate(false);
        });
    }

    /// The main screen's frame, menu bar and notch, read on the main thread.
    pub fn screen<R: Runtime>(app: &AppHandle<R>, w: &WebviewWindow<R>) -> Option<Screen> {
        // AppKit's y runs up from the primary screen's bottom; Tauri's runs down from its top
        let primary = w.primary_monitor().ok().flatten()?;
        let primary_h = primary.size().to_logical::<f64>(primary.scale_factor()).height;
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = app.run_on_main_thread(move || {
            let _ = tx.send(read(primary_h));
        });
        let s = rx.recv_timeout(std::time::Duration::from_secs(2)).ok().flatten()?;
        *LAST.lock().unwrap() = Some(s.shape);
        Some(s)
    }

    fn read(primary_h: f64) -> Option<Screen> {
        let mtm = MainThreadMarker::new()?;
        let screen = NSScreen::mainScreen(mtm)?;
        let f = screen.frame();
        let v = screen.visibleFrame();
        let x = f.origin.x;
        let y = primary_h - (f.origin.y + f.size.height);
        // safeAreaInsets and the auxiliary areas are macOS 12 and later
        let has_notch_api = screen.respondsToSelector(objc2::sel!(safeAreaInsets))
            && screen.respondsToSelector(objc2::sel!(auxiliaryTopLeftArea));
        let notch = has_notch_api
            .then(|| NotchMetrics {
                screen_w: f.size.width,
                inset_top: screen.safeAreaInsets().top,
                left_w: screen.auxiliaryTopLeftArea().size.width,
                right_w: screen.auxiliaryTopRightArea().size.width,
            })
            .and_then(|m| perch::notch(&m));
        Some(match notch {
            Some((left, nw, nh)) => Screen { shape: Shape::Notch { w: nw, h: nh }, x, y, w: f.size.width, notch_left: left, top: 0.0 },
            // no notch: a tab just under the menu bar
            None => {
                let menu_h = (f.origin.y + f.size.height) - (v.origin.y + v.size.height);
                let menu_h = if menu_h.is_finite() { menu_h.clamp(0.0, 64.0) } else { 24.0 };
                Screen { shape: Shape::Float, x, y, w: f.size.width, notch_left: 0.0, top: menu_h }
            }
        })
    }
}

/// The glance caches the menu's poll keeps, one per place key.
pub type Caches = HashMap<String, GlanceCache>;

/// One round of glance polls, given each place's answer (`fetch`), its `/api/away` answer
/// (`away`) and the cache. Returns the snapshot.
pub fn snapshot(
    all: &[Place],
    caches: &mut Caches,
    away: &HashMap<String, nudge::Waiting>,
    eager: bool,
    mut fetch: impl FnMut(&Place, Option<&str>) -> remote::Glance,
) -> Snapshot {
    caches.retain(|k, _| all.iter().any(|p| &p.key() == k));
    let places = all
        .iter()
        .map(|place| {
            let key = place.key();
            let cache = caches.entry(key.clone()).or_default();
            // a place that keeps answering 304 is asked less often (GlanceCache::due)
            if cache.due(eager) {
                let answer = fetch(place, cache.etag.as_deref());
                cache.take(answer, away.get(&key));
            }
            view(place, cache)
        })
        .collect();
    Snapshot { places }
}
