//! Bows and the arrows they shoot (vanilla: `ItemBow`, `EntityArrow`).
//!
//! Drawing a bow is an item use of up to an hour; letting go shoots an arrow whose
//! speed grows with how long the bow was drawn. An arrow flies like a throwable (a
//! block and an entity ray trace per tick, then a move without collision), but sticks
//! in the block it hits, where players can pick it up, and hurts what it hits by its
//! speed.
//!
//! TODO(M5): arrows pressing wooden buttons and lighting TNT (burning arrows),
//! catching fire in lava, entering portals, dispensers and skeletons shooting them,
//! hitting mobs (endermen dodge, arthropod enchantments, a shooter that is not a
//! player), the death message of an arrow without a shooter (the arrow's display name).

use std::collections::BTreeSet;

use krol_java::math::{ceil_f64, cos, floor_f64, sin, sqrt_f64};
use krol_nbt::{Compound, Tag};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack, Metadata, MetadataValue};

use super::Game;
use super::damage::DamageSource;
use super::explosion::Target;
use super::food::UseAction;
use super::ids::{block, item};
use super::physics::Body;
use super::saved_entities::EntityBase;

/// Arrows are half a block wide and high (`setSize(0.5F, 0.5F)`).
pub const SIZE: f32 = 0.5;
/// The saved entity id (`EntityList` name), also its `type` for selectors.
pub const SAVED_ID: &str = "Arrow";
/// The en_US `entity.Arrow.name`.
pub const NAME: &str = "Arrow";
/// `S0EPacketSpawnObject` type.
pub const OBJECT_TYPE: i8 = 60;
/// How long a bow can be drawn (`getMaxItemUseDuration`).
pub const BOW_USE_DURATION: i32 = 72000;
/// The shot starts this far below the eyes...
const START_DROP: f32 = 0.1;
/// ...and this far to the side of them.
const START_SIDE: f32 = 0.16;
/// The spread of the heading per axis, times the inaccuracy (`0.0075F`).
const SPREAD: f32 = 0.0075;
/// `setThrowableHeading`'s inaccuracy argument for a bow shot.
const INACCURACY: f32 = 1.0;
/// The bow's power (0 to 1) times this is the arrow's velocity argument, which
/// `setThrowableHeading` scales by `SPEED_FACTOR`.
const POWER_VELOCITY: f32 = 2.0;
const SPEED_FACTOR: f32 = 1.5;
/// A bow drawn for less than this power shoots nothing.
const MIN_POWER: f64 = 0.1;
/// Ticks of drawing for full power, before the curve (`f / 20.0F`).
const DRAW_TICKS: f32 = 20.0;
/// Damage per block of speed (`damage = 2.0`).
const BASE_DAMAGE: f64 = 2.0;
/// Air drag per tick, the drag in water, and gravity.
const AIR_DRAG: f32 = 0.99;
const WATER_DRAG: f32 = 0.6;
const GRAVITY: f32 = 0.05;
/// How much of the turn towards the flight direction is made per tick.
const TURN_RATE: f32 = 0.2;
/// The shooter is ignored by the entity ray trace for this many ticks in the air.
const SHOOTER_GRACE: i32 = 5;
/// Collidable entities' boxes grow by this much for the hit test (`0.3F`).
const HIT_MARGIN: f32 = 0.3;
/// An arrow stuck in a block disappears after this many ticks.
const GROUND_LIFETIME: i32 = 1200;
/// A stuck arrow shakes (and cannot be picked up) for this many ticks.
const STUCK_SHAKE: i32 = 7;
/// A stuck arrow sits this far back from the hit along its flight (`0.05F`).
const STUCK_BACKOFF: f32 = 0.05;
/// The bounce off an entity it could not hurt (`-0.1F` of the motion).
const BOUNCE: f32 = -0.1;
/// Punch pushes the victim by this much per level along the flight (`0.6F`), and up.
const PUNCH_PUSH: f32 = 0.6;
const PUNCH_LIFT: f64 = 0.1;
/// A burning arrow sets what it hurts on fire for this many seconds.
const VICTIM_FIRE_SECONDS: i32 = 5;
/// A flame bow's arrow burns for this many seconds (`setFire(100)`).
const FLAME_SECONDS: i32 = 100;
/// Fire hurts (`attackEntityFrom(onFire)`) when its counter is a multiple of this.
const FIRE_HURT_PERIOD: i16 = 20;
/// Arrows a player can pick up (`canBePickedUp`): only creative players, for arrows
/// from creative players and infinity bows.
const PICKUP_ALLOWED: i8 = 1;
const PICKUP_CREATIVE_ONLY: i8 = 2;
/// `S2BPacketChangeGameState` reason 6: the shooter hears its arrow hit a player.
const ARROW_HIT_PLAYER: u8 = 6;
/// Sounds: shooting, hitting, being picked up.
const SHOOT_SOUND: &str = "random.bow";
const HIT_SOUND: &str = "random.bowhit";
const PICKUP_SOUND: &str = "random.pop";
const PICKUP_VOLUME: f32 = 0.2;
/// Metadata entry 16: bit 0 is set for a critical (fully drawn) arrow.
const CRITICAL_INDEX: u8 = 16;
/// Metadata entry 0: the base flags, bit 0 burning.
const FLAGS_INDEX: u8 = 0;
/// Every metadata entry of an arrow.
const ALL_METADATA: [u8; 6] = [0, 1, 2, 3, 4, CRITICAL_INDEX];
/// The creative game mode.
const CREATIVE: i32 = 1;
/// Player arrows stay stuck in a player for `20 * (30 - count)` updates each
/// (`arrowHitTimer`).
const ARROW_STUCK_BASE: i32 = 30;
const ARROW_STUCK_TICKS: i32 = 20;

