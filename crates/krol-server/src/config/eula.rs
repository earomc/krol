//! `eula.txt` handling (vanilla: `ServerEula`).

use std::path::Path;

use krol_java::properties::{JavaProperties, platform_line_separator};
use tracing::warn;

use super::java_date_string;

/// Comment vanilla writes into a generated `eula.txt`.
pub const EULA_COMMENT: &str = "By changing the setting below to TRUE you are indicating your agreement to our EULA (https://account.mojang.com/documents/minecraft_eula).";

/// Reads `eula.txt`; `true` only if it contains `eula=true` (case-insensitive).
///
/// A missing or unreadable file logs `Failed to load <file>` and is (re)created with
/// `eula=false`, like vanilla.
pub fn has_accepted(path: &Path) -> bool {
    let loaded = std::fs::read(path)
        .ok()
        .and_then(|bytes| JavaProperties::load(&bytes).ok());
    match loaded {
        Some(props) => props
            .get("eula")
            .unwrap_or("false")
            .eq_ignore_ascii_case("true"),
        None => {
            warn!("Failed to load {}", path.display());
            create(path);
            false
        }
    }
}

/// Writes a fresh `eula.txt` containing `eula=false` (vanilla: `createEULAFile`).
pub fn create(path: &Path) {
    let mut props = JavaProperties::new();
    props.set("eula", "false");
    let bytes = props.store(
        EULA_COMMENT,
        &java_date_string(std::time::SystemTime::now()),
        platform_line_separator(),
    );
    if let Err(e) = std::fs::write(path, bytes) {
        warn!("Failed to save {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_created_and_not_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("eula.txt");
        assert!(!has_accepted(&path));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(&format!("#{EULA_COMMENT}")));
        assert!(text.trim_end().ends_with("eula=false"));
    }

    #[test]
    fn accepts_true_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("eula.txt");
        std::fs::write(&path, "eula=TRUE\n").unwrap();
        assert!(has_accepted(&path));
        std::fs::write(&path, "eula=yes\n").unwrap();
        assert!(!has_accepted(&path));
    }
}
