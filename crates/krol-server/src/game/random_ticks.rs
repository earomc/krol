//! Random block ticks and the per-chunk world updates around them (vanilla:
//! `WorldServer.updateBlocks`, `World.setActivePlayerChunksAndCheckLight`,
//! `calculateSkylightSubtracted`, `getLight(pos, true)`, and the `randomTick` /
//! `updateTick` of grass, mycelium, farmland, crops, stems, sugar cane, cactus, ice,
//! snow, redstone ore, nether wart and plants).
//!
//! TODO(M5): lightning, freezing and snowfall (biome temperatures),
//! filling cauldrons with rain, `enqueueRelightChecks`, sapling growth (trees, M6),
//! mushroom spreading, leaf decay, fire, liquids, vines, cocoa, double plants, portals,
//! tripwires.

use super::Game;
use super::ids::block;
use super::world::{LightType, World};

/// `updateLCG` step: `lcg * 3 + 1013904223`.
const LCG_MULTIPLIER: i32 = 3;
const LCG_INCREMENT: i32 = 1_013_904_223;
/// Odds per active chunk and tick: a lightning try (1 in 100000), an ice/snow try
/// (1 in 16).
const THUNDER_ODDS: i32 = 100_000;
const ICE_AND_SNOW_ODDS: i32 = 16;
/// The random light check around a player reaches 5 blocks each way.
const PLAYER_LIGHT_CHECK_RANGE: i32 = 11;
/// Light thresholds.
const GRASS_DIES_BELOW: i32 = 4;
const GRASS_SPREADS_AT: i32 = 9;
const GRASS_OPACITY_LIMIT: i32 = 2;
const CROPS_GROW_AT: i32 = 9;
const ICE_MELT_LIGHT: i32 = 11;
const SNOW_MELT_LIGHT: i32 = 11;
/// Plants grow at most to this age (crops and stems 7, nether wart 3).
const CROP_MAX_AGE: u16 = 7;
const WART_MAX_AGE: u16 = 3;
/// Nether wart grows on 1 in 10 ticks; saplings try on 1 in 7.
const WART_ODDS: i32 = 10;
/// Sugar cane and cactus grow to 3 blocks, each step after 16 ticks.
const CANE_MAX_HEIGHT: i32 = 3;
const CANE_MAX_AGE: u16 = 15;
/// Farmland: wet at 7; water counts within 4 blocks.
const FARMLAND_WET: u16 = 7;
const FARMLAND_WATER_RANGE: i32 = 4;

/// `ChunkCoordIntPair.hashCode`.
fn chunk_hash(x: i32, z: i32) -> i32 {
    let a = 1_664_525i32.wrapping_mul(x).wrapping_add(1_013_904_223);
    let b = 1_664_525i32
        .wrapping_mul(z ^ -559_038_737)
        .wrapping_add(1_013_904_223);
    a ^ b
}

/// The cave mood countdown starts below this, and restarts at `AMBIENT_MIN` plus up to
/// this after a sound.
const AMBIENT_FIRST_MAX: i32 = 12000;
const AMBIENT_MIN: i32 = 6000;
/// The countdown `krolpin` sets (far from running out during a scenario).
#[cfg_attr(not(feature = "parity"), allow(dead_code))]
pub const PINNED_AMBIENT_COUNTDOWN: i32 = 12000;
/// A mood sound needs a player within 8 blocks, but further than 2.
const MOOD_PLAYER_RANGE: f64 = 8.0;
const MOOD_MIN_DISTANCE_SQ: f64 = 4.0;

