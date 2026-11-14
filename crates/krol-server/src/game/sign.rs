//! Signs and the tile entity data clients are sent (vanilla: `ItemSign`,
//! `TileEntitySign`, `NetHandlerPlayServer.processUpdateSign`,
//! `TileEntity.getDescriptionPacket` and its overrides).
//!
//! TODO(M4): sign text with score and selector components is not resolved
//! (`ChatComponentProcessor`); other tile entities are described from their stored NBT
//! rather than re-written by vanilla's `writeToNBT`.

use krol_nbt::{Compound, Tag};
use krol_protocol::packets::play::ClientboundPlay;

use super::Game;
use super::ids::block;
use super::placement::{self, offset};
use crate::chat::Component;

/// `S35PacketUpdateTileEntity` actions per tile entity kind.
mod action {
    pub const MOB_SPAWNER: u8 = 1;
    pub const COMMAND_BLOCK: u8 = 2;
    pub const BEACON: u8 = 3;
    pub const SKULL: u8 = 4;
    pub const FLOWER_POT: u8 = 5;
    pub const BANNER: u8 = 6;
}

/// Standing sign rotations per full turn.
const SIGN_ROTATIONS: f32 = 16.0;

/// The four lines of a sign tile entity as chat JSON (`TileEntitySign.readFromNBT`
/// then the serializer): text that is not chat JSON becomes literal text.
pub fn sign_lines(te: &Compound) -> [String; 4] {
    let line = |k: usize| {
        let raw = te.get_str(&format!("Text{k}"));
        Component::parse_json(raw)
            .unwrap_or_else(|_| Component::text(raw))
            .to_json_string()
    };
    [line(1), line(2), line(3), line(4)]
}

/// `getDescriptionPacket`: the packet that shows a tile entity to clients, if its kind
/// has one.
pub fn description(te: &Compound) -> Option<ClientboundPlay> {
    let pos = (te.get_i32("x"), te.get_i32("y"), te.get_i32("z"));
    let action = match te.get_str("id") {
        "Sign" => {
            return Some(ClientboundPlay::UpdateSign {
                pos,
                lines: sign_lines(te),
            });
        }
        "MobSpawner" => action::MOB_SPAWNER,
        "Control" => action::COMMAND_BLOCK,
        "Beacon" => action::BEACON,
        "Skull" => action::SKULL,
        "FlowerPot" => action::FLOWER_POT,
        "Banner" => action::BANNER,
        _ => return None,
    };
    let mut nbt = te.clone();
    match action {
        action::MOB_SPAWNER => {
            nbt.remove("SpawnPotentials");
        }
        action::FLOWER_POT => {
            // The flower's item goes as a numeric id.
            let id = match te.get("Item") {
                Some(Tag::String(name)) => krol_world::items::id_for_name(name).unwrap_or(0),
                Some(t) => t.as_i16().unwrap_or(0),
                None => 0,
            };
            nbt.remove("Item");
            nbt.insert("Item", Tag::Int(i32::from(id)));
        }
        _ => {}
    }
    Some(ClientboundPlay::UpdateTileEntity {
        pos,
        action,
        nbt: Some(nbt),
    })
}

/// `Material.isSolid`.
pub(super) fn solid_material(state: u16) -> bool {
    super::block_data::material(state >> 4).is_solid()
}

impl Game {
    /// The description packets of the tile entities in the given chunks, chunk by chunk
    /// (`getTileEntitiesIn` per chunk, then `sendTileEntityUpdate`).
    pub(super) fn chunk_descriptions(&mut self, chunks: &[(i32, i32)]) -> Vec<ClientboundPlay> {
        let mut out = Vec::new();
        for &(cx, cz) in chunks {
            for t in self.level.overworld.chunk(cx, cz).tile_entities.iter() {
                if let Tag::Compound(c) = t
                    && let Some(p) = description(c)
                {
                    out.push(p);
                }
            }
        }
        out
    }

    /// The description packet of the tile entity at `pos`, if any.
    pub(super) fn block_description(&mut self, pos: (i32, i32, i32)) -> Option<ClientboundPlay> {
        let block = self.level.overworld.block(pos.0, pos.1, pos.2) >> 4;
        if !super::world::has_tile_entity(block) {
            return None;
        }
        self.level.overworld.tile_entity(pos).and_then(description)
    }

    /// `ItemSign.onItemUse`: a standing sign on top of a block, a wall sign on its side;
    /// the placer gets the editor. Returns whether the sign item was used.
    pub(super) fn place_sign(&mut self, i: usize, pos: (i32, i32, i32), face: u8) -> bool {
        let w = &mut self.level.overworld;
        if face == placement::DOWN || !solid_material(w.block(pos.0, pos.1, pos.2)) {
            return false;
        }
        let at = offset(pos, face);
        // BlockSign.canPlaceBlockAt: replaceable, no cactus next to it.
        if !(0..256).contains(&at.1)
            || !super::blocks::is_replaceable(w.block(at.0, at.1, at.2))
            || [
                placement::NORTH,
                placement::SOUTH,
                placement::WEST,
                placement::EAST,
            ]
            .iter()
            .any(|&f| {
                let n = offset(at, f);
                w.block(n.0, n.1, n.2) >> 4 == block::CACTUS
            })
        {
            return false;
        }
        let state = if face == placement::UP {
            let yaw = self.players[i].rotation.0;
            let rotation =
                krol_java::math::floor_f64(f64::from((yaw + 180.0) * SIGN_ROTATIONS / 360.0) + 0.5)
                    & 15;
            block::STANDING_SIGN << 4 | rotation as u16
        } else {
            block::WALL_SIGN << 4 | u16::from(face)
        };
        self.set_block_notify(at, state);
        self.count_item_use(i);
        let player = &mut self.players[i];
        if player.gamemode != 1 {
            let held = 36 + player.selected;
            if let Some(s) = &mut player.inventory[held] {
                s.count -= 1;
            }
        }
        // TODO(M4): BlockEntityTag on the item (setTileEntityNBT) instead of the editor.
        // EntityPlayerMP.openEditSign.
        let conn = self.players[i].conn_id;
        self.sign_editors.insert(at, conn);
        self.players[i].send(ClientboundPlay::SignEditorOpen { pos: at });
        true
    }

    /// Vanilla `processUpdateSign`: the sign's editor sets its lines, without formatting
    /// codes; everyone watching gets the new text with the next block update flush.
    pub(super) fn update_sign(&mut self, i: usize, pos: (i32, i32, i32), lines: &[String; 4]) {
        let w = &mut self.level.overworld;
        if !w.is_chunk_loaded(pos.0 >> 4, pos.2 >> 4) {
            return;
        }
        if w.tile_entity(pos).is_none_or(|c| c.get_str("id") != "Sign") {
            return;
        }
        let conn = self.players[i].conn_id;
        if self.sign_editors.get(&pos) != Some(&conn) {
            tracing::warn!(
                "Player {} just tried to change non-editable sign",
                self.players[i].profile.name
            );
            return;
        }
        let texts: Vec<String> = lines
            .iter()
            .map(|l| {
                let unformatted = Component::parse_json(l)
                    .map(|c| c.unformatted())
                    .unwrap_or_default();
                Component::text(crate::chat::strip_formatting(&unformatted)).to_json_string()
            })
            .collect();
        let w = &mut self.level.overworld;
        if let Some(te) = w.tile_entity_mut(pos) {
            for (k, t) in texts.into_iter().enumerate() {
                te.insert(format!("Text{}", k + 1), Tag::String(t));
            }
        }
        w.mark_block_for_update(pos.0, pos.1, pos.2);
    }
}
