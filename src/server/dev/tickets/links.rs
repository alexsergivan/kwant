//! Links stay on the site (api-decisions 18): the ticket's link, and every link in a brief or a
//! citation, must have the same origin (scheme, host and port) as the saved Jira or Confluence
//! site. kwant drops any other link before it is saved or sent, so the card only ever opens the
//! person's own Atlassian site. `http` passes only when the saved site is itself `http` (a Data
//! Center site), since the scheme is part of the origin.

use super::brief::{Brief, SourceRef};
use url::Url;

/// The site's origins: its Jira address and, on Data Center, its own Confluence address. An
/// address that isn't `http(s)` with a host gives none.
pub fn origins(jira_url: &str, confluence_url: Option<&str>) -> Vec<Url> {
    std::iter::once(jira_url).chain(confluence_url).filter_map(base).collect()
}

fn base(s: &str) -> Option<Url> {
    let u = Url::parse(s.trim()).ok()?;
    (matches!(u.scheme(), "http" | "https") && u.host().is_some()).then_some(u)
}

/// Whether a link is on one of the site's origins. A link with a user or password in it never is,
/// nor one that isn't `http(s)` (`javascript:`, `data:`, `file:`).
pub fn on_site(link: &str, origins: &[Url]) -> bool {
    let Some(u) = base(link) else { return false };
    if !u.username().is_empty() || u.password().is_some() {
        return false;
    }
    origins.iter().any(|o| o.origin() == u.origin())
}

/// The link when it is on the site, else empty (the card shows the chip without a link).
pub fn keep(link: &str, origins: &[Url]) -> String {
    if on_site(link, origins) {
        link.trim().to_string()
    } else {
        String::new()
    }
}

/// The sources whose link is on the site, in order. A source with no link (a file, or one whose
/// link was already dropped) stays; one linking anywhere else is left out.
pub fn site_sources(sources: &[SourceRef], origins: &[Url]) -> Vec<SourceRef> {
    sources.iter().filter(|s| s.url.trim().is_empty() || on_site(&s.url, origins)).cloned().collect()
}

