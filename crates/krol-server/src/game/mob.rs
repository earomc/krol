//! Mobs (vanilla: `EntityLiving` on top of `EntityLivingBase`, with `EntityAgeable`,
//! `EntityAnimal` and the mob classes themselves).
//!
//! A mob runs the base and living entity updates: fire, lava, the void, water, air and
//! drowning, suffocation, the hurt and death animations, its ambient sound (`livingSound`
//! on its own random) and the ageable countdown. Players hurt it with their attacks
//! (knockback, hurt and death sounds); a dead mob drops its loot at once and its
//! experience when the death animation ends, 20 ticks later.
//!
//! A mob with AI (not `NoAI`) also thinks and moves: its goals, navigation and the
//! move, look, jump and body helpers ([`ai`]), then `moveEntityWithHeading` (walking,
//! swimming, ladders, falls and fall damage, step sounds). Every mob pushes and is
//! pushed by the players and mobs around it.
//!
//! Every mob has its own `java.util.Random` (`Entity.rand`), unseeded in vanilla; the
//! parity hook `krolpin` seeds it with the pin seed plus the entity id, like the vanilla
//! agent does.
//!
//! Players feed, saddle and name mobs, and animals in love breed ([`breeding`]). Spawn
//! eggs hatch mobs on blocks and in liquids ([`spawn_egg`]).
//!
//! TODO(M5): the other mob kinds (and their `onInitialSpawn`), horses, picking up
//! loot, status effects (`ActiveEffects` is kept as saved), jump boost, the combat
//! tracker, being hit by projectiles and explosions, arrows stuck in them, equipment
//! changes.

mod ai;
mod breeding;
mod leash;
mod spawn_egg;

use std::collections::BTreeSet;

use krol_java::JavaRandom;
use krol_java::math::{ceil_f32, cos, floor_f64, sin, sqrt_f32};
use krol_nbt::{Compound, List, Tag, TagType};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack, Metadata, MetadataValue, Property};

use super::Game;
use super::attributes::{KNOCKBACK_RESISTANCE, MAX_HEALTH, MOVEMENT_SPEED, Modifier};
use super::damage::{DamageSource, MAX_HURT_TIME};
use super::ids::{block, item};
use super::physics::{self, Body};
use super::saved_entities::{EntityBase, FULL_AIR};

/// `generic.followRange` (`EntityLiving`).
const FOLLOW_RANGE: &str = "generic.followRange";
/// `onInitialSpawn`'s follow range modifier: its name, the spread of its gaussian
/// amount and its operation (add a multiple of the base).
const SPAWN_BONUS_NAME: &str = "Random spawn bonus";
const SPAWN_BONUS_SPREAD: f64 = 0.05;
const ADD_MULTIPLE_OF_BASE: i8 = 1;
/// `EntityLivingBase.stepHeight`.
const STEP_HEIGHT: f32 = 0.6;
/// `maxHurtResistantTime`: hits within half of it only hurt by what they add.
const MAX_HURT_RESISTANT_TIME: i32 = 20;
/// The death animation lasts this many ticks before the mob is removed.
const DEATH_TICKS: i16 = 20;
/// `recentlyHit` after a player's hit: loot and experience need it to be running.
const RECENTLY_HIT: i32 = 100;
/// Entities below this height are killed (`Entity.onEntityUpdate`).
const VOID_Y: f64 = -64.0;
/// Lava sets entities on fire for this long (`setOnFireFromLava`) and hurts by this.
const LAVA_SECONDS: i32 = 15;
const LAVA_DAMAGE: f32 = 4.0;
/// Damage of burning (every second), drowning and suffocating.
const ON_FIRE_DAMAGE: f32 = 1.0;
const DROWN_DAMAGE: f32 = 2.0;
const IN_WALL_DAMAGE: f32 = 1.0;
/// The air value at which a drowning entity is hurt and its air reset to 0.
const DROWN_AT: i16 = -20;
/// Tiny motion stops (`onLivingUpdate`).
const MOTION_EPSILON: f64 = 0.005;
/// A mob without AI only slows down (`onLivingUpdate`, not a server world).
const NO_AI_DRAG: f64 = 0.98;
/// `EntityLivingBase.knockBack`'s strength, also the cap on the upward motion.
const KNOCKBACK: f32 = 0.4;
/// The chance of `addRandomDrop` after a player's kill, plus this per looting level.
const RARE_DROP_CHANCE: f32 = 0.025;
const RARE_DROP_LOOTING: f32 = 0.01;
/// `EntityLiving.equipmentDropChances` of a new mob.
const DEFAULT_DROP_CHANCE: f32 = 0.085;
/// The equipment slots: held item, boots, leggings, chestplate, helmet.
const SLOTS: usize = 5;
/// `EntityItem.setDefaultPickupDelay`.
const DROP_PICKUP_DELAY: i32 = 10;
/// `moveEntityWithHeading`: gravity, vertical drag and the friction in the air.
const GRAVITY: f64 = 0.08;
const DRAG: f32 = 0.98;
const AIR_FRICTION: f32 = 0.91;
/// The ground acceleration is the move speed times this over the friction cubed.
const GROUND_ACCELERATION: f32 = 0.162_771_36;
/// `jumpMovementFactor`: the acceleration in the air.
const JUMP_MOVEMENT_FACTOR: f32 = 0.02;
/// In water: the friction, the acceleration and the gravity (also in lava).
const WATER_FRICTION: f32 = 0.8;
const WATER_ACCELERATION: f32 = 0.02;
const LIQUID_GRAVITY: f64 = 0.02;
/// Depth strider levels count up to this; at full strength the water friction becomes
/// the second value.
const MAX_DEPTH_STRIDER: f32 = 3.0;
const STRIDER_FRICTION: f32 = 0.546_000_06;
/// Swimming against a ledge: the room checked above and the hop out.
const LEDGE_CHECK_LIFT: f32 = 0.6;
const LEDGE_JUMP: f32 = 0.3;
/// Ladders cap the speed and climb at this.
const LADDER_SPEED: f64 = 0.15;
const LADDER_CLIMB: f64 = 0.2;
/// Jumping and floating up (`getJumpUpwardsMotion`, `updateAITick`).
const JUMP_MOTION: f32 = 0.42;
const SWIM_UP: f32 = 0.04;
/// Ticks between jumps while the mob keeps jumping.
const JUMP_COOLDOWN: i32 = 10;
/// Falls hurt from this far on.
const SAFE_FALL: f32 = 3.0;
/// Standing in fire: the damage, and the burning after a while.
const IN_FIRE_DAMAGE: f32 = 1.0;
const FIRE_BLOCK_SECONDS: i32 = 8;
/// `Entity.fireResistance`.
const FIRE_RESISTANCE: i16 = 1;
/// `BlockFarmland.onFallenUpon` tramples from this far above half a block.
const TRAMPLE_OFFSET: f32 = 0.5;
/// The revenge target is forgotten this many ticks after the hit.
const REVENGE_TICKS: i32 = 100;
/// `S2APacketParticles` id of `BLOCK_DUST`.
const BLOCK_DUST: i32 = 38;
const MOB_GRIEFING: &str = "mobGriefing";

/// `S19PacketEntityStatus` statuses.
const STATUS_HURT: i8 = 2;
const STATUS_DEATH: i8 = 3;

/// Metadata indices (`DataWatcher`).
const FLAGS_INDEX: u8 = 0;
const CUSTOM_NAME_INDEX: u8 = 2;
const AIR_INDEX: u8 = 1;
const HEALTH_INDEX: u8 = 6;
const POTION_COLOR_INDEX: u8 = 7;
const POTION_AMBIENT_INDEX: u8 = 8;
const ARROWS_INDEX: u8 = 9;
const AGE_INDEX: u8 = 12;
const NO_AI_INDEX: u8 = 15;
const SADDLE_INDEX: u8 = 16;
/// The burning bit of the flags.
const BURNING_BIT: i8 = 1;

/// The kinds of mobs krol simulates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobKind {
    /// `EntityPig`.
    Pig,
    /// `EntityCow`.
    Cow,
}

impl MobKind {
    /// The kind saved under `id` (`EntityList` names).
    pub fn from_saved_id(id: &str) -> Option<Self> {
        match id {
            "Pig" => Some(Self::Pig),
            "Cow" => Some(Self::Cow),
            _ => None,
        }
    }

    /// The kind with `EntityList` id `id` (a spawn egg's damage).
    pub fn from_type_id(id: i32) -> Option<Self> {
        [Self::Pig, Self::Cow]
            .into_iter()
            .find(|k| i32::from(k.type_id()) == id)
    }

    /// The saved id (`EntityList` name).
    pub fn saved_id(self) -> &'static str {
        match self {
            Self::Pig => "Pig",
            Self::Cow => "Cow",
        }
    }

    /// The `EntityList` id, the mob type of `S0FPacketSpawnMob`.
    pub fn type_id(self) -> u8 {
        match self {
            Self::Pig => 90,
            Self::Cow => 92,
        }
    }

    /// The en_US `entity.<id>.name`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Pig => "Pig",
            Self::Cow => "Cow",
        }
    }

    /// `setSize` of an adult.
    fn size(self) -> (f32, f32) {
        match self {
            Self::Pig => (0.9, 0.9),
            Self::Cow => (0.9, 1.3),
        }
    }

    /// The bases of the mob's attributes (`applyEntityAttributes`), in registration
    /// order: max health, knockback resistance, movement speed, follow range.
    fn attribute_bases(self) -> [(&'static str, f64); 4] {
        match self {
            Self::Pig => [
                (MAX_HEALTH, 10.0),
                (KNOCKBACK_RESISTANCE, 0.0),
                (MOVEMENT_SPEED, 0.25),
                (FOLLOW_RANGE, 16.0),
            ],
            Self::Cow => [
                (MAX_HEALTH, 10.0),
                (KNOCKBACK_RESISTANCE, 0.0),
                // Set from a float.
                (MOVEMENT_SPEED, 0.2f32 as f64),
                (FOLLOW_RANGE, 16.0),
            ],
        }
    }

    /// `getLivingSound`, `getHurtSound`, `getDeathSound`.
    fn sounds(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Pig => ("mob.pig.say", "mob.pig.say", "mob.pig.death"),
            Self::Cow => ("mob.cow.say", "mob.cow.hurt", "mob.cow.hurt"),
        }
    }

    /// `getSoundVolume`.
    fn sound_volume(self) -> f32 {
        match self {
            Self::Pig => 1.0,
            Self::Cow => 0.4,
        }
    }

    /// `playStepSound`: the sound and its volume.
    fn step_sound(self) -> (&'static str, f32) {
        match self {
            Self::Pig => ("mob.pig.step", 0.15),
            Self::Cow => ("mob.cow.step", 0.15),
        }
    }

    /// `getTalkInterval`: animals talk less often than other mobs.
    fn talk_interval(self) -> i32 {
        match self {
            Self::Pig | Self::Cow => 120,
        }
    }

    /// Not an `IMob` (`EntityLiving.allowLeashing`): it takes a lead.
    fn leashable(self) -> bool {
        match self {
            Self::Pig | Self::Cow => true,
        }
    }

    /// An `EntityAnimal` (or water mob): removed when `spawn-animals` is off.
    fn animal(self) -> bool {
        match self {
            Self::Pig | Self::Cow => true,
        }
    }

    /// An `EntityAgeable` (an age entry in its metadata, children).
    fn ageable(self) -> bool {
        match self {
            Self::Pig | Self::Cow => true,
        }
    }
}

