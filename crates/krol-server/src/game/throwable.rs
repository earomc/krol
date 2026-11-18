//! Thrown projectiles (vanilla: `EntityThrowable` with `EntitySnowball`, `EntityEgg`,
//! `EntityEnderPearl`, `EntityExpBottle` and `EntityPotion`, thrown by `ItemSnowball`,
//! `ItemEgg`, `ItemEnderPearl`, `ItemExpBottle` and `ItemPotion`).
//!
//! A throwable flies in a straight step per tick: it ray traces the blocks and the
//! collidable entities between its position and the next, reacts to the closest hit
//! (`onImpact`), then moves without any collision and slows down by drag and gravity.
//! None of these entity kinds survive their impact.
//!
//! TODO(M5): eggs hatching chickens and ender pearls spawning endermites (both roll
//! their chance already; the mobs are missing), splash potions affecting mobs,
//! throwables entering nether portals, catching fire in lava, dispensers firing them.

use krol_java::math::{atan2, cos, floor_f64, sin, sqrt_f64};
use krol_nbt::{Compound, Tag, TagType};
use krol_protocol::packets::play::{ItemStack, Metadata};

use super::Game;
use super::damage::DamageSource;
use super::explosion::Target;
use super::ids::{block, item};
use super::physics::Body;
use super::saved_entities::EntityBase;

/// Throwables are 0.25 blocks wide and high (`setSize(0.25F, 0.25F)`).
pub const SIZE: f32 = 0.25;
/// The throw starts this far below the eyes (`posY -= 0.1F`)...
const START_DROP: f32 = 0.1;
/// ...and this far to the side of them.
const START_SIDE: f32 = 0.16;
/// The speed of the aim vector before `setThrowableHeading` scales it (`0.4F`).
const AIM_SPEED: f32 = 0.4;
/// The spread of the heading per axis, times the inaccuracy (`0.0075F`).
const SPREAD: f32 = 0.0075;
/// `setThrowableHeading`'s inaccuracy argument from the thrower constructor.
const HEADING_INACCURACY: f32 = 1.0;
/// Air drag per tick, and the drag in water.
const AIR_DRAG: f32 = 0.99;
const WATER_DRAG: f32 = 0.8;
/// How much of the turn towards the flight direction is made per tick.
const TURN_RATE: f32 = 0.2;
/// The thrower is ignored by the entity ray trace for this many ticks in the air.
const THROWER_GRACE: i32 = 5;
/// Collidable entities' boxes grow by this much for the hit test (`0.3F`).
const HIT_MARGIN: f32 = 0.3;
/// A throwable stuck in a block disappears after this many ticks.
const GROUND_LIFETIME: i32 = 1200;
/// Entities below this height are killed (`Entity.onEntityUpdate`).
const VOID_Y: f64 = -64.0;
/// The throw sound (`random.bow` at volume 0.5, pitch `0.4 / (rand * 0.4 + 0.8)`).
const THROW_SOUND: &str = "random.bow";
const THROW_VOLUME: f32 = 0.5;
/// The splash of an entity falling into water (`getSplashSound`).
const SPLASH_SOUND: &str = "game.neutral.swim.splash";
/// The client effect of a breaking bottle (`playAuxSFX(2002, ...)`), which also shows
/// the potion color given as data (0 for experience bottles).
const BOTTLE_BREAK_EFFECT: i32 = 2002;
/// An egg hatches with a chance of 1 in 8, and then 4 chicks with a chance of 1 in 32.
const EGG_HATCH_CHANCE: i32 = 8;
const EGG_QUADRUPLETS_CHANCE: i32 = 32;
/// Particles of an egg cracking (each draws three floats) and of a pearl landing.
const EGG_PARTICLES: usize = 8;
const PEARL_PARTICLES: usize = 32;
/// An ender pearl spawns an endermite with a 5% chance (`doMobSpawning` on).
const ENDERMITE_CHANCE: f32 = 0.05;
/// Fall damage of an ender pearl teleport.
const PEARL_DAMAGE: f32 = 5.0;
/// Experience in a bottle: 3 plus two rolls of `nextInt(5)`.
const BOTTLE_XP_BASE: i32 = 3;
const BOTTLE_XP_ROLL: i32 = 5;
/// The creative game mode.
const CREATIVE: i32 = 1;
/// A splash potion reaches living entities in its box grown by this much
/// (`expand(4.0, 2.0, 4.0)`)...
const SPLASH_REACH: [f64; 3] = [4.0, 2.0, 4.0];
/// ...and closer than this (squared distance between positions below 16).
const SPLASH_RADIUS: f64 = 4.0;
/// A splash potion's effect lasting this many ticks or less is not given.
const SPLASH_MIN_DURATION: i32 = 20;
/// Instant health heals `4 << amplifier`, instant damage hurts `6 << amplifier`, scaled
/// by the share of the splash.
const HEAL_BASE: i32 = 4;
const HARM_BASE: i32 = 6;

