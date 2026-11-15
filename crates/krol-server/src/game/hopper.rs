//! Hoppers and the inventories they move items between (vanilla: `TileEntityHopper`,
//! `BlockHopper`, `ContainerHopper`, `ISidedInventory` of furnaces).
//!
//! TODO(M5): minecarts with chests and hoppers, dispensers, droppers and brewing
//! stands as inventories.

use krol_nbt::{Compound, List, Tag, TagType};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack};

use super::Game;
use super::container::{Layout, OpenContainer};
use super::ids::block;
use super::placement::{DOWN, UP, offset, opposite};

/// Hopper slots.
pub const HOPPER_SLOTS: usize = 5;
/// Ticks between two transfers (`setTransferCooldown(8)`).
const TRANSFER_COOLDOWN: i32 = 8;
/// A hopper fed by another one restarts its cooldown only when it is nearly over
/// (`mayTransfer`).
const MAY_TRANSFER: i32 = 1;
/// The hopper's disabled bit (powered).
const DISABLED: u16 = 8;
/// Item entities within this distance of the point above the hopper are taken.
const PICKUP_RADIUS: f64 = 0.5;

/// An inventory a hopper can reach.
#[derive(Debug, Clone)]
enum Inv {
    /// A chest, or both halves of a double chest.
    Chest(Vec<(i32, i32, i32)>),
    Furnace((i32, i32, i32)),
    Hopper((i32, i32, i32)),
    Dispenser((i32, i32, i32)),
    Brewing((i32, i32, i32)),
}

/// The facing of a hopper (`EnumFacing.getFront(meta & 7)`).
fn hopper_facing(meta: u16) -> u8 {
    super::pistons::facing(meta)
}

/// `ItemStack` combination test of `insertStack` (`canCombine`).
fn can_combine(existing: &ItemStack, stack: &ItemStack) -> bool {
    existing.id == stack.id
        && existing.damage == stack.damage
        && existing.count <= super::item_data::max_stack(existing.id)
        && existing.nbt == stack.nbt
}

/// A hopper's saved state.
#[derive(Debug, Clone, Default, PartialEq)]
struct HopperData {
    items: Vec<Option<ItemStack>>,
    cooldown: i32,
}

impl HopperData {
    fn read(c: &Compound) -> Self {
        let mut items = vec![None; HOPPER_SLOTS];
        if let Some(list) = c.get_list("Items", TagType::Compound) {
            for tag in list.iter() {
                let Tag::Compound(e) = tag else { continue };
                let slot = usize::from(e.get_i8("Slot") as u8);
                if slot < HOPPER_SLOTS {
                    items[slot] = super::player::item_from_nbt(e);
                }
            }
        }
        Self {
            items,
            cooldown: c.get_i32("TransferCooldown"),
        }
    }

    fn write(&self, c: &mut Compound) {
        let mut list = List::new();
        for (slot, item) in self.items.iter().enumerate() {
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
        c.insert("Items", Tag::List(list));
        c.insert("TransferCooldown", Tag::Int(self.cooldown));
    }
}

impl Game {
    fn hopper_data(&self, pos: (i32, i32, i32)) -> Option<HopperData> {
        self.level
            .overworld
            .tile_entity(pos)
            .filter(|c| c.get_str("id") == "Hopper")
            .map(HopperData::read)
    }

    /// Saves a hopper; changed items update the comparators reading it (`markDirty`).
    fn store_hopper(&mut self, pos: (i32, i32, i32), data: &HopperData) {
        let old = self.hopper_data(pos);
        if old.as_ref() == Some(data) {
            return;
        }
        if let Some(c) = self.level.overworld.tile_entity_mut(pos) {
            data.write(c);
        }
        if old.is_none_or(|o| o.items != data.items) {
            self.update_comparators(pos);
        }
    }

    /// A hopper's five slots (for its window and comparators).
    pub(super) fn hopper_items(&self, pos: (i32, i32, i32)) -> Vec<Option<ItemStack>> {
        self.hopper_data(pos)
            .map(|d| d.items)
            .unwrap_or_else(|| vec![None; HOPPER_SLOTS])
    }

    /// Writes a hopper window's slots back.
    pub(super) fn set_hopper_items(&mut self, pos: (i32, i32, i32), items: &[Option<ItemStack>]) {
        if let Some(mut data) = self.hopper_data(pos) {
            data.items = items.to_vec();
            self.store_hopper(pos, &data);
        }
    }

