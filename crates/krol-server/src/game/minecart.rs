//! Minecarts (vanilla: `EntityMinecart` with `EntityMinecartEmpty`, placed by
//! `ItemMinecart`).
//!
//! A minecart on a rail follows it: its motion is turned along the rail, it snaps onto
//! the rail's line, moves at most 0.4 per tick, slows down by 4% a tick, speeds up down
//! slopes and on powered rails (and stops on unpowered ones). Off the rails it falls and
//! slides. Minecarts bump into each other, players push them by walking into them, and
//! hits wobble them and add damage that wears off; past 40 (or at once for a creative
//! player) they break, dropping a minecart item.
//!
//! TODO(M5): riding (players and mobs getting in, mobs pulled in by moving carts,
//! steering, dismounting on activator rails), chest, hopper, furnace, TNT, spawner and
//! command block minecarts, dispensers placing minecarts, nether portals.

use std::collections::BTreeSet;

use krol_java::math::{floor_f64, wrap_degrees_f32};
use krol_nbt::{Compound, Tag};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack, Metadata, MetadataValue};

use super::Game;
use super::damage::DamageSource;
use super::ids::{block, item};
use super::physics::{self, Aabb, Body};
use super::rails;
use super::saved_entities::EntityBase;

/// Saved id of the rideable minecart (`EntityList` name).
pub const RIDEABLE_ID: &str = "MinecartRideable";
/// `getName`: the untranslated key (en_US names only `entity.Minecart.name`).
pub const RIDEABLE_NAME: &str = "entity.MinecartRideable.name";
/// `S0EPacketSpawnObject` type of minecarts; the data is the minecart type.
pub const OBJECT_TYPE: i8 = 10;
/// `EnumMinecartType.RIDEABLE`'s network id.
const RIDEABLE_TYPE: i32 = 0;
/// `setSize(0.98F, 0.7F)`.
const WIDTH: f32 = 0.98;
const HEIGHT: f32 = 0.7;
/// Gravity (`0.04F`).
const GRAVITY: f32 = 0.04;
/// `getMaximumSpeed`.
const MAX_SPEED: f64 = 0.4;
/// The speed along a rail never starts above this.
const MAX_RAIL_SPEED: f64 = 2.0;
/// Slopes pull the motion down by this much each tick.
const SLOPE_PULL: f64 = 0.0078125;
/// Unpowered powered rails stop carts slower than this and halve faster ones.
const BRAKE_STOP_SPEED: f64 = 0.03;
/// Powered rails push carts faster than this by 0.06, and start resting ones against a
/// wall at 0.02.
const BOOST_MIN_SPEED: f64 = 0.01;
const BOOST: f64 = 0.06;
const KICK_OFF: f64 = 0.02;
/// Drag on the rails without a rider (`0.96F`) and off them (`0.95F`, airborne).
const RAIL_DRAG: f32 = 0.96;
const AIR_DRAG: f32 = 0.95;
/// Drag on the rails with a rider (`0.997F`).
const RIDDEN_DRAG: f32 = 0.997;
/// A ridden cart moves at this share of its motion.
const RIDDEN_SHARE: f64 = 0.75;
/// A rider pushing forward adds this to a cart slower than the root of
/// `RIDER_START_SPEED_SQ`.
const RIDER_PUSH: f64 = 0.1;
const RIDER_START_SPEED_SQ: f64 = 0.01;
/// Height differences along the rail change the speed by this share.
const SLOPE_SPEED: f64 = 0.05;
/// The rail line sits this high above the rail block.
const RAIL_HEIGHT: f64 = 0.0625;
/// Entities below this height are killed.
const VOID_Y: f64 = -64.0;
/// Turning more than this in a tick flips the cart around instead.
const FLIP_ANGLE: f64 = 170.0;
/// Hits add 10 damage per point; past this the cart breaks.
const DAMAGE_PER_POINT: f32 = 10.0;
const BREAK_DAMAGE: f32 = 40.0;
/// A hit wobbles the cart this much (counted down every tick).
const WOBBLE: i32 = 10;
/// Activator rails set this damage (without breaking the cart).
const ACTIVATOR_DAMAGE: f32 = 50.0;
/// Carts push each other only when lined up at least this well.
const LINED_UP: f32 = 0.8;
/// `Entity.fireResistance`.
const FIRE_RESISTANCE: i16 = 1;
/// Fire blocks set entities that keep standing in them on fire for this long.
const FIRE_BLOCK_SECONDS: i16 = 8;
/// The fizz of a burning cart getting wet.
const FIZZ_VOLUME: f32 = 0.7;
const FIZZ_PITCH: f32 = 1.6;
const FIZZ_SPREAD: f32 = 0.4;
/// The pickup delay of dropped items (`setDefaultPickupDelay`).
const DROP_PICKUP_DELAY: i32 = 10;
/// The creative game mode.
const CREATIVE: i32 = 1;
/// The spectator game mode.
const SPECTATOR: i32 = 3;

/// Metadata entries: wobble amplitude and direction, damage, display tile, its
/// offset, and whether it is custom.
const ROLLING_AMPLITUDE: u8 = 17;
const ROLLING_DIRECTION: u8 = 18;
const DAMAGE: u8 = 19;
const DISPLAY_TILE: u8 = 20;
const DISPLAY_OFFSET: u8 = 21;
const HAS_DISPLAY: u8 = 22;
/// `getDefaultDisplayTileOffset`.
const DEFAULT_DISPLAY_OFFSET: i32 = 6;

