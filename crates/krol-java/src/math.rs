//! Vanilla's math helpers (vanilla: `MathHelper`, MCP names in each item's docs).
//!
//! Vanilla deliberately trades accuracy for speed in places — most notably `sin`/`cos`
//! use a 65536-entry lookup table — and many call sites depend on Java's float→int cast
//! semantics. Use these helpers wherever vanilla uses `MathHelper` rather than the
//! "better" Rust standard library equivalents.
//!
//! Java's `(int) x` cast saturates and maps NaN to 0, which is exactly Rust's `as` cast,
//! so plain `as` is used for those conversions.

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::JavaRandom;

const TABLE_SIZE: usize = 65536;
/// `65536 / (2π)` as vanilla's float literal.
const RADIANS_TO_INDEX: f32 = 10430.378;

fn sin_table() -> &'static [f32; TABLE_SIZE] {
    static TABLE: OnceLock<Box<[f32; TABLE_SIZE]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = Box::new([0.0f32; TABLE_SIZE]);
        for (i, v) in table.iter_mut().enumerate() {
            // Same operation order as vanilla: ((i * PI) * 2) / 65536.
            *v = (i as f64 * PI * 2.0 / 65536.0).sin() as f32;
        }
        table
    })
}

/// FNV-1a hash over the bits of the sine table; used by tests to compare the whole table
/// against a JVM-generated reference.
#[doc(hidden)]
pub fn sin_table_fnv1a() -> u64 {
    sin_table().iter().fold(0xcbf2_9ce4_8422_2325u64, |h, v| {
        (h ^ u64::from(v.to_bits())).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Table-based sine of `radians` (vanilla: `MathHelper.sin`).
///
/// ```
/// use krol_java::math;
/// assert_eq!(math::sin(0.0), 0.0);
/// assert!((math::sin(std::f32::consts::FRAC_PI_2) - 1.0).abs() < 1e-4);
/// ```
pub fn sin(radians: f32) -> f32 {
    sin_table()[((radians * RADIANS_TO_INDEX) as i32 & 0xffff) as usize]
}

/// Table-based cosine of `radians` (vanilla: `MathHelper.cos`).
pub fn cos(radians: f32) -> f32 {
    sin_table()[((radians * RADIANS_TO_INDEX + 16384.0) as i32 & 0xffff) as usize]
}

/// Square root computed in double precision, returned as float
/// (vanilla: `MathHelper.sqrt_float`).
pub fn sqrt_f32(value: f32) -> f32 {
    f64::from(value).sqrt() as f32
}

/// Square root of a double, returned as float (vanilla: `MathHelper.sqrt_double`).
pub fn sqrt_f64(value: f64) -> f32 {
    value.sqrt() as f32
}

/// Floor of a float as an int, via truncation (vanilla: `MathHelper.floor_float`).
pub fn floor_f32(value: f32) -> i32 {
    let i = value as i32;
    if value < i as f32 { i - 1 } else { i }
}

/// Floor of a double as an int, via truncation (vanilla: `MathHelper.floor_double`).
///
/// ```
/// use krol_java::math;
/// assert_eq!(math::floor_f64(-0.5), -1);
/// assert_eq!(math::floor_f64(1e20), i32::MAX); // saturates like Java's cast
/// ```
pub fn floor_f64(value: f64) -> i32 {
    let i = value as i32;
    if value < f64::from(i) { i - 1 } else { i }
}

/// Floor of a double as a long (vanilla: `MathHelper.floor_double_long`).
pub fn floor_f64_long(value: f64) -> i64 {
    let i = value as i64;
    if value < i as f64 { i - 1 } else { i }
}

/// Ceiling of a float as an int (vanilla: `MathHelper.ceiling_float_int`).
pub fn ceil_f32(value: f32) -> i32 {
    let i = value as i32;
    if value > i as f32 { i + 1 } else { i }
}

/// Ceiling of a double as an int (vanilla: `MathHelper.ceiling_double_int`).
pub fn ceil_f64(value: f64) -> i32 {
    let i = value as i32;
    if value > f64::from(i) { i + 1 } else { i }
}

/// Absolute value of a float; returns `-0.0` unchanged (vanilla: `MathHelper.abs`).
pub fn abs_f32(value: f32) -> f32 {
    if value >= 0.0 { value } else { -value }
}

/// Clamps `value` into `[min, max]` (vanilla: `MathHelper.clamp_int`).
pub fn clamp_i32(value: i32, min: i32, max: i32) -> i32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// Clamps `value` into `[min, max]`; NaN passes through (vanilla: `MathHelper.clamp_float`).
pub fn clamp_f32(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// Clamps `value` into `[min, max]`; NaN passes through (vanilla: `MathHelper.clamp_double`).
pub fn clamp_f64(value: f64, min: f64, max: f64) -> f64 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// Linear interpolation from `lower` to `upper` with `slide` clamped to `[0, 1]`
/// (vanilla: `MathHelper.denormalizeClamp`).
pub fn clamped_lerp(lower: f64, upper: f64, slide: f64) -> f64 {
    if slide < 0.0 {
        lower
    } else if slide > 1.0 {
        upper
    } else {
        lower + (upper - lower) * slide
    }
}

/// The larger of the absolute values of `a` and `b` (vanilla: `MathHelper.abs_max`).
pub fn abs_max(a: f64, b: f64) -> f64 {
    let a = if a < 0.0 { -a } else { a };
    let b = if b < 0.0 { -b } else { b };
    if a > b { a } else { b }
}

/// Floor division of `value` by a positive `factor` (vanilla: `MathHelper.bucketInt`).
pub fn bucket_i32(value: i32, factor: i32) -> i32 {
    if value < 0 {
        -((-value - 1) / factor) - 1
    } else {
        value / factor
    }
}

/// A random int in `min..=max`, or `min` if `min >= max`
/// (vanilla: `MathHelper.getRandomIntegerInRange`).
pub fn random_i32_in_range(rand: &mut JavaRandom, min: i32, max: i32) -> i32 {
    if min >= max {
        min
    } else {
        rand.next_int_bounded(max - min + 1) + min
    }
}

/// A random double in `[min, max)`, or `min` if `min >= max`
/// (vanilla: `MathHelper.getRandomDoubleInRange`).
pub fn random_f64_in_range(rand: &mut JavaRandom, min: f64, max: f64) -> f64 {
    if min >= max {
        min
    } else {
        rand.next_double() * (max - min) + min
    }
}

/// Wraps an angle in degrees into `[-180, 180)` (vanilla: `MathHelper.wrapAngleTo180_float`).
///
/// ```
/// use krol_java::math;
/// assert_eq!(math::wrap_degrees_f32(190.0), -170.0);
/// assert_eq!(math::wrap_degrees_f32(-180.0), -180.0);
/// ```
pub fn wrap_degrees_f32(value: f32) -> f32 {
    // Rust's float `%` has the same truncating semantics as Java's.
    let mut v = value % 360.0;
    if v >= 180.0 {
        v -= 360.0;
    }
    if v < -180.0 {
        v += 360.0;
    }
    v
}

/// Wraps an angle in degrees into `[-180, 180)` (vanilla: `MathHelper.wrapAngleTo180_double`).
pub fn wrap_degrees_f64(value: f64) -> f64 {
    let mut v = value % 360.0;
    if v >= 180.0 {
        v -= 360.0;
    }
    if v < -180.0 {
        v += 360.0;
    }
    v
}

/// Smallest power of two `>= value` for positive values, with Java's overflow behavior
/// (vanilla: `MathHelper.roundUpToPowerOfTwo`).
pub fn round_up_to_power_of_two(value: i32) -> i32 {
    let mut j = value.wrapping_sub(1);
    j |= j >> 1;
    j |= j >> 2;
    j |= j >> 4;
    j |= j >> 8;
    j |= j >> 16;
    j.wrapping_add(1)
}

/// A random version-4 UUID drawn from `rand`, as vanilla generates entity UUIDs
/// (vanilla: `MathHelper.getRandomUuid`). Returns the 128-bit value with the most
/// significant Java `long` in the high 64 bits.
pub fn random_uuid(rand: &mut JavaRandom) -> u128 {
    let most = (rand.next_long() & !0xf000) | 0x4000;
    let least = (rand.next_long() & 0x3fff_ffff_ffff_ffff) | i64::MIN;
    (u128::from(most as u64) << 64) | u128::from(least as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floors_and_ceils() {
        assert_eq!(floor_f32(-0.0), 0);
        assert_eq!(floor_f32(-1.5), -2);
        assert_eq!(floor_f64(2.999), 2);
        assert_eq!(floor_f64(f64::NAN), 0);
        assert_eq!(floor_f64_long(-1e-9), -1);
        assert_eq!(ceil_f32(1.1), 2);
        assert_eq!(ceil_f64(-1.1), -1);
    }

    #[test]
    fn cos_is_shifted_sin() {
        assert_eq!(cos(0.0), 1.0);
        assert!(sin(std::f32::consts::PI).abs() < 1e-3);
    }

    #[test]
    fn misc() {
        assert_eq!(bucket_i32(-1, 16), -1);
        assert_eq!(bucket_i32(-16, 16), -1);
        assert_eq!(bucket_i32(-17, 16), -2);
        assert_eq!(round_up_to_power_of_two(17), 32);
        assert_eq!(round_up_to_power_of_two(16), 16);
        assert_eq!(wrap_degrees_f64(540.0), -180.0);
        assert_eq!(clamped_lerp(0.0, 10.0, 2.0), 10.0);
        assert_eq!(abs_max(-3.0, 2.0), 3.0);
    }

    #[test]
    fn uuid_has_version_and_variant() {
        let id = random_uuid(&mut JavaRandom::new(0));
        assert_eq!((id >> 76) & 0xf, 4);
        assert_eq!((id >> 62) & 0b11, 0b10);
    }
}
