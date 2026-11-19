//! Leads (vanilla: the leash of `EntityLiving`, `EntityCreature.updateLeashedState`,
//! `EntityLeashKnot` and `ItemLead`).
//!
//! A player using a lead on a mob leashes it to himself; clicking it again lets go
//! (the mob drops the lead, except for creative players). Clicking a fence ties every
//! mob he leads within 7 blocks to a leash knot on it (`BlockFence.onBlockActivated`
//! with any item, `ItemLead.onItemUse` with a lead). Clicking a knot with a lead ties
//! the mobs he leads to it as well; when there are none (or without a lead) the knot
//! breaks, and its mobs drop their leads.
//!
//! A leashed mob checks its holder after its update: a dead mob or a holder that is
//! gone (logged out, respawned, a broken knot) drops the lead. A creature gets a home
//! 5 blocks around its holder and `EntityAIMoveTowardsRestriction`, walks towards the
//! holder beyond 4 blocks, is pulled beyond 6 and breaks free beyond 10, dropping the
//! lead.
//!
//! A loaded mob finds its holder in its first update: the player with the saved UUID
//! within 10 blocks, or the knot on the saved fence (tied anew if missing, since knots
//! are never saved).
//!
//! TODO(M5): sitting tamed mobs (they only break free beyond 10 blocks, without the home
//! and pull), leashable mobs that are no `EntityCreature` (only the base update), and
//! a parity scenario that loads a leashed mob.

use krol_java::math::sqrt_f32;
use krol_nbt::{Compound, Tag, TagType};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack};

use super::ai::Goal;
use super::{Game, Mob};
use crate::game::hanging::{Hanging, Kind};
use crate::game::ids::item;
use crate::game::physics::Aabb;

/// `ItemLead.attachToFence` and `EntityLeashKnot.interactFirst` look for mobs this far
/// around the fence.
const FENCE_REACH: f64 = 7.0;
/// `recreateLeash` looks for the saved player this far around the mob.
const HOLDER_SEARCH: f64 = 10.0;
/// `EntityCreature.updateLeashedState`: the home radius around the holder, and the
/// distances beyond which the mob walks towards the holder, is pulled, breaks free.
const HOME_RADIUS: f32 = 5.0;
const WALK_DISTANCE: f32 = 4.0;
const PULL_DISTANCE: f32 = 6.0;
const BREAK_DISTANCE: f32 = 10.0;
/// The pull adds the squared direction (keeping its sign) times this.
const PULL: f64 = 0.4;
/// `EntityCreature.aiBase` is added with this priority.
const RESTRICTION_PRIORITY: i32 = 2;
/// The navigator speed of `tryMoveToEntityLiving` and of the restriction goal.
const LEASH_SPEED: f64 = 1.0;
/// `S1BPacketEntityAttach` without a holder.
const NO_HOLDER: i32 = -1;

impl Mob {
    /// Whether the mob is on a lead (`getLeashed`).
    pub fn leashed(&self) -> bool {
        self.leashed
    }

    /// `leashedToEntity`.
    pub fn leash_holder(&self) -> Option<i32> {
        self.leash_holder
    }

    /// `EntityCreature.isWithinHomeDistanceFromPosition`.
    pub(super) fn within_home(&self, pos: (i32, i32, i32)) -> bool {
        match self.home {
            None => true,
            Some((home, max)) => block_dist_sq(home, pos) < f64::from(max * max),
        }
    }

    /// The home (`getHomePosition`, `getMaximumHomeDistance`) while it has one.
    pub(super) fn home(&self) -> Option<((i32, i32, i32), f32)> {
        self.home
    }
}

/// `Vec3i.distanceSq` between two block positions.
pub(super) fn block_dist_sq(a: (i32, i32, i32), b: (i32, i32, i32)) -> f64 {
    let d = (
        f64::from(a.0) - f64::from(b.0),
        f64::from(a.1) - f64::from(b.1),
        f64::from(a.2) - f64::from(b.2),
    );
    d.0 * d.0 + d.1 * d.1 + d.2 * d.2
}

/// A box reaching `r` around `center` on every axis.
fn around(center: (f64, f64, f64), r: f64) -> Aabb {
    Aabb::new(
        [center.0 - r, center.1 - r, center.2 - r],
        [center.0 + r, center.1 + r, center.2 + r],
    )
}

impl Game {
    /// Where a leash holder is (position, and the bottom of its box, which paths aim
    /// at), if it is still in the world (not `isDead`).
    fn holder(&self, holder: i32) -> Option<((f64, f64, f64), f64)> {
        if let Some(p) = self.players.iter().find(|p| p.entity_id == holder) {
            return (!p.dead_marked && !p.removed).then_some((p.pos, p.pos.1));
        }
        let h = self.hanging.get(&holder)?;
        (!h.dead).then_some((h.body.pos, h.bb.min[1]))
    }

