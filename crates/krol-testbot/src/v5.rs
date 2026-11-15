//! The test client's side of protocol 5 (1.7.6–1.7.10).
//!
//! The bot thinks in protocol 47: what it sends is encoded in 1.7 form here, and what
//! it receives is mapped back onto protocol-47 packet ids, decoded where the 1.7 form
//! carries the same information. This lets play sessions of 1.7 clients be compared
//! with recordings of 1.8 clients (see `cargo xtask parity run --protocol 5`).

use std::io::Write;

use flate2::Compression;
use flate2::write::GzEncoder;
use krol_protocol::EncodeError;
use krol_protocol::buf::{PacketReader, PacketWriter};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack, ServerboundPlay};

use crate::BotError;

/// The eye height 1.7 positions include.
const EYE_HEIGHT: f64 = 1.620_000_004_768_371_6;

fn write_item(w: &mut PacketWriter, item: Option<&ItemStack>) -> Result<(), EncodeError> {
    let Some(s) = item else {
        w.i16(-1);
        return Ok(());
    };
    w.i16(s.id).i8(s.count).i16(s.damage);
    match &s.nbt {
        None => {
            w.i16(-1);
        }
        Some(nbt) => {
            let mut raw = Vec::new();
            krol_nbt::write(&mut raw, "", nbt).map_err(|_| EncodeError::StringTooBig(0))?;
            let mut gz = GzEncoder::new(Vec::new(), Compression::default());
            let _ = gz.write_all(&raw);
            w.short_byte_array(&gz.finish().unwrap_or_default());
        }
    }
    Ok(())
}

/// Encodes a serverbound packet the way a 1.7 client sends it. `own_entity` is the
/// client's entity id (1.7 sends it in Animation and Entity Action).
///
/// # Errors
///
/// [`EncodeError`] for oversized strings; packets the bot never sends give `None`.
pub fn encode(p: &ServerboundPlay, own_entity: i32) -> Result<Option<Vec<u8>>, EncodeError> {
    let mut w = PacketWriter::new();
    match p {
        ServerboundPlay::KeepAlive { id } => {
            w.varint(0x00).i32(*id);
        }
        ServerboundPlay::Chat { message } => {
            w.varint(0x01).string(message)?;
        }
        ServerboundPlay::Player { on_ground } => {
            w.varint(0x03).bool(*on_ground);
        }
        ServerboundPlay::Position { x, y, z, on_ground } => {
            w.varint(0x04)
                .f64(*x)
                .f64(*y)
                .f64(y + EYE_HEIGHT)
                .f64(*z)
                .bool(*on_ground);
        }
        ServerboundPlay::Look {
            yaw,
            pitch,
            on_ground,
        } => {
            w.varint(0x05).f32(*yaw).f32(*pitch).bool(*on_ground);
        }
        ServerboundPlay::PositionLook {
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
                .f64(y + EYE_HEIGHT)
                .f64(*z)
                .f32(*yaw)
                .f32(*pitch)
                .bool(*on_ground);
        }
        ServerboundPlay::Digging { status, pos, face } => {
            w.varint(0x07)
                .i8(*status)
                .i32(pos.0)
                .u8(pos.1 as u8)
                .i32(pos.2)
                .i8(*face);
        }
        ServerboundPlay::BlockPlacement {
            pos,
            face,
            item,
            cursor,
        } => {
            w.varint(0x08)
                .i32(pos.0)
                .u8(pos.1 as u8)
                .i32(pos.2)
                .i8(*face);
            write_item(&mut w, item.as_ref())?;
            w.u8(cursor.0 as u8).u8(cursor.1 as u8).u8(cursor.2 as u8);
        }
        ServerboundPlay::HeldItemChange { slot } => {
            w.varint(0x09).i16(*slot);
        }
        ServerboundPlay::Animation => {
            w.varint(0x0a).i32(own_entity).i8(1);
        }
        ServerboundPlay::CreativeInventoryAction { slot, item } => {
            w.varint(0x10).i16(*slot);
            write_item(&mut w, item.as_ref())?;
        }
        ServerboundPlay::ClientSettings {
            locale,
            view_distance,
            chat_mode,
            chat_colors,
            ..
        } => {
            w.varint(0x15).string(locale)?;
            w.i8(*view_distance)
                .i8(*chat_mode as i8)
                .bool(*chat_colors)
                .u8(2)
                .bool(true);
        }
        ServerboundPlay::ClickWindow {
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
            write_item(&mut w, item.as_ref())?;
        }
        ServerboundPlay::ConfirmTransaction {
            window,
            action,
            accepted,
        } => {
            w.varint(0x0f).i8(*window).i16(*action).bool(*accepted);
        }
        ServerboundPlay::CloseWindow { window } => {
            w.varint(0x0d).i8(*window);
        }
        ServerboundPlay::UpdateSign { pos, lines } => {
            w.varint(0x12).i32(pos.0).i16(pos.1 as i16).i32(pos.2);
            for line in lines {
                // 1.8 lines are JSON strings; 1.7 sends the text.
                let text = line
                    .strip_prefix('"')
                    .and_then(|l| l.strip_suffix('"'))
                    .unwrap_or(line);
                w.string(text)?;
            }
        }
        ServerboundPlay::ClientStatus { action } => {
            w.varint(0x16).i8(*action as i8);
        }
        ServerboundPlay::PluginMessage { channel, data } => {
            w.varint(0x17).string(channel)?;
            w.short_byte_array(data);
        }
        _ => return Ok(None),
    }
    Ok(Some(w.into_bytes()))
}

