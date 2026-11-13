//! Minecraft network protocol codecs for protocol 47 (1.8.x) and protocol 5
//! (1.7.6 – 1.7.10).
//!
//! Protocol 47 is krol's canonical model: game logic only ever sees protocol-47
//! semantics, and protocol 5 is translated at this layer (see `AGENTS.md` §7).
//!
//! Layers, bottom-up:
//! - [`buf`]: field primitives (VarInt, strings, positions, ...) with vanilla's limits and
//!   error behavior,
//! - [`frame`]: VarInt length-prefixed framing,
//! - [`packets`]: typed packets per connection state.

#![deny(missing_docs)]

pub mod buf;
mod error;
pub mod frame;
pub mod packets;
mod version;

pub use error::{DecodeError, EncodeError};
pub use version::ProtocolVersion;
