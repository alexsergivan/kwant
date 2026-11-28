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

/// No bot needs more than this many distinct findings in one list; past it, a linear dedup scan
/// on every call turns quadratic (Lin's review, 1c). A length check is O(1), so capping here
/// bounds the whole lint to O(n) in the input size again.
const MAX_FINDINGS: usize = 20;

impl Report {
    fn refuse(&mut self, rule: &'static str, sentence: String) {
        if self.refusals.len() >= MAX_FINDINGS {
            return;
        }
        if !self.refusals.iter().any(|f| f.rule == rule && f.sentence == sentence) {
            self.refusals.push(Finding { rule, sentence });
        }
    }

    fn warn(&mut self, rule: &'static str, sentence: String) {
        if self.warnings.len() >= MAX_FINDINGS {
            return;
        }
        if !self.warnings.iter().any(|f| f.rule == rule && f.sentence == sentence) {
            self.warnings.push(Finding { rule, sentence });
        }
    }
}

/// The lint: checks one surface's `html`, `css` and `js` against every style-guide.md section 7
/// rule, plus the cheap additions. One finding per rule per distinct value.
pub fn lint(surface: Surface, html: &str, css: &str, js: &str) -> Report {
    let mut r = Report::default();
    if lint_size(&mut r, html, css, js) {
        // Oversized: C6 alone, nothing else scanned (Lin's review, 1c).
        return r;
    }
    lint_html(&mut r, html);
    lint_css(&mut r, css);
    lint_js(&mut r, surface, js);
    r
}

/// Caps `s` at 40 printable-ASCII characters for embedding app-supplied text (an attribute name
/// or an input type) into a guest-facing sentence; anything past that, or any non-printable byte,
/// becomes `?` (Lin's review, 1d: this text is unbounded and goes to the bot and the host).
fn sanitize_for_sentence(s: &str) -> String {
    const MAX_LEN: usize = 40;
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= MAX_LEN {
            out.push('?');
            break;
        }
        if c.is_ascii() && (c.is_ascii_graphic() || c == ' ') {
            out.push(c);
        } else {
            out.push('?');
        }
    }
    out
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

/// Drops every `/* ... */` CSS comment (a comment inside a quoted string is left alone), so
/// `position:/**/fixed` reads as `position:fixed` instead of hiding the value from every
/// substring/keyword check below (Rigel's QA, B2).
fn strip_css_comments(css: &str) -> String {
    let b = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' => {
                let q = b[i];
                let start = i;
                i += 1;
                while i < b.len() && b[i] != q {
                    i += if b[i] == b'\\' && i + 1 < b.len() { 2 } else { 1 };
                }
                if i < b.len() {
                    i += 1;
                }
                out.push_str(&css[start..i.min(b.len())]);
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'*' => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
            }
            _ => {
                let len = utf8_char_len(b[i]).min(b.len() - i);
                out.push_str(&css[i..i + len]);
                i += len;
            }
        }
    }
    out
}

/// Decodes a CSS escape (`\` + up to 6 hex digits, with one optional trailing space, or `\` +
/// one literal character) wherever it sits outside a quoted string, so `\66ixed` reads as `fixed`
/// instead of dodging every keyword/value check below (Rigel's QA, B2: C1/C2/C8 all spell their
/// trigger word this way). A string literal's own content is copied through untouched, since
/// `string_literals`/`is_off_kwant` already expect to see its original escapes.
fn decode_css_escapes_outside_strings(css: &str) -> String {
    let b = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' => {
                let q = b[i];
                let start = i;
                i += 1;
                while i < b.len() && b[i] != q {
                    i += if b[i] == b'\\' && i + 1 < b.len() { 2 } else { 1 };
                }
                if i < b.len() {
                    i += 1;
                }
                out.push_str(&css[start..i.min(b.len())]);
            }
            b'\\' if i + 1 < b.len() => {
                let hex_start = i + 1;
                let mut j = hex_start;
                while j < b.len() && j - hex_start < 6 && b[j].is_ascii_hexdigit() {
                    j += 1;
                }
                if j > hex_start {
                    if let Ok(code) = u32::from_str_radix(&css[hex_start..j], 16) {
                        if let Some(c) = char::from_u32(code) {
                            out.push(c);
                        }
                    }
                    if j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    i = j;
                } else {
                    let len = utf8_char_len(b[hex_start]).min(b.len() - hex_start);
                    out.push_str(&css[hex_start..hex_start + len]);
                    i = hex_start + len;
                }
            }
            _ => {
                let len = utf8_char_len(b[i]).min(b.len() - i);
                out.push_str(&css[i..i + len]);
                i += len;
            }
        }
    }
    out
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

