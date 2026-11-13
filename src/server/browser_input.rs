//! Pure pieces of the live-view input protocol (v2): the key table, key event building, the
//! resize clamp, cursor names and the move rate limit. No I/O, so every rule is unit-tested here;
//! [`super::browser_stream`] does the sending.

use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// CDP modifier bits.
pub const ALT: u8 = 1;
pub const CTRL: u8 = 2;
pub const META: u8 = 4;
pub const SHIFT: u8 = 8;

/// The Windows virtual key code Chromium expects for a `KeyboardEvent.code`, or `None` for a
/// code we don't know (refused, so a page never sees a key we can't describe properly).
pub fn virtual_key_code(code: &str) -> Option<u16> {
    let bytes = code.as_bytes();
    // KeyA..KeyZ -> 65..90
    if let Some(rest) = code.strip_prefix("Key") {
        if rest.len() == 1 && bytes[3].is_ascii_uppercase() {
            return Some(bytes[3] as u16);
        }
        return None;
    }
    // Digit0..Digit9 -> 48..57
    if let Some(rest) = code.strip_prefix("Digit") {
        if rest.len() == 1 && bytes[5].is_ascii_digit() {
            return Some(bytes[5] as u16);
        }
        return None;
    }
    // Numpad0..Numpad9 -> 96..105
    if let Some(rest) = code.strip_prefix("Numpad") {
        if rest.len() == 1 && bytes[6].is_ascii_digit() {
            return Some(96 + (bytes[6] - b'0') as u16);
        }
    }
    // F1..F24 -> 112..135
    if let Some(n) = code.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()) {
        if (1..=24).contains(&n) && !code[1..].starts_with('0') {
            return Some(111 + n);
        }
        return None;
    }
    Some(match code {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" | "NumpadEnter" => 13,
        "ShiftLeft" | "ShiftRight" => 16,
        "ControlLeft" | "ControlRight" => 17,
        "AltLeft" | "AltRight" => 18,
        "Pause" => 19,
        "CapsLock" => 20,
        "Escape" => 27,
        "Space" => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "PrintScreen" => 44,
        "Insert" => 45,
        "Delete" => 46,
        "MetaLeft" => 91,
        "MetaRight" => 92,
        "ContextMenu" => 93,
        "NumpadMultiply" => 106,
        "NumpadAdd" => 107,
        "NumpadComma" => 108,
        "NumpadSubtract" => 109,
        "NumpadDecimal" => 110,
        "NumpadDivide" => 111,
        "NumpadEqual" => 187,
        "NumLock" => 144,
        "ScrollLock" => 145,
        "Semicolon" => 186,
        "Equal" => 187,
        "Comma" => 188,
        "Minus" => 189,
        "Period" => 190,
        "Slash" => 191,
        "Backquote" => 192,
        "BracketLeft" => 219,
        "Backslash" => 220,
        "BracketRight" => 221,
        "Quote" => 222,
        "IntlBackslash" => 226,
        "IntlRo" => 193,
        "IntlYen" => 255,
        _ => return None,
    })
}

