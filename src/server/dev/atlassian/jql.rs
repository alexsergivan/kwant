//! The pickup JQL (J4) and the Confluence CQL, built here and only here, with every value quoted.
//! A bot never writes a query: `search_pages` takes words, and the person's Advanced JQL is always
//! wrapped in the site's project scope (`project in (…) AND (<theirs>)`), so neither can reach a
//! project or space nobody chose (AND binds tighter than OR, so pasting without parentheses would).

use crate::server::dev::tickets::source::valid_project;

/// How a ticket is handed to the team: one rule, picked in the sheet (J4 as amended). "Assignee
/// OR label" is only possible as Advanced JQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Handover {
    /// Assigned to one Jira user.
    Assignee,
    /// Carries a label (`kwant` by default; the human assignee stays).
    Label,
}

impl Handover {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "assignee" => Some(Self::Assignee),
            "label" => Some(Self::Label),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Assignee => "assignee",
            Self::Label => "label",
        }
    }
}

/// The rule a built JQL says, kept so the poller can re-check one ticket locally (search results
/// lag edits on Cloud, so "missing from the search" alone never pauses a ticket).
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub projects: Vec<String>,
    pub handover: Handover,
    pub assignee: Option<String>,
    pub label: Option<String>,
}

/// The longest Advanced JQL kwant accepts.
pub const MAX_JQL: usize = 2_000;
/// Most projects one site picks up from.
pub const MAX_PROJECTS: usize = 20;
/// Most Confluence spaces one site searches.
pub const MAX_SPACES: usize = 20;

/// A JQL or CQL string literal: `"…"` with `\` and `"` escaped; control characters become spaces.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A Jira user reference: Cloud accountId or DC username/key. 1–128 printable chars, no quotes.
pub fn valid_user(s: &str) -> bool {
    !s.is_empty() && s.chars().count() <= 128 && s.chars().all(|c| !c.is_control() && c != '"' && c != '\\')
}

/// A Jira label: 1–255 chars, no whitespace (Jira refuses spaces in labels), no quotes.
pub fn valid_label(s: &str) -> bool {
    !s.is_empty() && s.chars().count() <= 255 && s.chars().all(|c| !c.is_whitespace() && !c.is_control() && c != '"' && c != '\\')
}

/// A Confluence space key: letters, digits, `_`, `-`, `.`; a personal space starts with `~`.
pub fn valid_space(s: &str) -> bool {
    let b = s.strip_prefix('~').unwrap_or(s);
    (1..=255).contains(&b.len()) && b.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
}

impl Rule {
    pub fn validate(&self) -> Result<(), String> {
        if self.projects.is_empty() {
            return Err("pick at least one Jira project".into());
        }
        if self.projects.len() > MAX_PROJECTS {
            return Err(format!("pick at most {MAX_PROJECTS} projects"));
        }
        if let Some(p) = self.projects.iter().find(|p| !valid_project(p)) {
            return Err(format!("{p:?} isn't a Jira project key"));
        }
        let needs_user = self.handover == Handover::Assignee;
        let needs_label = self.handover == Handover::Label;
        match &self.assignee {
            Some(u) if !valid_user(u) => return Err("that Jira user can't be used".into()),
            None if needs_user => return Err("pick the Jira user tickets are assigned to".into()),
            _ => {}
        }
        match &self.label {
            Some(l) if !valid_label(l) => return Err("a label has no spaces or quotes".into()),
            None if needs_label => return Err("say which label hands a ticket over".into()),
            _ => {}
        }
        Ok(())
    }

    /// `project in ("A","B")`.
    pub fn scope(&self) -> String {
        scope(&self.projects)
    }

    /// The handover clause alone: `assignee = "x"` or `labels = "kwant"`. A rule that passed
    /// [`Rule::validate`] always has the value its handover needs.
    fn handover_clause(&self) -> String {
        match self.handover {
            Handover::Assignee => format!("assignee = {}", quote(self.assignee.as_deref().unwrap_or_default())),
            Handover::Label => format!("labels = {}", quote(self.label.as_deref().unwrap_or("kwant"))),
        }
    }

