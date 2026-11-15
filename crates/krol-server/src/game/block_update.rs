//! Block updates: `World.setBlockState` with update flags, neighbor notifications
//! (`notifyNeighborsOfStateChange` → `onNeighborBlockChange`), scheduled block ticks
//! (`updateTick`), and the blocks that use them so far: plants, torches, signs,
//! ladders, levers and buttons.
//!
//! TODO(M5): redstone power (levers and buttons only change their own state), falling
//! blocks, liquids, doors and the other blocks that react to neighbors;
//! `onBlockAdded`; comparator updates.

use super::Game;
use super::ids::block;
use super::placement::{
    self, DOWN, EAST, NORTH, SOUTH, UP, WEST, is_normal_cube, offset, opposite, solid_top,
};

/// `setBlockState` flags.
pub mod flags {
    /// Notify the neighbors (`onNeighborBlockChange`).
    pub const NOTIFY: u8 = 1;
    /// Send the change to clients. krol always does; kept for readability.
    pub const SEND: u8 = 2;
    /// Both (vanilla's default).
    pub const DEFAULT: u8 = NOTIFY | SEND;
}

/// The order `notifyNeighborsOfStateChange` visits the neighbors in.
const NEIGHBOR_ORDER: [u8; 6] = [WEST, EAST, DOWN, UP, NORTH, SOUTH];
/// Button press duration in ticks (`tickRate`): wooden buttons stay pressed longer.
const STONE_BUTTON_TICKS: i32 = 20;
const WOODEN_BUTTON_TICKS: i32 = 30;
/// The powered bit of levers and buttons.
const POWERED: u16 = 8;
/// Volume and pitches of `random.click`.
const CLICK_VOLUME: f32 = 0.3;
const CLICK_ON: f32 = 0.6;
const CLICK_OFF: f32 = 0.5;

use super::redstone::mount_facing;

/// `BlockButton.func_181088_a`: the block on side `dir` can hold a button or lever
/// (solid top when below, else a normal cube).
pub fn can_mount(world: &mut super::world::World, pos: (i32, i32, i32), dir: u8) -> bool {
    let n = offset(pos, dir);
    let s = world.block(n.0, n.1, n.2);
    if dir == DOWN {
        solid_top(s)
    } else {
        is_normal_cube(s)
    }
}

/// What a plant needs below it (`canPlaceBlockOn` of the bushes).
fn plant_soil(plant: u16, below: u16) -> Option<bool> {
    Some(match plant {
        block::SAPLING | block::TALLGRASS | block::YELLOW_FLOWER | block::RED_FLOWER => {
            matches!(below, block::GRASS | block::DIRT | block::FARMLAND)
        }
        block::DEADBUSH => matches!(
            below,
            block::SAND | block::HARDENED_CLAY | block::STAINED_HARDENED_CLAY | block::DIRT
        ),
        block::WHEAT | block::CARROTS | block::POTATOES => below == block::FARMLAND,
        block::NETHER_WART => below == block::SOUL_SAND,
        _ => return None,
    })
}

impl Game {
    /// `World.setBlockState(pos, state, flags)`: sets the block, runs the old block's
    /// `breakBlock` and, with [`flags::NOTIFY`], notifies the neighbors with the old
    /// block.
    pub(super) fn set_block_flags(&mut self, pos: (i32, i32, i32), state: u16, flags: u8) {
        let w = &mut self.level.overworld;
        if !(0..256).contains(&pos.1) {
            return;
        }
        let old = w.block(pos.0, pos.1, pos.2);
        if old == state {
            return;
        }
        if !w.set_block_unmarked(pos.0, pos.1, pos.2, state) {
            return;
        }
        if old >> 4 != state >> 4 {
            self.block_removed(pos, old);
            self.block_added(pos, state);
        }
        // World.setBlockState: the light check before the update.
        self.level.overworld.check_light_if_changed(pos, old, state);
        if flags & flags::SEND != 0 {
            self.level
                .overworld
                .mark_block_for_update(pos.0, pos.1, pos.2);
        }
        if flags & flags::NOTIFY != 0 {
            self.notify_neighbors(pos, old >> 4);
            // hasComparatorInputOverride: containers update the comparators reading them.
            let id = state >> 4;
            if super::ids::block::is_chest(id)
                || super::ids::block::is_furnace(id)
                || id == super::ids::block::HOPPER
                || super::dispenser::is_dispenser(id)
            {
                self.update_comparators(pos);
            }
        }
    }

    /// `setBlockState(pos, state)` with the default flags.
    pub(super) fn set_block_notify(&mut self, pos: (i32, i32, i32), state: u16) {
        self.set_block_flags(pos, state, flags::DEFAULT);
    }