/// `KeyboardEvent.location` for a code: 1 left, 2 right, 3 numpad, else 0.
pub fn key_location(code: &str) -> u8 {
    if code.starts_with("Numpad") {
        3
    } else if code.ends_with("Left") && !code.starts_with("Arrow") && code != "BracketLeft" {
        1
    } else if code.ends_with("Right") && !code.starts_with("Arrow") && code != "BracketRight" {
        2
    } else {
        0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyKind {
    Down,
    Up,
}

/// One v2 key event as sent by the client.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyEvent<'a> {
    pub kind: KeyKind,
    pub key: &'a str,
    pub code: &'a str,
    pub text: Option<&'a str>,
    pub modifiers: u8,
    pub location: Option<u8>,
    pub auto_repeat: bool,
}

/// The text a key press types: what the client sent, else the key itself when it is one
/// printable character, `\r` for Enter. `None` while Meta, or Ctrl without Alt, is held: those
/// are shortcuts (Ctrl+A selects, it doesn't type "a"). Ctrl+Alt stays text, which is AltGr on
/// Windows keyboards.
pub fn key_text(event: &KeyEvent<'_>) -> Result<Option<String>, &'static str> {
    if event.kind == KeyKind::Up {
        return Ok(None);
    }
    let derived = match event.text {
        Some(text) => (!text.is_empty()).then(|| text.to_string()),
        None if event.code == "Enter" || event.code == "NumpadEnter" => Some("\r".to_string()),
        None if event.key.chars().count() == 1 => Some(event.key.to_string()),
        None => None,
    };
    if let Some(text) = &derived {
        if text.chars().count() > 4 || text.chars().any(|c| c.is_control() && c != '\r') {
            return Err("Invalid key text");
        }
    }
    let shortcut = event.modifiers & META != 0 || (event.modifiers & CTRL != 0 && event.modifiers & ALT == 0);
    Ok(if shortcut { None } else { derived })
}

/// The editing command a shortcut runs, sent with the key so it works on every platform the
/// browser runs on (headless Chrome on a Mac doesn't bind Ctrl+A to select all).
pub fn shortcut_command(event: &KeyEvent<'_>) -> Option<&'static str> {
    if event.kind != KeyKind::Down {
        return None;
    }
    let m = event.modifiers;
    // exactly one of Ctrl and Meta, and no Alt
    if m & ALT != 0 || (m & CTRL != 0) == (m & META != 0) {
        return None;
    }
    let shift = m & SHIFT != 0;
    Some(match (event.code, shift) {
        ("KeyA", false) => "selectAll",
        ("KeyC", false) => "copy",
        ("KeyX", false) => "cut",
        ("KeyV", false) => "paste",
        ("KeyZ", false) => "undo",
        ("KeyZ", true) | ("KeyY", false) => "redo",
        _ => return None,
    })
}

/// The `Input.dispatchKeyEvent` params for one v2 key event: `keyDown` with text for keys that
/// type something (so keydown, keypress and input all fire), `rawKeyDown` otherwise, `keyUp`.
pub fn key_params(event: &KeyEvent<'_>) -> Result<Value, &'static str> {
    if event.key.is_empty() || event.key.chars().count() > 32 || event.code.len() > 32 {
        return Err("Invalid key");
    }
    let vk = virtual_key_code(event.code).ok_or("Unsupported key code")?;
    if event.modifiers > 15 {
        return Err("Invalid modifiers");
    }
    let location = match event.location {
        Some(l) if l <= 3 => l,
        Some(_) => return Err("Invalid key location"),
        None => key_location(event.code),
    };
    let text = key_text(event)?;
    let kind = match (event.kind, &text) {
        (KeyKind::Down, Some(_)) => "keyDown",
        (KeyKind::Down, None) => "rawKeyDown",
        (KeyKind::Up, _) => "keyUp",
    };
    let mut params = json!({
        "type": kind,
        "key": event.key,
        "code": event.code,
        // No `nativeVirtualKeyCode`: Chrome reads it as the OS's own key number, and on a Mac
        // builds a real key event from it. A Windows number there is a different Mac key (65,
        // "A", is the keypad's "."), so nothing typed on a Mac Chrome. Puppeteer and Playwright
        // leave it out too.
        "windowsVirtualKeyCode": vk,
        "modifiers": event.modifiers,
        "location": location,
        "autoRepeat": event.auto_repeat,
        "isKeypad": location == 3,
    });
    if let Some(text) = text {
        params["text"] = json!(text);
        params["unmodifiedText"] = json!(text);
    }
    if let Some(command) = shortcut_command(event) {
        params["commands"] = json!([command]);
    }
    Ok(params)
}

/// Viewport limits for `resize`, in CSS px, and the device pixel ratio.
pub const RESIZE_WIDTH: (f64, f64) = (360.0, 1920.0);
pub const RESIZE_HEIGHT: (f64, f64) = (360.0, 1200.0);
pub const RESIZE_DPR: (f64, f64) = (1.0, 2.0);

/// A `resize` request clamped to the limits: `(width, height, dpr)`. Non-finite values are
/// refused, a missing dpr is 1.
pub fn clamp_resize(width: f64, height: f64, dpr: Option<f64>) -> Result<(u32, u32, f64), &'static str> {
    let dpr = dpr.unwrap_or(1.0);
    if !width.is_finite() || !height.is_finite() || !dpr.is_finite() {
        return Err("Invalid size");
    }
    let w = width.round().clamp(RESIZE_WIDTH.0, RESIZE_WIDTH.1) as u32;
    let h = height.round().clamp(RESIZE_HEIGHT.0, RESIZE_HEIGHT.1) as u32;
    // two decimals is plenty and keeps the screencast size an integer
    let dpr = (dpr.clamp(RESIZE_DPR.0, RESIZE_DPR.1) * 100.0).round() / 100.0;
    Ok((w, h, dpr))
}

/// The point if it is inside a `width` x `height` viewport (CSS px), else an error.
pub fn in_viewport(x: f64, y: f64, width: f64, height: f64) -> Result<(f64, f64), &'static str> {
    if x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0 && x < width && y < height {
        Ok((x, y))
    } else {
        Err("Pointer outside viewport")
    }
}

/// Mouse buttons as the client names them, with their CDP `buttons` bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    #[default]
    None,
    Left,
    Middle,
    Right,
    Back,
    Forward,
}

impl MouseButton {
    pub fn bit(self) -> u8 {
        match self {
            MouseButton::None => 0,
            MouseButton::Left => 1,
            MouseButton::Right => 2,
            MouseButton::Middle => 4,
            MouseButton::Back => 8,
            MouseButton::Forward => 16,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            MouseButton::None => "none",
            MouseButton::Left => "left",
            MouseButton::Middle => "middle",
            MouseButton::Right => "right",
            MouseButton::Back => "back",
            MouseButton::Forward => "forward",
        }
    }
}

/// The button a move reports while `buttons` are held: the first held one, else "none".
pub fn moving_button(buttons: u8) -> &'static str {
    crate::sandbox::browser::MOUSE_BUTTON_BITS
        .iter()
        .find(|(bit, _)| buttons & bit != 0)
        .map(|(_, name)| *name)
        .unwrap_or("none")
}