/// One arrow (`EntityArrow`).
#[derive(Debug, Clone)]
pub struct Arrow {
    /// Position and motion (it never collides, so only these and the box matter).
    pub body: Body,
    /// The base entity state (rotation follows the flight direction).
    pub base: EntityBase,
    /// The block it is stuck in (`xTile`, `yTile`, `zTile`).
    pub tile: (i32, i32, i32),
    /// That block's id when it stuck (`inTile`); `None` saves as `""`.
    pub in_tile: Option<u16>,
    /// That block's metadata when it stuck (`inData`).
    pub in_data: u8,
    /// `inGround`.
    pub in_ground: bool,
    /// `canBePickedUp`: 0 never, 1 by anyone (giving an arrow), 2 by creative players.
    pub pickup: i8,
    /// `arrowShake` (saved, counts down).
    pub shake: i32,
    /// The entity id of the player who shot it (`shootingEntity`, not saved).
    pub shooter: Option<i32>,
    /// `ticksInGround` (saved as `life`).
    pub ticks_in_ground: i32,
    /// `ticksInAir`.
    pub ticks_in_air: i32,
    /// Damage per block of speed (`damage`).
    pub damage: f64,
    /// Punch level (`knockbackStrength`, not saved).
    pub knockback: i32,
    /// Metadata 16 bit 0 (`getIsCritical`, not saved).
    pub critical: bool,
    /// The burning bit of the metadata flags (`getFlag(0)`).
    pub burning: bool,
    /// Metadata entries not yet sent as changes (`WatchableObject.watched`): all of them
    /// at first, since the spawn packet's full list does not count.
    pub dirty_metadata: BTreeSet<u8>,
    /// An entry changed since the tracker last sent changes (`objectChanged`): only
    /// then are the watched entries sent.
    pub metadata_changed: bool,
    /// `firstUpdate`: no splash on the first tick.
    pub first_update: bool,
    /// Removed from the world (`isDead`).
    pub dead: bool,
    /// The chunk the entity was last added to.
    pub chunk: (i32, i32),
}

impl Arrow {
    /// Vanilla's metadata for an arrow: the base entries (with the burning flag) and
    /// the critical byte, in the data watcher's `HashMap` order (0, 16, 1, 2, 3, 4).
    pub fn metadata(&self) -> Vec<Metadata> {
        let mut metadata = self.base.metadata();
        if let Some(flags) = metadata.iter_mut().find(|m| m.index == FLAGS_INDEX) {
            flags.value = MetadataValue::Byte(i8::from(self.burning));
        }
        metadata.push(Metadata {
            index: CRITICAL_INDEX,
            value: MetadataValue::Byte(i8::from(self.critical)),
        });
        let indices: Vec<i32> = metadata.iter().map(|m| i32::from(m.index)).collect();
        krol_java::java_hash_order(&indices)
            .into_iter()
            .map(|k| metadata[k].clone())
            .collect()
    }

    /// `DataWatcher.updateObject` of a changed entry.
    fn metadata_update(&mut self, index: u8) {
        self.dirty_metadata.insert(index);
        self.metadata_changed = true;
    }

