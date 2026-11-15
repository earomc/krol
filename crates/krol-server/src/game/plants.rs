//! Grass-like plants (vanilla `BlockGrass.grow`, `BlockTallGrass`, `BlockDoublePlant`
//! and the biomes' `pickRandomFlower`).

use std::sync::OnceLock;

use krol_protocol::packets::play::ItemStack;

use super::Game;
use super::block_data::{Material, material};
use super::block_update::flags;
use super::ids::block;
use super::noise::OctaveNoise;

/// `double_plant`.
pub const DOUBLE_PLANT: u16 = 175;
/// The upper half bit of double plant metadata.
const UPPER: u16 = 8;
/// An upper half as vanilla stores it: [`UPPER`] plus the horizontal index of its
/// (unused) facing, north.
const UPPER_STATE: u16 = UPPER | 2;

/// `BlockDoublePlant.EnumPlantType` metadata.
const SUNFLOWER: u16 = 0;
const DOUBLE_GRASS: u16 = 2;
const LARGE_FERN: u16 = 3;
/// `BlockTallGrass.EnumType` metadata.
const DEAD_BUSH: u16 = 0;
const GRASS: u16 = 1;
const FERN: u16 = 2;

/// `BlockFlower.EnumFlowerType` in declaration order: `(block, metadata)`.
const FLOWERS: [(u16, u16); 10] = [
    (block::YELLOW_FLOWER, 0), // dandelion
    (block::RED_FLOWER, 0),    // poppy
    (block::RED_FLOWER, 1),    // blue orchid
    (block::RED_FLOWER, 2),    // allium
    (block::RED_FLOWER, 3),    // houstonia
    (block::RED_FLOWER, 4),    // red tulip
    (block::RED_FLOWER, 5),    // orange tulip
    (block::RED_FLOWER, 6),    // white tulip
    (block::RED_FLOWER, 7),    // pink tulip
    (block::RED_FLOWER, 8),    // oxeye daisy
];
const DANDELION: usize = 0;
const POPPY: usize = 1;
const BLUE_ORCHID: usize = 2;
const HOUSTONIA: usize = 4;
const RED_TULIP: usize = 5;
const ORANGE_TULIP: usize = 6;
const WHITE_TULIP: usize = 7;
const PINK_TULIP: usize = 8;
const OXEYE_DAISY: usize = 9;

/// Biomes with their own flowers.
const PLAINS: u8 = 1;
const SWAMPLAND: u8 = 6;
const FLOWER_FOREST: u8 = 132;
const SUNFLOWER_PLAINS: u8 = 129;
/// The seed of `BiomeGenBase.GRASS_COLOR_NOISE`.
const GRASS_COLOR_NOISE_SEED: i64 = 2345;
/// Bone meal on grass makes this many attempts.
const GRASS_ATTEMPTS: i32 = 128;
/// One attempt in 8 tries a flower instead of tall grass.
const FLOWER_ODDS: i32 = 8;

fn grass_color_noise() -> &'static OctaveNoise {
    static NOISE: OnceLock<OctaveNoise> = OnceLock::new();
    NOISE.get_or_init(|| {
        OctaveNoise::new(&mut krol_java::JavaRandom::new(GRASS_COLOR_NOISE_SEED), 1)
    })
}

/// `BiomeGenBase.pickRandomFlower` and its overrides (`BiomeGenPlains`,
/// `BiomeGenSwamp`, the flower forest): an index into [`FLOWERS`].
fn pick_flower(rand: &mut krol_java::JavaRandom, biome: u8, pos: (i32, i32, i32)) -> usize {
    let (x, z) = (f64::from(pos.0), f64::from(pos.2));
    match biome {
        PLAINS | SUNFLOWER_PLAINS => {
            if grass_color_noise().value(x / 200.0, z / 200.0) < -0.8 {
                match rand.next_int_bounded(4) {
                    0 => ORANGE_TULIP,
                    1 => RED_TULIP,
                    2 => PINK_TULIP,
                    _ => WHITE_TULIP,
                }
            } else if rand.next_int_bounded(3) > 0 {
                match rand.next_int_bounded(3) {
                    0 => POPPY,
                    1 => HOUSTONIA,
                    _ => OXEYE_DAISY,
                }
            } else {
                DANDELION
            }
        }
        SWAMPLAND => BLUE_ORCHID,
        FLOWER_FOREST => {
            let n =
                ((1.0 + grass_color_noise().value(x / 48.0, z / 48.0)) / 2.0).clamp(0.0, 0.9999);
            let flower = (n * FLOWERS.len() as f64) as usize;
            if flower == BLUE_ORCHID { POPPY } else { flower }
        }
        _ => {
            if rand.next_int_bounded(3) > 0 {
                DANDELION
            } else {
                POPPY
            }
        }
    }
}

