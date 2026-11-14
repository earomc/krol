//! Connected players (vanilla: `EntityPlayerMP`, `NetHandlerPlayServer`,
//! `SaveHandler.writePlayerData`).

use std::collections::{HashSet, VecDeque};
use std::net::SocketAddr;
use std::path::Path;

use krol_nbt::{Compound, List, Tag, TagType};
use krol_protocol::ProtocolVersion;
use krol_protocol::packets::play::ClientboundPlay;
use tokio::sync::mpsc::UnboundedSender;
use tracing::warn;

use crate::auth::GameProfile;
use crate::status::java_uuid_string;

use super::Outgoing;

/// A connected player.
#[derive(Debug)]
pub struct Player {
    /// Connection id assigned by the network layer.
    pub conn_id: u64,
    /// Entity id.
    pub entity_id: i32,
    /// Profile (UUID, name, properties).
    pub profile: GameProfile,
    /// Remote address.
    pub peer: SocketAddr,
    /// Client protocol.
    pub version: ProtocolVersion,
    /// Outgoing packet queue (drained by the connection's writer task).
    pub out: UnboundedSender<Outgoing>,
    /// Position.
    pub pos: (f64, f64, f64),
    /// Rotation (yaw, pitch).
    pub rotation: (f32, f32),
    /// On-ground flag from the last movement packet.
    pub on_ground: bool,
    /// Game mode id.
    pub gamemode: i32,
    /// Skin parts from Client Settings (metadata index 10).
    pub skin_parts: u8,
    /// Ticks since joining.
    pub ticks: u64,
    /// Network tick of the last keep-alive sent.
    pub last_keep_alive: u64,
    /// Chunks the player watches (vanilla: `PlayerInstance.playersWatchingChunk`); a
    /// chunk is watched from the moment it is queued, before it is sent.
    pub watching: HashSet<(i32, i32)>,
    /// Watched chunks not yet sent, in vanilla's spiral order (`loadedChunks`).
    pub chunk_queue: VecDeque<(i32, i32)>,
    /// Position the watched area was last computed for (`managedPosX/Z`).
    pub managed_pos: (f64, f64),
    /// Movement state (vanilla: `NetHandlerPlayServer` fields).
    pub movement: Movement,
    /// Chunks watched since the game last loaded them.
    pub new_watches: Vec<(i32, i32)>,
    /// The tick in which the player joined.
    pub joined_tick: u64,
    /// The player inventory window (45 slots, vanilla `ContainerPlayer`).
    pub inventory: Vec<Option<krol_protocol::packets::play::ItemStack>>,
    /// What the client was last told per slot (`Container.inventoryItemStacks`); starts
    /// empty, which makes vanilla resend every slot on the first tick.
    pub sent_inventory: Vec<Option<krol_protocol::packets::play::ItemStack>>,
    /// Selected hotbar slot (0-8).
    pub selected: usize,
    /// Statistics (`stats/<uuid>.json`).
    pub stats: super::stats::Stats,
    /// Experience changed since the last Set Experience packet.
    pub xp_dirty: bool,
    /// Data watcher indices changed since the entity tracker last sent them (all of them
    /// on creation).
    pub dirty_metadata: std::collections::BTreeSet<u8>,
    /// Watched attributes changed since the tracker last sent them.
    pub dirty_attributes: bool,
    /// The chunk the player entity was last added to (vanilla `chunkCoordX/Z`).
    pub entity_chunk: (i32, i32),
    /// Arm swing state (`isSwingInProgress`, `swingProgressInt`).
    pub swing: (bool, i32),
    /// Motion per tick of the server's copy of the player (`motionX/Y/Z`).
    pub motion: (f64, f64, f64),
    /// Sneaking (entity flag 1).
    pub sneaking: bool,
    /// Sprinting (entity flag 3).
    pub sprinting: bool,
    /// Flying (`capabilities.isFlying`).
    pub flying: bool,
    /// Jumped since the tracker last ran (`isAirBorne`).
    pub airborne: bool,
    /// Distance fallen since last on the ground (`fallDistance`).
    pub fall_distance: f32,
    /// In water (`inWater`).
    pub in_water: bool,
    /// The last move was blocked horizontally.
    pub collided_horizontally: bool,
    /// Distance walked, for step sounds (`distanceWalkedOnStepModified`).
    pub distance_walked_on_step: f32,
    /// Distance of the next step sound (`nextStepDistance`).
    pub next_step_distance: i32,
    /// Equipment as last announced to viewers (`previousEquipment`).
    pub last_equipment: [Option<krol_protocol::packets::play::ItemStack>; 5],
    /// Chat spam counter (vanilla: `chatSpamThresholdCount`).
    pub chat_spam: i32,
    /// Raw player data, kept to preserve fields krol does not model yet.
    pub data: Compound,
}