/// A computed CSS cursor reduced to one plain keyword: custom `url(...)` cursors give their
/// fallback, anything odd gives "auto".
pub fn sanitize_cursor(raw: &str) -> String {
    let last = raw.rsplit(',').next().unwrap_or("").trim();
    if !last.is_empty() && last.len() <= 32 && last.bytes().all(|b| b.is_ascii_lowercase() || b == b'-') {
        last.to_string()
    } else {
        "auto".to_string()
    }
}

/// A token bucket: at most `rate` per second, bursts up to `burst`. Extra moves are dropped.
#[derive(Debug)]
pub struct RateLimit {
    rate: f64,
    burst: f64,
    tokens: f64,
    at: Instant,
}

impl RateLimit {
    pub fn new(rate: f64, burst: f64, now: Instant) -> Self {
        Self { rate, burst, tokens: burst, at: now }
    }

    pub fn allow(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.at).as_secs_f64();
        self.at = now;
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Moves per second the stream forwards.
pub const MOVES_PER_SECOND: f64 = 120.0;
/// How often a cursor change is looked up after moves.
pub const CURSOR_EVERY: Duration = Duration::from_millis(125);

/// Finds the `<select>` under a point, in the top document. Evaluated with returnByValue false,
/// so the element comes back as a remote object for [`SELECT_INFO_FN`] and [`SELECT_SET_FN`].
pub fn select_probe_js(x: f64, y: f64) -> String {
    format!("(() => {{ const s = document.elementFromPoint({x}, {y})?.closest('select'); return s && !s.disabled ? s : null; }})()")
}

/// Called on the select: focuses it and describes its options and where it is.
pub const SELECT_INFO_FN: &str = r#"function () {
  this.focus();
  const r = this.getBoundingClientRect();
  const cut = (s) => String(s ?? '').slice(0, 200);
  return {
    multiple: this.multiple,
    rect: { x: r.x, y: r.y, width: r.width, height: r.height },
    options: Array.from(this.options).slice(0, 500).map((o) => ({
      label: cut(o.label || o.text),
      value: cut(o.value),
      disabled: o.disabled || (o.parentElement && o.parentElement.tagName === 'OPTGROUP' && o.parentElement.disabled),
      selected: o.selected,
      group: o.parentElement && o.parentElement.tagName === 'OPTGROUP' ? cut(o.parentElement.label) : null,
    })),
  };
}"#;

