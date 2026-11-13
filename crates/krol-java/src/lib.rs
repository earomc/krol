//! Bit-exact ports of the Java platform behavior that the vanilla 1.8.8 server relies on.
//!
//! Vanilla Minecraft's observable behavior (world generation, mob AI, drops, iteration
//! order, ...) depends on details of the Java runtime: the `java.util.Random` LCG,
//! `StrictMath`, `String.hashCode`, Java's modified UTF-8 and the vanilla `MathHelper`
//! lookup tables. This crate reproduces them exactly so that the rest of krol can match
//! vanilla bit for bit (see `AGENTS.md` §5).
//!
//! Everything here is verified against values produced by a real JVM; see
//! `tests/java_vectors.rs` and the generator in `tools/JavaVectors.java`.

#![deny(missing_docs)]

pub mod gson;
mod hashtable;
pub mod math;
pub mod modified_utf8;
pub mod properties;
mod random;
mod strict_math;
mod string;

pub use hashtable::JavaHashtable;
pub use random::JavaRandom;
pub use strict_math::strict_log;
pub use string::java_string_hash;
