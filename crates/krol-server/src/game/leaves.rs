//! Leaf decay (vanilla `BlockLeaves`, `BlockOldLeaf`, `BlockNewLeaf`, `BlockLog`
//! breaking): leaves more than 4 blocks (through leaves) from a log disappear on a
//! random tick once a nearby log or leaf block was removed.

use krol_protocol::packets::play::ItemStack;

use super::Game;
use super::block_data::{Material, material};
use super::ids::block;

/// Metadata bit: placed by a player, never decays (`DECAYABLE` false).
const NO_DECAY: u16 = 4;
/// Metadata bit: check whether to decay on the next random tick (`CHECK_DECAY`).
const CHECK_DECAY: u16 = 8;
/// How far leaves can be from a log (through leaves) and survive.
const REACH: i32 = 4;
/// The side of the distance cube (`BlockLeaves.surroundings`).
const SIDE: usize = 32;
const HALF: i32 = (SIDE / 2) as i32;
/// A log in the distance cube.
const LOG: i32 = 0;
/// A leaf block not reached yet.
const UNREACHED_LEAF: i32 = -2;
/// Anything else.
const OTHER: i32 = -1;
/// How far breaking a log or leaf block marks leaves for checking.
const LOG_MARK_RADIUS: i32 = 4;
const LEAF_MARK_RADIUS: i32 = 1;
/// 1 in this many leaves drop a sapling (jungle: twice as rare).
const SAPLING_CHANCE: i32 = 20;
const JUNGLE_SAPLING_CHANCE: i32 = 40;
/// 1 in this many oak or dark oak leaves drop an apple.
const APPLE_CHANCE: i32 = 200;
/// Leaf variants (metadata & 3) of `leaves` and `leaves2`.
const OAK: u16 = 0;
const JUNGLE: u16 = 3;
const DARK_OAK: u16 = 1;
/// `apple`.
const APPLE: i16 = 260;

/// Leaves of either kind.
pub fn is_leaves(id: u16) -> bool {
    material(id) == Material::Leaves
}

fn is_log(id: u16) -> bool {
    id == block::LOG || id == block::LOG2
}

fn index(dx: i32, dy: i32, dz: i32) -> usize {
    ((dx + HALF) as usize * SIDE + (dy + HALF) as usize) * SIDE + (dz + HALF) as usize
}

impl Game {
    /// `BlockLeaves.breakBlock` / `BlockLog.breakBlock`: nearby leaves are marked to
    /// check for decay (without updates).
    pub(super) fn mark_leaves_around(&mut self, pos: (i32, i32, i32), radius: i32) {
        for dx in -radius..=radius {
            for dy in -radius..=radius {
                for dz in -radius..=radius {
                    let p = (pos.0 + dx, pos.1 + dy, pos.2 + dz);
                    let state = self.level.overworld.block(p.0, p.1, p.2);
                    if is_leaves(state >> 4) && state & CHECK_DECAY == 0 {
                        self.set_block_flags(p, state | CHECK_DECAY, 0);
                    }
                }
            }
        }
    }

    /// The breakBlock hooks of logs and leaves.
    pub(super) fn leaves_block_removed(&mut self, pos: (i32, i32, i32), old: u16) {
        let id = old >> 4;
        if is_log(id) {
            self.mark_leaves_around(pos, LOG_MARK_RADIUS);
        } else if is_leaves(id) {
            self.mark_leaves_around(pos, LEAF_MARK_RADIUS);
        }
    }

    /// `BlockLeaves.updateTick` (its random tick): marked, decayable leaves without a
    /// log within reach drop and disappear.
    pub(super) fn leaves_tick(&mut self, pos: (i32, i32, i32), state: u16) {
        if state & CHECK_DECAY == 0 || state & NO_DECAY != 0 {
            return;
        }
        let mut cube = vec![OTHER; SIDE * SIDE * SIDE];
        for dx in -REACH..=REACH {
            for dy in -REACH..=REACH {
                for dz in -REACH..=REACH {
                    let id = self
                        .level
                        .overworld
                        .block(pos.0 + dx, pos.1 + dy, pos.2 + dz)
                        >> 4;
                    cube[index(dx, dy, dz)] = if is_log(id) {
                        LOG
                    } else if is_leaves(id) {
                        UNREACHED_LEAF
                    } else {
                        OTHER
                    };
                }
            }
        }
        for distance in 1..=REACH {
            for dx in -REACH..=REACH {
                for dy in -REACH..=REACH {
                    for dz in -REACH..=REACH {
                        if cube[index(dx, dy, dz)] != distance - 1 {
                            continue;
                        }
                        for (ax, ay, az) in [
                            (-1, 0, 0),
                            (1, 0, 0),
                            (0, -1, 0),
                            (0, 1, 0),
                            (0, 0, -1),
                            (0, 0, 1),
                        ] {
                            let k = index(dx + ax, dy + ay, dz + az);
                            if cube[k] == UNREACHED_LEAF {
                                cube[k] = distance;
                            }
                        }
                    }
                }
            }
        }
        if cube[index(0, 0, 0)] >= 0 {
            self.set_block_flags(pos, state & !CHECK_DECAY, 0);
        } else {
            self.drop_block_items(pos, state);
            self.set_block_notify(pos, 0);
        }
    }

    /// `BlockLeaves.dropBlockAsItemWithChance` (no fortune): sometimes a sapling of the
    /// leaf's kind, and from oak and dark oak sometimes an apple.
    pub(super) fn drop_leaves(&mut self, pos: (i32, i32, i32), state: u16) {
        let (id, variant) = (state >> 4, state & 3);
        let old = id == block::LEAVES;
        let chance = if old && variant == JUNGLE {
            JUNGLE_SAPLING_CHANCE
        } else {
            SAPLING_CHANCE
        };
        if self.world_rand.next_int_bounded(chance) == 0 {
            let damage = if old { variant } else { variant + 4 };
            self.spawn_as_entity(pos, stack(super::ids::item::SAPLING, damage as i16));
        }
        let apples = if old {
            variant == OAK
        } else {
            variant == DARK_OAK
        };
        // dropApple draws only for the kinds that drop apples.
        if apples && self.world_rand.next_int_bounded(APPLE_CHANCE) == 0 {
            self.spawn_as_entity(pos, stack(APPLE, 0));
        }
    }
}

fn stack(id: i16, damage: i16) -> ItemStack {
    ItemStack {
        id,
        count: 1,
        damage,
        nbt: None,
    }
}
