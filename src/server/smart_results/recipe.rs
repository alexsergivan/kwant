//! The recipe selector: one recipe in a reply, read as a `recipe` block whose source is the run of
//! top-level blocks it fills, from the dish heading to its last list.
//!
//! It reads only the shape bots write: an optional dish heading and Serves/Prep line, an
//! "Ingredients" heading (or line), one or more bullet lists (each optionally under a group label
//! like "For the sauce:"), then the method (a numbered list, or a list under Method / Instructions
//! / Steps / Directions / Preparation), then optional Notes / Tips. Every block of that run must
//! be one of these; anything else between, or more than one recipe in the reply, and there is no
//! block. Contract: docs/smart-results.md.

use super::selector::{is_task, parse, words, Doc, List, Top};
use super::validate::{has_html, utf16_len};
use super::{Ingredient, Provenance, Recipe, RecipeStep, RecipeTime, Source, MAX_REPLY};

pub(super) const INGREDIENTS: (usize, usize) = (2, 40);
pub(super) const STEPS: (usize, usize) = (2, 30);
pub(super) const MAX_NOTES: usize = 6;
pub(super) const MAX_TIMES: usize = 3;
pub(super) const SERVES: (u32, u32) = (1, 24);
pub(super) const MAX_LEAD: usize = 12;
pub(super) const TITLE_MAX: usize = 300;
pub(super) const TIME_LABEL_MAX: usize = 40;
pub(super) const TIME_TEXT_MAX: usize = 60;
pub(super) const INGREDIENT_MAX: usize = 300;
pub(super) const QTY_MAX: usize = 40;
pub(super) const GROUP_MAX: usize = 100;
pub(super) const STEP_MAX: usize = 600;
pub(super) const NOTE_MAX: usize = 600;

/// The fraction glyphs an amount may use (design.md §3, `scaleQty`'s `LEAD`).
pub(super) const GLYPHS: [char; 6] = ['¼', '½', '¾', '⅓', '⅔', '⅛'];

/// The reply's one recipe, as [`recipe`] reads it (for grounding a `present_result` call).
pub fn recipe_of(text: &str) -> Option<Recipe> {
    if text.len() > MAX_REPLY {
        return None;
    }
    recipe(&parse(text)).map(|(_, r)| r)
}

