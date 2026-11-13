/// Computes Java's `String.hashCode()` for `s`.
///
/// Java hashes the UTF-16 code units of a string as `h = 31 * h + unit` with wrapping
/// 32-bit arithmetic. Characters outside the Basic Multilingual Plane therefore contribute
/// two code units (a surrogate pair), exactly as in Java.
///
/// ```
/// use krol_java::java_string_hash;
/// assert_eq!(java_string_hash(""), 0);
/// assert_eq!(java_string_hash("hello"), 99_162_322);
/// ```
pub fn java_string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, unit| {
        h.wrapping_mul(31).wrapping_add(i32::from(unit))
    })
}
