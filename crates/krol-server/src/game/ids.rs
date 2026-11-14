//! Named block and item ids (vanilla 1.8.8 registry numbers).
//!
//! Block ids are the upper 12 bits of a block state (`state >> 4`); item ids are the
//! numbers item stacks carry. Add names here instead of writing bare numbers.

/// Block ids.
pub mod block {
    /// `grass`.
    pub const GRASS: u16 = 2;
    /// `dirt`.
    pub const DIRT: u16 = 3;
    /// `sapling`.
    pub const SAPLING: u16 = 6;
    /// `sand`.
    pub const SAND: u16 = 12;
    /// `gravel`.
    pub const GRAVEL: u16 = 13;
    /// `sponge`.
    pub const SPONGE: u16 = 19;
    /// `coal_ore`.
    pub const COAL_ORE: u16 = 16;
    /// `lapis_ore`.
    pub const LAPIS_ORE: u16 = 21;
    /// `tallgrass`.
    pub const TALLGRASS: u16 = 31;
    /// `deadbush`.
    pub const DEADBUSH: u16 = 32;
    /// `yellow_flower`.
    pub const YELLOW_FLOWER: u16 = 37;
    /// `red_flower`.
    pub const RED_FLOWER: u16 = 38;
    /// `torch`.
    pub const TORCH: u16 = 50;
    /// `fire`.
    pub const FIRE: u16 = 51;
    /// `chest`.
    pub const CHEST: u16 = 54;
    /// `diamond_ore`.
    pub const DIAMOND_ORE: u16 = 56;
    /// `crafting_table`.
    pub const CRAFTING_TABLE: u16 = 58;
    /// `wheat` (the crop).
    pub const WHEAT: u16 = 59;
    /// `farmland`.
    pub const FARMLAND: u16 = 60;
    /// `furnace`.
    pub const FURNACE: u16 = 61;
    /// `lit_furnace`.
    pub const LIT_FURNACE: u16 = 62;
    /// `standing_sign`.
    pub const STANDING_SIGN: u16 = 63;
    /// `ladder`.
    pub const LADDER: u16 = 65;
    /// `wall_sign`.
    pub const WALL_SIGN: u16 = 68;
    /// `lever`.
    pub const LEVER: u16 = 69;
    /// `redstone_ore`.
    pub const REDSTONE_ORE: u16 = 73;
    /// `lit_redstone_ore`.
    pub const LIT_REDSTONE_ORE: u16 = 74;
    /// `stone_button`.
    pub const STONE_BUTTON: u16 = 77;
    /// `ice`.
    pub const ICE: u16 = 79;
    /// `cactus`.
    pub const CACTUS: u16 = 81;
    /// `soul_sand`.
    pub const SOUL_SAND: u16 = 88;
    /// `nether_wart`.
    pub const NETHER_WART: u16 = 115;
    /// `wooden_slab`.
    pub const WOODEN_SLAB: u16 = 126;
    /// `emerald_ore`.
    pub const EMERALD_ORE: u16 = 129;
    /// `carrots`.
    pub const CARROTS: u16 = 141;
    /// `potatoes`.
    pub const POTATOES: u16 = 142;
    /// `wooden_button`.
    pub const WOODEN_BUTTON: u16 = 143;
    /// `trapped_chest`.
    pub const TRAPPED_CHEST: u16 = 146;
    /// `quartz_ore`.
    pub const QUARTZ_ORE: u16 = 153;
    /// `stained_hardened_clay`.
    pub const STAINED_HARDENED_CLAY: u16 = 159;
    /// `slime`.
    pub const SLIME: u16 = 165;
    /// `hardened_clay`.
    pub const HARDENED_CLAY: u16 = 172;
    /// `coal_block`.
    pub const COAL_BLOCK: u16 = 173;
    /// `packed_ice`.
    pub const PACKED_ICE: u16 = 174;

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
