# Vanilla parity suite

Runs the same scripted scenarios against the vanilla 1.8.8 server and krol, normalizes
what was observed and diffs it. See `AGENTS.md` §10.2 for the full design.

```sh
cargo xtask parity run [--filter status]    # krol vs golden (or live vanilla); writes parity/report.json
cargo xtask parity record [--rerecord]      # record vanilla observations into parity/golden/
cargo xtask parity diff status/modern_ping  # one scenario's diffs from the last report
cargo xtask parity coverage                 # feature coverage (parity/coverage.toml)
```

## Fuzzing

```sh
cargo xtask parity fuzz [--seed N] [--steps M] [--cases K] [--protocol 5] [--shrink-runs R]
```

Generates random bot sessions and runs each on live vanilla (`KROL_VANILLA_JAR`) and krol at
once. A case is a player (an operator, starting in creative) in the flat fixture world
taking `M` steps (default 20): walking, looking, setting hotbar slots, selecting one,
placing or using on a block, using in the air (golden apples, milk, armor), digging,
dropping items, clicking in window 0 or 1, closing it, sneaking, switching between
creative and survival (`/gamemode`), summoning armor stands and minecarts, and attacking
or interacting with one of the three most recently spawned entities (the bot's `#<n>`
targets). In survival, walks stay on the ground or go up 4 blocks to fall back down, and
digs finish 18 ticks after they start (no block mined by hand needs within a tick of
that) or are aborted after 5. Each step gets a second, and actions are sent while
the player stands still, following the race-free rules below. After the last step the
window is closed, so a stack left on the cursor drops before logout. Each case observes the bot's
packets, every block, light level and tile entity in the area, the chunk's saved entities,
the player file and the log. `--seed` defaults to the current time; `--cases K` runs seeds
`N..N+K`.

When the servers differ, vanilla runs the case a second time. Paths where the two vanilla
runs disagree are ignored from then on (a whole list for a list element). If nothing else
differs, the case is `noisy`: the generator or harness has to change, not krol. Otherwise
it is `fail`: the case is shrunk by removing steps (ddmin, at most `R` runs of both servers,
default 40), run once more to check that it still fails (`reproduced`), and written to
`scenarios/fuzz/seed-N.toml` with its first differences in a header comment. Record it,
fix krol, then rename the scenario after what it covers and keep it. The report goes to
`parity/fuzz-report.json` (`fuzz-report-v5.json` with `--protocol 5`, where krol gets a
1.7 client and both sides are reduced to the 1.7 view). One case of 20 steps takes about a
minute; shrinking takes a minute per attempt.

