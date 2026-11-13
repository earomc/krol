//! Launching a server (krol or vanilla) in a prepared directory.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use super::scenario::Setup;

/// Which server implementation to run.
#[derive(Debug, Clone)]
pub enum Target {
    /// The krol binary at this path.
    Krol(PathBuf),
    /// `java -jar <jar> nogui`.
    Vanilla { java: String, jar: PathBuf },
}

impl Target {
    pub fn name(&self) -> &'static str {
        match self {
            Target::Krol(_) => "krol",
            Target::Vanilla { .. } => "vanilla",
        }
    }
}

/// How long to wait for `Done (` after start.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(180);
/// How long to wait for exit after `stop`.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(60);

/// A running server.
pub struct RunningServer {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<String>,
    log: Vec<String>,
    pub addr: SocketAddr,
    /// Whether the server reached `Done (` (false if it exited during startup).
    pub started: bool,
}

/// Returns a currently free local TCP port.
pub fn free_port() -> Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// Prepares `dir` according to `setup` and returns the chosen port.
pub fn prepare_dir(dir: &Path, setup: &Setup, parity_root: &Path) -> Result<u16> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).with_context(|| format!("cleaning {}", dir.display()))?;
    }
    std::fs::create_dir_all(dir)?;
    let port = free_port()?;
    if setup.eula {
        std::fs::write(dir.join("eula.txt"), "eula=true\n")?;
    }
    if setup.write_properties {
        let mut text = format!("server-port={port}\n");
        for (k, v) in &setup.properties {
            text.push_str(&format!("{k}={v}\n"));
        }
        std::fs::write(dir.join("server.properties"), text)?;
    }
    if let Some(world) = &setup.world {
        copy_dir(&parity_root.join(world), &dir.join("world"))
            .with_context(|| format!("copying world fixture {world}"))?;
    }
    for (dest, src) in &setup.files {
        std::fs::copy(parity_root.join(src), dir.join(dest))
            .with_context(|| format!("copying fixture {src}"))?;
    }
    Ok(port)
}

fn copy_dir(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

impl RunningServer {
    /// Starts `target` in `dir` and waits until it is ready or has exited.
    pub fn start(target: &Target, dir: &Path, port: u16) -> Result<Self> {
        let mut cmd = match target {
            Target::Krol(exe) => Command::new(exe),
            Target::Vanilla { java, jar } => {
                let mut c = Command::new(java);
                c.arg("-Xmx1G").arg("-jar").arg(jar).arg("nogui");
                c
            }
        };
        let mut child = cmd
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("starting {} server", target.name()))?;
        let stdin = child.stdin.take();
        let (tx, lines) = mpsc::channel();
        for stream in [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        let mut server = Self {
            child,
            stdin,
            lines,
            log: Vec::new(),
            addr: SocketAddr::from(([127, 0, 0, 1], port)),
            started: false,
        };
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                server.kill();
                bail!("{} server did not finish starting in time", target.name());
            }
            match server
                .lines
                .recv_timeout(remaining.min(Duration::from_millis(200)))
            {
                Ok(line) => {
                    let done = line.contains("]: Done (");
                    server.log.push(line);
                    if done {
                        server.started = true;
                        return Ok(server);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if server.child.try_wait()?.is_some() {
                        server.drain();
                        return Ok(server);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    server.child.wait()?;
                    return Ok(server);
                }
            }
        }
    }

    fn drain(&mut self) {
        while let Ok(line) = self.lines.recv_timeout(Duration::from_millis(200)) {
            self.log.push(line);
        }
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Writes a line to the server console.
    pub fn console(&mut self, line: &str) {
        if let Some(stdin) = &mut self.stdin {
            let _ = stdin.write_all(
                format!(
                    "{line}
"
                )
                .as_bytes(),
            );
            let _ = stdin.flush();
        }
    }

    /// Sends `stop`, waits for exit and returns the full console log.
    pub fn stop(mut self) -> Result<Vec<String>> {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.write_all(b"stop\n");
            let _ = stdin.flush();
        }
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        loop {
            if self.child.try_wait()?.is_some() {
                break;
            }
            if Instant::now() > deadline {
                self.kill();
                break;
            }
            if let Ok(line) = self.lines.recv_timeout(Duration::from_millis(100)) {
                self.log.push(line);
            }
        }
        self.drain();
        Ok(self.log)
    }
}
