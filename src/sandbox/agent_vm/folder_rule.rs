//! Which folders a bot may share with its isolated Claude container (Ada's D5 as amended at
//! 15:05 UTC on 2026-11-08, after Mira found that D5 as first written refused every default
//! folder: `agents-in-vm/ada-decisions-2.md`, "Addendum").
//!
//! The folder is canonicalized first (symlinks resolved), then refused when it:
//! - is `/` ([`UnsafeKind::Root`]);
//! - is the person's home, or holds it ([`UnsafeKind::Home`], [`UnsafeKind::ContainsHome`]);
//!   a project folder *inside* the home is fine;
//! - is, holds or sits inside a sign-in folder: kwnt's account profiles, the turn's own
//!   credential folder, `~/.ssh`, `~/.claude`, `~/.codex`, `~/.grok` ([`UnsafeKind::SignIns`]);
//! - is, holds or sits inside the folder of Docker's control socket ([`UnsafeKind::Docker`]);
//! - is a space's shared default folder `<space data>/workspace`, or inside it
//!   ([`UnsafeKind::SharedFolder`]): every team without its own folder shares it, so D4's mark
//!   would then refuse Codex and Grok in all of them; the refusal lands on the bot that opted in;
//! - is or holds kwnt's data dir, or sits inside it anywhere but a bot's or a team's own work
//!   folder ([`UnsafeKind::KwantData`]).
//!
//! Inside the data dir only these are allowed, because a bot's and a repo team's default folders
//! live there: `<space data>/bot-workspaces/<bot>/…` and `<space data>/workspaces/<team>/…`, where
//! `<space data>` is the data dir (Default space) or `spaces/<space>`. Everything else there (the
//! database, vault, keys, profiles, checkpoints, the CLI homes in `agent-vm/`, `spaces/<space>`
//! and the parents `bot-workspaces/` and `workspaces/` themselves) is refused.

use std::path::{Component, Path, PathBuf};

/// Why a folder is refused. Sent to the chat as `unsafe_kind`, which picks the reason sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsafeKind {
    Home,
    ContainsHome,
    SignIns,
    KwantData,
    Root,
    Docker,
    SharedFolder,
}

impl UnsafeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            UnsafeKind::Home => "home",
            UnsafeKind::ContainsHome => "contains_home",
            UnsafeKind::SignIns => "sign_ins",
            UnsafeKind::KwantData => "kwant_data",
            UnsafeKind::Root => "root",
            UnsafeKind::Docker => "docker",
            UnsafeKind::SharedFolder => "shared_folder",
        }
    }
}

/// The places a shared folder is checked against, canonicalized where they exist.
#[derive(Debug, Clone, Default)]
pub struct Places {
    pub home: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub sign_ins: Vec<PathBuf>,
    pub docker: Vec<PathBuf>,
}

/// Credential folders under the person's home that are never shared.
const HOME_SIGN_INS: &[&str] = &[".ssh", ".claude", ".codex", ".grok"];

impl Places {
    /// This computer's places, for a turn whose credential lives in `credential_home`.
    pub fn for_host(data_dir: &Path, credential_home: Option<&Path>) -> Self {
        let home = dirs::home_dir();
        let mut sign_ins = vec![data_dir.join("profiles")];
        sign_ins.extend(credential_home.map(Path::to_path_buf));
        if let Some(h) = &home {
            sign_ins.extend(HOME_SIGN_INS.iter().map(|d| h.join(d)));
        }
        Self {
            home: home.map(|h| canon(&h)),
            data_dir: canon(data_dir),
            sign_ins: sign_ins.iter().map(|p| canon(p)).collect(),
            docker: docker_socket_dirs().iter().map(|p| canon(p)).collect(),
        }
    }
}

/// `None` when `folder` (already canonical) may be shared, else why not. The order picks the
/// most specific sentence: a sign-in folder inside the data dir says "sign-ins", not "kwant data".
pub fn classify(folder: &Path, p: &Places) -> Option<UnsafeKind> {
    if folder.parent().is_none() {
        return Some(UnsafeKind::Root);
    }
    if let Some(home) = &p.home {
        if folder == home {
            return Some(UnsafeKind::Home);
        }
        if home.starts_with(folder) {
            return Some(UnsafeKind::ContainsHome);
        }
    }
    // Holding the data dir also holds its sign-ins; Mira's `kwant_data` sentence says both
    if p.data_dir.starts_with(folder) {
        return Some(UnsafeKind::KwantData);
    }
    let related = |s: &PathBuf| s.starts_with(folder) || folder.starts_with(s);
    if p.sign_ins.iter().any(related) {
        return Some(UnsafeKind::SignIns);
    }
    if p.docker.iter().any(related) {
        return Some(UnsafeKind::Docker);
    }
    match folder.strip_prefix(&p.data_dir) {
        Ok(rel) => in_data_dir(rel),
        Err(_) => None,
    }
}

