//! `kwnt doctor`: every part kwant needs on this machine, whether it works, and the exact command
//! that fixes it here. The fix text is per platform (Mac, Linux, inside Docker), and Settings shows
//! the same advice for the computer modes and the Laya card.

use crate::config::Config;
use crate::profiles::Engine;
use std::path::Path;
use std::time::Duration;

/// Where kwnt runs, which decides how a missing part is installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Linux,
    /// Inside a container, e.g. the compose stack: parts come from compose services and build args.
    Docker,
}

impl Platform {
    /// This process's platform. `KWNT_IN_DOCKER=1` (set by kwant's image) or `/.dockerenv` means
    /// Docker.
    pub fn detect() -> Self {
        let flagged = std::env::var("KWNT_IN_DOCKER").is_ok_and(|v| v == "1");
        Self::from_parts(std::env::consts::OS, flagged || Path::new("/.dockerenv").exists())
    }

    pub fn from_parts(os: &str, in_docker: bool) -> Self {
        match (in_docker, os) {
            (true, _) => Platform::Docker,
            (false, "macos") => Platform::Mac,
            _ => Platform::Linux,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Platform::Mac => "Mac",
            Platform::Linux => "Linux",
            Platform::Docker => "Docker container",
        }
    }
}

/// How to get a Chrome for local mode.
pub fn chrome_fix(p: Platform) -> &'static str {
    match p {
        Platform::Mac => "kwnt setup --only browser   (or: brew install --cask google-chrome, or set KWNT_CHROME_PATH to a Chrome you have)",
        Platform::Linux => "kwnt setup --only browser   (a private Chromium without admin rights, or the sudo command where Linux blocks its sandbox; or: sudo apt-get install -y chromium, Ubuntu: sudo snap install chromium; or set KWNT_CHROME_PATH)",
        Platform::Docker => "Inside Docker, use VM mode with the compose browser service: docker compose up -d browser",
    }
}

/// How to make VM mode's browser available.
pub fn vm_fix(p: Platform) -> &'static str {
    match p {
        Platform::Mac => "Install and open Docker Desktop: brew install --cask docker",
        Platform::Linux => "Install Docker: curl -fsSL https://get.docker.com | sudo sh && sudo usermod -aG docker $USER   (then log in again), or set KWNT_BROWSER_CDP_URL to a private Chromium",
        Platform::Docker => "Set KWNT_BROWSER_CDP_URL=http://browser:9222 for kwnt and start the compose browser service: docker compose up -d browser",
    }
}

/// How to get Laya answering at the configured URL.
pub fn laya_fix(p: Platform, managed: bool) -> &'static str {
    match (p, managed) {
        (Platform::Docker, _) => "Start the Laya service: docker compose up -d laya   (logs: docker compose logs laya; kwnt reads LAYA_URL=http://laya:8000)",
        (Platform::Mac | Platform::Linux, true) => "kwnt setup --only laya   (installs Laya and its model into kwant's data folder, no admin rights; `kwnt serve` also does it on first start)",
        (_, false) => "Let kwnt run Laya: start `kwnt serve` without --no-laya and without KWNT_LAYA_MANAGED=0, or start laya-serve at LAYA_URL yourself",
    }
}

/// How to install an engine's CLI here.
pub fn cli_install_fix(engine: Engine, p: Platform) -> &'static str {
    match (engine, p) {
        (_, Platform::Docker) => "Rebuild the image with the CLIs: KWNT_CLIS=claude,codex,grok docker compose up -d --build",
        (Engine::Claude, _) => "kwnt setup --clis claude   (or: curl -fsSL https://claude.ai/install.sh | bash)",
        (Engine::Codex, Platform::Mac) => "kwnt setup --clis codex   (or: brew install --cask codex, or npm install -g @openai/codex)",
        (Engine::Codex, _) => "kwnt setup --clis codex   (or: npm install -g @openai/codex; needs Node.js 18+)",
        (Engine::Grok, _) => "kwnt setup --clis grok   (or: curl -fsSL https://x.ai/cli/install.sh | bash)",
        (Engine::Custom, _) => "Add a Custom API account in the app: Settings › This space › Accounts",
    }
}

