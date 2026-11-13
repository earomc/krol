//! Play state packets in protocol 47 form (the canonical model).
//!
//! Every serverbound packet is decoded with vanilla's field limits so malformed input is
//! rejected exactly where vanilla rejects it. Clientbound packets cover what the server
//! sends so far; [`ClientboundPlay::decode`] is for test clients and returns `None` for
//! packets it does not model.

use super::{ConnectionState, finish};
use crate::buf::{PacketReader, PacketWriter};
use crate::{DecodeError, EncodeError, ProtocolVersion};

/// An item stack as sent on the wire (`Slot`).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemStack {
    /// Item id.
    pub id: i16,
    /// Stack size.
    pub count: i8,
    /// Damage / metadata.
    pub damage: i16,
    /// Item NBT.
    pub nbt: Option<krol_nbt::Compound>,
}

impl ItemStack {
    fn read(r: &mut PacketReader<'_>) -> Result<Option<Self>, DecodeError> {
        let id = r.i16()?;
        if id < 0 {
            return Ok(None);
        }
        Ok(Some(Self {
            id,
            count: r.i8()?,
            damage: r.i16()?,
            nbt: r.nbt()?,
        }))
    }

    fn write(w: &mut PacketWriter, item: Option<&Self>) -> Result<(), EncodeError> {
        match item {
            None => {
                w.i16(-1);
            }
            Some(s) => {
                w.i16(s.id).i8(s.count).i16(s.damage);
                w.nbt(s.nbt.as_ref())?;
            }
        }
        Ok(())
    }
}

/// A block position.
pub type BlockPos = (i32, i32, i32);

/// Serverbound play packets (protocol 47 ids).
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)] // fields mirror the protocol documentation
pub enum ServerboundPlay {
    /// `0x00`
    KeepAlive { id: i32 },
    /// `0x01` (message at most 100 characters)
    Chat { message: String },
    /// `0x02`
    UseEntity {
        target: i32,
        action: i32,
        target_pos: Option<(f32, f32, f32)>,
    },
    /// `0x03`
    Player { on_ground: bool },
    /// `0x04`
    Position {
        x: f64,
        y: f64,
        z: f64,
        on_ground: bool,
    },
    /// `0x05`
    Look {
        yaw: f32,
        pitch: f32,
        on_ground: bool,
    },
    /// `0x06`
    PositionLook {
        x: f64,
        y: f64,
        z: f64,
        yaw: f32,
        pitch: f32,
        on_ground: bool,
    },
    /// `0x07`
    Digging { status: i8, pos: BlockPos, face: i8 },
    /// `0x08`
    BlockPlacement {
        pos: BlockPos,
        face: i8,
        item: Option<ItemStack>,
        cursor: (i8, i8, i8),
    },
    /// `0x09`
    HeldItemChange { slot: i16 },
    /// `0x0A`
    Animation,
    /// `0x0B`
    EntityAction {
        entity: i32,
        action: i32,
        jump_boost: i32,
    },
    /// `0x0C`
    SteerVehicle {
        sideways: f32,
        forward: f32,
        flags: u8,
    },
    /// `0x0D`
    CloseWindow { window: i8 },
    /// `0x0E`
    ClickWindow {
        window: i8,
        slot: i16,
        button: i8,
        action: i16,
        mode: i8,
        item: Option<ItemStack>,
    },
    /// `0x0F`
    ConfirmTransaction {
        window: i8,
        action: i16,
        accepted: bool,
    },
    /// `0x10`
    CreativeInventoryAction { slot: i16, item: Option<ItemStack> },
    /// `0x11`
    EnchantItem { window: i8, enchantment: i8 },
    /// `0x12` (four JSON chat lines)
    UpdateSign { pos: BlockPos, lines: [String; 4] },
    /// `0x13`
    PlayerAbilities {
        flags: i8,
        flying_speed: f32,
        walking_speed: f32,
    },
    /// `0x14`
    TabComplete {
        text: String,
        looking_at: Option<BlockPos>,
    },
    /// `0x15`
    ClientSettings {
        locale: String,
        view_distance: i8,
        chat_mode: i32,
        chat_colors: bool,
        skin_parts: u8,
    },
    /// `0x16`
    ClientStatus { action: i32 },
    /// `0x17` (data at most 32767 bytes)
    PluginMessage { channel: String, data: Vec<u8> },
    /// `0x18`
    Spectate { target: u128 },
    /// `0x19`
    ResourcePackStatus { hash: String, result: i32 },
}