/// The reply's one recipe and the line its section starts on.
pub(super) fn recipe(doc: &Doc) -> Option<(usize, Recipe)> {
    // Exactly one ingredients cue, and at most one method cue, in the whole reply
    let mut cue = None;
    let mut cues = 0;
    let mut methods = 0;
    for k in 0..doc.blocks.len() {
        if let Some(c) = ingredients_cue(doc, k) {
            cues += 1;
            cue = Some((k, c));
        }
        if label_at(doc, k).is_some_and(|t| is_method(label(t))) {
            methods += 1;
        }
    }
    if cues != 1 || methods > 1 {
        return None;
    }
    let (c, cue) = cue?;
    let mut meta = Meta::default();
    let mut title = None;
    // Where the section starts, and where the walk forward from the cue does
    let (start, mut k) = match cue {
        Cue::Unplaceable => return None,
        // "Here's what you'll need for the ingredients:" stays text; the section starts at its list
        Cue::Intro => (c + 1, c + 1),
        Cue::Exact(serves) => {
            meta.serve(serves)?;
            // Back from the cue: Serves/Prep lines, then the dish heading
            let mut start = c;
            while start > 0 && meta_at(doc, start - 1, true).is_some() {
                start -= 1;
            }
            for j in start..c {
                meta.merge(meta_at(doc, j, true)?)?;
            }
            if start > 0 {
                if let Some(t) = title_at(doc, start - 1) {
                    title = Some(t);
                    start -= 1;
                }
            }
            // Forward: a Serves/Prep line right under the cue
            let mut k = c + 1;
            if let Some(m) = meta_at(doc, k, false) {
                meta.merge(m)?;
                k += 1;
            }
            (start, k)
        }
    };
    // The ingredient lists, each under an optional group label
    let mut groups: Vec<(Option<&str>, &List)> = Vec::new();
    loop {
        if let Some(l) = bullets_at(doc, k) {
            groups.push((None, l));
            k += 1;
            continue;
        }
        match (label_at(doc, k).filter(|t| is_group(doc, k, t)), bullets_at(doc, k + 1)) {
            (Some(t), Some(l)) => {
                groups.push((Some(label(t)), l));
                k += 2;
            }
            _ => break,
        }
    }
    if groups.is_empty() {
        return None;
    }
    // The method: a list under a method cue, a numbered list under a one-line intro, or a
    // numbered list straight after the ingredients
    let (method, at) = match (label_at(doc, k), list_at(doc, k), list_at(doc, k + 1)) {
        (Some(t), _, Some(l)) if is_method(label(t)) => (l, k + 1),
        (Some(t), _, Some(l)) if l.ordered && is_step_intro(doc, k, t) => (l, k + 1),
        (_, Some(l), _) if l.ordered => (l, k),
        _ => return None,
    };
    let mut end = at;
    let k = at + 1;
    // A second list right after the method (a split method, more steps): not one card
    if list_at(doc, k).is_some() {
        return None;
    }
    let mut notes: Option<&List> = None;
    if let (Some(t), Some(l)) = (label_at(doc, k), list_at(doc, k + 1)) {
        if is_notes(label(t)) && (1..=MAX_NOTES).contains(&l.items.len()) {
            notes = Some(l);
            end = k + 1;
        } else if l.ordered {
            // "For the sauce:" then more numbered steps
            return None;
        }
    }
    // Every list of the section reads the same in the chat, and nothing up to its end is uncertain
    let section = &doc.blocks[start..=end];
    let lists: Vec<&List> = section
        .iter()
        .filter_map(|b| match b {
            Top::List(n) => Some(&doc.lists[*n]),
            _ => None,
        })
        .collect();
    if lists.iter().any(|l| !l.clean || l.start >= doc.list_unsure_at(l.ordered)) {
        return None;
    }
    let after = doc.starts.get(end + 1).copied().unwrap_or(doc.lines.len());
    if doc.unsure_at() < after {
        return None;
    }
    let lead = section.iter().position(|b| matches!(b, Top::List(_)))?;
    if lead > MAX_LEAD {
        return None;
    }
    let (head, tail) = (lists[0], lists[lists.len() - 1]);
    // Another ingredients-then-numbered-list pair outside the section: a second dish
    if second_pair(doc, start, end) {
        return None;
    }
    // A plan by day or meal is not a method
    if method.items.iter().filter(|t| plan_like(t)).count() >= 2 {
        return None;
    }

    let mut ingredients = Vec::new();
    for (group, l) in &groups {
        for item in &l.items {
            let (qty, text) = split_qty(strip_task(item));
            ingredients.push(Ingredient {
                id: format!("i{}", ingredients.len()),
                text: text.to_string(),
                qty: qty.map(str::to_string),
                group: group.map(str::to_string),
            });
        }
    }
    let steps = method.items.iter().enumerate().map(|(n, t)| RecipeStep { id: format!("s{n}"), text: t.to_string() });
    let r = Recipe {
        id: "b0".into(),
        title: title.map(str::to_string),
        source: Source::section((head.index, head.ordered), (tail.index, tail.ordered), lead),
        serves: meta.serves,
        times: meta.times.iter().map(|(l, t)| RecipeTime { label: l.to_string(), text: t.to_string() }).collect(),
        ingredients,
        steps: steps.collect(),
        notes: notes.map(|l| l.items.iter().map(|t| t.to_string()).collect()).unwrap_or_default(),
        provenance: Provenance::reply_text(),
    };
    fits(&r).then_some((doc.starts[start], r))
}

