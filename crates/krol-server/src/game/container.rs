//! Inventory windows (vanilla: `Container.slotClick`, `mergeItemStack`,
//! `ContainerPlayer`, `ContainerWorkbench`, `NetHandlerPlayServer.processClickWindow` /
//! `processConfirmTransaction` / `processCloseWindow`, `EntityPlayerMP.displayGui`).
//!
//! A click works on a flat view of the open window (container slots followed by the
//! player's main inventory and hotbar), which is written back afterwards.
//! TODO(M4): enchanting tables, anvils, beacons and the other containers.

use krol_protocol::packets::play::{ClientboundPlay, ItemStack};

use super::Game;
use super::ids::{block, item};
use super::item_entity::max_stack;

/// A window's slot layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// The player's own window (id 0): result, 2x2 grid, armor, main, hotbar.
    Player,
    /// A crafting table: result, 3x3 grid, main, hotbar.
    Workbench,
    /// A chest with this many rows (3 or 6), then main and hotbar.
    Chest(u8),
    /// A furnace: input, fuel, output, main, hotbar.
    Furnace,
    /// A hopper: five slots, main, hotbar.
    Hopper,
    /// A dispenser or dropper: nine slots, main, hotbar.
    Dispenser,
    /// A brewing stand: three bottles, the ingredient, main, hotbar.
    Brewing,
}

impl Layout {
    pub fn len(self) -> usize {
        match self {
            Self::Player => 45,
            Self::Workbench => 46,
            Self::Chest(rows) => usize::from(rows) * 9 + 36,
            Self::Furnace => 39,
            Self::Hopper => super::hopper::HOPPER_SLOTS + 36,
            Self::Dispenser => super::dispenser::DISPENSER_SLOTS + 36,
            Self::Brewing => super::brewing::SLOTS + 36,
        }
    }

    /// Whether slot 0 is a crafting result.
    fn has_result(self) -> bool {
        !matches!(
            self,
            Self::Chest(_) | Self::Furnace | Self::Hopper | Self::Dispenser | Self::Brewing
        )
    }

    /// The crafting grid's slots and width.
    fn grid(self) -> (std::ops::Range<usize>, usize) {
        match self {
            Self::Player => (1..5, 2),
            Self::Workbench => (1..10, 3),
            Self::Chest(_) | Self::Furnace | Self::Hopper | Self::Dispenser | Self::Brewing => {
                (0..0, 0)
            }
        }
    }

    /// The first of the 27 main inventory slots.
    fn main_start(self) -> usize {
        match self {
            Self::Player => 9,
            Self::Workbench => 10,
            Self::Chest(rows) => usize::from(rows) * 9,
            Self::Furnace => 3,
            Self::Hopper => super::hopper::HOPPER_SLOTS,
            Self::Dispenser => super::dispenser::DISPENSER_SLOTS,
            Self::Brewing => super::brewing::SLOTS,
        }
    }

    /// The window slot of main inventory index `m` (0..9 hotbar, 9..36 main).
    fn player_slot(self, m: usize) -> usize {
        if m < 9 {
            self.main_start() + 27 + m
        } else {
            self.main_start() + m - 9
        }
    }

    /// Whether the slot belongs to the player's inventory (`slot.inventory == player`).
    fn in_player_inventory(self, k: usize) -> bool {
        match self {
            Self::Player => k >= 5,
            Self::Workbench => k >= 10,
            Self::Chest(rows) => k >= usize::from(rows) * 9,
            Self::Furnace => k >= 3,
            Self::Hopper => k >= super::hopper::HOPPER_SLOTS,
            Self::Dispenser => k >= super::dispenser::DISPENSER_SLOTS,
            Self::Brewing => k >= super::brewing::SLOTS,
        }
    }

