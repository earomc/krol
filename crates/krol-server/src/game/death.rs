//! Player death and respawn (vanilla: `EntityPlayerMP.onDeath`, `CombatTracker`,
//! `InventoryPlayer.dropAllItems`, `NetHandlerPlayServer.processClientStatus`,
//! `ServerConfigurationManager.recreatePlayerEntity`).
//!
//! TODO(M5): hardcore bans; killers that disconnected before the death are forgotten
//! (vanilla keeps their entity).

use krol_nbt::Tag;
use krol_protocol::packets::play::{ChunkColumn, ClientboundPlay, PlayerListAction};

use super::Game;
use super::damage::DamageSource;
use super::player::default_player_data;
use crate::chat::{Arg, Component};

/// `S42PacketCombatEvent.Event` ids.
const COMBAT_ENTER: u8 = 0;
const COMBAT_END: u8 = 1;
/// Ticks after the last hit when combat entries are forgotten, and when in combat.
const COMBAT_WINDOW: u64 = 100;
const IN_COMBAT_WINDOW: u64 = 300;
/// `EntityLivingBase.onUpdate` resets the combat tracker every 20 ticks.
pub const COMBAT_RESET_INTERVAL: u64 = 20;

/// One recorded hit (vanilla `CombatEntry`).
#[derive(Debug, Clone)]
pub struct CombatEntry {
    source: DamageSource,
    damage: f32,
    /// The fall distance when hit (`getDamageAmount`; the most for out-of-world).
    fall: f32,
    /// Where the player was (`ladder`, `vines`, `water`), for fall messages.
    location: Option<&'static str>,
}

/// Recent damage, for the death message (vanilla `CombatTracker`).
#[derive(Debug, Clone, Default)]
pub struct CombatTracker {
    entries: Vec<CombatEntry>,
    last_damage_tick: u64,
    active: bool,
    /// Hurt by a living entity (`field_94552_d`): the client is told combat started.
    in_combat: bool,
    /// When combat started and ended (`field_152775_d`, `field_152776_e`).
    combat_start: u64,
    combat_end: u64,
}

impl Game {
    /// Records damage taken (`CombatTracker.trackDamage`).
    pub(super) fn track_damage(&mut self, i: usize, source: DamageSource, damage: f32) {
        self.reset_combat(i);
        let location = self.combat_location(i);
        let p = &mut self.players[i];
        let fall = if source == DamageSource::OutOfWorld {
            f32::MAX
        } else {
            p.fall_distance
        };
        p.combat.entries.push(CombatEntry {
            source,
            damage,
            fall,
            location,
        });
        p.combat.last_damage_tick = p.ticks;
        p.combat.active = true;
        // isLivingDamageSrc: hurt by a player.
        let living = source
            .attacker()
            .is_some_and(|id| self.players.iter().any(|q| q.entity_id == id));
        let p = &mut self.players[i];
        if living && !p.combat.in_combat && p.health() > 0.0 {
            p.combat.in_combat = true;
            p.combat.combat_start = p.ticks;
            p.combat.combat_end = p.ticks;
            p.send(ClientboundPlay::CombatEvent {
                event: COMBAT_ENTER,
                duration: 0,
                entity_id: 0,
                killer: 0,
                message: String::new(),
            });
        }
    }

