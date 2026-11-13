//! Anvil region files (`r.<x>.<z>.mca`; vanilla: `RegionFile`).
//!
//! A region stores 32×32 chunks. The file starts with two 4 KiB tables: chunk locations
//! (3-byte sector offset + 1-byte sector count) and last-write timestamps (seconds).
//! Each chunk is `u32 length`, `u8 compression` (1 = gzip, 2 = zlib) and `length - 1`
//! bytes of compressed NBT, padded to whole 4 KiB sectors.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use krol_nbt::Compound;
use thiserror::Error;

/// Bytes per sector.
pub const SECTOR_BYTES: usize = 4096;
/// Chunks needing this many sectors (1 MiB) or more are not saved, as in vanilla.
pub const MAX_SECTORS_PER_CHUNK: usize = 256;

/// Region file errors.
#[derive(Debug, Error)]
pub enum RegionError {
    /// File I/O failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The chunk payload could not be decoded.
    #[error(transparent)]
    Nbt(#[from] krol_nbt::Error),
    /// Coordinates outside `0..32`.
    #[error("chunk coordinates ({0}, {1}) outside region")]
    OutOfBounds(i32, i32),
}

/// An open region file.
#[derive(Debug)]
pub struct RegionFile {
    file: File,
    offsets: [u32; 1024],
    timestamps: [u32; 1024],
    /// `true` for sectors in use (the two header sectors are always used).
    used: Vec<bool>,
}

fn index(x: i32, z: i32) -> Result<usize, RegionError> {
    if !(0..32).contains(&x) || !(0..32).contains(&z) {
        return Err(RegionError::OutOfBounds(x, z));
    }
    Ok((x + z * 32) as usize)
}

impl RegionFile {
    /// Opens (creating if needed) the region file at `path`, like vanilla: a short file is
    /// padded to the header size and to a whole number of sectors.
    ///
    /// # Errors
    ///
    /// I/O errors.
    pub fn open(path: &Path) -> Result<Self, RegionError> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let mut len = file.metadata()?.len() as usize;
        if len < 2 * SECTOR_BYTES {
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&vec![0u8; 2 * SECTOR_BYTES])?;
            len = 2 * SECTOR_BYTES;
        }
        if len % SECTOR_BYTES != 0 {
            file.seek(SeekFrom::End(0))?;
            file.write_all(&vec![0u8; SECTOR_BYTES - len % SECTOR_BYTES])?;
            len += SECTOR_BYTES - len % SECTOR_BYTES;
        }
        let sectors = len / SECTOR_BYTES;
        let mut used = vec![false; sectors];
        used[0] = true;
        used[1] = true;