    /// `Slot.isItemValid`.
    fn accepts(self, k: usize, stack: &ItemStack) -> bool {
        if k == 0 && self.has_result() {
            return false; // crafting results
        }
        // ContainerBrewingStand.Potion / Ingredient.isItemValid.
        if self == Self::Brewing {
            return match k {
                0..=2 => super::brewing::fits_bottle_slot(stack),
                3 => super::brewing::is_ingredient(stack),
                _ => true,
            };
        }
        if self == Self::Furnace {
            return match k {
                1 => super::furnace::fits_fuel_slot(stack),
                2 => false,
                _ => true,
            };
        }
        if self == Self::Player && (5..=8).contains(&k) {
            let slot = k - 5;
            return match armor_type(stack.id) {
                Some(t) => t == slot,
                // Pumpkins and skulls fit the head.
                None => slot == 0 && (stack.id == 86 || stack.id == 397),
            };
        }
        true
    }

    /// `Slot.getItemStackLimit`: armor slots hold one item, a furnace's fuel slot one
    /// bucket.
    fn limit(self, k: usize, stack: &ItemStack) -> i8 {
        if (self == Self::Player && (5..=8).contains(&k))
            || (self == Self::Brewing && k < 3)
            || (self == Self::Furnace && k == super::furnace::FUEL && stack.id == item::BUCKET)
        {
            1
        } else {
            64
        }
    }

