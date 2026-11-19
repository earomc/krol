//! Block collision shapes (vanilla `Block.addCollisionBoxesToList` /
//! `getCollisionBoundingBox` and their overrides): slabs, stairs with their corners,
//! fences, walls and panes connecting to their neighbors, doors, trapdoors, gates,
//! ladders, containers with walls (cauldrons, hoppers) and the other shaped blocks.
//!
//! Vanilla stores block bounds as floats in double fields, so the local bounds here are
//! `f32` and widen exactly. A few blocks compute the box in float from the block
//! position (cactus, cake, fence gates): far from the origin that loses precision, and
//! so does krol.
//!
//! TODO(M5): blocks whose collision box is whatever bounds a ray trace last left on
//! the shared block object (anvils, double chests) use their constructor bounds.

use super::block_data::{Material, material};
use super::ids::block;
use super::physics::Aabb;

/// A box in block-local coordinates: `(min, max)`.
type Local = ([f32; 3], [f32; 3]);

/// Reads the block state at a position (the neighbors some shapes depend on).
pub type Neighbors<'a> = dyn FnMut((i32, i32, i32)) -> u16 + 'a;

/// Blocks without a collision box (`getCollisionBoundingBox` returns null, or an empty
/// `addCollisionBoxesToList`).
pub fn passable(id: u16) -> bool {
    matches!(
        id,
        0 | 6 | 8..=11 | 27 | 28 | 30..=32 | 36..=40 | 50 | 51 | 55 | 59 | 63 | 66 | 68..=70
            | 72 | 75..=77 | 83 | 90 | 104..=106 | 115 | 119 | 131 | 132 | 141..=143 | 147
            | 148 | 157 | 175..=177
    )
}

/// `EnumFacing` as krol uses it here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Facing {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

/// `EnumFacing.getFront`.
fn front(index: u16) -> Facing {
    match index {
        0 => Facing::Down,
        1 => Facing::Up,
        2 => Facing::North,
        3 => Facing::South,
        4 => Facing::West,
        _ => Facing::East,
    }
}

/// `EnumFacing.getHorizontal`.
fn horizontal(index: u16) -> Facing {
    match index & 3 {
        0 => Facing::South,
        1 => Facing::West,
        2 => Facing::North,
        _ => Facing::East,
    }
}

const FULL: Local = ([0.0; 3], [1.0; 3]);

fn up_to(top: f32) -> Local {
    ([0.0; 3], [1.0, top, 1.0])
}

/// Adds the collision boxes of `state` at `pos` that intersect `mask`; `neighbors` reads
/// the blocks around it.
pub fn add_block_boxes(
    state: u16,
    pos: (i32, i32, i32),
    mask: &Aabb,
    out: &mut Vec<Aabb>,
    neighbors: &mut Neighbors<'_>,
) {
    let id = state >> 4;
    if passable(id) {
        return;
    }
    let mut push_world = |b: Aabb| {
        if mask.intersects(&b) {
            out.push(b);
        }
    };
    // Boxes computed in float from the block position.
    if let Some(b) = float_box(state, pos) {
        if let Some(b) = b {
            push_world(b);
        }
        return;
    }
    let (x, y, z) = (f64::from(pos.0), f64::from(pos.1), f64::from(pos.2));
    for (min, max) in local_boxes(state, pos, neighbors) {
        push_world(Aabb::new(
            [
                x + f64::from(min[0]),
                y + f64::from(min[1]),
                z + f64::from(min[2]),
            ],
            [
                x + f64::from(max[0]),
                y + f64::from(max[1]),
                z + f64::from(max[2]),
            ],
        ));
    }
}

