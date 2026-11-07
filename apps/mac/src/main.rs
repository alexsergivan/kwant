//! kwant for Mac (and Linux): every place in its own window, all at once (spec.md in
//! next-075/side-by-side). A place is the kwant on this Mac (run in this process, or one already
//! running here) or a kwant server the person added. A menu-bar (tray) icon lists them; closing a
//! window keeps its place running, and Quit in the menu stops the app. Nothing relaunches.

mod connect;
mod links;
mod menu;
mod nudge;
#[cfg(target_os = "linux")]
mod nudge_linux;
#[cfg(target_os = "macos")]
mod nudge_mac;
mod places;
mod redact;
mod remote;
mod startup;
mod target;
mod update;

#[cfg(target_os = "linux")]
use nudge_linux as platform;
#[cfg(target_os = "macos")]
use nudge_mac as platform;
use places::{Place, Places};
use startup::PortChoice;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use target::{Home, Origin};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{TrayIcon, TrayIconBuilder},
    webview::NewWindowResponse,
    AppHandle, Manager, RunEvent, Runtime, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent, Wry,
};

/// This Mac's window.
const MAIN: &str = "main";
/// The "Add a server" window.
const CONNECT: &str = "connect";

/// Tells kwant's page it runs in the app, so sign-ins say "finish in your browser". The page
/// knows these marks (frontend/src/externalLinks.ts DESKTOP_APPS).
#[cfg(target_os = "macos")]
const APP_MARK: &str = "window.__KWANT_APP__ = 'mac';";
#[cfg(not(target_os = "macos"))]
const APP_MARK: &str = "window.__KWANT_APP__ = 'linux';";

/// The menu icon. macOS tints a black template icon for the menu bar; Linux panels show the
/// picture as it is, so they get the coloured app icon (a black glyph vanishes on dark panels).
#[cfg(target_os = "macos")]
const TRAY_ICON: &[u8] = include_bytes!("../icons/tray-template.png");
#[cfg(not(target_os = "macos"))]
const TRAY_ICON: &[u8] = include_bytes!("../icons/32x32.png");

/// The shell asked for the login PATH when `$SHELL` is unset.
#[cfg(target_os = "macos")]
const FALLBACK_SHELL: &str = "/bin/zsh";
#[cfg(not(target_os = "macos"))]
const FALLBACK_SHELL: &str = "/bin/bash";

/// The app's own log in the data dir.
#[cfg(target_os = "macos")]
const LOG_FILE: &str = "mac-app.log";
#[cfg(not(target_os = "macos"))]
const LOG_FILE: &str = "desktop-app.log";

/// The words of the one Remove confirm (Mira's design.md §4.1), for the tray and the pages.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoveWords {
    title: String,
    message: String,
    /// The confirm already says the server can't be reached (so no second message after).
    says_unreachable: bool,
}

fn remove_words(host: &str, unreachable: bool, last_place: bool, mac: bool) -> RemoveWords {
    let mut message = format!(
        "Its window closes and this app signs out of it. The bots on {host} keep running.\n\n\
         To use it here again, choose Add a server… and sign in."
    );
    if unreachable {
        message.push_str(
            "\n\nIt can't be reached right now, so it will still list this app under Settings → \
             Signed-in devices. You can remove it there later.",
        );
    }
    if last_place {
        let here = if mac { "this Mac" } else { "this computer" };
        message.push_str(&format!(
            "\n\n“Run kwant on {here}” turns back on, so the app still has a place to open."
        ));
    }
    RemoveWords { title: format!("Remove {host}?"), message, says_unreachable: unreachable }
}

/// After a Remove the server didn't confirm, when the confirm didn't already say so.
fn removed_words(host: &str) -> (String, &'static str) {
    (
        format!("Removed {host}"),
        "It couldn't be reached, so it still lists this app under Settings → Signed-in devices. \
         You can remove it there later.",
    )
}

/// Shows the native dialogs. Tests answer with a fixed choice and keep what was said.
enum Asker {
    Native,
    #[cfg(test)]
    Fixed { answer: bool, said: Mutex<Vec<(String, String)>> },
}

impl Asker {
    /// Remove / Cancel. Blocks: never on the main thread.
    fn confirm<R: Runtime>(&self, app: &AppHandle<R>, title: &str, message: &str) -> bool {
        match self {
            Asker::Native => {
                use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
                app.dialog()
                    .message(message)
                    .title(title)
                    .kind(MessageDialogKind::Warning)
                    .buttons(MessageDialogButtons::OkCancelCustom("Remove".into(), "Cancel".into()))
                    .blocking_show()
            }
            #[cfg(test)]
            Asker::Fixed { answer, said } => {
                said.lock().unwrap().push((title.into(), message.into()));
                *answer
            }
        }
    }

    /// One OK. Blocks: never on the main thread.
    fn tell<R: Runtime>(&self, app: &AppHandle<R>, title: &str, message: &str) {
        match self {
            Asker::Native => {
                use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
                app.dialog()
                    .message(message)
                    .title(title)
                    .kind(MessageDialogKind::Info)
                    .buttons(MessageDialogButtons::OkCustom("OK".into()))
                    .blocking_show();
            }
            #[cfg(test)]
            Asker::Fixed { said, .. } => said.lock().unwrap().push((title.into(), message.into())),
        }
    }
}

/// What one Remove did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Removed {
    cancelled: bool,
    removed: bool,
    revoked: bool,
}

/// The running server, when this app started one (not when it attached to another kwant).
struct Server {
    stop: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    done: Mutex<Option<mpsc::Receiver<()>>>,
}

impl Server {
    /// Asks kwant to stop and waits for it (it gives Laya 5 s) before returning.
    fn shutdown(&self) {
        let stop = self.stop.lock().unwrap().take();
        if let Some(stop) = stop {
            let _ = stop.send(());
        }
        let done = self.done.lock().unwrap().take();
        if let Some(done) = done {
            let _ = done.recv_timeout(Duration::from_secs(8));
        }
    }
}

/// The kwant on this Mac while "Run kwant on this Mac" is on.
#[derive(Default)]
struct Local {
    /// Where it listens; `None` while off or before it started.
    port: Option<u16>,
    /// Set when this app started it (it stops with the app or the toggle); an attached kwant
    /// (`kwnt serve` in a terminal, a service) keeps running.
    server: Option<Server>,
    /// It didn't answer within 2 minutes, or couldn't start.
    failed: bool,
}

/// How far a server's window got with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    Connecting,
    Connected,
    Unreachable,
}

impl Reach {
    fn as_str(self) -> &'static str {
        match self {
            Reach::Connecting => "connecting",
            Reach::Connected => "connected",
            Reach::Unreachable => "unreachable",
        }
    }
}

/// Everything the windows, the menu and the commands share. Locks are held only to read or
/// write a value, never across a call into a window: those wait for the main thread, which may
/// itself be waiting for the same lock in a navigation check.
struct Shared {
    data_dir: PathBuf,
    /// kwant's settings, used each time this Mac's kwant starts.
    config: kwnt::Config,
    places: Mutex<Places>,
    local: Mutex<Local>,
    /// Held while this Mac's kwant starts, so two windows never start two.
    starting: Mutex<()>,
    /// A shutdown still running after "Run kwant on this Mac" went off.
    stopping: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// This app started kwant here at some point (it then offers its own updates).
    started_kwant: AtomicBool,
    reach: Mutex<HashMap<String, Reach>>,
    /// Servers the menu's last poll couldn't reach (the row says Can't reach it).
    unreachable: Mutex<std::collections::HashSet<String>>,
    /// The place window that had the keyboard last (the Dock click shows it).
    last_focused: Mutex<Option<String>>,
    /// Wakes the menu's poll after a change.
    wake: Mutex<Option<mpsc::Sender<()>>>,
}

impl Shared {
    fn new(data_dir: PathBuf, config: kwnt::Config, places: Places) -> Shared {
        Shared {
            data_dir,
            config,
            places: Mutex::new(places),
            local: Mutex::new(Local::default()),
            starting: Mutex::new(()),
            stopping: Mutex::new(None),
            started_kwant: AtomicBool::new(false),
            reach: Mutex::new(HashMap::new()),
            unreachable: Mutex::new(Default::default()),
            last_focused: Mutex::new(None),
            wake: Mutex::new(None),
        }
    }
}

/// "Show nudges" in the tray menu, saved in the data dir.
struct Nudges {
    data_dir: PathBuf,
    show: AtomicBool,
}

/// Where a window goes once its place answers.
#[derive(Debug, Clone)]
enum Goto {
    /// The page it shows (kwant's home for a new window).
    Stay,
    /// One of kwant's pages, e.g. a nudge's item.
    Path(String),
    /// An address on the place itself: a link from another window, a sign-in link.
    Url(Url),
}

/// Set (to its start) when the app runs as `--demo-nudge`.
static DEMO: OnceLock<Instant> = OnceLock::new();

/// The demo's last step plays as under Reduce Motion, which CI can't switch on.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn demo_reduce_motion() -> bool {
    DEMO.get().is_some_and(|t| nudge::demo::step(t.elapsed()) >= nudge::demo::REDUCE_MOTION_STEP)
}

