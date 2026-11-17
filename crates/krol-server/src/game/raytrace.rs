//! Block ray tracing (vanilla `World.rayTraceBlocks` and `Block.collisionRayTrace`), used
//! where the server looks along a player's view itself, such as buckets.

use super::block_data::{Material, material};
use super::ids::block;
use super::world::World;

/// A point in the world.
pub type Vec3 = (f64, f64, f64);

/// How many blocks `rayTraceBlocks` steps through at most.
const MAX_STEPS: i32 = 200;
/// `Vec3.getIntermediateWith*Value`: segments shorter than this along the axis miss.
const MIN_AXIS_LENGTH: f64 = 1.0e-7_f32 as f64;
/// Stands in for a step length of exactly -0.0.
const NEGATIVE_ZERO_STEP: f64 = -1.0e-4;
/// The step length of an axis the ray does not cross.
const NO_STEP: f64 = 999.0;

/// The faces in vanilla's `EnumFacing` index order.
pub mod face {
    /// Down (-y).
    pub const DOWN: u8 = 0;
    /// Up (+y).
    pub const UP: u8 = 1;
    /// North (-z).
    pub const NORTH: u8 = 2;
    /// South (+z).
    pub const SOUTH: u8 = 3;
    /// West (-x).
    pub const WEST: u8 = 4;
    /// East (+x).
    pub const EAST: u8 = 5;
}

/// A block hit by a ray (`MovingObjectPosition` of type `BLOCK`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// The block.
    pub pos: (i32, i32, i32),
    /// The face that was hit.
    pub face: u8,
    /// Where the ray hit it.
    pub vec: Vec3,
}

type Bounds = ([f32; 3], [f32; 3]);

const FULL: Bounds = ([0.0; 3], [1.0; 3]);

/// A box centered horizontally: `(0.5 - r, 0, 0.5 - r) .. (0.5 + r, height, 0.5 + r)`.
fn centered(r: f32, height: f32) -> Bounds {
    ([0.5 - r, 0.0, 0.5 - r], [0.5 + r, height, 0.5 + r])
}

fn flat(height: f32) -> Bounds {
    ([0.0; 3], [1.0, height, 1.0])
}

/// The block bounds `collisionRayTrace` uses (`setBlockBoundsBasedOnState`, or the
/// bounds the block sets in `collisionRayTrace` itself).
fn bounds(state: u16) -> Bounds {
    let (id, meta) = (state >> 4, state & 15);
    match id {
        block::YELLOW_FLOWER | block::RED_FLOWER => centered(0.2, 0.2 * 3.0),
        block::TALLGRASS | block::DEADBUSH => centered(0.4, 0.8),
        block::SAPLING => centered(0.4, 0.4 * 2.0),
        block::BROWN_MUSHROOM | block::RED_MUSHROOM => centered(0.2, 0.2 * 2.0),
        block::WHEAT | block::CARROTS | block::POTATOES | block::NETHER_WART => centered(0.5, 0.25),
        block::REEDS => centered(0.375, 1.0),
        block::PUMPKIN_STEM | block::MELON_STEM => {
            centered(0.125, (f32::from(meta) * 2.0 + 2.0) / 16.0)
        }
        block::TORCH | block::REDSTONE_TORCH | block::UNLIT_REDSTONE_TORCH => {
            let r = 0.15f32;
            match meta {
                1 => ([0.0, 0.2, 0.5 - r], [r * 2.0, 0.8, 0.5 + r]),
                2 => ([1.0 - r * 2.0, 0.2, 0.5 - r], [1.0, 0.8, 0.5 + r]),
                3 => ([0.5 - r, 0.2, 0.0], [0.5 + r, 0.8, r * 2.0]),
                4 => ([0.5 - r, 0.2, 1.0 - r * 2.0], [0.5 + r, 0.8, 1.0]),
                _ => centered(0.1, 0.6),
            }
        }
        block::LADDER => {
            let d = 0.125f32;
            match meta {
                2 => ([0.0, 0.0, 1.0 - d], [1.0, 1.0, 1.0]),
                3 => ([0.0; 3], [1.0, 1.0, d]),
                4 => ([1.0 - d, 0.0, 0.0], [1.0, 1.0, 1.0]),
                _ => ([0.0; 3], [d, 1.0, 1.0]),
            }
        }
        block::SNOW_LAYER => flat(f32::from((meta & 7) + 1) / 8.0),
        block::CARPET => flat(0.0625),
        block::REDSTONE_WIRE => flat(0.0625),
        block::RAIL | block::GOLDEN_RAIL | block::DETECTOR_RAIL | block::ACTIVATOR_RAIL => {
            // Ascending rails (shapes 2-5) are taller.
            let shape = if id == block::RAIL { meta } else { meta & 7 };
            flat(if (2..=5).contains(&shape) {
                0.625
            } else {
                0.125
            })
        }
        block::STONE_SLAB | block::WOODEN_SLAB | block::STONE_SLAB2 => {
            if meta & 8 != 0 {
                ([0.0, 0.5, 0.0], [1.0; 3])
            } else {
                flat(0.5)
            }
        }
        _ => {
            // TODO(M5): the remaining shaped blocks (stairs, fences, doors, ...) use
            // their first collision box, or a full cube.
            match super::physics::block_boxes(state, (0, 0, 0)).first() {
                Some(b) => (
                    [b.min[0] as f32, b.min[1] as f32, b.min[2] as f32],
                    [b.max[0] as f32, b.max[1] as f32, b.max[2] as f32],
                ),
                None => FULL,
            }
        }
    }
}