/// For a path relative to the data dir: `None` inside a bot's or a team's own work folder,
/// else why it's refused.
fn in_data_dir(rel: &Path) -> Option<UnsafeKind> {
    let parts: Vec<&str> = rel
        .components()
        .map(|c| match c {
            Component::Normal(s) => s.to_str().unwrap_or("\u{0}"),
            _ => "\u{0}",
        })
        .collect();
    let in_space = |p: &[&str]| match p {
        ["workspaces", _, ..] | ["bot-workspaces", _, ..] => None,
        ["workspace", ..] => Some(UnsafeKind::SharedFolder),
        _ => Some(UnsafeKind::KwantData),
    };
    match parts.as_slice() {
        ["spaces", _, rest @ ..] if !rest.is_empty() => in_space(rest),
        ["spaces", ..] => Some(UnsafeKind::KwantData),
        all => in_space(all),
    }
}

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Folders holding a Docker control socket: the system one, a `unix://` `DOCKER_HOST`, and the
/// rootless one in `XDG_RUNTIME_DIR`.
fn docker_socket_dirs() -> Vec<PathBuf> {
    let mut socks = vec![PathBuf::from("/var/run/docker.sock"), PathBuf::from("/run/docker.sock")];
    if let Some(h) = std::env::var("DOCKER_HOST").ok().and_then(|h| h.strip_prefix("unix://").map(PathBuf::from)) {
        socks.push(h);
    }
    if let Some(x) = std::env::var_os("XDG_RUNTIME_DIR") {
        socks.push(PathBuf::from(x).join("docker.sock"));
    }
    let mut dirs: Vec<PathBuf> = socks.iter().filter(|s| s.exists()).filter_map(|s| canon(s).parent().map(Path::to_path_buf)).collect();
    dirs.sort();
    dirs.dedup();
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places() -> Places {
        Places {
            home: Some("/home/p".into()),
            data_dir: "/home/p/.kwnt".into(),
            sign_ins: vec![
                "/home/p/.kwnt/profiles".into(),
                "/srv/creds/claude".into(),
                "/home/p/.ssh".into(),
                "/home/p/.claude".into(),
                "/home/p/.codex".into(),
                "/home/p/.grok".into(),
            ],
            docker: vec!["/run".into()],
        }
    }

    fn kind(f: &str) -> Option<&'static str> {
        classify(Path::new(f), &places()).map(UnsafeKind::as_str)
    }

    #[test]
    fn a_default_solo_bot_folder_is_accepted() {
        // tools::bot_workspace(space_data_dir, id) for the Default space and another space
        let data = Path::new("/home/p/.kwnt");
        let def = crate::tools::bot_workspace_path(data, "kw_abc");
        assert_eq!(def, Path::new("/home/p/.kwnt/bot-workspaces/kw_abc"));
        assert_eq!(classify(&def, &places()), None);
        let other = crate::tools::bot_workspace_path(&data.join("spaces").join("spc_2"), "kw_abc");
        assert_eq!(classify(&other, &places()), None);
    }

    #[test]
    fn a_repo_teams_default_folder_is_accepted() {
        let data = Path::new("/home/p/.kwnt");
        let f = crate::tools::team_workspace_path(data, "Checkout", None, Some("https://github.com/a/b"));
        assert_eq!(f, Path::new("/home/p/.kwnt/workspaces/Checkout"));
        assert_eq!(classify(&f, &places()), None);
        for ok in [
            "/home/p/.kwnt/workspaces/Checkout/src",
            "/home/p/.kwnt/spaces/spc_2/workspaces/Checkout",
            "/home/p/.kwnt/spaces/spc_2/bot-workspaces/kw_1/sub",
            "/home/p/projects/site",
            "/srv/other",
        ] {
            assert_eq!(kind(ok), None, "{ok}");
        }
    }

    #[test]
    fn the_shared_default_folder_is_refused_as_shared() {
        let data = Path::new("/home/p/.kwnt");
        let f = crate::tools::team_workspace_path(data, "Checkout", None, None);
        assert_eq!(f, Path::new("/home/p/.kwnt/workspace"));
        assert_eq!(classify(&f, &places()), Some(UnsafeKind::SharedFolder));
        for bad in ["/home/p/.kwnt/workspace/site", "/home/p/.kwnt/spaces/spc_2/workspace"] {
            assert_eq!(kind(bad), Some("shared_folder"), "{bad}");
        }
    }

    #[test]
    fn the_rest_of_the_data_dir_is_refused() {
        for bad in [
            "/home/p/.kwnt",
            "/home/p/.kwnt/workspaces",
            "/home/p/.kwnt/bot-workspaces",
            "/home/p/.kwnt/spaces",
            "/home/p/.kwnt/spaces/spc_2",
            "/home/p/.kwnt/spaces/spc_2/workspaces",
            "/home/p/.kwnt/spaces/spc_2/bot-workspaces",
            "/home/p/.kwnt/agent-vm",
            "/home/p/.kwnt/agent-vm/0123abcd/home",
            "/home/p/.kwnt/checkpoints/t1",
            "/home/p/.kwnt/browser-cookies",
        ] {
            assert_eq!(kind(bad), Some("kwant_data"), "{bad}");
        }
    }

    #[test]
    fn each_refusal_names_its_kind() {
        let cases = [
            ("/", "root"),
            ("/home/p", "home"),
            ("/home", "contains_home"),
            ("/home/p/.kwnt/profiles", "sign_ins"),
            ("/home/p/.kwnt/profiles/pr_1", "sign_ins"),
            ("/home/p/.ssh", "sign_ins"),
            ("/home/p/.claude/projects", "sign_ins"),
            ("/home/p/.codex", "sign_ins"),
            ("/home/p/.grok", "sign_ins"),
            ("/srv/creds", "sign_ins"),
            ("/srv/creds/claude/x", "sign_ins"),
            ("/run", "docker"),
            ("/run/user/1000", "docker"),
        ];
        for (f, k) in cases {
            assert_eq!(kind(f), Some(k), "{f}");
        }
    }

    #[test]
    fn a_name_that_only_looks_like_a_work_folder_is_refused() {
        // starts_with compares whole components, and only kwnt's own layout counts
        assert_eq!(kind("/home/p/.kwnt/workspace-old"), Some("kwant_data"));
        assert_eq!(kind("/home/p/.kwnt/profiles/../workspace"), Some("sign_ins"));
        assert_eq!(kind("/home/p/.kwnt/x/bot-workspaces/kw_1"), Some("kwant_data"));
    }
}
