//! Armor stands (vanilla: `EntityArmorStand`, a living entity, placed by
//! `ItemArmorStand`).
//!
//! An armor stand runs the living entity update (fire, air, gravity, water and lava,
//! ladders, falling) but takes no ordinary damage: a punch from a player marks it, and
//! a second punch within 5 ticks (or any arrow) knocks it over, dropping itself and
//! what it wears. Creative players remove it at once, explosions break it without
//! dropping the stand, fire burns it down. Right-clicking it with an item puts the item
//! on (armor, heads and pumpkins go to their slot, anything else into its hand if it
//! shows arms); right-clicking it with an empty hand takes back what was clicked.
//!
//! TODO(M5): status effects on armor stands (`ActiveEffects` is kept as saved), the
//! sounds of armor stands loaded without `Silent` (steps, splashes, falls), riding
//! minecarts pushing them, dispensers putting armor on them, death by health set to 0.

use std::collections::BTreeSet;

use krol_java::math::{floor_f32, floor_f64, wrap_degrees_f32};
use krol_nbt::{Compound, List, Tag, TagType};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack, Metadata, MetadataValue, Property};

use super::Game;
use super::attributes::{KNOCKBACK_RESISTANCE, MAX_HEALTH, MOVEMENT_SPEED};
use super::damage::DamageSource;
use super::ids::{block, item};
use super::physics::{self, Aabb, Body};
use super::saved_entities::EntityBase;

/// Saved id (`EntityList` name).
pub const SAVED_ID: &str = "ArmorStand";
/// The en_US `entity.ArmorStand.name`.
pub const NAME: &str = "Armor Stand";
/// `S0EPacketSpawnObject` type of armor stands.
pub const OBJECT_TYPE: i8 = 78;
/// `setSize(0.5F, 1.975F)` (markers shrink to nothing).
const WIDTH: f32 = 0.5;
const HEIGHT: f32 = 1.975;
/// `EntityLivingBase.stepHeight`.
const STEP_HEIGHT: f32 = 0.6;
/// The base of `generic.maxHealth`, and a new stand's health.
const BASE_MAX_HEALTH: f32 = 20.0;
/// A burning stand breaks once its health would drop to this or below.
const BREAK_HEALTH: f32 = 0.5;
/// Fire blocks hurt an already burning stand by this much.
const IN_FIRE_DAMAGE: f32 = 0.15;
/// Burning hurts the stand by this much every second.
const ON_FIRE_DAMAGE: f32 = 4.0;
/// Fire blocks set a stand on fire for this long.
const IN_FIRE_SECONDS: i32 = 5;
/// Lava sets entities on fire for this long (`setOnFireFromLava`).
const LAVA_SECONDS: i32 = 15;
/// Fire blocks set entities that keep standing in them on fire for this long.
const FIRE_BLOCK_SECONDS: i32 = 8;
/// Fire arrows set what they hit on fire for this long.
const ARROW_FIRE_SECONDS: i32 = 5;
/// A second punch within this many ticks knocks the stand over.
const PUNCH_WINDOW: i64 = 5;
/// Entities below this height are killed (`Entity.onEntityUpdate`).
const VOID_Y: f64 = -64.0;
/// `getAir` out of water.
const FULL_AIR: i16 = 300;
/// The air value at which a drowning entity is hurt and its air reset to 0.
const DROWN_AT: i16 = -20;
/// `Entity.fireResistance`: after a dry move the fire counter rests at minus this.
const FIRE_RESISTANCE: i16 = 1;
/// Depth strider counts up to this level.
const MAX_DEPTH_STRIDER: f32 = 3.0;
/// Fire protection shortens fire by 15% per level.
const FIRE_PROTECTION_FACTOR: f32 = 0.15;
/// The motion below which an axis stops (`onLivingUpdate`).
const MOTION_EPSILON: f64 = 0.005;
/// Gravity and drag on land (`moveEntityWithHeading`).
const GRAVITY: f64 = 0.08;
const DRAG: f32 = 0.98;
const AIR_FRICTION: f32 = 0.91;
/// Gravity in liquids.
const LIQUID_GRAVITY: f64 = 0.02;
/// Climbing out of a liquid onto a ledge.
const LEDGE_CHECK_LIFT: f32 = 0.6;
const LEDGE_JUMP: f32 = 0.3;
/// Ladders and vines: the horizontal speed limit, the slowest fall and the climb.
const LADDER_SPEED: f64 = 0.15;
const LADDER_CLIMB: f64 = 0.2;
/// Landing particles: `BLOCK_DUST`, and the plank state id of the break particles.
const BLOCK_DUST: i32 = 38;
const PLANKS_STATE_ID: i32 = 5;
/// Break particles: count and speed.
const BREAK_PARTICLES: i32 = 10;
const BREAK_PARTICLE_SPEED: f32 = 0.05;
/// Game modes.
const CREATIVE: i32 = 1;
const ADVENTURE: i32 = 2;
const SPECTATOR: i32 = 3;
/// The game rule that lets broken mobs trample farmland.
const MOB_GRIEFING: &str = "mobGriefing";

/// Equipment slots (`getEquipmentInSlot`): held item, boots, leggings, chestplate,
/// helmet.
pub const SLOTS: usize = 5;
const HAND: usize = 0;
const BOOTS: usize = 1;
const LEGS: usize = 2;
const CHEST: usize = 3;
const HEAD: usize = 4;

/// Metadata indices.
const FLAGS_INDEX: u8 = 0;
const AIR_INDEX: u8 = 1;
const HEALTH_INDEX: u8 = 6;
const POTION_COLOR_INDEX: u8 = 7;
const POTION_AMBIENT_INDEX: u8 = 8;
const ARROWS_INDEX: u8 = 9;
const STAND_FLAGS_INDEX: u8 = 10;
const POSE_INDEX: u8 = 11;
/// Entity flag bits (metadata entry 0).
const BURNING_BIT: i8 = 1;
const INVISIBLE_BIT: i8 = 1 << 5;
/// Armor stand flag bits (metadata entry 10).
pub const SMALL: u8 = 1;
pub const NO_GRAVITY: u8 = 2;
pub const SHOW_ARMS: u8 = 4;
pub const NO_BASE_PLATE: u8 = 8;
pub const MARKER: u8 = 16;
/// `DisabledSlots`: bit `slot` keeps the slot from being swapped at all, bit
/// `slot + 8` from being emptied, bit `slot + 16` from being filled.
const TAKE_SHIFT: usize = 8;
const PUT_SHIFT: usize = 16;

/// The posed body parts, as saved under `Pose`, and their default angles in degrees.
const POSE_KEYS: [&str; 6] = ["Head", "Body", "LeftArm", "RightArm", "LeftLeg", "RightLeg"];
const DEFAULT_POSE: [[f32; 3]; 6] = [
    [0.0, 0.0, 0.0],
    [0.0, 0.0, 0.0],
    [-10.0, 0.0, -10.0],
    [-15.0, 0.0, 10.0],
    [-1.0, 0.0, -1.0],
    [1.0, 0.0, 1.0],
];
const HEAD_PART: usize = 0;
const BODY_PART: usize = 1;
/// `ItemArmorStand.applyRandomRotations`: the head tilts back up to 5 degrees and turns
/// up to 10 either way, the body turns up to 5 either way.
const HEAD_TILT: f32 = 5.0;
const HEAD_TURN: f32 = 20.0;
const BODY_TURN: f32 = 10.0;