/// `prop: value` pairs read directly from `css`'s token stream in a single linear pass. The input
/// is split at every top-level `{`, `;` and `}` (a `'`/`"` string is skipped as one unit so a
/// boundary byte inside it can't be misread as one); a segment that ends at a `{` is a selector or
/// an at-rule prelude (`.x:hover`, `@media (min-width:1px)`) and is discarded whole, so its own
/// colon never gets misread as a `name: value` pair and swallows the real declaration that follows
/// (Rigel's QA, B2). Every other segment — ending at a `;`, a `}`, or end of input — is exactly one
/// statement, checked on its own for a `name: value` shape. Because every segment, at every CSS
/// nesting depth, is visited exactly once, an outer block's own declaration is never skipped just
/// because it nests another rule inside it (`.a { position: fixed; .b { margin: 0; } }`, Rigel's
/// QA, B4), and a block left open at end of input is still read up to there, same as a browser
/// applies it (Rigel's QA, B4) — without the quadratic blow-up of first collecting every `{ }`
/// body's byte range and then re-scanning each one's full text once per level it's nested at,
/// which is what the range-collecting version of this function used to do (Rigel's QA, B5). A
/// quote only closes the string at its matching quote byte: an unescaped newline, carriage
/// return or form feed ends it first, same as a browser (a CSS string can't contain a literal
/// newline), so everything after a dropped closing quote isn't swallowed into it and hidden from
/// this scan (Rigel's QA, B6); that stray newline byte is then read again by the outer loop as
/// ordinary text, since it's no longer inside the string.
fn declarations(css: &str) -> Vec<(String, String)> {
    let b = css.as_bytes();
    let mut out = Vec::new();
    let mut seg_start = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' => {
                let q = b[i];
                i += 1;
                while i < b.len() && b[i] != q && b[i] != b'\n' && b[i] != b'\r' && b[i] != 0x0c {
                    i += if b[i] == b'\\' && i + 1 < b.len() { 2 } else { 1 };
                }
                if i < b.len() && b[i] == q {
                    i += 1;
                }
            }
            b'{' => {
                seg_start = i + 1;
                i += 1;
            }
            b';' | b'}' => {
                push_declaration(&css[seg_start..i], &mut out);
                seg_start = i + 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    push_declaration(&css[seg_start..], &mut out);
    out
}

/// Reads the single `name: value` declaration out of one statement already isolated by
/// `declarations`'s boundary scan: `value` never needs to stop early at a `;`, `}` or `{` of its
/// own, since there isn't one left in `segment`.
fn push_declaration(segment: &str, out: &mut Vec<(String, String)>) {
    let b = segment.as_bytes();
    let mut i = 0;
    while i < b.len() && !(b[i].is_ascii_alphabetic() || b[i] == b'-') {
        i += 1;
    }
    let start = i;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-') {
        i += 1;
    }
    if i == start {
        return;
    }
    let name = segment[start..i].to_string();
    let mut j = i;
    while j < b.len() && b[j].is_ascii_whitespace() {
        j += 1;
    }
    if j < b.len() && b[j] == b':' {
        out.push((name, segment[j + 1..].trim().to_string()));
    }
}

// ---------------------------------------------------------------------------------------------
// C6: combined size budget (html + css + js at most 120 KB per surface)
// ---------------------------------------------------------------------------------------------

const MAX_SURFACE_BYTES: usize = 120 * 1024;

