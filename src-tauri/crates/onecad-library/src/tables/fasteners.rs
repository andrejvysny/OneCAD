//! The six fastener families WP-A1 added beside ISO 4762 (spec §6.2's seed
//! catalog): ISO 7380-1 button head, ISO 4014/4017 hex screws, ISO 4032 hex
//! nut, ISO 7089 / ISO 7093-1 plain washers.
//!
//! Each table carries ONLY the columns something on this side actually reads
//! — the geometry columns the worker builds from (so the cross-pinning tests
//! can catch drift) plus the pitch where a thread exists. Columns nobody
//! consumes are not transcribed: an untested number in a table is a liability,
//! not documentation.
//!
//! PROVENANCE:
//!   * ISO 4014 / 4017 — BOLTS `data/hex.blt`, classes `hexbolt2` / `hexscrew2`.
//!   * ISO 4032 — BOLTS `data/nut.blt`, class `hexagonnut1`.
//!   * ISO 7089 — BOLTS `data/washer.blt`, class `plainwasher1`, WITH ONE
//!     CORRECTION (M10 inner Ø, see below).
//!     All four retrieved 2026-08-13 from `github.com/boltsparts/BOLTS_archive`,
//!     LGPL 2.1+, author Johannes Reinhardt — see `THIRD_PARTY_NOTICES`.
//!   * ISO 7380-1 and ISO 7093-1 — **not present in BOLTS at all** (checked:
//!     `hex_socket.blt` has no button-head class; `washer.blt` carries
//!     7089/7090/7091/7092 and no 7093). Hand-transcribed from the standards'
//!     published dimension tables.
//!   * `pitch_mm` is always the ISO 261 coarse-pitch series — a public numeric
//!     standard, hand-transcribed, not BOLTS data.

/// A headed screw whose head is a solid of revolution (ISO 7380-1 button
/// head). Column names follow the standards: `dk` head Ø, `k` head height,
/// `d1` shank/thread nominal Ø.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrewRow {
    pub d1: f64,
    pub dk: f64,
    pub k: f64,
    pub pitch_mm: f64,
}

/// A hex-headed screw (ISO 4014 partially threaded, ISO 4017 fully threaded).
/// `s` is width across flats; `b` is the threaded length from the tip and is
/// `None` for a fully-threaded family — "all of it", not a missing value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HexScrewRow {
    pub d1: f64,
    pub k: f64,
    pub s: f64,
    pub b: Option<f64>,
    pub pitch_mm: f64,
}

/// A hex nut (ISO 4032). `d1` is the thread nominal Ø, `s` width across
/// flats, `m` thickness.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NutRow {
    pub d1: f64,
    pub s: f64,
    pub m: f64,
}

/// A plain washer (ISO 7089 normal series, ISO 7093-1 large series). `d1`
/// inner Ø, `d2` outer Ø, `s` thickness.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WasherRow {
    pub d1: f64,
    pub d2: f64,
    pub s: f64,
}

/// `(thread, d1, dk, k, pitch)` — ISO 7380-1, M3–M12 (the standard defines no
/// M2/M2.5 in this series, so there is no row for them).
const ISO7380_ROWS: &[(&str, f64, f64, f64, f64)] = &[
    ("M3", 3.0, 5.7, 1.65, 0.50),
    ("M4", 4.0, 7.6, 2.20, 0.70),
    ("M5", 5.0, 9.5, 2.75, 0.80),
    ("M6", 6.0, 10.5, 3.30, 1.00),
    ("M8", 8.0, 14.0, 4.40, 1.25),
    ("M10", 10.0, 17.5, 5.50, 1.50),
    ("M12", 12.0, 21.0, 6.60, 1.75),
];

/// `(thread, d1, k, s, b, pitch)` — ISO 4014, M3–M12. `b` is BOLTS' `b1`, the
/// threaded length for bolts under 125 mm, the only band this seed range
/// reaches.
const ISO4014_ROWS: &[(&str, f64, f64, f64, f64, f64)] = &[
    ("M3", 3.0, 2.0, 5.5, 12.0, 0.50),
    ("M4", 4.0, 2.8, 7.0, 14.0, 0.70),
    ("M5", 5.0, 3.5, 8.0, 16.0, 0.80),
    ("M6", 6.0, 4.0, 10.0, 18.0, 1.00),
    ("M8", 8.0, 5.3, 13.0, 22.0, 1.25),
    ("M10", 10.0, 6.4, 16.0, 26.0, 1.50),
    ("M12", 12.0, 7.5, 18.0, 30.0, 1.75),
];

