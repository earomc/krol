//! Anvils (vanilla: `BlockAnvil.onBlockActivated`, `ContainerRepair`, the output slot's
//! `canTakeStack` / `onPickupFromSlot`, `MC|ItemName`).
//!
//! The repair output is recomputed whenever an input slot changes or the client sends a
//! new name. The cost is sent to the client only when the window opens; afterwards the
//! client computes it itself. Falling anvils live in `falling_block`.

use krol_nbt::{Compound, List, Tag, TagType};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack};

use super::Game;
use super::container::{Layout, OpenContainer};
use super::enchanting;
use super::ids::block;
use super::item_data;

/// The anvil's own slots: two inputs and the output.
pub const ANVIL_SLOTS: usize = 3;
/// The output slot.
pub const OUTPUT: usize = 2;
/// `enchanted_book`.
const ENCHANTED_BOOK: i16 = 403;
/// A cost from here on is "Too Expensive!" outside creative mode.
const TOO_EXPENSIVE: i32 = 40;
/// Chance that taking the output damages the anvil.
const DAMAGE_CHANCE: f32 = 0.12;
/// `playAuxSFX` ids: the anvil breaks / is used.
const EFFECT_ANVIL_BREAK: i32 = 1020;
const EFFECT_ANVIL_USE: i32 = 1021;
/// Names longer than this (in UTF-16 units) are ignored.
const MAX_NAME: usize = 30;

/// What `ContainerRepair` keeps besides its slots.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnvilState {
    /// `maximumCost`.
    pub cost: i32,
    /// `materialCost`: material items a repair uses up.
    pub material: i32,
    /// `repairedItemName`.
    pub name: Option<String>,
}

/// The outcome of `updateRepairOutput`.
struct Repair {
    output: Option<ItemStack>,
    cost: i32,
    material: i32,
    /// The computation ran to its end, which includes a change check.
    detect: bool,
}

/// `ItemStack.getRepairCost`.
fn repair_cost(stack: &ItemStack) -> i32 {
    match stack.nbt.as_ref().and_then(|c| c.get("RepairCost")) {
        Some(Tag::Int(v)) => *v,
        _ => 0,
    }
}

/// `ItemStack.isItemStackDamageable`.
fn damageable(stack: &ItemStack) -> bool {
    item_data::max_damage(stack.id) > 0
        && !stack
            .nbt
            .as_ref()
            .is_some_and(|c| c.get_i8("Unbreakable") != 0)
}

/// `Item.getIsRepairable`: tools, swords and armor with their material's repair item.
fn repairable(item: i16, material: i16) -> bool {
    const PLANKS: i16 = 5;
    const COBBLESTONE: i16 = 4;
    const IRON_INGOT: i16 = 265;
    const GOLD_INGOT: i16 = 266;
    const DIAMOND: i16 = 264;
    const LEATHER: i16 = 334;
    let repair = match item {
        // Wood, stone, iron, diamond and gold tools and swords.
        268..=271 | 290 => Some(PLANKS),
        272..=275 | 291 => Some(COBBLESTONE),
        256..=258 | 267 | 292 => Some(IRON_INGOT),
        276..=279 | 293 => Some(DIAMOND),
        283..=286 | 294 => Some(GOLD_INGOT),
        298..=301 => Some(LEATHER),
        302..=309 => Some(IRON_INGOT),
        310..=313 => Some(DIAMOND),
        314..=317 => Some(GOLD_INGOT),
        _ => None,
    };
    // Hoes (290-294) are no ItemTool and do not override getIsRepairable.
    !(290..=294).contains(&item) && repair == Some(material)
}

/// `EnchantmentHelper.getEnchantments`: `(id, level)` in list order, a later duplicate
/// updating the earlier entry (a `LinkedHashMap`).
fn enchantment_map(stack: &ItemStack) -> Vec<(i16, i32)> {
    let key = if stack.id == ENCHANTED_BOOK {
        "StoredEnchantments"
    } else {
        "ench"
    };
    let mut map: Vec<(i16, i32)> = Vec::new();
    if let Some(list) = stack
        .nbt
        .as_ref()
        .and_then(|c| c.get_list(key, TagType::Compound))
    {
        for t in list.iter() {
            if let Tag::Compound(c) = t {
                put(&mut map, c.get_i16("id"), i32::from(c.get_i16("lvl")));
            }
        }
    }
    map
}

/// `Map.put` on the ordered enchantment map.
fn put(map: &mut Vec<(i16, i32)>, id: i16, level: i32) {
    match map.iter_mut().find(|e| e.0 == id) {
        Some(e) => e.1 = level,
        None => map.push((id, level)),
    }
}

/// The tag of a stack, created if missing.
fn tag(stack: &mut ItemStack) -> &mut Compound {
    stack.nbt.get_or_insert_with(Compound::new)
}

