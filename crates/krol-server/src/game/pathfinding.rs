//! Path finding for walking mobs (vanilla `PathFinder` with its `Path` heap and
//! `PathPoint`s, `WalkNodeProcessor`, and the path a mob follows, `PathEntity`).
//!
//! The search is vanilla's A* variant: the open set is a binary heap ordered by the
//! squared distances (in `float`, never NaN), ties broken by the heap's own sift order, so the
//! heap is reproduced exactly rather than using `BinaryHeap`. A path either reaches
//! the target or ends at the point that came closest to it.
//!
//! Vanilla looks points up by `PathPoint.makeHash` alone; its collisions need points
//! 128 or more blocks apart, far beyond any search, so a map keyed by coordinates
//! behaves the same.
//!
//! Vanilla searches a `ChunkCache` of the area around the mob; krol reads the world
//! directly (the same blocks, as long as the chunks are loaded).

use std::collections::HashMap;

use krol_java::math::{floor_f32, floor_f64, sqrt_f32};

use super::block_data::{Material, material};
use super::ids::block;
use super::world::World;

/// A path a mob follows (`PathEntity`): block positions, the index of the next one and
/// how many of them count.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    points: Vec<(i32, i32, i32)>,
    /// `currentPathIndex`.
    pub index: usize,
    /// `pathLength`.
    pub length: usize,
}

impl Path {
    fn new(points: Vec<(i32, i32, i32)>) -> Self {
        let length = points.len();
        Self {
            points,
            index: 0,
            length,
        }
    }

    /// `isFinished`.
    pub fn finished(&self) -> bool {
        self.index >= self.length
    }

    /// `getPathPointFromIndex`.
    pub fn point(&self, k: usize) -> (i32, i32, i32) {
        self.points[k]
    }

    /// `getVectorFromIndex`: the point's corner plus half the mob's width rounded up to
    /// whole blocks.
    pub fn vector(&self, k: usize, width: f32) -> (f64, f64, f64) {
        let p = self.points[k];
        let half = f64::from((width + 1.0) as i32) * 0.5;
        (f64::from(p.0) + half, f64::from(p.1), f64::from(p.2) + half)
    }

    /// `isSamePath`: the same points (the index and length aside).
    pub fn same_points(&self, other: &Path) -> bool {
        self.points == other.points
    }
}

/// What the walk node processor knows about the mob looking for a path.
#[derive(Debug, Clone, Copy)]
pub struct Walker {
    /// `posX`, `posY`, `posZ`.
    pub pos: (f64, f64, f64),
    /// `getEntityBoundingBox().minX/minY/minZ`.
    pub min: [f64; 3],
    pub width: f32,
    pub height: f32,
    /// `isInWater`.
    pub in_water: bool,
    /// `isInLava`.
    pub in_lava: bool,
    /// `getMaxFallHeight`.
    pub max_fall: i32,
}

/// The node processor's settings (`WalkNodeProcessor` flags set by the navigator).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkFlags {
    /// `canEnterDoors`.
    pub enter_doors: bool,
    /// `canBreakDoors`.
    pub break_doors: bool,
    /// `avoidsWater`.
    pub avoids_water: bool,
    /// `canSwim`.
    pub can_swim: bool,
}

/// `getVerticalOffset` results.
const OPEN: i32 = 1;
const WATER_OR_TRAPDOOR: i32 = 2;
const BLOCKED: i32 = 0;
const AVOIDED_WATER: i32 = -1;
const LAVA: i32 = -2;
const FENCE_OR_RAIL: i32 = -3;
const CLOSED_TRAPDOOR: i32 = -4;

/// The most path options a point has (`pathOptions` is longer; four are used).
const MAX_OPTIONS: usize = 4;

#[derive(Debug, Clone)]
struct Point {
    pos: (i32, i32, i32),
    /// The heap slot, or `None` when not in the heap (`index`, -1).
    heap: Option<usize>,
    total: f32,
    to_next: f32,
    to_target: f32,
    previous: Option<usize>,
    visited: bool,
}

impl Point {
    fn distance_sq(&self, other: &Point) -> f32 {
        let dx = (other.pos.0 - self.pos.0) as f32;
        let dy = (other.pos.1 - self.pos.1) as f32;
        let dz = (other.pos.2 - self.pos.2) as f32;
        dx * dx + dy * dy + dz * dz
    }
}

/// One search: the points opened so far and the open set.
struct Search<'w> {
    world: &'w mut World,
    walker: Walker,
    /// `BlockPos(entity)`: the rails check compares with the block the mob is in.
    walker_block: (i32, i32, i32),
    size: (i32, i32, i32),
    flags: WalkFlags,
    points: Vec<Point>,
    by_pos: HashMap<(i32, i32, i32), usize>,
    heap: Vec<usize>,
}

