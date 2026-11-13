/// A protocol version krol speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolVersion {
    /// Protocol 5: Minecraft 1.7.6 – 1.7.10. Translated to/from protocol 47.
    V5,
    /// Protocol 47: Minecraft 1.8 – 1.8.9. The canonical protocol.
    V47,
}

impl ProtocolVersion {
    /// Maps a handshake protocol number to a supported version.
    pub fn from_number(number: i32) -> Option<Self> {
        match number {
            5 => Some(Self::V5),
            47 => Some(Self::V47),
            _ => None,
        }
    }

    /// The protocol number sent in the handshake.
    pub fn number(self) -> i32 {
        match self {
            Self::V5 => 5,
            Self::V47 => 47,
        }
    }

    /// The version name shown to clients of this protocol in the server list.
    pub fn version_name(self) -> &'static str {
        match self {
            Self::V5 => "1.7.10",
            Self::V47 => "1.8.8",
        }
    }
}