/// Called on the select with the chosen indexes: selects them (only the first unless multiple,
/// never a disabled one) and fires bubbling input and change, as a real pick does.
pub const SELECT_SET_FN: &str = r#"function (indexes) {
  const opts = Array.from(this.options);
  const ok = indexes.filter((i) => Number.isInteger(i) && i >= 0 && i < opts.length && !opts[i].disabled);
  if (!ok.length) return false;
  if (this.multiple) { opts.forEach((o, i) => { o.selected = ok.includes(i); }); }
  else { this.selectedIndex = ok[0]; }
  this.dispatchEvent(new Event('input', { bubbles: true }));
  this.dispatchEvent(new Event('change', { bubbles: true }));
  return true;
}"#;

/// The page's selected text, for `copy`. Never from a password field.
pub const COPY_JS: &str = r#"(() => {
  const a = document.activeElement;
  if (a && (a.tagName === 'INPUT' || a.tagName === 'TEXTAREA')) {
    if (a.type === 'password' || typeof a.selectionStart !== 'number') return '';
    return a.value.slice(a.selectionStart, a.selectionEnd);
  }
  return String(getSelection() || '');
})()"#;

/// Longest text `copy` returns.
pub const COPY_MAX: usize = 65536;

/// Called on a file input (resolved into the recorder's isolated world) to start one file: a
/// fresh list of Blob parts, with the name, type and `lastModified` to give the finished `File`.
pub const UPLOAD_BEGIN_FN: &str = r#"function (meta) {
  if (!this.__kwntUpload) this.__kwntUpload = { files: [] };
  this.__kwntUpload.files.push({ name: meta.name, mime: meta.mime, lastModified: meta.lastModified, chunks: [] });
  return true;
}"#;

/// Appends one base64-decoded chunk (at most 1 MB before encoding) to the file `UPLOAD_BEGIN_FN`
/// last started. Kept as `Uint8Array`s, not a plain byte array, so a 100 MB file doesn't balloon
/// into thirty-odd million boxed numbers in a small container.
pub const UPLOAD_CHUNK_FN: &str = r#"function (b64) {
  const u = this.__kwntUpload;
  if (!u || !u.files.length) return false;
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  u.files[u.files.length - 1].chunks.push(bytes);
  return true;
}"#;

/// Builds the `File`s from their chunks, sets them on the input through a `DataTransfer` (the
/// only way to set `.files` from script) and fires bubbling input and change, as a real pick
/// does. Then tells the recorder, in this same world, so Show it once logs the label, never the
/// name (capture.rs S-C13); a no-op where nothing is recording.
pub const UPLOAD_FINISH_FN: &str = r#"function () {
  const u = this.__kwntUpload;
  if (!u || !u.files.length) return false;
  const dt = new DataTransfer();
  u.files.forEach((f) => dt.items.add(new File(f.chunks, f.name, { type: f.mime, lastModified: f.lastModified })));
  this.files = dt.files;
  delete this.__kwntUpload;
  this.dispatchEvent(new Event('input', { bubbles: true }));
  this.dispatchEvent(new Event('change', { bubbles: true }));
  const rec = globalThis.__kwntRecorderUpload;
  if (typeof rec === 'function') rec(this);
  return true;
}"#;

