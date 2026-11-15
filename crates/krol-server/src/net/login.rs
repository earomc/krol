//! The login state (vanilla: `NetHandlerLoginServer`).

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use krol_protocol::ProtocolVersion;
use krol_protocol::packets::login::{ClientboundLogin, ServerboundLogin};
use krol_protocol::packets::play::ClientboundPlay;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;
use tracing::{debug, error, info};

use super::connection::Connection;
use super::{ConnectionError, NetShared, chat_text};
use crate::auth::{self, GameProfile, SessionResult};
use crate::status::java_uuid_string;

/// Vanilla's login timeout: the check `connectionTimer++ == 600` trips on the 601st tick,
/// 30.05 s at 20 TPS. An idle client therefore hits the 30 s read timeout ("Timed out")
/// first; "Took too long to log in" needs a client that keeps sending data.
pub const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(30_050);

/// The profile as vanilla knows it during login: the id is unknown until authenticated
/// (online) or derived (offline).
struct LoginProfile {
    id: Option<u128>,
    name: String,
    properties: Vec<auth::ProfileProperty>,
    /// Stand-in for Java's identity hash code in `GameProfile.toString()`.
    identity_hash: u32,
}

impl LoginProfile {
    fn describe(&self) -> String {
        let id = self
            .id
            .map_or_else(|| "<null>".to_owned(), java_uuid_string);
        let properties = if self.properties.is_empty() {
            "{}".to_owned()
        } else {
            let entries: Vec<String> = self
                .properties
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    format!(
                        "{}=[com.mojang.authlib.properties.Property@{:x}]",
                        p.name,
                        self.identity_hash.wrapping_add(i as u32 + 1)
                    )
                })
                .collect();
            format!("{{{}}}", entries.join(", "))
        };
        format!(
            "com.mojang.authlib.GameProfile@{:x}[id={id},name={},properties={properties},legacy=false]",
            self.identity_hash, self.name
        )
    }
}

struct LoginContext<'a> {
    shared: &'a NetShared,
    peer: SocketAddr,
    profile: Option<LoginProfile>,
}

impl LoginContext<'_> {
    /// Vanilla `getConnectionInfo()`.
    fn info(&self) -> String {
        let peer = java_socket_address(self.peer);
        match &self.profile {
            Some(p) => format!("{} ({peer})", p.describe()),
            None => peer,
        }
    }
}

/// Java's `InetSocketAddress.toString()` for a resolved address without host name.
fn java_socket_address(addr: SocketAddr) -> String {
    format!("/{}:{}", addr.ip(), addr.port())
}

/// Vanilla `closeConnection(reason)`: log, send Disconnect, close; then the handler's
/// `onDisconnect` logs "lost connection".
async fn close_connection<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    ctx: &LoginContext<'_>,
    version: ProtocolVersion,
    reason: &str,
    thread: &str,
) -> Result<(), ConnectionError> {
    info!(thread = thread, "Disconnecting {}: {reason}", ctx.info());
    let packet = ClientboundLogin::Disconnect(chat_text(reason));
    let result = conn.send(&packet.encode(version)?).await;
    conn.close().await;
    info!("{} lost connection: {reason}", ctx.info());
    result
}

/// Runs the login state after a handshake requesting it with `version`.
pub(super) async fn run<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    mut conn: Connection<S>,
    shared: Arc<NetShared>,
    peer: SocketAddr,
    version: ProtocolVersion,
) -> Result<(), ConnectionError> {
    let deadline = Instant::now() + LOGIN_TIMEOUT;
    let mut ctx = LoginContext {
        shared: &shared,
        peer,
        profile: None,
    };
    match login(&mut conn, &mut ctx, version, deadline).await {
        Ok(Some(profile)) => match shared.game.get() {
            Some(game) => {
                let info = ctx.info();
                super::play::run(
                    conn,
                    shared.clone(),
                    game.clone(),
                    peer,
                    version,
                    profile,
                    info,
                )
                .await
            }
            None => {
                // No world loaded (world creation is not implemented yet).
                debug!("{} logged in but no world is loaded", profile.name);
                let bye = ClientboundPlay::Disconnect(chat_text("krol: no world is loaded"));
                conn.send(&bye.encode(version)?).await?;
                conn.close().await;
                Ok(())
            }
        },
        Ok(None) => Ok(()),
        Err(ConnectionError::Deadline) => {
            close_connection(
                &mut conn,
                &ctx,
                version,
                "Took too long to log in",
                "Server thread",
            )
            .await
        }
        Err(e) => {
            // The connection dies with an exception or a closed socket; vanilla's
            // onDisconnect logs the reason.
            let reason = match &e {
                ConnectionError::Timeout => "Timed out".to_owned(),
                ConnectionError::Closed => "End of stream".to_owned(),
                other => format!("Internal Exception: {}", other.java_exception()),
            };
            conn.close().await;
            info!("{} lost connection: {reason}", ctx.info());
            match e {
                ConnectionError::Closed => Ok(()),
                other => Err(other),
            }
        }
    }
}

