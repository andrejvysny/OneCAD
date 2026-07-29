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

// `mapBaseEdge` maps a LoopDetector base entity id to its wire entity id.
// Simple unsplit, hole-free cells retain the legacy FNV identity. Material
// boundaries with holes or intersection fragments use a complete cell
// signature, while keeping the same r_<16hex> wire shape.
RegionTable buildRegionTable(const LoopDetectionResult& result,
                             const WireEdgeMapper& mapBaseEdge,
                             double tolerance);

}  // namespace onecad::core::loop