/// Within the contract's limits (the ones `validate` checks), so a reply past one stays text
/// rather than becoming a refused block.
fn fits(r: &Recipe) -> bool {
    let within = |s: &str, max: usize| !s.trim().is_empty() && utf16_len(s) <= max && !has_html(s);
    (INGREDIENTS.0..=INGREDIENTS.1).contains(&r.ingredients.len())
        && (STEPS.0..=STEPS.1).contains(&r.steps.len())
        && r.notes.len() <= MAX_NOTES
        && r.times.len() <= MAX_TIMES
        && r.title.as_deref().is_none_or(|t| within(t, TITLE_MAX))
        && r.times.iter().all(|t| within(&t.label, TIME_LABEL_MAX) && within(&t.text, TIME_TEXT_MAX))
        && r.ingredients.iter().all(|i| {
            within(&i.text, INGREDIENT_MAX)
                && i.qty.as_deref().is_none_or(|q| within(q, QTY_MAX))
                && i.group.as_deref().is_none_or(|g| within(g, GROUP_MAX))
        })
        && r.steps.iter().all(|s| within(&s.text, STEP_MAX))
        && r.notes.iter().all(|n| within(n, NOTE_MAX))
}

// The blocks of a section

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cue {
    /// "Ingredients", alone or with a servings count ("Ingredients (serves 4)", "Ingredients for 4").
    Exact(Option<u32>),
    /// A one-line intro ending in ":" that names the ingredients, right before a bullet list.
    Intro,
    /// A heading or label that mentions ingredients some other way ("## Ingredients and method"):
    /// it counts as the reply's cue, but this reader doesn't place it.
    Unplaceable,
}

fn ingredients_cue(doc: &Doc, k: usize) -> Option<Cue> {
    let t = label_at(doc, k)?;
    let l = label(t);
    let lower = l.to_lowercase();
    if matches!(lower.as_str(), "ingredients" | "ingredient list" | "ingredients list") {
        return Some(Cue::Exact(None));
    }
    if let Some(rest) = lower.strip_prefix("ingredients") {
        let rest = rest.trim();
        let rest = rest.strip_prefix('(').and_then(|r| r.strip_suffix(')')).unwrap_or(rest);
        if let Some(n) = serves_in(rest) {
            return Some(Cue::Exact(Some(n)));
        }
    }
    if !words(l).iter().any(|w| w == "ingredients" || w == "ingredient") {
        return None;
    }
    let one_line_para = matches!(doc.blocks[k], Top::Para { .. });
    if one_line_para && ends_with_colon(t) && bullets_at(doc, k + 1).is_some() {
        return Some(Cue::Intro);
    }
    if matches!(doc.blocks[k], Top::Heading(_)) || emphasised(t) || ends_with_colon(t) {
        return Some(Cue::Unplaceable);
    }
    None
}

/// A heading's text, or a one-line paragraph's line.
fn label_at<'a>(doc: &Doc<'a>, k: usize) -> Option<&'a str> {
    match doc.blocks.get(k)? {
        Top::Heading(h) => Some(h),
        Top::Para { start, end } if start == end => Some(doc.lines[*start].trim()),
        _ => None,
    }
}

fn list_at<'d>(doc: &'d Doc, k: usize) -> Option<&'d List<'d>> {
    match doc.blocks.get(k)? {
        Top::List(n) => Some(&doc.lists[*n]),
        _ => None,
    }
}

fn bullets_at<'d>(doc: &'d Doc, k: usize) -> Option<&'d List<'d>> {
    list_at(doc, k).filter(|l| !l.ordered)
}

/// `t` without wrapping emphasis or a trailing ":": "**For the sauce:**" → "For the sauce".
pub(super) fn label(t: &str) -> &str {
    let mut s = t.trim();
    loop {
        let before = s;
        s = s.strip_suffix(':').unwrap_or(s).trim_end();
        for w in ["**", "__", "*", "_"] {
            if s.len() > 2 * w.len() && s.starts_with(w) && s.ends_with(w) {
                s = s[w.len()..s.len() - w.len()].trim();
                break;
            }
        }
        if s == before {
            return s;
        }
    }
}

