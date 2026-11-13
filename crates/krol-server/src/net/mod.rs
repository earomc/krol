//! Network connection handling (vanilla: `NetworkSystem`, `NetworkManager` and the
//! handshake/status/login net handlers).
//!
//! Networking runs on tokio. Packets that affect the game are handed to the tick thread
//! (from M3 on); handshake, status and login are handled here, as vanilla handles them on
//! its network threads without touching world state.

mod connection;
mod login;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::time::Duration;

use krol_java::gson::JsonValue;
use krol_protocol::legacy::{self, LegacyStatus};
use krol_protocol::packets::handshake::Handshake;
use krol_protocol::packets::login::ClientboundLogin;
use krol_protocol::packets::status::{ClientboundStatus, ServerboundStatus};
use krol_protocol::{DecodeError, EncodeError, ProtocolVersion};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::RwLock;
use tracing::debug;

use crate::auth::ServerKeyPair;
use crate::status::StatusResponse;
use connection::{Connection, READ_CHUNK};

pub use login::LOGIN_TIMEOUT;

/// Vanilla's read timeout (`ReadTimeoutHandler(30)`).
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Network-relevant server settings.
#[derive(Debug)]
pub struct NetConfig {
    /// `online-mode`: authenticate players with the session server and encrypt.
    pub online_mode: bool,
    /// `network-compression-threshold`; negative disables compression.
    pub compression_threshold: i32,
    /// `max-players`.
    pub max_players: i32,
    /// Session server `hasJoined` URL.
    pub session_server: String,
    /// The server's RSA key pair.
    pub keys: ServerKeyPair,
}

/// State shared between the network layer and the game.
#[derive(Debug)]
pub struct NetShared {
    /// Network settings.
    pub config: NetConfig,
    /// The status response; vanilla refreshes its player data every 5 seconds.
    pub status: RwLock<StatusResponse>,
    /// The live player count (the legacy ping and login checks read it directly).
    pub online_players: AtomicI32,
    /// Counter for "User Authenticator #N" thread names.
    authenticator_ids: AtomicU32,
    /// Source of stand-in identity hash codes for logged Java objects.
    identity_hashes: AtomicU32,
}

impl NetShared {
    /// Creates shared state.
    pub fn new(config: NetConfig, status: StatusResponse) -> Arc<Self> {
        Arc::new(Self {
            config,
            online_players: AtomicI32::new(status.online_players),
            status: RwLock::new(status),
            authenticator_ids: AtomicU32::new(0),
            identity_hashes: AtomicU32::new(0x1b6d_3586),
        })
    }

    /// A plausible, varying stand-in for `System.identityHashCode` in log output.
    fn next_identity_hash(&self) -> u32 {
        let n = self
            .identity_hashes
            .fetch_add(0x9e37_79b9, Ordering::Relaxed);
        (n ^ (n >> 15)).wrapping_mul(0x2c1b_3c6d) & 0x7fff_ffff
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
    /// A state-specific deadline passed (e.g. the login timeout).
    #[error("deadline passed")]
    Deadline,
    /// The client closed the connection.
    #[error("end of stream")]
    Closed,
    /// The client sent undecodable data.
    #[error(transparent)]
    Decode(#[from] DecodeError),
    /// A packet could not be encoded.
    #[error(transparent)]
    Encode(#[from] EncodeError),
    /// The handshake requested an unknown next state.
    #[error("Invalid intention {0}")]
    InvalidIntention(i32),
    /// A packet arrived in a state where vanilla rejects it (`IllegalStateException`).
    #[error("{0}")]
    IllegalState(&'static str),
}

impl ConnectionError {
    /// The Java exception vanilla would report, as in "Internal Exception: ...".
    ///
    /// Approximate for decode errors: vanilla wraps them in varying Netty exceptions.
    pub fn java_exception(&self) -> String {
        match self {
            Self::Io(e) => format!("java.io.IOException: {e}"),
            Self::Timeout => "io.netty.handler.timeout.ReadTimeoutException".into(),
            Self::Deadline | Self::Closed => "java.io.IOException: closed".into(),
            Self::Decode(e) => format!("io.netty.handler.codec.DecoderException: {e}"),
            Self::Encode(e) => format!("io.netty.handler.codec.EncoderException: {e}"),
            Self::InvalidIntention(n) => {
                format!("java.lang.UnsupportedOperationException: Invalid intention {n}")
            }
            Self::IllegalState(msg) => format!("java.lang.IllegalStateException: {msg}"),
        }
    }
}

/// Drives one client connection until it closes.
///
/// # Errors
///
/// Returns why the connection ended if it was not a clean close.
pub async fn handle_connection<S>(
    mut stream: S,
    peer: SocketAddr,
    shared: Arc<NetShared>,
) -> Result<(), ConnectionError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; READ_CHUNK];
    let n = match tokio::time::timeout(READ_TIMEOUT, stream.read(&mut buf)).await {
        Ok(result) => result?,
        Err(_) => return Err(ConnectionError::Timeout),
    };
    if n == 0 {
        return Ok(());
    }

    if let Some(ping) = legacy::classify(&buf[..n]) {
        debug!("Ping: legacy {ping:?} from {peer}");
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

    let mut conn = Connection::new(stream, &buf[..n]);
    let Some(frame) = conn.next_packet(None).await? else {
        return Ok(());
    };
    let hs = Handshake::decode(&frame)?;
    match hs.next_state {
        1 => status(conn, &shared, hs.protocol_version).await,
        2 => match ProtocolVersion::from_number(hs.protocol_version) {
            Some(version) => login::run(conn, shared, peer, version).await,
            None => {
                let reason = if hs.protocol_version > 47 {
                    "Outdated server! I'm still on 1.8.8"
                } else {
                    "Outdated client! Please use 1.8.8"
                };
                // Vanilla answers in LOGIN state with its own (47) encoding.
                let packet = ClientboundLogin::Disconnect(chat_text(reason));
                conn.send(&packet.encode(ProtocolVersion::V47)?).await?;
                conn.close().await;
                Ok(())
            }
        },
        other => Err(ConnectionError::InvalidIntention(other)),
    }
}

/// The status state (vanilla: `NetHandlerStatusServer`).
async fn status<S: AsyncRead + AsyncWrite + Unpin>(
    mut conn: Connection<S>,
    shared: &NetShared,
    protocol: i32,
) -> Result<(), ConnectionError> {
    let mut responded = false;
    while let Some(body) = conn.next_packet(None).await? {
        match ServerboundStatus::decode(&body)? {
            ServerboundStatus::Request => {
                if responded {
                    // Vanilla: "Status request has been handled."
                    break;
                }
                responded = true;
                let version =
                    ProtocolVersion::from_number(protocol).unwrap_or(ProtocolVersion::V47);
                let json = shared.status.read().await.to_json(version);
                conn.send(&ClientboundStatus::Response(json).encode()?)
                    .await?;
            }
            ServerboundStatus::Ping(payload) => {
                conn.send(&ClientboundStatus::Pong(payload).encode()?)
                    .await?;
                break;
            }
        }
    }
    conn.close().await;
    Ok(())
}

/// Serializes a plain text chat component like vanilla (a bare JSON string).
pub fn chat_text(text: &str) -> String {
    JsonValue::from(text).to_gson_string()
}

#[cfg(test)]
mod tests;
