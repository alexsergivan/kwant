//! `kwnt setup` and the app's Setup checklist: installs what kwant needs on a Mac or a Linux
//! machine. Everything it runs itself goes into the person's own folders (Laya in the kwant data
//! folder, CLIs in `~/.local` or their installers' own folders, a private Chromium in the
//! Playwright cache). Anything that needs admin rights (apt, dnf, Docker on Linux) is only shown,
//! as a command for the person to run. A part that is already there is left alone, so running
//! setup again changes nothing.
//!
//! [`probe`] looks at the machine, [`items`] says what is missing, [`plan`] says how to install
//! one part here, and [`run_plan`] runs it, streaming each output line.

use crate::doctor::Platform;
use crate::profiles::Engine;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

/// uv, which fetches a Python for Laya when the machine has no usable one. Pinned.
pub const UV_VERSION: &str = "0.13.5";
/// Playwright, whose `install chromium` puts a private Chromium in the person's cache. Pinned.
pub const PLAYWRIGHT_VERSION: &str = "1.64.1";
pub const CODEX_NPM: &str = "@openai/codex";

/// A part `kwnt setup` can install.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemId {
    Laya,
    Browser,
    Claude,
    Codex,
    Grok,
    Docker,
}

impl ItemId {
    pub const ALL: [ItemId; 6] = [ItemId::Laya, ItemId::Browser, ItemId::Claude, ItemId::Codex, ItemId::Grok, ItemId::Docker];

    pub fn as_str(self) -> &'static str {
        match self {
            ItemId::Laya => "laya",
            ItemId::Browser => "browser",
            ItemId::Claude => "claude",
            ItemId::Codex => "codex",
            ItemId::Grok => "grok",
            ItemId::Docker => "docker",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.as_str() == s.trim().to_ascii_lowercase())
    }

    pub fn label(self) -> &'static str {
        match self {
            ItemId::Laya => "Laya router",
            ItemId::Browser => "Browser (Chrome or Chromium)",
            ItemId::Claude => "Claude Code CLI",
            ItemId::Codex => "Codex CLI",
            ItemId::Grok => "Grok CLI",
            ItemId::Docker => "Docker (VM mode)",
        }
    }

    pub fn engine(self) -> Option<Engine> {
        match self {
            ItemId::Claude => Some(Engine::Claude),
            ItemId::Codex => Some(Engine::Codex),
            ItemId::Grok => Some(Engine::Grok),
            _ => None,
        }
    }

    const CLIS: [ItemId; 3] = [ItemId::Claude, ItemId::Codex, ItemId::Grok];
}

/// What [`probe`] found on this machine.
#[derive(Clone, Debug)]
pub struct Probe {
    pub platform: Platform,
    pub home: PathBuf,
    pub data_dir: PathBuf,
    pub root: bool,
    /// `kwnt serve` runs Laya here (managed, at a loopback URL).
    pub laya_local: bool,
    pub laya_installed: bool,
    /// The routing model was downloaded and answered once (see [`laya_model_marker`]).
    pub laya_model_ready: bool,
    pub laya_up: bool,
    pub laya_models: String,
    pub uv: Option<PathBuf>,
    /// A Python 3 that can make a venv (on a Mac, not Apple's /usr/bin/python3 stub).
    pub python: Option<PathBuf>,
    pub chrome: Option<PathBuf>,
    /// Linux won't let a Chromium in the home folder start its sandbox, so only a packaged
    /// browser (which needs sudo) works.
    pub chrome_sandbox_blocked: bool,
    /// A browser service (`KWNT_BROWSER_CDP_URL`) is set, so local Chrome is optional.
    pub cdp: bool,
    pub npm: Option<PathBuf>,
    pub npx: Option<PathBuf>,
    /// npm's global prefix is writable, so `npm install -g` works without sudo.
    pub npm_global_writable: bool,
    pub brew: Option<PathBuf>,
    pub curl: bool,
    /// Linux package manager for the commands shown to the person: `apt`, `dnf` or `None`.
    pub linux_pm: Option<&'static str>,
    pub ubuntu: bool,
    pub clis: Vec<(Engine, Option<String>)>,
    pub docker: bool,
}

impl Probe {
    fn cli(&self, engine: Engine) -> Option<&str> {
        self.clis.iter().find(|(e, _)| *e == engine).and_then(|(_, b)| b.as_deref())
    }

    fn any_cli(&self) -> bool {
        self.clis.iter().any(|(_, b)| b.is_some())
    }

    fn sudo(&self) -> &'static str {
        if self.root { "" } else { "sudo " }
    }

    /// Where setup puts uv when it installs it: `<data>/bin/uv`.
    pub fn own_uv(&self) -> PathBuf {
        self.data_dir.join("bin").join("uv")
    }
}

/// `<data>/laya`, where the managed Laya lives.
pub fn laya_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("laya")
}

pub fn laya_serve_bin(data_dir: &Path) -> PathBuf {
    laya_dir(data_dir).join("venv").join("bin").join("laya-serve")
}

/// Written after the routing model downloaded and answered; holds the model list.
pub fn laya_model_marker(data_dir: &Path) -> PathBuf {
    laya_dir(data_dir).join("model-ready")
}

/// `uv` on PATH, else the one setup installs in `<data>/bin`, else uv's own install folders.
pub fn find_uv(data_dir: &Path, home: Option<&Path>) -> Option<PathBuf> {
    which("uv")
        .into_iter()
        .chain(std::iter::once(data_dir.join("bin").join("uv")))
        .chain(home.into_iter().flat_map(|h| [h.join(".local/bin/uv"), h.join(".cargo/bin/uv")]))
        .find(|p| p.is_file())
}

