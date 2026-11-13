# Third-party code and libraries

Everything listed here must be compatible with krol's `MIT OR Apache-2.0` license
(see `AGENTS.md` §4.0).

## Code derived from third-party sources

| Location | Origin | License |
|----------|--------|---------|
| `crates/krol-java/src/strict_math.rs` | Algorithm and constants of fdlibm 5.3 `e_log.c` (the specification of Java's `StrictMath.log`) | fdlibm notice: "Permission to use, copy, modify, and distribute this software is freely granted, provided that this notice is preserved." (notice preserved in the file) |

## Bundled runtime libraries

None yet. The Java compatibility layer (M8) will bundle the libraries Spigot 1.8.8 put on the
classpath; record each here with version and license.