    /// J4's JQL: `project in (…) AND <handover> AND statusCategory != Done ORDER BY created ASC`.
    pub fn jql(&self) -> String {
        format!("{} AND {} AND statusCategory != Done ORDER BY created ASC", self.scope(), self.handover_clause())
    }

    /// Whether one ticket still matches, by its fields (no search): the poller's guard against
    /// search lag. Done is checked separately (it closes rather than pauses).
    pub fn matches(&self, project: &str, assignee: Option<&str>, labels: &[String]) -> bool {
        if !self.projects.iter().any(|p| p == project) {
            return false;
        }
        match self.handover {
            Handover::Assignee => self.assignee.as_deref().is_some_and(|u| assignee == Some(u)),
            Handover::Label => self.label.as_deref().is_some_and(|l| labels.iter().any(|x| x == l)),
        }
    }
}

/// `project in ("A","B")`.
pub fn scope(projects: &[String]) -> String {
    format!("project in ({})", projects.iter().map(|p| quote(p)).collect::<Vec<_>>().join(", "))
}

/// The person's Advanced JQL, scoped: `project in (…) AND (<theirs>) ORDER BY created ASC`. Their
/// own `ORDER BY` is dropped (the queue has its own order, J13). Err when it's empty, too long, or
/// its quotes or parentheses don't balance (so it can't close our parenthesis early).
pub fn scoped_custom(projects: &[String], custom: &str) -> Result<String, String> {
    let custom = custom.trim();
    if custom.is_empty() {
        return Err("the JQL is empty".into());
    }
    if custom.chars().count() > MAX_JQL {
        return Err(format!("the JQL is longer than {MAX_JQL} characters"));
    }
    let body = strip_order_by(custom)?;
    if body.trim().is_empty() {
        return Err("the JQL has no condition".into());
    }
    Ok(format!("{} AND ({}) ORDER BY created ASC", scope(projects), body.trim()))
}

