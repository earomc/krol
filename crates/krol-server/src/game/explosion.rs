//! Explosions (vanilla `Explosion`, `World.getBlockDensity`, `WorldServer.newExplosion`).
//!
//! 1352 rays from the center (one world-random float each) lose strength through the
//! blocks' explosion resistance; every position a ray reaches with strength left joins
//! the affected set (air included), which a Java `HashSet<BlockPos>` orders. Entities
//! within twice the strength are hurt and pushed by how exposed they are. The server
//! never spawns the particles (and so draws nothing for them).
//!
//! TODO(M5): flaming explosions (ghast fireballs, beds in the nether); mobs; the order of
//! a treeified `HashSet` bucket (more than 8 positions in one bucket).

use krol_protocol::packets::play::ClientboundPlay;

use super::Game;
use super::block_data::{Material, material};
use super::damage::DamageSource;
use super::physics::Aabb;

/// Rays per side of the cube they start on (16 x 16 x 16, shell only).
const RAYS: i32 = 16;
/// Ray strength factor range: `0.7 + 0.6 * random`.
const RAY_MIN: f32 = 0.7;
const RAY_SPREAD: f32 = 0.6;
/// Rays advance this far per step...
const RAY_STEP: f32 = 0.3;
/// ...losing this much strength per step (`0.22500001F`)...
const RAY_DECAY: f32 = 0.225_000_01;
/// ...and `(resistance + 0.3) * 0.3` in a block.
const RESISTANCE_OFFSET: f32 = 0.3;
/// Players this close (squared) see the explosion packet.
const PACKET_RANGE_SQ: f64 = 4096.0;
/// `random.explode` plays at volume 4.
const SOUND_VOLUME: f32 = 4.0;

/// An entity an explosion or a falling anvil may hit.
#[derive(Debug, Clone, Copy)]
pub(super) enum Target {
    Player(usize),
    Item(i32),
    Orb(i32),
    Falling(i32),
    Tnt(i32),
    Thrown(i32),
    Arrow(i32),
}

impl Game {
    /// `World.createExplosion(exploder, x, y, z, strength, true)` on the server:
    /// destroys blocks (smoking), not flaming.
    pub(super) fn explode(&mut self, exploder: Option<i32>, pos: (f64, f64, f64), strength: f32) {
        let placed_by = exploder
            .and_then(|id| self.tnt.get(&id))
            .and_then(|t| t.placed_by);
        let affected = self.explosion_rays(pos, strength);
        let knockback = self.explosion_entities(exploder, placed_by, pos, strength);
        // doExplosionB(false).
        let r = &mut self.world_rand;
        let pitch = (1.0 + (r.next_float() - r.next_float()) * 0.2) * 0.7;
        self.play_sound_near_except(None, "random.explode", pos, SOUND_VOLUME, pitch);
        for &p in &affected {
            let state = self.level.overworld.block(p.0, p.1, p.2);
            let id = state >> 4;
            if material(id) == Material::Air {
                continue;
            }
            // canDropFromExplosion: everything but TNT.
            if id != super::tnt::TNT {
                self.drop_block_items_with_chance(p, state, 1.0 / strength, 0);
            }
            self.drop_container_contents(p);
            self.set_block_notify(p, 0);
            // onBlockDestroyedByExplosion.
            if id == super::tnt::TNT {
                self.tnt_exploded(p, placed_by);
            }
        }
        // WorldServer.newExplosion: the packet to every player within 64 blocks.
        let base = (pos.0 as i32, pos.1 as i32, pos.2 as i32);
        let records: Vec<(i8, i8, i8)> = affected
            .iter()
            .map(|p| {
                (
                    (p.0 - base.0) as i8,
                    (p.1 - base.1) as i8,
                    (p.2 - base.2) as i8,
                )
            })
            .collect();
        for p in &self.players {
            let d = (p.pos.0 - pos.0, p.pos.1 - pos.1, p.pos.2 - pos.2);
            if d.0 * d.0 + d.1 * d.1 + d.2 * d.2 >= PACKET_RANGE_SQ {
                continue;
            }
            let motion = knockback
                .iter()
                .find(|(id, _)| *id == p.entity_id)
                .map_or((0.0, 0.0, 0.0), |(_, m)| {
                    (m.0 as f32, m.1 as f32, m.2 as f32)
                });
            p.send(ClientboundPlay::Explosion {
                pos: (pos.0 as f32, pos.1 as f32, pos.2 as f32),
                radius: strength,
                records: records.clone(),
                motion,
            });
        }
    }

