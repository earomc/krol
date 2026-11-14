//! Flowing water and lava (vanilla `BlockLiquid`, `BlockDynamicLiquid`,
//! `BlockStaticLiquid`).
//!
//! A liquid's metadata is its level: 0 is a source, 1-7 flow further from it, and 8 and
//! above mark a falling column (`8 + level` of the block above). Flowing blocks run
//! scheduled ticks; still blocks turn back into flowing ones when a neighbor changes.

use super::Game;
use super::block_data::{Material, material};
use super::block_update::flags;
use super::ids::block;
use super::world::World;

/// `BlockLiquid.tickRate` of water.
const WATER_TICK_RATE: i32 = 5;
/// `BlockLiquid.tickRate` of lava in the overworld (10 where water vaporizes).
const LAVA_TICK_RATE: i32 = 30;
/// How much the level drops per block for water.
const WATER_DECAY: i32 = 1;
/// How much the level drops per block for lava in the overworld (1 in the nether).
const LAVA_DECAY: i32 = 2;
/// Levels at or above this mark falling liquid.
const FALLING: i32 = 8;
/// The `getLevel` of a block that is not this liquid.
const NOT_LIQUID: i32 = -1;
/// How far `getPossibleFlowDirections` looks for a way down.
const MAX_SLOPE_DISTANCE: i32 = 4;
/// The cost of a direction without a way down in reach.
const NO_SLOPE: i32 = 1000;
/// Lava spreading away from its source slows down 4x, 3 times in 4.
const LAVA_SLOWDOWN: i32 = 4;

/// The `EnumFacing.Plane.HORIZONTAL` iteration order: north, east, south, west.
const HORIZONTAL: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
/// The `EnumSet<EnumFacing>` order of the horizontal faces: north, south, west, east.
const FLOW_ORDER: [(i32, i32); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];

/// `random.fizz` volume of liquids mixing.
const FIZZ_VOLUME: f32 = 0.5;
/// `random.fizz` base pitch.
const FIZZ_PITCH: f32 = 2.6;
/// `random.fizz` pitch spread.
const FIZZ_SPREAD: f32 = 0.8;

/// Water or lava, flowing or still.
pub fn is_liquid(id: u16) -> bool {
    matches!(
        id,
        block::FLOWING_WATER | block::WATER | block::FLOWING_LAVA | block::LAVA
    )
}

/// The flowing block of a liquid material.
fn flowing_of(m: Material) -> u16 {
    if m == Material::Lava {
        block::FLOWING_LAVA
    } else {
        block::FLOWING_WATER
    }
}

/// The still block of a liquid material.
fn still_of(m: Material) -> u16 {
    if m == Material::Lava {
        block::LAVA
    } else {
        block::WATER
    }
}

/// `BlockLiquid.tickRate`.
pub fn tick_rate(id: u16) -> i32 {
    if material(id) == Material::Lava {
        LAVA_TICK_RATE
    } else {
        WATER_TICK_RATE
    }
}

fn add(p: (i32, i32, i32), d: (i32, i32)) -> (i32, i32, i32) {
    (p.0 + d.0, p.1, p.2 + d.1)
}

fn down(p: (i32, i32, i32)) -> (i32, i32, i32) {
    (p.0, p.1 - 1, p.2)
}

impl Game {
    fn state_at(&mut self, p: (i32, i32, i32)) -> u16 {
        self.level.overworld.block(p.0, p.1, p.2)
    }

    /// `BlockLiquid.getLevel`: the level of the block at `p` if it is the same liquid.
    fn liquid_level(&mut self, p: (i32, i32, i32), m: Material) -> i32 {
        let s = self.state_at(p);
        if material(s >> 4) == m {
            i32::from(s & 15)
        } else {
            NOT_LIQUID
        }
    }

    /// `BlockDynamicLiquid.isBlocked`: doors, signs, ladders, sugar cane, portals and
    /// anything that blocks movement stop the flow.
    fn liquid_blocked(&mut self, p: (i32, i32, i32)) -> bool {
        let id = self.state_at(p) >> 4;
        if super::powered_blocks::is_door(id)
            || matches!(id, block::STANDING_SIGN | block::LADDER | block::REEDS)
        {
            return true;
        }
        let m = material(id);
        m == Material::Portal || m.blocks_movement()
    }

    /// `BlockDynamicLiquid.canFlowInto`.
    fn can_flow_into(&mut self, p: (i32, i32, i32), m: Material) -> bool {
        let other = material(self.state_at(p) >> 4);
        other != m && other != Material::Lava && !self.liquid_blocked(p)
    }

