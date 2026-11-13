//! Server configuration files.

mod property_manager;
mod server_properties;

pub use property_manager::{PropertyManager, parse_java_int, parse_java_long};
pub use server_properties::ServerProperties;

use std::time::{SystemTime, UNIX_EPOCH};

/// Formats `time` like Java's `Date.toString()` (`EEE MMM dd HH:mm:ss zzz yyyy`).
///
/// Known deviation: Java uses the JVM's default time zone; krol always prints UTC. The
/// parity suite normalizes this timestamp line.
pub fn java_date_string(time: SystemTime) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{} {} {:02} {:02}:{:02}:{:02} UTC {}",
        DAYS[days.rem_euclid(7) as usize],
        MONTHS[(month - 1) as usize],
        day,
        sod / 3600,
        (sod / 60) % 60,
        sod % 60,
        year
    )
}

/// Converts days since 1970-01-01 to a proleptic Gregorian (year, month, day).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    // Howard Hinnant's algorithm.
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn formats_dates() {
        assert_eq!(java_date_string(UNIX_EPOCH), "Thu Jan 01 00:00:00 UTC 1970");
        let t = UNIX_EPOCH + Duration::from_secs(1_795_000_000);
        assert_eq!(java_date_string(t), "Wed Nov 18 11:06:40 UTC 2026");
        let leap = UNIX_EPOCH + Duration::from_secs(951_782_400); // 2000-02-29
        assert_eq!(java_date_string(leap), "Tue Feb 29 00:00:00 UTC 2000");
    }
}