The generator leaves out items whose behavior draws from unseeded randoms or runs on its
own clock (falling blocks, TNT, liquids, spawn eggs, throwables, bone meal) and turns
off tile drops and random block ticks. Tossed items keep their random yaw and spread: their motion and
rotation are ignored, and their position (1 block), `Age` and `PickupDelay` (2 ticks)
compared with tolerances (an `[*]` in an ignore or tolerance path matches any list
index). When a noisy path keeps coming up, fix it at the source (pin it,
or extend the generator's fixed ignores) rather than letting every case pay for a second
vanilla run.

`--filter` keeps the scenarios whose name contains the text (a substring, not a glob:
`--filter play/armor_stand` matches both armor stand scenarios; a `*` matches nothing).

`run` takes `--protocol 5` (replay the play scenarios as 1.7 clients; scenarios with
`v5 = false` in `[setup]` are left out, for what 1.7 clients cannot do, such as
interacting with an armor stand at a point) and `--jobs N` (`-j N`,
scenarios at once, default 4, capped at the CPU count). Scenarios mostly wait on scripted bot
sessions, so 4 jobs bring a full run from about 18 to about 5 minutes. A scenario that fails
during a parallel run is run once more on its own; if it passes then, it counts as passed and
is marked "failed in parallel, passed alone" (`retried_alone` in the report, with the parallel
run's `parallel_diffs`). Those are
scenarios whose bot timing races with the tick: worth making robust (see the rules below).
`record` takes `--jobs` too but defaults to 1: every recording starts a JVM, whose default
heap is a quarter of the RAM.

Environment (never commit the jars):

| Variable | Purpose |
|----------|---------|
| `KROL_VANILLA_JAR` | Vanilla 1.8.8 server jar, for `record` and live comparison |
| `KROL_JAVA` | Java executable for vanilla (default `java`; 1.8.8 expects Java 8) |
| `KROL_CRAFTBUKKIT_JAR` | CraftBukkit/Spigot 1.8.8 jar (plugin parity, later) |
| `KROL_BUKKIT_API_JAR` | Bukkit 1.8.8 API jar (API compatibility check, later) |

Layout:

- `scenarios/<area>/<name>.toml` — scenarios (`setup`, `steps`, `observe`).
- `golden/<area>/<name>.json` — recorded vanilla observations (committed).
- `coverage.toml` — vanilla feature → scenario map.
- `report.json` — last run's machine-readable report (git-ignored).

Scenario status: `pass`, `fail` (with structured diffs), `error`, or `no_golden` (nothing
to compare against yet: record with `KROL_VANILLA_JAR`). Work directories and full
observations are kept under `target/parity/` for inspection.

Step actions: `status_ping` (`protocol`, `payload`), `raw` (`hex`: first chunk, observe
until close), `login_start` (`protocol`, `name`), `restart` (stop the server, which saves
everything, and start it again in the same directory; background play steps are collected
first and the log runs on). `[observe] entities = [[cx, cz]]` compares the chunks' saved
`Entities` lists after the final stop, with each `UUIDMost`/`UUIDLeast` pair shown as one
UUID string (so random ones compare as `<uuid-N>`). `[observe] files` reads NBT files gzipped or uncompressed (`data/idcounts.dat`); byte arrays show as their length and an FNV-1a hash, so map colors compare by content. Observation paths listed in
`observe.ignore` are excluded — use only for documented, deliberate deviations.

## Pinning vanilla's randoms

A scenario with `pin_random = true` in `[setup]` loads `parity/vanilla-agent` into
vanilla (built by xtask with the local JDK: `javac`/`jar`, or `KROL_JAVAC` and
`KROL_JAR_TOOL`). It adds the console command
`krolpin <seed> [randomTickSpeed] [ticks] [-- <command>]`, which seeds every world's
`World.rand`, random tick generator (`updateLCG`) and the JVM's `Math.random()` generator
(primed TNT, item and orb motion), optionally sets the random tick speed in the same tick
and back to 0 after exactly that many ticks, and runs the command after `--` right away
in the same tick (e.g. a `setblock` that primes TNT); krol answers the same command in
builds with the `parity` feature (xtask builds krol that way, into `target/parity-build`).
Issue `krolpin` from the console right before the behavior to compare, once every player
has joined: each active chunk draws from the world random every tick, so a player
arriving later changes the draws. Start and stop random ticks, and trigger what should be
compared, through `krolpin` itself rather than separate commands, whose tick depends on
timing. After the command, `krolpin` also seeds every mob's own random (`Entity.rand` of
each `EntityLiving`) with the seed plus its entity id, so a mob summoned by the command
starts pinned. Mobs draw from their random every tick (the ambient sound roll), so what
a bot does to a mob at some wall-clock moment would meet an unknown state;
`krolpin mobs <seed> <ticks>` seeds every mob's random again at the end of each of the
next ticks, so the bot's action meets the same state whichever tick it lands in (mob
ambient rolls then repeat the same value every tick: pick the seed so the sound comes at
a quiet time). `krolpin spawns <seed>` pins the random of every mob that joins a world
from then on (summoned, or born to breeding parents) with the seed plus its entity id,
before its first update; chunk loads are left alone. `krolpin world <seed> <ticks>` pins the world
random, the random tick generator and `Math.random()` like `krolpin <seed>`, now and at
the end of each of the next ticks, so a bot's action that draws from the world random
(a spawn egg's facing) meets the same state whichever tick it lands in. Other entities' randoms (sound
pitches, knockback rolls of players, item pushes) are not pinned.

## Writing race-free scenarios

Vanilla and krol only compare equal when nothing depends on how the bot's packets fall
into server ticks. Rules learned from flaky scenarios:

- **Order only through the bot.** Console steps run on absolute time from the scenario
  start, bot actions on time since the bot's join. Anything that must happen in order
  relative to the bot (game mode switches, removing a block it just used) belongs to
  the bot itself, or to console steps *before* the `play` step. Prefer a fixture player
  file (`tools/fixtures/players.py`) over a console `/gamemode` switch.
- **`wait_ms` waits after a console command**, not before; use a `wait` step to delay
  one. A `background = true` play step is followed by a 1.5 s pause before the next step,
  so console steps after it run 1.5 s later than their own waits suggest: a `/give`
  after a 1.5 s `wait` lands about 3 s into the bot's timeline (`play/maps`).
  The bot's `@ms` clock starts with its connection, and vanilla logs in more slowly than
  krol, so leave a second or more between bot actions and console steps they race with.
- **Turn off random block ticks** (`gamerule randomTickSpeed 0` before the player
  joins) whenever blocks are placed, dug or set: grass dies and spreads at random.
- **Keep bot actions 200 ms or more apart** when their order matters. Two actions in
  one tick can be answered in a different order (inventory updates go out in slot
  order). Use 1 s for clicks whose effects must not merge into one batch: a short
  stall makes the server catch up several ticks at once.
- **Mind the unseeded randoms**: item arcs, fire spread, lava receding, drop counts.
  Keep players out of reach of drops until the step that should pick them up, turn off
  tile drops where they don't matter, and observe outcomes that cannot change by
  chance (e.g. fire's light level instead of its age).
- **Avoid competing timers**: a damage source that both hits and sets on fire races its
  own burning for the hurt cooldown; prefer one with a single effect (cactus).
- When something genuinely counts packets (stats, durations left), use a commented
  `[observe.tolerance]` entry for exactly those values.
- **Leave room after a click for what it triggers**: when a click sets off block changes
  a few ticks later (comparators, lamps), give the next action 800 ms so those land first
  even on a busy machine (parallel runs).
- When two packets' order depends on which tick a console command lands in (in vanilla
  too: e.g. an effect's attribute properties vs its particle color, which waits for the
  next movement packet, with the player tracker flushing every 2 ticks), declare them in
  a commented `[observe] unordered = [["entity_metadata", "entity_properties"]]` group:
  directly adjacent packets of those types are compared in a canonical order.
- Random UUIDs (version 4: non-player entities draw theirs from an unseeded random)
  are compared as `<uuid-N>`, numbered in order of first appearance, on both sides.
  Offline players' UUIDs are name-based (version 3) and compared as they are.
- Relative teleports (`tp ~ ~ ~`) are compared for 1.7 clients by resolving them against
  the previous teleport, which assumes the bot did not move in between.
- **Keep a victim silent when hits must compare exactly**: a player's own update (health
  packet, burning flag, burn damage) runs when its movement packet is handled, before or
  after another player's attack in the same tick depending on arrival order. A bot with
  `moves` of only `[]` sends no packets, so everything that happens to it comes from the
  attacker's packets and the entity loop (see `play/pvp_effects`).
