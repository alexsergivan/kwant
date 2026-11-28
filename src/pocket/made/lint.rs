//! The lint for bot-written pocket app screens (pa12 K2): the first gate a `made` app's
//! `{html, css, js}` passes through, before the sandbox, CSP and Trusted Types take over as the
//! real security boundary (security-review.md T3, T8, T9). Pure functions, no I/O: a small
//! allowlist tokenizer, not a browser-grade HTML/CSS/JS parser, so it is cheap to run on every
//! save and every bot turn.
//!
//! Every rule below is style-guide.md section 7, word for word in its `sentence`, plus Rigel's
//! cheap additions from rigel-cases.md (AutoFill tokens past `off`, `-webkit-text-security`,
//! `contenteditable`, a `</script` breakout). Wiring this into a route, the builder or the check
//! pipeline is V3/K3/L5's job; a secrets scan is L7's (`secrets_in` lands in this file later).

/// Which surface an app's `{html, css, js}` is for: the rules differ only at J3 (the screen
/// document never calls `kwant.act`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Phone,
    Screen,
}

/// One rule hit: `rule` is its style-guide.md section 7 ID (`"M1".."J5"`, plus `"C8"`, `"C9"`,
/// `"J6"` for the cheap additions this file owns), `sentence` is the exact guest-facing wording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub rule: &'static str,
    pub sentence: String,
}

/// What `lint` found. `refusals` block the app; `warnings` (the copy rules) do not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub refusals: Vec<Finding>,
    pub warnings: Vec<Finding>,
}

impl Report {
    fn refuse(&mut self, rule: &'static str, sentence: String) {
        if !self.refusals.iter().any(|f| f.rule == rule && f.sentence == sentence) {
            self.refusals.push(Finding { rule, sentence });
        }
    }

    fn warn(&mut self, rule: &'static str, sentence: String) {
        if !self.warnings.iter().any(|f| f.rule == rule && f.sentence == sentence) {
            self.warnings.push(Finding { rule, sentence });
        }
    }
}

/// The lint: checks one surface's `html`, `css` and `js` against every style-guide.md section 7
/// rule, plus the cheap additions. One finding per rule per distinct value.
pub fn lint(surface: Surface, html: &str, css: &str, js: &str) -> Report {
    let mut r = Report::default();
    lint_size(&mut r, html, css, js);
    lint_html(&mut r, html);
    lint_css(&mut r, css);
    lint_js(&mut r, surface, js);
    r
}

// ---------------------------------------------------------------------------------------------
// shared scanning helpers
// ---------------------------------------------------------------------------------------------

fn utf8_char_len(b0: u8) -> usize {
    if b0 < 0x80 {
        1
    } else if b0 >> 5 == 0b110 {
        2
    } else if b0 >> 4 == 0b1110 {
        3
    } else if b0 >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

/// `true` if `c` can be part of a bare identifier or a CSS property/at-rule name, for the word
/// and token boundary checks below.
fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c == b'-'
}

/// Whether `phrase` occurs in `haystack` (already same-case) with a non-word byte (or the string
/// edge) on both sides, so `"pin"` does not match inside `"spinning"`.
fn contains_word(haystack: &str, phrase: &str) -> bool {
    if phrase.is_empty() {
        return false;
    }
    let hb = haystack.as_bytes();
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(phrase) {
        let s = start + pos;
        let e = s + phrase.len();
        let before_ok = s == 0 || !is_word_byte(hb[s - 1]);
        let after_ok = e >= hb.len() || !is_word_byte(hb[e]);
        if before_ok && after_ok {
            return true;
        }
        start = s + 1;
        if start >= haystack.len() {
            break;
        }
    }
    false
}

fn is_allowed_data_image(value: &str) -> bool {
    let v = value.trim().to_ascii_lowercase();
    const ALLOWED: &[&str] = &["data:image/png", "data:image/gif", "data:image/webp", "data:image/svg+xml"];
    ALLOWED.iter().any(|p| v.starts_with(p))
}

/// Returns `true` if `value` points off kwant: not an allowed `data:image` URI, and it carries a
/// network or script scheme (M6: "anywhere in markup, CSS or script").
fn is_off_kwant(value: &str) -> bool {
    if is_allowed_data_image(value) {
        return false;
    }
    let v = value.trim().to_ascii_lowercase();
    if v.starts_with("javascript:") || v.starts_with("blob:") || v.starts_with("http:") || v.starts_with("https:") {
        return true;
    }
    if v.starts_with("//") && v.as_bytes().get(2).is_some_and(|c| c.is_ascii_alphanumeric()) {
        return true;
    }
    false
}

