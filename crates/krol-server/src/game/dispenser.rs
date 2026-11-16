//! Dispensers and droppers (vanilla: `BlockDispenser`, `BlockDropper`,
//! `TileEntityDispenser`, `ContainerDispenser`, `BehaviorDefaultDispenseItem`).
//!
//! TODO(M5): the special dispense behaviors (arrows, eggs, snowballs, potions,
//! experience bottles, spawn eggs, fireworks, fire charges, boats, minecarts, buckets,
//! flint and steel, bone meal, TNT, armor onto entities). Items with one stay in the
//! dispenser until then.

use krol_nbt::{Compound, List, Tag, TagType};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack};

use super::Game;
use super::container::{Layout, OpenContainer};
use super::ids::block;
use super::placement::{DOWN, NORTH, SOUTH, UP, WEST, offset, opposite};

/// Dispenser and dropper slots.
pub const DISPENSER_SLOTS: usize = 9;
/// Ticks between the rising edge and the dispense (`tickRate`).
const DISPENSE_DELAY: i32 = 4;
/// The triggered bit (set without updates while powered).
const TRIGGERED: u16 = 8;
/// Items leave this far in front of the block's center.
const DISPENSE_DISTANCE: f64 = 0.7;
/// Height adjustment of the dropped item: up/down and sideways.
const DROP_OFFSET_VERTICAL: f64 = 0.125;
const DROP_OFFSET_SIDE: f64 = 0.15625;
/// Item speed: base, random part, upward, spread (`doDispense(..., 6, ...)`).
const SPEED_BASE: f64 = 0.2;
const SPEED_RANDOM: f64 = 0.1;
const SPEED_UP: f32 = 0.2;
const SPREAD: f32 = 0.0075;
const SPREAD_FACTOR: f64 = 6.0;
/// Effects: dispense click, failure click, smoke (data: the direction).
const EFFECT_DISPENSE: i32 = 1000;
const EFFECT_FAIL: i32 = 1001;
const EFFECT_SMOKE: i32 = 2000;

/// Bone meal is dye with this damage.
const BONE_MEAL_DAMAGE: i16 = 15;

/// Items with their own dispense behavior in vanilla (not modeled yet): projectiles,
/// potions, spawn eggs, fireworks, fire charges, boats, minecarts, buckets, flint and
/// steel, TNT, armor and bone meal.
fn has_special_behavior(stack: &ItemStack) -> bool {
    use super::ids::item;
    const SPECIAL: [i16; 19] = [
        262,
        344,
        332,
        384,
        373,
        383,
        401,
        385,
        333,
        item::LAVA_BUCKET,
        item::WATER_BUCKET,
        item::BUCKET,
        259,
        46,
        328,
        342,
        343,
        407,
        408,
    ];
    SPECIAL.contains(&stack.id)
        || stack.id == 422
        || (298..=317).contains(&stack.id)
        || (stack.id == 351 && stack.damage == BONE_MEAL_DAMAGE)
}

/// Dispensers and droppers.
pub fn is_dispenser(id: u16) -> bool {
    id == block::DISPENSER || id == block::DROPPER
}

/// `(front offset x, y, z)` of a facing.
fn front(face: u8) -> (i32, i32, i32) {
    match face {
        DOWN => (0, -1, 0),
        UP => (0, 1, 0),
        NORTH => (0, 0, -1),
        SOUTH => (0, 0, 1),
        WEST => (-1, 0, 0),
        _ => (1, 0, 0),
    }
}

/// The slots of a tile entity's `Items` list.
pub fn te_items(te: Option<&Compound>, slots: usize) -> Vec<Option<ItemStack>> {
    let mut items = vec![None; slots];
    if let Some(list) = te.and_then(|c| c.get_list("Items", TagType::Compound)) {
        for tag in list.iter() {
            let Tag::Compound(e) = tag else { continue };
            let slot = usize::from(e.get_i8("Slot") as u8);
            if slot < slots {
                items[slot] = super::player::item_from_nbt(e);
            }
        }
    }
    items
}

/// An `Items` list for the given slots.
pub fn items_tag(items: &[Option<ItemStack>]) -> List {
    let mut list = List::new();
    for (slot, item) in items.iter().enumerate() {
        let Some(nbt) = item.as_ref().and_then(super::player::item_to_nbt) else {
            continue;
        };
        let mut entry = Compound::new();
        entry.insert("Slot", Tag::Byte(slot as i8));
        for (k, v) in nbt.iter() {
            entry.insert(k.clone(), v.clone());
        }
        let _ = list.push(Tag::Compound(entry));
    }
    list
}

impl Game {
    /// A dispenser's or dropper's nine slots.
    pub(super) fn dispenser_items(&self, pos: (i32, i32, i32)) -> Vec<Option<ItemStack>> {
        te_items(self.level.overworld.tile_entity(pos), DISPENSER_SLOTS)
    }

    /// Writes the slots back; a change updates comparators (`markDirty`).
    pub(super) fn set_dispenser_items(
        &mut self,
        pos: (i32, i32, i32),
        items: &[Option<ItemStack>],
    ) {
        if self.dispenser_items(pos) == items {
            return;
        }
        if let Some(c) = self.level.overworld.tile_entity_mut(pos) {
            c.insert("Items", Tag::List(items_tag(items)));
        }
        self.update_comparators(pos);
    }

    /// `BlockDispenser.onNeighborBlockChange`: a rising edge (power at the block or
    /// above it) schedules a dispense.
    pub(super) fn dispenser_changed(&mut self, pos: (i32, i32, i32), state: u16) {
        let id = state >> 4;
        let powered =
            self.is_block_powered(pos) || self.is_block_powered((pos.0, pos.1 + 1, pos.2));
        let triggered = state & TRIGGERED != 0;
        if powered && !triggered {
            self.schedule_update(pos, id, DISPENSE_DELAY);
            self.set_block_flags(pos, state | TRIGGERED, 0);
        } else if !powered && triggered {
            self.set_block_flags(pos, state & !TRIGGERED, 0);
        }
    }

