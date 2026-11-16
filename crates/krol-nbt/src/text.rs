//! The text forms of NBT in 1.8: `NBTBase.toString` (shown in item tooltips' hover
//! data and command output) and the lenient "JSON" parser commands accept for data
//! tags (vanilla: `JsonToNBT`).
//!
//! The parser keeps every quirk of the 1.8 one: list elements may carry ignored
//! `index:` prefixes, `[1,2]` is an int array but `[]` an empty list, `1|` parses as a
//! double (the `[d|D]` character class), numbers that overflow their type become
//! strings, and a list refuses elements of another type than its first (reported
//! through the `warn` callback like vanilla logs it).

use krol_java::{java_hash_order, java_string_hash};

use crate::{Compound, List, Tag};

/// Why a data tag could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Vanilla's `NBTException`, with its message.
    Syntax(String),
    /// Input that makes vanilla's parser throw an unchecked exception (a lone `{`, `[`
    /// or `"`), which commands report as an unknown error.
    Internal,
}

type Parsed<T> = std::result::Result<T, ParseError>;

fn syntax<T>(message: impl Into<String>) -> Parsed<T> {
    Err(ParseError::Syntax(message.into()))
}

/// `NBTBase.toString`: compounds as `{key:value,...}` in Java `HashMap` order (keys
/// unquoted), lists as `[0:a,1:b]`, numbers with their type suffix, strings quoted with
/// only `"` escaped, byte arrays as `[N bytes]` and int arrays as `[1,2,]`.
///
/// ```
/// use krol_nbt::{Compound, Tag};
///
/// let mut c = Compound::new();
/// c.insert("id", Tag::String("minecraft:stone".into()));
/// c.insert("Count", Tag::Byte(1));
/// c.insert("Damage", Tag::Short(0));
/// assert_eq!(
///     krol_nbt::text::to_java_string(&Tag::Compound(c)),
///     "{id:\"minecraft:stone\",Count:1b,Damage:0s}"
/// );
/// ```
pub fn to_java_string(tag: &Tag) -> String {
    let mut out = String::new();
    write_tag(&mut out, tag);
    out
}

fn write_tag(out: &mut String, tag: &Tag) {
    use krol_java::fmt::{java_double, java_float};
    match tag {
        Tag::Byte(v) => out.push_str(&format!("{v}b")),
        Tag::Short(v) => out.push_str(&format!("{v}s")),
        Tag::Int(v) => out.push_str(&v.to_string()),
        Tag::Long(v) => out.push_str(&format!("{v}L")),
        Tag::Float(v) => {
            out.push_str(&java_float(*v));
            out.push('f');
        }
        Tag::Double(v) => {
            out.push_str(&java_double(*v));
            out.push('d');
        }
        Tag::ByteArray(v) => out.push_str(&format!("[{} bytes]", v.len())),
        Tag::String(s) => {
            out.push('"');
            out.push_str(&s.replace('"', "\\\""));
            out.push('"');
        }
        Tag::List(list) => {
            out.push('[');
            for (k, item) in list.iter().enumerate() {
                if k != 0 {
                    out.push(',');
                }
                out.push_str(&k.to_string());
                out.push(':');
                write_tag(out, item);
            }
            out.push(']');
        }
        Tag::Compound(c) => {
            out.push('{');
            let entries: Vec<(&String, &Tag)> = c.iter().collect();
            let hashes: Vec<i32> = entries.iter().map(|(k, _)| java_string_hash(k)).collect();
            for (n, k) in java_hash_order(&hashes).into_iter().enumerate() {
                if n != 0 {
                    out.push(',');
                }
                let (key, value) = entries[k];
                out.push_str(key);
                out.push(':');
                write_tag(out, value);
            }
            out.push('}');
        }
        Tag::IntArray(v) => {
            out.push('[');
            for x in v {
                out.push_str(&x.to_string());
                out.push(',');
            }
            out.push(']');
        }
    }
}

