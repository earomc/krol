//! Boats (vanilla: `EntityBoat`, placed by `ItemBoat`).
//!
//! A boat floats: the share of its height under water lifts it, the rest sinks it. A
//! player in it steers it with the movement keys, speeding up to 0.35 a tick while it
//! keeps gaining speed. It turns towards where it moves, at most 20 degrees a tick,
//! breaks lily pads and snow it runs into, and falls apart into planks and sticks when
//! it crashes faster than 0.2975 a tick or lands from more than 3 blocks. Hits wobble it
//! and add damage that wears off; past 40 (or at once for a creative player) it breaks,
//! dropping a boat. Boats are solid: other entities collide with them, and they with
//! every entity around.
//!
//! TODO(M5): items, orbs, arrows, falling blocks and primed TNT colliding with boats;
//! the rider taking fall damage when the boat lands hard; mobs riding boats.

use std::collections::BTreeSet;

use krol_java::math::{floor_f64, wrap_degrees_f64};
use krol_nbt::Compound;
use krol_protocol::packets::play::{ItemStack, Metadata, MetadataValue};

use super::Game;
use super::damage::DamageSource;
use super::ids::{block, item};
use super::physics::{self, Aabb, Body};
use super::saved_entities::EntityBase;

/// Saved id (`EntityList` name).
pub const SAVED_ID: &str = "Boat";
/// The en_US `entity.Boat.name`.
pub const NAME: &str = "Boat";
/// `S0EPacketSpawnObject` type of boats.
pub const OBJECT_TYPE: i8 = 1;
/// `setSize(1.5F, 0.6F)`.
const WIDTH: f32 = 1.5;
const HEIGHT: f32 = 0.6;
/// `getMountedYOffset`.
pub const MOUNTED_Y_OFFSET: f64 = -0.3;
/// The rider sits this far ahead of the boat's middle (`updateRiderPosition`).
pub const RIDER_AHEAD: f64 = 0.4;
/// `speedMultiplier` starts at and falls back to this, and grows towards the top speed.
const BASE_SPEED_MULTIPLIER: f64 = 0.07;
const MAX_SPEED: f64 = 0.35;
/// The speed multiplier moves this share of the way each tick.
const SPEED_STEPS: f64 = 35.0;
/// Crashing faster than this (and the splash particles) start here.
const CRASH_SPEED: f64 = 0.2975;
/// Slices of the boat's height tested for water.
const SLICES: i32 = 5;
/// Each slice is tested this much lower.
const SLICE_DROP: f64 = 0.125;
/// Out of water the boat sinks or rises by `0.04F` scaled by the share under water...
const BUOYANCY: f32 = 0.04;
/// ...and fully under water it rises by `0.007F` a tick (after halving a fall).
const FLOAT_UP: f32 = 0.007;
/// The rider's push (`0.05F`).
const STEER: f32 = 0.05;
/// Drag: `0.99F` sideways, `0.95F` up and down.
const DRAG: f32 = 0.99;
const VERTICAL_DRAG: f32 = 0.95;
/// The boat turns at most this much a tick.
const MAX_TURN: f64 = 20.0;
/// Landing from higher breaks the boat.
const BREAK_FALL: f32 = 3.0;
/// Hits add 10 damage per point; past this the boat breaks.
const DAMAGE_PER_POINT: f32 = 10.0;
const BREAK_DAMAGE: f32 = 40.0;
/// A hit wobbles the boat this long.
const WOBBLE: i32 = 10;
/// Planks and sticks a wrecked boat drops.
const WRECK_PLANKS: usize = 3;
const WRECK_STICKS: usize = 2;
/// The pickup delay of dropped items (`setDefaultPickupDelay`).
const DROP_PICKUP_DELAY: i32 = 10;
/// `Entity.fireResistance`.
const FIRE_RESISTANCE: i16 = 1;
/// Fire blocks set entities that keep standing in them on fire for this long.
const FIRE_BLOCK_SECONDS: i16 = 8;
/// Lava sets entities on fire for this long (`setOnFireFromLava`) and hurts this much.
const LAVA_SECONDS: i16 = 15;
const LAVA_DAMAGE: f32 = 4.0;
/// Entities below this height are killed.
const VOID_Y: f64 = -64.0;
/// The fizz of a burning boat getting wet.
const FIZZ_VOLUME: f32 = 0.7;
const FIZZ_PITCH: f32 = 1.6;
const FIZZ_SPREAD: f32 = 0.4;
/// `Entity.applyEntityCollision` pushes by `0.05` per unit of overlap...
const PUSH: f64 = 0.050_000_000_745_058_06;
/// ...once the entities are at least this far apart on an axis.
const PUSH_MIN_DISTANCE: f64 = 0.009_999_999_776_482_582;
/// The creative game mode.
const CREATIVE: i32 = 1;
/// The spectator game mode.
const SPECTATOR: i32 = 3;