impl Search<'_> {
    /// `openPoint`.
    fn open(&mut self, pos: (i32, i32, i32)) -> usize {
        if let Some(&k) = self.by_pos.get(&pos) {
            return k;
        }
        let k = self.points.len();
        self.points.push(Point {
            pos,
            heap: None,
            total: 0.0,
            to_next: 0.0,
            to_target: 0.0,
            previous: None,
            visited: false,
        });
        self.by_pos.insert(pos, k);
        k
    }

    fn dist_sq(&self, a: usize, b: usize) -> f32 {
        self.points[a].distance_sq(&self.points[b])
    }

    // The heap (`Path`).

    fn heap_add(&mut self, k: usize) {
        let slot = self.heap.len();
        self.heap.push(k);
        self.points[k].heap = Some(slot);
        self.sort_back(slot);
    }

    fn heap_dequeue(&mut self) -> usize {
        let top = self.heap[0];
        let last = self.heap.pop().unwrap_or(top);
        if !self.heap.is_empty() {
            self.heap[0] = last;
            self.sort_forward(0);
        }
        self.points[top].heap = None;
        top
    }

    fn heap_change(&mut self, k: usize, to_target: f32) {
        let old = self.points[k].to_target;
        self.points[k].to_target = to_target;
        let Some(slot) = self.points[k].heap else {
            return;
        };
        if to_target < old {
            self.sort_back(slot);
        } else {
            self.sort_forward(slot);
        }
    }

    fn sort_back(&mut self, mut slot: usize) {
        let k = self.heap[slot];
        let d = self.points[k].to_target;
        while slot > 0 {
            let parent = (slot - 1) >> 1;
            let p = self.heap[parent];
            if d >= self.points[p].to_target {
                break;
            }
            self.heap[slot] = p;
            self.points[p].heap = Some(slot);
            slot = parent;
        }
        self.heap[slot] = k;
        self.points[k].heap = Some(slot);
    }

    fn sort_forward(&mut self, mut slot: usize) {
        let k = self.heap[slot];
        let d = self.points[k].to_target;
        let count = self.heap.len();
        loop {
            let left = 1 + (slot << 1);
            let right = left + 1;
            if left >= count {
                break;
            }
            let l = self.heap[left];
            let dl = self.points[l].to_target;
            let (r, dr) = if right >= count {
                (None, f32::INFINITY)
            } else {
                let r = self.heap[right];
                (Some(r), self.points[r].to_target)
            };
            if dl < dr {
                if dl >= d {
                    break;
                }
                self.heap[slot] = l;
                self.points[l].heap = Some(slot);
                slot = left;
            } else {
                let Some(r) = r else { break };
                if dr >= d {
                    break;
                }
                self.heap[slot] = r;
                self.points[r].heap = Some(slot);
                slot = right;
            }
        }
        self.heap[slot] = k;
        self.points[k].heap = Some(slot);
    }

    // The walk node processor.

    /// `findPathOptions`: the safe points south, west, east and north (stepping up a
    /// block if there is room above), unvisited and within `max` of the target.
    fn options(&mut self, from: usize, target: usize, max: f32) -> Vec<usize> {
        let (x, y, z) = self.points[from].pos;
        let step = if self.vertical_offset((x, y + 1, z)) == OPEN {
            1
        } else {
            0
        };
        let candidates = [
            self.safe_point((x, y, z + 1), step),
            self.safe_point((x - 1, y, z), step),
            self.safe_point((x + 1, y, z), step),
            self.safe_point((x, y, z - 1), step),
        ];
        let mut out = Vec::with_capacity(MAX_OPTIONS);
        for k in candidates.into_iter().flatten() {
            if !self.points[k].visited && sqrt_f32(self.dist_sq(k, target)) < max {
                out.push(k);
            }
        }
        out
    }

    /// `getSafePoint`: the point the mob ends up at walking into `(x, y, z)`: there,
    /// one block up, or after falling no further than it may.
    fn safe_point(&mut self, (x, mut y, z): (i32, i32, i32), step: i32) -> Option<usize> {
        let offset = self.vertical_offset((x, y, z));
        if offset == WATER_OR_TRAPDOOR {
            return Some(self.open((x, y, z)));
        }
        let mut point = (offset == OPEN).then(|| self.open((x, y, z)));
        if point.is_none()
            && step > 0
            && offset != FENCE_OR_RAIL
            && offset != CLOSED_TRAPDOOR
            && self.vertical_offset((x, y + step, z)) == OPEN
        {
            point = Some(self.open((x, y + step, z)));
            y += step;
        }
        point?;
        let mut fallen = 0;
        let mut below = 0;
        while y > 0 {
            below = self.vertical_offset((x, y - 1, z));
            if self.flags.avoids_water && below == AVOIDED_WATER {
                return None;
            }
            if below != OPEN {
                break;
            }
            let fell = fallen;
            fallen += 1;
            if fell >= self.walker.max_fall {
                return None;
            }
            y -= 1;
            if y <= 0 {
                return None;
            }
            point = Some(self.open((x, y, z)));
        }
        if below == LAVA {
            return None;
        }
        point
    }

    /// `func_176170_a`: what the mob's box at `(x, y, z)` (rounded up to whole blocks)
    /// runs into.
    fn vertical_offset(&mut self, (x, y, z): (i32, i32, i32)) -> i32 {
        let mut wet = false;
        for i in x..x + self.size.0 {
            for j in y..y + self.size.1 {
                for k in z..z + self.size.2 {
                    let state = self.world.block(i, j, k);
                    let id = state >> 4;
                    let mat = material(id);
                    if mat == Material::Air {
                        continue;
                    }
                    if is_trapdoor(id) {
                        wet = true;
                    } else if is_water(id) {
                        if self.flags.avoids_water {
                            return AVOIDED_WATER;
                        }
                        wet = true;
                    } else if !self.flags.enter_doors && is_door(id) && mat == Material::Wood {
                        return BLOCKED;
                    }
                    if is_rail(id) {
                        let (bx, by, bz) = self.walker_block;
                        if !is_rail(self.world.block(bx, by, bz) >> 4)
                            && !is_rail(self.world.block(bx, by - 1, bz) >> 4)
                        {
                            return FENCE_OR_RAIL;
                        }
                    } else if !passable(self.world, (i, j, k), state)
                        && !(self.flags.break_doors && is_door(id) && mat == Material::Wood)
                    {
                        if is_fence_gate_or_wall(id) {
                            return FENCE_OR_RAIL;
                        }
                        if is_trapdoor(id) {
                            return CLOSED_TRAPDOOR;
                        }
                        if mat != Material::Lava {
                            return BLOCKED;
                        }
                        if !self.walker.in_lava {
                            return LAVA;
                        }
                    }
                }
            }
        }
        if wet { WATER_OR_TRAPDOOR } else { OPEN }
    }

    /// `addToPath`: the search from `start` towards `end`.
    fn search(&mut self, start: usize, end: usize, max: f32) -> Option<Path> {
        self.points[start].total = 0.0;
        self.points[start].to_next = self.dist_sq(start, end);
        self.points[start].to_target = self.points[start].to_next;
        self.heap.clear();
        self.heap_add(start);
        let mut closest = start;
        while !self.heap.is_empty() {
            let current = self.heap_dequeue();
            if self.points[current].pos == self.points[end].pos {
                return Some(self.build(current));
            }
            if self.dist_sq(current, end) < self.dist_sq(closest, end) {
                closest = current;
            }
            self.points[current].visited = true;
            for option in self.options(current, end, max) {
                let total = self.points[current].total + self.dist_sq(current, option);
                let assigned = self.points[option].heap.is_some();
                if total < max * 2.0 && (!assigned || total < self.points[option].total) {
                    let to_next = self.dist_sq(option, end);
                    let p = &mut self.points[option];
                    p.previous = Some(current);
                    p.total = total;
                    p.to_next = to_next;
                    if assigned {
                        self.heap_change(option, total + to_next);
                    } else {
                        self.points[option].to_target = total + to_next;
                        self.heap_add(option);
                    }
                }
            }
        }
        (closest != start).then(|| self.build(closest))
    }

    /// `createEntityPath`: the points from the start to `last`.
    fn build(&self, last: usize) -> Path {
        let mut points = vec![self.points[last].pos];
        let mut k = last;
        while let Some(previous) = self.points[k].previous {
            points.push(self.points[previous].pos);
            k = previous;
        }
        points.reverse();
        Path::new(points)
    }
}

