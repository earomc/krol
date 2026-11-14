//! Experience orbs (vanilla: `EntityXPOrb`, `Block.dropXpOnBlockBreak`,
//! `SlotFurnaceOutput.onCrafting`).
//!
//! TODO(M5): orbs in lava (fizz), pushOutOfBlocks, water flow, damage, saving orbs with
//! their chunk; experience from mobs, breeding, trading, fishing and the player's own
//! death.

use krol_java::math::floor_f64;
use krol_protocol::packets::play::{ClientboundPlay, ItemStack, Metadata, MetadataValue};

use super::Game;
use super::physics::{self, Body};

/// Orbs are 0.5 x 0.5 on the server (`setSize` in the constructor).
const SIZE: f32 = 0.5;
/// Orbs fly to the closest player within this distance.
const ATTRACT_RANGE: f64 = 8.0;
/// Gravity per tick.
const GRAVITY: f32 = 0.03;
/// Vertical speed kept (inverted) when bouncing off the ground.
const BOUNCE: f32 = -0.9;
/// Air drag per tick.
const DRAG: f32 = 0.98;
/// Ticks until an orb despawns.
const LIFETIME: i32 = 6000;
/// Ticks a player waits between two orbs (`xpCooldown`).
pub const PICKUP_COOLDOWN: i32 = 2;
/// Orb values an amount of experience is split into, largest first
/// (`EntityXPOrb.getXPSplit`).
const SPLITS: [i32; 10] = [2477, 1237, 617, 307, 149, 73, 37, 17, 7, 3];

/// `EntityXPOrb.getXPSplit`: the largest orb value not above `amount`.
pub fn xp_split(amount: i32) -> i32 {
    SPLITS.into_iter().find(|&s| amount >= s).unwrap_or(1)
}

/// One experience orb.
#[derive(Debug, Clone)]
pub struct XpOrb {
    /// Physics state.
    pub body: Body,
    /// Experience it gives (`xpValue`).
    pub value: i32,
    /// Ticks lived (`xpOrbAge`).
    age: i32,
    /// `xpColor`: counts ticks; also paces the search for a player.
    color: i32,
    /// `xpTargetColor`: `color` at the last search.
    target_color: i32,
    /// The player it flies to (entity id).
    target: Option<i32>,
    /// Ticks until it can be picked up (`delayBeforeCanPickup`).
    pickup_delay: i32,
    /// Removed from the world (`isDead`).
    pub dead: bool,
    /// The chunk the entity was last added to.
    pub chunk: (i32, i32),
}

impl XpOrb {
    /// Vanilla's metadata for an orb (the base entity values).
    pub fn metadata() -> Vec<Metadata> {
        use MetadataValue::{Byte, Short, String};
        let md = |index, value| Metadata { index, value };
        vec![
            md(0, Byte(0)),
            md(1, Short(300)),
            md(2, String(std::string::String::new())),
            md(3, Byte(0)),
            md(4, Byte(0)),
        ]
    }
}

impl Game {
    /// Spawns one orb (`new EntityXPOrb` + `spawnEntityInWorld`): random yaw and start
    /// motion from `Math.random()`.
    pub(super) fn spawn_orb(&mut self, pos: (f64, f64, f64), value: i32) {
        let entity_id = self.next_entity_id();
        let r = &mut self.world_rand;
        let _yaw = r.next_double();
        let mut horizontal =
            || f64::from((r.next_double() * f64::from(0.2f32) - f64::from(0.1f32)) as f32 * 2.0);
        let mx = horizontal();
        let my = f64::from((r.next_double() * 0.2) as f32 * 2.0);
        let mz = {
            let r = &mut self.world_rand;
            f64::from((r.next_double() * f64::from(0.2f32) - f64::from(0.1f32)) as f32 * 2.0)
        };
        let mut body = Body::player(pos, false);
        body.width = SIZE;
        body.height = SIZE;
        body.step_height = 0.0;
        body.motion = (mx, my, mz);
        self.orbs.insert(
            entity_id,
            XpOrb {
                body,
                value,
                age: 0,
                color: 0,
                target_color: 0,
                target: None,
                pickup_delay: 0,
                dead: false,
                chunk: (floor_f64(pos.0) >> 4, floor_f64(pos.2) >> 4),
            },
        );
        self.track_entity(entity_id);
    }

