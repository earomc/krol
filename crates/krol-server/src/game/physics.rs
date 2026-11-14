//! Entity movement through the world: bounding boxes, block collision boxes and
//! `moveEntity` (vanilla: `AxisAlignedBB`, `Block.addCollisionBoxesToList`,
//! `World.getCollidingBoundingBoxes`, `Entity.moveEntity`).
//!
//! Collision shapes are modeled for full cubes, blocks without collision and the common
//! partial blocks (slabs, snow layers, soul sand, cactus, farmland, carpet, chests,
//! beds, enchanting tables, daylight sensors, lily pads, end portal frames).
//! TODO(M5): stairs, fences, walls, panes, gates, doors, trapdoors, ladders and the
//! other neighbor- or state-dependent shapes (approximated below), entity collision
//! boxes (boats, minecarts) and the world border.

use krol_java::math::floor_f64;

use super::blocks::{self, SoundType};
use super::world::World;

/// An axis-aligned bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    /// Minimum corner (x, y, z).
    pub min: [f64; 3],
    /// Maximum corner (x, y, z).
    pub max: [f64; 3],
}

impl Aabb {
    /// A box from its corners.
    pub const fn new(min: [f64; 3], max: [f64; 3]) -> Self {
        Self { min, max }
    }

    /// A box of `width` x `height` standing at `pos` (vanilla `setPosition`; the half
    /// width is computed in `float`).
    pub fn at(pos: (f64, f64, f64), width: f32, height: f32) -> Self {
        let half = f64::from(width / 2.0);
        Self::new(
            [pos.0 - half, pos.1, pos.2 - half],
            [pos.0 + half, pos.1 + f64::from(height), pos.2 + half],
        )
    }

    /// Moved by `d`.
    pub fn offset(&self, d: [f64; 3]) -> Self {
        Self::new(
            [self.min[0] + d[0], self.min[1] + d[1], self.min[2] + d[2]],
            [self.max[0] + d[0], self.max[1] + d[1], self.max[2] + d[2]],
        )
    }

    /// Stretched in the direction of `d` (`addCoord`).
    pub fn add_coord(&self, d: [f64; 3]) -> Self {
        let mut b = *self;
        for (i, d) in d.into_iter().enumerate() {
            if d < 0.0 {
                b.min[i] += d;
            } else if d > 0.0 {
                b.max[i] += d;
            }
        }
        b
    }

    /// Grown by `d` on every side (`expand`).
    pub fn expand(&self, d: [f64; 3]) -> Self {
        Self::new(
            [self.min[0] - d[0], self.min[1] - d[1], self.min[2] - d[2]],
            [self.max[0] + d[0], self.max[1] + d[1], self.max[2] + d[2]],
        )
    }

    /// Shrunk by `d` on every side (`contract`).
    pub fn contract(&self, d: [f64; 3]) -> Self {
        self.expand([-d[0], -d[1], -d[2]])
    }

    /// Whether the boxes overlap (strictly).
    pub fn intersects(&self, o: &Aabb) -> bool {
        (0..3).all(|i| o.max[i] > self.min[i] && o.min[i] < self.max[i])
    }

    /// How far `other` may move by `d` along `axis` before hitting this box
    /// (`calculateXOffset` / `Y` / `Z`).
    pub fn axis_offset(&self, other: &Aabb, axis: usize, mut d: f64) -> f64 {
        let overlaps = (0..3)
            .filter(|&i| i != axis)
            .all(|i| other.max[i] > self.min[i] && other.min[i] < self.max[i]);
        if !overlaps {
            return d;
        }
        if d > 0.0 && other.max[axis] <= self.min[axis] {
            let gap = self.min[axis] - other.max[axis];
            if gap < d {
                d = gap;
            }
        } else if d < 0.0 && other.min[axis] >= self.max[axis] {
            let gap = self.max[axis] - other.min[axis];
            if gap > d {
                d = gap;
            }
        }
        d
    }
}

/// Blocks without a collision box (`getCollisionBoundingBox` returns null, or an empty
/// `addCollisionBoxesToList`).
fn passable(id: u16) -> bool {
    matches!(
        id,
        0 | 6 | 8..=11 | 27 | 28 | 30..=32 | 36..=40 | 50 | 51 | 55 | 59 | 63 | 66 | 68..=70
            | 72 | 75..=77 | 83 | 90 | 104..=106 | 115 | 119 | 131 | 132 | 141..=143 | 147
            | 148 | 157 | 175..=177
    )
}

