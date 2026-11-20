//! Named block and item ids (vanilla 1.8.8 registry numbers).
//!
//! Block ids are the upper 12 bits of a block state (`state >> 4`); item ids are the
//! numbers item stacks carry. Add names here instead of writing bare numbers.

/// Block ids.
pub mod block {
    /// `air`.
    pub const AIR: u16 = 0;
    /// `stone`.
    pub const STONE: u16 = 1;
    /// `grass`.
    pub const GRASS: u16 = 2;
    /// `cobblestone`.
    pub const COBBLESTONE: u16 = 4;
    /// `flowing_water`.
    pub const FLOWING_WATER: u16 = 8;
    /// `water` (still).
    pub const WATER: u16 = 9;
    /// `flowing_lava`.
    pub const FLOWING_LAVA: u16 = 10;
    /// `lava` (still).
    pub const LAVA: u16 = 11;
    /// `dirt`.
    pub const DIRT: u16 = 3;
    /// `sapling`.
    pub const SAPLING: u16 = 6;
    /// `dispenser`.
    pub const DISPENSER: u16 = 23;
    /// `noteblock`.
    pub const NOTEBLOCK: u16 = 25;
    /// `bed`.
    pub const BED: u16 = 26;
    /// `golden_rail` (powered rail).
    pub const GOLDEN_RAIL: u16 = 27;
    /// `detector_rail`.
    pub const DETECTOR_RAIL: u16 = 28;
    /// `sticky_piston`.
    pub const STICKY_PISTON: u16 = 29;
    /// `piston`.
    pub const PISTON: u16 = 33;
    /// `piston_head`.
    pub const PISTON_HEAD: u16 = 34;
    /// `barrier`.
    pub const BARRIER: u16 = 166;
    /// `piston_extension` (a block being moved by a piston).
    pub const PISTON_EXTENSION: u16 = 36;
    /// `sand`.
    pub const SAND: u16 = 12;
    /// `gravel`.
    pub const GRAVEL: u16 = 13;
    /// `sponge`.
    pub const SPONGE: u16 = 19;
    /// `coal_ore`.
    pub const COAL_ORE: u16 = 16;
    /// `brown_mushroom`.
    pub const BROWN_MUSHROOM: u16 = 39;
    /// `red_mushroom`.
    pub const RED_MUSHROOM: u16 = 40;
    /// `stone_slab`.
    pub const STONE_SLAB: u16 = 44;
    /// `double_stone_slab`.
    pub const DOUBLE_STONE_SLAB: u16 = 43;
    /// `double_wooden_slab`.
    pub const DOUBLE_WOODEN_SLAB: u16 = 125;
    /// `double_stone_slab2`.
    pub const DOUBLE_STONE_SLAB2: u16 = 181;
    /// `carpet`.
    pub const CARPET: u16 = 171;
    /// `stone_slab2` (red sandstone).
    pub const STONE_SLAB2: u16 = 182;
    /// `log` (oak, spruce, birch, jungle).
    pub const LOG: u16 = 17;
    /// `leaves`.
    pub const LEAVES: u16 = 18;
    /// `lapis_ore`.
    pub const LAPIS_ORE: u16 = 21;
    /// `wool` (the color in the metadata).
    pub const WOOL: u16 = 35;
    /// `tallgrass`.
    pub const TALLGRASS: u16 = 31;
    /// `deadbush`.
    pub const DEADBUSH: u16 = 32;
    /// `yellow_flower`.
    pub const YELLOW_FLOWER: u16 = 37;
    /// `red_flower`.
    pub const RED_FLOWER: u16 = 38;
    /// `cocoa`.
    pub const COCOA: u16 = 127;
    /// `torch`.
    pub const TORCH: u16 = 50;
    /// `obsidian`.
    pub const OBSIDIAN: u16 = 49;
    /// `fire`.
    pub const FIRE: u16 = 51;
    /// `chest`.
    pub const CHEST: u16 = 54;
    /// `redstone_wire`.
    pub const REDSTONE_WIRE: u16 = 55;
    /// `diamond_ore`.
    pub const DIAMOND_ORE: u16 = 56;
    /// `crafting_table`.
    pub const CRAFTING_TABLE: u16 = 58;
    /// `wooden_door` (oak).
    pub const WOODEN_DOOR: u16 = 64;
    /// `iron_door`.
    pub const IRON_DOOR: u16 = 71;
    /// `trapdoor` (wooden).
    pub const TRAPDOOR: u16 = 96;
    /// `fence` (oak).
    pub const OAK_FENCE: u16 = 85;
    /// `nether_brick_fence`.
    pub const NETHER_BRICK_FENCE: u16 = 113;
    /// `spruce_fence`; birch, jungle, dark oak and acacia fences follow (188-192).
    pub const SPRUCE_FENCE: u16 = 188;
    /// `acacia_fence`.
    pub const ACACIA_FENCE: u16 = 192;
    /// `fence_gate` (oak).
    pub const OAK_FENCE_GATE: u16 = 107;
    /// `iron_trapdoor`.
    pub const IRON_TRAPDOOR: u16 = 167;
    /// `spruce_fence_gate`; birch, jungle, dark oak and acacia gates follow (183-187).
    pub const SPRUCE_FENCE_GATE: u16 = 183;
    /// `acacia_fence_gate`.
    pub const ACACIA_FENCE_GATE: u16 = 187;
    /// `spruce_door`; birch, jungle, acacia and dark oak doors follow (193-197).
    pub const SPRUCE_DOOR: u16 = 193;
    /// `dark_oak_door`.
    pub const DARK_OAK_DOOR: u16 = 197;
    /// `mob_spawner`.
    pub const MOB_SPAWNER: u16 = 52;
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
    /// `standing_banner`.
    pub const STANDING_BANNER: u16 = 176;
    /// `wall_banner`.
    pub const WALL_BANNER: u16 = 177;
    /// `ladder`.
    pub const LADDER: u16 = 65;
    /// `vine`.
    pub const VINE: u16 = 106;
    /// `rail`.
    pub const RAIL: u16 = 66;
    /// `wall_sign`.
    pub const WALL_SIGN: u16 = 68;
    /// `lever`.
    pub const LEVER: u16 = 69;
    /// `stone_pressure_plate`.
    pub const STONE_PRESSURE_PLATE: u16 = 70;
    /// `wooden_pressure_plate`.
    pub const WOODEN_PRESSURE_PLATE: u16 = 72;
    /// `redstone_ore`.
    pub const REDSTONE_ORE: u16 = 73;
    /// `lit_redstone_ore`.
    pub const LIT_REDSTONE_ORE: u16 = 74;
    /// `unlit_redstone_torch`.
    pub const UNLIT_REDSTONE_TORCH: u16 = 75;
    /// `redstone_torch` (lit).
    pub const REDSTONE_TORCH: u16 = 76;
    /// `stone_button`.
    pub const STONE_BUTTON: u16 = 77;
    /// `snow_layer`.
    pub const SNOW_LAYER: u16 = 78;
    /// `planks`.
    pub const PLANKS: u16 = 5;
    /// `waterlily` (lily pad).
    pub const WATERLILY: u16 = 111;
    /// `ice`.
    pub const ICE: u16 = 79;
    /// `snow` (the block).
    pub const SNOW: u16 = 80;
    /// `cactus`.
    pub const CACTUS: u16 = 81;
    /// `reeds` (sugar cane).
    pub const REEDS: u16 = 83;
    /// `pumpkin`.
    pub const PUMPKIN: u16 = 86;
    /// `lit_pumpkin` (jack o'lantern).
    pub const LIT_PUMPKIN: u16 = 91;
    /// `soul_sand`.
    pub const SOUL_SAND: u16 = 88;
    /// `web` (cobweb).
    pub const WEB: u16 = 30;
    /// `melon_block`.
    pub const MELON_BLOCK: u16 = 103;
    /// `pumpkin_stem`.
    pub const PUMPKIN_STEM: u16 = 104;
    /// `melon_stem`.
    pub const MELON_STEM: u16 = 105;
    /// `mycelium`.
    pub const MYCELIUM: u16 = 110;
    /// `glowstone`.
    pub const GLOWSTONE: u16 = 89;
    /// `portal` (nether portal).
    pub const PORTAL: u16 = 90;
    /// `unpowered_repeater`.
    pub const UNPOWERED_REPEATER: u16 = 93;
    /// `powered_repeater`.
    pub const POWERED_REPEATER: u16 = 94;
    /// `nether_wart`.
    pub const NETHER_WART: u16 = 115;
    /// `redstone_lamp` (off).
    pub const REDSTONE_LAMP: u16 = 123;
    /// `lit_redstone_lamp`.
    pub const LIT_REDSTONE_LAMP: u16 = 124;
    /// `wooden_slab`.
    pub const WOODEN_SLAB: u16 = 126;
    /// `emerald_ore`.
    pub const EMERALD_ORE: u16 = 129;
    /// `tripwire_hook`.
    pub const TRIPWIRE_HOOK: u16 = 131;
    /// `carrots`.
    pub const CARROTS: u16 = 141;
    /// `potatoes`.
    pub const POTATOES: u16 = 142;
    /// `wooden_button`.
    pub const WOODEN_BUTTON: u16 = 143;
    /// `trapped_chest`.
    pub const TRAPPED_CHEST: u16 = 146;
    /// `anvil`.
    pub const ANVIL: u16 = 145;
    /// `enchanting_table`.
    pub const ENCHANTING_TABLE: u16 = 116;
    /// `ender_chest`.
    pub const ENDER_CHEST: u16 = 130;
    /// `light_weighted_pressure_plate`.
    pub const LIGHT_PRESSURE_PLATE: u16 = 147;
    /// `heavy_weighted_pressure_plate`.
    pub const HEAVY_PRESSURE_PLATE: u16 = 148;
    /// `unpowered_comparator`.
    pub const UNPOWERED_COMPARATOR: u16 = 149;
    /// `powered_comparator`.
    pub const POWERED_COMPARATOR: u16 = 150;
    /// `daylight_detector`.
    pub const DAYLIGHT_DETECTOR: u16 = 151;
    /// `redstone_block`.
    pub const REDSTONE_BLOCK: u16 = 152;
    /// `hopper`.
    pub const HOPPER: u16 = 154;
    /// `dropper`.
    pub const DROPPER: u16 = 158;
    /// `activator_rail`.
    pub const ACTIVATOR_RAIL: u16 = 157;
    /// `quartz_ore`.
    pub const QUARTZ_ORE: u16 = 153;
    /// `stained_hardened_clay`.
    pub const STAINED_HARDENED_CLAY: u16 = 159;
    /// `log2` (acacia, dark oak).
    pub const LOG2: u16 = 162;
    /// `leaves2` (acacia and dark oak).
    pub const LEAVES2: u16 = 161;
    /// `slime`.
    pub const SLIME: u16 = 165;
    /// `hardened_clay`.
    pub const HARDENED_CLAY: u16 = 172;
    /// `coal_block`.
    pub const COAL_BLOCK: u16 = 173;
    /// `packed_ice`.
    pub const PACKED_ICE: u16 = 174;
    /// `daylight_detector_inverted`.
    pub const DAYLIGHT_DETECTOR_INVERTED: u16 = 178;

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
    /// `shears`.
    pub const SHEARS: i16 = 359;
    /// `dye` (the color's dye damage in the damage).
    pub const DYE: i16 = 351;
    /// `wool` (the block item).
    pub const WOOL: i16 = super::block::WOOL as i16;
    /// `mutton` (raw).
    pub const MUTTON: i16 = 423;
    /// `cooked_mutton`.
    pub const COOKED_MUTTON: i16 = 424;
    /// `flint_and_steel`.
    pub const FLINT_AND_STEEL: i16 = 259;
    /// `coal` (and charcoal, damage 1).
    pub const COAL: i16 = 263;
    /// `wooden_sword`.
    pub const WOODEN_SWORD: i16 = 268;
    /// `wooden_axe`; the wooden sword, shovel, pickaxe and axe are 268-271.
    pub const WOODEN_AXE: i16 = 271;
    /// `stick`.
    pub const STICK: i16 = 280;
    /// `wheat_seeds`.
    pub const WHEAT_SEEDS: i16 = 295;
    /// `wooden_hoe`.
    pub const WOODEN_HOE: i16 = 290;
    /// `sign`.
    pub const SIGN: i16 = 323;
    /// `banner`.
    pub const BANNER: i16 = 425;
    /// `painting`.
    pub const PAINTING: i16 = 321;
    /// `item_frame`.
    pub const ITEM_FRAME: i16 = 389;
    /// `armor_stand`.
    pub const ARMOR_STAND: i16 = 416;
    /// `minecart`.
    pub const MINECART: i16 = 328;
    /// `boat`.
    pub const BOAT: i16 = 333;
    /// `chest_minecart`.
    pub const CHEST_MINECART: i16 = 342;
    /// `furnace_minecart`.
    pub const FURNACE_MINECART: i16 = 343;
    /// `hopper_minecart`.
    pub const HOPPER_MINECART: i16 = 408;
    /// `tnt_minecart`.
    pub const TNT_MINECART: i16 = 407;
    /// `skull` (mob heads).
    pub const SKULL: i16 = 397;
    /// `bucket`.
    pub const BUCKET: i16 = 325;
    /// `water_bucket`.
    pub const WATER_BUCKET: i16 = 326;
    /// `lava_bucket`.
    pub const LAVA_BUCKET: i16 = 327;
    /// `potion` (the damage value encodes the effects).
    pub const POTION: i16 = 373;
    /// `snowball`.
    pub const SNOWBALL: i16 = 332;
    /// `bow`.
    pub const BOW: i16 = 261;
    /// `arrow`.
    pub const ARROW: i16 = 262;
    /// `egg`.
    pub const EGG: i16 = 344;
    /// `ender_pearl`.
    pub const ENDER_PEARL: i16 = 368;
    /// `experience_bottle`.
    pub const EXPERIENCE_BOTTLE: i16 = 384;
    /// `glass_bottle`.
    pub const GLASS_BOTTLE: i16 = 374;
    /// `milk_bucket`.
    pub const MILK_BUCKET: i16 = 335;
    /// `redstone` (the dust).
    pub const REDSTONE: i16 = 331;
    /// `repeater` (the item).
    pub const REPEATER: i16 = 356;
    /// `comparator` (the item).
    pub const COMPARATOR: i16 = 404;
    /// `blaze_rod`.
    pub const BLAZE_ROD: i16 = 369;
    /// `porkchop` (raw).
    pub const PORKCHOP: i16 = 319;
    /// `cooked_porkchop`.
    pub const COOKED_PORKCHOP: i16 = 320;
    /// `saddle`.
    pub const SADDLE: i16 = 329;
    /// `feather`.
    pub const FEATHER: i16 = 288;
    /// `chicken` (raw).
    pub const CHICKEN: i16 = 365;
    /// `cooked_chicken`.
    pub const COOKED_CHICKEN: i16 = 366;
    /// `bowl`.
    pub const BOWL: i16 = 281;
    /// `mushroom_stew`.
    pub const MUSHROOM_STEW: i16 = 282;
    /// `wheat` (the item).
    pub const WHEAT: i16 = 296;
    /// `leather`.
    pub const LEATHER: i16 = 334;
    /// `beef` (raw).
    pub const BEEF: i16 = 363;
    /// `cooked_beef`.
    pub const COOKED_BEEF: i16 = 364;
    /// `carrot` (the food).
    pub const CARROT: i16 = 391;
    /// `carrot_on_a_stick`.
    pub const CARROT_ON_A_STICK: i16 = 398;
    /// `fishing_rod`.
    pub const FISHING_ROD: i16 = 346;
    /// `spawn_egg` (the damage is the entity type id).
    pub const SPAWN_EGG: i16 = 383;
    /// `name_tag`.
    pub const NAME_TAG: i16 = 421;
    /// `lead`.
    pub const LEAD: i16 = 420;
}