pub fn cursor_js(x: f64, y: f64) -> String {
    format!(
        "(() => {{ const e = document.elementFromPoint({x}, {y}); if (!e) return 'auto'; const c = getComputedStyle(e).cursor; \
         if (c !== 'auto') return c; if (e.isContentEditable || e.tagName === 'TEXTAREA' || (e.tagName === 'INPUT' && !['button','submit','reset','checkbox','radio','range','color','file','image'].includes(e.type))) return 'text'; return 'default'; }})()"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn down<'a>(key: &'a str, code: &'a str, modifiers: u8) -> KeyEvent<'a> {
        KeyEvent { kind: KeyKind::Down, key, code, text: None, modifiers, location: None, auto_repeat: false }
    }

    #[test]
    fn the_key_table_covers_letters_digits_function_numpad_punctuation_and_editing_keys() {
        assert_eq!(virtual_key_code("KeyA"), Some(65));
        assert_eq!(virtual_key_code("KeyZ"), Some(90));
        assert_eq!(virtual_key_code("Digit0"), Some(48));
        assert_eq!(virtual_key_code("Digit9"), Some(57));
        assert_eq!(virtual_key_code("F1"), Some(112));
        assert_eq!(virtual_key_code("F12"), Some(123));
        assert_eq!(virtual_key_code("F24"), Some(135));
        assert_eq!(virtual_key_code("Numpad0"), Some(96));
        assert_eq!(virtual_key_code("Numpad9"), Some(105));
        assert_eq!(virtual_key_code("NumpadEnter"), Some(13));
        assert_eq!(virtual_key_code("NumpadAdd"), Some(107));
        assert_eq!(virtual_key_code("Semicolon"), Some(186));
        assert_eq!(virtual_key_code("Quote"), Some(222));
        assert_eq!(virtual_key_code("BracketLeft"), Some(219));
        assert_eq!(virtual_key_code("PageUp"), Some(33));
        assert_eq!(virtual_key_code("PageDown"), Some(34));
        assert_eq!(virtual_key_code("Tab"), Some(9));
        assert_eq!(virtual_key_code("Space"), Some(32));
        assert_eq!(virtual_key_code("ShiftRight"), Some(16));
        assert_eq!(virtual_key_code("MetaLeft"), Some(91));
        // unknown codes are refused, whatever the key says
        for bad in ["", "Keya", "KeyAA", "Key1", "Digit", "Digit10", "F0", "F25", "F01", "Numpad", "NumpadX", "Unidentified", "keyA"] {
            assert_eq!(virtual_key_code(bad), None, "{bad}");
        }
    }

    #[test]
    fn key_locations_follow_the_code() {
        assert_eq!(key_location("ShiftLeft"), 1);
        assert_eq!(key_location("ControlRight"), 2);
        assert_eq!(key_location("Numpad5"), 3);
        assert_eq!(key_location("ArrowLeft"), 0);
        assert_eq!(key_location("BracketRight"), 0);
        assert_eq!(key_location("KeyA"), 0);
    }

    #[test]
    fn printable_keys_go_down_with_text_and_others_raw() {
        let p = key_params(&down("a", "KeyA", 0)).unwrap();
        assert_eq!(p["type"], "keyDown");
        assert_eq!(p["text"], "a");
        assert_eq!(p["windowsVirtualKeyCode"], 65);
        let p = key_params(&down("A", "KeyA", SHIFT)).unwrap();
        assert_eq!((p["type"].as_str(), p["text"].as_str(), p["modifiers"].as_u64()), (Some("keyDown"), Some("A"), Some(8)));
        let p = key_params(&down("Enter", "Enter", 0)).unwrap();
        assert_eq!((p["type"].as_str(), p["text"].as_str()), (Some("keyDown"), Some("\r")));
        let p = key_params(&down(" ", "Space", 0)).unwrap();
        assert_eq!(p["text"], " ");
        let p = key_params(&down("Tab", "Tab", SHIFT)).unwrap();
        assert_eq!((p["type"].as_str(), p.get("text"), p["modifiers"].as_u64()), (Some("rawKeyDown"), None, Some(8)));
        let p = key_params(&down("PageDown", "PageDown", 0)).unwrap();
        assert_eq!((p["type"].as_str(), p["windowsVirtualKeyCode"].as_u64()), (Some("rawKeyDown"), Some(34)));
        let p = key_params(&down("1", "Numpad1", 0)).unwrap();
        assert_eq!((p["location"].as_u64(), p["isKeypad"].as_bool()), (Some(3), Some(true)));
        let up = KeyEvent { kind: KeyKind::Up, ..down("a", "KeyA", 0) };
        let p = key_params(&up).unwrap();
        assert_eq!((p["type"].as_str(), p.get("text")), (Some("keyUp"), None));
        // client text wins (a dead key's composed character), within limits
        let p = key_params(&KeyEvent { text: Some("é"), ..down("é", "KeyE", 0) }).unwrap();
        assert_eq!(p["text"], "é");
        assert!(key_params(&KeyEvent { text: Some("toolong"), ..down("a", "KeyA", 0) }).is_err());
        assert!(key_params(&KeyEvent { text: Some("\u{7}"), ..down("a", "KeyA", 0) }).is_err());
        let auto = key_params(&KeyEvent { auto_repeat: true, ..down("ArrowDown", "ArrowDown", 0) }).unwrap();
        assert_eq!(auto["autoRepeat"], true);
    }

    #[test]
    fn keys_never_carry_a_native_key_code() {
        // Chrome on a Mac turns nativeVirtualKeyCode into a real Mac key event, and a Windows
        // number there is another key: typing did nothing in the Mac app
        for (key, code, mods) in [("a", "KeyA", 0), ("Backspace", "Backspace", 0), ("Enter", "Enter", 0), ("a", "KeyA", CTRL), ("1", "Numpad1", 0)] {
            let p = key_params(&down(key, code, mods)).unwrap();
            assert!(p.get("nativeVirtualKeyCode").is_none(), "{code}");
            let up = key_params(&KeyEvent { kind: KeyKind::Up, ..down(key, code, mods) }).unwrap();
            assert!(up.get("nativeVirtualKeyCode").is_none(), "{code} up");
        }
    }

    #[test]
    fn shortcuts_carry_no_text_and_an_editing_command() {
        let p = key_params(&down("a", "KeyA", CTRL)).unwrap();
        assert_eq!((p["type"].as_str(), p.get("text")), (Some("rawKeyDown"), None));
        assert_eq!(p["commands"], json!(["selectAll"]));
        assert_eq!(key_params(&down("a", "KeyA", META)).unwrap()["commands"], json!(["selectAll"]));
        assert_eq!(key_params(&down("z", "KeyZ", CTRL)).unwrap()["commands"], json!(["undo"]));
        assert_eq!(key_params(&down("Z", "KeyZ", CTRL | SHIFT)).unwrap()["commands"], json!(["redo"]));
        assert_eq!(key_params(&down("c", "KeyC", CTRL)).unwrap()["commands"], json!(["copy"]));
        assert!(key_params(&down("b", "KeyB", CTRL)).unwrap().get("commands").is_none());
        // AltGr (Ctrl+Alt) still types its character and runs no command
        let p = key_params(&down("@", "KeyQ", CTRL | ALT)).unwrap();
        assert_eq!((p["text"].as_str(), p.get("commands")), (Some("@"), None));
        // Alt alone (Mac Option) types too
        assert_eq!(key_params(&down("å", "KeyA", ALT)).unwrap()["text"], "å");
        // Ctrl+Meta together is not a known shortcut
        assert!(key_params(&down("a", "KeyA", CTRL | META)).unwrap().get("commands").is_none());
        // the up of a shortcut runs nothing
        assert!(key_params(&KeyEvent { kind: KeyKind::Up, ..down("a", "KeyA", CTRL) }).unwrap().get("commands").is_none());
    }

    #[test]
    fn bad_keys_are_refused() {
        assert_eq!(key_params(&down("a", "Unidentified", 0)), Err("Unsupported key code"));
        assert_eq!(key_params(&down("", "KeyA", 0)), Err("Invalid key"));
        assert_eq!(key_params(&down("a", "KeyA", 16)), Err("Invalid modifiers"));
        assert_eq!(key_params(&KeyEvent { location: Some(4), ..down("a", "KeyA", 0) }), Err("Invalid key location"));
        let long = "x".repeat(33);
        assert_eq!(key_params(&down(&long, "KeyA", 0)), Err("Invalid key"));
    }

    #[test]
    fn resize_is_clamped_and_refuses_nonsense() {
        assert_eq!(clamp_resize(1440.0, 900.0, Some(2.0)), Ok((1440, 900, 2.0)));
        assert_eq!(clamp_resize(100.0, 100.0, Some(0.5)), Ok((360, 360, 1.0)));
        assert_eq!(clamp_resize(5000.0, 5000.0, Some(3.0)), Ok((1920, 1200, 2.0)));
        assert_eq!(clamp_resize(1280.4, 799.6, None), Ok((1280, 800, 1.0)));
        assert_eq!(clamp_resize(1000.0, 700.0, Some(1.333333)), Ok((1000, 700, 1.33)));
        assert!(clamp_resize(f64::NAN, 800.0, None).is_err());
        assert!(clamp_resize(1280.0, f64::INFINITY, None).is_err());
        assert!(clamp_resize(1280.0, 800.0, Some(f64::NAN)).is_err());
    }

    #[test]
    fn points_are_checked_against_the_current_viewport() {
        assert!(in_viewport(0.0, 0.0, 1280.0, 800.0).is_ok());
        assert!(in_viewport(1279.9, 799.9, 1280.0, 800.0).is_ok());
        assert!(in_viewport(1280.0, 10.0, 1280.0, 800.0).is_err());
        assert!(in_viewport(10.0, 800.0, 1280.0, 800.0).is_err());
        assert!(in_viewport(-1.0, 10.0, 1280.0, 800.0).is_err());
        assert!(in_viewport(f64::NAN, 10.0, 1280.0, 800.0).is_err());
        // after a resize to 1600x1000 the old limits no longer apply
        assert!(in_viewport(1500.0, 900.0, 1600.0, 1000.0).is_ok());
        assert!(in_viewport(1500.0, 900.0, 1280.0, 800.0).is_err());
    }

    #[test]
    fn buttons_and_moves() {
        assert_eq!(MouseButton::Left.bit() | MouseButton::Right.bit() | MouseButton::Middle.bit(), 7);
        assert_eq!(moving_button(0), "none");
        assert_eq!(moving_button(1), "left");
        assert_eq!(moving_button(2), "right");
        assert_eq!(moving_button(6), "right");
        assert_eq!(moving_button(4), "middle");
    }

    #[test]
    fn cursors_are_plain_keywords() {
        assert_eq!(sanitize_cursor("pointer"), "pointer");
        assert_eq!(sanitize_cursor("url(\"x.png\") 2 2, pointer"), "pointer");
        assert_eq!(sanitize_cursor("url(x.png)"), "auto");
        assert_eq!(sanitize_cursor("ns-resize"), "ns-resize");
        assert_eq!(sanitize_cursor("<script>"), "auto");
        assert_eq!(sanitize_cursor(""), "auto");
    }

    #[test]
    fn moves_are_limited_to_the_rate_with_a_small_burst() {
        let t0 = Instant::now();
        let mut limit = RateLimit::new(MOVES_PER_SECOND, 10.0, t0);
        let burst = (0..50).filter(|_| limit.allow(t0)).count();
        assert_eq!(burst, 10, "a burst is capped");
        // one second at 1000 moves/s lets about 120 through
        let mut passed = 0;
        for i in 1..=1000 {
            if limit.allow(t0 + Duration::from_millis(i)) {
                passed += 1;
            }
        }
        assert!((118..=122).contains(&passed), "{passed}");
        // 60 moves/s (one per frame) all pass
        let mut limit = RateLimit::new(MOVES_PER_SECOND, 10.0, t0);
        assert!((1..=120).all(|i| limit.allow(t0 + Duration::from_micros(16_667 * i))));
    }
}
