//! Maps (vanilla: `ItemEmptyMap`, `ItemMap`, `MapData`, `MapStorage`, `S34PacketMaps`):
//! creating a map, its data in `data/map_<id>.dat` and the id counter in
//! `data/idcounts.dat`, drawing the world onto a held map column by column, the player
//! markers, and the map packets.
//!
//! Maps in item frames get their viewers and packets from the tracker
//! ([`Game::update_framed_map`]), with an icon for the frame.
//!
//! TODO(M4): zoomed maps taken
//! from the crafting grid by shift-click (`onCreated` runs only for the cursor), chunks the map reaches that are not loaded (vanilla
//! loads them), ties between colors when a pixel covers several blocks (vanilla picks by
//! `MapColor` identity hash order), the nether's maps.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use krol_nbt::{Compound, Tag, TagType};
use krol_protocol::packets::play::{ClientboundPlay, ItemStack, MapIcon};
use tracing::error;

use super::Game;

/// `map` (the empty map item).
pub const EMPTY_MAP: i16 = 395;
/// `filled_map`.
pub const FILLED_MAP: i16 = 358;
/// A map is 128 pixels square.
const SIZE: i32 = 128;
const PIXELS: usize = (SIZE * SIZE) as usize;
/// `MapColor.airColor` and `waterColor`.
const AIR_COLOR: u8 = 0;
const WATER_COLOR: u8 = 12;
/// Icons further out than this many pixels from the center sit on the edge.
const EDGE: f32 = 63.0;
/// Icons are dropped beyond this many pixels.
const ICON_RANGE: f32 = 320.0;
/// The icon type of a player beyond the edge.
const OFF_MAP_ICON: u8 = 6;
/// A viewer gets the icons every fifth packet call without color changes.
const ICON_PERIOD: i32 = 5;
/// The largest scale.
const MAX_SCALE: i8 = 4;
/// A map made without data: centered on the spawn at this scale.
const LOST_MAP_SCALE: i8 = 3;
/// Icon types: a player, and an item frame showing the map.
const PLAYER_ICON: u8 = 0;
const FRAME_ICON: u8 = 1;

/// An item frame showing a filled map, as its map icon needs it.
#[derive(Debug, Clone, Copy)]
pub struct FramedMap {
    /// The frame's entity id (its icon is named after it).
    pub entity_id: i32,
    /// Its hanging position.
    pub tile: (i32, i32, i32),
    /// Its facing as a horizontal index.
    pub facing: u8,
}

/// The icon name of an item frame (`"frame-" + getEntityId()`).
fn frame_icon_name(entity_id: i32) -> String {
    format!("frame-{entity_id}")
}

/// `Block.getMapColor` for every block state, extracted from vanilla
/// (`tools/datagen/MapColorDump.java`).
fn map_color(state: u16) -> u8 {
    static TABLE: OnceLock<Vec<[u8; 16]>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = vec![[0u8; 16]; 4096];
        for line in include_str!("../../data/map_colors.txt").lines() {
            let mut it = line
                .split_whitespace()
                .filter_map(|v| v.parse::<i32>().ok());
            let Some(id) = it.next() else { continue };
            let Some(row) = t.get_mut(id as usize) else {
                continue;
            };
            for (k, v) in it.enumerate().take(16) {
                row[k] = v.max(0) as u8;
            }
        }
        t
    });
    table
        .get(usize::from(state >> 4))
        .map_or(AIR_COLOR, |row| row[usize::from(state & 15)])
}

/// One player looking at a map (`MapData.MapInfo`): the changed rectangle not yet sent.
#[derive(Debug, Clone)]
struct Viewer {
    entity_id: i32,
    /// Changes wait to be sent (`field_176105_d`; a new viewer gets the whole map).
    pending: bool,
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
    /// Packet calls without changes (`field_176109_i`).
    calls: i32,
    /// Updates of the held map (`field_82569_d`): picks the columns redrawn.
    step: i32,
    /// The player entity it belongs to died and respawned as a new entity
    /// (`entityplayerObj.isDead`): it no longer matches the player and is removed by
    /// the next viewer update.
    stale: bool,
}

