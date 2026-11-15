//! The place picker (bundled `loading/places.html`, design.md 9.2) and switching in place
//! (desktop-island/build-brief.md, Part 1). The rules are in `switcher.rs`; this is the Tauri
//! side.
//!
//! - A place's page asks for the picker with `kwant-app://places?at=x,y` (`route` hands it here):
//!   at most once a second per window, x,y clamped to the window, never loaded.
//! - The picker page talks back only with `kwant-places://…` navigations, naming a row by its
//!   index in the rows Rust gave it. It has no command capability (its label is in no
//!   capability file).
//! - Switching moves the target into the window you were in (its frame, maximized and
//!   full-screen state) and hides the one you left, which stays connected. ⌥↩ opens side by side.
//! - Only This Mac's window, showing This Mac's kwant, learns how much waits elsewhere.

use crate::places::{self, Place};
use crate::switcher::{self, AppAsk, PickerAsk, Plan, Rect};
use crate::{current, is_place_window, menu, open_place, set_local, show, show_connect, wake, Goto, Shared, MAIN};
use std::time::{Duration, Instant};
use tauri::webview::{NewWindowResponse, PageLoadEvent};
use tauri::{AppHandle, LogicalSize, Manager, Runtime, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// A focus change this soon after the picker showed doesn't close it (showing it moves focus).
const BLUR_GRACE: Duration = Duration::from_millis(300);

/// What the picker shows now.
#[derive(Default)]
pub struct PickerState {
    /// The window it was opened from: the "from" of a switch.
    from: Option<String>,
    /// The rows as given to the page, in the page's order.
    rows: Vec<Place>,
    /// The page has loaded and said `ready`.
    ready: bool,
    /// Rows waiting for `ready`.
    pending: Option<String>,
    /// When it last showed; `None` while hidden.
    shown_at: Option<Instant>,
}

/// A place's page navigated to `kwant-app://…` (never loaded). `label`: that page's window.
/// Returns what was accepted (`None`: unknown, or asked again within a second).
pub fn on_app_url<R: Runtime>(app: &AppHandle<R>, label: &str, url: &Url) -> Option<AppAsk> {
    let Some(ask) = switcher::parse_app(url.as_str()) else {
        tracing::info!("ignored a kwant-app address the app doesn't know");
        return None;
    };
    let key = match &ask {
        // Perch settings: only This Mac's window, on its own budget (its page is checked next)
        AppAsk::Island { .. } if label != MAIN => {
            tracing::info!(window = label, "ignored Perch settings from a window that isn't This Mac's");
            return None;
        }
        AppAsk::Island { .. } => switcher::island_key(label),
        AppAsk::Places { .. } => label.to_string(),
    };
    if !app.state::<Shared>().asks.lock().unwrap().allow(&key, Instant::now(), switcher::ASK_EVERY) {
        tracing::debug!(window = label, "ignored a second ask within a second");
        return None;
    }
    match &ask {
        AppAsk::Places { at } => {
            let (app, label, at) = (app.clone(), label.to_string(), *at);
            // never build or show a window inside the webview's own callback
            std::thread::spawn(move || open(&app, Some(label), at));
        }
        AppAsk::Island { mode, shortcut } => {
            let (app, mode, shortcut) = (app.clone(), *mode, *shortcut);
            std::thread::spawn(move || island(&app, mode, shortcut));
        }
    }
    Some(ask)
}

/// Perch settings from `main`: kept only while `main` shows This Mac's own page (a server's
/// page loaded there can't change them).
pub fn island<R: Runtime>(app: &AppHandle<R>, mode: switcher::PerchMode, shortcut: switcher::PerchShortcut) -> bool {
    let port = app.state::<Shared>().local.lock().unwrap().port;
    let local = app
        .get_webview_window(MAIN)
        .and_then(|w| w.url().ok())
        .is_some_and(|url| switcher::is_local_page(url.as_str(), port));
    if !switcher::island_allowed(MAIN, local) {
        tracing::info!("ignored Perch settings from a page that isn't This Mac's");
        return false;
    }
    crate::perch_window::set_prefs(app, mode, shortcut);
    true
}

/// The window the person is in: the one that had the keyboard last, while it shows.
pub fn last_place_window<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let last = app.state::<Shared>().last_focused.lock().unwrap().clone();
    last.filter(|l| shows(app, l))
}

