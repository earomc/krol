//! The AI of mobs (vanilla `EntityLiving.updateEntityActionState`): the goal tasks
//! (`EntityAITasks` with the `EntityAI*` goals), the move, look and jump helpers, the
//! body helper and ground navigation (`PathNavigateGround`).
//!
//! The goals draw from the mob's own random in vanilla's order, so they all run, even
//! those whose effects krol does not have yet (see the TODOs).
//!
//! TODO(M5): the target tasks (no mob kind has any yet), `EntitySenses`, homes and
//! leashes (`EntityAIMoveTowardsRestriction`), sun avoidance.

use krol_java::math::{atan2, floor_f64, sqrt_f64, wrap_degrees_f32};

use super::Game;
use crate::game::ids::{block, item};
use crate::game::pathfinding::{self, Path, WalkFlags, Walker};
use crate::game::physics::Aabb;

/// `EntityAITasks.tickRate`: tasks are (re)considered every this many ticks.
const TASK_TICK_RATE: i32 = 3;
/// `getVerticalFaceSpeed` of a mob.
const VERTICAL_FACE_SPEED: f32 = 40.0;
/// The look helper turns the head back towards the body this fast when idle.
const IDLE_HEAD_SPEED: f32 = 10.0;
/// The head stays within this of the body while the mob follows a path.
const HEAD_LIMIT: f32 = 75.0;
/// `EntityMoveHelper` turns the body at most this much per tick.
const MOVE_TURN: f32 = 30.0;
/// `getMaxFallHeight` without an attack target.
const MAX_FALL: i32 = 3;
/// `PathNavigate.checkForStuck`: every this many ticks a mob that moved less than
/// the distance (squared) gives up its path.
const STUCK_TICKS: i32 = 100;
const STUCK_DISTANCE_SQ: f64 = 2.25;
/// `despawnEntity`: within this distance (squared) of a player the idle time resets;
/// beyond it an old enough mob may despawn.
const NEAR_PLAYER_SQ: f64 = 1024.0;
const DESPAWN_AGE: i32 = 600;
const DESPAWN_CHANCE: i32 = 800;
/// `EntityAIWander` stops wandering once the mob has been idle this long.
const WANDER_IDLE_LIMIT: i32 = 100;
/// `EntityAIWander`'s default chance (one in this many task ticks).
const WANDER_CHANCE: i32 = 120;
/// `EntityAIWatchClosest`'s default chance per task tick.
const WATCH_CHANCE: f32 = 0.02;
/// `EntityAILookIdle`'s chance per task tick.
const LOOK_IDLE_CHANCE: f32 = 0.02;
/// `EntityAISwimming` jumps with this chance every tick.
const SWIM_JUMP_CHANCE: f32 = 0.8;
/// `EntityAITempt`: looks for players this close, waits this long after giving up,
/// stops this close (squared) to the player.
const TEMPT_RANGE: f64 = 10.0;
const TEMPT_DELAY: i32 = 100;
const TEMPT_STOP_SQ: f64 = 6.25;
/// `EntityAIFollowParent`: parents within this box, followed between these distances
/// (squared), the path renewed every this many ticks.
const PARENT_SEARCH: [f64; 3] = [8.0, 4.0, 8.0];
const PARENT_NEAR_SQ: f64 = 9.0;
const PARENT_FAR_SQ: f64 = 256.0;
const PARENT_REPATH: i32 = 10;
/// `EntityAnimal.getBlockPathWeight` on grass.
const GRASS_WEIGHT: f32 = 10.0;

/// One goal (`EntityAIBase` subclasses) with its state.
#[derive(Debug, Clone)]
enum Goal {
    /// `EntityAISwimming`: jump to keep afloat.
    Swimming,
    /// `EntityAIPanic`: run to a random spot after being hurt or while burning.
    Panic { speed: f64, target: (f64, f64, f64) },
    /// `EntityAIControlledByPlayer`.
    ///
    /// TODO(M5): riding pigs (it never starts without a rider).
    ControlledByPlayer,
    /// `EntityAIMate`.
    ///
    /// TODO(M5): breeding (it only starts with a partner in love nearby, which needs
    /// feeding).
    Mate,
    /// `EntityAITempt`: follow a player holding the item.
    Tempt {
        speed: f64,
        item: i16,
        player: Option<i32>,
        delay: i32,
        avoided_water: bool,
    },
    /// `EntityAIFollowParent`: children follow the nearest adult of their kind.
    FollowParent {
        speed: f64,
        parent: Option<i32>,
        delay: i32,
    },
    /// `EntityAIWander`: walk to a random spot now and then.
    Wander {
        speed: f64,
        chance: i32,
        target: (f64, f64, f64),
        must_update: bool,
    },
    /// `EntityAIWatchClosest` (players): look at the nearest player for a while.
    WatchClosest {
        range: f32,
        chance: f32,
        target: Option<i32>,
        look_time: i32,
    },
    /// `EntityAILookIdle`: look around.
    LookIdle { look: (f64, f64), time: i32 },
}

impl Goal {
    /// `getMutexBits`: goals sharing a bit do not run together.
    fn mutex(&self) -> u32 {
        match self {
            Self::Swimming => 4,
            Self::Panic { .. } | Self::Wander { .. } => 1,
            Self::ControlledByPlayer => 7,
            Self::Mate | Self::Tempt { .. } | Self::LookIdle { .. } => 3,
            Self::FollowParent { .. } => 0,
            Self::WatchClosest { .. } => 2,
        }
    }

    /// `isInterruptible`: every goal so far may be.
    fn interruptible(&self) -> bool {
        true
    }
}

