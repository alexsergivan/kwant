//! The new-package check: packages a PR adds to a manifest, looked up on npm, PyPI or crates.io
//! for "doesn't exist", "under 30 days old", "very few downloads" and "one letter off a popular
//! name".
//!
//! Only new names count: the manifest's head text is parsed, its base is rebuilt by undoing the
//! PR's patch, and a version bump of a package already there isn't new. Path, git, workspace and
//! URL dependencies aren't registry packages and are left alone; a custom registry or index can't
//! be checked and makes the tile `not_run` with the reason. Only the URL-encoded name is sent to
//! a registry, with GET and no body: never source, paths or versions.

use super::super::facts::FileDiff;
use super::{as_data, Finding, Tile};
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// Under this many days old is "new".
pub const NEW_DAYS: i64 = 30;
/// npm and PyPI: fewer downloads than this in the last week is "very few".
pub const FEW_WEEKLY: u64 = 100;
/// crates.io: fewer than this in its "recent" (90-day) count is "very few".
pub const FEW_RECENT_CRATES: u64 = 1_000;
/// Most new packages looked up per PR; the rest leave the tile `not_run`.
pub const MAX_LOOKUPS: usize = 25;
/// Most findings listed (the tile still counts every hit).
const MAX_FINDINGS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Eco {
    Npm,
    PyPi,
    Crates,
}