/// A Python 3 that can make a venv. On a Mac `/usr/bin/python3` is Apple's stub, which asks to
/// install the Xcode tools in a dialog, so only a real Python (Homebrew, python.org) counts there.
pub fn find_python(platform: Platform) -> Option<PathBuf> {
    let py = which("python3")?;
    if platform == Platform::Mac && py == Path::new("/usr/bin/python3") {
        return None;
    }
    let ok = std::process::Command::new(&py)
        .args(["-c", "import sys, venv, ensurepip; sys.exit(0 if sys.version_info >= (3, 10) else 1)"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    ok.then_some(py)
}

/// One command, run as the current user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Cmd {
    pub label: String,
    pub argv: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<(String, String)>,
}

impl Cmd {
    fn new(label: impl Into<String>, argv: &[&str]) -> Self {
        Self { label: label.into(), argv: argv.iter().map(|s| s.to_string()).collect(), env: Vec::new() }
    }

    fn sh(label: impl Into<String>, script: &str) -> Self {
        Self::new(label, &["sh", "-c", script])
    }

    fn env(mut self, k: &str, v: impl Into<String>) -> Self {
        self.env.push((k.to_string(), v.into()));
        self
    }

    /// The command as the person would type it.
    pub fn display(&self) -> String {
        let env: String = self.env.iter().map(|(k, v)| format!("{k}={} ", shell_quote(v))).collect();
        match self.argv.as_slice() {
            [sh, c, script] if sh == "sh" && c == "-c" => format!("{env}{script}"),
            argv => format!("{env}{}", argv.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ")),
        }
    }
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@,+".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// One step of a [`Plan`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Step {
    /// Runs as the current user, never with sudo.
    Run(Cmd),
    /// Starts Laya once on a free port so it downloads its model, asks it one question, stops it.
    LayaModel { label: String },
    /// Opens the found Chrome without a window to check it starts.
    CheckChrome { label: String },
    /// Needs admin rights or a download page: shown to the person, never run.
    Manual { label: String, command: String },
}

impl Step {
    pub fn label(&self) -> &str {
        match self {
            Step::Run(c) => &c.label,
            Step::LayaModel { label } | Step::CheckChrome { label } | Step::Manual { label, .. } => label,
        }
    }

    pub fn display(&self) -> String {
        match self {
            Step::Run(c) => c.display(),
            Step::LayaModel { .. } => "(kwant starts Laya once to download and test its routing model)".into(),
            Step::CheckChrome { .. } => "(kwant opens the browser once without a window to check it starts)".into(),
            Step::Manual { command, .. } => command.clone(),
        }
    }
}

/// How to install one part here. `steps` empty: nothing to do.
#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    pub item: ItemId,
    pub steps: Vec<Step>,
}

impl Plan {
    /// Steps kwant runs itself.
    pub fn automatic(&self) -> bool {
        self.steps.iter().any(|s| !matches!(s, Step::Manual { .. }))
    }

    pub fn manual(&self) -> impl Iterator<Item = &Step> {
        self.steps.iter().filter(|s| matches!(s, Step::Manual { .. }))
    }
}

/// Commands that put `laya-serve` into `<data>/laya/venv`: with uv when there is one (it brings
/// its own Python), else with a usable Python 3, else by installing uv into `<data>/bin` first.
/// Also used by the sidecar, so `kwnt serve` and the Mac app install Laya the same way.
/// PyTorch's CPU-only wheels, as in `docker/laya.Dockerfile`: on Linux the default wheels bring
/// CUDA (about 6 GB more), which Laya's small router model doesn't need.
pub const TORCH_CPU_INDEX: &str = "https://download.pytorch.org/whl/cpu";

/// `cpu_torch`: install CPU-only PyTorch first (Linux; Mac wheels have no CUDA).
pub fn laya_commands(data_dir: &Path, uv: Option<&Path>, python: Option<&Path>, cpu_torch: bool) -> Vec<Cmd> {
    let venv = laya_dir(data_dir).join("venv");
    let venv_s = venv.display().to_string();
    let venv_py = venv.join("bin").join("python").display().to_string();
    let pkg = crate::router::sidecar::LAYA_PACKAGE;
    let with_uv = |uv: &str| {
        let mut cmds = vec![Cmd::new("Make Laya's Python environment", &[uv, "venv", "--python", "3.12", &venv_s])];
        if cpu_torch {
            cmds.push(Cmd::new(
                "Install CPU-only PyTorch",
                &[uv, "pip", "install", "--python", &venv_py, "--index-url", TORCH_CPU_INDEX, "torch"],
            ));
        }
        cmds.push(Cmd::new("Install Laya (one-time, downloads PyTorch)", &[uv, "pip", "install", "--python", &venv_py, pkg]));
        cmds
    };
    match (uv, python) {
        (Some(uv), _) => with_uv(&uv.display().to_string()),
        (None, Some(py)) => {
            let py = py.display().to_string();
            let mut cmds = vec![
                Cmd::new("Make Laya's Python environment", &[&py, "-m", "venv", &venv_s]),
                Cmd::new("Update pip", &[&venv_py, "-m", "pip", "install", "--upgrade", "pip"]),
            ];
            if cpu_torch {
                cmds.push(Cmd::new(
                    "Install CPU-only PyTorch",
                    &[&venv_py, "-m", "pip", "install", "--index-url", TORCH_CPU_INDEX, "torch"],
                ));
            }
            cmds.push(Cmd::new("Install Laya (one-time, downloads PyTorch)", &[&venv_py, "-m", "pip", "install", pkg]));
            cmds
        }
        (None, None) => {
            let bin = data_dir.join("bin");
            let own = bin.join("uv").display().to_string();
            let mut cmds = vec![Cmd::sh(
                format!("Install uv {UV_VERSION} (it brings Python) into kwant's data folder"),
                &format!("curl -LsSf https://astral.sh/uv/{UV_VERSION}/install.sh | sh"),
            )
            .env("UV_INSTALL_DIR", bin.display().to_string())
            .env("UV_NO_MODIFY_PATH", "1")];
            cmds.extend(with_uv(&own));
            cmds
        }
    }
}

/// How to install `item` on this machine, given what [`probe`] found. Pure, so every platform's
/// plan is unit-tested here.
pub fn plan(item: ItemId, p: &Probe) -> Plan {
    let mut steps = Vec::new();
    let sudo = p.sudo();
    match item {
        ItemId::Laya if p.platform == Platform::Docker || !p.laya_local => {}
        ItemId::Laya => {
            if !p.laya_installed {
                if p.uv.is_none() && p.python.is_none() && !p.curl {
                    steps.push(Step::Manual {
                        label: "Install uv (needs curl, which isn't here)".into(),
                        command: "https://docs.astral.sh/uv/getting-started/installation/".into(),
                    });
                    return Plan { item, steps };
                }
                steps.extend(laya_commands(&p.data_dir, p.uv.as_deref(), p.python.as_deref(), p.platform == Platform::Linux).into_iter().map(Step::Run));
            }
            if !p.laya_model_ready && !p.laya_up {
                steps.push(Step::LayaModel { label: "Download and test Laya's routing model (~420 MB)".into() });
            }
        }
        ItemId::Browser if p.chrome.is_some() || p.platform == Platform::Docker => {}
        ItemId::Browser => match p.platform {
            Platform::Mac if p.brew.is_some() => {
                steps.push(Step::Run(Cmd::new("Install Google Chrome with Homebrew", &["brew", "install", "--cask", "google-chrome"])));
                steps.push(Step::CheckChrome { label: "Check Chrome starts".into() });
            }
            _ if p.npx.is_some() && !p.chrome_sandbox_blocked => {
                steps.push(Step::Run(Cmd::new(
                    "Download a private Chromium into your cache folder (no admin rights)",
                    &["npx", "--yes", &format!("playwright@{PLAYWRIGHT_VERSION}"), "install", "--no-shell", "chromium"],
                )));
                steps.push(Step::CheckChrome { label: "Check Chromium starts".into() });
            }
            Platform::Mac => steps.push(Step::Manual {
                label: "Install Google Chrome".into(),
                command: "Download it from https://www.google.com/chrome/ (or install Homebrew from https://brew.sh, then: brew install --cask google-chrome)".into(),
            }),
            _ => steps.push(Step::Manual {
                label: if p.chrome_sandbox_blocked {
                    "Install Chromium (needs admin rights: this Linux blocks the sandbox of a browser in your home folder)".into()
                } else {
                    "Install Chromium (needs admin rights)".into()
                },
                command: match (p.linux_pm, p.ubuntu) {
                    (Some("apt"), true) => format!("{sudo}snap install chromium"),
                    (Some("apt"), false) => format!("{sudo}apt-get install -y chromium"),
                    (Some("dnf"), _) => format!("{sudo}dnf install -y chromium"),
                    _ => "Install Chromium or Google Chrome with your package manager, or Node.js so kwnt setup can fetch a private Chromium".into(),
                },
            }),
        },
        ItemId::Claude | ItemId::Grok if p.platform == Platform::Docker => {}
        ItemId::Claude | ItemId::Grok if p.cli(item.engine().unwrap_or(Engine::Claude)).is_some() => {}
        ItemId::Claude | ItemId::Grok => {
            let (label, url) = match item {
                ItemId::Claude => ("Install Claude Code into ~/.local/bin", "https://claude.ai/install.sh"),
                _ => ("Install the Grok CLI into ~/.grok", "https://x.ai/cli/install.sh"),
            };
            steps.push(if p.curl {
                Step::Run(Cmd::sh(label, &format!("curl -fsSL {url} | bash")))
            } else {
                Step::Manual { label: format!("{label} (needs curl)"), command: format!("curl -fsSL {url} | bash") }
            });
        }
        ItemId::Codex if p.platform == Platform::Docker || p.cli(Engine::Codex).is_some() => {}
        ItemId::Codex => match (p.platform, &p.npm, &p.brew) {
            (_, Some(_), _) if p.npm_global_writable => {
                steps.push(Step::Run(Cmd::new("Install the Codex CLI with npm", &["npm", "install", "-g", CODEX_NPM])))
            }
            (_, Some(_), _) => {
                let prefix = p.home.join(".local").display().to_string();
                steps.push(Step::Run(Cmd::new(
                    "Install the Codex CLI into ~/.local (npm's global folder needs admin rights)",
                    &["npm", "install", "-g", "--prefix", &prefix, CODEX_NPM],
                )))
            }
            (Platform::Mac, None, Some(_)) => {
                steps.push(Step::Run(Cmd::new("Install the Codex CLI with Homebrew", &["brew", "install", "--cask", "codex"])))
            }
            (Platform::Mac, None, None) => steps.push(Step::Manual {
                label: "Install Node.js, then the Codex CLI".into(),
                command: format!("Install Node.js from https://nodejs.org, then: npm install -g {CODEX_NPM}"),
            }),
            _ => steps.push(Step::Manual {
                label: "Install Node.js (needs admin rights), then the Codex CLI".into(),
                command: format!(
                    "{}, then: npm install -g --prefix ~/.local {CODEX_NPM}",
                    match p.linux_pm {
                        Some("dnf") => format!("{sudo}dnf install -y nodejs npm"),
                        _ => format!("{sudo}apt-get install -y nodejs npm"),
                    }
                ),
            }),
        },
        ItemId::Docker if p.docker || p.platform == Platform::Docker => {}
        ItemId::Docker => match (p.platform, &p.brew) {
            (Platform::Mac, Some(_)) => {
                steps.push(Step::Run(Cmd::new("Install Docker Desktop with Homebrew", &["brew", "install", "--cask", "docker"])));
                steps.push(Step::Manual { label: "Start Docker Desktop once".into(), command: "open -a Docker".into() });
            }
            (Platform::Mac, None) => steps.push(Step::Manual {
                label: "Install Docker Desktop".into(),
                command: "Download it from https://www.docker.com/products/docker-desktop/ and open it once".into(),
            }),
            _ => steps.push(Step::Manual {
                label: "Install Docker (needs admin rights)".into(),
                command: format!("curl -fsSL https://get.docker.com | {sudo}sh && {sudo}usermod -aG docker $USER   (then log in again)"),
            }),
        },
    }
    Plan { item, steps }
}

/// How much kwant needs a part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Need {
    /// kwant can't do its job without it (a browser).
    Required,
    /// kwant runs without it, but worse (Laya; an AI CLI while none is installed).
    Recommended,
    /// Extra (more CLIs, Docker for VM mode).
    Optional,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    Ready,
    Missing,
    /// Inside Docker: the compose stack provides it (`detail` says how).
    Managed,
}

