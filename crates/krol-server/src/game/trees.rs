//! Saplings and the tree generators they use (vanilla `BlockSapling`, `WorldGenTrees`,
//! `WorldGenForest`, `WorldGenTaiga2`). World generation will reuse the generators.
//!
//! TODO(M5): the 2x2 spruce and jungle trees.

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
/// Acacia and dark oak: `leaves2`/`log2` variants 0 and 1, sapling types 4 and 5.
const ACACIA: u16 = 4;
const DARK_OAK: u16 = 5;
/// Their variants in `log2` and `leaves2` (type minus 4).
const ACACIA_VARIANT: u16 = ACACIA - 4;
const DARK_OAK_VARIANT: u16 = DARK_OAK - 4;
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

/// `WorldGenBigTree`: the fancy oak, with its own random seeded from the world's.
struct BigTree {
    rand: JavaRandom,
    base: (i32, i32, i32),
    height_limit: i32,
    height: i32,
    /// Leaf clusters: position and the trunk height their branch starts at.
    nodes: Vec<((i32, i32, i32), i32)>,
}

/// `WorldGenBigTree` constants.
const HEIGHT_ATTENUATION: f64 = 0.618;
const BRANCH_SLOPE: f64 = 0.381;
const SCALE_WIDTH: f64 = 1.0;
const LEAF_DENSITY: f64 = 1.0;
const HEIGHT_LIMIT_LIMIT: i32 = 12;
const LEAF_DISTANCE_LIMIT: i32 = 4;
/// Log axis bits (`BlockLog.EnumAxis`).
const AXIS_X: u16 = 4;
const AXIS_Z: u16 = 8;

/// `new BlockPos(base.x + dx, base.y + dy, base.z + dz)` with doubles (floored).
fn add_f(p: (i32, i32, i32), dx: f64, dy: f64, dz: f64) -> (i32, i32, i32) {
    let floor = krol_java::math::floor_f64;
    (
        floor(f64::from(p.0) + dx),
        floor(f64::from(p.1) + dy),
        floor(f64::from(p.2) + dz),
    )
}

fn greatest_distance(d: (i32, i32, i32)) -> i32 {
    let (x, y, z) = (d.0.abs(), d.1.abs(), d.2.abs());
    if z > x && z > y {
        z
    } else if y > x {
        y
    } else {
        x
    }
}

/// The points of the line from `a` to `b` that `WorldGenBigTree` steps along.
fn line_points(a: (i32, i32, i32), b: (i32, i32, i32)) -> Vec<(i32, i32, i32)> {
    let d = (b.0 - a.0, b.1 - a.1, b.2 - a.2);
    let n = greatest_distance(d);
    let (sx, sy, sz) = (
        d.0 as f32 / n as f32,
        d.1 as f32 / n as f32,
        d.2 as f32 / n as f32,
    );
    (0..=n)
        .map(|k| {
            add_f(
                a,
                f64::from(0.5 + k as f32 * sx),
                f64::from(0.5 + k as f32 * sy),
                f64::from(0.5 + k as f32 * sz),
            )
        })
        .collect()
}

impl BigTree {
    /// `checkBlockLine`: the step at which the line hits something a tree cannot grow
    /// into, or -1.
    fn check_line(g: &mut Gen, a: (i32, i32, i32), b: (i32, i32, i32)) -> i32 {
        let d = (b.0 - a.0, b.1 - a.1, b.2 - a.2);
        if greatest_distance(d) == 0 {
            return -1;
        }
        for (k, p) in line_points(a, b).into_iter().enumerate() {
            if !g.can_grow_into(p) {
                return k as i32;
            }
        }
        -1
    }

    /// `layerSize`.
    fn layer_size(&self, y: i32) -> f32 {
        if (y as f32) < self.height_limit as f32 * 0.3 {
            return -1.0;
        }
        let half = self.height_limit as f32 / 2.0;
        let dy = half - y as f32;
        let mut size = krol_java::math::sqrt_f32(half * half - dy * dy);
        if dy == 0.0 {
            size = half;
        } else if dy.abs() >= half {
            return 0.0;
        }
        size * 0.5
    }

