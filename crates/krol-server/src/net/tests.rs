use std::sync::OnceLock;

use super::*;
use krol_protocol::cipher::Cfb8;
use krol_protocol::compression;
use krol_protocol::frame::{FrameDecoder, encode_frame};
use krol_protocol::packets::login::ServerboundLogin;
use krol_protocol::packets::play::ClientboundPlay;

/// RSA key generation is slow in debug builds; share one key pair across tests.
fn keys() -> ServerKeyPair {
    static KEYS: OnceLock<ServerKeyPair> = OnceLock::new();
    KEYS.get_or_init(|| ServerKeyPair::generate().unwrap())
        .clone()
}

fn shared_with(online_mode: bool, session_server: &str, max_players: i32) -> Arc<NetShared> {
    NetShared::new(
        NetConfig {
            online_mode,
            compression_threshold: 256,
            max_players,
            session_server: session_server.to_owned(),
            keys: keys(),
        },
        StatusResponse {
            motd: "A Minecraft Server".into(),
            max_players,
            online_players: 0,
            sample: vec![],
            favicon: None,
        },
        crate::userlist::UserLists::load(
            &std::env::temp_dir().join("krol-net-tests-nonexistent"),
            false,
        ),
    )
}

fn shared() -> Arc<NetShared> {
    shared_with(false, "http://127.0.0.1:1/", 20)
}

