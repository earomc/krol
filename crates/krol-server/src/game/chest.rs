//! Chests (vanilla: `BlockChest`, `TileEntityChest`, `InventoryLargeChest`,
//! `ContainerChest`, `EntityPlayerMP.displayGUIChest`, `InventoryHelper`, block events).
//!
//! A chest's items live in its tile entity NBT in the chunk, so every viewer works on
//! the same contents. The runtime state vanilla keeps on the tile entity (players using
//! it, lid angle) is kept here for chests that are open or closing.
//!
//! TODO(M4): ender chests, lock codes, ocelots sitting on chests, trapped chest
//! redstone output, comparators.
//! TODO(M4): vanilla's sync counter starts when the tile entity is loaded; krol starts
//! it when a chest first opens, which shifts the 200-tick viewer recount.

use krol_nbt::{Compound, List, Tag, TagType};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack};

use super::Game;
use super::container::{Layout, OpenContainer};
use super::ids::block;
use super::placement::{self, is_normal_cube, offset};
use super::world::World;

/// Slots in one chest.
pub const CHEST_SLOTS: usize = 27;
/// The block event a chest sends with its number of viewers.
const EVENT_VIEWERS: u8 = 1;
/// How often (in ticks) an open chest recounts the players using it.
const RECOUNT_INTERVAL: i32 = 200;
/// Players within this distance of a chest count in the recount.
const RECOUNT_RANGE: f32 = 5.0;
/// How far the lid moves per tick (fully open is 1).
const LID_STEP: f32 = 0.1;
/// The lid angle below which closing plays the sound.
const LID_CLOSE_SOUND: f32 = 0.5;
/// Volume of the lid sounds.
const LID_VOLUME: f32 = 0.5;

/// Tile entity state of a chest that is open or whose lid is still moving.
#[derive(Debug, Clone)]
pub struct ChestRuntime {
    pos: (i32, i32, i32),
    /// `numPlayersUsing`.
    viewers: i32,
    lid: f32,
    /// `ticksSinceSync`.
    ticks: i32,
}

/// A queued block event (`BlockEventData`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockEvent {
    pos: (i32, i32, i32),
    block: u16,
    action: u8,
    param: i32,
}

/// The 27 slots of a chest's tile entity (`TileEntityChest.readFromNBT`).
pub fn chest_items(world: &World, pos: (i32, i32, i32)) -> Vec<Option<ItemStack>> {
    let mut items = vec![None; CHEST_SLOTS];
    if let Some(list) = world
        .tile_entity(pos)
        .and_then(|c| c.get_list("Items", TagType::Compound))
    {
        for tag in list.iter() {
            let Tag::Compound(c) = tag else { continue };
            let slot = usize::from(c.get_i8("Slot") as u8);
            if slot < CHEST_SLOTS {
                items[slot] = super::player::item_from_nbt(c);
            }
        }
    }
    items
}

