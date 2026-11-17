//! Mob heads (vanilla: `ItemSkull`, `BlockSkull`, `TileEntitySkull`, `NBTUtil` game
//! profiles): placing a head on the floor or a wall, its tile entity, and the head it
//! drops when it goes.
//!
//! TODO(M3): completing player heads' profiles through the profile cache and the
//! session server (`TileEntitySkull.updateGameprofile`): krol keeps the profile as
//! given.
//! TODO(M3): converting a `SkullOwner` name into a profile when an item is read from NBT
//! (`ItemSkull.updateItemStackNBT`).
//! TODO(M5): summoning the wither from wither skeleton skulls on soul sand.

use krol_nbt::{Compound, Tag, TagType};
use krol_protocol::packets::play::ItemStack;

use super::Game;
use super::placement::{self, offset};

/// `skull` (the block).
pub const SKULL: u16 = 144;
/// `skull` (the item).
pub const SKULL_ITEM: i16 = 397;
/// The block metadata bit that keeps a head from dropping (`NODROP`).
const NO_DROP: u16 = 8;
/// The player head's skull type.
const PLAYER: i32 = 3;
/// A head on the floor turns in 16 steps.
const ROTATIONS: f32 = 16.0;

/// A new head's tile entity (`TileEntitySkull.writeToNBT` before the item's values).
pub fn new_tile_entity(pos: (i32, i32, i32)) -> Compound {
    let mut c = Compound::new();
    c.insert("id", Tag::String("Skull".into()));
    c.insert("x", Tag::Int(pos.0));
    c.insert("y", Tag::Int(pos.1));
    c.insert("z", Tag::Int(pos.2));
    c.insert("SkullType", Tag::Byte(0));
    c.insert("Rot", Tag::Byte(0));
    c
}

/// `Long.parseLong(s, 16)`: an optional sign and hex digits, range-checked.
fn parse_hex_long(s: &str) -> Option<i64> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    i64::from_str_radix(s, 16).ok()
}

/// Java's `UUID.fromString` then `toString`: five dash-separated hex fields, printed in
/// canonical form.
fn normalize_uuid(text: &str) -> Option<String> {
    let parts: Vec<&str> = text.split('-').collect();
    if parts.len() != 5 {
        return None;
    }
    let mut v = [0i64; 5];
    for (slot, part) in v.iter_mut().zip(&parts) {
        *slot = parse_hex_long(part)?;
    }
    let most = (v[0] << 32) | ((v[1] & 0xffff) << 16) | (v[2] & 0xffff);
    let least = ((v[3] & 0xffff) << 48) | (v[4] & 0xffff_ffff_ffff);
    let (m, l) = (most as u64, least as u64);
    Some(format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        m >> 32,
        (m >> 16) & 0xffff,
        m & 0xffff,
        l >> 48,
        l & 0xffff_ffff_ffff
    ))
}

/// `NBTUtil.readGameProfileFromNBT` then `writeGameProfile`: the profile as a head
/// stores it (`Name`, a valid `Id`, `Properties` rebuilt), or nothing without a name
/// and an id.
pub fn read_profile(c: &Compound) -> Option<Compound> {
    let name = c
        .contains_key_of("Name", Some(TagType::String))
        .then(|| c.get_str("Name").to_owned())
        .filter(|n| !n.is_empty());
    let id_text = c
        .contains_key_of("Id", Some(TagType::String))
        .then(|| c.get_str("Id").to_owned())
        .filter(|i| !i.is_empty());
    if name.is_none() && id_text.is_none() {
        return None;
    }
    let mut out = Compound::new();
    if let Some(name) = name {
        out.insert("Name", Tag::String(name));
    }
    if let Some(id) = id_text.as_deref().and_then(normalize_uuid) {
        out.insert("Id", Tag::String(id));
    }
    if let Some(props) = c.get_compound("Properties") {
        let mut written = Compound::new();
        for (key, _) in props.iter() {
            let mut list = krol_nbt::List::new();
            for p in props.get_list(key, TagType::Compound).into_iter().flatten() {
                let Tag::Compound(p) = p else { continue };
                let mut e = Compound::new();
                e.insert("Value", Tag::String(p.get_str("Value").to_owned()));
                if p.contains_key_of("Signature", Some(TagType::String)) {
                    e.insert("Signature", Tag::String(p.get_str("Signature").to_owned()));
                }
                let _ = list.push(Tag::Compound(e));
            }
            // An empty property list leaves no entry in the profile's multimap.
            if !list.is_empty() {
                written.insert(key.clone(), Tag::List(list));
            }
        }
        if !written.is_empty() {
            out.insert("Properties", Tag::Compound(written));
        }
    }
    Some(out)
}