/// What was thrown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `EntitySnowball`.
    Snowball,
    /// `EntityEgg`.
    Egg,
    /// `EntityEnderPearl`.
    EnderPearl,
    /// `EntityExpBottle`.
    ExpBottle,
    /// `EntityPotion` (splash potions).
    Potion,
}

/// Every kind, in a fixed order.
const KINDS: [Kind; 5] = [
    Kind::Snowball,
    Kind::Egg,
    Kind::EnderPearl,
    Kind::ExpBottle,
    Kind::Potion,
];

impl Kind {
    /// The saved entity id (`EntityList` name).
    pub fn saved_id(self) -> &'static str {
        match self {
            Self::Snowball => "Snowball",
            Self::Egg => "ThrownEgg",
            Self::EnderPearl => "ThrownEnderpearl",
            Self::ExpBottle => "ThrownExpBottle",
            Self::Potion => "ThrownPotion",
        }
    }

    /// The kind saved under `id`.
    pub fn from_saved_id(id: &str) -> Option<Self> {
        KINDS.into_iter().find(|k| k.saved_id() == id)
    }

    /// The entity's name (`getName`: the translated `entity.<id>.name`; en_US only
    /// names snowballs, the others show the key).
    pub fn name(self) -> String {
        match self {
            Self::Snowball => "Snowball".into(),
            other => format!("entity.{}.name", other.saved_id()),
        }
    }

    /// `S0EPacketSpawnObject` type.
    pub fn object_type(self) -> i8 {
        match self {
            Self::Snowball => 61,
            Self::Egg => 62,
            Self::EnderPearl => 65,
            Self::ExpBottle => 75,
            Self::Potion => 73,
        }
    }

    /// The item stack that throws it (potions only when they are splash potions,
    /// `ItemPotion.isSplash`).
    fn from_item(stack: &ItemStack) -> Option<Self> {
        match stack.id {
            item::SNOWBALL => Some(Self::Snowball),
            item::EGG => Some(Self::Egg),
            item::ENDER_PEARL => Some(Self::EnderPearl),
            item::EXPERIENCE_BOTTLE => Some(Self::ExpBottle),
            item::POTION if i32::from(stack.damage) & super::potion_helper::SPLASH != 0 => {
                Some(Self::Potion)
            }
            _ => None,
        }
    }

    /// Launch speed (`getVelocity`).
    fn velocity(self) -> f32 {
        match self {
            Self::ExpBottle => 0.7,
            Self::Potion => 0.5,
            _ => 1.5,
        }
    }

    /// Extra pitch of the launch, in degrees (`getInaccuracy`; bottles are lobbed).
    fn launch_pitch(self) -> f32 {
        match self {
            Self::ExpBottle | Self::Potion => -20.0,
            _ => 0.0,
        }
    }

    /// Downward acceleration per tick (`getGravityVelocity`).
    fn gravity(self) -> f32 {
        match self {
            Self::ExpBottle => 0.07,
            Self::Potion => 0.05,
            _ => 0.03,
        }
    }
}

/// One thrown entity.
#[derive(Debug, Clone)]
pub struct Throwable {
    /// What it is.
    pub kind: Kind,
    /// Position and motion (it never collides, so only these and the box matter).
    pub body: Body,
    /// The base entity state (rotation follows the flight direction).
    pub base: EntityBase,
    /// The block it is stuck in (`xTile`, `yTile`, `zTile`; vanilla never sets them in
    /// flight, so only loaded entities have one).
    pub tile: (i32, i32, i32),
    /// The block id it is stuck in (`inTile`), as saved: `None` saves as `""`.
    pub in_tile: Option<u16>,
    /// `throwableShake` (saved, counts down).
    pub shake: i32,
    /// `inGround`.
    pub in_ground: bool,
    /// The entity id of the player who threw it, once known (`thrower`).
    pub thrower: Option<i32>,
    /// The thrower's name (`throwerName`, saved as `ownerName`).
    pub owner_name: Option<String>,
    /// Ender pearls fly through the player who threw them (`field_181555_c`, not
    /// saved).
    pub launched_by: Option<i32>,
    /// `ticksInGround`.
    pub ticks_in_ground: i32,
    /// `ticksInAir`.
    pub ticks_in_air: i32,
    /// `firstUpdate`: no splash on the first tick.
    pub first_update: bool,
    /// Removed from the world (`isDead`).
    pub dead: bool,
    /// The chunk the entity was last added to.
    pub chunk: (i32, i32),
    /// A splash potion's item (`potionDamage`). Vanilla keeps the thrown stack itself,
    /// so a survival throw leaves it with a count of 0, which is also saved.
    pub potion: Option<ItemStack>,
}