/// Collision boxes that vanilla builds from float sums with the block position:
/// `Some(None)` for none, `None` for blocks with local boxes.
fn float_box(state: u16, pos: (i32, i32, i32)) -> Option<Option<Aabb>> {
    let (id, meta) = (state >> 4, state & 15);
    let (fx, fy, fz) = (pos.0 as f32, pos.1 as f32, pos.2 as f32);
    let y = f64::from(pos.1);
    Some(match id {
        // BlockCactus.
        block::CACTUS => {
            let f = 0.0625f32;
            Some(Aabb::new(
                [f64::from(fx + f), y, f64::from(fz + f)],
                [
                    f64::from((pos.0 + 1) as f32 - f),
                    f64::from((pos.1 + 1) as f32 - f),
                    f64::from((pos.2 + 1) as f32 - f),
                ],
            ))
        }
        // BlockCake: shrinks from the west with every bite.
        super::farming::CAKE => {
            let f = 0.0625f32;
            let bitten = (1 + meta * 2) as f32 / 16.0;
            Some(Aabb::new(
                [f64::from(fx + bitten), y, f64::from(fz + f)],
                [
                    f64::from((pos.0 + 1) as f32 - f),
                    f64::from(fy + 0.5),
                    f64::from((pos.2 + 1) as f32 - f),
                ],
            ))
        }
        // BlockFenceGate: nothing when open, else a 1.5 high plank along its axis.
        107 | 183..=187 => {
            if meta & 4 != 0 {
                None
            } else if matches!(horizontal(meta), Facing::North | Facing::South) {
                Some(Aabb::new(
                    [f64::from(pos.0), y, f64::from(fz + 0.375)],
                    [
                        f64::from(pos.0 + 1),
                        f64::from(fy + 1.5),
                        f64::from(fz + 0.625),
                    ],
                ))
            } else {
                Some(Aabb::new(
                    [f64::from(fx + 0.375), y, f64::from(pos.2)],
                    [
                        f64::from(fx + 0.625),
                        f64::from(fy + 1.5),
                        f64::from(pos.2 + 1),
                    ],
                ))
            }
        }
        _ => return None,
    })
}

