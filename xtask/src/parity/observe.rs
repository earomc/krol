//! Running scenario steps and normalizing what was observed.

use std::path::Path;

use anyhow::Result;
use krol_protocol::ProtocolVersion;
use krol_protocol::packets::login::ClientboundLogin;
use serde_json::{Map, Value, json};

use super::scenario::{Observe, Scenario, Step};
use super::server::{RunningServer, Target};

/// Runs `scenario` against `target` in `dir`, returning normalized observations.
pub fn run(scenario: &Scenario, target: &Target, dir: &Path, parity_root: &Path) -> Result<Value> {
    let port = super::server::prepare_dir(dir, &scenario.setup, parity_root)?;
    let server = RunningServer::start(target, dir, port)?;
    let mut obs = Map::new();
    obs.insert("started".into(), Value::Bool(server.started));

    if server.started {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        for step in &scenario.steps {
            if let Step::Wait { ms } = step {
                std::thread::sleep(std::time::Duration::from_millis(*ms));
                continue;
            }
            let value = runtime.block_on(run_step(step, server.addr));
            if let Some(id) = step.id() {
                obs.insert(id.to_owned(), value);
            }
        }
    }

    let log = server.stop()?;
    if scenario.observe.log {
        obs.insert("log".into(), normalize_log(&log, &scenario.observe));
    }
    for file in &scenario.observe.files {
        let value = match std::fs::read(dir.join(file)) {
            Ok(bytes) => normalize_file(file, &String::from_utf8_lossy(&bytes)),
            Err(_) => Value::Null,
        };
        obs.insert(format!("file:{file}"), value);
    }
    Ok(Value::Object(obs))
}

async fn run_step(step: &Step, addr: std::net::SocketAddr) -> Value {
    match step {
        Step::StatusPing {
            protocol, payload, ..
        } => match krol_testbot::status_ping(addr, *protocol, *payload).await {
            Ok(o) => json!({
                "raw": o.json,
                "parsed": serde_json::from_str::<Value>(&o.json).unwrap_or(Value::Null),
                "pong": o.pong,
                "closed_after_pong": o.closed_after_pong,
            }),
            Err(e) => json!({ "error": e.to_string() }),
        },
        Step::Raw { hex, .. } => {
            let bytes = decode_hex(hex);
            match krol_testbot::raw_exchange(addr, &bytes).await {
                Ok(out) => {
                    let mut v = json!({ "hex": encode_hex(&out) });
                    if out.first() == Some(&0xff) && out.len() >= 3 {
                        let units: Vec<u16> = out[3..]
                            .chunks_exact(2)
                            .map(|c| u16::from_be_bytes([c[0], c[1]]))
                            .collect();
                        v["kick_text"] = Value::String(String::from_utf16_lossy(&units));
                    }
                    v
                }
                Err(e) => json!({ "error": e.to_string() }),
            }
        }
        Step::Wait { .. } => Value::Null,
        Step::LoginStart { protocol, name, .. } => {
            match krol_testbot::login_start(addr, *protocol, name).await {
                Ok(packets) => Value::Array(packets.iter().map(login_packet_json).collect()),
                Err(e) => json!({ "error": e.to_string() }),
            }
        }
    }
}

fn login_packet_json(p: &ClientboundLogin) -> Value {
    match p {
        ClientboundLogin::Disconnect(reason) => json!({ "type": "disconnect", "reason": reason }),
        ClientboundLogin::EncryptionRequest {
            server_id,
            public_key,
            verify_token,
        } => json!({
            "type": "encryption_request",
            "server_id": server_id,
            "public_key_len": public_key.len(),
            "verify_token_len": verify_token.len(),
        }),
        ClientboundLogin::Success { uuid, username } => {
            json!({ "type": "success", "uuid": uuid, "username": username })
        }
        ClientboundLogin::SetCompression(t) => json!({ "type": "set_compression", "threshold": t }),
    }
}

/// Strips timestamps and run-specific values from console lines.
fn normalize_log(lines: &[String], observe: &Observe) -> Value {
    let out: Vec<Value> = lines
        .iter()
        .filter_map(|line| {
            // JVM stack frames are implementation details of vanilla, not behavior.
            if line.starts_with("	at ") || (line.starts_with("	... ") && line.ends_with(" more")) {
                return None;
            }
            // "[HH:MM:SS] [thread/LEVEL]: msg" -> "[thread/LEVEL]: msg"
            let line = match line.strip_prefix('[') {
                Some(rest) if rest.len() > 9 && rest.as_bytes()[8] == b']' => {
                    rest[9..].trim_start()
                }
                _ => line.as_str(),
            };
            let message = line.split_once("]: ").map_or(line, |(_, m)| m);
            if observe
                .log_ignore_prefixes
                .iter()
                .any(|p| message.starts_with(p.as_str()))
            {
                return None;
            }
            Some(Value::String(normalize_text(line)))
        })
        .collect();
    Value::Array(out)
}

fn normalize_text(line: &str) -> String {
    let mut s = line.to_owned();
    if let Some(start) = s.find("Done (")
        && let Some(end) = s[start..].find(")!")
    {
        s.replace_range(start + 6..start + end, "<time>");
    }
    if let Some(idx) = s.find("Starting Minecraft server on ")
        && let Some(colon) = s[idx..].rfind(':')
    {
        s.replace_range(idx + colon + 1.., "<port>");
    }
    s
}

fn normalize_file(name: &str, text: &str) -> Value {
    let is_properties = name.ends_with(".properties") || name.ends_with("eula.txt");
    let lines: Vec<Value> = text
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let line = if is_properties && i == 1 && line.starts_with('#') {
                "#<date>".to_owned()
            } else if line.starts_with("server-port=") {
                "server-port=<port>".to_owned()
            } else {
                line.to_owned()
            };
            Value::String(line)
        })
        .collect();
    Value::Array(lines)
}

fn decode_hex(hex: &str) -> Vec<u8> {
    let clean: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
    (0..clean.len() / 2)
        .filter_map(|i| u8::from_str_radix(&clean[2 * i..2 * i + 2], 16).ok())
        .collect()
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Protocol name helper for reports.
#[allow(dead_code)]
pub fn protocol_label(protocol: i32) -> String {
    match ProtocolVersion::from_number(protocol) {
        Some(v) => format!("{} ({})", v.number(), v.version_name()),
        None => protocol.to_string(),
    }
}
