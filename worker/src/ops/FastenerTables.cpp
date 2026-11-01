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
//   * A family the BOLTS dataset does not carry is hand-transcribed from the
//     standard's own published table and says so at its definition — never
//     silently mixed in with the BOLTS-sourced rows.
//
// Seed range is spec §6.2's M2–M12 where the standard defines it; a family
// whose standard starts higher simply carries fewer rows rather than having
// values invented for the gap.
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

// ISO 4032 hexagon nut, style 1 (mm). BOLTS `data/nut.blt`, class
// `hexagonnut1`, retrieved 2026-08-13. Columns used: `d1` (thread nominal Ø,
// which is the bore this cosmetic rendering cuts), `s` (width across flats),
// `m_max` (thickness). Verbatim from the source.
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

}  // namespace onecad::ops::fasteners
