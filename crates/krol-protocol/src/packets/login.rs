//! Login state packets.
//!
//! Differences between the protocols: protocol 5 prefixes byte arrays with a `short`
//! length where 47 uses a VarInt, and protocol 5 has no Set Compression packet.

use super::{ConnectionState, finish};
use crate::buf::{PacketReader, PacketWriter};
use crate::{DecodeError, EncodeError, ProtocolVersion};

/// Serverbound login packets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerboundLogin {
    /// `0x00` Login Start with the player name (at most 16 characters).
    Start {
        /// Requested player name.
        name: String,
    },
    /// `0x01` Encryption Response: RSA-encrypted shared secret and verify token.
    EncryptionResponse {
        /// Encrypted shared secret.
        shared_secret: Vec<u8>,
        /// Encrypted verify token.
        verify_token: Vec<u8>,
    },
}

fn read_array<'a>(
    r: &mut PacketReader<'a>,
    version: ProtocolVersion,
) -> Result<&'a [u8], DecodeError> {
    let max = r.remaining();
    match version {
        ProtocolVersion::V47 => r.byte_array(max),
        ProtocolVersion::V5 => r.short_byte_array(max),
    }
}

fn write_array(w: &mut PacketWriter, version: ProtocolVersion, data: &[u8]) {
    match version {
        ProtocolVersion::V47 => w.byte_array(data),
        ProtocolVersion::V5 => w.short_byte_array(data),
    };
}

impl ServerboundLogin {
    /// Decodes a login-state frame body.
    ///
    /// # Errors
    ///
    /// [`DecodeError::BadPacketId`], field errors, or [`DecodeError::TrailingBytes`].
    pub fn decode(frame: &[u8], version: ProtocolVersion) -> Result<Self, DecodeError> {
        let mut r = PacketReader::new(frame);
        let id = r.varint()?;
        let packet = match id {
            0x00 => Self::Start {
                name: r.string(16)?,
            },
            0x01 => Self::EncryptionResponse {
                shared_secret: read_array(&mut r, version)?.to_vec(),
                verify_token: read_array(&mut r, version)?.to_vec(),
            },
            other => return Err(DecodeError::BadPacketId(other)),
        };
        finish(&r, ConnectionState::Login, id)?;
        Ok(packet)
    }

    /// Encodes the packet (used by test clients).
    ///
    /// # Errors
    ///
    /// [`EncodeError::StringTooBig`] for an oversized name.
    pub fn encode(&self, version: ProtocolVersion) -> Result<Vec<u8>, EncodeError> {
        let mut w = PacketWriter::new();
        match self {
            Self::Start { name } => {
                w.varint(0x00).string(name)?;
            }
            Self::EncryptionResponse {
                shared_secret,
                verify_token,
            } => {
                w.varint(0x01);
                write_array(&mut w, version, shared_secret);
                write_array(&mut w, version, verify_token);
            }
        }
        Ok(w.into_bytes())
    }
}

/// Clientbound login packets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientboundLogin {
    /// `0x00` Disconnect with a JSON chat component reason.
    Disconnect(String),
    /// `0x01` Encryption Request.
    EncryptionRequest {
        /// Server id string (empty in vanilla 1.8).
        server_id: String,
        /// DER-encoded RSA public key.
        public_key: Vec<u8>,
        /// Random verify token.
        verify_token: Vec<u8>,
    },
    /// `0x02` Login Success.
    Success {
        /// The player's UUID in hyphenated string form.
        uuid: String,
        /// The player's name.
        username: String,
    },
    /// `0x03` Set Compression (protocol 47 only).
    SetCompression(i32),
}

