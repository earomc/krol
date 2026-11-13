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
    /// `mob.slime.big`).
    pub place: Option<&'static str>,
}

const fn sound(name: &'static str, frequency: f32) -> SoundType {
    SoundType {
        name,
        volume: 1.0,
        frequency,
        place: None,
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
};
const SLIME: SoundType = SoundType {
    name: "slime",
    volume: 1.0,
    frequency: 1.0,
    place: Some("mob.slime.big"),
};

impl SoundType {
    /// The sound played when the block is placed (`getPlaceSound`).
    pub fn place_sound(&self) -> String {
        self.place
            .map_or_else(|| format!("dig.{}", self.name), str::to_owned)
    }
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