/// `BlockBush.canPlaceBlockOn` of grass, flowers and double plants.
fn bush_soil(below: u16) -> bool {
    matches!(below, block::GRASS | block::DIRT | block::FARMLAND)
}

/// What a double plant drops (`getItemDropped`, `damageDropped`): nothing from the
/// upper half or a large fern, seeds 1 time in 8 from double grass, else itself.
pub fn double_plant_drop(rand: &mut krol_java::JavaRandom, meta: u16) -> Option<(i16, i16)> {
    if meta & UPPER != 0 || meta == LARGE_FERN {
        None
    } else if meta == DOUBLE_GRASS {
        (rand.next_int_bounded(8) == 0).then_some((super::ids::item::WHEAT_SEEDS, 0))
    } else {
        Some((DOUBLE_PLANT as i16, meta as i16))
    }
}

impl Game {
    fn plant_block(&mut self, p: (i32, i32, i32)) -> u16 {
        self.level.overworld.block(p.0, p.1, p.2)
    }

    /// `BlockDoublePlant.getVariant`: an upper half has the variant of the lower half
    /// below it (a sunflower without one).
    fn double_plant_variant(&mut self, pos: (i32, i32, i32), state: u16) -> u16 {
        if state & UPPER == 0 {
            return state & 7;
        }
        let below = self.plant_block((pos.0, pos.1 - 1, pos.2));
        if below >> 4 == DOUBLE_PLANT {
            below & 7
        } else {
            SUNFLOWER
        }
    }

    /// `IGrowable.canGrow` of grass, tall grass and double plants (`None`: another
    /// block).
    pub(super) fn plant_can_grow(&mut self, pos: (i32, i32, i32), state: u16) -> Option<bool> {
        Some(match state >> 4 {
            block::GRASS => true,
            block::TALLGRASS => state & 15 != DEAD_BUSH,
            DOUBLE_PLANT => !matches!(
                self.double_plant_variant(pos, state),
                DOUBLE_GRASS | LARGE_FERN
            ),
            _ => return None,
        })
    }

    /// `IGrowable.grow` of grass, tall grass and double plants.
    pub(super) fn grow_plant(&mut self, pos: (i32, i32, i32), state: u16) {
        match state >> 4 {
            block::GRASS => self.grow_grass(pos),
            block::TALLGRASS => {
                let variant = if state & 15 == FERN {
                    LARGE_FERN
                } else {
                    DOUBLE_GRASS
                };
                // BlockDoublePlant.canPlaceBlockAt: the tall grass is replaceable.
                let up = (pos.0, pos.1 + 1, pos.2);
                if bush_soil(self.plant_block((pos.0, pos.1 - 1, pos.2)) >> 4)
                    && self.plant_block(up) >> 4 == block::AIR
                {
                    self.set_block_flags(pos, DOUBLE_PLANT << 4 | variant, flags::SEND);
                    self.set_block_flags(up, DOUBLE_PLANT << 4 | UPPER_STATE, flags::SEND);
                }
            }
            DOUBLE_PLANT => {
                let variant = self.double_plant_variant(pos, state);
                let stack = ItemStack {
                    id: DOUBLE_PLANT as i16,
                    count: 1,
                    damage: variant as i16,
                    nbt: None,
                };
                self.spawn_as_entity(pos, stack);
            }
            _ => {}
        }
    }