/// `EntityLookHelper`.
#[derive(Debug, Clone, Default)]
struct Look {
    yaw_speed: f32,
    pitch_speed: f32,
    looking: bool,
    pos: (f64, f64, f64),
}

/// `EntityMoveHelper`.
#[derive(Debug, Clone, Default)]
struct Mover {
    pos: (f64, f64, f64),
    speed: f64,
    update: bool,
}

/// `EntityBodyHelper`.
#[derive(Debug, Clone, Default)]
struct BodyHelper {
    counter: i32,
    prev_head: f32,
}

/// `PathNavigateGround`.
#[derive(Debug, Clone)]
pub struct Navigator {
    path: Option<Path>,
    speed: f64,
    total_ticks: i32,
    ticks_at_last_pos: i32,
    last_pos_check: (f64, f64, f64),
    /// `heightRequirement`.
    height_requirement: f32,
    flags: WalkFlags,
}

impl Navigator {
    fn new() -> Self {
        Self {
            path: None,
            speed: 0.0,
            total_ticks: 0,
            ticks_at_last_pos: 0,
            last_pos_check: (0.0, 0.0, 0.0),
            height_requirement: 1.0,
            flags: WalkFlags {
                enter_doors: true,
                break_doors: false,
                avoids_water: false,
                can_swim: false,
            },
        }
    }

    /// `noPath`.
    pub fn no_path(&self) -> bool {
        self.path.as_ref().is_none_or(Path::finished)
    }

    /// `clearPathEntity`.
    fn clear(&mut self) {
        self.path = None;
    }
}

/// A mob's AI state.
#[derive(Debug, Clone)]
pub struct MobAi {
    /// `tasks.taskEntries`: priority and goal, in the order they were added.
    goals: Vec<(i32, Goal)>,
    /// `tasks.executingTaskEntries`: indices into `goals`, in the order they started.
    executing: Vec<usize>,
    tick_count: i32,
    look: Look,
    mover: Mover,
    /// `EntityJumpHelper.isJumping`.
    jump: bool,
    body: BodyHelper,
    /// The navigator.
    pub nav: Navigator,
}

impl MobAi {
    /// The AI of a pig (`EntityPig`'s constructor).
    pub fn pig(pos: (f64, f64, f64)) -> Self {
        let mut ai = Self::empty(pos);
        ai.nav.flags.avoids_water = true;
        // EntityAISwimming's constructor lets the navigator swim.
        ai.nav.flags.can_swim = true;
        ai.goals = vec![
            (0, Goal::Swimming),
            (
                1,
                Goal::Panic {
                    speed: 1.25,
                    target: (0.0, 0.0, 0.0),
                },
            ),
            (2, Goal::ControlledByPlayer),
            (3, Goal::Mate),
            (4, tempt(1.2, item::CARROT_ON_A_STICK)),
            (4, tempt(1.2, item::CARROT)),
            (
                5,
                Goal::FollowParent {
                    speed: 1.1,
                    parent: None,
                    delay: 0,
                },
            ),
            (
                6,
                Goal::Wander {
                    speed: 1.0,
                    chance: WANDER_CHANCE,
                    target: (0.0, 0.0, 0.0),
                    must_update: false,
                },
            ),
            (
                7,
                Goal::WatchClosest {
                    range: 6.0,
                    chance: WATCH_CHANCE,
                    target: None,
                    look_time: 0,
                },
            ),
            (
                8,
                Goal::LookIdle {
                    look: (0.0, 0.0),
                    time: 0,
                },
            ),
        ];
        ai
    }

    fn empty(pos: (f64, f64, f64)) -> Self {
        Self {
            goals: Vec::new(),
            executing: Vec::new(),
            tick_count: 0,
            look: Look::default(),
            // The move helper starts at the mob's position.
            mover: Mover {
                pos,
                ..Mover::default()
            },
            jump: false,
            body: BodyHelper::default(),
            nav: Navigator::new(),
        }
    }
}

fn tempt(speed: f64, item: i16) -> Goal {
    Goal::Tempt {
        speed,
        item,
        player: None,
        delay: 0,
        avoided_water: false,
    }
}

/// `EntityLookHelper.updateRotation` / `EntityLiving.updateRotation`: turn `from`
/// towards `to` by at most `max`.
fn turn(from: f32, to: f32, max: f32) -> f32 {
    let d = wrap_degrees_f32(to - from).clamp(-max, max);
    from + d
}

/// `EntityBodyHelper.computeAngleWithBound`.
fn bound_angle(target: f32, angle: f32, bound: f32) -> f32 {
    let mut d = wrap_degrees_f32(target - angle);
    if d < -bound {
        d = -bound;
    }
    if d >= bound {
        d = bound;
    }
    target - d
}

fn dist_sq(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    let d = (a.0 - b.0, a.1 - b.1, a.2 - b.2);
    d.0 * d.0 + d.1 * d.1 + d.2 * d.2
}

impl Game {
    /// `EntityLiving.updateEntityActionState`: idle time and despawning, the goals,
    /// navigation, then the move, look and jump helpers.
    pub(super) fn mob_action_state(&mut self, id: i32) {
        let Some(mut ai) = self.mobs.get_mut(&id).and_then(|m| m.ai.take()) else {
            return;
        };
        if let Some(m) = self.mobs.get_mut(&id) {
            m.entity_age += 1;
        }
        self.mob_despawn(id);
        self.update_goals(id, &mut ai);
        self.nav_update(id, &mut ai);
        // EntityAnimal.updateAITasks: love ends for children and parents.
        if let Some(m) = self.mobs.get_mut(&id)
            && m.kind.ageable()
            && m.age != 0
        {
            m.in_love = 0;
        }
        self.mover_update(id, &mut ai);
        self.look_update(id, &mut ai);
        if let Some(m) = self.mobs.get_mut(&id) {
            m.jumping = std::mem::take(&mut ai.jump);
            m.ai = Some(ai);
        }
    }

