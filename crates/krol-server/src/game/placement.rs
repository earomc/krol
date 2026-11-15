//! The block state an item places (vanilla: `ItemBlock` subclasses' `getMetadata`,
//! `Block.onBlockPlaced`, `canPlaceBlockAt` / `canPlaceBlockOnSide`, and the
//! `getMetaFromState` encodings).
//!
//! TODO(M4): items that place blocks without being block items (signs, doors, beds,
//! redstone, repeaters, comparators, cake, reeds, cauldrons, brewing stands, flower
//! pots, skulls, seeds), levers, buttons, vines, lily pads, snow layer stacking,
//! redstone torches' power, double chests' second half, and the neighbor checks of
//! cactus and mushrooms.

use super::block_data::{self, Material};
use super::ids;
use super::world::World;

/// Faces in vanilla's order (`EnumFacing` index).
pub const DOWN: u8 = 0;
/// Up.
pub const UP: u8 = 1;
/// North (-z).
pub const NORTH: u8 = 2;
/// South (+z).
pub const SOUTH: u8 = 3;
/// West (-x).
pub const WEST: u8 = 4;
/// East (+x).
pub const EAST: u8 = 5;

/// The position next to `p` on `face`.
pub fn offset(p: (i32, i32, i32), face: u8) -> (i32, i32, i32) {
    match face {
        DOWN => (p.0, p.1 - 1, p.2),
        UP => (p.0, p.1 + 1, p.2),
        NORTH => (p.0, p.1, p.2 - 1),
        SOUTH => (p.0, p.1, p.2 + 1),
        WEST => (p.0 - 1, p.1, p.2),
        _ => (p.0 + 1, p.1, p.2),
    }
}

pub fn opposite(face: u8) -> u8 {
    face ^ 1
}

/// `EnumFacing.getHorizontal`: 0 south, 1 west, 2 north, 3 east.
fn horizontal(index: i32) -> u8 {
    [SOUTH, WEST, NORTH, EAST][(index & 3) as usize]
}

/// `getHorizontalIndex` of a horizontal face.
fn horizontal_index(face: u8) -> u8 {
    match face {
        SOUTH => 0,
        WEST => 1,
        NORTH => 2,
        _ => 3,
    }
}

/// `rotateY` (clockwise seen from above).
fn rotate_y(face: u8) -> u8 {
    match face {
        NORTH => EAST,
        EAST => SOUTH,
        SOUTH => WEST,
        _ => NORTH,
    }
}

/// What a placement knows about the player and the click.
#[derive(Debug, Clone, Copy)]
pub struct Click {
    /// The clicked face.
    pub face: u8,
    /// Where on the face (0..1 each).
    pub hit: (f32, f32, f32),
    /// The player's yaw.
    pub yaw: f32,
    /// The player's position.
    pub pos: (f64, f64, f64),
}

impl Click {
    /// `Entity.getHorizontalFacing`.
    fn facing(&self) -> u8 {
        horizontal(krol_java::math::floor_f64(
            f64::from(self.yaw * 4.0 / 360.0) + 0.5,
        ))
    }

    /// `BlockPistonBase.getFacingFromEntity`.
    fn facing_3d(&self, at: (i32, i32, i32)) -> u8 {
        if (self.pos.0 as f32 - at.0 as f32).abs() < 2.0
            && (self.pos.2 as f32 - at.2 as f32).abs() < 2.0
        {
            let eye = self.pos.1 + f64::from(1.62f32);
            if eye - f64::from(at.1) > 2.0 {
                return UP;
            }
            if f64::from(at.1) - eye > 0.0 {
                return DOWN;
            }
        }
        opposite(self.facing())
    }

    /// Top half for stairs and slabs.
    fn top_half(&self) -> bool {
        self.face == DOWN || (self.face != UP && f64::from(self.hit.1) > 0.5)
    }
}

