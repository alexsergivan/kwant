//! Test plan #10 (F-1 … F-6, F-11, D1): with "Run Claude inside the VM" on, every way the
//! isolated turn can fail is the turn's typed notice. A host `claude` (a shim that leaves a
//! marker) never starts, and the mock upstream sees nothing.
//!
//! One test function on purpose: the "Docker missing" case empties `PATH` for a moment, which
//! no other test in this binary may see.
#![cfg(target_os = "linux")]

#[path = "common/exe.rs"]
mod exe;

use axum::Router;
use kwnt::{
    db::models::Kwant,
    drivers::{cli::Isolation, CliDriver, Driver, TurnInput, TurnOutput},
    sandbox::agent_vm::{broker::BrokerConfig, image_tag, AgentVm, AgentsInVmConfig},
};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::{tempdir, TempDir};

fn turn(folder: Option<&Path>) -> TurnInput {
    TurnInput {
        continuity: String::new(),
        skills: String::new(),
        person: Default::default(),
        store: None,
        browser_api_url: None,
        thread_id: "t1".into(),
        kwant: Kwant {
            space_id: kwnt::db::spaces::DEFAULT_SPACE_ID.into(),
            id: "fail-closed".into(),
            name: "Ada".into(),
            title: "Lead".into(),
            persona_prompt: "p".into(),
            soul: String::new(),
            avatar_sprite: "s".into(),
            driver_type: "cli".into(),
            preferred_model: "claude".into(),
            temperature: 0.7,
            max_tokens: 1000,
            level: 1,
            turns_completed: 0,
            created_at: None,
        },
        prompt: "hi".into(),
        history: vec![],
        model_override: None,
        model: None,
        effort_level: Some("low".into()),
        team_context: None,
        workspace_dir: folder.map(Path::to_path_buf),
        sandbox_id: None,
        teammates: vec![],
        mcp: None,
        no_tools: false,
        read_only: false,
    }
}

/// A mock Anthropic that only counts what reaches it.
async fn counting_upstream() -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let app = Router::new().fallback(move || {
        h.fetch_add(1, Ordering::SeqCst);
        async { "{}" }
    });
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}"), hits)
}

struct Case {
    shim: TempDir,
    upstream_hits: Arc<AtomicUsize>,
}

impl Case {
    fn marker(&self) -> std::path::PathBuf {
        self.shim.path().join("marker")
    }

    async fn run(&self, vm: &Arc<AgentVm>, profile_home: Option<&Path>, folder: Option<&Path>) -> TurnOutput {
        let bin = self.shim.path().join("claude").to_string_lossy().into_owned();
        let driver = CliDriver::new(Some(bin), Some("claude".into())).with_isolation(Some(Isolation {
            vm: vm.clone(),
            profile_home: profile_home.map(Path::to_path_buf),
            kwnt_origin: "http://127.0.0.1:9".into(),
        }));
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let out = driver.stream_turn(turn(folder), tx).await.expect("a refusal is a notice, not an error");
        drain.await.unwrap();
        out
    }

    /// The turn ended with `turn_isolation_failed` and `reason`, nothing ran on this computer and
    /// nothing reached the model.
    fn refused(&self, out: &TurnOutput, reason: &str, case: &str) {
        let (text, meta) = out.notice.as_ref().unwrap_or_else(|| panic!("{case}: a notice, not a reply"));
        assert!(!text.is_empty(), "{case}");
        assert_eq!(meta["reason"], reason, "{case}: {meta}");
        assert!(!self.marker().exists(), "{case}: host claude started");
        assert_eq!(self.upstream_hits.load(Ordering::SeqCst), 0, "{case}: the upstream was reached");
    }
}

fn sign_in(home: &Path, body: &str) {
    std::fs::create_dir_all(home).unwrap();
    std::fs::write(home.join(".credentials.json"), body).unwrap();
}

fn docker_available() -> bool {
    std::process::Command::new("docker").arg("version").output().is_ok_and(|o| o.status.success())
}