/// `ItemStack.setStackDisplayName`.
fn set_name(stack: &mut ItemStack, name: &str) {
    let tag = tag(stack);
    let mut display = tag.get_compound("display").cloned().unwrap_or_default();
    display.insert("Name", Tag::String(name.to_owned()));
    tag.insert("display", Tag::Compound(display));
}

/// `ItemStack.clearCustomName`: an emptied display compound goes, and then an empty tag.
fn clear_name(stack: &mut ItemStack) {
    let Some(nbt) = stack.nbt.as_mut() else {
        return;
    };
    let Some(mut display) = nbt.get_compound("display").cloned() else {
        return;
    };
    display.remove("Name");
    if display.is_empty() {
        nbt.remove("display");
        if nbt.is_empty() {
            stack.nbt = None;
        }
    } else {
        nbt.insert("display", Tag::Compound(display));
    }
}

/// `EnchantmentHelper.setEnchantments`.
fn set_enchantments(map: &[(i16, i32)], stack: &mut ItemStack) {
    let mut list = List::new();
    let book = stack.id == ENCHANTED_BOOK;
    for &(id, level) in map {
        if !enchanting::exists(i32::from(id)) {
            continue;
        }
        let mut e = Compound::new();
        e.insert("id", Tag::Short(id));
        e.insert("lvl", Tag::Short(level as i16));
        let _ = list.push(Tag::Compound(e));
        if book {
            enchanting::store_enchantment(stack, id, level);
        }
    }
    if !list.is_empty() {
        if !book {
            tag(stack).insert("ench", Tag::List(list));
        }
    } else if let Some(nbt) = stack.nbt.as_mut() {
        nbt.remove("ench");
    }
}

/// `StringUtils.isBlank`.
fn is_blank(name: Option<&str>) -> bool {
    name.is_none_or(|n| n.chars().all(char::is_whitespace))
}

/// The anvil cost of one level of an enchantment, by its weight.
fn level_cost(id: i16, from_book: bool) -> i32 {
    let cost = match enchanting::weight_of(id) {
        1 => 8,
        2 => 4,
        5 => 2,
        10 => 1,
        _ => 0,
    };
    if from_book { (cost / 2).max(1) } else { cost }
}

/// `ContainerRepair.updateRepairOutput`.
fn update_repair_output(
    first: Option<&ItemStack>,
    second: Option<&ItemStack>,
    name: Option<&str>,
    creative: bool,
) -> Repair {
    let fail = Repair {
        output: None,
        cost: 0,
        material: 0,
        detect: false,
    };
    let Some(first) = first else {
        return fail;
    };
    let mut out = first.clone();
    let mut enchantments = enchantment_map(&out);
    let mut total = 0;
    let base = repair_cost(first) + second.map_or(0, repair_cost);
    let mut material = 0;
    let mut rename = 0;
    if let Some(second) = second {
        let book = second.id == ENCHANTED_BOOK && !enchantment_map(second).is_empty();
        let max = i32::from(item_data::max_damage(out.id));
        if damageable(&out) && repairable(first.id, second.id) {
            let mut repair = i32::from(out.damage).min(max / 4);
            if repair <= 0 {
                return fail;
            }
            let mut used = 0;
            while repair > 0 && used < i32::from(second.count) {
                out.damage = (i32::from(out.damage) - repair).max(0) as i16;
                total += 1;
                repair = i32::from(out.damage).min(max / 4);
                used += 1;
            }
            material = used;
        } else {
            if !book && (out.id != second.id || !damageable(&out)) {
                return fail;
            }
            if damageable(&out) && !book {
                let left = max - i32::from(first.damage);
                let other = i32::from(item_data::max_damage(second.id)) - i32::from(second.damage);
                let bonus = other + max * 12 / 100;
                let damage = (max - (left + bonus)).max(0);
                if damage < i32::from(out.damage) {
                    out.damage = damage as i16;
                    total += 2;
                }
            }
            for (id, other_level) in enchantment_map(second) {
                if !enchanting::exists(i32::from(id)) {
                    continue;
                }
                let current = enchantments.iter().find(|e| e.0 == id).map_or(0, |e| e.1);
                let mut level = if current == other_level {
                    other_level + 1
                } else {
                    other_level.max(current)
                };
                let mut ok = enchanting::can_apply(id, first);
                if creative || first.id == ENCHANTED_BOOK {
                    ok = true;
                }
                for &(present, _) in &enchantments {
                    if present != id && !enchanting::can_apply_together(id, present) {
                        ok = false;
                        total += 1;
                    }
                }
                if ok {
                    level = level.min(enchanting::max_level_of(id));
                    put(&mut enchantments, id, level);
                    total += level_cost(id, book) * level;
                }
            }
        }
    }
    if is_blank(name) {
        if item_data::custom_name(first).is_some() {
            rename = 1;
            total += rename;
            clear_name(&mut out);
        }
    } else if let Some(name) = name
        && name != item_data::display_name(first)
    {
        rename = 1;
        total += rename;
        set_name(&mut out, name);
    }
    let mut cost = base + total;
    let mut output = (total > 0).then_some(out);
    if rename == total && rename > 0 && cost >= TOO_EXPENSIVE {
        cost = TOO_EXPENSIVE - 1;
    }
    if cost >= TOO_EXPENSIVE && !creative {
        output = None;
    }
    if let Some(o) = &mut output {
        let mut next = repair_cost(o);
        if let Some(second) = second
            && next < repair_cost(second)
        {
            next = repair_cost(second);
        }
        tag(o).insert("RepairCost", Tag::Int(next * 2 + 1));
        set_enchantments(&enchantments, o);
    }
    Repair {
        output,
        cost,
        material,
        detect: true,
    }
}

