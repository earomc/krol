//! What blocks drop (vanilla: `Block.dropBlockAsItemWithChance`, `quantityDroppedWithBonus`,
//! `quantityDropped`, `getItemDropped`, `damageDropped`, `canSilkHarvest`,
//! `createStackedBlock` and their overrides in the block classes).
//!
//! Every roll is made on the world random in vanilla's order: the quantity first (with
//! the fortune bonus), then per drop a chance float and the item. Blocks whose quantity
//! is 1 but drop nothing still roll the chance float.
//!
//! TODO(M5): pistons' moving blocks (`BlockPistonMoving` drops the block it carries),
//! skulls and jukebox records.

use krol_protocol::packets::play::ItemStack;

use super::Game;
use super::ids::{block, item};

/// Items blocks drop that have no constant elsewhere.
const WHEAT: i16 = 296;
const CARROT: i16 = 391;
const POTATO: i16 = 392;
const POISONOUS_POTATO: i16 = 394;
const PUMPKIN_SEEDS: i16 = 361;
const MELON_SEEDS: i16 = 362;
const NETHER_WART_ITEM: i16 = 372;
const REEDS_ITEM: i16 = 338;
const STRING: i16 = 287;
const FLINT: i16 = 318;
const DIAMOND: i16 = 264;
const EMERALD: i16 = 388;
const QUARTZ: i16 = 406;
const DYE: i16 = 351;
const LAPIS_DAMAGE: i16 = 4;
const GLOWSTONE_DUST: i16 = 348;
const PRISMARINE_CRYSTALS: i16 = 410;
const MELON: i16 = 360;
const SNOWBALL: i16 = 332;

/// Blocks only this file needs by id.
const GLASS: u16 = 20;
const STAINED_GLASS: u16 = 95;
const IRON_BARS: u16 = 101;
const GLASS_PANE: u16 = 102;
const STAINED_GLASS_PANE: u16 = 160;
const SEA_LANTERN: u16 = 169;
const VINE: u16 = 106;
const END_PORTAL_FRAME: u16 = 120;
const COMMAND_BLOCK: u16 = 137;
const COBBLESTONE_WALL: u16 = 139;
const TRIPWIRE: u16 = 132;
const QUARTZ_BLOCK: u16 = 155;
const HAY_BLOCK: u16 = 170;
const MONSTER_EGG: u16 = 97;
const STONEBRICK: u16 = 98;
const DOUBLE_STONE_SLAB: u16 = 43;
const DOUBLE_WOODEN_SLAB: u16 = 125;
const DOUBLE_STONE_SLAB2: u16 = 181;

/// Fully grown crops (`BlockCrops` age 7).
const CROP_MAX_AGE: u16 = 7;
/// Ripe nether wart (`BlockNetherWart` age 3).
const WART_MAX_AGE: u16 = 3;
/// Crops and stems drop a seed per roll of `nextInt(15)` at most their age.
const SEED_ROLL: i32 = 15;
/// Crops roll this many extra seeds (plus the fortune level), stems this many.
const CROP_SEED_ROLLS: i32 = 3;
const STEM_SEED_ROLLS: i32 = 3;
/// Ripe potatoes drop a poisonous potato with chance 1 in this.
const POISONOUS_ODDS: i32 = 50;
/// Gravel drops flint with chance 1 in this, minus 3 per fortune level up to 3.
const FLINT_ODDS: i32 = 10;
const FLINT_FORTUNE_STEP: i32 = 3;
const FLINT_FORTUNE_MAX: i32 = 3;
/// Tall grass drops wheat seeds with chance 1 in this.
const SEED_ODDS: i32 = 8;

/// What a block drops: `quantityDroppedWithBonus` rolled up front, the item decided per
/// drop ([`pick_drop`]).
struct DropPlan {
    block: u16,
    meta: u16,
    count: i32,
    item: Option<i16>,
    damage: i16,
}

/// `BlockOre.quantityDroppedWithBonus`: with fortune, ores dropping an item drop up to
/// `fortune + 1` times as much.
fn ore_multiplier(rand: &mut krol_java::JavaRandom, fortune: i32) -> i32 {
    if fortune > 0 {
        (rand.next_int_bounded(fortune + 2) - 1).max(0) + 1
    } else {
        1
    }
}

