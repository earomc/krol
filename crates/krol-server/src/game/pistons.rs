//! Pistons (vanilla: `BlockPistonBase`, `BlockPistonStructureHelper`,
//! `BlockPistonExtension`, `BlockPistonMoving`, `TileEntityPiston`).
//!
//! TODO(M5): moving and launching entities (`launchWithSlimeBlock`), the world border.

use krol_nbt::{Compound, Tag};

use super::Game;
use super::block_data::{self, Material};
use super::ids::block;
use super::placement::{DOWN, EAST, NORTH, SOUTH, UP, WEST, offset, opposite};

/// Block event ids: extend and retract (the parameter is the facing).
pub const EVENT_EXTEND: u8 = 0;
pub const EVENT_RETRACT: u8 = 1;
/// Piston metadata: extended (base), sticky (head and moving block).
const EXTENDED: u16 = 8;
const STICKY: u16 = 8;
/// A piston moves at most this many blocks.
const PUSH_LIMIT: usize = 12;
/// Moving blocks advance this much per tick (`TileEntityPiston.update`).
const PROGRESS_STEP: f32 = 0.5;
/// Piston sounds: volume, and random pitch base and range.
const SOUND_VOLUME: f32 = 0.5;
const OUT_PITCH_RANGE: f32 = 0.25;
const IN_PITCH_RANGE: f32 = 0.15;
const PITCH_BASE: f32 = 0.6;

/// `EnumFacing.values()` order.
const ALL_FACES: [u8; 6] = [DOWN, UP, NORTH, SOUTH, WEST, EAST];

/// Piston bases.
pub fn is_piston(id: u16) -> bool {
    id == block::PISTON || id == block::STICKY_PISTON
}

/// The facing of a piston base, head or moving block (`EnumFacing.getFront`).
pub fn facing(meta: u16) -> u8 {
    match meta & 7 {
        0 => DOWN,
        1 => UP,
        2 => NORTH,
        3 => SOUTH,
        4 => WEST,
        _ => EAST,
    }
}

fn facing_index(face: u8) -> u16 {
    u16::from(face)
}

/// `Block.getMobilityFlag`: 0 pushable, 1 destroyed by a push, 2 immovable.
fn mobility(id: u16) -> u8 {
    match id {
        block::STONE_PRESSURE_PLATE
        | block::WOODEN_PRESSURE_PLATE
        | block::LIGHT_PRESSURE_PLATE
        | block::HEAVY_PRESSURE_PLATE
        | block::BED => return 1,
        _ if super::powered_blocks::is_door(id) => return 1,
        block::ICE
        | block::RAIL
        | block::DETECTOR_RAIL
        | block::GOLDEN_RAIL
        | block::ACTIVATOR_RAIL => {
            return 0;
        }
        _ => {}
    }
    use Material::*;
    match block_data::material(id) {
        Anvil | Portal | Piston | Barrier => 2,
        Water | Lava | Leaves | Plants | Vine | Fire | Circuits | Coral | Snow | Cactus | Gourd
        | DragonEgg | Cake | Web => 1,
        _ => 0,
    }
}

/// The piston tile entity of a moving block (`TileEntityPiston`). Only
/// `lastProgress` is saved (as `progress`); the current `progress` is kept in
/// `Game::moving_progress`.
#[derive(Debug, Clone, Copy)]
struct Moving {
    /// The block it carries (`pistonState`).
    state: u16,
    facing: u8,
    /// `lastProgress`.
    last: f32,
    extending: bool,
}

impl Moving {
    fn read(c: &Compound) -> Self {
        Self {
            state: (c.get_i32("blockId") as u16) << 4 | (c.get_i32("blockData") as u16 & 15),
            facing: c.get_i32("facing") as u8,
            last: c.get_f32("progress"),
            extending: c.get_bool("extending"),
        }
    }