- **Players cannot be hurt by players for 60 ticks** after joining or respawning
  (`respawnInvulnerabilityTicks`), and a respawned player keeps its entity id (target it
  with the same `!attack` id).
- **Items spawned inside a block get pushed out by their own unseeded random**
  (`pushOutOfBlocks`: a record popping out of a jukebox, drops landing in a wall).
  They can slide several blocks, so wall them in away from players and leave their
  position, motion and yaw out of the comparison (`play/jukeboxes`). `/give` hands items
  over as entities picked up a tick later: wait before using what it gives.
- **Space drops by their fall time**: a block dropped from 26 blocks up lands about 2 s
  later; console drops closer together race one landing against the next spawn.
- Resting items and orbs keep a horizontal motion that shrinks every tick, and their
  `Age` counts ticks lived: compare saved entities with tolerances on both.
- Spawn with `gamemode = "2"` in the properties (switch modes by console later): other
  modes randomize the spawn point with an unpinned random.
- Vanilla on Windows logs in cp1252; the harness reads non-UTF-8 lines as Latin-1 so
  `§` in team-colored names compares equal.
- When a server repeats the same packet a timing-dependent number of times (in vanilla
  too: e.g. position corrections while the bot keeps walking into a wall), list its type
  in a commented `[observe] collapse_repeats` entry: directly repeated identical copies
  count once, so the distinct packets and their order are still compared.
