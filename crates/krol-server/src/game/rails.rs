//! Rails (vanilla: `BlockRailBase` with its `Rail` helper, `BlockRail`,
//! `BlockRailPowered` for powered and activator rails, `BlockRailDetector`).
//!
//! A rail needs a solid top below it (and an ascending rail one on its high side). When
//! placed it picks a shape from the rails around it that are free to connect, then bends
//! those neighbors towards itself. Plain rails can curve; the others only run straight
//! or up slopes. Powered and activator rails pass power on along up to 8 rails of their
//! own kind; detector rails are powered by minecarts on them.
//!
//! TODO(M5): detector rails seeing minecarts (they never find one yet).

use super::Game;
use super::ids::block;
use super::placement::solid_top;

/// The shapes (`EnumRailDirection` metadata).
pub const NORTH_SOUTH: u16 = 0;
pub const EAST_WEST: u16 = 1;
pub const ASCENDING_EAST: u16 = 2;
pub const ASCENDING_WEST: u16 = 3;
pub const ASCENDING_NORTH: u16 = 4;
pub const ASCENDING_SOUTH: u16 = 5;
pub const SOUTH_EAST: u16 = 6;
pub const SOUTH_WEST: u16 = 7;
pub const NORTH_WEST: u16 = 8;
pub const NORTH_EAST: u16 = 9;
/// The powered bit of powered, activator and detector rails.
pub const POWERED_BIT: u16 = 8;
/// Powered and activator rails pass power along at most this many rails.
const MAX_POWER_DISTANCE: i32 = 8;
/// Detector rails check for minecarts again this often while powered.
const DETECTOR_TICK_RATE: i32 = 20;

type Pos = (i32, i32, i32);

/// `BlockRailBase.isRailBlock`.
pub fn is_rail(id: u16) -> bool {
    matches!(
        id,
        block::RAIL | block::GOLDEN_RAIL | block::DETECTOR_RAIL | block::ACTIVATOR_RAIL
    )
}

/// Rails that only run straight (`BlockRailBase.isPowered`: all but the plain rail).
fn straight_only(id: u16) -> bool {
    id != block::RAIL
}

/// The shape of a rail state (`byMetadata`: unknown values read as north-south).
pub fn shape(state: u16) -> u16 {
    let (id, meta) = (state >> 4, state & 15);
    let s = if straight_only(id) { meta & 7 } else { meta };
    if s > NORTH_EAST { NORTH_SOUTH } else { s }
}

/// The state with another shape, keeping the powered bit.
fn with_shape(state: u16, shape: u16) -> u16 {
    let id = state >> 4;
    let keep = if straight_only(id) {
        state & POWERED_BIT
    } else {
        0
    };
    id << 4 | keep | shape
}

/// Ascending shapes (2-5).
pub fn is_ascending(shape: u16) -> bool {
    (ASCENDING_EAST..=ASCENDING_SOUTH).contains(&shape)
}

fn north(p: Pos) -> Pos {
    (p.0, p.1, p.2 - 1)
}
fn south(p: Pos) -> Pos {
    (p.0, p.1, p.2 + 1)
}
fn west(p: Pos) -> Pos {
    (p.0 - 1, p.1, p.2)
}
fn east(p: Pos) -> Pos {
    (p.0 + 1, p.1, p.2)
}
fn up(p: Pos) -> Pos {
    (p.0, p.1 + 1, p.2)
}
fn down(p: Pos) -> Pos {
    (p.0, p.1 - 1, p.2)
}

/// The two positions a shape connects (`func_180360_a`).
fn links_of(pos: Pos, shape: u16) -> Vec<Pos> {
    match shape {
        EAST_WEST => vec![west(pos), east(pos)],
        ASCENDING_EAST => vec![west(pos), up(east(pos))],
        ASCENDING_WEST => vec![up(west(pos)), east(pos)],
        ASCENDING_NORTH => vec![up(north(pos)), south(pos)],
        ASCENDING_SOUTH => vec![north(pos), up(south(pos))],
        SOUTH_EAST => vec![east(pos), south(pos)],
        SOUTH_WEST => vec![west(pos), south(pos)],
        NORTH_WEST => vec![west(pos), north(pos)],
        NORTH_EAST => vec![east(pos), north(pos)],
        _ => vec![north(pos), south(pos)],
    }
}

/// One rail and the positions it connects to (vanilla `BlockRailBase.Rail`).
#[derive(Debug, Clone)]
struct Rail {
    pos: Pos,
    state: u16,
    links: Vec<Pos>,
}

impl Rail {
    fn new(pos: Pos, state: u16) -> Self {
        Self {
            pos,
            state,
            links: links_of(pos, shape(state)),
        }
    }

    /// `func_180363_c`: a link above, at or below `p`.
    fn links_column(&self, p: Pos) -> bool {
        self.links.iter().any(|l| l.0 == p.0 && l.2 == p.2)
    }

