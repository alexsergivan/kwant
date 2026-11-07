//! The Linux side of the "needs you" nudge. Linux panels show tray pictures as they are (no
//! template tinting) and can't say where the icon is, so there is no pill and no frame animation:
//! the icon swaps once to Mira's colour "needs you" mark, the window asks for attention, and a
//! system notification names who needs you. The rules are in `nudge.rs`.
//!
//! Notifications go straight to the freedesktop notification service over D-Bus with zbus,
//! which the app already links (tauri-plugin-single-instance uses it), so no new crate.

use crate::nudge::{self, Change};
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use tauri::{image::Image, tray::TrayIcon, AppHandle, UserAttentionType};

const NORMAL_DARK: &[u8] = include_bytes!("../icons/linux/tray-normal-dark.png");
const NORMAL_LIGHT: &[u8] = include_bytes!("../icons/linux/tray-normal-light.png");
const NEEDS_YOU_DARK: &[u8] = include_bytes!("../icons/linux/tray-needs-you-dark.png");
const NEEDS_YOU_LIGHT: &[u8] = include_bytes!("../icons/linux/tray-needs-you-light.png");

/// The app icon, sent with each notification: an AppImage installs no themed "kwant" icon, so a
/// name alone shows no picture (dunst: "No icon found in path: 'kwant'").
const APP_ICON: &[u8] = include_bytes!("../icons/128x128.png");

const BUS_NAME: &str = "org.freedesktop.Notifications";
const BUS_PATH: &str = "/org/freedesktop/Notifications";

/// Whether the panel the icon sits on is dark. Mira's rule (spec §6): dark unless the desktop
/// says it is light, because GNOME's top bar is always dark and most default panels are too.
/// GNOME, Ubuntu, Pantheon and Cinnamon count as dark whatever the app theme; elsewhere a light
/// colour scheme (`prefer-light`, or a KDE scheme without "dark" in its name) means light.
pub fn panel_is_dark(desktop: &str, color_scheme: Option<&str>, kde_scheme: Option<&str>) -> bool {
    let desktop = desktop.to_ascii_lowercase();
    if ["gnome", "unity", "ubuntu", "pantheon", "cinnamon"].iter().any(|d| desktop.contains(d)) {
        return true;
    }
    if color_scheme.is_some_and(|s| s.contains("prefer-light")) {
        return false;
    }
    if desktop.contains("kde") {
        if let Some(k) = kde_scheme {
            return k.to_ascii_lowercase().contains("dark");
        }
    }
    true
}

fn detect_dark_panel() -> bool {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let scheme = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let kde = dirs::config_dir()
        .and_then(|d| std::fs::read_to_string(d.join("kdeglobals")).ok())
        .and_then(|s| s.lines().find(|l| l.starts_with("ColorScheme=")).map(str::to_string));
    panel_is_dark(&desktop, scheme.as_deref(), kde.as_deref())
}

/// The picture for the tray: Mira's colour mark (design/linux/*-22@2x, 44 px), plain or with the
/// "!" badge.
pub fn tray_icon(needs_you: bool, dark_panel: bool) -> &'static [u8] {
    match (needs_you, dark_panel) {
        (false, true) => NORMAL_DARK,
        (false, false) => NORMAL_LIGHT,
        (true, true) => NEEDS_YOU_DARK,
        (true, false) => NEEDS_YOU_LIGHT,
    }
}

