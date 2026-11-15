//! Scenario files (`parity/scenarios/<area>/<name>.toml`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// A parity scenario.
#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Default, Deserialize)]
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
    /// A world directory (relative to `parity/`) copied to `<server>/world`.
    #[serde(default)]
    pub world: Option<String>,
    /// Load `parity/vanilla-agent` into vanilla, which adds the `krolpin <seed>` console
    /// command (krol has it in `parity` builds): it seeds the world random and the
    /// random tick generator so that behavior driven by them compares exactly.
    #[serde(default)]
    pub pin_random: bool,
}

fn yes() -> bool {
    true
}

/// One scripted action; its observation is stored under `id`.
#[derive(Debug, Clone, Deserialize)]
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
    /// Joins and plays as an idle vanilla client for `seconds`, optionally chatting.
    Play {
        id: String,
        protocol: i32,
        name: String,
        #[serde(default = "three")]
        seconds: u64,
        #[serde(default)]
        chat: Vec<String>,
        /// Offsets from the spawn position, one Position packet per tick.
        #[serde(default)]
        moves: Vec<Vec<f64>>,
        /// Run concurrently with the following steps (e.g. console commands); the
        /// observation is collected at the end.
        #[serde(default)]
        background: bool,
    },
    /// Types a line into the server console; output appears in the log observation.
    /// Waits `wait_ms` (default 300) afterwards so the output is captured in order.
    Console {
        command: String,
        #[serde(default = "console_wait")]
        wait_ms: u64,
    },
    /// Sleeps; produces no observation. Use to step past timing-dependent windows
    /// (e.g. vanilla's status has no player data before the first tick).
    Wait { ms: u64 },
}

fn one() -> i64 {
    1
}

fn three() -> u64 {
    3
}

fn console_wait() -> u64 {
    300
}

impl Step {
    pub fn id(&self) -> Option<&str> {
        match self {
            Step::StatusPing { id, .. }
            | Step::Raw { id, .. }
            | Step::LoginStart { id, .. }
            | Step::Play { id, .. } => Some(id),
            Step::Wait { .. } | Step::Console { .. } => None,
        }
    }
}

/// What to capture besides step results.
#[derive(Debug, Clone, Default, Deserialize)]
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
    /// Block positions read from the saved world (`world/region`) after shutdown,
    /// reported as `id:meta`.
    #[serde(default)]
    pub blocks: Vec<[i32; 3]>,
    /// Light levels read from the saved world after shutdown, reported as
    /// `sky:block` (null where the section is missing).
    #[serde(default)]
    pub light: Vec<[i32; 3]>,
    /// Tile entities read from the saved world after shutdown, reported as NBT (or
    /// null when there is none).
    #[serde(default)]
    pub tile_entities: Vec<[i32; 3]>,
    /// Observation paths excluded from comparison (known, documented deviations), e.g.
    /// `status_5.parsed.version`. Each entry should carry a comment in the TOML saying why.
    #[serde(default)]
    pub ignore: Vec<String>,
    /// Absolute tolerances for numbers at observation paths (and below them), for
    /// values that depend on how network packets interleave with ticks. Each entry
    /// should carry a comment in the TOML saying why.
    #[serde(default)]
    pub tolerance: super::diff::Tolerances,
    /// Groups of packet types (as named in a bot's `sequence`) whose order among
    /// themselves is not compared where they follow each other directly, for packets
    /// whose order depends on which tick a console command or bot packet lands in (in
    /// vanilla too). Each group should carry a comment in the TOML saying why.
    #[serde(default)]
    pub unordered: Vec<Vec<String>>,
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