/// `PathFinder.createEntityPathTo` for a walking mob towards `target` (a block
/// position's center for `tryMoveToXYZ`, an entity's feet for `tryMoveToEntityLiving`),
/// searching at most `range` (`followRange`) away.
pub fn find_path(
    world: &mut World,
    walker: Walker,
    flags: WalkFlags,
    target: (f64, f64, f64),
    range: f32,
) -> Option<Path> {
    let mut search = Search {
        world,
        walker,
        walker_block: (
            floor_f64(walker.pos.0),
            floor_f64(walker.pos.1),
            floor_f64(walker.pos.2),
        ),
        size: (
            floor_f32(walker.width + 1.0),
            floor_f32(walker.height + 1.0),
            floor_f32(walker.width + 1.0),
        ),
        flags,
        points: Vec::new(),
        by_pos: HashMap::new(),
        heap: Vec::new(),
    };
    // getPathPointTo: a swimming mob starts from the water's surface, and does not
    // avoid water for this search.
    let y = if flags.can_swim && walker.in_water {
        let (x, z) = (floor_f64(walker.pos.0), floor_f64(walker.pos.2));
        let mut y = walker.min[1] as i32;
        while is_water(search.world.block(x, y, z) >> 4) {
            y += 1;
        }
        search.flags.avoids_water = false;
        y
    } else {
        floor_f64(walker.min[1] + 0.5)
    };
    let start = search.open((floor_f64(walker.min[0]), y, floor_f64(walker.min[2])));
    // getPathPointToCoords.
    let half = f64::from(walker.width / 2.0);
    let end = search.open((
        floor_f64(target.0 - half),
        floor_f64(target.1),
        floor_f64(target.2 - half),
    ));
    search.search(start, end, range)
}

