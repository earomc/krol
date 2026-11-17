//! Players attacking (vanilla: `NetHandlerPlayServer.processUseEntity`,
//! `EntityPlayer.attackTargetEntityWithCurrentItem`), the attribute modifiers of held
//! items (`ItemStack.getAttributeModifiers`), thorns, killers
//! (`EntityLivingBase.func_94060_bK`) and the scores a kill gives
//! (`EntityPlayer.addToPlayerScore`).
//!
//! Paintings and item frames can be hit (which breaks them) and item frames used.
//!
//! TODO(M5): attacking other entities (only players are living entities in krol;
//! falling blocks, primed TNT and projectiles are ignored), interacting with other
//! entities, spectators attacking entities to spectate them, the overkill achievement.

use krol_protocol::packets::play::{ClientboundPlay, ItemStack};

use super::Game;
use super::attributes::{self, Modifier};
use super::damage::DamageSource;

/// `Item.itemModifierUUID`: the id of a weapon's or tool's damage modifier.
const ITEM_MODIFIER_UUID: u128 = 0xcb3f55d3_645c_4f38_a497_9c13a33db5cf;
/// `Item.ToolMaterial.getDamageVsEntity` for wood, stone, iron, diamond and gold.
const MATERIAL_DAMAGE: [f64; 5] = [0.0, 1.0, 2.0, 3.0, 0.0];
/// `ItemSword`: 4 plus the material's damage.
const SWORD_DAMAGE: f64 = 4.0;
/// `ItemAxe`, `ItemPickaxe`, `ItemSpade`: their base damage before the material's.
const AXE_DAMAGE: f64 = 3.0;
const PICKAXE_DAMAGE: f64 = 2.0;
const SHOVEL_DAMAGE: f64 = 1.0;

/// Sword, axe, pickaxe and shovel ids by material (wood, stone, iron, diamond, gold).
const SWORDS: [i16; 5] = [268, 272, 267, 276, 283];
const AXES: [i16; 5] = [271, 275, 258, 279, 286];
const PICKAXES: [i16; 5] = [270, 274, 257, 278, 285];
const SHOVELS: [i16; 5] = [269, 273, 256, 277, 284];

/// Enchantment ids.
const SHARPNESS: i16 = 16;
const KNOCKBACK: i16 = 19;
const FIRE_ASPECT: i16 = 20;
const THORNS: i16 = 7;
/// `Potion.blindness`.
const BLINDNESS: u8 = 15;

/// Squared reach of an attack on an entity the player can see, and on one it cannot.
const REACH_SQ_VISIBLE: f64 = 36.0;
const REACH_SQ_HIDDEN: f64 = 9.0;
/// `C02PacketUseEntity.Action` ids.
const INTERACT: i32 = 0;
const ATTACK: i32 = 1;
/// The spectator game mode.
const SPECTATOR: i32 = 3;
/// The sound thorns make at the attacker (`damage.thorns`, volume 0.5, pitch 1).
const THORNS_SOUND: &str = "damage.thorns";
const THORNS_VOLUME: f32 = 0.5;
/// `S0BPacketAnimation` ids of critical hits and enchanted hits.
const CRITICAL_EFFECT: u8 = 4;
const MAGIC_CRITICAL_EFFECT: u8 = 5;
/// Exhaustion of an attack (`addExhaustion(0.3F)`).
const ATTACK_EXHAUSTION: f32 = 0.3;
/// How long an attacker counts as the killer without combat entries (`recentlyHit`).
pub const RECENTLY_HIT: i32 = 100;

/// `ItemStack.getAttributeModifiers`: the stack's `AttributeModifiers` tag if it has
/// one, else the item's own (weapons and tools add attack damage).
pub fn item_modifiers(stack: &ItemStack) -> Vec<Modifier> {
    if let Some(list) = stack
        .nbt
        .as_ref()
        .and_then(|n| n.get_list("AttributeModifiers", krol_nbt::TagType::Compound))
    {
        let mut out = Vec::new();
        for t in list.iter() {
            let krol_nbt::Tag::Compound(c) = t else {
                continue;
            };
            let (most, least) = (c.get_i64("UUIDMost"), c.get_i64("UUIDLeast"));
            let name = c.get_str("Name");
            let operation = c.get_i32("Operation");
            // readAttributeModifierFromNBT fails on an empty name or a bad operation;
            // getAttributeModifiers drops zero UUID halves and unknown attributes.
            let Some(attribute) = attributes::attribute_key(c.get_str("AttributeName")) else {
                continue;
            };
            if name.is_empty() || !(0..=2).contains(&operation) || most == 0 || least == 0 {
                continue;
            }
            out.push(Modifier {
                attribute,
                uuid: u128::from(most as u64) << 64 | u128::from(least as u64),
                name: name.to_owned(),
                amount: c.get_f64("Amount"),
                operation: operation as i8,
                saved: true,
            });
        }
        return out;
    }
    let find = |ids: &[i16; 5]| ids.iter().position(|&id| id == stack.id);
    let (base, name) = if let Some(m) = find(&SWORDS) {
        (SWORD_DAMAGE + MATERIAL_DAMAGE[m], "Weapon modifier")
    } else if let Some(m) = find(&AXES) {
        (AXE_DAMAGE + MATERIAL_DAMAGE[m], "Tool modifier")
    } else if let Some(m) = find(&PICKAXES) {
        (PICKAXE_DAMAGE + MATERIAL_DAMAGE[m], "Tool modifier")
    } else if let Some(m) = find(&SHOVELS) {
        (SHOVEL_DAMAGE + MATERIAL_DAMAGE[m], "Tool modifier")
    } else {
        return Vec::new();
    };
    vec![Modifier {
        attribute: attributes::ATTACK_DAMAGE,
        uuid: ITEM_MODIFIER_UUID,
        name: name.to_owned(),
        // The damage is a float field.
        amount: f64::from(base as f32),
        operation: 0,
        saved: true,
    }]
}