fn main() {
    fix_path();
    if std::env::args().any(|a| a == "--check") {
        std::process::exit(check());
    }

    let demo = std::env::args().any(|a| a == "--demo-nudge");
    let (config, places) = if demo { demo_start() } else { normal_start() };
    let data_dir = config.data_dir.clone();
    let show = nudge::Prefs::load(&data_dir).show_nudges;

    let builder = tauri::Builder::default();
    // A second launch from the app menu shows a window instead of opening another kwant.
    // (macOS does this itself: a second click on the app reaches RunEvent::Reopen.)
    #[cfg(not(target_os = "macos"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_last(app)));
    let app = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .manage(Shared::new(data_dir.clone(), config, places))
        .manage(Nudges { data_dir, show: AtomicBool::new(show) })
        .manage(Asker::Native)
        .invoke_handler(tauri::generate_handler![
            connect::places_state,
            connect::check_server,
            connect::add_server,
            connect::remove_server,
            connect::set_this_mac,
            connect::close_connect,
            connect::retry_start,
        ])
        .setup(move |app| {
            let handle = app.handle();
            // This Mac's kwant runs while the toggle is on, window or not
            if current(handle).this_mac {
                let h = handle.clone();
                std::thread::spawn(move || {
                    ensure_local(&h);
                    wake(&h);
                });
            }
            build_tray(handle)?;
            if demo {
                // hidden, so the pill shows (it never covers a focused kwant window); a click on
                // the pill shows it
                make_window(handle, &Place::ThisMac, Goto::Stay, false);
                let h = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(nudge::demo::RUNS_FOR);
                    println!("demo-nudge: done");
                    h.exit(0);
                });
            } else {
                let places = current(handle);
                let to_open = places.to_open();
                if to_open.is_empty() {
                    show_connect(handle);
                }
                for place in to_open {
                    make_window(handle, &place, Goto::Stay, true);
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // Closing a place's window keeps the place running; the Add a server window closes
            WindowEvent::CloseRequested { api, .. } if window.label() != CONNECT => {
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::Focused(true) if is_place_window(window.label()) => {
                *window.state::<Shared>().last_focused.lock().unwrap() = Some(window.label().to_string());
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .unwrap_or_else(|e| fatal(&format!("kwant couldn't open its window: {e}")));

    app.run(|handle, event| match event {
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => show_last(handle),
        RunEvent::ExitRequested { .. } => save_open(handle),
        RunEvent::Exit => {
            stop_server(handle);
            if DEMO.get().is_some() {
                let _ = std::fs::remove_dir_all(&handle.state::<Nudges>().data_dir);
            }
        }
        _ => {}
    });
}

/// The person's settings and places. kwant itself starts in `ensure_local`.
fn normal_start() -> (kwnt::Config, Places) {
    let mut config = match kwnt::Config::load() {
        Ok(c) => c,
        Err(e) => fatal(&format!("kwant couldn't read its settings: {e}")),
    };
    init_logging(&config.data_dir);
    // The app is a local window: never listen beyond this machine, whatever config.toml says
    config.host = "127.0.0.1".to_string();
    let mut places = Places::load(&config.data_dir);
    if places.all().is_empty() {
        // nothing to show at all: This Mac comes back on
        places.set_this_mac(true);
    }
    tracing::info!(this_mac = places.this_mac, servers = places.servers.len(), "starting");
    (config, places)
}

/// `--demo-nudge`: kwant on a free port with a throwaway data dir, and made-up waiting items
/// (`nudge::demo`) instead of the real ones, so CI can screenshot the nudges. It prints each
/// step and where the icon and the pill are, and quits after `nudge::demo::RUNS_FOR`.
fn demo_start() -> (kwnt::Config, Places) {
    let _ = DEMO.set(Instant::now());
    tracing_subscriber::fmt().with_env_filter("info").init();
    let data_dir = std::env::temp_dir().join(format!("kwant-demo-nudge-{}", std::process::id()));
    let port = startup::free_port().unwrap_or_else(|e| fatal(&format!("kwant --demo-nudge: no free port: {e}")));
    println!("demo-nudge: port {port}");
    let config = kwnt::Config {
        data_dir,
        host: "127.0.0.1".to_string(),
        port,
        laya_managed: false,
        ..Default::default()
    };
    (config, Places::default())
}

/// Finder (and a Linux app menu) starts apps with a bare PATH, so the claude, codex and grok CLIs
/// would be missing. Runs before any thread exists, so setting the variable is safe.
fn fix_path() {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| FALLBACK_SHELL.to_string());
    let login = startup::login_shell_path(&shell, Duration::from_secs(3));
    let current = std::env::var("PATH").unwrap_or_default();
    let home = dirs::home_dir().unwrap_or_default();
    std::env::set_var("PATH", startup::merged_path(login.as_deref(), &current, &home));
}

/// Starts kwant on its own runtime thread; the returned handle stops it.
fn start_server(config: kwnt::Config) -> Server {
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("kwant-server".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            let result = runtime.block_on(kwnt::serve::run(config, async {
                let _ = stop_rx.await;
            }));
            if let Err(e) = result {
                tracing::error!(error = %e, "kwant stopped");
            }
            let _ = done_tx.send(());
        })
        .expect("server thread");
    Server {
        stop: Mutex::new(Some(stop_tx)),
        done: Mutex::new(Some(done_rx)),
    }
}

/// The kwant on this Mac: started by this app, or the one already running here.
fn start_local(config: &kwnt::Config) -> Local {
    if DEMO.get().is_some() {
        return Local { port: Some(config.port), server: Some(start_server(config.clone())), failed: false };
    }
    match startup::choose_port(config.port) {
        Ok(PortChoice::Attach(port)) => {
            tracing::info!(port, "kwant already runs here; attaching to it");
            Local { port: Some(port), server: None, failed: false }
        }
        Ok(PortChoice::Serve(port)) => {
            let mut config = config.clone();
            config.port = port;
            Local { port: Some(port), server: Some(start_server(config)), failed: false }
        }
        Err(e) => {
            tracing::error!(error = %e, "kwant couldn't find a free port");
            Local { port: None, server: None, failed: true }
        }
    }
}

/// This Mac's port, starting kwant first when the toggle is on and it isn't running. Never on
/// the main thread: it may wait for a shutdown.
fn ensure_local<R: Runtime>(app: &AppHandle<R>) -> Option<u16> {
    let shared = app.state::<Shared>();
    let _starting = shared.starting.lock().unwrap();
    if let Some(port) = shared.local.lock().unwrap().port {
        return Some(port);
    }
    if !shared.places.lock().unwrap().this_mac {
        return None;
    }
    // a kwant still stopping holds the port and the database
    let stopping = shared.stopping.lock().unwrap().take();
    if let Some(stopping) = stopping {
        let _ = stopping.join();
    }
    let local = start_local(&shared.config);
    if local.server.is_some() {
        shared.started_kwant.store(true, Ordering::Relaxed);
    }
    let port = local.port;
    *shared.local.lock().unwrap() = local;
    port
}

/// Stops the kwant this app started before the app exits. An attached kwant keeps running.
fn stop_server<R: Runtime>(handle: &AppHandle<R>) {
    let Some(shared) = handle.try_state::<Shared>() else { return };
    let server = shared.local.lock().unwrap().server.take();
    if let Some(server) = server {
        server.shutdown();
    }
    let stopping = shared.stopping.lock().unwrap().take();
    if let Some(stopping) = stopping {
        let _ = stopping.join();
    }
}

/// A copy of the places (so no lock is held while windows are touched).
fn current<R: Runtime>(app: &AppHandle<R>) -> Places {
    app.state::<Shared>().places.lock().unwrap().clone()
}

fn place_by_key<R: Runtime>(app: &AppHandle<R>, key: &str) -> Option<Place> {
    current(app).all().into_iter().find(|p| p.key() == key)
}

fn is_place_window(label: &str) -> bool {
    label == MAIN || label.starts_with(places::SERVER_WINDOW_PREFIX)
}

/// Where a place's kwant answers (`None`: This Mac while its kwant isn't running).
fn home<R: Runtime>(app: &AppHandle<R>, place: &Place) -> Option<Home> {
    match place {
        Place::ThisMac => app.state::<Shared>().local.lock().unwrap().port.map(|port| Home::Local { port }),
        Place::Server(origin) => Some(Home::Remote { origin: origin.clone() }),
    }
}

fn set_reach<R: Runtime>(app: &AppHandle<R>, origin: &Origin, reach: Reach) {
    app.state::<Shared>().reach.lock().unwrap().insert(origin.as_str(), reach);
    wake(app);
}

/// Has the menu look again now.
fn wake<R: Runtime>(app: &AppHandle<R>) {
    if let Some(tx) = app.state::<Shared>().wake.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
}

/// Shows a place's window (creating it when needed), at `goto`.
fn open_place<R: Runtime>(app: &AppHandle<R>, place: &Place, goto: Goto) {
    {
        let shared = app.state::<Shared>();
        let mut places = shared.places.lock().unwrap();
        if !places.has(place) {
            return;
        }
        places.set_open(place, true);
    }
    let Some(window) = app.get_webview_window(&place.window_label()) else {
        make_window(app, place, goto, true);
        return;
    };
    let url = match goto {
        Goto::Stay => None,
        Goto::Path(path) => home(app, place).and_then(|h| h.page(&path)),
        Goto::Url(url) => Some(url),
    };
    if let Some(url) = url {
        let _ = window.navigate(url);
    }
    show(&window);
}

/// A nudge's item (`path`) in its place's window (`key`: `places::Place::key`). A place gone
/// meanwhile falls back to the first one.
fn open_place_at(handle: &AppHandle, key: &str, path: Option<&str>) {
    let Some(place) = place_by_key(handle, key).or_else(|| current(handle).all().into_iter().next()) else {
        return;
    };
    open_place(handle, &place, path.map_or(Goto::Stay, |p| Goto::Path(p.to_string())));
}

fn show<R: Runtime>(window: &WebviewWindow<R>) {
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

/// The Dock click (and a second launch on Linux): the window used last, else the first place's,
/// else the Add a server window.
fn show_last<R: Runtime>(app: &AppHandle<R>) {
    let last = app.state::<Shared>().last_focused.lock().unwrap().clone();
    if let Some(window) = last.and_then(|l| app.get_webview_window(&l)) {
        show(&window);
        return;
    }
    match current(app).all().into_iter().next() {
        Some(place) => open_place(app, &place, Goto::Stay),
        None => show_connect(app),
    }
}

/// Whether one of kwant's windows has the keyboard (the nudges then stay quiet).
fn kwant_focused(handle: &AppHandle) -> bool {
    handle
        .webview_windows()
        .values()
        .any(|w| is_place_window(w.label()) && w.is_focused().unwrap_or(false))
}

/// A place's window, for app-wide things (the Dock badge, attention): the one for `key` when it
/// is there, else any.
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
fn place_window(handle: &AppHandle, key: Option<&str>) -> Option<WebviewWindow> {
    let wanted = key.and_then(|k| place_by_key(handle, k)).map(|p| p.window_label());
    if let Some(w) = wanted.and_then(|l| handle.get_webview_window(&l)) {
        return Some(w);
    }
    handle.webview_windows().into_values().find(|w| is_place_window(w.label()))
}

/// Titles follow the places: "kwant" with This Mac alone, else "kwant – This Mac" / "kwant – host".
fn refresh_titles<R: Runtime>(app: &AppHandle<R>) {
    let places = current(app);
    for place in places.all() {
        if let Some(w) = app.get_webview_window(&place.window_label()) {
            let _ = w.set_title(&places::window_title(&place, &places));
        }
    }
}

/// Remembers which windows show, for the next launch.
fn save_open<R: Runtime>(app: &AppHandle<R>) {
    if DEMO.get().is_some() {
        return;
    }
    let shown: Vec<Place> = current(app)
        .all()
        .into_iter()
        .filter(|p| {
            app.get_webview_window(&p.window_label())
                .and_then(|w| w.is_visible().ok())
                .unwrap_or(false)
        })
        .collect();
    let shared = app.state::<Shared>();
    let mut places = shared.places.lock().unwrap();
    for place in places.all() {
        places.set_open(&place, shown.contains(&place));
    }
    if let Err(e) = places.save(&shared.data_dir) {
        tracing::warn!(error = %e, "couldn't save which windows were open");
    }
}

/// What a link in a place's window does (`popup`: it asked for a new window). A link to another
/// place opens that place's window; a page off every place goes to the browser.
fn route<R: Runtime>(app: &AppHandle<R>, me: &Place, url: &Url, popup: bool) -> bool {
    // port 0 never matches a link: This Mac's window before its kwant runs
    let mine = home(app, me).unwrap_or(Home::Local { port: 0 });
    let others: Vec<(Place, Home)> = current(app)
        .all()
        .into_iter()
        .filter(|p| p != me)
        .filter_map(|p| home(app, &p).map(|h| (p, h)))
        .collect();
    let homes: Vec<Home> = others.iter().map(|(_, h)| h.clone()).collect();
    let link = if popup { links::new_window(url, &mine, &homes) } else { links::navigation(url, &mine, &homes) };
    match link {
        links::Link::Stay => !popup,
        links::Link::Place(i) => {
            let place = others[i].0.clone();
            let (app, url) = (app.clone(), url.clone());
            // never build or show a window inside the webview's own callback
            std::thread::spawn(move || open_place(&app, &place, Goto::Url(url)));
            false
        }
        links::Link::Browser => {
            links::open_in_browser(url);
            false
        }
        links::Link::Drop => {
            tracing::info!(scheme = url.scheme(), "blocked a link the app doesn't open");
            false
        }
    }
}

/// A place's window. It opens on the "Starting kwant…" page and moves to the place once it
/// answers.
fn make_window<R: Runtime>(app: &AppHandle<R>, place: &Place, goto: Goto, visible: bool) {
    let title = places::window_title(place, &current(app));
    let (nav_app, nav_place) = (app.clone(), place.clone());
    let (pop_app, pop_place) = (app.clone(), place.clone());
    let window = match WebviewWindowBuilder::new(app, place.window_label(), WebviewUrl::App("index.html".into()))
        .title(title)
        .visible(visible)
        .inner_size(1280.0, 820.0)
        .min_inner_size(900.0, 600.0)
        .initialization_script(APP_MARK)
        // The window only shows its place; another place opens in its own window, every other
        // page in the default browser
        .on_navigation(move |url| route(&nav_app, &nav_place, url, false))
        .on_new_window(move |url, _| {
            route(&pop_app, &pop_place, &url, true);
            NewWindowResponse::Deny
        })
        .build()
    {
        Ok(w) => w,
        Err(e) => {
            tracing::error!(error = %e, "couldn't open the window");
            return;
        }
    };
    begin(window, place.clone(), goto);
}

/// Waits for the place (This Mac: /api/health on loopback; a server: the native check), then
/// moves the window from the loading page to `goto`. On failure the loading page says so.
fn begin<R: Runtime>(window: WebviewWindow<R>, place: Place, goto: Goto) {
    std::thread::spawn(move || {
        let app = window.app_handle().clone();
        match &place {
            Place::ThisMac => {
                app.state::<Shared>().local.lock().unwrap().failed = false;
                wake(&app);
                let port = ensure_local(&app);
                let answered = port.filter(|p| startup::wait_for_health(*p, Duration::from_secs(120)).is_some());
                let Some(port) = answered else {
                    tracing::error!(?port, "kwant didn't answer within 2 minutes");
                    app.state::<Shared>().local.lock().unwrap().failed = true;
                    wake(&app);
                    let _ = window.eval("document.body.dataset.state = 'failed'");
                    return;
                };
                let home = Home::Local { port };
                let url = match goto {
                    Goto::Url(url) => Some(url),
                    Goto::Path(path) => home.page(&path),
                    Goto::Stay => home.page("/"),
                };
                if let Some(url) = url {
                    let _ = window.navigate(url);
                }
            }
            Place::Server(origin) => {
                set_reach(&app, origin, Reach::Connecting);
                let client = remote::client();
                let mut result = Err(remote::Failure::Timeout);
                // a laptop just woken up may need a moment for its network
                for attempt in 0..3 {
                    if attempt > 0 {
                        std::thread::sleep(Duration::from_secs(3));
                    }
                    result = tauri::async_runtime::block_on(remote::check(&client, origin));
                    if result.is_ok() {
                        break;
                    }
                }
                match result {
                    Ok(_) => {
                        set_reach(&app, origin, Reach::Connected);
                        let url = match goto {
                            Goto::Url(url) => url,
                            Goto::Path(path) => origin.page(&path),
                            Goto::Stay => origin.page("/"),
                        };
                        let _ = window.navigate(url);
                    }
                    Err(failure) => {
                        set_reach(&app, origin, Reach::Unreachable);
                        let host = origin.host();
                        tracing::warn!(host = %host, reason = failure.code(), "the kwant server didn't answer");
                        let state = serde_json::json!({ "host": host, "code": failure.code(), "message": failure.message(&host) });
                        let js = format!(
                            "(function(s){{var b=document.body;b.dataset.host=s.host;b.dataset.code=s.code;b.dataset.message=s.message;b.dataset.state='unreachable';}})({state})"
                        );
                        let _ = window.eval(js.as_str());
                    }
                }
            }
        }
    });
}

/// The loading page's Try again: the window's own place, checked (or started) again.
fn show_home<R: Runtime>(app: &AppHandle<R>, place: &Place) {
    let Some(window) = app.get_webview_window(&place.window_label()) else {
        return;
    };
    let _ = window.navigate(connect::bundled("index.html"));
    begin(window, place.clone(), Goto::Stay);
}

/// The "Add a server" window (one at a time).
fn show_connect<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(CONNECT) {
        show(&window);
        return;
    }
    // it only shows the app's own page; a link on it opens in the browser
    let off_page = Home::Local { port: 0 };
    let popup_page = off_page.clone();
    let built = WebviewWindowBuilder::new(app, CONNECT, WebviewUrl::App("connect.html".into()))
        .title("Add a server")
        .inner_size(560.0, 640.0)
        .min_inner_size(420.0, 480.0)
        .initialization_script(APP_MARK)
        .on_navigation(move |url| match links::navigation(url, &off_page, &[]) {
            links::Link::Stay => true,
            links::Link::Browser => {
                links::open_in_browser(url);
                false
            }
            _ => false,
        })
        .on_new_window(move |url, _| {
            if links::new_window(&url, &popup_page, &[]) == links::Link::Browser {
                links::open_in_browser(&url);
            }
            NewWindowResponse::Deny
        })
        .build();
    match built {
        Ok(window) => show(&window),
        Err(e) => tracing::error!(error = %e, "couldn't open the Add a server window"),
    }
}

/// Adds a checked server (and saves it), then shows its window; a sign-in link opens there once.
/// `Err`: the file couldn't be written (nothing changed).
fn add_place<R: Runtime>(app: &AppHandle<R>, origin: Origin, link: Option<Url>) -> std::io::Result<bool> {
    let place = Place::Server(origin.clone());
    let added = {
        let shared = app.state::<Shared>();
        let mut places = shared.places.lock().unwrap();
        let before = places.clone();
        let added = places.add_server(origin.clone());
        if let Err(e) = places.save(&shared.data_dir) {
            *places = before;
            return Err(e);
        }
        added
    };
    tracing::info!(host = %origin.host(), added, "a server is listed");
    let window = app.get_webview_window(&place.window_label());
    match (window, link) {
        // a listed server's window: the link opens there, after its place answers
        (Some(window), Some(link)) => {
            let _ = window.navigate(connect::bundled("index.html"));
            begin(window.clone(), place.clone(), Goto::Url(link));
            show(&window);
        }
        (Some(window), None) => show(&window),
        (None, link) => make_window(app, &place, link.map_or(Goto::Stay, Goto::Url), true),
    }
    if let Some(w) = app.get_webview_window(CONNECT) {
        let _ = w.close();
    }
    refresh_titles(app);
    wake(app);
    Ok(added)
}

/// Takes a server out: its sign-in cookie, its window, its line in the file. With no place
/// left, This Mac comes back on. Never on the main thread (cookies are read there).
fn remove_place<R: Runtime>(app: &AppHandle<R>, origin: &Origin) -> bool {
    let label = Place::Server(origin.clone()).window_label();
    clear_token(app, origin);
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.destroy();
    }
    let shared = app.state::<Shared>();
    let (removed, back_here) = {
        let mut places = shared.places.lock().unwrap();
        let removed = places.remove_server(origin);
        let back_here = removed && places.all().is_empty();
        if back_here {
            places.set_this_mac(true);
        }
        if removed {
            if let Err(e) = places.save(&shared.data_dir) {
                tracing::warn!(error = %e, "couldn't save the server list");
            }
        }
        (removed, back_here)
    };
    shared.reach.lock().unwrap().remove(&origin.as_str());
    {
        let mut last = shared.last_focused.lock().unwrap();
        if last.as_deref() == Some(label.as_str()) {
            *last = None;
        }
    }
    if removed {
        tracing::info!(host = %origin.host(), "removed a server");
    }
    if back_here {
        open_place(app, &Place::ThisMac, Goto::Stay);
    }
    refresh_titles(app);
    wake(app);
    removed
}

/// Why "Run kwant on this Mac" didn't change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToggleRefused {
    /// Off with no server would leave nothing to show.
    LastPlace,
    SaveFailed,
}

