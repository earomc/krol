//! Survival digging (vanilla: `ItemInWorldManager`, `Block.getPlayerRelativeBlockHardness`,
//! `EntityPlayer.getToolDigEfficiency`, `ItemTool`/`ItemPickaxe`/`ItemAxe`/`ItemSpade`/
//! `ItemSword`/`ItemShears`, `Block.harvestBlock`, `StatList` mining statistics).
//!
//! The client sends "start digging" and "stop digging"; the server tracks how many ticks
//! passed in between. Stopping counts if the block would be at least 70% broken by
//! then; otherwise the server keeps counting and breaks the block itself once it is
//! fully broken. Other players see the progress as block break animations.
//!
//! TODO(M5): adventure mode `CanDestroy` tags.

use krol_protocol::packets::play::ClientboundPlay;

use super::Game;
use super::block_data::{self, Material};

/// `snowball`.
const SNOWBALL: i16 = 332;

/// Per-player digging state (`ItemInWorldManager` fields).
#[derive(Debug, Default, Clone)]
pub struct DigState {
    /// Ticks counted (`curblockDamage`).
    pub ticks: i32,
    destroying: bool,
    initial: i32,
    pos: (i32, i32, i32),
    finish_received: bool,
    finish_pos: (i32, i32, i32),
    finish_initial: i32,
    progress_sent: i32,
}

impl DigState {
    /// A fresh state (`durabilityRemainingOnBlock` starts at -1).
    pub fn new() -> Self {
        Self {
            progress_sent: -1,
            ..Self::default()
        }
    }
}

/// A tool's kind and material, from the held item id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Pickaxe(u8),
    Axe(u8),
    Shovel(u8),
    Sword,
    Shears,
}

/// Tool materials: (harvest level, efficiency) for wood, stone, iron, diamond, gold.
const TOOL_MATERIALS: [(u8, f32); 5] = [(0, 2.0), (1, 4.0), (2, 6.0), (3, 8.0), (0, 12.0)];

fn tool(id: i16) -> Option<Tool> {
    Some(match id {
        270 => Tool::Pickaxe(0),
        274 => Tool::Pickaxe(1),
        257 => Tool::Pickaxe(2),
        278 => Tool::Pickaxe(3),
        285 => Tool::Pickaxe(4),
        271 => Tool::Axe(0),
        275 => Tool::Axe(1),
        258 => Tool::Axe(2),
        279 => Tool::Axe(3),
        286 => Tool::Axe(4),
        269 => Tool::Shovel(0),
        273 => Tool::Shovel(1),
        256 => Tool::Shovel(2),
        277 => Tool::Shovel(3),
        284 => Tool::Shovel(4),
        268 | 272 | 267 | 276 | 283 => Tool::Sword,
        359 => Tool::Shears,
        _ => return None,
    })
}

/// Blocks a pickaxe is fast on regardless of material (`ItemPickaxe.EFFECTIVE_ON`).
fn pickaxe_effective(b: u16) -> bool {
    matches!(
        b,
        157 | 16
            | 4
            | 28
            | 57
            | 56
            | 43
            | 27
            | 41
            | 14
            | 79
            | 42
            | 15
            | 22
            | 21
            | 74
            | 48
            | 87
            | 174
            | 66
            | 73
            | 24
            | 179
            | 1
            | 44
    )
}

/// `ItemAxe.EFFECTIVE_ON`.
fn axe_effective(b: u16) -> bool {
    matches!(b, 5 | 47 | 17 | 162 | 54 | 86 | 91 | 103 | 65)
}

/// `ItemSpade.EFFECTIVE_ON`.
fn shovel_effective(b: u16) -> bool {
    matches!(b, 82 | 3 | 60 | 2 | 13 | 110 | 12 | 80 | 78 | 88)
}

