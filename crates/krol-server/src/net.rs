//! Network connection handling (vanilla: `NetworkSystem`, `NetworkManager` and the
//! handshake/status/login net handlers).
//!
//! Networking runs on tokio. Packets that affect the game are handed to the tick thread
//! (from M3 on); handshake and status are answered here, as vanilla answers them on the
//! network threads' behalf without touching world state.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use krol_java::gson::JsonValue;
use krol_protocol::frame::{FrameDecoder, encode_frame};
use krol_protocol::legacy::{self, LegacyStatus};
use krol_protocol::packets::handshake::Handshake;
use krol_protocol::packets::login::{ClientboundLogin, ServerboundLogin};
use krol_protocol::packets::status::{ClientboundStatus, ServerboundStatus};
use krol_protocol::{DecodeError, EncodeError, ProtocolVersion};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::RwLock;
use tracing::debug;

use crate::status::StatusResponse;

/// Vanilla's read timeout (`ReadTimeoutHandler(30)`).
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Size of each socket read (Netty's initial adaptive receive buffer).
const READ_CHUNK: usize = 1024;

/// State shared between the network layer and the game.
#[derive(Debug)]
pub struct NetShared {
    /// The status response; vanilla refreshes its player data every 5 seconds.
    pub status: RwLock<StatusResponse>,
    /// The live player count (the legacy ping reads it directly).
    pub online_players: AtomicI32,
}

impl NetShared {
    /// Creates shared state with the given initial status.
    pub fn new(status: StatusResponse) -> Arc<Self> {
        Arc::new(Self {
            online_players: AtomicI32::new(status.online_players),
            status: RwLock::new(status),
        })
    }
}

/// Why a connection ended abnormally.
#[derive(Debug, Error)]
pub enum ConnectionError {
    /// Socket I/O failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// No data within [`READ_TIMEOUT`].
    #[error("timed out")]
    Timeout,
    /// The client sent undecodable data.
    #[error(transparent)]
    Decode(#[from] DecodeError),
    /// A packet could not be encoded.
    #[error(transparent)]
    Encode(#[from] EncodeError),
    /// The handshake requested an unknown next state.
    #[error("Invalid intention {0}")]
    InvalidIntention(i32),
}

enum State {
    Handshaking,
    Status { protocol: i32, responded: bool },
    Login { version: ProtocolVersion },
}

/// Drives one client connection until it closes.
///
/// # Errors
///
/// Returns why the connection ended if it was not a clean close.
pub async fn handle_connection<S>(
    mut stream: S,
    shared: Arc<NetShared>,
) -> Result<(), ConnectionError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; READ_CHUNK];
    let n = read_with_timeout(&mut stream, &mut buf).await?;
    if n == 0 {
        return Ok(());
    }

    if let Some(ping) = legacy::classify(&buf[..n]) {
        debug!("Ping: legacy {ping:?}");
        let status = shared.status.read().await;
        let response = legacy::response(
            &ping,
            &LegacyStatus {
                motd: &status.motd,
                version_name: ProtocolVersion::V47.version_name(),
                online: shared.online_players.load(Ordering::Relaxed),
                max: status.max_players,
            },
        );
        drop(status);
        stream.write_all(&response).await?;
        stream.shutdown().await?;
        return Ok(());
    }

    let mut decoder = FrameDecoder::new();
    decoder.extend(&buf[..n]);
    let mut state = State::Handshaking;

    loop {
        while let Some(frame) = decoder.next_frame()? {
            match &mut state {
                State::Handshaking => {
                    let hs = Handshake::decode(&frame)?;
                    state = match hs.next_state {
                        1 => State::Status {
                            protocol: hs.protocol_version,
                            responded: false,
                        },
                        2 => match ProtocolVersion::from_number(hs.protocol_version) {
                            Some(version) => State::Login { version },
                            None => {
                                let reason = if hs.protocol_version > 47 {
                                    "Outdated server! I'm still on 1.8.8"
                                } else {
                                    "Outdated client! Please use 1.8.8"
                                };
                                // Vanilla answers in LOGIN state with its own (47) encoding.
                                let packet = ClientboundLogin::Disconnect(chat_text(reason));
                                send(&mut stream, &packet.encode(ProtocolVersion::V47)?).await?;
                                stream.shutdown().await?;
                                return Ok(());
                            }
                        },
                        other => return Err(ConnectionError::InvalidIntention(other)),
                    };
                }
                State::Status {
                    protocol,
                    responded,
                } => match ServerboundStatus::decode(&frame)? {
                    ServerboundStatus::Request => {
                        if *responded {
                            // Vanilla: "Status request has been handled."
                            stream.shutdown().await?;
                            return Ok(());
                        }
                        *responded = true;
                        let version =
                            ProtocolVersion::from_number(*protocol).unwrap_or(ProtocolVersion::V47);
                        let json = shared.status.read().await.to_json(version);
                        send(&mut stream, &ClientboundStatus::Response(json).encode()?).await?;
                    }
                    ServerboundStatus::Ping(payload) => {
                        send(&mut stream, &ClientboundStatus::Pong(payload).encode()?).await?;
                        stream.shutdown().await?;
                        return Ok(());
                    }
                },
                State::Login { version } => match ServerboundLogin::decode(&frame, *version)? {
                    ServerboundLogin::Start { name } => {
                        debug!("Login start from {name} (login not implemented yet)");
                        let packet = ClientboundLogin::Disconnect(chat_text(
                            "krol: login is not implemented yet",
                        ));
                        send(&mut stream, &packet.encode(*version)?).await?;
                        stream.shutdown().await?;
                        return Ok(());
                    }
                    ServerboundLogin::EncryptionResponse { .. } => {
                        return Err(DecodeError::InvalidValue(
                            "unexpected encryption response".into(),
                        )
                        .into());
                    }
                },
            }
        }
        let n = read_with_timeout(&mut stream, &mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        decoder.extend(&buf[..n]);
    }
}

/// Serializes a plain text chat component like vanilla (a bare JSON string).
pub fn chat_text(text: &str) -> String {
    JsonValue::from(text).to_gson_string()
}

async fn read_with_timeout<S: AsyncRead + Unpin>(
    stream: &mut S,
    buf: &mut [u8],
) -> Result<usize, ConnectionError> {
    match tokio::time::timeout(READ_TIMEOUT, stream.read(buf)).await {
        Ok(result) => Ok(result?),
        Err(_) => Err(ConnectionError::Timeout),
    }
}

async fn send<S: AsyncWrite + Unpin>(stream: &mut S, body: &[u8]) -> Result<(), ConnectionError> {
    stream.write_all(&encode_frame(body)?).await?;
    stream.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use krol_protocol::packets::handshake::Handshake;

    fn shared() -> Arc<NetShared> {
        NetShared::new(StatusResponse {
            motd: "A Minecraft Server".into(),
            max_players: 20,
            online_players: 0,
            sample: vec![],
            favicon: None,
        })
    }

    fn frame(body: Vec<u8>) -> Vec<u8> {
        encode_frame(&body).unwrap()
    }

    fn handshake(protocol: i32, next: i32) -> Vec<u8> {
        frame(
            Handshake {
                protocol_version: protocol,
                server_address: "localhost".into(),
                server_port: 25565,
                next_state: next,
            }
            .encode()
            .unwrap(),
        )
    }

    async fn exchange(input: Vec<u8>) -> (Result<(), ConnectionError>, Vec<u8>) {
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(handle_connection(server, shared()));
        client.write_all(&input).await.unwrap();
        let mut out = Vec::new();
        client.read_to_end(&mut out).await.unwrap();
        (task.await.unwrap(), out)
    }

    fn frames(bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut dec = FrameDecoder::new();
        dec.extend(bytes);
        std::iter::from_fn(|| dec.next_frame().unwrap()).collect()
    }

    #[tokio::test]
    async fn status_then_ping() {
        let mut input = handshake(47, 1);
        input.extend(frame(ServerboundStatus::Request.encode()));
        input.extend(frame(ServerboundStatus::Ping(1234).encode()));
        let (result, out) = exchange(input).await;
        result.unwrap();
        let f = frames(&out);
        assert_eq!(f.len(), 2);
        let ClientboundStatus::Response(json) = ClientboundStatus::decode(&f[0]).unwrap() else {
            panic!("expected response");
        };
        assert!(json.contains("\"protocol\":47"));
        assert_eq!(
            ClientboundStatus::decode(&f[1]).unwrap(),
            ClientboundStatus::Pong(1234)
        );
    }

    #[tokio::test]
    async fn status_reports_v5_to_1_7_clients() {
        let mut input = handshake(5, 1);
        input.extend(frame(ServerboundStatus::Request.encode()));
        input.extend(frame(ServerboundStatus::Ping(1).encode()));
        let (_, out) = exchange(input).await;
        let ClientboundStatus::Response(json) =
            ClientboundStatus::decode(&frames(&out)[0]).unwrap()
        else {
            panic!("expected response");
        };
        assert!(json.contains("\"name\":\"1.7.10\",\"protocol\":5"));
    }

    #[tokio::test]
    async fn second_request_closes() {
        let mut input = handshake(47, 1);
        input.extend(frame(ServerboundStatus::Request.encode()));
        input.extend(frame(ServerboundStatus::Request.encode()));
        let (result, out) = exchange(input).await;
        result.unwrap();
        assert_eq!(frames(&out).len(), 1);
    }

    #[tokio::test]
    async fn rejects_unsupported_login_versions() {
        for (protocol, reason) in [
            (48, "\"Outdated server! I\\u0027m still on 1.8.8\""),
            (4, "\"Outdated client! Please use 1.8.8\""),
        ] {
            let (result, out) = exchange(handshake(protocol, 2)).await;
            result.unwrap();
            let f = frames(&out);
            assert_eq!(
                ClientboundLogin::decode(&f[0], ProtocolVersion::V47).unwrap(),
                ClientboundLogin::Disconnect(reason.into())
            );
        }
    }

    #[tokio::test]
    async fn legacy_ping() {
        let (result, out) = exchange(vec![0xfe, 0x01]).await;
        result.unwrap();
        assert_eq!(out[0], 0xff);
        let units: Vec<u16> = out[3..]
            .chunks(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(
            String::from_utf16(&units).unwrap(),
            "\u{a7}1\u{0}127\u{0}1.8.8\u{0}A Minecraft Server\u{0}0\u{0}20"
        );
    }

    #[tokio::test]
    async fn invalid_intention_errors() {
        let (result, _) = exchange(handshake(47, 3)).await;
        assert!(matches!(result, Err(ConnectionError::InvalidIntention(3))));
    }

    #[tokio::test(start_paused = true)]
    async fn idle_connection_times_out() {
        let (_client, server) = tokio::io::duplex(1024);
        let result = handle_connection(server, shared()).await;
        assert!(matches!(result, Err(ConnectionError::Timeout)));
    }
}
