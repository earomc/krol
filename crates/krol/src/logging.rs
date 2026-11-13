//! Vanilla-compatible logging (vanilla: log4j2 with Mojang's `log4j2.xml`).
//!
//! Console and `logs/latest.log` both use the pattern
//! `[HH:mm:ss] [<thread>/<LEVEL>]: <message>`. At startup an existing `latest.log` is
//! compressed to `logs/<yyyy-MM-dd>-<n>.log.gz` (log4j's `OnStartupTriggeringPolicy`).
//!
//! The thread name defaults to `Server thread`; events can override it with a `thread`
//! field, e.g. `info!(thread = "User Authenticator #1", "...")`, to match the vanilla
//! thread that logs the same message.

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Local};
use flate2::Compression;
use flate2::write::GzEncoder;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};

/// Installs the global logger writing to stdout and `logs/latest.log` under `dir`.
///
/// # Errors
///
/// Fails if the log directory or file cannot be prepared.
pub fn init(dir: &Path) -> io::Result<()> {
    let logs = dir.join("logs");
    fs::create_dir_all(&logs)?;
    let latest = logs.join("latest.log");
    roll_over(&latest, &logs)?;
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&latest)?;
    let layer = VanillaLayer {
        file: Mutex::new(file),
    };
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::set_global_default(subscriber).map_err(|e| io::Error::other(e.to_string()))
}

/// Compresses a non-empty `latest.log` to the first free `<date>-<n>.log.gz`.
fn roll_over(latest: &Path, logs: &Path) -> io::Result<()> {
    let Ok(meta) = fs::metadata(latest) else {
        return Ok(());
    };
    if meta.len() == 0 {
        return Ok(());
    }
    let date: DateTime<Local> = meta.modified()?.into();
    let day = date.format("%Y-%m-%d").to_string();
    let target: PathBuf = (1..)
        .map(|i| logs.join(format!("{day}-{i}.log.gz")))
        .find(|p| !p.exists())
        .expect("unbounded range");
    let data = fs::read(latest)?;
    let mut encoder = GzEncoder::new(File::create(target)?, Compression::default());
    encoder.write_all(&data)?;
    encoder.finish()?;
    Ok(())
}

struct VanillaLayer {
    file: Mutex<File>,
}

#[derive(Default)]
struct Fields {
    message: String,
    thread: Option<String>,
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message.push_str(value),
            "thread" => self.thread = Some(value.to_owned()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => {
                let _ = write!(self.message, "{value:?}");
            }
            "thread" => self.thread = Some(format!("{value:?}")),
            _ => {}
        }
    }
}

impl<S: Subscriber> Layer<S> for VanillaLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let level = *event.metadata().level();
        // Vanilla's root logger level is INFO.
        if level > Level::INFO {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        let level_name = match level {
            Level::ERROR => "ERROR",
            Level::WARN => "WARN",
            _ => "INFO",
        };
        let line = format!(
            "[{}] [{}/{}]: {}\n",
            Local::now().format("%H:%M:%S"),
            fields.thread.as_deref().unwrap_or("Server thread"),
            level_name,
            fields.message
        );
        let _ = io::stdout().lock().write_all(line.as_bytes());
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(line.as_bytes());
        }
    }
}