impl Viewer {
    fn new(entity_id: i32) -> Self {
        Self {
            entity_id,
            pending: true,
            min_x: 0,
            min_y: 0,
            max_x: SIZE - 1,
            max_y: SIZE - 1,
            calls: 0,
            step: 0,
            stale: false,
        }
    }

    /// `MapInfo.update`: the changed rectangle grows to cover the pixel.
    fn update(&mut self, x: i32, y: i32) {
        if self.pending {
            self.min_x = self.min_x.min(x);
            self.min_y = self.min_y.min(y);
            self.max_x = self.max_x.max(x);
            self.max_y = self.max_y.max(y);
        } else {
            self.pending = true;
            (self.min_x, self.min_y, self.max_x, self.max_y) = (x, y, x, y);
        }
    }
}

/// One map's data (`MapData`).
#[derive(Debug, Clone)]
pub struct MapData {
    x_center: i32,
    z_center: i32,
    dimension: i8,
    /// 0 to 4: each pixel covers `2^scale` blocks square.
    pub scale: i8,
    colors: Vec<u8>,
    viewers: Vec<Viewer>,
    /// Icons by name, in the order they first appeared (`LinkedHashMap`).
    icons: Vec<(String, MapIcon)>,
    dirty: bool,
}

impl MapData {
    fn new(scale: i8, x: f64, z: f64) -> Self {
        let mut m = Self {
            x_center: 0,
            z_center: 0,
            dimension: 0,
            scale,
            colors: vec![0; PIXELS],
            viewers: Vec::new(),
            icons: Vec::new(),
            dirty: true,
        };
        m.center_on(x, z);
        m
    }

    /// `calculateMapCenter`: the center of the map grid cell holding the point.
    fn center_on(&mut self, x: f64, z: f64) {
        let width = SIZE * (1 << self.scale);
        let cell = |v: f64| krol_java::math::floor_f64((v + 64.0) / f64::from(width));
        self.x_center = cell(x) * width + width / 2 - 64;
        self.z_center = cell(z) * width + width / 2 - 64;
    }

    /// `readFromNBT`.
    fn read(c: &Compound) -> Self {
        let width = i32::from(c.get_i16("width"));
        let height = i32::from(c.get_i16("height"));
        let stored = match c.get("colors") {
            Some(Tag::ByteArray(b)) => b.iter().map(|&v| v as u8).collect::<Vec<u8>>(),
            _ => Vec::new(),
        };
        let mut colors = vec![0u8; PIXELS];
        if width == SIZE && height == SIZE {
            for (k, v) in stored.iter().take(PIXELS).enumerate() {
                colors[k] = *v;
            }
        } else {
            // Smaller old maps sit in the middle.
            let (ox, oz) = ((SIZE - width) / 2, (SIZE - height) / 2);
            for z in 0..height {
                for x in 0..width {
                    let (tx, tz) = (x + ox, z + oz);
                    if (0..SIZE).contains(&tx) && (0..SIZE).contains(&tz) {
                        colors[(tx + tz * SIZE) as usize] =
                            stored.get((x + z * width) as usize).copied().unwrap_or(0);
                    }
                }
            }
        }
        Self {
            x_center: c.get_i32("xCenter"),
            z_center: c.get_i32("zCenter"),
            dimension: c.get_i8("dimension"),
            scale: c.get_i8("scale").clamp(0, MAX_SCALE),
            colors,
            viewers: Vec::new(),
            icons: Vec::new(),
            dirty: false,
        }
    }

    /// `writeToNBT`.
    fn write(&self) -> Compound {
        let mut c = Compound::new();
        c.insert("dimension", Tag::Byte(self.dimension));
        c.insert("xCenter", Tag::Int(self.x_center));
        c.insert("zCenter", Tag::Int(self.z_center));
        c.insert("scale", Tag::Byte(self.scale));
        c.insert("width", Tag::Short(SIZE as i16));
        c.insert("height", Tag::Short(SIZE as i16));
        c.insert(
            "colors",
            Tag::ByteArray(self.colors.iter().map(|&v| v as i8).collect()),
        );
        c
    }