/// "Run kwant on this Mac": on starts it (or finds the one running here) and opens its window;
/// off closes the window and stops the kwant this app started.
fn set_local<R: Runtime>(app: &AppHandle<R>, on: bool) -> Result<(), ToggleRefused> {
    let shared = app.state::<Shared>();
    {
        let mut places = shared.places.lock().unwrap();
        if places.this_mac == on {
            return Ok(());
        }
        if !on && places.servers.is_empty() {
            return Err(ToggleRefused::LastPlace);
        }
        let before = places.clone();
        places.set_this_mac(on);
        if let Err(e) = places.save(&shared.data_dir) {
            tracing::warn!(error = %e, "couldn't save Run kwant on this Mac");
            *places = before;
            return Err(ToggleRefused::SaveFailed);
        }
    }
    tracing::info!(on, "Run kwant on this Mac");
    if on {
        let app = app.clone();
        std::thread::spawn(move || {
            ensure_local(&app);
            open_place(&app, &Place::ThisMac, Goto::Stay);
            refresh_titles(&app);
            wake(&app);
        });
        return Ok(());
    }
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.destroy();
    }
    let server = std::mem::take(&mut *shared.local.lock().unwrap()).server;
    if let Some(server) = server {
        let stopping = std::thread::spawn(move || server.shutdown());
        *shared.stopping.lock().unwrap() = Some(stopping);
    }
    {
        let mut last = shared.last_focused.lock().unwrap();
        if last.as_deref() == Some(MAIN) {
            *last = None;
        }
    }
    refresh_titles(app);
    wake(app);
    Ok(())
}