/// How to sign an engine's CLI in here (or the API-key alternative).
pub fn cli_sign_in_fix(engine: Engine, p: Platform) -> String {
    let (cmd, key) = match engine {
        Engine::Claude => ("claude auth login", "ANTHROPIC_API_KEY"),
        Engine::Codex => ("codex login --device-auth", "OPENAI_API_KEY"),
        Engine::Grok => ("grok login --device-auth", "XAI_API_KEY"),
        Engine::Custom => return cli_install_fix(engine, p).to_string(),
    };
    let run = match p {
        Platform::Docker => format!("docker compose exec kwnt {cmd}"),
        _ => cmd.to_string(),
    };
    format!("{run}   (or set {key}, or use Sign in / an API key in the app: Settings › This space › Accounts)")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    /// Works without it, but something is missing or worth doing.
    Warn,
    /// kwant can't do its job without it.
    Fail,
    /// Not used on this platform.
    Skip,
}

impl Status {
    fn mark(self) -> &'static str {
        match self {
            Status::Ok => "ok  ",
            Status::Warn => "warn",
            Status::Fail => "FAIL",
            Status::Skip => "  - ",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Check {
    pub name: String,
    pub status: Status,
    pub detail: String,
    pub fix: Option<String>,
    /// Not needed here (another part covers it, or it's an extra like VM mode): shown, but not
    /// counted as missing.
    pub optional: bool,
}

impl Check {
    fn new(name: &str, status: Status, detail: impl Into<String>, fix: Option<String>) -> Self {
        Self { name: name.into(), status, detail: detail.into(), fix, optional: false }
    }

    fn optional_if(mut self, yes: bool) -> Self {
        self.optional = yes && self.status != Status::Ok;
        self
    }
}

/// What the checks found on this machine; gathered by [`gather`], judged by [`checks`].
#[derive(Clone, Debug)]
pub struct Facts {
    pub data_dir: String,
    pub data_dir_writable: Result<(), String>,
    pub laya_url: String,
    pub laya_managed: bool,
    pub laya_up: bool,
    /// `kwnt serve` runs Laya here and it is installed in the data folder.
    pub laya_installed: bool,
    pub cdp_url: Option<String>,
    pub cdp_up: bool,
    pub docker: bool,
    pub chrome: Option<String>,
    /// Per engine: installed binary, signed in (`None`: unknown), signed in via an API key env.
    pub clis: Vec<(Engine, Option<String>, Option<bool>, bool)>,
    pub loopback_bind: bool,
    pub token: bool,
    pub https_origin: Option<String>,
}

/// Turns the facts into checks with fixes for `p`.
pub fn checks(f: &Facts, p: Platform) -> Vec<Check> {
    let mut out = Vec::new();
    out.push(match &f.data_dir_writable {
        Ok(()) => Check::new("Data folder", Status::Ok, format!("{} is writable", f.data_dir), None),
        Err(e) => Check::new(
            "Data folder",
            Status::Fail,
            format!("{}: {e}", f.data_dir),
            Some(match p {
                Platform::Docker => "docker compose run --rm -u root kwnt chown -R 10001:10001 /data".into(),
                _ => format!("Run kwnt as the user that owns {0}, or: sudo chown -R $USER {0}", f.data_dir),
            }),
        ),
    });

    out.push(if f.laya_up {
        Check::new("Laya router", Status::Ok, format!("answers at {}", f.laya_url), None)
    } else if f.laya_installed && p != Platform::Docker {
        Check::new("Laya router", Status::Ok, "installed; `kwnt serve` starts it", None)
    } else {
        Check::new(
            "Laya router",
            Status::Warn,
            format!("not answering at {}; bots are picked by keywords", f.laya_url),
            Some(laya_fix(p, f.laya_managed).into()),
        )
    });

    // The browser: VM (a CDP service or Docker) and/or local Chrome; one of them is enough.
    let vm = match &f.cdp_url {
        Some(url) if f.cdp_up => (Status::Ok, format!("browser service answers at {url}")),
        Some(url) => (Status::Fail, format!("browser service at {url} isn't answering")),
        None if f.docker => (Status::Ok, "Docker is running; kwant starts its browser container on first use".into()),
        None => (Status::Fail, "no Docker and no KWNT_BROWSER_CDP_URL".into()),
    };
    let local = match (&f.chrome, p) {
        (_, Platform::Docker) => (Status::Skip, "not used inside Docker".to_string()),
        (Some(path), _) => (Status::Ok, format!("Chrome at {path}")),
        (None, _) => (Status::Fail, "no Chrome or Chromium found".to_string()),
    };
    let any_browser = vm.0 == Status::Ok || local.0 == Status::Ok;
    let soften = |s: Status| if s == Status::Fail && any_browser { Status::Warn } else { s };
    let vm_status = soften(vm.0);
    let local_ok = local.0 == Status::Ok;
    out.push(
        Check::new("Browser (VM)", vm_status, vm.1, (vm_status != Status::Ok).then(|| vm_fix(p).into()))
            .optional_if(local_ok),
    );
    let local_status = soften(local.0);
    out.push(
        Check::new(
            "Browser (local)",
            local_status,
            local.1,
            matches!(local_status, Status::Warn | Status::Fail).then(|| chrome_fix(p).into()),
        )
        .optional_if(vm.0 == Status::Ok),
    );

    out.push(match p {
        Platform::Docker => Check::new(
            "kwant shell",
            Status::Skip,
            "kwant's own execute_shell needs Docker on the host; inside the container bots use their CLI's shell tools",
            None,
        ),
        _ if f.docker => Check::new("kwant shell", Status::Ok, "Docker runs kwant's execute_shell containers", None),
        _ => Check::new(
            "kwant shell",
            Status::Warn,
            "no Docker: kwant's execute_shell is off in VM mode (the CLIs' own shell tools still work)",
            Some(vm_fix(p).into()),
        )
        .optional_if(true),
    });

    let mut ready = 0;
    let any_installed = f.clis.iter().any(|(_, bin, _, key)| bin.is_some() || *key);
    for (engine, bin, signed_in, via_key) in &f.clis {
        let name = engine.as_str();
        let check = match (bin, signed_in, via_key) {
            (_, _, true) => {
                ready += 1;
                Check::new(name, Status::Ok, "API key set in the environment", None)
            }
            (None, _, _) => Check::new(name, Status::Warn, "not installed", Some(cli_install_fix(*engine, p).into()))
                .optional_if(any_installed),
            (Some(path), Some(true), _) => {
                ready += 1;
                Check::new(name, Status::Ok, format!("{path}, signed in"), None)
            }
            (Some(path), state, _) => Check::new(
                name,
                Status::Warn,
                format!("{path}, {}", if state.is_none() { "sign-in state unknown" } else { "not signed in" }),
                Some(cli_sign_in_fix(*engine, p)),
            ),
        };
        out.push(check);
    }
    if ready == 0 {
        out.push(Check::new(
            "AI account",
            Status::Warn,
            "no CLI is signed in here (API keys added in the app aren't checked by doctor)",
            Some("Sign one CLI in (see above) or add an API key in the app: Settings › This space › Accounts".into()),
        ));
    }

    out.push(if f.loopback_bind {
        Check::new("Access", Status::Ok, "listening on this machine only", None)
    } else if !f.token {
        Check::new(
            "Access",
            Status::Warn,
            "no sign-in token yet; `kwnt serve` creates one on first start",
            Some(match p {
                Platform::Docker => "docker compose exec kwnt kwnt token --raw".into(),
                _ => "kwnt token".into(),
            }),
        )
    } else if let Some(origin) = &f.https_origin {
        Check::new("Access", Status::Ok, format!("token set; HTTPS origin {origin}"), None)
    } else {
        Check::new(
            "Access",
            Status::Warn,
            "token set, but no HTTPS origin: don't open kwant to the internet over plain http",
            Some("Put HTTPS in front, e.g. Caddy: `your.domain { reverse_proxy 127.0.0.1:8799 }`, and set KWNT_ALLOWED_ORIGINS=https://your.domain".into()),
        )
    });
    out
}

/// Looks at this machine. Network checks time out after a few seconds each.
pub async fn gather(config: &Config) -> Facts {
    let http = reqwest::Client::builder().timeout(Duration::from_secs(3)).build().unwrap_or_default();
    let laya_url = config.laya_url.trim_end_matches('/').to_string();
    let laya_up = http
        .get(format!("{laya_url}/health"))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success());
    let cdp_url = config.browser_cdp_url.clone().filter(|u| !u.trim().is_empty());
    let cdp_up = match &cdp_url {
        // Chrome only answers its HTTP endpoints for an IP or localhost Host header
        Some(url) => http
            .get(format!("{}/json/version", url.trim_end_matches('/')))
            .header(reqwest::header::HOST, "localhost")
            .send()
            .await
            .is_ok_and(|r| r.status().is_success()),
        None => false,
    };
    let docker = crate::sandbox::SandboxManager::new().is_docker_available().await;
    let chrome = crate::sandbox::computer::local_chrome_path().map(|p| p.display().to_string());

    let mut clis = Vec::new();
    for (engine, key) in [(Engine::Claude, "ANTHROPIC_API_KEY"), (Engine::Codex, "OPENAI_API_KEY"), (Engine::Grok, "XAI_API_KEY")] {
        let bin = crate::drivers::find_cli_binary(engine.as_str());
        let signed_in = if bin.is_some() {
            crate::profiles::status::probe(engine, None).await.signed_in
        } else {
            None
        };
        let via_key = std::env::var(key).is_ok_and(|v| !v.trim().is_empty());
        clis.push((engine, bin, signed_in, via_key));
    }

    let laya_installed = config.laya_managed
        && crate::router::sidecar::local_port(&laya_url).is_some()
        && (crate::setup::laya_serve_bin(&config.data_dir).is_file()
            || std::env::var("KWNT_LAYA_SERVE_BIN").is_ok_and(|b| !b.is_empty()));
    let https_origin = std::env::var("KWNT_ALLOWED_ORIGINS")
        .ok()
        .and_then(|v| v.split(',').map(str::trim).find(|o| o.starts_with("https://")).map(String::from));
    Facts {
        data_dir: config.data_dir.display().to_string(),
        data_dir_writable: writable(&config.data_dir),
        laya_url,
        laya_managed: config.laya_managed,
        laya_up,
        laya_installed,
        cdp_url,
        cdp_up,
        docker,
        chrome,
        clis,
        // In a container kwnt binds 0.0.0.0 and the published port decides; judge by token+HTTPS
        loopback_bind: Platform::detect() != Platform::Docker && crate::server::auth::is_loopback_bind(&config.host),
        token: crate::server::auth::env_token().is_some() || crate::server::auth::read_existing(&config.data_dir).is_ok(),
        https_origin,
    }
}

fn writable(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let probe = dir.join(format!(".kwnt-doctor-{}", std::process::id()));
    std::fs::write(&probe, b"ok").map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

/// The report `kwnt doctor` prints.
pub fn render(checks: &[Check], p: Platform) -> String {
    let mut s = format!("kwant doctor · {}\n\n", p.label());
    for c in checks {
        let mark = if c.optional { "opt " } else { c.status.mark() };
        s.push_str(&format!("  {}  {:<16} {}\n", mark, c.name, c.detail));
        if let Some(fix) = &c.fix {
            s.push_str(&format!("        fix: {fix}\n"));
        }
    }
    let fails = checks.iter().filter(|c| c.status == Status::Fail).count();
    let warns = checks.iter().filter(|c| c.status == Status::Warn && !c.optional).count();
    s.push('\n');
    let installable = checks.iter().any(|c| c.fix.as_deref().is_some_and(|f| f.starts_with("kwnt setup")) && !c.optional);
    if installable {
        s.push_str("Install the missing parts in one go (asks before each step): kwnt setup\n");
    }
    s.push_str(&match (fails, warns) {
        (0, 0) => "Everything kwant needs is here.\n".to_string(),
        (0, w) => format!("kwant can run. {w} optional part(s) missing or not set up; see the fixes above.\n"),
        (f, _) => format!("{f} required part(s) missing; kwant can't work fully until they're fixed.\n"),
    });
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy() -> Facts {
        Facts {
            data_dir: "/data".into(),
            data_dir_writable: Ok(()),
            laya_url: "http://laya:8000".into(),
            laya_managed: false,
            laya_up: true,
            laya_installed: false,
            cdp_url: Some("http://browser:9222".into()),
            cdp_up: true,
            docker: false,
            chrome: None,
            clis: vec![
                (Engine::Claude, Some("/opt/clis/bin/claude".into()), Some(false), false),
                (Engine::Codex, None, None, false),
                (Engine::Grok, None, None, false),
            ],
            loopback_bind: false,
            token: true,
            https_origin: Some("https://kwant.example".into()),
        }
    }

    fn by_name<'a>(cs: &'a [Check], name: &str) -> &'a Check {
        cs.iter().find(|c| c.name == name).unwrap()
    }

