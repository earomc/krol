//! Beds (vanilla `ItemBed`, `BlockBed`, `EntityPlayer.trySleep` / `wakeUpPlayer`,
//! `WorldServer.areAllPlayersAsleep`): placing, sleeping, skipping the night, spawn
//! points and respawning at the bed.
//!
//! TODO(M5): a sleeping player's 0.2 x 0.2 box; mobs keeping players awake
//! (`tile.bed.notSafe`); beds exploding in the nether and the end; forced spawn points
//! (`/spawnpoint`).

use krol_protocol::packets::play::ClientboundPlay;

use super::Game;
use super::block_data::material;
use super::ids::block;

/// `bed` (the block) and `bed` (the item).
pub const BED: u16 = 26;
pub const BED_ITEM: i16 = 355;
/// Bed metadata: the horizontal facing (foot to head), occupied, head part.
const FACING: u16 = 3;
const OCCUPIED: u16 = 4;
const HEAD: u16 = 8;
/// A sleeping player lies this high above the bed's block.
const LYING_HEIGHT: f32 = 0.6875;
/// A player is fully asleep after 100 ticks in bed; the timer runs on to 110 after
/// waking up.
const FULLY_ASLEEP: i32 = 100;
const AFTER_WAKING: i32 = 110;
/// How far (blocks) from the head a player may be to lie down.
const REACH_HORIZONTAL: f64 = 3.0;
const REACH_VERTICAL: f64 = 2.0;
/// Night is when the skylight is dimmed by 4 or more (`World.isDaytime`).
const NIGHT_SKYLIGHT: i32 = 4;
/// A day has 24000 ticks.
const DAY: i64 = 24000;
/// `S2BPacketChangeGameState` reason: "Your home bed was missing or obstructed".
const BED_MISSING: u8 = 0;
/// `S0BPacketAnimation` id of leaving a bed.
const LEAVE_BED_ANIMATION: u8 = 2;
/// Game modes.
const SPECTATOR: i32 = 3;
const CREATIVE: i32 = 1;

/// `trySleep`'s result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SleepStatus {
    Ok,
    NotPossibleNow,
    TooFarAway,
    OtherProblem,
}

/// `(dx, dz)` of a horizontal facing (`EnumFacing.getHorizontal`: south, west, north,
/// east).
fn facing_offset(meta: u16) -> (i32, i32) {
    match meta & FACING {
        0 => (0, 1),
        1 => (-1, 0),
        2 => (0, -1),
        _ => (1, 0),
    }
}

/// The other half of the bed at `pos`: towards the facing from the foot, against it
/// from the head.
pub fn other_half(pos: (i32, i32, i32), meta: u16) -> (i32, i32, i32) {
    let (dx, dz) = facing_offset(meta);
    if meta & HEAD != 0 {
        (pos.0 - dx, pos.1, pos.2 - dz)
    } else {
        (pos.0 + dx, pos.1, pos.2 + dz)
    }
}

/// `BlockBed.getItemDropped` / `dropBlockAsItemWithChance`: only the foot drops the
/// bed (the head rolls nothing at all).
pub fn drops_item(meta: u16) -> bool {
    meta & HEAD == 0
}

impl Game {
    fn bed_block(&mut self, p: (i32, i32, i32)) -> u16 {
        self.level.overworld.block(p.0, p.1, p.2)
    }

    /// `BlockBed.hasRoomForPlayer`: a solid top below and nothing solid in the two
    /// blocks a player stands in.
    fn has_room_for_player(&mut self, p: (i32, i32, i32)) -> bool {
        let below = self.bed_block((p.0, p.1 - 1, p.2));
        super::placement::solid_top(below)
            && !material(self.bed_block(p) >> 4).is_solid()
            && !material(self.bed_block((p.0, p.1 + 1, p.2)) >> 4).is_solid()
    }

    /// `BlockBed.getSafeExitLocation(world, pos, 0)`: the first spot with room around
    /// the bed, scanning from behind the head along the bed.
    fn safe_exit(&mut self, pos: (i32, i32, i32)) -> Option<(i32, i32, i32)> {
        let meta = self.bed_block(pos) & 15;
        let (fx, fz) = facing_offset(meta);
        for k in 0..=1 {
            let x0 = pos.0 - fx * k - 1;
            let z0 = pos.2 - fz * k - 1;
            for x in x0..=x0 + 2 {
                for z in z0..=z0 + 2 {
                    let p = (x, pos.1, z);
                    if self.has_room_for_player(p) {
                        return Some(p);
                    }
                }
            }
        }
        None
    }

