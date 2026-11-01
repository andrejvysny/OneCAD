//! ISO 7089 plain washer, normal series, 200 HV (spec §6.2).
//!
//! Rust-side mirror of the worker's `FastenerTables.cpp::iso7089_table()`;
//! a SEPARATE copy, never a geometry authority (see `tables/mod.rs`).
//!
//! Source: BOLTS (`github.com/boltsparts/BOLTS_archive`, `data/washer.blt`,
//! class `plainwasher1`, retrieved 2026-08-13), LGPL 2.1+, author Johannes
//! Reinhardt — see `THIRD_PARTY_NOTICES`.
//!
//! ONE DELIBERATE DEVIATION FROM BOLTS: it lists M10 as inner Ø 10.0. ISO 7089
//! (and DIN 125 A, which BOLTS anchors this table to) both publish **10.5** —
//! a 10.0 bore would not pass an M10 bolt. Corrected here and pinned by a test
//! so a future re-import cannot silently undo it. Every other row is verbatim.

/// One ISO 7089 size row. `d1` inner Ø, `d2` outer Ø, `s` thickness (mm).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Iso7089Row {
    pub d1: f64,
    pub d2: f64,
    pub s: f64,
}

/// `(thread, d1, d2, s)`, M2–M12.
const ISO7089_ROWS: &[(&str, f64, f64, f64)] = &[
    ("M2", 2.2, 5.0, 0.3),
    ("M2.5", 2.7, 6.0, 0.5),
    ("M3", 3.2, 7.0, 0.5),
    ("M4", 4.3, 9.0, 0.8),
    ("M5", 5.3, 10.0, 1.0),
    ("M6", 6.4, 12.0, 1.6),
    ("M8", 8.4, 16.0, 1.6),
    ("M10", 10.5, 20.0, 2.0),
    ("M12", 13.0, 24.0, 2.5),
];

/// ISO 7089 plain washer table.
pub struct Iso7089Table;

impl Iso7089Table {
    #[must_use]
    pub fn row(thread: &str) -> Option<Iso7089Row> {
        ISO7089_ROWS
            .iter()
            .find(|(key, ..)| *key == thread)
            .map(|(_, d1, d2, s)| Iso7089Row {
                d1: *d1,
                d2: *d2,
                s: *s,
            })
    }

    #[must_use]
    pub fn threads() -> Vec<&'static str> {
        ISO7089_ROWS.iter().map(|(key, ..)| *key).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_thread_is_none_not_a_fabricated_row() {
        assert!(Iso7089Table::row("M99").is_none());
    }

    #[test]
    fn threads_lists_every_seeded_size_in_order() {
        assert_eq!(
            Iso7089Table::threads(),
            vec!["M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12"]
        );
    }

    #[test]
    fn spot_check_iso7089_against_its_source() {
        let m6 = Iso7089Table::row("M6").unwrap();
        assert_eq!((m6.d1, m6.d2, m6.s), (6.4, 12.0, 1.6));
    }

    // BOLTS lists M10 as a 10.0 bore; ISO 7089 and DIN 125 A publish 10.5.
    // Pinned on BOTH sides (the worker ctest pins the same value).
    #[test]
    fn iso7089_m10_bore_is_corrected_away_from_the_bolts_slip() {
        assert_eq!(Iso7089Table::row("M10").unwrap().d1, 10.5);
    }

    // Every row, typed out as the WORKER's `iso7089_table()` holds it.
    #[test]
    fn every_row_agrees_with_the_worker_table() {
        let worker: &[(&str, f64, f64, f64)] = &[
            ("M2", 2.2, 5.0, 0.3),
            ("M2.5", 2.7, 6.0, 0.5),
            ("M3", 3.2, 7.0, 0.5),
            ("M4", 4.3, 9.0, 0.8),
            ("M5", 5.3, 10.0, 1.0),
            ("M6", 6.4, 12.0, 1.6),
            ("M8", 8.4, 16.0, 1.6),
            ("M10", 10.5, 20.0, 2.0),
            ("M12", 13.0, 24.0, 2.5),
        ];
        for (thread, d1, d2, s) in worker {
            let row = Iso7089Table::row(thread)
                .unwrap_or_else(|| panic!("{thread} missing from Rust-side table"));
            assert_eq!((row.d1, row.d2, row.s), (*d1, *d2, *s), "{thread}");
        }
        assert_eq!(Iso7089Table::threads().len(), worker.len());
    }
}