    /// `generateLeafNodeList`.
    fn leaf_nodes(&mut self, g: &mut Gen) {
        self.height = (f64::from(self.height_limit) * HEIGHT_ATTENUATION) as i32;
        if self.height >= self.height_limit {
            self.height = self.height_limit - 1;
        }
        let density = LEAF_DENSITY * f64::from(self.height_limit) / 13.0;
        let per_layer = ((1.382 + density * density) as i32).max(1);
        let top = self.base.1 + self.height;
        let mut y = self.height_limit - LEAF_DISTANCE_LIMIT;
        self.nodes = vec![((self.base.0, self.base.1 + y, self.base.2), top)];
        while y >= 0 {
            let size = self.layer_size(y);
            if size >= 0.0 {
                for _ in 0..per_layer {
                    let radius =
                        SCALE_WIDTH * f64::from(size) * (f64::from(self.rand.next_float()) + 0.328);
                    let angle = f64::from(self.rand.next_float() * 2.0) * std::f64::consts::PI;
                    // Vanilla uses Math.sin/cos here (within an ulp of these).
                    let dx = radius * angle.sin() + 0.5;
                    let dz = radius * angle.cos() + 0.5;
                    let node = add_f(self.base, dx, f64::from(y - 1), dz);
                    let above = (node.0, node.1 + LEAF_DISTANCE_LIMIT, node.2);
                    if Self::check_line(g, node, above) != -1 {
                        continue;
                    }
                    let ox = self.base.0 - node.0;
                    let oz = self.base.2 - node.2;
                    let branch =
                        f64::from(node.1) - f64::from(ox * ox + oz * oz).sqrt() * BRANCH_SLOPE;
                    let branch_y = if branch > f64::from(top) {
                        top
                    } else {
                        branch as i32
                    };
                    let start = (self.base.0, branch_y, self.base.2);
                    if Self::check_line(g, start, node) == -1 {
                        self.nodes.push((node, branch_y));
                    }
                }
            }
            y -= 1;
        }
    }

    /// `func_181631_a`: one round layer of a leaf cluster.
    fn leaf_layer(g: &mut Gen, center: (i32, i32, i32), size: f32) {
        let r = (f64::from(size) + 0.618) as i32;
        for dx in -r..=r {
            for dz in -r..=r {
                let a = f64::from(dx.abs()) + 0.5;
                let b = f64::from(dz.abs()) + 0.5;
                if a * a + b * b <= f64::from(size * size) {
                    let p = (center.0 + dx, center.1, center.2 + dz);
                    if matches!(material(g.id(p)), Material::Air | Material::Leaves) {
                        g.set(p, block::LEAVES << 4);
                    }
                }
            }
        }
    }

    /// `leafSize`.
    fn leaf_size(y: i32) -> f32 {
        if !(0..LEAF_DISTANCE_LIMIT).contains(&y) {
            -1.0
        } else if y != 0 && y != LEAF_DISTANCE_LIMIT - 1 {
            3.0
        } else {
            2.0
        }
    }

    /// `func_175937_a`: a line of logs, each turned along the line's main axis.
    fn log_line(g: &mut Gen, a: (i32, i32, i32), b: (i32, i32, i32)) {
        for p in line_points(a, b) {
            let dx = (p.0 - a.0).abs();
            let dz = (p.2 - a.2).abs();
            let m = dx.max(dz);
            let axis = if m > 0 && dx == m {
                AXIS_X
            } else if m > 0 && dz == m {
                AXIS_Z
            } else {
                0
            };
            g.set(p, block::LOG << 4 | axis);
        }
    }