    /// `EntityLiving.despawnEntity`: animals never despawn, but the chance roll still
    /// draws from the random once the mob has been idle long enough.
    fn mob_despawn(&mut self, id: i32) {
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        if m.persistence_required {
            if let Some(m) = self.mobs.get_mut(&id) {
                m.entity_age = 0;
            }
            return;
        }
        let pos = m.body.pos;
        let Some(p) = self.closest_player_to(pos, None) else {
            return;
        };
        let d = dist_sq(self.players[p].pos, pos);
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        let can_despawn = !m.kind.animal();
        if can_despawn && d > NEAR_PLAYER_SQ * 16.0 {
            m.dead = true;
        }
        if m.entity_age > DESPAWN_AGE
            && m.rand.next_int_bounded(DESPAWN_CHANCE) == 0
            && d > NEAR_PLAYER_SQ
            && can_despawn
        {
            m.dead = true;
        } else if d < NEAR_PLAYER_SQ {
            m.entity_age = 0;
        }
    }

    /// `World.getClosestPlayer`: the nearest player not spectating, within `range` if
    /// given, first in join order on ties.
    pub(super) fn closest_player_to(
        &self,
        pos: (f64, f64, f64),
        range: Option<f64>,
    ) -> Option<usize> {
        let mut best: Option<(f64, usize)> = None;
        for (i, p) in self.players.iter().enumerate() {
            if p.gamemode == SPECTATOR {
                continue;
            }
            let d = dist_sq(p.pos, pos);
            if range.is_none_or(|r| d < r * r) && best.is_none_or(|(b, _)| d < b) {
                best = Some((d, i));
            }
        }
        best.map(|(_, i)| i)
    }

    /// The player with entity id `id` if it is alive (`isEntityAlive`).
    fn living_player(&self, id: i32) -> Option<usize> {
        self.players
            .iter()
            .position(|p| p.entity_id == id && p.health() > 0.0 && !p.dead_marked && !p.removed)
    }

    // The goal selector (EntityAITasks.onUpdateTasks).

    fn update_goals(&mut self, id: i32, ai: &mut MobAi) {
        let tick = ai.tick_count;
        ai.tick_count = ai.tick_count.wrapping_add(1);
        if tick % TASK_TICK_RATE == 0 {
            for k in 0..ai.goals.len() {
                if ai.executing.contains(&k) {
                    if can_use(ai, k) && self.goal_continue(id, ai, k) {
                        continue;
                    }
                    self.goal_reset(id, ai, k);
                    ai.executing.retain(|&e| e != k);
                }
                if can_use(ai, k) && self.goal_should_execute(id, ai, k) {
                    self.goal_start(id, ai, k);
                    ai.executing.push(k);
                }
            }
        } else {
            let mut j = 0;
            while j < ai.executing.len() {
                let k = ai.executing[j];
                if self.goal_continue(id, ai, k) {
                    j += 1;
                } else {
                    self.goal_reset(id, ai, k);
                    ai.executing.remove(j);
                }
            }
        }
        for k in ai.executing.clone() {
            self.goal_update(id, ai, k);
        }
    }

    /// Runs `f` on goal `k`, taken out of the AI so that both can be changed.
    fn with_goal<R>(
        &mut self,
        id: i32,
        ai: &mut MobAi,
        k: usize,
        f: impl FnOnce(&mut Self, i32, &mut MobAi, &mut Goal) -> R,
    ) -> R {
        let mut goal = std::mem::replace(&mut ai.goals[k].1, Goal::Swimming);
        let r = f(self, id, ai, &mut goal);
        ai.goals[k].1 = goal;
        r
    }

    /// `shouldExecute`.
    fn goal_should_execute(&mut self, id: i32, ai: &mut MobAi, k: usize) -> bool {
        self.with_goal(id, ai, k, |game, id, ai, goal| {
            game.should_execute(id, ai, goal)
        })
    }