/// The attributes of a living entity in save order, with their base values
/// (`movementSpeed` keeps the default of 0.7F: only players set it).
const ATTRIBUTES: [(&str, f64); 3] = [
    (MAX_HEALTH, 20.0),
    (KNOCKBACK_RESISTANCE, 0.0),
    (MOVEMENT_SPEED, 0.699_999_988_079_071),
];
/// The attributes clients are told about, in the order the tracker sends them.
const WATCHED: [usize; 2] = [2, 0];

/// One armor stand.
#[derive(Debug, Clone)]
pub struct ArmorStand {
    /// Position, motion, size and the movement state.
    pub body: Body,
    /// The base entity state.
    pub base: EntityBase,
    /// `rotationYawHead` (random from the constructor, never turned).
    pub head_yaw: f32,
    /// What it wears and holds (`contents`).
    pub equipment: [Option<ItemStack>; SLOTS],
    /// `previousEquipment`: what the viewers were last told.
    previous_equipment: [Option<ItemStack>; SLOTS],
    /// Metadata entry 10.
    pub flags: u8,
    /// The angles of head, body, arms and legs (metadata entries 11-16).
    pub pose: [[f32; 3]; 6],
    /// `disabledSlots`.
    disabled_slots: i32,
    /// `canInteract` (the invisible flag, bit 5 of metadata entry 0).
    pub invisible: bool,
    /// The burning bit of metadata entry 0 (`setFlag(0, fire > 0)` each update).
    burning_flag: bool,
    /// `getHealth` (metadata entry 6).
    health: f32,
    /// `absorptionAmount`.
    absorption: f32,
    hurt_time: i16,
    hurt_by_timestamp: i32,
    death_time: i16,
    /// The attribute bases and saved modifier lists, in [`ATTRIBUTES`] order.
    attributes: [(f64, Option<List>); 3],
    /// `ActiveEffects`, kept as saved.
    effects: Option<List>,
    /// `punchCooldown`: the world time of the last punch.
    punch_time: i64,
    /// `field_181028_bj`: whether the stand had its full size at its last update.
    sized: bool,
    /// `firstUpdate`.
    first_update: bool,
    /// Metadata entries changed since the tracker last sent them (all at first).
    pub dirty_metadata: BTreeSet<u8>,
    /// Some metadata changed since the tracker last looked (`objectChanged`; the
    /// constructor's health and silence count).
    pub metadata_changed: bool,
    /// Removed from the world (`isDead`).
    pub dead: bool,
    /// The chunk it is in (as of its last update).
    pub chunk: (i32, i32),
}

/// The angle a stand placed by a player looking `yaw` faces: the player's back, in
/// steps of 45 degrees.
fn placed_yaw(yaw: f32) -> f32 {
    floor_f32((wrap_degrees_f32(yaw - 180.0) + 22.5) / 45.0) as f32 * 45.0
}

/// The chunk of a position.
fn chunk_of(pos: (f64, f64, f64)) -> (i32, i32) {
    (floor_f64(pos.0) >> 4, floor_f64(pos.2) >> 4)
}

/// `NBTTagCompound.merge`: compounds merge key by key, anything else replaces.
fn merge(into: &mut Compound, from: &Compound) {
    for (key, tag) in from.iter() {
        if let (Tag::Compound(inner), Some(Tag::Compound(existing))) = (tag, into.get_mut(key)) {
            merge(existing, inner);
            continue;
        }
        into.insert(key.clone(), tag.clone());
    }
}

/// `EntityLiving.getArmorPosition` for the items a stand wears (heads and pumpkins on
/// the head go through [`ArmorStand::interact`]).
fn armor_slot(id: i16) -> usize {
    match id {
        298..=317 => HEAD - ((id - 298) % 4) as usize,
        _ => HAND,
    }
}

impl ArmorStand {
    /// A new stand at `pos` (`EntityArmorStand(world, x, y, z)` after the living
    /// constructor turned its head to `head_yaw`).
    fn new(pos: (f64, f64, f64), head_yaw: f32) -> Self {
        let mut body = Body::player(pos, false);
        body.width = WIDTH;
        body.height = HEIGHT;
        body.step_height = STEP_HEIGHT;
        let mut s = Self {
            body,
            base: EntityBase {
                yaw: head_yaw,
                silent: true,
                ..EntityBase::default()
            },
            head_yaw,
            equipment: Default::default(),
            previous_equipment: Default::default(),
            flags: 0,
            pose: DEFAULT_POSE,
            disabled_slots: 0,
            invisible: false,
            burning_flag: false,
            health: BASE_MAX_HEALTH,
            absorption: 0.0,
            hurt_time: 0,
            hurt_by_timestamp: 0,
            death_time: 0,
            attributes: ATTRIBUTES.map(|(_, b)| (b, None)),
            effects: None,
            punch_time: 0,
            sized: false,
            first_update: true,
            dirty_metadata: BTreeSet::new(),
            metadata_changed: true,
            dead: false,
            chunk: chunk_of(pos),
        };
        s.dirty_metadata = s.metadata().iter().map(|m| m.index).collect();
        s
    }

    /// `DataWatcher.updateObject` of entry `index` with a changed value.
    fn metadata_updated(&mut self, index: u8) {
        self.dirty_metadata.insert(index);
        self.metadata_changed = true;
    }

    /// Small (child-sized).
    pub fn is_small(&self) -> bool {
        self.flags & SMALL != 0
    }

    /// A marker: no size, no interaction, no damage.
    pub fn is_marker(&self) -> bool {
        self.flags & MARKER != 0
    }

    fn no_gravity(&self) -> bool {
        self.flags & NO_GRAVITY != 0
    }

    /// `getEyeHeight` (small stands keep their full box: half its height).
    pub fn eye_height(&self) -> f32 {
        if self.is_small() {
            self.body.height * 0.5
        } else {
            self.body.height * 0.9
        }
    }

    /// `getName`.
    pub fn name(&self) -> String {
        self.base
            .custom_name
            .clone()
            .unwrap_or_else(|| NAME.to_owned())
    }

    /// `isEntityAlive`.
    pub fn alive(&self) -> bool {
        !self.dead && self.health > 0.0
    }

    /// `isBurning` on the server.
    fn burning(&self) -> bool {
        self.base.fire > 0
    }

    /// `getMaxHealth` (saved modifiers are not applied).
    fn max_health(&self) -> f32 {
        self.attributes[0].0.clamp(0.0, 1024.0) as f32
    }

    /// `setHealth`.
    fn set_health(&mut self, health: f32) {
        let health = health.clamp(0.0, self.max_health());
        if health != self.health {
            self.health = health;
            self.metadata_updated(HEALTH_INDEX);
        }
    }

    /// `setAir`.
    fn set_air(&mut self, air: i16) {
        if air != self.base.air {
            self.base.air = air;
            self.metadata_updated(AIR_INDEX);
        }
    }

    /// Sets a bit of metadata entry 10.
    fn set_flag(&mut self, bit: u8, on: bool) {
        let flags = if on {
            self.flags | bit
        } else {
            self.flags & !bit
        };
        if flags != self.flags {
            self.flags = flags;
            self.metadata_updated(STAND_FLAGS_INDEX);
        }
    }

    /// `setInvisible`.
    fn set_invisible(&mut self, invisible: bool) {
        if invisible != self.invisible {
            self.invisible = invisible;
            self.metadata_updated(FLAGS_INDEX);
        }
    }

    /// Sets a body part's angles (`setHeadRotation` and the like).
    fn set_part(&mut self, part: usize, angles: [f32; 3]) {
        if angles != self.pose[part] {
            self.pose[part] = angles;
            self.metadata_updated(POSE_INDEX + part as u8);
        }
    }