/// Wrapped in `**`/`__` (or `*`/`_`) as a whole, a trailing ":" aside.
fn emphasised(t: &str) -> bool {
    let s = t.trim();
    let s = s.strip_suffix(':').unwrap_or(s).trim_end();
    ["**", "__", "*", "_"].iter().any(|w| s.len() > 2 * w.len() && s.starts_with(w) && s.ends_with(w))
}

fn ends_with_colon(t: &str) -> bool {
    t.trim_end().trim_end_matches(['*', '_']).trim_end().ends_with(':')
}

const METHOD: [&str; 5] = ["method", "instructions", "steps", "directions", "preparation"];

/// "Method", "Instructions:", "**Steps**", "How to make it".
fn is_method(l: &str) -> bool {
    let w = words(l);
    !l.contains(|c: char| c.is_ascii_digit())
        && (1..=6).contains(&w.len())
        && (METHOD.contains(&w[0].as_str()) || w.starts_with(&["how".into(), "to".into(), "make".into()]))
}

/// "Notes", "Tips:", "Cook's notes".
fn is_notes(l: &str) -> bool {
    let w = words(l);
    !l.contains(|c: char| c.is_ascii_digit())
        && (1..=4).contains(&w.len())
        && w.iter().any(|w| matches!(w.as_str(), "notes" | "note" | "tips" | "tip"))
}

/// "Then follow these steps:" before a numbered method.
fn is_step_intro(doc: &Doc, k: usize, t: &str) -> bool {
    matches!(doc.blocks[k], Top::Para { .. }) && ends_with_colon(t) && words(t).len() <= 12 && !is_notes(label(t))
}

/// "For the sauce:", "**Topping**", "### Dressing" over an ingredient list.
fn is_group(doc: &Doc, k: usize, t: &str) -> bool {
    let l = label(t);
    let w = words(l);
    !l.is_empty()
        && l.chars().count() <= 60
        && (matches!(doc.blocks[k], Top::Heading(_)) || ends_with_colon(t) || emphasised(t))
        && !is_method(l)
        && !is_notes(l)
        && !w.iter().any(|w| matches!(w.as_str(), "step" | "steps" | "method" | "instructions" | "directions"))
        && meta_at(doc, k, false).is_none()
}

/// The dish name: a heading, or a line in bold, of at most 120 characters.
fn title_at<'a>(doc: &Doc<'a>, k: usize) -> Option<&'a str> {
    let t = label_at(doc, k)?;
    if !matches!(doc.blocks[k], Top::Heading(_)) && !emphasised(t) {
        return None;
    }
    let l = label(t);
    (!l.is_empty() && l.chars().count() <= 120 && !is_method(l) && !is_notes(l)).then_some(l)
}

/// A bullet list, then within two blocks a numbered list, both outside `start..=end`.
fn second_pair(doc: &Doc, start: usize, end: usize) -> bool {
    let outside = |p: usize| p < start || p > end;
    (0..doc.blocks.len()).filter(|&p| outside(p)).any(|p| {
        bullets_at(doc, p).is_some_and(|l| l.items.len() >= 2)
            && (p + 1..=p + 3).take_while(|&q| outside(q)).any(|q| list_at(doc, q).is_some_and(|l| l.ordered && l.items.len() >= 2))
    })
}

const PLAN_WORDS: [&str; 14] = [
    "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday", "day", "breakfast", "brunch", "lunch",
    "dinner", "supper", "snack",
];

fn plan_like(item: &str) -> bool {
    words(item).first().is_some_and(|w| PLAN_WORDS.contains(&w.as_str()))
}

fn strip_task(item: &str) -> &str {
    if is_task(item) {
        item[3..].trim_start()
    } else {
        item
    }
}