fn local_boxes(state: u16, pos: (i32, i32, i32), neighbors: &mut Neighbors<'_>) -> Vec<Local> {
    let (id, meta) = (state >> 4, state & 15);
    let one = |b: Local| vec![b];
    match id {
        // Slabs: the top or bottom half.
        44 | 126 | 182 => one(if meta & 8 != 0 {
            ([0.0, 0.5, 0.0], [1.0; 3])
        } else {
            up_to(0.5)
        }),
        // BlockSnow: one layer lower than it looks.
        block::SNOW_LAYER => one(up_to(f32::from(meta & 7) * 0.125)),
        block::SOUL_SAND => one(up_to(1.0 - 0.125)),
        block::CARPET => one(up_to(0.0625)),
        // BlockRedstoneDiode.
        93 | 94 | 149 | 150 => one(up_to(0.125)),
        // Chests (constructor bounds, see the module docs).
        54 | 130 | 146 => one(([0.0625, 0.0, 0.0625], [0.9375, 0.875, 0.9375])),
        // BlockBed, BlockEnchantmentTable, BlockDaylightDetector, BlockLilyPad.
        26 => one(up_to(0.5625)),
        116 => one(up_to(0.75)),
        151 | 178 => one(up_to(0.375)),
        111 => one(up_to(0.015625)),
        // BlockFlowerPot, BlockDragonEgg.
        140 => one(([0.3125, 0.0, 0.3125], [0.6875, 0.375, 0.6875])),
        122 => one(([0.0625, 0.0, 0.0625], [0.9375, 1.0, 0.9375])),
        // BlockEndPortalFrame: the eye adds a box on top.
        120 => {
            let mut v = vec![up_to(0.8125)];
            if meta & 4 != 0 {
                v.push(([0.3125, 0.8125, 0.3125], [0.6875, 1.0, 0.6875]));
            }
            v
        }
        // BlockCauldron and BlockHopper: a floor and four walls.
        118 | 154 => {
            let w = 0.125;
            vec![
                up_to(if id == 118 { 0.3125 } else { 0.625 }),
                ([0.0; 3], [w, 1.0, 1.0]),
                ([0.0; 3], [1.0, 1.0, w]),
                ([1.0 - w, 0.0, 0.0], [1.0; 3]),
                ([0.0, 0.0, 1.0 - w], [1.0; 3]),
            ]
        }
        // BlockBrewingStand: the rod and the base.
        117 => vec![
            ([0.4375, 0.0, 0.4375], [0.5625, 0.875, 0.5625]),
            up_to(0.125),
        ],
        // BlockLadder: against the block behind it.
        65 => one(match front(meta) {
            Facing::South => ([0.0; 3], [1.0, 1.0, 0.125]),
            Facing::West => ([0.875, 0.0, 0.0], [1.0; 3]),
            Facing::East => ([0.0; 3], [0.125, 1.0, 1.0]),
            _ => ([0.0, 0.0, 0.875], [1.0; 3]),
        }),
        // BlockTrapDoor.
        96 | 167 => one(trapdoor(meta)),
        // BlockDoor (the halves read each other).
        64 | 71 | 193..=197 => {
            let (lower, upper) = if meta & 8 != 0 {
                (neighbors((pos.0, pos.1 - 1, pos.2)) & 15, meta)
            } else {
                (meta, neighbors((pos.0, pos.1 + 1, pos.2)) & 15)
            };
            one(door(lower, upper & 1 != 0))
        }
        // BlockSkull.
        144 => one(match front(meta & 7) {
            Facing::North => ([0.25, 0.25, 0.5], [0.75, 0.75, 1.0]),
            Facing::South => ([0.25, 0.25, 0.0], [0.75, 0.75, 0.5]),
            Facing::West => ([0.5, 0.25, 0.25], [1.0, 0.75, 0.75]),
            Facing::East => ([0.0, 0.25, 0.25], [0.5, 0.75, 0.75]),
            _ => ([0.25, 0.0, 0.25], [0.75, 0.5, 0.75]),
        }),
        // BlockCocoa: grows with its age, against its log.
        block::COCOA => one(cocoa(meta)),
        // BlockPistonExtension: the head plate and the arm.
        block::PISTON_HEAD => piston_head(front(meta & 7)),
        // BlockFence: one box per axis (1.5 high).
        85 | 113 | 188..=192 => fence(id, pos, neighbors),
        // BlockWall: a single box (1.5 high).
        139 => one(wall(pos, neighbors)),
        // BlockPane (glass panes, iron bars).
        101 | 102 | 160 => pane(id, pos, neighbors),
        // BlockStairs.
        53 | 67 | 108 | 109 | 114 | 128 | 134..=136 | 156 | 163 | 164 | 180 => {
            stairs(state, pos, neighbors)
        }
        _ => one(FULL),
    }
}

/// `BlockTrapDoor.setBounds`.
fn trapdoor(meta: u16) -> Local {
    let t = 0.1875;
    if meta & 4 != 0 {
        return match meta & 3 {
            0 => ([0.0, 0.0, 1.0 - t], [1.0; 3]),
            1 => ([0.0; 3], [1.0, 1.0, t]),
            2 => ([1.0 - t, 0.0, 0.0], [1.0; 3]),
            _ => ([0.0; 3], [t, 1.0, 1.0]),
        };
    }
    if meta & 8 != 0 {
        ([0.0, 1.0 - t, 0.0], [1.0; 3])
    } else {
        up_to(t)
    }
}