/// Metadata entries: time since the last hit, the wobble direction, the damage.
const TIME_SINCE_HIT: u8 = 17;
const FORWARD_DIRECTION: u8 = 18;
const DAMAGE: u8 = 19;
/// The base flags entry (bit 0: burning).
const FLAGS: u8 = 0;

/// One boat.
#[derive(Debug, Clone)]
pub struct Boat {
    /// Position, motion and the movement state.
    pub body: Body,
    /// The base entity state.
    pub base: EntityBase,
    /// Metadata 17-19.
    time_since_hit: i32,
    forward_direction: i32,
    damage: f32,
    /// `speedMultiplier` (not saved).
    speed_multiplier: f64,
    /// Metadata 0, bit 0: burning (`setFlag(0, fire > 0)`).
    burning: bool,
    /// The player riding it, by entity id.
    pub rider: Option<i32>,
    /// `isAirBorne`: pushed by `addVelocity`; the tracker sends the move at once.
    pub airborne: bool,
    /// Removed from the world (`isDead`).
    pub dead: bool,
    /// `firstUpdate`: no splash before the first update ends.
    first_update: bool,
    /// The chunk it is in (as of its last update).
    pub chunk: (i32, i32),
    /// Metadata entries changed since the tracker last sent them.
    pub dirty_metadata: BTreeSet<u8>,
    /// Some metadata changed since the tracker last looked.
    pub metadata_changed: bool,
}

fn chunk_of(pos: (f64, f64, f64)) -> (i32, i32) {
    (floor_f64(pos.0) >> 4, floor_f64(pos.2) >> 4)
}

impl Boat {
    /// A new boat at `pos`, facing `yaw`.
    pub fn new(pos: (f64, f64, f64), yaw: f32) -> Self {
        let mut body = Body::player(pos, false);
        body.width = WIDTH;
        body.height = HEIGHT;
        body.step_height = 0.0;
        let mut b = Self {
            body,
            base: EntityBase::facing(yaw),
            time_since_hit: 0,
            forward_direction: 1,
            damage: 0.0,
            speed_multiplier: BASE_SPEED_MULTIPLIER,
            burning: false,
            rider: None,
            airborne: false,
            dead: false,
            first_update: true,
            chunk: chunk_of(pos),
            dirty_metadata: BTreeSet::new(),
            metadata_changed: false,
        };
        // A new data watcher's entries are all watched: its first change sends them all.
        b.dirty_metadata = b.metadata().iter().map(|e| e.index).collect();
        b
    }

    /// `readEntityFromNBT` of a boat (`body` and `base` already read; boats save
    /// nothing of their own).
    pub fn load(mut body: Body, base: EntityBase) -> Self {
        body.width = WIDTH;
        body.height = HEIGHT;
        body.step_height = 0.0;
        let mut b = Self::new(body.pos, base.yaw);
        b.body = body;
        b.base = base;
        // Reading the base state changes the data watcher where it differs from a new
        // boat's.
        if b.base.air != super::saved_entities::FULL_AIR
            || b.base.custom_name.is_some()
            || b.base.custom_name_visible
            || b.base.silent
        {
            b.metadata_changed = true;
        }
        b.chunk = chunk_of(b.body.pos);
        b
    }

    /// `getName`.
    pub fn name(&self) -> String {
        self.base
            .custom_name
            .clone()
            .unwrap_or_else(|| NAME.to_owned())
    }

    fn metadata_updated(&mut self, index: u8) {
        self.dirty_metadata.insert(index);
        self.metadata_changed = true;
    }

    fn set_time_since_hit(&mut self, v: i32) {
        if v != self.time_since_hit {
            self.time_since_hit = v;
            self.metadata_updated(TIME_SINCE_HIT);
        }
    }

    fn set_forward_direction(&mut self, v: i32) {
        if v != self.forward_direction {
            self.forward_direction = v;
            self.metadata_updated(FORWARD_DIRECTION);
        }
    }

    fn set_damage(&mut self, v: f32) {
        if v != self.damage {
            self.damage = v;
            self.metadata_updated(DAMAGE);
        }
    }

    /// The complete data watcher, in `HashMap` order.
    pub fn metadata(&self) -> Vec<Metadata> {
        use MetadataValue::{Byte, Float, Int};
        let md = |index, value| Metadata { index, value };
        let mut out = self.base.metadata();
        out[0] = md(FLAGS, Byte(i8::from(self.burning)));
        out.push(md(TIME_SINCE_HIT, Int(self.time_since_hit)));
        out.push(md(FORWARD_DIRECTION, Int(self.forward_direction)));
        out.push(md(DAMAGE, Float(self.damage)));
        let indices: Vec<i32> = out.iter().map(|e| i32::from(e.index)).collect();
        krol_java::java_hash_order(&indices)
            .into_iter()
            .map(|k| out[k].clone())
            .collect()
    }