/// Every string literal's content (quotes stripped, escapes left as-is: we only need substring
/// and prefix checks, never the exact runtime value).
fn string_literals(src: &str) -> Vec<&str> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'\'' || c == b'"' || c == b'`' {
            let q = c;
            let start = i + 1;
            let mut j = start;
            while j < b.len() && b[j] != q {
                if b[j] == b'\\' && j + 1 < b.len() {
                    j += 2;
                } else {
                    j += 1;
                }
            }
            out.push(&src[start..j.min(b.len())]);
            i = if j < b.len() { j + 1 } else { j };
        } else {
            i += 1;
        }
    }
    out
}

/// Bare identifier byte ranges (ASCII letters/digits/`_`/`$`, not starting with a digit).
fn identifiers(src: &str) -> Vec<(usize, usize)> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_alphabetic() || b[i] == b'_' || b[i] == b'$' {
            let s = i;
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$') {
                i += 1;
            }
            out.push((s, i));
        } else {
            i += 1;
        }
    }
    out
}

/// `prop: value` pairs scanned flat over any CSS text (no selector/nesting awareness: cheap but
/// enough to check a declaration's value).
fn declarations(css: &str) -> Vec<(String, String)> {
    let b = css.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && !(b[i].is_ascii_alphabetic() || b[i] == b'-') {
            i += 1;
        }
        let start = i;
        while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-') {
            i += 1;
        }
        if i == start {
            continue;
        }
        let name = css[start..i].to_string();
        let mut j = i;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if j < b.len() && b[j] == b':' {
            j += 1;
            let vstart = j;
            while j < b.len() && b[j] != b';' && b[j] != b'}' {
                j += 1;
            }
            out.push((name, css[vstart..j].trim().to_string()));
            i = j;
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// C6: combined size budget (html + css + js at most 120 KB per surface)
// ---------------------------------------------------------------------------------------------

const MAX_SURFACE_BYTES: usize = 120 * 1024;

fn lint_size(r: &mut Report, html: &str, css: &str, js: &str) {
    if html.len() + css.len() + js.len() > MAX_SURFACE_BYTES {
        r.refuse("C6", "It's too big for a phone.".to_string());
    }
}

// ---------------------------------------------------------------------------------------------
// markup: M1-M7
// ---------------------------------------------------------------------------------------------

const M1_ALLOWED_TAGS: &[&str] = &[
    "div", "section", "header", "footer", "main", "h1", "h2", "h3", "p", "span", "b", "strong", "em", "small", "ul",
    "ol", "li", "button", "label", "input", "select", "option", "textarea", "fieldset", "legend", "img", "svg",
    "rect", "circle", "path", "g", "title", "template", "time", "br", "hr",
];

const M2_ALLOWED_ATTRS: &[&str] = &[
    "id", "class", "type", "name", "value", "min", "max", "step", "maxlength", "placeholder", "autocomplete", "for",
    "role", "hidden", "disabled", "checked", "selected", "alt", "src", "width", "height", "viewbox", "x", "y", "rx",
    "cx", "cy", "r", "d", "fill", "datetime", "lang", "dir", "tabindex",
];

const M3_ALLOWED_INPUT_TYPES: &[&str] = &["text", "number", "radio", "checkbox", "range"];

const M5_REFUSE_WORDS: &[&str] = &[
    "password",
    "passcode",
    "pin",
    "card number",
    "credit card",
    "debit card",
    "cvv",
    "cvc",
    "security code",
    "expiry",
    "sort code",
    "iban",
    "bank account",
    "log in",
    "sign in",
    "verification code",
    "one-time code",
    "seed phrase",
];

const M5_WARN_WORDS: &[&str] = &["email", "phone", "address", "birthday"];

/// Tokens that let AutoFill reach past the guest-typed value into the phone's saved details
/// (Rigel's F8, cheap at lint level): anything other than absent or `off`.
const M4_LISTED_TOKENS: &[&str] = &[
    "username",
    "email",
    "tel",
    "street-address",
    "name",
    "current-password",
    "new-password",
    "one-time-code",
];

/// One `<tag attr="value" ...>` or `</tag>`: `closing` is true for the latter, `attrs` is empty
/// for it too.
struct Tag {
    closing: bool,
    name: String,
    attrs: Vec<(String, String)>,
}

/// Scans `html` for every `<...>` tag, honoring quoted attribute values so a `>` inside one does
/// not end the tag early. Not a full HTML parser: no entity decoding, no void-element table, no
/// DOCTYPE/comment awareness beyond skipping past them as unmatched `<` text.
fn scan_tags(html: &str) -> Vec<Tag> {
    let b = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += utf8_char_len(b[i]);
            continue;
        }
        let mut j = i + 1;
        let closing = j < b.len() && b[j] == b'/';
        if closing {
            j += 1;
        }
        let name_start = j;
        while j < b.len() && b[j].is_ascii_alphanumeric() {
            j += 1;
        }
        if j == name_start {
            // "<!--", "<!doctype", or a lone '<': not a tag we can name, skip the char.
            i += 1;
            continue;
        }
        let name = html[name_start..j].to_ascii_lowercase();
        let mut attrs = Vec::new();
        let mut k = j;
        while k < b.len() && b[k] != b'>' {
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if k >= b.len() || b[k] == b'>' {
                break;
            }
            if b[k] == b'/' {
                k += 1;
                continue;
            }
            let an_start = k;
            while k < b.len() && b[k] != b'=' && b[k] != b'>' && b[k] != b'/' && !b[k].is_ascii_whitespace() {
                k += 1;
            }
            if k == an_start {
                k += 1;
                continue;
            }
            let attr_name = html[an_start..k].to_ascii_lowercase();
            let before_eq = k;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            let mut value = String::new();
            if k < b.len() && b[k] == b'=' {
                k += 1;
                while k < b.len() && b[k].is_ascii_whitespace() {
                    k += 1;
                }
                if k < b.len() && (b[k] == b'"' || b[k] == b'\'') {
                    let q = b[k];
                    k += 1;
                    let vs = k;
                    while k < b.len() && b[k] != q {
                        k += 1;
                    }
                    value = html[vs..k.min(b.len())].to_string();
                    if k < b.len() {
                        k += 1;
                    }
                } else {
                    let vs = k;
                    while k < b.len() && !b[k].is_ascii_whitespace() && b[k] != b'>' {
                        k += 1;
                    }
                    value = html[vs..k].to_string();
                }
            } else {
                k = before_eq; // boolean attribute, no '='
            }
            attrs.push((attr_name, value));
        }
        out.push(Tag { closing, name, attrs });
        i = if k < b.len() { k + 1 } else { k };
    }
    out
}

