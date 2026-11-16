//! Crafting recipes vanilla computes in code rather than from a pattern (`IRecipe`
//! implementations: `RecipeRepairItem`, `RecipesArmorDyes`, `RecipeBookCloning`,
//! `RecipesMapCloning`, `RecipesMapExtending`, `RecipeFireworks`, and
//! `RecipesBanners.RecipeAddPattern` / `RecipeDuplicatePattern`).

use krol_nbt::{Compound, Tag, TagType};
use krol_protocol::packets::play::ItemStack;

use super::item_data;

/// Item ids the special recipes look at.
mod id {
    pub const LEATHER_HELMET: i16 = 298;
    pub const LEATHER_BOOTS: i16 = 301;
    pub const GOLDEN_BOOTS: i16 = 317;
    pub const PAPER: i16 = 339;
    pub const DYE: i16 = 351;
    pub const FILLED_MAP: i16 = 358;
    pub const WRITABLE_BOOK: i16 = 386;
    pub const WRITTEN_BOOK: i16 = 387;
    pub const MAP: i16 = 395;
    pub const RED_FLOWER: i16 = 38;
    pub const BRICK_BLOCK: i16 = 45;
    pub const VINE: i16 = 106;
    pub const DIAMOND: i16 = 264;
    pub const FEATHER: i16 = 288;
    pub const GUNPOWDER: i16 = 289;
    pub const GOLDEN_APPLE: i16 = 322;
    pub const GLOWSTONE_DUST: i16 = 348;
    pub const GOLD_NUGGET: i16 = 371;
    pub const FIRE_CHARGE: i16 = 385;
    pub const SKULL: i16 = 397;
    pub const FIREWORKS: i16 = 401;
    pub const FIREWORK_CHARGE: i16 = 402;
    pub const BANNER: i16 = 425;
}

/// Firework colors by dye damage (`ItemDye.dyeColors`).
const DYE_COLORS: [i32; 16] = [
    1973019, 11743532, 3887386, 5320730, 2437522, 8073150, 2651799, 11250603, 4408131, 14188952,
    4312372, 14602026, 6719955, 12801229, 15435844, 15790320,
];
/// A rocket takes at most this much gunpowder (its flight duration).
const MAX_FLIGHT: i32 = 3;
/// Firework star shapes (`Type`) by shape ingredient.
const SHAPE_LARGE_BALL: i8 = 1;
const SHAPE_STAR: i8 = 2;
const SHAPE_CREEPER: i8 = 3;
const SHAPE_BURST: i8 = 4;
/// A banner takes at most this many patterns.
const MAX_BANNER_PATTERNS: usize = 6;
/// The oxeye daisy among the red flowers.
const OXEYE_DAISY: i16 = 8;
/// Skull damage values: wither skeleton skull, creeper head.
const WITHER_SKULL: i16 = 1;
const CREEPER_HEAD: i16 = 4;
/// The enchanted golden apple.
const NOTCH_APPLE: i16 = 1;

