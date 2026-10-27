//! kwant for Mac: the kwant server in this process, its UI in a window, and a menu-bar icon.
//! Closing the window keeps kwant running; Quit in the menu stops it.

mod links;
mod startup;
mod update;

use startup::PortChoice;
use std::sync::{mpsc, Mutex};
use std::time::Duration;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    webview::NewWindowResponse,
    Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

const MAIN: &str = "main";

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

fn main() {
    fix_path();
    if std::env::args().any(|a| a == "--check") {
        std::process::exit(check());
    }

    let mut config = match kwnt::Config::load() {
        Ok(c) => c,
        Err(e) => fatal(&format!("kwant couldn't read its settings: {e}")),
    };
    init_logging(&config.data_dir);
    // The app is a local window: never listen beyond this machine, whatever config.toml says
    config.host = "127.0.0.1".to_string();
    let choice = match startup::choose_port(config.port) {
        Ok(c) => c,
        Err(e) => fatal(&format!("kwant couldn't find a free port: {e}")),
    };
    let (port, server) = match choice {
        PortChoice::Attach(port) => {
            tracing::info!(port, "kwant already runs here; attaching to it");
            (port, None)
        }
        PortChoice::Serve(port) => {
            config.port = port;
            (port, Some(start_server(config)))
        }
    };

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Target { port })
        .setup(move |app| {
            if let Some(server) = server {
                app.manage(server);
                update::start(app.handle());
            }
            build_tray(app.handle())?;
            open_window(app.handle());
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
        RunEvent::Exit => stop_server(handle),
        _ => {}
    });
}

/// Finder starts apps with a bare PATH, so the claude, codex and grok CLIs would be missing.
/// Runs before any thread exists, so setting the variable is safe.
fn fix_path() {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
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
fn stop_server(handle: &tauri::AppHandle) {
    if let Some(server) = handle.try_state::<Server>() {
        server.shutdown();
    }
}

/// Shows the window, creating it the first time. It opens on a "Starting kwant…" page and moves
/// to kwant's own page once /api/health answers.
fn open_window(handle: &tauri::AppHandle) {
    if let Some(window) = handle.get_webview_window(MAIN) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    let port = handle.state::<Target>().port;
    let window = match WebviewWindowBuilder::new(handle, MAIN, WebviewUrl::App("index.html".into()))
        .title("kwant")
        .inner_size(1280.0, 820.0)
        .min_inner_size(900.0, 600.0)
        // Tells kwant's page it runs in the app, so sign-ins say "finish in your browser"
        .initialization_script("window.__KWANT_APP__ = 'mac';")
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
        let url = format!("http://127.0.0.1:{port}/");
        if let Ok(url) = url.parse() {
            let _ = window.navigate(url);
        }
    });
}

/// The menu-bar icon: Open kwant, a status line, Quit. The status counts approvals waiting.
/// "Update to x.y.z…" appears at the top while a newer version is on offer.
fn build_tray(handle: &tauri::AppHandle) -> tauri::Result<()> {
    let update = MenuItem::with_id(handle, "update", "Update kwant bots…", true, None::<&str>)?;
    let update_separator = PredefinedMenuItem::separator(handle)?;
    let open = MenuItem::with_id(handle, "open", "Open kwant", true, None::<&str>)?;
    let status = MenuItem::with_id(handle, "status", "Starting…", false, None::<&str>)?;
    let quit = MenuItem::with_id(handle, "quit", "Quit kwant", true, Some("CmdOrCtrl+Q"))?;
    let separator = PredefinedMenuItem::separator(handle)?;
    let menu = Menu::with_items(handle, &[&open, &status, &separator, &quit])?;
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))?;
    let tray = TrayIconBuilder::with_id("kwant")
        .icon(icon)
        .icon_as_template(true)
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
            "quit" => app.exit(0),
            _ => {}
        })
        .build(handle)?;

    let port = handle.state::<Target>().port;
    let hub = kwnt::app_update::UpdateHub::shared();
    std::thread::spawn(move || {
        let mut shown = false;
        loop {
            let (line, badge) = status_line(port);
            let _ = status.set_text(line);
            let _ = tray.set_title(badge.as_deref());
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
            std::thread::sleep(Duration::from_secs(5));
        }
    });
    Ok(())
}

/// The menu's status line and the number shown next to the icon (`None`: nothing waiting).
fn status_line(port: u16) -> (String, Option<String>) {
    if !startup::is_kwant(port) {
        return ("Starting…".to_string(), None);
    }
    let waiting = startup::http_get(port, "/api/approvals?status=pending", Duration::from_secs(2))
        .filter(|(status, _)| *status == 200)
        .and_then(|(_, body)| serde_json::from_str::<Vec<serde_json::Value>>(&body).ok())
        .map(|rows| rows.len());
    match waiting {
        Some(0) | None => (format!("Running on port {port}"), None),
        Some(1) => ("1 approval waiting for you".to_string(), Some("1".to_string())),
        Some(n) => (format!("{n} approvals waiting for you"), Some(n.to_string())),
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

/// Logs go to ~/.kwnt/mac-app.log: an app opened from Finder has no terminal.
fn init_logging(data_dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(data_dir);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir.join("mac-app.log"));
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
