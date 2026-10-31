//! Progress in a bot's browser: what an action changed on the page, and when the bot is going
//! round in circles. Pure, so the rules are tested as tables; `routes::act_sandbox_allowed` takes
//! a [`Snapshot`] before and after each action and keeps one [`Tracker`] per browser.

use super::ClickTarget;

/// What the page shows, for comparing before and after an action. `state` covers what the text
/// misses: scroll position and how much is in each field (lengths only, never values).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub url: String,
    pub title: String,
    pub text: String,
    pub state: String,
    /// The focused element. A move is progress for a key (Tab), not for a click, which always
    /// focuses what it hits.
    pub focus: String,
}

/// Lines shown per side of a change, and the most characters per line.
const SHOWN_LINES: usize = 4;
const LINE_CHARS: usize = 80;

fn lines(text: &str) -> Vec<&str> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect()
}

fn quote(line: &str) -> String {
    let mut short: String = line.chars().take(LINE_CHARS).collect();
    if line.chars().count() > LINE_CHARS {
        short.push('…');
    }
    format!("\"{short}\"")
}

/// Lines of `a` that aren't in `b`, in page order, each once.
fn only_in<'a>(a: &[&'a str], b: &[&str]) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for line in a {
        if !b.contains(line) && !out.contains(line) {
            out.push(line);
        }
    }
    out
}

fn list(label: &str, found: &[&str]) -> Option<String> {
    if found.is_empty() {
        return None;
    }
    let shown: Vec<String> = found.iter().take(SHOWN_LINES).map(|l| quote(l)).collect();
    let more = found.len().saturating_sub(SHOWN_LINES);
    let more = if more > 0 { format!(" (+{more} more)") } else { String::new() };
    Some(format!("{label}: {}{more}.", shown.join(", ")))
}

/// A short "what changed" from `before` to `after`, or `None` when nothing did. `focus`: whether
/// the focus moving counts.
pub fn what_changed(before: &Snapshot, after: &Snapshot, focus: bool) -> Option<String> {
    let same_focus = !focus || before.focus == after.focus;
    if before.url == after.url && before.title == after.title && before.text == after.text && before.state == after.state && same_focus {
        return None;
    }
    if before.url != after.url {
        let title = if after.title.is_empty() { String::new() } else { format!(" ({})", quote(&after.title)) };
        return Some(format!("Went to {}{title}.", after.url));
    }
    let mut parts: Vec<String> = Vec::new();
    if before.title != after.title {
        parts.push(format!("Title is now {}.", quote(&after.title)));
    }
    let (old, new) = (lines(&before.text), lines(&after.text));
    parts.extend(list("New", &only_in(&new, &old)));
    parts.extend(list("Gone", &only_in(&old, &new)));
    if parts.is_empty() {
        if before.text != after.text {
            parts.push("The text moved around; no new lines.".into());
        } else if before.state != after.state {
            parts.push("No new text; the scroll position or a field changed.".into());
        } else {
            parts.push("The focus moved.".into());
        }
    }
    Some(parts.join(" "))
}

/// Which actions count as "the same action" for the loop stop: clicks on the same target and
/// typing the same text into the same place. Scrolls and keys (Tab, arrows) are left out: they
/// repeat on purpose.
pub fn action_key(action: &str, target: Option<&ClickTarget>, text: Option<&str>) -> Option<String> {
    let at = |t: Option<&ClickTarget>| match t {
        Some(ClickTarget::Ref(r)) => format!("ref {r}"),
        Some(ClickTarget::Text(l)) => format!("label {}", l.to_lowercase()),
        None => "focus".into(),
    };
    match action {
        "click" | "click_text" => Some(format!("click {}", at(target))),
        "type" => Some(format!("type {} into {}", text.unwrap_or_default(), at(target))),
        _ => None,
    }
}

/// How many times in a row before a bot is stopped.
pub const STALL_AFTER: u32 = 3;

pub const REPEAT_STOP: &str = "Not done: this is the 3rd time in a row you asked for exactly this action on this page. \
     Don't repeat it; try something else (another control, scroll, a different page), or tell the person what is in the way.";
pub const STALL_STOP: &str = "Stop: your last 3 actions changed nothing on this page. Don't repeat them; try something else \
     (another control, scroll, a different page), or tell the person what is in the way.";

/// One browser's recent actions.
#[derive(Debug, Default)]
pub struct Tracker {
    last: Option<(String, String)>,
    repeats: u32,
    unchanged: u32,
}

impl Tracker {
    /// A new page (the bot opened an address): start over.
    pub fn reset(&mut self) {
        *self = Tracker::default();
    }

