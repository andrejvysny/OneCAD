// A machine-routable reason for a loop-detection refusal, carried beside the
// human sentence. SCHEMA §7.4 `SketchRegions` `diagnostics`.
#pragma once

#include "../sketch/SketchTypes.h"

#include <cstdint>
#include <string>
#include <vector>

namespace onecad::core::loop {

namespace sk = onecad::core::sketch;

/**
 * @brief Closed reason-code set for the loop-detector refusal surface.
 *
 * `None` means this producer made no claim — the field is then absent on the
 * wire and a consumer may infer nothing from its absence (SCHEMA §7.2).
 * Cancellation deliberately has no member: it is unreachable for the solver
 * lane and the Rust `Cancelled` error has nowhere to carry a diagnostic.
 */
enum class ProfileRefusalReason {
    None,
    OverlappingCurves,
    Discontinuous,
    LimitExceeded,
    RefinementFailed,
    GraphBuildFailed,
};

/// SCREAMING_SNAKE wire token, or nullptr for `None`.
inline const char* profileRefusalReasonCode(ProfileRefusalReason reason) {
    switch (reason) {
        case ProfileRefusalReason::OverlappingCurves:
            return "SKETCH_PROFILE_OVERLAPPING_CURVES";
        case ProfileRefusalReason::Discontinuous:
            return "SKETCH_PROFILE_DISCONTINUOUS";
        case ProfileRefusalReason::LimitExceeded:
            return "SKETCH_PROFILE_LIMIT_EXCEEDED";
        case ProfileRefusalReason::RefinementFailed:
            return "SKETCH_PROFILE_REFINEMENT_FAILED";
        case ProfileRefusalReason::GraphBuildFailed:
            return "SKETCH_PROFILE_GRAPH_BUILD_FAILED";
        case ProfileRefusalReason::None:
            break;
    }
    return nullptr;
}

/**
 * @brief Structured evidence for one refusal. Never widens or narrows what the
 * detector accepts; it only says which entities are at fault.
 */
struct ProfileRefusal {
    ProfileRefusalReason reason = ProfileRefusalReason::None;

    /// Base entity ids at fault, ordered ascending by analytic source index.
    /// The id SPACE depends on who holds the struct: INTERNAL while the
    /// detector owns it, WIRE once `buildRegionTable` has remapped it. An id
    /// that cannot be remapped drops the whole vector rather than publishing an
    /// id no consumer can resolve.
    std::vector<sk::EntityID> entityIds;

    /// `LimitExceeded` evidence. `measured` is the count that broke the ceiling.
    std::string limitName;
    std::uint64_t limit = 0;
    std::uint64_t measured = 0;

    bool has() const { return reason != ProfileRefusalReason::None; }
    bool hasLimit() const { return !limitName.empty(); }
};

}  // namespace onecad::core::loop