    /// The highest level of an enchantment on anything it wears or holds
    /// (`EnchantmentHelper.getMaxEnchantmentLevel` over `getInventory`).
    fn max_enchantment(&self, id: i16) -> i32 {
        self.equipment
            .iter()
            .map(|s| super::item_data::enchantment_level(s.as_ref(), id))
            .max()
            .unwrap_or(0)
    }

    /// `Entity.setFire(seconds)`, shortened by fire protection
    /// (`EnchantmentProtection.getFireTimeForEntity`).
    fn set_fire(&mut self, seconds: i32) {
        let mut ticks = seconds * 20;
        let level = self.max_enchantment(super::item_data::FIRE_PROTECTION);
        if level > 0 {
            ticks -= (ticks as f32 * level as f32 * FIRE_PROTECTION_FACTOR).floor() as i32;
        }
        if i32::from(self.base.fire) < ticks {
            self.base.fire = ticks as i16;
        }
    }

    /// `func_181550_a`: back to full size or shrunk to nothing, keeping the position.
    fn resize(&mut self, full: bool) {
        let (w, h) = if full { (WIDTH, HEIGHT) } else { (0.0, 0.0) };
        self.body.width = w;
        self.body.height = h;
    }

    /// The complete data watcher, in index order.
    pub fn metadata(&self) -> Vec<Metadata> {
        use MetadataValue::{Byte, Float, Int, Rotation};
        let md = |index, value| Metadata { index, value };
        let mut out = self.base.metadata();
        out[0] = md(
            FLAGS_INDEX,
            Byte(
                (if self.burning_flag { BURNING_BIT } else { 0 })
                    | (if self.invisible { INVISIBLE_BIT } else { 0 }),
            ),
        );
        out.push(md(HEALTH_INDEX, Float(self.health)));
        out.push(md(POTION_COLOR_INDEX, Int(0)));
        out.push(md(POTION_AMBIENT_INDEX, Byte(0)));
        out.push(md(ARROWS_INDEX, Byte(0)));
        out.push(md(STAND_FLAGS_INDEX, Byte(self.flags as i8)));
        for (k, [x, y, z]) in self.pose.iter().enumerate() {
            out.push(md(POSE_INDEX + k as u8, Rotation(*x, *y, *z)));
        }
        out
    }

    /// The watched attributes (`getWatchedAttributes`) for Entity Properties.
    pub fn properties(&self) -> Vec<Property> {
        WATCHED
            .iter()
            .map(|&k| (ATTRIBUTES[k].0.to_owned(), self.attributes[k].0, Vec::new()))
            .collect()
    }

    /// `readEntityFromNBT` of a living entity and an armor stand, for one loaded with
    /// its chunk or spawned with an `EntityTag` (`body` and `base` already read).
    pub fn load(mut body: Body, base: EntityBase, c: &Compound, head_yaw: f32) -> Self {
        body.width = WIDTH;
        body.height = HEIGHT;
        body.step_height = STEP_HEIGHT;
        let mut s = Self::new(body.pos, head_yaw);
        s.body = body;
        s.base = base;
        // EntityLivingBase.readEntityFromNBT.
        s.absorption = c.get_f32("AbsorptionAmount").max(0.0);
        if let Some(list) = c.get_list("Attributes", TagType::Compound) {
            for a in list.iter() {
                let Tag::Compound(a) = a else { continue };
                let name = a.get_str("Name");
                let Some(k) = ATTRIBUTES.iter().position(|(n, _)| *n == name) else {
                    tracing::warn!("Ignoring unknown attribute '{name}'");
                    continue;
                };
                s.attributes[k] = (
                    a.get_f64("Base"),
                    a.get_list("Modifiers", TagType::Compound)
                        .filter(|l| !l.is_empty())
                        .cloned(),
                );
            }
        }
        s.effects = c
            .get_list("ActiveEffects", TagType::Compound)
            .filter(|l| !l.is_empty())
            .cloned();
        let health = match c.get("HealF").filter(|t| t.as_f32().is_some()) {
            Some(t) => t.as_f32().unwrap_or(0.0),
            None => match c.get("Health") {
                None => s.max_health(),
                Some(Tag::Float(v)) => *v,
                Some(Tag::Short(v)) => f32::from(*v),
                Some(_) => s.health,
            },
        };
        s.health = BASE_MAX_HEALTH;
        s.set_health(health);
        s.hurt_time = c.get_i16("HurtTime");
        s.death_time = c.get_i16("DeathTime");
        s.hurt_by_timestamp = c.get_i32("HurtByTimestamp");
        // EntityArmorStand.readEntityFromNBT.
        if let Some(list) = c.get_list("Equipment", TagType::Compound) {
            for (k, slot) in s.equipment.iter_mut().enumerate() {
                *slot = list.get(k).and_then(|t| match t {
                    Tag::Compound(item) => super::player::item_from_nbt(item),
                    _ => None,
                });
            }
        } else if c.contains_key_of("Equipment", Some(TagType::List)) {
            // An empty list of no type.
            s.equipment = Default::default();
        }
        s.set_invisible(c.get_bool("Invisible"));
        s.set_flag(SMALL, c.get_bool("Small"));
        s.set_flag(SHOW_ARMS, c.get_bool("ShowArms"));
        s.disabled_slots = c.get_i32("DisabledSlots");
        s.set_flag(NO_GRAVITY, c.get_bool("NoGravity"));
        s.set_flag(NO_BASE_PLATE, c.get_bool("NoBasePlate"));
        s.set_flag(MARKER, c.get_bool("Marker"));
        s.sized = !s.is_marker();
        let pose = c.get_compound("Pose").cloned().unwrap_or_default();
        for (part, key) in POSE_KEYS.iter().enumerate() {
            let angles = match pose.get_list(key, TagType::Float) {
                Some(l) if !l.is_empty() => {
                    std::array::from_fn(|k| l.get(k).and_then(Tag::as_f32).unwrap_or(0.0))
                }
                _ => DEFAULT_POSE[part],
            };
            s.set_part(part, angles);
        }
        s.chunk = chunk_of(s.body.pos);
        s
    }

