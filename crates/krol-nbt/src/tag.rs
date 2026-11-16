use indexmap::IndexMap;
use krol_java::math::{floor_f32, floor_f64};

use crate::{Error, Result};

/// The type id of an NBT tag, as written on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TagType {
    /// Marks the end of a compound; also the element type of an empty list.
    End = 0,
    /// Signed 8-bit integer.
    Byte = 1,
    /// Signed 16-bit integer.
    Short = 2,
    /// Signed 32-bit integer.
    Int = 3,
    /// Signed 64-bit integer.
    Long = 4,
    /// 32-bit IEEE float.
    Float = 5,
    /// 64-bit IEEE float.
    Double = 6,
    /// Array of signed bytes.
    ByteArray = 7,
    /// Modified-UTF-8 string.
    String = 8,
    /// Homogeneous list of unnamed tags.
    List = 9,
    /// Map of named tags.
    Compound = 10,
    /// Array of signed 32-bit integers.
    IntArray = 11,
}

impl TagType {
    /// Returns the tag type for a wire id.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidTagType`] if `id > 11`.
    pub fn from_id(id: u8) -> Result<Self> {
        Ok(match id {
            0 => Self::End,
            1 => Self::Byte,
            2 => Self::Short,
            3 => Self::Int,
            4 => Self::Long,
            5 => Self::Float,
            6 => Self::Double,
            7 => Self::ByteArray,
            8 => Self::String,
            9 => Self::List,
            10 => Self::Compound,
            11 => Self::IntArray,
            other => return Err(Error::InvalidTagType(other)),
        })
    }

    /// The wire id of this tag type.
    pub fn id(self) -> u8 {
        self as u8
    }

    /// Whether this is one of the numeric types `Byte..=Double` (vanilla's "type 99").
    pub fn is_numeric(self) -> bool {
        (1..=6).contains(&self.id())
    }
}

/// An NBT value.
#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    /// Signed 8-bit integer (also used for booleans).
    Byte(i8),
    /// Signed 16-bit integer.
    Short(i16),
    /// Signed 32-bit integer.
    Int(i32),
    /// Signed 64-bit integer.
    Long(i64),
    /// 32-bit float.
    Float(f32),
    /// 64-bit float.
    Double(f64),
    /// Array of signed bytes.
    ByteArray(Vec<i8>),
    /// String.
    String(String),
    /// Homogeneous list.
    List(List),
    /// Named-tag map.
    Compound(Compound),
    /// Array of signed 32-bit integers.
    IntArray(Vec<i32>),
}

impl Tag {
    /// The type of this tag.
    pub fn tag_type(&self) -> TagType {
        match self {
            Tag::Byte(_) => TagType::Byte,
            Tag::Short(_) => TagType::Short,
            Tag::Int(_) => TagType::Int,
            Tag::Long(_) => TagType::Long,
            Tag::Float(_) => TagType::Float,
            Tag::Double(_) => TagType::Double,
            Tag::ByteArray(_) => TagType::ByteArray,
            Tag::String(_) => TagType::String,
            Tag::List(_) => TagType::List,
            Tag::Compound(_) => TagType::Compound,
            Tag::IntArray(_) => TagType::IntArray,
        }
    }

    /// Converts a numeric tag to `long` the way vanilla does (vanilla:
    /// `NBTBase.NBTPrimitive#getLong`); `None` for non-numeric tags.
    pub fn as_i64(&self) -> Option<i64> {
        Some(match *self {
            Tag::Byte(v) => i64::from(v),
            Tag::Short(v) => i64::from(v),
            Tag::Int(v) => i64::from(v),
            Tag::Long(v) => v,
            Tag::Float(v) => v as i64,
            Tag::Double(v) => v.floor() as i64,
            _ => return None,
        })
    }

