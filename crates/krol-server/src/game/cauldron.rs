//! Cauldrons (vanilla: `BlockCauldron`, the cauldron `ItemReed`): filling with water
//! buckets and rain, filling bottles, washing dyed leather armor and banner patterns,
//! putting out burning players, and the comparator reading the water level.

use krol_nbt::{Tag, TagType};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack};

use super::Game;
use super::block_update::flags;

/// `cauldron` (the block).
pub const CAULDRON: u16 = 118;
/// `cauldron` (the item).
pub const CAULDRON_ITEM: i16 = 380;
/// A full cauldron's level (the block metadata).
const FULL: u16 = 3;
/// Rain fills a cauldron by one level with this chance (`nextInt(20) == 1`).
const RAIN_ODDS: i32 = 20;
/// Items the cauldron reacts to.
const WATER_BUCKET: i16 = 326;
const BUCKET: i16 = 325;
const GLASS_BOTTLE: i16 = 374;
const POTION: i16 = 373;
const BANNER: i16 = 425;
/// Leather armor, helmet to boots.
const LEATHER_ARMOR: std::ops::RangeInclusive<i16> = 298..=301;
/// The water surface is this high above the cauldron's bottom at level 0, plus
/// [`LEVEL_HEIGHT`] per level (in sixteenths).
const WATER_BASE: f32 = 6.0;
const LEVEL_HEIGHT: f32 = 3.0;

/// `ItemArmor.hasColor` for leather armor: a `display.color` int.
fn has_color(s: &ItemStack) -> bool {
    s.nbt
        .as_ref()
        .and_then(|n| n.get_compound("display"))
        .is_some_and(|d| d.contains_key_of("color", Some(TagType::Int)))
}

/// `TileEntityBanner.removeBannerData`: the last pattern goes; without patterns left
/// the `BlockEntityTag` goes, and an empty tag with it.
fn remove_last_pattern(s: &mut ItemStack) {
    let Some(nbt) = s.nbt.as_mut() else {
        return;
    };
    let emptied = match nbt.get_mut("BlockEntityTag") {
        Some(Tag::Compound(block)) => match block.get_mut("Patterns") {
            Some(Tag::List(list)) if !list.is_empty() => {
                list.remove(list.len() - 1);
                list.is_empty()
            }
            _ => false,
        },
        _ => false,
    };
    if emptied {
        nbt.remove("BlockEntityTag");
        if nbt.is_empty() {
            s.nbt = None;
        }
    }
}

/// The banner patterns on an item (`TileEntityBanner.getPatterns`).
fn patterns(s: &ItemStack) -> usize {
    s.nbt
        .as_ref()
        .and_then(|n| n.get_compound("BlockEntityTag"))
        .filter(|b| b.contains_key("Patterns"))
        .and_then(|b| b.get_list("Patterns", TagType::Compound))
        .map_or(0, krol_nbt::List::len)
}

impl Game {
    /// `BlockCauldron.setWaterLevel`: the new level, sent but not notified, then the
    /// comparators.
    fn set_water_level(&mut self, pos: (i32, i32, i32), level: u16) {
        self.set_block_flags(pos, CAULDRON << 4 | level.min(FULL), flags::SEND);
        self.update_comparators(pos);
    }

    /// Gives the item to the player, or drops it above the cauldron if it does not fit,
    /// then resends the inventory (`sendContainerToPlayer`) when it fit.
    fn give_from_cauldron(&mut self, i: usize, pos: (i32, i32, i32), stack: ItemStack) {
        let mut stack = stack;
        let creative = self.players[i].gamemode == 1;
        if super::item_entity::add_to_inventory(
            &mut self.players[i].inventory,
            &mut stack,
            creative,
        ) {
            let p = &self.players[i];
            p.send(ClientboundPlay::WindowItems {
                window: 0,
                items: p.inventory.clone(),
            });
            p.send(ClientboundPlay::SetSlot {
                window: -1,
                slot: -1,
                item: p.window.cursor.clone(),
            });
        } else {
            let at = (
                f64::from(pos.0) + 0.5,
                f64::from(pos.1) + 1.5,
                f64::from(pos.2) + 0.5,
            );
            self.spawn_item(at, stack, 0);
        }
    }