    /// `notifyNeighborsOfStateChange`.
    pub(super) fn notify_neighbors(&mut self, pos: (i32, i32, i32), source: u16) {
        for dir in NEIGHBOR_ORDER {
            self.neighbor_changed(offset(pos, dir), source);
        }
    }

    /// `BlockBush.checkAndDropBlock`: a plant without its soil pops off.
    pub(super) fn check_plant(&mut self, pos: (i32, i32, i32), state: u16) {
        let below = self.level.overworld.block(pos.0, pos.1 - 1, pos.2) >> 4;
        if plant_soil(state >> 4, below) == Some(false) {
            self.pop_off(pos, state);
        }
    }

    /// `dropBlockAsItem` + `setBlockToAir` for other modules.
    pub(super) fn pop_off_block(&mut self, pos: (i32, i32, i32), state: u16) {
        self.pop_off(pos, state);
    }

    /// `dropBlockAsItem` + `setBlockToAir`.
    fn pop_off(&mut self, pos: (i32, i32, i32), state: u16) {
        self.drop_block_items(pos, state);
        self.set_block_notify(pos, 0);
    }

    /// `notifyNeighborsOfStateExcept`: like [`Self::notify_neighbors`], skipping one
    /// side.
    pub(super) fn notify_neighbors_except(
        &mut self,
        pos: (i32, i32, i32),
        source: u16,
        except: u8,
    ) {
        for dir in NEIGHBOR_ORDER {
            if dir != except {
                self.neighbor_changed(offset(pos, dir), source);
            }
        }
    }

    /// `onNeighborBlockChange` of the block at `pos` (`notifyBlockOfStateChange`).
    pub(super) fn neighbor_changed(&mut self, pos: (i32, i32, i32), source: u16) {
        let w = &mut self.level.overworld;
        let state = w.block(pos.0, pos.1, pos.2);
        let (id, meta) = (state >> 4, state & 15);
        if super::falling_block::falls(id) {
            self.schedule_update(pos, id, super::falling_block::FALL_DELAY);
            return;
        }
        if self.powered_block_changed(pos, state, source) {
            return;
        }
        match id {
            _ if super::liquids::is_liquid(id) => {
                self.liquid_neighbor_changed(pos, state);
                return;
            }
            block::FIRE => {
                self.fire_neighbor_changed(pos);
                return;
            }
            super::vines::VINE => {
                self.vine_neighbor_changed(pos, state);
                return;
            }
            super::farming::CAKE => {
                self.cake_neighbor_changed(pos);
                return;
            }
            block::REDSTONE_WIRE => {
                if self.wire_can_stay(pos) {
                    self.wire_neighbor_changed(pos, state);
                } else {
                    self.pop_off(pos, state);
                }
                return;
            }
            block::REDSTONE_TORCH | block::UNLIT_REDSTONE_TORCH => {
                let face = super::redstone::torch_facing(meta);
                if placement::torch_can_attach(&mut self.level.overworld, pos, face) {
                    self.torch_neighbor_changed(pos, state);
                } else {
                    self.pop_off(pos, state);
                }
                return;
            }
            block::REDSTONE_LAMP | block::LIT_REDSTONE_LAMP => {
                self.lamp_neighbor_changed(pos, id);
                return;
            }
            block::PISTON | block::STICKY_PISTON => {
                self.piston_check(pos, state);
                return;
            }
            block::PISTON_HEAD => {
                self.piston_head_changed(pos, state);
                return;
            }
            block::PISTON_EXTENSION => return,
            block::NOTEBLOCK => {
                self.note_block_changed(pos);
                return;
            }
            // BlockCactus: destroyBlock without support; BlockReed: checkForDrop.
            block::CACTUS | block::REEDS => {
                if !self.cane_stays(pos, id == block::CACTUS) {
                    if id == block::CACTUS {
                        self.destroy_block(pos, state);
                    } else {
                        self.pop_off(pos, state);
                    }
                }
                return;
            }
            block::HOPPER => {
                self.hopper_update_state(pos, state);
                return;
            }
            block::DISPENSER | block::DROPPER => {
                self.dispenser_changed(pos, state);
                return;
            }
            _ if super::pressure_plates::is_plate(id) => {
                if !self.plate_can_stay(pos) {
                    self.pop_off(pos, state);
                }
                return;
            }
            block::UNPOWERED_REPEATER
            | block::POWERED_REPEATER
            | block::UNPOWERED_COMPARATOR
            | block::POWERED_COMPARATOR => {
                let below = self.level.overworld.block(pos.0, pos.1 - 1, pos.2);
                if !solid_top(below) {
                    self.pop_off(pos, state);
                    self.torch_notify(pos, id);
                } else if super::redstone::is_comparator(id) {
                    self.comparator_update_state(pos, state);
                } else {
                    self.repeater_update_state(pos, state);
                }
                return;
            }
            _ => {}
        }
        let w = &mut self.level.overworld;
        let below = w.block(pos.0, pos.1 - 1, pos.2);
        let stays = if let Some(ok) = plant_soil(id, below >> 4) {
            // BlockBush.checkAndDropBlock.
            ok
        } else {
            match id {
                block::TORCH => {
                    let face = match meta {
                        1 => EAST,
                        2 => WEST,
                        3 => SOUTH,
                        4 => NORTH,
                        _ => UP,
                    };
                    placement::torch_can_attach(w, pos, face)
                }
                block::STANDING_SIGN => super::sign::solid_material(below),
                block::WALL_SIGN => {
                    let n = offset(pos, opposite(meta as u8));
                    super::sign::solid_material(w.block(n.0, n.1, n.2))
                }
                block::LADDER => {
                    let n = offset(pos, opposite(meta as u8));
                    is_normal_cube(w.block(n.0, n.1, n.2))
                }
                block::LEVER | block::STONE_BUTTON | block::WOODEN_BUTTON => {
                    // checkForDrop (any side can hold it), then its own side.
                    let any = [DOWN, UP, NORTH, SOUTH, WEST, EAST]
                        .iter()
                        .any(|&d| can_mount(w, pos, d));
                    any && can_mount(w, pos, opposite(mount_facing(id, meta)))
                }
                _ => true,
            }
        };
        if !stays {
            self.pop_off(pos, state);
        }
    }

