//! In-memory chunks and their Anvil NBT form (vanilla: `Chunk`, `ExtendedBlockStorage`,
//! `AnvilChunkLoader`).
//!
//! Blocks are stored as 1.8 block states: `id << 4 | metadata` (12-bit id). Raw values
//! are preserved even for unregistered ids, as vanilla saves its raw arrays unchanged.

use krol_nbt::{Compound, List, Tag, TagType};
use thiserror::Error;

/// Errors converting chunk NBT.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChunkError {
    /// The root has no `Level` compound.
    #[error("chunk has no Level tag")]
    MissingLevel,
    /// The stored position does not match the expected one.
    #[error("chunk file at {expected:?} is in the wrong location ({found:?})")]
    WrongLocation {
        /// Requested chunk coordinates.
        expected: (i32, i32),
        /// Coordinates stored in the file.
        found: (i32, i32),
    },
    /// An array has the wrong length (vanilla: "ChunkNibbleArrays should be 2048 bytes
    /// not: N").
    #[error("{name} should be {expected} bytes not: {found}")]
    BadArrayLength {
        /// Array name.
        name: &'static str,
        /// Required length.
        expected: usize,
        /// Actual length.
        found: usize,
    },
}

/// A 4-bit-per-entry array of 4096 values (vanilla: `NibbleArray`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NibbleArray(Box<[u8; 2048]>);

impl Default for NibbleArray {
    fn default() -> Self {
        Self(Box::new([0; 2048]))
    }
}

impl NibbleArray {
    /// Creates an array with every entry set to `value`.
    pub fn filled(value: u8) -> Self {
        let v = value & 15;
        Self(Box::new([v | (v << 4); 2048]))
    }

    fn from_bytes(name: &'static str, bytes: &[i8]) -> Result<Self, ChunkError> {
        if bytes.len() != 2048 {
            return Err(ChunkError::BadArrayLength {
                name,
                expected: 2048,
                found: bytes.len(),
            });
        }
        let mut out = Box::new([0u8; 2048]);
        for (o, b) in out.iter_mut().zip(bytes) {
            *o = *b as u8;
        }
        Ok(Self(out))
    }

    fn to_tag(&self) -> Tag {
        Tag::ByteArray(self.0.iter().map(|&b| b as i8).collect())
    }

    /// The value at section index `i` (`y << 8 | z << 4 | x`).
    pub fn get(&self, i: usize) -> u8 {
        let b = self.0[i >> 1];
        if i & 1 == 0 { b & 15 } else { b >> 4 }
    }

    /// Sets the value at section index `i`.
    pub fn set(&mut self, i: usize, value: u8) {
        let b = &mut self.0[i >> 1];
        if i & 1 == 0 {
            *b = (*b & 0xf0) | (value & 15);
        } else {
            *b = (*b & 0x0f) | ((value & 15) << 4);
        }
    }
}

/// A 16×16×16 section (vanilla: `ExtendedBlockStorage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Section index (0–15).
    pub y: i8,
    /// Block states, indexed `y << 8 | z << 4 | x`.
    pub blocks: Box<[u16; 4096]>,
    /// Block light.
    pub block_light: NibbleArray,
    /// Sky light (absent in dimensions without sky).
    pub sky_light: Option<NibbleArray>,
}

impl Section {
    /// An all-air section; sky light starts at 15 when present, like vanilla.
    pub fn empty(y: i8, has_sky: bool) -> Self {
        Self {
            y,
            blocks: Box::new([0; 4096]),
            block_light: NibbleArray::default(),
            sky_light: has_sky.then(|| NibbleArray::filled(15)),
        }
    }

    /// Whether the section has no non-air blocks (vanilla: `isEmpty`, `blockRefCount == 0`).
    ///
    /// Known deviation: vanilla counts unregistered block ids as air; krol counts any
    /// non-zero id as a block.
    pub fn is_empty(&self) -> bool {
        self.blocks.iter().all(|&s| s >> 4 == 0)
    }

    /// The section-local index of `(x, y, z)`.
    pub fn index(x: usize, y: usize, z: usize) -> usize {
        (y << 8) | (z << 4) | x
    }