/// How a banner pattern is crafted (`TileEntityBanner.EnumBannerPattern`).
enum PatternRecipe {
    /// Dye in these three rows (`#`) of a 3x3 grid, the banner on a blank.
    Layers([&'static str; 3]),
    /// The banner, this item (id and damage) and at most one dye, anywhere.
    Item(i16, i16),
}

/// The craftable banner patterns in vanilla's order, by pattern id (`base` has no
/// recipe).
const PATTERNS: [(&str, PatternRecipe); 38] = {
    use PatternRecipe::{Item, Layers};
    [
        ("bl", Layers(["   ", "   ", "#  "])),
        ("br", Layers(["   ", "   ", "  #"])),
        ("tl", Layers(["#  ", "   ", "   "])),
        ("tr", Layers(["  #", "   ", "   "])),
        ("bs", Layers(["   ", "   ", "###"])),
        ("ts", Layers(["###", "   ", "   "])),
        ("ls", Layers(["#  ", "#  ", "#  "])),
        ("rs", Layers(["  #", "  #", "  #"])),
        ("cs", Layers([" # ", " # ", " # "])),
        ("ms", Layers(["   ", "###", "   "])),
        ("drs", Layers(["#  ", " # ", "  #"])),
        ("dls", Layers(["  #", " # ", "#  "])),
        ("ss", Layers(["# #", "# #", "   "])),
        ("cr", Layers(["# #", " # ", "# #"])),
        ("sc", Layers([" # ", "###", " # "])),
        ("bt", Layers(["   ", " # ", "# #"])),
        ("tt", Layers(["# #", " # ", "   "])),
        ("bts", Layers(["   ", "# #", " # "])),
        ("tts", Layers([" # ", "# #", "   "])),
        ("ld", Layers(["## ", "#  ", "   "])),
        ("rd", Layers(["   ", "  #", " ##"])),
        ("lud", Layers(["   ", "#  ", "## "])),
        ("rud", Layers([" ##", "  #", "   "])),
        ("mc", Layers(["   ", " # ", "   "])),
        ("mr", Layers([" # ", "# #", " # "])),
        ("vh", Layers(["## ", "## ", "## "])),
        ("hh", Layers(["###", "###", "   "])),
        ("vhr", Layers([" ##", " ##", " ##"])),
        ("hhb", Layers(["   ", "###", "###"])),
        ("bo", Layers(["###", "# #", "###"])),
        ("cbo", Item(id::VINE, 0)),
        ("cre", Item(id::SKULL, CREEPER_HEAD)),
        ("gra", Layers(["# #", " # ", " # "])),
        ("gru", Layers([" # ", " # ", "# #"])),
        ("bri", Item(id::BRICK_BLOCK, 0)),
        ("sku", Item(id::SKULL, WITHER_SKULL)),
        ("flo", Item(id::RED_FLOWER, OXEYE_DAISY)),
        ("moj", Item(id::GOLDEN_APPLE, NOTCH_APPLE)),
    ]
};

/// Maps scale up to this level (`MapData.scale < 4`).
const MAX_MAP_SCALE: i8 = 4;
/// Written books can be copied up to this generation (copy of a copy).
const MAX_BOOK_GENERATION: i32 = 2;
/// Repairing adds this percentage of the maximum durability.
const REPAIR_BONUS_PERCENT: i32 = 5;

/// Fleece colors by dye damage (`EntitySheep.func_175513_a` of
/// `EnumDyeColor.byDyeDamage`): black (ink sac) first, white (bone meal) last.
const DYE_RGB: [[f32; 3]; 16] = [
    [0.1, 0.1, 0.1],
    [0.6, 0.2, 0.2],
    [0.4, 0.5, 0.2],
    [0.4, 0.3, 0.2],
    [0.2, 0.3, 0.7],
    [0.5, 0.25, 0.7],
    [0.3, 0.5, 0.6],
    [0.6, 0.6, 0.6],
    [0.3, 0.3, 0.3],
    [0.95, 0.5, 0.65],
    [0.5, 0.8, 0.1],
    [0.9, 0.9, 0.2],
    [0.4, 0.6, 0.85],
    [0.7, 0.3, 0.85],
    [0.85, 0.5, 0.2],
    [1.0, 1.0, 1.0],
];

/// A special recipe, by its vanilla class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Special {
    /// `RecipesArmorDyes`.
    ArmorDyes,
    /// `RecipeFireworks`.
    Fireworks,
    /// `RecipeAddPattern`.
    AddPattern,
    /// `RecipeBookCloning`.
    BookCloning,
    /// `RecipesMapCloning`.
    MapCloning,
    /// `RecipesMapExtending`.
    MapExtending,
    /// `RecipeRepairItem`.
    RepairItem,
    /// `RecipeDuplicatePattern`.
    DuplicatePattern,
}

impl Special {
    /// The recipe for a vanilla class name.
    pub fn from_class(name: &str) -> Option<Self> {
        Some(match name {
            "RecipesArmorDyes" => Self::ArmorDyes,
            "RecipeFireworks" => Self::Fireworks,
            "RecipeAddPattern" => Self::AddPattern,
            "RecipeBookCloning" => Self::BookCloning,
            "RecipesMapCloning" => Self::MapCloning,
            "RecipesMapExtending" => Self::MapExtending,
            "RecipeRepairItem" => Self::RepairItem,
            "RecipeDuplicatePattern" => Self::DuplicatePattern,
            _ => return None,
        })
    }

