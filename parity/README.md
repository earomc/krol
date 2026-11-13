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
