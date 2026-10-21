//! Runs a scenario: every option through the container harness, one after another, kept in
//! `pu_runs` as it goes, then compared ([`report`]).
//!
//! One run at a time server-wide ([`Runner`]). Options run sequentially, so at most one branch's
//! containers exist at once; the scenario's 3072 MB cap still holds if that ever changes. A run
//! copies its scenario when it starts, so editing or deleting the scenario later never changes
//! what a past run measured.
//!
//! When the server stops mid-run, the containers may outlive it. [`startup`] marks such runs
//! `interrupted` and sweeps their label; it only sweeps runs this database knows, so another kwnt
//! on the same Docker keeps its own containers.

use super::harness::{self, BranchOutcome, BranchSpec};
use super::prices::{self, PlanKind};
use super::{CriterionKey, Figure, Scenario, Tag};
use crate::db::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    /// Every option was tried (some may have failed).
    Finished,
    /// The server stopped before the run finished.
    Interrupted,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Finished => "finished",
            RunStatus::Interrupted => "interrupted",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchStatus {
    Pending,
    Running,
    Ok,
    Failed,
}

/// One option's part of a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Branch {
    pub key: String,
    pub label: String,
    pub status: BranchStatus,
    /// `HarnessError::code`, for the UI.
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    /// All `verified`.
    #[serde(default)]
    pub figures: Vec<Figure>,
    /// What the harness could not remove (empty when everything is gone).
    #[serde(default)]
    pub cleanup_errors: Vec<String>,
    /// Whether `docker build` had the network for this option (only ever the build step).
    #[serde(default)]
    pub build_network_used: bool,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub scenario_id: String,
    pub status: RunStatus,
    /// The scenario as it was when the run started.
    pub scenario: Scenario,
    pub branches: Vec<Branch>,
    /// Problems from the final label sweep.
    pub cleanup_errors: Vec<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// When a later start removed what this run left behind.
    pub swept_at: Option<DateTime<Utc>>,
}

impl Run {
    pub fn new(scenario: &Scenario) -> Self {
        Run {
            id: uuid::Uuid::new_v4().to_string(),
            scenario_id: scenario.id.clone(),
            status: RunStatus::Running,
            scenario: scenario.clone(),
            branches: scenario
                .options
                .iter()
                .map(|o| Branch {
                    key: o.key.clone(),
                    label: o.label.clone(),
                    status: BranchStatus::Pending,
                    error_code: None,
                    error: None,
                    figures: Vec::new(),
                    cleanup_errors: Vec::new(),
                    build_network_used: false,
                    started_at: None,
                    finished_at: None,
                })
                .collect(),
            cleanup_errors: Vec::new(),
            started_at: Utc::now(),
            finished_at: None,
            swept_at: None,
        }
    }

    /// Whether any option's build had the network.
    pub fn build_network_used(&self) -> bool {
        self.branches.iter().any(|b| b.build_network_used)
    }
}

pub type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What runs a branch and sweeps a run's label: Docker in the server, a fake in tests.
pub trait Harness: Send + Sync {
    fn run_branch(&self, spec: BranchSpec) -> BoxFut<BranchOutcome>;
    fn sweep(&self, run_id: String) -> BoxFut<Result<usize, String>>;
}

pub struct DockerHarness;

impl Harness for DockerHarness {
    fn run_branch(&self, spec: BranchSpec) -> BoxFut<BranchOutcome> {
        Box::pin(async move { harness::run_branch(&spec).await })
    }
    fn sweep(&self, run_id: String) -> BoxFut<Result<usize, String>> {
        Box::pin(async move {
            if !harness::docker_available().await {
                return Err("Docker is not available on this machine".into());
            }
            harness::sweep(Some(&run_id)).await
        })
    }
}

/// One run at a time, server-wide.
pub struct Runner {
    lock: Arc<Mutex<()>>,
    pub harness: Arc<dyn Harness>,
}

impl Default for Runner {
    fn default() -> Self {
        Runner::with_harness(Arc::new(DockerHarness))
    }
}

impl Runner {
    pub fn with_harness(harness: Arc<dyn Harness>) -> Self {
        Runner { lock: Default::default(), harness }
    }

