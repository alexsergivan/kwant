//! 27e: a reply that says a device in the home changed. On kwant 0.9.21 with a 7B model, House
//! replied "Unlocking the front door now." after a turn that ran no tool at all: no lock card, and
//! the door stayed locked while the person could believe it was open. The delivery check
//! ([`super::delivery_check`]) asks this matcher, and only for a turn that had Home Assistant's
//! tools, whether the reply claims a change that no Home Assistant step made.
//!
//! The match is narrow on purpose, like [`super::picture_claims`]: a sentence must say, as done
//! or as being done now, that the bot changed a device: "Unlocking the front door now.", "I've
//! unlocked the front door.", "Turned off Living room lamp.", "the porch light is now off". A
//! question, a negation or inability ("I can't unlock…"), a promise or a condition ("I'll unlock
//! it once you approve the card"), a wait ("Waiting for your OK…"), a plain state report ("The
//! front door is locked.") and code are never claims.

use super::picture_claims::{sentences, words, Word};

/// A change said as being done now: "Unlocking the front door now.", "I'm turning off…".
const DOING: &[&str] = &[
    "unlocking", "locking", "turning", "switching", "opening", "closing", "arming", "disarming", "dimming", "brightening",
    "setting", "starting", "stopping", "pausing", "resuming",
];

/// A change said as done: "I've unlocked…", "Turned off…", "has been unlocked".
const DONE: &[&str] = &[
    "unlocked", "locked", "turned", "switched", "opened", "closed", "armed", "disarmed", "dimmed", "brightened", "set",
    "started", "stopped", "paused", "resumed",
];

/// Verbs that change something only with a particle after them: "turned off", "switch on".
const NEEDS_ON_OFF: &[&str] = &["turning", "turned", "switching", "switched"];

/// "Setting the thermostat to 21", "set the lamp to 40%": `set` needs a `to`.
const NEEDS_TO: &[&str] = &["setting", "set"];

/// Start, stop, pause and resume count only for what Home Assistant plays or runs: "Stopped the
/// vacuum", never "I started looking".
const NEEDS_PLAYER: &[&str] = &["starting", "stopping", "pausing", "resuming", "started", "stopped", "paused", "resumed"];
const PLAYERS: &[&str] = &["vacuum", "music", "playback", "media", "player", "tv", "speaker", "speakers", "radio", "robot", "cleaning", "song", "playlist"];

/// How a done change may be said by the bot: right before the verb ("I've unlocked", "just
/// turned", "Done: turned off").
const SUBJECTS: &[&str] = &["i", "i've", "we", "we've", "have", "just", "already", "also", "done", "now", "been"];

/// What may open the sentence before the verb: "Done — turned off…", "OK, unlocking…".
const LEADS: &[&str] = &["done", "ok", "okay", "sure", "alright", "all", "right", "now", "and", "then", "also", "yes", "great"];

/// "I'm unlocking", "now opening": a doing form after one of these is the bot's.
const DOING_AFTER: &[&str] = &["i'm", "im", "am", "we're", "now", "currently", "also", "and"];

/// What a device is after a change: "the porch light is now off".
const STATES: &[&str] = &["on", "off", "locked", "unlocked", "open", "opened", "closed", "armed", "disarmed", "dimmed", "paused", "stopped"];

/// Words that turn a sentence into a negation, an inability, a promise, a condition, a wait or
/// an offer. One anywhere in the sentence and it is not a claim.
const GUARDS: &[&str] = &[
    "not", "no", "never", "cannot", "unable", "without", "failed", "will", "would", "can", "could", "should", "may", "might",
    "shall", "want", "wants", "if", "once", "when", "after", "until", "before", "unless", "let", "able", "try", "trying",
    "going", "plan", "planning", "waiting", "wait", "approve", "approval", "confirm", "please", "need", "needs", "ready",
    "nothing", "tried",
];

fn is_guard(w: &str) -> bool {
    GUARDS.contains(&w) || w.ends_with("n't") || w.ends_with("'ll") || w.ends_with("'d")
}

fn is(w: &Word, set: &[&str]) -> bool {
    set.contains(&w.text.as_str())
}

/// The verb at `i` names a change with what has to follow it: on/off, a `to`, or a player.
fn completes(w: &[Word], i: usize) -> bool {
    let after = &w[i + 1..];
    let near = |n: usize, set: &[&str]| after.iter().take(n).any(|x| is(x, set));
    if is(&w[i], NEEDS_ON_OFF) {
        return near(6, &["on", "off"]);
    }
    if is(&w[i], NEEDS_TO) {
        return near(8, &["to"]);
    }
    if is(&w[i], NEEDS_PLAYER) {
        return near(5, PLAYERS);
    }
    true
}