/// Whether the item is a sword (blocks while used, wears by 1 when hitting).
pub fn is_sword(id: i16) -> bool {
    SWORDS.contains(&id)
}

/// `ItemTool.hitEntity`: tools wear by 2 when hitting.
fn is_tool(id: i16) -> bool {
    AXES.contains(&id) || PICKAXES.contains(&id) || SHOVELS.contains(&id)
}

/// `EnchantmentHelper.func_152377_a` against an undefined creature (players):
/// sharpness adds 1.25 per level.
fn enchantment_damage(stack: Option<&ItemStack>) -> f32 {
    let Some(stack) = stack else {
        return 0.0;
    };
    let mut total = 0.0f32;
    for (id, level) in super::item_data::enchantments(stack) {
        if id == SHARPNESS {
            total += level as f32 * 1.25;
        }
    }
    total
}

impl Game {
    /// `EntityLivingBase.onUpdate`'s equipment change: the old item's modifiers go, the
    /// new item's come (watched attributes are sent again).
    pub(super) fn swap_equipment_modifiers(
        &mut self,
        i: usize,
        old: Option<&ItemStack>,
        new: Option<&ItemStack>,
    ) {
        let p = &mut self.players[i];
        for m in old.map(item_modifiers).unwrap_or_default() {
            if let Some(k) = p
                .modifiers
                .iter()
                .position(|x| x.attribute == m.attribute && x.uuid == m.uuid)
            {
                p.modifiers.remove(k);
                p.dirty_attributes.insert(m.attribute);
            }
        }
        for m in new.map(item_modifiers).unwrap_or_default() {
            p.modifiers
                .retain(|x| !(x.attribute == m.attribute && x.uuid == m.uuid));
            p.dirty_attributes.insert(m.attribute);
            p.modifiers.push(m);
        }
    }

    /// `processUseEntity`: attacks and interactions within reach (6 blocks, 3 through
    /// walls).
    pub(super) fn use_entity(&mut self, i: usize, target: i32, action: i32) {
        let target_player = self
            .players
            .iter()
            .position(|p| p.entity_id == target && !p.removed);
        let hanging = self
            .hanging
            .get(&target)
            .map(|h| (h.body.pos, h.body.height * 0.85));
        let pos = if let Some(t) = target_player {
            Some(self.players[t].pos)
        } else if let Some((pos, _)) = hanging {
            Some(pos)
        } else if let Some(it) = self.items.get(&target) {
            Some(it.body.pos)
        } else {
            self.orbs.get(&target).map(|o| o.body.pos)
        };
        let Some(pos) = pos else {
            return;
        };
        let seen = match (target_player, hanging) {
            (Some(t), _) => self.can_see(i, t),
            (None, Some((pos, eye))) => {
                self.can_see_point(i, (pos.0, pos.1 + f64::from(eye), pos.2))
            }
            (None, None) => true,
        };
        let reach = if seen {
            REACH_SQ_VISIBLE
        } else {
            REACH_SQ_HIDDEN
        };
        let p = self.players[i].pos;
        let d = (p.0 - pos.0, p.1 - pos.1, p.2 - pos.2);
        if d.0 * d.0 + d.1 * d.1 + d.2 * d.2 >= reach {
            return;
        }
        if hanging.is_some() {
            match action {
                INTERACT => self.interact_hanging(i, target),
                // EntityHanging.hitByEntity: a hit from a player that cannot hurt it
                // (invulnerable) goes on as a plain attack, which hurts nothing either.
                ATTACK if self.players[i].gamemode != SPECTATOR => {
                    let attacker = self.players[i].entity_id;
                    let _ = self.attack_hanging(target, DamageSource::Player(attacker));
                }
                _ => {}
            }
            return;
        }
        if action != ATTACK {
            return;
        }
        match target_player {
            Some(t) if t != i => self.attack_with_item(i, t),
            _ => {
                let name = self.players[i].profile.name.clone();
                self.kick(i, "Attempting to attack an invalid entity");
                tracing::warn!("Player {name} tried to attack an invalid entity");
            }
        }
    }

