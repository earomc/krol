//! Sheep (vanilla `EntitySheep`, `EntityAIEatGrass`, `ItemDye.itemInteractionForEntity`):
//! the fleece (a color and a sheared bit in metadata entry 16), shearing, dyeing,
//! eating grass to grow the wool back, the colors of new and newborn sheep.

use krol_java::JavaRandom;
use krol_protocol::packets::play::{ClientboundPlay, ItemStack};

use super::{FLEECE_INDEX, Mob, MobKind};
use crate::game::Game;
use crate::game::block_update::flags;
use crate::game::ids::{block, item};

/// The sheared bit of the fleece byte; the color is in the bits below it.
const SHEARED_BIT: i8 = 16;
const COLOR_MASK: i8 = 15;
/// `EnumDyeColor` metadata (wool damage) of the colors sheep spawn with.
const WHITE: i8 = 0;
const PINK: i8 = 6;
const GRAY: i8 = 7;
const SILVER: i8 = 8;
const BROWN: i8 = 12;
const BLACK: i8 = 15;
/// There are 16 colors; a dye's damage is 15 minus the color's metadata.
const COLORS: i32 = 16;
/// `EntitySheep.interact`: shearing drops 1 to this many wool from 1 block up.
const SHEARED_WOOL: i32 = 3;
const SHEARED_WOOL_LIFT: f64 = 1.0;
/// `EntityItem.setDefaultPickupDelay`.
const DROP_PICKUP_DELAY: i32 = 10;
/// `EntityAIEatGrass`: the chance of a child and of an adult starting to eat (one in
/// this many task ticks), how long it eats and when the grass goes.
const EAT_CHANCE_CHILD: i32 = 50;
const EAT_CHANCE_ADULT: i32 = 1000;
pub(super) const EAT_TICKS: i32 = 40;
pub(super) const EAT_AT: i32 = 4;
/// `S19PacketEntityStatus` 10: the head lowered to eat.
const STATUS_EAT_GRASS: i8 = 10;
/// `BlockTallGrass.EnumType.GRASS` (not the dead bush or the fern).
const TALL_GRASS_META: u16 = 1;
/// `playAuxSFX` 2001: a block's break effect.
const BREAK_EFFECT: i32 = 2001;
/// `eatGrassBonus`: a child grows by this many seconds.
const EAT_GROWTH_SECONDS: i32 = 60;
const MOB_GRIEFING: &str = "mobGriefing";

/// `EnumDyeColor.byMetadata`: out-of-range metadata is white.
pub(super) fn color_by_meta(meta: i32) -> i8 {
    if (0..COLORS).contains(&meta) {
        meta as i8
    } else {
        WHITE
    }
}

/// `EnumDyeColor.byDyeDamage`: out-of-range damage is that of ink, black.
fn color_by_dye_damage(damage: i32) -> i8 {
    if (0..COLORS).contains(&damage) {
        (COLORS - 1 - damage) as i8
    } else {
        BLACK
    }
}

/// `EnumDyeColor.getDyeDamage`.
fn dye_damage(color: i8) -> i16 {
    (COLORS as i16 - 1) - i16::from(color)
}

/// `EntitySheep.getRandomSheepColor`.
pub(super) fn random_color(rand: &mut JavaRandom) -> i8 {
    match rand.next_int_bounded(100) {
        0..5 => BLACK,
        5..10 => GRAY,
        10..15 => SILVER,
        15..18 => BROWN,
        _ if rand.next_int_bounded(500) == 0 => PINK,
        _ => WHITE,
    }
}

/// One wool block of `color`.
pub(super) fn wool(color: i8) -> ItemStack {
    ItemStack {
        id: item::WOOL,
        count: 1,
        damage: i16::from(color),
        nbt: None,
    }
}

impl Mob {
    /// `getFleeceColor`.
    pub(super) fn fleece_color(&self) -> i8 {
        self.fleece & COLOR_MASK
    }

    /// `setFleeceColor`.
    pub(super) fn set_fleece_color(&mut self, color: i8) {
        self.set_fleece(self.fleece & !COLOR_MASK | color & COLOR_MASK);
    }

    /// `getSheared`.
    pub(super) fn sheared(&self) -> bool {
        self.fleece & SHEARED_BIT != 0
    }

    /// `setSheared`.
    pub(super) fn set_sheared(&mut self, sheared: bool) {
        let fleece = if sheared {
            self.fleece | SHEARED_BIT
        } else {
            self.fleece & !SHEARED_BIT
        };
        self.set_fleece(fleece);
    }

    fn set_fleece(&mut self, fleece: i8) {
        if fleece != self.fleece {
            self.fleece = fleece;
            self.metadata_updated(FLEECE_INDEX);
        }
    }
}