impl Throwable {
    /// Vanilla's metadata for a throwable (the base entity values).
    pub fn metadata(&self) -> Vec<Metadata> {
        self.base.metadata()
    }

    /// `EntityThrowable.readEntityFromNBT` for a throwable loaded with its chunk.
    pub fn load(kind: Kind, body: Body, base: EntityBase, c: &Compound) -> Self {
        let p = body.pos;
        let in_tile = in_tile_from_nbt(c);
        let owner = c.get_str("ownerName");
        // EntityPotion: the `Potion` item, else an old `potionValue`; a potion whose item
        // does not load is removed.
        let potion = (kind == Kind::Potion).then(|| {
            if c.contains_key_of("Potion", Some(TagType::Compound)) {
                c.get_compound("Potion")
                    .and_then(super::player::item_from_nbt)
            } else {
                Some(ItemStack {
                    id: item::POTION,
                    count: 1,
                    damage: c.get_i32("potionValue").max(0) as i16,
                    nbt: None,
                })
            }
        });
        let dead = potion.as_ref().is_some_and(Option::is_none);
        Self {
            kind,
            body,
            base,
            tile: (
                i32::from(c.get_i16("xTile")),
                i32::from(c.get_i16("yTile")),
                i32::from(c.get_i16("zTile")),
            ),
            in_tile,
            shake: i32::from(c.get_i8("shake") as u8),
            in_ground: c.get_i8("inGround") == 1,
            thrower: None,
            owner_name: (!owner.is_empty()).then(|| owner.to_owned()),
            launched_by: None,
            ticks_in_ground: 0,
            ticks_in_air: 0,
            first_update: true,
            dead,
            chunk: (floor_f64(p.0) >> 4, floor_f64(p.2) >> 4),
            potion: potion.flatten(),
        }
    }

    /// `EntityThrowable.writeEntityToNBT`.
    pub fn write(&self, c: &mut Compound) {
        c.insert("xTile", Tag::Short(self.tile.0 as i16));
        c.insert("yTile", Tag::Short(self.tile.1 as i16));
        c.insert("zTile", Tag::Short(self.tile.2 as i16));
        let tile_name = self
            .in_tile
            .and_then(|id| krol_world::items::name_for_id(id as i16))
            .unwrap_or_default();
        c.insert("inTile", Tag::String(tile_name.to_owned()));
        c.insert("shake", Tag::Byte(self.shake as i8));
        c.insert("inGround", Tag::Byte(i8::from(self.in_ground)));
        c.insert(
            "ownerName",
            Tag::String(self.owner_name.clone().unwrap_or_default()),
        );
        if let Some(potion) = &self.potion {
            c.insert("Potion", Tag::Compound(super::item_data::stack_nbt(potion)));
        }
    }

    /// `EntityPotion.getPotionDamage`: the potion's damage value (its spawn data).
    pub fn potion_damage(&self) -> i32 {
        self.potion.as_ref().map_or(0, |p| i32::from(p.damage))
    }
}

/// The saved `inTile` of a projectile: a block name, or in old saves a numeric id
/// (unknown ids read as air).
pub(super) fn in_tile_from_nbt(c: &Compound) -> Option<u16> {
    if c.contains_key_of("inTile", Some(TagType::String)) {
        block_from_name(c.get_str("inTile"))
    } else {
        let id = (c.get_i8("inTile") as u8) as i16;
        Some(krol_world::items::name_for_id(id).map_or(block::AIR, |_| id as u16))
    }
}

/// `Block.getBlockFromName`: a registered name, else a numeric id, else nothing.
fn block_from_name(name: &str) -> Option<u16> {
    if let Some(id) = krol_world::items::id_for_name(name).filter(|&id| (0..256).contains(&id)) {
        return Some(id as u16);
    }
    let id: i32 = name.parse().ok()?;
    // getObjectById falls back to air for unknown ids.
    Some(
        u16::try_from(id)
            .ok()
            .filter(|&id| id < 256 && krol_world::items::name_for_id(id as i16).is_some())
            .unwrap_or(block::AIR),
    )
}

/// The rotation of a flight direction (`atan2` of the motion, in degrees).
pub(super) fn heading(motion: (f64, f64, f64)) -> (f32, f32) {
    let horizontal = sqrt_f64(motion.0 * motion.0 + motion.2 * motion.2);
    // Vanilla: (float) (atan2(..) * 180.0 / (float) Math.PI), evaluated left to right.
    let degrees = |radians: f64| (radians * 180.0 / f64::from(std::f32::consts::PI)) as f32;
    (
        degrees(atan2(motion.0, motion.2)),
        degrees(atan2(motion.1, f64::from(horizontal))),
    )
}

