//! Big-endian binary NBT encoding.

use std::io::{Read, Write};

use krol_java::modified_utf8;

use crate::{Compound, Error, List, Result, Tag, TagType};

/// Maximum nesting depth of compounds and lists accepted when reading, as in vanilla.
pub const MAX_DEPTH: usize = 512;

/// Reads a named root compound, returning its name and contents.
///
/// This is the format of `level.dat`, player data, region chunks (after decompression)
/// and NBT in protocol-47 packets.
///
/// # Errors
///
/// [`Error::RootNotCompound`] if the root tag is not a compound, plus any decoding or I/O
/// error.
pub fn read(reader: &mut impl Read) -> Result<(String, Compound)> {
    let ty = TagType::from_id(read_u8(reader)?)?;
    if ty != TagType::Compound {
        return Err(Error::RootNotCompound);
    }
    let name = read_string(reader)?;
    let root = read_compound(reader, 0)?;
    Ok((name, root))
}

/// Writes `root` as a named root compound called `name`.
///
/// # Errors
///
/// [`Error::StringTooLong`] for strings over 65535 encoded bytes, plus any I/O error.
pub fn write(writer: &mut impl Write, name: &str, root: &Compound) -> Result<()> {
    writer.write_all(&[TagType::Compound.id()])?;
    write_string(writer, name)?;
    write_compound(writer, root)
}

fn read_payload(reader: &mut impl Read, ty: TagType, depth: usize) -> Result<Tag> {
    Ok(match ty {
        TagType::End => return Err(Error::InvalidTagType(0)),
        TagType::Byte => Tag::Byte(read_u8(reader)? as i8),
        TagType::Short => Tag::Short(i16::from_be_bytes(read_array(reader)?)),
        TagType::Int => Tag::Int(read_i32(reader)?),
        TagType::Long => Tag::Long(i64::from_be_bytes(read_array(reader)?)),
        TagType::Float => Tag::Float(f32::from_be_bytes(read_array(reader)?)),
        TagType::Double => Tag::Double(f64::from_be_bytes(read_array(reader)?)),
        TagType::ByteArray => {
            let len = read_len(reader)?;
            let mut bytes = Vec::new();
            reader.take(len as u64).read_to_end(&mut bytes)?;
            if bytes.len() != len {
                return Err(unexpected_eof());
            }
            Tag::ByteArray(bytes.into_iter().map(|b| b as i8).collect())
        }
        TagType::String => Tag::String(read_string(reader)?),
        TagType::List => Tag::List(read_list(reader, depth)?),
        TagType::Compound => Tag::Compound(read_compound(reader, depth)?),
        TagType::IntArray => {
            let len = read_len(reader)?;
            // Grow as data arrives so a bogus length cannot force a huge allocation.
            let mut ints = Vec::with_capacity(len.min(4096));
            for _ in 0..len {
                ints.push(read_i32(reader)?);
            }
            Tag::IntArray(ints)
        }
    })
}

fn read_compound(reader: &mut impl Read, depth: usize) -> Result<Compound> {
    if depth > MAX_DEPTH {
        return Err(Error::TooDeep);
    }
    let mut compound = Compound::new();
    loop {
        let ty = TagType::from_id(read_u8(reader)?)?;
        if ty == TagType::End {
            return Ok(compound);
        }
        let name = read_string(reader)?;
        let tag = read_payload(reader, ty, depth + 1)?;
        compound.insert(name, tag);
    }
}

fn read_list(reader: &mut impl Read, depth: usize) -> Result<List> {
    if depth > MAX_DEPTH {
        return Err(Error::TooDeep);
    }
    let ty = TagType::from_id(read_u8(reader)?)?;
    // Vanilla quirk: a negative list length reads as an empty list (no error), unlike
    // negative array lengths.
    let len = read_i32(reader)?;
    let mut list = List::new();
    for _ in 0..len.max(0) {
        list.push(read_payload(reader, ty, depth + 1)?)?;
    }
    Ok(list)
}

fn write_payload(writer: &mut impl Write, tag: &Tag) -> Result<()> {
    match tag {
        Tag::Byte(v) => writer.write_all(&v.to_be_bytes())?,
        Tag::Short(v) => writer.write_all(&v.to_be_bytes())?,
        Tag::Int(v) => writer.write_all(&v.to_be_bytes())?,
        Tag::Long(v) => writer.write_all(&v.to_be_bytes())?,
        Tag::Float(v) => writer.write_all(&v.to_be_bytes())?,
        Tag::Double(v) => writer.write_all(&v.to_be_bytes())?,
        Tag::ByteArray(bytes) => {
            write_len(writer, bytes.len())?;
            let raw: Vec<u8> = bytes.iter().map(|&b| b as u8).collect();
            writer.write_all(&raw)?;
        }
        Tag::String(s) => write_string(writer, s)?,
        Tag::List(list) => {
            writer.write_all(&[list.element_type().id()])?;
            write_len(writer, list.len())?;
            for item in list {
                write_payload(writer, item)?;
            }
        }
        Tag::Compound(c) => write_compound(writer, c)?,
        Tag::IntArray(ints) => {
            write_len(writer, ints.len())?;
            for v in ints {
                writer.write_all(&v.to_be_bytes())?;
            }
        }
    }
    Ok(())
}

fn write_compound(writer: &mut impl Write, compound: &Compound) -> Result<()> {
    for (name, tag) in compound {
        writer.write_all(&[tag.tag_type().id()])?;
        write_string(writer, name)?;
        write_payload(writer, tag)?;
    }
    writer.write_all(&[TagType::End.id()])?;
    Ok(())
}

fn read_string(reader: &mut impl Read) -> Result<String> {
    let len = u16::from_be_bytes(read_array(reader)?);
    let mut buf = vec![0; usize::from(len)];
    reader.read_exact(&mut buf)?;
    Ok(modified_utf8::decode(&buf)?)
}

fn write_string(writer: &mut impl Write, s: &str) -> Result<()> {
    let bytes = modified_utf8::encode(s);
    let len = u16::try_from(bytes.len()).map_err(|_| Error::StringTooLong(bytes.len()))?;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(&bytes)?;
    Ok(())
}

fn read_len(reader: &mut impl Read) -> Result<usize> {
    let len = read_i32(reader)?;
    usize::try_from(len).map_err(|_| Error::NegativeLength(len))
}

fn write_len(writer: &mut impl Write, len: usize) -> Result<()> {
    let len = i32::try_from(len)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "NBT array too long"))?;
    writer.write_all(&len.to_be_bytes())?;
    Ok(())
}

fn read_u8(reader: &mut impl Read) -> Result<u8> {
    Ok(read_array::<1>(reader)?[0])
}

fn read_i32(reader: &mut impl Read) -> Result<i32> {
    Ok(i32::from_be_bytes(read_array(reader)?))
}

fn read_array<const N: usize>(reader: &mut impl Read) -> Result<[u8; N]> {
    let mut buf = [0; N];
    reader.read_exact(&mut buf)?;
    Ok(buf)
}

fn unexpected_eof() -> Error {
    std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into()
}