- `[observe] dedupe_runs` does the same for packets a server resends every tick while
  something stays open (e.g. the enchanting table's window properties, `"0x31"`).
- `[observe] drop_repeated` drops a packet equal to the last one of its type even with
  other packets in between, for packets resent unchanged on a timer while the scenario
  does other things (a carried map's icon packets, `"0x34"` in `play/framed_maps`):
  what changes still shows, in order.
- **Armor stands placed by a player turn their head and body by the world random** in
  whatever tick the placement lands. The parity view zeroes those two rotations
  (metadata entries 11 and 12, the only rotation-typed entries) in every Entity Metadata
  packet; ignore `Pose.Head` and `Pose.Body` of saved stands (`play/armor_stands`).
- **Rolling minecarts never stop exactly** on plain rails (4% drag a tick): end a track
  with an unpowered powered rail, which brakes carts to zero, or give a cart still
  creeping at the end a commented tolerance (`play/minecarts`). Their physics draws no
  random numbers, so everything else compares exactly.
- **A riding player's data holds its vehicle** (`Riding`, with the vehicle's random
  UUID as two longs that the UUID normalization does not see: ignore them), and a
  player reloaded from data with an `XpSeed` of 0 draws a new seed at random (ignore
  `XpSeed` after a restart; `play/riding`).
- **Entity windows carry a random UUID in their title** (a container minecart's display
  name has a `show_entity` hover and the UUID as insertion). Open Window is decoded so
  the title is a string the UUID normalization sees; a packet compared as hex keeps its
  UUIDs. Bot clicks without a predicted item (`!click` with no item fields) are rejected
  whenever the click moves something (Confirm Transaction false, the window resent),
  on both servers (`play/container_minecarts`).
- **Goldens are parsed with `float_roundtrip`**: without it `serde_json` misreads some
  doubles by an ulp, which showed as diffs between identical-looking numbers.
- **Entity metadata comes in `HashMap` order**: entities with 12 entries or fewer keep
  16 buckets, so entries 16-31 come between the low ones (a minecart: 0, 1, 17, 2, 18,
  ...). A metadata hex diff that only reorders entries means a missing
  `java_hash_order`.
- **Burning entities in fire never take the burn damage**: the fire counter rises by one
  with each move in the fire and drops by one with each update, so it never reaches the
  multiple of 20 that hurts. Only the fire block's own damage counts; time scenarios
  for that (`play/armor_stands`).
- **Tolerances on NBT lists go on the list itself** (`"file:<path>.dat.Pos" = 0.1`):
  lists of values are otherwise diffed line by line, where element tolerances never
  apply. Packet tolerances name a packet index, and the 1.7 view (`--protocol 5`) drops
  the 1.8-only packets before it, so a packet that needs a tolerance usually needs a
  second entry at its 1.7 index (`play/throwables`).
- Thrown projectiles take their spread from the entity's own unseeded random: aim at
  something whose hit tick is far from a tick boundary, and leave out anything whose
  outcome is a roll on that random (eggs hatching chicks, endermites from pearls: turn
  off `doMobSpawning`).
- Projectiles hitting a player directly are exact when the outcome does not depend on
  where the hit lands (splash potions give a direct hit the full effect): place the
  target so its box edge (grown by 0.3) falls mid-step, not at a tick boundary of the
  flight (`play/splash_potions`). Console steps run about 3 seconds after the first
  background bot joins (each background `play` step waits 1.5 s), so time bot actions
  that need given items after that.
- **A play session counts its `seconds` from the login, but `@<ms>` actions from the
  first position**, and vanilla takes about 4 ticks longer to place the player: what
  ages between an action and the logout (dropped items) differs by that much. Set
  `seconds_from_spawn = true` to count the session from the position too; it then ends
  on one of the bot's ticks, like the actions. The fuzzer always does.
- **Console lines sent together run in one tick** (`wait_ms = 0` on all but the last),
  which makes a burst of summons and a `kick` exact. A kicked player leaves in the next
  tick's network phase (`play/chunk_unload`).
- **Drops from paintings and item frames start half inside their wall**, and once the
  random start motion carries an item's middle into the wall block, `pushOutOfBlocks`
  shoots it back out at up to 0.3 blocks per tick: no distance is safe. Wall the drops
  in with glass and use the entities through it, within the 3 blocks allowed without
  line of sight (`play/hanging`).
- **`@e` matches are sorted by distance from the sender**, the console's being the
  origin: killing randomly placed drops logs them in a random order
  (`log_ignore_prefixes`).
- A painting's art is drawn from its own unseeded random among the arts that fit:
  leave its title out of the comparison (`player.packets[N].title`).
- Damage with a random part (critical arrows) can still be compared exactly when every
  outcome kills: bring the victim down to a few hearts first (instant damage from the
  console after the respawn protection) and turn off `naturalRegeneration`, whose timer
  phase is not reproducible (`play/bows`).
- **Saved doubles keep the sign of zero** (`-0d` is not `0d`): `x *= 0.0` on a negative
  motion leaves `-0.0`, and vanilla's `addVelocity(x, 0.0, z)` turns it back into `0.0`,
  so add the `0.0` wherever vanilla calls it (`play/furnace_minecarts`). A powered cart
  pressed against a block reaches a steady state, so its saved position and motion
  compare exactly however long the server runs. krol has no `/summon` yet: set up
  entities from items.
- **Console steps race the bot's join**: vanilla's slower join can let a console step meant
  for after a bot action land before it (a teleport made a later attack miss in
  `play/tnt_minecarts`). Leave seconds of margin, or avoid the console step. Explosions
  centered inside a rail (TNT minecarts) give every entity exposure 0: the rail's
  selection box stops each `getBlockDensity` ray.
- **Floating boats never settle**: the share of a boat under water comes in fifths, so
  it bobs forever, and its handleWaterMovement box is inside out, so its fall distance
  keeps growing. Give its height, vertical motion, fall distance, the boat distance stat
  and where its rider gets out small tolerances. Steer it into a bank below the crash
  speed (0.2975) so its horizontal resting place is exact (`play/boats`).
- **Animals need `spawn-animals=true`**: with it off (the usual scenario setup), vanilla
  sets every animal dead at its next update. Turn natural spawning off instead with
  `gamerule doMobSpawning false` (`play/mobs_no_ai`; `play/mobs_animals_off` covers the
  removal).
- **Knockback velocities follow the tracker's beat**: a knocked-back entity sends its
  decaying velocity every third tick, counted from when it started being tracked, so the
  values depend on which tick a bot's hit lands in. Give a mob a knockback resistance of
  1 through its `Attributes` tag when the hit itself is what matters (`play/mobs_no_ai`).
- **Mobs that a bot uses items on**: keep them without AI and silent (their own random
  then never shows) and behind glass in a cell of their own, so mobs with AI cannot
  push them (a pushed mob without AI gathers motion every tick). Glass hides them, so
  the bot reaches them only within 3 blocks. A pig in love is summoned with
  `{InLove:600}` under `krolpin` so that breeding starts at a known tick, and
  `krolpin spawns` pins the child (`play/mobs_breed`). Mind that feeding a piglet grows
  it by a tenth of its time left in whole seconds: pick its `Age` so the feeding tick
  lands mid-step. Mobs with AI look around up to the stop: ignore their final tracked
  `head`, and give ages a tolerance.
- **Spawn a mob on the bot's clock when bot actions steer it**: a mob summoned from
  the console runs on the console's clock, and its tracker sends velocities every third
  tick counted from its spawn, so what a bot does to it lands at a different point of
  that beat each run. A player saved riding a pig (`Riding` in a fixture player file)
  gets it back when joining: it is created with him, and `krolpin spawns` pins it.
  A pig that never walked cannot be steered (its move speed is only set by its move
  helper): let it panic first (`Fire` in its tag). Its AI still takes goals in 3-tick
  steps that a slower join shifts by one against the bot's clock, so leave out what
  depends on how many steps a goal ran (`play/mobs_riding`).
- **Keep console-clock mobs out of the bot's sight**: mobs spawned by a bot's actions
  run on the bot's clock and a summoned one on the console's, so their packets
  interleave differently every run. Summon it more than 80 blocks (an animal's tracking
  range) from the bot and compare only its saved data (`play/spawn_eggs`).
- **Mobs made new draw unpinned values**: `onInitialSpawn` (spawn eggs, `/summon`
  without a data tag) draws the follow range bonus from the mob's random before
  `krolpin spawns` pins it, and the modifier's UUID comes from `ThreadLocalRandom`:
  ignore the saved `Attributes[3].Modifiers` of each such mob.