/// Tag and attribute text stripped out, leaving the document's visible words (quoted attribute
/// values are dropped, not kept): used for the M5/copy word scans over label, legend and button
/// text. `placeholder`/`aria-label` values are added back in separately by the caller, since
/// they live only inside a tag.
fn strip_tags(html: &str) -> String {
    let b = html.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    let mut in_tag = false;
    while i < b.len() {
        if !in_tag && b[i] == b'<' {
            in_tag = true;
            i += 1;
            continue;
        }
        if in_tag {
            if b[i] == b'"' || b[i] == b'\'' {
                let q = b[i];
                i += 1;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                if i < b.len() {
                    i += 1;
                }
                continue;
            }
            if b[i] == b'>' {
                in_tag = false;
                i += 1;
                out.push(' ');
                continue;
            }
            i += 1;
            continue;
        }
        let len = utf8_char_len(b[i]).min(b.len() - i);
        out.push_str(&html[i..i + len]);
        i += len;
    }
    out
}

fn lint_html(r: &mut Report, html: &str) {
    let tags = scan_tags(html);

    for t in &tags {
        if !M1_ALLOWED_TAGS.contains(&t.name.as_str()) {
            r.refuse("M1", format!("It used a <{}>, and pocket apps can't use that.", t.name));
        }
        if t.closing {
            continue;
        }

        for (attr, value) in &t.attrs {
            let allowed = M2_ALLOWED_ATTRS.contains(&attr.as_str()) || attr.starts_with("aria-");
            let bad_tabindex = attr == "tabindex" && value.trim() != "0" && value.trim() != "-1";
            if !allowed || bad_tabindex {
                r.refuse(
                    "M2",
                    format!("It put {attr} on a <{tag}>; style and events go in the CSS and the script.", attr = attr, tag = t.name),
                );
            }

            if attr == "contenteditable" {
                r.refuse(
                    "M2",
                    format!("It put {attr} on a <{tag}>; style and events go in the CSS and the script.", attr = attr, tag = t.name),
                );
            }

            if t.name == "input" && attr == "type" {
                let v = value.trim().to_ascii_lowercase();
                if !M3_ALLOWED_INPUT_TYPES.contains(&v.as_str()) {
                    r.refuse("M3", format!("It asked for a {} field, and pocket apps can't.", v));
                }
            }

            if attr == "autocomplete" {
                let v = value.trim().to_ascii_lowercase();
                if v != "off" && !v.is_empty() {
                    r.refuse("M4", "It asked the phone to fill in saved details.".to_string());
                }
                if M4_LISTED_TOKENS.contains(&v.as_str()) || v.starts_with("cc-") {
                    r.refuse("M4", "It asked the phone to fill in saved details.".to_string());
                }
            }

            if is_off_kwant(value) {
                r.refuse("M6", "It tried to load something from the web.".to_string());
            }

            if attr == "class" || attr == "id" {
                for tok in value.split_whitespace() {
                    let tl = tok.to_ascii_lowercase();
                    if tl.starts_with("kw-") || tl.starts_with("kw_") {
                        r.refuse("M7", "It used kwant's own names; those are kwant's alone.".to_string());
                    }
                }
            }
        }
    }

    // M6, read across the whole document text too: "anywhere in markup ... except" the SVG
    // namespace string inside an already-allowed data:image URI.
    let flat = html.to_ascii_lowercase();
    for scheme in ["javascript:", "blob:"] {
        if flat.contains(scheme) {
            r.refuse("M6", "It tried to load something from the web.".to_string());
        }
    }

    // M5: refuse/warn words, scanned over visible text plus placeholder/aria-label attributes.
    let mut haystack = strip_tags(html);
    for t in &tags {
        for (attr, value) in &t.attrs {
            if attr == "placeholder" || attr == "aria-label" {
                haystack.push(' ');
                haystack.push_str(value);
            }
        }
    }
    let haystack_lower = haystack.to_ascii_lowercase();
    for word in M5_REFUSE_WORDS {
        if contains_word(&haystack_lower, word) {
            r.refuse("M5", format!("It asked guests for a {word}. Pocket apps never ask for that."));
        }
    }
    for word in M5_WARN_WORDS {
        if contains_word(&haystack_lower, word) {
            r.warn("M5", format!("It asked guests for a {word}. Pocket apps never ask for that."));
        }
    }

    lint_copy(r, html, &tags, &haystack_lower);
}