    /// `doExplosionA`, blocks: the affected positions in `HashSet` order.
    fn explosion_rays(&mut self, pos: (f64, f64, f64), strength: f32) -> Vec<(i32, i32, i32)> {
        let mut found: Vec<(i32, i32, i32)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let edge = RAYS - 1;
        for i in 0..RAYS {
            for j in 0..RAYS {
                for k in 0..RAYS {
                    if !(i == 0 || i == edge || j == 0 || j == edge || k == 0 || k == edge) {
                        continue;
                    }
                    let axis = |v: i32| f64::from(v as f32 / edge as f32 * 2.0 - 1.0);
                    let (mut dx, mut dy, mut dz) = (axis(i), axis(j), axis(k));
                    let len = (dx * dx + dy * dy + dz * dz).sqrt();
                    dx /= len;
                    dy /= len;
                    dz /= len;
                    let mut left = strength * (RAY_MIN + self.world_rand.next_float() * RAY_SPREAD);
                    let (mut x, mut y, mut z) = pos;
                    let step = f64::from(RAY_STEP);
                    while left > 0.0 {
                        let p = (
                            krol_java::math::floor_f64(x),
                            krol_java::math::floor_f64(y),
                            krol_java::math::floor_f64(z),
                        );
                        let id = self.level.overworld.block(p.0, p.1, p.2) >> 4;
                        if material(id) != Material::Air {
                            let resistance = super::light_data::explosion_resistance(id);
                            left -= (resistance + RESISTANCE_OFFSET) * RAY_STEP;
                        }
                        if left > 0.0 && seen.insert(p) {
                            found.push(p);
                        }
                        x += dx * step;
                        y += dy * step;
                        z += dz * step;
                        left -= RAY_DECAY;
                    }
                }
            }
        }
        let hashes: Vec<i32> = found
            .iter()
            .map(|p| super::world::block_pos_hash(p.0, p.1, p.2))
            .collect();
        krol_java::java_hash_order(&hashes)
            .into_iter()
            .map(|k| found[k])
            .collect()
    }

    /// `doExplosionA`, entities: hurts and pushes everything in range; returns the
    /// knockback of the players who can be hurt (`playerKnockbackMap`).
    fn explosion_entities(
        &mut self,
        exploder: Option<i32>,
        placed_by: Option<i32>,
        pos: (f64, f64, f64),
        strength: f32,
    ) -> Vec<(i32, (f64, f64, f64))> {
        let reach = strength * 2.0;
        let floor = |v: f64| f64::from(krol_java::math::floor_f64(v));
        let r = f64::from(reach) + 1.0;
        let area = Aabb::new(
            [floor(pos.0 - r), floor(pos.1 - r), floor(pos.2 - r)],
            [floor(pos.0 + r), floor(pos.1 + r), floor(pos.2 + r)],
        );
        let targets = self.entities_within(&area, exploder);
        let source = placed_by.map_or(DamageSource::Explosion, DamageSource::ExplosionBy);
        let mut knockback = Vec::new();
        for (entity_id, target) in targets {
            let Some((feet, eye, bb)) = self.target_shape(target) else {
                continue;
            };
            let dist = f64::from(krol_java::math::sqrt_f64(
                (feet.0 - pos.0).powi(2) + (feet.1 - pos.1).powi(2) + (feet.2 - pos.2).powi(2),
            )) / f64::from(reach);
            if dist > 1.0 {
                continue;
            }
            let mut d = (
                feet.0 - pos.0,
                feet.1 + f64::from(eye) - pos.1,
                feet.2 - pos.2,
            );
            let len = f64::from(krol_java::math::sqrt_f64(d.0 * d.0 + d.1 * d.1 + d.2 * d.2));
            if len == 0.0 {
                continue;
            }
            d = (d.0 / len, d.1 / len, d.2 / len);
            let density = f64::from(self.block_density(pos, &bb));
            let impact = (1.0 - dist) * density;
            let damage =
                ((impact * impact + impact) / 2.0 * 8.0 * f64::from(reach) + 1.0) as i32 as f32;
            let push = self.hit_by_explosion(target, source, damage, impact);
            let add = |m: &mut (f64, f64, f64)| {
                m.0 += d.0 * push;
                m.1 += d.1 * push;
                m.2 += d.2 * push;
            };
            match target {
                Target::Player(i) => {
                    let p = &mut self.players[i];
                    add(&mut p.motion);
                    if !matches!(p.gamemode, CREATIVE | SPECTATOR) {
                        knockback.push((entity_id, (d.0 * impact, d.1 * impact, d.2 * impact)));
                    }
                }
                Target::Item(id) => {
                    if let Some(it) = self.items.get_mut(&id) {
                        add(&mut it.body.motion);
                    }
                }
                Target::Orb(id) => {
                    if let Some(o) = self.orbs.get_mut(&id) {
                        add(&mut o.body.motion);
                    }
                }
                Target::Falling(id) => {
                    if let Some(f) = self.falling.get_mut(&id) {
                        add(&mut f.body.motion);
                    }
                }
                Target::Tnt(id) => {
                    if let Some(t) = self.tnt.get_mut(&id) {
                        add(&mut t.body.motion);
                    }
                }
                Target::Thrown(id) => {
                    if let Some(t) = self.thrown.get_mut(&id) {
                        add(&mut t.body.motion);
                    }
                }
                Target::Arrow(id) => {
                    if let Some(a) = self.arrows.get_mut(&id) {
                        add(&mut a.body.motion);
                    }
                }
            }
        }
        knockback
    }

