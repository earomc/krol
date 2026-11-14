//! Right-clicking blocks (vanilla: `ItemInWorldManager.activateBlockOrUseItem`,
//! `Block.onBlockActivated` of doors, trapdoors and fence gates, `ItemDoor`).
//!
//! TODO(M4): container GUIs (chests, furnaces, crafting tables, ...), note blocks,
//! jukeboxes, beds, cake, levers, buttons, repeaters, comparators, daylight sensors,
//! the dragon egg, redstone ore lighting up, cauldrons, flower pots, command blocks,
//! signs; spectators opening containers.

use krol_protocol::packets::play::ClientboundPlay;

use super::Game;
use super::placement::{self, is_normal_cube, offset};

/// Blocks whose `onBlockActivated` returns true without krol modeling what they do yet
/// (the click is used up, nothing is placed).
fn consumes_click(state: u16) -> bool {
    let (id, meta) = (state >> 4, state & 15);
    match id {
        23 | 25 | 26 | 54 | 58 | 61 | 62 | 69 | 77 | 92 | 93 | 94 | 116 | 117 | 118 | 122 | 130
        | 137 | 138 | 143 | 145 | 146 | 149 | 150 | 151 | 154 | 158 | 178 => true,
        // A jukebox only reacts while it holds a record.
        84 => meta & 1 != 0,
        _ => false,
    }
}

/// `EnumFacing.fromAngle`.
fn facing_from_angle(yaw: f32) -> u8 {
    let index = krol_java::math::floor_f64(f64::from(yaw) / 90.0 + 0.5) & 3;
    [
        placement::SOUTH,
        placement::WEST,
        placement::NORTH,
        placement::EAST,
    ][index as usize]
}

fn horizontal_index(face: u8) -> u16 {
    match face {
        placement::SOUTH => 0,
        placement::WEST => 1,
        placement::NORTH => 2,
        _ => 3,
    }
}

fn rotate_y(face: u8) -> u8 {
    match face {
        placement::NORTH => placement::EAST,
        placement::EAST => placement::SOUTH,
        placement::SOUTH => placement::WEST,
        _ => placement::NORTH,
    }
}

/// The door block an item places (`ItemDoor`).
fn door_block(item: i16) -> Option<u16> {
    Some(match item {
        324 => 64,
        330 => 71,
        427 => 193,
        428 => 194,
        429 => 195,
        430 => 196,
        431 => 197,
        _ => return None,
    })
}

impl Game {
    /// `playAuxSFXAtEntity(player, id, pos, 0)`: the effect to everyone else within 64
    /// blocks.
    fn aux_effect_except(&self, i: usize, id: i32, pos: (i32, i32, i32)) {
        let conn = self.players[i].conn_id;
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
            if p.conn_id != conn && d.0 * d.0 + d.1 * d.1 + d.2 * d.2 < 64.0 * 64.0 {
                p.send(packet.clone());
            }
        }
    }

    /// `Block.onBlockActivated` for the clicked block; returns whether the click was used.
    pub(super) fn activate_block(&mut self, i: usize, pos: (i32, i32, i32)) -> bool {
        let state = self.level.overworld.block(pos.0, pos.1, pos.2);
        let (id, meta) = (state >> 4, state & 15);
        match id {
            // Doors: the lower half holds the open bit; iron doors ignore clicks.
            64 | 71 | 193..=197 => {
                if id == 71 {
                    return true;
                }
                let lower = if meta & 8 != 0 {
                    (pos.0, pos.1 - 1, pos.2)
                } else {
                    pos
                };
                let lower_state = self.level.overworld.block(lower.0, lower.1, lower.2);
                if lower_state >> 4 != id {
                    return false;
                }
                let toggled = lower_state ^ 4;
                self.level
                    .overworld
                    .set_block(lower.0, lower.1, lower.2, toggled);
                let open = toggled & 4 != 0;
                self.aux_effect_except(i, if open { 1003 } else { 1006 }, pos);
                true
            }
            96 | 167 => {
                if id == 167 {
                    return true;
                }
                let toggled = state ^ 4;
                self.level.overworld.set_block(pos.0, pos.1, pos.2, toggled);
                self.aux_effect_except(i, if toggled & 4 != 0 { 1003 } else { 1006 }, pos);
                true
            }
            107 | 183..=187 => {
                let new = if meta & 4 != 0 {
                    state & !4
                } else {
                    // Opening swings away from the player.
                    let player = facing_from_angle(self.players[i].rotation.0);
                    let facing = [
                        placement::SOUTH,
                        placement::WEST,
                        placement::NORTH,
                        placement::EAST,
                    ][usize::from(meta & 3)];
                    let mut m = meta;
                    if facing == player ^ 1 {
                        m = (m & !3) | horizontal_index(player);
                    }
                    id << 4 | m | 4
                };
                self.level.overworld.set_block(pos.0, pos.1, pos.2, new);
                self.aux_effect_except(i, if new & 4 != 0 { 1003 } else { 1006 }, pos);
                true
            }
            _ => consumes_click(state),
        }
    }

    /// `ItemDoor.onItemUse`: two halves with the hinge on the side with fewer solid
    /// blocks (or next to another door). Returns whether the item was used.
    pub(super) fn place_door(
        &mut self,
        i: usize,
        item: i16,
        pos: (i32, i32, i32),
        face: u8,
    ) -> bool {
        let Some(block) = door_block(item) else {
            return false;
        };
        if face != placement::UP {
            return false;
        }
        let w = &mut self.level.overworld;
        let clicked = w.block(pos.0, pos.1, pos.2);
        let at = if super::blocks::is_replaceable(clicked) {
            pos
        } else {
            offset(pos, face)
        };
        // BlockDoor.canPlaceBlockAt.
        if at.1 >= 255
            || !placement::solid_top(w.block(at.0, at.1 - 1, at.2))
            || !super::blocks::is_replaceable(w.block(at.0, at.1, at.2))
            || !super::blocks::is_replaceable(w.block(at.0, at.1 + 1, at.2))
        {
            return false;
        }
        let facing = facing_from_angle(self.players[i].rotation.0);
        let right = offset(at, rotate_y(facing));
        let left = offset(at, rotate_y(rotate_y(rotate_y(facing))));
        let solid = |w: &mut super::world::World, p: (i32, i32, i32)| {
            u8::from(is_normal_cube(w.block(p.0, p.1, p.2)))
        };
        let left_solid = solid(w, left) + solid(w, (left.0, left.1 + 1, left.2));
        let right_solid = solid(w, right) + solid(w, (right.0, right.1 + 1, right.2));
        let is_door = |w: &mut super::world::World, p: (i32, i32, i32)| {
            w.block(p.0, p.1, p.2) >> 4 == block || w.block(p.0, p.1 + 1, p.2) >> 4 == block
        };
        let door_left = is_door(w, left);
        let door_right = is_door(w, right);
        let hinge_right = (door_left && !door_right) || right_solid > left_solid;
        let lower = block << 4 | horizontal_index(rotate_y(facing));
        let upper = block << 4 | 8 | u16::from(hinge_right);
        w.set_block(at.0, at.1, at.2, lower);
        w.set_block(at.0, at.1 + 1, at.2, upper);
        let player = &mut self.players[i];
        if player.gamemode != 1 {
            let held = 36 + player.selected;
            if let Some(s) = &mut player.inventory[held] {
                s.count -= 1;
            }
        }
        true
    }
}