    /// `writeEntityToNBT` of a living entity and an armor stand.
    pub fn write(&self, c: &mut Compound) {
        c.insert("HealF", Tag::Float(self.health));
        c.insert(
            "Health",
            Tag::Short(f64::from(self.health).ceil() as i32 as i16),
        );
        c.insert("HurtTime", Tag::Short(self.hurt_time));
        c.insert("HurtByTimestamp", Tag::Int(self.hurt_by_timestamp));
        c.insert("DeathTime", Tag::Short(self.death_time));
        c.insert("AbsorptionAmount", Tag::Float(self.absorption));
        let mut attributes = List::new();
        for ((name, _), (base, modifiers)) in ATTRIBUTES.iter().zip(&self.attributes) {
            let mut a = Compound::new();
            a.insert("Name", Tag::String((*name).to_owned()));
            a.insert("Base", Tag::Double(*base));
            if let Some(m) = modifiers {
                a.insert("Modifiers", Tag::List(m.clone()));
            }
            let _ = attributes.push(Tag::Compound(a));
        }
        c.insert("Attributes", Tag::List(attributes));
        if let Some(effects) = &self.effects {
            c.insert("ActiveEffects", Tag::List(effects.clone()));
        }
        let mut equipment = List::new();
        for slot in &self.equipment {
            let item = slot
                .as_ref()
                .map(super::item_data::stack_nbt)
                .unwrap_or_default();
            let _ = equipment.push(Tag::Compound(item));
        }
        c.insert("Equipment", Tag::List(equipment));
        if self.base.custom_name_visible && self.base.custom_name.is_none() {
            c.insert("CustomNameVisible", Tag::Byte(1));
        }
        c.insert("Invisible", Tag::Byte(i8::from(self.invisible)));
        c.insert("Small", Tag::Byte(i8::from(self.is_small())));
        c.insert("ShowArms", Tag::Byte(i8::from(self.flags & SHOW_ARMS != 0)));
        c.insert("DisabledSlots", Tag::Int(self.disabled_slots));
        c.insert("NoGravity", Tag::Byte(i8::from(self.no_gravity())));
        c.insert(
            "NoBasePlate",
            Tag::Byte(i8::from(self.flags & NO_BASE_PLATE != 0)),
        );
        if self.is_marker() {
            c.insert("Marker", Tag::Byte(1));
        }
        let mut pose = Compound::new();
        for (part, key) in POSE_KEYS.iter().enumerate() {
            if self.pose[part] != DEFAULT_POSE[part] {
                let mut l = List::new();
                for v in self.pose[part] {
                    let _ = l.push(Tag::Float(v));
                }
                pose.insert(*key, Tag::List(l));
            }
        }
        c.insert("Pose", Tag::Compound(pose));
    }
}

impl Game {
    /// The living constructor's three `Math.random()` calls (after the entity id and
    /// UUID), the last of which turns the head; armor stands loaded from a chunk draw
    /// them too.
    pub(super) fn new_stand_head(&mut self) -> f32 {
        let _ = self.math_random.next_double();
        let _ = self.math_random.next_double();
        (self.math_random.next_double() * f64::from(std::f32::consts::PI) * 2.0) as f32
    }

    /// `ItemArmorStand.onItemUse`: on any face but the bottom, in the clicked block if
    /// it can be replaced or else in front of it, a stand rises if both its block
    /// spaces are free of blocks and entities. It faces the player in steps of 45
    /// degrees with a slightly random head and body; an `EntityTag` on the item merges
    /// into it.
    pub(super) fn place_armor_stand(&mut self, i: usize, pos: (i32, i32, i32), face: u8) {
        if face == super::placement::DOWN {
            return;
        }
        let Some(stack) = self.players[i].held().cloned() else {
            return;
        };
        let clicked = self.level.overworld.block(pos.0, pos.1, pos.2);
        let at = if super::blocks::is_replaceable(clicked) {
            pos
        } else {
            super::placement::offset(pos, face)
        };
        // canPlayerEdit: adventure players and spectators never get here.
        let above = (at.0, at.1 + 1, at.2);
        for p in [at, above] {
            let state = self.level.overworld.block(p.0, p.1, p.2);
            if state >> 4 != block::AIR && !super::blocks::is_replaceable(state) {
                return;
            }
        }
        let (x, y, z) = (f64::from(at.0), f64::from(at.1), f64::from(at.2));
        let space = Aabb::new([x, y, z], [x + 1.0, y + 2.0, z + 1.0]);
        if !self.entities_within(&space, None).is_empty() {
            return;
        }
        self.set_block_notify(at, block::AIR << 4);
        self.set_block_notify(above, block::AIR << 4);
        let id = self.next_entity_id();
        let head = self.new_stand_head();
        let mut stand = ArmorStand::new((x + 0.5, y, z + 0.5), head);
        stand.base.yaw = placed_yaw(self.players[i].rotation.0);
        stand.base.pitch = 0.0;
        // applyRandomRotations, with the world's random.
        let r = &mut self.world_rand;
        let [hx, hy, hz] = stand.pose[HEAD_PART];
        let tilt = r.next_float() * HEAD_TILT;
        let turn = r.next_float() * HEAD_TURN - HEAD_TURN / 2.0;
        stand.set_part(HEAD_PART, [hx + tilt, hy + turn, hz]);
        let [bx, by, bz] = stand.pose[BODY_PART];
        let turn = r.next_float() * BODY_TURN - BODY_TURN / 2.0;
        stand.set_part(BODY_PART, [bx, by + turn, bz]);
        if let Some(tag) = stack
            .nbt
            .as_ref()
            .and_then(|t| t.get_compound("EntityTag"))
            .cloned()
        {
            // writeToNBTOptional, merge, readFromNBT.
            let uuid = self.entity_uuids.get(&id).copied().unwrap_or_default();
            let mut c = self.armor_stand_nbt(&stand, uuid);
            merge(&mut c, &tag);
            let (loaded, uuid) = self.armor_stand_from_nbt(&c, head);
            if let Some(uuid) = uuid {
                self.entity_uuids.insert(id, uuid);
            }
            let mut loaded = loaded;
            // readFromNBT keeps the data watcher's change flag and earlier changes.
            loaded
                .dirty_metadata
                .extend(stand.dirty_metadata.iter().copied());
            stand = loaded;
        }
        self.armor_stands.insert(id, stand);
        self.spawn_entity_in_world(id);
        let p = &mut self.players[i];
        if p.gamemode != CREATIVE {
            let held = 36 + p.selected;
            if let Some(s) = p.inventory[held].as_mut() {
                s.count -= 1;
            }
        }
        self.count_item_use(i);
    }

    /// One armor stand's turn in the entity loop (`EntityArmorStand.onUpdate`).
    pub(super) fn update_armor_stand(&mut self, id: i32) {
        if self.armor_stands.get(&id).is_some_and(|s| !s.dead) {
            self.living_base_update(id);
            if self.armor_stands.get(&id).is_some_and(|s| !s.dead) {
                self.send_stand_equipment(id);
                self.stand_living_update(id);
                self.stand_bumps_minecarts(id);
            }
            if let Some(s) = self.armor_stands.get_mut(&id) {
                // The marker flag changes the size on the next update.
                let marker = s.is_marker();
                if !s.sized && marker {
                    s.resize(false);
                    s.sized = marker;
                } else if s.sized && !marker {
                    s.resize(true);
                    s.sized = marker;
                }
                s.chunk = chunk_of(s.body.pos);
            }
            self.relocate_entity(id);
        }
        if self.armor_stands.get(&id).is_some_and(|s| s.dead) {
            self.armor_stands.remove(&id);
            self.remove_entity_from_chunk(id);
            self.untrack_entity(id);
        }
    }

