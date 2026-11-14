//! Summarizing play sessions into comparable observations.
//!
//! Timing-dependent details are separated from the strict sequence:
//! - `sequence`: packet names in arrival order, without keep-alives, time updates and
//!   chunk packets (their batching depends on tick timing),
//! - `packets`: every packet of the sequence, decoded where modeled, otherwise as hex
//!   (volatile fields such as keep-alive ids and world time are dropped),
//! - `chunks`: the sorted set of chunk columns received (`x,z,mask`),
//! - `periodic`: which periodic packets were seen at all,
//! - `entities`: the final position, angles and ground flag of every entity spawned with
//!   Spawn Player, after applying all movement packets (relative moves, looks, teleports
//!   and head looks are left out of `sequence`: how movement is split into packets
//!   depends on when client packets land in server ticks, the sum does not).

use std::collections::{BTreeMap, BTreeSet};

use krol_protocol::buf::PacketReader;
use krol_protocol::packets::login::ClientboundLogin;
use krol_protocol::packets::play::ClientboundPlay;
use krol_testbot::PlayObservation;
use serde_json::{Value, json};

/// Vanilla's names for protocol-47 clientbound play packet ids.
fn name(id: i32) -> String {
    let known = match id {
        0x00 => "keep_alive",
        0x01 => "join_game",
        0x02 => "chat",
        0x03 => "time_update",
        0x04 => "entity_equipment",
        0x05 => "spawn_position",
        0x06 => "update_health",
        0x07 => "respawn",
        0x08 => "position_look",
        0x09 => "held_item_change",
        0x0b => "animation",
        0x0c => "spawn_player",
        0x0e => "spawn_object",
        0x0f => "spawn_mob",
        0x12 => "entity_velocity",
        0x13 => "destroy_entities",
        0x15 => "entity_rel_move",
        0x16 => "entity_look",
        0x17 => "entity_look_move",
        0x18 => "entity_teleport",
        0x19 => "entity_head_look",
        0x1a => "entity_status",
        0x1c => "entity_metadata",
        0x1d => "entity_effect",
        0x1f => "set_experience",
        0x20 => "entity_properties",
        0x21 => "chunk_data",
        0x22 => "multi_block_change",
        0x23 => "block_change",
        0x26 => "map_chunk_bulk",
        0x29 => "sound_effect",
        0x2b => "change_game_state",
        0x2f => "set_slot",
        0x30 => "window_items",
        0x35 => "update_tile_entity",
        0x37 => "statistics",
        0x38 => "player_list_item",
        0x39 => "player_abilities",
        0x3b => "scoreboard_objective",
        0x3e => "teams",
        0x3f => "plugin_message",
        0x40 => "disconnect",
        0x41 => "server_difficulty",
        0x44 => "world_border",
        0x47 => "player_list_header_footer",
        _ => "",
    };
    if known.is_empty() {
        format!("0x{id:02x}")
    } else {
        known.to_owned()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn decoded_json(p: &ClientboundPlay) -> Value {
    match p {
        ClientboundPlay::JoinGame {
            gamemode,
            dimension,
            difficulty,
            max_players,
            level_type,
            reduced_debug_info,
            ..
        } => json!({
            "gamemode": gamemode, "dimension": dimension, "difficulty": difficulty,
            "max_players": max_players, "level_type": level_type,
            "reduced_debug_info": reduced_debug_info,
        }),
        ClientboundPlay::Chat { json, position } => json!({ "json": json, "position": position }),
        ClientboundPlay::SpawnPosition { pos } => json!({ "pos": [pos.0, pos.1, pos.2] }),
        ClientboundPlay::PositionLook {
            x,
            y,
            z,
            yaw,
            pitch,
            flags,
        } => json!({ "x": x, "y": y, "z": z, "yaw": yaw, "pitch": pitch, "flags": flags }),
        ClientboundPlay::HeldItemChange { slot } => json!({ "slot": slot }),
        ClientboundPlay::PlayerAbilities {
            flags,
            flying_speed,
            walking_speed,
        } => {
            json!({ "flags": flags, "flying_speed": flying_speed, "walking_speed": walking_speed })
        }
        ClientboundPlay::PluginMessage { channel, data } => {
            json!({ "channel": channel, "data": hex(data) })
        }
        ClientboundPlay::Disconnect(reason) => json!({ "reason": reason }),
        ClientboundPlay::ServerDifficulty { difficulty } => json!({ "difficulty": difficulty }),
        // Vanilla sends attributes in `HashSet` order of identity hash codes, which
        // changes from run to run.
        ClientboundPlay::EntityProperties {
            entity_id,
            properties,
        } => {
            let mut sorted = properties.clone();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            json!({ "entity_id": entity_id, "properties": sorted })
        }
        // Volatile values (ids, world time) are not compared.
        ClientboundPlay::KeepAlive { .. } | ClientboundPlay::TimeUpdate { .. } => json!({}),
        other => json!({ "debug": format!("{other:?}") }),
    }
}

/// Chunk columns contained in a Chunk Data or Map Chunk Bulk body (after the id).
fn chunk_columns(id: i32, body: &[u8]) -> Vec<String> {
    let mut r = PacketReader::new(body);
    let mut out = Vec::new();
    let mut read = || -> Option<()> {
        match id {
            0x21 => {
                let x = r.i32().ok()?;
                let z = r.i32().ok()?;
                let full = r.bool().ok()?;
                let mask = r.u16().ok()?;
                out.push(format!(
                    "{x},{z},{mask:04x}{}",
                    if full { "" } else { ",partial" }
                ));
            }
            0x26 => {
                let _sky = r.bool().ok()?;
                let n = r.varint().ok()?;
                for _ in 0..n {
                    let x = r.i32().ok()?;
                    let z = r.i32().ok()?;
                    let mask = r.u16().ok()?;
                    out.push(format!("{x},{z},{mask:04x}"));
                }
            }
            _ => {}
        }
        Some(())
    };
    let _ = read();
    out
}

fn login_json(p: &ClientboundLogin) -> Value {
    match p {
        ClientboundLogin::Disconnect(r) => json!({ "type": "disconnect", "reason": r }),
        ClientboundLogin::SetCompression(t) => json!({ "type": "set_compression", "threshold": t }),
        ClientboundLogin::Success { uuid, username } => {
            json!({ "type": "success", "uuid": uuid, "username": username })
        }
        ClientboundLogin::EncryptionRequest { .. } => json!({ "type": "encryption_request" }),
    }
}

/// An entity's state as the client sees it (1/32 blocks, 1/256 turns).
#[derive(Default)]
struct EntityState {
    pos: [i64; 3],
    yaw: i8,
    pitch: i8,
    head: i8,
    on_ground: Option<bool>,
}

fn spawn_player_state(body: &[u8]) -> Option<(i32, EntityState)> {
    let mut r = PacketReader::new(body);
    let id = r.varint().ok()?;
    r.uuid().ok()?;
    let pos = [
        i64::from(r.i32().ok()?),
        i64::from(r.i32().ok()?),
        i64::from(r.i32().ok()?),
    ];
    let yaw = r.i8().ok()?;
    let pitch = r.i8().ok()?;
    Some((
        id,
        EntityState {
            pos,
            yaw,
            pitch,
            head: yaw,
            on_ground: None,
        },
    ))
}

/// Applies a movement packet; returns `false` for other packets.
fn apply_movement(entities: &mut BTreeMap<i32, EntityState>, p: &ClientboundPlay) -> bool {
    let (id, delta, angles, abs, head, ground) = match *p {
        ClientboundPlay::EntityRelMove {
            entity_id,
            dx,
            dy,
            dz,
            on_ground,
        } => (
            entity_id,
            Some([dx, dy, dz]),
            None,
            None,
            None,
            Some(on_ground),
        ),
        ClientboundPlay::EntityLook {
            entity_id,
            yaw,
            pitch,
            on_ground,
        } => (
            entity_id,
            None,
            Some((yaw, pitch)),
            None,
            None,
            Some(on_ground),
        ),
        ClientboundPlay::EntityLookMove {
            entity_id,
            dx,
            dy,
            dz,
            yaw,
            pitch,
            on_ground,
        } => (
            entity_id,
            Some([dx, dy, dz]),
            Some((yaw, pitch)),
            None,
            None,
            Some(on_ground),
        ),
        ClientboundPlay::EntityTeleport {
            entity_id,
            x,
            y,
            z,
            yaw,
            pitch,
            on_ground,
        } => (
            entity_id,
            None,
            Some((yaw, pitch)),
            Some([x, y, z]),
            None,
            Some(on_ground),
        ),
        ClientboundPlay::EntityHeadLook { entity_id, yaw } => {
            (entity_id, None, None, None, Some(yaw), None)
        }
        _ => return false,
    };
    let e = entities.entry(id).or_default();
    if let Some(d) = delta {
        for (p, d) in e.pos.iter_mut().zip(d) {
            *p += i64::from(d);
        }
    }
    if let Some(a) = abs {
        e.pos = a.map(i64::from);
    }
    if let Some((yaw, pitch)) = angles {
        e.yaw = yaw;
        e.pitch = pitch;
    }
    if let Some(h) = head {
        e.head = h;
    }
    if ground.is_some() {
        e.on_ground = ground;
    }
    true
}

/// Turns a play session into a comparable observation.
pub fn summarize(o: &PlayObservation) -> Value {
    let mut entities: BTreeMap<i32, EntityState> = BTreeMap::new();
    let mut sequence = Vec::new();
    let mut packets: Vec<Value> = Vec::new();
    let mut chunks = BTreeSet::new();
    let mut periodic: BTreeMap<String, bool> = BTreeMap::new();
    let mut chat = Vec::new();

    for p in &o.packets {
        let n = name(p.id);
        match p.id {
            0x00 | 0x03 => {
                periodic.insert(n.clone(), true);
                continue;
            }
            0x21 | 0x26 => {
                chunks.extend(chunk_columns(p.id, &p.body));
                continue;
            }
            0x15..=0x19 => {
                if let Some(d) = &p.decoded {
                    apply_movement(&mut entities, d);
                }
                continue;
            }
            0x0c => {
                if let Some((id, state)) = spawn_player_state(&p.body) {
                    entities.insert(id, state);
                }
                sequence.push(Value::String(n.clone()));
            }
            _ => sequence.push(Value::String(n.clone())),
        }
        if let Some(ClientboundPlay::Chat { json, position }) = &p.decoded {
            chat.push(json!({ "json": json, "position": position }));
        }
        let mut entry = match &p.decoded {
            Some(d) => decoded_json(d),
            None => json!({ "hex": hex(&p.body) }),
        };
        if let Value::Object(m) = &mut entry {
            m.insert("packet".into(), Value::String(n));
        }
        packets.push(entry);
    }
    json!({
        "login": o.login.iter().map(login_json).collect::<Vec<_>>(),
        "sequence": sequence,
        "packets": packets,
        "chat": chat,
        "chunks": chunks.into_iter().collect::<Vec<_>>(),
        "periodic": periodic,
        "entities": entities
            .iter()
            .map(|(id, e)| {
                (
                    id.to_string(),
                    json!({
                        "pos": e.pos, "yaw": e.yaw, "pitch": e.pitch, "head": e.head,
                        "on_ground": e.on_ground,
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>(),
        "closed_by_server": o.closed_by_server,
    })
}
