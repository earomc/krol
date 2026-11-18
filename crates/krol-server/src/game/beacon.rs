//! Beacons (vanilla: `BlockBeacon`, `TileEntityBeacon`, `ContainerBeacon`, `MC|Beacon`):
//! the pyramid levels and the beam check every 80 ticks and on neighbor changes, the
//! effects for players in range, the window with its payment slot, and choosing the
//! powers.
//!
//! TODO(M4): stained glass placed or broken above a beacon (`updateColorAsync`), custom
//! names from renamed items (the window title), the `fullBeacon` achievement.

use krol_protocol::packets::play::{ClientboundPlay, ItemStack};

use super::Game;
use super::container::{Layout, OpenContainer};
use super::potions::{Effect, effect};

/// `beacon`.
pub const BEACON: u16 = 138;
/// The payment slot.
pub const SLOTS: usize = 1;
/// `TileEntityBeacon.update` runs the beacon every 80 ticks of world time.
const UPDATE_PERIOD: i64 = 80;
/// The block event that makes a beacon update again (`receiveClientEvent` 1).
const EVENT_UPDATE: u8 = 1;
/// Pyramid layers count up to this.
const MAX_LEVELS: i32 = 4;
/// Blocks a pyramid is built from: iron, gold, diamond and emerald blocks.
const PYRAMID_BLOCKS: [u16; 4] = [42, 41, 57, 133];
/// What lets the beam through without counting as glass: stained glass and panes color
/// it, other blocks below full opacity and bedrock let it pass.
const STAINED_GLASS: u16 = 95;
const STAINED_GLASS_PANE: u16 = 160;
const BEDROCK: u16 = 7;
const FULL_OPACITY: i32 = 15;
/// Effects last 9 seconds and are renewed every 4.
const EFFECT_TICKS: i32 = 180;
/// The range is 10 blocks per level plus 10.
const RANGE_PER_LEVEL: i32 = 10;
/// Effects the window offers (`func_183001_h` accepts only these).
const POWERS: [u8; 6] = [
    effect::SPEED,
    effect::HASTE,
    effect::RESISTANCE,
    JUMP_BOOST,
    effect::STRENGTH,
    effect::REGENERATION,
];
/// `jump`.
const JUMP_BOOST: u8 = 8;
/// Items that pay for a power: emeralds, diamonds, gold and iron ingots.
const PAYMENTS: [i16; 4] = [388, 264, 266, 265];

/// `ContainerBeacon.BeaconSlot.isItemValid`.
pub fn is_payment(stack: &ItemStack) -> bool {
    PAYMENTS.contains(&stack.id)
}

/// `TileEntityBeacon.func_183001_h`: a power the beacon may give, else 0.
fn valid_power(id: i32) -> i32 {
    if POWERS.iter().any(|&p| i32::from(p) == id) {
        id
    } else {
        0
    }
}

impl Game {
    /// `TileEntityBeacon.update`: the beacon runs when the world time is a multiple of 80.
    pub(super) fn tick_beacon(&mut self, pos: (i32, i32, i32)) {
        if self.level.overworld.total_time.rem_euclid(UPDATE_PERIOD) == 0 {
            self.update_beacon(pos);
        }
    }

    /// `TileEntityBeacon.updateBeacon`: the beam and the pyramid, then the effects.
    pub(super) fn update_beacon(&mut self, pos: (i32, i32, i32)) {
        let w = &mut self.level.overworld;
        // updateSegmentColors: the beam must reach the sky through blocks below full
        // opacity (bedrock lets it through too).
        let mut complete = (pos.1 + 1..256).all(|y| {
            let id = w.block(pos.0, y, pos.2) >> 4;
            id == STAINED_GLASS
                || id == STAINED_GLASS_PANE
                || id == BEDROCK
                || super::light_data::opacity(id) < FULL_OPACITY
        });
        let mut levels = 0;
        if complete {
            for level in 1..=MAX_LEVELS {
                let y = pos.1 - level;
                if y < 0 {
                    break;
                }
                let full = (pos.0 - level..=pos.0 + level).all(|x| {
                    (pos.2 - level..=pos.2 + level)
                        .all(|z| PYRAMID_BLOCKS.contains(&(w.block(x, y, z) >> 4)))
                });
                if !full {
                    break;
                }
                levels = level;
            }
            if levels == 0 {
                complete = false;
            }
        }
        let Some(te) = w.tile_entity_mut(pos) else {
            return;
        };
        te.insert("Levels", krol_nbt::Tag::Int(levels));
        let primary = te.get_i32("Primary");
        let secondary = te.get_i32("Secondary");
        // addEffectsToPlayers.
        if !complete || levels <= 0 || primary <= 0 {
            return;
        }
        let range = f64::from(levels * RANGE_PER_LEVEL + RANGE_PER_LEVEL);
        let min = [
            f64::from(pos.0) - range,
            f64::from(pos.1) - range,
            f64::from(pos.2) - range,
        ];
        // The box reaches the world height above its top.
        let max = [
            f64::from(pos.0 + 1) + range,
            f64::from(pos.1 + 1) + range + 256.0,
            f64::from(pos.2 + 1) + range,
        ];
        let area = super::physics::Aabb::new(min, max);
        let inside: Vec<usize> = (0..self.players.len())
            .filter(|&i| super::physics::Aabb::at(self.players[i].pos, 0.6, 1.8).intersects(&area))
            .collect();
        let doubled = levels >= MAX_LEVELS && primary == secondary;
        for &i in &inside {
            self.add_effect(
                i,
                Effect {
                    id: primary as u8,
                    amplifier: u8::from(doubled),
                    duration: EFFECT_TICKS,
                    ambient: true,
                    particles: true,
                },
            );
        }
        if levels >= MAX_LEVELS && primary != secondary && secondary > 0 {
            for &i in &inside {
                self.add_effect(
                    i,
                    Effect {
                        id: secondary as u8,
                        amplifier: 0,
                        duration: EFFECT_TICKS,
                        ambient: true,
                        particles: true,
                    },
                );
            }
        }
    }