    fn write(&self, pos: (i32, i32, i32)) -> Compound {
        let mut c = Compound::new();
        c.insert("id", Tag::String("Piston".into()));
        c.insert("x", Tag::Int(pos.0));
        c.insert("y", Tag::Int(pos.1));
        c.insert("z", Tag::Int(pos.2));
        c.insert("blockId", Tag::Int(i32::from(self.state >> 4)));
        c.insert("blockData", Tag::Int(i32::from(self.state & 15)));
        c.insert("facing", Tag::Int(i32::from(self.facing)));
        c.insert("progress", Tag::Float(self.last));
        c.insert("extending", Tag::Byte(i8::from(self.extending)));
        c
    }
}

/// `BlockPistonStructureHelper`: what a push or pull moves and destroys.
struct Structure {
    piston: (i32, i32, i32),
    start: (i32, i32, i32),
    direction: u8,
    to_move: Vec<(i32, i32, i32)>,
    to_destroy: Vec<(i32, i32, i32)>,
}

impl Game {
    fn block_id_at(&mut self, pos: (i32, i32, i32)) -> u16 {
        self.level.overworld.block(pos.0, pos.1, pos.2) >> 4
    }

    /// `BlockPistonBase.canPush`.
    fn can_push(&mut self, id: u16, pos: (i32, i32, i32), dir: u8, allow_destroy: bool) -> bool {
        if id == block::OBSIDIAN {
            return false;
        }
        if pos.1 < 0 || (dir == DOWN && pos.1 == 0) || pos.1 > 255 || (dir == UP && pos.1 == 255) {
            return false;
        }
        if is_piston(id) {
            let state = self.level.overworld.block(pos.0, pos.1, pos.2);
            if state & EXTENDED != 0 {
                return false;
            }
        } else {
            if block_data::hardness(id) == -1.0 {
                return false;
            }
            match mobility(id) {
                2 => return false,
                1 => return allow_destroy,
                _ => {}
            }
        }
        !super::world::has_tile_entity(id)
    }

    fn structure(&self, piston: (i32, i32, i32), facing: u8, extending: bool) -> Structure {
        let (direction, start) = if extending {
            (facing, offset(piston, facing))
        } else {
            (opposite(facing), offset(offset(piston, facing), facing))
        };
        Structure {
            piston,
            start,
            direction,
            to_move: Vec::new(),
            to_destroy: Vec::new(),
        }
    }

    /// `BlockPistonStructureHelper.canMove`.
    fn can_move(&mut self, s: &mut Structure) -> bool {
        s.to_move.clear();
        s.to_destroy.clear();
        let id = self.block_id_at(s.start);
        if !self.can_push(id, s.start, s.direction, false) {
            if mobility(id) != 1 {
                return false;
            }
            s.to_destroy.push(s.start);
            return true;
        }
        if !self.add_line(s, s.start) {
            return false;
        }
        let mut k = 0;
        while k < s.to_move.len() {
            let p = s.to_move[k];
            if self.block_id_at(p) == block::SLIME && !self.add_branches(s, p) {
                return false;
            }
            k += 1;
        }
        true
    }

    /// `func_177251_a`: adds the line of blocks through `pos` (with slime blocks behind
    /// it) to the structure.
    fn add_line(&mut self, s: &mut Structure, pos: (i32, i32, i32)) -> bool {
        let mut id = self.block_id_at(pos);
        if id == block::AIR
            || !self.can_push(id, pos, s.direction, false)
            || pos == s.piston
            || s.to_move.contains(&pos)
        {
            return true;
        }
        let back = opposite(s.direction);
        let mut behind = 1usize;
        if behind + s.to_move.len() > PUSH_LIMIT {
            return false;
        }
        while id == block::SLIME {
            let p = step(pos, back, behind as i32);
            id = self.block_id_at(p);
            if id == block::AIR || !self.can_push(id, p, s.direction, false) || p == s.piston {
                break;
            }
            behind += 1;
            if behind + s.to_move.len() > PUSH_LIMIT {
                return false;
            }
        }
        let mut added = 0usize;
        for k in (0..behind).rev() {
            s.to_move.push(step(pos, back, k as i32));
            added += 1;
        }
        let mut ahead = 1;
        loop {
            let p = step(pos, s.direction, ahead);
            if let Some(index) = s.to_move.iter().position(|&m| m == p) {
                reorder(&mut s.to_move, added, index);
                for k in 0..=(index + added) {
                    let m = s.to_move[k];
                    if self.block_id_at(m) == block::SLIME && !self.add_branches(s, m) {
                        return false;
                    }
                }
                return true;
            }
            let id = self.block_id_at(p);
            if id == block::AIR {
                return true;
            }
            if !self.can_push(id, p, s.direction, true) || p == s.piston {
                return false;
            }
            if mobility(id) == 1 {
                s.to_destroy.push(p);
                return true;
            }
            if s.to_move.len() >= PUSH_LIMIT {
                return false;
            }
            s.to_move.push(p);
            added += 1;
            ahead += 1;
        }
    }

