//! Player movement packets and server-side player physics (vanilla:
//! `NetHandlerPlayServer.processPlayer`, `EntityLivingBase.moveEntityWithHeading`,
//! `EntityPlayer.jump`, `EntityPlayer.addMovementStat`, `EntityPlayerMP.handleFalling`).
//!
//! Every accepted movement packet first runs the player's living update (vanilla calls
//! `onUpdateEntity` from `processPlayer`): the server applies gravity and friction to
//! its own copy of the player and moves it, then puts it back where the last packet
//! left it. Only then is the client's move checked by moving the bounding box through
//! the world. The side effects of both moves (step sounds, ground state, statistics)
//! are what clients observe.

use krol_java::math::floor_f64;
use krol_nbt::Tag;
use krol_protocol::packets::play::{ChunkColumn, ClientboundPlay};

use super::Game;
use super::physics::{self, Aabb, Body, MoveOutcome};

/// Splash volume per unit of (weighted) speed (`resetHeight`).
const SPLASH_VOLUME_FACTOR: f32 = 0.2;
/// Random pitch spread of swim and splash sounds.
const SPLASH_PITCH_SPREAD: f32 = 0.4;

/// Updates after death until the body disappears and drops its experience.
const DEATH_ANIMATION_TICKS: i16 = 20;
/// Experience a dead player drops per level.
const XP_PER_LEVEL_DROPPED: i32 = 7;
/// The most experience a dead player drops.
const MAX_XP_DROPPED: i32 = 100;
/// Explosion particles when the body disappears.
const DEATH_PARTICLES: usize = 20;

/// A full air supply (`Entity.getAir`).
const MAX_AIR: i16 = 300;
/// The air supply at which a drowning player takes damage (and goes back to 0).
const DROWN_AT: i16 = -20;
/// Suffocation damage per tick inside a block.
const SUFFOCATION_DAMAGE: f32 = 1.0;
/// A player's width (`EntityPlayer.setSize(0.6, 1.8)`).
const PLAYER_WIDTH: f32 = 0.6;
/// The spectator game mode.
const SPECTATOR: i32 = 3;
/// Drowning damage.
const DROWN_DAMAGE: f32 = 2.0;
/// Bubble particles spawned when drowning damage hits.
const DROWN_BUBBLES: usize = 8;
/// `isInsideOfMaterial`: the eyes count as under water a ninth below the surface.
const SURFACE_OFFSET: f32 = 0.111_111_11;

/// `(double) 0.98F`.
const DRAG_Y: f64 = 0.980_000_019_073_486_3;

impl Game {
    /// The player's body for a physics step.
    fn body(&self, i: usize) -> Body {
        let p = &self.players[i];
        let mut b = Body::player(p.pos, p.on_ground);
        b.motion = p.motion;
        b.sneaking = p.sneaking;
        b.distance_walked_on_step = p.distance_walked_on_step;
        b.next_step_distance = p.next_step_distance;
        b.in_water = p.in_water;
        b
    }

    /// Moves the player's bounding box through the world (`moveEntity`) and plays the
    /// resulting step sound to the players nearby.
    fn move_player(&mut self, i: usize, d: (f64, f64, f64)) -> MoveOutcome {
        let mut body = self.body(i);
        let outcome = physics::move_entity(&mut self.level.overworld, &mut body, d.0, d.1, d.2);
        let p = &mut self.players[i];
        p.pos = body.pos;
        p.motion = body.motion;
        p.on_ground = body.on_ground;
        p.collided_horizontally = body.collided_horizontally;
        p.distance_walked_on_step = body.distance_walked_on_step;
        p.next_step_distance = body.next_step_distance;
        if let Some(volume) = outcome.swim {
            let pitch = 1.0 + (p.rand.next_float() - p.rand.next_float()) * SPLASH_PITCH_SPREAD;
            let (pos, conn) = (p.pos, p.conn_id);
            self.play_sound_near_except(Some(conn), "game.player.swim", pos, volume, pitch);
        }
        let p = &self.players[i];
        if let Some(step) = &outcome.step {
            let (pos, conn) = (p.pos, p.conn_id);
            self.play_sound_near_except(Some(conn), &step.name, pos, step.volume, step.pitch);
        }
        // moveEntity ends with doBlockCollisions (spectators do not collide).
        let p = &self.players[i];
        if p.gamemode != 3 {
            let bb = Aabb::at(p.pos, 0.6, 1.8);
            self.block_collisions_of(&bb, Some(i));
            self.player_move_fire(i);
        }
        outcome
    }