    /// Where its rider sits (`updateRiderPosition`): ahead of the middle along the
    /// boat's yaw. `y_offset` is the rider's `getYOffset`.
    pub fn seat(&self, y_offset: f64) -> (f64, f64, f64) {
        // Math.cos and Math.sin of a double (HotSpot may differ from f64 by an ulp).
        let yaw = f64::from(self.base.yaw) * std::f64::consts::PI / 180.0;
        let p = self.body.pos;
        (
            p.0 + yaw.cos() * RIDER_AHEAD,
            p.1 + MOUNTED_Y_OFFSET + y_offset,
            p.2 + yaw.sin() * RIDER_AHEAD,
        )
    }
}

/// `World.isAABBInMaterial(bb, water)`: a water block in the box whose surface is at or
/// above the box's bottom.
fn in_water(world: &mut super::world::World, bb: &Aabb) -> bool {
    let (x0, x1) = (floor_f64(bb.min[0]), floor_f64(bb.max[0] + 1.0));
    let (y0, y1) = (floor_f64(bb.min[1]), floor_f64(bb.max[1] + 1.0));
    let (z0, z1) = (floor_f64(bb.min[2]), floor_f64(bb.max[2] + 1.0));
    for x in x0..x1 {
        for y in y0..y1 {
            for z in z0..z1 {
                let state = world.block(x, y, z);
                if !matches!(state >> 4, block::FLOWING_WATER | block::WATER) {
                    continue;
                }
                let level = state & 15;
                let mut top = f64::from(y + 1);
                if level < 8 {
                    top -= f64::from(level) / 8.0;
                }
                if top >= bb.min[1] {
                    return true;
                }
            }
        }
    }
    false
}

impl Game {
    /// `ItemBoat.onItemRightClick`: a boat on the block (or water) the player looks at
    /// within 5 blocks, facing away from the player, unless the player's eyes are inside
    /// an entity or the boat would not fit.
    pub(super) fn use_boat(&mut self, i: usize) -> super::buckets::UseResult {
        use super::buckets::UseResult;
        let Some(hit) = self.view_ray_trace(i, true) else {
            return UseResult::Unchanged;
        };
        let p = &self.players[i];
        let eye = (p.pos.0, p.pos.1 + f64::from(p.eye_height()), p.pos.2);
        let me = p.entity_id;
        if self.entity_boxes_near(None).iter().any(|&(id, bb)| {
            // getCollisionBorderSize.
            let border = f64::from(0.1f32);
            id != me && bb.expand([border, border, border]).contains(eye)
        }) {
            return UseResult::Unchanged;
        }
        let mut at = hit.pos;
        if self.level.overworld.block(at.0, at.1, at.2) >> 4 == block::SNOW_LAYER {
            at.1 -= 1;
        }
        let pos = (
            f64::from(at.0 as f32 + 0.5),
            f64::from(at.1 as f32 + 1.0),
            f64::from(at.2 as f32 + 0.5),
        );
        let yaw_quarter = floor_f64(f64::from(self.players[i].rotation.0 * 4.0 / 360.0) + 0.5);
        let yaw = (((yaw_quarter & 3) - 1) * 90) as f32;
        let boat = Boat::new(pos, yaw);
        let room = boat.body.bb().expand([-0.1, -0.1, -0.1]);
        let blocked = !physics::colliding_boxes(&mut self.level.overworld, &room).is_empty()
            || self
                .boats
                .values()
                .any(|b| !b.dead && b.body.bb().intersects(&room));
        if blocked {
            return UseResult::Unchanged;
        }
        let id = self.next_entity_id();
        self.boats.insert(id, boat);
        self.spawn_entity_in_world(id);
        let creative = self.players[i].gamemode == CREATIVE;
        self.count_item_use(i);
        if creative {
            return UseResult::Unchanged;
        }
        let held = 36 + self.players[i].selected;
        if let Some(s) = self.players[i].inventory[held].as_mut() {
            s.count -= 1;
        }
        UseResult::Resized
    }

