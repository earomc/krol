//! Furnaces (vanilla: `BlockFurnace`, `TileEntityFurnace`, `ContainerFurnace`,
//! `SlotFurnaceFuel`, `SlotFurnaceOutput`).
//!
//! The furnace's items and timers live in its tile entity NBT; only
//! `currentItemBurnTime`, which vanilla does not save, is kept on the side.
//!
//! TODO(M5): the acquireIron and cookFish achievements. TODO(M4): hoppers,
//! comparators.

use krol_nbt::{Compound, List, Tag, TagType};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack};

use super::Game;
use super::block_data::{self, Material};
use super::container::{Layout, OpenContainer};
use super::ids::{block, item};
use super::world::World;

/// The input slot.
pub const INPUT: usize = 0;
/// The fuel slot.
pub const FUEL: usize = 1;
/// The output slot.
pub const OUTPUT: usize = 2;
/// Ticks to smelt one item (`getCookTime`).
const COOK_TIME: i32 = 200;
/// The window properties, in `getField` order.
const FIELD_BURN: usize = 0;
const FIELD_ITEM_BURN: usize = 1;
const FIELD_COOK: usize = 2;
const FIELD_TOTAL: usize = 3;
/// The order `ContainerFurnace.detectAndSendChanges` checks the properties in.
const FIELD_SEND_ORDER: [usize; 4] = [FIELD_COOK, FIELD_BURN, FIELD_ITEM_BURN, FIELD_TOTAL];
/// The furnace's stack size limit (`getInventoryStackLimit`).
const STACK_LIMIT: i8 = 64;

/// A furnace's saved state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FurnaceData {
    /// Input, fuel and output.
    pub items: [Option<ItemStack>; 3],
    /// `furnaceBurnTime`.
    pub burn: i32,
    /// `cookTime`.
    pub cook: i32,
    /// `totalCookTime`.
    pub total: i32,
}

impl FurnaceData {
    /// `TileEntityFurnace.readFromNBT`.
    pub fn read(c: &Compound) -> Self {
        let mut items: [Option<ItemStack>; 3] = Default::default();
        if let Some(list) = c.get_list("Items", TagType::Compound) {
            for tag in list.iter() {
                let Tag::Compound(e) = tag else { continue };
                let slot = e.get_i8("Slot");
                if (0..3).contains(&slot) {
                    items[slot as usize] = super::player::item_from_nbt(e);
                }
            }
        }
        Self {
            items,
            burn: i32::from(c.get_i16("BurnTime")),
            cook: i32::from(c.get_i16("CookTime")),
            total: i32::from(c.get_i16("CookTimeTotal")),
        }
    }

    /// `TileEntityFurnace.writeToNBT` (into the existing compound, keeping its order).
    fn write(&self, c: &mut Compound) {
        c.insert("BurnTime", Tag::Short(self.burn as i16));
        c.insert("CookTime", Tag::Short(self.cook as i16));
        c.insert("CookTimeTotal", Tag::Short(self.total as i16));
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
    }

    fn burning(&self) -> bool {
        self.burn > 0
    }

    /// `canSmelt`.
    fn can_smelt(&self) -> bool {
        let Some(input) = &self.items[INPUT] else {
            return false;
        };
        let Some(result) = super::recipes::smelting(input) else {
            return false;
        };
        let Some(out) = &self.items[OUTPUT] else {
            return true;
        };
        if out.id != result.output.0 || out.damage != result.output.2 {
            return false;
        }
        let max = super::item_entity::max_stack(out.id);
        if out.count < STACK_LIMIT && out.count < max {
            true
        } else {
            out.count < super::item_entity::max_stack(result.output.0)
        }
    }

    /// `smeltItem`.
    fn smelt(&mut self) {
        if !self.can_smelt() {
            return;
        }
        let Some(input) = self.items[INPUT].clone() else {
            return;
        };
        let Some(result) = super::recipes::smelting(&input) else {
            return;
        };
        match &mut self.items[OUTPUT] {
            None => {
                self.items[OUTPUT] = Some(ItemStack {
                    id: result.output.0,
                    count: result.output.1,
                    damage: result.output.2,
                    nbt: None,
                });
            }
            // Vanilla quirk: one more item regardless of the recipe's output count.
            Some(out) if out.id == result.output.0 => out.count += 1,
            Some(_) => {}
        }
        // A wet sponge fills a bucket in the fuel slot.
        if input.id == item::SPONGE
            && input.damage == item::WET_SPONGE_DAMAGE
            && self.items[FUEL]
                .as_ref()
                .is_some_and(|f| f.id == item::BUCKET)
        {
            self.items[FUEL] = Some(ItemStack {
                id: item::WATER_BUCKET,
                count: 1,
                damage: 0,
                nbt: None,
            });
        }
        if let Some(s) = &mut self.items[INPUT] {
            s.count -= 1;
            if s.count <= 0 {
                self.items[INPUT] = None;
            }
        }
    }
}