/// One row of the Setup checklist.
#[derive(Clone, Debug, Serialize)]
pub struct Item {
    pub id: ItemId,
    pub label: &'static str,
    pub need: Need,
    pub state: ItemState,
    pub detail: String,
    /// kwant can install it itself (an Install button).
    pub can_install: bool,
    /// What Install would do, step by step, as commands.
    pub steps: Vec<Step>,
    /// CLIs: signed in (`None`: not installed or unknown).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signed_in: Option<bool>,
    /// CLIs: the sign-in command, once installed and not signed in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sign_in: Option<String>,
}

/// The checklist: every part, whether it is here, and how it would be installed.
/// `signed_in` per CLI comes from the caller (it needs a probe of the CLI).
pub fn items(p: &Probe, signed_in: &[(Engine, Option<bool>)]) -> Vec<Item> {
    ItemId::ALL
        .into_iter()
        .map(|id| {
            let plan = plan(id, p);
            let need = match id {
                ItemId::Browser => Need::Required,
                ItemId::Laya => Need::Recommended,
                ItemId::Docker => Need::Optional,
                _ if !p.any_cli() => Need::Recommended,
                _ => Need::Optional,
            };
            let (state, detail) = match id {
                _ if p.platform == Platform::Docker => (ItemState::Managed, docker_detail(id).to_string()),
                ItemId::Laya if p.laya_up => (ItemState::Ready, "running".to_string()),
                ItemId::Laya if !p.laya_local => (ItemState::Missing, "kwant doesn't run Laya here (--no-laya, KWNT_LAYA_MANAGED=0, or a LAYA_URL on another machine); start Laya there, or kwnt serve without --no-laya".to_string()),
                ItemId::Laya if p.laya_installed && p.laya_model_ready => {
                    (ItemState::Ready, "installed; kwant starts it with the server".to_string())
                }
                ItemId::Laya if p.laya_installed => (
                    ItemState::Ready,
                    "installed; the routing model downloads on first start (~420 MB)".to_string(),
                ),
                ItemId::Laya => (ItemState::Missing, "not installed; bots are picked by keywords until it is".to_string()),
                ItemId::Browser => match (&p.chrome, p.cdp) {
                    (Some(c), _) => (ItemState::Ready, c.display().to_string()),
                    (None, true) => (ItemState::Ready, "a browser service is set (KWNT_BROWSER_CDP_URL)".to_string()),
                    (None, false) => (ItemState::Missing, "no Chrome or Chromium found".to_string()),
                },
                ItemId::Docker if p.docker => (ItemState::Ready, "running".to_string()),
                ItemId::Docker => (ItemState::Missing, "only needed for VM mode".to_string()),
                _ => match p.cli(id.engine().unwrap_or(Engine::Claude)) {
                    Some(bin) => (ItemState::Ready, bin.to_string()),
                    None => (ItemState::Missing, "not installed".to_string()),
                },
            };
            // A browser service makes local Chrome optional
            let need = if id == ItemId::Browser && p.cdp { Need::Optional } else { need };
            let signed = id
                .engine()
                .and_then(|e| signed_in.iter().find(|(x, _)| *x == e).and_then(|(_, s)| *s))
                .filter(|_| state == ItemState::Ready);
            let sign_in = id
                .engine()
                .filter(|_| state == ItemState::Ready && signed != Some(true))
                .map(|e| crate::doctor::cli_sign_in_fix(e, p.platform));
            Item {
                id,
                label: id.label(),
                need,
                state,
                detail,
                can_install: state == ItemState::Missing && plan.automatic(),
                steps: if state == ItemState::Missing { plan.steps } else { Vec::new() },
                signed_in: signed,
                sign_in,
            }
        })
        .collect()
}