/// Returns `true` (and refuses C6) when the surface is oversized, so the caller can skip every
/// other scan: those are what made a huge, mostly-repetitive input quadratically slow (Lin's
/// review, 1c).
fn lint_size(r: &mut Report, html: &str, css: &str, js: &str) -> bool {
    if html.len() + css.len() + js.len() > MAX_SURFACE_BYTES {
        r.refuse("C6", "It's too big for a phone.".to_string());
        return true;
    }
    false
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

/// The guide's own casing for a matched M5 word (style-guide.md section 7's "Extra refusals"
/// table), since the matched list entry itself is lowercase for case-insensitive matching.
fn m5_display(word: &str) -> String {
    match word {
        "pin" => "PIN".to_string(),
        "cvv" => "CVV".to_string(),
        "cvc" => "CVC".to_string(),
        "iban" => "IBAN".to_string(),
        "expiry" => "card expiry date".to_string(),
        "log in" | "sign in" => "login".to_string(),
        "phone" => "phone number".to_string(),
        other => other.to_string(),
    }
}

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
            if (!allowed || bad_tabindex) && attr != "contenteditable" {
                r.refuse(
                    "M2",
                    format!(
                        "It put {attr} on a <{tag}>; style and events go in the CSS and the script.",
                        attr = sanitize_for_sentence(attr),
                        tag = t.name
                    ),
                );
            }

            if attr == "contenteditable" {
                r.refuse(
                    "M2",
                    format!("It let guests type into a <{tag}>; use an input.", tag = t.name),
                );
            }

            if t.name == "input" && attr == "type" {
                let v = value.trim().to_ascii_lowercase();
                if !M3_ALLOWED_INPUT_TYPES.contains(&v.as_str()) {
                    r.refuse("M3", format!("It asked for a {} field, and pocket apps can't.", sanitize_for_sentence(&v)));
                }
            }

            if attr == "autocomplete" {
                let v = value.trim().to_ascii_lowercase();
                if v != "off" && !v.is_empty() {
                    r.refuse("M4", "It asked the phone to fill in saved details.".to_string());
                }
            }

            if is_off_kwant(value) {
                r.refuse("M6", "It tried to load something from the web.".to_string());
            }
            // The guide says `src` is only ever an allowed data:image, stricter than the generic
            // off-kwant check above: a relative path or a backslash-disguised host (Lin's review,
            // section 2) isn't "off kwant" by scheme, but still isn't an allowed data:image.
            if attr == "src" && !is_allowed_data_image(value) {
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

    // M5: refuse/warn words, scanned only over the text the guide names ("in any label,
    // placeholder, legend, aria-label or text near an input"), not the whole page (Lin's review,
    // 1a): an M1 heading like "Pin your favourite" must not trip M5.
    let mut m5_haystack = String::new();
    for tag_name in ["label", "legend", "button"] {
        append_tag_text(html, &flat, tag_name, &mut m5_haystack);
    }
    for t in &tags {
        for (attr, value) in &t.attrs {
            if attr == "placeholder" || attr == "aria-label" {
                m5_haystack.push(' ');
                m5_haystack.push_str(value);
            }
        }
    }
    let m5_haystack_lower = m5_haystack.to_ascii_lowercase();
    for word in M5_REFUSE_WORDS {
        if contains_word(&m5_haystack_lower, word) {
            r.refuse(
                "M5",
                format!("It asked guests for their {}. Pocket apps never ask for that.", m5_display(word)),
            );
        }
    }
    for word in M5_WARN_WORDS {
        if contains_word(&m5_haystack_lower, word) {
            r.warn(
                "M5",
                format!("It asked guests for their {}. Pocket apps don't need that.", m5_display(word)),
            );
        }
    }

    // The copy rules keep scanning the whole page (Lin's review, 1a: "the copy warnings can keep
    // scanning the whole page"), plus placeholder/aria-label values the same way M5 used to.
    let mut copy_haystack = strip_tags(html);
    for t in &tags {
        for (attr, value) in &t.attrs {
            if attr == "placeholder" || attr == "aria-label" {
                copy_haystack.push(' ');
                copy_haystack.push_str(value);
            }
        }
    }
    let copy_haystack_lower = copy_haystack.to_ascii_lowercase();
    lint_copy(r, html, &copy_haystack_lower);
}

/// Appends the visible text inside every `<tag_name>...</tag_name>` span in `html` to `out`
/// (`lower` is `html`'s lowercase copy, reused by the caller to avoid recomputing it per tag).
fn append_tag_text(html: &str, lower: &str, tag_name: &str, out: &mut String) {
    let open = format!("<{tag_name}");
    let close = format!("</{tag_name}");
    let mut search_from = 0;
    while let Some(open_pos) = lower[search_from..].find(open.as_str()) {
        let tag_start = search_from + open_pos;
        let Some(gt) = lower[tag_start..].find('>') else { break };
        let body_start = tag_start + gt + 1;
        let Some(close_rel) = lower[body_start..].find(close.as_str()) else { break };
        let body_end = body_start + close_rel;
        out.push(' ');
        out.push_str(&strip_tags(&html[body_start..body_end]));
        search_from = body_end + close.len();
    }
}

// ---------------------------------------------------------------------------------------------
// copy rules (warnings only)
// ---------------------------------------------------------------------------------------------

const BANNED_BUTTON_WORDS: &[&str] = &["submit", "ok", "click here"];
const BANNED_CLAIM_WORDS: &[&str] = &["checked", "verified", "safe", "official"];

fn lint_copy(r: &mut Report, html: &str, haystack_lower: &str) {
    for word in BANNED_CLAIM_WORDS {
        if contains_word(haystack_lower, word) {
            let mut chars = word.chars();
            let cap_word = match chars.next() {
                Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
                None => word.to_string(),
            };
            r.warn("copy", format!("It said \"{cap_word}\"; only kwant says that."));
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
            r.warn("copy", format!("A button said \"{trimmed}\"; say what happens instead, like \"Vote\"."));
        }
        search_from = body_end + 1;
    }
}

// ---------------------------------------------------------------------------------------------
// CSS: C1-C9
// ---------------------------------------------------------------------------------------------

const C3_ALLOWED_KEYWORDS: &[&str] = &["transparent", "currentcolor", "inherit"];
// `color-mix(` is deliberately not in this allowlist: it's only let through when its own
// arguments pass the same hex/rgb-function/named-color checks below, so `color-mix(in srgb, red
// 10%, blue)` still gets refused instead of smuggling a named colour past the guide (Rigel's QA,
// "not blocking, but worth fixing").
const C3_ALLOWED_FN_PREFIXES: &[&str] = &["var(--kw-", "var(--zinc-", "var(--sky-", "var(--team-"];

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

/// Drops a trailing `!important` (any case, any spacing before it) so a value like
/// `fixed !important` still compares equal to `fixed` (Lin's review, section 2: C2 and C7 both
/// passed when a bot added `!important`).
fn strip_important(value: &str) -> &str {
    let v = value.trim();
    let lower = v.to_ascii_lowercase();
    match lower.find("!important") {
        Some(pos) => v[..pos].trim_end(),
        None => v,
    }
}

fn lint_css(r: &mut Report, css_in: &str) {
    // Comments and CSS escapes are resolved once, up front, so every check below (including
    // `declarations()`'s own boundary scan) sees a bot's actual property names and values
    // instead of a `/**/`- or `\XX`-obscured version of them (Rigel's QA, B2).
    let normalized = decode_css_escapes_outside_strings(&strip_css_comments(css_in));
    let css: &str = &normalized;
    let lower = css.to_ascii_lowercase();

    // C1: one stylesheet's rules only; no @import / @font-face / @namespace; url() only data:image.
    for at_rule in ["@import", "@font-face", "@namespace"] {
        if contains_word(&lower, at_rule) {
            r.refuse("C1", "Its CSS tried to load a file or a font.".to_string());
        }
    }
    scan_urls(css, r, "C1", "Its CSS tried to load a file or a font.");

    // C1/M6: an off-kwant URL hidden in a CSS string literal (e.g. `image-set("https://...")`),
    // the same check the JS side already runs over its own string literals (Lin's review,
    // section 2).
    for lit in string_literals(css) {
        if is_off_kwant(lit) {
            r.refuse("C1", "Its CSS tried to load a file or a font.".to_string());
            r.refuse("M6", "It tried to load something from the web.".to_string());
        }
    }

    // C2: no position: fixed, even with `!important` tacked on.
    for (name, value) in declarations(css) {
        if name.eq_ignore_ascii_case("position") && strip_important(&value).eq_ignore_ascii_case("fixed") {
            r.refuse("C2", "It tried to pin something over the page.".to_string());
        }
    }

    // C3: colours only through kwant's own custom properties (or transparent/currentColor/inherit).
    for (name, value) in declarations(css) {
        let nl = name.to_ascii_lowercase();
        if !(nl == "color"
            || nl.starts_with("background")
            || nl.starts_with("border")
            || nl.starts_with("outline")
            || nl.ends_with("-color")
            || nl == "fill"
            || nl == "stroke"
            || nl == "box-shadow"
            || nl == "text-shadow"
            || nl == "column-rule")
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
            if let Ok(n) = strip_important(&value).parse::<i64>() {
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
        r.refuse("C8", "Its CSS hid typed letters like a password box.".to_string());
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
        // A number starts at a digit (`160ms`) or at a leading dot immediately followed by one
        // (`.16s`, valid CSS for 160ms): without the second arm, the leading `.` was skipped as
        // punctuation and the scan started on the `16` that follows, misreading `.16s` as `16s`
        // (16000ms, outside the 120-200ms window) instead of the 160ms it actually is (Rigel's
        // QA, "not blocking, but worth fixing").
        let starts_number =
            b[i].is_ascii_digit() || (b[i] == b'.' && b.get(i + 1).is_some_and(|c| c.is_ascii_digit()));
        if starts_number {
            let start = i;
            let mut seen_dot = b[i] == b'.';
            i += 1;
            while i < b.len() && (b[i].is_ascii_digit() || (b[i] == b'.' && !seen_dot)) {
                if b[i] == b'.' {
                    seen_dot = true;
                }
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
            let after_byte = lower.as_bytes().get(end).copied();
            // A longhand like `animation-name`/`transition-property` is still the property this
            // rule means to catch; `is_word_byte` counts `-` as a word byte, so without this the
            // boundary check below would treat it as a different, unmatched identifier (Lin's
            // review, section 2).
            let after_ok = after_byte.is_none()
                || !is_word_byte(after_byte.unwrap())
                || (after_byte == Some(b'-') && (token == "animation" || token == "transition"));
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
    "execCommand(",
];

fn lint_js(r: &mut Report, surface: Surface, js: &str) {
    // J1: word-matched tokens.
    for (s, e) in identifiers(js) {
        let ident = &js[s..e];
        if J1_WORD_TOKENS.contains(&ident) {
            r.refuse("J1", format!("Its script used {ident}, which pocket apps can't."));
        }
    }
    // The substring checks below are punctuation-disambiguated (`fetch(`, `window[`, ...), so a
    // bot can dodge them with a space before the punctuation (`fetch ('/x')`); run them over a
    // whitespace-collapsed copy instead (Lin's review, section 2).
    let collapsed = collapse_call_whitespace(js);
    // J1: punctuation-disambiguated substrings.
    for token in J1_SUBSTRING_TOKENS {
        if collapsed.contains(token) {
            r.refuse("J1", format!("Its script used {token}, which pocket apps can't."));
        }
    }
    for q in ["setAttribute('style'", "setAttribute(\"style\""] {
        if collapsed.contains(q) {
            r.refuse("J1", "Its script used setAttribute('style'), which pocket apps can't.".to_string());
        }
    }
    // J1: setTimeout/setInterval called with a string body, the classic eval-alike.
    for name in ["setTimeout(", "setInterval("] {
        let mut from = 0;
        while let Some(pos) = collapsed[from..].find(name) {
            let after = from + pos + name.len();
            let b = collapsed.as_bytes();
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
    // J1: a unicode escape outside any string/template literal is how a bare identifier can spell
    // a refused word (e.g. "top") without that exact token ever appearing in the source, dodging
    // the word-matched check above; a pocket app's JS never needs this outside a string (Lin's
    // review, section 2).
    if has_unicode_escape_outside_strings(js) {
        r.refuse("J1", "Its script hid a name behind a \\u escape, which pocket apps can't.".to_string());
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

/// Drops ASCII whitespace that sits directly before `(`, `[` or `.`, so `fetch ('/x')` and
/// `window [ 'top' ]` match the same punctuation-disambiguated substrings as their un-spaced
/// forms (Lin's review, section 2). Not full tokenizing: the checks this feeds already match on
/// plain substrings, so collapsing is enough.
fn collapse_call_whitespace(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            let mut j = i;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && matches!(chars[j], '(' | '[' | '.') {
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Whether a JS unicode escape appears outside any string/template literal: that's the one place
/// a bare identifier can legitimately carry one, and it's exactly what lets a refused word (e.g.
/// "top") dodge the word-matched J1 tokens above without that literal token ever appearing.
fn has_unicode_escape_outside_strings(src: &str) -> bool {
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'\'' || c == b'"' || c == b'`' {
            let q = c;
            i += 1;
            while i < b.len() && b[i] != q {
                if b[i] == b'\\' && i + 1 < b.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if i < b.len() {
                i += 1;
            }
            continue;
        }
        if c == b'\\' && i + 1 < b.len() && b[i + 1] == b'u' {
            return true;
        }
        i += 1;
    }
    false
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