    /// `matches`: whether the recipe claims the grid. `map_scale` looks up a map's
    /// saved scale by map id.
    pub fn matches(
        self,
        grid: &[Option<ItemStack>],
        size: usize,
        map_scale: &dyn Fn(i16) -> Option<i8>,
    ) -> bool {
        let stacks = || grid.iter().flatten();
        match self {
            Self::RepairItem => {
                let items: Vec<&ItemStack> = stacks().collect();
                items.len() == 2 && repairable_pair(items[0], items[1])
            }
            Self::ArmorDyes => {
                let mut armor = 0;
                let mut dyes = 0;
                for s in stacks() {
                    if is_armor(s.id) {
                        if !is_leather(s.id) || armor > 0 {
                            return false;
                        }
                        armor += 1;
                    } else if s.id == id::DYE {
                        dyes += 1;
                    } else {
                        return false;
                    }
                }
                armor == 1 && dyes > 0
            }
            Self::BookCloning => original_and_copies(grid, id::WRITTEN_BOOK, id::WRITABLE_BOOK)
                .is_some_and(|(_, n)| n > 0),
            Self::MapCloning => {
                original_and_copies(grid, id::FILLED_MAP, id::MAP).is_some_and(|(_, n)| n > 0)
            }
            Self::MapExtending => {
                // A shaped recipe: paper around a filled map, on a 3x3 grid.
                let shape_ok = size == 3
                    && grid.iter().enumerate().all(|(k, s)| match (k, s) {
                        (4, Some(s)) => s.id == id::FILLED_MAP,
                        (4, None) => false,
                        (_, Some(s)) => s.id == id::PAPER && s.damage == 0,
                        (_, None) => false,
                    });
                shape_ok
                    && grid
                        .iter()
                        .flatten()
                        .find(|s| s.id == id::FILLED_MAP)
                        .and_then(|m| map_scale(m.damage))
                        .is_some_and(|scale| scale < MAX_MAP_SCALE)
            }
            Self::Fireworks => fireworks(grid).is_some(),
            Self::AddPattern => {
                let mut banner = false;
                for s in stacks().filter(|s| s.id == id::BANNER) {
                    if banner || banner_patterns(s) >= MAX_BANNER_PATTERNS {
                        return false;
                    }
                    banner = true;
                }
                banner && banner_pattern(grid, size).is_some()
            }
            Self::DuplicatePattern => duplicate_pattern(grid),
        }
    }

    /// `getCraftingResult` (vanilla may match and still give nothing).
    pub fn result(self, grid: &[Option<ItemStack>], size: usize) -> Option<ItemStack> {
        match self {
            Self::RepairItem => {
                let items: Vec<&ItemStack> = grid.iter().flatten().collect();
                let [a, b] = items[..] else {
                    return None;
                };
                let max = i32::from(item_data::max_damage(a.id));
                let left = |s: &ItemStack| max - i32::from(s.damage);
                let durability = left(a) + left(b) + max * REPAIR_BONUS_PERCENT / 100;
                let damage = (max - durability).max(0);
                Some(ItemStack {
                    id: a.id,
                    count: 1,
                    damage: damage as i16,
                    nbt: None,
                })
            }
            Self::ArmorDyes => dyed_armor(grid),
            Self::BookCloning => {
                let (book, copies) =
                    original_and_copies(grid, id::WRITTEN_BOOK, id::WRITABLE_BOOK)?;
                let mut tag = book.nbt.clone().unwrap_or_default();
                let generation = tag.get_i32("generation");
                if copies < 1 || generation >= MAX_BOOK_GENERATION {
                    return None;
                }
                tag.insert("generation", Tag::Int(generation + 1));
                Some(ItemStack {
                    id: id::WRITTEN_BOOK,
                    count: copies as i8,
                    damage: 0,
                    nbt: Some(tag),
                })
            }
            Self::MapCloning => {
                let (map, copies) = original_and_copies(grid, id::FILLED_MAP, id::MAP)?;
                Some(ItemStack {
                    id: id::FILLED_MAP,
                    count: copies as i8 + 1,
                    damage: map.damage,
                    nbt: display_name(map).map(|name| {
                        let mut display = Compound::new();
                        display.insert("Name", Tag::String(name));
                        let mut tag = Compound::new();
                        tag.insert("display", Tag::Compound(display));
                        tag
                    }),
                })
            }
            Self::MapExtending => {
                let map = grid.iter().flatten().find(|s| s.id == id::FILLED_MAP)?;
                let mut tag = map.nbt.clone().unwrap_or_default();
                tag.insert("map_is_scaling", Tag::Byte(1));
                Some(ItemStack {
                    count: 1,
                    nbt: Some(tag),
                    ..map.clone()
                })
            }
            Self::Fireworks => fireworks(grid).flatten(),
            Self::AddPattern => add_pattern(grid, size),
            Self::DuplicatePattern => {
                grid.iter()
                    .flatten()
                    .find(|s| banner_patterns(s) > 0)
                    .map(|s| ItemStack {
                        count: 1,
                        ..s.clone()
                    })
            }
        }
    }