/// The two ends of each rail shape relative to the rail block (`matrix`): x, y, z.
const ENDS: [[[i32; 3]; 2]; 10] = [
    [[0, 0, -1], [0, 0, 1]],
    [[-1, 0, 0], [1, 0, 0]],
    [[-1, -1, 0], [1, 0, 0]],
    [[-1, 0, 0], [1, -1, 0]],
    [[0, 0, -1], [0, -1, 1]],
    [[0, -1, -1], [0, 0, 1]],
    [[0, 0, 1], [1, 0, 0]],
    [[0, 0, 1], [-1, 0, 0]],
    [[0, 0, -1], [-1, 0, 0]],
    [[0, 0, -1], [1, 0, 0]],
];

/// One minecart.
#[derive(Debug, Clone)]
pub struct Minecart {
    /// Position, motion and the movement state.
    pub body: Body,
    /// The base entity state (its custom name is the cart's `entityName`, which the
    /// data watcher never shows).
    pub base: EntityBase,
    /// `isInReverse`.
    reverse: bool,
    /// Metadata 17 and 18: the wobble.
    rolling_amplitude: i32,
    rolling_direction: i32,
    /// Metadata 19.
    damage: f32,
    /// A custom display tile (metadata 20-22): state and offset.
    display: Option<(u16, i32)>,
    /// Metadata entries changed since the tracker last sent them.
    pub dirty_metadata: BTreeSet<u8>,
    /// Some metadata changed since the tracker last looked.
    pub metadata_changed: bool,
    /// The player riding it (`riddenByEntity`), by entity id.
    pub rider: Option<i32>,
    /// `isAirBorne`: pushed by `addVelocity`; the tracker sends the move at once and
    /// clears it.
    pub airborne: bool,
    /// Removed from the world (`isDead`).
    pub dead: bool,
    /// The chunk it is in (as of its last update).
    pub chunk: (i32, i32),
}

/// `Block.getStateId`.
fn state_id(state: u16) -> i32 {
    i32::from(state >> 4) | (i32::from(state & 15) << 12)
}

/// `ItemStack.setStackDisplayName`.
fn set_display_name(stack: &mut ItemStack, name: &str) {
    let tag = stack.nbt.get_or_insert_with(Compound::new);
    let mut display = tag.get_compound("display").cloned().unwrap_or_default();
    display.insert("Name", Tag::String(name.to_owned()));
    tag.insert("display", Tag::Compound(display));
}

fn chunk_of(pos: (f64, f64, f64)) -> (i32, i32) {
    (floor_f64(pos.0) >> 4, floor_f64(pos.2) >> 4)
}

/// `Vec3.normalize` of a horizontal vector.
fn normalize(x: f64, z: f64) -> (f64, f64) {
    let d = f64::from(krol_java::math::sqrt_f64(x * x + z * z));
    if d < 1.0e-4 {
        (0.0, 0.0)
    } else {
        (x / d, z / d)
    }
}

impl Minecart {
    /// A new rideable minecart at `pos` (`EntityMinecart(world, x, y, z)`).
    fn new(pos: (f64, f64, f64)) -> Self {
        let mut body = Body::player(pos, false);
        body.width = WIDTH;
        body.height = HEIGHT;
        body.step_height = 0.0;
        let mut m = Self {
            body,
            base: EntityBase::default(),
            reverse: false,
            rolling_amplitude: 0,
            rolling_direction: 1,
            damage: 0.0,
            display: None,
            dirty_metadata: BTreeSet::new(),
            metadata_changed: false,
            rider: None,
            airborne: false,
            dead: false,
            chunk: chunk_of(pos),
        };
        m.dirty_metadata = m.metadata().iter().map(|e| e.index).collect();
        m
    }

    fn metadata_updated(&mut self, index: u8) {
        self.dirty_metadata.insert(index);
        self.metadata_changed = true;
    }

    fn set_rolling_amplitude(&mut self, v: i32) {
        if v != self.rolling_amplitude {
            self.rolling_amplitude = v;
            self.metadata_updated(ROLLING_AMPLITUDE);
        }
    }

    fn set_rolling_direction(&mut self, v: i32) {
        if v != self.rolling_direction {
            self.rolling_direction = v;
            self.metadata_updated(ROLLING_DIRECTION);
        }
    }

    fn set_damage(&mut self, v: f32) {
        if v != self.damage {
            self.damage = v;
            self.metadata_updated(DAMAGE);
        }
    }

    /// `getName`.
    pub fn name(&self) -> String {
        self.base
            .custom_name
            .clone()
            .unwrap_or_else(|| RIDEABLE_NAME.to_owned())
    }

    /// The minecart type the spawn packet carries.
    pub fn type_id(&self) -> i32 {
        RIDEABLE_TYPE
    }

    /// The complete data watcher, in index order.
    pub fn metadata(&self) -> Vec<Metadata> {
        use MetadataValue::{Byte, Float, Int, String};
        let md = |index, value| Metadata { index, value };
        let mut out = self.base.metadata();
        // The custom name lives outside the data watcher.
        out[2] = md(2, String(std::string::String::new()));
        let (tile, offset, has) = match self.display {
            Some((state, offset)) => (state_id(state), offset, 1),
            None => (0, DEFAULT_DISPLAY_OFFSET, 0),
        };
        out.push(md(ROLLING_AMPLITUDE, Int(self.rolling_amplitude)));
        out.push(md(ROLLING_DIRECTION, Int(self.rolling_direction)));
        out.push(md(DAMAGE, Float(self.damage)));
        out.push(md(DISPLAY_TILE, Int(tile)));
        out.push(md(DISPLAY_OFFSET, Int(offset)));
        out.push(md(HAS_DISPLAY, Byte(has)));
        // DataWatcher keeps its entries in a HashMap: with only 11 of them (16
        // buckets) entries 17-22 come between 0-4.
        let indices: Vec<i32> = out.iter().map(|e| i32::from(e.index)).collect();
        krol_java::java_hash_order(&indices)
            .into_iter()
            .map(|k| out[k].clone())
            .collect()
    }

