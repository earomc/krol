# Vanilla parity suite

Runs the same scripted scenarios against the vanilla 1.8.8 server and krol, normalizes
what was observed and diffs it. See `AGENTS.md` §10.2 for the full design.

```sh
cargo xtask parity run [--filter status]    # krol vs golden (or live vanilla); writes parity/report.json
cargo xtask parity record [--rerecord]      # record vanilla observations into parity/golden/
cargo xtask parity diff status/modern_ping  # one scenario's diffs from the last report
cargo xtask parity coverage                 # feature coverage (parity/coverage.toml)
```

`run` takes `--protocol 5` (replay the play scenarios as 1.7 clients) and `--jobs N` (`-j N`,
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
until close), `login_start` (`protocol`, `name`). Observation paths listed in
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
timing. Per-entity randoms (`Entity.rand`: sound pitches, knockback rolls) are not pinned.

## Writing race-free scenarios

Vanilla and krol only compare equal when nothing depends on how the bot's packets fall
into server ticks. Rules learned from flaky scenarios:

- **Order only through the bot.** Console steps run on absolute time from the scenario
  start, bot actions on time since the bot's join. Anything that must happen in order
  relative to the bot (game mode switches, removing a block it just used) belongs to
  the bot itself, or to console steps *before* the `play` step. Prefer a fixture player
  file (`tools/fixtures/players.py`) over a console `/gamemode` switch.
- **`wait_ms` waits after a console command**, not before; use a `wait` step to delay
  one. A `background = true` play step is followed by a 1.5 s pause before the next step.
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