    /// `TileEntityHopper.getInventoryAtPosition`: the inventory block at `pos` (a
    /// blocked chest is none).
    fn inventory_at(&mut self, pos: (i32, i32, i32)) -> Option<Inv> {
        let id = self.level.overworld.block(pos.0, pos.1, pos.2) >> 4;
        self.level.overworld.tile_entity(pos)?;
        match id {
            _ if block::is_chest(id) => self.chest_parts(pos).map(Inv::Chest),
            _ if block::is_furnace(id) => Some(Inv::Furnace(pos)),
            block::HOPPER => Some(Inv::Hopper(pos)),
            _ if super::dispenser::is_dispenser(id) => Some(Inv::Dispenser(pos)),
            super::brewing::BREWING_STAND => Some(Inv::Brewing(pos)),
            _ => None,
        }
    }

    fn inv_items(&self, inv: &Inv) -> Vec<Option<ItemStack>> {
        match inv {
            Inv::Chest(parts) => self.chest_contents(parts),
            Inv::Furnace(pos) => self.furnace_items(*pos),
            Inv::Hopper(pos) => self.hopper_items(*pos),
            Inv::Dispenser(pos) => self.dispenser_items(*pos),
            Inv::Brewing(pos) => self.brewing_items(*pos),
        }
    }

    /// Writes an inventory back; `put_input` says a furnace's input slot got a new
    /// stack (`setInventorySlotContents(0, ...)`).
    fn set_inv_items(&mut self, inv: &Inv, items: &[Option<ItemStack>], put_input: bool) {
        match inv {
            Inv::Chest(parts) => self.set_chest_contents(parts, items),
            Inv::Furnace(pos) => self.set_furnace_items(*pos, items, put_input),
            Inv::Hopper(pos) => self.set_hopper_items(*pos, items),
            Inv::Dispenser(pos) => self.set_dispenser_items(*pos, items),
            Inv::Brewing(pos) => self.set_brewing_items(*pos, items),
        }
    }

    /// `getSlotsForFace` for sided inventories (furnaces); all slots otherwise.
    fn slots_for_face(inv: &Inv, len: usize, face: Option<u8>) -> Vec<usize> {
        match (inv, face) {
            (Inv::Furnace(_), Some(DOWN)) => vec![super::furnace::OUTPUT, super::furnace::FUEL],
            (Inv::Furnace(_), Some(UP)) => vec![super::furnace::INPUT],
            (Inv::Furnace(_), Some(_)) => vec![super::furnace::FUEL],
            // TileEntityBrewingStand.getSlotsForFace: the ingredient from above, the
            // bottles from everywhere else.
            (Inv::Brewing(_), Some(UP)) => vec![super::brewing::INGREDIENT],
            (Inv::Brewing(_), Some(_)) => vec![0, 1, 2],
            _ => (0..len).collect(),
        }
    }

    /// `isItemValidForSlot` (and `canInsertItem`, the same for furnaces).
    fn can_insert(inv: &Inv, slot: usize, stack: &ItemStack) -> bool {
        match inv {
            Inv::Furnace(_) => match slot {
                super::furnace::OUTPUT => false,
                super::furnace::FUEL => super::furnace::fits_fuel_slot(stack),
                _ => true,
            },
            // TileEntityBrewingStand.isItemValidForSlot.
            Inv::Brewing(_) => {
                if slot == super::brewing::INGREDIENT {
                    super::brewing::is_ingredient(stack)
                } else {
                    super::brewing::fits_bottle_slot(stack)
                }
            }
            _ => true,
        }
    }

    /// `canExtractItem`: only buckets come out of a furnace's fuel slot from below.
    fn can_extract(inv: &Inv, slot: usize, stack: &ItemStack, face: u8) -> bool {
        use super::ids::item;
        match inv {
            Inv::Furnace(_) if face == DOWN && slot == super::furnace::FUEL => {
                stack.id == item::WATER_BUCKET || stack.id == item::BUCKET
            }
            _ => true,
        }
    }