/// A received 1.7 packet mapped onto protocol 47: the packet id, a body in which the
/// fields the summary reads (chunk headers, spawn positions) use the 47 layout, and the
/// decoded packet where the 1.7 form has the same information.
#[derive(Debug)]
pub struct Received {
    /// Protocol-47 packet id (1.7 and 1.8 share the play ids).
    pub id: i32,
    /// Body after the id.
    pub body: Vec<u8>,
    /// Decoded form.
    pub decoded: Option<ClientboundPlay>,
}

/// Maps a received 1.7 packet frame onto protocol 47.
///
/// # Errors
///
/// [`BotError`] for malformed packets.
pub fn decode(frame: &[u8]) -> Result<Received, BotError> {
    let mut r = PacketReader::new(frame);
    let id = r.varint()?;
    let raw = r.rest().to_vec();
    let mut body = raw.clone();
    let decoded = match id {
        0x00 => Some(ClientboundPlay::KeepAlive { id: r.i32()? }),
        0x01 => Some(ClientboundPlay::JoinGame {
            entity_id: r.i32()?,
            gamemode: r.u8()?,
            dimension: r.i8()?,
            difficulty: r.u8()?,
            max_players: r.u8()?,
            level_type: r.string(16)?,
            reduced_debug_info: false,
        }),
        0x02 => Some(ClientboundPlay::Chat {
            json: r.string(32767)?,
            position: 0,
        }),
        0x03 => Some(ClientboundPlay::TimeUpdate {
            world_age: r.i64()?,
            time_of_day: r.i64()?,
        }),
        0x05 => Some(ClientboundPlay::SpawnPosition {
            pos: (r.i32()?, r.i32()?, r.i32()?),
        }),
        0x08 => Some(ClientboundPlay::PositionLook {
            x: r.f64()?,
            y: r.f64()? - EYE_HEIGHT,
            z: r.f64()?,
            yaw: r.f32()?,
            pitch: r.f32()?,
            flags: {
                r.bool()?;
                0
            },
        }),
        0x09 => Some(ClientboundPlay::HeldItemChange { slot: r.i8()? }),
        0x0c => {
            let entity_id = r.varint()?;
            let uuid_text = r.string(36)?;
            let _name = r.string(16)?;
            for _ in 0..r.varint()? {
                r.string(32767)?;
                r.string(32767)?;
                r.string(32767)?;
            }
            let uuid = u128::from_str_radix(&uuid_text.replace('-', ""), 16).unwrap_or(0);
            let mut w = PacketWriter::new();
            w.varint(entity_id)
                .uuid(uuid)
                .i32(r.i32()?)
                .i32(r.i32()?)
                .i32(r.i32()?)
                .i8(r.i8()?)
                .i8(r.i8()?);
            body = w.into_bytes();
            None
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
            Some(ClientboundPlay::SpawnObject {
                entity_id,
                kind,
                x,
                y,
                z,
                pitch,
                yaw,
                data,
                velocity,
            })
        }
        0x0d => Some(ClientboundPlay::CollectItem {
            collected: r.i32()?,
            collector: r.i32()?,
        }),
        0x12 => Some(ClientboundPlay::EntityVelocity {
            entity_id: r.i32()?,
            x: r.i16()?,
            y: r.i16()?,
            z: r.i16()?,
        }),
        0x33 => {
            // 1.7 lines are plain text; shown as the JSON strings a 1.8 client gets.
            let pos = (r.i32()?, i32::from(r.i16()?), r.i32()?);
            let mut line = || -> Result<String, BotError> {
                let text = r.string(15)?;
                Ok(format!(
                    "\"{}\"",
                    text.replace('\\', "\\\\").replace('"', "\\\"")
                ))
            };
            Some(ClientboundPlay::UpdateSign {
                pos,
                lines: [line()?, line()?, line()?, line()?],
            })
        }
        0x35 => Some(ClientboundPlay::UpdateTileEntity {
            pos: (r.i32()?, i32::from(r.i16()?), r.i32()?),
            action: r.u8()?,
            nbt: {
                let len = r.i16()?;
                if len < 0 {
                    None
                } else {
                    let bytes = r.bytes(len as usize)?;
                    krol_nbt::read_gzip(bytes).ok().map(|(_, c)| c)
                }
            },
        }),
        0x36 => Some(ClientboundPlay::SignEditorOpen {
            pos: (r.i32()?, r.i32()?, r.i32()?),
        }),
        0x11 => Some(ClientboundPlay::SpawnExperienceOrb {
            entity_id: r.varint()?,
            x: r.i32()?,
            y: r.i32()?,
            z: r.i32()?,
            count: r.i16()?,
        }),
        0x31 => Some(ClientboundPlay::WindowProperty {
            window: r.u8()?,
            property: r.i16()?,
            value: r.i16()?,
        }),
        0x24 => Some(ClientboundPlay::BlockAction {
            pos: (r.i32()?, i32::from(r.i16()?), r.i32()?),
            action: r.u8()?,
            param: r.u8()?,
            block: r.varint()?,
        }),
        // Protocol 5 sends explosions exactly like 47.
        0x27 => {
            let pos = (r.f32()?, r.f32()?, r.f32()?);
            let radius = r.f32()?;
            let count = r.i32()?;
            let mut records = Vec::new();
            for _ in 0..count.max(0) {
                records.push((r.i8()?, r.i8()?, r.i8()?));
            }
            let motion = (r.f32()?, r.f32()?, r.f32()?);
            Some(ClientboundPlay::Explosion {
                pos,
                radius,
                records,
                motion,
            })
        }
        0x25 => Some(ClientboundPlay::BlockBreakAnimation {
            entity_id: r.varint()?,
            pos: (r.i32()?, r.i32()?, r.i32()?),
            stage: r.i8()?,
        }),
        0x15 => Some(ClientboundPlay::EntityRelMove {
            entity_id: r.i32()?,
            dx: r.i8()?,
            dy: r.i8()?,
            dz: r.i8()?,
            on_ground: false,
        }),
        0x16 => Some(ClientboundPlay::EntityLook {
            entity_id: r.i32()?,
            yaw: r.i8()?,
            pitch: r.i8()?,
            on_ground: false,
        }),
        0x17 => Some(ClientboundPlay::EntityLookMove {
            entity_id: r.i32()?,
            dx: r.i8()?,
            dy: r.i8()?,
            dz: r.i8()?,
            yaw: r.i8()?,
            pitch: r.i8()?,
            on_ground: false,
        }),
        0x18 => Some(ClientboundPlay::EntityTeleport {
            entity_id: r.i32()?,
            x: r.i32()?,
            y: r.i32()?,
            z: r.i32()?,
            yaw: r.i8()?,
            pitch: r.i8()?,
            on_ground: false,
        }),
        0x1d => Some(ClientboundPlay::EntityEffect {
            entity_id: r.i32()?,
            effect: r.i8()?,
            amplifier: r.i8()?,
            duration: i32::from(r.i16()?),
            show_particles: true,
        }),
        0x1e => Some(ClientboundPlay::RemoveEntityEffect {
            entity_id: r.i32()?,
            effect: r.i8()?,
        }),
        0x19 => Some(ClientboundPlay::EntityHeadLook {
            entity_id: r.i32()?,
            yaw: r.i8()?,
        }),
        0x21 => {
            let (x, z, full, mask) = (r.i32()?, r.i32()?, r.bool()?, r.u16()?);
            let mut w = PacketWriter::new();
            w.i32(x).i32(z).bool(full).u16(mask);
            body = w.into_bytes();
            None
        }
        0x26 => {
            let n = r.i16()?;
            let len = r.i32()?;
            let sky = r.bool()?;
            r.bytes(len.max(0) as usize)?;
            let mut w = PacketWriter::new();
            w.bool(sky).varint(i32::from(n));
            for _ in 0..n {
                let (x, z, mask) = (r.i32()?, r.i32()?, r.u16()?);
                r.u16()?;
                w.i32(x).i32(z).u16(mask);
            }
            body = w.into_bytes();
            None
        }
        0x39 => Some(ClientboundPlay::PlayerAbilities {
            flags: r.i8()?,
            flying_speed: r.f32()?,
            walking_speed: r.f32()?,
        }),
        0x3f => {
            let channel = r.string(20)?;
            let data = r.short_byte_array(32767)?.to_vec();
            Some(ClientboundPlay::PluginMessage { channel, data })
        }
        0x40 => Some(ClientboundPlay::Disconnect(r.string(32767)?)),
        _ => None,
    };
    Ok(Received { id, body, decoded })
}
