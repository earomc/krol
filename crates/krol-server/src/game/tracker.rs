//! Which clients see which entities, and the packets that keep them in sync
//! (vanilla: `EntityTracker`, `EntityTrackerEntry`).
//!
//! Players, dropped items, experience orbs, falling blocks, primed TNT, projectiles,
//! paintings, item frames and armor stands are tracked so far. Every tracked entity has
//! an entry holding what its viewers were last told (position in 1/32 blocks, angles in
//! 1/256 turns). Each tick, after the entity updates, every entry:
//!
//! - re-checks which players can see it, on its first update or after moving more than
//!   4 blocks (a new player's first update also makes every other entry check it),
//! - every `frequency` ticks (or when its metadata changed) sends a relative move, look,
//!   look-and-move or teleport, then the changed metadata and attributes, then the head
//!   rotation.
//!
//! A player sees an entity when the entity is within the tracking range on both
//! horizontal axes and the chunk the entity is in has already been sent to the player.
//! Entities in a chunk are also spawned for a player right when that chunk is sent.

use std::collections::{BTreeMap, BTreeSet};

use krol_java::math::{floor_f32, floor_f64};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack, Metadata, MetadataValue};

use super::Game;
use super::player::Player;

/// Vanilla tracking range of players (`EntityTracker.trackEntity`), before the clamp
/// to the view distance.
const PLAYER_RANGE: i32 = 512;
/// Players are updated every 2 ticks.
const PLAYER_FREQUENCY: u32 = 2;
/// Items: range 64, every 20 ticks, with velocity updates.
const ITEM_RANGE: i32 = 64;
const ITEM_FREQUENCY: u32 = 20;
/// Falling blocks: range 160, every 20 ticks, with velocity updates.
const FALLING_RANGE: i32 = 160;
const FALLING_FREQUENCY: u32 = 20;
/// `S0EPacketSpawnObject` type of falling blocks.
const FALLING_OBJECT: i8 = 70;
/// Primed TNT: range 160, every 10 ticks, with velocity updates; spawn object type 50.
const TNT_RANGE: i32 = 160;
const TNT_FREQUENCY: u32 = 10;
const TNT_OBJECT: i8 = 50;
/// Snowballs, eggs, ender pearls and experience bottles: range 64, every 10 ticks, with
/// velocity updates.
const THROWN_RANGE: i32 = 64;
const THROWN_FREQUENCY: u32 = 10;
/// Arrows: range 64, every 20 ticks, no velocity updates.
const ARROW_RANGE: i32 = 64;
const ARROW_FREQUENCY: u32 = 20;
/// Paintings and item frames: range 160, never updated after the first time
/// (`Integer.MAX_VALUE`), no velocity updates.
const HANGING_RANGE: i32 = 160;
const HANGING_FREQUENCY: u32 = i32::MAX as u32;
/// Item frames send their changed metadata every 10 ticks.
const ITEM_FRAME_METADATA_INTERVAL: u32 = 10;
/// Armor stands: range 160, every 3 ticks, with velocity updates.
const ARMOR_STAND_RANGE: i32 = 160;
const ARMOR_STAND_FREQUENCY: u32 = 3;
/// Mobs (`IAnimals`): range 80, every 3 ticks, with velocity updates.
const MOB_RANGE: i32 = 80;
const MOB_FREQUENCY: u32 = 3;
/// Minecarts: range 80, every 3 ticks, with velocity updates.
const MINECART_RANGE: i32 = 80;
const MINECART_FREQUENCY: u32 = 3;
/// Experience orbs: range 160, every 20 ticks, with velocity updates.
const ORB_RANGE: i32 = 160;
const ORB_FREQUENCY: u32 = 20;

/// One tracked entity (vanilla `EntityTrackerEntry`).
#[derive(Debug)]
pub struct TrackerEntry {
    range: i32,
    frequency: u32,
    encoded_pos: (i32, i32, i32),
    encoded_rot: (i32, i32),
    last_head: i32,
    last_tracked: (f64, f64, f64),
    first_update_done: bool,
    update_counter: u32,
    ticks_since_teleport: u32,
    on_ground: bool,
    velocity_updates: bool,
    last_motion: (f64, f64, f64),
    /// Entity ids of the players that see this entity (vanilla `trackingPlayers`).
    tracking: BTreeSet<i32>,
    /// The vehicle the viewers were last told about (`field_85178_v`).
    last_vehicle: Option<i32>,
    /// The entity rode something at the last update (`ridingEntity`): the first update
    /// after it gets off sends a teleport.
    was_riding: bool,
}

impl TrackerEntry {
    /// Whether the player with this entity id sees the entity.
    pub fn is_tracked_by(&self, viewer: i32) -> bool {
        self.tracking.contains(&viewer)
    }

    /// Forgets that the player sees the entity (no packet).
    pub fn stop_tracking(&mut self, viewer: i32) {
        self.tracking.remove(&viewer);
    }
}

/// All tracked entities, keyed by entity id.
///
/// Vanilla keeps the entries in a `HashSet` whose hash is the entity id, so with the few
/// small ids krol has so far its iteration order is ascending id.
/// TODO(M5): reproduce `HashSet` order once ids exceed the table size.
pub type Tracker = BTreeMap<i32, TrackerEntry>;

/// Angle in 1/256 turns, as the tracker computes it (`MathHelper.floor_float`).
fn angle(degrees: f32) -> i32 {
    floor_f32(degrees * 256.0 / 360.0)
}

/// Position in 1/32 blocks.
fn fixed(v: f64) -> i32 {
    floor_f64(v * 32.0)
}

/// The complete data watcher of a player, in index order.
pub fn player_metadata(p: &Player) -> Vec<Metadata> {
    use MetadataValue::{Byte, Float, Int, Short, String};
    let md = |index, value| Metadata { index, value };
    let d = &p.data;
    vec![
        md(
            0,
            Byte(
                i8::from(p.burning_flag)
                    | (i8::from(p.sneaking) << 1)
                    | (i8::from(p.sprinting) << 3)
                    | (i8::from(p.using.is_some()) << 4),
            ),
        ),
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
        md(7, Int(p.effect_color)),
        md(8, Byte(i8::from(p.effect_ambient))),
        md(ARROW_COUNT_INDEX, Byte(p.arrow_count)),
        md(10, Byte(p.skin_parts as i8)),
        md(16, Byte(0)),
        md(17, Float(d.get_f32("AbsorptionAmount"))),
        md(18, Int(d.get_i32("Score"))),
    ]
}

