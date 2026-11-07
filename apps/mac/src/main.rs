//! kwant for Mac (and Linux): the kwant server in this process, its UI in a window, and a
//! menu-bar (tray) icon. Closing the window keeps kwant running; Quit in the menu stops it.
//! On Windows (0.7.6 beta) the icon sits in the notification area.

// No console window behind the app on Windows
#![cfg_attr(windows, windows_subsystem = "windows")]

mod links;
mod nudge;
#[cfg(target_os = "linux")]
mod nudge_linux;
#[cfg(target_os = "macos")]
mod nudge_mac;
#[cfg(windows)]
mod nudge_windows;
mod startup;
mod update;

#[cfg(target_os = "linux")]
use nudge_linux as platform;
#[cfg(target_os = "macos")]
use nudge_mac as platform;
#[cfg(windows)]
use nudge_windows as platform;
use startup::PortChoice;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    webview::NewWindowResponse,
    Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

const MAIN: &str = "main";

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
#[cfg(not(any(target_os = "macos", windows)))]
const FALLBACK_SHELL: &str = "/bin/bash";

/// The app's own log in the data dir.
#[cfg(target_os = "macos")]
const LOG_FILE: &str = "mac-app.log";
#[cfg(not(target_os = "macos"))]
const LOG_FILE: &str = "desktop-app.log";

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

/// Where the window points: kwant's own page on loopback.
struct Target {
    port: u16,
}

/// "Show nudges" in the tray menu, saved in the data dir.
struct Nudges {
    data_dir: PathBuf,
    show: AtomicBool,
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
    let (port, server, data_dir) = if demo { demo_start() } else { normal_start() };
    let show = nudge::Prefs::load(&data_dir).show_nudges;

    let builder = tauri::Builder::default();
    // A second launch from the app menu focuses this window instead of opening another kwant.
    // (macOS does this itself: a second click on the app reaches RunEvent::Reopen.)
    #[cfg(not(target_os = "macos"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        open_window(app)
    }));
    let app = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Target { port })
        .manage(Nudges { data_dir, show: AtomicBool::new(show) })
        .setup(move |app| {
            if let Some(server) = server {
                app.manage(server);
                // In-app updates are off on Windows in the beta (no signed Windows release yet)
                if !demo && !cfg!(windows) {
                    update::start(app.handle());
                }
            }
            build_tray(app.handle())?;
            if demo {
                // hidden, so the pill shows (it never covers a focused kwant window); a click on
                // the pill shows it
                make_window(app.handle(), None, false);
                let h = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(nudge::demo::RUNS_FOR);
                    println!("demo-nudge: done");
                    h.exit(0);
                });
            } else {
                open_window(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Closing the window keeps kwant running in the menu bar
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .unwrap_or_else(|e| fatal(&format!("kwant couldn't open its window: {e}")));

    app.run(|handle, event| match event {
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => open_window(handle),
        RunEvent::Exit => {
            stop_server(handle);
            if DEMO.get().is_some() {
                let _ = std::fs::remove_dir_all(&handle.state::<Nudges>().data_dir);
            }
        }
        _ => {}
    });
}

/// The app's own kwant (or the one already running here), with the person's settings.
fn normal_start() -> (u16, Option<Server>, PathBuf) {
    let mut config = match kwnt::Config::load() {
        Ok(c) => c,
        Err(e) => fatal(&format!("kwant couldn't read its settings: {e}")),
    };
    init_logging(&config.data_dir);
    let data_dir = config.data_dir.clone();
    // The app is a local window: never listen beyond this machine, whatever config.toml says
    config.host = "127.0.0.1".to_string();
    let choice = match startup::choose_port(config.port) {
        Ok(c) => c,
        Err(e) => fatal(&format!("kwant couldn't find a free port: {e}")),
    };
    match choice {
        PortChoice::Attach(port) => {
            tracing::info!(port, "kwant already runs here; attaching to it");
            (port, None, data_dir)
        }
        PortChoice::Serve(port) => {
            config.port = port;
            (port, Some(start_server(config)), data_dir)
        }
    }
}

/// `--demo-nudge`: kwant on a free port with a throwaway data dir, and made-up waiting items
/// (`nudge::demo`) instead of the real ones, so CI can screenshot the nudges. It prints each
/// step and where the icon and the pill are, and quits after `nudge::demo::RUNS_FOR`.
fn demo_start() -> (u16, Option<Server>, PathBuf) {
    let _ = DEMO.set(Instant::now());
    tracing_subscriber::fmt().with_env_filter("info").init();
    let data_dir = std::env::temp_dir().join(format!("kwant-demo-nudge-{}", std::process::id()));
    let port = startup::free_port().unwrap_or_else(|e| fatal(&format!("kwant --demo-nudge: no free port: {e}")));
    let config = kwnt::Config {
        data_dir: data_dir.clone(),
        host: "127.0.0.1".to_string(),
        port,
        laya_managed: false,
        ..Default::default()
    };
    println!("demo-nudge: port {port}");
    (port, Some(start_server(config)), data_dir)
}

/// Finder (and a Linux app menu) starts apps with a bare PATH, so the claude, codex and grok CLIs
/// would be missing. Runs before any thread exists, so setting the variable is safe.
#[cfg(not(windows))]
fn fix_path() {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| FALLBACK_SHELL.to_string());
    let login = startup::login_shell_path(&shell, Duration::from_secs(3));
    let current = std::env::var("PATH").unwrap_or_default();
    let home = dirs::home_dir().unwrap_or_default();
    std::env::set_var("PATH", startup::merged_path(login.as_deref(), &current, &home));
}

