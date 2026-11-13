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
/// Optional `chat` lines are sent once the position is known, 500 ms apart.
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
    let mut pending_chat: Vec<String> = chat.iter().rev().cloned().collect();
    let mut next_chat = start;

    loop {
        tokio::select! {
            () = tokio::time::sleep_until(deadline) => break,
            _ = tick.tick() => {
                if positioned {
                    send_play(&mut conn, &ServerboundPlay::Player { on_ground: true }).await?;
                    if Instant::now() >= next_chat && let Some(message) = pending_chat.pop() {
                        send_play(&mut conn, &ServerboundPlay::Chat { message }).await?;
                        next_chat = Instant::now() + Duration::from_millis(500);
                    }
                }
            }
            packet = conn.recv_packet() => {
                let Some(body) = packet? else {
                    obs.closed_by_server = true;
                    break;
                };
                let mut r = krol_protocol::buf::PacketReader::new(&body);
                let id = r.varint()?;
                let decoded = ClientboundPlay::decode(&body, version)?;
                match &decoded {
                    Some(ClientboundPlay::KeepAlive { id }) => {
                        send_play(&mut conn, &ServerboundPlay::KeepAlive { id: *id }).await?;
                    }
                    Some(ClientboundPlay::JoinGame { .. }) => {
                        send_play(&mut conn, &ServerboundPlay::ClientSettings {
                            locale: "en_US".into(),
                            view_distance: 10,
                            chat_mode: 0,
                            chat_colors: true,
                            skin_parts: 0x7f,
                        }).await?;
                        send_play(&mut conn, &ServerboundPlay::PluginMessage {
                            channel: "MC|Brand".into(),
                            data: b"\x07vanilla".to_vec(),
                        }).await?;
                    }
                    Some(ClientboundPlay::PositionLook { x, y, z, yaw, pitch, .. }) => {
                        send_play(&mut conn, &ServerboundPlay::PositionLook {
                            x: *x, y: *y, z: *z, yaw: *yaw, pitch: *pitch, on_ground: false,
                        }).await?;
                        positioned = true;
                        next_chat = Instant::now() + Duration::from_millis(500);
                    }
                    _ => {}
                }
                obs.packets.push(ObservedPacket {
                    at_ms: start.elapsed().as_millis() as u64,
                    id,
                    body: r.rest().to_vec(),
                    decoded,
                });
            }
        }
    }
    Ok(obs)
}

async fn send_play(conn: &mut Conn, packet: &ServerboundPlay) -> Result<(), BotError> {
    if let Some(bytes) = packet.encode()? {
        conn.send(&bytes).await?;
    }
    Ok(())
}