// ---------------------------------------------------------------------------------------------
// copy rules (warnings only)
// ---------------------------------------------------------------------------------------------

const BANNED_BUTTON_WORDS: &[&str] = &["submit", "ok", "click here"];
const BANNED_CLAIM_WORDS: &[&str] = &["checked", "verified", "safe", "official"];

fn lint_copy(r: &mut Report, html: &str, tags: &[Tag], haystack_lower: &str) {
    for word in BANNED_CLAIM_WORDS {
        if contains_word(haystack_lower, word) {
            r.warn("copy", format!("It called something \"{word}\"; pocket apps don't claim that."));
        }
    }

    // button inner text: look for <button ...>...</button> spans and check the exact wording.
    let lower = html.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(open_pos) = lower[search_from..].find("<button") {
        let tag_start = search_from + open_pos;
        let Some(gt) = lower[tag_start..].find('>') else { break };
        let body_start = tag_start + gt + 1;
        let Some(close_rel) = lower[body_start..].find("</button") else { break };
        let body_end = body_start + close_rel;
        let inner = strip_tags(&html[body_start..body_end]);
        let trimmed = inner.trim();
        let trimmed_lower = trimmed.to_ascii_lowercase();
        if BANNED_BUTTON_WORDS.contains(&trimmed_lower.as_str()) {
            r.warn("copy", format!("A button said \"{trimmed}\"; pocket apps use a word that says what it does."));
        }
        search_from = body_end + 1;
    }
    let _ = tags;
}

// ---------------------------------------------------------------------------------------------
// CSS: C1-C9
// ---------------------------------------------------------------------------------------------

const C3_ALLOWED_KEYWORDS: &[&str] = &["transparent", "currentcolor", "inherit"];
const C3_ALLOWED_FN_PREFIXES: &[&str] = &["var(--kw-", "var(--zinc-", "var(--sky-", "var(--team-", "color-mix("];