/// Metadata entry 9 of living entities: arrows stuck in the body.
pub const ARROW_COUNT_INDEX: u8 = 9;

/// All indices of a player's data watcher (everything is dirty on creation).
pub const PLAYER_METADATA_INDICES: [u8; 13] = [0, 1, 2, 3, 4, 6, 7, 8, 9, 10, 16, 17, 18];

/// The item in each equipment slot: held item, boots, leggings, chestplate, helmet.
pub fn equipment(p: &Player) -> [Option<ItemStack>; 5] {
    [
        p.held().cloned(),
        p.inventory[8].clone(),
        p.inventory[7].clone(),
        p.inventory[6].clone(),
        p.inventory[5].clone(),
    ]
}

/// What the tracker needs to know about a tracked entity.
struct View {
    pos: (f64, f64, f64),
    yaw: f32,
    pitch: f32,
    head: f32,
    on_ground: bool,
    motion: (f64, f64, f64),
    chunk: (i32, i32),
    airborne: bool,
    metadata_changed: bool,
    /// Index into `players` for players.
    player: Option<usize>,
}

impl Game {
    fn player_by_entity(&self, entity_id: i32) -> Option<usize> {
        self.players.iter().position(|p| p.entity_id == entity_id)
    }

    fn view(&self, entity_id: i32) -> Option<View> {
        if let Some(i) = self.player_by_entity(entity_id) {
            let p = &self.players[i];
            return Some(View {
                pos: p.pos,
                yaw: p.rotation.0,
                pitch: p.rotation.1,
                head: p.rotation.0,
                on_ground: p.on_ground,
                motion: p.motion,
                chunk: p.entity_chunk,
                airborne: p.airborne,
                metadata_changed: !p.dirty_metadata.is_empty(),
                player: Some(i),
            });
        }
        if let Some(f) = self.falling.get(&entity_id) {
            return Some(View {
                pos: f.body.pos,
                yaw: 0.0,
                pitch: 0.0,
                head: 0.0,
                on_ground: f.body.on_ground,
                motion: f.body.motion,
                chunk: f.chunk,
                airborne: false,
                metadata_changed: f.base.metadata_changed,
                player: None,
            });
        }
        if let Some(t) = self.tnt.get(&entity_id) {
            return Some(View {
                pos: t.body.pos,
                yaw: 0.0,
                pitch: 0.0,
                head: 0.0,
                on_ground: t.body.on_ground,
                motion: t.body.motion,
                chunk: t.chunk,
                airborne: false,
                metadata_changed: t.base.metadata_changed,
                player: None,
            });
        }
        if let Some(a) = self.arrows.get(&entity_id) {
            return Some(View {
                pos: a.body.pos,
                yaw: a.base.yaw,
                pitch: a.base.pitch,
                head: 0.0,
                on_ground: a.body.on_ground,
                motion: a.body.motion,
                chunk: a.chunk,
                airborne: false,
                metadata_changed: a.metadata_changed,
                player: None,
            });
        }
        if let Some(t) = self.thrown.get(&entity_id) {
            return Some(View {
                pos: t.body.pos,
                yaw: t.base.yaw,
                pitch: t.base.pitch,
                head: 0.0,
                on_ground: t.body.on_ground,
                motion: t.body.motion,
                chunk: t.chunk,
                airborne: false,
                metadata_changed: t.base.metadata_changed,
                player: None,
            });
        }
        if let Some(h) = self.hanging.get(&entity_id) {
            return Some(View {
                pos: h.body.pos,
                yaw: h.base.yaw,
                pitch: h.base.pitch,
                head: 0.0,
                on_ground: h.body.on_ground,
                motion: h.body.motion,
                chunk: h.chunk,
                airborne: h.airborne,
                metadata_changed: h.metadata_changed,
                player: None,
            });
        }
        if let Some(b) = self.boats.get(&entity_id) {
            return Some(View {
                pos: b.body.pos,
                yaw: b.base.yaw,
                pitch: b.base.pitch,
                head: 0.0,
                on_ground: b.body.on_ground,
                motion: b.body.motion,
                chunk: b.chunk,
                airborne: b.airborne,
                metadata_changed: b.metadata_changed,
                player: None,
            });
        }
        if let Some(m) = self.minecarts.get(&entity_id) {
            return Some(View {
                pos: m.body.pos,
                yaw: m.base.yaw,
                pitch: m.base.pitch,
                head: 0.0,
                on_ground: m.body.on_ground,
                motion: m.body.motion,
                chunk: m.chunk,
                airborne: m.airborne,
                metadata_changed: m.metadata_changed,
                player: None,
            });
        }
        if let Some(m) = self.mobs.get(&entity_id) {
            return Some(View {
                pos: m.body.pos,
                yaw: m.base.yaw,
                pitch: m.base.pitch,
                head: m.head_yaw,
                on_ground: m.body.on_ground,
                motion: m.body.motion,
                chunk: m.chunk,
                airborne: m.airborne,
                metadata_changed: m.metadata_changed,
                player: None,
            });
        }
        if let Some(s) = self.armor_stands.get(&entity_id) {
            return Some(View {
                pos: s.body.pos,
                yaw: s.base.yaw,
                pitch: s.base.pitch,
                head: s.head_yaw,
                on_ground: s.body.on_ground,
                motion: s.body.motion,
                chunk: s.chunk,
                airborne: false,
                metadata_changed: s.metadata_changed,
                player: None,
            });
        }
        if let Some(o) = self.orbs.get(&entity_id) {
            return Some(View {
                pos: o.body.pos,
                yaw: 0.0,
                pitch: 0.0,
                head: 0.0,
                on_ground: o.body.on_ground,
                motion: o.body.motion,
                chunk: o.chunk,
                airborne: false,
                metadata_changed: o.base.metadata_changed,
                player: None,
            });
        }
        let it = self.items.get(&entity_id)?;
        Some(View {
            pos: it.body.pos,
            yaw: it.base.yaw,
            pitch: it.base.pitch,
            head: 0.0,
            on_ground: it.body.on_ground,
            motion: it.body.motion,
            chunk: it.chunk,
            airborne: false,
            metadata_changed: !it.dirty_metadata.is_empty(),
            player: None,
        })
    }

