//! The legacy (pre-1.7) server list ping (vanilla: `LegacyPingHandler`).
//!
//! Vanilla inspects only the **first chunk** of bytes received on a connection. If it
//! starts with `0xFE` and matches one of three legacy formats, the server answers with a
//! `0xFF` kick packet carrying the status string and closes the connection. Anything
//! else — including malformed or truncated legacy pings — falls through to the modern
//! protocol with the chunk untouched.

use crate::buf::PacketReader;

/// The legacy protocol number vanilla reports in 1.4+ style responses.
pub const LEGACY_PROTOCOL: i32 = 127;

/// A recognized legacy ping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyPing {
    /// Beta 1.8 – 1.3: the single byte `FE`.
    Beta,
    /// 1.4 – 1.5: `FE 01`.
    V1_4,
    /// 1.6: `FE 01 FA` followed by an `MC|PingHost` plugin message.
    V1_6 {
        /// Client protocol number (at least 73).
        protocol: u8,
        /// Host name the client connected to.
        host: String,
        /// Port the client connected to.
        port: i32,
    },
}

/// Classifies the first chunk received on a connection.
///
/// Returns `None` when vanilla would hand the chunk to the modern protocol.
///
/// ```
/// use krol_protocol::legacy::{LegacyPing, classify};
/// assert_eq!(classify(&[0xfe]), Some(LegacyPing::Beta));
/// assert_eq!(classify(&[0xfe, 0x01]), Some(LegacyPing::V1_4));
/// assert_eq!(classify(&[0xfe, 0x02]), None);
/// assert_eq!(classify(&[0x10, 0x00]), None);
/// ```
pub fn classify(chunk: &[u8]) -> Option<LegacyPing> {
    let (&first, rest) = chunk.split_first()?;
    if first != 0xfe {
        return None;
    }
    match rest.len() {
        0 => Some(LegacyPing::Beta),
        1 => (rest[0] == 1).then_some(LegacyPing::V1_4),
        _ => classify_1_6(rest),
    }
}

fn classify_1_6(data: &[u8]) -> Option<LegacyPing> {
    // Vanilla evaluates every check (reading all fields) and only then decides; any
    // short read aborts to the modern pipeline. Both behave the same for us.
    let mut r = PacketReader::new(data);
    let mut ok = r.u8().ok()? == 1;
    ok &= r.u8().ok()? == 0xfa;
    ok &= read_utf16(&mut r)? == "MC|PingHost";
    let declared = usize::from(r.u16().ok()?);
    let protocol = r.u8().ok()?;
    ok &= protocol >= 73;
    let host_len_bytes = usize::try_from(r.i16().ok()?).ok()? * 2;
    let host_bytes = r.bytes(host_len_bytes).ok()?;
    ok &= 3 + host_bytes.len() + 4 == declared;
    let port = r.i32().ok()?;
    ok &= port <= 65535;
    ok &= r.remaining() == 0;
    ok.then(|| LegacyPing::V1_6 {
        protocol,
        host: decode_utf16be(host_bytes),
        port,
    })
}

fn read_utf16(r: &mut PacketReader<'_>) -> Option<String> {
    let chars = usize::try_from(r.i16().ok()?).ok()?;
    Some(decode_utf16be(r.bytes(chars * 2).ok()?))
}

fn decode_utf16be(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Server information included in a legacy ping response.
#[derive(Debug, Clone, Copy)]
pub struct LegacyStatus<'a> {
    /// The MOTD exactly as configured (`motd` in `server.properties`).
    pub motd: &'a str,
    /// Version name, e.g. `1.8.8`.
    pub version_name: &'a str,
    /// Current player count.
    pub online: i32,
    /// Maximum player count.
    pub max: i32,
}

/// Builds the complete response bytes (a `0xFF` kick packet) for `ping`. The server
/// closes the connection after sending it.
///
/// ```
/// use krol_protocol::legacy::{LegacyPing, LegacyStatus, response};
/// let status = LegacyStatus { motd: "Hi", version_name: "1.8.8", online: 1, max: 20 };
/// let bytes = response(&LegacyPing::Beta, &status);
/// assert_eq!(bytes[0], 0xff);
/// ```
pub fn response(ping: &LegacyPing, status: &LegacyStatus<'_>) -> Vec<u8> {
    let text = match ping {
        LegacyPing::Beta => format!("{}\u{a7}{}\u{a7}{}", status.motd, status.online, status.max),
        LegacyPing::V1_4 | LegacyPing::V1_6 { .. } => format!(
            "\u{a7}1\0{}\0{}\0{}\0{}\0{}",
            LEGACY_PROTOCOL, status.version_name, status.motd, status.online, status.max
        ),
    };
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut out = Vec::with_capacity(3 + units.len() * 2);
    out.push(0xff);
    // Vanilla quirk: writeShort of the char count silently wraps for huge MOTDs.
    out.extend_from_slice(&(units.len() as u16).to_be_bytes());
    for u in units {
        out.extend_from_slice(&u.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_be_bytes).collect()
    }

    fn ping_1_6(host: &str, port: i32, protocol: u8, fudge_len: i32) -> Vec<u8> {
        let mut v = vec![0xfe, 0x01, 0xfa];
        v.extend_from_slice(&11i16.to_be_bytes());
        v.extend(utf16("MC|PingHost"));
        let declared = (7 + host.encode_utf16().count() * 2) as i32 + fudge_len;
        v.extend_from_slice(&(declared as u16).to_be_bytes());
        v.push(protocol);
        v.extend_from_slice(&(host.encode_utf16().count() as i16).to_be_bytes());
        v.extend(utf16(host));
        v.extend_from_slice(&port.to_be_bytes());
        v
    }

    #[test]
    fn classifies_1_6() {
        assert_eq!(
            classify(&ping_1_6("localhost", 25565, 78, 0)),
            Some(LegacyPing::V1_6 {
                protocol: 78,
                host: "localhost".into(),
                port: 25565
            })
        );
        // Any failed check falls through to the modern protocol.
        assert_eq!(classify(&ping_1_6("localhost", 25565, 72, 0)), None);
        assert_eq!(classify(&ping_1_6("localhost", 25565, 78, 1)), None);
        assert_eq!(classify(&ping_1_6("localhost", 65536, 78, 0)), None);
        let mut extra = ping_1_6("h", 1, 78, 0);
        extra.push(0);
        assert_eq!(classify(&extra), None);
        let full = ping_1_6("localhost", 25565, 78, 0);
        assert_eq!(classify(&full[..full.len() - 1]), None, "truncated");
    }

    #[test]
    fn response_formats() {
        let status = LegacyStatus {
            motd: "A Minecraft Server",
            version_name: "1.8.8",
            online: 0,
            max: 20,
        };
        let beta = response(&LegacyPing::Beta, &status);
        let text = "A Minecraft Server\u{a7}0\u{a7}20";
        assert_eq!(
            &beta[1..3],
            &(text.encode_utf16().count() as u16).to_be_bytes()
        );
        assert_eq!(&beta[3..], utf16(text).as_slice());

        let modern = response(&LegacyPing::V1_4, &status);
        assert_eq!(
            &modern[3..],
            utf16("\u{a7}1\u{0}127\u{0}1.8.8\u{0}A Minecraft Server\u{0}0\u{0}20").as_slice()
        );
    }
}
