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
    let mut server = RunningServer::start(target, dir, port)?;
    let mut obs = Map::new();
    obs.insert("started".into(), Value::Bool(server.started));

    if server.started {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let mut background = Vec::new();
        for step in &scenario.steps {
            if let Step::Play {
                background: true,
                id,
                ..
            } = step
            {
                let step = step.clone();
                let addr = server.addr;
                background.push((
                    id.clone(),
                    runtime.spawn(async move { run_step(&step, addr).await }),
                ));
                // Give the join a moment so following steps see the player online.
                std::thread::sleep(std::time::Duration::from_millis(1500));
                continue;
            }
            if let Step::Wait { ms } = step {
                std::thread::sleep(std::time::Duration::from_millis(*ms));
                continue;
            }
            if let Step::Console { command, wait_ms } = step {
                server.console(command);
                std::thread::sleep(std::time::Duration::from_millis(*wait_ms));
                continue;
            }
            let value = runtime.block_on(run_step(step, server.addr));
            if let Some(id) = step.id() {
                obs.insert(id.to_owned(), value);
            }
        }
        for (id, handle) in background {
            let value = runtime
                .block_on(handle)
                .unwrap_or_else(|e| json!({ "error": e.to_string() }));
            obs.insert(id, value);
        }
    }

    let log = server.stop()?;
    if scenario.observe.log {
        obs.insert("log".into(), normalize_log(&log, &scenario.observe));
    }
    for file in &scenario.observe.files {
        let value = match std::fs::read(dir.join(file)) {
            Ok(bytes) if file.ends_with(".dat") => nbt_file_json(&bytes),
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
        Step::Wait { .. } | Step::Console { .. } => Value::Null,
        Step::Play {
            protocol,
            name,
            seconds,
            chat,
            moves,
            ..
        } => match krol_testbot::play_session(
            addr,
            *protocol,
            name,
            std::time::Duration::from_secs(*seconds),
            chat,
            moves,
        )
        .await
        {
            Ok(o) => super::play::summarize(&o),
            Err(e) => json!({ "error": e.to_string() }),
        },
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
    // Vanilla's JVM shutdown hook runs a second stopServer() while log4j shuts down,
    // which nondeterministically adds lines and log4j errors after this one. That is a
    // teardown race, not server behavior, so the log is cut here.
    let end = lines
        .iter()
        .position(|l| l.contains("[Server Shutdown Thread/INFO]: Stopping server"))
        .map_or(lines.len(), |i| i + 1);
    let out: Vec<Value> = lines[..end]
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

/// Replaces the port of every `/<ip>:<port>` (client socket addresses) with `<port>`.
fn normalize_socket_ports(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(slash) = rest.find('/') {
        out.push_str(&rest[..=slash]);
        rest = &rest[slash + 1..];
        let ip_len = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        if ip_len > 0 && rest[ip_len..].starts_with(':') {
            let digits = rest[ip_len + 1..]
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len() - ip_len - 1);
            if digits > 0 {
                out.push_str(&rest[..ip_len]);
                out.push_str(":<port>");
                rest = &rest[ip_len + 1 + digits..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn normalize_text(line: &str) -> String {
    let mut s = normalize_socket_ports(line);
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

/// Replaces `yyyy-MM-dd HH:mm:ss +zzzz` timestamps (ban/usercache dates) with `<date>`.
fn normalize_dates(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let digit = |j: usize, n: usize| j + n <= b.len() && b[j..j + n].iter().all(u8::is_ascii_digit);
    while i < b.len() {
        let is_date = digit(i, 4)
            && b.get(i + 4) == Some(&b'-')
            && digit(i + 5, 2)
            && b.get(i + 7) == Some(&b'-')
            && digit(i + 8, 2)
            && b.get(i + 10) == Some(&b' ')
            && digit(i + 11, 2)
            && b.get(i + 13) == Some(&b':')
            && digit(i + 14, 2)
            && b.get(i + 16) == Some(&b':')
            && digit(i + 17, 2)
            && b.get(i + 19) == Some(&b' ')
            && matches!(b.get(i + 20), Some(b'+' | b'-'))
            && digit(i + 21, 4);
        if is_date {
            out.push_str("<date>");
            i += 25;
        } else {
            let ch = s[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn normalize_file(name: &str, text: &str) -> Value {
    if name.ends_with(".json") {
        return Value::String(normalize_dates(text));
    }
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

/// A gzip NBT file as JSON with sorted compound keys (vanilla writes compounds in
/// `HashMap` order, which carries no meaning). Values are tagged with their NBT type.
fn nbt_file_json(bytes: &[u8]) -> Value {
    match krol_nbt::read_gzip(bytes) {
        Ok((_, root)) => compound_json(&root),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

fn compound_json(c: &krol_nbt::Compound) -> Value {
    let mut entries: Vec<(&String, &krol_nbt::Tag)> = c.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut map = serde_json::Map::new();
    for (k, v) in entries {
        map.insert(k.clone(), tag_json(v));
    }
    Value::Object(map)
}

fn tag_json(t: &krol_nbt::Tag) -> Value {
    use krol_nbt::Tag;
    match t {
        Tag::Byte(v) => json!(format!("{v}b")),
        Tag::Short(v) => json!(format!("{v}s")),
        Tag::Int(v) => json!(v),
        Tag::Long(v) => json!(format!("{v}L")),
        Tag::Float(v) => json!(format!("{v}f")),
        Tag::Double(v) => json!(format!("{v}d")),
        Tag::ByteArray(a) => json!(format!("<{} bytes>", a.len())),
        Tag::String(s) => json!(s),
        Tag::List(l) => Value::Array(l.iter().map(tag_json).collect()),
        Tag::Compound(c) => compound_json(c),
        Tag::IntArray(a) => json!(a),
    }
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