impl ServerboundPlay {
    /// Decodes a play-state frame body.
    ///
    /// # Errors
    ///
    /// [`DecodeError::BadPacketId`], field errors, or [`DecodeError::TrailingBytes`].
    pub fn decode(frame: &[u8], _version: ProtocolVersion) -> Result<Self, DecodeError> {
        let mut r = PacketReader::new(frame);
        let id = r.varint()?;
        let p = match id {
            0x00 => Self::KeepAlive { id: r.varint()? },
            0x01 => Self::Chat {
                message: r.string(100)?,
            },
            0x02 => {
                let target = r.varint()?;
                let action = r.varint()?;
                let target_pos = if action == 2 {
                    Some((r.f32()?, r.f32()?, r.f32()?))
                } else {
                    None
                };
                Self::UseEntity {
                    target,
                    action,
                    target_pos,
                }
            }
            0x03 => Self::Player {
                on_ground: r.bool()?,
            },
            0x04 => Self::Position {
                x: r.f64()?,
                y: r.f64()?,
                z: r.f64()?,
                on_ground: r.bool()?,
            },
            0x05 => Self::Look {
                yaw: r.f32()?,
                pitch: r.f32()?,
                on_ground: r.bool()?,
            },
            0x06 => Self::PositionLook {
                x: r.f64()?,
                y: r.f64()?,
                z: r.f64()?,
                yaw: r.f32()?,
                pitch: r.f32()?,
                on_ground: r.bool()?,
            },
            0x07 => Self::Digging {
                status: r.i8()?,
                pos: r.position()?,
                face: r.i8()?,
            },
            0x08 => Self::BlockPlacement {
                pos: r.position()?,
                face: r.i8()?,
                item: ItemStack::read(&mut r)?,
                cursor: (r.i8()?, r.i8()?, r.i8()?),
            },
            0x09 => Self::HeldItemChange { slot: r.i16()? },
            0x0a => Self::Animation,
            0x0b => Self::EntityAction {
                entity: r.varint()?,
                action: r.varint()?,
                jump_boost: r.varint()?,
            },
            0x0c => Self::SteerVehicle {
                sideways: r.f32()?,
                forward: r.f32()?,
                flags: r.u8()?,
            },
            0x0d => Self::CloseWindow { window: r.i8()? },
            0x0e => Self::ClickWindow {
                window: r.i8()?,
                slot: r.i16()?,
                button: r.i8()?,
                action: r.i16()?,
                mode: r.i8()?,
                item: ItemStack::read(&mut r)?,
            },
            0x0f => Self::ConfirmTransaction {
                window: r.i8()?,
                action: r.i16()?,
                accepted: r.bool()?,
            },
            0x10 => Self::CreativeInventoryAction {
                slot: r.i16()?,
                item: ItemStack::read(&mut r)?,
            },
            0x11 => Self::EnchantItem {
                window: r.i8()?,
                enchantment: r.i8()?,
            },
            0x12 => Self::UpdateSign {
                pos: r.position()?,
                lines: [
                    r.string(32767)?,
                    r.string(32767)?,
                    r.string(32767)?,
                    r.string(32767)?,
                ],
            },
            0x13 => Self::PlayerAbilities {
                flags: r.i8()?,
                flying_speed: r.f32()?,
                walking_speed: r.f32()?,
            },
            0x14 => {
                let text = r.string(32767)?;
                let looking_at = if r.bool()? { Some(r.position()?) } else { None };
                Self::TabComplete { text, looking_at }
            }
            0x15 => Self::ClientSettings {
                locale: r.string(7)?,
                view_distance: r.i8()?,
                chat_mode: r.varint()?,
                chat_colors: r.bool()?,
                skin_parts: r.u8()?,
            },
            0x16 => Self::ClientStatus {
                action: r.varint()?,
            },
            0x17 => {
                let channel = r.string(20)?;
                let rest = r.remaining();
                if rest > 32767 {
                    return Err(DecodeError::InvalidValue(
                        "Payload may not be larger than 32767 bytes".into(),
                    ));
                }
                Self::PluginMessage {
                    channel,
                    data: r.bytes(rest)?.to_vec(),
                }
            }
            0x18 => Self::Spectate { target: r.uuid()? },
            0x19 => Self::ResourcePackStatus {
                hash: r.string(40)?,
                result: r.varint()?,
            },
            other => return Err(DecodeError::BadPacketId(other)),
        };
        finish(&r, ConnectionState::Play, id)?;
        Ok(p)
    }