    /// Splits `amount` into orbs at `pos` (`dropXpOnBlockBreak`, death drops).
    pub(super) fn spawn_orbs(&mut self, pos: (f64, f64, f64), mut amount: i32) {
        while amount > 0 {
            let value = xp_split(amount);
            amount -= value;
            self.spawn_orb(pos, value);
        }
    }

    /// One orb's turn in the entity loop: tick it unless dead, then remove it if dead.
    pub(super) fn update_orb(&mut self, id: i32) {
        if self.orbs.get(&id).is_some_and(|o| !o.dead) {
            self.tick_orb(id);
        }
        if self.orbs.get(&id).is_some_and(|o| o.dead) {
            self.orbs.remove(&id);
            self.untrack_entity(id);
        }
    }

    /// `getClosestPlayerToEntity(orb, 8)`: the nearest non-spectator, first in join order
    /// on ties.
    fn closest_player(&self, pos: (f64, f64, f64)) -> Option<i32> {
        let mut best: Option<(f64, i32)> = None;
        for p in self.players.iter().filter(|p| p.gamemode != 3) {
            let d = (p.pos.0 - pos.0, p.pos.1 - pos.1, p.pos.2 - pos.2);
            let dist = d.0 * d.0 + d.1 * d.1 + d.2 * d.2;
            if dist < ATTRACT_RANGE * ATTRACT_RANGE && best.is_none_or(|(b, _)| dist < b) {
                best = Some((dist, p.entity_id));
            }
        }
        best.map(|(_, id)| id)
    }

    /// `EntityXPOrb.onUpdate`.
    fn tick_orb(&mut self, id: i32) {
        let Some(mut orb) = self.orbs.remove(&id) else {
            return;
        };
        if orb.pickup_delay > 0 {
            orb.pickup_delay -= 1;
        }
        orb.body.motion.1 -= f64::from(GRAVITY);
        // Vanilla quirk: how often the orb looks for a player depends on its entity id.
        if orb.target_color < orb.color - 20 + id % 100 {
            let far = orb.target.is_none_or(|t| {
                self.players
                    .iter()
                    .find(|p| p.entity_id == t)
                    .is_none_or(|p| {
                        let d = (
                            p.pos.0 - orb.body.pos.0,
                            p.pos.1 - orb.body.pos.1,
                            p.pos.2 - orb.body.pos.2,
                        );
                        d.0 * d.0 + d.1 * d.1 + d.2 * d.2 > ATTRACT_RANGE * ATTRACT_RANGE
                    })
            });
            if far {
                orb.target = self.closest_player(orb.body.pos);
            }
            orb.target_color = orb.color;
        }
        let target = orb
            .target
            .and_then(|t| self.players.iter().find(|p| p.entity_id == t))
            .filter(|p| p.gamemode != 3);
        if target.is_none() {
            orb.target = None;
        }
        if let Some(p) = target {
            let eye = if p.sneaking { 1.62f32 - 0.08 } else { 1.62 };
            let dx = (p.pos.0 - orb.body.pos.0) / ATTRACT_RANGE;
            let dy = (p.pos.1 + f64::from(eye) - orb.body.pos.1) / ATTRACT_RANGE;
            let dz = (p.pos.2 - orb.body.pos.2) / ATTRACT_RANGE;
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            let mut pull = 1.0 - dist;
            if pull > 0.0 {
                pull *= pull;
                orb.body.motion.0 += dx / dist * pull * 0.1;
                orb.body.motion.1 += dy / dist * pull * 0.1;
                orb.body.motion.2 += dz / dist * pull * 0.1;
            }
        }
        let m = orb.body.motion;
        physics::move_entity_quiet(&mut self.level.overworld, &mut orb.body, m.0, m.1, m.2);
        // doBlockCollisions, with the orb in the world.
        let bb = orb.body.bb();
        self.orbs.insert(id, orb);
        self.block_collisions(&bb);
        let Some(mut orb) = self.orbs.remove(&id) else {
            return;
        };
        let p = orb.body.pos;
        let mut friction = DRAG;
        if orb.body.on_ground {
            let below =
                self.level
                    .overworld
                    .block(floor_f64(p.0), floor_f64(p.1) - 1, floor_f64(p.2));
            friction = super::blocks::slipperiness(below >> 4) * DRAG;
        }
        orb.body.motion.0 *= f64::from(friction);
        orb.body.motion.1 *= f64::from(DRAG);
        orb.body.motion.2 *= f64::from(friction);
        if orb.body.on_ground {
            orb.body.motion.1 *= f64::from(BOUNCE);
        }
        orb.color += 1;
        orb.age += 1;
        if orb.age >= LIFETIME {
            orb.dead = true;
        }
        orb.chunk = (floor_f64(p.0) >> 4, floor_f64(p.2) >> 4);
        self.orbs.insert(id, orb);
    }

