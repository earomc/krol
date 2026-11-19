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
use super::shapes;
use super::world::World;

/// `Math.min` of two doubles (`-0.0` below `0.0`; NaN never occurs here).
const fn java_min(a: f64, b: f64) -> f64 {
    if a < b || (a == b && a.is_sign_negative()) {
        a
    } else {
        b
    }
}

/// `Math.max` of two doubles (`0.0` above `-0.0`).
const fn java_max(a: f64, b: f64) -> f64 {
    if a > b || (a == b && a.is_sign_positive()) {
        a
    } else {
        b
    }
}

/// An axis-aligned bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    /// Minimum corner (x, y, z).
    pub min: [f64; 3],
    /// Maximum corner (x, y, z).
    pub max: [f64; 3],
}

impl Aabb {
    /// A box from its corners. Like vanilla's `AxisAlignedBB` constructor, each axis is
    /// put in order (`Math.min` / `Math.max`), so a box shrunk past its size, such as
    /// a small entity's box in `handleWaterMovement`, turns inside out into a real one.
    pub const fn new(a: [f64; 3], b: [f64; 3]) -> Self {
        Self {
            min: [
                java_min(a[0], b[0]),
                java_min(a[1], b[1]),
                java_min(a[2], b[2]),
            ],
            max: [
                java_max(a[0], b[0]),
                java_max(a[1], b[1]),
                java_max(a[2], b[2]),
            ],
        }
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

    /// Whether the point lies strictly inside (`isVecInside`).
    pub fn contains(&self, p: (f64, f64, f64)) -> bool {
        let p = [p.0, p.1, p.2];
        (0..3).all(|i| p[i] > self.min[i] && p[i] < self.max[i])
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

/// The collision boxes of one block (`getCollisionBoundingBox` / `addCollisionBoxesToList`
/// with no mask).
pub fn block_boxes(state: u16, pos: (i32, i32, i32)) -> Vec<Aabb> {
    let everything = Aabb::new([-3.0e7, -1.0e4, -3.0e7], [3.0e7, 1.0e4, 3.0e7]);
    let mut out = Vec::new();
    // Without a world: no neighbors (fences stand alone, doors read only themselves).
    shapes::add_block_boxes(state, pos, &everything, &mut out, &mut |_| 0);
    out
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
                let mut neighbors = |p: (i32, i32, i32)| world.block(p.0, p.1, p.2);
                shapes::add_block_boxes(state, (x, y, z), bb, &mut out, &mut neighbors);
            }
        }
    }
    out
}

/// `World.isBlockFullCube`: the block's collision box averages at least 1 per edge.
/// The box is the union of the block's collision boxes (vanilla reads the block's
/// current bounds, which match that for the blocks an item can be stuck in).
fn is_full_cube_box(world: &mut World, pos: (i32, i32, i32)) -> bool {
    let state = world.block(pos.0, pos.1, pos.2);
    let boxes = block_boxes(state, pos);
    let Some(first) = boxes.first() else {
        return false;
    };
    let mut u = *first;
    for b in &boxes[1..] {
        for a in 0..3 {
            u.min[a] = u.min[a].min(b.min[a]);
            u.max[a] = u.max[a].max(b.max[a]);
        }
    }
    let edges = (u.max[0] - u.min[0]) + (u.max[1] - u.min[1]) + (u.max[2] - u.min[2]);
    edges / 3.0 >= 1.0
}

