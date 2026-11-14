//! Damage to players (vanilla: `DamageSource`, `EntityPlayerMP/EntityPlayer/
//! EntityLivingBase.attackEntityFrom`, `EntityPlayer.damageEntity`).
//!
//! TODO(M5): armor and its durability, the resistance potion, protection enchantments
//! (their modifier uses an unseeded random), blocking, absorption hearts from potions,
//! the combat tracker, knockback from attackers and death.

use krol_nbt::Tag;
use krol_protocol::packets::play::ClientboundPlay;

use super::Game;

/// Where damage comes from (vanilla `DamageSource`), with the properties that matter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageSource {
    /// Falling (`DamageSource.fall`: bypasses armor, no exhaustion).
    Fall,
    /// Running out of air under water (`DamageSource.drown`: bypasses armor, no
    /// exhaustion, no knockback).
    Drown,
    /// Burning (`DamageSource.onFire`: bypasses armor, no exhaustion).
    OnFire,
    /// Standing in fire or lava blocks (`DamageSource.inFire`).
    InFire,
    /// Touching lava (`DamageSource.lava`).
    Lava,
}

impl DamageSource {
    /// The damage type (`damageType`), used in death messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::Fall => "fall",
            Self::Drown => "drown",
            Self::OnFire => "onFire",
            Self::InFire => "inFire",
            Self::Lava => "lava",
        }
    }

    /// Armor does not reduce it (`isUnblockable`).
    pub fn unblockable(self) -> bool {
        matches!(self, Self::Fall | Self::Drown | Self::OnFire)
    }

    /// Fire damage (`isFireDamage`), reduced by fire protection.
    pub fn fire(self) -> bool {
        matches!(self, Self::OnFire | Self::InFire | Self::Lava)
    }

    /// Damage creative and spectator players take too (`canHarmInCreative`).
    fn harms_in_creative(self) -> bool {
        false
    }

    /// Exhaustion added to the victim (`getHungerDamage`).
    fn hunger_damage(self) -> f32 {
        match self {
            Self::Fall | Self::Drown | Self::OnFire => 0.0,
            Self::InFire | Self::Lava => DEFAULT_HUNGER_DAMAGE,
        }
    }
}

/// The armor slots of the player window: helmet, chestplate, leggings, boots
/// (`armorInventory` is boots first).
const ARMOR_SLOTS: [usize; 4] = [5, 6, 7, 8];

/// `EnchantmentProtection.calcModifierDamage`.
fn protection_modifier(id: i16, level: i32, source: DamageSource) -> i32 {
    use super::item_data::{FEATHER_FALLING, FIRE_PROTECTION, PROTECTION};
    if source.harms_in_creative() {
        return 0;
    }
    let f = (6 + level * level) as f32 / 3.0;
    let factor = match id {
        PROTECTION => 0.75,
        FIRE_PROTECTION if source.fire() => 1.25,
        FEATHER_FALLING if source == DamageSource::Fall => 2.5,
        // TODO(M5): blast (3) and projectile (4) protection.
        _ => return 0,
    };
    krol_java::math::floor_f32(f * factor)
}

/// `DamageSource.hungerDamage` of damage that armor can block.
const DEFAULT_HUNGER_DAMAGE: f32 = 0.3;

/// `EntityLivingBase.maxHurtTime`: how long the hurt animation lasts.
pub const MAX_HURT_TIME: i16 = 10;

impl Game {
    /// Vanilla `attackEntityFrom` for a player. Returns whether the damage was taken.
    pub(super) fn attack_player(&mut self, i: usize, source: DamageSource, amount: f32) -> bool {
        let pvp = self.config.pvp;
        let p = &mut self.players[i];
        // EntityPlayerMP: fall damage ignores the respawn protection when PvP is on.
        let fall_ignores_protection = pvp && source.name() == "fall";
        if !fall_ignores_protection && p.respawn_invulnerability > 0 {
            return false;
        }
        // EntityPlayer.
        let disable_damage = p.gamemode == 1 || p.gamemode == 3;
        if disable_damage && !source.harms_in_creative() {
            return false;
        }
        if p.health() <= 0.0 || amount == 0.0 {
            return false;
        }
        // EntityLivingBase (maxHurtResistantTime 20).
        let fresh = if p.hurt_resistant_time as f32 > 10.0 {
            if amount <= p.last_damage {
                return false;
            }
            let extra = amount - p.last_damage;
            p.last_damage = amount;
            self.damage_player(i, source, extra);
            false
        } else {
            p.last_damage = amount;
            p.hurt_resistant_time = 20;
            self.damage_player(i, source, amount);
            true
        };
        let entity_id = self.players[i].entity_id;
        if fresh {
            // hurtTime = maxHurtTime (counts down in the living update).
            self.players[i]
                .data
                .insert("HurtTime", Tag::Short(MAX_HURT_TIME));
            // setEntityState(2): the hurt animation, to the viewers and the player.
            let status = ClientboundPlay::EntityStatus {
                entity_id,
                status: 2,
            };
            self.players[i].send(status.clone());
            self.send_to_tracking(entity_id, &status);
            // setBeenAttacked (not for drowning): velocityChanged = rand.nextDouble() >=
            // knockback resistance, which is always true for players.
            if source != DamageSource::Drown {
                let p = &mut self.players[i];
                let _ = p.rand.next_double();
                p.velocity_changed = true;
            }
        }
        let p = &mut self.players[i];
        let (pos, conn) = (p.pos, p.conn_id);
        if p.health() <= 0.0 {
            if fresh {
                let pitch = Self::sound_pitch(&mut self.players[i].rand);
                self.play_sound_near_except(Some(conn), "game.player.die", pos, 1.0, pitch);
            }
            self.player_died(i);
        } else if fresh {
            let pitch = Self::sound_pitch(&mut self.players[i].rand);
            self.play_sound_near_except(Some(conn), "game.player.hurt", pos, 1.0, pitch);
        }
        true
    }

