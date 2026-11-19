//! Paintings, item frames and leash knots (vanilla: `EntityHanging` with
//! `EntityPainting` and `EntityItemFrame`, placed by `ItemHangingEntity`, and
//! `EntityLeashKnot`, tied by leads; see [`super::mob`]).
//!
//! A hanging entity belongs to the block space in front of a wall (its hanging
//! position) and faces away from the wall. It never moves: every 101 updates it checks
//! that its wall is still there and nothing overlaps it, and breaks otherwise. Any hit
//! that can hurt it breaks it (an item frame holding an item loses the item first),
//! dropping its item unless the game rule `doEntityDrops` is off or a creative player
//! broke it.
//!
//! A filled map in a frame is shown to every player by the tracker (see
//! [`super::maps`]).
//!
//! A leash knot sits in the middle of a fence block, has no facing and is never saved
//! (mobs leashed to it remember the fence and tie a new one when loaded). It breaks
//! when its fence is gone or it is hit, and drops nothing itself.
//!
//! TODO(M5): hanging entities pushed by pistons.

use std::collections::BTreeSet;

use krol_java::math::floor_f64;
use krol_nbt::{Compound, Tag};
use krol_protocol::packets::play::{ItemStack, Metadata, MetadataValue};

use super::Game;
use super::damage::DamageSource;
use super::ids::{block, item};
use super::physics::{Aabb, Body};
use super::saved_entities::EntityBase;

/// Saved ids (`EntityList` names).
pub const PAINTING_ID: &str = "Painting";
pub const ITEM_FRAME_ID: &str = "ItemFrame";
/// `S0EPacketSpawnObject` type of item frames.
pub const ITEM_FRAME_OBJECT: i8 = 71;
/// The `EntityList` name of leash knots (never saved, but `@e[type=...]` sees it).
pub const LEASH_KNOT_ID: &str = "LeashKnot";
/// `S0EPacketSpawnObject` type of leash knots.
pub const LEASH_KNOT_OBJECT: i8 = 77;
/// A leash knot's picture size in pixels (`getWidthPixels`, `getHeightPixels`).
const KNOT_PIXELS: i32 = 9;
/// A leash knot's box: this far out from the middle of the fence horizontally, and
/// this far up and down from a point this much above the middle.
const KNOT_HALF_WIDTH: f64 = 0.1875;
const KNOT_HALF_HEIGHT: f64 = 0.25;
const KNOT_RAISE: f64 = 0.125;
/// `setSize(0.5F, 0.5F)`: the size that counts for the eye height.
const SIZE: f32 = 0.5;
/// The entity sits this far from the middle of its block space towards the wall.
const WALL_OFFSET: f64 = 0.46875;
/// An item frame's picture is 12 pixels square.
const FRAME_PIXELS: i32 = 12;
/// A hanging entity checks its surface when its counter reaches this
/// (`tickCounter1++ == 100`, so every 101 updates).
const SURVIVAL_CHECK: i32 = 100;
/// Item frame metadata: the shown item, and its rotation in eighths of a turn.
const FRAME_ITEM_INDEX: u8 = 8;
const FRAME_ROTATION_INDEX: u8 = 9;
const FRAME_ROTATIONS: i8 = 8;
/// The pickup delay of dropped items (`setDefaultPickupDelay`).
const DROP_PICKUP_DELAY: i32 = 10;
/// The creative game mode.
const CREATIVE: i32 = 1;
/// The spectator game mode.
const SPECTATOR: i32 = 3;

