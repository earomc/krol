//! Play state packets (grows with M3+).

use crate::buf::{PacketReader, PacketWriter};
use crate::{DecodeError, EncodeError, ProtocolVersion};

/// Clientbound play packets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientboundPlay {
    /// `0x40` Disconnect with a JSON chat component reason (same id in 5 and 47).
    Disconnect(String),
}

impl ClientboundPlay {
    /// Encodes the packet for `version`.
    ///
    /// # Errors
    ///
    /// [`EncodeError::StringTooBig`] for oversized strings.
    pub fn encode(&self, _version: ProtocolVersion) -> Result<Vec<u8>, EncodeError> {
        let mut w = PacketWriter::new();
        match self {
            Self::Disconnect(json) => {
                w.varint(0x40).string(json)?;
            }
        }
        Ok(w.into_bytes())
    }

    /// Decodes a clientbound play frame body (used by test clients). Unknown packets
    /// yield `Ok(None)` so clients can skip what they do not model yet.
    ///
    /// # Errors
    ///
    /// Field errors for known packets.
    pub fn decode(frame: &[u8], _version: ProtocolVersion) -> Result<Option<Self>, DecodeError> {
        let mut r = PacketReader::new(frame);
        Ok(match r.varint()? {
            0x40 => Some(Self::Disconnect(r.string(32767)?)),
            _ => None,
        })
    }
}