    /// `EntityXPOrb.onCollideWithPlayer`: the player collects the orb unless it is still
    /// fresh or the player collected one just now.
    pub(super) fn collect_orb(&mut self, i: usize, id: i32) {
        let Some(orb) = self.orbs.get(&id) else {
            return;
        };
        if orb.dead || orb.pickup_delay != 0 || self.players[i].xp_cooldown != 0 {
            return;
        }
        let value = orb.value;
        self.players[i].xp_cooldown = PICKUP_COOLDOWN;
        let r = &mut self.world_rand;
        let pitch = 0.5 * ((r.next_float() - r.next_float()) * 0.7 + 1.8);
        let pos = self.players[i].pos;
        self.play_sound_near_except(None, "random.orb", pos, 0.1, pitch);
        let collector = self.players[i].entity_id;
        self.send_to_tracking(
            id,
            &ClientboundPlay::CollectItem {
                collected: id,
                collector,
            },
        );
        self.players[i].add_experience(value);
        if let Some(orb) = self.orbs.get_mut(&id) {
            orb.dead = true;
        }
    }

    /// `Block.dropXpOnBlockBreak` after a survival harvest of an ore that dropped its
    /// item rather than itself.
    pub(super) fn drop_block_xp(&mut self, pos: (i32, i32, i32), block: u16) {
        use super::ids::block as b;
        use krol_java::math::random_i32_in_range as range;
        let r = &mut self.world_rand;
        let amount = match block {
            b::COAL_ORE => range(r, 0, 2),
            b::DIAMOND_ORE | b::EMERALD_ORE => range(r, 3, 7),
            b::LAPIS_ORE | b::QUARTZ_ORE => range(r, 2, 5),
            b::REDSTONE_ORE | b::LIT_REDSTONE_ORE => 1 + r.next_int_bounded(5),
            _ => return,
        };
        if self.level.data.game_rule("doTileDrops") == Some("false") {
            return;
        }
        let at = (
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 0.5,
            f64::from(pos.2) + 0.5,
        );
        self.spawn_orbs(at, amount);
    }

    /// `SlotFurnaceOutput.onCrafting`: the smelting experience for `taken` items, with
    /// the fraction rounded up at random (`Math.random`), as orbs at the player.
    pub(super) fn furnace_xp(&mut self, i: usize, output: &ItemStack, taken: i32) {
        let xp = super::recipes::smelting_xp(output);
        let mut amount = taken;
        if xp == 0.0 {
            amount = 0;
        } else if xp < 1.0 {
            let exact = taken as f32 * xp;
            let mut whole = krol_java::math::floor_f32(exact);
            if whole < krol_java::math::ceil_f32(exact)
                && self.world_rand.next_double() < f64::from(exact - whole as f32)
            {
                whole += 1;
            }
            amount = whole;
        }
        let p = self.players[i].pos;
        let at = (p.0, p.1 + 0.5, p.2 + 0.5);
        self.spawn_orbs(at, amount);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_vanilla() {
        assert_eq!(xp_split(1), 1);
        assert_eq!(xp_split(2), 1);
        assert_eq!(xp_split(3), 3);
        assert_eq!(xp_split(16), 7);
        assert_eq!(xp_split(100), 73);
        assert_eq!(xp_split(5000), 2477);
    }
}
