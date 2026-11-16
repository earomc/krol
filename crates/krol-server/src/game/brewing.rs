//! Brewing stands (vanilla `BlockBrewingStand`, `TileEntityBrewingStand`,
//! `ContainerBrewingStand`, `PotionHelper.applyIngredient`).
//!
//! TODO(M5): the potion achievement.

use krol_nbt::{Compound, List, Tag, TagType};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack};

use super::Game;
use super::container::{Layout, OpenContainer};
use super::ids::item;

/// `brewing_stand` (the block) and `brewing_stand` (the item, placed like sugar cane).
pub const BREWING_STAND: u16 = 117;
pub const BREWING_STAND_ITEM: i16 = 379;
/// Slots: three bottles, then the ingredient.
pub const SLOTS: usize = 4;
pub const INGREDIENT: usize = 3;
/// A brew takes 400 ticks.
const BREW_TIME: i32 = 400;
/// The block's metadata has one bit per bottle slot holding something.
const BOTTLE_BITS: [u16; 3] = [1, 2, 4];
/// `fish` with this damage is a pufferfish.
const FISH: i16 = 349;
const PUFFERFISH: i16 = 3;

/// `Item.getPotionEffect`: how an ingredient changes a potion's damage bits.
fn ingredient_effect(stack: &ItemStack) -> Option<&'static str> {
    Some(match stack.id {
        289 => "+14&13-13",       // gunpowder
        331 => "-5+6-7",          // redstone
        348 => "+5-6-7",          // glowstone dust
        353 => "-0+1-2-3&4-4+13", // sugar
        370 => "+0-1-2-3&4-4+13", // ghast tear
        372 => "+4",              // nether wart
        375 => "-0-1+2-3&4-4+13", // spider eye
        376 => "-0+3-4+13",       // fermented spider eye
        377 => "+0-1-2+3&4-4+13", // blaze powder
        378 => "+0+1-2-3&4-4+13", // magma cream
        382 => "+0-1+2-3&4-4+13", // glistering melon
        396 => "-0+1+2-3+13&4-4", // golden carrot
        414 => "+0+1-2+3&4-4+13", // rabbit's foot
        FISH if stack.damage == PUFFERFISH => "+0-1+2+3+13&4-4",
        _ => return None,
    })
}

/// `Item.isPotionIngredient`.
pub fn is_ingredient(stack: &ItemStack) -> bool {
    ingredient_effect(stack).is_some()
}

/// `ContainerBrewingStand.Potion.canHoldPotion`.
pub fn fits_bottle_slot(stack: &ItemStack) -> bool {
    stack.id == item::POTION || stack.id == item::GLASS_BOTTLE
}

