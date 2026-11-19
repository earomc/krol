//! Spawn eggs used on blocks and on liquids (vanilla `ItemMonsterPlacer`). On a mob
//! the egg makes a child ([`super::breeding`]).

use krol_java::math::wrap_degrees_f32;
use krol_protocol::packets::play::ItemStack;

use super::{CUSTOM_NAME_INDEX, Mob, MobKind};
use crate::game::Game;
use crate::game::buckets::UseResult;
use crate::game::ids::block;
use crate::game::placement;
use crate::game::player::Player;

/// A mob hatched in a liquid stands this far into the liquid's block.
const LIQUID_LIFT: f64 = 0.5;

impl Game {
    /// `ItemMonsterPlacer.onItemUse`: the egg hatches its mob in front of the clicked
    /// face, named after a named egg, and is used up unless the player is in creative.
    /// The use counts even if nothing hatched.
    ///
    /// TODO(M5): on a mob spawner the egg sets what the spawner spawns (spawners are
    /// kept as saved; the egg does nothing there yet); eggs of the mobs krol does not
    /// simulate yet hatch nothing.
    pub(in crate::game) fn place_spawn_egg(&mut self, i: usize, pos: (i32, i32, i32), face: u8) {
        let Some(stack) = self.players[i].held().cloned() else {
            return;
        };
        // canPlayerEdit: adventure players and spectators never get here.
        let clicked = self.level.overworld.block(pos.0, pos.1, pos.2) >> 4;
        if clicked == block::MOB_SPAWNER {
            return;
        }
        let at = placement::offset(pos, face);
        // Vanilla quirk: the half block lift on top of a fence never happens, as the
        // check asks whether the clicked block *state* is a `BlockFence`.
        let spawn_at = (
            f64::from(at.0) + 0.5,
            f64::from(at.1),
            f64::from(at.2) + 0.5,
        );
        let name = crate::game::item_data::custom_name(&stack).map(str::to_owned);
        if self.spawn_creature(stack.damage, spawn_at, name).is_some() {
            self.use_held_egg(i);
        }
        self.count_item_use(i);
    }

    /// `ItemMonsterPlacer.onItemRightClick`: aimed at a liquid in reach (in a block
    /// the player may change), the egg hatches its mob in it like on a block, and the
    /// use counts.
    pub(in crate::game) fn use_spawn_egg(&mut self, i: usize, held: &ItemStack) -> UseResult {
        let Some(hit) = self.view_ray_trace(i, true) else {
            return UseResult::Unchanged;
        };
        if self.is_block_protected(i, hit.pos) || matches!(self.players[i].gamemode, 2 | 3) {
            return UseResult::Unchanged;
        }
        let state = self.level.overworld.block(hit.pos.0, hit.pos.1, hit.pos.2);
        if !matches!(
            state >> 4,
            block::FLOWING_WATER | block::WATER | block::FLOWING_LAVA | block::LAVA
        ) {
            return UseResult::Unchanged;
        }
        let spawn_at = (
            f64::from(hit.pos.0) + 0.5,
            f64::from(hit.pos.1) + LIQUID_LIFT,
            f64::from(hit.pos.2) + 0.5,
        );
        let name = crate::game::item_data::custom_name(held).map(str::to_owned);
        if self.spawn_creature(held.damage, spawn_at, name).is_none() {
            return UseResult::Unchanged;
        }
        let creative = self.players[i].gamemode == Player::CREATIVE;
        self.use_held_egg(i);
        self.count_item_use(i);
        if creative {
            UseResult::Unchanged
        } else {
            UseResult::Resized
        }
    }

    /// `ItemMonsterPlacer.spawnCreature`: a new mob of the egg's kind at `pos`, facing
    /// a random way (the world's random), with its initial spawn setup; it joins the
    /// world and makes its ambient sound. Then `onItemUse` names it after a named egg
    /// (`name`). `None` for eggs of no mob krol simulates.
    pub(in crate::game) fn spawn_creature(
        &mut self,
        egg: i16,
        pos: (f64, f64, f64),
        name: Option<String>,
    ) -> Option<i32> {
        // TODO(M5): the other mobs with eggs.
        let kind = MobKind::from_type_id(i32::from(egg))?;
        let id = self.next_entity_id();
        let head = self.new_stand_head();
        let seed = self.new_mob_seed();
        let mut m = Mob::new(kind, pos, head, seed);
        // setLocationAndAngles, then the head and body turned the same way.
        let yaw = wrap_degrees_f32(self.world_rand.next_float() * 360.0);
        m.base.yaw = yaw;
        m.base.pitch = 0.0;
        m.head_yaw = yaw;
        m.body_yaw = yaw;
        m.initial_spawn(&mut self.entity_rand);
        // The name is set after the mob joined the world, but the spawn packets hold
        // the data watcher itself and are written out later, so they show it too: set
        // it before, flagged as a change.
        if name.is_some() {
            m.base.custom_name = name;
            m.metadata_updated(CUSTOM_NAME_INDEX);
        }
        self.mobs.insert(id, m);
        self.spawn_entity_in_world(id);
        // playLivingSound.
        self.mob_sound(id, kind.sounds().0);
        Some(id)
    }

    /// `stackSize--` on the held egg, unless the player is in creative (the slot is
    /// emptied by the caller's resync).
    fn use_held_egg(&mut self, i: usize) {
        let p = &mut self.players[i];
        if p.gamemode == Player::CREATIVE {
            return;
        }
        let held = 36 + p.selected;
        if let Some(s) = p.inventory[held].as_mut() {
            s.count -= 1;
        }
    }
}
