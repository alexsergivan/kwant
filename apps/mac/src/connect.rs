//! The commands behind the bundled pages: "Add a server" (connect.html) and "Starting kwant…"
//! (index.html). spec.md in next-075/side-by-side lists them. Only the bundled pages can call
//! them: the capability in capabilities/connect.json is local-only, and each command also looks
//! at the address of the page that called it. A server's own pages get no commands at all.

use crate::places::{Place, THIS_MAC};
use crate::target::{parse_address, Origin};
use crate::{links, remote, Shared, ToggleRefused, CONNECT};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager, Runtime, Url, Webview};

/// A bundled page: `connect.html`, `index.html`.
pub fn bundled(page: &str) -> Url {
    let base = if cfg!(any(windows, target_os = "android")) { "http://tauri.localhost" } else { "tauri://localhost" };
    format!("{base}/{page}").parse().expect("a bundled page address")
}

pub(crate) fn error(code: &str, message: impl Into<String>, host: Option<String>) -> Value {
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

/// Every place, which windows show, and the place of the window that asks.
#[tauri::command]
pub fn places_state<R: Runtime>(app: AppHandle<R>, webview: Webview<R>) -> Result<Value, Value> {
    bundled_only(&webview)?;
    let places = crate::current(&app);
    let shared = app.state::<Shared>();
    let reach = shared.reach.lock().unwrap().clone();
    let shown = |place: &Place| {
        app.get_webview_window(&place.window_label())
            .and_then(|w| w.is_visible().ok())
            .unwrap_or(false)
    };
    let servers: Vec<Value> = places
        .servers
        .iter()
        .map(|o| {
            json!({
                "origin": o.as_str(),
                "host": o.host(),
                "open": shown(&Place::Server(o.clone())),
                "reach": reach.get(&o.as_str()).map(|r| r.as_str()),
            })
        })
        .collect();
    let place = places.by_window(webview.label()).map(|p| match &p {
        Place::ThisMac => json!({ "key": THIS_MAC, "kind": "this_mac", "host": null }),
        Place::Server(o) => json!({ "key": o.as_str(), "kind": "server", "host": o.host() }),
    });
    let local_kwant = {
        let local = shared.local.lock().unwrap();
        match (places.this_mac, local.port, local.server.is_some()) {
            (true, Some(_), true) => Some("app"),
            (true, Some(_), false) => Some("outside"),
            _ => None,
        }
    };
    Ok(json!({
        "this_mac": places.this_mac,
        "servers": servers,
        "last_origin": places.servers.last().map(Origin::as_str),
        "place": place,
        "local_kwant": local_kwant,
        "started_kwant": shared.started_kwant.load(Ordering::Relaxed),
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

/// Checks the server, lists it and opens its window (a listed one is focused). A pasted
/// sign-in link opens once in that window, so the server sets its cookie there. The Add a
/// server window closes. Nothing relaunches.
///
/// A listed server without a link ("Show it", design.md §3.1) isn't checked: its window is
/// focused and shows its own failure page if the server is down. With a link it is checked
/// first, so an unreachable server never loses its old session.
#[tauri::command]
pub async fn add_server<R: Runtime>(app: AppHandle<R>, webview: Webview<R>, address: String) -> Result<Value, Value> {
    bundled_only(&webview)?;
    let parsed = parse_address(&address).map_err(|r| error(r.code(), r.message(), None))?;
    let listed = crate::current(&app).servers.contains(&parsed.origin);
    let address = if listed && parsed.sign_in.is_none() { parsed } else { checked(&address).await?.0 };
    let origin = address.origin;
    let host = origin.host();
    let link = address.sign_in.map(|l| l.url().clone());
    if link.is_some() && listed {
        // a sign-in link signs a listed server's window in again: end the old session first,
        // so Signed-in devices doesn't list this app twice
        crate::sign_out_of(&app, &origin).await;
    }
    let added = crate::add_place(&app, origin.clone(), link).map_err(|e| {
        tracing::warn!(error = %e, "couldn't save the server list");
        error("save_failed", "kwant couldn't save the server. Check that ~/.kwnt can be written.", Some(host.clone()))
    })?;
    Ok(json!({ "added": added, "origin": origin.as_str(), "host": host }))
}

/// Remove this server…: the same native confirm as the tray (no page builds its own), then the
/// server sign-out and the remove (its window, its sign-in cookie, its line in the file).
#[tauri::command]
pub async fn remove_server<R: Runtime>(app: AppHandle<R>, webview: Webview<R>, origin: String) -> Result<Value, Value> {
    bundled_only(&webview)?;
    let listed = crate::current(&app).servers.into_iter().find(|o| o.as_str() == origin);
    let Some(origin) = listed else {
        return Ok(json!({ "removed": false, "revoked": false, "cancelled": false }));
    };
    // the dialog and the sign-out block: a blocking thread, never the main thread
    let done = tauri::async_runtime::spawn_blocking(move || crate::confirm_and_remove(&app, &origin))
        .await
        .unwrap_or_default();
    Ok(json!({ "removed": done.removed, "revoked": done.revoked, "cancelled": done.cancelled }))
}

/// "Run kwant on this Mac".
#[tauri::command]
pub fn set_this_mac<R: Runtime>(app: AppHandle<R>, webview: Webview<R>, on: bool) -> Result<Value, Value> {
    bundled_only(&webview)?;
    crate::set_local(&app, on).map_err(|refused| match refused {
        ToggleRefused::LastPlace => error(
            "last_place",
            format!("Add a server before you stop kwant on {}.", this_machine()),
            None,
        ),
        ToggleRefused::SaveFailed => {
            error("save_failed", "kwant couldn't save the change. Check that ~/.kwnt can be written.", None)
        }
    })?;
    Ok(json!({ "this_mac": crate::current(&app).this_mac }))
}

fn this_machine() -> &'static str {
    if cfg!(target_os = "macos") {
        "this Mac"
    } else {
        "this computer"
    }
}

/// Closes the Add a server window.
#[tauri::command]
pub fn close_connect<R: Runtime>(app: AppHandle<R>, webview: Webview<R>) -> Result<(), Value> {
    bundled_only(&webview)?;
    if let Some(w) = app.get_webview_window(CONNECT) {
        let _ = w.close();
    }
    Ok(())
}

/// The loading page's Try again: the window's own place, checked (or started) again.
#[tauri::command]
pub fn retry_start<R: Runtime>(app: AppHandle<R>, webview: Webview<R>) -> Result<(), Value> {
    bundled_only(&webview)?;
    if let Some(place) = crate::current(&app).by_window(webview.label()) {
        crate::show_home(&app, &place);
    }
    Ok(())
}
