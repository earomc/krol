//! Entities saved with their chunk (vanilla: the entity half of
//! `AnvilChunkLoader.writeChunkToNBT` / `readChunkFromNBT`, `Chunk.entityLists`,
//! `Chunk.onChunkLoad`, `EntityList.createEntityFromNBT`, `Entity.writeToNBT` /
//! `readFromNBT` and the `writeEntityToNBT` / `readEntityFromNBT` of items, experience
//! orbs, falling blocks and primed TNT).
//!
//! Every chunk keeps its entities in 16 lists, one per 16-block section, in the order
//! they entered the section; the chunk saves them in that order. Saved entities of the
//! kinds krol does not simulate yet keep their NBT as it was read and are written back
//! unchanged.
//!
//! TODO(M5): simulate (tick, track) the other entity kinds instead of keeping their NBT;
//! log vanilla's "Wrong location!" warning for entities saved in the wrong chunk.

use std::collections::HashMap;

use krol_java::math::floor_f64;
use krol_nbt::{Compound, List, Tag, TagType};
use krol_protocol::packets::play::{Metadata, MetadataValue};
use tracing::warn;

use super::Game;
use super::falling_block::FallingBlock;
use super::item_entity::ItemEntity;
use super::physics::Body;
use super::tnt::PrimedTnt;
use super::xp_orb::XpOrb;

/// The entity ids `EntityList` can create from NBT. Saved entities with other ids are
/// skipped with a warning (and lost on the next save).
const KNOWN_IDS: &[&str] = &[
    "Item",
    "XPOrb",
    "ThrownEgg",
    "LeashKnot",
    "Painting",
    "Arrow",
    "Snowball",
    "Fireball",
    "SmallFireball",
    "ThrownEnderpearl",
    "EyeOfEnderSignal",
    "ThrownPotion",
    "ThrownExpBottle",
    "ItemFrame",
    "WitherSkull",
    "PrimedTnt",
    "FallingSand",
    "FireworksRocketEntity",
    "ArmorStand",
    "Boat",
    "MinecartRideable",
    "MinecartChest",
    "MinecartFurnace",
    "MinecartTNT",
    "MinecartSpawner",
    "MinecartHopper",
    "MinecartCommandBlock",
    "Mob",
    "Monster",
    "Creeper",
    "Skeleton",
    "Spider",
    "Giant",
    "Zombie",
    "Slime",
    "Ghast",
    "PigZombie",
    "Enderman",
    "CaveSpider",
    "Silverfish",
    "Blaze",
    "LavaSlime",
    "EnderDragon",
    "WitherBoss",
    "Bat",
    "Witch",
    "Endermite",
    "Guardian",
    "Pig",
    "Sheep",
    "Cow",
    "Chicken",
    "Squid",
    "Wolf",
    "MushroomCow",
    "SnowMan",
    "Ozelot",
    "VillagerGolem",
    "EntityHorse",
    "Rabbit",
    "Villager",
    "EnderCrystal",
];

/// The pre-1.6 id of every minecart, told apart by its `Type`
/// (`EntityMinecart.EnumMinecartType`, by network id; unknown types are rideable).
const LEGACY_MINECART_ID: &str = "Minecart";
const MINECART_IDS: [&str; 7] = [
    "MinecartRideable",
    "MinecartChest",
    "MinecartFurnace",
    "MinecartTNT",
    "MinecartSpawner",
    "MinecartHopper",
    "MinecartCommandBlock",
];

/// The saved ids of the entity kinds krol simulates (`EntityList` names).
const ITEM_ID: &str = "Item";
const ORB_ID: &str = "XPOrb";
const FALLING_ID: &str = "FallingSand";
const TNT_ID: &str = "PrimedTnt";

/// The sections of a chunk (`Chunk.entityLists`).
const SECTIONS: usize = 16;