/// Parses a command's data tag like vanilla's `JsonToNBT.getTagFromJson`.
///
/// `warn` receives the warnings vanilla logs while building the tag ("Adding
/// mismatching tag types to tag list" when a list element is dropped).
///
/// # Errors
///
/// [`ParseError::Syntax`] with vanilla's message, or [`ParseError::Internal`] where
/// vanilla throws an unchecked exception.
///
/// ```
/// use krol_nbt::Tag;
///
/// let tag = krol_nbt::text::parse_data_tag("{display:{Name:\"Pick\"},Unbreakable:1b}", &mut |_| {})
///     .unwrap();
/// assert_eq!(tag.get("Unbreakable"), Some(&Tag::Byte(1)));
/// assert_eq!(tag.get_compound("display").unwrap().get_str("Name"), "Pick");
/// ```
pub fn parse_data_tag(text: &str, warn: &mut dyn FnMut(&str)) -> Parsed<Compound> {
    let s: Vec<char> = java_trim(&text.chars().collect::<Vec<_>>()).to_vec();
    if s.first() != Some(&'{') {
        return syntax("Invalid tag encountered, expected '{' as first char.");
    }
    if count_top_tags(&s)? != 1 {
        return syntax("Encountered multiple top tags, only one expected");
    }
    match build(&s)?.into_tag(warn) {
        Tag::Compound(c) => Ok(c),
        // Unreachable: the input starts with '{'.
        _ => Err(ParseError::Internal),
    }
}

/// The parsed structure before tags are built (vanilla: `JsonToNBT.Any`).
enum Node {
    Compound(Vec<(String, Node)>),
    List(Vec<Node>),
    Primitive(String),
}

impl Node {
    fn into_tag(self, warn: &mut dyn FnMut(&str)) -> Tag {
        match self {
            Node::Compound(children) => {
                let mut c = Compound::new();
                for (name, child) in children {
                    let tag = child.into_tag(warn);
                    c.insert(name, tag);
                }
                Tag::Compound(c)
            }
            Node::List(children) => {
                let mut list = List::new();
                for child in children {
                    let tag = child.into_tag(warn);
                    if list.push(tag).is_err() {
                        warn("Adding mismatching tag types to tag list");
                    }
                }
                Tag::List(list)
            }
            Node::Primitive(value) => primitive(&value).unwrap_or_else(|| {
                // Unreachable after `build` validated a lone quote; keep the text.
                Tag::String(value)
            }),
        }
    }
}

/// `String.trim`: strips characters up to U+0020 from both ends.
fn java_trim(s: &[char]) -> &[char] {
    let start = s.iter().position(|&c| c > ' ').unwrap_or(s.len());
    let end = s.iter().rposition(|&c| c > ' ').map_or(start, |e| e + 1);
    &s[start..end]
}

/// `Character.isWhitespace`.
fn java_is_whitespace(c: char) -> bool {
    match c {
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | '\u{1c}'..='\u{1f}' => true,
        '\u{a0}' | '\u{2007}' | '\u{202f}' => false,
        // Unicode space, line and paragraph separators.
        c => matches!(
            c,
            ' ' | '\u{1680}' | '\u{2000}'
                ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{205f}' | '\u{3000}'
        ),
    }
}

fn string(s: &[char]) -> String {
    s.iter().collect()
}

/// Whether the quote at `i` is escaped: an odd number of backslashes precede it.
fn escaped(s: &[char], i: usize) -> bool {
    s[..i].iter().rev().take_while(|&&c| c == '\\').count() % 2 == 1
}

/// Counts the top-level `{...}`/`[...]` groups, checking quotes and brackets balance.
fn count_top_tags(s: &[char]) -> Parsed<usize> {
    let all = string(s);
    let mut count = 0;
    let mut quoted = false;
    let mut stack = Vec::new();
    for (i, &c) in s.iter().enumerate() {
        if c == '"' {
            if escaped(s, i) {
                if !quoted {
                    return syntax(format!("Illegal use of \\\": {all}"));
                }
            } else {
                quoted = !quoted;
            }
        } else if !quoted {
            if c == '{' || c == '[' {
                if stack.is_empty() {
                    count += 1;
                }
                stack.push(c);
            } else {
                close_bracket(&mut stack, c, &all)?;
            }
        }
    }
    if quoted {
        return syntax(format!("Unbalanced quotation: {all}"));
    }
    if !stack.is_empty() {
        return syntax(format!("Unbalanced brackets: {all}"));
    }
    if count == 0 && !s.is_empty() {
        count = 1;
    }
    Ok(count)
}

/// Checks a closing bracket against the open ones.
fn close_bracket(stack: &mut Vec<char>, c: char, all: &str) -> Parsed<()> {
    if c == '}' && stack.pop() != Some('{') {
        return syntax(format!("Unbalanced curly brackets {{}}: {all}"));
    }
    if c == ']' && stack.pop() != Some('[') {
        return syntax(format!("Unbalanced square brackets []: {all}"));
    }
    Ok(())
}