/// Windows starts apps with the person's full PATH from the registry, so no login shell is
/// asked (Git Bash would answer in /c/... form). Only npm's and the native Claude install's
/// folders are added, in case a fresh install hasn't reached this session's PATH yet.
#[cfg(windows)]
fn fix_path() {
    let current = std::env::var("PATH").unwrap_or_default();
    let appdata = std::env::var("APPDATA").unwrap_or_default();
    let home = dirs::home_dir().unwrap_or_default();
    std::env::set_var("PATH", startup::windows_path(&current, &appdata, &home.display().to_string()));
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

/// Stops the server this app started before the app exits. An attached kwant keeps running.
fn stop_server(handle: &tauri::AppHandle) {
    if let Some(server) = handle.try_state::<Server>() {
        server.shutdown();
    }
}

/// Shows the window, creating it the first time. It opens on a "Starting kwant…" page and moves
/// to kwant's own page once /api/health answers.
fn open_window(handle: &tauri::AppHandle) {
    open_window_at(handle, None)
}

/// Shows the window at one of kwant's pages (`path`, e.g. `/s/spc_default/inbox`): a nudge's
/// item. `None` keeps the page it shows.
fn open_window_at(handle: &tauri::AppHandle, path: Option<&str>) {
    let Some(window) = handle.get_webview_window(MAIN) else {
        make_window(handle, path, true);
        return;
    };
    if let Some(path) = path {
        let port = handle.state::<Target>().port;
        if let Ok(url) = format!("http://127.0.0.1:{port}{path}").parse() {
            let _ = window.navigate(url);
        }
    }
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

/// Whether the kwant window has the keyboard (the nudges then stay quiet).
fn main_focused(handle: &tauri::AppHandle) -> bool {
    handle
        .get_webview_window(MAIN)
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false)
}

fn make_window(handle: &tauri::AppHandle, path: Option<&str>, visible: bool) {
    let port = handle.state::<Target>().port;
    let path = path.unwrap_or("/").to_string();
    let window = match WebviewWindowBuilder::new(handle, MAIN, WebviewUrl::App("index.html".into()))
        .title("kwant")
        .visible(visible)
        .inner_size(1280.0, 820.0)
        .min_inner_size(900.0, 600.0)
        .initialization_script(APP_MARK)
        // The window only shows kwant; every other page opens in the default browser
        .on_navigation(move |url| match links::navigation(url, port) {
            links::Link::Stay => true,
            links::Link::Browser => {
                links::open_in_browser(url);
                false
            }
            links::Link::Drop => {
                tracing::info!(scheme = url.scheme(), "blocked a link the app doesn't open");
                false
            }
        })
        .on_new_window(move |url, _| {
            if links::new_window(&url, port) == links::Link::Browser {
                links::open_in_browser(&url);
            }
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
    std::thread::spawn(move || {
        if startup::wait_for_health(port, Duration::from_secs(120)).is_none() {
            tracing::error!(port, "kwant didn't answer within 2 minutes");
            let _ = window.eval("document.body.dataset.state = 'failed'");
            return;
        }
        let url = format!("http://127.0.0.1:{port}{path}");
        if let Ok(url) = url.parse() {
            let _ = window.navigate(url);
        }
    });
}

/// The menu-bar icon: Open kwant, a status line, Show nudges, Quit. The status counts what waits
/// for the person (questions and approvals); the nudges (`nudge.rs`) follow the same poll.
/// "Update to x.y.z…" appears at the top while a newer version is on offer.
fn build_tray(handle: &tauri::AppHandle) -> tauri::Result<()> {
    let update = MenuItem::with_id(handle, "update", "Update kwant bots…", true, None::<&str>)?;
    let update_separator = PredefinedMenuItem::separator(handle)?;
    let open = MenuItem::with_id(handle, "open", "Open kwant", true, None::<&str>)?;
    let status = MenuItem::with_id(handle, "status", "Starting…", false, None::<&str>)?;
    let show = handle.state::<Nudges>().show.load(Ordering::Relaxed);
    let nudges = CheckMenuItem::with_id(handle, "nudges", "Show nudges", true, show, None::<&str>)?;
    let quit = MenuItem::with_id(handle, "quit", "Quit kwant", true, Some("CmdOrCtrl+Q"))?;
    let separator = PredefinedMenuItem::separator(handle)?;
    let separator2 = PredefinedMenuItem::separator(handle)?;
    let menu = Menu::with_items(handle, &[&open, &status, &separator, &nudges, &separator2, &quit])?;
    let icon = tauri::image::Image::from_bytes(TRAY_ICON)?;
    let tray = TrayIconBuilder::with_id("kwant")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("kwant")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => open_window(app),
            "update" => {
                // The window shows the download and the relaunch
                if let Err(e) = kwnt::app_update::UpdateHub::shared().install() {
                    tracing::info!(reason = e.message(), "update from the menu not started");
                }
                open_window(app);
            }
            "nudges" => {
                let state = app.state::<Nudges>();
                let on = !state.show.load(Ordering::Relaxed);
                state.show.store(on, Ordering::Relaxed);
                if let Err(e) = (nudge::Prefs { show_nudges: on }).save(&state.data_dir) {
                    tracing::warn!(error = %e, "couldn't save Show nudges");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(handle)?;

    let port = handle.state::<Target>().port;
    let hub = kwnt::app_update::UpdateHub::shared();
    let app = handle.clone();
    std::thread::spawn(move || {
        let mut shown = false;
        let mut checked = show;
        let mut nudger = platform::Nudger::start(&app, tray.clone());
        let mut tracker = nudge::Tracker::default();
        let mut demo_step = 0;
        loop {
            let demo = DEMO.get().map(|t| nudge::demo::step(t.elapsed()));
            let waiting = match demo {
                Some(step) => Some(nudge::demo::waiting(step)),
                None => poll_waiting(port),
            };
            let show = app.state::<Nudges>().show.load(Ordering::Relaxed);
            if show != checked {
                checked = nudges.set_checked(show).map(|_| show).unwrap_or(checked);
            }
            let (line, badge) = match &waiting {
                Some(w) => nudge::status_line(w, port),
                None => ("Starting…".to_string(), None),
            };
            let _ = status.set_text(&line);
            let _ = tray.set_title(badge.as_deref());
            if let Some(w) = &waiting {
                let change = tracker.update(w, Instant::now());
                nudger.on_poll(&app, &change, show);
            }
            if let Some(step) = demo.filter(|s| *s != demo_step) {
                demo_step = step;
                println!("demo-nudge: step {step}: {line}");
            }
            match update::menu_item(&hub.status()) {
                Some((text, enabled)) => {
                    if !shown {
                        shown = menu.insert_items(&[&update, &update_separator], 0).is_ok();
                    }
                    let _ = update.set_text(text);
                    let _ = update.set_enabled(enabled);
                }
                None if shown => {
                    let _ = menu.remove(&update);
                    let _ = menu.remove(&update_separator);
                    shown = false;
                }
                None => {}
            }
            // the demo moves on a timeline CI screenshots, so it looks every second
            std::thread::sleep(Duration::from_secs(if demo.is_some() { 1 } else { 5 }));
        }
    });
    Ok(())
}

/// What waits for the person in every space (`None`: kwant isn't answering yet). A kwant too old
/// for /api/away still gets its approvals counted, without nudges.
fn poll_waiting(port: u16) -> Option<nudge::Waiting> {
    if !startup::is_kwant(port) {
        return None;
    }
    let get = |path: &str| {
        startup::http_get(port, path, Duration::from_secs(2))
            .filter(|(status, _)| *status == 200)
            .map(|(_, body)| body)
    };
    let spaces = get("/api/spaces").and_then(|b| nudge::parse_spaces(&b));
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
    Some(all)
}

/// Platforms without a nudge of their own (the app ships for macOS, Linux and Windows) keep the count.
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
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
/// app menu has no terminal.
fn init_logging(data_dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(data_dir);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir.join(LOG_FILE));
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    match file {
        Ok(file) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(Mutex::new(file))
            .init(),
        Err(_) => tracing_subscriber::fmt().with_env_filter(filter).init(),
    }
}

fn fatal(message: &str) -> ! {
    eprintln!("{message}");
    tracing::error!("{message}");
    std::process::exit(1);
}
