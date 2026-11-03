//! The macOS side of the "needs you" nudge: the menu-bar icon's template-frame animation, the
//! non-activating pill under the icon and the Dock badge. The rules (what is new, when to replay,
//! what the pill says) are in `nudge.rs`; Mira's spec is
//! /home/maus/shared/ship-software-safely/mac-nudge/design/spec.md (§1 and §3).

use crate::nudge::{self, Change, Item};
use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use tauri::{image::Image, tray::TrayIcon, AppHandle, LogicalPosition, Manager, WebviewUrl, WebviewWindowBuilder};

/// Mira's frames (design/tray/18pt @2x: 36×36 black + alpha; tray-icon draws the icon 18 pt tall,
/// so each cell lands on 2 device pixels). Frame 11 is the "needs you" rest pose.
const FRAMES: [&[u8]; 12] = [
    include_bytes!("../icons/nudge/frame-00.png"),
    include_bytes!("../icons/nudge/frame-01.png"),
    include_bytes!("../icons/nudge/frame-02.png"),
    include_bytes!("../icons/nudge/frame-03.png"),
    include_bytes!("../icons/nudge/frame-04.png"),
    include_bytes!("../icons/nudge/frame-05.png"),
    include_bytes!("../icons/nudge/frame-06.png"),
    include_bytes!("../icons/nudge/frame-07.png"),
    include_bytes!("../icons/nudge/frame-08.png"),
    include_bytes!("../icons/nudge/frame-09.png"),
    include_bytes!("../icons/nudge/frame-10.png"),
    include_bytes!("../icons/nudge/frame-11.png"),
];
const NEEDS_YOU: &[u8] = include_bytes!("../icons/nudge/needs-you.png");
/// Today's mark drawn on the same 18 pt grid, so the icon doesn't change size between states.
const IDLE: &[u8] = include_bytes!("../icons/nudge/idle.png");
/// 12 frames × 100 ms = 1.2 s, 10 fps.
const FRAME: Duration = Duration::from_millis(100);

const PILL: &str = "nudge";
/// The pill window: the 320 × 52 pill at (10, 4) plus room for its shadow, in points.
const PILL_W: f64 = 340.0;
const PILL_H: f64 = 76.0;
/// The page says when its pill has gone; if it never does (a hide asked for by the app), the
/// window hides after this anyway.
const HIDE_FALLBACK: Duration = Duration::from_millis(600);

/// Whether the person asked macOS to reduce motion (System Settings › Accessibility › Display).
/// Read each time, so a change applies to the next nudge. The pill page follows the same setting
/// through `prefers-reduced-motion`.
pub fn reduce_motion() -> bool {
    if crate::demo_reduce_motion() {
        return true;
    }
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

enum Icon {
    /// Play the frames once, then rest on "needs you".
    Play,
    /// Rest on "needs you" (true) or the plain icon (false).
    Rest(bool),
}

enum PillCmd {
    /// A poll while a pill may show: what just arrived, and what still waits.
    Update { arrived: Vec<Item>, live: HashSet<String> },
    /// The kwant window has focus, nudges are off or nothing waits: no pill.
    Quiet,
    /// The page: the pill was clicked.
    Open,
    /// The page: the pill has gone (its time ran out).
    Hidden,
}

pub struct Nudger {
    icon: Sender<Icon>,
    pill: Sender<PillCmd>,
    badge: Option<usize>,
}

impl Nudger {
    pub fn start(handle: &AppHandle, tray: TrayIcon) -> Nudger {
        let (icon_tx, icon_rx) = mpsc::channel();
        let (pill_tx, pill_rx) = mpsc::channel();
        let anim_tray = tray.clone();
        std::thread::Builder::new()
            .name("kwant-nudge-icon".into())
            .spawn(move || animate(anim_tray, icon_rx))
            .expect("nudge icon thread");
        let h = handle.clone();
        let opener = pill_tx.clone();
        std::thread::Builder::new()
            .name("kwant-nudge-pill".into())
            .spawn(move || pill_loop(h, tray, pill_rx, opener))
            .expect("nudge pill thread");
        Nudger { icon: icon_tx, pill: pill_tx, badge: None }
    }

    /// One poll's change. The Dock badge and the resting "needs you" icon follow the count (that
    /// is state, not motion); the animation and the pill only while "Show nudges" is on, and the
    /// pill only while the kwant window isn't focused.
    pub fn on_poll(&mut self, handle: &AppHandle, change: &Change, show: bool) {
        let badge = (change.count > 0).then_some(change.count);
        if badge != self.badge {
            if let Some(main) = handle.get_webview_window(crate::MAIN) {
                if main.set_badge_count(badge.map(|n| n as i64)).is_ok() {
                    self.badge = badge;
                }
            }
        }
        let _ = self.icon.send(if show && change.play { Icon::Play } else { Icon::Rest(change.count > 0) });
        let cmd = if !show || change.count == 0 || crate::main_focused(handle) {
            PillCmd::Quiet
        } else {
            PillCmd::Update { arrived: change.arrived.clone(), live: change.live.clone() }
        };
        let _ = self.pill.send(cmd);
    }
}

fn decode(png: &[u8]) -> Option<Image<'static>> {
    Image::from_bytes(png).ok()
}

