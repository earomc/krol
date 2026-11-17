//! Redstone power (vanilla: `World.getStrongPower` / `getRedstonePower` /
//! `isBlockPowered` / `isBlockIndirectlyGettingPowered`, `getWeakPower` /
//! `getStrongPower` of the power sources, `BlockRedstoneWire`, `BlockRedstoneTorch`,
//! `BlockRedstoneLight`).
//!
//! TODO(M5): comparators reading hoppers, dispensers and the other containers and item
//! frames, pistons, dispensers and droppers, note blocks,
//! TNT, rails, pressure plates, tripwires, daylight sensors, doors and trapdoors
//! reacting to power; torch burnout smoke particles.

use super::Game;
use super::ids::block;
use super::placement::{
    DOWN, EAST, NORTH, SOUTH, UP, WEST, is_normal_cube, offset, opposite, solid_top,
};

/// Full redstone power.
pub const MAX_POWER: i32 = 15;
/// Redstone torches react after this many ticks (`tickRate`).
const TORCH_DELAY: i32 = 2;
/// A torch that toggled this often within [`BURNOUT_WINDOW`] ticks burns out
/// (`isBurnedOut`) and tries again after [`BURNOUT_RETRY`] ticks.
const BURNOUT_TOGGLES: usize = 8;
const BURNOUT_WINDOW: i64 = 60;
const BURNOUT_RETRY: i32 = 160;
/// A lamp goes out this many ticks after losing power.
const LAMP_OFF_DELAY: i32 = 4;

/// `EnumFacing.values()` order.
const ALL_FACES: [u8; 6] = [DOWN, UP, NORTH, SOUTH, WEST, EAST];
/// `EnumFacing.Plane.HORIZONTAL` order.
const HORIZONTAL: [u8; 4] = [NORTH, EAST, SOUTH, WEST];
/// `EnumFacing.Plane.VERTICAL` order.
const VERTICAL: [u8; 2] = [UP, DOWN];
/// The order `World.getStrongPower(pos)` and `isBlockPowered` ask the neighbors in.
const POWER_QUERY_ORDER: [u8; 6] = [DOWN, UP, NORTH, SOUTH, WEST, EAST];

/// `EnumFacing.rotateY` (clockwise seen from above).
fn rotate_y(face: u8) -> u8 {
    match face {
        NORTH => EAST,
        EAST => SOUTH,
        SOUTH => WEST,
        _ => NORTH,
    }
}

/// The facing of a torch from its metadata (`BlockTorch.FACING`).
pub fn torch_facing(meta: u16) -> u8 {
    match meta {
        1 => EAST,
        2 => WEST,
        3 => SOUTH,
        4 => NORTH,
        _ => UP,
    }
}

/// The face a lever or button is mounted on, from its metadata.
pub fn mount_facing(id: u16, meta: u16) -> u8 {
    match (id == block::LEVER, meta & 7) {
        (true, 0 | 7) | (false, 0) => DOWN,
        (_, 1) => EAST,
        (_, 2) => WEST,
        (_, 3) => SOUTH,
        (_, 4) => NORTH,
        _ => UP,
    }
}

/// Blocks treated as one for scheduled ticks (`Block.isAssociatedBlock`): lit and unlit
/// variants.
pub fn associated(id: u16) -> u16 {
    match id {
        block::UNLIT_REDSTONE_TORCH => block::REDSTONE_TORCH,
        block::POWERED_REPEATER => block::UNPOWERED_REPEATER,
        block::POWERED_COMPARATOR => block::UNPOWERED_COMPARATOR,
        other => other,
    }
}

/// Lever and button powered bit.
const POWERED: u16 = 8;

impl Game {
    /// `Block.canProvidePower` (the wire only while it is not computing its own input).
    fn provides_power(&self, id: u16) -> bool {
        match id {
            block::REDSTONE_WIRE => self.wire_gives_power,
            block::LEVER
            | block::STONE_BUTTON
            | block::WOODEN_BUTTON
            | block::REDSTONE_TORCH
            | block::UNLIT_REDSTONE_TORCH
            | block::REDSTONE_BLOCK
            | block::UNPOWERED_REPEATER
            | block::POWERED_REPEATER
            | block::UNPOWERED_COMPARATOR
            | block::POWERED_COMPARATOR
            | block::STONE_PRESSURE_PLATE
            | block::WOODEN_PRESSURE_PLATE
            | block::LIGHT_PRESSURE_PLATE
            | block::HEAVY_PRESSURE_PLATE
            | block::TRAPPED_CHEST
            | block::DAYLIGHT_DETECTOR
            | block::DAYLIGHT_DETECTOR_INVERTED
            | block::DETECTOR_RAIL
            | block::TRIPWIRE_HOOK => true,
            _ => false,
        }
    }

    /// `Block.canProvidePower` for other modules.
    pub(super) fn block_provides_power(&self, id: u16) -> bool {
        self.provides_power(id)
    }