        let mut header = vec![0u8; 2 * SECTOR_BYTES];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut header)?;
        let mut offsets = [0u32; 1024];
        let mut timestamps = [0u32; 1024];
        for i in 0..1024 {
            let o = u32::from_be_bytes(header[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
            offsets[i] = o;
            let start = (o >> 8) as usize;
            let count = (o & 0xff) as usize;
            if o != 0 && start + count <= sectors {
                for s in &mut used[start..start + count] {
                    *s = true;
                }
            }
            let t = SECTOR_BYTES + i * 4;
            timestamps[i] = u32::from_be_bytes(header[t..t + 4].try_into().expect("4 bytes"));
        }
        Ok(Self {
            file,
            offsets,
            timestamps,
            used,
        })
    }

    /// Whether the chunk at region-local `(x, z)` is stored.
    pub fn has_chunk(&self, x: i32, z: i32) -> bool {
        index(x, z).is_ok_and(|i| self.offsets[i] != 0)
    }

    /// The last-write timestamp (Unix seconds) of a chunk.
    pub fn timestamp(&self, x: i32, z: i32) -> Option<u32> {
        index(x, z).ok().map(|i| self.timestamps[i])
    }

    /// Reads the chunk at region-local `(x, z)`.
    ///
    /// Like vanilla, missing chunks and corrupt locations (pointing past the end, bad
    /// lengths or unknown compression) yield `Ok(None)`.
    ///
    /// # Errors
    ///
    /// I/O errors and undecodable NBT.
    pub fn read_chunk(&mut self, x: i32, z: i32) -> Result<Option<Compound>, RegionError> {
        let o = self.offsets[index(x, z)?];
        if o == 0 {
            return Ok(None);
        }
        let start = (o >> 8) as usize;
        let count = (o & 0xff) as usize;
        if start + count > self.used.len() {
            return Ok(None);
        }
        self.file
            .seek(SeekFrom::Start((start * SECTOR_BYTES) as u64))?;
        let mut head = [0u8; 5];
        self.file.read_exact(&mut head)?;
        let length = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        if length == 0 || length > count * SECTOR_BYTES {
            return Ok(None);
        }
        let mut data = vec![0u8; length - 1];
        self.file.read_exact(&mut data)?;
        let root = match head[4] {
            1 => krol_nbt::read_gzip(data.as_slice())?.1,
            2 => krol_nbt::read_zlib(data.as_slice())?.1,
            _ => return Ok(None),
        };
        Ok(Some(root))
    }

    /// Writes the chunk at region-local `(x, z)` (zlib, like vanilla), stamping it with
    /// `timestamp` (Unix seconds).
    ///
    /// Returns `Ok(false)` without writing if the chunk needs 256 sectors (1 MiB) or more
    /// — vanilla quirk: such chunks are silently not saved.
    ///
    /// # Errors
    ///
    /// I/O and encoding errors.
    pub fn write_chunk(
        &mut self,
        x: i32,
        z: i32,
        chunk: &Compound,
        timestamp: u32,
    ) -> Result<bool, RegionError> {
        let i = index(x, z)?;
        let mut data = Vec::new();
        krol_nbt::write_zlib(&mut data, "", chunk)?;
        let length = data.len() + 1;
        let needed = (length + 4).div_ceil(SECTOR_BYTES);
        if needed >= MAX_SECTORS_PER_CHUNK {
            return Ok(false);
        }

        let old = self.offsets[i];
        let old_start = (old >> 8) as usize;
        let old_count = (old & 0xff) as usize;
        let start = if old != 0 && old_count == needed {
            old_start
        } else {
            // Free the old sectors, then take the first run that fits or append.
            if old != 0 {
                for s in self.used.iter_mut().skip(old_start).take(old_count) {
                    *s = false;
                }
            }
            match self.find_free_run(needed) {
                Some(s) => s,
                None => {
                    let s = self.used.len();
                    self.file.seek(SeekFrom::End(0))?;
                    self.file.write_all(&vec![0u8; needed * SECTOR_BYTES])?;
                    self.used.resize(s + needed, false);
                    s
                }
            }
        };
        for s in &mut self.used[start..start + needed] {
            *s = true;
        }

        self.file
            .seek(SeekFrom::Start((start * SECTOR_BYTES) as u64))?;
        self.file.write_all(&(length as u32).to_be_bytes())?;
        self.file.write_all(&[2])?;
        self.file.write_all(&data)?;

        self.offsets[i] = ((start as u32) << 8) | needed as u32;
        self.file.seek(SeekFrom::Start((i * 4) as u64))?;
        self.file.write_all(&self.offsets[i].to_be_bytes())?;
        self.timestamps[i] = timestamp;
        self.file
            .seek(SeekFrom::Start((SECTOR_BYTES + i * 4) as u64))?;
        self.file.write_all(&timestamp.to_be_bytes())?;
        Ok(true)
    }

    fn find_free_run(&self, needed: usize) -> Option<usize> {
        let mut run_start = 0;
        let mut run_len = 0;
        for (s, &used) in self.used.iter().enumerate() {
            if used {
                run_len = 0;
            } else {
                if run_len == 0 {
                    run_start = s;
                }
                run_len += 1;
                if run_len == needed {
                    return Some(run_start);
                }
            }
        }
        None
    }

    /// Flushes pending writes.
    ///
    /// # Errors
    ///
    /// I/O errors.
    pub fn flush(&mut self) -> Result<(), RegionError> {
        self.file.flush()?;
        Ok(())
    }
}

/// The region file name for chunk `(cx, cz)`: `r.<cx >> 5>.<cz >> 5>.mca`.
pub fn region_file_name(cx: i32, cz: i32) -> String {
    format!("r.{}.{}.mca", cx >> 5, cz >> 5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use krol_nbt::Tag;

    fn chunk(marker: i32, filler: usize) -> Compound {
        let mut c = Compound::new();
        c.insert("marker", Tag::Int(marker));
        // Incompressible filler (xorshift) to control the sector count.
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let bytes: Vec<i8> = (0..filler)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as i8
            })
            .collect();
        c.insert("filler", Tag::ByteArray(bytes));
        c
    }

    #[test]
    fn write_read_and_reuse_sectors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.0.0.mca");
        let mut region = RegionFile::open(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 8192);
        assert!(!region.has_chunk(3, 4));
        assert!(region.write_chunk(3, 4, &chunk(1, 100), 1234).unwrap());
        assert!(region.write_chunk(31, 31, &chunk(2, 10_000), 99).unwrap());
        assert_eq!(
            region.read_chunk(3, 4).unwrap().unwrap().get_i32("marker"),
            1
        );
        assert_eq!(region.timestamp(3, 4), Some(1234));
        drop(region);

        let mut region = RegionFile::open(&path).unwrap();
        assert_eq!(
            region
                .read_chunk(31, 31)
                .unwrap()
                .unwrap()
                .get_i32("marker"),
            2
        );
        let size_before = std::fs::metadata(&path).unwrap().len();
        // Same size: rewritten in place.
        assert!(region.write_chunk(3, 4, &chunk(3, 100), 5).unwrap());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), size_before);
        assert_eq!(
            region.read_chunk(3, 4).unwrap().unwrap().get_i32("marker"),
            3
        );
        assert_eq!(region.read_chunk(0, 0).unwrap(), None);
        assert!(region.read_chunk(32, 0).is_err());
    }

    #[test]
    fn oversized_chunks_are_not_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mut region = RegionFile::open(&dir.path().join("r.0.0.mca")).unwrap();
        assert!(!region.write_chunk(0, 0, &chunk(1, 1_100_000), 0).unwrap());
        assert!(!region.has_chunk(0, 0));
    }

    #[test]
    fn region_names() {
        assert_eq!(region_file_name(0, 0), "r.0.0.mca");
        assert_eq!(region_file_name(-1, 32), "r.-1.1.mca");
        assert_eq!(region_file_name(-33, -32), "r.-2.-1.mca");
    }
}