    /// `readEntityFromNBT` of a minecart (`body` and `base` already read).
    pub fn load(mut body: Body, base: EntityBase, c: &Compound) -> Self {
        body.width = WIDTH;
        body.height = HEIGHT;
        body.step_height = 0.0;
        let mut m = Self::new(body.pos);
        m.body = body;
        m.base = base;
        if c.get_bool("CustomDisplayTile") {
            let data = c.get_i32("DisplayData");
            let id = if c.contains_key_of("DisplayTile", Some(krol_nbt::TagType::String)) {
                super::scheduler::block_by_name(c.get_str("DisplayTile"))
            } else {
                u16::try_from(c.get_i32("DisplayTile"))
                    .ok()
                    .filter(|&id| super::scheduler::block_name(id).is_some())
            };
            let state = id.map_or(0, |id| {
                let meta = super::world::normalize_meta(id, data as u8).unwrap_or(0);
                id << 4 | u16::from(meta)
            });
            m.display = Some((state, c.get_i32("DisplayOffset")));
            for index in [DISPLAY_TILE, DISPLAY_OFFSET, HAS_DISPLAY] {
                m.metadata_updated(index);
            }
        }
        // Reading the base state changes the data watcher only where the values differ
        // from a new cart's (the custom name lives outside it).
        if m.base.air != super::saved_entities::FULL_AIR
            || m.base.custom_name_visible
            || m.base.silent
        {
            m.metadata_changed = true;
        }
        m.chunk = chunk_of(m.body.pos);
        m
    }

    /// `writeEntityToNBT` of a minecart.
    pub fn write(&self, c: &mut Compound) {
        if let Some((state, offset)) = self.display {
            c.insert("CustomDisplayTile", Tag::Byte(1));
            c.insert(
                "DisplayTile",
                Tag::String(super::scheduler::block_name(state >> 4).unwrap_or_default()),
            );
            c.insert("DisplayData", Tag::Int(i32::from(state & 15)));
            c.insert("DisplayOffset", Tag::Int(offset));
        }
        if let Some(name) = &self.base.custom_name {
            c.insert("CustomName", Tag::String(name.clone()));
        }
    }
}

/// `func_70489_a`: the point on the rail line under `pos`, if there is a rail there (or
/// just below).
fn rail_point(world: &mut super::world::World, pos: (f64, f64, f64)) -> Option<(f64, f64, f64)> {
    let (bx, mut by, bz) = (floor_f64(pos.0), floor_f64(pos.1), floor_f64(pos.2));
    if rails::is_rail(world.block(bx, by - 1, bz) >> 4) {
        by -= 1;
    }
    let state = world.block(bx, by, bz);
    if !rails::is_rail(state >> 4) {
        return None;
    }
    let e = ENDS[usize::from(rails::shape(state))];
    let (x0, y0, z0) = (
        f64::from(bx) + 0.5 + f64::from(e[0][0]) * 0.5,
        f64::from(by) + RAIL_HEIGHT + f64::from(e[0][1]) * 0.5,
        f64::from(bz) + 0.5 + f64::from(e[0][2]) * 0.5,
    );
    let (x1, y1, z1) = (
        f64::from(bx) + 0.5 + f64::from(e[1][0]) * 0.5,
        f64::from(by) + RAIL_HEIGHT + f64::from(e[1][1]) * 0.5,
        f64::from(bz) + 0.5 + f64::from(e[1][2]) * 0.5,
    );
    let (dx, dy, dz) = (x1 - x0, (y1 - y0) * 2.0, z1 - z0);
    let (x, z) = (pos.0, pos.2);
    let t = if dx == 0.0 {
        z - f64::from(bz)
    } else if dz == 0.0 {
        x - f64::from(bx)
    } else {
        ((x - x0) * dx + (z - z0) * dz) * 2.0
    };
    let mut y = y0 + dy * t;
    if dy < 0.0 {
        y += 1.0;
    }
    if dy > 0.0 {
        y += 0.5;
    }
    Some((x0 + dx * t, y, z0 + dz * t))
}

impl Game {
    /// `ItemMinecart.onItemUse`: on a rail, a minecart appears on it (half a block
    /// higher on a slope), named after the item if it has a name. Adventure players may
    /// do this too.
    pub(super) fn place_minecart(&mut self, i: usize, pos: (i32, i32, i32)) -> bool {
        let state = self.level.overworld.block(pos.0, pos.1, pos.2);
        if !rails::is_rail(state >> 4) {
            return false;
        }
        let Some(stack) = self.players[i].held().cloned() else {
            return false;
        };
        let lift = if rails::is_ascending(rails::shape(state)) {
            0.5
        } else {
            0.0
        };
        let at = (
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + RAIL_HEIGHT + lift,
            f64::from(pos.2) + 0.5,
        );
        let id = self.next_entity_id();
        let mut cart = Minecart::new(at);
        cart.base.custom_name = super::item_data::custom_name(&stack).map(str::to_owned);
        self.minecarts.insert(id, cart);
        self.spawn_entity_in_world(id);
        let p = &mut self.players[i];
        if p.gamemode != CREATIVE {
            let held = 36 + p.selected;
            if let Some(s) = p.inventory[held].as_mut() {
                s.count -= 1;
            }
        }
        self.count_item_use(i);
        true
    }