    /// `EntityArrow.readEntityFromNBT` for an arrow loaded with its chunk.
    pub fn load(body: Body, base: EntityBase, c: &Compound) -> Self {
        let p = body.pos;
        let numeric = |key: &str| c.get(key).and_then(Tag::as_f64).is_some();
        let pickup = if numeric("pickup") {
            c.get_i8("pickup")
        } else if numeric("player") {
            i8::from(c.get_bool("player"))
        } else {
            0
        };
        Self {
            body,
            base,
            tile: (
                i32::from(c.get_i16("xTile")),
                i32::from(c.get_i16("yTile")),
                i32::from(c.get_i16("zTile")),
            ),
            in_tile: super::throwable::in_tile_from_nbt(c),
            in_data: c.get_i8("inData") as u8,
            in_ground: c.get_i8("inGround") == 1,
            pickup,
            shake: i32::from(c.get_i8("shake") as u8),
            shooter: None,
            ticks_in_ground: i32::from(c.get_i16("life")),
            ticks_in_air: 0,
            damage: if numeric("damage") {
                c.get_f64("damage")
            } else {
                BASE_DAMAGE
            },
            knockback: 0,
            critical: false,
            burning: false,
            dirty_metadata: ALL_METADATA.into_iter().collect(),
            metadata_changed: false,
            first_update: true,
            dead: false,
            chunk: (floor_f64(p.0) >> 4, floor_f64(p.2) >> 4),
        }
    }

    /// `EntityArrow.writeEntityToNBT`.
    pub fn write(&self, c: &mut Compound) {
        c.insert("xTile", Tag::Short(self.tile.0 as i16));
        c.insert("yTile", Tag::Short(self.tile.1 as i16));
        c.insert("zTile", Tag::Short(self.tile.2 as i16));
        c.insert("life", Tag::Short(self.ticks_in_ground as i16));
        let tile_name = self
            .in_tile
            .and_then(|id| krol_world::items::name_for_id(id as i16))
            .unwrap_or_default();
        c.insert("inTile", Tag::String(tile_name.to_owned()));
        c.insert("inData", Tag::Byte(self.in_data as i8));
        c.insert("shake", Tag::Byte(self.shake as i8));
        c.insert("inGround", Tag::Byte(i8::from(self.in_ground)));
        c.insert("pickup", Tag::Byte(self.pickup));
        c.insert("damage", Tag::Double(self.damage));
    }

    /// `setIsCritical`: the metadata changes only if the bit does.
    fn set_critical(&mut self, critical: bool) {
        if self.critical != critical {
            self.critical = critical;
            self.metadata_update(CRITICAL_INDEX);
        }
    }
}

/// The bow's power after drawing it for `ticks` (`ItemBow.onPlayerStoppedUsing`):
/// `None` if too weak to shoot, else up to 1.
fn bow_power(ticks: i32) -> Option<f32> {
    let f = ticks as f32 / DRAW_TICKS;
    let f = (f * f + f * 2.0) / 3.0;
    if f64::from(f) < MIN_POWER {
        return None;
    }
    Some(f.min(1.0))
}

/// What an arrow hit this tick.
enum Hit {
    Block(super::raytrace::Hit),
    Entity(i32, Target),
}

impl Game {
    /// `ItemBow.onItemRightClick`: creative players and players carrying arrows start
    /// drawing the bow.
    pub(super) fn start_drawing_bow(&mut self, i: usize, held: &ItemStack) {
        let p = &self.players[i];
        if p.gamemode == CREATIVE || first_arrow_slot(&p.inventory).is_some() {
            self.start_using(i, held, UseAction::Bow, BOW_USE_DURATION);
        }
    }

    /// `EntityPlayer.stopUsingItem` (letting go of the use key): a drawn bow shoots
    /// (`onPlayerStoppedUsing`), then the use ends.
    pub(super) fn stop_using_item(&mut self, i: usize) {
        if let Some(u) = self.players[i].using.clone()
            && u.action == UseAction::Bow
        {
            self.release_bow(i, u.slot, u.count);
        }
        self.clear_item_in_use(i);
    }

