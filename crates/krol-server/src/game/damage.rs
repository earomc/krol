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
    /// Poison and instant damage (`DamageSource.magic`: bypasses armor).
    Magic,
    /// The wither effect (`DamageSource.wither`: bypasses armor).
    Wither,
    /// Touching a cactus (`DamageSource.cactus`).
    Cactus,
    /// Suffocating inside a block (`DamageSource.inWall`: bypasses armor, no
    /// exhaustion).
    InWall,
    /// `/kill` (`DamageSource.outOfWorld`: bypasses armor and respawn protection, harms
    /// creative players, no exhaustion).
    OutOfWorld,
    /// An explosion nobody set off (`DamageSource.setExplosionSource`: difficulty
    /// scaled).
    Explosion,
    /// An explosion of TNT a player lit, with that player's entity id
    /// (`EntityDamageSourceIndirect("explosion.player")`: difficulty scaled, knocks the
    /// victim away from the player).
    ExplosionBy(i32),
    /// A player's attack, with the attacker's entity id
    /// (`EntityDamageSource("player")`).
    Player(i32),
    /// Thorns armor hurting an attacker, with the wearer's entity id
    /// (`EntityDamageSource("thorns")`).
    Thorns(i32),
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
            Self::Magic => "magic",
            Self::Wither => "wither",
            Self::Cactus => "cactus",
            Self::InWall => "inWall",
            Self::OutOfWorld => "outOfWorld",
            Self::Explosion => "explosion",
            Self::ExplosionBy(_) => "explosion.player",
            Self::Player(_) => "player",
            Self::Thorns(_) => "thorns",
        }
    }

    /// Armor does not reduce it (`isUnblockable`).
    pub fn unblockable(self) -> bool {
        matches!(
            self,
            Self::Fall
                | Self::Drown
                | Self::OnFire
                | Self::Magic
                | Self::Wither
                | Self::InWall
                | Self::OutOfWorld
        )
    }

    /// Fire damage (`isFireDamage`), reduced by fire protection.
    pub fn fire(self) -> bool {
        matches!(self, Self::OnFire | Self::InFire | Self::Lava)
    }

    /// An explosion (`isExplosion`).
    pub fn explosion(self) -> bool {
        matches!(self, Self::Explosion | Self::ExplosionBy(_))
    }

    /// Scaled by the difficulty (`isDifficultyScaled`).
    fn difficulty_scaled(self) -> bool {
        self.explosion()
    }

    /// The entity the damage is attributed to (`getEntity`), which knocks the victim
    /// back.
    pub fn attacker(self) -> Option<i32> {
        match self {
            Self::ExplosionBy(id) | Self::Player(id) | Self::Thorns(id) => Some(id),
            _ => None,
        }
    }

    /// Damage creative and spectator players take too (`canHarmInCreative`).
    fn harms_in_creative(self) -> bool {
        self == Self::OutOfWorld
    }

    /// Exhaustion added to the victim (`getHungerDamage`).
    fn hunger_damage(self) -> f32 {
        match self {
            Self::Fall
            | Self::Drown
            | Self::OnFire
            | Self::Magic
            | Self::Wither
            | Self::InWall
            | Self::OutOfWorld => 0.0,
            Self::InFire
            | Self::Lava
            | Self::Cactus
            | Self::Explosion
            | Self::ExplosionBy(_)
            | Self::Player(_)
            | Self::Thorns(_) => DEFAULT_HUNGER_DAMAGE,
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
        super::item_data::BLAST_PROTECTION if source.explosion() => 1.5,
        // TODO(M5): projectile protection (4).
        _ => return 0,
    };
    krol_java::math::floor_f32(f * factor)
}