/// The attributes clients are told about (`shouldWatch`), in the order the tracker
/// sends them: movement speed, then max health.
const WATCHED: [usize; 2] = [2, 0];

/// One mob.
#[derive(Debug, Clone)]
pub struct Mob {
    /// What kind of mob it is.
    pub kind: MobKind,
    /// Position, motion, size and the movement state.
    pub body: Body,
    /// The base entity state.
    pub base: EntityBase,
    /// `Entity.rand`: the mob's own random.
    pub rand: JavaRandom,
    /// `rotationYawHead`.
    pub head_yaw: f32,
    /// `ticksExisted`.
    ticks_existed: i32,
    /// `getHealth` (metadata entry 6).
    health: f32,
    /// `getAbsorptionAmount`.
    absorption: f32,
    /// `hurtTime`: the hurt animation counts down from 10.
    hurt_time: i16,
    /// `hurtResistantTime`: further hits only hurt by what they add while above 10.
    hurt_resistant_time: i32,
    /// `lastDamage`.
    last_damage: f32,
    /// `deathTime`: ticks since the mob died.
    death_time: i16,
    /// `revengeTimer` (`HurtByTimestamp`): when the mob was last attacked.
    hurt_by_timestamp: i32,
    /// `recentlyHit`: ticks left in which a player's kill drops loot and experience.
    recently_hit: i32,
    /// `attackingPlayer`: the entity id of the player who hit it last.
    attacking_player: Option<i32>,
    /// `livingSoundTime`: the ambient sound becomes likelier every tick.
    living_sound_time: i32,
    /// The attribute bases, in [`MobKind::attribute_bases`] order.
    attributes: [f64; 4],
    /// The attribute modifiers (all saved).
    modifiers: Vec<Modifier>,
    /// `ActiveEffects`, kept as saved.
    effects: Option<List>,
    /// What it holds and wears.
    pub equipment: [Option<ItemStack>; SLOTS],
    /// `equipmentDropChances`.
    drop_chances: [f32; SLOTS],
    /// `canPickUpLoot`.
    can_pick_up_loot: bool,
    /// `persistenceRequired`.
    persistence_required: bool,
    /// `isLeashed`.
    leashed: bool,
    /// `leashNBTTag`: the loaded `Leash` tag, until the first update finds its holder.
    leash: Option<Compound>,
    /// `leashedToEntity`: the entity id of the player or leash knot holding the lead.
    leash_holder: Option<i32>,
    /// `EntityCreature.homePosition` and `maximumHomeDistance` while it has a home
    /// (a leashed mob stays near its holder).
    home: Option<((i32, i32, i32), f32)>,
    /// `isMovementAITaskSet`: the leash's `EntityAIMoveTowardsRestriction` was added.
    movement_task_set: bool,
    /// The `Leash` tag of the holder (`UUIDMost`/`UUIDLeast` of a player, `X`/`Y`/`Z`
    /// of a knot), kept up to date by the game while the mob is leashed.
    pub saved_holder: Option<Compound>,
    /// `isAIDisabled` (metadata entry 15).
    pub no_ai: bool,
    /// `EntityAgeable.growingAge` (metadata entry 12 holds its sign) and
    /// `field_175502_b` (`ForcedAge`).
    age: i32,
    forced_age: i32,
    /// `EntityAnimal.inLove`.
    in_love: i32,
    /// `EntityAnimal.playerInLove`: the entity id of the player who fed it last.
    player_in_love: Option<i32>,
    /// `EntityPig` saddle (metadata entry 16).
    saddled: bool,
    /// The burning flag as last set in the metadata.
    burning_flag: bool,
    /// `firstUpdate`.
    first_update: bool,
    /// Metadata entries changed since the tracker last sent them (all at first).
    pub dirty_metadata: BTreeSet<u8>,
    /// Some metadata changed since the tracker last looked (`objectChanged`; setting
    /// the health in the constructor makes it true from the start).
    pub metadata_changed: bool,
    /// Watched attributes changed since the tracker last sent them (the constructor
    /// sets the max health and speed, so they start out changed).
    pub dirty_attributes: bool,
    /// `isAirBorne`: knocked back, so the tracker updates it at once.
    pub airborne: bool,
    /// `isDead`: removed at its next turn in the entity loop.
    pub dead: bool,
    /// The chunk the tracker checks viewers against (`chunkCoordX/Z`).
    pub chunk: (i32, i32),
    /// `renderYawOffset`: the body's yaw.
    body_yaw: f32,
    /// `prevPosX/Y/Z`: where the mob was when its update began.
    prev_pos: (f64, f64, f64),
    /// `moveForward`, `moveStrafing`: the inputs of `moveEntityWithHeading`.
    pub(super) move_forward: f32,
    move_strafing: f32,
    /// `landMovementFactor` (`getAIMoveSpeed`).
    ai_move_speed: f32,
    /// `isJumping`, `jumpTicks`.
    jumping: bool,
    jump_ticks: i32,
    /// `entityAge`: ticks since a player was near or the mob was hurt.
    entity_age: i32,
    /// `entityLivingToAttack` (`getAITarget`): who hurt it last.
    revenge_target: Option<i32>,
    /// The AI (taken out while it runs).
    ai: Option<Box<ai::MobAi>>,
    /// `riddenByEntity`: the entity id of the player riding it.
    pub rider: Option<i32>,
    /// `ridingEntity`: the entity id of the vehicle it rides (a minecart that picked
    /// it up, or a mount loaded or summoned with it).
    pub riding: Option<i32>,
}

fn chunk_of(pos: (f64, f64, f64)) -> (i32, i32) {
    (floor_f64(pos.0) >> 4, floor_f64(pos.2) >> 4)
}

impl Mob {
    /// A new mob (the constructors), its head turned to `head_yaw` by the living
    /// constructor, with its own random seeded with `seed`.
    pub fn new(kind: MobKind, pos: (f64, f64, f64), head_yaw: f32, seed: i64) -> Self {
        let mut body = Body::player(pos, false);
        let (width, height) = kind.size();
        body.width = width;
        body.height = height;
        body.step_height = STEP_HEIGHT;
        let bases = kind.attribute_bases();
        let mut m = Self {
            kind,
            body,
            base: EntityBase::facing(head_yaw),
            rand: JavaRandom::new(seed),
            head_yaw,
            ticks_existed: 0,
            health: bases[0].1 as f32,
            absorption: 0.0,
            hurt_time: 0,
            hurt_resistant_time: 0,
            last_damage: 0.0,
            death_time: 0,
            hurt_by_timestamp: 0,
            recently_hit: 0,
            attacking_player: None,
            living_sound_time: 0,
            attributes: bases.map(|(_, b)| b),
            modifiers: Vec::new(),
            effects: None,
            equipment: Default::default(),
            drop_chances: [DEFAULT_DROP_CHANCE; SLOTS],
            can_pick_up_loot: false,
            persistence_required: false,
            leashed: false,
            leash: None,
            leash_holder: None,
            home: None,
            movement_task_set: false,
            saved_holder: None,
            no_ai: false,
            age: 0,
            forced_age: 0,
            in_love: 0,
            player_in_love: None,
            saddled: false,
            burning_flag: false,
            first_update: true,
            dirty_metadata: BTreeSet::new(),
            metadata_changed: true,
            dirty_attributes: true,
            airborne: false,
            dead: false,
            chunk: chunk_of(pos),
            body_yaw: 0.0,
            prev_pos: pos,
            move_forward: 0.0,
            move_strafing: 0.0,
            ai_move_speed: 0.0,
            jumping: false,
            jump_ticks: 0,
            entity_age: 0,
            revenge_target: None,
            rider: None,
            riding: None,
            ai: Some(Box::new(match kind {
                MobKind::Pig => ai::MobAi::pig(pos),
                MobKind::Cow => ai::MobAi::cow(pos),
            })),
        };
        m.dirty_metadata = m.metadata().iter().map(|e| e.index).collect();
        m
    }