    /// The boxes of the entities that can be hit or collided with
    /// (`canBeCollidedWith`), with their ids; `except` is left out.
    fn entity_boxes_near(&self, except: Option<i32>) -> Vec<(i32, Aabb)> {
        let mut out: Vec<(i32, Aabb)> = self
            .players
            .iter()
            .filter(|p| !p.removed && p.gamemode != SPECTATOR && !p.dead_marked)
            .map(|p| (p.entity_id, Aabb::at(p.pos, 0.6, 1.8)))
            .collect();
        out.extend(
            self.minecarts
                .iter()
                .filter(|(_, m)| !m.dead)
                .map(|(&id, m)| (id, m.body.bb())),
        );
        out.extend(
            self.boats
                .iter()
                .filter(|(_, b)| !b.dead)
                .map(|(&id, b)| (id, b.body.bb())),
        );
        out.extend(
            self.armor_stands
                .iter()
                .filter(|(_, s)| !s.dead)
                .map(|(&id, s)| (id, s.body.bb())),
        );
        out.extend(
            self.hanging
                .iter()
                .filter(|(_, h)| !h.dead)
                .map(|(&id, h)| (id, h.bb)),
        );
        out.extend(
            self.tnt
                .iter()
                .filter(|(_, t)| !t.dead)
                .map(|(&id, t)| (id, t.body.bb())),
        );
        out.extend(self.falling.iter().map(|(&id, f)| (id, f.body.bb())));
        out.retain(|&(id, _)| Some(id) != except);
        out
    }

    /// The boxes a moving boat collides with besides blocks (`getCollisionBox` returns
    /// every other entity's box): everything near it but its rider and spectators.
    fn boat_obstacles(&self, id: i32, area: &Aabb) -> Vec<Aabb> {
        let near = area.expand([0.25, 0.25, 0.25]);
        let rider = self.boats.get(&id).and_then(|b| b.rider);
        let mut boxes: Vec<Aabb> = self
            .players
            .iter()
            .filter(|p| !p.removed && p.gamemode != SPECTATOR && Some(p.entity_id) != rider)
            .map(|p| Aabb::at(p.pos, 0.6, 1.8))
            .collect();
        boxes.extend(self.items.values().map(|it| it.body.bb()));
        boxes.extend(self.orbs.values().map(|o| o.body.bb()));
        boxes.extend(self.falling.values().map(|f| f.body.bb()));
        boxes.extend(self.tnt.values().map(|t| t.body.bb()));
        boxes.extend(self.thrown.values().map(|t| t.body.bb()));
        boxes.extend(self.arrows.values().map(|a| a.body.bb()));
        boxes.extend(self.hanging.values().map(|h| h.bb));
        boxes.extend(self.armor_stands.values().map(|s| s.body.bb()));
        boxes.extend(self.minecarts.values().map(|m| m.body.bb()));
        boxes.extend(
            self.boats
                .iter()
                .filter(|&(&o, _)| o != id)
                .map(|(_, b)| b.body.bb()),
        );
        boxes.retain(|b| b.intersects(&near));
        boxes
    }

    /// The boxes of the boats near `area` (`getCollisionBoundingBox`), for other
    /// entities moving; `except` is the mover's own vehicle.
    pub(super) fn boat_boxes(&self, area: &Aabb, except: Option<i32>) -> Vec<Aabb> {
        let near = area.expand([0.25, 0.25, 0.25]);
        self.boats
            .iter()
            .filter(|&(&id, b)| !b.dead && Some(id) != except)
            .map(|(_, b)| b.body.bb())
            .filter(|b| b.intersects(&near))
            .collect()
    }

    /// One boat's turn in the entity loop.
    pub(super) fn update_boat(&mut self, id: i32) {
        if self.boats.get(&id).is_some_and(|b| !b.dead) {
            self.tick_boat(id);
            if let Some(b) = self.boats.get_mut(&id) {
                b.chunk = chunk_of(b.body.pos);
            }
            self.relocate_entity(id);
            self.update_vehicle_rider(id);
        }
        if self.boats.get(&id).is_some_and(|b| b.dead) {
            if let Some(b) = self.boats.remove(&id)
                && let Some(rider) = b.rider
                && let Some(p) = self.players.iter_mut().find(|p| p.entity_id == rider)
                && p.riding == Some(id)
            {
                p.riding = None;
            }
            self.remove_entity_from_chunk(id);
            self.untrack_entity(id);
        }
    }

