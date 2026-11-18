//! Players riding minecarts (vanilla: `Entity.mountEntity`, `updateRidden`,
//! `updateRiderPosition`, `EntityLivingBase.dismountEntity`, `EntityPlayerMP.mountEntity`,
//! the riding branch of `NetHandlerPlayServer.processPlayer`, `processInput`, and the
//! `Riding` tag of player data).
//!
//! A rider leaves the entity loop: right after its vehicle updates, it updates itself
//! (`updateRidden`) and is put back on the vehicle. Its movement packets only bring its
//! look and its own update; its position follows the vehicle. Sneaking gets it out, onto
//! a free spot next to the vehicle. A player who logs out in a minecart takes it along:
//! the cart is saved in the player's data and comes back when the player does.
//!
//! TODO(M5): mobs and armor stands riding (moving rideable carts pick them up), boats,
//! pigs and horses, a rider steering (`moveForward` only starts a resting cart), the
//! rider's own move while riding (block collisions under the cart).

use krol_nbt::{Compound, Tag};
use krol_protocol::packets::play::ClientboundPlay;

use super::Game;
use super::physics::Aabb;
use super::placement::solid_top;

/// `EntityPlayer.getYOffset`: a riding player sits this far below the mount point.
const PLAYER_Y_OFFSET: f64 = -0.35;
/// `EntityMinecart.getMountedYOffset`.
const MINECART_MOUNT_OFFSET: f64 = 0.0;
/// The on-a-rail achievement needs this squared distance from where riding started.
const ON_A_RAIL_DISTANCE_SQ: f64 = 1_000_000.0;
/// `C0CPacketInput` flags.
const STEER_JUMP: u8 = 1;
const STEER_UNMOUNT: u8 = 2;
/// The spectator game mode.
const SPECTATOR: i32 = 3;
/// Water stops a dismount fall (`Material.water`).
const WATER: u16 = 8;
const FLOWING: u16 = 9;

impl Game {
    /// Where a rider of the cart `cart` sits (`updateRiderPosition`).
    fn rider_spot(&self, cart: i32) -> Option<(f64, f64, f64)> {
        let m = self.minecarts.get(&cart)?;
        let p = m.body.pos;
        Some((p.0, p.1 + MINECART_MOUNT_OFFSET + PLAYER_Y_OFFSET, p.2))
    }

    /// `updateRiderPosition`: the cart's rider sits on it.
    pub(super) fn update_rider_position(&mut self, cart: i32) {
        let Some(rider) = self.minecarts.get(&cart).and_then(|m| m.rider) else {
            return;
        };
        let Some(spot) = self.rider_spot(cart) else {
            return;
        };
        if let Some(p) = self.players.iter_mut().find(|p| p.entity_id == rider) {
            p.pos = spot;
        }
    }

    /// `EntityMinecartEmpty.interactFirst`: an empty cart (or one the player is in)
    /// takes the player in.
    pub(super) fn interact_minecart(&mut self, i: usize, cart: i32) {
        // EntityMinecartContainer.interactFirst: container carts open their window
        // (for spectators too, EntityPlayer.interactWith).
        if self
            .minecarts
            .get(&cart)
            .is_some_and(|m| m.kind.slots() > 0)
        {
            self.open_cart_window(i, cart);
            return;
        }
        // EntityPlayer.interactWith: spectators only open inventories.
        if self.players[i].gamemode == SPECTATOR {
            return;
        }
        let Some(m) = self.minecarts.get(&cart) else {
            return;
        };
        match m.kind {
            super::minecart::CartKind::Furnace => {
                self.interact_furnace_cart(i, cart);
                return;
            }
            // Entity.interactFirst: TNT carts do nothing.
            super::minecart::CartKind::Tnt => return,
            _ => {}
        }
        let me = self.players[i].entity_id;
        if m.rider.is_some_and(|r| r != me) {
            return;
        }
        self.mount_player(i, cart);
    }

