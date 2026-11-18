//! Play-state sessions: join, behave like an idle vanilla client, record everything.

use std::net::SocketAddr;
use std::time::Duration;

use krol_protocol::ProtocolVersion;
use krol_protocol::packets::login::{ClientboundLogin, ServerboundLogin};
use krol_protocol::packets::play::{ClientboundPlay, ServerboundPlay};
use tokio::time::Instant;

use crate::{BotError, Conn};

/// A packet received during a play session.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservedPacket {
    /// Milliseconds since Login Success.
    pub at_ms: u64,
    /// Packet id.
    pub id: i32,
    /// Raw packet body (after the id).
    pub body: Vec<u8>,
    /// Decoded form, if modeled by `krol-protocol`.
    pub decoded: Option<ClientboundPlay>,
}

/// Everything observed in a play session.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayObservation {
    /// Login packets (Set Compression, Login Success or a disconnect).
    pub login: Vec<ClientboundLogin>,
    /// Play packets in arrival order.
    pub packets: Vec<ObservedPacket>,
    /// Whether the server closed the connection before the session ended.
    pub closed_by_server: bool,
}

/// Joins with `protocol` and `name`, then for `duration` behaves like an idle vanilla
/// client and records every packet:
///
/// - after Join Game: sends Client Settings (`en_US`, view distance 10) and the
///   `MC|Brand` plugin message `vanilla`,
/// - answers Keep Alive with the same id,
/// - answers Player Position And Look with a matching serverbound position,
/// - then sends a Player (on ground) packet every 50 ms, like a client tick.
///
/// Optional `chat` lines are sent once the position is known, 500 ms apart; a line
/// written `@<ms> <text>` is instead sent `<ms>` after the position was received.
/// Lines starting with `!` (after the timing) are bot actions instead of chat:
/// `!dig <x> <y> <z> <face> [status]` (default: start digging, status 0),
/// `!creative <slot> <id> <count> <damage>` (set a slot in creative mode),
/// `!place <x> <y> <z> <face> [cx cy cz]` (use the held item, cursor 8/8/8 unless
/// given in 1/16; sends the swing first),
/// `!use` (use the held item, face 255), `!hold <slot>` (select hotbar slot),
/// `!swing` (arm animation), `!attack <entity id>` (swing, then attack),
/// `!interact <entity id>` (right click on an entity), `!interactat <entity id> <x> <y>
/// <z>` (a 1.8 client's right click: interact at the hit point relative to the entity,
/// then interact),
/// `!crit <entity id>` (a hop: up 1 block and 0.25 back down off the ground, the attack
/// while falling, then landing where it started), `!sprint` / `!unsprint` (Entity
/// Action start / stop sprinting),
/// `!look <yaw> <pitch>` (Player Look, on ground), `!respawn` (Client Status 0),
/// `!click <window> <slot> <button> <action> <mode> [<id> <count> <damage>]`,
/// `!confirm <window> <action> <accepted 0|1>`, `!close <window>`, `!enchant <window>
/// <button>`, `!itemname [text]` (anvil name field), `!beacon <primary> <secondary>`
/// (beacon powers).
/// Optional
/// `moves` (offsets `[dx, dy, dz]` from the first server-given position, optionally
/// `[dx, dy, dz, on_ground]` with on_ground 0 or 1, default 1; `[]` sends nothing that
/// tick) replace the idle Player
/// packet, one Position packet per tick, after which the bot idles again.
///
/// # Errors
///
/// [`BotError`] on connection failure or undecodable data.
pub async fn play_session(
    addr: SocketAddr,
    protocol: i32,
    name: &str,
    duration: Duration,
    chat: &[String],
    moves: &[Vec<f64>],
) -> Result<PlayObservation, BotError> {
    let version = ProtocolVersion::from_number(protocol).unwrap_or(ProtocolVersion::V47);
    let mut conn = Conn::connect(addr).await?;
    conn.handshake(addr, protocol, 2).await?;
    conn.send(
        &ServerboundLogin::Start {
            name: name.to_owned(),
        }
        .encode(version)?,
    )
    .await?;
    let login = crate::login_exchange(&mut conn, version).await?;
    let mut obs = PlayObservation {
        login,
        packets: Vec::new(),
        closed_by_server: false,
    };
    if !matches!(obs.login.last(), Some(ClientboundLogin::Success { .. })) {
        obs.closed_by_server = true;
        return Ok(obs);
    }

    let start = Instant::now();
    let deadline = start + duration;
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    let mut positioned = false;
    let mut origin: Option<(f64, f64, f64)> = None;
    // Where the bot last told the server it is, and its rotation: relative teleports
    // are resolved against them, like the vanilla client does.
    let mut here = (0.0f64, 0.0f64, 0.0f64);
    let mut facing = (0.0f32, 0.0f32);
    let mut next_move = 0usize;
    let mut pending_chat: Vec<String> = chat
        .iter()
        .filter(|c| !c.starts_with('@'))
        .rev()
        .cloned()
        .collect();
    let mut timed_chat: Vec<(u64, String)> = chat
        .iter()
        .filter_map(|c| {
            let (at, text) = c.strip_prefix('@')?.split_once(' ')?;
            Some((at.parse().ok()?, text.to_owned()))
        })
        .collect();
    let mut positioned_at: Option<Instant> = None;
    let mut held: Vec<Option<krol_protocol::packets::play::ItemStack>> = vec![None; 9];
    let mut held_slot: i16 = 0;
    let mut next_chat = start;
    // For 1.7 clients: the own entity id (sent in some 1.7 packets), once known.
    let mut v5_eid: Option<i32> = (version == ProtocolVersion::V5).then_some(0);
    // The own entity id from Join Game (for Entity Action).
    let mut own_eid = 0;

    let result: Result<(), BotError> = async { loop {
        tokio::select! {
            () = tokio::time::sleep_until(deadline) => break,
            _ = tick.tick() => {
                if positioned {
                    // Scripted actions go out right before this tick's movement packet, so
                    // the server always handles both in the same tick.
                    if let Some(at) = positioned_at {
                        let elapsed = at.elapsed().as_millis() as u64;
                        let due: Vec<String> = timed_chat
                            .iter()
                            .filter(|(ms, _)| *ms <= elapsed)
                            .map(|(_, t)| t.clone())
                            .collect();
                        timed_chat.retain(|(ms, _)| *ms > elapsed);
                        for message in due {
                            for packet in action_or_chat(&message, &held, here, own_eid) {
                                if let ServerboundPlay::HeldItemChange { slot } = packet {
                                    held_slot = slot;
                                }
                                if let ServerboundPlay::Look { yaw, pitch, .. } = packet {
                                    facing = (yaw, pitch);
                                }
                                if let ServerboundPlay::CreativeInventoryAction { slot, item } = &packet {
                                    if *slot >= 36 {
                                        held[(*slot - 36) as usize] = item.clone();
                                    }
                                }
                                send_play(&mut conn, v5_eid, &packet).await?;
                            }
                            let _ = held_slot;
                        }
                    }
                    match (origin, moves.get(next_move)) {
                        // An empty move: send nothing this tick.
                        (Some(_), Some(m)) if m.is_empty() => next_move += 1,
                        (Some((ox, oy, oz)), Some(m)) => {
                            next_move += 1;
                            let at = |k: usize| m.get(k).copied().unwrap_or(0.0);
                            here = (ox + at(0), oy + at(1), oz + at(2));
                            send_play(&mut conn, v5_eid, &ServerboundPlay::Position {
                                x: here.0, y: here.1, z: here.2,
                                on_ground: m.get(3).is_none_or(|g| *g != 0.0),
                            }).await?;
                        }
                        _ => send_play(&mut conn, v5_eid, &ServerboundPlay::Player { on_ground: true }).await?,
                    }
                    if Instant::now() >= next_chat && let Some(message) = pending_chat.pop() {
                        send_play(&mut conn, v5_eid, &ServerboundPlay::Chat { message }).await?;
                        next_chat = Instant::now() + Duration::from_millis(500);
                    }
                }
            }
            packet = conn.recv_packet() => {
                let Some(body) = packet? else {
                    obs.closed_by_server = true;
                    break;
                };
                let disconnect = body.first() == Some(&0x40);
                let (id, rest, decoded) = if v5_eid.is_some() {
                    let received = crate::v5::decode(&body)?;
                    (received.id, received.body, received.decoded)
                } else {
                    let mut r = krol_protocol::buf::PacketReader::new(&body);
                    let id = r.varint()?;
                    (id, r.rest().to_vec(), ClientboundPlay::decode(&body, version)?)
                };
                match &decoded {
                    Some(ClientboundPlay::KeepAlive { id }) => {
                        send_play(&mut conn, v5_eid, &ServerboundPlay::KeepAlive { id: *id }).await?;
                    }
                    Some(ClientboundPlay::JoinGame { entity_id, .. }) => {
                        own_eid = *entity_id;
                        if v5_eid.is_some() {
                            v5_eid = Some(*entity_id);
                        }
                        send_play(&mut conn, v5_eid, &ServerboundPlay::ClientSettings {
                            locale: "en_US".into(),
                            view_distance: 10,
                            chat_mode: 0,
                            chat_colors: true,
                            skin_parts: 0x7f,
                        }).await?;
                        send_play(&mut conn, v5_eid, &ServerboundPlay::PluginMessage {
                            channel: "MC|Brand".into(),
                            data: b"\x07vanilla".to_vec(),
                        }).await?;
                    }
                    Some(ClientboundPlay::PositionLook { x, y, z, yaw, pitch, flags }) => {
                        // S08PacketPlayerPosLook flags: x, y, z, yaw, pitch relative.
                        let rel = |bit: i8, base: f64, v: f64| if flags & bit != 0 { base + v } else { v };
                        here = (rel(0x01, here.0, *x), rel(0x02, here.1, *y), rel(0x04, here.2, *z));
                        facing = (
                            rel(0x08, f64::from(facing.0), f64::from(*yaw)) as f32,
                            rel(0x10, f64::from(facing.1), f64::from(*pitch)) as f32,
                        );
                        send_play(&mut conn, v5_eid, &ServerboundPlay::PositionLook {
                            x: here.0, y: here.1, z: here.2, yaw: facing.0, pitch: facing.1,
                            on_ground: false,
                        }).await?;
                        positioned = true;
                        positioned_at.get_or_insert_with(Instant::now);
                        origin.get_or_insert(here);
                        next_chat = Instant::now() + Duration::from_millis(500);
                    }
                    _ => {}
                }
                obs.packets.push(ObservedPacket {
                    at_ms: start.elapsed().as_millis() as u64,
                    id,
                    body: rest,
                    decoded,
                });
                if disconnect {
                    obs.closed_by_server = true;
                    break;
                }
            }
        }
    }
    Ok(()) }.await;
    match result {
        Ok(()) => {}
        // The server closing the socket while the bot is sending is a normal end.
        Err(e) if is_closed(&e) => obs.closed_by_server = true,
        Err(e) => return Err(e),
    }
    Ok(obs)
}