    /// `putStackInInventoryAllSlots` (with `insertStack`): returns what did not fit.
    fn put_stack(&mut self, inv: &Inv, stack: ItemStack, face: Option<u8>) -> Option<ItemStack> {
        let mut items = self.inv_items(inv);
        let mut stack = Some(stack);
        let mut changed = false;
        let mut put_input = false;
        for slot in Self::slots_for_face(inv, items.len(), face) {
            let Some(s) = stack.as_mut() else { break };
            if s.count <= 0 {
                break;
            }
            if !Self::can_insert(inv, slot, s) {
                continue;
            }
            let moved = match &mut items[slot] {
                None => {
                    items[slot] = stack.take();
                    put_input |= slot == super::furnace::INPUT;
                    true
                }
                Some(existing) if can_combine(existing, s) => {
                    let room = super::item_data::max_stack(existing.id) - existing.count;
                    let n = s.count.min(room);
                    s.count -= n;
                    existing.count += n;
                    n > 0
                }
                Some(_) => false,
            };
            changed |= moved;
            // A hopper receiving items waits before passing them on.
            if moved && let Inv::Hopper(pos) = inv {
                let pos = *pos;
                if let Some(mut data) = self.hopper_data(pos)
                    && data.cooldown <= MAY_TRANSFER
                {
                    data.cooldown = TRANSFER_COOLDOWN;
                    data.items = items.clone();
                    self.store_hopper(pos, &data);
                }
            }
        }
        if changed {
            let put_input = put_input && matches!(inv, Inv::Furnace(_));
            self.set_inv_items(inv, &items, put_input);
        }
        stack.filter(|s| s.count > 0)
    }

    /// `isInventoryFull` / `isInventoryEmpty` on the slots facing `face`.
    fn inv_state(&self, inv: &Inv, face: u8) -> (bool, bool) {
        let items = self.inv_items(inv);
        let slots = Self::slots_for_face(inv, items.len(), Some(face));
        let full = slots.iter().all(|&k| {
            items[k]
                .as_ref()
                .is_some_and(|s| s.count == super::item_data::max_stack(s.id))
        });
        let empty = slots.iter().all(|&k| items[k].is_none());
        (full, empty)
    }

    /// `putStackInInventoryAllSlots` into the inventory at `pos` (droppers): `None`
    /// without an inventory, else whether all of `stack` went in.
    pub(super) fn insert_into_inventory_at(
        &mut self,
        pos: (i32, i32, i32),
        stack: ItemStack,
        face: u8,
    ) -> Option<bool> {
        let inv = self.inventory_at(pos)?;
        Some(self.put_stack(&inv, stack, Some(face)).is_none())
    }

    /// `TileEntityHopper.transferItemsOut`: one item into the inventory it faces.
    fn hopper_push(&mut self, pos: (i32, i32, i32), facing: u8) -> bool {
        let Some(target) = self.inventory_at(offset(pos, facing)) else {
            return false;
        };
        let face = opposite(facing);
        if self.inv_state(&target, face).0 {
            return false;
        }
        for k in 0..HOPPER_SLOTS {
            let Some(mut data) = self.hopper_data(pos) else {
                return false;
            };
            let Some(stack) = data.items[k].clone() else {
                continue;
            };
            let one = ItemStack {
                count: 1,
                ..stack.clone()
            };
            let left = stack.count - 1;
            data.items[k] = (left > 0).then(|| ItemStack {
                count: left,
                ..stack.clone()
            });
            self.store_hopper(pos, &data);
            if self.put_stack(&target, one, Some(face)).is_none() {
                return true;
            }
            // setInventorySlotContents(k, copy): put the item back.
            if let Some(mut data) = self.hopper_data(pos) {
                data.items[k] = Some(stack);
                self.store_hopper(pos, &data);
            }
        }
        false
    }

