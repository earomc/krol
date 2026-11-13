use thiserror::Error;

use crate::TagType;

/// Errors produced while reading or writing NBT.
#[derive(Debug, Error)]
pub enum Error {
    /// The underlying reader or writer failed (including unexpected end of input).
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// A tag type id outside `0..=11` was encountered.
    #[error("invalid NBT tag type {0}")]
    InvalidTagType(u8),
    /// The root tag of a file is not a compound.
    #[error("root tag must be a named compound tag")]
    RootNotCompound,
    /// An array or list declared a negative length.
    #[error("negative NBT length {0}")]
    NegativeLength(i32),
    /// A string is longer than 65535 bytes when encoded as modified UTF-8.
    #[error("encoded string too long: {0} bytes")]
    StringTooLong(usize),
    /// A string is not valid modified UTF-8.
    #[error(transparent)]
    InvalidString(#[from] krol_java::modified_utf8::DecodeError),
    /// Nesting exceeded [`MAX_DEPTH`](crate::MAX_DEPTH).
    #[error(
        "tried to read NBT tag with too high complexity, depth > {}",
        crate::MAX_DEPTH
    )]
    TooDeep,
    /// A list element does not match the list's element type.
    #[error("list element type {found:?} does not match list type {expected:?}")]
    HeterogeneousList {
        /// The list's element type.
        expected: TagType,
        /// The offending element's type.
        found: TagType,
    },
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
