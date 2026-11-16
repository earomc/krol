//! Enchanting tables (vanilla: `BlockEnchantmentTable`, `ContainerEnchantment`,
//! `EnchantmentHelper.calcItemStackEnchantability` / `buildEnchantmentList` /
//! `mapEnchantmentData`, the `Enchantment` registry, `WeightedRandom`).
//!
//! The offers are a pure function of the player's `XpSeed`, the item and the
//! bookshelves around the table; the container reseeds its own random from the seed for
//! every computation, so no shared random is involved. Paying for an enchantment draws a
//! new seed from the player entity's (unseeded) random.
//!
//! TODO(M4): the table's book animation (`TileEntityEnchantmentTable.update`) runs on
//! the server too but has no observable effect, so it is not ticked.

use krol_java::JavaRandom;
use krol_nbt::{Compound, List, Tag, TagType};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack};

use super::Game;
use super::container::{Layout, OpenContainer};

/// Slots of the table itself: the item and the lapis.
pub const TABLE_SLOTS: usize = 2;
/// The lapis slot.
pub const LAPIS_SLOT: usize = 1;
/// `book` and `enchanted_book`.
const BOOK: i16 = 340;
const ENCHANTED_BOOK: i16 = 403;
/// `dye` with damage 4 (blue: lapis lazuli).
const DYE: i16 = 351;
const LAPIS_DAMAGE: i16 = 4;
/// `bookshelf`.
const BOOKSHELF: u16 = 47;
/// Bookshelves beyond this count do not raise the offers.
const MAX_POWER: i32 = 15;
/// `bow`, `fishing_rod`.
const BOW: i16 = 261;
const FISHING_ROD: i16 = 346;

/// What `ContainerEnchantment` keeps: the seed it computes with and the three offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offers {
    /// `xpSeed`.
    pub seed: i32,
    /// `enchantLevels`.
    pub levels: [i32; 3],
    /// `field_178151_h`: enchantment id | level << 8 of the hint, or -1.
    pub clues: [i32; 3],
}

impl Default for Offers {
    fn default() -> Self {
        Self {
            seed: 0,
            levels: [0; 3],
            clues: [-1; 3],
        }
    }
}

/// `Enchantment.enchantmentsBookList`: every enchantment id in registration order.
const ENCHANTMENTS: [i16; 25] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 16, 17, 18, 19, 20, 21, 32, 33, 34, 35, 48, 49, 50, 51, 61, 62,
];

/// `EnumEnchantmentType` (the kinds the registry uses).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Armor,
    ArmorFeet,
    ArmorHead,
    ArmorTorso,
    Weapon,
    Digger,
    Breakable,
    Bow,
    FishingRod,
}

/// The kind of enchantment `id` (protection's `ARMOR`, except feather falling).
fn kind(id: i16) -> Kind {
    match id {
        2 | 8 => Kind::ArmorFeet,
        0..=4 => Kind::Armor,
        5 | 6 => Kind::ArmorHead,
        7 => Kind::ArmorTorso,
        16..=21 => Kind::Weapon,
        34 => Kind::Breakable,
        32 | 33 | 35 => Kind::Digger,
        48..=51 => Kind::Bow,
        _ => Kind::FishingRod,
    }
}

/// `Enchantment.getWeight`.
fn weight(id: i16) -> i32 {
    match id {
        0 | 16 | 32 | 48 => 10,
        1 | 2 | 4 | 17 | 18 | 19 | 34 => 5,
        7 | 33 | 51 => 1,
        _ => 2,
    }
}

/// `Enchantment.getMaxLevel`.
fn max_level(id: i16) -> i32 {
    match id {
        0..=4 => 4,
        16..=18 | 32 | 48 => 5,
        5 | 7 | 8 | 21 | 34 | 35 | 61 | 62 => 3,
        19 | 20 | 49 => 2,
        _ => 1,
    }
}

/// `Enchantment.getMinEnchantability` of the base class, which several subclasses'
/// maximums build on.
fn base_min(level: i32) -> i32 {
    1 + level * 10
}