/// `Block.isPassable` and its overrides: whether a mob may walk through the block.
pub fn passable(world: &mut World, pos: (i32, i32, i32), state: u16) -> bool {
    let id = state >> 4;
    let meta = state & 15;
    match id {
        // BlockDoor: open, as the lower half says.
        _ if is_door(id) => {
            let lower = if meta & 8 != 0 {
                world.block(pos.0, pos.1 - 1, pos.2) & 15
            } else {
                meta
            };
            lower & 4 != 0
        }
        // BlockTrapDoor: closed.
        _ if is_trapdoor(id) => meta & 4 == 0,
        // BlockFenceGate: open.
        block::OAK_FENCE_GATE | block::SPRUCE_FENCE_GATE..=block::ACACIA_FENCE_GATE => {
            meta & 4 != 0
        }
        // BlockSnow: fewer than 5 layers.
        block::SNOW_LAYER => meta & 7 < 4,
        // Banners, pressure plates and signs.
        176 | 177 | 70 | 72 | 147 | 148 | 63 | 68 => true,
        // Fences and walls.
        85 | 113 | 188..=192 | 139 => false,
        // BlockLiquid: not lava.
        8..=11 => material(id) != Material::Lava,
        _ => !material(id).blocks_movement(),
    }
}

fn is_water(id: u16) -> bool {
    matches!(id, block::FLOWING_WATER | block::WATER)
}

fn is_trapdoor(id: u16) -> bool {
    matches!(id, block::TRAPDOOR | block::IRON_TRAPDOOR)
}

/// `BlockDoor` (wooden and iron).
pub fn is_door(id: u16) -> bool {
    matches!(
        id,
        block::WOODEN_DOOR | block::IRON_DOOR | block::SPRUCE_DOOR..=block::DARK_OAK_DOOR
    )
}

/// `BlockRailBase`.
fn is_rail(id: u16) -> bool {
    matches!(
        id,
        block::GOLDEN_RAIL | block::DETECTOR_RAIL | block::RAIL | block::ACTIVATOR_RAIL
    )
}

/// `BlockFence`, `BlockFenceGate` and `BlockWall`.
fn is_fence_gate_or_wall(id: u16) -> bool {
    super::blocks::is_fence_like(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heap_order(distances: &[f32]) -> Vec<f32> {
        let mut world = World::new(0, std::path::PathBuf::from("no-such-world"));
        let mut s = Search {
            world: &mut world,
            walker: Walker {
                pos: (0.0, 0.0, 0.0),
                min: [0.0; 3],
                width: 0.9,
                height: 0.9,
                in_water: false,
                in_lava: false,
                max_fall: 3,
            },
            walker_block: (0, 0, 0),
            size: (1, 1, 1),
            flags: WalkFlags {
                enter_doors: true,
                break_doors: false,
                avoids_water: true,
                can_swim: true,
            },
            points: Vec::new(),
            by_pos: HashMap::new(),
            heap: Vec::new(),
        };
        for (k, &d) in distances.iter().enumerate() {
            let p = s.open((k as i32, 0, 0));
            s.points[p].to_target = d;
            s.heap_add(p);
        }
        let mut out = Vec::new();
        while !s.heap.is_empty() {
            let k = s.heap_dequeue();
            out.push(s.points[k].to_target);
        }
        out
    }

    #[test]
    fn heap_dequeues_the_smallest_first() {
        assert_eq!(
            heap_order(&[5.0, 3.0, 8.0, 1.0, 4.0, 1.0]),
            [1.0, 1.0, 3.0, 4.0, 5.0, 8.0]
        );
    }

    #[test]
    fn path_vectors_center_on_the_mob() {
        let path = Path::new(vec![(1, 64, -2)]);
        // A pig (0.9 wide) aims for the block center.
        assert_eq!(path.vector(0, 0.9), (1.5, 64.0, -1.5));
        // A wider mob aims for the corner between two blocks.
        assert_eq!(path.vector(0, 1.4), (2.0, 64.0, -1.0));
    }
}
