//! Dropped items (vanilla: `EntityItem`, `Block.dropBlockAsItemWithChance`,
//! `Block.spawnAsEntity`, `InventoryPlayer.addItemStackToInventory`).
//!
//! TODO(M5): items in lava, being pushed out of solid blocks, despawn and
//! merge rules for items with NBT, saving items with their chunk, thrown items.

use krol_protocol::packets::play::{ClientboundPlay, ItemStack, Metadata, MetadataValue};

use super::Game;
use super::physics::{self, Aabb, Body};

/// A dropped item stack in the world.
#[derive(Debug, Clone)]
pub struct ItemEntity {
    /// Physics state (0.25 x 0.25 box).
    pub body: Body,
    /// Yaw (random on creation, only shown in the spawn packet).
    pub yaw: f32,
    /// The stack.
    pub stack: ItemStack,
    /// Ticks lived (`age`; despawns at 6000).
    pub age: i32,
    /// Ticks until it can be picked up (`delayBeforeCanPickup`).
    pub pickup_delay: i32,
    /// Ticks existed (`ticksExisted`).
    pub ticks: i32,
    /// Removed from the world (`isDead`).
    pub dead: bool,
    /// The chunk the entity was last added to.
    pub chunk: (i32, i32),
    /// Metadata indices changed since the tracker last sent them (all of them on
    /// creation).
    pub dirty_metadata: std::collections::BTreeSet<u8>,
}

impl ItemEntity {
    /// Vanilla's metadata for an item entity.
    pub fn metadata(&self) -> Vec<Metadata> {
        use MetadataValue::{Byte, Item, Short, String};
        let md = |index, value| Metadata { index, value };
        vec![
            md(0, Byte(0)),
            md(1, Short(300)),
            md(2, String(std::string::String::new())),
            md(3, Byte(0)),
            md(4, Byte(0)),
            md(10, Item(Some(self.stack.clone()))),
        ]
    }
}

/// Maximum stack size of an item (`Item.getItemStackLimit`).
pub fn max_stack(id: i16) -> i8 {
    super::item_data::max_stack(id)
}

/// Whether items with this id distinguish damage values (`getHasSubtypes`); tools and
/// armor use damage as durability instead.
fn has_subtypes(id: i16) -> bool {
    super::item_data::has_subtypes(id)
}