    fn should_execute(&mut self, id: i32, _ai: &mut MobAi, goal: &mut Goal) -> bool {
        let in_lava = self.mob_in_lava(id);
        let Some(m) = self.mobs.get(&id) else {
            return false;
        };
        let pos = m.body.pos;
        match goal {
            Goal::Swimming => m.body.in_water || in_lava,
            Goal::Panic { target, .. } => {
                if m.revenge_target.is_none() && !m.burning() {
                    return false;
                }
                match self.random_target(id, 5, 4) {
                    Some(t) => {
                        *target = t;
                        true
                    }
                    None => false,
                }
            }
            Goal::ControlledByPlayer | Goal::Mate => false,
            Goal::Tempt {
                item,
                player,
                delay,
                ..
            } => {
                if *delay > 0 {
                    *delay -= 1;
                    return false;
                }
                *player = self
                    .closest_player_to(pos, Some(TEMPT_RANGE))
                    .map(|i| self.players[i].entity_id);
                player
                    .and_then(|p| self.players.iter().find(|q| q.entity_id == p))
                    .and_then(|p| p.held())
                    .is_some_and(|held| held.id == *item)
            }
            Goal::FollowParent { parent, .. } => {
                if m.age >= 0 {
                    return false;
                }
                let area = m.body.bb().expand(PARENT_SEARCH);
                let kind = m.kind;
                let mut best: Option<(f64, i32)> = None;
                for (&other, o) in &self.mobs {
                    if o.kind != kind || o.age < 0 || !o.body.bb().intersects(&area) {
                        continue;
                    }
                    let d = dist_sq(o.body.pos, pos);
                    if best.is_none_or(|(b, _)| d <= b) {
                        best = Some((d, other));
                    }
                }
                match best {
                    Some((d, other)) if d >= PARENT_NEAR_SQ => {
                        *parent = Some(other);
                        true
                    }
                    _ => false,
                }
            }
            Goal::Wander {
                chance,
                target,
                must_update,
                ..
            } => {
                if !*must_update {
                    if m.entity_age >= WANDER_IDLE_LIMIT {
                        return false;
                    }
                    let chance = *chance;
                    let Some(m) = self.mobs.get_mut(&id) else {
                        return false;
                    };
                    if m.rand.next_int_bounded(chance) != 0 {
                        return false;
                    }
                }
                match self.random_target(id, 10, 7) {
                    Some(t) => {
                        *target = t;
                        *must_update = false;
                        true
                    }
                    None => false,
                }
            }
            Goal::WatchClosest {
                range,
                chance,
                target,
                ..
            } => {
                let Some(m) = self.mobs.get_mut(&id) else {
                    return false;
                };
                if m.rand.next_float() >= *chance {
                    return false;
                }
                *target = self
                    .closest_player_to(pos, Some(f64::from(*range)))
                    .map(|i| self.players[i].entity_id);
                target.is_some()
            }
            Goal::LookIdle { .. } => {
                let Some(m) = self.mobs.get_mut(&id) else {
                    return false;
                };
                m.rand.next_float() < LOOK_IDLE_CHANCE
            }
        }
    }

    /// `continueExecuting`.
    fn goal_continue(&mut self, id: i32, ai: &mut MobAi, k: usize) -> bool {
        self.with_goal(id, ai, k, |game, id, ai, goal| match goal {
            Goal::Panic { .. } | Goal::Wander { .. } => !ai.nav.no_path(),
            Goal::FollowParent { parent, .. } => {
                let Some(m) = game.mobs.get(&id) else {
                    return false;
                };
                if m.age >= 0 {
                    return false;
                }
                let Some(p) = parent.and_then(|p| game.mobs.get(&p)).filter(|p| p.alive()) else {
                    return false;
                };
                let d = dist_sq(m.body.pos, p.body.pos);
                (PARENT_NEAR_SQ..=PARENT_FAR_SQ).contains(&d)
            }
            Goal::WatchClosest {
                range,
                target,
                look_time,
                ..
            } => {
                let Some(p) = target.and_then(|t| game.living_player(t)) else {
                    return false;
                };
                let Some(m) = game.mobs.get(&id) else {
                    return false;
                };
                let r = f64::from(*range * *range);
                dist_sq(m.body.pos, game.players[p].pos) <= r && *look_time > 0
            }
            Goal::LookIdle { time, .. } => *time >= 0,
            _ => game.should_execute(id, ai, goal),
        })
    }

    /// `startExecuting`.
    fn goal_start(&mut self, id: i32, ai: &mut MobAi, k: usize) {
        self.with_goal(id, ai, k, |game, id, ai, goal| match goal {
            Goal::Panic { speed, target } | Goal::Wander { speed, target, .. } => {
                game.nav_move_to_pos(id, ai, *target, *speed);
            }
            Goal::Tempt { avoided_water, .. } => {
                *avoided_water = ai.nav.flags.avoids_water;
                ai.nav.flags.avoids_water = false;
            }
            Goal::FollowParent { delay, .. } => *delay = 0,
            Goal::WatchClosest { look_time, .. } => {
                if let Some(m) = game.mobs.get_mut(&id) {
                    *look_time = 40 + m.rand.next_int_bounded(40);
                }
            }
            Goal::LookIdle { look, time } => {
                if let Some(m) = game.mobs.get_mut(&id) {
                    // Math.cos and Math.sin of a random angle.
                    let a = std::f64::consts::TAU * m.rand.next_double();
                    *look = (a.cos(), a.sin());
                    *time = 20 + m.rand.next_int_bounded(20);
                }
            }
            _ => {}
        });
    }

    /// `resetTask`.
    fn goal_reset(&mut self, id: i32, ai: &mut MobAi, k: usize) {
        self.with_goal(id, ai, k, |_, _, ai, goal| match goal {
            Goal::Tempt {
                player,
                delay,
                avoided_water,
                ..
            } => {
                *player = None;
                ai.nav.clear();
                *delay = TEMPT_DELAY;
                ai.nav.flags.avoids_water = *avoided_water;
            }
            Goal::FollowParent { parent, .. } => *parent = None,
            Goal::WatchClosest { target, .. } => *target = None,
            _ => {}
        });
    }

