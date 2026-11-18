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
            Self::UseEntity {
                target,
                action,
                target_pos,
            } => {
                w.varint(0x02).varint(*target).varint(*action);
                if let (2, Some((x, y, z))) = (action, target_pos) {
                    w.f32(*x).f32(*y).f32(*z);
                }
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
            Self::Digging { status, pos, face } => {
                w.varint(0x07)
                    .i8(*status)
                    .position(pos.0, pos.1, pos.2)
                    .i8(*face);
            }
            Self::BlockPlacement {
                pos,
                face,
                item,
                cursor,
            } => {
                w.varint(0x08).position(pos.0, pos.1, pos.2).i8(*face);
                ItemStack::write(&mut w, item.as_ref())?;
                w.i8(cursor.0).i8(cursor.1).i8(cursor.2);
            }
            Self::CreativeInventoryAction { slot, item } => {
                w.varint(0x10).i16(*slot);
                ItemStack::write(&mut w, item.as_ref())?;
            }
            Self::HeldItemChange { slot } => {
                w.varint(0x09).i16(*slot);
            }
            Self::Animation => {
                w.varint(0x0a);
            }
            Self::EntityAction {
                entity,
                action,
                jump_boost,
            } => {
                w.varint(0x0b)
                    .varint(*entity)
                    .varint(*action)
                    .varint(*jump_boost);
            }
            Self::SteerVehicle {
                sideways,
                forward,
                flags,
            } => {
                w.varint(0x0c).f32(*sideways).f32(*forward).u8(*flags);
            }
            Self::PluginMessage { channel, data } => {
                w.varint(0x17).string(channel)?;
                w.bytes(data);
            }
            Self::ClickWindow {
                window,
                slot,
                button,
                action,
                mode,
                item,
            } => {
                w.varint(0x0e)
                    .i8(*window)
                    .i16(*slot)
                    .i8(*button)
                    .i16(*action)
                    .i8(*mode);
                ItemStack::write(&mut w, item.as_ref())?;
            }
            Self::ConfirmTransaction {
                window,
                action,
                accepted,
            } => {
                w.varint(0x0f).i8(*window).i16(*action).bool(*accepted);
            }
            Self::CloseWindow { window } => {
                w.varint(0x0d).i8(*window);
            }
            Self::EnchantItem {
                window,
                enchantment,
            } => {
                w.varint(0x11).i8(*window).i8(*enchantment);
            }
            Self::UpdateSign { pos, lines } => {
                w.varint(0x12).position(pos.0, pos.1, pos.2);
                for line in lines {
                    w.string(line)?;
                }
            }
            _ => return Ok(None),
        }
        Ok(Some(w.into_bytes()))
    }
}

/// A team's properties in a Teams packet (modes 0 and 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamInfo {
    /// Display name.
    pub display: String,
    /// Prefix before members' names.
    pub prefix: String,
    /// Suffix after members' names.
    pub suffix: String,
    /// Bit 0: friendly fire, bit 1: see invisible teammates.
    pub flags: u8,
    /// Name tag visibility (`always`, `never`, `hideForOtherTeams`, `hideForOwnTeam`).
    pub nametag: String,
    /// Color index (-1 for none).
    pub color: i8,
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

/// An attribute modifier in Entity Properties: `(uuid, amount, operation)`.
pub type AttributeModifier = (u128, f64, i8);
/// An attribute in Entity Properties: `(key, base value, modifiers)`.
pub type Property = (String, f64, Vec<AttributeModifier>);

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

/// One icon on a map (vanilla `Vec4b`): its type (0 for a player), position in half
/// pixels from the center, and rotation in 16 steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapIcon {
    /// The icon type (0-15).
    pub kind: u8,
    /// Position from the center, in half pixels.
    pub x: i8,
    /// Position from the center, in half pixels.
    pub z: i8,
    /// Rotation in 16 steps (0-15).
    pub rotation: u8,
}

impl MapIcon {
    /// The type in the high nibble, the rotation in the low one.
    pub fn packed(&self) -> u8 {
        (self.kind & 15) << 4 | (self.rotation & 15)
    }