fn docker_detail(id: ItemId) -> &'static str {
    match id {
        ItemId::Laya => "the compose laya service: docker compose up -d laya",
        ItemId::Browser => "the compose browser service: docker compose up -d browser",
        ItemId::Docker => "not used inside the container",
        _ => "build the image with KWNT_CLIS=claude,codex,grok",
    }
}

/// What `kwnt setup` installs when the person doesn't pick: every missing required or recommended
/// part, plus `clis` (or Claude Code when no CLI is installed and none was named), plus Docker
/// only when asked.
pub fn default_selection(p: &Probe, clis: Option<&[ItemId]>, docker: bool) -> Vec<ItemId> {
    let mut out = vec![ItemId::Laya, ItemId::Browser];
    match clis {
        Some(list) => out.extend(list.iter().copied().filter(|i| ItemId::CLIS.contains(i))),
        None if !p.any_cli() => out.push(ItemId::Claude),
        None => {}
    }
    if docker {
        out.push(ItemId::Docker);
    }
    out
}

/// Looks at this machine (a few short process calls and one HTTP check).
pub async fn probe(config: &crate::config::Config) -> Probe {
    let platform = Platform::detect();
    let home = dirs::home_dir().unwrap_or_default();
    let data_dir = config.data_dir.clone();
    let laya_local = config.laya_managed && crate::router::sidecar::local_port(&config.laya_url).is_some();
    let laya_installed = std::env::var("KWNT_LAYA_SERVE_BIN").is_ok_and(|b| !b.is_empty() && Path::new(&b).is_file())
        || laya_serve_bin(&data_dir).is_file();
    let laya_model_ready = std::fs::read_to_string(laya_model_marker(&data_dir))
        .is_ok_and(|m| m.trim() == config.laya_models.trim());
    let laya_up = crate::router::LayaRouter::new(config.laya_url.clone()).is_reachable().await;
    let npm = which("npm");
    let npm_global_writable = match &npm {
        Some(npm) => npm_prefix_writable(npm),
        None => false,
    };
    let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let mut clis = Vec::new();
    for e in [Engine::Claude, Engine::Codex, Engine::Grok] {
        clis.push((e, crate::drivers::find_cli_binary(e.as_str())));
    }
    Probe {
        platform,
        root: is_root(),
        laya_local,
        laya_installed,
        laya_model_ready,
        laya_up,
        laya_models: config.laya_models.clone(),
        uv: find_uv(&data_dir, Some(&home)),
        python: find_python(platform),
        chrome: crate::sandbox::computer::local_chrome_path(),
        chrome_sandbox_blocked: crate::sandbox::computer::chrome_sandbox_blocked(),
        cdp: config.browser_cdp_url.as_deref().is_some_and(|u| !u.trim().is_empty()),
        npx: which("npx"),
        npm,
        npm_global_writable,
        brew: which("brew").or_else(|| {
            ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"].into_iter().map(PathBuf::from).find(|p| p.is_file())
        }),
        curl: which("curl").is_some(),
        linux_pm: if which("apt-get").is_some() {
            Some("apt")
        } else if which("dnf").is_some() {
            Some("dnf")
        } else {
            None
        },
        ubuntu: os_release.lines().any(|l| l.trim() == "ID=ubuntu"),
        clis,
        docker: crate::sandbox::SandboxManager::new().is_docker_available().await,
        home,
        data_dir,
    }
}