    #[test]
    fn platform_detection() {
        assert_eq!(Platform::from_parts("macos", false), Platform::Mac);
        assert_eq!(Platform::from_parts("linux", false), Platform::Linux);
        assert_eq!(Platform::from_parts("macos", true), Platform::Docker);
        assert_eq!(Platform::from_parts("linux", true), Platform::Docker);
    }

    #[test]
    fn compose_stack_is_green_except_unsigned_clis() {
        let cs = checks(&healthy(), Platform::Docker);
        assert!(cs.iter().all(|c| c.status != Status::Fail));
        for name in ["Data folder", "Laya router", "Browser (VM)", "Access"] {
            assert_eq!(by_name(&cs, name).status, Status::Ok, "{name}");
        }
        assert_eq!(by_name(&cs, "Browser (local)").status, Status::Skip);
        let claude = by_name(&cs, "claude");
        assert_eq!(claude.status, Status::Warn);
        assert!(claude.fix.as_deref().unwrap().starts_with("docker compose exec kwnt claude auth login"));
        assert!(by_name(&cs, "codex").fix.as_deref().unwrap().contains("KWNT_CLIS="));
    }

    #[test]
    fn no_browser_at_all_fails_with_platform_fixes() {
        let mut f = healthy();
        f.cdp_url = None;
        let docker = checks(&f, Platform::Docker);
        assert_eq!(by_name(&docker, "Browser (VM)").status, Status::Fail);
        assert!(by_name(&docker, "Browser (VM)").fix.as_deref().unwrap().contains("docker compose up -d browser"));

        let linux = checks(&f, Platform::Linux);
        assert_eq!(by_name(&linux, "Browser (VM)").status, Status::Fail);
        assert_eq!(by_name(&linux, "Browser (local)").status, Status::Fail);
        let fix = by_name(&linux, "Browser (VM)").fix.clone().unwrap();
        assert!(fix.contains("get.docker.com") && !fix.contains("Docker Desktop"));
        assert!(by_name(&linux, "Browser (local)").fix.as_deref().unwrap().contains("apt-get install -y chromium"));

        let mac = checks(&f, Platform::Mac);
        assert!(by_name(&mac, "Browser (VM)").fix.as_deref().unwrap().contains("Docker Desktop"));
        assert!(by_name(&mac, "Browser (local)").fix.as_deref().unwrap().contains("brew install --cask google-chrome"));
    }

