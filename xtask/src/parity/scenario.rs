//! Scenario files (`parity/scenarios/<area>/<name>.toml`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// A parity scenario.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Human-readable purpose.
    pub description: String,
    /// Subsystem tag used in reports to point agents at the right code.
    pub subsystem: String,
    #[serde(default)]
    pub setup: Setup,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub observe: Observe,
    /// Name derived from the file path (`<area>/<name>`), not part of the TOML.
    #[serde(skip)]
    pub name: String,
}

/// Server directory preparation.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setup {
    /// Write `eula.txt` with `eula=true` (default `true`).
    #[serde(default = "yes")]
    pub eula: bool,
    /// `server.properties` overrides. The runner always sets `server-port`. If empty and
    /// `write_properties` is false, no `server.properties` is created.
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
    /// Whether to write `server.properties` at all (default `true`).
    #[serde(default = "yes")]
    pub write_properties: bool,
    /// Files to copy into the server directory: destination → source (relative to
    /// `parity/`).
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

fn yes() -> bool {
    true
}

/// One scripted action; its observation is stored under `id`.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    /// Modern server list ping.
    StatusPing {
        id: String,
        protocol: i32,
        #[serde(default = "one")]
        payload: i64,
    },
    /// Raw bytes as the first chunk; observes everything until close.
    Raw { id: String, hex: String },
    /// Handshake + Login Start; observes login packets.
    LoginStart {
        id: String,
        protocol: i32,
        name: String,
    },
}

fn one() -> i64 {
    1
}

impl Step {
    pub fn id(&self) -> &str {
        match self {
            Step::StatusPing { id, .. } | Step::Raw { id, .. } | Step::LoginStart { id, .. } => id,
        }
    }
}

/// What to capture besides step results.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observe {
    /// Files read from the server directory after shutdown.
    #[serde(default)]
    pub files: Vec<String>,
    /// Capture the console log.
    #[serde(default)]
    pub log: bool,
    /// Log lines starting with any of these (after the `[time] ` prefix) are dropped on
    /// both sides, for lines whose presence legitimately differs.
    #[serde(default)]
    pub log_ignore_prefixes: Vec<String>,
    /// Observation paths excluded from comparison (known, documented deviations), e.g.
    /// `status_5.parsed.version`. Each entry should carry a comment in the TOML saying why.
    #[serde(default)]
    pub ignore: Vec<String>,
}

/// Loads all scenarios under `root`, optionally filtered by a name substring.
pub fn load_all(root: &Path, filter: Option<&str>) -> Result<Vec<Scenario>> {
    let mut paths = Vec::new();
    collect(root, &mut paths)?;
    paths.sort();
    let mut scenarios = Vec::new();
    for path in paths {
        let name = path
            .strip_prefix(root)?
            .with_extension("")
            .to_string_lossy()
            .replace('\\', "/");
        if filter.is_some_and(|f| !name.contains(f)) {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let mut scenario: Scenario =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        scenario.name = name;
        scenarios.push(scenario);
    }
    Ok(scenarios)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "toml") {
            out.push(path);
        }
    }
    Ok(())
}
