//! VarInt length-prefixed framing (vanilla: `MessageDeserializer2` / `MessagePrepender`).
//!
//! Each packet on the wire is `VarInt length` + `length` bytes. The length prefix is at
//! most 3 bytes (21 bits). Compression (protocol 47) and encryption are layered around
//! this by the connection.

use crate::buf::{PacketWriter, varint_len};
use crate::{DecodeError, EncodeError};

/// Largest frame body the 3-byte length prefix can describe.
pub const MAX_FRAME_LEN: usize = (1 << 21) - 1;

/// Incrementally splits a byte stream into frames.
///
/// ```
/// use krol_protocol::frame::FrameDecoder;
///
/// let mut dec = FrameDecoder::new();
/// dec.extend(&[0x02, 0xaa]);
/// assert_eq!(dec.next_frame().unwrap(), None); // incomplete
/// dec.extend(&[0xbb, 0x01]);
/// assert_eq!(dec.next_frame().unwrap(), Some(vec![0xaa, 0xbb]));
/// assert_eq!(dec.next_frame().unwrap(), None);
/// ```
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
    start: usize,
}

impl FrameDecoder {
    /// Creates an empty decoder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends received bytes.
    pub fn extend(&mut self, data: &[u8]) {
        if self.start > 0 && self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        }
        self.buf.extend_from_slice(data);
    }

    /// Bytes received but not yet returned as frames.
    pub fn buffered(&self) -> &[u8] {
        &self.buf[self.start..]
    }

    /// Returns the next complete, non-empty frame body, or `None` if more data is needed.
    ///
    /// Zero-length frames are skipped silently, as vanilla ignores them.
    ///
    /// # Errors
    ///
    /// [`DecodeError::FrameLengthTooWide`] if the length prefix exceeds 3 bytes.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, DecodeError> {
        loop {
            let data = &self.buf[self.start..];
            let mut len: usize = 0;
            let mut header = None;
            for i in 0..3 {
                let Some(&b) = data.get(i) else {
                    return Ok(None);
                };
                len |= usize::from(b & 0x7f) << (7 * i);
                if b & 0x80 == 0 {
                    header = Some(i + 1);
                    break;
                }
            }
            let Some(header) = header else {
                return Err(DecodeError::FrameLengthTooWide);
            };
            if data.len() < header + len {
                return Ok(None);
            }
            let body = data[header..header + len].to_vec();
            self.start += header + len;
            if self.start > 64 * 1024 && self.start * 2 > self.buf.len() {
                self.buf.drain(..self.start);
                self.start = 0;
            }
            if !body.is_empty() {
                return Ok(Some(body));
            }
        }
    }
}

/// Prefixes `body` with its VarInt length.
///
/// # Errors
///
/// [`EncodeError::FrameTooLarge`] if the length needs more than 3 bytes.
pub fn encode_frame(body: &[u8]) -> Result<Vec<u8>, EncodeError> {
    if body.len() > MAX_FRAME_LEN {
        return Err(EncodeError::FrameTooLarge(body.len()));
    }
    let mut w = PacketWriter::new();
    w.varint(body.len() as i32);
    let mut out = w.into_bytes();
    debug_assert!(varint_len(body.len() as i32) <= 3);
    out.extend_from_slice(body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_multiple_and_skips_empty() {
        let mut dec = FrameDecoder::new();
        dec.extend(&[0x01, 0x10, 0x00, 0x02, 0x20, 0x21]);
        assert_eq!(dec.next_frame().unwrap(), Some(vec![0x10]));
        assert_eq!(dec.next_frame().unwrap(), Some(vec![0x20, 0x21]));
        assert_eq!(dec.next_frame().unwrap(), None);
    }

    #[test]
    fn rejects_wide_length() {
        let mut dec = FrameDecoder::new();
        dec.extend(&[0x80, 0x80, 0x80, 0x01]);
        assert_eq!(dec.next_frame(), Err(DecodeError::FrameLengthTooWide));
    }

    #[test]
    fn byte_at_a_time() {
        let frame = encode_frame(&[7; 300]).unwrap();
        let mut dec = FrameDecoder::new();
        let mut out = None;
        for b in &frame {
            assert!(out.is_none());
            dec.extend(&[*b]);
            out = dec.next_frame().unwrap();
        }
        assert_eq!(out, Some(vec![7; 300]));
    }

    #[test]
    fn encode_limits() {
        assert!(encode_frame(&vec![0; MAX_FRAME_LEN]).is_ok());
        assert!(encode_frame(&vec![0; MAX_FRAME_LEN + 1]).is_err());
    }
}