    /// `World.getEntitiesWithinAABBExcludingEntity(excluded, area)`: every entity but
    /// spectators whose box intersects `area`, in chunk list order.
    pub(super) fn entities_within(&self, area: &Aabb, excluded: Option<i32>) -> Vec<(i32, Target)> {
        let mut targets: Vec<(i32, Target)> = Vec::new();
        for (i, p) in self.players.iter().enumerate() {
            if p.gamemode != SPECTATOR && Aabb::at(p.pos, 0.6, 1.8).intersects(area) {
                targets.push((p.entity_id, Target::Player(i)));
            }
        }
        let mut add = |id: i32, bb: Aabb, t: Target| {
            if Some(id) != excluded && bb.intersects(area) {
                targets.push((id, t));
            }
        };
        for (&id, it) in &self.items {
            add(id, it.body.bb(), Target::Item(id));
        }
        for (&id, o) in &self.orbs {
            add(id, o.body.bb(), Target::Orb(id));
        }
        for (&id, f) in &self.falling {
            add(id, f.body.bb(), Target::Falling(id));
        }
        for (&id, t) in &self.tnt {
            if !t.dead {
                add(id, t.body.bb(), Target::Tnt(id));
            }
        }
        for (&id, t) in &self.thrown {
            if !t.dead {
                add(id, t.body.bb(), Target::Thrown(id));
            }
        }
        for (&id, a) in &self.arrows {
            if !a.dead {
                add(id, a.body.bb(), Target::Arrow(id));
            }
        }
        // Chunk entity lists keep insertion order, which follows entity ids here.
        targets.sort_by_key(|(id, _)| *id);
        targets
    }

    /// `attackEntityFrom(source, damage)` on any entity.
    pub(super) fn attack_target(&mut self, target: Target, source: DamageSource, damage: f32) {
        match target {
            Target::Player(i) => {
                self.attack_player(i, source, damage);
            }
            Target::Item(id) => {
                if let Some(it) = self.items.get_mut(&id) {
                    // Nether stars survive explosions.
                    if !(source.explosion() && it.stack.id == NETHER_STAR) {
                        it.body.velocity_changed = true;
                        it.health = (it.health as f32 - damage) as i32;
                        if it.health <= 0 {
                            it.dead = true;
                        }
                    }
                }
            }
            Target::Orb(id) => {
                if let Some(o) = self.orbs.get_mut(&id) {
                    o.body.velocity_changed = true;
                    o.health = (o.health as f32 - damage) as i32;
                    if o.health <= 0 {
                        o.dead = true;
                    }
                }
            }
            // Entity.attackEntityFrom: only setBeenAttacked.
            Target::Falling(id) => {
                if let Some(f) = self.falling.get_mut(&id) {
                    f.body.velocity_changed = true;
                }
            }
            Target::Tnt(id) => {
                if let Some(t) = self.tnt.get_mut(&id) {
                    t.body.velocity_changed = true;
                }
            }
            Target::Thrown(id) => {
                if let Some(t) = self.thrown.get_mut(&id) {
                    t.body.velocity_changed = true;
                }
            }
            Target::Arrow(id) => {
                if let Some(a) = self.arrows.get_mut(&id) {
                    a.body.velocity_changed = true;
                }
            }
        }
    }