    /// `func_177250_b`: a slime block pulls the lines beside it (off the move axis).
    fn add_branches(&mut self, s: &mut Structure, pos: (i32, i32, i32)) -> bool {
        let axis = axis(s.direction);
        for face in ALL_FACES {
            if axis_of(face) != axis && !self.add_line(s, offset(pos, face)) {
                return false;
            }
        }
        true
    }

    /// `BlockPistonBase.shouldBeExtended`, with quasi-connectivity (power around the
    /// block above counts).
    fn piston_powered(&mut self, pos: (i32, i32, i32), facing: u8) -> bool {
        for face in ALL_FACES {
            if face != facing && self.side_powered(offset(pos, face), face) {
                return true;
            }
        }
        if self.side_powered(pos, DOWN) {
            return true;
        }
        let up = (pos.0, pos.1 + 1, pos.2);
        ALL_FACES
            .iter()
            .any(|&face| face != DOWN && self.side_powered(offset(up, face), face))
    }

    /// `BlockPistonBase.checkForMove`.
    pub(super) fn piston_check(&mut self, pos: (i32, i32, i32), state: u16) {
        let (id, meta) = (state >> 4, state & 15);
        let face = facing(meta);
        let powered = self.piston_powered(pos, face);
        if powered && meta & EXTENDED == 0 {
            let mut s = self.structure(pos, face, true);
            if self.can_move(&mut s) {
                self.queue_block_event(pos, id, EVENT_EXTEND, i32::from(facing_index(face)));
            }
        } else if !powered && meta & EXTENDED != 0 {
            self.set_block_flags(pos, state & !EXTENDED, super::block_update::flags::SEND);
            self.queue_block_event(pos, id, EVENT_RETRACT, i32::from(facing_index(face)));
        }
    }

    /// The moving block state with its tile entity at `pos` (flags 4: no updates, not
    /// sent).
    fn place_moving(&mut self, pos: (i32, i32, i32), moving_meta: u16, data: Moving) {
        self.set_block_flags(pos, block::PISTON_EXTENSION << 4 | moving_meta, 0);
        self.level.overworld.set_tile_entity(pos, data.write(pos));
        self.level.overworld.moving.push(pos);
        self.moving_progress.insert(pos, 0.0);
    }