    /// `updateTask`.
    fn goal_update(&mut self, id: i32, ai: &mut MobAi, k: usize) {
        self.with_goal(id, ai, k, |game, id, ai, goal| {
            let Some(m) = game.mobs.get(&id) else {
                return;
            };
            let (pos, eye) = (m.body.pos, m.eye_height());
            match goal {
                Goal::Swimming => {
                    if let Some(m) = game.mobs.get_mut(&id)
                        && m.rand.next_float() < SWIM_JUMP_CHANCE
                    {
                        ai.jump = true;
                    }
                }
                Goal::Tempt { speed, player, .. } => {
                    let Some(p) =
                        player.and_then(|p| game.players.iter().find(|q| q.entity_id == p))
                    else {
                        return;
                    };
                    let (ppos, peye) = (p.pos, p.eye_height());
                    look_at(ai, (ppos.0, ppos.1 + f64::from(peye), ppos.2), 30.0);
                    if dist_sq(pos, ppos) < TEMPT_STOP_SQ {
                        ai.nav.clear();
                    } else {
                        game.nav_move_to_entity(id, ai, ppos, *speed);
                    }
                }
                Goal::FollowParent {
                    speed,
                    parent,
                    delay,
                } => {
                    *delay -= 1;
                    if *delay <= 0 {
                        *delay = PARENT_REPATH;
                        if let Some(p) = parent.and_then(|p| game.mobs.get(&p)) {
                            let feet = p.body.pos;
                            game.nav_move_to_entity(id, ai, feet, *speed);
                        }
                    }
                }
                Goal::WatchClosest {
                    target, look_time, ..
                } => {
                    if let Some(p) =
                        target.and_then(|t| game.players.iter().find(|q| q.entity_id == t))
                    {
                        let (ppos, peye) = (p.pos, p.eye_height());
                        look_at(ai, (ppos.0, ppos.1 + f64::from(peye), ppos.2), 10.0);
                    }
                    *look_time -= 1;
                }
                Goal::LookIdle { look, time } => {
                    *time -= 1;
                    look_at(
                        ai,
                        (pos.0 + look.0, pos.1 + f64::from(eye), pos.2 + look.1),
                        10.0,
                    );
                }
                _ => {}
            }
        });
    }

    /// `RandomPositionGenerator.findRandomTarget` (no home): the best of ten random
    /// blocks within `xz` and `y` of the mob by `getBlockPathWeight`.
    fn random_target(&mut self, id: i32, xz: i32, y: i32) -> Option<(f64, f64, f64)> {
        let mut best: Option<(f32, (i32, i32, i32))> = None;
        for _ in 0..10 {
            let m = self.mobs.get_mut(&id)?;
            let dx = m.rand.next_int_bounded(2 * xz + 1) - xz;
            let dy = m.rand.next_int_bounded(2 * y + 1) - y;
            let dz = m.rand.next_int_bounded(2 * xz + 1) - xz;
            let p = m.body.pos;
            let at = (
                dx + floor_f64(p.0),
                dy + floor_f64(p.1),
                dz + floor_f64(p.2),
            );
            let weight = self.block_path_weight(at);
            if best.is_none_or(|(b, _)| weight > b) {
                best = Some((weight, at));
            }
        }
        best.map(|(_, (x, y, z))| (f64::from(x), f64::from(y), f64::from(z)))
    }

    /// `EntityAnimal.getBlockPathWeight`: grass below, else the light.
    fn block_path_weight(&mut self, pos: (i32, i32, i32)) -> f32 {
        if self.level.overworld.block(pos.0, pos.1 - 1, pos.2) >> 4 == block::GRASS {
            return GRASS_WEIGHT;
        }
        // World.getLightBrightness: the overworld's brightness table.
        let light = self.light_from_neighbors(pos);
        let dark = 1.0 - light as f32 / 15.0;
        (1.0 - dark) / (dark * 3.0 + 1.0) - 0.5
    }

    // Navigation (PathNavigate / PathNavigateGround).

    fn walker(&mut self, id: i32) -> Option<Walker> {
        let in_lava = self.mob_in_lava(id);
        let m = self.mobs.get(&id)?;
        Some(Walker {
            pos: m.body.pos,
            min: m.body.bb().min,
            width: m.body.width,
            height: m.body.height,
            in_water: m.body.in_water,
            in_lava,
            max_fall: MAX_FALL,
        })
    }

    /// `canNavigate`: on the ground, or swimming.
    fn nav_can_navigate(&mut self, id: i32, ai: &MobAi) -> bool {
        let in_lava = self.mob_in_lava(id);
        self.mobs.get(&id).is_some_and(|m| {
            m.body.on_ground || (ai.nav.flags.can_swim && (m.body.in_water || in_lava))
        })
    }

    /// `PathFinder.createEntityPathTo` within the follow range, if the mob can
    /// navigate.
    fn nav_path_to(&mut self, id: i32, ai: &MobAi, target: (f64, f64, f64)) -> Option<Path> {
        if !self.nav_can_navigate(id, ai) {
            return None;
        }
        let range = self.mobs.get(&id)?.follow_range() as f32;
        let walker = self.walker(id)?;
        pathfinding::find_path(
            &mut self.level.overworld,
            walker,
            ai.nav.flags,
            target,
            range,
        )
    }

    /// `tryMoveToXYZ`: a path to the block at `target`.
    fn nav_move_to_pos(
        &mut self,
        id: i32,
        ai: &mut MobAi,
        target: (f64, f64, f64),
        speed: f64,
    ) -> bool {
        // getPathToXYZ, then the block's center (in float).
        let (x, y, z) = (floor_f64(target.0), target.1 as i32, floor_f64(target.2));
        let center = (
            f64::from(x as f32 + 0.5),
            f64::from(y as f32 + 0.5),
            f64::from(z as f32 + 0.5),
        );
        let path = self.nav_path_to(id, ai, center);
        self.nav_set_path(id, ai, path, speed)
    }

    /// `tryMoveToEntityLiving`: a path to an entity's feet; no path keeps the old one.
    fn nav_move_to_entity(
        &mut self,
        id: i32,
        ai: &mut MobAi,
        feet: (f64, f64, f64),
        speed: f64,
    ) -> bool {
        match self.nav_path_to(id, ai, feet) {
            Some(path) => self.nav_set_path(id, ai, Some(path), speed),
            None => false,
        }
    }