    /// `TileEntityDispenser.getDispenseSlot`: a random non-empty slot.
    fn dispense_slot(&mut self, items: &[Option<ItemStack>]) -> Option<usize> {
        let mut chosen = None;
        let mut seen = 1;
        for (k, item) in items.iter().enumerate() {
            if item.is_some() {
                let pick = self.world_rand.next_int_bounded(seen) == 0;
                seen += 1;
                if pick {
                    chosen = Some(k);
                }
            }
        }
        chosen
    }

    /// `BehaviorDefaultDispenseItem.dispense`: one item flies out of the front, with
    /// the click and smoke effects.
    fn dispense_default(&mut self, pos: (i32, i32, i32), face: u8, stack: &ItemStack) {
        let (fx, fy, fz) = front(face);
        let mut at = (
            f64::from(pos.0) + 0.5 + DISPENSE_DISTANCE * f64::from(fx),
            f64::from(pos.1) + 0.5 + DISPENSE_DISTANCE * f64::from(fy),
            f64::from(pos.2) + 0.5 + DISPENSE_DISTANCE * f64::from(fz),
        );
        at.1 -= if face == UP || face == DOWN {
            DROP_OFFSET_VERTICAL
        } else {
            DROP_OFFSET_SIDE
        };
        let r = &mut self.world_rand;
        let speed = r.next_double() * SPEED_RANDOM + SPEED_BASE;
        let spread = f64::from(SPREAD) * SPREAD_FACTOR;
        let motion = (
            f64::from(fx) * speed + r.next_gaussian() * spread,
            f64::from(SPEED_UP) + r.next_gaussian() * spread,
            f64::from(fz) * speed + r.next_gaussian() * spread,
        );
        let one = ItemStack {
            count: 1,
            ..stack.clone()
        };
        self.spawn_item_moving(at, one, 0, Some(motion));
        self.block_effect(EFFECT_DISPENSE, pos);
        let smoke = fx + 1 + (fz + 1) * 3;
        self.block_effect_data(EFFECT_SMOKE, pos, smoke);
    }

    /// `BlockDispenser.dispense` / `BlockDropper.dispense` (the scheduled tick).
    pub(super) fn dispense(&mut self, pos: (i32, i32, i32), state: u16) {
        if self.level.overworld.tile_entity(pos).is_none() {
            return;
        }
        let mut items = self.dispenser_items(pos);
        let Some(slot) = self.dispense_slot(&items) else {
            self.block_effect(EFFECT_FAIL, pos);
            return;
        };
        let Some(stack) = items[slot].clone() else {
            return;
        };
        let face = super::pistons::facing(state & 7);
        if state >> 4 == block::DROPPER {
            // Into the inventory in front, else dropped.
            let one = ItemStack {
                count: 1,
                ..stack.clone()
            };
            let remaining =
                match self.insert_into_inventory_at(offset(pos, face), one, opposite(face)) {
                    Some(true) => (stack.count > 1).then(|| ItemStack {
                        count: stack.count - 1,
                        ..stack.clone()
                    }),
                    Some(false) => Some(stack.clone()),
                    None => {
                        self.dispense_default(pos, face, &stack);
                        (stack.count > 1).then(|| ItemStack {
                            count: stack.count - 1,
                            ..stack.clone()
                        })
                    }
                };
            items[slot] = remaining;
        } else {
            if has_special_behavior(&stack) {
                return;
            }
            self.dispense_default(pos, face, &stack);
            items[slot] = (stack.count > 1).then(|| ItemStack {
                count: stack.count - 1,
                ..stack.clone()
            });
        }
        self.set_dispenser_items(pos, &items);
    }

    /// `BlockDispenser.onBlockActivated` (`displayGUIChest`).
    pub(super) fn activate_dispenser(&mut self, i: usize, pos: (i32, i32, i32), id: u16) {
        let Some(c) = self.level.overworld.tile_entity(pos) else {
            return;
        };
        let custom = c
            .contains_key_of("CustomName", Some(TagType::String))
            .then(|| c.get_str("CustomName").to_owned());
        if self.players[i].window.open.is_some() {
            self.close_screen(i);
        }
        let (kind, key, stat) = if id == block::DROPPER {
            (
                "minecraft:dropper",
                "container.dropper",
                "stat.dropperInspected",
            )
        } else {
            (
                "minecraft:dispenser",
                "container.dispenser",
                "stat.dispenserInspected",
            )
        };
        let title = match custom {
            Some(name) => crate::chat::Component::text(name),
            None => crate::chat::Component::translate(key, vec![]),
        }
        .to_json_string();
        let window = self.players[i].window.next_window_id();
        self.players[i].send(ClientboundPlay::OpenWindow {
            window: window as u8,
            kind: kind.into(),
            title,
            slots: DISPENSER_SLOTS as u8,
        });
        let layout = Layout::Dispenser;
        self.players[i].window.open = Some(OpenContainer {
            id: window,
            layout,
            pos,
            slots: Vec::new(),
            parts: vec![pos],
            ender: false,
            sent: vec![None; layout.len()],
            fields: [0; 4],
            taken: 0,
        });
        self.open_container_gui(i);
        self.add_stat(i, stat, 1);
    }

    /// The contents dropped when the block breaks.
    pub(super) fn dispenser_contents(&self, pos: (i32, i32, i32)) -> Vec<ItemStack> {
        self.dispenser_items(pos).into_iter().flatten().collect()
    }
}
