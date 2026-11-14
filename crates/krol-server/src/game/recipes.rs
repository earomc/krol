//! Crafting and smelting recipes (vanilla: `CraftingManager`, `ShapedRecipes`,
//! `ShapelessRecipes`, `FurnaceRecipes`).
//!
//! The recipe list is data extracted from vanilla 1.8.8 by
//! `tools/datagen/RecipeDump.java` (`data/recipes.txt`), in vanilla's sorted order: the
//! first recipe that matches wins.
//!
//! TODO(M4): the special recipes (armor dyes, fireworks, banners, book and map cloning,
//! map extending, item repair).

use std::sync::OnceLock;

use krol_protocol::packets::play::ItemStack;

/// Damage value meaning "any" in recipe ingredients.
const ANY: i16 = 32767;

/// An ingredient: item id and damage (or [`ANY`]).
type Ingredient = Option<(i16, i16)>;

/// One crafting recipe.
#[derive(Debug, Clone)]
enum Recipe {
    Shaped {
        width: usize,
        height: usize,
        items: Vec<Ingredient>,
        output: (i16, i8, i16),
    },
    Shapeless {
        items: Vec<(i16, i16)>,
        output: (i16, i8, i16),
    },
    Special,
}

/// A smelting recipe: input, output, experience.
#[derive(Debug, Clone)]
pub struct Smelting {
    /// Input item and damage (or any).
    pub input: (i16, i16),
    /// Output item, count and damage.
    pub output: (i16, i8, i16),
    /// Experience per item.
    pub xp: f32,
}

struct Recipes {
    crafting: Vec<Recipe>,
    smelting: Vec<Smelting>,
}

fn parse_ingredient(s: &str) -> Ingredient {
    let (id, damage) = s.split_once(':')?;
    Some((id.parse().ok()?, damage.parse().ok()?))
}

fn recipes() -> &'static Recipes {
    static RECIPES: OnceLock<Recipes> = OnceLock::new();
    RECIPES.get_or_init(|| {
        let mut crafting = Vec::new();
        let mut smelting = Vec::new();
        for line in include_str!("../../data/recipes.txt").lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            let num = |k: usize| f.get(k).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
            match f.first().copied() {
                Some("shaped") => crafting.push(Recipe::Shaped {
                    width: num(1) as usize,
                    height: num(2) as usize,
                    output: (num(3) as i16, num(4) as i8, num(5) as i16),
                    items: f[6..].iter().map(|s| parse_ingredient(s)).collect(),
                }),
                Some("shapeless") => crafting.push(Recipe::Shapeless {
                    output: (num(1) as i16, num(2) as i8, num(3) as i16),
                    items: f[4..].iter().filter_map(|s| parse_ingredient(s)).collect(),
                }),
                Some("special") => crafting.push(Recipe::Special),
                Some("smelt") => {
                    if let Some(input) = f.get(1).and_then(|s| parse_ingredient(s)) {
                        smelting.push(Smelting {
                            input,
                            output: (num(2) as i16, num(3) as i8, num(4) as i16),
                            xp: f.get(5).and_then(|v| v.parse().ok()).unwrap_or(0.0),
                        });
                    }
                }
                _ => {}
            }
        }
        Recipes { crafting, smelting }
    })
}

fn matches(ingredient: (i16, i16), stack: &ItemStack) -> bool {
    stack.id == ingredient.0 && (ingredient.1 == ANY || stack.damage == ingredient.1)
}

/// `ShapedRecipes.checkMatch` at an offset, mirrored or not, on a `size` x `size` grid
/// (vanilla always tries offsets as if the grid were 3 wide).
fn check_shaped(
    grid: &[Option<ItemStack>],
    size: usize,
    (w, h, items): (usize, usize, &[Ingredient]),
    (dx, dy): (usize, usize),
    mirrored: bool,
) -> bool {
    for col in 0..3usize {
        for row in 0..3usize {
            let want = match (col.checked_sub(dx), row.checked_sub(dy)) {
                (Some(m), Some(n)) if m < w && n < h => {
                    if mirrored {
                        items[w - m - 1 + n * w]
                    } else {
                        items[m + n * w]
                    }
                }
                _ => None,
            };
            let have = (col < size && row < size)
                .then(|| grid[col + row * size].as_ref())
                .flatten();
            match (want, have) {
                (None, None) => {}
                (Some(i), Some(s)) if matches(i, s) => {}
                _ => return false,
            }
        }
    }
    true
}