/// `Block.canCollideCheck`: liquids only count as a source when asked to; air and fire
/// never.
fn can_collide(state: u16, hit_liquids: bool) -> bool {
    let id = state >> 4;
    match material(id) {
        Material::Water | Material::Lava => hit_liquids && state & 15 == 0,
        _ => !matches!(id, block::AIR | block::FIRE),
    }
}

fn sq_dist(a: Vec3, b: Vec3) -> f64 {
    let d = (b.0 - a.0, b.1 - a.1, b.2 - a.2);
    d.0 * d.0 + d.1 * d.1 + d.2 * d.2
}

/// `Vec3.getIntermediateWith{X,Y,Z}Value`: where the segment crosses `value` on `axis`.
fn intermediate(from: Vec3, to: Vec3, axis: usize, value: f64) -> Option<Vec3> {
    let f = [from.0, from.1, from.2];
    let d = [to.0 - from.0, to.1 - from.1, to.2 - from.2];
    if d[axis] * d[axis] < MIN_AXIS_LENGTH {
        return None;
    }
    let t = (value - f[axis]) / d[axis];
    (0.0..=1.0)
        .contains(&t)
        .then(|| (f[0] + d[0] * t, f[1] + d[1] * t, f[2] + d[2] * t))
}

/// `AxisAlignedBB.calculateIntercept`: where the segment from `start` to `end` first
/// enters `bb` (the closest crossing of its six faces), if it does.
pub fn intercept(bb: &super::physics::Aabb, start: Vec3, end: Vec3) -> Option<Vec3> {
    let within = |v: Vec3, a: usize, b: usize| {
        let v = [v.0, v.1, v.2];
        v[a] >= bb.min[a] && v[a] <= bb.max[a] && v[b] >= bb.min[b] && v[b] <= bb.max[b]
    };
    let mut best: Option<Vec3> = None;
    for axis in 0..3 {
        let (a, b) = match axis {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        for value in [bb.min[axis], bb.max[axis]] {
            if let Some(v) = intermediate(start, end, axis, value).filter(|&v| within(v, a, b))
                && best.is_none_or(|b| sq_dist(start, v) < sq_dist(start, b))
            {
                best = Some(v);
            }
        }
    }
    best
}

/// `Block.collisionRayTrace` against the block's bounds.
fn collision_ray_trace(state: u16, pos: (i32, i32, i32), start: Vec3, end: Vec3) -> Option<Hit> {
    let (lo, hi) = bounds(state);
    let lo = [f64::from(lo[0]), f64::from(lo[1]), f64::from(lo[2])];
    let hi = [f64::from(hi[0]), f64::from(hi[1]), f64::from(hi[2])];
    let o = (f64::from(pos.0), f64::from(pos.1), f64::from(pos.2));
    let s = (start.0 - o.0, start.1 - o.1, start.2 - o.2);
    let e = (end.0 - o.0, end.1 - o.1, end.2 - o.2);
    let inside = |v: Vec3, a: usize, b: usize| {
        let v = [v.0, v.1, v.2];
        v[a] >= lo[a] && v[a] <= hi[a] && v[b] >= lo[b] && v[b] <= hi[b]
    };
    // Candidates in vanilla's order; a later one wins only when strictly closer.
    let candidates = [
        (
            intermediate(s, e, 0, lo[0]).filter(|&v| inside(v, 1, 2)),
            face::WEST,
        ),
        (
            intermediate(s, e, 0, hi[0]).filter(|&v| inside(v, 1, 2)),
            face::EAST,
        ),
        (
            intermediate(s, e, 1, lo[1]).filter(|&v| inside(v, 0, 2)),
            face::DOWN,
        ),
        (
            intermediate(s, e, 1, hi[1]).filter(|&v| inside(v, 0, 2)),
            face::UP,
        ),
        (
            intermediate(s, e, 2, lo[2]).filter(|&v| inside(v, 0, 1)),
            face::NORTH,
        ),
        (
            intermediate(s, e, 2, hi[2]).filter(|&v| inside(v, 0, 1)),
            face::SOUTH,
        ),
    ];
    let mut best: Option<(Vec3, u8)> = None;
    for (v, f) in candidates {
        if let Some(v) = v
            && best.is_none_or(|(b, _)| sq_dist(s, v) < sq_dist(s, b))
        {
            best = Some((v, f));
        }
    }
    best.map(|(v, f)| Hit {
        pos,
        face: f,
        vec: (v.0 + o.0, v.1 + o.1, v.2 + o.2),
    })
}

/// `World.rayTraceBlocks(start, end, stopOnLiquid, ignoreBlockWithoutBoundingBox,
/// false)`: the first block hit on the way from `start` to `end`.
pub fn ray_trace_blocks(
    world: &mut World,
    mut start: Vec3,
    end: Vec3,
    stop_on_liquid: bool,
    ignore_no_box: bool,
) -> Option<Hit> {
    let nan = |v: Vec3| v.0.is_nan() || v.1.is_nan() || v.2.is_nan();
    if nan(start) || nan(end) {
        return None;
    }
    let floor = krol_java::math::floor_f64;
    let target = (floor(end.0), floor(end.1), floor(end.2));
    let mut cur = (floor(start.0), floor(start.1), floor(start.2));
    let check = |world: &mut World, p: (i32, i32, i32), start: Vec3| {
        let state = world.block(p.0, p.1, p.2);
        let has_box = !super::physics::block_boxes(state, p).is_empty();
        if (!ignore_no_box || has_box) && can_collide(state, stop_on_liquid) {
            collision_ray_trace(state, p, start, end)
        } else {
            None
        }
    };
    if let Some(hit) = check(world, cur, start) {
        return Some(hit);
    }
    let mut steps = MAX_STEPS;
    while steps >= 0 {
        steps -= 1;
        if nan(start) || cur == target {
            return None;
        }
        let next_plane = |from: i32, to: i32| {
            if to > from {
                Some(f64::from(from) + 1.0)
            } else if to < from {
                Some(f64::from(from))
            } else {
                None
            }
        };
        let planes = [
            next_plane(cur.0, target.0),
            next_plane(cur.1, target.1),
            next_plane(cur.2, target.2),
        ];
        let s = [start.0, start.1, start.2];
        let d = [end.0 - start.0, end.1 - start.1, end.2 - start.2];
        let mut t = [NO_STEP; 3];
        for a in 0..3 {
            if let Some(p) = planes[a] {
                t[a] = (p - s[a]) / d[a];
            }
            // Vanilla compares with == -0.0, which also matches +0.0.
            if t[a] == 0.0 {
                t[a] = NEGATIVE_ZERO_STEP;
            }
        }
        let f = if t[0] < t[1] && t[0] < t[2] {
            start = (
                planes[0].unwrap_or(NO_STEP),
                s[1] + d[1] * t[0],
                s[2] + d[2] * t[0],
            );
            if target.0 > cur.0 {
                face::WEST
            } else {
                face::EAST
            }
        } else if t[1] < t[2] {
            start = (
                s[0] + d[0] * t[1],
                planes[1].unwrap_or(NO_STEP),
                s[2] + d[2] * t[1],
            );
            if target.1 > cur.1 {
                face::DOWN
            } else {
                face::UP
            }
        } else {
            start = (
                s[0] + d[0] * t[2],
                s[1] + d[1] * t[2],
                planes[2].unwrap_or(NO_STEP),
            );
            if target.2 > cur.2 {
                face::NORTH
            } else {
                face::SOUTH
            }
        };
        cur = (
            floor(start.0) - i32::from(f == face::EAST),
            floor(start.1) - i32::from(f == face::UP),
            floor(start.2) - i32::from(f == face::SOUTH),
        );
        if let Some(hit) = check(world, cur, start) {
            return Some(hit);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_block_faces() {
        let stone = block::STONE << 4;
        let hit = collision_ray_trace(stone, (0, 0, 0), (0.5, 2.0, 0.5), (0.5, -1.0, 0.5));
        assert_eq!(
            hit.map(|h| (h.face, h.vec)),
            Some((face::UP, (0.5, 1.0, 0.5)))
        );
        let hit = collision_ray_trace(stone, (3, 0, 0), (0.0, 0.5, 0.5), (5.0, 0.5, 0.5));
        assert_eq!(hit.map(|h| h.face), Some(face::WEST));
        assert!(collision_ray_trace(stone, (0, 0, 0), (2.0, 2.0, 2.0), (3.0, 3.0, 3.0)).is_none());
    }

    #[test]
    fn liquids_only_as_sources() {
        assert!(can_collide(block::WATER << 4, true));
        assert!(!can_collide(block::WATER << 4 | 1, true));
        assert!(!can_collide(block::WATER << 4, false));
        assert!(!can_collide(0, true));
    }
}