impl Game {
    /// `EntitySheep.interact`, before the animal's: shears cut an adult sheep's wool,
    /// 1 to 3 blocks of its color thrown from 1 block up, the shears wear and
    /// `mob.sheep.shear` plays. The click goes on to the animal either way.
    ///
    /// Vanilla quirk: the click is not taken, so shears worn out by the cut are left in
    /// the hand as an empty stack (`EntityPlayer.interactWith` only clears it when
    /// something took the click).
    pub(super) fn shear_sheep(&mut self, i: usize, id: i32) {
        if self.players[i].held().is_none_or(|h| h.id != item::SHEARS) {
            return;
        }
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        if m.kind != MobKind::Sheep || m.sheared() || m.child() {
            return;
        }
        m.set_sheared(true);
        let count = 1 + m.rand.next_int_bounded(SHEARED_WOOL);
        let (pos, color) = (m.body.pos, m.fleece_color());
        let top = (pos.0, pos.1 + SHEARED_WOOL_LIFT, pos.2);
        for _ in 0..count {
            let wool_id = self.spawn_item_moving(top, wool(color), DROP_PICKUP_DELAY, None);
            let Some(m) = self.mobs.get_mut(&id) else {
                return;
            };
            let dy = m.rand.next_float() * 0.05;
            let dx = (m.rand.next_float() - m.rand.next_float()) * 0.1;
            let dz = (m.rand.next_float() - m.rand.next_float()) * 0.1;
            if let Some(it) = self.items.get_mut(&wool_id) {
                it.body.motion.1 += f64::from(dy);
                it.body.motion.0 += f64::from(dx);
                it.body.motion.2 += f64::from(dz);
            }
        }
        self.damage_held_item(i, 1);
        if let Some(m) = self.mobs.get(&id)
            && !m.base.silent
        {
            let pos = m.body.pos;
            self.play_sound_near_except(None, "mob.sheep.shear", pos, 1.0, 1.0);
        }
    }

    /// `ItemDye.itemInteractionForEntity`: a dye takes the click on any sheep and
    /// colors one that is not sheared and of another color (used up). Returns whether
    /// it took the click.
    pub(super) fn dye_sheep(&mut self, i: usize, id: i32, held: &ItemStack) -> bool {
        let Some(m) = self.mobs.get_mut(&id) else {
            return false;
        };
        if m.kind != MobKind::Sheep {
            return false;
        }
        let color = color_by_dye_damage(i32::from(held.damage));
        if !m.sheared() && m.fleece_color() != color {
            m.set_fleece_color(color);
            self.use_held(i);
        }
        true
    }

    /// `EntitySheep.getDyeColorMixFromParents` for a child of `parent` and `mate`: the
    /// dye a crafting grid makes of the parents' dyes, else either parent's color (the
    /// world's random picks).
    pub(super) fn child_fleece(&mut self, parent: i32, mate: i32) -> Option<i8> {
        let a = self.mobs.get(&parent)?.fleece_color();
        let b = self.mobs.get(&mate)?.fleece_color();
        let dye = |color| {
            Some(ItemStack {
                id: item::DYE,
                count: 1,
                damage: dye_damage(color),
                nbt: None,
            })
        };
        // A 2 x 1 grid: a 2 x 2 one with an empty bottom row matches the same.
        let grid = [dye(a), dye(b), None, None];
        let mixed = crate::game::recipes::craft(&grid, 2, &|_| None).filter(|r| r.id == item::DYE);
        let damage = match mixed {
            Some(r) => i32::from(r.damage),
            None if self.world_rand.next_boolean() => i32::from(dye_damage(a)),
            None => i32::from(dye_damage(b)),
        };
        Some(color_by_dye_damage(damage))
    }

    /// `EntityAIEatGrass.shouldExecute` after its chance: tall grass where the mob
    /// stands, or a grass block below it.
    pub(super) fn grass_to_eat(&mut self, id: i32) -> bool {
        let Some(m) = self.mobs.get_mut(&id) else {
            return false;
        };
        let chance = if m.child() {
            EAT_CHANCE_CHILD
        } else {
            EAT_CHANCE_ADULT
        };
        if m.rand.next_int_bounded(chance) != 0 {
            return false;
        }
        let feet = block_pos(m.body.pos);
        let w = &mut self.level.overworld;
        w.block(feet.0, feet.1, feet.2) == tall_grass()
            || w.block(feet.0, feet.1 - 1, feet.2) >> 4 == block::GRASS
    }

    /// `EntityAIEatGrass.startExecuting`, apart from the timer and the path: the head
    /// goes down for everyone watching.
    pub(super) fn start_eating_grass(&mut self, id: i32) {
        self.send_to_tracking(
            id,
            &ClientboundPlay::EntityStatus {
                entity_id: id,
                status: STATUS_EAT_GRASS,
            },
        );
    }

