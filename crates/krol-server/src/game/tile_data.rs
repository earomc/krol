//! Loading tile entity data from a command's data tag (vanilla: each tile entity's
//! `readFromNBT`, as `/setblock` calls it on the block's fresh tile entity). krol keeps
//! tile entities as NBT, so reading a tag means keeping what `writeToNBT` would write
//! after `readFromNBT`: the known fields with their types, valid item slots, and nothing
//! else.

use krol_nbt::{Compound, List, Tag, TagType};

use crate::chat::Component;

/// The inventory size of each tile entity kind with items.
fn inventory_size(id: &str) -> Option<usize> {
    Some(match id {
        "Chest" => 27,
        "Furnace" => 3,
        "Trap" | "Dropper" => 9,
        "Hopper" => 5,
        "Cauldron" => 4,
        _ => return None,
    })
}

/// `fresh` is the tile entity the block just got (its `id` and position); returns it
/// after reading `tag` (whose own position and id are ignored).
///
/// TODO(M4): the tile entities krol does not create yet (skulls, banners, beacons,
/// spawners, command blocks, flower pots, daylight sensors) ignore the tag.
pub fn read_tile_entity(fresh: &Compound, tag: &Compound) -> Compound {
    let id = fresh.get_str("id").to_owned();
    // TileEntity.writeToNBT.
    let mut out = Compound::new();
    out.insert("id", Tag::String(id.clone()));
    for k in ["x", "y", "z"] {
        out.insert(k, Tag::Int(fresh.get_i32(k)));
    }
    let custom_name = |out: &mut Compound| {
        // hasKey("CustomName", 8), written back only when not empty.
        if let Some(Tag::String(name)) = tag.get("CustomName")
            && !name.is_empty()
        {
            out.insert("CustomName", Tag::String(name.clone()));
        }
    };
    if let Some(size) = inventory_size(&id) {
        // TileEntityLockable: LockCode.fromNBT, always written.
        let lock = match tag.get("Lock") {
            Some(Tag::String(lock)) => lock.clone(),
            _ => String::new(),
        };
        out.insert("Lock", Tag::String(lock));
        match id.as_str() {
            "Furnace" => {
                for k in ["BurnTime", "CookTime", "CookTimeTotal"] {
                    out.insert(k, Tag::Short(tag.get_i16(k)));
                }
            }
            "Cauldron" => {
                out.insert("BrewTime", Tag::Short(tag.get_i16("BrewTime")));
            }
            _ => {}
        }
        out.insert("Items", Tag::List(read_items(tag, size)));
        if id == "Hopper" {
            out.insert(
                "TransferCooldown",
                Tag::Int(tag.get_i32("TransferCooldown")),
            );
        }
        custom_name(&mut out);
        return out;
    }
    match id.as_str() {
        "Sign" => return read_sign(out, tag),
        "Music" => {
            /// The highest note (`TileEntityNote` clamps to two octaves).
            const MAX_NOTE: i8 = 24;
            out.insert("note", Tag::Byte(tag.get_i8("note").clamp(0, MAX_NOTE)));
        }
        "Comparator" => {
            out.insert("OutputSignal", Tag::Int(tag.get_i32("OutputSignal")));
        }
        "EnchantTable" => custom_name(&mut out),
        "EnderChest" => {}
        // Kinds without a reader keep what they had.
        _ => return fresh.clone(),
    }
    out
}

/// The `Items` list as read into `size` slots and written back in slot order.
fn read_items(tag: &Compound, size: usize) -> List {
    let mut slots: Vec<Option<Compound>> = vec![None; size];
    if let Some(list) = tag.get_list("Items", TagType::Compound) {
        for entry in list.iter() {
            let Tag::Compound(c) = entry else { continue };
            let slot = usize::from(c.get_i8("Slot") as u8);
            if slot < size {
                slots[slot] = super::player::item_from_nbt(c)
                    .as_ref()
                    .and_then(super::player::item_to_nbt);
            }
        }
    }
    let mut list = List::new();
    for (slot, item) in slots.into_iter().enumerate() {
        let Some(item) = item else { continue };
        let mut entry = Compound::new();
        entry.insert("Slot", Tag::Byte(slot as u8 as i8));
        for (k, v) in item.iter() {
            entry.insert(k.clone(), v.clone());
        }
        let _ = list.push(Tag::Compound(entry));
    }
    list
}