/// What a block drops when harvested without enchantments (`quantityDropped`,
/// `getItemDropped`, `damageDropped`), using the world random like vanilla.
/// TODO(M5): crops, beds and the other special drops.
fn block_drops(rand: &mut krol_java::JavaRandom, state: u16) -> Vec<ItemStack> {
    let (b, m) = (state >> 4, (state & 15) as i16);
    let stack = |id: i16, damage: i16| ItemStack {
        id,
        count: 1,
        damage,
        nbt: None,
    };
    // (quantity, item, damage); a quantity of 0 drops nothing.
    let (count, item, damage): (i32, Option<i16>, i16) = match b {
        1 => (1, Some(if m == 0 { 4 } else { 1 }), m),
        2 | 60 | 110 => (1, Some(3), 0),
        3 => (1, Some(3), if m == 2 { 0 } else { m }),
        5 | 12 | 19 | 24 | 35 | 38 | 95 | 98 | 159 | 168 | 171 | 179 => (1, Some(b as i16), m),
        6 => (1, Some(6), m & 7),
        17 | 162 => (1, Some(b as i16), m & 3),
        44 | 126 | 182 => (1, Some(b as i16), m & 7),
        43 => (2, Some(44), m & 7),
        125 => (2, Some(126), m & 7),
        181 => (2, Some(182), m & 7),
        155 => (
            1,
            Some(155),
            if m == 1 {
                1
            } else if m >= 2 {
                2
            } else {
                0
            },
        ),
        13 => (1, None, 0), // decided per drop below
        16 => (1, Some(263), 0),
        56 => (1, Some(264), 0),
        129 => (1, Some(388), 0),
        153 => (1, Some(406), 0),
        21 => (4 + rand.next_int_bounded(5), Some(351), 4),
        73 | 74 => (4 + rand.next_int_bounded(2), Some(331), 0),
        89 => ((2 + rand.next_int_bounded(3)).clamp(1, 4), Some(348), 0),
        47 => (3, Some(340), 0),
        82 => (4, Some(337), 0),
        80 => (4, Some(332), 0),
        78 => (i32::from(m & 7) + 1, Some(332), 0),
        103 => ((3 + rand.next_int_bounded(5)).min(9), Some(360), 0),
        62 => (1, Some(61), 0),
        // Redstone: wire drops dust, unlit torches and lit lamps their normal item.
        55 => (1, Some(331), 0),
        75 => (1, Some(76), 0),
        124 => (1, Some(123), 0),
        93 | 94 => (1, Some(356), 0),
        149 | 150 => (1, Some(404), 0),
        // Doors drop from their lower half only.
        64 | 71 | 193..=197 if m & 8 != 0 => (0, None, 0),
        64 => (1, Some(324), 0),
        71 => (1, Some(330), 0),
        193..=197 => (1, Some(427 + b as i16 - 193), 0),
        super::ids::block::STANDING_SIGN | super::ids::block::WALL_SIGN => {
            (1, Some(super::ids::item::SIGN), 0)
        }
        20 | 79 | 102 | 160 | 52 | 97 | 18 | 161 => (0, None, 0),
        31 | super::plants::DOUBLE_PLANT => (1, None, 0), // decided per drop below
        super::ids::block::DEADBUSH => (0, None, 0),
        _ => {
            if krol_world::items::name_for_id(b as i16).is_some() {
                (1, Some(b as i16), 0)
            } else {
                (0, None, 0)
            }
        }
    };
    let mut out = Vec::new();
    for _ in 0..count {
        // dropBlockAsItemWithChance: chance 1.0 still draws a float.
        if rand.next_float() > 1.0 {
            continue;
        }
        let item = match b {
            13 => Some(if rand.next_int_bounded(10) == 0 {
                318
            } else {
                13
            }),
            31 => (rand.next_int_bounded(8) == 0).then_some(295),
            super::plants::DOUBLE_PLANT => {
                if let Some((id, damage)) = super::plants::double_plant_drop(rand, m as u16) {
                    out.push(stack(id, damage));
                }
                None
            }
            _ => item,
        };
        if let Some(id) = item {
            out.push(stack(id, damage));
        }
    }
    out
}

impl Game {
    /// Allocates an entity id (vanilla `Entity.nextEntityID`).
    pub(super) fn next_entity_id(&mut self) -> i32 {
        let id = self.next_entity_id;
        self.next_entity_id += 1;
        id
    }

    /// Drops what the block leaves when harvested (`dropBlockAsItem`), unless the
    /// `doTileDrops` game rule is off.
    pub(super) fn drop_block_items(&mut self, pos: (i32, i32, i32), state: u16) {
        if super::leaves::is_leaves(state >> 4) {
            self.drop_leaves(pos, state);
            return;
        }
        for stack in block_drops(&mut self.world_rand, state) {
            self.spawn_as_entity(pos, stack);
        }
    }

    /// `Block.spawnAsEntity`: one stack at a random spot in the block, unless the
    /// `doTileDrops` game rule is off.
    pub(super) fn spawn_as_entity(&mut self, pos: (i32, i32, i32), stack: ItemStack) {
        {
            if self.level.data.game_rule("doTileDrops") == Some("false") {
                return;
            }
            let r = &mut self.world_rand;
            let f = 0.5f32;
            let mut offset = || f64::from(r.next_float() * f) + f64::from(1.0 - f) * 0.5;
            let at = (
                f64::from(pos.0) + offset(),
                f64::from(pos.1) + offset(),
                f64::from(pos.2) + offset(),
            );
            self.spawn_item(at, stack, 10);
        }
    }

    /// Creates an item entity (`new EntityItem`) and adds it to the world.
    pub(super) fn spawn_item(&mut self, pos: (f64, f64, f64), stack: ItemStack, delay: i32) {
        self.spawn_item_moving(pos, stack, delay, None);
    }