    /// `BlockBeacon.onNeighborBlockChange`: the beacon updates, and again when its block
    /// event comes round.
    pub(super) fn beacon_neighbor_changed(&mut self, pos: (i32, i32, i32)) {
        if self.level.overworld.tile_entity(pos).is_none() {
            return;
        }
        self.update_beacon(pos);
        self.queue_block_event(pos, BEACON, EVENT_UPDATE, 0);
    }

    /// `TileEntityBeacon.receiveClientEvent`: event 1 updates the beacon (and is sent).
    pub(super) fn beacon_event(&mut self, pos: (i32, i32, i32), action: u8) -> bool {
        if action != EVENT_UPDATE || self.level.overworld.tile_entity(pos).is_none() {
            return false;
        }
        self.update_beacon(pos);
        true
    }

    /// `BlockBeacon.onBlockActivated`: the beacon window.
    pub(super) fn activate_beacon(&mut self, i: usize, pos: (i32, i32, i32)) {
        if self.level.overworld.tile_entity(pos).is_none() {
            return;
        }
        if self.players[i].window.open.is_some() {
            self.close_screen(i);
        }
        let title = crate::chat::Component::translate("container.beacon", vec![]).to_json_string();
        let id = self.players[i].window.next_window_id();
        self.players[i].send(ClientboundPlay::OpenWindow {
            window: id as u8,
            kind: "minecraft:beacon".into(),
            title,
            slots: SLOTS as u8,
        });
        let layout = Layout::Beacon;
        self.players[i].window.open = Some(OpenContainer {
            id,
            layout,
            pos,
            slots: vec![None; SLOTS],
            parts: Vec::new(),
            ender: false,
            sent: vec![None; layout.len()],
            fields: [0; 4],
            taken: 0,
            cart: None,
        });
        self.open_container_gui(i);
        // ContainerBeacon.onCraftGuiOpened: the levels and powers, once.
        let te = self.level.overworld.tile_entity(pos);
        let values = te.map_or([0; 3], |t| {
            [
                t.get_i32("Levels"),
                t.get_i32("Primary"),
                t.get_i32("Secondary"),
            ]
        });
        let p = &self.players[i];
        for (property, value) in values.into_iter().enumerate() {
            p.send(ClientboundPlay::WindowProperty {
                window: id as u8,
                property: property as i16,
                value: value as i16,
            });
        }
        self.add_stat(i, "stat.beaconInteraction", 1);
    }

    /// `MC|Beacon` with a beacon window open: with a payment in the slot, it is used up
    /// and the chosen powers are set (vanilla does not check them against the levels).
    pub(super) fn choose_beacon_powers(&mut self, i: usize, data: &[u8]) {
        let Some(o) = self.players[i]
            .window
            .open
            .as_mut()
            .filter(|o| o.layout == Layout::Beacon)
        else {
            return;
        };
        if data.len() < 8 {
            // readInt past the end: "Couldn't set beacon" with the exception.
            tracing::error!("Couldn't set beacon");
            return;
        }
        let primary = i32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let secondary = i32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        if o.slots[0].is_none() {
            return;
        }
        // decrStackSize(1) on a slot that holds one item.
        if let Some(s) = &mut o.slots[0] {
            s.count -= 1;
            if s.count <= 0 {
                o.slots[0] = None;
            }
        }
        let pos = o.pos;
        if let Some(te) = self.level.overworld.tile_entity_mut(pos) {
            te.insert("Primary", krol_nbt::Tag::Int(valid_power(primary)));
            te.insert("Secondary", krol_nbt::Tag::Int(valid_power(secondary)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powers_and_payments() {
        assert_eq!(valid_power(1), 1);
        assert_eq!(valid_power(8), 8);
        assert_eq!(valid_power(7), 0);
        assert_eq!(valid_power(-1), 0);
        assert!(is_payment(&ItemStack {
            id: 264,
            count: 1,
            damage: 0,
            nbt: None
        }));
    }
}
