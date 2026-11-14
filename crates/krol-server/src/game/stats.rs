//! Player statistics, `<world>/stats/<uuid>.json` (vanilla: `StatisticsFile`).
//!
//! Stats live in a `ConcurrentHashMap` keyed by stat id and are written in its iteration
//! order as compact Gson. Achievements with progress (e.g. `exploreAllBiomes`) are
//! written as `{"value":v,"progress":[...]}`, the progress being a `HashSet`.

use std::path::{Path, PathBuf};

use krol_java::gson::JsonValue;
use krol_java::{JavaConcurrentHashMap, JavaHashMap};
use tracing::warn;

use crate::status::java_uuid_string;

/// One stat value.
#[derive(Debug, Clone, Default)]
pub struct StatValue {
    /// The counter.
    pub value: i32,
    /// Progress set for achievements that track one (insertion-faithful `HashSet`).
    pub progress: Option<JavaHashMap<()>>,
}

/// A player's statistics.
#[derive(Debug)]
pub struct Stats {
    path: PathBuf,
    data: JavaConcurrentHashMap<StatValue>,
}

impl Stats {
    /// Loads `<world>/stats/<uuid>.json` (missing or unreadable: empty).
    pub fn load(world_dir: &Path, uuid: u128) -> Self {
        let path = world_dir
            .join("stats")
            .join(format!("{}.json", java_uuid_string(uuid)));
        let mut data = JavaConcurrentHashMap::with_guava_defaults();
        if let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(JsonValue::Object(members)) = JsonValue::parse(&text)
        {
            for (key, value) in members {
                let stat = match &value {
                    JsonValue::Int(v) => StatValue {
                        value: *v as i32,
                        progress: None,
                    },
                    JsonValue::Object(_) => {
                        let mut set = JavaHashMap::new();
                        if let Some(JsonValue::Array(items)) = value.get("progress") {
                            for item in items {
                                if let Some(s) = item.as_str() {
                                    set.insert(s.to_owned(), ());
                                }
                            }
                        }
                        StatValue {
                            value: value.get("value").and_then(JsonValue::as_i64).unwrap_or(0)
                                as i32,
                            progress: Some(set),
                        }
                    }
                    _ => continue,
                };
                data.insert(key, stat);
            }
        }
        Self { path, data }
    }

    /// Adds `amount` to a counter (vanilla `addStat` / `triggerAchievement`).
    pub fn add(&mut self, key: &str, amount: i32) {
        match self.data.get_mut(key) {
            Some(s) => s.value = s.value.wrapping_add(amount),
            None => {
                self.data.insert(
                    key.to_owned(),
                    StatValue {
                        value: amount,
                        progress: None,
                    },
                );
            }
        }
    }

    /// Sets a counter (vanilla `unlockAchievement` with a value, e.g. resetting
    /// `stat.timeSinceDeath`).
    pub fn set(&mut self, key: &str, value: i32) {
        match self.data.get_mut(key) {
            Some(s) => s.value = value,
            None => {
                self.data.insert(
                    key.to_owned(),
                    StatValue {
                        value,
                        progress: None,
                    },
                );
            }
        }
    }

    /// Adds an entry to an achievement's progress set, creating it if needed.
    pub fn add_progress(&mut self, key: &str, entry: &str) {
        if self.data.get(key).is_none() {
            self.data.insert(
                key.to_owned(),
                StatValue {
                    value: 0,
                    progress: Some(JavaHashMap::new()),
                },
            );
        }
        if let Some(s) = self.data.get_mut(key) {
            s.progress
                .get_or_insert_with(JavaHashMap::new)
                .insert(entry.to_owned(), ());
        }
    }

    /// Writes the file (vanilla `saveStatFile`).
    pub fn save(&self) {
        let members = self
            .data
            .iter()
            .map(|(k, v)| {
                let value = match &v.progress {
                    None => JsonValue::Int(i64::from(v.value)),
                    Some(set) => JsonValue::object().with("value", v.value).with(
                        "progress",
                        set.iter()
                            .map(|(s, ())| JsonValue::from(s.as_str()))
                            .collect::<Vec<_>>(),
                    ),
                };
                (k.clone(), value)
            })
            .collect();
        let text = JsonValue::Object(members).to_gson_string();
        let result = self
            .path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&self.path, text));
        if let Err(e) = result {
            warn!("Couldn't save stats: {e}");
        }
    }
}

/// Vanilla biome names by id (`BiomeGenBase.biomeName`), used for `exploreAllBiomes`.
pub fn biome_name(id: u8) -> &'static str {
    match id {
        0 => "Ocean",
        1 => "Plains",
        2 => "Desert",
        3 => "Extreme Hills",
        4 => "Forest",
        5 => "Taiga",
        6 => "Swampland",
        7 => "River",
        8 => "Hell",
        9 => "The End",
        10 => "FrozenOcean",
        11 => "FrozenRiver",
        12 => "Ice Plains",
        13 => "Ice Mountains",
        14 => "MushroomIsland",
        15 => "MushroomIslandShore",
        16 => "Beach",
        17 => "DesertHills",
        18 => "ForestHills",
        19 => "TaigaHills",
        20 => "Extreme Hills Edge",
        21 => "Jungle",
        22 => "JungleHills",
        23 => "JungleEdge",
        24 => "Deep Ocean",
        25 => "Stone Beach",
        26 => "Cold Beach",
        27 => "Birch Forest",
        28 => "Birch Forest Hills",
        29 => "Roofed Forest",
        30 => "Cold Taiga",
        31 => "Cold Taiga Hills",
        32 => "Mega Taiga",
        33 => "Mega Taiga Hills",
        34 => "Extreme Hills+",
        35 => "Savanna",
        36 => "Savanna Plateau",
        37 => "Mesa",
        38 => "Mesa Plateau F",
        39 => "Mesa Plateau",
        // TODO(M6): mutated biomes (128+) and ungenerated columns (255).
        _ => "Plains",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Key order and format of a vanilla 1.8.8 stats file after a short session.
    #[test]
    fn writes_like_vanilla() {
        let dir = tempfile::tempdir().unwrap();
        let mut stats = Stats::load(dir.path(), 1);
        stats.add("stat.playOneMinute", 74);
        stats.add("stat.timeSinceDeath", 74);
        stats.add_progress("achievement.exploreAllBiomes", "Plains");
        stats.add("stat.leaveGame", 1);
        stats.save();
        let text = std::fs::read_to_string(
            dir.path()
                .join("stats")
                .join("00000000-0000-0000-0000-000000000001.json"),
        )
        .unwrap();
        assert_eq!(
            text,
            r#"{"stat.playOneMinute":74,"stat.leaveGame":1,"stat.timeSinceDeath":74,"achievement.exploreAllBiomes":{"value":0,"progress":["Plains"]}}"#
        );
        let mut reloaded = Stats::load(dir.path(), 1);
        reloaded.add("stat.leaveGame", 1);
        assert_eq!(reloaded.data.get("stat.leaveGame").unwrap().value, 2);
    }
}