/// Blocks with an opaque, full, non-power-providing cube (`isBlockNormalCube`), for the
/// blocks krol models.
pub fn is_normal_cube(state: u16) -> bool {
    let id = state >> 4;
    let m = block_data::material(id);
    let translucent = matches!(
        m,
        Material::Leaves
            | Material::Glass
            | Material::Ice
            | Material::Tnt
            | Material::Snow
            | Material::Cactus
            | Material::Air
            | Material::Water
            | Material::Lava
            | Material::Plants
            | Material::Vine
            | Material::Circuits
            | Material::Carpet
            | Material::Fire
            | Material::Portal
            | Material::Web
    );
    let full = !matches!(
        id,
        26 | 44 | 126 | 182 | 53 | 67 | 108 | 109 | 114 | 128 | 134..=136 | 156 | 163 | 164
            | 180 | 54 | 130 | 146 | 81 | 88 | 60 | 78 | 92 | 116 | 117 | 118 | 120 | 122 | 138
            | 140 | 144 | 145 | 151 | 154 | 178 | 85 | 101 | 102 | 107 | 113 | 139 | 160
            | 183..=192 | 64 | 71 | 96 | 167 | 193..=197 | 29 | 33 | 34 | 36 | 65
            // Pressure plates.
            | 70 | 72 | 147 | 148
    );
    // Redstone blocks provide power.
    !translucent && full && id != 152
}

/// `World.doesBlockHaveSolidTopSurface`: opaque full cubes, upside-down stairs and
/// slabs, hoppers, snow layers of 8.
pub fn solid_top(state: u16) -> bool {
    let (id, meta) = (state >> 4, state & 15);
    match id {
        53 | 67 | 108 | 109 | 114 | 128 | 134..=136 | 156 | 163 | 164 | 180 => meta & 4 != 0,
        44 | 126 | 182 => meta & 8 != 0,
        43 | 125 | 181 | 154 => true,
        78 => meta & 7 == 7,
        _ => is_normal_cube(state),
    }
}

/// `BlockTorch.canPlaceAt`: a torch facing `face` is held by the block behind it (a
/// normal cube for wall torches; for standing torches a solid top, a fence, a wall or
/// glass).
pub fn torch_can_attach(world: &mut World, at: (i32, i32, i32), face: u8) -> bool {
    let support = offset(at, opposite(face));
    let s = world.block(support.0, support.1, support.2);
    (face != UP && face != DOWN && is_normal_cube(s))
        || (face == UP && (solid_top(s) || matches!(s >> 4, 85 | 113 | 188..=192 | 20 | 95 | 139)))
}

/// Item damage values become block metadata only for items with subtypes
/// (`ItemBlock.getMetadata` returns 0 otherwise).
fn item_meta(id: u16, damage: i16) -> u8 {
    let d = (damage & 15) as u8;
    match id {
        1 | 3 | 5 | 6 | 12 | 19 | 24 | 35 | 38 | 95 | 97 | 98 | 139 | 159 | 160 | 168 | 171
        | 175 | 179 | 31 => d,
        17 | 162 => d & 3,
        // ItemLeaves sets the "no decay" bit.
        18 | 161 => (d & 3) | 4,
        44 | 126 | 182 => d & 7,
        _ => 0,
    }
}

/// Blocks placed as themselves with no orientation and no support rules beyond being
/// replaceable (the item id is the block id).
fn plain(id: u16) -> bool {
    matches!(
        id,
        1..=5 | 7 | 12..=16 | 19..=22 | 24 | 25 | 35 | 41 | 42 | 45..=49 | 52 | 56 | 57 | 58 | 73
            | 79 | 80 | 82 | 84 | 85 | 87..=89 | 95 | 97 | 98 | 101..=103 | 110 | 112 | 113
            | 121..=123 | 129 | 133 | 138 | 139 | 152 | 153 | 159 | 160 | 165 | 166 | 168 | 169
            | 172..=174 | 179 | 18 | 161 | 30 | 188..=192
    )
}

