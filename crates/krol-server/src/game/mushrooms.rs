//! Mushrooms (vanilla `BlockMushroom`): spreading on random ticks, popping off in
//! light, and growing into huge mushrooms with bone meal (`WorldGenBigMushroom`).

use super::Game;
use super::block_data::{Material, material};
use super::block_update::flags;
use super::ids::block;
use super::world::World;

/// `brown_mushroom_block` / `red_mushroom_block` (the huge mushroom blocks).
const BROWN_MUSHROOM_BLOCK: u16 = 99;
const RED_MUSHROOM_BLOCK: u16 = 100;
/// `BlockHugeMushroom.EnumType` metadata.
const CENTER: u16 = 5;
const NORTH_WEST: u16 = 1;
const NORTH_EAST: u16 = 3;
const SOUTH_WEST: u16 = 7;
const SOUTH_EAST: u16 = 9;
const STEM: u16 = 10;
const ALL_INSIDE: u16 = 0;
/// `dirt` metadata of podzol.
const PODZOL: u16 = 2;
/// Mushrooms need light below this on most blocks.
const MAX_LIGHT: i32 = 13;
/// A mushroom spreads 1 random tick in 25.
const SPREAD_ODDS: i32 = 25;
/// It stops spreading once this many mushrooms are close.
const CROWD: i32 = 5;
const CROWD_RADIUS: i32 = 4;
/// Bone meal grows a huge mushroom 40% of the time.
const BONE_MEAL_CHANCE: f64 = 0.4;

/// Whether `id` is a small mushroom.
pub fn is_mushroom(id: u16) -> bool {
    id == block::BROWN_MUSHROOM || id == block::RED_MUSHROOM
}

/// `BlockMushroom.canBlockStay`: on mycelium or podzol, or on a full block in light
/// below 13 (`World.getLight`, which ignores the time of day).
pub fn mushroom_can_stay(world: &mut World, pos: (i32, i32, i32)) -> bool {
    if !(0..256).contains(&pos.1) {
        return false;
    }
    let below = world.block(pos.0, pos.1 - 1, pos.2);
    if below >> 4 == block::MYCELIUM || below == block::DIRT << 4 | PODZOL {
        return true;
    }
    world.light_subtracted(pos, 0) < MAX_LIGHT && super::trees::full_block(below >> 4)
}

impl Game {
    fn shroom_block(&mut self, p: (i32, i32, i32)) -> u16 {
        self.level.overworld.block(p.0, p.1, p.2)
    }

    fn shroom_spot(&mut self, p: (i32, i32, i32)) -> bool {
        material(self.shroom_block(p) >> 4) == Material::Air
            && mushroom_can_stay(&mut self.level.overworld, p)
    }

    /// `BlockBush.checkAndDropBlock` with the mushroom rule.
    pub(super) fn check_mushroom(&mut self, pos: (i32, i32, i32), state: u16) {
        if !mushroom_can_stay(&mut self.level.overworld, pos) {
            self.pop_off_block(pos, state);
        }
    }

    /// `BlockMushroom.updateTick`: spreads to a random spot nearby unless crowded (no
    /// support check of its own here, unlike other bushes).
    pub(super) fn mushroom_tick(&mut self, pos: (i32, i32, i32), state: u16) {
        if self.world_rand.next_int_bounded(SPREAD_ODDS) != 0 {
            return;
        }
        let id = state >> 4;
        let mut left = CROWD;
        // BlockPos.getAllInBoxMutable: x fastest, then y, then z.
        for dz in -CROWD_RADIUS..=CROWD_RADIUS {
            for dy in -1..=1 {
                for dx in -CROWD_RADIUS..=CROWD_RADIUS {
                    if self.shroom_block((pos.0 + dx, pos.1 + dy, pos.2 + dz)) >> 4 == id {
                        left -= 1;
                        if left <= 0 {
                            return;
                        }
                    }
                }
            }
        }
        let mut from = pos;
        let mut next = self.shroom_step(from);
        for _ in 0..4 {
            if self.shroom_spot(next) {
                from = next;
            }
            next = self.shroom_step(from);
        }
        if self.shroom_spot(next) {
            self.set_block_flags(next, id << 4, flags::SEND);
        }
    }

    fn shroom_step(&mut self, p: (i32, i32, i32)) -> (i32, i32, i32) {
        let r = &mut self.world_rand;
        let dx = r.next_int_bounded(3) - 1;
        let dy = r.next_int_bounded(2) - r.next_int_bounded(2);
        let dz = r.next_int_bounded(3) - 1;
        (p.0 + dx, p.1 + dy, p.2 + dz)
    }

