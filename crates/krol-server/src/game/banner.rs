//! Banners as blocks (vanilla: `ItemBanner`, `BlockBanner`, `TileEntityBanner`): placing
//! standing and wall banners with the item's colors and patterns, and the item they
//! drop. Crafting their patterns is in [`super::special_recipes`].

use krol_nbt::{Compound, Tag, TagType};
use krol_protocol::packets::play::ItemStack;

use super::Game;
use super::ids::{block, item};
use super::placement::{self, offset};
use super::sign::solid_material;

/// A standing banner's 16 rotations (like signs).
const ROTATIONS: f32 = 16.0;

/// Whether `id` is a banner block.
pub fn is_banner(id: u16) -> bool {
    id == block::STANDING_BANNER || id == block::WALL_BANNER
}

/// `TileEntityBanner.func_181020_a`: `Base`, and `Patterns` if the banner has them.
fn write_banner(c: &mut Compound, base: i32, patterns: Option<&krol_nbt::List>) {
    c.insert("Base", Tag::Int(base));
    if let Some(p) = patterns {
        c.insert("Patterns", Tag::List(p.clone()));
    }
}

/// The new tile entity of a banner block (`TileEntityBanner.writeToNBT` before any item
/// values: base color 0, no patterns).
pub fn new_tile_entity(pos: (i32, i32, i32)) -> Compound {
    let mut c = Compound::new();
    c.insert("id", Tag::String("Banner".into()));
    c.insert("x", Tag::Int(pos.0));
    c.insert("y", Tag::Int(pos.1));
    c.insert("z", Tag::Int(pos.2));
    write_banner(&mut c, 0, None);
    c
}

/// The banner item a harvested banner drops (`BlockBanner.harvestBlock`): its base
/// color as damage, and `BlockEntityTag` with `Base` and its patterns.
fn harvest_drop(te: &Compound) -> ItemStack {
    let base = te.get_i32("Base");
    let mut tag = Compound::new();
    write_banner(&mut tag, base, banner_patterns(te));
    item_with_tag(base, tag)
}

/// The banner item a banner drops otherwise (`BlockBanner.dropBlockAsItemWithChance`):
/// the whole tile entity without its id and position.
fn other_drop(te: &Compound) -> ItemStack {
    let mut tag = te.clone();
    for key in ["x", "y", "z", "id"] {
        tag.remove(key);
    }
    item_with_tag(te.get_i32("Base"), tag)
}

/// The patterns a loaded banner writes back (`readFromNBT` reads a list in any case).
fn banner_patterns(te: &Compound) -> Option<&krol_nbt::List> {
    if !te.contains_key("Patterns") {
        return None;
    }
    te.get_list("Patterns", TagType::Compound)
}

fn item_with_tag(base: i32, block_entity: Compound) -> ItemStack {
    let mut nbt = Compound::new();
    nbt.insert("BlockEntityTag", Tag::Compound(block_entity));
    ItemStack {
        id: item::BANNER,
        count: 1,
        damage: base as i16,
        nbt: Some(nbt),
    }
}

impl Game {
    /// `ItemBanner.onItemUse`: a standing banner on top of a solid block, a wall banner
    /// on its side, taking the item's base color and patterns. Returns whether the item
    /// was used.
    pub(super) fn place_banner(&mut self, i: usize, pos: (i32, i32, i32), face: u8) -> bool {
        let w = &mut self.level.overworld;
        if face == placement::DOWN || !solid_material(w.block(pos.0, pos.1, pos.2)) {
            return false;
        }
        let at = offset(pos, face);
        // BlockBanner.canPlaceBlockAt: replaceable, no cactus next to it (BlockContainer).
        if !(0..256).contains(&at.1)
            || !super::blocks::is_replaceable(w.block(at.0, at.1, at.2))
            || [
                placement::NORTH,
                placement::SOUTH,
                placement::WEST,
                placement::EAST,
            ]
            .iter()
            .any(|&f| {
                let n = offset(at, f);
                w.block(n.0, n.1, n.2) >> 4 == block::CACTUS
            })
        {
            return false;
        }
        let state = if face == placement::UP {
            let yaw = self.players[i].rotation.0;
            let rotation =
                krol_java::math::floor_f64(f64::from((yaw + 180.0) * ROTATIONS / 360.0) + 0.5) & 15;
            block::STANDING_BANNER << 4 | rotation as u16
        } else {
            block::WALL_BANNER << 4 | u16::from(face)
        };
        let held = 36 + self.players[i].selected;
        let Some(stack) = self.players[i].inventory[held].clone() else {
            return false;
        };
        self.set_block_notify(at, state);
        self.count_item_use(i);
        let player = &mut self.players[i];
        if player.gamemode != 1
            && let Some(s) = &mut player.inventory[held]
        {
            s.count -= 1;
        }
        // TileEntityBanner.setItemValues.
        let tag = stack
            .nbt
            .as_ref()
            .and_then(|n| n.get_compound("BlockEntityTag"));
        let patterns = tag.filter(|t| t.contains_key("Patterns")).map(|t| {
            t.get_list("Patterns", TagType::Compound)
                .cloned()
                .unwrap_or_default()
        });
        let base = match tag {
            Some(t) if t.contains_key_of("Base", None) => t.get_i32("Base"),
            _ => i32::from(stack.damage & 15),
        };
        if let Some(te) = self.level.overworld.tile_entity_mut(at) {
            te.remove("Patterns");
            write_banner(te, base, patterns.as_ref());
        }
        true
    }

    /// `BlockBanner.harvestBlock` with the banner's tile entity (taken before the block
    /// was removed): the banner item, without the exhaustion other blocks cost.
    pub(super) fn harvest_banner(&mut self, pos: (i32, i32, i32), te: &Compound) {
        self.spawn_as_entity(pos, harvest_drop(te));
    }

    /// `BlockBanner.dropBlockAsItemWithChance`: the banner item whatever the chance, or
    /// nothing without a tile entity. Returns whether `state` was a banner.
    pub(super) fn drop_banner(&mut self, pos: (i32, i32, i32), state: u16) -> bool {
        if !is_banner(state >> 4) {
            return false;
        }
        if let Some(stack) = self.level.overworld.tile_entity(pos).map(other_drop) {
            self.spawn_as_entity(pos, stack);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_carry_the_tile_entity() {
        let mut te = new_tile_entity((1, 2, 3));
        te.insert("Base", Tag::Int(4));
        let mut patterns = krol_nbt::List::new();
        let mut p = Compound::new();
        p.insert("Pattern", Tag::String("cbo".into()));
        p.insert("Color", Tag::Int(0));
        let _ = patterns.push(Tag::Compound(p));
        te.insert("Patterns", Tag::List(patterns));
        let harvested = harvest_drop(&te);
        assert_eq!((harvested.id, harvested.damage), (item::BANNER, 4));
        let tag = harvested
            .nbt
            .as_ref()
            .and_then(|n| n.get_compound("BlockEntityTag"));
        assert_eq!(tag.map(|t| t.len()), Some(2));
        let other = other_drop(&te);
        let tag = other
            .nbt
            .as_ref()
            .and_then(|n| n.get_compound("BlockEntityTag"));
        assert!(tag.is_some_and(|t| !t.contains_key("x") && t.contains_key("Patterns")));
        // A fresh banner has no patterns to write.
        let plain = harvest_drop(&new_tile_entity((0, 0, 0)));
        let tag = plain
            .nbt
            .as_ref()
            .and_then(|n| n.get_compound("BlockEntityTag"));
        assert!(tag.is_some_and(|t| !t.contains_key("Patterns")));
    }
}