    /// `Block.getWeakPower` of the block at `pos`, asked from side `side`.
    fn weak_power(&mut self, pos: (i32, i32, i32), side: u8) -> i32 {
        let state = self.level.overworld.block(pos.0, pos.1, pos.2);
        let (id, meta) = (state >> 4, state & 15);
        match id {
            block::LEVER | block::STONE_BUTTON | block::WOODEN_BUTTON => {
                if meta & POWERED != 0 {
                    MAX_POWER
                } else {
                    0
                }
            }
            block::REDSTONE_TORCH => {
                if torch_facing(meta) != side {
                    MAX_POWER
                } else {
                    0
                }
            }
            block::REDSTONE_BLOCK => MAX_POWER,
            block::REDSTONE_WIRE => self.wire_weak_power(pos, i32::from(meta), side),
            block::UNPOWERED_REPEATER | block::POWERED_REPEATER => {
                Self::repeater_power(id, meta, side)
            }
            block::UNPOWERED_COMPARATOR | block::POWERED_COMPARATOR => {
                self.comparator_power(pos, id, meta, side)
            }
            _ if super::pressure_plates::is_plate(id) => {
                super::pressure_plates::plate_strength(state)
            }
            block::TRAPPED_CHEST => self.trapped_chest_power(pos),
            _ => 0,
        }
    }

    /// `Block.getStrongPower` of the block at `pos`, asked from side `side`.
    fn strong_power(&mut self, pos: (i32, i32, i32), side: u8) -> i32 {
        let state = self.level.overworld.block(pos.0, pos.1, pos.2);
        let (id, meta) = (state >> 4, state & 15);
        match id {
            block::LEVER | block::STONE_BUTTON | block::WOODEN_BUTTON => {
                if meta & POWERED != 0 && mount_facing(id, meta) == side {
                    MAX_POWER
                } else {
                    0
                }
            }
            block::REDSTONE_TORCH if side == DOWN => self.weak_power(pos, side),
            block::REDSTONE_WIRE if self.wire_gives_power => self.weak_power(pos, side),
            block::UNPOWERED_REPEATER | block::POWERED_REPEATER => {
                Self::repeater_power(id, meta, side)
            }
            block::UNPOWERED_COMPARATOR | block::POWERED_COMPARATOR => {
                self.comparator_power(pos, id, meta, side)
            }
            _ if super::pressure_plates::is_plate(id) && side == UP => {
                super::pressure_plates::plate_strength(state)
            }
            block::TRAPPED_CHEST if side == UP => self.trapped_chest_power(pos),
            _ => 0,
        }
    }

    /// `World.getStrongPower(pos)`: the strongest strong power any neighbor sends in.
    fn strong_power_into(&mut self, pos: (i32, i32, i32)) -> i32 {
        let mut max = 0;
        for face in POWER_QUERY_ORDER {
            max = max.max(self.strong_power(offset(pos, face), face));
            if max >= MAX_POWER {
                break;
            }
        }
        max
    }

    /// `World.getRedstonePower(pos, side)`: a normal cube relays the strong power it
    /// gets; anything else gives its own weak power.
    fn redstone_power(&mut self, pos: (i32, i32, i32), side: u8) -> i32 {
        let state = self.level.overworld.block(pos.0, pos.1, pos.2);
        if is_normal_cube(state) {
            self.strong_power_into(pos)
        } else {
            self.weak_power(pos, side)
        }
    }

    /// `World.isSidePowered`.
    pub(super) fn side_powered(&mut self, pos: (i32, i32, i32), side: u8) -> bool {
        self.redstone_power(pos, side) > 0
    }

    /// `World.isBlockPowered`.
    pub(super) fn is_block_powered(&mut self, pos: (i32, i32, i32)) -> bool {
        POWER_QUERY_ORDER
            .iter()
            .any(|&face| self.redstone_power(offset(pos, face), face) > 0)
    }

    /// `World.isBlockIndirectlyGettingPowered`.
    fn indirect_power(&mut self, pos: (i32, i32, i32)) -> i32 {
        let mut max = 0;
        for face in ALL_FACES {
            let p = self.redstone_power(offset(pos, face), face);
            if p >= MAX_POWER {
                return MAX_POWER;
            }
            max = max.max(p);
        }
        max
    }

    // --- Redstone wire -------------------------------------------------------------

    fn is_cube_at(&mut self, pos: (i32, i32, i32)) -> bool {
        let s = self.level.overworld.block(pos.0, pos.1, pos.2);
        is_normal_cube(s)
    }

