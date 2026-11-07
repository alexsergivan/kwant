//! kwant for Mac (and Linux): the kwant server in this process, its UI in a window, and a
//! menu-bar (tray) icon. Closing the window keeps kwant running; Quit in the menu stops it.

mod connect;
mod links;
mod nudge;
#[cfg(target_os = "linux")]
mod nudge_linux;
#[cfg(target_os = "macos")]
mod nudge_mac;
mod redact;
mod remote;
mod startup;
mod target;
mod update;

#[cfg(target_os = "linux")]
use nudge_linux as platform;
#[cfg(target_os = "macos")]
use nudge_mac as platform;
use startup::PortChoice;
use std::path::PathBuf;
use target::{Home, Origin, Saved};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    webview::NewWindowResponse,
    AppHandle, Manager, RunEvent, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
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
#[cfg(not(target_os = "macos"))]
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

/// Where the window points: kwant's own page on loopback, or kwant on a saved server.
struct Target {
    home: Home,
    /// The server used last, offered again by the connect page.
    last: Option<Origin>,
    /// In Local mode: this app started kwant (it stops with the app) rather than finding one
    /// already running (`kwnt serve` in a terminal, a service), which keeps running.
    started_kwant: bool,
}

/// A server the connect page is signing in to right now (a pasted sign-in link): the window
/// may open it before the app relaunches on it.
#[derive(Default)]
struct Connecting(Mutex<Option<Origin>>);

/// How far the window got with the server (`nudge::REACH_*`), for the tray's server line.
#[derive(Default)]
struct Reach(AtomicU8);

fn set_reach<R: Runtime>(app: &AppHandle<R>, reach: u8) {
    if let Some(state) = app.try_state::<Reach>() {
        state.0.store(reach, Ordering::Relaxed);
    }
}

/// How this run of the app starts.
struct Start {
    home: Home,
    last: Option<Origin>,
    server: Option<Server>,
    data_dir: PathBuf,
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
    let Start { home, last, server, data_dir } = if demo { demo_start() } else { normal_start() };
    let show = nudge::Prefs::load(&data_dir).show_nudges;
    let remote = matches!(home, Home::Remote { .. });
    let started_kwant = server.is_some();