    /// One minecart's turn in the entity loop (`EntityMinecart.onUpdate` on the server).
    pub(super) fn update_minecart(&mut self, id: i32, prev_yaw: f32) {
        if self.minecarts.get(&id).is_some_and(|m| !m.dead) {
            self.tick_minecart(id, prev_yaw);
            if let Some(m) = self.minecarts.get_mut(&id) {
                m.chunk = chunk_of(m.body.pos);
            }
            self.relocate_entity(id);
            // updateEntityWithOptionalForce: then its rider.
            self.update_vehicle_rider(id);
        }
        if self.minecarts.get(&id).is_some_and(|m| m.dead) {
            if let Some(m) = self.minecarts.remove(&id)
                && let Some(rider) = m.rider
                && let Some(p) = self.players.iter_mut().find(|p| p.entity_id == rider)
                && p.riding == Some(id)
            {
                p.riding = None;
            }
            self.remove_entity_from_chunk(id);
            self.untrack_entity(id);
        }
    }

    fn tick_minecart(&mut self, id: i32, prev_yaw: f32) {
        let Some(m) = self.minecarts.get_mut(&id) else {
            return;
        };
        if m.rolling_amplitude > 0 {
            m.set_rolling_amplitude(m.rolling_amplitude - 1);
        }
        if m.damage > 0.0 {
            m.set_damage(m.damage - 1.0);
        }
        if m.body.pos.1 < VOID_Y {
            m.dead = true;
        }
        if m.base.portal_cooldown > 0 {
            m.base.portal_cooldown -= 1;
        }
        let prev = m.body.pos;
        m.body.motion.1 -= f64::from(GRAVITY);
        let (x, mut y, z) = (floor_f64(prev.0), floor_f64(prev.1), floor_f64(prev.2));
        if rails::is_rail(self.level.overworld.block(x, y - 1, z) >> 4) {
            y -= 1;
        }
        let state = self.level.overworld.block(x, y, z);
        if rails::is_rail(state >> 4) {
            self.minecart_on_rail(id, (x, y, z), state);
            if state >> 4 == block::ACTIVATOR_RAIL {
                self.minecart_activator_pass(id, state & rails::POWERED_BIT != 0);
            }
        } else {
            self.minecart_derailed(id);
        }
        // onUpdate runs doBlockCollisions once more after moveEntity's.
        self.minecart_block_collisions(id);
        let Some(m) = self.minecarts.get_mut(&id) else {
            return;
        };
        m.base.pitch = 0.0;
        let (dx, dz) = (prev.0 - m.body.pos.0, prev.2 - m.body.pos.2);
        if dx * dx + dz * dz > 0.001 {
            m.base.yaw = (krol_java::math::atan2(dz, dx) * 180.0 / std::f64::consts::PI) as f32;
            if m.reverse {
                m.base.yaw += 180.0;
            }
        }
        let turn = f64::from(wrap_degrees_f32(m.base.yaw - prev_yaw));
        if !(-FLIP_ANGLE..FLIP_ANGLE).contains(&turn) {
            m.base.yaw += 180.0;
            m.reverse = !m.reverse;
        }
        // setRotation.
        m.base.yaw %= 360.0;
        m.base.pitch %= 360.0;
        // Other minecarts bump into this one.
        let area = m
            .body
            .bb()
            .expand([f64::from(0.2f32), 0.0, f64::from(0.2f32)]);
        let others: Vec<i32> = self
            .minecarts
            .iter()
            .filter(|&(&o, c)| o != id && !c.dead && c.body.bb().intersects(&area))
            .map(|(&o, _)| o)
            .collect();
        for other in others {
            self.minecart_collision(other, CartPusher::Minecart(id));
        }
        self.minecart_water_movement(id);
    }