/// `BlockDoor.setBoundBasedOnMeta`: `lower` holds the facing and the open bit, the
/// upper half's metadata the hinge.
fn door(lower: u16, hinge_left: bool) -> Local {
    let t = 0.1875;
    // getHorizontal(meta & 3).rotateYCCW().
    let facing = match lower & 3 {
        0 => Facing::East,
        1 => Facing::South,
        2 => Facing::West,
        _ => Facing::North,
    };
    let north_edge = ([0.0; 3], [1.0, 1.0, t]);
    let south_edge = ([0.0, 0.0, 1.0 - t], [1.0; 3]);
    let west_edge = ([0.0; 3], [t, 1.0, 1.0]);
    let east_edge = ([1.0 - t, 0.0, 0.0], [1.0; 3]);
    if lower & 4 != 0 {
        match (facing, hinge_left) {
            (Facing::East, false) | (Facing::West, true) => north_edge,
            (Facing::East, true) | (Facing::West, false) => south_edge,
            (Facing::South, false) | (Facing::North, true) => east_edge,
            _ => west_edge,
        }
    } else {
        match facing {
            Facing::East => west_edge,
            Facing::South => north_edge,
            Facing::West => east_edge,
            _ => south_edge,
        }
    }
}

/// `BlockCocoa.setBlockBoundsBasedOnState`.
fn cocoa(meta: u16) -> Local {
    let age = f32::from(meta >> 2);
    let width = 4.0 + age * 2.0;
    let height = 5.0 + age * 2.0;
    let half = width / 2.0;
    let (y0, y1) = ((12.0 - height) / 16.0, 0.75);
    match horizontal(meta) {
        Facing::South => (
            [(8.0 - half) / 16.0, y0, (15.0 - width) / 16.0],
            [(8.0 + half) / 16.0, y1, 0.9375],
        ),
        Facing::North => (
            [(8.0 - half) / 16.0, y0, 0.0625],
            [(8.0 + half) / 16.0, y1, (1.0 + width) / 16.0],
        ),
        Facing::West => (
            [0.0625, y0, (8.0 - half) / 16.0],
            [(1.0 + width) / 16.0, y1, (8.0 + half) / 16.0],
        ),
        _ => (
            [(15.0 - width) / 16.0, y0, (8.0 - half) / 16.0],
            [0.9375, y1, (8.0 + half) / 16.0],
        ),
    }
}

/// `BlockPistonExtension.applyHeadBounds` + `applyCoreBounds`.
fn piston_head(facing: Facing) -> Vec<Local> {
    let (head, arm) = match facing {
        Facing::Down => (up_to(0.25), ([0.375, 0.25, 0.375], [0.625, 1.0, 0.625])),
        Facing::Up => (
            ([0.0, 0.75, 0.0], [1.0; 3]),
            ([0.375, 0.0, 0.375], [0.625, 0.75, 0.625]),
        ),
        Facing::North => (
            ([0.0; 3], [1.0, 1.0, 0.25]),
            ([0.25, 0.375, 0.25], [0.75, 0.625, 1.0]),
        ),
        Facing::South => (
            ([0.0, 0.0, 0.75], [1.0; 3]),
            ([0.25, 0.375, 0.0], [0.75, 0.625, 0.75]),
        ),
        Facing::West => (
            ([0.0; 3], [0.25, 1.0, 1.0]),
            // Vanilla quirk: the west arm runs north-south instead of east-west.
            ([0.375, 0.25, 0.25], [0.625, 0.75, 1.0]),
        ),
        Facing::East => (
            ([0.75, 0.0, 0.0], [1.0; 3]),
            ([0.0, 0.375, 0.25], [0.75, 0.625, 0.75]),
        ),
    };
    vec![head, arm]
}

fn offset(p: (i32, i32, i32), f: Facing) -> (i32, i32, i32) {
    match f {
        Facing::Down => (p.0, p.1 - 1, p.2),
        Facing::Up => (p.0, p.1 + 1, p.2),
        Facing::North => (p.0, p.1, p.2 - 1),
        Facing::South => (p.0, p.1, p.2 + 1),
        Facing::West => (p.0 - 1, p.1, p.2),
        Facing::East => (p.0 + 1, p.1, p.2),
    }
}

fn is_gate(id: u16) -> bool {
    matches!(id, 107 | 183..=187)
}

