//! Checks a Quiz pocket.json and lists every problem in plain, numbered words a bot can fix
//! ("3. round 2, question 1: a picture needs a licence and a credit").

use super::matching::normalise;
use super::spec::{Kind, Picture, QuizSpec, TeamMode, LICENCES, VERSION};
use crate::kits::Problem;

/// Limits (also in schema.md).
pub const MAX_SPEC_BYTES: usize = 256 * 1024;
pub const MAX_QUESTIONS: usize = 100;
pub const MAX_ROUNDS: usize = 20;
pub const MAX_TITLE: usize = 80;
pub const MAX_ROUND_TITLE: usize = 60;
pub const MAX_QUESTION_TEXT: usize = 300;
pub const MAX_OPTION: usize = 80;
pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 6;
pub const MAX_ANSWER: usize = 80;
pub const MAX_ACCEPT: usize = 10;
pub const MAX_QUOTE: usize = 300;
pub const MAX_URL: usize = 500;
pub const MAX_LINE: usize = 280;
pub const MAX_REVEAL_LINES: usize = 20;
pub const MAX_CREDIT: usize = 200;
pub const MAX_ALT: usize = 200;
pub const MAX_TEAM_NAME: usize = 20;
pub const MAX_TEAM_NAMES: usize = 20;
pub const MIN_POINTS: u32 = 1;
pub const MAX_POINTS: u32 = 100;
pub const MIN_TIME_LIMIT_S: u32 = 5;
pub const MAX_TIME_LIMIT_S: u32 = 300;
pub const MIN_TEAM_SIZE: u32 = 2;
pub const MAX_TEAM_SIZE: u32 = 10;
pub const MIN_AUTO_ADVANCE_S: u32 = 3;
pub const MAX_AUTO_ADVANCE_S: u32 = 120;
/// The kit's player cap (pa1's default).
pub const MAX_PLAYERS: u32 = 60;

/// Field names that mean sound; there are none in pocket.json (plan section 8 rule 8).
const AUDIO_WORDS: &[&str] = &["audio", "sound", "music", "clip", "song", "mp3", "video"];

struct Found(Vec<(Option<String>, String)>);