/// Owns the menu-bar icon picture. Every frame is set as a template: on macOS `set_icon` alone
/// turns the template flag off, and the icon would stop following the menu bar's colour.
fn animate(tray: TrayIcon, rx: Receiver<Icon>) {
    let frames: Vec<Image<'static>> = FRAMES.iter().filter_map(|f| decode(f)).collect();
    let (Some(still), Some(idle)) = (decode(NEEDS_YOU), decode(IDLE)) else {
        tracing::error!("the nudge icons don't decode");
        return;
    };
    let set = |img: &Image<'static>| {
        let _ = tray.set_icon_with_as_template(Some(img.clone()), true);
    };
    set(&idle);
    let mut resting_on_still = false;
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Icon::Play if reduce_motion() => {
                set(&still);
                resting_on_still = true;
            }
            Icon::Play => {
                let mut stop = false;
                for f in &frames {
                    set(f);
                    std::thread::sleep(FRAME);
                    // answered mid-animation: drop the rest of it
                    while let Ok(next) = rx.try_recv() {
                        if let Icon::Rest(false) = next {
                            stop = true;
                        }
                    }
                    if stop {
                        break;
                    }
                }
                set(if stop { &idle } else { &still });
                resting_on_still = !stop;
            }
            Icon::Rest(waiting) if waiting != resting_on_still => {
                set(if waiting { &still } else { &idle });
                resting_on_still = waiting;
            }
            Icon::Rest(_) => {}
        }
    }
}

/// What the pill page's `kwantNudge.show` takes: every item on the pill, oldest first. The page
/// writes the text in escaped.
fn payload(items: &[Item]) -> String {
    let list: Vec<serde_json::Value> = items
        .iter()
        .map(|i| serde_json::json!({ "kind": i.kind.as_str(), "name": i.kwant_name, "title": i.title }))
        .collect();
    serde_json::Value::Array(list).to_string()
}

fn build_pill(handle: &AppHandle, opener: Sender<PillCmd>) -> Option<tauri::WebviewWindow> {
    if let Some(w) = handle.get_webview_window(PILL) {
        return Some(w);
    }
    let built = WebviewWindowBuilder::new(handle, PILL, WebviewUrl::App("nudge.html".into()))
        .title("kwant needs you")
        .inner_size(PILL_W, PILL_H)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        // Never key or main: showing the pill doesn't take focus from the app being typed in
        .focusable(false)
        .focused(false)
        .accept_first_mouse(true)
        .visible(false)
        .on_navigation(move |url| {
            if url.scheme() == "kwant-nudge" {
                // kwant-nudge://open (clicked) or kwant-nudge://hidden (gone); never loaded
                let _ = opener.send(if url.host_str() == Some("open") { PillCmd::Open } else { PillCmd::Hidden });
                return false;
            }
            // only the bundled pill page itself
            url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost")
        })
        .build();
    match built {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::error!(error = %e, "couldn't make the nudge pill");
            None
        }
    }
}