/// Writes a chest's slots back (`TileEntityChest.writeToNBT`'s `Items`).
fn set_chest_items(world: &mut World, pos: (i32, i32, i32), items: &[Option<ItemStack>]) {
    if chest_items(world, pos) == items {
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
    if let Some(te) = world.tile_entity_mut(pos) {
        te.insert("Items", Tag::List(list));
    }
}

/// The chest window title (`getDisplayName`): a custom name as text, else the
/// translation key.
fn title(world: &World, parts: &[(i32, i32, i32)]) -> String {
    let custom = parts.iter().find_map(|&p| {
        world
            .tile_entity(p)
            .filter(|c| c.contains_key_of("CustomName", Some(TagType::String)))
            .map(|c| c.get_str("CustomName").to_owned())
            .filter(|n| !n.is_empty())
    });
    let component = match custom {
        Some(name) => crate::chat::Component::text(name),
        None if parts.len() > 1 => {
            crate::chat::Component::translate("container.chestDouble", vec![])
        }
        None => crate::chat::Component::translate("container.chest", vec![]),
    };
    component.to_json_string()
}

impl Game {
    /// The chest at `pos` is blocked by a solid block above (`BlockChest.isBlocked`).
    fn chest_blocked(&mut self, pos: (i32, i32, i32)) -> bool {
        is_normal_cube(self.level.overworld.block(pos.0, pos.1 + 1, pos.2))
    }

    /// `BlockChest.getLockableContainer`: the chest's tile entities, the west/north one
    /// of a double chest first; `None` when it cannot be opened.
    pub(super) fn chest_parts(&mut self, pos: (i32, i32, i32)) -> Option<Vec<(i32, i32, i32)>> {
        let w = &mut self.level.overworld;
        let block = w.block(pos.0, pos.1, pos.2) >> 4;
        w.tile_entity(pos)?;
        if self.chest_blocked(pos) {
            return None;
        }
        let mut parts = vec![pos];
        for face in [
            placement::NORTH,
            placement::EAST,
            placement::SOUTH,
            placement::WEST,
        ] {
            let n = offset(pos, face);
            if self.level.overworld.block(n.0, n.1, n.2) >> 4 != block {
                continue;
            }
            if self.chest_blocked(n) {
                return None;
            }
            if self.level.overworld.tile_entity(n).is_some() {
                let own = parts.clone();
                parts = if face == placement::WEST || face == placement::NORTH {
                    let mut v = vec![n];
                    v.extend(own);
                    v
                } else {
                    let mut v = own;
                    v.push(n);
                    v
                };
            }
        }
        // Placement rules keep chests to pairs; anything else opens as a single chest.
        if parts.len() > 2 {
            parts = vec![pos];
        }
        Some(parts)
    }

    /// The contents of an open chest window's tile entities.
    pub(super) fn chest_contents(&self, parts: &[(i32, i32, i32)]) -> Vec<Option<ItemStack>> {
        parts
            .iter()
            .flat_map(|&p| chest_items(&self.level.overworld, p))
            .collect()
    }

    /// Writes an open chest window's contents back to its tile entities.
    pub(super) fn set_chest_contents(
        &mut self,
        parts: &[(i32, i32, i32)],
        items: &[Option<ItemStack>],
    ) {
        let changed = self.chest_contents(parts) != items;
        for (k, &p) in parts.iter().enumerate() {
            set_chest_items(
                &mut self.level.overworld,
                p,
                &items[k * CHEST_SLOTS..(k + 1) * CHEST_SLOTS],
            );
        }
        // TileEntity.markDirty: comparators reading the chest re-check.
        if changed {
            for &p in parts {
                self.update_comparators(p);
            }
        }
    }

    /// `BlockChest.onBlockActivated`.
    pub(super) fn activate_chest(&mut self, i: usize, pos: (i32, i32, i32), block: u16) {
        let Some(parts) = self.chest_parts(pos) else {
            return;
        };
        self.open_chest(i, pos, parts);
        let stat = if block == block::CHEST {
            "stat.chestOpened"
        } else {
            "stat.trappedChestTriggered"
        };
        self.players[i].stats.add(stat, 1);
    }

    /// `EntityPlayerMP.displayGUIChest` for a chest.
    fn open_chest(&mut self, i: usize, pos: (i32, i32, i32), parts: Vec<(i32, i32, i32)>) {
        if self.players[i].window.open.is_some() {
            self.close_screen(i);
        }
        let rows = (parts.len() * CHEST_SLOTS / 9) as u8;
        let title = title(&self.level.overworld, &parts);
        let id = self.players[i].window.next_window_id();
        self.players[i].send(ClientboundPlay::OpenWindow {
            window: id as u8,
            kind: "minecraft:chest".into(),
            title,
            slots: rows * 9,
        });
        // ContainerChest: the chest counts the player as using it.
        if self.players[i].gamemode != 3 {
            for &p in &parts {
                self.change_chest_viewers(p, 1);
            }
        }
        let layout = Layout::Chest(rows);
        self.players[i].window.open = Some(OpenContainer {
            id,
            layout,
            pos,
            slots: Vec::new(),
            parts,
            sent: vec![None; layout.len()],
            fields: [0; 4],
            taken: 0,
        });
        self.open_container_gui(i);
    }

    /// `TileEntityChest.openInventory` / `closeInventory`.
    pub(super) fn change_chest_viewers(&mut self, pos: (i32, i32, i32), delta: i32) {
        let block = self.level.overworld.block(pos.0, pos.1, pos.2) >> 4;
        let k = match self.chests.iter().position(|c| c.pos == pos) {
            Some(k) => k,
            None => {
                self.chests.push(ChestRuntime {
                    pos,
                    viewers: 0,
                    lid: 0.0,
                    ticks: 0,
                });
                self.chests.len() - 1
            }
        };
        let c = &mut self.chests[k];
        if delta > 0 && c.viewers < 0 {
            c.viewers = 0;
        }
        c.viewers += delta;
        let viewers = c.viewers;
        self.queue_block_event(pos, block, EVENT_VIEWERS, viewers);
    }

    /// `World.addBlockEvent`: identical queued events are not added twice.
    pub(super) fn queue_block_event(
        &mut self,
        pos: (i32, i32, i32),
        block: u16,
        action: u8,
        param: i32,
    ) {
        let event = BlockEvent {
            pos,
            block,
            action,
            param,
        };
        if !self.block_events.contains(&event) {
            self.block_events.push(event);
        }
    }

    /// `WorldServer.sendQueuedBlockEvents`: events whose block is still there are
    /// applied and sent to players within 64 blocks.
    pub(super) fn send_block_events(&mut self) {
        while !self.block_events.is_empty() {
            for e in std::mem::take(&mut self.block_events) {
                let state = self.level.overworld.block(e.pos.0, e.pos.1, e.pos.2);
                if state >> 4 != e.block {
                    continue;
                }
                // onBlockEventReceived: whether clients get the event.
                let send = if super::pistons::is_piston(e.block) {
                    self.piston_event(e.pos, state, e.action, e.param)
                } else if e.block == block::NOTEBLOCK {
                    self.note_event(e.pos, e.action, e.param)
                } else if block::is_chest(e.block) && e.action == EVENT_VIEWERS {
                    // TileEntityChest.receiveClientEvent.
                    let has_te = self.level.overworld.tile_entity(e.pos).is_some();
                    if has_te && let Some(c) = self.chests.iter_mut().find(|c| c.pos == e.pos) {
                        c.viewers = e.param;
                    }
                    has_te
                } else {
                    false
                };
                if !send {
                    continue;
                }
                let packet = ClientboundPlay::BlockAction {
                    pos: e.pos,
                    action: e.action,
                    param: e.param as u8,
                    block: i32::from(e.block),
                };
                for p in &self.players {
                    let d = (
                        f64::from(e.pos.0) - p.pos.0,
                        f64::from(e.pos.1) - p.pos.1,
                        f64::from(e.pos.2) - p.pos.2,
                    );
                    if d.0 * d.0 + d.1 * d.1 + d.2 * d.2 < 64.0 * 64.0 {
                        p.send(packet.clone());
                    }
                }
            }
        }
    }

    /// Whether a chest of the same kind with a tile entity is next to `pos` on `face`
    /// (`TileEntityChest.getAdjacentChest`).
    fn adjacent_chest(&mut self, pos: (i32, i32, i32), block: u16, face: u8) -> bool {
        let n = offset(pos, face);
        let w = &mut self.level.overworld;
        w.block(n.0, n.1, n.2) >> 4 == block && w.tile_entity(n).is_some()
    }

    /// `TileEntityChest.update` for chests in use: the viewer recount every 200 ticks,
    /// the lid, and its sounds (played by the west/north chest of a pair, from the
    /// middle of the pair).
    pub(super) fn tick_chests(&mut self) {
        let mut k = 0;
        while k < self.chests.len() {
            let pos = self.chests[k].pos;
            let id = self.level.overworld.block(pos.0, pos.1, pos.2) >> 4;
            if !block::is_chest(id) || self.level.overworld.tile_entity(pos).is_none() {
                self.chests.remove(k);
                continue;
            }
            let west = self.adjacent_chest(pos, id, placement::WEST);
            let east = self.adjacent_chest(pos, id, placement::EAST);
            let north = self.adjacent_chest(pos, id, placement::NORTH);
            let south = self.adjacent_chest(pos, id, placement::SOUTH);
            let c = &mut self.chests[k];
            c.ticks += 1;
            if c.viewers != 0 && (c.ticks + pos.0 + pos.1 + pos.2) % RECOUNT_INTERVAL == 0 {
                let area = super::physics::Aabb {
                    min: [
                        f64::from(pos.0 as f32 - RECOUNT_RANGE),
                        f64::from(pos.1 as f32 - RECOUNT_RANGE),
                        f64::from(pos.2 as f32 - RECOUNT_RANGE),
                    ],
                    max: [
                        f64::from((pos.0 + 1) as f32 + RECOUNT_RANGE),
                        f64::from((pos.1 + 1) as f32 + RECOUNT_RANGE),
                        f64::from((pos.2 + 1) as f32 + RECOUNT_RANGE),
                    ],
                };
                let count = self
                    .players
                    .iter()
                    .filter(|p| {
                        super::physics::Aabb::at(p.pos, 0.6, 1.8).intersects(&area)
                            && p.window
                                .open
                                .as_ref()
                                .is_some_and(|o| o.parts.contains(&pos))
                    })
                    .count();
                self.chests[k].viewers = count as i32;
            }
            let primary = !north && !west;
            let sound_pos = (
                f64::from(pos.0) + 0.5 + if east { 0.5 } else { 0.0 },
                f64::from(pos.1) + 0.5,
                f64::from(pos.2) + 0.5 + if south { 0.5 } else { 0.0 },
            );
            let c = &self.chests[k];
            let (viewers, lid) = (c.viewers, c.lid);
            if viewers > 0 && lid == 0.0 && primary {
                let pitch = self.world_rand.next_float() * 0.1 + 0.9;
                self.play_sound_near_except(None, "random.chestopen", sound_pos, LID_VOLUME, pitch);
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
                if lid < LID_CLOSE_SOUND && before >= LID_CLOSE_SOUND && primary {
                    let pitch = self.world_rand.next_float() * 0.1 + 0.9;
                    self.play_sound_near_except(
                        None,
                        "random.chestclosed",
                        sound_pos,
                        LID_VOLUME,
                        pitch,
                    );
                }
                if lid < 0.0 {
                    lid = 0.0;
                }
                self.chests[k].lid = lid;
            }
            let c = &self.chests[k];
            if c.viewers <= 0 && c.lid == 0.0 {
                self.chests.remove(k);
            } else {
                k += 1;
            }
        }
    }

    /// `breakBlock` of chests and furnaces (`InventoryHelper.dropInventoryItems`): the
    /// contents spill out in random clumps of 10-30 items.
    pub(super) fn drop_container_contents(&mut self, pos: (i32, i32, i32)) {
        let block = self.level.overworld.block(pos.0, pos.1, pos.2) >> 4;
        let contents: Vec<ItemStack> = match block {
            id if block::is_chest(id) => chest_items(&self.level.overworld, pos)
                .into_iter()
                .flatten()
                .collect(),
            id if block::is_furnace(id) => {
                let items = self.furnace_contents(pos);
                self.furnace_burn.remove(&pos);
                items
            }
            block::HOPPER => self.hopper_contents(pos),
            super::brewing::BREWING_STAND => self.brewing_contents(pos),
            block::DISPENSER | block::DROPPER => self.dispenser_contents(pos),
            _ => return,
        };
        for stack in contents {
            // InventoryHelper keeps its own unseeded Random.
            let r = &mut self.world_rand;
            let fx = r.next_float() * 0.8 + 0.1;
            let fy = r.next_float() * 0.8 + 0.1;
            let fz = r.next_float() * 0.8 + 0.1;
            let mut left = stack.count;
            while left > 0 {
                let mut n = (self.world_rand.next_int_bounded(21) + 10) as i8;
                if n > left {
                    n = left;
                }
                left -= n;
                let r = &mut self.world_rand;
                let motion = (
                    r.next_gaussian() * f64::from(0.05f32),
                    r.next_gaussian() * f64::from(0.05f32) + f64::from(0.2f32),
                    r.next_gaussian() * f64::from(0.05f32),
                );
                let at = (
                    f64::from(pos.0) + f64::from(fx),
                    f64::from(pos.1) + f64::from(fy),
                    f64::from(pos.2) + f64::from(fz),
                );
                let piece = ItemStack {
                    count: n,
                    ..stack.clone()
                };
                self.spawn_item_moving(at, piece, 0, Some(motion));
            }
        }
    }
}
