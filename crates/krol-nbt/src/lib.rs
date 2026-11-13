//! The Named Binary Tag (NBT) format as used by Minecraft 1.8.
//!
//! NBT is used for world files (`level.dat`, region chunks, player data) and inside
//! network packets (item stacks, tile entity data). This crate provides:
//!
//! - an in-memory model: [`Tag`], [`Compound`] and [`List`],
//! - big-endian binary encoding and decoding ([`read`], [`write`]) with vanilla's
//!   nesting-depth limit.
//!
//! 1.8 knows tag types 0–11; there is no long array.
//!
//! ```
//! use krol_nbt::{Compound, Tag};
//!
//! let mut root = Compound::new();
//! root.insert("Health", Tag::Short(20));
//! root.insert("Name", Tag::String("Steve".into()));
//!
//! let mut bytes = Vec::new();
//! krol_nbt::write(&mut bytes, "", &root).unwrap();
//! let (name, decoded) = krol_nbt::read(&mut bytes.as_slice()).unwrap();
//! assert_eq!(name, "");
//! assert_eq!(decoded, root);
//! ```

#![deny(missing_docs)]

mod codec;
mod error;
mod tag;

pub use codec::{MAX_DEPTH, read, write};
pub use error::{Error, Result};
pub use tag::{Compound, List, Tag, TagType};