    /// `BlockCauldron.onBlockActivated`: water buckets fill it, bottles take water,
    /// dyed leather armor and patterned banners are washed. An empty hand takes the
    /// click too. Returns whether the click was used.
    pub(super) fn activate_cauldron(&mut self, i: usize, pos: (i32, i32, i32), state: u16) -> bool {
        let held = 36 + self.players[i].selected;
        let Some(stack) = self.players[i].inventory[held].clone() else {
            return true;
        };
        let level = state & 15;
        let creative = self.players[i].gamemode == 1;
        match stack.id {
            WATER_BUCKET => {
                if level < FULL {
                    if !creative {
                        self.players[i].inventory[held] = Some(ItemStack {
                            id: BUCKET,
                            count: 1,
                            damage: 0,
                            nbt: None,
                        });
                    }
                    self.add_stat(i, "stat.cauldronFilled", 1);
                    self.set_water_level(pos, FULL);
                }
                true
            }
            GLASS_BOTTLE => {
                if level > 0 {
                    if !creative {
                        let water = ItemStack {
                            id: POTION,
                            count: 1,
                            damage: 0,
                            nbt: None,
                        };
                        self.give_from_cauldron(i, pos, water);
                        self.add_stat(i, "stat.cauldronUsed", 1);
                        let p = &mut self.players[i];
                        if let Some(s) = &mut p.inventory[held] {
                            s.count -= 1;
                            if s.count <= 0 {
                                p.inventory[held] = None;
                            }
                        }
                    }
                    self.set_water_level(pos, level - 1);
                }
                true
            }
            id if level > 0 && LEATHER_ARMOR.contains(&id) && has_color(&stack) => {
                // ItemArmor.removeColor leaves the display compound.
                if let Some(Some(s)) = self.players[i].inventory.get_mut(held)
                    && let Some(Tag::Compound(display)) =
                        s.nbt.as_mut().and_then(|n| n.get_mut("display"))
                {
                    display.remove("color");
                }
                self.set_water_level(pos, level - 1);
                self.add_stat(i, "stat.armorCleaned", 1);
                true
            }
            BANNER if level > 0 && patterns(&stack) > 0 => {
                let mut washed = ItemStack {
                    count: 1,
                    ..stack.clone()
                };
                remove_last_pattern(&mut washed);
                if stack.count <= 1 && !creative {
                    self.players[i].inventory[held] = Some(washed);
                } else {
                    self.give_from_cauldron(i, pos, washed);
                    self.add_stat(i, "stat.bannerCleaned", 1);
                    if !creative && let Some(s) = &mut self.players[i].inventory[held] {
                        s.count -= 1;
                    }
                }
                if !creative {
                    self.set_water_level(pos, level - 1);
                }
                true
            }
            _ => false,
        }
    }

    /// `BlockCauldron.fillWithRain`, for the block below the precipitation height that
    /// the weather tick picked.
    pub(super) fn fill_cauldron_with_rain(&mut self, pos: (i32, i32, i32)) {
        let state = self.level.overworld.block(pos.0, pos.1, pos.2);
        if state >> 4 != CAULDRON {
            return;
        }
        if self.world_rand.next_int_bounded(RAIN_ODDS) == 1 && state & 15 < FULL {
            // cycleProperty, without the comparator update.
            self.set_block_flags(pos, state + 1, flags::SEND);
        }
    }

    /// `BlockCauldron.onEntityCollidedWithBlock` for a player: water puts a burning
    /// player out, using up a level.
    pub(super) fn cauldron_collision(
        &mut self,
        i: usize,
        pos: (i32, i32, i32),
        state: u16,
        min_y: f64,
    ) {
        let level = state & 15;
        let surface = pos.1 as f32 + (WATER_BASE + LEVEL_HEIGHT * f32::from(level)) / 16.0;
        if self.player_fire(i) > 0 && level > 0 && min_y <= f64::from(surface) {
            self.extinguish_player(i);
            self.set_water_level(pos, level - 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use krol_nbt::Compound;

    #[test]
    fn washing_banners_removes_the_last_pattern() {
        let pattern = |id: &str| {
            let mut p = Compound::new();
            p.insert("Pattern", Tag::String(id.into()));
            Tag::Compound(p)
        };
        let mut list = krol_nbt::List::new();
        let _ = list.push(pattern("bs"));
        let _ = list.push(pattern("ts"));
        let mut block = Compound::new();
        block.insert("Base", Tag::Int(4));
        block.insert("Patterns", Tag::List(list));
        let mut nbt = Compound::new();
        nbt.insert("BlockEntityTag", Tag::Compound(block));
        let mut s = ItemStack {
            id: BANNER,
            count: 1,
            damage: 4,
            nbt: Some(nbt),
        };
        remove_last_pattern(&mut s);
        assert_eq!(patterns(&s), 1);
        remove_last_pattern(&mut s);
        // The last pattern takes the BlockEntityTag (and the emptied tag) with it.
        assert!(s.nbt.is_none());
    }
}
