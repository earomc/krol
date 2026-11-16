//! Falling sand, gravel and anvils (vanilla: `BlockFalling`, `BlockAnvil`,
//! `EntityFallingBlock`).
//!
//! TODO(M5): dragon eggs; falling blocks on pistons; tile entity data of falling
//! blocks; saving them with their chunk.

use krol_java::math::floor_f64;
use krol_protocol::packets::play::{ItemStack, Metadata, MetadataValue};

use super::Game;
use super::block_data::{self, Material};
use super::damage::DamageSource;
use super::ids::block;
use super::physics::{self, Body};

/// Ticks between a sand or gravel block being disturbed and falling (`tickRate`).
pub const FALL_DELAY: i32 = 2;
/// Falling blocks only become entities when everything this far around is loaded;
/// otherwise the block drops straight down (`checkFallable`).
const LOADED_RADIUS: i32 = 32;
/// Entity size (`setSize(0.98F, 0.98F)`).
const SIZE: f32 = 0.98;
/// Gravity and drag per tick.
const GRAVITY: f32 = 0.04;
const DRAG: f32 = 0.98;
/// Horizontal speed kept when landing.
const LANDING_FRICTION: f32 = 0.7;
/// A falling block gives up (drops as an item) after this many ticks outside the
/// world's height, or after [`MAX_FALL_TIME`] ticks anywhere.
const OUT_OF_WORLD_TIME: i32 = 100;
const MAX_FALL_TIME: i32 = 600;
/// Pickup delay of the dropped item (`setDefaultPickupDelay`).
const DROP_PICKUP_DELAY: i32 = 10;
/// Falling blocks that hurt entities deal this much per block fallen beyond the first
/// (`fallHurtAmount`)...
const FALL_HURT_AMOUNT: f32 = 2.0;
/// ...up to this much (`fallHurtMax`).
const FALL_HURT_MAX: i32 = 40;
/// A landing anvil is damaged with chance `0.05F` plus this much per block fallen
/// beyond the first.
const ANVIL_DAMAGE_CHANCE_BASE: f32 = 0.05;
const ANVIL_DAMAGE_CHANCE_PER_BLOCK: f64 = 0.05;
/// The anvil's damage bits in its metadata (`BlockAnvil.DAMAGE`, values 0-2).
const ANVIL_DAMAGE_SHIFT: u16 = 2;
const ANVIL_DAMAGE_MASK: u16 = 0b1100;
const ANVIL_MAX_DAMAGE: u16 = 2;
/// The default anvil faces north (horizontal index 2).
const ANVIL_DEFAULT_META: u16 = 2;
/// `playAuxSFX` id of an anvil landing (`BlockAnvil.onEndFalling`).
const EFFECT_ANVIL_LAND: i32 = 1022;

/// Blocks that fall (subclasses of `BlockFalling` krol models).
pub fn falls(id: u16) -> bool {
    id == block::SAND || id == block::GRAVEL || id == block::ANVIL
}

/// The default state of a falling block (`getDefaultState`).
fn default_state(id: u16) -> u16 {
    if id == block::ANVIL {
        id << 4 | ANVIL_DEFAULT_META
    } else {
        id << 4
    }
}

/// `damageDropped` of a falling block: sand keeps its variant, anvils their damage,
/// gravel drops plain gravel.
fn damage_dropped(state: u16) -> i16 {
    match state >> 4 {
        block::SAND => (state & 15) as i16,
        block::ANVIL => ((state & ANVIL_DAMAGE_MASK) >> ANVIL_DAMAGE_SHIFT) as i16,
        _ => 0,
    }
}

/// `BlockFalling.canFallInto`: fire, air, water or lava.
pub fn can_fall_into(state: u16) -> bool {
    let id = state >> 4;
    id == block::FIRE
        || matches!(
            block_data::material(id),
            Material::Air | Material::Water | Material::Lava
        )
}

/// One falling block.
#[derive(Debug, Clone)]
pub struct FallingBlock {
    /// Physics state.
    pub body: Body,
    /// The block state it carries (`fallTile`).
    pub state: u16,
    /// Ticks fallen (`fallTime`).
    fall_time: i32,
    /// Distance fallen since it last touched the ground (`fallDistance`).
    fall_distance: f32,
    /// Hurts entities it lands on (`hurtEntities`, set for anvils).
    hurt_entities: bool,
    /// Broke apart on landing and leaves nothing behind (`canSetAsBlock`, an anvil
    /// damaged beyond its last stage).
    shattered: bool,
    /// Removed from the world (`isDead`).
    pub dead: bool,
    /// The chunk the entity was last added to.
    pub chunk: (i32, i32),
}