    /// `EntityPlayerMP.mountEntity(cart)`: the player gets in (out of anything else),
    /// told with Attach Entity and a teleport to where it is.
    pub(super) fn mount_player(&mut self, i: usize, cart: i32) {
        let old = self.players[i].riding;
        if let Some(prev) = old
            && let Some(m) = self.minecarts.get_mut(&prev)
        {
            m.rider = None;
        }
        let me = self.players[i].entity_id;
        if let Some(m) = self.minecarts.get_mut(&cart) {
            m.rider = Some(me);
        }
        self.players[i].riding = Some(cart);
        if old != Some(cart) {
            self.players[i].send(ClientboundPlay::AttachEntity {
                entity_id: me,
                vehicle_id: cart,
                leash: false,
            });
            let p = &mut self.players[i];
            let (pos, rot) = (p.pos, p.rotation);
            Self::set_player_location(p, pos, rot);
        }
    }

    /// `EntityPlayerMP.mountEntity(null)`: the player gets out onto a free spot next to
    /// its vehicle (`dismountEntity`), told with Attach Entity and a teleport.
    pub(super) fn dismount_player(&mut self, i: usize) {
        let Some(cart) = self.players[i].riding else {
            return;
        };
        if let Some(m) = self.minecarts.get(&cart) {
            let (vpos, vbb, vheight) = (m.body.pos, m.body.bb(), m.body.height);
            let spot = self.dismount_spot(i, vpos, vbb.min[1] + f64::from(vheight));
            let p = &mut self.players[i];
            let rot = p.rotation;
            // setPositionAndUpdate.
            Self::set_player_location(p, spot, rot);
        }
        if let Some(m) = self.minecarts.get_mut(&cart) {
            m.rider = None;
        }
        let p = &mut self.players[i];
        p.riding = None;
        let me = p.entity_id;
        p.send(ClientboundPlay::AttachEntity {
            entity_id: me,
            vehicle_id: -1,
            leash: false,
        });
        let (pos, rot) = (p.pos, p.rotation);
        Self::set_player_location(p, pos, rot);
    }

    /// `riddenByEntity.mountEntity(null)` from the vehicle's side (it breaks, or an
    /// activator rail throws the rider out).
    pub(super) fn dismount_rider_of(&mut self, cart: i32) {
        let Some(rider) = self.minecarts.get(&cart).and_then(|m| m.rider) else {
            return;
        };
        match self.players.iter().position(|p| p.entity_id == rider) {
            Some(i) if self.players[i].riding == Some(cart) => self.dismount_player(i),
            _ => {
                if let Some(m) = self.minecarts.get_mut(&cart) {
                    m.rider = None;
                }
            }
        }
    }

    /// `EntityLivingBase.dismountEntity`: the first block space next to the player (one
    /// up) free of blocks over a solid top, else the last one over something solid or
    /// water, else the top of the vehicle.
    fn dismount_spot(&mut self, i: usize, vehicle: (f64, f64, f64), top: f64) -> (f64, f64, f64) {
        let pos = self.players[i].pos;
        let bb = Aabb::at(pos, 0.6, 1.8);
        let mut spot = (vehicle.0, top, vehicle.2);
        for dx in -1i32..=1 {
            for dz in -1i32..1 {
                if dx == 0 && dz == 0 {
                    continue;
                }
                // Vanilla truncates the coordinates towards zero here.
                let x = (pos.0 + f64::from(dx)) as i32;
                let z = (pos.2 + f64::from(dz)) as i32;
                let y = pos.1 as i32;
                let moved = bb.offset([f64::from(dx), 1.0, f64::from(dz)]);
                if !super::physics::colliding_boxes(&mut self.level.overworld, &moved).is_empty() {
                    continue;
                }
                let at = (pos.0 + f64::from(dx), pos.1 + 1.0, pos.2 + f64::from(dz));
                if solid_top(self.level.overworld.block(x, y, z)) {
                    return at;
                }
                let below = self.level.overworld.block(x, y - 1, z);
                if solid_top(below) || matches!(below >> 4, WATER | FLOWING) {
                    spot = at;
                }
            }
        }
        spot
    }