    /// `func_180460_a`: moving along the rail at `rail`.
    fn minecart_on_rail(&mut self, id: i32, rail: (i32, i32, i32), state: u16) {
        let Some(m) = self.minecarts.get(&id) else {
            return;
        };
        let start_pos = m.body.pos;
        let start = rail_point(&mut self.level.overworld, start_pos);
        let Some(m) = self.minecarts.get_mut(&id) else {
            return;
        };
        m.base.fall_distance = 0.0;
        m.body.pos.1 = f64::from(rail.1);
        let golden = state >> 4 == block::GOLDEN_RAIL;
        let boost = golden && state & rails::POWERED_BIT != 0;
        let mut brake = golden && !boost;
        let rider = m
            .rider
            .and_then(|r| self.players.iter().find(|p| p.entity_id == r))
            .map(|p| (p.move_forward, p.rotation.0));
        let shape = rails::shape(state);
        match shape {
            rails::ASCENDING_EAST => {
                m.body.motion.0 -= SLOPE_PULL;
                m.body.pos.1 += 1.0;
            }
            rails::ASCENDING_WEST => {
                m.body.motion.0 += SLOPE_PULL;
                m.body.pos.1 += 1.0;
            }
            rails::ASCENDING_NORTH => {
                m.body.motion.2 += SLOPE_PULL;
                m.body.pos.1 += 1.0;
            }
            rails::ASCENDING_SOUTH => {
                m.body.motion.2 -= SLOPE_PULL;
                m.body.pos.1 += 1.0;
            }
            _ => {}
        }
        let e = ENDS[usize::from(shape)];
        let mut dx = f64::from(e[1][0] - e[0][0]);
        let mut dz = f64::from(e[1][2] - e[0][2]);
        let len = (dx * dx + dz * dz).sqrt();
        let (mx, mz) = (m.body.motion.0, m.body.motion.2);
        if mx * dx + mz * dz < 0.0 {
            dx = -dx;
            dz = -dz;
        }
        let speed = (mx * mx + mz * mz).sqrt().min(MAX_RAIL_SPEED);
        m.body.motion.0 = speed * dx / len;
        m.body.motion.2 = speed * dz / len;
        // A rider pushing forward starts a (nearly) resting cart where it looks.
        if let Some((forward, yaw)) = rider
            && forward > 0.0
        {
            // Math.sin and Math.cos of a double (HotSpot may differ from f64 by an ulp).
            let angle = f64::from(yaw * std::f32::consts::PI / 180.0);
            let (px, pz) = (-angle.sin(), angle.cos());
            let sq = m.body.motion.0 * m.body.motion.0 + m.body.motion.2 * m.body.motion.2;
            if sq < RIDER_START_SPEED_SQ {
                m.body.motion.0 += px * RIDER_PUSH;
                m.body.motion.2 += pz * RIDER_PUSH;
                brake = false;
            }
        }
        if brake {
            let s = (m.body.motion.0 * m.body.motion.0 + m.body.motion.2 * m.body.motion.2).sqrt();
            if s < BRAKE_STOP_SPEED {
                m.body.motion.0 *= 0.0;
                m.body.motion.1 *= 0.0;
                m.body.motion.2 *= 0.0;
            } else {
                m.body.motion.0 *= 0.5;
                m.body.motion.1 *= 0.0;
                m.body.motion.2 *= 0.5;
            }
        }
        let (bx, bz) = (f64::from(rail.0), f64::from(rail.2));
        let x0 = bx + 0.5 + f64::from(e[0][0]) * 0.5;
        let z0 = bz + 0.5 + f64::from(e[0][2]) * 0.5;
        let x1 = bx + 0.5 + f64::from(e[1][0]) * 0.5;
        let z1 = bz + 0.5 + f64::from(e[1][2]) * 0.5;
        let (dx, dz) = (x1 - x0, z1 - z0);
        let t = if dx == 0.0 {
            m.body.pos.0 = bx + 0.5;
            m.body.pos.2 - bz
        } else if dz == 0.0 {
            m.body.pos.2 = bz + 0.5;
            m.body.pos.0 - bx
        } else {
            ((m.body.pos.0 - x0) * dx + (m.body.pos.2 - z0) * dz) * 2.0
        };
        m.body.pos.0 = x0 + dx * t;
        m.body.pos.2 = z0 + dz * t;
        // A cart with a rider moves at three quarters of its motion.
        let share = if rider.is_some() { RIDDEN_SHARE } else { 1.0 };
        let step = (
            (m.body.motion.0 * share).clamp(-MAX_SPEED, MAX_SPEED),
            0.0,
            (m.body.motion.2 * share).clamp(-MAX_SPEED, MAX_SPEED),
        );
        self.move_minecart(id, step);
        let Some(m) = self.minecarts.get_mut(&id) else {
            return;
        };
        let (fx, fz) = (floor_f64(m.body.pos.0), floor_f64(m.body.pos.2));
        if e[0][1] != 0 && fx - rail.0 == e[0][0] && fz - rail.2 == e[0][2] {
            m.body.pos.1 += f64::from(e[0][1]);
        } else if e[1][1] != 0 && fx - rail.0 == e[1][0] && fz - rail.2 == e[1][2] {
            m.body.pos.1 += f64::from(e[1][1]);
        }
        // applyDrag: much less with a rider.
        let drag = if m.rider.is_some() {
            RIDDEN_DRAG
        } else {
            RAIL_DRAG
        };
        m.body.motion.0 *= f64::from(drag);
        m.body.motion.1 *= 0.0;
        m.body.motion.2 *= f64::from(drag);
        let pos = m.body.pos;
        let end = rail_point(&mut self.level.overworld, pos);
        let Some(m) = self.minecarts.get_mut(&id) else {
            return;
        };
        if let (Some(end), Some(start)) = (end, start) {
            let lift = (start.1 - end.1) * SLOPE_SPEED;
            let s = (m.body.motion.0 * m.body.motion.0 + m.body.motion.2 * m.body.motion.2).sqrt();
            if s > 0.0 {
                m.body.motion.0 = m.body.motion.0 / s * (s + lift);
                m.body.motion.2 = m.body.motion.2 / s * (s + lift);
            }
            m.body.pos.1 = end.1;
        }
        let (fx, fz) = (floor_f64(m.body.pos.0), floor_f64(m.body.pos.2));
        if fx != rail.0 || fz != rail.2 {
            let s = (m.body.motion.0 * m.body.motion.0 + m.body.motion.2 * m.body.motion.2).sqrt();
            m.body.motion.0 = s * f64::from(fx - rail.0);
            m.body.motion.2 = s * f64::from(fz - rail.2);
        }
        if boost {
            let s = (m.body.motion.0 * m.body.motion.0 + m.body.motion.2 * m.body.motion.2).sqrt();
            if s > BOOST_MIN_SPEED {
                m.body.motion.0 += m.body.motion.0 / s * BOOST;
                m.body.motion.2 += m.body.motion.2 / s * BOOST;
            } else {
                let wall = |g: &mut Self, p: (i32, i32, i32)| {
                    super::placement::is_normal_cube(g.level.overworld.block(p.0, p.1, p.2))
                };
                let (x, y, z) = rail;
                if shape == rails::EAST_WEST {
                    if wall(self, (x - 1, y, z)) {
                        self.set_cart_motion_x(id, KICK_OFF);
                    } else if wall(self, (x + 1, y, z)) {
                        self.set_cart_motion_x(id, -KICK_OFF);
                    }
                } else if shape == rails::NORTH_SOUTH {
                    if wall(self, (x, y, z - 1)) {
                        self.set_cart_motion_z(id, KICK_OFF);
                    } else if wall(self, (x, y, z + 1)) {
                        self.set_cart_motion_z(id, -KICK_OFF);
                    }
                }
            }
        }
    }

    fn set_cart_motion_x(&mut self, id: i32, v: f64) {
        if let Some(m) = self.minecarts.get_mut(&id) {
            m.body.motion.0 = v;
        }
    }