    /// `Entity.onEntityUpdate` for a boat: water, fire, lava and the void.
    fn boat_base_update(&mut self, id: i32) {
        let Some(b) = self.boats.get_mut(&id) else {
            return;
        };
        if b.base.portal_cooldown > 0 {
            b.base.portal_cooldown -= 1;
        }
        // handleWaterMovement.
        let bb = b
            .body
            .bb()
            .expand([0.0, f64::from(-0.4f32), 0.0])
            .contract([0.001, 0.001, 0.001]);
        let (water, push) = super::liquids::water_movement(&mut self.level.overworld, &bb, true);
        let Some(b) = self.boats.get_mut(&id) else {
            return;
        };
        if push != (0.0, 0.0, 0.0) {
            b.body.motion.0 += push.0;
            b.body.motion.1 += push.1;
            b.body.motion.2 += push.2;
        }
        if water && !b.body.in_water && !b.first_update {
            // resetHeight: the splash (particles are client-side).
            let m = b.body.motion;
            let volume = (krol_java::math::sqrt_f64(
                m.0 * m.0 * f64::from(0.2f32) + m.1 * m.1 + m.2 * m.2 * f64::from(0.2f32),
            ) * 0.2)
                .min(1.0);
            let (pos, silent) = (b.body.pos, b.base.silent);
            // The boat's own random (unseeded in vanilla).
            let r = &mut self.entity_rand;
            let pitch = 1.0 + (r.next_float() - r.next_float()) * 0.4;
            if !silent {
                self.play_sound_near_except(None, "game.neutral.swim.splash", pos, volume, pitch);
            }
        }
        let Some(b) = self.boats.get_mut(&id) else {
            return;
        };
        b.body.in_water = water;
        if water {
            b.base.wet();
        }
        if b.base.fire > 0 {
            let burn = b.base.fire % 20 == 0;
            if burn {
                self.attack_boat(id, DamageSource::OnFire, 1.0);
            }
            if let Some(b) = self.boats.get_mut(&id) {
                b.base.fire -= 1;
            }
        }
        let Some(b) = self.boats.get(&id) else {
            return;
        };
        let lava_box = b.body.bb().expand([-0.1, -0.4, -0.1]);
        if self.any_block_in(&lava_box, block::FLOWING_LAVA..=block::LAVA) {
            // setOnFireFromLava.
            self.attack_boat(id, DamageSource::Lava, LAVA_DAMAGE);
            if let Some(b) = self.boats.get_mut(&id) {
                b.base.fire = b.base.fire.max(LAVA_SECONDS * 20);
                b.base.fall_distance *= 0.5;
            }
        }
        let Some(b) = self.boats.get_mut(&id) else {
            return;
        };
        if b.body.pos.1 < VOID_Y {
            // kill: setDead.
            b.dead = true;
        }
        let burning = b.base.fire > 0;
        if burning != b.burning {
            b.burning = burning;
            b.metadata_updated(FLAGS);
        }
        b.first_update = false;
    }