/// `TileEntityFurnace.getItemBurnTime`.
pub fn burn_time(stack: Option<&ItemStack>) -> i32 {
    let Some(s) = stack else {
        return 0;
    };
    if (1..256).contains(&s.id) {
        let id = s.id as u16;
        if id == block::WOODEN_SLAB {
            return 150;
        }
        if block_data::material(id) == Material::Wood {
            return 300;
        }
        if id == block::COAL_BLOCK {
            return 16000;
        }
    }
    match s.id {
        // Wooden tools, sword and hoe.
        item::WOODEN_SWORD..=item::WOODEN_AXE | item::WOODEN_HOE => 200,
        item::STICK => 100,
        item::COAL => 1600,
        item::LAVA_BUCKET => 20000,
        item::SAPLING => 100,
        item::BLAZE_ROD => 2400,
        _ => 0,
    }
}

/// `SlotFurnaceFuel.isItemValid`: fuel or an empty bucket.
pub fn fits_fuel_slot(stack: &ItemStack) -> bool {
    burn_time(Some(stack)) > 0 || stack.id == item::BUCKET
}

/// A furnace's state, if `pos` has a furnace tile entity.
pub fn furnace_data(world: &World, pos: (i32, i32, i32)) -> Option<FurnaceData> {
    world
        .tile_entity(pos)
        .filter(|c| c.get_str("id") == "Furnace")
        .map(FurnaceData::read)
}

impl Game {
    /// `currentItemBurnTime`; after loading, vanilla derives it from the fuel slot.
    fn current_burn(&mut self, pos: (i32, i32, i32), data: &FurnaceData) -> i32 {
        *self
            .furnace_burn
            .entry(pos)
            .or_insert_with(|| burn_time(data.items[FUEL].as_ref()))
    }

    /// The window property values (`getField` 0-3).
    pub(super) fn furnace_fields(&mut self, pos: (i32, i32, i32)) -> [i32; 4] {
        let Some(data) = furnace_data(&self.level.overworld, pos) else {
            return [0; 4];
        };
        let current = self.current_burn(pos, &data);
        let mut fields = [0; 4];
        fields[FIELD_BURN] = data.burn;
        fields[FIELD_ITEM_BURN] = current;
        fields[FIELD_COOK] = data.cook;
        fields[FIELD_TOTAL] = data.total;
        fields
    }

    fn store_furnace(&mut self, pos: (i32, i32, i32), data: &FurnaceData) {
        let old = furnace_data(&self.level.overworld, pos);
        if old.as_ref() == Some(data) {
            return;
        }
        if let Some(c) = self.level.overworld.tile_entity_mut(pos) {
            data.write(c);
        }
        // TileEntity.markDirty: comparators reading the furnace re-check.
        if old.is_none_or(|o| o.items != data.items) {
            self.update_comparators(pos);
        }
    }

    /// An open furnace window's three slots.
    pub(super) fn furnace_items(&self, pos: (i32, i32, i32)) -> Vec<Option<ItemStack>> {
        furnace_data(&self.level.overworld, pos)
            .map(|d| d.items.to_vec())
            .unwrap_or_else(|| vec![None; 3])
    }

    /// Writes an open furnace window's slots back. `put_input` says the click put a
    /// stack into the input slot (`setInventorySlotContents(0, ...)`), which restarts
    /// the cooking unless the same kind of item was there.
    pub(super) fn set_furnace_items(
        &mut self,
        pos: (i32, i32, i32),
        items: &[Option<ItemStack>],
        put_input: bool,
    ) {
        let Some(mut data) = furnace_data(&self.level.overworld, pos) else {
            return;
        };
        let before = data.items[INPUT].clone();
        let new = items[INPUT].clone();
        let same = match (&new, &before) {
            (Some(a), Some(b)) => a.id == b.id && a.damage == b.damage && a.nbt == b.nbt,
            _ => false,
        };
        if put_input && !same {
            data.total = COOK_TIME;
            data.cook = 0;
        }
        data.items = [
            items[INPUT].clone(),
            items[FUEL].clone(),
            items[OUTPUT].clone(),
        ];
        self.store_furnace(pos, &data);
    }