    fn set_cart_motion_z(&mut self, id: i32, v: f64) {
        if let Some(m) = self.minecarts.get_mut(&id) {
            m.body.motion.2 = v;
        }
    }

    /// `moveDerailedMinecart`: off the rails, at most 0.4 per tick sideways, halved on
    /// the ground, 5% drag in the air.
    fn minecart_derailed(&mut self, id: i32) {
        let Some(m) = self.minecarts.get_mut(&id) else {
            return;
        };
        m.body.motion.0 = m.body.motion.0.clamp(-MAX_SPEED, MAX_SPEED);
        m.body.motion.2 = m.body.motion.2.clamp(-MAX_SPEED, MAX_SPEED);
        if m.body.on_ground {
            m.body.motion.0 *= 0.5;
            m.body.motion.1 *= 0.5;
            m.body.motion.2 *= 0.5;
        }
        let step = m.body.motion;
        self.move_minecart(id, step);
        if let Some(m) = self.minecarts.get_mut(&id)
            && !m.body.on_ground
        {
            m.body.motion.0 *= f64::from(AIR_DRAG);
            m.body.motion.1 *= f64::from(AIR_DRAG);
            m.body.motion.2 *= f64::from(AIR_DRAG);
        }
    }

    /// `EntityMinecartEmpty.onActivatorRailPass`: a powered activator rail wobbles the
    /// cart (and would throw out its rider).
    fn minecart_activator_pass(&mut self, id: i32, powered: bool) {
        if !powered {
            return;
        }
        self.dismount_rider_of(id);
        if let Some(m) = self.minecarts.get_mut(&id)
            && m.rolling_amplitude == 0
        {
            m.set_rolling_direction(-m.rolling_direction);
            m.set_rolling_amplitude(WOBBLE);
            m.set_damage(ACTIVATOR_DAMAGE);
            m.body.velocity_changed = true;
        }
    }

    /// The boxes a moving minecart collides with besides blocks: pushable entities
    /// near it (players but spectators, other minecarts).
    fn cart_obstacles(&self, id: i32, area: &Aabb) -> Vec<Aabb> {
        let near = area.expand([0.25, 0.25, 0.25]);
        // Not its own rider.
        let rider = self.minecarts.get(&id).and_then(|m| m.rider);
        let mut boxes: Vec<Aabb> = self
            .players
            .iter()
            .filter(|p| {
                !p.removed
                    && p.gamemode != SPECTATOR
                    && p.health() > 0.0
                    && Some(p.entity_id) != rider
            })
            .map(|p| Aabb::at(p.pos, 0.6, 1.8))
            .filter(|b| b.intersects(&near))
            .collect();
        boxes.extend(
            self.minecarts
                .iter()
                .filter(|&(&o, c)| o != id && !c.dead)
                .map(|(_, c)| c.body.bb())
                .filter(|b| b.intersects(&near)),
        );
        boxes
    }

    /// `Entity.moveEntity` for a minecart: no step sounds; then the fall, the blocks
    /// touched and fire.
    fn move_minecart(&mut self, id: i32, d: (f64, f64, f64)) {
        let Some(m) = self.minecarts.get(&id) else {
            return;
        };
        let mut body = m.body.clone();
        let obstacles = self.cart_obstacles(id, &body.bb().add_coord([d.0, d.1, d.2]));
        let outcome =
            physics::move_entity_among(&mut self.level.overworld, &mut body, d, &obstacles);
        let Some(m) = self.minecarts.get_mut(&id) else {
            return;
        };
        m.body = body;
        m.base.update_fall_state(m.body.on_ground, outcome.moved_y);
        self.minecart_block_collisions(id);
        self.minecart_move_fire(id);
    }

    /// `doBlockCollisions` for a minecart: webs, soul sand, pressure plates and
    /// detector rails.
    fn minecart_block_collisions(&mut self, id: i32) {
        let Some(m) = self.minecarts.get(&id) else {
            return;
        };
        let bb = m.body.bb();
        let touched = self.block_collisions(&bb);
        if let Some(m) = self.minecarts.get_mut(&id) {
            m.body.touched(touched);
            if touched.web {
                m.base.fall_distance = 0.0;
            }
        }
    }

    /// The end of `moveEntity`: fire or lava blocks hurt the cart and set it on fire
    /// after a while; getting wet puts the fire out with a fizz.
    fn minecart_move_fire(&mut self, id: i32) {
        let wet = self.minecart_wet(id);
        let Some(m) = self.minecarts.get(&id) else {
            return;
        };
        let bb = m.body.bb().contract([0.001, 0.001, 0.001]);
        if self.flammable_within(&bb) {
            let _ = self.attack_minecart(id, DamageSource::InFire, 1.0);
            if !wet && let Some(m) = self.minecarts.get_mut(&id) {
                m.base.fire += 1;
                if m.base.fire == 0 {
                    m.base.fire = FIRE_BLOCK_SECONDS * 20;
                }
            }
        } else if let Some(m) = self.minecarts.get_mut(&id)
            && m.base.fire <= 0
        {
            m.base.fire = -FIRE_RESISTANCE;
        }
        if let Some(m) = self.minecarts.get_mut(&id)
            && wet
            && m.base.fire > 0
        {
            m.base.fire = -FIRE_RESISTANCE;
            let pos = m.body.pos;
            let silent = m.base.silent;
            // The cart's own random (unseeded in vanilla).
            let r = &mut self.entity_rand;
            let pitch = FIZZ_PITCH + (r.next_float() - r.next_float()) * FIZZ_SPREAD;
            if !silent {
                self.play_sound_near_except(None, "random.fizz", pos, FIZZ_VOLUME, pitch);
            }
        }
    }

