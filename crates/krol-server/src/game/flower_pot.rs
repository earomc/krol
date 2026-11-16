//! Flower pots (vanilla: `BlockFlowerPot`, `TileEntityFlowerPot`, the flower pot
//! `ItemReed`): placing a pot, potting a plant, and the plant dropping when the pot goes.

use krol_nbt::{Compound, Tag};
use krol_protocol::packets::play::ItemStack;

use super::Game;

/// `flower_pot` (the block).
pub const FLOWER_POT: u16 = 140;
/// `flower_pot` (the item).
pub const FLOWER_POT_ITEM: i16 = 390;
/// Block ids below this have an `ItemBlock` (what a pot accepts plants from).
const BLOCK_ITEMS_END: i16 = 256;
/// Plants a pot holds in any variant: sapling, dead bush, dandelion, flowers, both
/// mushrooms, cactus.
const POTTABLE: [i16; 7] = [6, 32, 37, 38, 39, 40, 81];
/// Tall grass only as a fern.
const TALL_GRASS: i16 = 31;
const FERN: i16 = 2;

/// `BlockFlowerPot.canNotContain` (it returns whether the pot *can* hold the plant).
fn can_hold(id: i16, damage: i16) -> bool {
    POTTABLE.contains(&id) || (id == TALL_GRASS && damage == FERN)
}

/// A new pot's tile entity (`TileEntityFlowerPot.writeToNBT` while empty).
pub fn new_tile_entity(pos: (i32, i32, i32)) -> Compound {
    let mut c = Compound::new();
    c.insert("id", Tag::String("FlowerPot".into()));
    c.insert("x", Tag::Int(pos.0));
    c.insert("y", Tag::Int(pos.1));
    c.insert("z", Tag::Int(pos.2));
    c.insert("Item", Tag::String(String::new()));
    c.insert("Data", Tag::Int(0));
    c
}

/// What the pot holds (`getFlowerPotItem`, `getFlowerPotData`): the item by name, or by
/// its numeric id in old data.
fn contents(te: &Compound) -> Option<ItemStack> {
    let id = match te.get("Item")? {
        Tag::String(name) => krol_world::items::id_for_name(name)?,
        t => t.as_i16().filter(|&id| id > 0)?,
    };
    Some(ItemStack {
        id,
        count: 1,
        damage: te.get_i32("Data") as i16,
        nbt: None,
    })
}

impl Game {
    /// `BlockFlowerPot.onBlockActivated`: a plant from the held block item goes into an
    /// empty pot. Returns whether the click was used.
    pub(super) fn activate_flower_pot(&mut self, i: usize, pos: (i32, i32, i32)) -> bool {
        let held = 36 + self.players[i].selected;
        let Some(stack) = self.players[i].inventory[held].clone() else {
            return false;
        };
        if stack.id >= BLOCK_ITEMS_END {
            return false;
        }
        let Some(te) = self.level.overworld.tile_entity(pos) else {
            return false;
        };
        if contents(te).is_some() || !can_hold(stack.id, stack.damage) {
            return false;
        }
        let Some(name) = krol_world::items::name_for_id(stack.id) else {
            return false;
        };
        if let Some(te) = self.level.overworld.tile_entity_mut(pos) {
            te.insert("Item", Tag::String(name.to_owned()));
            te.insert("Data", Tag::Int(i32::from(stack.damage)));
        }
        self.level
            .overworld
            .mark_block_for_update(pos.0, pos.1, pos.2);
        self.add_stat(i, "stat.flowerPotted", 1);
        let player = &mut self.players[i];
        if player.gamemode != 1
            && let Some(s) = &mut player.inventory[held]
        {
            s.count -= 1;
            if s.count <= 0 {
                player.inventory[held] = None;
            }
        }
        true
    }

    /// The plant in the pot at `pos`, read before the pot is replaced: `breakBlock`
    /// drops it once the block changed.
    pub(super) fn flower_pot_contents(&self, pos: (i32, i32, i32)) -> Option<ItemStack> {
        self.level.overworld.tile_entity(pos).and_then(contents)
    }

    /// `BlockFlowerPot.onBlockHarvested` in creative mode: the pot is emptied first, so
    /// the plant does not drop.
    pub(super) fn empty_flower_pot(&mut self, pos: (i32, i32, i32)) {
        if self.level.overworld.block(pos.0, pos.1, pos.2) >> 4 != FLOWER_POT {
            return;
        }
        if let Some(te) = self.level.overworld.tile_entity_mut(pos) {
            te.insert("Item", Tag::String(String::new()));
            te.insert("Data", Tag::Int(0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pots_hold_plants_only() {
        assert!(can_hold(38, 8));
        assert!(can_hold(TALL_GRASS, FERN));
        assert!(!can_hold(TALL_GRASS, 1));
        assert!(!can_hold(1, 0));
        let mut te = new_tile_entity((0, 0, 0));
        assert!(contents(&te).is_none());
        te.insert("Item", Tag::String("minecraft:red_flower".into()));
        te.insert("Data", Tag::Int(3));
        assert_eq!(contents(&te).map(|s| (s.id, s.damage)), Some((38, 3)));
        te.insert("Item", Tag::Int(6));
        assert_eq!(contents(&te).map(|s| s.id), Some(6));
    }
}
