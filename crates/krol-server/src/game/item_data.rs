//! Per-item properties of vanilla 1.8.8 (`Item.getItemStackLimit`, `getMaxDamage`,
//! `getHasSubtypes`, `getContainerItem`), extracted by `tools/datagen/ItemDump.java`
//! into `data/items.txt`.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Stack size of an id vanilla does not know.
const DEFAULT_STACK: i8 = 64;

/// One item's properties.
#[derive(Debug, Clone, Copy)]
struct ItemInfo {
    max_stack: i8,
    max_damage: i16,
    has_subtypes: bool,
    container: Option<i16>,
}

fn table() -> &'static HashMap<i16, ItemInfo> {
    static TABLE: OnceLock<HashMap<i16, ItemInfo>> = OnceLock::new();
    TABLE.get_or_init(|| {
        include_str!("../../data/items.txt")
            .lines()
            .filter_map(|line| {
                let f: Vec<i32> = line
                    .split_whitespace()
                    .filter_map(|v| v.parse().ok())
                    .collect();
                let [id, max_stack, max_damage, subtypes, container] = f[..] else {
                    return None;
                };
                Some((
                    id as i16,
                    ItemInfo {
                        max_stack: max_stack as i8,
                        max_damage: max_damage as i16,
                        has_subtypes: subtypes != 0,
                        container: (container != 0).then_some(container as i16),
                    },
                ))
            })
            .collect()
    })
}

/// `Item.getItemStackLimit`.
pub fn max_stack(id: i16) -> i8 {
    table().get(&id).map_or(DEFAULT_STACK, |i| i.max_stack)
}

/// `Item.getMaxDamage` (0 for items without durability).
pub fn max_damage(id: i16) -> i16 {
    table().get(&id).map_or(0, |i| i.max_damage)
}

/// `Item.getHasSubtypes`: the damage value names a variant rather than wear.
pub fn has_subtypes(id: i16) -> bool {
    table().get(&id).is_some_and(|i| i.has_subtypes)
}

/// `Item.isDamageable`: has durability.
pub fn is_damageable(id: i16) -> bool {
    max_damage(id) > 0 && !has_subtypes(id)
}

/// `Item.getContainerItem`: what stays behind when the item is used up in crafting or
/// as fuel (buckets).
pub fn container_item(id: i16) -> Option<i16> {
    table().get(&id).and_then(|i| i.container)
}

/// `ItemArmor.damageReduceAmount`: the armor points of an armor piece (0 for other
/// items). Helmet, chestplate, leggings and boots of leather, chain, iron, diamond and
/// gold are items 298 to 317.
pub fn armor_points(id: i16) -> i32 {
    const POINTS: [[i32; 4]; 5] = [
        [1, 3, 2, 1], // leather
        [2, 5, 4, 1], // chain
        [2, 6, 5, 2], // iron
        [3, 8, 6, 3], // diamond
        [2, 5, 3, 1], // gold
    ];
    if !(298..=317).contains(&id) {
        return 0;
    }
    let k = (id - 298) as usize;
    POINTS[k / 4][k % 4]
}

/// `Item.getMaxItemUseDuration`: how long right-clicking uses the item (0: not used
/// over time).
pub fn use_duration(id: i16, damage: i16) -> i32 {
    const BLOCK_OR_DRAW: i32 = 72000;
    match id {
        _ if super::food::food(id, damage).is_some() => super::food::EAT_DURATION,
        // Potions and milk are drunk like food is eaten.
        373 | 335 => super::food::EAT_DURATION,
        // Bows and swords (blocking).
        261 | 267 | 268 | 272 | 276 | 283 => BLOCK_OR_DRAW,
        _ => 0,
    }
}

/// `Enchantment.protection`'s id; fire, feather falling, blast and projectile
/// protection follow (`EnchantmentProtection.protectionType` 0 to 4).
pub const PROTECTION: i16 = 0;
/// `Enchantment.featherFalling`'s id.
pub const FEATHER_FALLING: i16 = 2;

/// `Enchantment.fireProtection`'s id.
pub const FIRE_PROTECTION: i16 = 1;

/// `Enchantment.unbreaking`'s id.
pub const UNBREAKING: i16 = 34;

/// `Enchantment.respiration`'s id.
pub const RESPIRATION: i16 = 5;

/// The `(id, level)` pairs of a stack's enchantments, in order.
pub fn enchantments(stack: &krol_protocol::packets::play::ItemStack) -> Vec<(i16, i32)> {
    let Some(list) = stack
        .nbt
        .as_ref()
        .and_then(|n| n.get_list("ench", krol_nbt::TagType::Compound))
    else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|e| match e {
            krol_nbt::Tag::Compound(c) => Some((c.get_i16("id"), i32::from(c.get_i16("lvl")))),
            _ => None,
        })
        .collect()
}

/// The highest level of enchantment `id` on the armor pieces
/// (`EnchantmentHelper.getMaxEnchantmentLevel(id, player.getInventory())`).
pub fn armor_enchantment(
    inventory: &[Option<krol_protocol::packets::play::ItemStack>],
    id: i16,
) -> i32 {
    [5usize, 6, 7, 8]
        .iter()
        .map(|&s| enchantment_level(inventory[s].as_ref(), id))
        .max()
        .unwrap_or(0)
}

/// `EnchantmentHelper.getEnchantmentLevel`: the level of enchantment `id` on a stack.
pub fn enchantment_level(stack: Option<&krol_protocol::packets::play::ItemStack>, id: i16) -> i32 {
    let Some(list) = stack
        .and_then(|s| s.nbt.as_ref())
        .and_then(|n| n.get_list("ench", krol_nbt::TagType::Compound))
    else {
        return 0;
    };
    for e in list.iter() {
        if let krol_nbt::Tag::Compound(c) = e
            && c.get_i16("id") == id
        {
            return i32::from(c.get_i16("lvl"));
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_values() {
        assert_eq!(max_stack(1), 64);
        assert_eq!(max_stack(323), 16);
        assert_eq!(max_stack(325), 16);
        assert_eq!(max_stack(326), 1);
        assert_eq!(max_damage(276), 1561);
        assert!(is_damageable(276));
        assert!(!is_damageable(373));
        assert!(has_subtypes(35));
        assert_eq!(container_item(327), Some(325));
        assert_eq!(container_item(1), None);
    }
}
