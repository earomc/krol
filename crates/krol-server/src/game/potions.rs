//! Status effects (vanilla `Potion`, `PotionEffect` and `EntityLivingBase`'s active
//! effects) on players. Effect properties come from `tools/datagen/PotionDump.java`
//! (`data/potions.txt`).

use std::sync::OnceLock;

use krol_nbt::{Compound, List, Tag, TagType};
use krol_protocol::packets::play::ClientboundPlay;

use super::Game;
use super::damage::DamageSource;

/// Effect ids in the table (`Potion.potionTypes` has 32 slots).
const IDS: usize = 32;
/// Metadata 7 of an entity with no effects shown (`calcPotionLiquidColor`'s default is
/// only used for potion items).
const NO_COLOR: i32 = 0;
/// `updatePotionEffects` resends an effect every 30 s.
const RESEND_PERIOD: i32 = 600;

/// Effect ids (`Potion.*.id`).
pub mod effect {
    /// `moveSpeed`.
    pub const SPEED: u8 = 1;
    /// `moveSlowdown`.
    pub const SLOWNESS: u8 = 2;
    /// `damageBoost`.
    pub const STRENGTH: u8 = 5;
    /// `weakness`.
    pub const WEAKNESS: u8 = 18;
    /// `healthBoost`.
    pub const HEALTH_BOOST: u8 = 21;
    /// `regeneration`.
    pub const REGENERATION: u8 = 10;
    /// `confusion` (nausea).
    pub const NAUSEA: u8 = 9;
    /// `resistance`.
    pub const RESISTANCE: u8 = 11;
    /// `fireResistance`.
    pub const FIRE_RESISTANCE: u8 = 12;
    /// `heal` (instant health).
    pub const HEAL: u8 = 6;
    /// `harm` (instant damage).
    pub const HARM: u8 = 7;
    /// `hunger`.
    pub const HUNGER: u8 = 17;
    /// `poison`.
    pub const POISON: u8 = 19;
    /// `wither`.
    pub const WITHER: u8 = 20;
    /// `absorption`.
    pub const ABSORPTION: u8 = 22;
    /// `saturation`.
    pub const SATURATION: u8 = 23;
}

/// One active effect (`PotionEffect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effect {
    /// The effect id.
    pub id: u8,
    /// Level minus one.
    pub amplifier: u8,
    /// Ticks left.
    pub duration: i32,
    /// From a beacon (fewer particles).
    pub ambient: bool,
    /// Whether particles show (and the effect counts for the particle color).
    pub particles: bool,
}

/// An effect's static properties.
#[derive(Debug, Clone, Copy)]
pub struct EffectType {
    /// `getLiquidColor`.
    pub color: i32,
    /// `isInstant`.
    pub instant: bool,
    /// The translation key (`getName`).
    pub name: &'static str,
}

fn table() -> &'static [Option<EffectType>; IDS] {
    static TABLE: OnceLock<[Option<EffectType>; IDS]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [None; IDS];
        for line in include_str!("../../data/potions.txt").lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if let [id, color, instant, _bad, name] = f[..]
                && let (Ok(id), Ok(color)) = (id.parse::<usize>(), i32::from_str_radix(color, 16))
                && id < IDS
            {
                t[id] = Some(EffectType {
                    color,
                    instant: instant == "1",
                    name,
                });
            }
        }
        t
    })
}

/// The properties of an effect id, if it exists.
pub fn effect_type(id: i32) -> Option<EffectType> {
    usize::try_from(id)
        .ok()
        .and_then(|i| table().get(i).copied().flatten())
}

/// The registry names of the effects (`Potion.getPotionFromResourceLocation`).
pub fn effect_by_name(name: &str) -> Option<u8> {
    const NAMES: [&str; 23] = [
        "speed",
        "slowness",
        "haste",
        "mining_fatigue",
        "strength",
        "instant_health",
        "instant_damage",
        "jump_boost",
        "nausea",
        "regeneration",
        "resistance",
        "fire_resistance",
        "water_breathing",
        "invisibility",
        "blindness",
        "night_vision",
        "hunger",
        "weakness",
        "poison",
        "wither",
        "health_boost",
        "absorption",
        "saturation",
    ];
    let path = name.strip_prefix("minecraft:").unwrap_or(name);
    NAMES.iter().position(|n| *n == path).map(|k| k as u8 + 1)
}