/// Sends a packet in protocol 47, or in 1.7 form when `v5_eid` (the client's entity id
/// in a 1.7 session) is set.
async fn send_play(
    conn: &mut Conn,
    v5_eid: Option<i32>,
    packet: &ServerboundPlay,
) -> Result<(), BotError> {
    let bytes = match v5_eid {
        Some(eid) => crate::v5::encode(packet, eid)?,
        None => packet.encode()?,
    };
    if let Some(bytes) = bytes {
        conn.send(&bytes).await?;
    }
    Ok(())
}

/// Parses a scripted line into packets: a `!` action or a chat message.
fn action_or_chat(
    line: &str,
    held: &[Option<krol_protocol::packets::play::ItemStack>],
    here: (f64, f64, f64),
    own_eid: i32,
) -> Vec<ServerboundPlay> {
    let Some(action) = line.strip_prefix('!') else {
        return vec![ServerboundPlay::Chat {
            message: line.to_owned(),
        }];
    };
    let parts: Vec<&str> = action.split_whitespace().collect();
    let num = |i: usize| {
        parts
            .get(i)
            .and_then(|p| p.parse::<i32>().ok())
            .unwrap_or(0)
    };
    match parts.first().copied() {
        Some("dig") => vec![ServerboundPlay::Digging {
            status: num(5) as i8,
            pos: (num(1), num(2), num(3)),
            face: num(4) as i8,
        }],
        Some("creative") => vec![ServerboundPlay::CreativeInventoryAction {
            slot: num(1) as i16,
            item: Some(krol_protocol::packets::play::ItemStack {
                id: num(2) as i16,
                count: num(3) as i8,
                damage: num(4) as i16,
                nbt: None,
            }),
        }],
        Some("swing") => vec![ServerboundPlay::Animation],
        Some("respawn") => vec![ServerboundPlay::ClientStatus { action: 0 }],
        // !click <window> <slot> <button> <action> <mode> [<id> <count> <damage>]
        Some("click") => vec![ServerboundPlay::ClickWindow {
            window: num(1) as i8,
            slot: num(2) as i16,
            button: num(3) as i8,
            action: num(4) as i16,
            mode: num(5) as i8,
            item: (parts.len() > 8).then(|| krol_protocol::packets::play::ItemStack {
                id: num(6) as i16,
                count: num(7) as i8,
                damage: num(8) as i16,
                nbt: None,
            }),
        }],
        Some("confirm") => vec![ServerboundPlay::ConfirmTransaction {
            window: num(1) as i8,
            action: num(2) as i16,
            accepted: num(3) != 0,
        }],
        Some("close") => vec![ServerboundPlay::CloseWindow {
            window: num(1) as i8,
        }],
        // !itemname [text]: the anvil's name field (MC|ItemName with the text, possibly
        // empty).
        Some("itemname") => {
            let text = parts.get(1..).map(|p| p.join(" ")).unwrap_or_default();
            let mut w = krol_protocol::buf::PacketWriter::new();
            let _ = w.string(&text);
            vec![ServerboundPlay::PluginMessage {
                channel: "MC|ItemName".into(),
                data: w.into_bytes(),
            }]
        }
        // !beacon <primary> <secondary>: a beacon's powers (MC|Beacon, two ints).
        Some("beacon") => {
            let mut data = num(1).to_be_bytes().to_vec();
            data.extend_from_slice(&num(2).to_be_bytes());
            vec![ServerboundPlay::PluginMessage {
                channel: "MC|Beacon".into(),
                data,
            }]
        }
        // !enchant <window> <button>: picks an enchanting table offer (0-2).
        Some("enchant") => vec![ServerboundPlay::EnchantItem {
            window: num(1) as i8,
            enchantment: num(2) as i8,
        }],
        // !sign <x> <y> <z> <line1>|<line2>|<line3>|<line4> (plain text lines)
        Some("sign") => {
            let text = parts.get(4..).map(|p| p.join(" ")).unwrap_or_default();
            let mut lines = text.split('|').map(|l| {
                let escaped = l.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{escaped}\"")
            });
            let mut line = || lines.next().unwrap_or_else(|| "\"\"".into());
            vec![ServerboundPlay::UpdateSign {
                pos: (num(1), num(2), num(3)),
                lines: [line(), line(), line(), line()],
            }]
        }
        Some("look") => vec![ServerboundPlay::Look {
            yaw: parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0.0),
            pitch: parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(0.0),
            on_ground: true,
        }],
        // !interact <entity>: a right click on an entity. (A 1.8 client sends an
        // interaction at the hit point first, which only armor stands answer.)
        Some("interact") => vec![ServerboundPlay::UseEntity {
            target: num(1),
            action: 0,
            target_pos: None,
        }],
        // !interactat <entity> <x> <y> <z>: a 1.8 client's right click on an entity, hit
        // at that point relative to the entity's position.
        Some("interactat") => {
            let f = |k: usize| parts.get(k).and_then(|p| p.parse().ok()).unwrap_or(0.0);
            vec![
                ServerboundPlay::UseEntity {
                    target: num(1),
                    action: 2,
                    target_pos: Some((f(2), f(3), f(4))),
                },
                ServerboundPlay::UseEntity {
                    target: num(1),
                    action: 0,
                    target_pos: None,
                },
            ]
        }
        // The swing comes first, like a client's left click.
        Some("attack") => vec![
            ServerboundPlay::Animation,
            ServerboundPlay::UseEntity {
                target: num(1),
                action: 1,
                target_pos: None,
            },
        ],
        Some("crit") => {
            let at = |dy: f64, on_ground: bool| ServerboundPlay::Position {
                x: here.0,
                y: here.1 + dy,
                z: here.2,
                on_ground,
            };
            vec![
                at(1.0, false),
                at(0.75, false),
                ServerboundPlay::Animation,
                ServerboundPlay::UseEntity {
                    target: num(1),
                    action: 1,
                    target_pos: None,
                },
                at(0.0, true),
            ]
        }
        Some(action @ ("sprint" | "unsprint")) => vec![ServerboundPlay::EntityAction {
            entity: own_eid,
            action: if action == "sprint" { 3 } else { 4 },
            jump_boost: 0,
        }],
        Some("hold") => vec![ServerboundPlay::HeldItemChange {
            slot: num(1) as i16,
        }],
        // Using the held item, not at a block: what clients send for buckets, food, ...
        Some("use") => vec![ServerboundPlay::BlockPlacement {
            pos: (-1, -1, -1),
            face: -1,
            item: held.first().cloned().flatten(),
            cursor: (0, 0, 0),
        }],
        Some("place") => vec![
            ServerboundPlay::Animation,
            ServerboundPlay::BlockPlacement {
                pos: (num(1), num(2), num(3)),
                face: num(4) as i8,
                item: held.first().cloned().flatten(),
                cursor: if parts.len() > 7 {
                    (num(5) as i8, num(6) as i8, num(7) as i8)
                } else {
                    (8, 8, 8)
                },
            },
        ],
        _ => Vec::new(),
    }
}

/// Whether an error means the server closed the connection.
fn is_closed(e: &BotError) -> bool {
    matches!(e, BotError::Io(io) if matches!(
        io.kind(),
        std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::UnexpectedEof
    ))
}