    /// Like [`Self::spawn_item`], replacing the random start motion.
    pub(super) fn spawn_item_moving(
        &mut self,
        pos: (f64, f64, f64),
        stack: ItemStack,
        delay: i32,
        motion: Option<(f64, f64, f64)>,
    ) {
        let entity_id = self.next_entity_id();
        // Vanilla uses Math.random() here.
        let r = &mut self.world_rand;
        let yaw = (r.next_double() * 360.0) as f32;
        let mx = f64::from((r.next_double() * f64::from(0.2f32) - f64::from(0.1f32)) as f32);
        let mz = f64::from((r.next_double() * f64::from(0.2f32) - f64::from(0.1f32)) as f32);
        let mut body = Body::player(pos, false);
        body.width = 0.25;
        body.height = 0.25;
        body.step_height = 0.0;
        body.motion = motion.unwrap_or((mx, f64::from(0.2f32), mz));
        let item = ItemEntity {
            body,
            yaw,
            stack,
            age: 0,
            pickup_delay: delay,
            ticks: 0,
            dead: false,
            chunk: (
                krol_java::math::floor_f64(pos.0) >> 4,
                krol_java::math::floor_f64(pos.2) >> 4,
            ),
            dirty_metadata: [0, 1, 2, 3, 4, 10].into_iter().collect(),
        };
        self.items.insert(entity_id, item);
        self.track_entity(entity_id);
    }

    /// One item's turn in the entity loop: tick it unless dead, then remove it if dead.
    pub(super) fn update_item(&mut self, id: i32) {
        if self.items.get(&id).is_some_and(|it| !it.dead) {
            self.tick_item(id);
        }
        if self.items.get(&id).is_some_and(|it| it.dead) {
            self.items.remove(&id);
            self.untrack_entity(id);
        }
    }

    fn tick_item(&mut self, id: i32) {
        let Some(mut it) = self.items.remove(&id) else {
            return;
        };
        it.ticks += 1;
        // Entity.handleWaterMovement.
        let bb = it
            .body
            .bb()
            .expand([0.0, f64::from(-0.4f32), 0.0])
            .contract([0.001, 0.001, 0.001]);
        let (water, push) = super::liquids::water_movement(&mut self.level.overworld, &bb, true);
        it.body.motion.0 += push.0;
        it.body.motion.1 += push.1;
        it.body.motion.2 += push.2;
        if water && !it.body.in_water && it.ticks > 1 {
            // resetHeight: the splash (particles are client-side).
            let m = it.body.motion;
            let volume = (krol_java::math::sqrt_f64(
                m.0 * m.0 * f64::from(0.2f32) + m.1 * m.1 + m.2 * m.2 * f64::from(0.2f32),
            ) * 0.2)
                .min(1.0);
            let r = &mut self.world_rand;
            let pitch = 1.0 + (r.next_float() - r.next_float()) * 0.4;
            let pos = it.body.pos;
            self.play_sound_near_except(None, "game.neutral.swim.splash", pos, volume, pitch);
        }
        it.body.in_water = water;
        if it.pickup_delay > 0 && it.pickup_delay != 32767 {
            it.pickup_delay -= 1;
        }
        let prev = it.body.pos;
        it.body.motion.1 -= f64::from(0.04f32);
        // TODO(M5): pushOutOfBlocks when stuck inside a solid block.
        let m = it.body.motion;
        physics::move_entity_quiet(&mut self.level.overworld, &mut it.body, m.0, m.1, m.2);
        // doBlockCollisions, with the item in the world.
        let bb = it.body.bb();
        self.items.insert(id, it);
        self.block_collisions(&bb);
        let Some(mut it) = self.items.remove(&id) else {
            return;
        };
        let p = it.body.pos;
        let crossed = prev.0 as i32 != p.0 as i32
            || prev.1 as i32 != p.1 as i32
            || prev.2 as i32 != p.2 as i32;
        let merge = crossed || it.ticks % 25 == 0;
        let mut friction = 0.98f32;
        if it.body.on_ground {
            let below = self.level.overworld.block(
                krol_java::math::floor_f64(p.0),
                krol_java::math::floor_f64(p.1) - 1,
                krol_java::math::floor_f64(p.2),
            ) >> 4;
            friction = super::blocks::slipperiness(below) * 0.98;
        }
        it.body.motion.0 *= f64::from(friction);
        it.body.motion.1 *= f64::from(0.98f32);
        it.body.motion.2 *= f64::from(friction);
        if it.body.on_ground {
            it.body.motion.1 *= -0.5;
        }
        if it.age != -32768 {
            it.age += 1;
        }
        if it.age >= 6000 {
            it.dead = true;
        }
        it.chunk = (
            krol_java::math::floor_f64(p.0) >> 4,
            krol_java::math::floor_f64(p.2) >> 4,
        );
        self.items.insert(id, it);
        if merge {
            self.merge_items(id);
        }
    }