impl Player {
    /// Queues a packet for the client.
    pub fn send(&self, packet: ClientboundPlay) {
        let _ = self.out.send(Outgoing::Packet(packet));
    }

    /// Vanilla quirk: `PlayerManager` finds the player's chunk with `(int) pos >> 4`
    /// (truncation toward zero, then shift), not `floor(pos) >> 4`.
    fn managed_chunk(x: f64, z: f64) -> (i32, i32) {
        ((x as i32) >> 4, (z as i32) >> 4)
    }

    /// Newly watched chunks since the last call; the game loads them (vanilla's
    /// `PlayerInstance` constructor loads its chunk).
    pub fn take_new_watches(&mut self) -> Vec<(i32, i32)> {
        std::mem::take(&mut self.new_watches)
    }

    /// Vanilla `PlayerManager.addPlayer`: watch every chunk within `radius`.
    pub fn start_watching(&mut self, radius: i32) {
        let (cx, cz) = Self::managed_chunk(self.pos.0, self.pos.2);
        self.managed_pos = (self.pos.0, self.pos.2);
        for x in cx - radius..=cx + radius {
            for z in cz - radius..=cz + radius {
                self.watch((x, z));
            }
        }
        self.filter_chunk_queue(radius);
    }

    fn watch(&mut self, c: (i32, i32)) {
        if self.watching.insert(c) {
            self.chunk_queue.push_back(c);
            self.new_watches.push(c);
        }
    }

    /// Vanilla `PlayerManager.updateMountedMovingPlayer`: after moving at least 8 blocks
    /// from the managed position, watch newly covered chunks and stop watching the rest.
    /// Returns the chunks to unload (vanilla sends an unload for every watched chunk it
    /// drops, even if it was never sent).
    pub fn update_watching(&mut self, radius: i32) -> Vec<(i32, i32)> {
        let dx = self.managed_pos.0 - self.pos.0;
        let dz = self.managed_pos.1 - self.pos.2;
        if dx * dx + dz * dz < 64.0 {
            return Vec::new();
        }
        let (i, j) = Self::managed_chunk(self.pos.0, self.pos.2);
        let (k, l) = Self::managed_chunk(self.managed_pos.0, self.managed_pos.1);
        let (di, dj) = (i - k, j - l);
        if di == 0 && dj == 0 {
            return Vec::new();
        }
        let overlaps =
            |x: i32, z: i32, cx: i32, cz: i32| (x - cx).abs() <= radius && (z - cz).abs() <= radius;
        let mut unload = Vec::new();
        for x in i - radius..=i + radius {
            for z in j - radius..=j + radius {
                if !overlaps(x, z, k, l) {
                    self.watch((x, z));
                }
                let old = (x - di, z - dj);
                if !overlaps(old.0, old.1, i, j) && self.watching.remove(&old) {
                    self.chunk_queue.retain(|&c| c != old);
                    unload.push(old);
                }
            }
        }
        self.filter_chunk_queue(radius);
        self.managed_pos = (self.pos.0, self.pos.2);
        unload
    }