    /// Encodes the packet (used by test clients). Only the packets a test client sends
    /// are supported; others return `None`.
    ///
    /// # Errors
    ///
    /// [`EncodeError::StringTooBig`] for oversized strings.
    pub fn encode(&self) -> Result<Option<Vec<u8>>, EncodeError> {
        let mut w = PacketWriter::new();
        match self {
            Self::KeepAlive { id } => {
                w.varint(0x00).varint(*id);
            }
            Self::Chat { message } => {
                w.varint(0x01).string(message)?;
            }
            Self::Player { on_ground } => {
                w.varint(0x03).bool(*on_ground);
            }
            Self::Position { x, y, z, on_ground } => {
                w.varint(0x04).f64(*x).f64(*y).f64(*z).bool(*on_ground);
            }
            Self::Look {
                yaw,
                pitch,
                on_ground,
            } => {
                w.varint(0x05).f32(*yaw).f32(*pitch).bool(*on_ground);
            }
            Self::PositionLook {
                x,
                y,
                z,
                yaw,
                pitch,
                on_ground,
            } => {
                w.varint(0x06)
                    .f64(*x)
                    .f64(*y)
                    .f64(*z)
                    .f32(*yaw)
                    .f32(*pitch)
                    .bool(*on_ground);
            }
            Self::ClientSettings {
                locale,
                view_distance,
                chat_mode,
                chat_colors,
                skin_parts,
            } => {
                w.varint(0x15).string(locale)?;
                w.i8(*view_distance)
                    .varint(*chat_mode)
                    .bool(*chat_colors)
                    .u8(*skin_parts);
            }
            Self::ClientStatus { action } => {
                w.varint(0x16).varint(*action);
            }
            Self::PluginMessage { channel, data } => {
                w.varint(0x17).string(channel)?;
                w.bytes(data);
            }
            _ => return Ok(None),
        }
        Ok(Some(w.into_bytes()))
    }
}

/// One entry of a Player List Item packet (action 0, add player).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerListAdd {
    /// Player UUID.
    pub uuid: u128,
    /// Player name.
    pub name: String,
    /// Profile properties `(name, value, signature)`.
    pub properties: Vec<(String, String, Option<String>)>,
    /// Game mode id.
    pub gamemode: i32,
    /// Latency in ms.
    pub ping: i32,
    /// Optional display name (JSON chat).
    pub display_name: Option<String>,
}

/// Player List Item actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerListAction {
    /// `0`: add players.
    Add(Vec<PlayerListAdd>),
    /// `1`: update game modes.
    UpdateGamemode(Vec<(u128, i32)>),
    /// `2`: update latencies.
    UpdateLatency(Vec<(u128, i32)>),
    /// `3`: update display names.
    UpdateDisplayName(Vec<(u128, Option<String>)>),
    /// `4`: remove players.
    Remove(Vec<u128>),
}