/// `(thread, d1, k, s, pitch)` — ISO 4017, M2–M12. Fully threaded, so no `b`.
const ISO4017_ROWS: &[(&str, f64, f64, f64, f64)] = &[
    ("M2", 2.0, 1.4, 4.0, 0.40),
    ("M2.5", 2.5, 1.7, 5.0, 0.45),
    ("M3", 3.0, 2.0, 5.5, 0.50),
    ("M4", 4.0, 2.8, 7.0, 0.70),
    ("M5", 5.0, 3.5, 8.0, 0.80),
    ("M6", 6.0, 4.0, 10.0, 1.00),
    ("M8", 8.0, 5.3, 13.0, 1.25),
    ("M10", 10.0, 6.4, 16.0, 1.50),
    ("M12", 12.0, 7.5, 18.0, 1.75),
];

/// `(thread, d1, s, m)` — ISO 4032 style 1, M2–M12. `m` is BOLTS' `m_max`.
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

/// `(thread, d1, d2, s)` — ISO 7089, M2–M12.
///
/// ONE DELIBERATE DEVIATION FROM BOLTS: it lists M10 as inner Ø 10.0. ISO 7089
/// (and DIN 125 A, which BOLTS anchors this table to) both publish **10.5** —
/// a 10.0 bore would not pass an M10 bolt. Corrected here and pinned by a test
/// so a future re-import cannot silently undo it. Every other row is verbatim.
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

/// `(thread, d1, d2, s)` — ISO 7093-1 large series, M3–M12.
const ISO7093_ROWS: &[(&str, f64, f64, f64)] = &[
    ("M3", 3.2, 9.0, 0.8),
    ("M4", 4.3, 12.0, 1.0),
    ("M5", 5.3, 15.0, 1.2),
    ("M6", 6.4, 18.0, 1.6),
    ("M8", 8.4, 24.0, 2.0),
    ("M10", 10.5, 30.0, 2.5),
    ("M12", 13.0, 37.0, 3.0),
];

/// ISO 7380-1 button head socket screw.
pub struct Iso7380Table;

impl Iso7380Table {
    /// The row for `thread`, or `None` for an undeclared size — never a
    /// fabricated fallback (spec §0 invariant #4).
    #[must_use]
    pub fn row(thread: &str) -> Option<ScrewRow> {
        ISO7380_ROWS
            .iter()
            .find(|(key, ..)| *key == thread)
            .map(|(_, d1, dk, k, pitch_mm)| ScrewRow {
                d1: *d1,
                dk: *dk,
                k: *k,
                pitch_mm: *pitch_mm,
            })
    }

    /// Every declared thread designation, in table order — feeds a seed
    /// package's `[parameters].thread.domain`.
    #[must_use]
    pub fn threads() -> Vec<&'static str> {
        ISO7380_ROWS.iter().map(|(key, ..)| *key).collect()
    }
}

/// ISO 4014 hexagon head bolt (partially threaded).
pub struct Iso4014Table;

impl Iso4014Table {
    #[must_use]
    pub fn row(thread: &str) -> Option<HexScrewRow> {
        ISO4014_ROWS
            .iter()
            .find(|(key, ..)| *key == thread)
            .map(|(_, d1, k, s, b, pitch_mm)| HexScrewRow {
                d1: *d1,
                k: *k,
                s: *s,
                b: Some(*b),
                pitch_mm: *pitch_mm,
            })
    }

    #[must_use]
    pub fn threads() -> Vec<&'static str> {
        ISO4014_ROWS.iter().map(|(key, ..)| *key).collect()
    }
}

/// ISO 4017 hexagon head screw (fully threaded).
pub struct Iso4017Table;

impl Iso4017Table {
    #[must_use]
    pub fn row(thread: &str) -> Option<HexScrewRow> {
        ISO4017_ROWS
            .iter()
            .find(|(key, ..)| *key == thread)
            .map(|(_, d1, k, s, pitch_mm)| HexScrewRow {
                d1: *d1,
                k: *k,
                s: *s,
                b: None,
                pitch_mm: *pitch_mm,
            })
    }

    #[must_use]
    pub fn threads() -> Vec<&'static str> {
        ISO4017_ROWS.iter().map(|(key, ..)| *key).collect()
    }
}