    /// `func_150649_b`: whether it can take `other` as a link (it already points
    /// there, or it is not connected at both ends).
    fn can_link(&self, other: &Rail) -> bool {
        self.links_column(other.pos) || self.links.len() != 2
    }
}

impl Game {
    fn rail_block(&mut self, p: Pos) -> bool {
        is_rail(self.level.overworld.block(p.0, p.1, p.2) >> 4)
    }

    /// `findRailAt`: a rail at `p`, above it or below it.
    fn rail_at(&mut self, p: Pos) -> Option<Rail> {
        for q in [p, up(p), down(p)] {
            let state = self.level.overworld.block(q.0, q.1, q.2);
            if is_rail(state >> 4) {
                return Some(Rail::new(q, state));
            }
        }
        None
    }

    /// `func_150651_b`: keeps only the links that lead to a rail pointing back.
    fn refresh_links(&mut self, r: &mut Rail) {
        let mut k = 0;
        while k < r.links.len() {
            match self.rail_at(r.links[k]) {
                Some(other) if other.links_column(r.pos) => {
                    r.links[k] = other.pos;
                    k += 1;
                }
                _ => {
                    r.links.remove(k);
                }
            }
        }
    }

    /// `func_180361_d`: whether the rail at `p` would connect to `r`.
    fn rail_would_link(&mut self, p: Pos, r: &Rail) -> bool {
        let Some(mut other) = self.rail_at(p) else {
            return false;
        };
        self.refresh_links(&mut other);
        other.can_link(r)
    }

    /// `countAdjacentRails`: the horizontal neighbors with a rail at, above or below
    /// them.
    fn adjacent_rails(&mut self, pos: Pos) -> usize {
        [north(pos), east(pos), south(pos), west(pos)]
            .into_iter()
            .filter(|&p| self.rail_block(p) || self.rail_block(up(p)) || self.rail_block(down(p)))
            .count()
    }

    /// Turns a straight shape into a slope towards a rail one block higher.
    fn slope(&mut self, pos: Pos, shape: u16) -> u16 {
        let mut s = shape;
        if shape == NORTH_SOUTH {
            if self.rail_block(up(north(pos))) {
                s = ASCENDING_NORTH;
            }
            if self.rail_block(up(south(pos))) {
                s = ASCENDING_SOUTH;
            }
        }
        if shape == EAST_WEST {
            if self.rail_block(up(east(pos))) {
                s = ASCENDING_EAST;
            }
            if self.rail_block(up(west(pos))) {
                s = ASCENDING_WEST;
            }
        }
        s
    }

    /// `func_150645_c`: `r` takes `to` as a link and reshapes to match.
    fn link_rail(&mut self, r: &mut Rail, to: Pos) {
        r.links.push(to);
        let pos = r.pos;
        let (n, s, w, e) = (
            r.links_column(north(pos)),
            r.links_column(south(pos)),
            r.links_column(west(pos)),
            r.links_column(east(pos)),
        );
        let mut shape = None;
        if n || s {
            shape = Some(NORTH_SOUTH);
        }
        if w || e {
            shape = Some(EAST_WEST);
        }
        if !straight_only(r.state >> 4) {
            if s && e && !n && !w {
                shape = Some(SOUTH_EAST);
            }
            if s && w && !n && !e {
                shape = Some(SOUTH_WEST);
            }
            if n && w && !s && !e {
                shape = Some(NORTH_WEST);
            }
            if n && e && !s && !w {
                shape = Some(NORTH_EAST);
            }
        }
        let shape = match shape {
            Some(sh) => self.slope(pos, sh),
            None => NORTH_SOUTH,
        };
        r.state = with_shape(r.state, shape);
        self.set_block_notify(pos, r.state);
    }