    /// `searchForOtherItemsNearby` + `combineItems`.
    fn merge_items(&mut self, id: i32) {
        let Some(area) = self
            .items
            .get(&id)
            .map(|it| it.body.bb().expand([0.5, 0.0, 0.5]))
        else {
            return;
        };
        let others: Vec<i32> = self
            .items
            .iter()
            .filter(|(o, it)| **o != id && !it.dead && it.body.bb().intersects(&area))
            .map(|(o, _)| *o)
            .collect();
        for other in others {
            self.combine(id, other);
        }
    }

    fn combine(&mut self, a: i32, b: i32) -> bool {
        let (Some(this), Some(that)) = (self.items.get(&a), self.items.get(&b)) else {
            return false;
        };
        if this.dead || that.dead || this.pickup_delay == 32767 || that.pickup_delay == 32767 {
            return false;
        }
        if this.age == -32768 || that.age == -32768 {
            return false;
        }
        let (s, o) = (&this.stack, &that.stack);
        if o.id != s.id || o.nbt.is_some() != s.nbt.is_some() || (o.nbt.is_some() && o.nbt != s.nbt)
        {
            return false;
        }
        if has_subtypes(o.id) && o.damage != s.damage {
            return false;
        }
        if o.count < s.count {
            return self.combine(b, a);
        }
        if i32::from(o.count) + i32::from(s.count) > i32::from(max_stack(o.id)) {
            return false;
        }
        let (count, delay, age) = (s.count, this.pickup_delay, this.age);
        if let Some(that) = self.items.get_mut(&b) {
            that.stack.count += count;
            that.pickup_delay = that.pickup_delay.max(delay);
            that.age = that.age.min(age);
            that.dirty_metadata.insert(10);
        }
        if let Some(this) = self.items.get_mut(&a) {
            this.dead = true;
        }
        true
    }

    /// Vanilla `EntityPlayer.dropOneItem` (Q, or Ctrl+Q for the whole stack) and
    /// `dropItem` with the thrower set.
    pub(super) fn drop_held(&mut self, i: usize, all: bool) {
        let p = &mut self.players[i];
        let slot = 36 + p.selected;
        let Some(held) = p.inventory[slot].as_mut() else {
            return;
        };
        let amount = if all { held.count } else { 1 };
        let stack = if held.count <= amount {
            p.inventory[slot].take()
        } else {
            held.count -= amount;
            Some(ItemStack {
                count: amount,
                ..held.clone()
            })
        };
        let Some(stack) = stack.filter(|s| s.count != 0) else {
            return;
        };
        self.toss(i, stack, true);
    }

    /// `ItemStack.damageItem(amount, player)` on the held item.
    pub(super) fn damage_held_item(&mut self, i: usize, amount: i32) {
        let slot = 36 + self.players[i].selected;
        self.damage_slot_item(i, slot, amount);
    }

