//! The krol game server.
//!
//! Configuration, game logic and connection handling. Everything observable must match
//! the vanilla 1.8.8 server (see `AGENTS.md` §5).

#![deny(missing_docs)]

pub mod auth;
pub mod config;
pub mod net;
pub mod status;
