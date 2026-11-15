//! The vanilla parity suite (see `AGENTS.md` §10.2).

mod agent;
mod diff;
mod observe;
mod play;
mod scenario;
mod server;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use diff::Difference;
use server::Target;

/// Maximum number of differences stored per scenario in the report.
const MAX_DIFFS: usize = 50;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the repository root")
        .to_path_buf()
}

fn parity_root() -> PathBuf {
    repo_root().join("parity")
}

fn work_dir(scenario: &str, target: &str) -> PathBuf {
    repo_root()
        .join("target")
        .join("parity")
        .join(scenario)
        .join(target)
}

pub fn main(args: &[String]) -> Result<ExitCode> {
    let filter = flag_value(args, "--filter");
    match args.first().map(String::as_str) {
        Some("run") => {
            let protocol = flag_value(args, "--protocol")
                .map(|p| p.parse::<i32>())
                .transpose()
                .context("--protocol takes 47 or 5")?;
            let jobs = jobs(args, DEFAULT_RUN_JOBS)?;
            run(filter.as_deref(), protocol == Some(5), jobs)
        }
        Some("record") => {
            let jobs = jobs(args, DEFAULT_RECORD_JOBS)?;
            record(
                filter.as_deref(),
                args.iter().any(|a| a == "--rerecord"),
                jobs,
            )
        }
        Some("diff") => show_diff(args.get(1).context("usage: parity diff <scenario>")?),
        Some("coverage") => coverage(),
        _ => bail!(
            "usage: cargo xtask parity <run|record|diff|coverage> [--filter <substr>] [--protocol 5] [--jobs N]"
        ),
    }
}

/// Scenarios run at once by `parity run` unless `--jobs` says otherwise. Scenarios spend
/// most of their time waiting for scripted bot sessions, and a krol server needs little
/// CPU or memory, so a few at once cut the wall time without disturbing tick timing.
const DEFAULT_RUN_JOBS: usize = 4;
/// `parity record` starts a JVM per scenario (vanilla's default heap is a quarter of the
/// RAM), so it stays sequential unless asked.
const DEFAULT_RECORD_JOBS: usize = 1;

/// `--jobs N` / `-j N`, capped at the number of CPUs.
fn jobs(args: &[String], default: usize) -> Result<usize> {
    let value = flag_value(args, "--jobs").or_else(|| flag_value(args, "-j"));
    let jobs = match value {
        Some(v) => v
            .parse::<usize>()
            .context("--jobs takes a positive number")?,
        None => default,
    };
    let cpus = std::thread::available_parallelism().map_or(1, usize::from);
    Ok(jobs.clamp(1, cpus))
}

/// Runs `f` on every item with up to `jobs` worker threads; the results keep the order
/// of the items.
fn parallel_map<T: Sync, R: Send>(items: &[T], jobs: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let results: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..jobs.min(items.len()) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let r = f(item);
                    if let Ok(mut slot) = results[i].lock() {
                        *slot = Some(r);
                    }
                }
            });
        }
    });
    results
        .into_iter()
        .filter_map(|m| m.into_inner().ok().flatten())
        .collect()
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

fn vanilla_target() -> Option<Target> {
    let jar = std::env::var_os("KROL_VANILLA_JAR")?;
    Some(Target::Vanilla {
        java: std::env::var("KROL_JAVA").unwrap_or_else(|_| "java".into()),
        jar: PathBuf::from(jar),
    })
}

fn build_krol() -> Result<Target> {
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        // The parity hooks (`krolpin`) are only in builds with the `parity` feature,
        // kept in their own target directory so normal builds do not rebuild.
        .args([
            "build",
            "--quiet",
            "--package",
            "krol",
            "--features",
            "parity",
            "--target-dir",
            "target/parity-build",
        ])
        .current_dir(repo_root())
        .status()
        .context("running cargo build")?;
    if !status.success() {
        bail!("building krol failed");
    }
    let exe = repo_root()
        .join("target")
        .join("parity-build")
        .join("debug")
        .join(format!("krol{}", std::env::consts::EXE_SUFFIX));
    Ok(Target::Krol(exe))
}