fn npm_prefix_writable(npm: &Path) -> bool {
    let Ok(out) = std::process::Command::new(npm).args(["prefix", "-g", "--logs-max=0"]).stderr(Stdio::null()).output() else {
        return false;
    };
    let prefix = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let target = if cfg!(windows) { prefix.clone() } else { prefix.join("lib").join("node_modules") };
    let dir = if target.is_dir() { target } else { prefix };
    let probe = dir.join(format!(".kwnt-setup-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(probe);
    ok
}

#[cfg(unix)]
fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(unix))]
fn is_root() -> bool {
    false
}

pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|c| c.is_file())
}

/// Runs a plan's automatic steps in order, sending every output line to `log`. Manual steps are
/// logged as "run this yourself" and skipped. Stops at the first failing step.
pub async fn run_plan(plan: &Plan, config: &crate::config::Config, log: &(dyn Fn(String) + Send + Sync)) -> Result<(), String> {
    for step in &plan.steps {
        match step {
            Step::Manual { label, command } => {
                log(format!("· {label}: run this yourself: {command}"));
            }
            Step::Run(cmd) => {
                log(format!("▸ {}", cmd.label));
                log(format!("$ {}", cmd.display()));
                run_cmd(cmd, log).await.map_err(|e| format!("{}: {e}", cmd.label))?;
            }
            Step::LayaModel { label } => {
                log(format!("▸ {label}"));
                laya_model(config, log).await.map_err(|e| format!("{label}: {e}"))?;
            }
            Step::CheckChrome { label } => {
                log(format!("▸ {label}"));
                check_chrome(log).await.map_err(|e| format!("{label}: {e}"))?;
            }
        }
    }
    Ok(())
}

async fn run_cmd(cmd: &Cmd, log: &(dyn Fn(String) + Send + Sync)) -> Result<(), String> {
    let mut child = tokio::process::Command::new(&cmd.argv[0])
        .args(&cmd.argv[1..])
        .envs(cmd.env.iter().map(|(k, v)| (k, v)))
        // Never wait for a prompt: npm, brew and the installers all read stdin when they can
        .stdin(Stdio::null())
        .env("CI", "1")
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("couldn't start {}: {e}", cmd.argv[0]))?;
    let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        return Err("no output pipes".into());
    };
    let last = std::sync::Mutex::new(Vec::<String>::new());
    let forward = |line: String| {
        let line = clean_line(&line);
        if line.is_empty() {
            return;
        }
        if let Ok(mut l) = last.lock() {
            l.push(line.clone());
            if l.len() > 5 {
                l.remove(0);
            }
        }
        log(format!("  {line}"));
    };
    let mut out = BufReader::new(out).lines();
    let mut err = BufReader::new(err).lines();
    let (mut out_done, mut err_done) = (false, false);
    while !(out_done && err_done) {
        tokio::select! {
            l = out.next_line(), if !out_done => match l {
                Ok(Some(l)) => forward(l),
                _ => out_done = true,
            },
            l = err.next_line(), if !err_done => match l {
                Ok(Some(l)) => forward(l),
                _ => err_done = true,
            },
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        let tail = last.lock().map(|l| l.last().cloned().unwrap_or_default()).unwrap_or_default();
        Err(format!("exited with {status}{}", if tail.is_empty() { String::new() } else { format!(" ({tail})") }))
    }
}

/// Drops ANSI colour codes and carriage-return progress redraws, and caps the length.
fn clean_line(line: &str) -> String {
    let line = line.rsplit('\r').find(|s| !s.trim().is_empty()).unwrap_or("");
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if !c.is_control() {
            out.push(c);
        }
    }
    let out = out.trim_end().to_string();
    if out.len() > 400 {
        format!("{}…", &out[..out.floor_char_boundary(400)])
    } else {
        out
    }
}

/// Starts the installed laya-serve on a free port so it downloads its model, checks it answers a
/// routing question, stops it and writes the marker.
async fn laya_model(config: &crate::config::Config, log: &(dyn Fn(String) + Send + Sync)) -> Result<(), String> {
    let data_dir = &config.data_dir;
    let bin = std::env::var("KWNT_LAYA_SERVE_BIN")
        .ok()
        .filter(|b| !b.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| laya_serve_bin(data_dir));
    if !bin.is_file() {
        return Err(format!("{} is missing", bin.display()));
    }
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| e.to_string())?;
    let dir = laya_dir(data_dir);
    let log_path = dir.join("laya.log");
    let offset = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
    let mut child = crate::router::sidecar::start(&bin, &dir, port, &config.laya_models)?;
    let router = crate::router::LayaRouter::new(format!("http://127.0.0.1:{port}"));
    let started = std::time::Instant::now();
    let limit = Duration::from_secs(30 * 60);
    let mut next_note = Duration::from_secs(30);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            let why = crate::router::sidecar::last_error_line_from(&log_path, offset).unwrap_or_default();
            return Err(format!("laya-serve exited ({status}) {why}"));
        }
        if router.is_reachable().await {
            break;
        }
        if started.elapsed() > limit {
            let _ = child.kill().await;
            return Err("Laya didn't answer within 30 minutes".into());
        }
        if started.elapsed() >= next_note {
            log(format!("  still loading ({}s)", started.elapsed().as_secs()));
            next_note += Duration::from_secs(30);
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    log("  Laya is up; asking it one routing question".into());
    let answer = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/v1/systemone"))
        .json(&serde_json::json!({
            "state": {"prompt": "hello, can you help me plan a trip?"},
            "questions": {"setup": {"type": "choice", "instructions": "Who should answer?",
                "criteria": {"a": "travel and planning", "b": "software engineering"}}}
        }))
        .timeout(Duration::from_secs(600))
        .send()
        .await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    match answer {
        Ok(r) if r.status().is_success() => {
            std::fs::write(laya_model_marker(data_dir), &config.laya_models).map_err(|e| e.to_string())?;
            log(format!("  Laya answered in {}s overall; model ready", started.elapsed().as_secs()));
            Ok(())
        }
        Ok(r) => Err(format!("Laya answered {}", r.status())),
        Err(e) => Err(format!("Laya didn't answer: {e}")),
    }
}