    /// `EntityLivingBase.canEntityBeSeen`: nothing blocks the line between the eyes.
    fn can_see(&mut self, i: usize, t: usize) -> bool {
        let p = &self.players[t];
        let to = (p.pos.0, p.pos.1 + f64::from(p.eye_height()), p.pos.2);
        self.can_see_point(i, to)
    }

    /// Whether nothing blocks the line from the player's eyes to `to` (the eyes of the
    /// entity looked at).
    fn can_see_point(&mut self, i: usize, to: (f64, f64, f64)) -> bool {
        let p = &self.players[i];
        let from = (p.pos.0, p.pos.1 + f64::from(p.eye_height()), p.pos.2);
        super::raytrace::ray_trace_blocks(&mut self.level.overworld, from, to, false, false)
            .is_none()
    }

    /// `attackTargetEntityWithCurrentItem` on another player.
    fn attack_with_item(&mut self, i: usize, t: usize) {
        let held = self.players[i].held().cloned();
        let mut damage =
            attributes::value(attributes::ATTACK_DAMAGE, &self.players[i].modifiers) as f32;
        let bonus = enchantment_damage(held.as_ref());
        let mut knockback = super::item_data::enchantment_level(held.as_ref(), KNOCKBACK);
        if self.players[i].sprinting {
            knockback += 1;
        }
        if damage <= 0.0 && bonus <= 0.0 {
            return;
        }
        let on_ladder = self.on_ladder(i);
        let p = &self.players[i];
        let critical = p.fall_distance > 0.0
            && !p.on_ground
            && !on_ladder
            && !p.in_water
            && !p.effects.iter().any(|e| e.id == BLINDNESS);
        if critical && damage > 0.0 {
            damage *= 1.5;
        }
        damage += bonus;
        let fire_aspect = super::item_data::enchantment_level(held.as_ref(), FIRE_ASPECT);
        let mut lit = false;
        if fire_aspect > 0 && self.players[t].data.get_i16("Fire") <= 0 {
            lit = true;
            self.ignite_player(t, 1);
        }
        let saved_motion = self.players[t].motion;
        let attacker_id = self.players[i].entity_id;
        if !self.attack_player(t, DamageSource::Player(attacker_id), damage) {
            if lit {
                self.extinguish_player(t);
            }
            return;
        }
        if knockback > 0 {
            let yaw = self.players[i].rotation.0 * std::f32::consts::PI / 180.0;
            let k = knockback as f32 * 0.5;
            let v = &mut self.players[t];
            v.motion.0 += f64::from(-krol_java::math::sin(yaw) * k);
            v.motion.1 += 0.1;
            v.motion.2 += f64::from(krol_java::math::cos(yaw) * k);
            v.airborne = true;
            v.velocity_changed = true;
            let a = &mut self.players[i];
            a.motion.0 *= 0.6;
            a.motion.2 *= 0.6;
            self.set_sprinting(i, false);
        }
        // The victim's client gets its knockback now, while the server keeps the old
        // motion (vanilla quirk).
        let v = &mut self.players[t];
        if v.velocity_changed {
            let packet = super::tracker::velocity_packet(v.entity_id, v.motion);
            v.send(packet);
            v.velocity_changed = false;
            v.motion = saved_motion;
        }
        let victim_id = self.players[t].entity_id;
        if critical {
            self.send_to_self_and_tracking(
                i,
                ClientboundPlay::Animation {
                    entity_id: victim_id,
                    animation: CRITICAL_EFFECT,
                },
            );
        }
        if bonus > 0.0 {
            self.send_to_self_and_tracking(
                i,
                ClientboundPlay::Animation {
                    entity_id: victim_id,
                    animation: MAGIC_CRITICAL_EFFECT,
                },
            );
        }
        // TODO(M5): the overkill achievement (18 damage or more).
        self.apply_thorns(t, i);
        if let Some(stack) = held {
            // ItemStack.hitEntity: swords wear by 1, tools by 2, and both count as used.
            if is_sword(stack.id) || is_tool(stack.id) {
                self.damage_held_item(i, if is_sword(stack.id) { 1 } else { 2 });
                self.count_item_use(i);
            }
            let p = &mut self.players[i];
            let slot = 36 + p.selected;
            if p.inventory[slot].as_ref().is_some_and(|s| s.count <= 0) {
                // destroyCurrentEquippedItem.
                p.inventory[slot] = None;
            }
        }
        self.add_stat(i, "stat.damageDealt", (damage * 10.0).round() as i32);
        if fire_aspect > 0 {
            self.ignite_player(t, fire_aspect * 4);
        }
        self.add_exhaustion(i, ATTACK_EXHAUSTION);
    }