    /// `BlockRedstoneWire.canConnectTo(state, side)`; `side` `None` is
    /// `canConnectUpwardsTo` (only wire).
    fn wire_connects_to(&self, state: u16, side: Option<u8>) -> bool {
        let (id, meta) = (state >> 4, state & 15);
        match id {
            block::REDSTONE_WIRE => true,
            block::UNPOWERED_REPEATER | block::POWERED_REPEATER => {
                // Repeater FACING from its metadata (south, west, north, east).
                let facing = [SOUTH, WEST, NORTH, EAST][usize::from(meta & 3)];
                side.is_some_and(|s| facing == s || opposite(facing) == s)
            }
            _ => side.is_some() && self.provides_power(id),
        }
    }

    /// `BlockRedstoneWire.func_176339_d`: whether the wire at `pos` points toward `side`.
    fn wire_points(&mut self, pos: (i32, i32, i32), side: u8) -> bool {
        let n = offset(pos, side);
        let w = &mut self.level.overworld;
        let n_state = w.block(n.0, n.1, n.2);
        let n_cube = is_normal_cube(n_state);
        let above_cube = is_normal_cube(w.block(pos.0, pos.1 + 1, pos.2));
        let n_up = w.block(n.0, n.1 + 1, n.2);
        let n_down = w.block(n.0, n.1 - 1, n.2);
        if !above_cube && n_cube && self.wire_connects_to(n_up, None) {
            return true;
        }
        if self.wire_connects_to(n_state, Some(side)) {
            return true;
        }
        let repeater_facing = [SOUTH, WEST, NORTH, EAST][usize::from(n_state & 3)];
        if n_state >> 4 == block::POWERED_REPEATER && repeater_facing == side {
            return true;
        }
        !n_cube && self.wire_connects_to(n_down, None)
    }

    /// `BlockRedstoneWire.getWeakPower`.
    fn wire_weak_power(&mut self, pos: (i32, i32, i32), power: i32, side: u8) -> i32 {
        if !self.wire_gives_power || power == 0 {
            return 0;
        }
        if side == UP {
            return power;
        }
        let mut points = Vec::new();
        for face in HORIZONTAL {
            if self.wire_points(pos, face) {
                points.push(face);
            }
        }
        let horizontal = side != UP && side != DOWN;
        if horizontal && points.is_empty() {
            return power;
        }
        let ccw = rotate_y(rotate_y(rotate_y(side)));
        if points.contains(&side) && !points.contains(&ccw) && !points.contains(&rotate_y(side)) {
            power
        } else {
            0
        }
    }

    /// `getMaxCurrentStrength`: `strength` or the power of a wire at `pos`, whichever
    /// is higher.
    fn max_wire_power(&mut self, pos: (i32, i32, i32), strength: i32) -> i32 {
        let s = self.level.overworld.block(pos.0, pos.1, pos.2);
        if s >> 4 != block::REDSTONE_WIRE {
            return strength;
        }
        strength.max(i32::from(s & 15))
    }

    /// `calculateCurrentChanges`: the wire's new power; a change is stored (without
    /// updates) and queues the wire and its neighbors for notification.
    fn calculate_wire(&mut self, pos: (i32, i32, i32), state: u16) {
        let old = i32::from(state & 15);
        let mut power = self.max_wire_power(pos, 0);
        self.wire_gives_power = false;
        let input = self.indirect_power(pos);
        self.wire_gives_power = true;
        if input > 0 && input > power - 1 {
            power = input;
        }
        let mut from_wires = 0;
        let above = (pos.0, pos.1 + 1, pos.2);
        let above_cube = self.is_cube_at(above);
        for face in HORIZONTAL {
            let n = offset(pos, face);
            from_wires = self.max_wire_power(n, from_wires);
            if self.is_cube_at(n) {
                if !above_cube {
                    from_wires = self.max_wire_power((n.0, n.1 + 1, n.2), from_wires);
                }
            } else {
                from_wires = self.max_wire_power((n.0, n.1 - 1, n.2), from_wires);
            }
        }
        if from_wires > power {
            power = from_wires - 1;
        } else if power > 0 {
            power -= 1;
        } else {
            power = 0;
        }
        if input > power - 1 {
            power = input;
        }
        if old != power {
            let new_state = (state & !15) | power as u16;
            if self.level.overworld.block(pos.0, pos.1, pos.2) == state {
                self.set_block_flags(pos, new_state, super::block_update::flags::SEND);
            }
            self.wire_queue.push(pos);
            for face in ALL_FACES {
                self.wire_queue.push(offset(pos, face));
            }
        }
    }

    /// `updateSurroundingRedstone`: recomputes the wire, then notifies around every
    /// queued position in the order vanilla's `HashSet<BlockPos>` iterates them.
    fn update_wire(&mut self, pos: (i32, i32, i32), state: u16) {
        self.calculate_wire(pos, state);
        let queued = std::mem::take(&mut self.wire_queue);
        let mut unique: Vec<(i32, i32, i32)> = Vec::new();
        for p in queued {
            if !unique.contains(&p) {
                unique.push(p);
            }
        }
        let hashes: Vec<i32> = unique
            .iter()
            .map(|p| super::world::block_pos_hash(p.0, p.1, p.2))
            .collect();
        for k in krol_java::java_hash_order(&hashes) {
            self.notify_neighbors(unique[k], block::REDSTONE_WIRE);
        }
    }

