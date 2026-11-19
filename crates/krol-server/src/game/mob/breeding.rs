//! Players using items on mobs and animals breeding (vanilla:
//! `EntityPlayer.interactWith`, `EntityLiving.interactFirst`, `EntityPig.interact`,
//! `EntityAnimal.interact`, `EntityAgeable.interact`, `Item.itemInteractionForEntity`
//! of saddles and name tags, `EntityAIMate.spawnBaby`).
//!
//! TODO(M5): leads (`interactFirst` leashes a mob to a player holding one, and lets
//! go of it for its player), riding saddled pigs (`EntityPig.interact` mounts the
//! player; krol answers the click without mounting).

use krol_protocol::packets::play::ClientboundPlay;

use super::{Mob, MobKind};
use crate::game::Game;
use crate::game::ids::item;
use crate::game::item_data;

/// `EntityAnimal.setInLove`: love lasts this many ticks.
const LOVE_TICKS: i32 = 600;
/// `S19PacketEntityStatus` 18: hearts over an animal in love.
const STATUS_LOVE: i8 = 18;
/// `EntityAIMate.spawnBaby`: the parents may breed again after this many ticks, the
/// child grows up after this many.
const PARENT_AGE: i32 = 6000;
const CHILD_AGE: i32 = -24000;
/// The hearts around the parent when the child is born (client-side, but drawn from
/// the parent's random).
const BIRTH_HEARTS: usize = 7;
/// The experience orb of a birth is worth 1 to this many points.
const BIRTH_XP: i32 = 7;
/// The `animalsBred` statistic.
const ANIMALS_BRED: &str = "stat.animalsBred";

impl MobKind {
    /// `EntityAnimal.isBreedingItem`.
    fn breeding_item(self) -> i16 {
        match self {
            Self::Pig => item::CARROT,
        }
    }
}

impl Mob {
    /// `EntityAgeable.func_175501_a`: the age moves on by `seconds` (children stop at 0;
    /// a forced growth also adds to `ForcedAge`, which becomes the age once it is 0).
    fn add_growth(&mut self, seconds: i32, forced: bool) {
        let old = self.age;
        let mut age = old.wrapping_add(seconds.wrapping_mul(20));
        if age > 0 {
            // onGrowingAdult does nothing for animals.
            age = 0;
        }
        let grown = age.wrapping_sub(old);
        self.set_age(age);
        if forced {
            // The green particles' counter only runs on clients.
            self.forced_age = self.forced_age.wrapping_add(grown);
        }
        if self.age == 0 {
            self.set_age(self.forced_age);
        }
    }
}

impl Game {
    /// `EntityPlayer.interactWith` on a mob: the mob's own answer to the click
    /// (`interactFirst`), else the held item's (`ItemStack.interactWithEntity`).
    /// Whatever is used up comes out of the held stack in survival and adventure;
    /// creative players keep it.
    pub(in crate::game) fn interact_mob(&mut self, i: usize, id: i32) {
        // Spectators only open inventories.
        if self.players[i].gamemode == super::SPECTATOR {
            return;
        }
        if !self.mobs.contains_key(&id) {
            return;
        }
        if self.mob_interact(i, id) {
            return;
        }
        let Some(held) = self.players[i].held().cloned() else {
            return;
        };
        match held.id {
            // ItemSaddle: adult pigs without a saddle get one; the click counts on any
            // pig.
            item::SADDLE => {
                let Some(m) = self.mobs.get_mut(&id) else {
                    return;
                };
                if m.kind != MobKind::Pig {
                    return;
                }
                if !m.saddled && !m.child() {
                    m.set_saddled(true);
                    let pos = m.body.pos;
                    self.play_sound_near_except(None, "mob.horse.leather", pos, 0.5, 1.0);
                    self.use_held(i);
                }
            }
            // ItemNameTag: a named tag names the mob, which no longer despawns.
            item::NAME_TAG => {
                let Some(name) = item_data::custom_name(&held).map(str::to_owned) else {
                    return;
                };
                let Some(m) = self.mobs.get_mut(&id) else {
                    return;
                };
                m.base.custom_name = Some(name);
                m.metadata_updated(super::CUSTOM_NAME_INDEX);
                m.persistence_required = true;
                self.use_held(i);
            }
            _ => {}
        }
    }

    /// `EntityLiving.interactFirst` and the mob's `interact`: whether the mob took the
    /// click.
    fn mob_interact(&mut self, i: usize, id: i32) -> bool {
        // TODO(M5): leads (a leashed mob lets go of its player; a lead leashes it).
        let held = self.players[i].held().cloned();
        let me = self.players[i].entity_id;
        let Some(m) = self.mobs.get_mut(&id) else {
            return false;
        };
        // EntityAnimal.interact: the breeding item puts an adult in love, or makes a
        // child grow by a tenth of its time left.
        if let Some(held) = &held
            && held.id == m.kind.breeding_item()
        {
            if m.age == 0 && m.in_love <= 0 {
                m.in_love = LOVE_TICKS;
                m.player_in_love = Some(me);
                self.send_to_tracking(
                    id,
                    &ClientboundPlay::EntityStatus {
                        entity_id: id,
                        status: STATUS_LOVE,
                    },
                );
                self.use_held(i);
                return true;
            }
            if m.child() {
                let seconds = ((-m.age / 20) as f32 * 0.1) as i32;
                m.add_growth(seconds, true);
                self.use_held(i);
                return true;
            }
        }
        // EntityAgeable.interact: any spawn egg takes the click; one of the mob's kind
        // makes a child.
        if let Some(held) = &held
            && held.id == item::SPAWN_EGG
        {
            if held.damage == i16::from(m.kind.type_id()) {
                let name = item_data::custom_name(held).map(str::to_owned);
                self.spawn_child(id, name);
                self.use_held(i);
            }
            return true;
        }
        let Some(m) = self.mobs.get(&id) else {
            return false;
        };
        match m.kind {
            // EntityPig.interact: a saddled pig takes a player in, unless someone else
            // rides it. TODO(M5): riding pigs (the click is taken, nobody mounts).
            MobKind::Pig => m.saddled,
        }
    }