    let builder = tauri::Builder::default();
    // A second launch from the app menu focuses this window instead of opening another kwant.
    // (macOS does this itself: a second click on the app reaches RunEvent::Reopen.)
    #[cfg(not(target_os = "macos"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        open_window(app)
    }));
    let app = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Target { home, last, started_kwant })
        .manage(Connecting::default())
        .manage(Reach::default())
        .manage(Nudges { data_dir, show: AtomicBool::new(show) })
        .invoke_handler(tauri::generate_handler![
            connect::connect_state,
            connect::check_server,
            connect::connect_server,
            connect::use_this_computer,
            connect::close_connect,
            connect::retry_start,
        ])
        .setup(move |app| {
            if let Some(server) = server {
                app.manage(server);
            }
            // The app's own update stays on with a server too; an attached kwant updates itself
            if !demo && (started_kwant || remote) {
                update::start(app.handle());
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

/// The app's own kwant (or the one already running here), with the person's settings; or,
/// when the person connected the app to a server, nothing local at all.
fn normal_start() -> Start {
    let mut config = match kwnt::Config::load() {
        Ok(c) => c,
        Err(e) => fatal(&format!("kwant couldn't read its settings: {e}")),
    };
    init_logging(&config.data_dir);
    let data_dir = config.data_dir.clone();
    let last = match Saved::load(&data_dir) {
        Saved::Remote(origin) => {
            tracing::info!(host = %origin.host(), "connected to a kwant server; not starting kwant here");
            return Start { home: Home::Remote { origin }, last: None, server: None, data_dir };
        }
        Saved::Local { last } => last,
    };
    // The app is a local window: never listen beyond this machine, whatever config.toml says
    config.host = "127.0.0.1".to_string();
    let choice = match startup::choose_port(config.port) {
        Ok(c) => c,
        Err(e) => fatal(&format!("kwant couldn't find a free port: {e}")),
    };
    match choice {
        PortChoice::Attach(port) => {
            tracing::info!(port, "kwant already runs here; attaching to it");
            Start { home: Home::Local { port }, last, server: None, data_dir }
        }
        PortChoice::Serve(port) => {
            config.port = port;
            Start { home: Home::Local { port }, last, server: Some(start_server(config)), data_dir }
        }
    }
}

/// `--demo-nudge`: kwant on a free port with a throwaway data dir, and made-up waiting items
/// (`nudge::demo`) instead of the real ones, so CI can screenshot the nudges. It prints each
/// step and where the icon and the pill are, and quits after `nudge::demo::RUNS_FOR`.
fn demo_start() -> Start {
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
    Start { home: Home::Local { port }, last: None, server: Some(start_server(config)), data_dir }
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

/// Stops the server this app started before the app exits. An attached kwant keeps running.
fn stop_server<R: Runtime>(handle: &AppHandle<R>) {
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
    if let Some(url) = path.and_then(|p| handle.state::<Target>().home.page(p)) {
        let _ = window.navigate(url);
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
    let home = handle.state::<Target>().home.clone();
    let path = path.unwrap_or("/").to_string();
    let (nav_home, nav_app) = (home.clone(), handle.clone());
    let popup_home = home.clone();
    let window = match WebviewWindowBuilder::new(handle, MAIN, WebviewUrl::App("index.html".into()))
        .title("kwant")
        .visible(visible)
        .inner_size(1280.0, 820.0)
        .min_inner_size(900.0, 600.0)
        .initialization_script(APP_MARK)
        // The window only shows kwant; every other page opens in the default browser
        .on_navigation(move |url| {
            let connecting = nav_app.state::<Connecting>().0.lock().unwrap().clone();
            match links::navigation(url, &nav_home, connecting.as_ref()) {
                links::Link::Stay => true,
                links::Link::Browser => {
                    links::open_in_browser(url);
                    false
                }
                links::Link::Drop => {
                    tracing::info!(scheme = url.scheme(), "blocked a link the app doesn't open");
                    false
                }
            }
        })
        .on_new_window(move |url, _| {
            if links::new_window(&url, &popup_home) == links::Link::Browser {
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
    begin(window, home, path);
}

/// Waits for kwant (here: /api/health on loopback; on a server: the native check), then moves
/// the window from the loading page to kwant's `path`. On failure the loading page says so.
fn begin<R: Runtime>(window: WebviewWindow<R>, home: Home, path: String) {
    std::thread::spawn(move || match &home {
        Home::Local { port } => {
            let port = *port;
            if startup::wait_for_health(port, Duration::from_secs(120)).is_none() {
                tracing::error!(port, "kwant didn't answer within 2 minutes");
                let _ = window.eval("document.body.dataset.state = 'failed'");
                return;
            }
            if let Some(url) = home.page(&path) {
                let _ = window.navigate(url);
            }
        }
        Home::Remote { origin } => {
            let app = window.app_handle().clone();
            set_reach(&app, nudge::REACH_CONNECTING);
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
                    set_reach(&app, nudge::REACH_CONNECTED);
                    let _ = window.navigate(origin.page(&path));
                }
                Err(failure) => {
                    set_reach(&app, nudge::REACH_FAILED);
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
    });
}

/// Back to kwant's own page from a bundled page (the connect page, or the loading page's
/// Try again): the loading page, then kwant once it answers.
fn show_home<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(MAIN) else {
        return;
    };
    let home = app.state::<Target>().home.clone();
    let _ = window.navigate(connect::bundled("index.html"));
    begin(window, home, "/".into());
}

/// Shows the connect page in the kwant window.
fn show_connect(app: &AppHandle) {
    open_window(app);
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.navigate(connect::bundled("connect.html"));
    }
}

/// The window's sign-in cookie for the server, in memory only. Never call it on the main
/// thread: the webview answers there.
fn read_token<R: Runtime>(app: &AppHandle<R>, origin: &Origin) -> Option<remote::Token> {
    let cookies = app.get_webview_window(MAIN)?.cookies().ok()?;
    remote::pick_token(
        origin,
        cookies.iter().map(|c| (c.name(), c.value(), c.domain(), c.path())),
    )
}

/// Removes kwant's sign-in cookie for this server from the window (one left from an earlier
/// session), so it can't pass for the sign-in about to happen.
fn clear_token<R: Runtime>(app: &AppHandle<R>, origin: &Origin) {
    let Some(window) = app.get_webview_window(MAIN) else { return };
    let Ok(cookies) = window.cookies() else { return };
    for cookie in cookies {
        let ours = remote::pick_token(origin, [(cookie.name(), cookie.value(), cookie.domain(), cookie.path())]).is_some();
        if ours && window.delete_cookie(cookie).is_err() {
            tracing::warn!("couldn't clear an old sign-in cookie");
        }
    }
}

/// The menu-bar icon: Open kwant, a status line, Show nudges, the server items, Quit. The status
/// counts what waits for the person (questions and approvals); the nudges (`nudge.rs`) follow the
/// same poll. "Update to x.y.z…" appears at the top while a newer version is on offer.
///
/// Connected to a server, a grey server line follows the window (Connecting to / Connected to /
/// Can't reach), and the count comes from the server with the window's sign-in cookie (read from
/// the webview, kept in memory only).
fn build_tray(handle: &tauri::AppHandle) -> tauri::Result<()> {
    let home = handle.state::<Target>().home.clone();
    let update = MenuItem::with_id(handle, "update", "Update kwant bots…", true, None::<&str>)?;
    let update_separator = PredefinedMenuItem::separator(handle)?;
    let open = MenuItem::with_id(handle, "open", "Open kwant", true, None::<&str>)?;
    let first_line = match &home {
        Home::Local { .. } => "Starting…",
        Home::Remote { .. } => SIGN_IN_LINE,
    };
    let status = MenuItem::with_id(handle, "status", first_line, false, None::<&str>)?;
    let show = handle.state::<Nudges>().show.load(Ordering::Relaxed);
    let nudges = CheckMenuItem::with_id(handle, "nudges", "Show nudges", true, show, None::<&str>)?;
    let quit = MenuItem::with_id(handle, "quit", "Quit kwant", true, Some("CmdOrCtrl+Q"))?;
    let separator = PredefinedMenuItem::separator(handle)?;
    let separator2 = PredefinedMenuItem::separator(handle)?;
    let separator3 = PredefinedMenuItem::separator(handle)?;
    let host = match &home {
        Home::Remote { origin } => origin.host(),
        Home::Local { .. } => String::new(),
    };
    let server = MenuItem::with_id(handle, "server", nudge::server_line(nudge::REACH_CONNECTING, &host), false, None::<&str>)?;
    let menu = match &home {
        Home::Local { .. } => {
            let connect = MenuItem::with_id(handle, "connect", "Connect to a server…", true, None::<&str>)?;
            Menu::with_items(handle, &[&open, &status, &separator, &nudges, &separator2, &connect, &separator3, &quit])?
        }
        Home::Remote { .. } => {
            let change = MenuItem::with_id(handle, "connect", "Change server…", true, None::<&str>)?;
            let local = MenuItem::with_id(handle, "local", "Use kwant on this computer", true, None::<&str>)?;
            Menu::with_items(
                handle,
                &[&open, &status, &separator, &nudges, &separator2, &server, &change, &local, &separator3, &quit],
            )?
        }
    };
    let icon = tauri::image::Image::from_bytes(TRAY_ICON)?;
    let tray = TrayIconBuilder::with_id("kwant")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip(match &home {
            Home::Remote { origin } => format!("kwant – {}", origin.host()),
            Home::Local { .. } => "kwant".to_string(),
        })
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
            "connect" => show_connect(app),
            "local" => {
                if let Home::Remote { origin } = &app.state::<Target>().home {
                    if connect::use_this_computer_from(app, origin.clone()).is_err() {
                        show_connect(app);
                    }
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(handle)?;

    let hub = kwnt::app_update::UpdateHub::shared();
    let app = handle.clone();
    std::thread::spawn(move || {
        let mut shown = false;
        let mut checked = show;
        let mut nudger = platform::Nudger::start(&app, tray.clone());
        let mut tracker = nudge::Tracker::default();
        let mut demo_step = 0;
        let mut reach_shown = nudge::REACH_CONNECTING;
        let mut poll = match &home {
            Home::Remote { origin } => Some(RemotePoll::new(origin.clone())),
            Home::Local { .. } => None,
        };
        loop {
            let demo = DEMO.get().map(|t| nudge::demo::step(t.elapsed()));
            let waiting = match (demo, &home, poll.as_mut()) {
                (Some(step), _, _) => Some(nudge::demo::waiting(step)),
                (None, _, Some(poll)) => poll.poll(&app),
                (None, Home::Local { port }, None) => poll_waiting(*port),
                (None, Home::Remote { .. }, None) => None,
            };
            let show = app.state::<Nudges>().show.load(Ordering::Relaxed);
            if show != checked {
                checked = nudges.set_checked(show).map(|_| show).unwrap_or(checked);
            }
            let (line, badge) = match (&waiting, &home) {
                (Some(w), Home::Local { port }) => nudge::status_line(w, *port),
                (Some(w), Home::Remote { .. }) => nudge::remote_status_line(w),
                (None, Home::Local { .. }) => ("Starting…".to_string(), None),
                (None, Home::Remote { .. }) => (SIGN_IN_LINE.to_string(), None),
            };
            if matches!(home, Home::Remote { .. }) {
                let reach = app.state::<Reach>().0.load(Ordering::Relaxed);
                if reach != reach_shown && server.set_text(nudge::server_line(reach, &host)).is_ok() {
                    reach_shown = reach;
                }
            }
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
            let offer = update::menu_item(&hub.status());
            // in Remote mode "Update" alone could read as updating the server
            let offer = match &home {
                Home::Remote { .. } => offer.map(|(text, on)| (text.replacen("Update to ", "Update this app to ", 1), on)),
                Home::Local { .. } => offer,
            };
            match offer {
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

/// The status line on a server before the app has the window's sign-in cookie.
const SIGN_IN_LINE: &str = "Sign in to see what's waiting";

/// The tray's poll of kwant on a server, signed in with the window's cookie. Without the
/// cookie (not signed in yet, or the webview won't say) it gives `None`: the menu then reads
/// "Connected to <host>" with no count and no nudges.
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

    fn poll(&mut self, app: &AppHandle) -> Option<nudge::Waiting> {
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
                return None;
            }
            remote::Answer::Failed => return None,
        };
        let (client, origin, token) = (&self.client, &self.origin, self.token.as_ref());
        let ok = |path: &str| match tauri::async_runtime::block_on(remote::get(client, origin, token, path)) {
            remote::Answer::Ok(body) => Some(body),
            _ => None,
        };
        Some(collect_waiting(Some(spaces), ok))
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
        "connect_state",
        "check_server",
        "connect_server",
        "use_this_computer",
        "close_connect",
        "retry_start",
    ];

    /// The app's commands and its real capabilities, on Tauri's mock runtime.
    fn app(name: &str) -> (tauri::App<MockRuntime>, WebviewWindow<MockRuntime>, PathBuf) {
        let data_dir = std::env::temp_dir().join(format!("kwant-ipc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data_dir);
        let app = mock_builder()
            .manage(Target { home: Home::Local { port: 1 }, last: None, started_kwant: true })
            .manage(Connecting::default())
            .manage(Reach::default())
            .manage(Nudges { data_dir: data_dir.clone(), show: AtomicBool::new(true) })
            .invoke_handler(tauri::generate_handler![
                connect::connect_state,
                connect::check_server,
                connect::connect_server,
                connect::use_this_computer,
                connect::close_connect,
                connect::retry_start,
            ])
            .build(tauri::generate_context!(test = true))
            .expect("the mock app");
        let window = WebviewWindowBuilder::new(&app, MAIN, WebviewUrl::App("index.html".into()))
            .build()
            .expect("the mock window");
        (app, window, data_dir)
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

    #[test]
    fn a_server_s_pages_can_call_no_command() {
        let (_app, window, data_dir) = app("remote");
        let args = serde_json::json!({ "address": "https://kwant.example.com" });
        for from in [
            "https://kwant.example.com/",
            "https://kwant.example.com/s/spc_default/inbox",
            "http://127.0.0.1:2222/",
            "https://localhost/",
            "https://tauri.localhost.evil.example/",
        ] {
            for cmd in COMMANDS {
                let refused = invoke(&window, cmd, from, args.clone());
                // the access control answers (a plain message), the command never runs
                assert!(
                    matches!(&refused, Err(serde_json::Value::String(_))),
                    "{cmd} from {from}: {refused:?}"
                );
            }
        }
        assert!(!data_dir.join(target::TARGET_FILE).exists(), "nothing was saved");
        let _ = std::fs::remove_dir_all(data_dir);
    }

    /// Rigel's F7: Tauri's access check compares only the scheme and the host, so a page at the
    /// bundled host with a port or a user name gets past it. The command's own check then
    /// refuses it, and nothing is saved.
    #[test]
    fn a_bundled_look_alike_with_a_port_or_a_user_name_can_call_no_command() {
        let (_app, window, data_dir) = app("lookalike");
        let args = serde_json::json!({ "address": "https://kwant.example.com" });
        for from in [
            "http://tauri.localhost:8080/",
            "tauri://localhost:8080/",
            "http://u:p@tauri.localhost/",
            "tauri://u@localhost/",
        ] {
            window.navigate(from.parse().unwrap()).unwrap();
            for cmd in COMMANDS {
                let refused = invoke(&window, cmd, from, args.clone());
                let by_acl = matches!(&refused, Err(serde_json::Value::String(_)));
                let by_command = matches!(&refused, Err(e) if e["code"] == "not_allowed");
                assert!(by_acl || by_command, "{cmd} from {from}: {refused:?}");
            }
        }
        assert!(!data_dir.join(target::TARGET_FILE).exists(), "nothing was saved");
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn the_bundled_pages_can() {
        let (_app, window, data_dir) = app("bundled");
        let state = invoke(&window, "connect_state", &connect::bundled("connect.html").to_string(), serde_json::json!({}))
            .expect("connect_state from the connect page");
        assert_eq!(state["mode"], "local");
        // the command itself runs (and refuses plain http before contacting anything)
        let refused = invoke(
            &window,
            "check_server",
            &connect::bundled("connect.html").to_string(),
            serde_json::json!({ "address": "http://kwant.example.com" }),
        )
        .unwrap_err();
        assert_eq!(refused["code"], "http_not_allowed", "{refused}");
        let _ = std::fs::remove_dir_all(data_dir);
    }
}
