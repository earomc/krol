//! Note blocks (vanilla: `BlockNote`, `TileEntityNote`).

use super::Game;
use super::block_data::{self, Material};
use super::ids::block;

/// Notes run from 0 to 24 (`changePitch` wraps).
const NOTES: i32 = 25;
/// The note that plays at pitch 1.
const MIDDLE_NOTE: i32 = 12;
/// Note sounds are loud (`playSoundEffect(..., 3.0F, ...)`).
const NOTE_VOLUME: f32 = 3.0;
/// Instruments by the block below (`triggerNote`); the event id indexes this list.
const INSTRUMENTS: [&str; 5] = ["harp", "bd", "snare", "hat", "bassattack"];

impl Game {
    fn note_of(&self, pos: (i32, i32, i32)) -> i32 {
        self.level
            .overworld
            .tile_entity(pos)
            .map_or(0, |c| i32::from(c.get_i8("note")).clamp(0, NOTES - 1))
    }

    /// `TileEntityNote.triggerNote`: with air above, queues the note as a block event
    /// with the instrument chosen by the block below.
    fn trigger_note(&mut self, pos: (i32, i32, i32)) {
        let w = &mut self.level.overworld;
        if block_data::material(w.block(pos.0, pos.1 + 1, pos.2) >> 4) != Material::Air {
            return;
        }
        let instrument = match block_data::material(w.block(pos.0, pos.1 - 1, pos.2) >> 4) {
            Material::Rock => 1,
            Material::Sand => 2,
            Material::Glass => 3,
            Material::Wood => 4,
            _ => 0,
        };
        let note = self.note_of(pos);
        self.queue_block_event(pos, block::NOTEBLOCK, instrument, note);
    }

    /// `BlockNote.onBlockActivated`: one note higher, then play it.
    pub(super) fn activate_note_block(&mut self, i: usize, pos: (i32, i32, i32)) {
        if self.level.overworld.tile_entity(pos).is_none() {
            return;
        }
        let note = (self.note_of(pos) + 1) % NOTES;
        if let Some(c) = self.level.overworld.tile_entity_mut(pos) {
            c.insert("note", krol_nbt::Tag::Byte(note as i8));
        }
        self.trigger_note(pos);
        self.players[i].stats.add("stat.noteblockTuned", 1);
    }

    /// `BlockNote.onBlockClicked` (a survival left-click): play the note.
    pub(super) fn click_note_block(&mut self, i: usize, pos: (i32, i32, i32)) {
        if self.level.overworld.tile_entity(pos).is_some() {
            self.trigger_note(pos);
            self.players[i].stats.add("stat.noteblockPlayed", 1);
        }
    }

    /// `BlockNote.onNeighborBlockChange`: plays when power arrives.
    pub(super) fn note_block_changed(&mut self, pos: (i32, i32, i32)) {
        let powered = self.is_block_powered(pos);
        if self.level.overworld.tile_entity(pos).is_none() {
            return;
        }
        let before = self.note_powered.get(&pos).copied().unwrap_or(false);
        if before != powered {
            if powered {
                self.trigger_note(pos);
            }
            self.note_powered.insert(pos, powered);
        }
    }

    /// `BlockNote.onBlockEventReceived`: the sound (the note particle is client-side).
    pub(super) fn note_event(&mut self, pos: (i32, i32, i32), instrument: u8, note: i32) -> bool {
        let pitch = 2f64.powf(f64::from(note - MIDDLE_NOTE) / 12.0) as f32;
        let name = INSTRUMENTS
            .get(usize::from(instrument))
            .copied()
            .unwrap_or(INSTRUMENTS[0]);
        let center = (
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 0.5,
            f64::from(pos.2) + 0.5,
        );
        self.play_sound_near_except(None, &format!("note.{name}"), center, NOTE_VOLUME, pitch);
        true
    }
}
