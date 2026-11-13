//! Ports of the `StrictMath` functions vanilla depends on.
//!
//! `StrictMath` is specified to return exactly the results of fdlibm 5.3, so these are
//! straight ports of the fdlibm algorithms (not of any Java source). Platform `libm`
//! implementations may differ in the last ulp, which would desynchronize e.g.
//! `Random.nextGaussian`.
//
// The algorithm and constants below are derived from fdlibm, which carries this notice:
//
//   Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.
//
//   Developed at SunSoft, a Sun Microsystems, Inc. business.
//   Permission to use, copy, modify, and distribute this
//   software is freely granted, provided that this notice
//   is preserved.

const LN2_HI: f64 = f64::from_bits(0x3fe62e42_fee00000);
const LN2_LO: f64 = f64::from_bits(0x3dea39ef_35793c76);
const TWO54: f64 = f64::from_bits(0x43500000_00000000);
const LG1: f64 = f64::from_bits(0x3fe55555_55555593);
const LG2: f64 = f64::from_bits(0x3fd99999_9997fa04);
const LG3: f64 = f64::from_bits(0x3fd24924_94229359);
const LG4: f64 = f64::from_bits(0x3fcc71c5_1d8e78af);
const LG5: f64 = f64::from_bits(0x3fc74664_96cb03de);
const LG6: f64 = f64::from_bits(0x3fc39a09_d078c69f);
const LG7: f64 = f64::from_bits(0x3fc2f112_df3e5244);

fn high_word(x: f64) -> i32 {
    (x.to_bits() >> 32) as i32
}

fn low_word(x: f64) -> u32 {
    x.to_bits() as u32
}

fn with_high_word(x: f64, hi: i32) -> f64 {
    f64::from_bits((u64::from(hi as u32) << 32) | u64::from(low_word(x)))
}

/// Natural logarithm with exactly the results of Java's `StrictMath.log` (fdlibm `log`).
///
/// ```
/// use krol_java::strict_log;
/// assert_eq!(strict_log(1.0), 0.0);
/// assert!(strict_log(-1.0).is_nan());
/// assert_eq!(strict_log(0.0), f64::NEG_INFINITY);
/// ```
#[allow(clippy::many_single_char_names)]
pub fn strict_log(mut x: f64) -> f64 {
    let mut hx = high_word(x);
    let lx = low_word(x);
    let mut k: i32 = 0;

    if hx < 0x0010_0000 {
        // x < 2^-1022
        if ((hx & 0x7fff_ffff) as u32 | lx) == 0 {
            return f64::NEG_INFINITY; // log(+-0) = -inf
        }
        if hx < 0 {
            return f64::NAN; // log(negative) = NaN
        }
        // Subnormal: scale up.
        k -= 54;
        x *= TWO54;
        hx = high_word(x);
    }
    if hx >= 0x7ff0_0000 {
        return x + x; // inf or NaN
    }
    k += (hx >> 20) - 1023;
    hx &= 0x000f_ffff;
    let i = (hx + 0x95f64) & 0x10_0000;
    x = with_high_word(x, hx | (i ^ 0x3ff0_0000)); // normalize x or x/2
    k += i >> 20;
    let f = x - 1.0;

    if (0x000f_ffff & (2 + hx)) < 3 {
        // |f| < 2^-20
        if f == 0.0 {
            if k == 0 {
                return 0.0;
            }
            let dk = f64::from(k);
            return dk * LN2_HI + dk * LN2_LO;
        }
        let r = f * f * (0.5 - (1.0 / 3.0) * f);
        if k == 0 {
            return f - r;
        }
        let dk = f64::from(k);
        return dk * LN2_HI - ((r - dk * LN2_LO) - f);
    }

    let s = f / (2.0 + f);
    let dk = f64::from(k);
    let z = s * s;
    let mut i = hx - 0x6147a;
    let w = z * z;
    let j = 0x6b851 - hx;
    let t1 = w * (LG2 + w * (LG4 + w * LG6));
    let t2 = z * (LG1 + w * (LG3 + w * (LG5 + w * LG7)));
    i |= j;
    let r = t2 + t1;
    if i > 0 {
        let hfsq = 0.5 * f * f;
        if k == 0 {
            f - (hfsq - s * (hfsq + r))
        } else {
            dk * LN2_HI - ((hfsq - (s * (hfsq + r) + dk * LN2_LO)) - f)
        }
    } else if k == 0 {
        f - s * (f - r)
    } else {
        dk * LN2_HI - ((s * (f - r) - dk * LN2_LO) - f)
    }
}