/// Data for one chunk column in a Chunk Data / Map Chunk Bulk packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkColumn {
    /// Chunk x.
    pub x: i32,
    /// Chunk z.
    pub z: i32,
    /// Bit mask of included sections.
    pub mask: u16,
    /// Serialized section data (see `krol-world`'s packet encoder).
    pub data: Vec<u8>,
}

/// Clientbound play packets (protocol 47 ids).
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)] // fields mirror the protocol documentation
pub enum ClientboundPlay {
    /// `0x00`
    KeepAlive { id: i32 },
    /// `0x01`
    JoinGame {
        entity_id: i32,
        gamemode: u8,
        dimension: i8,
        difficulty: u8,
        max_players: u8,
        level_type: String,
        reduced_debug_info: bool,
    },
    /// `0x02` (position: 0 chat, 1 system, 2 action bar)
    Chat { json: String, position: i8 },
    /// `0x03`
    TimeUpdate { world_age: i64, time_of_day: i64 },
    /// `0x05`
    SpawnPosition { pos: BlockPos },
    /// `0x06`
    UpdateHealth {
        health: f32,
        food: i32,
        saturation: f32,
    },
    /// `0x08` (flags mark relative fields)
    PositionLook {
        x: f64,
        y: f64,
        z: f64,
        yaw: f32,
        pitch: f32,
        flags: i8,
    },
    /// `0x09`
    HeldItemChange { slot: i8 },
    /// `0x1A`
    EntityStatus { entity_id: i32, status: i8 },
    /// `0x1F`
    SetExperience { bar: f32, level: i32, total: i32 },
    /// `0x21`
    ChunkData { column: ChunkColumn, full: bool },
    /// `0x26`
    MapChunkBulk {
        sky_light: bool,
        columns: Vec<ChunkColumn>,
    },
    /// `0x2B`
    ChangeGameState { reason: u8, value: f32 },
    /// `0x2F`
    SetSlot {
        window: i8,
        slot: i16,
        item: Option<ItemStack>,
    },
    /// `0x30`
    WindowItems {
        window: u8,
        items: Vec<Option<ItemStack>>,
    },
    /// `0x38`
    PlayerListItem(PlayerListAction),
    /// `0x39`
    PlayerAbilities {
        flags: i8,
        flying_speed: f32,
        walking_speed: f32,
    },
    /// `0x3F`
    PluginMessage { channel: String, data: Vec<u8> },
    /// `0x40` with a JSON chat component reason.
    Disconnect(String),
    /// `0x41`
    ServerDifficulty { difficulty: u8 },
    /// `0x44` action 3 (initialize)
    WorldBorderInit {
        x: f64,
        z: f64,
        old_diameter: f64,
        new_diameter: f64,
        speed: i64,
        portal_teleport_boundary: i32,
        warning_time: i32,
        warning_blocks: i32,
    },
}

impl ClientboundPlay {
    /// The protocol-47 packet id.
    pub fn id(&self) -> i32 {
        match self {
            Self::KeepAlive { .. } => 0x00,
            Self::JoinGame { .. } => 0x01,
            Self::Chat { .. } => 0x02,
            Self::TimeUpdate { .. } => 0x03,
            Self::SpawnPosition { .. } => 0x05,
            Self::UpdateHealth { .. } => 0x06,
            Self::PositionLook { .. } => 0x08,
            Self::HeldItemChange { .. } => 0x09,
            Self::EntityStatus { .. } => 0x1a,
            Self::SetExperience { .. } => 0x1f,
            Self::ChunkData { .. } => 0x21,
            Self::MapChunkBulk { .. } => 0x26,
            Self::ChangeGameState { .. } => 0x2b,
            Self::SetSlot { .. } => 0x2f,
            Self::WindowItems { .. } => 0x30,
            Self::PlayerListItem(_) => 0x38,
            Self::PlayerAbilities { .. } => 0x39,
            Self::PluginMessage { .. } => 0x3f,
            Self::Disconnect(_) => 0x40,
            Self::ServerDifficulty { .. } => 0x41,
            Self::WorldBorderInit { .. } => 0x44,
        }
    }

