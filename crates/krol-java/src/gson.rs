//! JSON serialization byte-compatible with Gson's default output.
//!
//! Vanilla serializes status responses and chat components with a default-configured
//! Gson instance. Its output is compact (no whitespace), keeps object members in the order
//! they were added, and is **HTML-safe**: besides the usual JSON escapes it writes `<`,
//! `>`, `&`, `=` and `'` as `<`, `>`, `&`, `=` and `'`. Clients
//! see these bytes, so krol reproduces them.
//!
//! Floating-point numbers are not supported yet: Gson formats them with Java's
//! `Double.toString`, which needs its own port.

use std::fmt::Write as _;

/// A JSON value with insertion-ordered objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonValue {
    /// `null`.
    Null,
    /// `true` / `false`.
    Bool(bool),
    /// An integral number.
    Int(i64),
    /// A string.
    String(String),
    /// An array.
    Array(Vec<JsonValue>),
    /// An object; members are serialized in this order.
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    /// Creates an empty object.
    pub fn object() -> Self {
        JsonValue::Object(Vec::new())
    }

    /// Appends a member to an object (builder style).
    ///
    /// # Panics
    ///
    /// Panics if `self` is not an object.
    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: impl Into<JsonValue>) -> Self {
        match &mut self {
            JsonValue::Object(members) => members.push((key.into(), value.into())),
            _ => panic!("JsonValue::with called on a non-object"),
        }
        self
    }

    /// Serializes like `new Gson().toJson(...)`.
    ///
    /// ```
    /// use krol_java::gson::JsonValue;
    ///
    /// let v = JsonValue::object()
    ///     .with("text", "a=b <c>")
    ///     .with("bold", true);
    /// assert_eq!(
    ///     v.to_gson_string(),
    ///     "{\"text\":\"a\\u003db \\u003cc\\u003e\",\"bold\":true}"
    /// );
    /// ```
    pub fn to_gson_string(&self) -> String {
        let mut out = String::new();
        self.write_to(&mut out);
        out
    }

    /// Serializes like a Gson instance with `setPrettyPrinting()`: two-space indentation,
    /// `": "` between keys and values, and `[]` / `{}` for empty containers.
    ///
    /// ```
    /// use krol_java::gson::JsonValue;
    ///
    /// let v = JsonValue::Array(vec![JsonValue::object().with("level", 4)]);
    /// assert_eq!(v.to_gson_pretty_string(), "[\n  {\n    \"level\": 4\n  }\n]");
    /// ```
    pub fn to_gson_pretty_string(&self) -> String {
        let mut out = String::new();
        self.write_pretty(&mut out, 0);
        out
    }

    fn write_pretty(&self, out: &mut String, depth: usize) {
        let newline = |out: &mut String, depth: usize| {
            out.push('\n');
            for _ in 0..depth {
                out.push_str("  ");
            }
        };
        match self {
            JsonValue::Array(items) if !items.is_empty() => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    newline(out, depth + 1);
                    item.write_pretty(out, depth + 1);
                }
                newline(out, depth);
                out.push(']');
            }
            JsonValue::Object(members) if !members.is_empty() => {
                out.push('{');
                for (i, (key, value)) in members.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    newline(out, depth + 1);
                    write_string(out, key);
                    out.push_str(": ");
                    value.write_pretty(out, depth + 1);
                }
                newline(out, depth);
                out.push('}');
            }
            other => other.write_to(out),
        }
    }

    /// Parses JSON text (lenient about whitespace; strict otherwise).
    ///
    /// Numbers with a fraction or exponent are rejected, as [`JsonValue`] has no float
    /// variant yet.
    ///
    /// # Errors
    ///
    /// A message describing the first syntax error.
    pub fn parse(text: &str) -> Result<JsonValue, String> {
        let mut p = Parser {
            chars: text.char_indices().peekable(),
            text,
        };
        let v = p.value()?;
        p.skip_ws();
        match p.chars.next() {
            None => Ok(v),
            Some((i, _)) => Err(format!("trailing data at {i}")),
        }
    }

    /// The member `key` of an object.
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The string value, if this is a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// The integer value, if this is an integer.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            JsonValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// The boolean value, if this is a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsonValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn write_to(&self, out: &mut String) {
        match self {
            JsonValue::Null => out.push_str("null"),
            JsonValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            JsonValue::Int(i) => {
                let _ = write!(out, "{i}");
            }
            JsonValue::String(s) => write_string(out, s),
            JsonValue::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_to(out);
                }
                out.push(']');
            }
            JsonValue::Object(members) => {
                out.push('{');
                for (i, (key, value)) in members.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(out, key);
                    out.push(':');
                    value.write_to(out);
                }
                out.push('}');
            }
        }
    }
}

