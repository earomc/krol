//! Inventory window clicks (vanilla: `Container.slotClick`, `mergeItemStack`,
//! `ContainerPlayer`, `NetHandlerPlayServer.processClickWindow` /
//! `processConfirmTransaction` / `processCloseWindow`).
//!
//! Only the player's own window (id 0) exists so far.
//! TODO(M4): crafting results (recipes), other containers (chests, furnaces, crafting
//! tables, ...).

use krol_protocol::packets::play::{ClientboundPlay, ItemStack};

use super::Game;
use super::item_entity::max_stack;

/// Window state per player (`Container` drag fields, `getCanCraft`, the pending
/// transaction map).
#[derive(Debug, Default, Clone)]
pub struct WindowState {
    /// The item on the cursor (`InventoryPlayer.getItemStack`).
    pub cursor: Option<ItemStack>,
    drag_event: i32,
    drag_mode: i32,
    drag_slots: Vec<usize>,
    /// The window refuses clicks until the client confirms a rejected one.
    locked: bool,
    /// The rejected action the client has to confirm (`field_147372_n`).
    pending: Option<(i8, i16)>,
}

/// Armor type of an item (`ItemArmor.armorType`: 0 helmet .. 3 boots).
fn armor_type(id: i16) -> Option<usize> {
    (298..=317).contains(&id).then(|| ((id - 298) % 4) as usize)
}

/// Whether the player window's slot accepts the stack (`Slot.isItemValid`).
fn slot_accepts(slot: usize, stack: &ItemStack) -> bool {
    match slot {
        // The crafting result slot.
        0 => false,
        5..=8 => {
            let k = slot - 5;
            match armor_type(stack.id) {
                Some(t) => t == k,
                // Pumpkins and skulls fit the head.
                None => k == 0 && (stack.id == 86 || stack.id == 397),
            }
        }
        _ => true,
    }
}

/// `Slot.getItemStackLimit`: armor slots hold one item.
fn slot_limit(slot: usize) -> i8 {
    if (5..=8).contains(&slot) { 1 } else { 64 }
}

fn has_subtypes(id: i16) -> bool {
    max_stack(id) != 1
}

/// `ItemStack.isItemEqual` + `areItemStackTagsEqual`.
fn same_kind(a: &ItemStack, b: &ItemStack) -> bool {
    a.id == b.id && a.damage == b.damage && a.nbt == b.nbt
}

/// `Container.canAddItemToSlot`.
fn can_add(slot: &Option<ItemStack>, stack: Option<&ItemStack>, stack_size_matters: bool) -> bool {
    let mut ok = slot.is_none();
    if let (Some(s), Some(st)) = (slot, stack)
        && same_kind(s, st)
    {
        let extra = if stack_size_matters {
            0
        } else {
            i32::from(st.count)
        };
        ok |= i32::from(s.count) + extra <= i32::from(max_stack(st.id));
    }
    ok
}

/// `Container.mergeItemStack` over window slots `[start, end)`.
fn merge(
    slots: &mut [Option<ItemStack>],
    stack: &mut ItemStack,
    start: usize,
    end: usize,
    reverse: bool,
) -> bool {
    let mut moved = false;
    let max = max_stack(stack.id);
    let order: Vec<usize> = if reverse {
        (start..end).rev().collect()
    } else {
        (start..end).collect()
    };
    if max > 1 {
        for &k in &order {
            if stack.count <= 0 {
                break;
            }
            if let Some(existing) = &mut slots[k]
                && existing.id == stack.id
                && (!has_subtypes(stack.id) || existing.damage == stack.damage)
                && existing.nbt == stack.nbt
            {
                let total = i32::from(existing.count) + i32::from(stack.count);
                if total <= i32::from(max) {
                    stack.count = 0;
                    existing.count = total as i8;
                    moved = true;
                } else if existing.count < max {
                    stack.count -= max - existing.count;
                    existing.count = max;
                    moved = true;
                }
            }
        }
    }
    if stack.count > 0 {
        for &k in &order {
            if slots[k].is_none() {
                slots[k] = Some(stack.clone());
                stack.count = 0;
                moved = true;
                break;
            }
        }
    }
    moved
}

