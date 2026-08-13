// FastenerTables.h — dimension tables for the built-in fastener generators
// (Component Library spec §6.1-§6.3, WP-A1).
//
// DATA ONLY. Every geometry decision lives in `ComponentGenerators.cpp`, so a
// table edit can never change HOW a family is built and a geometry change can
// never silently redefine a dimension. This mirrors spec §6's "separate
// dimension data (tables) from geometry generators (code)".
//
// Each table is keyed by the THREAD DESIGNATION, never a raw diameter (spec
// §6.1: an "M6 washer" has a 6.4 mm hole). Provenance for every table is
// recorded at its definition in `FastenerTables.cpp`; the BOLTS-sourced ones
// are credited in the repo-root `THIRD_PARTY_NOTICES`.
//
// ONE table, because the seed catalog is one family. `known_threads` and the
// `std::map` keying stay generic rather than collapsing to this row type — the
// loud-failure message is the shape every added family needs on day one.
//
// `onecad-library::tables` (Rust) carries an authoring/metadata mirror of the
// same data with the fuller column set. That copy is NEVER a geometry
// authority — this one is (spec §6: "generators are built-in and versioned"
// in the worker) — and a cross-pinning test in that crate fails loudly if the
// two drift.
#ifndef ONECAD_OPS_FASTENERTABLES_H
#define ONECAD_OPS_FASTENERTABLES_H

#include <map>
#include <string>

namespace onecad::ops::fasteners {

/// A headed screw whose head is a solid of revolution about the shank axis
/// (ISO 4762 socket cap). `head_diameter_mm` is the standard's `dk`,
/// `head_height_mm` its `k`, `shank_diameter_mm` its `d` (the thread's nominal
/// major Ø — what a cosmetic thread renders).
struct ScrewSize {
    double head_diameter_mm;
    double head_height_mm;
    double shank_diameter_mm;
    double pitch_mm;
};

const std::map<std::string, ScrewSize>& iso4762_table();

/// `"M2, M2.5, M3, …"` for the named table — the loud-failure message every
/// generator emits on an unknown thread designation (spec §0 invariant 4: a
/// missing size is an error, never a silent substitution).
template <typename Row>
std::string known_threads(const std::map<std::string, Row>& table) {
    std::string out;
    for (const auto& entry : table) {
        if (!out.empty()) out += ", ";
        out += entry.first;
    }
    return out;
}

}  // namespace onecad::ops::fasteners

#endif  // ONECAD_OPS_FASTENERTABLES_H