impl Eco {
    pub fn label(self) -> &'static str {
        match self {
            Eco::Npm => "npm",
            Eco::PyPi => "PyPI",
            Eco::Crates => "crates.io",
        }
    }

    /// The name as the registry compares names: npm as is, PyPI per PEP 503, crates with `_`
    /// and `-` the same and case ignored.
    pub fn norm(self, name: &str) -> String {
        match self {
            Eco::Npm => name.to_string(),
            Eco::PyPi => {
                let mut out = String::new();
                let mut sep = false;
                for c in name.chars() {
                    if matches!(c, '-' | '_' | '.') {
                        sep = true;
                    } else {
                        if sep && !out.is_empty() {
                            out.push('-');
                        }
                        sep = false;
                        out.extend(c.to_lowercase());
                    }
                }
                out
            }
            Eco::Crates => name.to_lowercase().replace('_', "-"),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Reading manifests

/// Which manifest a path is, if any.
pub fn manifest_kind(path: &str) -> Option<Eco> {
    if path.split('/').any(|p| p == "node_modules" || p == "vendor" || p == "target") {
        return None;
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    match name {
        "Cargo.toml" => Some(Eco::Crates),
        "package.json" => Some(Eco::Npm),
        "pyproject.toml" => Some(Eco::PyPi),
        n if n.starts_with("requirements") && (n.ends_with(".txt") || n.ends_with(".in")) => Some(Eco::PyPi),
        _ => None,
    }
}

/// What one manifest declares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Declared {
    /// Registry packages: normalized name → the name as written (the real one for an alias).
    pub deps: BTreeMap<String, String>,
    /// Packages from a custom registry or index: normalized name → why.
    pub custom: BTreeMap<String, String>,
    /// The project's own names (its package name, path and workspace members).
    pub own: BTreeSet<String>,
}

/// Parses one manifest. `None` when it can't be read.
pub fn declared(path: &str, text: &str) -> Option<Declared> {
    let name = path.rsplit('/').next().unwrap_or(path);
    match manifest_kind(path)? {
        Eco::Crates => cargo(text),
        Eco::Npm => npm(text),
        Eco::PyPi if name == "pyproject.toml" => pyproject(text),
        Eco::PyPi => Some(requirements(text)),
    }
}

fn cargo(text: &str) -> Option<Declared> {
    let doc: toml::Value = toml::from_str(text).ok()?;
    let mut d = Declared::default();
    let norm = |n: &str| Eco::Crates.norm(n);
    if let Some(n) = doc.get("package").and_then(|p| p.get("name")).and_then(|n| n.as_str()) {
        d.own.insert(norm(n));
    }
    let mut tables: Vec<&toml::Value> = Vec::new();
    for key in ["dependencies", "dev-dependencies", "dev_dependencies", "build-dependencies", "build_dependencies"] {
        tables.extend(doc.get(key));
        tables.extend(doc.get("workspace").and_then(|w| w.get(key)));
    }
    if let Some(targets) = doc.get("target").and_then(|t| t.as_table()) {
        for t in targets.values() {
            for key in ["dependencies", "dev-dependencies", "dev_dependencies", "build-dependencies", "build_dependencies"] {
                tables.extend(t.get(key));
            }
        }
    }
    for table in tables.into_iter().filter_map(|t| t.as_table()) {
        for (key, v) in table {
            match v {
                toml::Value::String(_) => {
                    d.deps.insert(norm(key), key.clone());
                }
                toml::Value::Table(t) => {
                    let real = t.get("package").and_then(|p| p.as_str()).unwrap_or(key);
                    let local = t.contains_key("path") || t.contains_key("git") || t.get("workspace").and_then(|w| w.as_bool()) == Some(true);
                    if local {
                        d.own.insert(norm(key));
                        d.own.insert(norm(real));
                    } else if let Some(r) = t.get("registry").or_else(|| t.get("registry-index")).and_then(|r| r.as_str()) {
                        d.custom.insert(norm(real), format!("{} comes from the custom registry {}", as_data(real), as_data(r)));
                    } else {
                        d.deps.insert(norm(real), real.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    Some(d)
}

fn npm(text: &str) -> Option<Declared> {
    let doc: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut d = Declared::default();
    if let Some(n) = doc.get("name").and_then(|n| n.as_str()) {
        d.own.insert(n.to_string());
    }
    for key in ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"] {
        let Some(map) = doc.get(key).and_then(|m| m.as_object()) else { continue };
        for (name, spec) in map {
            let spec = spec.as_str().unwrap_or("").trim();
            if let Some(rest) = spec.strip_prefix("npm:") {
                // `"alias": "npm:real@^1"`: the real package is what gets installed
                let real = match rest.rfind('@') {
                    Some(i) if i > 0 => &rest[..i],
                    _ => rest,
                };
                d.deps.insert(real.to_string(), real.to_string());
                continue;
            }
            let lower = spec.to_ascii_lowercase();
            let local = ["file:", "link:", "workspace:", "portal:", "catalog:", "patch:", "git+", "git:", "github:", "gitlab:", "bitbucket:", "http:", "https:"]
                .iter()
                .any(|p| lower.starts_with(p))
                || (spec.contains('/') && !spec.starts_with('@'));
            if local {
                d.own.insert(name.clone());
            } else {
                d.deps.insert(name.clone(), name.clone());
            }
        }
    }
    Some(d)
}

/// The name at the start of a PEP 508 requirement, or none for a URL or option line.
fn pep508_name(req: &str) -> Option<&str> {
    let req = req.trim();
    let end = req.find(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))).unwrap_or(req.len());
    let name = &req[..end];
    if name.is_empty() {
        return None;
    }
    // `name @ https://…` installs from a URL, not the index
    let rest = req[end..].trim_start();
    let rest = match rest.strip_prefix('[') {
        Some(r) => r.split_once(']').map_or("", |(_, after)| after).trim_start(),
        None => rest,
    };
    if rest.starts_with('@') {
        return None;
    }
    Some(name)
}

fn requirements(text: &str) -> Declared {
    let mut d = Declared::default();
    let mut index = None;
    let mut names = Vec::new();
    for raw in text.lines() {
        let line = raw.split(" #").next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('-') {
            let opt = line.split([' ', '=']).next().unwrap_or("");
            if matches!(opt, "-i" | "--index-url" | "--extra-index-url") {
                index = Some(opt.to_string());
            }
            continue;
        }
        if line.contains("://") {
            continue;
        }
        if let Some(n) = pep508_name(line) {
            names.push(n.to_string());
        }
    }
    for n in names {
        match &index {
            Some(opt) => {
                d.custom.insert(Eco::PyPi.norm(&n), format!("{} may come from a custom index ({} in the requirements file)", as_data(&n), as_data(opt)));
            }
            None => {
                d.deps.insert(Eco::PyPi.norm(&n), n);
            }
        }
    }
    d
}

fn pyproject(text: &str) -> Option<Declared> {
    let doc: toml::Value = toml::from_str(text).ok()?;
    let mut d = Declared::default();
    let norm = |n: &str| Eco::PyPi.norm(n);
    let project = doc.get("project");
    let poetry = doc.get("tool").and_then(|t| t.get("poetry"));
    let uv = doc.get("tool").and_then(|t| t.get("uv"));
    for n in [project, poetry].into_iter().flatten().filter_map(|p| p.get("name")).filter_map(|n| n.as_str()) {
        d.own.insert(norm(n));
    }
    // uv: a default index replaces PyPI for everything; `tool.uv.sources` pins single packages
    let uv_default = uv
        .and_then(|u| u.get("index"))
        .and_then(|i| i.as_array())
        .is_some_and(|a| a.iter().any(|i| i.get("default").and_then(|d| d.as_bool()) == Some(true)));
    let uv_sources = uv.and_then(|u| u.get("sources")).and_then(|s| s.as_table());
    let add = |d: &mut Declared, name: &str, how: Option<&str>| {
        let n = norm(name);
        if n == "python" {
            return;
        }
        if let Some(src) = uv_sources.and_then(|s| s.get(name).or_else(|| s.get(&n))) {
            if src.get("index").is_some() {
                d.custom.insert(n, format!("{} comes from a custom index (tool.uv.sources)", as_data(name)));
            } else {
                d.own.insert(n);
            }
            return;
        }
        match how {
            Some(why) => {
                d.custom.insert(n, format!("{} {why}", as_data(name)));
            }
            None if uv_default => {
                d.custom.insert(n, format!("{} comes from a custom default index (tool.uv.index)", as_data(name)));
            }
            None => {
                d.deps.insert(n, name.to_string());
            }
        }
    };
    let pep508 = |d: &mut Declared, list: Option<&toml::Value>| {
        for s in list.and_then(|l| l.as_array()).into_iter().flatten().filter_map(|s| s.as_str()) {
            if let Some(n) = pep508_name(s) {
                add(d, n, None);
            }
        }
    };
    pep508(&mut d, project.and_then(|p| p.get("dependencies")));
    for v in project.and_then(|p| p.get("optional-dependencies")).and_then(|o| o.as_table()).into_iter().flat_map(|t| t.values()) {
        pep508(&mut d, Some(v));
    }
    for v in doc.get("dependency-groups").and_then(|o| o.as_table()).into_iter().flat_map(|t| t.values()) {
        pep508(&mut d, Some(v));
    }
    let mut poetry_tables: Vec<&toml::Value> = Vec::new();
    if let Some(p) = poetry {
        poetry_tables.extend(p.get("dependencies"));
        poetry_tables.extend(p.get("dev-dependencies"));
        for g in p.get("group").and_then(|g| g.as_table()).into_iter().flat_map(|t| t.values()) {
            poetry_tables.extend(g.get("dependencies"));
        }
    }
    for t in poetry_tables.into_iter().filter_map(|t| t.as_table()) {
        for (name, v) in t {
            match v {
                toml::Value::Table(spec) if spec.contains_key("path") || spec.contains_key("git") || spec.contains_key("url") => {
                    d.own.insert(norm(name));
                }
                toml::Value::Table(spec) if spec.contains_key("source") => {
                    let src = spec.get("source").and_then(|s| s.as_str()).unwrap_or("");
                    add(&mut d, name, Some(&format!("comes from the custom source {}", as_data(src))));
                }
                _ => add(&mut d, name, None),
            }
        }
    }
    Some(d)
}

/// The file before the PR: `head` with the patch undone. `None` when the patch doesn't fit.
pub fn base_text(head: &str, patch: &str) -> Option<String> {
    let head_lines: Vec<&str> = head.split('\n').collect();
    let mut out: Vec<String> = Vec::new();
    let mut at = 0usize; // next head line (0-based) not yet copied
    let mut lines = patch.split('\n').peekable();
    while let Some(line) = lines.next() {
        let Some((_, new_start)) = super::hunk_starts(line) else {
            if line.is_empty() && lines.peek().is_none() {
                break;
            }
            return None; // text outside a hunk
        };
        let start = (new_start as usize).saturating_sub(1).max(at);
        if start > head_lines.len() {
            return None;
        }
        out.extend(head_lines[at..start].iter().map(|s| s.to_string()));
        at = start;
        while let Some(l) = lines.peek() {
            if super::hunk_starts(l).is_some() {
                break;
            }
            let l = lines.next().unwrap_or_default();
            match l.as_bytes().first() {
                Some(b'+') => {
                    if head_lines.get(at).map(|h| h.trim_end_matches('\r')) != Some(l[1..].trim_end_matches('\r')) {
                        return None;
                    }
                    at += 1;
                }
                Some(b'-') => out.push(l[1..].to_string()),
                Some(b'\\') => {}
                Some(b' ') => {
                    if head_lines.get(at).map(|h| h.trim_end_matches('\r')) != Some(l[1..].trim_end_matches('\r')) {
                        return None;
                    }
                    out.push(head_lines[at].to_string());
                    at += 1;
                }
                None if lines.peek().is_none() => {}
                _ => return None,
            }
        }
    }
    out.extend(head_lines[at.min(head_lines.len())..].iter().map(|s| s.to_string()));
    Some(out.join("\n"))
}

/// A package the PR adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDep {
    pub eco: Eco,
    /// As written in the manifest (the real name for an alias).
    pub name: String,
    pub file: String,
    pub line: Option<u64>,
}

/// What the PR's manifests add.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Added {
    pub deps: Vec<NewDep>,
    /// Why some packages couldn't be checked.
    pub skipped: Vec<String>,
    /// The repo's own names, per ecosystem, from every manifest read.
    pub own: BTreeMap<Eco, BTreeSet<String>>,
}

/// The packages the PR adds, from its changed manifests' head text in `contents`.
pub fn added(files: &[FileDiff], contents: &BTreeMap<String, String>) -> Added {
    let mut a = Added::default();
    let mut seen = BTreeSet::new();
    for f in files.iter().filter(|f| f.status != "removed") {
        let Some(eco) = manifest_kind(&f.path) else { continue };
        let Some(head) = contents.get(&f.path) else {
            a.skipped.push(format!("{} couldn't be read", f.path));
            continue;
        };
        let Some(now) = declared(&f.path, head) else {
            a.skipped.push(format!("{} couldn't be parsed", f.path));
            continue;
        };
        let before = if f.status == "added" {
            Declared::default()
        } else {
            match f.patch.as_deref().and_then(|p| base_text(head, p)) {
                Some(base) => declared(f.previous_path.as_deref().unwrap_or(&f.path), &base).unwrap_or_default(),
                None if f.patch.is_none() && f.additions == 0 => now.clone(),
                None => {
                    a.skipped.push(format!("{}'s earlier version couldn't be rebuilt from the diff", f.path));
                    continue;
                }
            }
        };
        a.own.entry(eco).or_default().extend(now.own.iter().cloned());
        let added_lines = f.patch.as_deref().map(super::gitleaks::added_lines).unwrap_or_default();
        for (key, reason) in &now.custom {
            if !before.custom.contains_key(key) && !before.deps.contains_key(key) {
                a.skipped.push(reason.clone());
            }
        }
        for (key, name) in &now.deps {
            if before.deps.contains_key(key) || before.custom.contains_key(key) || !seen.insert((eco, key.clone())) {
                continue;
            }
            let line = added_lines.iter().find(|(_, l)| l.contains(name.as_str())).map(|(n, _)| *n);
            a.deps.push(NewDep { eco, name: name.clone(), file: f.path.clone(), line });
        }
    }
    a
}

// ---------------------------------------------------------------------------------------------
// One letter off

/// Pinned popular names per registry: never looked up, and what a near-miss is measured against.
pub fn popular(eco: Eco) -> &'static [&'static str] {
    match eco {
        Eco::Npm => &[
            "react", "react-dom", "preact", "vue", "svelte", "angular", "next", "nuxt", "vite", "webpack", "rollup", "esbuild", "parcel",
            "babel", "typescript", "eslint", "prettier", "jest", "vitest", "mocha", "chai", "sinon", "lodash", "underscore", "ramda",
            "axios", "express", "koa", "fastify", "hapi", "request", "chalk", "colors", "color", "commander", "yargs", "minimist",
            "debug", "dotenv", "uuid", "nanoid", "moment", "dayjs", "luxon", "date-fns", "classnames", "clsx", "rxjs", "redux", "mobx",
            "zustand", "jquery", "bootstrap", "tailwindcss", "postcss", "autoprefixer", "sass", "less", "glob", "rimraf", "mkdirp",
            "semver", "inquirer", "ora", "execa", "cross-env", "nodemon", "concurrently", "zod", "yup", "joi", "ajv", "socket.io",
            "ws", "graphql", "apollo", "prisma", "mongoose", "sequelize", "knex", "pg", "mysql", "mysql2", "redis", "ioredis", "sqlite3",
            "bcrypt", "bcryptjs", "jsonwebtoken", "passport", "cors", "helmet", "morgan", "body-parser", "cookie-parser", "multer",
            "sharp", "jimp", "puppeteer", "playwright", "cheerio", "jsdom", "marked", "markdown-it", "highlight.js", "prismjs",
            "three", "d3", "chart.js", "echarts", "lucide-react", "framer-motion", "immer", "electron", "tslib", "core-js",
            "regenerator-runtime", "node-fetch", "cross-fetch", "got", "superagent", "form-data", "qs", "async", "bluebird",
            "fs-extra", "chokidar", "yaml", "js-yaml", "ini", "xml2js", "csv-parse", "papaparse", "winston", "pino", "bunyan",
        ],
        Eco::PyPi => &[
            "requests", "urllib3", "httpx", "aiohttp", "numpy", "pandas", "scipy", "matplotlib", "seaborn", "plotly", "scikit-learn",
            "torch", "tensorflow", "keras", "transformers", "flask", "django", "fastapi", "starlette", "uvicorn", "gunicorn",
            "pydantic", "sqlalchemy", "alembic", "psycopg2", "psycopg", "pymysql", "redis", "celery", "boto3", "botocore",
            "pytest", "tox", "nox", "black", "ruff", "flake8", "pylint", "mypy", "isort", "click", "typer", "rich", "tqdm", "pyyaml",
            "toml", "tomli", "jinja2", "markupsafe", "werkzeug", "itsdangerous", "six", "attrs", "setuptools", "wheel", "pip",
            "certifi", "idna", "charset-normalizer", "chardet", "cryptography", "pyjwt", "bcrypt", "paramiko", "pillow",
            "beautifulsoup4", "lxml", "selenium", "playwright", "openai", "anthropic", "langchain", "python-dotenv", "dateutil",
            "python-dateutil", "pytz", "arrow", "pendulum", "docker", "kubernetes", "grpcio", "protobuf", "websockets", "pyzmq",
            "jupyter", "ipython", "notebook", "polars", "pyarrow", "dask", "numba", "sympy", "networkx", "opencv-python",
        ],
        Eco::Crates => &[
            "serde", "serde-json", "serde-yaml", "toml", "tokio", "async-std", "futures", "reqwest", "hyper", "axum", "actix-web",
            "warp", "tower", "tower-http", "tonic", "prost", "rand", "regex", "chrono", "time", "uuid", "anyhow", "thiserror",
            "clap", "structopt", "log", "env-logger", "tracing", "tracing-subscriber", "once-cell", "lazy-static", "itertools",
            "rayon", "crossbeam", "parking-lot", "bytes", "byteorder", "base64", "hex", "sha2", "sha1", "md5", "blake3", "ring",
            "rustls", "openssl", "native-tls", "sqlx", "diesel", "rusqlite", "redis", "mongodb", "url", "percent-encoding",
            "flate2", "zip", "tar", "walkdir", "glob", "tempfile", "dirs", "libc", "nix", "bitflags", "num", "num-traits", "smallvec",
            "indexmap", "hashbrown", "ahash", "dashmap", "arc-swap", "semver", "syn", "quote", "proc-macro2", "pin-project",
            "async-trait", "tokio-util", "tokio-stream", "image", "wasm-bindgen", "web-sys", "js-sys", "criterion", "proptest",
            "insta", "mockall", "zeroize", "argon2", "bcrypt", "jsonwebtoken", "minijinja", "tera", "handlebars", "pulldown-cmark",
        ],
    }
}

/// Optimal string alignment distance, stopping early once it passes `max`.
fn osa(a: &str, b: &str, max: usize) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > max {
        return max + 1;
    }
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    d[a.len()][b.len()]
}

/// The popular name `name` is one letter off, if any. Popular names, the repo's own names and
/// names under 4 letters never count.
pub fn near_popular(eco: Eco, name: &str, own: &BTreeSet<String>) -> Option<&'static str> {
    let n = eco.norm(name);
    let list = popular(eco);
    if n.chars().count() < 4 || own.contains(&n) || list.iter().any(|p| eco.norm(p) == n) {
        return None;
    }
    // npm scopes: compare the bare name too (`@evil/lodahs`)
    let bare = n.rsplit('/').next().unwrap_or(&n).to_string();
    list.iter().copied().find(|p| {
        let p = eco.norm(p);
        osa(&n, &p, 1) == 1 || (bare != n && bare.chars().count() >= 4 && osa(&bare, &p, 1) == 1)
    })
}