impl FallingBlock {
    /// Vanilla's metadata for a falling block (the base entity values).
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

    /// The spawn packet's data: the block state id (`Block.getStateId`).
    pub fn state_id(&self) -> i32 {
        i32::from(self.state >> 4) | i32::from(self.state & 15) << 12
    }
}

impl Game {
    /// `BlockFalling.checkFallable`, the scheduled tick of sand and gravel.
    pub(super) fn check_fallable(&mut self, pos: (i32, i32, i32), state: u16) {
        let w = &mut self.level.overworld;
        if !can_fall_into(w.block(pos.0, pos.1 - 1, pos.2)) || pos.1 < 0 {
            return;
        }
        let loaded = (((pos.0 - LOADED_RADIUS) >> 4)..=((pos.0 + LOADED_RADIUS) >> 4)).all(|cx| {
            (((pos.2 - LOADED_RADIUS) >> 4)..=((pos.2 + LOADED_RADIUS) >> 4))
                .all(|cz| w.is_chunk_loaded(cx, cz))
        });
        if !loaded {
            // Drop straight to the ground without an entity.
            self.set_block_notify(pos, 0);
            let mut below = (pos.0, pos.1 - 1, pos.2);
            while below.1 > 0
                && can_fall_into(self.level.overworld.block(below.0, below.1, below.2))
            {
                below.1 -= 1;
            }
            if below.1 > 0 {
                // Vanilla quirk: the default state lands (red sand becomes sand, anvils
                // lose their damage and face north).
                self.set_block_notify((below.0, below.1 + 1, below.2), default_state(state >> 4));
            }
            return;
        }
        let entity_id = self.next_entity_id();
        let at = (
            f64::from(pos.0) + 0.5,
            f64::from(pos.1),
            f64::from(pos.2) + 0.5,
        );
        let mut body = Body::player(at, false);
        body.width = SIZE;
        body.height = SIZE;
        body.step_height = 0.0;
        body.motion = (0.0, 0.0, 0.0);
        self.falling.insert(
            entity_id,
            FallingBlock {
                body,
                state,
                fall_time: 0,
                fall_distance: 0.0,
                // BlockAnvil.onStartFalling.
                hurt_entities: state >> 4 == block::ANVIL,
                shattered: false,
                dead: false,
                chunk: (floor_f64(at.0) >> 4, floor_f64(at.2) >> 4),
            },
        );
        self.track_entity(entity_id);
    }

    /// One falling block's turn in the entity loop.
    pub(super) fn update_falling(&mut self, id: i32) {
        if self.falling.get(&id).is_some_and(|f| !f.dead) {
            self.tick_falling(id);
        }
        if self.falling.get(&id).is_some_and(|f| f.dead) {
            self.falling.remove(&id);
            self.untrack_entity(id);
        }
    }

    /// The dropped item of a block that could not land (`entityDropItem(stack, 0)`).
    fn drop_fallen(&mut self, pos: (f64, f64, f64), state: u16) {
        if self.level.data.game_rule("doEntityDrops") == Some("false") {
            return;
        }
        let stack = ItemStack {
            id: (state >> 4) as i16,
            count: 1,
            damage: damage_dropped(state),
            nbt: None,
        };
        self.spawn_item(pos, stack, DROP_PICKUP_DELAY);
    }

