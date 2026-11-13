//! Field-level encoding primitives (vanilla: `PacketBuffer`).

use crate::{DecodeError, EncodeError};

/// Maximum UTF-8 byte length of any string written by vanilla.
pub const MAX_STRING_BYTES: usize = 32767;

/// Number of bytes `value` occupies as a VarInt.
pub fn varint_len(value: i32) -> usize {
    let v = value as u32;
    match v {
        0..=0x7f => 1,
        0x80..=0x3fff => 2,
        0x4000..=0x1f_ffff => 3,
        0x20_0000..=0x0fff_ffff => 4,
        _ => 5,
    }
}

/// A cursor reading protocol fields from a byte slice.
#[derive(Debug, Clone)]
pub struct PacketReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> PacketReader<'a> {
    /// Creates a reader over `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Bytes not yet read.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// The unread bytes, without consuming them.
    pub fn rest(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }

    /// Reads exactly `n` bytes.
    ///
    /// # Errors
    ///
    /// [`DecodeError::UnexpectedEnd`] if fewer remain.
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.remaining() < n {
            return Err(DecodeError::UnexpectedEnd);
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let mut out = [0; N];
        out.copy_from_slice(self.bytes(N)?);
        Ok(out)
    }

    /// Reads an unsigned byte.
    ///
    /// # Errors
    ///
    /// [`DecodeError::UnexpectedEnd`].
    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.array::<1>()?[0])
    }

    /// Reads a signed byte.
    ///
    /// # Errors
    ///
    /// [`DecodeError::UnexpectedEnd`].
    pub fn i8(&mut self) -> Result<i8, DecodeError> {
        Ok(self.u8()? as i8)
    }

    /// Reads a boolean; like Java's `readBoolean`, any non-zero byte is `true`.
    ///
    /// # Errors
    ///
    /// [`DecodeError::UnexpectedEnd`].
    pub fn bool(&mut self) -> Result<bool, DecodeError> {
        Ok(self.u8()? != 0)
    }

    /// Reads a big-endian `i16`.
    ///
    /// # Errors
    ///
    /// [`DecodeError::UnexpectedEnd`].
    pub fn i16(&mut self) -> Result<i16, DecodeError> {
        Ok(i16::from_be_bytes(self.array()?))
    }

    /// Reads a big-endian `u16`.
    ///
    /// # Errors
    ///
    /// [`DecodeError::UnexpectedEnd`].
    pub fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    /// Reads a big-endian `i32`.
    ///
    /// # Errors
    ///
    /// [`DecodeError::UnexpectedEnd`].
    pub fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_be_bytes(self.array()?))
    }

    /// Reads a big-endian `i64`.
    ///
    /// # Errors
    ///
    /// [`DecodeError::UnexpectedEnd`].
    pub fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_be_bytes(self.array()?))
    }

    /// Reads a big-endian `f32`.
    ///
    /// # Errors
    ///
    /// [`DecodeError::UnexpectedEnd`].
    pub fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_be_bytes(self.array()?))
    }

    /// Reads a big-endian `f64`.
    ///
    /// # Errors
    ///
    /// [`DecodeError::UnexpectedEnd`].
    pub fn f64(&mut self) -> Result<f64, DecodeError> {
        Ok(f64::from_be_bytes(self.array()?))
    }

    /// Reads a VarInt (at most 5 bytes).
    ///
    /// # Errors
    ///
    /// [`DecodeError::VarIntTooBig`] or [`DecodeError::UnexpectedEnd`].
    pub fn varint(&mut self) -> Result<i32, DecodeError> {
        let mut result: u32 = 0;
        for i in 0..5 {
            let b = self.u8()?;
            result |= u32::from(b & 0x7f) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(result as i32);
            }
        }
        Err(DecodeError::VarIntTooBig)
    }

    /// Reads a VarLong (at most 10 bytes).
    ///
    /// # Errors
    ///
    /// [`DecodeError::VarIntTooBig`] or [`DecodeError::UnexpectedEnd`].
    pub fn varlong(&mut self) -> Result<i64, DecodeError> {
        let mut result: u64 = 0;
        for i in 0..10 {
            let b = self.u8()?;
            result |= u64::from(b & 0x7f) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(result as i64);
            }
        }
        Err(DecodeError::VarIntTooBig)
    }

    /// Reads a VarInt-prefixed UTF-8 string of at most `max_chars` UTF-16 units
    /// (vanilla: `readStringFromBuffer`).
    ///
    /// Malformed UTF-8 is replaced with U+FFFD, as Java's decoder does.
    ///
    /// # Errors
    ///
    /// The string length errors of [`DecodeError`], or [`DecodeError::UnexpectedEnd`].
    pub fn string(&mut self, max_chars: usize) -> Result<String, DecodeError> {
        let len = self.varint()?;
        let max_bytes = (max_chars as i32).saturating_mul(4);
        if len > max_bytes {
            return Err(DecodeError::StringBytesTooLong { len, max_bytes });
        }
        if len < 0 {
            return Err(DecodeError::NegativeStringLength);
        }
        let s = String::from_utf8_lossy(self.bytes(len as usize)?).into_owned();
        if s.encode_utf16().count() > max_chars {
            return Err(DecodeError::StringTooLong {
                len,
                max: max_chars,
            });
        }
        Ok(s)
    }

    /// Reads a 128-bit UUID as two big-endian longs (most significant first).
    ///
    /// # Errors
    ///
    /// [`DecodeError::UnexpectedEnd`].
    pub fn uuid(&mut self) -> Result<u128, DecodeError> {
        Ok(u128::from_be_bytes(self.array()?))
    }

    /// Reads a protocol-47 packed block position `(x, y, z)`.
    ///
    /// # Errors
    ///
    /// [`DecodeError::UnexpectedEnd`].
    pub fn position(&mut self) -> Result<(i32, i32, i32), DecodeError> {
        Ok(unpack_position(self.i64()?))
    }

    /// Reads a VarInt-length-prefixed byte array (protocol 47) of at most `max` bytes.
    ///
    /// # Errors
    ///
    /// [`DecodeError::InvalidLength`] or [`DecodeError::UnexpectedEnd`].
    pub fn byte_array(&mut self, max: usize) -> Result<&'a [u8], DecodeError> {
        let len = self.varint()?;
        if len < 0 || len as usize > max {
            return Err(DecodeError::InvalidLength(len));
        }
        self.bytes(len as usize)
    }

    /// Reads a short-length-prefixed byte array (protocol 5) of at most `max` bytes.
    ///
    /// # Errors
    ///
    /// [`DecodeError::InvalidLength`] or [`DecodeError::UnexpectedEnd`].
    pub fn short_byte_array(&mut self, max: usize) -> Result<&'a [u8], DecodeError> {
        let len = i32::from(self.i16()?);
        if len < 0 || len as usize > max {
            return Err(DecodeError::InvalidLength(len));
        }
        self.bytes(len as usize)
    }

    /// Reads optional NBT as embedded in protocol-47 packets: a single `0` byte means
    /// "none", otherwise an uncompressed named root compound.
    ///
    /// # Errors
    ///
    /// [`DecodeError::Nbt`] or [`DecodeError::UnexpectedEnd`].
    pub fn nbt(&mut self) -> Result<Option<krol_nbt::Compound>, DecodeError> {
        if self.rest().first() == Some(&0) {
            self.pos += 1;
            return Ok(None);
        }
        let mut slice = self.rest();
        let before = slice.len();
        let (_, compound) =
            krol_nbt::read(&mut slice).map_err(|e| DecodeError::Nbt(e.to_string()))?;
        self.pos += before - slice.len();
        Ok(Some(compound))
    }
}

