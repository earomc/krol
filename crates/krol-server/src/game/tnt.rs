//! TNT (vanilla `BlockTNT`, `EntityTNTPrimed`): priming by redstone, flint and steel,
//! fire, explosions and breaking primed-marked TNT, and the primed entity that explodes
//! after its fuse.
//!
//! TODO(M5): dispensers placing primed TNT, burning arrows, fire charges, TNT minecarts,
//! saving primed TNT with its chunk.

use krol_java::math::floor_f64;
use krol_protocol::packets::play::{Metadata, MetadataValue};

use super::Game;
use super::physics::{self, Body};

/// `tnt`; metadata bit 1 is `explode` (breaking it primes it).
pub const TNT: u16 = 46;
const EXPLODE_BIT: u16 = 1;
/// Entity size (`setSize(0.98F, 0.98F)`).
const SIZE: f32 = 0.98;
/// The fuse of TNT primed by a player, redstone or fire (`fuse = 80`).
pub const FUSE: i32 = 80;
/// Upward speed when primed (`motionY = 0.2F`).
const HOP: f32 = 0.2;
/// Horizontal speed when primed, in a random direction (`0.02F`).
const NUDGE: f32 = 0.02;
const GRAVITY: f32 = 0.04;
const DRAG: f32 = 0.98;
const LANDING_FRICTION: f32 = 0.7;
const BOUNCE: f64 = -0.5;
/// Explosion strength of TNT.
const POWER: f32 = 4.0;
/// `game.tnt.primed` is played at volume and pitch 1.
const PRIMED_SOUND: &str = "game.tnt.primed";

/// One primed TNT entity.
#[derive(Debug, Clone)]
pub struct PrimedTnt {
    /// Physics state.
    pub body: Body,
    /// Ticks until it explodes (`fuse`).
    pub fuse: i32,
    /// The player who lit it, by entity id (`tntPlacedBy`).
    pub placed_by: Option<i32>,
    /// Removed from the world (`isDead`).
    pub dead: bool,
    /// The chunk the entity was last added to.
    pub chunk: (i32, i32),
}

impl PrimedTnt {
    /// Vanilla's metadata for primed TNT (the base entity values).
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
    /// `new EntityTNTPrimed(world, x + 0.5F, y, z + 0.5F, placer)` + `spawnEntityInWorld`:
    /// the entity hops up in a direction from `Math.random()`. `fuse` replaces the
    /// default fuse (TNT set off by an explosion). Returns the entity id.
    pub(super) fn spawn_primed_tnt(
        &mut self,
        pos: (i32, i32, i32),
        placed_by: Option<i32>,
        fuse: impl FnOnce(&mut krol_java::JavaRandom) -> i32,
    ) -> i32 {
        let at = (
            f64::from(pos.0 as f32 + 0.5),
            f64::from(pos.1),
            f64::from(pos.2 as f32 + 0.5),
        );
        let angle = (self.math_random.next_double() * f64::from(std::f32::consts::PI) * 2.0) as f32;
        let mut body = Body::player(at, false);
        body.width = SIZE;
        body.height = SIZE;
        body.step_height = 0.0;
        body.motion = (
            f64::from(-(f64::from(angle).sin() as f32) * NUDGE),
            f64::from(HOP),
            f64::from(-(f64::from(angle).cos() as f32) * NUDGE),
        );
        let fuse = fuse(&mut self.world_rand);
        let entity_id = self.next_entity_id();
        self.tnt.insert(
            entity_id,
            PrimedTnt {
                body,
                fuse,
                placed_by,
                dead: false,
                chunk: (floor_f64(at.0) >> 4, floor_f64(at.2) >> 4),
            },
        );
        self.track_entity(entity_id);
        entity_id
    }

    /// `BlockTNT.explode` with the `explode` property set: primes TNT at `pos` (whose
    /// block the caller removes) with the default fuse and the primed sound.
    pub(super) fn prime_tnt(&mut self, pos: (i32, i32, i32), placed_by: Option<i32>) {
        let id = self.spawn_primed_tnt(pos, placed_by, |_| FUSE);
        if let Some(t) = self.tnt.get(&id) {
            let at = t.body.pos;
            self.play_sound_near_except(None, PRIMED_SOUND, at, 1.0, 1.0);
        }
    }