    /// `Entity.isWet`: in water or in the rain at the bottom or the top.
    fn minecart_wet(&mut self, id: i32) -> bool {
        let Some(m) = self.minecarts.get(&id) else {
            return false;
        };
        if m.body.in_water {
            return true;
        }
        let p = m.body.pos;
        let (x, z) = (floor_f64(p.0), floor_f64(p.2));
        let (low, top) = (floor_f64(p.1), floor_f64(p.1 + f64::from(m.body.height)));
        self.rains_at((x, low, z)) || self.rains_at((x, top, z))
    }

    /// `handleWaterMovement` at the end of the update: water pushes the cart and puts
    /// out its fire. (Vanilla quirk: minecarts never finish their first update, so they
    /// never splash.)
    fn minecart_water_movement(&mut self, id: i32) {
        let Some(m) = self.minecarts.get(&id) else {
            return;
        };
        let bb = m
            .body
            .bb()
            .expand([0.0, f64::from(-0.4f32), 0.0])
            .contract([0.001, 0.001, 0.001]);
        let (water, push) = super::liquids::water_movement(&mut self.level.overworld, &bb, true);
        if let Some(m) = self.minecarts.get_mut(&id) {
            // handleMaterialAcceleration adds nothing without a push (keeping a -0.0).
            if push != (0.0, 0.0, 0.0) {
                m.body.motion.0 += push.0;
                m.body.motion.1 += push.1;
                m.body.motion.2 += push.2;
            }
            m.body.in_water = water;
            if water {
                m.base.wet();
            }
        }
    }

    /// `EntityMinecart.applyEntityCollision`: the cart and `pusher` push each other
    /// apart (minecarts lined up share their motion).
    pub(super) fn minecart_collision(&mut self, id: i32, pusher: CartPusher) {
        let Some(m) = self.minecarts.get(&id) else {
            return;
        };
        let (cart_pos, cart_yaw, cart_motion) = (m.body.pos, m.base.yaw, m.body.motion);
        let other_pos = match pusher {
            CartPusher::Player(i) => {
                // A cart is not pushed by its own rider.
                if m.rider == Some(self.players[i].entity_id) {
                    return;
                }
                self.players[i].pos
            }
            CartPusher::ArmorStand(s) => match self.armor_stands.get(&s) {
                Some(s) => s.body.pos,
                None => return,
            },
            CartPusher::Minecart(o) => match self.minecarts.get(&o) {
                Some(o) => o.body.pos,
                None => return,
            },
        };
        // TODO(M5): moving rideable carts pick up mobs (and armor stands) they hit.
        let (mut dx, mut dz) = (other_pos.0 - cart_pos.0, other_pos.2 - cart_pos.2);
        let mut d = dx * dx + dz * dz;
        if d < f64::from(1.0e-4f32) {
            return;
        }
        d = f64::from(krol_java::math::sqrt_f64(d));
        dx /= d;
        dz /= d;
        let inv = (1.0 / d).min(1.0);
        dx *= inv;
        dz *= inv;
        dx *= f64::from(0.1f32);
        dz *= f64::from(0.1f32);
        dx *= 0.5;
        dz *= 0.5;
        match pusher {
            CartPusher::Minecart(o) => {
                let (ox, oz) = normalize(other_pos.0 - cart_pos.0, other_pos.2 - cart_pos.2);
                let yaw = cart_yaw * std::f32::consts::PI / 180.0;
                let (fx, fz) = normalize(
                    f64::from(krol_java::math::cos(yaw)),
                    f64::from(krol_java::math::sin(yaw)),
                );
                if (ox * fx + oz * fz).abs() < f64::from(LINED_UP) {
                    return;
                }
                let Some(other_motion) = self.minecarts.get(&o).map(|c| c.body.motion) else {
                    return;
                };
                let sx = (other_motion.0 + cart_motion.0) / 2.0;
                let sz = (other_motion.2 + cart_motion.2) / 2.0;
                let share = f64::from(0.2f32);
                if let Some(m) = self.minecarts.get_mut(&id) {
                    m.body.motion.0 = m.body.motion.0 * share + (sx - dx);
                    m.body.motion.2 = m.body.motion.2 * share + (sz - dz);
                    m.airborne = true;
                }
                if let Some(c) = self.minecarts.get_mut(&o) {
                    c.body.motion.0 = c.body.motion.0 * share + (sx + dx);
                    c.body.motion.2 = c.body.motion.2 * share + (sz + dz);
                    c.airborne = true;
                }
            }
            CartPusher::Player(i) => {
                if let Some(m) = self.minecarts.get_mut(&id) {
                    m.body.motion.0 -= dx;
                    m.body.motion.2 -= dz;
                    m.airborne = true;
                }
                let p = &mut self.players[i];
                p.motion.0 += dx / 4.0;
                p.motion.2 += dz / 4.0;
                p.airborne = true;
            }
            CartPusher::ArmorStand(s) => {
                if let Some(m) = self.minecarts.get_mut(&id) {
                    m.body.motion.0 -= dx;
                    m.body.motion.2 -= dz;
                    m.airborne = true;
                }
                if let Some(s) = self.armor_stands.get_mut(&s) {
                    s.body.motion.0 += dx / 4.0;
                    s.body.motion.2 += dz / 4.0;
                }
            }
        }
    }

