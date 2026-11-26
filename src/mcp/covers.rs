//! sign-in-wall-uses-tools A: which of a bot's MCP servers cover a website, so kwnt can point the
//! bot at its tools instead of sending the person to sign in on that site in the bot browser.
//!
//! A server covers a host when the host's site label (`github` for github.com, gist.github.com
//! or api.github.com) is the server's name (lower-cased, `mcp` stripped: `github-mcp` → `github`),
//! or a word of its URL host (`api.githubcopilot.com` → `github` through [`ALIASES`]), or a word
//! of its stdio command or arguments (`@modelcontextprotocol/server-github`, `github-mcp-server`).
//! Words, not substrings: a `gitlab` server never covers github.com. No network calls; whether
//! the server works is asked separately (the Tools screen's last test).
//!
//! Composio apps aren't checked here: kwnt keeps no list of a space's connected toolkits (asking
//! Composio is a network call), so only MCP servers count.

use crate::db::models::McpServer;

/// Host words that stand for another site's label: GitHub's MCP endpoint lives on
/// githubcopilot.com, its raw files on githubusercontent.com.
const ALIASES: [(&str, &str); 2] = [("githubcopilot", "github"), ("githubusercontent", "github")];

/// Two-part public suffixes common enough to matter (`bbc.co.uk` → `bbc`). Anything else takes the
/// label left of the last dot.
const TWO_PART_SUFFIXES: [&str; 12] =
    ["co.uk", "org.uk", "ac.uk", "gov.uk", "com.au", "net.au", "org.au", "co.jp", "co.nz", "com.br", "co.in", "com.mx"];

fn alias(word: &str) -> &str {
    ALIASES.iter().find(|(from, _)| *from == word).map(|(_, to)| *to).unwrap_or(word)
}

/// The site label of a host: `github` for `github.com`, `gist.github.com` and `api.github.com`;
/// `bbc` for `www.bbc.co.uk`. `None` for an IP address, `localhost` or a label under 2 characters.
pub fn site_label(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
        return None;
    }
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    if labels.len() < 2 {
        return None;
    }
    let last_two = labels[labels.len() - 2..].join(".");
    let label = if TWO_PART_SUFFIXES.contains(&last_two.as_str()) && labels.len() >= 3 {
        labels[labels.len() - 3]
    } else {
        labels[labels.len() - 2]
    };
    let label = alias(label);
    (label.chars().count() >= 2).then(|| label.to_string())
}

/// The words of a name, host or command: split on anything that isn't a letter or digit.
fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).map(|w| alias(&w.to_ascii_lowercase()).to_string())
}

/// A server's name as a site label: lower-cased, with `mcp` taken off either end (`GitHub MCP`,
/// `github-mcp`, `mcp_github` → `github`).
fn name_label(name: &str) -> String {
    let words: Vec<String> = words(name).filter(|w| w != "mcp").collect();
    words.join("-")
}

/// Whether `server` covers the site labelled `label` ([`site_label`]). Off servers never do.
pub fn server_covers(server: &McpServer, label: &str) -> bool {
    if !server.enabled || label.is_empty() {
        return false;
    }
    if name_label(&server.name) == label {
        return true;
    }
    let host_has = server
        .url
        .as_deref()
        .and_then(|u| url::Url::parse(u).ok())
        .and_then(|u| u.host_str().map(str::to_string))
        .is_some_and(|h| words(&h).any(|w| w == label));
    if host_has {
        return true;
    }
    // stdio: the words of the command's file name and of the arguments that name what runs
    // (`npx -y @modelcontextprotocol/server-github`, `github-mcp-server stdio`). Compared only,
    // never shown: arguments can hold tokens
    server.transport == "stdio"
        && server.command.as_deref().map(file_name).into_iter().chain(naming_args(&server.args)).any(|part| words(part).any(|w| w == label))
}

/// The last part of a path (`/usr/local/bin/github-mcp-server` → `github-mcp-server`): the folder
/// a program sits in says nothing about what it is for.
fn file_name(command: &str) -> &str {
    command.rsplit(['/', '\\']).next().unwrap_or(command)
}

/// Whether an argument is a place on disk (`/Users/oleks/GitHub`, `~/github/kwant`, `./repo`,
/// `C:\code`, `$HOME/src`) or an address (`git+https://github.com/x/y`): a folder or a source
/// a server works on or comes from, not the site it is for (#1262 F2).
fn is_place(arg: &str) -> bool {
    let mut chars = arg.chars();
    let (first, second) = (chars.next(), chars.next());
    matches!(first, Some('/' | '~' | '.' | '\\' | '$' | '%'))
        || (first.is_some_and(|c| c.is_ascii_alphabetic()) && second == Some(':'))
        || arg.contains("://")
}