    /// `readEntityFromNBT` of a mob (`body` and `base` already read): the living
    /// state, then `EntityLiving`'s, the ageable and animal state and the mob's own.
    pub fn load(
        kind: MobKind,
        body: Body,
        base: EntityBase,
        c: &Compound,
        head_yaw: f32,
        seed: i64,
    ) -> Self {
        let mut m = Self::new(kind, body.pos, head_yaw, seed);
        let (width, height, step) = (m.body.width, m.body.height, m.body.step_height);
        m.body = body;
        m.body.width = width;
        m.body.height = height;
        m.body.step_height = step;
        m.base = base;
        // Entity.readFromNBT turns the head and body to the saved yaw.
        m.head_yaw = m.base.yaw;
        m.body_yaw = m.base.yaw;
        if m.base.air != FULL_AIR {
            m.metadata_updated(AIR_INDEX);
        }
        // EntityLivingBase.readEntityFromNBT.
        m.absorption = c.get_f32("AbsorptionAmount").max(0.0);
        if let Some(list) = c.get_list("ActiveEffects", TagType::Compound) {
            m.effects = (!list.is_empty()).then(|| list.clone());
        }
        if let Some(list) = c.get_list("Attributes", TagType::Compound) {
            let bases = kind.attribute_bases();
            for a in list.iter() {
                let Tag::Compound(a) = a else { continue };
                let name = a.get_str("Name");
                let Some(k) = bases.iter().position(|(n, _)| *n == name) else {
                    tracing::warn!("Ignoring unknown attribute '{name}'");
                    continue;
                };
                m.attributes[k] = a.get_f64("Base");
                let attribute = bases[k].0;
                for modifier in a
                    .get_list("Modifiers", TagType::Compound)
                    .iter()
                    .flat_map(|l| l.iter())
                {
                    let Tag::Compound(modifier) = modifier else {
                        continue;
                    };
                    let modifier = super::attributes::read_modifier(attribute, modifier);
                    super::attributes::apply_modifier(&mut m.modifiers, modifier);
                }
            }
        }
        let health = match c.get("HealF").filter(|t| t.as_f32().is_some()) {
            Some(t) => t.as_f32().unwrap_or(0.0),
            None => match c.get("Health") {
                None => m.max_health(),
                Some(Tag::Float(v)) => *v,
                Some(Tag::Short(v)) => f32::from(*v),
                Some(_) => m.health,
            },
        };
        m.set_health(health);
        m.hurt_time = c.get_i16("HurtTime");
        m.death_time = c.get_i16("DeathTime");
        m.hurt_by_timestamp = c.get_i32("HurtByTimestamp");
        // EntityLiving.readEntityFromNBT.
        if c.contains_key_of("CanPickUpLoot", Some(TagType::Byte)) {
            m.can_pick_up_loot = c.get_bool("CanPickUpLoot");
        }
        m.persistence_required = c.get_bool("PersistenceRequired");
        if let Some(list) = c.get_list("Equipment", TagType::Compound) {
            for (k, slot) in m.equipment.iter_mut().enumerate() {
                *slot = list.get(k).and_then(|t| match t {
                    Tag::Compound(it) => super::player::item_from_nbt(it),
                    _ => None,
                });
            }
        }
        if let Some(list) = c.get_list("DropChances", TagType::Float) {
            for (k, chance) in list.iter().enumerate().take(SLOTS) {
                m.drop_chances[k] = chance.as_f32().unwrap_or(0.0);
            }
        }
        m.leashed = c.get_bool("Leashed");
        if m.leashed {
            m.leash = c.get_compound("Leash").cloned();
        }
        m.set_no_ai(c.get_bool("NoAI"));
        // EntityAgeable, EntityAnimal, EntityPig.
        if kind.ageable() {
            m.set_age(c.get_i32("Age"));
            m.forced_age = c.get_i32("ForcedAge");
            m.in_love = c.get_i32("InLove");
        }
        match kind {
            MobKind::Pig => m.set_saddled(c.get_bool("Saddle")),
            MobKind::Cow => {}
        }
        m.chunk = chunk_of(m.body.pos);
        m
    }

    /// `writeEntityToNBT` of a mob.
    pub fn write(&self, c: &mut Compound) {
        // EntityLivingBase.
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
        for ((name, _), base) in self.kind.attribute_bases().iter().zip(&self.attributes) {
            let mut a = Compound::new();
            a.insert("Name", Tag::String((*name).to_owned()));
            a.insert("Base", Tag::Double(*base));
            let modifiers = super::attributes::modifiers_of(name, &self.modifiers);
            if !modifiers.is_empty() {
                let mut list = List::new();
                for m in modifiers {
                    let _ = list.push(Tag::Compound(super::attributes::modifier_nbt(m)));
                }
                a.insert("Modifiers", Tag::List(list));
            }
            let _ = attributes.push(Tag::Compound(a));
        }
        c.insert("Attributes", Tag::List(attributes));
        if let Some(effects) = &self.effects {
            c.insert("ActiveEffects", Tag::List(effects.clone()));
        }
        // EntityLiving.
        c.insert("CanPickUpLoot", Tag::Byte(i8::from(self.can_pick_up_loot)));
        c.insert(
            "PersistenceRequired",
            Tag::Byte(i8::from(self.persistence_required)),
        );
        let mut equipment = List::new();
        for slot in &self.equipment {
            let item = slot
                .as_ref()
                .map(super::item_data::stack_nbt)
                .unwrap_or_default();
            let _ = equipment.push(Tag::Compound(item));
        }
        c.insert("Equipment", Tag::List(equipment));
        let mut chances = List::new();
        for chance in self.drop_chances {
            let _ = chances.push(Tag::Float(chance));
        }
        c.insert("DropChances", Tag::List(chances));
        c.insert("Leashed", Tag::Byte(i8::from(self.leashed)));
        // Vanilla quirk: only a found holder is saved, so a mob saved before its first
        // update loses the loaded `Leash` tag (and drops the lead when loaded again).
        if let Some(holder) = &self.saved_holder {
            c.insert("Leash", Tag::Compound(holder.clone()));
        }
        if self.no_ai {
            c.insert("NoAI", Tag::Byte(1));
        }
        // EntityAgeable, EntityAnimal, EntityPig.
        if self.kind.ageable() {
            c.insert("Age", Tag::Int(self.age));
            c.insert("ForcedAge", Tag::Int(self.forced_age));
            c.insert("InLove", Tag::Int(self.in_love));
        }
        match self.kind {
            MobKind::Pig => {
                c.insert("Saddle", Tag::Byte(i8::from(self.saddled)));
            }
            MobKind::Cow => {}
        }
    }

    /// `getName`.
    pub fn name(&self) -> String {
        self.base
            .custom_name
            .clone()
            .unwrap_or_else(|| self.kind.name().to_owned())
    }

    /// `isEntityAlive`.
    pub fn alive(&self) -> bool {
        !self.dead && self.health > 0.0
    }

    /// `isBurning`.
    fn burning(&self) -> bool {
        self.base.fire > 0
    }

    /// `isChild`.
    fn child(&self) -> bool {
        self.kind.ageable() && self.age < 0
    }

    /// `getEyeHeight`.
    pub fn eye_height(&self) -> f32 {
        match self.kind {
            MobKind::Pig => self.body.height * 0.85,
            MobKind::Cow => self.body.height,
        }
    }

    /// The value of attribute `k` (in [`MobKind::attribute_bases`] order) with its
    /// modifiers.
    fn attribute(&self, k: usize) -> f64 {
        let name = self.kind.attribute_bases()[k].0;
        super::attributes::compute(name, self.attributes[k], &self.modifiers)
    }

    fn max_health(&self) -> f32 {
        self.attribute(0) as f32
    }

    fn knockback_resistance(&self) -> f64 {
        self.attribute(1)
    }

    fn movement_speed(&self) -> f64 {
        self.attribute(2)
    }

    fn follow_range(&self) -> f64 {
        self.attribute(3)
    }

    /// `EntityLiving.onInitialSpawn`: a mob spawned anew (not from saved data) gets a
    /// random follow range bonus, drawn from its own random. The modifier's UUID comes
    /// from `uuid_rand` (vanilla: `ThreadLocalRandom`).
    pub(super) fn initial_spawn(&mut self, uuid_rand: &mut JavaRandom) {
        let amount = self.rand.next_gaussian() * SPAWN_BONUS_SPREAD;
        let uuid = super::attributes::random_uuid(uuid_rand);
        let modifier = Modifier {
            attribute: FOLLOW_RANGE,
            uuid,
            name: SPAWN_BONUS_NAME.to_owned(),
            amount,
            operation: ADD_MULTIPLE_OF_BASE,
            saved: true,
        };
        super::attributes::apply_modifier(&mut self.modifiers, modifier);
    }

    /// `DataWatcher.updateObject` of entry `index` with a changed value.
    fn metadata_updated(&mut self, index: u8) {
        self.dirty_metadata.insert(index);
        self.metadata_changed = true;
    }

    /// `setHealth`: clamped to the max health.
    fn set_health(&mut self, health: f32) {
        let health = health.clamp(0.0, self.max_health());
        if health != self.health {
            self.health = health;
            self.metadata_updated(HEALTH_INDEX);
        }
    }

    fn set_air(&mut self, air: i16) {
        if air != self.base.air {
            self.base.air = air;
            self.metadata_updated(AIR_INDEX);
        }
    }

    fn set_no_ai(&mut self, no_ai: bool) {
        if no_ai != self.no_ai {
            self.no_ai = no_ai;
            self.metadata_updated(NO_AI_INDEX);
        }
    }

    /// `EntityAgeable.setGrowingAge` (the metadata holds the sign; children are half
    /// the size). The box changes size on its lower corner (`Entity.setSize`); a mob
    /// that grew after its first update then moves back by the growth on both
    /// horizontal axes, which this returns for the caller to do with
    /// [`Game::move_mob_by`].
    fn set_age(&mut self, age: i32) -> Option<f64> {
        let old = self.age.clamp(-1, 1);
        self.age = age;
        if age.clamp(-1, 1) != old {
            self.metadata_updated(AGE_INDEX);
        }
        let (width, height) = self.kind.size();
        let scale = if self.child() { 0.5 } else { 1.0 };
        let (width, height) = (width * scale, height * scale);
        let before = self.body.set_size(width, height);
        (width > before && !self.first_update).then(|| f64::from(before - width))
    }

    fn set_saddled(&mut self, saddled: bool) {
        if saddled != self.saddled {
            self.saddled = saddled;
            self.metadata_updated(SADDLE_INDEX);
        }
    }

    /// `setFire`: the longer of the current fire and `seconds`.
    fn set_fire(&mut self, seconds: i32) {
        // TODO(M5): fire protection on armor shortens it.
        let ticks = seconds * 20;
        if i32::from(self.base.fire) < ticks {
            self.base.fire = ticks as i16;
        }
    }

    /// `EnchantmentHelper.getMaxEnchantmentLevel` over what the mob holds and wears.
    fn max_enchantment(&self, id: i16) -> i32 {
        self.equipment
            .iter()
            .map(|s| super::item_data::enchantment_level(s.as_ref(), id))
            .max()
            .unwrap_or(0)
    }

    /// `getSoundPitch`: children squeak higher.
    fn sound_pitch(&mut self) -> f32 {
        let base = if self.child() { 1.5 } else { 1.0 };
        (self.rand.next_float() - self.rand.next_float()) * 0.2 + base
    }

