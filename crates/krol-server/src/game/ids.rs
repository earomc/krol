//! Named block and item ids (vanilla 1.8.8 registry numbers).
//!
//! Block ids are the upper 12 bits of a block state (`state >> 4`); item ids are the
//! numbers item stacks carry. Add names here instead of writing bare numbers.

/// Block ids.
pub mod block {
    /// `sapling`.
    pub const SAPLING: u16 = 6;
    /// `sponge`.
    pub const SPONGE: u16 = 19;
    /// `chest`.
    pub const CHEST: u16 = 54;
    /// `crafting_table`.
    pub const CRAFTING_TABLE: u16 = 58;
    /// `furnace`.
    pub const FURNACE: u16 = 61;
    /// `lit_furnace`.
    pub const LIT_FURNACE: u16 = 62;
    /// `standing_sign`.
    pub const STANDING_SIGN: u16 = 63;
    /// `wall_sign`.
    pub const WALL_SIGN: u16 = 68;
    /// `cactus`.
    pub const CACTUS: u16 = 81;
    /// `wooden_slab`.
    pub const WOODEN_SLAB: u16 = 126;
    /// `trapped_chest`.
    pub const TRAPPED_CHEST: u16 = 146;
    /// `coal_block`.
    pub const COAL_BLOCK: u16 = 173;

    /// Chests of either kind.
    pub fn is_chest(id: u16) -> bool {
        id == CHEST || id == TRAPPED_CHEST
    }

    /// Furnaces, lit or not.
    pub fn is_furnace(id: u16) -> bool {
        id == FURNACE || id == LIT_FURNACE
    }

    /// Signs, standing or on a wall.
    pub fn is_sign(id: u16) -> bool {
        id == STANDING_SIGN || id == WALL_SIGN
    }
}

/// Item ids.
pub mod item {
    /// Wet sponge is `sponge` with this damage.
    pub const WET_SPONGE_DAMAGE: i16 = 1;
    /// `sapling` (the block item).
    pub const SAPLING: i16 = super::block::SAPLING as i16;
    /// `sponge` (the block item).
    pub const SPONGE: i16 = super::block::SPONGE as i16;
    /// `coal` (and charcoal, damage 1).
    pub const COAL: i16 = 263;
    /// `wooden_sword`.
    pub const WOODEN_SWORD: i16 = 268;
    /// `wooden_axe`; the wooden sword, shovel, pickaxe and axe are 268-271.
    pub const WOODEN_AXE: i16 = 271;
    /// `stick`.
    pub const STICK: i16 = 280;
    /// `wooden_hoe`.
    pub const WOODEN_HOE: i16 = 290;
    /// `sign`.
    pub const SIGN: i16 = 323;
    /// `bucket`.
    pub const BUCKET: i16 = 325;
    /// `water_bucket`.
    pub const WATER_BUCKET: i16 = 326;
    /// `lava_bucket`.
    pub const LAVA_BUCKET: i16 = 327;
    /// `milk_bucket`.
    pub const MILK_BUCKET: i16 = 335;
    /// `blaze_rod`.
    pub const BLAZE_ROD: i16 = 369;
}