impl Game {
    /// `BlockAnvil.onBlockActivated` -> `displayGui(new BlockAnvil.Anvil(...))`.
    pub(super) fn open_anvil(&mut self, i: usize, pos: (i32, i32, i32)) {
        if self.players[i].window.open.is_some() {
            self.close_screen(i);
        }
        let title = crate::chat::Component::translate("tile.anvil.name", vec![]).to_json_string();
        let id = self.players[i].window.next_window_id();
        self.players[i].send(ClientboundPlay::OpenWindow {
            window: id as u8,
            kind: "minecraft:anvil".into(),
            title,
            slots: 0,
        });
        self.players[i].window.anvil = AnvilState::default();
        let layout = Layout::Anvil;
        self.players[i].window.open = Some(OpenContainer {
            id,
            layout,
            pos,
            slots: vec![None; ANVIL_SLOTS],
            parts: Vec::new(),
            ender: false,
            sent: vec![None; layout.len()],
            fields: [0; 4],
            taken: 0,
            cart: None,
        });
        self.open_container_gui(i);
        // ContainerRepair.onCraftGuiOpened: the cost, once.
        self.players[i].send(ClientboundPlay::WindowProperty {
            window: id as u8,
            property: 0,
            value: 0,
        });
    }

    /// The anvil window's slots, if one is open.
    pub(super) fn anvil_slots(&self, i: usize) -> Option<Vec<Option<ItemStack>>> {
        self.players[i]
            .window
            .open
            .as_ref()
            .filter(|o| o.layout == Layout::Anvil)
            .map(|o| o.slots.clone())
    }

    /// The output slot's `canTakeStack`.
    pub(super) fn anvil_can_take(&self, i: usize) -> bool {
        let p = &self.players[i];
        let cost = p.window.anvil.cost;
        let has = p
            .window
            .open
            .as_ref()
            .is_some_and(|o| o.slots[OUTPUT].is_some());
        (p.gamemode == 1 || p.data.get_i32("XpLevel") >= cost) && cost > 0 && has
    }

