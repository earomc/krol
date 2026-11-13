//! Compressed NBT containers.
//!
//! - **gzip**: `level.dat`, `playerdata/*.dat` and other standalone NBT files
//!   (vanilla: `CompressedStreamTools.readCompressed` / `writeCompressed`), and item NBT
//!   in protocol 5.
//! - **zlib**: chunk payloads inside Anvil region files.
//!
//! Compressed bytes need not match vanilla's exactly — only the decompressed NBT is
//! observable — but the default compression level (6) is used like Java's.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::{GzEncoder, ZlibEncoder};

use crate::{Compound, Result};

/// Reads a gzip-compressed named root compound.
///
/// # Errors
///
/// Any decompression, decoding or I/O error.
pub fn read_gzip(reader: impl Read) -> Result<(String, Compound)> {
    crate::read(&mut GzDecoder::new(reader))
}

/// Writes `root` as a gzip-compressed named root compound.
///
/// # Errors
///
/// Any encoding or I/O error.
pub fn write_gzip(writer: impl Write, name: &str, root: &Compound) -> Result<()> {
    let mut encoder = GzEncoder::new(writer, Compression::default());
    crate::write(&mut encoder, name, root)?;
    encoder.finish()?;
    Ok(())
}

/// Reads a zlib-compressed named root compound.
///
/// # Errors
///
/// Any decompression, decoding or I/O error.
pub fn read_zlib(reader: impl Read) -> Result<(String, Compound)> {
    crate::read(&mut ZlibDecoder::new(reader))
}

/// Writes `root` as a zlib-compressed named root compound.
///
/// # Errors
///
/// Any encoding or I/O error.
pub fn write_zlib(writer: impl Write, name: &str, root: &Compound) -> Result<()> {
    let mut encoder = ZlibEncoder::new(writer, Compression::default());
    crate::write(&mut encoder, name, root)?;
    encoder.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tag;

    fn sample() -> Compound {
        let mut c = Compound::new();
        c.insert("LevelName", Tag::String("world".into()));
        c.insert("RandomSeed", Tag::Long(-4_172_144_997_902_289_642));
        c
    }

    #[test]
    fn gzip_round_trip() {
        let mut buf = Vec::new();
        write_gzip(&mut buf, "", &sample()).unwrap();
        assert_eq!(&buf[..2], &[0x1f, 0x8b], "gzip magic");
        assert_eq!(
            read_gzip(buf.as_slice()).unwrap(),
            (String::new(), sample())
        );
    }

    #[test]
    fn zlib_round_trip() {
        let mut buf = Vec::new();
        write_zlib(&mut buf, "", &sample()).unwrap();
        assert_eq!(buf[0], 0x78, "zlib header");
        assert_eq!(
            read_zlib(buf.as_slice()).unwrap(),
            (String::new(), sample())
        );
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(read_gzip(&b"not gzip"[..]).is_err());
        assert!(read_zlib(&b"not zlib"[..]).is_err());
    }
}