/// The 26 paintings (`EntityPainting.EnumArt`, in order): title, width and height in
/// pixels.
pub const ARTS: [(&str, i32, i32); 26] = [
    ("Kebab", 16, 16),
    ("Aztec", 16, 16),
    ("Alban", 16, 16),
    ("Aztec2", 16, 16),
    ("Bomb", 16, 16),
    ("Plant", 16, 16),
    ("Wasteland", 16, 16),
    ("Pool", 32, 16),
    ("Courbet", 32, 16),
    ("Sea", 32, 16),
    ("Sunset", 32, 16),
    ("Creebet", 32, 16),
    ("Wanderer", 16, 32),
    ("Graham", 16, 32),
    ("Match", 32, 32),
    ("Bust", 32, 32),
    ("Stage", 32, 32),
    ("Void", 32, 32),
    ("SkullAndRoses", 32, 32),
    ("Wither", 32, 32),
    ("Fighters", 64, 32),
    ("Pointer", 64, 64),
    ("Pigscene", 64, 64),
    ("BurningSkull", 64, 64),
    ("Skeleton", 64, 48),
    ("DonkeyKong", 64, 48),
];

/// What hangs.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// A painting showing `ARTS[art]`.
    Painting { art: usize },
    /// An item frame.
    ItemFrame {
        /// The shown item (one of it, metadata entry 8).
        item: Option<ItemStack>,
        /// The item's rotation (metadata entry 9; `% 8` as Java computes it).
        rotation: i8,
        /// `itemDropChance`.
        drop_chance: f32,
    },
    /// A leash knot on a fence.
    LeashKnot,
}

/// One painting or item frame.
#[derive(Debug, Clone)]
pub struct Hanging {
    /// What it is.
    pub kind: Kind,
    /// The block space it hangs in (`hangingPosition`).
    pub tile: (i32, i32, i32),
    /// The direction it faces, as a horizontal index (south, west, north, east).
    pub facing: u8,
    /// Position (the middle of the picture), saved motion and ground flag.
    pub body: Body,
    /// Its box (vanilla computes it from the hanging position, not the position).
    pub bb: Aabb,
    /// The base entity state (yaw follows the facing).
    pub base: EntityBase,
    /// `tickCounter1`.
    pub tick_counter: i32,
    /// `isAirBorne`: set when it is created or moved to another block space; the
    /// tracker clears it.
    pub airborne: bool,
    /// Metadata entries not sent since they last changed (`WatchableObject.watched`:
    /// all of them at first, so the first change sends everything).
    pub dirty_metadata: BTreeSet<u8>,
    /// Some metadata changed since the tracker last looked (`objectChanged`).
    pub metadata_changed: bool,
    /// Removed from the world (`isDead`).
    pub dead: bool,
    /// The chunk it is in.
    pub chunk: (i32, i32),
}

/// The offset of a horizontal index (`EnumFacing.getFrontOffsetX/Z`).
fn front(facing: u8) -> (i32, i32) {
    match facing & 3 {
        0 => (0, 1),
        1 => (-1, 0),
        2 => (0, -1),
        _ => (1, 0),
    }
}

/// `EnumFacing.rotateYCCW` of a horizontal index.
fn ccw(facing: u8) -> u8 {
    (facing + 3) & 3
}

/// `EnumFacing.getHorizontal`: an index of any sign wraps (`abs(i % 4)`).
fn horizontal(i: i32) -> u8 {
    (i % 4).unsigned_abs() as u8
}

/// The horizontal index of a block face (`EnumFacing` index: down, up, north, south,
/// west, east), if it is horizontal.
fn face_to_horizontal(face: u8) -> Option<u8> {
    match face {
        super::placement::SOUTH => Some(0),
        super::placement::WEST => Some(1),
        super::placement::NORTH => Some(2),
        super::placement::EAST => Some(3),
        _ => None,
    }
}

/// Half a block for sizes of whole blocks in pairs (`func_174858_a`): pictures an even
/// number of blocks wide or high sit between two block spaces.
fn half_offset(pixels: i32) -> f64 {
    if pixels % 32 == 0 { 0.5 } else { 0.0 }
}

impl Kind {
    /// The picture's width and height in pixels.
    fn pixels(&self) -> (i32, i32) {
        match self {
            Self::Painting { art } => {
                let (_, w, h) = ARTS[*art];
                (w, h)
            }
            Self::ItemFrame { .. } => (FRAME_PIXELS, FRAME_PIXELS),
            Self::LeashKnot => (KNOT_PIXELS, KNOT_PIXELS),
        }
    }