const C3_NAMED_COLORS: &[&str] = &[
    "aliceblue", "antiquewhite", "aqua", "aquamarine", "azure", "beige", "bisque", "black", "blanchedalmond", "blue",
    "blueviolet", "brown", "burlywood", "cadetblue", "chartreuse", "chocolate", "coral", "cornflowerblue",
    "cornsilk", "crimson", "cyan", "darkblue", "darkcyan", "darkgoldenrod", "darkgray", "darkgreen", "darkgrey",
    "darkkhaki", "darkmagenta", "darkolivegreen", "darkorange", "darkorchid", "darkred", "darksalmon",
    "darkseagreen", "darkslateblue", "darkslategray", "darkslategrey", "darkturquoise", "darkviolet", "deeppink",
    "deepskyblue", "dimgray", "dimgrey", "dodgerblue", "firebrick", "floralwhite", "forestgreen", "fuchsia",
    "gainsboro", "ghostwhite", "gold", "goldenrod", "gray", "green", "greenyellow", "grey", "honeydew", "hotpink",
    "indianred", "indigo", "ivory", "khaki", "lavender", "lavenderblush", "lawngreen", "lemonchiffon", "lightblue",
    "lightcoral", "lightcyan", "lightgoldenrodyellow", "lightgray", "lightgreen", "lightgrey", "lightpink",
    "lightsalmon", "lightseagreen", "lightskyblue", "lightslategray", "lightslategrey", "lightsteelblue",
    "lightyellow", "lime", "limegreen", "linen", "magenta", "maroon", "mediumaquamarine", "mediumblue",
    "mediumorchid", "mediumpurple", "mediumseagreen", "mediumslateblue", "mediumspringgreen", "mediumturquoise",
    "mediumvioletred", "midnightblue", "mintcream", "mistyrose", "moccasin", "navajowhite", "navy", "oldlace",
    "olive", "olivedrab", "orange", "orangered", "orchid", "palegoldenrod", "palegreen", "paleturquoise",
    "palevioletred", "papayawhip", "peachpuff", "peru", "pink", "plum", "powderblue", "purple", "rebeccapurple",
    "red", "rosybrown", "royalblue", "saddlebrown", "salmon", "sandybrown", "seagreen", "seashell", "sienna",
    "silver", "skyblue", "slateblue", "slategray", "slategrey", "snow", "springgreen", "steelblue", "tan", "teal",
    "thistle", "tomato", "turquoise", "violet", "wheat", "white", "whitesmoke", "yellow", "yellowgreen",
];

fn scan_urls(css: &str, r: &mut Report, rule: &'static str, sentence: &str) {
    let lower = css.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find("url(") {
        let start = from + pos + 4;
        let Some(end_rel) = css[start..].find(')') else { break };
        let end = start + end_rel;
        let raw = css[start..end].trim();
        let unquoted = raw.trim_matches(|c| c == '"' || c == '\'');
        if is_off_kwant(unquoted) || (!is_allowed_data_image(unquoted) && !unquoted.is_empty()) {
            r.refuse(rule, sentence.to_string());
        }
        from = end + 1;
    }
}

fn is_allowed_color_value(value_lower: &str) -> bool {
    let v = value_lower.trim();
    if C3_ALLOWED_KEYWORDS.contains(&v) {
        return true;
    }
    C3_ALLOWED_FN_PREFIXES.iter().any(|p| v.starts_with(p))
}

fn lint_css(r: &mut Report, css: &str) {
    let lower = css.to_ascii_lowercase();

    // C1: one stylesheet's rules only; no @import / @font-face / @namespace; url() only data:image.
    for at_rule in ["@import", "@font-face", "@namespace"] {
        if contains_word(&lower, at_rule) {
            r.refuse("C1", "Its CSS tried to load a file or a font.".to_string());
        }
    }
    scan_urls(css, r, "C1", "Its CSS tried to load a file or a font.");

    // C2: no position: fixed.
    for (name, value) in declarations(css) {
        if name.eq_ignore_ascii_case("position") && value.trim().eq_ignore_ascii_case("fixed") {
            r.refuse("C2", "It tried to pin something over the page.".to_string());
        }
    }

    // C3: colours only through kwant's own custom properties (or transparent/currentColor/inherit).
    for (name, value) in declarations(css) {
        let nl = name.to_ascii_lowercase();
        if !(nl == "color"
            || nl.starts_with("background")
            || nl == "border-color"
            || nl.ends_with("-color")
            || nl == "fill"
            || nl == "stroke"
            || nl == "outline-color")
        {
            continue;
        }
        let vl = value.to_ascii_lowercase();
        if is_allowed_color_value(&vl) {
            continue;
        }
        let mut bad = false;
        // hex colours: '#' + 3/4/6/8 hex digits, not immediately followed by another hex digit.
        let vb = vl.as_bytes();
        for (i, &c) in vb.iter().enumerate() {
            if c == b'#' {
                let mut j = i + 1;
                while j < vb.len() && vb[j].is_ascii_hexdigit() {
                    j += 1;
                }
                let len = j - (i + 1);
                if matches!(len, 3 | 4 | 6 | 8) {
                    bad = true;
                }
            }
        }
        for func in ["rgb(", "rgba(", "hsl(", "hsla(", "hwb(", "lab(", "lch("] {
            if vl.contains(func) {
                bad = true;
            }
        }
        for name in C3_NAMED_COLORS {
            if contains_word(&vl, name) {
                bad = true;
            }
        }
        if bad {
            r.refuse("C3", "It used its own colour; use kwant's colour names.".to_string());
        }
    }

    // C4: font-family (and the font shorthand) only inherit.
    for (name, value) in declarations(css) {
        let nl = name.to_ascii_lowercase();
        if (nl == "font-family" || nl == "font") && !value.trim().eq_ignore_ascii_case("inherit") {
            r.refuse("C4", "It changed the font.".to_string());
        }
    }

    // C5: animation/transition/@keyframes must sit inside an explicit reduced-motion guard,
    // run 120-200ms, and only animate transform/opacity.
    lint_css_motion(r, css, &lower);

    // C6 is checked once, combined, by lint_size.

    // C7: no z-index above 10; no oversized negative margins.
    for (name, value) in declarations(css) {
        let nl = name.to_ascii_lowercase();
        if nl == "z-index" {
            if let Ok(n) = value.trim().parse::<i64>() {
                if n > 10 {
                    r.refuse("C7", "It tried to cover part of the page.".to_string());
                }
            }
        }
        if nl.starts_with("margin") {
            for tok in value.split_whitespace() {
                let t = tok.trim_end_matches("px");
                if let Ok(n) = t.parse::<f64>() {
                    if n < -999.0 {
                        r.refuse("C7", "It tried to cover part of the page.".to_string());
                    }
                }
            }
        }
    }

    // C8 (cheap addition): -webkit-text-security can disguise typed text as a password.
    if lower.contains("-webkit-text-security") {
        r.refuse("C8", "It used -webkit-text-security, which can disguise typed text as a password.".to_string());
    }

    // C9 (cheap addition): a CSS payload trying to close the style tag early.
    if lower.contains("</style") {
        r.refuse("C9", "Its CSS tried to close the style tag.".to_string());
    }

    // M6, CSS side, outside of url(...): javascript:/blob: never belong in CSS text at all, so a
    // global scan is safe. http:/https: are not scanned globally here because the SVG namespace
    // exception legitimately puts "http://www.w3.org/2000/svg" inside an allowed data:image URI;
    // scan_urls above already refuses a url(...) whose scheme isn't an allowed data:image.
    for scheme in ["javascript:", "blob:"] {
        if lower.contains(scheme) {
            r.refuse("M6", "It tried to load something from the web.".to_string());
        }
    }
}