/// A name with characters no registry accepts, such as look-alike letters from other alphabets.
fn odd_chars(name: &str) -> bool {
    !name.is_ascii() || name.chars().any(|c| c.is_control() || c.is_whitespace())
}

// ---------------------------------------------------------------------------------------------
// Looking names up

/// The registries' addresses and limits.
#[derive(Debug, Clone)]
pub struct Registries {
    pub npm: String,
    pub npm_downloads: String,
    pub pypi: String,
    pub pypistats: String,
    pub crates: String,
    /// Per request.
    pub timeout: Duration,
    /// For the whole check.
    pub budget: Duration,
    /// Age is measured from this; none means now.
    pub now: Option<DateTime<Utc>>,
}

impl Default for Registries {
    fn default() -> Self {
        Registries {
            npm: "https://registry.npmjs.org".into(),
            npm_downloads: "https://api.npmjs.org".into(),
            pypi: "https://pypi.org".into(),
            pypistats: "https://pypistats.org".into(),
            crates: "https://crates.io".into(),
            timeout: Duration::from_secs(10),
            budget: Duration::from_secs(60),
            now: None,
        }
    }
}

impl Registries {
    /// Every registry under one base (`/npm`, `/npm-downloads`, `/pypi`, `/pypistats`, `/crates`).
    pub fn all_at(base: &str) -> Registries {
        Registries {
            npm: format!("{base}/npm"),
            npm_downloads: format!("{base}/npm-downloads"),
            pypi: format!("{base}/pypi"),
            pypistats: format!("{base}/pypistats"),
            crates: format!("{base}/crates"),
            ..Registries::default()
        }
    }
}