/// How fast the held item breaks the block (`getStrVsBlock`).
fn strength(held: Option<i16>, b: u16) -> f32 {
    let m = block_data::material(b);
    match held.and_then(tool) {
        Some(Tool::Pickaxe(t)) => {
            if matches!(m, Material::Iron | Material::Anvil | Material::Rock)
                || pickaxe_effective(b)
            {
                TOOL_MATERIALS[usize::from(t)].1
            } else {
                1.0
            }
        }
        Some(Tool::Axe(t)) => {
            if matches!(m, Material::Wood | Material::Plants | Material::Vine) || axe_effective(b) {
                TOOL_MATERIALS[usize::from(t)].1
            } else {
                1.0
            }
        }
        Some(Tool::Shovel(t)) => {
            if shovel_effective(b) {
                TOOL_MATERIALS[usize::from(t)].1
            } else {
                1.0
            }
        }
        Some(Tool::Sword) => {
            if b == 30 {
                15.0
            } else if matches!(
                m,
                Material::Plants
                    | Material::Vine
                    | Material::Coral
                    | Material::Leaves
                    | Material::Gourd
            ) {
                1.5
            } else {
                1.0
            }
        }
        Some(Tool::Shears) => {
            if b == 30 || m == Material::Leaves {
                15.0
            } else if b == 35 {
                5.0
            } else {
                1.0
            }
        }
        None => 1.0,
    }
}

/// Whether the held item lets the player harvest the block (`canHeldItemHarvest`).
pub fn can_harvest(held: Option<i16>, b: u16) -> bool {
    let m = block_data::material(b);
    if m.tool_not_required() {
        return true;
    }
    match held.and_then(tool) {
        Some(Tool::Pickaxe(t)) => {
            let level = TOOL_MATERIALS[usize::from(t)].0;
            match b {
                49 => level == 3,
                57 | 56 | 129 | 133 | 41 | 14 | 73 | 74 => level >= 2,
                42 | 15 | 22 | 21 => level >= 1,
                _ => matches!(m, Material::Rock | Material::Iron | Material::Anvil),
            }
        }
        Some(Tool::Shovel(_)) => b == 78 || b == 80,
        Some(Tool::Sword) => b == 30,
        Some(Tool::Shears) => matches!(b, 30 | 55 | 132),
        _ => false,
    }
}

/// What speeds up or slows down a player's digging besides the tool itself
/// (`EntityPlayer.getToolDigEfficiency`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DigSpeed {
    /// The held item's efficiency level.
    pub efficiency: i32,
    /// The haste amplifier, if active.
    pub haste: Option<u8>,
    /// The mining fatigue amplifier, if active.
    pub fatigue: Option<u8>,
    /// Eyes in water without aqua affinity on the armor.
    pub under_water: bool,
    /// Standing on the ground.
    pub on_ground: bool,
}

impl DigSpeed {
    /// Plain digging on the ground: no enchantments, effects or water.
    #[cfg(test)]
    pub const PLAIN: Self = Self {
        efficiency: 0,
        haste: None,
        fatigue: None,
        under_water: false,
        on_ground: true,
    };
}

/// Haste speeds digging up by this much per level.
const HASTE_PER_LEVEL: f32 = 0.2;
/// Mining fatigue's factor by amplifier (the last one for 3 and above).
const FATIGUE_FACTORS: [f32; 4] = [0.3, 0.09, 0.0027, 8.1E-4];
/// Digging under water or in the air is this many times slower.
const WATER_OR_AIR_SLOWDOWN: f32 = 5.0;

/// `EntityPlayer.getToolDigEfficiency`.
fn dig_efficiency(held: Option<i16>, b: u16, speed: DigSpeed) -> f32 {
    let mut f = strength(held, b);
    // Vanilla: efficiency only helps where the tool is already faster than a hand.
    if f > 1.0 && speed.efficiency > 0 && held.is_some() {
        f += (speed.efficiency * speed.efficiency + 1) as f32;
    }
    if let Some(amp) = speed.haste {
        f *= 1.0 + (i32::from(amp) + 1) as f32 * HASTE_PER_LEVEL;
    }
    if let Some(amp) = speed.fatigue {
        f *= FATIGUE_FACTORS[usize::from(amp).min(FATIGUE_FACTORS.len() - 1)];
    }
    if speed.under_water {
        f /= WATER_OR_AIR_SLOWDOWN;
    }
    if !speed.on_ground {
        f /= WATER_OR_AIR_SLOWDOWN;
    }
    f
}

/// The share of the block broken per tick (`getPlayerRelativeBlockHardness`).
pub fn relative_hardness(held: Option<i16>, b: u16, speed: DigSpeed) -> f32 {
    let hardness = block_data::hardness(b);
    if hardness < 0.0 {
        return 0.0;
    }
    let efficiency = dig_efficiency(held, b, speed);
    if can_harvest(held, b) {
        efficiency / hardness / 30.0
    } else {
        efficiency / hardness / 100.0
    }
}