/// Writes `s` as a Gson HTML-safe JSON string literal.
fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{c}' => out.push_str("\\f"),
            '\0'..='\u{1f}' | '<' | '>' | '&' | '=' | '\'' | '\u{2028}' | '\u{2029}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            _ => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    text: &'a str,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while self
            .chars
            .peek()
            .is_some_and(|(_, c)| matches!(c, ' ' | '\t' | '\n' | '\r'))
        {
            self.chars.next();
        }
    }

    fn expect(&mut self, want: char) -> Result<(), String> {
        match self.chars.next() {
            Some((_, c)) if c == want => Ok(()),
            Some((i, c)) => Err(format!("expected '{want}' at {i}, found '{c}'")),
            None => Err(format!("expected '{want}', found end of input")),
        }
    }

    fn value(&mut self) -> Result<JsonValue, String> {
        self.skip_ws();
        let Some(&(start, c)) = self.chars.peek() else {
            return Err("unexpected end of input".into());
        };
        match c {
            '{' => {
                self.chars.next();
                let mut members = Vec::new();
                self.skip_ws();
                if self.chars.peek().is_some_and(|(_, c)| *c == '}') {
                    self.chars.next();
                    return Ok(JsonValue::Object(members));
                }
                loop {
                    self.skip_ws();
                    let key = self.string()?;
                    self.skip_ws();
                    self.expect(':')?;
                    let value = self.value()?;
                    members.push((key, value));
                    self.skip_ws();
                    match self.chars.next() {
                        Some((_, ',')) => continue,
                        Some((_, '}')) => return Ok(JsonValue::Object(members)),
                        Some((i, c)) => return Err(format!("unexpected '{c}' at {i}")),
                        None => return Err("unterminated object".into()),
                    }
                }
            }
            '[' => {
                self.chars.next();
                let mut items = Vec::new();
                self.skip_ws();
                if self.chars.peek().is_some_and(|(_, c)| *c == ']') {
                    self.chars.next();
                    return Ok(JsonValue::Array(items));
                }
                loop {
                    items.push(self.value()?);
                    self.skip_ws();
                    match self.chars.next() {
                        Some((_, ',')) => continue,
                        Some((_, ']')) => return Ok(JsonValue::Array(items)),
                        Some((i, c)) => return Err(format!("unexpected '{c}' at {i}")),
                        None => return Err("unterminated array".into()),
                    }
                }
            }
            '"' => Ok(JsonValue::String(self.string()?)),
            't' | 'f' | 'n' => {
                let rest = &self.text[start..];
                let (word, value) = if rest.starts_with("true") {
                    ("true", JsonValue::Bool(true))
                } else if rest.starts_with("false") {
                    ("false", JsonValue::Bool(false))
                } else if rest.starts_with("null") {
                    ("null", JsonValue::Null)
                } else {
                    return Err(format!("unexpected literal at {start}"));
                };
                for _ in 0..word.len() {
                    self.chars.next();
                }
                Ok(value)
            }
            '-' | '0'..='9' => {
                let mut end = start;
                while let Some(&(i, c)) = self.chars.peek() {
                    if c == '-' || c.is_ascii_digit() {
                        end = i + 1;
                        self.chars.next();
                    } else if matches!(c, '.' | 'e' | 'E' | '+') {
                        return Err(format!("floating-point numbers are not supported (at {i})"));
                    } else {
                        break;
                    }
                }
                self.text[start..end]
                    .parse()
                    .map(JsonValue::Int)
                    .map_err(|e| format!("bad number at {start}: {e}"))
            }
            other => Err(format!("unexpected '{other}' at {start}")),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect('"')?;
        let mut out = String::new();
        let mut pending_high: Option<u16> = None;
        loop {
            let Some((i, c)) = self.chars.next() else {
                return Err("unterminated string".into());
            };
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let Some((_, esc)) = self.chars.next() else {
                        return Err("unterminated escape".into());
                    };
                    let ch = match esc {
                        '"' => '"',
                        '\\' => '\\',
                        '/' => '/',
                        'b' => '\u{8}',
                        'f' => '\u{c}',
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        'u' => {
                            let mut code = 0u16;
                            for _ in 0..4 {
                                let (_, h) = self.chars.next().ok_or("short \\u escape")?;
                                let d = h
                                    .to_digit(16)
                                    .ok_or_else(|| format!("bad \\u escape at {i}"))?;
                                code = (code << 4) | d as u16;
                            }
                            if (0xd800..0xdc00).contains(&code) {
                                pending_high = Some(code);
                                continue;
                            }
                            if let Some(high) = pending_high.take() {
                                let s = String::from_utf16(&[high, code])
                                    .unwrap_or_else(|_| "\u{fffd}".into());
                                out.push_str(&s);
                                continue;
                            }
                            char::from_u32(u32::from(code)).unwrap_or('\u{fffd}')
                        }
                        other => return Err(format!("bad escape '\\{other}' at {i}")),
                    };
                    out.push(ch);
                }
                c => out.push(c),
            }
        }
    }
}