/// `Entity.pushOutOfBlocks` at `at` (an item passes its box's vertical middle): an
/// entity overlapping a block gets pushed toward the nearest side that is not a full
/// cube (up if all are), at `push` (the entity's `0.1 + nextFloat() * 0.2`, drawn only
/// then). Returns whether it overlaps, in which case its next move passes through
/// blocks (`noClip`).
pub fn push_out_of_blocks(
    world: &mut World,
    body: &mut Body,
    at: (f64, f64, f64),
    push: &mut dyn FnMut() -> f32,
) -> bool {
    let pos = (floor_f64(at.0), floor_f64(at.1), floor_f64(at.2));
    let (fx, fy, fz) = (
        at.0 - f64::from(pos.0),
        at.1 - f64::from(pos.1),
        at.2 - f64::from(pos.2),
    );
    // World.func_147461_a: colliding_boxes without entities.
    if colliding_boxes(world, &body.bb()).is_empty() && !is_full_cube_box(world, pos) {
        return false;
    }
    // 0 west, 1 east, 3 up, 4 north, 5 south (EnumFacing order, down never).
    let mut side = 3;
    let mut nearest = 9999.0;
    let candidates = [
        ((-1, 0, 0), fx, 0),
        ((1, 0, 0), 1.0 - fx, 1),
        ((0, 1, 0), 1.0 - fy, 3),
        ((0, 0, -1), fz, 4),
        ((0, 0, 1), 1.0 - fz, 5),
    ];
    for (d, distance, s) in candidates {
        let n = (pos.0 + d.0, pos.1 + d.1, pos.2 + d.2);
        if !is_full_cube_box(world, n) && distance < nearest {
            nearest = distance;
            side = s;
        }
    }
    let speed = f64::from(push());
    match side {
        0 => body.motion.0 = -speed,
        1 => body.motion.0 = speed,
        3 => body.motion.1 = speed,
        4 => body.motion.2 = -speed,
        _ => body.motion.2 = speed,
    }
    true
}

