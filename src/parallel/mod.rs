//! Parallel Universe: compares up to three ways to deploy one app ("current host vs a VPS vs
//! serverless") without spending anything. Each option is tried in a throwaway container on this
//! machine ([`harness`]); costs come from a dated price table checked into the repo ([`prices`]).
//!
//! Every number is a [`Figure`] tagged [`Tag::Verified`] (measured here, with the command and its
//! raw output), [`Tag::Estimate`] (computed from a priced plan, with the source and its date) or
//! [`Tag::Speculation`] (neither). No cloud account is used and the server never fetches prices.
//!
//! See docs/parallel-universe.md.

pub mod harness;
pub mod prices;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// Most options one scenario compares (the server has 4 GB of RAM).
pub const MAX_OPTIONS: usize = 3;
/// What all options of a scenario may reserve together.
pub const MAX_TOTAL_MEMORY_MB: u32 = 3072;
/// Longest raw output kept on a verified figure.
pub const MAX_RAW_OUTPUT: usize = 64 * 1024;

/// What a scenario asks and how it is judged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub requirements: Requirements,
    pub traffic: Traffic,
    #[serde(default)]
    pub load: LoadSpec,
    #[serde(default)]
    pub criteria: Vec<Criterion>,
    pub options: Vec<DeployOption>,
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Requirements {
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub min_memory_mb: Option<u32>,
    #[serde(default)]
    pub needs_persistent_disk: bool,
    #[serde(default)]
    pub regions: Vec<String>,
}

/// The traffic the estimates assume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Traffic {
    pub monthly_requests: u64,
    #[serde(default)]
    pub peak_rps: f64,
    /// Average response size, for egress.
    #[serde(default)]
    pub avg_response_kb: f64,
    /// Average time one request runs, for per-second (function) pricing.
    #[serde(default)]
    pub avg_duration_ms: f64,
}

/// The load test the harness runs against each option.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadSpec {
    pub requests: u32,
    pub concurrency: u32,
    pub path: String,
}

impl Default for LoadSpec {
    fn default() -> Self {
        Self { requests: 2000, concurrency: 10, path: "/".into() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionKey {
    Cost,
    P95Latency,
    OpsBurden,
    Rollback,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Criterion {
    pub key: CriterionKey,
    pub weight: u8,
}

/// One way to deploy the app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeployOption {
    /// Unique within the scenario: a lowercase slug, used in container names.
    pub key: String,
    pub label: String,
    /// A price-table id; none means the cost is speculation.
    #[serde(default)]
    pub plan: Option<String>,
    pub app: AppSpec,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub ops_notes: String,
    #[serde(default)]
    pub rollback_notes: String,
}

/// What runs in the container: an image already on this machine, or a directory to build.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSpec {
    #[serde(default)]
    pub image: Option<String>,
    /// Absolute directory with a Dockerfile, built offline.
    #[serde(default)]
    pub context: Option<String>,
    /// Relative to `context`.
    #[serde(default)]
    pub dockerfile: Option<String>,
    /// The port the app listens on inside its container (never published on the host).
    pub port: u16,
    #[serde(default = "root_path")]
    pub health_path: String,
}

fn root_path() -> String {
    "/".into()
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    pub cpus: f64,
    pub memory_mb: u32,
    pub pids: u32,
    pub timeout_secs: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { cpus: 1.0, memory_mb: 512, pids: 128, timeout_secs: 300 }
    }
}

impl Limits {
    pub fn validate(&self) -> Result<(), String> {
        if !(0.1..=2.0).contains(&self.cpus) {
            return Err("cpus must be between 0.1 and 2".into());
        }
        if !(64..=2048).contains(&self.memory_mb) {
            return Err("memory_mb must be between 64 and 2048".into());
        }
        if !(16..=512).contains(&self.pids) {
            return Err("pids must be between 16 and 512".into());
        }
        if !(30..=900).contains(&self.timeout_secs) {
            return Err("timeout_secs must be between 30 and 900".into());
        }
        Ok(())
    }
}

/// A slug for container names: `a-z`, `0-9`, `-`, starting with a letter or digit.
pub fn is_slug(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
}

/// An image reference docker accepts and that cannot be read as a flag.
pub fn is_image_ref(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && !s.starts_with('-')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || "./:_@-".contains(c))
}

fn is_url_path(s: &str) -> bool {
    s.starts_with('/') && s.len() <= 512 && s.chars().all(|c| c.is_ascii_graphic())
}