    /// Converts a numeric tag to `int` the way vanilla does: integers are truncated,
    /// floats are floored (vanilla: `NBTPrimitive#getInt`); `None` for non-numeric tags.
    pub fn as_i32(&self) -> Option<i32> {
        Some(match *self {
            Tag::Byte(v) => i32::from(v),
            Tag::Short(v) => i32::from(v),
            Tag::Int(v) => v,
            Tag::Long(v) => v as i32,
            Tag::Float(v) => floor_f32(v),
            Tag::Double(v) => floor_f64(v),
            _ => return None,
        })
    }

    /// Converts a numeric tag to `short` like vanilla (`NBTPrimitive#getShort`).
    pub fn as_i16(&self) -> Option<i16> {
        match *self {
            Tag::Byte(v) => Some(i16::from(v)),
            Tag::Short(v) => Some(v),
            _ => self.as_i32().map(|v| v as i16),
        }
    }

    /// Converts a numeric tag to `byte` like vanilla (`NBTPrimitive#getByte`).
    pub fn as_i8(&self) -> Option<i8> {
        match *self {
            Tag::Byte(v) => Some(v),
            _ => self.as_i32().map(|v| v as i8),
        }
    }

    /// Converts a numeric tag to `double` like vanilla (`NBTPrimitive#getDouble`).
    pub fn as_f64(&self) -> Option<f64> {
        Some(match *self {
            Tag::Byte(v) => f64::from(v),
            Tag::Short(v) => f64::from(v),
            Tag::Int(v) => f64::from(v),
            Tag::Long(v) => v as f64,
            Tag::Float(v) => f64::from(v),
            Tag::Double(v) => v,
            _ => return None,
        })
    }

    /// Converts a numeric tag to `float` like vanilla (`NBTPrimitive#getFloat`).
    pub fn as_f32(&self) -> Option<f32> {
        Some(match *self {
            Tag::Byte(v) => f32::from(v),
            Tag::Short(v) => f32::from(v),
            Tag::Int(v) => v as f32,
            Tag::Long(v) => v as f32,
            Tag::Float(v) => v,
            Tag::Double(v) => v as f32,
            _ => return None,
        })
    }
}

/// A homogeneous list of unnamed tags.
///
/// All elements share one [`TagType`]. Like vanilla, the element type written for an
/// empty list is [`TagType::End`], whatever type it was read with.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct List {
    items: Vec<Tag>,
}

impl List {
    /// Creates an empty list.
    pub fn new() -> Self {
        Self::default()
    }

    /// The element type: that of the first element, or [`TagType::End`] if empty.
    pub fn element_type(&self) -> TagType {
        self.items.first().map_or(TagType::End, Tag::tag_type)
    }

    /// Appends `tag`.
    ///
    /// # Errors
    ///
    /// [`Error::HeterogeneousList`] if `tag`'s type differs from the existing elements'
    /// (vanilla refuses such appends too).
    pub fn push(&mut self, tag: Tag) -> Result<()> {
        if let Some(first) = self.items.first()
            && first.tag_type() != tag.tag_type()
        {
            return Err(Error::HeterogeneousList {
                expected: first.tag_type(),
                found: tag.tag_type(),
            });
        }
        self.items.push(tag);
        Ok(())
    }

    /// The element at `index`, if any.
    pub fn get(&self, index: usize) -> Option<&Tag> {
        self.items.get(index)
    }

    /// Mutable access to the element at `index`, if any.
    ///
    /// Callers must not change the element's type.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Tag> {
        self.items.get_mut(index)
    }

    /// Removes and returns the element at `index`, if any.
    pub fn remove(&mut self, index: usize) -> Option<Tag> {
        (index < self.items.len()).then(|| self.items.remove(index))
    }

    /// Number of elements.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Iterates over the elements.
    pub fn iter(&self) -> std::slice::Iter<'_, Tag> {
        self.items.iter()
    }
}

impl<'a> IntoIterator for &'a List {
    type Item = &'a Tag;
    type IntoIter = std::slice::Iter<'a, Tag>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

/// A map of named tags.
///
/// Entries keep their insertion (or file) order. Vanilla stores compounds in a Java
/// `HashMap`, so the writer emits entries in that map's iteration order (see
/// [`crate::write`]); files written by vanilla round-trip byte for byte.
///
/// The `get_*` accessors follow vanilla's lenient getters: a missing key or a tag of the
/// wrong type yields a default (`0`, `""`, an empty compound, ...), and any numeric tag is
/// converted to the requested numeric type.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Compound {
    entries: IndexMap<String, Tag>,
}