    /// `EntityBoat.onUpdate` on the server.
    fn tick_boat(&mut self, id: i32) {
        self.boat_base_update(id);
        let Some(b) = self.boats.get_mut(&id) else {
            return;
        };
        if b.time_since_hit > 0 {
            b.set_time_since_hit(b.time_since_hit - 1);
        }
        if b.damage > 0.0 {
            b.set_damage(b.damage - 1.0);
        }
        let prev = b.body.pos;
        // The share of the boat's height (tested a bit low) in water.
        let bb = b.body.bb();
        let mut wet = 0.0;
        for k in 0..SLICES {
            let h = bb.max[1] - bb.min[1];
            let y0 = bb.min[1] + h * f64::from(k) / f64::from(SLICES) - SLICE_DROP;
            let y1 = bb.min[1] + h * f64::from(k + 1) / f64::from(SLICES) - SLICE_DROP;
            let slice = Aabb::new([bb.min[0], y0, bb.min[2]], [bb.max[0], y1, bb.max[2]]);
            if in_water(&mut self.level.overworld, &slice) {
                wet += 1.0 / f64::from(SLICES);
            }
        }
        let Some(b) = self.boats.get_mut(&id) else {
            return;
        };
        let (mx, mz) = (b.body.motion.0, b.body.motion.2);
        // (Fast boats make splash particles from their own random, client-side only.)
        let speed_before = (mx * mx + mz * mz).sqrt();
        if wet < 1.0 {
            b.body.motion.1 += f64::from(BUOYANCY) * (wet * 2.0 - 1.0);
        } else {
            if b.body.motion.1 < 0.0 {
                b.body.motion.1 /= 2.0;
            }
            b.body.motion.1 += f64::from(FLOAT_UP);
        }
        let rider = b
            .rider
            .and_then(|r| self.players.iter().find(|p| p.entity_id == r))
            .map(|p| (p.rotation.0, p.move_strafing, p.move_forward));
        if let Some((yaw, strafing, forward)) = rider {
            let heading = yaw + -strafing * 90.0;
            let angle = f64::from(heading * std::f32::consts::PI / 180.0);
            let push = b.speed_multiplier * f64::from(forward) * f64::from(STEER);
            // Math.sin and Math.cos of a double (HotSpot may differ from f64 by an ulp).
            b.body.motion.0 += -angle.sin() * push;
            b.body.motion.2 += angle.cos() * push;
        }
        let mut speed =
            (b.body.motion.0 * b.body.motion.0 + b.body.motion.2 * b.body.motion.2).sqrt();
        if speed > MAX_SPEED {
            let k = MAX_SPEED / speed;
            b.body.motion.0 *= k;
            b.body.motion.2 *= k;
            speed = MAX_SPEED;
        }
        if speed > speed_before && b.speed_multiplier < MAX_SPEED {
            b.speed_multiplier += (MAX_SPEED - b.speed_multiplier) / SPEED_STEPS;
            b.speed_multiplier = b.speed_multiplier.min(MAX_SPEED);
        } else {
            b.speed_multiplier -= (b.speed_multiplier - BASE_SPEED_MULTIPLIER) / SPEED_STEPS;
            b.speed_multiplier = b.speed_multiplier.max(BASE_SPEED_MULTIPLIER);
        }
        // Snow and lily pads at the corners give way.
        let pos = b.body.pos;
        for k in 0..4 {
            let x = floor_f64(pos.0 + (f64::from(k % 2) - 0.5) * 0.8);
            let z = floor_f64(pos.2 + (f64::from(k / 2) - 0.5) * 0.8);
            for dy in 0..2 {
                let at = (x, floor_f64(pos.1) + dy, z);
                let state = self.level.overworld.block(at.0, at.1, at.2);
                match state >> 4 {
                    block::SNOW_LAYER => {
                        self.set_block_notify(at, 0);
                        if let Some(b) = self.boats.get_mut(&id) {
                            b.body.collided_horizontally = false;
                        }
                    }
                    block::WATERLILY => {
                        self.destroy_block(at, state);
                        if let Some(b) = self.boats.get_mut(&id) {
                            b.body.collided_horizontally = false;
                        }
                    }
                    _ => {}
                }
            }
        }
        let Some(b) = self.boats.get_mut(&id) else {
            return;
        };
        if b.body.on_ground {
            b.body.motion.0 *= 0.5;
            b.body.motion.1 *= 0.5;
            b.body.motion.2 *= 0.5;
        }
        let step = b.body.motion;
        self.move_boat(id, step);
        let Some(b) = self.boats.get_mut(&id) else {
            return;
        };
        if !b.body.collided_horizontally || speed_before <= CRASH_SPEED {
            b.body.motion.0 *= f64::from(DRAG);
            b.body.motion.1 *= f64::from(VERTICAL_DRAG);
            b.body.motion.2 *= f64::from(DRAG);
        } else if !b.dead {
            self.wreck_boat(id);
        }
        let Some(b) = self.boats.get_mut(&id) else {
            return;
        };
        // Turn towards where it went, at most 20 degrees.
        b.base.pitch = 0.0;
        let mut target = f64::from(b.base.yaw);
        let (dx, dz) = (prev.0 - b.body.pos.0, prev.2 - b.body.pos.2);
        if dx * dx + dz * dz > 0.001 {
            target =
                f64::from((krol_java::math::atan2(dz, dx) * 180.0 / std::f64::consts::PI) as f32);
        }
        let turn = wrap_degrees_f64(target - f64::from(b.base.yaw)).clamp(-MAX_TURN, MAX_TURN);
        b.base.yaw = (f64::from(b.base.yaw) + turn) as f32;
        // setRotation.
        b.base.yaw %= 360.0;
        b.base.pitch %= 360.0;
        // Other boats next to it get pushed apart from it.
        let area = b
            .body
            .bb()
            .expand([f64::from(0.2f32), 0.0, f64::from(0.2f32)]);
        let rider = b.rider;
        let others: Vec<i32> = self
            .boats
            .iter()
            .filter(|&(&o, c)| o != id && Some(o) != rider && c.body.bb().intersects(&area))
            .map(|(&o, _)| o)
            .collect();
        for other in others {
            self.boat_pushed_by_boat(other, id);
        }
        // A dead rider lets go.
        if let Some(b) = self.boats.get(&id)
            && let Some(r) = b.rider
            && !self
                .players
                .iter()
                .any(|p| p.entity_id == r && !p.removed && !p.dead_marked)
        {
            if let Some(b) = self.boats.get_mut(&id) {
                b.rider = None;
            }
        }
    }

    /// `Entity.applyEntityCollision` of the boat `id` with the boat `pusher`: both move
    /// apart, each only without a rider.
    fn boat_pushed_by_boat(&mut self, id: i32, pusher: i32) {
        let (Some(a), Some(p)) = (self.boats.get(&id), self.boats.get(&pusher)) else {
            return;
        };
        let Some((dx, dz)) = collision_push(a.body.pos, p.body.pos) else {
            return;
        };
        let (a_free, p_free) = (a.rider.is_none(), p.rider.is_none());
        if a_free && let Some(a) = self.boats.get_mut(&id) {
            add_velocity(a, -dx, -dz);
        }
        if p_free && let Some(p) = self.boats.get_mut(&pusher) {
            add_velocity(p, dx, dz);
        }
    }