    /// `onBlockAdded` of the new block: sand and gravel check whether to fall.
    fn block_added(&mut self, pos: (i32, i32, i32), state: u16) {
        let id = state >> 4;
        match id {
            _ if super::falling_block::falls(id) => {
                self.schedule_update(pos, id, super::falling_block::FALL_DELAY);
            }
            _ if super::liquids::is_liquid(id) => self.liquid_added(pos, state),
            block::FIRE => self.fire_added(pos),
            block::REDSTONE_WIRE => self.wire_added(pos, state),
            block::REDSTONE_TORCH => self.torch_notify(pos, id),
            block::REDSTONE_LAMP | block::LIT_REDSTONE_LAMP => self.lamp_added(pos, id),
            // BlockPistonBase.onBlockAdded (pistons never have a tile entity).
            block::PISTON | block::STICKY_PISTON => self.piston_check(pos, state),
            block::HOPPER => self.hopper_update_state(pos, state),
            block::UNPOWERED_REPEATER
            | block::POWERED_REPEATER
            | block::UNPOWERED_COMPARATOR
            | block::POWERED_COMPARATOR => self.repeater_notify(pos, state),
            _ => {}
        }
    }

    /// `onBlockDestroyedByPlayer`, after a player broke the block.
    pub(super) fn destroyed_by_player(&mut self, pos: (i32, i32, i32), old: u16) {
        self.repeater_destroyed(pos, old);
    }

    /// `breakBlock` of the removed block: a powered lever or button updates around
    /// itself and the block it was on.
    fn block_removed(&mut self, pos: (i32, i32, i32), old: u16) {
        let (id, meta) = (old >> 4, old & 15);
        self.leaves_block_removed(pos, old);
        match id {
            block::LEVER | block::STONE_BUTTON | block::WOODEN_BUTTON if meta & POWERED != 0 => {
                self.notify_mounted(pos, id, meta);
            }
            block::REDSTONE_WIRE => self.wire_removed(pos, old),
            block::REDSTONE_TORCH => self.torch_notify(pos, id),
            block::PISTON_HEAD => self.piston_head_removed(pos, old),
            _ if super::pressure_plates::is_plate(id) => self.plate_removed(pos, old),
            block::PISTON_EXTENSION => self.moving_removed(pos),
            block::UNPOWERED_REPEATER
            | block::POWERED_REPEATER
            | block::UNPOWERED_COMPARATOR
            | block::POWERED_COMPARATOR => self.repeater_notify(pos, old),
            _ => {}
        }
    }

    /// Notifies around a lever or button and around the block it is mounted on.
    fn notify_mounted(&mut self, pos: (i32, i32, i32), id: u16, meta: u16) {
        self.notify_neighbors(pos, id);
        let mounted_on = offset(pos, opposite(mount_facing(id, meta)));
        self.notify_neighbors(mounted_on, id);
    }

    fn click(&self, pos: (i32, i32, i32), pitch: f32) {
        let center = (
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 0.5,
            f64::from(pos.2) + 0.5,
        );
        self.play_sound_near_except(None, "random.click", center, CLICK_VOLUME, pitch);
    }