    fn from_nbt(tag: &Compound, has_sky: bool) -> Result<Self, ChunkError> {
        let raw = tag.get_byte_array("Blocks");
        if raw.len() != 4096 {
            return Err(ChunkError::BadArrayLength {
                name: "Blocks",
                expected: 4096,
                found: raw.len(),
            });
        }
        let data = NibbleArray::from_bytes("Data", tag.get_byte_array("Data"))?;
        let add = match tag.get("Add") {
            Some(Tag::ByteArray(a)) => Some(NibbleArray::from_bytes("Add", a)?),
            _ => None,
        };
        let mut blocks = Box::new([0u16; 4096]);
        for (i, state) in blocks.iter_mut().enumerate() {
            let high = add.as_ref().map_or(0, |a| u16::from(a.get(i)));
            *state = (high << 12) | (u16::from(raw[i] as u8) << 4) | u16::from(data.get(i));
        }
        Ok(Self {
            y: tag.get_i8("Y"),
            blocks,
            block_light: NibbleArray::from_bytes("BlockLight", tag.get_byte_array("BlockLight"))?,
            sky_light: if has_sky {
                Some(NibbleArray::from_bytes(
                    "SkyLight",
                    tag.get_byte_array("SkyLight"),
                )?)
            } else {
                None
            },
        })
    }

    fn to_nbt(&self) -> Compound {
        let mut raw = Vec::with_capacity(4096);
        let mut data = NibbleArray::default();
        let mut add: Option<NibbleArray> = None;
        for (i, &state) in self.blocks.iter().enumerate() {
            raw.push((state >> 4) as u8 as i8);
            data.set(i, (state & 15) as u8);
            if state >> 12 != 0 {
                add.get_or_insert_with(NibbleArray::default)
                    .set(i, (state >> 12) as u8);
            }
        }
        let mut c = Compound::new();
        c.insert("Y", Tag::Byte(self.y));
        c.insert("Blocks", Tag::ByteArray(raw));
        if let Some(add) = add {
            c.insert("Add", add.to_tag());
        }
        c.insert("Data", data.to_tag());
        c.insert("BlockLight", self.block_light.to_tag());
        if let Some(sky) = &self.sky_light {
            c.insert("SkyLight", sky.to_tag());
        }
        c
    }
}

/// A 16×256×16 chunk column.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// Chunk x.
    pub x: i32,
    /// Chunk z.
    pub z: i32,
    /// Sections by index; `None` for sections vanilla does not store.
    pub sections: [Option<Section>; 16],
    /// Biome id per column (`z << 4 | x`); 255 = not yet generated.
    pub biomes: [u8; 256],
    /// Height map per column.
    pub height_map: [i32; 256],
    /// `LastUpdate` (world time of the last save).
    pub last_update: i64,
    /// `InhabitedTime` (ticks players spent in the chunk).
    pub inhabited_time: i64,
    /// `TerrainPopulated`.
    pub terrain_populated: bool,
    /// `LightPopulated`.
    pub light_populated: bool,
    /// Entity NBT (kept raw until entities are modeled).
    pub entities: List,
    /// Tile entity NBT (kept raw until tile entities are modeled).
    pub tile_entities: List,
    /// Scheduled block ticks, if stored.
    pub tile_ticks: Option<List>,
    /// Any other `Level` keys, preserved for lossless round-trips.
    pub extra: Compound,
}

const KNOWN_KEYS: [&str; 13] = [
    "xPos",
    "zPos",
    "LastUpdate",
    "LightPopulated",
    "TerrainPopulated",
    "V",
    "InhabitedTime",
    "Biomes",
    "HeightMap",
    "Sections",
    "Entities",
    "TileEntities",
    "TileTicks",
];