    /// A change check limited to the anvil's own slots, or over the whole window.
    fn anvil_detect(&mut self, i: usize, whole: bool) {
        let p = &mut self.players[i];
        let mut packets = Vec::new();
        if let Some(o) = p.window.open.as_mut() {
            let end = if whole { o.sent.len() } else { ANVIL_SLOTS };
            let mut flat = o.slots.clone();
            flat.extend(p.inventory[9..45].iter().cloned());
            for (k, item) in flat.into_iter().enumerate().take(end) {
                if o.sent[k] != item {
                    o.sent[k] = item.clone();
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
    }

    /// `ContainerRepair.onCraftMatrixChanged` after a click changed an input: the
    /// container's change check (`super`), then the new output. The check sees the
    /// whole window as the click left it, except after a shift-click from the player's
    /// inventory into an input, whose source slot is only emptied afterwards.
    ///
    /// A slot write marks the inputs dirty twice (`setInventorySlotContents`, then
    /// `onSlotChanged`), so a second check follows the recomputation and sends an output
    /// it cleared even when it stopped early.
    pub(super) fn anvil_inputs_changed(&mut self, i: usize, mode: i8, slot: i16) {
        let whole = !(mode == 1 && slot >= ANVIL_SLOTS as i16);
        self.anvil_detect(i, whole);
        self.update_anvil_output(i);
        self.anvil_detect(i, whole);
    }

    /// `updateRepairOutput`: the output and cost, and (when it ran to its end) a change
    /// check.
    pub(super) fn update_anvil_output(&mut self, i: usize) {
        let Some(slots) = self.anvil_slots(i) else {
            return;
        };
        let p = &self.players[i];
        let repair = update_repair_output(
            slots[0].as_ref(),
            slots[1].as_ref(),
            p.window.anvil.name.as_deref(),
            p.gamemode == 1,
        );
        let p = &mut self.players[i];
        p.window.anvil.cost = repair.cost;
        p.window.anvil.material = repair.material;
        if let Some(o) = p.window.open.as_mut() {
            o.slots[OUTPUT] = repair.output;
        }
        if repair.detect {
            self.anvil_detect(i, false);
        }
    }

    /// The output slot's `onPickupFromSlot`: pay the levels, use up the inputs (each
    /// write runs the container's change check over the whole window, then a
    /// recomputation that only clears the output), and maybe damage the anvil.
    pub(super) fn anvil_output_taken(&mut self, i: usize) {
        let Some(pos) = self.players[i].window.open.as_ref().map(|o| o.pos) else {
            return;
        };
        let creative = self.players[i].gamemode == 1;
        let AnvilState { cost, material, .. } = self.players[i].window.anvil.clone();
        if !creative {
            self.players[i].add_experience_levels(-cost);
        }
        if let Some(o) = self.players[i].window.open.as_mut() {
            o.slots[0] = None;
        }
        self.anvil_detect(i, true);
        self.update_anvil_output(i);
        if let Some(o) = self.players[i].window.open.as_mut() {
            let keep = o.slots[1]
                .as_ref()
                .is_some_and(|s| material > 0 && i32::from(s.count) > material);
            if keep {
                if let Some(s) = o.slots[1].as_mut() {
                    s.count -= material as i8;
                }
            } else {
                o.slots[1] = None;
            }
        }
        self.anvil_detect(i, true);
        self.update_anvil_output(i);
        self.players[i].window.anvil.cost = 0;
        let state = self.level.overworld.block(pos.0, pos.1, pos.2);
        // The player's own (unseeded) random.
        let roll = krol_java::JavaRandom::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as i64),
        )
        .next_float();
        if !creative && state >> 4 == block::ANVIL && roll < DAMAGE_CHANCE {
            let damage = ((state & 15) >> 2) + 1;
            if damage > 2 {
                self.set_block_notify(pos, 0);
                self.block_effect(EFFECT_ANVIL_BREAK, pos);
            } else {
                self.level.overworld.set_block(
                    pos.0,
                    pos.1,
                    pos.2,
                    (state & !0b1100) | damage << 2,
                );
                self.block_effect(EFFECT_ANVIL_USE, pos);
            }
        } else {
            self.block_effect(EFFECT_ANVIL_USE, pos);
        }
    }

    /// `MC|ItemName` with an anvil open: the filtered name (at most 30 characters, else
    /// ignored), or no name for an empty payload.
    pub(super) fn anvil_item_name(&mut self, i: usize, data: &[u8]) {
        if self.anvil_slots(i).is_none() {
            return;
        }
        let name = if data.is_empty() {
            String::new()
        } else {
            let mut r = krol_protocol::buf::PacketReader::new(data);
            let Ok(raw) = r.string(32767) else {
                // TODO(M4): vanilla logs the exception of a malformed payload.
                return;
            };
            // ChatAllowedCharacters.filterAllowedCharacters.
            let name: String = raw
                .chars()
                .filter(|&c| c != '\u{a7}' && c >= ' ' && c != '\u{7f}')
                .collect();
            if name.encode_utf16().count() > MAX_NAME {
                return;
            }
            name
        };
        self.players[i].window.anvil.name = Some(name);
        self.update_anvil_output(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sword(damage: i16) -> ItemStack {
        ItemStack {
            id: 276,
            count: 1,
            damage,
            nbt: None,
        }
    }

    #[test]
    fn repairing_with_diamonds() {
        // A quarter of 1561 per diamond: 1000 damage takes three diamonds.
        let diamonds = ItemStack {
            id: 264,
            count: 5,
            damage: 0,
            nbt: None,
        };
        let r = update_repair_output(Some(&sword(1000)), Some(&diamonds), None, false);
        assert_eq!(r.material, 3);
        assert_eq!(r.cost, 3);
        let out = r.output.unwrap();
        assert_eq!(out.damage, 0);
        assert_eq!(repair_cost(&out), 1);
    }

    #[test]
    fn renaming_to_the_same_name_costs_nothing() {
        let r = update_repair_output(Some(&sword(0)), None, Some("Diamond Sword"), false);
        assert!(r.output.is_none());
        let r = update_repair_output(Some(&sword(0)), None, Some("Excalibur"), false);
        assert_eq!(r.cost, 1);
        assert_eq!(
            item_data::custom_name(r.output.as_ref().unwrap()),
            Some("Excalibur")
        );
    }
}
