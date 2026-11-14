//! Blocks that open with redstone and need support: doors, trapdoors and fence gates
//! (`onNeighborBlockChange` of `BlockDoor`, `BlockTrapDoor`, `BlockFenceGate`).

use krol_protocol::packets::play::ClientboundPlay;

use super::Game;
use super::ids::block;
use super::placement::{EAST, NORTH, SOUTH, WEST, is_normal_cube, offset, opposite, solid_top};

/// Door metadata: lower half open bit; upper half marker, powered bit.
const DOOR_OPEN: u16 = 4;
const DOOR_UPPER: u16 = 8;
const DOOR_POWERED: u16 = 2;
/// Trapdoor and fence gate open bit; fence gate powered bit.
const OPEN: u16 = 4;
const GATE_POWERED: u16 = 8;
/// Door, trapdoor and gate sound effects (`playAuxSFX`).
const EFFECT_OPEN: i32 = 1003;
const EFFECT_CLOSE: i32 = 1006;
/// Players within this distance hear block effects.
const EFFECT_RANGE: f64 = 64.0;

/// Door blocks.
pub fn is_door(id: u16) -> bool {
    id == block::WOODEN_DOOR
        || id == block::IRON_DOOR
        || (block::SPRUCE_DOOR..=block::DARK_OAK_DOOR).contains(&id)
}

fn is_trapdoor(id: u16) -> bool {
    id == block::TRAPDOOR || id == block::IRON_TRAPDOOR
}

fn is_fence_gate(id: u16) -> bool {
    id == block::OAK_FENCE_GATE
        || (block::SPRUCE_FENCE_GATE..=block::ACACIA_FENCE_GATE).contains(&id)
}

/// The facing of a trapdoor from its metadata (`BlockTrapDoor.getFacing`).
fn trapdoor_facing(meta: u16) -> u8 {
    [NORTH, SOUTH, WEST, EAST][usize::from(meta & 3)]
}

/// `BlockTrapDoor.isValidSupportBlock`: opaque full cubes, glowstone, slabs and stairs.
fn trapdoor_support(state: u16) -> bool {
    let id = state >> 4;
    is_normal_cube(state)
        || id == block::REDSTONE_BLOCK
        || id == block::GLOWSTONE
        || matches!(id, 43 | 44 | 125 | 126 | 181 | 182)
        || matches!(
            id,
            53 | 67 | 108 | 109 | 114 | 128 | 134..=136 | 156 | 163 | 164 | 180
        )
}

impl Game {
    /// `playAuxSFXAtEntity(null, id, pos, 0)`: the effect to everyone within 64 blocks.
    pub(super) fn block_effect(&self, id: i32, pos: (i32, i32, i32)) {
        let packet = ClientboundPlay::Effect {
            id,
            pos,
            data: 0,
            disable_relative_volume: false,
        };
        for p in &self.players {
            let d = (
                f64::from(pos.0) - p.pos.0,
                f64::from(pos.1) - p.pos.1,
                f64::from(pos.2) - p.pos.2,
            );
            if d.0 * d.0 + d.1 * d.1 + d.2 * d.2 < EFFECT_RANGE * EFFECT_RANGE {
                p.send(packet.clone());
            }
        }
    }

    /// Handles a neighbor change for doors, trapdoors and fence gates; returns whether
    /// the block was one of them.
    pub(super) fn powered_block_changed(
        &mut self,
        pos: (i32, i32, i32),
        state: u16,
        source: u16,
    ) -> bool {
        let id = state >> 4;
        if is_door(id) {
            self.door_changed(pos, state, source);
        } else if is_trapdoor(id) {
            self.trapdoor_changed(pos, state, source);
        } else if is_fence_gate(id) {
            self.fence_gate_changed(pos, state, source);
        } else {
            return false;
        }
        true
    }

    /// `BlockDoor.onNeighborBlockChange`.
    fn door_changed(&mut self, pos: (i32, i32, i32), state: u16, source: u16) {
        let (id, meta) = (state >> 4, state & 15);
        let flags = super::block_update::flags::SEND;
        if meta & DOOR_UPPER != 0 {
            let lower = (pos.0, pos.1 - 1, pos.2);
            let lower_state = self.level.overworld.block(lower.0, lower.1, lower.2);
            if lower_state >> 4 != id {
                self.set_block_notify(pos, 0);
            } else if source != id {
                self.door_changed(lower, lower_state, source);
            }
            return;
        }
        let upper = (pos.0, pos.1 + 1, pos.2);
        let upper_state = self.level.overworld.block(upper.0, upper.1, upper.2);
        let mut broken = false;
        if upper_state >> 4 != id {
            self.set_block_notify(pos, 0);
            broken = true;
        }
        if !solid_top(self.level.overworld.block(pos.0, pos.1 - 1, pos.2)) {
            self.set_block_notify(pos, 0);
            broken = true;
            if upper_state >> 4 == id {
                self.set_block_notify(upper, 0);
            }
        }
        if broken {
            self.drop_block_items(pos, state);
            return;
        }
        let powered = self.is_block_powered(pos) || self.is_block_powered(upper);
        let upper_powered = upper_state & DOOR_POWERED != 0;
        if (powered || self.block_provides_power(source))
            && source != id
            && powered != upper_powered
        {
            let new_upper = if powered {
                upper_state | DOOR_POWERED
            } else {
                upper_state & !DOOR_POWERED
            };
            self.set_block_flags(upper, new_upper, flags);
            if powered != (meta & DOOR_OPEN != 0) {
                let new_lower = if powered {
                    state | DOOR_OPEN
                } else {
                    state & !DOOR_OPEN
                };
                self.set_block_flags(pos, new_lower, flags);
                self.block_effect(if powered { EFFECT_OPEN } else { EFFECT_CLOSE }, pos);
            }
        }
    }

    /// `BlockTrapDoor.onNeighborBlockChange`.
    fn trapdoor_changed(&mut self, pos: (i32, i32, i32), state: u16, source: u16) {
        let meta = state & 15;
        let support = offset(pos, opposite(trapdoor_facing(meta)));
        let s = self.level.overworld.block(support.0, support.1, support.2);
        if !trapdoor_support(s) {
            self.set_block_notify(pos, 0);
            self.drop_block_items(pos, state);
            return;
        }
        let powered = self.is_block_powered(pos);
        if (powered || self.block_provides_power(source)) && (meta & OPEN != 0) != powered {
            let new = if powered { state | OPEN } else { state & !OPEN };
            self.set_block_flags(pos, new, super::block_update::flags::SEND);
            self.block_effect(if powered { EFFECT_OPEN } else { EFFECT_CLOSE }, pos);
        }
    }

    /// `BlockFenceGate.onNeighborBlockChange`.
    fn fence_gate_changed(&mut self, pos: (i32, i32, i32), state: u16, source: u16) {
        let powered = self.is_block_powered(pos);
        if !powered && !self.block_provides_power(source) {
            return;
        }
        let (open, was_powered) = (state & OPEN != 0, state & GATE_POWERED != 0);
        let flags = super::block_update::flags::SEND;
        if powered && !open && !was_powered {
            self.set_block_flags(pos, state | OPEN | GATE_POWERED, flags);
            self.block_effect(EFFECT_OPEN, pos);
        } else if !powered && open && was_powered {
            self.set_block_flags(pos, state & !(OPEN | GATE_POWERED), flags);
            self.block_effect(EFFECT_CLOSE, pos);
        } else if powered != was_powered {
            let new = if powered {
                state | GATE_POWERED
            } else {
                state & !GATE_POWERED
            };
            self.set_block_flags(pos, new, flags);
        }
    }
}
