//! Ender chests (vanilla: `BlockEnderChest`, `TileEntityEnderChest`,
//! `InventoryEnderChest`).
//!
//! Every player has one 27-slot ender inventory, stored as `EnderItems` in their player
//! data; any ender chest opens it. The chest block itself only keeps the lid state, and
//! unlike a chest it ticks all the time, telling nearby clients how many players use it.
//!
//! TODO(M4): opening an ender chest while another ender chest's window is open runs
//! vanilla's close/open on the new chest (`setChestTileEntity` comes first); clients
//! cannot do that, so it is not modeled.

use krol_nbt::{Compound, List, Tag, TagType};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack};

use super::Game;
use super::container::{Layout, OpenContainer};
use super::ids::block;
use super::placement::is_normal_cube;

/// Slots in the ender inventory.
pub const ENDER_SLOTS: usize = 27;
/// The block event carrying the number of players using the chest.
const EVENT_VIEWERS: u8 = 1;
/// How often (in ticks) the chest repeats its viewer count.
const SYNC_INTERVAL: i32 = 20;
/// How far the lid moves per tick (fully open is 1).
const LID_STEP: f32 = 0.1;
/// The lid angle below which closing plays the sound.
const LID_CLOSE_SOUND: f32 = 0.5;
/// Volume of the lid sounds.
const LID_VOLUME: f32 = 0.5;

/// What `TileEntityEnderChest` keeps besides its position.
#[derive(Debug, Clone, Default)]
pub struct EnderChestState {
    /// `numPlayersUsing`.
    viewers: i32,
    /// `lidAngle`.
    lid: f32,
    /// `ticksSinceSync`.
    ticks: i32,
}

/// The ender inventory in a player's data (`InventoryEnderChest.loadInventoryFromNBT`).
pub fn ender_items(data: &Compound) -> Vec<Option<ItemStack>> {
    let mut items = vec![None; ENDER_SLOTS];
    if let Some(list) = data.get_list("EnderItems", TagType::Compound) {
        for tag in list.iter() {
            let Tag::Compound(c) = tag else { continue };
            let slot = usize::from(c.get_i8("Slot") as u8);
            if slot < ENDER_SLOTS {
                items[slot] = super::player::item_from_nbt(c);
            }
        }
    }
    items
}

/// Writes the ender inventory back (`saveInventoryToNBT`).
pub fn set_ender_items(data: &mut Compound, items: &[Option<ItemStack>]) {
    if ender_items(data) == items {
        return;
    }
    let mut list = List::new();
    for (slot, item) in items.iter().enumerate() {
        let Some(c) = item.as_ref().and_then(super::player::item_to_nbt) else {
            continue;
        };
        let mut entry = Compound::new();
        entry.insert("Slot", Tag::Byte(slot as u8 as i8));
        for (k, v) in c.iter() {
            entry.insert(k.clone(), v.clone());
        }
        let _ = list.push(Tag::Compound(entry));
    }
    data.insert("EnderItems", Tag::List(list));
}

impl Game {
    /// `BlockEnderChest.onBlockActivated`: opens the player's ender inventory unless a
    /// normal cube sits on the chest. The click is used up either way.
    pub(super) fn activate_ender_chest(&mut self, i: usize, pos: (i32, i32, i32)) {
        let w = &mut self.level.overworld;
        if !w.ender_chests.contains_key(&pos) || is_normal_cube(w.block(pos.0, pos.1 + 1, pos.2)) {
            return;
        }
        if self.players[i].window.open.is_some() {
            self.close_screen(i);
        }
        // displayGUIChest: an InventoryBasic is no IInteractionObject, so the window is
        // a generic "minecraft:container" named by its translation key.
        let title =
            crate::chat::Component::translate("container.enderchest", vec![]).to_json_string();
        let id = self.players[i].window.next_window_id();
        self.players[i].send(ClientboundPlay::OpenWindow {
            window: id as u8,
            kind: "minecraft:container".into(),
            title,
            slots: ENDER_SLOTS as u8,
        });
        // ContainerChest -> InventoryEnderChest.openInventory: the chest counts the
        // player, spectators included.
        self.change_ender_viewers(pos, 1);
        let layout = Layout::Chest((ENDER_SLOTS / 9) as u8);
        self.players[i].window.open = Some(OpenContainer {
            id,
            layout,
            pos,
            slots: Vec::new(),
            parts: Vec::new(),
            ender: true,
            sent: vec![None; layout.len()],
            fields: [0; 4],
            taken: 0,
            cart: None,
        });
        self.open_container_gui(i);
        self.add_stat(i, "stat.enderchestOpened", 1);
    }

    /// `TileEntityEnderChest.openChest` / `closeChest`.
    pub(super) fn change_ender_viewers(&mut self, pos: (i32, i32, i32), delta: i32) {
        let Some(state) = self.level.overworld.ender_chests.get_mut(&pos) else {
            return;
        };
        state.viewers += delta;
        let viewers = state.viewers;
        self.queue_block_event(pos, block::ENDER_CHEST, EVENT_VIEWERS, viewers);
    }

    /// `TileEntityEnderChest.receiveClientEvent` on the server: the event's count
    /// replaces the chest's.
    pub(super) fn ender_chest_event(
        &mut self,
        pos: (i32, i32, i32),
        action: u8,
        param: i32,
    ) -> bool {
        if action != EVENT_VIEWERS {
            return false;
        }
        match self.level.overworld.ender_chests.get_mut(&pos) {
            Some(state) => {
                state.viewers = param;
                true
            }
            None => false,
        }
    }

    /// `TileEntityEnderChest.update`.
    pub(super) fn tick_ender_chest(&mut self, pos: (i32, i32, i32)) {
        let Some(state) = self.level.overworld.ender_chests.get_mut(&pos) else {
            return;
        };
        state.ticks += 1;
        // Vanilla quirk: `++ticksSinceSync % 20 * 4 == 0` is `(ticks % 20) * 4`, so the
        // count goes out every 20 ticks rather than every 80.
        let sync = (state.ticks % SYNC_INTERVAL).wrapping_mul(4) == 0;
        let (viewers, lid) = (state.viewers, state.lid);
        if sync {
            self.queue_block_event(pos, block::ENDER_CHEST, EVENT_VIEWERS, viewers);
        }
        let middle = (
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 0.5,
            f64::from(pos.2) + 0.5,
        );
        if viewers > 0 && lid == 0.0 {
            let pitch = self.world_rand.next_float() * 0.1 + 0.9;
            self.play_sound_near_except(None, "random.chestopen", middle, LID_VOLUME, pitch);
        }
        if (viewers == 0 && lid > 0.0) || (viewers > 0 && lid < 1.0) {
            let before = lid;
            let mut lid = if viewers > 0 {
                lid + LID_STEP
            } else {
                lid - LID_STEP
            };
            if lid > 1.0 {
                lid = 1.0;
            }
            if lid < LID_CLOSE_SOUND && before >= LID_CLOSE_SOUND {
                let pitch = self.world_rand.next_float() * 0.1 + 0.9;
                self.play_sound_near_except(None, "random.chestclosed", middle, LID_VOLUME, pitch);
            }
            if lid < 0.0 {
                lid = 0.0;
            }
            if let Some(state) = self.level.overworld.ender_chests.get_mut(&pos) {
                state.lid = lid;
            }
        }
    }
}