    /// The saved entity id.
    pub fn saved_id(&self) -> &'static str {
        match self {
            Self::Painting { .. } => PAINTING_ID,
            Self::ItemFrame { .. } => ITEM_FRAME_ID,
            Self::LeashKnot => LEASH_KNOT_ID,
        }
    }

    /// `getName`: the translated `entity.<id>.name` (en_US has no item frame name).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Painting { .. } => "Painting",
            Self::ItemFrame { .. } => "entity.ItemFrame.name",
            Self::LeashKnot => "entity.LeashKnot.name",
        }
    }
}

impl Hanging {
    /// A new hanging entity in `tile` facing `facing` (`updateFacingWithBoundingBox`).
    fn new(kind: Kind, tile: (i32, i32, i32), facing: u8) -> Self {
        let mut body = Body::player((0.0, 0.0, 0.0), false);
        body.width = SIZE;
        body.height = SIZE;
        body.step_height = 0.0;
        let mut h = Self {
            kind,
            tile,
            facing,
            body,
            bb: Aabb::new([0.0; 3], [0.0; 3]),
            base: EntityBase::default(),
            tick_counter: 0,
            airborne: true,
            dirty_metadata: BTreeSet::new(),
            metadata_changed: false,
            dead: false,
            chunk: (0, 0),
        };
        h.set_facing(facing);
        h.dirty_metadata = h.metadata().iter().map(|m| m.index).collect();
        h
    }

    /// `EntityLeashKnot`'s constructor: a knot in the middle of the fence at `tile`,
    /// with its own box and no facing.
    pub fn knot(tile: (i32, i32, i32)) -> Self {
        let mut h = Self::new(Kind::LeashKnot, tile, 0);
        let (x, y, z) = (
            f64::from(tile.0) + 0.5,
            f64::from(tile.1) + 0.5,
            f64::from(tile.2) + 0.5,
        );
        h.body.pos = (x, y, z);
        h.base.yaw = 0.0;
        h.bb = Aabb::new(
            [
                x - KNOT_HALF_WIDTH,
                y - KNOT_HALF_HEIGHT + KNOT_RAISE,
                z - KNOT_HALF_WIDTH,
            ],
            [
                x + KNOT_HALF_WIDTH,
                y + KNOT_HALF_HEIGHT + KNOT_RAISE,
                z + KNOT_HALF_WIDTH,
            ],
        );
        h.chunk = (floor_f64(x) >> 4, floor_f64(z) >> 4);
        h
    }

    /// `DataWatcher.updateObject` / `setObjectWatched` of entry `index`.
    fn metadata_updated(&mut self, index: u8) {
        self.dirty_metadata.insert(index);
        self.metadata_changed = true;
    }

    /// `updateFacingWithBoundingBox`: faces `facing`, turned to match, and places the
    /// position and box.
    fn set_facing(&mut self, facing: u8) {
        self.facing = facing;
        self.base.yaw = f32::from(facing) * 90.0;
        self.update_box();
    }

    /// `updateBoundingBox`: the middle of the picture, moved against the wall and
    /// shifted half a block for even sizes, and a box 1/32 thick.
    fn update_box(&mut self) {
        // EntityLeashKnot has no facing: its box stays the one it was made with.
        if self.kind == Kind::LeashKnot {
            return;
        }
        let (w, h) = self.kind.pixels();
        let (fx, fz) = front(self.facing);
        let (cx, cz) = front(ccw(self.facing));
        let mut x = f64::from(self.tile.0) + 0.5;
        let mut y = f64::from(self.tile.1) + 0.5;
        let mut z = f64::from(self.tile.2) + 0.5;
        x -= f64::from(fx) * WALL_OFFSET;
        z -= f64::from(fz) * WALL_OFFSET;
        y += half_offset(h);
        x += half_offset(w) * f64::from(cx);
        z += half_offset(w) * f64::from(cz);
        self.body.pos = (x, y, z);
        let (mut dx, dy, mut dz) = (f64::from(w), f64::from(h), f64::from(w));
        // Facing along z, the box is thin in z; along x, thin in x.
        if self.facing & 1 == 0 {
            dz = 1.0;
        } else {
            dx = 1.0;
        }
        let (dx, dy, dz) = (dx / 32.0, dy / 32.0, dz / 32.0);
        self.bb = Aabb::new([x - dx, y - dy, z - dz], [x + dx, y + dy, z + dz]);
        self.chunk = (floor_f64(x) >> 4, floor_f64(z) >> 4);
    }

