//! Per-block light properties of vanilla 1.8.8 (`Block.getLightOpacity`,
//! `getLightValue`, `getUseNeighborBrightness`), extracted by
//! `tools/datagen/BlockLightDump.java` into `data/block_light.txt`.

use std::sync::OnceLock;

/// Block ids in the table (0-255).
const IDS: usize = 256;

/// `(opacity, light value, uses neighbor brightness)` per block id.
fn table() -> &'static [(u8, u8, bool); IDS] {
    static TABLE: OnceLock<[(u8, u8, bool); IDS]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [(0u8, 0u8, false); IDS];
        for line in include_str!("../../data/block_light.txt").lines() {
            let f: Vec<u32> = line
                .split_whitespace()
                .filter_map(|v| v.parse().ok())
                .collect();
            if let [id, opacity, light, neighbor] = f[..]
                && (id as usize) < IDS
            {
                t[id as usize] = (opacity as u8, light as u8, neighbor != 0);
            }
        }
        t
    })
}

/// `Block.getLightOpacity` (0-255; 255 for opaque cubes).
pub fn opacity(id: u16) -> i32 {
    table().get(usize::from(id)).map_or(0, |e| i32::from(e.0))
}

/// `Block.getLightValue` (0-15).
pub fn light_value(id: u16) -> i32 {
    table().get(usize::from(id)).map_or(0, |e| i32::from(e.1))
}

/// `Block.getUseNeighborBrightness` (slabs, stairs, farmland, ...).
#[allow(dead_code)] // getLightFromNeighbors for random ticks: TODO(M5)
pub fn uses_neighbor_brightness(id: u16) -> bool {
    table().get(usize::from(id)).is_some_and(|e| e.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_values() {
        assert_eq!(opacity(1), 255);
        assert_eq!(opacity(8), 3);
        assert_eq!(opacity(18), 1);
        assert_eq!(light_value(50), 14);
        assert_eq!(light_value(89), 15);
        assert!(uses_neighbor_brightness(44));
        assert!(!uses_neighbor_brightness(1));
    }
}