    fn generate(mut self, g: &mut Gen) -> bool {
        self.height_limit = 5 + self.rand.next_int_bounded(HEIGHT_LIMIT_LIMIT);
        // validTreeLocation.
        if !matches!(
            g.id((self.base.0, self.base.1 - 1, self.base.2)),
            block::DIRT | block::GRASS | block::FARMLAND
        ) {
            return false;
        }
        let top = (
            self.base.0,
            self.base.1 + self.height_limit - 1,
            self.base.2,
        );
        let blocked = Self::check_line(g, self.base, top);
        if blocked != -1 {
            if blocked < 6 {
                return false;
            }
            self.height_limit = blocked;
        }
        self.leaf_nodes(g);
        for &(node, _) in &self.nodes {
            for k in 0..LEAF_DISTANCE_LIMIT {
                Self::leaf_layer(g, (node.0, node.1 + k, node.2), Self::leaf_size(k));
            }
        }
        let trunk_top = (self.base.0, self.base.1 + self.height, self.base.2);
        Self::log_line(g, self.base, trunk_top);
        for &(node, branch_y) in &self.nodes {
            let start = (self.base.0, branch_y, self.base.2);
            // leafNodeNeedsBase.
            let needs = f64::from(branch_y - self.base.1) >= f64::from(self.height_limit) * 0.2;
            if start != node && needs {
                Self::log_line(g, start, node);
            }
        }
        true
    }
}

/// `EnumFacing.Plane.HORIZONTAL.random`: north, east, south or west, as `(dx, dz)`.
fn random_horizontal(g: &mut Gen) -> (i32, i32) {
    [(0, -1), (1, 0), (0, 1), (-1, 0)][g.rand().next_int_bounded(4) as usize]
}

/// `WorldGenSavannaTree.func_175924_b`: acacia leaves where there is air or leaves.
fn acacia_leaves(g: &mut Gen, p: (i32, i32, i32)) {
    if matches!(material(g.id(p)), Material::Air | Material::Leaves) {
        g.set(p, block::LEAVES2 << 4 | ACACIA_VARIANT);
    }
}

/// `WorldGenSavannaTree`: acacia, a trunk that bends to one side and maybe a second
/// branch, each ending in a flat crown.
fn acacia_tree(g: &mut Gen, pos: (i32, i32, i32)) -> bool {
    let height = g.rand().next_int_bounded(3) + g.rand().next_int_bounded(3) + 5;
    if pos.1 < 1 || pos.1 + height + 1 > 256 || !space_for_trunk(g, pos, height) {
        return false;
    }
    let below = g.id((pos.0, pos.1 - 1, pos.2));
    if !matches!(below, block::GRASS | block::DIRT) || pos.1 >= 256 - height - 1 {
        return false;
    }
    g.set_dirt((pos.0, pos.1 - 1, pos.2));
    let wood = block::LOG2 << 4 | ACACIA_VARIANT;
    let lean = random_horizontal(g);
    let bend_at = height - g.rand().next_int_bounded(4) - 1;
    let mut bend = 3 - g.rand().next_int_bounded(3);
    let (mut x, mut z) = (pos.0, pos.2);
    let mut top = 0;
    for k in 0..height {
        let y = pos.1 + k;
        if k >= bend_at && bend > 0 {
            x += lean.0;
            z += lean.1;
            bend -= 1;
        }
        if matches!(material(g.id((x, y, z))), Material::Air | Material::Leaves) {
            g.set((x, y, z), wood);
            top = y;
        }
    }
    let crown = (x, top, z);
    for dx in -3i32..=3 {
        for dz in -3i32..=3 {
            if dx.abs() != 3 || dz.abs() != 3 {
                acacia_leaves(g, (crown.0 + dx, crown.1, crown.2 + dz));
            }
        }
    }
    let up = (crown.0, crown.1 + 1, crown.2);
    for dx in -1..=1 {
        for dz in -1..=1 {
            acacia_leaves(g, (up.0 + dx, up.1, up.2 + dz));
        }
    }
    for (dx, dz) in [(2, 0), (-2, 0), (0, 2), (0, -2)] {
        acacia_leaves(g, (up.0 + dx, up.1, up.2 + dz));
    }
    let (mut x, mut z) = (pos.0, pos.2);
    let branch = random_horizontal(g);
    if branch != lean {
        let mut k = bend_at - g.rand().next_int_bounded(2) - 1;
        let mut left = 1 + g.rand().next_int_bounded(3);
        let mut end = 0;
        while k < height && left > 0 {
            if k >= 1 {
                let y = pos.1 + k;
                x += branch.0;
                z += branch.1;
                if matches!(material(g.id((x, y, z))), Material::Air | Material::Leaves) {
                    g.set((x, y, z), wood);
                    end = y;
                }
            }
            k += 1;
            left -= 1;
        }
        if end > 0 {
            let c = (x, end, z);
            for dx in -2i32..=2 {
                for dz in -2i32..=2 {
                    if dx.abs() != 2 || dz.abs() != 2 {
                        acacia_leaves(g, (c.0 + dx, c.1, c.2 + dz));
                    }
                }
            }
            for dx in -1..=1 {
                for dz in -1..=1 {
                    acacia_leaves(g, (c.0 + dx, c.1 + 1, c.2 + dz));
                }
            }
        }
    }
    true
}

