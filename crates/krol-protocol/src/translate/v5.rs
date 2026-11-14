//! The protocol 5 translator.
//!
//! Notable differences from protocol 47 handled here:
//! - entity ids are `int` instead of VarInt in most entity packets; movement packets
//!   have no on-ground flag,
//! - block positions are three separate coordinates instead of a packed long,
//! - item stacks carry gzipped NBT with a length prefix,
//! - chunks store block ids and metadata in separate arrays and are zlib-compressed,
//! - the player list is keyed by display name (one packet per entry) instead of UUID,
//!   and Spawn Player carries the name and skin properties itself,
//! - Player Position And Look has no relative flags and sends the eye height
//!   (feet + 1.62),
//! - 1.8-only packets (difficulty, world border, titles, ...) are not sent,
//! - 1.8-only blocks and items are replaced (see [`super::v5_ids`]).

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::{GzEncoder, ZlibEncoder};

use super::v5_ids;
use crate::buf::{PacketReader, PacketWriter};
use crate::packets::play::{
    ChunkColumn, ClientboundPlay, ItemStack, Metadata, MetadataValue, PlayerListAction,
    ServerboundPlay,
};
use crate::packets::{ConnectionState, finish};
use crate::{DecodeError, EncodeError};

/// The eye height 1.7 clients expect in Player Position And Look (`1.62f` as double).
const EYE_HEIGHT: f64 = 1.620_000_004_768_371_6;

/// Skin properties of a profile: (name, value, signature).
type Properties = Vec<(String, String, Option<String>)>;

/// Per-connection state for translating a 1.7 client's play session.
#[derive(Debug, Default)]
pub struct V5Translator {
    /// The client's own entity id (from Join Game).
    own_entity: Option<i32>,
    /// The client's position (feet) and rotation as last known, for relative teleports.
    pos: (f64, f64, f64),
    rot: (f32, f32),
    /// Player list entries by UUID: name and skin properties.
    profiles: HashMap<u128, (String, Properties)>,
    /// Entity ids of spawned players.
    player_entities: HashSet<i32>,
}

/// A 1.7 packet under construction.
fn packet(id: i32) -> PacketWriter {
    let mut w = PacketWriter::new();
    w.varint(id);
    w
}