/// The sign-in cookie for a server, in memory only. WebKit keeps one cookie store for all the
/// app's windows, so any window can read it; the server's own is asked first. Never call it on
/// the main thread: the webview answers there.
fn read_token<R: Runtime>(app: &AppHandle<R>, origin: &Origin) -> Option<remote::Token> {
    let cookies = cookie_window(app, origin)?.cookies().ok()?;
    remote::pick_token(
        origin,
        cookies.iter().map(|c| (c.name(), c.value(), c.domain(), c.path())),
    )
}

/// Removes kwant's sign-in cookie for this server (Remove a server). Not on the main thread.
fn clear_token<R: Runtime>(app: &AppHandle<R>, origin: &Origin) {
    let Some(window) = cookie_window(app, origin) else { return };
    let Ok(cookies) = window.cookies() else { return };
    for cookie in cookies {
        let ours = remote::pick_token(origin, [(cookie.name(), cookie.value(), cookie.domain(), cookie.path())]).is_some();
        if ours && window.delete_cookie(cookie).is_err() {
            tracing::warn!("couldn't clear a sign-in cookie");
        }
    }
}

fn cookie_window<R: Runtime>(app: &AppHandle<R>, origin: &Origin) -> Option<WebviewWindow<R>> {
    app.get_webview_window(&Place::Server(origin.clone()).window_label())
        .or_else(|| app.webview_windows().into_values().find(|w| is_place_window(w.label())))
        .or_else(|| app.webview_windows().into_values().next())
}

/// The tray's Remove: the one confirm, off the main thread.
fn confirm_remove(app: &AppHandle, origin: &str) {
    let Some(origin) = current(app).servers.into_iter().find(|o| o.as_str() == origin) else {
        return;
    };
    let app = app.clone();
    std::thread::spawn(move || {
        confirm_and_remove(&app, &origin);
    });
}

/// The server's row says it can't be reached (its window's check or the menu's last poll).
fn unreachable_now<R: Runtime>(app: &AppHandle<R>, origin: &Origin) -> bool {
    let shared = app.state::<Shared>();
    let by_window = shared.reach.lock().unwrap().get(&origin.as_str()) == Some(&Reach::Unreachable);
    by_window || shared.unreachable.lock().unwrap().contains(&Place::Server(origin.clone()).key())
}

/// Every Remove (tray or page): the native confirm, then the server sign-out, then the remove;
/// a sign-out the server didn't confirm is said once. Blocks: never on the main thread or on
/// an async task (it waits for the sign-out).
fn confirm_and_remove<R: Runtime>(app: &AppHandle<R>, origin: &Origin) -> Removed {
    let places = current(app);
    if !places.servers.contains(origin) {
        return Removed::default();
    }
    let last_place = places.servers.len() == 1 && !places.this_mac;
    let host = origin.host();
    let words = remove_words(&host, unreachable_now(app, origin), last_place, cfg!(target_os = "macos"));
    let asker = app.state::<Asker>();
    if !asker.confirm(app, &words.title, &words.message) {
        return Removed { cancelled: true, ..Removed::default() };
    }
    let revoked = tauri::async_runtime::block_on(sign_out_of(app, origin));
    let removed = remove_place(app, origin);
    if removed && !revoked && !words.says_unreachable {
        let (title, message) = removed_words(&host);
        asker.tell(app, &title, message);
    }
    Removed { cancelled: false, removed, revoked }
}

/// Signs this app out of a server with the cookie it keeps for it (`DELETE /api/auth/session`).
/// `false` for no cookie, no answer or an answer that isn't 2xx; never blocks a remove.
/// Not on the main thread (cookies are read there).
async fn sign_out_of<R: Runtime>(app: &AppHandle<R>, origin: &Origin) -> bool {
    let token = read_token(app, origin);
    let revoked = remote::sign_out(&remote::client(), origin, token.as_ref()).await;
    tracing::info!(host = %origin.host(), revoked, "signed this app out of a server");
    revoked
}

/// One item of the tray menu, kept to change its text or tick in place.
enum Handle {
    Item(MenuItem<Wry>),
    Check(CheckMenuItem<Wry>),
    Other,
}

/// The tray menu as built, with the entries it shows.
struct TrayMenu {
    entries: Vec<menu::Entry>,
    handles: Vec<Handle>,
}