/// Adds the collision boxes of the block `state` at `pos` that intersect `mask`
/// (`Block.addCollisionBoxesToList`).
fn add_block_boxes(state: u16, pos: (i32, i32, i32), mask: &Aabb, out: &mut Vec<Aabb>) {
    let (id, meta) = (state >> 4, (state & 15) as u8);
    if passable(id) {
        return;
    }
    let (x, y, z) = (f64::from(pos.0), f64::from(pos.1), f64::from(pos.2));
    let mut push = |min: [f64; 3], max: [f64; 3]| {
        let b = Aabb::new(
            [x + min[0], y + min[1], z + min[2]],
            [x + max[0], y + max[1], z + max[2]],
        );
        if mask.intersects(&b) {
            out.push(b);
        }
    };
    let h = |top: f64| ([0.0, 0.0, 0.0], [1.0, top, 1.0]);
    let (min, max) = match id {
        // Slabs: top or bottom half.
        44 | 126 | 182 => {
            if meta & 8 != 0 {
                ([0.0, 0.5, 0.0], [1.0, 1.0, 1.0])
            } else {
                h(0.5)
            }
        }
        // Snow layers: one layer lower than they look.
        78 => h(f64::from(f32::from(meta & 7) * 0.125)),
        // Soul sand.
        88 => h(1.0 - 0.125),
        // Cactus: computed in float from the block position (vanilla quirk: loses
        // precision far from the origin).
        81 => {
            let f = 0.0625f32;
            let b = Aabb::new(
                [f64::from(pos.0 as f32 + f), y, f64::from(pos.2 as f32 + f)],
                [
                    f64::from((pos.0 + 1) as f32 - f),
                    f64::from((pos.1 + 1) as f32 - f),
                    f64::from((pos.2 + 1) as f32 - f),
                ],
            );
            if mask.intersects(&b) {
                out.push(b);
            }
            return;
        }
        171 => h(0.0625),
        // TODO(M5): double chests extend to their neighbor.
        54 | 130 | 146 => ([0.0625, 0.0, 0.0625], [0.9375, 0.875, 0.9375]),
        26 => h(0.5625),
        116 => h(0.75),
        151 | 178 => h(0.375),
        111 => h(0.015625),
        // TODO(M5): the eye adds a box on top.
        120 => h(0.8125),
        // TODO(M5): fences, walls and gates connect to neighbors (1.5 high); modeled as
        // their post only.
        85 | 113 | 188..=192 | 139 => {
            let w = if id == 139 { 0.25 } else { 0.375 };
            ([w, 0.0, w], [1.0 - w, 1.5, 1.0 - w])
        }
        107 | 183..=187 => {
            if meta & 4 != 0 {
                return; // open
            }
            ([0.0, 0.0, 0.375], [1.0, 1.5, 0.625])
        }
        // TODO(M5): stairs are a slab plus one or two quarter blocks; modeled as full.
        _ => h(1.0),
    };
    push(min, max);
}

/// The collision boxes of all blocks touching `bb` (`World.getCollidingBoundingBoxes`
/// without entities). Columns whose chunk is not loaded contribute nothing.
pub fn colliding_boxes(world: &mut World, bb: &Aabb) -> Vec<Aabb> {
    let mut out = Vec::new();
    let (x0, x1) = (floor_f64(bb.min[0]), floor_f64(bb.max[0] + 1.0));
    let (y0, y1) = (floor_f64(bb.min[1]), floor_f64(bb.max[1] + 1.0));
    let (z0, z1) = (floor_f64(bb.min[2]), floor_f64(bb.max[2] + 1.0));
    for x in x0..x1 {
        for z in z0..z1 {
            if !world.is_chunk_loaded(x >> 4, z >> 4) {
                continue;
            }
            for y in y0 - 1..y1 {
                let state = world.block(x, y, z);
                add_block_boxes(state, (x, y, z), bb, &mut out);
            }
        }
    }
    out
}