    /// `ItemBow.onPlayerStoppedUsing` with `left` updates of the use remaining: shoots
    /// an arrow (critical at full power; power, punch and flame applied), wears the bow,
    /// plays the shot and uses up an arrow unless the player is creative or the bow has
    /// infinity.
    fn release_bow(&mut self, i: usize, slot: usize, left: i32) {
        use super::item_data::{FLAME, INFINITY, POWER, PUNCH, enchantment_level};
        let p = &self.players[i];
        let Some(bow) = p.inventory[slot].clone().filter(|s| s.id == item::BOW) else {
            return;
        };
        let infinite = p.gamemode == CREATIVE || enchantment_level(Some(&bow), INFINITY) > 0;
        let arrow_slot = first_arrow_slot(&p.inventory);
        if !infinite && arrow_slot.is_none() {
            return;
        }
        let Some(power) = bow_power(BOW_USE_DURATION - left) else {
            return;
        };
        let (id, mut arrow) = self.new_shot(i, power * POWER_VELOCITY);
        if power == 1.0 {
            arrow.set_critical(true);
        }
        let level = enchantment_level(Some(&bow), POWER);
        if level > 0 {
            arrow.damage += f64::from(level) * 0.5 + 0.5;
        }
        let level = enchantment_level(Some(&bow), PUNCH);
        if level > 0 {
            arrow.knockback = level;
        }
        if enchantment_level(Some(&bow), FLAME) > 0 {
            arrow.base.fire = (FLAME_SECONDS * 20) as i16;
        }
        self.damage_slot_item(i, slot, 1);
        // ItemStack.damageItem: a broken bow is removed from the hand
        // (destroyCurrentEquippedItem).
        if self.players[i].inventory[slot]
            .as_ref()
            .is_some_and(|s| s.count <= 0)
        {
            let current = 36 + self.players[i].selected;
            self.players[i].inventory[current] = None;
        }
        // Item.itemRand (unseeded in vanilla).
        let pitch = 1.0 / (self.item_rand.next_float() * 0.4 + 1.2) + power * 0.5;
        let pos = self.players[i].pos;
        self.play_sound_near_except(None, SHOOT_SOUND, pos, 1.0, pitch);
        if infinite {
            arrow.pickup = PICKUP_CREATIVE_ONLY;
        } else if let Some(s) = arrow_slot {
            // InventoryPlayer.consumeInventoryItem.
            let inv = &mut self.players[i].inventory;
            if let Some(stack) = inv[s].as_mut() {
                stack.count -= 1;
                if stack.count <= 0 {
                    inv[s] = None;
                }
            }
        }
        if let Some(stat) = super::digging::use_stat(item::BOW) {
            self.add_stat(i, &stat, 1);
        }
        self.arrows.insert(id, arrow);
        self.spawn_entity_in_world(id);
    }

    /// `new EntityArrow(world, shooter, velocity)`: from the player's eyes, a little to
    /// the side and below, flying where the player looks with a small random spread.
    fn new_shot(&mut self, i: usize, velocity: f32) -> (i32, Arrow) {
        let id = self.next_entity_id();
        let p = &self.players[i];
        let (yaw, pitch) = p.rotation;
        // Vanilla's operation order: degrees / 180 * PI, in floats.
        let radians = |degrees: f32| degrees / 180.0 * std::f32::consts::PI;
        let (yaw_rad, pitch_rad) = (radians(yaw), radians(pitch));
        let pos = (
            p.pos.0 - f64::from(cos(yaw_rad) * START_SIDE),
            p.pos.1 + f64::from(p.eye_height()) - f64::from(START_DROP),
            p.pos.2 - f64::from(sin(yaw_rad) * START_SIDE),
        );
        let aim = (
            f64::from(-sin(yaw_rad) * cos(pitch_rad)),
            f64::from(-sin(pitch_rad)),
            f64::from(cos(yaw_rad) * cos(pitch_rad)),
        );
        let shooter = p.entity_id;
        // EntityArrow.setThrowableHeading: normalize, add the spread with a random sign
        // per axis, scale to the speed (the entity's own random, unseeded in vanilla).
        let length = f64::from(sqrt_f64(aim.0 * aim.0 + aim.1 * aim.1 + aim.2 * aim.2));
        let speed = f64::from(velocity * SPEED_FACTOR);
        let spread = f64::from(SPREAD) * f64::from(INACCURACY);
        let r = &mut self.entity_rand;
        let mut axis = |v: f64| {
            let gaussian = r.next_gaussian();
            let sign = if r.next_boolean() { -1.0 } else { 1.0 };
            (v / length + gaussian * sign * spread) * speed
        };
        let motion = (axis(aim.0), axis(aim.1), axis(aim.2));
        let (yaw, pitch) = super::throwable::heading(motion);
        let mut body = Body::player(pos, false);
        body.width = SIZE;
        body.height = SIZE;
        body.step_height = 0.0;
        body.motion = motion;
        let mut base = EntityBase::facing(yaw);
        base.pitch = pitch;
        let arrow = Arrow {
            body,
            base,
            tile: (-1, -1, -1),
            in_tile: None,
            in_data: 0,
            in_ground: false,
            pickup: PICKUP_ALLOWED,
            shake: 0,
            shooter: Some(shooter),
            ticks_in_ground: 0,
            ticks_in_air: 0,
            damage: BASE_DAMAGE,
            knockback: 0,
            critical: false,
            burning: false,
            dirty_metadata: ALL_METADATA.into_iter().collect(),
            metadata_changed: false,
            first_update: true,
            dead: false,
            chunk: (floor_f64(pos.0) >> 4, floor_f64(pos.2) >> 4),
        };
        (id, arrow)
    }

