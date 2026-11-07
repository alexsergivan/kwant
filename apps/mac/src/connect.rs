//! The commands behind "Connect to a server" (api.md in the connect-server folder describes
//! them for the page). Only the bundled pages can call them: the capability in
//! capabilities/connect.json is local-only, and each command that checks or saves also looks at
//! the address of the page that called it. A server's own pages get no commands at all.

use crate::target::{parse_address, Home, Origin, Saved};
use crate::{links, remote, Connecting, Nudges, Target, MAIN};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Runtime, Url, Webview};

/// How long the app waits for a server to set its sign-in cookie after a sign-in link.
const SIGN_IN_WAIT: Duration = Duration::from_secs(45);
/// Lets the page show "Restarting…" before the app goes.
const RESTART_DELAY: Duration = Duration::from_millis(400);

/// A bundled page: `connect.html`, `index.html`.
pub fn bundled(page: &str) -> Url {
    let base = if cfg!(any(windows, target_os = "android")) { "http://tauri.localhost" } else { "tauri://localhost" };
    format!("{base}/{page}").parse().expect("a bundled page address")
}

fn error(code: &str, message: impl Into<String>, host: Option<String>) -> Value {
    json!({ "code": code, "message": message.into(), "host": host })
}

/// The capability already keeps server pages out; this is the second lock on the same door.
fn bundled_only<R: Runtime>(webview: &Webview<R>) -> Result<(), Value> {
    match webview.url() {
        Ok(url) if links::is_bundled(&url) => Ok(()),
        _ => Err(error("not_allowed", "Only the app's own pages can do this.", None)),
    }
}

fn app_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "mac"
    } else {
        "linux"
    }
}

/// Where the app points now, and the server used last.
#[tauri::command]
pub fn connect_state<R: Runtime>(app: AppHandle<R>, webview: Webview<R>) -> Result<Value, Value> {
    bundled_only(&webview)?;
    let target = app.state::<Target>();
    let (mode, origin) = match &target.home {
        Home::Local { .. } => ("local", None),
        Home::Remote { origin } => ("remote", Some(origin)),
    };
    let last = target.last.as_ref().or(origin);
    Ok(json!({
        "mode": mode,
        "origin": origin.map(Origin::as_str),
        "host": origin.map(Origin::host),
        "last_origin": last.map(Origin::as_str),
        "last_host": last.map(Origin::host),
        // Local mode: "app" when this app started kwant (its bots pause while connected),
        // "outside" when kwant was already running (it keeps running)
        "local_kwant": match (&target.home, target.started_kwant) {
            (Home::Local { .. }, true) => Some("app"),
            (Home::Local { .. }, false) => Some("outside"),
            (Home::Remote { .. }, _) => None,
        },
        "app": app_name(),
        "version": app.package_info().version.to_string(),
    }))
}

/// Reads and checks an address without saving anything.
async fn checked(address: &str) -> Result<(crate::target::Address, String), Value> {
    let address = parse_address(address).map_err(|r| error(r.code(), r.message(), None))?;
    let host = address.origin.host();
    let version = remote::check(&remote::client(), &address.origin)
        .await
        .map_err(|f| error(f.code(), f.message(&host), Some(host.clone())))?;
    tracing::info!(host = %host, version = %version, "a kwant server answered");
    Ok((address, version))
}

/// Checks that an address is a kwant server (its /api/health answers as kwant).
#[tauri::command]
pub async fn check_server<R: Runtime>(webview: Webview<R>, address: String) -> Result<Value, Value> {
    bundled_only(&webview)?;
    let (address, version) = checked(&address).await?;
    Ok(json!({
        "origin": address.origin.as_str(),
        "host": address.origin.host(),
        "version": version,
        "sign_in_link": address.sign_in.is_some(),
    }))
}

