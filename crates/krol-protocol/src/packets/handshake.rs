//! Handshaking state (identical in protocols 5 and 47).

use super::{ConnectionState, finish};
use crate::buf::{PacketReader, PacketWriter};
use crate::{DecodeError, EncodeError};

/// The handshake packet (serverbound `0x00`), the first packet of every modern connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    /// The client's protocol number (5 or 47 for supported clients).
    pub protocol_version: i32,
    /// Host name the client connected to (at most 255 characters).
    pub server_address: String,
    /// Port the client connected to.
    pub server_port: u16,
    /// Requested next state: `1` = status, `2` = login. Other values are passed through so
    /// the server can react like vanilla.
    pub next_state: i32,
}

impl Handshake {
    /// Packet id.
    pub const ID: i32 = 0x00;

    /// Decodes a handshaking-state frame body.
    ///
    /// # Errors
    ///
    /// [`DecodeError::BadPacketId`] for any id other than `0x00`, field errors, or
    /// [`DecodeError::TrailingBytes`].
    pub fn decode(frame: &[u8]) -> Result<Self, DecodeError> {
        let mut r = PacketReader::new(frame);
        let id = r.varint()?;
        if id != Self::ID {
            return Err(DecodeError::BadPacketId(id));
        }
        let packet = Self {
            protocol_version: r.varint()?,
            server_address: r.string(255)?,
            server_port: r.u16()?,
            next_state: r.varint()?,
        };
        finish(&r, ConnectionState::Handshaking, id)?;
        Ok(packet)
    }

    /// Encodes the packet (used by test clients).
    ///
    /// # Errors
    ///
    /// [`EncodeError::StringTooBig`] for an oversized address.
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut w = PacketWriter::new();
        w.varint(Self::ID).varint(self.protocol_version);
        w.string(&self.server_address)?;
        w.u16(self.server_port).varint(self.next_state);
        Ok(w.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_bytes() {
        let hs = Handshake {
            protocol_version: 47,
            server_address: "localhost".into(),
            server_port: 25565,
            next_state: 1,
        };
        let bytes = hs.encode().unwrap();
        assert_eq!(
            bytes,
            [
                0x00, 0x2f, 0x09, b'l', b'o', b'c', b'a', b'l', b'h', b'o', b's', b't', 0x63, 0xdd,
                0x01
            ]
        );
        assert_eq!(Handshake::decode(&bytes).unwrap(), hs);
    }

    #[test]
    fn rejects_wrong_id_and_trailing() {
        assert_eq!(Handshake::decode(&[0x01]), Err(DecodeError::BadPacketId(1)));
        let mut bytes = Handshake {
            protocol_version: 5,
            server_address: String::new(),
            server_port: 1,
            next_state: 2,
        }
        .encode()
        .unwrap();
        bytes.push(0);
        assert!(matches!(
            Handshake::decode(&bytes),
            Err(DecodeError::TrailingBytes { extra: 1, .. })
        ));
    }
}