/// The statistic counted for mining a block (`StatList.mineBlockStatArray`): blocks
/// with an item and enabled statistics count themselves; some blocks count as another
/// (`mergeStatBases`: grass and farmland count as dirt, lit and double variants as the
/// plain ones).
pub fn mine_stat(b: u16) -> Option<String> {
    const DISABLED: [u16; 32] = [
        7, 8, 9, 10, 11, 26, 51, 52, 55, 59, 63, 64, 68, 71, 83, 92, 93, 94, 96, 141, 142, 149,
        150, 166, 167, 176, 177, 193, 194, 195, 196, 197,
    ];
    merged_stat("stat.mineBlock.", b, |id| DISABLED.contains(&id))
}

/// Shovels, pickaxes and axes (`ItemTool`).
fn is_tool(id: i16) -> bool {
    matches!(id, 256..=258 | 269..=271 | 273..=275 | 277..=279 | 284..=286)
}

/// Swords (`ItemSword`).
fn is_sword(id: i16) -> bool {
    matches!(id, 267 | 268 | 272 | 276 | 283)
}

/// What shears wear on (`ItemShears.onBlockDestroyed`): leaves, cobwebs, tall grass,
/// vines, tripwire and wool.
fn shears_cut(b: u16) -> bool {
    super::block_data::material(b) == super::block_data::Material::Leaves
        || matches!(b, 30 | 31 | 106 | 132 | 35)
}

/// The statistic counted for using an item (`StatList.objectUseStats`, with the same
/// block merges as mining: placing grass counts as dirt, ...).
pub fn use_stat(item: i16) -> Option<String> {
    if item >= 256 {
        let name = krol_world::items::name_for_id(item)?;
        return Some(format!("stat.useItem.{}", name.replace(':', ".")));
    }
    merged_stat("stat.useItem.", item as u16, |_| false)
}

/// A per-block statistic after `StatList.replaceAllSimilarBlocks`.
fn merged_stat(prefix: &str, b: u16, disabled: impl Fn(u16) -> bool) -> Option<String> {
    let own = |id: u16| -> Option<String> {
        if disabled(id) {
            return None;
        }
        let name = krol_world::items::name_for_id(id as i16)?;
        Some(format!("{prefix}{}", name.replace(':', ".")))
    };
    const MERGES: [(u16, u16); 14] = [
        (9, 8),
        (11, 10),
        (91, 86),
        (62, 61),
        (74, 73),
        (94, 93),
        (150, 149),
        (76, 75),
        (124, 123),
        (43, 44),
        (125, 126),
        (181, 182),
        (2, 3),
        (60, 3),
    ];
    let mut stat = own(b);
    for (a, other) in MERGES {
        if b == a {
            stat = if own(a).is_some() && own(other).is_none() {
                own(a)
            } else {
                own(other)
            };
        } else if b == other && own(a).is_some() && own(other).is_none() {
            stat = own(a);
        }
    }
    stat
}

impl Game {
    fn held_id(&self, i: usize) -> Option<i16> {
        self.players[i].held().map(|s| s.id)
    }

    fn relative_hardness_for(&mut self, i: usize, pos: (i32, i32, i32)) -> f32 {
        let b = self.level.overworld.block(pos.0, pos.1, pos.2) >> 4;
        let under_water = self.eyes_in_water(i)
            && super::item_data::armor_enchantment(
                &self.players[i].inventory,
                super::item_data::AQUA_AFFINITY,
            ) == 0;
        let p = &self.players[i];
        let effect = |id| p.effects.iter().find(|e| e.id == id).map(|e| e.amplifier);
        let speed = DigSpeed {
            efficiency: super::item_data::enchantment_level(p.held(), super::item_data::EFFICIENCY),
            haste: effect(super::potions::effect::HASTE),
            fatigue: effect(super::potions::effect::MINING_FATIGUE),
            under_water,
            on_ground: p.on_ground,
        };
        relative_hardness(self.held_id(i), b, speed)
    }

    /// Sends a block break animation to the other players within 32 blocks
    /// (`WorldManager.sendBlockBreakProgress`).
    fn send_break_progress(&self, i: usize, pos: (i32, i32, i32), stage: i32) {
        let p = &self.players[i];
        let packet = ClientboundPlay::BlockBreakAnimation {
            entity_id: p.entity_id,
            pos,
            stage: stage as i8,
        };
        for other in &self.players {
            if other.entity_id == p.entity_id {
                continue;
            }
            let d = (
                f64::from(pos.0) - other.pos.0,
                f64::from(pos.1) - other.pos.1,
                f64::from(pos.2) - other.pos.2,
            );
            if d.0 * d.0 + d.1 * d.1 + d.2 * d.2 < 1024.0 {
                other.send(packet.clone());
            }
        }
    }