/// Drops every link in a brief that isn't on the site: line and conflict sources, flags, and the
/// Jira comments of asked questions and answers. How many were dropped.
pub fn keep_site_links(b: &mut Brief, origins: &[Url]) -> usize {
    let mut n = 0;
    let mut one = |s: &mut String| {
        if !s.is_empty() && !on_site(s, origins) {
            s.clear();
            n += 1;
        }
    };
    let mut sources: Vec<&mut SourceRef> = Vec::new();
    for l in std::iter::once(&mut b.goal).chain(&mut b.done_when).chain(&mut b.where_in_code).chain(&mut b.plan).chain(&mut b.assumptions) {
        sources.extend(l.sources.iter_mut());
    }
    for f in &mut b.flags {
        sources.push(&mut f.from);
    }
    for q in &mut b.questions {
        sources.extend(q.conflict_sources.iter_mut());
        if let Some(a) = &mut q.asked_on_jira {
            one(&mut a.comment_url);
        }
        if let Some(url) = q.answer.as_mut().and_then(|a| a.comment_url.as_mut()) {
            one(url);
        }
        if let Some(r) = &mut q.jira_reply {
            one(&mut r.comment_url);
        }
    }
    for s in sources {
        one(&mut s.url);
    }
    for q in &mut b.questions {
        if let Some(a) = &mut q.answer {
            if a.comment_url.as_deref() == Some("") {
                a.comment_url = None;
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::dev::tickets::brief::{Answer, AskedOnJira, Flag, Line, Question};

    fn cloud() -> Vec<Url> {
        origins("https://yourcompany.atlassian.net", None)
    }

    #[test]
    fn only_the_sites_own_origin_passes() {
        let o = cloud();
        assert!(on_site("https://yourcompany.atlassian.net/browse/TRAIL-102", &o));
        assert!(on_site("https://YourCompany.Atlassian.net:443/wiki/spaces/OPS/pages/1", &o), "case and the default port are the same origin");
        for bad in [
            "javascript:alert(document.cookie)",
            "JavaScript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///etc/passwd",
            "http://yourcompany.atlassian.net/browse/TRAIL-102",
            "https://yourcompany.atlassian.net.evil.example/browse/TRAIL-102",
            "https://yourcompany-atlassian.net/browse/TRAIL-102",
            "https://xn--yourcompny-0db.atlassian.net/browse/TRAIL-102",
            "https://yourcompany.atlassian.net@evil.example/browse/TRAIL-102",
            "https://ann@yourcompany.atlassian.net/browse/TRAIL-102",
            "https://yourcompany.atlassian.net:8443/browse/TRAIL-102",
            "https://other.atlassian.net/browse/TRAIL-102",
            "//evil.example/x",
            "/browse/TRAIL-102",
            "",
        ] {
            assert!(!on_site(bad, &o), "{bad}");
            assert_eq!(keep(bad, &o), "", "{bad}");
        }
    }

    #[test]
    fn http_only_for_a_data_center_site_saved_as_http_and_its_own_confluence() {
        let dc = origins("http://jira.example.com:8080", Some("https://confluence.example.com"));
        assert!(on_site("http://jira.example.com:8080/browse/OPS-1", &dc));
        assert!(!on_site("https://jira.example.com:8080/browse/OPS-1", &dc), "the scheme is part of the origin");
        assert!(!on_site("http://jira.example.com/browse/OPS-1", &dc), "the port too");
        assert!(on_site("https://confluence.example.com/pages/viewpage.action?pageId=7", &dc));
        assert!(!on_site("http://confluence.example.com/pages/viewpage.action?pageId=7", &dc));
        assert!(origins("javascript:alert(1)", Some("not a url")).is_empty());
        assert!(!on_site("https://jira.example.com/x", &[]), "no site, no links");
    }

    #[test]
    fn a_brief_keeps_only_links_on_the_site() {
        let src = |url: &str| SourceRef { kind: "page".into(), label: "p".into(), url: url.into(), ..Default::default() };
        let line = |url: &str| Line { text: "t".into(), sources: vec![src(url)], inferred: false, by_person: false };
        let good = "https://yourcompany.atlassian.net/wiki/spaces/OPS/pages/65601";
        let mut b = Brief {
            goal: line(good),
            done_when: vec![line("javascript:alert(1)")],
            plan: vec![line("https://yourcompany.atlassian.net.evil.example/x")],
            flags: vec![Flag { quote: "q".into(), from: src("https://evil.example/"), why: "w".into(), by: "kwant".into() }],
            questions: vec![Question {
                conflict_sources: vec![src(good), src("http://yourcompany.atlassian.net/x")],
                asked_on_jira: Some(AskedOnJira { at: "t".into(), comment_url: "https://evil.example/c".into(), to: "Ann".into() }),
                answer: Some(Answer { comment_url: Some("javascript:void(0)".into()), ..Default::default() }),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(keep_site_links(&mut b, &cloud()), 6);
        assert_eq!(b.goal.sources[0].url, good);
        assert_eq!(b.questions[0].conflict_sources[0].url, good);
        assert_eq!((b.done_when[0].sources[0].url.as_str(), b.plan[0].sources[0].url.as_str(), b.flags[0].from.url.as_str()), ("", "", ""));
        assert_eq!(b.questions[0].conflict_sources[1].url, "");
        assert_eq!(b.questions[0].asked_on_jira.as_ref().unwrap().comment_url, "");
        assert_eq!(b.questions[0].answer.as_ref().unwrap().comment_url, None);
        assert_eq!(keep_site_links(&mut b, &cloud()), 0, "nothing left to drop");
    }

    /// api-decisions 21: a "Matches the ticket" item copies its done-when line's sources, and one
    /// linking off the site is dropped from it; files and on-site links stay, as does `inferred`.
    #[test]
    fn a_matches_item_keeps_only_the_sites_sources() {
        use crate::server::dev::tickets::brief::{check_from, Brief};
        let src = |kind: &str, url: &str| SourceRef { kind: kind.into(), label: kind.into(), url: url.into(), cite: kind.into(), ..Default::default() };
        let on = src("ticket", "https://yourcompany.atlassian.net/browse/TRAIL-102");
        let file = SourceRef { file: Some("src/a.rs".into()), ..src("file", "") };
        let lines = vec![
            Line { text: "A CSV downloads".into(), sources: vec![on.clone(), src("page", "https://yourcompany.atlassian.net.evil.example/wiki/x"), file.clone()], ..Default::default() },
            Line { text: "It's fast".into(), sources: vec![src("comment", "javascript:alert(1)"), src("page", "http://yourcompany.atlassian.net/wiki/y")], inferred: true, ..Default::default() },
        ];
        let b = Brief { key: "TRAIL-102".into(), done_when: lines, ..Default::default() };
        let t = check_from("TRAIL-102".into(), "https://evil.example.com/browse/TRAIL-102", &b, &cloud()).unwrap();
        assert_eq!(t.lines, ["A CSV downloads", "It's fast"]);
        assert_eq!(t.cites[0], (vec![on, file], false), "the lookalike host is dropped");
        assert_eq!(t.cites[1], (vec![], true), "javascript: and another scheme are dropped; inferred stays");
        assert_eq!(t.url, "", "the ticket link is kept on the site only");
        assert!(check_from("TRAIL-1".into(), "", &Brief::default(), &cloud()).is_none());
    }
}