    /// The tracking range after vanilla's clamp to `view-distance * 16 - 16`.
    fn tracking_range(&self, range: i32) -> i32 {
        range.min(self.config.view_distance * 16 - 16)
    }

    /// Sends `packet` to every player that sees `entity_id` (`sendToAllTrackingEntity`).
    pub(super) fn send_to_tracking(&self, entity_id: i32, packet: &ClientboundPlay) {
        if let Some(entry) = self.tracker.get(&entity_id) {
            for p in &self.players {
                if entry.tracking.contains(&p.entity_id) {
                    p.send(packet.clone());
                }
            }
        }
    }

    /// Like [`Self::send_to_tracking`], also sending to the entity itself if it is a
    /// player (`func_151261_b`).
    fn send_to_tracking_and_self(&self, entity_id: i32, packet: &ClientboundPlay) {
        if let Some(i) = self.player_by_entity(entity_id) {
            self.players[i].send(packet.clone());
        }
        self.send_to_tracking(entity_id, packet);
    }

    /// Starts tracking an entity (vanilla `EntityTracker.trackEntity`, run when it is
    /// added to the world).
    pub(super) fn track_entity(&mut self, entity_id: i32) {
        let Some(v) = self.view(entity_id) else {
            return;
        };
        let (range, frequency, velocity_updates) = if v.player.is_some() {
            (PLAYER_RANGE, PLAYER_FREQUENCY, false)
        } else if self.orbs.contains_key(&entity_id) {
            (ORB_RANGE, ORB_FREQUENCY, true)
        } else if self.falling.contains_key(&entity_id) {
            (FALLING_RANGE, FALLING_FREQUENCY, true)
        } else if self.tnt.contains_key(&entity_id) {
            (TNT_RANGE, TNT_FREQUENCY, true)
        } else if self.thrown.contains_key(&entity_id) {
            (THROWN_RANGE, THROWN_FREQUENCY, true)
        } else if self.arrows.contains_key(&entity_id) {
            (ARROW_RANGE, ARROW_FREQUENCY, false)
        } else if self.hanging.contains_key(&entity_id) {
            (HANGING_RANGE, HANGING_FREQUENCY, false)
        } else if self.armor_stands.contains_key(&entity_id) {
            (ARMOR_STAND_RANGE, ARMOR_STAND_FREQUENCY, true)
        } else if self.mobs.contains_key(&entity_id) {
            // IAnimals.
            (MOB_RANGE, MOB_FREQUENCY, true)
        } else if self.minecarts.contains_key(&entity_id) || self.boats.contains_key(&entity_id) {
            // Boats are tracked like minecarts.
            (MINECART_RANGE, MINECART_FREQUENCY, true)
        } else {
            (ITEM_RANGE, ITEM_FREQUENCY, true)
        };
        let entry = TrackerEntry {
            range: self.tracking_range(range),
            frequency,
            encoded_pos: (fixed(v.pos.0), fixed(v.pos.1), fixed(v.pos.2)),
            encoded_rot: (angle(v.yaw), angle(v.pitch)),
            last_head: angle(v.head),
            last_tracked: (0.0, 0.0, 0.0),
            first_update_done: false,
            update_counter: 0,
            ticks_since_teleport: 0,
            on_ground: v.on_ground,
            velocity_updates,
            last_motion: v.motion,
            tracking: BTreeSet::new(),
            last_vehicle: None,
            was_riding: false,
        };
        self.tracker.insert(entity_id, entry);
        self.update_player_entities(entity_id);
        if let Some(i) = v.player {
            // A new player is checked against every other entity.
            let others: Vec<i32> = self
                .tracker
                .keys()
                .copied()
                .filter(|&e| e != entity_id)
                .collect();
            for other in others {
                self.update_player_entity(other, i);
            }
        }
    }

    /// Starts tracking the player at index `i`.
    pub(super) fn track_player(&mut self, i: usize) {
        let id = self.players[i].entity_id;
        self.track_entity(id);
    }

    /// Stops tracking the player at index `i` (vanilla `EntityTracker.untrackEntity`):
    /// it no longer sees anything, and everyone who saw it gets Destroy Entities.
    pub(super) fn untrack_player(&mut self, i: usize) {
        let entity_id = self.players[i].entity_id;
        for entry in self.tracker.values_mut() {
            entry.tracking.remove(&entity_id);
        }
        if let Some(entry) = self.tracker.remove(&entity_id) {
            for p in &self.players {
                if entry.tracking.contains(&p.entity_id) {
                    p.send(ClientboundPlay::DestroyEntities(vec![entity_id]));
                }
            }
        }
    }

    /// `World.removeEntity` of a dead player (`EntityTracker.untrackEntity`): it stops
    /// seeing everything (players at once, other entities on its next update) and the
    /// others destroy it.
    pub(super) fn remove_dead_player(&mut self, i: usize) {
        let entity_id = self.players[i].entity_id;
        let seen: Vec<i32> = self
            .tracker
            .iter()
            .filter(|(_, e)| e.tracking.contains(&entity_id))
            .map(|(id, _)| *id)
            .collect();
        for id in seen {
            if let Some(entry) = self.tracker.get_mut(&id) {
                entry.tracking.remove(&entity_id);
            }
            if self.players.iter().any(|p| p.entity_id == id) {
                self.players[i].send(ClientboundPlay::DestroyEntities(vec![id]));
            } else {
                self.players[i].pending_destroy.push(id);
            }
        }
        self.untrack_player(i);
        self.players[i].removed = true;
    }

    /// Stops tracking a non-player entity; its viewers destroy it on their next update
    /// (`EntityPlayerMP.removeEntity` queues it in `destroyedItemsNetCache`).
    ///
    /// Only dead non-player entities are untracked, so this is also where they leave
    /// the scoreboard (`WorldManager.onEntityRemoved` -> `Scoreboard.func_181140_a`):
    /// their scores and team membership, held under their UUID, go.
    pub(super) fn untrack_entity(&mut self, entity_id: i32) {
        if let Some(uuid) = self.entity_uuids.remove(&entity_id) {
            let holder = crate::status::java_uuid_string(uuid);
            self.sb_reset(&holder, None);
            self.sb_leave_teams(&holder);
        }
        if let Some(entry) = self.tracker.remove(&entity_id) {
            for p in &mut self.players {
                if entry.tracking.contains(&p.entity_id) {
                    p.pending_destroy.push(entity_id);
                }
            }
        }
    }