    /// The `Leash` tag that saves `holder` (a player's UUID, a knot's fence).
    fn holder_tag(&self, holder: i32) -> Compound {
        let mut c = Compound::new();
        if let Some(p) = self.players.iter().find(|p| p.entity_id == holder) {
            let id = p.profile.id;
            c.insert("UUIDMost", Tag::Long((id >> 64) as u64 as i64));
            c.insert("UUIDLeast", Tag::Long(id as u64 as i64));
        } else if let Some(h) = self.hanging.get(&holder) {
            c.insert("X", Tag::Int(h.tile.0));
            c.insert("Y", Tag::Int(h.tile.1));
            c.insert("Z", Tag::Int(h.tile.2));
        }
        c
    }

    /// `setLeashedToEntity`: the mob is held by `holder`, and with `send` the players
    /// tracking it see the lead.
    fn set_leashed(&mut self, id: i32, holder: i32, send: bool) {
        let tag = self.holder_tag(holder);
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        m.leashed = true;
        m.leash_holder = Some(holder);
        m.saved_holder = Some(tag);
        if send {
            self.send_to_tracking(
                id,
                &ClientboundPlay::AttachEntity {
                    entity_id: id,
                    vehicle_id: holder,
                    leash: true,
                },
            );
        }
    }

    /// `clearLeashed(send, drop)`: the mob lets go of its lead, dropping it at its feet
    /// with `drop`, and with `send` the players tracking it see it go.
    pub(super) fn clear_leashed(&mut self, id: i32, send: bool, drop: bool) {
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        if !m.leashed {
            return;
        }
        m.leashed = false;
        m.leash_holder = None;
        m.saved_holder = None;
        let pos = m.body.pos;
        if drop {
            self.mob_drop_item(pos, single(item::LEAD));
        }
        if send {
            self.send_to_tracking(
                id,
                &ClientboundPlay::AttachEntity {
                    entity_id: id,
                    vehicle_id: NO_HOLDER,
                    leash: true,
                },
            );
        }
    }

    /// The mobs whose box intersects `area`, in `getEntitiesWithinAABB` order (by
    /// chunk, x then z, then section, then the order they entered the section).
    fn mobs_within(&self, area: &Aabb) -> Vec<i32> {
        let mut found: Vec<((i32, i32, usize, usize), i32)> = self
            .mobs
            .iter()
            .filter(|(_, m)| m.body.bb().intersects(area))
            .filter_map(|(&id, _)| Some((self.chunk_entities.list_key(id)?, id)))
            .collect();
        found.sort_unstable();
        found.into_iter().map(|(_, id)| id).collect()
    }

    /// `EntityLeashKnot.getKnotForPosition`: the knot on the fence at `tile` (one being
    /// removed counts too).
    fn knot_at(&self, tile: (i32, i32, i32)) -> Option<i32> {
        let (x, y, z) = (f64::from(tile.0), f64::from(tile.1), f64::from(tile.2));
        let area = Aabb::new([x - 1.0, y - 1.0, z - 1.0], [x + 1.0, y + 1.0, z + 1.0]);
        self.hanging
            .iter()
            .find(|(_, h)| h.kind == Kind::LeashKnot && h.tile == tile && h.bb.intersects(&area))
            .map(|(&id, _)| id)
    }

    /// `EntityLeashKnot.createKnot`: a new knot on the fence at `tile`.
    fn create_knot(&mut self, tile: (i32, i32, i32)) -> i32 {
        let id = self.next_entity_id();
        self.hanging.insert(id, Hanging::knot(tile));
        self.spawn_entity_in_world(id);
        id
    }

    /// `EntityLiving.interactFirst`'s leash part: the player's own leashed mob lets go
    /// (dropping the lead unless he is in creative), and a lead leashes a mob that takes
    /// one. Returns whether the click was taken.
    pub(super) fn leash_interact(&mut self, i: usize, id: i32) -> bool {
        let me = self.players[i].entity_id;
        let creative = self.players[i].gamemode == crate::game::player::Player::CREATIVE;
        let Some(m) = self.mobs.get(&id) else {
            return false;
        };
        if m.leashed && m.leash_holder == Some(me) {
            self.clear_leashed(id, true, !creative);
            return true;
        }
        let holds_lead = self.players[i].held().is_some_and(|s| s.id == item::LEAD);
        if holds_lead && !m.leashed && m.kind.leashable() {
            self.set_leashed(id, me, true);
            self.use_held(i);
            return true;
        }
        false
    }

