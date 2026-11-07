//! Which fields are masked (decisions.md Q-S1). The recorder in the page applies the same rule so
//! a masked value never leaves the browser, and the server applies it again to whatever arrives:
//! a value the page sent for a masked field is dropped here, never stored.

/// Why a field is masked, in precedence order: the first that holds wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mask {
    Password,
    Code,
    Payment,
    Secret,
    SavedLogin,
}

impl Mask {
    pub fn as_str(self) -> &'static str {
        match self {
            Mask::Password => "password",
            Mask::Code => "code",
            Mask::Payment => "payment",
            Mask::Secret => "secret",
            Mask::SavedLogin => "saved_login",
        }
    }

    pub fn parse(s: &str) -> Option<Mask> {
        [Mask::Password, Mask::Code, Mask::Payment, Mask::Secret, Mask::SavedLogin]
            .into_iter()
            .find(|m| m.as_str() == s)
    }

    /// What a step shows instead of the value.
    pub fn placeholder(self) -> &'static str {
        match self {
            Mask::Password => "[your password: use the saved login]",
            Mask::Code => "[code]",
            Mask::Payment => "[card details]",
            Mask::Secret => "[secret]",
            Mask::SavedLogin => "[filled from your saved login]",
        }
    }
}

/// Words that mask a field when its name, id or label contains one (case-insensitive substring),
/// exactly decisions.md's list, with the kind each one gives.
pub const MASK_WORDS: &[(&str, Mask)] = &[
    ("passw", Mask::Password),
    ("otp", Mask::Code),
    ("code", Mask::Code),
    ("pin", Mask::Code),
    ("card", Mask::Payment),
    ("cvv", Mask::Payment),
    ("cvc", Mask::Payment),
    ("iban", Mask::Payment),
    ("ssn", Mask::Secret),
    ("secret", Mask::Secret),
    ("token", Mask::Secret),
];

/// What the recorder knows about a field.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Field<'a> {
    /// The `type` attribute now.
    pub input_type: &'a str,
    /// The field was `type=password` at any time on this page (a show-password toggle).
    pub ever_password: bool,
    /// The `autocomplete` attribute (space-separated tokens).
    pub autocomplete: &'a str,
    pub name: &'a str,
    pub id: &'a str,
    /// Every label the field has: `<label>`, `aria-label`, `aria-labelledby`, `placeholder`.
    pub labels: &'a [&'a str],
    /// The vault, or any script, set its value (an untrusted input event).
    pub script_filled: bool,
}