/// Percent-encodes every byte but ASCII letters, digits, `-`, `_`, `.` and `~`, so a name is
/// always one path segment (`../x` → `..%2Fx`).
pub fn enc(name: &str) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');
    percent_encoding::utf8_percent_encode(name, SET).to_string()
}

/// The registry path segment for a name, or none when it can't be a package name at all.
pub fn segment(eco: Eco, name: &str) -> Option<String> {
    if name.is_empty() || name.chars().all(|c| c == '.') {
        return None;
    }
    if eco == Eco::Npm {
        if let Some(rest) = name.strip_prefix('@') {
            let (scope, bare) = rest.split_once('/')?;
            if scope.is_empty() || bare.is_empty() || bare.chars().all(|c| c == '.') || scope.chars().all(|c| c == '.') {
                return None;
            }
            return Some(format!("@{}%2F{}", enc(scope), enc(bare)));
        }
    }
    let n = if eco == Eco::PyPi { eco.norm(name) } else { name.to_string() };
    (!n.is_empty()).then(|| enc(&n))
}

/// A registry's answer about one name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Looked {
    Missing,
    Found { created: Option<DateTime<Utc>>, downloads: Option<u64> },
    /// 5xx, 429, a timeout or anything else unreadable: why.
    Unchecked(String),
}