/// Checks the server, saves its origin and relaunches the app on it. With a pasted sign-in
/// link, the window first opens the link once, so the server sets its cookie, and the app
/// relaunches once the cookie is there (or after `SIGN_IN_WAIT`).
#[tauri::command]
pub async fn connect_server<R: Runtime>(app: AppHandle<R>, webview: Webview<R>, address: String) -> Result<Value, Value> {
    bundled_only(&webview)?;
    let (address, _) = checked(&address).await?;
    let origin = address.origin;
    let host = origin.host();
    if let Err(e) = Saved::Remote(origin.clone()).save(&app.state::<Nudges>().data_dir) {
        tracing::warn!(error = %e, "couldn't save the server choice");
        return Err(error("save_failed", "kwant couldn't save the server. Check that ~/.kwnt can be written.", Some(host)));
    }
    tracing::info!(host = %host, "connecting to a kwant server; relaunching");
    let signing_in = address.sign_in.is_some();
    match (address.sign_in, app.get_webview_window(MAIN)) {
        (Some(link), Some(window)) => {
            *app.state::<Connecting>().0.lock().unwrap() = Some(origin.clone());
            // a cookie from an earlier session would end the wait at the first look, before
            // the link has even loaded: clear it, and ignore one that couldn't be cleared
            crate::clear_token(&app, &origin);
            let stale = crate::read_token(&app, &origin);
            if window.navigate(link.url().clone()).is_err() {
                tracing::warn!(host = %host, "couldn't open the sign-in link");
            }
            std::thread::spawn(move || {
                let started = Instant::now();
                while started.elapsed() < SIGN_IN_WAIT {
                    std::thread::sleep(Duration::from_millis(500));
                    if crate::read_token(&app, &origin).is_some_and(|t| Some(&t) != stale.as_ref()) {
                        tracing::info!(host = %host, "signed in");
                        break;
                    }
                }
                std::thread::sleep(RESTART_DELAY);
                app.restart();
            });
        }
        _ => {
            std::thread::spawn(move || {
                std::thread::sleep(RESTART_DELAY);
                app.restart();
            });
        }
    }
    Ok(json!({ "restarting": true, "signing_in": signing_in }))
}

/// Back to the kwant on this computer. In Remote mode it saves that and relaunches; in Local
/// mode it just leaves the connect page.
#[tauri::command]
pub fn use_this_computer<R: Runtime>(app: AppHandle<R>, webview: Webview<R>) -> Result<Value, Value> {
    bundled_only(&webview)?;
    let origin = match &app.state::<Target>().home {
        Home::Remote { origin } => origin.clone(),
        Home::Local { .. } => {
            crate::show_home(&app);
            return Ok(json!({ "restarting": false }));
        }
    };
    use_this_computer_from(&app, origin)?;
    Ok(json!({ "restarting": true }))
}

/// Saves "this computer" (keeping the server as the last one) and relaunches.
pub fn use_this_computer_from<R: Runtime>(app: &AppHandle<R>, origin: Origin) -> Result<(), Value> {
    if let Err(e) = (Saved::Local { last: Some(origin) }).save(&app.state::<Nudges>().data_dir) {
        tracing::warn!(error = %e, "couldn't save the choice of this computer");
        return Err(error("save_failed", "kwant couldn't save the choice. Check that ~/.kwnt can be written.", None));
    }
    tracing::info!("switching to the kwant on this computer; relaunching");
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(RESTART_DELAY);
        app.restart();
    });
    Ok(())
}

/// Leaves the connect page for kwant's own page, unchanged.
#[tauri::command]
pub fn close_connect<R: Runtime>(app: AppHandle<R>, webview: Webview<R>) -> Result<(), Value> {
    bundled_only(&webview)?;
    crate::show_home(&app);
    Ok(())
}

/// The loading page's Try again: checks the server (or the local kwant) again.
#[tauri::command]
pub fn retry_start<R: Runtime>(app: AppHandle<R>, webview: Webview<R>) -> Result<(), Value> {
    bundled_only(&webview)?;
    crate::show_home(&app);
    Ok(())
}
