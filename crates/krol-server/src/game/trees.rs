//! Saplings and the tree generators they use (vanilla `BlockSapling`, `WorldGenTrees`,
//! `WorldGenForest`, `WorldGenTaiga2`). World generation will reuse the generators.
//!
//! TODO(M5): fancy oak (`WorldGenBigTree`), acacia, dark oak and the 2x2 trees.

use krol_java::JavaRandom;

use super::Game;
use super::block_data::{Material, material};
use super::block_update::flags;
use super::ids::block;

/// Sapling metadata: the wood type in bits 0-2, the growth stage in bit 3.
const SAPLING_TYPE: u16 = 7;
const SAPLING_STAGE: u16 = 8;
/// Wood types (`BlockPlanks.EnumType`).
const OAK: u16 = 0;
const SPRUCE: u16 = 1;
const BIRCH: u16 = 2;
const JUNGLE: u16 = 3;
/// Saplings need this much light above them to grow, and grow 1 time in 7.
const SAPLING_LIGHT: i32 = 9;
const SAPLING_GROW_CHANCE: i32 = 7;
/// 1 in 10 oak saplings would become a fancy oak.
const BIG_TREE_CHANCE: i32 = 10;
/// `vine` side bits (`BlockVine`): the side the vine hangs on.
const VINE_SOUTH: u16 = 1;
const VINE_WEST: u16 = 2;
const VINE_NORTH: u16 = 4;
const VINE_EAST: u16 = 8;
/// `cocoa`.
const COCOA: u16 = 127;

/// What a tree generator writes and reads: a world with its random.
struct Gen<'a> {
    game: &'a mut Game,
    /// `setBlockAndNotifyAdequately`: flags 3 when growing from a sapling, else 2.
    notify: bool,
}

impl Gen<'_> {
    fn get(&mut self, p: (i32, i32, i32)) -> u16 {
        self.game.level.overworld.block(p.0, p.1, p.2)
    }

    fn id(&mut self, p: (i32, i32, i32)) -> u16 {
        self.get(p) >> 4
    }

    fn set(&mut self, p: (i32, i32, i32), state: u16) {
        let f = if self.notify {
            flags::DEFAULT
        } else {
            flags::SEND
        };
        self.game.set_block_flags(p, state, f);
    }

    fn rand(&mut self) -> &mut JavaRandom {
        &mut self.game.world_rand
    }

    /// `WorldGenAbstractTree.func_150523_a`: blocks a tree may grow into.
    fn can_grow_into(&mut self, p: (i32, i32, i32)) -> bool {
        let id = self.id(p);
        matches!(material(id), Material::Air | Material::Leaves)
            || matches!(
                id,
                block::GRASS | block::DIRT | block::LOG | block::LOG2 | block::SAPLING | VINE
            )
    }

    /// Air, leaves or vines (the blocks trunks and leaves replace).
    fn soft(&mut self, p: (i32, i32, i32)) -> bool {
        matches!(
            material(self.id(p)),
            Material::Air | Material::Leaves | Material::Vine
        )
    }

    /// `WorldGenAbstractTree.func_175921_a`: the ground under the trunk becomes dirt.
    fn set_dirt(&mut self, p: (i32, i32, i32)) {
        if self.id(p) != block::DIRT {
            self.set(p, block::DIRT << 4);
        }
    }

    /// The trunk stands on grass, dirt or farmland.
    fn on_soil(&mut self, p: (i32, i32, i32)) -> bool {
        matches!(
            self.id((p.0, p.1 - 1, p.2)),
            block::GRASS | block::DIRT | block::FARMLAND
        )
    }
}

/// `vine`.
const VINE: u16 = 106;

/// The space check shared by `WorldGenTrees` and `WorldGenForest`: radius 0 at the
/// base, 1 up the trunk, 2 for the top two layers.
fn space_for_trunk(g: &mut Gen, pos: (i32, i32, i32), height: i32) -> bool {
    for y in pos.1..=pos.1 + 1 + height {
        let r = if y == pos.1 {
            0
        } else if y >= pos.1 + 1 + height - 2 {
            2
        } else {
            1
        };
        for x in pos.0 - r..=pos.0 + r {
            for z in pos.2 - r..=pos.2 + r {
                if !(0..256).contains(&y) || !g.can_grow_into((x, y, z)) {
                    return false;
                }
            }
        }
    }
    true
}

/// The rounded leaf blob of oak and birch: four layers, radius 2, 2, 1, 1 from the
/// bottom, corners left out at random (always at the top).
fn leaf_blob(g: &mut Gen, pos: (i32, i32, i32), height: i32, leaves: u16, replace_vines: bool) {
    for y in pos.1 - 3 + height..=pos.1 + height {
        let dy = y - (pos.1 + height);
        let r = 1 - dy / 2;
        for x in pos.0 - r..=pos.0 + r {
            let dx = x - pos.0;
            for z in pos.2 - r..=pos.2 + r {
                let dz = z - pos.2;
                if dx.abs() != r || dz.abs() != r || g.rand().next_int_bounded(2) != 0 && dy != 0 {
                    let m = material(g.id((x, y, z)));
                    if m == Material::Air
                        || m == Material::Leaves
                        || (replace_vines && m == Material::Vine)
                    {
                        g.set((x, y, z), leaves);
                    }
                }
            }
        }
    }
}

