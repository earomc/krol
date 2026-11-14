//! Pressure plates and entity-block collisions (vanilla: `Entity.doBlockCollisions`,
//! `BlockBasePressurePlate`, `BlockPressurePlate`, `BlockPressurePlateWeighted`).
//!
//! TODO(M5): the other `onEntityCollidedWithBlock` blocks (cactus, webs, tripwires,
//! portals, wooden buttons with arrows, fire, soul sand is a collision box).

use krol_java::math::floor_f64;

use super::Game;
use super::ids::block;
use super::physics::Aabb;
use super::placement::DOWN;

/// Stone and wooden plates recheck every 20 ticks, weighted plates every 10.
const PLATE_TICKS: i32 = 20;
const WEIGHTED_PLATE_TICKS: i32 = 10;
/// Entities needed for full power on weighted plates (light / heavy).
const LIGHT_PLATE_MAX: usize = 15;
const HEAVY_PLATE_MAX: usize = 150;
/// The sensitive area: inset 1/8 at the sides, 1/4 high.
const PLATE_INSET: f32 = 0.125;
const PLATE_HEIGHT: f64 = 0.25;
/// Click sound when a plate turns on and off.
const CLICK_VOLUME: f32 = 0.3;
const CLICK_ON: f32 = 0.6;
const CLICK_OFF: f32 = 0.5;
/// The stone/wooden plate's powered bit.
const PLATE_POWERED: u16 = 1;
/// `doBlockCollisions` shrinks the entity box by this much.
const COLLISION_EPSILON: f64 = 0.001;

/// Pressure plates.
pub fn is_plate(id: u16) -> bool {
    matches!(
        id,
        block::STONE_PRESSURE_PLATE
            | block::WOODEN_PRESSURE_PLATE
            | block::LIGHT_PRESSURE_PLATE
            | block::HEAVY_PRESSURE_PLATE
    )
}

fn is_weighted(id: u16) -> bool {
    id == block::LIGHT_PRESSURE_PLATE || id == block::HEAVY_PRESSURE_PLATE
}

/// `getRedstoneStrength`.
pub fn plate_strength(state: u16) -> i32 {
    let (id, meta) = (state >> 4, state & 15);
    if is_weighted(id) {
        i32::from(meta)
    } else if meta & PLATE_POWERED != 0 {
        super::redstone::MAX_POWER
    } else {
        0
    }
}

/// `setRedstoneStrength`.
fn with_strength(state: u16, strength: i32) -> u16 {
    let id = state >> 4;
    let meta = if is_weighted(id) {
        strength as u16 & 15
    } else {
        u16::from(strength > 0)
    };
    id << 4 | meta
}

impl Game {
    /// Entities whose box intersects `area` (non-spectators; only living ones if
    /// `living`), as `getEntitiesWithinAABB` counts them.
    fn entities_in(&self, area: &Aabb, living: bool) -> usize {
        let players = self
            .players
            .iter()
            .filter(|p| p.gamemode != 3 && Aabb::at(p.pos, 0.6, 1.8).intersects(area))
            .count();
        if living {
            return players;
        }
        players
            + self
                .items
                .values()
                .filter(|it| !it.dead && it.body.bb().intersects(area))
                .count()
            + self
                .orbs
                .values()
                .filter(|o| !o.dead && o.body.bb().intersects(area))
                .count()
            + self
                .falling
                .values()
                .filter(|f| !f.dead && f.body.bb().intersects(area))
                .count()
    }

    /// `computeRedstoneStrength`.
    fn compute_plate_strength(&self, pos: (i32, i32, i32), id: u16) -> i32 {
        let area = Aabb::new(
            [
                f64::from(pos.0 as f32 + PLATE_INSET),
                f64::from(pos.1),
                f64::from(pos.2 as f32 + PLATE_INSET),
            ],
            [
                f64::from((pos.0 + 1) as f32 - PLATE_INSET),
                f64::from(pos.1) + PLATE_HEIGHT,
                f64::from((pos.2 + 1) as f32 - PLATE_INSET),
            ],
        );
        match id {
            block::STONE_PRESSURE_PLATE | block::WOODEN_PRESSURE_PLATE => {
                if self.entities_in(&area, id == block::STONE_PRESSURE_PLATE) > 0 {
                    super::redstone::MAX_POWER
                } else {
                    0
                }
            }
            _ => {
                let max = if id == block::LIGHT_PRESSURE_PLATE {
                    LIGHT_PLATE_MAX
                } else {
                    HEAVY_PLATE_MAX
                };
                let count = self.entities_in(&area, false).min(max);
                if count == 0 {
                    0
                } else {
                    krol_java::math::ceil_f32(count as f32 / max as f32 * 15.0)
                }
            }
        }
    }