    /// The icon from its packed byte and position.
    pub fn unpack(packed: u8, x: i8, z: i8) -> Self {
        Self {
            kind: packed >> 4,
            x,
            z,
            rotation: packed & 15,
        }
    }
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
    /// `0x1D`: a status effect on an entity (duration in ticks). Vanilla 1.8 writes
    /// whether particles show into the byte documented as "hide particles".
    EntityEffect {
        entity_id: i32,
        effect: i8,
        amplifier: i8,
        duration: i32,
        show_particles: bool,
    },
    /// `0x1E`
    RemoveEntityEffect { entity_id: i32, effect: i8 },
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
        warning_distance: i32,
        warning_time: i32,
    },
    /// `0x37` (statistic name, value) pairs
    Statistics(Vec<(String, i32)>),
    /// `0x23`
    BlockChange { pos: BlockPos, state: i32 },
    /// `0x22` changes inside one chunk: (local x, y, local z, state)
    MultiBlockChange {
        chunk_x: i32,
        chunk_z: i32,
        records: Vec<(u8, u8, u8, i32)>,
    },
    /// `0x29` (position in 1/8 blocks, pitch as `pitch * 63`)
    SoundEffect {
        name: String,
        x: i32,
        y: i32,
        z: i32,
        volume: f32,
        pitch: u8,
    },
    /// `0x28`
    Effect {
        id: i32,
        pos: BlockPos,
        data: i32,
        disable_relative_volume: bool,
    },
    /// `0x1C`
    EntityMetadata {
        entity_id: i32,
        metadata: Vec<Metadata>,
    },
    /// `0x20`: attributes as `(key, base value, modifiers)`, each modifier
    /// `(uuid, amount, operation)`.
    EntityProperties {
        entity_id: i32,
        properties: Vec<Property>,
    },
    /// `0x04`
    EntityEquipment {
        entity_id: i32,
        slot: i16,
        item: Option<ItemStack>,
    },
    /// `0x0B`
    Animation { entity_id: i32, animation: u8 },
    /// `0x0C` (positions in 1/32 blocks, angles in 1/256 turns)
    SpawnPlayer {
        entity_id: i32,
        uuid: u128,
        x: i32,
        y: i32,
        z: i32,
        yaw: i8,
        pitch: i8,
        current_item: i16,
        metadata: Vec<Metadata>,
    },
    /// `0x13`
    DestroyEntities(Vec<i32>),
    /// `0x15`
    EntityRelMove {
        entity_id: i32,
        dx: i8,
        dy: i8,
        dz: i8,
        on_ground: bool,
    },
    /// `0x16`
    EntityLook {
        entity_id: i32,
        yaw: i8,
        pitch: i8,
        on_ground: bool,
    },
    /// `0x17`
    EntityLookMove {
        entity_id: i32,
        dx: i8,
        dy: i8,
        dz: i8,
        yaw: i8,
        pitch: i8,
        on_ground: bool,
    },
    /// `0x18`
    EntityTeleport {
        entity_id: i32,
        x: i32,
        y: i32,
        z: i32,
        yaw: i8,
        pitch: i8,
        on_ground: bool,
    },
    /// `0x19`
    EntityHeadLook { entity_id: i32, yaw: i8 },
    /// `0x0E` (velocity present when `data` is not 0)
    SpawnObject {
        entity_id: i32,
        kind: i8,
        x: i32,
        y: i32,
        z: i32,
        pitch: i8,
        yaw: i8,
        data: i32,
        velocity: Option<(i16, i16, i16)>,
    },
    /// `0x0D`
    CollectItem { collected: i32, collector: i32 },
    /// `0x1B` (`vehicle_id` -1 to get off; `leash` for leads)
    AttachEntity {
        entity_id: i32,
        vehicle_id: i32,
        leash: bool,
    },
    /// `0x2D` (title: chat JSON)
    OpenWindow {
        window: u8,
        kind: String,
        title: String,
        slots: u8,
    },
    /// `0x2E`
    CloseWindow { window: u8 },
    /// `0x32`
    ConfirmTransaction {
        window: i8,
        action: i16,
        accepted: bool,
    },
    /// `0x07`
    Respawn {
        dimension: i32,
        difficulty: u8,
        gamemode: u8,
        level_type: String,
    },
    /// `0x11`: an experience orb appears (position in 1/32 blocks).
    SpawnExperienceOrb {
        entity_id: i32,
        x: i32,
        y: i32,
        z: i32,
        count: i16,
    },
    /// `0x10`: a painting's title, hanging position and facing (horizontal index:
    /// south, west, north, east).
    SpawnPainting {
        entity_id: i32,
        title: String,
        pos: BlockPos,
        facing: u8,
    },
    /// `0x33`: a sign's four lines (JSON chat).
    UpdateSign { pos: BlockPos, lines: [String; 4] },
    /// `0x34`: a map's scale, icons, and the colors of a rectangle (`columns` 0: none).
    Maps {
        /// The map's item damage.
        map: i32,
        scale: i8,
        /// Icons as (type, x, z, rotation).
        icons: Vec<MapIcon>,
        columns: u8,
        rows: u8,
        x: u8,
        z: u8,
        /// The rectangle's colors, row by row.
        data: Vec<u8>,
    },
    /// `0x3B`: creates (mode 0), removes (1) or updates (2) an objective; the display
    /// name and render type (`integer` or `hearts`) are sent for modes 0 and 2.
    ScoreboardObjective {
        name: String,
        mode: u8,
        display: String,
        render: String,
    },
    /// `0x3C`: sets a score, or removes one (`remove`; an empty objective removes the
    /// name from every objective).
    UpdateScore {
        name: String,
        remove: bool,
        objective: String,
        value: i32,
    },
    /// `0x42`: a combat event for the player's own client: entering combat (0), ending
    /// it (1, with its length in ticks and the main attacker's entity id or -1) or dying
    /// (2, with the player's entity id, the killer's and the death message).
    CombatEvent {
        event: u8,
        duration: i32,
        entity_id: i32,
        killer: i32,
        message: String,
    },
    /// `0x3D`: shows an objective in a display slot (an empty name clears it).
    DisplayScoreboard { position: u8, name: String },
    /// `0x3E`: creates (mode 0), removes (1) or updates (2) a team, or adds (3) or removes
    /// (4) members.
    Teams {
        name: String,
        mode: u8,
        info: Option<TeamInfo>,
        players: Vec<String>,
    },
    /// `0x35`: tile entity data (spawners, command blocks, beacons, skulls, flower pots,
    /// banners).
    UpdateTileEntity {
        pos: BlockPos,
        action: u8,
        nbt: Option<krol_nbt::Compound>,
    },
    /// `0x36`: opens the sign editor.
    SignEditorOpen { pos: BlockPos },
    /// `0x31`: a window property (furnace progress, enchantment levels, ...).
    WindowProperty {
        window: u8,
        property: i16,
        value: i16,
    },
    /// `0x24`: a block event (chest lid, note block, piston).
    BlockAction {
        pos: BlockPos,
        action: u8,
        param: u8,
        block: i32,
    },
    /// `0x0A`: a player lies down in the bed at `pos`.
    UseBed { entity_id: i32, pos: BlockPos },
    /// `0x25`
    BlockBreakAnimation {
        entity_id: i32,
        pos: BlockPos,
        stage: i8,
    },
    /// `0x27`: an explosion at `pos` with `radius`; `records` are the destroyed blocks
    /// relative to the truncated (`(int)`) position, `motion` the knockback for the
    /// receiving player.
    Explosion {
        pos: (f32, f32, f32),
        radius: f32,
        records: Vec<(i8, i8, i8)>,
        motion: (f32, f32, f32),
    },
    /// `0x2A`
    Particle {
        id: i32,
        long_distance: bool,
        pos: (f32, f32, f32),
        offset: (f32, f32, f32),
        speed: f32,
        count: i32,
        args: Vec<i32>,
    },
    /// `0x12` (1/8000 blocks per tick)
    EntityVelocity {
        entity_id: i32,
        x: i16,
        y: i16,
        z: i16,
    },
}

