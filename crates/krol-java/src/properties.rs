//! A port of Java 8 `java.util.Properties` file handling.
//!
//! Vanilla reads and writes `server.properties` with `Properties.load(InputStream)` and
//! `Properties.store(OutputStream, comment)`. Reproducing it exactly matters for drop-in
//! compatibility:
//!
//! - files are decoded as **ISO-8859-1** (vanilla quirk: UTF-8 text in a MOTD turns into
//!   mojibake) and written as ASCII with `\uXXXX` escapes,
//! - `load` supports `=`, `:` and whitespace separators, `#`/`!` comments, line
//!   continuations and backslash escapes,
//! - `store` writes a comment line, a date line, then every entry in `Hashtable` order
//!   (see [`JavaHashtable`]).

use crate::JavaHashtable;
use thiserror::Error;

/// Error from [`JavaProperties::load`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PropertiesError {
    /// A `\u` escape not followed by four hex digits (Java: `IllegalArgumentException`).
    #[error("Malformed \\uxxxx encoding.")]
    MalformedUnicodeEscape,
}

/// A Java 8 `Properties` table.
///
/// ```
/// use krol_java::properties::JavaProperties;
///
/// let mut p = JavaProperties::load(b"motd = Hello\\\n   World\n# comment\nport:25565").unwrap();
/// assert_eq!(p.get("motd"), Some("HelloWorld"));
/// assert_eq!(p.get("port"), Some("25565"));
/// p.set("pvp", "true");
/// let out = String::from_utf8(p.store("Minecraft server properties", "Thu Jan 01 00:00:00 UTC 1970", "\n")).unwrap();
/// assert!(out.starts_with("#Minecraft server properties\n#Thu Jan 01 00:00:00 UTC 1970\n"));
/// ```
#[derive(Debug, Clone, Default)]
pub struct JavaProperties {
    table: JavaHashtable<String>,
}

impl JavaProperties {
    /// Creates an empty table (like `new Properties()`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses `bytes` like `Properties.load(InputStream)`, inserting entries in file order.
    ///
    /// # Errors
    ///
    /// [`PropertiesError::MalformedUnicodeEscape`] for a bad `\u` escape.
    pub fn load(bytes: &[u8]) -> Result<Self, PropertiesError> {
        let mut props = Self::new();
        props.load_into(bytes)?;
        Ok(props)
    }

    /// Parses `bytes` into this table, overwriting existing keys.
    ///
    /// # Errors
    ///
    /// [`PropertiesError::MalformedUnicodeEscape`] for a bad `\u` escape.
    pub fn load_into(&mut self, bytes: &[u8]) -> Result<(), PropertiesError> {
        // ISO-8859-1: every byte is one char.
        let chars: Vec<char> = bytes.iter().map(|&b| char::from(b)).collect();
        let mut reader = LineReader {
            chars: &chars,
            pos: 0,
        };
        while let Some(line) = reader.read_line() {
            let (key, value) = split_key_value(&line);
            self.table.insert(load_convert(key)?, load_convert(value)?);
        }
        Ok(())
    }

    /// The value for `key` (`getProperty`).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.table.get(key).map(String::as_str)
    }

    /// Sets `key` to `value` (`setProperty`).
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.table.insert(key.into(), value.into());
    }

    /// Removes `key`.
    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.table.remove(key)
    }

    /// Iterates entries in `Hashtable` order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.table.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Serializes like `Properties.store(OutputStream, comment)`.
    ///
    /// `date` is the text of the timestamp line (Java writes `new Date().toString()`), and
    /// `line_separator` is the platform separator Java's `BufferedWriter.newLine` uses
    /// (`"\r\n"` on Windows, `"\n"` elsewhere). `comment` must be printable ASCII.
    pub fn store(&self, comment: &str, date: &str, line_separator: &str) -> Vec<u8> {
        let mut out = String::new();
        out.push('#');
        out.push_str(comment);
        out.push_str(line_separator);
        out.push('#');
        out.push_str(date);
        out.push_str(line_separator);
        for (key, value) in self.iter() {
            save_convert(&mut out, key, true);
            out.push('=');
            save_convert(&mut out, value, false);
            out.push_str(line_separator);
        }
        out.into_bytes()
    }
}

/// The platform line separator Java would use (`System.lineSeparator()`).
pub fn platform_line_separator() -> &'static str {
    if cfg!(windows) { "\r\n" } else { "\n" }
}

struct LineReader<'a> {
    chars: &'a [char],
    pos: usize,
}