/// Puts the pill window under the menu-bar icon: centred on it, 2 pt below it, kept 8 pt inside
/// the screen (at the top right when macOS won't say where the icon is).
fn place(handle: &AppHandle, tray: &TrayIcon, pill: &tauri::WebviewWindow) -> Option<(f64, f64)> {
    let monitor = handle.primary_monitor().ok().flatten()?;
    let scale = monitor.scale_factor();
    let screen = monitor.size().to_logical::<f64>(scale);
    let origin = monitor.position().to_logical::<f64>(scale);
    let (cx, bottom) = match tray.rect().ok().flatten() {
        Some(r) => {
            let pos = r.position.to_logical::<f64>(scale);
            let size = r.size.to_logical::<f64>(scale);
            if crate::DEMO.get().is_some() {
                println!("demo-nudge: tray {:.0},{:.0},{:.0},{:.0}", pos.x, pos.y, size.width, size.height);
            }
            (pos.x + size.width / 2.0, pos.y + size.height)
        }
        None => (origin.x + screen.width - 8.0 - PILL_W / 2.0, origin.y + 24.0),
    };
    let x = (cx - PILL_W / 2.0).clamp(origin.x + 8.0, origin.x + screen.width - 8.0 - PILL_W);
    let y = bottom + 2.0;
    let _ = pill.set_position(LogicalPosition::new(x, y));
    Some((x, y))
}

/// Drives the pill page: shows and coalesces arrivals, drops what was answered meanwhile, hides it
/// when the person turns to kwant, and opens the item on a click. The page owns the timing (5 s,
/// held while the pointer is on it, 12 s at most) and says when the pill has gone.
fn pill_loop(handle: AppHandle, tray: TrayIcon, rx: Receiver<PillCmd>, opener: Sender<PillCmd>) {
    // Made ahead, hidden, so the first nudge doesn't wait for a page load
    let mut window = build_pill(&handle, opener.clone());
    // What the pill shows now, oldest first; empty when no pill is up
    let mut shown: Vec<Item> = Vec::new();
    let mut last_show: Option<Instant> = None;
    let mut hide_by: Option<Instant> = None;
    let demo = crate::DEMO.get().is_some();
    let hide_window = |w: &Option<tauri::WebviewWindow>| {
        if let Some(w) = w {
            let _ = w.hide();
        }
    };
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(PillCmd::Update { arrived, live }) => {
                let before = shown.len();
                shown.retain(|i| live.contains(&i.key()));
                if arrived.is_empty() && shown.len() == before {
                    continue;
                }
                if shown.is_empty() && arrived.is_empty() {
                    // everything on it was answered elsewhere
                    if let Some(w) = &window {
                        let _ = w.eval("window.kwantNudge && kwantNudge.hide()");
                    }
                    hide_by = Some(Instant::now() + HIDE_FALLBACK);
                    continue;
                }
                let fresh = shown.is_empty();
                // the server lists newest first; the pill wants oldest first
                shown.extend(arrived.into_iter().rev());
                if window.is_none() {
                    window = build_pill(&handle, opener.clone());
                }
                let Some(w) = &window else { continue };
                let reduce = crate::demo_reduce_motion();
                let script = format!(
                    "document.documentElement.classList.toggle('kw-reduce', {reduce}); window.kwantNudge && kwantNudge.show({})",
                    payload(&shown)
                );
                let spot = if fresh { place(&handle, &tray, w) } else { None };
                let _ = w.eval(&script);
                let _ = w.show();
                last_show = Some(Instant::now());
                hide_by = None;
                if demo {
                    match spot {
                        Some((x, y)) => println!("demo-nudge: pill {x:.0},{y:.0},{PILL_W:.0},{PILL_H:.0} items={}", shown.len()),
                        None => println!("demo-nudge: pill coalesced items={}", shown.len()),
                    }
                }
            }
            Ok(PillCmd::Quiet) => {
                if !shown.is_empty() {
                    if let Some(w) = &window {
                        let _ = w.eval("window.kwantNudge && kwantNudge.hide()");
                    }
                    shown.clear();
                    hide_by = Some(Instant::now() + HIDE_FALLBACK);
                }
            }
            Ok(PillCmd::Open) => {
                if let Some(p) = nudge::pill(&shown) {
                    let h = handle.clone();
                    let _ = handle.run_on_main_thread(move || crate::open_window_at(&h, Some(&p.target)));
                }
                hide_window(&window);
                shown.clear();
                hide_by = None;
            }
            Ok(PillCmd::Hidden) => {
                // a pill shown a moment ago is a new one; this is the old one's goodbye
                if last_show.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
                    continue;
                }
                hide_window(&window);
                shown.clear();
                hide_by = None;
                if demo {
                    println!("demo-nudge: pill hidden");
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if hide_by.is_some_and(|t| Instant::now() >= t) {
            hide_window(&window);
            hide_by = None;
        }
    }
}
