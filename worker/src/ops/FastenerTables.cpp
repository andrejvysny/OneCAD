// FastenerTables.cpp — see FastenerTables.h.
//
// PROVENANCE, per table. Where a value came from is part of the data: a
// dimension nobody can trace is a dimension nobody can check (spec §6.5).
//
//   * BOLTS-sourced tables carry the file + class they were read from.
//     `github.com/boltsparts/BOLTS_archive`, LGPL 2.1+, author Johannes
//     Reinhardt — credited in the repo-root `THIRD_PARTY_NOTICES`.
//   * `pitch_mm` is ALWAYS the ISO 261 coarse-pitch series — a public numeric
//     standard, hand-transcribed, NOT BOLTS data and not subject to the
//     notices file. (Same call WP-2.1 made for ISO 4762.)
//   * ISO 7380-1 and ISO 7093-1 are NOT in the BOLTS dataset at all (checked:
//     `hex_socket.blt` carries no button-head class, `washer.blt` carries
//     7089/7090/7091/7092 but no 7093). Those two are hand-transcribed from
//     the standards' published dimension tables and say so at their
//     definition — never silently mixed in with the BOLTS-sourced rows.
//
// Seed range is spec §6.2's M2–M12 where the standard defines it; a family
// whose standard starts higher (ISO 7380-1 and ISO 4014 start at M3) simply
// carries fewer rows rather than having values invented for the gap.
#include "ops/FastenerTables.h"

