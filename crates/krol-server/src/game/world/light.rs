//! Light (vanilla: `World.checkLight` / `checkLightFor` / `getRawLight` /
//! `getLightFor` / `setLightFor` / `markBlocksDirtyVertical`, and `Chunk.relightBlock`
//! / `generateSkylightMap` / `recheckGaps` / `updateSkylightNeighborHeight`).

use krol_world::chunk::Section;

use super::World;
use crate::game::light_data::{light_value, opacity};

/// Sky or block light (`EnumSkyBlock`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightType {
    /// Light from the sky.
    Sky,
    /// Light from blocks such as torches.
    Block,
}

impl LightType {
    /// `defaultLightValue`.
    fn default_value(self) -> i32 {
        match self {
            Self::Sky => 15,
            Self::Block => 0,
        }
    }
}

/// Full light.
const MAX_LIGHT: i32 = 15;
/// `lightUpdateBlockList` size.
const QUEUE_LEN: usize = 32768;
/// Light updates spread at most this far (Manhattan distance) from the change; also
/// the loaded radius they need.
const REACH: i32 = 17;
/// Gap rechecks and neighbor heights need everything this far around loaded.
const GAP_RADIUS: i32 = 16;
/// Queue entries hold coordinates relative to the start, offset by this.
const QUEUE_OFFSET: i32 = 32;
/// The initial queue entry (`133152`: the start position, offset 32 on each axis).
const QUEUE_START: i32 = QUEUE_OFFSET | QUEUE_OFFSET << 6 | QUEUE_OFFSET << 12;

/// `EnumFacing.values()` offsets.
const FACES: [(i32, i32, i32); 6] = [
    (0, -1, 0),
    (0, 1, 0),
    (0, 0, -1),
    (0, 0, 1),
    (-1, 0, 0),
    (1, 0, 0),
];
/// `EnumFacing.Plane.HORIZONTAL` offsets (north, east, south, west).
const HORIZONTAL: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

fn section_index(x: i32, y: i32, z: i32) -> usize {
    Section::index((x & 15) as usize, (y & 15) as usize, (z & 15) as usize)
}

impl World {
    fn block_if_loaded(&self, x: i32, y: i32, z: i32) -> u16 {
        if !(0..256).contains(&y) {
            return 0;
        }
        self.chunks.get(&(x >> 4, z >> 4)).map_or(0, |c| {
            c.chunk
                .block((x & 15) as usize, y as usize, (z & 15) as usize)
        })
    }

    /// `World.isAreaLoaded(pos, radius, true)`.
    fn area_loaded(&self, pos: (i32, i32, i32), radius: i32) -> bool {
        if pos.1 + radius < 0 || pos.1 - radius >= 256 {
            return false;
        }
        (((pos.0 - radius) >> 4)..=((pos.0 + radius) >> 4)).all(|cx| {
            (((pos.2 - radius) >> 4)..=((pos.2 + radius) >> 4)).all(|cz| self.loaded(cx, cz))
        })
    }

    /// `Chunk.canSeeSky`: at or above the height map.
    fn can_see_sky(&self, pos: (i32, i32, i32)) -> bool {
        self.chunks.get(&(pos.0 >> 4, pos.2 >> 4)).is_some_and(|c| {
            pos.1 >= c.chunk.height_map[((pos.2 & 15) << 4 | (pos.0 & 15)) as usize]
        })
    }

    /// `World.getLightFor`.
    pub fn light(&self, t: LightType, pos: (i32, i32, i32)) -> i32 {
        let pos = (pos.0, pos.1.max(0), pos.2);
        if pos.1 >= 256 {
            return t.default_value();
        }
        let Some(c) = self.chunks.get(&(pos.0 >> 4, pos.2 >> 4)) else {
            return t.default_value();
        };
        match &c.chunk.sections[(pos.1 >> 4) as usize] {
            None => {
                if self.can_see_sky(pos) {
                    t.default_value()
                } else {
                    0
                }
            }
            Some(s) => {
                let i = section_index(pos.0, pos.1, pos.2);
                match t {
                    LightType::Sky => s.sky_light.as_ref().map_or(0, |l| i32::from(l.get(i))),
                    LightType::Block => i32::from(s.block_light.get(i)),
                }
            }
        }
    }

