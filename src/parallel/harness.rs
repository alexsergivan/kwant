//! Runs one deployment option in throwaway containers and measures it.
//!
//! For each branch: an `--internal` docker network (no route to the internet, the host or
//! production), the app container with CPU, memory and process limits and no published port,
//! an oha load test from a second container on that network, and a memory sample. Everything it
//! created is removed afterwards, whether the run succeeded, failed or timed out; every object
//! carries the label `kwnt.pu.run=<run id>`, so [`sweep`] can remove anything a crash left behind
//! without touching other containers.

use super::{AppSpec, Figure, Limits, LoadSpec};
use chrono::Utc;
use serde_json::Value;
use std::time::Duration;
use thiserror::Error;
use tokio::process::Command;
use tokio::time::{sleep, timeout, Instant};

/// The load tool, run as a container next to the app (pinned; `docker pull` it once).
pub const OHA_IMAGE: &str = "ghcr.io/hatoo/oha:1.16.0";
pub const RUN_LABEL: &str = "kwnt.pu.run";

#[derive(Debug, Error, PartialEq)]
pub enum HarnessError {
    #[error("Docker is not available on this machine")]
    DockerUnavailable,
    #[error("the load tool image is missing: run `docker pull {OHA_IMAGE}`")]
    LoadToolMissing,
    #[error("{0}")]
    Invalid(String),
    #[error("building the app failed: {0}")]
    Build(String),
    #[error("starting the app failed: {0}")]
    Start(String),
    #[error("the app did not answer on its port in time")]
    NotReady,
    #[error("the load test failed: {0}")]
    Load(String),
    #[error("the branch ran past its time limit")]
    Timeout,
}

/// What to run for one option.
#[derive(Debug, Clone)]
pub struct BranchSpec {
    pub run_id: String,
    pub branch: String,
    pub app: AppSpec,
    pub limits: Limits,
    pub load: LoadSpec,
}

/// What one branch produced. `cleanup_errors` is empty when everything it made is gone.
#[derive(Debug)]
pub struct BranchOutcome {
    pub result: Result<Vec<Figure>, HarnessError>,
    pub cleanup_errors: Vec<String>,
}

/// The docker objects one branch creates.
#[derive(Debug, Clone, PartialEq)]
pub struct Names {
    pub label: String,
    pub network: String,
    pub app: String,
    pub probe: String,
    pub load: String,
    pub image: String,
}