    /// Encodes the packet in protocol-47 form.
    ///
    /// Protocol 5 clients receive translated packets (see the translation layer); this
    /// encoder ignores `version` for now.
    ///
    /// # Errors
    ///
    /// [`EncodeError::StringTooBig`] for oversized strings.
    pub fn encode(&self, _version: ProtocolVersion) -> Result<Vec<u8>, EncodeError> {
        let mut w = PacketWriter::new();
        w.varint(self.id());
        match self {
            Self::KeepAlive { id } => {
                w.varint(*id);
            }
            Self::JoinGame {
                entity_id,
                gamemode,
                dimension,
                difficulty,
                max_players,
                level_type,
                reduced_debug_info,
            } => {
                w.i32(*entity_id)
                    .u8(*gamemode)
                    .i8(*dimension)
                    .u8(*difficulty)
                    .u8(*max_players);
                w.string(level_type)?;
                w.bool(*reduced_debug_info);
            }
            Self::Chat { json, position } => {
                w.string(json)?;
                w.i8(*position);
            }
            Self::TimeUpdate {
                world_age,
                time_of_day,
            } => {
                w.i64(*world_age).i64(*time_of_day);
            }
            Self::SpawnPosition { pos } => {
                w.position(pos.0, pos.1, pos.2);
            }
            Self::UpdateHealth {
                health,
                food,
                saturation,
            } => {
                w.f32(*health).varint(*food).f32(*saturation);
            }
            Self::PositionLook {
                x,
                y,
                z,
                yaw,
                pitch,
                flags,
            } => {
                w.f64(*x).f64(*y).f64(*z).f32(*yaw).f32(*pitch).i8(*flags);
            }
            Self::HeldItemChange { slot } => {
                w.i8(*slot);
            }
            Self::EntityStatus { entity_id, status } => {
                w.i32(*entity_id).i8(*status);
            }
            Self::SetExperience { bar, level, total } => {
                w.f32(*bar).varint(*level).varint(*total);
            }
            Self::ChunkData { column, full } => {
                w.i32(column.x).i32(column.z).bool(*full).u16(column.mask);
                w.byte_array(&column.data);
            }
            Self::MapChunkBulk { sky_light, columns } => {
                w.bool(*sky_light).varint(columns.len() as i32);
                for c in columns {
                    w.i32(c.x).i32(c.z).u16(c.mask);
                }
                for c in columns {
                    w.bytes(&c.data);
                }
            }
            Self::ChangeGameState { reason, value } => {
                w.u8(*reason).f32(*value);
            }
            Self::SetSlot { window, slot, item } => {
                w.i8(*window).i16(*slot);
                ItemStack::write(&mut w, item.as_ref())?;
            }
            Self::WindowItems { window, items } => {
                w.u8(*window).i16(items.len() as i16);
                for item in items {
                    ItemStack::write(&mut w, item.as_ref())?;
                }
            }
            Self::PlayerListItem(action) => encode_player_list(&mut w, action)?,
            Self::PlayerAbilities {
                flags,
                flying_speed,
                walking_speed,
            } => {
                w.i8(*flags).f32(*flying_speed).f32(*walking_speed);
            }
            Self::PluginMessage { channel, data } => {
                w.string(channel)?;
                w.bytes(data);
            }
            Self::Disconnect(json) => {
                w.string(json)?;
            }
            Self::ServerDifficulty { difficulty } => {
                w.u8(*difficulty);
            }
            Self::WorldBorderInit {
                x,
                z,
                old_diameter,
                new_diameter,
                speed,
                portal_teleport_boundary,
                warning_time,
                warning_blocks,
            } => {
                w.varint(3)
                    .f64(*x)
                    .f64(*z)
                    .f64(*old_diameter)
                    .f64(*new_diameter)
                    .varlong(*speed)
                    .varint(*portal_teleport_boundary)
                    .varint(*warning_time)
                    .varint(*warning_blocks);
            }
        }
        Ok(w.into_bytes())
    }

