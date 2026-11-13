//! `level.dat` and `session.lock` (vanilla: `WorldInfo`, `SaveHandler`).
//!
//! The `Data` compound is kept whole so unknown or unmodeled keys survive a save; typed
//! accessors cover what the server uses.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use krol_nbt::{Compound, Tag};
use thiserror::Error;

/// `version` value vanilla 1.8 writes (Anvil).
pub const ANVIL_VERSION: i32 = 19133;

/// Errors loading or saving world metadata.
#[derive(Debug, Error)]
pub enum LevelError {
    /// File I/O failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// NBT could not be decoded.
    #[error(transparent)]
    Nbt(#[from] krol_nbt::Error),
    /// Another process holds the session lock (vanilla: `MinecraftException`).
    #[error("The save is being accessed from another location, aborting")]
    SessionLockLost,
}

/// The contents of `level.dat`'s `Data` compound.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelData {
    /// The raw `Data` compound.
    pub data: Compound,
}

macro_rules! accessors {
    ($($(#[$doc:meta])* $get:ident / $set:ident : $key:literal => $ty:ty, $getter:ident, $variant:ident;)*) => {
        $(
            $(#[$doc])*
            pub fn $get(&self) -> $ty {
                self.data.$getter($key)
            }

            #[doc = concat!("Sets `", $key, "`.")]
            pub fn $set(&mut self, value: $ty) {
                self.data.insert($key, Tag::$variant(value));
            }
        )*
    };
}

impl LevelData {
    accessors! {
        /// `RandomSeed`: the world seed.
        seed / set_seed: "RandomSeed" => i64, get_i64, Long;
        /// `SpawnX`.
        spawn_x / set_spawn_x: "SpawnX" => i32, get_i32, Int;
        /// `SpawnY`.
        spawn_y / set_spawn_y: "SpawnY" => i32, get_i32, Int;
        /// `SpawnZ`.
        spawn_z / set_spawn_z: "SpawnZ" => i32, get_i32, Int;
        /// `Time`: total world ticks.
        time / set_time: "Time" => i64, get_i64, Long;
        /// `DayTime`: time of day ticks.
        day_time / set_day_time: "DayTime" => i64, get_i64, Long;
        /// `GameType`: default game mode id.
        game_type / set_game_type: "GameType" => i32, get_i32, Int;
        /// `rainTime`.
        rain_time / set_rain_time: "rainTime" => i32, get_i32, Int;
        /// `thunderTime`.
        thunder_time / set_thunder_time: "thunderTime" => i32, get_i32, Int;
        /// `clearWeatherTime`.
        clear_weather_time / set_clear_weather_time: "clearWeatherTime" => i32, get_i32, Int;
        /// `LastPlayed`: Unix millis of the last save.
        last_played / set_last_played: "LastPlayed" => i64, get_i64, Long;
    }

    /// `LevelName`.
    pub fn level_name(&self) -> &str {
        self.data.get_str("LevelName")
    }

    /// `generatorName` (e.g. `default`, `flat`).
    pub fn generator_name(&self) -> &str {
        self.data.get_str("generatorName")
    }

    /// `raining`.
    pub fn raining(&self) -> bool {
        self.data.get_bool("raining")
    }

    /// `thundering`.
    pub fn thundering(&self) -> bool {
        self.data.get_bool("thundering")
    }

    /// `hardcore`.
    pub fn hardcore(&self) -> bool {
        self.data.get_bool("hardcore")
    }

    /// `Difficulty`, if stored (absent in pre-1.8 worlds).
    pub fn difficulty(&self) -> Option<i8> {
        self.data
            .contains_key("Difficulty")
            .then(|| self.data.get_i8("Difficulty"))
    }

    /// A game rule's string value from `GameRules`.
    pub fn game_rule(&self, name: &str) -> Option<&str> {
        match self.data.get_compound("GameRules")?.get(name)? {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }

    /// Loads `level.dat` from `world_dir`, falling back to `level.dat_old` like vanilla.
    /// Returns `Ok(None)` if neither exists or both are unreadable.
    ///
    /// # Errors
    ///
    /// Never for missing or corrupt files (vanilla logs and falls back); reserved for
    /// future strictness.
    pub fn load(world_dir: &Path) -> Result<Option<Self>, LevelError> {
        for name in ["level.dat", "level.dat_old"] {
            let path = world_dir.join(name);
            if !path.exists() {
                continue;
            }
            let loaded = std::fs::File::open(&path)
                .map_err(LevelError::from)
                .and_then(|f| Ok(krol_nbt::read_gzip(std::io::BufReader::new(f))?));
            if let Ok((_, root)) = loaded
                && let Some(data) = root.get_compound("Data")
            {
                return Ok(Some(Self { data: data.clone() }));
            }
        }
        Ok(None)
    }

    /// Saves to `world_dir/level.dat` with vanilla's rotation: write `level.dat_new`, move
    /// `level.dat` to `level.dat_old`, then `level.dat_new` to `level.dat`. Sets
    /// `LastPlayed` to now.
    ///
    /// # Errors
    ///
    /// I/O or encoding errors.
    pub fn save(&mut self, world_dir: &Path) -> Result<(), LevelError> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        self.set_last_played(now_ms);
        self.data.insert("version", Tag::Int(ANVIL_VERSION));
        let mut root = Compound::new();
        root.insert("Data", Tag::Compound(self.data.clone()));

        let new = world_dir.join("level.dat_new");
        let old = world_dir.join("level.dat_old");
        let current = world_dir.join("level.dat");
        krol_nbt::write_gzip(std::fs::File::create(&new)?, "", &root)?;
        if old.exists() {
            std::fs::remove_file(&old)?;
        }
        if current.exists() {
            std::fs::rename(&current, &old)?;
        }
        std::fs::rename(&new, &current)?;
        Ok(())
    }
}

/// The world's `session.lock`: the start time (Unix millis, big-endian) of the server that
/// owns the world. Vanilla checks before saving that the file still holds its own value.
#[derive(Debug)]
pub struct SessionLock {
    path: PathBuf,
    stamp: i64,
}

impl SessionLock {
    /// Writes a fresh lock into `world_dir` (creating the directory).
    ///
    /// # Errors
    ///
    /// I/O errors (vanilla: "Failed to check session lock, aborting").
    pub fn acquire(world_dir: &Path) -> Result<Self, LevelError> {
        std::fs::create_dir_all(world_dir)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let path = world_dir.join("session.lock");
        std::fs::write(&path, stamp.to_be_bytes())?;
        Ok(Self { path, stamp })
    }

    /// Verifies the lock is still ours.
    ///
    /// # Errors
    ///
    /// [`LevelError::SessionLockLost`] if another process overwrote it.
    pub fn check(&self) -> Result<(), LevelError> {
        let bytes = std::fs::read(&self.path)?;
        let current = bytes
            .get(..8)
            .and_then(|b| b.try_into().ok())
            .map(i64::from_be_bytes);
        if current == Some(self.stamp) {
            Ok(())
        } else {
            Err(LevelError::SessionLockLost)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LevelData {
        let mut data = Compound::new();
        data.insert("LevelName", Tag::String("world".into()));
        data.insert("RandomSeed", Tag::Long(-4_172_144_997_902_289_642));
        data.insert("SpawnX", Tag::Int(10));
        data.insert("Unmodeled", Tag::String("kept".into()));
        let mut rules = Compound::new();
        rules.insert("doDaylightCycle", Tag::String("true".into()));
        data.insert("GameRules", Tag::Compound(rules));
        LevelData { data }
    }

    #[test]
    fn save_rotates_and_load_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        assert!(LevelData::load(dir.path()).unwrap().is_none());
        let mut level = sample();
        level.save(dir.path()).unwrap();
        assert!(dir.path().join("level.dat").exists());
        assert!(!dir.path().join("level.dat_old").exists());
        level.set_time(500);
        level.save(dir.path()).unwrap();
        assert!(dir.path().join("level.dat_old").exists());
        assert!(!dir.path().join("level.dat_new").exists());

        let loaded = LevelData::load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.time(), 500);
        assert_eq!(loaded.seed(), -4_172_144_997_902_289_642);
        assert_eq!(loaded.data.get_str("Unmodeled"), "kept");
        assert_eq!(loaded.game_rule("doDaylightCycle"), Some("true"));
        assert_eq!(loaded.data.get_i32("version"), ANVIL_VERSION);

        // Corrupt level.dat: the previous save in level.dat_old is used.
        std::fs::write(dir.path().join("level.dat"), b"garbage").unwrap();
        let fallback = LevelData::load(dir.path()).unwrap().unwrap();
        assert_eq!(fallback.time(), 0);
    }

    #[test]
    fn session_lock_detects_takeover() {
        let dir = tempfile::tempdir().unwrap();
        let lock = SessionLock::acquire(dir.path()).unwrap();
        lock.check().unwrap();
        std::fs::write(dir.path().join("session.lock"), 1i64.to_be_bytes()).unwrap();
        assert!(matches!(lock.check(), Err(LevelError::SessionLockLost)));
    }
}