/// `getMinEnchantability(level)` of enchantment `id`.
fn min_enchantability(id: i16, level: i32) -> i32 {
    // EnchantmentProtection / EnchantmentDamage tables.
    const PROTECTION: [(i32, i32); 5] = [(1, 11), (10, 8), (5, 6), (5, 8), (3, 6)];
    const DAMAGE: [(i32, i32); 3] = [(1, 11), (5, 8), (5, 8)];
    match id {
        0..=4 => {
            let (base, step) = PROTECTION[id as usize];
            base + (level - 1) * step
        }
        16..=18 => {
            let (base, step) = DAMAGE[(id - 16) as usize];
            base + (level - 1) * step
        }
        5 | 8 => 10 * level,
        6 => 1,
        7 | 20 => 10 + 20 * (level - 1),
        19 => 5 + 20 * (level - 1),
        21 | 35 | 61 | 62 => 15 + (level - 1) * 9,
        32 | 48 => 1 + 10 * (level - 1),
        33 => 15,
        34 => 5 + (level - 1) * 8,
        49 => 12 + (level - 1) * 20,
        _ => 20,
    }
}

/// `getMaxEnchantability(level)` of enchantment `id`.
fn max_enchantability(id: i16, level: i32) -> i32 {
    const PROTECTION_SPAN: [i32; 5] = [20, 12, 10, 12, 15];
    match id {
        0..=4 => min_enchantability(id, level) + PROTECTION_SPAN[id as usize],
        16..=18 => min_enchantability(id, level) + 20,
        5 => min_enchantability(id, level) + 30,
        6 => min_enchantability(id, level) + 40,
        8 | 48 => min_enchantability(id, level) + 15,
        49 => min_enchantability(id, level) + 25,
        50 | 51 => 50,
        // Vanilla quirk: these add 50 to the base class's minimum, not their own.
        _ => base_min(level) + 50,
    }
}

/// `chosen.canApplyTogether(other)`.
fn compatible(chosen: i16, other: i16) -> bool {
    match chosen {
        0..=4 if (0..=4).contains(&other) => chosen != other && (chosen == 2 || other == 2),
        16..=18 => !(16..=18).contains(&other),
        21 | 35 | 61 => chosen != other && other != 33,
        33 => chosen != other && other != 35,
        _ => chosen != other,
    }
}

/// Registry names (`Enchantment.locationEnchantments`), in the order of [`ENCHANTMENTS`].
const NAMES: [&str; 25] = [
    "protection",
    "fire_protection",
    "feather_falling",
    "blast_protection",
    "projectile_protection",
    "respiration",
    "aqua_affinity",
    "thorns",
    "depth_strider",
    "sharpness",
    "smite",
    "bane_of_arthropods",
    "knockback",
    "fire_aspect",
    "looting",
    "efficiency",
    "silk_touch",
    "unbreaking",
    "fortune",
    "power",
    "punch",
    "flame",
    "infinity",
    "luck_of_the_sea",
    "lure",
];

/// English names (`en_US.lang`, translated on the server), in the order of
/// [`ENCHANTMENTS`].
const ENGLISH: [&str; 25] = [
    "Protection",
    "Fire Protection",
    "Feather Falling",
    "Blast Protection",
    "Projectile Protection",
    "Respiration",
    "Aqua Affinity",
    "Thorns",
    "Depth Strider",
    "Sharpness",
    "Smite",
    "Bane of Arthropods",
    "Knockback",
    "Fire Aspect",
    "Looting",
    "Efficiency",
    "Silk Touch",
    "Unbreaking",
    "Fortune",
    "Power",
    "Punch",
    "Flame",
    "Infinity",
    "Luck of the Sea",
    "Lure",
];

/// Whether enchantment `id` exists (`Enchantment.getEnchantmentById` is not null).
pub fn exists(id: i32) -> bool {
    ENCHANTMENTS.iter().any(|&e| i32::from(e) == id)
}

/// `Enchantment.getEnchantmentByLocation`: the id for a registry name. Like
/// `ResourceLocation`, the domain is lowercased and the path kept as typed.
pub fn by_name(name: &str) -> Option<i16> {
    let (domain, path) = match name.find(':') {
        // Vanilla quirk: a one-character domain is dropped (only `i > 1` takes it).
        Some(i) if i > 1 => (name[..i].to_lowercase(), &name[i + 1..]),
        Some(i) => ("minecraft".to_owned(), &name[i + 1..]),
        None => ("minecraft".to_owned(), name),
    };
    if domain != "minecraft" {
        return None;
    }
    NAMES
        .iter()
        .position(|n| *n == path)
        .map(|k| ENCHANTMENTS[k])
}