    /// The viewer of a player (`playersHashMap.get`).
    fn viewer_index(&self, entity_id: i32) -> Option<usize> {
        self.viewers
            .iter()
            .position(|v| v.entity_id == entity_id && !v.stale)
    }

    fn viewer_mut(&mut self, entity_id: i32) -> &mut Viewer {
        if let Some(k) = self.viewer_index(entity_id) {
            &mut self.viewers[k]
        } else {
            self.viewers.push(Viewer::new(entity_id));
            let last = self.viewers.len() - 1;
            &mut self.viewers[last]
        }
    }

    /// `updateMapData`: a pixel changed.
    fn changed(&mut self, x: i32, y: i32) {
        self.dirty = true;
        for v in &mut self.viewers {
            v.update(x, y);
        }
    }

    /// `updateDecorations`: the icon of `name` at a position and rotation (on nether
    /// maps the rotation spins with the world time).
    fn put_icon(&mut self, kind: u8, name: &str, x: f64, z: f64, rotation: f64, time: i64) {
        let factor = (1 << self.scale) as f32;
        let fx = (x - f64::from(self.x_center)) as f32 / factor;
        let fz = (z - f64::from(self.z_center)) as f32 / factor;
        let mut px = (f64::from(fx * 2.0) + 0.5) as i32 as i8;
        let mut pz = (f64::from(fz * 2.0) + 0.5) as i32 as i8;
        let mut kind = kind;
        let turn;
        if fx >= -EDGE && fz >= -EDGE && fx <= EDGE && fz <= EDGE {
            let r = rotation + if rotation < 0.0 { -8.0 } else { 8.0 };
            let mut t = (r * 16.0 / 360.0) as i32 as i8;
            if self.dimension < 0 {
                let n = (time / 10) as i32;
                let spin = n
                    .wrapping_mul(n)
                    .wrapping_mul(34_187_121)
                    .wrapping_add(n.wrapping_mul(121));
                t = ((spin >> 15) & 15) as i8;
            }
            turn = t;
        } else {
            if !(fx.abs() < ICON_RANGE && fz.abs() < ICON_RANGE) {
                self.icons.retain(|(n, _)| n != name);
                return;
            }
            kind = OFF_MAP_ICON;
            turn = 0;
            // Vanilla quirk: the west and north edges wrap to -128.
            if fx <= -EDGE {
                px = (f64::from(EDGE * 2.0) + 2.5) as i32 as i8;
            }
            if fz <= -EDGE {
                pz = (f64::from(EDGE * 2.0) + 2.5) as i32 as i8;
            }
            if fx >= EDGE {
                px = (EDGE as i32 * 2 + 1) as i8;
            }
            if fz >= EDGE {
                pz = (EDGE as i32 * 2 + 1) as i8;
            }
        }
        let icon = MapIcon {
            kind,
            x: px,
            z: pz,
            rotation: turn as u8,
        };
        match self.icons.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = icon,
            None => self.icons.push((name.to_owned(), icon)),
        }
    }
}

/// The world's maps (`MapStorage` for `map_*` data and the `map` id counter).
#[derive(Debug, Default)]
pub struct MapStorage {
    /// The last map id handed out (`idCounts["map"]`).
    last_id: Option<i16>,
    /// Maps loaded or made, by id.
    maps: BTreeMap<i16, MapData>,
    /// Ids whose file was looked for and is missing.
    missing: std::collections::HashSet<i16>,
}

