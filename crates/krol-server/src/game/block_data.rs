//! Block properties from the vanilla 1.8.8 block registry (generated from the
//! registrations: material and hardness per block id).

/// A block material (vanilla `Material`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Material {
    Air,
    Grass,
    Ground,
    Wood,
    Rock,
    Iron,
    Anvil,
    Water,
    Lava,
    Leaves,
    Plants,
    Vine,
    Sponge,
    Cloth,
    Fire,
    Sand,
    Circuits,
    Carpet,
    Glass,
    RedstoneLight,
    Tnt,
    Coral,
    Ice,
    PackedIce,
    Snow,
    CraftedSnow,
    Cactus,
    Clay,
    Gourd,
    DragonEgg,
    Portal,
    Cake,
    Web,
    Piston,
    Barrier,
}

impl Material {
    /// `Material.isSolid`: false for air, fire, liquids, portals and the "logic"
    /// materials (plants, vines, circuits, carpet, snow layers).
    pub fn is_solid(self) -> bool {
        !matches!(
            self,
            Self::Air
                | Self::Fire
                | Self::Water
                | Self::Lava
                | Self::Portal
                | Self::Plants
                | Self::Vine
                | Self::Circuits
                | Self::Carpet
                | Self::Snow
        )
    }

    /// `Material.blocksMovement`: like [`Self::is_solid`], except that cobwebs do not.
    pub fn blocks_movement(self) -> bool {
        self.is_solid() && self != Self::Web
    }

    /// `Material.isLiquid`.
    pub fn is_liquid(self) -> bool {
        matches!(self, Self::Water | Self::Lava)
    }

    /// `Material.getCanBurn`.
    pub fn can_burn(self) -> bool {
        matches!(
            self,
            Self::Wood | Self::Leaves | Self::Vine | Self::Cloth | Self::Carpet | Self::Tnt
        )
    }

    /// Whether any player can harvest blocks of this material, with or without a tool
    /// (`isToolNotRequired`).
    pub fn tool_not_required(self) -> bool {
        !matches!(
            self,
            Self::Rock
                | Self::Iron
                | Self::Anvil
                | Self::Snow
                | Self::CraftedSnow
                | Self::Web
                | Self::Barrier
        )
    }
}