// Serves and times

#[derive(Default, Debug)]
struct Meta<'a> {
    serves: Option<u32>,
    times: Vec<(&'a str, &'a str)>,
}

impl<'a> Meta<'a> {
    /// `None` when it says a different number of servings than before.
    fn serve(&mut self, n: Option<u32>) -> Option<()> {
        match (self.serves, n) {
            (Some(a), Some(b)) if a != b => None,
            (_, Some(b)) => {
                self.serves = Some(b);
                Some(())
            }
            _ => Some(()),
        }
    }
    fn merge(&mut self, m: Meta<'a>) -> Option<()> {
        self.serve(m.serves)?;
        self.times.extend(m.times);
        (self.times.len() <= MAX_TIMES).then_some(())
    }
}

/// A paragraph (or, `lists` allowed, a list) that says only how many it serves and how long it
/// takes: "**Serves 4** · Prep 15 min · Cook 35 min".
fn meta_at<'a>(doc: &Doc<'a>, k: usize, lists: bool) -> Option<Meta<'a>> {
    let texts: Vec<&'a str> = match doc.blocks.get(k)? {
        Top::Para { start, end } => doc.lines[*start..=*end].to_vec(),
        Top::List(n) if lists => doc.lists[*n].items.clone(),
        _ => return None,
    };
    let mut m = Meta::default();
    let mut any = false;
    for t in texts {
        for seg in segments(t) {
            let (l, v) = parts(seg);
            if l.is_empty() && v.is_empty() {
                continue;
            }
            if let Some(n) = serves_of(l, v) {
                m.serve(Some(n))?;
            } else if is_time(l, v) {
                m.times.push((l, v));
            } else {
                return None;
            }
            any = true;
        }
    }
    (any && m.times.len() <= MAX_TIMES).then_some(m)
}

/// `s` cut at `·`, `•`, `|`, `;`, a spaced dash and ", ".
fn segments(s: &str) -> Vec<&str> {
    const WIDE: [&str; 4] = [" — ", " – ", " - ", ", "];
    let mut out = Vec::new();
    let mut from = 0;
    let mut i = 0;
    while i < s.len() {
        if let Some(w) = WIDE.iter().find(|w| s[i..].starts_with(**w)) {
            out.push(&s[from..i]);
            i += w.len();
            from = i;
            continue;
        }
        let c = s[i..].chars().next().unwrap_or(' ');
        if matches!(c, '·' | '•' | '|' | ';') {
            out.push(&s[from..i]);
            from = i + c.len_utf8();
        }
        i += c.len_utf8();
    }
    out.push(&s[from..]);
    out
}

/// A segment's label and value: "**Prep:** 15 min" → ("Prep", "15 min"), "Serves 4" →
/// ("Serves", "4"), "4 servings" → ("", "4 servings"), "Chill overnight" → ("Chill", "overnight").
fn parts(seg: &str) -> (&str, &str) {
    fn t(s: &str) -> &str {
        s.trim_matches(|c: char| c.is_whitespace() || c == '*' || c == '_')
    }
    let seg = t(seg);
    if let Some((l, v)) = seg.split_once(':') {
        return (t(l), t(v));
    }
    let cut = seg.find(|c: char| c.is_ascii_digit() || GLYPHS.contains(&c)).or_else(|| seg.to_ascii_lowercase().find("overnight"));
    match cut {
        Some(i) => (t(&seg[..i]), t(&seg[i..])),
        None => (seg, ""),
    }
}