fn is_fence(id: u16) -> bool {
    matches!(id, 85 | 113 | 188..=192)
}

/// What fences and walls connect to besides their own kind: opaque full cubes that
/// are not gourds (`Material.isOpaque && isFullCube`, barriers excepted).
fn solid_neighbor(id: u16) -> bool {
    let m = material(id);
    id != block::BARRIER
        && m.is_opaque()
        && super::light_data::full_cube(id)
        && m != Material::Gourd
}

/// `BlockFence.canConnectTo`.
fn fence_connects(fence: u16, other: u16) -> bool {
    if other == block::BARRIER {
        return false;
    }
    if (is_fence(other) && material(other) == material(fence)) || is_gate(other) {
        return true;
    }
    solid_neighbor(other)
}

/// The four horizontal neighbors' connections: `(north, south, west, east)`.
fn connections(
    pos: (i32, i32, i32),
    neighbors: &mut Neighbors<'_>,
    connects: impl Fn(u16) -> bool,
) -> (bool, bool, bool, bool) {
    let mut c = |f| connects(neighbors(offset(pos, f)) >> 4);
    (
        c(Facing::North),
        c(Facing::South),
        c(Facing::West),
        c(Facing::East),
    )
}

/// `BlockFence.addCollisionBoxesToList`: a north-south box if it connects that way, and
/// an east-west box if it connects that way or not at all.
fn fence(id: u16, pos: (i32, i32, i32), neighbors: &mut Neighbors<'_>) -> Vec<Local> {
    let (n, s, w, e) = connections(pos, neighbors, |o| fence_connects(id, o));
    let mut v = Vec::new();
    if n || s {
        let z0 = if n { 0.0 } else { 0.375 };
        let z1 = if s { 1.0 } else { 0.625 };
        v.push(([0.375, 0.0, z0], [0.625, 1.5, z1]));
    }
    if w || e || (!n && !s) {
        let x0 = if w { 0.0 } else { 0.375 };
        let x1 = if e { 1.0 } else { 0.625 };
        v.push(([x0, 0.0, 0.375], [x1, 1.5, 0.625]));
    }
    v
}

/// `BlockWall.setBlockBoundsBasedOnState`, raised to 1.5 by `getCollisionBoundingBox`.
fn wall(pos: (i32, i32, i32), neighbors: &mut Neighbors<'_>) -> Local {
    let connects = |o: u16| o != block::BARRIER && (o == 139 || is_gate(o) || solid_neighbor(o));
    let (n, s, w, e) = connections(pos, neighbors, connects);
    let mut min = [0.25, 0.0, 0.25];
    let mut max = [0.75, 1.5, 0.75];
    if n {
        min[2] = 0.0;
    }
    if s {
        max[2] = 1.0;
    }
    if w {
        min[0] = 0.0;
    }
    if e {
        max[0] = 1.0;
    }
    // A straight wall is thinner (its height is overridden to 1.5 anyway).
    if n && s && !w && !e {
        min[0] = 0.3125;
        max[0] = 0.6875;
    } else if !n && !s && w && e {
        min[2] = 0.3125;
        max[2] = 0.6875;
    }
    (min, max)
}

/// `BlockPane.canPaneConnectToBlock`.
fn pane_connects(pane: u16, other: u16) -> bool {
    super::light_data::full_block(other)
        || other == pane
        || matches!(other, 20 | 95 | 101 | 102 | 160)
}