    /// `BlockFurnace.onBlockActivated`.
    pub(super) fn activate_furnace(&mut self, i: usize, pos: (i32, i32, i32)) {
        let Some(c) = self.level.overworld.tile_entity(pos) else {
            return;
        };
        if c.get_str("id") != "Furnace" {
            return;
        }
        let custom = c
            .contains_key_of("CustomName", Some(TagType::String))
            .then(|| c.get_str("CustomName").to_owned());
        // EntityPlayerMP.displayGUIChest.
        if self.players[i].window.open.is_some() {
            self.close_screen(i);
        }
        let title = match custom {
            Some(name) => crate::chat::Component::text(name),
            None => crate::chat::Component::translate("container.furnace", vec![]),
        }
        .to_json_string();
        let id = self.players[i].window.next_window_id();
        self.players[i].send(ClientboundPlay::OpenWindow {
            window: id as u8,
            kind: "minecraft:furnace".into(),
            title,
            slots: (OUTPUT + 1) as u8,
        });
        let layout = Layout::Furnace;
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
        // ContainerFurnace.onCraftGuiOpened: then every property.
        let fields = self.furnace_fields(pos);
        for (property, value) in fields.into_iter().enumerate() {
            self.players[i].send(ClientboundPlay::WindowProperty {
                window: id as u8,
                property: property as i16,
                value: value as i16,
            });
        }
        self.players[i].stats.add("stat.furnaceInteraction", 1);
    }

    /// `ContainerFurnace.detectAndSendChanges`: the properties that changed, in
    /// vanilla's order (cook time, burn time, item burn time, total cook time).
    pub(super) fn send_furnace_fields(&mut self, i: usize) {
        let Some((pos, id, sent)) = self.players[i]
            .window
            .open
            .as_ref()
            .filter(|o| o.layout == Layout::Furnace)
            .map(|o| (o.pos, o.id, o.fields))
        else {
            return;
        };
        let now = self.furnace_fields(pos);
        for property in FIELD_SEND_ORDER {
            if sent[property] != now[property] {
                self.players[i].send(ClientboundPlay::WindowProperty {
                    window: id as u8,
                    property: property as i16,
                    value: now[property] as i16,
                });
            }
        }
        if let Some(o) = &mut self.players[i].window.open {
            o.fields = now;
        }
    }

    /// `SlotFurnaceOutput`: items taken from the output count as crafted
    /// (`ItemStack.onCrafting`) once the slot reports a pickup; a double click only
    /// adds to the count.
    pub(super) fn furnace_output_taken(
        &mut self,
        i: usize,
        output: &ItemStack,
        amount: i32,
        pickup: bool,
    ) {
        let Some(o) = &mut self.players[i].window.open else {
            return;
        };
        o.taken += amount;
        if !pickup {
            return;
        }
        let taken = std::mem::take(&mut o.taken);
        if let Some(name) = krol_world::items::name_for_id(output.id) {
            self.players[i]
                .stats
                .add(&format!("stat.craftItem.{}", name.replace(':', ".")), taken);
        }
        self.furnace_xp(i, output, taken);
    }

    /// `TileEntityFurnace.update`.
    pub(super) fn tick_furnace(&mut self, pos: (i32, i32, i32)) {
        {
            let Some(mut data) = furnace_data(&self.level.overworld, pos) else {
                return;
            };
            let mut current = self.current_burn(pos, &data);
            let was_burning = data.burning();
            if data.burning() {
                data.burn -= 1;
            }
            if data.burning() || (data.items[FUEL].is_some() && data.items[INPUT].is_some()) {
                if !data.burning() && data.can_smelt() {
                    data.burn = burn_time(data.items[FUEL].as_ref());
                    current = data.burn;
                    if data.burning()
                        && let Some(fuel) = &mut data.items[FUEL]
                    {
                        fuel.count -= 1;
                        if fuel.count == 0 {
                            data.items[FUEL] = super::recipes::container_item(fuel.id);
                        }
                    }
                }
                if data.burning() && data.can_smelt() {
                    data.cook += 1;
                    if data.cook == data.total {
                        data.cook = 0;
                        data.total = COOK_TIME;
                        data.smelt();
                    }
                } else {
                    data.cook = 0;
                }
            } else if !data.burning() && data.cook > 0 {
                data.cook = (data.cook - 2).clamp(0, data.total);
            }
            self.furnace_burn.insert(pos, current);
            self.store_furnace(pos, &data);
            if was_burning != data.burning() {
                // BlockFurnace.setState: lit or unlit, same facing, same tile entity.
                let w = &mut self.level.overworld;
                let state = w.block(pos.0, pos.1, pos.2);
                let id = if data.burning() {
                    block::LIT_FURNACE
                } else {
                    block::FURNACE
                };
                self.set_block_notify(pos, id << 4 | (state & 15));
            }
        }
    }

    /// `BlockFurnace.breakBlock`: the contents spill out like a chest's.
    pub(super) fn furnace_contents(&self, pos: (i32, i32, i32)) -> Vec<ItemStack> {
        furnace_data(&self.level.overworld, pos)
            .map(|d| d.items.into_iter().flatten().collect())
            .unwrap_or_default()
    }
}