/// ISO 4032 hexagon nut.
pub struct Iso4032Table;

impl Iso4032Table {
    #[must_use]
    pub fn row(thread: &str) -> Option<NutRow> {
        ISO4032_ROWS
            .iter()
            .find(|(key, ..)| *key == thread)
            .map(|(_, d1, s, m)| NutRow {
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

/// ISO 7089 plain washer, normal series.
pub struct Iso7089Table;

impl Iso7089Table {
    #[must_use]
    pub fn row(thread: &str) -> Option<WasherRow> {
        washer_row(ISO7089_ROWS, thread)
    }

    #[must_use]
    pub fn threads() -> Vec<&'static str> {
        ISO7089_ROWS.iter().map(|(key, ..)| *key).collect()
    }
}

/// ISO 7093-1 plain washer, large series.
pub struct Iso7093Table;

impl Iso7093Table {
    #[must_use]
    pub fn row(thread: &str) -> Option<WasherRow> {
        washer_row(ISO7093_ROWS, thread)
    }

    #[must_use]
    pub fn threads() -> Vec<&'static str> {
        ISO7093_ROWS.iter().map(|(key, ..)| *key).collect()
    }
}

fn washer_row(rows: &'static [(&'static str, f64, f64, f64)], thread: &str) -> Option<WasherRow> {
    rows.iter()
        .find(|(key, ..)| *key == thread)
        .map(|(_, d1, d2, s)| WasherRow {
            d1: *d1,
            d2: *d2,
            s: *s,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_undeclared_size_is_none_in_every_family() {
        assert!(Iso7380Table::row("M99").is_none());
        assert!(Iso4014Table::row("M99").is_none());
        assert!(Iso4017Table::row("M99").is_none());
        assert!(Iso4032Table::row("M99").is_none());
        assert!(Iso7089Table::row("M99").is_none());
        assert!(Iso7093Table::row("M99").is_none());
    }

    // A family's standard, not the seed range, decides its rows: ISO 7380-1 and
    // ISO 7093-1 start at M3, so they carry no M2. Pinned because "add the
    // missing sizes for consistency" is exactly the tempting, wrong edit.
    #[test]
    fn families_carry_only_the_sizes_their_standard_defines() {
        assert_eq!(
            Iso7380Table::threads(),
            vec!["M3", "M4", "M5", "M6", "M8", "M10", "M12"]
        );
        assert_eq!(
            Iso7093Table::threads(),
            vec!["M3", "M4", "M5", "M6", "M8", "M10", "M12"]
        );
        assert_eq!(
            Iso4017Table::threads(),
            vec!["M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12"]
        );
        assert!(Iso7380Table::row("M2").is_none());
        assert!(Iso7093Table::row("M2").is_none());
    }

    // Spot-checks (spec §6.5): key sizes against the SOURCE values, typed here
    // independently of both this table and the worker's.
    #[test]
    fn spot_check_iso7380_against_the_standard() {
        let m6 = Iso7380Table::row("M6").unwrap();
        assert_eq!((m6.dk, m6.k, m6.d1), (10.5, 3.3, 6.0));
        let m3 = Iso7380Table::row("M3").unwrap();
        assert_eq!((m3.dk, m3.k), (5.7, 1.65));
        let m12 = Iso7380Table::row("M12").unwrap();
        assert_eq!((m12.dk, m12.k), (21.0, 6.6));
    }

    #[test]
    fn spot_check_iso4014_and_4017_against_bolts() {
        let b6 = Iso4014Table::row("M6").unwrap();
        assert_eq!((b6.d1, b6.k, b6.s, b6.b), (6.0, 4.0, 10.0, Some(18.0)));
        let s6 = Iso4017Table::row("M6").unwrap();
        assert_eq!((s6.d1, s6.k, s6.s, s6.b), (6.0, 4.0, 10.0, None));
        let b12 = Iso4014Table::row("M12").unwrap();
        assert_eq!((b12.k, b12.s, b12.b), (7.5, 18.0, Some(30.0)));
    }

    // The two hex standards share every geometric column at every shared size
    // and differ ONLY in `b`. A divergence means one table was mistyped.
    #[test]
    fn iso4014_and_iso4017_agree_on_head_and_shank() {
        for thread in Iso4014Table::threads() {
            let bolt = Iso4014Table::row(thread).unwrap();
            let screw = Iso4017Table::row(thread).unwrap();
            assert_eq!(
                (bolt.d1, bolt.k, bolt.s, bolt.pitch_mm),
                (screw.d1, screw.k, screw.s, screw.pitch_mm),
                "{thread}: ISO 4014 and ISO 4017 must share head/shank columns"
            );
            assert!(bolt.b.is_some() && screw.b.is_none());
        }
    }

    #[test]
    fn spot_check_iso4032_against_bolts() {
        let m6 = Iso4032Table::row("M6").unwrap();
        assert_eq!((m6.d1, m6.s, m6.m), (6.0, 10.0, 5.2));
        let m10 = Iso4032Table::row("M10").unwrap();
        assert_eq!((m10.s, m10.m), (16.0, 8.4));
    }

    #[test]
    fn spot_check_washers_against_their_sources() {
        let n6 = Iso7089Table::row("M6").unwrap();
        assert_eq!((n6.d1, n6.d2, n6.s), (6.4, 12.0, 1.6));
        let l6 = Iso7093Table::row("M6").unwrap();
        assert_eq!((l6.d1, l6.d2, l6.s), (6.4, 18.0, 1.6));
        // The large series' whole point: same bore, bigger disc.
        assert_eq!(n6.d1, l6.d1);
        assert!(l6.d2 > n6.d2);
    }

    // BOLTS lists ISO 7089 M10 as a 10.0 bore; ISO 7089 and DIN 125 A both
    // publish 10.5, and a 10.0 bore would not pass an M10 bolt. The correction
    // is pinned on BOTH sides (the worker has the same test).
    #[test]
    fn iso7089_m10_bore_is_corrected_away_from_the_bolts_slip() {
        assert_eq!(Iso7089Table::row("M10").unwrap().d1, 10.5);
    }

    // Pins every geometry-relevant column against the worker's own table
    // (`worker/src/ops/FastenerTables.cpp`), size by size — the same
    // cross-pinning `Iso4762Table` has. Values here are the WORKER's, typed
    // out; if either copy is edited alone this fails.
    #[test]
    fn geometry_columns_agree_with_the_worker_tables() {
        // (thread, dk, k, d1, pitch)
        for (thread, dk, k, d1, pitch) in [
            ("M3", 5.7, 1.65, 3.0, 0.50),
            ("M4", 7.6, 2.20, 4.0, 0.70),
            ("M5", 9.5, 2.75, 5.0, 0.80),
            ("M6", 10.5, 3.30, 6.0, 1.00),
            ("M8", 14.0, 4.40, 8.0, 1.25),
            ("M10", 17.5, 5.50, 10.0, 1.50),
            ("M12", 21.0, 6.60, 12.0, 1.75),
        ] {
            let row = Iso7380Table::row(thread).unwrap();
            assert_eq!((row.dk, row.k, row.d1, row.pitch_mm), (dk, k, d1, pitch));
        }
        // (thread, d1, k, s, b, pitch)
        for (thread, d1, k, s, b, pitch) in [
            ("M3", 3.0, 2.0, 5.5, 12.0, 0.50),
            ("M6", 6.0, 4.0, 10.0, 18.0, 1.00),
            ("M12", 12.0, 7.5, 18.0, 30.0, 1.75),
        ] {
            let row = Iso4014Table::row(thread).unwrap();
            assert_eq!(
                (row.d1, row.k, row.s, row.b, row.pitch_mm),
                (d1, k, s, Some(b), pitch)
            );
        }
        // (thread, d1, s, m)
        for (thread, d1, s, m) in [("M2", 2.0, 4.0, 1.6), ("M12", 12.0, 18.0, 10.8)] {
            let row = Iso4032Table::row(thread).unwrap();
            assert_eq!((row.d1, row.s, row.m), (d1, s, m));
        }
        // (thread, d1, d2, s) — normal then large series
        for (thread, d1, d2, s) in [("M2", 2.2, 5.0, 0.3), ("M12", 13.0, 24.0, 2.5)] {
            let row = Iso7089Table::row(thread).unwrap();
            assert_eq!((row.d1, row.d2, row.s), (d1, d2, s));
        }
        for (thread, d1, d2, s) in [("M3", 3.2, 9.0, 0.8), ("M12", 13.0, 37.0, 3.0)] {
            let row = Iso7093Table::row(thread).unwrap();
            assert_eq!((row.d1, row.d2, row.s), (d1, d2, s));
        }
    }
}