/// The state the held item places at `at` (already chosen: the clicked block if
/// replaceable, else the block next to it), or `None` if it cannot be placed there.
pub fn placed_state(
    world: &mut World,
    item: i16,
    damage: i16,
    at: (i32, i32, i32),
    click: &Click,
) -> Option<u16> {
    if !(0..256).contains(&item) {
        return None;
    }
    let id = item as u16;
    let meta = item_meta(id, damage);
    let below = world.block(at.0, at.1 - 1, at.2);
    let state = |m: u8| Some(id << 4 | u16::from(m));
    match id {
        _ if plain(id) => state(meta),
        // Logs and hay: the axis of the clicked face.
        17 | 162 | 170 => {
            let axis = match click.face {
                WEST | EAST => 4,
                NORTH | SOUTH => 8,
                _ => 0,
            };
            state(if id == 170 { axis } else { meta | axis })
        }
        // Quartz pillars (damage 2) follow the face axis too.
        155 => state(match damage {
            2 => match click.face {
                WEST | EAST => 3,
                NORTH | SOUTH => 4,
                _ => 2,
            },
            1 => 1,
            _ => 0,
        }),
        53 | 67 | 108 | 109 | 114 | 128 | 134..=136 | 156 | 163 | 164 | 180 => {
            let half = if click.top_half() { 4 } else { 0 };
            state(half | (5 - click.facing()))
        }
        44 | 126 | 182 => state(meta | if click.top_half() { 8 } else { 0 }),
        43 | 125 | 181 => state(meta),
        // Torches attach to the clicked side, else to any side that can hold them.
        50 | 76 => {
            let can = |face: u8, world: &mut World| torch_can_attach(world, at, face);
            let any = [EAST, WEST, SOUTH, NORTH, UP]
                .iter()
                .any(|&f| can(f, world));
            if !any {
                return None;
            }
            let face = if can(click.face, world) {
                click.face
            } else {
                [NORTH, EAST, SOUTH, WEST]
                    .into_iter()
                    .find(|&f| {
                        let s = offset(at, opposite(f));
                        is_normal_cube(world.block(s.0, s.1, s.2))
                    })
                    .unwrap_or(UP)
            };
            state(match face {
                EAST => 1,
                WEST => 2,
                SOUTH => 3,
                NORTH => 4,
                _ => 5,
            })
        }
        65 => {
            let holds = |face: u8, world: &mut World| {
                let s = offset(at, opposite(face));
                is_normal_cube(world.block(s.0, s.1, s.2))
            };
            if ![NORTH, SOUTH, WEST, EAST].iter().any(|&f| holds(f, world)) {
                return None;
            }
            let face = if click.face >= NORTH && holds(click.face, world) {
                click.face
            } else {
                [NORTH, EAST, SOUTH, WEST]
                    .into_iter()
                    .find(|&f| holds(f, world))
                    .unwrap_or(NORTH)
            };
            state(face)
        }
        // Buttons and levers need a block to hold them on the clicked side
        // (canPlaceBlockOnSide); levers on floors and ceilings follow the player.
        ids::block::STONE_BUTTON | ids::block::WOODEN_BUTTON | ids::block::LEVER => {
            if !super::block_update::can_mount(world, at, opposite(click.face)) {
                return None;
            }
            let along_x = matches!(click.facing(), EAST | WEST);
            let lever = id == ids::block::LEVER;
            state(match click.face {
                UP if lever => {
                    if along_x {
                        6
                    } else {
                        5
                    }
                }
                DOWN if lever => {
                    if along_x {
                        0
                    } else {
                        7
                    }
                }
                DOWN => 0,
                EAST => 1,
                WEST => 2,
                SOUTH => 3,
                NORTH => 4,
                _ => 5,
            })
        }
        // BlockChest.canPlaceBlockAt: at most one neighboring chest of the same kind,
        // and that one must not be part of a double chest already.
        ids::block::CHEST | ids::block::TRAPPED_CHEST => {
            let is_chest =
                |p: (i32, i32, i32), world: &mut World| world.block(p.0, p.1, p.2) >> 4 == id;
            let mut count = 0;
            for f in [WEST, EAST, NORTH, SOUTH] {
                let n = offset(at, f);
                if is_chest(n, world) {
                    if [WEST, EAST, NORTH, SOUTH]
                        .iter()
                        .any(|&g| is_chest(offset(n, g), world))
                    {
                        return None;
                    }
                    count += 1;
                }
            }
            if count > 1 {
                return None;
            }
            state(opposite(click.facing()))
        }
        61 | 130 => state(opposite(click.facing())),
        86 | 91 => state(horizontal_index(opposite(click.facing()))),
        107 | 183..=187 => state(horizontal_index(click.facing())),
        145 => state(horizontal_index(rotate_y(click.facing())) | (((damage as u8) & 3) << 2)),
        23 | 158 | 29 | 33 => state(click.facing_3d(at)),
        154 => {
            let f = opposite(click.face);
            state(if f == UP { DOWN } else { f })
        }
        96 | 167 => {
            if click.face == UP || click.face == DOWN {
                return None;
            }
            let support = offset(at, opposite(click.face));
            let s = world.block(support.0, support.1, support.2);
            if !(is_normal_cube(s) || matches!(s >> 4, 89 | 44 | 126 | 182) || (s >> 4 == 53)) {
                return None;
            }
            let facing = match click.face {
                NORTH => 0,
                SOUTH => 1,
                WEST => 2,
                _ => 3,
            };
            state(facing | if click.hit.1 > 0.5 { 8 } else { 0 })
        }
        // Plants need dirt, grass or farmland below (BlockBush.canPlaceBlockOn).
        6 | 31 | 32 | 37 | 38 => {
            let ok = match id {
                32 => matches!(below >> 4, 12 | 172 | 159 | 3),
                _ => matches!(below >> 4, 2 | 3 | 60),
            };
            ok.then(|| id << 4 | u16::from(meta))
        }
        // BlockReed.canPlaceBlockAt: sugar cane on cane, or on grass, dirt or sand by water.
        ids::block::REEDS => {
            super::random_ticks::cane_can_stay(world, at, false).then_some(id << 4)
        }
        // BlockCake.canBlockStay: something solid below.
        CAKE => super::block_data::material(below >> 4)
            .is_solid()
            .then_some(id << 4),
        // Farmland is an ordinary block item (dry when placed).
        ids::block::FARMLAND => state(0),
        // BlockCactus.canPlaceBlockAt: sand or cactus below, nothing solid beside.
        ids::block::CACTUS => {
            super::random_ticks::cane_can_stay(world, at, true).then_some(id << 4)
        }
        // BlockSnow.canPlaceBlockAt: not on ice; on leaves, a full snow layer, or an
        // opaque block that blocks movement.
        ids::block::SNOW_LAYER => {
            let b = below >> 4;
            let ok = b != ids::block::ICE
                && b != ids::block::PACKED_ICE
                && (matches!(b, ids::block::LEAVES | ids::block::LEAVES2)
                    || (b == ids::block::SNOW_LAYER && below & 7 == 7)
                    || is_normal_cube(below));
            ok.then_some(id << 4)
        }
        // Carpet needs something below; pressure plates a solid top or a fence.
        171 => (below >> 4 != 0).then(|| id << 4 | u16::from(meta)),
        70 | 72 | 147 | 148 => {
            (solid_top(below) || matches!(below >> 4, 85 | 113 | 188..=192)).then_some(id << 4)
        }
        _ => None,
    }
}

