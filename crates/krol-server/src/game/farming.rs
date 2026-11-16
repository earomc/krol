//! Hoes and bone meal (vanilla `ItemHoe`, `ItemDye.applyBonemeal` and the `IGrowable`
//! blocks krol models).
//! Cocoa beans (vanilla `BlockCocoa` and `ItemDye.onItemUse`) live here too.

use super::Game;
use super::block_data::{Material, material};
use super::block_update::flags;
use super::ids::block;

/// `cake`.
pub const CAKE: u16 = 92;
/// A cake has 7 slices (bites 0 to 6).
const LAST_BITE: u16 = 6;

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

/// Dye damage of cocoa beans (brown).
const COCOA_BEANS_DAMAGE: i16 = 3;
/// `log` variant metadata of jungle wood.
const JUNGLE: u16 = 3;
/// Cocoa metadata: the horizontal index of the side its log is on, then the age.
const COCOA_FACING: u16 = 3;
const COCOA_AGE_SHIFT: u16 = 2;
const COCOA_MAX_AGE: u16 = 2;
/// Cocoa grows 1 random tick in 5.
const COCOA_GROW_ODDS: i32 = 5;
/// Ripe cocoa drops 3 beans, unripe 1.
const RIPE_COCOA_BEANS: usize = 3;

/// Whether the held item is cocoa beans.
pub fn is_cocoa_beans(id: i16, damage: i16) -> bool {
    id == DYE && damage == COCOA_BEANS_DAMAGE
}

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

    /// `BlockCake.eatCake`: a hungry player (not in creative) eats a slice: 2 food,
    /// saturation 0.1, the statistic; the last slice removes the cake.
    pub(super) fn eat_cake(&mut self, i: usize, pos: (i32, i32, i32)) {
        let p = &self.players[i];
        if p.data.get_i32("foodLevel") >= 20 || matches!(p.gamemode, 1 | 3) {
            return;
        }
        self.add_stat(i, "stat.cakeSlicesEaten", 1);
        self.add_food(i, 2, 0.1);
        let state = self.level.overworld.block(pos.0, pos.1, pos.2);
        if state & 15 < LAST_BITE {
            self.set_block_notify(pos, state + 1);
        } else {
            self.set_block_notify(pos, 0);
        }
    }

    /// `BlockCake.onNeighborBlockChange`: without something solid below it disappears.
    pub(super) fn cake_neighbor_changed(&mut self, pos: (i32, i32, i32)) {
        let below = self.level.overworld.block(pos.0, pos.1 - 1, pos.2);
        if !material(below >> 4).is_solid() {
            self.set_block_notify(pos, 0);
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
            _ if super::mushrooms::is_mushroom(id) => true,
            block::COCOA => age >> COCOA_AGE_SHIFT < COCOA_MAX_AGE,
            _ => self.plant_can_grow(pos, state).unwrap_or(false),
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
            _ if super::mushrooms::is_mushroom(id) => self.bone_meal_mushroom(pos, state),
            // BlockCocoa.grow: one stage.
            block::COCOA => {
                let grown = state + (1 << COCOA_AGE_SHIFT);
                self.set_block_flags(pos, grown, flags::SEND);
            }
            block::GRASS | block::TALLGRASS | super::plants::DOUBLE_PLANT => {
                self.grow_plant(pos, state);
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

    /// `ItemDye.onItemUse` with cocoa beans: on the side of a jungle log with air in
    /// front, a cocoa pod facing the log.
    pub(super) fn place_cocoa(&mut self, i: usize, pos: (i32, i32, i32), face: u8) -> bool {
        let clicked = self.level.overworld.block(pos.0, pos.1, pos.2);
        if clicked >> 4 != block::LOG || clicked & 3 != JUNGLE {
            return false;
        }
        if face == super::placement::DOWN || face == super::placement::UP {
            return false;
        }
        let at = super::placement::offset(pos, face);
        // Vanilla quirk: a blocked spot still counts as using the item.
        if self.level.overworld.block(at.0, at.1, at.2) >> 4 != block::AIR {
            self.count_item_use(i);
            return true;
        }
        let facing = super::placement::horizontal_index(super::placement::opposite(face));
        self.set_block_flags(at, block::COCOA << 4 | u16::from(facing), flags::SEND);
        self.uses_up_one(i);
        self.count_item_use(i);
        true
    }

    /// `BlockCocoa.canBlockStay`: the log it faces is jungle wood.
    fn cocoa_stays(&mut self, pos: (i32, i32, i32), state: u16) -> bool {
        let dir = match state & COCOA_FACING {
            0 => super::placement::SOUTH,
            1 => super::placement::WEST,
            2 => super::placement::NORTH,
            _ => super::placement::EAST,
        };
        let log = super::placement::offset(pos, dir);
        let s = self.level.overworld.block(log.0, log.1, log.2);
        s >> 4 == block::LOG && s & 3 == JUNGLE
    }

    /// `BlockCocoa.dropBlock`.
    fn drop_cocoa(&mut self, pos: (i32, i32, i32), state: u16) {
        self.set_block_notify(pos, 0);
        self.drop_block_items(pos, state);
    }

    /// `BlockCocoa.onNeighborBlockChange`.
    pub(super) fn cocoa_neighbor_changed(&mut self, pos: (i32, i32, i32), state: u16) {
        if !self.cocoa_stays(pos, state) {
            self.drop_cocoa(pos, state);
        }
    }

    /// `BlockCocoa.updateTick`: drops without its log, else grows 1 time in 5.
    pub(super) fn cocoa_tick(&mut self, pos: (i32, i32, i32), state: u16) {
        if !self.cocoa_stays(pos, state) {
            self.drop_cocoa(pos, state);
        } else if self.world_rand.next_int_bounded(COCOA_GROW_ODDS) == 0
            && (state & 15) >> COCOA_AGE_SHIFT < COCOA_MAX_AGE
        {
            self.set_block_flags(pos, state + (1 << COCOA_AGE_SHIFT), flags::SEND);
        }
    }
}

/// `BlockCocoa.dropBlockAsItemWithChance`: 3 beans when ripe, else 1 (no chance roll).
pub fn cocoa_drops(meta: u16) -> usize {
    if meta >> COCOA_AGE_SHIFT >= COCOA_MAX_AGE {
        RIPE_COCOA_BEANS
    } else {
        1
    }
}

/// The cocoa bean stack.
pub fn cocoa_beans() -> krol_protocol::packets::play::ItemStack {
    krol_protocol::packets::play::ItemStack {
        id: DYE,
        count: 1,
        damage: COCOA_BEANS_DAMAGE,
        nbt: None,
    }
}
