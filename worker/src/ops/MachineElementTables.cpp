// MachineElementTables.cpp — see MachineElementTables.h.
#include "ops/MachineElementTables.h"

namespace onecad::ops::machine_elements {

// Deep-groove ball bearings, ISO 15 boundary dimensions (mm): bore `d`,
// outside Ø `D`, width `B`. SELF-AUTHORED from the published ISO 15 boundary
// series — the eight codes a hobby/machined-part catalog actually reaches for
// (the 600/620/62/68 series plus the 608 every 3D printer carries). These are
// the *boundary* dimensions only: chamfer radii, race geometry and ball data
// are not tabulated because the generator does not model them (spec §6.2:
// "stepped-ring geometry suffices").
//
// A code outside this list is refused with the known codes, never rounded to
// a neighbour — two bearings one code apart differ by millimetres of bore.
const std::map<std::string, BearingSize>& iso15_table() {
    static const std::map<std::string, BearingSize> table = {
        {"6000", {10.0, 26.0, 8.0}},  {"6001", {12.0, 28.0, 8.0}},
        {"6200", {10.0, 30.0, 9.0}},  {"6201", {12.0, 32.0, 10.0}},
        {"6202", {15.0, 35.0, 11.0}}, {"608", {8.0, 22.0, 7.0}},
        {"625", {5.0, 16.0, 5.0}},    {"6802", {15.0, 24.0, 5.0}},
    };
    return table;
}

// NEMA stepper motor frames (mm). SELF-AUTHORED from the NEMA ICS 16 frame
// dimensions every stepper vendor publishes identically:
//
//   * NEMA 17 — 42.3 mm frame, M3 mounting holes on a 31.0 mm square,
//     Ø22.0 × 2.0 pilot boss, Ø5.0 × 24 mm shaft.
//   * NEMA 23 — 56.4 mm frame, Ø5.1 CLEARANCE holes (for M5) on a 47.14 mm
//     square, Ø38.1 × 1.6 pilot boss, Ø6.35 × 21 mm shaft.
//
// Mounting-hole DEPTH is the one column the frame standard does not fix (it
// depends on the end-bell casting): 4.5 / 6.0 mm are conservative values that
// clear any real tapped depth, and they exist here only so the blind holes
// this generator drills are table data like everything else rather than a
// constant buried in the geometry code.
//
// Body length is the free per-instance value (spec §6.2). The default is the
// commonest stocked length for the frame; the minimum is the point below
// which no such motor is built.
const std::map<std::string, MotorSize>& nema_table() {
    static const std::map<std::string, MotorSize> table = {
        {"17", {42.3, 3.0, 31.0, 4.5, 22.0, 2.0, 5.0, 24.0, 40.0, 20.0}},
        {"23", {56.4, 5.1, 47.14, 6.0, 38.1, 1.6, 6.35, 21.0, 40.0, 20.0}},
    };
    return table;
}

}  // namespace onecad::ops::machine_elements