    /// `ItemStack.damageItem(amount, player)` on the item in a player window slot: wear
    /// (unless creative, unbreakable or negated by unbreaking), breaking it past its
    /// maximum damage (the stack is left with count 0).
    pub(super) fn damage_slot_item(&mut self, i: usize, slot: usize, amount: i32) {
        let p = &mut self.players[i];
        if p.gamemode == 1 {
            return;
        }
        let Some(stack) = p.inventory[slot].clone() else {
            return;
        };
        let max = i32::from(super::item_data::max_damage(stack.id));
        let unbreakable = stack
            .nbt
            .as_ref()
            .is_some_and(|n| n.get_bool("Unbreakable"));
        if max <= 0 || unbreakable {
            return;
        }
        let mut amount = amount;
        if amount > 0 {
            let level =
                super::item_data::enchantment_level(Some(&stack), super::item_data::UNBREAKING);
            let armor = (ARMOR_FIRST..=ARMOR_LAST).contains(&stack.id);
            let mut negated = 0;
            let mut k = 0;
            while level > 0 && k < amount {
                // EnchantmentDurability.negateDamage.
                let kept = armor && p.rand.next_float() < ARMOR_UNBREAKING_IGNORED;
                if !kept && p.rand.next_int_bounded(level + 1) > 0 {
                    negated += 1;
                }
                k += 1;
            }
            amount -= negated;
            if amount <= 0 {
                return;
            }
        }
        let mut stack = stack;
        let damage = i32::from(stack.damage) + amount;
        if damage > max {
            // renderBrokenItemStack: the sound for the others; the particles draw from
            // the player's random.
            let pitch = BREAK_PITCH_BASE + self.world_rand.next_float() * BREAK_PITCH_SPREAD;
            let p = &mut self.players[i];
            for _ in 0..BREAK_PARTICLES * 3 {
                p.rand.next_float();
            }
            let (pos, conn) = (p.pos, p.conn_id);
            self.play_sound_near_except(Some(conn), "random.break", pos, BREAK_VOLUME, pitch);
            let p = &mut self.players[i];
            if let Some(name) = krol_world::items::name_for_id(stack.id) {
                p.stats
                    .add(&format!("stat.breakItem.{}", name.replace(':', ".")), 1);
            }
            stack.count = (stack.count - 1).max(0);
            stack.damage = 0;
        } else {
            stack.damage = damage as i16;
        }
        self.players[i].inventory[slot] = Some(stack);
    }

    /// Vanilla `EntityPlayer.dropItem(stack, false, traced)`: thrown along the look
    /// direction from eye height; `traced` drops count `stat.drop`.
    pub(super) fn toss(&mut self, i: usize, stack: ItemStack, traced: bool) {
        let p = &mut self.players[i];
        let y = p.pos.1 - f64::from(0.3f32) + f64::from(1.62f32);
        let (yaw, pitch) = (
            p.rotation.0 / 180.0 * std::f32::consts::PI,
            p.rotation.1 / 180.0 * std::f32::consts::PI,
        );
        use krol_java::math::{cos, sin};
        let f = 0.3f32;
        let mut motion = (
            f64::from(-sin(yaw) * cos(pitch) * f),
            f64::from(-sin(pitch) * f + 0.1),
            f64::from(cos(yaw) * cos(pitch) * f),
        );
        let r = &mut p.rand;
        let angle = r.next_float() * std::f32::consts::PI * 2.0;
        let speed = 0.02 * r.next_float();
        motion.0 += f64::from(angle).cos() * f64::from(speed);
        motion.1 += f64::from((r.next_float() - r.next_float()) * 0.1);
        motion.2 += f64::from(angle).sin() * f64::from(speed);
        let pos = (p.pos.0, y, p.pos.2);
        if traced {
            p.stats.add("stat.drop", 1);
        }
        // TODO(M5): remember the thrower (diamondsToYou achievement).
        self.spawn_item_moving(pos, stack, 40, Some(motion));
    }

    /// Items near the player get picked up (`EntityPlayer.onLivingUpdate` collisions,
    /// `EntityItem.onCollideWithPlayer`).
    pub(super) fn pick_up_items(&mut self, i: usize) {
        let p = &self.players[i];
        if p.health() <= 0.0 || p.gamemode == 3 {
            return;
        }
        let area = Aabb::at(p.pos, 0.6, 1.8).expand([1.0, 0.5, 1.0]);
        let mut near: Vec<i32> = self
            .items
            .iter()
            .filter(|(_, it)| !it.dead && it.body.bb().intersects(&area))
            .map(|(id, _)| *id)
            .chain(
                self.orbs
                    .iter()
                    .filter(|(_, o)| !o.dead && o.body.bb().intersects(&area))
                    .map(|(id, _)| *id),
            )
            .collect();
        // TODO(M5): vanilla goes through the chunk's entity lists; entity id order is
        // the order they were added.
        near.sort_unstable();
        for id in near {
            if self.orbs.contains_key(&id) {
                self.collect_orb(i, id);
                continue;
            }
            let Some(it) = self.items.get(&id) else {
                continue;
            };
            if it.pickup_delay != 0 {
                continue;
            }
            let mut stack = it.stack.clone();
            let before = stack.count;
            if !add_to_inventory(&mut self.players[i].inventory, &mut stack) {
                continue;
            }
            // TODO(M5): pickup achievements (mineWood, killCow, diamonds, blazeRod).
            let r = &mut self.world_rand;
            let pitch = ((r.next_float() - r.next_float()) * 0.7 + 1.0) * 2.0;
            let pos = self.players[i].pos;
            self.play_sound_near_except(None, "random.pop", pos, 0.2, pitch);
            let collector = self.players[i].entity_id;
            self.send_to_tracking(
                id,
                &ClientboundPlay::CollectItem {
                    collected: id,
                    collector,
                },
            );
            let _ = before;
            if let Some(it) = self.items.get_mut(&id) {
                if stack.count <= 0 {
                    it.dead = true;
                } else {
                    it.stack = stack;
                    it.dirty_metadata.insert(10);
                }
            }
        }
    }
}