    /// `BlockTNT.onBlockAdded` / `onNeighborBlockChange`: powered TNT primes itself.
    pub(super) fn tnt_check_power(&mut self, pos: (i32, i32, i32)) {
        if self.is_block_powered(pos) {
            self.prime_tnt(pos, None);
            self.set_block_notify(pos, 0);
        }
    }

    /// `BlockTNT.onBlockDestroyedByPlayer` (also used by fire): TNT marked to explode
    /// primes when it goes.
    pub(super) fn tnt_destroyed(&mut self, pos: (i32, i32, i32), state: u16) {
        if state >> 4 == TNT && state & EXPLODE_BIT != 0 {
            self.prime_tnt(pos, None);
        }
    }

    /// `BlockTNT.onBlockActivated` with flint and steel: primes the TNT (lit by the
    /// player), removes the block and wears the flint and steel. Returns whether the
    /// click was used.
    pub(super) fn light_tnt(&mut self, i: usize, pos: (i32, i32, i32)) -> bool {
        let held = self.players[i].held().map(|s| s.id);
        if held != Some(super::ids::item::FLINT_AND_STEEL) {
            // TODO(M5): fire charges.
            return false;
        }
        let igniter = self.players[i].entity_id;
        self.prime_tnt(pos, Some(igniter));
        self.set_block_notify(pos, 0);
        self.damage_held_item(i, 1);
        let p = &mut self.players[i];
        let slot = 36 + p.selected;
        if p.inventory[slot].as_ref().is_some_and(|s| s.count == 0) {
            p.inventory[slot] = None;
        }
        true
    }

    /// One primed TNT's turn in the entity loop.
    pub(super) fn update_tnt(&mut self, id: i32) {
        if self.tnt.get(&id).is_some_and(|t| !t.dead) {
            self.tick_tnt(id);
        }
        if self.tnt.get(&id).is_some_and(|t| t.dead) {
            self.tnt.remove(&id);
            self.untrack_entity(id);
        }
    }

    /// `EntityTNTPrimed.onUpdate`.
    fn tick_tnt(&mut self, id: i32) {
        let Some(mut t) = self.tnt.remove(&id) else {
            return;
        };
        t.body.motion.1 -= f64::from(GRAVITY);
        let m = t.body.motion;
        physics::move_entity_quiet(&mut self.level.overworld, &mut t.body, m.0, m.1, m.2);
        let bb = t.body.bb();
        self.tnt.insert(id, t);
        let touched = self.block_collisions(&bb);
        let Some(mut t) = self.tnt.remove(&id) else {
            return;
        };
        t.body.touched(touched);
        t.body.motion.0 *= f64::from(DRAG);
        t.body.motion.1 *= f64::from(DRAG);
        t.body.motion.2 *= f64::from(DRAG);
        if t.body.on_ground {
            t.body.motion.0 *= f64::from(LANDING_FRICTION);
            t.body.motion.2 *= f64::from(LANDING_FRICTION);
            t.body.motion.1 *= BOUNCE;
        }
        let p = t.body.pos;
        t.chunk = (floor_f64(p.0) >> 4, floor_f64(p.2) >> 4);
        let fuse = t.fuse;
        t.fuse -= 1;
        if fuse <= 0 {
            t.dead = true;
            let at = (p.0, p.1 + f64::from(SIZE / 16.0), p.2);
            self.tnt.insert(id, t);
            self.explode(Some(id), at, POWER);
            return;
        }
        // handleWaterMovement. Vanilla quirk: primed TNT never runs onEntityUpdate, so it
        // stays on its "first update" and never splashes.
        let bb = t
            .body
            .bb()
            .expand([0.0, f64::from(-0.4f32), 0.0])
            .contract([0.001, 0.001, 0.001]);
        let (water, push) = super::liquids::water_movement(&mut self.level.overworld, &bb, true);
        t.body.in_water = water;
        t.body.motion.0 += push.0;
        t.body.motion.1 += push.1;
        t.body.motion.2 += push.2;
        self.tnt.insert(id, t);
    }

    /// `BlockTNT.onBlockDestroyedByExplosion`: TNT caught in an explosion primes with a
    /// short random fuse.
    pub(super) fn tnt_exploded(&mut self, pos: (i32, i32, i32), placed_by: Option<i32>) {
        self.spawn_primed_tnt(pos, placed_by, |r| r.next_int_bounded(FUSE / 4) + FUSE / 8);
    }
}