    #[test]
    fn one_working_browser_is_enough() {
        let mut f = healthy();
        f.cdp_url = None;
        f.chrome = Some("/usr/bin/chromium".into());
        let cs = checks(&f, Platform::Linux);
        assert_eq!(by_name(&cs, "Browser (local)").status, Status::Ok);
        assert_eq!(by_name(&cs, "Browser (VM)").status, Status::Warn);
        assert!(cs.iter().all(|c| c.status != Status::Fail));
    }

    #[test]
    fn laya_and_data_dir_fixes() {
        let mut f = healthy();
        f.laya_up = false;
        f.data_dir_writable = Err("Permission denied".into());
        let cs = checks(&f, Platform::Docker);
        assert_eq!(by_name(&cs, "Laya router").status, Status::Warn);
        assert!(by_name(&cs, "Laya router").fix.as_deref().unwrap().contains("docker compose up -d laya"));
        assert_eq!(by_name(&cs, "Data folder").status, Status::Fail);
        assert!(by_name(&cs, "Data folder").fix.as_deref().unwrap().contains("chown -R 10001:10001 /data"));
        f.laya_managed = true;
        assert!(laya_fix(Platform::Linux, true).starts_with("kwnt setup --only laya"));
        assert!(laya_fix(Platform::Mac, true).starts_with("kwnt setup --only laya"));
        // installed by kwnt setup, not running because kwnt serve isn't: fine locally
        f.laya_installed = true;
        assert_eq!(by_name(&checks(&f, Platform::Linux), "Laya router").status, Status::Ok);
    }

