//! Status state: the server list ping (identical in protocols 5 and 47).

use super::{ConnectionState, finish};
use crate::buf::{PacketReader, PacketWriter};
use crate::{DecodeError, EncodeError};

/// Serverbound status packets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerboundStatus {
    /// `0x00` Status Request.
    Request,
    /// `0x01` Ping with an opaque payload (the client's clock).
    Ping(i64),
}

impl ServerboundStatus {
    /// Decodes a status-state frame body.
    ///
    /// # Errors
    ///
    /// [`DecodeError::BadPacketId`], field errors, or [`DecodeError::TrailingBytes`].
    pub fn decode(frame: &[u8]) -> Result<Self, DecodeError> {
        let mut r = PacketReader::new(frame);
        let id = r.varint()?;
        let packet = match id {
            0x00 => Self::Request,
            0x01 => Self::Ping(r.i64()?),
            other => return Err(DecodeError::BadPacketId(other)),
        };
        finish(&r, ConnectionState::Status, id)?;
        Ok(packet)
    }

    /// Encodes the packet (used by test clients).
    pub fn encode(&self) -> Vec<u8> {
        let mut w = PacketWriter::new();
        match self {
            Self::Request => w.varint(0x00),
            Self::Ping(payload) => w.varint(0x01).i64(*payload),
        };
        w.into_bytes()
    }
}

/// Clientbound status packets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientboundStatus {
    /// `0x00` Status Response carrying the server list JSON.
    Response(String),
    /// `0x01` Pong echoing the ping payload.
    Pong(i64),
}

impl ClientboundStatus {
    /// Encodes the packet.
    ///
    /// # Errors
    ///
    /// [`EncodeError::StringTooBig`] if the JSON exceeds 32767 bytes.
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut w = PacketWriter::new();
        match self {
            Self::Response(json) => {
                w.varint(0x00).string(json)?;
            }
            Self::Pong(payload) => {
                w.varint(0x01).i64(*payload);
            }
        }
        Ok(w.into_bytes())
    }

    /// Decodes a clientbound status frame body (used by test clients).
    ///
    /// # Errors
    ///
    /// [`DecodeError::BadPacketId`], field errors, or [`DecodeError::TrailingBytes`].
    pub fn decode(frame: &[u8]) -> Result<Self, DecodeError> {
        let mut r = PacketReader::new(frame);
        let id = r.varint()?;
        let packet = match id {
            0x00 => Self::Response(r.string(32767)?),
            0x01 => Self::Pong(r.i64()?),
            other => return Err(DecodeError::BadPacketId(other)),
        };
        finish(&r, ConnectionState::Status, id)?;
        Ok(packet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for p in [ServerboundStatus::Request, ServerboundStatus::Ping(-42)] {
            assert_eq!(ServerboundStatus::decode(&p.encode()).unwrap(), p);
        }
        for p in [
            ClientboundStatus::Response("{}".into()),
            ClientboundStatus::Pong(i64::MAX),
        ] {
            assert_eq!(ClientboundStatus::decode(&p.encode().unwrap()).unwrap(), p);
        }
        assert_eq!(
            ServerboundStatus::Ping(1).encode(),
            [0x01, 0, 0, 0, 0, 0, 0, 0, 1]
        );
    }

    #[test]
    fn request_with_body_is_rejected() {
        assert!(matches!(
            ServerboundStatus::decode(&[0x00, 0x00]),
            Err(DecodeError::TrailingBytes { .. })
        ));
        assert_eq!(
            ServerboundStatus::decode(&[0x05]),
            Err(DecodeError::BadPacketId(5))
        );
    }
}