async fn expect_packet<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    version: ProtocolVersion,
    deadline: Instant,
) -> Result<ServerboundLogin, ConnectionError> {
    match conn.next_packet(Some(deadline)).await? {
        Some(body) => Ok(ServerboundLogin::decode(&body, version)?),
        None => Err(ConnectionError::Closed),
    }
}

/// Runs the login exchange; returns the profile once Login Success was sent, or `None`
/// if the login ended with a disconnect.
async fn login<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    ctx: &mut LoginContext<'_>,
    version: ProtocolVersion,
    deadline: Instant,
) -> Result<Option<GameProfile>, ConnectionError> {
    let shared = ctx.shared;
    let name = match expect_packet(conn, version, deadline).await? {
        ServerboundLogin::Start { name } => name,
        ServerboundLogin::EncryptionResponse { .. } => {
            return Err(ConnectionError::IllegalState("Unexpected key packet"));
        }
    };
    ctx.profile = Some(LoginProfile {
        id: None,
        name: name.clone(),
        properties: Vec::new(),
        identity_hash: shared.next_identity_hash(),
    });

    let profile = if shared.config.online_mode {
        let keys = &shared.config.keys;
        let mut verify_token = [0u8; 4];
        getrandom::fill(&mut verify_token).map_err(|e| std::io::Error::other(e.to_string()))?;
        let request = ClientboundLogin::EncryptionRequest {
            server_id: String::new(),
            public_key: keys.public_der().to_vec(),
            verify_token: verify_token.to_vec(),
        };
        conn.send(&request.encode(version)?).await?;

        let (secret, token) = match expect_packet(conn, version, deadline).await? {
            ServerboundLogin::EncryptionResponse {
                shared_secret,
                verify_token,
            } => (shared_secret, verify_token),
            ServerboundLogin::Start { .. } => {
                return Err(ConnectionError::IllegalState("Unexpected hello packet"));
            }
        };
        let decrypted_token = keys.decrypt(&token).unwrap_or_default();
        if decrypted_token != verify_token {
            return Err(ConnectionError::IllegalState("Invalid nonce!"));
        }
        let secret: [u8; 16] = keys
            .decrypt(&secret)
            .ok()
            .and_then(|s| s.try_into().ok())
            .ok_or(ConnectionError::IllegalState("Invalid shared secret"))?;
        conn.enable_encryption(&secret);

        let hash = auth::server_id_hash("", &secret, keys.public_der());
        let thread = format!(
            "User Authenticator #{}",
            shared.authenticator_ids.fetch_add(1, Ordering::Relaxed) + 1
        );
        let url = shared.config.session_server.clone();
        let query_name = name.clone();
        let lookup =
            tokio::task::spawn_blocking(move || auth::has_joined(&url, &query_name, &hash));
        let result = match tokio::time::timeout_at(deadline, lookup).await {
            Ok(joined) => joined.map_err(|e| std::io::Error::other(e.to_string()))?,
            Err(_) => return Err(ConnectionError::Deadline),
        };
        match result {
            SessionResult::Joined(profile) => {
                info!(
                    thread = thread.as_str(),
                    "UUID of player {} is {}",
                    profile.name,
                    java_uuid_string(profile.id)
                );
                profile
            }
            SessionResult::NotJoined => {
                close_connection(conn, ctx, version, "Failed to verify username!", &thread).await?;
                error!(
                    thread = thread.as_str(),
                    "Username '{name}' tried to join with an invalid session"
                );
                return Ok(None);
            }
            SessionResult::Unavailable(reason) => {
                debug!("session server unavailable: {reason}");
                close_connection(
                    conn,
                    ctx,
                    version,
                    "Authentication servers are down. Please try again later, sorry!",
                    &thread,
                )
                .await?;
                error!(
                    thread = thread.as_str(),
                    "Couldn't verify username because servers are unavailable"
                );
                return Ok(None);
            }
        }
    } else {
        GameProfile {
            id: auth::offline_uuid(&name),
            name: name.clone(),
            properties: Vec::new(),
        }
    };
    if let Some(p) = &mut ctx.profile {
        p.id = Some(profile.id);
        p.name.clone_from(&profile.name);
        p.properties.clone_from(&profile.properties);
    }

    // Vanilla `tryAcceptPlayer` / `allowUserToConnect`.
    let refusal = shared
        .user_lists
        .lock()
        .map_err(|_| std::io::Error::other("user lists poisoned"))?
        .allow_user_to_connect(
            profile.id,
            ctx.peer,
            shared.online_players.load(Ordering::Relaxed),
            shared.config.max_players,
        );
    if let Some(reason) = refusal {
        close_connection(conn, ctx, version, &reason, "Server thread").await?;
        return Ok(None);
    }
    if let Ok(threshold) = usize::try_from(shared.config.compression_threshold)
        && version == ProtocolVersion::V47
    {
        conn.send(&ClientboundLogin::SetCompression(threshold as i32).encode(version)?)
            .await?;
        conn.enable_compression(threshold);
    }
    let success = ClientboundLogin::Success {
        uuid: java_uuid_string(profile.id),
        username: profile.name.clone(),
    };
    conn.send(&success.encode(version)?).await?;
    Ok(Some(profile))
}