fn drop_plan(rand: &mut krol_java::JavaRandom, state: u16, fortune: i32) -> DropPlan {
    let (b, m) = (state >> 4, (state & 15) as i16);
    // (quantity, item, damage); a quantity of 0 drops nothing and rolls nothing.
    let (count, item, damage): (i32, Option<i16>, i16) = match b {
        1 => (1, Some(if m == 0 { 4 } else { 1 }), m),
        2 | 60 | 110 => (1, Some(3), 0),
        3 => (1, Some(3), if m == 2 { 0 } else { m }),
        5 | 12 | 19 | 24 | 35 | 38 | 98 | 159 | 168 | 171 | 179 | COBBLESTONE_WALL => {
            (1, Some(b as i16), m)
        }
        6 => (1, Some(6), m & 7),
        17 | 162 => (1, Some(b as i16), m & 3),
        44 | 126 | 182 => (1, Some(b as i16), m & 7),
        DOUBLE_STONE_SLAB => (2, Some(44), m & 7),
        DOUBLE_WOODEN_SLAB => (2, Some(126), m & 7),
        DOUBLE_STONE_SLAB2 => (2, Some(182), m & 7),
        QUARTZ_BLOCK => (
            1,
            Some(155),
            if m == 1 {
                1
            } else if m >= 2 {
                2
            } else {
                0
            },
        ),
        // BlockAnvil: the damage, not the facing.
        block::ANVIL => (1, Some(block::ANVIL as i16), m >> 2),
        // Decided per drop below.
        block::GRAVEL | super::plants::DOUBLE_PLANT => (1, None, 0),
        // BlockTallGrass.quantityDroppedWithBonus rolls even without fortune.
        block::TALLGRASS => (1 + rand.next_int_bounded(fortune * 2 + 1), None, 0),
        // BlockHugeMushroom: up to 2 small mushrooms.
        99 | 100 => (
            (rand.next_int_bounded(10) - 7).max(0),
            Some(b as i16 - 60),
            0,
        ),
        block::COAL_ORE => (ore_multiplier(rand, fortune), Some(item::COAL), 0),
        block::DIAMOND_ORE => (ore_multiplier(rand, fortune), Some(DIAMOND), 0),
        block::EMERALD_ORE => (ore_multiplier(rand, fortune), Some(EMERALD), 0),
        block::QUARTZ_ORE => (ore_multiplier(rand, fortune), Some(QUARTZ), 0),
        block::LAPIS_ORE => {
            let times = ore_multiplier(rand, fortune);
            (
                (4 + rand.next_int_bounded(5)) * times,
                Some(DYE),
                LAPIS_DAMAGE,
            )
        }
        block::REDSTONE_ORE | block::LIT_REDSTONE_ORE => (
            4 + rand.next_int_bounded(2) + rand.next_int_bounded(fortune + 1),
            Some(item::REDSTONE),
            0,
        ),
        block::GLOWSTONE => (
            (2 + rand.next_int_bounded(3) + rand.next_int_bounded(fortune + 1)).clamp(1, 4),
            Some(GLOWSTONE_DUST),
            0,
        ),
        SEA_LANTERN => (
            (2 + rand.next_int_bounded(2) + rand.next_int_bounded(fortune + 1)).clamp(1, 5),
            Some(PRISMARINE_CRYSTALS),
            0,
        ),
        block::MELON_BLOCK => (
            (3 + rand.next_int_bounded(5) + rand.next_int_bounded(1 + fortune)).min(9),
            Some(MELON),
            0,
        ),
        47 => (3, Some(340), 0),
        82 => (4, Some(337), 0),
        block::SNOW => (4, Some(SNOWBALL), 0),
        block::LIT_FURNACE => (1, Some(block::FURNACE as i16), 0),
        block::ENDER_CHEST => (8, Some(block::OBSIDIAN as i16), 0),
        // Redstone: wire drops dust, unlit torches and lit lamps their normal item.
        block::REDSTONE_WIRE => (1, Some(item::REDSTONE), 0),
        block::UNLIT_REDSTONE_TORCH => (1, Some(block::REDSTONE_TORCH as i16), 0),
        block::LIT_REDSTONE_LAMP => (1, Some(block::REDSTONE_LAMP as i16), 0),
        block::UNPOWERED_REPEATER | block::POWERED_REPEATER => (1, Some(item::REPEATER), 0),
        block::UNPOWERED_COMPARATOR | block::POWERED_COMPARATOR => (1, Some(item::COMPARATOR), 0),
        block::DAYLIGHT_DETECTOR_INVERTED => (1, Some(block::DAYLIGHT_DETECTOR as i16), 0),
        // Doors drop from their lower half only (the upper half still rolls).
        64 | 71 | 193..=197 if m & 8 != 0 => (1, None, 0),
        64 => (1, Some(324), 0),
        71 => (1, Some(330), 0),
        193..=197 => (1, Some(427 + b as i16 - 193), 0),
        block::STANDING_SIGN | block::WALL_SIGN => (1, Some(item::SIGN), 0),
        block::REEDS => (1, Some(REEDS_ITEM), 0),
        block::WEB | TRIPWIRE => (1, Some(STRING), 0),
        // Crops: the crop when ripe, else the seed; the extra seeds come after.
        block::WHEAT => (
            1,
            Some(if m as u16 == CROP_MAX_AGE {
                WHEAT
            } else {
                item::WHEAT_SEEDS
            }),
            0,
        ),
        block::CARROTS => (1, Some(CARROT), 0),
        block::POTATOES => (1, Some(POTATO), 0),
        // Nothing to drop, but a quantity of 1: the chance is still rolled.
        block::DEADBUSH
        | GLASS_PANE
        | STAINED_GLASS_PANE
        | END_PORTAL_FRAME
        | block::PUMPKIN_STEM
        | block::MELON_STEM => (1, None, 0),
        // A quantity of 0.
        GLASS
        | STAINED_GLASS
        | block::ICE
        | block::PACKED_ICE
        | block::SNOW_LAYER
        | VINE
        | COMMAND_BLOCK
        | 52
        | MONSTER_EGG
        | block::NETHER_WART
        | block::BARRIER
        | block::LEAVES
        | block::LEAVES2 => (0, None, 0),
        super::brewing::BREWING_STAND => (1, Some(super::brewing::BREWING_STAND_ITEM), 0),
        super::flower_pot::FLOWER_POT => (1, Some(super::flower_pot::FLOWER_POT_ITEM), 0),
        super::cauldron::CAULDRON => (1, Some(super::cauldron::CAULDRON_ITEM), 0),
        super::beds::BED if super::beds::drops_item(m as u16) => {
            (1, Some(super::beds::BED_ITEM), 0)
        }
        super::beds::BED => (0, None, 0),
        _ => {
            if krol_world::items::name_for_id(b as i16).is_some() {
                (1, Some(b as i16), 0)
            } else {
                (0, None, 0)
            }
        }
    };
    DropPlan {
        block: b,
        meta: m as u16,
        count,
        item,
        damage,
    }
}

