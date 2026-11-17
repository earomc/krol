//! Jukeboxes and records (vanilla: `BlockJukebox`, `BlockJukebox.TileEntityJukebox`,
//! `ItemRecord`): putting a record in, taking it out, the record popping out when the
//! jukebox goes, and the comparator reading which record plays.
//!
//! TODO(M4): `/setblock` data tags for jukeboxes, converting the old numeric `Record`
//! tag when a saved jukebox loads.

use krol_nbt::Tag;
use krol_protocol::packets::play::ItemStack;

use super::Game;
use super::block_update::flags;

/// `jukebox`.
pub const JUKEBOX: u16 = 84;
/// The block metadata bit of a jukebox holding a record (`HAS_RECORD`).
const HAS_RECORD: u16 = 1;
/// `record_13` and `record_wait`, the first and last record.
const FIRST_RECORD: i16 = 2256;
const LAST_RECORD: i16 = 2267;
/// `playAuxSFX` 1005: a record starts playing (data: the record's item id) or stops (0).
const RECORD_EFFECT: i32 = 1005;
/// The ejected record appears in the middle 70% of the block, raised by 0.6.
const SPREAD: f32 = 0.7;
const EJECT_RISE: f64 = 0.6;
/// `EntityItem.setDefaultPickupDelay`.
const PICKUP_DELAY: i32 = 10;

/// Whether `id` is a record (`ItemRecord`).
pub fn is_record(id: i16) -> bool {
    (FIRST_RECORD..=LAST_RECORD).contains(&id)
}

impl Game {
    fn jukebox_record(&self, pos: (i32, i32, i32)) -> Option<ItemStack> {
        let te = self.level.overworld.tile_entity(pos)?;
        super::player::item_from_nbt(te.get_compound("RecordItem")?)
    }

    /// `TileEntityJukebox.setRecord`, with `markDirty` updating the comparators.
    fn set_jukebox_record(&mut self, pos: (i32, i32, i32), record: Option<&ItemStack>) {
        if let Some(te) = self.level.overworld.tile_entity_mut(pos) {
            match record.and_then(super::player::item_to_nbt) {
                Some(c) => {
                    te.insert("RecordItem", Tag::Compound(c));
                }
                None => {
                    te.remove("RecordItem");
                }
            }
        }
        self.update_comparators(pos);
    }

    /// `ItemRecord.onItemUse`: a record goes into an empty jukebox and starts playing.
    /// Returns whether the record was used.
    pub(super) fn insert_record(&mut self, i: usize, pos: (i32, i32, i32)) -> bool {
        let state = self.level.overworld.block(pos.0, pos.1, pos.2);
        if state >> 4 != JUKEBOX || state & HAS_RECORD != 0 {
            return false;
        }
        let held = 36 + self.players[i].selected;
        let Some(stack) = self.players[i].inventory[held].clone() else {
            return false;
        };
        // BlockJukebox.insertRecord keeps the item and its damage, not its tag.
        if self.level.overworld.tile_entity(pos).is_some() {
            let record = ItemStack {
                id: stack.id,
                count: 1,
                damage: stack.damage,
                nbt: None,
            };
            self.set_jukebox_record(pos, Some(&record));
            self.set_block_flags(pos, state | HAS_RECORD, flags::SEND);
        }
        self.block_effect_data(RECORD_EFFECT, pos, i32::from(stack.id));
        let player = &mut self.players[i];
        if player.gamemode != 1
            && let Some(s) = &mut player.inventory[held]
        {
            s.count -= 1;
        }
        self.add_stat(i, "stat.recordPlayed", 1);
        self.count_item_use(i);
        true
    }

    /// `BlockJukebox.onBlockActivated`: a jukebox holding a record gives it back.
    pub(super) fn activate_jukebox(&mut self, pos: (i32, i32, i32), state: u16) -> bool {
        if state & HAS_RECORD == 0 {
            return false;
        }
        self.eject_record(pos);
        self.set_block_flags(pos, state & !HAS_RECORD, flags::SEND);
        true
    }

    /// `BlockJukebox.dropRecord`: the record pops out above the jukebox and the music
    /// stops. Also run when the jukebox goes (`breakBlock`), with the record taken from
    /// the tile entity before it went.
    pub(super) fn eject_record(&mut self, pos: (i32, i32, i32)) {
        let Some(record) = self.jukebox_record(pos) else {
            return;
        };
        self.block_effect_data(RECORD_EFFECT, pos, 0);
        self.set_jukebox_record(pos, None);
        self.spawn_ejected_record(pos, record);
    }

    /// The ejected record's item (not `spawnAsEntity`: the `doTileDrops` rule does not
    /// apply).
    pub(super) fn spawn_ejected_record(&mut self, pos: (i32, i32, i32), record: ItemStack) {
        let r = &mut self.world_rand;
        let x = f64::from(r.next_float() * SPREAD) + f64::from(1.0 - SPREAD) * 0.5;
        let y = f64::from(r.next_float() * SPREAD) + f64::from(1.0 - SPREAD) * 0.2 + EJECT_RISE;
        let z = f64::from(r.next_float() * SPREAD) + f64::from(1.0 - SPREAD) * 0.5;
        let at = (
            f64::from(pos.0) + x,
            f64::from(pos.1) + y,
            f64::from(pos.2) + z,
        );
        self.spawn_item(at, record, PICKUP_DELAY);
    }

    /// The record in a jukebox about to be replaced, for `breakBlock` once it is gone.
    pub(super) fn jukebox_contents(
        &self,
        pos: (i32, i32, i32),
        old: u16,
        new: u16,
    ) -> Option<ItemStack> {
        (old >> 4 == JUKEBOX && new >> 4 != JUKEBOX)
            .then(|| self.jukebox_record(pos))
            .flatten()
    }

    /// `BlockJukebox.breakBlock` after the block went: the effect, the comparators and the
    /// record popping out.
    pub(super) fn jukebox_broken(&mut self, pos: (i32, i32, i32), record: ItemStack) {
        self.block_effect_data(RECORD_EFFECT, pos, 0);
        self.update_comparators(pos);
        self.spawn_ejected_record(pos, record);
    }

    /// `BlockJukebox.getComparatorInputOverride`: which record plays, 1 for the first.
    pub(super) fn jukebox_signal(&self, pos: (i32, i32, i32)) -> i32 {
        self.jukebox_record(pos)
            .map_or(0, |r| i32::from(r.id) + 1 - i32::from(FIRST_RECORD))
    }
}
