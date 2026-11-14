//! Which clients see which entities, and the packets that keep them in sync
//! (vanilla: `EntityTracker`, `EntityTrackerEntry`).
//!
//! Only players are tracked so far. Every tracked entity has an entry holding what its
//! viewers were last told (position in 1/32 blocks, angles in 1/256 turns). Each tick,
//! after the entity updates, every entry:
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
    /// Entity ids of the players that see this entity (vanilla `trackingPlayers`).
    tracking: BTreeSet<i32>,
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
            Byte((i8::from(p.sneaking) << 1) | (i8::from(p.sprinting) << 3)),
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
        md(7, Int(0)),
        md(8, Byte(0)),
        md(9, Byte(0)),
        md(10, Byte(p.skin_parts as i8)),
        md(16, Byte(0)),
        md(17, Float(d.get_f32("AbsorptionAmount"))),
        md(18, Int(d.get_i32("Score"))),
    ]
}

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

impl Game {
    fn player_by_entity(&self, entity_id: i32) -> Option<usize> {
        self.players.iter().position(|p| p.entity_id == entity_id)
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

    /// Starts tracking the player at index `i` (vanilla `EntityTracker.trackEntity`, run
    /// when the player is added to the world).
    pub(super) fn track_player(&mut self, i: usize) {
        let p = &self.players[i];
        let entity_id = p.entity_id;
        let entry = TrackerEntry {
            range: self.tracking_range(PLAYER_RANGE),
            frequency: PLAYER_FREQUENCY,
            encoded_pos: (fixed(p.pos.0), fixed(p.pos.1), fixed(p.pos.2)),
            encoded_rot: (angle(p.rotation.0), angle(p.rotation.1)),
            last_head: angle(p.rotation.0),
            last_tracked: (0.0, 0.0, 0.0),
            first_update_done: false,
            update_counter: 0,
            ticks_since_teleport: 0,
            on_ground: p.on_ground,
            tracking: BTreeSet::new(),
        };
        self.tracker.insert(entity_id, entry);
        self.update_player_entities(entity_id);
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

    /// Spawns the entities in freshly sent chunks for the player at index `viewer`
    /// (vanilla `EntityTracker.func_85172_a`).
    pub(super) fn chunks_sent(&mut self, viewer: usize, chunks: &[(i32, i32)]) {
        for &c in chunks {
            let in_chunk: Vec<i32> = self
                .players
                .iter()
                .filter(|p| p.entity_chunk == c && p.entity_id != self.players[viewer].entity_id)
                .map(|p| p.entity_id)
                .filter(|e| self.tracker.contains_key(e))
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

    /// Spawns or despawns the entity for one player (`updatePlayerEntity`).
    fn update_player_entity(&mut self, entity_id: i32, viewer: usize) {
        let Some(target) = self.player_by_entity(entity_id) else {
            return;
        };
        if target == viewer {
            return;
        }
        let Some(entry) = self.tracker.get(&entity_id) else {
            return;
        };
        let v = &self.players[viewer];
        // Vanilla quirk: the encoded position is divided as an integer (truncating).
        let dx = v.pos.0 - f64::from(entry.encoded_pos.0 / 32);
        let dz = v.pos.2 - f64::from(entry.encoded_pos.2 / 32);
        let range = f64::from(entry.range);
        let in_range = dx >= -range && dx <= range && dz >= -range && dz <= range;
        // TODO(M5): spectators are only visible to spectators (`isSpectatedByPlayer`).
        let viewer_id = v.entity_id;
        if in_range {
            let t = &self.players[target];
            let chunk = t.entity_chunk;
            let sent = v.watching.contains(&chunk) && !v.chunk_queue.contains(&chunk);
            if entry.tracking.contains(&viewer_id) || !sent {
                return;
            }
            let mut packets = vec![ClientboundPlay::SpawnPlayer {
                entity_id,
                uuid: t.profile.id,
                x: fixed(t.pos.0),
                y: fixed(t.pos.1),
                z: fixed(t.pos.2),
                yaw: (t.rotation.0 * 256.0 / 360.0) as i32 as i8,
                pitch: (t.rotation.1 * 256.0 / 360.0) as i32 as i8,
                current_item: t.held().map_or(0, |s| s.id),
                metadata: player_metadata(t),
            }];
            packets.push(ClientboundPlay::EntityMetadata {
                entity_id,
                metadata: player_metadata(t),
            });
            packets.push(ClientboundPlay::EntityProperties {
                entity_id,
                properties: vec![
                    ("generic.movementSpeed".into(), f64::from(0.1f32)),
                    ("generic.maxHealth".into(), 20.0),
                ],
            });
            for (slot, item) in equipment(t).into_iter().enumerate() {
                if item.is_some() {
                    packets.push(ClientboundPlay::EntityEquipment {
                        entity_id,
                        slot: slot as i16,
                        item,
                    });
                }
            }
            // TODO(M5): use bed, potion effects.
            for packet in packets {
                self.players[viewer].send(packet);
            }
            if let Some(entry) = self.tracker.get_mut(&entity_id) {
                entry.tracking.insert(viewer_id);
            }
        } else if entry.tracking.contains(&viewer_id) {
            if let Some(entry) = self.tracker.get_mut(&entity_id) {
                entry.tracking.remove(&viewer_id);
            }
            // Players are destroyed right away (`EntityPlayerMP.removeEntity`).
            self.players[viewer].send(ClientboundPlay::DestroyEntities(vec![entity_id]));
        }
    }

    /// The tracker pass at the end of the entity updates
    /// (`EntityTracker.updateTrackedEntities`).
    pub(super) fn update_tracked_entities(&mut self) {
        let ids: Vec<i32> = self.tracker.keys().copied().collect();
        let mut updated_players = Vec::new();
        for id in ids {
            if self.update_entry(id) {
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
        let Some(i) = self.player_by_entity(entity_id) else {
            return false;
        };
        // Vanilla adds players in the network phase, after the tracker pass of that tick.
        if self.players[i].joined_tick == self.tick_count {
            return false;
        }
        let pos = self.players[i].pos;
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

        let p = &self.players[i];
        let metadata_changed = !p.dirty_metadata.is_empty();
        let airborne = p.airborne;
        let Some(entry) = self.tracker.get_mut(&entity_id) else {
            return rechecked;
        };
        let counter = entry.update_counter;
        if counter % entry.frequency == 0 || airborne || metadata_changed {
            entry.ticks_since_teleport += 1;
            let (x, y, z) = (fixed(p.pos.0), fixed(p.pos.1), fixed(p.pos.2));
            let (yaw, pitch) = (angle(p.rotation.0), angle(p.rotation.1));
            let (dx, dy, dz) = (
                x - entry.encoded_pos.0,
                y - entry.encoded_pos.1,
                z - entry.encoded_pos.2,
            );
            let moved = dx.abs() >= 4 || dy.abs() >= 4 || dz.abs() >= 4 || counter % 60 == 0;
            let turned =
                (yaw - entry.encoded_rot.0).abs() >= 4 || (pitch - entry.encoded_rot.1).abs() >= 4;
            let mut packet = None;
            if counter > 0 {
                let small = |d: i32| (-128..128).contains(&d);
                if small(dx)
                    && small(dy)
                    && small(dz)
                    && entry.ticks_since_teleport <= 400
                    && entry.on_ground == p.on_ground
                {
                    let on_ground = p.on_ground;
                    packet = match (moved, turned) {
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
                    entry.on_ground = p.on_ground;
                    entry.ticks_since_teleport = 0;
                    packet = Some(ClientboundPlay::EntityTeleport {
                        entity_id,
                        x,
                        y,
                        z,
                        yaw: yaw as i8,
                        pitch: pitch as i8,
                        on_ground: p.on_ground,
                    });
                }
            }
            if moved {
                entry.encoded_pos = (x, y, z);
            }
            if turned {
                entry.encoded_rot = (yaw, pitch);
            }
            let head = angle(p.rotation.0);
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
            self.send_dirty_data(i);
            if let Some(packet) = head_packet {
                self.send_to_tracking(entity_id, &packet);
            }
            self.players[i].airborne = false;
        }
        if let Some(entry) = self.tracker.get_mut(&entity_id) {
            entry.update_counter += 1;
        }
        rechecked
    }

    /// Changed metadata and attributes go to the viewers and the player itself
    /// (`sendMetadataToAllAssociatedPlayers`).
    fn send_dirty_data(&mut self, i: usize) {
        let p = &mut self.players[i];
        let entity_id = p.entity_id;
        let mut packets = Vec::new();
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
        if std::mem::take(&mut p.dirty_attributes) {
            packets.push(ClientboundPlay::EntityProperties {
                entity_id,
                properties: vec![("generic.movementSpeed".into(), f64::from(0.1f32))],
            });
        }
        for packet in packets {
            self.send_to_tracking_and_self(entity_id, &packet);
        }
    }
}