impl MapStorage {
    /// `MapStorage.loadIdCounts`: `data/idcounts.dat` (uncompressed).
    pub fn load(world_dir: &Path) -> Self {
        let mut s = Self::default();
        let path = world_dir.join("data").join("idcounts.dat");
        let Ok(bytes) = std::fs::read(&path) else {
            return s;
        };
        match krol_nbt::read(&mut bytes.as_slice()) {
            Ok((_, root)) => {
                if root.contains_key_of("map", Some(TagType::Short)) {
                    s.last_id = Some(root.get_i16("map"));
                }
            }
            Err(e) => error!("Couldn't load map id counts: {e}"),
        }
        s
    }

    /// `getUniqueDataId("map")`: the next id, written to `idcounts.dat` right away.
    fn next_id(&mut self, world_dir: &Path) -> i16 {
        let id = self.last_id.map_or(0, |v| v.wrapping_add(1));
        self.last_id = Some(id);
        let mut root = Compound::new();
        root.insert("map", Tag::Short(id));
        let dir = world_dir.join("data");
        let _ = std::fs::create_dir_all(&dir);
        let mut out = Vec::new();
        if krol_nbt::write(&mut out, "", &root).is_ok()
            && let Err(e) = std::fs::write(dir.join("idcounts.dat"), out)
        {
            error!("Couldn't save map id counts: {e}");
        }
        self.missing.remove(&id);
        id
    }

    /// `loadItemData`: the map, read from `data/map_<id>.dat` the first time.
    fn get(&mut self, world_dir: &Path, id: i16) -> Option<&mut MapData> {
        if !self.maps.contains_key(&id) && !self.missing.contains(&id) {
            let path = world_dir.join("data").join(format!("map_{id}.dat"));
            match std::fs::File::open(&path) {
                Ok(file) => match krol_nbt::read_gzip(std::io::BufReader::new(file)) {
                    Ok((_, root)) => {
                        let data = root.get_compound("data").cloned().unwrap_or_default();
                        self.maps.insert(id, MapData::read(&data));
                    }
                    Err(e) => {
                        error!("Couldn't load map_{id}: {e}");
                        self.missing.insert(id);
                    }
                },
                Err(_) => {
                    self.missing.insert(id);
                }
            }
        }
        self.maps.get_mut(&id)
    }

    /// A player entity left the world for good (it respawned as a new entity): its
    /// viewers on every loaded map go stale.
    pub(super) fn player_entity_replaced(&mut self, entity_id: i32) {
        for m in self.maps.values_mut() {
            for v in &mut m.viewers {
                if v.entity_id == entity_id {
                    v.stale = true;
                }
            }
        }
    }

    /// A loaded map's scale, if it is loaded.
    pub fn loaded_scale(&self, id: i16) -> Option<i8> {
        self.maps.get(&id).map(|m| m.scale)
    }

    /// `saveAllData`: every changed map to `data/map_<id>.dat` (gzip).
    pub fn save(&mut self, world_dir: &Path) {
        let dir = world_dir.join("data");
        for (id, m) in &mut self.maps {
            if !m.dirty {
                continue;
            }
            let mut root = Compound::new();
            root.insert("data", Tag::Compound(m.write()));
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join(format!("map_{id}.dat"));
            let result = std::fs::File::create(&path)
                .map_err(krol_nbt::Error::from)
                .and_then(|f| krol_nbt::write_gzip(f, "", &root));
            match result {
                Ok(()) => m.dirty = false,
                Err(e) => error!("Couldn't save map_{id}: {e}"),
            }
        }
    }
}

/// The window slot of vanilla's inventory index (hotbar 0-8, main 9-35, armor 36-39
/// boots first).
fn window_slot(index: usize) -> usize {
    match index {
        0..=8 => 36 + index,
        9..=35 => index,
        _ => 8 - (index - 36),
    }
}