impl Game {
    /// `World.calculateSkylightSubtracted(1.0F)`: how much the sky is darkened by time
    /// and weather.
    pub(super) fn update_skylight_subtracted(&mut self) {
        let time = self.level.data.day_time();
        // WorldProvider.calculateCelestialAngle.
        let mut f = ((time % 24000) as i32 as f32 + 1.0) / 24000.0 - 0.25;
        if f < 0.0 {
            f += 1.0;
        }
        if f > 1.0 {
            f -= 1.0;
        }
        let f1 = f;
        f = 1.0 - (((f64::from(f) * std::f64::consts::PI).cos() + 1.0) / 2.0) as f32;
        let angle = f1 + (f - f1) / 3.0;
        let mut light =
            1.0 - (krol_java::math::cos(angle * std::f32::consts::PI * 2.0) * 2.0 + 0.5);
        light = light.clamp(0.0, 1.0);
        light = 1.0 - light;
        let rain = self.rain.0;
        let thunder = self.thunder.0 * rain;
        light = (f64::from(light) * (1.0 - f64::from(rain * 5.0) / 16.0)) as f32;
        light = (f64::from(light) * (1.0 - f64::from(thunder * 5.0) / 16.0)) as f32;
        light = 1.0 - light;
        self.skylight_subtracted = (light * 11.0) as i32;
    }

    /// `World.getLight(pos, false)`.
    fn light_at(&mut self, pos: (i32, i32, i32)) -> i32 {
        if pos.1 < 0 {
            return 0;
        }
        let pos = (pos.0, pos.1.min(255), pos.2);
        self.level.overworld.chunk(pos.0 >> 4, pos.2 >> 4);
        let sub = self.skylight_subtracted;
        self.level.overworld.light_subtracted(pos, sub)
    }

    /// `World.getLightFromNeighbors`: blocks such as slabs take their brightest
    /// neighbor's light.
    pub(super) fn light_from_neighbors(&mut self, pos: (i32, i32, i32)) -> i32 {
        let id = self.level.overworld.block(pos.0, pos.1, pos.2) >> 4;
        if super::light_data::uses_neighbor_brightness(id) {
            let mut light = self.light_at((pos.0, pos.1 + 1, pos.2));
            for n in [
                (pos.0 + 1, pos.1, pos.2),
                (pos.0 - 1, pos.1, pos.2),
                (pos.0, pos.1, pos.2 + 1),
                (pos.0, pos.1, pos.2 - 1),
            ] {
                light = light.max(self.light_at(n));
            }
            return light;
        }
        self.light_at(pos)
    }

    fn step_lcg(&mut self) -> i32 {
        self.update_lcg = self
            .update_lcg
            .wrapping_mul(LCG_MULTIPLIER)
            .wrapping_add(LCG_INCREMENT);
        self.update_lcg >> 2
    }

    pub(super) fn is_raining(&self) -> bool {
        self.rain.0 > 0.2
    }

