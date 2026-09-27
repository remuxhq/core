//! What a recording is called.

/// The name of a recording that started at this many seconds past the epoch.
///
/// A name, not a path: where it goes is the operator's folder and this never
/// learns it. The date is in the name because a folder of recordings is sorted
/// by name far more often than by anything else, and a name that sorts by name
/// is the same order as by time.
///
/// The arithmetic is here rather than a dependency because it is fifteen lines
/// of civil calendar and this crate's whole dependency tree is `serde`. It is
/// UTC: a recording named in local time is a recording that appears twice on
/// the night the clocks go back.
pub fn name(unix_seconds: i64) -> String {
    let (year, month, day, hour, minute, second) = civil(unix_seconds);
    format!("remux-{year:04}-{month:02}-{day:02}-{hour:02}{minute:02}{second:02}.mp4")
}

/// Seconds past the epoch as a civil date and time, UTC.
///
/// Howard Hinnant's `civil_from_days`, which is the shortest correct version
/// of this and handles every leap year without a table.
fn civil(unix_seconds: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = unix_seconds.div_euclid(86_400);
    let rest = unix_seconds.rem_euclid(86_400);

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };

    (
        year,
        month,
        day,
        (rest / 3600) as u32,
        ((rest % 3600) / 60) as u32,
        (rest % 60) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recording_is_named_after_the_moment_it_started() {
        // 2026-09-01 03:04:05 UTC
        assert_eq!(name(1_788_231_845), "remux-2026-09-01-030405.mp4");
    }

    #[test]
    fn the_epoch_itself() {
        assert_eq!(name(0), "remux-1970-01-01-000000.mp4");
    }

    #[test]
    fn a_leap_day_is_a_day() {
        // 2024-02-29 12:00:00 UTC
        assert_eq!(name(1_709_208_000), "remux-2024-02-29-120000.mp4");
    }

    #[test]
    fn the_end_of_a_century_that_is_not_a_leap_year() {
        // 1900 was not a leap year; 2000 was. 2000-03-01 00:00:00 UTC.
        assert_eq!(name(951_868_800), "remux-2000-03-01-000000.mp4");
    }

    #[test]
    fn names_sort_the_way_the_recordings_happened() {
        let mut made = vec![name(1_788_231_845), name(0), name(1_709_208_000)];
        made.sort();
        assert_eq!(
            made,
            vec![
                "remux-1970-01-01-000000.mp4",
                "remux-2024-02-29-120000.mp4",
                "remux-2026-09-01-030405.mp4",
            ]
        );
    }
}
