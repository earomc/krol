//! Java's `Double.toString` / `Float.toString` formatting.
//!
//! Java prints the shortest decimal that round-trips, as `ddd.ddd` for magnitudes in
//! `[10^-3, 10^7)` and as computerized scientific notation (`d.dddE<exp>`) otherwise,
//! always with at least one digit after the point (`4.0`, `1.0E7`).
//!
//! Known deviation: JDK 8's algorithm occasionally emits a non-shortest digit string;
//! krol always uses the shortest one (Rust's formatter). The difference only shows for
//! rare values.

/// Formats like Java's `Double.toString(value)`.
///
/// ```
/// use krol_java::fmt::java_double;
/// assert_eq!(java_double(4.0), "4.0");
/// assert_eq!(java_double(-471.5), "-471.5");
/// assert_eq!(java_double(1.0e7), "1.0E7");
/// assert_eq!(java_double(0.0001), "1.0E-4");
/// ```
pub fn java_double(value: f64) -> String {
    format_java(value, value.abs(), |v| format!("{v:e}"), |v| format!("{v}"))
}

/// Formats like Java's `Float.toString(value)`.
pub fn java_float(value: f32) -> String {
    format_java(
        f64::from(value),
        f64::from(value.abs()),
        |_| format!("{value:e}"),
        |_| float_plain_half_even(value),
    )
}

/// Rust's shortest representation, but with Java's tie-break: when the float lies exactly
/// halfway between two shortest candidates, Java picks the one with an even last digit.
fn float_plain_half_even(value: f32) -> String {
    let s = format!("{value}");
    let Some((int_part, frac)) = s.split_once('.') else {
        return s;
    };
    let k = frac.len() as i32;
    if k == 0 || k > 9 {
        return s;
    }
    // A float times 10^k (k <= 9) is exact in f64.
    let scaled = f64::from(value) * 10f64.powi(k);
    if (scaled.abs().fract() - 0.5).abs() != 0.0 {
        return s;
    }
    let even = {
        let floor = scaled.abs().floor();
        if floor % 2.0 == 0.0 {
            floor
        } else {
            floor + 1.0
        }
    };
    let digits = format!("{even:.0}");
    let digits = format!("{digits:0>width$}", width = k as usize + 1);
    let (i, f) = digits.split_at(digits.len() - k as usize);
    let sign = if int_part.starts_with('-') { "-" } else { "" };
    let candidate = format!("{sign}{i}.{f}");
    // Only accept it if it still round-trips (it always does for genuine ties).
    if candidate.parse::<f32>() == Ok(value) {
        candidate
    } else {
        s
    }
}

fn format_java(
    value: f64,
    abs: f64,
    sci: impl Fn(f64) -> String,
    plain: impl Fn(f64) -> String,
) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .into();
    }
    if (1e-3..1e7).contains(&abs) {
        let s = plain(value);
        if s.contains('.') { s } else { format!("{s}.0") }
    } else {
        // Rust: "1.5e7" / "1e-4" -> Java: "1.5E7" / "1.0E-4".
        let s = sci(value);
        let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
        let mantissa = if mantissa.contains('.') {
            mantissa.to_owned()
        } else {
            format!("{mantissa}.0")
        };
        format!("{mantissa}E{exp}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles() {
        assert_eq!(java_double(0.0), "0.0");
        assert_eq!(java_double(-0.0), "-0.0");
        assert_eq!(java_double(0.1), "0.1");
        assert_eq!(java_double(0.001), "0.001");
        assert_eq!(java_double(9_999_999.0), "9999999.0");
        assert_eq!(java_double(12_345_678.9), "1.23456789E7");
        assert_eq!(java_double(-0.0784000015258789), "-0.0784000015258789");
        assert_eq!(java_double(f64::NAN), "NaN");
        assert_eq!(java_double(f64::NEG_INFINITY), "-Infinity");
    }

    #[test]
    fn floats() {
        assert_eq!(java_float(0.1), "0.1");
        assert_eq!(java_float(20.0), "20.0");
        assert_eq!(java_float(1.0e10), "1.0E10");
    }
}