/// The mask for a field, or `None` when its value may be recorded.
pub fn mask_for(f: &Field) -> Option<Mask> {
    let tokens: Vec<String> = f.autocomplete.split_whitespace().map(|t| t.to_ascii_lowercase()).collect();
    let has_token = |pred: &dyn Fn(&str) -> bool| tokens.iter().any(|t| pred(t));
    let haystacks: Vec<String> = [f.name, f.id]
        .into_iter()
        .chain(f.labels.iter().copied())
        .map(|s| s.to_lowercase())
        .collect();
    let has_word = |kind: Mask| {
        MASK_WORDS
            .iter()
            .filter(|(_, k)| *k == kind)
            .any(|(w, _)| haystacks.iter().any(|h| h.contains(w)))
    };
    if f.input_type.eq_ignore_ascii_case("password")
        || f.ever_password
        || has_token(&|t| t == "current-password" || t == "new-password")
        || has_word(Mask::Password)
    {
        return Some(Mask::Password);
    }
    if has_token(&|t| t == "one-time-code") || has_word(Mask::Code) {
        return Some(Mask::Code);
    }
    if has_token(&|t| t.starts_with("cc-")) || has_word(Mask::Payment) {
        return Some(Mask::Payment);
    }
    if has_word(Mask::Secret) {
        return Some(Mask::Secret);
    }
    if f.script_filled {
        return Some(Mask::SavedLogin);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> Option<Mask> {
        mask_for(&Field { input_type: "text", name, ..Default::default() })
    }

    #[test]
    fn type_password_masks_now_or_ever() {
        assert_eq!(mask_for(&Field { input_type: "password", ..Default::default() }), Some(Mask::Password));
        assert_eq!(mask_for(&Field { input_type: "PASSWORD", ..Default::default() }), Some(Mask::Password));
        // A show-password toggle turned it into text: still masked
        assert_eq!(mask_for(&Field { input_type: "text", ever_password: true, ..Default::default() }), Some(Mask::Password));
    }

    #[test]
    fn every_autocomplete_token_in_the_rule() {
        for (ac, want) in [
            ("one-time-code", Mask::Code),
            ("current-password", Mask::Password),
            ("new-password", Mask::Password),
            ("cc-number", Mask::Payment),
            ("cc-csc", Mask::Payment),
            ("cc-exp", Mask::Payment),
            ("cc-name", Mask::Payment),
            ("section-pay billing cc-number", Mask::Payment),
            ("username ONE-TIME-CODE", Mask::Code),
        ] {
            let f = Field { input_type: "text", autocomplete: ac, ..Default::default() };
            assert_eq!(mask_for(&f), Some(want), "{ac}");
        }
        for ac in ["email", "username", "off", "name", "street-address"] {
            let f = Field { input_type: "text", autocomplete: ac, ..Default::default() };
            assert_eq!(mask_for(&f), None, "{ac}");
        }
    }

    #[test]
    fn every_word_in_the_list_masks_by_name_id_or_label() {
        for (word, want) in MASK_WORDS {
            assert_eq!(named(&format!("user_{word}_field")), Some(*want), "name {word}");
            let up = word.to_uppercase();
            assert_eq!(mask_for(&Field { id: &up, ..Default::default() }), Some(*want), "id {word}");
            let label = format!("Your {up} here");
            let labels = [label.as_str()];
            assert_eq!(mask_for(&Field { labels: &labels, ..Default::default() }), Some(*want), "label {word}");
        }
    }

    #[test]
    fn real_world_names() {
        assert_eq!(named("Passwort"), Some(Mask::Password));
        assert_eq!(named("totp"), Some(Mask::Code));
        assert_eq!(named("verification_code"), Some(Mask::Code));
        assert_eq!(named("PIN"), Some(Mask::Code));
        assert_eq!(named("cardNumber"), Some(Mask::Payment));
        assert_eq!(named("cvc2"), Some(Mask::Payment));
        assert_eq!(named("IBAN"), Some(Mask::Payment));
        assert_eq!(named("ssn_last4"), Some(Mask::Secret));
        assert_eq!(named("client_secret"), Some(Mask::Secret));
        assert_eq!(named("api_token"), Some(Mask::Secret));
        // Not on the list (Q-S9): recorded as typed
        for plain in ["pwd", "apikey", "2fa", "email", "q", "search", "date_range"] {
            assert_eq!(named(plain), None, "{plain}");
        }
    }

    #[test]
    fn a_field_the_vault_filled_is_masked_as_a_saved_login() {
        let f = Field { input_type: "email", name: "email", script_filled: true, ..Default::default() };
        assert_eq!(mask_for(&f), Some(Mask::SavedLogin));
        let f = Field { input_type: "text", name: "username", script_filled: false, ..Default::default() };
        assert_eq!(mask_for(&f), None);
    }

    #[test]
    fn the_first_row_wins() {
        // password beats code, code beats payment, payment beats secret, all beat saved_login
        let f = Field { input_type: "password", name: "pin", script_filled: true, ..Default::default() };
        assert_eq!(mask_for(&f), Some(Mask::Password));
        let f = Field { name: "card_code", script_filled: true, ..Default::default() };
        assert_eq!(mask_for(&f), Some(Mask::Code));
        let f = Field { name: "card_token", ..Default::default() };
        assert_eq!(mask_for(&f), Some(Mask::Payment));
        let f = Field { name: "secret", script_filled: true, ..Default::default() };
        assert_eq!(mask_for(&f), Some(Mask::Secret));
    }

    #[test]
    fn kinds_round_trip_and_have_their_placeholders() {
        for m in [Mask::Password, Mask::Code, Mask::Payment, Mask::Secret, Mask::SavedLogin] {
            assert_eq!(Mask::parse(m.as_str()), Some(m));
            assert!(m.placeholder().starts_with('[') && m.placeholder().ends_with(']'));
        }
        assert_eq!(Mask::Password.placeholder(), "[your password: use the saved login]");
        assert_eq!(Mask::parse("other"), None);
    }
}