    /// The metadata of every watched entry, in the data watcher's `HashMap` order
    /// (16 buckets: 16 shares the bucket of 0, after it).
    pub fn metadata(&self) -> Vec<Metadata> {
        use MetadataValue::{Byte, Float, Int};
        let md = |index, value| Metadata { index, value };
        let mut out = self.base.metadata();
        out[FLAGS_INDEX as usize] = md(
            FLAGS_INDEX,
            Byte(if self.burning_flag { BURNING_BIT } else { 0 }),
        );
        out.push(md(HEALTH_INDEX, Float(self.health)));
        out.push(md(POTION_COLOR_INDEX, Int(0)));
        out.push(md(POTION_AMBIENT_INDEX, Byte(0)));
        out.push(md(ARROWS_INDEX, Byte(0)));
        if self.kind.ageable() {
            out.push(md(AGE_INDEX, Byte(self.age.clamp(-1, 1) as i8)));
        }
        out.push(md(NO_AI_INDEX, Byte(i8::from(self.no_ai))));
        match self.kind {
            MobKind::Pig => {
                out.insert(1, md(SADDLE_INDEX, Byte(i8::from(self.saddled))));
            }
            MobKind::Cow => {}
        }
        out
    }

    /// The watched attributes the tracker sends.
    pub fn properties(&self) -> Vec<Property> {
        let bases = self.kind.attribute_bases();
        WATCHED
            .iter()
            .map(|&k| {
                let modifiers = super::attributes::modifiers_of(bases[k].0, &self.modifiers)
                    .into_iter()
                    .map(|m| (m.uuid, m.amount, m.operation))
                    .collect();
                (bases[k].0.to_owned(), self.attributes[k], modifiers)
            })
            .collect()
    }
}

impl Game {
    /// A new mob's own random, unseeded in vanilla (`new Random()`).
    pub(super) fn new_mob_seed(&mut self) -> i64 {
        self.entity_rand.next_long()
    }

    /// One mob's turn in the entity loop (`World.updateEntities`): a rider waits for
    /// its vehicle (one whose vehicle is gone or has let go of it gets off silently);
    /// otherwise its update unless dead, then its removal if dead.
    pub(super) fn update_mob(&mut self, id: i32) {
        if let Some(vehicle) = self.mobs.get(&id).and_then(|m| m.riding) {
            if !self.vehicle_dead(vehicle) && self.vehicle_rider(vehicle) == Some(id) {
                return;
            }
            if self.vehicle_rider(vehicle) == Some(id) {
                self.set_vehicle_rider(vehicle, None);
            }
            if let Some(m) = self.mobs.get_mut(&id) {
                m.riding = None;
            }
        }
        self.update_mob_entity(id);
        if self.mobs.get(&id).is_some_and(|m| m.dead) {
            if let Some(m) = self.mobs.remove(&id)
                && let Some(rider) = m.rider
            {
                self.let_go_of_rider(id, rider);
            }
            self.remove_entity_from_chunk(id);
            self.untrack_entity(id);
        }
    }

    /// `World.updateEntityWithOptionalForce` for a mob that is alive: its update
    /// (`updateRidden` while it rides), its chunk, then its own rider. A rider runs
    /// this right after its vehicle.
    pub(super) fn update_mob_entity(&mut self, id: i32) {
        if !self.mobs.get(&id).is_some_and(|m| !m.dead) {
            return;
        }
        // WorldServer: animals go when the server does not spawn them, after one more
        // update.
        let spawn_animals = self.config.spawn_animals;
        let mut riding = None;
        if let Some(m) = self.mobs.get_mut(&id) {
            if !spawn_animals && m.kind.animal() {
                m.dead = true;
            }
            m.ticks_existed += 1;
            riding = m.riding;
        }
        match riding {
            Some(vehicle) => self.update_ridden_mob(id, vehicle),
            None => self.mob_on_update(id),
        }
        if let Some(m) = self.mobs.get_mut(&id) {
            m.chunk = chunk_of(m.body.pos);
        }
        self.relocate_entity(id);
        self.update_vehicle_rider(id);
    }

    /// `EntityLiving.onUpdate`. The update runs to its end even if the mob dies on
    /// the way.
    fn mob_on_update(&mut self, id: i32) {
        self.mob_base_update(id);
        self.mob_living_update(id);
        self.mob_body_update(id);
        // EntityLiving.onUpdate: the lead, after the living entity's update.
        self.update_leashed_state(id);
    }

    /// `EntityLivingBase.updateRidden`: a dead vehicle is let go (no update this
    /// tick); otherwise the mob updates from rest, sits on its vehicle again and
    /// forgets its fall.
    fn update_ridden_mob(&mut self, id: i32, vehicle: i32) {
        if self.vehicle_dead(vehicle) {
            if let Some(m) = self.mobs.get_mut(&id) {
                m.riding = None;
            }
            return;
        }
        if let Some(m) = self.mobs.get_mut(&id) {
            m.body.motion = (0.0, 0.0, 0.0);
        }
        self.mob_on_update(id);
        if let Some(vehicle) = self.mobs.get(&id).and_then(|m| m.riding) {
            self.update_rider_position(vehicle);
        }
        if let Some(m) = self.mobs.get_mut(&id) {
            m.base.fall_distance = 0.0;
        }
    }

    /// `EntityLiving.onEntityUpdate`: `Entity.onEntityUpdate`, the living part of
    /// `EntityLivingBase.onEntityUpdate`, then the ambient sound.
    fn mob_base_update(&mut self, id: i32) {
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        m.prev_pos = m.body.pos;
        if m.base.portal_cooldown > 0 {
            m.base.portal_cooldown -= 1;
        }
        self.mob_water_movement(id);
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        if m.base.fire > 0 {
            if m.base.fire % 20 == 0 {
                self.attack_mob(id, DamageSource::OnFire, ON_FIRE_DAMAGE);
            }
            if let Some(m) = self.mobs.get_mut(&id) {
                m.base.fire -= 1;
            }
        }
        if self.mob_in_lava(id) {
            // setOnFireFromLava.
            self.attack_mob(id, DamageSource::Lava, LAVA_DAMAGE);
            if let Some(m) = self.mobs.get_mut(&id) {
                m.set_fire(LAVA_SECONDS);
                m.base.fall_distance *= 0.5;
            }
        }
        if self.mobs.get(&id).is_some_and(|m| m.body.pos.1 < VOID_Y) {
            // EntityLivingBase.kill.
            self.attack_mob(id, DamageSource::OutOfWorld, f32::MAX);
        }
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        let burning = m.base.fire > 0;
        if burning != m.burning_flag {
            m.burning_flag = burning;
            m.metadata_updated(FLAGS_INDEX);
        }
        m.first_update = false;
        // EntityLivingBase.onEntityUpdate.
        if m.alive() && self.mob_inside_opaque_block(id) {
            self.attack_mob(id, DamageSource::InWall, IN_WALL_DAMAGE);
        }
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        if m.alive() {
            let (pos, eye) = (m.body.pos, m.eye_height());
            if self.eyes_in_water_at(pos, eye) {
                let Some(m) = self.mobs.get_mut(&id) else {
                    return;
                };
                // decreaseAirSupply: respiration may save the air.
                let respiration = m.max_enchantment(super::item_data::RESPIRATION);
                let mut air = m.base.air;
                if !(respiration > 0 && m.rand.next_int_bounded(respiration + 1) > 0) {
                    air -= 1;
                }
                m.set_air(air);
                if m.base.air == DROWN_AT {
                    m.set_air(0);
                    // The bubbles are client-side but draw from the random.
                    for _ in 0..8 * 6 {
                        let _ = m.rand.next_float();
                    }
                    self.attack_mob(id, DamageSource::Drown, DROWN_DAMAGE);
                }
            } else if let Some(m) = self.mobs.get_mut(&id) {
                m.set_air(FULL_AIR);
            }
        }
        if self.mobs.get(&id).is_some_and(Mob::alive) && self.mob_wet(id) {
            if let Some(m) = self.mobs.get_mut(&id) {
                m.base.fire = 0;
            }
        }
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        if m.hurt_time > 0 {
            m.hurt_time -= 1;
        }
        if m.hurt_resistant_time > 0 {
            m.hurt_resistant_time -= 1;
        }
        if m.health <= 0.0 {
            self.mob_death_update(id);
        }
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        if m.recently_hit > 0 {
            m.recently_hit -= 1;
        } else {
            m.attacking_player = None;
        }
        if let Some(target) = m.revenge_target {
            let expired = m.ticks_existed - m.hurt_by_timestamp > REVENGE_TICKS;
            let alive =
                self.players.iter().any(|p| {
                    p.entity_id == target && p.health() > 0.0 && !p.dead_marked && !p.removed
                }) || self.mobs.get(&target).is_some_and(Mob::alive);
            let Some(m) = self.mobs.get_mut(&id) else {
                return;
            };
            if !alive || expired {
                m.revenge_target = None;
            }
        }
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        // EntityLiving.onEntityUpdate: the ambient sound.
        if m.alive() {
            let roll = m.rand.next_int_bounded(1000);
            let time = m.living_sound_time;
            m.living_sound_time += 1;
            if roll < time {
                m.living_sound_time = -m.kind.talk_interval();
                let sound = m.kind.sounds().0;
                self.mob_sound(id, sound);
            }
        }
    }

    /// `onDeathUpdate`: 20 ticks after its death the mob drops its experience (after a
    /// player's hit) and is removed.
    fn mob_death_update(&mut self, id: i32) {
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        m.death_time += 1;
        if m.death_time != DEATH_TICKS {
            return;
        }
        let (pos, recently_hit, child) = (m.body.pos, m.recently_hit > 0, m.child());
        if recently_hit && !child && self.level.data.game_rule("doMobLoot") != Some("false") {
            // EntityAnimal.getExperiencePoints: from the world's random.
            let xp = 1 + self.world_rand.next_int_bounded(3);
            self.spawn_orbs(pos, xp);
        }
        // The smoke is client-side; the mob is gone, so its random no longer matters.
        if let Some(m) = self.mobs.get_mut(&id) {
            m.dead = true;
        }
    }