fn build_menu(app: &AppHandle, entries: Vec<menu::Entry>) -> tauri::Result<(Menu<Wry>, TrayMenu)> {
    let built = Menu::new(app)?;
    let mut handles = Vec::new();
    for entry in &entries {
        match entry {
            menu::Entry::Item { id, text, enabled } => {
                let accel = (id == "quit").then_some("CmdOrCtrl+Q");
                let item = MenuItem::with_id(app, id, text, *enabled, accel)?;
                built.append(&item)?;
                handles.push(Handle::Item(item));
            }
            menu::Entry::Check { id, text, checked } => {
                let item = CheckMenuItem::with_id(app, id, text, true, *checked, None::<&str>)?;
                built.append(&item)?;
                handles.push(Handle::Check(item));
            }
            menu::Entry::Submenu { text, items } => {
                let sub = Submenu::new(app, text, true)?;
                for (id, text) in items {
                    sub.append(&MenuItem::with_id(app, id, text, true, None::<&str>)?)?;
                }
                built.append(&sub)?;
                handles.push(Handle::Other);
            }
            menu::Entry::Separator => {
                built.append(&PredefinedMenuItem::separator(app)?)?;
                handles.push(Handle::Other);
            }
        }
    }
    Ok((built, TrayMenu { entries, handles }))
}

/// Shows `entries`: in place when only texts and ticks changed, else as a new menu.
fn apply_menu(app: &AppHandle, tray: &TrayIcon, shown: &mut TrayMenu, entries: Vec<menu::Entry>) {
    let same_shape = shown.entries.len() == entries.len()
        && shown.entries.iter().zip(&entries).all(|(a, b)| a.shape() == b.shape());
    if !same_shape {
        match build_menu(app, entries) {
            Ok((built, fresh)) => {
                if tray.set_menu(Some(built)).is_ok() {
                    *shown = fresh;
                }
            }
            Err(e) => tracing::warn!(error = %e, "couldn't rebuild the menu"),
        }
        return;
    }
    for (i, entry) in entries.into_iter().enumerate() {
        if shown.entries[i] == entry {
            continue;
        }
        match (&shown.handles[i], &entry) {
            (Handle::Item(item), menu::Entry::Item { text, enabled, .. }) => {
                let _ = item.set_text(text);
                let _ = item.set_enabled(*enabled);
            }
            (Handle::Check(item), menu::Entry::Check { text, checked, .. }) => {
                let _ = item.set_text(text);
                let _ = item.set_checked(*checked);
            }
            _ => {}
        }
        shown.entries[i] = entry;
    }
}

/// ⌘1, ⌘2… in the Window menu, one per place (spec P8).
#[cfg(target_os = "macos")]
fn set_window_menu(app: &AppHandle, places: &[Place]) {
    let built = (|| -> tauri::Result<()> {
        let bar = Menu::default(app)?;
        if let Some(window) = bar.get(tauri::menu::WINDOW_SUBMENU_ID).and_then(|k| k.as_submenu().cloned()) {
            window.append(&PredefinedMenuItem::separator(app)?)?;
            for (i, place) in places.iter().enumerate() {
                let accel = (i < 9).then(|| format!("CmdOrCtrl+{}", i + 1));
                let text = match place {
                    Place::ThisMac => place.label(),
                    Place::Server(o) => menu::short_host(&o.host()),
                };
                window.append(&MenuItem::with_id(app, format!("win:{}", place.key()), text, true, accel.as_deref())?)?;
            }
        }
        app.set_menu(bar)?;
        Ok(())
    })();
    if let Err(e) = built {
        tracing::warn!(error = %e, "couldn't set the Window menu");
    }
}

/// Every menu click: the tray's and the Window menu's.
fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "open" => open_place(app, &Place::ThisMac, Goto::Stay),
        "update" => {
            // The window shows the download and the relaunch
            if let Err(e) = kwnt::app_update::UpdateHub::shared().install() {
                tracing::info!(reason = e.message(), "update from the menu not started");
            }
            show_last(app);
        }
        "nudges" => {
            let state = app.state::<Nudges>();
            let on = !state.show.load(Ordering::Relaxed);
            state.show.store(on, Ordering::Relaxed);
            if let Err(e) = (nudge::Prefs { show_nudges: on }).save(&state.data_dir) {
                tracing::warn!(error = %e, "couldn't save Show nudges");
            }
            wake(app);
        }
        "add" => show_connect(app),
        "this_mac" => {
            let on = !current(app).this_mac;
            if let Err(e) = set_local(app, on) {
                tracing::info!(?e, "Run kwant on this Mac unchanged");
            }
            wake(app);
        }
        "quit" => {
            save_open(app);
            app.exit(0);
        }
        _ => {
            if let Some(key) = id.strip_prefix("place:").or_else(|| id.strip_prefix("win:")) {
                if let Some(place) = place_by_key(app, key) {
                    open_place(app, &place, Goto::Stay);
                }
            } else if let Some(origin) = id.strip_prefix("remove:") {
                confirm_remove(app, origin);
            }
        }
    }
}

/// The menu-bar icon (spec P4, `menu.rs`): each place with what waits there, Add a server…,
/// Remove a server, Run kwant on this Mac, Show nudges, Quit. With This Mac alone it reads as
/// before. "Update to x.y.z…" appears at the top while a newer version is on offer.
///
/// A server's count comes from the server with the windows' sign-in cookie (read from the
/// webview, kept in memory only). The nudges (`nudge.rs`) follow the same poll.
fn build_tray(handle: &AppHandle) -> tauri::Result<()> {
    let show = handle.state::<Nudges>().show.load(Ordering::Relaxed);
    let places = current(handle);
    let first = menu::model(&menu::Input {
        places: &places,
        status: &|p| if *p == Place::ThisMac { menu::Status::Starting } else { menu::Status::Connecting },
        local_line: menu::Status::Starting.text(),
        show_nudges: show,
        update: None,
    });
    let (first_menu, mut shown) = build_menu(handle, first)?;
    let icon = tauri::image::Image::from_bytes(TRAY_ICON)?;
    let tray = TrayIconBuilder::with_id("kwant")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("kwant")
        .menu(&first_menu)
        .show_menu_on_left_click(true)
        // this handler gets every menu's clicks, the Window menu's too
        .on_menu_event(|app, event| on_menu(app, event.id.as_ref()))
        .build(handle)?;

    let (wake_tx, wake_rx) = mpsc::channel();
    *handle.state::<Shared>().wake.lock().unwrap() = Some(wake_tx);
    let hub = kwnt::app_update::UpdateHub::shared();
    let app = handle.clone();
    std::thread::spawn(move || {
        let mut nudger = platform::Nudger::start(&app, tray.clone());
        let mut tracker = nudge::Tracker::default();
        let mut demo_step = 0;
        let mut updates = false;
        let mut polls: HashMap<String, RemotePoll> = HashMap::new();
        let mut status: HashMap<String, menu::Status> = HashMap::new();
        // what each place said last, so a missed poll doesn't make its items new again
        let mut last: HashMap<String, nudge::Waiting> = HashMap::new();
        #[cfg(target_os = "macos")]
        let mut window_menu: Option<Vec<String>> = None;
        loop {
            let shared = app.state::<Shared>();
            let places = current(&app);
            let all = places.all();
            let keys: Vec<String> = all.iter().map(Place::key).collect();
            polls.retain(|k, _| keys.contains(k));
            status.retain(|k, _| keys.contains(k));
            last.retain(|k, _| keys.contains(k));
            let demo = DEMO.get().map(|t| nudge::demo::step(t.elapsed()));
            let (port, failed) = {
                let local = shared.local.lock().unwrap();
                (local.port, local.failed)
            };
            let reach = shared.reach.lock().unwrap().clone();
            for place in &all {
                let key = place.key();
                let (said, waiting) = match place {
                    Place::ThisMac => {
                        let waiting = match (demo, port) {
                            (Some(step), _) => Some(nudge::demo::waiting(step)),
                            (None, Some(port)) if !failed => poll_waiting(port),
                            _ => None,
                        };
                        let said = match &waiting {
                            Some(w) => menu::Status::Waiting(w.count),
                            None if failed => menu::Status::DidntStart,
                            None => menu::Status::Starting,
                        };
                        (said, waiting)
                    }
                    Place::Server(origin) => {
                        let poll = polls.entry(key.clone()).or_insert_with(|| RemotePoll::new(origin.clone()));
                        match poll.poll(&app) {
                            Polled::Ok(w) => (menu::Status::Waiting(w.count), Some(w)),
                            Polled::SignedOut => (menu::Status::SignIn, None),
                            Polled::Failed if reach.get(&key) == Some(&Reach::Connecting) => (menu::Status::Connecting, None),
                            Polled::Failed => (menu::Status::Unreachable, None),
                        }
                    }
                };
                status.insert(key.clone(), said);
                if let Some(w) = waiting {
                    last.insert(key, w);
                }
            }
            // P7: with more than one place, each nudge names its place
            let labelled = all.len() > 1;
            let mut merged: Option<nudge::Waiting> = None;
            for place in &all {
                if let Some(w) = last.get(&place.key()) {
                    let label = if labelled { place.label() } else { String::new() };
                    merged.get_or_insert_with(Default::default).add(w.clone().in_place(&place.key(), &label));
                }
            }
            let status_of = |p: &Place| {
                status.get(&p.key()).copied().unwrap_or(if *p == Place::ThisMac {
                    menu::Status::Starting
                } else {
                    menu::Status::Connecting
                })
            };
            *shared.unreachable.lock().unwrap() = all
                .iter()
                .filter(|p| status_of(p) == menu::Status::Unreachable)
                .map(Place::key)
                .collect();
            let show = app.state::<Nudges>().show.load(Ordering::Relaxed);
            let local_line = match (status_of(&Place::ThisMac), last.get(places::THIS_MAC), port) {
                (menu::Status::Waiting(_), Some(w), Some(port)) => nudge::status_line(w, port).0,
                (said, _, _) => said.text(),
            };
            let _ = tray.set_title(menu::badge(&places, &status_of).as_deref());
            if let Some(w) = &merged {
                let change = tracker.update(w, Instant::now());
                nudger.on_poll(&app, &change, show);
            }
            if let Some(step) = demo.filter(|s| *s != demo_step) {
                demo_step = step;
                println!("demo-nudge: step {step}: {local_line}");
            }
            let entries = menu::model(&menu::Input {
                places: &places,
                status: &status_of,
                local_line,
                show_nudges: show,
                update: update::menu_item(&hub.status()),
            });
            apply_menu(&app, &tray, &mut shown, entries);
            #[cfg(target_os = "macos")]
            if demo.is_none() && window_menu.as_ref() != Some(&keys) {
                set_window_menu(&app, &all);
                window_menu = Some(keys.clone());
            }
            // The app's own update: when it runs kwant here or shows a server (an attached
            // kwant updates itself)
            if !updates && demo.is_none() && (shared.started_kwant.load(Ordering::Relaxed) || !places.servers.is_empty()) {
                updates = true;
                update::start(&app);
            }
            // the demo moves on a timeline CI screenshots, so it looks every second
            let _ = wake_rx.recv_timeout(Duration::from_secs(if demo.is_some() { 1 } else { 5 }));
            while wake_rx.try_recv().is_ok() {}
        }
    });
    Ok(())
}