    /// `BlockPistonBase.doMove`.
    fn piston_move(
        &mut self,
        pos: (i32, i32, i32),
        face: u8,
        extending: bool,
        sticky: bool,
    ) -> bool {
        if !extending {
            self.set_block_notify(offset(pos, face), 0);
        }
        let mut s = self.structure(pos, face, extending);
        if !self.can_move(&mut s) {
            return false;
        }
        let mut n = s.to_move.len() + s.to_destroy.len();
        let mut blocks = vec![0u16; n];
        let dir = if extending { face } else { opposite(face) };
        for &p in s.to_destroy.iter().rev() {
            let state = self.level.overworld.block(p.0, p.1, p.2);
            self.drop_block_items(p, state);
            self.set_block_notify(p, 0);
            n -= 1;
            blocks[n] = state >> 4;
        }
        for &p in s.to_move.iter().rev() {
            let state = self.level.overworld.block(p.0, p.1, p.2);
            self.set_block_notify(p, 0);
            let to = offset(p, dir);
            let data = Moving {
                state,
                facing: face,
                last: 0.0,
                extending,
            };
            self.place_moving(to, facing_index(face), data);
            n -= 1;
            blocks[n] = state >> 4;
        }
        let head = offset(pos, face);
        let sticky_bit = if sticky { STICKY } else { 0 };
        if extending {
            let data = Moving {
                state: block::PISTON_HEAD << 4 | facing_index(face) | sticky_bit,
                facing: face,
                last: 0.0,
                extending: true,
            };
            self.place_moving(head, facing_index(face) | sticky_bit, data);
        }
        // Vanilla quirk: the source blocks are taken from the array from the start, so
        // they do not match the positions.
        let mut k = 0;
        for &p in s.to_destroy.iter().rev() {
            self.notify_neighbors(p, blocks[k]);
            k += 1;
        }
        for &p in s.to_move.iter().rev() {
            self.notify_neighbors(p, blocks[k]);
            k += 1;
        }
        if extending {
            self.notify_neighbors(head, block::PISTON_HEAD);
            let id = if sticky {
                block::STICKY_PISTON
            } else {
                block::PISTON
            };
            self.notify_neighbors(pos, id);
        }
        true
    }

    fn piston_sound(&mut self, pos: (i32, i32, i32), name: &str, range: f32) {
        let pitch = self.world_rand.next_float() * range + PITCH_BASE;
        let center = (
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 0.5,
            f64::from(pos.2) + 0.5,
        );
        self.play_sound_near_except(None, name, center, SOUND_VOLUME, pitch);
    }

    /// `BlockPistonBase.onBlockEventReceived`; returns whether clients get the event.
    pub(super) fn piston_event(
        &mut self,
        pos: (i32, i32, i32),
        state: u16,
        action: u8,
        param: i32,
    ) -> bool {
        let (id, meta) = (state >> 4, state & 15);
        let face = facing(meta);
        let sticky = id == block::STICKY_PISTON;
        let powered = self.piston_powered(pos, face);
        let flags = super::block_update::flags::SEND;
        if powered && action == EVENT_RETRACT {
            self.set_block_flags(pos, state | EXTENDED, flags);
            return false;
        }
        if !powered && action == EVENT_EXTEND {
            return false;
        }
        if action == EVENT_EXTEND {
            if !self.piston_move(pos, face, true, sticky) {
                return false;
            }
            self.set_block_flags(pos, state | EXTENDED, flags);
            self.piston_sound(pos, "tile.piston.out", OUT_PITCH_RANGE);
        } else if action == EVENT_RETRACT {
            let head = offset(pos, face);
            self.clear_moving(head);
            let sticky_bit = if sticky { STICKY } else { 0 };
            self.set_block_notify(
                pos,
                block::PISTON_EXTENSION << 4 | facing_index(face) | sticky_bit,
            );
            let data = Moving {
                state: id << 4 | (param as u16 & 7),
                facing: face,
                last: 0.0,
                extending: false,
            };
            self.level.overworld.set_tile_entity(pos, data.write(pos));
            self.level.overworld.moving.push(pos);
            self.moving_progress.insert(pos, 0.0);
            if sticky {
                let far = offset(head, face);
                let far_id = self.block_id_at(far);
                let mut cleared = false;
                if far_id == block::PISTON_EXTENSION
                    && let Some(m) = self.level.overworld.tile_entity(far).map(Moving::read)
                    && m.facing == face
                    && m.extending
                {
                    self.clear_moving(far);
                    cleared = true;
                }
                if !cleared
                    && far_id != block::AIR
                    && self.can_push(far_id, far, opposite(face), false)
                    && (mobility(far_id) == 0 || is_piston(far_id))
                {
                    self.piston_move(pos, face, false, sticky);
                }
            } else {
                self.set_block_notify(head, 0);
            }
            self.piston_sound(pos, "tile.piston.in", IN_PITCH_RANGE);
        }
        true
    }