/// `CraftingManager.findMatchingRecipe` on a square crafting grid (2 or 3 wide).
pub fn craft(grid: &[Option<ItemStack>], size: usize) -> Option<ItemStack> {
    for recipe in &recipes().crafting {
        let output = match recipe {
            Recipe::Shaped {
                width,
                height,
                items,
                output,
            } => {
                let mut found = false;
                'outer: for dx in 0..=3usize.saturating_sub(*width) {
                    for dy in 0..=3usize.saturating_sub(*height) {
                        let shape = (*width, *height, items.as_slice());
                        if check_shaped(grid, size, shape, (dx, dy), true)
                            || check_shaped(grid, size, shape, (dx, dy), false)
                        {
                            found = true;
                            break 'outer;
                        }
                    }
                }
                found.then_some(*output)
            }
            Recipe::Shapeless { items, output } => {
                let mut left: Vec<(i16, i16)> = items.clone();
                let mut ok = true;
                for stack in grid.iter().flatten() {
                    match left.iter().position(|&i| matches(i, stack)) {
                        Some(k) => {
                            left.remove(k);
                        }
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                (ok && left.is_empty()).then_some(*output)
            }
            Recipe::Special => None,
        };
        if let Some((id, count, damage)) = output {
            return Some(ItemStack {
                id,
                count,
                damage,
                nbt: None,
            });
        }
    }
    None
}

/// What an ingredient leaves in the grid (`Item.getContainerItem`): buckets.
pub fn container_item(id: i16) -> Option<ItemStack> {
    use super::ids::item;
    matches!(
        id,
        item::WATER_BUCKET | item::LAVA_BUCKET | item::MILK_BUCKET
    )
    .then_some(ItemStack {
        id: item::BUCKET,
        count: 1,
        damage: 0,
        nbt: None,
    })
}

/// The experience per item for a smelting output (`getSmeltingExperience`).
pub fn smelting_xp(output: &ItemStack) -> f32 {
    recipes()
        .smelting
        .iter()
        .find(|r| r.output.0 == output.id && (r.output.2 == ANY || r.output.2 == output.damage))
        .map_or(0.0, |r| r.xp)
}

/// The smelting recipe for an input (`FurnaceRecipes.getSmeltingResult`).
pub fn smelting(stack: &ItemStack) -> Option<&'static Smelting> {
    recipes().smelting.iter().find(|r| matches(r.input, stack))
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
    fn crafts_like_vanilla() {
        // Planks from a birch log (2x2 grid, any position).
        let grid = [None, s(17, 2), None, None];
        assert_eq!(
            craft(&grid, 2).map(|o| (o.id, o.count, o.damage)),
            Some((5, 4, 2))
        );
        // A crafting table from four planks.
        let grid = [s(5, 0), s(5, 1), s(5, 3), s(5, 0)];
        assert_eq!(craft(&grid, 2).map(|o| o.id), Some(58));
        // A wooden pickaxe on a 3x3 grid.
        let p = s(5, 0);
        let st = s(280, 0);
        let grid = [
            p.clone(),
            p.clone(),
            p,
            None,
            st.clone(),
            None,
            None,
            st,
            None,
        ];
        assert_eq!(craft(&grid, 3).map(|o| o.id), Some(270));
        assert_eq!(
            craft(&[s(1, 0), None, None, None], 2).map(|o| o.id),
            Some(77)
        );
        assert_eq!(craft(&[s(3, 0), None, None, None], 2), None);
    }
}
