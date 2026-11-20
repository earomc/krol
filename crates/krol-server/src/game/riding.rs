//! Players riding minecarts, boats and pigs (vanilla: `Entity.mountEntity`, `updateRidden`,
//! `updateRiderPosition`, `EntityLivingBase.dismountEntity`, `EntityPlayerMP.mountEntity`,
//! the riding branch of `NetHandlerPlayServer.processPlayer`, `processInput`, and the
//! `Riding` tag of player data).
//!
//! A rider leaves the entity loop: right after its vehicle updates, it updates itself
//! (`updateRidden`) and is put back on the vehicle. Its movement packets only bring its
//! look and its own update; its position follows the vehicle. Sneaking gets it out, onto
//! a free spot next to the vehicle. A player who logs out in a vehicle takes it along:
//! it is saved in the player's data and comes back when the player does.
//!
//! Mobs ride too: a moving rideable minecart picks up a mob it bumps into, and mobs
//! loaded or summoned with a `Riding` tag sit on their mount. A riding mob runs its whole
//! update (AI included) from rest, then sits on its vehicle again; it is saved with its
//! vehicle in its `Riding` tag, and the vehicle is not saved on its own.
//!
//! TODO(M5): armor stands riding, horses, the rider's own move while riding (block
//! collisions under the vehicle), a player's pig that rides a cart coming back with it.

use krol_nbt::{Compound, Tag};
use krol_protocol::packets::play::ClientboundPlay;

use super::Game;
use super::physics::{Aabb, Body};
use super::placement::solid_top;
use super::saved_entities::EntityBase;