/// The air of an entity that is not drowning (`dataWatcher` entry 1 starts at 300).
pub const FULL_AIR: i16 = 300;
/// `Entity.fireResistance` of the entity kinds krol simulates: after a dry move the fire
/// counter rests at minus this.
const FIRE_RESISTANCE: i16 = 1;
/// `Entity.readFromNBT` drops saved motion faster than this on any axis.
const MAX_LOADED_MOTION: f64 = 10.0;
/// The command result types an entity can report to the scoreboard
/// (`CommandResultStats.Type`), saved as `<type>Name` / `<type>Objective`.
const COMMAND_STAT_TYPES: [&str; 5] = [
    "SuccessCount",
    "AffectedBlocks",
    "AffectedEntities",
    "AffectedItems",
    "QueryResult",
];

/// The state every entity has besides its physics (vanilla `Entity` fields saved by
/// `writeToNBT`).
#[derive(Debug, Clone)]
pub struct EntityBase {
    /// `rotationYaw`.
    pub yaw: f32,
    /// `rotationPitch`.
    pub pitch: f32,
    /// `fallDistance`.
    pub fall_distance: f32,
    /// `fire`: ticks left burning when positive.
    pub fire: i16,
    /// `getAir` (metadata entry 1).
    pub air: i16,
    /// `invulnerable`.
    pub invulnerable: bool,
    /// `timeUntilPortal`.
    pub portal_cooldown: i32,
    /// `dimension`.
    pub dimension: i32,
    /// `getCustomNameTag` (metadata entry 2), if not empty.
    pub custom_name: Option<String>,
    /// `getAlwaysRenderNameTag` (metadata entry 3).
    pub custom_name_visible: bool,
    /// `isSilent` (metadata entry 4).
    pub silent: bool,
    /// `cmdResultStats`: the complete `<type>Name` / `<type>Objective` pairs.
    pub command_stats: Option<Compound>,
}

impl Default for EntityBase {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            fall_distance: 0.0,
            fire: 0,
            air: FULL_AIR,
            invulnerable: false,
            portal_cooldown: 0,
            dimension: 0,
            custom_name: None,
            custom_name_visible: false,
            silent: false,
            command_stats: None,
        }
    }
}

impl EntityBase {
    /// A new entity facing `yaw`.
    pub fn facing(yaw: f32) -> Self {
        Self {
            yaw,
            ..Self::default()
        }
    }

    /// The base entity's metadata entries 0-4 (flags, air, custom name, name visible,
    /// silent).
    pub fn metadata(&self) -> Vec<Metadata> {
        use MetadataValue::{Byte, Short, String};
        let md = |index, value| Metadata { index, value };
        vec![
            md(0, Byte(0)),
            md(1, Short(self.air)),
            md(2, String(self.custom_name.clone().unwrap_or_default())),
            md(3, Byte(i8::from(self.custom_name_visible))),
            md(4, Byte(i8::from(self.silent))),
        ]
    }

    /// The fire bookkeeping at the end of `moveEntity`, for an entity touching neither
    /// fire nor lava: the counter rests at `-fireResistance`.
    ///
    /// TODO(M5): entities in fire or lava catch fire and burn.
    pub fn moved(&mut self) {
        if self.fire <= 0 {
            self.fire = -FIRE_RESISTANCE;
        }
    }

    /// `Entity.updateFallState` after a move of `moved_y` that ended `on_ground`.
    pub fn update_fall_state(&mut self, on_ground: bool, moved_y: f64) {
        if on_ground {
            self.fall_distance = 0.0;
        } else if moved_y < 0.0 {
            self.fall_distance = (f64::from(self.fall_distance) - moved_y) as f32;
        }
    }

    /// `handleWaterMovement` finding the entity in water: falls end and fire goes out.
    pub fn wet(&mut self) {
        self.fall_distance = 0.0;
        self.fire = 0;
    }