/// `getItemDropped` / `damageDropped` for one drop of `plan`.
fn pick_drop(rand: &mut krol_java::JavaRandom, plan: &DropPlan, fortune: i32) -> Option<ItemStack> {
    let item = match plan.block {
        block::GRAVEL => {
            let odds = FLINT_ODDS - fortune.min(FLINT_FORTUNE_MAX) * FLINT_FORTUNE_STEP;
            Some(if rand.next_int_bounded(odds) == 0 {
                FLINT
            } else {
                block::GRAVEL as i16
            })
        }
        block::TALLGRASS => (rand.next_int_bounded(SEED_ODDS) == 0).then_some(item::WHEAT_SEEDS),
        super::plants::DOUBLE_PLANT => {
            return super::plants::double_plant_drop(rand, plan.meta)
                .map(|(id, damage)| stack(id, damage));
        }
        _ => plan.item,
    };
    item.map(|id| stack(id, plan.damage))
}

fn stack(id: i16, damage: i16) -> ItemStack {
    ItemStack {
        id,
        count: 1,
        damage,
        nbt: None,
    }
}

/// `Block.canSilkHarvest`: full cubes without a tile entity, plus glass, panes, ice, sea
/// lanterns, cobwebs and ender chests; never slabs.
pub fn can_silk_harvest(b: u16) -> bool {
    match b {
        block::ENDER_CHEST
        | GLASS
        | STAINED_GLASS
        | IRON_BARS
        | GLASS_PANE
        | STAINED_GLASS_PANE
        | block::ICE
        | SEA_LANTERN
        | block::WEB => true,
        DOUBLE_STONE_SLAB | DOUBLE_WOODEN_SLAB | DOUBLE_STONE_SLAB2 => false,
        _ => super::light_data::full_cube(b) && !super::world::has_tile_entity(b),
    }
}

