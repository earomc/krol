# AGENTS.md — Guide for coding agents working on krol

This file is the authoritative brief for any agent (human or AI) contributing to krol.
Read it fully before writing code. When this file and the README disagree, this file wins;
when this file is silent, ask the user instead of guessing on anything listed under
[Ask before](#ask-the-user-before).

---

## 1. Mission

krol is a **behavior-exact, drop-in Rust reimplementation of the vanilla Minecraft 1.8.8
server** that additionally accepts **1.7.6–1.7.10 clients**, and offers two plugin layers:
a native **Rust plugin API** and a **Java Bukkit 1.8.8 compatibility layer**.

"Drop-in" means: take an existing vanilla 1.8.8 server directory, replace
`java -jar server.jar` with `./krol`, and everything — world, configs, ops, bans,
whitelist, player data — keeps working, and players cannot tell the difference.

## 2. Reference versions

| Concern              | Reference                                                          |
|----------------------|--------------------------------------------------------------------|
| Game logic / backend | Vanilla **server 1.8.8** (1.8.9 is protocol- and server-identical, see below) |
| Canonical protocol   | **47** (1.8 – 1.8.9) — the internal model of the server            |
| Secondary protocol   | **5** (1.7.6 – 1.7.10) — translated to/from 47 at the network edge |
| Java plugin API      | **Bukkit 1.8.8**, CraftBukkit/NMS **`v1_8_R3`**                     |
| Version list         | https://minecraft.wiki/w/Protocol_version#List_of_protocol_versions |
| Packet docs          | wiki.vg archive pages for protocol 47 and protocol 5 (on minecraft.wiki) |

**Why 1.8.8 and not 1.8.9:** the 1.8.9 bug fixes were client-side. Diffing the official
server jars (2026-11-15: same 5765 files and obfuscated names; 262 classes recompiled)
leaves only the version string changing from `1.8.8` to `1.8.9`, in the startup log, the
handshake's outdated client/server messages, the status ping's version name and crash
reports, plus one client-only line in `en_US.lang`. Bukkit/CraftBukkit/Spigot never made
a 1.8.9 build (`v1_8_R3` is their last 1.8), so krol keeps reporting 1.8.8 like the
servers it replaces.

## 3. Scope

### In scope
- Connection states Handshake, Status, Login, Play for protocols 47 and 5.
- Legacy (pre-Netty, `0xFE`) server list ping, answered like vanilla 1.8.8 does.
- zlib packet compression (protocol 47 only — protocol 5 has none) and AES/CFB8 encryption.
- `online-mode=true` authentication against Mojang's session server; offline mode with
  vanilla's offline UUID derivation.
- All vanilla gameplay: blocks, items, inventories, crafting, entities and AI, redstone,
  physics, weather, day cycle, dimensions, scoreboards, achievements/stats.
- All vanilla console/player commands with identical syntax, permissions and output.
- **Seed-exact world generation** (same seed ⇒ identical chunks to vanilla).
- RCON and Query protocols.
- Both plugin layers (see [§8](#8-plugin-layers)).

### Non-goals
- Protocol 4 (1.7.2–1.7.5) or any version newer than 1.8.x.
- Gameplay "improvements", bug fixes or performance tweaks that change observable behavior.
  (Server-side performance work that keeps behavior identical is welcome.)

## 4. Source and legal policy

*Not legal advice — this is the project's working policy.*

### 4.0 License
- All krol code is licensed **`MIT OR Apache-2.0`** (`LICENSE-MIT`, `LICENSE-APACHE`).
  Every crate inherits `license = "MIT OR Apache-2.0"` from `[workspace.package]`;
  every Java source file starts with `// SPDX-License-Identifier: MIT OR Apache-2.0`.
  Contributions are accepted under the same terms.
- **No copyleft in krol.** No GPL/AGPL code or dependencies anywhere. LGPL is allowed only
  for runtime Java libraries plugins expect, shipped as **unmodified separate jars**
  (e.g. Ebean 2.8.1, exposed by `JavaPlugin#getDatabase`). A `cargo-deny` license check
  (`deny.toml`) enforces this for Rust dependencies in CI.

### 4.1 Vanilla (Mojang) code
- You **may** read decompiled, deobfuscated (MCP-mapped) vanilla 1.8.8 server code as a
  **behavior reference** — it is the specification.
- Keep decompiled sources, the vanilla jar and any Mojang assets **outside the repository**
  (local `/reference/` is git-ignored). **Never commit them.**
- Write **original Rust** from understood behavior. Do not transliterate Java line by line,
  and do not copy Mojang comments, identifiers-as-prose, or assets. Literal strings and
  constants are fine only where the protocol or observable behavior requires them
  (e.g. chat messages, translation keys, magic numbers).
- You **may** cite vanilla names to aid traceability:
  `// Vanilla: EntityPlayer#onUpdate (MCP)`.

### 4.2 Bukkit / CraftBukkit / Spigot code (independent implementation)
Bukkit is GPLv3 and CraftBukkit LGPLv3. krol stays permissive by **reimplementing the API
without using any of their code** — the same model as for vanilla: reference yes, copying no.
Reimplementing an API's interface is the part covered by Google v. Oracle (US, 2021) and
SAS v. WPL / Directive 2009/24/EC Art. 1(2) (EU).

- **Allowed:**
  - Reading Bukkit, CraftBukkit and Spigot 1.8.8 source and Javadoc to understand
    behavior and how to bridge it onto the Rust core.
  - Reproducing **public API signatures exactly**: package/class/member names, descriptors,
    modifiers, type hierarchy, API constant values, enum constant names and order — binary
    compatibility requires it. This applies equally to the `org.bukkit.craftbukkit.v1_8_R3`
    and `net.minecraft.server.v1_8_R3` shims.
- **Forbidden:**
  - Copying, transliterating or lightly editing any Bukkit/CraftBukkit/Spigot method
    bodies, private helpers, comments or Javadoc text.
  - Committing their jars or sources (keep them in git-ignored `/reference/`).
- **Practice:**
  - Understand first, then close the reference and write the implementation against krol's
    own architecture (JNI calls into the Rust core). The backend is entirely different, so
    bodies should naturally differ.
  - Write all Javadoc fresh, in your own words.
  - Settle subtle behavior (YAML formatting, `plugin.yml` parsing, event dispatch order,
    class loading) with parity tests against CraftBukkit 1.8.8.
- **Caveat:** reading the source forgoes strict clean-room *evidence* of independent
  creation. Copyright protects expression, so the safeguard is the "no copied bodies" rule
  above — follow it rigorously.
- **Bundled runtime libraries** match what Spigot 1.8.8 put on the classpath (Guava 17,
  Gson 2.2.4, SnakeYAML, commons-lang 2.6, json-simple — Apache-2.0; Ebean — LGPL,
  unmodified). Record them with versions and licenses in `docs/third-party.md`.

## 5. The fidelity rule

> **Code may differ from vanilla. Observable behavior may not.**

Anything a client, a plugin, a world file, a log reader or a player can observe must match
vanilla 1.8.8. Known traps — handle them deliberately:

- **Tick model.** One single-threaded game loop at 20 TPS, preserving vanilla's ordering
  (per world: scheduled ticks, block/random ticks, entities, tile entities; then
  player/network processing — follow `MinecraftServer#tick` / `WorldServer#tick`).
  Networking may be async (tokio), but packets are queued to and processed on the tick
  thread exactly where vanilla processes them.
- **Java numeric semantics.** Java `int`/`long` arithmetic wraps — use `wrapping_*` /
  `std::num::Wrapping` wherever vanilla can overflow (debug-mode panics are bugs).
  Reproduce Java's float→int casts, `MathHelper.floor_*`, the `MathHelper` 65536-entry
  sin/cos table, and use `f32` vs `f64` exactly where vanilla uses `float` vs `double`.
  Never "upgrade" precision.
- **Randomness.** Use the bit-exact `java.util.Random` port in `krol-java`. Keep every
  vanilla `Random` instance, its seeding, and the exact order and kind of calls on it —
  world generation, mob AI, drops and enchanting all depend on it.
- **Iteration order.** Where vanilla iterates a `HashMap`/`HashSet` and order is
  observable, reproduce Java's order (ported `String.hashCode`, `HashMap` bucket/resize
  behavior) or an equivalent ordered structure. Don't use Rust's randomized `HashMap` there.
- **Bugs and quirks are spec.** Redstone update order, quasi-connectivity, dupes, collision
  oddities, off-by-ones — preserve them and mark each with `// Vanilla quirk: <what/why>`.
- **Text output.** Chat JSON, kick messages, command feedback and log lines must match
  byte for byte (including formatting and translation keys).
- **Timing.** Keep-alive intervals, timeouts, autosave interval, and chunk send order/rate
  must match vanilla.

## 6. Drop-in compatibility checklist

krol must read, write and honor exactly what vanilla does:

- `server.properties` — same keys, defaults, parsing, and rewrite-on-start behavior.
- `eula.txt`, `ops.json`, `whitelist.json`, `banned-players.json`, `banned-ips.json`,
  `usercache.json`, `server-icon.png`.
- World directory: Anvil `.mca` regions, `level.dat`, `playerdata/<uuid>.dat`,
  `stats/<uuid>.json`, `data/` (maps, villages, scoreboard, …), `DIM-1/` and `DIM1/`.
- Console command input on stdin, `logs/latest.log` with gzip rotation, vanilla log format.
- RCON and Query when enabled in `server.properties`.
- With the Bukkit layer enabled: `bukkit.yml`, `commands.yml`, `help.yml`,
  `permissions.yml`, `plugins/` — matching CraftBukkit 1.8.8.

## 7. Architecture

Initial Cargo workspace layout. Create crates in this shape; **propose restructuring via an
ADR in `docs/adr/` before deviating.**

```
crates/
  krol-java/      Java-semantics primitives: java.util.Random, String.hashCode,
                  MathHelper, Java HashMap ordering, modified UTF-8
  krol-nbt/       NBT read/write (uncompressed, gzip, zlib)
  krol-protocol/  VarInt/VarLong, packet codecs for 47, 5<->47 translation,
                  compression, encryption, legacy ping
  krol-world/     Anvil region I/O, chunk sections, block id + metadata model
  krol-server/    Game logic: tick loop, worlds, entities, AI, worldgen,
                  commands, config files, RCON/Query; implements krol-api
  krol-api/       Rust plugin API (public, stable, fully documented)
  krol-bukkit/    JNI bridge to the embedded JVM (feature `bukkit`)
  krol-testbot/   Headless scriptable client for protocols 47 and 5
  krol/           The server binary
java/             Bukkit 1.8.8 API + CraftBukkit/NMS v1_8_R3 shims (Gradle/Maven)
parity/           Vanilla parity suite: scenarios, golden data, vanilla agent
xtask/            `cargo xtask` automation (parity runner, codegen)
docs/             ADRs, compat trackers, design notes
```

Dependency rules:
- `krol-java`, `krol-nbt`, `krol-protocol`, `krol-world` contain **no game logic**.
- `krol-server` depends on them and implements `krol-api`.
- Plugin layers (`krol-bukkit`, Rust plugins) depend **only on `krol-api`**
  (plus whatever internal hooks `krol-api` deliberately exposes for NMS shims).
- Protocol 5 exists only inside `krol-protocol`'s translation layer; game logic only ever
  sees protocol-47 semantics.

## 8. Plugin layers

### 8.1 Java / Bukkit layer (`krol-bukkit` + `java/`)

- The server embeds a JVM through JNI (`jni` crate), compiled behind cargo feature
  `bukkit` and started at runtime only when `plugins/*.jar` exist.
- Expose the **entire public Bukkit 1.8.8 API with identical signatures and semantics**.
  Event types, firing points, ordering, priorities and cancellation behavior must match
  CraftBukkit 1.8.8.
- **CraftBukkit and NMS internals must be supported too**, because many real plugins use
  them. Provide shim classes under the real package names
  `org.bukkit.craftbukkit.v1_8_R3.*` and `net.minecraft.server.v1_8_R3.*`, backed by the
  Rust core. Prioritize by real-world usage, e.g.:
  - `CraftPlayer#getHandle`, `EntityPlayer.playerConnection.sendPacket(...)`, `Packet*` classes
  - `NBTTagCompound`, `NBTCompressedStreamTools`, `CraftItemStack.asNMSCopy/asBukkitCopy`
  - `MinecraftServer.getServer()`, `CraftServer`, `CraftWorld#getHandle`
  - Netty channel access used by packet-listener libraries (ProtocolLib-style injection)
  - Reflection: field and method names must match the 1.8.8 (Spigot-mapped/obfuscated)
    names that plugins actually reflect on.
- **Extra Java APIs are allowed** when they help plugins run (e.g. widely used
  Spigot/Paper 1.8 additions such as `Player.Spigot`). They must be **purely additive** —
  never alter a Bukkit 1.8.8 signature or behavior.
- All Java code is krol's own, written under the rules in [§4.2](#42-bukkit--craftbukkit--spigot-code-independent-implementation).
- **API compatibility check in CI:** a japicmp-style comparison of krol's Java jar against
  the official `bukkit-1.8.8-R0.1-SNAPSHOT` API jar (downloaded at test time from the
  Spigot Maven repository, or `KROL_BUKKIT_API_JAR`; never committed). Any missing or
  differing public class/member fails the build; additive extras are listed in an allowlist.
- Track progress in `docs/bukkit-compat.md` (supported / partial / missing per class and
  member) and maintain a corpus of real plugins used for plugin parity tests (§10.2).

### 8.2 Rust plugin API (`krol-api`)

- Plugins are crates implementing `krol_api::Plugin`, **linked at compile time** into a
  custom server build (via cargo features or a `krol-build` template crate). This is
  intentionally not drop-in; document the build workflow clearly.
- The API **may improve on Bukkit 1.8.8's design**, drawing on modern Bukkit/Paper:
  strongly typed event bus with priorities and cancellation, Adventure-style text
  components, sync/async scheduler, Brigadier-like command builder, persistent data
  container, builder-style construction, `Result`-based errors.
- **Hard requirement: everything possible with the Bukkit 1.8.8 API must be possible with
  `krol-api`.** Maintain `docs/bukkit-to-rust.md` mapping every Bukkit capability to its
  Rust equivalent. A missing mapping is a bug.
- **It must be perfectly documented:**
  - `#![deny(missing_docs)]` and `#![deny(rustdoc::broken_intra_doc_links)]` on the crate.
  - Every public item documents: what it does, its semantics (which thread / tick phase it
    runs in, how it corresponds to vanilla and to Bukkit), errors and panics, and includes
    a **compiling doctest example**.
  - A crate-level guide: writing a first plugin, plugin lifecycle, events, scheduler and
    threading model, commands, permissions, building a server with plugins.
  - A runnable example plugin under `examples/`.
  - CI: `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p krol-api` and
    `cargo test --doc -p krol-api` must pass.
- Changes to public `krol-api` signatures require user approval and a `CHANGELOG.md` entry.

## 9. Code conventions

- Stable Rust, edition 2024. Must pass `cargo fmt --check` and
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- `unsafe` only in `krol-bukkit` (JNI) and `krol-java` if strictly needed, each block with
  a `// SAFETY:` comment.
- No `unwrap()`/`expect()` on data from the network, disk or plugins. Use `thiserror` in
  libraries, `anyhow` only in the binary and xtask. Log with `tracing`.
- Malformed client input must never panic the server: handle it exactly as vanilla does
  (usually a disconnect with vanilla's message).
- Prefer explicit, readable code over clever abstractions; game logic should be easy to
  compare against vanilla behavior.
- Name types/functions idiomatically in Rust; reference vanilla names in doc comments.
- No magic values: use named constants for block and item ids (`krol-server`
  `game/ids.rs`), packet actions, slot indices and tick durations, and convert literals in
  code you touch.

## 10. Testing

### 10.1 Unit and golden tests
- Unit tests in every crate.
- Golden-byte encode/decode tests for every packet in protocols 47 and 5, and for each
  5↔47 translation.
- `krol-java` is tested against values produced by a real JVM (e.g. the first N outputs of
  `new Random(seed).nextInt()`, `String.hashCode`, `MathHelper.sin`).
- NBT/Anvil round-trip tests against real vanilla-written files.

### 10.2 Vanilla parity suite (`parity/`, AI-drivable)

The parity suite is the main way to prove fidelity, and is designed to be driven by AI
agents in a closed loop. Run it with `cargo xtask parity <cmd>`.

**Principle:** the same scenario runs against the real vanilla 1.8.8 server and against
krol with identical inputs. Observations are normalized and diffed.

**Inputs (never committed):**
- `KROL_VANILLA_JAR` — path to the vanilla 1.8.8 server jar (Java 8 runtime required).
- `KROL_CRAFTBUKKIT_JAR` — path to a CraftBukkit/Spigot 1.8.8 jar for plugin parity.
- `KROL_BUKKIT_API_JAR` — optional local Bukkit 1.8.8 API jar for the API compatibility check.

**Scenarios** — declarative TOML in `parity/scenarios/<area>/<name>.toml`:
- `[setup]` — world seed or template from `parity/worlds/`, `server.properties`
  overrides, plugins to load.
- `[[steps]]` — testbot actions (connect with protocol 47 or 5, move, look, dig, place,
  use item, click window, chat), console commands, `wait_ticks`.
- `[[observe]]` — per-bot packet streams, world region / NBT snapshots, `level.dat`,
  playerdata, console log, command output; compared exactly, except paths listed in
  `ignore` and absolute numeric tolerances in `[observe.tolerance]` (for values that
  depend on how packets interleave with ticks; each with a comment saying why).
- Complex cases may be written as Rust scenarios in the same harness.

**Determinism:**
- Fixed seeds and committed world templates.
- A test-only Java agent (`parity/vanilla-agent/`) pins vanilla's unseeded `Random`
  instances and controls its clock; krol exposes equivalent hooks behind cargo feature
  `parity`. This makes most observations comparable exactly.
- Normalization strips or remaps entity IDs, timestamps, keep-alive IDs and random UUIDs.

**Commands:**
- `parity run [--filter <glob>] [--protocol 47|5]` — run scenarios, diff, report.
- `parity record [--rerecord]` — capture vanilla observations into `parity/golden/` so most
  runs don't need the vanilla jar.
- `parity diff <scenario>` — show the detailed diff for one scenario.
- `parity fuzz --seed N --steps M` — random action sequences; failing cases are shrunk and
  written out as new scenario files.
- `parity worldgen --seeds N --radius R` — block-by-block chunk comparison across seeds.
- `parity coverage` — report covered / uncovered / failing features.
- `parity plugins` — run the plugin corpus on CraftBukkit 1.8.8 vs krol and diff effects.

**Agent-friendly output:** `parity/report.json` with, per scenario: status, first
diverging tick, a minimal structured diff (packet field paths, block positions with
expected/actual state, NBT paths), relevant log excerpts, and a suggested subsystem tag.
A short human summary goes to stdout; exit code is non-zero on any failure.

**Coverage manifest:** `parity/coverage.toml` lists vanilla features — every packet,
command, block/item/entity behavior, worldgen feature, config file — and the scenarios
covering them.

**The agent loop:**
1. `cargo xtask parity coverage` — pick an uncovered or failing entry.
2. Write or extend a scenario for it.
3. `cargo xtask parity record --filter <it>` against vanilla.
4. Implement in krol.
5. `cargo xtask parity run --filter <it>` and read `parity/report.json`.
6. Iterate until green; commit the scenario, golden data and implementation together.

Every gameplay change must name the parity scenarios it adds or turns green.

## 11. Workflow

- Small, focused changes; one subsystem per change where possible.
- Build bottom-up, following the roadmap below.
- Keep `docs/` (ADRs, `bukkit-compat.md`, `bukkit-to-rust.md`) and this file current when
  decisions change.
- Commit messages: imperative, scoped — e.g. `protocol: translate 1.7 chunk bulk packet`.
- Before finishing a change: `cargo fmt`, `cargo clippy`, `cargo test --workspace`, and the
  relevant parity scenarios.
- Parity scope: run the scenarios of the feature being built (`--filter`) plus the few
  that share the touched code, in both protocols where they have play sessions. Pick
  them by what the change can make observable (grep `parity/coverage.toml` and the
  scenarios for the packets, commands or files involved), not by how many files it
  touches: a helper replacing many call sites, or a new kind of saved data, still only
  needs the scenarios that observe it. Run the full suite (`cargo xtask parity run`, then
  `--protocol 5`; ~5 min each) only for changes every scenario passes through (NBT or
  protocol encoding, tick order, networking, inventory rules, normalization, the
  harness itself) or before larger milestones, not once per feature. A scenario that
  "failed in parallel, passed alone" is a timing race to make robust, not a pass. Say in
  the summary which scenarios ran. `parity record` runs one JVM at a time.
- CI (`.github/workflows/ci.yml`) is manual for now (`gh workflow run CI`); don't wait for it.
  Run fmt, clippy, tests and docs locally before each commit, and `cargo deny check` when
  dependencies change.
- Leave nothing undone unrecorded: every gap or known deviation goes into
  `docs/roadmap.md` (and a `TODO(M<n>)` at the spot in the code), and lessons about writing
  scenarios into `parity/README.md`.

### Ask the user before
- Changing the license.
- Adding any GPL/AGPL/LGPL code or dependency.
- Changing the source policy (§4, including relaxing §4.2) or the fidelity rule (§5).
- Supporting additional protocol versions.
- Changing public `krol-api` signatures.
- Restructuring the workspace (write an ADR first).
- Adding heavyweight dependencies or a different async runtime.

## 12. Roadmap

| Milestone | Goal |
|-----------|------|
| M0 | Workspace skeleton (`[workspace.package] license`), `krol-java`, `krol-nbt`, CI (fmt, clippy, tests, docs, `cargo-deny` licenses) |
| M1 | Server list ping (modern + legacy) for 47 and 5; parity harness skeleton (testbot, runner, `report.json`, ping scenarios) |
| M2 | Login: offline/online mode, compression, encryption, config + ops/whitelist/bans |
| M3 | Load an existing vanilla world; spawn, movement, chunk streaming, chat, commands basics |
| M4 | Block interaction, inventories, crafting, survival basics, world saving |
| M5 | Entities, mobs and AI, redstone, physics |
| M6 | Seed-exact world generation for all dimensions |
| M7 | Rust plugin API (`krol-api`) with complete documentation |
| M8 | Embedded JVM + Bukkit 1.8.8 API layer, API compatibility check in CI |
| M9 | CraftBukkit/NMS `v1_8_R3` shims, driven by the plugin corpus |

The parity harness arrives in M1 on purpose: every later milestone is developed against it.

[`docs/roadmap.md`](docs/roadmap.md) lists every vanilla feature by package with its status
(done / partial / missing) and starts with a **Next up** queue of the work to pick up
next; keep both current along with the table below.

### Current status (keep this updated)

| Area | State |
|------|-------|
| M0 | Done: workspace, CI, `cargo-deny`, `krol-java` (Random, StrictMath.log, String.hashCode, modified UTF-8, MathHelper, Hashtable/HashMap order, Properties, Gson writer/parser — all JVM-verified via `crates/krol-java/tools/JavaVectors.java`), `krol-nbt` |
| M1 | Done: status (modern + legacy) for 47 and 5, `server.properties`, EULA, vanilla logging, parity runner (`run`, `record`, `diff`, `coverage`). Not yet: `fuzz`, `worldgen`, `plugins` commands. The vanilla Java agent (`parity/vanilla-agent`, `krolpin`, `pin_random`) pins the world random, the random tick generator and `Math.random()`, and can run a command in the pinning tick; per-entity randoms and the clock are not pinned yet |
| M2 | Done: offline/online login, encryption, compression, ops/whitelist/bans, `usercache.json`, duplicate logins |
| M3 | Mostly done: 20 TPS game thread, joining existing worlds (byte-identical join sequence), vanilla chunk tracking and chunk lifecycle (ticked/terrain/light population), movement handling (teleport confirm, moved too quickly, floating, illegal position), chat, weather cycle, autosave, the command system with ~35 commands implemented and all 58 listed, entity selectors (`@p`/`@a`/`@r`/`@e`), the scoreboard (objectives, scores, teams, display slots, `/scoreboard`, `/trigger`, `data/scoreboard.dat`; kill criteria missing). permissions, operator notifications, op/ban/whitelist commands and usercache.json are done. 1.7 clients play through the protocol-5 translation layer (`krol-protocol::translate`; `cargo xtask parity run --protocol 5` replays the play scenarios as 1.7 clients against the 1.8 recordings). Missing: Mojang profile lookups for unknown names |
| M4 | In progress: stored inventories (verified 1.8.8 item id table), inventory window clicks (all click modes, armor slots, rejected predictions), stats files, creative building, block placement rules (orientation, slabs, supports, doors), block activation (doors, trapdoors, gates), survival digging (vanilla hardness/material table, tool rules, 70% rule, break animations, mining stats), block drops for the common blocks, dropping items, fall damage, food and natural regeneration, death and respawn, world save checked against vanilla, crafting (recipe table extracted from vanilla, 2x2 grid, crafting table, all special recipes (fireworks and banners too); item properties table extracted from vanilla), chests (tile entities, single/double windows, shared contents, block events, lid sounds, contents dropped when broken), furnaces (fuel, smelting, lit/unlit swap, window properties), signs (placement, editing, tile entity data sent with chunks and block changes), ender chests (shared ender inventory, lid events), trapped chest power, enchanting tables, anvils (repair window, en_US item names). Missing: maps, the other containers, other item-placed blocks, the remaining special block drops |
| M5 | Started: entity tracker for players and dropped items (spawn, movement, head rotation, metadata, arm swing, equipment, velocity, destroy); server-side player physics (vanilla `moveEntity` with block collision boxes, the living update on every movement packet, collision rollback and "moved wrongly", step sounds, jumps, movement statistics, exhaustion, fall distance, players pushing each other). item entities (physics, merging, pickup, despawn); items, orbs, falling blocks and primed TNT saved with their chunk and loaded back (other saved entity kinds kept as NBT); experience orbs (furnace output, ores); block updates (neighbor notifications, scheduled ticks with TileTicks persistence) with plants/torches/signs/ladders popping off and levers/buttons; falling sand, gravel and anvils (hurting entities, damaged on landing); lighting (sky and block light updates, height maps, gap rechecks); redstone basics (power, wire, torches, lamps, repeaters, comparators reading chests and furnaces, doors/trapdoors/fence gates, pistons, note blocks, pressure plates, hoppers, dispensers and droppers); entities update in vanilla order; random block ticks (leaf decay, saplings growing every tree kind, vines growing, grass, farmland, crops, stems, sugar cane, cactus, ice/snow melting); flowing water and lava with mixing, water pushing players and items; drowning; dead players dropping experience; fire and flint and steel; burning players and lava damage; tool wear when mining; planting seeds, sugar cane and cake; hoes and bone meal (crops, stems, saplings, grass with biome flowers, tall grass, double plants, mushrooms into huge mushrooms, cocoa); cocoa beans (placing, growing, dropping); double plants placed and breaking as a pair; mushrooms spreading and popping off in light; suffocation; cobwebs and soul sand slowing entities; farmland trampling, redstone ore lighting up, ice melting into water when broken, slime slowing walkers; collision shapes of every block (stairs, fences, walls, panes, doors, trapdoors, ...); TNT and explosions (block rays, damage, knockback, chain reactions); beds (sleeping, night skip, spawn points); drinking potions and filling bottles; brewing stands; armor; eating (food effects, golden apples, milk, cake); status effects and /effect; attribute modifiers (sprinting, effects); biome weather (temperature noise, rain/snow); buckets (server-side block ray trace). Missing: all other entities, AI, the other redstone components (special dispense behaviors, rails, tripwires, daylight sensors), pistons moving entities, splash potions |
| Goldens | Recorded from vanilla 1.8.8 on Java 8 (Windows). 99/99 scenarios pass (and the 87 with play sessions also as 1.7 clients). The runner builds krol with the `parity` profile (optimized, overflow checks and debug assertions on). Re-record with `KROL_VANILLA_JAR=reference/minecraft_server.1.8.8.jar KROL_JAVA=<java 8> cargo xtask parity record --rerecord` after changing scenarios or normalization |

A decompiled, MCP-named vanilla 1.8.8 server (reference only, §4.1) can be built into the
git-ignored `reference/src/` from the official jar with Vineflower, SpecialSource and the MCP
1.8.8 mappings (`reference/tools/`). Read it to understand behavior; never copy from it, and
still verify with parity recordings.

Without an existing world (generation is M6) players are disconnected after login with
"krol: no world is loaded".
