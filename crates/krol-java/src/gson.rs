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

    #[test]
    fn nested_compact() {
        let v = JsonValue::object()
            .with("a", vec![JsonValue::Int(1), JsonValue::Null])
            .with("b", JsonValue::object());
        assert_eq!(v.to_gson_string(), r#"{"a":[1,null],"b":{}}"#);
    }
}