/// Vanilla `InventoryPlayer.addItemStackToInventory` on the player window: damaged items
/// go to the first empty slot; others fill matching stacks first, then empty slots.
/// Main inventory slots are scanned hotbar first. Returns whether anything was added
/// (`stack.count` is reduced by what fit).
pub fn add_to_inventory(window: &mut [Option<ItemStack>], stack: &mut ItemStack) -> bool {
    // mainInventory index i: hotbar 0..9 is window slot 36 + i, the rest is i.
    add_to_inventory_mapped(window, stack, |i| if i < 9 { 36 + i } else { i })
}

/// [`add_to_inventory`] on any window, given where main inventory index `i` is.
pub fn add_to_inventory_mapped(
    window: &mut [Option<ItemStack>],
    stack: &mut ItemStack,
    slot: impl Fn(usize) -> usize,
) -> bool {
    let max = max_stack(stack.id);
    let damaged = max == 1 && stack.damage > 0;
    if stack.count <= 0 {
        return false;
    }
    if damaged {
        if let Some(i) = (0..36).find(|&i| window[slot(i)].is_none()) {
            window[slot(i)] = Some(stack.clone());
            stack.count = 0;
            return true;
        }
        return false;
    }
    let start = stack.count;
    while stack.count > 0 {
        let matching = (0..36).find(|&i| {
            window[slot(i)].as_ref().is_some_and(|s| {
                s.id == stack.id
                    && s.count < max
                    && (!has_subtypes(s.id) || s.damage == stack.damage)
                    && s.nbt == stack.nbt
            })
        });
        let target = matching.or_else(|| (0..36).find(|&i| window[slot(i)].is_none()));
        let Some(i) = target else {
            break;
        };
        let entry = &mut window[slot(i)];
        let existing = entry.get_or_insert_with(|| ItemStack {
            count: 0,
            ..stack.clone()
        });
        let room = max - existing.count;
        let moved = room.min(stack.count);
        if moved <= 0 {
            break;
        }
        existing.count += moved;
        stack.count -= moved;
    }
    stack.count < start
}

/// The armor items (helmets to boots, leather to gold): unbreaking helps them less.
const ARMOR_FIRST: i16 = 298;
const ARMOR_LAST: i16 = 317;
/// `EnchantmentDurability.negateDamage`: armor ignores unbreaking 60% of the time.
const ARMOR_UNBREAKING_IGNORED: f32 = 0.6;
/// `random.break` when an item breaks.
const BREAK_VOLUME: f32 = 0.8;
const BREAK_PITCH_BASE: f32 = 0.8;
const BREAK_PITCH_SPREAD: f32 = 0.4;
/// Item crumbs shown when an item breaks.
const BREAK_PARTICLES: usize = 5;

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i16, count: i8) -> ItemStack {
        ItemStack {
            id,
            count,
            damage: 0,
            nbt: None,
        }
    }

    #[test]
    fn inventory_adds_like_vanilla() {
        let mut window = vec![None; 45];
        window[40] = Some(item(3, 63));
        let mut s = item(3, 3);
        assert!(add_to_inventory(&mut window, &mut s));
        assert_eq!(window[40].as_ref().unwrap().count, 64);
        // The rest goes to the first empty hotbar slot.
        assert_eq!(window[36].as_ref().unwrap().count, 2);
        assert_eq!(s.count, 0);
    }
}