/// `Enchantment.getTranslatedName`: the English name and the level as a Roman numeral
/// (levels without a translation show their key).
pub fn translated_name(id: i16, level: i32) -> String {
    const LEVELS: [&str; 10] = ["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"];
    let name = ENCHANTMENTS
        .iter()
        .position(|&e| e == id)
        .map_or("", |k| ENGLISH[k]);
    let level = usize::try_from(level)
        .ok()
        .and_then(|l| l.checked_sub(1))
        .and_then(|l| LEVELS.get(l))
        .map_or_else(|| format!("enchantment.level.{level}"), |l| (*l).to_owned());
    format!("{name} {level}")
}

/// `Enchantment.canApply`: the kind's items, plus each enchantment's extras (axes for
/// the damage enchantments, any armor for thorns, shears for efficiency and silk touch,
/// anything damageable for unbreaking).
pub fn can_apply(id: i16, stack: &ItemStack) -> bool {
    const SHEARS: i16 = 359;
    let extra = match id {
        16..=18 => matches!(stack.id, 258 | 271 | 275 | 279 | 286),
        7 => armor_slot(stack.id).is_some(),
        32 | 33 => stack.id == SHEARS,
        // ItemStack.isItemStackDamageable: not with the Unbreakable tag.
        34 => {
            super::item_data::max_damage(stack.id) > 0
                && !stack
                    .nbt
                    .as_ref()
                    .is_some_and(|c| c.get_i8("Unbreakable") != 0)
        }
        _ => false,
    };
    extra || applies(kind(id), stack.id)
}

/// `Enchantment.getMaxLevel`, for commands.
pub fn max_level_of(id: i16) -> i32 {
    max_level(id)
}

/// `chosen.canApplyTogether(other)`, for commands.
pub fn can_apply_together(chosen: i16, other: i16) -> bool {
    compatible(chosen, other)
}

/// `ItemEnchantedBook.addEnchantment` (the higher level of a stored enchantment
/// stays).
pub fn store_enchantment(stack: &mut ItemStack, id: i16, level: i32) {
    add_enchantment(stack, id, level, true);
}

/// `Enchantment.getWeight`, for anvils.
pub fn weight_of(id: i16) -> i32 {
    weight(id)
}

/// `ItemStack.addEnchantment`, for commands.
pub fn enchant_stack(stack: &mut ItemStack, id: i16, level: i32) {
    add_enchantment(stack, id, level, false);
}

/// The tool material or armor material index of an item, if it has one.
fn armor_slot(item: i16) -> Option<i16> {
    (298..=317).contains(&item).then(|| (item - 298) % 4)
}

fn is_sword(item: i16) -> bool {
    matches!(item, 267 | 268 | 272 | 276 | 283)
}

/// `ItemTool`: shovels, pickaxes and axes (hoes and shears are not).
fn is_tool(item: i16) -> bool {
    matches!(item, 256..=258 | 269..=271 | 273..=275 | 277..=279 | 284..=286)
}

/// `EnumEnchantmentType.canEnchantItem`.
fn applies(kind: Kind, item: i16) -> bool {
    if kind == Kind::Breakable && super::item_data::is_damageable(item) {
        return true;
    }
    if let Some(slot) = armor_slot(item) {
        return kind == Kind::Armor
            || match slot {
                0 => kind == Kind::ArmorHead,
                1 => kind == Kind::ArmorTorso,
                3 => kind == Kind::ArmorFeet,
                _ => false,
            };
    }
    if is_sword(item) {
        kind == Kind::Weapon
    } else if is_tool(item) {
        kind == Kind::Digger
    } else if item == BOW {
        kind == Kind::Bow
    } else {
        item == FISHING_ROD && kind == Kind::FishingRod
    }
}

/// `Item.getItemEnchantability`: the tool or armor material's, 1 for bows, fishing rods
/// and books, else 0.
fn enchantability(item: i16) -> i32 {
    match item {
        268..=271 => 15,
        272..=275 => 5,
        256..=258 | 267 => 14,
        276..=279 => 10,
        283..=286 => 22,
        298..=317 => [15, 12, 9, 10, 25][((item - 298) / 4) as usize],
        BOW | FISHING_ROD | BOOK => 1,
        _ => 0,
    }
}

