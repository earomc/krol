//! Vines (vanilla `BlockVine`): growing up, down and sideways on random ticks, and
//! falling off when nothing holds them.

use super::Game;
use super::block_data::{Material, material};
use super::block_update::flags;

/// `vine`.
pub const VINE: u16 = 106;
/// The side bits in vine metadata: the side the vine hangs on.
const SOUTH: u16 = 1;
const WEST: u16 = 2;
const NORTH: u16 = 4;
const EAST: u16 = 8;
/// Vines grow 1 random tick in 4.
const GROW_CHANCE: i32 = 4;
/// Vines stop spreading sideways and up once this many are close.
const CROWD: i32 = 5;
const CROWD_RADIUS: i32 = 4;

/// A horizontal direction: `(dx, dz, side bit)`.
type Dir = (i32, i32, u16);
const N: Dir = (0, -1, NORTH);
const E: Dir = (1, 0, EAST);
const S: Dir = (0, 1, SOUTH);
const W: Dir = (-1, 0, WEST);
/// `EnumFacing.Plane.HORIZONTAL` order.
const HORIZONTAL: [Dir; 4] = [N, E, S, W];

/// `EnumFacing.rotateY` (clockwise seen from above).
fn rotate_cw(d: Dir) -> Dir {
    match d.2 {
        NORTH => E,
        EAST => S,
        SOUTH => W,
        _ => N,
    }
}

/// `EnumFacing.rotateYCCW`.
fn rotate_ccw(d: Dir) -> Dir {
    match d.2 {
        NORTH => W,
        WEST => S,
        SOUTH => E,
        _ => N,
    }
}

fn opposite(d: Dir) -> Dir {
    rotate_cw(rotate_cw(d))
}

fn offset(p: (i32, i32, i32), d: Dir) -> (i32, i32, i32) {
    (p.0 + d.0, p.1, p.2 + d.1)
}

/// A full, solid cube (`isFullCube() && blocksMovement()`); leaves count.
fn full_solid(state: u16) -> bool {
    let id = state >> 4;
    if !material(id).blocks_movement() {
        return false;
    }
    let boxes = super::physics::block_boxes(state, (0, 0, 0));
    boxes.len() == 1 && boxes[0].min == [0.0; 3] && boxes[0].max == [1.0; 3]
}

impl Game {
    fn vine_state(&mut self, p: (i32, i32, i32)) -> u16 {
        self.level.overworld.block(p.0, p.1, p.2)
    }

    /// `BlockVine.canPlaceOn` for the block at `p`.
    fn vine_can_hold(&mut self, p: (i32, i32, i32)) -> bool {
        let s = self.vine_state(p);
        full_solid(s)
    }

    fn is_air(&mut self, p: (i32, i32, i32)) -> bool {
        material(self.vine_state(p) >> 4) == Material::Air
    }

    /// `BlockVine.onNeighborBlockChange` / `recheckGrownSides`: sides without a block
    /// to hang on (and no vine side above to hang from) fall away; no side left, and
    /// the vine drops.
    pub(super) fn vine_neighbor_changed(&mut self, pos: (i32, i32, i32), state: u16) {
        let mut sides = state & 15;
        for d in HORIZONTAL {
            if sides & d.2 != 0 && !self.vine_can_hold(offset(pos, d)) {
                let above = self.vine_state((pos.0, pos.1 + 1, pos.2));
                if above >> 4 != VINE || above & d.2 == 0 {
                    sides &= !d.2;
                }
            }
        }
        // The "up" face is not part of the stored state, so it never counts here.
        if sides == 0 {
            self.drop_block_items(pos, state);
            self.set_block_notify(pos, 0);
        } else if sides != state & 15 {
            self.set_block_flags(pos, VINE << 4 | sides, flags::SEND);
        }
    }

    /// `BlockVine.updateTick` (its random tick).
    pub(super) fn vine_tick(&mut self, pos: (i32, i32, i32), state: u16) {
        if self.world_rand.next_int_bounded(GROW_CHANCE) != 0 {
            return;
        }
        let mut left = CROWD;
        let mut crowded = false;
        'count: for dx in -CROWD_RADIUS..=CROWD_RADIUS {
            for dz in -CROWD_RADIUS..=CROWD_RADIUS {
                for dy in -1..=1 {
                    if self.vine_state((pos.0 + dx, pos.1 + dy, pos.2 + dz)) >> 4 == VINE {
                        left -= 1;
                        if left <= 0 {
                            crowded = true;
                            break 'count;
                        }
                    }
                }
            }
        }
        // EnumFacing.random: down, up, north, south, west, east.
        let facing = self.world_rand.next_int_bounded(6);
        let up = (pos.0, pos.1 + 1, pos.2);
        let sides = state & 15;
        let dir = match facing {
            2 => Some(N),
            3 => Some(S),
            4 => Some(W),
            5 => Some(E),
            _ => None,
        };
        if facing == 1 && pos.1 < 255 && self.is_air(up) {
            if !crowded {
                let mut grown = sides;
                for d in HORIZONTAL {
                    if self.world_rand.next_boolean() || !self.vine_can_hold(offset(up, d)) {
                        grown &= !d.2;
                    }
                }
                if grown != 0 {
                    self.set_block_flags(up, VINE << 4 | grown, flags::SEND);
                }
            }
        } else if dir.is_none_or(|d| sides & d.2 != 0) {
            if pos.1 > 1 {
                let below = (pos.0, pos.1 - 1, pos.2);
                let under = self.vine_state(below);
                if material(under >> 4) == Material::Air {
                    let mut grown = sides;
                    for d in HORIZONTAL {
                        if self.world_rand.next_boolean() {
                            grown &= !d.2;
                        }
                    }
                    if grown != 0 {
                        self.set_block_flags(below, VINE << 4 | grown, flags::SEND);
                    }
                } else if under >> 4 == VINE {
                    let mut grown = under & 15;
                    for d in HORIZONTAL {
                        if self.world_rand.next_boolean() && sides & d.2 != 0 {
                            grown |= d.2;
                        }
                    }
                    if grown != 0 {
                        self.set_block_flags(below, VINE << 4 | grown, flags::SEND);
                    }
                }
            }
        } else if let Some(d) = dir
            && !crowded
        {
            let next = offset(pos, d);
            let there = self.vine_state(next);
            if material(there >> 4) == Material::Air {
                let (cw, ccw) = (rotate_cw(d), rotate_ccw(d));
                let has_cw = sides & cw.2 != 0;
                let has_ccw = sides & ccw.2 != 0;
                let next_cw = offset(next, cw);
                let next_ccw = offset(next, ccw);
                if has_cw && self.vine_can_hold(next_cw) {
                    self.set_block_flags(next, VINE << 4 | cw.2, flags::SEND);
                } else if has_ccw && self.vine_can_hold(next_ccw) {
                    self.set_block_flags(next, VINE << 4 | ccw.2, flags::SEND);
                } else if has_cw && self.is_air(next_cw) && self.vine_can_hold(offset(pos, cw)) {
                    self.set_block_flags(next_cw, VINE << 4 | opposite(d).2, flags::SEND);
                } else if has_ccw && self.is_air(next_ccw) && self.vine_can_hold(offset(pos, ccw)) {
                    self.set_block_flags(next_ccw, VINE << 4 | opposite(d).2, flags::SEND);
                } else if self.vine_can_hold((next.0, next.1 + 1, next.2)) {
                    self.set_block_flags(next, VINE << 4, flags::SEND);
                }
            } else if material(there >> 4).is_opaque() && full_solid(there) {
                self.set_block_flags(pos, state | d.2, flags::SEND);
            }
        }
    }
}