    /// `CombatTracker.func_94545_a`: on a ladder or vines, or in water.
    fn combat_location(&mut self, i: usize) -> Option<&'static str> {
        let p = &self.players[i];
        let (x, y, z) = (
            krol_java::math::floor_f64(p.pos.0),
            krol_java::math::floor_f64(p.pos.1),
            krol_java::math::floor_f64(p.pos.2),
        );
        let in_water = p.in_water;
        let gamemode = p.gamemode;
        match self.level.overworld.block(x, y, z) >> 4 {
            65 if gamemode != 3 => Some("ladder"),
            106 if gamemode != 3 => Some("vines"),
            _ if in_water => Some("water"),
            _ => None,
        }
    }

    /// `CombatTracker.reset`: entries are forgotten 100 ticks after the last hit (or
    /// once the player is dead).
    /// Combat ends 300 ticks after the last hit (`sendEndCombat`, with its length
    /// and the main attacker).
    pub(super) fn reset_combat(&mut self, i: usize) {
        let p = &self.players[i];
        let alive = p.health() > 0.0;
        let window = if p.combat.in_combat {
            IN_COMBAT_WINDOW
        } else {
            COMBAT_WINDOW
        };
        if !(p.combat.active
            && (!alive || p.ticks.saturating_sub(p.combat.last_damage_tick) > window))
        {
            return;
        }
        let was = p.combat.in_combat;
        let killer = self
            .best_attacker(i)
            .map_or(-1, |k| self.players[k].entity_id);
        let p = &mut self.players[i];
        p.combat.active = false;
        p.combat.in_combat = false;
        p.combat.combat_end = p.ticks;
        if was {
            let duration = p.combat.combat_end.wrapping_sub(p.combat.combat_start) as i32;
            p.send(ClientboundPlay::CombatEvent {
                event: COMBAT_END,
                duration,
                entity_id: 0,
                killer,
                message: String::new(),
            });
        }
        p.combat.entries.clear();
    }

    /// `CombatTracker.func_94550_c`: the player who dealt the most damage.
    fn best_attacker(&self, i: usize) -> Option<usize> {
        let mut best: Option<(i32, f32)> = None;
        for e in &self.players[i].combat.entries {
            if let Some(id) = e.source.attacker()
                && self.players.iter().any(|q| q.entity_id == id)
                && best.is_none_or(|(_, d)| e.damage > d)
            {
                best = Some((id, e.damage));
            }
        }
        let id = best?.0;
        self.players.iter().position(|q| q.entity_id == id)
    }

    /// The display name of a damage source's entity (`getDamageSrcDisplayName`).
    fn source_name(&self, source: DamageSource) -> Option<Component> {
        let id = source.attacker()?;
        let p = self.players.iter().find(|p| p.entity_id == id)?;
        Some(self.display_name(&p.profile))
    }

    /// The held item of a damage source's entity, if it has a custom name (for the
    /// `.item` death messages).
    fn source_named_item(&self, source: DamageSource) -> Option<Component> {
        let id = source.attacker()?;
        let p = self.players.iter().find(|p| p.entity_id == id)?;
        let held = p.held()?;
        super::item_data::custom_name(held)?;
        Some(super::item_data::chat_component(held))
    }

    /// `CombatTracker.getDeathMessage`.
    fn death_message(&self, i: usize) -> Component {
        let p = &self.players[i];
        let name = Arg::Component(self.display_name(&p.profile));
        let entries = &p.combat.entries;
        let Some(last) = entries.last() else {
            return Component::translate("death.attack.generic", vec![name]);
        };
        // func_94544_f: the entry before the farthest fall (or out-of-world), if that
        // fall was over 5 blocks. (Its second candidate, the farthest fall from a ladder,
        // vines or water, can never be chosen: vanilla compares it with a counter that
        // stays 0.) Vanilla quirk: the killing hit's trackDamage runs after the health
        // dropped to 0, so its reset() has already cleared every earlier entry; with the
        // fall as the only entry, a fatal fall is always an accident, and the assist,
        // killer and finish messages never appear in 1.8.
        let mut worst: Option<&CombatEntry> = None;
        let mut farthest = 0.0f32;
        for (k, e) in entries.iter().enumerate() {
            // CombatEntry.getDamageAmount: the fall distance, or the most for the void.
            let amount = if e.source == DamageSource::OutOfWorld {
                f32::MAX
            } else {
                e.fall
            };
            if matches!(e.source, DamageSource::Fall | DamageSource::OutOfWorld)
                && amount > 0.0
                && (worst.is_none() || amount > farthest)
            {
                worst = Some(if k > 0 { &entries[k - 1] } else { e });
                farthest = amount;
            }
        }
        let worst = worst.filter(|_| farthest > 5.0);
        if let Some(w) = worst
            && last.source == DamageSource::Fall
        {
            if matches!(w.source, DamageSource::Fall | DamageSource::OutOfWorld) {
                let suffix = w.location.unwrap_or("generic");
                return Component::translate(format!("death.fell.accident.{suffix}"), vec![name]);
            }
            // The fall itself has no attacker, so any attacker of the entry before it
            // differs from it: an assist.
            if let Some(by) = self.source_name(w.source) {
                return match self.source_named_item(w.source) {
                    Some(item) => Component::translate(
                        "death.fell.assist.item",
                        vec![name, Arg::Component(by), Arg::Component(item)],
                    ),
                    None => {
                        Component::translate("death.fell.assist", vec![name, Arg::Component(by)])
                    }
                };
            }
            return Component::translate("death.fell.killer", vec![name]);
        }
        let key = format!("death.attack.{}", last.source.name());
        let Some(by) = self.source_name(last.source) else {
            // TODO(M5): the attacker's entity outlives its connection in vanilla.
            let key = match last.source {
                DamageSource::ExplosionBy(_) => "death.attack.explosion".to_owned(),
                _ => key,
            };
            return Component::translate(key, vec![name]);
        };
        // EntityDamageSource(Indirect): ".item" when the attacker's held item has a
        // custom name and the key exists (player, arrow, thrown and indirectMagic).
        if matches!(
            last.source,
            DamageSource::Player(_)
                | DamageSource::Arrow { .. }
                | DamageSource::Thrown(_)
                | DamageSource::IndirectMagic(_)
        ) && let Some(item) = self.source_named_item(last.source)
        {
            return Component::translate(
                format!("{key}.item"),
                vec![name, Arg::Component(by), Arg::Component(item)],
            );
        }
        Component::translate(key, vec![name, Arg::Component(by)])
    }

    /// `EntityLivingBase.func_94060_bK`: the player who dealt the most damage
    /// (`CombatTracker.func_94550_c`), else the last attacking player within 100
    /// updates (`attackingPlayer`).
    fn killer_of(&self, i: usize) -> Option<usize> {
        if let Some(k) = self.best_attacker(i) {
            return Some(k);
        }
        let (id, _) = self.players[i].attacking_player.filter(|&(_, t)| t > 0)?;
        self.players.iter().position(|q| q.entity_id == id)
    }

    /// Vanilla `EntityPlayerMP.onDeath`.
    pub(super) fn player_died(&mut self, i: usize) {
        let rules_show = self.level.data.game_rule("showDeathMessages") != Some("false");
        if rules_show {
            let message = self.death_message(i);
            self.send_death_message(i, &message);
        }
        if self.level.data.game_rule("keepInventory") != Some("true") {
            self.drop_all_items(i);
        }
        let name = self.players[i].profile.name.clone();
        self.sb_increment(&name, super::scoreboard::DEATH_COUNT);
        if let Some(killer) = self.killer_of(i) {
            self.award_player_kill(killer, i);
        }
        self.add_stat(i, "stat.deaths", 1);
        self.reset_stat(i, "stat.timeSinceDeath");
        self.reset_combat(i);
    }

    /// The death message by the team's death message visibility: everyone, only
    /// teammates (`sendMessageToAllTeamMembers`), only the others
    /// (`sendMessageToTeamOrEvryPlayer`), or nobody.
    fn send_death_message(&self, i: usize, message: &Component) {
        let team = self.team_of_player(i).map(str::to_owned);
        let visibility = team
            .as_deref()
            .and_then(|t| self.scoreboard.teams.get(t))
            .map_or("always", |t| t.death_messages);
        let chat = || ClientboundPlay::Chat {
            json: message.to_json_string(),
            position: 1,
        };
        match (visibility, team.as_deref()) {
            ("hideForOtherTeams", Some(team)) => {
                let Some(t) = self.scoreboard.teams.get(team) else {
                    return;
                };
                for (member, _) in t.members.iter() {
                    if let Some(p) = self
                        .players
                        .iter()
                        .find(|p| p.profile.name.eq_ignore_ascii_case(member))
                        && p.conn_id != self.players[i].conn_id
                    {
                        p.send(chat());
                    }
                }
            }
            ("hideForOwnTeam", Some(team)) => {
                for (k, p) in self.players.iter().enumerate() {
                    if self.team_of_player(k) != Some(team) {
                        p.send(chat());
                    }
                }
            }
            ("never", _) => {}
            _ => self.broadcast_chat(message, None),
        }
    }

    /// Vanilla `InventoryPlayer.dropAllItems`: main inventory, then armor, each thrown
    /// in a random direction.
    fn drop_all_items(&mut self, i: usize) {
        // mainInventory index -> window slot, then armor (boots first) -> window 8..5.
        let slots: Vec<usize> = (0..36)
            .map(|k| if k < 9 { 36 + k } else { k })
            .chain([8, 7, 6, 5])
            .collect();
        for slot in slots {
            let Some(stack) = self.players[i].inventory[slot].take() else {
                continue;
            };
            let p = &mut self.players[i];
            let y = p.pos.1 - f64::from(0.3f32) + f64::from(1.62f32);
            let r = &mut p.rand;
            let speed = r.next_float() * 0.5;
            let angle = r.next_float() * std::f32::consts::PI * 2.0;
            let motion = (
                f64::from(-krol_java::math::sin(angle) * speed),
                f64::from(0.2f32),
                f64::from(krol_java::math::cos(angle) * speed),
            );
            let pos = (p.pos.0, y, p.pos.2);
            self.spawn_item_moving(pos, stack, 40, Some(motion));
        }
    }

    /// Vanilla `processClientStatus`.
    pub(super) fn client_status(&mut self, i: usize, action: i32) {
        // TODO(M5): hardcore worlds ban the player instead; statistics requests (1)
        // and the openInventory achievement (2).
        if action == 0 && self.players[i].health() <= 0.0 {
            self.respawn(i);
        }
    }

    /// Vanilla `recreatePlayerEntity` (same dimension, not from the End).
    fn respawn(&mut self, i: usize) {
        let entity_id = self.players[i].entity_id;
        // The old entity stays a dead viewer of the maps it saw.
        self.maps.player_entity_replaced(entity_id);
        // removePlayerFromTrackers: the player stops seeing everything (players are
        // destroyed on its client right away).
        let seen: Vec<i32> = self
            .tracker
            .iter()
            .filter(|(_, e)| e.is_tracked_by(entity_id))
            .map(|(id, _)| *id)
            .collect();
        for id in seen {
            if let Some(entry) = self.tracker.get_mut(&id) {
                entry.stop_tracking(entity_id);
            }
            if self.players.iter().any(|p| p.entity_id == id) {
                self.players[i].send(ClientboundPlay::DestroyEntities(vec![id]));
            }
        }
        // untrackEntity: the others destroy the old player.
        self.untrack_player(i);
        // PlayerManager.removePlayer: unload every watched chunk.
        let sky = self.level.overworld.has_sky;
        let watched = self.stop_watching_all(i);
        for (x, z) in watched {
            let (_, data) = self.level.overworld.chunk(x, z).packet_data(true, 0, sky);
            self.players[i].send(ClientboundPlay::ChunkData {
                column: ChunkColumn {
                    x,
                    z,
                    mask: 0,
                    data,
                },
                full: true,
            });
        }

        // The new player entity: defaults, keeping what clonePlayer keeps.
        let keep = self.level.data.game_rule("keepInventory") == Some("true");
        let default_mode = self.config.gamemode;
        let pos = self.spawn_position(default_mode);
        let p = &mut self.players[i];
        let gamemode = p.gamemode;
        // clonePlayer does not keep the spawn point: only a bed found again sets it.
        let spawn_point = super::beds::spawn_point(&p.data);
        let mut data = default_player_data(gamemode);
        for key in ["XpSeed", "EnderItems"] {
            if let Some(v) = p.data.get(key) {
                data.insert(key, v.clone());
            }
        }
        if keep {
            for key in ["XpLevel", "XpTotal", "XpP", "Score"] {
                if let Some(v) = p.data.get(key) {
                    data.insert(key, v.clone());
                }
            }
        } else {
            p.inventory = vec![None; 45];
        }
        data.insert("playerGameType", Tag::Int(gamemode));
        p.data = data;
        p.pos = pos;
        p.rotation = (0.0, 0.0);
        p.motion = (0.0, 0.0, 0.0);
        p.on_ground = false;
        p.fall_distance = 0.0;
        p.first_update = true;
        p.using = None;
        p.effects.clear();
        p.potions_dirty = false;
        p.effect_color = 0;
        p.effect_ambient = false;
        p.arrow_count = 0;
        p.arrow_hit_timer = 0;
        p.in_water = false;
        p.sneaking = false;
        p.sprinting = false;
        p.combat = CombatTracker::default();
        p.hurt_resistant_time = 0;
        p.respawn_invulnerability = 60;
        p.last_health_sent = None;
        p.combined_health = super::player::NO_HEALTH_YET;
        p.dirty_metadata = super::tracker::PLAYER_METADATA_INDICES
            .into_iter()
            .collect();
        p.dirty_attributes = [super::attributes::MOVEMENT_SPEED].into_iter().collect();
        p.modifiers.clear();
        p.last_equipment = Default::default();
        p.sent_inventory = vec![None; 45];
        p.dig = super::digging::DigState::new();
        p.pending_destroy.clear();
        p.entity_chunk = (
            krol_java::math::floor_f64(pos.0) >> 4,
            krol_java::math::floor_f64(pos.2) >> 4,
        );

        // setPlayerGameTypeBasedOnOther: the game type is set twice (abilities to the
        // player, the list update to everyone else each time).
        let uuid = p.profile.id;
        for _ in 0..2 {
            self.players[i].send(super::abilities_packet(gamemode));
            let update = ClientboundPlay::PlayerListItem(PlayerListAction::UpdateGamemode(vec![(
                uuid, gamemode,
            )]));
            for (k, other) in self.players.iter().enumerate() {
                if k != i {
                    other.send(update.clone());
                }
            }
        }
        // The bed of the spawn point, if it is still there (else the player is told).
        let mut pos = pos;
        if let Some(at) = self.bed_respawn(i, spawn_point) {
            pos = at;
            while pos.1 < 255.0 {
                let bb = super::physics::Aabb::at(pos, 0.6, 1.8);
                if super::physics::colliding_boxes(&mut self.level.overworld, &bb).is_empty() {
                    break;
                }
                pos.1 += 1.0;
            }
            let p = &mut self.players[i];
            p.pos = pos;
            p.entity_chunk = (
                krol_java::math::floor_f64(pos.0) >> 4,
                krol_java::math::floor_f64(pos.2) >> 4,
            );
        }
        let difficulty = self.config.difficulty as u8;
        let level_type = self.level.data.generator_name().to_owned();
        let p = &mut self.players[i];
        p.send(ClientboundPlay::Respawn {
            dimension: 0,
            difficulty,
            gamemode: gamemode as u8,
            level_type,
        });
        Self::set_player_location(p, pos, (0.0, 0.0));
        let level = &self.level.data;
        let spawn = (level.spawn_x(), level.spawn_y(), level.spawn_z());
        let p = &self.players[i];
        p.send(ClientboundPlay::SpawnPosition { pos: spawn });
        let d = &p.data;
        p.send(ClientboundPlay::SetExperience {
            bar: d.get_f32("XpP"),
            level: d.get_i32("XpLevel"),
            total: d.get_i32("XpTotal"),
        });
        self.send_time_and_weather(i);
        let radius = self.config.view_distance;
        self.players[i].start_watching(radius);
        for (x, z) in self.players[i].take_new_watches() {
            self.level.overworld.load_watched(x, z);
        }
        self.players[i].removed = false;
        self.players[i].dead_marked = false;
        // The new entity's ticksExisted starts over.
        self.players[i].spawn_tick = self.players[i].ticks;
        // spawnEntityInWorld and playerEntityList.add: the new entity joins the end of
        // the world's lists (keeping the old entity id).
        self.players[i].load_order = self.next_entity_id;
        let player = self.players.remove(i);
        self.players.push(player);
        let i = self.players.len() - 1;
        self.track_player(i);
        // addSelfToInternalCraftingInventory.
        let p = &self.players[i];
        p.send(ClientboundPlay::WindowItems {
            window: 0,
            items: p.inventory.clone(),
        });
        p.send(ClientboundPlay::SetSlot {
            window: -1,
            slot: -1,
            item: None,
        });
        self.detect_inventory_changes(i);
    }
}
