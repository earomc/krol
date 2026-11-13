//! Typed view of `server.properties`.

use std::path::PathBuf;

use super::PropertyManager;

/// Values read from `server.properties`, with vanilla 1.8.8 keys and defaults.
///
/// [`load`](Self::load) reads every key in the order vanilla's startup reads them. The
/// order matters: missing keys are inserted in this order, which decides the order of
/// colliding keys in the rewritten file (see [`PropertyManager`]).
// The order of the getter calls below is believed to match vanilla's startup; confirm it
// with the parity scenario for a fresh server directory.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)] // field names mirror the property keys
pub struct ServerProperties {
    pub online_mode: bool,
    pub server_ip: String,
    pub spawn_animals: bool,
    pub spawn_npcs: bool,
    pub pvp: bool,
    pub allow_flight: bool,
    pub resource_pack: String,
    pub resource_pack_hash: String,
    pub motd: String,
    pub force_gamemode: bool,
    pub player_idle_timeout: i32,
    pub difficulty: i32,
    pub generate_structures: bool,
    pub gamemode: i32,
    pub server_port: i32,
    pub use_native_transport: bool,
    pub view_distance: i32,
    pub max_players: i32,
    pub white_list: bool,
    pub level_name: String,
    pub level_seed: String,
    pub level_type: String,
    pub generator_settings: String,
    pub announce_player_achievements: bool,
    pub enable_command_block: bool,
    pub op_permission_level: i32,
    pub snooper_enabled: bool,
    pub network_compression_threshold: i32,
    pub max_build_height: i32,
    pub allow_nether: bool,
    pub hardcore: bool,
    pub spawn_monsters: bool,
    pub max_world_size: i32,
    pub enable_query: bool,
    pub enable_rcon: bool,
    pub max_tick_time: i64,
}

impl ServerProperties {
    /// Opens the file at `path` and reads all values (convenience for
    /// [`PropertyManager::open`] + [`read`](Self::read)).
    pub fn load(path: impl Into<PathBuf>) -> (Self, PropertyManager) {
        let mut pm = PropertyManager::open(path);
        let props = Self::read(&mut pm);
        (props, pm)
    }