    /// One arrow's turn in the entity loop.
    pub(super) fn update_arrow(&mut self, id: i32) {
        if self.arrows.get(&id).is_some_and(|a| !a.dead) {
            self.tick_arrow(id);
            self.relocate_entity(id);
        }
        if self.arrows.get(&id).is_some_and(|a| a.dead) {
            self.arrows.remove(&id);
            self.remove_entity_from_chunk(id);
            self.untrack_entity(id);
        }
    }

    /// `EntityArrow.onUpdate`.
    fn tick_arrow(&mut self, id: i32) {
        let Some(mut a) = self.arrows.remove(&id) else {
            return;
        };
        // Entity.onEntityUpdate: water, fire, the void, the burning flag.
        if self.projectile_entity_update(&mut a.body, &mut a.base, &mut a.first_update, SIZE) {
            a.dead = true;
        }
        if a.base.fire > 0 {
            // attackEntityFrom(onFire, 1): an arrow is only setBeenAttacked.
            if a.base.fire % FIRE_HURT_PERIOD == 0 && !a.base.invulnerable {
                a.body.velocity_changed = true;
            }
            a.base.fire -= 1;
        }
        // TODO(M5): lava sets it on fire (setOnFireFromLava).
        let burning = a.base.fire > 0;
        if burning != a.burning {
            a.burning = burning;
            a.metadata_update(FLAGS_INDEX);
        }
        // A loaded arrow with no rotation turns along its motion.
        if a.base.yaw == 0.0 && a.base.pitch == 0.0 {
            (a.base.yaw, a.base.pitch) = super::throwable::heading(a.body.motion);
        }
        // Stuck when inside the collision box of the block it hit.
        let (tx, ty, tz) = a.tile;
        let state = self.level.overworld.block(tx, ty, tz);
        if state >> 4 != block::AIR {
            // TODO(M5): vanilla tests the block's single collision box
            // (getCollisionBoundingBox); stairs use their full cube there.
            let p = a.body.pos;
            let w = &mut self.level.overworld;
            let mut boxes = Vec::new();
            let everything = super::physics::Aabb::new([-3.0e7; 3], [3.0e7; 3]);
            let mut neighbors = |q: (i32, i32, i32)| w.block(q.0, q.1, q.2);
            super::shapes::add_block_boxes(state, a.tile, &everything, &mut boxes, &mut neighbors);
            if boxes.iter().any(|b| {
                (0..3).all(|k| {
                    let v = [p.0, p.1, p.2][k];
                    v > b.min[k] && v < b.max[k]
                })
            }) {
                a.in_ground = true;
            }
        }
        if a.shake > 0 {
            a.shake -= 1;
        }
        if a.in_ground {
            if Some(state >> 4) == a.in_tile && (state & 15) as u8 == a.in_data {
                a.ticks_in_ground += 1;
                if a.ticks_in_ground >= GROUND_LIFETIME {
                    a.dead = true;
                }
            } else {
                // The block changed: the arrow falls out.
                a.in_ground = false;
                let r = &mut self.entity_rand;
                a.body.motion.0 *= f64::from(r.next_float() * 0.2);
                a.body.motion.1 *= f64::from(r.next_float() * 0.2);
                a.body.motion.2 *= f64::from(r.next_float() * 0.2);
                a.ticks_in_ground = 0;
                a.ticks_in_air = 0;
            }
            self.arrows.insert(id, a);
            return;
        }
        a.ticks_in_air += 1;
        let mut prev_rotation = (a.base.yaw, a.base.pitch);
        let hit = self.arrow_hit(id, &a);
        match hit {
            Some(Hit::Entity(other, target)) => {
                self.arrows.insert(id, a);
                let bounced = !self.arrow_hits_entity(id, other, target);
                let Some(again) = self.arrows.remove(&id) else {
                    return;
                };
                a = again;
                if bounced {
                    let m = &mut a.body.motion;
                    m.0 *= f64::from(BOUNCE);
                    m.1 *= f64::from(BOUNCE);
                    m.2 *= f64::from(BOUNCE);
                    a.base.yaw += 180.0;
                    prev_rotation.0 += 180.0;
                    a.ticks_in_air = 0;
                }
            }
            Some(Hit::Block(h)) => {
                a.tile = h.pos;
                let state = self.level.overworld.block(h.pos.0, h.pos.1, h.pos.2);
                a.in_tile = Some(state >> 4);
                a.in_data = (state & 15) as u8;
                let p = a.body.pos;
                // Vanilla goes through floats here.
                let m = (
                    f64::from((h.vec.0 - p.0) as f32),
                    f64::from((h.vec.1 - p.1) as f32),
                    f64::from((h.vec.2 - p.2) as f32),
                );
                a.body.motion = m;
                let length = f64::from(sqrt_f64(m.0 * m.0 + m.1 * m.1 + m.2 * m.2));
                let back = f64::from(STUCK_BACKOFF);
                a.body.pos = (
                    p.0 - m.0 / length * back,
                    p.1 - m.1 / length * back,
                    p.2 - m.2 / length * back,
                );
                let pitch = 1.2 / (self.entity_rand.next_float() * 0.2 + 0.9);
                if !a.base.silent {
                    self.play_sound_near_except(None, HIT_SOUND, a.body.pos, 1.0, pitch);
                }
                a.in_ground = true;
                a.shake = STUCK_SHAKE;
                a.set_critical(false);
                // TODO(M5): onEntityCollidedWithBlock of the block hit (wooden buttons,
                // TNT lit by a burning arrow).
            }
            None => {}
        }
        // The critical trail particles are client-side. The arrow moves without
        // collision, turns a fifth of the way towards its flight and slows down.
        let m = a.body.motion;
        a.body.pos = (a.body.pos.0 + m.0, a.body.pos.1 + m.1, a.body.pos.2 + m.2);
        let (yaw, pitch) = super::throwable::heading(m);
        let (mut prev_yaw, mut prev_pitch) = prev_rotation;
        while pitch - prev_pitch < -180.0 {
            prev_pitch -= 360.0;
        }
        while pitch - prev_pitch >= 180.0 {
            prev_pitch += 360.0;
        }
        while yaw - prev_yaw < -180.0 {
            prev_yaw -= 360.0;
        }
        while yaw - prev_yaw >= 180.0 {
            prev_yaw += 360.0;
        }
        a.base.pitch = prev_pitch + (pitch - prev_pitch) * TURN_RATE;
        a.base.yaw = prev_yaw + (yaw - prev_yaw) * TURN_RATE;
        // The bubble particles in water are client-side.
        let drag = f64::from(if a.body.in_water {
            WATER_DRAG
        } else {
            AIR_DRAG
        });
        if self.projectile_wet(&a.body) {
            a.base.fire = 0;
        }
        a.body.motion.0 *= drag;
        a.body.motion.1 *= drag;
        a.body.motion.2 *= drag;
        a.body.motion.1 -= f64::from(GRAVITY);
        let p = a.body.pos;
        a.chunk = (floor_f64(p.0) >> 4, floor_f64(p.2) >> 4);
        // doBlockCollisions, with the arrow in the world (pressure plates count it).
        let bb = a.body.bb();
        self.arrows.insert(id, a);
        self.block_collisions(&bb);
    }