    /// One of the held item used up (`stackSize--`, the slot emptied at 0), unless
    /// the player is in creative.
    fn use_held(&mut self, i: usize) {
        let p = &mut self.players[i];
        if p.gamemode == super::super::player::Player::CREATIVE {
            return;
        }
        let slot = 36 + p.selected;
        if let Some(s) = p.inventory[slot].as_mut() {
            s.count -= 1;
            if s.count <= 0 {
                p.inventory[slot] = None;
            }
        }
    }

    /// `createChild`, then the child set up like `EntityAIMate.spawnBaby` and
    /// `EntityAgeable.interact` do: a child at the parent's position facing south,
    /// named after a named spawn egg. Returns its entity id.
    fn spawn_child(&mut self, parent: i32, name: Option<String>) -> Option<i32> {
        let (kind, pos) = self.mobs.get(&parent).map(|m| (m.kind, m.body.pos))?;
        let id = self.next_entity_id();
        // The living constructor turns the head at random (Math.random).
        let head = self.new_stand_head();
        let seed = self.new_mob_seed();
        let mut child = Mob::new(kind, pos, head, seed);
        child.set_age(CHILD_AGE);
        child.base.yaw = 0.0;
        child.base.pitch = 0.0;
        if name.is_some() {
            child.base.custom_name = name;
        }
        child.dirty_metadata = child.metadata().iter().map(|e| e.index).collect();
        self.mobs.insert(id, child);
        self.spawn_entity_in_world(id);
        Some(id)
    }

    /// `EntityAIMate.getNearbyMate`: the closest mob of the same kind within 8 blocks
    /// (by box) that can mate with `id`.
    pub(super) fn nearby_mate(&self, id: i32) -> Option<i32> {
        let m = self.mobs.get(&id)?;
        let area = m.body.bb().expand([MATE_RANGE; 3]);
        let mut best: Option<(f64, i32)> = None;
        for (&other, o) in &self.mobs {
            if other == id || o.kind != m.kind || !o.body.bb().intersects(&area) {
                continue;
            }
            // EntityAnimal.canMateWith.
            if m.in_love <= 0 || o.in_love <= 0 {
                continue;
            }
            let d = dist_sq(m.body.pos, o.body.pos);
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, other));
            }
        }
        best.map(|(_, other)| other)
    }

    /// `EntityAIMate.spawnBaby`: the player who fed either parent counts the birth, the
    /// parents rest, the child is born with hearts around and an experience orb.
    pub(super) fn breed(&mut self, id: i32, mate: i32) {
        let feeder = self
            .mobs
            .get(&id)
            .and_then(|m| m.player_in_love)
            .or_else(|| self.mobs.get(&mate).and_then(|m| m.player_in_love));
        // Vanilla keeps the player object; one who left counts it on that object, lost.
        if let Some(i) = feeder.and_then(|p| {
            self.players
                .iter()
                .position(|q| q.entity_id == p && !q.removed)
        }) {
            self.add_stat(i, ANIMALS_BRED, 1);
        }
        for parent in [id, mate] {
            if let Some(m) = self.mobs.get_mut(&parent) {
                m.set_age(PARENT_AGE);
            }
        }
        for parent in [id, mate] {
            if let Some(m) = self.mobs.get_mut(&parent) {
                m.in_love = 0;
            }
        }
        if self.spawn_child(id, None).is_none() {
            return;
        }
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        for _ in 0..BIRTH_HEARTS {
            for _ in 0..3 {
                let _ = m.rand.next_gaussian();
            }
            for _ in 0..3 {
                let _ = m.rand.next_double();
            }
        }
        if self.level.data.game_rule("doMobLoot") != Some("false") {
            let Some(m) = self.mobs.get_mut(&id) else {
                return;
            };
            let value = m.rand.next_int_bounded(BIRTH_XP) + 1;
            let pos = m.body.pos;
            self.spawn_orb(pos, value);
        }
    }
}

/// `EntityAIMate.getNearbyMate` looks this far around the mob's box.
const MATE_RANGE: f64 = 8.0;

fn dist_sq(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    let d = (a.0 - b.0, a.1 - b.1, a.2 - b.2);
    d.0 * d.0 + d.1 * d.1 + d.2 * d.2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feeding_children_grows_them_and_counts_the_forced_age() {
        let mut pig = Mob::new(MobKind::Pig, (0.0, 64.0, 0.0), 0.0, 1);
        pig.set_age(-1100);
        // A tenth of 55 seconds left, rounded down: 5 seconds.
        let seconds = ((-pig.age / 20) as f32 * 0.1) as i32;
        pig.add_growth(seconds, true);
        assert_eq!((pig.age, pig.forced_age), (-1000, 100));
        // Growing past adulthood stops at 0, which then takes the forced age.
        pig.add_growth(60, true);
        assert_eq!((pig.age, pig.forced_age), (1100, 1100));
        assert!(!pig.child());
    }
}