    /// Before an action: `Some(REPEAT_STOP)` when it would be the same action a 3rd time in a row
    /// on `url`. It keeps refusing until the bot does something else.
    pub fn before(&mut self, url: &str, key: Option<&str>) -> Option<&'static str> {
        let key = key?;
        let same = self.last.as_ref().is_some_and(|(u, k)| u == url && k == key);
        if same && self.repeats + 1 >= STALL_AFTER {
            return Some(REPEAT_STOP);
        }
        self.repeats = if same { self.repeats + 1 } else { 1 };
        self.last = Some((url.to_string(), key.to_string()));
        None
    }

    /// After an action: `Some(STALL_STOP)` when it is the 3rd in a row that changed nothing.
    /// The count starts over after it is reported.
    pub fn after(&mut self, changed: bool) -> Option<&'static str> {
        if changed {
            self.unchanged = 0;
            return None;
        }
        self.unchanged += 1;
        if self.unchanged >= STALL_AFTER {
            self.unchanged = 0;
            return Some(STALL_STOP);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(url: &str, title: &str, text: &str, state: &str) -> Snapshot {
        Snapshot { url: url.into(), title: title.into(), text: text.into(), state: state.into(), focus: String::new() }
    }

    #[test]
    fn a_change_is_told_in_a_line() {
        let a = snap("https://s.test/", "Shop", "Backpack\nAdd to cart\nBike Light\nAdd to cart", "0");
        assert_eq!(what_changed(&a, &a, true), None);
        let b = snap("https://s.test/", "Shop", "Backpack\nRemove\nBike Light\nAdd to cart\n1", "0");
        assert_eq!(what_changed(&a, &b, true).unwrap(), "New: \"Remove\", \"1\".");
        let c = snap("https://s.test/", "Cart (1)", "Backpack", "0");
        assert_eq!(what_changed(&a, &c, true).unwrap(), "Title is now \"Cart (1)\". Gone: \"Add to cart\", \"Bike Light\".");
        let d = snap("https://s.test/cart", "Your cart", "anything", "0");
        assert_eq!(what_changed(&a, &d, true).unwrap(), "Went to https://s.test/cart (\"Your cart\").");
        let e = snap("https://s.test/", "Shop", "Bike Light\nAdd to cart\nBackpack\nAdd to cart", "0");
        assert_eq!(what_changed(&a, &e, true).unwrap(), "The text moved around; no new lines.");
        let f = snap("https://s.test/", "Shop", a.text.as_str(), "600");
        assert!(what_changed(&a, &f, true).unwrap().starts_with("No new text"));
        let g = Snapshot { focus: "BUTTON#".into(), ..a.clone() };
        assert!(what_changed(&a, &g, true).is_some(), "Tab moved the focus");
        assert_eq!(what_changed(&a, &g, false), None, "a click always focuses what it hits");
    }

    #[test]
    fn long_changes_are_cut_short() {
        let a = snap("u", "t", "", "");
        assert_eq!(what_changed(&a, &Snapshot { focus: "INPUT#q".into(), ..a.clone() }, true).unwrap(), "The focus moved.");
        let many: Vec<String> = (1..=7).map(|i| format!("line {i}")).collect();
        let b = snap("u", "t", &many.join("\n"), "");
        assert_eq!(what_changed(&a, &b, true).unwrap(), "New: \"line 1\", \"line 2\", \"line 3\", \"line 4\" (+3 more).");
        let long = "x".repeat(200);
        let c = snap("u", "t", &long, "");
        assert_eq!(what_changed(&a, &c, true).unwrap(), format!("New: \"{}…\".", "x".repeat(80)));
    }

    #[test]
    fn the_same_action_a_third_time_is_refused_until_the_bot_does_something_else() {
        let mut t = Tracker::default();
        let add = action_key("click", Some(&ClickTarget::Ref(9)), None);
        let add = add.as_deref();
        assert_eq!(t.before("u", add), None);
        assert_eq!(t.before("u", add), None);
        assert_eq!(t.before("u", add), Some(REPEAT_STOP));
        assert_eq!(t.before("u", add), Some(REPEAT_STOP), "still refused");
        assert_eq!(t.before("u", Some("click ref 3")), None);
        assert_eq!(t.before("u", add), None, "after something else it may again");
        // Scrolls and keys don't count and don't break a run
        assert_eq!(action_key("scroll", None, None), None);
        assert_eq!(action_key("key", None, None), None);
        assert_eq!(t.before("u", None), None);
        assert_eq!(t.before("u", add), None);
        assert_eq!(t.before("u", add), Some(REPEAT_STOP));
        // The same click on another page is another action
        assert_eq!(t.before("u2", add), None);
        // Typing different text is a different action; a label is matched without case
        assert_ne!(action_key("type", None, Some("a")), action_key("type", None, Some("b")));
        assert_eq!(
            action_key("click_text", Some(&ClickTarget::Text("Next".into())), None),
            action_key("click", Some(&ClickTarget::Text("next".into())), None)
        );
        t.reset();
        assert_eq!(t.before("u2", add), None);
    }

    #[test]
    fn three_actions_that_change_nothing_stop_the_bot() {
        let mut t = Tracker::default();
        assert_eq!(t.after(false), None);
        assert_eq!(t.after(false), None);
        assert_eq!(t.after(true), None, "a change starts over");
        assert_eq!(t.after(false), None);
        assert_eq!(t.after(false), None);
        assert_eq!(t.after(false), Some(STALL_STOP));
        assert_eq!(t.after(false), None, "counted afresh after it is told");
    }
}
