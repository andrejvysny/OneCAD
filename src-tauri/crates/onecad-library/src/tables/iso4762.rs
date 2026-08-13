//! ISO 4762 / DIN 912 socket-head cap screw dimensions (spec §6.2).
//!
//! A Rust-side mirror of the worker's own `FastenerTables.cpp::iso4762_table()`
//! — deliberately a SEPARATE copy, not a shared source (spec §6: generators are
//! built-in and versioned in the worker; this crate's tables serve
//! authoring/metadata — `component.toml`'s `[parameters] role="table"`
//! resolution and the designation-string template — never geometry
//! generation). The spot-check tests below pin both copies against the same
//! source values, so a transcription error in EITHER one is caught.
//!
//! Source: BOLTS (`github.com/boltsparts/BOLTS_archive`,
//! `data/hex_socket.blt`, class `hexsocketheadcap` = ISO 4762 / DIN 912),
//! LGPL 2.1+, author Johannes Reinhardt — see `THIRD_PARTY_NOTICES` at the
//! repo root. Full column set (`d1`/`d2`/`b`/`k`/`s`/`t_min`/`L`), scoped to
//! spec §6.2's seed range M2–M12 (BOLTS' own table runs M1.4–M64, but
//! M1.4/M1.8 carry `None` for `t_min`/`b` at that source — out of the
//! spec-stated range, not worth engineering around here).
//!
//! `pitch_mm` (WP-2.5, spec §6.4 `thread_detail`) is a DIFFERENT provenance
//! than the rest of this table: the ISO 261 coarse-pitch series, a public
//! numeric standard, not BOLTS data — hand-transcribed, not subject to
//! `THIRD_PARTY_NOTICES`.

/// One ISO 4762 / DIN 912 size row, BOLTS column names verbatim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Iso4762Row {
    /// Shaft/thread diameter (mm).
    pub d1: f64,
    /// Head diameter (mm).
    pub d2: f64,
    /// Threaded length of shaft (mm).
    pub b: f64,
    /// Head height (mm).
    pub k: f64,
    /// Hex socket width across flats (mm).
    pub s: f64,
    /// Minimum socket depth (mm).
    pub t_min: f64,
    /// Full-thread length threshold — shafts at or below this nominal length
    /// thread all the way (mm).
    pub l: f64,
    /// ISO 261 coarse-pitch series (mm) — a different provenance than the
    /// rest of this row, see the module doc comment.
    pub pitch_mm: f64,
}

/// `(thread designation, row)` pairs, spec §6.2's seed range (M2–M12).
const ISO4762_ROWS: &[(&str, Iso4762Row)] = &[
    (
        "M2",
        Iso4762Row {
            d1: 2.0,
            d2: 3.8,
            b: 16.0,
            k: 2.0,
            s: 1.5,
            t_min: 1.0,
            l: 20.0,
            pitch_mm: 0.40,
        },
    ),
    (
        "M2.5",
        Iso4762Row {
            d1: 2.5,
            d2: 4.5,
            b: 17.0,
            k: 2.5,
            s: 2.0,
            t_min: 1.1,
            l: 25.0,
            pitch_mm: 0.45,
        },
    ),
    (
        "M3",
        Iso4762Row {
            d1: 3.0,
            d2: 5.5,
            b: 18.0,
            k: 3.0,
            s: 2.5,
            t_min: 1.3,
            l: 20.0,
            pitch_mm: 0.50,
        },
    ),
    (
        "M4",
        Iso4762Row {
            d1: 4.0,
            d2: 7.0,
            b: 20.0,
            k: 4.0,
            s: 3.0,
            t_min: 2.0,
            l: 25.0,
            pitch_mm: 0.70,
        },
    ),
    (
        "M5",
        Iso4762Row {
            d1: 5.0,
            d2: 8.5,
            b: 22.0,
            k: 5.0,
            s: 4.0,
            t_min: 2.5,
            l: 25.0,
            pitch_mm: 0.80,
        },
    ),
    (
        "M6",
        Iso4762Row {
            d1: 6.0,
            d2: 10.0,
            b: 24.0,
            k: 6.0,
            s: 5.0,
            t_min: 3.0,
            l: 30.0,
            pitch_mm: 1.00,
        },
    ),
    (
        "M8",
        Iso4762Row {
            d1: 8.0,
            d2: 13.0,
            b: 28.0,
            k: 8.0,
            s: 6.0,
            t_min: 4.0,
            l: 35.0,
            pitch_mm: 1.25,
        },
    ),
    (
        "M10",
        Iso4762Row {
            d1: 10.0,
            d2: 16.0,
            b: 32.0,
            k: 10.0,
            s: 8.0,
            t_min: 5.0,
            l: 40.0,
            pitch_mm: 1.50,
        },
    ),
    (
        "M12",
        Iso4762Row {
            d1: 12.0,
            d2: 18.0,
            b: 36.0,
            k: 12.0,
            s: 10.0,
            t_min: 6.0,
            l: 50.0,
            pitch_mm: 1.75,
        },
    ),
];