/// `PotionHelper.applyIngredient`: the expression flips, sets or clears bits (`+n` sets,
/// `-n` clears, `!n` toggles, `&n` requires the bit); the result keeps 15 bits.
fn apply_ingredient(damage: i32, effect: &str) -> i32 {
    let mut d = damage;
    let mut number = 0;
    let mut has_number = false;
    let (mut toggle, mut clear, mut require) = (false, false, false);
    let step = |d: &mut i32, n: i32, clear: bool, toggle: bool, require: bool| {
        let bit = 1i32.wrapping_shl(n as u32);
        if require {
            if *d & bit == 0 {
                *d = 0;
            }
        } else if clear {
            *d &= !bit;
        } else if toggle {
            *d ^= bit;
        } else {
            *d |= bit;
        }
    };
    for c in effect.bytes() {
        match c {
            b'0'..=b'9' => {
                number = number * 10 + i32::from(c - b'0');
                has_number = true;
            }
            b'!' | b'-' | b'+' | b'&' => {
                if has_number {
                    step(&mut d, number, clear, toggle, require);
                    (toggle, clear, require, has_number, number) = (false, false, false, false, 0);
                }
                match c {
                    b'!' => toggle = true,
                    b'-' => clear = true,
                    b'&' => require = true,
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if has_number {
        step(&mut d, number, clear, toggle, require);
    }
    d & 32767
}

/// `Items.potionitem.getEffects(damage)` with vanilla's cache: the same damage value
/// gives the same list object, and no effects give `null`.
#[derive(PartialEq)]
struct Effects {
    damage: i32,
    list: Vec<super::potion_helper::PotionEffect>,
}

impl Effects {
    fn of(damage: i32) -> Option<Self> {
        let list = super::potion_helper::potion_effects(damage);
        (!list.is_empty()).then_some(Self { damage, list })
    }

    /// `PotionEffect.equals` over the lists (the splash flag included).
    fn same_contents(&self, other: &Self) -> bool {
        self.list == other.list
            && (self.damage & super::potion_helper::SPLASH == 0)
                == (other.damage & super::potion_helper::SPLASH == 0)
    }
}

/// `a == b` for the cached lists: the same object.
fn same_list(a: &Option<Effects>, b: &Option<Effects>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x.damage == y.damage,
        _ => false,
    }
}

fn is_splash(damage: i32) -> bool {
    damage & super::potion_helper::SPLASH != 0
}

/// A brewing stand's saved state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrewingData {
    pub items: Vec<Option<ItemStack>>,
    pub brew_time: i32,
}

impl BrewingData {
    fn read(c: &Compound) -> Self {
        let mut items = vec![None; SLOTS];
        if let Some(list) = c.get_list("Items", TagType::Compound) {
            for tag in list.iter() {
                let Tag::Compound(e) = tag else { continue };
                let slot = usize::from(e.get_i8("Slot") as u8);
                if slot < SLOTS {
                    items[slot] = super::player::item_from_nbt(e);
                }
            }
        }
        Self {
            items,
            brew_time: i32::from(c.get_i16("BrewTime")),
        }
    }

    fn write(&self, c: &mut Compound) {
        c.insert("BrewTime", Tag::Short(self.brew_time as i16));
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

    /// `getPotionResult`.
    fn result(&self, damage: i32) -> i32 {
        match self.items[INGREDIENT].as_ref().and_then(ingredient_effect) {
            Some(effect) => apply_ingredient(damage, effect),
            None => damage,
        }
    }

    fn bottle_damage(&self, k: usize) -> Option<i32> {
        self.items[k]
            .as_ref()
            .filter(|s| s.id == item::POTION)
            .map(|s| i32::from(s.damage))
    }

    /// `canBrew`: some bottle would change.
    fn can_brew(&self) -> bool {
        let Some(ing) = self.items[INGREDIENT].as_ref().filter(|s| s.count > 0) else {
            return false;
        };
        if !is_ingredient(ing) {
            return false;
        }
        (0..3).any(|k| {
            let Some(a) = self.bottle_damage(k) else {
                return false;
            };
            let b = self.result(a);
            if !is_splash(a) && is_splash(b) {
                return true;
            }
            let (ea, eb) = (Effects::of(a), Effects::of(b));
            (a <= 0 || !same_list(&ea, &eb))
                && ea
                    .as_ref()
                    .is_none_or(|x| eb.as_ref().is_some_and(|y| !x.same_contents(y)))
                && a != b
        })
    }

    /// `brewPotions`.
    fn brew(&mut self) {
        if !self.can_brew() {
            return;
        }
        for k in 0..3 {
            let Some(a) = self.bottle_damage(k) else {
                continue;
            };
            let b = self.result(a);
            let (ea, eb) = (Effects::of(a), Effects::of(b));
            let unchanged = (a > 0 && same_list(&ea, &eb))
                || ea
                    .as_ref()
                    .is_some_and(|x| eb.as_ref().is_none_or(|y| x.same_contents(y)));
            let change = if unchanged {
                !is_splash(a) && is_splash(b)
            } else {
                a != b
            };
            if change && let Some(s) = &mut self.items[k] {
                s.damage = b as i16;
            }
        }
        if let Some(ing) = &mut self.items[INGREDIENT] {
            ing.count -= 1;
            if ing.count <= 0 {
                self.items[INGREDIENT] = None;
            }
        }
    }

    /// The bottle bits of the block's metadata (`func_174902_m`).
    fn bottle_bits(&self) -> u16 {
        (0..3)
            .filter(|&k| self.items[k].is_some())
            .map(|k| BOTTLE_BITS[k])
            .sum()
    }
}

impl Game {
    pub(super) fn brewing_data(&self, pos: (i32, i32, i32)) -> Option<BrewingData> {
        self.level
            .overworld
            .tile_entity(pos)
            .filter(|c| c.get_str("id") == "Cauldron")
            .map(BrewingData::read)
    }

    /// Saves a brewing stand (`markDirty`).
    fn store_brewing(&mut self, pos: (i32, i32, i32), data: &BrewingData) {
        let old = self.brewing_data(pos);
        if old.as_ref() == Some(data) {
            return;
        }
        if let Some(c) = self.level.overworld.tile_entity_mut(pos) {
            data.write(c);
        }
        // markDirty after a change of contents: comparators reading the stand.
        if old.is_none_or(|o| o.items != data.items) {
            self.update_comparators(pos);
        }
    }

    /// A brewing stand's four slots (for its window).
    pub(super) fn brewing_items(&self, pos: (i32, i32, i32)) -> Vec<Option<ItemStack>> {
        self.brewing_data(pos)
            .map(|d| d.items)
            .unwrap_or_else(|| vec![None; SLOTS])
    }

    /// Writes a brewing stand window's slots back.
    pub(super) fn set_brewing_items(&mut self, pos: (i32, i32, i32), items: &[Option<ItemStack>]) {
        if let Some(mut data) = self.brewing_data(pos) {
            data.items = items.to_vec();
            self.store_brewing(pos, &data);
        }
    }

    /// `BlockBrewingStand.breakBlock`: the contents spill out.
    pub(super) fn brewing_contents(&self, pos: (i32, i32, i32)) -> Vec<ItemStack> {
        self.brewing_items(pos).into_iter().flatten().collect()
    }

    /// `TileEntityBrewingStand.update`.
    pub(super) fn tick_brewing_stand(&mut self, pos: (i32, i32, i32)) {
        {
            let Some(mut data) = self.brewing_data(pos) else {
                return;
            };
            let ingredient = data.items[INGREDIENT].as_ref().map(|s| s.id);
            if data.brew_time > 0 {
                data.brew_time -= 1;
                if data.brew_time == 0 {
                    data.brew();
                } else if !data.can_brew()
                    || self.brewing_ingredient.get(&pos) != ingredient.as_ref()
                {
                    // The ingredient is not saved: after a reload the brew restarts.
                    data.brew_time = 0;
                }
            } else if data.can_brew() {
                data.brew_time = BREW_TIME;
                if let Some(id) = ingredient {
                    self.brewing_ingredient.insert(pos, id);
                }
            }
            self.store_brewing(pos, &data);
            // The bottles show on the block (setBlockState with flags 2).
            let bits = data.bottle_bits();
            let state = self.level.overworld.block(pos.0, pos.1, pos.2);
            if state >> 4 == BREWING_STAND && state & 7 != bits {
                self.set_block_flags(
                    pos,
                    BREWING_STAND << 4 | bits,
                    super::block_update::flags::SEND,
                );
            }
        }
    }

    /// `BlockBrewingStand.onBlockActivated`: the window (`displayGUIChest`).
    pub(super) fn activate_brewing_stand(&mut self, i: usize, pos: (i32, i32, i32)) {
        let Some(c) = self.level.overworld.tile_entity(pos) else {
            return;
        };
        if c.get_str("id") != "Cauldron" {
            return;
        }
        let custom = c
            .contains_key_of("CustomName", Some(TagType::String))
            .then(|| c.get_str("CustomName").to_owned());
        if self.players[i].window.open.is_some() {
            self.close_screen(i);
        }
        let title = match custom {
            Some(name) => crate::chat::Component::text(name),
            None => crate::chat::Component::translate("container.brewing", vec![]),
        }
        .to_json_string();
        let id = self.players[i].window.next_window_id();
        self.players[i].send(ClientboundPlay::OpenWindow {
            window: id as u8,
            kind: "minecraft:brewing_stand".into(),
            title,
            slots: SLOTS as u8,
        });
        let layout = Layout::Brewing;
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
        // ContainerBrewingStand.onCraftGuiOpened: then the brew time.
        let time = self.brewing_data(pos).map_or(0, |d| d.brew_time);
        self.players[i].send(ClientboundPlay::WindowProperty {
            window: id as u8,
            property: 0,
            value: time as i16,
        });
        if let Some(o) = &mut self.players[i].window.open {
            o.fields[0] = time;
        }
        self.add_stat(i, "stat.brewingstandInteraction", 1);
    }

    /// `ContainerBrewingStand.detectAndSendChanges`: the brew time when it changed.
    pub(super) fn send_brewing_fields(&mut self, i: usize) {
        let Some((pos, id, sent)) = self.players[i]
            .window
            .open
            .as_ref()
            .filter(|o| o.layout == Layout::Brewing)
            .map(|o| (o.pos, o.id, o.fields[0]))
        else {
            return;
        };
        let now = self.brewing_data(pos).map_or(0, |d| d.brew_time);
        if now != sent {
            self.players[i].send(ClientboundPlay::WindowProperty {
                window: id as u8,
                property: 0,
                value: now as i16,
            });
        }
        if let Some(o) = &mut self.players[i].window.open {
            o.fields[0] = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nether_wart_makes_awkward_potions() {
        // Water bottle (0) + nether wart: bit 4 (an awkward potion, 16).
        assert_eq!(apply_ingredient(0, "+4"), 16);
        // Awkward + sugar: swiftness (8194).
        assert_eq!(apply_ingredient(16, "-0+1-2-3&4-4+13"), 8194);
        // Water + sugar: the & needs bit 4, so nothing is left but the following bits.
        assert_eq!(apply_ingredient(0, "-0+1-2-3&4-4+13"), 8192);
    }
}
