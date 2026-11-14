use krol_nbt::{Compound, Error, List, Tag, TagType};

/// The canonical "hello world" example from the NBT specification.
const HELLO_WORLD: &[u8] = &[
    0x0a, 0x00, 0x0b, b'h', b'e', b'l', b'l', b'o', b' ', b'w', b'o', b'r', b'l', b'd', 0x08, 0x00,
    0x04, b'n', b'a', b'm', b'e', 0x00, 0x09, b'B', b'a', b'n', b'a', b'n', b'r', b'a', b'm', b'a',
    0x00,
];

#[test]
fn reads_and_writes_spec_example() {
    let (name, root) = krol_nbt::read(&mut &HELLO_WORLD[..]).unwrap();
    assert_eq!(name, "hello world");
    assert_eq!(root.get_str("name"), "Bananrama");

    let mut out = Vec::new();
    krol_nbt::write(&mut out, &name, &root).unwrap();
    assert_eq!(out, HELLO_WORLD);
}

#[test]
fn round_trips_every_type_in_order() {
    let mut inner = Compound::new();
    inner.insert("nested", Tag::String("\0日本\u{1F600}".into()));
    let mut list = List::new();
    list.push(Tag::Compound(inner.clone())).unwrap();
    list.push(Tag::Compound(Compound::new())).unwrap();

    let mut root = Compound::new();
    root.insert("z_byte", Tag::Byte(-1));
    root.insert("short", Tag::Short(i16::MIN));
    root.insert("int", Tag::Int(i32::MAX));
    root.insert("long", Tag::Long(i64::MIN));
    root.insert("float", Tag::Float(f32::from_bits(0x7fc0_0001)));
    root.insert("double", Tag::Double(-0.0));
    root.insert("bytes", Tag::ByteArray(vec![-128, 0, 127]));
    root.insert("list", Tag::List(list));
    root.insert("empty_list", Tag::List(List::new()));
    root.insert("compound", Tag::Compound(inner));
    root.insert("ints", Tag::IntArray(vec![1, -1, i32::MIN]));

    let mut first = Vec::new();
    krol_nbt::write(&mut first, "root", &root).unwrap();
    let (name, decoded) = krol_nbt::read(&mut first.as_slice()).unwrap();
    assert_eq!(name, "root");
    let keys: Vec<_> = decoded.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys[0], "z_byte", "file order is preserved");

    let mut second = Vec::new();
    krol_nbt::write(&mut second, &name, &decoded).unwrap();
    assert_eq!(first, second, "re-encoding is byte-identical");
}

#[test]
fn empty_list_is_written_with_end_type() {
    // A list read as "empty list of compounds" is written back with element type End.
    let input = [
        0x0a, 0x00, 0x00, 0x09, 0x00, 0x01, b'l', 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    let (_, root) = krol_nbt::read(&mut &input[..]).unwrap();
    let mut out = Vec::new();
    krol_nbt::write(&mut out, "", &root).unwrap();
    assert_eq!(out[7], TagType::End.id());
}

#[test]
fn negative_list_length_reads_as_empty() {
    let input = [
        0x0a, 0x00, 0x00, 0x09, 0x00, 0x01, b'l', 0x03, 0xff, 0xff, 0xff, 0xff, 0x00,
    ];
    let (_, root) = krol_nbt::read(&mut &input[..]).unwrap();
    assert!(root.get_list("l", TagType::Int).unwrap().is_empty());
}

#[test]
fn negative_array_length_is_an_error() {
    let input = [
        0x0a, 0x00, 0x00, 0x07, 0x00, 0x01, b'a', 0xff, 0xff, 0xff, 0xfe, 0x00,
    ];
    assert!(matches!(
        krol_nbt::read(&mut &input[..]),
        Err(Error::NegativeLength(-2))
    ));
}

#[test]
fn rejects_non_compound_root_and_bad_types() {
    assert!(matches!(
        krol_nbt::read(&mut &[0x08, 0x00, 0x00, 0x00, 0x00][..]),
        Err(Error::RootNotCompound)
    ));
    assert!(matches!(
        krol_nbt::read(&mut &[0x0a, 0x00, 0x00, 0x0c][..]),
        Err(Error::InvalidTagType(12))
    ));
}

#[test]
fn truncated_input_is_an_io_error() {
    assert!(matches!(
        krol_nbt::read(&mut &HELLO_WORLD[..HELLO_WORLD.len() - 3]),
        Err(Error::Io(_))
    ));
    // A huge declared byte-array length must fail cleanly, not allocate.
    let input = [
        0x0a, 0x00, 0x00, 0x07, 0x00, 0x01, b'a', 0x7f, 0xff, 0xff, 0xff,
    ];
    assert!(matches!(krol_nbt::read(&mut &input[..]), Err(Error::Io(_))));
}

fn nested_compounds(levels: usize) -> Vec<u8> {
    let mut bytes = vec![0x0a, 0x00, 0x00];
    for _ in 0..levels {
        bytes.extend_from_slice(&[0x0a, 0x00, 0x00]);
    }
    bytes.extend(std::iter::repeat_n(0x00, levels + 1));
    bytes
}

#[test]
fn depth_limit_matches_vanilla() {
    // Root is depth 0; vanilla rejects a compound once its depth exceeds 512.
    assert!(krol_nbt::read(&mut nested_compounds(512).as_slice()).is_ok());
    assert!(matches!(
        krol_nbt::read(&mut nested_compounds(513).as_slice()),
        Err(Error::TooDeep)
    ));
}

#[test]
fn string_too_long_is_rejected_on_write() {
    let mut root = Compound::new();
    root.insert("s", Tag::String("a".repeat(65536)));
    assert!(matches!(
        krol_nbt::write(&mut Vec::new(), "", &root),
        Err(Error::StringTooLong(65536))
    ));
}

/// Compounds written by vanilla 1.8.8 (whose `NBTTagCompound` is a `HashMap`) come back
/// byte for byte: the file order is the map's iteration order.
#[test]
fn vanilla_file_round_trips_byte_for_byte() {
    use std::io::Read;
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../parity/fixtures/worlds/flat/level.dat"
    );
    let gz = std::fs::read(path).expect("fixture level.dat");
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(&gz[..])
        .read_to_end(&mut raw)
        .expect("gzip");
    let (name, root) = krol_nbt::read(&mut &raw[..]).expect("nbt");
    let mut out = Vec::new();
    krol_nbt::write(&mut out, &name, &root).expect("write");
    assert_eq!(out, raw);
}