    /// `World.setLightFor` (a missing section is created and gets its sky light map).
    fn set_light(&mut self, t: LightType, pos: (i32, i32, i32), value: i32) {
        if !(0..256).contains(&pos.1) || !self.loaded(pos.0 >> 4, pos.2 >> 4) {
            return;
        }
        let (cx, cz) = (pos.0 >> 4, pos.2 >> 4);
        let has_sky = self.has_sky;
        let index = (pos.1 >> 4) as usize;
        if self.chunk_mut(cx, cz).sections[index].is_none() {
            self.chunk_mut(cx, cz).sections[index] = Some(Section::empty(index as i8, has_sky));
            self.generate_skylight_map(cx, cz);
        }
        let i = section_index(pos.0, pos.1, pos.2);
        if let Some(s) = self.chunk_mut(cx, cz).sections[index].as_mut() {
            match t {
                LightType::Sky => {
                    if let Some(l) = s.sky_light.as_mut() {
                        l.set(i, value as u8);
                    }
                }
                LightType::Block => s.block_light.set(i, value as u8),
            }
        }
    }

    /// `World.getRawLight`: what the light at `pos` should be from its own emission
    /// and its neighbors.
    fn raw_light(&self, pos: (i32, i32, i32), t: LightType) -> i32 {
        if t == LightType::Sky && self.can_see_sky(pos) {
            return MAX_LIGHT;
        }
        let id = self.block_if_loaded(pos.0, pos.1, pos.2) >> 4;
        let mut light = if t == LightType::Sky {
            0
        } else {
            light_value(id)
        };
        let mut op = opacity(id);
        if op >= MAX_LIGHT && light_value(id) > 0 {
            op = 1;
        }
        if op < 1 {
            op = 1;
        }
        if op >= MAX_LIGHT {
            return 0;
        }
        if light >= MAX_LIGHT - 1 {
            return light;
        }
        for (dx, dy, dz) in FACES {
            let l = self.light(t, (pos.0 + dx, pos.1 + dy, pos.2 + dz)) - op;
            if l > light {
                light = l;
            }
            if light >= MAX_LIGHT - 1 {
                return light;
            }
        }
        light
    }

    /// `World.checkLight`: sky light (with a sky), then block light.
    pub fn check_light(&mut self, pos: (i32, i32, i32)) {
        if self.has_sky {
            self.check_light_for(LightType::Sky, pos);
        }
        self.check_light_for(LightType::Block, pos);
    }

    /// `World.checkLightFor`: darkens what depended on the old light, then spreads the
    /// new light, within 17 blocks.
    pub fn check_light_for(&mut self, t: LightType, pos: (i32, i32, i32)) -> bool {
        if !self.area_loaded(pos, REACH) {
            return false;
        }
        let mut queue = vec![0i32; QUEUE_LEN];
        let (mut head, mut tail) = (0usize, 0usize);
        let current = self.light(t, pos);
        let raw = self.raw_light(pos, t);
        let decode = |e: i32| {
            (
                (e & 63) - QUEUE_OFFSET + pos.0,
                (e >> 6 & 63) - QUEUE_OFFSET + pos.1,
                (e >> 12 & 63) - QUEUE_OFFSET + pos.2,
            )
        };
        let encode = |p: (i32, i32, i32)| {
            (p.0 - pos.0 + QUEUE_OFFSET)
                | (p.1 - pos.1 + QUEUE_OFFSET) << 6
                | (p.2 - pos.2 + QUEUE_OFFSET) << 12
        };
        if raw > current {
            queue[tail] = QUEUE_START;
            tail += 1;
        } else if raw < current {
            queue[tail] = QUEUE_START | current << 18;
            tail += 1;
            while head < tail {
                let e = queue[head];
                head += 1;
                let p = decode(e);
                let level = e >> 18 & 15;
                if self.light(t, p) == level {
                    self.set_light(t, p, 0);
                    if level > 0 {
                        let d = (p.0 - pos.0).abs() + (p.1 - pos.1).abs() + (p.2 - pos.2).abs();
                        if d < REACH {
                            for (dx, dy, dz) in FACES {
                                let n = (p.0 + dx, p.1 + dy, p.2 + dz);
                                let op = opacity(self.block_if_loaded(n.0, n.1, n.2) >> 4).max(1);
                                if self.light(t, n) == level - op && tail < QUEUE_LEN {
                                    queue[tail] = encode(n) | (level - op) << 18;
                                    tail += 1;
                                }
                            }
                        }
                    }
                }
            }
            head = 0;
        }
        while head < tail {
            let e = queue[head];
            head += 1;
            let p = decode(e);
            let current = self.light(t, p);
            let raw = self.raw_light(p, t);
            if raw == current {
                continue;
            }
            self.set_light(t, p, raw);
            if raw > current {
                let d = (p.0 - pos.0).abs() + (p.1 - pos.1).abs() + (p.2 - pos.2).abs();
                if d < REACH && tail < QUEUE_LEN - 6 {
                    for (dx, dy, dz) in [
                        (-1, 0, 0),
                        (1, 0, 0),
                        (0, -1, 0),
                        (0, 1, 0),
                        (0, 0, -1),
                        (0, 0, 1),
                    ] {
                        let n = (p.0 + dx, p.1 + dy, p.2 + dz);
                        if self.light(t, n) < raw {
                            queue[tail] = encode(n);
                            tail += 1;
                        }
                    }
                }
            }
        }
        true
    }