/// `BlockPane.addCollisionBoxesToList`.
fn pane(id: u16, pos: (i32, i32, i32), neighbors: &mut Neighbors<'_>) -> Vec<Local> {
    let (n, s, w, e) = connections(pos, neighbors, |o| pane_connects(id, o));
    let any = n || s || w || e;
    let (z0, z1) = (0.4375, 0.5625);
    let mut v = Vec::new();
    if (!w || !e) && any {
        if w {
            v.push(([0.0, 0.0, z0], [0.5, 1.0, z1]));
        } else if e {
            v.push(([0.5, 0.0, z0], [1.0, 1.0, z1]));
        }
    } else {
        v.push(([0.0, 0.0, z0], [1.0, 1.0, z1]));
    }
    if (!n || !s) && any {
        if n {
            v.push(([z0, 0.0, 0.0], [z1, 1.0, 0.5]));
        } else if s {
            v.push(([z0, 0.0, 0.5], [z1, 1.0, 1.0]));
        }
    } else {
        v.push(([z0, 0.0, 0.0], [z1, 1.0, 1.0]));
    }
    v
}

pub(super) fn is_stairs(id: u16) -> bool {
    matches!(
        id,
        53 | 67 | 108 | 109 | 114 | 128 | 134..=136 | 156 | 163 | 164 | 180
    )
}

/// A stair's facing (`5 - (meta & 3)`: east, west, south, north) and top half.
fn stair_facing(meta: u16) -> Facing {
    match meta & 3 {
        0 => Facing::East,
        1 => Facing::West,
        2 => Facing::South,
        _ => Facing::North,
    }
}