/// Long flags that take no value, so the argument after them is still the server's own
/// (`docker run -i --rm ghcr.io/github/github-mcp-server`).
const SWITCHES: [&str; 9] = ["--rm", "--yes", "--quiet", "--stdio", "--interactive", "--read-only", "--init", "--offline", "--prefer-offline"];

/// The arguments that can say which site a stdio server is for: not paths or addresses, not the
/// value of a long flag (`--repository ~/github/kwant`, `--dir=GitHub`), and only the name of a
/// `NAME=value` pair. A one-letter flag (`-y`, `-e`) is taken as a switch: `npx -y <package>`.
fn naming_args(args: &[String]) -> impl Iterator<Item = &str> + '_ {
    let mut value_next = false;
    args.iter().filter_map(move |arg| {
        let arg = arg.trim();
        if let Some(long) = arg.strip_prefix("--") {
            value_next = !long.is_empty() && !long.contains('=') && !SWITCHES.contains(&arg);
            return None;
        }
        if arg.starts_with('-') {
            value_next = false;
            return None;
        }
        if std::mem::take(&mut value_next) || is_place(arg) {
            return None;
        }
        let name = arg.split('=').next().unwrap_or(arg);
        (!name.is_empty()).then_some(name)
    })
}

/// The servers among `servers` that cover `host`, in their order. Empty when the host has no
/// usable site label.
pub fn covering<'a>(servers: &'a [McpServer], host: &str) -> Vec<&'a McpServer> {
    let Some(label) = site_label(host) else { return Vec::new() };
    servers.iter().filter(|s| server_covers(s, &label)).collect()
}

