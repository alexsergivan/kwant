//! Keys, codes and PIN hashes for pockets. Guest and host keys are 32 random bytes (base64url),
//! kept only as sha256. PINs and reset codes are kept only as argon2id with a per-player salt.

use base64::Engine;
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};

/// Pocket codes: no 0/o, 1/l/i, so a code read out loud or typed from a poster works.
const CODE_ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
pub const CODE_LEN: usize = 6;
/// Reset codes: 4 characters, upper case, same look-alike rule.
const RESET_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
pub const RESET_LEN: usize = 4;

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    SystemRandom::new().fill(&mut buf).expect("the OS random generator works");
    buf
}

fn random_from(alphabet: &[u8], len: usize) -> String {
    // Rejection sampling keeps every character equally likely
    let limit = 256 - (256 % alphabet.len());
    let mut out = String::with_capacity(len);
    while out.len() < len {
        for b in random_bytes::<32>() {
            if (b as usize) < limit && out.len() < len {
                out.push(alphabet[b as usize % alphabet.len()] as char);
            }
        }
    }
    out
}

/// A new key: 32 random bytes, base64url without padding (43 characters).
pub fn new_key() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes::<32>())
}

pub fn hash_key(key: &str) -> String {
    hex(&Sha256::digest(key.as_bytes()))
}

pub fn new_code() -> String {
    random_from(CODE_ALPHABET, CODE_LEN)
}

/// A pocket code as it appears in a path: exactly [`CODE_LEN`] characters of the alphabet.
pub fn valid_code(code: &str) -> bool {
    code.len() == CODE_LEN && code.bytes().all(|b| CODE_ALPHABET.contains(&b))
}

pub fn new_reset_code() -> String {
    random_from(RESET_ALPHABET, RESET_LEN)
}

/// A reset code as typed: case and spaces don't matter.
pub fn normalize_reset_code(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase()
}

pub fn new_salt() -> String {
    hex(&random_bytes::<16>())
}

pub fn new_player_id() -> String {
    format!("pl_{}", random_from(CODE_ALPHABET, 10))
}

/// A PIN as a pocket takes it: exactly 4 digits.
pub fn valid_pin(pin: &str) -> bool {
    pin.len() == 4 && pin.bytes().all(|b| b.is_ascii_digit())
}

/// argon2id (19 MiB, 2 passes; OWASP's minimum) of `secret` with the player's salt, as hex.
pub fn hash_secret(secret: &str, salt_hex: &str) -> String {
    let params = argon2::Params::new(19 * 1024, 2, 1, Some(32)).expect("fixed argon2 params are valid");
    let mut out = [0u8; 32];
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
        .hash_password_into(secret.as_bytes(), salt_hex.as_bytes(), &mut out)
        .expect("argon2 with a 32-character salt works");
    hex(&out)
}

pub fn secret_matches(secret: &str, salt_hex: &str, hash_hex: &str) -> bool {
    let got = hash_secret(secret, salt_hex);
    got.len() == hash_hex.len() && got.bytes().zip(hash_hex.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_codes_and_hashes() {
        let k = new_key();
        assert_eq!(k.len(), 43);
        assert_ne!(k, new_key());
        assert_eq!(hash_key(&k).len(), 64);
        let c = new_code();
        assert!(valid_code(&c), "{c}");
        assert!(!valid_code("ABCDEF") && !valid_code("abc") && !valid_code("abcde0"));
        assert_eq!(new_reset_code().len(), 4);
        assert_eq!(normalize_reset_code(" k7 qx "), "K7QX");
        assert!(valid_pin("0042") && !valid_pin("42") && !valid_pin("12a4") && !valid_pin("12345"));
        let salt = new_salt();
        let h = hash_secret("1234", &salt);
        assert!(secret_matches("1234", &salt, &h));
        assert!(!secret_matches("1235", &salt, &h));
        assert_ne!(h, hash_secret("1234", &new_salt()), "the salt is per player");
    }
}