impl Compound {
    /// Creates an empty compound.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts `tag` under `key`, returning the previous tag. A replaced key keeps its
    /// position.
    pub fn insert(&mut self, key: impl Into<String>, tag: Tag) -> Option<Tag> {
        self.entries.insert(key.into(), tag)
    }

    /// The tag stored under `key`.
    pub fn get(&self, key: &str) -> Option<&Tag> {
        self.entries.get(key)
    }

    /// Mutable access to the tag stored under `key`.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Tag> {
        self.entries.get_mut(key)
    }

    /// Removes the tag under `key`, preserving the order of the remaining entries.
    pub fn remove(&mut self, key: &str) -> Option<Tag> {
        self.entries.shift_remove(key)
    }

    /// Whether `key` is present (vanilla: `hasKey(key)`).
    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Whether `key` holds a tag of type `ty` (vanilla: `hasKey(key, type)`).
    /// Pass `None` to match any numeric type (vanilla's type `99`).
    pub fn contains_key_of(&self, key: &str, ty: Option<TagType>) -> bool {
        match (self.get(key), ty) {
            (Some(tag), Some(ty)) => tag.tag_type() == ty,
            (Some(tag), None) => tag.tag_type().is_numeric(),
            (None, _) => false,
        }
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the compound is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterates over `(key, tag)` pairs in order.
    pub fn iter(&self) -> indexmap::map::Iter<'_, String, Tag> {
        self.entries.iter()
    }

    /// Vanilla `getByte`: any numeric tag converted, else `0`.
    pub fn get_i8(&self, key: &str) -> i8 {
        self.get(key).and_then(Tag::as_i8).unwrap_or(0)
    }

    /// Vanilla `getShort`: any numeric tag converted, else `0`.
    pub fn get_i16(&self, key: &str) -> i16 {
        self.get(key).and_then(Tag::as_i16).unwrap_or(0)
    }

    /// Vanilla `getInteger`: any numeric tag converted, else `0`.
    pub fn get_i32(&self, key: &str) -> i32 {
        self.get(key).and_then(Tag::as_i32).unwrap_or(0)
    }

    /// Vanilla `getLong`: any numeric tag converted, else `0`.
    pub fn get_i64(&self, key: &str) -> i64 {
        self.get(key).and_then(Tag::as_i64).unwrap_or(0)
    }

    /// Vanilla `getFloat`: any numeric tag converted, else `0.0`.
    pub fn get_f32(&self, key: &str) -> f32 {
        self.get(key).and_then(Tag::as_f32).unwrap_or(0.0)
    }

    /// Vanilla `getDouble`: any numeric tag converted, else `0.0`.
    pub fn get_f64(&self, key: &str) -> f64 {
        self.get(key).and_then(Tag::as_f64).unwrap_or(0.0)
    }

    /// Vanilla `getBoolean`: `get_i8(key) != 0`.
    pub fn get_bool(&self, key: &str) -> bool {
        self.get_i8(key) != 0
    }

    /// Vanilla `getString`: the string, or `""` if missing or not a string.
    pub fn get_str(&self, key: &str) -> &str {
        match self.get(key) {
            Some(Tag::String(s)) => s,
            _ => "",
        }
    }

    /// Vanilla `getByteArray`: the array, or empty.
    pub fn get_byte_array(&self, key: &str) -> &[i8] {
        match self.get(key) {
            Some(Tag::ByteArray(a)) => a,
            _ => &[],
        }
    }

    /// Vanilla `getIntArray`: the array, or empty.
    pub fn get_int_array(&self, key: &str) -> &[i32] {
        match self.get(key) {
            Some(Tag::IntArray(a)) => a,
            _ => &[],
        }
    }

    /// Vanilla `getCompoundTag`: the compound, or `None` where vanilla would return a
    /// fresh empty compound.
    pub fn get_compound(&self, key: &str) -> Option<&Compound> {
        match self.get(key) {
            Some(Tag::Compound(c)) => Some(c),
            _ => None,
        }
    }

    /// Vanilla `getTagList(key, type)`: the list if present and its elements are of type
    /// `ty` (an empty list always matches); `None` where vanilla returns an empty list.
    pub fn get_list(&self, key: &str, ty: TagType) -> Option<&List> {
        match self.get(key) {
            Some(Tag::List(l)) if l.is_empty() || l.element_type() == ty => Some(l),
            _ => None,
        }
    }
}