    /// `setPath`: an equal path keeps the current one (and its progress).
    fn nav_set_path(&mut self, id: i32, ai: &mut MobAi, path: Option<Path>, speed: f64) -> bool {
        let Some(path) = path else {
            ai.nav.path = None;
            return false;
        };
        if !ai.nav.path.as_ref().is_some_and(|p| path.same_points(p)) {
            ai.nav.path = Some(path);
        }
        if ai.nav.path.as_ref().is_some_and(|p| p.length == 0) {
            return false;
        }
        ai.nav.speed = speed;
        ai.nav.ticks_at_last_pos = ai.nav.total_ticks;
        ai.nav.last_pos_check = self.nav_entity_position(id, ai);
        true
    }

    /// `getEntityPosition`: the feet, at the water's surface when swimming.
    fn nav_entity_position(&mut self, id: i32, ai: &MobAi) -> (f64, f64, f64) {
        let Some(m) = self.mobs.get(&id) else {
            return (0.0, 0.0, 0.0);
        };
        let (pos, min_y, in_water) = (m.body.pos, m.body.bb().min[1], m.body.in_water);
        // getPathablePosY.
        let y = if in_water && ai.nav.flags.can_swim {
            let (x, z) = (floor_f64(pos.0), floor_f64(pos.2));
            let mut y = min_y as i32;
            let mut climbed = 0;
            loop {
                let id = self.level.overworld.block(x, y, z) >> 4;
                if !matches!(id, block::FLOWING_WATER | block::WATER) {
                    break y;
                }
                y += 1;
                climbed += 1;
                if climbed > 16 {
                    break min_y as i32;
                }
            }
        } else {
            (min_y + 0.5) as i32
        };
        (pos.0, f64::from(y), pos.2)
    }

    /// `onUpdateNavigation`: follow the path and aim the move helper at its next
    /// point.
    fn nav_update(&mut self, id: i32, ai: &mut MobAi) {
        ai.nav.total_ticks += 1;
        if ai.nav.no_path() {
            return;
        }
        let Some(width) = self.mobs.get(&id).map(|m| m.body.width) else {
            return;
        };
        if self.nav_can_navigate(id, ai) {
            self.nav_follow(id, ai);
        } else if let Some(path) = ai.nav.path.as_ref()
            && path.index < path.length
        {
            let here = self.nav_entity_position(id, ai);
            let Some(path) = ai.nav.path.as_mut() else {
                return;
            };
            let next = path.vector(path.index, width);
            let on_ground = self.mobs.get(&id).is_some_and(|m| m.body.on_ground);
            if here.1 > next.1
                && !on_ground
                && floor_f64(here.0) == floor_f64(next.0)
                && floor_f64(here.2) == floor_f64(next.2)
            {
                path.index += 1;
            }
        }
        if ai.nav.no_path() {
            return;
        }
        let Some(path) = ai.nav.path.as_ref() else {
            return;
        };
        let v = path.vector(path.index, width);
        // The move target sits on whatever is below the point.
        let bb = Aabb::new([v.0, v.1, v.2], [v.0, v.1, v.2]).expand([0.5, 0.5, 0.5]);
        let boxes = crate::game::physics::colliding_boxes(
            &mut self.level.overworld,
            &bb.add_coord([0.0, -1.0, 0.0]),
        );
        let lifted = bb.offset([0.0, 1.0, 0.0]);
        let mut dy = -1.0;
        for b in &boxes {
            dy = b.axis_offset(&lifted, 1, dy);
        }
        ai.mover = Mover {
            pos: (v.0, v.1 + dy, v.2),
            speed: ai.nav.speed,
            update: true,
        };
    }

    /// `pathFollow`: skip the points already reached on this level, and those a
    /// straight walk can cut, then check for being stuck.
    fn nav_follow(&mut self, id: i32, ai: &mut MobAi) {
        let here = self.nav_entity_position(id, ai);
        let Some(m) = self.mobs.get(&id) else {
            return;
        };
        let (width, height) = (m.body.width, m.body.height);
        let in_water = m.body.in_water;
        let Some(path) = ai.nav.path.as_mut() else {
            return;
        };
        let mut level_end = path.length;
        for k in path.index..path.length {
            if path.point(k).1 != here.1 as i32 {
                level_end = k;
                break;
            }
        }
        let reach = width * width * ai.nav.height_requirement;
        for k in path.index..level_end {
            if dist_sq(here, path.vector(k, width)) < f64::from(reach) {
                path.index = k + 1;
            }
        }
        let size_x = krol_java::math::ceil_f32(width);
        let size_y = height as i32 + 1;
        let size_z = size_x;
        let mut k = level_end;
        while k > 0 {
            k -= 1;
            let Some(path) = ai.nav.path.as_ref() else {
                return;
            };
            if k < path.index {
                break;
            }
            let target = path.vector(k, width);
            if self.direct_path(here, target, (size_x, size_y, size_z), in_water) {
                if let Some(path) = ai.nav.path.as_mut() {
                    path.index = k;
                }
                break;
            }
        }
        // checkForStuck.
        if ai.nav.total_ticks - ai.nav.ticks_at_last_pos > STUCK_TICKS {
            if dist_sq(here, ai.nav.last_pos_check) < STUCK_DISTANCE_SQ {
                ai.nav.clear();
            }
            ai.nav.ticks_at_last_pos = ai.nav.total_ticks;
            ai.nav.last_pos_check = here;
        }
    }

