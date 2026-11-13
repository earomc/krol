//! The game: a single "Server thread" ticking at 20 TPS (vanilla: `MinecraftServer.run`,
//! `ServerConfigurationManager`, `NetHandlerPlayServer`).
//!
//! Network tasks send [`GameEvent`]s; the tick drains them first, like vanilla runs its
//! scheduled packet tasks at the start of each tick. Packets to clients go through each
//! player's [`Outgoing`] queue.

pub mod commands;
pub mod player;
pub mod world;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use krol_protocol::ProtocolVersion;
use krol_protocol::packets::play::{
    ChunkColumn, ClientboundPlay, Metadata, MetadataValue, PlayerListAction, PlayerListAdd,
    ServerboundPlay,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, error::TryRecvError};
use tracing::info;

use crate::auth::GameProfile;
use crate::chat::{Arg, Component};
use crate::config::PropertyManager;
use crate::net::NetShared;
use crate::status::java_uuid_string;
use player::{Player, default_player_data, load_player_data, stored_position};
use world::Level;

/// Milliseconds per tick.
const TICK_MS: u64 = 50;
/// Maximum chunks sent to one player per tick (vanilla sends up to 10 per bulk packet).
const CHUNKS_PER_TICK: usize = 10;

/// Something sent to the client by the game.
#[derive(Debug, Clone)]
pub enum Outgoing {
    /// A play packet.
    Packet(ClientboundPlay),
    /// Disconnect with a JSON reason, then close.
    Disconnect(String),
}

/// Why a player's connection ended (from the network side).
#[derive(Debug, Clone)]
pub enum DisconnectReason {
    /// The client closed the connection.
    Closed,
    /// No data for 30 seconds.
    Timeout,
    /// An exception while handling the connection (vanilla's "Internal Exception").
    Internal(String),
}

impl DisconnectReason {
    fn component(&self) -> Component {
        match self {
            Self::Closed => Component::text("Disconnected"),
            Self::Timeout => Component::translate("disconnect.timeout", vec![]),
            Self::Internal(e) => Component::translate(
                "disconnect.genericReason",
                vec![Arg::Str(format!("Internal Exception: {e}"))],
            ),
        }
    }
}

/// Events from the network layer.
#[derive(Debug)]
pub enum GameEvent {
    /// A player finished logging in.
    Join {
        /// Connection id.
        conn_id: u64,
        /// Authenticated profile.
        profile: GameProfile,
        /// Remote address.
        peer: SocketAddr,
        /// Client protocol.
        version: ProtocolVersion,
        /// The player's outgoing queue.
        out: UnboundedSender<Outgoing>,
    },
    /// A play packet from a player.
    Packet {
        /// Connection id.
        conn_id: u64,
        /// The packet.
        packet: ServerboundPlay,
    },
    /// A player's connection ended.
    Disconnected {
        /// Connection id.
        conn_id: u64,
        /// Why.
        reason: DisconnectReason,
    },
    /// A console command line (executed at the end of the tick, like vanilla's
    /// `executePendingCommands`).
    Console(String),
    /// Stop the server (e.g. Ctrl-C) without running a command.
    Shutdown,
}

/// Settings the game reads from `server.properties`.
#[derive(Debug, Clone)]
pub struct GameConfig {
    /// `view-distance`.
    pub view_distance: i32,
    /// `max-players`.
    pub max_players: i32,
    /// `gamemode`.
    pub gamemode: i32,
    /// `force-gamemode`.
    pub force_gamemode: bool,
    /// `difficulty`.
    pub difficulty: i32,
    /// `allow-flight`.
    pub allow_flight: bool,
    /// `op-permission-level`.
    pub op_permission_level: i32,
}

/// Handle to the running game thread.
#[derive(Debug)]
pub struct GameHandle {
    tx: UnboundedSender<GameEvent>,
    thread: Option<JoinHandle<()>>,
    stopped: Option<tokio::sync::oneshot::Receiver<()>>,
}

impl GameHandle {
    /// The event sender for network tasks.
    pub fn sender(&self) -> UnboundedSender<GameEvent> {
        self.tx.clone()
    }

    /// Resolves once the game has stopped by itself (e.g. the `stop` command) and
    /// finished saving. Can be awaited once.
    pub fn stopped(&mut self) -> tokio::sync::oneshot::Receiver<()> {
        self.stopped
            .take()
            .unwrap_or_else(|| tokio::sync::oneshot::channel().1)
    }