enum Got {
    Json(serde_json::Value),
    NotFound,
    Failed(String),
}

async fn get(http: &reqwest::Client, url: &str, what: &str) -> Got {
    let res = match http.get(url).send().await {
        Ok(r) => r,
        Err(e) if e.is_timeout() => return Got::Failed(format!("{what} timed out")),
        Err(_) => return Got::Failed(format!("{what} couldn't be reached")),
    };
    match res.status().as_u16() {
        200 => {}
        404 | 410 => return Got::NotFound,
        429 => return Got::Failed(format!("{what} is rate limiting (429)")),
        s => return Got::Failed(format!("{what} answered {s}")),
    }
    let mut body = Vec::new();
    let mut res = res;
    loop {
        match res.chunk().await {
            Ok(Some(c)) if body.len() + c.len() <= 16 << 20 => body.extend_from_slice(&c),
            Ok(Some(_)) => return Got::Failed(format!("{what} sent too much")),
            Ok(None) => break,
            Err(e) if e.is_timeout() => return Got::Failed(format!("{what} timed out")),
            Err(_) => return Got::Failed(format!("{what} stopped answering")),
        }
    }
    match serde_json::from_slice(&body) {
        Ok(v) => Got::Json(v),
        Err(_) => Got::Failed(format!("{what} sent an answer kwant couldn't read")),
    }
}