fn image_present() -> bool {
    std::process::Command::new("docker").args(["image", "inspect", &image_tag()]).output().is_ok_and(|o| o.status.success())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_failure_is_a_notice_and_nothing_runs_on_this_computer() {
    let shim = tempdir().unwrap();
    exe::write_executable(shim.path().join("claude"), format!("#!/bin/sh\necho started >> {}/marker\n", shim.path().display()));
    let (origin, hits) = counting_upstream().await;
    let case = Case { shim, upstream_hits: hits };
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let work = tmp.path().join("projects").join("site");
    std::fs::create_dir_all(&work).unwrap();
    let mut cfg = BrokerConfig::new(data.join("broker.log"));
    cfg.anthropic_origin = origin.clone();
    cfg.oauth_token_url = format!("{origin}/v1/oauth/token");
    let vm = AgentVm::with_parts(&data, cfg.clone(), Some(image_tag()), AgentsInVmConfig::default(), None);

    let good = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-REALCANARY","refreshToken":"r","expiresAt":99999999999999}}"#;
    let profile = tmp.path().join("profile");
    sign_in(&profile, good);
    let docker = docker_available();
    // Without Docker every later check is never reached: the turn says Docker isn't running
    let or_docker = |code: &'static str| if docker { code } else { "docker_unavailable" };

    // F-5: no sign-in in the profile home
    let empty = tmp.path().join("empty-profile");
    std::fs::create_dir_all(&empty).unwrap();
    case.refused(&case.run(&vm, Some(&empty), Some(&work)).await, "no_credential", "no sign-in");

    // F-1: Docker missing (no `docker` on PATH)
    let path = std::env::var_os("PATH");
    std::env::set_var("PATH", tmp.path().join("no-bin"));
    let out = case.run(&vm, Some(&profile), Some(&work)).await;
    match &path {
        Some(p) => std::env::set_var("PATH", p),
        None => std::env::remove_var("PATH"),
    }
    case.refused(&out, "docker_unavailable", "docker missing");

    // F-5: a sign-in file that isn't JSON, or has no token
    for (i, bad) in ["{not json", r#"{"claudeAiOauth":{}}"#].iter().enumerate() {
        let home = tmp.path().join(format!("bad-profile-{i}"));
        sign_in(&home, bad);
        case.refused(&case.run(&vm, Some(&home), Some(&work)).await, or_docker("no_credential"), "malformed sign-in");
    }

    // S-1: the folder is the sign-in home itself, or kwnt's data folder
    case.refused(&case.run(&vm, Some(&profile), Some(&profile)).await, or_docker("unsafe_folder"), "sign-in home as folder");
    case.refused(&case.run(&vm, Some(&profile), Some(&data)).await, or_docker("unsafe_folder"), "data dir as folder");

    // F-3: the image isn't there (a tag nobody built): `preparing`, then `download_failed`
    let missing = AgentVm::with_parts(&data, cfg.clone(), Some(format!("kwnt-agent:missing-{}", uuid::Uuid::new_v4().simple())), AgentsInVmConfig::default(), None);
    case.refused(&case.run(&missing, Some(&profile), Some(&work)).await, or_docker("preparing"), "image missing");
    if docker {
        // The background setup of a test image fails at once; the next reply says so
        for _ in 0..100 {
            let out = case.run(&missing, Some(&profile), Some(&work)).await;
            let reason = out.notice.as_ref().map(|(_, m)| m["reason"].clone());
            if reason.as_ref().and_then(|r| r.as_str()) != Some("preparing") {
                case.refused(&out, "download_failed", "image setup failed");
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    // F-2: kwnt's broker can't open its socket (the VM's own folder is a file)
    if docker && image_present() {
        let blocked = tmp.path().join("blocked-data");
        std::fs::create_dir_all(&blocked).unwrap();
        std::fs::write(blocked.join("agent-vm"), "not a folder").unwrap();
        let vm = AgentVm::with_parts(&blocked, cfg.clone(), Some(image_tag()), AgentsInVmConfig::default(), None);
        case.refused(&case.run(&vm, Some(&profile), Some(&work)).await, "relay_down", "broker down");
        vm.remove_containers(|_| true).await;
    } else {
        eprintln!("broker-down case skipped: needs Docker and {}", image_tag());
    }
    // A missing folder is never swapped for one on this computer: the shim still never ran
    assert!(!case.marker().exists());
}
