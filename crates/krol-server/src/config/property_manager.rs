//! Port of vanilla's `PropertyManager`: lazy defaults written back to disk.
//!
//! Vanilla semantics reproduced here:
//! - A missing file logs `<file> does not exist` and `Generating new properties file`,
//!   then writes an empty properties file.
//! - Every getter whose key is missing inserts the default **and saves the whole file**.
//!   The final file therefore contains every key read so far, in `Hashtable` order.
//! - An unparsable int/long is replaced by the default and saved; booleans never fail
//!   (anything but `true`, case-insensitively, is `false`) and are left as written.
//! - [`set`](PropertyManager::set) changes the value in memory only; it reaches disk with
//!   the next save.
//!
//! Vanilla quirk (emergent): `Properties.load` re-inserts keys in file order, so keys that
//! share a hash bucket (e.g. `motd` and `enable-rcon` in a default file) swap places on
//! every restart.

use std::path::{Path, PathBuf};

use krol_java::properties::{JavaProperties, platform_line_separator};
use tracing::{info, warn};

use super::java_date_string;

/// The comment vanilla writes on the first line of `server.properties`.
pub const COMMENT: &str = "Minecraft server properties";

/// A properties file with vanilla's read-default-and-save behavior.
#[derive(Debug)]
pub struct PropertyManager {
    path: PathBuf,
    props: JavaProperties,
}

impl PropertyManager {
    /// Opens `path`, creating it like vanilla if it does not exist.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut manager = Self {
            path,
            props: JavaProperties::new(),
        };
        if manager.path.exists() {
            match std::fs::read(&manager.path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| JavaProperties::load(&bytes).map_err(|e| e.to_string()))
            {
                Ok(props) => manager.props = props,
                Err(_) => {
                    warn!("Failed to load {}", manager.path.display());
                    manager.generate_new();
                }
            }
        } else {
            warn!("{} does not exist", manager.path.display());
            manager.generate_new();
        }
        manager
    }

    fn generate_new(&mut self) {
        info!("Generating new properties file");
        self.save();
    }

    /// The file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes the file (vanilla: `saveProperties`). Failures are logged, not returned.
    pub fn save(&self) {
        let bytes = self.props.store(
            COMMENT,
            &java_date_string(std::time::SystemTime::now()),
            platform_line_separator(),
        );
        if let Err(e) = std::fs::write(&self.path, bytes) {
            warn!("Failed to save {}: {e}", self.path.display());
            self.generate_new_after_failure();
        }
    }

    fn generate_new_after_failure(&self) {
        // Vanilla calls generateNewProperties() here, which tries to save again and can
        // recurse until the stack overflows; krol logs once instead.
        info!("Generating new properties file");
    }

    /// Vanilla `getStringProperty`: the value, inserting and saving `default` if missing.
    pub fn get_string(&mut self, key: &str, default: &str) -> String {
        if self.props.get(key).is_none() {
            self.props.set(key, default);
            self.save();
        }
        self.props.get(key).unwrap_or(default).to_owned()
    }

    /// Vanilla `getIntProperty`: unparsable values are replaced by `default` and saved.
    pub fn get_int(&mut self, key: &str, default: i32) -> i32 {
        let raw = self.get_string(key, &default.to_string());
        match parse_java_int(&raw) {
            Some(v) => v,
            None => {
                self.props.set(key, default.to_string());
                self.save();
                default
            }
        }
    }

    /// Vanilla `getLongProperty`: unparsable values are replaced by `default` and saved.
    pub fn get_long(&mut self, key: &str, default: i64) -> i64 {
        let raw = self.get_string(key, &default.to_string());
        match parse_java_long(&raw) {
            Some(v) => v,
            None => {
                self.props.set(key, default.to_string());
                self.save();
                default
            }
        }
    }

    /// Vanilla `getBooleanProperty`: `Boolean.parseBoolean` semantics.
    pub fn get_bool(&mut self, key: &str, default: bool) -> bool {
        self.get_string(key, if default { "true" } else { "false" })
            .eq_ignore_ascii_case("true")
    }

    /// Vanilla `setProperty`: sets the value in memory without saving.
    pub fn set(&mut self, key: &str, value: impl ToString) {
        self.props.set(key, value.to_string());
    }

    /// Whether `key` is present.
    pub fn contains(&self, key: &str) -> bool {
        self.props.get(key).is_some()
    }

    /// The underlying table.
    pub fn properties(&self) -> &JavaProperties {
        &self.props
    }
}

/// `Integer.parseInt` semantics: optional sign, ASCII digits only, range-checked.
pub fn parse_java_int(s: &str) -> Option<i32> {
    parse_java_long(s).and_then(|v| i32::try_from(v).ok())
}

/// `Long.parseLong` semantics: optional `+`/`-` sign, digits, range-checked.
///
/// Known deviation: Java also accepts non-ASCII Unicode decimal digits; krol does not.
pub fn parse_java_long(s: &str) -> Option<i64> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<i64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_number_parsing() {
        assert_eq!(parse_java_int("+12"), Some(12));
        assert_eq!(parse_java_int("-0"), Some(0));
        assert_eq!(parse_java_int(" 1"), None);
        assert_eq!(parse_java_int("1_000"), None);
        assert_eq!(parse_java_int("2147483648"), None);
        assert_eq!(parse_java_int(""), None);
        assert_eq!(parse_java_int("-"), None);
        assert_eq!(parse_java_long("-9223372036854775808"), Some(i64::MIN));
    }

    #[test]
    fn defaults_are_written_and_bad_ints_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.properties");
        std::fs::write(&path, "max-players=lots\npvp=yes\n").unwrap();
        let mut pm = PropertyManager::open(&path);
        assert_eq!(pm.get_int("max-players", 20), 20);
        assert!(!pm.get_bool("pvp", true), "parseBoolean(\"yes\") is false");
        assert_eq!(
            pm.get_string("motd", "A Minecraft Server"),
            "A Minecraft Server"
        );
        let reloaded = JavaProperties::load(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(reloaded.get("max-players"), Some("20"));
        assert_eq!(
            reloaded.get("pvp"),
            Some("yes"),
            "booleans are not rewritten"
        );
        assert_eq!(reloaded.get("motd"), Some("A Minecraft Server"));
    }

    #[test]
    fn missing_file_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.properties");
        let _pm = PropertyManager::open(&path);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("#Minecraft server properties"));
    }
}