    /// The arrow's ray traces: the first block on the way, and the closest collidable
    /// entity before it (not the shooter in its first ticks). A player who cannot be
    /// hurt by it cancels the whole hit.
    fn arrow_hit(&mut self, id: i32, a: &Arrow) -> Option<Hit> {
        let (start, m) = (a.body.pos, a.body.motion);
        let mut end = (start.0 + m.0, start.1 + m.1, start.2 + m.2);
        let block_hit =
            super::raytrace::ray_trace_blocks(&mut self.level.overworld, start, end, false, true);
        if let Some(h) = &block_hit {
            end = h.vec;
        }
        let area = a
            .body
            .bb()
            .add_coord([m.0, m.1, m.2])
            .expand([1.0, 1.0, 1.0]);
        let margin = f64::from(HIT_MARGIN);
        let mut closest: Option<(f64, i32, Target)> = None;
        for (other, target) in self.entities_within(&area, Some(id)) {
            if !self.can_be_collided_with(target)
                || (Some(other) == a.shooter && a.ticks_in_air < SHOOTER_GRACE)
            {
                continue;
            }
            let Some(bb) = self.target_box(target) else {
                continue;
            };
            let bb = bb.expand([margin, margin, margin]);
            if let Some(v) = super::raytrace::intercept(&bb, start, end) {
                let d = (v.0 - start.0).powi(2) + (v.1 - start.1).powi(2) + (v.2 - start.2).powi(2);
                if closest.is_none_or(|(best, _, _)| d < best || best == 0.0) {
                    closest = Some((d, other, target));
                }
            }
        }
        let hit = match closest {
            Some((_, other, target)) => Some(Hit::Entity(other, target)),
            None => block_hit.map(Hit::Block),
        };
        // Vanilla quirk: a player the arrow may not hurt (creative, or protected from
        // the shooter by PvP or friendly fire) cancels the block hit too, so the arrow
        // flies on this tick.
        if let Some(Hit::Entity(_, Target::Player(t))) = hit {
            let p = &self.players[t];
            let disabled = matches!(p.gamemode, 1 | 3);
            let shooter_is_player = a
                .shooter
                .is_some_and(|s| self.players.iter().any(|q| q.entity_id == s));
            let protected =
                shooter_is_player && a.shooter.is_some_and(|s| !self.can_be_attacked_by(t, s));
            if disabled || protected {
                return None;
            }
        }
        hit
    }

