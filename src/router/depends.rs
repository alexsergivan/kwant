//! Which @mentions of one reply wait for another: "@Scriptwriter, once @Analyst has the numbers,
//! write the script" hands work to both, but the Scriptwriter only starts with the Analyst's result.

/// Words that open a dependency on the teammate named right after them.
const TRIGGERS: &[&str] = &["as soon as", "once", "after", "when", "waiting for", "waits for", "wait for", "until", "based on"];
/// Triggers that need a "done" verb (or a possessive) after the name: "when @Analyst is here"
/// isn't a dependency, "when @Analyst has the numbers" is.
const NEEDS_DONE_VERB: &[&str] = &["as soon as", "once", "when", "until"];
const DONE_VERBS: &[&str] = &[
    "has", "have", "had", "finishes", "finished", "is done", "is finished", "sends", "sent", "delivers", "delivered",
    "reports", "reported", "shares", "shared", "returns", "returned", "posts", "posted", "completes", "completed",
    "pulls", "pulled", "provides", "provided", "gets back", "got", "comes back", "hands", "wraps up", "gives",
];
/// How far after the trigger the teammate's name may start, in words.
const MAX_WORDS_TO_NAME: usize = 3;
/// How far after the name the done verb may start, in characters.
const DONE_VERB_WINDOW: usize = 30;

/// The dependencies in `text` among `names` (teammate names, matched case-insensitively, with or
/// without @): `(dependent, prerequisite)` index pairs into `names`. Each dependent waits for at
/// most one teammate; self-dependencies and cycles are dropped, so every pair can be run in order.
pub fn handoff_dependencies(text: &str, names: &[&str]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let lower = text.to_lowercase();
    for sentence in lower.split(['\n', '!', '?', ';']).flat_map(|s| s.split(". ")) {
        for (trigger_at, trigger) in triggers(sentence) {
            let after_trigger = trigger_at + trigger.len();
            let Some((prereq, name_end)) = name_after(sentence, after_trigger, names) else { continue };
            if NEEDS_DONE_VERB.contains(&trigger) && !done_after(sentence, name_end) {
                continue;
            }
            let Some(dependent) = dependent_of(sentence, trigger_at, prereq, names) else { continue };
            if out.iter().any(|&(d, _)| d == dependent) || reaches(&out, prereq, dependent) {
                continue;
            }
            out.push((dependent, prereq));
        }
    }
    out
}

/// Trigger words in `sentence`, at word boundaries, in order.
fn triggers(sentence: &str) -> Vec<(usize, &'static str)> {
    let mut found: Vec<(usize, &'static str)> = Vec::new();
    for &t in TRIGGERS {
        for (at, _) in sentence.match_indices(t) {
            if bounded(sentence, at, at + t.len()) && !found.iter().any(|&(a, f)| at >= a && at < a + f.len()) {
                found.push((at, t));
            }
        }
    }
    found.sort();
    found
}

fn bounded(s: &str, start: usize, end: usize) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    !s[..start].chars().next_back().is_some_and(word) && !s[end..].chars().next().is_some_and(word)
}

/// The teammate named within a few words after `from` (no comma or colon in between), and where
/// the name ends (after a possessive "'s", which counts as done).
fn name_after(sentence: &str, from: usize, names: &[&str]) -> Option<(usize, usize)> {
    let rest = &sentence[from..];
    let mut best: Option<(usize, usize, usize)> = None; // (start in rest, index, end in sentence)
    for (i, n) in names.iter().enumerate() {
        let n = n.to_lowercase();
        for (at, _) in rest.match_indices(&n) {
            let end = at + n.len();
            if !bounded(rest, at, end) || best.is_some_and(|(b, _, _)| b <= at) {
                continue;
            }
            best = Some((at, i, from + end));
            break;
        }
    }
    let (at, i, end) = best?;
    let between = rest[..at].trim_end_matches('@');
    if between.contains([',', ':', '(', ')']) || between.split_whitespace().count() > MAX_WORDS_TO_NAME {
        return None;
    }
    Some((i, end))
}