    /// Decodes a clientbound play frame body (used by test clients). Packets not modeled
    /// here yield `Ok(None)` so clients can skip them.
    ///
    /// # Errors
    ///
    /// Field errors for modeled packets.
    pub fn decode(frame: &[u8], _version: ProtocolVersion) -> Result<Option<Self>, DecodeError> {
        let mut r = PacketReader::new(frame);
        let p = match r.varint()? {
            0x00 => Self::KeepAlive { id: r.varint()? },
            0x01 => Self::JoinGame {
                entity_id: r.i32()?,
                gamemode: r.u8()?,
                dimension: r.i8()?,
                difficulty: r.u8()?,
                max_players: r.u8()?,
                level_type: r.string(16)?,
                reduced_debug_info: r.bool()?,
            },
            0x02 => Self::Chat {
                json: r.string(32767)?,
                position: r.i8()?,
            },
            0x03 => Self::TimeUpdate {
                world_age: r.i64()?,
                time_of_day: r.i64()?,
            },
            0x05 => Self::SpawnPosition { pos: r.position()? },
            0x08 => Self::PositionLook {
                x: r.f64()?,
                y: r.f64()?,
                z: r.f64()?,
                yaw: r.f32()?,
                pitch: r.f32()?,
                flags: r.i8()?,
            },
            0x09 => Self::HeldItemChange { slot: r.i8()? },
            0x39 => Self::PlayerAbilities {
                flags: r.i8()?,
                flying_speed: r.f32()?,
                walking_speed: r.f32()?,
            },
            0x3f => {
                let channel = r.string(20)?;
                Self::PluginMessage {
                    channel,
                    data: r.rest().to_vec(),
                }
            }
            0x40 => Self::Disconnect(r.string(32767)?),
            0x41 => Self::ServerDifficulty {
                difficulty: r.u8()?,
            },
            _ => return Ok(None),
        };
        Ok(Some(p))
    }
}