impl<'a> IntoIterator for &'a Compound {
    type Item = (&'a String, &'a Tag);
    type IntoIter = indexmap::map::Iter<'a, String, Tag>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

/// Whether `target` contains `pattern` (vanilla: `NBTUtil.func_181123_a`, used by
/// `/clear`, `/testfor` and friends): every key of a pattern compound must match in the
/// target, recursively; with `partial_lists` every element of a pattern list must match
/// some target element (an empty pattern list only matches an empty list), otherwise
/// lists must be equal. Other tags must be equal. A missing target never matches.
///
/// ```
/// use krol_nbt::{Compound, Tag, matches_pattern};
///
/// let mut target = Compound::new();
/// target.insert("a", Tag::Int(1));
/// target.insert("b", Tag::Int(2));
/// let mut pattern = Compound::new();
/// pattern.insert("a", Tag::Int(1));
/// assert!(matches_pattern(&Tag::Compound(pattern), Some(&Tag::Compound(target)), true));
/// ```
pub fn matches_pattern(pattern: &Tag, target: Option<&Tag>, partial_lists: bool) -> bool {
    let Some(target) = target else {
        return false;
    };
    match (pattern, target) {
        (Tag::Compound(p), Tag::Compound(t)) => p
            .iter()
            .all(|(k, v)| matches_pattern(v, t.get(k), partial_lists)),
        (Tag::List(p), Tag::List(t)) if partial_lists => {
            if p.is_empty() {
                return t.is_empty();
            }
            p.iter()
                .all(|e| t.iter().any(|x| matches_pattern(e, Some(x), partial_lists)))
        }
        _ => pattern == target,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_numeric_getters() {
        let mut c = Compound::new();
        c.insert("d", Tag::Double(-1.5));
        c.insert("f", Tag::Float(2.7));
        c.insert("l", Tag::Long(0x1_0000_0101));
        c.insert("s", Tag::String("x".into()));
        assert_eq!(c.get_i32("d"), -2); // floored
        assert_eq!(c.get_i64("d"), -2);
        assert_eq!(c.get_i32("f"), 2);
        assert_eq!(c.get_i32("l"), 0x101); // truncated
        assert_eq!(c.get_i8("l"), 1);
        assert_eq!(c.get_i32("s"), 0);
        assert_eq!(c.get_i32("missing"), 0);
        assert_eq!(c.get_str("d"), "");
        assert!(c.contains_key_of("f", None));
        assert!(!c.contains_key_of("s", None));
    }

    #[test]
    fn list_is_homogeneous() {
        let mut l = List::new();
        assert_eq!(l.element_type(), TagType::End);
        l.push(Tag::Int(1)).unwrap();
        assert!(l.push(Tag::Byte(1)).is_err());
        assert_eq!(l.element_type(), TagType::Int);
    }

    #[test]
    fn get_list_checks_type() {
        let mut c = Compound::new();
        let mut l = List::new();
        l.push(Tag::Int(1)).unwrap();
        c.insert("ints", Tag::List(l));
        c.insert("empty", Tag::List(List::new()));
        assert!(c.get_list("ints", TagType::Int).is_some());
        assert!(c.get_list("ints", TagType::Compound).is_none());
        assert!(c.get_list("empty", TagType::Compound).is_some());
    }
}
