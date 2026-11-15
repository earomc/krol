//! Per-block light and shape properties of vanilla 1.8.8 (`Block.getLightOpacity`,
//! `getLightValue`, `getUseNeighborBrightness`, `getTickRandomly`, `isVisuallyOpaque`,
//! `isFullBlock`, `isFullCube`, `isNormalCube`, `getExplosionResistance`), extracted by
//! `tools/datagen/BlockLightDump.java` into `data/block_light.txt`.

use std::sync::OnceLock;

/// Block ids in the table (0-255).
const IDS: usize = 256;

/// One block's properties.
#[derive(Debug, Clone, Copy, Default)]
struct Entry {
    opacity: u8,
    light: u8,
    neighbor_brightness: bool,
    ticks_randomly: bool,
    visually_opaque: bool,
    full_block: bool,
    full_cube: bool,
    normal_cube: bool,
    resistance: f32,
}

fn table() -> &'static [Entry; IDS] {
    static TABLE: OnceLock<[Entry; IDS]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [Entry::default(); IDS];
        for line in include_str!("../../data/block_light.txt").lines() {
            let cols: Vec<&str> = line.split_whitespace().collect();
            let Some((resistance, ints)) = cols.split_last() else {
                continue;
            };
            let f: Vec<u32> = ints.iter().filter_map(|v| v.parse().ok()).collect();
            let resistance = u32::from_str_radix(resistance, 16).map_or(0.0, f32::from_bits);
            if let [
                id,
                opacity,
                light,
                neighbor,
                random,
                visual,
                block,
                cube,
                normal,
            ] = f[..]
                && (id as usize) < IDS
            {
                t[id as usize] = Entry {
                    opacity: opacity as u8,
                    light: light as u8,
                    neighbor_brightness: neighbor != 0,
                    ticks_randomly: random != 0,
                    visually_opaque: visual != 0,
                    full_block: block != 0,
                    full_cube: cube != 0,
                    normal_cube: normal != 0,
                    resistance,
                };
            }
        }
        t
    })
}

/// `Block.getLightOpacity` (0-255; 255 for opaque cubes).
pub fn opacity(id: u16) -> i32 {
    table()
        .get(usize::from(id))
        .map_or(0, |e| i32::from(e.opacity))
}

/// `Block.getLightValue` (0-15).
pub fn light_value(id: u16) -> i32 {
    table()
        .get(usize::from(id))
        .map_or(0, |e| i32::from(e.light))
}

/// `Block.getTickRandomly`.
pub fn ticks_randomly(id: u16) -> bool {
    table()
        .get(usize::from(id))
        .is_some_and(|e| e.ticks_randomly)
}

/// `Block.getUseNeighborBrightness` (slabs, stairs, farmland, ...).
pub fn uses_neighbor_brightness(id: u16) -> bool {
    table()
        .get(usize::from(id))
        .is_some_and(|e| e.neighbor_brightness)
}

fn flag(id: u16, f: impl Fn(&Entry) -> bool) -> bool {
    table().get(usize::from(id)).is_some_and(f)
}

/// `Block.isVisuallyOpaque`: what entities suffocate in.
pub fn visually_opaque(id: u16) -> bool {
    flag(id, |e| e.visually_opaque)
}

/// `Block.isFullBlock` (opaque cubes when registered; leaves count).
pub fn full_block(id: u16) -> bool {
    flag(id, |e| e.full_block)
}

/// `Block.isFullCube`.
pub fn full_cube(id: u16) -> bool {
    flag(id, |e| e.full_cube)
}

/// `Block.getExplosionResistance(null)`: the block's resistance to explosion rays.
pub fn explosion_resistance(id: u16) -> f32 {
    table().get(usize::from(id)).map_or(0.0, |e| e.resistance)
}

/// `Block.isNormalCube`.
pub fn normal_cube(id: u16) -> bool {
    flag(id, |e| e.normal_cube)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_values() {
        assert_eq!(opacity(1), 255);
        assert_eq!(explosion_resistance(1), 6.0);
        assert_eq!(explosion_resistance(49), 1200.0);
        assert_eq!(explosion_resistance(8), 100.0);
        assert_eq!(opacity(8), 3);
        assert_eq!(opacity(18), 1);
        assert_eq!(light_value(50), 14);
        assert_eq!(light_value(89), 15);
        assert!(uses_neighbor_brightness(44));
        assert!(!uses_neighbor_brightness(1));
    }
}