    /// `TileEntityPiston.clearPistonTileEntity`: finishes a move at once.
    fn clear_moving(&mut self, pos: (i32, i32, i32)) {
        let Some(m) = self
            .level
            .overworld
            .tile_entity(pos)
            .filter(|c| c.get_str("id") == "Piston")
            .map(Moving::read)
        else {
            return;
        };
        if m.last >= 1.0 {
            return;
        }
        self.finish_moving(pos, m);
    }

    /// The moving block becomes its block (`removeTileEntity`, `setBlockState`,
    /// `notifyBlockOfStateChange`).
    fn finish_moving(&mut self, pos: (i32, i32, i32), m: Moving) {
        self.level.overworld.remove_tile_entity(pos);
        self.level.overworld.moving.retain(|&p| p != pos);
        self.moving_progress.remove(&pos);
        if self.block_id_at(pos) == block::PISTON_EXTENSION {
            self.set_block_notify(pos, m.state);
            self.neighbor_changed(pos, m.state >> 4);
        }
    }

    /// `TileEntityPiston.update` for every moving block, in creation order.
    pub(super) fn tick_moving(&mut self) {
        let list = self.level.overworld.moving.clone();
        for pos in list {
            let Some(mut m) = self
                .level
                .overworld
                .tile_entity(pos)
                .filter(|c| c.get_str("id") == "Piston")
                .map(Moving::read)
            else {
                self.level.overworld.moving.retain(|&p| p != pos);
                continue;
            };
            let progress = self.moving_progress.get(&pos).copied().unwrap_or(m.last);
            m.last = progress;
            if m.last >= 1.0 {
                self.finish_moving(pos, m);
            } else {
                self.moving_progress
                    .insert(pos, (progress + PROGRESS_STEP).min(1.0));
                // TODO(M5): extending moves push entities (launchWithSlimeBlock).
                self.level.overworld.set_tile_entity(pos, m.write(pos));
            }
        }
    }

    /// `BlockPistonExtension.onNeighborBlockChange`: a head without its base goes;
    /// otherwise the base checks itself.
    pub(super) fn piston_head_changed(&mut self, pos: (i32, i32, i32), state: u16) {
        let base = offset(pos, opposite(facing(state & 15)));
        let base_state = self.level.overworld.block(base.0, base.1, base.2);
        if is_piston(base_state >> 4) {
            self.piston_check(base, base_state);
        } else {
            self.set_block_notify(pos, 0);
        }
    }

    /// `BlockPistonExtension.breakBlock`: an extended base behind the head breaks too.
    pub(super) fn piston_head_removed(&mut self, pos: (i32, i32, i32), old: u16) {
        let base = offset(pos, opposite(facing(old & 15)));
        let base_state = self.level.overworld.block(base.0, base.1, base.2);
        if is_piston(base_state >> 4) && base_state & EXTENDED != 0 {
            self.drop_block_items(base, base_state);
            self.set_block_notify(base, 0);
        }
    }

    /// `BlockPistonMoving.breakBlock`: a replaced moving block finishes its move.
    pub(super) fn moving_removed(&mut self, pos: (i32, i32, i32)) {
        self.clear_moving(pos);
    }
}

/// `pos.offset(face, n)`.
fn step(pos: (i32, i32, i32), face: u8, n: i32) -> (i32, i32, i32) {
    let mut p = pos;
    for _ in 0..n {
        p = offset(p, face);
    }
    p
}

fn axis_of(face: u8) -> u8 {
    match face {
        DOWN | UP => 1,
        NORTH | SOUTH => 2,
        _ => 0,
    }
}

fn axis(face: u8) -> u8 {
    axis_of(face)
}

/// `func_177255_a`: moves the last `added` entries to just before `index`.
fn reorder(list: &mut Vec<(i32, i32, i32)>, added: usize, index: usize) {
    let len = list.len();
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(&list[..index]);
    out.extend_from_slice(&list[len - added..]);
    out.extend_from_slice(&list[index..len - added]);
    *list = out;
}