/// `ContainerPlayer.transferStackInSlot` (shift-click).
fn transfer(slots: &mut [Option<ItemStack>], index: usize) -> Option<ItemStack> {
    let mut stack = slots[index].clone()?;
    let original = stack.clone();
    let armor_slot = armor_type(stack.id).map(|t| 5 + t);
    let ok = match index {
        0 => merge(slots, &mut stack, 9, 45, true),
        1..=8 => merge(slots, &mut stack, 9, 45, false),
        _ if armor_slot.is_some_and(|a| slots[a].is_none()) => {
            let a = armor_slot.unwrap_or(5);
            merge(slots, &mut stack, a, a + 1, false)
        }
        9..=35 => merge(slots, &mut stack, 36, 45, false),
        _ => merge(slots, &mut stack, 9, 36, false),
    };
    if !ok {
        return None;
    }
    slots[index] = (stack.count > 0).then_some(stack.clone());
    if stack.count == original.count {
        return None;
    }
    Some(original)
}

impl Game {
    /// Vanilla `Container.slotClick` on the player window. Returns the stack vanilla
    /// compares with the client's prediction.
    fn slot_click(&mut self, i: usize, slot: i16, button: i8, mode: i8) -> Option<ItemStack> {
        let creative = self.players[i].gamemode == 1;
        let p = &mut self.players[i];
        let w = &mut p.window;
        let n = p.inventory.len();
        let index = |s: i16| (s >= 0 && (s as usize) < n).then_some(s as usize);
        let mut result = None;
        let button = i32::from(button);
        let mut drops: Vec<ItemStack> = Vec::new();
        if mode == 5 {
            let previous = w.drag_event;
            w.drag_event = button & 3;
            if ((previous != 1 || w.drag_event != 2) && previous != w.drag_event)
                || w.cursor.is_none()
            {
                w.drag_event = 0;
                w.drag_slots.clear();
            } else if w.drag_event == 0 {
                w.drag_mode = (button >> 2) & 3;
                if w.drag_mode == 0 || w.drag_mode == 1 || (w.drag_mode == 2 && creative) {
                    w.drag_event = 1;
                    w.drag_slots.clear();
                } else {
                    w.drag_event = 0;
                    w.drag_slots.clear();
                }
            } else if w.drag_event == 1 {
                if let (Some(k), Some(cursor)) = (index(slot), w.cursor.as_ref())
                    && can_add(&p.inventory[k], Some(cursor), true)
                    && slot_accepts(k, cursor)
                    && i32::from(cursor.count) > w.drag_slots.len() as i32
                    && !w.drag_slots.contains(&k)
                {
                    w.drag_slots.push(k);
                }
            } else if w.drag_event == 2 {
                if !w.drag_slots.is_empty()
                    && let Some(cursor) = w.cursor.clone()
                {
                    let mut left = i32::from(cursor.count);
                    let count = w.drag_slots.len() as i32;
                    for &k in &w.drag_slots.clone() {
                        if can_add(&p.inventory[k], Some(&cursor), true)
                            && slot_accepts(k, &cursor)
                            && i32::from(cursor.count) >= count
                        {
                            let had = p.inventory[k].as_ref().map_or(0, |s| i32::from(s.count));
                            let mut amount = match w.drag_mode {
                                0 => krol_java::math::floor_f32(cursor.count as f32 / count as f32),
                                1 => 1,
                                _ => i32::from(max_stack(cursor.id)),
                            } + had;
                            amount = amount.min(i32::from(max_stack(cursor.id)));
                            amount = amount.min(i32::from(slot_limit(k)));
                            left -= amount - had;
                            p.inventory[k] = Some(ItemStack {
                                count: amount as i8,
                                ..cursor.clone()
                            });
                        }
                    }
                    w.cursor = (left > 0).then(|| ItemStack {
                        count: left as i8,
                        ..cursor
                    });
                }
                w.drag_event = 0;
                w.drag_slots.clear();
            } else {
                w.drag_event = 0;
                w.drag_slots.clear();
            }
        } else if w.drag_event != 0 {
            w.drag_event = 0;
            w.drag_slots.clear();
        } else if (mode == 0 || mode == 1) && (button == 0 || button == 1) {
            if slot == -999 {
                if let Some(cursor) = w.cursor.as_mut() {
                    if button == 0 {
                        drops.push(cursor.clone());
                        w.cursor = None;
                    } else {
                        drops.push(ItemStack {
                            count: 1,
                            ..cursor.clone()
                        });
                        cursor.count -= 1;
                        if cursor.count == 0 {
                            w.cursor = None;
                        }
                    }
                }
            } else if mode == 1 {
                let k = index(slot)?;
                if let Some(moved) = transfer(&mut p.inventory, k) {
                    result = Some(moved.clone());
                    if p.inventory[k].as_ref().is_some_and(|s| s.id == moved.id) {
                        // retrySlotClick.
                        self.slot_click(i, slot, button as i8, 1);
                        return result;
                    }
                }
            } else {
                let k = index(slot)?;
                let in_slot = p.inventory[k].clone();
                let cursor = w.cursor.clone();
                if in_slot.is_some() {
                    result = in_slot.clone();
                }
                match (in_slot, cursor) {
                    (None, Some(mut c)) if slot_accepts(k, &c) => {
                        let amount = if button == 0 { c.count } else { 1 }.min(slot_limit(k));
                        if c.count >= amount {
                            p.inventory[k] = Some(ItemStack {
                                count: amount,
                                ..c.clone()
                            });
                            c.count -= amount;
                        }
                        w.cursor = (c.count != 0).then_some(c);
                    }
                    (None, _) => {}
                    (Some(mut s), None) => {
                        let take = if button == 0 {
                            s.count
                        } else {
                            (s.count + 1) / 2
                        };
                        w.cursor = Some(ItemStack {
                            count: take,
                            ..s.clone()
                        });
                        s.count -= take;
                        p.inventory[k] = (s.count != 0).then_some(s);
                    }
                    (Some(mut s), Some(mut c)) => {
                        if slot_accepts(k, &c) {
                            if s.id == c.id && s.damage == c.damage && s.nbt == c.nbt {
                                let mut amount = if button == 0 { c.count } else { 1 };
                                amount = amount.min(slot_limit(k) - s.count);
                                amount = amount.min(max_stack(c.id) - s.count);
                                c.count -= amount;
                                s.count += amount;
                                p.inventory[k] = Some(s);
                                w.cursor = (c.count != 0).then_some(c);
                            } else if c.count <= slot_limit(k) {
                                p.inventory[k] = Some(c);
                                w.cursor = Some(s);
                            }
                        } else if s.id == c.id
                            && max_stack(c.id) > 1
                            && (!has_subtypes(s.id) || s.damage == c.damage)
                            && s.nbt == c.nbt
                        {
                            let amount = s.count;
                            if amount > 0
                                && i32::from(amount) + i32::from(c.count)
                                    <= i32::from(max_stack(c.id))
                            {
                                c.count += amount;
                                p.inventory[k] = None;
                                w.cursor = Some(c);
                                let _ = &mut s;
                            }
                        }
                    }
                }
            }
        } else if mode == 2 && (0..9).contains(&button) {
            let k = index(slot)?;
            let hotbar = 36 + button as usize;
            let hb = p.inventory[hotbar].clone();
            let in_player_inv = k >= 5;
            let mut fits = hb
                .as_ref()
                .is_none_or(|h| in_player_inv && slot_accepts(k, h));
            let mut empty = None;
            if !fits {
                // getFirstEmptyStack: hotbar first, then the main inventory.
                empty = (0..36)
                    .map(|m| if m < 9 { 36 + m } else { m })
                    .find(|&s| p.inventory[s].is_none());
                fits = empty.is_some();
            }
            if let Some(s) = p.inventory[k].clone().filter(|_| fits) {
                p.inventory[hotbar] = Some(s);
                match hb {
                    Some(h) if !(in_player_inv && slot_accepts(k, &h)) => {
                        if empty.is_some() {
                            let mut h = h;
                            super::item_entity::add_to_inventory(&mut p.inventory, &mut h);
                            p.inventory[k] = None;
                        }
                    }
                    other => p.inventory[k] = other,
                }
            } else if p.inventory[k].is_none()
                && let Some(h) = hb.filter(|h| slot_accepts(k, h))
            {
                p.inventory[hotbar] = None;
                p.inventory[k] = Some(h);
            }
        } else if mode == 3 && creative && w.cursor.is_none() && slot >= 0 {
            if let Some(s) = index(slot).and_then(|k| p.inventory[k].clone()) {
                w.cursor = Some(ItemStack {
                    count: max_stack(s.id),
                    ..s
                });
            }
        } else if mode == 4 && w.cursor.is_none() && slot >= 0 {
            if let Some(k) = index(slot)
                && let Some(mut s) = p.inventory[k].clone()
            {
                let amount = if button == 0 { 1 } else { s.count };
                drops.push(ItemStack {
                    count: amount,
                    ..s.clone()
                });
                s.count -= amount;
                p.inventory[k] = (s.count > 0).then_some(s);
            }
        } else if mode == 6 && slot >= 0 {
            let k = index(slot);
            if let Some(mut c) = w.cursor.clone()
                && k.is_none_or(|k| p.inventory[k].is_none())
            {
                let (start, step) = if button == 0 {
                    (0i32, 1i32)
                } else {
                    (n as i32 - 1, -1)
                };
                for pass in 0..2 {
                    let mut s = start;
                    while s >= 0 && (s as usize) < n && c.count < max_stack(c.id) {
                        let idx = s as usize;
                        // ContainerPlayer.canMergeSlot: never from the crafting result.
                        if idx != 0
                            && let Some(st) = p.inventory[idx].clone()
                            && can_add(&p.inventory[idx], Some(&c), true)
                            && (pass != 0 || st.count != max_stack(st.id))
                        {
                            let take = (max_stack(c.id) - c.count).min(st.count);
                            c.count += take;
                            let rest = st.count - take;
                            p.inventory[idx] = (rest > 0).then(|| ItemStack { count: rest, ..st });
                        }
                        s += step;
                    }
                }
                w.cursor = Some(c);
            }
        }
        for stack in drops {
            self.drop_from_window(i, stack);
        }
        result
    }