fn block_after(css: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut i = from;
    while i < css.len() && css[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= css.len() || css[i] != b'{' {
        return None;
    }
    let mut depth = 0i32;
    let start = i;
    while i < css.len() {
        match css[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((start + 1, i));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn match_media_guard(css: &str, at: usize) -> Option<usize> {
    let s = &css[at..];
    let sb = s.as_bytes();
    let mut i = 0;
    let lit = |sb: &[u8], i: &mut usize, token: &str| -> bool {
        while *i < sb.len() && sb[*i].is_ascii_whitespace() {
            *i += 1;
        }
        let tb = token.as_bytes();
        if *i + tb.len() <= sb.len() && sb[*i..*i + tb.len()].eq_ignore_ascii_case(tb) {
            *i += tb.len();
            true
        } else {
            false
        }
    };
    if !lit(sb, &mut i, "(") {
        return None;
    }
    if !lit(sb, &mut i, "prefers-reduced-motion") {
        return None;
    }
    if !lit(sb, &mut i, ":") {
        return None;
    }
    if !lit(sb, &mut i, "no-preference") {
        return None;
    }
    if !lit(sb, &mut i, ")") {
        return None;
    }
    Some(at + i)
}

fn durations_ms(s: &str) -> Vec<f64> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            let num_str = &s[start..i];
            if i + 1 < b.len() && (b[i] == b'm' || b[i] == b'M') && (b[i + 1] == b's' || b[i + 1] == b'S') {
                if let Ok(n) = num_str.parse::<f64>() {
                    out.push(n);
                }
                i += 2;
            } else if i < b.len() && (b[i] == b's' || b[i] == b'S') {
                if let Ok(n) = num_str.parse::<f64>() {
                    out.push(n * 1000.0);
                }
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

fn lint_css_motion(r: &mut Report, css: &str, lower: &str) {
    // Collect byte ranges that sit inside a `@media (prefers-reduced-motion: no-preference) {...}` guard.
    let mut guarded: Vec<(usize, usize)> = Vec::new();
    let mut from = 0;
    while let Some(pos) = lower[from..].find("@media") {
        let at = from + pos + "@media".len();
        if let Some(after_guard) = match_media_guard(css, at) {
            if let Some((inner_start, inner_end)) = block_after(css.as_bytes(), after_guard) {
                guarded.push((inner_start, inner_end));
                for (name, _) in declarations(&css[inner_start..inner_end]) {
                    let _ = name;
                }
            }
        }
        from = at;
    }

    let in_guard = |pos: usize| guarded.iter().any(|(s, e)| pos >= *s && pos < *e);

    for token in ["animation", "transition", "@keyframes"] {
        let mut from = 0;
        while let Some(pos) = lower[from..].find(token) {
            let abs = from + pos;
            let before_ok = abs == 0 || !is_word_byte(lower.as_bytes()[abs - 1]);
            let end = abs + token.len();
            let after_ok = end >= lower.len() || !is_word_byte(lower.as_bytes()[end]);
            if before_ok && after_ok && !in_guard(abs) {
                r.refuse("C5", "It moves without checking 'reduce motion'.".to_string());
            }
            from = abs + token.len();
        }
    }

    for (start, end) in &guarded {
        let block = &css[*start..*end];
        for d in durations_ms(block) {
            if !(120.0..=200.0).contains(&d) {
                r.refuse("C5", "It moves without checking 'reduce motion'.".to_string());
            }
        }
        for (name, value) in declarations(block) {
            let nl = name.to_ascii_lowercase();
            if nl == "transition" {
                for clause in value.split(',') {
                    let first = clause.split_whitespace().next().unwrap_or("");
                    let fl = first.to_ascii_lowercase();
                    if !fl.is_empty() && fl != "transform" && fl != "opacity" {
                        r.refuse("C5", "It moves without checking 'reduce motion'.".to_string());
                    }
                }
            }
        }
        // inside @keyframes blocks: only transform/opacity may be set per frame.
        for (name, _) in declarations(block) {
            let nl = name.to_ascii_lowercase();
            if nl != "transform" && nl != "opacity" && !nl.is_empty() && block.contains('%') {
                // best-effort: a non-transform/opacity declaration inside a guarded block that
                // also contains frame selectors ("0%", "50%", ...) is very likely a @keyframes body.
                if lower[*start..*end].contains('%') {
                    r.refuse("C5", "It moves without checking 'reduce motion'.".to_string());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// script: J1-J6
// ---------------------------------------------------------------------------------------------

/// J1 tokens matched as whole identifiers only (`topping` is fine, `top` is not).
const J1_WORD_TOKENS: &[&str] = &[
    "eval",
    "import",
    "Worker",
    "SharedWorker",
    "serviceWorker",
    "RTCPeerConnection",
    "WebTransport",
    "WebSocket",
    "EventSource",
    "XMLHttpRequest",
    "sendBeacon",
    "location",
    "top",
    "parent",
    "opener",
    "frames",
    "postMessage",
    "cssText",
    "attachShadow",
    "trustedTypes",
    "alert",
    "confirm",
    "prompt",
    "print",
    "requestFullscreen",
    "clipboard",
    "Notification",
    "vibrate",
    "localStorage",
    "sessionStorage",
    "indexedDB",
    "caches",
    "innerHTML",
    "outerHTML",
    "insertAdjacentHTML",
    "createContextualFragment",
    "DOMParser",
];

/// J1 tokens matched as plain substrings: their own punctuation already tells them apart from an
/// ordinary identifier, so a word-boundary check would just add false negatives.
const J1_SUBSTRING_TOKENS: &[&str] = &[
    "Function(",
    "require(",
    "fetch(",
    "open(",
    "document.cookie",
    "document.write",
    ".style",
    "window[",
    "self[",
    "globalThis[",
    "document[",
];

fn lint_js(r: &mut Report, surface: Surface, js: &str) {
    // J1: word-matched tokens.
    for (s, e) in identifiers(js) {
        let ident = &js[s..e];
        if J1_WORD_TOKENS.contains(&ident) {
            r.refuse("J1", format!("Its script used {ident}, which pocket apps can't."));
        }
    }
    // J1: punctuation-disambiguated substrings.
    for token in J1_SUBSTRING_TOKENS {
        if js.contains(token) {
            r.refuse("J1", format!("Its script used {token}, which pocket apps can't."));
        }
    }
    for q in ["setAttribute('style'", "setAttribute(\"style\""] {
        if js.contains(q) {
            r.refuse("J1", "Its script used setAttribute('style'), which pocket apps can't.".to_string());
        }
    }
    // J1: setTimeout/setInterval called with a string body, the classic eval-alike.
    for name in ["setTimeout(", "setInterval("] {
        let mut from = 0;
        while let Some(pos) = js[from..].find(name) {
            let after = from + pos + name.len();
            let b = js.as_bytes();
            let mut k = after;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if k < b.len() && (b[k] == b'\'' || b[k] == b'"' || b[k] == b'`') {
                r.refuse("J1", format!("Its script used {}, which pocket apps can't.", name.trim_end_matches('(')));
            }
            from = after;
        }
    }

    // J2: events only through addEventListener.
    if has_on_assignment(js) {
        r.refuse("J2", "It wired an event the old way.".to_string());
    }

    // J3: the only cross-frame calls are kwant.onState/kwant.act; act names are a fixed literal
    // list; the screen document never calls kwant.act.
    lint_js_kwant(r, surface, js);

    // J4: text from state only via textContent/value/alt/aria-*/title; images only from
    // s.faces or kwant's own data: images (M6 handles the scheme check on any literal).
    lint_js_src_assignment(r, js);

    // J5: one script element, no src (checked against the raw js text: a bot's script text
    // should never itself contain a <script> tag).
    if js.to_ascii_lowercase().contains("<script") {
        r.refuse("J5", "It had more than one script.".to_string());
    }
    // J6 (cheap addition): a JS payload trying to break out of its own <script> element.
    if js.to_ascii_lowercase().contains("</script") {
        r.refuse("J6", "Its script tried to close the script tag.".to_string());
    }

    // M6, JS side: string literals pointing off kwant.
    for lit in string_literals(js) {
        if is_off_kwant(lit) {
            r.refuse("M6", "It tried to load something from the web.".to_string());
        }
    }
}

fn has_on_assignment(js: &str) -> bool {
    let b = js.as_bytes();
    let mut i = 0;
    while let Some(pos) = js[i..].find(".on") {
        let start = i + pos;
        let mut j = start + 3;
        let name_start = j;
        while j < b.len() && b[j].is_ascii_lowercase() {
            j += 1;
        }
        if j > name_start {
            let mut k = j;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if k < b.len() && b[k] == b'=' && (k + 1 >= b.len() || b[k + 1] != b'=') {
                return true;
            }
        }
        i = start + 3;
        if i >= js.len() {
            break;
        }
    }
    false
}

const J3_ALLOWED_ACT_NAMES: &[&str] = &["put", "take_back", "answer", "vote"];

fn lint_js_kwant(r: &mut Report, surface: Surface, js: &str) {
    let mut from = 0;
    while let Some(pos) = js[from..].find("kwant.") {
        let after = from + pos + "kwant.".len();
        let b = js.as_bytes();
        let name_start = after;
        let mut k = name_start;
        while k < b.len() && (b[k].is_ascii_alphanumeric() || b[k] == b'_') {
            k += 1;
        }
        let name = &js[name_start..k];

        if surface == Surface::Screen && name == "act" {
            r.refuse("J3", "The big screen tried to change something.".to_string());
            from = k;
            continue;
        }

        if name != "onState" && name != "act" {
            r.refuse("J3", "It tried an action guests can't do.".to_string());
        } else if name == "act" {
            // first string-literal argument must be exactly one of the allowed act names.
            let mut m = k;
            while m < b.len() && b[m].is_ascii_whitespace() {
                m += 1;
            }
            let mut ok = false;
            if m < b.len() && b[m] == b'(' {
                let mut n = m + 1;
                while n < b.len() && b[n].is_ascii_whitespace() {
                    n += 1;
                }
                if n < b.len() && (b[n] == b'\'' || b[n] == b'"') {
                    let q = b[n];
                    let vs = n + 1;
                    let mut ve = vs;
                    while ve < b.len() && b[ve] != q {
                        ve += 1;
                    }
                    let arg = &js[vs..ve.min(b.len())];
                    ok = J3_ALLOWED_ACT_NAMES.contains(&arg);
                }
            }
            if !ok {
                r.refuse("J3", "It tried an action guests can't do.".to_string());
            }
        }
        from = k;
    }
}

/// J4's "images only from `s.faces` or kwant's own `data:` images" is a dataflow claim this
/// lexical scanner cannot verify (the M1 worked example sets `img.src = src` from a plain local
/// variable fed by `s.faces[id]`, which must keep passing). So this only catches the part that
/// *is* lexically checkable: a `.src` assigned straight from a string literal that isn't an
/// allowed `data:image` URI (a non-literal source is trusted, same as the shipped example;
/// M6 still refuses a literal off-kwant URL wherever it appears, including here).
fn lint_js_src_assignment(r: &mut Report, js: &str) {
    let b = js.as_bytes();
    let mut from = 0;
    while let Some(pos) = js[from..].find(".src") {
        let start = from + pos;
        let mut k = start + 4;
        while k < b.len() && b[k].is_ascii_whitespace() {
            k += 1;
        }
        if k < b.len() && b[k] == b'=' && (k + 1 >= b.len() || b[k + 1] != b'=') {
            k += 1;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if k < b.len() && (b[k] == b'\'' || b[k] == b'"' || b[k] == b'`') {
                let q = b[k];
                let vs = k + 1;
                let mut ve = vs;
                while ve < b.len() && b[ve] != q {
                    ve += 1;
                }
                let value = &js[vs..ve.min(b.len())];
                if !is_allowed_data_image(value) {
                    r.refuse("J4", "It put guests' words in as code.".to_string());
                }
            }
        }
        from = start + 4;
    }
}