/// Whether a possessive or a "done" verb follows the name ending at `end`.
fn done_after(sentence: &str, end: usize) -> bool {
    let rest = &sentence[end..];
    if rest.starts_with("'s") || rest.starts_with("’s") {
        return true;
    }
    let window: String = rest.chars().take(DONE_VERB_WINDOW).collect();
    DONE_VERBS.iter().any(|v| window.match_indices(v).any(|(at, _)| bounded(&window, at, at + v.len())))
}

/// The @mentioned teammate who waits: the last one before the trigger, else the first after it,
/// never the prerequisite itself.
fn dependent_of(sentence: &str, trigger_at: usize, prereq: usize, names: &[&str]) -> Option<usize> {
    let mut mentions: Vec<(usize, usize)> = Vec::new();
    for (i, n) in names.iter().enumerate() {
        let tag = format!("@{}", n.to_lowercase());
        for (at, _) in sentence.match_indices(&tag) {
            if i != prereq && bounded(sentence, at + 1, at + tag.len()) {
                mentions.push((at, i));
            }
        }
    }
    mentions.sort();
    mentions.iter().rev().find(|&&(at, _)| at < trigger_at).or_else(|| mentions.iter().find(|&&(at, _)| at > trigger_at)).map(|&(_, i)| i)
}

/// Whether `from` already (transitively) waits for `to` in `edges`.
fn reaches(edges: &[(usize, usize)], from: usize, to: usize) -> bool {
    let mut at = from;
    for _ in 0..=edges.len() {
        if at == to {
            return true;
        }
        match edges.iter().find(|&&(d, _)| d == at) {
            Some(&(_, p)) => at = p,
            None => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: &[&str] = &["Analyst", "Scriptwriter", "Editor", "Designer"];

    #[test]
    fn once_has_makes_the_mention_wait() {
        let deps = handoff_dependencies("@Scriptwriter, once @Analyst has the numbers, write the script.", NAMES);
        assert_eq!(deps, vec![(1, 0)]);
    }

    #[test]
    fn after_and_possessive_count() {
        assert_eq!(handoff_dependencies("@Editor please polish it after @Scriptwriter finishes.", NAMES), vec![(2, 1)]);
        assert_eq!(handoff_dependencies("@Designer: when @Analyst's report is in, make the chart", NAMES), vec![(3, 0)]);
        assert_eq!(handoff_dependencies("@Designer, wait for @Analyst and then draw it", NAMES), vec![(3, 0)]);
    }

    #[test]
    fn independent_mentions_have_no_dependencies() {
        assert!(handoff_dependencies("@Analyst pull the numbers. @Scriptwriter draft the hook.", NAMES).is_empty());
        assert!(handoff_dependencies("@Analyst and @Designer, go ahead.", NAMES).is_empty());
    }

    #[test]
    fn when_without_a_done_verb_is_not_a_dependency() {
        assert!(handoff_dependencies("@Designer, when @Analyst is around, say hi.", NAMES).is_empty());
    }

    #[test]
    fn a_comma_between_trigger_and_name_is_not_a_dependency() {
        // "after you're done, ping @Editor" is about the Analyst's own order, not a wait on the Editor
        assert!(handoff_dependencies("@Analyst, after you're done, ping @Editor.", NAMES).is_empty());
    }

    #[test]
    fn dependent_may_come_after_the_trigger() {
        assert_eq!(handoff_dependencies("Once @Analyst has the data, @Designer makes the chart.", NAMES), vec![(3, 0)]);
    }

    #[test]
    fn cycles_and_self_waits_are_dropped() {
        let deps = handoff_dependencies(
            "@Analyst, once @Editor has the draft, check it. @Editor, once @Analyst has checked, edit it.",
            NAMES,
        );
        assert_eq!(deps, vec![(0, 2)]);
        assert!(handoff_dependencies("@Analyst, once @Analyst has it, go.", NAMES).is_empty());
    }

    #[test]
    fn chains_keep_one_prerequisite_each() {
        let deps = handoff_dependencies(
            "@Scriptwriter, after @Analyst sends the numbers write it. @Editor, once @Scriptwriter has the draft, cut it.",
            NAMES,
        );
        assert_eq!(deps, vec![(1, 0), (2, 1)]);
    }
}