/// `ItemStack.isItemEnchanted`: an `ench` list in the tag.
fn is_enchanted(stack: &ItemStack) -> bool {
    stack
        .nbt
        .as_ref()
        .is_some_and(|c| c.get_list("ench", TagType::Compound).is_some())
}

/// `ItemStack.isItemEnchantable`: a tool (one per stack and damageable, or a single
/// book) that is not enchanted yet.
fn is_enchantable(stack: &ItemStack) -> bool {
    let tool = if stack.id == BOOK {
        stack.count == 1
    } else {
        super::item_entity::max_stack(stack.id) == 1 && super::item_data::is_damageable(stack.id)
    };
    tool && !is_enchanted(stack)
}

/// Whether a stack is lapis lazuli (the lapis slot's `isItemValid`).
pub fn is_lapis(stack: &ItemStack) -> bool {
    stack.id == DYE && stack.damage == LAPIS_DAMAGE
}

/// `EnchantmentHelper.calcItemStackEnchantability`: the level offered in `slot`.
fn offer_level(rand: &mut JavaRandom, slot: i32, power: i32, item: i16) -> i32 {
    if enchantability(item) <= 0 {
        return 0;
    }
    let power = power.min(MAX_POWER);
    let j = rand.next_int_bounded(8) + 1 + (power >> 1) + rand.next_int_bounded(power + 1);
    match slot {
        0 => (j / 3).max(1),
        1 => j * 2 / 3 + 1,
        _ => j.max(power * 2),
    }
}

/// `mapEnchantmentData` as the `HashMap<Integer, EnchantmentData>` it builds: entries
/// are listed in Java's iteration order (by bucket, then insertion) when read through
/// [`JavaIntMap::values`].
struct JavaIntMap {
    /// `(id, level)` in insertion order.
    entries: Vec<(i16, i32)>,
    /// The table size; it grows like Java's (16, doubling past 3/4 full) and never
    /// shrinks.
    capacity: usize,
}