fn shows<R: Runtime>(app: &AppHandle<R>, label: &str) -> bool {
    is_place_window(label) && app.get_webview_window(label).and_then(|w| w.is_visible().ok()).unwrap_or(false)
}

/// The picker's rows and the page's JSON for them.
fn rows<R: Runtime>(app: &AppHandle<R>, from: Option<&str>) -> (Vec<Place>, usize, String) {
    let saved = current(app);
    let status = app.state::<Shared>().status.lock().unwrap().clone();
    let all = saved.all();
    let keys: Vec<String> = all.iter().map(Place::key).collect();
    let here = from.and_then(|l| saved.by_window(l)).map(|p| p.key());
    let mut shown: Vec<Place> = switcher::row_order(&keys, here.as_deref()).into_iter().map(|i| all[i].clone()).collect();
    let mut json: Vec<serde_json::Value> = shown
        .iter()
        .map(|p| {
            let said = status.get(&p.key()).copied().unwrap_or(if *p == Place::ThisMac {
                menu::Status::Starting
            } else {
                menu::Status::Connecting
            });
            let (kind, name) = match p {
                Place::ThisMac => ("mac", places::THIS_MAC_LABEL.to_string()),
                Place::Server(o) => ("server", o.host()),
            };
            serde_json::json!({
                "kind": kind,
                "name": name,
                "sub": said.text(),
                "bad": matches!(said, menu::Status::Unreachable | menu::Status::DidntStart),
                "n": said.count(),
                "cur": here.as_deref() == Some(p.key().as_str()),
            })
        })
        .collect();
    // This Mac's kwant off: its row says so, and picking it starts it (mockup rowsB)
    if !saved.this_mac {
        shown.push(Place::ThisMac);
        json.push(serde_json::json!({ "kind": "mac", "name": places::THIS_MAC_LABEL, "sub": "Not running", "bad": false, "n": 0, "cur": false }));
    }
    let mut actions = vec![serde_json::json!({ "id": "add", "label": "Add a server…", "icon": "plus" })];
    if !saved.this_mac {
        actions.push(serde_json::json!({ "id": "run-here", "label": places::RUN_HERE, "icon": "play" }));
    }
    let n_actions = actions.len();
    let page = serde_json::json!({ "rows": json, "actions": actions, "mac": cfg!(target_os = "macos") });
    (shown, n_actions, page.to_string())
}

/// Opens the picker: under the chip (`at`, in `from`'s CSS px), at `from`'s top centre without
/// one, or at the top centre of the screen when no place's window shows.
pub fn open<R: Runtime>(app: &AppHandle<R>, from: Option<String>, at: Option<(f64, f64)>) {
    let from = from.filter(|l| shows(app, l));
    let (rows, n_actions, page) = rows(app, from.as_deref());
    let Some(picker) = window(app) else {
        return;
    };
    let height = switcher::picker_height(switcher::card_height(rows.len(), n_actions));
    let spot = from.as_deref().and_then(|l| app.get_webview_window(l)).and_then(|w| frame_of(&w, at));
    let spot = spot.or_else(|| screen_of(&picker).map(|s| (s, None, picker.scale_factor().unwrap_or(1.0), s)));
    let ready = {
        let shared = app.state::<Shared>();
        let mut state = shared.picker.lock().unwrap();
        state.from = from;
        state.rows = rows;
        state.shown_at = Some(Instant::now());
        state.pending = (!state.ready).then(|| page.clone());
        state.ready
    };
    let _ = picker.set_size(LogicalSize::new(switcher::PICKER_W + 2.0 * switcher::PICKER_MARGIN, height));
    if let Some((window, at, scale, screen)) = spot {
        let (x, y) = switcher::picker_spot(window, at, scale, height, screen);
        let _ = picker.set_position(tauri::PhysicalPosition::new(x as i32, y as i32));
    }
    if ready {
        let _ = picker.eval(format!("window.kwantPlaces&&kwantPlaces.show({page})"));
    }
    show(&picker);
}

/// The asking window's content area, the chip's point clamped into it, its scale and its screen.
fn frame_of<R: Runtime>(w: &WebviewWindow<R>, at: Option<(f64, f64)>) -> Option<(Rect, Option<(f64, f64)>, f64, Rect)> {
    let pos = w.inner_position().ok()?;
    let size = w.inner_size().ok()?;
    let scale = w.scale_factor().unwrap_or(1.0);
    let window = Rect { x: pos.x as f64, y: pos.y as f64, w: size.width as f64, h: size.height as f64 };
    let css = (window.w / scale, window.h / scale);
    let screen = screen_of(w).unwrap_or(window);
    Some((window, at.map(|a| switcher::clamp_at(a, css)), scale, screen))
}