    /// `EntityAIEatGrass.updateTask` at the bite: tall grass where the mob stands is
    /// broken (no drops), else a grass block below turns into dirt (both only with
    /// `mobGriefing`), and the mob gets `eatGrassBonus` either way.
    pub(super) fn eat_grass(&mut self, id: i32) {
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        let feet = block_pos(m.body.pos);
        let griefing = self.level.data.game_rule(MOB_GRIEFING) != Some("false");
        let here = self.level.overworld.block(feet.0, feet.1, feet.2);
        if here == tall_grass() {
            if griefing {
                // World.destroyBlock without drops.
                let state_id = i32::from(here >> 4) | i32::from(here & 15) << 12;
                self.block_effect_data(BREAK_EFFECT, feet, state_id);
                self.set_block_notify(feet, block::AIR << 4);
            }
        } else {
            let below = (feet.0, feet.1 - 1, feet.2);
            if self.level.overworld.block(below.0, below.1, below.2) >> 4 != block::GRASS {
                return;
            }
            if griefing {
                self.block_effect_data(BREAK_EFFECT, below, i32::from(block::GRASS));
                self.set_block_flags(below, block::DIRT << 4, flags::SEND);
            }
        }
        self.eat_grass_bonus(id);
    }

    /// `EntitySheep.eatGrassBonus`: the wool grows back and a child grows by a minute.
    fn eat_grass_bonus(&mut self, id: i32) {
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        if m.kind != MobKind::Sheep {
            return;
        }
        m.set_sheared(false);
        if m.child()
            && let Some(d) = m.add_growth(EAT_GROWTH_SECONDS, false)
        {
            self.move_mob_by(id, (d, 0.0, d));
        }
    }
}

/// `new BlockPos(posX, posY, posZ)`.
fn block_pos(pos: (f64, f64, f64)) -> (i32, i32, i32) {
    use krol_java::math::floor_f64;
    (floor_f64(pos.0), floor_f64(pos.1), floor_f64(pos.2))
}

/// The block state of tall grass (not a fern or dead bush).
fn tall_grass() -> u16 {
    block::TALLGRASS << 4 | TALL_GRASS_META
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dye_damage_and_colors_mirror_each_other() {
        // Bone meal (15) is white, ink (0) black, rose red (1) red (14).
        assert_eq!(color_by_dye_damage(15), WHITE);
        assert_eq!(color_by_dye_damage(0), BLACK);
        assert_eq!(color_by_dye_damage(1), 14);
        assert_eq!(color_by_dye_damage(16), BLACK);
        assert_eq!(dye_damage(14), 1);
        assert_eq!(color_by_meta(20), WHITE);
    }

    #[test]
    fn the_fleece_byte_holds_color_and_sheared_bit() {
        let mut sheep = Mob::new(MobKind::Sheep, (0.0, 64.0, 0.0), 0.0, 1);
        sheep.dirty_metadata.clear();
        sheep.set_fleece_color(14);
        sheep.set_sheared(true);
        assert_eq!(sheep.fleece, 14 | SHEARED_BIT);
        assert!(sheep.dirty_metadata.contains(&FLEECE_INDEX));
        sheep.set_sheared(false);
        assert_eq!((sheep.fleece_color(), sheep.sheared()), (14, false));
    }

    #[test]
    fn sheep_save_and_load_their_fleece() {
        let mut sheep = Mob::new(MobKind::Sheep, (0.0, 64.0, 0.0), 0.0, 1);
        sheep.set_fleece_color(5);
        sheep.set_sheared(true);
        let mut c = krol_nbt::Compound::new();
        sheep.write(&mut c);
        assert_eq!(c.get_i8("Color"), 5);
        assert!(c.get_bool("Sheared"));
        let loaded = Mob::load(
            MobKind::Sheep,
            sheep.body.clone(),
            sheep.base.clone(),
            &c,
            0.0,
            1,
        );
        assert_eq!(loaded.fleece, 5 | SHEARED_BIT);
        assert_eq!(MobKind::from_type_id(91), Some(MobKind::Sheep));
        assert_eq!(MobKind::from_saved_id("Sheep"), Some(MobKind::Sheep));
    }

    #[test]
    fn white_and_red_dyes_make_pink_lambs() {
        let dye = |color| {
            Some(ItemStack {
                id: item::DYE,
                count: 1,
                damage: dye_damage(color),
                nbt: None,
            })
        };
        let grid = [dye(WHITE), dye(14), None, None];
        let r = crate::game::recipes::craft(&grid, 2, &|_| None);
        assert_eq!(
            r.map(|r| (r.id, color_by_dye_damage(i32::from(r.damage)))),
            Some((item::DYE, PINK))
        );
        let grid = [dye(WHITE), dye(WHITE), None, None];
        assert!(crate::game::recipes::craft(&grid, 2, &|_| None).is_none());
    }
}