    /// `getRemainingItems`: book cloning keeps the original book; the others leave
    /// container items.
    pub fn remaining(self, grid: &[Option<ItemStack>]) -> Vec<Option<ItemStack>> {
        if self == Self::BookCloning {
            let mut out = vec![None; grid.len()];
            if let Some(k) = grid
                .iter()
                .position(|s| s.as_ref().is_some_and(|s| s.id == id::WRITTEN_BOOK))
            {
                out[k] = grid[k].clone();
            }
            return out;
        }
        if self == Self::DuplicatePattern {
            // The patterned banner stays in the grid.
            return grid
                .iter()
                .map(|s| {
                    let s = s.as_ref()?;
                    super::recipes::container_item(s.id).or_else(|| {
                        (s.nbt.is_some() && banner_patterns(s) > 0).then(|| ItemStack {
                            count: 1,
                            ..s.clone()
                        })
                    })
                })
                .collect();
        }
        container_items(grid)
    }
}

/// The default `getRemainingItems`: each ingredient's container item.
pub fn container_items(grid: &[Option<ItemStack>]) -> Vec<Option<ItemStack>> {
    grid.iter()
        .map(|s| {
            s.as_ref()
                .and_then(|s| super::recipes::container_item(s.id))
        })
        .collect()
}

fn is_armor(item: i16) -> bool {
    (id::LEATHER_HELMET..=id::GOLDEN_BOOTS).contains(&item)
}

fn is_leather(item: i16) -> bool {
    (id::LEATHER_HELMET..=id::LEATHER_BOOTS).contains(&item)
}

/// Two single damageable items of the same kind.
fn repairable_pair(a: &ItemStack, b: &ItemStack) -> bool {
    a.id == b.id && a.count == 1 && b.count == 1 && item_data::is_damageable(a.id)
}

/// The original (exactly one) and the number of blanks, if the grid holds only those.
fn original_and_copies(
    grid: &[Option<ItemStack>],
    original: i16,
    blank: i16,
) -> Option<(&ItemStack, i32)> {
    let mut found = None;
    let mut blanks = 0;
    for s in grid.iter().flatten() {
        if s.id == original {
            if found.is_some() {
                return None;
            }
            found = Some(s);
        } else if s.id == blank {
            blanks += 1;
        } else {
            return None;
        }
    }
    found.map(|f| (f, blanks))
}

/// A stack's custom name (`display.Name`).
fn display_name(s: &ItemStack) -> Option<String> {
    let display = s.nbt.as_ref()?.get_compound("display")?;
    display
        .contains_key_of("Name", Some(TagType::String))
        .then(|| display.get_str("Name").to_owned())
}

/// The leather armor color, if dyed (`ItemArmor.hasColor` / `getColor`).
fn leather_color(s: &ItemStack) -> Option<i32> {
    let display = s.nbt.as_ref()?.get_compound("display")?;
    display
        .contains_key_of("color", Some(TagType::Int))
        .then(|| display.get_i32("color"))
}

/// `RecipesArmorDyes.getCraftingResult`: the average of the armor's color and the dyes,
/// scaled back to the brightest component's average.
fn dyed_armor(grid: &[Option<ItemStack>]) -> Option<ItemStack> {
    let mut armor: Option<ItemStack> = None;
    let mut sum = [0i32; 3];
    let mut brightest = 0i32;
    let mut count = 0i32;
    for s in grid.iter().flatten() {
        if is_armor(s.id) {
            if !is_leather(s.id) || armor.is_some() {
                return None;
            }
            let mut copy = s.clone();
            copy.count = 1;
            if let Some(c) = leather_color(s) {
                let rgb = [
                    (c >> 16 & 0xff) as f32 / 255.0,
                    (c >> 8 & 0xff) as f32 / 255.0,
                    (c & 0xff) as f32 / 255.0,
                ];
                let max = rgb[0].max(rgb[1].max(rgb[2]));
                brightest = (brightest as f32 + max * 255.0) as i32;
                for (total, v) in sum.iter_mut().zip(rgb) {
                    *total = (*total as f32 + v * 255.0) as i32;
                }
                count += 1;
            }
            armor = Some(copy);
        } else if s.id == id::DYE {
            let index = usize::try_from(s.damage)
                .ok()
                .filter(|&d| d < DYE_RGB.len())
                .unwrap_or(0);
            let rgb = DYE_RGB[index].map(|v| (v * 255.0) as i32);
            brightest += rgb[0].max(rgb[1].max(rgb[2]));
            for (total, v) in sum.iter_mut().zip(rgb) {
                *total += v;
            }
            count += 1;
        } else {
            return None;
        }
    }
    let mut armor = armor?;
    let avg = sum.map(|v| v / count);
    let gain = brightest as f32 / count as f32;
    let max = avg[0].max(avg[1].max(avg[2])) as f32;
    let [r, g, b] = avg.map(|v| (v as f32 * gain / max) as i32);
    let color = (((r << 8) + g) << 8) + b;
    let tag = armor.nbt.get_or_insert_with(Compound::new);
    if !tag.contains_key_of("display", Some(TagType::Compound)) {
        tag.insert("display", Tag::Compound(Compound::new()));
    }
    if let Some(Tag::Compound(display)) = tag.get_mut("display") {
        display.insert("color", Tag::Int(color));
    }
    Some(armor)
}

/// `RecipeFireworks.matches`, which also builds the result: `None` if it does not
/// match, `Some(None)` if it matches without a result.
fn fireworks(grid: &[Option<ItemStack>]) -> Option<Option<ItemStack>> {
    let (mut paper, mut gunpowder, mut dyes, mut stars, mut effects, mut shapes) =
        (0, 0, 0, 0, 0, 0);
    for s in grid.iter().flatten() {
        match s.id {
            id::GUNPOWDER => gunpowder += 1,
            id::FIREWORK_CHARGE => stars += 1,
            id::DYE => dyes += 1,
            id::PAPER => paper += 1,
            id::GLOWSTONE_DUST | id::DIAMOND => effects += 1,
            id::FIRE_CHARGE | id::FEATHER | id::GOLD_NUGGET | id::SKULL => shapes += 1,
            _ => return None,
        }
    }
    let others = effects + dyes + shapes;
    let stack = |id, nbt| ItemStack {
        id,
        count: 1,
        damage: 0,
        nbt,
    };
    if gunpowder > MAX_FLIGHT || paper > 1 {
        return None;
    }
    if gunpowder >= 1 && paper == 1 && others == 0 {
        // A rocket; without stars it has no tag at all.
        if stars == 0 {
            return Some(Some(stack(id::FIREWORKS, None)));
        }
        let mut explosions = krol_nbt::List::new();
        for s in grid
            .iter()
            .flatten()
            .filter(|s| s.id == id::FIREWORK_CHARGE)
        {
            if let Some(e) = s.nbt.as_ref().and_then(|n| n.get_compound("Explosion")) {
                let _ = explosions.push(Tag::Compound(e.clone()));
            }
        }
        let mut fireworks = Compound::new();
        fireworks.insert("Explosions", Tag::List(explosions));
        fireworks.insert("Flight", Tag::Byte(gunpowder as i8));
        let mut tag = Compound::new();
        tag.insert("Fireworks", Tag::Compound(fireworks));
        return Some(Some(stack(id::FIREWORKS, Some(tag))));
    }
    if gunpowder == 1 && paper == 0 && stars == 0 && dyes > 0 && shapes <= 1 {
        // A star: colors in grid order, effects and shape.
        let mut explosion = Compound::new();
        let mut shape = 0i8;
        let mut colors = Vec::new();
        for s in grid.iter().flatten() {
            match s.id {
                id::DYE => colors.push(DYE_COLORS[(s.damage & 15) as usize]),
                id::GLOWSTONE_DUST => {
                    explosion.insert("Flicker", Tag::Byte(1));
                }
                id::DIAMOND => {
                    explosion.insert("Trail", Tag::Byte(1));
                }
                id::FIRE_CHARGE => shape = SHAPE_LARGE_BALL,
                id::FEATHER => shape = SHAPE_BURST,
                id::GOLD_NUGGET => shape = SHAPE_STAR,
                id::SKULL => shape = SHAPE_CREEPER,
                _ => {}
            }
        }
        explosion.insert("Colors", Tag::IntArray(colors));
        explosion.insert("Type", Tag::Byte(shape));
        let mut tag = Compound::new();
        tag.insert("Explosion", Tag::Compound(explosion));
        return Some(Some(stack(id::FIREWORK_CHARGE, Some(tag))));
    }
    if gunpowder == 0 && paper == 0 && stars == 1 && dyes > 0 && dyes == others {
        // Fade colors for a star, which needs a tag. Vanilla quirk: a tag without
        // `Explosion` still matches, the fade colors going to a detached compound (the
        // star comes out unchanged).
        let colors: Vec<i32> = grid
            .iter()
            .flatten()
            .filter(|s| s.id == id::DYE)
            .map(|s| DYE_COLORS[(s.damage & 15) as usize])
            .collect();
        let star = grid
            .iter()
            .flatten()
            .find(|s| s.id == id::FIREWORK_CHARGE)?;
        let mut result = ItemStack {
            count: 1,
            ..star.clone()
        };
        let tag = result.nbt.as_mut()?;
        if let Some(Tag::Compound(e)) = tag.get_mut("Explosion") {
            e.insert("FadeColors", Tag::IntArray(colors));
        }
        return Some(Some(result));
    }
    None
}

/// `TileEntityBanner.getPatterns`: how many patterns a banner item has.
fn banner_patterns(s: &ItemStack) -> usize {
    s.nbt
        .as_ref()
        .and_then(|n| n.get_compound("BlockEntityTag"))
        .filter(|b| b.contains_key("Patterns"))
        .and_then(|b| b.get_list("Patterns", TagType::Compound))
        .map_or(0, krol_nbt::List::len)
}

/// `TileEntityBanner.getBaseColor`: `Base` from the item's tile entity data, else its
/// damage.
fn banner_base(s: &ItemStack) -> i32 {
    s.nbt
        .as_ref()
        .and_then(|n| n.get_compound("BlockEntityTag"))
        .filter(|b| b.contains_key("Base"))
        .map_or(i32::from(s.damage), |b| b.get_i32("Base"))
}

/// Whether the grid shows a pattern's item recipe: the item (id and damage, like
/// `isItemEqual`) once and at most one dye besides banners.
fn shows_item(grid: &[Option<ItemStack>], item: i16, damage: i16) -> bool {
    let (mut dye, mut found) = (false, false);
    for s in grid.iter().flatten().filter(|s| s.id != id::BANNER) {
        if s.id == id::DYE {
            if dye {
                return false;
            }
            dye = true;
        } else {
            if found || s.id != item || s.damage != damage {
                return false;
            }
            found = true;
        }
    }
    found
}

/// Whether a 3x3 grid shows a pattern's layers: dye of one color on every `#` and
/// nowhere else, banners only on blanks.
fn shows_layers(grid: &[Option<ItemStack>], size: usize, rows: &[&str; 3]) -> bool {
    if size * size != 9 {
        return false;
    }
    let mut color = None;
    for (k, s) in grid.iter().enumerate() {
        let marked = rows[k / 3].as_bytes()[k % 3] != b' ';
        match s {
            Some(s) if s.id != id::BANNER => {
                if s.id != id::DYE || color.is_some_and(|c| c != s.damage) || !marked {
                    return false;
                }
                color = Some(s.damage);
            }
            _ if marked => return false,
            _ => {}
        }
    }
    true
}

/// `RecipeAddPattern.func_179533_c`: the first pattern whose recipe the grid shows.
fn banner_pattern(grid: &[Option<ItemStack>], size: usize) -> Option<&'static str> {
    PATTERNS.iter().find_map(|(pattern, recipe)| {
        let shown = match recipe {
            PatternRecipe::Item(item, damage) => shows_item(grid, *item, *damage),
            PatternRecipe::Layers(rows) => shows_layers(grid, size, rows),
        };
        shown.then_some(*pattern)
    })
}

