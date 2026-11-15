//! Hoes and bone meal (vanilla `ItemHoe`, `ItemDye.applyBonemeal` and the `IGrowable`
//! blocks krol models).
//!
//! TODO(M5): bone meal on grass (flowers and tall grass by biome), tall grass and
//! double plants, mushrooms (huge mushrooms) and cocoa.

use super::Game;
use super::block_data::{Material, material};
use super::block_update::flags;
use super::ids::block;

/// The hoes (wood, stone, iron, diamond, gold).
pub fn is_hoe(id: i16) -> bool {
    (290..=294).contains(&id)
}

/// `dye`; damage 15 is bone meal.
const DYE: i16 = 351;
const BONE_MEAL_DAMAGE: i16 = 15;
/// `dirt` variant metadata: coarse dirt.
const COARSE_DIRT: u16 = 1;
/// Crops and stems are fully grown at age 7.
const MAX_AGE: u16 = 7;
/// Bone meal grows a sapling 45% of the time.
const SAPLING_BONE_MEAL_CHANCE: f64 = 0.45;
/// The `Effect` id of bone meal particles.
const BONE_MEAL_EFFECT: i32 = 2005;

/// Whether the held item is bone meal.
pub fn is_bone_meal(id: i16, damage: i16) -> bool {
    id == DYE && damage == BONE_MEAL_DAMAGE
}

impl Game {
    fn uses_up_one(&mut self, i: usize) {
        let p = &mut self.players[i];
        if p.gamemode != 1 {
            let held = 36 + p.selected;
            if let Some(s) = &mut p.inventory[held] {
                s.count -= 1;
            }
        }
    }

    /// `ItemHoe.onItemUse`: grass and dirt with air above turn into farmland (coarse dirt
    /// into dirt), with the new block's step sound; the hoe wears.
    pub(super) fn use_hoe(&mut self, i: usize, pos: (i32, i32, i32), face: u8) -> bool {
        if face == super::placement::DOWN {
            return false;
        }
        let w = &mut self.level.overworld;
        let state = w.block(pos.0, pos.1, pos.2);
        if material(w.block(pos.0, pos.1 + 1, pos.2) >> 4) != Material::Air {
            return false;
        }
        let tilled = match state >> 4 {
            block::GRASS => block::FARMLAND << 4,
            block::DIRT if state & 15 == 0 => block::FARMLAND << 4,
            block::DIRT if state & 15 == COARSE_DIRT => block::DIRT << 4,
            _ => return false,
        };
        let sound = super::blocks::sound_type(tilled >> 4);
        let center = (
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 0.5,
            f64::from(pos.2) + 0.5,
        );
        self.play_sound_near_except(
            None,
            &sound.step_sound(),
            center,
            (sound.volume + 1.0) / 2.0,
            sound.frequency * 0.8,
        );
        self.set_block_notify(pos, tilled);
        self.damage_held_item(i, 1);
        self.count_item_use(i);
        let p = &mut self.players[i];
        let held = 36 + p.selected;
        if p.inventory[held].as_ref().is_some_and(|s| s.count == 0) {
            p.inventory[held] = None;
        }
        true
    }

    /// `ItemDye.applyBonemeal` + the particles: a growable block that can grow takes the
    /// bone meal (whether or not it grows this time).
    pub(super) fn use_bone_meal(&mut self, i: usize, pos: (i32, i32, i32)) -> bool {
        let state = self.level.overworld.block(pos.0, pos.1, pos.2);
        let id = state >> 4;
        let age = state & 15;
        let grows = match id {
            // IGrowable.canGrow.
            block::WHEAT | block::CARROTS | block::POTATOES => age < MAX_AGE,
            block::PUMPKIN_STEM | block::MELON_STEM => age != MAX_AGE,
            block::SAPLING => true,
            _ => false,
        };
        if !grows {
            return false;
        }
        match id {
            block::SAPLING => {
                // canUseBonemeal, then grow.
                if f64::from(self.world_rand.next_float()) < SAPLING_BONE_MEAL_CHANCE {
                    self.grow_sapling(pos, state);
                }
            }
            _ => {
                // BlockCrops.grow / BlockStem.growStem: 2 to 5 stages more.
                let more = 2 + self.world_rand.next_int_bounded(4) as u16;
                let new = (age + more).min(MAX_AGE);
                self.set_block_flags(pos, id << 4 | new, flags::SEND);
            }
        }
        self.uses_up_one(i);
        self.block_effect_data(BONE_MEAL_EFFECT, pos, 0);
        self.count_item_use(i);
        true
    }
}
