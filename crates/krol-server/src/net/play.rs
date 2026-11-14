//! The play state's network side: decode packets for the game thread and write the
//! game's packets to the socket.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use krol_protocol::ProtocolVersion;
use krol_protocol::packets::play::{ClientboundPlay, ServerboundPlay};
use krol_protocol::translate::V5Translator;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc::UnboundedSender;

use super::connection::Connection;
use super::{ConnectionError, NetShared};
use crate::auth::GameProfile;
use crate::game::{DisconnectReason, GameEvent, Outgoing};

/// Hands a logged-in connection to the game and pumps packets until it ends.
pub(super) async fn run<S>(
    conn: Connection<S>,
    shared: Arc<NetShared>,
    game: UnboundedSender<GameEvent>,
    peer: SocketAddr,
    version: ProtocolVersion,
    profile: GameProfile,
) -> Result<(), ConnectionError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let conn_id = shared.next_conn_id.fetch_add(1, Ordering::Relaxed);
    let (mut reader, mut writer) = conn.into_split();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Outgoing>();
    if game
        .send(GameEvent::Join {
            conn_id,
            profile,
            peer,
            version,
            out: out_tx,
        })
        .is_err()
    {
        return Ok(());
    }

    // 1.7 clients: packets are translated at this edge; the game only sees protocol 47.
    let translator = (version == ProtocolVersion::V5)
        .then(|| std::sync::Arc::new(std::sync::Mutex::new(V5Translator::new())));
    let encoder = translator.clone();
    let encode = move |p: &ClientboundPlay| -> Vec<Vec<u8>> {
        match &encoder {
            Some(t) => t
                .lock()
                .ok()
                .and_then(|mut t| t.clientbound(p).ok())
                .unwrap_or_default(),
            None => p.encode(version).map(|b| vec![b]).unwrap_or_default(),
        }
    };

    let writer_task = tokio::spawn(async move {
        while let Some(out) = out_rx.recv().await {
            match out {
                Outgoing::Packet(p) => {
                    for bytes in encode(&p) {
                        if writer.send(&bytes).await.is_err() {
                            return false;
                        }
                    }
                }
                Outgoing::Disconnect(json) => {
                    for bytes in encode(&ClientboundPlay::Disconnect(json)) {
                        let _ = writer.send(&bytes).await;
                    }
                    writer.close().await;
                    return true;
                }
            }
        }
        false
    });

    let reason = loop {
        tokio::select! {
            packet = reader.next_packet() => match packet {
                Ok(Some(body)) => match match &translator {
                    Some(t) => t
                        .lock()
                        .map_err(|_| krol_protocol::DecodeError::UnexpectedEnd)
                        .and_then(|mut t| t.serverbound(&body)),
                    None => ServerboundPlay::decode(&body, version),
                } {
                    Ok(packet) => {
                        if game.send(GameEvent::Packet { conn_id, packet }).is_err() {
                            break None;
                        }
                    }
                    Err(e) => {
                        break Some(DisconnectReason::Internal(
                            ConnectionError::from(e).java_exception(),
                        ));
                    }
                },
                Ok(None) => break Some(DisconnectReason::Closed),
                // A reset (e.g. a client closing with unread data on Windows) ends the
                // connection like a normal close in vanilla's logs.
                Err(ConnectionError::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                    ) =>
                {
                    break Some(DisconnectReason::Closed);
                }
                Err(ConnectionError::Timeout) => break Some(DisconnectReason::Timeout),
                Err(e) => break Some(DisconnectReason::Internal(e.java_exception())),
            },
            // The writer ends when the game disconnected the player (kick/shutdown).
            finished = async { while !writer_task.is_finished() { tokio::time::sleep(std::time::Duration::from_millis(20)).await; } } => {
                let () = finished;
                break None;
            }
        }
    };
    if let Some(reason) = reason {
        let _ = game.send(GameEvent::Disconnected { conn_id, reason });
    }
    Ok(())
}
