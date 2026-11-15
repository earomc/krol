//! Biome weather properties of vanilla 1.8.8 (`BiomeGenBase`), extracted by
//! `tools/datagen/BiomeDump.java` into `data/biomes.txt`, and the weather queries built
//! on them (`World.canLightningStrike`, `canSnowAt`, `isBlockinHighHumidity`).

use std::sync::OnceLock;

use super::Game;
use super::block_data::material;
use super::noise::OctaveNoise;

/// Biome ids in the table.
const IDS: usize = 256;
/// `BiomeGenBase.plains`, used where a chunk has no biome stored.
const PLAINS: u8 = 1;
/// Above this height the temperature drops with height and noise.
const TEMPERATURE_FALLOFF_START: i32 = 64;
/// At or below this temperature it snows instead of raining.
const SNOW_TEMPERATURE: f32 = 0.15;
/// The seed of the temperature noise (`BiomeGenBase.temperatureNoise`).
const TEMPERATURE_NOISE_SEED: i64 = 1234;

/// A biome's weather properties.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Biome {
    /// `temperature`.
    pub temperature: f32,
    /// `rainfall`.
    pub rainfall: f32,
    /// `getEnableSnow`.
    pub snowy: bool,
    /// `canSpawnLightningBolt`: rain falls here.
    pub lightning: bool,
    /// `isHighHumidity`.
    pub humid: bool,
}

fn table() -> &'static [Option<Biome>; IDS] {
    static TABLE: OnceLock<[Option<Biome>; IDS]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [None; IDS];
        for line in include_str!("../../data/biomes.txt").lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            let bits = |s: &str| u32::from_str_radix(s, 16).ok().map(f32::from_bits);
            if let [id, temp, rain, snowy, lightning, humid] = f[..]
                && let (Ok(id), Some(temperature), Some(rainfall)) =
                    (id.parse::<usize>(), bits(temp), bits(rain))
                && id < IDS
            {
                t[id] = Some(Biome {
                    temperature,
                    rainfall,
                    snowy: snowy == "1",
                    lightning: lightning == "1",
                    humid: humid == "1",
                });
            }
        }
        t
    })
}

/// The properties of a biome id (unknown ids: plains, like `Chunk.getBiome`).
pub fn biome(id: u8) -> Biome {
    let t = table();
    t[usize::from(id)]
        .or(t[usize::from(PLAINS)])
        .unwrap_or(Biome {
            temperature: 0.8,
            rainfall: 0.4,
            snowy: false,
            lightning: true,
            humid: false,
        })
}

fn temperature_noise() -> &'static OctaveNoise {
    static NOISE: OnceLock<OctaveNoise> = OnceLock::new();
    NOISE.get_or_init(|| {
        OctaveNoise::new(&mut krol_java::JavaRandom::new(TEMPERATURE_NOISE_SEED), 1)
    })
}

/// `BiomeGenBase.getFloatTemperature`: colder with height above y = 64.
pub fn temperature(b: Biome, pos: (i32, i32, i32)) -> f32 {
    if pos.1 > TEMPERATURE_FALLOFF_START {
        let n = (temperature_noise().value(f64::from(pos.0) / 8.0, f64::from(pos.2) / 8.0) * 4.0)
            as f32;
        b.temperature - (n + pos.1 as f32 - 64.0) * 0.05 / 30.0
    } else {
        b.temperature
    }
}

impl Game {
    /// `World.getBiomeGenForCoords` (chunks without stored biomes count as plains).
    ///
    /// TODO(M6): vanilla generates the biome of such columns.
    pub(super) fn biome_at(&mut self, x: i32, z: i32) -> Biome {
        biome(self.biome_id_at(x, z))
    }

    /// The biome id at a column (`BiomeGenBase.biomeID` of
    /// [`Self::biome_at`]): unknown ids and chunks without biomes count as plains.
    pub(super) fn biome_id_at(&mut self, x: i32, z: i32) -> u8 {
        let id = self.level.overworld.biome_id(x, z);
        if id == u8::MAX || table()[usize::from(id)].is_none() {
            PLAINS
        } else {
            id
        }
    }

    /// `Chunk.getPrecipitationHeight`: one above the highest block that blocks
    /// movement or is a liquid.
    pub(super) fn precipitation_height(&mut self, x: i32, z: i32) -> i32 {
        let w = &mut self.level.overworld;
        let mut y = w.top_filled_y(x, z);
        while y > 0 {
            let m = material(w.block(x, y, z) >> 4);
            if m.blocks_movement() || m.is_liquid() {
                return y + 1;
            }
            y -= 1;
        }
        -1
    }

    /// `World.canSnowAt(pos, false)`: cold enough for snow.
    pub(super) fn cold_enough_for_snow(&mut self, pos: (i32, i32, i32)) -> bool {
        let b = self.biome_at(pos.0, pos.2);
        temperature(b, pos) <= SNOW_TEMPERATURE
    }

    /// `World.canLightningStrike`: rain falls on this block right now.
    pub(super) fn rains_at(&mut self, pos: (i32, i32, i32)) -> bool {
        if !self.is_raining() || !self.level.overworld.sees_sky(pos) {
            return false;
        }
        if self.precipitation_height(pos.0, pos.2) > pos.1 {
            return false;
        }
        let b = self.biome_at(pos.0, pos.2);
        !b.snowy && !self.cold_enough_for_snow(pos) && b.lightning
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_biomes() {
        let plains = biome(1);
        assert_eq!(plains.temperature, 0.8);
        assert_eq!(plains.rainfall, 0.4);
        assert!(plains.lightning && !plains.snowy && !plains.humid);
        assert!(!biome(2).lightning); // desert
        assert!(biome(6).humid); // swampland
        assert!(biome(12).snowy); // ice plains
    }
}