/// `EnumDifficulty` ids.
const PEACEFUL: i8 = 0;
const EASY: i8 = 1;
const HARD: i8 = 3;
/// `knockBack`'s strength (`0.4F`), also the cap on the upward motion.
const KNOCKBACK: f32 = 0.4;

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
        if !fall_ignores_protection
            && p.respawn_invulnerability > 0
            && source != DamageSource::OutOfWorld
        {
            return false;
        }
        // EntityDamageSource: players only hurt players with PvP on, and teammates with
        // friendly fire.
        if let Some(attacker) = source.attacker()
            && self.players.iter().any(|p| p.entity_id == attacker)
            && !self.can_be_attacked_by(i, attacker)
        {
            return false;
        }
        let p = &mut self.players[i];
        // EntityPlayer.
        let disable_damage = p.gamemode == 1 || p.gamemode == 3;
        if disable_damage && !source.harms_in_creative() {
            return false;
        }
        if p.health() <= 0.0 {
            return false;
        }
        if p.sleeping {
            self.wake_up(i, true, true, false);
        }
        let mut amount = amount;
        if source.difficulty_scaled() {
            match self.level.data.data.get_i8("Difficulty") {
                PEACEFUL => amount = 0.0,
                EASY => amount = amount / 2.0 + 1.0,
                HARD => amount = amount * 3.0 / 2.0,
                _ => {}
            }
        }
        let p = &mut self.players[i];
        if amount == 0.0 {
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
        if fresh
            && let Some(attacker) = source.attacker()
            && self.players.iter().any(|p| p.entity_id == attacker)
        {
            let p = &mut self.players[i];
            p.attacking_player = Some((attacker, super::combat::RECENTLY_HIT));
            p.set_revenge_target(Some(attacker));
        }
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
            let attacker = source
                .attacker()
                .and_then(|id| self.players.iter().find(|p| p.entity_id == id))
                .map(|a| a.pos);
            match attacker {
                Some(from) => self.knock_back(i, from),
                // attackedAtYaw is picked with Math.random().
                None => {
                    self.math_random.next_double();
                }
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

    /// `EntityLivingBase.attackEntityFrom`'s knockback from the attacker at `from`
    /// (`knockBack`; players have no knockback resistance, but the roll is made).
    fn knock_back(&mut self, i: usize, from: (f64, f64, f64)) {
        let p = &self.players[i];
        let mut dx = from.0 - p.pos.0;
        let mut dz = from.2 - p.pos.2;
        while dx * dx + dz * dz < 1.0e-4 {
            dx = (self.math_random.next_double() - self.math_random.next_double()) * 0.01;
            dz = (self.math_random.next_double() - self.math_random.next_double()) * 0.01;
        }
        let p = &mut self.players[i];
        // rand.nextDouble() >= knockback resistance (0 for players): always knocked back.
        let _ = p.rand.next_double();
        p.airborne = true;
        let len = f64::from(krol_java::math::sqrt_f64(dx * dx + dz * dz));
        let strength = f64::from(KNOCKBACK);
        let m = &mut p.motion;
        m.0 /= 2.0;
        m.1 /= 2.0;
        m.2 /= 2.0;
        m.0 -= dx / len * strength;
        m.1 += strength;
        m.2 -= dz / len * strength;
        if m.1 > f64::from(KNOCKBACK) {
            m.1 = f64::from(KNOCKBACK);
        }
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

    /// Vanilla `EntityPlayer.damageEntity`: blocking with a sword halves blockable damage
    /// (plus a half), then armor and protection, then absorption.
    fn damage_player(&mut self, i: usize, source: DamageSource, amount: f32) {
        let blocking = self.players[i]
            .using
            .as_ref()
            .is_some_and(|u| super::combat::is_sword(u.id));
        let amount = if !source.unblockable() && blocking && amount > 0.0 {
            (1.0 + amount) * 0.5
        } else {
            amount
        };
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
            // Vanilla quirk: the health drops before the hit is recorded, so a deadly
            // hit finds the player dead, ends the combat and forgets the earlier hits.
            let p = &mut self.players[i];
            let health = p.health();
            p.set_health(health - damage);
            self.track_damage(i, source, damage);
            let p = &mut self.players[i];
            if damage < 3.402_823_5e37 {
                p.stats
                    .add("stat.damageTaken", (damage * 10.0).round() as i32);
            }
        }
    }
}