/// `EntityPlayer.getYOffset`: a riding player sits this far below the mount point.
const PLAYER_Y_OFFSET: f64 = -0.35;
/// `Entity.getYOffset` of mobs: they sit on the mount point.
const MOB_Y_OFFSET: f64 = 0.0;
/// `EntityMinecart.getMountedYOffset`.
const MINECART_MOUNT_OFFSET: f64 = 0.0;
/// `Entity.getMountedYOffset` of mobs: this share of their height.
const MOB_MOUNT_OFFSET: f64 = 0.75;
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
    /// The rider of the vehicle `v` (a minecart or a boat).
    pub(super) fn vehicle_rider(&self, v: i32) -> Option<i32> {
        match self.minecarts.get(&v) {
            Some(m) => m.rider,
            None => match self.boats.get(&v) {
                Some(b) => b.rider,
                None => self.mobs.get(&v).and_then(|m| m.rider),
            },
        }
    }

    /// Sets the rider of the vehicle `v`.
    pub(super) fn set_vehicle_rider(&mut self, v: i32, rider: Option<i32>) {
        if let Some(m) = self.minecarts.get_mut(&v) {
            m.rider = rider;
        } else if let Some(b) = self.boats.get_mut(&v) {
            b.rider = rider;
        } else if let Some(m) = self.mobs.get_mut(&v) {
            m.rider = rider;
        }
    }

    /// Whether the vehicle `v` is dead (or gone).
    pub(super) fn vehicle_dead(&self, v: i32) -> bool {
        match self.minecarts.get(&v) {
            Some(m) => m.dead,
            None => match self.boats.get(&v) {
                Some(b) => b.dead,
                None => self.mobs.get(&v).is_none_or(|m| m.dead),
            },
        }
    }

    /// The body and base state of the vehicle `v`.
    pub(super) fn vehicle_state(&self, v: i32) -> Option<(&Body, &EntityBase)> {
        match self.minecarts.get(&v) {
            Some(m) => Some((&m.body, &m.base)),
            None => match self.boats.get(&v) {
                Some(b) => Some((&b.body, &b.base)),
                None => self.mobs.get(&v).map(|m| (&m.body, &m.base)),
            },
        }
    }

    /// Marks the vehicle `v` airborne (its move goes out at once).
    fn set_vehicle_airborne(&mut self, v: i32) {
        if let Some(m) = self.minecarts.get_mut(&v) {
            m.airborne = true;
        } else if let Some(b) = self.boats.get_mut(&v) {
            b.airborne = true;
        } else if let Some(m) = self.mobs.get_mut(&v) {
            m.airborne = true;
        }
    }

    /// Where a rider of the vehicle `v` sits (`updateRiderPosition`), `y_offset` being
    /// the rider's `getYOffset`.
    fn rider_spot(&self, v: i32, y_offset: f64) -> Option<(f64, f64, f64)> {
        if let Some(b) = self.boats.get(&v) {
            return Some(b.seat(y_offset));
        }
        if let Some(m) = self.mobs.get(&v) {
            // Entity.getMountedYOffset: three quarters up.
            let p = m.body.pos;
            let top = f64::from(m.body.height) * MOB_MOUNT_OFFSET;
            return Some((p.0, p.1 + top + y_offset, p.2));
        }
        let m = self.minecarts.get(&v)?;
        let p = m.body.pos;
        Some((p.0, p.1 + MINECART_MOUNT_OFFSET + y_offset, p.2))
    }

    /// `updateRiderPosition`: the vehicle's rider sits on it.
    pub(super) fn update_rider_position(&mut self, cart: i32) {
        let Some(rider) = self.vehicle_rider(cart) else {
            return;
        };
        if let Some(i) = self.players.iter().position(|p| p.entity_id == rider) {
            if let Some(spot) = self.rider_spot(cart, PLAYER_Y_OFFSET) {
                self.players[i].pos = spot;
            }
        } else if self.mobs.contains_key(&rider) {
            // Mobs sit on the mount point (getYOffset 0).
            if let Some(spot) = self.rider_spot(cart, MOB_Y_OFFSET)
                && let Some(m) = self.mobs.get_mut(&rider)
            {
                m.body.pos = spot;
            }
        }
    }

    /// `Entity.mountEntity(vehicle)` for a mob (a moving rideable cart picking it
    /// up); the tracker tells the viewers.
    pub(super) fn mount_mob(&mut self, mob: i32, vehicle: i32) {
        // Not onto a vehicle that rides it, however deep.
        let mut v = Some(vehicle);
        while let Some(at) = v {
            if at == mob {
                return;
            }
            v = self.mobs.get(&at).and_then(|m| m.riding);
        }
        if let Some(old) = self.mobs.get(&mob).and_then(|m| m.riding) {
            self.set_vehicle_rider(old, None);
        }
        if let Some(m) = self.mobs.get_mut(&mob) {
            m.riding = Some(vehicle);
        }
        self.set_vehicle_rider(vehicle, Some(mob));
    }

    /// `EntityLivingBase.mountEntity(null)` for a mob: it gets off onto a free spot
    /// next to its vehicle (`dismountEntity`); the tracker tells the viewers.
    fn dismount_mob(&mut self, mob: i32) {
        let Some(vehicle) = self.mobs.get(&mob).and_then(|m| m.riding) else {
            return;
        };
        if let Some((body, _)) = self.vehicle_state(vehicle)
            && let Some(m) = self.mobs.get(&mob)
        {
            let (vpos, vbb, vheight) = (body.pos, body.bb(), body.height);
            let (pos, bb) = (m.body.pos, m.body.bb());
            let spot = self.dismount_spot(pos, &bb, vpos, vbb.min[1] + f64::from(vheight));
            // setPositionAndUpdate: setLocationAndAngles with its own rotation.
            let rotation = self.mobs.get(&mob).map(|m| (m.base.yaw, m.base.pitch));
            if let Some(rotation) = rotation {
                self.set_location_and_angles(mob, spot, rotation);
            }
        }
        self.set_vehicle_rider(vehicle, None);
        if let Some(m) = self.mobs.get_mut(&mob) {
            m.riding = None;
        }
    }

    /// A vehicle leaving the world lets go of its rider (whose vehicle it still is).
    pub(super) fn let_go_of_rider(&mut self, vehicle: i32, rider: i32) {
        if let Some(p) = self.players.iter_mut().find(|p| p.entity_id == rider)
            && p.riding == Some(vehicle)
        {
            p.riding = None;
        }
        if let Some(m) = self.mobs.get_mut(&rider)
            && m.riding == Some(vehicle)
        {
            m.riding = None;
        }
    }

    /// The vehicle a player or mob rides, if any (`ridingEntity`).
    pub(super) fn entity_vehicle(&self, id: i32) -> Option<i32> {
        match self.players.iter().find(|p| p.entity_id == id) {
            Some(p) => p.riding,
            None => self.mobs.get(&id).and_then(|m| m.riding),
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
        if let Some(prev) = old {
            self.set_vehicle_rider(prev, None);
        }
        let me = self.players[i].entity_id;
        self.set_vehicle_rider(cart, Some(me));
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
        if let Some((body, _)) = self.vehicle_state(cart) {
            let (vpos, vbb, vheight) = (body.pos, body.bb(), body.height);
            let pos = self.players[i].pos;
            let bb = Aabb::at(pos, 0.6, 1.8);
            let spot = self.dismount_spot(pos, &bb, vpos, vbb.min[1] + f64::from(vheight));
            let p = &mut self.players[i];
            let rot = p.rotation;
            // setPositionAndUpdate.
            Self::set_player_location(p, spot, rot);
        }
        self.set_vehicle_rider(cart, None);
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
        let Some(rider) = self.vehicle_rider(cart) else {
            return;
        };
        match self.players.iter().position(|p| p.entity_id == rider) {
            Some(i) if self.players[i].riding == Some(cart) => self.dismount_player(i),
            _ if self
                .mobs
                .get(&rider)
                .is_some_and(|m| m.riding == Some(cart)) =>
            {
                self.dismount_mob(rider);
            }
            _ => self.set_vehicle_rider(cart, None),
        }
    }

    /// `EntityLivingBase.dismountEntity` for a rider at `pos` with the box `bb`: the
    /// first block space next to it (one up) free of blocks over a solid top, else the
    /// last one over something solid or water, else the top of the vehicle.
    fn dismount_spot(
        &mut self,
        pos: (f64, f64, f64),
        bb: &Aabb,
        vehicle: (f64, f64, f64),
        top: f64,
    ) -> (f64, f64, f64) {
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
                && let Some((body, base)) = self.vehicle_state(cart)
            {
                let packet = ClientboundPlay::EntityTeleport {
                    entity_id: cart,
                    x: krol_java::math::floor_f64(body.pos.0 * 32.0),
                    y: krol_java::math::floor_f64(body.pos.1 * 32.0),
                    z: krol_java::math::floor_f64(body.pos.2 * 32.0),
                    yaw: (base.yaw * 256.0 / 360.0) as i32 as i8,
                    pitch: (base.pitch * 256.0 / 360.0) as i32 as i8,
                    on_ground: body.on_ground,
                };
                let p = &mut self.players[i];
                p.send(packet);
                let (pos, rot) = (p.pos, p.rotation);
                Self::set_player_location(p, pos, rot);
            }
            self.set_vehicle_airborne(cart);
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
        let Some(rider) = self.vehicle_rider(cart) else {
            return;
        };
        let i = self.players.iter().position(|p| {
            p.entity_id == rider && !p.removed && !p.dead_marked && p.riding == Some(cart)
        });
        let Some(i) = i else {
            // A mob rider (World.updateEntity(riddenByEntity)).
            if self
                .mobs
                .get(&rider)
                .is_some_and(|m| !m.dead && m.riding == Some(cart))
            {
                self.update_mob_entity(rider);
                return;
            }
            self.set_vehicle_rider(cart, None);
            if let Some(m) = self.mobs.get_mut(&rider)
                && m.riding == Some(cart)
            {
                m.riding = None;
            }
            return;
        };
        if self.vehicle_dead(cart) {
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
        if cm > 0 && self.boats.contains_key(&cart) {
            self.add_stat(i, "stat.boatOneCm", cm);
        } else if cm > 0 && self.mobs.contains_key(&cart) {
            // Only pigs can be ridden so far.
            self.add_stat(i, "stat.pigOneCm", cm);
        } else if cm > 0 {
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
        self.mount_nbt(self.players[i].riding?)
    }

    /// `writeMountToNBT` of the vehicle `cart`: its NBT with its id, if it is alive.
    pub(super) fn mount_nbt(&self, cart: i32) -> Option<Compound> {
        let uuid = self.entity_uuids.get(&cart).copied().unwrap_or_default();
        let mut c = Compound::new();
        if let Some(b) = self.boats.get(&cart) {
            if b.dead {
                return None;
            }
            self.boat_nbt(b, uuid, &mut c);
            return Some(c);
        }
        if let Some(m) = self.mobs.get(&cart) {
            return (!m.dead).then(|| self.mob_nbt(m, uuid));
        }
        let m = self.minecarts.get(&cart).filter(|m| !m.dead)?;
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
        if self.minecarts.remove(&cart).is_some()
            || self.boats.remove(&cart).is_some()
            || self.mobs.remove(&cart).is_some()
        {
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
        let saved_id = saved.get_str("id");
        if super::mob::MobKind::from_saved_id(saved_id).is_some() {
            // Entities saved in chunks loaded since are created first.
            if self.level.overworld.has_loaded_entities() {
                self.load_chunk_entities();
            }
            let Some((id, _)) = self.create_entity_from_nbt(&saved) else {
                return;
            };
            self.force_spawn = Some(id);
            self.spawn_entity_in_world(id);
            self.mount_player(i, id);
            self.force_spawn = None;
            return;
        }
        if saved_id != super::minecart::RIDEABLE_ID && saved_id != super::boat::SAVED_ID {
            // TODO(M5): other vehicles (horses).
            return;
        }
        let id = self.next_entity_id();
        if saved_id == super::boat::SAVED_ID {
            let (boat, uuid) = self.boat_from_nbt(&saved);
            if let Some(uuid) = uuid {
                self.entity_uuids.insert(id, uuid);
            }
            self.boats.insert(id, boat);
        } else {
            let (cart, uuid) = self.minecart_from_nbt(&saved);
            if let Some(uuid) = uuid {
                self.entity_uuids.insert(id, uuid);
            }
            self.minecarts.insert(id, cart);
        }
        self.force_spawn = Some(id);
        self.spawn_entity_in_world(id);
        self.mount_player(i, id);
        self.force_spawn = None;
    }
}