/// `RecipeAddPattern.getCraftingResult`: the banner with one more pattern, in the color
/// of the first dye (black without one).
fn add_pattern(grid: &[Option<ItemStack>], size: usize) -> Option<ItemStack> {
    let banner = grid.iter().flatten().find(|s| s.id == id::BANNER)?;
    let mut result = ItemStack {
        count: 1,
        ..banner.clone()
    };
    let Some(pattern) = banner_pattern(grid, size) else {
        return Some(result);
    };
    let color = grid
        .iter()
        .flatten()
        .find(|s| s.id == id::DYE)
        .map_or(0, |s| i32::from(s.damage));
    let tag = result.nbt.get_or_insert_with(Compound::new);
    if !matches!(tag.get("BlockEntityTag"), Some(Tag::Compound(_))) {
        tag.insert("BlockEntityTag", Tag::Compound(Compound::new()));
    }
    if let Some(Tag::Compound(block)) = tag.get_mut("BlockEntityTag") {
        if !matches!(block.get("Patterns"), Some(Tag::List(_))) {
            block.insert("Patterns", Tag::List(krol_nbt::List::new()));
        }
        let mut entry = Compound::new();
        entry.insert("Pattern", Tag::String(pattern.to_owned()));
        entry.insert("Color", Tag::Int(color));
        if let Some(Tag::List(list)) = block.get_mut("Patterns") {
            let _ = list.push(Tag::Compound(entry));
        }
    }
    Some(result)
}

