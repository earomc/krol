//! Typed packets, grouped by connection state.
//!
//! Decoders take a complete frame body (packet id + fields) and require that the whole
//! body is consumed, like vanilla. Encoders return a frame body ready for
//! [`encode_frame`](crate::frame::encode_frame).

pub mod handshake;
pub mod login;
pub mod play;
pub mod status;

use crate::DecodeError;
use crate::buf::PacketReader;

/// The state of a connection, which determines how packet ids are interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionState {
    /// Initial state; only the handshake packet is valid.
    Handshaking,
    /// Server list ping.
    Status,
    /// Authentication and encryption setup.
    Login,
    /// In game.
    Play,
}

impl ConnectionState {
    /// Vanilla's name for the state (`EnumConnectionState`), used in error messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::Handshaking => "HANDSHAKING",
            Self::Status => "STATUS",
            Self::Login => "LOGIN",
            Self::Play => "PLAY",
        }
    }
}

/// Fails with [`DecodeError::TrailingBytes`] if `r` has unread data.
pub(crate) fn finish(
    r: &PacketReader<'_>,
    state: ConnectionState,
    id: i32,
) -> Result<(), DecodeError> {
    match r.remaining() {
        0 => Ok(()),
        extra => Err(DecodeError::TrailingBytes {
            state: state.name(),
            id,
            extra,
        }),
    }
}