    /// Sends a sound to every player within hearing distance except `except`
    /// (`World.playSoundToNearExcept` / `sendToAllNearExcept`).
    pub(super) fn play_sound_near_except(
        &self,
        except: Option<u64>,
        name: &str,
        pos: (f64, f64, f64),
        volume: f32,
        pitch: f32,
    ) {
        let range = if volume > 1.0 {
            f64::from(16.0 * volume)
        } else {
            16.0
        };
        let packet = ClientboundPlay::SoundEffect {
            name: name.to_owned(),
            x: (pos.0 * 8.0) as i32,
            y: (pos.1 * 8.0) as i32,
            z: (pos.2 * 8.0) as i32,
            volume,
            pitch: ((pitch * 63.0) as i32).clamp(0, 255) as u8,
        };
        for p in &self.players {
            if Some(p.conn_id) == except {
                continue;
            }
            let d = (p.pos.0 - pos.0, p.pos.1 - pos.1, p.pos.2 - pos.2);
            if d.0 * d.0 + d.1 * d.1 + d.2 * d.2 < range * range {
                p.send(packet.clone());
            }
        }
    }

    /// Whether any block in `bb` has the given ids (`World.isMaterialInBB` for liquids).
    fn any_block_in(&mut self, bb: &Aabb, ids: std::ops::RangeInclusive<u16>) -> bool {
        for x in floor_f64(bb.min[0])..floor_f64(bb.max[0] + 1.0) {
            for y in floor_f64(bb.min[1])..floor_f64(bb.max[1] + 1.0) {
                for z in floor_f64(bb.min[2])..floor_f64(bb.max[2] + 1.0) {
                    if ids.contains(&(self.level.overworld.block(x, y, z) >> 4)) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Whether the player stands in a ladder or vine (`isOnLadder`).
    fn on_ladder(&mut self, i: usize) -> bool {
        let p = &self.players[i];
        if p.gamemode == 3 {
            return false;
        }
        let (x, y, z) = (floor_f64(p.pos.0), floor_f64(p.pos.1), floor_f64(p.pos.2));
        matches!(self.level.overworld.block(x, y, z) >> 4, 65 | 106)
    }

    /// Block slipperiness below the player (`Block.slipperiness`).
    fn slipperiness_below(&mut self, i: usize) -> f32 {
        let p = &self.players[i];
        let (x, y, z) = (
            floor_f64(p.pos.0),
            floor_f64(p.pos.1) - 1,
            floor_f64(p.pos.2),
        );
        match self.level.overworld.block(x, y, z) >> 4 {
            79 | 174 => 0.98,
            165 => 0.8,
            _ => 0.6,
        }
    }

    /// The part of the living update that moves the player (`onLivingUpdate` with no
    /// input: `moveEntityWithHeading(0, 0)`), then `EntityPlayer`'s movement statistics.
    ///
    /// TODO(M5): the rest of the living update (fire, air, potions, food, regeneration,
    /// entity pushing).
    /// `EntityLivingBase.onUpdate`: equipment changes go to the viewers.
    fn send_equipment_changes(&mut self, i: usize) {
        let player = &mut self.players[i];
        let equipment = super::tracker::equipment(player);
        let mut changed = Vec::new();
        for (slot, item) in equipment.into_iter().enumerate() {
            if player.last_equipment[slot] != item {
                player.last_equipment[slot] = item.clone();
                changed.push(ClientboundPlay::EntityEquipment {
                    entity_id: player.entity_id,
                    slot: slot as i16,
                    item,
                });
            }
        }
        let entity_id = player.entity_id;
        for packet in &changed {
            self.send_to_tracking(entity_id, packet);
        }
    }

    /// `Entity.resetHeight` for a player entering water: the splash sound for the
    /// others nearby (the particles are client-side).
    fn water_splash(&mut self, i: usize) {
        let p = &mut self.players[i];
        let m = p.motion;
        let volume = (krol_java::math::sqrt_f64(
            m.0 * m.0 * f64::from(0.2f32) + m.1 * m.1 + m.2 * m.2 * f64::from(0.2f32),
        ) * SPLASH_VOLUME_FACTOR)
            .min(1.0);
        let pitch = 1.0 + (p.rand.next_float() - p.rand.next_float()) * SPLASH_PITCH_SPREAD;
        let (pos, conn) = (p.pos, p.conn_id);
        self.play_sound_near_except(Some(conn), "game.player.swim.splash", pos, volume, pitch);
    }

    /// `Entity.isInsideOfMaterial(water)` for a player: the eyes are below the water's
    /// surface.
    fn eyes_in_water(&mut self, i: usize) -> bool {
        let p = &self.players[i];
        let eye_y = p.pos.1 + f64::from(p.eye_height());
        let (x, y, z) = (floor_f64(p.pos.0), floor_f64(eye_y), floor_f64(p.pos.2));
        let state = self.level.overworld.block(x, y, z);
        if super::block_data::material(state >> 4) != super::block_data::Material::Water {
            return false;
        }
        let gap = super::liquids::liquid_height_percent(state & 15) - SURFACE_OFFSET;
        eye_y < f64::from((y + 1) as f32 - gap)
    }

    /// `Entity.isEntityInsideOpaqueBlock` for a player: any of 8 points around the eyes
    /// in a visually opaque block (spectators pass through blocks).
    fn inside_opaque_block(&mut self, i: usize) -> bool {
        let p = &self.players[i];
        if p.gamemode == SPECTATOR {
            return false;
        }
        let (pos, eye) = (p.pos, p.eye_height());
        let mut last = None;
        for k in 0..8 {
            let side = |bit: i32| (bit % 2) as f32 - 0.5;
            let y = floor_f64(pos.1 + f64::from(side(k) * 0.1) + f64::from(eye));
            let x = floor_f64(pos.0 + f64::from(side(k >> 1) * PLAYER_WIDTH * 0.8));
            let z = floor_f64(pos.2 + f64::from(side(k >> 2) * PLAYER_WIDTH * 0.8));
            if last == Some((x, y, z)) {
                continue;
            }
            last = Some((x, y, z));
            let id = self.level.overworld.block(x, y, z) >> 4;
            if super::light_data::visually_opaque(id) {
                return true;
            }
        }
        false
    }

    /// `EntityLivingBase.onEntityUpdate`'s air supply: it runs out under water (unless
    /// damage is disabled) and every 20 ticks without air drown for 2; it refills at
    /// once out of water.
    fn air_tick(&mut self, i: usize) {
        if self.players[i].health() <= 0.0 {
            return;
        }
        let set_air = |p: &mut super::player::Player, air: i16| {
            if p.data.get_i16("Air") != air {
                p.data.insert("Air", Tag::Short(air));
                p.dirty_metadata.insert(1);
            }
        };
        if !self.eyes_in_water(i) {
            set_air(&mut self.players[i], MAX_AIR);
            return;
        }
        let p = &mut self.players[i];
        if matches!(p.gamemode, 1 | 3) {
            return;
        }
        // decreaseAirSupply: respiration can skip the decrease.
        let respiration =
            super::item_data::armor_enchantment(&p.inventory, super::item_data::RESPIRATION);
        let air = p.data.get_i16("Air");
        let keep = respiration > 0 && p.rand.next_int_bounded(respiration + 1) > 0;
        let air = if keep { air } else { air - 1 };
        set_air(p, air);
        if air == DROWN_AT {
            set_air(p, 0);
            // The bubble particles (client-side) still draw from the entity's random.
            for _ in 0..DROWN_BUBBLES * 6 {
                p.rand.next_float();
            }
            self.attack_player(i, super::damage::DamageSource::Drown, DROWN_DAMAGE);
        }
    }

    /// `EntityLivingBase.onDeathUpdate` for a dead player: after 20 updates the
    /// experience drops as orbs (`getExperiencePoints`: 7 per level, at most 100,
    /// none with keepInventory).
    ///
    /// TODO(M5): the entity is then removed (`setDead`, untracked for the others).
    fn death_update(&mut self, i: usize) {
        let p = &mut self.players[i];
        let time = p.data.get_i16("DeathTime").wrapping_add(1);
        p.data.insert("DeathTime", Tag::Short(time));
        if time != DEATH_ANIMATION_TICKS {
            return;
        }
        let rules = &self.level.data;
        let keep = rules.game_rule("keepInventory") == Some("true");
        let loot = rules.game_rule("doMobLoot") != Some("false");
        let p = &mut self.players[i];
        if loot && !keep {
            let xp = p
                .data
                .get_i32("XpLevel")
                .wrapping_mul(XP_PER_LEVEL_DROPPED)
                .min(MAX_XP_DROPPED);
            let pos = p.pos;
            self.spawn_orbs(pos, xp);
        }
        // The explosion particles (client-side) still draw from the entity's random.
        let p = &mut self.players[i];
        for _ in 0..DEATH_PARTICLES {
            p.rand.next_gaussian();
            p.rand.next_gaussian();
            p.rand.next_gaussian();
            p.rand.next_float();
            p.rand.next_float();
            p.rand.next_float();
        }
    }

    fn living_move(&mut self, i: usize) {
        // EntityPlayer.onUpdate, before the living update.
        self.item_use_update(i);
        let p = &mut self.players[i];
        if p.xp_cooldown > 0 {
            p.xp_cooldown -= 1;
        }
        self.send_equipment_changes(i);
        // Entity.handleWaterMovement: flowing water pushes players who are not flying.
        let (water, push) = {
            let p = &self.players[i];
            let bb = Aabb::at(p.pos, 0.6, 1.8)
                .expand([0.0, f64::from(-0.4f32), 0.0])
                .contract([0.001, 0.001, 0.001]);
            let pushed = !p.flying;
            super::liquids::water_movement(&mut self.level.overworld, &bb, pushed)
        };
        let p = &mut self.players[i];
        p.motion.0 += push.0;
        p.motion.1 += push.1;
        p.motion.2 += push.2;
        if water {
            if !p.in_water && !p.first_update {
                self.water_splash(i);
            }
            let p = &mut self.players[i];
            p.fall_distance = 0.0;
            // TODO(M5): extinguishing (fire = 0).
        }
        let p = &mut self.players[i];
        p.in_water = water;
        p.first_update = false;
        let in_lava = {
            let bb = Aabb::at(self.players[i].pos, 0.6, 1.8).expand([
                f64::from(-0.1f32),
                f64::from(-0.4f32),
                f64::from(-0.1f32),
            ]);
            self.any_block_in(&bb, 10..=11)
        };
        self.player_fire_update(i, in_lava);
        // EntityLivingBase.onEntityUpdate: suffocation.
        // TODO(M5): damage outside the world border.
        if self.players[i].health() > 0.0 && self.inside_opaque_block(i) {
            self.attack_player(i, super::damage::DamageSource::InWall, SUFFOCATION_DAMAGE);
        }
        self.air_tick(i);
        self.player_extinguish_if_wet(i);
        let p = &mut self.players[i];
        let hurt = p.data.get_i16("HurtTime");
        if hurt > 0 {
            p.data.insert("HurtTime", Tag::Short(hurt - 1));
        }
        if self.players[i].health() <= 0.0 {
            self.death_update(i);
        }
        self.update_effects(i);
        let p = &mut self.players[i];
        for m in [&mut p.motion.0, &mut p.motion.1, &mut p.motion.2] {
            if m.abs() < 0.005 {
                *m = 0.0;
            }
        }
        let start = p.pos;
        let flying = p.flying;
        let saved_y = p.motion.1;
        let lava = !water && {
            let bb = Aabb::at(start, 0.6, 1.8).expand([-0.1, -0.4, -0.1]);
            self.any_block_in(&bb, 10..=11)
        };
        if water && !flying {
            let y0 = start.1;
            let m = self.players[i].motion;
            self.move_player(i, m);
            let p = &mut self.players[i];
            p.motion.0 *= f64::from(0.8f32);
            p.motion.1 *= f64::from(0.8f32);
            p.motion.2 *= f64::from(0.8f32);
            p.motion.1 -= 0.02;
            self.liquid_climb(i, y0);
        } else if lava && !flying {
            let y0 = start.1;
            let m = self.players[i].motion;
            self.move_player(i, m);
            let p = &mut self.players[i];
            p.motion.0 *= 0.5;
            p.motion.1 *= 0.5;
            p.motion.2 *= 0.5;
            p.motion.1 -= 0.02;
            self.liquid_climb(i, y0);
        } else {
            let friction = if self.players[i].on_ground {
                self.slipperiness_below(i) * 0.91
            } else {
                0.91
            };
            let ladder = self.on_ladder(i);
            if ladder {
                let p = &mut self.players[i];
                p.motion.0 = p.motion.0.clamp(-0.15, 0.15);
                p.motion.2 = p.motion.2.clamp(-0.15, 0.15);
                p.fall_distance = 0.0;
                if p.motion.1 < -0.15 {
                    p.motion.1 = -0.15;
                }
                if p.sneaking && p.motion.1 < 0.0 {
                    p.motion.1 = 0.0;
                }
            }
            let m = self.players[i].motion;
            self.move_player(i, m);
            let ladder_now = self.on_ladder(i);
            let p = &mut self.players[i];
            if p.collided_horizontally && ladder_now {
                p.motion.1 = 0.2;
            }
            p.motion.1 -= 0.08;
            p.motion.1 *= DRAG_Y;
            p.motion.0 *= f64::from(friction);
            p.motion.2 *= f64::from(friction);
        }
        if flying {
            self.players[i].motion.1 = saved_y * 0.6;
        }
        let end = self.players[i].pos;
        self.add_movement_stat(i, (end.0 - start.0, end.1 - start.1, end.2 - start.2));
        self.push_nearby(i);
        // EntityPlayer.onLivingUpdate: touching items.
        self.pick_up_items(i);
    }

    /// Vanilla `collideWithNearbyEntities`: overlapping entities push each other apart
    /// (`Entity.applyEntityCollision`). Spectators neither push nor get pushed.
    ///
    /// TODO(M5): entities other than players.
    fn push_nearby(&mut self, i: usize) {
        if self.players[i].gamemode == 3 {
            return;
        }
        let area = Aabb::at(self.players[i].pos, 0.6, 1.8).expand([0.2, 0.0, 0.2]);
        for j in 0..self.players.len() {
            let other = &self.players[j];
            if j == i || other.gamemode == 3 || !area.intersects(&Aabb::at(other.pos, 0.6, 1.8)) {
                continue;
            }
            let me = self.players[i].pos;
            let (mut dx, mut dz) = (me.0 - other.pos.0, me.2 - other.pos.2);
            let mut d = dx.abs().max(dz.abs());
            if d < f64::from(0.01f32) {
                continue;
            }
            d = f64::from(krol_java::math::sqrt_f64(d));
            dx /= d;
            dz /= d;
            let scale = (1.0 / d).min(1.0);
            dx *= scale * f64::from(0.05f32);
            dz *= scale * f64::from(0.05f32);
            let other = &mut self.players[j];
            other.motion.0 -= dx;
            other.motion.2 -= dz;
            other.airborne = true;
            let me = &mut self.players[i];
            me.motion.0 += dx;
            me.motion.2 += dz;
            me.airborne = true;
        }
    }

    /// Swimming out of a liquid onto a ledge (`isOffsetPositionInLiquid` check).
    fn liquid_climb(&mut self, i: usize, y0: f64) {
        let p = &self.players[i];
        if !p.collided_horizontally {
            return;
        }
        let m = p.motion;
        let offset = [m.0, m.1 + f64::from(0.6f32) - p.pos.1 + y0, m.2];
        let bb = Aabb::at(p.pos, 0.6, 1.8).offset(offset);
        let free = physics::colliding_boxes(&mut self.level.overworld, &bb).is_empty()
            && !self.any_block_in(&bb, 8..=11);
        if free {
            self.players[i].motion.1 = f64::from(0.3f32);
        }
    }

    /// Vanilla `EntityPlayer.jump`.
    fn jump(&mut self, i: usize) {
        let p = &mut self.players[i];
        // TODO(M5): jump boost.
        p.motion.1 = f64::from(0.42f32);
        if p.sprinting {
            let yaw = p.rotation.0 * (std::f32::consts::PI / 180.0);
            p.motion.0 -= f64::from(krol_java::math::sin(yaw) * 0.2);
            p.motion.2 += f64::from(krol_java::math::cos(yaw) * 0.2);
        }
        p.airborne = true;
        p.stats.add("stat.jump", 1);
        let amount = if p.sprinting { 0.8 } else { 0.2 };
        self.add_exhaustion(i, amount);
    }

    /// Vanilla `EntityPlayer.addExhaustion` (`FoodStats.addExhaustion`).
    pub(super) fn add_exhaustion(&mut self, i: usize, amount: f32) {
        let p = &mut self.players[i];
        // Creative and spectator players take no damage and get no exhaustion.
        if p.gamemode == 1 || p.gamemode == 3 {
            return;
        }
        let level = (p.data.get_f32("foodExhaustionLevel") + amount).min(40.0);
        p.data.insert("foodExhaustionLevel", Tag::Float(level));
    }

    /// Vanilla `EntityPlayer.addMovementStat`.
    fn add_movement_stat(&mut self, i: usize, d: (f64, f64, f64)) {
        // TODO(M5): riding.
        let eye_water = {
            let p = &self.players[i];
            let eye = p.pos.1 + f64::from(1.62f32);
            let (x, y, z) = (floor_f64(p.pos.0), floor_f64(eye), floor_f64(p.pos.2));
            let state = self.level.overworld.block(x, y, z);
            if matches!(state >> 4, 8 | 9) {
                // BlockLiquid.getLiquidHeightPercent.
                let mut level = i32::from(state & 15);
                if level >= 8 {
                    level = 0;
                }
                let filled = (level + 1) as f32 / 9.0 - 0.111_111_11;
                eye < f64::from((y + 1) as f32 - filled)
            } else {
                false
            }
        };
        let ladder = self.on_ladder(i);
        let horizontal =
            || (krol_java::math::sqrt_f64(d.0 * d.0 + d.2 * d.2) * 100.0).round() as i32;
        let p = &mut self.players[i];
        let mut exhaustion = 0.0f32;
        if eye_water {
            let cm = (krol_java::math::sqrt_f64(d.0 * d.0 + d.1 * d.1 + d.2 * d.2) * 100.0).round()
                as i32;
            if cm > 0 {
                p.stats.add("stat.diveOneCm", cm);
                exhaustion = 0.015 * cm as f32 * 0.01;
            }
        } else if p.in_water {
            let cm = horizontal();
            if cm > 0 {
                p.stats.add("stat.swimOneCm", cm);
                exhaustion = 0.015 * cm as f32 * 0.01;
            }
        } else if ladder {
            if d.1 > 0.0 {
                p.stats.add("stat.climbOneCm", (d.1 * 100.0).round() as i32);
            }
        } else if p.on_ground {
            let cm = horizontal();
            if cm > 0 {
                p.stats.add("stat.walkOneCm", cm);
                if p.sprinting {
                    p.stats.add("stat.sprintOneCm", cm);
                    exhaustion = 0.099_999_994 * cm as f32 * 0.01;
                } else {
                    if p.sneaking {
                        p.stats.add("stat.crouchOneCm", cm);
                    }
                    exhaustion = 0.01 * cm as f32 * 0.01;
                }
            }
        } else {
            let cm = horizontal();
            if cm > 25 {
                p.stats.add("stat.flyOneCm", cm);
            }
        }
        if exhaustion != 0.0 {
            self.add_exhaustion(i, exhaustion);
        }
    }

    /// Vanilla `EntityPlayerMP.handleFalling` (`Entity.updateFallState`).
    fn handle_falling(&mut self, i: usize, dy: f64, on_ground: bool) {
        let p = &self.players[i];
        let (x, y, z) = (
            floor_f64(p.pos.0),
            floor_f64(p.pos.1 - f64::from(0.2f32)),
            floor_f64(p.pos.2),
        );
        let mut block = self.level.overworld.block(x, y, z) >> 4;
        if block == 0 {
            let down = self.level.overworld.block(x, y - 1, z) >> 4;
            if super::blocks::is_fence_like(down) {
                block = down;
            }
        }
        let p = &self.players[i];
        let below = (
            x,
            if block == self.level.overworld.block(x, y, z) >> 4 {
                y
            } else {
                y - 1
            },
            z,
        );
        if on_ground && p.fall_distance > 3.0 && block != 0 {
            // EntityLivingBase.updateFallState: block dust where the player lands.
            let extra = krol_java::math::ceil_f32(p.fall_distance - 3.0) as f32;
            let strength = f64::from((0.2 + extra / 15.0).min(10.0)).min(2.5);
            let count = (150.0 * strength) as i32;
            let state = i32::from(self.level.overworld.block(below.0, below.1, below.2));
            let packet = ClientboundPlay::Particle {
                id: 38,
                long_distance: false,
                pos: (p.pos.0 as f32, p.pos.1 as f32, p.pos.2 as f32),
                offset: (0.0, 0.0, 0.0),
                speed: 0.15,
                count,
                args: vec![(state >> 4) | ((state & 15) << 12)],
            };
            self.send_particle(&packet, p.pos);
        }
        let p = &mut self.players[i];
        if on_ground {
            if p.fall_distance > 0.0 {
                let distance = p.fall_distance;
                p.fall_distance = 0.0;
                // Block.onFallenUpon: slime cancels the damage (unless sneaking), hay
                // bales reduce it; farmland trampling is TODO(M5).
                let multiplier = match block {
                    165 if !p.sneaking => 0.0,
                    170 => 0.2,
                    _ => 1.0,
                };
                self.player_fall(i, distance, multiplier);
            }
        } else if dy < 0.0 {
            p.fall_distance = (f64::from(p.fall_distance) - dy) as f32;
        }
    }

    /// Sends a particle packet to every player within 16 blocks (measured from the
    /// player's block position, vanilla `WorldServer.spawnParticle`).
    fn send_particle(&self, packet: &ClientboundPlay, pos: (f64, f64, f64)) {
        for p in &self.players {
            let b = (
                f64::from(floor_f64(p.pos.0)),
                f64::from(floor_f64(p.pos.1)),
                f64::from(floor_f64(p.pos.2)),
            );
            let d = (b.0 - pos.0, b.1 - pos.1, b.2 - pos.2);
            if d.0 * d.0 + d.1 * d.1 + d.2 * d.2 <= 256.0 {
                p.send(packet.clone());
            }
        }
    }

    /// The end of `EntityPlayer.onUpdate`, run with every movement packet: food
    /// (`FoodStats.onUpdate`) and the time statistics.
    fn player_update(&mut self, i: usize) {
        let difficulty = self.config.difficulty;
        let regeneration = self.level.data.game_rule("naturalRegeneration") != Some("false");
        let p = &mut self.players[i];
        let d = &mut p.data;
        let mut exhaustion = d.get_f32("foodExhaustionLevel");
        let mut saturation = d.get_f32("foodSaturationLevel");
        let mut food = d.get_i32("foodLevel");
        let mut timer = d.get_i32("foodTickTimer");
        if exhaustion > 4.0 {
            exhaustion -= 4.0;
            if saturation > 0.0 {
                saturation = (saturation - 1.0).max(0.0);
            } else if difficulty != 0 {
                food = (food - 1).max(0);
            }
        }
        d.insert("foodExhaustionLevel", Tag::Float(exhaustion));
        d.insert("foodSaturationLevel", Tag::Float(saturation));
        d.insert("foodLevel", Tag::Int(food));
        let health = p.health();
        let should_heal = health > 0.0 && health < 20.0;
        let mut starve = false;
        if regeneration && food >= 18 && should_heal {
            timer += 1;
            if timer >= 80 {
                p.set_health(health + 1.0);
                // FoodStats.addExhaustion (not EntityPlayer's: creative players too).
                let level = (p.data.get_f32("foodExhaustionLevel") + 3.0).min(40.0);
                p.data.insert("foodExhaustionLevel", Tag::Float(level));
                timer = 0;
            }
        } else if food <= 0 {
            timer += 1;
            if timer >= 80 {
                starve = health > 10.0 || difficulty == 3 || (health > 1.0 && difficulty == 2);
                timer = 0;
            }
        } else {
            timer = 0;
        }
        p.data.insert("foodTickTimer", Tag::Int(timer));
        if starve {
            // TODO(M5): DamageSource.starve.
        }
        let p = &mut self.players[i];
        p.stats.add("stat.playOneMinute", 1);
        p.stats.add("stat.timeSinceDeath", 1);
    }

    /// The end of `EntityPlayerMP.onUpdateEntity`: health and food go out when they
    /// changed.
    fn send_health_if_changed(&mut self, i: usize) {
        let p = &mut self.players[i];
        let now = (
            p.health(),
            p.data.get_i32("foodLevel"),
            p.data.get_f32("foodSaturationLevel"),
        );
        // The first update after joining or respawning sends both (vanilla starts
        // lastHealth and lastExperience at impossible values).
        let first = p.last_health_sent.is_none();
        if first {
            p.xp_dirty = true;
        }
        if p.last_health_sent != Some((now.0, now.1, now.2 == 0.0)) {
            p.last_health_sent = Some((now.0, now.1, now.2 == 0.0));
            p.send(ClientboundPlay::UpdateHealth {
                health: now.0,
                food: now.1,
                saturation: now.2,
            });
        }
        if std::mem::take(&mut p.xp_dirty) {
            let d = &p.data;
            p.send(ClientboundPlay::SetExperience {
                bar: d.get_f32("XpP"),
                level: d.get_i32("XpLevel"),
                total: d.get_i32("XpTotal"),
            });
        }
    }

    /// Vanilla `EntityPlayer.fall` / `EntityLivingBase.fall`.
    fn player_fall(&mut self, i: usize, distance: f32, multiplier: f32) {
        let p = &mut self.players[i];
        let allow_flying = p.gamemode == 1 || p.gamemode == 3;
        if allow_flying {
            return;
        }
        if distance >= 2.0 {
            p.stats.add(
                "stat.fallOneCm",
                (f64::from(distance) * 100.0).round() as i32,
            );
        }
        // TODO(M5): jump boost reduces the distance.
        let damage = krol_java::math::ceil_f32((distance - 3.0) * multiplier);
        if damage <= 0 {
            return;
        }
        let (pos, conn) = (p.pos, p.conn_id);
        let sound = if damage > 4 {
            "game.player.hurt.fall.big"
        } else {
            "game.player.hurt.fall.small"
        };
        self.play_sound_near_except(Some(conn), sound, pos, 1.0, 1.0);
        self.attack_player(i, super::damage::DamageSource::Fall, damage as f32);
        let p = &self.players[i];
        let (x, y, z) = (
            floor_f64(p.pos.0),
            floor_f64(p.pos.1 - f64::from(0.2f32)),
            floor_f64(p.pos.2),
        );
        let block = self.level.overworld.block(x, y, z) >> 4;
        if block != 0 {
            let s = super::blocks::sound_type(block);
            self.play_sound_near_except(
                Some(conn),
                &s.step_sound(),
                pos,
                s.volume * 0.5,
                s.frequency * 0.75,
            );
        }
    }

    /// Vanilla `NetHandlerPlayServer.processPlayer` for C03 to C06.
    pub(super) fn process_movement(
        &mut self,
        i: usize,
        position: Option<(f64, f64, f64)>,
        rotation: Option<(f32, f32)>,
        on_ground: bool,
    ) {
        let finite = position.is_none_or(|p| p.0.is_finite() && p.1.is_finite() && p.2.is_finite())
            && rotation.is_none_or(|r| r.0.is_finite() && r.1.is_finite());
        if !finite {
            self.kick(i, "Invalid move packet received");
            return;
        }
        let radius = self.config.view_distance;
        let allow_flight = self.config.allow_flight;
        let player = &mut self.players[i];
        let start = player.pos;
        let mut position = position;
        if let Some(p) = position {
            let m = &mut player.movement;
            let d = (p.0 - m.last_pos.0, p.1 - m.last_pos.1, p.2 - m.last_pos.2);
            if !m.has_moved && d.0 * d.0 + d.1 * d.1 + d.2 * d.2 < 0.25 {
                m.has_moved = true;
            }
        }
        if !player.movement.has_moved {
            if player.movement.network_ticks - player.movement.last_move_tick > 20 {
                let (pos, rot) = (player.movement.last_pos, player.rotation);
                Self::set_player_location(player, pos, rot);
            }
            return;
        }
        player.movement.last_move_tick = player.movement.network_ticks;
        // TODO(M5): riding and sleeping players.
        let prev_y = player.pos.1;
        player.movement.last_pos = player.pos;
        let last = player.pos;
        if position.is_some_and(|p| p.1 == -999.0) {
            position = None;
        }
        if let Some(p) = position
            && (p.0.abs() > 3.0e7 || p.2.abs() > 3.0e7)
        {
            self.kick(i, "Illegal position");
            return;
        }
        let target = position.unwrap_or(last);
        let rot = rotation.unwrap_or(self.players[i].rotation);

        // onUpdateEntity, then back to the last accepted position.
        self.living_move(i);
        self.player_update(i);
        self.send_health_if_changed(i);
        let player = &mut self.players[i];
        player.pos = last;
        player.rotation = rot;

        let d = (target.0 - last.0, target.1 - last.1, target.2 - last.2);
        let m = player.motion;
        let motion_sq = m.0 * m.0 + m.1 * m.1 + m.2 * m.2;
        if d.0 * d.0 + d.1 * d.1 + d.2 * d.2 - motion_sq > 100.0 {
            let (dx, dy, dz) = (
                krol_java::fmt::java_double(d.0),
                krol_java::fmt::java_double(d.1),
                krol_java::fmt::java_double(d.2),
            );
            // Vanilla quirk: the same three numbers are printed twice.
            tracing::warn!(
                "{} moved too quickly! {dx},{dy},{dz} ({dx}, {dy}, {dz})",
                player.profile.name
            );
            let rot = player.rotation;
            Self::set_player_location(player, last, rot);
            return;
        }

        let shrink = [0.0625; 3];
        let free_before = physics::colliding_boxes(
            &mut self.level.overworld,
            &Aabb::at(last, 0.6, 1.8).contract(shrink),
        )
        .is_empty();
        if self.players[i].on_ground && !on_ground && d.1 > 0.0 {
            self.jump(i);
        }
        self.move_player(i, d);
        let player = &mut self.players[i];
        player.on_ground = on_ground;
        let rest = (target.0 - player.pos.0, target.2 - player.pos.2);
        // Vanilla quirk: `dy > -0.5 || dy < 0.5` is always true, so the vertical
        // difference never counts (and the floating check below always sees 0).
        let dy = 0.0f64;
        let wrong = rest.0 * rest.0 + dy * dy + rest.1 * rest.1 > 0.0625 && player.gamemode != 1;
        if wrong {
            tracing::warn!("{} moved wrongly!", player.profile.name);
        }
        player.pos = target;
        self.add_movement_stat(
            i,
            (target.0 - start.0, target.1 - start.1, target.2 - start.2),
        );
        // Spectators have no clip.
        if self.players[i].gamemode != 3 {
            let free_after = physics::colliding_boxes(
                &mut self.level.overworld,
                &Aabb::at(target, 0.6, 1.8).contract(shrink),
            )
            .is_empty();
            if free_before && (wrong || !free_after) {
                let player = &mut self.players[i];
                Self::set_player_location(player, last, rot);
                return;
            }
        }

        // Floating (vanilla: checkBlockCollision on the box grown by 1/16 and extended
        // 0.55 down; any non-air block counts).
        let gamemode = self.players[i].gamemode;
        let flying_allowed = allow_flight || gamemode == 1 || gamemode == 3;
        let touching = flying_allowed || {
            let bb = Aabb::at(target, 0.6, 1.8)
                .expand([0.0625; 3])
                .add_coord([0.0, -0.55, 0.0]);
            self.any_block_in(&bb, 1..=u16::MAX)
        };
        let player = &mut self.players[i];
        if touching {
            player.movement.floating_ticks = 0;
        } else if dy >= -0.03125 {
            player.movement.floating_ticks += 1;
            if player.movement.floating_ticks > 80 {
                tracing::warn!("{} was kicked for floating too long!", player.profile.name);
                self.kick(i, "Flying is not enabled on this server");
                return;
            }
        }
        self.players[i].on_ground = on_ground;

        let unload = self.players[i].update_watching(radius);
        for (x, z) in self.players[i].take_new_watches() {
            self.level.overworld.chunk(x, z);
        }
        let sky = self.level.overworld.has_sky;
        for (ux, uz) in unload {
            let (_, data) = self.level.overworld.chunk(ux, uz).packet_data(true, 0, sky);
            self.players[i].send(ClientboundPlay::ChunkData {
                column: ChunkColumn {
                    x: ux,
                    z: uz,
                    mask: 0,
                    data,
                },
                full: true,
            });
        }
        let dy_fall = self.players[i].pos.1 - prev_y;
        self.handle_falling(i, dy_fall, on_ground);
    }

    /// Vanilla `processEntityAction`.
    pub(super) fn entity_action(&mut self, i: usize, action: i32) {
        let p = &mut self.players[i];
        match action {
            0 | 1 => {
                let sneaking = action == 0;
                if p.sneaking != sneaking {
                    p.sneaking = sneaking;
                    p.dirty_metadata.insert(0);
                }
            }
            3 | 4 => {
                let sprinting = action == 3;
                if p.sprinting != sprinting {
                    p.sprinting = sprinting;
                    p.dirty_metadata.insert(0);
                }
                // EntityLivingBase.setSprinting: the speed boost is removed and, when
                // sprinting, applied again (both mark the attribute changed).
                use super::attributes::{MOVEMENT_SPEED, Modifier, SPRINT_AMOUNT, SPRINT_UUID};
                if let Some(k) = p.modifiers.iter().position(|m| m.uuid == SPRINT_UUID) {
                    p.modifiers.remove(k);
                    p.dirty_attributes.insert(MOVEMENT_SPEED);
                }
                if sprinting {
                    p.modifiers.push(Modifier {
                        attribute: MOVEMENT_SPEED,
                        uuid: SPRINT_UUID,
                        name: "Sprinting speed boost".into(),
                        amount: SPRINT_AMOUNT,
                        operation: 2,
                        saved: false,
                    });
                    p.dirty_attributes.insert(MOVEMENT_SPEED);
                }
            }
            // TODO(M5): leave bed, horse jump, horse inventory.
            _ => {}
        }
    }
}
