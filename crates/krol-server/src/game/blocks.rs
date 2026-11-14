//! Block properties krol models so far (vanilla: `Block` subclasses, `Block.SoundType`).
//!
//! TODO(M4/M5): grow into a full block registry (hardness, materials, shapes, behaviors).

/// A block's step sound (vanilla `Block.SoundType`): `(name, volume, frequency)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundType {
    /// Sound name used for `dig.<name>`.
    pub name: &'static str,
    /// Volume.
    pub volume: f32,
    /// Frequency.
    pub frequency: f32,
    /// Overridden place sound (glass places with `step.stone`, slime with
    /// `mob.slime.big`, anvils with `random.anvil_land`, ladders with `dig.wood`).
    pub place: Option<&'static str>,
    /// Overridden step sound (slime: `mob.slime.small`).
    pub step: Option<&'static str>,
}

const fn sound(name: &'static str, frequency: f32) -> SoundType {
    SoundType {
        name,
        volume: 1.0,
        frequency,
        place: None,
        step: None,
    }
}

const STONE: SoundType = sound("stone", 1.0);
const METAL: SoundType = sound("stone", 1.5);
const WOOD: SoundType = sound("wood", 1.0);
const GRAVEL: SoundType = sound("gravel", 1.0);
const GRASS: SoundType = sound("grass", 1.0);
const SAND: SoundType = sound("sand", 1.0);
const CLOTH: SoundType = sound("cloth", 1.0);
const SNOW: SoundType = sound("snow", 1.0);
const GLASS: SoundType = SoundType {
    name: "stone",
    volume: 1.0,
    frequency: 1.0,
    place: Some("step.stone"),
    step: None,
};
const SLIME: SoundType = SoundType {
    name: "slime",
    volume: 1.0,
    frequency: 1.0,
    place: Some("mob.slime.big"),
    step: Some("mob.slime.small"),
};
const LADDER: SoundType = SoundType {
    name: "ladder",
    volume: 1.0,
    frequency: 1.0,
    place: Some("dig.wood"),
    step: None,
};
const ANVIL: SoundType = SoundType {
    name: "anvil",
    volume: 0.3,
    frequency: 1.0,
    place: Some("random.anvil_land"),
    step: None,
};

impl SoundType {
    /// The sound played when the block is placed (`getPlaceSound`).
    pub fn place_sound(&self) -> String {
        self.place
            .map_or_else(|| format!("dig.{}", self.name), str::to_owned)
    }

    /// The sound played when an entity steps on the block (`getStepSound`).
    pub fn step_sound(&self) -> String {
        self.step
            .map_or_else(|| format!("step.{}", self.name), str::to_owned)
    }
}

/// Every block's sound type (vanilla `Block.stepSound`, from the block registry).
pub fn sound_type(id: u16) -> SoundType {
    match id {
        2
        | 6
        | 18..=19
        | 31..=32
        | 37..=40
        | 46
        | 59
        | 83
        | 106
        | 110..=111
        | 141..=142
        | 161
        | 170
        | 175 => GRASS,
        3 | 13 | 60 | 82 => GRAVEL,
        5
        | 17
        | 26
        | 47
        | 50
        | 53..=54
        | 58
        | 63..=64
        | 68..=69
        | 72
        | 75..=76
        | 85..=86
        | 91
        | 93..=94
        | 96
        | 99..=100
        | 103..=105
        | 107
        | 125..=127
        | 134..=136
        | 143
        | 146..=151
        | 162..=164
        | 176..=178
        | 183..=197 => WOOD,
        12 | 88 => SAND,
        20 | 79 | 89..=90 | 95 | 102 | 120 | 123..=124 | 160 | 169 | 174 => GLASS,
        27..=28 | 41..=42 | 52 | 57 | 66 | 71 | 101 | 133 | 152 | 154 | 157 | 167 => METAL,
        35 | 51 | 81 | 92 | 171 => CLOTH,
        65 => LADDER,
        78 | 80 => SNOW,
        145 => ANVIL,
        165 => SLIME,
        _ => STONE,
    }
}

/// Fences, walls and fence gates (`instanceof BlockFence/BlockWall/BlockFenceGate`).
pub fn is_fence_like(id: u16) -> bool {
    matches!(id, 85 | 107 | 113 | 139 | 183..=192)
}

/// How an item places as a block, for the full cube blocks krol supports so far:
/// `(block id, metadata comes from the item damage, sound)`.
///
/// Blocks whose placement depends on facing, neighbours or other state (logs, stairs,
/// slabs, torches, doors, ...) are not supported yet and are not placed.
pub fn simple_block(id: i16) -> Option<(u16, bool, SoundType)> {
    let (subtypes, sound) = match id {
        1 => (true, STONE),
        2 => (false, GRASS),
        3 => (true, GRAVEL),
        4 | 48 | 49 | 87 | 112 | 121 | 172 | 173 | 22 | 14 | 15 | 16 | 21 | 56 | 73 | 129 | 153 => {
            (false, STONE)
        }
        5 => (true, WOOD),
        47 => (false, WOOD),
        7 => (false, STONE),
        12 => (true, SAND),
        88 => (false, SAND),
        13 | 82 => (false, GRAVEL),
        19 => (true, GRASS),
        20 | 89 | 169 | 79 | 174 => (false, GLASS),
        95 => (true, GLASS),
        24 | 98 | 168 | 179 => (true, STONE),
        35 => (true, CLOTH),
        41 | 42 | 57 | 133 | 152 => (false, METAL),
        45 => (false, STONE),
        80 => (false, SNOW),
        159 => (true, STONE),
        165 => (false, SLIME),
        _ => return None,
    };
    Some((id as u16, subtypes, sound))
}

/// Blocks that can be replaced by placing into them (vanilla `Material.isReplaceable`
/// for the blocks krol knows): air, water, lava, tall grass, dead bush, fire, vines,
/// double plants.
pub fn is_replaceable(state: u16) -> bool {
    matches!(state >> 4, 0 | 8 | 9 | 10 | 11 | 31 | 32 | 51 | 106 | 175)
}
