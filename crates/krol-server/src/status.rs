//! The server list status response (vanilla: `ServerStatusResponse`).

use std::path::Path;

use krol_java::gson::JsonValue;
use krol_protocol::ProtocolVersion;
use tracing::error;

/// Data shown in the multiplayer server list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusResponse {
    /// The MOTD (`motd` property), shown as a plain text component.
    pub motd: String,
    /// Maximum players.
    pub max_players: i32,
    /// Online players. Vanilla refreshes this (and the sample) only every 5 seconds.
    pub online_players: i32,
    /// Up to 12 sampled players as `(uuid, name)`.
    pub sample: Vec<(u128, String)>,
    /// `data:image/png;base64,...` favicon, if `server-icon.png` is valid.
    pub favicon: Option<String>,
}

impl StatusResponse {
    /// Serializes the response for a client speaking `protocol`, like vanilla's Gson
    /// serializer: `description`, `players` (`max`, `online`, `sample` if non-empty),
    /// `version` (`name`, `protocol`), then `favicon`.
    ///
    /// The plain-text description is serialized as a bare JSON string, as vanilla's chat
    /// serializer does for unstyled text components.
    ///
    /// Deliberate deviation: vanilla always reports protocol 47 / "1.8.8". krol reports
    /// the client's own supported protocol so 1.7 clients see a compatible server.
    pub fn to_json(&self, protocol: ProtocolVersion) -> String {
        let mut players = JsonValue::object()
            .with("max", self.max_players)
            .with("online", self.online_players);
        if !self.sample.is_empty() {
            let sample = self
                .sample
                .iter()
                .map(|(id, name)| {
                    JsonValue::object()
                        .with("id", java_uuid_string(*id))
                        .with("name", name.as_str())
                })
                .collect::<Vec<_>>();
            players = players.with("sample", sample);
        }
        let mut root = JsonValue::object()
            .with("description", self.motd.as_str())
            .with("players", players)
            .with(
                "version",
                JsonValue::object()
                    .with("name", protocol.version_name())
                    .with("protocol", protocol.number()),
            );
        if let Some(favicon) = &self.favicon {
            root = root.with("favicon", favicon.as_str());
        }
        root.to_gson_string()
    }
}

/// Formats a UUID like Java's `UUID.toString()`.
pub fn java_uuid_string(id: u128) -> String {
    let h = format!("{id:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// Loads `server-icon.png` like vanilla: it must be a 64×64 PNG; otherwise an error is
/// logged and no favicon is sent. A missing file is silently ignored.
///
/// Known deviation: vanilla decodes and re-encodes the image with `ImageIO`, so its PNG
/// bytes differ from the file; krol sends the file as is. Clients render both the same.
pub fn load_favicon(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let result = std::fs::read(path)
        .map_err(|e| format!("java.io.IOException: {e}"))
        .and_then(|bytes| check_png_64(&bytes).map(|()| bytes));
    match result {
        Ok(bytes) => Some(format!("data:image/png;base64,{}", netty_base64(&bytes))),
        Err(reason) => {
            error!("Couldn't load server icon\n{reason}");
            None
        }
    }
}

fn check_png_64(bytes: &[u8]) -> Result<(), String> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return Err("java.lang.IllegalArgumentException: image == null!".into());
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if width != 64 {
        return Err("java.lang.IllegalStateException: Must be 64 pixels wide".into());
    }
    if height != 64 {
        return Err("java.lang.IllegalStateException: Must be 64 pixels high".into());
    }
    Ok(())
}

/// Base64 exactly like Netty's `Base64.encode(ByteBuf)`: standard alphabet with padding,
/// and — vanilla quirk — a `\n` after every 76 output characters (no trailing newline).
pub fn netty_base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len() * 4 / 3 + data.len() / 57 + 4);
    let mut line_len = 0;
    let full = data.len() / 3 * 3;
    for chunk in data[..full].chunks_exact(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        for shift in [18, 12, 6, 0] {
            out.push(char::from(ALPHABET[((n >> shift) & 63) as usize]));
        }
        line_len += 4;
        if line_len == 76 {
            out.push('\n');
            line_len = 0;
        }
    }
    let rest = &data[full..];
    if !rest.is_empty() {
        let b1 = u32::from(rest[0]);
        let b2 = rest.get(1).map_or(0, |&b| u32::from(b));
        let n = (b1 << 16) | (b2 << 8);
        out.push(char::from(ALPHABET[((n >> 18) & 63) as usize]));
        out.push(char::from(ALPHABET[((n >> 12) & 63) as usize]));
        out.push(if rest.len() == 2 {
            char::from(ALPHABET[((n >> 6) & 63) as usize])
        } else {
            '='
        });
        out.push('=');
    }
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_layout() {
        let mut s = StatusResponse {
            motd: "A Minecraft Server".into(),
            max_players: 20,
            online_players: 0,
            sample: vec![],
            favicon: None,
        };
        assert_eq!(
            s.to_json(ProtocolVersion::V47),
            r#"{"description":"A Minecraft Server","players":{"max":20,"online":0},"version":{"name":"1.8.8","protocol":47}}"#
        );
        s.sample = vec![(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef, "Steve".into())];
        s.favicon = Some("data:x".into());
        s.motd = "a=b".into();
        assert_eq!(
            s.to_json(ProtocolVersion::V5),
            "{\"description\":\"a\\u003db\",\"players\":{\"max\":20,\"online\":0,\"sample\":[{\"id\":\"01234567-89ab-cdef-0123-456789abcdef\",\"name\":\"Steve\"}]},\"version\":{\"name\":\"1.7.10\",\"protocol\":5},\"favicon\":\"data:x\"}"
        );
    }

    #[test]
    fn base64_matches_netty() {
        assert_eq!(netty_base64(b""), "");
        assert_eq!(netty_base64(b"f"), "Zg==");
        assert_eq!(netty_base64(b"fo"), "Zm8=");
        assert_eq!(netty_base64(b"foo"), "Zm9v");
        // 57 input bytes = exactly 76 output chars: no trailing newline.
        let line = netty_base64(&[0u8; 57]);
        assert_eq!(line.len(), 76);
        assert!(!line.contains('\n'));
        let two = netty_base64(&[0u8; 58]);
        assert_eq!(two.find('\n'), Some(76));
        assert_eq!(two.len(), 76 + 1 + 4);
    }

    #[test]
    fn favicon_validation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server-icon.png");
        assert_eq!(load_favicon(&path), None);
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&64u32.to_be_bytes());
        png.extend_from_slice(&64u32.to_be_bytes());
        std::fs::write(&path, &png).unwrap();
        assert!(
            load_favicon(&path)
                .unwrap()
                .starts_with("data:image/png;base64,iVBORw0KGgo")
        );
        png[19] = 32; // width 32
        std::fs::write(&path, &png).unwrap();
        assert_eq!(load_favicon(&path), None);
    }
}