/// "Serves 4", "Servings: 4", "Yield: 4", "4 servings", "for 4 people", "Makes 4 portions".
fn serves_of(label: &str, value: &str) -> Option<u32> {
    let mut w = value.split_whitespace();
    let n: u32 = w.next()?.parse().ok()?;
    let unit = w.next().map(str::to_lowercase);
    if w.next().is_some() {
        return None;
    }
    let unit_ok = unit.as_deref().is_none_or(|u| matches!(u, "servings" | "serving" | "people" | "persons" | "portions" | "portion"));
    let label_ok = match label.to_lowercase().as_str() {
        "serves" | "servings" | "serving" | "yield" | "yields" | "feeds" | "portions" | "for" => unit_ok,
        "" | "makes" => unit.is_some() && unit_ok,
        _ => false,
    };
    (label_ok && (SERVES.0..=SERVES.1).contains(&n)).then_some(n)
}

fn serves_in(s: &str) -> Option<u32> {
    let (l, v) = parts(s);
    serves_of(l, v)
}

const TIME_LABELS: [&str; 37] = [
    "prep", "preparation", "prep time", "preparation time", "cook", "cooking", "cook time", "cooking time", "bake",
    "baking", "bake time", "baking time", "total", "total time", "time", "rest", "resting", "resting time", "chill",
    "chilling", "chill time", "chilling time", "marinate", "marinating", "active", "active time", "inactive",
    "inactive time", "ready in", "rise", "rising", "proof", "proofing", "simmer", "roast", "roasting", "hands-on time",
];
const TIME_UNITS: [&str; 16] =
    ["min", "mins", "minute", "minutes", "hr", "hrs", "hour", "hours", "h", "m", "sec", "secs", "second", "seconds", "day", "days"];

/// "Prep" + "15 min", "Total time" + "1 hr 10 min", "Chill" + "overnight".
fn is_time(label: &str, value: &str) -> bool {
    let label = label.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    if !TIME_LABELS.contains(&label.as_str()) {
        return false;
    }
    let v = value.to_lowercase();
    if v == "overnight" {
        return true;
    }
    if !value.starts_with(|c: char| c.is_ascii_digit() || GLYPHS.contains(&c)) {
        return false;
    }
    let runs: Vec<&str> = v.split(|c: char| !c.is_alphabetic()).filter(|r| !r.is_empty()).collect();
    runs.iter().any(|r| TIME_UNITS.contains(r))
        && runs.iter().all(|r| TIME_UNITS.contains(r) || matches!(*r, "and" | "to" | "or" | "plus"))
}

// Amounts

/// Units written straight after an amount ("200g") or after one space ("1 tbsp."), which may
/// keep a ".".
const ABBREV: [&str; 17] = ["g", "kg", "mg", "ml", "l", "dl", "cl", "oz", "lb", "lbs", "tbsp", "tbs", "tsp", "pt", "qt", "cm", "mm"];
/// Unit words and Mira's countables, after one space.
const UNIT_WORDS: [&str; 78] = [
    "cup", "cups", "tablespoon", "tablespoons", "teaspoon", "teaspoons", "clove", "cloves", "can", "cans", "tin", "tins",
    "slice", "slices", "stick", "sticks", "sprig", "sprigs", "pinch", "pinches", "handful", "handfuls", "bunch", "bunches",
    "piece", "pieces", "jar", "jars", "packet", "packets", "package", "packages", "head", "heads", "stalk", "stalks",
    "sheet", "sheets", "knob", "knobs", "dash", "dashes", "drop", "drops", "pint", "pints", "quart", "quarts", "pound",
    "pounds", "ounce", "ounces", "gram", "grams", "kilogram", "kilograms", "litre", "litres", "liter", "liters",
    "millilitre", "millilitres", "milliliter", "milliliters", "onion", "onions", "lemon", "lemons", "egg", "eggs",
    "carrot", "carrots", "potato", "potatoes", "tomato", "tomatoes", "bulb", "bulbs",
];