    /// `EntityLivingBase.onLivingUpdate` with `EntityLiving`'s: the AI (unless the mob
    /// has none or is dead), jumping, `moveEntityWithHeading` and pushing; then
    /// `EntityAgeable` and `EntityAnimal`. A mob without AI only slows down.
    fn mob_living_update(&mut self, id: i32) {
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        if m.jump_ticks > 0 {
            m.jump_ticks -= 1;
        }
        // isServerWorld: false for a mob without AI.
        let server = !m.no_ai;
        if !server {
            m.body.motion.0 *= NO_AI_DRAG;
            m.body.motion.1 *= NO_AI_DRAG;
            m.body.motion.2 *= NO_AI_DRAG;
        }
        for v in [
            &mut m.body.motion.0,
            &mut m.body.motion.1,
            &mut m.body.motion.2,
        ] {
            if v.abs() < MOTION_EPSILON {
                *v = 0.0;
            }
        }
        // isMovementBlocked: dead mobs stop thinking.
        if m.health <= 0.0 {
            m.jumping = false;
            m.move_strafing = 0.0;
            m.move_forward = 0.0;
        } else if server {
            self.mob_action_state(id);
        }
        if self.mobs.get(&id).is_some_and(|m| m.jumping) {
            let in_lava = self.mob_in_lava(id);
            let Some(m) = self.mobs.get_mut(&id) else {
                return;
            };
            if m.body.in_water || in_lava {
                // updateAITick / handleJumpLava.
                m.body.motion.1 += f64::from(SWIM_UP);
            } else if m.body.on_ground && m.jump_ticks == 0 {
                // jump. TODO(M5): jump boost.
                m.body.motion.1 = f64::from(JUMP_MOTION);
                m.airborne = true;
                m.jump_ticks = JUMP_COOLDOWN;
            }
        } else if let Some(m) = self.mobs.get_mut(&id) {
            m.jump_ticks = 0;
        }
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        m.move_strafing *= 0.98;
        m.move_forward *= 0.98;
        let (strafe, forward) = (m.move_strafing, m.move_forward);
        if server {
            self.mob_travel(id, strafe, forward);
        }
        self.mob_collide_nearby(id);
        // TODO(M5): picking up loot (EntityLiving.onLivingUpdate).
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        // EntityAgeable: the age counts towards 0.
        if m.kind.ageable() {
            let mut growth = None;
            if m.age < 0 {
                let age = m.age + 1;
                growth = m.set_age(age);
            } else if m.age > 0 {
                let age = m.age - 1;
                growth = m.set_age(age);
            }
            if let Some(d) = growth {
                self.move_mob_by(id, (d, 0.0, d));
            }
            let Some(m) = self.mobs.get_mut(&id) else {
                return;
            };
            // EntityAnimal: love ends for children and parents, and runs out.
            if m.age != 0 {
                m.in_love = 0;
            }
            if m.in_love > 0 {
                m.in_love -= 1;
                if m.in_love % 10 == 0 {
                    // The hearts are client-side but draw from the random.
                    for _ in 0..3 {
                        let _ = m.rand.next_gaussian();
                    }
                    for _ in 0..3 {
                        let _ = m.rand.next_float();
                    }
                }
            }
        }
    }

    /// `EntityLivingBase.moveEntityWithHeading` on the server: walking (with ladders),
    /// swimming in water (depth strider) or wading through lava.
    fn mob_travel(&mut self, id: i32, strafe: f32, forward: f32) {
        let in_lava = self.mob_in_lava(id);
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        if m.body.in_water {
            let y0 = m.body.pos.1;
            let mut friction = WATER_FRICTION;
            let mut acceleration = WATER_ACCELERATION;
            let mut strider =
                (m.max_enchantment(super::item_data::DEPTH_STRIDER) as f32).min(MAX_DEPTH_STRIDER);
            if !m.body.on_ground {
                strider *= 0.5;
            }
            if strider > 0.0 {
                friction += (STRIDER_FRICTION - friction) * strider / 3.0;
                acceleration += (m.ai_move_speed - acceleration) * strider / 3.0;
            }
            if let Some(m) = self.mobs.get_mut(&id) {
                move_flying(m, strafe, forward, acceleration);
            }
            self.move_mob(id);
            let Some(m) = self.mobs.get_mut(&id) else {
                return;
            };
            m.body.motion.0 *= f64::from(friction);
            m.body.motion.1 *= f64::from(0.8f32);
            m.body.motion.2 *= f64::from(friction);
            m.body.motion.1 -= LIQUID_GRAVITY;
            self.mob_ledge_climb(id, y0);
        } else if in_lava {
            let y0 = m.body.pos.1;
            if let Some(m) = self.mobs.get_mut(&id) {
                move_flying(m, strafe, forward, WATER_ACCELERATION);
            }
            self.move_mob(id);
            let Some(m) = self.mobs.get_mut(&id) else {
                return;
            };
            m.body.motion.0 *= 0.5;
            m.body.motion.1 *= 0.5;
            m.body.motion.2 *= 0.5;
            m.body.motion.1 -= LIQUID_GRAVITY;
            self.mob_ledge_climb(id, y0);
        } else {
            let friction = self.mob_friction(id);
            let on_ladder = self.mob_on_ladder(id);
            let Some(m) = self.mobs.get_mut(&id) else {
                return;
            };
            let acceleration = if m.body.on_ground {
                m.ai_move_speed * (GROUND_ACCELERATION / (friction * friction * friction))
            } else {
                JUMP_MOVEMENT_FACTOR
            };
            move_flying(m, strafe, forward, acceleration);
            if on_ladder {
                m.body.motion.0 = m.body.motion.0.clamp(-LADDER_SPEED, LADDER_SPEED);
                m.body.motion.2 = m.body.motion.2.clamp(-LADDER_SPEED, LADDER_SPEED);
                m.base.fall_distance = 0.0;
                if m.body.motion.1 < -LADDER_SPEED {
                    m.body.motion.1 = -LADDER_SPEED;
                }
            }
            self.move_mob(id);
            let climbing = self
                .mobs
                .get(&id)
                .is_some_and(|m| m.body.collided_horizontally)
                && self.mob_on_ladder(id);
            let Some(m) = self.mobs.get_mut(&id) else {
                return;
            };
            if climbing {
                m.body.motion.1 = LADDER_CLIMB;
            }
            m.body.motion.1 -= GRAVITY;
            m.body.motion.1 *= f64::from(DRAG);
            m.body.motion.0 *= f64::from(friction);
            m.body.motion.2 *= f64::from(friction);
        }
    }

    /// The horizontal friction on land: the slipperiness of the block below when on
    /// the ground.
    fn mob_friction(&mut self, id: i32) -> f32 {
        let Some(m) = self.mobs.get(&id) else {
            return AIR_FRICTION;
        };
        if !m.body.on_ground {
            return AIR_FRICTION;
        }
        let p = m.body.pos;
        let below = self.level.overworld.block(
            floor_f64(p.0),
            floor_f64(m.body.bb().min[1]) - 1,
            floor_f64(p.2),
        ) >> 4;
        super::blocks::slipperiness(below) * AIR_FRICTION
    }

    /// `isOnLadder`: the mob's feet are in a ladder or vine.
    fn mob_on_ladder(&mut self, id: i32) -> bool {
        let Some(m) = self.mobs.get(&id) else {
            return false;
        };
        let p = m.body.pos;
        let state = self.level.overworld.block(
            floor_f64(p.0),
            floor_f64(m.body.bb().min[1]),
            floor_f64(p.2),
        );
        matches!(state >> 4, block::LADDER | block::VINE)
    }

    /// Swimming out of a liquid onto a ledge (`isOffsetPositionInLiquid`).
    fn mob_ledge_climb(&mut self, id: i32, y0: f64) {
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        if !m.body.collided_horizontally {
            return;
        }
        let v = m.body.motion;
        let offset = [
            v.0,
            v.1 + f64::from(LEDGE_CHECK_LIFT) - m.body.pos.1 + y0,
            v.2,
        ];
        let bb = m.body.bb().offset(offset);
        let free = physics::colliding_boxes(&mut self.level.overworld, &bb).is_empty()
            && !self.any_block_in(&bb, block::FLOWING_WATER..=block::LAVA);
        if free && let Some(m) = self.mobs.get_mut(&id) {
            m.body.motion.1 = f64::from(LEDGE_JUMP);
        }
    }

    /// `Entity.moveEntity` with the mob's motion, then what follows in vanilla: the
    /// steps, the fall (`EntityLivingBase.updateFallState`), the blocks touched, fire
    /// and water.
    fn move_mob(&mut self, id: i32) {
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        let v = m.body.motion;
        self.move_mob_by(id, v);
    }

    /// `Entity.moveEntity` by `d` and what follows it, like [`Self::move_mob`].
    pub(in crate::game) fn move_mob_by(&mut self, id: i32, d: (f64, f64, f64)) {
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        let mut body = m.body.clone();
        let riding = m.riding;
        // Boats are solid (getCollisionBoundingBox), its own one too (the vanilla quirk
        // in Game::cart_obstacles).
        let boats = self.boat_boxes(&body.bb().add_coord([d.0, d.1, d.2]), None);
        // A rider does not walk: no steps, no block walked on.
        let outcome = if riding.is_some() {
            physics::move_entity_among(&mut self.level.overworld, &mut body, d, &boats)
        } else {
            physics::move_entity_walking_among(&mut self.level.overworld, &mut body, d, &boats)
        };
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        m.body = body;
        if outcome.stepped {
            if let Some(volume) = outcome.swim {
                let pitch = 1.0 + (m.rand.next_float() - m.rand.next_float()) * 0.4;
                let (pos, silent) = (m.body.pos, m.base.silent);
                if !silent {
                    self.play_sound_near_except(None, "game.neutral.swim", pos, volume, pitch);
                }
            }
            self.mob_step_sound(id);
        }
        if let Some((pos, state)) = outcome.walked_on
            && matches!(state >> 4, block::REDSTONE_ORE | block::LIT_REDSTONE_ORE)
        {
            self.activate_redstone_ore(pos, state);
        }
        // EntityLivingBase.updateFallState: water is checked once more first.
        if self.mobs.get(&id).is_some_and(|m| !m.body.in_water) {
            self.mob_water_movement(id);
        }
        self.mob_fall(id, outcome.moved_y, outcome.below);
        // doBlockCollisions.
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        let bb = m.body.bb();
        let touched = self.block_collisions(&bb);
        if let Some(m) = self.mobs.get_mut(&id) {
            m.body.touched(touched);
            if touched.web {
                m.base.fall_distance = 0.0;
            }
        }
        self.mob_move_fire(id);
    }