    pub fn running(&self) -> bool {
        self.lock.try_lock().is_err()
    }

    /// The right to run, or `None` while another run holds it.
    pub fn try_begin(&self) -> Option<OwnedMutexGuard<()>> {
        self.lock.clone().try_lock_owned().ok()
    }
}

/// Tries every option of `run` in turn, saving after each step. `_guard` is held until the end.
pub async fn execute(db: Database, harness: Arc<dyn Harness>, mut run: Run, _guard: OwnedMutexGuard<()>) -> Result<Run, DbError> {
    let scenario = run.scenario.clone();
    for (i, option) in scenario.options.iter().enumerate() {
        run.branches[i].status = BranchStatus::Running;
        run.branches[i].started_at = Some(Utc::now());
        db.pu_put_run(&run).await?;
        let spec = BranchSpec {
            run_id: run.id.clone(),
            branch: option.key.clone(),
            app: option.app.clone(),
            limits: option.limits,
            load: scenario.load.clone(),
            build_network: scenario.build_network,
        };
        let outcome = harness.run_branch(spec).await;
        let b = &mut run.branches[i];
        b.build_network_used = scenario.build_network && option.app.context.is_some();
        b.cleanup_errors = outcome.cleanup_errors;
        b.finished_at = Some(Utc::now());
        match outcome.result {
            Ok(figures) => {
                b.status = BranchStatus::Ok;
                b.figures = figures;
            }
            Err(e) => {
                b.status = BranchStatus::Failed;
                b.error_code = Some(e.code().into());
                b.error = Some(e.to_string());
            }
        }
        db.pu_put_run(&run).await?;
    }
    // anything a branch could not remove, by label
    match harness.sweep(run.id.clone()).await {
        Ok(0) => {}
        Ok(n) => run.cleanup_errors.push(format!("the final sweep removed {n} leftover objects")),
        Err(e) => run.cleanup_errors.push(format!("the final sweep failed: {e}")),
    }
    run.status = RunStatus::Finished;
    run.finished_at = Some(Utc::now());
    db.pu_put_run(&run).await?;
    Ok(run)
}

/// At server start: marks runs a stopped server left `running` as interrupted, and removes what
/// they (or runs whose cleanup failed) left in Docker. Returns how many objects were removed.
pub async fn startup(db: &Database, harness: &dyn Harness) -> Result<usize, DbError> {
    let mut removed = 0;
    for mut run in db.pu_runs_to_sweep().await? {
        if run.status == RunStatus::Running {
            run.status = RunStatus::Interrupted;
            run.finished_at = Some(Utc::now());
            for b in &mut run.branches {
                if matches!(b.status, BranchStatus::Pending | BranchStatus::Running) {
                    b.status = BranchStatus::Failed;
                    b.error_code = Some("interrupted".into());
                    b.error = Some("the server stopped during the run".into());
                    b.finished_at = run.finished_at;
                }
            }
        }
        match harness.sweep(run.id.clone()).await {
            Ok(n) => {
                removed += n;
                run.swept_at = Some(Utc::now());
                if n > 0 {
                    run.cleanup_errors.push(format!("removed {n} leftover objects at startup"));
                }
            }
            Err(e) => tracing::info!("parallel: could not sweep run {}: {e}", run.id),
        }
        db.pu_put_run(&run).await?;
    }
    Ok(removed)
}

// ---- the comparison report ----

/// A sentence about one criterion, tagged like a figure (always `speculation` here).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub criterion: CriterionKey,
    pub text: String,
    #[serde(flatten)]
    pub tag: Tag,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptionReport {
    pub key: String,
    pub label: String,
    pub plan: Option<String>,
    pub status: BranchStatus,
    pub error_code: Option<String>,
    pub error: Option<String>,
    /// Measured on this server (`verified`).
    pub measured: Vec<Figure>,
    /// `estimate` from the price table, or `speculation` without a plan.
    pub cost: Figure,
    /// Ops burden and rollback: `speculation`.
    pub notes: Vec<Note>,
    pub build_network_used: bool,
}