/// `RecipeDuplicatePattern.matches`: exactly two banners of the same base color, one
/// with patterns and one without.
fn duplicate_pattern(grid: &[Option<ItemStack>]) -> bool {
    let mut patterned: Option<&ItemStack> = None;
    let mut plain: Option<&ItemStack> = None;
    for s in grid.iter().flatten() {
        if s.id != id::BANNER || (patterned.is_some() && plain.is_some()) {
            return false;
        }
        let base = banner_base(s);
        let has = banner_patterns(s) > 0;
        if let Some(p) = patterned {
            if has || base != banner_base(p) {
                return false;
            }
            plain = Some(s);
        } else if let Some(p) = plain {
            if !has || base != banner_base(p) {
                return false;
            }
            patterned = Some(s);
        } else if has {
            patterned = Some(s);
        } else {
            plain = Some(s);
        }
    }
    patterned.is_some() && plain.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: i16, damage: i16) -> Option<ItemStack> {
        Some(ItemStack {
            id,
            count: 1,
            damage,
            nbt: None,
        })
    }

    #[test]
    fn repairs_like_vanilla() {
        // Two iron pickaxes (250 durability) at 200 and 100 damage: 50 + 150 + 12 left.
        let grid = [s(257, 200), None, None, s(257, 100)];
        assert!(Special::RepairItem.matches(&grid, 2, &|_| None));
        assert_eq!(
            Special::RepairItem.result(&grid, 2).map(|r| r.damage),
            Some(38)
        );
        assert!(!Special::RepairItem.matches(&[s(1, 0), s(1, 0), None, None], 2, &|_| None));
    }

    #[test]
    fn dyes_leather_like_vanilla() {
        // Leather helmet + red dye (damage 1): 153,51,51 stays as is.
        let grid = [s(298, 0), s(351, 1), None, None];
        assert!(Special::ArmorDyes.matches(&grid, 2, &|_| None));
        let out = Special::ArmorDyes
            .result(&grid, 2)
            .and_then(|r| leather_color(&r));
        assert_eq!(out, Some(153 << 16 | 51 << 8 | 51));
    }

    #[test]
    fn clones_maps() {
        let grid = [s(358, 7), s(395, 0), s(395, 0), None];
        assert!(Special::MapCloning.matches(&grid, 2, &|_| None));
        let r = Special::MapCloning.result(&grid, 2);
        assert_eq!(r.map(|r| (r.count, r.damage)), Some((3, 7)));
    }

    #[test]
    fn fireworks_rockets_and_stars() {
        let g = |ids: &[(i16, i16)]| -> Vec<Option<ItemStack>> {
            let mut v: Vec<Option<ItemStack>> = ids.iter().map(|&(i, d)| s(i, d)).collect();
            v.resize(9, None);
            v
        };
        // Gunpowder and paper: a rocket without any tag.
        let grid = g(&[(id::GUNPOWDER, 0), (id::PAPER, 0)]);
        assert!(Special::Fireworks.matches(&grid, 3, &|_| None));
        let rocket = Special::Fireworks.result(&grid, 3).expect("rocket");
        assert_eq!(
            (rocket.id, rocket.count, rocket.nbt.is_none()),
            (id::FIREWORKS, 1, true)
        );
        // A star: red, flicker, star shape.
        let grid = g(&[
            (id::GUNPOWDER, 0),
            (id::DYE, 1),
            (id::GLOWSTONE_DUST, 0),
            (id::GOLD_NUGGET, 0),
        ]);
        let star = Special::Fireworks.result(&grid, 3).expect("star");
        let e = star
            .nbt
            .as_ref()
            .and_then(|n| n.get_compound("Explosion"))
            .expect("explosion");
        assert_eq!(e.get_int_array("Colors"), &[11743532]);
        assert!(e.get_bool("Flicker"));
        assert_eq!(e.get_i8("Type"), SHAPE_STAR);
        // Two shapes, or a fourth gunpowder, do not match.
        let grid = g(&[
            (id::GUNPOWDER, 0),
            (id::DYE, 1),
            (id::FEATHER, 0),
            (id::GOLD_NUGGET, 0),
        ]);
        assert!(!Special::Fireworks.matches(&grid, 3, &|_| None));
        // The star in a rocket with two gunpowder.
        let mut grid = g(&[(id::GUNPOWDER, 0), (id::GUNPOWDER, 0), (id::PAPER, 0)]);
        grid[3] = Some(star.clone());
        let rocket = Special::Fireworks.result(&grid, 3).expect("rocket");
        let f = rocket
            .nbt
            .as_ref()
            .and_then(|n| n.get_compound("Fireworks"))
            .expect("tag");
        assert_eq!(f.get_i8("Flight"), 2);
        assert_eq!(
            f.get_list("Explosions", TagType::Compound)
                .map(krol_nbt::List::len),
            Some(1)
        );
        // Fading to white.
        let mut grid = g(&[(id::DYE, 15)]);
        grid[1] = Some(star);
        let faded = Special::Fireworks.result(&grid, 3).expect("faded");
        let e = faded
            .nbt
            .as_ref()
            .and_then(|n| n.get_compound("Explosion"))
            .expect("explosion");
        assert_eq!(e.get_int_array("FadeColors"), &[15790320]);
    }

    #[test]
    fn banner_patterns_are_added_and_copied() {
        // Red dye across the top row of a 3x3 grid, a white banner below: stripe top.
        let mut grid = vec![None; 9];
        for slot in &mut grid[0..3] {
            *slot = s(id::DYE, 1);
        }
        grid[4] = s(id::BANNER, 15);
        assert!(Special::AddPattern.matches(&grid, 3, &|_| None));
        let banner = Special::AddPattern.result(&grid, 3).expect("banner");
        assert_eq!(banner_patterns(&banner), 1);
        let p = banner
            .nbt
            .as_ref()
            .and_then(|n| n.get_compound("BlockEntityTag"))
            .and_then(|b| b.get_list("Patterns", TagType::Compound))
            .and_then(|l| l.get(0).cloned());
        let Some(Tag::Compound(p)) = p else {
            panic!("no pattern");
        };
        assert_eq!((p.get_str("Pattern"), p.get_i32("Color")), ("ts", 1));
        // Layers need a 3x3 grid; vine works anywhere, black without a dye.
        let grid = [s(id::BANNER, 15), s(id::VINE, 0), None, None];
        let b = Special::AddPattern.result(&grid, 2).expect("banner");
        assert_eq!(banner_patterns(&b), 1);
        // Copying the pattern onto a plain banner of the same color keeps the original.
        let grid = [Some(banner.clone()), s(id::BANNER, 15), None, None];
        assert!(Special::DuplicatePattern.matches(&grid, 2, &|_| None));
        let copy = Special::DuplicatePattern.result(&grid, 2).expect("copy");
        assert_eq!(banner_patterns(&copy), 1);
        let left = Special::DuplicatePattern.remaining(&grid);
        assert!(left[0].is_some() && left[1].is_none());
        // Different base colors do not match.
        let grid = [Some(banner), s(id::BANNER, 1), None, None];
        assert!(!Special::DuplicatePattern.matches(&grid, 2, &|_| None));
    }
}
