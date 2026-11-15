//! Building `parity/vanilla-agent` (a Java agent for vanilla, see its source) into
//! `target/parity/vanilla-agent/krol-agent.jar` with the JDK's `javac` and `jar`
//! (`KROL_JAVAC` and `KROL_JAR_TOOL` override them).

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

use super::repo_root;

/// The agent's source file.
const SOURCE: &str = "parity/vanilla-agent/KrolAgent.java";

/// The agent jar, built when missing or older than its source.
pub fn jar() -> Result<PathBuf> {
    let root = repo_root();
    let source = root.join(SOURCE);
    let out = root.join("target").join("parity").join("vanilla-agent");
    let jar = out.join("krol-agent.jar");
    let modified = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    if let (Some(j), Some(s)) = (modified(&jar), modified(&source))
        && j >= s
    {
        return Ok(jar);
    }
    let classes = out.join("classes");
    std::fs::create_dir_all(&classes)?;
    let javac = std::env::var("KROL_JAVAC").unwrap_or_else(|_| "javac".into());
    let status = Command::new(&javac)
        .args(["--release", "8", "-Xlint:-options", "-d"])
        .arg(&classes)
        .arg(&source)
        .status()
        .with_context(|| format!("running {javac} (set KROL_JAVAC to a JDK's javac)"))?;
    if !status.success() {
        bail!("compiling the vanilla agent failed");
    }
    let manifest = out.join("manifest.txt");
    std::fs::write(&manifest, "Premain-Class: KrolAgent\n")?;
    let tool = std::env::var("KROL_JAR_TOOL").unwrap_or_else(|_| "jar".into());
    let status = Command::new(&tool)
        .arg("cfm")
        .arg(&jar)
        .arg(&manifest)
        .arg("-C")
        .arg(&classes)
        .arg(".")
        .status()
        .with_context(|| format!("running {tool} (set KROL_JAR_TOOL to a JDK's jar)"))?;
    if !status.success() {
        bail!("packaging the vanilla agent failed");
    }
    Ok(jar)
}