/// A moving entity's physical state (the fields of vanilla `Entity` used by
/// `moveEntity`).
#[derive(Debug, Clone)]
pub struct Body {
    /// Position (feet).
    pub pos: (f64, f64, f64),
    /// Motion per tick.
    pub motion: (f64, f64, f64),
    /// Bounding box width.
    pub width: f32,
    /// Bounding box height.
    pub height: f32,
    /// How high the entity steps up without jumping.
    pub step_height: f32,
    /// Whether the last move ended on the ground.
    pub on_ground: bool,
    /// Sneaking (players do not walk off edges).
    pub sneaking: bool,
    /// The last move was blocked horizontally.
    pub collided_horizontally: bool,
    /// The last move was blocked vertically.
    pub collided_vertically: bool,
    /// Distance walked, for step sounds (`distanceWalkedOnStepModified`).
    pub distance_walked_on_step: f32,
    /// Distance of the next step sound.
    pub next_step_distance: i32,
}

impl Body {
    /// A player's body at `pos`.
    pub fn player(pos: (f64, f64, f64), on_ground: bool) -> Self {
        Self {
            pos,
            motion: (0.0, 0.0, 0.0),
            width: 0.6,
            height: 1.8,
            step_height: 0.6,
            on_ground,
            sneaking: false,
            collided_horizontally: false,
            collided_vertically: false,
            distance_walked_on_step: 0.0,
            next_step_distance: 1,
        }
    }

    /// The current bounding box.
    pub fn bb(&self) -> Aabb {
        Aabb::at(self.pos, self.width, self.height)
    }
}

/// A step sound to play at the entity (`playStepSound`).
#[derive(Debug, Clone, PartialEq)]
pub struct StepSound {
    /// Sound name.
    pub name: String,
    /// Volume.
    pub volume: f32,
    /// Pitch.
    pub pitch: f32,
}

/// What a move did besides changing the body.
#[derive(Debug, Default)]
pub struct MoveOutcome {
    /// Step sound to play, if the entity made a step.
    pub step: Option<StepSound>,
}

fn creep(d: f64) -> f64 {
    const STEP: f64 = 0.05;
    if (-STEP..STEP).contains(&d) {
        0.0
    } else if d > 0.0 {
        d - STEP
    } else {
        d + STEP
    }
}

fn clip(boxes: &[Aabb], bb: &Aabb, axis: usize, mut d: f64) -> f64 {
    for b in boxes {
        d = b.axis_offset(bb, axis, d);
    }
    d
}