    #[test]
    fn api_key_or_sign_in_counts_as_ready() {
        let mut f = healthy();
        let cs = checks(&f, Platform::Docker);
        assert_eq!(by_name(&cs, "AI account").status, Status::Warn);
        f.clis[0].3 = true;
        let cs = checks(&f, Platform::Docker);
        assert_eq!(by_name(&cs, "claude").status, Status::Ok);
        assert!(cs.iter().all(|c| c.name != "AI account"));
    }

    #[test]
    fn access_without_https_warns() {
        let mut f = healthy();
        f.https_origin = None;
        let cs = checks(&f, Platform::Linux);
        assert_eq!(by_name(&cs, "Access").status, Status::Warn);
        f.loopback_bind = true;
        assert_eq!(by_name(&checks(&f, Platform::Linux), "Access").status, Status::Ok);
    }

    #[test]
    fn render_summarises() {
        let out = render(&checks(&healthy(), Platform::Docker), Platform::Docker);
        assert!(out.starts_with("kwant doctor · Docker container"));
        assert!(out.contains("kwant can run."));
    }

    #[test]
    fn local_machine_after_kwnt_setup_has_only_sign_in_warnings() {
        // kwnt setup --yes on a Linux desktop: Laya installed, a Chromium, Claude Code; no Docker
        let f = Facts {
            data_dir: "/home/u/.kwnt".into(),
            data_dir_writable: Ok(()),
            laya_url: "http://127.0.0.1:8000".into(),
            laya_managed: true,
            laya_up: false,
            laya_installed: true,
            cdp_url: None,
            cdp_up: false,
            docker: false,
            chrome: Some("/home/u/.cache/ms-playwright/chromium-1200/chrome-linux-arm64/chrome".into()),
            clis: vec![
                (Engine::Claude, Some("/home/u/.local/bin/claude".into()), Some(false), false),
                (Engine::Codex, None, None, false),
                (Engine::Grok, None, None, false),
            ],
            loopback_bind: true,
            token: false,
            https_origin: None,
        };
        for p in [Platform::Linux, Platform::Mac] {
            let cs = checks(&f, p);
            assert!(cs.iter().all(|c| c.status != Status::Fail));
            let counted: Vec<&str> =
                cs.iter().filter(|c| c.status == Status::Warn && !c.optional).map(|c| c.name.as_str()).collect();
            assert_eq!(counted, vec!["claude", "AI account"], "{p:?}");
            for name in ["Browser (VM)", "kwant shell", "codex", "grok"] {
                assert!(by_name(&cs, name).optional, "{name}");
            }
            let out = render(&cs, p);
            assert!(out.contains("opt   Browser (VM)"));
            assert!(!out.contains("Install the missing parts"), "{out}");
        }

        // a bare machine is pointed at kwnt setup
        let mut bare = f.clone();
        bare.laya_installed = false;
        bare.chrome = None;
        bare.clis = vec![(Engine::Claude, None, None, false), (Engine::Codex, None, None, false), (Engine::Grok, None, None, false)];
        let cs = checks(&bare, Platform::Linux);
        assert_eq!(by_name(&cs, "Browser (local)").status, Status::Fail);
        assert!(!by_name(&cs, "claude").optional);
        assert!(render(&cs, Platform::Linux).contains("Install the missing parts in one go (asks before each step): kwnt setup"));
    }
}