/// `Block.createStackedBlock`: what a block mined with silk touch drops, the block
/// itself (with its metadata if the item has subtypes).
pub fn stacked_block(state: u16) -> Option<ItemStack> {
    let (b, m) = (state >> 4, (state & 15) as i16);
    let (id, damage) = match b {
        block::LEAVES | block::LEAVES2 | block::LOG | block::LOG2 => (b as i16, m & 3),
        // BlockQuartz: sideways pillars drop as upright ones.
        QUARTZ_BLOCK => (b as i16, m.min(2)),
        HAY_BLOCK => (b as i16, 0),
        block::LIT_REDSTONE_LAMP => (block::REDSTONE_LAMP as i16, 0),
        block::LIT_REDSTONE_ORE => (block::REDSTONE_ORE as i16, 0),
        // BlockSilverfish: the block it imitates.
        MONSTER_EGG => match m {
            1 => (block::COBBLESTONE as i16, 0),
            2..=5 => (STONEBRICK as i16, m - 2),
            _ => (block::STONE as i16, 0),
        },
        _ => {
            krol_world::items::name_for_id(b as i16)?;
            let subtypes = super::item_data::has_subtypes(b as i16);
            (b as i16, if subtypes { m } else { 0 })
        }
    };
    Some(stack(id, damage))
}

impl Game {
    /// Drops what the block leaves when harvested (`dropBlockAsItem`), unless the
    /// `doTileDrops` game rule is off.
    pub(super) fn drop_block_items(&mut self, pos: (i32, i32, i32), state: u16) {
        self.drop_block_items_with_chance(pos, state, 1.0, 0);
    }

    /// `dropBlockAsItemWithChance`: each drop happens if a random float is at most
    /// `chance` (explosions drop with `1 / strength`); every roll is made even at 1.
    pub(super) fn drop_block_items_with_chance(
        &mut self,
        pos: (i32, i32, i32),
        state: u16,
        chance: f32,
        fortune: i32,
    ) {
        let (b, m) = (state >> 4, state & 15);
        if super::leaves::is_leaves(b) {
            self.drop_leaves(pos, state, fortune);
            return;
        }
        if self.drop_banner(pos, state) {
            return;
        }
        match b {
            block::COCOA => {
                for _ in 0..super::farming::cocoa_drops(m) {
                    self.spawn_as_entity(pos, super::farming::cocoa_beans());
                }
                return;
            }
            // BlockNetherWart: 1, or 2 to 4 when ripe (more with fortune).
            block::NETHER_WART => {
                let mut count = 1;
                if m >= WART_MAX_AGE {
                    count = 2 + self.world_rand.next_int_bounded(3);
                    if fortune > 0 {
                        count += self.world_rand.next_int_bounded(fortune + 1);
                    }
                }
                for _ in 0..count {
                    self.spawn_as_entity(pos, stack(NETHER_WART_ITEM, 0));
                }
                return;
            }
            _ => {}
        }
        // BlockCrops passes no fortune to the plain drop.
        let crop = matches!(b, block::WHEAT | block::CARROTS | block::POTATOES);
        let plan = drop_plan(&mut self.world_rand, state, if crop { 0 } else { fortune });
        for _ in 0..plan.count {
            if self.world_rand.next_float() > chance {
                continue;
            }
            if let Some(stack) = pick_drop(&mut self.world_rand, &plan, fortune) {
                self.spawn_as_entity(pos, stack);
            }
        }
        match b {
            // BlockCrops: ripe crops roll for extra seeds.
            _ if crop && m >= CROP_MAX_AGE => {
                let seed = match b {
                    block::WHEAT => item::WHEAT_SEEDS,
                    block::CARROTS => CARROT,
                    _ => POTATO,
                };
                for _ in 0..CROP_SEED_ROLLS + fortune {
                    if self.world_rand.next_int_bounded(SEED_ROLL) <= i32::from(m) {
                        self.spawn_as_entity(pos, stack(seed, 0));
                    }
                }
                // BlockPotato: sometimes a poisonous one.
                if b == block::POTATOES && self.world_rand.next_int_bounded(POISONOUS_ODDS) == 0 {
                    self.spawn_as_entity(pos, stack(POISONOUS_POTATO, 0));
                }
            }
            // BlockStem: seeds by age.
            block::PUMPKIN_STEM | block::MELON_STEM => {
                let seed = if b == block::PUMPKIN_STEM {
                    PUMPKIN_SEEDS
                } else {
                    MELON_SEEDS
                };
                for _ in 0..STEM_SEED_ROLLS {
                    if self.world_rand.next_int_bounded(SEED_ROLL) <= i32::from(m) {
                        self.spawn_as_entity(pos, stack(seed, 0));
                    }
                }
            }
            _ => {}
        }
        // BlockOre / BlockRedstoneOre / BlockMobSpawner.dropBlockAsItemWithChance: ores
        // that drop an item rather than themselves, and spawners, also drop experience.
        self.drop_block_xp(pos, b);
    }