    /// `playStepSound` of the mob's kind.
    fn mob_step_sound(&mut self, id: i32) {
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        let (pos, silent) = (m.body.pos, m.base.silent);
        let (sound, volume) = m.kind.step_sound();
        if !silent {
            self.play_sound_near_except(None, sound, pos, volume, 1.0);
        }
    }

    /// `updateFallState` after a move: landing from more than 3 blocks raises dust; a
    /// landing ends the fall through the block below (`onFallenUpon`), falling counts
    /// down.
    fn mob_fall(&mut self, id: i32, moved_y: f64, below: ((i32, i32, i32), u16)) {
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        let (pos, on_ground, distance) = (m.body.pos, m.body.on_ground, m.base.fall_distance);
        let (block_pos, state) = below;
        if on_ground && distance > SAFE_FALL && state >> 4 != block::AIR {
            let extra = ceil_f32(distance - SAFE_FALL) as f32;
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
                let mut multiplier = 1.0;
                let mut falls = true;
                match state >> 4 {
                    // BlockFarmland.onFallenUpon: the world's random decides; without
                    // mob griefing nothing happens, not even the fall.
                    block::FARMLAND => {
                        if self.world_rand.next_float() < distance - TRAMPLE_OFFSET {
                            if self.level.data.game_rule(MOB_GRIEFING) == Some("false") {
                                falls = false;
                            } else {
                                self.set_block_notify(block_pos, block::DIRT << 4);
                            }
                        }
                    }
                    // BlockSlime.onFallenUpon: no damage (mobs never sneak).
                    block::SLIME => multiplier = 0.0,
                    _ => {}
                }
                if falls {
                    self.mob_fall_damage(id, distance, multiplier);
                }
                if let Some(m) = self.mobs.get_mut(&id) {
                    m.base.fall_distance = 0.0;
                }
            }
        } else if moved_y < 0.0
            && let Some(m) = self.mobs.get_mut(&id)
        {
            m.base.fall_distance = (f64::from(m.base.fall_distance) - moved_y) as f32;
        }
    }

    /// `EntityLivingBase.fall`: the fall sound, the damage, then the landing sound of
    /// the block below.
    fn mob_fall_damage(&mut self, id: i32, distance: f32, multiplier: f32) {
        // TODO(M5): EntityPig.fall gives a rider the flyPig achievement after a fall of
        // more than 5 blocks (achievements are missing).
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        let damage = ceil_f32((distance - SAFE_FALL) * multiplier);
        if damage <= 0 {
            return;
        }
        let (pos, silent) = (m.body.pos, m.base.silent);
        let sound = if damage > 4 {
            "game.neutral.hurt.fall.big"
        } else {
            "game.neutral.hurt.fall.small"
        };
        if !silent {
            self.play_sound_near_except(None, sound, pos, 1.0, 1.0);
        }
        self.attack_mob(id, DamageSource::Fall, damage as f32);
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        let (pos, silent) = (m.body.pos, m.base.silent);
        let below = self.level.overworld.block(
            floor_f64(pos.0),
            floor_f64(pos.1 - f64::from(0.2f32)),
            floor_f64(pos.2),
        ) >> 4;
        if below != block::AIR && !silent {
            let s = super::blocks::sound_type(below);
            self.play_sound_near_except(
                None,
                &s.step_sound(),
                pos,
                s.volume * 0.5,
                s.frequency * 0.75,
            );
        }
    }

    /// The end of `moveEntity`: standing in fire or lava burns (and sets on fire after
    /// a while); getting wet puts the fire out.
    fn mob_move_fire(&mut self, id: i32) {
        let wet = self.mob_wet(id);
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        let bb = m.body.bb().contract([0.001, 0.001, 0.001]);
        if self.flammable_within(&bb) {
            // dealFireDamage.
            self.attack_mob(id, DamageSource::InFire, IN_FIRE_DAMAGE);
            if !wet && let Some(m) = self.mobs.get_mut(&id) {
                m.base.fire += 1;
                if m.base.fire == 0 {
                    m.set_fire(FIRE_BLOCK_SECONDS);
                }
            }
        } else if let Some(m) = self.mobs.get_mut(&id)
            && m.base.fire <= 0
        {
            m.base.fire = -FIRE_RESISTANCE;
        }
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        if wet && m.base.fire > 0 {
            let pitch = 1.6 + (m.rand.next_float() - m.rand.next_float()) * 0.4;
            m.base.fire = -FIRE_RESISTANCE;
            let (pos, silent) = (m.body.pos, m.base.silent);
            if !silent {
                self.play_sound_near_except(None, "random.fizz", pos, 0.7, pitch);
            }
        }
    }

    /// `collideWithNearbyEntities`: the pushable entities overlapping the mob (its box
    /// grown by 0.2 sideways) push it and are pushed (their `applyEntityCollision`),
    /// in chunk list order: players, other mobs, minecarts and boats.
    fn mob_collide_nearby(&mut self, id: i32) {
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        let area = m
            .body
            .bb()
            .expand([f64::from(0.2f32), 0.0, f64::from(0.2f32)]);
        let mut others: Vec<(i32, Pushed)> = Vec::new();
        for (i, p) in self.players.iter().enumerate() {
            // applyEntityCollision: not with its own rider.
            if p.gamemode != SPECTATOR
                && p.riding != Some(id)
                && !p.dead_marked
                && !p.removed
                && physics::Aabb::at(p.pos, 0.6, 1.8).intersects(&area)
            {
                others.push((p.entity_id, Pushed::Player(i)));
            }
        }
        // Entity.applyEntityCollision: not between a rider and its vehicle.
        let (rider, riding) = (m.rider, m.riding);
        for (&other, o) in &self.mobs {
            if other != id
                && !o.dead
                && Some(other) != rider
                && Some(other) != riding
                && o.body.bb().intersects(&area)
            {
                others.push((other, Pushed::Mob));
            }
        }
        for (&cart, c) in &self.minecarts {
            if !c.dead && c.body.bb().intersects(&area) {
                others.push((cart, Pushed::Minecart));
            }
        }
        for (&boat, b) in &self.boats {
            if !b.dead && b.body.bb().intersects(&area) {
                others.push((boat, Pushed::Boat));
            }
        }
        others.sort_by_key(|(other, _)| *other);
        for (other, kind) in others {
            let Some((me, free)) = self.mobs.get(&id).map(|m| (m.body.pos, m.rider.is_none()))
            else {
                return;
            };
            match kind {
                Pushed::Player(i) => {
                    let Some((dx, dz)) = super::boat::collision_push(self.players[i].pos, me)
                    else {
                        continue;
                    };
                    // Nothing rides players.
                    let p = &mut self.players[i];
                    p.motion.0 -= dx;
                    p.motion.2 -= dz;
                    p.airborne = true;
                    if free && let Some(m) = self.mobs.get_mut(&id) {
                        add_velocity(m, dx, dz);
                    }
                }
                Pushed::Mob => {
                    let Some((at, other_free)) = self
                        .mobs
                        .get(&other)
                        .map(|o| (o.body.pos, o.rider.is_none()))
                    else {
                        continue;
                    };
                    let Some((dx, dz)) = super::boat::collision_push(at, me) else {
                        continue;
                    };
                    if other_free && let Some(o) = self.mobs.get_mut(&other) {
                        add_velocity(o, -dx, -dz);
                    }
                    if free && let Some(m) = self.mobs.get_mut(&id) {
                        add_velocity(m, dx, dz);
                    }
                }
                Pushed::Minecart => {
                    self.minecart_collision(other, super::minecart::CartPusher::Mob(id));
                }
                Pushed::Boat => self.boat_pushed_by_mob(other, id),
            }
        }
    }

    /// A player overlapping the mob pushes it (`applyEntityCollision` of the mob with
    /// the player `i`).
    pub(super) fn player_pushes_mob(&mut self, i: usize, id: i32) {
        // Not the mob it rides.
        if self.players[i].riding == Some(id) {
            return;
        }
        let Some((at, free)) = self.mobs.get(&id).map(|m| (m.body.pos, m.rider.is_none())) else {
            return;
        };
        let Some((dx, dz)) = super::boat::collision_push(at, self.players[i].pos) else {
            return;
        };
        // A ridden mob is not pushed.
        if free && let Some(m) = self.mobs.get_mut(&id) {
            add_velocity(m, -dx, -dz);
        }
        let p = &mut self.players[i];
        p.motion.0 += dx;
        p.motion.2 += dz;
        p.airborne = true;
    }

    /// `Entity.handleWaterMovement`: flowing water pushes the mob; getting into water
    /// splashes, ends the fall and puts the fire out.
    fn mob_water_movement(&mut self, id: i32) {
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        let bb = m
            .body
            .bb()
            .expand([0.0, f64::from(-0.4f32), 0.0])
            .contract([0.001, 0.001, 0.001]);
        let (water, push) = super::liquids::water_movement(&mut self.level.overworld, &bb, true);
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        if push != (0.0, 0.0, 0.0) {
            m.body.motion.0 += push.0;
            m.body.motion.1 += push.1;
            m.body.motion.2 += push.2;
        }
        if water && !m.body.in_water && !m.first_update {
            // resetHeight: the splash, then its particles (client-side, but they draw
            // from the random).
            let mo = m.body.motion;
            let volume =
                ((mo.0 * mo.0 * f64::from(0.2f32) + mo.1 * mo.1 + mo.2 * mo.2 * f64::from(0.2f32))
                    .sqrt() as f32
                    * 0.2)
                    .min(1.0);
            let pitch = 1.0 + (m.rand.next_float() - m.rand.next_float()) * 0.4;
            let particles = (1.0 + m.body.width * 20.0) as usize;
            for _ in 0..particles * 5 {
                let _ = m.rand.next_float();
            }
            let (pos, silent) = (m.body.pos, m.base.silent);
            if !silent {
                self.play_sound_near_except(None, "game.neutral.swim.splash", pos, volume, pitch);
            }
        }
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        m.body.in_water = water;
        if water {
            m.base.wet();
        }
    }

    /// `isInLava`.
    fn mob_in_lava(&mut self, id: i32) -> bool {
        let Some(m) = self.mobs.get(&id) else {
            return false;
        };
        let bb = m
            .body
            .bb()
            .expand([f64::from(-0.1f32), f64::from(-0.4f32), f64::from(-0.1f32)]);
        self.any_block_in(&bb, block::FLOWING_LAVA..=block::LAVA)
    }

    /// `Entity.isWet`: in water or in the rain at the feet or the top.
    fn mob_wet(&mut self, id: i32) -> bool {
        let Some(m) = self.mobs.get(&id) else {
            return false;
        };
        if m.body.in_water {
            return true;
        }
        let p = m.body.pos;
        let (x, z) = (floor_f64(p.0), floor_f64(p.2));
        let (feet, top) = (floor_f64(p.1), floor_f64(p.1 + f64::from(m.body.height)));
        self.rains_at((x, feet, z)) || self.rains_at((x, top, z))
    }

    /// `Entity.isEntityInsideOpaqueBlock`: any of 8 points around the eyes in a
    /// visually opaque block.
    fn mob_inside_opaque_block(&mut self, id: i32) -> bool {
        let Some(m) = self.mobs.get(&id) else {
            return false;
        };
        let (pos, eye, width) = (m.body.pos, m.eye_height(), m.body.width);
        let mut last = None;
        for k in 0..8 {
            let side = |bit: i32| (bit % 2) as f32 - 0.5;
            let y = floor_f64(pos.1 + f64::from(side(k) * 0.1) + f64::from(eye));
            let x = floor_f64(pos.0 + f64::from(side(k >> 1) * width * 0.8));
            let z = floor_f64(pos.2 + f64::from(side(k >> 2) * width * 0.8));
            if last == Some((x, y, z)) {
                continue;
            }
            last = Some((x, y, z));
            let id = self.level.overworld.block(x, y, z) >> 4;
            if super::light_data::visually_opaque(id) {
                return true;
            }
        }
        false
    }

    /// `Entity.playSound` of a mob at its kind's volume (`getSoundVolume`), unless
    /// silent.
    fn mob_sound(&mut self, id: i32, sound: &str) {
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        let pitch = m.sound_pitch();
        let (pos, silent, volume) = (m.body.pos, m.base.silent, m.kind.sound_volume());
        if !silent {
            self.play_sound_near_except(None, sound, pos, volume, pitch);
        }
    }

    /// `EntityAnimal.attackEntityFrom` and `EntityLivingBase.attackEntityFrom` for a
    /// mob. Returns whether the damage was taken.
    pub(super) fn attack_mob(&mut self, id: i32, source: DamageSource, amount: f32) -> bool {
        let creative_attacker = source.attacker().is_some_and(|a| {
            matches!(source, DamageSource::Player(_))
                && self
                    .players
                    .iter()
                    .any(|p| p.entity_id == a && p.gamemode == super::player::Player::CREATIVE)
        });
        let Some(m) = self.mobs.get_mut(&id) else {
            return false;
        };
        // isEntityInvulnerable.
        if m.base.invulnerable && source != DamageSource::OutOfWorld && !creative_attacker {
            return false;
        }
        m.entity_age = 0;
        if m.kind.ageable() {
            m.in_love = 0;
        }
        if m.health <= 0.0 {
            return false;
        }
        // TODO(M5): fire resistance.
        let mut amount = amount;
        if matches!(source, DamageSource::Anvil | DamageSource::FallingBlock)
            && m.equipment[4].is_some()
        {
            // TODO(M5): the helmet wears.
            let _wear = (amount * 4.0 + m.rand.next_float() * amount * 2.0) as i32;
            amount *= 0.75;
        }
        let fresh = if m.hurt_resistant_time as f32 > MAX_HURT_RESISTANT_TIME as f32 / 2.0 {
            if amount <= m.last_damage {
                return false;
            }
            let extra = amount - m.last_damage;
            self.damage_mob(id, source, extra);
            if let Some(m) = self.mobs.get_mut(&id) {
                m.last_damage = amount;
            }
            false
        } else {
            m.last_damage = amount;
            m.hurt_resistant_time = MAX_HURT_RESISTANT_TIME;
            self.damage_mob(id, source, amount);
            if let Some(m) = self.mobs.get_mut(&id) {
                m.hurt_time = MAX_HURT_TIME;
            }
            true
        };
        let attacker = source.attacker();
        let attacker_player = attacker.filter(|a| self.players.iter().any(|p| p.entity_id == *a));
        let Some(m) = self.mobs.get_mut(&id) else {
            return true;
        };
        if let Some(a) = attacker_player {
            // setRevengeTarget, then the player's hit counts for loot.
            m.revenge_target = Some(a);
            m.hurt_by_timestamp = m.ticks_existed;
            m.recently_hit = RECENTLY_HIT;
            m.attacking_player = Some(a);
        }
        if fresh {
            self.send_to_tracking(
                id,
                &ClientboundPlay::EntityStatus {
                    entity_id: id,
                    status: STATUS_HURT,
                },
            );
            let Some(m) = self.mobs.get_mut(&id) else {
                return true;
            };
            if source != DamageSource::Drown {
                // setBeenAttacked: knockback resistance may keep the motion unsent.
                let resisted = m.rand.next_double() < m.knockback_resistance();
                m.body.velocity_changed = !resisted;
            }
            let from = match attacker {
                Some(a) => match self.players.iter().find(|p| p.entity_id == a) {
                    Some(p) => Some(p.pos),
                    None => self.entity_position(a),
                },
                None => None,
            };
            match from {
                Some(from) => self.knock_back_mob(id, from),
                // attackedAtYaw is picked with Math.random().
                None => {
                    let _ = self.math_random.next_double();
                }
            }
        }
        let Some(m) = self.mobs.get(&id) else {
            return true;
        };
        let (_, hurt, death) = m.kind.sounds();
        if m.health <= 0.0 {
            if fresh {
                self.mob_sound(id, death);
            }
            self.mob_died(id, source);
        } else if fresh {
            self.mob_sound(id, hurt);
        }
        true
    }

    /// `EntityLivingBase.knockBack` away from `from` (after `attackEntityFrom` drew the
    /// direction with `Math.random()` when the attacker is right above).
    fn knock_back_mob(&mut self, id: i32, from: (f64, f64, f64)) {
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        let mut dx = from.0 - m.body.pos.0;
        let mut dz = from.2 - m.body.pos.2;
        while dx * dx + dz * dz < 1.0e-4 {
            dx = (self.math_random.next_double() - self.math_random.next_double()) * 0.01;
            dz = (self.math_random.next_double() - self.math_random.next_double()) * 0.01;
        }
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        if m.rand.next_double() < m.knockback_resistance() {
            return;
        }
        m.airborne = true;
        let len = f64::from(krol_java::math::sqrt_f64(dx * dx + dz * dz));
        let strength = f64::from(KNOCKBACK);
        let v = &mut m.body.motion;
        v.0 /= 2.0;
        v.1 /= 2.0;
        v.2 /= 2.0;
        v.0 -= dx / len * strength;
        v.1 += strength;
        v.2 -= dz / len * strength;
        if v.1 > f64::from(KNOCKBACK) {
            v.1 = f64::from(KNOCKBACK);
        }
    }

    /// `EntityLivingBase.damageEntity` for a mob: armor, protection, absorption, then
    /// the health.
    fn damage_mob(&mut self, id: i32, source: DamageSource, amount: f32) {
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        if m.base.invulnerable && source != DamageSource::OutOfWorld {
            return;
        }
        // applyArmorCalculations (a mob's armor does not wear).
        let mut amount = amount;
        if !source.unblockable() {
            let armor: i32 = m.equipment[1..]
                .iter()
                .flatten()
                .map(|s| super::item_data::armor_points(s.id))
                .sum();
            amount = amount * (25 - armor) as f32 / 25.0;
        }
        // applyPotionDamageCalculations: protection enchantments draw from
        // EnchantmentHelper's random even when there are none.
        // TODO(M5): the resistance potion.
        if amount <= 0.0 {
            amount = 0.0;
        } else {
            let mut modifier = 0;
            for stack in m.equipment.iter().flatten() {
                for (ench, level) in super::item_data::enchantments(stack) {
                    modifier += super::damage::protection_modifier(ench, level, source);
                }
            }
            let modifier = modifier.clamp(0, 25);
            let k = ((modifier + 1) >> 1) + self.enchant_rand.next_int_bounded((modifier >> 1) + 1);
            let k = k.min(20);
            if k > 0 {
                amount = amount * (25 - k) as f32 / 25.0;
            }
        }
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        let damage = (amount - m.absorption).max(0.0);
        m.absorption = (m.absorption - (amount - damage)).max(0.0);
        if damage != 0.0 {
            let health = m.health;
            m.set_health(health - damage);
            // TODO(M5): the combat tracker.
        }
    }

    /// `EntityLivingBase.onDeath` for a mob: the killer's score and kill stats, the
    /// loot, then the death status.
    fn mob_died(&mut self, id: i32, source: DamageSource) {
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        // func_94060_bK: without a combat tracker, the player who hit it last.
        let killer = m
            .attacking_player
            .and_then(|a| self.players.iter().position(|p| p.entity_id == a));
        let kind = m.kind;
        if let Some(k) = killer {
            // addToPlayerScore with a score value of 0.
            self.add_stat(k, "stat.mobKills", 1);
            let name = self.players[k].profile.name.clone();
            for objective in self.scoreboard.objectives_for("totalKillCount") {
                self.sb_add_score(&name, &objective, 1);
            }
        }
        let source_player = source
            .attacker()
            .and_then(|a| self.players.iter().position(|p| p.entity_id == a));
        if let Some(p) = source_player {
            // EntityPlayer.onKillEntity: the kill stat of mobs with a spawn egg.
            self.add_stat(p, &format!("stat.killEntity.{}", kind.saved_id()), 1);
        }
        let looting = match source {
            DamageSource::Player(_) => source_player.map_or(0, |p| {
                super::item_data::enchantment_level(
                    self.players[p].held(),
                    super::item_data::LOOTING,
                )
            }),
            _ => 0,
        };
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        let recently_hit = m.recently_hit > 0;
        if !m.child() && self.level.data.game_rule("doMobLoot") != Some("false") {
            self.mob_drop_loot(id, looting);
            self.mob_drop_equipment(id, recently_hit, looting);
            if recently_hit
                && let Some(m) = self.mobs.get_mut(&id)
                && m.rand.next_float() < RARE_DROP_CHANCE + looting as f32 * RARE_DROP_LOOTING
            {
                // addRandomDrop: pigs and cows have none.
            }
        }
        self.send_to_tracking(
            id,
            &ClientboundPlay::EntityStatus {
                entity_id: id,
                status: STATUS_DEATH,
            },
        );
    }

    /// `dropFewItems` of a mob.
    fn mob_drop_loot(&mut self, id: i32, looting: i32) {
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        let pos = m.body.pos;
        let mut drops = Vec::new();
        match m.kind {
            MobKind::Pig => {
                let n = m.rand.next_int_bounded(3) + 1 + m.rand.next_int_bounded(1 + looting);
                let meat = if m.burning() {
                    item::COOKED_PORKCHOP
                } else {
                    item::PORKCHOP
                };
                drops.extend(std::iter::repeat_n(meat, n.max(0) as usize));
                if m.saddled {
                    drops.push(item::SADDLE);
                }
            }
            MobKind::Cow => {
                let n = m.rand.next_int_bounded(3) + m.rand.next_int_bounded(1 + looting);
                drops.extend(std::iter::repeat_n(item::LEATHER, n.max(0) as usize));
                let n = m.rand.next_int_bounded(3) + 1 + m.rand.next_int_bounded(1 + looting);
                let meat = if m.burning() {
                    item::COOKED_BEEF
                } else {
                    item::BEEF
                };
                drops.extend(std::iter::repeat_n(meat, n.max(0) as usize));
            }
        }
        for id in drops {
            self.mob_drop_item(pos, single(id));
        }
    }

    /// `EntityLiving.dropEquipment`: each piece it holds or wears drops by its drop
    /// chance (after a player's hit, unless the chance is above 1).
    fn mob_drop_equipment(&mut self, id: i32, recently_hit: bool, looting: i32) {
        for slot in 0..SLOTS {
            let Some(m) = self.mobs.get_mut(&id) else {
                return;
            };
            let chance = m.drop_chances[slot];
            let guaranteed = chance > 1.0;
            let Some(stack) = m.equipment[slot].clone() else {
                continue;
            };
            if !(recently_hit || guaranteed) {
                continue;
            }
            if m.rand.next_float() - looting as f32 * 0.01 < chance {
                // TODO(M5): damageable items drop worn at random unless guaranteed.
                let pos = m.body.pos;
                self.mob_drop_item(pos, stack);
            }
        }
    }

    /// `Entity.entityDropItem` at the mob's feet.
    fn mob_drop_item(&mut self, pos: (f64, f64, f64), stack: ItemStack) {
        if stack.count != 0 {
            self.spawn_item(pos, stack, DROP_PICKUP_DELAY);
        }
    }

    /// `EntityPlayer.attackTargetEntityWithCurrentItem` on a mob.
    pub(super) fn player_attacks_mob(&mut self, i: usize, id: i32) {
        let held = self.players[i].held().cloned();
        let mut damage =
            super::attributes::value(super::attributes::ATTACK_DAMAGE, &self.players[i].modifiers)
                as f32;
        // TODO(M5): smite and bane of arthropods against undead and arthropod mobs.
        let bonus = super::combat::enchantment_damage(held.as_ref());
        let mut knockback =
            super::item_data::enchantment_level(held.as_ref(), super::combat::KNOCKBACK);
        if self.players[i].sprinting {
            knockback += 1;
        }
        if damage <= 0.0 && bonus <= 0.0 {
            return;
        }
        let critical = self.critical_hit(i);
        if critical && damage > 0.0 {
            damage *= 1.5;
        }
        damage += bonus;
        let fire_aspect =
            super::item_data::enchantment_level(held.as_ref(), super::item_data::FIRE_ASPECT);
        let mut lit = false;
        if fire_aspect > 0 && self.mobs.get(&id).is_some_and(|m| !m.burning()) {
            lit = true;
            if let Some(m) = self.mobs.get_mut(&id) {
                m.set_fire(1);
            }
        }
        let attacker = self.players[i].entity_id;
        if !self.attack_mob(id, DamageSource::Player(attacker), damage) {
            if lit && let Some(m) = self.mobs.get_mut(&id) {
                m.base.fire = 0;
            }
            return;
        }
        if knockback > 0 {
            let yaw = self.players[i].rotation.0 * std::f32::consts::PI / 180.0;
            let k = knockback as f32 * 0.5;
            if let Some(m) = self.mobs.get_mut(&id) {
                // addVelocity.
                m.body.motion.0 += f64::from(-krol_java::math::sin(yaw) * k);
                m.body.motion.1 += 0.1;
                m.body.motion.2 += f64::from(krol_java::math::cos(yaw) * k);
            }
            let a = &mut self.players[i];
            a.motion.0 *= 0.6;
            a.motion.2 *= 0.6;
            self.set_sprinting(i, false);
        }
        self.critical_effects(i, id, critical, bonus);
        // TODO(M5): thorns on what the mob wears.
        self.wear_weapon(i, held);
        self.add_stat(i, "stat.damageDealt", (damage * 10.0).round() as i32);
        if fire_aspect > 0
            && let Some(m) = self.mobs.get_mut(&id)
        {
            m.set_fire(fire_aspect * 4);
        }
        self.add_exhaustion(i, super::combat::ATTACK_EXHAUSTION);
    }

    /// The entity of a mob as saved.
    pub(super) fn mob_nbt(&self, m: &Mob, uuid: u128) -> Compound {
        let mut c = Compound::new();
        c.insert("id", Tag::String(m.kind.saved_id().into()));
        m.base.write_entity(&m.body, uuid, &mut c);
        m.write(&mut c);
        // Entity.writeToNBT: the vehicle it rides, if alive (writeMountToNBT).
        if let Some(mount) = m.riding.and_then(|v| self.mount_nbt(v)) {
            c.insert("Riding", Tag::Compound(mount));
        }
        c
    }
}