/// Opens the found Chrome without a window on a blank page.
async fn check_chrome(log: &(dyn Fn(String) + Send + Sync)) -> Result<(), String> {
    let chrome = crate::sandbox::computer::local_chrome_path().ok_or("no Chrome or Chromium found after installing")?;
    log(format!("  found {}", chrome.display()));
    let profile = std::env::temp_dir().join(format!("kwnt-chrome-check-{}", std::process::id()));
    let out = tokio::time::timeout(
        Duration::from_secs(60),
        tokio::process::Command::new(&chrome)
            .args([
                "--headless=new",
                "--disable-gpu",
                "--no-first-run",
                &format!("--user-data-dir={}", profile.display()),
                "--dump-dom",
                "about:blank",
            ])
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await;
    let _ = std::fs::remove_dir_all(&profile);
    let out = out.map_err(|_| "it didn't finish within 60 s".to_string())?.map_err(|e| e.to_string())?;
    if out.status.success() && String::from_utf8_lossy(&out.stdout).contains("<html") {
        log("  it starts and loads a page".into());
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().rev().find(|l| l.contains("error") || l.contains("while loading")).unwrap_or("").trim();
        Err(format!(
            "it didn't start ({}{}). Missing system libraries? Run: sudo npx playwright@{PLAYWRIGHT_VERSION} install-deps chromium",
            out.status,
            if line.is_empty() { String::new() } else { format!(": {line}") }
        ))
    }
}

/// `kwnt setup` options.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Install without asking.
    pub yes: bool,
    /// Only these parts (default: [`default_selection`]).
    pub only: Option<Vec<ItemId>>,
    /// Which AI CLIs to install.
    pub clis: Option<Vec<ItemId>>,
    /// Also Docker, for VM mode.
    pub docker: bool,
    /// Show what would run; run nothing.
    pub dry_run: bool,
}

/// Parses a comma list of part names; the error names the bad one.
pub fn parse_list(list: &str) -> Result<Vec<ItemId>, String> {
    list.split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| ItemId::parse(s).ok_or_else(|| format!("unknown part '{}' (use laya, browser, claude, codex, grok, docker)", s.trim())))
        .collect()
}

