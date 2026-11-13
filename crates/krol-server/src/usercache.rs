//! `usercache.json` (vanilla: `PlayerProfileCache`): known name ↔ UUID pairs, most
//! recently used first, each valid for one month.

use std::path::PathBuf;

use chrono::{DateTime, FixedOffset, Local, Months};
use krol_java::gson::JsonValue;
use tracing::warn;

use crate::status::java_uuid_string;

const DATE_FORMAT: &str = "%Y-%m-%d %H:%M:%S %z";
/// Vanilla keeps at most this many entries in the file.
const MAX_SAVED: usize = 1000;

/// One cached profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    /// Player name.
    pub name: String,
    /// Player UUID.
    pub uuid: u128,
    /// When the entry expires.
    pub expires: DateTime<FixedOffset>,
}

/// The profile cache.
#[derive(Debug)]
pub struct UserCache {
    path: PathBuf,
    /// Most recently used first.
    entries: Vec<CacheEntry>,
}

fn parse_uuid(s: &str) -> Option<u128> {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    (hex.len() == 32).then(|| u128::from_str_radix(&hex, 16).ok())?
}

impl UserCache {
    /// Loads `path` if it exists (unreadable files leave the cache empty).
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut entries = Vec::new();
        if let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(JsonValue::Array(items)) = JsonValue::parse(&text)
        {
            for item in &items {
                let entry = (|| {
                    Some(CacheEntry {
                        name: item.get("name")?.as_str()?.to_owned(),
                        uuid: parse_uuid(item.get("uuid")?.as_str()?)?,
                        expires: DateTime::parse_from_str(
                            item.get("expiresOn")?.as_str()?,
                            DATE_FORMAT,
                        )
                        .ok()?,
                    })
                })();
                if let Some(e) = entry {
                    entries.push(e);
                }
            }
        }
        Self { path, entries }
    }

    /// Looks up a name (case-insensitive), dropping it if expired, and marks it used
    /// (vanilla: `getGameProfileForUsername` for cached names).
    pub fn get(&mut self, name: &str) -> Option<(u128, String)> {
        let i = self
            .entries
            .iter()
            .position(|e| e.name.eq_ignore_ascii_case(name))?;
        if self.entries[i].expires < Local::now() {
            self.entries.remove(i);
            return None;
        }
        let entry = self.entries.remove(i);
        let result = (entry.uuid, entry.name.clone());
        self.entries.insert(0, entry);
        Some(result)
    }

    /// Adds or refreshes a profile (expires one month from now) and saves the file.
    pub fn add(&mut self, uuid: u128, name: &str) {
        self.entries
            .retain(|e| e.uuid != uuid && !e.name.eq_ignore_ascii_case(name));
        let now = Local::now().fixed_offset();
        let expires = now.checked_add_months(Months::new(1)).unwrap_or(now);
        self.entries.insert(
            0,
            CacheEntry {
                name: name.to_owned(),
                uuid,
                expires,
            },
        );
        self.save();
    }

    /// Writes the file like vanilla's (compact Gson).
    pub fn save(&self) {
        let items = self
            .entries
            .iter()
            .take(MAX_SAVED)
            .map(|e| {
                JsonValue::object()
                    .with("name", e.name.as_str())
                    .with("uuid", java_uuid_string(e.uuid))
                    .with(
                        "expiresOn",
                        e.expires
                            .with_timezone(&Local)
                            .format(DATE_FORMAT)
                            .to_string(),
                    )
            })
            .collect::<Vec<_>>();
        if let Err(e) = std::fs::write(&self.path, JsonValue::Array(items).to_gson_string()) {
            warn!("Failed to save usercache.json: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mru_order_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usercache.json");
        let mut cache = UserCache::load(&path);
        cache.add(1, "Alice");
        cache.add(2, "Bob");
        assert_eq!(cache.get("alice"), Some((1, "Alice".into())));
        cache.save();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(
            r#"[{"name":"Alice","uuid":"00000000-0000-0000-0000-000000000001","expiresOn":""#
        ));
        let mut reloaded = UserCache::load(&path);
        assert_eq!(reloaded.get("BOB"), Some((2, "Bob".into())));
    }
}