    /// `ItemBed.onItemUse`: on top of a block, a bed along the player's facing, both
    /// halves on solid tops. Returns whether the item was used.
    pub(super) fn place_bed(&mut self, i: usize, clicked: (i32, i32, i32), face: u8) -> bool {
        if face != super::placement::UP {
            return false;
        }
        let replaceable = super::blocks::is_replaceable(self.bed_block(clicked));
        let foot = if replaceable {
            clicked
        } else {
            (clicked.0, clicked.1 + 1, clicked.2)
        };
        let yaw = self.players[i].rotation.0;
        let index = (krol_java::math::floor_f64(f64::from(yaw * 4.0 / 360.0) + 0.5) & 3) as u16;
        let (dx, dz) = facing_offset(index);
        let head = (foot.0 + dx, foot.1, foot.2 + dz);
        if self.is_block_protected(i, foot) || self.is_block_protected(i, head) {
            return false;
        }
        let air = |s: u16| s >> 4 == block::AIR;
        let foot_state = self.bed_block(foot);
        let head_state = self.bed_block(head);
        let foot_ok = replaceable || air(foot_state);
        let head_ok = super::blocks::is_replaceable(head_state) || air(head_state);
        let below_foot = self.bed_block((foot.0, foot.1 - 1, foot.2));
        let below_head = self.bed_block((head.0, head.1 - 1, head.2));
        if !(foot_ok
            && head_ok
            && super::placement::solid_top(below_foot)
            && super::placement::solid_top(below_head))
        {
            return false;
        }
        let state = BED << 4 | index;
        self.set_block_notify(foot, state);
        if self.bed_block(foot) == state {
            self.set_block_notify(head, state | HEAD);
        }
        let p = &mut self.players[i];
        if p.gamemode != CREATIVE {
            let slot = 36 + p.selected;
            if let Some(s) = &mut p.inventory[slot] {
                s.count -= 1;
            }
        }
        self.count_item_use(i);
        true
    }

    /// `BlockBed.onBlockActivated` in the overworld: lie down in the bed, or say why
    /// not. Always takes the click.
    pub(super) fn use_bed(&mut self, i: usize, pos: (i32, i32, i32), state: u16) -> bool {
        let mut head = pos;
        let mut state = state;
        if state & HEAD == 0 {
            head = other_half(pos, state & 15);
            state = self.bed_block(head);
            if state >> 4 != BED {
                return true;
            }
        }
        if state & OCCUPIED != 0 {
            let taken = self
                .players
                .iter()
                .any(|p| p.sleeping && p.bed == Some(head));
            if taken {
                self.bed_message(i, "tile.bed.occupied");
                return true;
            }
            state &= !OCCUPIED;
            self.set_bed_state_quietly(head.0, head.1, head.2, state);
        }
        match self.try_sleep(i, head) {
            SleepStatus::Ok => {
                self.set_bed_state_quietly(head.0, head.1, head.2, state | OCCUPIED);
            }
            SleepStatus::NotPossibleNow => self.bed_message(i, "tile.bed.noSleep"),
            SleepStatus::TooFarAway | SleepStatus::OtherProblem => {}
        }
        true
    }

    /// `setBlockState(pos, state, 4)`: changes the occupied bit without telling
    /// neighbors or clients (no light change either).
    fn set_bed_state_quietly(&mut self, x: i32, y: i32, z: i32, state: u16) {
        self.level.overworld.set_block_unmarked(x, y, z, state);
    }

    fn bed_message(&self, i: usize, key: &str) {
        let msg = crate::chat::Component::translate(key, vec![]);
        self.players[i].send(ClientboundPlay::Chat {
            json: msg.to_json_string(),
            position: 1,
        });
    }

    /// `World.isDaytime`.
    fn is_daytime(&self) -> bool {
        self.skylight_subtracted < NIGHT_SKYLIGHT
    }