/// Whether `s` is an int array literal (`\[[-+\d|,\s]+\]`).
fn is_int_array(s: &[char]) -> bool {
    s.len() > 2
        && s[0] == '['
        && s[s.len() - 1] == ']'
        && s[1..s.len() - 1].iter().all(|&c| {
            c.is_ascii_digit()
                || matches!(
                    c,
                    '-' | '+' | '|' | ',' | ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r'
                )
        })
}

/// Builds the node for a value (vanilla: `JsonToNBT.func_150316_a`).
fn build(value: &[char]) -> Parsed<Node> {
    let v = java_trim(value);
    let compound = v.first() == Some(&'{');
    if !compound && !(v.first() == Some(&'[') && !is_int_array(v)) {
        let text = string(v);
        if text == "\"" {
            // Vanilla strips the quotes of `"` with substring(1, 0).
            return Err(ParseError::Internal);
        }
        return Ok(Node::Primitive(text));
    }
    if v.len() < 2 {
        return Err(ParseError::Internal);
    }
    let mut rest: &[char] = &v[1..v.len() - 1];
    let mut compound_children = Vec::new();
    let mut list_children = Vec::new();
    while !rest.is_empty() {
        let token = next_token(rest, compound)?;
        if !token.is_empty() {
            let name = token_name(token, !compound)?;
            let value = token_value(token, !compound)?;
            let node = build(value)?;
            if compound {
                compound_children.push((string(name), node));
            } else {
                list_children.push(node);
            }
        }
        if rest.len() < token.len() + 1 {
            break;
        }
        let c = rest[token.len()];
        if !matches!(c, ',' | '{' | '}' | '[' | ']') {
            return syntax(format!(
                "Unexpected token '{c}' at: {}",
                string(&rest[token.len()..])
            ));
        }
        rest = &rest[token.len() + 1..];
    }
    Ok(if compound {
        Node::Compound(compound_children)
    } else {
        Node::List(list_children)
    })
}

/// The index of `wanted` outside quotes, unless an opening bracket comes first.
fn find_unquoted(s: &[char], wanted: char) -> Option<usize> {
    let mut outside = true;
    for (i, &c) in s.iter().enumerate() {
        if c == '"' {
            if !escaped(s, i) {
                outside = !outside;
            }
        } else if outside {
            if c == wanted {
                return Some(i);
            }
            if c == '{' || c == '[' {
                return None;
            }
        }
    }
    None
}

/// The next `name:value` (compound) or element (list) token (vanilla:
/// `JsonToNBT.func_150314_a`).
fn next_token(s: &[char], compound: bool) -> Parsed<&[char]> {
    let mut colon = find_unquoted(s, ':');
    let comma = find_unquoted(s, ',');
    if compound {
        let Some(c) = colon else {
            return syntax(format!(
                "Unable to locate name/value separator for string: {}",
                string(s)
            ));
        };
        if comma.is_some_and(|k| k < c) {
            return syntax(format!("Name error at: {}", string(s)));
        }
    } else if colon.is_none() || colon > comma {
        // Vanilla quirk: `-1 > comma` is false only for a found comma, so a list element
        // keeps an `index:` prefix only when a comma follows it.
        colon = None;
    }
    scan_token(s, colon)
}

/// Scans to the end of a token after the separator (vanilla: `JsonToNBT.func_179269_a`):
/// up to a top-level comma, or just past a quoted value.
fn scan_token(s: &[char], separator: Option<usize>) -> Parsed<&[char]> {
    let all = string(s);
    let mut stack = Vec::new();
    let mut quoted = false;
    let mut starts_quoted = false;
    let mut seen = false;
    let mut quote_end = 0;
    let mut i = separator.map_or(0, |c| c + 1);
    while i < s.len() {
        let c = s[i];
        if c == '"' {
            if escaped(s, i) {
                if !quoted {
                    return syntax(format!("Illegal use of \\\": {all}"));
                }
            } else {
                quoted = !quoted;
                if quoted && !seen {
                    starts_quoted = true;
                }
                if !quoted {
                    quote_end = i;
                }
            }
        } else if !quoted {
            if c == '{' || c == '[' {
                stack.push(c);
            } else {
                close_bracket(&mut stack, c, &all)?;
                if c == ',' && stack.is_empty() {
                    return Ok(&s[..i]);
                }
            }
        }
        if !java_is_whitespace(c) {
            if !quoted && starts_quoted && quote_end != i {
                return Ok(&s[..quote_end + 1]);
            }
            seen = true;
        }
        i += 1;
    }
    Ok(&s[..i])
}