    /// `TileEntityHopper.captureDroppedItems`: one item from the inventory above, or
    /// item entities just above.
    fn hopper_pull(&mut self, pos: (i32, i32, i32)) -> bool {
        let above = (pos.0, pos.1 + 1, pos.2);
        let hopper = Inv::Hopper(pos);
        if let Some(source) = self.inventory_at(above) {
            if self.inv_state(&source, DOWN).1 {
                return false;
            }
            let len = self.inv_items(&source).len();
            for slot in Self::slots_for_face(&source, len, Some(DOWN)) {
                let mut items = self.inv_items(&source);
                let Some(stack) = items[slot].clone() else {
                    continue;
                };
                if !Self::can_extract(&source, slot, &stack, DOWN) {
                    continue;
                }
                let one = ItemStack {
                    count: 1,
                    ..stack.clone()
                };
                let left = stack.count - 1;
                items[slot] = (left > 0).then(|| ItemStack {
                    count: left,
                    ..stack.clone()
                });
                self.set_inv_items(&source, &items, false);
                if self.put_stack(&hopper, one, None).is_none() {
                    return true;
                }
                let mut items = self.inv_items(&source);
                items[slot] = Some(stack);
                self.set_inv_items(&source, &items, slot == super::furnace::INPUT);
            }
            return false;
        }
        let center = (
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 1.5,
            f64::from(pos.2) + 0.5,
        );
        let area = super::physics::Aabb::new(
            [
                center.0 - PICKUP_RADIUS,
                center.1 - PICKUP_RADIUS,
                center.2 - PICKUP_RADIUS,
            ],
            [
                center.0 + PICKUP_RADIUS,
                center.1 + PICKUP_RADIUS,
                center.2 + PICKUP_RADIUS,
            ],
        );
        let near: Vec<i32> = self
            .items
            .iter()
            .filter(|(_, it)| !it.dead && it.body.bb().intersects(&area))
            .map(|(id, _)| *id)
            .collect();
        for id in near {
            let Some(stack) = self.items.get(&id).map(|it| it.stack.clone()) else {
                continue;
            };
            // putDropInInventoryAllSlots.
            match self.put_stack(&hopper, stack, None) {
                None => {
                    if let Some(it) = self.items.get_mut(&id) {
                        it.dead = true;
                    }
                    return true;
                }
                Some(rest) => {
                    if let Some(it) = self.items.get_mut(&id) {
                        if it.stack != rest {
                            it.dirty_metadata.insert(10);
                        }
                        it.stack = rest;
                    }
                }
            }
        }
        false
    }

    /// `TileEntityHopper.update`.
    pub(super) fn tick_hopper(&mut self, pos: (i32, i32, i32)) {
        {
            let Some(mut data) = self.hopper_data(pos) else {
                return;
            };
            data.cooldown -= 1;
            self.store_hopper(pos, &data);
            if data.cooldown > 0 {
                return;
            }
            data.cooldown = 0;
            self.store_hopper(pos, &data);
            let state = self.level.overworld.block(pos.0, pos.1, pos.2);
            if state & DISABLED != 0 {
                return;
            }
            let mut moved = false;
            if data.items.iter().any(Option::is_some) {
                moved = self.hopper_push(pos, hopper_facing(state & 15));
            }
            let full = self.hopper_items(pos).iter().all(|s| {
                s.as_ref()
                    .is_some_and(|s| s.count == super::item_data::max_stack(s.id))
            });
            if !full {
                moved = self.hopper_pull(pos) || moved;
            }
            if moved && let Some(mut data) = self.hopper_data(pos) {
                data.cooldown = TRANSFER_COOLDOWN;
                self.store_hopper(pos, &data);
                self.update_comparators(pos);
            }
        }
    }

    /// `BlockHopper.updateState`: disabled while powered (set without updates and
    /// without telling clients).
    pub(super) fn hopper_update_state(&mut self, pos: (i32, i32, i32), state: u16) {
        let enabled = !self.is_block_powered(pos);
        if enabled == (state & DISABLED != 0) {
            let new = if enabled {
                state & !DISABLED
            } else {
                state | DISABLED
            };
            self.set_block_flags(pos, new, 0);
        }
    }

    /// `BlockHopper.onBlockActivated` (`displayGUIChest` of the hopper).
    pub(super) fn activate_hopper(&mut self, i: usize, pos: (i32, i32, i32)) {
        let Some(c) = self.level.overworld.tile_entity(pos) else {
            return;
        };
        let custom = c
            .contains_key_of("CustomName", Some(TagType::String))
            .then(|| c.get_str("CustomName").to_owned());
        if self.players[i].window.open.is_some() {
            self.close_screen(i);
        }
        let title = match custom {
            Some(name) => crate::chat::Component::text(name),
            None => crate::chat::Component::translate("container.hopper", vec![]),
        }
        .to_json_string();
        let id = self.players[i].window.next_window_id();
        self.players[i].send(ClientboundPlay::OpenWindow {
            window: id as u8,
            kind: "minecraft:hopper".into(),
            title,
            slots: HOPPER_SLOTS as u8,
        });
        let layout = Layout::Hopper;
        self.players[i].window.open = Some(OpenContainer {
            id,
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
        self.players[i].stats.add("stat.hopperInspected", 1);
    }

    /// `BlockHopper.breakBlock`: the contents spill out.
    pub(super) fn hopper_contents(&self, pos: (i32, i32, i32)) -> Vec<ItemStack> {
        self.hopper_items(pos).into_iter().flatten().collect()
    }
}