/// One entity metadata entry (protocol 47 types).
#[derive(Debug, Clone, PartialEq)]
pub enum MetadataValue {
    /// Type 0.
    Byte(i8),
    /// Type 1.
    Short(i16),
    /// Type 2.
    Int(i32),
    /// Type 3.
    Float(f32),
    /// Type 4.
    String(String),
    /// Type 5.
    Item(Option<ItemStack>),
    /// Type 6.
    Position(i32, i32, i32),
    /// Type 7.
    Rotation(f32, f32, f32),
}

/// An entity metadata entry: index (0–31) and value.
#[derive(Debug, Clone, PartialEq)]
pub struct Metadata {
    /// Metadata index.
    pub index: u8,
    /// Value.
    pub value: MetadataValue,
}

fn write_metadata(w: &mut PacketWriter, entries: &[Metadata]) -> Result<(), EncodeError> {
    for m in entries {
        let ty = match m.value {
            MetadataValue::Byte(_) => 0,
            MetadataValue::Short(_) => 1,
            MetadataValue::Int(_) => 2,
            MetadataValue::Float(_) => 3,
            MetadataValue::String(_) => 4,
            MetadataValue::Item(_) => 5,
            MetadataValue::Position(..) => 6,
            MetadataValue::Rotation(..) => 7,
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
            MetadataValue::Item(v) => ItemStack::write(w, v.as_ref())?,
            MetadataValue::Position(x, y, z) => {
                w.i32(*x).i32(*y).i32(*z);
            }
            MetadataValue::Rotation(x, y, z) => {
                w.f32(*x).f32(*y).f32(*z);
            }
        }
    }
    w.u8(0x7f);
    Ok(())
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
            Self::EntityEffect { .. } => 0x1d,
            Self::RemoveEntityEffect { .. } => 0x1e,
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
            Self::Statistics(_) => 0x37,
            Self::BlockChange { .. } => 0x23,
            Self::MultiBlockChange { .. } => 0x22,
            Self::SoundEffect { .. } => 0x29,
            Self::Effect { .. } => 0x28,
            Self::EntityMetadata { .. } => 0x1c,
            Self::EntityProperties { .. } => 0x20,
            Self::EntityEquipment { .. } => 0x04,
            Self::Animation { .. } => 0x0b,
            Self::SpawnPlayer { .. } => 0x0c,
            Self::DestroyEntities(_) => 0x13,
            Self::EntityRelMove { .. } => 0x15,
            Self::EntityLook { .. } => 0x16,
            Self::EntityLookMove { .. } => 0x17,
            Self::EntityTeleport { .. } => 0x18,
            Self::EntityHeadLook { .. } => 0x19,
            Self::EntityVelocity { .. } => 0x12,
            Self::Particle { .. } => 0x2a,
            Self::BlockAction { .. } => 0x24,
            Self::WindowProperty { .. } => 0x31,
            Self::SpawnExperienceOrb { .. } => 0x11,
            Self::SpawnPainting { .. } => 0x10,
            Self::UpdateSign { .. } => 0x33,
            Self::Maps { .. } => 0x34,
            Self::ScoreboardObjective { .. } => 0x3b,
            Self::CombatEvent { .. } => 0x42,
            Self::UpdateScore { .. } => 0x3c,
            Self::DisplayScoreboard { .. } => 0x3d,
            Self::Teams { .. } => 0x3e,
            Self::UpdateTileEntity { .. } => 0x35,
            Self::SignEditorOpen { .. } => 0x36,
            Self::BlockBreakAnimation { .. } => 0x25,
            Self::UseBed { .. } => 0x0a,
            Self::Explosion { .. } => 0x27,
            Self::SpawnObject { .. } => 0x0e,
            Self::CollectItem { .. } => 0x0d,
            Self::AttachEntity { .. } => 0x1b,
            Self::Respawn { .. } => 0x07,
            Self::ConfirmTransaction { .. } => 0x32,
            Self::OpenWindow { .. } => 0x2d,
            Self::CloseWindow { .. } => 0x2e,
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
            Self::EntityEffect {
                entity_id,
                effect,
                amplifier,
                duration,
                show_particles,
            } => {
                w.varint(*entity_id)
                    .i8(*effect)
                    .i8(*amplifier)
                    .varint(*duration)
                    .bool(*show_particles);
            }
            Self::RemoveEntityEffect { entity_id, effect } => {
                w.varint(*entity_id).i8(*effect);
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
                warning_distance,
                warning_time,
            } => {
                w.varint(3)
                    .f64(*x)
                    .f64(*z)
                    .f64(*old_diameter)
                    .f64(*new_diameter)
                    .varlong(*speed)
                    .varint(*portal_teleport_boundary)
                    .varint(*warning_distance)
                    .varint(*warning_time);
            }
            Self::BlockChange { pos, state } => {
                w.position(pos.0, pos.1, pos.2).varint(*state);
            }
            Self::MultiBlockChange {
                chunk_x,
                chunk_z,
                records,
            } => {
                w.i32(*chunk_x).i32(*chunk_z).varint(records.len() as i32);
                for (x, y, z, state) in records {
                    w.u8((x << 4) | (z & 15)).u8(*y).varint(*state);
                }
            }
            Self::SoundEffect {
                name,
                x,
                y,
                z,
                volume,
                pitch,
            } => {
                w.string(name)?;
                w.i32(*x).i32(*y).i32(*z).f32(*volume).u8(*pitch);
            }
            Self::Effect {
                id,
                pos,
                data,
                disable_relative_volume,
            } => {
                w.i32(*id)
                    .position(pos.0, pos.1, pos.2)
                    .i32(*data)
                    .bool(*disable_relative_volume);
            }
            Self::Statistics(stats) => {
                w.varint(stats.len() as i32);
                for (name, value) in stats {
                    w.string(name)?.varint(*value);
                }
            }
            Self::EntityMetadata {
                entity_id,
                metadata,
            } => {
                w.varint(*entity_id);
                write_metadata(&mut w, metadata)?;
            }
            Self::EntityProperties {
                entity_id,
                properties,
            } => {
                w.varint(*entity_id).i32(properties.len() as i32);
                for (key, value, modifiers) in properties {
                    w.string(key)?.f64(*value).varint(modifiers.len() as i32);
                    for (uuid, amount, operation) in modifiers {
                        w.uuid(*uuid).f64(*amount).i8(*operation);
                    }
                }
            }
            Self::EntityEquipment {
                entity_id,
                slot,
                item,
            } => {
                w.varint(*entity_id).i16(*slot);
                ItemStack::write(&mut w, item.as_ref())?;
            }
            Self::Animation {
                entity_id,
                animation,
            } => {
                w.varint(*entity_id).u8(*animation);
            }
            Self::SpawnPlayer {
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
                w.varint(*entity_id)
                    .uuid(*uuid)
                    .i32(*x)
                    .i32(*y)
                    .i32(*z)
                    .i8(*yaw)
                    .i8(*pitch)
                    .i16(*current_item);
                write_metadata(&mut w, metadata)?;
            }
            Self::DestroyEntities(ids) => {
                w.varint(ids.len() as i32);
                for id in ids {
                    w.varint(*id);
                }
            }
            Self::EntityRelMove {
                entity_id,
                dx,
                dy,
                dz,
                on_ground,
            } => {
                w.varint(*entity_id)
                    .i8(*dx)
                    .i8(*dy)
                    .i8(*dz)
                    .bool(*on_ground);
            }
            Self::EntityLook {
                entity_id,
                yaw,
                pitch,
                on_ground,
            } => {
                w.varint(*entity_id).i8(*yaw).i8(*pitch).bool(*on_ground);
            }
            Self::EntityLookMove {
                entity_id,
                dx,
                dy,
                dz,
                yaw,
                pitch,
                on_ground,
            } => {
                w.varint(*entity_id)
                    .i8(*dx)
                    .i8(*dy)
                    .i8(*dz)
                    .i8(*yaw)
                    .i8(*pitch)
                    .bool(*on_ground);
            }
            Self::EntityTeleport {
                entity_id,
                x,
                y,
                z,
                yaw,
                pitch,
                on_ground,
            } => {
                w.varint(*entity_id)
                    .i32(*x)
                    .i32(*y)
                    .i32(*z)
                    .i8(*yaw)
                    .i8(*pitch)
                    .bool(*on_ground);
            }
            Self::EntityHeadLook { entity_id, yaw } => {
                w.varint(*entity_id).i8(*yaw);
            }
            Self::EntityVelocity { entity_id, x, y, z } => {
                w.varint(*entity_id).i16(*x).i16(*y).i16(*z);
            }
            Self::SpawnObject {
                entity_id,
                kind,
                x,
                y,
                z,
                pitch,
                yaw,
                data,
                velocity,
            } => {
                w.varint(*entity_id)
                    .i8(*kind)
                    .i32(*x)
                    .i32(*y)
                    .i32(*z)
                    .i8(*pitch)
                    .i8(*yaw)
                    .i32(*data);
                if *data != 0 {
                    let v = velocity.unwrap_or_default();
                    w.i16(v.0).i16(v.1).i16(v.2);
                }
            }
            Self::CollectItem {
                collected,
                collector,
            } => {
                w.varint(*collected).varint(*collector);
            }
            Self::AttachEntity {
                entity_id,
                vehicle_id,
                leash,
            } => {
                w.i32(*entity_id).i32(*vehicle_id).u8(u8::from(*leash));
            }
            Self::Respawn {
                dimension,
                difficulty,
                gamemode,
                level_type,
            } => {
                w.i32(*dimension).u8(*difficulty).u8(*gamemode);
                w.string(level_type)?;
            }
            Self::ConfirmTransaction {
                window,
                action,
                accepted,
            } => {
                w.i8(*window).i16(*action).bool(*accepted);
            }
            Self::OpenWindow {
                window,
                kind,
                title,
                slots,
            } => {
                w.u8(*window);
                w.string(kind)?;
                w.string(title)?;
                w.u8(*slots);
            }
            Self::CloseWindow { window } => {
                w.u8(*window);
            }
            Self::WindowProperty {
                window,
                property,
                value,
            } => {
                w.u8(*window).i16(*property).i16(*value);
            }
            Self::SpawnExperienceOrb {
                entity_id,
                x,
                y,
                z,
                count,
            } => {
                w.varint(*entity_id).i32(*x).i32(*y).i32(*z).i16(*count);
            }
            Self::SpawnPainting {
                entity_id,
                title,
                pos,
                facing,
            } => {
                w.varint(*entity_id)
                    .string(title)?
                    .position(pos.0, pos.1, pos.2)
                    .u8(*facing);
            }
            Self::UpdateSign { pos, lines } => {
                w.position(pos.0, pos.1, pos.2);
                for line in lines {
                    w.string(line)?;
                }
            }
            Self::Maps {
                map,
                scale,
                icons,
                columns,
                rows,
                x,
                z,
                data,
            } => {
                w.varint(*map).i8(*scale).varint(icons.len() as i32);
                for icon in icons {
                    w.u8(icon.packed()).i8(icon.x).i8(icon.z);
                }
                w.u8(*columns);
                if *columns > 0 {
                    w.u8(*rows).u8(*x).u8(*z).byte_array(data);
                }
            }
            Self::ScoreboardObjective {
                name,
                mode,
                display,
                render,
            } => {
                w.string(name)?.u8(*mode);
                if *mode == 0 || *mode == 2 {
                    w.string(display)?.string(render)?;
                }
            }
            Self::UpdateScore {
                name,
                remove,
                objective,
                value,
            } => {
                w.string(name)?
                    .varint(i32::from(*remove))
                    .string(objective)?;
                if !remove {
                    w.varint(*value);
                }
            }
            Self::DisplayScoreboard { position, name } => {
                w.u8(*position).string(name)?;
            }
            Self::CombatEvent {
                event,
                duration,
                entity_id,
                killer,
                message,
            } => {
                w.varint(i32::from(*event));
                match event {
                    1 => {
                        w.varint(*duration).i32(*killer);
                    }
                    2 => {
                        w.varint(*entity_id).i32(*killer).string(message)?;
                    }
                    _ => {}
                }
            }
            Self::Teams {
                name,
                mode,
                info,
                players,
            } => {
                w.string(name)?.u8(*mode);
                if let Some(t) = info.as_ref().filter(|_| *mode == 0 || *mode == 2) {
                    w.string(&t.display)?
                        .string(&t.prefix)?
                        .string(&t.suffix)?
                        .u8(t.flags)
                        .string(&t.nametag)?
                        .i8(t.color);
                }
                if matches!(mode, 0 | 3 | 4) {
                    w.varint(players.len() as i32);
                    for p in players {
                        w.string(p)?;
                    }
                }
            }
            Self::UpdateTileEntity { pos, action, nbt } => {
                w.position(pos.0, pos.1, pos.2)
                    .u8(*action)
                    .nbt(nbt.as_ref())?;
            }
            Self::SignEditorOpen { pos } => {
                w.position(pos.0, pos.1, pos.2);
            }
            Self::BlockAction {
                pos,
                action,
                param,
                block,
            } => {
                w.position(pos.0, pos.1, pos.2)
                    .u8(*action)
                    .u8(*param)
                    .varint(*block);
            }
            Self::BlockBreakAnimation {
                entity_id,
                pos,
                stage,
            } => {
                w.varint(*entity_id)
                    .position(pos.0, pos.1, pos.2)
                    .i8(*stage);
            }
            Self::Explosion {
                pos,
                radius,
                records,
                motion,
            } => write_explosion(&mut w, *pos, *radius, records, *motion),
            Self::UseBed { entity_id, pos } => {
                w.varint(*entity_id).position(pos.0, pos.1, pos.2);
            }
            Self::Particle {
                id,
                long_distance,
                pos,
                offset,
                speed,
                count,
                args,
            } => {
                w.i32(*id)
                    .bool(*long_distance)
                    .f32(pos.0)
                    .f32(pos.1)
                    .f32(pos.2)
                    .f32(offset.0)
                    .f32(offset.1)
                    .f32(offset.2)
                    .f32(*speed)
                    .i32(*count);
                for a in args {
                    w.varint(*a);
                }
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
            0x13 => {
                let n = r.varint()?;
                let mut ids = Vec::new();
                for _ in 0..n.clamp(0, 1 << 16) {
                    ids.push(r.varint()?);
                }
                Self::DestroyEntities(ids)
            }
            0x15 => Self::EntityRelMove {
                entity_id: r.varint()?,
                dx: r.i8()?,
                dy: r.i8()?,
                dz: r.i8()?,
                on_ground: r.bool()?,
            },
            0x16 => Self::EntityLook {
                entity_id: r.varint()?,
                yaw: r.i8()?,
                pitch: r.i8()?,
                on_ground: r.bool()?,
            },
            0x17 => Self::EntityLookMove {
                entity_id: r.varint()?,
                dx: r.i8()?,
                dy: r.i8()?,
                dz: r.i8()?,
                yaw: r.i8()?,
                pitch: r.i8()?,
                on_ground: r.bool()?,
            },
            0x18 => Self::EntityTeleport {
                entity_id: r.varint()?,
                x: r.i32()?,
                y: r.i32()?,
                z: r.i32()?,
                yaw: r.i8()?,
                pitch: r.i8()?,
                on_ground: r.bool()?,
            },
            0x20 => {
                let entity_id = r.varint()?;
                let n = r.i32()?;
                let mut properties = Vec::new();
                for _ in 0..n.clamp(0, 1024) {
                    let key = r.string(64)?;
                    let value = r.f64()?;
                    let mut modifiers = Vec::new();
                    for _ in 0..r.varint()?.clamp(0, 1024) {
                        modifiers.push((r.uuid()?, r.f64()?, r.i8()?));
                    }
                    properties.push((key, value, modifiers));
                }
                Self::EntityProperties {
                    entity_id,
                    properties,
                }
            }
            0x0e => {
                let entity_id = r.varint()?;
                let kind = r.i8()?;
                let (x, y, z) = (r.i32()?, r.i32()?, r.i32()?);
                let (pitch, yaw) = (r.i8()?, r.i8()?);
                let data = r.i32()?;
                let velocity = if data != 0 {
                    Some((r.i16()?, r.i16()?, r.i16()?))
                } else {
                    None
                };
                Self::SpawnObject {
                    entity_id,
                    kind,
                    x,
                    y,
                    z,
                    pitch,
                    yaw,
                    data,
                    velocity,
                }
            }
            0x11 => Self::SpawnExperienceOrb {
                entity_id: r.varint()?,
                x: r.i32()?,
                y: r.i32()?,
                z: r.i32()?,
                count: r.i16()?,
            },
            // The longest title is "SkullAndRoses".
            0x10 => Self::SpawnPainting {
                entity_id: r.varint()?,
                title: r.string(13)?,
                pos: r.position()?,
                facing: r.u8()?,
            },
            0x33 => Self::UpdateSign {
                pos: r.position()?,
                lines: [
                    r.string(32767)?,
                    r.string(32767)?,
                    r.string(32767)?,
                    r.string(32767)?,
                ],
            },
            0x34 => {
                let map = r.varint()?;
                let scale = r.i8()?;
                let n = r.varint()?.max(0) as usize;
                let mut icons = Vec::new();
                for _ in 0..n {
                    let packed = r.u8()?;
                    icons.push(MapIcon::unpack(packed, r.i8()?, r.i8()?));
                }
                let columns = r.u8()?;
                let (rows, x, z, data) = if columns > 0 {
                    (r.u8()?, r.u8()?, r.u8()?, r.byte_array(1 << 21)?.to_vec())
                } else {
                    (0, 0, 0, Vec::new())
                };
                Self::Maps {
                    map,
                    scale,
                    icons,
                    columns,
                    rows,
                    x,
                    z,
                    data,
                }
            }
            0x35 => Self::UpdateTileEntity {
                pos: r.position()?,
                action: r.u8()?,
                nbt: r.nbt()?,
            },
            0x36 => Self::SignEditorOpen { pos: r.position()? },
            0x3b => {
                let name = r.string(16)?;
                let mode = r.u8()?;
                let (display, render) = if mode == 0 || mode == 2 {
                    (r.string(32)?, r.string(16)?)
                } else {
                    (String::new(), String::new())
                };
                Self::ScoreboardObjective {
                    name,
                    mode,
                    display,
                    render,
                }
            }
            0x3c => {
                let name = r.string(40)?;
                let remove = r.varint()? == 1;
                let objective = r.string(16)?;
                let value = if remove { 0 } else { r.varint()? };
                Self::UpdateScore {
                    name,
                    remove,
                    objective,
                    value,
                }
            }
            0x42 => {
                let event = r.varint()? as u8;
                let (mut duration, mut entity_id, mut killer, mut message) =
                    (0, 0, 0, String::new());
                match event {
                    1 => {
                        duration = r.varint()?;
                        killer = r.i32()?;
                    }
                    2 => {
                        entity_id = r.varint()?;
                        killer = r.i32()?;
                        message = r.string(32767)?;
                    }
                    _ => {}
                }
                Self::CombatEvent {
                    event,
                    duration,
                    entity_id,
                    killer,
                    message,
                }
            }
            0x3d => Self::DisplayScoreboard {
                position: r.u8()?,
                name: r.string(16)?,
            },
            0x3e => {
                let name = r.string(16)?;
                let mode = r.u8()?;
                let info = if mode == 0 || mode == 2 {
                    Some(TeamInfo {
                        display: r.string(32)?,
                        prefix: r.string(16)?,
                        suffix: r.string(16)?,
                        flags: r.u8()?,
                        nametag: r.string(32)?,
                        color: r.i8()?,
                    })
                } else {
                    None
                };
                let mut players = Vec::new();
                if matches!(mode, 0 | 3 | 4) {
                    let n = r.varint()?;
                    for _ in 0..n.max(0) {
                        players.push(r.string(40)?);
                    }
                }
                Self::Teams {
                    name,
                    mode,
                    info,
                    players,
                }
            }
            0x31 => Self::WindowProperty {
                window: r.u8()?,
                property: r.i16()?,
                value: r.i16()?,
            },
            0x24 => Self::BlockAction {
                pos: r.position()?,
                action: r.u8()?,
                param: r.u8()?,
                block: r.varint()?,
            },
            0x25 => Self::BlockBreakAnimation {
                entity_id: r.varint()?,
                pos: r.position()?,
                stage: r.i8()?,
            },
            0x27 => read_explosion(&mut r)?,
            0x0a => Self::UseBed {
                entity_id: r.varint()?,
                pos: r.position()?,
            },
            0x0d => Self::CollectItem {
                collected: r.varint()?,
                collector: r.varint()?,
            },
            0x1b => Self::AttachEntity {
                entity_id: r.i32()?,
                vehicle_id: r.i32()?,
                leash: r.u8()? != 0,
            },
            0x12 => Self::EntityVelocity {
                entity_id: r.varint()?,
                x: r.i16()?,
                y: r.i16()?,
                z: r.i16()?,
            },
            0x1a => Self::EntityStatus {
                entity_id: r.i32()?,
                status: r.i8()?,
            },
            0x1d => Self::EntityEffect {
                entity_id: r.varint()?,
                effect: r.i8()?,
                amplifier: r.i8()?,
                duration: r.varint()?,
                show_particles: r.bool()?,
            },
            0x1e => Self::RemoveEntityEffect {
                entity_id: r.varint()?,
                effect: r.i8()?,
            },
            0x29 => Self::SoundEffect {
                name: r.string(256)?,
                x: r.i32()?,
                y: r.i32()?,
                z: r.i32()?,
                volume: r.f32()?,
                pitch: r.u8()?,
            },
            0x19 => Self::EntityHeadLook {
                entity_id: r.varint()?,
                yaw: r.i8()?,
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

/// Body of `S27PacketExplosion` (the same in protocols 47 and 5).
pub(crate) fn write_explosion(
    w: &mut PacketWriter,
    pos: (f32, f32, f32),
    radius: f32,
    records: &[(i8, i8, i8)],
    motion: (f32, f32, f32),
) {
    w.f32(pos.0).f32(pos.1).f32(pos.2).f32(radius);
    w.i32(records.len() as i32);
    for r in records {
        w.i8(r.0).i8(r.1).i8(r.2);
    }
    w.f32(motion.0).f32(motion.1).f32(motion.2);
}

/// Reads the body of `S27PacketExplosion`.
fn read_explosion(r: &mut PacketReader<'_>) -> Result<ClientboundPlay, DecodeError> {
    let pos = (r.f32()?, r.f32()?, r.f32()?);
    let radius = r.f32()?;
    let count = r.i32()?;
    let mut records = Vec::new();
    for _ in 0..count.max(0) {
        records.push((r.i8()?, r.i8()?, r.i8()?));
    }
    let motion = (r.f32()?, r.f32()?, r.f32()?);
    Ok(ClientboundPlay::Explosion {
        pos,
        radius,
        records,
        motion,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: ProtocolVersion = ProtocolVersion::V47;

    #[test]
    fn spawn_painting_bytes() {
        let p = ClientboundPlay::SpawnPainting {
            entity_id: 5,
            title: "Kebab".into(),
            pos: (1, 64, -1),
            facing: 2,
        };
        let bytes = p.encode(V).unwrap();
        // Id, entity id, title, position (x << 38 | y << 26 | z & 0x3ffffff), facing.
        let mut want = vec![0x10, 0x05, 0x05];
        want.extend_from_slice(b"Kebab");
        want.extend_from_slice(&(1u64 << 38 | 64 << 26 | 0x3ff_ffff).to_be_bytes());
        want.push(2);
        assert_eq!(bytes, want);
        assert_eq!(ClientboundPlay::decode(&bytes, V).unwrap(), Some(p));
    }

    #[test]
    fn riding_golden_bytes() {
        let attach = ClientboundPlay::AttachEntity {
            entity_id: 1,
            vehicle_id: -1,
            leash: false,
        };
        let bytes = attach.encode(V).unwrap();
        assert_eq!(bytes, [0x1b, 0, 0, 0, 1, 0xff, 0xff, 0xff, 0xff, 0]);
        assert_eq!(ClientboundPlay::decode(&bytes, V).unwrap(), Some(attach));
        let steer = ServerboundPlay::SteerVehicle {
            sideways: 0.0,
            forward: 1.0,
            flags: 2,
        };
        assert_eq!(
            steer.encode().unwrap().unwrap(),
            [0x0c, 0, 0, 0, 0, 0x3f, 0x80, 0, 0, 2]
        );
    }

    #[test]
    fn entity_action_golden_bytes() {
        let p = ServerboundPlay::EntityAction {
            entity: 300,
            action: 3,
            jump_boost: 0,
        };
        assert_eq!(p.encode().unwrap().unwrap(), [0x0b, 0xac, 0x02, 0x03, 0x00]);
    }

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
            ServerboundPlay::EnchantItem {
                window: 3,
                enchantment: 2,
            },
            ServerboundPlay::EntityAction {
                entity: 7,
                action: 3,
                jump_boost: 0,
            },
        ];
        for p in packets {
            let bytes = p.encode().unwrap().unwrap();
            assert_eq!(ServerboundPlay::decode(&bytes, V).unwrap(), p);
        }
    }

    #[test]
    fn scoreboard_bytes() {
        let cases = [
            (
                ClientboundPlay::ScoreboardObjective {
                    name: "k".into(),
                    mode: 0,
                    display: "K".into(),
                    render: "integer".into(),
                },
                "3b016b00014b07696e7465676572",
            ),
            (
                ClientboundPlay::ScoreboardObjective {
                    name: "k".into(),
                    mode: 1,
                    display: String::new(),
                    render: String::new(),
                },
                "3b016b01",
            ),
            (
                ClientboundPlay::UpdateScore {
                    name: "a".into(),
                    remove: false,
                    objective: "k".into(),
                    value: 300,
                },
                "3c016100016bac02",
            ),
            (
                ClientboundPlay::UpdateScore {
                    name: "a".into(),
                    remove: true,
                    objective: String::new(),
                    value: 0,
                },
                "3c01610100",
            ),
            (
                ClientboundPlay::CombatEvent {
                    event: 1,
                    duration: 50,
                    entity_id: 0,
                    killer: -1,
                    message: String::new(),
                },
                "4201 32ffffffff",
            ),
            (
                ClientboundPlay::DisplayScoreboard {
                    position: 1,
                    name: "k".into(),
                },
                "3d01016b",
            ),
            (
                ClientboundPlay::Teams {
                    name: "t".into(),
                    mode: 0,
                    info: Some(TeamInfo {
                        display: "T".into(),
                        prefix: String::new(),
                        suffix: String::new(),
                        flags: 3,
                        nametag: "always".into(),
                        color: -1,
                    }),
                    players: vec!["a".into()],
                },
                "3e017400015400000306616c77617973ff010161",
            ),
            (
                ClientboundPlay::Teams {
                    name: "t".into(),
                    mode: 4,
                    info: None,
                    players: vec!["a".into()],
                },
                "3e0174040101 61",
            ),
        ];
        for (p, expected) in cases {
            let bytes = p.encode(V).unwrap();
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, expected.replace(' ', ""));
            assert_eq!(ClientboundPlay::decode(&bytes, V).unwrap(), Some(p));
        }
    }

    #[test]
    fn enchant_item_bytes() {
        let p = ServerboundPlay::EnchantItem {
            window: 3,
            enchantment: 2,
        };
        assert_eq!(p.encode().unwrap().unwrap(), [0x11, 0x03, 0x02]);
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
    fn effect_packets_match_vanilla() {
        // From parity/golden/play/effects.json (vanilla 1.8.8): regeneration II for 5 s
        // with particles shown, then its removal.
        let effect = ClientboundPlay::EntityEffect {
            entity_id: 0,
            effect: 10,
            amplifier: 1,
            duration: 100,
            show_particles: true,
        };
        let bytes = effect.encode(V).unwrap();
        assert_eq!(bytes, [0x1d, 0, 10, 1, 100, 1]);
        assert_eq!(ClientboundPlay::decode(&bytes, V).unwrap(), Some(effect));
        let remove = ClientboundPlay::RemoveEntityEffect {
            entity_id: 0,
            effect: 10,
        };
        let bytes = remove.encode(V).unwrap();
        assert_eq!(bytes, [0x1e, 0, 10]);
        assert_eq!(ClientboundPlay::decode(&bytes, V).unwrap(), Some(remove));
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
            warning_distance: 5,
            warning_time: 15,
        };
        // Bytes recorded from vanilla 1.8.8 (parity/golden/play/join_flat.json).
        assert_eq!(
            hex(&border.encode(V).unwrap()[1..]),
            "0300000000000000000000000000000000418c9c3800000000418c9c380000000000f086a70e050f"
        );
        assert_eq!(
            ClientboundPlay::Disconnect("\"x\"".into())
                .encode(V)
                .unwrap(),
            [0x40, 3, b'"', b'x', b'"']
        );
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// Player metadata and properties exactly as vanilla 1.8.8 sent them on join.
    #[test]
    fn vanilla_join_entity_packets() {
        use MetadataValue::*;
        let md = |index, value| Metadata { index, value };
        let meta = ClientboundPlay::EntityMetadata {
            entity_id: 0,
            metadata: vec![
                md(0, Byte(0)),
                md(1, Short(300)),
                md(2, String(std::string::String::new())),
                md(3, Byte(0)),
                md(4, Byte(0)),
                md(6, Float(20.0)),
                md(7, Int(0)),
                md(8, Byte(0)),
                md(9, Byte(0)),
                md(10, Byte(0x7f)),
                md(16, Byte(0)),
                md(17, Float(0.0)),
                md(18, Int(0)),
            ],
        };
        assert_eq!(
            hex(&meta.encode(V).unwrap()[1..]),
            "00000021012c8200030004006641a000004700000000080009000a7f1000710000000052000000007f"
        );
        let props = ClientboundPlay::EntityProperties {
            entity_id: 0,
            properties: vec![("generic.movementSpeed".into(), f64::from(0.1f32), vec![])],
        };
        assert_eq!(
            hex(&props.encode(V).unwrap()[1..]),
            "00000000011567656e657269632e6d6f76656d656e7453706565643fb99999a000000000"
        );
        assert_eq!(
            hex(&ClientboundPlay::Statistics(vec![]).encode(V).unwrap()[1..]),
            "00"
        );
        let items = ClientboundPlay::WindowItems {
            window: 0,
            items: vec![None; 45],
        };
        assert_eq!(
            hex(&items.encode(V).unwrap()[1..]),
            format!("00002d{}", "ffff".repeat(45))
        );
        let health = ClientboundPlay::UpdateHealth {
            health: 20.0,
            food: 20,
            saturation: 5.0,
        };
        assert_eq!(hex(&health.encode(V).unwrap()[1..]), "41a000001440a00000");
    }

    /// Bytes recorded from vanilla 1.8.8 (parity/golden/play/build_creative.json).
    #[test]
    fn vanilla_build_packets() {
        let sound = ClientboundPlay::SoundEffect {
            name: "dig.stone".into(),
            x: -3748,
            y: 28,
            z: -3476,
            volume: 1.0,
            pitch: 50,
        };
        assert_eq!(
            hex(&sound.encode(V).unwrap()[1..]),
            "096469672e73746f6e65fffff15c0000001cfffff26c3f80000032"
        );
        let change = ClientboundPlay::BlockChange {
            pos: (-469, 2, -435),
            state: 48,
        };
        assert_eq!(hex(&change.encode(V).unwrap()[1..]), "ffff8ac00bfffe4d30");
    }

    /// Bytes recorded from vanilla 1.8.8 (parity/golden/play/two_players.json).
    #[test]
    fn vanilla_entity_packets() {
        use MetadataValue::*;
        let md = |index, value| Metadata { index, value };
        let spawn = ClientboundPlay::SpawnPlayer {
            entity_id: 1,
            uuid: 0xa762f560_4fce_3236_812a_b80efff0b62b,
            x: -15088,
            y: 128,
            z: -13904,
            yaw: 0,
            pitch: 0,
            current_item: 0,
            metadata: vec![
                md(0, Byte(0)),
                md(1, Short(300)),
                md(2, String(std::string::String::new())),
                md(3, Byte(0)),
                md(4, Byte(0)),
                md(6, Float(20.0)),
                md(7, Int(0)),
                md(8, Byte(0)),
                md(9, Byte(0)),
                md(10, Byte(0)),
                md(16, Byte(0)),
                md(17, Float(0.0)),
                md(18, Int(0)),
            ],
        };
        assert_eq!(
            hex(&spawn.encode(V).unwrap()[1..]),
            "01a762f5604fce3236812ab80efff0b62bffffc51000000080ffffc9b000000000000021012c8200030004006641a000004700000000080009000a001000710000000052000000007f"
        );
        let cases: [(ClientboundPlay, &str); 6] = [
            (
                ClientboundPlay::EntityTeleport {
                    entity_id: 1,
                    x: -15080,
                    y: 128,
                    z: -13904,
                    yaw: 0,
                    pitch: 0,
                    on_ground: true,
                },
                "01ffffc51800000080ffffc9b0000001",
            ),
            (
                ClientboundPlay::EntityRelMove {
                    entity_id: 1,
                    dx: 8,
                    dy: 0,
                    dz: 0,
                    on_ground: true,
                },
                "0108000001",
            ),
            (
                ClientboundPlay::EntityLook {
                    entity_id: 1,
                    yaw: 64,
                    pitch: 21,
                    on_ground: true,
                },
                "01401501",
            ),
            (
                ClientboundPlay::EntityHeadLook {
                    entity_id: 1,
                    yaw: 64,
                },
                "0140",
            ),
            (
                ClientboundPlay::Animation {
                    entity_id: 1,
                    animation: 0,
                },
                "0100",
            ),
            (ClientboundPlay::DestroyEntities(vec![1]), "0101"),
        ];
        for (packet, expected) in cases {
            let bytes = packet.encode(V).unwrap();
            assert_eq!(hex(&bytes[1..]), expected);
            if let Some(decoded) = ClientboundPlay::decode(&bytes, V).unwrap() {
                assert_eq!(decoded, packet);
            }
        }
    }

    #[test]
    fn use_bed_round_trip() {
        let p = ClientboundPlay::UseBed {
            entity_id: 5,
            pos: (1, 64, -1),
        };
        let bytes = p.encode(V).unwrap();
        // Position: x << 38 | (y & 0xfff) << 26 | (z & 0x3ffffff).
        assert_eq!(bytes, [0x0a, 5, 0, 0, 0, 0x41, 0x03, 0xff, 0xff, 0xff]);
        assert_eq!(ClientboundPlay::decode(&bytes, V).unwrap(), Some(p));
    }

    #[test]
    fn explosion_round_trip() {
        let p = ClientboundPlay::Explosion {
            pos: (1.5, 2.0, -3.0),
            radius: 4.0,
            records: vec![(1, -1, 0)],
            motion: (0.0, 0.5, 0.0),
        };
        let bytes = p.encode(V).unwrap();
        assert_eq!(
            bytes,
            [
                0x27, 0x3f, 0xc0, 0, 0, 0x40, 0, 0, 0, 0xc0, 0x40, 0, 0, 0x40, 0x80, 0, 0, 0, 0, 0,
                1, 1, 0xff, 0, 0, 0, 0, 0, 0x3f, 0, 0, 0, 0, 0, 0, 0
            ]
        );
        assert_eq!(ClientboundPlay::decode(&bytes, V).unwrap(), Some(p));
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

    #[test]
    fn maps_round_trip() {
        let p = ClientboundPlay::Maps {
            map: 3,
            scale: 0,
            icons: vec![MapIcon {
                kind: 0,
                x: -2,
                z: 5,
                rotation: 8,
            }],
            columns: 2,
            rows: 1,
            x: 10,
            z: 20,
            data: vec![4, 5],
        };
        let bytes = p.encode(V).unwrap();
        assert_eq!(bytes, [0x34, 3, 0, 1, 0x08, 0xfe, 5, 2, 1, 10, 20, 2, 4, 5]);
        assert_eq!(ClientboundPlay::decode(&bytes, V).unwrap(), Some(p));
        // Icons only: no rectangle follows.
        let p = ClientboundPlay::Maps {
            map: 0,
            scale: 1,
            icons: Vec::new(),
            columns: 0,
            rows: 0,
            x: 0,
            z: 0,
            data: Vec::new(),
        };
        let bytes = p.encode(V).unwrap();
        assert_eq!(bytes, [0x34, 0, 1, 0, 0]);
        assert_eq!(ClientboundPlay::decode(&bytes, V).unwrap(), Some(p));
    }
}