    /// `EntityLivingBase.onEntityUpdate` (with `Entity.onEntityUpdate`).
    fn living_base_update(&mut self, id: i32) {
        let Some(s) = self.armor_stands.get_mut(&id) else {
            return;
        };
        if s.base.portal_cooldown > 0 {
            s.base.portal_cooldown -= 1;
        }
        self.stand_water_movement(id);
        let Some(s) = self.armor_stands.get_mut(&id) else {
            return;
        };
        if s.base.fire > 0 {
            if s.base.fire % 20 == 0 {
                self.attack_armor_stand(id, DamageSource::OnFire);
            }
            if let Some(s) = self.armor_stands.get_mut(&id) {
                s.base.fire -= 1;
            }
        }
        let Some(s) = self.armor_stands.get(&id) else {
            return;
        };
        let lava_box =
            s.body
                .bb()
                .expand([f64::from(-0.1f32), f64::from(-0.4f32), f64::from(-0.1f32)]);
        if self.any_block_in(&lava_box, block::FLOWING_LAVA..=block::LAVA) {
            // setOnFireFromLava: the lava damage itself does nothing to a stand.
            self.attack_armor_stand(id, DamageSource::Lava);
            if let Some(s) = self.armor_stands.get_mut(&id) {
                s.set_fire(LAVA_SECONDS);
                s.base.fall_distance *= 0.5;
            }
        }
        let Some(s) = self.armor_stands.get_mut(&id) else {
            return;
        };
        if s.body.pos.1 < VOID_Y {
            // EntityLivingBase.kill.
            self.attack_armor_stand(id, DamageSource::OutOfWorld);
        }
        let Some(s) = self.armor_stands.get_mut(&id) else {
            return;
        };
        let burning = s.base.fire > 0;
        if burning != s.burning_flag {
            s.burning_flag = burning;
            s.metadata_updated(FLAGS_INDEX);
        }
        s.first_update = false;
        // The living part. Suffocation does nothing to a stand.
        if s.alive() {
            let (pos, eye) = (s.body.pos, s.eye_height());
            if self.eyes_in_water_at(pos, eye) {
                let Some(s) = self.armor_stands.get_mut(&id) else {
                    return;
                };
                // decreaseAirSupply: respiration may save the air, with the stand's
                // own random (unseeded in vanilla).
                let respiration = s.max_enchantment(super::item_data::RESPIRATION);
                let mut air = s.base.air;
                if respiration <= 0 || self.entity_rand.next_int_bounded(respiration + 1) <= 0 {
                    air -= 1;
                }
                s.set_air(air);
                if s.base.air == DROWN_AT {
                    s.set_air(0);
                    // The bubbles are client-side but draw from the random; the
                    // drowning damage does nothing to a stand.
                    for _ in 0..8 * 6 {
                        let _ = self.entity_rand.next_float();
                    }
                }
            } else if let Some(s) = self.armor_stands.get_mut(&id) {
                s.set_air(FULL_AIR);
            }
        }
        let Some(s) = self.armor_stands.get(&id) else {
            return;
        };
        if s.alive() && self.stand_wet(id) {
            if let Some(s) = self.armor_stands.get_mut(&id) {
                s.base.fire = 0;
            }
        }
        if let Some(s) = self.armor_stands.get_mut(&id)
            && s.hurt_time > 0
        {
            s.hurt_time -= 1;
        }
    }

    /// `Entity.handleWaterMovement`: flowing water pushes the stand; in water, falls
    /// end and fire goes out.
    fn stand_water_movement(&mut self, id: i32) {
        let Some(s) = self.armor_stands.get(&id) else {
            return;
        };
        let bb = s
            .body
            .bb()
            .expand([0.0, f64::from(-0.4f32), 0.0])
            .contract([0.001, 0.001, 0.001]);
        let (water, push) = super::liquids::water_movement(&mut self.level.overworld, &bb, true);
        let Some(s) = self.armor_stands.get_mut(&id) else {
            return;
        };
        // handleMaterialAcceleration adds nothing without a push (keeping a -0.0).
        if push != (0.0, 0.0, 0.0) {
            s.body.motion.0 += push.0;
            s.body.motion.1 += push.1;
            s.body.motion.2 += push.2;
        }
        if water && !s.body.in_water && !s.first_update {
            // resetHeight: the splash is silent and its particles client-side, but they
            // draw from the stand's own random (unseeded in vanilla).
            let particles = (1.0 + s.body.width * 20.0) as usize;
            for _ in 0..2 + particles * 5 {
                let _ = self.entity_rand.next_float();
            }
        }
        let Some(s) = self.armor_stands.get_mut(&id) else {
            return;
        };
        s.body.in_water = water;
        if water {
            s.base.wet();
        }
    }

    /// `Entity.isWet`: in water or in the rain at the feet or the top.
    fn stand_wet(&mut self, id: i32) -> bool {
        let Some(s) = self.armor_stands.get(&id) else {
            return false;
        };
        if s.body.in_water {
            return true;
        }
        let p = s.body.pos;
        let (x, z) = (floor_f64(p.0), floor_f64(p.2));
        let (feet, top) = (floor_f64(p.1), floor_f64(p.1 + f64::from(s.body.height)));
        self.rains_at((x, feet, z)) || self.rains_at((x, top, z))
    }

    /// `EntityLivingBase.onUpdate`: changed equipment goes to the viewers. (A stand's
    /// attributes have no `attackDamage` for weapons to change.)
    fn send_stand_equipment(&mut self, id: i32) {
        let Some(s) = self.armor_stands.get_mut(&id) else {
            return;
        };
        let mut packets = Vec::new();
        for slot in 0..SLOTS {
            if s.equipment[slot] != s.previous_equipment[slot] {
                s.previous_equipment[slot] = s.equipment[slot].clone();
                packets.push(ClientboundPlay::EntityEquipment {
                    entity_id: id,
                    slot: slot as i16,
                    item: s.equipment[slot].clone(),
                });
            }
        }
        for packet in packets {
            self.send_to_tracking(id, &packet);
        }
    }

    /// `EntityLivingBase.onLivingUpdate` with no input: tiny motion stops, then
    /// `moveEntityWithHeading(0, 0)` unless the stand has no gravity.
    fn stand_living_update(&mut self, id: i32) {
        let Some(s) = self.armor_stands.get_mut(&id) else {
            return;
        };
        for m in [
            &mut s.body.motion.0,
            &mut s.body.motion.1,
            &mut s.body.motion.2,
        ] {
            if m.abs() < MOTION_EPSILON {
                *m = 0.0;
            }
        }
        if s.no_gravity() {
            return;
        }
        let in_water = s.body.in_water;
        let lava_box =
            s.body
                .bb()
                .expand([f64::from(-0.1f32), f64::from(-0.4f32), f64::from(-0.1f32)]);
        if in_water {
            let y0 = s.body.pos.1;
            let mut friction = 0.8f32;
            let mut strider =
                (s.max_enchantment(super::item_data::DEPTH_STRIDER) as f32).min(MAX_DEPTH_STRIDER);
            if !s.body.on_ground {
                strider *= 0.5;
            }
            if strider > 0.0 {
                friction += (0.546_000_06 - friction) * strider / 3.0;
            }
            self.move_stand(id);
            let Some(s) = self.armor_stands.get_mut(&id) else {
                return;
            };
            s.body.motion.0 *= f64::from(friction);
            s.body.motion.1 *= f64::from(0.8f32);
            s.body.motion.2 *= f64::from(friction);
            s.body.motion.1 -= LIQUID_GRAVITY;
            self.stand_ledge_climb(id, y0);
        } else if self.any_block_in(&lava_box, block::FLOWING_LAVA..=block::LAVA) {
            let y0 = self.armor_stands.get(&id).map_or(0.0, |s| s.body.pos.1);
            self.move_stand(id);
            let Some(s) = self.armor_stands.get_mut(&id) else {
                return;
            };
            s.body.motion.0 *= 0.5;
            s.body.motion.1 *= 0.5;
            s.body.motion.2 *= 0.5;
            s.body.motion.1 -= LIQUID_GRAVITY;
            self.stand_ledge_climb(id, y0);
        } else {
            let friction = self.stand_friction(id);
            if self.stand_on_ladder(id)
                && let Some(s) = self.armor_stands.get_mut(&id)
            {
                s.body.motion.0 = s.body.motion.0.clamp(-LADDER_SPEED, LADDER_SPEED);
                s.body.motion.2 = s.body.motion.2.clamp(-LADDER_SPEED, LADDER_SPEED);
                s.base.fall_distance = 0.0;
                if s.body.motion.1 < -LADDER_SPEED {
                    s.body.motion.1 = -LADDER_SPEED;
                }
            }
            self.move_stand(id);
            let climbing = self
                .armor_stands
                .get(&id)
                .is_some_and(|s| s.body.collided_horizontally)
                && self.stand_on_ladder(id);
            let Some(s) = self.armor_stands.get_mut(&id) else {
                return;
            };
            if climbing {
                s.body.motion.1 = LADDER_CLIMB;
            }
            s.body.motion.1 -= GRAVITY;
            s.body.motion.1 *= f64::from(DRAG);
            s.body.motion.0 *= f64::from(friction);
            s.body.motion.2 *= f64::from(friction);
        }
    }