impl Game {
    /// `ItemEmptyMap.onItemRightClick`: a new map of the area around the player.
    pub(super) fn use_empty_map(
        &mut self,
        i: usize,
        held: &ItemStack,
    ) -> super::buckets::UseResult {
        let dir = self.level.dir.clone();
        let id = self.maps.next_id(&dir);
        let p = &self.players[i];
        let data = MapData::new(0, p.pos.0, p.pos.2);
        self.maps.maps.insert(id, data);
        let map = ItemStack {
            id: FILLED_MAP,
            count: 1,
            damage: id,
            nbt: None,
        };
        if held.count <= 1 {
            return super::buckets::UseResult::Replaced(Some(map));
        }
        let p = &mut self.players[i];
        let slot = 36 + p.selected;
        if let Some(s) = p.inventory[slot].as_mut() {
            s.count -= 1;
        }
        let mut copy = map.clone();
        let creative = p.gamemode == super::player::Player::CREATIVE;
        if !super::item_entity::add_to_inventory(&mut p.inventory, &mut copy, creative) {
            self.toss(i, map, false);
        }
        // Vanilla quirk: the use only counts when empty maps are left.
        self.count_item_use(i);
        super::buckets::UseResult::Resized
    }

    /// `ItemMap.getMapData`: the map a filled map item shows; without data, a new map
    /// around the spawn takes the item (its damage changes to the new id).
    fn map_data_id(&mut self, stack: &mut ItemStack) -> Option<i16> {
        if stack.id != FILLED_MAP {
            return None;
        }
        let dir = self.level.dir.clone();
        if self.maps.get(&dir, stack.damage).is_some() {
            return Some(stack.damage);
        }
        let new = self.maps.next_id(&dir);
        let (sx, sz) = (self.level.data.spawn_x(), self.level.data.spawn_z());
        let data = MapData::new(LOST_MAP_SCALE, f64::from(sx), f64::from(sz));
        self.maps.maps.insert(new, data);
        stack.damage = new;
        Some(new)
    }

    /// [`Self::map_data_id`] of an inventory slot.
    fn map_of_slot(&mut self, i: usize, slot: usize) -> Option<i16> {
        let mut stack = self.players[i].inventory[slot].take()?;
        let id = self.map_data_id(&mut stack);
        self.players[i].inventory[slot] = Some(stack);
        id
    }

    /// `InventoryPlayer.decrementAnimations` for maps (`ItemMap.onUpdate`), run with the
    /// living update: every map in the main inventory sees its viewers, and the held one
    /// draws the world around the player.
    pub(super) fn update_carried_maps(&mut self, i: usize) {
        for index in 0..36 {
            let slot = window_slot(index);
            let Some(id) = self.map_of_slot(i, slot) else {
                continue;
            };
            let nbt = self.players[i].inventory[slot]
                .as_ref()
                .and_then(|s| s.nbt.clone());
            self.update_map_viewers(i, id, nbt.as_ref(), None);
            if index == self.players[i].selected {
                self.draw_map(i, id);
            }
        }
    }