/// The work area (without the menu bar and the Dock or taskbar) of a window's screen.
fn screen_of<R: Runtime>(w: &WebviewWindow<R>) -> Option<Rect> {
    let monitor = w.current_monitor().ok().flatten().or_else(|| w.primary_monitor().ok().flatten())?;
    let area = monitor.work_area();
    Some(Rect {
        x: area.position.x as f64,
        y: area.position.y as f64,
        w: area.size.width as f64,
        h: area.size.height as f64,
    })
}

/// The picker's window (made hidden the first time).
fn window<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    if let Some(w) = app.get_webview_window(switcher::PICKER_WINDOW) {
        return Some(w);
    }
    let nav_app = app.clone();
    let built = WebviewWindowBuilder::new(app, switcher::PICKER_WINDOW, WebviewUrl::App("places.html".into()))
        .title("Places")
        .inner_size(switcher::PICKER_W + 2.0 * switcher::PICKER_MARGIN, switcher::PICKER_MIN_H)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .on_navigation(move |url| {
            if url.scheme() == "kwant-places" {
                on_picker_url(&nav_app, url);
                return false;
            }
            // only the bundled picker page itself
            crate::links::is_bundled(url)
        })
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .build();
    match built {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::error!(error = %e, "couldn't make the place picker");
            None
        }
    }
}

/// The picker page navigated to `kwant-places://…` (never loaded).
fn on_picker_url<R: Runtime>(app: &AppHandle<R>, url: &Url) {
    let Some(ask) = switcher::parse_picker(url.as_str()) else {
        tracing::info!("ignored a picker address the app doesn't know");
        return;
    };
    let app = app.clone();
    std::thread::spawn(move || act(&app, ask));
}

fn act<R: Runtime>(app: &AppHandle<R>, ask: PickerAsk) {
    let picker = app.get_webview_window(switcher::PICKER_WINDOW);
    match ask {
        PickerAsk::Ready => {
            let pending = {
                let shared = app.state::<Shared>();
                let mut state = shared.picker.lock().unwrap();
                state.ready = true;
                state.pending.take()
            };
            if let (Some(page), Some(w)) = (pending, picker) {
                let _ = w.eval(format!("window.kwantPlaces&&kwantPlaces.show({page})"));
            }
        }
        PickerAsk::Size { h } => {
            if let Some(w) = picker {
                let _ = w.set_size(LogicalSize::new(switcher::PICKER_W + 2.0 * switcher::PICKER_MARGIN, switcher::picker_height(h)));
            }
        }
        PickerAsk::Close => {
            let from = hide(app);
            if let Some(w) = from.and_then(|l| app.get_webview_window(&l)) {
                let _ = w.set_focus();
            }
        }
        PickerAsk::Add => {
            hide(app);
            show_connect(app);
        }
        PickerAsk::RunHere => {
            hide(app);
            run_here(app);
        }
        PickerAsk::Switch { index, side } => {
            let row = app.state::<Shared>().picker.lock().unwrap().rows.get(index).cloned();
            let Some(place) = row else {
                return;
            };
            let from = hide(app);
            if place == Place::ThisMac && !current(app).this_mac {
                run_here(app);
            } else {
                switch_to(app, from, &place, side);
            }
        }
    }
}

fn run_here<R: Runtime>(app: &AppHandle<R>) {
    if let Err(e) = set_local(app, true) {
        tracing::info!(?e, "Run kwant on this Mac from the picker unchanged");
    }
    wake(app);
}

/// Hides the picker; returns the window it was opened from.
fn hide<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let from = {
        let shared = app.state::<Shared>();
        let mut state = shared.picker.lock().unwrap();
        state.shown_at = None;
        state.from.clone()
    };
    if let Some(w) = app.get_webview_window(switcher::PICKER_WINDOW) {
        let _ = w.hide();
    }
    from
}

/// The picker lost the keyboard (a click outside it): it closes, unless it only just showed.
pub fn blurred<R: Runtime>(app: &AppHandle<R>) {
    let shown_at = app.state::<Shared>().picker.lock().unwrap().shown_at;
    if shown_at.is_some_and(|t| t.elapsed() >= BLUR_GRACE) {
        hide(app);
    }
}

