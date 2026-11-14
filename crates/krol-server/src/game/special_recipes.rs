//! Crafting recipes vanilla computes in code rather than from a pattern (`IRecipe`
//! implementations: `RecipeRepairItem`, `RecipesArmorDyes`, `RecipeBookCloning`,
//! `RecipesMapCloning`, `RecipesMapExtending`).
//!
//! TODO(M4): fireworks (`RecipeFireworks`) and banner patterns (`RecipeAddPattern`,
//! `RecipeDuplicatePattern`) never match yet.

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
}

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
            Self::Fireworks | Self::AddPattern | Self::DuplicatePattern => false,
        }
    }

    /// `getCraftingResult` (vanilla may match and still give nothing).
    pub fn result(self, grid: &[Option<ItemStack>]) -> Option<ItemStack> {
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
            Self::Fireworks | Self::AddPattern | Self::DuplicatePattern => None,
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
            Special::RepairItem.result(&grid).map(|r| r.damage),
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
            .result(&grid)
            .and_then(|r| leather_color(&r));
        assert_eq!(out, Some(153 << 16 | 51 << 8 | 51));
    }

    #[test]
    fn clones_maps() {
        let grid = [s(358, 7), s(395, 0), s(395, 0), None];
        assert!(Special::MapCloning.matches(&grid, 2, &|_| None));
        let r = Special::MapCloning.result(&grid);
        assert_eq!(r.map(|r| (r.count, r.damage)), Some((3, 7)));
    }
}