    /// `Entity.readFromNBT`: the base state, the position, motion and ground flag
    /// (into `body`), and the saved UUID if there is one.
    fn read(c: &Compound, body: &mut Body) -> (Self, Option<u128>) {
        let pos = doubles(c, "Pos");
        let mut motion = doubles(c, "Motion");
        for m in &mut motion {
            if m.abs() > MAX_LOADED_MOTION {
                *m = 0.0;
            }
        }
        let rotation = floats(c, "Rotation");
        body.pos = (pos[0], pos[1], pos[2]);
        body.motion = (motion[0], motion[1], motion[2]);
        body.on_ground = c.get_bool("OnGround");
        let uuid = if c.contains_key_of("UUIDMost", Some(TagType::Long))
            && c.contains_key_of("UUIDLeast", Some(TagType::Long))
        {
            Some(
                (c.get_i64("UUIDMost") as u64 as u128) << 64
                    | c.get_i64("UUIDLeast") as u64 as u128,
            )
        } else if c.contains_key_of("UUID", Some(TagType::String)) {
            parse_uuid(c.get_str("UUID"))
        } else {
            None
        };
        let name = c.get_str("CustomName");
        let base = Self {
            // setRotation keeps both angles within (-360, 360).
            yaw: rotation[0] % 360.0,
            pitch: rotation[1] % 360.0,
            fall_distance: c.get_f32("FallDistance"),
            fire: c.get_i16("Fire"),
            air: c.get_i16("Air"),
            invulnerable: c.get_bool("Invulnerable"),
            portal_cooldown: c.get_i32("PortalCooldown"),
            dimension: c.get_i32("Dimension"),
            custom_name: (!name.is_empty()).then(|| name.to_owned()),
            custom_name_visible: c.get_bool("CustomNameVisible"),
            silent: c.get_bool("Silent"),
            command_stats: c.get_compound("CommandStats").and_then(command_stats),
        };
        (base, uuid)
    }

    /// `Entity.writeToNBT` up to the entity's own fields.
    fn write(&self, body: &Body, uuid: u128, c: &mut Compound) {
        let (p, m) = (body.pos, body.motion);
        c.insert("Pos", double_list([p.0, p.1, p.2]));
        c.insert("Motion", double_list([m.0, m.1, m.2]));
        c.insert("Rotation", float_list([self.yaw, self.pitch]));
        c.insert("FallDistance", Tag::Float(self.fall_distance));
        c.insert("Fire", Tag::Short(self.fire));
        c.insert("Air", Tag::Short(self.air));
        c.insert("OnGround", Tag::Byte(i8::from(body.on_ground)));
        c.insert("Dimension", Tag::Int(self.dimension));
        c.insert("Invulnerable", Tag::Byte(i8::from(self.invulnerable)));
        c.insert("PortalCooldown", Tag::Int(self.portal_cooldown));
        c.insert("UUIDMost", Tag::Long((uuid >> 64) as u64 as i64));
        c.insert("UUIDLeast", Tag::Long(uuid as u64 as i64));
        if let Some(name) = &self.custom_name {
            c.insert("CustomName", Tag::String(name.clone()));
            c.insert(
                "CustomNameVisible",
                Tag::Byte(i8::from(self.custom_name_visible)),
            );
        }
        if let Some(stats) = &self.command_stats {
            c.insert("CommandStats", Tag::Compound(stats.clone()));
        }
        if self.silent {
            c.insert("Silent", Tag::Byte(1));
        }
    }
}