    /// Feet position, eye height (`getEyeHeight`) and box of a target.
    fn target_shape(&self, target: Target) -> Option<((f64, f64, f64), f32, Aabb)> {
        let body = match target {
            Target::Player(i) => {
                let p = &self.players[i];
                return Some((p.pos, p.eye_height(), Aabb::at(p.pos, 0.6, 1.8)));
            }
            Target::Item(id) => &self.items.get(&id)?.body,
            Target::Orb(id) => &self.orbs.get(&id)?.body,
            Target::Falling(id) => &self.falling.get(&id)?.body,
            Target::Thrown(id) => &self.thrown.get(&id)?.body,
            Target::Arrow(id) => {
                let a = self.arrows.get(&id)?;
                // EntityArrow.getEyeHeight.
                return Some((a.body.pos, 0.0, a.body.bb()));
            }
            Target::Tnt(id) => {
                let t = self.tnt.get(&id)?;
                // EntityTNTPrimed.getEyeHeight.
                return Some((t.body.pos, 0.0, t.body.bb()));
            }
        };
        // Entity.getEyeHeight: 85% of the height.
        Some((body.pos, body.height * 0.85, body.bb()))
    }

    /// `attackEntityFrom` for the target, then the push it gets
    /// (`EnchantmentProtection.func_92092_a`: blast protection weakens it).
    fn hit_by_explosion(
        &mut self,
        target: Target,
        source: DamageSource,
        damage: f32,
        impact: f64,
    ) -> f64 {
        self.attack_target(target, source, damage);
        match target {
            Target::Player(i) => {
                let level = super::item_data::armor_enchantment(
                    &self.players[i].inventory,
                    super::item_data::BLAST_PROTECTION,
                );
                if level > 0 {
                    impact
                        - f64::from(krol_java::math::floor_f64(
                            impact * f64::from(level as f32 * BLAST_KNOCKBACK_REDUCTION),
                        ))
                } else {
                    impact
                }
            }
            _ => impact,
        }
    }

    /// `World.getBlockDensity`: the share of rays from points spread over `bb` to `pos`
    /// that no block stops.
    fn block_density(&mut self, pos: (f64, f64, f64), bb: &Aabb) -> f32 {
        let size = [
            bb.max[0] - bb.min[0],
            bb.max[1] - bb.min[1],
            bb.max[2] - bb.min[2],
        ];
        let step = size.map(|s| 1.0 / (s * 2.0 + 1.0));
        let off_x = (1.0 - (1.0 / step[0]).floor() * step[0]) / 2.0;
        let off_z = (1.0 - (1.0 / step[2]).floor() * step[2]) / 2.0;
        if step.iter().any(|&s| s < 0.0) {
            return 0.0;
        }
        let (mut clear, mut total) = (0, 0);
        let mut fx = 0.0f32;
        while fx <= 1.0 {
            let mut fy = 0.0f32;
            while fy <= 1.0 {
                let mut fz = 0.0f32;
                while fz <= 1.0 {
                    let x = bb.min[0] + size[0] * f64::from(fx);
                    let y = bb.min[1] + size[1] * f64::from(fy);
                    let z = bb.min[2] + size[2] * f64::from(fz);
                    let hit = super::raytrace::ray_trace_blocks(
                        &mut self.level.overworld,
                        (x + off_x, y, z + off_z),
                        pos,
                        false,
                        false,
                    );
                    if hit.is_none() {
                        clear += 1;
                    }
                    total += 1;
                    fz = (f64::from(fz) + step[2]) as f32;
                }
                fy = (f64::from(fy) + step[1]) as f32;
            }
            fx = (f64::from(fx) + step[0]) as f32;
        }
        clear as f32 / total as f32
    }
}

/// Game modes.
const CREATIVE: i32 = 1;
const SPECTATOR: i32 = 3;
/// `nether_star`.
const NETHER_STAR: i16 = 399;
/// Blast protection removes 15% of the push per level.
const BLAST_KNOCKBACK_REDUCTION: f32 = 0.15;