    /// `BlockBasePressurePlate.updateNeighbors`: around the plate and the block below.
    fn plate_notify(&mut self, pos: (i32, i32, i32), id: u16) {
        self.notify_neighbors(pos, id);
        self.notify_neighbors(super::placement::offset(pos, DOWN), id);
    }

    /// `BlockBasePressurePlate.updateState`.
    fn update_plate(&mut self, pos: (i32, i32, i32), state: u16) {
        let id = state >> 4;
        let old = plate_strength(state);
        let new = self.compute_plate_strength(pos, id);
        if old != new {
            self.set_block_flags(
                pos,
                with_strength(state, new),
                super::block_update::flags::SEND,
            );
            self.plate_notify(pos, id);
        }
        let center = (
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 0.1,
            f64::from(pos.2) + 0.5,
        );
        if new == 0 && old > 0 {
            self.play_sound_near_except(None, "random.click", center, CLICK_VOLUME, CLICK_OFF);
        } else if new > 0 && old == 0 {
            self.play_sound_near_except(None, "random.click", center, CLICK_VOLUME, CLICK_ON);
        }
        if new > 0 {
            let delay = if is_weighted(id) {
                WEIGHTED_PLATE_TICKS
            } else {
                PLATE_TICKS
            };
            self.schedule_update(pos, id, delay);
        }
    }

    /// `BlockBasePressurePlate.updateTick`: a pressed plate rechecks.
    pub(super) fn plate_tick(&mut self, pos: (i32, i32, i32), state: u16) {
        if plate_strength(state) > 0 {
            self.update_plate(pos, state);
        }
    }

    /// `BlockBasePressurePlate.breakBlock`: a pressed plate updates around itself.
    pub(super) fn plate_removed(&mut self, pos: (i32, i32, i32), old: u16) {
        if plate_strength(old) > 0 {
            self.plate_notify(pos, old >> 4);
        }
    }

    /// `BlockBasePressurePlate.canPlaceBlockAt` (checked on neighbor changes): a solid
    /// top or a fence below.
    pub(super) fn plate_can_stay(&mut self, pos: (i32, i32, i32)) -> bool {
        let below = self.level.overworld.block(pos.0, pos.1 - 1, pos.2);
        super::placement::solid_top(below) || super::blocks::is_fence_like(below >> 4)
    }

    /// `Entity.doBlockCollisions`: every block the entity's box (shrunk by 0.001)
    /// touches reacts (`onEntityCollidedWithBlock`).
    pub(super) fn block_collisions(&mut self, bb: &Aabb) {
        self.block_collisions_of(bb, None);
    }

    /// [`Self::block_collisions`] for a player (at that index), which cactus also
    /// pricks.
    ///
    /// TODO(M5): items touching cactus are hurt too (and break).
    pub(super) fn block_collisions_of(&mut self, bb: &Aabb, player: Option<usize>) {
        let min = (
            floor_f64(bb.min[0] + COLLISION_EPSILON),
            floor_f64(bb.min[1] + COLLISION_EPSILON),
            floor_f64(bb.min[2] + COLLISION_EPSILON),
        );
        let max = (
            floor_f64(bb.max[0] - COLLISION_EPSILON),
            floor_f64(bb.max[1] - COLLISION_EPSILON),
            floor_f64(bb.max[2] - COLLISION_EPSILON),
        );
        if max.1 < 0 || min.1 >= 256 {
            return;
        }
        for cx in (min.0 >> 4)..=(max.0 >> 4) {
            for cz in (min.2 >> 4)..=(max.2 >> 4) {
                if !self.level.overworld.is_chunk_loaded(cx, cz) {
                    return;
                }
            }
        }
        for x in min.0..=max.0 {
            for y in min.1..=max.1 {
                for z in min.2..=max.2 {
                    let state = self.level.overworld.block(x, y, z);
                    // BlockBasePressurePlate.onEntityCollidedWithBlock.
                    if is_plate(state >> 4) && plate_strength(state) == 0 {
                        self.update_plate((x, y, z), state);
                    }
                    // BlockCactus.onEntityCollidedWithBlock.
                    if state >> 4 == super::ids::block::CACTUS
                        && let Some(i) = player
                    {
                        self.attack_player(i, super::damage::DamageSource::Cactus, 1.0);
                    }
                }
            }
        }
    }
}
