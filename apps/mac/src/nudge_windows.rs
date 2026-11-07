//! The Windows side of the "needs you" nudge (0.7.6 beta). The notification area shows tray
//! pictures as they are and can't say where the icon is, so, like Linux, there is no pill: the
//! icon swaps to Mira's colour "needs you" mark, its tooltip says how many wait, and the taskbar
//! button flashes once when something new arrives. No toast in the beta. The rules are in
//! `nudge.rs`.

use crate::nudge::Change;
use tauri::{image::Image, tray::TrayIcon, AppHandle, UserAttentionType};

// Mira's Linux colour marks (44 px): the notification area draws them as they are, too
const NORMAL_DARK: &[u8] = include_bytes!("../icons/linux/tray-normal-dark.png");
const NORMAL_LIGHT: &[u8] = include_bytes!("../icons/linux/tray-normal-light.png");
const NEEDS_YOU_DARK: &[u8] = include_bytes!("../icons/linux/tray-needs-you-dark.png");
const NEEDS_YOU_LIGHT: &[u8] = include_bytes!("../icons/linux/tray-needs-you-light.png");

/// Whether the taskbar is dark: Windows' "SystemUsesLightTheme" value, `0x0` for dark. Anything
/// unreadable counts as dark, Windows 10 and 11's default.
pub fn taskbar_is_dark(reg_output: Option<&str>) -> bool {
    let Some(out) = reg_output else { return true };
    let value = out
        .lines()
        .find(|l| l.contains("SystemUsesLightTheme"))
        .and_then(|l| l.split_whitespace().last());
    !matches!(value, Some("0x1"))
}

fn detect_dark_taskbar() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "/v",
            "SystemUsesLightTheme",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    taskbar_is_dark(out.as_deref())
}

pub fn tray_icon(needs_you: bool, dark_taskbar: bool) -> &'static [u8] {
    match (needs_you, dark_taskbar) {
        (false, true) => NORMAL_DARK,
        (false, false) => NORMAL_LIGHT,
        (true, true) => NEEDS_YOU_DARK,
        (true, false) => NEEDS_YOU_LIGHT,
    }
}

/// The icon's tooltip (Windows shows no text next to tray icons).
pub fn tooltip(count: usize) -> String {
    match count {
        0 => "kwant".to_string(),
        1 => "kwant: 1 thing needs you".to_string(),
        n => format!("kwant: {n} things need you"),
    }
}

pub struct Nudger {
    tray: TrayIcon,
    dark_taskbar: bool,
    count: Option<usize>,
}

impl Nudger {
    pub fn start(_: &AppHandle, tray: TrayIcon) -> Nudger {
        let dark_taskbar = detect_dark_taskbar();
        let _ = tray.set_icon(Image::from_bytes(tray_icon(false, dark_taskbar)).ok());
        Nudger { tray, dark_taskbar, count: None }
    }

    /// The icon and tooltip follow the count (state, so they stay with "Show nudges" off); the
    /// taskbar flash only while nudges are on and kwant isn't focused.
    pub fn on_poll(&mut self, handle: &AppHandle, change: &Change, show: bool) {
        if self.count != Some(change.count) {
            let needs_you = change.count > 0;
            let _ = self.tray.set_icon(Image::from_bytes(tray_icon(needs_you, self.dark_taskbar)).ok());
            let _ = self.tray.set_tooltip(Some(tooltip(change.count)));
            self.count = Some(change.count);
        }
        if !show || change.arrived.is_empty() || crate::kwant_focused(handle) {
            return;
        }
        let from = change.arrived.first().map(|i| i.place.as_str());
        if let Some(main) = crate::place_window(handle, from) {
            let _ = main.request_user_attention(Some(UserAttentionType::Informational));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_taskbar_theme_comes_from_the_registry_value() {
        let light = "\r\nHKEY_CURRENT_USER\\...\\Personalize\r\n    SystemUsesLightTheme    REG_DWORD    0x1\r\n";
        let dark = "\r\nHKEY_CURRENT_USER\\...\\Personalize\r\n    SystemUsesLightTheme    REG_DWORD    0x0\r\n";
        assert!(!taskbar_is_dark(Some(light)));
        assert!(taskbar_is_dark(Some(dark)));
        assert!(taskbar_is_dark(None));
        assert!(taskbar_is_dark(Some("")));
    }

    #[test]
    fn tooltips_count_what_waits() {
        assert_eq!(tooltip(0), "kwant");
        assert_eq!(tooltip(1), "kwant: 1 thing needs you");
        assert_eq!(tooltip(3), "kwant: 3 things need you");
    }

    #[test]
    fn every_tray_picture_decodes() {
        for (n, d) in [(false, false), (false, true), (true, false), (true, true)] {
            assert!(Image::from_bytes(tray_icon(n, d)).is_ok());
        }
    }
}