    /// `ItemLead.attachToFence`: every mob within 7 blocks that the player leads is
    /// tied to the knot on the fence at `pos` (made for the first one). Returns whether
    /// any was.
    pub(in crate::game) fn attach_to_fence(&mut self, i: usize, pos: (i32, i32, i32)) -> bool {
        let me = self.players[i].entity_id;
        let mut knot = self.knot_at(pos);
        let (x, y, z) = (f64::from(pos.0), f64::from(pos.1), f64::from(pos.2));
        let area = around((x, y, z), FENCE_REACH);
        let mut attached = false;
        for id in self.mobs_within(&area) {
            let Some(m) = self.mobs.get(&id) else {
                continue;
            };
            if m.leashed && m.leash_holder == Some(me) {
                let k = match knot {
                    Some(k) => k,
                    None => {
                        let k = self.create_knot(pos);
                        knot = Some(k);
                        k
                    }
                };
                self.set_leashed(id, k, true);
                attached = true;
            }
        }
        attached
    }

    /// `EntityLeashKnot.interactFirst`: with a lead, the mobs within 7 blocks that the
    /// player leads are tied to this knot; if there were none, the knot breaks, and a
    /// creative player also frees the mobs tied to it (keeping their leads).
    pub(in crate::game) fn interact_knot(&mut self, i: usize, knot: i32) {
        let Some(pos) = self.hanging.get(&knot).map(|h| h.body.pos) else {
            return;
        };
        let me = self.players[i].entity_id;
        let area = around(pos, FENCE_REACH);
        let mut taken = false;
        if self.players[i].held().is_some_and(|s| s.id == item::LEAD) {
            for id in self.mobs_within(&area) {
                if self
                    .mobs
                    .get(&id)
                    .is_some_and(|m| m.leashed && m.leash_holder == Some(me))
                {
                    self.set_leashed(id, knot, true);
                    taken = true;
                }
            }
        }
        if taken {
            return;
        }
        if let Some(h) = self.hanging.get_mut(&knot) {
            h.dead = true;
        }
        if self.players[i].gamemode == crate::game::player::Player::CREATIVE {
            for id in self.mobs_within(&area) {
                if self
                    .mobs
                    .get(&id)
                    .is_some_and(|m| m.leashed && m.leash_holder == Some(knot))
                {
                    self.clear_leashed(id, true, false);
                }
            }
        }
    }

    /// The player with entity id `entity_id` respawned as a new entity: mobs it led
    /// hold on to the old one, which is dead.
    pub(in crate::game) fn leash_holder_replaced(&mut self, entity_id: i32) {
        for m in self.mobs.values_mut() {
            if m.leash_holder == Some(entity_id) {
                m.leash_holder = None;
            }
        }
    }

    /// `recreateLeash`: a loaded mob finds the holder its `Leash` tag names. The players
    /// tracking it are not told (vanilla quirk: they see the lead only once they track
    /// the mob anew).
    fn recreate_leash(&mut self, id: i32) {
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        let Some(tag) = m.leash.take() else {
            return;
        };
        if !m.leashed {
            return;
        }
        let area = m.body.bb().expand([HOLDER_SEARCH; 3]);
        if tag.contains_key_of("UUIDMost", Some(TagType::Long))
            && tag.contains_key_of("UUIDLeast", Some(TagType::Long))
        {
            let uuid = (tag.get_i64("UUIDMost") as u64 as u128) << 64
                | tag.get_i64("UUIDLeast") as u64 as u128;
            let found = self
                .players
                .iter()
                .find(|p| {
                    !p.removed
                        && p.profile.id == uuid
                        && Aabb::at(p.pos, 0.6, 1.8).intersects(&area)
                })
                .map(|p| p.entity_id);
            if let Some(holder) = found {
                let tag = self.holder_tag(holder);
                if let Some(m) = self.mobs.get_mut(&id) {
                    m.leash_holder = Some(holder);
                    m.saved_holder = Some(tag);
                }
            }
        } else if ["X", "Y", "Z"].iter().all(|k| tag.contains_key_of(k, None)) {
            let tile = (tag.get_i32("X"), tag.get_i32("Y"), tag.get_i32("Z"));
            let knot = match self.knot_at(tile) {
                Some(k) => k,
                None => self.create_knot(tile),
            };
            let tag = self.holder_tag(knot);
            if let Some(m) = self.mobs.get_mut(&id) {
                m.leash_holder = Some(knot);
                m.saved_holder = Some(tag);
            }
        } else {
            self.clear_leashed(id, false, true);
        }
    }