    /// `notifyWireNeighborsOfStateChange`: around a wire and around its neighbors.
    fn notify_wire_neighbors(&mut self, pos: (i32, i32, i32)) {
        if self.level.overworld.block(pos.0, pos.1, pos.2) >> 4 != block::REDSTONE_WIRE {
            return;
        }
        self.notify_neighbors(pos, block::REDSTONE_WIRE);
        for face in ALL_FACES {
            self.notify_neighbors(offset(pos, face), block::REDSTONE_WIRE);
        }
    }

    /// The wire neighbors notified after a wire appears or goes away: the horizontal
    /// neighbors, then the wire one step up (past a cube) or down.
    fn notify_wire_surroundings(&mut self, pos: (i32, i32, i32)) {
        for face in HORIZONTAL {
            self.notify_wire_neighbors(offset(pos, face));
        }
        for face in HORIZONTAL {
            let n = offset(pos, face);
            if self.is_cube_at(n) {
                self.notify_wire_neighbors((n.0, n.1 + 1, n.2));
            } else {
                self.notify_wire_neighbors((n.0, n.1 - 1, n.2));
            }
        }
    }

    /// `BlockRedstoneWire.onBlockAdded`.
    pub(super) fn wire_added(&mut self, pos: (i32, i32, i32), state: u16) {
        self.update_wire(pos, state);
        for face in VERTICAL {
            self.notify_neighbors(offset(pos, face), block::REDSTONE_WIRE);
        }
        self.notify_wire_surroundings(pos);
    }

    /// `BlockRedstoneWire.breakBlock`.
    pub(super) fn wire_removed(&mut self, pos: (i32, i32, i32), old: u16) {
        for face in ALL_FACES {
            self.notify_neighbors(offset(pos, face), block::REDSTONE_WIRE);
        }
        self.update_wire(pos, old);
        self.notify_wire_surroundings(pos);
    }

    /// `BlockRedstoneWire.canPlaceBlockAt`: on a solid top or glowstone.
    pub(super) fn wire_can_stay(&mut self, pos: (i32, i32, i32)) -> bool {
        let below = self.level.overworld.block(pos.0, pos.1 - 1, pos.2);
        solid_top(below) || below >> 4 == block::GLOWSTONE
    }

    /// `BlockRedstoneWire.onNeighborBlockChange` when it can stay.
    pub(super) fn wire_neighbor_changed(&mut self, pos: (i32, i32, i32), state: u16) {
        self.update_wire(pos, state);
    }

    /// `ItemRedstone.onItemUse`: dust on a replaceable spot with a solid top below.
    pub(super) fn place_wire(&mut self, i: usize, pos: (i32, i32, i32), face: u8) -> bool {
        let clicked = self.level.overworld.block(pos.0, pos.1, pos.2);
        let at = if super::blocks::is_replaceable(clicked) {
            pos
        } else {
            offset(pos, face)
        };
        if !(0..256).contains(&at.1)
            || !super::blocks::is_replaceable(self.level.overworld.block(at.0, at.1, at.2))
            || !self.wire_can_stay(at)
        {
            return false;
        }
        let player = &mut self.players[i];
        if player.gamemode != 1 {
            let held = 36 + player.selected;
            if let Some(s) = &mut player.inventory[held] {
                s.count -= 1;
            }
        }
        self.set_block_notify(at, block::REDSTONE_WIRE << 4);
        self.count_item_use(i);
        true
    }

    // --- Redstone torches ----------------------------------------------------------

    /// `shouldBeOff`: the block the torch is on is powered.
    fn torch_should_be_off(&mut self, pos: (i32, i32, i32), meta: u16) -> bool {
        let towards = opposite(torch_facing(meta));
        self.redstone_power(offset(pos, towards), towards) > 0
    }

    /// `isBurnedOut`: counts this torch's recent toggles (adding one if `add`).
    fn torch_burned_out(&mut self, pos: (i32, i32, i32), add: bool) -> bool {
        if add {
            self.torch_toggles
                .push((pos, self.level.overworld.total_time));
        }
        self.torch_toggles.iter().filter(|(p, _)| *p == pos).count() >= BURNOUT_TOGGLES
    }

    /// Notifies around every neighbor of a lit torch (`onBlockAdded` / `breakBlock`).
    pub(super) fn torch_notify(&mut self, pos: (i32, i32, i32), id: u16) {
        for face in ALL_FACES {
            self.notify_neighbors(offset(pos, face), id);
        }
    }

