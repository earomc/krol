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
/// `!place <x> <y> <z> <face>` (use the held item, cursor 8/8/8; sends the swing first),
/// `!hold <slot>` (select hotbar slot), `!swing` (arm animation),
/// `!look <yaw> <pitch>` (Player Look, on ground). Optional
/// `moves` (offsets `[dx, dy, dz]` from the first server-given position, optionally
/// `[dx, dy, dz, on_ground]` with on_ground 0 or 1, default 1) replace the idle Player
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

    let result: Result<(), BotError> = async { loop {
        tokio::select! {
            () = tokio::time::sleep_until(deadline) => break,
            _ = tick.tick() => {
                if positioned {
                    match (origin, moves.get(next_move)) {
                        (Some((ox, oy, oz)), Some(m)) => {
                            next_move += 1;
                            let at = |k: usize| m.get(k).copied().unwrap_or(0.0);
                            send_play(&mut conn, v5_eid, &ServerboundPlay::Position {
                                x: ox + at(0), y: oy + at(1), z: oz + at(2),
                                on_ground: m.get(3).is_none_or(|g| *g != 0.0),
                            }).await?;
                        }
                        _ => send_play(&mut conn, v5_eid, &ServerboundPlay::Player { on_ground: true }).await?,
                    }
                    if Instant::now() >= next_chat && let Some(message) = pending_chat.pop() {
                        send_play(&mut conn, v5_eid, &ServerboundPlay::Chat { message }).await?;
                        next_chat = Instant::now() + Duration::from_millis(500);
                    }
                    if let Some(at) = positioned_at {
                        let elapsed = at.elapsed().as_millis() as u64;
                        let due: Vec<String> = timed_chat
                            .iter()
                            .filter(|(ms, _)| *ms <= elapsed)
                            .map(|(_, t)| t.clone())
                            .collect();
                        timed_chat.retain(|(ms, _)| *ms > elapsed);
                        for message in due {
                            for packet in action_or_chat(&message, &held) {
                                if let ServerboundPlay::HeldItemChange { slot } = packet {
                                    held_slot = slot;
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
                    Some(ClientboundPlay::PositionLook { x, y, z, yaw, pitch, .. }) => {
                        send_play(&mut conn, v5_eid, &ServerboundPlay::PositionLook {
                            x: *x, y: *y, z: *z, yaw: *yaw, pitch: *pitch, on_ground: false,
                        }).await?;
                        positioned = true;
                        positioned_at.get_or_insert_with(Instant::now);
                        origin.get_or_insert((*x, *y, *z));
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
        Some("look") => vec![ServerboundPlay::Look {
            yaw: parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0.0),
            pitch: parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(0.0),
            on_ground: true,
        }],
        Some("hold") => vec![ServerboundPlay::HeldItemChange {
            slot: num(1) as i16,
        }],
        Some("place") => vec![
            ServerboundPlay::Animation,
            ServerboundPlay::BlockPlacement {
                pos: (num(1), num(2), num(3)),
                face: num(4) as i8,
                item: held.first().cloned().flatten(),
                cursor: (8, 8, 8),
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