/// An ingredient line's leading amount (with its unit) and the rest, such that the line is
/// `qty + " " + text`, or `qty + text` when `text` starts with ",". `(None, line)` when the line
/// doesn't start with an amount or doesn't split that way.
pub(super) fn split_qty(line: &str) -> (Option<&str>, &str) {
    let Some(amount) = amount_end(line) else { return (None, line) };
    let rest = &line[amount..];
    let glued = alpha_run(rest);
    let (qty, text) = if !glued.is_empty() {
        // "200g flour": only a known abbreviation may be glued on
        if !ABBREV.contains(&glued.to_lowercase().as_str()) {
            return (None, line);
        }
        let q = amount + glued.len();
        match line[q..].strip_prefix(' ') {
            Some(t) => (&line[..q], t),
            None if line[q..].starts_with(',') => (&line[..q], &line[q..]),
            None => return (None, line),
        }
    } else if rest.starts_with(',') {
        (&line[..amount], rest)
    } else if let Some(after) = rest.strip_prefix(' ') {
        let w = alpha_run(after);
        let lower = w.to_lowercase();
        let abbrev = ABBREV.contains(&lower.as_str());
        let mut q = amount + 1 + w.len();
        if abbrev && line[q..].starts_with('.') {
            q += 1;
        }
        let tail = &line[q..];
        let unit = !w.is_empty() && (abbrev || UNIT_WORDS.contains(&lower.as_str()));
        match tail.strip_prefix(' ') {
            Some(t) if unit && !t.is_empty() => (&line[..q], t),
            _ if unit && tail.starts_with(',') => (&line[..q], tail),
            // No unit, or one that would end the line ("3 eggs"): the amount alone
            _ => (&line[..amount], after),
        }
    } else {
        return (None, line);
    };
    let joined = if text.starts_with(',') { format!("{qty}{text}") } else { format!("{qty} {text}") };
    if text.is_empty() || text.starts_with(char::is_whitespace) || joined != line {
        return (None, line);
    }
    (Some(qty), text)
}

/// The leading letters of `s`.
fn alpha_run(s: &str) -> &str {
    let end = s.find(|c: char| !c.is_alphabetic()).unwrap_or(s.len());
    &s[..end]
}

fn digits(s: &str, i: usize) -> usize {
    s[i..].bytes().take_while(u8::is_ascii_digit).count()
}

/// The end of `\d+/\d+` at `i`.
fn fraction(s: &str, i: usize) -> Option<usize> {
    let a = digits(s, i);
    if a == 0 || !s[i + a..].starts_with('/') {
        return None;
    }
    let b = digits(s, i + a + 1);
    (b > 0).then_some(i + a + 1 + b)
}

/// The end of one `NUM` of `LEAD` at `i`: `1 1/2`, `1/2`, `1.5`, `1,5`, `1½`, `1 ½`, `½`.
fn number(s: &str, i: usize) -> Option<usize> {
    let first = s[i..].chars().next()?;
    if GLYPHS.contains(&first) {
        return Some(i + first.len_utf8());
    }
    let d = digits(s, i);
    if d == 0 {
        return None;
    }
    let mut j = i + d;
    if s[j..].starts_with(' ') {
        if let Some(e) = fraction(s, j + 1) {
            return Some(e);
        }
    }
    if let Some(e) = fraction(s, i) {
        return Some(e);
    }
    if s[j..].starts_with(['.', ',']) && digits(s, j + 1) > 0 {
        j += 1 + digits(s, j + 1);
    }
    let spaces = s[j..].len() - s[j..].trim_start().len();
    if s[j + spaces..].starts_with(GLYPHS) {
        j += spaces + s[j + spaces..].chars().next().map_or(0, char::len_utf8);
    }
    Some(j)
}

/// The end of the amount a line starts with: one number, or a range of two (`1–2`, `1 - 2`).
fn amount_end(s: &str) -> Option<usize> {
    let a = number(s, 0)?;
    let after = s[a..].trim_start();
    if let Some(dash) = after.strip_prefix(['–', '-']) {
        let b = s.len() - dash.trim_start().len();
        if let Some(e) = number(s, b) {
            return Some(e);
        }
    }
    Some(a)
}

#[cfg(test)]
#[path = "recipe_tests.rs"]
mod tests;