/// Vanilla `Entity.moveEntity` for an entity that is not riding, in no web and not
/// noclipping. `walks` is `canTriggerWalking` and that the entity is a player for the
/// sneak edge check.
pub fn move_entity(world: &mut World, body: &mut Body, dx: f64, dy: f64, dz: f64) -> MoveOutcome {
    let mut outcome = MoveOutcome::default();
    let start = body.pos;
    let (mut x, mut y, mut z) = (dx, dy, dz);
    let (mut x0, y0, mut z0) = (x, y, z);
    let mut bb = body.bb();
    let edge = body.on_ground && body.sneaking;
    if edge {
        let empty =
            |world: &mut World, d: [f64; 3]| colliding_boxes(world, &bb.offset(d)).is_empty();
        while x != 0.0 && empty(world, [x, -1.0, 0.0]) {
            x = creep(x);
            x0 = x;
        }
        while z != 0.0 && empty(world, [0.0, -1.0, z]) {
            z = creep(z);
            z0 = z;
        }
        while x != 0.0 && z != 0.0 && empty(world, [x, -1.0, z]) {
            x = creep(x);
            x0 = x;
            z = creep(z);
            z0 = z;
        }
    }

    let boxes = colliding_boxes(world, &bb.add_coord([x, y, z]));
    let original = bb;
    y = clip(&boxes, &bb, 1, y);
    bb = bb.offset([0.0, y, 0.0]);
    let grounded = body.on_ground || (y0 != y && y0 < 0.0);
    x = clip(&boxes, &bb, 0, x);
    bb = bb.offset([x, 0.0, 0.0]);
    z = clip(&boxes, &bb, 2, z);
    bb = bb.offset([0.0, 0.0, z]);

    if body.step_height > 0.0 && grounded && (x0 != x || z0 != z) {
        let (px, py, pz, plain) = (x, y, z, bb);
        let step = f64::from(body.step_height);
        let boxes = colliding_boxes(world, &original.add_coord([x0, step, z0]));
        // Step up first, then move horizontally.
        let mut a = original;
        let ay = clip(&boxes, &a.add_coord([x0, 0.0, z0]), 1, step);
        a = a.offset([0.0, ay, 0.0]);
        let ax = clip(&boxes, &a, 0, x0);
        a = a.offset([ax, 0.0, 0.0]);
        let az = clip(&boxes, &a, 2, z0);
        a = a.offset([0.0, 0.0, az]);
        // Or move horizontally at the original height.
        let mut b = original;
        let by = clip(&boxes, &b, 1, step);
        b = b.offset([0.0, by, 0.0]);
        let bx = clip(&boxes, &b, 0, x0);
        b = b.offset([bx, 0.0, 0.0]);
        let bz = clip(&boxes, &b, 2, z0);
        b = b.offset([0.0, 0.0, bz]);
        if ax * ax + az * az > bx * bx + bz * bz {
            x = ax;
            z = az;
            y = -ay;
            bb = a;
        } else {
            x = bx;
            z = bz;
            y = -by;
            bb = b;
        }
        y = clip(&boxes, &bb, 1, y);
        bb = bb.offset([0.0, y, 0.0]);
        if px * px + pz * pz >= x * x + z * z {
            x = px;
            y = py;
            z = pz;
            bb = plain;
        }
    }

    body.pos = (
        (bb.min[0] + bb.max[0]) / 2.0,
        bb.min[1],
        (bb.min[2] + bb.max[2]) / 2.0,
    );
    body.collided_horizontally = x0 != x || z0 != z;
    body.collided_vertically = y0 != y;
    body.on_ground = body.collided_vertically && y0 < 0.0;

    let bx = floor_f64(body.pos.0);
    let by = floor_f64(body.pos.1 - f64::from(0.2f32));
    let bz = floor_f64(body.pos.2);
    let mut below = (bx, by, bz);
    let mut below_state = world.block(bx, by, bz);
    if below_state >> 4 == 0 {
        let down = world.block(bx, by - 1, bz) >> 4;
        if blocks::is_fence_like(down) {
            below = (bx, by - 1, bz);
            below_state = world.block(bx, by - 1, bz);
        }
    }
    if x0 != x {
        body.motion.0 = 0.0;
    }
    if z0 != z {
        body.motion.2 = 0.0;
    }
    if y0 != y {
        // Block.onLanded: slime bounces unless sneaking, everything else stops.
        if below_state >> 4 == 165 && !body.sneaking {
            if body.motion.1 < 0.0 {
                body.motion.1 = -body.motion.1;
            }
        } else {
            body.motion.1 = 0.0;
        }
    }

    if !edge {
        let wx = body.pos.0 - start.0;
        let mut wy = body.pos.1 - start.1;
        let wz = body.pos.2 - start.2;
        if below_state >> 4 != 65 {
            wy = 0.0;
        }
        // TODO(M5): onEntityCollidedWithBlock for the block below (soul sand, slime).
        let step = krol_java::math::sqrt_f64(wx * wx + wy * wy + wz * wz);
        body.distance_walked_on_step =
            (f64::from(body.distance_walked_on_step) + f64::from(step) * 0.6) as f32;
        if body.distance_walked_on_step > body.next_step_distance as f32 && below_state >> 4 != 0 {
            body.next_step_distance = body.distance_walked_on_step as i32 + 1;
            // TODO(M5): swim sounds in water.
            outcome.step = step_sound(world, below, below_state);
        }
    }
    // TODO(M5): doBlockCollisions (webs, portals, pressure plates, cactus), fire.
    outcome
}

/// Vanilla `Entity.playStepSound`.
fn step_sound(world: &mut World, pos: (i32, i32, i32), state: u16) -> Option<StepSound> {
    let above = world.block(pos.0, pos.1 + 1, pos.2) >> 4;
    let sound: SoundType = if above == 78 {
        blocks::sound_type(78)
    } else if matches!(state >> 4, 8..=11) {
        return None;
    } else {
        blocks::sound_type(state >> 4)
    };
    Some(StepSound {
        name: sound.step_sound(),
        volume: sound.volume * 0.15,
        pitch: sound.frequency,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_offsets() {
        let floor = Aabb::new([0.0, 3.0, 0.0], [1.0, 4.0, 1.0]);
        let player = Aabb::at((0.5, 4.0, 0.5), 0.6, 1.8);
        assert_eq!(floor.axis_offset(&player, 1, -0.0784), 0.0);
        let falling = Aabb::at((0.5, 4.5, 0.5), 0.6, 1.8);
        assert_eq!(floor.axis_offset(&falling, 1, -1.0), -0.5);
        // Vanilla computes the half width in float.
        assert_eq!(player.max[0] - 0.5, f64::from(0.3f32));
    }
}