/// One poll of a server.
enum Polled {
    Ok(nudge::Waiting),
    /// It answers, but not to this app's cookie (none yet, or signed out).
    SignedOut,
    Failed,
}

/// The tray's poll of kwant on a server, signed in with the windows' cookie.
struct RemotePoll {
    origin: Origin,
    client: reqwest::Client,
    token: Option<remote::Token>,
    read_at: Option<Instant>,
}

/// How often the app looks for a sign-in cookie it doesn't have (or that stopped working).
const COOKIE_RECHECK: Duration = Duration::from_secs(30);

impl RemotePoll {
    fn new(origin: Origin) -> RemotePoll {
        RemotePoll { origin, client: remote::client(), token: None, read_at: None }
    }

    fn poll(&mut self, app: &AppHandle) -> Polled {
        if self.token.is_none() && self.read_at.is_none_or(|t| t.elapsed() >= COOKIE_RECHECK) {
            self.read_at = Some(Instant::now());
            self.token = read_token(app, &self.origin);
        }
        let first = tauri::async_runtime::block_on(remote::get(&self.client, &self.origin, self.token.as_ref(), "/api/spaces"));
        let spaces = match first {
            remote::Answer::Ok(body) => body,
            remote::Answer::SignedOut => {
                // signed out, or the cookie changed: look again (at most every 30 s)
                self.token = None;
                return Polled::SignedOut;
            }
            remote::Answer::Failed => return Polled::Failed,
        };
        let (client, origin, token) = (&self.client, &self.origin, self.token.as_ref());
        let ok = |path: &str| match tauri::async_runtime::block_on(remote::get(client, origin, token, path)) {
            remote::Answer::Ok(body) => Some(body),
            _ => None,
        };
        Polled::Ok(collect_waiting(Some(spaces), ok))
    }
}

/// What waits for the person in every space (`None`: kwant isn't answering yet).
fn poll_waiting(port: u16) -> Option<nudge::Waiting> {
    if !startup::is_kwant(port) {
        return None;
    }
    let get = |path: &str| {
        startup::http_get(port, path, Duration::from_secs(2))
            .filter(|(status, _)| *status == 200)
            .map(|(_, body)| body)
    };
    Some(collect_waiting(get("/api/spaces"), get))
}

/// Counts what waits in every space from kwant's answers (`spaces`: the /api/spaces body). A
/// kwant too old for /api/away still gets its approvals counted, without nudges.
fn collect_waiting(spaces: Option<String>, get: impl Fn(&str) -> Option<String>) -> nudge::Waiting {
    let spaces = spaces.and_then(|b| nudge::parse_spaces(&b));
    let mut all = nudge::Waiting::default();
    let mut answered = false;
    match spaces {
        Some(ids) if !ids.is_empty() => {
            for id in ids {
                if let Some(w) = get(&nudge::away_path(Some(&id))).and_then(|b| nudge::parse_away(&b, Some(&id))) {
                    all.add(w);
                    answered = true;
                }
            }
        }
        _ => {
            if let Some(w) = get(&nudge::away_path(None)).and_then(|b| nudge::parse_away(&b, None)) {
                all.add(w);
                answered = true;
            }
        }
    }
    if !answered {
        let approvals = get("/api/approvals?status=pending")
            .and_then(|b| serde_json::from_str::<Vec<serde_json::Value>>(&b).ok())
            .map(|rows| rows.len())
            .unwrap_or(0);
        all.count = approvals;
    }
    all
}

/// Platforms without a nudge of their own (the app ships for macOS and Linux) keep the count.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    pub struct Nudger;
    impl Nudger {
        pub fn start(_: &tauri::AppHandle, _: tauri::tray::TrayIcon) -> Nudger {
            Nudger
        }
        pub fn on_poll(&mut self, _: &tauri::AppHandle, _: &crate::nudge::Change, _: bool) {}
    }
}

/// `--check`: start kwant headless on a free port with a throwaway data dir, wait for
/// /api/health, print it and exit 0 (1 if it never answers). CI runs this on the built app.
fn check() -> i32 {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let data_dir = std::env::temp_dir().join(format!("kwant-check-{}", std::process::id()));
    let port = match startup::free_port() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("kwant --check: no free port: {e}");
            return 1;
        }
    };
    let config = kwnt::Config {
        data_dir: data_dir.clone(),
        host: "127.0.0.1".to_string(),
        port,
        laya_managed: false,
        ..Default::default()
    };
    let server = start_server(config);
    let result = startup::wait_for_health(port, Duration::from_secs(60));
    server.shutdown();
    let _ = std::fs::remove_dir_all(&data_dir);
    match result {
        Some(body) => {
            println!("kwant --check: /api/health on port {port}: {body}");
            println!("kwant --check: PATH={}", std::env::var("PATH").unwrap_or_default());
            0
        }
        None => {
            eprintln!("kwant --check: /api/health never answered on port {port}");
            1
        }
    }
}

/// Logs go to ~/.kwnt/mac-app.log (desktop-app.log on Linux): an app opened from Finder or an
/// app menu has no terminal. They may name a server's host, never a token, cookie or fragment.
fn init_logging(data_dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(data_dir);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir.join(LOG_FILE));
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    // Never a token, a cookie or an address's fragment, whatever logs the line (redact.rs)
    match file {
        Ok(file) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(Mutex::new(redact::Redacting::new(file)))
            .init(),
        Err(_) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(|| redact::Redacting::new(std::io::stderr()))
            .init(),
    }
}