/// The bot's active MCP servers (on, in its space, not opted out) that cover `host`.
pub async fn for_kwant(db: &crate::db::Database, kwant_id: &str, host: &str) -> Vec<McpServer> {
    let servers = db.list_active_mcp_servers_for_kwant(kwant_id).await.unwrap_or_default();
    covering(&servers, host).into_iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str, url: Option<&str>, command: Option<&str>, args: &[&str]) -> McpServer {
        McpServer {
            id: format!("id-{name}"),
            name: name.into(),
            transport: if url.is_some() { "http" } else { "stdio" }.into(),
            command: command.map(str::to_string),
            args: args.iter().map(|a| a.to_string()).collect(),
            url: url.map(str::to_string),
            env: Default::default(),
            headers: Default::default(),
            enabled: true,
            timeout_secs: 30,
            created_at: None,
        }
    }

    fn names(servers: &[McpServer], host: &str) -> Vec<String> {
        covering(servers, host).into_iter().map(|s| s.name.clone()).collect()
    }

    #[test]
    fn a_host_has_one_site_label() {
        assert_eq!(site_label("github.com").as_deref(), Some("github"));
        assert_eq!(site_label("gist.github.com").as_deref(), Some("github"));
        assert_eq!(site_label("API.GitHub.com.").as_deref(), Some("github"));
        assert_eq!(site_label("api.githubcopilot.com").as_deref(), Some("github"));
        assert_eq!(site_label("www.bbc.co.uk").as_deref(), Some("bbc"));
        assert_eq!(site_label("localhost"), None);
        assert_eq!(site_label("127.0.0.1"), None);
        assert_eq!(site_label("x.com"), None, "a one-letter label would match too much");
    }

    /// Oleks's case: a server named `github` at api.githubcopilot.com covers github.com.
    #[test]
    fn a_github_server_covers_github_by_name_url_or_command() {
        let by_name = [server("GitHub MCP", Some("https://mcp.example.net/"), None, &[])];
        let by_url = [server("work tools", Some("https://api.githubcopilot.com/mcp/"), None, &[])];
        let by_package = [server("code", None, Some("npx"), &["-y", "@modelcontextprotocol/server-github"])];
        let by_binary = [server("code", None, Some("/usr/local/bin/github-mcp-server"), &["stdio"])];
        for servers in [&by_name[..], &by_url[..], &by_package[..], &by_binary[..]] {
            for host in ["github.com", "gist.github.com", "api.github.com"] {
                assert_eq!(names(servers, host).len(), 1, "{:?} on {host}", servers[0].name);
            }
        }
        let oleks = [server("github", Some("https://api.githubcopilot.com/mcp/"), None, &[])];
        assert_eq!(names(&oleks, "github.com"), ["github"]);
    }

    #[test]
    fn a_near_miss_doesnt_cover() {
        let servers = [
            server("gitlab", Some("https://gitlab.com/api/v4/mcp"), None, &[]),
            server("git", None, Some("uvx"), &["mcp-server-git"]),
            server("notion", Some("https://mcp.notion.com/mcp"), None, &[]),
        ];
        assert!(names(&servers, "github.com").is_empty());
        assert_eq!(names(&servers, "gitlab.com"), ["gitlab"]);
        assert_eq!(names(&servers, "www.notion.so"), ["notion"]);
        assert!(names(&servers, "example.com").is_empty());
    }

    /// Rigel's F2: a folder or repository a stdio server works on isn't the site it is for.
    #[test]
    fn a_stdio_servers_folder_or_flag_value_doesnt_cover() {
        let files = server("files", None, Some("npx"), &["-y", "@modelcontextprotocol/server-filesystem", "/Users/oleks/GitHub"]);
        let git = server("git", None, Some("uvx"), &["mcp-server-git", "--repository", "~/github/kwant"]);
        let git_eq = server("git", None, Some("uvx"), &["mcp-server-git", "--repository=github"]);
        let windows = server("files", None, Some("npx.cmd"), &["-y", "@modelcontextprotocol/server-filesystem", "C:\\Users\\o\\GitHub"]);
        let relative = server("files", None, Some("node"), &["./GitHub/server.js", "$HOME/github"]);
        let from_github = server("tasks", None, Some("uvx"), &["--from", "git+https://github.com/someone/tasks-mcp", "tasks-mcp"]);
        let in_github_dir = server("tasks", None, Some("/Users/oleks/GitHub/bin/tasks-mcp"), &[]);
        for s in [files, git, git_eq, windows, relative, from_github, in_github_dir] {
            assert!(names(std::slice::from_ref(&s), "github.com").is_empty(), "{:?} {:?} {:?}", s.name, s.command, s.args);
        }
    }

    #[test]
    fn a_github_stdio_server_still_covers_github() {
        let servers = [
            server("a", None, Some("npx"), &["-y", "@modelcontextprotocol/server-github"]),
            server("b", None, Some("github-mcp-server"), &["stdio"]),
            server("c", None, Some("C:\\tools\\github-mcp-server.exe"), &["stdio", "--toolsets", "repos"]),
            server("d", None, Some("docker"), &["run", "-i", "--rm", "-e", "GITHUB_PERSONAL_ACCESS_TOKEN", "ghcr.io/github/github-mcp-server"]),
            server("e", None, Some("docker"), &["run", "-i", "--rm", "ghcr.io/github/github-mcp-server"]),
            server("f", None, Some("npx"), &["-y", "@modelcontextprotocol/server-github", "--dir", "/Users/oleks/GitHub"]),
        ];
        for s in &servers {
            assert_eq!(names(std::slice::from_ref(s), "github.com").len(), 1, "{:?} {:?}", s.command, s.args);
        }
    }

    #[test]
    fn an_off_server_doesnt_cover() {
        let mut off = server("github", Some("https://api.githubcopilot.com/mcp/"), None, &[]);
        off.enabled = false;
        assert!(names(&[off], "github.com").is_empty());
    }

    /// An opted-out server (the bot has it switched off) isn't among the bot's active servers,
    /// so it never covers a site for that bot; another bot in the space still has it.
    #[tokio::test]
    async fn a_server_the_bot_opted_out_of_doesnt_cover_for_it() {
        let tmp = tempfile::tempdir().unwrap();
        let db = crate::db::Database::init(tmp.path()).await.unwrap();
        let space = crate::db::spaces::SpaceId::default_space();
        let mut ids = Vec::new();
        for name in ["Ann", "Ben"] {
            let bot = crate::db::models::NewKwant {
                name: name.into(),
                title: String::new(),
                persona_prompt: String::new(),
                soul: String::new(),
                avatar_sprite: String::new(),
                driver_type: "api".into(),
                preferred_model: "auto".into(),
            };
            ids.push(db.create_kwant(&space, &bot).await.unwrap().id);
        }
        let mut gh = server("github", Some("https://api.githubcopilot.com/mcp/"), None, &[]);
        gh.id = db.create_mcp_server(&space, &gh).await.unwrap().id;
        db.set_kwant_mcp_servers(&ids[0], &[]).await.unwrap();
        assert!(for_kwant(&db, &ids[0], "github.com").await.is_empty(), "Ann opted out");
        assert_eq!(for_kwant(&db, &ids[1], "github.com").await.len(), 1, "Ben still has it");
    }
}