    /// Vanilla `PlayerManager.filterChunkLoadQueue`: reorder the send queue as a spiral
    /// around the player's chunk.
    fn filter_chunk_queue(&mut self, radius: i32) {
        const DIRS: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];
        let pending: HashSet<(i32, i32)> = self.chunk_queue.drain(..).collect();
        let (k, l) = Self::managed_chunk(self.pos.0, self.pos.2);
        let push = |queue: &mut VecDeque<(i32, i32)>, c: (i32, i32)| {
            if pending.contains(&c) {
                queue.push_back(c);
            }
        };
        let mut queue = VecDeque::with_capacity(pending.len());
        push(&mut queue, (k, l));
        let (mut dir, mut ox, mut oz) = (0usize, 0, 0);
        for len in 1..=radius * 2 {
            for _ in 0..2 {
                let (ddx, ddz) = DIRS[dir % 4];
                dir += 1;
                for _ in 0..len {
                    ox += ddx;
                    oz += ddz;
                    push(&mut queue, (k + ox, l + oz));
                }
            }
        }
        dir %= 4;
        for _ in 0..radius * 2 {
            ox += DIRS[dir].0;
            oz += DIRS[dir].1;
            push(&mut queue, (k + ox, l + oz));
        }
        self.chunk_queue = queue;
    }

    /// Vanilla `EntityPlayer.xpBarCap`.
    fn xp_bar_cap(level: i32) -> i32 {
        if level >= 30 {
            112 + (level - 30) * 9
        } else if level >= 15 {
            37 + (level - 15) * 5
        } else {
            7 + level * 2
        }
    }

    /// Vanilla `EntityPlayer.addExperience` (float math as in vanilla).
    pub fn add_experience(&mut self, amount: i32) {
        let score = self.data.get_i32("Score").wrapping_add(amount);
        self.data.insert("Score", Tag::Int(score));
        self.dirty_metadata.insert(18);
        let mut total = self.data.get_i32("XpTotal");
        let mut level = self.data.get_i32("XpLevel");
        let mut bar = self.data.get_f32("XpP");
        let amount = amount.min(i32::MAX - total);
        bar += amount as f32 / Self::xp_bar_cap(level) as f32;
        total += amount;
        while bar >= 1.0 {
            bar = (bar - 1.0) * Self::xp_bar_cap(level) as f32;
            level += 1;
            bar /= Self::xp_bar_cap(level) as f32;
        }
        self.data.insert("XpTotal", Tag::Int(total));
        self.data.insert("XpLevel", Tag::Int(level));
        self.data.insert("XpP", Tag::Float(bar));
        self.xp_dirty = true;
    }

    /// Vanilla `EntityPlayerMP.addExperienceLevel`.
    pub fn add_experience_levels(&mut self, levels: i32) {
        let mut level = self.data.get_i32("XpLevel").wrapping_add(levels);
        if level < 0 {
            level = 0;
            self.data.insert("XpP", Tag::Float(0.0));
            self.data.insert("XpTotal", Tag::Int(0));
        }
        self.data.insert("XpLevel", Tag::Int(level));
        // TODO: the level-up sound every 5 levels.
        self.xp_dirty = true;
    }

    /// Writes the player's data to `<world>/playerdata/<uuid>.dat` (vanilla:
    /// `writePlayerData`: `<uuid>.dat.tmp`, then replace).
    pub fn save(&mut self, world_dir: &Path) {
        self.store_state();
        self.stats.save();
        let dir = world_dir.join("playerdata");
        let uuid = java_uuid_string(self.profile.id);
        let tmp = dir.join(format!("{uuid}.dat.tmp"));
        let target = dir.join(format!("{uuid}.dat"));
        let result = std::fs::create_dir_all(&dir)
            .map_err(krol_nbt::Error::from)
            .and_then(|()| krol_nbt::write_gzip(std::fs::File::create(&tmp)?, "", &self.data))
            .map_err(|e| e.to_string())
            .and_then(|()| {
                if target.exists() {
                    std::fs::remove_file(&target).map_err(|e| e.to_string())?;
                }
                std::fs::rename(&tmp, &target).map_err(|e| e.to_string())
            });
        if let Err(e) = result {
            warn!("Failed to save player data for {}: {e}", self.profile.name);
        }
    }

    /// The held item (vanilla `inventory.getCurrentItem()`).
    pub fn held(&self) -> Option<&krol_protocol::packets::play::ItemStack> {
        self.inventory[36 + self.selected].as_ref()
    }

    fn store_state(&mut self) {
        store_inventory(&mut self.data, &self.inventory);
        let m = self.motion;
        self.data.insert("Motion", doubles(&[m.0, m.1, m.2]));
        self.data
            .insert("FallDistance", Tag::Float(self.fall_distance));
        // Vanilla EntityPlayer.writeEntityToNBT also stores the held item on its own.
        match self.held().and_then(item_to_nbt) {
            Some(c) => {
                self.data.insert("SelectedItem", Tag::Compound(c));
            }
            None => {
                self.data.remove("SelectedItem");
            }
        }
        self.data
            .insert("SelectedItemSlot", Tag::Int(self.selected as i32));
        let d = &mut self.data;
        d.insert("Pos", doubles(&[self.pos.0, self.pos.1, self.pos.2]));
        d.insert("Rotation", floats(&[self.rotation.0, self.rotation.1]));
        d.insert("OnGround", Tag::Byte(i8::from(self.on_ground)));
        d.insert("playerGameType", Tag::Int(self.gamemode));
        let (most, least) = ((self.profile.id >> 64) as i64, self.profile.id as i64);
        d.insert("UUIDMost", Tag::Long(most));
        d.insert("UUIDLeast", Tag::Long(least));
    }
}