fn peer() -> SocketAddr {
    "127.0.0.1:50000".parse().unwrap()
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

async fn exchange_with(
    input: Vec<u8>,
    shared: Arc<NetShared>,
) -> (Result<(), ConnectionError>, Vec<u8>) {
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let task = tokio::spawn(handle_connection(server, peer(), shared));
    client.write_all(&input).await.unwrap();
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    (task.await.unwrap(), out)
}

async fn exchange(input: Vec<u8>) -> (Result<(), ConnectionError>, Vec<u8>) {
    exchange_with(input, shared()).await
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
    let ClientboundStatus::Response(json) = ClientboundStatus::decode(&frames(&out)[0]).unwrap()
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
    let result = handle_connection(server, peer(), shared()).await;
    assert!(matches!(result, Err(ConnectionError::Timeout)));
}

fn login_start(version: ProtocolVersion, name: &str) -> Vec<u8> {
    frame(
        ServerboundLogin::Start {
            name: name.to_owned(),
        }
        .encode(version)
        .unwrap(),
    )
}

#[tokio::test]
async fn offline_login_47_compresses_and_succeeds() {
    let mut input = handshake(47, 2);
    input.extend(login_start(ProtocolVersion::V47, "Notch"));
    let (result, out) = exchange(input).await;
    result.unwrap();
    let f = frames(&out);
    assert_eq!(
        ClientboundLogin::decode(&f[0], ProtocolVersion::V47).unwrap(),
        ClientboundLogin::SetCompression(256)
    );
    let success = compression::decompress(&f[1], 256).unwrap();
    assert_eq!(
        ClientboundLogin::decode(&success, ProtocolVersion::V47).unwrap(),
        ClientboundLogin::Success {
            uuid: "b50ad385-829d-3141-a216-7e7d7539ba7f".into(),
            username: "Notch".into()
        }
    );
}

#[tokio::test]
async fn offline_login_5_has_no_compression() {
    let mut input = handshake(5, 2);
    input.extend(login_start(ProtocolVersion::V5, "Notch"));
    let (result, out) = exchange(input).await;
    result.unwrap();
    let f = frames(&out);
    assert!(matches!(
        ClientboundLogin::decode(&f[0], ProtocolVersion::V5).unwrap(),
        ClientboundLogin::Success { .. }
    ));
}

#[tokio::test]
async fn full_server_rejects() {
    let mut input = handshake(47, 2);
    input.extend(login_start(ProtocolVersion::V47, "Steve"));
    let (result, out) = exchange_with(input, shared_with(false, "", 0)).await;
    result.unwrap();
    assert_eq!(
        ClientboundLogin::decode(&frames(&out)[0], ProtocolVersion::V47).unwrap(),
        ClientboundLogin::Disconnect("\"The server is full!\"".into())
    );
}

#[tokio::test(start_paused = true)]
async fn idle_login_hits_read_timeout_first() {
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let task = tokio::spawn(handle_connection(server, peer(), shared()));
    client.write_all(&handshake(47, 2)).await.unwrap();
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    assert!(matches!(task.await.unwrap(), Err(ConnectionError::Timeout)));
    assert!(out.is_empty(), "vanilla sends nothing on a read timeout");
}

#[tokio::test(start_paused = true)]
async fn slow_login_takes_too_long() {
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let task = tokio::spawn(handle_connection(server, peer(), shared()));
    client.write_all(&handshake(47, 2)).await.unwrap();
    // Trickle one byte of an unfinished packet every 10 s so the read timeout never fires.
    let mut trickle = vec![0x10u8, 0x00, 0x0e];
    trickle.extend(b"aaaaaaaaaa");
    for byte in trickle.iter().take(3) {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        client.write_all(&[*byte]).await.unwrap();
    }
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(
        ClientboundLogin::decode(&frames(&out)[0], ProtocolVersion::V47).unwrap(),
        ClientboundLogin::Disconnect("\"Took too long to log in\"".into())
    );
}

/// A one-shot HTTP server answering every request with `status` and `body`.
async fn mock_session_server(status: &'static str, body: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf).await;
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    format!("http://{addr}/session/minecraft/hasJoined")
}

/// Performs the client side of an online-mode login and returns the decrypted frames
/// received after encryption was enabled.
async fn online_login(shared: Arc<NetShared>) -> Vec<Vec<u8>> {
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let task = tokio::spawn(handle_connection(server, peer(), shared.clone()));
    let mut input = handshake(47, 2);
    input.extend(login_start(ProtocolVersion::V47, "Notch"));
    client.write_all(&input).await.unwrap();

    // Read the encryption request.
    let mut dec = FrameDecoder::new();
    let mut buf = [0u8; 4096];
    let request = loop {
        if let Some(f) = dec.next_frame().unwrap() {
            break f;
        }
        let n = client.read(&mut buf).await.unwrap();
        dec.extend(&buf[..n]);
    };
    let ClientboundLogin::EncryptionRequest {
        server_id,
        public_key,
        verify_token,
    } = ClientboundLogin::decode(&request, ProtocolVersion::V47).unwrap()
    else {
        panic!("expected encryption request");
    };
    assert_eq!(server_id, "");
    assert_eq!(public_key, shared.config.keys.public_der());
    assert_eq!(verify_token.len(), 4);

    let secret = [0x42u8; 16];
    let response = ServerboundLogin::EncryptionResponse {
        shared_secret: shared.config.keys.encrypt(&secret).unwrap(),
        verify_token: shared.config.keys.encrypt(&verify_token).unwrap(),
    };
    client
        .write_all(&frame(response.encode(ProtocolVersion::V47).unwrap()))
        .await
        .unwrap();

    let mut rest = dec.buffered().to_vec();
    client.read_to_end(&mut rest).await.unwrap();
    task.await.unwrap().unwrap();
    Cfb8::decryptor(&secret).apply(&mut rest);
    frames(&rest)
}

#[tokio::test]
async fn online_login_succeeds_with_valid_session() {
    let url = mock_session_server(
        "200 OK",
        r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch","properties":[]}"#,
    )
    .await;
    let f = online_login(shared_with(true, &url, 20)).await;
    assert_eq!(
        ClientboundLogin::decode(&f[0], ProtocolVersion::V47).unwrap(),
        ClientboundLogin::SetCompression(256)
    );
    let success = compression::decompress(&f[1], 256).unwrap();
    assert_eq!(
        ClientboundLogin::decode(&success, ProtocolVersion::V47).unwrap(),
        ClientboundLogin::Success {
            uuid: "069a79f4-44e9-4726-a5be-fca90e38aaf5".into(),
            username: "Notch".into()
        }
    );
    let bye = compression::decompress(&f[2], 256).unwrap();
    assert!(matches!(
        ClientboundPlay::decode(&bye, ProtocolVersion::V47).unwrap(),
        Some(ClientboundPlay::Disconnect(_))
    ));
}

#[tokio::test]
async fn online_login_rejects_invalid_session() {
    let url = mock_session_server("204 No Content", "").await;
    let f = online_login(shared_with(true, &url, 20)).await;
    assert_eq!(
        ClientboundLogin::decode(&f[0], ProtocolVersion::V47).unwrap(),
        ClientboundLogin::Disconnect("\"Failed to verify username!\"".into())
    );
}

#[tokio::test]
async fn online_login_reports_unavailable_session_server() {
    let f = online_login(shared_with(true, "http://127.0.0.1:1/hasJoined", 20)).await;
    assert_eq!(
        ClientboundLogin::decode(&f[0], ProtocolVersion::V47).unwrap(),
        ClientboundLogin::Disconnect(
            "\"Authentication servers are down. Please try again later, sorry!\"".into()
        )
    );
}

#[tokio::test]
async fn wrong_verify_token_kills_connection() {
    let shared = shared_with(true, "http://127.0.0.1:1/", 20);
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let task = tokio::spawn(handle_connection(server, peer(), shared.clone()));
    let mut input = handshake(47, 2);
    input.extend(login_start(ProtocolVersion::V47, "Notch"));
    let response = ServerboundLogin::EncryptionResponse {
        shared_secret: shared.config.keys.encrypt(&[1; 16]).unwrap(),
        verify_token: shared.config.keys.encrypt(&[9, 9, 9, 9]).unwrap(),
    };
    input.extend(frame(response.encode(ProtocolVersion::V47).unwrap()));
    client.write_all(&input).await.unwrap();
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(ConnectionError::IllegalState("Invalid nonce!"))
    ));
}