    /// `EntityMinecart.attackEntityFrom`: the cart wobbles and takes 10 damage per
    /// point; a creative player or more than 40 damage breaks it (dropping a minecart
    /// unless a creative player broke an unnamed one). Hits always count.
    pub(super) fn attack_minecart(&mut self, id: i32, source: DamageSource, amount: f32) -> bool {
        let creative = source.attacker().is_some_and(|a| {
            self.players
                .iter()
                .any(|p| p.entity_id == a && p.gamemode == CREATIVE)
        });
        let Some(m) = self.minecarts.get_mut(&id) else {
            return false;
        };
        if m.dead {
            return true;
        }
        if m.base.invulnerable && source != DamageSource::OutOfWorld && !creative {
            return false;
        }
        m.set_rolling_direction(-m.rolling_direction);
        m.set_rolling_amplitude(WOBBLE);
        m.body.velocity_changed = true;
        m.set_damage(m.damage + amount * DAMAGE_PER_POINT);
        if creative || m.damage > BREAK_DAMAGE {
            // The rider gets off first.
            if m.rider.is_some() {
                self.dismount_rider_of(id);
            }
            let Some(m) = self.minecarts.get_mut(&id) else {
                return true;
            };
            m.dead = true;
            if !(creative && m.base.custom_name.is_none())
                && self.level.data.game_rule("doEntityDrops") != Some("false")
            {
                // killMinecart: entityDropItem at the cart's feet.
                let mut stack = ItemStack {
                    id: item::MINECART,
                    count: 1,
                    damage: 0,
                    nbt: None,
                };
                if let Some(name) = m.base.custom_name.clone() {
                    set_display_name(&mut stack, &name);
                }
                let pos = m.body.pos;
                self.spawn_item(pos, stack, DROP_PICKUP_DELAY);
            }
        }
        true
    }

    /// A player's attack on a minecart (`attackTargetEntityWithCurrentItem` on an
    /// entity that is not alive): no critical hits and no wear, but sharpness counts,
    /// knockback pushes it, and it tires the player.
    pub(super) fn player_attacks_minecart(&mut self, i: usize, id: i32) {
        let held = self.players[i].held().cloned();
        let mut damage =
            super::attributes::value(super::attributes::ATTACK_DAMAGE, &self.players[i].modifiers)
                as f32;
        let bonus = super::combat::enchantment_damage(held.as_ref());
        let mut knockback =
            super::item_data::enchantment_level(held.as_ref(), super::combat::KNOCKBACK);
        if self.players[i].sprinting {
            knockback += 1;
        }
        if damage <= 0.0 && bonus <= 0.0 {
            return;
        }
        damage += bonus;
        let attacker = self.players[i].entity_id;
        if !self.attack_minecart(id, DamageSource::Player(attacker), damage) {
            return;
        }
        if knockback > 0 {
            let yaw = self.players[i].rotation.0 * std::f32::consts::PI / 180.0;
            let k = knockback as f32 * 0.5;
            if let Some(m) = self.minecarts.get_mut(&id) {
                m.body.motion.0 += f64::from(-krol_java::math::sin(yaw) * k);
                m.body.motion.1 += 0.1;
                m.body.motion.2 += f64::from(krol_java::math::cos(yaw) * k);
                m.airborne = true;
            }
            let a = &mut self.players[i];
            a.motion.0 *= 0.6;
            a.motion.2 *= 0.6;
            self.set_sprinting(i, false);
        }
        if bonus > 0.0 {
            self.send_to_self_and_tracking(
                i,
                ClientboundPlay::Animation {
                    entity_id: id,
                    animation: super::combat::MAGIC_CRITICAL_EFFECT,
                },
            );
        }
        self.add_exhaustion(i, super::combat::ATTACK_EXHAUSTION);
    }

    /// `BlockRailDetector.findMinecarts`: whether a minecart is in the detector rail's
    /// box at `pos`.
    pub(super) fn minecart_in_detector(&self, pos: (i32, i32, i32)) -> bool {
        let inset = 0.2f32;
        let area = Aabb::new(
            [
                f64::from(pos.0 as f32 + inset),
                f64::from(pos.1),
                f64::from(pos.2 as f32 + inset),
            ],
            [
                f64::from((pos.0 + 1) as f32 - inset),
                f64::from((pos.1 + 1) as f32 - inset),
                f64::from((pos.2 + 1) as f32 - inset),
            ],
        );
        self.minecarts
            .values()
            .any(|m| !m.dead && m.body.bb().intersects(&area))
    }

    /// `writeToNBTOptional` of a minecart.
    pub(super) fn minecart_nbt(&self, m: &Minecart, uuid: u128, c: &mut Compound) {
        c.insert("id", Tag::String(RIDEABLE_ID.into()));
        m.base.write_entity(&m.body, uuid, c);
        m.write(c);
    }
}

/// What pushes a minecart in [`Game::minecart_collision`].
#[derive(Debug, Clone, Copy)]
pub enum CartPusher {
    /// A player, by index.
    Player(usize),
    /// An armor stand, by entity id.
    ArmorStand(i32),
    /// Another minecart, by entity id.
    Minecart(i32),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rail_ends_match_the_shapes() {
        // Ascending east: low end west, high end east one up.
        assert_eq!(
            ENDS[usize::from(rails::ASCENDING_EAST)],
            [[-1, -1, 0], [1, 0, 0]]
        );
        assert_eq!(
            ENDS[usize::from(rails::NORTH_EAST)],
            [[0, 0, -1], [1, 0, 0]]
        );
    }

    #[test]
    fn metadata_hides_the_custom_name() {
        let mut m = Minecart::new((0.5, 4.0625, 0.5));
        m.base.custom_name = Some("Cart".into());
        let md = m.metadata();
        let indices: Vec<u8> = md.iter().map(|e| e.index).collect();
        assert_eq!(indices, [0, 1, 17, 2, 18, 3, 19, 4, 20, 21, 22]);
        assert_eq!(md[3].value, MetadataValue::String(String::new()));
        assert_eq!(md[4].value, MetadataValue::Int(1));
        assert_eq!(md[9].value, MetadataValue::Int(6));
    }
}