fn doubles(values: &[f64]) -> Tag {
    let mut l = List::new();
    for v in values {
        l.push(Tag::Double(*v)).expect("homogeneous");
    }
    Tag::List(l)
}

fn floats(values: &[f32]) -> Tag {
    let mut l = List::new();
    for v in values {
        l.push(Tag::Float(*v)).expect("homogeneous");
    }
    Tag::List(l)
}

/// Movement bookkeeping from vanilla's `NetHandlerPlayServer`.
#[derive(Debug, Clone, Default)]
pub struct Movement {
    /// Whether the client confirmed the last teleport (`hasMoved`).
    pub has_moved: bool,
    /// Last accepted position (`lastPosX/Y/Z`).
    pub last_pos: (f64, f64, f64),
    /// Network ticks (`networkTickCount`).
    pub network_ticks: u64,
    /// Network tick of the last accepted movement (`field_175090_f`).
    pub last_move_tick: u64,
    /// Ticks spent floating (`floatingTickCount`).
    pub floating_ticks: u32,
}

/// Reads an item stack from NBT (vanilla `ItemStack.readFromNBT`): the id is a name
/// (`minecraft:stone`) or, in old data, a numeric short; unknown items are dropped.
pub fn item_from_nbt(c: &Compound) -> Option<krol_protocol::packets::play::ItemStack> {
    let id = match c.get("id") {
        Some(Tag::String(name)) => krol_world::items::id_for_name(name)?,
        Some(tag) => tag.as_i16()?,
        None => return None,
    };
    Some(krol_protocol::packets::play::ItemStack {
        id,
        count: c.get_i8("Count"),
        damage: c.get_i16("Damage").max(0),
        nbt: c.get_compound("tag").cloned(),
    })
}

/// The 45 slots of the player inventory window (vanilla `ContainerPlayer`): crafting
/// result and grid (0–4, empty), armor 5–8 (helmet first), main inventory 9–35 and the
/// hotbar 36–44, filled from the stored `Inventory` list.
pub fn inventory_window(data: &Compound) -> Vec<Option<krol_protocol::packets::play::ItemStack>> {
    let mut window = vec![None; 45];
    if let Some(list) = data.get_list("Inventory", TagType::Compound) {
        for tag in list {
            let Tag::Compound(c) = tag else { continue };
            let slot = i32::from(c.get_i8("Slot") as u8);
            let index = match slot {
                0..=8 => 36 + slot,
                9..=35 => slot,
                100..=103 => 8 - (slot - 100),
                _ => continue,
            };
            window[index as usize] = item_from_nbt(c);
        }
    }
    window
}

/// An item stack as NBT (vanilla `ItemStack.writeToNBT`), without a slot.
fn item_to_nbt(item: &krol_protocol::packets::play::ItemStack) -> Option<Compound> {
    let name = krol_world::items::name_for_id(item.id)?;
    let mut c = Compound::new();
    c.insert("id", Tag::String(name.to_owned()));
    c.insert("Count", Tag::Byte(item.count));
    c.insert("Damage", Tag::Short(item.damage));
    if let Some(tag) = &item.nbt {
        c.insert("tag", Tag::Compound(tag.clone()));
    }
    Some(c)
}

/// Writes the window back as the `Inventory` list like vanilla `InventoryPlayer.writeToNBT`
/// (main slots 0-35, then armor 100-103), keeping entries krol cannot map unchanged.
fn store_inventory(
    data: &mut Compound,
    window: &[Option<krol_protocol::packets::play::ItemStack>],
) {
    let mut list = List::new();
    let mut push = |slot: i32, item: &Option<krol_protocol::packets::play::ItemStack>| {
        let Some(item) = item else { return };
        let Some(name) = krol_world::items::name_for_id(item.id) else {
            return;
        };
        let mut c = Compound::new();
        c.insert("Slot", Tag::Byte(slot as u8 as i8));
        c.insert("id", Tag::String(name.to_owned()));
        c.insert("Count", Tag::Byte(item.count));
        c.insert("Damage", Tag::Short(item.damage));
        if let Some(tag) = &item.nbt {
            c.insert("tag", Tag::Compound(tag.clone()));
        }
        let _ = list.push(Tag::Compound(c));
    };
    for slot in 0..36 {
        let index = if slot < 9 { 36 + slot } else { slot };
        push(slot as i32, &window[index]);
    }
    for armor in 0..4 {
        push(100 + armor as i32, &window[8 - armor]);
    }
    data.insert("Inventory", Tag::List(list));
}