/// The attribute modifier an effect registers (`registerPotionAttributeModifier`):
/// `(attribute, uuid, amount per level, operation)`.
fn effect_modifier(id: u8) -> Option<(&'static str, u128, f64, i8)> {
    use super::attributes::{ATTACK_DAMAGE, MAX_HEALTH, MOVEMENT_SPEED};
    Some(match id {
        effect::SPEED => (
            MOVEMENT_SPEED,
            0x91aeaa56_376b_4498_935b_2f7f68070635,
            f64::from(0.2f32),
            2,
        ),
        effect::SLOWNESS => (
            MOVEMENT_SPEED,
            0x7107de5e_7ce8_4030_940e_514c1f160890,
            f64::from(-0.15f32),
            2,
        ),
        effect::STRENGTH => (
            ATTACK_DAMAGE,
            0x648d7064_6a60_4f59_8abe_c2c23a6dd7a9,
            2.5,
            2,
        ),
        effect::WEAKNESS => (
            ATTACK_DAMAGE,
            0x22653b89_116e_49dc_9b6b_9971489b5be5,
            2.0,
            0,
        ),
        effect::HEALTH_BOOST => (MAX_HEALTH, 0x5d6f0ba2_1186_46ac_b896_c61c5cee99cc, 4.0, 0),
        _ => return None,
    })
}

/// `Potion.isReady`: whether the effect acts this tick.
fn is_ready(id: u8, duration: i32, amplifier: u8) -> bool {
    let every = |base: i32| {
        let k = base >> amplifier;
        k <= 0 || duration % k == 0
    };
    match id {
        effect::REGENERATION => every(50),
        effect::POISON => every(25),
        effect::WITHER => every(40),
        effect::HUNGER => true,
        // PotionHealth (instant health, damage, saturation): every tick.
        effect::HEAL | effect::HARM | effect::SATURATION => duration >= 1,
        _ => false,
    }
}

/// `PotionHelper.calcPotionLiquidColor` for the active effects (0 if none shows).
pub fn particle_color(effects: &[Effect]) -> i32 {
    let (mut r, mut g, mut b, mut n) = (0f32, 0f32, 0f32, 0f32);
    // Summed in activePotionsMap order: the float rounding depends on it.
    for e in in_map_order(effects).iter().filter(|e| e.particles) {
        let c = effect_type(i32::from(e.id)).map_or(0, |t| t.color);
        for _ in 0..=e.amplifier {
            r += (c >> 16 & 0xff) as f32 / 255.0;
            g += (c >> 8 & 0xff) as f32 / 255.0;
            b += (c & 0xff) as f32 / 255.0;
            n += 1.0;
        }
    }
    if n == 0.0 {
        return NO_COLOR;
    }
    ((r / n * 255.0) as i32) << 16 | ((g / n * 255.0) as i32) << 8 | (b / n * 255.0) as i32
}

/// The effects in `activePotionsMap` order (a `HashMap<Integer, PotionEffect>`).
pub fn in_map_order(effects: &[Effect]) -> Vec<Effect> {
    let hashes: Vec<i32> = effects.iter().map(|e| i32::from(e.id)).collect();
    krol_java::java_hash_order(&hashes)
        .into_iter()
        .map(|k| effects[k])
        .collect()
}

/// Reads `ActiveEffects` (`PotionEffect.readCustomPotionEffectFromNBT`).
pub fn read_effects(data: &Compound) -> Vec<Effect> {
    let Some(list) = data.get_list("ActiveEffects", TagType::Compound) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|t| match t {
            Tag::Compound(c) => {
                let id = c.get_i8("Id") as u8;
                effect_type(i32::from(id))?;
                Some(Effect {
                    id,
                    amplifier: c.get_i8("Amplifier") as u8,
                    duration: c.get_i32("Duration"),
                    ambient: c.get_bool("Ambient"),
                    particles: !c.contains_key_of("ShowParticles", Some(TagType::Byte))
                        || c.get_bool("ShowParticles"),
                })
            }
            _ => None,
        })
        .collect()
}

/// Writes `ActiveEffects` (only when there are effects, like vanilla).
pub fn write_effects(data: &mut Compound, effects: &[Effect]) {
    if effects.is_empty() {
        data.remove("ActiveEffects");
        return;
    }
    let mut list = List::new();
    for e in in_map_order(effects) {
        let mut c = Compound::new();
        c.insert("Id", Tag::Byte(e.id as i8));
        c.insert("Amplifier", Tag::Byte(e.amplifier as i8));
        c.insert("Duration", Tag::Int(e.duration));
        c.insert("Ambient", Tag::Byte(i8::from(e.ambient)));
        c.insert("ShowParticles", Tag::Byte(i8::from(e.particles)));
        let _ = list.push(Tag::Compound(c));
    }
    data.insert("ActiveEffects", Tag::List(list));
}