    /// `BlockMushroom.canUseBonemeal` + `grow`: 40% of the time it tries to become a
    /// huge mushroom.
    pub(super) fn bone_meal_mushroom(&mut self, pos: (i32, i32, i32), state: u16) {
        if f64::from(self.world_rand.next_float()) < BONE_MEAL_CHANCE {
            self.grow_huge_mushroom(pos, state);
        }
    }

    /// `BlockMushroom.generateBigMushroom`: the mushroom makes room and the huge one
    /// grows, or it comes back.
    fn grow_huge_mushroom(&mut self, pos: (i32, i32, i32), state: u16) {
        self.set_block_notify(pos, 0);
        let kind = if state >> 4 == block::BROWN_MUSHROOM {
            BROWN_MUSHROOM_BLOCK
        } else {
            RED_MUSHROOM_BLOCK
        };
        if !self.generate_huge_mushroom(pos, kind) {
            self.set_block_notify(pos, state);
        }
    }

    /// `WorldGenBigMushroom.generate` (with block notifications, as from bone meal).
    fn generate_huge_mushroom(&mut self, pos: (i32, i32, i32), kind: u16) -> bool {
        let height = self.world_rand.next_int_bounded(3) + 4;
        if pos.1 < 1 || pos.1 + height + 1 >= 256 {
            return false;
        }
        for y in pos.1..=pos.1 + 1 + height {
            let r = if y <= pos.1 + 3 { 0 } else { 3 };
            for x in pos.0 - r..=pos.0 + r {
                for z in pos.2 - r..=pos.2 + r {
                    if !(0..256).contains(&y)
                        || !matches!(
                            material(self.shroom_block((x, y, z)) >> 4),
                            Material::Air | Material::Leaves
                        )
                    {
                        return false;
                    }
                }
            }
        }
        if !matches!(
            self.shroom_block((pos.0, pos.1 - 1, pos.2)) >> 4,
            block::DIRT | block::GRASS | block::MYCELIUM
        ) {
            return false;
        }
        let brown = kind == BROWN_MUSHROOM_BLOCK;
        let top = pos.1 + height;
        let bottom = if brown { top } else { top - 3 };
        for y in bottom..=top {
            let r = if brown {
                3
            } else if y < top {
                2
            } else {
                1
            };
            let (x0, x1, z0, z1) = (pos.0 - r, pos.0 + r, pos.2 - r, pos.2 + r);
            for x in x0..=x1 {
                for z in z0..=z1 {
                    let mut variant = CENTER;
                    if x == x0 {
                        variant -= 1;
                    } else if x == x1 {
                        variant += 1;
                    }
                    if z == z0 {
                        variant -= 3;
                    } else if z == z1 {
                        variant += 3;
                    }
                    if brown || y < top {
                        if (x == x0 || x == x1) && (z == z0 || z == z1) {
                            continue;
                        }
                        // The cut corners take the corner faces.
                        let (inner_w, inner_e) = (pos.0 - (r - 1), pos.0 + (r - 1));
                        let (inner_n, inner_s) = (pos.2 - (r - 1), pos.2 + (r - 1));
                        if (x == inner_w && z == z0) || (x == x0 && z == inner_n) {
                            variant = NORTH_WEST;
                        }
                        if (x == inner_e && z == z0) || (x == x1 && z == inner_n) {
                            variant = NORTH_EAST;
                        }
                        if (x == inner_w && z == z1) || (x == x0 && z == inner_s) {
                            variant = SOUTH_WEST;
                        }
                        if (x == inner_e && z == z1) || (x == x1 && z == inner_s) {
                            variant = SOUTH_EAST;
                        }
                    }
                    if variant == CENTER && y < top {
                        variant = ALL_INSIDE;
                    }
                    // Vanilla quirk: compares the mushroom's own height instead of the
                    // layer's, so the inside blocks are never placed.
                    if pos.1 >= top - 1 || variant != ALL_INSIDE {
                        let p = (x, y, z);
                        if !super::trees::full_block(self.shroom_block(p) >> 4) {
                            self.set_block_notify(p, kind << 4 | variant);
                        }
                    }
                }
            }
        }
        for k in 0..height {
            let p = (pos.0, pos.1 + k, pos.2);
            if !super::trees::full_block(self.shroom_block(p) >> 4) {
                self.set_block_notify(p, kind << 4 | STEM);
            }
        }
        true
    }
}