    /// The arrow hits an entity: damage by its speed (more if critical), fire if it
    /// burns. On a hit that hurts, a player victim counts the arrow, punch pushes it,
    /// thorns answer, the shooter hears it, and the arrow is gone. Returns whether the
    /// victim was hurt (else the arrow bounces off).
    fn arrow_hits_entity(&mut self, id: i32, other: i32, target: Target) -> bool {
        let Some(a) = self.arrows.get(&id) else {
            return false;
        };
        let m = a.body.motion;
        let speed = sqrt_f64(m.0 * m.0 + m.1 * m.1 + m.2 * m.2);
        let mut damage = ceil_f64(f64::from(speed) * a.damage);
        let (shooter, burning, knockback) = (a.shooter, a.base.fire > 0, a.knockback);
        if a.critical {
            damage += self.entity_rand.next_int_bounded(damage / 2 + 2);
        }
        let source = DamageSource::Arrow { arrow: id, shooter };
        if burning && let Target::Player(t) = target {
            self.ignite_player(t, VICTIM_FIRE_SECONDS);
        }
        let hurt = match target {
            Target::Player(t) => self.attack_player(t, source, damage as f32),
            other_target => self.attack_target(other_target, source, damage as f32),
        };
        if !hurt {
            return false;
        }
        if let Target::Player(t) = target {
            let p = &mut self.players[t];
            p.arrow_count = p.arrow_count.wrapping_add(1);
            p.dirty_metadata.insert(super::tracker::ARROW_COUNT_INDEX);
            if knockback > 0 {
                let horizontal = sqrt_f64(m.0 * m.0 + m.2 * m.2);
                if horizontal > 0.0 {
                    let push = |v: f64| {
                        v * f64::from(knockback) * f64::from(PUNCH_PUSH) / f64::from(horizontal)
                    };
                    // Entity.addVelocity.
                    p.motion.0 += push(m.0);
                    p.motion.1 += PUNCH_LIFT;
                    p.motion.2 += push(m.2);
                    p.airborne = true;
                }
            }
            if let Some(s) =
                shooter.and_then(|s| self.players.iter().position(|q| q.entity_id == s))
            {
                self.apply_thorns(t, s);
                if other != self.players[s].entity_id {
                    self.players[s].send(ClientboundPlay::ChangeGameState {
                        reason: ARROW_HIT_PLAYER,
                        value: 0.0,
                    });
                }
            }
        }
        let Some(a) = self.arrows.get(&id) else {
            return true;
        };
        let (pos, silent) = (a.body.pos, a.base.silent);
        let pitch = 1.2 / (self.entity_rand.next_float() * 0.2 + 0.9);
        if !silent {
            self.play_sound_near_except(None, HIT_SOUND, pos, 1.0, pitch);
        }
        if let Some(a) = self.arrows.get_mut(&id) {
            a.dead = true;
        }
        true
    }