impl AppSpec {
    pub fn validate(&self) -> Result<(), String> {
        match (&self.image, &self.context) {
            (Some(image), None) => {
                if !is_image_ref(image) {
                    return Err("image must be a docker image name like name:tag".into());
                }
                if self.dockerfile.is_some() {
                    return Err("dockerfile goes with context, not image".into());
                }
            }
            (None, Some(context)) => {
                let p = std::path::Path::new(context);
                if !p.is_absolute() || context.len() > 1024 || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
                    return Err("context must be an absolute directory without ..".into());
                }
                if let Some(f) = &self.dockerfile {
                    let fp = std::path::Path::new(f);
                    if f.is_empty() || fp.is_absolute() || fp.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
                        return Err("dockerfile must be a path inside context".into());
                    }
                }
            }
            _ => return Err("an option needs exactly one of image or context".into()),
        }
        if self.port == 0 {
            return Err("port must be 1–65535".into());
        }
        if !is_url_path(&self.health_path) {
            return Err("health_path must start with /".into());
        }
        Ok(())
    }
}

impl Scenario {
    /// Checks every field the person sent; `Err` is a sentence for them.
    pub fn validate(&self) -> Result<(), String> {
        let title = self.title.trim();
        if title.is_empty() || title.chars().count() > 120 {
            return Err("title must be 1–120 characters".into());
        }
        if self.question.chars().count() > 2000 || self.requirements.notes.chars().count() > 4000 {
            return Err("question or notes are too long".into());
        }
        if self.requirements.regions.len() > 10 || self.requirements.regions.iter().any(|r| r.chars().count() > 40) {
            return Err("at most 10 regions, 40 characters each".into());
        }
        let t = &self.traffic;
        if t.monthly_requests > 100_000_000_000 {
            return Err("monthly_requests is too large".into());
        }
        for (name, v) in [("peak_rps", t.peak_rps), ("avg_response_kb", t.avg_response_kb), ("avg_duration_ms", t.avg_duration_ms)] {
            if !v.is_finite() || !(0.0..=1e7).contains(&v) {
                return Err(format!("{name} must be a number from 0"));
            }
        }
        if !(1..=100_000).contains(&self.load.requests) || !(1..=64).contains(&self.load.concurrency) {
            return Err("load: requests 1–100000, concurrency 1–64".into());
        }
        if !is_url_path(&self.load.path) {
            return Err("load path must start with /".into());
        }
        let mut seen = Vec::new();
        for c in &self.criteria {
            if c.weight > 10 {
                return Err("criterion weights are 0–10".into());
            }
            if seen.contains(&c.key) {
                return Err("each criterion at most once".into());
            }
            seen.push(c.key);
        }
        if self.options.is_empty() || self.options.len() > MAX_OPTIONS {
            return Err(format!("a scenario has 1–{MAX_OPTIONS} options"));
        }
        let mut keys = Vec::new();
        let mut memory = 0;
        for o in &self.options {
            if !is_slug(&o.key, 32) {
                return Err("option keys are lowercase letters, digits and -, at most 32".into());
            }
            if keys.contains(&o.key.as_str()) {
                return Err("option keys must be unique".into());
            }
            keys.push(o.key.as_str());
            let label = o.label.trim();
            if label.is_empty() || label.chars().count() > 80 {
                return Err("option labels are 1–80 characters".into());
            }
            if o.ops_notes.chars().count() > 2000 || o.rollback_notes.chars().count() > 2000 {
                return Err("option notes are too long".into());
            }
            if let Some(plan) = &o.plan {
                if prices::find(plan).is_none() {
                    return Err(format!("no price-table plan called {plan}"));
                }
            }
            o.app.validate().map_err(|e| format!("{}: {e}", o.key))?;
            o.limits.validate().map_err(|e| format!("{}: {e}", o.key))?;
            memory += o.limits.memory_mb;
        }
        if memory > MAX_TOTAL_MEMORY_MB {
            return Err(format!("the options may reserve at most {MAX_TOTAL_MEMORY_MB} MB together"));
        }
        Ok(())
    }
}

/// Where a number comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tag", rename_all = "snake_case")]
pub enum Tag {
    /// Measured on this machine: the exact command and what it printed.
    Verified { command: String, raw_output: String, measured_at: DateTime<Utc> },
    /// Computed from a priced plan.
    Estimate { source_url: String, as_of: NaiveDate, stale: bool, assumptions: Vec<String> },
    /// Neither measured nor sourced.
    Speculation { note: String },
}

/// One number in a comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Figure {
    pub metric: String,
    pub value: f64,
    pub unit: String,
    #[serde(flatten)]
    pub tag: Tag,
}

impl Figure {
    pub fn verified(metric: &str, value: f64, unit: &str, command: &str, raw_output: &str, at: DateTime<Utc>) -> Self {
        let raw = crate::zombie::clip(raw_output, MAX_RAW_OUTPUT);
        Figure { metric: metric.into(), value, unit: unit.into(), tag: Tag::Verified { command: command.into(), raw_output: raw, measured_at: at } }
    }
}