    /// `EntityPlayerMP.trySleep`.
    fn try_sleep(&mut self, i: usize, head: (i32, i32, i32)) -> SleepStatus {
        let p = &self.players[i];
        if p.sleeping || p.health() <= 0.0 {
            return SleepStatus::OtherProblem;
        }
        if self.is_daytime() {
            return SleepStatus::NotPossibleNow;
        }
        if (p.pos.0 - f64::from(head.0)).abs() > REACH_HORIZONTAL
            || (p.pos.1 - f64::from(head.1)).abs() > REACH_VERTICAL
            || (p.pos.2 - f64::from(head.2)).abs() > REACH_HORIZONTAL
        {
            return SleepStatus::TooFarAway;
        }
        // TODO(M5): monsters within 8 blocks keep the player awake (NOT_SAFE).
        let meta = self.bed_block(head) & 15;
        let (mut fx, mut fz) = (0.5f32, 0.5f32);
        match meta & FACING {
            0 => fz = 0.9,
            2 => fz = 0.1,
            1 => fx = 0.1,
            _ => fx = 0.9,
        }
        let lying = (
            f64::from(head.0 as f32 + fx),
            f64::from(head.1 as f32 + LYING_HEIGHT),
            f64::from(head.2 as f32 + fz),
        );
        let p = &mut self.players[i];
        p.pos = lying;
        p.sleeping = true;
        p.sleep_timer = 0;
        p.bed = Some(head);
        p.motion = (0.0, 0.0, 0.0);
        self.update_sleeping_flag();
        let entity_id = self.players[i].entity_id;
        let packet = ClientboundPlay::UseBed {
            entity_id,
            pos: head,
        };
        self.send_to_tracking(entity_id, &packet);
        let p = &mut self.players[i];
        let rot = p.rotation;
        Self::set_player_location(p, lying, rot);
        p.send(packet);
        SleepStatus::Ok
    }

    /// `EntityPlayerMP.wakeUpPlayer(immediately, updateWorldFlag, setSpawn)`.
    pub(super) fn wake_up(
        &mut self,
        i: usize,
        immediately: bool,
        update_flag: bool,
        set_spawn: bool,
    ) {
        let entity_id = self.players[i].entity_id;
        if self.players[i].sleeping {
            let packet = ClientboundPlay::Animation {
                entity_id,
                animation: LEAVE_BED_ANIMATION,
            };
            self.players[i].send(packet.clone());
            self.send_to_tracking(entity_id, &packet);
        }
        let bed = self.players[i].bed;
        if let Some(head) = bed {
            let state = self.bed_block(head);
            if state >> 4 == BED {
                self.set_bed_state_quietly(head.0, head.1, head.2, state & !OCCUPIED);
                let exit = self.safe_exit(head).unwrap_or((head.0, head.1 + 1, head.2));
                self.players[i].pos = (
                    f64::from(exit.0 as f32 + 0.5),
                    f64::from(exit.1 as f32 + 0.1),
                    f64::from(exit.2 as f32 + 0.5),
                );
            }
        }
        self.players[i].sleeping = false;
        if update_flag {
            self.update_sleeping_flag();
        }
        let p = &mut self.players[i];
        p.sleep_timer = if immediately { 0 } else { FULLY_ASLEEP };
        if set_spawn {
            set_spawn_point(p, bed);
        }
        let (pos, rot) = (p.pos, p.rotation);
        Self::set_player_location(p, pos, rot);
    }

    /// `WorldServer.updateAllPlayersSleepingFlag`: everyone but spectators is in bed.
    pub(super) fn update_sleeping_flag(&mut self) {
        let spectators = self
            .players
            .iter()
            .filter(|p| p.gamemode == SPECTATOR)
            .count();
        let sleeping = self
            .players
            .iter()
            .filter(|p| p.gamemode != SPECTATOR && p.sleeping)
            .count();
        self.all_sleeping = sleeping > 0 && sleeping >= self.players.len() - spectators;
    }

    /// `EntityPlayer.onUpdate`'s sleep timer: a sleeping player wakes when the bed goes
    /// or the day comes.
    pub(super) fn sleep_tick(&mut self, i: usize) {
        let p = &mut self.players[i];
        if p.sleeping {
            p.sleep_timer = (p.sleep_timer + 1).min(FULLY_ASLEEP);
            let in_bed = p
                .bed
                .is_some_and(|b| self.level.overworld.block(b.0, b.1, b.2) >> 4 == BED);
            if !in_bed {
                self.wake_up(i, true, true, false);
            } else if self.is_daytime() {
                self.wake_up(i, false, true, true);
            }
        } else if p.sleep_timer > 0 {
            p.sleep_timer += 1;
            if p.sleep_timer >= AFTER_WAKING {
                p.sleep_timer = 0;
            }
        }
    }

    /// `WorldServer.tick`: when everyone sleeps soundly, the night (and the weather)
    /// is skipped and everyone wakes up.
    pub(super) fn check_all_asleep(&mut self) {
        let asleep = self.all_sleeping
            && self
                .players
                .iter()
                .all(|p| p.gamemode != SPECTATOR && p.sleeping && p.sleep_timer >= FULLY_ASLEEP);
        if !asleep {
            return;
        }
        if self.level.data.game_rule("doDaylightCycle") != Some("false") {
            let next = self.level.data.day_time() + DAY;
            self.level.data.set_day_time(next - next % DAY);
        }
        // wakeAllPlayers.
        self.all_sleeping = false;
        for i in 0..self.players.len() {
            if self.players[i].sleeping {
                self.wake_up(i, false, false, true);
            }
        }
        // resetRainAndThunder.
        let d = &mut self.level.data;
        d.set_rain_time(0);
        d.data.insert("raining", krol_nbt::Tag::Byte(0));
        d.set_thunder_time(0);
        d.data.insert("thundering", krol_nbt::Tag::Byte(0));
    }