/// `BlockStairs.addCollisionBoxesToList`: the half slab, the upper step (shortened at
/// outer corners, `func_176306_h`) and the extra quarter of inner corners
/// (`func_176304_i`).
fn stairs(state: u16, pos: (i32, i32, i32), neighbors: &mut Neighbors<'_>) -> Vec<Local> {
    let meta = state & 15;
    let top = meta & 4 != 0;
    let facing = stair_facing(meta);
    // A stair with the same half at `p`, with its facing.
    let mut stair_at = |p: (i32, i32, i32)| {
        let s = neighbors(p);
        (is_stairs(s >> 4) && (s & 4 != 0) == top).then(|| stair_facing(s & 15))
    };
    let same = |p: (i32, i32, i32), stair_at: &mut dyn FnMut((i32, i32, i32)) -> Option<Facing>| {
        stair_at(p) == Some(facing)
    };
    let slab = if top {
        ([0.0, 0.5, 0.0], [1.0; 3])
    } else {
        up_to(0.5)
    };
    let (y0, y1) = if top { (0.0, 0.5) } else { (0.5, 1.0) };
    let mut boxes = vec![slab];

    // func_176306_h: the upper step.
    let (mut x0, mut x1, mut z0, mut z1) = (0.0, 1.0, 0.0, 0.5);
    let mut full_step = true;
    match facing {
        Facing::East => {
            x0 = 0.5;
            z1 = 1.0;
            match stair_at(offset(pos, Facing::East)) {
                Some(Facing::North) if !same(offset(pos, Facing::South), &mut stair_at) => {
                    z1 = 0.5;
                    full_step = false;
                }
                Some(Facing::South) if !same(offset(pos, Facing::North), &mut stair_at) => {
                    z0 = 0.5;
                    full_step = false;
                }
                _ => {}
            }
        }
        Facing::West => {
            x1 = 0.5;
            z1 = 1.0;
            match stair_at(offset(pos, Facing::West)) {
                Some(Facing::North) if !same(offset(pos, Facing::South), &mut stair_at) => {
                    z1 = 0.5;
                    full_step = false;
                }
                Some(Facing::South) if !same(offset(pos, Facing::North), &mut stair_at) => {
                    z0 = 0.5;
                    full_step = false;
                }
                _ => {}
            }
        }
        Facing::South => {
            z0 = 0.5;
            z1 = 1.0;
            match stair_at(offset(pos, Facing::South)) {
                Some(Facing::West) if !same(offset(pos, Facing::East), &mut stair_at) => {
                    x1 = 0.5;
                    full_step = false;
                }
                Some(Facing::East) if !same(offset(pos, Facing::West), &mut stair_at) => {
                    x0 = 0.5;
                    full_step = false;
                }
                _ => {}
            }
        }
        _ => match stair_at(offset(pos, Facing::North)) {
            Some(Facing::West) if !same(offset(pos, Facing::East), &mut stair_at) => {
                x1 = 0.5;
                full_step = false;
            }
            Some(Facing::East) if !same(offset(pos, Facing::West), &mut stair_at) => {
                x0 = 0.5;
                full_step = false;
            }
            _ => {}
        },
    }
    boxes.push(([x0, y0, z0], [x1, y1, z1]));

    // func_176304_i: the inner corner's extra quarter.
    if full_step {
        let (mut x0, mut x1, mut z0, mut z1) = (0.0, 0.5, 0.5, 1.0);
        let mut corner = false;
        match facing {
            Facing::East => match stair_at(offset(pos, Facing::West)) {
                Some(Facing::North) if !same(offset(pos, Facing::North), &mut stair_at) => {
                    z0 = 0.0;
                    z1 = 0.5;
                    corner = true;
                }
                Some(Facing::South) if !same(offset(pos, Facing::South), &mut stair_at) => {
                    corner = true;
                }
                _ => {}
            },
            Facing::West => {
                if let Some(f) = stair_at(offset(pos, Facing::East)) {
                    x0 = 0.5;
                    x1 = 1.0;
                    match f {
                        Facing::North if !same(offset(pos, Facing::North), &mut stair_at) => {
                            z0 = 0.0;
                            z1 = 0.5;
                            corner = true;
                        }
                        Facing::South if !same(offset(pos, Facing::South), &mut stair_at) => {
                            corner = true;
                        }
                        _ => {}
                    }
                }
            }
            Facing::South => {
                if let Some(f) = stair_at(offset(pos, Facing::North)) {
                    z0 = 0.0;
                    z1 = 0.5;
                    match f {
                        Facing::West if !same(offset(pos, Facing::West), &mut stair_at) => {
                            corner = true;
                        }
                        Facing::East if !same(offset(pos, Facing::East), &mut stair_at) => {
                            x0 = 0.5;
                            x1 = 1.0;
                            corner = true;
                        }
                        _ => {}
                    }
                }
            }
            _ => match stair_at(offset(pos, Facing::South)) {
                Some(Facing::West) if !same(offset(pos, Facing::West), &mut stair_at) => {
                    corner = true;
                }
                Some(Facing::East) if !same(offset(pos, Facing::East), &mut stair_at) => {
                    x0 = 0.5;
                    x1 = 1.0;
                    corner = true;
                }
                _ => {}
            },
        }
        if corner {
            boxes.push(([x0, y0, z0], [x1, y1, z1]));
        }
    }
    boxes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxes_with(world: &[((i32, i32, i32), u16)], state: u16) -> Vec<Local> {
        let mut neighbors =
            |p: (i32, i32, i32)| world.iter().find(|(q, _)| *q == p).map_or(0, |(_, s)| *s);
        local_boxes(state, (0, 0, 0), &mut neighbors)
    }

    #[test]
    fn lone_fence_is_a_post() {
        assert_eq!(
            boxes_with(&[], 85 << 4),
            vec![([0.375, 0.0, 0.375], [0.625, 1.5, 0.625])]
        );
    }

    #[test]
    fn fence_connects_north_to_stone() {
        let b = boxes_with(&[((0, 0, -1), 1 << 4)], 85 << 4);
        assert_eq!(b, vec![([0.375, 0.0, 0.0], [0.625, 1.5, 0.625])]);
    }

    #[test]
    fn straight_stair_has_slab_and_step() {
        // Bottom stairs facing east: the step on the east half.
        let b = boxes_with(&[], 53 << 4);
        assert_eq!(b, vec![up_to(0.5), ([0.5, 0.5, 0.0], [1.0, 1.0, 1.0])]);
    }

    #[test]
    fn closed_door_facing_east_is_on_the_west_edge() {
        assert_eq!(door(0, false), ([0.0; 3], [0.1875, 1.0, 1.0]));
    }
}