/// Block id, registry name, material and hardness (`-1`: unbreakable).
pub const BLOCKS: [(&str, Material, f32); 198] = [
    ("air", Material::Air, 0.0),
    ("stone", Material::Rock, 1.5),
    ("grass", Material::Grass, 0.6),
    ("dirt", Material::Ground, 0.5),
    ("cobblestone", Material::Rock, 2.0),
    ("planks", Material::Wood, 2.0),
    ("sapling", Material::Plants, 0.0),
    ("bedrock", Material::Rock, -1.0),
    ("flowing_water", Material::Water, 100.0),
    ("water", Material::Water, 100.0),
    ("flowing_lava", Material::Lava, 100.0),
    ("lava", Material::Lava, 100.0),
    ("sand", Material::Sand, 0.5),
    ("gravel", Material::Sand, 0.6),
    ("gold_ore", Material::Rock, 3.0),
    ("iron_ore", Material::Rock, 3.0),
    ("coal_ore", Material::Rock, 3.0),
    ("log", Material::Wood, 2.0),
    ("leaves", Material::Leaves, 0.2),
    ("sponge", Material::Sponge, 0.6),
    ("glass", Material::Glass, 0.3),
    ("lapis_ore", Material::Rock, 3.0),
    ("lapis_block", Material::Iron, 3.0),
    ("dispenser", Material::Rock, 3.5),
    ("sandstone", Material::Rock, 0.8),
    ("noteblock", Material::Wood, 0.8),
    ("bed", Material::Cloth, 0.2),
    ("golden_rail", Material::Circuits, 0.7),
    ("detector_rail", Material::Circuits, 0.7),
    ("sticky_piston", Material::Piston, 0.5),
    ("web", Material::Web, 4.0),
    ("tallgrass", Material::Vine, 0.0),
    ("deadbush", Material::Vine, 0.0),
    ("piston", Material::Piston, 0.5),
    ("piston_head", Material::Piston, 0.5),
    ("wool", Material::Cloth, 0.8),
    ("piston_extension", Material::Piston, -1.0),
    ("yellow_flower", Material::Plants, 0.0),
    ("red_flower", Material::Plants, 0.0),
    ("brown_mushroom", Material::Plants, 0.0),
    ("red_mushroom", Material::Plants, 0.0),
    ("gold_block", Material::Iron, 3.0),
    ("iron_block", Material::Iron, 5.0),
    ("double_stone_slab", Material::Rock, 2.0),
    ("stone_slab", Material::Rock, 2.0),
    ("brick_block", Material::Rock, 2.0),
    ("tnt", Material::Tnt, 0.0),
    ("bookshelf", Material::Wood, 1.5),
    ("mossy_cobblestone", Material::Rock, 2.0),
    ("obsidian", Material::Rock, 50.0),
    ("torch", Material::Circuits, 0.0),
    ("fire", Material::Fire, 0.0),
    ("mob_spawner", Material::Rock, 5.0),
    ("oak_stairs", Material::Wood, 2.0),
    ("chest", Material::Wood, 2.5),
    ("redstone_wire", Material::Circuits, 0.0),
    ("diamond_ore", Material::Rock, 3.0),
    ("diamond_block", Material::Iron, 5.0),
    ("crafting_table", Material::Wood, 2.5),
    ("wheat", Material::Plants, 0.0),
    ("farmland", Material::Ground, 0.6),
    ("furnace", Material::Rock, 3.5),
    ("lit_furnace", Material::Rock, 3.5),
    ("standing_sign", Material::Wood, 1.0),
    ("wooden_door", Material::Wood, 3.0),
    ("ladder", Material::Circuits, 0.4),
    ("rail", Material::Circuits, 0.7),
    ("stone_stairs", Material::Rock, 2.0),
    ("wall_sign", Material::Wood, 1.0),
    ("lever", Material::Circuits, 0.5),
    ("stone_pressure_plate", Material::Rock, 0.5),
    ("iron_door", Material::Iron, 5.0),
    ("wooden_pressure_plate", Material::Wood, 0.5),
    ("redstone_ore", Material::Rock, 3.0),
    ("lit_redstone_ore", Material::Rock, 3.0),
    ("unlit_redstone_torch", Material::Circuits, 0.0),
    ("redstone_torch", Material::Circuits, 0.0),
    ("stone_button", Material::Circuits, 0.5),
    ("snow_layer", Material::Snow, 0.1),
    ("ice", Material::Ice, 0.5),
    ("snow", Material::CraftedSnow, 0.2),
    ("cactus", Material::Cactus, 0.4),
    ("clay", Material::Clay, 0.6),
    ("reeds", Material::Plants, 0.0),
    ("jukebox", Material::Wood, 2.0),
    ("fence", Material::Wood, 2.0),
    ("pumpkin", Material::Gourd, 1.0),
    ("netherrack", Material::Rock, 0.4),
    ("soul_sand", Material::Sand, 0.5),
    ("glowstone", Material::Glass, 0.3),
    ("portal", Material::Portal, -1.0),
    ("lit_pumpkin", Material::Gourd, 1.0),
    ("cake", Material::Cake, 0.5),
    ("unpowered_repeater", Material::Circuits, 0.0),
    ("powered_repeater", Material::Circuits, 0.0),
    ("stained_glass", Material::Glass, 0.3),
    ("trapdoor", Material::Wood, 3.0),
    ("monster_egg", Material::Clay, 0.75),
    ("stonebrick", Material::Rock, 1.5),
    ("brown_mushroom_block", Material::Wood, 0.2),
    ("red_mushroom_block", Material::Wood, 0.2),
    ("iron_bars", Material::Iron, 5.0),
    ("glass_pane", Material::Glass, 0.3),
    ("melon_block", Material::Gourd, 1.0),
    ("pumpkin_stem", Material::Plants, 0.0),
    ("melon_stem", Material::Plants, 0.0),
    ("vine", Material::Vine, 0.2),
    ("fence_gate", Material::Wood, 2.0),
    ("brick_stairs", Material::Rock, 2.0),
    ("stone_brick_stairs", Material::Rock, 1.5),
    ("mycelium", Material::Grass, 0.6),
    ("waterlily", Material::Plants, 0.0),
    ("nether_brick", Material::Rock, 2.0),
    ("nether_brick_fence", Material::Rock, 2.0),
    ("nether_brick_stairs", Material::Rock, 2.0),
    ("nether_wart", Material::Plants, 0.0),
    ("enchanting_table", Material::Rock, 5.0),
    ("brewing_stand", Material::Iron, 0.5),
    ("cauldron", Material::Iron, 2.0),
    ("end_portal", Material::Portal, -1.0),
    ("end_portal_frame", Material::Rock, -1.0),
    ("end_stone", Material::Rock, 3.0),
    ("dragon_egg", Material::DragonEgg, 3.0),
    ("redstone_lamp", Material::RedstoneLight, 0.3),
    ("lit_redstone_lamp", Material::RedstoneLight, 0.3),
    ("double_wooden_slab", Material::Wood, 2.0),
    ("wooden_slab", Material::Wood, 2.0),
    ("cocoa", Material::Plants, 0.2),
    ("sandstone_stairs", Material::Rock, 0.8),
    ("emerald_ore", Material::Rock, 3.0),
    ("ender_chest", Material::Rock, 22.5),
    ("tripwire_hook", Material::Circuits, 0.0),
    ("tripwire", Material::Circuits, 0.0),
    ("emerald_block", Material::Iron, 5.0),
    ("spruce_stairs", Material::Wood, 2.0),
    ("birch_stairs", Material::Wood, 2.0),
    ("jungle_stairs", Material::Wood, 2.0),
    ("command_block", Material::Iron, -1.0),
    ("beacon", Material::Glass, 3.0),
    ("cobblestone_wall", Material::Rock, 2.0),
    ("flower_pot", Material::Circuits, 0.0),
    ("carrots", Material::Plants, 0.0),
    ("potatoes", Material::Plants, 0.0),
    ("wooden_button", Material::Circuits, 0.5),
    ("skull", Material::Circuits, 1.0),
    ("anvil", Material::Anvil, 5.0),
    ("trapped_chest", Material::Wood, 2.5),
    ("light_weighted_pressure_plate", Material::Iron, 0.5),
    ("heavy_weighted_pressure_plate", Material::Iron, 0.5),
    ("unpowered_comparator", Material::Circuits, 0.0),
    ("powered_comparator", Material::Circuits, 0.0),
    ("daylight_detector", Material::Wood, 0.2),
    ("redstone_block", Material::Iron, 5.0),
    ("quartz_ore", Material::Rock, 3.0),
    ("hopper", Material::Iron, 3.0),
    ("quartz_block", Material::Rock, 0.8),
    ("quartz_stairs", Material::Rock, 0.8),
    ("activator_rail", Material::Circuits, 0.7),
    ("dropper", Material::Rock, 3.5),
    ("stained_hardened_clay", Material::Rock, 1.25),
    ("stained_glass_pane", Material::Glass, 0.3),
    ("leaves2", Material::Leaves, 0.2),
    ("log2", Material::Wood, 2.0),
    ("acacia_stairs", Material::Wood, 2.0),
    ("dark_oak_stairs", Material::Wood, 2.0),
    ("slime", Material::Clay, 0.0),
    ("barrier", Material::Barrier, -1.0),
    ("iron_trapdoor", Material::Iron, 5.0),
    ("prismarine", Material::Rock, 1.5),
    ("sea_lantern", Material::Glass, 0.3),
    ("hay_block", Material::Grass, 0.5),
    ("carpet", Material::Carpet, 0.1),
    ("hardened_clay", Material::Rock, 1.25),
    ("coal_block", Material::Rock, 5.0),
    ("packed_ice", Material::PackedIce, 0.5),
    ("double_plant", Material::Vine, 0.0),
    ("standing_banner", Material::Wood, 1.0),
    ("wall_banner", Material::Wood, 1.0),
    ("daylight_detector_inverted", Material::Wood, 0.2),
    ("red_sandstone", Material::Rock, 0.8),
    ("red_sandstone_stairs", Material::Rock, 0.8),
    ("double_stone_slab2", Material::Rock, 2.0),
    ("stone_slab2", Material::Rock, 2.0),
    ("spruce_fence_gate", Material::Wood, 2.0),
    ("birch_fence_gate", Material::Wood, 2.0),
    ("jungle_fence_gate", Material::Wood, 2.0),
    ("dark_oak_fence_gate", Material::Wood, 2.0),
    ("acacia_fence_gate", Material::Wood, 2.0),
    ("spruce_fence", Material::Wood, 2.0),
    ("birch_fence", Material::Wood, 2.0),
    ("jungle_fence", Material::Wood, 2.0),
    ("dark_oak_fence", Material::Wood, 2.0),
    ("acacia_fence", Material::Wood, 2.0),
    ("spruce_door", Material::Wood, 3.0),
    ("birch_door", Material::Wood, 3.0),
    ("jungle_door", Material::Wood, 3.0),
    ("acacia_door", Material::Wood, 3.0),
    ("dark_oak_door", Material::Wood, 3.0),
];

/// The material of a block id (unknown ids: air).
pub fn material(id: u16) -> Material {
    BLOCKS.get(usize::from(id)).map_or(Material::Air, |b| b.1)
}

/// The hardness of a block id (`blockHardness`; -1 is unbreakable).
pub fn hardness(id: u16) -> f32 {
    BLOCKS.get(usize::from(id)).map_or(0.0, |b| b.2)
}

/// The registry name of a block id (`minecraft:<name>`).
pub fn name(id: u16) -> Option<&'static str> {
    BLOCKS.get(usize::from(id)).map(|b| b.0)
}