/// Java's `UUID.toString`.
fn uuid_string(uuid: u128) -> String {
    let h = format!("{uuid:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

fn nbt_error(e: krol_nbt::Error) -> EncodeError {
    match e {
        krol_nbt::Error::StringTooLong(n) => EncodeError::StringTooBig(n),
        _ => EncodeError::StringTooBig(0),
    }
}

/// Writes a 1.7 item stack: id, count, damage, then the NBT gzipped with a short
/// length prefix (`-1` for none).
fn write_item(w: &mut PacketWriter, item: Option<&ItemStack>) -> Result<(), EncodeError> {
    let Some(s) = item else {
        w.i16(-1);
        return Ok(());
    };
    let (id, damage) = v5_ids::item(s.id, s.damage);
    w.i16(id).i8(s.count).i16(damage);
    match &s.nbt {
        None => {
            w.i16(-1);
        }
        Some(nbt) => {
            let mut gz = GzEncoder::new(Vec::new(), Compression::default());
            krol_nbt::write(&mut gz, "", nbt).map_err(nbt_error)?;
            let bytes = gz.finish().map_err(|_| EncodeError::StringTooBig(0))?;
            w.short_byte_array(&bytes);
        }
    }
    Ok(())
}

/// Reads a 1.7 item stack.
fn read_item(r: &mut PacketReader<'_>) -> Result<Option<ItemStack>, DecodeError> {
    let id = r.i16()?;
    if id < 0 {
        return Ok(None);
    }
    let count = r.i8()?;
    let damage = r.i16()?;
    let len = r.i16()?;
    let nbt = if len < 0 {
        None
    } else {
        let bytes = r.bytes(len as usize)?;
        let mut raw = Vec::new();
        GzDecoder::new(bytes)
            .read_to_end(&mut raw)
            .map_err(|e| DecodeError::Nbt(e.to_string()))?;
        let (_, compound) =
            krol_nbt::read(&mut raw.as_slice()).map_err(|e| DecodeError::Nbt(e.to_string()))?;
        Some(compound)
    };
    Ok(Some(ItemStack {
        id,
        count,
        damage,
        nbt,
    }))
}

/// Writes 1.7 entity metadata, dropping entries 1.7 clients do not have.
fn write_metadata(
    w: &mut PacketWriter,
    entries: &[Metadata],
    is_player: bool,
) -> Result<(), EncodeError> {
    for m in entries {
        // 1.8 moved custom names to indices 2/3 for every entity and gave players skin
        // flags at 10; 1.7 players have none of these.
        // TODO(M5): move custom names of mobs to 1.7's indices 10/11.
        if m.index == 2 || m.index == 3 || (is_player && m.index == 10) {
            continue;
        }
        let ty = match m.value {
            MetadataValue::Byte(_) => 0,
            MetadataValue::Short(_) => 1,
            MetadataValue::Int(_) => 2,
            MetadataValue::Float(_) => 3,
            MetadataValue::String(_) => 4,
            MetadataValue::Item(_) => 5,
            MetadataValue::Position(..) => 6,
            // Rotations exist only for armor stands, which 1.7 does not have.
            MetadataValue::Rotation(..) => continue,
        };
        w.u8((ty << 5) | (m.index & 31));
        match &m.value {
            MetadataValue::Byte(v) => {
                w.i8(*v);
            }
            MetadataValue::Short(v) => {
                w.i16(*v);
            }
            MetadataValue::Int(v) => {
                w.i32(*v);
            }
            MetadataValue::Float(v) => {
                w.f32(*v);
            }
            MetadataValue::String(v) => {
                w.string(v)?;
            }
            MetadataValue::Item(v) => write_item(w, v.as_ref())?,
            MetadataValue::Position(x, y, z) => {
                w.i32(*x).i32(*y).i32(*z);
            }
            MetadataValue::Rotation(..) => {}
        }
    }
    w.u8(0x7f);
    Ok(())
}

/// Converts one column's 1.8 chunk data (16-bit block states, block light, optional
/// sky light, optional biomes) to 1.7's layout (block ids, metadata nibbles, block
/// light, sky light, biomes), uncompressed.
fn chunk_v5(mask: u16, data: &[u8], full: bool, sky: Option<bool>) -> (Vec<u8>, bool) {
    let n = mask.count_ones() as usize;
    let biomes = if full { 256 } else { 0 };
    let sky = sky.unwrap_or_else(|| data.len() >= n * (8192 + 2048 + 2048) + biomes && n > 0);
    let mut out = Vec::with_capacity(n * (4096 + 2048 * 3) + biomes);
    let blocks = data.get(..n * 8192).unwrap_or(&[]);
    let mut meta = vec![0u8; n * 2048];
    for (i, state) in blocks.chunks_exact(2).enumerate() {
        let state = u16::from_le_bytes([state[0], state[1]]);
        let (id, m) = v5_ids::block(state >> 4, (state & 15) as u8);
        out.push(id as u8);
        meta[i >> 1] |= if i & 1 == 0 { m & 15 } else { (m & 15) << 4 };
    }
    out.extend_from_slice(&meta);
    let light_len = n * 2048 * if sky { 2 } else { 1 };
    out.extend_from_slice(data.get(n * 8192..n * 8192 + light_len).unwrap_or(&[]));
    if full {
        let start = n * 8192 + light_len;
        out.extend_from_slice(data.get(start..start + 256).unwrap_or(&[0; 256]));
    }
    (out, sky)
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    // Writing to a Vec cannot fail.
    let _ = z.write_all(data);
    z.finish().unwrap_or_default()
}

impl V5Translator {
    /// A translator for a new play session.
    pub fn new() -> Self {
        Self::default()
    }

    /// Translates a clientbound packet into zero or more protocol-5 packet bodies
    /// (packet id followed by the payload).
    ///
    /// # Errors
    ///
    /// [`EncodeError::StringTooBig`] for oversized strings.
    pub fn clientbound(&mut self, p: &ClientboundPlay) -> Result<Vec<Vec<u8>>, EncodeError> {
        let mut out = Vec::new();
        let mut one = |w: PacketWriter| out.push(w.into_bytes());
        match p {
            ClientboundPlay::KeepAlive { id } => {
                let mut w = packet(0x00);
                w.i32(*id);
                one(w);
            }
            ClientboundPlay::JoinGame {
                entity_id,
                gamemode,
                dimension,
                difficulty,
                max_players,
                level_type,
                ..
            } => {
                self.own_entity = Some(*entity_id);
                self.player_entities.insert(*entity_id);
                let mut w = packet(0x01);
                w.i32(*entity_id)
                    .u8(gamemode_v5(*gamemode))
                    .i8(*dimension)
                    .u8(*difficulty)
                    .u8(*max_players);
                // "customized" worlds do not exist in 1.7.
                w.string(if level_type == "customized" {
                    "default"
                } else {
                    level_type
                })?;
                one(w);
            }
            ClientboundPlay::Chat { json, position } => {
                // 1.7 has no action bar.
                if *position != 2 {
                    let mut w = packet(0x02);
                    w.string(json)?;
                    one(w);
                }
            }
            ClientboundPlay::TimeUpdate {
                world_age,
                time_of_day,
            } => {
                let mut w = packet(0x03);
                w.i64(*world_age).i64(*time_of_day);
                one(w);
            }
            ClientboundPlay::SpawnPosition { pos } => {
                let mut w = packet(0x05);
                w.i32(pos.0).i32(pos.1).i32(pos.2);
                one(w);
            }
            ClientboundPlay::UpdateHealth {
                health,
                food,
                saturation,
            } => {
                let mut w = packet(0x06);
                w.f32(*health).i16(*food as i16).f32(*saturation);
                one(w);
            }
            ClientboundPlay::PositionLook {
                x,
                y,
                z,
                yaw,
                pitch,
                flags,
            } => {
                let rel = |bit: i8, base: f64, v: f64| if flags & bit != 0 { base + v } else { v };
                let (x, y, z) = (
                    rel(0x01, self.pos.0, *x),
                    rel(0x02, self.pos.1, *y),
                    rel(0x04, self.pos.2, *z),
                );
                let yaw = if flags & 0x08 != 0 {
                    self.rot.0 + yaw
                } else {
                    *yaw
                };
                let pitch = if flags & 0x10 != 0 {
                    self.rot.1 + pitch
                } else {
                    *pitch
                };
                self.pos = (x, y, z);
                self.rot = (yaw, pitch);
                let mut w = packet(0x08);
                w.f64(x)
                    .f64(y + EYE_HEIGHT)
                    .f64(z)
                    .f32(yaw)
                    .f32(pitch)
                    .bool(false);
                one(w);
            }
            ClientboundPlay::HeldItemChange { slot } => {
                let mut w = packet(0x09);
                w.i8(*slot);
                one(w);
            }
            ClientboundPlay::EntityStatus { entity_id, status } => {
                let mut w = packet(0x1a);
                w.i32(*entity_id).i8(*status);
                one(w);
            }
            ClientboundPlay::SetExperience { bar, level, total } => {
                let mut w = packet(0x1f);
                w.f32(*bar).i16(*level as i16).i16(*total as i16);
                one(w);
            }
            ClientboundPlay::ChunkData { column, full } => {
                let (data, _) = chunk_v5(column.mask, &column.data, *full, None);
                let compressed = zlib(&data);
                let mut w = packet(0x21);
                w.i32(column.x)
                    .i32(column.z)
                    .bool(*full)
                    .u16(column.mask)
                    .u16(0)
                    .i32(compressed.len() as i32)
                    .bytes(&compressed);
                one(w);
            }
            ClientboundPlay::MapChunkBulk { sky_light, columns } => {
                let mut data = Vec::new();
                for ChunkColumn { mask, data: d, .. } in columns {
                    data.extend(chunk_v5(*mask, d, true, Some(*sky_light)).0);
                }
                let compressed = zlib(&data);
                let mut w = packet(0x26);
                w.i16(columns.len() as i16)
                    .i32(compressed.len() as i32)
                    .bool(*sky_light)
                    .bytes(&compressed);
                for c in columns {
                    w.i32(c.x).i32(c.z).u16(c.mask).u16(0);
                }
                one(w);
            }
            ClientboundPlay::ChangeGameState { reason, value } => {
                let value = if *reason == 3 && *value == 3.0 {
                    2.0
                } else {
                    *value
                };
                let mut w = packet(0x2b);
                w.u8(*reason).f32(value);
                one(w);
            }
            ClientboundPlay::SetSlot { window, slot, item } => {
                let mut w = packet(0x2f);
                w.i8(*window).i16(*slot);
                write_item(&mut w, item.as_ref())?;
                one(w);
            }
            ClientboundPlay::WindowItems { window, items } => {
                let mut w = packet(0x30);
                w.u8(*window).i16(items.len() as i16);
                for item in items {
                    write_item(&mut w, item.as_ref())?;
                }
                one(w);
            }
            ClientboundPlay::PlayerListItem(action) => {
                for (name, online, ping) in self.player_list(action) {
                    let mut w = packet(0x38);
                    w.string(&name)?;
                    w.bool(online).i16(ping);
                    one(w);
                }
            }
            ClientboundPlay::PlayerAbilities {
                flags,
                flying_speed,
                walking_speed,
            } => {
                let mut w = packet(0x39);
                w.i8(*flags).f32(*flying_speed).f32(*walking_speed);
                one(w);
            }
            ClientboundPlay::PluginMessage { channel, data } => {
                let mut w = packet(0x3f);
                w.string(channel)?;
                w.short_byte_array(data);
                one(w);
            }
            ClientboundPlay::Disconnect(json) => {
                let mut w = packet(0x40);
                w.string(json)?;
                one(w);
            }
            // 1.8 only.
            ClientboundPlay::ServerDifficulty { .. } | ClientboundPlay::WorldBorderInit { .. } => {}
            ClientboundPlay::Statistics(stats) => {
                let mut w = packet(0x37);
                w.varint(stats.len() as i32);
                for (name, value) in stats {
                    w.string(name)?.varint(*value);
                }
                one(w);
            }
            ClientboundPlay::BlockChange { pos, state } => {
                let (id, meta) = v5_ids::block((*state >> 4) as u16, (*state & 15) as u8);
                let mut w = packet(0x23);
                w.i32(pos.0)
                    .u8(pos.1 as u8)
                    .i32(pos.2)
                    .varint(i32::from(id))
                    .u8(meta);
                one(w);
            }
            ClientboundPlay::MultiBlockChange {
                chunk_x,
                chunk_z,
                records,
            } => {
                let mut w = packet(0x22);
                w.i32(*chunk_x)
                    .i32(*chunk_z)
                    .i16(records.len() as i16)
                    .i32(records.len() as i32 * 4);
                for (x, y, z, state) in records {
                    let (id, meta) = v5_ids::block((*state >> 4) as u16, (*state & 15) as u8);
                    let record = (u32::from(*x & 15) << 28)
                        | (u32::from(*z & 15) << 24)
                        | (u32::from(*y) << 16)
                        | (u32::from(id) << 4)
                        | u32::from(meta);
                    w.i32(record as i32);
                }
                one(w);
            }
            ClientboundPlay::SoundEffect {
                name,
                x,
                y,
                z,
                volume,
                pitch,
            } => {
                let mut w = packet(0x29);
                w.string(name)?;
                w.i32(*x).i32(*y).i32(*z).f32(*volume).u8(*pitch);
                one(w);
            }
            ClientboundPlay::Effect {
                id,
                pos,
                data,
                disable_relative_volume,
            } => {
                // Block break particles carry a block state (id | meta << 12).
                let data = if *id == 2001 {
                    let (b, m) = v5_ids::block((*data & 4095) as u16, ((*data >> 12) & 15) as u8);
                    i32::from(b) | i32::from(m) << 12
                } else {
                    *data
                };
                let mut w = packet(0x28);
                w.i32(*id)
                    .i32(pos.0)
                    .i8(pos.1 as i8)
                    .i32(pos.2)
                    .i32(data)
                    .bool(*disable_relative_volume);
                one(w);
            }
            ClientboundPlay::EntityMetadata {
                entity_id,
                metadata,
            } => {
                let mut w = packet(0x1c);
                w.i32(*entity_id);
                write_metadata(&mut w, metadata, self.player_entities.contains(entity_id))?;
                one(w);
            }
            ClientboundPlay::EntityProperties {
                entity_id,
                properties,
            } => {
                let mut w = packet(0x20);
                w.i32(*entity_id).i32(properties.len() as i32);
                for (key, value) in properties {
                    w.string(key)?.f64(*value).i16(0);
                }
                one(w);
            }
            ClientboundPlay::EntityEquipment {
                entity_id,
                slot,
                item,
            } => {
                let mut w = packet(0x04);
                w.i32(*entity_id).i16(*slot);
                write_item(&mut w, item.as_ref())?;
                one(w);
            }
            ClientboundPlay::Animation {
                entity_id,
                animation,
            } => {
                let mut w = packet(0x0b);
                w.varint(*entity_id).u8(*animation);
                one(w);
            }
            ClientboundPlay::SpawnPlayer {
                entity_id,
                uuid,
                x,
                y,
                z,
                yaw,
                pitch,
                current_item,
                metadata,
            } => {
                self.player_entities.insert(*entity_id);
                let (name, properties) = self.profiles.get(uuid).cloned().unwrap_or_default();
                let mut w = packet(0x0c);
                w.varint(*entity_id);
                w.string(&uuid_string(*uuid))?;
                w.string(&name)?;
                w.varint(properties.len() as i32);
                for (name, value, signature) in &properties {
                    w.string(name)?.string(value)?;
                    w.string(signature.as_deref().unwrap_or(""))?;
                }
                let item = if *current_item > 0 {
                    v5_ids::item(*current_item, 0).0
                } else {
                    0
                };
                w.i32(*x).i32(*y).i32(*z).i8(*yaw).i8(*pitch).i16(item);
                write_metadata(&mut w, metadata, true)?;
                one(w);
            }
            ClientboundPlay::DestroyEntities(ids) => {
                for id in ids {
                    self.player_entities.remove(id);
                }
                // The count is a byte in 1.7.
                for chunk in ids.chunks(127) {
                    let mut w = packet(0x13);
                    w.i8(chunk.len() as i8);
                    for id in chunk {
                        w.i32(*id);
                    }
                    one(w);
                }
            }
            ClientboundPlay::EntityRelMove {
                entity_id,
                dx,
                dy,
                dz,
                ..
            } => {
                let mut w = packet(0x15);
                w.i32(*entity_id).i8(*dx).i8(*dy).i8(*dz);
                one(w);
            }
            ClientboundPlay::EntityLook {
                entity_id,
                yaw,
                pitch,
                ..
            } => {
                let mut w = packet(0x16);
                w.i32(*entity_id).i8(*yaw).i8(*pitch);
                one(w);
            }
            ClientboundPlay::EntityLookMove {
                entity_id,
                dx,
                dy,
                dz,
                yaw,
                pitch,
                ..
            } => {
                let mut w = packet(0x17);
                w.i32(*entity_id)
                    .i8(*dx)
                    .i8(*dy)
                    .i8(*dz)
                    .i8(*yaw)
                    .i8(*pitch);
                one(w);
            }
            ClientboundPlay::EntityTeleport {
                entity_id,
                x,
                y,
                z,
                yaw,
                pitch,
                ..
            } => {
                let mut w = packet(0x18);
                w.i32(*entity_id)
                    .i32(*x)
                    .i32(*y)
                    .i32(*z)
                    .i8(*yaw)
                    .i8(*pitch);
                one(w);
            }
            ClientboundPlay::Particle {
                id,
                pos,
                offset,
                speed,
                count,
                args,
                ..
            } => {
                // 1.7 names particles; block dust carries the block in its name.
                // TODO(M5): the other particle types.
                if *id == 38
                    && let Some(state) = args.first()
                {
                    let (b, m) = v5_ids::block((state & 4095) as u16, ((state >> 12) & 15) as u8);
                    let mut w = packet(0x2a);
                    w.string(&format!("blockdust_{b}_{m}"))?;
                    w.f32(pos.0)
                        .f32(pos.1)
                        .f32(pos.2)
                        .f32(offset.0)
                        .f32(offset.1)
                        .f32(offset.2)
                        .f32(*speed)
                        .i32(*count);
                    one(w);
                }
            }
            ClientboundPlay::EntityVelocity { entity_id, x, y, z } => {
                let mut w = packet(0x12);
                w.i32(*entity_id).i16(*x).i16(*y).i16(*z);
                one(w);
            }
            ClientboundPlay::EntityHeadLook { entity_id, yaw } => {
                let mut w = packet(0x19);
                w.i32(*entity_id).i8(*yaw);
                one(w);
            }
        }
        Ok(out)
    }

    /// Updates the known profiles and returns the 1.7 entries `(name, online, ping)`.
    fn player_list(&mut self, action: &PlayerListAction) -> Vec<(String, bool, i16)> {
        let ping = |p: i32| p.clamp(0, i32::from(i16::MAX)) as i16;
        match action {
            PlayerListAction::Add(entries) => entries
                .iter()
                .map(|e| {
                    self.profiles
                        .insert(e.uuid, (e.name.clone(), e.properties.clone()));
                    // TODO(M7): show display names (1.7 only takes plain names of up
                    // to 16 characters).
                    (e.name.clone(), true, ping(e.ping))
                })
                .collect(),
            PlayerListAction::UpdateLatency(entries) => entries
                .iter()
                .filter_map(|(uuid, p)| {
                    self.profiles
                        .get(uuid)
                        .map(|(name, _)| (name.clone(), true, ping(*p)))
                })
                .collect(),
            PlayerListAction::UpdateGamemode(_) | PlayerListAction::UpdateDisplayName(_) => {
                Vec::new()
            }
            PlayerListAction::Remove(uuids) => uuids
                .iter()
                .filter_map(|uuid| self.profiles.remove(uuid))
                .map(|(name, _)| (name, false, 0))
                .collect(),
        }
    }

    /// Decodes a protocol-5 serverbound frame into the protocol-47 model, with vanilla
    /// 1.7's field limits.
    ///
    /// # Errors
    ///
    /// [`DecodeError::BadPacketId`], field errors, or [`DecodeError::TrailingBytes`].
    pub fn serverbound(&mut self, frame: &[u8]) -> Result<ServerboundPlay, DecodeError> {
        let mut r = PacketReader::new(frame);
        let id = r.varint()?;
        let p = match id {
            0x00 => ServerboundPlay::KeepAlive { id: r.i32()? },
            0x01 => ServerboundPlay::Chat {
                message: r.string(100)?,
            },
            0x02 => ServerboundPlay::UseEntity {
                target: r.i32()?,
                action: i32::from(r.i8()?),
                target_pos: None,
            },
            0x03 => ServerboundPlay::Player {
                on_ground: r.bool()?,
            },
            0x04 => {
                let x = r.f64()?;
                let y = r.f64()?;
                let _head_y = r.f64()?;
                let z = r.f64()?;
                self.pos = (x, y, z);
                ServerboundPlay::Position {
                    x,
                    y,
                    z,
                    on_ground: r.bool()?,
                }
            }
            0x05 => {
                let (yaw, pitch) = (r.f32()?, r.f32()?);
                self.rot = (yaw, pitch);
                ServerboundPlay::Look {
                    yaw,
                    pitch,
                    on_ground: r.bool()?,
                }
            }
            0x06 => {
                let x = r.f64()?;
                let y = r.f64()?;
                let _head_y = r.f64()?;
                let z = r.f64()?;
                let (yaw, pitch) = (r.f32()?, r.f32()?);
                self.pos = (x, y, z);
                self.rot = (yaw, pitch);
                ServerboundPlay::PositionLook {
                    x,
                    y,
                    z,
                    yaw,
                    pitch,
                    on_ground: r.bool()?,
                }
            }
            0x07 => {
                let status = r.i8()?;
                let pos = (r.i32()?, i32::from(r.u8()?), r.i32()?);
                ServerboundPlay::Digging {
                    status,
                    pos,
                    face: r.i8()?,
                }
            }
            0x08 => {
                let pos = (r.i32()?, i32::from(r.u8()?), r.i32()?);
                let face = r.i8()?;
                let item = read_item(&mut r)?;
                let cursor = (r.u8()? as i8, r.u8()? as i8, r.u8()? as i8);
                ServerboundPlay::BlockPlacement {
                    pos,
                    face,
                    item,
                    cursor,
                }
            }
            0x09 => ServerboundPlay::HeldItemChange { slot: r.i16()? },
            0x0a => {
                // Entity id and animation (always "swing arm" from vanilla clients).
                r.i32()?;
                r.i8()?;
                ServerboundPlay::Animation
            }
            0x0b => ServerboundPlay::EntityAction {
                entity: r.i32()?,
                // 1.7 counts actions from 1.
                action: i32::from(r.i8()?) - 1,
                jump_boost: r.i32()?,
            },
            0x0c => {
                let sideways = r.f32()?;
                let forward = r.f32()?;
                let jump = r.bool()?;
                let unmount = r.bool()?;
                ServerboundPlay::SteerVehicle {
                    sideways,
                    forward,
                    flags: u8::from(jump) | u8::from(unmount) << 1,
                }
            }
            0x0d => ServerboundPlay::CloseWindow { window: r.i8()? },
            0x0e => ServerboundPlay::ClickWindow {
                window: r.i8()?,
                slot: r.i16()?,
                button: r.i8()?,
                action: r.i16()?,
                mode: r.i8()?,
                item: read_item(&mut r)?,
            },
            0x0f => ServerboundPlay::ConfirmTransaction {
                window: r.i8()?,
                action: r.i16()?,
                accepted: r.bool()?,
            },
            0x10 => ServerboundPlay::CreativeInventoryAction {
                slot: r.i16()?,
                item: read_item(&mut r)?,
            },
            0x11 => ServerboundPlay::EnchantItem {
                window: r.i8()?,
                enchantment: r.i8()?,
            },
            0x12 => {
                let pos = (r.i32()?, i32::from(r.i16()?), r.i32()?);
                // 1.7 sends plain text; 1.8 clients send each line as a JSON string.
                let mut line = || r.string(15).map(|s| json_string(&s));
                let lines = [line()?, line()?, line()?, line()?];
                ServerboundPlay::UpdateSign { pos, lines }
            }
            0x13 => ServerboundPlay::PlayerAbilities {
                flags: r.i8()?,
                flying_speed: r.f32()?,
                walking_speed: r.f32()?,
            },
            0x14 => ServerboundPlay::TabComplete {
                text: r.string(32767)?,
                looking_at: None,
            },
            0x15 => {
                let locale = r.string(7)?;
                let view_distance = r.i8()?;
                let chat_mode = i32::from(r.i8()?);
                let chat_colors = r.bool()?;
                let _difficulty = r.u8()?;
                let show_cape = r.bool()?;
                // 1.7 clients only draw the hat layer (and the cape if enabled).
                ServerboundPlay::ClientSettings {
                    locale,
                    view_distance,
                    chat_mode,
                    chat_colors,
                    skin_parts: 0x40 | u8::from(show_cape),
                }
            }
            0x16 => ServerboundPlay::ClientStatus {
                action: i32::from(r.i8()?),
            },
            0x17 => {
                let channel = r.string(20)?;
                let len = r.i16()?;
                if len < 0 {
                    return Err(DecodeError::InvalidValue(
                        "Payload may not be larger than 32767 bytes".into(),
                    ));
                }
                let data = r.bytes(len as usize)?;
                ServerboundPlay::PluginMessage {
                    data: plugin_data_v47(&channel, data)?,
                    channel,
                }
            }
            other => return Err(DecodeError::BadPacketId(other)),
        };
        finish(&r, ConnectionState::Play, id)?;
        Ok(p)
    }
}

/// 1.7 has no spectator mode; spectators play as adventure mode (flying comes from the
/// abilities packet). The hardcore flag (8) is kept.
fn gamemode_v5(gamemode: u8) -> u8 {
    if gamemode & 7 == 3 {
        (gamemode & 8) | 2
    } else {
        gamemode
    }
}

/// A JSON string literal (what a 1.8 client sends for a plain sign line).
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Converts plugin message payloads whose format changed in 1.8.
fn plugin_data_v47(channel: &str, data: &[u8]) -> Result<Vec<u8>, DecodeError> {
    match channel {
        // Book editing and signing carry an item stack.
        "MC|BEdit" | "MC|BSign" => {
            let mut r = PacketReader::new(data);
            let item = read_item(&mut r)?;
            let mut w = PacketWriter::new();
            match item {
                None => {
                    w.i16(-1);
                }
                Some(s) => {
                    w.i16(s.id).i8(s.count).i16(s.damage);
                    w.nbt(s.nbt.as_ref())
                        .map_err(|e| DecodeError::InvalidValue(e.to_string()))?;
                }
            }
            Ok(w.into_bytes())
        }
        // Command blocks: 1.8 adds a type byte (0 = block) and the track-output flag.
        "MC|AdvCdm" => {
            let mut w = PacketWriter::new();
            w.u8(0).bytes(data).bool(true);
            Ok(w.into_bytes())
        }
        _ => Ok(data.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::play::PlayerListAdd;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn entity_and_player_packets() {
        let mut t = V5Translator::new();
        let out = t
            .clientbound(&ClientboundPlay::PlayerListItem(PlayerListAction::Add(
                vec![PlayerListAdd {
                    uuid: 0xa762f560_4fce_3236_812a_b80efff0b62b,
                    name: "jeb_".into(),
                    properties: vec![],
                    gamemode: 2,
                    ping: 0,
                    display_name: None,
                }],
            )))
            .unwrap();
        assert_eq!(hex(&out[0]), "38046a65625f010000");
        let out = t
            .clientbound(&ClientboundPlay::SpawnPlayer {
                entity_id: 1,
                uuid: 0xa762f560_4fce_3236_812a_b80efff0b62b,
                x: -15088,
                y: 128,
                z: -13904,
                yaw: 0,
                pitch: 0,
                current_item: 0,
                metadata: vec![
                    Metadata {
                        index: 0,
                        value: MetadataValue::Byte(0),
                    },
                    Metadata {
                        index: 2,
                        value: MetadataValue::String(String::new()),
                    },
                    Metadata {
                        index: 10,
                        value: MetadataValue::Byte(0x7f),
                    },
                ],
            })
            .unwrap();
        assert_eq!(
            hex(&out[0]),
            format!(
                "0c0124{}046a65625f00ffffc51000000080ffffc9b00000000000007f",
                hex(b"a762f560-4fce-3236-812a-b80efff0b62b")
            )
        );
        let out = t
            .clientbound(&ClientboundPlay::EntityRelMove {
                entity_id: 1,
                dx: 8,
                dy: 0,
                dz: 0,
                on_ground: true,
            })
            .unwrap();
        assert_eq!(hex(&out[0]), "1500000001080000");
        let out = t
            .clientbound(&ClientboundPlay::PlayerListItem(PlayerListAction::Remove(
                vec![0xa762f560_4fce_3236_812a_b80efff0b62b],
            )))
            .unwrap();
        assert_eq!(hex(&out[0]), "38046a65625f000000");
        assert!(
            t.clientbound(&ClientboundPlay::ServerDifficulty { difficulty: 1 })
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn position_look_is_absolute_eye_height() {
        let mut t = V5Translator::new();
        t.serverbound(&{
            let mut w = PacketWriter::new();
            w.varint(0x04)
                .f64(10.0)
                .f64(64.0)
                .f64(65.62)
                .f64(-3.0)
                .bool(true);
            w.into_bytes()
        })
        .unwrap();
        let out = t
            .clientbound(&ClientboundPlay::PositionLook {
                x: 1.0,
                y: 0.0,
                z: 0.0,
                yaw: 0.0,
                pitch: 0.0,
                flags: 0x01 | 0x02 | 0x04,
            })
            .unwrap();
        let mut r = PacketReader::new(&out[0]);
        assert_eq!(r.varint().unwrap(), 0x08);
        assert_eq!(r.f64().unwrap(), 11.0);
        assert_eq!(r.f64().unwrap(), 64.0 + EYE_HEIGHT);
        assert_eq!(r.f64().unwrap(), -3.0);
    }

    #[test]
    fn serverbound_v5() {
        let mut t = V5Translator::new();
        let mut w = PacketWriter::new();
        w.varint(0x07).i8(0).i32(-469).u8(3).i32(-435).i8(1);
        assert_eq!(
            t.serverbound(w.as_bytes()).unwrap(),
            ServerboundPlay::Digging {
                status: 0,
                pos: (-469, 3, -435),
                face: 1
            }
        );
        let mut w = PacketWriter::new();
        w.varint(0x15).string("en_US").unwrap();
        w.i8(10).i8(0).bool(true).u8(2).bool(true);
        assert_eq!(
            t.serverbound(w.as_bytes()).unwrap(),
            ServerboundPlay::ClientSettings {
                locale: "en_US".into(),
                view_distance: 10,
                chat_mode: 0,
                chat_colors: true,
                skin_parts: 0x41
            }
        );
        let mut w = PacketWriter::new();
        w.varint(0x01).string(&"a".repeat(101)).unwrap();
        assert!(t.serverbound(w.as_bytes()).is_err());
        let mut w = PacketWriter::new();
        w.varint(0x17).string("MC|Brand").unwrap();
        w.short_byte_array(b"\x07vanilla");
        assert_eq!(
            t.serverbound(w.as_bytes()).unwrap(),
            ServerboundPlay::PluginMessage {
                channel: "MC|Brand".into(),
                data: b"\x07vanilla".to_vec()
            }
        );
    }

    #[test]
    fn items_round_trip_through_gzip() {
        let mut nbt = krol_nbt::Compound::new();
        nbt.insert("RepairCost", krol_nbt::Tag::Int(1));
        let stack = ItemStack {
            id: 276,
            count: 1,
            damage: 3,
            nbt: Some(nbt),
        };
        let mut w = PacketWriter::new();
        write_item(&mut w, Some(&stack)).unwrap();
        let bytes = w.into_bytes();
        assert_eq!(
            read_item(&mut PacketReader::new(&bytes)).unwrap(),
            Some(stack)
        );
    }

    #[test]
    fn chunk_layout() {
        // One section: stone (1:0) at index 0, granite (1:1) at index 1, wool 35:14 at 2.
        let mut data = vec![0u8; 8192 + 2048 + 2048 + 256];
        let mut put =
            |i: usize, state: u16| data[i * 2..i * 2 + 2].copy_from_slice(&state.to_le_bytes());
        put(0, 1 << 4);
        put(1, 1 << 4 | 1);
        put(2, 35 << 4 | 14);
        data[8192] = 0xab; // block light
        let (v5, sky) = chunk_v5(1, &data, true, None);
        assert!(sky);
        assert_eq!(v5.len(), 4096 + 2048 * 3 + 256);
        assert_eq!(&v5[..3], &[1, 1, 35]);
        assert_eq!(v5[4096], 0x00);
        assert_eq!(v5[4097], 0x0e);
        assert_eq!(v5[4096 + 2048], 0xab);
    }
}