    /// The metadata: the base entity's, and for item frames the item and rotation.
    pub fn metadata(&self) -> Vec<Metadata> {
        let mut md = self.base.metadata();
        if let Kind::ItemFrame { item, rotation, .. } = &self.kind {
            md.push(Metadata {
                index: FRAME_ITEM_INDEX,
                value: MetadataValue::Item(item.clone()),
            });
            md.push(Metadata {
                index: FRAME_ROTATION_INDEX,
                value: MetadataValue::Byte(*rotation),
            });
        }
        md
    }

    /// `EntityItemFrame.func_174866_q`: the signal comparators read (0 when empty, else
    /// the rotation plus one).
    fn frame_signal(&self) -> Option<i32> {
        match &self.kind {
            Kind::ItemFrame { item: None, .. } => Some(0),
            Kind::ItemFrame { rotation, .. } => {
                Some(i32::from(*rotation) % i32::from(FRAME_ROTATIONS) + 1)
            }
            Kind::Painting { .. } | Kind::LeashKnot => None,
        }
    }

    /// `readEntityFromNBT` of paintings and item frames, for one loaded with its chunk
    /// (`body` and `base` already read).
    pub fn load(saved_id: &str, body: Body, base: EntityBase, c: &Compound) -> Self {
        let kind = if saved_id == PAINTING_ID {
            // An unknown title shows the first painting.
            let motive = c.get_str("Motive");
            let art = ARTS.iter().position(|a| a.0 == motive).unwrap_or(0);
            Kind::Painting { art }
        } else {
            let mut item = None;
            let mut rotation = 0i8;
            let mut drop_chance = 1.0;
            if let Some(saved) = c.get_compound("Item").filter(|s| !s.is_empty()) {
                item = super::player::item_from_nbt(saved).map(|mut s| {
                    s.count = 1;
                    s
                });
                rotation = c.get_i8("ItemRotation") % FRAME_ROTATIONS;
                if c.contains_key_of("ItemDropChance", None) {
                    drop_chance = c.get_f32("ItemDropChance");
                }
                // Old saves turned items in quarters.
                if c.contains_key("Direction") {
                    rotation = rotation.wrapping_mul(2) % FRAME_ROTATIONS;
                }
            }
            Kind::ItemFrame {
                item,
                rotation,
                drop_chance,
            }
        };
        let mut tile = (c.get_i32("TileX"), c.get_i32("TileY"), c.get_i32("TileZ"));
        let facing = if c.contains_key_of("Direction", None) {
            // Old saves kept the wall block and a direction.
            let f = horizontal(i32::from(c.get_i8("Direction")));
            let (fx, fz) = front(f);
            tile = (tile.0 + fx, tile.1, tile.2 + fz);
            f
        } else if c.contains_key_of("Facing", None) {
            horizontal(i32::from(c.get_i8("Facing")))
        } else {
            horizontal(i32::from(c.get_i8("Dir")))
        };
        let mut h = Self::new(kind, tile, facing);
        h.body.motion = body.motion;
        h.body.on_ground = body.on_ground;
        let yaw = h.base.yaw;
        // Reading the base state and a framed item updates the data watcher where the
        // values differ from a new entity's, which the tracker then sends once more.
        h.metadata_changed = base.air != super::saved_entities::FULL_AIR
            || base.custom_name.is_some()
            || base.custom_name_visible
            || base.silent
            || matches!(h.kind, Kind::ItemFrame { item: Some(_), .. });
        h.base = base;
        h.base.yaw = yaw;
        h
    }