/// The lowest value of one criterion, where it can be compared at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Leader {
    pub criterion: CriterionKey,
    pub option: String,
    pub value: f64,
    pub unit: String,
    /// The tag of the figures compared: `verified` or `estimate`.
    pub basis: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub run_id: String,
    pub scenario_id: String,
    pub title: String,
    pub question: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub build_network_used: bool,
    pub criteria: Vec<super::Criterion>,
    pub options: Vec<OptionReport>,
    pub leaders: Vec<Leader>,
    /// What the reader must keep in mind; shown above the table.
    pub caveats: Vec<String>,
}

fn speculation(criterion: CriterionKey, text: String, note: &str) -> Note {
    Note { criterion, text, tag: Tag::Speculation { note: note.into() } }
}

/// What running and rolling back usually take on a kind of plan, when the person wrote nothing.
fn default_notes(kind: Option<PlanKind>) -> (&'static str, &'static str) {
    match kind {
        Some(PlanKind::Vm) => (
            "You patch the system, run backups and watch the server yourself.",
            "Keep the previous release on the server and switch back; you script that yourself.",
        ),
        Some(PlanKind::Container) => (
            "The provider runs the machine; you still own the image, health checks and scaling settings.",
            "Redeploy the previous image from the provider's dashboard or CLI.",
        ),
        Some(PlanKind::Function) => (
            "No server to patch; cold starts, time limits and per-request limits become your concern.",
            "Point the function back at its previous version.",
        ),
        None => ("No plan was chosen, so nothing is known about running it.", "No plan was chosen, so nothing is known about rolling it back."),
    }
}

fn metric<'a>(figures: &'a [Figure], name: &str) -> Option<&'a Figure> {
    figures.iter().find(|f| f.metric == name)
}

