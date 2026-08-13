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

}  // namespace onecad::ops::fasteners