    /// `writeEntityToNBT` of paintings and item frames.
    pub fn write(&self, c: &mut Compound) {
        match &self.kind {
            Kind::Painting { art } => {
                c.insert("Motive", Tag::String(ARTS[*art].0.to_owned()));
            }
            Kind::ItemFrame {
                item,
                rotation,
                drop_chance,
            } => {
                if let Some(item) = item {
                    c.insert("Item", Tag::Compound(super::item_data::stack_nbt(item)));
                    c.insert("ItemRotation", Tag::Byte(*rotation));
                    c.insert("ItemDropChance", Tag::Float(*drop_chance));
                }
            }
            // Never saved (writeToNBTOptional is false).
            Kind::LeashKnot => {}
        }
        c.insert("Facing", Tag::Byte(self.facing as i8));
        c.insert("TileX", Tag::Int(self.tile.0));
        c.insert("TileY", Tag::Int(self.tile.1));
        c.insert("TileZ", Tag::Int(self.tile.2));
    }
}

/// Whether an item places a hanging entity (`ItemHangingEntity`).
pub fn is_hanging_item(id: i16) -> bool {
    matches!(id, item::PAINTING | item::ITEM_FRAME)
}

impl Game {
    /// `EntityHanging.onValidSurface`: nothing collides with its box, every block of
    /// the wall behind it is solid (or a repeater), and no other hanging entity
    /// overlaps it.
    fn on_valid_surface(&mut self, h: &Hanging, me: Option<i32>) -> bool {
        // EntityLeashKnot.onValidSurface: only that its fence is still there.
        if h.kind == Kind::LeashKnot {
            let id = self.level.overworld.block(h.tile.0, h.tile.1, h.tile.2) >> 4;
            return super::blocks::is_fence(id);
        }
        if !super::physics::colliding_boxes(&mut self.level.overworld, &h.bb).is_empty() {
            return false;
        }
        let (w, ht) = h.kind.pixels();
        let (across, up) = ((w / 16).max(1), (ht / 16).max(1));
        let (fx, fz) = front(h.facing);
        let (cx, cz) = front(ccw(h.facing));
        let wall = (h.tile.0 - fx, h.tile.1, h.tile.2 - fz);
        // Vanilla quirk: the wall is checked from the hanging position on in the
        // counter-clockwise direction and upwards, while wide or tall pictures are
        // centered on it.
        for a in 0..across {
            for b in 0..up {
                let (x, y, z) = (wall.0 + cx * a, wall.1 + b, wall.2 + cz * a);
                let id = self.level.overworld.block(x, y, z) >> 4;
                if !super::block_data::material(id).is_solid()
                    && !matches!(id, block::UNPOWERED_REPEATER | block::POWERED_REPEATER)
                {
                    return false;
                }
            }
        }
        !self
            .hanging
            .iter()
            .any(|(&id, other)| Some(id) != me && !other.dead && other.bb.intersects(&h.bb))
    }