/// The comparison for `run`, computed from what it stored (cost staleness uses `today`).
pub fn report(run: &Run, today: chrono::NaiveDate) -> Report {
    let s = &run.scenario;
    let mut options = Vec::new();
    for (o, b) in s.options.iter().zip(&run.branches) {
        let entry = o.plan.as_deref().and_then(prices::find);
        let cost = match entry {
            Some(p) => prices::estimate_monthly(p, &s.traffic, today),
            None => prices::cost_figure(None, &s.traffic),
        };
        let (ops_default, rollback_default) = default_notes(entry.map(|p| p.kind));
        let note = |criterion, own: &str, fallback: &str| {
            if own.trim().is_empty() {
                speculation(criterion, fallback.into(), "kwant's general note for this kind of plan, not measured")
            } else {
                speculation(criterion, own.trim().into(), "the person's own note, not measured")
            }
        };
        let measured = ["p95_latency", "p50_latency", "p99_latency", "requests_per_sec", "ok_share", "success_rate", "memory_after_load"]
            .iter()
            .filter_map(|m| metric(&b.figures, m).cloned())
            .collect();
        options.push(OptionReport {
            key: o.key.clone(),
            label: o.label.clone(),
            plan: o.plan.clone(),
            status: b.status,
            error_code: b.error_code.clone(),
            error: b.error.clone(),
            measured,
            cost,
            notes: vec![note(CriterionKey::OpsBurden, &o.ops_notes, ops_default), note(CriterionKey::Rollback, &o.rollback_notes, rollback_default)],
            build_network_used: b.build_network_used,
        });
    }

    let mut leaders = Vec::new();
    // p95: only options that answered every request are comparable
    let p95: Vec<(&OptionReport, &Figure)> = options
        .iter()
        .filter(|o| o.status == BranchStatus::Ok && metric(&o.measured, "ok_share").is_some_and(|f| f.value >= 1.0))
        .filter_map(|o| metric(&o.measured, "p95_latency").map(|f| (o, f)))
        .collect();
    if p95.len() >= 2 {
        let (o, f) = p95.iter().min_by(|a, b| a.1.value.total_cmp(&b.1.value)).unwrap();
        leaders.push(Leader { criterion: CriterionKey::P95Latency, option: o.key.clone(), value: f.value, unit: f.unit.clone(), basis: "verified".into() });
    }
    // cost: only estimates in one currency (nothing is converted)
    let costs: Vec<&OptionReport> = options.iter().filter(|o| matches!(o.cost.tag, Tag::Estimate { .. })).collect();
    if costs.len() >= 2 && costs.iter().all(|o| o.cost.unit == costs[0].cost.unit) {
        let o = costs.iter().min_by(|a, b| a.cost.value.total_cmp(&b.cost.value)).unwrap();
        leaders.push(Leader { criterion: CriterionKey::Cost, option: o.key.clone(), value: o.cost.value, unit: o.cost.unit.clone(), basis: "estimate".into() });
    }

    let mut caveats = vec![
        "Latency, throughput and memory were measured on this server inside a container, not on the provider's hardware.".to_string(),
        "Ops burden and rollback notes are guesses, not measurements.".to_string(),
    ];
    if costs.len() >= 2 && !costs.iter().all(|o| o.cost.unit == costs[0].cost.unit) {
        caveats.push("The costs are in different currencies and were not converted, so they are not ranked.".into());
    }
    if options.iter().any(|o| matches!(o.cost.tag, Tag::Estimate { stale: true, .. })) {
        caveats.push(format!("Some prices are more than {} days old.", prices::STALE_AFTER_DAYS));
    }
    let with_network: Vec<&str> = options.iter().filter(|o| o.build_network_used).map(|o| o.key.as_str()).collect();
    if !with_network.is_empty() {
        caveats.push(format!(
            "The build step had network access for: {}. The app and the load test never did.",
            with_network.join(", ")
        ));
    }
    if options.iter().any(|o| o.status == BranchStatus::Failed) {
        caveats.push("Some options failed, so they have no measurements.".into());
    }
    if run.status == RunStatus::Interrupted {
        caveats.push("The server stopped during this run; it is incomplete.".into());
    }

    Report {
        run_id: run.id.clone(),
        scenario_id: run.scenario_id.clone(),
        title: s.title.clone(),
        question: s.question.clone(),
        status: run.status,
        started_at: run.started_at,
        finished_at: run.finished_at,
        build_network_used: run.build_network_used(),
        criteria: s.criteria.clone(),
        options,
        leaders,
        caveats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parallel::harness::HarnessError;
    use crate::parallel::{AppSpec, Criterion, DeployOption, Limits, LoadSpec, Requirements, Traffic};
    use chrono::NaiveDate;

    fn option(key: &str, plan: Option<&str>, context: bool) -> DeployOption {
        DeployOption {
            key: key.into(),
            label: key.to_uppercase(),
            plan: plan.map(String::from),
            app: AppSpec {
                image: (!context).then(|| "kwnt-executor:v1".into()),
                context: context.then(|| "/tmp/app".into()),
                dockerfile: None,
                port: 8080,
                health_path: "/".into(),
            },
            limits: Limits::default(),
            ops_notes: String::new(),
            rollback_notes: String::new(),
        }
    }

    fn scenario(options: Vec<DeployOption>) -> Scenario {
        Scenario {
            id: "s1".into(),
            title: "VPS or not".into(),
            question: "Move?".into(),
            requirements: Requirements::default(),
            traffic: Traffic { monthly_requests: 1_000_000, peak_rps: 10.0, avg_response_kb: 20.0, avg_duration_ms: 50.0 },
            load: LoadSpec::default(),
            criteria: vec![Criterion { key: CriterionKey::Cost, weight: 5 }],
            options,
            build_network: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn measured(p95: f64, ok: f64) -> Vec<Figure> {
        let at = Utc::now();
        vec![
            Figure::verified("p95_latency", p95, "ms", "docker run … oha", "{}", at),
            Figure::verified("requests_per_sec", 500.0, "1/s", "docker run … oha", "{}", at),
            Figure::verified("ok_share", ok, "ratio", "docker run … oha", "{}", at),
            Figure::verified("memory_after_load", 20.0, "MB", "docker stats", "20MiB / 128MiB", at),
        ]
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 21).unwrap()
    }

    #[test]
    fn the_report_tags_every_number_and_ranks_only_what_is_comparable() {
        let s = scenario(vec![option("vps", Some("do-basic-1gb"), false), option("fly", Some("fly-shared-1x-512"), true), option("lambda", None, false)]);
        let mut run = Run::new(&s);
        run.branches[0].status = BranchStatus::Ok;
        run.branches[0].figures = measured(9.0, 1.0);
        run.branches[1].status = BranchStatus::Ok;
        run.branches[1].figures = measured(4.0, 1.0);
        run.branches[2].status = BranchStatus::Failed;
        run.branches[2].error_code = Some(HarnessError::NotReady.code().into());
        run.status = RunStatus::Finished;
        let r = report(&run, today());

        assert_eq!(r.options.len(), 3);
        for o in &r.options {
            assert!(o.measured.iter().all(|f| matches!(f.tag, Tag::Verified { .. })));
            assert!(o.notes.iter().all(|n| matches!(n.tag, Tag::Speculation { .. })));
            assert_eq!(o.notes.iter().map(|n| n.criterion).collect::<Vec<_>>(), [CriterionKey::OpsBurden, CriterionKey::Rollback]);
        }
        assert!(matches!(r.options[0].cost.tag, Tag::Estimate { .. }));
        assert!(matches!(r.options[2].cost.tag, Tag::Speculation { .. }), "no plan: cost is a guess");
        assert!(r.options[2].measured.is_empty());
        assert_eq!(r.options[2].error_code.as_deref(), Some("not_ready"));

        let p95 = r.leaders.iter().find(|l| l.criterion == CriterionKey::P95Latency).unwrap();
        assert_eq!((p95.option.as_str(), p95.value, p95.basis.as_str()), ("fly", 4.0, "verified"));
        let cost = r.leaders.iter().find(|l| l.criterion == CriterionKey::Cost).unwrap();
        assert_eq!((cost.option.as_str(), cost.unit.as_str(), cost.basis.as_str()), ("fly", "USD", "estimate"));
        assert!(!r.leaders.iter().any(|l| matches!(l.criterion, CriterionKey::OpsBurden | CriterionKey::Rollback)), "guesses are never ranked");
        assert!(r.caveats.iter().any(|c| c.contains("not on the provider's hardware")));
        assert!(r.caveats.iter().any(|c| c.contains("failed")));
        assert!(!r.build_network_used);
    }

    #[test]
    fn an_option_that_dropped_requests_is_not_ranked_on_latency() {
        let s = scenario(vec![option("a", None, false), option("b", None, false)]);
        let mut run = Run::new(&s);
        for (b, figures) in run.branches.iter_mut().zip([measured(1.0, 0.5), measured(5.0, 1.0)]) {
            b.status = BranchStatus::Ok;
            b.figures = figures;
        }
        let r = report(&run, today());
        assert!(r.leaders.is_empty(), "only one option answered everything, and neither has a price: {:?}", r.leaders);
    }

    #[test]
    fn currencies_are_not_ranked_against_each_other_and_stale_prices_are_said() {
        let s = scenario(vec![option("hetzner", Some("hetzner-cpx12"), false), option("do", Some("do-basic-1gb"), false)]);
        let r = report(&Run::new(&s), NaiveDate::from_ymd_opt(2027, 9, 1).unwrap());
        assert!(!r.leaders.iter().any(|l| l.criterion == CriterionKey::Cost));
        assert!(r.caveats.iter().any(|c| c.contains("different currencies")));
        assert!(r.caveats.iter().any(|c| c.contains("days old")));
    }

    #[test]
    fn the_persons_own_notes_win_and_the_build_network_is_reported() {
        let mut o = option("vps", Some("do-basic-1gb"), true);
        o.ops_notes = "  I already run two of these  ".into();
        let mut s = scenario(vec![o, option("img", None, false)]);
        s.build_network = true;
        let mut run = Run::new(&s);
        run.branches[0].build_network_used = true;
        let r = report(&run, today());
        assert_eq!(r.options[0].notes[0].text, "I already run two of these");
        assert!(matches!(&r.options[0].notes[0].tag, Tag::Speculation { note } if note.contains("own note")));
        assert!(r.options[0].notes[1].text.contains("previous release"), "VM default rollback note");
        assert!(r.build_network_used && r.options[0].build_network_used && !r.options[1].build_network_used);
        assert!(r.caveats.iter().any(|c| c.contains("network access for: vps")));
    }
}