impl Found {
    fn add(&mut self, at: Option<&str>, msg: impl Into<String>) {
        self.0.push((at.map(str::to_string), msg.into()));
    }
    fn into_problems(self) -> Vec<Problem> {
        self.0.into_iter().enumerate().map(|(i, (at, message))| Problem { n: i + 1, at, message }).collect()
    }
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

/// Parses and validates; the spec is returned only when it has no problems.
pub fn parse(spec_json: &str) -> Result<QuizSpec, Vec<Problem>> {
    let mut f = Found(Vec::new());
    if spec_json.len() > MAX_SPEC_BYTES {
        f.add(None, format!("the file is {} KB; the most a quiz can be is {} KB", spec_json.len() / 1024, MAX_SPEC_BYTES / 1024));
        return Err(f.into_problems());
    }
    let spec: QuizSpec = match serde_json::from_str(spec_json) {
        Ok(s) => s,
        Err(e) => {
            let text = e.to_string();
            let audio = text.starts_with("unknown field `")
                && AUDIO_WORDS.iter().any(|w| text.split('`').nth(1).is_some_and(|field| field.to_lowercase().contains(w)));
            if audio {
                f.add(None, format!("there are no audio or video fields in a quiz: music clips aren't allowed ({text})"));
            } else {
                f.add(None, format!("this isn't valid pocket.json v1: {text}"));
            }
            return Err(f.into_problems());
        }
    };
    check(&spec, &mut f);
    if f.0.is_empty() {
        Ok(spec)
    } else {
        Err(f.into_problems())
    }
}

fn check(spec: &QuizSpec, f: &mut Found) {
    match spec.version {
        Some(VERSION) => {}
        Some(v) => f.add(None, format!("`version` is {v}; this kit reads version {VERSION}")),
        None => f.add(None, format!("`version` is missing; write \"version\": {VERSION}")),
    }
    if let Some(kit) = &spec.kit {
        if kit != "quiz" {
            f.add(None, format!("`kit` is \"{kit}\"; this file is for the \"quiz\" kit"));
        }
    }
    let title = spec.title.trim();
    if title.is_empty() {
        f.add(None, "the title is empty");
    } else if chars(title) > MAX_TITLE {
        f.add(None, format!("the title is {} characters; the most is {MAX_TITLE}", chars(title)));
    }
    if let Some(when) = &spec.when {
        if chrono::DateTime::parse_from_rfc3339(when).is_err() {
            f.add(None, format!("`when` is \"{when}\"; write a date and time with its offset, like \"2026-11-27T20:00:00+00:00\""));
        }
    }
    if let Some(theme) = &spec.theme {
        let ok = theme.len() == 7 && theme.starts_with('#') && theme[1..].chars().all(|c| c.is_ascii_hexdigit());
        if !ok {
            f.add(None, format!("the theme colour \"{theme}\" isn't a colour like \"#7c5cff\""));
        }
    }
    check_teams(spec, f);
    check_settings(spec, f);
    check_lines(spec, f);

    if spec.rounds.is_empty() {
        f.add(None, "there are no rounds; a quiz needs at least one round with a question");
    }
    if spec.rounds.len() > MAX_ROUNDS {
        f.add(None, format!("there are {} rounds; the most is {MAX_ROUNDS}", spec.rounds.len()));
    }
    let total: usize = spec.rounds.iter().map(|r| r.questions.len()).sum();
    if total > MAX_QUESTIONS {
        f.add(None, format!("there are {total} questions; the most is {MAX_QUESTIONS}"));
    }
    for (ri, round) in spec.rounds.iter().enumerate() {
        let rat = format!("round {}", ri + 1);
        let rt = round.title.trim();
        if rt.is_empty() {
            f.add(Some(&rat), "the round has no title");
        } else if chars(rt) > MAX_ROUND_TITLE {
            f.add(Some(&rat), format!("the title is {} characters; the most is {MAX_ROUND_TITLE}", chars(rt)));
        }
        if let Some(intro) = &round.intro {
            line(f, &rat, "the round intro", intro);
        }
        if round.questions.is_empty() {
            f.add(Some(&rat), "the round has no questions");
        }
        for (qi, q) in round.questions.iter().enumerate() {
            let at = format!("round {}, question {}", ri + 1, qi + 1);
            check_question(q, &at, f);
        }
    }
}

fn line(f: &mut Found, at: &str, what: &str, text: &str) {
    if text.trim().is_empty() {
        f.add(Some(at), format!("{what} is empty; leave it out instead"));
    } else if chars(text) > MAX_LINE {
        f.add(Some(at), format!("{what} is {} characters; the most is {MAX_LINE}", chars(text)));
    }
}

fn check_teams(spec: &QuizSpec, f: &mut Found) {
    let t = &spec.teams;
    let at = Some("teams");
    match t.mode {
        TeamMode::Solo => {
            if t.size.is_some() {
                f.add(at, "`size` only applies when `mode` is \"teams\"");
            }
            if t.names.is_some() {
                f.add(at, "`names` only applies when `mode` is \"teams\"");
            }
        }
        TeamMode::Teams => {
            match t.size {
                None => f.add(at, format!("teams need a `size` ({MIN_TEAM_SIZE} to {MAX_TEAM_SIZE} players a team)")),
                Some(s) if !(MIN_TEAM_SIZE..=MAX_TEAM_SIZE).contains(&s) => {
                    f.add(at, format!("a team size of {s} won't work; use {MIN_TEAM_SIZE} to {MAX_TEAM_SIZE}"))
                }
                Some(_) => {}
            }
            if let Some(names) = &t.names {
                if names.is_empty() {
                    f.add(at, "`names` is empty; leave it out to let players name their teams");
                }
                if names.len() > MAX_TEAM_NAMES {
                    f.add(at, format!("there are {} team names; the most is {MAX_TEAM_NAMES}", names.len()));
                }
                let mut seen: Vec<String> = Vec::new();
                for (i, n) in names.iter().enumerate() {
                    let norm = normalise(n);
                    if norm.is_empty() {
                        f.add(at, format!("team name {} is empty", i + 1));
                    } else if chars(n.trim()) > MAX_TEAM_NAME {
                        f.add(at, format!("team name {} is {} characters; the most is {MAX_TEAM_NAME}", i + 1, chars(n.trim())));
                    } else if seen.contains(&norm) {
                        f.add(at, format!("team name {} (\"{}\") is there twice", i + 1, n.trim()));
                    }
                    seen.push(norm);
                }
            }
        }
    }
}

fn check_settings(spec: &QuizSpec, f: &mut Found) {
    let s = &spec.settings;
    let at = Some("settings");
    if let Some(a) = s.auto_advance_s {
        if !(MIN_AUTO_ADVANCE_S..=MAX_AUTO_ADVANCE_S).contains(&a) {
            f.add(at, format!("`auto_advance_s` is {a}; use {MIN_AUTO_ADVANCE_S} to {MAX_AUTO_ADVANCE_S} seconds, or leave it out"));
        }
    }
    if let Some(m) = s.max_players {
        if !(1..=MAX_PLAYERS).contains(&m) {
            f.add(at, format!("`max_players` is {m}; use 1 to {MAX_PLAYERS}"));
        }
    }
}

fn check_lines(spec: &QuizSpec, f: &mut Found) {
    let l = &spec.lines;
    if let Some(intro) = &l.intro {
        line(f, "lines", "the intro", intro);
    }
    if let Some(fin) = &l.finale {
        line(f, "lines", "the final line", fin);
    }
    if l.reveal.len() > MAX_REVEAL_LINES {
        f.add(Some("lines"), format!("there are {} reveal lines; the most is {MAX_REVEAL_LINES}", l.reveal.len()));
    }
    for (i, r) in l.reveal.iter().enumerate() {
        line(f, "lines", &format!("reveal line {}", i + 1), r);
    }
}

fn check_question(q: &super::spec::Question, path: &str, f: &mut Found) {
    let at = Some(path);
    let text = q.text.trim();
    if text.is_empty() {
        f.add(at, "the question is empty");
    } else if chars(text) > MAX_QUESTION_TEXT {
        f.add(at, format!("the question is {} characters; the most is {MAX_QUESTION_TEXT}", chars(text)));
    }
    let Some(kind) = q.kind else {
        f.add(at, "`kind` is missing; use \"choice\", \"text\" or \"number\"");
        return;
    };
    if kind != Kind::Choice && q.options.is_some() {
        f.add(at, "only choice questions have `options`");
    }
    if kind != Kind::Text && q.accept.is_some() {
        f.add(at, "only text questions have `accept`");
    }
    if kind != Kind::Number && q.tolerance.is_some() {
        f.add(at, "only number questions have `tolerance`");
    }
    match kind {
        Kind::Choice => check_choice(q, at, f),
        Kind::Text => check_text(q, at, f),
        Kind::Number => match &q.answer {
            None => f.add(at, "the answer is empty"),
            Some(a) => match a.as_f64() {
                Some(n) if n.is_finite() => {}
                _ => f.add(at, "a number question's answer must be a number, like 1066 (not in quotes)"),
            },
        },
    }
    if let Some(t) = q.tolerance {
        if !t.is_finite() || t < 0.0 {
            f.add(at, "`tolerance` must be 0 or more");
        }
    }
    if let Some(p) = q.points {
        if !(MIN_POINTS..=MAX_POINTS).contains(&p) {
            f.add(at, format!("{p} points won't work; use {MIN_POINTS} to {MAX_POINTS}"));
        }
    }
    if let Some(s) = q.time_limit_s {
        if !(MIN_TIME_LIMIT_S..=MAX_TIME_LIMIT_S).contains(&s) {
            f.add(at, format!("a time limit of {s} seconds won't work; use {MIN_TIME_LIMIT_S} to {MAX_TIME_LIMIT_S}"));
        }
    }
    match &q.source {
        None => f.add(at, "the answer has no source; add `source` with the page's `url` and the `quote` that holds the answer"),
        // pa4c: the person's own question has no page to quote
        Some(src) if src.own => {
            if !src.url.is_empty() || !src.quote.is_empty() {
                f.add(at, "a source is either the person's own (`own`) or a page with `url` and `quote`, not both");
            }
        }
        Some(src) => {
            if !web_url(&src.url) {
                f.add(at, "the source needs a web address (http or https) in `url`");
            }
            let quote = src.quote.trim();
            if quote.is_empty() {
                f.add(at, "the source needs a `quote`: the line on that page that holds the answer");
            } else if chars(quote) > MAX_QUOTE {
                f.add(at, format!("the source quote is {} characters; the most is {MAX_QUOTE}", chars(quote)));
            }
        }
    }
    if let Some(p) = &q.picture {
        check_picture(p, at, f);
    }
    if let Some(r) = &q.reveal_line {
        line(f, path, "the reveal line", r);
    }
}

fn check_choice(q: &super::spec::Question, at: Option<&str>, f: &mut Found) {
    let options = q.options();
    if q.options.is_none() || !(MIN_OPTIONS..=MAX_OPTIONS).contains(&options.len()) {
        f.add(at, format!("a choice question needs {MIN_OPTIONS} to {MAX_OPTIONS} options; it has {}", options.len()));
    }
    let mut seen: Vec<String> = Vec::new();
    for (i, o) in options.iter().enumerate() {
        let norm = normalise(o);
        if norm.is_empty() {
            f.add(at, format!("option {} is empty", i + 1));
        } else if chars(o.trim()) > MAX_OPTION {
            f.add(at, format!("option {} is {} characters; the most is {MAX_OPTION}", i + 1, chars(o.trim())));
        } else if seen.contains(&norm) {
            f.add(at, format!("option {} (\"{}\") is there twice", i + 1, o.trim()));
        }
        seen.push(norm);
    }
    match &q.answer {
        None => f.add(at, "the answer is empty"),
        Some(a) => match a.as_str() {
            None => f.add(at, "a choice question's answer must be the right option's text, in quotes"),
            Some(s) if normalise(s).is_empty() => f.add(at, "the answer is empty"),
            Some(s) => {
                let n = normalise(s);
                if !options.is_empty() && !seen.contains(&n) {
                    f.add(at, format!("the answer \"{}\" isn't one of the options", s.trim()));
                }
            }
        },
    }
}

fn check_text(q: &super::spec::Question, at: Option<&str>, f: &mut Found) {
    match &q.answer {
        None => f.add(at, "the answer is empty"),
        Some(a) => match a.as_str() {
            None => f.add(at, "a text question's answer must be text, in quotes"),
            Some(s) if normalise(s).is_empty() => f.add(at, "the answer is empty"),
            Some(s) if chars(s.trim()) > MAX_ANSWER => {
                f.add(at, format!("the answer is {} characters; the most is {MAX_ANSWER}", chars(s.trim())))
            }
            Some(_) => {}
        },
    }
    let accept = q.accepted();
    if accept.len() > MAX_ACCEPT {
        f.add(at, format!("there are {} accepted spellings; the most is {MAX_ACCEPT}", accept.len()));
    }
    for (i, a) in accept.iter().enumerate() {
        if normalise(a).is_empty() {
            f.add(at, format!("accepted spelling {} is empty", i + 1));
        } else if chars(a.trim()) > MAX_ANSWER {
            f.add(at, format!("accepted spelling {} is {} characters; the most is {MAX_ANSWER}", i + 1, chars(a.trim())));
        }
    }
}

fn check_picture(p: &Picture, at: Option<&str>, f: &mut Found) {
    let licence = p.licence.as_deref().map(str::trim).filter(|l| !l.is_empty());
    let credit = p.credit.as_deref().map(str::trim).filter(|c| !c.is_empty());
    match (licence, credit) {
        (None, None) => f.add(at, "a picture needs a licence and a credit"),
        (None, Some(_)) => f.add(at, format!("a picture needs a licence: one of {}", LICENCES.join(", "))),
        (Some(l), c) => {
            if !LICENCES.contains(&l) {
                f.add(at, format!("the picture's licence \"{l}\" isn't allowed; use one of {} (CC BY with its credit)", LICENCES.join(", ")));
            } else if l != "own" && c.is_none() {
                f.add(at, format!("a {l} picture needs a credit: who made it"));
            }
        }
    }
    if let Some(c) = credit {
        if chars(c) > MAX_CREDIT {
            f.add(at, format!("the picture credit is {} characters; the most is {MAX_CREDIT}", chars(c)));
        }
    }
    if let Some(a) = &p.alt {
        if chars(a) > MAX_ALT {
            f.add(at, format!("the picture's alt text is {} characters; the most is {MAX_ALT}", chars(a)));
        }
    }
    let url = p.url.trim();
    let own_path = licence == Some("own") && own_file_path(url);
    if url.is_empty() {
        f.add(at, "the picture has no `url`");
    } else if url.len() > MAX_URL {
        f.add(at, format!("the picture's url is longer than {MAX_URL} characters"));
    } else if !own_path && (!url.starts_with("https://") || url::Url::parse(url).is_err()) {
        f.add(at, "the picture's `url` must be an https address (or, for the host's own photo, a path in the bot's files)");
    }
}

/// A relative path with no `..`, no scheme and no leading slash.
fn own_file_path(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains(':')
        && !p.contains('\\')
        && p.split('/').all(|seg| !seg.is_empty() && seg != "..")
}

fn web_url(u: &str) -> bool {
    let u = u.trim();
    u.len() <= MAX_URL
        && (u.starts_with("https://") || u.starts_with("http://"))
        && url::Url::parse(u).is_ok_and(|p| p.host_str().is_some_and(|h| !h.is_empty()))
}