    /// `PathNavigateGround.isDirectPathBetweenPoints`: whether the mob can walk
    /// straight from `from` to `to`, checking the blocks along the line.
    fn direct_path(
        &mut self,
        from: (f64, f64, f64),
        to: (f64, f64, f64),
        size: (i32, i32, i32),
        in_water: bool,
    ) -> bool {
        let mut x = floor_f64(from.0);
        let mut z = floor_f64(from.2);
        let mut dx = to.0 - from.0;
        let mut dz = to.2 - from.2;
        let len_sq = dx * dx + dz * dz;
        if len_sq < 1.0e-8 {
            return false;
        }
        let inv = 1.0 / len_sq.sqrt();
        dx *= inv;
        dz *= inv;
        let y = from.1 as i32;
        if !self.safe_to_stand_at(
            (x, y, z),
            (size.0 + 2, size.1, size.2 + 2),
            from,
            (dx, dz),
            in_water,
        ) {
            return false;
        }
        let step_x = 1.0 / dx.abs();
        let step_z = 1.0 / dz.abs();
        let mut next_x = f64::from(x) - from.0;
        let mut next_z = f64::from(z) - from.2;
        if dx >= 0.0 {
            next_x += 1.0;
        }
        if dz >= 0.0 {
            next_z += 1.0;
        }
        next_x /= dx;
        next_z /= dz;
        let sx = if dx < 0.0 { -1 } else { 1 };
        let sz = if dz < 0.0 { -1 } else { 1 };
        let end_x = floor_f64(to.0);
        let end_z = floor_f64(to.2);
        let mut left_x = end_x - x;
        let mut left_z = end_z - z;
        while left_x * sx > 0 || left_z * sz > 0 {
            if next_x < next_z {
                next_x += step_x;
                x += sx;
                left_x = end_x - x;
            } else {
                next_z += step_z;
                z += sz;
                left_z = end_z - z;
            }
            if !self.safe_to_stand_at((x, y, z), size, from, (dx, dz), in_water) {
                return false;
            }
        }
        true
    }

    /// `isSafeToStandAt`: the box around `(x, y, z)` is clear, and the blocks below it
    /// ahead of `origin` are solid ground, not water (unless swimming) or lava.
    fn safe_to_stand_at(
        &mut self,
        (x, y, z): (i32, i32, i32),
        size: (i32, i32, i32),
        origin: (f64, f64, f64),
        dir: (f64, f64),
        in_water: bool,
    ) -> bool {
        let x0 = x - size.0 / 2;
        let z0 = z - size.2 / 2;
        if !self.position_clear((x0, y, z0), size, origin, dir) {
            return false;
        }
        for i in x0..x0 + size.0 {
            for k in z0..z0 + size.2 {
                let ox = f64::from(i) + 0.5 - origin.0;
                let oz = f64::from(k) + 0.5 - origin.2;
                if ox * dir.0 + oz * dir.1 < 0.0 {
                    continue;
                }
                let id = self.level.overworld.block(i, y - 1, k) >> 4;
                match crate::game::block_data::material(id) {
                    crate::game::block_data::Material::Air => return false,
                    crate::game::block_data::Material::Water if !in_water => return false,
                    crate::game::block_data::Material::Lava => return false,
                    _ => {}
                }
            }
        }
        true
    }

    /// `isPositionClear`: every block of the box ahead of `origin` is passable.
    fn position_clear(
        &mut self,
        (x, y, z): (i32, i32, i32),
        size: (i32, i32, i32),
        origin: (f64, f64, f64),
        dir: (f64, f64),
    ) -> bool {
        for i in x..x + size.0 {
            for j in y..y + size.1 {
                for k in z..z + size.2 {
                    let ox = f64::from(i) + 0.5 - origin.0;
                    let oz = f64::from(k) + 0.5 - origin.2;
                    if ox * dir.0 + oz * dir.1 < 0.0 {
                        continue;
                    }
                    let state = self.level.overworld.block(i, j, k);
                    if !pathfinding::passable(&mut self.level.overworld, (i, j, k), state) {
                        return false;
                    }
                }
            }
        }
        true
    }

    // The helpers.

    /// `EntityMoveHelper.onUpdateMoveHelper`: turn towards the target and walk at the
    /// navigator's speed, jumping when it is higher and close.
    fn mover_update(&mut self, id: i32, ai: &mut MobAi) {
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        m.move_forward = 0.0;
        if !std::mem::take(&mut ai.mover.update) {
            return;
        }
        let feet = floor_f64(m.body.bb().min[1] + 0.5);
        let dx = ai.mover.pos.0 - m.body.pos.0;
        let dz = ai.mover.pos.2 - m.body.pos.2;
        let dy = ai.mover.pos.1 - f64::from(feet);
        let d = dx * dx + dy * dy + dz * dz;
        if d < f64::from(2.500_000_3e-7_f32) {
            return;
        }
        let yaw = (atan2(dz, dx) * 180.0 / f64::from(std::f32::consts::PI)) as f32 - 90.0;
        m.base.yaw = limit_angle(m.base.yaw, yaw, MOVE_TURN);
        let speed = (ai.mover.speed * m.movement_speed()) as f32;
        // EntityLiving.setAIMoveSpeed.
        m.ai_move_speed = speed;
        m.move_forward = speed;
        if dy > 0.0 && dx * dx + dz * dz < 1.0 {
            ai.jump = true;
        }
    }