impl From<bool> for JsonValue {
    fn from(v: bool) -> Self {
        JsonValue::Bool(v)
    }
}

impl From<i64> for JsonValue {
    fn from(v: i64) -> Self {
        JsonValue::Int(v)
    }
}

impl From<i32> for JsonValue {
    fn from(v: i32) -> Self {
        JsonValue::Int(i64::from(v))
    }
}

impl From<&str> for JsonValue {
    fn from(v: &str) -> Self {
        JsonValue::String(v.to_owned())
    }
}

impl From<String> for JsonValue {
    fn from(v: String) -> Self {
        JsonValue::String(v)
    }
}

impl From<Vec<JsonValue>> for JsonValue {
    fn from(v: Vec<JsonValue>) -> Self {
        JsonValue::Array(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_like_gson() {
        let s = JsonValue::from("\"\\\t\u{8}\n\r\u{c}\u{1}\u{1f}\u{7f}<>&='\u{2028}\u{2029}é§");
        assert_eq!(
            s.to_gson_string(),
            "\"\\\"\\\\\\t\\b\\n\\r\\f\\u0001\\u001f\u{7f}\\u003c\\u003e\\u0026\\u003d\\u0027\\u2028\\u2029é§\""
        );
    }

    /// Reference output produced by Gson 2.2.4 with `setPrettyPrinting()`.
    #[test]
    fn pretty_matches_gson() {
        let inner = JsonValue::object()
            .with("empty", JsonValue::Array(vec![]))
            .with("emptyObj", JsonValue::object())
            .with("nums", vec![JsonValue::Int(1), JsonValue::Int(2)]);
        let entry = JsonValue::object()
            .with("uuid", "069a79f4-44e9-4726-a5be-fca90e38aaf5")
            .with("name", "No<tch>")
            .with("level", 4)
            .with("bypassesPlayerLimit", false)
            .with("inner", inner);
        let v = JsonValue::Array(vec![entry, JsonValue::object()]);
        let expected = "[\n  {\n    \"uuid\": \"069a79f4-44e9-4726-a5be-fca90e38aaf5\",\n    \"name\": \"No\\u003ctch\\u003e\",\n    \"level\": 4,\n    \"bypassesPlayerLimit\": false,\n    \"inner\": {\n      \"empty\": [],\n      \"emptyObj\": {},\n      \"nums\": [\n        1,\n        2\n      ]\n    }\n  },\n  {}\n]";
        assert_eq!(v.to_gson_pretty_string(), expected);
        assert_eq!(JsonValue::Array(vec![]).to_gson_pretty_string(), "[]");
        assert_eq!(JsonValue::parse(expected).unwrap(), v);
    }

    #[test]
    fn parser_edge_cases() {
        assert_eq!(
            JsonValue::parse(r#" {"a" : [true, null, -12], "b": "é😀\n"} "#).unwrap(),
            JsonValue::object()
                .with(
                    "a",
                    vec![JsonValue::Bool(true), JsonValue::Null, JsonValue::Int(-12)]
                )
                .with("b", "\u{e9}\u{1F600}\n")
        );
        assert!(JsonValue::parse("[1,]").is_err());
        assert!(JsonValue::parse("1.5").is_err());
        assert!(JsonValue::parse("{} x").is_err());
        assert!(JsonValue::parse("\"open").is_err());
    }

    #[test]
    fn nested_compact() {
        let v = JsonValue::object()
            .with("a", vec![JsonValue::Int(1), JsonValue::Null])
            .with("b", JsonValue::object());
        assert_eq!(v.to_gson_string(), r#"{"a":[1,null],"b":{}}"#);
    }
}
