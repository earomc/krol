//! Packet compression (protocol 47 only; vanilla: `MessageCompressor` /
//! `MessageDeflater`).
//!
//! Once the server sends Set Compression with a threshold `t >= 0`, every frame body is
//! `VarInt data_length` followed by either the raw packet (`data_length == 0`, used for
//! packets shorter than `t`) or the zlib-compressed packet of `data_length` bytes.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

use crate::DecodeError;
use crate::buf::{PacketReader, PacketWriter};

/// Largest uncompressed packet vanilla accepts (2 MiB).
pub const MAX_UNCOMPRESSED: usize = 2_097_152;

/// Wraps an uncompressed packet body for a connection with compression `threshold`.
pub fn compress(packet: &[u8], threshold: usize) -> Vec<u8> {
    let mut w = PacketWriter::new();
    if packet.len() < threshold {
        w.varint(0).bytes(packet);
        return w.into_bytes();
    }
    w.varint(packet.len() as i32);
    let mut out = w.into_bytes();
    let mut encoder = ZlibEncoder::new(&mut out, Compression::default());
    // Writing into a Vec cannot fail.
    let _ = encoder.write_all(packet);
    let _ = encoder.finish();
    out
}

/// Unwraps a frame body from a connection with compression `threshold`.
///
/// # Errors
///
/// Vanilla's "Badly compressed packet" checks: a declared size below the threshold or
/// above [`MAX_UNCOMPRESSED`], or data that does not inflate to the declared size.
pub fn decompress(frame: &[u8], threshold: usize) -> Result<Vec<u8>, DecodeError> {
    let mut r = PacketReader::new(frame);
    let data_len = r.varint()?;
    if data_len == 0 {
        return Ok(r.rest().to_vec());
    }
    let len = usize::try_from(data_len).map_err(|_| bad_size(data_len, threshold))?;
    if len < threshold {
        return Err(DecodeError::InvalidValue(format!(
            "Badly compressed packet - size of {len} is below server threshold of {threshold}"
        )));
    }
    if len > MAX_UNCOMPRESSED {
        return Err(DecodeError::InvalidValue(format!(
            "Badly compressed packet - size of {len} is larger than protocol maximum of {MAX_UNCOMPRESSED}"
        )));
    }
    let mut out = Vec::with_capacity(len);
    ZlibDecoder::new(r.rest())
        .take(len as u64)
        .read_to_end(&mut out)
        .map_err(|e| DecodeError::InvalidValue(format!("bad zlib data: {e}")))?;
    if out.len() != len {
        return Err(DecodeError::InvalidValue(format!(
            "compressed packet inflated to {} bytes, expected {len}",
            out.len()
        )));
    }
    Ok(out)
}

fn bad_size(len: i32, threshold: usize) -> DecodeError {
    DecodeError::InvalidValue(format!(
        "Badly compressed packet - size of {len} is below server threshold of {threshold}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_packets_are_stored_raw() {
        let wrapped = compress(&[1, 2, 3], 256);
        assert_eq!(wrapped, [0, 1, 2, 3]);
        assert_eq!(decompress(&wrapped, 256).unwrap(), [1, 2, 3]);
    }

    #[test]
    fn large_packets_round_trip() {
        let packet: Vec<u8> = (0..1000u32).map(|i| (i % 7) as u8).collect();
        let wrapped = compress(&packet, 256);
        assert!(wrapped.len() < packet.len());
        assert_eq!(&wrapped[..2], &[0xe8, 0x07]); // VarInt 1000
        assert_eq!(decompress(&wrapped, 256).unwrap(), packet);
    }

    #[test]
    fn threshold_zero_compresses_everything() {
        let wrapped = compress(&[9], 0);
        assert_eq!(wrapped[0], 1);
        assert_eq!(decompress(&wrapped, 0).unwrap(), [9]);
    }

    #[test]
    fn rejects_bad_sizes() {
        let wrapped = compress(&[7; 300], 256);
        assert!(decompress(&wrapped, 512).is_err(), "below threshold");
        let mut w = PacketWriter::new();
        w.varint(3_000_000);
        assert!(decompress(w.as_bytes(), 256).is_err(), "above maximum");
        let mut lying = compress(&[7; 300], 256);
        lying[0] = 0xad; // claim 301 bytes (0xad 0x02)
        assert!(decompress(&lying, 256).is_err());
    }
}