    /// `EntityFallingBlock.onUpdate`.
    fn tick_falling(&mut self, id: i32) {
        let Some(mut f) = self.falling.remove(&id) else {
            return;
        };
        let block_pos = |b: &Body| (floor_f64(b.pos.0), floor_f64(b.pos.1), floor_f64(b.pos.2));
        let first = f.fall_time == 0;
        f.fall_time += 1;
        if first {
            // The block leaves its place on the first tick, or the entity gives up.
            let p = block_pos(&f.body);
            if self.level.overworld.block(p.0, p.1, p.2) >> 4 == f.state >> 4 {
                self.falling.insert(id, f);
                self.set_block_notify(p, 0);
                let Some(back) = self.falling.remove(&id) else {
                    return;
                };
                f = back;
            } else {
                f.dead = true;
                self.falling.insert(id, f);
                return;
            }
        }
        f.body.motion.1 -= f64::from(GRAVITY);
        let m = f.body.motion;
        let moved =
            physics::move_entity_quiet(&mut self.level.overworld, &mut f.body, m.0, m.1, m.2);
        // updateFallState.
        if f.body.on_ground {
            if f.fall_distance > 0.0 {
                // BlockFarmland.onFallenUpon only passes the fall on for living entities.
                if moved.below.1 >> 4 != block::FARMLAND {
                    self.falling_block_fall(id, &mut f);
                }
                f.fall_distance = 0.0;
            }
        } else if moved.moved_y < 0.0 {
            f.fall_distance = (f64::from(f.fall_distance) - moved.moved_y) as f32;
        }
        // doBlockCollisions, with the falling block in the world.
        let bb = f.body.bb();
        self.falling.insert(id, f);
        let touched = self.block_collisions(&bb);
        let Some(mut f) = self.falling.remove(&id) else {
            return;
        };
        f.body.touched(touched);
        f.body.motion.0 *= f64::from(DRAG);
        f.body.motion.1 *= f64::from(DRAG);
        f.body.motion.2 *= f64::from(DRAG);
        let p = block_pos(&f.body);
        f.chunk = (p.0 >> 4, p.2 >> 4);
        let mut landed = None;
        let mut drop = false;
        if f.body.on_ground {
            f.body.motion.0 *= f64::from(LANDING_FRICTION);
            f.body.motion.2 *= f64::from(LANDING_FRICTION);
            f.body.motion.1 *= -0.5;
            // TODO(M5): stays alive on piston extensions.
            f.dead = true;
            let w = &mut self.level.overworld;
            let target = w.block(p.0, p.1, p.2);
            // World.canBlockBePlaced: anvils also crush redstone, torches, rails, ...
            let placeable = super::blocks::is_replaceable(target)
                || (f.state >> 4 == block::ANVIL
                    && block_data::material(target >> 4) == Material::Circuits);
            if f.shattered {
                // A shattered anvil leaves nothing.
            } else if placeable
                && (0..256).contains(&p.1)
                && !can_fall_into(w.block(p.0, p.1 - 1, p.2))
            {
                landed = Some(p);
            } else {
                drop = true;
            }
        } else if (f.fall_time > OUT_OF_WORLD_TIME && (p.1 < 1 || p.1 > 256))
            || f.fall_time > MAX_FALL_TIME
        {
            drop = true;
            f.dead = true;
        }
        let (state, pos) = (f.state, f.body.pos);
        self.falling.insert(id, f);
        if let Some(at) = landed {
            self.set_block_notify(at, state);
            if state >> 4 == block::ANVIL {
                self.block_effect(EFFECT_ANVIL_LAND, at);
            }
        }
        if drop {
            self.drop_fallen(pos, state);
        }
    }

    /// `EntityFallingBlock.fall`: an anvil hurts everything it lands on, and may be
    /// damaged itself.
    fn falling_block_fall(&mut self, id: i32, f: &mut FallingBlock) {
        if !f.hurt_entities {
            return;
        }
        let blocks = krol_java::math::ceil_f32(f.fall_distance - 1.0);
        if blocks <= 0 {
            return;
        }
        let anvil = f.state >> 4 == block::ANVIL;
        let source = if anvil {
            DamageSource::Anvil
        } else {
            DamageSource::FallingBlock
        };
        let damage =
            krol_java::math::floor_f32(blocks as f32 * FALL_HURT_AMOUNT).min(FALL_HURT_MAX);
        // The ticking entity is out of `falling`, so it cannot hit itself.
        for (_, target) in self.entities_within(&f.body.bb(), Some(id)) {
            self.attack_target(target, source, damage as f32);
        }
        // The roll uses the entity's own (unseeded) random.
        let chance =
            f64::from(ANVIL_DAMAGE_CHANCE_BASE) + f64::from(blocks) * ANVIL_DAMAGE_CHANCE_PER_BLOCK;
        if anvil && f64::from(self.entity_rand.next_float()) < chance {
            let damage = ((f.state & ANVIL_DAMAGE_MASK) >> ANVIL_DAMAGE_SHIFT) + 1;
            if damage > ANVIL_MAX_DAMAGE {
                f.shattered = true;
            } else {
                f.state = (f.state & !ANVIL_DAMAGE_MASK) | damage << ANVIL_DAMAGE_SHIFT;
            }
        }
    }
}
