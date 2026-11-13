use thiserror::Error;

/// Errors while decoding data received from a client.
///
/// Vanilla reacts to any of these by closing the connection; messages follow vanilla's
/// where vanilla has one.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecodeError {
    /// The input ended before the field was complete.
    #[error("unexpected end of packet")]
    UnexpectedEnd,
    /// A VarInt longer than 5 bytes (or VarLong longer than 10).
    #[error("VarInt too big")]
    VarIntTooBig,
    /// A frame length prefix longer than 3 bytes.
    #[error("length wider than 21-bit")]
    FrameLengthTooWide,
    /// A negative string length.
    #[error("The received encoded string buffer length is less than zero! Weird string!")]
    NegativeStringLength,
    /// A string whose encoded length exceeds 4 × the allowed character count.
    #[error(
        "The received encoded string buffer length is longer than maximum allowed ({len} > {max_bytes})"
    )]
    StringBytesTooLong {
        /// Declared byte length.
        len: i32,
        /// Allowed byte length (4 × max characters).
        max_bytes: i32,
    },
    /// A string with more UTF-16 units than allowed.
    ///
    /// Vanilla quirk: the message reports the *byte* length, not the character count.
    #[error("The received string length is longer than maximum allowed ({len} > {max})")]
    StringTooLong {
        /// Declared byte length (what vanilla prints).
        len: i32,
        /// Allowed length.
        max: usize,
    },
    /// A negative or oversized array length.
    #[error("invalid array length {0}")]
    InvalidLength(i32),
    /// An unknown packet id for the current state.
    #[error("Bad packet id {0}")]
    BadPacketId(i32),
    /// A packet whose body was not fully consumed.
    #[error("Packet {state}/{id} was larger than I expected, found {extra} bytes extra")]
    TrailingBytes {
        /// Connection state name.
        state: &'static str,
        /// Packet id.
        id: i32,
        /// Number of unread bytes.
        extra: usize,
    },
    /// A field value outside its valid range (e.g. an unknown handshake next state).
    #[error("invalid value: {0}")]
    InvalidValue(String),
    /// Embedded NBT could not be decoded.
    #[error("invalid NBT: {0}")]
    Nbt(String),
}

/// Errors while encoding data to send.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EncodeError {
    /// A string over 32767 bytes of UTF-8.
    #[error("String too big (was {0} bytes encoded, max 32767)")]
    StringTooBig(usize),
    /// A frame larger than the 21-bit length prefix allows.
    #[error("frame of {0} bytes does not fit a 21-bit length")]
    FrameTooLarge(usize),
}