    /// `BlockLever.onBlockActivated`: flips the lever.
    pub(super) fn activate_lever(&mut self, pos: (i32, i32, i32)) {
        let state = self.level.overworld.block(pos.0, pos.1, pos.2);
        let flipped = state ^ POWERED;
        self.set_block_notify(pos, flipped);
        let on = flipped & POWERED != 0;
        self.click(pos, if on { CLICK_ON } else { CLICK_OFF });
        self.notify_mounted(pos, block::LEVER, flipped & 15);
    }

    /// `BlockButton.onBlockActivated`: presses the button until its scheduled tick.
    pub(super) fn activate_button(&mut self, pos: (i32, i32, i32)) {
        let state = self.level.overworld.block(pos.0, pos.1, pos.2);
        let id = state >> 4;
        if state & POWERED != 0 {
            return;
        }
        let pressed = state | POWERED;
        self.set_block_notify(pos, pressed);
        self.click(pos, CLICK_ON);
        self.notify_mounted(pos, id, pressed & 15);
        let delay = if id == block::WOODEN_BUTTON {
            WOODEN_BUTTON_TICKS
        } else {
            STONE_BUTTON_TICKS
        };
        self.schedule_update(pos, id, delay);
    }

    /// `World.scheduleUpdate`: a tick for the block at `pos` after `delay` ticks, if its
    /// chunk is loaded.
    pub(super) fn schedule_update(&mut self, pos: (i32, i32, i32), id: u16, delay: i32) {
        self.schedule_update_priority(pos, id, delay, 0);
    }

    /// `World.updateBlockTick` with a priority (lower runs first at the same time).
    pub(super) fn schedule_update_priority(
        &mut self,
        pos: (i32, i32, i32),
        id: u16,
        delay: i32,
        priority: i32,
    ) {
        let w = &mut self.level.overworld;
        if !(0..256).contains(&pos.1) || !w.is_chunk_loaded(pos.0 >> 4, pos.2 >> 4) {
            return;
        }
        let now = w.total_time;
        w.scheduler.schedule(pos, id, delay, priority, now);
    }

    /// `WorldServer.isBlockTickPending`: vanilla only looks at the ticks still waiting
    /// in the batch being run this tick.
    pub(super) fn is_tick_pending_now(&self, pos: (i32, i32, i32), id: u16) -> bool {
        let id = super::redstone::associated(id);
        self.ticks_this_tick
            .iter()
            .any(|&(p, b)| p == pos && super::redstone::associated(b) == id)
    }

    /// `WorldServer.tickUpdates`: runs the due scheduled ticks whose block is still
    /// there.
    pub(super) fn tick_scheduled(&mut self) {
        let now = self.level.data.time();
        self.level.overworld.total_time = now;
        self.ticks_this_tick = self.level.overworld.scheduler.take_due(now);
        while !self.ticks_this_tick.is_empty() {
            let (pos, id) = self.ticks_this_tick.remove(0);
            let state = self.level.overworld.block(pos.0, pos.1, pos.2);
            let associated = super::redstone::associated;
            if state >> 4 != 0 && associated(state >> 4) == associated(id) {
                self.update_tick(pos, state);
            }
        }
    }

    /// `Block.updateTick`.
    fn update_tick(&mut self, pos: (i32, i32, i32), state: u16) {
        let (id, meta) = (state >> 4, state & 15);
        match id {
            // BlockButton.updateTick (wooden buttons: checkForArrows, no arrows yet).
            block::STONE_BUTTON | block::WOODEN_BUTTON if meta & POWERED != 0 => {
                let released = state & !POWERED;
                self.set_block_notify(pos, released);
                self.notify_mounted(pos, id, released & 15);
                self.click(pos, CLICK_OFF);
            }
            _ if super::falling_block::falls(id) => self.check_fallable(pos, state),
            block::FLOWING_WATER | block::FLOWING_LAVA => self.flowing_liquid_tick(pos, state),
            block::FIRE => self.fire_tick(pos, state),
            block::REDSTONE_TORCH | block::UNLIT_REDSTONE_TORCH => self.torch_tick(pos, state),
            block::REDSTONE_LAMP | block::LIT_REDSTONE_LAMP => self.lamp_tick(pos, id),
            _ if super::pressure_plates::is_plate(id) => self.plate_tick(pos, state),
            block::DISPENSER | block::DROPPER => self.dispense(pos, state),
            block::UNPOWERED_REPEATER | block::POWERED_REPEATER => self.repeater_tick(pos, state),
            block::UNPOWERED_COMPARATOR | block::POWERED_COMPARATOR => {
                self.comparator_tick(pos, state);
            }
            _ => {
                let below = self.level.overworld.block(pos.0, pos.1 - 1, pos.2) >> 4;
                if plant_soil(id, below) == Some(false) {
                    self.pop_off(pos, state);
                }
            }
        }
    }
}