    /// `ItemHangingEntity.onItemUse`: on the side of a block, a painting (of a random
    /// art that fits) or an item frame hangs in front of it if it fits there. The item
    /// counts as used even when nothing fits.
    pub(super) fn place_hanging(&mut self, i: usize, pos: (i32, i32, i32), face: u8) {
        let Some(facing) = face_to_horizontal(face) else {
            return;
        };
        let Some(stack) = self.players[i].held().cloned() else {
            return;
        };
        let tile = super::placement::offset(pos, face);
        // The entity (with its id) is created before it is known whether it fits.
        let id = self.next_entity_id();
        let hanging = if stack.id == item::PAINTING {
            // EntityPainting's constructor: every art that fits, picked with the
            // entity's own random (unseeded in vanilla); the last art when none fits.
            let mut fitting = Vec::new();
            for art in 0..ARTS.len() {
                let h = Hanging::new(Kind::Painting { art }, tile, facing);
                if self.on_valid_surface(&h, None) {
                    fitting.push(art);
                }
            }
            let art = if fitting.is_empty() {
                ARTS.len() - 1
            } else {
                fitting[self.entity_rand.next_int_bounded(fitting.len() as i32) as usize]
            };
            Hanging::new(Kind::Painting { art }, tile, facing)
        } else {
            Hanging::new(
                Kind::ItemFrame {
                    item: None,
                    rotation: 0,
                    drop_chance: 1.0,
                },
                tile,
                facing,
            )
        };
        if self.on_valid_surface(&hanging, None) {
            self.hanging.insert(id, hanging);
            self.spawn_entity_in_world(id);
            // The creative stack is restored afterwards.
            let held = 36 + self.players[i].selected;
            if self.players[i].gamemode != CREATIVE
                && let Some(s) = self.players[i].inventory[held].as_mut()
            {
                s.count -= 1;
            }
        } else {
            self.entity_uuids.remove(&id);
        }
        self.count_item_use(i);
    }

    /// One hanging entity's turn in the entity loop (`EntityHanging.onUpdate`).
    pub(super) fn update_hanging(&mut self, id: i32) {
        if let Some(h) = self.hanging.get_mut(&id)
            && !h.dead
        {
            let counter = h.tick_counter;
            h.tick_counter += 1;
            if counter == SURVIVAL_CHECK {
                h.tick_counter = 0;
                let h = h.clone();
                if !self.on_valid_surface(&h, Some(id)) {
                    self.hanging_broken(id, None);
                }
            }
        }
        if self.hanging.get(&id).is_some_and(|h| h.dead) {
            self.hanging.remove(&id);
            self.remove_entity_from_chunk(id);
            self.untrack_entity(id);
        }
    }

    /// Whether the entity attributed with damage is a creative player.
    fn creative_attacker(&self, attacker: Option<i32>) -> bool {
        attacker.is_some_and(|a| {
            self.players
                .iter()
                .any(|p| p.entity_id == a && p.gamemode == CREATIVE)
        })
    }

    /// `attackEntityFrom` of paintings and item frames: an item frame showing an item
    /// loses it (except to explosions); otherwise the entity breaks. An invulnerable
    /// one only breaks for the void and creative players. Returns whether it was hit.
    pub(super) fn attack_hanging(&mut self, id: i32, source: DamageSource) -> bool {
        let Some(h) = self.hanging.get(&id) else {
            return false;
        };
        let attacker = source.attacker();
        if h.base.invulnerable
            && source != DamageSource::OutOfWorld
            && !self.creative_attacker(attacker)
        {
            return false;
        }
        if !source.explosion()
            && let Kind::ItemFrame { item: Some(_), .. } = h.kind
        {
            self.drop_frame_contents(id, attacker, false);
            self.set_frame_item(id, None);
            return true;
        }
        if !h.dead {
            self.hanging_broken(id, attacker);
        }
        true
    }

    /// The entity dies and `onBroken` drops its item.
    fn hanging_broken(&mut self, id: i32, by: Option<i32>) {
        let Some(h) = self.hanging.get_mut(&id) else {
            return;
        };
        h.dead = true;
        match h.kind {
            Kind::Painting { .. } => {
                let pos = h.body.pos;
                if self.level.data.game_rule("doEntityDrops") != Some("false")
                    && !self.creative_attacker(by)
                {
                    self.spawn_item(pos, single(item::PAINTING), DROP_PICKUP_DELAY);
                }
            }
            Kind::ItemFrame { .. } => self.drop_frame_contents(id, by, true),
            // The leads tied to it drop from their mobs once they see it gone.
            Kind::LeashKnot => {}
        }
    }