/// Whether one sentence claims the bot changed a device.
fn sentence_claims(sentence: &str) -> bool {
    if sentence.trim_end().ends_with('?') {
        return false;
    }
    let w = words(sentence);
    if w.iter().any(|x| is_guard(&x.text)) {
        return false;
    }
    // Where the sentence starts, past "Done —", "OK,", "Now"
    let start = w.iter().take_while(|x| is(x, LEADS)).count();
    for (i, word) in w.iter().enumerate() {
        let before = &w[..i];
        let prev = before.last();
        if is(word, DOING) && completes(&w, i) && (i == start || prev.is_some_and(|p| is(p, DOING_AFTER))) {
            return true;
        }
        if is(word, DONE) && completes(&w, i) && (i == start || before.iter().rev().take(2).any(|p| is(p, SUBJECTS))) {
            return true;
        }
        // "the porch light is now off"
        if (word.text == "is" || word.text == "are")
            && w.get(i + 1).is_some_and(|n| n.text == "now")
            && w.get(i + 2).is_some_and(|s| is(s, STATES))
        {
            return true;
        }
    }
    false
}

/// The first sentence of `reply` that says, as done or as being done now, that the bot changed
/// a device in the home; `None` when there is none. Code never counts.
pub fn claimed_change(reply: &str) -> Option<String> {
    if crate::NOFIX_27E {
        return None;
    }
    sentences(reply).into_iter().find(|s| sentence_claims(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// From the tester's report (issue.md, Nov 17), and the forms the brief lists.
    #[test]
    fn claims_of_a_change() {
        for (reply, first) in [
            ("Unlocking the front door now.", "Unlocking the front door now."),
            ("Unlocking the front door now.\n\nUnlocking the front door.", "Unlocking the front door now."),
            ("I've unlocked the front door.", "I've unlocked the front door."),
            ("I’ve unlocked the front door.", "I’ve unlocked the front door."),
            ("Turned off Living room lamp.", "Turned off Living room lamp."),
            ("I turned on the kitchen light and the porch light.", "I turned on the kitchen light and the porch light."),
            ("Done — the porch light is now off.", "Done — the porch light is now off."),
            ("Done: turned off the porch light.", "Done: turned off the porch light."),
            ("I'm unlocking the front door.", "I'm unlocking the front door."),
            ("I unlocked the front door for you.", "I unlocked the front door for you."),
            ("OK, switching the kitchen light off.", "OK, switching the kitchen light off."),
            ("I have locked the back door.", "I have locked the back door."),
            ("The front door has been unlocked.", "The front door has been unlocked."),
            ("Setting the living room lamp to 40%.", "Setting the living room lamp to 40%."),
            ("I set the thermostat to 21 °C.", "I set the thermostat to 21 °C."),
            ("I stopped the vacuum.", "I stopped the vacuum."),
            ("Disarming the alarm now.", "Disarming the alarm now."),
            ("Sure. Opening the garage door.", "Opening the garage door."),
        ] {
            assert_eq!(claimed_change(reply).as_deref(), Some(first), "{reply}");
        }
    }

    #[test]
    fn not_claims() {
        for reply in [
            "I can't unlock the front door. Please do it yourself or ask a trusted person.",
            "I can't unlock the front door on my own. Please confirm the request to unlock it.",
            "Shall I unlock the front door?",
            "I'll unlock it once you approve the card.",
            "The front door is locked.",
            "The kitchen light is on.",
            "Waiting for your OK to unlock Front door.",
            "I couldn't find a sensor for the temperature inside.",
            "There are no switches left on in the home.",
            "The living room light is off.",
            "Kitchen light: on. Porch light: on. Living room lamp: off.",
            "Unlock the front door.",
            "Do you want me to turn off the porch light?",
            "I didn't turn anything off.",
            "If you want, I can turn off the porch light.",
            "Approve the card to unlock Front door.",
            "You turned off the lamp at 9.",
            "Someone locked the front door at 21:40.",
            "I started looking at the sensors.",
            "I set up the list for you.",
            "The thermostat is set to 21 °C.",
            "Turning to the temperature: it's 21.4 °C in the living room.",
            "`turned off` is what the log says.",
            "```\nUnlocking the front door now.\n```",
            "The light turned off by itself at 10.",
            "Nothing in your home changed. No Home Assistant step ran.",
        ] {
            assert_eq!(claimed_change(reply), None, "{reply}");
        }
    }
}