    /// `func_180364_a`: the rail at `r` picks its shape from the neighbors that would
    /// connect (`powered` decides which way a plain rail at a T-junction curves), and on
    /// a change (or always when `initial`) bends those neighbors towards itself.
    fn reshape_rail(&mut self, mut r: Rail, powered: bool, initial: bool) -> Rail {
        let pos = r.pos;
        let n = self.rail_would_link(north(pos), &r);
        let s = self.rail_would_link(south(pos), &r);
        let w = self.rail_would_link(west(pos), &r);
        let e = self.rail_would_link(east(pos), &r);
        let curves = !straight_only(r.state >> 4);
        let mut shape = None;
        if (n || s) && !w && !e {
            shape = Some(NORTH_SOUTH);
        }
        if (w || e) && !n && !s {
            shape = Some(EAST_WEST);
        }
        if curves {
            if s && e && !n && !w {
                shape = Some(SOUTH_EAST);
            }
            if s && w && !n && !e {
                shape = Some(SOUTH_WEST);
            }
            if n && w && !s && !e {
                shape = Some(NORTH_WEST);
            }
            if n && e && !s && !w {
                shape = Some(NORTH_EAST);
            }
        }
        if shape.is_none() {
            if n || s {
                shape = Some(NORTH_SOUTH);
            }
            if w || e {
                shape = Some(EAST_WEST);
            }
            if curves {
                // At a T-junction the last match wins, in an order that depends on
                // whether the rail is powered.
                let corners = if powered {
                    [
                        (s && e, SOUTH_EAST),
                        (w && s, SOUTH_WEST),
                        (e && n, NORTH_EAST),
                        (n && w, NORTH_WEST),
                    ]
                } else {
                    [
                        (n && w, NORTH_WEST),
                        (e && n, NORTH_EAST),
                        (w && s, SOUTH_WEST),
                        (s && e, SOUTH_EAST),
                    ]
                };
                for (hit, corner) in corners {
                    if hit {
                        shape = Some(corner);
                    }
                }
            }
        }
        let shape = match shape {
            Some(sh) => self.slope(pos, sh),
            None => NORTH_SOUTH,
        };
        r.links = links_of(pos, shape);
        r.state = with_shape(r.state, shape);
        if initial || self.level.overworld.block(pos.0, pos.1, pos.2) != r.state {
            self.set_block_notify(pos, r.state);
            for k in 0..r.links.len() {
                if let Some(mut other) = self.rail_at(r.links[k]) {
                    self.refresh_links(&mut other);
                    if other.can_link(&r) {
                        self.link_rail(&mut other, pos);
                    }
                }
            }
        }
        r
    }

    /// `func_176564_a`: reshapes the rail at `pos` (powered or not decides T-junctions).
    fn update_rail_state(&mut self, pos: Pos, state: u16, initial: bool) -> u16 {
        let powered = self.is_block_powered(pos);
        self.reshape_rail(Rail::new(pos, state), powered, initial)
            .state
    }

    /// `BlockRailBase.onBlockAdded` (and the detector rail's minecart check).
    pub(super) fn rail_added(&mut self, pos: Pos, state: u16) {
        let state = self.update_rail_state(pos, state, true);
        let id = state >> 4;
        if straight_only(id) {
            self.rail_neighbor_changed(pos, state, id);
        }
        if id == block::DETECTOR_RAIL {
            let now = self.level.overworld.block(pos.0, pos.1, pos.2);
            self.detector_update(pos, now);
        }
    }

    /// `BlockRailBase.onNeighborBlockChange`: a rail without its support (below, and on
    /// the high side of a slope) pops off; otherwise plain rails at a junction re-check
    /// their shape when a power source changes, powered and activator rails their power.
    pub(super) fn rail_neighbor_changed(&mut self, pos: Pos, state: u16, source: u16) {
        let solid = |g: &mut Self, p: Pos| solid_top(g.level.overworld.block(p.0, p.1, p.2));
        let mut drop = !solid(self, down(pos));
        let high_side = match shape(state) {
            ASCENDING_EAST => Some(east(pos)),
            ASCENDING_WEST => Some(west(pos)),
            ASCENDING_NORTH => Some(north(pos)),
            ASCENDING_SOUTH => Some(south(pos)),
            _ => None,
        };
        if let Some(p) = high_side
            && !solid(self, p)
        {
            drop = true;
        }
        if drop {
            self.pop_off_block(pos, state);
            return;
        }
        match state >> 4 {
            block::RAIL => {
                if self.block_provides_power(source) && self.adjacent_rails(pos) == 3 {
                    self.update_rail_state(pos, state, false);
                }
            }
            block::GOLDEN_RAIL | block::ACTIVATOR_RAIL => self.powered_rail_check(pos, state),
            _ => {}
        }
    }

    /// `BlockRailPowered.onNeighborChangedInternal`: powered by redstone or by a
    /// powered rail of its kind within 8 rails along the track.
    fn powered_rail_check(&mut self, pos: Pos, state: u16) {
        let was = state & POWERED_BIT != 0;
        let now = self.is_block_powered(pos)
            || self.rail_power_along(pos, state, true, 0)
            || self.rail_power_along(pos, state, false, 0);
        if now != was {
            let id = state >> 4;
            let new = if now {
                state | POWERED_BIT
            } else {
                state & !POWERED_BIT
            };
            self.set_block_notify(pos, new);
            self.notify_neighbors(down(pos), id);
            if is_ascending(shape(state)) {
                self.notify_neighbors(up(pos), id);
            }
        }
    }