    /// `dropPlayerItemWithRandomChoice(stack, _)`: tossed forward, no statistic.
    fn drop_from_window(&mut self, i: usize, stack: ItemStack) {
        self.toss(i, stack, false);
    }

    /// Vanilla `processClickWindow` (the arguments are the packet's fields).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn click_window(
        &mut self,
        i: usize,
        window: i8,
        slot: i16,
        button: i8,
        action: i16,
        mode: i8,
        item: Option<ItemStack>,
    ) {
        let p = &self.players[i];
        if window != 0 || p.window.locked {
            return;
        }
        if p.gamemode == 3 {
            self.resend_window(i);
            return;
        }
        let result = self.slot_click(i, slot, button, mode);
        let p = &mut self.players[i];
        if item == result {
            p.send(ClientboundPlay::ConfirmTransaction {
                window,
                action,
                accepted: true,
            });
            // isChangingQuantityOnly: the client predicted this; only the tracking
            // catches up.
            p.sent_inventory = p.inventory.clone();
        } else {
            p.window.pending = Some((window, action));
            p.send(ClientboundPlay::ConfirmTransaction {
                window,
                action,
                accepted: false,
            });
            p.window.locked = true;
            self.resend_window(i);
        }
    }

    /// `updateCraftingInventory`: the whole window and the cursor.
    fn resend_window(&mut self, i: usize) {
        let p = &self.players[i];
        p.send(ClientboundPlay::WindowItems {
            window: 0,
            items: p.inventory.clone(),
        });
        p.send(ClientboundPlay::SetSlot {
            window: -1,
            slot: -1,
            item: p.window.cursor.clone(),
        });
    }

    /// Vanilla `processConfirmTransaction`: confirming the rejected action unlocks the
    /// window.
    pub(super) fn confirm_transaction(&mut self, i: usize, window: i8, action: i16) {
        let p = &mut self.players[i];
        if p.window.pending == Some((window, action)) && p.window.locked && p.gamemode != 3 {
            p.window.locked = false;
        }
    }

    /// Vanilla `processCloseWindow` (`ContainerPlayer.onContainerClosed`): the cursor
    /// item and the crafting grid are dropped.
    pub(super) fn close_window(&mut self, i: usize) {
        let mut drops = Vec::new();
        let p = &mut self.players[i];
        if let Some(c) = p.window.cursor.take() {
            drops.push(c);
        }
        for k in 1..=4 {
            if let Some(s) = p.inventory[k].take() {
                drops.push(s);
            }
        }
        p.inventory[0] = None;
        for stack in drops {
            self.drop_from_window(i, stack);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i16, count: i8) -> Option<ItemStack> {
        Some(ItemStack {
            id,
            count,
            damage: 0,
            nbt: None,
        })
    }

    #[test]
    fn shift_click_moves_between_main_and_hotbar() {
        let mut slots = vec![None; 45];
        slots[36] = item(1, 64);
        let moved = transfer(&mut slots, 36);
        assert_eq!(moved, item(1, 64));
        assert_eq!(slots[9], item(1, 64));
        assert_eq!(slots[36], None);
        // Armor goes to its slot.
        slots[10] = item(310, 1);
        transfer(&mut slots, 10);
        assert_eq!(slots[5], item(310, 1));
    }
}