/// `ItemSeeds` / `ItemSeedFood`: the soil an item is planted on and the crop it
/// becomes (wheat, pumpkin and melon seeds, carrots and potatoes on farmland; nether wart
/// on soul sand).
pub fn seed_crop(item: i16) -> Option<(u16, u16)> {
    use ids::block::{
        CARROTS, FARMLAND, MELON_STEM, NETHER_WART, POTATOES, PUMPKIN_STEM, SOUL_SAND, WHEAT,
    };
    Some(match item {
        295 => (FARMLAND, WHEAT),
        361 => (FARMLAND, PUMPKIN_STEM),
        362 => (FARMLAND, MELON_STEM),
        391 => (FARMLAND, CARROTS),
        392 => (FARMLAND, POTATOES),
        372 => (SOUL_SAND, NETHER_WART),
        _ => return None,
    })
}

/// `ItemReed`: items that place a block (sugar cane, cake).
pub fn reed_block(item: i16) -> Option<u16> {
    match item {
        338 => Some(ids::block::REEDS),
        354 => Some(CAKE),
        _ => None,
    }
}

/// `cake`.
const CAKE: u16 = 92;

/// The slab a double slab is made of (`ItemSlab`): `(single, double)`.
pub fn slab_pair(item: i16) -> Option<(u16, u16)> {
    match item {
        44 => Some((44, 43)),
        126 => Some((126, 125)),
        182 => Some((182, 181)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facings() {
        let click = |yaw: f32| Click {
            face: UP,
            hit: (0.5, 1.0, 0.5),
            yaw,
            pos: (0.5, 64.0, 0.5),
        };
        assert_eq!(click(0.0).facing(), SOUTH);
        assert_eq!(click(90.0).facing(), WEST);
        assert_eq!(click(180.0).facing(), NORTH);
        assert_eq!(click(-90.0).facing(), EAST);
        assert_eq!(click(-44.0).facing(), SOUTH);
    }
}
