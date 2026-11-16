//! Buckets (vanilla `ItemBucket.onItemRightClick`): picking up and placing water and
//! lava along the player's view.

use krol_protocol::packets::play::ItemStack;

use super::Game;
use super::block_data::{Material, material};
use super::ids::{block, item};
use super::raytrace::{self, Vec3};

/// How far `Item.getMovingObjectPositionFromPlayer` looks.
const BUCKET_REACH: f64 = 5.0;
/// `EntityPlayer.getEyeHeight` standing.
const EYE_HEIGHT: f32 = 1.62;
/// Sneaking lowers the eyes by this much.
const SNEAK_EYE_DROP: f32 = 0.08;

/// What using an item did to the held stack (`tryUseItem` compares the returned stack
/// with the held one).
pub enum UseResult {
    /// The same stack, unchanged.
    Unchanged,
    /// The same stack with a new size.
    Resized,
    /// A different stack (or none).
    Replaced(Option<ItemStack>),
    /// The same stack, which has a use duration (food): it counts as changed.
    Kept,
}

impl Game {
    /// `Item.getMovingObjectPositionFromPlayer`: the block the player looks at within
    /// [`BUCKET_REACH`].
    fn view_ray_trace(&mut self, i: usize, liquids: bool) -> Option<raytrace::Hit> {
        use krol_java::math::{cos, sin};
        let p = &self.players[i];
        let (yaw, pitch) = p.rotation;
        let eye = if p.sneaking {
            EYE_HEIGHT - SNEAK_EYE_DROP
        } else {
            EYE_HEIGHT
        };
        let start: Vec3 = (p.pos.0, p.pos.1 + f64::from(eye), p.pos.2);
        let to_rad = std::f64::consts::PI as f32 / 180.0;
        let pi = std::f64::consts::PI as f32;
        let f2 = cos(-yaw * to_rad - pi);
        let f3 = sin(-yaw * to_rad - pi);
        let f4 = -cos(-pitch * to_rad);
        let f5 = sin(-pitch * to_rad);
        let end = (
            start.0 + f64::from(f3 * f4) * BUCKET_REACH,
            start.1 + f64::from(f5) * BUCKET_REACH,
            start.2 + f64::from(f2 * f4) * BUCKET_REACH,
        );
        raytrace::ray_trace_blocks(&mut self.level.overworld, start, end, liquids, !liquids)
    }

    /// `EntityPlayer.canPlayerEdit` (without `CanPlaceOn` tags): adventure players
    /// cannot use buckets.
    fn can_edit(&self, i: usize) -> bool {
        !matches!(self.players[i].gamemode, 2 | 3)
    }

    /// `ItemBucket.onItemRightClick` for an empty, water or lava bucket in the held
    /// slot.
    pub(super) fn use_bucket(&mut self, i: usize, held: &ItemStack) -> UseResult {
        let full = match held.id {
            item::WATER_BUCKET => Some(block::FLOWING_WATER),
            item::LAVA_BUCKET => Some(block::FLOWING_LAVA),
            _ => None,
        };
        let Some(hit) = self.view_ray_trace(i, full.is_none()) else {
            return UseResult::Unchanged;
        };
        // World.isBlockModifiable: spawn protection (the world border is not modeled).
        if self.is_block_protected(i, hit.pos) {
            return UseResult::Unchanged;
        }
        if !self.can_edit(i) {
            return UseResult::Unchanged;
        }
        let creative = self.players[i].gamemode == 1;
        match full {
            None => {
                let state = self.level.overworld.block(hit.pos.0, hit.pos.1, hit.pos.2);
                let filled = match material(state >> 4) {
                    Material::Water if state & 15 == 0 => item::WATER_BUCKET,
                    Material::Lava if state & 15 == 0 => item::LAVA_BUCKET,
                    _ => return UseResult::Unchanged,
                };
                self.set_block_notify(hit.pos, 0);
                self.count_item_use(i);
                self.fill_bucket(i, held, filled, creative)
            }
            Some(liquid) => {
                let at = super::placement::offset(hit.pos, hit.face);
                if self.place_liquid(at, liquid) && !creative {
                    self.count_item_use(i);
                    return UseResult::Replaced(Some(bucket(item::BUCKET)));
                }
                UseResult::Unchanged
            }
        }
    }

    /// `ItemBucket.fillBucket`.
    fn fill_bucket(
        &mut self,
        i: usize,
        held: &ItemStack,
        filled: i16,
        creative: bool,
    ) -> UseResult {
        if creative {
            return UseResult::Unchanged;
        }
        if held.count <= 1 {
            return UseResult::Replaced(Some(bucket(filled)));
        }
        let p = &mut self.players[i];
        let slot = 36 + p.selected;
        if let Some(s) = p.inventory[slot].as_mut() {
            s.count -= 1;
        }
        let mut new = bucket(filled);
        if !super::item_entity::add_to_inventory(
            &mut p.inventory,
            &mut new,
            p.gamemode == super::player::Player::CREATIVE,
        ) {
            self.toss(i, bucket(filled), false);
        }
        UseResult::Resized
    }

    /// `ItemGlassBottle.onItemRightClick`: aimed at water (any level), the bottle fills
    /// (in creative too).
    pub(super) fn fill_bottle(&mut self, i: usize, held: &ItemStack) -> UseResult {
        let Some(hit) = self.view_ray_trace(i, true) else {
            return UseResult::Unchanged;
        };
        if self.is_block_protected(i, hit.pos) || !self.can_edit(i) {
            return UseResult::Unchanged;
        }
        let state = self.level.overworld.block(hit.pos.0, hit.pos.1, hit.pos.2);
        if material(state >> 4) != Material::Water {
            return UseResult::Unchanged;
        }
        self.count_item_use(i);
        let water = || ItemStack {
            id: item::POTION,
            count: 1,
            damage: 0,
            nbt: None,
        };
        if held.count <= 1 {
            return UseResult::Replaced(Some(water()));
        }
        let p = &mut self.players[i];
        let slot = 36 + p.selected;
        if let Some(s) = p.inventory[slot].as_mut() {
            s.count -= 1;
        }
        let mut new = water();
        if !super::item_entity::add_to_inventory(
            &mut p.inventory,
            &mut new,
            p.gamemode == super::player::Player::CREATIVE,
        ) {
            self.toss(i, water(), false);
        }
        UseResult::Resized
    }

    /// `ItemBucket.tryPlaceContainedLiquid` in a world where water does not vaporize.
    fn place_liquid(&mut self, at: (i32, i32, i32), liquid: u16) -> bool {
        if !(0..256).contains(&at.1) {
            return false;
        }
        let state = self.level.overworld.block(at.0, at.1, at.2);
        let m = material(state >> 4);
        if state >> 4 != block::AIR && m.is_solid() {
            return false;
        }
        if !m.is_solid() && !m.is_liquid() && state >> 4 != block::AIR {
            self.destroy_block(at, state);
        }
        self.set_block_notify(at, liquid << 4);
        true
    }
}

fn bucket(id: i16) -> ItemStack {
    ItemStack {
        id,
        count: 1,
        damage: 0,
        nbt: None,
    }
}
