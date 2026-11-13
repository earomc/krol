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
    /// Chunks the client has received.
    pub sent_chunks: HashSet<(i32, i32)>,
    /// Chunks waiting to be sent, nearest first.
    pub chunk_queue: VecDeque<(i32, i32)>,
    /// The chunk the tracking area is centered on.
    pub tracking_center: (i32, i32),
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

    /// Chunk coordinates the player stands in.
    pub fn chunk_pos(&self) -> (i32, i32) {
        (
            krol_java::math::floor_f64(self.pos.0) >> 4,
            krol_java::math::floor_f64(self.pos.2) >> 4,
        )
    }

    /// Recomputes the set of chunks in view (a square of `radius` around the player)
    /// and queues missing ones nearest-first. Returns chunks to unload.
    pub fn update_tracked_chunks(&mut self, radius: i32) -> Vec<(i32, i32)> {
        let (cx, cz) = self.chunk_pos();
        self.tracking_center = (cx, cz);
        let in_view = |(x, z): (i32, i32)| (x - cx).abs() <= radius && (z - cz).abs() <= radius;
        let unload: Vec<(i32, i32)> = self
            .sent_chunks
            .iter()
            .copied()
            .filter(|&c| !in_view(c))
            .collect();
        for c in &unload {
            self.sent_chunks.remove(c);
        }
        self.chunk_queue.retain(|&c| in_view(c));
        let mut wanted: Vec<(i32, i32)> = Vec::new();
        for x in cx - radius..=cx + radius {
            for z in cz - radius..=cz + radius {
                if !self.sent_chunks.contains(&(x, z)) && !self.chunk_queue.contains(&(x, z)) {
                    wanted.push((x, z));
                }
            }
        }
        wanted.sort_by_key(|&(x, z)| (x - cx) * (x - cx) + (z - cz) * (z - cz));
        self.chunk_queue.extend(wanted);
        unload
    }

    /// Writes the player's data to `<world>/playerdata/<uuid>.dat` (vanilla:
    /// `writePlayerData`: `<uuid>.dat.tmp`, then replace).
    pub fn save(&mut self, world_dir: &Path) {
        self.store_state();
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

    fn store_state(&mut self) {
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