/// `moveEntity` with `noClip`: the box moves by the motion, through everything, and
/// nothing else happens.
pub fn move_no_clip(body: &mut Body, dx: f64, dy: f64, dz: f64) {
    let bb = body.bb().offset([dx, dy, dz]);
    body.follow_box(&bb);
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
    /// In water (`inWater`, from the last `handleWaterMovement`): steps also splash.
    pub in_water: bool,
    /// Touched a cobweb in the last `doBlockCollisions` (`isInWeb`): the next move is
    /// slowed down and stops the motion.
    pub in_web: bool,
    /// The motion was changed from outside (`velocityChanged`, `setBeenAttacked`): the
    /// tracker sends it at its next update.
    pub velocity_changed: bool,
    /// A box that no longer sits centered on [`pos`](Self::pos): `Entity.setSize`
    /// keeps the box's lower corner when an entity changes size, and the position only
    /// follows the box at the next move. `None` while the box is the one at `pos`.
    pub resized_box: Option<Aabb>,
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
            in_water: false,
            in_web: false,
            velocity_changed: false,
            resized_box: None,
        }
    }

    /// Applies what `doBlockCollisions` found: cobwebs (`setInWeb`) and soul sand
    /// (each soul sand block touched slows the horizontal motion to 40%).
    pub fn touched(&mut self, t: Touched) {
        if t.web {
            self.in_web = true;
        }
        for _ in 0..t.soul_sand {
            self.motion.0 *= SOUL_SAND_SLOWDOWN;
            self.motion.2 *= SOUL_SAND_SLOWDOWN;
        }
    }

    /// The current bounding box.
    pub fn bb(&self) -> Aabb {
        self.resized_box
            .unwrap_or_else(|| Aabb::at(self.pos, self.width, self.height))
    }

    /// `Entity.setPosition`: the box is the one at `pos` again.
    pub fn set_position(&mut self, pos: (f64, f64, f64)) {
        self.pos = pos;
        self.resized_box = None;
    }

    /// `Entity.setSize`: a new width and height on the same lower box corner. Returns
    /// the width before.
    pub fn set_size(&mut self, width: f32, height: f32) -> f32 {
        let old = self.width;
        if width != self.width || height != self.height {
            let min = self.bb().min;
            self.width = width;
            self.height = height;
            self.resized_box = Some(Aabb::new(
                min,
                [
                    min[0] + f64::from(width),
                    min[1] + f64::from(height),
                    min[2] + f64::from(width),
                ],
            ));
        }
        old
    }

    /// `resetPositionToBB`: the position follows the box `bb` (centered on it).
    fn follow_box(&mut self, bb: &Aabb) {
        self.pos = (
            (bb.min[0] + bb.max[0]) / 2.0,
            bb.min[1],
            (bb.min[2] + bb.max[2]) / 2.0,
        );
        self.resized_box = None;
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

/// `slime`.
const SLIME: u16 = 165;
/// Walking on slime slows entities moving slower than this vertically...
const SLIME_SLOW_BELOW: f64 = 0.1;
/// ...to this much of their horizontal motion, plus a bit per vertical speed.
const SLIME_SLOWDOWN: f64 = 0.4;
const SLIME_SLOWDOWN_PER_SPEED: f64 = 0.2;

/// `BlockSoulSand.onEntityCollidedWithBlock` keeps this much of the horizontal motion.
pub const SOUL_SAND_SLOWDOWN: f64 = 0.4;
/// A move in a cobweb covers a quarter of the distance horizontally...
const WEB_HORIZONTAL: f64 = 0.25;
/// ...and `0.05F` of it vertically.
const WEB_VERTICAL: f32 = 0.05;

/// Blocks with an effect on entities that `doBlockCollisions` found.
#[derive(Debug, Default, Clone, Copy)]
pub struct Touched {
    /// A cobweb (`BlockWeb`).
    pub web: bool,
    /// How many soul sand blocks (`BlockSoulSand`).
    pub soul_sand: u32,
}

/// What a move did besides changing the body.
#[derive(Debug, Default)]
pub struct MoveOutcome {
    /// The entity made a step (`playStepSound` is due, even on a block without one).
    pub stepped: bool,
    /// Step sound to play, if the entity made a step.
    pub step: Option<StepSound>,
    /// Volume of the swim sound to play before the step sound (steps in water).
    pub swim: Option<f32>,
    /// The block walked on (`onEntityCollidedWithBlock(world, pos, entity)` of the block
    /// below, for entities that trigger walking and end the move on the ground).
    pub walked_on: Option<((i32, i32, i32), u16)>,
    /// The vertical distance actually moved after collisions (the `y` that
    /// `updateFallState` sees).
    pub moved_y: f64,
    /// The block under the entity after the move (`posY - 0.2`, or the fence below an
    /// air block): the block `updateFallState` lands on.
    pub below: ((i32, i32, i32), u16),
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

/// Vanilla `Entity.moveEntity` for an entity that is not riding and not noclipping
/// (cobwebs slow it down, see [`Body::in_web`]). `walks` is `canTriggerWalking` and that the entity is a player for the
/// sneak edge check.
pub fn move_entity(world: &mut World, body: &mut Body, dx: f64, dy: f64, dz: f64) -> MoveOutcome {
    move_entity_inner(world, body, dx, dy, dz, true, &[])
}

/// [`move_entity`] that also collides with the boxes of nearby entities (a player
/// walking into boats).
pub fn move_entity_walking_among(
    world: &mut World,
    body: &mut Body,
    d: (f64, f64, f64),
    entities: &[Aabb],
) -> MoveOutcome {
    move_entity_inner(world, body, d.0, d.1, d.2, true, entities)
}

/// `moveEntity` for entities that do not trigger walking (`canTriggerWalking` false,
/// e.g. items): no step sounds.
pub fn move_entity_quiet(
    world: &mut World,
    body: &mut Body,
    dx: f64,
    dy: f64,
    dz: f64,
) -> MoveOutcome {
    move_entity_inner(world, body, dx, dy, dz, false, &[])
}

/// [`move_entity_quiet`] that also collides with the boxes of nearby entities (vanilla
/// `getCollidingBoundingBoxes` adds `getCollisionBox` of the entities around the mover:
/// for a minecart, those of pushable entities).
pub fn move_entity_among(
    world: &mut World,
    body: &mut Body,
    d: (f64, f64, f64),
    entities: &[Aabb],
) -> MoveOutcome {
    move_entity_inner(world, body, d.0, d.1, d.2, false, entities)
}

fn move_entity_inner(
    world: &mut World,
    body: &mut Body,
    dx: f64,
    dy: f64,
    dz: f64,
    walks: bool,
    entities: &[Aabb],
) -> MoveOutcome {
    let colliding = |world: &mut World, query: &Aabb| {
        let mut boxes = colliding_boxes(world, query);
        boxes.extend(entities.iter().filter(|b| b.intersects(query)).copied());
        boxes
    };
    let mut outcome = MoveOutcome::default();
    let start = body.pos;
    let (mut x, mut y, mut z) = (dx, dy, dz);
    if body.in_web {
        body.in_web = false;
        x *= WEB_HORIZONTAL;
        y *= f64::from(WEB_VERTICAL);
        z *= WEB_HORIZONTAL;
        body.motion = (0.0, 0.0, 0.0);
    }
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

    let boxes = colliding(world, &bb.add_coord([x, y, z]));
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
        let boxes = colliding(world, &original.add_coord([x0, step, z0]));
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

    body.follow_box(&bb);
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
    outcome.moved_y = y;
    outcome.below = (below, below_state);
    if x0 != x {
        body.motion.0 = 0.0;
    }
    if z0 != z {
        body.motion.2 = 0.0;
    }
    if y0 != y {
        // Block.onLanded: slime bounces unless sneaking, everything else stops.
        if below_state >> 4 == SLIME && !body.sneaking {
            if body.motion.1 < 0.0 {
                body.motion.1 = -body.motion.1;
            }
        } else {
            body.motion.1 = 0.0;
        }
    }

    if walks && !edge {
        let wx = body.pos.0 - start.0;
        let mut wy = body.pos.1 - start.1;
        let wz = body.pos.2 - start.2;
        if below_state >> 4 != 65 {
            wy = 0.0;
        }
        if body.on_ground {
            // BlockSlime.onEntityCollidedWithBlock: walking on slime is slow.
            if below_state >> 4 == SLIME && body.motion.1.abs() < SLIME_SLOW_BELOW && !body.sneaking
            {
                let f = SLIME_SLOWDOWN + body.motion.1.abs() * SLIME_SLOWDOWN_PER_SPEED;
                body.motion.0 *= f;
                body.motion.2 *= f;
            }
            outcome.walked_on = Some((below, below_state));
        }
        let step = krol_java::math::sqrt_f64(wx * wx + wy * wy + wz * wz);
        body.distance_walked_on_step =
            (f64::from(body.distance_walked_on_step) + f64::from(step) * 0.6) as f32;
        if body.distance_walked_on_step > body.next_step_distance as f32 && below_state >> 4 != 0 {
            body.next_step_distance = body.distance_walked_on_step as i32 + 1;
            outcome.stepped = true;
            if body.in_water {
                let m = body.motion;
                let speed = krol_java::math::sqrt_f64(
                    m.0 * m.0 * f64::from(0.2f32) + m.1 * m.1 + m.2 * m.2 * f64::from(0.2f32),
                ) * SWIM_VOLUME_FACTOR;
                outcome.swim = Some(speed.min(1.0));
            }
            outcome.step = step_sound(world, below, below_state);
        }
    }
    // doBlockCollisions runs in the callers (`Game::block_collisions`), which see the
    // whole world.
    outcome
}

/// Swim sound volume per unit of (weighted) speed.
const SWIM_VOLUME_FACTOR: f32 = 0.35;

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
    fn shrunk_boxes_turn_inside_out() {
        // A piglet's box (0.45 high) in handleWaterMovement reaches 0.049 to 0.401 up.
        let bb = Aabb::at((0.5, 4.0, 0.5), 0.45, 0.45).expand([0.0, f64::from(-0.4f32), 0.0]);
        assert!(bb.min[1] < bb.max[1]);
        assert!((bb.min[1] - 4.049_999_994).abs() < 1e-6);
        assert_eq!(Aabb::new([0.0; 3], [-0.0; 3]).min, [-0.0; 3]);
    }

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