    /// `BlockRedstoneTorch.onNeighborBlockChange` after the support check.
    pub(super) fn torch_neighbor_changed(&mut self, pos: (i32, i32, i32), state: u16) {
        let (id, meta) = (state >> 4, state & 15);
        let lit = id == block::REDSTONE_TORCH;
        if lit == self.torch_should_be_off(pos, meta) {
            self.schedule_update(pos, id, TORCH_DELAY);
        }
    }

    /// `BlockRedstoneTorch.updateTick`.
    pub(super) fn torch_tick(&mut self, pos: (i32, i32, i32), state: u16) {
        let (id, meta) = (state >> 4, state & 15);
        let off = self.torch_should_be_off(pos, meta);
        let now = self.level.overworld.total_time;
        while self
            .torch_toggles
            .first()
            .is_some_and(|(_, t)| now - t > BURNOUT_WINDOW)
        {
            self.torch_toggles.remove(0);
        }
        if id == block::REDSTONE_TORCH {
            if off {
                self.set_block_notify(pos, block::UNLIT_REDSTONE_TORCH << 4 | meta);
                if self.torch_burned_out(pos, true) {
                    let r = &mut self.world_rand;
                    let pitch = 2.6 + (r.next_float() - r.next_float()) * 0.8;
                    let center = (
                        f64::from(pos.0 as f32 + 0.5),
                        f64::from(pos.1 as f32 + 0.5),
                        f64::from(pos.2 as f32 + 0.5),
                    );
                    self.play_sound_near_except(None, "random.fizz", center, 0.5, pitch);
                    // The five smoke particles vanilla spawns are not sent by a server.
                    let now_id = self.level.overworld.block(pos.0, pos.1, pos.2) >> 4;
                    self.schedule_update(pos, now_id, BURNOUT_RETRY);
                }
            }
        } else if !off && !self.torch_burned_out(pos, false) {
            self.set_block_notify(pos, block::REDSTONE_TORCH << 4 | meta);
        }
    }

    // --- Redstone lamps ------------------------------------------------------------

    /// `BlockRedstoneLight.onBlockAdded`.
    pub(super) fn lamp_added(&mut self, pos: (i32, i32, i32), id: u16) {
        let powered = self.is_block_powered(pos);
        if id == block::LIT_REDSTONE_LAMP && !powered {
            self.set_block_flags(
                pos,
                block::REDSTONE_LAMP << 4,
                super::block_update::flags::SEND,
            );
        } else if id == block::REDSTONE_LAMP && powered {
            self.set_block_flags(
                pos,
                block::LIT_REDSTONE_LAMP << 4,
                super::block_update::flags::SEND,
            );
        }
    }

    /// `BlockRedstoneLight.onNeighborBlockChange`.
    pub(super) fn lamp_neighbor_changed(&mut self, pos: (i32, i32, i32), id: u16) {
        let powered = self.is_block_powered(pos);
        if id == block::LIT_REDSTONE_LAMP && !powered {
            self.schedule_update(pos, id, LAMP_OFF_DELAY);
        } else if id == block::REDSTONE_LAMP && powered {
            self.set_block_flags(
                pos,
                block::LIT_REDSTONE_LAMP << 4,
                super::block_update::flags::SEND,
            );
        }
    }

    /// `BlockRedstoneLight.updateTick`.
    pub(super) fn lamp_tick(&mut self, pos: (i32, i32, i32), id: u16) {
        if id == block::LIT_REDSTONE_LAMP && !self.is_block_powered(pos) {
            self.set_block_flags(
                pos,
                block::REDSTONE_LAMP << 4,
                super::block_update::flags::SEND,
            );
        }
    }
}

// --- Repeaters (vanilla: BlockRedstoneDiode, BlockRedstoneRepeater) ----------------

/// The repeater's `FACING` (the input side) from its metadata.
pub fn repeater_facing(meta: u16) -> u8 {
    [SOUTH, WEST, NORTH, EAST][usize::from(meta & 3)]
}

/// The repeater's delay in ticks (`getDelay`: two per setting).
fn repeater_delay(meta: u16) -> i32 {
    (i32::from(meta >> 2) + 1) * 2
}

/// Scheduled tick priorities of repeaters (`updateState`): facing into another
/// repeater first, then turning off, then turning on.
const PRIORITY_INTO_REPEATER: i32 = -3;
const PRIORITY_TURN_OFF: i32 = -2;
const PRIORITY_TURN_ON: i32 = -1;

fn is_repeater(id: u16) -> bool {
    id == block::UNPOWERED_REPEATER || id == block::POWERED_REPEATER
}

impl Game {
    /// `BlockRedstoneDiode.getWeakPower` (and strong power): a powered repeater gives
    /// full power out of its front.
    fn repeater_power(id: u16, meta: u16, side: u8) -> i32 {
        if id == block::POWERED_REPEATER && repeater_facing(meta) == side {
            MAX_POWER
        } else {
            0
        }
    }