    /// `EntityLivingBase.getSoundPitch` for an adult.
    fn sound_pitch(rand: &mut krol_java::JavaRandom) -> f32 {
        (rand.next_float() - rand.next_float()) * 0.2 + 1.0
    }

    /// `EntityLivingBase.applyArmorCalculations`: armor points reduce blockable damage
    /// by 4% each, and the armor wears.
    fn apply_armor(&mut self, i: usize, source: DamageSource, amount: f32) -> f32 {
        if source.unblockable() {
            return amount;
        }
        let armor: i32 = ARMOR_SLOTS
            .iter()
            .filter_map(|&s| self.players[i].inventory[s].as_ref())
            .map(|s| super::item_data::armor_points(s.id))
            .sum();
        let reduced = amount * (25 - armor) as f32;
        self.damage_armor(i, amount);
        reduced / 25.0
    }

    /// `InventoryPlayer.damageArmor`: each armor piece (boots first) wears by a quarter
    /// of the damage, at least 1.
    fn damage_armor(&mut self, i: usize, amount: f32) {
        let wear = (amount / 4.0).max(1.0) as i32;
        for &slot in ARMOR_SLOTS.iter().rev() {
            let is_armor = self.players[i].inventory[slot]
                .as_ref()
                .is_some_and(|s| super::item_data::armor_points(s.id) > 0);
            if !is_armor {
                continue;
            }
            self.damage_slot_item(i, slot, wear);
            let p = &mut self.players[i];
            if p.inventory[slot].as_ref().is_some_and(|s| s.count == 0) {
                p.inventory[slot] = None;
            }
        }
    }

    /// `EntityLivingBase.applyPotionDamageCalculations` (without potions): protection
    /// enchantments on the armor (`EnchantmentHelper.getEnchantmentModifierDamage`).
    fn apply_protection(&mut self, i: usize, source: DamageSource, amount: f32) -> f32 {
        // TODO(M5): the resistance potion.
        if amount <= 0.0 {
            return 0.0;
        }
        let mut modifier = 0;
        for &slot in ARMOR_SLOTS.iter().rev() {
            let Some(stack) = self.players[i].inventory[slot].as_ref() else {
                continue;
            };
            for (id, level) in super::item_data::enchantments(stack) {
                modifier += protection_modifier(id, level, source);
            }
        }
        let modifier = modifier.clamp(0, 25);
        let k = ((modifier + 1) >> 1) + self.enchant_rand.next_int_bounded((modifier >> 1) + 1);
        let k = k.min(20);
        if k > 0 {
            amount * (25 - k) as f32 / 25.0
        } else {
            amount
        }
    }

    /// Vanilla `EntityPlayer.damageEntity` (no sword blocking yet).
    fn damage_player(&mut self, i: usize, source: DamageSource, amount: f32) {
        let amount = self.apply_armor(i, source, amount);
        let amount = self.apply_protection(i, source, amount);
        let p = &mut self.players[i];
        let absorption = p.data.get_f32("AbsorptionAmount");
        let damage = (amount - absorption).max(0.0);
        let new_absorption = absorption - (amount - damage);
        if new_absorption != absorption {
            p.data
                .insert("AbsorptionAmount", Tag::Float(new_absorption));
            p.dirty_metadata.insert(17);
        }
        if damage != 0.0 {
            self.add_exhaustion(i, source.hunger_damage());
            self.track_damage(i, source, damage);
            let p = &mut self.players[i];
            let health = p.health();
            p.set_health(health - damage);
            if damage < 3.402_823_5e37 {
                p.stats
                    .add("stat.damageTaken", (damage * 10.0).round() as i32);
            }
        }
    }
}