    /// `Chunk.getBlockLightOpacity` at chunk-local coordinates.
    fn local_opacity(&self, cx: i32, cz: i32, x: i32, y: i32, z: i32) -> i32 {
        self.chunks.get(&(cx, cz)).map_or(0, |c| {
            if (0..256).contains(&y) {
                opacity(c.chunk.block(x as usize, y as usize, z as usize) >> 4)
            } else {
                0
            }
        })
    }

    fn set_local_sky(&mut self, cx: i32, cz: i32, x: i32, y: i32, z: i32, value: i32) {
        if let Some(c) = self.chunks.get_mut(&(cx, cz))
            && let Some(s) = c.chunk.sections[(y >> 4) as usize].as_mut()
            && let Some(l) = s.sky_light.as_mut()
        {
            l.set(section_index(x, y, z), value as u8);
        }
    }

    /// `Chunk.getTopFilledSegment`.
    fn top_filled_segment(&self, cx: i32, cz: i32) -> i32 {
        self.chunks.get(&(cx, cz)).map_or(0, |c| {
            c.chunk
                .sections
                .iter()
                .rposition(Option::is_some)
                .map_or(0, |k| (k as i32) << 4)
        })
    }

    /// `Chunk.generateSkylightMap`: recomputes the height map and the column sky light.
    pub(super) fn generate_skylight_map(&mut self, cx: i32, cz: i32) {
        let top = self.top_filled_segment(cx, cz);
        let mut minimum = i32::MAX;
        for x in 0..16 {
            for z in 0..16 {
                let mut y = top + 16;
                while y > 0 {
                    if self.local_opacity(cx, cz, x, y - 1, z) != 0 {
                        if let Some(c) = self.chunks.get_mut(&(cx, cz)) {
                            c.chunk.height_map[(z << 4 | x) as usize] = y;
                        }
                        minimum = minimum.min(y);
                        break;
                    }
                    y -= 1;
                }
                if self.has_sky {
                    let mut light = MAX_LIGHT;
                    let mut y = top + 16 - 1;
                    loop {
                        let mut op = self.local_opacity(cx, cz, x, y, z);
                        if op == 0 && light != MAX_LIGHT {
                            op = 1;
                        }
                        light -= op;
                        if light > 0 {
                            self.set_local_sky(cx, cz, x, y, z, light);
                        }
                        y -= 1;
                        if y <= 0 || light <= 0 {
                            break;
                        }
                    }
                }
            }
        }
        if let Some(c) = self.chunks.get_mut(&(cx, cz)) {
            c.height_minimum = minimum;
            c.modified = true;
        }
    }

    /// `World.getHeight`: the height map value, 0 in unloaded chunks.
    fn height_at(&self, x: i32, z: i32) -> i32 {
        self.chunks.get(&(x >> 4, z >> 4)).map_or(0, |c| {
            c.chunk.height_map[((z & 15) << 4 | (x & 15)) as usize]
        })
    }

    /// `Chunk.updateSkylightNeighborHeight`: rechecks sky light in a column range.
    fn update_neighbor_height(&mut self, owner: (i32, i32), x: i32, z: i32, from: i32, to: i32) {
        if to > from && self.area_loaded((x, 0, z), GAP_RADIUS) {
            for y in from..to {
                self.check_light_for(LightType::Sky, (x, y, z));
            }
            if let Some(c) = self.chunks.get_mut(&owner) {
                c.modified = true;
            }
        }
    }

    /// `Chunk.checkSkylightNeighborHeight`.
    fn check_neighbor_height(&mut self, owner: (i32, i32), x: i32, z: i32, height: i32) {
        let h = self.height_at(x, z);
        if h > height {
            self.update_neighbor_height(owner, x, z, height, h + 1);
        } else if h < height {
            self.update_neighbor_height(owner, x, z, h, height + 1);
        }
    }