/// `moveFlying`: accelerate by `friction` towards the input direction, turned by the
/// mob's yaw.
fn move_flying(m: &mut Mob, strafe: f32, forward: f32, friction: f32) {
    let mut f = strafe * strafe + forward * forward;
    if f < 1.0e-4 {
        return;
    }
    f = sqrt_f32(f);
    if f < 1.0 {
        f = 1.0;
    }
    f = friction / f;
    let (strafe, forward) = (strafe * f, forward * f);
    let yaw = m.base.yaw * std::f32::consts::PI / 180.0;
    let (s, c) = (sin(yaw), cos(yaw));
    m.body.motion.0 += f64::from(strafe * c - forward * s);
    m.body.motion.2 += f64::from(forward * c + strafe * s);
}

/// `Entity.addVelocity(x, 0, z)`.
pub(super) fn add_velocity(m: &mut Mob, x: f64, z: f64) {
    m.body.motion.0 += x;
    // Adding 0.0 turns a -0.0 into 0.0.
    m.body.motion.1 += 0.0;
    m.body.motion.2 += z;
    m.airborne = true;
}

/// What a mob bumps into in [`Game::mob_collide_nearby`].
#[derive(Debug, Clone, Copy)]
enum Pushed {
    /// A player, by index.
    Player(usize),
    /// Another mob.
    Mob,
    /// A minecart.
    Minecart,
    /// A boat.
    Boat,
}