    /// `EntityPlayerMP.setEntityActionState` (Steer Vehicle): a rider's input and
    /// sneaking.
    pub(super) fn steer_vehicle(&mut self, i: usize, sideways: f32, forward: f32, flags: u8) {
        let p = &mut self.players[i];
        if p.riding.is_none() {
            return;
        }
        if (-1.0..=1.0).contains(&sideways) {
            p.move_strafing = sideways;
        }
        if (-1.0..=1.0).contains(&forward) {
            p.move_forward = forward;
        }
        p.jumping = flags & STEER_JUMP != 0;
        let sneaking = flags & STEER_UNMOUNT != 0;
        if p.sneaking != sneaking {
            p.sneaking = sneaking;
            p.dirty_metadata.insert(0);
        }
    }

    /// The riding branch of `processPlayer`: the player sits on its vehicle, turns as
    /// the packet says, and runs its own update; a client that thinks it is far away
    /// is put back.
    pub(super) fn riding_movement(
        &mut self,
        i: usize,
        position: Option<(f64, f64, f64)>,
        rotation: Option<(f32, f32)>,
        on_ground: bool,
    ) {
        let Some(cart) = self.players[i].riding else {
            return;
        };
        let moved_sq = position.map_or(0.0, |p| {
            let l = self.players[i].movement.last_pos;
            (p.0 - l.0).powi(2) + (p.1 - l.1).powi(2) + (p.2 - l.2).powi(2)
        });
        self.update_rider_position(cart);
        let seat = self.players[i].pos;
        let rot = rotation.unwrap_or(self.players[i].rotation);
        self.players[i].on_ground = on_ground;
        // onUpdateEntity.
        self.update_carried_maps(i);
        self.living_move(i);
        self.player_update(i);
        self.send_map_packets(i);
        self.send_health_if_changed(i);
        let p = &mut self.players[i];
        p.pos = seat;
        p.rotation = rot;
        if let Some(cart) = self.players[i].riding {
            self.update_rider_position(cart);
        }
        self.update_player_chunks(i);
        if let Some(cart) = self.players[i].riding {
            if moved_sq > 4.0
                && let Some(m) = self.minecarts.get(&cart)
            {
                let packet = ClientboundPlay::EntityTeleport {
                    entity_id: cart,
                    x: krol_java::math::floor_f64(m.body.pos.0 * 32.0),
                    y: krol_java::math::floor_f64(m.body.pos.1 * 32.0),
                    z: krol_java::math::floor_f64(m.body.pos.2 * 32.0),
                    yaw: (m.base.yaw * 256.0 / 360.0) as i32 as i8,
                    pitch: (m.base.pitch * 256.0 / 360.0) as i32 as i8,
                    on_ground: m.body.on_ground,
                };
                let p = &mut self.players[i];
                p.send(packet);
                let (pos, rot) = (p.pos, p.rotation);
                Self::set_player_location(p, pos, rot);
            }
            if let Some(m) = self.minecarts.get_mut(&cart) {
                m.airborne = true;
            }
        }
        let p = &mut self.players[i];
        p.movement.last_pos = p.pos;
        // World.updateEntity(player): forced, so the rider runs updateRidden once more
        // (its server tick, zero motion, back on the seat, the distance ridden).
        let radius = self.config.view_distance;
        if self.players[i].riding.is_some() {
            self.update_rider(i, radius);
        } else {
            self.tick_player(i, radius);
        }
        let p = &mut self.players[i];
        p.entity_chunk = (
            krol_java::math::floor_f64(p.pos.0) >> 4,
            krol_java::math::floor_f64(p.pos.2) >> 4,
        );
    }

    /// The end of a vehicle's `updateEntityWithOptionalForce`: its rider updates if it
    /// still rides it (one whose vehicle died in its update just gets off), else both
    /// let go silently.
    pub(super) fn update_vehicle_rider(&mut self, cart: i32) {
        let Some(rider) = self.minecarts.get(&cart).and_then(|m| m.rider) else {
            return;
        };
        let i = self.players.iter().position(|p| {
            p.entity_id == rider && !p.removed && !p.dead_marked && p.riding == Some(cart)
        });
        let Some(i) = i else {
            if let Some(m) = self.minecarts.get_mut(&cart) {
                m.rider = None;
            }
            return;
        };
        if self.minecarts.get(&cart).is_some_and(|m| m.dead) {
            // Entity.updateRidden: a dead vehicle is let go.
            self.players[i].riding = None;
            return;
        }
        let radius = self.config.view_distance;
        self.update_rider(i, radius);
    }