/// A token's name (vanilla: `JsonToNBT.func_150313_b`).
fn token_name(s: &[char], list: bool) -> Parsed<&[char]> {
    if list {
        let t = java_trim(s);
        if matches!(t.first(), Some('{' | '[')) {
            return Ok(&[]);
        }
    }
    match find_unquoted(s, ':') {
        Some(c) => Ok(java_trim(&s[..c])),
        None if list => Ok(&[]),
        None => syntax(format!(
            "Unable to locate name/value separator for string: {}",
            string(s)
        )),
    }
}

/// A token's value (vanilla: `JsonToNBT.func_150311_c`).
fn token_value(s: &[char], list: bool) -> Parsed<&[char]> {
    if list {
        let t = java_trim(s);
        if matches!(t.first(), Some('{' | '[')) {
            return Ok(t);
        }
    }
    match find_unquoted(s, ':') {
        Some(c) => Ok(java_trim(&s[c + 1..])),
        None if list => Ok(s),
        None => syntax(format!(
            "Unable to locate name/value separator for string: {}",
            string(s)
        )),
    }
}

/// `[-+]?[0-9]+`.
fn is_integer(s: &str) -> bool {
    let digits = s.strip_prefix(['-', '+']).unwrap_or(s);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// `[-+]?[0-9]*\.?[0-9]+`.
fn is_decimal(s: &str) -> bool {
    let body = s.strip_prefix(['-', '+']).unwrap_or(s);
    match body.split_once('.') {
        Some((int, frac)) => {
            int.bytes().all(|b| b.is_ascii_digit())
                && !frac.is_empty()
                && frac.bytes().all(|b| b.is_ascii_digit())
        }
        None => is_integer(body) && !body.starts_with(['-', '+']),
    }
}

/// `value` without its last character when that is one of `suffixes` and the rest
/// matches `body`.
fn with_suffix<'a>(value: &'a str, suffixes: &[char], body: fn(&str) -> bool) -> Option<&'a str> {
    let last = value.chars().last()?;
    let rest = &value[..value.len() - last.len_utf8()];
    (suffixes.contains(&last) && body(rest)).then_some(rest)
}

/// `replaceAll("\\\\\"", "\"")`: unescapes quotes.
fn unescape_quotes(s: &str) -> String {
    s.replace("\\\"", "\"")
}