/// Packs a block position into protocol 47's 64-bit format
/// (26 bits x, 12 bits y, 26 bits z).
pub fn pack_position(x: i32, y: i32, z: i32) -> i64 {
    ((i64::from(x) & 0x3ff_ffff) << 38)
        | ((i64::from(y) & 0xfff) << 26)
        | (i64::from(z) & 0x3ff_ffff)
}

/// Unpacks a protocol-47 position, sign-extending each component.
pub fn unpack_position(packed: i64) -> (i32, i32, i32) {
    let x = (packed >> 38) as i32;
    let y = ((packed << 26) >> 52) as i32;
    let z = ((packed << 38) >> 38) as i32;
    (x, y, z)
}

/// A growable buffer writing protocol fields.
#[derive(Debug, Clone, Default)]
pub struct PacketWriter {
    buf: Vec<u8>,
}

impl PacketWriter {
    /// Creates an empty writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// The bytes written so far.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Consumes the writer, returning its bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    /// Writes raw bytes.
    pub fn bytes(&mut self, data: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(data);
        self
    }

    /// Writes an unsigned byte.
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    /// Writes a signed byte.
    pub fn i8(&mut self, v: i8) -> &mut Self {
        self.u8(v as u8)
    }

    /// Writes a boolean as `0`/`1`.
    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.u8(u8::from(v))
    }

    /// Writes a big-endian `i16`.
    pub fn i16(&mut self, v: i16) -> &mut Self {
        self.bytes(&v.to_be_bytes())
    }

    /// Writes a big-endian `u16`.
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.bytes(&v.to_be_bytes())
    }

    /// Writes a big-endian `i32`.
    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.bytes(&v.to_be_bytes())
    }

    /// Writes a big-endian `i64`.
    pub fn i64(&mut self, v: i64) -> &mut Self {
        self.bytes(&v.to_be_bytes())
    }

    /// Writes a big-endian `f32`.
    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.bytes(&v.to_be_bytes())
    }

    /// Writes a big-endian `f64`.
    pub fn f64(&mut self, v: f64) -> &mut Self {
        self.bytes(&v.to_be_bytes())
    }

    /// Writes a VarInt.
    pub fn varint(&mut self, v: i32) -> &mut Self {
        let mut v = v as u32;
        while v & !0x7f != 0 {
            self.buf.push((v & 0x7f) as u8 | 0x80);
            v >>= 7;
        }
        self.buf.push(v as u8);
        self
    }

    /// Writes a VarLong.
    pub fn varlong(&mut self, v: i64) -> &mut Self {
        let mut v = v as u64;
        while v & !0x7f != 0 {
            self.buf.push((v & 0x7f) as u8 | 0x80);
            v >>= 7;
        }
        self.buf.push(v as u8);
        self
    }

    /// Writes a VarInt-prefixed UTF-8 string (vanilla: `writeString`).
    ///
    /// # Errors
    ///
    /// [`EncodeError::StringTooBig`] if the UTF-8 encoding exceeds 32767 bytes.
    pub fn string(&mut self, s: &str) -> Result<&mut Self, EncodeError> {
        if s.len() > MAX_STRING_BYTES {
            return Err(EncodeError::StringTooBig(s.len()));
        }
        self.varint(s.len() as i32);
        Ok(self.bytes(s.as_bytes()))
    }

    /// Writes a 128-bit UUID as two big-endian longs.
    pub fn uuid(&mut self, v: u128) -> &mut Self {
        self.bytes(&v.to_be_bytes())
    }

    /// Writes a protocol-47 packed block position.
    pub fn position(&mut self, x: i32, y: i32, z: i32) -> &mut Self {
        self.i64(pack_position(x, y, z))
    }

    /// Writes a VarInt-length-prefixed byte array (protocol 47).
    pub fn byte_array(&mut self, data: &[u8]) -> &mut Self {
        self.varint(data.len() as i32);
        self.bytes(data)
    }

    /// Writes a short-length-prefixed byte array (protocol 5).
    pub fn short_byte_array(&mut self, data: &[u8]) -> &mut Self {
        self.i16(data.len() as i16);
        self.bytes(data)
    }

    /// Writes optional NBT as embedded in protocol-47 packets.
    ///
    /// # Errors
    ///
    /// Propagates NBT string length errors as [`EncodeError::StringTooBig`].
    pub fn nbt(&mut self, nbt: Option<&krol_nbt::Compound>) -> Result<&mut Self, EncodeError> {
        match nbt {
            None => Ok(self.u8(0)),
            Some(c) => {
                krol_nbt::write(&mut self.buf, "", c).map_err(|e| match e {
                    krol_nbt::Error::StringTooLong(n) => EncodeError::StringTooBig(n),
                    _ => EncodeError::StringTooBig(0),
                })?;
                Ok(self)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_vectors() {
        let cases: [(i32, &[u8]); 8] = [
            (0, &[0x00]),
            (1, &[0x01]),
            (127, &[0x7f]),
            (128, &[0x80, 0x01]),
            (255, &[0xff, 0x01]),
            (2_147_483_647, &[0xff, 0xff, 0xff, 0xff, 0x07]),
            (-1, &[0xff, 0xff, 0xff, 0xff, 0x0f]),
            (-2_147_483_648, &[0x80, 0x80, 0x80, 0x80, 0x08]),
        ];
        for (value, bytes) in cases {
            let mut w = PacketWriter::new();
            w.varint(value);
            assert_eq!(w.as_bytes(), bytes, "encode {value}");
            assert_eq!(varint_len(value), bytes.len());
            assert_eq!(PacketReader::new(bytes).varint().unwrap(), value);
        }
        assert_eq!(
            PacketReader::new(&[0xff; 6]).varint(),
            Err(DecodeError::VarIntTooBig)
        );
        assert_eq!(
            PacketReader::new(&[0x80]).varint(),
            Err(DecodeError::UnexpectedEnd)
        );
    }

    #[test]
    fn varlong_vectors() {
        for v in [0i64, 1, -1, i64::MAX, i64::MIN, 300] {
            let mut w = PacketWriter::new();
            w.varlong(v);
            assert_eq!(PacketReader::new(w.as_bytes()).varlong().unwrap(), v);
        }
        let mut w = PacketWriter::new();
        w.varlong(-1);
        assert_eq!(w.as_bytes().len(), 10);
    }

    #[test]
    fn string_limits() {
        let mut w = PacketWriter::new();
        w.string("héllo").unwrap();
        assert_eq!(PacketReader::new(w.as_bytes()).string(5).unwrap(), "héllo");
        assert!(matches!(
            PacketReader::new(w.as_bytes()).string(4),
            Err(DecodeError::StringTooLong { len: 6, max: 4 })
        ));
        // Declared byte length above 4 * max is rejected before reading.
        assert!(matches!(
            PacketReader::new(&[0x15]).string(5),
            Err(DecodeError::StringBytesTooLong {
                len: 21,
                max_bytes: 20
            })
        ));
        assert_eq!(
            PacketReader::new(&[0xff, 0xff, 0xff, 0xff, 0x0f]).string(5),
            Err(DecodeError::NegativeStringLength)
        );
        assert!(PacketWriter::new().string(&"a".repeat(32768)).is_err());
    }

    #[test]
    fn position_round_trip() {
        for (x, y, z) in [
            (0, 0, 0),
            (-1, -1, -1),
            (33_554_431, 2047, -33_554_432),
            (100, 64, -200),
        ] {
            assert_eq!(unpack_position(pack_position(x, y, z)), (x, y, z));
        }
    }

    #[test]
    fn nbt_none_and_some() {
        let mut c = krol_nbt::Compound::new();
        c.insert("a", krol_nbt::Tag::Int(1));
        let mut w = PacketWriter::new();
        w.nbt(None).unwrap().nbt(Some(&c)).unwrap().u8(0x42);
        let mut r = PacketReader::new(w.as_bytes());
        assert_eq!(r.nbt().unwrap(), None);
        assert_eq!(r.nbt().unwrap(), Some(c));
        assert_eq!(r.u8().unwrap(), 0x42);
    }
}
