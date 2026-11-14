//! The game: a single "Server thread" ticking at 20 TPS (vanilla: `MinecraftServer.run`,
//! `ServerConfigurationManager`, `NetHandlerPlayServer`).
//!
//! Network tasks send [`GameEvent`]s; the tick drains them first, like vanilla runs its
//! scheduled packet tasks at the start of each tick. Packets to clients go through each
//! player's [`Outgoing`] queue.

mod biome;
pub mod block_data;
mod block_update;
pub mod blocks;
mod buckets;
mod chest;
pub mod commands;
mod container;
mod damage;
mod death;
mod digging;
mod dispenser;
mod falling_block;
mod fire;
mod food;
mod furnace;
mod hopper;
pub mod ids;
mod interact;
mod item_data;
mod item_entity;
mod light_data;
mod liquids;
mod movement;
mod noise;
mod note_block;
pub mod physics;
mod pistons;
mod placement;
pub mod player;
mod potions;
mod powered_blocks;
mod pressure_plates;
mod random_ticks;
mod raytrace;
mod recipes;
mod redstone;
mod scheduler;
mod sign;
mod special_recipes;
pub mod stats;
mod tracker;
pub mod world;
mod xp_orb;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use krol_protocol::ProtocolVersion;
use krol_protocol::packets::play::{
    ChunkColumn, ClientboundPlay, PlayerListAction, PlayerListAdd, ServerboundPlay,
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

/// Player Digging status 5: the player let go of the item they were using.
const RELEASE_USE_ITEM: i8 = 5;

/// The first armor item (`leather_helmet`); armor comes in sets of four.
const ARMOR_FIRST_ID: i16 = 298;

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
    /// `max-build-height`.
    pub max_build_height: i32,
    /// `pvp`.
    pub pvp: bool,
}

/// Handle to the running game thread.
#[derive(Debug)]
pub struct GameHandle {
    tx: UnboundedSender<GameEvent>,
    thread: Option<JoinHandle<()>>,
    stopped: Option<tokio::sync::oneshot::Receiver<()>>,
}