impl ClientboundLogin {
    /// Encodes the packet for `version`.
    ///
    /// # Errors
    ///
    /// [`EncodeError::StringTooBig`] for oversized strings.
    ///
    /// # Panics
    ///
    /// Panics if asked to encode [`SetCompression`](Self::SetCompression) for protocol 5,
    /// which has no such packet — a server logic error.
    pub fn encode(&self, version: ProtocolVersion) -> Result<Vec<u8>, EncodeError> {
        let mut w = PacketWriter::new();
        match self {
            Self::Disconnect(json) => {
                w.varint(0x00).string(json)?;
            }
            Self::EncryptionRequest {
                server_id,
                public_key,
                verify_token,
            } => {
                w.varint(0x01).string(server_id)?;
                write_array(&mut w, version, public_key);
                write_array(&mut w, version, verify_token);
            }
            Self::Success { uuid, username } => {
                w.varint(0x02).string(uuid)?.string(username)?;
            }
            Self::SetCompression(threshold) => {
                assert_eq!(
                    version,
                    ProtocolVersion::V47,
                    "protocol 5 has no Set Compression packet"
                );
                w.varint(0x03).varint(*threshold);
            }
        }
        Ok(w.into_bytes())
    }

    /// Decodes a clientbound login frame body (used by test clients).
    ///
    /// # Errors
    ///
    /// [`DecodeError::BadPacketId`], field errors, or [`DecodeError::TrailingBytes`].
    pub fn decode(frame: &[u8], version: ProtocolVersion) -> Result<Self, DecodeError> {
        let mut r = PacketReader::new(frame);
        let id = r.varint()?;
        let packet = match (id, version) {
            (0x00, _) => Self::Disconnect(r.string(32767)?),
            (0x01, _) => Self::EncryptionRequest {
                server_id: r.string(20)?,
                public_key: read_array(&mut r, version)?.to_vec(),
                verify_token: read_array(&mut r, version)?.to_vec(),
            },
            (0x02, _) => Self::Success {
                uuid: r.string(36)?,
                username: r.string(16)?,
            },
            (0x03, ProtocolVersion::V47) => Self::SetCompression(r.varint()?),
            (other, _) => return Err(DecodeError::BadPacketId(other)),
        };
        finish(&r, ConnectionState::Login, id)?;
        Ok(packet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_prefix_differs_by_version() {
        let p = ServerboundLogin::EncryptionResponse {
            shared_secret: vec![1, 2],
            verify_token: vec![3],
        };
        assert_eq!(
            p.encode(ProtocolVersion::V47).unwrap(),
            [0x01, 0x02, 1, 2, 0x01, 3]
        );
        assert_eq!(
            p.encode(ProtocolVersion::V5).unwrap(),
            [0x01, 0x00, 0x02, 1, 2, 0x00, 0x01, 3]
        );
        for v in [ProtocolVersion::V5, ProtocolVersion::V47] {
            assert_eq!(
                ServerboundLogin::decode(&p.encode(v).unwrap(), v).unwrap(),
                p
            );
        }
    }

    #[test]
    fn clientbound_round_trips() {
        let packets = [
            ClientboundLogin::Disconnect("\"bye\"".into()),
            ClientboundLogin::EncryptionRequest {
                server_id: String::new(),
                public_key: vec![9; 162],
                verify_token: vec![1, 2, 3, 4],
            },
            ClientboundLogin::Success {
                uuid: "069a79f4-44e9-4726-a5be-fca90e38aaf5".into(),
                username: "Notch".into(),
            },
        ];
        for v in [ProtocolVersion::V5, ProtocolVersion::V47] {
            for p in &packets {
                assert_eq!(
                    &ClientboundLogin::decode(&p.encode(v).unwrap(), v).unwrap(),
                    p
                );
            }
        }
        let sc = ClientboundLogin::SetCompression(256);
        assert_eq!(sc.encode(ProtocolVersion::V47).unwrap(), [0x03, 0x80, 0x02]);
        assert!(ClientboundLogin::decode(&[0x03, 0x00], ProtocolVersion::V5).is_err());
    }

    #[test]
    fn name_limit() {
        let long = ServerboundLogin::Start {
            name: "a".repeat(17),
        };
        let bytes = long.encode(ProtocolVersion::V47).unwrap();
        assert!(ServerboundLogin::decode(&bytes, ProtocolVersion::V47).is_err());
    }
}
