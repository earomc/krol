//! A headless, scriptable Minecraft client used by the parity suite to drive both the
//! vanilla server and krol with identical inputs.
//!
//! Every action returns what the client observed (raw packets, decoded where useful) so
//! the parity runner can diff vanilla against krol. Actions never panic on unexpected
//! server behavior; they report it as an observation or an error.

#![deny(missing_docs)]

use std::net::SocketAddr;
use std::time::Duration;

use krol_protocol::frame::{FrameDecoder, encode_frame};
use krol_protocol::packets::handshake::Handshake;
use krol_protocol::packets::login::{ClientboundLogin, ServerboundLogin};
use krol_protocol::packets::status::{ClientboundStatus, ServerboundStatus};
use krol_protocol::{DecodeError, EncodeError, ProtocolVersion};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// How long an action waits for the server before giving up.
pub const ACTION_TIMEOUT: Duration = Duration::from_secs(10);

/// Errors from bot actions.
#[derive(Debug, Error)]
pub enum BotError {
    /// Socket failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The server did not answer in time.
    #[error("timed out waiting for the server")]
    Timeout,
    /// The server sent undecodable data.
    #[error(transparent)]
    Decode(#[from] DecodeError),
    /// A packet could not be encoded.
    #[error(transparent)]
    Encode(#[from] EncodeError),
}

/// What a modern server list ping observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusObservation {
    /// The raw status JSON.
    pub json: String,
    /// The pong payload echoed back, if the server answered the ping.
    pub pong: Option<i64>,
    /// Whether the server closed the connection after the pong.
    pub closed_after_pong: bool,
}

/// A connection with framing helpers.
struct Conn {
    stream: TcpStream,
    decoder: FrameDecoder,
}

impl Conn {
    async fn connect(addr: SocketAddr) -> Result<Self, BotError> {
        let stream = with_timeout(TcpStream::connect(addr)).await??;
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            decoder: FrameDecoder::new(),
        })
    }

    async fn send(&mut self, body: &[u8]) -> Result<(), BotError> {
        self.stream.write_all(&encode_frame(body)?).await?;
        Ok(())
    }

    /// The next frame, or `None` if the server closed the connection.
    async fn recv(&mut self) -> Result<Option<Vec<u8>>, BotError> {
        let mut buf = [0u8; 4096];
        loop {
            if let Some(frame) = self.decoder.next_frame()? {
                return Ok(Some(frame));
            }
            let n = with_timeout(self.stream.read(&mut buf)).await??;
            if n == 0 {
                return Ok(None);
            }
            self.decoder.extend(&buf[..n]);
        }
    }

    async fn handshake(
        &mut self,
        addr: SocketAddr,
        protocol: i32,
        next_state: i32,
    ) -> Result<(), BotError> {
        let hs = Handshake {
            protocol_version: protocol,
            server_address: addr.ip().to_string(),
            server_port: addr.port(),
            next_state,
        };
        self.send(&hs.encode()?).await
    }
}

async fn with_timeout<F: std::future::Future>(f: F) -> Result<F::Output, BotError> {
    tokio::time::timeout(ACTION_TIMEOUT, f)
        .await
        .map_err(|_| BotError::Timeout)
}

/// Performs a modern server list ping (handshake, request, ping) with `protocol`.
///
/// # Errors
///
/// [`BotError`] on connection failure, timeout or undecodable responses.
pub async fn status_ping(
    addr: SocketAddr,
    protocol: i32,
    payload: i64,
) -> Result<StatusObservation, BotError> {
    let mut conn = Conn::connect(addr).await?;
    conn.handshake(addr, protocol, 1).await?;
    conn.send(&ServerboundStatus::Request.encode()).await?;
    let json = match conn.recv().await? {
        Some(frame) => match ClientboundStatus::decode(&frame)? {
            ClientboundStatus::Response(json) => json,
            ClientboundStatus::Pong(_) => {
                return Err(DecodeError::InvalidValue("pong before response".into()).into());
            }
        },
        None => {
            return Ok(StatusObservation {
                json: String::new(),
                pong: None,
                closed_after_pong: true,
            });
        }
    };
    conn.send(&ServerboundStatus::Ping(payload).encode())
        .await?;
    let pong = match conn.recv().await? {
        Some(frame) => match ClientboundStatus::decode(&frame)? {
            ClientboundStatus::Pong(p) => Some(p),
            ClientboundStatus::Response(_) => None,
        },
        None => None,
    };
    let closed_after_pong = conn.recv().await?.is_none();
    Ok(StatusObservation {
        json,
        pong,
        closed_after_pong,
    })
}

/// Sends raw bytes as the first chunk of a connection and returns everything the server
/// sends until it closes (used for legacy pings and malformed-input scenarios).
///
/// # Errors
///
/// [`BotError`] on connection failure or timeout.
pub async fn raw_exchange(addr: SocketAddr, bytes: &[u8]) -> Result<Vec<u8>, BotError> {
    let mut conn = Conn::connect(addr).await?;
    conn.stream.write_all(bytes).await?;
    let mut out = Vec::new();
    with_timeout(conn.stream.read_to_end(&mut out)).await??;
    Ok(out)
}

/// Starts a login with `protocol` and `name`, returning the login packets the server sent
/// before closing the connection or switching to encryption.
///
/// Stops after the first packet that ends the unauthenticated phase (disconnect,
/// encryption request or login success).
///
/// # Errors
///
/// [`BotError`] on connection failure, timeout or undecodable responses.
pub async fn login_start(
    addr: SocketAddr,
    protocol: i32,
    name: &str,
) -> Result<Vec<ClientboundLogin>, BotError> {
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
    let mut packets = Vec::new();
    while let Some(frame) = conn.recv().await? {
        let packet = ClientboundLogin::decode(&frame, version)?;
        let done = !matches!(packet, ClientboundLogin::SetCompression(_));
        packets.push(packet);
        if done {
            break;
        }
    }
    Ok(packets)
}