    /// A player walking into a boat (`EntityLivingBase.collideWithEntity`, the boat's
    /// `applyEntityCollision`): both move apart, the boat only without a rider. Its own
    /// rider does not push it.
    pub(super) fn boat_pushed_by_player(&mut self, id: i32, i: usize) {
        let me = self.players[i].entity_id;
        let Some(b) = self.boats.get(&id) else {
            return;
        };
        if self.players[i].riding == Some(id) || b.rider == Some(me) {
            return;
        }
        let Some((dx, dz)) = collision_push(b.body.pos, self.players[i].pos) else {
            return;
        };
        if b.rider.is_none()
            && let Some(b) = self.boats.get_mut(&id)
        {
            add_velocity(b, -dx, -dz);
        }
        // Nothing rides players, so they always take their share.
        let p = &mut self.players[i];
        p.motion.0 += dx;
        p.motion.1 += 0.0;
        p.motion.2 += dz;
        p.airborne = true;
    }

    /// `Entity.moveEntity` for a boat: its own fall rule, then the blocks touched and
    /// fire.
    fn move_boat(&mut self, id: i32, d: (f64, f64, f64)) {
        let Some(b) = self.boats.get(&id) else {
            return;
        };
        let mut body = b.body.clone();
        let obstacles = self.boat_obstacles(id, &body.bb().add_coord([d.0, d.1, d.2]));
        let outcome =
            physics::move_entity_among(&mut self.level.overworld, &mut body, d, &obstacles);
        let Some(b) = self.boats.get_mut(&id) else {
            return;
        };
        b.body = body;
        // EntityBoat.updateFallState.
        if b.body.on_ground {
            // Vanilla quirk: a landing from 3 blocks or less keeps the fall distance.
            if b.base.fall_distance > BREAK_FALL {
                let dead = b.dead;
                // TODO(M5): fall passes the fall on to the rider (fall damage).
                if !dead {
                    self.wreck_boat(id);
                }
                if let Some(b) = self.boats.get_mut(&id) {
                    b.base.fall_distance = 0.0;
                }
            }
        } else {
            let p = b.body.pos;
            let below =
                self.level
                    .overworld
                    .block(floor_f64(p.0), floor_f64(p.1) - 1, floor_f64(p.2))
                    >> 4;
            if !matches!(below, block::FLOWING_WATER | block::WATER) && outcome.moved_y < 0.0 {
                b.base.fall_distance = (f64::from(b.base.fall_distance) - outcome.moved_y) as f32;
            }
        }
        let Some(b) = self.boats.get(&id) else {
            return;
        };
        let bb = b.body.bb();
        let touched = self.block_collisions(&bb);
        if let Some(b) = self.boats.get_mut(&id) {
            b.body.touched(touched);
            if touched.web {
                b.base.fall_distance = 0.0;
            }
        }
        self.boat_move_fire(id);
    }

    /// The end of `moveEntity`: fire or lava blocks hurt the boat and set it on fire
    /// after a while; getting wet puts the fire out with a fizz.
    fn boat_move_fire(&mut self, id: i32) {
        let Some(b) = self.boats.get(&id) else {
            return;
        };
        let p = b.body.pos;
        let (x, z) = (floor_f64(p.0), floor_f64(p.2));
        let (low, top) = (floor_f64(p.1), floor_f64(p.1 + f64::from(b.body.height)));
        let in_water = b.body.in_water;
        let bb = b.body.bb().contract([0.001, 0.001, 0.001]);
        let wet = in_water || self.rains_at((x, low, z)) || self.rains_at((x, top, z));
        if self.flammable_within(&bb) {
            self.attack_boat(id, DamageSource::InFire, 1.0);
            if !wet && let Some(b) = self.boats.get_mut(&id) {
                b.base.fire += 1;
                if b.base.fire == 0 {
                    b.base.fire = FIRE_BLOCK_SECONDS * 20;
                }
            }
        } else if let Some(b) = self.boats.get_mut(&id)
            && b.base.fire <= 0
        {
            b.base.fire = -FIRE_RESISTANCE;
        }
        if let Some(b) = self.boats.get_mut(&id)
            && wet
            && b.base.fire > 0
        {
            b.base.fire = -FIRE_RESISTANCE;
            let (pos, silent) = (b.body.pos, b.base.silent);
            // The boat's own random (unseeded in vanilla).
            let r = &mut self.entity_rand;
            let pitch = FIZZ_PITCH + (r.next_float() - r.next_float()) * FIZZ_SPREAD;
            if !silent {
                self.play_sound_near_except(None, "random.fizz", pos, FIZZ_VOLUME, pitch);
            }
        }
    }