    /// `calculateInputStrength`: the power coming in at the back (wire counts with its
    /// own level).
    fn repeater_input(&mut self, pos: (i32, i32, i32), meta: u16) -> i32 {
        let facing = repeater_facing(meta);
        let back = offset(pos, facing);
        let power = self.redstone_power(back, facing);
        if power >= MAX_POWER {
            return power;
        }
        let s = self.level.overworld.block(back.0, back.1, back.2);
        let wire = if s >> 4 == block::REDSTONE_WIRE {
            i32::from(s & 15)
        } else {
            0
        };
        power.max(wire)
    }

    /// `isLocked`: a powered repeater points into one of its sides.
    fn repeater_locked(&mut self, pos: (i32, i32, i32), meta: u16) -> bool {
        let facing = repeater_facing(meta);
        for side in [rotate_y(facing), rotate_y(rotate_y(rotate_y(facing)))] {
            let n = offset(pos, side);
            let s = self.level.overworld.block(n.0, n.1, n.2);
            if is_repeater(s >> 4) && Self::repeater_power(s >> 4, s & 15, side) > 0 {
                return true;
            }
        }
        false
    }

    /// `isFacingTowardsRepeater`: the block in front is a repeater not facing back.
    fn repeater_faces_repeater(&mut self, pos: (i32, i32, i32), meta: u16) -> bool {
        let front = opposite(repeater_facing(meta));
        let n = offset(pos, front);
        let s = self.level.overworld.block(n.0, n.1, n.2);
        is_repeater(s >> 4) && repeater_facing(s & 15) != front
    }

    /// `BlockRedstoneDiode.notifyNeighbors`: the block in front and its neighbors
    /// except the repeater.
    pub(super) fn repeater_notify(&mut self, pos: (i32, i32, i32), state: u16) {
        let facing = repeater_facing(state & 15);
        let front = offset(pos, opposite(facing));
        let id = state >> 4;
        self.neighbor_changed(front, id);
        self.notify_neighbors_except(front, id, facing);
    }

    /// `BlockRedstoneDiode.updateState` (on a neighbor change).
    pub(super) fn repeater_update_state(&mut self, pos: (i32, i32, i32), state: u16) {
        let (id, meta) = (state >> 4, state & 15);
        if self.repeater_locked(pos, meta) {
            return;
        }
        let should = self.repeater_input(pos, meta) > 0;
        let powered = id == block::POWERED_REPEATER;
        if powered != should && !self.is_tick_pending_now(pos, id) {
            let priority = if self.repeater_faces_repeater(pos, meta) {
                PRIORITY_INTO_REPEATER
            } else if powered {
                PRIORITY_TURN_OFF
            } else {
                PRIORITY_TURN_ON
            };
            self.schedule_update_priority(pos, id, repeater_delay(meta), priority);
        }
    }

    /// `BlockRedstoneDiode.updateTick`.
    pub(super) fn repeater_tick(&mut self, pos: (i32, i32, i32), state: u16) {
        let (id, meta) = (state >> 4, state & 15);
        if self.repeater_locked(pos, meta) {
            return;
        }
        let should = self.repeater_input(pos, meta) > 0;
        let flags = super::block_update::flags::SEND;
        if id == block::POWERED_REPEATER && !should {
            self.set_block_flags(pos, block::UNPOWERED_REPEATER << 4 | meta, flags);
        } else if id == block::UNPOWERED_REPEATER {
            self.set_block_flags(pos, block::POWERED_REPEATER << 4 | meta, flags);
            if !should {
                self.schedule_update_priority(
                    pos,
                    block::POWERED_REPEATER,
                    repeater_delay(meta),
                    PRIORITY_TURN_ON,
                );
            }
        }
    }

    /// `BlockRedstoneDiode.onBlockPlacedBy`: an already powered input fires after one
    /// tick.
    pub(super) fn repeater_placed(&mut self, pos: (i32, i32, i32), state: u16) {
        if self.repeater_input(pos, state & 15) > 0 {
            self.schedule_update(pos, state >> 4, 1);
        }
    }

    /// `onBlockDestroyedByPlayer` of a powered repeater: updates around every neighbor.
    pub(super) fn repeater_destroyed(&mut self, pos: (i32, i32, i32), old: u16) {
        if old >> 4 == block::POWERED_REPEATER || old >> 4 == block::POWERED_COMPARATOR {
            self.torch_notify(pos, old >> 4);
        }
    }
}

// --- Comparators (vanilla: BlockRedstoneComparator, TileEntityComparator) ----------

/// Comparators react two ticks after a change.
const COMPARATOR_DELAY: i32 = 2;
/// Comparator metadata: subtract mode, powered.
const SUBTRACT: u16 = 4;
const COMPARATOR_POWERED: u16 = 8;
/// Scheduled tick priorities of comparators: facing into a repeater first.
const PRIORITY_COMPARATOR_INTO_REPEATER: i32 = -1;
const PRIORITY_COMPARATOR: i32 = 0;
/// Container signal: `floor(fullness * 14) + 1` for any non-empty container.
const CONTAINER_SIGNAL_STEPS: f32 = 14.0;
/// Container stack limit (`getInventoryStackLimit`).
const CONTAINER_STACK_LIMIT: i8 = 64;
/// Volume and pitches of the mode click.
const MODE_CLICK_VOLUME: f32 = 0.3;
const MODE_CLICK_SUBTRACT: f32 = 0.55;
const MODE_CLICK_COMPARE: f32 = 0.5;