    /// The rider's turn right after its vehicle's (`updateEntity(riddenByEntity)` with
    /// `EntityPlayer.updateRidden`): sneaking gets it out; otherwise it updates with no
    /// motion, sits on the vehicle again and counts the distance ridden.
    pub(super) fn update_rider(&mut self, i: usize, radius: i32) {
        if self.players[i].sneaking {
            self.dismount_player(i);
            let p = &mut self.players[i];
            p.sneaking = false;
            p.dirty_metadata.insert(0);
            return;
        }
        let start = self.players[i].pos;
        self.players[i].motion = (0.0, 0.0, 0.0);
        self.tick_player(i, radius);
        let Some(cart) = self.players[i].riding else {
            return;
        };
        self.update_rider_position(cart);
        let p = &self.players[i];
        let d = (p.pos.0 - start.0, p.pos.1 - start.1, p.pos.2 - start.2);
        let cm =
            (krol_java::math::sqrt_f64(d.0 * d.0 + d.1 * d.1 + d.2 * d.2) * 100.0).round() as i32;
        if cm > 0 {
            self.add_stat(i, "stat.minecartOneCm", cm);
            let p = &mut self.players[i];
            let block = (
                krol_java::math::floor_f64(p.pos.0),
                krol_java::math::floor_f64(p.pos.1),
                krol_java::math::floor_f64(p.pos.2),
            );
            match p.minecart_start {
                None => p.minecart_start = Some(block),
                Some(s) => {
                    let sq = f64::from(s.0 - block.0).powi(2)
                        + f64::from(s.1 - block.1).powi(2)
                        + f64::from(s.2 - block.2).powi(2);
                    if sq >= ON_A_RAIL_DISTANCE_SQ {
                        // TODO(M5): the onARail achievement (achievements are missing).
                    }
                }
            }
        }
        let p = &mut self.players[i];
        p.entity_chunk = (
            krol_java::math::floor_f64(p.pos.0) >> 4,
            krol_java::math::floor_f64(p.pos.2) >> 4,
        );
    }

    /// The vehicle's NBT as the player's `Riding` tag (`writeMountToNBT`), if it rides
    /// one that is alive.
    fn riding_nbt(&self, i: usize) -> Option<Compound> {
        let cart = self.players[i].riding?;
        let m = self.minecarts.get(&cart).filter(|m| !m.dead)?;
        let uuid = self.entity_uuids.get(&cart).copied().unwrap_or_default();
        let mut c = Compound::new();
        self.minecart_nbt(m, uuid, &mut c);
        Some(c)
    }

    /// Writes the player's data with its `Riding` tag.
    pub(super) fn save_player(&mut self, i: usize) {
        match self.riding_nbt(i) {
            Some(c) => {
                self.players[i].data.insert("Riding", Tag::Compound(c));
            }
            None => {
                self.players[i].data.remove("Riding");
            }
        }
        let dir = self.level.dir.clone();
        self.players[i].save(&dir);
    }

    /// `playerLoggedOut`: the vehicle of a player leaving goes with it
    /// (`removePlayerEntityDangerously`).
    pub(super) fn remove_mount_of_leaving(&mut self, i: usize) {
        let Some(cart) = self.players[i].riding.take() else {
            return;
        };
        if self.minecarts.remove(&cart).is_some() {
            self.remove_entity_from_chunk(cart);
            self.untrack_entity(cart);
        }
    }

    /// `initializeConnectionToPlayer`: a player saved in a vehicle gets it back (shown
    /// at once, even before its chunk) and is put in it.
    pub(super) fn restore_riding(&mut self, i: usize) {
        let Some(Tag::Compound(saved)) = self.players[i].data.remove("Riding") else {
            return;
        };
        if saved.get_str("id") != super::minecart::RIDEABLE_ID {
            // TODO(M5): other vehicles.
            return;
        }
        let id = self.next_entity_id();
        let (cart, uuid) = self.minecart_from_nbt(&saved);
        if let Some(uuid) = uuid {
            self.entity_uuids.insert(id, uuid);
        }
        self.minecarts.insert(id, cart);
        self.force_spawn = Some(id);
        self.spawn_entity_in_world(id);
        self.mount_player(i, id);
        self.force_spawn = None;
    }
}