/// What a throwable hit this tick.
enum Hit {
    Block,
    Entity(i32, Target),
}

impl Game {
    /// `onItemRightClick` of snowballs, eggs, ender pearls and experience bottles:
    /// throws one (creative players keep the stack and cannot throw pearls), with the
    /// throw sound and the use statistic. `None` if the item is not a throwable.
    pub(super) fn throw_held(
        &mut self,
        i: usize,
        held: &ItemStack,
    ) -> Option<super::buckets::UseResult> {
        let kind = Kind::from_item(held)?;
        let creative = self.players[i].gamemode == CREATIVE;
        if kind == Kind::EnderPearl && creative {
            return Some(super::buckets::UseResult::Unchanged);
        }
        let slot = 36 + self.players[i].selected;
        if !creative && let Some(s) = self.players[i].inventory[slot].as_mut() {
            s.count -= 1;
        }
        // Item.itemRand (unseeded in vanilla).
        let r = &mut self.item_rand;
        let pitch = 0.4 / (r.next_float() * 0.4 + 0.8);
        let pos = self.players[i].pos;
        self.play_sound_near_except(None, THROW_SOUND, pos, THROW_VOLUME, pitch);
        // The potion entity keeps the (already shrunk) stack.
        let potion = (kind == Kind::Potion).then(|| {
            let mut stack = held.clone();
            if !creative {
                stack.count -= 1;
            }
            stack
        });
        self.spawn_thrown(i, kind, potion);
        if let Some(stat) = super::digging::use_stat(held.id) {
            self.add_stat(i, &stat, 1);
        }
        Some(if creative {
            super::buckets::UseResult::Unchanged
        } else {
            super::buckets::UseResult::Resized
        })
    }