    /// Vanilla `ItemInWorldManager.onBlockClicked` for survival and adventure players.
    pub(super) fn start_digging(&mut self, i: usize, pos: (i32, i32, i32), face: i8) {
        let gamemode = self.players[i].gamemode;
        if gamemode == 3 {
            return;
        }
        if gamemode == 2 {
            // TODO(M5): CanDestroy tags; without them adventure players cannot dig.
            return;
        }
        // The clicked block is read before fire on its face is punched out.
        let b = self.level.overworld.block(pos.0, pos.1, pos.2) >> 4;
        self.extinguish_fire(pos, face);
        let dig = &mut self.players[i].dig;
        dig.initial = dig.ticks;
        let mut hardness = 1.0;
        if b != 0 {
            // The hardness is the clicked block's, even if the click removes it (the
            // last slice of a cake).
            hardness = self.relative_hardness_for(i, pos);
            // Block.onBlockClicked. TODO(M5): dragon eggs.
            if matches!(
                b,
                super::ids::block::REDSTONE_ORE | super::ids::block::LIT_REDSTONE_ORE
            ) {
                let state = self.level.overworld.block(pos.0, pos.1, pos.2);
                self.activate_redstone_ore(pos, state);
            }
            if b == super::ids::block::NOTEBLOCK {
                self.click_note_block(i, pos);
            }
            if b == super::farming::CAKE {
                // BlockCake.onBlockClicked eats a slice too.
                self.eat_cake(i, pos);
            }
        }
        if b != 0 && hardness >= 1.0 {
            self.harvest_survival(i, pos);
        } else {
            let stage = (hardness * 10.0) as i32;
            let dig = &mut self.players[i].dig;
            dig.destroying = true;
            dig.pos = pos;
            dig.progress_sent = stage;
            self.send_break_progress(i, pos, stage);
        }
    }

    /// Vanilla `ItemInWorldManager.blockRemoving` ("stop digging").
    pub(super) fn stop_digging(&mut self, i: usize, pos: (i32, i32, i32)) {
        let dig = &self.players[i].dig;
        if pos != dig.pos {
            return;
        }
        let elapsed = dig.ticks - dig.initial;
        let b = self.level.overworld.block(pos.0, pos.1, pos.2) >> 4;
        if b == 0 {
            return;
        }
        let progress = self.relative_hardness_for(i, pos) * (elapsed + 1) as f32;
        if progress >= 0.7 {
            self.players[i].dig.destroying = false;
            self.send_break_progress(i, pos, -1);
            self.harvest_survival(i, pos);
        } else if !self.players[i].dig.finish_received {
            let dig = &mut self.players[i].dig;
            dig.destroying = false;
            dig.finish_received = true;
            dig.finish_pos = pos;
            dig.finish_initial = dig.initial;
        }
    }

    /// Vanilla `ItemInWorldManager.cancelDestroyingBlock`.
    pub(super) fn cancel_digging(&mut self, i: usize) {
        let dig = &mut self.players[i].dig;
        dig.destroying = false;
        let pos = dig.pos;
        self.send_break_progress(i, pos, -1);
    }

    /// Vanilla `ItemInWorldManager.updateBlockRemoving`, run in the player's tick.
    pub(super) fn tick_digging(&mut self, i: usize) {
        self.players[i].dig.ticks += 1;
        let dig = self.players[i].dig.clone();
        if dig.finish_received {
            let b =
                self.level
                    .overworld
                    .block(dig.finish_pos.0, dig.finish_pos.1, dig.finish_pos.2)
                    >> 4;
            if b == 0 {
                self.players[i].dig.finish_received = false;
                return;
            }
            let progress = self.relative_hardness_for(i, dig.finish_pos)
                * (dig.ticks - dig.finish_initial + 1) as f32;
            let stage = (progress * 10.0) as i32;
            if stage != dig.progress_sent {
                self.send_break_progress(i, dig.finish_pos, stage);
                self.players[i].dig.progress_sent = stage;
            }
            if progress >= 1.0 {
                self.players[i].dig.finish_received = false;
                self.harvest_survival(i, dig.finish_pos);
            }
        } else if dig.destroying {
            let b = self.level.overworld.block(dig.pos.0, dig.pos.1, dig.pos.2) >> 4;
            if b == 0 {
                self.send_break_progress(i, dig.pos, -1);
                let d = &mut self.players[i].dig;
                d.progress_sent = -1;
                d.destroying = false;
                return;
            }
            let progress =
                self.relative_hardness_for(i, dig.pos) * (dig.ticks - dig.initial + 1) as f32;
            let stage = (progress * 10.0) as i32;
            if stage != dig.progress_sent {
                self.send_break_progress(i, dig.pos, stage);
                self.players[i].dig.progress_sent = stage;
            }
        }
    }