/// `TileEntitySign.readFromNBT`: each line parsed as chat JSON (literal text when it
/// is not), with score and selector components resolved; then the command stats.
fn read_sign(mut out: Compound, tag: &Compound) -> Compound {
    for k in 1..=4 {
        let key = format!("Text{k}");
        let raw = match tag.get(&key) {
            Some(Tag::String(s)) => s.as_str(),
            _ => "",
        };
        out.insert(key, Tag::String(sign_line_json(raw)));
    }
    if let Some(stats) = tag.get_compound("CommandStats") {
        // CommandResultStats: a name and an objective per result type, both non-empty.
        const TYPES: [&str; 5] = [
            "SuccessCount",
            "AffectedBlocks",
            "AffectedEntities",
            "AffectedItems",
            "QueryResult",
        ];
        let mut kept = Compound::new();
        for t in TYPES {
            let (name_key, objective_key) = (format!("{t}Name"), format!("{t}Objective"));
            if let (Some(Tag::String(name)), Some(Tag::String(objective))) =
                (stats.get(&name_key), stats.get(&objective_key))
                && !name.is_empty()
                && !objective.is_empty()
            {
                kept.insert(name_key, Tag::String(name.clone()));
                kept.insert(objective_key, Tag::String(objective.clone()));
            }
        }
        if !kept.is_empty() {
            out.insert("CommandStats", Tag::Compound(kept));
        }
    }
    out
}

/// One sign line read and written back: chat JSON is processed and re-serialized, other
/// text becomes a literal text component.
///
/// Vanilla quirk: an empty document or JSON `null` parses to a null component, which the
/// processor passes through and the serializer writes as `null`.
pub fn sign_line_json(raw: &str) -> String {
    /// What Gson writes for a null component.
    const NULL: &str = "null";
    if raw.trim().is_empty() || raw.trim() == NULL {
        return NULL.to_owned();
    }
    match Component::parse_json(raw) {
        Ok(c) => process_component(c).to_json_string(),
        Err(_) => Component::text(raw).to_json_string(),
    }
}

/// `ChatComponentProcessor.processComponent` for a sign: text and translations are kept
/// as they are, scores resolve against the scoreboard and selectors against the world.
///
/// TODO(M5): krol has no scoreboard or entity selectors yet: scores keep their stored
/// value (vanilla reads the score when the value is empty) and selectors become empty
/// text (what vanilla shows when nothing matches).
fn process_component(c: Component) -> Component {
    use crate::chat::{Arg, Content};
    let content = match c.content {
        Content::Selector(_) => Content::Text(String::new()),
        Content::Translate { key, with } => Content::Translate {
            key,
            with: with
                .into_iter()
                .map(|a| match a {
                    Arg::Component(c) => Arg::Component(process_component(c)),
                    other => other,
                })
                .collect(),
        },
        other => other,
    };
    Component {
        content,
        style: c.style,
        extra: c.extra.into_iter().map(process_component).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(id: &str) -> Compound {
        let mut c = Compound::new();
        c.insert("id", Tag::String(id.into()));
        c.insert("x", Tag::Int(1));
        c.insert("y", Tag::Int(2));
        c.insert("z", Tag::Int(3));
        c
    }

    fn tag(s: &str) -> Compound {
        krol_nbt::text::parse_data_tag(s, &mut |_| {}).unwrap()
    }

    #[test]
    fn chest_keeps_valid_slots() {
        let out = read_tile_entity(
            &fresh("Chest"),
            &tag(
                "{x:9,Items:[{Slot:3b,id:\"minecraft:stone\",Count:2b},{Slot:30b,id:\"minecraft:dirt\",Count:1b},{Slot:1b,id:\"nope\",Count:1b}],CustomName:\"Box\",Junk:1}",
            ),
        );
        assert_eq!(out.get_i32("x"), 1);
        assert!(!out.contains_key("Junk"));
        assert_eq!(out.get_str("CustomName"), "Box");
        assert_eq!(out.get_str("Lock"), "");
        let items = out.get_list("Items", TagType::Compound).unwrap();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn sign_lines() {
        let out = read_tile_entity(
            &fresh("Sign"),
            &tag(
                "{Text1:\"{\\\"text\\\":\\\"hi\\\"}\",Text2:\"plain words\",Text3:\"\\\"q\\\"\",Text4:\"[\\\"a\\\",\\\"b\\\"]\"}",
            ),
        );
        assert_eq!(out.get_str("Text1"), "\"hi\"");
        assert_eq!(out.get_str("Text2"), "\"plain words\"");
        assert_eq!(out.get_str("Text3"), "\"q\"");
        let partial = read_tile_entity(&fresh("Sign"), &tag("{Text1:\"\\\"a\\\"\"}"));
        assert_eq!(partial.get_str("Text1"), "\"a\"");
        assert_eq!(partial.get_str("Text2"), "null");
    }

    #[test]
    fn note_is_clamped() {
        let out = read_tile_entity(&fresh("Music"), &tag("{note:30b}"));
        assert_eq!(out.get_i8("note"), 24);
    }
}