impl Chunk {
    /// Serializes sections for a protocol-47 Chunk Data / Map Chunk Bulk packet (vanilla:
    /// `S21PacketChunkData.func_179756_a`). Returns the included section mask and data:
    /// all block states (little-endian `u16`), then block light, then sky light (if
    /// `sky`), then biomes (if `full`). In a full chunk, empty sections are omitted.
    pub fn packet_data(&self, full: bool, mask: u16, sky: bool) -> (u16, Vec<u8>) {
        let included: Vec<(usize, &Section)> = self
            .sections
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|s| (i, s)))
            .filter(|(i, s)| (!full || !s.is_empty()) && mask & (1 << i) != 0)
            .collect();
        let out_mask = included.iter().fold(0u16, |m, (i, _)| m | (1 << i));
        let n = included.len();
        let mut data =
            Vec::with_capacity(n * 8192 + n * 2048 + if sky { n * 2048 } else { 0 } + 256);
        for (_, s) in &included {
            for &state in s.blocks.iter() {
                data.extend_from_slice(&state.to_le_bytes());
            }
        }
        for (_, s) in &included {
            data.extend_from_slice(&s.block_light.0[..]);
        }
        if sky {
            for (_, s) in &included {
                // A section without stored sky light (loaded in a sky-less dimension) is
                // sent dark.
                match &s.sky_light {
                    Some(l) => data.extend_from_slice(&l.0[..]),
                    None => data.extend_from_slice(&[0u8; 2048]),
                }
            }
        }
        if full {
            data.extend_from_slice(&self.biomes);
        }
        (out_mask, data)
    }

    /// An empty, ungenerated chunk.
    pub fn empty(x: i32, z: i32) -> Self {
        Self {
            x,
            z,
            sections: Default::default(),
            biomes: [255; 256],
            height_map: [0; 256],
            last_update: 0,
            inhabited_time: 0,
            terrain_populated: false,
            light_populated: false,
            entities: List::new(),
            tile_entities: List::new(),
            tile_ticks: None,
            extra: Compound::new(),
        }
    }

    /// The block state at chunk-local `(x, y, z)`; 0 (air) outside stored sections.
    pub fn block(&self, x: usize, y: usize, z: usize) -> u16 {
        self.sections
            .get(y >> 4)
            .and_then(Option::as_ref)
            .map_or(0, |s| s.blocks[Section::index(x, y & 15, z)])
    }

    /// Sets the block state at chunk-local `(x, y, z)`, creating the section if needed.
    pub fn set_block(&mut self, x: usize, y: usize, z: usize, state: u16, has_sky: bool) {
        let index = y >> 4;
        let section =
            self.sections[index].get_or_insert_with(|| Section::empty(index as i8, has_sky));
        section.blocks[Section::index(x, y & 15, z)] = state;
    }

    /// Reads a chunk from its region-file root compound, checking the position like
    /// vanilla's `checkedReadChunkFromNBT`.
    ///
    /// # Errors
    ///
    /// See [`ChunkError`].
    pub fn from_nbt(root: &Compound, x: i32, z: i32, has_sky: bool) -> Result<Self, ChunkError> {
        let level = root.get_compound("Level").ok_or(ChunkError::MissingLevel)?;
        let found = (level.get_i32("xPos"), level.get_i32("zPos"));
        if found != (x, z) {
            return Err(ChunkError::WrongLocation {
                expected: (x, z),
                found,
            });
        }
        let mut chunk = Self::empty(x, z);
        chunk.last_update = level.get_i64("LastUpdate");
        chunk.inhabited_time = level.get_i64("InhabitedTime");
        chunk.terrain_populated = level.get_bool("TerrainPopulated");
        chunk.light_populated = level.get_bool("LightPopulated");

        let heights = level.get_int_array("HeightMap");
        if heights.len() == 256 {
            chunk.height_map.copy_from_slice(heights);
        }
        let biomes = level.get_byte_array("Biomes");
        if biomes.len() == 256 {
            for (b, &v) in chunk.biomes.iter_mut().zip(biomes) {
                *b = v as u8;
            }
        }
        if let Some(sections) = level.get_list("Sections", TagType::Compound) {
            for tag in sections {
                if let Tag::Compound(c) = tag {
                    let section = Section::from_nbt(c, has_sky)?;
                    let y = section.y;
                    if (0..16).contains(&y) {
                        chunk.sections[y as usize] = Some(section);
                    }
                }
            }
        }
        if let Some(l) = level.get_list("Entities", TagType::Compound) {
            chunk.entities = l.clone();
        }
        if let Some(l) = level.get_list("TileEntities", TagType::Compound) {
            chunk.tile_entities = l.clone();
        }
        chunk.tile_ticks = level.get_list("TileTicks", TagType::Compound).cloned();
        for (k, v) in level {
            if !KNOWN_KEYS.contains(&k.as_str()) {
                chunk.extra.insert(k.clone(), v.clone());
            }
        }
        Ok(chunk)
    }

    /// Writes the chunk as a region-file root compound (`{Level: {...}}`), like vanilla's
    /// `writeChunkToNBT`; `world_time` becomes `LastUpdate`.
    pub fn to_nbt(&self, world_time: i64) -> Compound {
        let mut level = Compound::new();
        level.insert("V", Tag::Byte(1));
        level.insert("xPos", Tag::Int(self.x));
        level.insert("zPos", Tag::Int(self.z));
        level.insert("LastUpdate", Tag::Long(world_time));
        level.insert("HeightMap", Tag::IntArray(self.height_map.to_vec()));
        level.insert(
            "TerrainPopulated",
            Tag::Byte(i8::from(self.terrain_populated)),
        );
        level.insert("LightPopulated", Tag::Byte(i8::from(self.light_populated)));
        level.insert("InhabitedTime", Tag::Long(self.inhabited_time));
        let mut sections = List::new();
        for section in self.sections.iter().flatten() {
            sections
                .push(Tag::Compound(section.to_nbt()))
                .expect("homogeneous");
        }
        level.insert("Sections", Tag::List(sections));
        level.insert(
            "Biomes",
            Tag::ByteArray(self.biomes.iter().map(|&b| b as i8).collect()),
        );
        level.insert("Entities", Tag::List(self.entities.clone()));
        level.insert("TileEntities", Tag::List(self.tile_entities.clone()));
        if let Some(ticks) = &self.tile_ticks {
            level.insert("TileTicks", Tag::List(ticks.clone()));
        }
        for (k, v) in &self.extra {
            level.insert(k.clone(), v.clone());
        }
        let mut root = Compound::new();
        root.insert("Level", Tag::Compound(level));
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nibbles() {
        let mut n = NibbleArray::default();
        n.set(0, 0xa);
        n.set(1, 0x5);
        n.set(4095, 0xf);
        assert_eq!((n.get(0), n.get(1), n.get(4095)), (0xa, 0x5, 0xf));
        assert_eq!(n.0[0], 0x5a);
    }

    #[test]
    fn round_trip_preserves_states_and_extra_keys() {
        let mut chunk = Chunk::empty(3, -7);
        chunk.set_block(1, 2, 3, (1 << 4) | 2, true); // granite? (stone:2)
        chunk.set_block(15, 255, 15, (0xfff << 4) | 0xf, true); // unregistered high id
        chunk.biomes[5] = 4;
        chunk.extra.insert("Custom", Tag::Int(42));
        let root = chunk.to_nbt(1000);
        let level = root.get_compound("Level").unwrap();
        let sections = level.get_list("Sections", TagType::Compound).unwrap();
        assert_eq!(sections.len(), 2);
        let Tag::Compound(top) = sections.get(1).unwrap() else {
            panic!()
        };
        assert!(top.contains_key("Add"), "high ids need the Add array");
        let Tag::Compound(bottom) = sections.get(0).unwrap() else {
            panic!()
        };
        assert!(!bottom.contains_key("Add"));

        let back = Chunk::from_nbt(&root, 3, -7, true).unwrap();
        assert_eq!(back.block(1, 2, 3), (1 << 4) | 2);
        assert_eq!(back.block(15, 255, 15), 0xffff);
        assert_eq!(back.block(0, 100, 0), 0);
        assert_eq!(back.extra.get_i32("Custom"), 42);
        assert_eq!(back.last_update, 1000);
        assert_eq!(back.biomes[5], 4);
    }

    #[test]
    fn packet_layout() {
        let mut chunk = Chunk::empty(0, 0);
        chunk.set_block(0, 0, 0, 0x0123, true);
        chunk.sections[5] = Some(Section::empty(5, true)); // empty: omitted when full
        let (mask, data) = chunk.packet_data(true, 0xffff, true);
        assert_eq!(mask, 1);
        assert_eq!(data.len(), 8192 + 2048 + 2048 + 256);
        assert_eq!(&data[..2], &[0x23, 0x01]);
        assert_eq!(data[8192 + 2048], 0xff, "sky light starts at 15");
        let (mask, data) = chunk.packet_data(false, 0xffff, false);
        assert_eq!(mask, 0b10_0001);
        assert_eq!(data.len(), 2 * (8192 + 2048));
        let (mask, data) = chunk.packet_data(true, 0, true);
        assert_eq!(
            (mask, data.len()),
            (0, 256),
            "unload packet carries biomes only"
        );
    }

    #[test]
    fn rejects_wrong_location_and_bad_arrays() {
        let root = Chunk::empty(0, 0).to_nbt(0);
        assert_eq!(
            Chunk::from_nbt(&root, 1, 0, true).unwrap_err(),
            ChunkError::WrongLocation {
                expected: (1, 0),
                found: (0, 0)
            }
        );
        let mut bad = Chunk::empty(0, 0);
        bad.set_block(0, 0, 0, 16, true);
        let mut root = bad.to_nbt(0);
        if let Some(Tag::Compound(level)) = root.get_mut("Level")
            && let Some(Tag::List(sections)) = level.get_mut("Sections")
            && let Some(Tag::Compound(s)) = sections.get_mut(0)
        {
            s.insert("Data", Tag::ByteArray(vec![0; 10]));
        }
        assert!(matches!(
            Chunk::from_nbt(&root, 0, 0, true),
            Err(ChunkError::BadArrayLength {
                name: "Data",
                found: 10,
                ..
            })
        ));
    }
}