    /// Spawns the entities in freshly sent chunks for the player at index `viewer`
    /// (vanilla `EntityTracker.func_85172_a`).
    pub(super) fn chunks_sent(&mut self, viewer: usize, chunks: &[(i32, i32)]) {
        let me = self.players[viewer].entity_id;
        for &c in chunks {
            let in_chunk: Vec<i32> = self
                .tracker
                .keys()
                .copied()
                .filter(|&e| e != me && self.view(e).is_some_and(|v| v.chunk == c))
                .collect();
            for e in in_chunk {
                self.update_player_entity(e, viewer);
            }
        }
    }

    /// Re-checks every player against the entity (`updatePlayerEntities`).
    fn update_player_entities(&mut self, entity_id: i32) {
        for viewer in 0..self.players.len() {
            self.update_player_entity(entity_id, viewer);
        }
    }

    /// The packets that make an entity appear for a player (`func_151260_c` and the
    /// rest of `updatePlayerEntity`).
    fn spawn_packets(&self, entity_id: i32, v: &View) -> Vec<ClientboundPlay> {
        let speed = |m: f64| (m.clamp(-3.9, 3.9) * 8000.0) as i32 as i16;
        let mut packets = Vec::new();
        if let Some(i) = v.player {
            let t = &self.players[i];
            packets.push(ClientboundPlay::SpawnPlayer {
                entity_id,
                uuid: t.profile.id,
                x: fixed(t.pos.0),
                y: fixed(t.pos.1),
                z: fixed(t.pos.2),
                yaw: (t.rotation.0 * 256.0 / 360.0) as i32 as i8,
                pitch: (t.rotation.1 * 256.0 / 360.0) as i32 as i8,
                current_item: t.held().map_or(0, |s| s.id),
                metadata: player_metadata(t),
            });
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata: player_metadata(t),
            });
            packets.push(ClientboundPlay::EntityProperties {
                entity_id,
                properties: super::attributes::properties(
                    &[
                        super::attributes::MOVEMENT_SPEED,
                        super::attributes::MAX_HEALTH,
                    ],
                    &t.modifiers,
                ),
            });
            if let Some(vehicle_id) = t.riding {
                packets.push(ClientboundPlay::AttachEntity {
                    entity_id,
                    vehicle_id,
                    leash: false,
                });
            }
            for (slot, item) in equipment(t).into_iter().enumerate() {
                if item.is_some() {
                    packets.push(ClientboundPlay::EntityEquipment {
                        entity_id,
                        slot: slot as i16,
                        item,
                    });
                }
            }
            // TODO(M5): use bed.
            packets.extend(self.effect_packets(i));
        } else if let Some(f) = self.falling.get(&entity_id) {
            let velocity = (speed(v.motion.0), speed(v.motion.1), speed(v.motion.2));
            packets.push(ClientboundPlay::SpawnObject {
                entity_id,
                kind: FALLING_OBJECT,
                x: fixed(v.pos.0),
                y: fixed(v.pos.1),
                z: fixed(v.pos.2),
                pitch: 0,
                yaw: 0,
                data: f.state_id(),
                velocity: Some(velocity),
            });
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata: f.metadata(),
            });
            packets.push(ClientboundPlay::EntityVelocity {
                entity_id,
                x: velocity.0,
                y: velocity.1,
                z: velocity.2,
            });
        } else if let Some(t) = self.tnt.get(&entity_id) {
            // Data 0: the spawn packet carries no velocity.
            packets.push(ClientboundPlay::SpawnObject {
                entity_id,
                kind: TNT_OBJECT,
                x: fixed(v.pos.0),
                y: fixed(v.pos.1),
                z: fixed(v.pos.2),
                pitch: 0,
                yaw: 0,
                data: 0,
                velocity: None,
            });
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata: t.metadata(),
            });
            packets.push(ClientboundPlay::EntityVelocity {
                entity_id,
                x: speed(v.motion.0),
                y: speed(v.motion.1),
                z: speed(v.motion.2),
            });
        } else if let Some(a) = self.arrows.get(&entity_id) {
            // The data is the shooter's entity id (the arrow's own without one); like any
            // data but 0 it brings the velocity. No velocity packet follows.
            let data = a.shooter.unwrap_or(entity_id);
            packets.push(ClientboundPlay::SpawnObject {
                entity_id,
                kind: super::arrow::OBJECT_TYPE,
                x: fixed(v.pos.0),
                y: fixed(v.pos.1),
                z: fixed(v.pos.2),
                pitch: angle(v.pitch) as i8,
                yaw: angle(v.yaw) as i8,
                data,
                velocity: (data != 0)
                    .then(|| (speed(v.motion.0), speed(v.motion.1), speed(v.motion.2))),
            });
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata: a.metadata(),
            });
        } else if let Some(t) = self.thrown.get(&entity_id) {
            // Only splash potions have data (their damage value); data 0 carries no
            // velocity.
            let data = t.potion_damage();
            packets.push(ClientboundPlay::SpawnObject {
                entity_id,
                kind: t.kind.object_type(),
                x: fixed(v.pos.0),
                y: fixed(v.pos.1),
                z: fixed(v.pos.2),
                pitch: angle(v.pitch) as i8,
                yaw: angle(v.yaw) as i8,
                data,
                velocity: (data != 0)
                    .then(|| (speed(v.motion.0), speed(v.motion.1), speed(v.motion.2))),
            });
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata: t.metadata(),
            });
            packets.push(ClientboundPlay::EntityVelocity {
                entity_id,
                x: speed(v.motion.0),
                y: speed(v.motion.1),
                z: speed(v.motion.2),
            });
        } else if let Some(h) = self.hanging.get(&entity_id) {
            match &h.kind {
                super::hanging::Kind::Painting { art } => {
                    packets.push(ClientboundPlay::SpawnPainting {
                        entity_id,
                        title: super::hanging::ARTS[*art].0.to_owned(),
                        pos: h.tile,
                        facing: h.facing,
                    });
                }
                super::hanging::Kind::ItemFrame { .. } => {
                    // The facing is the data; the position is the hanging position's
                    // corner.
                    let data = i32::from(h.facing);
                    packets.push(ClientboundPlay::SpawnObject {
                        entity_id,
                        kind: super::hanging::ITEM_FRAME_OBJECT,
                        x: h.tile.0.wrapping_mul(32),
                        y: h.tile.1.wrapping_mul(32),
                        z: h.tile.2.wrapping_mul(32),
                        pitch: angle(v.pitch) as i8,
                        yaw: angle(v.yaw) as i8,
                        data,
                        velocity: (data != 0)
                            .then(|| (speed(v.motion.0), speed(v.motion.1), speed(v.motion.2))),
                    });
                }
            }
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata: h.metadata(),
            });
        } else if let Some(m) = self.mobs.get(&entity_id) {
            // The spawn packet carries the motion and metadata; the metadata follows
            // again, then the watched attributes and what it holds and wears.
            packets.push(ClientboundPlay::SpawnMob {
                entity_id,
                kind: m.kind.type_id(),
                x: fixed(v.pos.0),
                y: fixed(v.pos.1),
                z: fixed(v.pos.2),
                // S0FPacketSpawnMob truncates its angles (toward zero), unlike the
                // tracker's floored ones.
                yaw: (v.yaw * 256.0 / 360.0) as i32 as i8,
                pitch: (v.pitch * 256.0 / 360.0) as i32 as i8,
                head_pitch: (v.head * 256.0 / 360.0) as i32 as i8,
                velocity: (speed(v.motion.0), speed(v.motion.1), speed(v.motion.2)),
                metadata: m.metadata(),
            });
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata: m.metadata(),
            });
            packets.push(ClientboundPlay::EntityProperties {
                entity_id,
                properties: m.properties(),
            });
            for (slot, item) in m.equipment.iter().enumerate() {
                if item.is_some() {
                    packets.push(ClientboundPlay::EntityEquipment {
                        entity_id,
                        slot: slot as i16,
                        item: item.clone(),
                    });
                }
            }
        } else if let Some(s) = self.armor_stands.get(&entity_id) {
            // Data 0: no velocity in the spawn packet, a velocity packet follows the
            // attributes. Then what it wears, like any living entity.
            packets.push(ClientboundPlay::SpawnObject {
                entity_id,
                kind: super::armor_stand::OBJECT_TYPE,
                x: fixed(v.pos.0),
                y: fixed(v.pos.1),
                z: fixed(v.pos.2),
                pitch: angle(v.pitch) as i8,
                yaw: angle(v.yaw) as i8,
                data: 0,
                velocity: None,
            });
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata: s.metadata(),
            });
            packets.push(ClientboundPlay::EntityProperties {
                entity_id,
                properties: s.properties(),
            });
            packets.push(ClientboundPlay::EntityVelocity {
                entity_id,
                x: speed(v.motion.0),
                y: speed(v.motion.1),
                z: speed(v.motion.2),
            });
            for (slot, item) in s.equipment.iter().enumerate() {
                if item.is_some() {
                    packets.push(ClientboundPlay::EntityEquipment {
                        entity_id,
                        slot: slot as i16,
                        item: item.clone(),
                    });
                }
            }
        } else if let Some(m) = self.minecarts.get(&entity_id) {
            // The minecart type is the data; type 0 brings no velocity, so a velocity
            // packet follows.
            let data = m.type_id();
            packets.push(ClientboundPlay::SpawnObject {
                entity_id,
                kind: super::minecart::OBJECT_TYPE,
                x: fixed(v.pos.0),
                y: fixed(v.pos.1),
                z: fixed(v.pos.2),
                pitch: angle(v.pitch) as i8,
                yaw: angle(v.yaw) as i8,
                data,
                velocity: (data != 0)
                    .then(|| (speed(v.motion.0), speed(v.motion.1), speed(v.motion.2))),
            });
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata: m.metadata(),
            });
            packets.push(ClientboundPlay::EntityVelocity {
                entity_id,
                x: speed(v.motion.0),
                y: speed(v.motion.1),
                z: speed(v.motion.2),
            });
        } else if let Some(b) = self.boats.get(&entity_id) {
            // Data 0 brings no velocity, so a velocity packet follows.
            packets.push(ClientboundPlay::SpawnObject {
                entity_id,
                kind: super::boat::OBJECT_TYPE,
                x: fixed(v.pos.0),
                y: fixed(v.pos.1),
                z: fixed(v.pos.2),
                pitch: angle(v.pitch) as i8,
                yaw: angle(v.yaw) as i8,
                data: 0,
                velocity: None,
            });
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata: b.metadata(),
            });
            packets.push(ClientboundPlay::EntityVelocity {
                entity_id,
                x: speed(v.motion.0),
                y: speed(v.motion.1),
                z: speed(v.motion.2),
            });
        } else if let Some(o) = self.orbs.get(&entity_id) {
            packets.push(ClientboundPlay::SpawnExperienceOrb {
                entity_id,
                x: floor_f64(v.pos.0 * 32.0),
                y: floor_f64(v.pos.1 * 32.0),
                z: floor_f64(v.pos.2 * 32.0),
                count: o.value as i16,
            });
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata: o.metadata(),
            });
            packets.push(ClientboundPlay::EntityVelocity {
                entity_id,
                x: speed(v.motion.0),
                y: speed(v.motion.1),
                z: speed(v.motion.2),
            });
        } else if let Some(it) = self.items.get(&entity_id) {
            let velocity = (speed(v.motion.0), speed(v.motion.1), speed(v.motion.2));
            packets.push(ClientboundPlay::SpawnObject {
                entity_id,
                kind: 2,
                x: fixed(v.pos.0),
                y: fixed(v.pos.1),
                z: fixed(v.pos.2),
                pitch: angle(v.pitch) as i8,
                yaw: angle(v.yaw) as i8,
                data: 1,
                velocity: Some(velocity),
            });
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata: it.metadata(),
            });
            packets.push(ClientboundPlay::EntityVelocity {
                entity_id,
                x: velocity.0,
                y: velocity.1,
                z: velocity.2,
            });
        }
        packets
    }

    /// Spawns or despawns the entity for one player (`updatePlayerEntity`).
    fn update_player_entity(&mut self, entity_id: i32, viewer: usize) {
        let Some(v) = self.view(entity_id) else {
            return;
        };
        // Vanilla quirk: a dead player's body leaves the entity list but stays among the
        // world's players until it respawns, so it keeps being shown entities whose
        // tracker entries update their viewers (new ones, those that moved far).
        if v.player == Some(viewer) {
            return;
        }
        let Some(entry) = self.tracker.get(&entity_id) else {
            return;
        };
        let viewer_pos = self.players[viewer].pos;
        // Vanilla quirk: the encoded position is divided as an integer (truncating).
        let dx = viewer_pos.0 - f64::from(entry.encoded_pos.0 / 32);
        let dz = viewer_pos.2 - f64::from(entry.encoded_pos.2 / 32);
        let range = f64::from(entry.range);
        let in_range = dx >= -range && dx <= range && dz >= -range && dz <= range;
        // TODO(M5): spectators are only visible to spectators (`isSpectatedByPlayer`).
        let viewer_id = self.players[viewer].entity_id;
        if in_range {
            let vp = &self.players[viewer];
            let sent = vp.watching.contains(&v.chunk) && !vp.chunk_queue.contains(&v.chunk);
            // forceSpawn: a vehicle restored with a joining player shows at once.
            let forced = self.force_spawn == Some(entity_id);
            if entry.tracking.contains(&viewer_id) || !(sent || forced) {
                return;
            }
            for packet in self.spawn_packets(entity_id, &v) {
                self.players[viewer].send(packet);
            }
            if let Some(entry) = self.tracker.get_mut(&entity_id) {
                entry.tracking.insert(viewer_id);
                entry.last_motion = v.motion;
            }
        } else if entry.tracking.contains(&viewer_id) {
            if let Some(entry) = self.tracker.get_mut(&entity_id) {
                entry.tracking.remove(&viewer_id);
            }
            if v.player.is_some() {
                // Players are destroyed right away (`EntityPlayerMP.removeEntity`).
                self.players[viewer].send(ClientboundPlay::DestroyEntities(vec![entity_id]));
            } else {
                self.players[viewer].pending_destroy.push(entity_id);
            }
        }
    }

    /// The tracker pass at the end of the entity updates
    /// (`EntityTracker.updateTrackedEntities`).
    pub(super) fn update_tracked_entities(&mut self) {
        let ids: Vec<i32> = self.tracker.keys().copied().collect();
        let mut updated_players = Vec::new();
        for id in ids {
            if self.update_entry(id) && self.player_by_entity(id).is_some() {
                updated_players.push(id);
            }
        }
        for player_id in updated_players {
            let Some(viewer) = self.player_by_entity(player_id) else {
                continue;
            };
            let ids: Vec<i32> = self.tracker.keys().copied().collect();
            for id in ids {
                if id != player_id {
                    self.update_player_entity(id, viewer);
                }
            }
        }
    }

    /// One entry's update (`EntityTrackerEntry.updatePlayerList`); returns whether it
    /// re-checked its viewers.
    fn update_entry(&mut self, entity_id: i32) -> bool {
        let Some(v) = self.view(entity_id) else {
            return false;
        };
        // Vanilla adds players in the network phase, after the tracker pass of that tick.
        if let Some(i) = v.player
            && self.players[i].joined_tick == self.tick_count
        {
            return false;
        }
        let pos = v.pos;
        let mut rechecked = false;
        if let Some(entry) = self.tracker.get_mut(&entity_id) {
            let (lx, ly, lz) = entry.last_tracked;
            let moved = (pos.0 - lx).powi(2) + (pos.1 - ly).powi(2) + (pos.2 - lz).powi(2);
            if !entry.first_update_done || moved > 16.0 {
                entry.last_tracked = pos;
                entry.first_update_done = true;
                rechecked = true;
            }
        }
        if rechecked {
            self.update_player_entities(entity_id);
        }

        // The vehicle goes to the viewers when it changes, and every 60 updates while
        // riding.
        let vehicle = v.player.and_then(|i| self.players[i].riding);
        if let Some(entry) = self.tracker.get_mut(&entity_id)
            && (entry.last_vehicle != vehicle
                || (vehicle.is_some() && entry.update_counter % 60 == 0))
        {
            entry.last_vehicle = vehicle;
            let packet = ClientboundPlay::AttachEntity {
                entity_id,
                vehicle_id: vehicle.unwrap_or(-1),
                leash: false,
            };
            self.send_to_tracking(entity_id, &packet);
        }

        // Item frames send the map they show and changed metadata every 10 ticks.
        let mut v = v;
        let frame = self.hanging.get(&entity_id).and_then(|h| match h.kind {
            super::hanging::Kind::ItemFrame { .. } => Some(super::maps::FramedMap {
                entity_id,
                tile: h.tile,
                facing: h.facing,
            }),
            super::hanging::Kind::Painting { .. } => None,
        });
        if let Some(frame) = frame
            && self
                .tracker
                .get(&entity_id)
                .is_some_and(|e| e.update_counter % ITEM_FRAME_METADATA_INTERVAL == 0)
        {
            self.update_framed_map(&frame);
            self.send_dirty_data(entity_id, &v);
            v.metadata_changed = false;
        }

        // Arrows move from their first update on and always send look and move together.
        let arrow = self.arrows.contains_key(&entity_id);
        let Some(entry) = self.tracker.get_mut(&entity_id) else {
            return rechecked;
        };
        let counter = entry.update_counter;
        let due = counter % entry.frequency == 0 || v.airborne || v.metadata_changed;
        if due && vehicle.is_some() {
            // A rider only sends its look; its position follows the vehicle.
            let (yaw, pitch) = (angle(v.yaw), angle(v.pitch));
            let turned =
                (yaw - entry.encoded_rot.0).abs() >= 4 || (pitch - entry.encoded_rot.1).abs() >= 4;
            let look = turned.then(|| {
                entry.encoded_rot = (yaw, pitch);
                ClientboundPlay::EntityLook {
                    entity_id,
                    yaw: yaw as i8,
                    pitch: pitch as i8,
                    on_ground: v.on_ground,
                }
            });
            entry.encoded_pos = (fixed(v.pos.0), fixed(v.pos.1), fixed(v.pos.2));
            entry.was_riding = true;
            let head = angle(v.head);
            let head_packet = ((head - entry.last_head).abs() >= 4).then(|| {
                entry.last_head = head;
                ClientboundPlay::EntityHeadLook {
                    entity_id,
                    yaw: head as i8,
                }
            });
            if let Some(packet) = look {
                self.send_to_tracking(entity_id, &packet);
            }
            self.send_dirty_data(entity_id, &v);
            if let Some(packet) = head_packet {
                self.send_to_tracking(entity_id, &packet);
            }
            if let Some(i) = v.player {
                self.players[i].airborne = false;
            }
        } else if due {
            entry.ticks_since_teleport += 1;
            let (x, y, z) = (fixed(v.pos.0), fixed(v.pos.1), fixed(v.pos.2));
            let (yaw, pitch) = (angle(v.yaw), angle(v.pitch));
            let (dx, dy, dz) = (
                x - entry.encoded_pos.0,
                y - entry.encoded_pos.1,
                z - entry.encoded_pos.2,
            );
            let moved = dx.abs() >= 4 || dy.abs() >= 4 || dz.abs() >= 4 || counter % 60 == 0;
            let turned =
                (yaw - entry.encoded_rot.0).abs() >= 4 || (pitch - entry.encoded_rot.1).abs() >= 4;
            let mut packet = None;
            if counter > 0 || arrow {
                let small = |d: i32| (-128..128).contains(&d);
                if small(dx)
                    && small(dy)
                    && small(dz)
                    && entry.ticks_since_teleport <= 400
                    && !entry.was_riding
                    && entry.on_ground == v.on_ground
                {
                    let on_ground = v.on_ground;
                    // Vanilla quirk: an arrow sends the look and move packet even when
                    // it neither moved nor turned, with a delta from the last position
                    // that was counted as moved.
                    packet = match (moved || arrow, turned || arrow) {
                        (true, true) => Some(ClientboundPlay::EntityLookMove {
                            entity_id,
                            dx: dx as i8,
                            dy: dy as i8,
                            dz: dz as i8,
                            yaw: yaw as i8,
                            pitch: pitch as i8,
                            on_ground,
                        }),
                        (true, false) => Some(ClientboundPlay::EntityRelMove {
                            entity_id,
                            dx: dx as i8,
                            dy: dy as i8,
                            dz: dz as i8,
                            on_ground,
                        }),
                        (false, true) => Some(ClientboundPlay::EntityLook {
                            entity_id,
                            yaw: yaw as i8,
                            pitch: pitch as i8,
                            on_ground,
                        }),
                        (false, false) => None,
                    };
                } else {
                    entry.on_ground = v.on_ground;
                    entry.ticks_since_teleport = 0;
                    packet = Some(ClientboundPlay::EntityTeleport {
                        entity_id,
                        x,
                        y,
                        z,
                        yaw: yaw as i8,
                        pitch: pitch as i8,
                        on_ground: v.on_ground,
                    });
                }
            }
            let mut velocity = None;
            if entry.velocity_updates {
                let d = (
                    v.motion.0 - entry.last_motion.0,
                    v.motion.1 - entry.last_motion.1,
                    v.motion.2 - entry.last_motion.2,
                );
                let change = d.0 * d.0 + d.1 * d.1 + d.2 * d.2;
                let stopped = v.motion == (0.0, 0.0, 0.0);
                if change > 0.02 * 0.02 || (change > 0.0 && stopped) {
                    entry.last_motion = v.motion;
                    let s = |m: f64| (m.clamp(-3.9, 3.9) * 8000.0) as i16;
                    velocity = Some(ClientboundPlay::EntityVelocity {
                        entity_id,
                        x: s(v.motion.0),
                        y: s(v.motion.1),
                        z: s(v.motion.2),
                    });
                }
            }
            if moved {
                entry.encoded_pos = (x, y, z);
            }
            entry.was_riding = false;
            if turned {
                entry.encoded_rot = (yaw, pitch);
            }
            let head = angle(v.head);
            let head_packet = ((head - entry.last_head).abs() >= 4).then(|| {
                entry.last_head = head;
                ClientboundPlay::EntityHeadLook {
                    entity_id,
                    yaw: head as i8,
                }
            });
            if let Some(packet) = packet {
                self.send_to_tracking(entity_id, &packet);
            }
            if let Some(packet) = velocity {
                self.send_to_tracking(entity_id, &packet);
            }
            self.send_dirty_data(entity_id, &v);
            if let Some(packet) = head_packet {
                self.send_to_tracking(entity_id, &packet);
            }
            if let Some(i) = v.player {
                self.players[i].airborne = false;
            } else if let Some(h) = self.hanging.get_mut(&entity_id) {
                h.airborne = false;
            } else if let Some(m) = self.minecarts.get_mut(&entity_id) {
                m.airborne = false;
            } else if let Some(b) = self.boats.get_mut(&entity_id) {
                b.airborne = false;
            } else if let Some(m) = self.mobs.get_mut(&entity_id) {
                m.airborne = false;
            }
        }
        if let Some(entry) = self.tracker.get_mut(&entity_id) {
            entry.update_counter += 1;
        }
        if let Some(i) = v.player {
            let p = &mut self.players[i];
            if std::mem::take(&mut p.velocity_changed) {
                let m = p.motion;
                let s = |m: f64| (m.clamp(-3.9, 3.9) * 8000.0) as i16;
                let packet = ClientboundPlay::EntityVelocity {
                    entity_id,
                    x: s(m.0),
                    y: s(m.1),
                    z: s(m.2),
                };
                self.send_to_tracking_and_self(entity_id, &packet);
            }
        } else if let Some(m) = self.take_velocity_changed(entity_id) {
            let s = |m: f64| (m.clamp(-3.9, 3.9) * 8000.0) as i16;
            let packet = ClientboundPlay::EntityVelocity {
                entity_id,
                x: s(m.0),
                y: s(m.1),
                z: s(m.2),
            };
            self.send_to_tracking(entity_id, &packet);
        }
        rechecked
    }

    /// Clears a non-player entity's `velocityChanged` and returns its motion if it was
    /// set.
    fn take_velocity_changed(&mut self, entity_id: i32) -> Option<(f64, f64, f64)> {
        let body = if let Some(it) = self.items.get_mut(&entity_id) {
            &mut it.body
        } else if let Some(o) = self.orbs.get_mut(&entity_id) {
            &mut o.body
        } else if let Some(f) = self.falling.get_mut(&entity_id) {
            &mut f.body
        } else if let Some(t) = self.thrown.get_mut(&entity_id) {
            &mut t.body
        } else if let Some(a) = self.arrows.get_mut(&entity_id) {
            &mut a.body
        } else if let Some(s) = self.armor_stands.get_mut(&entity_id) {
            &mut s.body
        } else if let Some(m) = self.minecarts.get_mut(&entity_id) {
            &mut m.body
        } else if let Some(b) = self.boats.get_mut(&entity_id) {
            &mut b.body
        } else if let Some(m) = self.mobs.get_mut(&entity_id) {
            &mut m.body
        } else {
            &mut self.tnt.get_mut(&entity_id)?.body
        };
        std::mem::take(&mut body.velocity_changed).then_some(body.motion)
    }

    /// Changed metadata and attributes go to the viewers and the player itself
    /// (`sendMetadataToAllAssociatedPlayers`).
    fn send_dirty_data(&mut self, entity_id: i32, v: &View) {
        let mut packets = Vec::new();
        if let Some(i) = v.player {
            let p = &mut self.players[i];
            if !p.dirty_metadata.is_empty() {
                let dirty = std::mem::take(&mut p.dirty_metadata);
                let metadata = player_metadata(p)
                    .into_iter()
                    .filter(|m| dirty.contains(&m.index))
                    .collect();
                packets.push(ClientboundPlay::EntityMetadata {
                    entity_id,
                    metadata,
                });
            }
            let dirty: Vec<&'static str> = std::mem::take(&mut p.dirty_attributes)
                .into_iter()
                .filter(|a| super::attributes::watched(a))
                .collect();
            if !dirty.is_empty() {
                packets.push(ClientboundPlay::EntityProperties {
                    entity_id,
                    properties: super::attributes::properties(&dirty, &p.modifiers),
                });
            }
        } else if let Some(a) = self.arrows.get_mut(&entity_id)
            && std::mem::take(&mut a.metadata_changed)
        {
            // DataWatcher.getChanged: every entry still watched.
            let dirty = std::mem::take(&mut a.dirty_metadata);
            let metadata = a
                .metadata()
                .into_iter()
                .filter(|m| dirty.contains(&m.index))
                .collect();
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata,
            });
        } else if let Some(h) = self.hanging.get_mut(&entity_id)
            && std::mem::take(&mut h.metadata_changed)
        {
            // DataWatcher.getChanged: every entry still watched.
            let dirty = std::mem::take(&mut h.dirty_metadata);
            let metadata = h
                .metadata()
                .into_iter()
                .filter(|m| dirty.contains(&m.index))
                .collect();
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata,
            });
        } else if let Some(b) = self.boats.get_mut(&entity_id)
            && std::mem::take(&mut b.metadata_changed)
        {
            // DataWatcher.getChanged: every entry still watched.
            let dirty = std::mem::take(&mut b.dirty_metadata);
            let metadata = b
                .metadata()
                .into_iter()
                .filter(|e| dirty.contains(&e.index))
                .collect();
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata,
            });
        } else if let Some(m) = self.minecarts.get_mut(&entity_id)
            && std::mem::take(&mut m.metadata_changed)
        {
            // DataWatcher.getChanged: every entry still watched.
            let dirty = std::mem::take(&mut m.dirty_metadata);
            let metadata = m
                .metadata()
                .into_iter()
                .filter(|e| dirty.contains(&e.index))
                .collect();
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata,
            });
        } else if let Some(m) = self.mobs.get_mut(&entity_id) {
            if std::mem::take(&mut m.metadata_changed) {
                // DataWatcher.getChanged: every entry still watched.
                let dirty = std::mem::take(&mut m.dirty_metadata);
                let metadata = m
                    .metadata()
                    .into_iter()
                    .filter(|e| dirty.contains(&e.index))
                    .collect();
                packets.push(ClientboundPlay::EntityMetadata {
                    entity_id,
                    metadata,
                });
            }
            // ServersideAttributeMap.getAttributeInstanceSet: the changed watched
            // attributes.
            if std::mem::take(&mut m.dirty_attributes) {
                packets.push(ClientboundPlay::EntityProperties {
                    entity_id,
                    properties: m.properties(),
                });
            }
        } else if let Some(s) = self.armor_stands.get_mut(&entity_id)
            && std::mem::take(&mut s.metadata_changed)
        {
            // DataWatcher.getChanged: every entry still watched.
            let dirty = std::mem::take(&mut s.dirty_metadata);
            let metadata = s
                .metadata()
                .into_iter()
                .filter(|m| dirty.contains(&m.index))
                .collect();
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata,
            });
        } else if let Some(it) = self.items.get_mut(&entity_id)
            && !it.dirty_metadata.is_empty()
        {
            let dirty = std::mem::take(&mut it.dirty_metadata);
            let metadata = it
                .metadata()
                .into_iter()
                .filter(|m| dirty.contains(&m.index))
                .collect();
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata,
            });
        }
        // DataWatcher.getChanged of a watcher whose entries were never sent as changed:
        // all of them.
        let base_only = if let Some(f) = self.falling.get_mut(&entity_id) {
            std::mem::take(&mut f.base.metadata_changed).then(|| f.metadata())
        } else if let Some(t) = self.tnt.get_mut(&entity_id) {
            std::mem::take(&mut t.base.metadata_changed).then(|| t.metadata())
        } else if let Some(t) = self.thrown.get_mut(&entity_id) {
            std::mem::take(&mut t.base.metadata_changed).then(|| t.metadata())
        } else if let Some(o) = self.orbs.get_mut(&entity_id) {
            std::mem::take(&mut o.base.metadata_changed).then(|| o.metadata())
        } else {
            None
        };
        if let Some(metadata) = base_only {
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata,
            });
        }
        for packet in packets {
            self.send_to_tracking_and_self(entity_id, &packet);
        }
    }
}

/// `S12PacketEntityVelocity`: motion clamped to 3.9 per axis, in 1/8000 blocks per tick.
pub fn velocity_packet(entity_id: i32, motion: (f64, f64, f64)) -> ClientboundPlay {
    let s = |m: f64| (m.clamp(-3.9, 3.9) * 8000.0) as i32 as i16;
    ClientboundPlay::EntityVelocity {
        entity_id,
        x: s(motion.0),
        y: s(motion.1),
        z: s(motion.2),
    }
}