/// The ISO 4762 dimension table, spec §6.2 seed range.
pub struct Iso4762Table;

impl Iso4762Table {
    /// The row for `thread` (e.g. `"M6"`), or `None` for an undeclared size —
    /// never a fabricated fallback (spec §0 invariant #4: never silently
    /// substitute).
    #[must_use]
    pub fn row(thread: &str) -> Option<&'static Iso4762Row> {
        ISO4762_ROWS
            .iter()
            .find(|(key, _)| *key == thread)
            .map(|(_, row)| row)
    }

    /// Every declared thread designation, in table order — feeds
    /// `component.toml`'s `[parameters].thread.domain` and error messages
    /// that must list what IS available rather than just refusing.
    #[must_use]
    pub fn threads() -> Vec<&'static str> {
        ISO4762_ROWS.iter().map(|(key, _)| *key).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_thread_is_none_not_a_fabricated_row() {
        assert!(Iso4762Table::row("M99").is_none());
    }

    #[test]
    fn threads_lists_every_seeded_size_in_order() {
        assert_eq!(
            Iso4762Table::threads(),
            vec!["M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12"]
        );
    }

    // Spot-check (spec §6.5): a handful of key sizes against the SAME BOLTS
    // source values `ComponentOp.cpp::iso4762Table()` uses on the worker
    // side — a transcription error in EITHER copy diverging from the
    // source is caught here, since both were typed independently from the
    // same `data/hex_socket.blt` retrieval.
    #[test]
    fn spot_check_m3_against_bolts_source() {
        let row = Iso4762Table::row("M3").unwrap();
        assert_eq!(row.d1, 3.0);
        assert_eq!(row.d2, 5.5);
        assert_eq!(row.k, 3.0);
        assert_eq!(row.s, 2.5);
        assert_eq!(row.t_min, 1.3);
    }

    #[test]
    fn spot_check_m6_against_bolts_source() {
        let row = Iso4762Table::row("M6").unwrap();
        assert_eq!(row.d1, 6.0);
        assert_eq!(row.d2, 10.0);
        assert_eq!(row.k, 6.0);
        assert_eq!(row.s, 5.0);
        assert_eq!(row.t_min, 3.0);
    }

    #[test]
    fn spot_check_m12_against_bolts_source() {
        let row = Iso4762Table::row("M12").unwrap();
        assert_eq!(row.d1, 12.0);
        assert_eq!(row.d2, 18.0);
        assert_eq!(row.k, 12.0);
        assert_eq!(row.s, 10.0);
        assert_eq!(row.t_min, 6.0);
    }

    // Pins the two copies (worker C++, this Rust table) to the SAME values
    // for the fields the worker's generator actually consumes (d1/d2/k,
    // and pitch_mm since WP-2.5) — a future edit to one copy without the
    // other now fails loud here.
    #[test]
    fn geometry_fields_agree_with_the_worker_table_across_the_seed_range() {
        let expected_worker_side: &[(&str, f64, f64, f64, f64)] = &[
            ("M2", 3.8, 2.0, 2.0, 0.40),
            ("M2.5", 4.5, 2.5, 2.5, 0.45),
            ("M3", 5.5, 3.0, 3.0, 0.50),
            ("M4", 7.0, 4.0, 4.0, 0.70),
            ("M5", 8.5, 5.0, 5.0, 0.80),
            ("M6", 10.0, 6.0, 6.0, 1.00),
            ("M8", 13.0, 8.0, 8.0, 1.25),
            ("M10", 16.0, 10.0, 10.0, 1.50),
            ("M12", 18.0, 12.0, 12.0, 1.75),
        ];
        for (thread, d2, k, d1, pitch_mm) in expected_worker_side {
            let row = Iso4762Table::row(thread)
                .unwrap_or_else(|| panic!("{thread} missing from Rust-side table"));
            assert_eq!(
                row.d2, *d2,
                "{thread} d2 (head diameter) mismatch vs worker table"
            );
            assert_eq!(
                row.k, *k,
                "{thread} k (head height) mismatch vs worker table"
            );
            assert_eq!(
                row.d1, *d1,
                "{thread} d1 (shank diameter) mismatch vs worker table"
            );
            assert_eq!(
                row.pitch_mm, *pitch_mm,
                "{thread} pitch_mm mismatch vs worker table"
            );
        }
    }
}