/// `WorldSettings.GameType.SPECTATOR`.
const SPECTATOR: i32 = 3;

/// One item of `id`.
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
    fn pig_metadata_follows_the_watcher_order() {
        let pig = Mob::new(MobKind::Pig, (0.0, 64.0, 0.0), 0.0, 1);
        let order: Vec<u8> = pig.metadata().iter().map(|m| m.index).collect();
        assert_eq!(order, [0, 16, 1, 2, 3, 4, 6, 7, 8, 9, 12, 15]);
        assert!(pig.metadata_changed);
        assert_eq!(pig.properties()[0].0, MOVEMENT_SPEED);
    }

    #[test]
    fn cows_have_their_own_size_speed_and_eyes() {
        let cow = Mob::new(MobKind::Cow, (0.0, 64.0, 0.0), 0.0, 1);
        let order: Vec<u8> = cow.metadata().iter().map(|m| m.index).collect();
        assert_eq!(order, [0, 1, 2, 3, 4, 6, 7, 8, 9, 12, 15]);
        assert_eq!((cow.body.width, cow.body.height), (0.9, 1.3));
        assert_eq!(cow.eye_height(), 1.3);
        assert_eq!(cow.properties()[0].1, 0.20000000298023224);
        assert_eq!(MobKind::from_type_id(92), Some(MobKind::Cow));
        assert_eq!(MobKind::from_saved_id("Cow"), Some(MobKind::Cow));
    }

    #[test]
    fn growing_up_keeps_the_box_corner_and_moves_back() {
        let mut pig = Mob::new(MobKind::Pig, (0.5, 64.0, 0.5), 0.0, 1);
        // Loaded as a child: the box shrinks on its corner, the position stays.
        assert_eq!(pig.set_age(-1), None);
        let bb = pig.body.bb();
        let corner = 0.5 - f64::from(0.45f32);
        assert_eq!(bb.min, [corner, 64.0, corner]);
        assert_eq!(bb.max[0], corner + f64::from(0.45f32));
        assert_eq!(pig.body.pos, (0.5, 64.0, 0.5));
        // After its first update, growing up asks for a move back by the growth.
        pig.first_update = false;
        assert_eq!(pig.set_age(0), Some(f64::from(0.45f32 - 0.9f32)));
        assert_eq!(pig.body.bb().min, [corner, 64.0, corner]);
        assert_eq!(pig.body.width, 0.9);
        // Shrinking never moves.
        assert_eq!(pig.set_age(-5), None);
    }

    #[test]
    fn saves_round_trip() {
        let mut pig = Mob::new(MobKind::Pig, (1.5, 64.0, 2.5), 0.0, 1);
        pig.set_saddled(true);
        pig.set_no_ai(true);
        pig.set_age(-100);
        let mut c = Compound::new();
        pig.write(&mut c);
        let loaded = Mob::load(MobKind::Pig, pig.body.clone(), pig.base.clone(), &c, 0.0, 1);
        assert!(loaded.saddled && loaded.no_ai && loaded.child());
        assert_eq!(loaded.body.width, 0.45);
        assert_eq!(loaded.health, 10.0);
    }
}