    /// `BlockLiquid.triggerMixEffects`: the fizz sound (the smoke particles are
    /// client-side only).
    fn mix_effects(&mut self, p: (i32, i32, i32)) {
        let r = &mut self.world_rand;
        let pitch = FIZZ_PITCH + (r.next_float() - r.next_float()) * FIZZ_SPREAD;
        let center = (
            f64::from(p.0) + 0.5,
            f64::from(p.1) + 0.5,
            f64::from(p.2) + 0.5,
        );
        self.play_sound_near_except(None, "random.fizz", center, FIZZ_VOLUME, pitch);
    }

    /// `BlockLiquid.checkForMixing`: lava touching water (except from below) hardens
    /// into obsidian (a source) or cobblestone (level 4 or less).
    fn check_for_mixing(&mut self, pos: (i32, i32, i32), state: u16) -> bool {
        if material(state >> 4) != Material::Lava {
            return false;
        }
        let sides = [(0, 1, 0), (0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)];
        let touches_water = sides.iter().any(|&(dx, dy, dz)| {
            let s = self.state_at((pos.0 + dx, pos.1 + dy, pos.2 + dz));
            material(s >> 4) == Material::Water
        });
        if !touches_water {
            return false;
        }
        let level = i32::from(state & 15);
        let result = if level == 0 {
            block::OBSIDIAN
        } else if level <= 4 {
            block::COBBLESTONE
        } else {
            return false;
        };
        self.set_block_notify(pos, result << 4);
        self.mix_effects(pos);
        true
    }

    /// `onBlockAdded` of a liquid.
    pub(super) fn liquid_added(&mut self, pos: (i32, i32, i32), state: u16) {
        let id = state >> 4;
        let mixed = self.check_for_mixing(pos, state);
        if !mixed && matches!(id, block::FLOWING_WATER | block::FLOWING_LAVA) {
            self.schedule_update(pos, id, tick_rate(id));
        }
    }

    /// `onNeighborBlockChange` of a liquid: still liquids start flowing again.
    pub(super) fn liquid_neighbor_changed(&mut self, pos: (i32, i32, i32), state: u16) {
        let id = state >> 4;
        if self.check_for_mixing(pos, state) {
            return;
        }
        if matches!(id, block::WATER | block::LAVA) {
            // BlockStaticLiquid.updateLiquid.
            let flowing = flowing_of(material(id));
            self.set_block_flags(pos, flowing << 4 | (state & 15), flags::SEND);
            self.schedule_update(pos, flowing, tick_rate(id));
        }
    }

    /// `BlockDynamicLiquid.placeStaticBlock`.
    fn place_static(&mut self, pos: (i32, i32, i32), state: u16) {
        let still = still_of(material(state >> 4));
        self.set_block_flags(pos, still << 4 | (state & 15), flags::SEND);
    }

    /// `BlockDynamicLiquid.updateTick`.
    pub(super) fn flowing_liquid_tick(&mut self, pos: (i32, i32, i32), mut state: u16) {
        let id = state >> 4;
        let m = material(id);
        let lava = m == Material::Lava;
        let mut level = i32::from(state & 15);
        let decay = if lava { LAVA_DECAY } else { WATER_DECAY };
        let mut rate = tick_rate(id);
        if level > 0 {
            let mut smallest = -100;
            let mut sources = 0;
            for d in HORIZONTAL {
                // checkAdjacentBlock.
                let mut l = self.liquid_level(add(pos, d), m);
                if l < 0 {
                    continue;
                }
                if l == 0 {
                    sources += 1;
                }
                if l >= FALLING {
                    l = 0;
                }
                if !(smallest >= 0 && l >= smallest) {
                    smallest = l;
                }
            }
            let mut new_level = smallest + decay;
            if new_level >= FALLING || smallest < 0 {
                new_level = NOT_LIQUID;
            }
            let above = self.liquid_level((pos.0, pos.1 + 1, pos.2), m);
            if above >= 0 {
                new_level = if above >= FALLING {
                    above
                } else {
                    above + FALLING
                };
            }
            if sources >= 2 && !lava {
                let below = self.state_at(down(pos));
                let bm = material(below >> 4);
                if bm.is_solid() || (bm == m && below & 15 == 0) {
                    new_level = 0;
                }
            }
            if lava
                && level < FALLING
                && new_level < FALLING
                && new_level > level
                && self.world_rand.next_int_bounded(4) != 0
            {
                rate *= LAVA_SLOWDOWN;
            }
            if new_level == level {
                self.place_static(pos, state);
            } else {
                level = new_level;
                if new_level < 0 {
                    self.set_block_notify(pos, 0);
                } else {
                    state = id << 4 | new_level as u16;
                    self.set_block_flags(pos, state, flags::SEND);
                    self.schedule_update(pos, id, rate);
                    self.notify_neighbors(pos, id);
                }
            }
        } else {
            self.place_static(pos, state);
        }

        let below = down(pos);
        if self.can_flow_into(below, m) {
            if lava && material(self.state_at(below) >> 4) == Material::Water {
                self.set_block_notify(below, block::STONE << 4);
                self.mix_effects(below);
                return;
            }
            // Vanilla quirk: a block that just dried up (level -1) still flows down
            // with level 7.
            let l = if level >= FALLING {
                level
            } else {
                level + FALLING
            };
            self.try_flow_into(below, id, l);
        } else if level >= 0 && (level == 0 || self.liquid_blocked(below)) {
            let dirs = self.flow_directions(pos, m);
            let next = if level >= FALLING { 1 } else { level + decay };
            if next >= FALLING {
                return;
            }
            for d in dirs {
                self.try_flow_into(add(pos, d), id, next);
            }
        }
    }