    /// Asks the game to stop (if still running) and waits for it to finish saving.
    pub fn stop(mut self) {
        let _ = self.tx.send(GameEvent::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The game state owned by the server thread.
struct Game {
    level: Level,
    config: GameConfig,
    props: PropertyManager,
    shared: Arc<NetShared>,
    players: Vec<Player>,
    next_entity_id: i32,
    tick_count: u64,
    last_status_refresh: Option<Instant>,
    stopping: bool,
    /// `usercache.json`.
    user_cache: crate::usercache::UserCache,
    /// Players disconnected by the server, removed (with logs) in the next network tick.
    pending_removals: Vec<(u64, Component)>,
    /// `save-on` / `save-off`.
    saving_enabled: bool,
    /// The world's unseeded `Random` (weather durations).
    weather_rand: krol_java::JavaRandom,
    /// Rain strength and its previous-tick value (vanilla: `rainingStrength`).
    rain: (f32, f32),
    /// Thunder strength and its previous-tick value.
    thunder: (f32, f32),
}

/// Starts the server thread for `level`.
pub fn start(
    mut level: Level,
    config: GameConfig,
    props: PropertyManager,
    shared: Arc<NetShared>,
) -> GameHandle {
    // Vanilla applies the configured default game mode and difficulty to the world.
    level.data.set_game_type(config.gamemode);
    level
        .data
        .data
        .insert("Difficulty", krol_nbt::Tag::Byte(config.difficulty as i8));
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let (stopped_tx, stopped_rx) = tokio::sync::oneshot::channel();
    let game = Game {
        level,
        config,
        props,
        shared,
        players: Vec::new(),
        next_entity_id: 0,
        tick_count: 0,
        last_status_refresh: None,
        stopping: false,
        pending_removals: Vec::new(),
        user_cache: crate::usercache::UserCache::load("usercache.json"),
        saving_enabled: true,
        weather_rand: krol_java::JavaRandom::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as i64),
        ),
        rain: (0.0, 0.0),
        thunder: (0.0, 0.0),
    };
    let mut game = game;
    // Vanilla World.calculateInitialWeather.
    if game.level.data.raining() {
        game.rain = (1.0, 1.0);
        if game.level.data.thundering() {
            game.thunder = (1.0, 1.0);
        }
    }
    let thread = std::thread::Builder::new()
        .name("Server thread".into())
        .spawn(move || {
            game.run(rx);
            let _ = stopped_tx.send(());
        })
        .expect("spawn server thread");
    GameHandle {
        tx,
        thread: Some(thread),
        stopped: Some(stopped_rx),
    }
}

impl Game {
    /// Vanilla's main loop: accumulate elapsed time and run one tick per 50 ms.
    fn run(mut self, mut rx: UnboundedReceiver<GameEvent>) {
        let mut last = Instant::now();
        let mut pending = Duration::ZERO;
        loop {
            let now = Instant::now();
            pending += now - last;
            last = now;
            // Vanilla quirk: after a 2 s lag spike it drops the backlog ("Can't keep up!").
            if pending > Duration::from_secs(2) {
                pending = Duration::ZERO;
            }
            while pending > Duration::from_millis(TICK_MS) {
                pending -= Duration::from_millis(TICK_MS);
                if !self.tick(&mut rx) {
                    self.shutdown();
                    return;
                }
            }
            std::thread::sleep(
                Duration::from_millis(TICK_MS)
                    .saturating_sub(pending)
                    .max(Duration::from_millis(1)),
            );
        }
    }

    /// One tick; returns `false` when the server should stop.
    fn tick(&mut self, rx: &mut UnboundedReceiver<GameEvent>) -> bool {
        self.tick_count += 1;
        let mut commands = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(GameEvent::Shutdown) | Err(TryRecvError::Disconnected) => return false,
                Ok(GameEvent::Console(line)) => commands.push(line),
                Ok(event) => self.handle_event(event),
                Err(TryRecvError::Empty) => break,
            }
        }
        // Vanilla NetworkSystem.networkTick: connections closed by the server are
        // handled after the packets.
        for (conn_id, reason) in std::mem::take(&mut self.pending_removals) {
            if let Some(i) = self.players.iter().position(|p| p.conn_id == conn_id) {
                self.remove_player(i, &reason);
            }
        }