    /// Reads all values like vanilla's startup, inserting and saving missing defaults.
    pub fn read(p: &mut PropertyManager) -> Self {
        let online_mode = p.get_bool("online-mode", true);
        let server_ip = p.get_string("server-ip", "");
        let spawn_animals = p.get_bool("spawn-animals", true);
        let spawn_npcs = p.get_bool("spawn-npcs", true);
        let pvp = p.get_bool("pvp", true);
        let allow_flight = p.get_bool("allow-flight", false);
        let resource_pack = p.get_string("resource-pack", "");
        let resource_pack_hash = p.get_string("resource-pack-hash", "");
        let motd = p.get_string("motd", "A Minecraft Server");
        let force_gamemode = p.get_bool("force-gamemode", false);
        let player_idle_timeout = p.get_int("player-idle-timeout", 0);

        // Vanilla clamps the difficulty in memory (setProperty, no save).
        let mut difficulty = p.get_int("difficulty", 1);
        if difficulty < 0 {
            difficulty = 0;
            p.set("difficulty", 0);
        } else if difficulty > 3 {
            difficulty = 3;
            p.set("difficulty", 3);
        }

        let generate_structures = p.get_bool("generate-structures", true);
        let gamemode = p.get_int("gamemode", 0);
        let server_port = p.get_int("server-port", 25565);
        // Vanilla evaluates `Epoll.isAvailable() && shouldUseNativeTransport()`: the key is
        // only read (and therefore written) where Netty's epoll transport exists.
        let use_native_transport =
            cfg!(target_os = "linux") && p.get_bool("use-native-transport", true);

        // DedicatedPlayerList constructor.
        let view_distance = p.get_int("view-distance", 10);
        let max_players = p.get_int("max-players", 20);
        let white_list = p.get_bool("white-list", false);

        let level_name = p.get_string("level-name", "world");
        let level_seed = p.get_string("level-seed", "");
        let level_type = p.get_string("level-type", "DEFAULT");
        let generator_settings = p.get_string("generator-settings", "");
        let announce_player_achievements = p.get_bool("announce-player-achievements", true);
        let enable_command_block = p.get_bool("enable-command-block", false);
        let op_permission_level = p.get_int("op-permission-level", 4);
        let snooper_enabled = p.get_bool("snooper-enabled", true);
        let network_compression_threshold = p.get_int("network-compression-threshold", 256);

        // Rounded to a multiple of 16 and clamped to [64, 256], written back in memory.
        let raw_height = p.get_int("max-build-height", 256);
        let max_build_height = (raw_height.wrapping_add(8) / 16 * 16).clamp(64, 256);
        p.set("max-build-height", max_build_height);

        let allow_nether = p.get_bool("allow-nether", true);
        let hardcore = p.get_bool("hardcore", false);
        let spawn_monsters = p.get_bool("spawn-monsters", true);
        let max_world_size = p.get_int("max-world-size", 29_999_984).clamp(1, 29_999_984);
        // `spawn-protection` and `debug` are only read lazily by vanilla (not during
        // startup), so a fresh file does not contain them; see the parity golden
        // `startup/fresh_properties`.
        let enable_query = p.get_bool("enable-query", false);
        let enable_rcon = p.get_bool("enable-rcon", false);
        let max_tick_time = p.get_long("max-tick-time", 60_000);

        Self {
            online_mode,
            server_ip,
            spawn_animals,
            spawn_npcs,
            pvp,
            allow_flight,
            resource_pack,
            resource_pack_hash,
            motd,
            force_gamemode,
            player_idle_timeout,
            difficulty,
            generate_structures,
            gamemode,
            server_port,
            use_native_transport,
            view_distance,
            max_players,
            white_list,
            level_name,
            level_seed,
            level_type,
            generator_settings,
            announce_player_achievements,
            enable_command_block,
            op_permission_level,
            snooper_enabled,
            network_compression_threshold,
            max_build_height,
            allow_nether,
            hardcore,
            spawn_monsters,
            max_world_size,
            enable_query,
            enable_rcon,
            max_tick_time,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Key order vanilla 1.8.8 (Java 8, Windows) wrote for a file that initially held only
    /// `server-port` (recorded in `parity/golden/startup/fresh_properties.json`).
    #[cfg(not(target_os = "linux"))]
    const VANILLA_ORDER: [&str; 35] = [
        "generator-settings",
        "op-permission-level",
        "allow-nether",
        "resource-pack-hash",
        "level-name",
        "enable-query",
        "allow-flight",
        "announce-player-achievements",
        "server-port",
        "max-world-size",
        "level-type",
        "enable-rcon",
        "level-seed",
        "force-gamemode",
        "server-ip",
        "network-compression-threshold",
        "max-build-height",
        "spawn-npcs",
        "white-list",
        "spawn-animals",
        "hardcore",
        "snooper-enabled",
        "online-mode",
        "resource-pack",
        "pvp",
        "difficulty",
        "enable-command-block",
        "gamemode",
        "player-idle-timeout",
        "max-players",
        "max-tick-time",
        "spawn-monsters",
        "generate-structures",
        "view-distance",
        "motd",
    ];

    /// Keys in file line order (re-parsing into a table would reorder colliding keys).
    #[cfg(not(target_os = "linux"))]
    fn keys(path: &std::path::Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .filter(|l| !l.starts_with('#'))
            .map(|l| l.split('=').next().unwrap().to_owned())
            .collect()
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn fresh_file_matches_vanilla_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.properties");
        std::fs::write(
            &path,
            "server-port=25565
",
        )
        .unwrap();
        let (props, _) = ServerProperties::load(&path);
        assert_eq!(props.motd, "A Minecraft Server");
        assert_eq!(props.max_tick_time, 60_000);
        assert_eq!(keys(&path), VANILLA_ORDER);
    }

    #[test]
    fn values_are_clamped_like_vanilla() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.properties");
        std::fs::write(
            &path,
            "difficulty=7\nmax-build-height=1000\nmax-world-size=0\n",
        )
        .unwrap();
        let (props, pm) = ServerProperties::load(&path);
        assert_eq!(props.difficulty, 3);
        assert_eq!(props.max_build_height, 256);
        assert_eq!(props.max_world_size, 1);
        assert_eq!(pm.properties().get("difficulty"), Some("3"));
    }
}