    /// `World.updateBlocks` / `WorldServer.updateBlocks`: the chunks around players in
    /// `HashSet<ChunkCoordIntPair>` order, each ticked (gap lighting), with its random
    /// block ticks.
    pub(super) fn tick_active_chunks(&mut self, radius: i32) {
        let mut active: Vec<(i32, i32)> = Vec::new();
        for p in &self.players {
            let cx = krol_java::math::floor_f64(p.pos.0 / 16.0);
            let cz = krol_java::math::floor_f64(p.pos.2 / 16.0);
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    let c = (cx + dx, cz + dz);
                    if !active.contains(&c) {
                        active.push(c);
                    }
                }
            }
        }
        // World's constructor draws the countdown (krol: on the first tick).
        if self.ambient_countdown == super::AMBIENT_COUNTDOWN_UNSET {
            self.ambient_countdown = self.world_rand.next_int_bounded(AMBIENT_FIRST_MAX);
        }
        if self.ambient_countdown > 0 {
            self.ambient_countdown -= 1;
        }
        // setActivePlayerChunksAndCheckLight: a light check near a random player.
        if !self.players.is_empty() {
            let k = self.world_rand.next_int_bounded(self.players.len() as i32) as usize;
            let p = self.players[k].pos;
            let r = &mut self.world_rand;
            let x =
                krol_java::math::floor_f64(p.0) + r.next_int_bounded(PLAYER_LIGHT_CHECK_RANGE) - 5;
            let y =
                krol_java::math::floor_f64(p.1) + r.next_int_bounded(PLAYER_LIGHT_CHECK_RANGE) - 5;
            let z =
                krol_java::math::floor_f64(p.2) + r.next_int_bounded(PLAYER_LIGHT_CHECK_RANGE) - 5;
            self.level.overworld.check_light((x, y, z));
        }
        let hashes: Vec<i32> = active.iter().map(|&(x, z)| chunk_hash(x, z)).collect();
        let speed = self
            .level
            .data
            .game_rule("randomTickSpeed")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(3);
        for k in krol_java::java_hash_order(&hashes) {
            let (cx, cz) = active[k];
            self.mood_sound(cx, cz);
            self.level.overworld.tick_chunk(cx, cz);
            // TODO(M5): lightning.
            if self.world_rand.next_int_bounded(THUNDER_ODDS) == 0
                && self.is_raining()
                && self.thunder.0 > 0.9
            {
                self.step_lcg();
            }
            if self.world_rand.next_int_bounded(ICE_AND_SNOW_ODDS) == 0 {
                // TODO(M5): freezing water and snowfall need biome temperatures.
                self.step_lcg();
            }
            if speed <= 0 {
                continue;
            }
            for section in 0..16 {
                if self.level.overworld.random_tick_count(cx, cz, section) == 0 {
                    continue;
                }
                for _ in 0..speed {
                    let r = self.step_lcg();
                    let pos = (
                        cx * 16 + (r & 15),
                        section * 16 + (r >> 16 & 15),
                        cz * 16 + (r >> 8 & 15),
                    );
                    let state = self.level.overworld.block(pos.0, pos.1, pos.2);
                    if super::light_data::ticks_randomly(state >> 4) {
                        self.random_tick(pos, state);
                    }
                }
            }
        }
    }

    /// `World.playMoodSoundAndCheckLight`: once the countdown ran out, a random dark air
    /// block of the chunk near (but not next to) a player plays a cave sound.
    fn mood_sound(&mut self, cx: i32, cz: i32) {
        if self.ambient_countdown != 0 {
            return;
        }
        let r = self.step_lcg();
        let (x, z, y) = (cx * 16 + (r & 15), cz * 16 + (r >> 8 & 15), r >> 16 & 0xff);
        let w = &mut self.level.overworld;
        if w.block(x, y, z) >> 4 != block::AIR {
            return;
        }
        let light = w.light_subtracted((x, y, z), 0);
        if light > self.world_rand.next_int_bounded(8) {
            return;
        }
        if self.level.overworld.light(LightType::Sky, (x, y, z)) > 0 {
            return;
        }
        let center = (f64::from(x) + 0.5, f64::from(y) + 0.5, f64::from(z) + 0.5);
        let dist = |p: &super::player::Player| {
            let d = (p.pos.0 - center.0, p.pos.1 - center.1, p.pos.2 - center.2);
            d.0 * d.0 + d.1 * d.1 + d.2 * d.2
        };
        // getClosestPlayer(..., 8): the nearest player not spectating within 8 blocks.
        let closest = self
            .players
            .iter()
            .filter(|p| p.gamemode != 3)
            .map(dist)
            .filter(|&d| d < MOOD_PLAYER_RANGE * MOOD_PLAYER_RANGE)
            .fold(None, |best: Option<f64>, d| {
                Some(best.map_or(d, |b| b.min(d)))
            });
        if closest.is_some_and(|d| d > MOOD_MIN_DISTANCE_SQ) {
            let pitch = 0.8 + self.world_rand.next_float() * 0.2;
            self.play_sound_near_except(None, "ambient.cave.cave", center, 0.7, pitch);
            self.ambient_countdown =
                self.world_rand.next_int_bounded(AMBIENT_FIRST_MAX) + AMBIENT_MIN;
        }
    }

    /// `Block.randomTick` of the blocks krol models.
    fn random_tick(&mut self, pos: (i32, i32, i32), state: u16) {
        let (id, meta) = (state >> 4, state & 15);
        let flags_send = super::block_update::flags::SEND;
        match id {
            block::GRASS | block::MYCELIUM => self.grass_tick(pos, id),
            block::FARMLAND => self.farmland_tick(pos, state),
            block::WHEAT | block::CARROTS | block::POTATOES => {
                // BlockBush.updateTick first; vanilla quirk: the crop may still grow
                // back into the spot it just popped off from.
                self.check_plant(pos, state);
                if self.light_from_neighbors((pos.0, pos.1 + 1, pos.2)) >= CROPS_GROW_AT
                    && meta < CROP_MAX_AGE
                {
                    let chance = self.growth_chance(pos, id);
                    if self.world_rand.next_int_bounded((25.0 / chance) as i32 + 1) == 0 {
                        self.set_block_flags(pos, state + 1, flags_send);
                    }
                }
            }
            block::PUMPKIN_STEM | block::MELON_STEM => self.stem_tick(pos, state),
            // Block.randomTick runs updateTick.
            block::FIRE => self.fire_tick(pos, state),
            block::LAVA => self.lava_tick(pos),
            block::LEAVES | block::LEAVES2 => self.leaves_tick(pos, state),
            super::vines::VINE => self.vine_tick(pos, state),
            block::FLOWING_WATER | block::FLOWING_LAVA => self.flowing_liquid_tick(pos, state),
            block::REEDS => self.cane_tick(pos, state, false),
            block::CACTUS => self.cane_tick(pos, state, true),
            block::ICE => {
                // BlockIce: melts in block light above 11 minus its opacity.
                let limit = ICE_MELT_LIGHT - super::light_data::opacity(id);
                if self.level.overworld.light(LightType::Block, pos) > limit {
                    self.drop_block_items(pos, state);
                    self.set_block_notify(pos, block::FLOWING_WATER << 4);
                }
            }
            block::SNOW_LAYER | block::SNOW => {
                if self.level.overworld.light(LightType::Block, pos) > SNOW_MELT_LIGHT {
                    self.drop_block_items(pos, state);
                    self.set_block_notify(pos, 0);
                }
            }
            block::LIT_REDSTONE_ORE => self.set_block_notify(pos, block::REDSTONE_ORE << 4),
            block::NETHER_WART => {
                if meta < WART_MAX_AGE && self.world_rand.next_int_bounded(WART_ODDS) == 0 {
                    self.set_block_flags(pos, state + 1, flags_send);
                }
                let now = self.level.overworld.block(pos.0, pos.1, pos.2);
                self.check_plant(pos, now);
            }
            block::SAPLING => {
                self.check_plant(pos, state);
                // Vanilla quirk: growth still uses the state from before the bush check,
                // even if the sapling just popped off.
                self.sapling_tick(pos, state);
            }
            block::TALLGRASS | block::DEADBUSH | block::YELLOW_FLOWER | block::RED_FLOWER => {
                self.check_plant(pos, state);
            }
            super::plants::DOUBLE_PLANT => self.check_double_plant(pos, state),
            _ if super::mushrooms::is_mushroom(id) => self.mushroom_tick(pos, state),
            block::COCOA => self.cocoa_tick(pos, state),
            _ => {}
        }
    }

    /// `BlockGrass.updateTick` / `BlockMycelium.updateTick`.
    fn grass_tick(&mut self, pos: (i32, i32, i32), id: u16) {
        let up = (pos.0, pos.1 + 1, pos.2);
        let above = self.level.overworld.block(up.0, up.1, up.2) >> 4;
        if self.light_from_neighbors(up) < GRASS_DIES_BELOW
            && super::light_data::opacity(above) > GRASS_OPACITY_LIMIT
        {
            self.set_block_notify(pos, block::DIRT << 4);
            return;
        }
        if self.light_from_neighbors(up) < GRASS_SPREADS_AT {
            return;
        }
        for _ in 0..4 {
            let r = &mut self.world_rand;
            let dx = r.next_int_bounded(3) - 1;
            let dy = r.next_int_bounded(5) - 3;
            let dz = r.next_int_bounded(3) - 1;
            let t = (pos.0 + dx, pos.1 + dy, pos.2 + dz);
            let t_up = (t.0, t.1 + 1, t.2);
            let above = self.level.overworld.block(t_up.0, t_up.1, t_up.2) >> 4;
            let target = self.level.overworld.block(t.0, t.1, t.2);
            // Plain dirt only (not coarse dirt or podzol).
            if target == block::DIRT << 4
                && self.light_from_neighbors(t_up) >= GRASS_DIES_BELOW
                && super::light_data::opacity(above) <= GRASS_OPACITY_LIMIT
            {
                self.set_block_notify(t, id << 4);
            }
        }
    }

    /// `BlockFarmland.updateTick`: dries out without water or rain.
    fn farmland_tick(&mut self, pos: (i32, i32, i32), state: u16) {
        let moisture = state & 15;
        let mut water = false;
        'search: for x in -FARMLAND_WATER_RANGE..=FARMLAND_WATER_RANGE {
            for y in 0..=1 {
                for z in -FARMLAND_WATER_RANGE..=FARMLAND_WATER_RANGE {
                    let id = self.level.overworld.block(pos.0 + x, pos.1 + y, pos.2 + z) >> 4;
                    if id == block::FLOWING_WATER || id == block::WATER {
                        water = true;
                        break 'search;
                    }
                }
            }
        }
        let rain = self.rains_at((pos.0, pos.1 + 1, pos.2));
        let flags_send = super::block_update::flags::SEND;
        if !water && !rain {
            if moisture > 0 {
                self.set_block_flags(pos, state - 1, flags_send);
            } else {
                let above = self.level.overworld.block(pos.0, pos.1 + 1, pos.2) >> 4;
                let crops = matches!(
                    above,
                    block::WHEAT
                        | block::CARROTS
                        | block::POTATOES
                        | block::PUMPKIN_STEM
                        | block::MELON_STEM
                );
                if !crops {
                    self.set_block_notify(pos, block::DIRT << 4);
                }
            }
        } else if moisture < FARMLAND_WET {
            self.set_block_flags(pos, (state & !15) | FARMLAND_WET, flags_send);
        }
    }

    /// `BlockCrops.getGrowthChance`: more with wet farmland around, less in crowded
    /// rows of the same plant.
    fn growth_chance(&mut self, pos: (i32, i32, i32), id: u16) -> f32 {
        let mut chance = 1.0f32;
        for dx in -1..=1 {
            for dz in -1..=1 {
                let s = self
                    .level
                    .overworld
                    .block(pos.0 + dx, pos.1 - 1, pos.2 + dz);
                let mut v = 0.0f32;
                if s >> 4 == block::FARMLAND {
                    v = if s & 15 > 0 { 3.0 } else { 1.0 };
                }
                if dx != 0 || dz != 0 {
                    v /= 4.0;
                }
                chance += v;
            }
        }
        let is =
            |g: &mut Self, p: (i32, i32, i32)| g.level.overworld.block(p.0, p.1, p.2) >> 4 == id;
        let (n, s) = ((pos.0, pos.1, pos.2 - 1), (pos.0, pos.1, pos.2 + 1));
        let (w, e) = ((pos.0 - 1, pos.1, pos.2), (pos.0 + 1, pos.1, pos.2));
        let row_x = is(self, w) || is(self, e);
        let row_z = is(self, n) || is(self, s);
        // Halved in a crossing of rows, or with the same plant diagonally.
        let crowded = (row_x && row_z)
            || is(self, (w.0, w.1, w.2 - 1))
            || is(self, (e.0, e.1, e.2 - 1))
            || is(self, (e.0, e.1, e.2 + 1))
            || is(self, (w.0, w.1, w.2 + 1));
        if crowded {
            chance /= 2.0;
        }
        chance
    }

    /// `BlockStem.updateTick`: grows, then places its fruit beside it.
    fn stem_tick(&mut self, pos: (i32, i32, i32), state: u16) {
        let id = state >> 4;
        self.check_plant(pos, state);
        if self.light_from_neighbors((pos.0, pos.1 + 1, pos.2)) < CROPS_GROW_AT {
            return;
        }
        let chance = self.growth_chance(pos, id);
        if self.world_rand.next_int_bounded((25.0 / chance) as i32 + 1) != 0 {
            return;
        }
        if state & 15 < CROP_MAX_AGE {
            self.set_block_flags(pos, state + 1, super::block_update::flags::SEND);
            return;
        }
        let fruit = if id == block::PUMPKIN_STEM {
            block::PUMPKIN
        } else {
            block::MELON_BLOCK
        };
        let sides = [(0, -1), (1, 0), (0, 1), (-1, 0)];
        if sides.iter().any(|&(dx, dz)| {
            self.level.overworld.block(pos.0 + dx, pos.1, pos.2 + dz) >> 4 == fruit
        }) {
            return;
        }
        let (dx, dz) = sides[self.world_rand.next_int_bounded(4) as usize];
        let at = (pos.0 + dx, pos.1, pos.2 + dz);
        let below = self.level.overworld.block(at.0, at.1 - 1, at.2) >> 4;
        if self.level.overworld.block(at.0, at.1, at.2) >> 4 == block::AIR
            && matches!(below, block::FARMLAND | block::DIRT | block::GRASS)
        {
            self.set_block_notify(at, fruit << 4);
        }
    }

    /// Whether sugar cane or a cactus can stay at `pos` (`canBlockStay`).
    pub(super) fn cane_stays(&mut self, pos: (i32, i32, i32), cactus: bool) -> bool {
        cane_can_stay(&mut self.level.overworld, pos, cactus)
    }

    /// `BlockReed.updateTick` / `BlockCactus.updateTick`: grows up to 3 high, one step
    /// every 16 ticks.
    fn cane_tick(&mut self, pos: (i32, i32, i32), state: u16, cactus: bool) {
        let id = state >> 4;
        if !cactus {
            let below = self.level.overworld.block(pos.0, pos.1 - 1, pos.2) >> 4;
            if below != block::REEDS && !self.cane_stays(pos, false) {
                // checkForDrop.
                self.pop_off_block(pos, state);
                return;
            }
        }
        let up = (pos.0, pos.1 + 1, pos.2);
        if self.level.overworld.block(up.0, up.1, up.2) >> 4 != block::AIR {
            return;
        }
        let mut height = 1;
        while self.level.overworld.block(pos.0, pos.1 - height, pos.2) >> 4 == id {
            height += 1;
        }
        if height >= CANE_MAX_HEIGHT {
            return;
        }
        let age = state & 15;
        if age == CANE_MAX_AGE {
            self.set_block_notify(up, id << 4);
            self.set_block_flags(pos, state & !15, 0);
            // BlockCactus: the new block checks whether it can stay.
            if cactus && !self.cane_stays(up, true) {
                let s = self.level.overworld.block(up.0, up.1, up.2);
                self.destroy_block(up, s);
            }
        } else {
            self.set_block_flags(pos, state + 1, 0);
        }
    }

    /// `World.destroyBlock(pos, true)`: break effect, drops, air.
    pub(super) fn destroy_block(&mut self, pos: (i32, i32, i32), state: u16) {
        let state_id = i32::from(state >> 4) | i32::from(state & 15) << 12;
        self.block_effect_data(2001, pos, state_id);
        self.drop_block_items(pos, state);
        self.set_block_notify(pos, 0);
    }
}

/// Whether sugar cane or a cactus can stay at `pos` (`BlockReed.canBlockStay`,
/// `BlockCactus.canBlockStay`): a cactus needs sand or cactus below and no solid block
/// beside it; sugar cane needs cane below, or grass, dirt or sand next to water.
pub(super) fn cane_can_stay(w: &mut World, pos: (i32, i32, i32), cactus: bool) -> bool {
    let below = w.block(pos.0, pos.1 - 1, pos.2) >> 4;
    let sides = [(0, -1), (1, 0), (0, 1), (-1, 0)];
    if cactus {
        if sides
            .iter()
            .any(|&(dx, dz)| super::sign::solid_material(w.block(pos.0 + dx, pos.1, pos.2 + dz)))
        {
            return false;
        }
        below == block::CACTUS || below == block::SAND
    } else {
        below == block::REEDS
            || (matches!(below, block::GRASS | block::DIRT | block::SAND)
                && sides.iter().any(|&(dx, dz)| {
                    let id = w.block(pos.0 + dx, pos.1 - 1, pos.2 + dz) >> 4;
                    id == block::FLOWING_WATER || id == block::WATER
                }))
    }
}