namespace onecad::ops::fasteners {

// ISO 4762 / DIN 912 socket-head cap screw (mm). BOLTS `data/hex_socket.blt`,
// class `hexsocketheadcap`, retrieved 2026-08-12. Columns used: `d1` (shank
// Ø), `d2` (head Ø, the standard's `dk`), `k` (head height). BOLTS' own table
// runs M1.4–M64; M1.4/M1.8 carry `None` for columns a practical hex socket
// needs, so this stops at the spec's stated range rather than engineering
// around incomplete rows outside it.
//
// UNCHANGED from WP-2.1's original in-ComponentOp table — the extraction to
// this file is a move, and ISO 4762's exact-volume ctests are the proof.
const std::map<std::string, ScrewSize>& iso4762_table() {
    static const std::map<std::string, ScrewSize> table = {
        {"M2", {3.8, 2.0, 2.0, 0.40}},    {"M2.5", {4.5, 2.5, 2.5, 0.45}},
        {"M3", {5.5, 3.0, 3.0, 0.50}},    {"M4", {7.0, 4.0, 4.0, 0.70}},
        {"M5", {8.5, 5.0, 5.0, 0.80}},    {"M6", {10.0, 6.0, 6.0, 1.00}},
        {"M8", {13.0, 8.0, 8.0, 1.25}},   {"M10", {16.0, 10.0, 10.0, 1.50}},
        {"M12", {18.0, 12.0, 12.0, 1.75}},
    };
    return table;
}

// ISO 7380-1 button-head socket screw (mm). **NOT BOLTS data** — the BOLTS
// archive has no button-head class (verified against `data/hex_socket.blt`,
// which carries countersunk/cap/set-screw classes only). Hand-transcribed
// from ISO 7380-1's published `dk`/`k` columns. The standard defines M3
// upward, so there is no M2/M2.5 row here; the package's `thread` domain
// declares that range and a request outside it fails loudly.
const std::map<std::string, ScrewSize>& iso7380_table() {
    static const std::map<std::string, ScrewSize> table = {
        {"M3", {5.7, 1.65, 3.0, 0.50}},  {"M4", {7.6, 2.20, 4.0, 0.70}},
        {"M5", {9.5, 2.75, 5.0, 0.80}},  {"M6", {10.5, 3.30, 6.0, 1.00}},
        {"M8", {14.0, 4.40, 8.0, 1.25}}, {"M10", {17.5, 5.50, 10.0, 1.50}},
        {"M12", {21.0, 6.60, 12.0, 1.75}},
    };
    return table;
}

// ISO 4014 hexagon head bolt, PARTIALLY threaded (mm). BOLTS
// `data/hex.blt`, class `hexbolt2`, retrieved 2026-08-13. Columns used:
// `d1` (shank Ø), `k` (head height), `s` (width across flats), `b1` (threaded
// length for l < 125 mm — the only length band this seed range reaches).
const std::map<std::string, HexScrewSize>& iso4014_table() {
    static const std::map<std::string, HexScrewSize> table = {
        {"M3", {3.0, 2.0, 5.5, 12.0, 0.50}},   {"M4", {4.0, 2.8, 7.0, 14.0, 0.70}},
        {"M5", {5.0, 3.5, 8.0, 16.0, 0.80}},   {"M6", {6.0, 4.0, 10.0, 18.0, 1.00}},
        {"M8", {8.0, 5.3, 13.0, 22.0, 1.25}},  {"M10", {10.0, 6.4, 16.0, 26.0, 1.50}},
        {"M12", {12.0, 7.5, 18.0, 30.0, 1.75}},
    };
    return table;
}

// ISO 4017 hexagon head screw, FULLY threaded (mm). BOLTS `data/hex.blt`,
// class `hexscrew2`, retrieved 2026-08-13. Columns used: `d1`, `k`, `s`.
// `thread_length_mm` is 0 = "thread the whole shank", which is what fully
// threaded means — not a missing value.
const std::map<std::string, HexScrewSize>& iso4017_table() {
    static const std::map<std::string, HexScrewSize> table = {
        {"M2", {2.0, 1.4, 4.0, 0.0, 0.40}},   {"M2.5", {2.5, 1.7, 5.0, 0.0, 0.45}},
        {"M3", {3.0, 2.0, 5.5, 0.0, 0.50}},   {"M4", {4.0, 2.8, 7.0, 0.0, 0.70}},
        {"M5", {5.0, 3.5, 8.0, 0.0, 0.80}},   {"M6", {6.0, 4.0, 10.0, 0.0, 1.00}},
        {"M8", {8.0, 5.3, 13.0, 0.0, 1.25}},  {"M10", {10.0, 6.4, 16.0, 0.0, 1.50}},
        {"M12", {12.0, 7.5, 18.0, 0.0, 1.75}},
    };
    return table;
}

// ISO 4032 hexagon nut, style 1 (mm). BOLTS `data/nut.blt`, class
// `hexagonnut1`, retrieved 2026-08-13. Columns used: `d1` (thread nominal Ø,
// which is the bore this cosmetic rendering cuts), `s` (width across flats),
// `m_max` (thickness).
const std::map<std::string, NutSize>& iso4032_table() {
    static const std::map<std::string, NutSize> table = {
        {"M2", {2.0, 4.0, 1.6}},    {"M2.5", {2.5, 5.0, 2.0}},  {"M3", {3.0, 5.5, 2.4}},
        {"M4", {4.0, 7.0, 3.2}},    {"M5", {5.0, 8.0, 4.7}},    {"M6", {6.0, 10.0, 5.2}},
        {"M8", {8.0, 13.0, 6.8}},   {"M10", {10.0, 16.0, 8.4}}, {"M12", {12.0, 18.0, 10.8}},
    };
    return table;
}

// ISO 7089 plain washer, normal series, 200 HV (mm). BOLTS
// `data/washer.blt`, class `plainwasher1`, retrieved 2026-08-13. Columns
// used: `d1` (inner Ø), `d2` (outer Ø), `s` (thickness).
//
// ONE DELIBERATE DEVIATION FROM THE SOURCE: BOLTS lists M10 as inner Ø 10.0.
// ISO 7089 (and DIN 125 A, which BOLTS anchors this table to) both publish
// **10.5** — a washer with a 10.0 bore would not pass an M10 bolt. Taken as a
// slip in the source and corrected here, recorded rather than silently fixed;
// every other row matches BOLTS verbatim.
const std::map<std::string, WasherSize>& iso7089_table() {
    static const std::map<std::string, WasherSize> table = {
        {"M2", {2.2, 5.0, 0.3}},   {"M2.5", {2.7, 6.0, 0.5}}, {"M3", {3.2, 7.0, 0.5}},
        {"M4", {4.3, 9.0, 0.8}},   {"M5", {5.3, 10.0, 1.0}},  {"M6", {6.4, 12.0, 1.6}},
        {"M8", {8.4, 16.0, 1.6}},  {"M10", {10.5, 20.0, 2.0}}, {"M12", {13.0, 24.0, 2.5}},
    };
    return table;
}

// ISO 7093-1 plain washer, LARGE series, 200 HV (mm). **NOT BOLTS data** —
// the archive's `washer.blt` carries 7089/7090/7091/7092 and no 7093 class at
// all. Hand-transcribed from ISO 7093-1's published table. The standard
// defines M3 upward in this series, so there is no M2/M2.5 row.
const std::map<std::string, WasherSize>& iso7093_table() {
    static const std::map<std::string, WasherSize> table = {
        {"M3", {3.2, 9.0, 0.8}},   {"M4", {4.3, 12.0, 1.0}}, {"M5", {5.3, 15.0, 1.2}},
        {"M6", {6.4, 18.0, 1.6}},  {"M8", {8.4, 24.0, 2.0}}, {"M10", {10.5, 30.0, 2.5}},
        {"M12", {13.0, 37.0, 3.0}},
    };
    return table;
}

}  // namespace onecad::ops::fasteners