    /// `MapData.updateVisiblePlayers` for player `i` and a stack of map `id`: carried
    /// by the player, or shown in an item frame (`frame`).
    fn update_map_viewers(
        &mut self,
        i: usize,
        id: i16,
        nbt: Option<&Compound>,
        frame: Option<&FramedMap>,
    ) {
        let entity_id = self.players[i].entity_id;
        // `InventoryPlayer.hasItemStack`: armor or main inventory, same item and damage.
        let carries = |p: &super::player::Player| {
            p.inventory[5..45]
                .iter()
                .flatten()
                .any(|s| s.id == FILLED_MAP && s.damage == id)
        };
        let viewer_name = self.players[i].profile.name.clone();
        let viewer_carries = carries(&self.players[i]);
        // The living players (`!isDead`), whether they carry the map, where they are.
        let alive: Vec<(i32, bool, String, f64, f64, f64)> = self
            .players
            .iter()
            .filter(|p| !p.dead_marked && !p.removed)
            .map(|p| {
                (
                    p.entity_id,
                    carries(p),
                    p.profile.name.clone(),
                    p.pos.0,
                    p.pos.2,
                    f64::from(p.rotation.0),
                )
            })
            .collect();
        let time = self.level.data.day_time();
        let dir = self.level.dir.clone();
        let Some(m) = self.maps.get(&dir, id) else {
            return;
        };
        if m.viewer_index(entity_id).is_none() {
            m.viewers.push(Viewer::new(entity_id));
        }
        if !viewer_carries {
            m.icons.retain(|(n, _)| *n != viewer_name);
        }
        // Vanilla quirk: removing a viewer skips the one after it.
        let mut k = 0;
        while k < m.viewers.len() {
            let viewer = &m.viewers[k];
            let found = alive
                .iter()
                .find(|c| c.0 == viewer.entity_id)
                .filter(|_| !viewer.stale);
            match found {
                Some((_, carrying, name, x, z, yaw)) if *carrying || frame.is_some() => {
                    if frame.is_none() && m.dimension == 0 {
                        let (name, x, z, yaw) = (name.clone(), *x, *z, *yaw);
                        m.put_icon(PLAYER_ICON, &name, x, z, yaw, time);
                    }
                }
                _ => {
                    m.viewers.remove(k);
                }
            }
            k += 1;
        }
        if let Some(f) = frame {
            let rotation = f64::from(i32::from(f.facing) * 90);
            m.put_icon(
                FRAME_ICON,
                &frame_icon_name(f.entity_id),
                f64::from(f.tile.0),
                f64::from(f.tile.2),
                rotation,
                time,
            );
        }
        // Icons the item itself carries (`Decorations`), unless one of that name is shown.
        if let Some(list) = nbt.and_then(|n| n.get_list("Decorations", TagType::Compound)) {
            for tag in list.iter() {
                let Tag::Compound(d) = tag else { continue };
                let name = d.get_str("id");
                if m.icons.iter().any(|(n, _)| n == name) {
                    continue;
                }
                let name = name.to_owned();
                m.put_icon(
                    d.get_i8("type") as u8,
                    &name,
                    d.get_f64("x"),
                    d.get_f64("z"),
                    d.get_f64("rot"),
                    time,
                );
            }
        }
    }

    /// `EntityTrackerEntry.updatePlayerList` every 10 ticks for an item frame showing a
    /// filled map: every player in the world becomes a viewer and gets the map packet
    /// due (before the frame's metadata).
    pub(super) fn update_framed_map(&mut self, frame: &FramedMap) {
        let Some(h) = self.hanging.get(&frame.entity_id) else {
            return;
        };
        let super::hanging::Kind::ItemFrame {
            item: Some(stack), ..
        } = &h.kind
        else {
            return;
        };
        let mut stack = stack.clone();
        let Some(id) = self.map_data_id(&mut stack) else {
            return;
        };
        // A map without data took a new id: the frame's item changes in place.
        if let Some(h) = self.hanging.get_mut(&frame.entity_id)
            && let super::hanging::Kind::ItemFrame {
                item: Some(shown), ..
            } = &mut h.kind
        {
            shown.damage = id;
        }
        for i in 0..self.players.len() {
            self.update_map_viewers(i, id, stack.nbt.as_ref(), Some(frame));
            if let Some(packet) = self.map_packet(i, id) {
                self.players[i].send(packet);
            }
        }
    }

    /// `EntityItemFrame.removeFrameFromMap`: a filled map leaving a frame takes the
    /// frame's icon off its map.
    pub(super) fn remove_frame_icon(&mut self, stack: &mut ItemStack, frame_id: i32) {
        let Some(id) = self.map_data_id(stack) else {
            return;
        };
        let dir = self.level.dir.clone();
        if let Some(m) = self.maps.get(&dir, id) {
            let name = frame_icon_name(frame_id);
            m.icons.retain(|(n, _)| *n != name);
        }
    }