    /// A crash or a hard landing: the boat falls apart into planks and sticks.
    fn wreck_boat(&mut self, id: i32) {
        let Some(b) = self.boats.get_mut(&id) else {
            return;
        };
        b.dead = true;
        let pos = b.body.pos;
        if self.level.data.game_rule("doEntityDrops") == Some("false") {
            return;
        }
        let drop = |id: i16| ItemStack {
            id,
            count: 1,
            damage: 0,
            nbt: None,
        };
        for _ in 0..WRECK_PLANKS {
            self.spawn_item(pos, drop(block::PLANKS as i16), DROP_PICKUP_DELAY);
        }
        for _ in 0..WRECK_STICKS {
            self.spawn_item(pos, drop(item::STICK), DROP_PICKUP_DELAY);
        }
    }

    /// `EntityBoat.attackEntityFrom`: the boat wobbles and takes 10 damage per point; a
    /// creative player or more than 40 damage breaks it (dropping a boat unless a
    /// creative player broke it). Its rider's own projectiles do nothing.
    pub(super) fn attack_boat(&mut self, id: i32, source: DamageSource, amount: f32) -> bool {
        // The entity behind the damage (a shooter too) being a creative player.
        let creative = source.attacker().is_some_and(|a| {
            self.players
                .iter()
                .any(|p| p.entity_id == a && p.gamemode == CREATIVE)
        });
        let Some(b) = self.boats.get_mut(&id) else {
            return false;
        };
        if b.base.invulnerable && source != DamageSource::OutOfWorld && !creative {
            return false;
        }
        if b.dead {
            return true;
        }
        let indirect = matches!(
            source,
            DamageSource::Arrow { .. }
                | DamageSource::Thrown(_)
                | DamageSource::IndirectMagic(_)
                | DamageSource::ExplosionBy(_)
        );
        if indirect && b.rider.is_some() && b.rider == source.attacker() {
            return false;
        }
        b.set_forward_direction(-b.forward_direction);
        b.set_time_since_hit(WOBBLE);
        b.set_damage(b.damage + amount * DAMAGE_PER_POINT);
        b.body.velocity_changed = true;
        if creative || b.damage > BREAK_DAMAGE {
            // Vanilla quirk: the rider "mounts" the boat it already rides (nothing
            // happens) and only lets go silently once the boat is dead.
            let pos = b.body.pos;
            b.dead = true;
            if !creative && self.level.data.game_rule("doEntityDrops") != Some("false") {
                let stack = ItemStack {
                    id: item::BOAT,
                    count: 1,
                    damage: 0,
                    nbt: None,
                };
                self.spawn_item(pos, stack, DROP_PICKUP_DELAY);
            }
        }
        true
    }

    /// `EntityBoat.interactFirst`: a boat without another player in it takes the
    /// player in.
    pub(super) fn interact_boat(&mut self, i: usize, id: i32) {
        // EntityPlayer.interactWith: spectators only open inventories.
        if self.players[i].gamemode == SPECTATOR {
            return;
        }
        let me = self.players[i].entity_id;
        let Some(b) = self.boats.get(&id) else {
            return;
        };
        if b.rider.is_some_and(|r| r != me) {
            return;
        }
        self.mount_player(i, id);
    }

    /// `writeToNBTOptional` of a boat.
    pub(super) fn boat_nbt(&self, b: &Boat, uuid: u128, c: &mut Compound) {
        c.insert("id", krol_nbt::Tag::String(SAVED_ID.into()));
        b.base.write_entity(&b.body, uuid, c);
    }
}

/// The push of `Entity.applyEntityCollision` between an entity at `at` and one at
/// `other`: what `other` gets (the first one gets the opposite), if they are apart.
fn collision_push(at: (f64, f64, f64), other: (f64, f64, f64)) -> Option<(f64, f64)> {
    let (mut dx, mut dz) = (other.0 - at.0, other.2 - at.2);
    let mut d = krol_java::math::abs_max(dx, dz);
    if d < PUSH_MIN_DISTANCE {
        return None;
    }
    d = f64::from(krol_java::math::sqrt_f64(d));
    dx /= d;
    dz /= d;
    let inv = (1.0 / d).min(1.0);
    dx *= inv;
    dz *= inv;
    Some((dx * PUSH, dz * PUSH))
}

/// `Entity.addVelocity(x, 0, z)` on a boat.
fn add_velocity(b: &mut Boat, x: f64, z: f64) {
    b.body.motion.0 += x;
    b.body.motion.1 += 0.0;
    b.body.motion.2 += z;
    b.airborne = true;
}