    /// `EntityLiving.updateLeashedState` and `EntityCreature`'s, after the mob's update.
    pub(super) fn update_leashed_state(&mut self, id: i32) {
        if self.mobs.get(&id).is_some_and(|m| m.leash.is_some()) {
            self.recreate_leash(id);
        }
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        if m.leashed {
            if !m.alive() {
                self.clear_leashed(id, true, true);
            }
            let gone = self
                .mobs
                .get(&id)
                .and_then(|m| m.leash_holder)
                .and_then(|h| self.holder(h))
                .is_none();
            if gone {
                self.clear_leashed(id, true, true);
            }
        }
        // EntityCreature.
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        let held_by = m
            .leash_holder
            .filter(|_| m.leashed)
            .and_then(|h| self.holder(h));
        if let Some((holder_pos, holder_bottom)) = held_by {
            let Some(m) = self.mobs.get_mut(&id) else {
                return;
            };
            // Vanilla quirk: the home is the holder's position cast to int (towards
            // zero), not floored.
            let home = (
                holder_pos.0 as i32,
                holder_pos.1 as i32,
                holder_pos.2 as i32,
            );
            m.home = Some((home, HOME_RADIUS));
            let pos = m.body.pos;
            // getDistanceToEntity, in float.
            let d = (
                (pos.0 - holder_pos.0) as f32,
                (pos.1 - holder_pos.1) as f32,
                (pos.2 - holder_pos.2) as f32,
            );
            let dist = sqrt_f32(d.0 * d.0 + d.1 * d.1 + d.2 * d.2);
            let Some(mut ai) = m.ai.take() else {
                return;
            };
            if !m.movement_task_set {
                ai.add_goal(
                    RESTRICTION_PRIORITY,
                    Goal::MoveTowardsRestriction {
                        speed: LEASH_SPEED,
                        target: (0.0, 0.0, 0.0),
                    },
                );
                ai.nav.flags.avoids_water = false;
                m.movement_task_set = true;
            }
            if dist > WALK_DISTANCE {
                let feet = (holder_pos.0, holder_bottom, holder_pos.2);
                self.nav_move_to_entity(id, &mut ai, feet, LEASH_SPEED);
            }
            let Some(m) = self.mobs.get_mut(&id) else {
                return;
            };
            m.ai = Some(ai);
            if dist > PULL_DISTANCE {
                let dist = f64::from(dist);
                let dx = (holder_pos.0 - pos.0) / dist;
                let dy = (holder_pos.1 - pos.1) / dist;
                let dz = (holder_pos.2 - pos.2) / dist;
                m.body.motion.0 += dx * dx.abs() * PULL;
                m.body.motion.1 += dy * dy.abs() * PULL;
                m.body.motion.2 += dz * dz.abs() * PULL;
            }
            if dist > BREAK_DISTANCE {
                self.clear_leashed(id, true, true);
            }
        } else if let Some(m) = self.mobs.get_mut(&id)
            && !m.leashed
            && m.movement_task_set
        {
            m.movement_task_set = false;
            if let Some(ai) = m.ai.as_mut() {
                ai.remove_restriction_goal();
                ai.nav.flags.avoids_water = true;
            }
            m.home = None;
        }
    }
}

/// One of an item.
fn single(id: i16) -> ItemStack {
    ItemStack {
        id,
        count: 1,
        damage: 0,
        nbt: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::mob::MobKind;
    use crate::game::mob::ai::MobAi;

    #[test]
    fn home_distance_is_strictly_inside_the_radius() {
        let mut pig = Mob::new(MobKind::Pig, (0.5, 64.0, 0.5), 0.0, 1);
        assert!(pig.within_home((100, 64, 100)), "no home: anywhere");
        pig.home = Some(((0, 64, 0), HOME_RADIUS));
        assert!(pig.within_home((4, 64, 0)));
        assert!(pig.within_home((3, 64, 3)), "18 < 25");
        assert!(!pig.within_home((5, 64, 0)), "25 is not < 25");
    }

    #[test]
    fn removing_the_restriction_goal_keeps_the_others_running() {
        let mut ai = MobAi::pig((0.0, 64.0, 0.0));
        let others = ai.goal_count();
        ai.add_goal(
            RESTRICTION_PRIORITY,
            Goal::MoveTowardsRestriction {
                speed: LEASH_SPEED,
                target: (0.0, 0.0, 0.0),
            },
        );
        ai.set_executing(vec![others, 6]);
        ai.remove_restriction_goal();
        assert_eq!(ai.goal_count(), others);
        assert_eq!(ai.executing(), &[6]);
    }

    #[test]
    fn knots_sit_in_the_middle_of_their_fence() {
        let k = Hanging::knot((10, 4, -3));
        assert_eq!(k.body.pos, (10.5, 4.5, -2.5));
        assert_eq!(k.bb.min, [10.3125, 4.375, -2.6875]);
        assert_eq!(k.bb.max, [10.6875, 4.875, -2.3125]);
        assert_eq!(k.kind.saved_id(), "LeashKnot");
    }
}