/// ⌘1/⌘2, the tray's rows: switch in place from the window the person is in.
pub fn switch_from_last<R: Runtime>(app: &AppHandle<R>, place: &Place) {
    let from = last_place_window(app);
    switch_to(app, from, place, false);
}

/// Goes to `place` from the window `from` (switcher::plan).
pub fn switch_to<R: Runtime>(app: &AppHandle<R>, from: Option<String>, place: &Place, side: bool) {
    let label = place.window_label();
    let from = from.filter(|l| shows(app, l));
    let target_shows = app.get_webview_window(&label).and_then(|w| w.is_visible().ok()).unwrap_or(false);
    match switcher::plan(from.as_deref(), &label, target_shows, side) {
        Plan::Show | Plan::SideBySide => open_place(app, place, Goto::Stay),
        Plan::TakeFrame { from } => take_frame(app, &from, place),
    }
}

/// `place`'s window takes `from`'s frame and shows; `from` hides (it stays connected and keeps
/// its nudges).
fn take_frame<R: Runtime>(app: &AppHandle<R>, from: &str, place: &Place) {
    let Some(old) = app.get_webview_window(from) else {
        open_place(app, place, Goto::Stay);
        return;
    };
    {
        let shared = app.state::<Shared>();
        let mut places = shared.places.lock().unwrap();
        if !places.has(place) {
            return;
        }
        places.set_open(place, true);
    }
    let label = place.window_label();
    if app.get_webview_window(&label).is_none() {
        // made hidden, so it appears straight in the old window's frame
        crate::make_window(app, place, Goto::Stay, false);
    }
    let Some(new) = app.get_webview_window(&label) else {
        return;
    };
    let full = old.is_fullscreen().unwrap_or(false);
    let maximized = old.is_maximized().unwrap_or(false);
    if new.is_fullscreen().unwrap_or(false) && !full {
        let _ = new.set_fullscreen(false);
    }
    if new.is_maximized().unwrap_or(false) && !maximized {
        let _ = new.unmaximize();
    }
    if let (Ok(pos), Ok(size)) = (old.outer_position(), old.inner_size()) {
        let _ = new.set_size(size);
        let _ = new.set_position(pos);
    }
    if maximized {
        let _ = new.maximize();
    }
    if full {
        let _ = new.set_fullscreen(true);
    }
    show(&new);
    *app.state::<Shared>().last_focused.lock().unwrap() = Some(label);
    if full {
        let _ = old.set_fullscreen(false);
    }
    let _ = old.hide();
}

/// The script for a page that just loaded in `label`'s window, if it may have one.
pub fn elsewhere_for<R: Runtime>(app: &AppHandle<R>, label: &str, url: &Url) -> Option<String> {
    let port = app.state::<Shared>().local.lock().unwrap().port;
    let n = app.state::<Shared>().elsewhere.lock().unwrap().unwrap_or(0);
    switcher::elsewhere_script(label, switcher::is_local_page(url.as_str(), port), n)
}

/// A page finished loading in a place's window: This Mac's page gets the count again.
pub fn page_loaded<R: Runtime>(window: &WebviewWindow<R>, url: &Url, event: PageLoadEvent) {
    if !matches!(event, PageLoadEvent::Finished) {
        return;
    }
    if let Some(js) = elsewhere_for(window.app_handle(), window.label(), url) {
        let _ = window.eval(js.as_str());
    }
    // This Mac's Settings page reads the Perch settings (nothing for any other window or page)
    if let Some(js) = crate::perch_window::island_for(window.app_handle(), window.label(), url) {
        let _ = window.eval(js.as_str());
    }
}

/// The menu's poll counted what waits in the places other than This Mac: This Mac's page hears
/// of a change.
pub fn set_elsewhere<R: Runtime>(app: &AppHandle<R>, n: usize) {
    {
        let shared = app.state::<Shared>();
        let mut now = shared.elsewhere.lock().unwrap();
        if *now == Some(n) {
            return;
        }
        *now = Some(n);
    }
    let Some(main) = app.get_webview_window(MAIN) else {
        return;
    };
    let Ok(url) = main.url() else {
        return;
    };
    if let Some(js) = elsewhere_for(app, MAIN, &url) {
        let _ = main.eval(js.as_str());
    }
}
