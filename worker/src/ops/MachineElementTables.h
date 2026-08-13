// MachineElementTables.h — dimension tables for the built-in NON-fastener
// component generators: ISO 15 deep-groove ball bearings and NEMA stepper
// motors (Component Library spec §6.2, WP-F2).
//
// Separate from `FastenerTables.h` on purpose: neither family is a fastener,
// neither is keyed by a thread designation, and neither comes from the BOLTS
// dataset. The DATA ONLY rule is the same one — every geometry decision lives
// in `ComponentGenerators.cpp`, so a table edit can never change HOW a family
// is built (spec §6: "separate dimension data (tables) from geometry
// generators (code)").
//
// PROVENANCE: both tables are SELF-AUTHORED from public standard dimensions,
// which spec §6.3 explicitly calls for ("author bearing/motor tables yourself
// — they're trivial"). Nothing here is copied from a dataset, so there is no
// `THIRD_PARTY_NOTICES` entry to make. Per-table sources are recorded at the
// definitions in `MachineElementTables.cpp`.
#ifndef ONECAD_OPS_MACHINEELEMENTTABLES_H
#define ONECAD_OPS_MACHINEELEMENTTABLES_H

#include <map>
#include <string>

namespace onecad::ops::machine_elements {

/// One deep-groove ball bearing, keyed by its BEARING CODE (`"608"`), which
/// is what a catalog, a drawing, and a user all name it by. ISO 15 calls
/// these the boundary dimensions `d` / `D` / `B`, and they are the only
/// dimensions placement needs (spec §6.2: "only bore/OD/width matter").
struct BearingSize {
    double bore_diameter_mm;
    double outer_diameter_mm;
    double width_mm;
};

/// One NEMA stepper motor frame. Everything here is FIXED by the frame size
/// except the body length, which is the free per-instance value (spec §6.2) —
/// `default_body_length_mm` is what an instance that names no length gets,
/// and `min_body_length_mm` is the floor below which the generator refuses
/// rather than building a motor that does not exist.
///
/// `mounting_hole_pitch_mm` is the SQUARE pattern's side (hole centres sit at
/// ±pitch/2 in both axes), never a bolt-circle diameter.
struct MotorSize {
    double frame_square_mm;
    double mounting_hole_diameter_mm;
    double mounting_hole_pitch_mm;
    double mounting_hole_depth_mm;
    double pilot_diameter_mm;
    double pilot_height_mm;
    double shaft_diameter_mm;
    double shaft_length_mm;
    double default_body_length_mm;
    double min_body_length_mm;
};

const std::map<std::string, BearingSize>& iso15_table();

/// Keyed by the FRAME NUMBER (`"17"`, `"23"`) — the generator ids are
/// `nema17` / `nema23`, one per family in the house style, and each looks up
/// its own row here.
const std::map<std::string, MotorSize>& nema_table();

/// `"6000, 6001, …"` for the named table — the loud-failure message a
/// generator emits on an unknown key (spec §0 invariant 4: a missing size is
/// an error, never a silent substitution). Mirrors
/// `fasteners::known_threads`, kept separate because these tables are not
/// keyed by a thread and share no vocabulary with that header.
template <typename Row>
std::string known_keys(const std::map<std::string, Row>& table) {
    std::string out;
    for (const auto& entry : table) {
        if (!out.empty()) out += ", ";
        out += entry.first;
    }
    return out;
}

}  // namespace onecad::ops::machine_elements

#endif  // ONECAD_OPS_MACHINEELEMENTTABLES_H