    /// `EntityLookHelper.onUpdateLook`: turn the head towards what the mob looks at,
    /// or back to the body.
    fn look_update(&mut self, id: i32, ai: &mut MobAi) {
        let no_path = ai.nav.no_path();
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        m.base.pitch = 0.0;
        if std::mem::take(&mut ai.look.looking) {
            let dx = ai.look.pos.0 - m.body.pos.0;
            let dy = ai.look.pos.1 - (m.body.pos.1 + f64::from(m.eye_height()));
            let dz = ai.look.pos.2 - m.body.pos.2;
            let horizontal = f64::from(sqrt_f64(dx * dx + dz * dz));
            let pi = f64::from(std::f32::consts::PI);
            let yaw = (atan2(dz, dx) * 180.0 / pi) as f32 - 90.0;
            let pitch = (-(atan2(dy, horizontal) * 180.0 / pi)) as f32;
            m.base.pitch = turn(m.base.pitch, pitch, ai.look.pitch_speed);
            m.head_yaw = turn(m.head_yaw, yaw, ai.look.yaw_speed);
        } else {
            m.head_yaw = turn(m.head_yaw, m.body_yaw, IDLE_HEAD_SPEED);
        }
        let d = wrap_degrees_f32(m.head_yaw - m.body_yaw);
        if !no_path {
            if d < -HEAD_LIMIT {
                m.head_yaw = m.body_yaw - HEAD_LIMIT;
            }
            if d > HEAD_LIMIT {
                m.head_yaw = m.body_yaw + HEAD_LIMIT;
            }
        }
    }

    /// `EntityBodyHelper.updateRenderAngles` (in place of the plain body turn of
    /// `EntityLivingBase`): a moving mob's body follows its yaw and drags the head
    /// along; a still one turns its body towards the head, ever slower after ten ticks.
    pub(super) fn mob_body_update(&mut self, id: i32) {
        let Some(m) = self.mobs.get_mut(&id) else {
            return;
        };
        let Some(ai) = m.ai.as_mut() else {
            return;
        };
        let dx = m.body.pos.0 - m.prev_pos.0;
        let dz = m.body.pos.2 - m.prev_pos.2;
        if dx * dx + dz * dz > f64::from(2.500_000_3e-7_f32) {
            m.body_yaw = m.base.yaw;
            m.head_yaw = bound_angle(m.body_yaw, m.head_yaw, HEAD_LIMIT);
            ai.body.prev_head = m.head_yaw;
            ai.body.counter = 0;
            return;
        }
        let mut bound = HEAD_LIMIT;
        if (m.head_yaw - ai.body.prev_head).abs() > 15.0 {
            ai.body.counter = 0;
            ai.body.prev_head = m.head_yaw;
        } else {
            ai.body.counter += 1;
            if ai.body.counter > 10 {
                bound = (1.0 - (ai.body.counter - 10) as f32 / 10.0).max(0.0) * HEAD_LIMIT;
            }
        }
        m.body_yaw = bound_angle(m.head_yaw, m.body_yaw, bound);
    }
}

/// `EntityLookHelper.setLookPosition` with the mob's vertical face speed.
fn look_at(ai: &mut MobAi, pos: (f64, f64, f64), yaw_speed: f32) {
    ai.look = Look {
        yaw_speed,
        pitch_speed: VERTICAL_FACE_SPEED,
        looking: true,
        pos,
    };
}

/// `EntityMoveHelper.limitAngle`: turn by at most `max`, kept within 0 to 360.
fn limit_angle(from: f32, to: f32, max: f32) -> f32 {
    let mut r = from + wrap_degrees_f32(to - from).clamp(-max, max);
    if r < 0.0 {
        r += 360.0;
    } else if r > 360.0 {
        r -= 360.0;
    }
    r
}

/// `EntityAITasks.canUse`: no running goal of the same or a higher priority shares a
/// mutex bit with goal `k`, and no running goal of a lower priority refuses to be
/// interrupted.
fn can_use(ai: &MobAi, k: usize) -> bool {
    let (priority, goal) = &ai.goals[k];
    for (j, (other_priority, other)) in ai.goals.iter().enumerate() {
        if j == k {
            continue;
        }
        let running = ai.executing.contains(&j);
        if priority >= other_priority {
            if goal.mutex() & other.mutex() != 0 && running {
                return false;
            }
        } else if !other.interruptible() && running {
            return false;
        }
    }
    true
}

/// `WorldSettings.GameType.SPECTATOR`.
const SPECTATOR: i32 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_are_limited_and_wrapped() {
        assert_eq!(turn(0.0, 90.0, 10.0), 10.0);
        assert_eq!(turn(170.0, -170.0, 30.0), 190.0);
        assert_eq!(limit_angle(350.0, 20.0, 30.0), 20.0);
        assert_eq!(limit_angle(10.0, -40.0, 30.0), 340.0);
    }

    #[test]
    fn the_body_helper_keeps_angles_within_the_bound() {
        assert_eq!(bound_angle(0.0, 100.0, 75.0), 75.0);
        assert_eq!(bound_angle(0.0, -100.0, 75.0), -75.0);
        assert_eq!(bound_angle(0.0, 30.0, 75.0), 30.0);
    }

    #[test]
    fn pig_goals_are_in_priority_order() {
        let ai = MobAi::pig((0.0, 0.0, 0.0));
        let priorities: Vec<i32> = ai.goals.iter().map(|(p, _)| *p).collect();
        assert_eq!(priorities, [0, 1, 2, 3, 4, 4, 5, 6, 7, 8]);
        assert!(ai.nav.flags.avoids_water && ai.nav.flags.can_swim);
    }
}