    /// `Block.harvestBlock` past the block-specific cases: the block itself with silk
    /// touch where it can be, otherwise its drops with the held item's fortune. Returns
    /// whether silk touch took the block.
    pub(super) fn harvest_drops(&mut self, i: usize, pos: (i32, i32, i32), state: u16) -> bool {
        let held = self.players[i].held().cloned();
        let silk =
            super::item_data::enchantment_level(held.as_ref(), super::item_data::SILK_TOUCH) > 0;
        if silk && can_silk_harvest(state >> 4) {
            if let Some(stack) = stacked_block(state) {
                self.spawn_as_entity(pos, stack);
            }
            return true;
        }
        let fortune = super::item_data::enchantment_level(held.as_ref(), super::item_data::FORTUNE);
        self.drop_block_items_with_chance(pos, state, 1.0, fortune);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silk_touch_rules() {
        assert!(can_silk_harvest(block::STONE));
        assert!(can_silk_harvest(GLASS));
        assert!(can_silk_harvest(block::ENDER_CHEST));
        assert!(!can_silk_harvest(block::CHEST));
        assert!(!can_silk_harvest(DOUBLE_STONE_SLAB));
        assert!(!can_silk_harvest(block::TORCH));
        let s = stacked_block(QUARTZ_BLOCK << 4 | 4);
        assert_eq!(s.map(|s| (s.id, s.damage)), Some((155, 2)));
        let s = stacked_block(MONSTER_EGG << 4 | 3);
        assert_eq!(s.map(|s| (s.id, s.damage)), Some((98, 1)));
        let s = stacked_block(block::LOG2 << 4 | 0b1001);
        assert_eq!(s.map(|s| (s.id, s.damage)), Some((162, 1)));
        let s = stacked_block(block::LIT_REDSTONE_ORE << 4);
        assert_eq!(s.map(|s| s.id), Some(73));
    }

    #[test]
    fn fortune_quantities() {
        // Without fortune, ores drop one item and roll nothing for the quantity.
        let mut r = krol_java::JavaRandom::new(1);
        let mut r2 = krol_java::JavaRandom::new(1);
        let plan = drop_plan(&mut r, block::DIAMOND_ORE << 4, 0);
        assert_eq!(plan.count, 1);
        assert_eq!(r.next_int(), r2.next_int());
        // Fortune III: 1 to 4 diamonds.
        for seed in 0..50 {
            let mut r = krol_java::JavaRandom::new(seed);
            let plan = drop_plan(&mut r, block::DIAMOND_ORE << 4, 3);
            assert!((1..=4).contains(&plan.count));
        }
        // Tall grass rolls its quantity even without fortune.
        let mut r = krol_java::JavaRandom::new(1);
        let mut r2 = krol_java::JavaRandom::new(1);
        assert_eq!(drop_plan(&mut r, block::TALLGRASS << 4 | 1, 0).count, 1);
        r2.next_int();
        assert_eq!(r.next_int(), r2.next_int());
    }
}