        self.refresh_status();
        self.tick_weather();
        self.tick_time();
        if self.tick_count % 900 == 0 && self.saving_enabled {
            // Vanilla autosave (saveAllWorlds(true): no log lines).
            let dir = self.level.dir.clone();
            for p in &mut self.players {
                p.save(&dir);
            }
            self.level.save_quietly();
        }
        let radius = self.config.view_distance;
        self.tick_active_chunks(radius);
        for i in 0..self.players.len() {
            self.tick_player(i, radius);
        }
        for line in commands {
            self.console_command(&line);
        }
        !self.stopping
    }

    /// Runs a console command line.
    fn console_command(&mut self, line: &str) {
        if line.trim().is_empty() {
            return;
        }
        self.execute_command(commands::Sender::Console, line);
    }

    /// Vanilla `World.updateWeather` + `WorldServer.updateWeather` (overworld).
    fn tick_weather(&mut self) {
        let d = &mut self.level.data;
        let was_raining = self.rain.0 > 0.2;
        let clear = d.clear_weather_time();
        if clear > 0 {
            d.set_clear_weather_time(clear - 1);
            d.set_thunder_time(if d.thundering() { 1 } else { 2 });
            d.set_rain_time(if d.raining() { 1 } else { 2 });
        }
        let t = d.thunder_time();
        if t <= 0 {
            let next = if d.thundering() {
                self.weather_rand.next_int_bounded(12000) + 3600
            } else {
                self.weather_rand.next_int_bounded(168_000) + 12000
            };
            d.set_thunder_time(next);
        } else {
            d.set_thunder_time(t - 1);
            if t - 1 <= 0 {
                let v = !d.thundering();
                d.data
                    .insert("thundering", krol_nbt::Tag::Byte(i8::from(v)));
            }
        }
        self.thunder.1 = self.thunder.0;
        self.thunder.0 =
            (self.thunder.0 + if d.thundering() { 0.01 } else { -0.01 }).clamp(0.0, 1.0);
        let r = d.rain_time();
        if r <= 0 {
            let next = if d.raining() {
                self.weather_rand.next_int_bounded(12000) + 12000
            } else {
                self.weather_rand.next_int_bounded(168_000) + 12000
            };
            d.set_rain_time(next);
        } else {
            d.set_rain_time(r - 1);
            if r - 1 <= 0 {
                let v = !d.raining();
                d.data.insert("raining", krol_nbt::Tag::Byte(i8::from(v)));
            }
        }
        self.rain.1 = self.rain.0;
        self.rain.0 = (self.rain.0 + if d.raining() { 0.01 } else { -0.01 }).clamp(0.0, 1.0);

        let mut packets = Vec::new();
        if self.rain.1 != self.rain.0 {
            packets.push(ClientboundPlay::ChangeGameState {
                reason: 7,
                value: self.rain.0,
            });
        }
        if self.thunder.1 != self.thunder.0 {
            packets.push(ClientboundPlay::ChangeGameState {
                reason: 8,
                value: self.thunder.0,
            });
        }
        if was_raining != (self.rain.0 > 0.2) {
            packets.push(ClientboundPlay::ChangeGameState {
                reason: if was_raining { 2 } else { 1 },
                value: 0.0,
            });
            packets.push(ClientboundPlay::ChangeGameState {
                reason: 7,
                value: self.rain.0,
            });
            packets.push(ClientboundPlay::ChangeGameState {
                reason: 8,
                value: self.thunder.0,
            });
        }
        for p in &self.players {
            for packet in &packets {
                p.send(packet.clone());
            }
        }
    }

    fn refresh_status(&mut self) {
        let due = self
            .last_status_refresh
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(5));
        if !due {
            return;
        }
        self.last_status_refresh = Some(Instant::now());
        let online = self.players.len() as i32;
        self.shared.online_players.store(online, Ordering::Relaxed);
        let mut status = self.shared.status.blocking_write();
        status.online_players = online;
        status.players_known = true;
        // TODO: vanilla samples up to 12 players at a random offset and shuffles them.
        status.sample = self
            .players
            .iter()
            .take(12)
            .map(|p| (p.profile.id, p.profile.name.clone()))
            .collect();
    }

    fn tick_time(&mut self) {
        let data = &mut self.level.data;
        data.set_time(data.time() + 1);
        let daylight = data.game_rule("doDaylightCycle") != Some("false");
        if daylight {
            data.set_day_time(data.day_time() + 1);
        }
        if self.tick_count % 20 == 0 {
            let packet = self.time_packet();
            for p in &self.players {
                p.send(packet.clone());
            }
        }
    }

    fn time_packet(&self) -> ClientboundPlay {
        let data = &self.level.data;
        let mut time_of_day = data.day_time();
        if data.game_rule("doDaylightCycle") == Some("false") {
            // Vanilla sends a negative time to stop the client's clock.
            time_of_day = -time_of_day;
            if time_of_day == 0 {
                time_of_day = -1;
            }
        }
        ClientboundPlay::TimeUpdate {
            world_age: data.time(),
            time_of_day,
        }
    }

    fn handle_event(&mut self, event: GameEvent) {
        match event {
            GameEvent::Join {
                conn_id,
                profile,
                peer,
                version,
                out,
            } => self.join(conn_id, profile, peer, version, out),
            GameEvent::Packet { conn_id, packet } => {
                if let Some(i) = self.players.iter().position(|p| p.conn_id == conn_id) {
                    self.handle_packet(i, packet);
                }
            }
            GameEvent::Disconnected { conn_id, reason } => {
                // A kick already scheduled its own removal with the kick reason.
                if self.pending_removals.iter().any(|(c, _)| *c == conn_id) {
                    return;
                }
                if let Some(i) = self.players.iter().position(|p| p.conn_id == conn_id) {
                    self.remove_player(i, &reason.component());
                }
            }
            GameEvent::Shutdown | GameEvent::Console(_) => {}
        }
    }

    /// Vanilla `sendChatMsgImpl`: logs and sends to everyone. Player chat uses position
    /// 0; server messages (`sendChatMsg`) are system messages (position 1).
    fn broadcast(&self, message: &Component, except: Option<u64>, player_chat: bool) {
        info!("{}", message.unformatted());
        let json = message.to_json_string();
        for p in &self.players {
            if Some(p.conn_id) != except {
                p.send(ClientboundPlay::Chat {
                    json: json.clone(),
                    position: if player_chat { 0 } else { 1 },
                });
            }
        }
    }

    /// Vanilla `sendChatMsg` (system message to everyone).
    fn broadcast_chat(&self, message: &Component, except: Option<u64>) {
        self.broadcast(message, except, false);
    }

    fn display_name(profile: &GameProfile) -> Component {
        Component::player_name(&profile.name, &java_uuid_string(profile.id))
    }

    /// Spawn position for a new player (vanilla: `EntityPlayerMP` constructor).
    fn spawn_position(&mut self, gamemode: i32) -> (f64, f64, f64) {
        let data = &self.level.data;
        let (sx, sy, sz) = (data.spawn_x(), data.spawn_y(), data.spawn_z());
        let (mut x, mut y, mut z) = (sx, sy, sz);
        if gamemode != 2 {
            // Read lazily, like vanilla (this is when the key appears in the file).
            let protection = self.props.get_int("spawn-protection", 16);
            let radius = (protection - 6).max(5);
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as i64);
            // Vanilla uses the entity's own unseeded Random, so this is nondeterministic
            // there too.
            let mut rand = krol_java::JavaRandom::new(seed);
            x = sx + rand.next_int_bounded(radius * 2) - radius;
            z = sz + rand.next_int_bounded(radius * 2) - radius;
            y = self.top_solid_block(x, z);
        }
        // Move up out of blocks (approximation of vanilla's bounding box check).
        while y < 255
            && (self.level.overworld.block(x, y, z) != 0
                || self.level.overworld.block(x, y + 1, z) != 0)
        {
            y += 1;
        }
        (f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5)
    }

    fn top_solid_block(&mut self, x: i32, z: i32) -> i32 {
        for y in (0..256).rev() {
            let state = self.level.overworld.block(x, y, z);
            let id = state >> 4;
            // Air and leaves do not count (vanilla: getTopSolidOrLiquidBlock).
            if id != 0 && id != 18 && id != 161 {
                return y + 1;
            }
        }
        0
    }

    /// Vanilla `initializeConnectionToPlayer` + `playerLoggedIn`.
    fn join(
        &mut self,
        conn_id: u64,
        profile: GameProfile,
        peer: SocketAddr,
        version: ProtocolVersion,
        out: UnboundedSender<Outgoing>,
    ) {
        self.user_cache.add(profile.id, &profile.name);
        let stored = load_player_data(&self.level.dir, profile.id);
        let gamemode = match &stored {
            Some(d) if !self.config.force_gamemode && d.contains_key("playerGameType") => {
                d.get_i32("playerGameType")
            }
            _ => self.config.gamemode,
        };
        let (pos, rotation) = match stored.as_ref().and_then(stored_position) {
            Some(p) => p,
            None => (self.spawn_position(gamemode), (0.0, 0.0)),
        };
        let entity_id = self.next_entity_id;
        self.next_entity_id += 1;
        let data = stored.unwrap_or_else(|| default_player_data(gamemode));
        let selected_slot = data.get_i32("SelectedItemSlot") as i8;

        info!(
            "{}[/{}:{}] logged in with entity id {} at ({}, {}, {})",
            profile.name,
            peer.ip(),
            peer.port(),
            entity_id,
            krol_java::fmt::java_double(pos.0),
            krol_java::fmt::java_double(pos.1),
            krol_java::fmt::java_double(pos.2)
        );

        let player = Player {
            conn_id,
            entity_id,
            profile,
            peer,
            version,
            out,
            pos,
            rotation,
            on_ground: true,
            gamemode,
            skin_parts: 0,
            ticks: 0,
            last_keep_alive: 0,
            watching: Default::default(),
            chunk_queue: Default::default(),
            managed_pos: (0.0, 0.0),
            movement: Default::default(),
            new_watches: Vec::new(),
            xp_dirty: false,
            score_dirty: false,
            chat_spam: 0,
            data,
        };

        let level = &self.level.data;
        let difficulty = self.config.difficulty as u8;
        player.send(ClientboundPlay::JoinGame {
            entity_id,
            gamemode: gamemode as u8 | if self.level.data.hardcore() { 8 } else { 0 },
            dimension: 0,
            difficulty,
            max_players: self.config.max_players as u8,
            level_type: level.generator_name().to_owned(),
            reduced_debug_info: level.game_rule("reducedDebugInfo") == Some("true"),
        });
        player.send(ClientboundPlay::PluginMessage {
            channel: "MC|Brand".into(),
            data: b"\x07vanilla".to_vec(),
        });
        player.send(ClientboundPlay::ServerDifficulty { difficulty });
        player.send(ClientboundPlay::SpawnPosition {
            pos: (level.spawn_x(), level.spawn_y(), level.spawn_z()),
        });
        player.send(abilities_packet(gamemode));
        player.send(ClientboundPlay::HeldItemChange {
            slot: selected_slot,
        });
        player.send(ClientboundPlay::Statistics(Vec::new()));

        let joined = Component::translate(
            "multiplayer.player.joined",
            vec![Arg::Component(Self::display_name(&player.profile))],
        )
        .color("yellow");
        self.broadcast_chat(&joined, None);

        // playerLoggedIn: announce the new player to everyone (including itself), then
        // tell the new player about everyone.
        self.players.push(player);
        let new_index = self.players.len() - 1;
        let add_new =
            ClientboundPlay::PlayerListItem(PlayerListAction::Add(vec![player_list_entry(
                &self.players[new_index],
            )]));
        for p in &self.players {
            p.send(add_new.clone());
        }
        for p in &self.players {
            self.players[new_index].send(ClientboundPlay::PlayerListItem(PlayerListAction::Add(
                vec![player_list_entry(p)],
            )));
        }

        let radius = self.config.view_distance;
        let player = &mut self.players[new_index];
        player.start_watching(radius);
        for (x, z) in self.players[new_index].take_new_watches() {
            self.level.overworld.chunk(x, z);
        }
        let player = &mut self.players[new_index];
        let (pos, rot) = (player.pos, player.rotation);
        Self::set_player_location(player, pos, rot);

        // updateTimeAndWeatherForPlayer
        let data = &self.level.data.data;
        let border_size = if data.contains_key("BorderSize") {
            data.get_f64("BorderSize")
        } else {
            6.0e7
        };
        let world_border = ClientboundPlay::WorldBorderInit {
            x: data.get_f64("BorderCenterX"),
            z: data.get_f64("BorderCenterZ"),
            old_diameter: border_size,
            new_diameter: border_size,
            speed: 0,
            portal_teleport_boundary: 29_999_984,
            warning_distance: if data.contains_key("BorderWarningBlocks") {
                data.get_f64("BorderWarningBlocks") as i32
            } else {
                5
            },
            warning_time: if data.contains_key("BorderWarningTime") {
                data.get_f64("BorderWarningTime") as i32
            } else {
                15
            },
        };
        let time = self.time_packet();
        let raining = self.rain.0 > 0.2;
        let (rain_strength, thunder_strength) = (self.rain.0, self.thunder.0 * self.rain.0);
        let player = &self.players[new_index];
        player.send(world_border);
        player.send(time);
        if raining {
            player.send(ClientboundPlay::ChangeGameState {
                reason: 1,
                value: 0.0,
            });
            player.send(ClientboundPlay::ChangeGameState {
                reason: 7,
                value: rain_strength,
            });
            player.send(ClientboundPlay::ChangeGameState {
                reason: 8,
                value: thunder_strength,
            });
        }
        // TODO(M4): send the stored inventory instead of an empty one.
        player.send(ClientboundPlay::WindowItems {
            window: 0,
            items: vec![None; 45],
        });
        player.send(ClientboundPlay::SetSlot {
            window: -1,
            slot: -1,
            item: None,
        });
    }

    fn tick_player(&mut self, i: usize, radius: i32) {
        let player = &mut self.players[i];
        player.ticks += 1;
        if player.ticks == 1 {
            // First EntityPlayerMP update and entity tracker pass after joining.
            let d = &player.data;
            player.send(ClientboundPlay::UpdateHealth {
                health: d.get_f32("HealF"),
                food: d.get_i32("foodLevel"),
                saturation: d.get_f32("foodSaturationLevel"),
            });
            player.send(ClientboundPlay::SetExperience {
                bar: d.get_f32("XpP"),
                level: d.get_i32("XpLevel"),
                total: d.get_i32("XpTotal"),
            });
            player.send(self_metadata(player));
            player.send(ClientboundPlay::EntityProperties {
                entity_id: player.entity_id,
                properties: vec![("generic.movementSpeed".into(), f64::from(0.1f32))],
            });
        }
        if player.ticks > 1 && player.xp_dirty {
            // Vanilla EntityPlayerMP.onUpdateEntity sends experience when it changed.
            player.xp_dirty = false;
            let d = &player.data;
            player.send(ClientboundPlay::SetExperience {
                bar: d.get_f32("XpP"),
                level: d.get_i32("XpLevel"),
                total: d.get_i32("XpTotal"),
            });
        }
        if player.score_dirty {
            // Entity tracker: changed data watcher entries (score is index 18).
            player.score_dirty = false;
            player.send(ClientboundPlay::EntityMetadata {
                entity_id: player.entity_id,
                metadata: vec![Metadata {
                    index: 18,
                    value: MetadataValue::Int(player.data.get_i32("Score")),
                }],
            });
        }
        if player.ticks - player.last_keep_alive > 40 {
            player.last_keep_alive = player.ticks;
            let id = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64) as i32;
            player.send(ClientboundPlay::KeepAlive { id });
        }
        if player.chat_spam > 0 {
            player.chat_spam -= 1;
        }

        player.movement.network_ticks += 1;

        // Chunk streaming (vanilla: EntityPlayerMP.onUpdate): up to 10 queued chunks per
        // tick; a single chunk goes as Chunk Data, several as Map Chunk Bulk.
        let _ = radius;
        // Like vanilla's iterator: take up to 10 sendable chunks, leave the rest queued.
        let mut batch: Vec<(i32, i32)> = Vec::new();
        let mut k = 0;
        while batch.len() < CHUNKS_PER_TICK && k < self.players[i].chunk_queue.len() {
            let c = self.players[i].chunk_queue[k];
            if self.level.overworld.is_sendable(c.0, c.1) {
                self.players[i].chunk_queue.remove(k);
                batch.push(c);
            } else {
                k += 1;
            }
        }
        if batch.is_empty() {
            return;
        }
        let sky = self.level.overworld.has_sky;
        let mut columns: Vec<ChunkColumn> = batch
            .iter()
            .map(|&(x, z)| {
                let (mask, data) = self
                    .level
                    .overworld
                    .chunk(x, z)
                    .packet_data(true, 0xffff, sky);
                ChunkColumn { x, z, mask, data }
            })
            .collect();
        let player = &mut self.players[i];
        if columns.len() == 1 {
            player.send(ClientboundPlay::ChunkData {
                column: columns.remove(0),
                full: true,
            });
        } else {
            player.send(ClientboundPlay::MapChunkBulk {
                sky_light: sky,
                columns,
            });
        }
    }

    /// Vanilla `World.setActivePlayerChunksAndCheckLight` + `updateBlocks`: every chunk
    /// within the view distance of a player (`floor(pos / 16)`) is ticked.
    fn tick_active_chunks(&mut self, radius: i32) {
        let mut active = std::collections::HashSet::new();
        for p in &self.players {
            let cx = krol_java::math::floor_f64(p.pos.0 / 16.0);
            let cz = krol_java::math::floor_f64(p.pos.2 / 16.0);
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    active.insert((cx + dx, cz + dz));
                }
            }
        }
        for (x, z) in active {
            self.level.overworld.tick_chunk(x, z);
        }
    }

    /// Vanilla `setPlayerLocation`: teleport and wait for the client to confirm.
    fn set_player_location(player: &mut Player, pos: (f64, f64, f64), rot: (f32, f32)) {
        player.movement.has_moved = false;
        player.movement.last_pos = pos;
        player.pos = pos;
        player.rotation = rot;
        player.send(ClientboundPlay::PositionLook {
            x: pos.0,
            y: pos.1,
            z: pos.2,
            yaw: rot.0,
            pitch: rot.1,
            flags: 0,
        });
    }

    /// Vanilla `NetHandlerPlayServer.processPlayer` for C03 to C06.
    ///
    /// Known deviation: vanilla moves the bounding box through the world (`moveEntity`)
    /// and reports "moved wrongly!" or undoes moves into blocks; krol does not model
    /// collision yet (M5) and accepts those moves.
    fn process_movement(
        &mut self,
        i: usize,
        position: Option<(f64, f64, f64)>,
        rotation: Option<(f32, f32)>,
        on_ground: bool,
    ) {
        let radius = self.config.view_distance;
        let allow_flight = self.config.allow_flight;
        let player = &mut self.players[i];
        let mut position = position;
        if let Some(p) = position {
            let m = &mut player.movement;
            let d = (p.0 - m.last_pos.0, p.1 - m.last_pos.1, p.2 - m.last_pos.2);
            if !m.has_moved && d.0 * d.0 + d.1 * d.1 + d.2 * d.2 < 0.25 {
                m.has_moved = true;
            }
        }
        if !player.movement.has_moved {
            if player.movement.network_ticks - player.movement.last_move_tick > 20 {
                let (pos, rot) = (player.movement.last_pos, player.rotation);
                Self::set_player_location(player, pos, rot);
            }
            return;
        }
        player.movement.last_move_tick = player.movement.network_ticks;
        player.movement.last_pos = player.pos;
        if position.is_some_and(|p| p.1 == -999.0) {
            position = None;
        }
        if let Some(p) = position
            && (p.0.abs() > 3.0e7 || p.2.abs() > 3.0e7)
        {
            self.kick(i, "Illegal position");
            return;
        }
        let player = &mut self.players[i];
        let target = position.unwrap_or(player.pos);
        let rot = rotation.unwrap_or(player.rotation);
        let d = (
            target.0 - player.pos.0,
            target.1 - player.pos.1,
            target.2 - player.pos.2,
        );
        // TODO(M5): subtract the squared motion (vanilla: `d15 - d14 > 100`).
        if d.0 * d.0 + d.1 * d.1 + d.2 * d.2 > 100.0 {
            let (dx, dy, dz) = (
                krol_java::fmt::java_double(d.0),
                krol_java::fmt::java_double(d.1),
                krol_java::fmt::java_double(d.2),
            );
            // Vanilla quirk: the same three numbers are printed twice.
            tracing::warn!(
                "{} moved too quickly! {dx},{dy},{dz} ({dx}, {dy}, {dz})",
                player.profile.name
            );
            let last = player.movement.last_pos;
            let rot = player.rotation;
            Self::set_player_location(player, last, rot);
            return;
        }
        player.pos = target;
        player.rotation = rot;
        player.on_ground = on_ground;
        let gamemode = player.gamemode;
        let (x, y, z) = player.pos;

        // Floating check (vanilla: checkBlockCollision on the box grown by 1/16 and
        // extended 0.55 down; any non-air block counts).
        let flying_allowed = allow_flight || gamemode == 1 || gamemode == 3;
        let floor = krol_java::math::floor_f64;
        let mut touching = false;
        'scan: for bx in floor(x - 0.3625)..floor(x + 0.3625 + 1.0) {
            for by in floor(y - 0.6125)..floor(y + 1.8625 + 1.0) {
                for bz in floor(z - 0.3625)..floor(z + 0.3625 + 1.0) {
                    if self.level.overworld.block(bx, by, bz) != 0 {
                        touching = true;
                        break 'scan;
                    }
                }
            }
        }
        let player = &mut self.players[i];
        if !flying_allowed && !touching {
            if d.1 >= -0.03125 {
                player.movement.floating_ticks += 1;
                if player.movement.floating_ticks > 80 {
                    tracing::warn!("{} was kicked for floating too long!", player.profile.name);
                    self.kick(i, "Flying is not enabled on this server");
                    return;
                }
            }
        } else {
            player.movement.floating_ticks = 0;
        }

        let unload = self.players[i].update_watching(radius);
        for (x, z) in self.players[i].take_new_watches() {
            self.level.overworld.chunk(x, z);
        }
        let sky = self.level.overworld.has_sky;
        for (ux, uz) in unload {
            let (_, data) = self.level.overworld.chunk(ux, uz).packet_data(true, 0, sky);
            self.players[i].send(ClientboundPlay::ChunkData {
                column: ChunkColumn {
                    x: ux,
                    z: uz,
                    mask: 0,
                    data,
                },
                full: true,
            });
        }
    }

    fn handle_packet(&mut self, i: usize, packet: ServerboundPlay) {
        match packet {
            ServerboundPlay::Chat { message } => self.chat(i, &message),
            ServerboundPlay::Position { x, y, z, on_ground } => {
                self.process_movement(i, Some((x, y, z)), None, on_ground);
            }
            ServerboundPlay::Look {
                yaw,
                pitch,
                on_ground,
            } => self.process_movement(i, None, Some((yaw, pitch)), on_ground),
            ServerboundPlay::PositionLook {
                x,
                y,
                z,
                yaw,
                pitch,
                on_ground,
            } => self.process_movement(i, Some((x, y, z)), Some((yaw, pitch)), on_ground),
            ServerboundPlay::Player { on_ground } => {
                self.process_movement(i, None, None, on_ground);
            }
            ServerboundPlay::ClientSettings { skin_parts, .. } => {
                self.players[i].skin_parts = skin_parts;
            }
            // TODO(M3/M4): movement validation, interaction, inventories, ...
            _ => {}
        }
    }

    /// Vanilla `processChatMessage`.
    fn chat(&mut self, i: usize, raw: &str) {
        let message = normalize_space(raw);
        if message.chars().any(|c| !allowed_chat_char(c)) {
            self.kick(i, "Illegal characters in chat");
            return;
        }
        if let Some(command) = message.strip_prefix('/') {
            // Vanilla 1.8.8 does not log player commands (only admin notifications).
            // TODO: permission levels (non-ops may only use a few commands).
            self.execute_command(commands::Sender::Player(i), command);
        } else {
            let player = &self.players[i];
            let msg = Component::translate(
                "chat.type.text",
                vec![
                    Arg::Component(Self::display_name(&player.profile)),
                    Arg::Str(message),
                ],
            );
            self.broadcast(&msg, None, true);
        }
        let player = &mut self.players[i];
        player.chat_spam += 20;
        if player.chat_spam > 200 && !self.is_op(i) {
            let reason = Component::translate("disconnect.spam", vec![]);
            self.kick_with(i, reason);
        }
    }

    fn is_op(&self, i: usize) -> bool {
        let key = java_uuid_string(self.players[i].profile.id);
        self.shared
            .user_lists
            .lock()
            .is_ok_and(|mut l| l.ops.contains(&key))
    }

    /// Vanilla `EntityPlayerMP.setGameType`.
    fn set_game_mode(&mut self, i: usize, mode: i32) {
        let player = &mut self.players[i];
        player.gamemode = mode;
        let abilities = abilities_packet(mode);
        player.send(abilities.clone());
        let uuid = player.profile.id;
        let update =
            ClientboundPlay::PlayerListItem(PlayerListAction::UpdateGamemode(vec![(uuid, mode)]));
        for p in &self.players {
            p.send(update.clone());
        }
        let player = &self.players[i];
        player.send(ClientboundPlay::ChangeGameState {
            reason: 3,
            value: mode as f32,
        });
        player.send(abilities);
    }

    /// Vanilla `kickPlayerFromServer`.
    fn kick(&mut self, i: usize, reason: &str) {
        self.kick_with(i, Component::text(reason));
    }

    fn kick_with(&mut self, i: usize, reason: Component) {
        let _ = self.players[i]
            .out
            .send(Outgoing::Disconnect(reason.to_json_string()));
        let conn_id = self.players[i].conn_id;
        if !self.pending_removals.iter().any(|(c, _)| *c == conn_id) {
            self.pending_removals.push((conn_id, reason));
        }
    }

    /// Vanilla `onDisconnect` + `playerLoggedOut`.
    fn remove_player(&mut self, i: usize, reason: &Component) {
        let name = self.players[i].profile.name.clone();
        info!("{name} lost connection: {}", reason.java_to_string());
        let left = Component::translate(
            "multiplayer.player.left",
            vec![Arg::Component(Self::display_name(&self.players[i].profile))],
        )
        .color("yellow");
        let conn_id = self.players[i].conn_id;
        self.broadcast_chat(&left, Some(conn_id));
        let mut player = self.players.remove(i);
        player.save(&self.level.dir);
        let remove =
            ClientboundPlay::PlayerListItem(PlayerListAction::Remove(vec![player.profile.id]));
        for p in &self.players {
            p.send(remove.clone());
        }
    }

    /// Vanilla `stopServer`.
    fn shutdown(&mut self) {
        info!("Stopping server");
        info!("Saving players");
        let dir = self.level.dir.clone();
        for p in &mut self.players {
            p.save(&dir);
        }
        let closed = Component::text("Server closed").to_json_string();
        for p in &self.players {
            let _ = p.out.send(Outgoing::Disconnect(closed.clone()));
        }
        self.players.clear();
        info!("Saving worlds");
        self.level.save_all();
    }
}

