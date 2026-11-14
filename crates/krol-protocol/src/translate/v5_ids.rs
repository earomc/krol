//! Block and item ids for 1.7 clients.
//!
//! 1.8 added blocks and items that 1.7 clients do not know; an unknown id makes them
//! render nothing or fail, so each is replaced with the closest block or item 1.7 has.
//! These substitutions are krol's choice (there is no vanilla behavior to match: vanilla
//! 1.8.8 does not accept 1.7 clients). Everything else passes through unchanged.

/// Maps a 1.8 block (id, metadata) to one a 1.7 client knows.
pub fn block(id: u16, meta: u8) -> (u16, u8) {
    match id {
        // Granite, diorite, andesite and their polished forms.
        1 => (1, 0),
        // Wet sponge.
        19 => (19, 0),
        // Slime block: lime wool.
        165 => (35, 5),
        // Barrier: glass, so it still looks solid.
        166 => (20, 0),
        // Iron trapdoor: wooden trapdoor (same metadata layout).
        167 => (96, meta),
        // Prismarine (all kinds): stone bricks.
        168 => (98, 0),
        // Sea lantern: glowstone.
        169 => (89, 0),
        // Standing and wall banners: nothing (they have no collision).
        176 | 177 => (0, 0),
        // Inverted daylight sensor: daylight sensor.
        178 => (151, meta),
        // Red sandstone, its stairs and slabs: the sandstone forms.
        179 => (24, meta),
        180 => (128, meta),
        181 => (43, if meta & 8 != 0 { 9 } else { 1 }),
        182 => (44, 1 | (meta & 8)),
        // Spruce, birch, jungle, dark oak and acacia fence gates and fences: oak.
        183..=187 => (107, meta),
        188..=192 => (85, 0),
        // Spruce, birch, jungle, acacia and dark oak doors: oak door.
        193..=197 => (64, meta),
        _ => (id, meta),
    }
}

/// Maps a 1.8 item (id, damage) to one a 1.7 client knows.
pub fn item(id: i16, damage: i16) -> (i16, i16) {
    match id {
        0..=255 => {
            // Block items carry the block metadata in their damage (doors and banners
            // are separate items).
            let (b, m) = block(id as u16, (damage & 15) as u8);
            if b == 0 && id != 0 {
                // Banners never appear as block items; keep the id stable anyway.
                return (323, 0);
            }
            let damage = if b == id as u16 && m == (damage & 15) as u8 {
                damage
            } else {
                i16::from(m)
            };
            (b as i16, damage)
        }
        // Prismarine shard / crystals: quartz / glowstone dust.
        409 => (406, 0),
        410 => (348, 0),
        // Raw / cooked rabbit: chicken; rabbit stew: mushroom stew.
        411 => (365, 0),
        412 => (366, 0),
        413 => (282, 0),
        // Rabbit's foot: feather; rabbit hide: leather.
        414 => (288, 0),
        415 => (334, 0),
        // Armor stand: stick.
        416 => (280, 0),
        // Raw / cooked mutton: beef / steak.
        423 => (363, 0),
        424 => (364, 0),
        // Banner: sign.
        425 => (323, 0),
        // Wooden doors: oak door.
        427..=431 => (324, 0),
        _ => (id, damage),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutions() {
        assert_eq!(block(1, 3), (1, 0));
        assert_eq!(block(2, 0), (2, 0));
        assert_eq!(block(35, 14), (35, 14));
        assert_eq!(block(182, 8), (44, 9));
        assert_eq!(block(193, 9), (64, 9));
        assert_eq!(item(1, 5), (1, 0));
        assert_eq!(item(35, 3), (35, 3));
        assert_eq!(item(276, 120), (276, 120));
        assert_eq!(item(179, 2), (24, 2));
        assert_eq!(item(423, 0), (363, 0));
    }
}
