//! Vanilla 1.8.8's 2D simplex noise (`NoiseGeneratorSimplex`) and its octave sum
//! (`NoiseGeneratorPerlin`), used for the biome temperature variation above y = 64 and
//! later by world generation.

use krol_java::JavaRandom;

/// The twelve gradient directions (only x and y are used in 2D).
const GRADIENTS: [[i32; 3]; 12] = [
    [1, 1, 0],
    [-1, 1, 0],
    [1, -1, 0],
    [-1, -1, 0],
    [1, 0, 1],
    [-1, 0, 1],
    [1, 0, -1],
    [-1, 0, -1],
    [0, 1, 1],
    [0, -1, 1],
    [0, 1, -1],
    [0, -1, -1],
];
/// Scale of the summed corner contributions.
const OUTPUT_SCALE: f64 = 70.0;
/// Radius term of a corner's contribution falloff.
const FALLOFF: f64 = 0.5;

/// One simplex noise generator with its permutation table.
#[derive(Debug, Clone)]
pub struct SimplexNoise {
    perm: [i32; 512],
    /// Coordinate offsets (`xo`, `yo`, `zo`); only the bulk generator uses them.
    #[allow(dead_code)] // TODO(M6): the bulk noise of world generation.
    offset: [f64; 3],
}

/// Vanilla's floor for the skew: `x > 0 ? (int) x : (int) x - 1` (vanilla quirk: whole
/// negative numbers land one too low).
fn fast_floor(x: f64) -> i32 {
    if x > 0.0 { x as i32 } else { (x as i32) - 1 }
}

fn dot(g: [i32; 3], x: f64, y: f64) -> f64 {
    f64::from(g[0]) * x + f64::from(g[1]) * y
}

impl SimplexNoise {
    /// `new NoiseGeneratorSimplex(random)`: three offsets, then a shuffled permutation.
    pub fn new(rand: &mut JavaRandom) -> Self {
        let offset = [
            rand.next_double() * 256.0,
            rand.next_double() * 256.0,
            rand.next_double() * 256.0,
        ];
        let mut perm = [0i32; 512];
        for (i, p) in perm.iter_mut().take(256).enumerate() {
            *p = i as i32;
        }
        for i in 0..256 {
            let j = rand.next_int_bounded(256 - i as i32) as usize + i;
            perm.swap(i, j);
            perm[i + 256] = perm[i];
        }
        Self { perm, offset }
    }

    /// `func_151605_a`: the noise at `(x, y)`.
    pub fn noise(&self, x: f64, y: f64) -> f64 {
        let sqrt3 = 3.0f64.sqrt();
        let skew = FALLOFF * (sqrt3 - 1.0);
        let s = (x + y) * skew;
        let i = fast_floor(x + s);
        let j = fast_floor(y + s);
        let unskew = (3.0 - sqrt3) / 6.0;
        let t = f64::from(i + j) * unskew;
        let x0 = x - (f64::from(i) - t);
        let y0 = y - (f64::from(j) - t);
        let (i1, j1): (usize, usize) = if x0 > y0 { (1, 0) } else { (0, 1) };
        let x1 = x0 - i1 as f64 + unskew;
        let y1 = y0 - j1 as f64 + unskew;
        let x2 = x0 - 1.0 + 2.0 * unskew;
        let y2 = y0 - 1.0 + 2.0 * unskew;
        let ii = (i & 0xff) as usize;
        let jj = (j & 0xff) as usize;
        let p = &self.perm;
        let g0 = p[ii + p[jj] as usize] % 12;
        let g1 = p[ii + i1 + p[jj + j1] as usize] % 12;
        let g2 = p[ii + 1 + p[jj + 1] as usize] % 12;
        let corner = |g: i32, cx: f64, cy: f64| {
            let mut t = FALLOFF - cx * cx - cy * cy;
            if t < 0.0 {
                0.0
            } else {
                t *= t;
                t * t * dot(GRADIENTS[g as usize], cx, cy)
            }
        };
        OUTPUT_SCALE * (corner(g0, x0, y0) + corner(g1, x1, y1) + corner(g2, x2, y2))
    }
}

/// Octaves of simplex noise (`NoiseGeneratorPerlin`).
#[derive(Debug, Clone)]
pub struct OctaveNoise {
    octaves: Vec<SimplexNoise>,
}

impl OctaveNoise {
    /// `new NoiseGeneratorPerlin(random, octaves)`.
    pub fn new(rand: &mut JavaRandom, octaves: usize) -> Self {
        Self {
            octaves: (0..octaves).map(|_| SimplexNoise::new(rand)).collect(),
        }
    }

    /// `func_151601_a`: the sum of the octaves, each sampled at half the frequency of
    /// the one before and weighted by the inverse of that frequency.
    pub fn value(&self, x: f64, y: f64) -> f64 {
        let mut sum = 0.0;
        let mut scale = 1.0;
        for o in &self.octaves {
            sum += o.noise(x * scale, y * scale) / scale;
            scale /= 2.0;
        }
        sum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// From `tools/datagen/NoiseVectors.java` on vanilla 1.8.8: `(x, y, seed 1234 with 1
    /// octave, seed 42 with 4 octaves)` as double bits.
    const VECTORS: [(f64, f64, u64, u64); 5] = [
        (0.0, 0.0, 0, 0),
        (1.5, -2.25, 0x3fd4158d61164130, 0x400638738392399b),
        (-58.875, 54.125, 0x3fd39b3f9e861baf, 0x400889dbbd174806),
        (123.456, 789.012, 0xbfe7bb4c5bda2907, 0x3ff8a949f07a1a64),
        (-10000.0, 3.3, 0x3fe3b6e3787f23cd, 0x4011c3c936699d87),
    ];

    #[test]
    fn matches_vanilla() {
        let temperature = OctaveNoise::new(&mut JavaRandom::new(1234), 1);
        let four = OctaveNoise::new(&mut JavaRandom::new(42), 4);
        for (x, y, a, b) in VECTORS {
            assert_eq!(
                temperature.value(x, y).to_bits(),
                a,
                "seed 1234 at {x}, {y}"
            );
            assert_eq!(four.value(x, y).to_bits(), b, "seed 42 at {x}, {y}");
        }
    }
}