impl JavaIntMap {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            capacity: 16,
        }
    }

    fn put(&mut self, id: i16, level: i32) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.0 == id) {
            e.1 = level;
            return;
        }
        self.entries.push((id, level));
        if self.entries.len() > self.capacity * 3 / 4 {
            self.capacity *= 2;
        }
    }

    /// The entries in iteration order (`Integer.hashCode` is the value itself).
    fn values(&self) -> Vec<(i16, i32)> {
        let mut v = self.entries.clone();
        let mask = self.capacity - 1;
        v.sort_by_key(|e| (e.0 as usize) & mask);
        v
    }

    fn retain(&mut self, keep: impl Fn(i16) -> bool) {
        self.entries.retain(|e| keep(e.0));
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// `mapEnchantmentData`: every enchantment that fits the item (anything for a book) with
/// its highest level whose enchantability range holds `level`.
fn candidates(level: i32, item: i16) -> JavaIntMap {
    let mut map = JavaIntMap::new();
    for id in ENCHANTMENTS {
        if applies(kind(id), item) || item == BOOK {
            for l in 1..=max_level(id) {
                if level >= min_enchantability(id, l) && level <= max_enchantability(id, l) {
                    map.put(id, l);
                }
            }
        }
    }
    map
}

/// `WeightedRandom.getRandomItem`.
fn pick_weighted(rand: &mut JavaRandom, entries: &[(i16, i32)]) -> Option<(i16, i32)> {
    let total: i32 = entries.iter().map(|e| weight(e.0)).sum();
    if total <= 0 {
        return None;
    }
    let mut n = rand.next_int_bounded(total);
    for &e in entries {
        n -= weight(e.0);
        if n < 0 {
            return Some(e);
        }
    }
    None
}

/// `EnchantmentHelper.buildEnchantmentList`.
fn build_list(rand: &mut JavaRandom, item: i16, level: i32) -> Option<Vec<(i16, i32)>> {
    let e = enchantability(item);
    if e <= 0 {
        return None;
    }
    let e = e / 2;
    let e = 1 + rand.next_int_bounded((e >> 1) + 1) + rand.next_int_bounded((e >> 1) + 1);
    let k = e + level;
    let spread = (rand.next_float() + rand.next_float() - 1.0) * 0.15;
    let l = ((k as f32 * (1.0 + spread) + 0.5) as i32).max(1);
    let mut map = candidates(l, item);
    if map.is_empty() {
        return None;
    }
    let first = pick_weighted(rand, &map.values())?;
    let mut list = vec![first];
    let mut j = l;
    while rand.next_int_bounded(50) <= j {
        let chosen = list.clone();
        map.retain(|id| chosen.iter().all(|c| compatible(c.0, id)));
        if !map.is_empty()
            && let Some(more) = pick_weighted(rand, &map.values())
        {
            list.push(more);
        }
        j >>= 1;
    }
    Some(list)
}

/// `ContainerEnchantment.func_178148_a`: the enchantments behind offer `slot`; a book
/// loses one of several at random.
fn offer_list(
    rand: &mut JavaRandom,
    seed: i32,
    item: i16,
    slot: i32,
    level: i32,
) -> Option<Vec<(i16, i32)>> {
    rand.set_seed(i64::from(seed.wrapping_add(slot)));
    let mut list = build_list(rand, item, level)?;
    if item == BOOK && list.len() > 1 {
        let k = rand.next_int_bounded(list.len() as i32) as usize;
        list.remove(k);
    }
    Some(list)
}

/// `ItemStack.addEnchantment` / `ItemEnchantedBook.addEnchantment`.
fn add_enchantment(stack: &mut ItemStack, id: i16, level: i32, stored: bool) {
    let tag = stack.nbt.get_or_insert_with(Compound::new);
    let key = if stored { "StoredEnchantments" } else { "ench" };
    let mut list = tag
        .get_list(key, TagType::Compound)
        .cloned()
        .unwrap_or_else(List::new);
    if stored
        && list
            .iter()
            .any(|t| matches!(t, Tag::Compound(c) if c.get_i16("id") == id))
    {
        // A book keeps the higher level of an enchantment it already has.
        let mut updated = List::new();
        for t in list.iter() {
            let mut t = t.clone();
            if let Tag::Compound(c) = &mut t
                && c.get_i16("id") == id
                && i32::from(c.get_i16("lvl")) < level
            {
                c.insert("lvl", Tag::Short(level as i16));
            }
            let _ = updated.push(t);
        }
        tag.insert(key, Tag::List(updated));
        return;
    }
    let mut entry = Compound::new();
    entry.insert("id", Tag::Short(id));
    // ItemStack.addEnchantment stores (short)(byte)level.
    let lvl = if stored {
        level as i16
    } else {
        i16::from(level as i8)
    };
    entry.insert("lvl", Tag::Short(lvl));
    let _ = list.push(Tag::Compound(entry));
    tag.insert(key, Tag::List(list));
}

impl Game {
    /// `BlockEnchantmentTable.onBlockActivated` -> `displayGui`.
    pub(super) fn open_enchanting_table(&mut self, i: usize, pos: (i32, i32, i32)) {
        let Some(custom) = self
            .level
            .overworld
            .tile_entity(pos)
            .map(|c| c.get_str("CustomName").to_owned())
        else {
            return;
        };
        if self.players[i].window.open.is_some() {
            self.close_screen(i);
        }
        let title = if custom.is_empty() {
            crate::chat::Component::translate("container.enchant", vec![])
        } else {
            crate::chat::Component::text(&custom)
        }
        .to_json_string();
        let id = self.players[i].window.next_window_id();
        self.players[i].send(ClientboundPlay::OpenWindow {
            window: id as u8,
            kind: "minecraft:enchanting_table".into(),
            title,
            slots: 0,
        });
        let seed = self.players[i].data.get_i32("XpSeed");
        self.players[i].window.offers = Offers {
            seed,
            ..Offers::default()
        };
        let layout = Layout::Enchanting;
        self.players[i].window.open = Some(OpenContainer {
            id,
            layout,
            pos,
            slots: vec![None; TABLE_SLOTS],
            parts: Vec::new(),
            ender: false,
            sent: vec![None; layout.len()],
            fields: [0; 4],
            taken: 0,
        });
        // Container.onCraftGuiOpened (with its change check), then the table's own
        // properties.
        self.open_container_gui(i);
        self.send_enchanting_fields(i);
    }

    /// `ContainerEnchantment.detectAndSendChanges`' properties: all seven, every time.
    pub(super) fn send_enchanting_fields(&mut self, i: usize) {
        let p = &self.players[i];
        let Some(window) = p
            .window
            .open
            .as_ref()
            .filter(|o| o.layout == Layout::Enchanting)
            .map(|o| o.id as u8)
        else {
            return;
        };
        let o = p.window.offers;
        let values = [
            o.levels[0],
            o.levels[1],
            o.levels[2],
            o.seed & -16,
            o.clues[0],
            o.clues[1],
            o.clues[2],
        ];
        for (property, value) in values.into_iter().enumerate() {
            // S31PacketWindowProperty writes the value as a short.
            p.send(ClientboundPlay::WindowProperty {
                window,
                property: property as i16,
                value: value as i16,
            });
        }
    }

    /// `ContainerEnchantment.onCraftMatrixChanged` after a table slot changed. With an
    /// enchantable item it recomputes the offers and runs a change check of its own:
    /// the table slots that changed and the properties go out right away.
    pub(super) fn enchanting_slots_changed(&mut self, i: usize) {
        let Some((pos, item)) = self.players[i]
            .window
            .open
            .as_ref()
            .filter(|o| o.layout == Layout::Enchanting)
            .map(|o| (o.pos, o.slots[0].clone()))
        else {
            return;
        };
        let Some(item) = item.filter(is_enchantable) else {
            let offers = &mut self.players[i].window.offers;
            offers.levels = [0; 3];
            offers.clues = [-1; 3];
            return;
        };
        let power = self.bookshelf_power(pos);
        let mut offers = self.players[i].window.offers;
        let mut rand = JavaRandom::new(i64::from(offers.seed));
        for slot in 0..3 {
            let level = offer_level(&mut rand, slot as i32, power, item.id);
            offers.levels[slot] = if level < slot as i32 + 1 { 0 } else { level };
            offers.clues[slot] = -1;
        }
        for slot in 0..3 {
            if offers.levels[slot] > 0
                && let Some(list) = offer_list(
                    &mut rand,
                    offers.seed,
                    item.id,
                    slot as i32,
                    offers.levels[slot],
                )
                && !list.is_empty()
            {
                let (id, level) = list[rand.next_int_bounded(list.len() as i32) as usize];
                offers.clues[slot] = i32::from(id) | level << 8;
            }
        }
        self.players[i].window.offers = offers;
        // The table's change check: only the table slots have changed at this point
        // of the click (the player's slots are written after it).
        let p = &mut self.players[i];
        let mut packets = Vec::new();
        if let Some(o) = p.window.open.as_mut() {
            for k in 0..TABLE_SLOTS {
                if o.sent[k] != o.slots[k] {
                    o.sent[k] = o.slots[k].clone();
                    packets.push(ClientboundPlay::SetSlot {
                        window: o.id,
                        slot: k as i16,
                        item: o.slots[k].clone(),
                    });
                }
            }
        }
        for packet in packets {
            p.send(packet);
        }
        self.send_enchanting_fields(i);
    }

    /// The bookshelves around a table (`onCraftMatrixChanged`): two blocks out, on the
    /// table's level and the one above, each behind an air gap.
    fn bookshelf_power(&mut self, pos: (i32, i32, i32)) -> i32 {
        let w = &mut self.level.overworld;
        let mut id = |x: i32, y: i32, z: i32| w.block(pos.0 + x, pos.1 + y, pos.2 + z) >> 4;
        let mut power = 0;
        for dz in -1..=1 {
            for dx in -1..=1 {
                if (dx == 0 && dz == 0) || id(dx, 0, dz) != 0 || id(dx, 1, dz) != 0 {
                    continue;
                }
                let mut shelves = |spots: &[(i32, i32)]| {
                    let mut n = 0;
                    for &(x, z) in spots {
                        for y in 0..2 {
                            n += i32::from(id(x, y, z) == BOOKSHELF);
                        }
                    }
                    n
                };
                power += shelves(&[(dx * 2, dz * 2)]);
                if dx != 0 && dz != 0 {
                    power += shelves(&[(dx * 2, dz), (dx, dz * 2)]);
                }
            }
        }
        power
    }

    /// `ContainerEnchantment.enchantItem` for offer `button`, from the Enchant Item
    /// packet (`processEnchantItem`), followed by its change check.
    pub(super) fn enchant_item(&mut self, i: usize, window: i8, button: i8) {
        let p = &self.players[i];
        let Some(o) = p
            .window
            .open
            .as_ref()
            .filter(|o| o.layout == Layout::Enchanting && o.id == window)
        else {
            return;
        };
        if p.window.is_locked() || p.gamemode == 3 {
            return;
        }
        let Ok(slot) = usize::try_from(button) else {
            return;
        };
        if slot >= 3 {
            return;
        }
        let creative = p.gamemode == 1;
        let cost = slot as i32 + 1;
        let offers = p.window.offers;
        let (item, lapis) = (o.slots[0].clone(), o.slots[LAPIS_SLOT].clone());
        let paid = lapis.as_ref().is_some_and(|l| i32::from(l.count) >= cost);
        let level = p.data.get_i32("XpLevel");
        if (paid || creative)
            && offers.levels[slot] > 0
            && let Some(mut item) = item
            && ((level >= cost && level >= offers.levels[slot]) || creative)
        {
            let mut rand = JavaRandom::new(0);
            if let Some(list) = offer_list(
                &mut rand,
                offers.seed,
                item.id,
                slot as i32,
                offers.levels[slot],
            ) {
                self.remove_experience_levels(i, cost);
                let book = item.id == BOOK;
                if book {
                    item.id = ENCHANTED_BOOK;
                }
                for (id, l) in list {
                    add_enchantment(&mut item, id, l, book);
                }
                let mut lapis = lapis;
                if !creative && let Some(l) = &mut lapis {
                    l.count -= cost as i8;
                    if l.count <= 0 {
                        lapis = None;
                    }
                }
                self.add_stat(i, "stat.itemEnchanted", 1);
                let seed = self.players[i].data.get_i32("XpSeed");
                let p = &mut self.players[i];
                p.window.offers.seed = seed;
                if let Some(o) = p.window.open.as_mut() {
                    o.slots[0] = Some(item);
                    o.slots[LAPIS_SLOT] = lapis;
                }
                // markDirty and the explicit call: the item is enchanted now, so both
                // only clear the offers.
                self.enchanting_slots_changed(i);
            }
        }
        self.detect_container_changes(i);
    }

    /// `EntityPlayerMP.removeExperienceLevel`: the level drops, Set Experience is forced
    /// out, and a new enchantment seed comes from the player's own random.
    fn remove_experience_levels(&mut self, i: usize, levels: i32) {
        let data = &mut self.players[i].data;
        let mut level = data.get_i32("XpLevel") - levels;
        if level < 0 {
            level = 0;
            data.insert("XpP", Tag::Float(0.0));
            data.insert("XpTotal", Tag::Int(0));
        }
        data.insert("XpLevel", Tag::Int(level));
        self.players[i].xp_dirty = true;
        // The entity random is unseeded in vanilla.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as i64);
        let seed = JavaRandom::new(nanos ^ i64::from(self.players[i].entity_id)).next_int();
        self.players[i].data.insert("XpSeed", Tag::Int(seed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_map_iterates_like_java() {
        // 16 buckets: 17 lands after 0 and 16 (bucket 0 and 1), 33 in bucket 1 too.
        let mut m = JavaIntMap::new();
        for id in [33, 17, 0, 16, 2] {
            m.put(id, 1);
        }
        let ids: Vec<i16> = m.values().iter().map(|e| e.0).collect();
        assert_eq!(ids, [0, 16, 33, 17, 2]);
        // A thirteenth entry doubles the table to 32 buckets: 33 moves to bucket 1
        // and 17 to bucket 17, 16 to bucket 16.
        for id in [3, 4, 5, 6, 7, 8, 18, 19] {
            m.put(id, 1);
        }
        let ids: Vec<i16> = m.values().iter().map(|e| e.0).collect();
        assert_eq!(ids, [0, 33, 2, 3, 4, 5, 6, 7, 8, 16, 17, 18, 19]);
        // Removing entries does not shrink the table again.
        m.retain(|id| id < 10 || id == 33);
        let ids: Vec<i16> = m.values().iter().map(|e| e.0).collect();
        assert_eq!(ids, [0, 33, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn enchantability_windows() {
        // Sharpness V: 45 to 65; knockback II adds 50 to the base minimum (21 + 50).
        assert_eq!(min_enchantability(16, 5), 45);
        assert_eq!(max_enchantability(16, 5), 65);
        assert_eq!(min_enchantability(19, 2), 25);
        assert_eq!(max_enchantability(19, 2), 71);
        // Feather falling goes with other protections; two others do not.
        assert!(compatible(2, 0) && compatible(0, 2));
        assert!(!compatible(0, 1));
        assert!(!compatible(33, 35) && !compatible(35, 33));
    }
}