/// Comparator blocks.
pub fn is_comparator(id: u16) -> bool {
    id == block::UNPOWERED_COMPARATOR || id == block::POWERED_COMPARATOR
}

/// `Container.calcRedstoneFromInventory`.
pub fn container_signal(items: &[Option<krol_protocol::packets::play::ItemStack>]) -> i32 {
    if items.is_empty() {
        return 0;
    }
    let mut fullness = 0.0f32;
    let mut filled = 0;
    for s in items.iter().flatten() {
        let limit = CONTAINER_STACK_LIMIT.min(super::item_data::max_stack(s.id));
        fullness += f32::from(s.count) / f32::from(limit);
        filled += 1;
    }
    fullness /= items.len() as f32;
    krol_java::math::floor_f32(fullness * CONTAINER_SIGNAL_STEPS) + i32::from(filled > 0)
}

impl Game {
    /// `TileEntityComparator.getOutputSignal`.
    fn comparator_signal(&self, pos: (i32, i32, i32)) -> i32 {
        self.level
            .overworld
            .tile_entity(pos)
            .map_or(0, |c| c.get_i32("OutputSignal"))
    }

    fn set_comparator_signal(&mut self, pos: (i32, i32, i32), signal: i32) {
        if let Some(c) = self.level.overworld.tile_entity_mut(pos) {
            c.insert("OutputSignal", krol_nbt::Tag::Int(signal));
        }
    }

    /// `BlockRedstoneComparator.isPowered`.
    fn comparator_powered(id: u16, meta: u16) -> bool {
        id == block::POWERED_COMPARATOR || meta & COMPARATOR_POWERED != 0
    }

    /// The comparator's weak and strong power out of its front (`getActiveSignal`).
    fn comparator_power(&self, pos: (i32, i32, i32), id: u16, meta: u16, side: u8) -> i32 {
        if Self::comparator_powered(id, meta) && repeater_facing(meta) == side {
            self.comparator_signal(pos)
        } else {
            0
        }
    }

    /// `getComparatorInputOverride` of the containers krol models: chests (the whole
    /// double chest; nothing if blocked) and furnaces.
    pub(super) fn comparator_override(&mut self, pos: (i32, i32, i32), state: u16) -> Option<i32> {
        let id = state >> 4;
        if block::is_chest(id) {
            let parts = self.chest_parts(pos);
            return Some(parts.map_or(0, |parts| container_signal(&self.chest_contents(&parts))));
        }
        if block::is_furnace(id) {
            return Some(container_signal(&self.furnace_items(pos)));
        }
        if id == block::HOPPER {
            return Some(container_signal(&self.hopper_items(pos)));
        }
        if super::dispenser::is_dispenser(id) {
            return Some(container_signal(&self.dispenser_items(pos)));
        }
        if id == super::cauldron::CAULDRON {
            return Some(i32::from(state & 15));
        }
        if id == super::jukebox::JUKEBOX {
            return Some(self.jukebox_signal(pos));
        }
        if id == super::brewing::BREWING_STAND {
            return Some(container_signal(&self.brewing_items(pos)));
        }
        None
    }

    /// `getPowerOnSides` with the diode's `canPowerSide` (any power source; wire counts
    /// with its level, others with their strong power).
    fn comparator_side_power(&mut self, pos: (i32, i32, i32), meta: u16) -> i32 {
        let facing = repeater_facing(meta);
        let mut max = 0;
        for side in [rotate_y(facing), rotate_y(rotate_y(rotate_y(facing)))] {
            let n = offset(pos, side);
            let s = self.level.overworld.block(n.0, n.1, n.2);
            if !self.provides_power(s >> 4) {
                continue;
            }
            let p = if s >> 4 == block::REDSTONE_WIRE {
                i32::from(s & 15)
            } else {
                self.strong_power(n, side)
            };
            max = max.max(p);
        }
        max
    }

    /// `BlockRedstoneComparator.calculateInputStrength`: the diode input, replaced by a
    /// container's signal behind it (or behind a cube).
    fn comparator_input(&mut self, pos: (i32, i32, i32), meta: u16) -> i32 {
        let mut power = self.repeater_input(pos, meta);
        let facing = repeater_facing(meta);
        let back = offset(pos, facing);
        let s = self.level.overworld.block(back.0, back.1, back.2);
        if let Some(signal) = self.comparator_override(back, s) {
            power = signal;
        } else if power < MAX_POWER && is_normal_cube(s) {
            let behind = offset(back, facing);
            let s2 = self.level.overworld.block(behind.0, behind.1, behind.2);
            if let Some(signal) = self.comparator_override(behind, s2) {
                power = signal;
            }
            // TODO(M5): item frames on the far side of the cube.
        }
        power
    }