    /// `func_176566_a`: the next rail along the track in one direction (`forward`:
    /// south, west, or up a slope's low end) passes power on.
    fn rail_power_along(&mut self, pos: Pos, state: u16, forward: bool, depth: i32) -> bool {
        if depth >= MAX_POWER_DISTANCE {
            return false;
        }
        let (mut x, mut y, mut z) = pos;
        let mut try_below = true;
        let mut sh = shape(state);
        match sh {
            NORTH_SOUTH => z += if forward { 1 } else { -1 },
            EAST_WEST => x += if forward { -1 } else { 1 },
            ASCENDING_EAST => {
                if forward {
                    x -= 1;
                } else {
                    x += 1;
                    y += 1;
                    try_below = false;
                }
                sh = EAST_WEST;
            }
            ASCENDING_WEST => {
                if forward {
                    x -= 1;
                    y += 1;
                    try_below = false;
                } else {
                    x += 1;
                }
                sh = EAST_WEST;
            }
            ASCENDING_NORTH => {
                if forward {
                    z += 1;
                } else {
                    z -= 1;
                    y += 1;
                    try_below = false;
                }
                sh = NORTH_SOUTH;
            }
            ASCENDING_SOUTH => {
                if forward {
                    z += 1;
                    y += 1;
                    try_below = false;
                } else {
                    z -= 1;
                }
                sh = NORTH_SOUTH;
            }
            _ => {}
        }
        let id = state >> 4;
        self.rail_gives_power((x, y, z), id, forward, depth, sh)
            || (try_below && self.rail_gives_power((x, y - 1, z), id, forward, depth, sh))
    }

    /// `func_176567_a`: a powered rail of the same kind at `p`, running the same way
    /// as `along`, that is powered by redstone or by the next rail on.
    fn rail_gives_power(&mut self, p: Pos, id: u16, forward: bool, depth: i32, along: u16) -> bool {
        let state = self.level.overworld.block(p.0, p.1, p.2);
        if state >> 4 != id {
            return false;
        }
        let other = shape(state);
        let crosses = (along == EAST_WEST
            && matches!(other, NORTH_SOUTH | ASCENDING_NORTH | ASCENDING_SOUTH))
            || (along == NORTH_SOUTH
                && matches!(other, EAST_WEST | ASCENDING_EAST | ASCENDING_WEST));
        if crosses || state & POWERED_BIT == 0 {
            return false;
        }
        self.is_block_powered(p) || self.rail_power_along(p, state, forward, depth + 1)
    }

    /// `BlockRailBase.breakBlock`: a slope updates above it, the straight-only rails
    /// around and below themselves.
    pub(super) fn rail_removed(&mut self, pos: Pos, old: u16) {
        let id = old >> 4;
        if is_ascending(shape(old)) {
            self.notify_neighbors(up(pos), id);
        }
        if straight_only(id) {
            self.notify_neighbors(pos, id);
            self.notify_neighbors(down(pos), id);
        }
    }

    /// Whether a minecart is in a detector rail's box (`findMinecarts`).
    ///
    /// TODO(M5): minecarts.
    fn minecart_on_detector(&mut self, _pos: Pos) -> bool {
        false
    }

    /// `BlockRailDetector.updatePoweredState`: powered while a minecart is on it,
    /// re-checked every 20 ticks.
    pub(super) fn detector_update(&mut self, pos: Pos, state: u16) {
        if state >> 4 != block::DETECTOR_RAIL {
            return;
        }
        let was = state & POWERED_BIT != 0;
        let now = self.minecart_on_detector(pos);
        if now != was {
            let new = if now {
                state | POWERED_BIT
            } else {
                state & !POWERED_BIT
            };
            self.set_block_notify(pos, new);
            self.notify_neighbors(pos, block::DETECTOR_RAIL);
            self.notify_neighbors(down(pos), block::DETECTOR_RAIL);
        }
        if now {
            self.schedule_update(pos, block::DETECTOR_RAIL, DETECTOR_TICK_RATE);
        }
        self.update_comparators(pos);
    }

    /// `BlockRailDetector.updateTick`: a powered detector rail looks again.
    pub(super) fn detector_tick(&mut self, pos: Pos, state: u16) {
        if state & POWERED_BIT != 0 {
            self.detector_update(pos, state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_keep_the_powered_bit() {
        let golden = block::GOLDEN_RAIL << 4 | POWERED_BIT | NORTH_SOUTH;
        assert_eq!(with_shape(golden, EAST_WEST), golden | EAST_WEST);
        assert_eq!(shape(golden | ASCENDING_SOUTH), ASCENDING_SOUTH);
        assert_eq!(shape(block::RAIL << 4 | NORTH_EAST), NORTH_EAST);
        // Unknown shapes read as north-south.
        assert_eq!(shape(block::RAIL << 4 | 12), NORTH_SOUTH);
    }

    #[test]
    fn slopes_link_up_their_high_side() {
        let links = links_of((0, 64, 0), ASCENDING_EAST);
        assert_eq!(links, vec![(-1, 64, 0), (1, 65, 0)]);
    }
}
