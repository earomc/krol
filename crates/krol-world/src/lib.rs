//! Minecraft 1.8 world storage: Anvil region files, chunk sections and `level.dat`.
//!
//! This crate contains no game logic; it only converts between files and in-memory
//! structures, preserving everything vanilla stores (see `AGENTS.md` §6).

#![deny(missing_docs)]

pub mod chunk;
pub mod region;
