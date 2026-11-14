//! Eating (vanilla `ItemFood`, `ItemSoup` and `EntityPlayer`'s item in use): food is
//! held for 32 updates, with eating sounds, then restores hunger and saturation.
//! Food values come from `tools/datagen/FoodDump.java` (`data/foods.txt`).

use std::collections::HashMap;
use std::sync::OnceLock;

use krol_nbt::Tag;
use krol_protocol::packets::play::{ClientboundPlay, ItemStack};

use super::Game;

/// `ItemFood.getMaxItemUseDuration`.
pub const EAT_DURATION: i32 = 32;
/// Eating sounds play every 4 updates from this many left.
const EAT_SOUNDS_FROM: i32 = 25;
/// Crumb particles per eating sound, and when finishing.
const EAT_PARTICLES: usize = 5;
const FINISH_PARTICLES: usize = 16;
/// The hunger bar is full at this level.
const MAX_FOOD: i32 = 20;
/// `EntityStatus` 9: the player finished using an item.
const STATUS_USE_FINISHED: i8 = 9;
/// `random.drink` volume.
const DRINK_VOLUME: f32 = 0.5;
/// `golden_apple`.
const GOLDEN_APPLE: i16 = 322;
/// `fish` (raw); damage 3 is the pufferfish.
const FISH: i16 = 349;
const PUFFERFISH: i16 = 3;

/// The `Animation` packet's "eat food".
const ANIMATION_EAT: u8 = 3;
/// `random.burp` volume.
const BURP_VOLUME: f32 = 0.5;

/// A food's values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Food {
    /// `getHealAmount`: hunger restored.
    pub hunger: i32,
    /// `getSaturationModifier`.
    pub saturation: f32,
    /// `alwaysEdible`: edible with a full hunger bar.
    pub always_edible: bool,
    /// What is left in the hand (`ItemSoup`: a bowl), or 0.
    pub leftover: i16,
}

fn table() -> &'static HashMap<(i16, i16), Food> {
    static TABLE: OnceLock<HashMap<(i16, i16), Food>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = HashMap::new();
        for line in include_str!("../../data/foods.txt").lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if let [id, damage, hunger, sat, always, _effects, leftover] = f[..]
                && let (Ok(id), Ok(damage), Ok(hunger), Ok(sat), Ok(leftover)) = (
                    id.parse(),
                    damage.parse(),
                    hunger.parse(),
                    u32::from_str_radix(sat, 16),
                    leftover.parse(),
                )
            {
                t.insert(
                    (id, damage),
                    Food {
                        hunger,
                        saturation: f32::from_bits(sat),
                        always_edible: always == "1",
                        leftover,
                    },
                );
            }
        }
        t
    })
}

/// The food values of an item, if it is food (subtypes by damage, e.g. fish).
pub fn food(id: i16, damage: i16) -> Option<Food> {
    let t = table();
    t.get(&(id, damage)).or_else(|| t.get(&(id, 0))).copied()
}

/// An item being used (`itemInUse`, `itemInUseCount`). Vanilla keeps the stack object
/// and stops when the held stack is another object; krol compares the slot, id and
/// damage.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemUse {
    /// The hotbar window slot.
    pub slot: usize,
    /// The item id.
    pub id: i16,
    /// The item damage.
    pub damage: i16,
    /// Updates left.
    pub count: i32,
    /// Drunk (milk) rather than eaten.
    pub drink: bool,
}

impl Game {
    /// `EntityPlayer.canEat`: hungry (or always edible) and taking damage.
    fn can_eat(&self, i: usize, always: bool) -> bool {
        let p = &self.players[i];
        (always || p.data.get_i32("foodLevel") < MAX_FOOD) && !matches!(p.gamemode, 1 | 3)
    }