/// Builds a primitive tag (vanilla: `JsonToNBT.Primitive.parse`). `None` only for a
/// lone `"`, which [`build`] rejects first.
fn primitive(value: &str) -> Option<Tag> {
    // Vanilla quirk: the type suffix classes are `[d|D]` and so on, so `|` is accepted
    // as a suffix too.
    let number = if let Some(v) = with_suffix(value, &['d', '|', 'D'], is_decimal) {
        v.parse().ok().map(Tag::Double)
    } else if let Some(v) = with_suffix(value, &['f', '|', 'F'], is_decimal) {
        v.parse().ok().map(Tag::Float)
    } else if let Some(v) = with_suffix(value, &['b', '|', 'B'], is_integer) {
        Some(
            v.parse()
                .map_or_else(|_| Tag::String(unescape_quotes(value)), Tag::Byte),
        )
    } else if let Some(v) = with_suffix(value, &['l', '|', 'L'], is_integer) {
        Some(
            v.parse()
                .map_or_else(|_| Tag::String(unescape_quotes(value)), Tag::Long),
        )
    } else if let Some(v) = with_suffix(value, &['s', '|', 'S'], is_integer) {
        Some(
            v.parse()
                .map_or_else(|_| Tag::String(unescape_quotes(value)), Tag::Short),
        )
    } else if is_integer(value) {
        Some(
            value
                .parse()
                .map_or_else(|_| Tag::String(unescape_quotes(value)), Tag::Int),
        )
    } else if is_decimal(value) {
        value.parse().ok().map(Tag::Double)
    } else if value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false") {
        Some(Tag::Byte(i8::from(value.eq_ignore_ascii_case("true"))))
    } else {
        None
    };
    if number.is_some() {
        return number;
    }
    if value.starts_with('[') && value.ends_with(']') && value.len() >= 2 {
        let inner = &value[1..value.len() - 1];
        let ints: Option<Vec<i32>> = inner
            .split(',')
            .filter(|p| !p.is_empty())
            .map(|p| {
                java_trim(&p.chars().collect::<Vec<_>>())
                    .iter()
                    .collect::<String>()
                    .parse()
                    .ok()
            })
            .collect();
        return Some(ints.map_or_else(|| Tag::String(value.to_owned()), Tag::IntArray));
    }
    let mut v = value;
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        v = &v[1..v.len() - 1];
    } else if v == "\"" {
        return None;
    }
    let v = unescape_quotes(v);
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'\\') {
            chars.next();
        }
        out.push(c);
    }
    Some(Tag::String(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Parsed<Compound> {
        parse_data_tag(s, &mut |_| {})
    }

    fn text(s: &str) -> String {
        to_java_string(&Tag::Compound(parse(s).unwrap()))
    }

    #[test]
    fn primitives() {
        let c =
            parse("{a:1,b:2b,c:3s,d:4L,e:1.5f,f:2.5d,g:3.5,h:true,i:x,j:\"y z\",k:1|}").unwrap();
        assert_eq!(c.get("a"), Some(&Tag::Int(1)));
        assert_eq!(c.get("b"), Some(&Tag::Byte(2)));
        assert_eq!(c.get("c"), Some(&Tag::Short(3)));
        assert_eq!(c.get("d"), Some(&Tag::Long(4)));
        assert_eq!(c.get("e"), Some(&Tag::Float(1.5)));
        assert_eq!(c.get("f"), Some(&Tag::Double(2.5)));
        assert_eq!(c.get("g"), Some(&Tag::Double(3.5)));
        assert_eq!(c.get("h"), Some(&Tag::Byte(1)));
        assert_eq!(c.get("i"), Some(&Tag::String("x".into())));
        assert_eq!(c.get("j"), Some(&Tag::String("y z".into())));
        assert_eq!(c.get("k"), Some(&Tag::Double(1.0)));
    }

    #[test]
    fn overflow_becomes_string() {
        let c = parse("{a:300b,b:99999999999}").unwrap();
        assert_eq!(c.get("a"), Some(&Tag::String("300b".into())));
        assert_eq!(c.get("b"), Some(&Tag::String("99999999999".into())));
    }

    #[test]
    fn lists_and_arrays() {
        let c = parse("{a:[1,2,3],b:[],c:[0:\"x\",1:\"y\"],d:[{n:1},{n:2}]}").unwrap();
        assert_eq!(c.get("a"), Some(&Tag::IntArray(vec![1, 2, 3])));
        assert_eq!(c.get("b"), Some(&Tag::List(List::new())));
        let Some(Tag::List(l)) = c.get("c") else {
            panic!()
        };
        assert_eq!(l.get(1), Some(&Tag::String("y".into())));
        let Some(Tag::List(l)) = c.get("d") else {
            panic!()
        };
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn mismatched_list_elements_warn() {
        let mut warnings = Vec::new();
        let c = parse_data_tag("{a:[\"x\",{n:1}]}", &mut |w| warnings.push(w.to_owned())).unwrap();
        let Some(Tag::List(l)) = c.get("a") else {
            panic!()
        };
        assert_eq!(l.len(), 1);
        assert_eq!(warnings, ["Adding mismatching tag types to tag list"]);
    }

    #[test]
    fn errors() {
        let e = |s: &str| match parse(s) {
            Err(ParseError::Syntax(m)) => m,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            e("a"),
            "Invalid tag encountered, expected '{' as first char."
        );
        assert_eq!(
            e("{a:1}{b:2}"),
            "Encountered multiple top tags, only one expected"
        );
        assert_eq!(e("{a:1"), "Unbalanced brackets: {a:1");
        assert_eq!(e("{a:\"1}"), "Unbalanced quotation: {a:\"1}");
        assert_eq!(
            e("{a}"),
            "Unable to locate name/value separator for string: a"
        );
        assert_eq!(e("{a:1]}"), "Unbalanced square brackets []: {a:1]}");
        assert_eq!(e("{a:\"x\"y}"), "Unexpected token 'y' at: y");
    }

    #[test]
    fn java_strings() {
        assert_eq!(text("{a:\"q\\\"\"}"), "{a:\"q\\\"\"}");
        assert_eq!(text("{l:[1b,2b]}"), "{l:[0:1b,1:2b]}");
        assert_eq!(text("{i:[1,2]}"), "{i:[1,2,]}");
        assert_eq!(text("{f:0.1f,d:2.5}"), "{d:2.5d,f:0.1f}");
    }
}