    /// `EnchantmentHelper.applyThornEnchantments`: each thorns piece the victim holds or
    /// wears may hurt the attacker (`EnchantmentThorns.onUserHurt`, drawing from the
    /// victim's random) and wears.
    pub(super) fn apply_thorns(&mut self, victim: usize, attacker: usize) {
        // getInventory: the held item, then the armor from the boots up.
        let slots = [36 + self.players[victim].selected, 8, 7, 6, 5];
        for slot in slots {
            let Some(stack) = self.players[victim].inventory[slot].clone() else {
                continue;
            };
            for (id, level) in super::item_data::enchantments(&stack) {
                if id != THORNS {
                    continue;
                }
                // getEnchantedItem: the first equipment piece with thorns.
                let worn = slots.into_iter().find(|&s| {
                    super::item_data::enchantment_level(
                        self.players[victim].inventory[s].as_ref(),
                        THORNS,
                    ) > 0
                });
                let r = &mut self.players[victim].rand;
                if level > 0 && r.next_float() < 0.15 * level as f32 {
                    let amount = if level > 10 {
                        level - 10
                    } else {
                        1 + r.next_int_bounded(4)
                    };
                    let source = DamageSource::Thorns(self.players[victim].entity_id);
                    self.attack_player(attacker, source, amount as f32);
                    // EntityPlayer.playSound: heard by everyone near but the attacker.
                    let a = &self.players[attacker];
                    let (pos, conn) = (a.pos, a.conn_id);
                    self.play_sound_near_except(Some(conn), THORNS_SOUND, pos, THORNS_VOLUME, 1.0);
                    if let Some(s) = worn {
                        self.damage_slot_item(victim, s, 3);
                    }
                } else if let Some(s) = worn {
                    self.damage_slot_item(victim, s, 1);
                }
                let p = &mut self.players[victim];
                if let Some(s) = worn
                    && p.inventory[s].as_ref().is_some_and(|x| x.count <= 0)
                {
                    p.inventory[s] = None;
                }
            }
        }
    }

    /// `EntityTracker.func_151248_b`: to the player and everyone tracking it.
    fn send_to_self_and_tracking(&mut self, i: usize, packet: ClientboundPlay) {
        let entity_id = self.players[i].entity_id;
        self.players[i].send(packet.clone());
        self.send_to_tracking(entity_id, &packet);
    }

    /// `EntityPlayer.canAttackPlayer` from the victim's side: PvP must be on, and
    /// teammates only hurt each other with friendly fire.
    pub(super) fn can_be_attacked_by(&self, victim: usize, attacker_entity: i32) -> bool {
        if !self.config.pvp {
            return false;
        }
        let Some(a) = self
            .players
            .iter()
            .position(|p| p.entity_id == attacker_entity)
        else {
            return true;
        };
        let Some(team) = self.team_of_player(victim) else {
            return true;
        };
        if self.team_of_player(a) != Some(team) {
            return true;
        }
        self.scoreboard
            .teams
            .get(team)
            .is_none_or(|t| t.friendly_fire)
    }

    /// `EntityPlayer.addToPlayerScore` for a player kill: the kill stat, then the
    /// victim's `killedByTeam.<killer's color>` scores, then the killer's
    /// `totalKillCount`, `playerKillCount` and `teamkill.<victim's color>` scores.
    pub(super) fn award_player_kill(&mut self, killer: usize, victim: usize) {
        self.add_stat(killer, "stat.playerKills", 1);
        let killer_name = self.players[killer].profile.name.clone();
        let victim_name = self.players[victim].profile.name.clone();
        let mut objectives = self.scoreboard.objectives_for("totalKillCount");
        objectives.extend(self.scoreboard.objectives_for("playerKillCount"));
        let color = |name: &str| {
            self.scoreboard
                .team_of(name)
                .and_then(|t| self.scoreboard.teams.get(t))
                .filter(|t| t.color < 16)
                .map(|t| super::scoreboard::FORMATTING[t.color].0)
        };
        let (killer_color, victim_color) = (color(&killer_name), color(&victim_name));
        if let Some(c) = killer_color {
            for objective in self.scoreboard.objectives_for(&format!("killedByTeam.{c}")) {
                self.sb_add_score(&victim_name, &objective, 1);
            }
        }
        if let Some(c) = victim_color {
            objectives.extend(self.scoreboard.objectives_for(&format!("teamkill.{c}")));
        }
        for objective in objectives {
            self.sb_add_score(&killer_name, &objective, 1);
        }
    }
}