    /// `EntityItemFrame.dropItemOrSelf`: the frame itself (when broken) and its item
    /// (by the drop chance, rolled on the entity's random), unless entity drops are off
    /// or a creative player did it.
    fn drop_frame_contents(&mut self, id: i32, by: Option<i32>, with_frame: bool) {
        if self.level.data.game_rule("doEntityDrops") == Some("false") {
            return;
        }
        let Some(h) = self.hanging.get(&id) else {
            return;
        };
        let Kind::ItemFrame {
            item, drop_chance, ..
        } = &h.kind
        else {
            return;
        };
        let (pos, shown, chance) = (h.body.pos, item.clone(), *drop_chance);
        if self.creative_attacker(by) {
            // The frame keeps showing its item (vanilla mutates the shown stack).
            if let Some(mut shown) = shown {
                self.remove_frame_icon(&mut shown, id);
                if let Some(h) = self.hanging.get_mut(&id)
                    && let Kind::ItemFrame { item: Some(s), .. } = &mut h.kind
                {
                    s.damage = shown.damage;
                }
            }
            return;
        }
        if with_frame {
            self.spawn_item(pos, single(item::ITEM_FRAME), DROP_PICKUP_DELAY);
        }
        if let Some(mut shown) = shown
            && self.entity_rand.next_float() < chance
        {
            self.remove_frame_icon(&mut shown, id);
            self.spawn_item(pos, shown, DROP_PICKUP_DELAY);
        }
    }

    /// `setDisplayedItem`: one of `stack` (or nothing) shows in the frame, and
    /// comparators next to its block space re-check.
    fn set_frame_item(&mut self, id: i32, stack: Option<ItemStack>) {
        let Some(h) = self.hanging.get_mut(&id) else {
            return;
        };
        let Kind::ItemFrame { item, .. } = &mut h.kind else {
            return;
        };
        *item = stack.map(|mut s| {
            s.count = 1;
            s
        });
        // setObjectWatched: sent even when nothing changed.
        h.metadata_updated(FRAME_ITEM_INDEX);
        let tile = h.tile;
        self.update_comparators(tile);
    }

    /// `EntityPlayer.interactWith` on a hanging entity: an empty item frame takes one of
    /// the held item (creative players keep theirs), a full one turns its item by an
    /// eighth. Paintings do nothing.
    pub(super) fn interact_hanging(&mut self, i: usize, id: i32) {
        if self.players[i].gamemode == SPECTATOR {
            return;
        }
        let Some(h) = self.hanging.get_mut(&id) else {
            return;
        };
        if h.kind == Kind::LeashKnot {
            self.interact_knot(i, id);
            return;
        }
        let Kind::ItemFrame { item, rotation, .. } = &mut h.kind else {
            return;
        };
        if item.is_some() {
            let turned = (i32::from(*rotation) + 1) % i32::from(FRAME_ROTATIONS);
            *rotation = turned as i8;
            h.metadata_updated(FRAME_ROTATION_INDEX);
            let tile = h.tile;
            self.update_comparators(tile);
            return;
        }
        let Some(held) = self.players[i].held().cloned() else {
            return;
        };
        self.set_frame_item(id, Some(held));
        let p = &mut self.players[i];
        if p.gamemode != CREATIVE {
            let slot = 36 + p.selected;
            if let Some(s) = p.inventory[slot].as_mut() {
                s.count -= 1;
                if s.count <= 0 {
                    p.inventory[slot] = None;
                }
            }
        }
    }

    /// `BlockRedstoneComparator.findItemFrame`: the signal of the one item frame in the
    /// block space `pos` that faces `facing` (a block face), if there is exactly one.
    pub(super) fn item_frame_signal(&self, pos: (i32, i32, i32), facing: u8) -> Option<i32> {
        let facing = face_to_horizontal(facing)?;
        let space = Aabb::new(
            [f64::from(pos.0), f64::from(pos.1), f64::from(pos.2)],
            [
                f64::from(pos.0 + 1),
                f64::from(pos.1 + 1),
                f64::from(pos.2 + 1),
            ],
        );
        let mut frames = self.hanging.values().filter(|h| {
            matches!(h.kind, Kind::ItemFrame { .. })
                && h.facing == facing
                && h.bb.intersects(&space)
        });
        match (frames.next(), frames.next()) {
            (Some(h), None) => h.frame_signal(),
            _ => None,
        }
    }

