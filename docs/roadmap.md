# krol feature roadmap

A detailed inventory of what krol implements and what is still missing, organized by
vanilla 1.8.8's package structure (`net.minecraft.*`, MCP names), followed by krol's own
non-vanilla parts (protocol 5, plugin layers, parity harness). It is a map of the
territory, **not** an implementation order: the milestones in [AGENTS.md §12](../AGENTS.md#12-roadmap)
set the order. Parity coverage per feature lives in [`parity/coverage.toml`](../parity/coverage.toml).

Status as of 2026-11-19 (commit `server: feeding and breeding pigs`).

**Legend**

| Mark | Meaning |
|------|---------|
| ✅ | Implemented and covered by parity scenarios (or JVM-verified unit tests) |
| 🟡 | Partly implemented. The note says what is missing |
| ❌ | Missing |
| ➖ | Not applicable to a dedicated server (client-only, demo, LAN, ...) |

## Next up

The working queue within the current milestones (M4/M5), most useful first. Update it
when an item is done or priorities change.

1. **M5 mobs**: pigs run with AI, are fed, saddled and named, and breed
   (`game/mob.rs`, `game/mob/ai.rs`, `game/mob/breeding.rs`, `game/pathfinding.rs`).
   Next: riding saddled pigs (`EntityAIControlledByPlayer`, carrots on a stick),
   leads, spawn eggs (on blocks; on mobs they are done), `onInitialSpawn` (the follow
   range bonus from `/summon` without a data tag), attribute modifiers, children
   growing up (`Entity.setSize` keeps the box's corner, so the mob moves by the change
   in width; also for piglets loaded from a chunk), pushing minecarts and boats, then
   the other mobs, kind by kind (each new navigator: swimming, climbing). Share the living code with armor
   stands (`game/armor_stand.rs` has its own copy). Scenarios summon one pig per
   `krolpin` (its random pinned once); `/execute` or `/entitydata` would allow several.
   `krolpin spawns` pins mobs born later (`play/mobs_breed`).
2. **Disconnect timing**: vanilla handles a client's closed connection in
   `NetworkSystem.networkTick`, after the world tick; krol removes the player at the
   start of the tick, so entities run one tick without the player that vanilla runs
   with (seen as a pig's saved pitch in `play/mobs_tempt`). Moving it is a tick order
   change: run the full suite.
3. **Entity collisions with boats**: items, orbs, arrows, falling blocks and primed TNT
   moving into boats (only players, minecarts and boats collide with them so far), and
   armor stands bumping into boats.
4. **Parity robustness**: `play/tnt_pinned` replayed as 1.7 clients failed once in a
   parallel full run (a health update arriving after the explosion's block changes) and
   passed alone; make its timing robust.

"Overworld only" applies to the whole game layer: krol loads and ticks only dimension 0.
The nether and the end are ❌ (see [world](#netminecraftworld)).

---

## Contents

- [Next up](#next-up)
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
| Duplicate login (same UUID kicks the older connection) | ✅ | The new login waits for the old session to leave (`DELAY_ACCEPT`) |
| Whitelist, ops (levels, bypassesPlayerLimit), banned players/IPs, `usercache.json` | ✅ | |
| Mojang profile lookup for unknown names (op/whitelist offline players) | ❌ | Only cached or online names resolve |
| Online-mode authentication (session server `hasJoined`) | ✅ | No parity scenario (needs real accounts) |
| `player-idle-timeout` (`/setidletimeout`) | ✅ | |
| `ItemInWorldManager`: digging states, game modes, block activation, item use | 🟡 | See [block](#netminecraftblock) and [item](#netminecraftitem) for per-feature gaps |
| `PlayerManager`: chunk tracking, view distance, chunk send order and rate, block change batching (`multi_block_change`) | ✅ | |
| `EntityTracker`: tracking ranges, update intervals, spawn/move/look/teleport/velocity packets | 🟡 | Players, items, XP orbs, falling blocks, primed TNT, snowballs, eggs, ender pearls, experience bottles, splash potions, arrows (always look-and-move packets, from the first update) only |
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
| 0x04 Entity Equipment | ✅ | Players and armor stands |
| 0x0A Use Bed | ✅ | |
| 0x0B Animation | ✅ | Swing arm, hurt (critical effects missing) |
| 0x0C Spawn Player | ✅ | |
| 0x0D Collect Item | ✅ | Items and XP orbs |
| 0x0E Spawn Object | 🟡 | Items, falling blocks, primed TNT, snowballs, eggs, ender pearls, experience bottles, splash potions, arrows, item frames, armor stands, rideable, chest, hopper, furnace and TNT minecarts, boats. Missing: spawner and command block minecarts, fireballs, leash knots, fishing hooks, ... |
| 0x0F Spawn Mob | ✅ | Pigs (`play/mobs_no_ai`); 1.7: the 1.8-only entries dropped, the age as an int. Custom names of mobs for 1.7 ❌ |
| 0x10 Spawn Painting | ✅ | |
| 0x11 Spawn Experience Orb | ✅ | |
| 0x12 Entity Velocity · 0x13 Destroy Entities · 0x15 Entity Relative Move · 0x16 Entity Look · 0x17 Entity Look And Relative Move · 0x18 Entity Teleport · 0x19 Entity Head Look | ✅ | |
| 0x14 Entity (no-op) | ❌ | Not sent by krol yet; vanilla sends it for entities that didn't move in some update cycles |
| 0x1A Entity Status | 🟡 | Hurt, death, eating done (mobs too); other mob statuses (love, taming, ...) missing |
| 0x1B Attach Entity | 🟡 | Riding: to the rider on mounting and dismounting, to its viewers when spawned, on a change and every 60 updates (`play/riding`). Leashes ❌ |
| 0x1C Entity Metadata | 🟡 | Players, items, orbs, falling blocks, projectiles, hanging entities, armor stands (rotations included) |
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
| 0x2B Change Game State | 🟡 | Rain, game mode, respawn screen, missing bed, arrow hitting a player; missing: demo, credits, mob appearance |
| 0x2C Spawn Global Entity | ❌ | Lightning |
| 0x2D Open Window · 0x2E Close Window · 0x2F Set Slot · 0x30 Window Items · 0x31 Window Property · 0x32 Confirm Transaction | ✅ | For the implemented windows |
| 0x33 Update Sign · 0x36 Sign Editor Open | ✅ | |
| 0x34 Maps | ✅ | Whole map, changed rectangles, icons; 1.7: an icon list, the scale and one packet per column |
| 0x35 Update Block Entity | 🟡 | Signs, note blocks (via chunk), banners, flower pots, skulls, beacons; mob spawners missing |
| 0x37 Statistics | ✅ | |
| 0x38 Player List Item | 🟡 | Add, game mode, remove, latency (keep-alive round trips, every 600 ticks) ✅; display names ❌ |
| 0x39 Player Abilities | ✅ | |
| 0x3A Tab-Complete | ❌ | Serverbound tab-complete not handled |
| 0x3B Scoreboard Objective · 0x3C Update Score · 0x3D Display Scoreboard · 0x3E Teams | ✅ | 1.7: no render type, name tag visibility or team color; removing one score removes the name's scores |
| 0x3F Plugin Message | 🟡 | `MC|Brand` on join; `MC|BOpen`, `MC|TrList` missing |
| 0x40 Disconnect | ✅ | |
| 0x41 Server Difficulty | ✅ | |
| 0x42 Combat Event | ✅ | Enter and end combat (vanilla 1.8.8 never sends "entity died"); not sent to 1.7 |
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
| 0x02 Use Entity (attack, interact, interact at) | 🟡 | Attacking players, hanging entities and armor stands, interacting with item frames, interacting at armor stands ✅ (items, orbs and oneself kick like vanilla); mobs ❌ |
| 0x0C Steer Vehicle | 🟡 | The unmount flag gets a rider out; forward input starts a resting minecart (`play/riding`) and steers boats (`play/boats`). Pigs and horses ❌ |
| 0x11 Enchant Item | ✅ | |
| 0x14 Tab-Complete | ❌ | |
| 0x17 Plugin Message | 🟡 | Received; `MC|ItemName` ✅ (1.7's raw text translated), `MC|Beacon` ✅; `MC|BEdit`, `MC|BSign`, `MC|TrSel`, `MC|AdvCdm`, `MC|AutoCmd`, `MC|PickItem` unhandled |
| 0x18 Spectate | ❌ | |
| 0x19 Resource Pack Status | ❌ | |

## net.minecraft.nbt

| Feature | Status | Notes |
|---|---|---|
| All tag types, named root, uncompressed/gzip/zlib | ✅ | `krol-nbt` |
| Modified UTF-8 strings | ✅ | `krol-java` |
| `JsonToNBT` (NBT in commands: `/give`, `/summon`, `/setblock` data tags, `/entitydata`, ...) | ✅ | `krol_nbt::text::parse_data_tag`, with its quirks; used by `/give` and `/summon` |
| `NBTBase.toString` (item hover text) | ✅ | `krol_nbt::text::to_java_string` |
| `NBTUtil` (game profiles in skulls) | ✅ | `readGameProfileFromNBT`/`writeGameProfile` |
| `NBTUtil.func_181123_a` (partial NBT matching for `/clear`, `/testfor`) | ✅ | `krol_nbt::matches_pattern` |

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
| Map data (`MapData`, `data/map_*.dat`, `idcounts.dat`) | ✅ | Read (old smaller maps centered), written on save; the id counter written at once |
| Scoreboard data (`data/scoreboard.dat`) | ✅ | |
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
| `Block` (hardness, resistance, materials, sounds, light, drops, `isNormalCube`/`isFullCube`/`isVisuallyOpaque`/`isFullBlock`) | ✅ | Property tables extracted from vanilla. Drops of every block (`quantityDroppedWithBonus`, `getItemDropped`, `damageDropped` and the `dropBlockAsItemWithChance` overrides: crops, stems, nether wart, potatoes, spawners' experience), with fortune and silk touch (`canSilkHarvest`, `createStackedBlock`) ✅; heads (`BlockSkull.breakBlock`, `NODROP`), moving pistons' carried blocks (no scenario: they last two ticks) ✅ |
| `BlockAir`, `BlockBarrier`, `BlockStone`, `BlockDirt` (coarse dirt, podzol), `BlockPlanks`, `BlockSand`, `BlockSandStone`, `BlockRedSandstone`, `BlockClay`, `BlockGlowstone`, `BlockObsidian`, `BlockBookshelf`, `BlockNetherBrick`, `BlockNetherrack`, `BlockHardenedClay`, `BlockColored`, `BlockCompressedPowered` (redstone block), `BlockOre`, `BlockPrismarine`, `BlockQuartz`, `BlockSeaLantern`, `BlockStoneBrick`, `BlockMelon`, `BlockHay`, `BlockGlass`, `BlockStainedGlass`, `BlockPackedIce` | ✅ | Drops per block (ores, glowstone, melons, clay, sea lanterns, ...). Hay does not soften falls in 1.8.8 |
| `BlockGrass`, `BlockMycelium` | ✅ | Spreading/dying, snowy variant, bone meal on grass |
| `BlockGravel` | ✅ | Falling, flint drop chance, fortune |
| `BlockFalling` (sand, gravel, anvils) | ✅ | Dragon eggs ❌ |
| `BlockLog`, `BlockOldLog`, `BlockNewLog`, `BlockRotatedPillar` | ✅ | Axis by clicked face; leaf decay triggers |
| `BlockLeaves`, `BlockOldLeaf`, `BlockNewLeaf` | ✅ | Decay, drops (saplings, apples), shears, player-placed |
| `BlockSapling` | ✅ | Every tree kind, 2x2 trees, bone meal |
| `BlockBush`, `BlockFlower`, `BlockYellowFlower`, `BlockRedFlower`, `BlockTallGrass`, `BlockDeadBush`, `BlockDoublePlant` | ✅ | Support rules, drops, bone meal, shears. Double plant: the creative/shears-specific harvest rules (`onBlockHarvested`) 🟡 |
| `BlockMushroom`, `BlockHugeMushroom` | ✅ | Spreading, light rule, huge mushrooms, drops |
| `BlockCrops`, `BlockCarrot`, `BlockPotato`, `BlockStem`, `BlockNetherWart` | ✅ | Growth, bone meal, drops. Stems growing pumpkins/melons ✅ |
| `BlockFarmland` | ✅ | Moisture, drying, reverting, turning into dirt under a solid block, trampling by players (mobs with mob griefing arrive with mobs) |
| `BlockReed`, `BlockCactus` | ✅ | Growth, support, cactus damage |
| `BlockCocoa` | ✅ | Placement, growth, bone meal, drops, collision box |
| `BlockVine` | ✅ | Growth in all directions, support, shears. Climbing ✅ |
| `BlockLilyPad` | 🟡 | Placement rule; `ItemLilyPad` on water ❌ |
| `BlockSnow` (layers), `BlockSnowBlock`, `BlockIce` | ✅ | Melting, placement rules, layer stacking, snowball drops, ice turning into water when broken above something solid or liquid unless silk touch takes it, a harvested snow layer as one stack of layers + 1 snowballs (nether vaporizing ❌ with the nether) |
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
| `BlockChest` | ✅ | Trapped chests: weak and strong power from viewers; neighbors notified on open/close |
| `BlockEnderChest` | 🟡 | Placement, window, blocked by a normal cube on top, lid events and sounds, 8 obsidian, silk touch ✅ |
| `BlockFurnace` | ✅ | |
| `BlockDispenser`, `BlockDropper` | 🟡 | Default behavior ✅; special dispense behaviors ❌ |
| `BlockHopper` | ✅ | Pulling from and pushing into container minecarts (`play/container_minecarts`) |
| `BlockSign`, `BlockStandingSign`, `BlockWallSign` | ✅ | |
| `BlockCake` | ✅ | |
| `BlockBed` | 🟡 | Placement, sleeping (occupied bit written quietly like vanilla), skipping the night, spawn point, halves breaking together, drops from the foot ✅; mobs keeping players awake, nether explosion ❌ |
| `BlockAnvil` | ✅ | Placement, repair window, damage from use, falling (landing sound, damage on landing, shattering, hurting entities, crushing circuits blocks) |
| `BlockEnchantmentTable` | ✅ | Opens the window (custom names from the tile entity) |
| `BlockBrewingStand` | 🟡 | Placement (item), window, brewing, bottle bits, contents dropped, hoppers, comparators ✅ |
| `BlockCauldron` | ✅ | Water buckets, bottles, washing leather armor and banner patterns (with vanilla's stat quirk), rain, putting out burning players, comparator output. Rain and burning players are not in a scenario (rain needs pinned randoms) |
| `BlockBeacon` | 🟡 | Window, neighbor changes updating it twice (directly and by its block event) ✅; stained glass above (`updateColorAsync`), custom names from renamed items ❌ |
| `BlockFlowerPot` | ✅ | Placement on a solid top, potting plants (flowers, saplings, mushrooms, cactus, dead bush, fern), support, the plant dropping on any removal (not after a creative harvest) |
| `BlockJukebox` | ✅ | Records in and out (effect 1005), the record popping out when the jukebox goes, comparator output |
| `BlockSkull` | 🟡 | Placement, tile entity, drops, creative `NODROP` ✅; wither summoning ❌ (M5) |
| `BlockBanner` | ✅ | Standing and wall banners, support, harvest drop (Base and Patterns, no exhaustion) and other drops (the whole tile entity) |
| `BlockMobSpawner` | ❌ | Spawning logic, XP drop |
| `BlockCommandBlock` | ❌ | |
| `BlockDragonEgg` | ❌ | |
| `BlockPortal`, `BlockEndPortal`, `BlockEndPortalFrame` | ❌ | |
| `BlockSilverfish` | 🟡 | Drops nothing ✅; spawning silverfish ❌ |
| `BlockTNT` | 🟡 | Primed by redstone, flint and steel, fire, explosions, breaking explode-marked TNT ✅; fire charges, burning arrows, dispensers ❌ |
| `BlockRedstoneWire` | ✅ | Update order quirks reproduced |
| `BlockRedstoneTorch` | ✅ | Burnout |
| `BlockRedstoneLight` | ✅ | |
| `BlockRedstoneRepeater`, `BlockRedstoneComparator`, `BlockRedstoneDiode` | ✅ | Comparators reading containers, cake, cauldrons, item frames ✅; jukeboxes, brewing stands, end portal frames ❌ |
| `BlockLever`, `BlockButton`, `BlockButtonStone`, `BlockButtonWood` | 🟡 | Wooden buttons pressed by arrows ❌ |
| `BlockBasePressurePlate`, `BlockPressurePlate`, `BlockPressurePlateWeighted` | ✅ | Entity kinds limited to those krol has |
| `BlockPistonBase`, `BlockPistonExtension`, `BlockPistonMoving` | 🟡 | Moving entities ❌ |
| `BlockNote` | ✅ | |
| `BlockRedstoneOre` | ✅ | Lighting up on left-click, right-click and walking (with the particle draws on the world random), reverting on random ticks, drops, XP |
| `BlockDaylightDetector` | ❌ | |
| `BlockTripWire`, `BlockTripWireHook` | ❌ | |
| `BlockRail`, `BlockRailBase`, `BlockRailDetector`, `BlockRailPowered` | 🟡 | Placing (solid top below), shapes from the neighbors that connect (curves, slopes, T-junctions by power), bending the neighbors, popping off without support, powered and activator rails passing power along 8 rails, detector rail power and comparator updates (`play/rails`). Detector rails seeing minecarts (`play/minecarts`) and giving comparators a container cart's signal (`play/container_minecarts`) |

## net.minecraft.item

| Class | Status | Notes |
|---|---|---|
| `Item` registry, ids, names, max stack, max damage, subtypes | ✅ | Verified 1.8.8 table |
| `ItemStack`: NBT, display names, lore, `onBlockDestroyed` wear | ✅ | |
| `ItemBlock`, `ItemMultiTexture`, `ItemCloth`, `ItemColored`, `ItemLeaves`, `ItemSlab`, `ItemPiston`, `ItemSnow`, `ItemDoublePlant` | ✅ | |
| `ItemAnvilBlock` | 🟡 | Placement only |
| `ItemReed` (sugar cane, cake, repeater, comparator, brewing stand, cauldron, flower pot, skull?) | 🟡 | Sugar cane, cake, repeaters, comparators, brewing stands, flower pots, cauldrons ✅ |
| `ItemSeeds`, `ItemSeedFood` | ✅ | |
| `ItemDoor`, `ItemSign`, `ItemRedstone` | ✅ | |
| `ItemBed` | ✅ | |
| `ItemBanner` | ✅ | Placement (rotation, wall facing, no cactus next to it), base color and patterns from the item |
| `ItemSkull` | 🟡 | Placement, owner names (`%s's Head`) ✅; completing profiles through the profile cache and session server, `updateItemStackNBT` ❌ |
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
| `ItemPotion`, `ItemGlassBottle` | 🟡 | Drinking (effects from the damage value via a port of `PotionHelper`, checked against all 32768 values, or `CustomPotionEffects`), filling bottles at water, throwing splash potions (`play/splash_potions`), display names ("Splash Potion of Harming", "Thin Potion"; checked against vanilla's names of damage values 1-255) ✅ |
| `ItemExpBottle` | ❌ | |
| `ItemBow` | ✅ | Drawing (creative or with arrows; up to 72000 updates), releasing (power curve, nothing under 0.1, critical at full power), power/punch/flame/infinity, wear (a broken bow leaves the hand), `random.bow` from `Item.itemRand`, the first arrow stack used up (hotbar first), `stat.useItem`. `play/bows` |
| `ItemFishingRod` | ❌ | |
| `ItemSnowball`, `ItemEgg`, `ItemEnderPearl`, `ItemExpBottle` | ✅ | Throwing (see [entity.projectile](#entityprojectile)): the stack shrinks unless creative (creative players cannot throw pearls), `random.bow` with `Item.itemRand`, the use statistic |
| `ItemEnderEye`, `ItemFireball`, `ItemFirework`, `ItemFireworkCharge` | ❌ | |
| `ItemMinecart`, `ItemBoat` | 🟡 | Minecarts, chest, hopper, furnace and TNT minecarts placed on rails (`play/minecarts`, `play/container_minecarts`, `play/furnace_minecarts`, `play/tnt_minecarts`). Boats placed at the block or water in sight, facing away from the player, unless the player's eyes are in an entity or the boat does not fit (`play/boats`). Command block minecarts ❌ |
| `ItemHangingEntity` (paintings, item frames) | ✅ | The random art among those that fit, the use statistic even when nothing fits |
| `ItemArmorStand` | ✅ | On top of or in a replaceable block, both spaces free of blocks and entities (`play/armor_stands`), facing the player in eighths with a random head and body turn from the world random, `EntityTag` merged into the new stand (`play/armor_stand_equipment`) |
| `ItemMonsterPlacer` (spawn eggs) | 🟡 | Used on a pig (`EntityAgeable.interact`): a child of its kind, named after the egg (not covered by a scenario); on blocks and in dispensers ❌ |
| `ItemSaddle`, `ItemLead`, `ItemNameTag`, `ItemCarrotOnAStick` | 🟡 | Saddles on pigs, name tags on mobs (`play/mobs_breed`); leads and carrots on a stick ❌ |
| `ItemMap`, `ItemEmptyMap` | 🟡 | Making maps, drawing the held map (block map colors extracted from vanilla, height shading, water depth, the rim), player icons and the off-map icon, map packets, maps without data recreated around the spawn, zoomed maps from the crafting grid (cursor pickup), cloning/extending recipes, maps in item frames (the frame icon, map packets from the tracker to every player in the world), `Decorations` lists on items ✅ (`Decorations` lists and nether icon spin not in a scenario); shift-clicked zoom results, unloaded chunks, color ties at larger scales, the nether ❌ |
| `ItemBook`, `ItemWritableBook`, `ItemEditableBook`, `ItemEnchantedBook` | 🟡 | Book cloning recipe ✅; editing/signing books (`MC|BEdit`, `MC|BSign`) ❌ |
| `ItemRecord` | ✅ | `stat.recordPlayed` |
| `ItemCoal`, `ItemSimpleFoiled` | ✅ | |

### net.minecraft.item.crafting

| Feature | Status | Notes |
|---|---|---|
| `CraftingManager`: shaped and shapeless recipes, order | ✅ | Extracted from vanilla (`RecipeDump.java`) |
| `FurnaceRecipes`: smelting results and experience | ✅ | |
| `RecipeRepairItem`, `RecipesArmorDyes`, `RecipeBookCloning`, `RecipesMapCloning`, `RecipesMapExtending` | ✅ | |
| `RecipeFireworks` | ✅ | Rockets (no tag without stars), stars, fading (the detached-tag quirk) |
| `RecipesBanners` (patterns, duplication) | ✅ | Layered patterns only on a 3x3 grid; item patterns anywhere (black without dye) |
| `StatList.initCraftableStats` | ✅ | Only items some fixed recipe or smelting makes count `stat.craftItem` (not firework stars, rockets, filled maps, book copies) |

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
| `InventoryEnderChest` | ✅ | `EnderItems` in player data |
| `ContainerEnchantment` | ✅ | Offers from `XpSeed` and bookshelves, hints, slot rules, shift-click, paying levels and lapis, books; all seven properties every tick like vanilla |
| `ContainerBrewingStand` | ✅ | Slot rules, shift-click, brew time property; the potion achievement ❌ |
| `ContainerRepair` (anvil, renaming via `MC|ItemName`) | ✅ | Repair with material, combining, enchantments, renaming against the en_US item names, prior work cost, Too Expensive, the output's `canTakeStack`, vanilla's change checks |
| `ContainerBeacon` | ✅ | Payment slot (one emerald, diamond, gold or iron ingot), shift-clicks, the payment dropped on close, window properties sent once |
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
| `TileEntityEnderChest` | ✅ | Ticks with the other tickables; the 20-tick viewer resync (`% 20 * 4` quirk) |
| `TileEntityEnchantmentTable` | 🟡 | Created and stored; its book animation tick (no observable effect) is not run |
| `TileEntityBrewingStand` | ✅ | Brewing with vanilla's list-identity rules, the unsaved ingredient (a reload restarts the brew) |
| `TileEntityBeacon` | 🟡 | Levels and the beam check every 80 ticks, effects for players in range (doubled primary at level 4, secondary), `MC|Beacon` powers ✅; beam colors (client-only), the `fullBeacon` achievement ❌ |
| `TileEntityMobSpawner`, `MobSpawnerBaseLogic` | ❌ | |
| `TileEntitySkull` | 🟡 | `SkullType`, `Rot`, `Owner`, description packet ✅; `updateGameprofile` lookups and the old `ExtraType` ❌ |
| `TileEntityBanner` | ✅ | Base and patterns, description packet (action 6; not sent to 1.7 clients, which have no banners) |
| `TileEntityFlowerPot` | ✅ | Item and data, description packet (action 5, numeric item id) |
| `TileEntityDaylightDetector` | ❌ | |
| `TileEntityCommandBlock` | ❌ | |
| `TileEntityEndPortal` | ❌ | |
| Jukebox (`BlockJukebox.TileEntityJukebox`) | 🟡 | `RecordItem` ✅; the old numeric `Record` tag on load and `/setblock` data tags ❌ |

## net.minecraft.entity

### Base classes

| Class | Status | Notes |
|---|---|---|
| `Entity`: ids, `moveEntity` (collision, stepping, sneaking edges, webs), fall distance, water/lava handling, fire, `doBlockCollisions` | 🟡 | Cobwebs, soul sand, cactus, pressure plates ✅; portals, tripwires, detector rails ❌ |
| `Entity.isEntityInsideOpaqueBlock` | ✅ | Suffocation (players) |
| `DataWatcher` (metadata) | ✅ | |
| `EntityLivingBase`: health, hurt time, hurt resistance, `attackEntityFrom`, armor, absorption, effects, attributes, air, death update, knockback | 🟡 | Knockback from attacking entities ✅ (explosions); combat tracker death messages ✅ for implemented sources |
| `EntityLiving` (mobs: AI tasks, equipment, despawning, leashes) | 🟡 | `game/mob.rs`: spawn packets (the watcher's `HashMap` order, attributes changed by the constructor resent), fire, lava, the void, water, drowning, suffocation, the ambient sound on the mob's own random, hurt and death (status, sounds, knockback), loot and experience after a player's hit, equipment drops by drop chance, `/kill`, saving (`Equipment`, `DropChances`, `NoAI`, `Leash` kept, ...), animals removed with `spawn-animals=false`. Mobs with AI: `updateEntityActionState` (idle time and the despawn roll, goals, navigation, move/look/jump helpers), the body helper, jumping and swimming up, `moveEntityWithHeading` (ground, air, water with depth strider, lava, ladders), falls with damage and dust, farmland and slime landings, step and swim sounds, fire contact, the revenge target, pushing and being pushed by players and mobs. Name tags (`play/mobs_breed`). `play/mobs_ai`, `play/mobs_follow_parent`, `play/mobs_tempt`, `play/mobs_no_ai`, `play/mobs_animals_off`. `onInitialSpawn`, attribute modifiers, leads, picking up loot, effects, jump boost, the combat tracker, pushing minecarts and boats, projectiles and explosions hitting mobs ❌ |
| `EntityHanging` | ✅ | See `EntityItemFrame`, `EntityPainting` |
| `EntityCreature`, `EntityAgeable`, `EntityFlying` | ❌ | |
| `EntityTracker`, `EntityTrackerEntry` | 🟡 | All implemented entity kinds; ranges/intervals for missing kinds |
| `EntityList` (ids, names, NBT creation, spawn eggs) | ❌ | |
| `SharedMonsterAttributes`, `ai.attributes` | ✅ | Players |
| `EntityBodyHelper`, look/move/jump helpers | ❌ | |
| Entities in chunk NBT (`Entities`): loading and saving | 🟡 | Items, XP orbs, falling blocks, primed TNT, the throwables (snowballs, eggs, ender pearls, experience bottles, splash potions) and arrows are loaded (UUIDs, base entity fields, legacy minecart ids, unknown ids skipped with vanilla's warning) and saved per chunk section in vanilla's order, with `needsSaving`'s entity rule. Other kinds keep their saved NBT and are written back unchanged (not ticked or tracked); their `Riding` mounts only take entity ids. Update order follows entity ids, which matches section order for vanilla-written chunks. The "Wrong location!" warning is not logged |

### entity.player

| Feature | Status | Notes |
|---|---|---|
| `EntityPlayerMP` join, respawn, game modes, abilities, keep-alive | ✅ | |
| Movement validation (`NetHandlerPlayServer.processPlayer`): teleport confirmation, moved too quickly/wrongly, floating kick | ✅ | |
| Food stats: exhaustion, saturation, natural regeneration, starvation | ✅ | |
| Experience: levels, orbs, death drops | ✅ | Enchanting cost ❌ |
| Inventory, held item, armor, creative inventory | ✅ | |
| Death: messages, drops, keepInventory, respawn | ✅ | The body leaves the world 20 ticks after death (`setDead`, removed in the next entity loop) but stays among the world's players until it respawns: it is shown new entities and is the orbs' closest player. A respawned player keeps its entity id but moves to the end of the player list and the entity update order |
| Sneaking, sprinting (metadata, speed boost, hunger) | ✅ | |
| Suffocation, drowning, burning, fall damage, cactus | ✅ | |
| Damage outside the world border | ❌ | |
| Sleeping | 🟡 | Use bed, sleep timer, waking (day, bed gone, leaving, night skipped), spawn point, respawning at the bed or being told it is missing ✅; the 0.2 x 0.2 sleeping box, players saved asleep moved out of the bed ❌ |
| Attacking (`attackTargetEntityWithCurrentItem`): damage, critical hits, knockback, sprint attacks, enchantments, PvP setting | ✅ | Players only. Held item attack modifiers (also saved), sword blocking, thorns, tool wear and use stats, killers (combat tracker, `attackingPlayer`), revenge target (`HurtByTimestamp`), PvP death messages. Critical hits, sprint knockback, fire aspect (the burning flag follows on the victim's own update) and thorns (the sound at the attacker) are covered by `play/pvp_effects`; the overkill achievement ❌ |
| Interacting with entities (`interactWith`) | 🟡 | Minecarts, boats, item frames, armor stands and mobs (feeding, saddles, name tags, spawn eggs; `play/mobs_breed`); leads and riding pigs ❌ |
| Riding (`mountEntity`) | 🟡 | Players in rideable minecarts: getting in by interacting, riders updating right after their vehicle (and once more with each movement packet), sitting on the cart, sneaking or the unmount input getting out onto a free spot, getting out when the cart breaks or passes a powered activator rail, letting go silently of dead carts, the cart saved in the player's `Riding` tag (not with its chunk) and taken along on logout, back with the player on login (spawned at once), riders sending only looks to viewers (a teleport after getting off), carts moving slower and with less drag with a rider, the rider's push starting a resting cart, the distance ridden (`play/riding`). Players in boats: getting in, sitting ahead of the middle along the boat's yaw, steering, getting out, the boat distance, staying "in" a broken boat until it is gone (`play/boats`). Missing: mobs and armor stands riding (moving carts picking them up), pigs, horses, the onARail achievement, the rider's own move under the cart (block collisions) |
| Spectator mode: camera, teleport via `Spectate` | 🟡 | Game mode and noclip ✅; camera/spectate packets ❌ |
| Achievements awarded during play | 🟡 | `exploreAllBiomes` ✅; `openInventory`, `mineWood`, crafting and pickup achievements ❌ |
| Chat visibility and other client settings | 🟡 | Skin parts ✅; chat visibility filtering ❌ |
| Player list latency updates | ❌ | |

### entity.item

| Class | Status | Notes |
|---|---|---|
| `EntityItem` | ✅ | Physics (pushed out of blocks), merging, pickup, despawn, water, thrower, saving with the chunk. Burning in lava/fire (the saved `Fire` counter) ❌ |
| `EntityXPOrb` | ✅ | Pushed out of blocks, saving with the chunk (loaded orbs are 0.25 wide, a vanilla quirk). Lava, water flow ❌ |
| `EntityFallingBlock` | ✅ | Anvils (fall distance, hurting entities, helmet wear, the damage roll from the entity's own random), saving with the chunk (`TileEntityData` kept); dragon eggs, piston extensions, placing `TileEntityData` ❌ |
| `EntityTNTPrimed` | ✅ | Motion from `Math.random()`, fuse, water push, no splash (vanilla quirk), tracking (spawn object 50), saving with the chunk |
| `EntityMinecart*` (empty, chest, furnace, hopper, TNT, spawner, command block) | 🟡 | Rideable minecarts: placed on rails by `ItemMinecart` (adventure players too), following rails (snapping onto the line, at most 0.4 a tick, drag, slopes, powered rails boosting and kicking off from a wall, unpowered ones braking), derailed movement, bumping into each other (lined up, sharing motion), players pushing them, colliding with pushable entities while moving, hits (wobble and damage metadata, sharpness, knockback, exhaustion, broken past 40 damage or by creative players, the minecart dropped with its name), fire, water, activator rails, detector rails, tracking (spawn object 10, metadata in `HashMap` order), saving (custom display tiles kept), selectors, `/tp`, `/kill`, explosions, projectiles, pressure plates, blocking block placement (`play/minecarts`), riders (`play/riding`). Chest and hopper minecarts: windows titled with the cart's display name (spectators open them too), contents saved, spilled when the cart dies (`setDead`, twice when broken), the chest or hopper dropped, drag by fullness, hopper carts pulling every tick (the cooldown quirk) from the inventory above or item entities, turned off by powered activator rails, hoppers and droppers reaching cart inventories (a random cart from the world random), detector rail comparator signal (`play/container_minecarts`). Furnace minecarts: coal adding 3600 ticks of fuel, the push away from the player (renormalized along the rail each tick, dropped when moving against it), half speed, their own drag, handing their motion to carts they bump into, burning metadata, the furnace dropped unless an explosion killed them, saving (the fuel wrapping as a short) (`play/furnace_minecarts`). TNT minecarts: lit by powered activator rails (entity status 10, the fuse sound, 80 ticks), exploding with strength `4 + random * 1.5 * min(speed, 5)` from the cart's own random (resting carts always 4), a lit cart's explosion sparing rails and the blocks under them, exploding when crashing fast, landing from 3 blocks, killed by fire, explosions or while moving, hit by burning arrows, the TNT dropped unless an explosion killed them, `TNTFuse` saved (`play/tnt_minecarts`). Missing: spawner and command block minecarts, dispensers placing minecarts, nether portals, custom display tiles for 1.7 clients (their metadata 20 encodes the state differently). Riding: see Riding |
| `EntityBoat` | 🟡 | Placed by `ItemBoat`, tracked (spawn object 1, metadata 17-19 in `HashMap` order), floating by the share of its height in water, steered by its rider (speed multiplier, 0.35 top speed), turning at most 20 degrees a tick, drag, breaking lily pads and snow, wrecked into planks and sticks by crashes faster than 0.2975 or landings from more than 3 blocks (the fall distance kept on softer landings), hits (wobble and damage metadata, broken past 40 damage or by creative players, the boat dropped unless a creative player broke it, the rider's own projectiles ignored), fire, lava, water, the void, pushing other boats, solid for players and minecarts, colliding with every entity, saving, selectors, `/kill`, explosions, projectiles, wooden pressure plates, blocking block placement (`play/boats`). Missing: items, orbs, arrows, falling blocks and primed TNT colliding with boats, armor stands bumping into them, the rider's fall damage when a boat lands hard |
| `EntityItemFrame`, `EntityPainting` | ✅ | Placing, the surface check every 101 updates, breaking by hits, projectiles, explosions and falling anvils (no drops for creative players, `doEntityDrops`), items put in and turned (metadata, comparators), knocked out by hits but not explosions, the drop chance, tracking (spawn object 71, spawn painting; frames flush metadata every 10 ticks), saving with the chunk (old `Direction` saves), selectors, `/tp`, `/kill`, maps shown in frames (the frame icon, map packets every 10 ticks, the icon removed with the map). Invulnerable frames, `/tp` and old saves are not in a scenario |
| `EntityLeashKnot` | ❌ | |
| `EntityArmorStand` | 🟡 | The living update without input (gravity, water and lava with depth strider, ladders, falls with landing dust and farmland trampling, fire and lava setting it on fire, air under water with respiration, the void), punches (a second within 5 ticks knocks it over with its drops, adventure players cannot), creative players removing it, fire burning it down (`play/armor_stands`), equipment put on and taken off by interacting at a point with disabled slots, small stands, arms (`play/armor_stand_equipment`), arrows and explosions (drops only the contents; invisible stands are immune, blast protection on what it wears), markers (no size, no hits), tracking (spawn object 78, metadata with the pose, attributes, velocity, equipment), saving with the chunk, selectors, `/tp`, `/kill`, pressure plates, blocking block placement. Arrows, explosions, markers, loading and `/tp` are not in a scenario. Missing: status effects (`ActiveEffects` kept as saved), sounds of stands loaded without `Silent`, riding minecarts pushing them, dispensers putting armor on them, death at 0 health set by commands, saved attribute modifiers (kept, not applied) |
| `EntityEnderCrystal` | ❌ | |
| `EntityFireworkRocket` | ❌ | |
| `EntityEnderPearl` | ✅ | Thrown (not in creative), flies through its thrower, teleports the thrower (online, awake, alive; else the pearl just breaks) with 5 fall damage, dies with its thrower. Endermites (the 5% roll is made) and dismounting ❌ |
| `EntityExpBottle` | 🟡 | Thrown, lobbed (speed 0.7, -20° pitch, gravity 0.07), breaks with effect 2002 and 3-11 experience in orbs from the world random. No scenario: the world random cannot be pinned to the throw |
| `EntityEnderEye` | ❌ | |

### entity.projectile

| Class | Status | Notes |
|---|---|---|
| `EntityThrowable` | ✅ | Thrown from the eyes with a random spread (the entity's own random, unseeded), block and entity ray traces (collidable: players, falling blocks, primed TNT; the thrower only after 5 ticks), drag (0.8 in water), gravity, water push and splash, killed below y -64, saved with the chunk (`xTile`/`inTile`/`shake`/`inGround`/`ownerName`), thrower found again by name after loading. `play/throwables`. Nether portals (`setPortal`), catching fire in lava ❌ |
| `EntitySnowball` | ✅ | Hits do 0 damage (players ignore it; other entities get `setBeenAttacked`). Blazes (3 damage) ❌ |
| `EntityEgg` | 🟡 | Thrown and breaking (with the hatch rolls); the chicks ❌ (no mobs). No scenario: vanilla's 1-in-8 chick roll uses the unpinned entity random |
| `EntityPotion` (splash potions) | ✅ | Thrown from the item (speed 0.5, lobbed 20°, gravity 0.05), spawn object 73 with the damage value as data (and the spawn velocity), effect 2002 with the potion's damage value. The splash reaches players (not spectating) in the box grown by 4/2/4 and within 4 blocks, scaled by distance (1.0 for the entity hit): instant health and damage (`indirectMagic` with the thrower as attacker, plain magic without one), other effects over 20 ticks. Saved with the `Potion` item (a survival throw saves its count as 0, like vanilla; old `potionValue` read). `play/splash_potions`. Mobs (undead reversed) ❌ |
| `EntityArrow` | ✅ | Shot from bows (heading with a random sign per axis), flight with block and entity ray traces, sticking in blocks (pulled back 0.05, shake 7, falling out when the block changes, gone after 1200 ticks), hits on players (damage by speed, critical bonus from the unpinned random, fire from burning arrows, arrow count metadata and its countdown, punch, thorns, change game state 6 to the shooter, bouncing off when the hit does nothing; a player it may not hurt cancels the whole hit, a vanilla quirk), pickup (`pickup` 1 or 2), burning (velocity resent each second), water drag 0.6 and rain putting it out, pressure plates, saving with the chunk, the data watcher's `HashMap` order and first full change packet. `play/bows`. Wooden buttons and TNT hit by arrows, lava, mobs, dispensers ❌ |
| `EntityFishHook`, `EntityLargeFireball`, `EntitySmallFireball`, `EntityWitherSkull` | ❌ | |

### entity.passive, entity.monster, entity.boss, entity.ai

| Group | Status | Notes |
|---|---|---|
| `entity.ai` (task system, targeting, pathfinding navigation, look/move helpers, villager/animal behaviors) | 🟡 | `game/mob/ai.rs`: the goal selector (`EntityAITasks`), the move, look, jump and body helpers, `PathNavigateGround`, `RandomPositionGenerator` (no homes), goals swimming, panic, mate, tempt, follow parent, wander, watch closest (players), look idle (`play/mobs_ai`, `play/mobs_follow_parent`, `play/mobs_tempt`, `play/mobs_breed`). `EntityAIControlledByPlayer` never starts yet; target tasks, `EntitySenses`, homes and `EntityAIMoveTowardsRestriction`, sun avoidance, the other goals ❌. Look idle's `Math.cos`/`Math.sin` use Rust's libm (vanilla's may differ in the last bit) |
| Passive: pig (🟡 AI, sounds, steps, loot, cooked when burning, feeding, breeding, saddles; riding ❌), sheep (wool, shearing, dye), cow, mooshroom, chicken (eggs), squid, wolf, ocelot, horse (all variants, inventory), rabbit, bat, villager (trading, professions), iron golem, snow golem | ❌ | |
| Monsters: zombie (villagers, sieges, reinforcements), skeleton (wither skeleton), creeper, spider, cave spider, enderman, slime, magma cube, ghast, blaze, zombie pigman, silverfish, endermite, witch, guardian (elder) | ❌ | |
| Boss: ender dragon, wither | ❌ | |
| `entity.effect`: lightning bolt | ❌ | |
| Breeding, taming, leashing, name tags, saddles | 🟡 | `game/mob/breeding.rs`: pigs fed (love, piglets growing), breeding (`EntityAIMate.spawnBaby`), saddles, name tags (`play/mobs_breed`); taming and leashing ❌ (no tameable mobs yet) |
| Mob drops, XP, looting | 🟡 | Pigs: porkchops (cooked when burning, looting), saddles, equipment by drop chance, the rare drop roll, experience from the world random 20 ticks after death (`play/mobs_no_ai`); the other mobs ❌ |
| Natural spawning, spawners, spawn eggs | ❌ | `/summon` creates the non-mob entities and pigs without AI (see [commands](#netminecraftcommand)) |

## net.minecraft.potion

| Feature | Status | Notes |
|---|---|---|
| `Potion` table (ids, names, colors, instant/bad, liquid color) | ✅ | Extracted from vanilla (`PotionDump.java`) |
| `PotionEffect`: adding, combining, durations, amplifiers, ambient, particles flag, NBT | ✅ | |
| Ticking effects: regeneration, poison, wither, hunger, saturation, instant health/damage, absorption, health boost | ✅ | |
| Attribute effects: speed, slowness, strength, weakness | ✅ | Strength/weakness only affect attacking, which is ❌ |
| Haste, mining fatigue (dig speed) | ✅ | Mining fatigue by unit test |
| Jump boost, night vision, blindness, nausea, water breathing, fire resistance, resistance, invisibility | 🟡 | Fire resistance and resistance ✅ (golden apples); jump boost/water breathing server effects ❌; client-side effects ➖ |
| `PotionHelper` | ✅ | `getPotionEffects`, `applyIngredient` |
| Effect particles' random draws on the entity random | ❌ | |

## net.minecraft.enchantment

| Enchantment | Status | Notes |
|---|---|---|
| Protection, Fire Protection, Feather Falling | ✅ | |
| Blast Protection | ✅ | Damage and explosion knockback |
| Projectile Protection | 🟡 | Reduces arrow and thrown damage; not yet in a scenario |
| Respiration | ✅ | |
| Aqua Affinity, Efficiency | ✅ | Mining speed (efficiency only where the tool beats a hand); aqua affinity by unit test |
| Unbreaking | ✅ | Tool and armor wear |
| Thorns | ✅ | Players hurting players (the victim's unseeded random decides at levels up to 10) |
| Depth Strider | ❌ | |
| Silk Touch, Fortune | ✅ | Drops |
| Sharpness, Smite, Bane of Arthropods, Knockback, Fire Aspect, Looting | ❌ | Needs attacking and mobs |
| Power, Punch, Flame, Infinity | 🟡 | Implemented with bows; not yet in a scenario (`play/bows` uses a plain bow) |
| Luck of the Sea, Lure | ❌ | Needs fishing |
| `EnchantmentHelper` (enchanting table selection, `/enchant`) | 🟡 | Table selection (`calcItemStackEnchantability`, `buildEnchantmentList`, `mapEnchantmentData` in Java `HashMap` order), `/enchant` (`canApply`, compatibility, translated names) ✅; `addRandomEnchantment` ❌ |

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
| `effect` · `enchant` | ✅ | |
| `give` | ✅ | Leftovers dropped with their owner; command result stats ❌ |
| `setblock` | ✅ | Metadata normalized like `getStateFromMeta` (`data/block_meta.txt`); data tags for the tile entities krol creates (skulls, banners, beacons, spawners, command blocks, flower pots and daylight sensors ❌) |
| `tp` | ✅ | Every form, on players (relative flags) and other entities; the head yaw of players is not set |
| `kill` · `spawnpoint` | ✅ | |
| `clear` | ✅ | |
| `summon` | 🟡 | Every entity kind krol simulates, data tags, the legacy `Minecart` id, all errors ✅; mobs other than pigs are only kept as NBT (not ticked or shown), no `onInitialSpawn` without a data tag, `Riding` mounts are summoned but not mounted, `LightningBolt` only reports success ❌ (M5) |
| `achievement` · `blockdata` · `clone` · `debug` · `entitydata` · `execute` · `particle` · `playsound` · `replaceitem` · `spreadplayers` · `stats` · `testfor` · `testforblock` · `testforblocks` · `tellraw` · `title` · `worldborder` | ❌ | |
| `scoreboard` · `trigger` | ✅ | `players set` data tags for other entities than players ❌ |
| `publish` | ➖ | LAN only |
| `PlayerSelector` (`@p`, `@a`, `@r`, `@e` with arguments) | 🟡 | All arguments; one command run per target; UUIDs as names (entity UUIDs from their own unseeded random); selectors in `/say`, `/me`, `/tell`; `score_` for every entity, `team` for players; dropped items' names (`item.<unlocalized name>`, `data/item_entity_names.txt`); custom entity names ❌; `@r` shuffles with its own unseeded random like vanilla |
| `CommandResultStats` (`/stats` triggers) | ❌ | |
| Command blocks (`CommandBlockLogic`) and minecart command blocks | ❌ | |
| Operator notifications, `logAdminCommands`, `sendCommandFeedback` | ✅ | |

## net.minecraft.scoreboard

| Feature | Status | Notes |
|---|---|---|
| Objectives, scores, criteria: dummy, trigger, health, deathCount, statistics and achievements | ✅ | Statistic scores change the moment the statistic does (`addStat`), `stat.timeSinceDeath` scores reset on death; a holder's scores keep creation order (vanilla: identity hash order) |
| Kill criteria (`playerKillCount`, `totalKillCount`, `teamkill.*`, `killedByTeam.*`) | ✅ | Player kills; mob kills count for `totalKillCount` |
| Teams (prefix/suffix in display names, colors, name tag and death message visibility, packets) | ✅ | Friendly fire ✅; seeing invisible teammates affects nothing yet (no invisibility) |
| Display slots (sidebar, list, below name, team sidebars) | ✅ | |
| Non-player holders (scores and teams under the entity's UUID, dropped when a dead entity leaves the world) | ✅ | |
| `data/scoreboard.dat` | ✅ | |

## net.minecraft.stats

| Feature | Status | Notes |
|---|---|---|
| `stats/<uuid>.json`: format, key order, saving | ✅ | |
| General stats (play time, walking/swimming/falling/climbing/flying/diving/sprinting/crouching distances, jumps, deaths, drops, damage taken, time since death, leave game, cake slices, minecart and boat distances) | 🟡 | Pig, horse distances, damage dealt to mobs other than pigs, animals bred, fish caught, trading, talking to villagers ❌ (with their features) |
| Block mining, item crafting, item use, item break stats | ✅ | For implemented actions |
| Statistics packet (client request) | ✅ | |
| Achievements (`AchievementList`, requirements, chat announcements) | 🟡 | Storage and `exploreAllBiomes` ✅; awarding the others ❌ |

## net.minecraft.village / pathfinding

| Feature | Status | Notes |
|---|---|---|
| `VillageCollection`, doors, golems, reputation, sieges | ❌ | |
| `MerchantRecipe` lists (villager trades) | ❌ | |
| `pathfinding.*` (A*, path points, node processors for walking, swimming) | 🟡 | `game/pathfinding.rs`: `PathFinder` with its exact heap, `PathEntity`, `WalkNodeProcessor` (doors, trapdoors, fences, rails, water and lava, falls); `Block.isPassable` overrides. Swimming and climbing navigators ❌; the path's move target ignores entity collision boxes (`getCollidingBoundingBoxes`) |

## net.minecraft.util

| Feature | Status | Notes |
|---|---|---|
| `MathHelper` (sin table, floor, clamp, ...) | ✅ | JVM-verified (`krol-java`) |
| `java.util.Random` | ✅ | JVM-verified |
| `DamageSource` | 🟡 | fall, drown, onFire, inFire, lava, magic, wither, cactus, inWall, outOfWorld, explosion, explosion.player (difficulty scaling, knockback from the igniter), anvil, fallingBlock (helmet wear), player, thorns, thrown, indirectMagic (splash potions of harming), arrow (knockback from the shooter, else the arrow), projectile protection, the `.item` death messages of indirect sources ✅; generic, starve, mob, lightningBolt ❌ |
| `CombatTracker` (death messages) | 🟡 | For implemented sources; fall variants ✅ (vanilla quirk: the fatal hit clears the earlier entries, so a fatal fall is always an accident) |
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
| Scenario `restart` step, saved chunk entities observation | ✅ | |
| `--protocol 5` replays | ✅ | |
| Vanilla agent: pinning the world random and random tick LCG, timed random tick windows (`krolpin`) | ✅ | Mobs' own randoms: once (`krolpin`), every tick (`krolpin mobs`), at birth (`krolpin spawns`) |
| Pinning entity randoms and the clock | ❌ | |
| `[observe.tolerance]` | ✅ | |
| `parity fuzz` | ❌ | |
| `parity worldgen` | ❌ | M6 |
| `parity plugins` | ❌ | M8 |
| Testbot: protocol 47 and 5, movement, digging, placing, clicks, chat, item use, attacking (also critical hops), sprinting | ✅ | Interacting with entities ❌ |

### Tooling and docs

| Feature | Status | Notes |
|---|---|---|
| CI: fmt, clippy, tests, docs, `cargo-deny` | ✅ | Manual trigger |
| Data generators (`tools/datagen/*.java`): biomes, block light and shape flags, foods, items, noise vectors, potions, recipes, map colors | ✅ | |
| Player fixture generator (`tools/fixtures/players.py`) | ✅ | |
| `docs/third-party.md` | ✅ | |
| ADRs (`docs/adr/`), `docs/bukkit-compat.md`, `docs/bukkit-to-rust.md` | ❌ | Created with the plugin milestones |