fn encode_player_list(w: &mut PacketWriter, action: &PlayerListAction) -> Result<(), EncodeError> {
    match action {
        PlayerListAction::Add(entries) => {
            w.varint(0).varint(entries.len() as i32);
            for e in entries {
                w.uuid(e.uuid).string(&e.name)?;
                w.varint(e.properties.len() as i32);
                for (name, value, signature) in &e.properties {
                    w.string(name)?.string(value)?;
                    w.bool(signature.is_some());
                    if let Some(sig) = signature {
                        w.string(sig)?;
                    }
                }
                w.varint(e.gamemode)
                    .varint(e.ping)
                    .bool(e.display_name.is_some());
                if let Some(d) = &e.display_name {
                    w.string(d)?;
                }
            }
        }
        PlayerListAction::UpdateGamemode(entries) | PlayerListAction::UpdateLatency(entries) => {
            let code = if matches!(action, PlayerListAction::UpdateGamemode(_)) {
                1
            } else {
                2
            };
            w.varint(code).varint(entries.len() as i32);
            for (uuid, v) in entries {
                w.uuid(*uuid).varint(*v);
            }
        }
        PlayerListAction::UpdateDisplayName(entries) => {
            w.varint(3).varint(entries.len() as i32);
            for (uuid, name) in entries {
                w.uuid(*uuid).bool(name.is_some());
                if let Some(n) = name {
                    w.string(n)?;
                }
            }
        }
        PlayerListAction::Remove(uuids) => {
            w.varint(4).varint(uuids.len() as i32);
            for uuid in uuids {
                w.uuid(*uuid);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: ProtocolVersion = ProtocolVersion::V47;

    #[test]
    fn serverbound_round_trips() {
        let packets = [
            ServerboundPlay::KeepAlive { id: 99 },
            ServerboundPlay::Chat {
                message: "hello".into(),
            },
            ServerboundPlay::PositionLook {
                x: 1.5,
                y: 64.0,
                z: -3.25,
                yaw: 90.0,
                pitch: 10.0,
                on_ground: true,
            },
            ServerboundPlay::ClientSettings {
                locale: "en_US".into(),
                view_distance: 8,
                chat_mode: 0,
                chat_colors: true,
                skin_parts: 0x7f,
            },
            ServerboundPlay::PluginMessage {
                channel: "MC|Brand".into(),
                data: b"\x07vanilla".to_vec(),
            },
        ];
        for p in packets {
            let bytes = p.encode().unwrap().unwrap();
            assert_eq!(ServerboundPlay::decode(&bytes, V).unwrap(), p);
        }
    }

    #[test]
    fn serverbound_limits() {
        let mut w = PacketWriter::new();
        w.varint(0x01).string(&"a".repeat(101)).unwrap();
        assert!(ServerboundPlay::decode(w.as_bytes(), V).is_err());
        let mut w = PacketWriter::new();
        w.varint(0x17).string("x").unwrap();
        w.bytes(&vec![0; 32768]);
        assert!(ServerboundPlay::decode(w.as_bytes(), V).is_err());
        assert_eq!(
            ServerboundPlay::decode(&[0x1a], V),
            Err(DecodeError::BadPacketId(0x1a))
        );
        // Block placement with an item including NBT.
        let mut w = PacketWriter::new();
        w.varint(0x08).position(1, 2, 3).i8(1);
        w.i16(1).i8(64).i16(0).u8(0);
        w.i8(8).i8(8).i8(8);
        assert!(matches!(
            ServerboundPlay::decode(w.as_bytes(), V).unwrap(),
            ServerboundPlay::BlockPlacement { item: Some(_), .. }
        ));
    }

    #[test]
    fn clientbound_golden_bytes() {
        let join = ClientboundPlay::JoinGame {
            entity_id: 7,
            gamemode: 0,
            dimension: 0,
            difficulty: 1,
            max_players: 20,
            level_type: "default".into(),
            reduced_debug_info: false,
        };
        assert_eq!(
            join.encode(V).unwrap(),
            [
                0x01, 0, 0, 0, 7, 0, 0, 1, 20, 7, b'd', b'e', b'f', b'a', b'u', b'l', b't', 0
            ]
        );
        assert_eq!(
            ClientboundPlay::decode(&join.encode(V).unwrap(), V).unwrap(),
            Some(join)
        );
        let border = ClientboundPlay::WorldBorderInit {
            x: 0.0,
            z: 0.0,
            old_diameter: 6.0e7,
            new_diameter: 6.0e7,
            speed: 0,
            portal_teleport_boundary: 29_999_984,
            warning_time: 15,
            warning_blocks: 5,
        };
        let bytes = border.encode(V).unwrap();
        assert_eq!(&bytes[..2], &[0x44, 0x03]);
        assert_eq!(
            ClientboundPlay::Disconnect("\"x\"".into())
                .encode(V)
                .unwrap(),
            [0x40, 3, b'"', b'x', b'"']
        );
    }

    #[test]
    fn player_list_add() {
        let p = ClientboundPlay::PlayerListItem(PlayerListAction::Add(vec![PlayerListAdd {
            uuid: 1,
            name: "a".into(),
            properties: vec![("textures".into(), "v".into(), Some("s".into()))],
            gamemode: 0,
            ping: 0,
            display_name: None,
        }]));
        let bytes = p.encode(V).unwrap();
        assert_eq!(&bytes[..3], &[0x38, 0, 1]);
        assert_eq!(bytes.len(), 3 + 16 + 2 + 1 + 9 + 2 + 1 + 2 + 1 + 1 + 1);
    }
}