    /// `ItemMap.updateMapData`: a sixteenth of the columns within reach (those matching
    /// this update's step, plus the one after any column that changed) are drawn from
    /// the blocks under them.
    fn draw_map(&mut self, i: usize, id: i16) {
        let entity_id = self.players[i].entity_id;
        let pos = self.players[i].pos;
        let dir = self.level.dir.clone();
        let Some(m) = self.maps.get(&dir, id) else {
            return;
        };
        if m.dimension != 0 {
            return;
        }
        let factor = 1i32 << m.scale;
        let (xc, zc) = (m.x_center, m.z_center);
        // Java's integer division truncates toward zero.
        let px = krol_java::math::floor_f64(pos.0 - f64::from(xc)) / factor + 64;
        let pz = krol_java::math::floor_f64(pos.2 - f64::from(zc)) / factor + 64;
        let reach = SIZE / factor;
        let step = {
            let v = m.viewer_mut(entity_id);
            v.step += 1;
            v.step
        };
        let mut colors = std::mem::take(&mut m.colors);
        let mut changes = Vec::new();
        let w = &mut self.level.overworld;
        let mut follow = false;
        for x in px - reach + 1..px + reach {
            if (x & 15) != (step & 15) && !follow {
                continue;
            }
            follow = false;
            let mut previous = 0.0f64;
            for z in pz - reach - 1..pz + reach {
                if !(x >= 0 && z >= -1 && x < SIZE && z < SIZE) {
                    continue;
                }
                let (dx, dz) = (x - px, z - pz);
                let rim = dx * dx + dz * dz > (reach - 2) * (reach - 2);
                let bx = (xc / factor + x - 64) * factor;
                let bz = (zc / factor + z - 64) * factor;
                if !w.is_chunk_loaded(bx >> 4, bz >> 4) {
                    continue;
                }
                let mut counts: Vec<(u8, i32)> = Vec::new();
                let mut depth = 0;
                let mut height = 0.0f64;
                for ox in 0..factor {
                    for oz in 0..factor {
                        let (cx, cz) = (bx + ox, bz + oz);
                        let mut y = w.height_at(cx, cz) + 1;
                        let mut state = 0u16;
                        if y > 1 {
                            loop {
                                y -= 1;
                                state = w.block(cx, y, cz);
                                if !(map_color(state) == AIR_COLOR && y > 0) {
                                    break;
                                }
                            }
                            if y > 0 && super::block_data::material(state >> 4).is_liquid() {
                                let mut below = y - 1;
                                loop {
                                    let b = w.block(cx, below, cz);
                                    below -= 1;
                                    depth += 1;
                                    if !(below > 0
                                        && super::block_data::material(b >> 4).is_liquid())
                                    {
                                        break;
                                    }
                                }
                            }
                        }
                        height += f64::from(y) / f64::from(factor * factor);
                        let c = map_color(state);
                        match counts.iter_mut().find(|e| e.0 == c) {
                            Some(e) => e.1 += 1,
                            None => counts.push((c, 1)),
                        }
                    }
                }
                depth /= factor * factor;
                let checker = f64::from((x + z) & 1);
                let mut d =
                    (height - previous) * 4.0 / f64::from(factor + 4) + (checker - 0.5) * 0.4;
                let mut shade = 1u8;
                if d > 0.6 {
                    shade = 2;
                }
                if d < -0.6 {
                    shade = 0;
                }
                let color = counts
                    .iter()
                    .fold(None::<(u8, i32)>, |best, &e| match best {
                        Some(b) if b.1 >= e.1 => Some(b),
                        _ => Some(e),
                    })
                    .map_or(AIR_COLOR, |e| e.0);
                if color == WATER_COLOR {
                    d = f64::from(depth) * 0.1 + checker * 0.2;
                    shade = 1;
                    if d < 0.5 {
                        shade = 2;
                    }
                    if d > 0.9 {
                        shade = 0;
                    }
                }
                previous = height;
                if z >= 0 && dx * dx + dz * dz < reach * reach && (!rim || (x + z) & 1 != 0) {
                    let k = (x + z * SIZE) as usize;
                    let new = color.wrapping_mul(4).wrapping_add(shade);
                    if colors[k] != new {
                        colors[k] = new;
                        changes.push((x, z));
                        follow = true;
                    }
                }
            }
        }
        let Some(m) = self.maps.get(&dir, id) else {
            return;
        };
        m.colors = colors;
        for (x, z) in changes {
            m.changed(x, z);
        }
    }

