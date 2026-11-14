//! Translation between protocol 47 (the server's model) and protocol 5 (1.7.6–1.7.10).
//!
//! Game logic only ever sees protocol-47 packets. For a 1.7 client the network layer
//! runs every outgoing packet through [`V5Translator::clientbound`] and every incoming
//! frame through [`V5Translator::serverbound`].

mod v5;
pub mod v5_ids;

pub use v5::V5Translator;
