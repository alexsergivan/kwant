//! Pairing codes (G2): 16 random bytes as base64url, good for 10 minutes and once, kept only as
//! their SHA-256. The code travels in a `https://t.me/<bot>?start=<code>` link the person opens or
//! scans; Telegram then sends `/start <code>` from their account.

use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};

/// How long a code works.
pub const CODE_MINUTES: i64 = 10;
/// Wrong codes from one Telegram account in [`FAIL_WINDOW_MINUTES`] after which that account's
/// codes are ignored until a new code is made (Q-G1). Counted per sender, so a stranger can't lock
/// the person out (Lin's L1).
pub const MAX_FAILURES: i64 = 5;
/// Wrong codes from everyone together in [`FAIL_WINDOW_MINUTES`] that lock pairing for the space
/// (`pair_locked`) until a new code is made.
pub const MAX_SPACE_FAILURES: i64 = 25;
pub const FAIL_WINDOW_MINUTES: i64 = 10;

/// A fresh code: (code, its sha256 in hex, when it stops working).
pub fn new_code() -> (String, String, DateTime<Utc>) {
    let mut bytes = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes).expect("the system's random source works");
    let code = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let hash = code_hash(&code);
    (code, hash, crate::clock::now() + Duration::minutes(CODE_MINUTES))
}

pub fn code_hash(code: &str) -> String {
    hex(&Sha256::digest(code.trim().as_bytes()))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The deep link the person opens or scans.
pub fn link(bot_username: &str, code: &str) -> String {
    format!("https://t.me/{bot_username}?start={code}")
}

/// The code in a `/start <code>` message (`/start@BotName <code>` too); `Some("")` for a bare
/// `/start`, `None` for any other text.
pub fn start_code(text: &str) -> Option<&str> {
    let t = text.trim();
    let rest = t.strip_prefix("/start")?;
    let rest = match rest.strip_prefix('@') {
        Some(r) => r.split_once(char::is_whitespace).map(|x| x.1).unwrap_or(""),
        None if rest.is_empty() || rest.starts_with(char::is_whitespace) => rest,
        None => return None,
    };
    Some(rest.trim())
}

/// The link as a QR code, an SVG with one path (qrcode is built without its svg feature, so it's
/// drawn here from the modules, as `signin::qr_lines` does for the terminal).
pub fn qr_svg(text: &str) -> String {
    let Ok(code) = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M) else {
        return String::new();
    };
    let width = code.width();
    let colors = code.to_colors();
    let quiet = 4usize;
    let size = width + 2 * quiet;
    let mut path = String::new();
    for y in 0..width {
        for x in 0..width {
            if colors[y * width + x] == qrcode::Color::Dark {
                path.push_str(&format!("M{} {}h1v1h-1z", x + quiet, y + quiet));
            }
        }
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\" shape-rendering=\"crispEdges\"><rect width=\"{size}\" height=\"{size}\" fill=\"#fff\"/><path fill=\"#000\" d=\"{path}\"/></svg>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_random_hashed_and_link_safe() {
        let (a, ha, exp) = new_code();
        let (b, _, _) = new_code();
        assert_ne!(a, b);
        assert_eq!(a.len(), 22);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'), "Telegram's start parameter allows A-Z a-z 0-9 _ -");
        assert_eq!(ha, code_hash(&a));
        assert_eq!(ha.len(), 64);
        assert!(!ha.contains(&a));
        let left = exp - crate::clock::now();
        assert!(left > Duration::minutes(9) && left <= Duration::minutes(10));
        assert_eq!(link("kwant_bot", &a), format!("https://t.me/kwant_bot?start={a}"));
    }

    #[test]
    fn start_messages() {
        assert_eq!(start_code("/start abc"), Some("abc"));
        assert_eq!(start_code("  /start   abc  "), Some("abc"));
        assert_eq!(start_code("/start@kwant_bot abc"), Some("abc"));
        assert_eq!(start_code("/start"), Some(""));
        assert_eq!(start_code("/started abc"), None);
        assert_eq!(start_code("hello /start abc"), None);
    }

    #[test]
    fn the_qr_is_an_svg_of_the_link() {
        let svg = qr_svg("https://t.me/kwant_bot?start=AAAAAAAAAAAAAAAAAAAAAA");
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
        assert!(svg.contains("<path") && svg.matches('M').count() > 100);
        assert!(!svg.contains("start="), "the code is drawn, not written");
    }
}