fn effect_packet(entity_id: i32, e: &Effect) -> ClientboundPlay {
    ClientboundPlay::EntityEffect {
        entity_id,
        effect: e.id as i8,
        amplifier: e.amplifier as i8,
        duration: e.duration,
        show_particles: e.particles,
    }
}

impl Game {
    /// The effect packets of a player (sent when it joins and to new viewers).
    pub(super) fn effect_packets(&self, i: usize) -> Vec<ClientboundPlay> {
        let p = &self.players[i];
        in_map_order(&p.effects)
            .iter()
            .map(|e| effect_packet(p.entity_id, e))
            .collect()
    }

    /// `EntityLivingBase.addPotionEffect`: combines with an active effect of the same
    /// kind or starts it; the player is told either way.
    pub(super) fn add_effect(&mut self, i: usize, new: Effect) {
        let p = &mut self.players[i];
        let entity_id = p.entity_id;
        if let Some(old) = p.effects.iter_mut().find(|e| e.id == new.id) {
            // PotionEffect.combine.
            if new.amplifier > old.amplifier {
                old.amplifier = new.amplifier;
                old.duration = new.duration;
            } else if new.amplifier == old.amplifier && old.duration < new.duration {
                old.duration = new.duration;
            } else if !new.ambient && old.ambient {
                old.ambient = new.ambient;
            }
            old.particles = new.particles;
            let e = *old;
            p.potions_dirty = true;
            p.send(effect_packet(entity_id, &e));
            // onChangedPotionEffect(reapply): attribute modifiers are re-applied.
            self.remove_effect_modifiers(i, &e);
            self.apply_effect_modifiers(i, &e);
        } else {
            p.effects.push(new);
            p.potions_dirty = true;
            self.apply_effect_modifiers(i, &new);
            self.players[i].send(effect_packet(entity_id, &new));
        }
    }

    /// `EntityLivingBase.removePotionEffect`. Returns whether it was active.
    pub(super) fn remove_effect(&mut self, i: usize, id: u8) -> bool {
        let p = &mut self.players[i];
        let Some(k) = p.effects.iter().position(|e| e.id == id) else {
            return false;
        };
        let e = p.effects.remove(k);
        self.finished_effect(i, &e);
        true
    }

    /// `EntityLivingBase.clearActivePotions`.
    pub(super) fn clear_effects(&mut self, i: usize) {
        let all = in_map_order(&self.players[i].effects);
        self.players[i].effects.clear();
        for e in all {
            self.finished_effect(i, &e);
        }
    }

    /// `onFinishedPotionEffect` (EntityPlayerMP: the removal packet).
    fn finished_effect(&mut self, i: usize, e: &Effect) {
        let p = &mut self.players[i];
        p.potions_dirty = true;
        p.send(ClientboundPlay::RemoveEntityEffect {
            entity_id: p.entity_id,
            effect: e.id as i8,
        });
        self.remove_effect_modifiers(i, e);
    }

    /// `Potion.applyAttributesModifiersToEntity`: the effect's attribute modifier (its
    /// amount scaled by the level), or absorption health.
    fn apply_effect_modifiers(&mut self, i: usize, e: &Effect) {
        if e.id == effect::ABSORPTION {
            let p = &mut self.players[i];
            let amount =
                p.data.get_f32("AbsorptionAmount") + (4 * (i32::from(e.amplifier) + 1)) as f32;
            p.data
                .insert("AbsorptionAmount", Tag::Float(amount.max(0.0)));
            p.dirty_metadata.insert(17);
            return;
        }
        let Some((attribute, uuid, base, operation)) = effect_modifier(e.id) else {
            return;
        };
        let level = f64::from(e.amplifier) + 1.0;
        let amount = match e.id {
            // PotionAttackDamage.
            effect::STRENGTH => 1.3 * level,
            effect::WEAKNESS => f64::from(-0.5f32 * level as f32),
            _ => base * level,
        };
        let name = effect_type(i32::from(e.id)).map_or("", |t| t.name);
        let p = &mut self.players[i];
        p.modifiers.retain(|m| m.uuid != uuid);
        p.modifiers.push(super::attributes::Modifier {
            attribute,
            uuid,
            name: format!("{name} {}", e.amplifier),
            amount,
            operation,
            saved: true,
        });
        p.dirty_attributes.insert(attribute);
    }