fn fatal(message: &str) -> ! {
    eprintln!("{message}");
    tracing::error!("{message}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{get_ipc_response, mock_builder, MockRuntime, INVOKE_KEY};
    use tauri::webview::InvokeRequest;

    const COMMANDS: &[&str] = &[
        "places_state",
        "check_server",
        "add_server",
        "remove_server",
        "set_this_mac",
        "close_connect",
        "retry_start",
    ];

    /// The app's commands and its real capabilities, on Tauri's mock runtime, in a window
    /// labelled `label`.
    fn app_with(
        name: &str,
        label: &str,
        places: Places,
    ) -> (tauri::App<MockRuntime>, WebviewWindow<MockRuntime>, PathBuf) {
        app_answering(name, label, places, true)
    }

    /// The same, with the Remove confirm answered `answer`.
    fn app_answering(
        name: &str,
        label: &str,
        places: Places,
        answer: bool,
    ) -> (tauri::App<MockRuntime>, WebviewWindow<MockRuntime>, PathBuf) {
        let data_dir = std::env::temp_dir().join(format!("kwant-ipc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data_dir);
        let config = kwnt::Config { data_dir: data_dir.clone(), ..Default::default() };
        let shared = Shared::new(data_dir.clone(), config, places);
        // a port nothing listens on: tests never start or reach a real kwant
        shared.local.lock().unwrap().port = Some(1);
        let app = mock_builder()
            .manage(shared)
            .manage(Nudges { data_dir: data_dir.clone(), show: AtomicBool::new(true) })
            .manage(Asker::Fixed { answer, said: Mutex::new(Vec::new()) })
            .invoke_handler(tauri::generate_handler![
                connect::places_state,
                connect::check_server,
                connect::add_server,
                connect::remove_server,
                connect::set_this_mac,
                connect::close_connect,
                connect::retry_start,
            ])
            .build(tauri::generate_context!(test = true))
            .expect("the mock app");
        let window = WebviewWindowBuilder::new(&app, label, WebviewUrl::App("index.html".into()))
            .build()
            .expect("the mock window");
        (app, window, data_dir)
    }

    fn app(name: &str) -> (tauri::App<MockRuntime>, WebviewWindow<MockRuntime>, PathBuf) {
        app_with(name, MAIN, Places::default())
    }

    fn invoke(
        window: &WebviewWindow<MockRuntime>,
        cmd: &str,
        from: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, serde_json::Value> {
        get_ipc_response(
            window,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: from.parse().unwrap(),
                body: InvokeBody::Json(args),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|body| body.deserialize::<serde_json::Value>().unwrap_or_default())
    }

    fn nothing_saved(data_dir: &std::path::Path) {
        assert!(!data_dir.join(target::TARGET_FILE).exists(), "no old file was written");
        assert!(!data_dir.join(places::PLACES_FILE).exists(), "no places file was written");
    }

    fn args() -> serde_json::Value {
        serde_json::json!({ "address": "https://kwant.example.com", "origin": "https://kwant.example.com", "on": false })
    }

    #[test]
    fn a_server_s_pages_can_call_no_command() {
        let (_app, window, data_dir) = app("remote");
        for from in [
            "https://kwant.example.com/",
            "https://kwant.example.com/s/spc_default/inbox",
            "http://127.0.0.1:2222/",
            "https://localhost/",
            "https://tauri.localhost.evil.example/",
        ] {
            for cmd in COMMANDS {
                let refused = invoke(&window, cmd, from, args());
                // the access control answers (a plain message), the command never runs
                assert!(
                    matches!(&refused, Err(serde_json::Value::String(_))),
                    "{cmd} from {from}: {refused:?}"
                );
            }
        }
        nothing_saved(&data_dir);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    /// The same holds in a server's own window (its label is in the capability for the bundled
    /// loading page, never for the server's pages).
    #[test]
    fn a_server_window_s_pages_can_call_no_command() {
        let origin = target::parse_address("kwant.example.com").unwrap().origin;
        let mut places = Places::default();
        places.add_server(origin.clone());
        let label = Place::Server(origin).window_label();
        let (_app, window, data_dir) = app_with("server-window", &label, places);
        for from in ["https://kwant.example.com/", "https://kwant.example.com/s/spc_default/inbox"] {
            for cmd in COMMANDS {
                let refused = invoke(&window, cmd, from, args());
                assert!(matches!(&refused, Err(serde_json::Value::String(_))), "{cmd} from {from}: {refused:?}");
            }
        }
        nothing_saved(&data_dir);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    /// Rigel's F7: Tauri's access check compares only the scheme and the host, so a page at the
    /// bundled host with a port or a user name gets past it. The command's own check then
    /// refuses it, and nothing is saved.
    #[test]
    fn a_bundled_look_alike_with_a_port_or_a_user_name_can_call_no_command() {
        let (_app, window, data_dir) = app("lookalike");
        for from in [
            "http://tauri.localhost:8080/",
            "tauri://localhost:8080/",
            "http://u:p@tauri.localhost/",
            "tauri://u@localhost/",
        ] {
            window.navigate(from.parse().unwrap()).unwrap();
            for cmd in COMMANDS {
                let refused = invoke(&window, cmd, from, args());
                let by_acl = matches!(&refused, Err(serde_json::Value::String(_)));
                let by_command = matches!(&refused, Err(e) if e["code"] == "not_allowed");
                assert!(by_acl || by_command, "{cmd} from {from}: {refused:?}");
            }
        }
        nothing_saved(&data_dir);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn the_bundled_pages_can() {
        let (_app, window, data_dir) = app("bundled");
        let page = connect::bundled("connect.html").to_string();
        let state = invoke(&window, "places_state", &page, serde_json::json!({})).expect("places_state from a bundled page");
        assert_eq!(state["this_mac"], true);
        assert_eq!(state["servers"], serde_json::json!([]));
        assert_eq!(state["last_origin"], serde_json::Value::Null);
        assert_eq!(state["place"]["kind"], "this_mac", "{state}");
        // the command itself runs (and refuses plain http before contacting anything)
        let refused = invoke(&window, "check_server", &page, serde_json::json!({ "address": "http://kwant.example.com" }))
            .unwrap_err();
        assert_eq!(refused["code"], "http_not_allowed", "{refused}");
        let refused = invoke(&window, "add_server", &page, serde_json::json!({ "address": "http://kwant.example.com" }))
            .unwrap_err();
        assert_eq!(refused["code"], "http_not_allowed", "{refused}");
        nothing_saved(&data_dir);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    /// This Mac can't go off while it is the only place: there would be nothing to show.
    #[test]
    fn this_mac_stays_on_without_a_server() {
        let (app, window, data_dir) = app("last-place");
        let page = connect::bundled("index.html").to_string();
        let refused = invoke(&window, "set_this_mac", &page, serde_json::json!({ "on": false })).unwrap_err();
        assert_eq!(refused["code"], "last_place", "{refused}");
        assert!(current(app.handle()).this_mac);
        // on while on: nothing changes, nothing is written
        let state = invoke(&window, "set_this_mac", &page, serde_json::json!({ "on": true })).unwrap();
        assert_eq!(state["this_mac"], true);
        nothing_saved(&data_dir);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn removing_a_server_that_isn_t_listed_changes_nothing() {
        let (app, window, data_dir) = app("remove-unlisted");
        let page = connect::bundled("index.html").to_string();
        let answer =
            invoke(&window, "remove_server", &page, serde_json::json!({ "origin": "https://kwant.example.com" })).unwrap();
        assert_eq!(answer["removed"], false);
        assert_eq!(answer["revoked"], false);
        assert!(said(&app).is_empty(), "no confirm for a server that isn't listed");
        assert_eq!(current(app.handle()), Places::default());
        nothing_saved(&data_dir);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn a_listed_server_shows_in_the_state_with_its_reach() {
        let origin = target::parse_address("kwant.example.com").unwrap().origin;
        let mut places = Places::default();
        places.add_server(origin.clone());
        let (app, window, data_dir) = app_with("listed", MAIN, places);
        set_reach(app.handle(), &origin, Reach::Unreachable);
        let page = connect::bundled("index.html").to_string();
        let state = invoke(&window, "places_state", &page, serde_json::json!({})).unwrap();
        assert_eq!(state["servers"][0]["origin"], "https://kwant.example.com");
        assert_eq!(state["servers"][0]["host"], "kwant.example.com");
        assert_eq!(state["servers"][0]["reach"], "unreachable");
        assert_eq!(state["last_origin"], "https://kwant.example.com");
        let _ = std::fs::remove_dir_all(data_dir);
    }

    /// With a server listed, This Mac can go off and the file says so. (Whether its window
    /// closes is the real runtime's part: the mock keeps destroyed windows.)
    #[test]
    fn this_mac_goes_off_beside_a_server() {
        let origin = target::parse_address("kwant.example.com").unwrap().origin;
        let mut places = Places::default();
        places.add_server(origin.clone());
        let (app, window, data_dir) = app_with("this-mac-off", "connect", places);
        let main = WebviewWindowBuilder::new(&app, MAIN, WebviewUrl::App("index.html".into())).build().unwrap();
        drop(main);
        let page = connect::bundled("connect.html").to_string();
        let state = invoke(&window, "set_this_mac", &page, serde_json::json!({ "on": false })).unwrap();
        assert_eq!(state["this_mac"], false);
        let saved = Places::load(&data_dir);
        assert!(!saved.this_mac);
        assert_eq!(saved.servers, vec![origin.clone()]);
        assert_eq!(saved.all(), vec![Place::Server(origin)]);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    /// Removing the only server takes it out of the file; with This Mac off, This Mac comes back.
    #[test]
    fn removing_the_last_server_brings_this_mac_back() {
        let origin = target::parse_address("kwant.example.com").unwrap().origin;
        let mut places = Places::default();
        places.add_server(origin.clone());
        places.set_this_mac(false);
        let (app, _window, data_dir) = app_with("remove-last", "connect", places);
        assert!(remove_place(app.handle(), &origin));
        let after = current(app.handle());
        assert!(after.servers.is_empty());
        assert!(after.this_mac);
        let saved = Places::load(&data_dir);
        assert_eq!(saved.servers, Vec::<Origin>::new());
        assert!(saved.this_mac);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    fn said(app: &tauri::App<MockRuntime>) -> Vec<(String, String)> {
        match &*app.state::<Asker>() {
            Asker::Fixed { said, .. } => said.lock().unwrap().clone(),
            Asker::Native => unreachable!(),
        }
    }

    const PLAIN: &str = "Its window closes and this app signs out of it. The bots on kwant.example.com keep \
        running.\n\nTo use it here again, choose Add a server… and sign in.";
    const UNREACHABLE: &str = "\n\nIt can't be reached right now, so it will still list this app under \
        Settings → Signed-in devices. You can remove it there later.";

    #[test]
    fn the_remove_confirm_says_mira_s_words() {
        let host = "kwant.example.com";
        let plain = remove_words(host, false, false, true);
        assert_eq!(plain.title, "Remove kwant.example.com?");
        assert_eq!(plain.message, PLAIN);
        assert!(!plain.says_unreachable);

        let away = remove_words(host, true, false, true);
        assert_eq!(away.title, "Remove kwant.example.com?");
        assert_eq!(away.message, format!("{PLAIN}{UNREACHABLE}"));
        assert!(away.says_unreachable);

        let last = remove_words(host, false, true, true);
        assert_eq!(
            last.message,
            format!("{PLAIN}\n\n“Run kwant on this Mac” turns back on, so the app still has a place to open.")
        );

        let linux = remove_words(host, false, true, false);
        assert_eq!(
            linux.message,
            format!("{PLAIN}\n\n“Run kwant on this computer” turns back on, so the app still has a place to open.")
        );
        assert_eq!(remove_words(host, false, false, false), plain, "Linux words differ only for the last place");

        let both = remove_words(host, true, true, true);
        assert!(both.message.starts_with(&format!("{PLAIN}{UNREACHABLE}\n\n“Run kwant on this Mac”")));

        assert_eq!(
            removed_words(host),
            (
                "Removed kwant.example.com".to_string(),
                "It couldn't be reached, so it still lists this app under Settings → Signed-in devices. \
                 You can remove it there later."
            )
        );
    }

    /// The page's Remove this server… gets the native confirm too. No cookie here, so the
    /// sign-out isn't confirmed and the one follow-up message says so.
    #[test]
    fn the_page_s_remove_asks_natively_then_says_it_wasn_t_signed_out() {
        let origin = target::parse_address("kwant.example.com").unwrap().origin;
        let mut places = Places::default();
        places.add_server(origin.clone());
        let (app, window, data_dir) = app_with("remove-page", MAIN, places);
        let page = connect::bundled("index.html").to_string();
        let answer = invoke(&window, "remove_server", &page, serde_json::json!({ "origin": origin.as_str() })).unwrap();
        assert_eq!(answer, serde_json::json!({ "removed": true, "revoked": false, "cancelled": false }));
        let said = said(&app);
        assert_eq!(said.len(), 2, "{said:?}");
        assert_eq!(said[0], ("Remove kwant.example.com?".to_string(), PLAIN.to_string()));
        assert_eq!(said[1].0, "Removed kwant.example.com");
        assert!(current(app.handle()).servers.is_empty());
        assert!(Places::load(&data_dir).servers.is_empty());
        let _ = std::fs::remove_dir_all(data_dir);
    }

    /// Unreachable as the confirm opens: the confirm says so, and nothing more is said after.
    #[test]
    fn an_unreachable_server_is_said_once_in_the_confirm() {
        let origin = target::parse_address("kwant.example.com").unwrap().origin;
        let mut places = Places::default();
        places.add_server(origin.clone());
        places.set_this_mac(false);
        let (app, window, data_dir) = app_with("remove-away", "connect", places);
        set_reach(app.handle(), &origin, Reach::Unreachable);
        let page = connect::bundled("index.html").to_string();
        let answer = invoke(&window, "remove_server", &page, serde_json::json!({ "origin": origin.as_str() })).unwrap();
        assert_eq!(answer["removed"], true);
        assert_eq!(answer["revoked"], false);
        let said = said(&app);
        assert_eq!(said.len(), 1, "{said:?}");
        let here = if cfg!(target_os = "macos") { "this Mac" } else { "this computer" };
        assert_eq!(
            said[0].1,
            format!("{PLAIN}{UNREACHABLE}\n\n“Run kwant on {here}” turns back on, so the app still has a place to open.")
        );
        assert!(current(app.handle()).this_mac, "the last place brought This Mac back");
        let _ = std::fs::remove_dir_all(data_dir);
    }

    /// The menu's last poll couldn't reach it: that counts as unreachable too.
    #[test]
    fn a_failed_poll_counts_as_unreachable() {
        let origin = target::parse_address("kwant.example.com").unwrap().origin;
        let mut places = Places::default();
        places.add_server(origin.clone());
        let (app, _window, data_dir) = app_with("remove-polled", MAIN, places);
        assert!(!unreachable_now(app.handle(), &origin));
        app.state::<Shared>().unreachable.lock().unwrap().insert(Place::Server(origin.clone()).key());
        assert!(unreachable_now(app.handle(), &origin));
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn cancel_keeps_the_server() {
        let origin = target::parse_address("kwant.example.com").unwrap().origin;
        let mut places = Places::default();
        places.add_server(origin.clone());
        let (app, window, data_dir) = app_answering("remove-cancel", MAIN, places.clone(), false);
        let page = connect::bundled("index.html").to_string();
        let answer = invoke(&window, "remove_server", &page, serde_json::json!({ "origin": origin.as_str() })).unwrap();
        assert_eq!(answer, serde_json::json!({ "removed": false, "revoked": false, "cancelled": true }));
        assert_eq!(said(&app).len(), 1, "only the confirm");
        assert_eq!(current(app.handle()), places);
        nothing_saved(&data_dir);
        let _ = std::fs::remove_dir_all(data_dir);
    }


    /// A one-shot listener that records every request line it gets and answers none of them
    /// (the connection closes), so a health check fails and nothing after it is reached.
    fn recording_listener() -> (u16, std::sync::Arc<Mutex<Vec<String>>>) {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(mut socket) = socket else { break };
                let mut buf = [0u8; 2048];
                let n = socket.read(&mut buf).unwrap_or(0);
                let text = String::from_utf8_lossy(&buf[..n]);
                log.lock().unwrap().push(text.lines().next().unwrap_or("(connected)").to_string());
            }
        });
        (port, seen)
    }

    /// "Show it" (design.md §3.1): a listed server without a link is only focused; nothing is
    /// sent to it, even when it is down.
    #[test]
    fn show_it_on_a_listed_server_never_touches_the_network() {
        let (port, seen) = recording_listener();
        let address = format!("http://127.0.0.1:{port}");
        let origin = target::parse_address(&address).unwrap().origin;
        let mut places = Places::default();
        places.add_server(origin.clone());
        let (app, window, data_dir) = app_with("show-it", "connect", places.clone());
        let label = Place::Server(origin.clone()).window_label();
        let server_window = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
            .visible(false)
            .build()
            .unwrap();
        let page = connect::bundled("connect.html").to_string();
        let started = Instant::now();
        let answer = invoke(&window, "add_server", &page, serde_json::json!({ "address": address })).unwrap();
        assert_eq!(answer["added"], false);
        assert_eq!(answer["origin"], origin.as_str());
        assert!(started.elapsed() < Duration::from_secs(2), "no check was waited for");
        std::thread::sleep(Duration::from_millis(200));
        assert!(seen.lock().unwrap().is_empty(), "nothing was sent: {:?}", seen.lock().unwrap());
        assert_eq!(current(app.handle()), places);
        drop(server_window);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    /// The same listed server with a sign-in link is checked first; when the check fails, the
    /// error comes back and the old session isn't signed out.
    #[test]
    fn a_link_for_an_unreachable_listed_server_keeps_its_session() {
        let (port, seen) = recording_listener();
        let address = format!("http://127.0.0.1:{port}");
        let origin = target::parse_address(&address).unwrap().origin;
        let mut places = Places::default();
        places.add_server(origin.clone());
        let (app, window, data_dir) = app_with("show-it-link", "connect", places.clone());
        let page = connect::bundled("connect.html").to_string();
        let link = format!("{address}/#kwnt_token=kl1.TESTONLY");
        let refused = invoke(&window, "add_server", &page, serde_json::json!({ "address": link })).unwrap_err();
        assert!(refused["code"].is_string(), "{refused}");
        assert_eq!(refused["host"], origin.host());
        std::thread::sleep(Duration::from_millis(200));
        let seen = seen.lock().unwrap().clone();
        assert!(seen.iter().all(|l| l.starts_with("GET /api/health ")), "only the check: {seen:?}");
        assert!(!seen.is_empty(), "the check was made");
        assert!(!seen.iter().any(|l| l.starts_with("DELETE")), "no sign-out: {seen:?}");
        assert_eq!(current(app.handle()), places);

        // nothing listening at all: the same refusal, still no change
        let gone = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let gone_origin = target::parse_address(&format!("http://127.0.0.1:{gone}")).unwrap().origin;
        let mut places = Places::default();
        places.add_server(gone_origin.clone());
        let (app2, window2, data_dir2) = app_with("show-it-gone", "connect", places.clone());
        let link = format!("http://127.0.0.1:{gone}/#kwnt_token=kl1.TESTONLY");
        let refused = invoke(&window2, "add_server", &page, serde_json::json!({ "address": link })).unwrap_err();
        assert_eq!(refused["code"], "refused", "{refused}");
        assert_eq!(current(app2.handle()), places);
        let _ = std::fs::remove_dir_all(data_dir);
        let _ = std::fs::remove_dir_all(data_dir2);
    }

}