    /// Vanilla `ItemInWorldManager.tryHarvestBlock` for survival players.
    fn harvest_survival(&mut self, i: usize, pos: (i32, i32, i32)) {
        let state = self.level.overworld.block(pos.0, pos.1, pos.2);
        let b = state >> 4;
        self.break_effect(i, pos, state);
        // tryHarvestBlock takes the tile entity before the block goes.
        let banner_te = super::banner::is_banner(b)
            .then(|| self.level.overworld.tile_entity(pos).cloned())
            .flatten();
        // TODO(M5): onBlockHarvested / onBlockDestroyedByPlayer (doors, beds).
        self.drop_container_contents(pos);
        self.set_block_notify(pos, 0);
        self.destroyed_by_player(pos, state);
        let harvest = can_harvest(self.held_id(i), b);
        self.held_block_destroyed(i, b);
        // Plants and leaves cut with shears drop themselves and cost no exhaustion
        // (BlockTallGrass, BlockDeadBush, BlockLeaves, BlockVine harvestBlock).
        // TODO(M5): double plants.
        let shears = self.held_id(i) == Some(super::ids::item::SHEARS);
        if harvest && shears && matches!(b, 31 | 32 | 18 | 161 | 106) {
            if let Some(stat) = mine_stat(b) {
                self.add_stat(i, &stat, 1);
            }
            let damage = match b {
                31 => state & 15,
                18 | 161 => state & 3,
                _ => 0,
            };
            let stack = krol_protocol::packets::play::ItemStack {
                id: b as i16,
                count: 1,
                damage: damage as i16,
                nbt: None,
            };
            self.spawn_as_entity(pos, stack);
            return;
        }
        // BlockSnow.harvestBlock: one stack of a snowball per layer plus one, without
        // exhaustion.
        // Vanilla quirk: `layers + 1` where the metadata is already `layers - 1`.
        if harvest && b == super::ids::block::SNOW_LAYER {
            if let Some(stat) = mine_stat(b) {
                self.add_stat(i, &stat, 1);
            }
            let stack = krol_protocol::packets::play::ItemStack {
                id: SNOWBALL,
                count: (state & 7) as i8 + 2,
                damage: 0,
                nbt: None,
            };
            self.spawn_as_entity(pos, stack);
            return;
        }
        if harvest && let Some(te) = &banner_te {
            self.harvest_banner(pos, te);
            return;
        }
        if harvest {
            // Block.harvestBlock.
            if let Some(stat) = mine_stat(b) {
                self.add_stat(i, &stat, 1);
            }
            self.add_exhaustion(i, 0.025);
            let silk = self.harvest_drops(i, pos, state);
            // BlockIce.harvestBlock: without silk touch it melts into water when
            // something solid or liquid is below.
            if b == super::ids::block::ICE && !silk {
                let below = self.level.overworld.block(pos.0, pos.1 - 1, pos.2) >> 4;
                let m = super::block_data::material(below);
                if m.blocks_movement() || m.is_liquid() {
                    self.set_block_notify(pos, super::ids::block::FLOWING_WATER << 4);
                }
            }
        }
    }

    /// `ItemStack.onBlockDestroyed` for the held item: tools wear by 1 and swords by 2
    /// on blocks that are not instantly broken, shears by 1 on what they cut; a used
    /// item counts its use statistic, and a broken one leaves the hand.
    fn held_block_destroyed(&mut self, i: usize, b: u16) {
        let Some(id) = self.held_id(i) else {
            return;
        };
        let solid = super::block_data::hardness(b) != 0.0;
        let wear = match id {
            _ if is_tool(id) => Some(if solid { 1 } else { 0 }),
            _ if is_sword(id) => Some(if solid { 2 } else { 0 }),
            super::ids::item::SHEARS if shears_cut(b) => Some(1),
            _ => None,
        };
        if let Some(wear) = wear {
            if wear > 0 {
                self.damage_held_item(i, wear);
            }
            self.count_item_use(i);
        }
        let p = &mut self.players[i];
        let slot = 36 + p.selected;
        if p.inventory[slot].as_ref().is_some_and(|s| s.count == 0) {
            p.inventory[slot] = None;
        }
    }