    /// `Potion.removeAttributesModifiersFromEntity` (and `PotionAbsorption`,
    /// `PotionHealthBoost`).
    fn remove_effect_modifiers(&mut self, i: usize, e: &Effect) {
        if e.id == effect::ABSORPTION {
            let p = &mut self.players[i];
            let amount =
                p.data.get_f32("AbsorptionAmount") - (4 * (i32::from(e.amplifier) + 1)) as f32;
            p.data
                .insert("AbsorptionAmount", Tag::Float(amount.max(0.0)));
            p.dirty_metadata.insert(17);
            return;
        }
        let Some((attribute, uuid, _, _)) = effect_modifier(e.id) else {
            return;
        };
        let p = &mut self.players[i];
        p.modifiers.retain(|m| m.uuid != uuid);
        p.dirty_attributes.insert(attribute);
        if e.id == effect::HEALTH_BOOST && p.health() > p.max_health() {
            let max = p.max_health();
            p.set_health(max);
        }
    }

    /// `EntityLivingBase.heal`.
    pub(super) fn heal_player(&mut self, i: usize, amount: f32) {
        let p = &mut self.players[i];
        let health = p.health();
        if health > 0.0 {
            p.set_health(health + amount);
        }
    }

    /// `Potion.performEffect` on a player.
    fn perform_effect(&mut self, i: usize, id: u8, amplifier: u8) {
        let amp = i32::from(amplifier);
        match id {
            effect::REGENERATION => {
                if self.players[i].health() < self.players[i].max_health() {
                    self.heal_player(i, 1.0);
                }
            }
            effect::POISON => {
                if self.players[i].health() > 1.0 {
                    self.attack_player(i, DamageSource::Magic, 1.0);
                }
            }
            effect::WITHER => {
                self.attack_player(i, DamageSource::Wither, 1.0);
            }
            effect::HUNGER => self.add_exhaustion(i, 0.025 * (amp + 1) as f32),
            effect::SATURATION => self.add_food(i, amp + 1, 1.0),
            effect::HEAL => self.heal_player(i, (4 << amp).max(0) as f32),
            effect::HARM => {
                self.attack_player(i, DamageSource::Magic, (6 << amp) as f32);
            }
            _ => {}
        }
    }

    /// `EntityLivingBase.updatePotionEffects` for a player (once per update): effects
    /// act, count down and end; the particle color follows changes.
    pub(super) fn update_effects(&mut self, i: usize) {
        let order = in_map_order(&self.players[i].effects);
        for e in order {
            let Some(now) = self.players[i]
                .effects
                .iter()
                .find(|x| x.id == e.id)
                .copied()
            else {
                continue;
            };
            let mut left = now.duration;
            if left > 0 {
                if is_ready(now.id, left, now.amplifier) {
                    self.perform_effect(i, now.id, now.amplifier);
                }
                left -= 1;
            }
            let p = &mut self.players[i];
            if let Some(x) = p.effects.iter_mut().find(|x| x.id == e.id) {
                x.duration = left;
            }
            if left <= 0 {
                self.remove_effect(i, e.id);
            } else if left % RESEND_PERIOD == 0 {
                // onChangedPotionEffect(effect, false).
                let p = &mut self.players[i];
                p.potions_dirty = true;
                if let Some(x) = p.effects.iter().find(|x| x.id == e.id) {
                    let packet = effect_packet(p.entity_id, x);
                    p.send(packet);
                }
            }
        }
        let p = &mut self.players[i];
        if p.potions_dirty {
            // updatePotionMetadata (the data watcher only sends changed values).
            p.potions_dirty = false;
            let color = particle_color(&p.effects);
            let ambient = !p.effects.is_empty() && p.effects.iter().all(|e| e.ambient);
            if color != p.effect_color {
                p.effect_color = color;
                p.dirty_metadata.insert(7);
            }
            if ambient != p.effect_ambient {
                p.effect_ambient = ambient;
                p.dirty_metadata.insert(8);
            }
            // TODO(M5): invisibility.
        }
        // TODO(M5): the effect particles draw from the entity's random.
    }

    /// `FoodStats.addStats(food, saturation)`.
    pub(super) fn add_food(&mut self, i: usize, food: i32, saturation: f32) {
        let d = &mut self.players[i].data;
        let level = (food + d.get_i32("foodLevel")).min(20);
        let sat =
            (d.get_f32("foodSaturationLevel") + food as f32 * saturation * 2.0).min(level as f32);
        d.insert("foodLevel", Tag::Int(level));
        d.insert("foodSaturationLevel", Tag::Float(sat));
    }
}