/// Loads `<world>/playerdata/<uuid>.dat`, if present and readable.
pub fn load_player_data(world_dir: &Path, uuid: u128) -> Option<Compound> {
    let path = world_dir
        .join("playerdata")
        .join(format!("{}.dat", java_uuid_string(uuid)));
    let file = std::fs::File::open(path).ok()?;
    krol_nbt::read_gzip(std::io::BufReader::new(file))
        .ok()
        .map(|(_, c)| c)
}

/// A position and a (yaw, pitch) rotation.
pub type Placement = ((f64, f64, f64), (f32, f32));

/// Position and rotation stored in player data.
pub fn stored_position(data: &Compound) -> Option<Placement> {
    let pos = data.get_list("Pos", TagType::Double)?;
    let rot = data.get_list("Rotation", TagType::Float);
    let d = |i: usize| pos.get(i).and_then(Tag::as_f64);
    let r = |i: usize| {
        rot.and_then(|r| r.get(i))
            .and_then(Tag::as_f32)
            .unwrap_or(0.0)
    };
    Some(((d(0)?, d(1)?, d(2)?), (r(0), r(1))))
}

/// The player data compound vanilla 1.8.8 writes for a fresh player (recorded from a
/// vanilla `playerdata` file); position, rotation and identity are filled in on save.
pub fn default_player_data(gamemode: i32) -> Compound {
    let mut c = Compound::new();
    c.insert("HurtByTimestamp", Tag::Int(0));
    c.insert("SleepTimer", Tag::Short(0));
    let mut attributes = List::new();
    for (name, base) in [
        ("generic.maxHealth", 20.0),
        ("generic.knockbackResistance", 0.0),
        ("generic.movementSpeed", f64::from(0.1f32)),
        ("generic.attackDamage", 1.0),
    ] {
        let mut a = Compound::new();
        a.insert("Base", Tag::Double(base));
        a.insert("Name", Tag::String(name.into()));
        attributes.push(Tag::Compound(a)).expect("homogeneous");
    }
    c.insert("Attributes", Tag::List(attributes));
    c.insert("Invulnerable", Tag::Byte(0));
    c.insert("PortalCooldown", Tag::Int(0));
    c.insert("AbsorptionAmount", Tag::Float(0.0));
    let creative = gamemode == 1;
    let spectator = gamemode == 3;
    let mut abilities = Compound::new();
    abilities.insert("invulnerable", Tag::Byte(i8::from(creative || spectator)));
    abilities.insert("mayfly", Tag::Byte(i8::from(creative || spectator)));
    abilities.insert("instabuild", Tag::Byte(i8::from(creative)));
    abilities.insert("walkSpeed", Tag::Float(0.1));
    abilities.insert("mayBuild", Tag::Byte(i8::from(gamemode == 0 || creative)));
    abilities.insert("flying", Tag::Byte(i8::from(spectator)));
    abilities.insert("flySpeed", Tag::Float(0.05));
    c.insert("abilities", Tag::Compound(abilities));
    c.insert("FallDistance", Tag::Float(0.0));
    c.insert("DeathTime", Tag::Short(0));
    c.insert("XpSeed", Tag::Int(0));
    c.insert("HealF", Tag::Float(20.0));
    c.insert("XpTotal", Tag::Int(0));
    c.insert("playerGameType", Tag::Int(gamemode));
    c.insert("Motion", doubles(&[0.0, -0.0784000015258789, 0.0]));
    c.insert("Health", Tag::Short(20));
    c.insert("foodSaturationLevel", Tag::Float(5.0));
    c.insert("Air", Tag::Short(300));
    c.insert("OnGround", Tag::Byte(1));
    c.insert("Dimension", Tag::Int(0));
    c.insert("XpLevel", Tag::Int(0));
    c.insert("Score", Tag::Int(0));
    c.insert("Sleeping", Tag::Byte(0));
    c.insert("Fire", Tag::Short(-20));
    c.insert("XpP", Tag::Float(0.0));
    c.insert("EnderItems", Tag::List(List::new()));
    c.insert("foodLevel", Tag::Int(20));
    c.insert("foodExhaustionLevel", Tag::Float(0.0));
    c.insert("HurtTime", Tag::Short(0));
    c.insert("SelectedItemSlot", Tag::Int(0));
    c.insert("Inventory", Tag::List(List::new()));
    c.insert("foodTickTimer", Tag::Int(0));
    c
}