    /// `EntityPlayerMP.onUpdateEntity`: the packet of every map the player carries.
    pub(super) fn send_map_packets(&mut self, i: usize) {
        for index in 0..40 {
            let slot = window_slot(index);
            let Some(id) = self.map_of_slot(i, slot) else {
                continue;
            };
            if let Some(packet) = self.map_packet(i, id) {
                self.players[i].send(packet);
            }
        }
    }

    /// `MapData.getMapPacket`: for a viewer of the map, the changed rectangle, or every
    /// fifth time without changes just the icons.
    fn map_packet(&mut self, i: usize, id: i16) -> Option<ClientboundPlay> {
        let entity_id = self.players[i].entity_id;
        let dir = self.level.dir.clone();
        let m = self.maps.get(&dir, id)?;
        let k = m.viewer_index(entity_id)?;
        let icons: Vec<MapIcon> = m.icons.iter().map(|(_, icon)| *icon).collect();
        let v = &mut m.viewers[k];
        if v.pending {
            v.pending = false;
            let (columns, rows) = (v.max_x + 1 - v.min_x, v.max_y + 1 - v.min_y);
            let mut data = Vec::with_capacity((columns * rows).max(0) as usize);
            for r in 0..rows {
                for c in 0..columns {
                    data.push(m.colors[(v.min_x + c + (v.min_y + r) * SIZE) as usize]);
                }
            }
            Some(ClientboundPlay::Maps {
                map: i32::from(id),
                scale: m.scale,
                icons,
                columns: columns as u8,
                rows: rows as u8,
                x: v.min_x as u8,
                z: v.min_y as u8,
                data,
            })
        } else {
            let call = v.calls;
            v.calls += 1;
            (call % ICON_PERIOD == 0).then(|| ClientboundPlay::Maps {
                map: i32::from(id),
                scale: m.scale,
                icons,
                columns: 0,
                rows: 0,
                x: 0,
                z: 0,
                data: Vec::new(),
            })
        }
    }

    /// `ItemMap.onCreated` for a map taken from a zoom recipe (`map_is_scaling`): it
    /// becomes a new map, one scale larger, around the old one's center.
    pub(super) fn zoom_crafted_map(&mut self, stack: &mut ItemStack) {
        let scaling = stack
            .nbt
            .as_ref()
            .is_some_and(|n| n.get_i8("map_is_scaling") != 0);
        if stack.id != FILLED_MAP || !scaling {
            return;
        }
        let dir = self.level.dir.clone();
        let Some(old) = self.maps.get(&dir, stack.damage).cloned() else {
            return;
        };
        let id = self.maps.next_id(&dir);
        stack.damage = id;
        let scale = (old.scale + 1).min(MAX_SCALE);
        let mut data = MapData::new(scale, f64::from(old.x_center), f64::from(old.z_center));
        data.dimension = old.dimension;
        self.maps.maps.insert(id, data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centers_and_icons() {
        // The 128-block cell holding the spawn of the flat fixture.
        let m = MapData::new(0, -471.5, -434.5);
        assert_eq!((m.x_center, m.z_center), (-512, -384));
        let mut m = MapData::new(0, 0.0, 0.0);
        m.put_icon(0, "a", 10.0, -3.0, 90.0, 0);
        assert_eq!(
            m.icons[0].1,
            MapIcon {
                kind: 0,
                x: 20,
                z: -5,
                rotation: 4
            }
        );
        // Off the west edge: the edge icon, at -128.
        m.put_icon(0, "a", -100.0, 0.0, 0.0, 0);
        assert_eq!((m.icons[0].1.kind, m.icons[0].1.x), (OFF_MAP_ICON, -128));
        m.put_icon(0, "a", -1000.0, 0.0, 0.0, 0);
        assert!(m.icons.is_empty());
        assert_eq!(map_color(2 << 4), 1);
        assert_eq!(map_color(9 << 4), WATER_COLOR);
    }
}
