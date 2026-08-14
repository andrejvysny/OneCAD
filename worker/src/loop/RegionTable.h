// RegionTable.h — one canonical selectable-cell table shared by publication
// and modeling profile lookup.
#pragma once

#include <functional>
#include <string>
#include <vector>

#include "loop/RegionUtils.h"

namespace onecad::core::loop {

struct RegionTable {
    bool success = false;
    std::string errorMessage;
    std::vector<RegionDefinition> regions;
};

using WireEdgeMapper = std::function<std::string(const sk::EntityID&)>;

enum class RegionIdentityVersion {
    V2,
    V3,
};

// `mapBaseEdge` maps a LoopDetector base entity id to its wire entity id.
// V2 preserves legacy simple-cell bytes. V3 always hashes complete analytic
// provenance and normalized intervals, retaining the r_<16hex> wire shape.
RegionTable buildRegionTable(const LoopDetectionResult& result,
                             const WireEdgeMapper& mapBaseEdge,
                             double tolerance,
                             RegionIdentityVersion identityVersion =
                                 RegionIdentityVersion::V2);

}  // namespace onecad::core::loop