#[derive(Debug, Serialize, Deserialize)]
struct Golden {
    recorded_with: String,
    observations: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct ScenarioReport {
    name: String,
    description: String,
    subsystem: String,
    /// `pass`, `fail`, `no_golden` or `error`.
    status: String,
    /// Where the expected observations came from (`golden` or `vanilla-live`).
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_divergence: Option<String>,
    #[serde(default)]
    diff_count: usize,
    #[serde(default)]
    diffs: Vec<DiffEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// Path of the full krol observations for inspection.
    #[serde(skip_serializing_if = "Option::is_none")]
    actual_observations: Option<String>,
    /// Failed while other scenarios ran at the same time and passed when run alone: a
    /// timing-sensitive scenario worth making robust.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    retried_alone: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct DiffEntry {
    path: String,
    expected: Value,
    actual: Value,
}

impl From<Difference> for DiffEntry {
    fn from(d: Difference) -> Self {
        Self {
            path: d.path,
            expected: d.expected,
            actual: d.actual,
        }
    }
}

/// Runs the scenarios. With `v5`, only scenarios with play sessions run, every session
/// connects as a 1.7 client (protocol 5), and both the vanilla recording and krol's
/// observation are reduced to what a 1.7 client can observe before comparing.
fn run(filter: Option<&str>, v5: bool, jobs: usize) -> Result<ExitCode> {
    let mut scenarios = scenario::load_all(&parity_root().join("scenarios"), filter)?;
    if v5 {
        scenarios.retain(|sc| {
            sc.steps
                .iter()
                .any(|s| matches!(s, scenario::Step::Play { .. }))
        });
        for sc in &mut scenarios {
            for step in &mut sc.steps {
                if let scenario::Step::Play { protocol, .. } = step {
                    *protocol = 5;
                }
            }
        }
    }
    if scenarios.is_empty() {
        bail!("no scenarios matched");
    }
    let krol = build_krol()?;
    let vanilla = vanilla_target();
    let run_logged = |sc: &scenario::Scenario| {
        eprintln!("parity: {} ...", sc.name);
        let report = match run_one(sc, &krol, vanilla.as_ref(), v5) {
            Ok(r) => r,
            Err(e) => ScenarioReport {
                name: sc.name.clone(),
                description: sc.description.clone(),
                subsystem: sc.subsystem.clone(),
                status: "error".into(),
                expected_source: None,
                first_divergence: None,
                diff_count: 0,
                diffs: Vec::new(),
                error: Some(format!("{e:#}")),
                actual_observations: None,
                retried_alone: false,
            },
        };
        eprintln!("parity: {} {}", sc.name, report.status);
        report
    };
    let mut reports = parallel_map(&scenarios, jobs, run_logged);
    // Scenarios whose bot timing races with the tick fail more often under load; give
    // each failure one run on its own and keep the first result if that fails too.
    if jobs > 1 {
        for (sc, report) in scenarios.iter().zip(reports.iter_mut()) {
            if report.status == "fail" {
                let alone = run_logged(sc);
                if alone.status == "pass" {
                    *report = ScenarioReport {
                        retried_alone: true,
                        ..alone
                    };
                }
            }
        }
    }

    let summary = summarize(&reports);
    let report = json!({ "summary": summary, "scenarios": reports });
    // The 1.7 replay keeps its own report so both survive a CI run.
    let path = parity_root().join(if v5 { "report-v5.json" } else { "report.json" });
    std::fs::write(&path, serde_json::to_string_pretty(&report)? + "\n")?;

    println!();
    for r in &reports {
        let extra = match r.status.as_str() {
            "fail" => format!(
                " ({} diffs, first at {})",
                r.diff_count,
                r.first_divergence.as_deref().unwrap_or("?")
            ),
            "error" => format!(" ({})", r.error.as_deref().unwrap_or("?")),
            _ if r.retried_alone => " (failed in parallel, passed alone)".to_owned(),
            _ => String::new(),
        };
        println!(
            "{:<9} {} [{}]{}",
            r.status.to_uppercase(),
            r.name,
            r.subsystem,
            extra
        );
    }
    println!("\n{summary}");
    println!("report: {}", path.display());
    let failed = reports
        .iter()
        .any(|r| r.status == "fail" || r.status == "error");
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn summarize(reports: &[ScenarioReport]) -> String {
    let count = |s: &str| reports.iter().filter(|r| r.status == s).count();
    let retried = reports.iter().filter(|r| r.retried_alone).count();
    format!(
        "{} scenarios: {} pass ({} only alone), {} fail, {} error, {} no_golden",
        reports.len(),
        count("pass"),
        retried,
        count("fail"),
        count("error"),
        count("no_golden")
    )
}

fn golden_path(name: &str) -> PathBuf {
    parity_root().join("golden").join(format!("{name}.json"))
}

fn run_one(
    sc: &scenario::Scenario,
    krol: &Target,
    vanilla: Option<&Target>,
    v5: bool,
) -> Result<ScenarioReport> {
    let dir = work_dir(&sc.name, if v5 { "krol-v5" } else { "krol" });
    let mut actual = observe::run(sc, krol, &dir, &parity_root())?;
    if v5 {
        actual = play::v5_view_all(&actual, true);
    }
    let actual_path = dir.with_extension("observations.json");
    std::fs::write(&actual_path, serde_json::to_string_pretty(&actual)?)?;

    let (expected, source) = if let Ok(text) = std::fs::read_to_string(golden_path(&sc.name)) {
        let golden: Golden = serde_json::from_str(&text)?;
        let observations = if v5 {
            play::v5_view_all(&golden.observations, false)
        } else {
            golden.observations
        };
        (Some(observations), Some("golden".to_owned()))
    } else if let Some(vanilla) = vanilla.filter(|_| !v5) {
        let vdir = work_dir(&sc.name, "vanilla");
        (
            Some(observe::run(sc, vanilla, &vdir, &parity_root())?),
            Some("vanilla-live".to_owned()),
        )
    } else {
        (None, None)
    };

    let mut report = ScenarioReport {
        name: sc.name.clone(),
        description: sc.description.clone(),
        subsystem: sc.subsystem.clone(),
        status: "no_golden".into(),
        expected_source: source,
        first_divergence: None,
        diff_count: 0,
        diffs: Vec::new(),
        error: None,
        actual_observations: Some(actual_path.display().to_string()),
        retried_alone: false,
    };
    if let Some(mut expected) = expected {
        let mut actual = actual;
        for group in &sc.observe.unordered {
            diff::canonicalize_unordered(&mut expected, group);
            diff::canonicalize_unordered(&mut actual, group);
        }
        let diffs = diff::diff(
            &expected,
            &actual,
            &sc.observe.ignore,
            &sc.observe.tolerance,
        );
        report.status = if diffs.is_empty() { "pass" } else { "fail" }.into();
        report.first_divergence = diffs.first().map(|d| d.path.clone());
        report.diff_count = diffs.len();
        report.diffs = diffs
            .into_iter()
            .take(MAX_DIFFS)
            .map(DiffEntry::from)
            .collect();
    }
    Ok(report)
}

fn record(filter: Option<&str>, rerecord: bool, jobs: usize) -> Result<ExitCode> {
    let vanilla = vanilla_target().context(
        "set KROL_VANILLA_JAR to the vanilla 1.8.8 server jar (and optionally KROL_JAVA)",
    )?;
    let scenarios = scenario::load_all(&parity_root().join("scenarios"), filter)?;
    let results = parallel_map(&scenarios, jobs, |sc| -> Result<()> {
        let path = golden_path(&sc.name);
        if path.exists() && !rerecord {
            println!("skip     {} (golden exists; use --rerecord)", sc.name);
            return Ok(());
        }
        let observations =
            observe::run(sc, &vanilla, &work_dir(&sc.name, "vanilla"), &parity_root())
                .with_context(|| format!("recording {}", sc.name))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let golden = Golden {
            recorded_with: "vanilla 1.8.8".into(),
            observations,
        };
        std::fs::write(&path, serde_json::to_string_pretty(&golden)? + "\n")?;
        println!("recorded {}", sc.name);
        Ok(())
    });
    let mut failed = false;
    for e in results.into_iter().filter_map(Result::err) {
        eprintln!("error: {e:#}");
        failed = true;
    }
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn show_diff(name: &str) -> Result<ExitCode> {
    let text = std::fs::read_to_string(parity_root().join("report.json"))
        .context("no parity/report.json; run `cargo xtask parity run` first")?;
    let report: Value = serde_json::from_str(&text)?;
    let scenario = report["scenarios"]
        .as_array()
        .and_then(|s| s.iter().find(|s| s["name"] == name))
        .with_context(|| format!("scenario {name} not in the last report"))?;
    println!("{}", serde_json::to_string_pretty(scenario)?);
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, Deserialize)]
struct CoverageFile {
    #[serde(default)]
    feature: Vec<Feature>,
}

#[derive(Debug, Deserialize)]
struct Feature {
    id: String,
    description: String,
    #[serde(default)]
    scenarios: Vec<String>,
}

fn coverage() -> Result<ExitCode> {
    let text = std::fs::read_to_string(parity_root().join("coverage.toml"))?;
    let file: CoverageFile = toml::from_str(&text)?;
    let report: Value = std::fs::read_to_string(parity_root().join("report.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    let status_of = |name: &str| -> String {
        report["scenarios"]
            .as_array()
            .and_then(|s| s.iter().find(|s| s["name"] == name))
            .and_then(|s| s["status"].as_str())
            .unwrap_or("not_run")
            .to_owned()
    };
    let (mut covered, mut failing, mut uncovered) = (0, 0, 0);
    for f in &file.feature {
        let state = if f.scenarios.is_empty() {
            uncovered += 1;
            "UNCOVERED".to_owned()
        } else {
            let statuses: Vec<String> = f.scenarios.iter().map(|s| status_of(s)).collect();
            if statuses.iter().all(|s| s == "pass") {
                covered += 1;
                "COVERED".to_owned()
            } else {
                failing += 1;
                format!("OPEN ({})", statuses.join(", "))
            }
        };
        println!("{state:<24} {:<40} {}", f.id, f.description);
    }
    println!(
        "\n{} features: {covered} covered, {failing} open, {uncovered} uncovered",
        file.feature.len()
    );
    Ok(ExitCode::SUCCESS)
}
