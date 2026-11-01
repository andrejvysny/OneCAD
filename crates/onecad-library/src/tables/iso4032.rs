//! ISO 4032 hexagon nut, style 1 (spec §6.2).
//!
//! Rust-side mirror of the worker's `FastenerTables.cpp::iso4032_table()`;
//! a SEPARATE copy, never a geometry authority (see `tables/mod.rs`). The
//! tests below pin the same values as the worker table.
//!
//! Source: BOLTS (`github.com/boltsparts/BOLTS_archive`, `data/nut.blt`,
//! class `hexagonnut1`, retrieved 2026-08-13), LGPL 2.1+, author Johannes
//! Reinhardt — see `THIRD_PARTY_NOTICES`. `m` is BOLTS' `m_max`. Values are
//! verbatim from the source.

/// One ISO 4032 size row. `d1` thread nominal Ø, `s` width across flats,
/// `m` thickness (all mm).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Iso4032Row {
    pub d1: f64,
    pub s: f64,
    pub m: f64,
}

/// `(thread, d1, s, m)`, M2–M12.
const ISO4032_ROWS: &[(&str, f64, f64, f64)] = &[
    ("M2", 2.0, 4.0, 1.6),
    ("M2.5", 2.5, 5.0, 2.0),
    ("M3", 3.0, 5.5, 2.4),
    ("M4", 4.0, 7.0, 3.2),
    ("M5", 5.0, 8.0, 4.7),
    ("M6", 6.0, 10.0, 5.2),
    ("M8", 8.0, 13.0, 6.8),
    ("M10", 10.0, 16.0, 8.4),
    ("M12", 12.0, 18.0, 10.8),
];

/// ISO 4032 hexagon nut table.
pub struct Iso4032Table;

impl Iso4032Table {
    #[must_use]
    pub fn row(thread: &str) -> Option<Iso4032Row> {
        ISO4032_ROWS
            .iter()
            .find(|(key, ..)| *key == thread)
            .map(|(_, d1, s, m)| Iso4032Row {
                d1: *d1,
                s: *s,
                m: *m,
            })
    }

    #[must_use]
    pub fn threads() -> Vec<&'static str> {
        ISO4032_ROWS.iter().map(|(key, ..)| *key).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_thread_is_none_not_a_fabricated_row() {
        assert!(Iso4032Table::row("M99").is_none());
    }

    #[test]
    fn threads_lists_every_seeded_size_in_order() {
        assert_eq!(
            Iso4032Table::threads(),
            vec!["M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12"]
        );
    }

    #[test]
    fn spot_check_iso4032_against_bolts() {
        let m6 = Iso4032Table::row("M6").unwrap();
        assert_eq!((m6.d1, m6.s, m6.m), (6.0, 10.0, 5.2));
        let m10 = Iso4032Table::row("M10").unwrap();
        assert_eq!((m10.s, m10.m), (16.0, 8.4));
    }

    // Every row, typed out as the WORKER's `iso4032_table()` holds it.
    #[test]
    fn every_row_agrees_with_the_worker_table() {
        let worker: &[(&str, f64, f64, f64)] = &[
            ("M2", 2.0, 4.0, 1.6),
            ("M2.5", 2.5, 5.0, 2.0),
            ("M3", 3.0, 5.5, 2.4),
            ("M4", 4.0, 7.0, 3.2),
            ("M5", 5.0, 8.0, 4.7),
            ("M6", 6.0, 10.0, 5.2),
            ("M8", 8.0, 13.0, 6.8),
            ("M10", 10.0, 16.0, 8.4),
            ("M12", 12.0, 18.0, 10.8),
        ];
        for (thread, d1, s, m) in worker {
            let row = Iso4032Table::row(thread)
                .unwrap_or_else(|| panic!("{thread} missing from Rust-side table"));
            assert_eq!((row.d1, row.s, row.m), (*d1, *s, *m), "{thread}");
        }
        assert_eq!(Iso4032Table::threads().len(), worker.len());
    }
}