    /// `EntityHanging.setPosition`: a hanging entity moved (by a teleport) to another
    /// block space hangs there, and the tracker sends the move.
    pub(super) fn move_hanging(&mut self, id: i32, pos: (f64, f64, f64)) {
        let Some(h) = self.hanging.get_mut(&id) else {
            return;
        };
        let tile = (floor_f64(pos.0), floor_f64(pos.1), floor_f64(pos.2));
        if tile != h.tile {
            h.tile = tile;
            h.update_box();
            h.airborne = true;
        } else {
            h.body.pos = pos;
        }
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

    fn frame() -> Kind {
        Kind::ItemFrame {
            item: None,
            rotation: 0,
            drop_chance: 1.0,
        }
    }

    #[test]
    fn frames_sit_against_the_wall() {
        // Facing north: the wall is to the south, at z = 11.
        let h = Hanging::new(frame(), (5, 64, 10), 2);
        assert_eq!(h.body.pos, (5.5, 64.5, 10.96875));
        assert_eq!(h.bb.min, [5.5 - 0.375, 64.5 - 0.375, 10.96875 - 0.03125]);
        assert_eq!(h.bb.max, [5.5 + 0.375, 64.5 + 0.375, 10.96875 + 0.03125]);
        assert_eq!(h.base.yaw, 180.0);
    }

    #[test]
    fn wide_paintings_shift_counter_clockwise() {
        // Pool (32x16) facing south: counter-clockwise is east.
        let h = Hanging::new(Kind::Painting { art: 7 }, (0, 70, 0), 0);
        assert_eq!(h.body.pos, (1.0, 70.5, 0.03125));
        // Two blocks wide, one high, 1/16 thick.
        assert_eq!(h.bb.min, [0.0, 70.0, 0.0]);
        assert_eq!(h.bb.max, [2.0, 71.0, 0.0625]);
    }

    #[test]
    fn saves_round_trip() {
        let mut h = Hanging::new(frame(), (1, 2, 3), 3);
        h.kind = Kind::ItemFrame {
            item: Some(single(item::PAINTING)),
            rotation: 5,
            drop_chance: 0.5,
        };
        let mut c = Compound::new();
        h.write(&mut c);
        assert_eq!(c.get_i8("Facing"), 3);
        assert_eq!(c.get_i8("ItemRotation"), 5);
        let loaded = Hanging::load(ITEM_FRAME_ID, h.body.clone(), EntityBase::default(), &c);
        assert_eq!(loaded.kind, h.kind);
        assert_eq!(loaded.tile, (1, 2, 3));
        assert_eq!(loaded.body.pos, h.body.pos);
        // Old saves: the wall block and a direction, rotations in quarters.
        let mut old = Compound::new();
        old.insert("TileX", Tag::Int(1));
        old.insert("TileY", Tag::Int(2));
        old.insert("TileZ", Tag::Int(3));
        old.insert("Direction", Tag::Byte(-2));
        old.insert("Item", c.get("Item").cloned().unwrap_or(Tag::Byte(0)));
        old.insert("ItemRotation", Tag::Byte(3));
        let loaded = Hanging::load(ITEM_FRAME_ID, h.body.clone(), EntityBase::default(), &old);
        assert_eq!(loaded.facing, 2);
        assert_eq!(loaded.tile, (1, 2, 2));
        assert!(matches!(loaded.kind, Kind::ItemFrame { rotation: 6, .. }));
        // Unknown paintings show the first art.
        let mut p = Compound::new();
        p.insert("Motive", Tag::String("Mona".into()));
        let loaded = Hanging::load(PAINTING_ID, h.body, EntityBase::default(), &p);
        assert_eq!(loaded.kind, Kind::Painting { art: 0 });
    }
}