/// Notification bodies may be read as markup; titles are the person's own text.
pub fn escape_markup(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The `image-data` hint's shape (`(iiibiiay)`): width, height, rowstride, has alpha, bits per
/// sample, channels, RGBA bytes.
pub type ImageData = (i32, i32, i32, bool, i32, i32, Vec<u8>);

/// The `image-data` hint for a PNG.
pub fn image_data(png: &[u8]) -> Option<ImageData> {
    let img = Image::from_bytes(png).ok()?;
    let (w, h) = (i32::try_from(img.width()).ok()?, i32::try_from(img.height()).ok()?);
    let rgba = img.rgba().to_vec();
    (rgba.len() == (w as usize) * (h as usize) * 4).then_some((w, h, w * 4, true, 8, 4, rgba))
}

pub struct Nudger {
    tray: TrayIcon,
    dark_panel: bool,
    showing_needs_you: bool,
    notify: mpsc::Sender<nudge::Pill>,
}

impl Nudger {
    pub fn start(handle: &AppHandle, tray: TrayIcon) -> Nudger {
        let dark_panel = detect_dark_panel();
        let _ = tray.set_icon(Image::from_bytes(tray_icon(false, dark_panel)).ok());
        let (tx, rx) = mpsc::channel();
        let h = handle.clone();
        std::thread::Builder::new()
            .name("kwant-notify".into())
            .spawn(move || notify_loop(h, rx))
            .expect("notification thread");
        Nudger { tray, dark_panel, showing_needs_you: false, notify: tx }
    }

    /// The "needs you" icon follows the count (state, so it stays with "Show nudges" off); the
    /// attention request and the notification only while nudges are on and kwant isn't focused.
    pub fn on_poll(&mut self, handle: &AppHandle, change: &Change, show: bool) {
        let needs_you = change.count > 0;
        if needs_you != self.showing_needs_you {
            // appindicator writes the picture to a file each time, so only on a real change
            if self.tray.set_icon(Image::from_bytes(tray_icon(needs_you, self.dark_panel)).ok()).is_ok() {
                self.showing_needs_you = needs_you;
            }
        }
        if !show || change.arrived.is_empty() || crate::kwant_focused(handle) {
            return;
        }
        let from = change.arrived.first().map(|i| i.place.as_str());
        if let Some(main) = crate::place_window(handle, from) {
            let _ = main.request_user_attention(Some(UserAttentionType::Informational));
        }
        // One notification per poll: several at once say how many and open the Inbox
        if let Some(p) = nudge::pill(&nudge::one_place(&change.arrived)) {
            let _ = self.notify.send(p);
        }
    }
}

/// A notification's place (`Place::key`) and item, by its id.
type Targets = Arc<Mutex<HashMap<u32, (String, String)>>>;

fn notify_loop(handle: AppHandle, rx: mpsc::Receiver<nudge::Pill>) {
    let conn = match zbus::blocking::Connection::session() {
        Ok(c) => c,
        Err(e) => {
            tracing::info!(error = %e, "no session bus; nudges won't show notifications");
            while rx.recv().is_ok() {}
            return;
        }
    };
    let proxy = match zbus::blocking::Proxy::new(&conn, BUS_NAME, BUS_PATH, BUS_NAME) {
        Ok(p) => p,
        Err(e) => {
            tracing::info!(error = %e, "no notification service");
            while rx.recv().is_ok() {}
            return;
        }
    };
    let caps: Vec<String> = proxy.call("GetCapabilities", &()).unwrap_or_default();
    let actions = caps.iter().any(|c| c == "actions");
    let body_markup = caps.iter().any(|c| c == "body-markup");
    let icon = image_data(APP_ICON);
    let targets: Targets = Arc::default();
    if actions {
        listen_for_clicks(handle.clone(), proxy.clone(), targets.clone());
    }
    while let Ok(p) = rx.recv() {
        let body = if body_markup { escape_markup(&p.detail) } else { p.detail.clone() };
        // "default" is a click on the notification itself; no Allow/Deny buttons, ever
        let acts: Vec<&str> = if actions { vec!["default", "Open"] } else { vec![] };
        let mut hints: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
        hints.insert("desktop-entry", zbus::zvariant::Value::from("kwant"));
        hints.insert("urgency", zbus::zvariant::Value::from(1u8));
        if let Some(i) = &icon {
            hints.insert("image-data", zbus::zvariant::Value::from(zbus::zvariant::Structure::from(i.clone())));
        }
        // no app_icon name: the picture travels in image-data
        let sent = proxy.call::<_, _, u32>(
            "Notify",
            &("kwant", 0u32, "", p.headline.as_str(), body.as_str(), acts, hints, -1i32),
        );
        match sent {
            Ok(id) => {
                if crate::DEMO.get().is_some() {
                    println!("demo-nudge: notification {id} \"{}\"", p.headline);
                }
                if actions {
                    let mut t = targets.lock().unwrap();
                    if t.len() > 100 {
                        t.clear();
                    }
                    t.insert(id, (p.place, p.target));
                }
            }
            Err(e) => tracing::info!(error = %e, "the notification wasn't shown"),
        }
    }
}

/// A click on one of our notifications opens its item in the kwant window.
fn listen_for_clicks(handle: AppHandle, proxy: zbus::blocking::Proxy<'static>, targets: Targets) {
    std::thread::Builder::new()
        .name("kwant-notify-clicks".into())
        .spawn(move || {
            let Ok(signals) = proxy.receive_signal("ActionInvoked") else { return };
            for msg in signals {
                let Ok((id, _action)) = msg.body().deserialize::<(u32, String)>() else { continue };
                let Some((place, target)) = targets.lock().unwrap().remove(&id) else { continue };
                let h = handle.clone();
                let _ = handle.run_on_main_thread(move || crate::open_place_at(&h, &place, Some(&target)));
            }
        })
        .expect("notification click thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_panels_get_the_light_mark() {
        assert!(panel_is_dark("ubuntu:GNOME", None, None));
        assert!(panel_is_dark("ubuntu:GNOME", Some("'prefer-light'"), None));
        assert!(panel_is_dark("X-Cinnamon", None, None));
        assert!(!panel_is_dark("KDE", None, Some("ColorScheme=BreezeLight")));
        assert!(panel_is_dark("KDE", None, Some("ColorScheme=BreezeDark")));
        assert!(panel_is_dark("KDE", None, None));
        assert!(!panel_is_dark("XFCE", Some("'prefer-light'\n"), None));
        assert!(panel_is_dark("XFCE", Some("'default'"), None));
        // can't tell: dark, as Mira's spec says
        assert!(panel_is_dark("", None, None));
    }

    #[test]
    fn every_tray_picture_decodes() {
        for (n, d) in [(false, false), (false, true), (true, false), (true, true)] {
            let img = Image::from_bytes(tray_icon(n, d)).expect("decodes");
            assert_eq!((img.width(), img.height()), (44, 44));
        }
    }

    #[test]
    fn notifications_carry_the_app_icon_as_image_data() {
        let (w, h, stride, alpha, bits, channels, data) = image_data(APP_ICON).expect("the icon decodes");
        assert_eq!((w, h, stride, alpha, bits, channels), (128, 128, 512, true, 8, 4));
        assert_eq!(data.len(), 128 * 128 * 4);
        assert!(data.chunks(4).any(|px| px[3] > 0), "not a blank picture");
        let v = zbus::zvariant::Value::from(zbus::zvariant::Structure::from((w, h, stride, alpha, bits, channels, data)));
        assert_eq!(v.value_signature().to_string(), "(iiibiiay)");
        assert!(image_data(b"not a png").is_none());
    }

    #[test]
    fn titles_cannot_inject_markup() {
        assert_eq!(escape_markup("<b>x</b> & y"), "&lt;b&gt;x&lt;/b&gt; &amp; y");
    }
}
