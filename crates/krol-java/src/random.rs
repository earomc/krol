use crate::strict_math::strict_log;

const MULTIPLIER: i64 = 0x5_DEEC_E66D;
const ADDEND: i64 = 0xB;
const MASK: i64 = (1 << 48) - 1;
/// `0x1.0p-53`, the unit used by `nextDouble`.
const DOUBLE_UNIT: f64 = 1.0 / (1u64 << 53) as f64;
/// `1 << 24` as a float, the divisor used by `nextFloat`.
const FLOAT_UNIT: f32 = (1u32 << 24) as f32;

/// A bit-exact port of `java.util.Random`.
///
/// This is the 48-bit linear congruential generator specified by the Java platform. Given
/// the same seed and the same sequence of calls, it produces exactly the values Java
/// produces, which vanilla world generation, mob AI and loot depend on.
///
/// Method names mirror Java's (`nextInt` → [`next_int`](Self::next_int), `nextInt(int)` →
/// [`next_int_bounded`](Self::next_int_bounded), ...). Preserve the *order and kind* of
/// calls vanilla makes on each instance — swapping a `nextInt(2)` for a `nextBoolean()`
/// changes the stream.
///
/// ```
/// use krol_java::JavaRandom;
///
/// let mut r = JavaRandom::new(42);
/// assert_eq!(r.next_int(), -1_170_105_035); // same as `new Random(42).nextInt()`
/// ```
#[derive(Debug, Clone)]
pub struct JavaRandom {
    seed: i64,
    next_next_gaussian: Option<f64>,
}

impl JavaRandom {
    /// Creates a generator seeded like `new Random(seed)`.
    pub fn new(seed: i64) -> Self {
        Self {
            seed: scramble(seed),
            next_next_gaussian: None,
        }
    }

    /// Re-seeds the generator like `Random.setSeed(seed)`, also discarding any cached
    /// Gaussian value.
    pub fn set_seed(&mut self, seed: i64) {
        self.seed = scramble(seed);
        self.next_next_gaussian = None;
    }

    /// Advances the generator and returns its top `bits` bits, like the protected
    /// `Random.next(int bits)`.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `bits` is not in `1..=32`.
    pub fn next_bits(&mut self, bits: u32) -> i32 {
        debug_assert!((1..=32).contains(&bits), "bits must be in 1..=32");
        self.seed = (self.seed.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND)) & MASK;
        // The seed is a non-negative 48-bit value, so an arithmetic shift is unsigned here.
        (self.seed >> (48 - bits)) as i32
    }

    /// Returns the next uniformly distributed `int`, like `Random.nextInt()`.
    pub fn next_int(&mut self) -> i32 {
        self.next_bits(32)
    }

    /// Returns a uniformly distributed value in `0..bound`, like `Random.nextInt(bound)`.
    ///
    /// # Panics
    ///
    /// Panics if `bound <= 0`, where Java throws `IllegalArgumentException`.
    pub fn next_int_bounded(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        let mut r = self.next_bits(31);
        let m = bound - 1;
        if bound & m == 0 {
            // Power of two: take the high bits.
            ((i64::from(bound) * i64::from(r)) >> 31) as i32
        } else {
            // Rejection sampling; the `< 0` test relies on 32-bit overflow, as in Java.
            let mut u = r;
            loop {
                r = u % bound;
                if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                    return r;
                }
                u = self.next_bits(31);
            }
        }
    }

    /// Returns the next uniformly distributed `long`, like `Random.nextLong()`.
    pub fn next_long(&mut self) -> i64 {
        let hi = i64::from(self.next_bits(32)) << 32;
        hi.wrapping_add(i64::from(self.next_bits(32)))
    }

    /// Returns the next uniformly distributed `boolean`, like `Random.nextBoolean()`.
    pub fn next_boolean(&mut self) -> bool {
        self.next_bits(1) != 0
    }

    /// Returns a uniformly distributed `float` in `[0, 1)`, like `Random.nextFloat()`.
    pub fn next_float(&mut self) -> f32 {
        self.next_bits(24) as f32 / FLOAT_UNIT
    }

    /// Returns a uniformly distributed `double` in `[0, 1)`, like `Random.nextDouble()`.
    pub fn next_double(&mut self) -> f64 {
        let hi = i64::from(self.next_bits(26)) << 27;
        (hi + i64::from(self.next_bits(27))) as f64 * DOUBLE_UNIT
    }

    /// Returns a normally distributed `double` (mean 0, standard deviation 1), like
    /// `Random.nextGaussian()`.
    ///
    /// Uses the polar method with `StrictMath.log` / `StrictMath.sqrt`, and caches the
    /// second value of each generated pair exactly as Java does.
    pub fn next_gaussian(&mut self) -> f64 {
        if let Some(cached) = self.next_next_gaussian.take() {
            return cached;
        }
        loop {
            let v1 = 2.0 * self.next_double() - 1.0;
            let v2 = 2.0 * self.next_double() - 1.0;
            let s = v1 * v1 + v2 * v2;
            if s < 1.0 && s != 0.0 {
                // sqrt is correctly rounded in IEEE 754, so `f64::sqrt` equals StrictMath.sqrt.
                let multiplier = (-2.0 * strict_log(s) / s).sqrt();
                self.next_next_gaussian = Some(v2 * multiplier);
                return v1 * multiplier;
            }
        }
    }
}

fn scramble(seed: i64) -> i64 {
    (seed ^ MULTIPLIER) & MASK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "bound must be positive")]
    fn bounded_rejects_zero() {
        JavaRandom::new(0).next_int_bounded(0);
    }

    #[test]
    fn set_seed_matches_new() {
        let mut a = JavaRandom::new(99);
        a.next_gaussian();
        a.set_seed(5);
        let mut b = JavaRandom::new(5);
        assert_eq!(a.next_gaussian().to_bits(), b.next_gaussian().to_bits());
    }
}