/// `CommandResultStats.readStatsFromNBT`: the types with both a name and an objective,
/// or `None` if there are none (nothing is saved then).
fn command_stats(c: &Compound) -> Option<Compound> {
    let mut out = Compound::new();
    for t in COMMAND_STAT_TYPES {
        let (name, objective) = (format!("{t}Name"), format!("{t}Objective"));
        if c.contains_key_of(&name, Some(TagType::String))
            && c.contains_key_of(&objective, Some(TagType::String))
        {
            out.insert(name.clone(), Tag::String(c.get_str(&name).to_owned()));
            out.insert(
                objective.clone(),
                Tag::String(c.get_str(&objective).to_owned()),
            );
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The three doubles of a `Pos` or `Motion` list (missing entries read as 0, like
/// `getDoubleAt` on vanilla's empty fallback list).
fn doubles(c: &Compound, key: &str) -> [f64; 3] {
    let list = c.get_list(key, TagType::Double);
    std::array::from_fn(|k| {
        list.and_then(|l| l.get(k))
            .and_then(Tag::as_f64)
            .unwrap_or(0.0)
    })
}

/// The two floats of a `Rotation` list.
fn floats(c: &Compound, key: &str) -> [f32; 2] {
    let list = c.get_list(key, TagType::Float);
    std::array::from_fn(|k| {
        list.and_then(|l| l.get(k))
            .and_then(Tag::as_f32)
            .unwrap_or(0.0)
    })
}

fn double_list<const N: usize>(values: [f64; N]) -> Tag {
    let mut l = List::new();
    for v in values {
        let _ = l.push(Tag::Double(v));
    }
    Tag::List(l)
}

fn float_list<const N: usize>(values: [f32; N]) -> Tag {
    let mut l = List::new();
    for v in values {
        let _ = l.push(Tag::Float(v));
    }
    Tag::List(l)
}

/// `UUID.fromString` for the canonical form (old saves' `UUID` string).
fn parse_uuid(s: &str) -> Option<u128> {
    let hex: String = s.chars().filter(|&ch| ch != '-').collect();
    (hex.len() == 32)
        .then(|| u128::from_str_radix(&hex, 16).ok())
        .flatten()
}

/// The saved id with the pre-1.6 minecart id resolved (`createEntityFromNBT` rewrites
/// it and drops the `Type`).
fn resolve_legacy_id(c: &mut Compound) {
    if c.get_str("id") == LEGACY_MINECART_ID {
        let kind = usize::try_from(c.get_i32("Type")).ok();
        let id = kind
            .and_then(|k| MINECART_IDS.get(k))
            .unwrap_or(&MINECART_IDS[0]);
        c.insert("id", Tag::String((*id).to_owned()));
        c.remove("Type");
    }
}

/// The chunk section an entity at height `y` belongs to (`Chunk.addEntity`).
fn section_of(y: f64) -> usize {
    floor_f64(y / 16.0).clamp(0, SECTIONS as i32 - 1) as usize
}

/// Which entities each chunk section holds (vanilla `Chunk.entityLists`), and the saved
/// entities krol does not simulate.
#[derive(Debug, Default)]
pub struct ChunkEntities {
    /// Entity ids per chunk and section, in the order they were added.
    lists: HashMap<(i32, i32), [Vec<i32>; SECTIONS]>,
    /// Where each entity is listed (`chunkCoordX/Y/Z`).
    slots: HashMap<i32, (i32, i32, usize)>,
    /// Saved entities krol only keeps (by entity id), written back as read.
    kept: HashMap<i32, Compound>,
}

impl ChunkEntities {
    fn add(&mut self, id: i32, slot: (i32, i32, usize)) {
        self.lists.entry((slot.0, slot.1)).or_default()[slot.2].push(id);
        self.slots.insert(id, slot);
    }

    fn remove(&mut self, id: i32) {
        if let Some((cx, cz, section)) = self.slots.remove(&id)
            && let Some(lists) = self.lists.get_mut(&(cx, cz))
        {
            lists[section].retain(|&e| e != id);
            if lists.iter().all(Vec::is_empty) {
                self.lists.remove(&(cx, cz));
            }
        }
    }
}

impl Game {
    /// Where a simulated non-player entity is.
    fn entity_position(&self, id: i32) -> Option<(f64, f64, f64)> {
        if let Some(it) = self.items.get(&id) {
            Some(it.body.pos)
        } else if let Some(o) = self.orbs.get(&id) {
            Some(o.body.pos)
        } else if let Some(f) = self.falling.get(&id) {
            Some(f.body.pos)
        } else {
            self.tnt.get(&id).map(|t| t.body.pos)
        }
    }

    /// Lists the entity in the chunk section it is in (`Chunk.addEntity`), which marks
    /// the chunk as having entities.
    fn add_entity_to_chunk(&mut self, id: i32, pos: (f64, f64, f64)) {
        let (cx, cz) = (floor_f64(pos.0 / 16.0), floor_f64(pos.2 / 16.0));
        self.level.overworld.mark_has_entities(cx, cz);
        self.chunk_entities.add(id, (cx, cz, section_of(pos.1)));
    }

    /// `spawnEntityInWorld` for a new non-player entity already in its map: it joins its
    /// chunk and the entity tracker.
    pub(super) fn spawn_entity_in_world(&mut self, id: i32) {
        if let Some(pos) = self.entity_position(id) {
            self.add_entity_to_chunk(id, pos);
        }
        self.track_entity(id);
    }

    /// `updateEntityWithOptionalForce` after the entity's update: an entity that crossed
    /// into another chunk section moves to the end of that section's list.
    pub(super) fn relocate_entity(&mut self, id: i32) {
        let Some(pos) = self.entity_position(id) else {
            return;
        };
        let slot = (
            floor_f64(pos.0 / 16.0),
            floor_f64(pos.2 / 16.0),
            section_of(pos.1),
        );
        if self.chunk_entities.slots.get(&id) != Some(&slot) {
            self.chunk_entities.remove(id);
            self.level.overworld.mark_has_entities(slot.0, slot.1);
            self.chunk_entities.add(id, slot);
        }
    }

    /// A dead entity leaves its chunk (`Chunk.removeEntity` in `updateEntities`).
    pub(super) fn remove_entity_from_chunk(&mut self, id: i32) {
        self.chunk_entities.remove(id);
    }

    /// Creates the entities saved in chunks loaded since the last call
    /// (`AnvilChunkLoader.loadEntities` and `Chunk.onChunkLoad`). Entity ids follow the
    /// saved order; the new entities join the tracker section by section.
    pub(super) fn load_chunk_entities(&mut self) {
        for ((cx, cz), list) in self.level.overworld.take_loaded_entities() {
            let mut created = Vec::new();
            for tag in &list {
                let Tag::Compound(saved) = tag else { continue };
                let mut c = saved.clone();
                resolve_legacy_id(&mut c);
                if !self.create_saved_entity((cx, cz), &c, &mut created) {
                    continue;
                }
                // Passengers' mounts are saved inside them (`Riding`); vanilla creates
                // each mount as an entity of the chunk. krol keeps the whole stack in
                // the rider's NBT, so the mounts only take their entity ids.
                let mut riding = c.get_compound("Riding");
                while let Some(mount) = riding {
                    let mut m = mount.clone();
                    resolve_legacy_id(&mut m);
                    if KNOWN_IDS.contains(&m.get_str("id")) {
                        let _ = self.alloc_entity_id();
                    } else {
                        warn!("Skipping Entity with id {}", m.get_str("id"));
                    }
                    riding = mount.get_compound("Riding");
                }
            }
            // Chunk.onChunkLoad: section by section into the world and the tracker.
            created.sort_by_key(|&(section, _)| section);
            for (_, id) in created {
                if self.entity_position(id).is_some() {
                    self.track_entity(id);
                }
            }
        }
    }

    /// `EntityList.createEntityFromNBT` + `Chunk.addEntity` for one entity saved in
    /// `chunk`; `false` if vanilla does not know the saved id.
    fn create_saved_entity(
        &mut self,
        chunk: (i32, i32),
        c: &Compound,
        created: &mut Vec<(usize, i32)>,
    ) -> bool {
        let kind = c.get_str("id");
        if !KNOWN_IDS.contains(&kind) {
            warn!("Skipping Entity with id {kind}");
            return false;
        }
        let id = self.alloc_entity_id();
        let mut body = Body::player((0.0, 0.0, 0.0), false);
        body.step_height = 0.0;
        let pos = match kind {
            ITEM_ID => {
                // EntityItem's hoverStart is drawn in its constructor.
                let _hover_start = self.math_random.next_double();
                body.width = super::item_entity::SIZE;
                body.height = super::item_entity::SIZE;
                let (base, uuid) = EntityBase::read(c, &mut body);
                self.set_saved_uuid(id, uuid);
                let stack = c
                    .get_compound("Item")
                    .and_then(super::player::item_from_nbt);
                let pos = body.pos;
                let item = ItemEntity::load(body, base, c, stack);
                self.items.insert(id, item);
                pos
            }
            ORB_ID => {
                // Vanilla quirk: the world-only constructor makes loaded orbs 0.25
                // wide, half the size of spawned ones.
                body.width = super::xp_orb::LOADED_SIZE;
                body.height = super::xp_orb::LOADED_SIZE;
                let (base, uuid) = EntityBase::read(c, &mut body);
                self.set_saved_uuid(id, uuid);
                let pos = body.pos;
                self.orbs.insert(id, XpOrb::load(body, base, c));
                pos
            }
            FALLING_ID => {
                body.width = super::falling_block::SIZE;
                body.height = super::falling_block::SIZE;
                let (base, uuid) = EntityBase::read(c, &mut body);
                self.set_saved_uuid(id, uuid);
                let pos = body.pos;
                self.falling.insert(id, FallingBlock::load(body, base, c));
                pos
            }
            TNT_ID => {
                body.width = super::tnt::SIZE;
                body.height = super::tnt::SIZE;
                let (base, uuid) = EntityBase::read(c, &mut body);
                self.set_saved_uuid(id, uuid);
                let pos = body.pos;
                self.tnt.insert(id, PrimedTnt::load(body, base, c));
                pos
            }
            _ => {
                let p = doubles(c, "Pos");
                self.entity_uuids.remove(&id);
                self.chunk_entities.kept.insert(id, c.clone());
                (p[0], p[1], p[2])
            }
        };
        // Chunk.addEntity: an entity saved in the wrong chunk is set dead (it still
        // joins this chunk's list, and is not saved again).
        let (cx, cz) = (floor_f64(pos.0 / 16.0), floor_f64(pos.2 / 16.0));
        if (cx, cz) != chunk {
            self.set_entity_dead(id);
            self.chunk_entities.kept.remove(&id);
        }
        let section = section_of(pos.1);
        self.level.overworld.mark_has_entities(chunk.0, chunk.1);
        self.chunk_entities.add(id, (chunk.0, chunk.1, section));
        created.push((section, id));
        true
    }

    /// Replaces the random UUID a loaded entity got in its constructor with the saved
    /// one.
    fn set_saved_uuid(&mut self, id: i32, uuid: Option<u128>) {
        if let Some(uuid) = uuid {
            self.entity_uuids.insert(id, uuid);
        }
    }

    /// The `Entities` list of every chunk that holds entities: per section, in list
    /// order, every live entity that saves (`writeToNBTOptional`; players do not).
    pub(super) fn chunk_entity_nbt(&self) -> HashMap<(i32, i32), List> {
        let mut out = HashMap::new();
        for (&chunk, lists) in &self.chunk_entities.lists {
            let mut list = List::new();
            for &id in lists.iter().flatten() {
                if let Some(c) = self.saved_entity_nbt(id) {
                    let _ = list.push(Tag::Compound(c));
                }
            }
            out.insert(chunk, list);
        }
        out
    }

    /// One entity as saved, or `None` if it is dead or does not save.
    fn saved_entity_nbt(&self, id: i32) -> Option<Compound> {
        if let Some(c) = self.chunk_entities.kept.get(&id) {
            return Some(c.clone());
        }
        let uuid = self.entity_uuids.get(&id).copied().unwrap_or_default();
        let mut c = Compound::new();
        if let Some(it) = self.items.get(&id) {
            if it.dead {
                return None;
            }
            c.insert("id", Tag::String(ITEM_ID.into()));
            it.base.write(&it.body, uuid, &mut c);
            it.write(&mut c);
        } else if let Some(o) = self.orbs.get(&id) {
            if o.dead {
                return None;
            }
            c.insert("id", Tag::String(ORB_ID.into()));
            o.base.write(&o.body, uuid, &mut c);
            o.write(&mut c);
        } else if let Some(f) = self.falling.get(&id) {
            if f.dead {
                return None;
            }
            c.insert("id", Tag::String(FALLING_ID.into()));
            f.base.write(&f.body, uuid, &mut c);
            f.write(&mut c);
        } else {
            let t = self.tnt.get(&id).filter(|t| !t.dead)?;
            c.insert("id", Tag::String(TNT_ID.into()));
            t.base.write(&t.body, uuid, &mut c);
            t.write(&mut c);
        }
        Some(c)
    }

    /// Saves the level's chunks (with their entities) and `level.dat`; `log` adds
    /// vanilla's per-dimension lines (`saveAllWorlds(false)` at shutdown).
    pub(super) fn save_level(&mut self, log: bool) {
        self.load_chunk_entities();
        let entities = self.chunk_entity_nbt();
        if log {
            self.level.save_all(entities);
        } else {
            self.level.save_quietly(entities);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_minecarts_get_their_type_name() {
        let mut c = Compound::new();
        c.insert("id", Tag::String("Minecart".into()));
        c.insert("Type", Tag::Int(5));
        resolve_legacy_id(&mut c);
        assert_eq!(c.get_str("id"), "MinecartHopper");
        assert!(!c.contains_key("Type"));
        c.insert("id", Tag::String("Minecart".into()));
        c.insert("Type", Tag::Int(42));
        resolve_legacy_id(&mut c);
        assert_eq!(c.get_str("id"), "MinecartRideable");
    }

    #[test]
    fn sections_clamp_to_the_chunk() {
        assert_eq!(section_of(-5.0), 0);
        assert_eq!(section_of(15.9), 0);
        assert_eq!(section_of(16.0), 1);
        assert_eq!(section_of(300.0), 15);
    }

    #[test]
    fn base_round_trips() {
        let mut c = Compound::new();
        c.insert("Pos", double_list([1.5, 64.0, -3.25]));
        c.insert("Motion", double_list([0.1, 11.0, -0.2]));
        c.insert("Rotation", float_list([370.0, -10.0]));
        c.insert("Air", Tag::Short(300));
        c.insert("Fire", Tag::Short(-1));
        c.insert("OnGround", Tag::Byte(1));
        c.insert("UUIDMost", Tag::Long(-5));
        c.insert("UUIDLeast", Tag::Long(7));
        c.insert("CustomName", Tag::String("Bob".into()));
        let mut body = Body::player((0.0, 0.0, 0.0), false);
        let (base, uuid) = EntityBase::read(&c, &mut body);
        assert_eq!(body.pos, (1.5, 64.0, -3.25));
        // Motion beyond 10 is dropped.
        assert_eq!(body.motion, (0.1, 0.0, -0.2));
        assert!(body.on_ground);
        assert_eq!(base.yaw, 10.0);
        assert_eq!(base.fire, -1);
        let uuid = uuid.expect("saved uuid");
        let mut out = Compound::new();
        base.write(&body, uuid, &mut out);
        assert_eq!(out.get_i64("UUIDMost"), -5);
        assert_eq!(out.get_i64("UUIDLeast"), 7);
        assert_eq!(out.get_str("CustomName"), "Bob");
        assert!(!out.contains_key("Silent"));
        assert!(!out.contains_key("CommandStats"));
    }

    #[test]
    fn uuid_strings_parse() {
        assert_eq!(
            parse_uuid("00000000-0000-0001-0000-000000000002"),
            Some(1u128 << 64 | 2)
        );
        assert_eq!(parse_uuid("nope"), None);
    }
}