    /// `BlockGrass.grow`: tall grass and flowers on the grass around.
    fn grow_grass(&mut self, pos: (i32, i32, i32)) {
        let start = (pos.0, pos.1 + 1, pos.2);
        'attempt: for attempt in 0..GRASS_ATTEMPTS {
            let mut p = start;
            for _ in 0..attempt / 16 {
                let r = &mut self.world_rand;
                let dx = r.next_int_bounded(3) - 1;
                let dy = (r.next_int_bounded(3) - 1) * r.next_int_bounded(3) / 2;
                let dz = r.next_int_bounded(3) - 1;
                p = (p.0 + dx, p.1 + dy, p.2 + dz);
                if self.plant_block((p.0, p.1 - 1, p.2)) >> 4 != block::GRASS
                    || super::placement::is_normal_cube(self.plant_block(p))
                {
                    continue 'attempt;
                }
            }
            if material(self.plant_block(p) >> 4) != Material::Air {
                continue;
            }
            let soil = bush_soil(self.plant_block((p.0, p.1 - 1, p.2)) >> 4);
            if self.world_rand.next_int_bounded(FLOWER_ODDS) == 0 {
                let biome = self.biome_id_at(p.0, p.2);
                let (id, meta) = FLOWERS[pick_flower(&mut self.world_rand, biome, p)];
                if soil {
                    self.set_block_notify(p, id << 4 | meta);
                }
            } else if soil {
                self.set_block_notify(p, block::TALLGRASS << 4 | GRASS);
            }
        }
    }

    /// `BlockDoublePlant.onBlockPlacedBy`: the upper half above a placed lower half.
    pub(super) fn place_double_plant_top(&mut self, lower: (i32, i32, i32)) {
        let up = (lower.0, lower.1 + 1, lower.2);
        self.set_block_flags(up, DOUBLE_PLANT << 4 | UPPER_STATE, flags::SEND);
    }

    /// `BlockDoublePlant.checkAndDropBlock` (on neighbor changes and random ticks): an
    /// upper half needs its lower half, a lower half its upper half and soil. Both
    /// halves go; only a lower half drops its items.
    pub(super) fn check_double_plant(&mut self, pos: (i32, i32, i32), state: u16) {
        let upper = state & UPPER != 0;
        let stays = if upper {
            self.plant_block((pos.0, pos.1 - 1, pos.2)) >> 4 == DOUBLE_PLANT
        } else {
            self.plant_block((pos.0, pos.1 + 1, pos.2)) >> 4 == DOUBLE_PLANT
                && bush_soil(self.plant_block((pos.0, pos.1 - 1, pos.2)) >> 4)
        };
        if stays {
            return;
        }
        let (top, bottom) = if upper {
            (pos, (pos.0, pos.1 - 1, pos.2))
        } else {
            ((pos.0, pos.1 + 1, pos.2), pos)
        };
        if self.plant_block(top) >> 4 == DOUBLE_PLANT {
            self.set_block_flags(top, 0, flags::SEND);
        }
        if self.plant_block(bottom) >> 4 == DOUBLE_PLANT {
            self.set_block_notify(bottom, 0);
            if !upper {
                self.drop_block_items(bottom, state);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swamp_and_default_flowers() {
        let mut r = krol_java::JavaRandom::new(1);
        assert_eq!(pick_flower(&mut r, SWAMPLAND, (0, 64, 0)), BLUE_ORCHID);
        for _ in 0..20 {
            let f = pick_flower(&mut r, 4, (0, 64, 0));
            assert!(f == DANDELION || f == POPPY);
        }
    }

    #[test]
    fn double_plant_drops() {
        let mut r = krol_java::JavaRandom::new(1);
        assert_eq!(double_plant_drop(&mut r, UPPER_STATE), None);
        assert_eq!(double_plant_drop(&mut r, LARGE_FERN), None);
        assert_eq!(double_plant_drop(&mut r, 4), Some((DOUBLE_PLANT as i16, 4)));
    }
}
