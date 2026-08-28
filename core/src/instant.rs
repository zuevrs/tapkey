//! Naming a moment.
//!
//! A backup's directory name is a compact UTC instant: unique without coordination, free of
//! the `:` Windows forbids, and sorting in the order things happened. The ordering the sweep
//! and the history list use comes from the manifest, but a name that also sorts makes a
//! directory listing legible, which is worth thirty lines of civil-calendar arithmetic rather
//! than a dependency.

use std::time::{SystemTime, UNIX_EPOCH};

/// `YYYYMMDDTHHMMSS.mmmZ`, always UTC.
pub fn format_utc(t: SystemTime) -> String {
    let ms = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let total_seconds = (ms / 1000) as i64;
    let millis = (ms % 1000) as u32;

    // Clamped above, so the calendar never sees a day before the epoch.
    let days = (total_seconds.div_euclid(86_400)) as u64;
    let secs_of_day = total_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);

    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}.{millis:03}Z",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    )
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 to a calendar date, with the year
/// shifted so that leap days land at the end of the era. Copied deliberately rather than
/// improvised — month lengths and leap years are exactly where a hand-rolled version is wrong
/// once every four years, and the wrong backup would then be the one evicted.
///
/// The original handles days before the epoch; `format_utc` clamps there, so that branch would
/// be unreachable code, and unreachable code that a mutation run flags is worse than absent.
fn civil_from_days(days: u64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era as i64 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11], March-based
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}