impl Names {
    pub fn new(run_id: &str, branch: &str) -> Self {
        let run: String = run_id.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect::<String>().to_lowercase();
        let prefix = format!("kwnt-pu-{run}-{branch}");
        Names {
            label: format!("{RUN_LABEL}={run_id}"),
            network: format!("{prefix}-net"),
            app: format!("{prefix}-app"),
            probe: format!("{prefix}-probe"),
            load: format!("{prefix}-load"),
            image: format!("{prefix}:run"),
        }
    }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

pub fn network_args(n: &Names) -> Vec<String> {
    s(&["network", "create", "--internal", "--label", &n.label, &n.network])
}

/// An offline build: the Dockerfile cannot download anything.
pub fn build_args(n: &Names, app: &AppSpec) -> Option<Vec<String>> {
    let context = app.context.as_deref()?;
    let dockerfile = std::path::Path::new(context).join(app.dockerfile.as_deref().unwrap_or("Dockerfile"));
    Some(s(&["build", "--network", "none", "--label", &n.label, "-t", &n.image, "-f", &dockerfile.to_string_lossy(), context]))
}

/// The app container: limited, on the internal network only, no published port.
pub fn app_args(n: &Names, app: &AppSpec, limits: &Limits) -> Vec<String> {
    let image = app.image.clone().unwrap_or_else(|| n.image.clone());
    let memory = format!("{}m", limits.memory_mb);
    s(&[
        "run", "-d", "--name", &n.app, "--network", &n.network, "--label", &n.label,
        "--cpus", &format!("{}", limits.cpus), "--memory", &memory, "--memory-swap", &memory,
        "--pids-limit", &limits.pids.to_string(),
        "--cap-drop", "ALL", "--cap-add", "CHOWN", "--cap-add", "SETUID", "--cap-add", "SETGID",
        "--cap-add", "NET_BIND_SERVICE", "--cap-add", "DAC_OVERRIDE", "--cap-add", "FOWNER",
        "--security-opt", "no-new-privileges", "--restart", "no",
        &image,
    ])
}

fn target(n: &Names, app: &AppSpec, path: &str) -> String {
    format!("http://{}:{}{}", n.app, app.port, path)
}

pub fn probe_args(n: &Names, app: &AppSpec) -> Vec<String> {
    s(&[
        "run", "--rm", "--name", &n.probe, "--network", &n.network, "--label", &n.label, "--cpus", "0.5", "--memory", "128m",
        OHA_IMAGE, "-n", "1", "-t", "2s", "--no-tui", "--output-format", "json", &target(n, app, &app.health_path),
    ])
}

pub fn load_args(n: &Names, app: &AppSpec, load: &LoadSpec) -> Vec<String> {
    s(&[
        "run", "--rm", "--name", &n.load, "--network", &n.network, "--label", &n.label, "--cpus", "1", "--memory", "256m",
        OHA_IMAGE, "-n", &load.requests.to_string(), "-c", &load.concurrency.to_string(), "-t", "10s",
        "--no-tui", "--output-format", "json", &target(n, app, &load.path),
    ])
}

struct Out {
    ok: bool,
    stdout: String,
    stderr: String,
}

async fn docker(args: &[String], limit: Duration) -> Result<Out, String> {
    let run = Command::new("docker").args(args).kill_on_drop(true).output();
    match timeout(limit, run).await {
        Err(_) => Err(format!("docker {} timed out", args.first().map(String::as_str).unwrap_or(""))),
        Ok(Err(e)) => Err(e.to_string()),
        Ok(Ok(o)) => Ok(Out {
            ok: o.status.success(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: crate::zombie::clip(String::from_utf8_lossy(&o.stderr).trim(), 2000),
        }),
    }
}

pub async fn docker_available() -> bool {
    matches!(docker(&s(&["version", "--format", "{{.Server.Version}}"]), Duration::from_secs(3)).await, Ok(o) if o.ok)
}

/// What oha printed, as figures. `None` when it is not oha's JSON.
pub fn parse_oha(raw: &str) -> Option<OhaSummary> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let lat = &v["metrics"]["latency_ms"];
    let codes = v["statusCodeDistribution"].as_object();
    let total: u64 = codes.map(|c| c.values().filter_map(Value::as_u64).sum()).unwrap_or(0);
    let ok: u64 = codes
        .map(|c| c.iter().filter(|(k, _)| k.starts_with('2') || k.starts_with('3')).filter_map(|(_, v)| v.as_u64()).sum())
        .unwrap_or(0);
    Some(OhaSummary {
        p50_ms: lat["p50"].as_f64()?,
        p95_ms: lat["p95"].as_f64()?,
        p99_ms: lat["p99"].as_f64()?,
        requests_per_sec: v["metrics"]["requests_per_sec"].as_f64()?,
        success_rate: v["metrics"]["success_rate"].as_f64()?,
        responses: total,
        ok_responses: ok,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct OhaSummary {
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub requests_per_sec: f64,
    /// Requests that got any answer (oha's own measure).
    pub success_rate: f64,
    pub responses: u64,
    /// 2xx and 3xx.
    pub ok_responses: u64,
}

/// `docker stats` memory ("12.5MiB / 512MiB") in MB.
pub fn parse_mem_mb(s: &str) -> Option<f64> {
    let used = s.split('/').next()?.trim();
    let split = used.find(|c: char| c.is_ascii_alphabetic())?;
    let (num, unit) = used.split_at(split);
    let n: f64 = num.trim().parse().ok()?;
    let factor = match unit {
        "B" => 1.0 / (1024.0 * 1024.0),
        "KiB" | "kB" | "KB" => 1.0 / 1024.0,
        "MiB" | "MB" => 1.0,
        "GiB" | "GB" => 1024.0,
        _ => return None,
    };
    Some((n * factor * 10.0).round() / 10.0)
}

fn command_line(args: &[String]) -> String {
    std::iter::once("docker").chain(args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ")
}

impl BranchSpec {
    pub fn validate(&self) -> Result<(), HarnessError> {
        if self.run_id.chars().filter(|c| c.is_ascii_alphanumeric()).count() < 8 || self.run_id.len() > 64 {
            return Err(HarnessError::Invalid("run id needs at least 8 letters or digits".into()));
        }
        if !super::is_slug(&self.branch, 32) {
            return Err(HarnessError::Invalid("branch must be a slug".into()));
        }
        self.app.validate().map_err(HarnessError::Invalid)?;
        self.limits.validate().map_err(HarnessError::Invalid)?;
        if let Some(c) = &self.app.context {
            if !std::path::Path::new(c).is_dir() {
                return Err(HarnessError::Invalid("context is not a directory".into()));
            }
        }
        Ok(())
    }
}

/// Builds, starts, loads and measures one option, then removes everything it made.
pub async fn run_branch(spec: &BranchSpec) -> BranchOutcome {
    if let Err(e) = spec.validate() {
        return BranchOutcome { result: Err(e), cleanup_errors: Vec::new() };
    }
    if !docker_available().await {
        return BranchOutcome { result: Err(HarnessError::DockerUnavailable), cleanup_errors: Vec::new() };
    }
    let names = Names::new(&spec.run_id, &spec.branch);
    let limit = Duration::from_secs(spec.limits.timeout_secs);
    let result = match timeout(limit, steps(spec, &names, Instant::now() + limit / 2)).await {
        Ok(r) => r,
        Err(_) => Err(HarnessError::Timeout),
    };
    let cleanup_errors = cleanup(&names, spec.app.context.is_some()).await;
    BranchOutcome { result, cleanup_errors }
}

async fn steps(spec: &BranchSpec, n: &Names, ready_by: Instant) -> Result<Vec<Figure>, HarnessError> {
    let step = Duration::from_secs(spec.limits.timeout_secs);
    let inspect = docker(&s(&["image", "inspect", "--format", "{{.Id}}", OHA_IMAGE]), Duration::from_secs(10)).await;
    if !matches!(inspect, Ok(ref o) if o.ok) {
        return Err(HarnessError::LoadToolMissing);
    }
    let o = docker(&network_args(n), Duration::from_secs(30)).await.map_err(HarnessError::Start)?;
    if !o.ok {
        return Err(HarnessError::Start(o.stderr));
    }
    if let Some(args) = build_args(n, &spec.app) {
        let o = docker(&args, step).await.map_err(HarnessError::Build)?;
        if !o.ok {
            return Err(HarnessError::Build(o.stderr));
        }
    }
    let o = docker(&app_args(n, &spec.app, &spec.limits), Duration::from_secs(60)).await.map_err(HarnessError::Start)?;
    if !o.ok {
        return Err(HarnessError::Start(o.stderr));
    }
    // ready once the health path answers with anything below 500
    loop {
        if let Ok(o) = docker(&probe_args(n, &spec.app), Duration::from_secs(20)).await {
            if parse_oha(&o.stdout).is_some_and(|p| p.responses > 0 && p.ok_responses == p.responses) {
                break;
            }
        }
        let running = docker(&s(&["inspect", "-f", "{{.State.Running}}", &n.app]), Duration::from_secs(10)).await;
        if !matches!(running, Ok(ref o) if o.stdout.trim() == "true") {
            return Err(HarnessError::Start("the app container exited".into()));
        }
        if Instant::now() >= ready_by {
            return Err(HarnessError::NotReady);
        }
        sleep(Duration::from_secs(1)).await;
    }
    let args = load_args(n, &spec.app, &spec.load);
    let o = docker(&args, step).await.map_err(HarnessError::Load)?;
    let at = Utc::now();
    let summary = parse_oha(&o.stdout).ok_or_else(|| HarnessError::Load(if o.stderr.is_empty() { "no result".into() } else { o.stderr.clone() }))?;
    let cmd = command_line(&args);
    let raw = o.stdout.trim();
    let mut figures = vec![
        Figure::verified("p50_latency", summary.p50_ms, "ms", &cmd, raw, at),
        Figure::verified("p95_latency", summary.p95_ms, "ms", &cmd, raw, at),
        Figure::verified("p99_latency", summary.p99_ms, "ms", &cmd, raw, at),
        Figure::verified("requests_per_sec", summary.requests_per_sec, "1/s", &cmd, raw, at),
        Figure::verified("success_rate", summary.success_rate, "ratio", &cmd, raw, at),
    ];
    if summary.responses > 0 {
        figures.push(Figure::verified("ok_share", summary.ok_responses as f64 / summary.responses as f64, "ratio", &cmd, raw, at));
    }
    let stats = s(&["stats", "--no-stream", "--format", "{{.MemUsage}}", &n.app]);
    if let Ok(o) = docker(&stats, Duration::from_secs(20)).await {
        if let Some(mb) = parse_mem_mb(o.stdout.trim()) {
            figures.push(Figure::verified("memory_after_load", mb, "MB", &command_line(&stats), o.stdout.trim(), Utc::now()));
        }
    }
    Ok(figures)
}

fn gone(stderr: &str) -> bool {
    let e = stderr.to_lowercase();
    e.contains("no such") || e.contains("not found")
}

/// Removes everything `n` names; what could not be removed comes back as messages.
pub async fn cleanup(n: &Names, built: bool) -> Vec<String> {
    let mut errors = Vec::new();
    let run = |args: Vec<String>| async move {
        match docker(&args, Duration::from_secs(60)).await {
            Ok(o) if o.ok || gone(&o.stderr) => None,
            Ok(o) => Some(format!("{}: {}", command_line(&args), o.stderr)),
            Err(e) => Some(format!("{}: {e}", command_line(&args))),
        }
    };
    for name in [&n.load, &n.probe, &n.app] {
        errors.extend(run(s(&["rm", "-f", name])).await);
    }
    errors.extend(run(s(&["network", "rm", &n.network])).await);
    if built {
        errors.extend(run(s(&["rmi", "-f", &n.image])).await);
    }
    errors
}

/// Removes every container, network and image with the run label (one run's, or all runs'
/// when `run_id` is `None`). Returns how many objects were removed.
pub async fn sweep(run_id: Option<&str>) -> Result<usize, String> {
    let filter = match run_id {
        Some(id) => format!("label={RUN_LABEL}={id}"),
        None => format!("label={RUN_LABEL}"),
    };
    let mut removed = 0;
    for (list, rm) in [
        (s(&["ps", "-aq", "--filter", &filter]), s(&["rm", "-f"])),
        (s(&["network", "ls", "-q", "--filter", &filter]), s(&["network", "rm"])),
        (s(&["images", "-q", "--filter", &filter]), s(&["rmi", "-f"])),
    ] {
        let o = docker(&list, Duration::from_secs(30)).await?;
        let ids: Vec<String> = o.stdout.split_whitespace().map(String::from).collect();
        if ids.is_empty() {
            continue;
        }
        let mut args = rm;
        args.extend(ids.iter().cloned());
        let o = docker(&args, Duration::from_secs(60)).await?;
        if !o.ok {
            return Err(o.stderr);
        }
        removed += ids.len();
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> AppSpec {
        AppSpec { image: Some("kwnt-executor:v1".into()), context: None, dockerfile: None, port: 8080, health_path: "/".into() }
    }

    #[test]
    fn names_are_scoped_to_the_run_and_branch() {
        let n = Names::new("3f2a9c1e-0000-4000", "vps");
        assert_eq!(n.app, "kwnt-pu-3f2a9c1e-vps-app");
        assert_eq!(n.network, "kwnt-pu-3f2a9c1e-vps-net");
        assert_eq!(n.label, "kwnt.pu.run=3f2a9c1e-0000-4000");
        assert!(!n.app.starts_with("kwnt-browser") && !n.app.starts_with("kwnt-executor"));
    }

    #[test]
    fn the_app_runs_limited_on_an_internal_network_with_no_published_port() {
        let n = Names::new("3f2a9c1e", "vps");
        let limits = Limits { cpus: 0.5, memory_mb: 256, pids: 64, timeout_secs: 60 };
        let a = app_args(&n, &app(), &limits);
        let pairs: Vec<(String, String)> = a.windows(2).map(|w| (w[0].clone(), w[1].clone())).collect();
        for (k, v) in [("--cpus", "0.5"), ("--memory", "256m"), ("--memory-swap", "256m"), ("--pids-limit", "64"), ("--network", n.network.as_str()), ("--cap-drop", "ALL")] {
            assert!(pairs.contains(&(k.into(), v.into())), "{k} {v} in {a:?}");
        }
        for args in [a, network_args(&n), probe_args(&n, &app()), load_args(&n, &app(), &LoadSpec::default())] {
            assert!(!args.iter().any(|x| x == "-p" || x.starts_with("--publish") || x == "-P" || x == "--network=host" || x == "host" || x == "--privileged"), "{args:?}");
            assert!(!args.iter().any(|x| x.contains("8799") || x.contains("8800")), "{args:?}");
        }
        assert!(network_args(&n).contains(&"--internal".to_string()));
    }

    #[test]
    fn builds_are_offline_and_labelled() {
        let n = Names::new("3f2a9c1e", "vps");
        let spec = AppSpec { image: None, context: Some("/tmp/ctx".into()), dockerfile: Some("deploy/Dockerfile".into()), port: 80, health_path: "/".into() };
        let b = build_args(&n, &spec).unwrap();
        assert_eq!(&b[..3], ["build", "--network", "none"]);
        assert!(b.contains(&"/tmp/ctx/deploy/Dockerfile".to_string()) && b.contains(&n.label));
        assert!(build_args(&n, &app()).is_none());
        assert_eq!(app_args(&n, &spec, &Limits::default()).last().unwrap(), &n.image);
    }

    #[test]
    fn oha_json_and_docker_stats_are_read() {
        let raw = r#"{"summary":{"successRate":1.0},"metrics":{"success_rate":1.0,"requests_per_sec":1045.1,
            "latency_ms":{"min":2.0,"mean":4.4,"p50":4.28,"p95":6.698,"p99":9.78,"max":9.78}},
            "statusCodeDistribution":{"200":45,"404":5}}"#;
        let o = parse_oha(raw).unwrap();
        assert_eq!((o.p95_ms, o.responses, o.ok_responses), (6.698, 50, 45));
        assert!(parse_oha("Error: connection refused").is_none());
        assert_eq!(parse_mem_mb("12.5MiB / 512MiB"), Some(12.5));
        assert_eq!(parse_mem_mb("1.5GiB / 2GiB"), Some(1536.0));
        assert_eq!(parse_mem_mb("800KiB / 64MiB"), Some(0.8));
        assert_eq!(parse_mem_mb("--"), None);
    }

    #[test]
    fn a_bad_spec_is_refused_before_docker_is_touched() {
        let mut spec = BranchSpec { run_id: "short".into(), branch: "vps".into(), app: app(), limits: Limits::default(), load: LoadSpec::default() };
        assert!(matches!(spec.validate(), Err(HarnessError::Invalid(_))));
        spec.run_id = "3f2a9c1e-aaaa".into();
        spec.branch = "VPS; rm -rf /".into();
        assert!(matches!(spec.validate(), Err(HarnessError::Invalid(_))));
        spec.branch = "vps".into();
        spec.app.image = Some("--privileged".into());
        assert!(matches!(spec.validate(), Err(HarnessError::Invalid(_))));
        spec.app.image = Some("kwnt-executor:v1".into());
        spec.limits.memory_mb = 8192;
        assert!(matches!(spec.validate(), Err(HarnessError::Invalid(_))));
        spec.limits.memory_mb = 256;
        assert_eq!(spec.validate(), Ok(()));
    }
}