impl GameEvent {
    /// The connection an event belongs to.
    fn conn_id(&self) -> Option<u64> {
        match self {
            Self::Join { conn_id, .. }
            | Self::Packet { conn_id, .. }
            | Self::Disconnected { conn_id, .. } => Some(*conn_id),
            _ => None,
        }
    }
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
    /// Entity tracker entries.
    tracker: tracker::Tracker,
    /// `BlockRedstoneWire.canProvidePower`: off while a wire computes its input.
    wire_gives_power: bool,
    /// `BlockRedstoneWire.blocksNeedingUpdate`, in insertion order.
    wire_queue: Vec<(i32, i32, i32)>,
    /// `TileEntityNote.previousRedstoneState` (not saved).
    note_powered: std::collections::HashMap<(i32, i32, i32), bool>,
    /// The current `progress` of moving piston blocks (only `lastProgress` is saved).
    moving_progress: std::collections::HashMap<(i32, i32, i32), f32>,
    /// Scheduled ticks still waiting in the batch being run (`pendingTickListEntriesThisTick`).
    ticks_this_tick: Vec<((i32, i32, i32), u16)>,
    /// `World.updateLCG`: picks random tick positions (vanilla starts it at random;
    /// krol at 0).
    update_lcg: i32,
    /// `World.skylightSubtracted`: how much time and weather darken the sky.
    skylight_subtracted: i32,
    /// `BlockRedstoneTorch.toggles`: recent torch toggles (position, total time).
    torch_toggles: Vec<((i32, i32, i32), i64)>,
    /// Falling blocks by entity id.
    falling: std::collections::BTreeMap<i32, falling_block::FallingBlock>,
    /// Experience orbs by entity id.
    orbs: std::collections::BTreeMap<i32, xp_orb::XpOrb>,
    /// Dropped items by entity id.
    items: std::collections::BTreeMap<i32, item_entity::ItemEntity>,
    /// Chests that are open or closing.
    chests: Vec<chest::ChestRuntime>,
    /// Signs that can be edited, and by whom (`TileEntitySign.isEditable` and
    /// `getPlayer`).
    sign_editors: std::collections::HashMap<(i32, i32, i32), u64>,
    /// `currentItemBurnTime` of furnaces (not saved by vanilla).
    furnace_burn: std::collections::HashMap<(i32, i32, i32), i32>,
    /// Queued block events (`WorldServer.field_147490_S`).
    block_events: Vec<chest::BlockEvent>,
    /// The world's random (`World.rand`, unseeded in vanilla).
    world_rand: krol_java::JavaRandom,
    /// `EnchantmentHelper.enchantmentRand` (unseeded in vanilla).
    enchant_rand: krol_java::JavaRandom,
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
        tracker: Default::default(),
        items: Default::default(),
        orbs: Default::default(),
        falling: Default::default(),
        wire_gives_power: true,
        wire_queue: Vec::new(),
        torch_toggles: Vec::new(),
        update_lcg: 0,
        skylight_subtracted: 0,
        ticks_this_tick: Vec::new(),
        moving_progress: Default::default(),
        note_powered: Default::default(),
        chests: Vec::new(),
        block_events: Vec::new(),
        furnace_burn: Default::default(),
        sign_editors: Default::default(),
        world_rand: krol_java::JavaRandom::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as i64),
        ),
        enchant_rand: krol_java::JavaRandom::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.as_nanos() as i64 ^ 0x5DEE_CE66),
        ),
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
    /// Vanilla's main loop (`MinecraftServer.run`): accumulate elapsed milliseconds and
    /// run one tick per 50 ms. A delay of more than 2 s is cut to 2 s with a warning,
    /// at most once every 15 s; otherwise the whole backlog is caught up.
    fn run(mut self, mut rx: UnboundedReceiver<GameEvent>) {
        let start = Instant::now();
        let millis = || start.elapsed().as_millis() as i64;
        let mut current = millis();
        let mut last_warning: Option<i64> = None;
        let mut pending: i64 = 0;
        loop {
            let now = millis();
            let mut elapsed = now - current;
            if elapsed > 2000 && last_warning.is_none_or(|w| current - w >= 15_000) {
                tracing::warn!(
                    "Can't keep up! Did the system time change, or is the server overloaded? Running {}ms behind, skipping {} tick(s)",
                    elapsed,
                    elapsed / 50
                );
                elapsed = 2000;
                last_warning = Some(current);
            }
            pending += elapsed;
            current = now;
            while pending > TICK_MS as i64 {
                pending -= TICK_MS as i64;
                if !self.tick(&mut rx) {
                    self.shutdown();
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(
                (TICK_MS as i64 - pending).max(1) as u64
            ));
        }
    }

    /// One tick; returns `false` when the server should stop.
    fn tick(&mut self, rx: &mut UnboundedReceiver<GameEvent>) -> bool {
        self.tick_count += 1;
        let mut commands = Vec::new();
        // Vanilla runs play packets as scheduled tasks at the start of the tick but
        // accepts logins in the network phase at its end; a joining player's packets
        // wait for its login.
        let mut late: Vec<GameEvent> = Vec::new();
        let mut joining: Vec<u64> = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(GameEvent::Shutdown) | Err(TryRecvError::Disconnected) => return false,
                Ok(GameEvent::Console(line)) => commands.push(line),
                Ok(event @ GameEvent::Join { conn_id, .. }) => {
                    joining.push(conn_id);
                    late.push(event);
                }
                Ok(event) if event.conn_id().is_some_and(|c| joining.contains(&c)) => {
                    late.push(event);
                }
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
        self.update_skylight_subtracted();
        self.tick_time();
        self.tick_scheduled();
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
        self.flush_block_changes();
        self.send_block_events();
        // World.updateEntities: entities update in the order they were added, which is
        // entity id order; dead items are removed when the loop reaches them.
        let mut order: Vec<i32> = self.players.iter().map(|p| p.entity_id).collect();
        order.extend(self.items.keys().copied());
        order.extend(self.orbs.keys().copied());
        order.extend(self.falling.keys().copied());
        order.sort_unstable();
        for id in order {
            if self.items.contains_key(&id) {
                self.update_item(id);
                continue;
            }
            if self.orbs.contains_key(&id) {
                self.update_orb(id);
                continue;
            }
            if self.falling.contains_key(&id) {
                self.update_falling(id);
                continue;
            }
            let Some(i) = self.players.iter().position(|p| p.entity_id == id) else {
                continue;
            };
            // Vanilla accepts logins in the network phase at the end of a tick, so a new
            // player's first update happens on the following tick (after its packets).
            if self.players[i].joined_tick != self.tick_count {
                self.tick_player(i, radius);
                // World.updateEntityWithOptionalForce: move the entity to its new chunk.
                let p = &mut self.players[i];
                p.entity_chunk = (
                    krol_java::math::floor_f64(p.pos.0) >> 4,
                    krol_java::math::floor_f64(p.pos.2) >> 4,
                );
            }
        }
        // World.updateEntities: then the tile entities.
        self.tick_chests();
        self.tick_furnaces();
        self.tick_hoppers();
        self.tick_moving();
        self.update_tracked_entities();
        for event in late {
            self.handle_event(event);
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

    /// Vanilla `updateTimeAndWeatherForPlayer`: world border, time and rain.
    fn send_time_and_weather(&self, i: usize) {
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
        let player = &self.players[i];
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
            // The spawn fuzz depends on the world's game type, not the player's.
            None => (self.spawn_position(self.config.gamemode), (0.0, 0.0)),
        };
        let profile_id = profile.id;
        let entity_id = self.next_entity_id;
        self.next_entity_id += 1;
        let loaded = stored.is_some();
        // Vanilla reads what is stored and keeps defaults for everything missing, then
        // writes a complete record back.
        let mut data = default_player_data(gamemode);
        let mut stored_game_type = false;
        if let Some(stored) = stored {
            // Entity.readFromNBT reads a missing "Air" as 0 (getShort's default).
            if !stored.contains_key("Air") {
                data.insert("Air", krol_nbt::Tag::Short(0));
            }
            stored_game_type = stored.contains_key("playerGameType");
            for (k, v) in &stored {
                data.insert(k.clone(), v.clone());
            }
        }
        if loaded && data.get_i32("XpSeed") == 0 {
            // Vanilla EntityPlayer.readEntityFromNBT: a zero seed is replaced with the
            // entity's (unseeded) random; fresh players keep 0.
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as i64);
            let value = krol_java::JavaRandom::new(seed).next_int();
            data.insert("XpSeed", krol_nbt::Tag::Int(value));
        }
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

        // setPlayerGameTypeBasedOnOther: the game mode update goes to everyone already
        // online (the new player has no connection handler yet).
        let update = ClientboundPlay::PlayerListItem(PlayerListAction::UpdateGamemode(vec![(
            profile_id, gamemode,
        )]));
        // Vanilla quirk: reading a stored game type already set it once
        // (EntityPlayerMP.readEntityFromNBT), so stored players announce it twice.
        for _ in 0..if stored_game_type { 2 } else { 1 } {
            for p in &self.players {
                p.send(update.clone());
            }
        }
        // A new entity starts off the ground; stored players read `OnGround`.
        let on_ground = loaded && data.get_bool("OnGround");
        let inventory = player::inventory_window(&data);
        let selected = data.get_i32("SelectedItemSlot").clamp(0, 8) as usize;
        let player = Player {
            conn_id,
            entity_id,
            profile,
            peer,
            version,
            out,
            pos,
            rotation,
            on_ground,
            gamemode,
            skin_parts: 0,
            ticks: 0,
            last_keep_alive: 0,
            watching: Default::default(),
            chunk_queue: Default::default(),
            managed_pos: (0.0, 0.0),
            movement: Default::default(),
            new_watches: Vec::new(),
            joined_tick: self.tick_count,
            inventory,
            sent_inventory: vec![None; 45],
            selected,
            stats: stats::Stats::load(&self.level.dir, profile_id),
            xp_dirty: false,
            dirty_metadata: tracker::PLAYER_METADATA_INDICES.into_iter().collect(),
            dirty_attributes: true,
            entity_chunk: (
                krol_java::math::floor_f64(pos.0) >> 4,
                krol_java::math::floor_f64(pos.2) >> 4,
            ),
            swing: (false, 0),
            last_equipment: Default::default(),
            motion: {
                let m = data.get_list("Motion", krol_nbt::TagType::Double);
                let f = |k: usize| {
                    m.and_then(|l| l.get(k))
                        .and_then(|t| t.as_f64())
                        .unwrap_or(0.0)
                };
                (f(0), f(1), f(2))
            },
            sneaking: false,
            sprinting: false,
            flying: data
                .get_compound("abilities")
                .is_some_and(|a| a.get_bool("flying")),
            airborne: false,
            fall_distance: data.get_f32("FallDistance"),
            in_water: false,
            first_update: true,
            using: None,
            // Vanilla quirk: loaded effects do not set potionsNeedUpdate, so their
            // particle color stays off until an effect changes.
            effects: potions::read_effects(&data),
            potions_dirty: false,
            effect_color: 0,
            effect_ambient: false,
            collided_horizontally: false,
            distance_walked_on_step: 0.0,
            next_step_distance: 1,
            hurt_resistant_time: 0,
            xp_cooldown: 0,
            last_damage: 0.0,
            velocity_changed: false,
            respawn_invulnerability: 60,
            rand: krol_java::JavaRandom::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as i64)
                    ^ i64::from(entity_id),
            ),
            last_health_sent: None,
            dig: digging::DigState::new(),
            pending_destroy: Vec::new(),
            combat: Default::default(),
            window: Default::default(),
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
        // spawnEntityInWorld: start tracking the new player.
        self.track_player(new_index);
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

        self.send_time_and_weather(new_index);
        for packet in self.effect_packets(new_index) {
            self.players[new_index].send(packet);
        }
        let player = &self.players[new_index];
        player.send(ClientboundPlay::WindowItems {
            window: 0,
            items: player.inventory.clone(),
        });
        player.send(ClientboundPlay::SetSlot {
            window: -1,
            slot: -1,
            item: None,
        });
        // Container.onCraftGuiOpened checks for changes right away.
        self.detect_inventory_changes(new_index);
    }

    /// `Container.detectAndSendChanges` for the player window (vanilla quirk: the
    /// tracking starts empty, so the first pass resends every non-empty slot).
    pub(super) fn detect_inventory_changes(&mut self, i: usize) {
        let player = &mut self.players[i];
        for slot in 0..45 {
            if player.inventory[slot] != player.sent_inventory[slot] {
                player.sent_inventory[slot] = player.inventory[slot].clone();
                // EntityPlayerMP.sendSlotContents: the crafting result is never sent (the
                // client works it out itself).
                if slot == 0 {
                    continue;
                }
                player.send(ClientboundPlay::SetSlot {
                    window: 0,
                    slot: slot as i16,
                    item: player.inventory[slot].clone(),
                });
            }
        }
    }

    fn tick_player(&mut self, i: usize, radius: i32) {
        // EntityPlayerMP.onUpdate: the open container's changes, then whether it can
        // stay open.
        if self.detect_container_changes(i).is_none() {
            self.detect_inventory_changes(i);
        }
        self.check_open_container(i);
        let player = &mut self.players[i];
        // EntityPlayerMP.onUpdate: entities removed since the last update.
        if !player.pending_destroy.is_empty() {
            let ids = std::mem::take(&mut player.pending_destroy);
            player.send(ClientboundPlay::DestroyEntities(ids));
        }
        player.ticks += 1;
        // EntityPlayerMP.onUpdate.
        self.tick_digging(i);
        let player = &mut self.players[i];
        player.respawn_invulnerability -= 1;
        if player.hurt_resistant_time > 0 {
            player.hurt_resistant_time -= 1;
        }
        if (player.ticks - 1) % 20 == 0 {
            // EntityPlayerMP.updateBiomesExplored (every 20 ticks).
            let (x, z) = (
                krol_java::math::floor_f64(player.pos.0),
                krol_java::math::floor_f64(player.pos.2),
            );
            let biome = self.level.overworld.chunk(x >> 4, z >> 4).biomes
                [((z & 15) << 4 | (x & 15)) as usize];
            let player = &mut self.players[i];
            player
                .stats
                .add_progress("achievement.exploreAllBiomes", stats::biome_name(biome));
        }
        let player = &mut self.players[i];
        // EntityLivingBase.updateArmSwingProgress (swing duration 6 without haste).
        if player.swing.0 {
            player.swing.1 += 1;
            if player.swing.1 >= 6 {
                player.swing = (false, 0);
            }
        } else {
            player.swing.1 = 0;
        }
        let player = &mut self.players[i];
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
        for packet in self.chunk_descriptions(&batch) {
            self.players[i].send(packet);
        }
        self.chunks_sent(i, &batch);
    }

    /// Vanilla `PlayerManager.updatePlayerInstances`: send each changed chunk's block
    /// changes to the players who have that chunk (one change: Block Change; up to 63:
    /// Multi Block Change; 64: the changed sections).
    fn flush_block_changes(&mut self) {
        let mut changes = std::mem::take(&mut self.level.overworld.pending_changes);
        let order = std::mem::take(&mut self.level.overworld.pending_order);
        for (cx, cz) in order {
            let Some((blocks, section_mask)) = changes.remove(&(cx, cz)) else {
                continue;
            };
            // The changed blocks' tile entity data follows the block packet.
            let mut tiles = Vec::new();
            if blocks.len() < 64 {
                for &b in &blocks {
                    let pos = (
                        cx * 16 + i32::from(b >> 12),
                        i32::from(b & 0xff),
                        cz * 16 + i32::from((b >> 8) & 15),
                    );
                    tiles.extend(self.block_description(pos));
                }
            } else {
                for section in 0..16 {
                    if section_mask & (1 << section) == 0 {
                        continue;
                    }
                    for t in self.level.overworld.chunk(cx, cz).tile_entities.iter() {
                        if let krol_nbt::Tag::Compound(c) = t
                            && c.get_i32("y") >> 4 == section
                            && let Some(p) = sign::description(c)
                        {
                            tiles.push(p);
                        }
                    }
                }
            }
            let packet = if blocks.len() == 1 {
                let b = blocks[0];
                let (x, y, z) = (
                    cx * 16 + i32::from(b >> 12),
                    i32::from(b & 0xff),
                    cz * 16 + i32::from((b >> 8) & 15),
                );
                ClientboundPlay::BlockChange {
                    pos: (x, y, z),
                    state: i32::from(self.level.overworld.block(x, y, z)),
                }
            } else if blocks.len() < 64 {
                let records = blocks
                    .iter()
                    .map(|&b| {
                        let (lx, y, lz) =
                            ((b >> 12) as u8, (b & 0xff) as u8, ((b >> 8) & 15) as u8);
                        let state = self.level.overworld.block(
                            cx * 16 + i32::from(lx),
                            i32::from(y),
                            cz * 16 + i32::from(lz),
                        );
                        (lx, y, lz, i32::from(state))
                    })
                    .collect();
                ClientboundPlay::MultiBlockChange {
                    chunk_x: cx,
                    chunk_z: cz,
                    records,
                }
            } else {
                let sky = self.level.overworld.has_sky;
                let (mask, data) =
                    self.level
                        .overworld
                        .chunk(cx, cz)
                        .packet_data(false, section_mask, sky);
                ClientboundPlay::ChunkData {
                    column: ChunkColumn {
                        x: cx,
                        z: cz,
                        mask,
                        data,
                    },
                    full: false,
                }
            };
            for p in &self.players {
                if p.watching.contains(&(cx, cz)) && !p.chunk_queue.contains(&(cx, cz)) {
                    p.send(packet.clone());
                    for t in &tiles {
                        p.send(t.clone());
                    }
                }
            }
        }
    }

    /// Vanilla `DedicatedServer.isBlockProtected` (spawn protection).
    pub(super) fn is_block_protected(&mut self, i: usize, pos: (i32, i32, i32)) -> bool {
        let key = crate::status::java_uuid_string(self.players[i].profile.id);
        let (ops_empty, is_op) = match self.shared.user_lists.lock() {
            Ok(mut l) => {
                let empty = l.ops.iter().next().is_none();
                (empty, l.ops.contains(&key))
            }
            Err(_) => (true, false),
        };
        if ops_empty || is_op {
            return false;
        }
        let size = self.props.get_int("spawn-protection", 16);
        if size <= 0 {
            return false;
        }
        let d = &self.level.data;
        let k = (pos.0 - d.spawn_x()).abs().max((pos.2 - d.spawn_z()).abs());
        k <= size
    }

    /// Vanilla `processCreativeInventoryAction`.
    fn creative_inventory_action(
        &mut self,
        i: usize,
        slot: i16,
        item: Option<krol_protocol::packets::play::ItemStack>,
    ) {
        let player = &mut self.players[i];
        if player.gamemode != 1 {
            return;
        }
        let valid_item = item
            .as_ref()
            .is_none_or(|s| krol_world::items::name_for_id(s.id).is_some());
        let valid_size = item
            .as_ref()
            .is_none_or(|s| s.damage >= 0 && s.count <= 64 && s.count > 0);
        if (1..45).contains(&slot) && valid_item && valid_size {
            player.inventory[slot as usize] = item;
            if (1..=4).contains(&slot) {
                self.update_craft_result(i);
            }
        }
        // TODO: slot -1 drops the item as an entity (needs item entities).
    }

    /// Vanilla `processPlayerDigging` for start/abort/stop.
    fn digging(&mut self, i: usize, status: i8, pos: (i32, i32, i32)) {
        if matches!(status, 3 | 4) {
            // processPlayerDigging: drop one item or the whole stack.
            if self.players[i].gamemode != 3 {
                self.drop_held(i, status == 3);
            }
            return;
        }
        if status == RELEASE_USE_ITEM {
            // stopUsingItem. TODO(M5): bows shoot (onPlayerStoppedUsing).
            self.clear_item_in_use(i);
            return;
        }
        if !matches!(status, 0..=2) {
            return;
        }
        let p = &self.players[i];
        let d0 = p.pos.0 - (f64::from(pos.0) + 0.5);
        let d1 = p.pos.1 - (f64::from(pos.1) + 0.5) + 1.5;
        let d2 = p.pos.2 - (f64::from(pos.2) + 0.5);
        if d0 * d0 + d1 * d1 + d2 * d2 > 36.0 || pos.1 >= self.config.max_build_height {
            return;
        }
        match status {
            0 => {
                if self.is_block_protected(i, pos) {
                    self.send_block(i, pos);
                } else if self.players[i].gamemode == 1 {
                    self.block_clicked(i, pos);
                } else {
                    self.start_digging(i, pos);
                }
            }
            _ => {
                if status == 2 {
                    self.stop_digging(i, pos);
                } else {
                    self.cancel_digging(i);
                }
                if self.level.overworld.block(pos.0, pos.1, pos.2) != 0 {
                    self.send_block(i, pos);
                }
            }
        }
    }

    fn send_block(&mut self, i: usize, pos: (i32, i32, i32)) {
        let state = i32::from(self.level.overworld.block(pos.0, pos.1, pos.2));
        self.players[i].send(ClientboundPlay::BlockChange { pos, state });
    }

    /// Vanilla `ItemInWorldManager.onBlockClicked` (creative: instant break).
    fn block_clicked(&mut self, i: usize, pos: (i32, i32, i32)) {
        if self.players[i].gamemode != 1 {
            // Survival digging is not implemented yet; adventure/spectator cannot dig.
            return;
        }
        // Creative players holding a sword cannot break blocks.
        if self.players[i]
            .held()
            .is_some_and(|s| matches!(s.id, 267 | 268 | 272 | 276 | 283))
        {
            return;
        }
        let state = self.level.overworld.block(pos.0, pos.1, pos.2);
        self.break_effect(i, pos, state);
        // BlockPistonExtension.onBlockHarvested: in creative the base goes first.
        if state >> 4 == ids::block::PISTON_HEAD {
            let base = placement::offset(pos, placement::opposite(pistons::facing(state & 15)));
            if pistons::is_piston(self.level.overworld.block(base.0, base.1, base.2) >> 4) {
                self.set_block_notify(base, 0);
            }
        }
        self.drop_container_contents(pos);
        self.set_block_notify(pos, 0);
        self.destroyed_by_player(pos, state);
        self.send_block(i, pos);
    }

    /// Vanilla `processPlayerBlockPlacement` (block faces only).
    fn block_placement(
        &mut self,
        i: usize,
        pos: (i32, i32, i32),
        face: i8,
        packet_item: Option<krol_protocol::packets::play::ItemStack>,
        cursor: (i8, i8, i8),
    ) {
        if face == -1 {
            let Some(held) = self.players[i].held().cloned() else {
                return;
            };
            self.try_use_item(i, held);
            self.after_placement(i, packet_item);
            return;
        }
        let offset = |p: (i32, i32, i32), f: i8| match f {
            0 => (p.0, p.1 - 1, p.2),
            1 => (p.0, p.1 + 1, p.2),
            2 => (p.0, p.1, p.2 - 1),
            3 => (p.0, p.1, p.2 + 1),
            4 => (p.0 - 1, p.1, p.2),
            _ => (p.0 + 1, p.1, p.2),
        };
        let limit = self.config.max_build_height;
        if pos.1 < limit - 1 || (face != 1 && pos.1 < limit) {
            let p = &self.players[i];
            let (dx, dy, dz) = (
                f64::from(pos.0) + 0.5 - p.pos.0,
                f64::from(pos.1) + 0.5 - p.pos.1,
                f64::from(pos.2) + 0.5 - p.pos.2,
            );
            if p.movement.has_moved
                && dx * dx + dy * dy + dz * dz < 64.0
                && !self.is_block_protected(i, pos)
            {
                self.place_block(i, pos, face, cursor);
            }
        } else {
            let msg = crate::chat::Component::translate(
                "build.tooHigh",
                vec![crate::chat::Arg::Str(limit.to_string())],
            )
            .color("red");
            self.players[i].send(ClientboundPlay::Chat {
                json: msg.to_json_string(),
                position: 1,
            });
        }
        self.send_block(i, pos);
        self.send_block(i, offset(pos, face));
        self.after_placement(i, packet_item);
    }

    /// The end of `processPlayerBlockPlacement`: an emptied held stack is cleared and
    /// the held slot resynced if the client's stack differs.
    fn after_placement(
        &mut self,
        i: usize,
        packet_item: Option<krol_protocol::packets::play::ItemStack>,
    ) {
        let player = &mut self.players[i];
        let held = 36 + player.selected;
        if player.inventory[held]
            .as_ref()
            .is_some_and(|s| s.count == 0)
        {
            player.inventory[held] = None;
        }
        // Items with a use duration (food, bows, swords) are not resynced.
        if player.inventory[held]
            .as_ref()
            .is_some_and(|s| item_data::use_duration(s.id, s.damage) > 0)
        {
            return;
        }
        // isChangingQuantityOnly: the open window (possibly one this click opened) is
        // synced without packets, then the held slot is sent in that window's numbering
        // if the client's idea of the stack differs.
        let item = player.inventory[held].clone();
        let (window, slot) = self.held_window_slot(i);
        self.sync_window_silently(i);
        if item != packet_item {
            self.players[i].send(ClientboundPlay::SetSlot {
                window,
                slot: slot as i16,
                item,
            });
        }
    }

    /// `ItemInWorldManager.tryUseItem`: right-clicking with an item, not at a block.
    fn try_use_item(&mut self, i: usize, held: krol_protocol::packets::play::ItemStack) {
        if self.players[i].gamemode == 3 {
            return;
        }
        let result = match held.id {
            ids::item::BUCKET | ids::item::WATER_BUCKET | ids::item::LAVA_BUCKET => {
                self.use_bucket(i, &held)
            }
            id if item_data::armor_points(id) > 0 => self.equip_armor(i, &held),
            ids::item::MILK_BUCKET => {
                self.start_drinking_milk(i, &held);
                buckets::UseResult::Kept
            }
            id if food::food(id, held.damage).is_some() => {
                if let Some(f) = food::food(id, held.damage) {
                    self.start_eating(i, &held, f);
                }
                buckets::UseResult::Kept
            }
            // TODO(M4): food, potions, bows, throwables and the other usable items.
            _ => buckets::UseResult::Unchanged,
        };
        let slot = 36 + self.players[i].selected;
        let mut new = match result {
            buckets::UseResult::Unchanged => return,
            buckets::UseResult::Resized | buckets::UseResult::Kept => {
                self.players[i].inventory[slot].clone()
            }
            buckets::UseResult::Replaced(s) => s,
        };
        if self.players[i].gamemode == 1
            && let Some(s) = new.as_mut()
        {
            s.count = held.count;
            if item_data::max_damage(s.id) > 0 {
                s.damage = held.damage;
            }
        }
        if new.as_ref().is_some_and(|s| s.count == 0) {
            new = None;
        }
        let p = &mut self.players[i];
        p.inventory[slot] = new;
        if p.using.is_some() {
            return;
        }
        // sendContainerToPlayer(inventoryContainer), unless the item is now in use.
        p.send(ClientboundPlay::WindowItems {
            window: 0,
            items: p.inventory.clone(),
        });
        p.send(ClientboundPlay::SetSlot {
            window: -1,
            slot: -1,
            item: p.window.cursor.clone(),
        });
    }

    /// `ItemArmor.onItemRightClick`: the armor goes into its empty armor slot.
    ///
    /// TODO(M5): pumpkins and skulls go on the head too.
    fn equip_armor(
        &mut self,
        i: usize,
        held: &krol_protocol::packets::play::ItemStack,
    ) -> buckets::UseResult {
        // Helmet, chestplate, leggings, boots: window slots 5 to 8.
        let slot = 5 + ((held.id - ARMOR_FIRST_ID) % 4) as usize;
        let p = &mut self.players[i];
        if p.inventory[slot].is_some() {
            return buckets::UseResult::Unchanged;
        }
        p.inventory[slot] = Some(held.clone());
        let hand = 36 + p.selected;
        if let Some(s) = p.inventory[hand].as_mut() {
            s.count = 0;
        }
        buckets::UseResult::Resized
    }

    /// Vanilla `ItemBlock.onItemUse` / `ItemSlab.onItemUse` for the block items krol
    /// supports (see `placement`).
    fn place_block(&mut self, i: usize, pos: (i32, i32, i32), face: i8, cursor: (i8, i8, i8)) {
        let player = &self.players[i];
        // TODO(M5): spectators open containers instead.
        if player.gamemode == 3 {
            return;
        }
        // activateBlockOrUseItem: the block reacts first unless the player sneaks with an
        // item in hand.
        if !(player.sneaking && player.held().is_some()) && self.activate_block(i, pos) {
            return;
        }
        let player = &self.players[i];
        // Adventure players may not place blocks (no CanPlaceOn support yet).
        if player.gamemode == 2 {
            return;
        }
        let Some(stack) = player.held().cloned() else {
            return;
        };
        if stack.count <= 0 {
            return;
        }
        if self.place_door(i, stack.id, pos, face as u8) {
            return;
        }
        if stack.id == ids::item::FLINT_AND_STEEL {
            self.use_flint_and_steel(i, pos, face as u8);
            return;
        }
        if stack.id == ids::item::REPEATER {
            self.place_diode(i, pos, face as u8, ids::block::UNPOWERED_REPEATER);
            return;
        }
        if stack.id == ids::item::COMPARATOR {
            self.place_diode(i, pos, face as u8, ids::block::UNPOWERED_COMPARATOR);
            return;
        }
        if stack.id == ids::item::REDSTONE {
            self.place_wire(i, pos, face as u8);
            return;
        }
        if stack.id == ids::item::SIGN {
            self.place_sign(i, pos, face as u8);
            return;
        }
        let player = &self.players[i];
        let click = placement::Click {
            face: face as u8,
            hit: (
                f32::from(cursor.0) / 16.0,
                f32::from(cursor.1) / 16.0,
                f32::from(cursor.2) / 16.0,
            ),
            yaw: player.rotation.0,
            pos: player.pos,
        };
        // ItemSlab: a slab on a matching half slab makes a double slab.
        if let Some((single, double)) = placement::slab_pair(stack.id) {
            let variant = (stack.damage & 7) as u16;
            let clicked = self.level.overworld.block(pos.0, pos.1, pos.2);
            let merges_clicked = clicked >> 4 == single
                && clicked & 7 == variant
                && ((click.face == placement::UP && clicked & 8 == 0)
                    || (click.face == placement::DOWN && clicked & 8 != 0));
            let next = placement::offset(pos, click.face);
            let next_state = self.level.overworld.block(next.0, next.1, next.2);
            let target = if merges_clicked {
                Some(pos)
            } else if next_state >> 4 == single && next_state & 7 == variant {
                Some(next)
            } else {
                None
            };
            if let Some(at) = target {
                let state = double << 4 | variant;
                if !self.blocked_by_players(state, at) {
                    self.finish_placement(i, at, state);
                }
                return;
            }
        }
        let clicked = self.level.overworld.block(pos.0, pos.1, pos.2);
        let target = if blocks::is_replaceable(clicked) {
            pos
        } else {
            placement::offset(pos, click.face)
        };
        if !(0..256).contains(&target.1) {
            return;
        }
        // World.canBlockBePlaced: the target must be replaceable and no player may stand
        // in the block's (default state) collision box.
        if !blocks::is_replaceable(self.level.overworld.block(target.0, target.1, target.2)) {
            return;
        }
        let Some(state) = placement::placed_state(
            &mut self.level.overworld,
            stack.id,
            stack.damage,
            target,
            &click,
        ) else {
            return;
        };
        if self.blocked_by_players(state & !15, target) {
            return;
        }
        self.finish_placement(i, target, state);
        // BlockPistonBase.onBlockPlacedBy: checks whether to extend.
        if pistons::is_piston(state >> 4) {
            let now = self.level.overworld.block(target.0, target.1, target.2);
            if pistons::is_piston(now >> 4) {
                self.piston_check(target, now);
            }
        }
        // BlockChest.onBlockPlacedBy: line up with a neighboring chest.
        if ids::block::is_chest(state >> 4) {
            self.align_chests(target, state);
        }
        // onBlockPlacedBy of chests and furnaces: a renamed item names the container.
        if (ids::block::is_chest(state >> 4) || state >> 4 == ids::block::FURNACE)
            && let Some(name) = stack
                .nbt
                .as_ref()
                .and_then(|t| t.get_compound("display"))
                .filter(|d| d.contains_key_of("Name", Some(krol_nbt::TagType::String)))
                .map(|d| d.get_str("Name").to_owned())
            && let Some(te) = self.level.overworld.tile_entity_mut(target)
        {
            te.insert("CustomName", krol_nbt::Tag::String(name));
        }
    }

    /// `ItemReed.onItemUse` for repeaters and comparators: on a replaceable spot with a
    /// solid top below, facing away from the player.
    fn place_diode(&mut self, i: usize, pos: (i32, i32, i32), face: u8, block: u16) {
        let clicked = self.level.overworld.block(pos.0, pos.1, pos.2);
        let at = if blocks::is_replaceable(clicked) {
            pos
        } else {
            placement::offset(pos, face)
        };
        let w = &mut self.level.overworld;
        if !(0..256).contains(&at.1)
            || !blocks::is_replaceable(w.block(at.0, at.1, at.2))
            || !placement::solid_top(w.block(at.0, at.1 - 1, at.2))
        {
            return;
        }
        // getHorizontalFacing().getOpposite() as a horizontal index (south, west, north,
        // east).
        let yaw = self.players[i].rotation.0;
        let facing = krol_java::math::floor_f64(f64::from(yaw * 4.0 / 360.0) + 0.5) & 3;
        let state = block << 4 | ((facing + 2) & 3) as u16;
        if self.blocked_by_players(state, at) {
            return;
        }
        self.finish_placement(i, at, state);
        // BlockRedstoneDiode.onBlockPlacedBy.
        self.repeater_placed(at, state);
    }

    /// `ItemStack.onItemUse` succeeded: the held item's use statistic.
    pub(super) fn count_item_use(&mut self, i: usize) {
        let p = &mut self.players[i];
        if let Some(stat) = p.held().and_then(|s| digging::use_stat(s.id)) {
            p.stats.add(&stat, 1);
        }
    }

    /// Whether a player stands in the collision box of `state` at `at`
    /// (`checkNoEntityCollision`).
    fn blocked_by_players(&self, state: u16, at: (i32, i32, i32)) -> bool {
        let boxes = physics::block_boxes(state, at);
        self.players.iter().any(|p| {
            let bb = physics::Aabb::at(p.pos, 0.6, 1.8);
            boxes.iter().any(|b| b.intersects(&bb))
        })
    }

    /// Sets the placed block, plays its place sound to everyone within hearing distance
    /// and uses up one item in survival.
    fn finish_placement(&mut self, i: usize, at: (i32, i32, i32), state: u16) {
        self.set_block_notify(at, state);
        self.count_item_use(i);
        let sound = blocks::sound_type(state >> 4);
        let volume = (sound.volume + 1.0) / 2.0;
        let pitch = sound.frequency * 0.8;
        let center = (
            f64::from(at.0 as f32 + 0.5),
            f64::from(at.1 as f32 + 0.5),
            f64::from(at.2 as f32 + 0.5),
        );
        self.play_sound_near_except(None, &sound.place_sound(), center, volume, pitch);
        let player = &mut self.players[i];
        if player.gamemode != 1 {
            let held = 36 + player.selected;
            if let Some(s) = &mut player.inventory[held] {
                s.count -= 1;
            }
        }
    }

    /// The neighbor rule of `BlockChest.onBlockPlacedBy`.
    fn align_chests(&mut self, at: (i32, i32, i32), state: u16) {
        let id = state >> 4;
        let facing = (state & 15) as u8;
        let w = &mut self.level.overworld;
        let is = |w: &mut world::World, p: (i32, i32, i32)| w.block(p.0, p.1, p.2) >> 4 == id;
        let (n, so, we, ea) = (
            placement::offset(at, placement::NORTH),
            placement::offset(at, placement::SOUTH),
            placement::offset(at, placement::WEST),
            placement::offset(at, placement::EAST),
        );
        let (hn, hs, hw, he) = (is(w, n), is(w, so), is(w, we), is(w, ea));
        let x_axis = facing == placement::WEST || facing == placement::EAST;
        if !(hn || hs || hw || he) {
            return;
        }
        if !x_axis || (!hn && !hs) {
            if !x_axis && (hw || he) {
                let other = if hw { we } else { ea };
                self.set_block_notify(other, state);
            }
        } else {
            let other = if hn { n } else { so };
            self.set_block_notify(other, state);
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
                let p = &mut self.players[i];
                if p.skin_parts != skin_parts {
                    p.skin_parts = skin_parts;
                    p.dirty_metadata.insert(10);
                }
            }
            ServerboundPlay::EntityAction { action, .. } => self.entity_action(i, action),
            ServerboundPlay::ClientStatus { action } => self.client_status(i, action),
            ServerboundPlay::ClickWindow {
                window,
                slot,
                button,
                action,
                mode,
                item,
            } => self.click_window(i, window, slot, button, action, mode, item),
            ServerboundPlay::ConfirmTransaction { window, action, .. } => {
                self.confirm_transaction(i, window, action);
            }
            ServerboundPlay::CloseWindow { .. } => self.close_window(i),
            ServerboundPlay::UpdateSign { pos, lines } => self.update_sign(i, pos, &lines),
            ServerboundPlay::PlayerAbilities { flags, .. } => {
                // processPlayerAbilities: flying only where it is allowed.
                let p = &mut self.players[i];
                let allowed = p.gamemode == 1 || p.gamemode == 3;
                p.flying = flags & 2 != 0 && allowed;
                let flying = p.flying;
                if let Some(krol_nbt::Tag::Compound(a)) = p.data.get_mut("abilities") {
                    a.insert("flying", krol_nbt::Tag::Byte(i8::from(flying)));
                }
            }
            ServerboundPlay::Animation => {
                // EntityLivingBase.swingItem: viewers see the swing unless one is already
                // in its first half.
                let p = &mut self.players[i];
                if !p.swing.0 || p.swing.1 >= 3 || p.swing.1 < 0 {
                    p.swing = (true, -1);
                    let entity_id = p.entity_id;
                    self.send_to_tracking(
                        entity_id,
                        &ClientboundPlay::Animation {
                            entity_id,
                            animation: 0,
                        },
                    );
                }
            }
            ServerboundPlay::HeldItemChange { slot } => {
                if (0..9).contains(&slot) {
                    self.players[i].selected = slot as usize;
                } else {
                    tracing::warn!(
                        "{} tried to set an invalid carried item",
                        self.players[i].profile.name
                    );
                }
            }
            ServerboundPlay::CreativeInventoryAction { slot, item } => {
                self.creative_inventory_action(i, slot, item);
            }
            ServerboundPlay::Digging {
                status,
                pos,
                face: _,
            } => self.digging(i, status, pos),
            ServerboundPlay::BlockPlacement {
                pos,
                face,
                item,
                cursor,
            } => {
                self.block_placement(i, pos, face, item, cursor);
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
        player::set_abilities(&mut player.data, mode);
        player.flying = player
            .data
            .get_compound("abilities")
            .is_some_and(|a| a.get_bool("flying"));
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
        self.players[i].stats.add("stat.leaveGame", 1);
        let dir = self.level.dir.clone();
        self.players[i].save(&dir);
        // World.removeEntity: viewers destroy the player entity.
        self.untrack_player(i);
        let player = self.players.remove(i);
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