    /// `Chunk.relightBlock`: the column's height and sky light after a change at
    /// local `(x, y, z)`.
    pub(super) fn relight_block(&mut self, cx: i32, cz: i32, x: i32, y: i32, z: i32) {
        let index = (z << 4 | x) as usize;
        let Some(old) = self
            .chunks
            .get(&(cx, cz))
            .map(|c| c.chunk.height_map[index] & 0xff)
        else {
            return;
        };
        let mut height = old.max(y);
        while height > 0 && self.local_opacity(cx, cz, x, height - 1, z) == 0 {
            height -= 1;
        }
        if height == old {
            return;
        }
        let (wx, wz) = (cx * 16 + x, cz * 16 + z);
        // World.markBlocksDirtyVertical: sky light checks over the changed range, still
        // with the old height.
        if self.has_sky {
            for yy in height.min(old)..=height.max(old) {
                self.check_light_for(LightType::Sky, (wx, yy, wz));
            }
        }
        if let Some(c) = self.chunks.get_mut(&(cx, cz)) {
            c.chunk.height_map[index] = height;
        }
        if self.has_sky {
            if height < old {
                for yy in height..old {
                    self.set_local_sky(cx, cz, x, yy, z, MAX_LIGHT);
                }
            } else {
                for yy in old..height {
                    self.set_local_sky(cx, cz, x, yy, z, 0);
                }
            }
            let mut light = MAX_LIGHT;
            let mut yy = height;
            while yy > 0 && light > 0 {
                yy -= 1;
                let op = self.local_opacity(cx, cz, x, yy, z).max(1);
                light = (light - op).max(0);
                self.set_local_sky(cx, cz, x, yy, z, light);
            }
        }
        let new = height;
        let (low, high) = if new < old { (new, old) } else { (old, new) };
        if let Some(c) = self.chunks.get_mut(&(cx, cz))
            && new < c.height_minimum
        {
            c.height_minimum = new;
        }
        if self.has_sky {
            for (dx, dz) in HORIZONTAL {
                self.update_neighbor_height((cx, cz), wx + dx, wz + dz, low, high);
            }
            self.update_neighbor_height((cx, cz), wx, wz, low, high);
        }
        if let Some(c) = self.chunks.get_mut(&(cx, cz)) {
            c.modified = true;
        }
    }

    /// `Chunk.recheckGaps`, run when the chunk ticks: columns whose opacity changed
    /// recheck the sky light against their neighbors' heights.
    pub(super) fn recheck_gaps(&mut self, cx: i32, cz: i32) {
        if !self.area_loaded((cx * 16 + 8, 0, cz * 16 + 8), GAP_RADIUS) {
            return;
        }
        for x in 0..16 {
            for z in 0..16 {
                let index = (x + z * 16) as usize;
                let flagged = self
                    .chunks
                    .get_mut(&(cx, cz))
                    .map(|c| std::mem::replace(&mut c.gap_columns[index], false))
                    .unwrap_or(false);
                if !flagged {
                    continue;
                }
                let height = self.height_at(cx * 16 + x, cz * 16 + z);
                let (wx, wz) = (cx * 16 + x, cz * 16 + z);
                let mut horizon = i32::MAX;
                for (dx, dz) in HORIZONTAL {
                    let (nx, nz) = (wx + dx, wz + dz);
                    let lowest = self
                        .chunks
                        .get(&(nx >> 4, nz >> 4))
                        .map_or(0, |c| c.height_minimum);
                    horizon = horizon.min(lowest);
                }
                self.check_neighbor_height((cx, cz), wx, wz, horizon);
                for (dx, dz) in HORIZONTAL {
                    self.check_neighbor_height((cx, cz), wx + dx, wz + dz, height);
                }
            }
        }
        if let Some(c) = self.chunks.get_mut(&(cx, cz)) {
            c.gap_pending = false;
        }
    }

    /// The light part of `Chunk.setBlockState` after the block changed at `pos`:
    /// a new section above the height map regenerates the sky map, otherwise the column
    /// is relit and opacity changes flag a gap recheck.
    pub(super) fn chunk_light_after_set(
        &mut self,
        pos: (i32, i32, i32),
        old: u16,
        new: u16,
        old_height: i32,
        new_section: bool,
    ) {
        let (cx, cz) = (pos.0 >> 4, pos.2 >> 4);
        let (x, y, z) = (pos.0 & 15, pos.1, pos.2 & 15);
        if new_section {
            if y >= old_height {
                self.generate_skylight_map(cx, cz);
            }
            return;
        }
        let (op_new, op_old) = (opacity(new >> 4), opacity(old >> 4));
        if op_new > 0 {
            if y >= old_height {
                self.relight_block(cx, cz, x, y + 1, z);
            }
        } else if y == old_height - 1 {
            self.relight_block(cx, cz, x, y, z);
        }
        if op_new != op_old
            && (op_new < op_old
                || self.light(LightType::Sky, pos) > 0
                || self.light(LightType::Block, pos) > 0)
            && let Some(c) = self.chunks.get_mut(&(cx, cz))
        {
            // propagateSkylightOcclusion.
            c.gap_columns[(x + z * 16) as usize] = true;
            c.gap_pending = true;
        }
    }

    /// `World.setBlockState`'s light check: when opacity or light emission changed.
    pub fn check_light_if_changed(&mut self, pos: (i32, i32, i32), old: u16, new: u16) {
        let (o, n) = (old >> 4, new >> 4);
        if opacity(o) != opacity(n) || light_value(o) != light_value(n) {
            self.check_light(pos);
        }
    }
}