    /// `EntityArmorStand.collideWithNearbyEntities`: rideable minecarts overlapping the
    /// stand with their position within 0.2 (squared) of its own are pushed.
    fn stand_bumps_minecarts(&mut self, id: i32) {
        let Some(s) = self.armor_stands.get(&id) else {
            return;
        };
        let (bb, pos) = (s.body.bb(), s.body.pos);
        let carts: Vec<i32> = self
            .minecarts
            .iter()
            .filter(|&(_, m)| {
                let p = m.body.pos;
                let d = (p.0 - pos.0).powi(2) + (p.1 - pos.1).powi(2) + (p.2 - pos.2).powi(2);
                !m.dead && m.body.bb().intersects(&bb) && d <= 0.2
            })
            .map(|(&c, _)| c)
            .collect();
        for cart in carts {
            self.minecart_collision(cart, super::minecart::CartPusher::ArmorStand(id));
        }
    }

    /// The horizontal friction on land: the slipperiness of the block below when on
    /// the ground.
    fn stand_friction(&mut self, id: i32) -> f32 {
        let Some(s) = self.armor_stands.get(&id) else {
            return AIR_FRICTION;
        };
        if !s.body.on_ground {
            return AIR_FRICTION;
        }
        let p = s.body.pos;
        let below = self.level.overworld.block(
            floor_f64(p.0),
            floor_f64(s.body.bb().min[1]) - 1,
            floor_f64(p.2),
        ) >> 4;
        super::blocks::slipperiness(below) * AIR_FRICTION
    }

    /// `isOnLadder`: the stand's feet are in a ladder or vine.
    fn stand_on_ladder(&mut self, id: i32) -> bool {
        let Some(s) = self.armor_stands.get(&id) else {
            return false;
        };
        let p = s.body.pos;
        let state = self.level.overworld.block(
            floor_f64(p.0),
            floor_f64(s.body.bb().min[1]),
            floor_f64(p.2),
        );
        matches!(state >> 4, block::LADDER | block::VINE)
    }

    /// Swimming out of a liquid onto a ledge (`isOffsetPositionInLiquid`).
    fn stand_ledge_climb(&mut self, id: i32, y0: f64) {
        let Some(s) = self.armor_stands.get(&id) else {
            return;
        };
        if !s.body.collided_horizontally {
            return;
        }
        let m = s.body.motion;
        let offset = [
            m.0,
            m.1 + f64::from(LEDGE_CHECK_LIFT) - s.body.pos.1 + y0,
            m.2,
        ];
        let bb = s.body.bb().offset(offset);
        let free = physics::colliding_boxes(&mut self.level.overworld, &bb).is_empty()
            && !self.any_block_in(&bb, block::FLOWING_WATER..=block::LAVA);
        if free && let Some(s) = self.armor_stands.get_mut(&id) {
            s.body.motion.1 = f64::from(LEDGE_JUMP);
        }
    }

    /// `Entity.moveEntity` with the stand's motion, then what follows in vanilla: the
    /// fall (`EntityLivingBase.updateFallState`), the blocks touched, fire and water.
    fn move_stand(&mut self, id: i32) {
        let Some(s) = self.armor_stands.get_mut(&id) else {
            return;
        };
        let mut body = s.body.clone();
        let m = body.motion;
        let outcome = physics::move_entity(&mut self.level.overworld, &mut body, m.0, m.1, m.2);
        // Steps are silent.
        if let Some((pos, state)) = outcome.walked_on
            && matches!(state >> 4, block::REDSTONE_ORE | block::LIT_REDSTONE_ORE)
        {
            self.activate_redstone_ore(pos, state);
        }
        if let Some(s) = self.armor_stands.get_mut(&id) {
            s.body = body;
        }
        // EntityLivingBase.updateFallState: water is checked once more first.
        if self.armor_stands.get(&id).is_some_and(|s| !s.body.in_water) {
            self.stand_water_movement(id);
        }
        self.stand_fall(id, outcome.moved_y, outcome.below);
        // doBlockCollisions.
        let Some(s) = self.armor_stands.get(&id) else {
            return;
        };
        let bb = s.body.bb();
        let touched = self.block_collisions(&bb);
        if let Some(s) = self.armor_stands.get_mut(&id) {
            s.body.touched(touched);
            if touched.web {
                s.base.fall_distance = 0.0;
            }
        }
        self.stand_move_fire(id);
    }

    /// `updateFallState` after a move: landing from more than 3 blocks raises dust, a
    /// landing ends the fall (which hurts a stand no more than anything else, but may
    /// trample farmland), falling counts down.
    fn stand_fall(&mut self, id: i32, moved_y: f64, below: ((i32, i32, i32), u16)) {
        let Some(s) = self.armor_stands.get(&id) else {
            return;
        };
        let (pos, on_ground, distance) = (s.body.pos, s.body.on_ground, s.base.fall_distance);
        let (block_pos, state) = below;
        if on_ground && distance > 3.0 && state >> 4 != block::AIR {
            let extra = krol_java::math::ceil_f32(distance - 3.0) as f32;
            let strength = f64::from((0.2 + extra / 15.0).min(10.0)).min(2.5);
            let count = (150.0 * strength) as i32;
            let id_meta = i32::from(state);
            let packet = ClientboundPlay::Particle {
                id: BLOCK_DUST,
                long_distance: false,
                pos: (pos.0 as f32, pos.1 as f32, pos.2 as f32),
                offset: (0.0, 0.0, 0.0),
                speed: 0.15,
                count,
                args: vec![(id_meta >> 4) | ((id_meta & 15) << 12)],
            };
            self.send_particle(&packet, pos);
        }
        if on_ground {
            if distance > 0.0 {
                // BlockFarmland.onFallenUpon: the world's random decides, and only with
                // mob griefing on.
                if state >> 4 == block::FARMLAND
                    && self.world_rand.next_float() < distance - 0.5
                    && self.level.data.game_rule(MOB_GRIEFING) != Some("false")
                {
                    self.set_block_notify(block_pos, block::DIRT << 4);
                }
                if let Some(s) = self.armor_stands.get_mut(&id) {
                    s.base.fall_distance = 0.0;
                }
            }
        } else if moved_y < 0.0
            && let Some(s) = self.armor_stands.get_mut(&id)
        {
            s.base.fall_distance = (f64::from(s.base.fall_distance) - moved_y) as f32;
        }
    }