    /// `ItemFood.onItemRightClick`: starts eating if the player can (`setItemInUse`).
    /// Returns whether the player now uses the item.
    pub(super) fn start_eating(&mut self, i: usize, held: &ItemStack, f: Food) -> bool {
        if self.can_eat(i, f.always_edible) {
            let p = &mut self.players[i];
            let slot = 36 + p.selected;
            let same = p
                .using
                .as_ref()
                .is_some_and(|u| u.slot == slot && u.id == held.id && u.damage == held.damage);
            if !same {
                p.using = Some(ItemUse {
                    slot,
                    id: held.id,
                    damage: held.damage,
                    count: EAT_DURATION,
                    drink: false,
                });
                p.dirty_metadata.insert(0);
            }
            // EntityPlayerMP.setItemInUse: the eating animation, to the player and its
            // viewers (every time, even for the stack already in use).
            let entity_id = p.entity_id;
            let animation = ClientboundPlay::Animation {
                entity_id,
                animation: ANIMATION_EAT,
            };
            p.send(animation.clone());
            self.send_to_tracking(entity_id, &animation);
        }
        self.players[i].using.is_some()
    }

    /// `ItemBucketMilk.onItemRightClick`: milk is drunk in any game mode.
    pub(super) fn start_drinking_milk(&mut self, i: usize, held: &ItemStack) {
        let p = &mut self.players[i];
        let slot = 36 + p.selected;
        let same = p
            .using
            .as_ref()
            .is_some_and(|u| u.slot == slot && u.id == held.id && u.damage == held.damage);
        if !same {
            p.using = Some(ItemUse {
                slot,
                id: held.id,
                damage: held.damage,
                count: EAT_DURATION,
                drink: true,
            });
            p.dirty_metadata.insert(0);
        }
    }

    /// `EntityPlayer.clearItemInUse`.
    pub(super) fn clear_item_in_use(&mut self, i: usize) {
        let p = &mut self.players[i];
        if p.using.take().is_some() {
            p.dirty_metadata.insert(0);
        }
    }

    /// The item-in-use part of `EntityPlayer.onUpdate` (once per movement packet):
    /// eating sounds, then finishing; a different held item stops the use.
    pub(super) fn item_use_update(&mut self, i: usize) {
        let p = &self.players[i];
        let Some(u) = p.using.clone() else {
            return;
        };
        let current = p.inventory[36 + p.selected].as_ref();
        let still = 36 + p.selected == u.slot
            && current.is_some_and(|s| s.id == u.id && s.damage == u.damage && s.count > 0);
        if !still {
            self.clear_item_in_use(i);
            return;
        }
        if u.count <= EAT_SOUNDS_FROM && u.count % 4 == 0 {
            self.use_effects(i, u.drink, EAT_PARTICLES);
        }
        let left = u.count - 1;
        if let Some(use_) = self.players[i].using.as_mut() {
            use_.count = left;
        }
        if left == 0 {
            self.finish_item_use(i);
        }
    }

    /// `EntityPlayer.updateItemUse`: the drinking sound, or crumbs (client-side, from
    /// the player's random) and the eating sound, for the others.
    fn use_effects(&mut self, i: usize, drink: bool, particles: usize) {
        if drink {
            let pitch = self.world_rand.next_float() * 0.1 + 0.9;
            let p = &self.players[i];
            let (pos, conn) = (p.pos, p.conn_id);
            self.play_sound_near_except(Some(conn), "random.drink", pos, DRINK_VOLUME, pitch);
            return;
        }
        let p = &mut self.players[i];
        for _ in 0..particles * 3 {
            p.rand.next_float();
        }
        let volume = 0.5 + 0.5 * p.rand.next_int_bounded(2) as f32;
        let pitch = (p.rand.next_float() - p.rand.next_float()) * 0.2 + 1.0;
        let (pos, conn) = (p.pos, p.conn_id);
        self.play_sound_near_except(Some(conn), "random.eat", pos, volume, pitch);
    }

