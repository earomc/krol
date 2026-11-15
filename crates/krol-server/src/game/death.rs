//! Player death and respawn (vanilla: `EntityPlayerMP.onDeath`, `CombatTracker`,
//! `InventoryPlayer.dropAllItems`, `NetHandlerPlayServer.processClientStatus`,
//! `ServerConfigurationManager.recreatePlayerEntity`).
//!
//! TODO(M5): hardcore bans, team death message visibility, deathCount scoreboard
//! criteria, killer statistics and scores, experience orbs, bed spawn points.

use krol_nbt::Tag;
use krol_protocol::packets::play::{ChunkColumn, ClientboundPlay, PlayerListAction};

use super::Game;
use super::damage::DamageSource;
use super::player::default_player_data;
use crate::chat::{Arg, Component};

/// One recorded hit (vanilla `CombatEntry`).
#[derive(Debug, Clone)]
pub struct CombatEntry {
    source: DamageSource,
    damage: f32,
    /// Where the player was (`ladder`, `vines`, `water`), for fall messages.
    location: Option<&'static str>,
}

/// Recent damage, for the death message (vanilla `CombatTracker`).
#[derive(Debug, Clone, Default)]
pub struct CombatTracker {
    entries: Vec<CombatEntry>,
    last_damage_tick: u64,
    active: bool,
}

impl Game {
    /// Records damage taken (`CombatTracker.trackDamage`).
    pub(super) fn track_damage(&mut self, i: usize, source: DamageSource, damage: f32) {
        self.reset_combat(i);
        let location = self.combat_location(i);
        let p = &mut self.players[i];
        p.combat.entries.push(CombatEntry {
            source,
            damage,
            location,
        });
        p.combat.last_damage_tick = p.ticks;
        p.combat.active = true;
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
    pub(super) fn reset_combat(&mut self, i: usize) {
        let p = &mut self.players[i];
        let alive = p.health() > 0.0;
        if p.combat.active && (!alive || p.ticks.saturating_sub(p.combat.last_damage_tick) > 100) {
            p.combat = CombatTracker::default();
        }
    }

    /// `CombatTracker.getDeathMessage` for the damage sources krol has.
    fn death_message(&self, i: usize) -> Component {
        let p = &self.players[i];
        let name = Arg::Component(Self::display_name(&p.profile));
        let entries = &p.combat.entries;
        let Some(last) = entries.last() else {
            return Component::translate("death.attack.generic", vec![name]);
        };
        // The worst fall (over 5 damage) decides the message (`func_94544_f`).
        let mut worst: Option<&CombatEntry> = None;
        let mut worst_damage = 0.0;
        for (k, e) in entries.iter().enumerate() {
            if e.source == DamageSource::Fall
                && e.damage > 0.0
                && (worst.is_none() || e.damage > worst_damage)
            {
                worst = Some(if k > 0 { &entries[k - 1] } else { e });
                worst_damage = e.damage;
            }
        }
        let worst = worst.filter(|_| worst_damage > 5.0);
        match (worst, last.source) {
            (Some(w), DamageSource::Fall) if w.source == DamageSource::Fall => {
                let suffix = w.location.unwrap_or("generic");
                Component::translate(format!("death.fell.accident.{suffix}"), vec![name])
            }
            // EntityDamageSourceIndirect: the player who lit the TNT is named too.
            (_, DamageSource::ExplosionBy(igniter)) => {
                match self.players.iter().find(|p| p.entity_id == igniter) {
                    Some(by) => Component::translate(
                        "death.attack.explosion.player",
                        vec![name, Arg::Component(Self::display_name(&by.profile))],
                    ),
                    // TODO(M5): the igniter's entity outlives its connection in vanilla.
                    None => Component::translate("death.attack.explosion", vec![name]),
                }
            }
            // TODO(M5): falls finished or assisted by an attacker.
            _ => Component::translate(format!("death.attack.{}", last.source.name()), vec![name]),
        }
    }

    /// Vanilla `EntityPlayerMP.onDeath`.
    pub(super) fn player_died(&mut self, i: usize) {
        let rules_show = self.level.data.game_rule("showDeathMessages") != Some("false");
        if rules_show {
            let message = self.death_message(i);
            self.broadcast_chat(&message, None);
        }
        if self.level.data.game_rule("keepInventory") != Some("true") {
            self.drop_all_items(i);
        }
        let p = &mut self.players[i];
        p.stats.add("stat.deaths", 1);
        p.stats.set("stat.timeSinceDeath", 0);
        self.reset_combat(i);
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
        let watched: Vec<(i32, i32)> = self.players[i].watching.drain().collect();
        self.players[i].chunk_queue.clear();
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
        let mut data = default_player_data(gamemode);
        for key in [
            "XpSeed",
            "EnderItems",
            "SpawnX",
            "SpawnY",
            "SpawnZ",
            "SpawnForced",
        ] {
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
        p.in_water = false;
        p.sneaking = false;
        p.sprinting = false;
        p.combat = CombatTracker::default();
        p.hurt_resistant_time = 0;
        p.respawn_invulnerability = 60;
        p.last_health_sent = None;
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
            self.level.overworld.chunk(x, z);
        }
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