/// `jql` without a top-level `ORDER BY …`; Err when quotes or parentheses don't balance.
fn strip_order_by(jql: &str) -> Result<&str, String> {
    let bytes = jql.as_bytes();
    let (mut depth, mut quote_ch, mut i) = (0i32, None::<u8>, 0usize);
    let mut order_at = None;
    while i < bytes.len() {
        let c = bytes[i];
        match quote_ch {
            Some(q) => {
                if c == b'\\' {
                    i += 1;
                } else if c == q {
                    quote_ch = None;
                }
            }
            None => match c {
                b'"' | b'\'' => quote_ch = Some(c),
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth < 0 {
                        return Err("the JQL's parentheses don't match".into());
                    }
                }
                _ if depth == 0 && order_at.is_none() && jql[i..].len() >= 8 && jql[i..i + 5].eq_ignore_ascii_case("order") => {
                    let before_ok = i == 0 || bytes[i - 1].is_ascii_whitespace() || bytes[i - 1] == b')';
                    let after = jql[i + 5..].trim_start();
                    if before_ok && after.len() >= 2 && after[..2].eq_ignore_ascii_case("by") {
                        order_at = Some(i);
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    if quote_ch.is_some() {
        return Err("the JQL has a quote that isn't closed".into());
    }
    if depth != 0 {
        return Err("the JQL's parentheses don't match".into());
    }
    Ok(order_at.map_or(jql, |i| &jql[..i]))
}

/// Words for CQL `text ~` / `title ~`: Lucene's operators become spaces (a page search is words,
/// not a query language), at most 200 chars. Empty when nothing searchable is left.
pub fn search_words(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_alphanumeric() || c.is_whitespace() || matches!(c, '\'' | '.' | '_' | '@' | '#' | '/' | ',') { c } else { ' ' })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().filter(|w| !matches!(*w, "AND" | "OR" | "NOT")).collect();
    let joined = words.join(" ");
    crate::server::dev::tickets::source::cut(&joined, 200).0
}

/// CQL for a page search in `spaces` only: `type = page AND space in (…) AND text ~ "…"`.
/// `None` when no searchable word is left.
pub fn cql(spaces: &[String], words: &str, title_only: bool) -> Option<String> {
    let words = search_words(words);
    if words.is_empty() || spaces.is_empty() {
        return None;
    }
    let field = if title_only { "title" } else { "text" };
    let spaces = spaces.iter().map(|s| quote(s)).collect::<Vec<_>>().join(", ");
    Some(format!("type = page AND space in ({spaces}) AND {field} ~ {}", quote(&words)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(h: Handover) -> Rule {
        Rule { projects: vec!["TRAIL".into(), "OPS2".into()], handover: h, assignee: Some("u-kwant".into()), label: Some("kwant".into()) }
    }

    #[test]
    fn the_built_jql_is_scoped_and_quoted() {
        assert_eq!(rule(Handover::Assignee).jql(), r#"project in ("TRAIL", "OPS2") AND assignee = "u-kwant" AND statusCategory != Done ORDER BY created ASC"#);
        assert_eq!(Handover::parse("either"), None, "assignee OR label is Advanced JQL only (J4 as amended)");
        assert_eq!(rule(Handover::Label).jql(), r#"project in ("TRAIL", "OPS2") AND labels = "kwant" AND statusCategory != Done ORDER BY created ASC"#);
        let mut r = rule(Handover::Assignee);
        r.assignee = Some("ann\" OR project = HR".into());
        assert!(r.validate().is_err(), "a quote in a user id is refused");
        assert_eq!(quote(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[test]
    fn rules_are_validated() {
        let mut r = rule(Handover::Label);
        assert!(r.validate().is_ok());
        r.projects = vec!["trail".into()];
        assert!(r.validate().is_err());
        let mut r = rule(Handover::Label);
        r.label = Some("two words".into());
        assert!(r.validate().is_err());
        let mut r = rule(Handover::Assignee);
        r.assignee = None;
        assert!(r.validate().is_err());
        let mut r = rule(Handover::Label);
        r.projects.clear();
        assert!(r.validate().is_err());
    }

    #[test]
    fn a_ticket_is_rechecked_locally() {
        let by_user = rule(Handover::Assignee);
        assert!(by_user.matches("TRAIL", Some("u-kwant"), &[]));
        assert!(!by_user.matches("TRAIL", None, &["kwant".into()]), "the label alone doesn't hand over under the assignee rule");
        assert!(!by_user.matches("HR", Some("u-kwant"), &[]), "another project never matches");
        let by_label = rule(Handover::Label);
        assert!(by_label.matches("TRAIL", Some("u-ann"), &["kwant".into()]), "the label keeps the human assignee");
        assert!(!by_label.matches("TRAIL", Some("u-kwant"), &[]));
    }

    #[test]
    fn advanced_jql_stays_in_scope() {
        let p = vec!["TRAIL".to_string()];
        assert_eq!(
            scoped_custom(&p, r#"labels = kwant OR project = HR order by rank"#).unwrap(),
            r#"project in ("TRAIL") AND (labels = kwant OR project = HR) ORDER BY created ASC"#
        );
        assert_eq!(scoped_custom(&p, r#"summary ~ "order by" AND x = 1"#).unwrap(), r#"project in ("TRAIL") AND (summary ~ "order by" AND x = 1) ORDER BY created ASC"#);
        for bad in ["", "a = 1) OR (project = HR", "summary ~ \"x", "(a = 1", "ORDER BY created"] {
            assert!(scoped_custom(&p, bad).is_err(), "{bad}");
        }
        assert!(scoped_custom(&p, &"a".repeat(MAX_JQL + 1)).is_err());
    }

    #[test]
    fn cql_is_words_in_chosen_spaces() {
        let s = vec!["OPS".to_string()];
        assert_eq!(cql(&s, "orders export", false).unwrap(), r#"type = page AND space in ("OPS") AND text ~ "orders export""#);
        assert_eq!(cql(&s, r#"x" OR space = HR"#, true).unwrap(), r#"type = page AND space in ("OPS") AND title ~ "x space HR""#);
        assert_eq!(cql(&s, "*()~", false), None);
        assert_eq!(cql(&[], "x", false), None);
        assert!(valid_space("OPS") && valid_space("~5b10ac8d") && !valid_space("a b") && !valid_space(""));
    }
}
