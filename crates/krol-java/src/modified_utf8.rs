//! Java's "modified UTF-8", as used by `DataOutput.writeUTF` and therefore by NBT strings.
//!
//! It differs from standard UTF-8 in two ways: U+0000 is encoded as the two bytes
//! `C0 80`, and characters outside the Basic Multilingual Plane are encoded as a UTF-16
//! surrogate pair with each surrogate written as a separate 3-byte sequence.
//!
//! The functions here handle the string *body* only; length prefixes are the caller's
//! concern.

use thiserror::Error;

/// Error returned by [`decode`] for input Java's `readUTF` would reject, or which does not
/// form a valid Unicode string.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecodeError {
    /// A byte sequence that is not valid modified UTF-8 (Java: `UTFDataFormatException`).
    #[error("malformed modified UTF-8 at byte {0}")]
    Malformed(usize),
    /// The input ended in the middle of a multi-byte sequence.
    #[error("truncated modified UTF-8 sequence")]
    Truncated,
    /// The decoded UTF-16 contains an unpaired surrogate, which Rust strings cannot hold.
    #[error("unpaired surrogate in modified UTF-8 string")]
    UnpairedSurrogate,
}

/// Returns the number of bytes [`encode`] would produce for `s`.
pub fn encoded_len(s: &str) -> usize {
    s.encode_utf16().map(unit_len).sum()
}

fn unit_len(unit: u16) -> usize {
    match unit {
        0x0001..=0x007f => 1,
        0x0000 | 0x0080..=0x07ff => 2,
        _ => 3,
    }
}

/// Encodes `s` as modified UTF-8.
///
/// ```
/// use krol_java::modified_utf8;
/// assert_eq!(modified_utf8::encode("a\0"), vec![0x61, 0xc0, 0x80]);
/// ```
pub fn encode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(encoded_len(s));
    for unit in s.encode_utf16() {
        match unit_len(unit) {
            1 => out.push(unit as u8),
            2 => {
                out.push(0xc0 | (unit >> 6) as u8);
                out.push(0x80 | (unit & 0x3f) as u8);
            }
            _ => {
                out.push(0xe0 | (unit >> 12) as u8);
                out.push(0x80 | ((unit >> 6) & 0x3f) as u8);
                out.push(0x80 | (unit & 0x3f) as u8);
            }
        }
    }
    out
}

/// Decodes a modified UTF-8 byte string.
///
/// Like Java's `readUTF`, this accepts any well-formed 1-, 2- or 3-byte sequence
/// (including non-shortest forms) and rejects 4-byte lead bytes and bad continuation
/// bytes.
///
/// # Errors
///
/// See [`DecodeError`].
///
/// ```
/// use krol_java::modified_utf8;
/// assert_eq!(modified_utf8::decode(&[0x61, 0xc0, 0x80]).unwrap(), "a\0");
/// ```
pub fn decode(bytes: &[u8]) -> Result<String, DecodeError> {
    let cont = |idx: usize| -> Result<u16, DecodeError> {
        let b = *bytes.get(idx).ok_or(DecodeError::Truncated)?;
        if b & 0xc0 != 0x80 {
            return Err(DecodeError::Malformed(idx));
        }
        Ok(u16::from(b & 0x3f))
    };
    let mut units = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b >> 4 {
            0..=7 => {
                units.push(u16::from(b));
                i += 1;
            }
            12 | 13 => {
                units.push((u16::from(b & 0x1f) << 6) | cont(i + 1)?);
                i += 2;
            }
            14 => {
                units.push((u16::from(b & 0x0f) << 12) | (cont(i + 1)? << 6) | cont(i + 2)?);
                i += 3;
            }
            _ => return Err(DecodeError::Malformed(i)),
        }
    }
    String::from_utf16(&units).map_err(|_| DecodeError::UnpairedSurrogate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_input() {
        assert_eq!(
            decode(&[0xf0, 0x9f, 0x98, 0x80]),
            Err(DecodeError::Malformed(0))
        );
        assert_eq!(decode(&[0xc0]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0xc0, 0x41]), Err(DecodeError::Malformed(1)));
        assert_eq!(
            decode(&[0xed, 0xa0, 0x80]),
            Err(DecodeError::UnpairedSurrogate)
        );
    }

    #[test]
    fn encoded_len_matches() {
        for s in ["", "abc", "\0", "äöü", "日本", "\u{1F600}"] {
            assert_eq!(encoded_len(s), encode(s).len());
        }
    }
}