fn date(v: &serde_json::Value) -> Option<DateTime<Utc>> {
    v.as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|d| d.with_timezone(&Utc))
}

/// Looks one name up: whether it exists, when it was first published and its downloads.
pub async fn look_up(http: &reqwest::Client, r: &Registries, eco: Eco, name: &str) -> Looked {
    let Some(seg) = segment(eco, name) else { return Looked::Missing };
    let what = eco.label();
    match eco {
        Eco::Npm => {
            let doc = match get(http, &format!("{}/{seg}", r.npm), what).await {
                Got::Json(v) => v,
                Got::NotFound => return Looked::Missing,
                Got::Failed(e) => return Looked::Unchecked(e),
            };
            let created = date(&doc["time"]["created"]);
            let downloads = match get(http, &format!("{}/downloads/point/last-week/{seg}", r.npm_downloads), "npm's download counts").await {
                Got::Json(v) => v["downloads"].as_u64(),
                Got::NotFound => Some(0),
                Got::Failed(e) => return Looked::Unchecked(e),
            };
            Looked::Found { created, downloads }
        }
        Eco::PyPi => {
            let doc = match get(http, &format!("{}/pypi/{seg}/json", r.pypi), what).await {
                Got::Json(v) => v,
                Got::NotFound => return Looked::Missing,
                Got::Failed(e) => return Looked::Unchecked(e),
            };
            let created = doc["releases"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.values())
                .flat_map(|files| files.as_array().into_iter().flatten())
                .filter_map(|f| date(&f["upload_time_iso_8601"]))
                .min();
            let downloads = match get(http, &format!("{}/api/packages/{seg}/recent", r.pypistats), "pypistats").await {
                Got::Json(v) => v["data"]["last_week"].as_u64(),
                Got::NotFound => Some(0),
                Got::Failed(e) => return Looked::Unchecked(e),
            };
            Looked::Found { created, downloads }
        }
        Eco::Crates => {
            let doc = match get(http, &format!("{}/api/v1/crates/{seg}", r.crates), what).await {
                Got::Json(v) => v,
                Got::NotFound => return Looked::Missing,
                Got::Failed(e) => return Looked::Unchecked(e),
            };
            Looked::Found { created: date(&doc["crate"]["created_at"]), downloads: doc["crate"]["recent_downloads"].as_u64() }
        }
    }
}