/// `WorldGenCanopyTree.func_181639_b`: a dark oak log where a tree may grow.
fn canopy_log(g: &mut Gen, p: (i32, i32, i32)) {
    if g.can_grow_into(p) {
        g.set(p, block::LOG2 << 4 | DARK_OAK_VARIANT);
    }
}

/// `WorldGenCanopyTree.func_150526_a`: dark oak leaves in air only.
fn canopy_leaves(g: &mut Gen, p: (i32, i32, i32)) {
    if material(g.id(p)) == Material::Air {
        g.set(p, block::LEAVES2 << 4 | DARK_OAK_VARIANT);
    }
}

/// `WorldGenCanopyTree`: dark oak, a 2x2 trunk (`pos` is its north-west corner) that
/// may lean, a wide flat crown and short side branches.
fn dark_oak_tree(g: &mut Gen, pos: (i32, i32, i32)) -> bool {
    let height = g.rand().next_int_bounded(3) + g.rand().next_int_bounded(2) + 6;
    let (x0, y0, z0) = pos;
    if y0 < 1 || y0 + height + 1 >= 256 {
        return false;
    }
    if !matches!(g.id((x0, y0 - 1, z0)), block::GRASS | block::DIRT) {
        return false;
    }
    // func_181638_a: the space around the corner column.
    for dy in 0..=height + 1 {
        let r = if dy == 0 {
            0
        } else if dy >= height - 1 {
            2
        } else {
            1
        };
        for dx in -r..=r {
            for dz in -r..=r {
                if !g.can_grow_into((x0 + dx, y0 + dy, z0 + dz)) {
                    return false;
                }
            }
        }
    }
    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        g.set_dirt((x0 + dx, y0 - 1, z0 + dz));
    }
    let lean = random_horizontal(g);
    let bend_at = height - g.rand().next_int_bounded(4);
    let mut bend = 2 - g.rand().next_int_bounded(3);
    let (mut x, mut z) = (x0, z0);
    let top = y0 + height - 1;
    for k in 0..height {
        if k >= bend_at && bend > 0 {
            x += lean.0;
            z += lean.1;
            bend -= 1;
        }
        let p = (x, y0 + k, z);
        if matches!(material(g.id(p)), Material::Air | Material::Leaves) {
            for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                canopy_log(g, (p.0 + dx, p.1, p.2 + dz));
            }
        }
    }
    for dx in -2..=0 {
        for dz in -2..=0 {
            let mirrored = |g: &mut Gen, y: i32| {
                canopy_leaves(g, (x + dx, y, z + dz));
                canopy_leaves(g, (1 + x - dx, y, z + dz));
                canopy_leaves(g, (x + dx, y, 1 + z - dz));
                canopy_leaves(g, (1 + x - dx, y, 1 + z - dz));
            };
            mirrored(g, top - 1);
            if (dx > -2 || dz > -1) && (dx != -1 || dz != -2) {
                mirrored(g, top + 1);
            }
        }
    }
    if g.rand().next_boolean() {
        for (dx, dz) in [(0, 0), (1, 0), (1, 1), (0, 1)] {
            canopy_leaves(g, (x + dx, top + 2, z + dz));
        }
    }
    for dx in -3i32..=4 {
        for dz in -3i32..=4 {
            let corner = (dx == -3 || dx == 4) && (dz == -3 || dz == 4);
            if !corner && (dx.abs() < 3 || dz.abs() < 3) {
                canopy_leaves(g, (x + dx, top, z + dz));
            }
        }
    }
    for dx in -1..=2 {
        for dz in -1..=2 {
            let outer = !(0..=1).contains(&dx) || !(0..=1).contains(&dz);
            if outer && g.rand().next_int_bounded(3) <= 0 {
                let length = g.rand().next_int_bounded(3) + 2;
                // Vanilla quirk: the branch logs start from the unshifted base.
                for k in 0..length {
                    canopy_log(g, (x0 + dx, top - k - 1, z0 + dz));
                }
                for ax in -1..=1 {
                    for az in -1..=1 {
                        canopy_leaves(g, (x + dx + ax, top, z + dz + az));
                    }
                }
                for ax in -2i32..=2 {
                    for az in -2i32..=2 {
                        if ax.abs() != 2 || az.abs() != 2 {
                            canopy_leaves(g, (x + dx + ax, top - 1, z + dz + az));
                        }
                    }
                }
            }
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
        // Spruce, jungle and dark oak look for a 2x2 of their saplings that includes this
        // one (corner offsets 0 and -1, x outer).
        let square = if matches!(kind, SPRUCE | JUNGLE | DARK_OAK) {
            self.sapling_square(pos, kind)
        } else {
            None
        };
        let mut jungle_height = 4;
        match (kind, square) {
            (SPRUCE, Some(_)) => {
                // TODO(M5): WorldGenMegaPineTree(false, nextBoolean()).
                let _ = self.world_rand.next_boolean();
                return;
            }
            // TODO(M5): WorldGenMegaJungle.
            (JUNGLE, Some(_)) => return,
            (JUNGLE, None) => jungle_height = 4 + self.world_rand.next_int_bounded(7),
            // A single dark oak sapling does not grow.
            (DARK_OAK, None) => return,
            _ => {}
        }
        let corner = square.unwrap_or((0, 0));
        let base = (pos.0 + corner.0, pos.1, pos.2 + corner.1);
        let cells: &[(i32, i32)] = if square.is_some() {
            &[(0, 0), (1, 0), (0, 1), (1, 1)]
        } else {
            &[(0, 0)]
        };
        for &(dx, dz) in cells {
            self.set_block_flags((base.0 + dx, base.1, base.2 + dz), 0, 0);
        }
        let mut g = Gen {
            game: self,
            notify: true,
        };
        let grown = match kind {
            OAK if big => {
                let seed = g.rand().next_long();
                BigTree {
                    rand: JavaRandom::new(seed),
                    base,
                    height_limit: 0,
                    height: 0,
                    nodes: Vec::new(),
                }
                .generate(&mut g)
            }
            SPRUCE => spruce_tree(&mut g, base),
            BIRCH => birch_tree(&mut g, base, false),
            JUNGLE => small_tree(
                &mut g,
                base,
                jungle_height,
                block::LOG << 4 | JUNGLE,
                block::LEAVES << 4 | JUNGLE,
                false,
            ),
            ACACIA => acacia_tree(&mut g, base),
            DARK_OAK => dark_oak_tree(&mut g, base),
            _ => small_tree(
                &mut g,
                base,
                4,
                block::LOG << 4 | OAK,
                block::LEAVES << 4 | OAK,
                false,
            ),
        };
        if !grown {
            // Every cleared sapling comes back as the one that grew.
            for &(dx, dz) in cells {
                self.set_block_flags((base.0 + dx, base.1, base.2 + dz), state, 0);
            }
        }
    }

    /// `BlockSapling.func_181624_a`: the north-west corner offset of a 2x2 of saplings
    /// of `kind` that includes `pos`, trying offsets 0 and -1 (x outer).
    fn sapling_square(&mut self, pos: (i32, i32, i32), kind: u16) -> Option<(i32, i32)> {
        let is = |g: &mut Self, x: i32, z: i32| {
            let s = g.level.overworld.block(x, pos.1, z);
            s >> 4 == block::SAPLING && s & SAPLING_TYPE == kind
        };
        for i in [0, -1] {
            for j in [0, -1] {
                let (x, z) = (pos.0 + i, pos.2 + j);
                if is(self, x, z)
                    && is(self, x + 1, z)
                    && is(self, x, z + 1)
                    && is(self, x + 1, z + 1)
                {
                    return Some((i, j));
                }
            }
        }
        None
    }
}