/// The player profile a player head item carries (`ItemSkull.onItemUse`): a
/// `SkullOwner` compound, or a non-empty `SkullOwner` name.
fn item_profile(stack: &ItemStack) -> Option<Compound> {
    let tag = stack.nbt.as_ref()?;
    if let Some(c) = tag.get_compound("SkullOwner") {
        return read_profile(c);
    }
    if tag.contains_key_of("SkullOwner", Some(TagType::String)) {
        let name = tag.get_str("SkullOwner");
        if !name.is_empty() {
            let mut c = Compound::new();
            c.insert("Name", Tag::String(name.to_owned()));
            return Some(c);
        }
    }
    None
}

/// The head a skull tile entity drops (`BlockSkull.breakBlock`): its type as damage,
/// and a player head's owner as `SkullOwner`.
fn dropped_head(te: &Compound) -> ItemStack {
    let kind = i32::from(te.get_i8("SkullType"));
    let owner = (kind == PLAYER)
        .then(|| te.get_compound("Owner").cloned())
        .flatten();
    let nbt = owner.map(|o| {
        let mut c = Compound::new();
        c.insert("SkullOwner", Tag::Compound(o));
        c
    });
    ItemStack {
        id: SKULL_ITEM,
        count: 1,
        damage: kind as i16,
        nbt,
    }
}

impl Game {
    /// `ItemSkull.onItemUse`: a head on top of or against the clicked block (or in place
    /// of a replaceable one), facing away from it; on the floor it turns with the player.
    /// Returns whether the item was used.
    pub(super) fn place_skull(&mut self, i: usize, pos: (i32, i32, i32), face: u8) -> bool {
        if face == placement::DOWN {
            return false;
        }
        let w = &mut self.level.overworld;
        let clicked = w.block(pos.0, pos.1, pos.2);
        let at = if super::blocks::is_replaceable(clicked) {
            pos
        } else {
            if !super::block_data::material(clicked >> 4).is_solid() {
                return false;
            }
            offset(pos, face)
        };
        if !(0..256).contains(&at.1) || !super::blocks::is_replaceable(w.block(at.0, at.1, at.2)) {
            return false;
        }
        let held = 36 + self.players[i].selected;
        let Some(stack) = self.players[i].inventory[held].clone() else {
            return false;
        };
        self.set_block_notify(at, SKULL << 4 | u16::from(face));
        let rotation = if face == placement::UP {
            let yaw = self.players[i].rotation.0;
            krol_java::math::floor_f64(f64::from(yaw * ROTATIONS / 360.0) + 0.5) & 15
        } else {
            0
        };
        // TileEntitySkull.setPlayerProfile / setType, then setSkullRotation.
        let profile = (stack.damage == PLAYER as i16)
            .then(|| item_profile(&stack))
            .flatten();
        if let Some(te) = self.level.overworld.tile_entity_mut(at) {
            te.insert("SkullType", Tag::Byte(stack.damage as i8));
            te.remove("Owner");
            if let Some(p) = profile {
                te.insert("Owner", Tag::Compound(p));
            }
            te.insert("Rot", Tag::Byte(rotation as i8));
        }
        self.count_item_use(i);
        let player = &mut self.players[i];
        if player.gamemode != 1
            && let Some(s) = &mut player.inventory[held]
        {
            s.count -= 1;
        }
        true
    }

    /// The head a skull about to be replaced drops, read before its tile entity goes
    /// (`breakBlock` drops it once the block changed), unless it is marked `NODROP`.
    pub(super) fn skull_drop(&self, pos: (i32, i32, i32), old: u16, new: u16) -> Option<ItemStack> {
        if old >> 4 != SKULL || new >> 4 == SKULL || old & NO_DROP != 0 {
            return None;
        }
        self.level.overworld.tile_entity(pos).map(dropped_head)
    }

    /// `BlockSkull.onBlockHarvested` in creative mode: the head is marked not to drop
    /// (quietly, flag 4).
    pub(super) fn skull_harvested_creative(&mut self, pos: (i32, i32, i32)) {
        let state = self.level.overworld.block(pos.0, pos.1, pos.2);
        if state >> 4 == SKULL {
            self.set_block_flags(pos, state | NO_DROP, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_like_nbt_util() {
        assert_eq!(
            normalize_uuid("1-2-3-4-5").as_deref(),
            Some("00000001-0002-0003-0004-000000000005")
        );
        assert_eq!(normalize_uuid("nope"), None);
        let mut c = Compound::new();
        c.insert("Name", Tag::String("Notch".into()));
        c.insert("Id", Tag::String("not-a-uuid".into()));
        let p = read_profile(&c);
        assert!(
            p.as_ref()
                .is_some_and(|p| p.contains_key("Name") && !p.contains_key("Id"))
        );
        assert!(read_profile(&Compound::new()).is_none());
        let mut te = new_tile_entity((0, 0, 0));
        te.insert("SkullType", Tag::Byte(3));
        te.insert("Owner", Tag::Compound(c));
        let head = dropped_head(&te);
        assert_eq!((head.id, head.damage), (SKULL_ITEM, 3));
        assert!(head.nbt.is_some_and(|n| n.contains_key("SkullOwner")));
    }
}