    /// `BlockDynamicLiquid.tryFlowInto`: washes away (or, for lava, burns with a fizz)
    /// what is there and places flowing liquid.
    fn try_flow_into(&mut self, p: (i32, i32, i32), id: u16, level: i32) {
        let m = material(id);
        if !self.can_flow_into(p, m) {
            return;
        }
        let old = self.state_at(p);
        if old >> 4 != block::AIR {
            if m == Material::Lava {
                self.mix_effects(p);
            } else {
                self.drop_block_items(p, old);
            }
        }
        self.set_block_notify(p, id << 4 | level as u16);
    }

    /// Whether liquid could flow sideways into `p` (not blocked, and not a source of
    /// the same liquid).
    fn open_for_flow(&mut self, p: (i32, i32, i32), m: Material) -> bool {
        if self.liquid_blocked(p) {
            return false;
        }
        let s = self.state_at(p);
        material(s >> 4) != m || s & 15 > 0
    }

    /// `BlockDynamicLiquid.getPossibleFlowDirections`: the horizontal directions with
    /// the shortest way to a drop.
    fn flow_directions(&mut self, pos: (i32, i32, i32), m: Material) -> Vec<(i32, i32)> {
        let mut best = NO_SLOPE;
        let mut found = [false; 4];
        for (k, d) in HORIZONTAL.into_iter().enumerate() {
            let p = add(pos, d);
            if !self.open_for_flow(p, m) {
                continue;
            }
            let cost = if self.liquid_blocked(down(p)) {
                self.slope_distance(p, 1, (-d.0, -d.1), m)
            } else {
                0
            };
            if cost < best {
                found = [false; 4];
            }
            if cost <= best {
                found[k] = true;
                best = cost;
            }
        }
        FLOW_ORDER
            .into_iter()
            .filter(|d| {
                HORIZONTAL
                    .iter()
                    .position(|h| h == d)
                    .is_some_and(|k| found[k])
            })
            .collect()
    }

    /// `BlockDynamicLiquid.func_176374_a`: the distance to the nearest drop, searching
    /// up to [`MAX_SLOPE_DISTANCE`] blocks without going back the way it came.
    fn slope_distance(
        &mut self,
        pos: (i32, i32, i32),
        distance: i32,
        from: (i32, i32),
        m: Material,
    ) -> i32 {
        let mut best = NO_SLOPE;
        for d in HORIZONTAL {
            if d == from {
                continue;
            }
            let p = add(pos, d);
            if !self.open_for_flow(p, m) {
                continue;
            }
            if !self.liquid_blocked(down(p)) {
                return distance;
            }
            if distance < MAX_SLOPE_DISTANCE {
                let c = self.slope_distance(p, distance + 1, (-d.0, -d.1), m);
                if c < best {
                    best = c;
                }
            }
        }
        best
    }
}

/// How strongly flowing water pushes entities (`handleMaterialAcceleration`).
const WATER_PUSH: f64 = 0.014;
/// `Vec3.normalize` returns zero below this length.
const MIN_NORMAL_LENGTH: f64 = 1.0e-4;
/// The downward pull added to falling water next to a wall (`getFlowVector`).
const FALLING_PULL: f64 = -6.0;

