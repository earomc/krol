# Vanilla parity suite

Runs the same scripted scenarios against the vanilla 1.8.8 server and krol, normalizes
what was observed and diffs it. See `AGENTS.md` §10.2 for the full design.

```sh
cargo xtask parity run [--filter status]    # krol vs golden (or live vanilla); writes parity/report.json
cargo xtask parity record [--rerecord]      # record vanilla observations into parity/golden/
cargo xtask parity diff status/modern_ping  # one scenario's diffs from the last report
cargo xtask parity coverage                 # feature coverage (parity/coverage.toml)
```

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
`KROL_JAR_TOOL`). It adds the console command `krolpin <seed> [randomTickSpeed] [ticks]`, which seeds
every world's `World.rand` and random tick generator (`updateLCG`), optionally sets the
random tick speed in the same tick and back to 0 after exactly that many ticks; krol
answers the same command in builds with the `parity` feature (xtask builds krol that
way, into `target/parity-build`). Issue `krolpin` from the console right before the
behavior to compare. With a player online every tick draws from the world random, so
start and stop random ticks through `krolpin` itself rather than separate `gamerule`
commands, whose tick depends on timing. Entity randoms (item arcs, sound pitches) are not
pinned yet.

## Writing race-free scenarios

Vanilla and krol only compare equal when nothing depends on how the bot's packets fall
into server ticks. Rules learned from flaky scenarios:

- **Order only through the bot.** Console steps run on absolute time from the scenario
  start, bot actions on time since the bot's join. Anything that must happen in order
  relative to the bot (game mode switches, removing a block it just used) belongs to
  the bot itself, or to console steps *before* the `play` step. Prefer a fixture player
  file (`tools/fixtures/players.py`) over a console `/gamemode` switch.
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