    /// `calculateOutput`.
    fn comparator_output(&mut self, pos: (i32, i32, i32), meta: u16) -> i32 {
        let input = self.comparator_input(pos, meta);
        if meta & SUBTRACT != 0 {
            (input - self.comparator_side_power(pos, meta)).max(0)
        } else {
            input
        }
    }

    /// `shouldBePowered`.
    fn comparator_should_power(&mut self, pos: (i32, i32, i32), meta: u16) -> bool {
        let input = self.comparator_input(pos, meta);
        if input >= MAX_POWER {
            return true;
        }
        if input == 0 {
            return false;
        }
        let sides = self.comparator_side_power(pos, meta);
        sides == 0 || input >= sides
    }

    /// `BlockRedstoneComparator.updateState` (on a neighbor change).
    pub(super) fn comparator_update_state(&mut self, pos: (i32, i32, i32), state: u16) {
        let (id, meta) = (state >> 4, state & 15);
        if self.is_tick_pending_now(pos, id) {
            return;
        }
        let output = self.comparator_output(pos, meta);
        let old = self.comparator_signal(pos);
        let should = self.comparator_should_power(pos, meta);
        if output != old || Self::comparator_powered(id, meta) != should {
            let priority = if self.repeater_faces_repeater(pos, meta) {
                PRIORITY_COMPARATOR_INTO_REPEATER
            } else {
                PRIORITY_COMPARATOR
            };
            self.schedule_update_priority(pos, id, COMPARATOR_DELAY, priority);
        }
    }

    /// `onStateChange`: stores the new signal and updates the powered bit and the
    /// block in front.
    fn comparator_state_change(&mut self, pos: (i32, i32, i32)) {
        let state = self.level.overworld.block(pos.0, pos.1, pos.2);
        let (id, meta) = (state >> 4, state & 15);
        let output = self.comparator_output(pos, meta);
        let old = self.comparator_signal(pos);
        self.set_comparator_signal(pos, output);
        if old != output || meta & SUBTRACT == 0 {
            let should = self.comparator_should_power(pos, meta);
            let powered = Self::comparator_powered(id, meta);
            let flags = super::block_update::flags::SEND;
            if powered && !should {
                self.set_block_flags(pos, state & !COMPARATOR_POWERED, flags);
            } else if !powered && should {
                self.set_block_flags(pos, state | COMPARATOR_POWERED, flags);
            }
            let now = self.level.overworld.block(pos.0, pos.1, pos.2);
            self.repeater_notify(pos, now);
        }
    }

    /// `BlockRedstoneComparator.updateTick`: the old powered block becomes the 1.8 form
    /// (without updates or sending), then `onStateChange`.
    pub(super) fn comparator_tick(&mut self, pos: (i32, i32, i32), state: u16) {
        if state >> 4 == block::POWERED_COMPARATOR {
            let converted = block::UNPOWERED_COMPARATOR << 4 | (state & 15) | COMPARATOR_POWERED;
            self.set_block_flags(pos, converted, 0);
        }
        self.comparator_state_change(pos);
    }

    /// `BlockRedstoneComparator.onBlockActivated`: toggles compare / subtract.
    pub(super) fn activate_comparator(&mut self, pos: (i32, i32, i32), state: u16) {
        let toggled = state ^ SUBTRACT;
        let pitch = if toggled & SUBTRACT != 0 {
            MODE_CLICK_SUBTRACT
        } else {
            MODE_CLICK_COMPARE
        };
        let center = (
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 0.5,
            f64::from(pos.2) + 0.5,
        );
        self.play_sound_near_except(None, "random.click", center, MODE_CLICK_VOLUME, pitch);
        self.set_block_flags(pos, toggled, super::block_update::flags::SEND);
        self.comparator_state_change(pos);
    }

    /// `World.updateComparatorOutputLevel`: comparators next to `pos`, or behind a cube
    /// next to it, re-check their input.
    pub(super) fn update_comparators(&mut self, pos: (i32, i32, i32)) {
        for face in HORIZONTAL {
            let n = offset(pos, face);
            if !self.level.overworld.is_chunk_loaded(n.0 >> 4, n.2 >> 4) {
                continue;
            }
            let s = self.level.overworld.block(n.0, n.1, n.2);
            if is_comparator(s >> 4) {
                self.comparator_update_state(n, s);
            } else if is_normal_cube(s) {
                let far = offset(n, face);
                let s2 = self.level.overworld.block(far.0, far.1, far.2);
                if is_comparator(s2 >> 4) {
                    self.comparator_update_state(far, s2);
                }
            }
        }
    }
}
