//! Fire (vanilla `BlockFire`): ageing, burning out, spreading to flammable blocks,
//! dying in the rain, and lighting it with flint and steel.

use super::Game;
use super::block_data::{Material, material};
use super::block_update::flags;
use super::ids::block;
use super::placement::solid_top;

/// `BlockFire.tickRate`.
const TICK_RATE: i32 = 30;
/// The oldest fire.
const MAX_AGE: u16 = 15;
/// Chance base for burning the blocks beside (`catchOnFire`, 300) and above/below (250).
const SIDE_BURN_CHANCE: i32 = 300;
const VERTICAL_BURN_CHANCE: i32 = 250;
/// Humid biomes make burning and spreading less likely.
const HUMID_PENALTY: i32 = 50;
/// `netherrack`: fire on it burns forever.
const NETHERRACK: u16 = 87;

/// `(encouragement, flammability)` of a block (`BlockFire.setFireInfo`): how readily
/// fire spreads next to it, and how readily it burns away.
pub fn fire_info(id: u16) -> (i32, i32) {
    match id {
        // Planks, wooden slabs, fences, fence gates and the oak, spruce, birch and jungle
        // stairs (vanilla quirk: acacia and dark oak stairs do not burn).
        5 | 125 | 126 | 85 | 107 | 183..=192 | 53 | 134..=136 => (5, 20),
        17 | 162 => (5, 5),                   // logs
        18 | 161 => (30, 60),                 // leaves
        47 => (30, 20),                       // bookshelf
        46 => (15, 100),                      // tnt
        31 | 175 | 37 | 38 | 32 => (60, 100), // grass, double plants, flowers, dead bush
        35 => (30, 60),                       // wool
        106 => (15, 100),                     // vine
        173 => (5, 5),                        // coal block
        170 => (60, 20),                      // hay
        171 => (60, 20),                      // carpet
        _ => (0, 0),
    }
}

impl Game {
    fn fire_block(&mut self, p: (i32, i32, i32)) -> u16 {
        self.level.overworld.block(p.0, p.1, p.2)
    }

    /// `BlockFire.canCatchFire`.
    fn can_catch_fire(&mut self, p: (i32, i32, i32)) -> bool {
        fire_info(self.fire_block(p) >> 4).0 > 0
    }

    /// `BlockFire.canNeighborCatchFire`: something flammable on any side.
    fn neighbor_can_catch_fire(&mut self, p: (i32, i32, i32)) -> bool {
        [
            (0, -1, 0),
            (0, 1, 0),
            (0, 0, -1),
            (0, 0, 1),
            (-1, 0, 0),
            (1, 0, 0),
        ]
        .iter()
        .any(|&(dx, dy, dz)| self.can_catch_fire((p.0 + dx, p.1 + dy, p.2 + dz)))
    }

    /// `BlockFire.canPlaceBlockAt`: on a solid top or next to something flammable.
    pub(super) fn fire_can_stay(&mut self, p: (i32, i32, i32)) -> bool {
        let below = self.fire_block((p.0, p.1 - 1, p.2));
        solid_top(below) || self.neighbor_can_catch_fire(p)
    }