    /// The end of `moveEntity`: standing in fire or lava burns (and sets on fire after
    /// a while); getting wet puts the fire out.
    fn stand_move_fire(&mut self, id: i32) {
        let wet = self.stand_wet(id);
        let Some(s) = self.armor_stands.get(&id) else {
            return;
        };
        let bb = s.body.bb().contract([0.001, 0.001, 0.001]);
        if self.flammable_within(&bb) {
            self.attack_armor_stand(id, DamageSource::InFire);
            if !wet && let Some(s) = self.armor_stands.get_mut(&id) {
                s.base.fire += 1;
                if s.base.fire == 0 {
                    s.set_fire(FIRE_BLOCK_SECONDS);
                }
            }
        } else if let Some(s) = self.armor_stands.get_mut(&id)
            && s.base.fire <= 0
        {
            s.base.fire = -FIRE_RESISTANCE;
        }
        if let Some(s) = self.armor_stands.get_mut(&id)
            && wet
            && s.base.fire > 0
        {
            // The fizz is silent but its pitch draws from the stand's own random.
            let _ = self.entity_rand.next_float();
            let _ = self.entity_rand.next_float();
            s.base.fire = -FIRE_RESISTANCE;
        }
    }

    /// `EntityArmorStand.attackEntityFrom`: the void and `/kill` remove it; nothing
    /// hurts an invulnerable, invisible or marker stand; explosions break it, dropping
    /// only what it wears; fire burns it; arrows and a second punch knock it over;
    /// creative players remove it. It never counts as hurt.
    pub(super) fn attack_armor_stand(&mut self, id: i32, source: DamageSource) {
        let Some(s) = self.armor_stands.get(&id) else {
            return;
        };
        if source == DamageSource::OutOfWorld {
            self.set_entity_dead(id);
            return;
        }
        let (invulnerable, untouchable) = (s.base.invulnerable, s.invisible || s.is_marker());
        let (burning, health) = (s.burning(), s.health);
        let attacker = source.attacker();
        let creative = self.creative_player(attacker);
        // isEntityInvulnerable.
        if (invulnerable && !creative) || untouchable {
            return;
        }
        if source.explosion() {
            self.drop_stand_contents(id);
            self.set_entity_dead(id);
            return;
        }
        match source {
            DamageSource::InFire => {
                if burning {
                    self.damage_armor_stand(id, IN_FIRE_DAMAGE);
                } else if let Some(s) = self.armor_stands.get_mut(&id) {
                    s.set_fire(IN_FIRE_SECONDS);
                }
                return;
            }
            DamageSource::OnFire if health > BREAK_HEALTH => {
                self.damage_armor_stand(id, ON_FIRE_DAMAGE);
                return;
            }
            _ => {}
        }
        let arrow = match source {
            DamageSource::Arrow { arrow, .. } => Some(arrow),
            DamageSource::Player(_) => None,
            _ => return,
        };
        if let Some(arrow) = arrow
            && let Some(a) = self.arrows.get_mut(&arrow)
        {
            a.dead = true;
        }
        // A player who may not edit the world (adventure, spectator) cannot break it.
        if let Some(i) = attacker.and_then(|a| self.players.iter().position(|p| p.entity_id == a))
            && matches!(self.players[i].gamemode, ADVENTURE | SPECTATOR)
        {
            return;
        }
        if creative {
            self.stand_break_particles(id);
            self.set_entity_dead(id);
            return;
        }
        let now = self.level.overworld.total_time;
        let Some(s) = self.armor_stands.get_mut(&id) else {
            return;
        };
        if now - s.punch_time > PUNCH_WINDOW && arrow.is_none() {
            s.punch_time = now;
        } else {
            // dropBlock: the stand itself, then what it wears.
            let at = s.body.pos;
            let at = (floor_f64(at.0), floor_f64(at.1), floor_f64(at.2));
            self.spawn_as_entity(at, single(item::ARMOR_STAND));
            self.drop_stand_contents(id);
            self.stand_break_particles(id);
            self.set_entity_dead(id);
        }
    }

    /// Whether the entity attributed with damage is a creative player
    /// (`DamageSource.isCreativePlayer`).
    fn creative_player(&self, attacker: Option<i32>) -> bool {
        attacker.is_some_and(|a| {
            self.players
                .iter()
                .any(|p| p.entity_id == a && p.gamemode == CREATIVE)
        })
    }

    /// `damageArmorStand`: health goes down; at 0.5 or below the stand breaks, dropping
    /// what it wears.
    fn damage_armor_stand(&mut self, id: i32, amount: f32) {
        let Some(s) = self.armor_stands.get_mut(&id) else {
            return;
        };
        let health = s.health - amount;
        if health <= BREAK_HEALTH {
            self.drop_stand_contents(id);
            self.set_entity_dead(id);
        } else {
            s.set_health(health);
        }
    }

    /// `dropContents`: everything it wears and holds drops from the block above its
    /// feet.
    fn drop_stand_contents(&mut self, id: i32) {
        let Some(s) = self.armor_stands.get_mut(&id) else {
            return;
        };
        let p = s.body.pos;
        let at = (floor_f64(p.0), floor_f64(p.1) + 1, floor_f64(p.2));
        let stacks: Vec<ItemStack> = s
            .equipment
            .iter_mut()
            .filter_map(|slot| match slot {
                Some(stack) if stack.count > 0 => slot.take(),
                _ => None,
            })
            .collect();
        for stack in stacks {
            self.spawn_as_entity(at, stack);
        }
    }

    /// `playParticles`: plank dust where the stand stood.
    fn stand_break_particles(&mut self, id: i32) {
        let Some(s) = self.armor_stands.get(&id) else {
            return;
        };
        let (w, h) = (s.body.width, s.body.height);
        let p = s.body.pos;
        let at = (p.0, p.1 + f64::from(h) / 1.5, p.2);
        let packet = ClientboundPlay::Particle {
            id: BLOCK_DUST,
            long_distance: false,
            pos: (at.0 as f32, at.1 as f32, at.2 as f32),
            offset: (w / 4.0, h / 4.0, w / 4.0),
            speed: BREAK_PARTICLE_SPEED,
            count: BREAK_PARTICLES,
            args: vec![PLANKS_STATE_ID],
        };
        self.send_particle(&packet, at);
    }

    /// `setFire` on a stand (fire arrows).
    pub(super) fn ignite_armor_stand(&mut self, id: i32) {
        if let Some(s) = self.armor_stands.get_mut(&id) {
            s.set_fire(ARROW_FIRE_SECONDS);
        }
    }

    /// A player's attack on a stand (`attackTargetEntityWithCurrentItem`): fire aspect
    /// sets it on fire, the hit does nothing to it, and the fire goes out again.
    pub(super) fn player_attacks_armor_stand(&mut self, i: usize, id: i32) {
        let fire_aspect = super::item_data::enchantment_level(
            self.players[i].held(),
            super::item_data::FIRE_ASPECT,
        );
        let ignited = fire_aspect > 0
            && self.armor_stands.get_mut(&id).is_some_and(|s| {
                if s.burning() {
                    return false;
                }
                s.set_fire(1);
                true
            });
        let attacker = self.players[i].entity_id;
        self.attack_armor_stand(id, DamageSource::Player(attacker));
        if ignited && let Some(s) = self.armor_stands.get_mut(&id) {
            s.base.fire = 0;
        }
    }