fn normalize(v: (f64, f64, f64)) -> (f64, f64, f64) {
    let len = (v.0 * v.0 + v.1 * v.1 + v.2 * v.2).sqrt();
    if len < MIN_NORMAL_LENGTH {
        (0.0, 0.0, 0.0)
    } else {
        (v.0 / len, v.1 / len, v.2 / len)
    }
}

/// `BlockLiquid.getLiquidHeightPercent`: how much of the block a level leaves empty.
pub fn liquid_height_percent(level: u16) -> f32 {
    let level = if i32::from(level) >= FALLING {
        0
    } else {
        level
    };
    f32::from(level + 1) / 9.0
}

/// `BlockLiquid.getEffectiveFlowDecay`: the level (falling counts as 0), or -1 if the
/// block is another material.
fn effective_decay(world: &mut World, p: (i32, i32, i32), m: Material) -> i32 {
    let s = world.block(p.0, p.1, p.2);
    if material(s >> 4) != m {
        return NOT_LIQUID;
    }
    let level = i32::from(s & 15);
    if level >= FALLING { 0 } else { level }
}

/// `BlockLiquid.isBlockSolid` for a neighbor seen from the liquid along a horizontal
/// face: the same liquid and ice are not solid.
fn liquid_side_solid(world: &mut World, p: (i32, i32, i32), m: Material) -> bool {
    let other = material(world.block(p.0, p.1, p.2) >> 4);
    other != m && other != Material::Ice && other.is_solid()
}

/// `BlockLiquid.getFlowVector`: the normalized direction the liquid at `pos` flows.
fn flow_vector(world: &mut World, pos: (i32, i32, i32), m: Material) -> (f64, f64, f64) {
    let mut v = (0.0, 0.0, 0.0);
    let own = effective_decay(world, pos, m);
    for d in HORIZONTAL {
        let n = add(pos, d);
        let mut decay = effective_decay(world, n, m);
        let weight = if decay < 0 {
            if material(world.block(n.0, n.1, n.2) >> 4).blocks_movement() {
                continue;
            }
            decay = effective_decay(world, down(n), m);
            if decay < 0 {
                continue;
            }
            decay - (own - FALLING)
        } else {
            decay - own
        };
        v.0 += f64::from(d.0 * weight);
        v.2 += f64::from(d.1 * weight);
    }
    if i32::from(world.block(pos.0, pos.1, pos.2) & 15) >= FALLING {
        for d in HORIZONTAL {
            let n = add(pos, d);
            if liquid_side_solid(world, n, m) || liquid_side_solid(world, (n.0, n.1 + 1, n.2), m) {
                let u = normalize(v);
                v = (u.0, u.1 + FALLING_PULL, u.2);
                break;
            }
        }
    }
    normalize(v)
}

/// `World.handleMaterialAcceleration(bb, water, entity)`: whether the box touches water
/// (by its surface height), and the push it gets from the flow (zero if `pushed` is
/// false or nothing flows).
pub fn water_movement(
    world: &mut World,
    bb: &super::physics::Aabb,
    pushed: bool,
) -> (bool, (f64, f64, f64)) {
    let floor = krol_java::math::floor_f64;
    let (x0, x1) = (floor(bb.min[0]), floor(bb.max[0] + 1.0));
    let (y0, y1) = (floor(bb.min[1]), floor(bb.max[1] + 1.0));
    let (z0, z1) = (floor(bb.min[2]), floor(bb.max[2] + 1.0));
    let mut inside = false;
    let mut acc = (0.0, 0.0, 0.0);
    for x in x0..x1 {
        for y in y0..y1 {
            for z in z0..z1 {
                let s = world.block(x, y, z);
                if material(s >> 4) != Material::Water {
                    continue;
                }
                let surface = f64::from((y + 1) as f32 - liquid_height_percent(s & 15));
                if f64::from(y1) >= surface {
                    inside = true;
                    let f = flow_vector(world, (x, y, z), Material::Water);
                    acc = (acc.0 + f.0, acc.1 + f.1, acc.2 + f.2);
                }
            }
        }
    }
    let len = (acc.0 * acc.0 + acc.1 * acc.1 + acc.2 * acc.2).sqrt();
    if len > 0.0 && pushed {
        let n = normalize(acc);
        return (
            inside,
            (n.0 * WATER_PUSH, n.1 * WATER_PUSH, n.2 * WATER_PUSH),
        );
    }
    (inside, (0.0, 0.0, 0.0))
}
