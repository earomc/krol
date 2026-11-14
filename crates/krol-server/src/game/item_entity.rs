//! Dropped items (vanilla: `EntityItem`, `Block.dropBlockAsItemWithChance`,
//! `Block.spawnAsEntity`, `InventoryPlayer.addItemStackToInventory`).
//!
//! TODO(M5): items in lava and water, being pushed out of solid blocks, despawn and
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
/// TODO(M5): complete per-item table.
pub fn max_stack(id: i16) -> i8 {
    match id {
        256..=259
        | 261
        | 267..=279
        | 283..=286
        | 290..=294
        | 298..=317
        | 325..=328
        | 333
        | 335
        | 342
        | 343
        | 346
        | 354
        | 359
        | 373
        | 387
        | 398
        | 403
        | 407
        | 408
        | 417..=419
        | 422
        | 2256..=2267 => 1,
        323 | 332 | 344 | 368 | 379 | 386 | 425 => 16,
        _ => 64,
    }
}

/// Whether items with this id distinguish damage values (`getHasSubtypes`); tools and
/// armor use damage as durability instead.
fn has_subtypes(id: i16) -> bool {
    max_stack(id) != 1 || matches!(id, 373)
}

/// What a block drops when harvested without enchantments (`quantityDropped`,
/// `getItemDropped`, `damageDropped`), using the world random like vanilla.
/// TODO(M5): leaves, crops, doors, beds, double plants and the other special drops.
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
        20 | 79 | 102 | 160 | 52 | 97 | 18 | 161 => (0, None, 0),
        31 => (1, None, 0), // seeds, decided per drop below
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
        for stack in block_drops(&mut self.world_rand, state) {
            if self.level.data.game_rule("doTileDrops") == Some("false") {
                continue;
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
        body.motion = (mx, f64::from(0.2f32), mz);
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

    /// Ticks every item (`EntityItem.onUpdate`) and removes dead ones.
    pub(super) fn tick_items(&mut self) {
        let ids: Vec<i32> = self.items.keys().copied().collect();
        for id in ids {
            let dead = self.items.get(&id).is_none_or(|it| it.dead);
            if !dead {
                self.tick_item(id);
            }
            if self.items.get(&id).is_some_and(|it| it.dead) {
                self.items.remove(&id);
                self.untrack_entity(id);
            }
        }
    }

    fn tick_item(&mut self, id: i32) {
        let Some(mut it) = self.items.remove(&id) else {
            return;
        };
        it.ticks += 1;
        if it.pickup_delay > 0 && it.pickup_delay != 32767 {
            it.pickup_delay -= 1;
        }
        let prev = it.body.pos;
        it.body.motion.1 -= f64::from(0.04f32);
        // TODO(M5): pushOutOfBlocks when stuck inside a solid block.
        let m = it.body.motion;
        physics::move_entity_quiet(&mut self.level.overworld, &mut it.body, m.0, m.1, m.2);
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
            let slip = match below {
                79 | 174 => 0.98,
                165 => 0.8,
                _ => 0.6,
            };
            friction = slip * 0.98;
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

    /// Items near the player get picked up (`EntityPlayer.onLivingUpdate` collisions,
    /// `EntityItem.onCollideWithPlayer`).
    pub(super) fn pick_up_items(&mut self, i: usize) {
        let p = &self.players[i];
        if p.health() <= 0.0 || p.gamemode == 3 {
            return;
        }
        let area = Aabb::at(p.pos, 0.6, 1.8).expand([1.0, 0.5, 1.0]);
        let near: Vec<i32> = self
            .items
            .iter()
            .filter(|(_, it)| !it.dead && it.body.bb().intersects(&area))
            .map(|(id, _)| *id)
            .collect();
        for id in near {
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
    let slot = |i: usize| if i < 9 { 36 + i } else { i };
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