impl LineReader<'_> {
    /// Returns the next logical line (continuations joined), mirroring Java 8's
    /// `Properties.LineReader.readLine`.
    fn read_line(&mut self) -> Option<Vec<char>> {
        let mut line = Vec::new();
        let mut skip_white_space = true;
        let mut is_comment_line = false;
        let mut is_new_line = true;
        let mut appended_line_begin = false;
        let mut preceding_backslash = false;
        let mut skip_lf = false;

        loop {
            let Some(&c) = self.chars.get(self.pos) else {
                if line.is_empty() || is_comment_line {
                    return None;
                }
                if preceding_backslash {
                    line.pop();
                }
                return Some(line);
            };
            self.pos += 1;

            if skip_lf {
                skip_lf = false;
                if c == '\n' {
                    continue;
                }
            }
            if skip_white_space {
                if matches!(c, ' ' | '\t' | '\u{c}') {
                    continue;
                }
                if !appended_line_begin && matches!(c, '\r' | '\n') {
                    continue;
                }
                skip_white_space = false;
                appended_line_begin = false;
            }
            if is_new_line {
                is_new_line = false;
                if c == '#' || c == '!' {
                    is_comment_line = true;
                    continue;
                }
            }
            if c != '\n' && c != '\r' {
                line.push(c);
                preceding_backslash = if c == '\\' {
                    !preceding_backslash
                } else {
                    false
                };
                continue;
            }
            // End of a natural line.
            if is_comment_line || line.is_empty() {
                is_comment_line = false;
                is_new_line = true;
                skip_white_space = true;
                line.clear();
                continue;
            }
            if self.pos >= self.chars.len() {
                if preceding_backslash {
                    line.pop();
                }
                return Some(line);
            }
            if preceding_backslash {
                line.pop();
                skip_white_space = true;
                appended_line_begin = true;
                preceding_backslash = false;
                if c == '\r' {
                    skip_lf = true;
                }
            } else {
                return Some(line);
            }
        }
    }
}

fn split_key_value(line: &[char]) -> (&[char], &[char]) {
    let limit = line.len();
    let mut key_len = 0;
    let mut value_start = limit;
    let mut has_sep = false;
    let mut preceding_backslash = false;
    while key_len < limit {
        let c = line[key_len];
        if (c == '=' || c == ':') && !preceding_backslash {
            value_start = key_len + 1;
            has_sep = true;
            break;
        } else if matches!(c, ' ' | '\t' | '\u{c}') && !preceding_backslash {
            value_start = key_len + 1;
            break;
        }
        preceding_backslash = if c == '\\' {
            !preceding_backslash
        } else {
            false
        };
        key_len += 1;
    }
    while value_start < limit {
        let c = line[value_start];
        if !matches!(c, ' ' | '\t' | '\u{c}') {
            if !has_sep && (c == '=' || c == ':') {
                has_sep = true;
            } else {
                break;
            }
        }
        value_start += 1;
    }
    (&line[..key_len], &line[value_start..])
}

fn load_convert(input: &[char]) -> Result<String, PropertiesError> {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let c = input[i];
        i += 1;
        if c != '\\' {
            out.push(c);
            continue;
        }
        // A trailing lone backslash is dropped (Java reads past it into stale buffer
        // contents; in practice continuation handling already removed it).
        let Some(&next) = input.get(i) else { break };
        i += 1;
        match next {
            'u' => {
                let mut value: u32 = 0;
                for _ in 0..4 {
                    let d = input
                        .get(i)
                        .and_then(|h| h.to_digit(16))
                        .ok_or(PropertiesError::MalformedUnicodeEscape)?;
                    value = (value << 4) | d;
                    i += 1;
                }
                // Lone surrogates cannot live in a Rust string; Java would keep them.
                out.push(char::from_u32(value).unwrap_or('\u{fffd}'));
            }
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'n' => out.push('\n'),
            'f' => out.push('\u{c}'),
            other => out.push(other),
        }
    }
    Ok(out)
}

fn save_convert(out: &mut String, s: &str, escape_space: bool) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for (x, unit) in s.encode_utf16().enumerate() {
        let c = unit;
        if c > 61 && c < 127 {
            if c == u16::from(b'\\') {
                out.push_str("\\\\");
            } else {
                out.push(char::from(c as u8));
            }
            continue;
        }
        match c {
            0x20 => {
                if x == 0 || escape_space {
                    out.push('\\');
                }
                out.push(' ');
            }
            0x09 => out.push_str("\\t"),
            0x0a => out.push_str("\\n"),
            0x0d => out.push_str("\\r"),
            0x0c => out.push_str("\\f"),
            0x3d | 0x3a | 0x23 | 0x21 => {
                out.push('\\');
                out.push(char::from(c as u8));
            }
            _ if !(0x20..=0x7e).contains(&c) => {
                out.push_str("\\u");
                for shift in [12, 8, 4, 0] {
                    out.push(char::from(HEX[usize::from((c >> shift) & 0xf)]));
                }
            }
            _ => out.push(char::from(c as u8)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin1_decoding() {
        // UTF-8 "é" is read as two Latin-1 chars, like vanilla.
        let p = JavaProperties::load("motd=\u{e9}".as_bytes()).unwrap();
        assert_eq!(p.get("motd"), Some("\u{c3}\u{a9}"));
    }

    #[test]
    fn store_escaping() {
        let mut p = JavaProperties::new();
        p.set(" k=:#!", " v=:#! \t\u{e9}\u{263a}\\");
        let out = String::from_utf8(p.store("c", "d", "\n")).unwrap();
        assert_eq!(
            out,
            "#c\n#d\n\\ k\\=\\:\\#\\!=\\ v\\=\\:\\#\\! \\t\\u00E9\\u263A\\\\\n"
        );
    }

    #[test]
    fn malformed_escape() {
        assert_eq!(
            JavaProperties::load(b"a=\\u12g4").unwrap_err(),
            PropertiesError::MalformedUnicodeEscape
        );
    }
}
