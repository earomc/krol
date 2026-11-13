//! The krol game server.
//!
//! Configuration, game logic and connection handling. Everything observable must match
//! the vanilla 1.8.8 server (see `AGENTS.md` §5).

#![deny(missing_docs)]

pub mod auth;
pub mod chat;
pub mod config;
pub mod game;
pub mod lang;
pub mod net;
pub mod status;
pub mod usercache;
pub mod userlist;