    /// `playAuxSFXAtEntity(player, 2001, ...)`: the break effect to everyone else within
    /// 64 blocks.
    pub(super) fn break_effect(&self, i: usize, pos: (i32, i32, i32), state: u16) {
        let state_id = i32::from(state >> 4) + (i32::from(state & 15) << 12);
        let effect = ClientboundPlay::Effect {
            id: 2001,
            pos,
            data: state_id,
            disable_relative_volume: false,
        };
        let conn = self.players[i].conn_id;
        for p in &self.players {
            let (dx, dy, dz) = (
                f64::from(pos.0) - p.pos.0,
                f64::from(pos.1) - p.pos.1,
                f64::from(pos.2) - p.pos.2,
            );
            if p.conn_id != conn && dx * dx + dy * dy + dz * dz < 64.0 * 64.0 {
                p.send(effect.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dig_speed_modifiers() {
        let plain = DigSpeed::PLAIN;
        // Efficiency V on a diamond pickaxe against stone: 8 + 26.
        let eff = DigSpeed {
            efficiency: 5,
            ..plain
        };
        assert_eq!(dig_efficiency(Some(278), 1, eff), 34.0);
        // Not where the tool is no better than a hand.
        assert_eq!(dig_efficiency(Some(278), 3, eff), 1.0);
        let haste = DigSpeed {
            haste: Some(1),
            ..eff
        };
        assert!(relative_hardness(Some(278), 1, haste) >= 1.0);
        assert!(relative_hardness(Some(278), 1, eff) < 1.0);
        for (amp, factor) in [
            (0, 0.3f32),
            (1, 0.09),
            (2, 0.0027),
            (3, 8.1E-4),
            (9, 8.1E-4),
        ] {
            let tired = DigSpeed {
                fatigue: Some(amp),
                ..plain
            };
            assert_eq!(dig_efficiency(None, 1, tired), factor);
        }
        // Under water (no aqua affinity) and in the air: each five times slower.
        let wet = DigSpeed {
            under_water: true,
            on_ground: false,
            ..plain
        };
        assert_eq!(dig_efficiency(None, 1, wet), 1.0 / 5.0 / 5.0);
    }

    #[test]
    fn dig_rules() {
        // Stone by hand: not harvestable, 1/1.5/100 per tick.
        assert!(!can_harvest(None, 1));
        assert_eq!(
            relative_hardness(None, 1, DigSpeed::PLAIN),
            1.0 / 1.5 / 100.0
        );
        // Wooden pickaxe: 2/1.5/30.
        assert_eq!(
            relative_hardness(Some(270), 1, DigSpeed::PLAIN),
            2.0 / 1.5 / 30.0
        );
        // Dirt by hand and with a shovel.
        assert_eq!(
            relative_hardness(None, 3, DigSpeed::PLAIN),
            1.0 / 0.5 / 30.0
        );
        assert_eq!(
            relative_hardness(Some(277), 3, DigSpeed::PLAIN),
            8.0 / 0.5 / 30.0
        );
        // Obsidian needs diamond.
        assert!(!can_harvest(Some(257), 49));
        assert!(can_harvest(Some(278), 49));
        // Instant blocks and bedrock.
        assert!(relative_hardness(None, 31, DigSpeed::PLAIN) >= 1.0);
        assert_eq!(relative_hardness(Some(278), 7, DigSpeed::PLAIN), 0.0);
    }

    #[test]
    fn mining_stats() {
        assert_eq!(
            mine_stat(1).as_deref(),
            Some("stat.mineBlock.minecraft.stone")
        );
        assert_eq!(
            mine_stat(2).as_deref(),
            Some("stat.mineBlock.minecraft.dirt")
        );
        assert_eq!(
            mine_stat(43).as_deref(),
            Some("stat.mineBlock.minecraft.stone_slab")
        );
        assert_eq!(
            mine_stat(74).as_deref(),
            Some("stat.mineBlock.minecraft.redstone_ore")
        );
        assert_eq!(mine_stat(59), None);
    }
}