/// Why one found package is flagged, if it is.
pub fn concerns(eco: Eco, created: Option<DateTime<Utc>>, downloads: Option<u64>, now: DateTime<Utc>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(c) = created {
        let days = (now - c).num_days();
        if days < NEW_DAYS {
            out.push(format!("first published {} ago", if days <= 0 { "today".to_string() } else { format!("{days} days") }));
        }
    }
    let few = match eco {
        Eco::Crates => FEW_RECENT_CRATES,
        _ => FEW_WEEKLY,
    };
    if let Some(d) = downloads {
        if d < few {
            out.push(match eco {
                Eco::Crates => format!("only {d} downloads in the last 90 days"),
                _ => format!("only {d} downloads last week"),
            });
        }
    }
    out
}

/// Checks every new package. The tile is `hit` when anything is flagged, `not_run` when anything
/// couldn't be checked, else `pass`.
pub async fn run(r: &Registries, files: &[FileDiff], contents: &BTreeMap<String, String>) -> (Tile, Vec<Finding>) {
    let a = added(files, contents);
    let mut stopped = a.skipped.clone();
    let mut flagged: Vec<(NewDep, &'static str, String)> = Vec::new(); // (dep, severity, why)
    let mut missing = 0u32;
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(r.timeout)
        .connect_timeout(r.timeout)
        .user_agent("kwant-dev-scan (https://kwant.si)")
        .build();
    let Ok(http) = http else { return (Tile::not_run("the package check couldn't start"), vec![]) };
    let now = r.now.unwrap_or_else(Utc::now);
    let deadline = tokio::time::Instant::now() + r.budget;
    let mut looked = 0usize;
    let mut unchecked = Vec::new();
    for dep in &a.deps {
        let own = a.own.get(&dep.eco).cloned().unwrap_or_default();
        if own.contains(&dep.eco.norm(&dep.name)) || popular(dep.eco).iter().any(|p| dep.eco.norm(p) == dep.eco.norm(&dep.name)) {
            continue;
        }
        let near = near_popular(dep.eco, &dep.name, &own);
        let odd = odd_chars(&dep.name);
        let result = if looked >= MAX_LOOKUPS {
            Looked::Unchecked(format!("more than {MAX_LOOKUPS} new packages"))
        } else {
            looked += 1;
            match tokio::time::timeout_at(deadline, look_up(&http, r, dep.eco, &dep.name)).await {
                Ok(l) => l,
                Err(_) => Looked::Unchecked(format!("the package check ran past {} seconds", r.budget.as_secs())),
            }
        };
        let mut why = Vec::new();
        let mut sev = "medium";
        match &result {
            Looked::Missing => {
                missing += 1;
                sev = "critical";
                why.push(format!("doesn't exist on {}", dep.eco.label()));
            }
            Looked::Found { created, downloads } => why.extend(concerns(dep.eco, *created, *downloads, now)),
            Looked::Unchecked(e) => unchecked.push(format!("{} ({e})", as_data(&dep.name))),
        }
        if odd {
            sev = if sev == "critical" { sev } else { "high" };
            why.push("has characters from outside plain ASCII, like look-alike letters".into());
        }
        if let Some(p) = near {
            sev = if sev == "critical" { sev } else { "high" };
            why.push(format!("is one letter off the popular {p}"));
        }
        if !why.is_empty() {
            flagged.push((dep.clone(), sev, why.join(", ")));
        }
    }
    if !unchecked.is_empty() {
        stopped.push(format!("{} not checked: {}", if unchecked.len() == 1 { "1 package" } else { "some packages" }, unchecked.join("; ")));
    }
    let findings = flagged
        .iter()
        .take(MAX_FINDINGS)
        .enumerate()
        .map(|(i, (d, sev, why))| Finding {
            id: format!("p{}", i + 1),
            severity: (*sev).into(),
            title: format!("New {} package {}: {}", d.eco.label(), as_data(&d.name), first_reason(why)),
            file: d.file.clone(),
            line: d.line,
            detail: format!("The PR adds {} from {}, which {why}.", as_data(&d.name), d.eco.label()),
            fix: if *sev == "critical" {
                "Check the name against the package you meant; a package that doesn't exist yet can be registered by anyone.".into()
            } else {
                "Check this is the package you meant and that its publisher is who you expect, or use a well-known one.".into()
            },
            source: "packages".into(),
        })
        .collect();
    (Tile::of(flagged.len() as u32, missing, &stopped), findings)
}

fn first_reason(why: &str) -> String {
    why.split(", ").next().unwrap_or(why).to_string()
}