    /// `BlockBed.onNeighborBlockChange`: a half without its other half goes (only the
    /// foot drops the bed).
    pub(super) fn bed_neighbor_changed(&mut self, pos: (i32, i32, i32), state: u16) {
        let other = other_half(pos, state & 15);
        if self.bed_block(other) >> 4 == BED {
            return;
        }
        self.set_block_notify(pos, 0);
        if drops_item(state & 15) {
            self.drop_block_items(pos, state);
        }
    }

    /// `BlockBed.onBlockHarvested`: breaking the head in creative removes the foot
    /// first (so it drops nothing).
    pub(super) fn bed_harvested_creative(&mut self, pos: (i32, i32, i32), state: u16) {
        if state >> 4 == BED && state & HEAD != 0 {
            let foot = other_half(pos, state & 15);
            if self.bed_block(foot) >> 4 == BED {
                self.set_block_notify(foot, 0);
            }
        }
    }

    /// `EntityPlayer.getBedSpawnLocation` for a respawn: the safe spot next to the bed
    /// of the player's spawn point, if the bed is still there. `None` with a spawn point
    /// set also tells the player the bed is gone.
    pub(super) fn bed_respawn(
        &mut self,
        i: usize,
        spawn_point: Option<((i32, i32, i32), bool)>,
    ) -> Option<(f64, f64, f64)> {
        let (bed, forced) = spawn_point?;
        let spot = if self.bed_block(bed) >> 4 == BED {
            self.safe_exit(bed)
        } else if forced {
            let free = |s: u16| {
                let m = material(s >> 4);
                !m.is_solid() && !m.is_liquid()
            };
            let (here, above) = (
                self.bed_block(bed),
                self.bed_block((bed.0, bed.1 + 1, bed.2)),
            );
            (free(here) && free(above)).then_some(bed)
        } else {
            None
        };
        match spot {
            Some(s) => {
                // setSpawnPoint(bed, forced) on the new player.
                set_spawn_point(&mut self.players[i], Some(bed));
                self.players[i]
                    .data
                    .insert("SpawnForced", krol_nbt::Tag::Byte(i8::from(forced)));
                Some((
                    f64::from(s.0 as f32 + 0.5),
                    f64::from(s.1 as f32 + 0.1),
                    f64::from(s.2 as f32 + 0.5),
                ))
            }
            None => {
                self.players[i].send(ClientboundPlay::ChangeGameState {
                    reason: BED_MISSING,
                    value: 0.0,
                });
                None
            }
        }
    }
}

/// A player's spawn point from its data (`SpawnX/Y/Z`, `SpawnForced`).
pub fn spawn_point(data: &krol_nbt::Compound) -> Option<((i32, i32, i32), bool)> {
    let has = |k| data.contains_key(k);
    (has("SpawnX") && has("SpawnY") && has("SpawnZ")).then(|| {
        (
            (
                data.get_i32("SpawnX"),
                data.get_i32("SpawnY"),
                data.get_i32("SpawnZ"),
            ),
            data.get_i8("SpawnForced") != 0,
        )
    })
}

/// `EntityPlayer.setSpawnPoint(pos, true)` (`/spawnpoint`): respawns there as long as
/// the two blocks are free, bed or not.
pub fn set_forced_spawn_point(p: &mut super::player::Player, pos: (i32, i32, i32)) {
    set_spawn_point(p, Some(pos));
    p.data.insert("SpawnForced", krol_nbt::Tag::Byte(1));
}

/// `EntityPlayer.setSpawnPoint(pos, false)`: the bed becomes the spawn point (none
/// clears it).
fn set_spawn_point(p: &mut super::player::Player, bed: Option<(i32, i32, i32)>) {
    use krol_nbt::Tag;
    match bed {
        Some(b) => {
            p.data.insert("SpawnX", Tag::Int(b.0));
            p.data.insert("SpawnY", Tag::Int(b.1));
            p.data.insert("SpawnZ", Tag::Int(b.2));
            p.data.insert("SpawnForced", Tag::Byte(0));
        }
        None => {
            for key in ["SpawnX", "SpawnY", "SpawnZ", "SpawnForced"] {
                p.data.remove(key);
            }
        }
    }
}
