//! Backups are named by the instant they were taken, so that name has to be right.

use std::time::{Duration, UNIX_EPOCH};
use tapkey_core::instant::format_utc;

/// Expected values come from `date -u -r <seconds>` and from Python's datetime, which agree
/// with each other and were consulted before the implementation existed. Deriving them the way
/// the code derives them would make this test pass by construction.
#[test]
fn formats_known_instants_compactly_and_in_utc() {
    let cases = [
        (1_787_866_640_123u64, "20260827T213720.123Z"),
        (0, "19700101T000000.000Z"),
        (1_709_164_800_000, "20240229T000000.000Z"), // a leap day
        (1_704_067_199_999, "20231231T235959.999Z"), // the last millisecond of a year
        (1_704_067_200_000, "20240101T000000.000Z"),
        (1_798_761_600_000, "20270101T000000.000Z"),
        // The last day of a 400-year era and its neighbours. A mutation run showed that the
        // term correcting for it was untested, and it fires on one day every four centuries —
        // which is precisely the arithmetic nobody would notice being wrong.
        (951_696_000_000, "20000228T000000.000Z"),
        (951_782_400_000, "20000229T000000.000Z"),
        (951_868_800_000, "20000301T000000.000Z"),
        (13_574_476_800_000, "24000228T000000.000Z"),
        (13_574_563_200_000, "24000229T000000.000Z"),
        (13_574_649_600_000, "24000301T000000.000Z"),
        // 2100 is the century that is not a leap year, and the first day after it is where the
        // century correction stops being absorbed by integer division. Found by searching for
        // a day on which flipping that term's sign changes the answer — a mutation survived
        // every date chosen for being interesting to a human.
        (4_107_456_000_000, "21000228T000000.000Z"),
        (4_107_542_400_000, "21000301T000000.000Z"),
        (13_543_459_200_000, "23990306T000000.000Z"),
    ];
    for (ms, expected) in cases {
        assert_eq!(
            format_utc(UNIX_EPOCH + Duration::from_millis(ms)),
            expected,
            "at {ms} ms"
        );
    }
}

#[test]
fn the_epoch_itself_is_the_first_possible_name() {
    assert_eq!(format_utc(UNIX_EPOCH), "19700101T000000.000Z");
}

/// A name that sorts is what the sweep and the history list both rely on, so leap years and
/// month lengths are not a detail: get one wrong and the wrong backup is evicted.
#[test]
fn names_sort_in_the_order_the_instants_happened() {
    let mut names: Vec<String> = [
        1_709_164_800_000u64, // 2024-02-29, a leap day
        1_704_067_199_999,    // 2023-12-31T23:59:59.999Z
        1_704_067_200_000,    // 2024-01-01T00:00:00.000Z
        1_798_761_600_000,    // 2027-01-01
    ]
    .iter()
    .map(|ms| format_utc(UNIX_EPOCH + Duration::from_millis(*ms)))
    .collect();
    let sorted = {
        let mut c = names.clone();
        c.sort();
        c
    };
    names.sort_by_key(|n| n.clone());
    assert_eq!(names, sorted);
    assert!(names[0].starts_with("20231231"), "{names:?}");
    assert!(names.iter().any(|n| n.starts_with("20240229")), "{names:?}");
}

#[test]
fn a_name_carries_no_character_windows_forbids() {
    let t = UNIX_EPOCH + Duration::from_millis(1_787_866_640_123);
    let name = format_utc(t);
    for bad in [':', '/', '\\', '*', '?', '"', '<', '>', '|'] {
        assert!(!name.contains(bad), "{name} contains {bad}");
    }
}