    /// `EntityArmorStand.interactAt`: `hit` is where the player's ray met the stand,
    /// relative to its position. A held item goes to its slot (armor, heads and
    /// pumpkins to theirs, anything else into the hand if the stand shows arms); an
    /// empty hand takes what was clicked.
    pub(super) fn interact_armor_stand(&mut self, i: usize, id: i32, hit: (f32, f32, f32)) {
        let Some(s) = self.armor_stands.get(&id) else {
            return;
        };
        if s.is_marker() || self.players[i].gamemode == SPECTATOR {
            return;
        }
        let held = self.players[i].held().cloned();
        let mut item_slot = held.as_ref().map_or(HAND, |h| armor_slot(h.id));
        if let Some(h) = &held
            && (h.id == item::SKULL || h.id == block::PUMPKIN as i16)
        {
            item_slot = HEAD;
        }
        let small = s.is_small();
        let y = if small {
            f64::from(hit.1) * 2.0
        } else {
            f64::from(hit.1)
        };
        let worn = |slot: usize| s.equipment[slot].is_some();
        let mut clicked = HAND;
        if y >= 0.1 && y < 0.1 + if small { 0.8 } else { 0.45 } && worn(BOOTS) {
            clicked = BOOTS;
        } else if y >= 0.9 + if small { 0.3 } else { 0.0 }
            && y < 0.9 + if small { 1.0 } else { 0.7 }
            && worn(CHEST)
        {
            clicked = CHEST;
        } else if y >= 0.4 && y < 0.4 + if small { 1.0 } else { 0.8 } && worn(LEGS) {
            clicked = LEGS;
        } else if y >= 1.6 && worn(HEAD) {
            clicked = HEAD;
        }
        let clicked_full = worn(clicked);
        let disabled = |slot: usize| s.disabled_slots & (1 << slot) != 0;
        if disabled(clicked) || disabled(item_slot) {
            clicked = item_slot;
            if disabled(item_slot) {
                if disabled(HAND) {
                    return;
                }
                clicked = HAND;
            }
        }
        if held.is_some() && item_slot == HAND && s.flags & SHOW_ARMS == 0 {
            return;
        }
        if held.is_some() {
            self.swap_with_stand(i, id, item_slot);
        } else if clicked_full {
            // Vanilla quirk: the slot may have moved to the hand above.
            self.swap_with_stand(i, id, clicked);
        }
    }

    /// `func_175422_a`: the held item and the stand's slot trade places (creative
    /// players put a copy of one into an empty slot; a held stack of more than one puts
    /// one into an empty slot).
    fn swap_with_stand(&mut self, i: usize, id: i32, slot: usize) {
        let Some(s) = self.armor_stands.get_mut(&id) else {
            return;
        };
        let current = s.equipment[slot].clone();
        let locked = |shift: usize| s.disabled_slots & (1 << (slot + shift)) != 0;
        if (current.is_some() && locked(TAKE_SHIFT)) || (current.is_none() && locked(PUT_SHIFT)) {
            return;
        }
        let p = &mut self.players[i];
        let hand = 36 + p.selected;
        let held = p.inventory[hand].clone();
        let creative = p.gamemode == CREATIVE;
        let one = |stack: &ItemStack| ItemStack {
            count: 1,
            ..stack.clone()
        };
        match (&held, &current) {
            (Some(h), c) if creative && c.as_ref().is_none_or(|c| c.id == 0) => {
                s.equipment[slot] = Some(one(h));
            }
            (h, _) if h.as_ref().is_none_or(|h| h.count <= 1) => {
                s.equipment[slot] = held;
                p.inventory[hand] = current;
            }
            (Some(h), None) => {
                s.equipment[slot] = Some(one(h));
                if let Some(h) = p.inventory[hand].as_mut() {
                    h.count -= 1;
                }
            }
            _ => {}
        }
    }

    /// `writeToNBTOptional` of a stand that is not in the world yet.
    fn armor_stand_nbt(&self, stand: &ArmorStand, uuid: u128) -> Compound {
        let mut c = Compound::new();
        c.insert("id", Tag::String(SAVED_ID.into()));
        stand.base.write_entity(&stand.body, uuid, &mut c);
        stand.write(&mut c);
        c
    }
}

/// One of an item.
fn single(id: i16) -> ItemStack {
    ItemStack {
        id,
        count: 1,
        damage: 0,
        nbt: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placed_stands_face_the_player_in_eighths() {
        // wrapAngleTo180 keeps -180.
        assert_eq!(placed_yaw(0.0), -180.0);
        assert_eq!(placed_yaw(180.0), 0.0);
        assert_eq!(placed_yaw(90.0), -90.0);
        assert_eq!(placed_yaw(22.0), -180.0);
        assert_eq!(placed_yaw(23.0), -135.0);
        assert_eq!(placed_yaw(-450.0), 90.0);
    }

    #[test]
    fn armor_goes_to_its_slot() {
        assert_eq!(armor_slot(298), HEAD); // leather helmet
        assert_eq!(armor_slot(299), CHEST);
        assert_eq!(armor_slot(300), LEGS);
        assert_eq!(armor_slot(301), BOOTS);
        assert_eq!(armor_slot(317), BOOTS); // golden boots
        assert_eq!(armor_slot(item::ARROW), HAND);
    }

    #[test]
    fn metadata_lists_every_entry_in_order() {
        let s = ArmorStand::new((0.5, 64.0, 0.5), 0.0);
        let indices: Vec<u8> = s.metadata().iter().map(|m| m.index).collect();
        assert_eq!(
            indices,
            [0, 1, 2, 3, 4, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
        assert!(s.metadata_changed);
        assert_eq!(s.dirty_metadata.len(), indices.len());
        // A new stand is silent and at full health.
        assert_eq!(s.metadata()[4].value, MetadataValue::Byte(1));
        assert_eq!(s.metadata()[5].value, MetadataValue::Float(20.0));
    }

    #[test]
    fn saves_round_trip() {
        let mut s = ArmorStand::new((1.5, 70.0, -2.5), 0.0);
        s.flags = SMALL | SHOW_ARMS | MARKER;
        s.pose[HEAD_PART] = [3.0, -4.5, 0.0];
        s.equipment[HEAD] = Some(single(item::SKULL));
        s.disabled_slots = 0x10101;
        s.health = 12.5;
        let mut c = Compound::new();
        s.write(&mut c);
        assert_eq!(c.get_i16("Health"), 13);
        assert_eq!(c.get_i8("Marker"), 1);
        let pose = c.get_compound("Pose").cloned().unwrap_or_default();
        assert!(pose.contains_key("Head"));
        assert!(!pose.contains_key("Body"));
        let attributes = c.get_list("Attributes", TagType::Compound).map(List::len);
        assert_eq!(attributes, Some(3));
        let loaded = ArmorStand::load(s.body.clone(), EntityBase::default(), &c, 0.0);
        assert_eq!(loaded.flags, s.flags);
        assert_eq!(loaded.pose, s.pose);
        assert_eq!(loaded.equipment, s.equipment);
        assert_eq!(loaded.disabled_slots, s.disabled_slots);
        assert_eq!(loaded.health, 12.5);
        // A loaded marker shrinks on its first update.
        assert!(!loaded.sized);
    }

    #[test]
    fn entity_tags_merge_into_compounds() {
        let mut into = Compound::new();
        let mut pose = Compound::new();
        pose.insert("Body", Tag::Byte(1));
        into.insert("Pose", Tag::Compound(pose));
        into.insert("Small", Tag::Byte(0));
        let mut from = Compound::new();
        let mut head = Compound::new();
        head.insert("Head", Tag::Byte(2));
        from.insert("Pose", Tag::Compound(head));
        from.insert("Small", Tag::Byte(1));
        merge(&mut into, &from);
        let pose = into.get_compound("Pose").cloned().unwrap_or_default();
        assert_eq!(pose.get_i8("Body"), 1);
        assert_eq!(pose.get_i8("Head"), 2);
        assert_eq!(into.get_i8("Small"), 1);
    }
}