/// `kwnt setup`: installs what is missing, asking before each part unless `--yes`, then prints
/// `kwnt doctor`. Returns the exit code.
pub async fn run_cli(config: &crate::config::Config, opts: Options) -> i32 {
    use std::io::{BufRead, IsTerminal, Write};
    let p = probe(config).await;
    if p.platform == Platform::Docker {
        println!("kwant runs in a Docker container: its parts come from the compose stack and build args.");
        println!("See what's missing and how to add it: kwnt doctor");
        return 0;
    }
    let selection = opts.only.clone().unwrap_or_else(|| default_selection(&p, opts.clis.as_deref(), opts.docker));
    println!("kwant setup · {}{}\n", p.platform.label(), if opts.dry_run { " · dry run, nothing is installed" } else { "" });

    let ask = !opts.yes && !opts.dry_run;
    let tty = std::io::stdin().is_terminal();
    let mut manual: Vec<(ItemId, String, String)> = Vec::new();
    let mut failed: Vec<(ItemId, String)> = Vec::new();
    let mut skipped = Vec::new();
    let mut installed = Vec::new();
    for item in selection {
        let plan = plan(item, &p);
        if plan.steps.is_empty() {
            println!("✓ {}: already here", item.label());
            continue;
        }
        for s in plan.manual() {
            if let Step::Manual { label, command } = s {
                manual.push((item, label.clone(), command.clone()));
            }
        }
        if !plan.automatic() {
            println!("· {}: needs a step only you can do (listed at the end)", item.label());
            continue;
        }
        println!("▸ {}: kwant will run", item.label());
        for s in plan.steps.iter().filter(|s| !matches!(s, Step::Manual { .. })) {
            println!("    {}", s.display());
        }
        if opts.dry_run {
            continue;
        }
        if ask {
            if !tty {
                println!("  skipped: no terminal to ask in; run `kwnt setup --yes` to install without asking");
                skipped.push(item);
                continue;
            }
            print!("  Install {}? [Y/n] ", item.label());
            let _ = std::io::stdout().flush();
            let mut answer = String::new();
            let _ = std::io::stdin().lock().read_line(&mut answer);
            if matches!(answer.trim().to_ascii_lowercase().as_str(), "n" | "no") {
                println!("  skipped");
                skipped.push(item);
                continue;
            }
        }
        let log = |line: String| println!("  {line}");
        match run_plan(&Plan { item, steps: plan.steps.into_iter().filter(|s| !matches!(s, Step::Manual { .. })).collect() }, config, &log).await {
            Ok(()) => {
                println!("✓ {}: installed\n", item.label());
                installed.push(item);
            }
            Err(e) => {
                println!("✗ {}: {e}\n", item.label());
                failed.push((item, e));
            }
        }
    }

    if !manual.is_empty() {
        println!("\nRun these yourself (they need admin rights or a download), then run kwnt setup again:");
        for (item, label, command) in &manual {
            println!("  {} · {label}:\n    {command}", item.label());
        }
    }
    if !skipped.is_empty() {
        println!("\nSkipped: {}", skipped.iter().map(|i| i.label()).collect::<Vec<_>>().join(", "));
    }
    if opts.dry_run {
        return 0;
    }
    crate::drivers::forget_cli_binaries();
    let platform = Platform::detect();
    let checks = crate::doctor::checks(&crate::doctor::gather(config).await, platform);
    println!("\n{}", crate::doctor::render(&checks, platform));
    for (item, _) in &failed {
        println!("{} wasn't installed; see the error above.", item.label());
    }
    if failed.is_empty() && checks.iter().all(|c| c.status != crate::doctor::Status::Fail) {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bare(platform: Platform) -> Probe {
        Probe {
            platform,
            home: PathBuf::from("/home/u"),
            data_dir: PathBuf::from("/home/u/.kwnt"),
            root: false,
            laya_local: true,
            laya_installed: false,
            laya_model_ready: false,
            laya_up: false,
            laya_models: "english".into(),
            uv: None,
            python: None,
            chrome: None,
            chrome_sandbox_blocked: false,
            cdp: false,
            npm: None,
            npx: None,
            npm_global_writable: false,
            brew: None,
            curl: true,
            linux_pm: Some("apt"),
            ubuntu: false,
            clis: vec![(Engine::Claude, None), (Engine::Codex, None), (Engine::Grok, None)],
            docker: false,
        }
    }

    fn mac() -> Probe {
        Probe { linux_pm: None, home: "/Users/u".into(), data_dir: "/Users/u/.kwnt".into(), ..bare(Platform::Mac) }
    }

    fn shown(plan: &Plan) -> Vec<String> {
        plan.steps.iter().map(Step::display).collect()
    }

    fn never_sudo(plan: &Plan) {
        for s in &plan.steps {
            if let Step::Run(c) = s {
                assert!(!c.argv.iter().any(|a| a.contains("sudo")), "runs sudo: {}", c.display());
            }
        }
    }

    #[test]
    fn laya_on_a_bare_machine_installs_uv_into_the_data_folder_then_laya() {
        for p in [bare(Platform::Linux), mac()] {
            let plan = plan(ItemId::Laya, &p);
            never_sudo(&plan);
            let s = shown(&plan);
            assert!(s[0].contains(&format!("astral.sh/uv/{UV_VERSION}/install.sh")), "{s:?}");
            assert!(s[0].contains(&format!("UV_INSTALL_DIR={}/bin", p.data_dir.display())));
            assert!(s[0].contains("UV_NO_MODIFY_PATH=1"));
            assert!(s[1].starts_with(&format!("{}/bin/uv venv --python 3.12 {}/laya/venv", p.data_dir.display(), p.data_dir.display())));
            // Linux gets CPU-only PyTorch first, as the Docker image does
            let linux = p.platform == Platform::Linux;
            if linux {
                assert_eq!(s[2], format!("{}/bin/uv pip install --python {}/laya/venv/bin/python --index-url {TORCH_CPU_INDEX} torch", p.data_dir.display(), p.data_dir.display()));
            }
            let laya = if linux { 3 } else { 2 };
            assert!(s[laya].contains("pip install") && s[laya].contains("laya[serve]"), "{s:?}");
            assert!(matches!(plan.steps[laya + 1], Step::LayaModel { .. }));
            assert_eq!(plan.steps.len(), laya + 2);
        }
    }

    #[test]
    fn laya_prefers_uv_then_a_real_python() {
        let mut p = bare(Platform::Linux);
        p.python = Some("/usr/bin/python3".into());
        let s = shown(&plan(ItemId::Laya, &p));
        assert!(s[0].starts_with("/usr/bin/python3 -m venv /home/u/.kwnt/laya/venv"), "{s:?}");
        assert!(s[2].contains("pip install --index-url https://download.pytorch.org/whl/cpu torch"), "{s:?}");
        p.uv = Some("/home/u/.local/bin/uv".into());
        let s = shown(&plan(ItemId::Laya, &p));
        assert!(s[0].starts_with("/home/u/.local/bin/uv venv"), "{s:?}");
    }

    #[test]
    fn laya_installed_only_fetches_the_model_and_ready_does_nothing() {
        let mut p = bare(Platform::Linux);
        p.laya_installed = true;
        let plan1 = plan(ItemId::Laya, &p);
        assert_eq!(plan1.steps.len(), 1);
        assert!(matches!(plan1.steps[0], Step::LayaModel { .. }));
        p.laya_model_ready = true;
        assert!(plan(ItemId::Laya, &p).steps.is_empty());
        // Laya run elsewhere (Docker, a remote LAYA_URL) is never installed here
        let mut remote = bare(Platform::Linux);
        remote.laya_local = false;
        assert!(plan(ItemId::Laya, &remote).steps.is_empty());
    }

    #[test]
    fn mac_apple_python_stub_is_not_used() {
        // find_python's rule; the plan then installs uv instead (see the bare-machine test)
        assert_eq!(Platform::Mac, mac().platform);
        if cfg!(target_os = "macos") && which("python3").as_deref() == Some(Path::new("/usr/bin/python3")) {
            assert!(find_python(Platform::Mac).is_none());
        }
    }

    #[test]
    fn mac_browser_uses_homebrew_else_playwright_else_the_download_page() {
        let mut p = mac();
        let s = shown(&plan(ItemId::Browser, &p));
        assert_eq!(s.len(), 1);
        assert!(s[0].contains("https://www.google.com/chrome/"));
        assert!(!plan(ItemId::Browser, &p).automatic());

        p.npx = Some("/usr/local/bin/npx".into());
        let pl = plan(ItemId::Browser, &p);
        assert!(shown(&pl)[0].starts_with(&format!("npx --yes playwright@{PLAYWRIGHT_VERSION} install --no-shell chromium")));
        assert!(matches!(pl.steps[1], Step::CheckChrome { .. }));

        p.brew = Some("/opt/homebrew/bin/brew".into());
        let pl = plan(ItemId::Browser, &p);
        assert_eq!(shown(&pl)[0], "brew install --cask google-chrome");
        never_sudo(&pl);
    }

    #[test]
    fn linux_browser_without_node_shows_the_sudo_command_and_runs_nothing() {
        let mut p = bare(Platform::Linux);
        let pl = plan(ItemId::Browser, &p);
        assert!(!pl.automatic());
        assert_eq!(shown(&pl), vec!["sudo apt-get install -y chromium"]);
        p.ubuntu = true;
        assert_eq!(shown(&plan(ItemId::Browser, &p)), vec!["sudo snap install chromium"]);
        p.linux_pm = Some("dnf");
        p.ubuntu = false;
        assert_eq!(shown(&plan(ItemId::Browser, &p)), vec!["sudo dnf install -y chromium"]);
        p.root = true;
        assert_eq!(shown(&plan(ItemId::Browser, &p)), vec!["dnf install -y chromium"]);

        p.npx = Some("/usr/bin/npx".into());
        let pl = plan(ItemId::Browser, &p);
        assert!(pl.automatic());
        never_sudo(&pl);
        // Ubuntu 23.10+: a Playwright Chromium can't start its sandbox, so only the package works
        p.chrome_sandbox_blocked = true;
        p.root = false;
        p.ubuntu = true;
        p.linux_pm = Some("apt");
        let pl = plan(ItemId::Browser, &p);
        assert!(!pl.automatic());
        assert_eq!(shown(&pl), vec!["sudo snap install chromium"]);
        p.chrome = Some("/usr/bin/chromium".into());
        assert!(plan(ItemId::Browser, &p).steps.is_empty());
    }

    #[test]
    fn clis_install_in_user_space() {
        let p = bare(Platform::Linux);
        assert_eq!(shown(&plan(ItemId::Claude, &p)), vec!["curl -fsSL https://claude.ai/install.sh | bash"]);
        assert_eq!(shown(&plan(ItemId::Grok, &p)), vec!["curl -fsSL https://x.ai/cli/install.sh | bash"]);

        // codex: no Node on Linux → the person installs Node (sudo), nothing runs
        let pl = plan(ItemId::Codex, &p);
        assert!(!pl.automatic());
        assert!(shown(&pl)[0].starts_with("sudo apt-get install -y nodejs npm, then: npm install -g --prefix ~/.local @openai/codex"));

        let mut p = bare(Platform::Linux);
        p.npm = Some("/usr/bin/npm".into());
        assert_eq!(shown(&plan(ItemId::Codex, &p)), vec!["npm install -g --prefix /home/u/.local @openai/codex"]);
        p.npm_global_writable = true;
        assert_eq!(shown(&plan(ItemId::Codex, &p)), vec!["npm install -g @openai/codex"]);

        let mut m = mac();
        m.brew = Some("/opt/homebrew/bin/brew".into());
        assert_eq!(shown(&plan(ItemId::Codex, &m)), vec!["brew install --cask codex"]);
        m.brew = None;
        assert!(shown(&plan(ItemId::Codex, &m))[0].contains("https://nodejs.org"));

        let mut installed = bare(Platform::Linux);
        installed.clis[0].1 = Some("/home/u/.local/bin/claude".into());
        assert!(plan(ItemId::Claude, &installed).steps.is_empty());
    }

    #[test]
    fn docker_is_never_run_with_sudo_and_only_on_request() {
        let p = bare(Platform::Linux);
        let pl = plan(ItemId::Docker, &p);
        assert!(!pl.automatic());
        assert!(shown(&pl)[0].contains("get.docker.com | sudo sh"));
        let mut m = mac();
        assert!(!plan(ItemId::Docker, &m).automatic());
        m.brew = Some("/opt/homebrew/bin/brew".into());
        assert_eq!(shown(&plan(ItemId::Docker, &m))[0], "brew install --cask docker");

        assert!(!default_selection(&p, None, false).contains(&ItemId::Docker));
        assert!(default_selection(&p, None, true).contains(&ItemId::Docker));
    }

    #[test]
    fn default_selection_adds_claude_only_when_no_cli_is_installed() {
        let p = bare(Platform::Linux);
        assert_eq!(default_selection(&p, None, false), vec![ItemId::Laya, ItemId::Browser, ItemId::Claude]);
        assert_eq!(
            default_selection(&p, Some(&[ItemId::Codex, ItemId::Laya]), false),
            vec![ItemId::Laya, ItemId::Browser, ItemId::Codex]
        );
        let mut has = bare(Platform::Linux);
        has.clis[2].1 = Some("/home/u/.grok/bin/grok".into());
        assert_eq!(default_selection(&has, None, false), vec![ItemId::Laya, ItemId::Browser]);
    }

    #[test]
    fn checklist_rows() {
        let mut p = bare(Platform::Linux);
        p.npx = Some("/usr/bin/npx".into());
        let rows = items(&p, &[]);
        let row = |id| rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(row(ItemId::Browser).need, Need::Required);
        assert_eq!(row(ItemId::Browser).state, ItemState::Missing);
        assert!(row(ItemId::Browser).can_install);
        assert_eq!(row(ItemId::Laya).need, Need::Recommended);
        assert_eq!(row(ItemId::Claude).need, Need::Recommended);
        assert!(!row(ItemId::Codex).can_install, "codex needs Node via sudo here");
        assert_eq!(row(ItemId::Docker).need, Need::Optional);
        assert!(!row(ItemId::Docker).can_install);

        p.clis[0].1 = Some("/home/u/.local/bin/claude".into());
        p.chrome = Some("/usr/bin/chromium".into());
        p.laya_installed = true;
        p.laya_model_ready = true;
        let rows = items(&p, &[(Engine::Claude, Some(false))]);
        let row = |id| rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(row(ItemId::Claude).state, ItemState::Ready);
        assert_eq!(row(ItemId::Claude).signed_in, Some(false));
        assert!(row(ItemId::Claude).sign_in.as_deref().unwrap().starts_with("claude auth login"));
        assert!(row(ItemId::Claude).steps.is_empty());
        assert_eq!(row(ItemId::Codex).need, Need::Optional);
        assert_eq!(row(ItemId::Laya).state, ItemState::Ready);
        assert_eq!(row(ItemId::Browser).state, ItemState::Ready);

        let rows = items(&bare(Platform::Docker), &[]);
        assert!(rows.iter().all(|r| r.state == ItemState::Managed && !r.can_install));
    }

    #[test]
    fn display_quotes_and_cleans() {
        let c = Cmd::new("x", &["npm", "install", "a b"]).env("K", "v w");
        assert_eq!(c.display(), "K='v w' npm install 'a b'");
        assert_eq!(clean_line("\u{1b}[32mok\u{1b}[0m"), "ok");
        assert_eq!(clean_line("10%\r50%\r100%"), "100%");
        assert_eq!(ItemId::parse(" Laya "), Some(ItemId::Laya));
        assert_eq!(ItemId::parse("chrome"), None);
    }
}