fn abilities_packet(gamemode: i32) -> ClientboundPlay {
    let creative = gamemode == 1;
    let spectator = gamemode == 3;
    let mut flags = 0i8;
    if creative || spectator {
        flags |= 1; // invulnerable
    }
    if spectator {
        flags |= 2; // flying
    }
    if creative || spectator {
        flags |= 4; // allow flying
    }
    if creative {
        flags |= 8; // instant build
    }
    ClientboundPlay::PlayerAbilities {
        flags,
        flying_speed: 0.05,
        walking_speed: 0.1,
    }
}

fn player_list_entry(p: &Player) -> PlayerListAdd {
    PlayerListAdd {
        uuid: p.profile.id,
        name: p.profile.name.clone(),
        properties: p
            .profile
            .properties
            .iter()
            .map(|pr| (pr.name.clone(), pr.value.clone(), pr.signature.clone()))
            .collect(),
        gamemode: p.gamemode,
        ping: 0,
        display_name: None,
    }
}

fn self_metadata(p: &Player) -> ClientboundPlay {
    use MetadataValue::{Byte, Float, Int, Short, String};
    let md = |index, value| Metadata { index, value };
    let d = &p.data;
    ClientboundPlay::EntityMetadata {
        entity_id: p.entity_id,
        metadata: vec![
            md(0, Byte(0)),
            md(
                1,
                Short(if d.contains_key("Air") {
                    d.get_i16("Air")
                } else {
                    300
                }),
            ),
            md(2, String(std::string::String::new())),
            md(3, Byte(0)),
            md(4, Byte(0)),
            md(6, Float(d.get_f32("HealF"))),
            md(7, Int(0)),
            md(8, Byte(0)),
            md(9, Byte(0)),
            md(10, Byte(p.skin_parts as i8)),
            md(16, Byte(0)),
            md(17, Float(d.get_f32("AbsorptionAmount"))),
            md(18, Int(d.get_i32("Score"))),
        ],
    }
}

/// Apache Commons `StringUtils.normalizeSpace`: trim and collapse whitespace runs.
fn normalize_space(s: &str) -> std::string::String {
    s.split(char::is_whitespace)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Vanilla `ChatAllowedCharacters.isAllowedCharacter`.
fn allowed_chat_char(c: char) -> bool {
    c != '\u{a7}' && c >= ' ' && c != '\u{7f}'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_helpers() {
        assert_eq!(normalize_space("  hello \t  world "), "hello world");
        assert!(!allowed_chat_char('\u{a7}'));
        assert!(!allowed_chat_char('\n'));
        assert!(allowed_chat_char('ü'));
    }
}