/// `WorldGenTrees`: oak, or jungle with vines and cocoa.
fn small_tree(
    g: &mut Gen,
    pos: (i32, i32, i32),
    min_height: i32,
    wood: u16,
    leaves: u16,
    vines: bool,
) -> bool {
    let height = g.rand().next_int_bounded(3) + min_height;
    if pos.1 < 1 || pos.1 + height + 1 > 256 || !space_for_trunk(g, pos, height) {
        return false;
    }
    if !g.on_soil(pos) || pos.1 >= 256 - height - 1 {
        return false;
    }
    g.set_dirt((pos.0, pos.1 - 1, pos.2));
    leaf_blob(g, pos, height, leaves, true);
    for k in 0..height {
        let p = (pos.0, pos.1 + k, pos.2);
        if !g.soft(p) {
            continue;
        }
        g.set(p, wood);
        if vines && k > 0 {
            for (dx, dz, side) in [
                (-1, 0, VINE_EAST),
                (1, 0, VINE_WEST),
                (0, -1, VINE_SOUTH),
                (0, 1, VINE_NORTH),
            ] {
                let q = (pos.0 + dx, pos.1 + k, pos.2 + dz);
                if g.rand().next_int_bounded(3) > 0 && g.id(q) == block::AIR {
                    g.set(q, VINE << 4 | side);
                }
            }
        }
    }
    if vines {
        for y in pos.1 - 3 + height..=pos.1 + height {
            let dy = y - (pos.1 + height);
            let r = 2 - dy / 2;
            for x in pos.0 - r..=pos.0 + r {
                for z in pos.2 - r..=pos.2 + r {
                    if material(g.id((x, y, z))) != Material::Leaves {
                        continue;
                    }
                    for (dx, dz, side) in [
                        (-1, 0, VINE_EAST),
                        (1, 0, VINE_WEST),
                        (0, -1, VINE_SOUTH),
                        (0, 1, VINE_NORTH),
                    ] {
                        let q = (x + dx, y, z + dz);
                        if g.rand().next_int_bounded(4) == 0 && g.id(q) == block::AIR {
                            hanging_vine(g, q, side);
                        }
                    }
                }
            }
        }
        if g.rand().next_int_bounded(5) == 0 && height > 5 {
            for k in 0..2 {
                // EnumFacing.Plane.HORIZONTAL: north, east, south, west, with their
                // horizontal indices (south 0, west 1, north 2, east 3).
                for (fx, fz, index) in [(0, -1, 2), (1, 0, 3), (0, 1, 0), (-1, 0, 1)] {
                    if g.rand().next_int_bounded(4 - k) == 0 {
                        let age = g.rand().next_int_bounded(3) as u16;
                        let q = (pos.0 - fx, pos.1 + height - 5 + k, pos.2 - fz);
                        g.set(q, COCOA << 4 | index | age << 2);
                    }
                }
            }
        }
    }
    true
}

/// `WorldGenTrees.func_181650_b`: a vine and up to four more below it.
fn hanging_vine(g: &mut Gen, p: (i32, i32, i32), side: u16) {
    g.set(p, VINE << 4 | side);
    let mut below = (p.0, p.1 - 1, p.2);
    let mut left = 4;
    while g.id(below) == block::AIR && left > 0 {
        g.set(below, VINE << 4 | side);
        below.1 -= 1;
        left -= 1;
    }
}

/// `WorldGenForest`: birch.
fn birch_tree(g: &mut Gen, pos: (i32, i32, i32), tall: bool) -> bool {
    let mut height = g.rand().next_int_bounded(3) + 5;
    if tall {
        height += g.rand().next_int_bounded(7);
    }
    if pos.1 < 1 || pos.1 + height + 1 > 256 || !space_for_trunk(g, pos, height) {
        return false;
    }
    if !g.on_soil(pos) || pos.1 >= 256 - height - 1 {
        return false;
    }
    g.set_dirt((pos.0, pos.1 - 1, pos.2));
    let leaves = block::LEAVES << 4 | BIRCH;
    leaf_blob(g, pos, height, leaves, false);
    for k in 0..height {
        let p = (pos.0, pos.1 + k, pos.2);
        if matches!(material(g.id(p)), Material::Air | Material::Leaves) {
            g.set(p, block::LOG << 4 | BIRCH);
        }
    }
    true
}

/// `Block.isFullBlock` for what spruce leaves may not replace: blocks that were opaque
/// cubes when registered (leaves count, see `Block`'s constructor).
fn full_block(id: u16) -> bool {
    super::leaves::is_leaves(id) || super::light_data::opacity(id) == 255 && !is_single_slab(id)
}