    /// `transferStackInSlot` (shift-click).
    fn transfer(self, slots: &mut [Option<ItemStack>], index: usize) -> Option<ItemStack> {
        let mut stack = slots[index].clone()?;
        let original = stack.clone();
        let ok = match self {
            Self::Player => {
                let armor_slot = armor_type(stack.id).map(|t| 5 + t);
                match index {
                    0 => merge(slots, &mut stack, 9, 45, true),
                    1..=8 => merge(slots, &mut stack, 9, 45, false),
                    _ if armor_slot.is_some_and(|a| slots[a].is_none()) => {
                        let a = armor_slot.unwrap_or(5);
                        merge(slots, &mut stack, a, a + 1, false)
                    }
                    9..=35 => merge(slots, &mut stack, 36, 45, false),
                    _ => merge(slots, &mut stack, 9, 36, false),
                }
            }
            Self::Workbench => match index {
                0 => merge(slots, &mut stack, 10, 46, true),
                10..=36 => merge(slots, &mut stack, 37, 46, false),
                37..=45 => merge(slots, &mut stack, 10, 37, false),
                _ => merge(slots, &mut stack, 10, 46, false),
            },
            // ContainerHopper / ContainerDispenser.transferStackInSlot.
            Self::Hopper | Self::Dispenser => {
                let own = if self == Self::Hopper {
                    super::hopper::HOPPER_SLOTS
                } else {
                    super::dispenser::DISPENSER_SLOTS
                };
                if index < own {
                    merge(slots, &mut stack, own, own + 36, true)
                } else {
                    merge(slots, &mut stack, 0, own, false)
                }
            }
            Self::Furnace => match index {
                2 => merge(slots, &mut stack, 3, 39, true),
                0 | 1 => merge(slots, &mut stack, 3, 39, false),
                _ if super::recipes::smelting(&stack).is_some() => {
                    merge(slots, &mut stack, 0, 1, false)
                }
                _ if super::furnace::burn_time(Some(&stack)) > 0 => {
                    merge(slots, &mut stack, 1, 2, false)
                }
                3..=29 => merge(slots, &mut stack, 30, 39, false),
                _ => merge(slots, &mut stack, 3, 30, false),
            },
            // ContainerBrewingStand.transferStackInSlot.
            Self::Brewing => match index {
                0..=3 => merge(slots, &mut stack, 4, 40, true),
                _ if slots[3].is_none() && super::brewing::is_ingredient(&stack) => {
                    merge(slots, &mut stack, 3, 4, false)
                }
                _ if super::brewing::fits_bottle_slot(&stack) => {
                    merge(slots, &mut stack, 0, 3, false)
                }
                4..=30 => merge(slots, &mut stack, 31, 40, false),
                31..=39 => merge(slots, &mut stack, 4, 31, false),
                _ => merge(slots, &mut stack, 4, 40, false),
            },
            Self::Chest(rows) => {
                let own = usize::from(rows) * 9;
                if index < own {
                    merge(slots, &mut stack, own, own + 36, true)
                } else {
                    merge(slots, &mut stack, 0, own, false)
                }
            }
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
}

/// An open container other than the player's own window.
#[derive(Debug, Clone)]
pub struct OpenContainer {
    /// Window id.
    pub id: i8,
    /// Layout.
    pub layout: Layout,
    /// The block it belongs to.
    pub pos: (i32, i32, i32),
    /// The container's own slots (for a workbench: result and grid; empty for chests,
    /// whose items stay in their tile entities).
    pub slots: Vec<Option<ItemStack>>,
    /// The tile entities whose inventories the window shows (chests, furnaces).
    pub parts: Vec<(i32, i32, i32)>,
    /// Window properties last sent (furnaces).
    pub fields: [i32; 4],
    /// Items taken from a furnace's output not yet counted (`SlotFurnaceOutput`).
    pub taken: i32,
    /// What the client was last told (`inventoryItemStacks`), for the whole window.
    pub sent: Vec<Option<ItemStack>>,
}

/// Window state per player (`Container` drag fields, `getCanCraft`, the pending
/// transaction map, `openContainer`, `currentWindowId`).
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
    /// The open container, if not the player's own window.
    pub open: Option<OpenContainer>,
    /// The last window id handed out.
    window_id: i32,
    /// The click put a stack into a furnace's input slot (`putStack` on slot 0).
    put_input: bool,
}

impl WindowState {
    /// `EntityPlayerMP.getNextWindowId`.
    pub fn next_window_id(&mut self) -> i8 {
        self.window_id = self.window_id % 100 + 1;
        self.window_id as i8
    }
}

/// Armor type of an item (`ItemArmor.armorType`: 0 helmet .. 3 boots).
fn armor_type(id: i16) -> Option<usize> {
    (298..=317).contains(&id).then(|| ((id - 298) % 4) as usize)
}

fn has_subtypes(id: i16) -> bool {
    super::item_data::has_subtypes(id)
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

impl Game {
    /// The open window's id and the window slot of the held hotbar slot
    /// (`getSlotFromInventory`).
    pub(super) fn held_window_slot(&self, i: usize) -> (i8, usize) {
        let (window, layout) = self.current_window(i);
        (window, layout.player_slot(self.players[i].selected))
    }

    /// `detectAndSendChanges` with `isChangingQuantityOnly`: the tracking catches up
    /// with the window without sending anything.
    pub(super) fn sync_window_silently(&mut self, i: usize) {
        let flat = self.gather(i);
        let p = &mut self.players[i];
        match &mut p.window.open {
            Some(o) => o.sent = flat,
            None => p.sent_inventory = flat,
        }
    }

    /// The open window's id and layout.
    fn current_window(&self, i: usize) -> (i8, Layout) {
        match &self.players[i].window.open {
            Some(o) => (o.id, o.layout),
            None => (0, Layout::Player),
        }
    }

    /// The open window as one slot list.
    fn gather(&self, i: usize) -> Vec<Option<ItemStack>> {
        let p = &self.players[i];
        match &p.window.open {
            None => p.inventory.clone(),
            Some(o) if o.layout == Layout::Furnace => {
                let mut flat = self.furnace_items(o.pos);
                flat.extend(p.inventory[9..45].iter().cloned());
                flat
            }
            Some(o) if o.layout == Layout::Hopper => {
                let mut flat = self.hopper_items(o.pos);
                flat.extend(p.inventory[9..45].iter().cloned());
                flat
            }
            Some(o) if o.layout == Layout::Brewing => {
                let mut flat = self.brewing_items(o.pos);
                flat.extend(p.inventory[9..45].iter().cloned());
                flat
            }
            Some(o) if o.layout == Layout::Dispenser => {
                let mut flat = self.dispenser_items(o.pos);
                flat.extend(p.inventory[9..45].iter().cloned());
                flat
            }
            Some(o) if !o.parts.is_empty() => {
                let mut flat = self.chest_contents(&o.parts);
                flat.extend(p.inventory[9..45].iter().cloned());
                flat
            }
            Some(o) => {
                let mut flat = o.slots.clone();
                flat.extend(p.inventory[9..45].iter().cloned());
                flat
            }
        }
    }

    /// Writes a slot list back into the open window.
    fn scatter(&mut self, i: usize, flat: Vec<Option<ItemStack>>) {
        let put_input = std::mem::take(&mut self.players[i].window.put_input);
        if let Some(pos) = self.players[i]
            .window
            .open
            .as_ref()
            .filter(|o| o.layout == Layout::Furnace)
            .map(|o| o.pos)
        {
            self.set_furnace_items(pos, &flat[..3], put_input);
            self.players[i].inventory[9..45].clone_from_slice(&flat[3..]);
            return;
        }
        if let Some(pos) = self.players[i]
            .window
            .open
            .as_ref()
            .filter(|o| o.layout == Layout::Hopper)
            .map(|o| o.pos)
        {
            let own = super::hopper::HOPPER_SLOTS;
            self.set_hopper_items(pos, &flat[..own]);
            self.players[i].inventory[9..45].clone_from_slice(&flat[own..]);
            return;
        }
        if let Some(pos) = self.players[i]
            .window
            .open
            .as_ref()
            .filter(|o| o.layout == Layout::Brewing)
            .map(|o| o.pos)
        {
            let own = super::brewing::SLOTS;
            self.set_brewing_items(pos, &flat[..own]);
            self.players[i].inventory[9..45].clone_from_slice(&flat[own..]);
            return;
        }
        if let Some(pos) = self.players[i]
            .window
            .open
            .as_ref()
            .filter(|o| o.layout == Layout::Dispenser)
            .map(|o| o.pos)
        {
            let own = super::dispenser::DISPENSER_SLOTS;
            self.set_dispenser_items(pos, &flat[..own]);
            self.players[i].inventory[9..45].clone_from_slice(&flat[own..]);
            return;
        }
        if let Some(parts) = self.players[i]
            .window
            .open
            .as_ref()
            .map(|o| o.parts.clone())
            .filter(|parts| !parts.is_empty())
        {
            let own = parts.len() * 27;
            self.set_chest_contents(&parts, &flat[..own]);
            self.players[i].inventory[9..45].clone_from_slice(&flat[own..]);
            return;
        }
        let p = &mut self.players[i];
        match &mut p.window.open {
            None => p.inventory = flat,
            Some(o) => {
                let own = o.slots.len();
                o.slots = flat[..own].to_vec();
                p.inventory[9..45].clone_from_slice(&flat[own..]);
            }
        }
    }

    /// Vanilla `Container.slotClick`. Returns the stack vanilla compares with the
    /// client's prediction.
    fn slot_click(&mut self, i: usize, slot: i16, button: i8, mode: i8) -> Option<ItemStack> {
        let creative = self.players[i].gamemode == 1;
        let (_, layout) = self.current_window(i);
        let mut flat = self.gather(i);
        let result_before = flat[0].clone();
        let furnace_before =
            (layout == Layout::Furnace).then(|| (flat[0].clone(), flat[2].clone()));
        let n = layout.len();
        let index = |s: i16| (s >= 0 && (s as usize) < n).then_some(s as usize);
        let mut result = None;
        let button = i32::from(button);
        let mut drops: Vec<ItemStack> = Vec::new();
        let w = &mut self.players[i].window;
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
                    && can_add(&flat[k], Some(cursor), true)
                    && layout.accepts(k, cursor)
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
                        if can_add(&flat[k], Some(&cursor), true)
                            && layout.accepts(k, &cursor)
                            && i32::from(cursor.count) >= count
                        {
                            let had = flat[k].as_ref().map_or(0, |s| i32::from(s.count));
                            let mut amount = match w.drag_mode {
                                0 => krol_java::math::floor_f32(cursor.count as f32 / count as f32),
                                1 => 1,
                                _ => i32::from(max_stack(cursor.id)),
                            } + had;
                            amount = amount.min(i32::from(max_stack(cursor.id)));
                            amount = amount.min(i32::from(layout.limit(k, &cursor)));
                            left -= amount - had;
                            flat[k] = Some(ItemStack {
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
                if let Some(moved) = layout.transfer(&mut flat, k) {
                    result = Some(moved.clone());
                    let after = (flat[0].clone(), flat.get(2).cloned().flatten());
                    self.furnace_click(i, &furnace_before, &after, mode, k);
                    self.scatter(i, flat);
                    self.furnace_output_click(i, &furnace_before, mode);
                    // The crafting result is consumed before the retry check.
                    self.result_taken(i, &result_before);
                    let again = self.gather(i);
                    if again[k].as_ref().is_some_and(|s| s.id == moved.id) {
                        // retrySlotClick.
                        self.slot_click(i, slot, button as i8, 1);
                    }
                    return result;
                }
            } else {
                let k = index(slot)?;
                let in_slot = flat[k].clone();
                let cursor = w.cursor.clone();
                if in_slot.is_some() {
                    result = in_slot.clone();
                }
                match (in_slot, cursor) {
                    (None, Some(mut c)) if layout.accepts(k, &c) => {
                        let amount = if button == 0 { c.count } else { 1 }.min(layout.limit(k, &c));
                        if c.count >= amount {
                            flat[k] = Some(ItemStack {
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
                        flat[k] = (s.count != 0).then_some(s);
                    }
                    (Some(mut s), Some(mut c)) => {
                        if layout.accepts(k, &c) {
                            if s.id == c.id && s.damage == c.damage && s.nbt == c.nbt {
                                let mut amount = if button == 0 { c.count } else { 1 };
                                amount = amount.min(layout.limit(k, &c) - s.count);
                                amount = amount.min(max_stack(c.id) - s.count);
                                c.count -= amount;
                                s.count += amount;
                                flat[k] = Some(s);
                                w.cursor = (c.count != 0).then_some(c);
                            } else if c.count <= layout.limit(k, &c) {
                                flat[k] = Some(c);
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
                                flat[k] = None;
                                w.cursor = Some(c);
                                let _ = &mut s;
                            }
                        }
                    }
                }
            }
        } else if mode == 2 && (0..9).contains(&button) {
            let k = index(slot)?;
            let hotbar = layout.player_slot(button as usize);
            let hb = flat[hotbar].clone();
            let in_player_inv = layout.in_player_inventory(k);
            let mut fits = hb
                .as_ref()
                .is_none_or(|h| in_player_inv && layout.accepts(k, h));
            let mut empty = None;
            if !fits {
                // getFirstEmptyStack: hotbar first, then the main inventory.
                empty = (0..36)
                    .map(|m| layout.player_slot(m))
                    .find(|&s| flat[s].is_none());
                fits = empty.is_some();
            }
            if let Some(s) = flat[k].clone().filter(|_| fits) {
                flat[hotbar] = Some(s);
                match hb {
                    Some(h) if !(in_player_inv && layout.accepts(k, &h)) => {
                        if empty.is_some() {
                            let mut h = h;
                            super::item_entity::add_to_inventory_mapped(&mut flat, &mut h, |m| {
                                layout.player_slot(m)
                            });
                            flat[k] = None;
                        }
                    }
                    other => flat[k] = other,
                }
            } else if flat[k].is_none()
                && let Some(h) = hb.filter(|h| layout.accepts(k, h))
            {
                flat[hotbar] = None;
                flat[k] = Some(h);
            }
        } else if mode == 3 && creative && w.cursor.is_none() && slot >= 0 {
            if let Some(s) = index(slot).and_then(|k| flat[k].clone()) {
                w.cursor = Some(ItemStack {
                    count: max_stack(s.id),
                    ..s
                });
            }
        } else if mode == 4 && w.cursor.is_none() && slot >= 0 {
            if let Some(k) = index(slot)
                && let Some(mut s) = flat[k].clone()
            {
                let amount = if button == 0 { 1 } else { s.count };
                drops.push(ItemStack {
                    count: amount,
                    ..s.clone()
                });
                s.count -= amount;
                flat[k] = (s.count > 0).then_some(s);
            }
        } else if mode == 6 && slot >= 0 {
            let k = index(slot);
            if let Some(mut c) = w.cursor.clone()
                && k.is_none_or(|k| flat[k].is_none())
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
                        // canMergeSlot: never from a crafting result.
                        if (idx != 0 || !layout.has_result())
                            && let Some(st) = flat[idx].clone()
                            && can_add(&flat[idx], Some(&c), true)
                            && (pass != 0 || st.count != max_stack(st.id))
                        {
                            let take = (max_stack(c.id) - c.count).min(st.count);
                            c.count += take;
                            let rest = st.count - take;
                            flat[idx] = (rest > 0).then(|| ItemStack { count: rest, ..st });
                        }
                        s += step;
                    }
                }
                w.cursor = Some(c);
            }
        }
        let after = (flat[0].clone(), flat.get(2).cloned().flatten());
        let k = index(slot).unwrap_or(usize::MAX);
        self.furnace_click(i, &furnace_before, &after, mode, k);
        self.scatter(i, flat);
        self.furnace_output_click(i, &furnace_before, mode);
        for stack in drops {
            self.drop_from_window(i, stack);
        }
        self.result_taken(i, &result_before);
        result
    }

    /// Whether a click on a furnace window put a stack into the input slot
    /// (`putStack(0, ...)`): placing into an empty slot or swapping, a shift-click
    /// emptying it, or a number key swap on it.
    fn furnace_click(
        &mut self,
        i: usize,
        before: &Option<(Option<ItemStack>, Option<ItemStack>)>,
        after: &(Option<ItemStack>, Option<ItemStack>),
        mode: i8,
        k: usize,
    ) {
        let Some((input, _)) = before else {
            return;
        };
        let new_kind = match (&after.0, input) {
            (Some(a), Some(b)) => !same_kind(a, b),
            (Some(_), None) => true,
            _ => false,
        };
        let put = new_kind
            || (mode == 1 && k == 0 && after.0.is_none() && input.is_some())
            || (mode == 2 && k == 0 && after.0 != *input);
        self.players[i].window.put_input = put;
    }

    /// Items taken from a furnace's output slot by a click.
    fn furnace_output_click(
        &mut self,
        i: usize,
        before: &Option<(Option<ItemStack>, Option<ItemStack>)>,
        mode: i8,
    ) {
        let Some((_, Some(out))) = before else {
            return;
        };
        let now = self.gather(i)[2].clone();
        let left = now.filter(|s| s.id == out.id).map_or(0, |s| s.count);
        let amount = i32::from(out.count - left);
        if amount > 0 {
            self.furnace_output_taken(i, out, amount, mode != 6);
        }
    }

    /// `SlotCrafting.onPickupFromSlot` when the result was taken (counting the crafted
    /// amount, using up one of each ingredient, leaving containers such as buckets),
    /// then `onCraftMatrixChanged`.
    fn result_taken(&mut self, i: usize, before: &Option<ItemStack>) {
        let (_, layout) = self.current_window(i);
        if !layout.has_result() {
            return;
        }
        let (grid, _) = layout.grid();
        let mut flat = self.gather(i);
        let mut extra = Vec::new();
        if let Some(b) = before
            && flat[0].as_ref() != Some(b)
        {
            let left = flat[0]
                .as_ref()
                .filter(|s| s.id == b.id)
                .map_or(0, |s| s.count);
            let amount = i32::from(b.count - left);
            if amount > 0
                && let Some(name) = krol_world::items::name_for_id(b.id)
            {
                self.players[i].stats.add(
                    &format!("stat.craftItem.{}", name.replace(':', ".")),
                    amount,
                );
            }
            // TODO(M5): crafting achievements (buildWorkBench, buildPickaxe, ...).
            let width = layout.grid().1;
            let scale = |map: i16| self.map_scale(map);
            let remaining = super::recipes::remaining_items(&flat[grid.clone()], width, &scale);
            for (k, leftover) in grid.zip(remaining) {
                if let Some(s) = &mut flat[k] {
                    s.count -= 1;
                    if s.count <= 0 {
                        flat[k] = None;
                    }
                }
                if let Some(c) = leftover {
                    if flat[k].is_none() {
                        flat[k] = Some(c);
                    } else {
                        extra.push(c);
                    }
                }
            }
        }
        self.scatter(i, flat);
        for mut c in extra {
            if !super::item_entity::add_to_inventory(&mut self.players[i].inventory, &mut c) {
                self.drop_from_window(i, c);
            }
        }
        self.update_craft_result(i);
    }

    /// `onCraftMatrixChanged`: the result slot follows the grid.
    pub(super) fn update_craft_result(&mut self, i: usize) {
        let (_, layout) = self.current_window(i);
        if !layout.has_result() {
            return;
        }
        let (grid, width) = layout.grid();
        let mut flat = self.gather(i);
        let scale = |map: i16| self.map_scale(map);
        flat[0] = super::recipes::craft(&flat[grid], width, &scale);
        self.scatter(i, flat);
    }

    /// A map's saved scale (`data/map_<id>.dat`, `MapStorage.loadData`), if it exists.
    fn map_scale(&self, map: i16) -> Option<i8> {
        let path = self.level.dir.join("data").join(format!("map_{map}.dat"));
        let file = std::fs::File::open(path).ok()?;
        let (_, root) = krol_nbt::read_gzip(std::io::BufReader::new(file)).ok()?;
        Some(root.get_compound("data")?.get_i8("scale"))
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
        let (current, _) = self.current_window(i);
        let p = &self.players[i];
        if window != current || p.window.locked {
            return;
        }
        if p.gamemode == 3 {
            self.resend_window(i);
            return;
        }
        let result = self.slot_click(i, slot, button, mode);
        if item == result {
            self.players[i].send(ClientboundPlay::ConfirmTransaction {
                window,
                action,
                accepted: true,
            });
            // isChangingQuantityOnly: the client predicted this; only the tracking
            // catches up.
            let flat = self.gather(i);
            let p = &mut self.players[i];
            match &mut p.window.open {
                Some(o) => o.sent = flat,
                None => p.sent_inventory = flat,
            }
            // Window properties are still sent.
            self.send_furnace_fields(i);
            self.send_brewing_fields(i);
        } else {
            let p = &mut self.players[i];
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
        let (window, _) = self.current_window(i);
        let items = self.gather(i);
        let p = &self.players[i];
        p.send(ClientboundPlay::WindowItems {
            window: window as u8,
            items,
        });
        p.send(ClientboundPlay::SetSlot {
            window: -1,
            slot: -1,
            item: p.window.cursor.clone(),
        });
    }

    /// `detectAndSendChanges` for an open container (`None` if the player window is
    /// open). The crafting result is never sent.
    pub(super) fn detect_container_changes(&mut self, i: usize) -> Option<()> {
        self.players[i].window.open.as_ref()?;
        let flat = self.gather(i);
        let p = &mut self.players[i];
        let o = p.window.open.as_mut()?;
        let skip_result = o.layout.has_result();
        let mut packets = Vec::new();
        for (k, item) in flat.into_iter().enumerate() {
            if o.sent[k] != item {
                o.sent[k] = item.clone();
                if k != 0 || !skip_result {
                    packets.push(ClientboundPlay::SetSlot {
                        window: o.id,
                        slot: k as i16,
                        item,
                    });
                }
            }
        }
        for packet in packets {
            p.send(packet);
        }
        self.send_furnace_fields(i);
        self.send_brewing_fields(i);
        Some(())
    }

    /// Vanilla `processConfirmTransaction`: confirming the rejected action unlocks the
    /// window.
    pub(super) fn confirm_transaction(&mut self, i: usize, window: i8, action: i16) {
        let p = &mut self.players[i];
        if p.window.pending == Some((window, action)) && p.window.locked && p.gamemode != 3 {
            p.window.locked = false;
        }
    }

    /// Vanilla `closeContainer` (`onContainerClosed`): the cursor item and the crafting
    /// grid are dropped, and the player window is open again.
    pub(super) fn close_window(&mut self, i: usize) {
        let mut drops = Vec::new();
        let (_, layout) = self.current_window(i);
        let (grid, _) = layout.grid();
        let mut flat = self.gather(i);
        if let Some(c) = self.players[i].window.cursor.take() {
            drops.push(c);
        }
        for k in grid {
            if let Some(s) = flat[k].take() {
                drops.push(s);
            }
        }
        if layout.has_result() {
            flat[0] = None;
        }
        self.scatter(i, flat);
        let closed = self.players[i].window.open.take();
        self.players[i].window.locked = false;
        // ContainerChest.onContainerClosed: the chests lose a viewer.
        if let Some(o) = closed
            && matches!(o.layout, Layout::Chest(_))
            && self.players[i].gamemode != 3
        {
            for p in o.parts {
                self.change_chest_viewers(p, -1);
            }
        }
        for stack in drops {
            self.drop_from_window(i, stack);
        }
    }

    /// `EntityPlayerMP.closeScreen`: the server closes the window.
    pub(super) fn close_screen(&mut self, i: usize) {
        if let Some(o) = &self.players[i].window.open {
            let id = o.id;
            self.players[i].send(ClientboundPlay::CloseWindow { window: id as u8 });
        }
        self.close_window(i);
    }

    /// `EntityPlayerMP.displayGui` for a crafting table.
    pub(super) fn open_workbench(&mut self, i: usize, pos: (i32, i32, i32)) {
        let p = &mut self.players[i];
        let id = p.window.next_window_id();
        let title =
            crate::chat::Component::translate("tile.workbench.name", vec![]).to_json_string();
        p.send(ClientboundPlay::OpenWindow {
            window: id as u8,
            kind: "minecraft:crafting_table".into(),
            title,
            slots: 0,
        });
        p.window.open = Some(OpenContainer {
            id,
            layout: Layout::Workbench,
            pos,
            slots: vec![None; 10],
            parts: Vec::new(),
            sent: vec![None; Layout::Workbench.len()],
            fields: [0; 4],
            taken: 0,
        });
        p.stats.add("stat.craftingTableInteraction", 1);
        self.open_container_gui(i);
    }

    /// `onCraftGuiOpened`: the whole window, the cursor, then the first change check.
    pub(super) fn open_container_gui(&mut self, i: usize) {
        self.resend_window(i);
        self.detect_container_changes(i);
    }

    /// `Container.canInteractWith`, checked every tick: the block must still be there and
    /// within 8 blocks.
    pub(super) fn check_open_container(&mut self, i: usize) {
        let Some(o) = &self.players[i].window.open else {
            return;
        };
        let (pos, layout) = (o.pos, o.layout);
        let parts = o.parts.clone();
        let player = self.players[i].pos;
        let near = |pos: (i32, i32, i32)| {
            let d = (
                f64::from(pos.0) + 0.5 - player.0,
                f64::from(pos.1) + 0.5 - player.1,
                f64::from(pos.2) + 0.5 - player.2,
            );
            d.0 * d.0 + d.1 * d.1 + d.2 * d.2 <= 64.0
        };
        let w = &mut self.level.overworld;
        let still = match layout {
            Layout::Workbench => {
                w.block(pos.0, pos.1, pos.2) >> 4 == block::CRAFTING_TABLE && near(pos)
            }
            // TileEntityChest.isUseableByPlayer, for each half.
            Layout::Chest(_) => parts.iter().all(|&p| {
                block::is_chest(w.block(p.0, p.1, p.2) >> 4)
                    && w.tile_entity(p).is_some()
                    && near(p)
            }),
            // TileEntityFurnace.isUseableByPlayer.
            Layout::Furnace => {
                block::is_furnace(w.block(pos.0, pos.1, pos.2) >> 4)
                    && w.tile_entity(pos).is_some()
                    && near(pos)
            }
            // TileEntityDispenser.isUseableByPlayer.
            Layout::Dispenser => {
                super::dispenser::is_dispenser(w.block(pos.0, pos.1, pos.2) >> 4)
                    && w.tile_entity(pos).is_some()
                    && near(pos)
            }
            // TileEntityBrewingStand.isUseableByPlayer.
            Layout::Brewing => {
                w.block(pos.0, pos.1, pos.2) >> 4 == super::brewing::BREWING_STAND
                    && w.tile_entity(pos).is_some()
                    && near(pos)
            }
            // TileEntityHopper.isUseableByPlayer.
            Layout::Hopper => {
                w.block(pos.0, pos.1, pos.2) >> 4 == block::HOPPER
                    && w.tile_entity(pos).is_some()
                    && near(pos)
            }
            Layout::Player => true,
        };
        if !still {
            self.close_screen(i);
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
        let moved = Layout::Player.transfer(&mut slots, 36);
        assert_eq!(moved, item(1, 64));
        assert_eq!(slots[9], item(1, 64));
        assert_eq!(slots[36], None);
        // Armor goes to its slot.
        slots[10] = item(310, 1);
        Layout::Player.transfer(&mut slots, 10);
        assert_eq!(slots[5], item(310, 1));
    }
}