    /// `EntityPlayerMP.onItemUseFinish` + `EntityPlayer.onItemUseFinish` +
    /// `ItemFood.onItemUseFinish`: status 9, the last crumbs, then the food is eaten.
    fn finish_item_use(&mut self, i: usize) {
        let p = &self.players[i];
        p.send(ClientboundPlay::EntityStatus {
            entity_id: p.entity_id,
            status: STATUS_USE_FINISHED,
        });
        let drink = self.players[i].using.as_ref().is_some_and(|u| u.drink);
        self.use_effects(i, drink, FINISH_PARTICLES);
        let p = &mut self.players[i];
        let slot = 36 + p.selected;
        let Some(stack) = p.inventory[slot].clone() else {
            self.clear_item_in_use(i);
            return;
        };
        if stack.id == super::ids::item::MILK_BUCKET {
            // ItemBucketMilk.onItemUseFinish.
            let mut milk = stack.clone();
            if p.gamemode != 1 {
                milk.count -= 1;
            }
            self.clear_effects(i);
            if let Some(stat) = super::digging::use_stat(stack.id) {
                self.players[i].stats.add(&stat, 1);
            }
            let result = if milk.count <= 0 {
                ItemStack {
                    id: super::ids::item::BUCKET,
                    count: 1,
                    damage: 0,
                    nbt: None,
                }
            } else {
                milk
            };
            self.players[i].inventory[slot] = Some(result);
            self.clear_item_in_use(i);
            return;
        }
        let Some(f) = food(stack.id, stack.damage) else {
            self.clear_item_in_use(i);
            return;
        };
        let mut eaten = stack.clone();
        eaten.count -= 1;
        // FoodStats.addStats.
        let d = &mut p.data;
        let level = (f.hunger + d.get_i32("foodLevel")).min(MAX_FOOD);
        let saturation = (d.get_f32("foodSaturationLevel") + f.hunger as f32 * f.saturation * 2.0)
            .min(level as f32);
        d.insert("foodLevel", Tag::Int(level));
        d.insert("foodSaturationLevel", Tag::Float(saturation));
        let pitch = self.world_rand.next_float() * 0.1 + 0.9;
        let pos = self.players[i].pos;
        self.play_sound_near_except(None, "random.burp", pos, BURP_VOLUME, pitch);
        self.food_eaten(i, stack.id, stack.damage);
        if let Some(stat) = super::digging::use_stat(stack.id) {
            self.players[i].stats.add(&stat, 1);
        }
        let result = if f.leftover != 0 {
            Some(ItemStack {
                id: f.leftover,
                count: 1,
                damage: 0,
                nbt: None,
            })
        } else {
            Some(eaten)
        };
        let p = &mut self.players[i];
        p.inventory[slot] = result.filter(|s| s.count > 0);
        self.clear_item_in_use(i);
    }

    /// `ItemFood.onFoodEaten` and its overrides (`ItemAppleGold`, `ItemFishFood`).
    fn food_eaten(&mut self, i: usize, id: i16, damage: i16) {
        use super::potions::{Effect, effect};
        let fx = |id: u8, duration: i32, amplifier: u8| Effect {
            id,
            amplifier,
            duration,
            ambient: false,
            particles: true,
        };
        // setPotionEffect(effect, seconds, amplifier, probability).
        let chance = |game: &mut Self, e: Effect, probability: f32| {
            if game.world_rand.next_float() < probability {
                game.add_effect(i, e);
            }
        };
        match (id, damage) {
            (GOLDEN_APPLE, _) => {
                self.add_effect(i, fx(effect::ABSORPTION, 2400, 0));
                if damage > 0 {
                    self.add_effect(i, fx(effect::REGENERATION, 600, 4));
                    self.add_effect(i, fx(effect::RESISTANCE, 6000, 0));
                    self.add_effect(i, fx(effect::FIRE_RESISTANCE, 6000, 0));
                } else {
                    chance(self, fx(effect::REGENERATION, 5 * 20, 1), 1.0);
                }
            }
            (FISH, PUFFERFISH) => {
                self.add_effect(i, fx(effect::POISON, 1200, 3));
                self.add_effect(i, fx(effect::HUNGER, 300, 2));
                self.add_effect(i, fx(effect::NAUSEA, 300, 1));
            }
            // Raw chicken and rotten flesh: hunger; spider eye and poisonous potato:
            // poison.
            (365, _) => chance(self, fx(effect::HUNGER, 30 * 20, 0), 0.3),
            (367, _) => chance(self, fx(effect::HUNGER, 30 * 20, 0), 0.8),
            (375, _) => chance(self, fx(effect::POISON, 5 * 20, 0), 1.0),
            (394, _) => chance(self, fx(effect::POISON, 5 * 20, 0), 0.6),
            _ => {}
        }
    }
}