fn is_single_slab(id: u16) -> bool {
    matches!(id, 44 | 126 | 182)
}

/// `WorldGenTaiga2`: spruce.
fn spruce_tree(g: &mut Gen, pos: (i32, i32, i32)) -> bool {
    let height = g.rand().next_int_bounded(4) + 6;
    let bare = 1 + g.rand().next_int_bounded(2);
    let leafy = height - bare;
    let max_radius = 2 + g.rand().next_int_bounded(2);
    if pos.1 < 1 || pos.1 + height + 1 > 256 {
        return false;
    }
    for y in pos.1..=pos.1 + 1 + height {
        let r = if y - pos.1 < bare { 0 } else { max_radius };
        for x in pos.0 - r..=pos.0 + r {
            for z in pos.2 - r..=pos.2 + r {
                if !(0..256).contains(&y) {
                    return false;
                }
                if !matches!(material(g.id((x, y, z))), Material::Air | Material::Leaves) {
                    return false;
                }
            }
        }
    }
    if !g.on_soil(pos) || pos.1 >= 256 - height - 1 {
        return false;
    }
    g.set_dirt((pos.0, pos.1 - 1, pos.2));
    let leaves = block::LEAVES << 4 | SPRUCE;
    let mut r = g.rand().next_int_bounded(2);
    let mut step = 1;
    let mut next_start = 0;
    for k in 0..=leafy {
        let y = pos.1 + height - k;
        for x in pos.0 - r..=pos.0 + r {
            for z in pos.2 - r..=pos.2 + r {
                if (x - pos.0).abs() != r || (z - pos.2).abs() != r || r <= 0 {
                    let id = g.id((x, y, z));
                    if !full_block(id) {
                        g.set((x, y, z), leaves);
                    }
                }
            }
        }
        if r >= step {
            r = next_start;
            next_start = 1;
            step += 1;
            if step > max_radius {
                step = max_radius;
            }
        } else {
            r += 1;
        }
    }
    let top_gap = g.rand().next_int_bounded(3);
    for k in 0..height - top_gap {
        let p = (pos.0, pos.1 + k, pos.2);
        if matches!(material(g.id(p)), Material::Air | Material::Leaves) {
            g.set(p, block::LOG << 4 | SPRUCE);
        }
    }
    true
}

impl Game {
    /// `BlockSapling.updateTick` after the bush check: in enough light, 1 time in 7 the
    /// sapling grows.
    pub(super) fn sapling_tick(&mut self, pos: (i32, i32, i32), state: u16) {
        if self.light_from_neighbors((pos.0, pos.1 + 1, pos.2)) >= SAPLING_LIGHT
            && self.world_rand.next_int_bounded(SAPLING_GROW_CHANCE) == 0
        {
            self.grow_sapling(pos, state);
        }
    }

    /// `BlockSapling.grow`: the first time to stage 1, then into a tree.
    pub(super) fn grow_sapling(&mut self, pos: (i32, i32, i32), state: u16) {
        if state & SAPLING_STAGE == 0 {
            self.set_block_flags(pos, state | SAPLING_STAGE, 0);
        } else {
            self.generate_tree(pos, state);
        }
    }

    /// `BlockSapling.generateTree` for the kinds krol models: the sapling is cleared,
    /// the tree generated, and the sapling put back if there was no room.
    fn generate_tree(&mut self, pos: (i32, i32, i32), state: u16) {
        // Drawn for every kind, before the kind is looked at.
        let big = self.world_rand.next_int_bounded(BIG_TREE_CHANCE) == 0;
        let kind = state & SAPLING_TYPE;
        let jungle_height = if kind == JUNGLE {
            // TODO(M5): four jungle saplings make a mega jungle tree.
            Some(4 + self.world_rand.next_int_bounded(7))
        } else {
            None
        };
        // TODO(M5): fancy oak, acacia, dark oak and the 2x2 spruce and jungle trees.
        if (kind == OAK && big) || !matches!(kind, OAK | SPRUCE | BIRCH | JUNGLE) {
            return;
        }
        self.set_block_flags(pos, 0, 0);
        let mut g = Gen {
            game: self,
            notify: true,
        };
        let grown = match kind {
            SPRUCE => spruce_tree(&mut g, pos),
            BIRCH => birch_tree(&mut g, pos, false),
            JUNGLE => small_tree(
                &mut g,
                pos,
                jungle_height.unwrap_or(4),
                block::LOG << 4 | JUNGLE,
                block::LEAVES << 4 | JUNGLE,
                false,
            ),
            _ => small_tree(
                &mut g,
                pos,
                4,
                block::LOG << 4 | OAK,
                block::LEAVES << 4 | OAK,
                false,
            ),
        };
        if !grown {
            self.set_block_flags(pos, state, 0);
        }
    }
}