    /// `new EntityThrowable(world, thrower)` + `spawnEntityInWorld`: the throwable
    /// starts at the player's eyes, a little to the side and below, flying where the
    /// player looks with a small random spread.
    fn spawn_thrown(&mut self, i: usize, kind: Kind, potion: Option<ItemStack>) -> i32 {
        let entity_id = self.next_entity_id();
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
            f64::from(-sin(yaw_rad) * cos(pitch_rad) * AIM_SPEED),
            f64::from(-sin(radians(pitch + kind.launch_pitch())) * AIM_SPEED),
            f64::from(cos(yaw_rad) * cos(pitch_rad) * AIM_SPEED),
        );
        let thrower = p.entity_id;
        let owner = p.profile.name.clone();
        // setThrowableHeading: normalize, add the spread, scale to the speed (the
        // entity's own random, unseeded in vanilla).
        let length = f64::from(sqrt_f64(aim.0 * aim.0 + aim.1 * aim.1 + aim.2 * aim.2));
        let r = &mut self.entity_rand;
        let spread = f64::from(SPREAD) * f64::from(HEADING_INACCURACY);
        let speed = f64::from(kind.velocity());
        let mut axis = |v: f64| (v / length + r.next_gaussian() * spread) * speed;
        let motion = (axis(aim.0), axis(aim.1), axis(aim.2));
        let (yaw, pitch) = heading(motion);
        let mut body = Body::player(pos, false);
        body.width = SIZE;
        body.height = SIZE;
        body.step_height = 0.0;
        body.motion = motion;
        let mut base = EntityBase::facing(yaw);
        base.pitch = pitch;
        self.thrown.insert(
            entity_id,
            Throwable {
                kind,
                body,
                base,
                tile: (-1, -1, -1),
                in_tile: None,
                shake: 0,
                in_ground: false,
                thrower: Some(thrower),
                owner_name: Some(owner),
                launched_by: (kind == Kind::EnderPearl).then_some(thrower),
                ticks_in_ground: 0,
                ticks_in_air: 0,
                first_update: true,
                dead: false,
                chunk: (floor_f64(pos.0) >> 4, floor_f64(pos.2) >> 4),
                potion,
            },
        );
        self.spawn_entity_in_world(entity_id);
        entity_id
    }

    /// One throwable's turn in the entity loop.
    pub(super) fn update_thrown(&mut self, id: i32) {
        if self.thrown.get(&id).is_some_and(|t| !t.dead) {
            self.tick_thrown(id);
            self.relocate_entity(id);
        }
        if self.thrown.get(&id).is_some_and(|t| t.dead) {
            self.thrown.remove(&id);
            self.remove_entity_from_chunk(id);
            self.untrack_entity(id);
        }
    }

    /// `EntityThrowable.getThrower`: the stored thrower, else the online player with the
    /// saved name (looked up until found).
    fn thrower_of(&mut self, id: i32) -> Option<i32> {
        let t = self.thrown.get(&id)?;
        if t.thrower.is_some() {
            return t.thrower;
        }
        let name = t.owner_name.clone()?;
        // TODO(M5): a UUID in `ownerName` names a non-player entity (getEntityFromUuid).
        let found = self
            .players
            .iter()
            .find(|p| !p.removed && p.profile.name == name)
            .map(|p| p.entity_id);
        if let Some(t) = self.thrown.get_mut(&id) {
            t.thrower = found;
        }
        found
    }

    /// `EntityThrowable.onUpdate` (with `EntityEnderPearl.onUpdate` in front).
    fn tick_thrown(&mut self, id: i32) {
        let thrower = self.thrower_of(id);
        let Some(t) = self.thrown.get(&id) else {
            return;
        };
        // A pearl dies with its thrower (a player who left the world counts as dead).
        if t.kind == Kind::EnderPearl
            && let Some(thrower) = thrower
            && !self
                .players
                .iter()
                .any(|p| p.entity_id == thrower && !p.removed && p.health() > 0.0)
        {
            if let Some(t) = self.thrown.get_mut(&id) {
                t.dead = true;
            }
            return;
        }
        let Some(mut t) = self.thrown.remove(&id) else {
            return;
        };
        let prev_rotation = (t.base.yaw, t.base.pitch);
        if self.projectile_entity_update(&mut t.body, &mut t.base, &mut t.first_update, SIZE) {
            t.dead = true;
        }
        if t.shake > 0 {
            t.shake -= 1;
        }
        if t.in_ground {
            let (x, y, z) = t.tile;
            let here = self.level.overworld.block(x, y, z) >> 4;
            if Some(here) == t.in_tile {
                t.ticks_in_ground += 1;
                if t.ticks_in_ground == GROUND_LIFETIME {
                    t.dead = true;
                }
                self.thrown.insert(id, t);
                return;
            }
            t.in_ground = false;
            let r = &mut self.entity_rand;
            t.body.motion.0 *= f64::from(r.next_float() * 0.2);
            t.body.motion.1 *= f64::from(r.next_float() * 0.2);
            t.body.motion.2 *= f64::from(r.next_float() * 0.2);
            t.ticks_in_ground = 0;
            t.ticks_in_air = 0;
        } else {
            t.ticks_in_air += 1;
        }
        let (pos, m) = (t.body.pos, t.body.motion);
        let start = pos;
        let mut end = (pos.0 + m.0, pos.1 + m.1, pos.2 + m.2);
        let block_hit =
            super::raytrace::ray_trace_blocks(&mut self.level.overworld, start, end, false, false);
        if let Some(h) = &block_hit {
            end = h.vec;
        }
        // The closest collidable entity on the way (the thrower only after a few ticks).
        let area = t
            .body
            .bb()
            .add_coord([m.0, m.1, m.2])
            .expand([1.0, 1.0, 1.0]);
        let mut closest: Option<(f64, i32, Target)> = None;
        for (other, target) in self.entities_within(&area, None) {
            if !self.can_be_collided_with(target)
                || (Some(other) == thrower && t.ticks_in_air < THROWER_GRACE)
            {
                continue;
            }
            let Some(bb) = self.target_box(target) else {
                continue;
            };
            let margin = f64::from(HIT_MARGIN);
            let bb = bb.expand([margin, margin, margin]);
            if let Some(v) = super::raytrace::intercept(&bb, start, end) {
                let d = (v.0 - start.0).powi(2) + (v.1 - start.1).powi(2) + (v.2 - start.2).powi(2);
                if closest.is_none_or(|(best, _, _)| d < best || best == 0.0) {
                    closest = Some((d, other, target));
                }
            }
        }
        let hit = match (closest, &block_hit) {
            (Some((_, other, target)), _) => Some(Hit::Entity(other, target)),
            (None, Some(h)) => {
                let state = self.level.overworld.block(h.pos.0, h.pos.1, h.pos.2);
                // TODO(M5): nether portals (setPortal) instead of an impact.
                (state >> 4 != block::PORTAL).then_some(Hit::Block)
            }
            (None, None) => None,
        };
        if let Some(hit) = hit {
            self.thrown.insert(id, t);
            self.thrown_impact(id, hit, thrower);
            let Some(again) = self.thrown.remove(&id) else {
                return;
            };
            t = again;
        }
        // Moves without collision, turns a fifth of the way towards the flight
        // direction and slows down.
        let m = t.body.motion;
        t.body.pos = (t.body.pos.0 + m.0, t.body.pos.1 + m.1, t.body.pos.2 + m.2);
        let (yaw, pitch) = heading(m);
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
        t.base.pitch = prev_pitch + (pitch - prev_pitch) * TURN_RATE;
        t.base.yaw = prev_yaw + (yaw - prev_yaw) * TURN_RATE;
        // Bubble particles in water are client-side.
        let drag = f64::from(if t.body.in_water {
            WATER_DRAG
        } else {
            AIR_DRAG
        });
        t.body.motion.0 *= drag;
        t.body.motion.1 *= drag;
        t.body.motion.2 *= drag;
        t.body.motion.1 -= f64::from(t.kind.gravity());
        let p = t.body.pos;
        t.chunk = (floor_f64(p.0) >> 4, floor_f64(p.2) >> 4);
        self.thrown.insert(id, t);
    }

    /// The parts of `Entity.onEntityUpdate` that matter for a projectile `width`
    /// wide: water pushes it and makes it splash (putting out fire), and below the
    /// world it dies (returns `true`).
    ///
    /// TODO(M5): lava sets it on fire.
    pub(super) fn projectile_entity_update(
        &mut self,
        body: &mut Body,
        base: &mut EntityBase,
        first_update: &mut bool,
        width: f32,
    ) -> bool {
        let bb = body
            .bb()
            .expand([0.0, f64::from(-0.4f32), 0.0])
            .contract([0.001, 0.001, 0.001]);
        let (water, push) = super::liquids::water_movement(&mut self.level.overworld, &bb, true);
        body.motion.0 += push.0;
        body.motion.1 += push.1;
        body.motion.2 += push.2;
        if water && !body.in_water && !*first_update {
            // resetHeight: the splash (particles are client-side), with the entity's own
            // random (unseeded in vanilla).
            let m = body.motion;
            let volume = (sqrt_f64(
                m.0 * m.0 * f64::from(0.2f32) + m.1 * m.1 + m.2 * m.2 * f64::from(0.2f32),
            ) * 0.2)
                .min(1.0);
            let r = &mut self.entity_rand;
            let pitch = 1.0 + (r.next_float() - r.next_float()) * 0.4;
            // The bubbles (three floats each) and splashes (two each) are client-side
            // but draw from the random.
            let particles = (1.0 + width * 20.0).ceil() as usize;
            for _ in 0..particles * (3 + 2) {
                let _ = r.next_float();
            }
            if !base.silent {
                self.play_sound_near_except(None, SPLASH_SOUND, body.pos, volume, pitch);
            }
        }
        body.in_water = water;
        if water {
            base.wet();
        }
        *first_update = false;
        body.pos.1 < VOID_Y
    }

    /// `Entity.isWet` of a projectile: in water, or in the rain at its feet or top.
    pub(super) fn projectile_wet(&mut self, body: &Body) -> bool {
        if body.in_water {
            return true;
        }
        let p = body.pos;
        let (x, z) = (floor_f64(p.0), floor_f64(p.2));
        let top = floor_f64(p.1 + f64::from(body.height));
        self.rains_at((x, floor_f64(p.1), z)) || self.rains_at((x, top, z))
    }

    /// `canBeCollidedWith` of the entities krol simulates: living players, falling
    /// blocks, primed TNT, paintings and item frames.
    pub(super) fn can_be_collided_with(&self, target: Target) -> bool {
        match target {
            Target::Player(i) => !self.players[i].removed,
            Target::Falling(id) => self.falling.get(&id).is_some_and(|f| !f.dead),
            Target::Tnt(id) => self.tnt.get(&id).is_some_and(|t| !t.dead),
            Target::Hanging(id) => self.hanging.get(&id).is_some_and(|h| !h.dead),
            // EntityArmorStand.canBeCollidedWith: not markers.
            Target::ArmorStand(id) => self
                .armor_stands
                .get(&id)
                .is_some_and(|s| !s.dead && !s.is_marker()),
            Target::Item(_) | Target::Orb(_) | Target::Thrown(_) | Target::Arrow(_) => false,
        }
    }

    /// `onImpact` of each kind.
    fn thrown_impact(&mut self, id: i32, hit: Hit, thrower: Option<i32>) {
        let Some(t) = self.thrown.get(&id) else {
            return;
        };
        let (kind, pos, launched_by) = (t.kind, t.body.pos, t.launched_by);
        let (bb, potion, potion_damage) = (t.body.bb(), t.potion.clone(), t.potion_damage());
        let source = DamageSource::Thrown(thrower);
        let entity_hit = match hit {
            Hit::Entity(other, target) => Some((other, target)),
            Hit::Block => None,
        };
        match kind {
            Kind::Snowball => {
                // Snowballs hurt blazes for 3 (TODO(M5)); everything else for 0.
                if let Some((_, target)) = entity_hit {
                    let _ = self.attack_target(target, source, 0.0);
                }
            }
            Kind::Egg => {
                if let Some((_, target)) = entity_hit {
                    let _ = self.attack_target(target, source, 0.0);
                }
                let r = &mut self.entity_rand;
                if r.next_int_bounded(EGG_HATCH_CHANCE) == 0 {
                    let _chicks = if r.next_int_bounded(EGG_QUADRUPLETS_CHANCE) == 0 {
                        4
                    } else {
                        1
                    };
                    // TODO(M5): spawn the chicks (baby chickens facing the egg's yaw).
                }
                // The crack particles are client-side but draw their speeds.
                for _ in 0..EGG_PARTICLES * 3 {
                    let _ = r.next_float();
                }
            }
            Kind::EnderPearl => {
                if let Some((other, target)) = entity_hit {
                    // The pearl flies on through its own thrower.
                    if Some(other) == launched_by {
                        return;
                    }
                    let _ = self.attack_target(target, source, 0.0);
                }
                let r = &mut self.entity_rand;
                for _ in 0..PEARL_PARTICLES {
                    let _ = (r.next_double(), r.next_gaussian(), r.next_gaussian());
                }
                self.pearl_teleport(thrower, pos);
            }
            Kind::ExpBottle => {
                self.block_effect_data(
                    BOTTLE_BREAK_EFFECT,
                    (floor_f64(pos.0), floor_f64(pos.1), floor_f64(pos.2)),
                    0,
                );
                let r = &mut self.world_rand;
                let xp = BOTTLE_XP_BASE
                    + r.next_int_bounded(BOTTLE_XP_ROLL)
                    + r.next_int_bounded(BOTTLE_XP_ROLL);
                self.spawn_orbs(pos, xp);
            }
            Kind::Potion => {
                let direct = entity_hit.map(|(other, _)| other);
                self.splash(pos, &bb, potion.as_ref(), direct, thrower);
                self.block_effect_data(
                    BOTTLE_BREAK_EFFECT,
                    (floor_f64(pos.0), floor_f64(pos.1), floor_f64(pos.2)),
                    potion_damage,
                );
            }
        }
        if let Some(t) = self.thrown.get_mut(&id) {
            t.dead = true;
        }
    }

    /// `EntityPotion.onImpact`'s area effect: every living entity (not spectating) in
    /// reach gets the potion's effects, scaled by its distance (fully for the entity hit
    /// directly). Instant effects act at once; the others last that share of their
    /// duration when it is over a second.
    ///
    /// TODO(M5): mobs (undead are healed by harming and hurt by healing).
    fn splash(
        &mut self,
        pos: (f64, f64, f64),
        bb: &super::physics::Aabb,
        potion: Option<&ItemStack>,
        direct: Option<i32>,
        thrower: Option<i32>,
    ) {
        use super::potions::{Effect, effect, effect_type};
        let effects = potion.map_or_else(Vec::new, super::food::potion_item_effects);
        if effects.is_empty() {
            return;
        }
        let area = bb.expand(SPLASH_REACH);
        for (other, target) in self.entities_within(&area, None) {
            let Target::Player(i) = target else {
                continue;
            };
            if self.players[i].removed {
                continue;
            }
            let at = self.players[i].pos;
            let d2 = (pos.0 - at.0).powi(2) + (pos.1 - at.1).powi(2) + (pos.2 - at.2).powi(2);
            if d2 >= SPLASH_RADIUS * SPLASH_RADIUS {
                continue;
            }
            let share = if Some(other) == direct {
                1.0
            } else {
                // Math.sqrt (double precision, not MathHelper).
                1.0 - d2.sqrt() / SPLASH_RADIUS
            };
            for e in &effects {
                if effect_type(i32::from(e.id)).is_some_and(|t| t.instant) {
                    // Potion.affectEntity (players are never undead).
                    let amount = |base: i32| {
                        (share * f64::from(base.wrapping_shl(u32::from(e.amplifier))) + 0.5) as i32
                    };
                    match e.id {
                        effect::HEAL => self.heal_player(i, amount(HEAL_BASE) as f32),
                        effect::HARM => {
                            let source = match thrower {
                                Some(by) => DamageSource::IndirectMagic(by),
                                None => DamageSource::Magic,
                            };
                            self.attack_player(i, source, amount(HARM_BASE) as f32);
                        }
                        // Saturation is instant too, but does nothing from a splash.
                        _ => {}
                    }
                } else {
                    let duration = (share * f64::from(e.duration) + 0.5) as i32;
                    if duration > SPLASH_MIN_DURATION {
                        self.add_effect(
                            i,
                            Effect {
                                id: e.id,
                                amplifier: e.amplifier,
                                duration,
                                ambient: false,
                                particles: true,
                            },
                        );
                    }
                }
            }
        }
    }

    /// `EntityEnderPearl.onImpact` on the server: a thrower who is still online, in this
    /// world and awake is teleported to the pearl and takes 5 fall damage.
    fn pearl_teleport(&mut self, thrower: Option<i32>, pos: (f64, f64, f64)) {
        let Some(i) = thrower.and_then(|id| {
            self.players
                .iter()
                .position(|p| p.entity_id == id && !p.removed)
        }) else {
            return;
        };
        if self.players[i].sleeping {
            return;
        }
        let roll = self.entity_rand.next_float();
        if roll < ENDERMITE_CHANCE && self.level.data.game_rule("doMobSpawning") != Some("false") {
            // TODO(M5): an endermite (spawned by a player) at the thrower's old place.
        }
        // TODO(M5): riders get off their mount first.
        let p = &mut self.players[i];
        let rot = p.rotation;
        Self::set_player_location(p, pos, rot);
        p.fall_distance = 0.0;
        self.attack_player(i, DamageSource::Fall, PEARL_DAMAGE);
    }

    /// The box a target collides with, for the projectiles' entity ray traces.
    pub(super) fn target_box(&self, target: Target) -> Option<super::physics::Aabb> {
        Some(match target {
            Target::Player(i) => super::physics::Aabb::at(self.players[i].pos, 0.6, 1.8),
            Target::Item(id) => self.items.get(&id)?.body.bb(),
            Target::Orb(id) => self.orbs.get(&id)?.body.bb(),
            Target::Falling(id) => self.falling.get(&id)?.body.bb(),
            Target::Tnt(id) => self.tnt.get(&id)?.body.bb(),
            Target::Thrown(id) => self.thrown.get(&id)?.body.bb(),
            Target::Arrow(id) => self.arrows.get(&id)?.body.bb(),
            Target::Hanging(id) => self.hanging.get(&id)?.bb,
            Target::ArmorStand(id) => self.armor_stands.get(&id)?.body.bb(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_their_saved_ids() {
        for kind in KINDS {
            assert_eq!(Kind::from_saved_id(kind.saved_id()), Some(kind));
        }
        assert_eq!(Kind::from_saved_id("Arrow"), None);
    }

    #[test]
    fn block_names_resolve_like_vanilla() {
        assert_eq!(block_from_name(""), None);
        assert_eq!(block_from_name("minecraft:stone"), Some(1));
        assert_eq!(block_from_name("stone"), Some(1));
        assert_eq!(block_from_name("4"), Some(4));
        assert_eq!(block_from_name("nonsense"), None);
    }

    #[test]
    fn thrown_entities_save_like_vanilla() {
        let mut fresh = Throwable::load(
            Kind::EnderPearl,
            Body::player((0.0, 70.0, 0.0), false),
            EntityBase::default(),
            &Compound::new(),
        );
        fresh.tile = (-1, -1, -1);
        fresh.owner_name = Some("jeb_".into());
        let mut c = Compound::new();
        fresh.write(&mut c);
        // A fresh throwable is in no block: inTile saves as "" and reads back as none.
        assert_eq!(c.get_str("inTile"), "");
        assert_eq!(c.get_i16("yTile"), -1);
        assert_eq!(c.get_str("ownerName"), "jeb_");
        let loaded = Throwable::load(
            Kind::EnderPearl,
            Body::player((0.0, 70.0, 0.0), false),
            EntityBase::default(),
            &c,
        );
        assert_eq!(loaded.in_tile, None);
        assert_eq!(loaded.owner_name.as_deref(), Some("jeb_"));
        assert_eq!(loaded.thrower, None);
        // Old saves keep the block as a byte.
        let mut old = Compound::new();
        old.insert("inTile", Tag::Byte(3));
        old.insert("inGround", Tag::Byte(1));
        let loaded = Throwable::load(
            Kind::Snowball,
            Body::player((0.0, 70.0, 0.0), false),
            EntityBase::default(),
            &old,
        );
        assert_eq!(loaded.in_tile, Some(3));
        assert!(loaded.in_ground);
    }

    #[test]
    fn heading_points_along_the_motion() {
        let (yaw, pitch) = heading((0.0, 0.0, 1.0));
        assert_eq!((yaw, pitch), (0.0, 0.0));
        let (yaw, _) = heading((1.0, 0.0, 0.0));
        assert!((yaw - 90.0).abs() < 1e-4);
    }
}