    /// `BlockFire.canDie`: rain falls on the fire or next to it.
    fn fire_can_die(&mut self, p: (i32, i32, i32)) -> bool {
        [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)]
            .iter()
            .any(|&(dx, dz)| self.rains_at((p.0 + dx, p.1, p.2 + dz)))
    }

    /// `BlockFire.onBlockAdded`.
    ///
    /// TODO(M5): lighting a nether portal frame (`BlockPortal.func_176548_d`).
    pub(super) fn fire_added(&mut self, p: (i32, i32, i32)) {
        if self.fire_can_stay(p) {
            let delay = TICK_RATE + self.world_rand.next_int_bounded(10);
            self.schedule_update(p, block::FIRE, delay);
        } else {
            self.set_block_notify(p, 0);
        }
    }

    /// `BlockFire.onNeighborBlockChange`.
    pub(super) fn fire_neighbor_changed(&mut self, p: (i32, i32, i32)) {
        if !self.fire_can_stay(p) {
            self.set_block_notify(p, 0);
        }
    }

    /// `BlockFire.updateTick` (also its random tick).
    pub(super) fn fire_tick(&mut self, p: (i32, i32, i32), state: u16) {
        if self.level.data.game_rule("doFireTick") == Some("false") {
            return;
        }
        if !self.fire_can_stay(p) {
            self.set_block_notify(p, 0);
        }
        // TODO(M6): bedrock in the End burns forever too.
        let forever = self.fire_block((p.0, p.1 - 1, p.2)) >> 4 == NETHERRACK;
        if !forever && self.is_raining() && self.fire_can_die(p) {
            self.set_block_notify(p, 0);
            return;
        }
        let age = state & 15;
        if age < MAX_AGE {
            let older = block::FIRE << 4 | (age + (self.world_rand.next_int_bounded(3) / 2) as u16);
            self.set_block_flags(p, older, 0);
        }
        let delay = TICK_RATE + self.world_rand.next_int_bounded(10);
        self.schedule_update(p, block::FIRE, delay);
        if !forever {
            if !self.neighbor_can_catch_fire(p) {
                let below = self.fire_block((p.0, p.1 - 1, p.2));
                if !solid_top(below) || age > 3 {
                    self.set_block_notify(p, 0);
                }
                return;
            }
            if !self.can_catch_fire((p.0, p.1 - 1, p.2))
                && age == MAX_AGE
                && self.world_rand.next_int_bounded(4) == 0
            {
                self.set_block_notify(p, 0);
                return;
            }
        }
        let humid = self.biome_at(p.0, p.2).humid;
        let penalty = if humid { -HUMID_PENALTY } else { 0 };
        let age = i32::from(age);
        for (d, chance) in [
            ((1, 0, 0), SIDE_BURN_CHANCE),
            ((-1, 0, 0), SIDE_BURN_CHANCE),
            ((0, -1, 0), VERTICAL_BURN_CHANCE),
            ((0, 1, 0), VERTICAL_BURN_CHANCE),
            ((0, 0, -1), SIDE_BURN_CHANCE),
            ((0, 0, 1), SIDE_BURN_CHANCE),
        ] {
            self.catch_on_fire((p.0 + d.0, p.1 + d.1, p.2 + d.2), chance + penalty, age);
        }
        let difficulty = i32::from(self.level.data.data.get_i8("Difficulty"));
        for dx in -1..=1 {
            for dz in -1..=1 {
                for dy in -1..=4 {
                    if dx == 0 && dy == 0 && dz == 0 {
                        continue;
                    }
                    let bound = if dy > 1 { 100 + (dy - 1) * 100 } else { 100 };
                    let q = (p.0 + dx, p.1 + dy, p.2 + dz);
                    let encouragement = self.neighbor_encouragement(q);
                    if encouragement <= 0 {
                        continue;
                    }
                    let mut spread = (encouragement + 40 + difficulty * 7) / (age + 30);
                    if humid {
                        spread /= 2;
                    }
                    if spread > 0
                        && self.world_rand.next_int_bounded(bound) <= spread
                        && !(self.is_raining() && self.fire_can_die(q))
                    {
                        let new_age = (age + self.world_rand.next_int_bounded(5) / 4).min(15);
                        self.set_block_notify(q, block::FIRE << 4 | new_age as u16);
                    }
                }
            }
        }
    }

    /// `BlockFire.catchOnFire`: a flammable neighbor may burn away or catch fire.
    fn catch_on_fire(&mut self, q: (i32, i32, i32), chance: i32, age: i32) {
        let old = self.fire_block(q);
        let flammability = fire_info(old >> 4).1;
        if self.world_rand.next_int_bounded(chance) >= flammability {
            return;
        }
        if self.world_rand.next_int_bounded(age + 10) < 5 && !self.rains_at(q) {
            let new_age = (age + self.world_rand.next_int_bounded(5) / 4).min(15);
            self.set_block_notify(q, block::FIRE << 4 | new_age as u16);
        } else {
            self.set_block_notify(q, 0);
        }
        // TODO(M5): burning TNT primes it.
    }

    /// `BlockFire.getNeighborEncouragement`: for an air block, the best encouragement
    /// around it.
    fn neighbor_encouragement(&mut self, q: (i32, i32, i32)) -> i32 {
        if material(self.fire_block(q) >> 4) != Material::Air {
            return 0;
        }
        [
            (0, -1, 0),
            (0, 1, 0),
            (0, 0, -1),
            (0, 0, 1),
            (-1, 0, 0),
            (1, 0, 0),
        ]
        .iter()
        .map(|&(dx, dy, dz)| fire_info(self.fire_block((q.0 + dx, q.1 + dy, q.2 + dz)) >> 4).0)
        .max()
        .unwrap_or(0)
    }

    /// The flags of `setBlockState(pos, state, 3)`, for other modules.
    pub(super) fn set_fire(&mut self, p: (i32, i32, i32)) {
        self.set_block_flags(p, block::FIRE << 4, flags::DEFAULT);
    }

    /// `ItemFlintAndSteel.onItemUse`: lights the air block on the clicked face and wears
    /// the item.
    pub(super) fn use_flint_and_steel(&mut self, i: usize, pos: (i32, i32, i32), face: u8) {
        let at = super::placement::offset(pos, face);
        if !(0..256).contains(&at.1) {
            return;
        }
        if material(self.fire_block(at) >> 4) == Material::Air {
            let pitch = self.world_rand.next_float() * IGNITE_PITCH_SPREAD + IGNITE_PITCH_BASE;
            let center = (
                f64::from(at.0) + 0.5,
                f64::from(at.1) + 0.5,
                f64::from(at.2) + 0.5,
            );
            self.play_sound_near_except(None, "fire.ignite", center, 1.0, pitch);
            self.set_fire(at);
        }
        self.damage_held_item(i, 1);
        self.count_item_use(i);
    }
}

/// `fire.ignite` pitch: base plus a random spread.
const IGNITE_PITCH_BASE: f32 = 0.8;
const IGNITE_PITCH_SPREAD: f32 = 0.4;