    /// `EntityArrow.onCollideWithPlayer`: a stuck arrow that stopped shaking is picked
    /// up (as an arrow item if it allows anyone; for nothing by creative players if it
    /// came from them or an infinity bow).
    pub(super) fn collect_arrow(&mut self, i: usize, id: i32) {
        let Some(a) = self.arrows.get(&id) else {
            return;
        };
        if a.dead || !a.in_ground || a.shake > 0 {
            return;
        }
        let creative = self.players[i].is_creative();
        let mut taken =
            a.pickup == PICKUP_ALLOWED || (a.pickup == PICKUP_CREATIVE_ONLY && creative);
        if a.pickup == PICKUP_ALLOWED {
            let mut stack = ItemStack {
                id: item::ARROW,
                count: 1,
                damage: 0,
                nbt: None,
            };
            if !super::item_entity::add_to_inventory(
                &mut self.players[i].inventory,
                &mut stack,
                creative,
            ) {
                taken = false;
            }
        }
        if !taken {
            return;
        }
        let (pos, silent) = (a.body.pos, a.base.silent);
        let r = &mut self.entity_rand;
        let pitch = ((r.next_float() - r.next_float()) * 0.7 + 1.0) * 2.0;
        if !silent {
            self.play_sound_near_except(None, PICKUP_SOUND, pos, PICKUP_VOLUME, pitch);
        }
        let collector = self.players[i].entity_id;
        self.send_to_tracking(
            id,
            &ClientboundPlay::CollectItem {
                collected: id,
                collector,
            },
        );
        if let Some(a) = self.arrows.get_mut(&id) {
            a.dead = true;
        }
    }

    /// The arrows-in-body countdown of `EntityLivingBase.onUpdate` for a player: each
    /// arrow stays `20 * (30 - count)` updates.
    pub(super) fn stuck_arrows_update(&mut self, i: usize) {
        let p = &mut self.players[i];
        if p.arrow_count > 0 {
            if p.arrow_hit_timer <= 0 {
                p.arrow_hit_timer =
                    ARROW_STUCK_TICKS * (ARROW_STUCK_BASE - i32::from(p.arrow_count));
            }
            p.arrow_hit_timer -= 1;
            if p.arrow_hit_timer <= 0 {
                p.arrow_count -= 1;
                p.dirty_metadata.insert(super::tracker::ARROW_COUNT_INDEX);
            }
        }
    }
}

/// The window slot of the first arrow stack in the main inventory, hotbar first
/// (`InventoryPlayer.hasItem` / `consumeInventoryItem`).
fn first_arrow_slot(window: &[Option<ItemStack>]) -> Option<usize> {
    (0..36).map(|k| if k < 9 { 36 + k } else { k }).find(|&s| {
        window
            .get(s)
            .and_then(Option::as_ref)
            .is_some_and(|s| s.id == item::ARROW)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bow_power_follows_vanilla_curve() {
        // Under a tenth of the power shoots nothing: 2 ticks give 0.0367.
        assert_eq!(bow_power(2), None);
        assert!(bow_power(3).is_some());
        // Full power after 20 ticks of drawing, capped beyond.
        assert_eq!(bow_power(20), Some(1.0));
        assert_eq!(bow_power(100), Some(1.0));
        let half = bow_power(10).unwrap_or_default();
        assert!((half - 1.25 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn arrows_save_like_vanilla() {
        let mut c = Compound::new();
        c.insert("inTile", Tag::String("minecraft:stone".into()));
        c.insert("inGround", Tag::Byte(1));
        c.insert("player", Tag::Byte(1));
        c.insert("life", Tag::Short(40));
        let a = Arrow::load(
            Body::player((0.0, 70.0, 0.0), false),
            EntityBase::default(),
            &c,
        );
        // Old saves name the pickup rule `player`; the damage defaults to 2.
        assert_eq!(a.pickup, 1);
        assert_eq!(a.damage, 2.0);
        assert_eq!(a.in_tile, Some(1));
        assert_eq!(a.ticks_in_ground, 40);
        let mut out = Compound::new();
        a.write(&mut out);
        assert_eq!(out.get_str("inTile"), "minecraft:stone");
        assert_eq!(out.get_i8("pickup"), 1);
        assert_eq!(out.get_f64("damage"), 2.0);
        assert_eq!(out.get_i16("life"), 40);
    }

    #[test]
    fn arrows_are_found_hotbar_first() {
        let mut window = vec![None; 45];
        let arrow = |count| {
            Some(ItemStack {
                id: item::ARROW,
                count,
                damage: 0,
                nbt: None,
            })
        };
        window[9] = arrow(5);
        assert_eq!(first_arrow_slot(&window), Some(9));
        window[40] = arrow(1);
        assert_eq!(first_arrow_slot(&window), Some(40));
    }
}
