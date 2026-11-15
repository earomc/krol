# krol feature roadmap

A detailed inventory of what krol implements and what is still missing, organized by
vanilla 1.8.8's package structure (`net.minecraft.*`, MCP names), followed by krol's own
non-vanilla parts (protocol 5, plugin layers, parity harness). It is a map of the
territory, **not** an implementation order: the milestones in [AGENTS.md §12](../AGENTS.md#12-roadmap)
set the order. Parity coverage per feature lives in [`parity/coverage.toml`](../parity/coverage.toml).

Status as of 2026-11-15 (commit after `server: suffocation`).

**Legend**

| Mark | Meaning |
|------|---------|
| ✅ | Implemented and covered by parity scenarios (or JVM-verified unit tests) |
| 🟡 | Partly implemented. The note says what is missing |
| ❌ | Missing |
| ➖ | Not applicable to a dedicated server (client-only, demo, LAN, ...) |

"Overworld only" applies to the whole game layer: krol loads and ticks only dimension 0.
The nether and the end are ❌ (see [world](#netminecraftworld)).

---

## Contents

- [net.minecraft.server: server lifecycle](#netminecraftserver)
- [net.minecraft.network: protocol 47](#netminecraftnetwork)
- [net.minecraft.nbt](#netminecraftnbt)
- [net.minecraft.world](#netminecraftworld)
  - [chunk / storage](#chunk--storage) · [lighting](#lighting) · [ticking](#world-ticking) ·
    [biome](#netminecraftworldbiome) · [border](#netminecraftworldborder) · [gen](#netminecraftworldgen)
- [net.minecraft.block](#netminecraftblock)
- [net.minecraft.item](#netminecraftitem) · [item.crafting](#netminecraftitemcrafting)
- [net.minecraft.inventory](#netminecraftinventory)
- [net.minecraft.tileentity](#netminecrafttileentity)
- [net.minecraft.entity](#netminecraftentity)
- [net.minecraft.potion](#netminecraftpotion) · [enchantment](#netminecraftenchantment)
- [net.minecraft.dispenser](#netminecraftdispenser)
- [net.minecraft.command](#netminecraftcommand)
- [net.minecraft.scoreboard](#netminecraftscoreboard)
- [net.minecraft.stats](#netminecraftstats)
- [net.minecraft.village / pathfinding](#netminecraftvillage--pathfinding)
- [net.minecraft.util](#netminecraftutil)
- [Beyond vanilla](#beyond-vanilla-krols-own-parts)

---

## net.minecraft.server

`MinecraftServer`, `server.dedicated.*`, `server.management.*`, `server.network.*`.

| Feature | Status | Notes |
|---|---|---|
| `DedicatedServer` startup: EULA check, `eula.txt` creation | ✅ | |
| `server.properties`: keys, defaults, vanilla key order, rewrite on start | ✅ | Parsing of invalid values (`config.server_properties.invalid_values`) has no scenario yet |
| Console input on stdin, command dispatch | ✅ | |
| Vanilla log format, `logs/latest.log`, gzip rotation on startup | ✅ | Startup/shutdown log lines have no scenario |
| 20 TPS main loop, catching up when behind | ✅ | Single game thread, async networking (tokio) |
| `MinecraftServer.tick` order (weather, time, scheduled ticks, block ticks, entities, tile entities, players/network) | ✅ | Per world, as `WorldServer.tick` does |
| Autosave every 900 ticks, `save-on`/`save-off`/`save-all` | ✅ | |
| Graceful stop (`/stop`, Ctrl+C): saving players, chunks, `level.dat` | ✅ | |
| Server list ping (`ServerStatusResponse`, Gson escaping) | ✅ | |
| Player sample in the status, refreshed every 5 s | 🟡 | Implemented; no scenario |
| `server-icon.png` favicon | 🟡 | Implemented; no scenario |
| Legacy `0xFE` ping (beta, 1.4, 1.6 formats) | ✅ | |
| `ServerConfigurationManager`: join/leave, player list, view distance, max players, the "joined the game" message | ✅ | |
| Duplicate login (same UUID kicks the older connection) | ❌ | |
| Whitelist, ops (levels, bypassesPlayerLimit), banned players/IPs, `usercache.json` | ✅ | |
| Mojang profile lookup for unknown names (op/whitelist offline players) | ❌ | Only cached or online names resolve |
| Online-mode authentication (session server `hasJoined`) | ✅ | No parity scenario (needs real accounts) |
| `player-idle-timeout` (`/setidletimeout`) | ✅ | |
| `ItemInWorldManager`: digging states, game modes, block activation, item use | 🟡 | See [block](#netminecraftblock) and [item](#netminecraftitem) for per-feature gaps |
| `PlayerManager`: chunk tracking, view distance, chunk send order and rate, block change batching (`multi_block_change`) | ✅ | |
| `EntityTracker`: tracking ranges, update intervals, spawn/move/look/teleport/velocity packets | 🟡 | Players, items, XP orbs, falling blocks only |
| Spawn chunk loading ("Preparing spawn area") | ✅ | |
| Resource pack (`resource-pack`, `resource-pack-hash`) | ❌ | |
| `network-compression-threshold` | ✅ | |
| `enable-command-block`, `op-permission-level`, `spawn-protection` | 🟡 | `op-permission-level`, `spawn-protection` (digging, placing, buckets; spawn fuzz radius) ✅; command blocks ❌ |
| `max-world-size`, `max-build-height` | 🟡 | Build height checks ✅; world border/size ❌ |
| `snooper-enabled` | ➖ | No snooper (nothing observable for players) |
| RCON (`enable-rcon`) | ❌ | `server.rcon.*` |
| Query (`enable-query`) | ❌ | `server.dedicated` / `rcon.RConThreadQuery` |
| `MinecraftServer` crash reports (`crash.CrashReport`) | ❌ | krol logs panics; no crash-report files |
| GUI (`MinecraftServerGui`) | ➖ | |
| Demo mode, LAN (`/publish`) | ➖ | |

## net.minecraft.network

Protocol 47 is krol's internal model. Codecs live in `krol-protocol`, handlers in
`krol-server`.

### Connection states

| State | Status | Notes |
|---|---|---|
| Handshake | ✅ | |
| Status (modern + legacy) | ✅ | |
| Login: offline mode (UUID from `OfflinePlayer:<name>`) | ✅ | |
| Login: online mode, RSA key exchange, AES/CFB8 | ✅ | |
| Login: Set Compression | ✅ | |
| Login: outdated client/server messages | ✅ | |
| Play | 🟡 | See packet tables |
| Keep-alive interval and timeout | ✅ | |
| Malformed packets: vanilla disconnect behavior, never a panic | ✅ | Fuzzing (`parity fuzz`) not built yet |

### Play: clientbound (server → client)

| Packet | Status | Notes |
|---|---|---|
| 0x00 Keep Alive · 0x01 Join Game · 0x02 Chat · 0x03 Time Update · 0x05 Spawn Position · 0x06 Update Health · 0x07 Respawn · 0x08 Player Position And Look · 0x09 Held Item Change | ✅ | |
| 0x04 Entity Equipment | ✅ | Players only |
| 0x0A Use Bed | ✅ | |
| 0x0B Animation | ✅ | Swing arm, hurt (critical effects missing) |
| 0x0C Spawn Player | ✅ | |
| 0x0D Collect Item | ✅ | Items and XP orbs |
| 0x0E Spawn Object | 🟡 | Items, falling blocks. Missing: boats, minecarts, projectiles, TNT, item frames, armor stands, fishing hooks, ... |
| 0x0F Spawn Mob | ❌ | No mobs |
| 0x10 Spawn Painting | ❌ | |
| 0x11 Spawn Experience Orb | ✅ | |
| 0x12 Entity Velocity · 0x13 Destroy Entities · 0x15 Entity Relative Move · 0x16 Entity Look · 0x17 Entity Look And Relative Move · 0x18 Entity Teleport · 0x19 Entity Head Look | ✅ | |
| 0x14 Entity (no-op) | ❌ | Not sent by krol yet; vanilla sends it for entities that didn't move in some update cycles |
| 0x1A Entity Status | 🟡 | Hurt, death, eating done; mob statuses missing |
| 0x1B Attach Entity | ❌ | Riding, leashes |
| 0x1C Entity Metadata | 🟡 | Players, items, orbs, falling blocks |
| 0x1D Entity Effect · 0x1E Remove Entity Effect | ✅ | |
| 0x1F Set Experience | ✅ | |
| 0x20 Entity Properties | ✅ | Players |
| 0x21 Chunk Data · 0x26 Map Chunk Bulk | ✅ | Byte-identical to vanilla |
| 0x22 Multi Block Change · 0x23 Block Change | ✅ | |
| 0x24 Block Action | ✅ | Chests, note blocks, pistons |
| 0x25 Block Break Animation | ✅ | |
| 0x27 Explosion | ✅ | |
| 0x28 Effect | 🟡 | Many effects (doors, fire, bone meal, dispensers, block break 2001, ...); missing those of unimplemented features (records, potion splash, portal travel, ...) |
| 0x29 Sound Effect | 🟡 | Block, step, player and liquid sounds; mob and many item sounds missing |
| 0x2A Particle | 🟡 | Block dust on falls; most server-sent particles belong to missing features |
| 0x2B Change Game State | 🟡 | Rain, game mode, respawn screen, missing bed; missing: demo, credits, arrow hit, mob appearance |
| 0x2C Spawn Global Entity | ❌ | Lightning |
| 0x2D Open Window · 0x2E Close Window · 0x2F Set Slot · 0x30 Window Items · 0x31 Window Property · 0x32 Confirm Transaction | ✅ | For the implemented windows |
| 0x33 Update Sign · 0x36 Sign Editor Open | ✅ | |
| 0x34 Maps | ❌ | |
| 0x35 Update Block Entity | 🟡 | Signs, note blocks (via chunk), mob spawners/skulls/banners/beacons/flower pots missing |
| 0x37 Statistics | ✅ | |
| 0x38 Player List Item | 🟡 | Add, game mode, remove, latency (keep-alive round trips, every 600 ticks) ✅; display names ❌ |
| 0x39 Player Abilities | ✅ | |
| 0x3A Tab-Complete | ❌ | Serverbound tab-complete not handled |
| 0x3B Scoreboard Objective · 0x3C Update Score · 0x3D Display Scoreboard · 0x3E Teams | ❌ | Scoreboard missing |
| 0x3F Plugin Message | 🟡 | `MC|Brand` on join; `MC|BOpen`, `MC|TrList` missing |
| 0x40 Disconnect | ✅ | |
| 0x41 Server Difficulty | ✅ | |
| 0x42 Combat Event | ❌ | Vanilla 1.8.8 sends it on death (`EntityPlayerMP.onDeath` via the combat tracker) |
| 0x43 Camera | ❌ | Spectator mode entity view |
| 0x44 World Border | 🟡 | Initialize on join; changes (`/worldborder`) ❌ |
| 0x45 Title | ❌ | `/title` |
| 0x46 Set Compression (play) | ➖ | Unused by vanilla's server in play |
| 0x47 Player List Header And Footer | ➖ | Not sent by vanilla's server |
| 0x48 Resource Pack Send | ❌ | |
| 0x49 Update Entity NBT | ➖ | Unused by vanilla |

### Play: serverbound (client → server)

| Packet | Status | Notes |
|---|---|---|
| 0x00 Keep Alive · 0x01 Chat · 0x03–0x06 Player/Position/Look · 0x07 Player Digging · 0x08 Block Placement · 0x09 Held Item Change · 0x0A Animation · 0x0B Entity Action · 0x0D Close Window · 0x0E Click Window · 0x0F Confirm Transaction · 0x10 Creative Inventory Action · 0x12 Update Sign · 0x13 Player Abilities · 0x15 Client Settings · 0x16 Client Status | ✅ | |
| 0x02 Use Entity (attack, interact, interact at) | ❌ | No PvP or entity interaction yet |
| 0x0C Steer Vehicle | ❌ | No vehicles |
| 0x11 Enchant Item | ❌ | No enchanting table |
| 0x14 Tab-Complete | ❌ | |
| 0x17 Plugin Message | 🟡 | Received; `MC|BEdit`, `MC|BSign`, `MC|TrSel`, `MC|Beacon`, `MC|ItemName`, `MC|AdvCdm`, `MC|AutoCmd`, `MC|PickItem` unhandled |
| 0x18 Spectate | ❌ | |
| 0x19 Resource Pack Status | ❌ | |

## net.minecraft.nbt

| Feature | Status | Notes |
|---|---|---|
| All tag types, named root, uncompressed/gzip/zlib | ✅ | `krol-nbt` |
| Modified UTF-8 strings | ✅ | `krol-java` |
| `JsonToNBT` (NBT in commands: `/give`, `/summon`, `/setblock` data tags, `/entitydata`, ...) | ❌ | |
| `NBTUtil` (game profiles in skulls) | ❌ | |

## net.minecraft.world

| Feature | Status | Notes |
|---|---|---|
| `WorldProviderSurface` (overworld) | ✅ | |
| `WorldProviderHell` (nether), `WorldProviderEnd` | ❌ | Not loaded or ticked; `DIM-1`/`DIM1` are left untouched |
| `WorldServerMulti` (shared data of secondary dimensions) | ❌ | |
| `Teleporter` (nether portals), end portal travel | ❌ | |
| `GameRules`: storage, `/gamerule`, all 1.8.8 rules listed | 🟡 | Honored: `doTileDrops`, `doFireTick`, `keepInventory`, `doMobLoot`, `naturalRegeneration`, `randomTickSpeed`, `doDaylightCycle`, `showDeathMessages`, `sendCommandFeedback`, `logAdminCommands`, `commandBlockOutput` (where commands exist), `reducedDebugInfo`. `doEntityDrops` for falling blocks. Pending with their features: `doMobSpawning`, `mobGriefing`, `doEntityDrops` for other entities |
| `EnumDifficulty` (`/difficulty`, effects on hunger damage and fire) | 🟡 | Mob-related difficulty effects arrive with mobs; `DifficultyInstance` (regional difficulty) ❌ |
| `Explosion` | ✅ | Rays with block resistance (extracted from vanilla), the affected `HashSet` order, entity damage by exposure (`getBlockDensity`), knockback, drops with 1/strength chance, chain TNT, the packet. Flaming explosions and mobs ❌; treeified `HashSet` buckets ❌ |
| `SpawnerAnimals` (natural spawning, chunk generation spawns) | ❌ | |
| `World.playAuxSFX`, `playSoundEffect`, `spawnParticle` routing | ✅ | |
| `WorldManager` / `IWorldAccess` (block change and sound broadcasting) | ✅ | |
| Day/night cycle (`doDaylightCycle`), time packets every 20 ticks | ✅ | |
| `skylightSubtracted` | ✅ | |
| Weather: rain/thunder timers, `/weather`, `/toggledownfall`, rain/thunder strength packets | ✅ | Weather cycle has no scenario (timers use the world random) |
| Lightning strikes in thunderstorms (`updateBlocks` → `EntityLightningBolt`) | ❌ | The random draw is modeled; the strike is not |
| Snowfall and freezing water in cold biomes (`canBlockFreeze`, `canSnowAt` in `updateBlocks`) | ❌ | The per-chunk draws are modeled; placing snow/ice is not |
| Cave mood sounds (`playMoodSoundAndCheckLight`, ambient countdown) | ✅ | |
| `World.getCollidingBoundingBoxes`, `isAABBInMaterial`, `handleMaterialAcceleration` | ✅ | Collision shapes of every block, including neighbor-dependent ones; anvils and double chests use their constructor bounds (vanilla uses whatever a ray trace last left) |
| `World.rayTraceBlocks` | 🟡 | Buckets; selection boxes of stairs, fences and other shaped blocks missing |
| Village collection (`VillageCollection`, `villages.dat`) | ❌ | See village |
| Map data (`MapData`, `data/map_*.dat`, `idcounts.dat`) | ❌ | |
| Scoreboard data (`data/scoreboard.dat`) | ❌ | |
| Structure data (`data/Village.dat`, `Mineshaft.dat`, `Temple.dat`, `Fortress.dat`, `Stronghold.dat`) | ❌ | Arrives with structure generation |
| `WorldInfo` / `level.dat`: read, write (all fields, game rules, border, weather, time) | ✅ | |
| `session.lock` | ✅ | |

### chunk / storage

| Feature | Status | Notes |
|---|---|---|
| Anvil region files (`.mca`): read, write, sector allocation like vanilla | ✅ | `krol-world` |
| Chunk NBT: sections, block ids + add, metadata, block/sky light, height map, biomes, entities, tile entities, `TileTicks`, `LightPopulated`, `TerrainPopulated`, `InhabitedTime`, `V` | ✅ | |
| `InhabitedTime` accumulation | ✅ | |
| Chunk lifecycle: loading on demand, unloading queue, ticked/terrain/light population rules | ✅ | |
| `playerdata/<uuid>.dat` read/write (all player fields) | ✅ | |
| Legacy `players/<name>.dat` conversion | ❌ | Not needed for 1.8 worlds; vanilla 1.8 still converts |
| `ChunkProviderServer` generating missing chunks | ❌ | M6. Without a world, players get "krol: no world is loaded" |

### lighting

| Feature | Status | Notes |
|---|---|---|
| `checkLightFor` (sky and block), vanilla queue order and reach | ✅ | |
| Height maps, precipitation height, column relighting, sky light for new sections | ✅ | |
| Gap rechecks on chunk ticks (`enqueueRelightChecks`), `isGapLightingUpdated` | ✅ | |
| Light properties per block (opacity, emission, neighbor brightness) | ✅ | Extracted from vanilla (`tools/datagen/BlockLightDump.java`) |
| Generated chunk light (`generateSkylightMap`, `func_150809_p`) | ❌ | With worldgen |

### World ticking

| Feature | Status | Notes |
|---|---|---|
| Scheduled block ticks (`NextTickListEntry`: priority, order, 65536 limit, `TileTicks`) | ✅ | |
| Random block ticks (active chunks in vanilla order, `updateLCG`, `randomTickSpeed`) | ✅ | Compared exactly with the pinned world random |
| Entity update order (players, then other entities in load order) | ✅ | |
| Tile entity update order | ✅ | |
| Block events (`addBlockEvent`, de-duplication, per-tick processing) | ✅ | |

### net.minecraft.world.biome

| Feature | Status | Notes |
|---|---|---|
| Biome table: temperature, rainfall, snow, lightning, humidity | ✅ | Extracted from vanilla (`BiomeDump.java`) |
| `getFloatTemperature` (height falloff with temperature noise) | ✅ | |
| `pickRandomFlower` (plains, swamp, flower forest, default) | ✅ | |
| Biome decorators (`BiomeDecorator`), `genTerrainBlocks`, spawn lists | ❌ | M6 / mobs |
| `WorldChunkManager` (biome generation for chunks without stored biomes) | ❌ | Treated as plains for now |
| Grass and foliage colors | ➖ | Client-side |

### net.minecraft.world.border

| Feature | Status | Notes |
|---|---|---|
| Border stored in `level.dat`, sent on join | ✅ | |
| `/worldborder` and its packets, lerping | ❌ | |
| Damage outside the border, movement limits, block placement limits | ❌ | |

### net.minecraft.world.gen

All of M6. Today krol only plays existing worlds; flat-world parity fixtures are pregenerated.

| Feature | Status | Notes |
|---|---|---|
| Noise generators (`NoiseGeneratorSimplex`, `NoiseGeneratorPerlin`) | ✅ | Used for biome temperature and flower noise; JVM-verified |
| `NoiseGeneratorOctaves`, `NoiseGeneratorImproved` | ❌ | |
| `ChunkProviderGenerate` (default, large biomes, amplified, customized) | ❌ | |
| `ChunkProviderFlat` (`FlatGeneratorInfo`, presets, flat structures) | ❌ | |
| `ChunkProviderHell`, `ChunkProviderEnd` | ❌ | |
| `ChunkProviderDebug` (debug world type) | ❌ | |
| `gen.layer` (biome layers: `GenLayer*`) | ❌ | |
| `MapGenCaves`, `MapGenCavesHell`, `MapGenRavine` | ❌ | |
| `gen.structure`: villages, mineshafts, strongholds, scattered features (temples, witch huts), nether fortresses, ocean monuments | ❌ | |
| `gen.feature`: trees grown from saplings — `WorldGenTrees` (with vines, cocoa), `WorldGenForest`, `WorldGenTaiga2`, `WorldGenBigTree`, `WorldGenSavannaTree`, `WorldGenCanopyTree`, `WorldGenMegaJungle`, `WorldGenMegaPineTree`, `WorldGenBigMushroom` | ✅ | Compared block for block with the pinned random. As decorators: ❌ (`WorldGenTaiga1`, `WorldGenSwamp`, `WorldGenShrub`, generation-mode flags) |
| `gen.feature`: ores (`WorldGenMinable`), lakes, dungeons, liquids, flowers, tall grass, double plants, dead bushes, reeds, cactus, pumpkins, melons, vines, lily pads, clay, sand, ice spikes/paths, desert wells, glowstone, hell lava, fire, spikes (end), block blobs, bonus chest | ❌ | |
| `WorldGeneratorBonusChest` (`bonus-chest`) | ❌ | |
| Initial spawn point search | ❌ | |

## net.minecraft.block

Behavior per block class. "Basics" means: placement (rules and orientation), breaking
(hardness, tools, drops, step/break sounds) and collision for full cubes, which krol has
for all ordinary blocks. Listed here: what a class does beyond the basics.

| Class | Status | Notes |
|---|---|---|
| `Block` (hardness, resistance, materials, sounds, light, drops, `isNormalCube`/`isFullCube`/`isVisuallyOpaque`/`isFullBlock`) | ✅ | Property tables extracted from vanilla. Fortune and silk touch drops ❌ |
| `BlockAir`, `BlockBarrier`, `BlockStone`, `BlockDirt` (coarse dirt, podzol), `BlockPlanks`, `BlockSand`, `BlockSandStone`, `BlockRedSandstone`, `BlockClay`, `BlockGlowstone`, `BlockObsidian`, `BlockBookshelf`, `BlockNetherBrick`, `BlockNetherrack`, `BlockHardenedClay`, `BlockColored`, `BlockCompressedPowered` (redstone block), `BlockOre`, `BlockPrismarine`, `BlockQuartz`, `BlockSeaLantern`, `BlockStoneBrick`, `BlockMelon`, `BlockHay`, `BlockGlass`, `BlockStainedGlass`, `BlockPackedIce` | ✅ | Drops per block (ores, glowstone, melons, clay, sea lanterns, ...). Hay reduces fall damage |
| `BlockGrass`, `BlockMycelium` | ✅ | Spreading/dying, snowy variant, bone meal on grass |
| `BlockGravel` | 🟡 | Falling ✅; flint drop chance ✅; fortune ❌ |
| `BlockFalling` (sand, gravel) | ✅ | Anvils and dragon eggs ❌ |
| `BlockLog`, `BlockOldLog`, `BlockNewLog`, `BlockRotatedPillar` | ✅ | Axis by clicked face; leaf decay triggers |
| `BlockLeaves`, `BlockOldLeaf`, `BlockNewLeaf` | ✅ | Decay, drops (saplings, apples), shears, player-placed |
| `BlockSapling` | ✅ | Every tree kind, 2x2 trees, bone meal |
| `BlockBush`, `BlockFlower`, `BlockYellowFlower`, `BlockRedFlower`, `BlockTallGrass`, `BlockDeadBush`, `BlockDoublePlant` | ✅ | Support rules, drops, bone meal, shears. Double plant: the creative/shears-specific harvest rules (`onBlockHarvested`) 🟡 |
| `BlockMushroom`, `BlockHugeMushroom` | ✅ | Spreading, light rule, huge mushrooms, drops |
| `BlockCrops`, `BlockCarrot`, `BlockPotato`, `BlockStem`, `BlockNetherWart` | ✅ | Growth, bone meal, drops. Stems growing pumpkins/melons ✅ |
| `BlockFarmland` | ✅ | Moisture, drying, reverting, trampling by players (mobs with mob griefing arrive with mobs) |
| `BlockReed`, `BlockCactus` | ✅ | Growth, support, cactus damage |
| `BlockCocoa` | ✅ | Placement, growth, bone meal, drops, collision box |
| `BlockVine` | ✅ | Growth in all directions, support, shears. Climbing ✅ |
| `BlockLilyPad` | 🟡 | Placement rule; `ItemLilyPad` on water ❌ |
| `BlockSnow` (layers), `BlockSnowBlock`, `BlockIce` | ✅ | Melting, placement rules, layer stacking, snowball drops, ice turning into water when broken above something solid or liquid (silk touch ❌, nether vaporizing ❌ with the nether) |
| `BlockFire` | ✅ | Spread, burning, rain, netherrack. Nether portal lighting ❌ |
| `BlockLiquid`, `BlockDynamicLiquid`, `BlockStaticLiquid` | ✅ | Flow, mixing, lava fire. Nether lava speed ❌ (no nether) |
| `BlockSponge` | ❌ | Absorbing water; wet sponge drying in furnaces ✅ |
| `BlockSoulSand` | ✅ | Collision box, slowing entities |
| `BlockWeb` | ✅ | Slowing entities (`setInWeb`), fall distance reset |
| `BlockSlime` | ✅ | Fall damage cancel, bouncing, walking slowdown, pistons |
| `BlockLadder` | ✅ | Placement, support, climbing |
| `BlockTorch` | ✅ | |
| `BlockSlab`, `BlockStoneSlab*`, `BlockWoodSlab`, `BlockHalfStoneSlab*`, `BlockDoubleStoneSlab*` | ✅ | Placement, merging, collision |
| `BlockStairs` | 🟡 | Placement, collision shapes with inner and outer corners ✅; selection boxes for ray traces ❌ |
| `BlockFence`, `BlockFenceGate`, `BlockWall`, `BlockPane`, `BlockStainedGlassPane` | 🟡 | Gates open/close, connected collision shapes ✅; selection boxes for ray traces ❌ |
| `BlockDoor` | ✅ | Placement, hinge, open/close, redstone, halves |
| `BlockTrapDoor` | ✅ | |
| `BlockCarpet` | ✅ | |
| `BlockPumpkin` | 🟡 | Placement ✅; building iron golems and snow golems ❌ |
| `BlockWorkbench` | ✅ | Crafting window |
| `BlockChest` | ✅ | Trapped chests open like chests; their redstone output from viewers ❌ |
| `BlockEnderChest` | ❌ | Ender inventory window |
| `BlockFurnace` | ✅ | |
| `BlockDispenser`, `BlockDropper` | 🟡 | Default behavior ✅; special dispense behaviors ❌ |
| `BlockHopper` | ✅ | Minecart hoppers ❌ |
| `BlockSign`, `BlockStandingSign`, `BlockWallSign` | ✅ | |
| `BlockCake` | ✅ | |
| `BlockBed` | 🟡 | Placement, sleeping (occupied bit written quietly like vanilla), skipping the night, spawn point, halves breaking together, drops from the foot ✅; mobs keeping players awake, nether explosion ❌ |
| `BlockAnvil` | 🟡 | Placement ✅; falling, damage, repair window ❌ |
| `BlockEnchantmentTable` | ❌ | |
| `BlockBrewingStand` | 🟡 | Placement (item), window, brewing, bottle bits, contents dropped, hoppers, comparators ✅ |
| `BlockCauldron` | ❌ | Filling, rain, armor washing, comparator output |
| `BlockBeacon` | ❌ | |
| `BlockFlowerPot` | ❌ | |
| `BlockJukebox` | ❌ | The click is consumed when holding a record; records ❌ |
| `BlockSkull` | ❌ | Placement, tile entity, wither summoning |
| `BlockBanner` | ❌ | Standing and wall banners, patterns |
| `BlockMobSpawner` | ❌ | Spawning logic, XP drop |
| `BlockCommandBlock` | ❌ | |
| `BlockDragonEgg` | ❌ | |
| `BlockPortal`, `BlockEndPortal`, `BlockEndPortalFrame` | ❌ | |
| `BlockSilverfish` | 🟡 | Drops nothing ✅; spawning silverfish ❌ |
| `BlockTNT` | 🟡 | Primed by redstone, flint and steel, fire, explosions, breaking explode-marked TNT ✅; fire charges, burning arrows, dispensers ❌ |
| `BlockRedstoneWire` | ✅ | Update order quirks reproduced |
| `BlockRedstoneTorch` | ✅ | Burnout |
| `BlockRedstoneLight` | ✅ | |
| `BlockRedstoneRepeater`, `BlockRedstoneComparator`, `BlockRedstoneDiode` | ✅ | Comparators reading containers, cake ✅; jukeboxes, cauldrons, brewing stands, end portal frames, item frames ❌ |
| `BlockLever`, `BlockButton`, `BlockButtonStone`, `BlockButtonWood` | 🟡 | Wooden buttons pressed by arrows ❌ |
| `BlockBasePressurePlate`, `BlockPressurePlate`, `BlockPressurePlateWeighted` | ✅ | Entity kinds limited to those krol has |
| `BlockPistonBase`, `BlockPistonExtension`, `BlockPistonMoving` | 🟡 | Moving entities ❌ |
| `BlockNote` | ✅ | |
| `BlockRedstoneOre` | ✅ | Lighting up on left-click, right-click and walking (with the particle draws on the world random), reverting on random ticks, drops, XP |
| `BlockDaylightDetector` | ❌ | |
| `BlockTripWire`, `BlockTripWireHook` | ❌ | |
| `BlockRail`, `BlockRailBase`, `BlockRailDetector`, `BlockRailPowered` | ❌ | Shapes, powering, minecarts |

## net.minecraft.item

| Class | Status | Notes |
|---|---|---|
| `Item` registry, ids, names, max stack, max damage, subtypes | ✅ | Verified 1.8.8 table |
| `ItemStack`: NBT, display names, lore, `onBlockDestroyed` wear | ✅ | |
| `ItemBlock`, `ItemMultiTexture`, `ItemCloth`, `ItemColored`, `ItemLeaves`, `ItemSlab`, `ItemPiston`, `ItemSnow`, `ItemDoublePlant` | ✅ | |
| `ItemAnvilBlock` | 🟡 | Placement only |
| `ItemReed` (sugar cane, cake, repeater, comparator, brewing stand, cauldron, flower pot, skull?) | 🟡 | Sugar cane, cake, repeaters, comparators, brewing stands ✅; cauldron, flower pot ❌ |
| `ItemSeeds`, `ItemSeedFood` | ✅ | |
| `ItemDoor`, `ItemSign`, `ItemRedstone` | ✅ | |
| `ItemBed` | ✅ | |
| `ItemSkull`, `ItemBanner` | ❌ | |
| `ItemLilyPad` | ❌ | |
| `ItemTool`, `ItemPickaxe`, `ItemAxe`, `ItemSpade`, `ItemSword` | ✅ | Mining speed, wear. Sword blocking ❌, attack damage ❌ (no attacking) |
| `ItemHoe` | ✅ | |
| `ItemShears` | ✅ | Shearing sheep ❌ |
| `ItemFlintAndSteel` | 🟡 | Fire, lighting TNT ✅; portals, creepers ❌ |
| `ItemBucket` | 🟡 | Water and lava in survival ✅; creative use, milking cows ❌ |
| `ItemBucketMilk` | ✅ | |
| `ItemFood`, `ItemAppleGold`, `ItemSoup`, `ItemFishFood` | ✅ | |
| `ItemDye` | ✅ | Bone meal, cocoa beans. Dyeing sheep ❌ |
| `ItemArmor` | 🟡 | Equipping, protection, wear ✅; leather dye ✅ (crafting); dispensing armor ❌ |
| `ItemPotion`, `ItemGlassBottle` | 🟡 | Drinking (effects from the damage value via a port of `PotionHelper`, checked against all 32768 values, or `CustomPotionEffects`), filling bottles at water ✅; splash potions (thrown) ❌ |
| `ItemExpBottle` | ❌ | |
| `ItemBow` | ❌ | |
| `ItemFishingRod` | ❌ | |
| `ItemSnowball`, `ItemEgg`, `ItemEnderPearl`, `ItemEnderEye`, `ItemFireball`, `ItemFirework`, `ItemFireworkCharge` | ❌ | Projectiles |
| `ItemMinecart`, `ItemBoat` | ❌ | |
| `ItemHangingEntity` (paintings, item frames), `ItemArmorStand` | ❌ | |
| `ItemMonsterPlacer` (spawn eggs) | ❌ | |
| `ItemSaddle`, `ItemLead`, `ItemNameTag`, `ItemCarrotOnAStick` | ❌ | |
| `ItemMap`, `ItemEmptyMap` | ❌ | Map cloning/extending recipes ✅ |
| `ItemBook`, `ItemWritableBook`, `ItemEditableBook`, `ItemEnchantedBook` | 🟡 | Book cloning recipe ✅; editing/signing books (`MC|BEdit`, `MC|BSign`) ❌ |
| `ItemRecord` | ❌ | |
| `ItemCoal`, `ItemSimpleFoiled` | ✅ | |

### net.minecraft.item.crafting

| Feature | Status | Notes |
|---|---|---|
| `CraftingManager`: shaped and shapeless recipes, order | ✅ | Extracted from vanilla (`RecipeDump.java`) |
| `FurnaceRecipes`: smelting results and experience | ✅ | |
| `RecipeRepairItem`, `RecipesArmorDyes`, `RecipeBookCloning`, `RecipesMapCloning`, `RecipesMapExtending` | ✅ | |
| `RecipeFireworks` | ❌ | |
| `RecipesBanners` (patterns, duplication) | ❌ | |

## net.minecraft.inventory

| Class | Status | Notes |
|---|---|---|
| `Container` click handling: all modes (pick/place/split, shift, number keys, drag painting, double click, drop, creative clone), transaction confirmation, rejected predictions | ✅ | |
| `ContainerPlayer` (2x2 crafting, armor slots) | ✅ | |
| `ContainerWorkbench` | ✅ | |
| `ContainerChest`, `InventoryLargeChest` | ✅ | |
| `ContainerFurnace`, `SlotFurnaceFuel`, `SlotFurnaceOutput` | ✅ | |
| `ContainerDispenser` | ✅ | |
| `ContainerHopper` | ✅ | |
| `InventoryEnderChest` | ❌ | |
| `ContainerEnchantment` | ❌ | |
| `ContainerBrewingStand` | ✅ | Slot rules, shift-click, brew time property; the potion achievement ❌ |
| `ContainerRepair` (anvil, renaming via `MC|ItemName`) | ❌ | |
| `ContainerBeacon` | ❌ | |
| `ContainerMerchant`, `InventoryMerchant`, `SlotMerchantResult` | ❌ | |
| `ContainerHorseInventory`, `AnimalChest` | ❌ | |
| `LockCode` (container locks) | ❌ | |
| Container custom names | ✅ | Chests and furnaces (from renamed items) |

## net.minecraft.tileentity

| Class | Status | Notes |
|---|---|---|
| `TileEntity` load/save, update packets | ✅ | |
| `TileEntityChest` | ✅ | |
| `TileEntityFurnace` | ✅ | |
| `TileEntityDispenser`, `TileEntityDropper` | ✅ | |
| `TileEntityHopper` | ✅ | |
| `TileEntitySign` | ✅ | Click events in sign text (`/execute`-free run_command) ❌ |
| `TileEntityNote` | ✅ | |
| `TileEntityComparator` | ✅ | |
| `TileEntityPiston` | ✅ | |
| `TileEntityEnderChest` | ❌ | |
| `TileEntityEnchantmentTable` | ❌ | |
| `TileEntityBrewingStand` | ✅ | Brewing with vanilla's list-identity rules, the unsaved ingredient (a reload restarts the brew) |
| `TileEntityBeacon` | ❌ | |
| `TileEntityMobSpawner`, `MobSpawnerBaseLogic` | ❌ | |
| `TileEntitySkull` | ❌ | |
| `TileEntityBanner` | ❌ | |
| `TileEntityFlowerPot` | ❌ | |
| `TileEntityDaylightDetector` | ❌ | |
| `TileEntityCommandBlock` | ❌ | |
| `TileEntityEndPortal` | ❌ | |
| Jukebox (`BlockJukebox.TileEntityJukebox`) | ❌ | |

## net.minecraft.entity

### Base classes

| Class | Status | Notes |
|---|---|---|
| `Entity`: ids, `moveEntity` (collision, stepping, sneaking edges, webs), fall distance, water/lava handling, fire, `doBlockCollisions` | 🟡 | Cobwebs, soul sand, cactus, pressure plates ✅; portals, tripwires, detector rails ❌ |
| `Entity.isEntityInsideOpaqueBlock` | ✅ | Suffocation (players) |
| `DataWatcher` (metadata) | ✅ | |
| `EntityLivingBase`: health, hurt time, hurt resistance, `attackEntityFrom`, armor, absorption, effects, attributes, air, death update, knockback | 🟡 | Knockback from attacking entities ✅ (explosions); combat tracker death messages ✅ for implemented sources |
| `EntityLiving` (mobs: AI tasks, equipment, despawning, leashes) | ❌ | |
| `EntityCreature`, `EntityAgeable`, `EntityFlying`, `EntityHanging` | ❌ | |
| `EntityTracker`, `EntityTrackerEntry` | 🟡 | All implemented entity kinds; ranges/intervals for missing kinds |
| `EntityList` (ids, names, NBT creation, spawn eggs) | ❌ | |
| `SharedMonsterAttributes`, `ai.attributes` | ✅ | Players |
| `EntityBodyHelper`, look/move/jump helpers | ❌ | |
| Entities in chunk NBT (`Entities`): loading and saving | 🟡 | Items, XP orbs, falling blocks; unknown kinds are kept in chunk data ✅ |

### entity.player

| Feature | Status | Notes |
|---|---|---|
| `EntityPlayerMP` join, respawn, game modes, abilities, keep-alive | ✅ | |
| Movement validation (`NetHandlerPlayServer.processPlayer`): teleport confirmation, moved too quickly/wrongly, floating kick | ✅ | |
| Food stats: exhaustion, saturation, natural regeneration, starvation | ✅ | |
| Experience: levels, orbs, death drops | ✅ | Enchanting cost ❌ |
| Inventory, held item, armor, creative inventory | ✅ | |
| Death: messages, drops, keepInventory, respawn | ✅ | The dead body staying tracked for others 🟡 |
| Sneaking, sprinting (metadata, speed boost, hunger) | ✅ | |
| Suffocation, drowning, burning, fall damage, cactus | ✅ | |
| Damage outside the world border | ❌ | |
| Sleeping | 🟡 | Use bed, sleep timer, waking (day, bed gone, leaving, night skipped), spawn point, respawning at the bed or being told it is missing ✅; the 0.2 x 0.2 sleeping box, players saved asleep moved out of the bed ❌ |
| Attacking (`attackTargetEntityWithCurrentItem`): damage, critical hits, knockback, sprint attacks, enchantments, PvP setting | ❌ | |
| Interacting with entities (`interactWith`) | ❌ | |
| Riding (`mountEntity`) | ❌ | |
| Spectator mode: camera, teleport via `Spectate` | 🟡 | Game mode and noclip ✅; camera/spectate packets ❌ |
| Achievements awarded during play | 🟡 | `exploreAllBiomes` ✅; `openInventory`, `mineWood`, crafting and pickup achievements ❌ |
| Chat visibility and other client settings | 🟡 | Skin parts ✅; chat visibility filtering ❌ |
| Player list latency updates | ❌ | |

### entity.item

| Class | Status | Notes |
|---|---|---|
| `EntityItem` | ✅ | Physics, merging, pickup, despawn, water. Burning in lava/fire ❌ |
| `EntityXPOrb` | ✅ | Lava ❌ |
| `EntityFallingBlock` | ✅ | Anvils (damage), dragon eggs ❌ |
| `EntityTNTPrimed` | ✅ | Motion from `Math.random()`, fuse, water push, no splash (vanilla quirk), tracking (spawn object 50). Saving with the chunk ❌ |
| `EntityMinecart*` (empty, chest, furnace, hopper, TNT, spawner, command block) | ❌ | |
| `EntityBoat` | ❌ | |
| `EntityItemFrame`, `EntityPainting`, `EntityLeashKnot` | ❌ | |
| `EntityArmorStand` | ❌ | |
| `EntityEnderCrystal` | ❌ | |
| `EntityFireworkRocket` | ❌ | |
| `EntityEnderEye`, `EntityEnderPearl`, `EntityExpBottle` | ❌ | |

### entity.projectile

`EntityArrow`, `EntitySnowball`, `EntityEgg`, `EntityPotion`, `EntityFishHook`,
`EntityLargeFireball`, `EntitySmallFireball`, `EntityWitherSkull`, `EntityThrowable`: ❌

### entity.passive, entity.monster, entity.boss, entity.ai

| Group | Status | Notes |
|---|---|---|
| `entity.ai` (task system, targeting, pathfinding navigation, look/move helpers, villager/animal behaviors) | ❌ | |
| Passive: pig, sheep (wool, shearing, dye), cow, mooshroom, chicken (eggs), squid, wolf, ocelot, horse (all variants, inventory), rabbit, bat, villager (trading, professions), iron golem, snow golem | ❌ | |
| Monsters: zombie (villagers, sieges, reinforcements), skeleton (wither skeleton), creeper, spider, cave spider, enderman, slime, magma cube, ghast, blaze, zombie pigman, silverfish, endermite, witch, guardian (elder) | ❌ | |
| Boss: ender dragon, wither | ❌ | |
| `entity.effect`: lightning bolt | ❌ | |
| Breeding, taming, leashing, name tags, saddles | ❌ | |
| Mob drops, XP, looting | ❌ | |
| Natural spawning, spawners, spawn eggs, `/summon` | ❌ | |

## net.minecraft.potion

| Feature | Status | Notes |
|---|---|---|
| `Potion` table (ids, names, colors, instant/bad, liquid color) | ✅ | Extracted from vanilla (`PotionDump.java`) |
| `PotionEffect`: adding, combining, durations, amplifiers, ambient, particles flag, NBT | ✅ | |
| Ticking effects: regeneration, poison, wither, hunger, saturation, instant health/damage, absorption, health boost | ✅ | |
| Attribute effects: speed, slowness, strength, weakness | ✅ | Strength/weakness only affect attacking, which is ❌ |
| Haste, mining fatigue (dig speed) | ❌ | |
| Jump boost, night vision, blindness, nausea, water breathing, fire resistance, resistance, invisibility | 🟡 | Fire resistance and resistance ✅ (golden apples); jump boost/water breathing server effects ❌; client-side effects ➖ |
| `PotionHelper` | ✅ | `getPotionEffects`, `applyIngredient` |
| Effect particles' random draws on the entity random | ❌ | |

## net.minecraft.enchantment

| Enchantment | Status | Notes |
|---|---|---|
| Protection, Fire Protection, Feather Falling | ✅ | |
| Blast Protection | ✅ | Damage and explosion knockback |
| Projectile Protection | ❌ | Needs projectiles |
| Respiration | ✅ | |
| Aqua Affinity, Efficiency | ❌ | Mining speed (the underwater and in-air penalties are ✅) |
| Unbreaking | ✅ | Tool and armor wear |
| Thorns | ❌ | |
| Depth Strider | ❌ | |
| Silk Touch, Fortune | ❌ | Drops |
| Sharpness, Smite, Bane of Arthropods, Knockback, Fire Aspect, Looting | ❌ | Needs attacking and mobs |
| Power, Punch, Flame, Infinity | ❌ | Needs bows |
| Luck of the Sea, Lure | ❌ | Needs fishing |
| `EnchantmentHelper` (enchanting table selection, `/enchant`) | ❌ | |

## net.minecraft.dispenser

| Feature | Status | Notes |
|---|---|---|
| `BehaviorDefaultDispenseItem` (dropping items, effects 1000/2000) | ✅ | |
| Projectile behaviors (arrows, eggs, snowballs, potions, XP bottles, fire charges, fireworks) | ❌ | |
| Buckets, bone meal, flint and steel, TNT, minecarts, boats, armor, spawn eggs, skulls/pumpkins | ❌ | |

## net.minecraft.command

Syntax, permission levels, usage messages and errors follow vanilla for every listed
command. All 58 commands are registered (help lists them); unimplemented ones reply with a
krol notice.

| Command | Status | Notes |
|---|---|---|
| `help` · `list` · `say` · `me` · `tell` (`msg`, `w`) | ✅ | |
| `time` · `weather` · `toggledownfall` · `difficulty` · `defaultgamemode` · `gamemode` · `gamerule` · `seed` | ✅ | |
| `xp` · `kick` · `setidletimeout` · `setworldspawn` · `stop` | ✅ | |
| `save-all` · `save-on` · `save-off` | ✅ | |
| `op` · `deop` · `ban` · `pardon` · `ban-ip` · `pardon-ip` · `banlist` · `whitelist` | ✅ | |
| `effect` | ✅ | |
| `setblock` | 🟡 | Data tags ❌ |
| `tp` | 🟡 | Coordinates and players ✅; some forms (entity targets, rotation) ❌ |
| `kill` · `spawnpoint` | ✅ | Players as targets (selectors ❌) |
| `clear` | 🟡 | Data tags ❌ |
| `achievement` · `blockdata` · `clone` · `debug` · `enchant` · `entitydata` · `execute` · `fill` · `give` · `particle` · `playsound` · `replaceitem` · `scoreboard` · `spreadplayers` · `stats` · `summon` · `testfor` · `testforblock` · `testforblocks` · `tellraw` · `title` · `trigger` · `worldborder` | ❌ | `give` needs item display names (`en_US.lang`) |
| `publish` | ➖ | LAN only |
| `PlayerSelector` (`@p`, `@a`, `@r`, `@e` with arguments) | ❌ | Plain player names only |
| `CommandResultStats` (`/stats` triggers) | ❌ | |
| Command blocks (`CommandBlockLogic`) and minecart command blocks | ❌ | |
| Operator notifications, `logAdminCommands`, `sendCommandFeedback` | ✅ | |

## net.minecraft.scoreboard

| Feature | Status | Notes |
|---|---|---|
| Objectives, criteria (dummy, trigger, health, deathCount, kill counts, stats, teamkill), scores | ❌ | |
| Teams (prefix/suffix, friendly fire, name tag visibility, colors) | ❌ | |
| Display slots (sidebar, list, below name, team sidebars) | ❌ | |
| `data/scoreboard.dat` | ❌ | |

## net.minecraft.stats

| Feature | Status | Notes |
|---|---|---|
| `stats/<uuid>.json`: format, key order, saving | ✅ | |
| General stats (play time, walking/swimming/falling/climbing/flying/diving/sprinting/crouching distances, jumps, deaths, drops, damage taken, time since death, leave game, cake slices) | 🟡 | Riding, boat, minecart, pig, horse distances, mob kills, damage dealt, animals bred, fish caught, trading, talking to villagers ❌ (with their features) |
| Block mining, item crafting, item use, item break stats | ✅ | For implemented actions |
| Statistics packet (client request) | ✅ | |
| Achievements (`AchievementList`, requirements, chat announcements) | 🟡 | Storage and `exploreAllBiomes` ✅; awarding the others ❌ |

## net.minecraft.village / pathfinding

| Feature | Status | Notes |
|---|---|---|
| `VillageCollection`, doors, golems, reputation, sieges | ❌ | |
| `MerchantRecipe` lists (villager trades) | ❌ | |
| `pathfinding.*` (A*, path points, node processors for walking, swimming) | ❌ | |

## net.minecraft.util

| Feature | Status | Notes |
|---|---|---|
| `MathHelper` (sin table, floor, clamp, ...) | ✅ | JVM-verified (`krol-java`) |
| `java.util.Random` | ✅ | JVM-verified |
| `DamageSource` | 🟡 | fall, drown, onFire, inFire, lava, magic, wither, cactus, inWall, outOfWorld, explosion, explosion.player (difficulty scaling, knockback from the igniter) ✅; generic, starve, anvil, fallingBlock, player/mob/projectile/thorns, indirect magic, lightningBolt ❌ |
| `CombatTracker` (death messages) | 🟡 | For implemented sources; fall variants ✅ |
| `IChatComponent` JSON (text, translation, styles, click/hover events) | ✅ | |
| `StatCollector` (translation of log lines) | ✅ | For the keys krol uses |
| `BlockPos`, `AxisAlignedBB`, `MovingObjectPosition`, `EnumFacing` | ✅ | |
| `WeightedRandom` | 🟡 | Where used; most users (mobs, fishing, enchanting, chests in structures) are missing |
| `CooldownTracker` | ➖ | Not in 1.8 |

---

## Beyond vanilla: krol's own parts

### Protocol 5 (1.7.6–1.7.10 clients)

| Feature | Status | Notes |
|---|---|---|
| Handshake, status (modern + legacy) | ✅ | |
| Login (no compression in protocol 5) | ✅ | |
| Play translation 5 ↔ 47 for every packet krol sends and handles | ✅ | All play scenarios replay as 1.7 clients (`parity run --protocol 5`) |
| Translation of packets krol doesn't send yet (mobs, scoreboard, titles → chat, ...) | ❌ | Arrives with each feature |

### Rust plugin API (`krol-api`, M7)

❌ Not started: the crate, event bus, scheduler, commands, permissions, documentation,
example plugin, `docs/bukkit-to-rust.md`.

### Bukkit 1.8.8 layer (`krol-bukkit`, `java/`, M8–M9)

❌ Not started: embedded JVM, the Bukkit 1.8.8 API, CraftBukkit/NMS `v1_8_R3` shims,
`bukkit.yml`/`commands.yml`/`help.yml`/`permissions.yml`, the japicmp check,
`docs/bukkit-compat.md`, the plugin corpus.

### Parity harness (`parity/`, `xtask`)

| Feature | Status | Notes |
|---|---|---|
| `parity run`, `record`, `diff`, `coverage`, `report.json` | ✅ | |
| `--protocol 5` replays | ✅ | |
| Vanilla agent: pinning the world random and random tick LCG, timed random tick windows (`krolpin`) | ✅ | |
| Pinning entity randoms and the clock | ❌ | |
| `[observe.tolerance]` | ✅ | |
| `parity fuzz` | ❌ | |
| `parity worldgen` | ❌ | M6 |
| `parity plugins` | ❌ | M8 |
| Testbot: protocol 47 and 5, movement, digging, placing, clicks, chat, item use | ✅ | Attacking/interacting with entities ❌ |

### Tooling and docs

| Feature | Status | Notes |
|---|---|---|
| CI: fmt, clippy, tests, docs, `cargo-deny` | ✅ | Manual trigger |
| Data generators (`tools/datagen/*.java`): biomes, block light and shape flags, foods, items, noise vectors, potions, recipes | ✅ | |
| Player fixture generator (`tools/fixtures/players.py`) | ✅ | |
| `docs/third-party.md` | ✅ | |
| ADRs (`docs/adr/`), `docs/bukkit-compat.md`, `docs/bukkit-to-rust.md` | ❌ | Created with the plugin milestones |
