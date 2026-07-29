// RegionTable.cpp — canonical selectable planar-cell identity and boundaries.
#include "loop/RegionTable.h"

#include <algorithm>
#include <charconv>
#include <unordered_set>

#include "sketch/RegionId.h"
#include "sketch/SketchTypes.h"

namespace onecad::core::loop {
namespace {

sk::EntityID baseEdgeId(const sk::EntityID& edgeId) {
    const std::size_t split = edgeId.find("#seg");
    return split == std::string::npos ? edgeId : edgeId.substr(0, split);
}

std::vector<std::string> mappedLoopEdges(const Loop& loop,
                                         const WireEdgeMapper& mapper) {
    std::vector<std::string> result;
    result.reserve(loop.wire.edges.size());
    for (const sk::EntityID& edgeId : loop.wire.edges) {
        std::string mapped = mapper(baseEdgeId(edgeId));
        if (!mapped.empty() && (result.empty() || result.back() != mapped)) {
            result.push_back(std::move(mapped));
        }
    }
    return result;
}

bool isSplitFragment(const sk::EntityID& edgeId) {
    const std::size_t segment = edgeId.find("#seg");
    const std::size_t part =
        segment == std::string::npos ? segment : edgeId.find("_p", segment);
    if (part == std::string::npos || part + 2 >= edgeId.size()) {
        return false;
    }
    unsigned int ordinal = 0;
    const char* begin = edgeId.data() + part + 2;
    const char* end = edgeId.data() + edgeId.size();
    const auto parsed = std::from_chars(begin, end, ordinal);
    return parsed.ec == std::errc{} && parsed.ptr == end && ordinal > 0;
}

void collectFragmentedBases(const Loop& loop,
                            std::unordered_set<sk::EntityID>& result) {
    for (const sk::EntityID& edgeId : loop.wire.edges) {
        if (isSplitFragment(edgeId)) {
            result.insert(baseEdgeId(edgeId));
        }
    }
}

std::unordered_set<sk::EntityID> fragmentedBases(
    const std::vector<RegionDefinition>& regions) {
    std::unordered_set<sk::EntityID> result;
    for (const RegionDefinition& region : regions) {
        collectFragmentedBases(region.outerLoop, result);
        for (const Loop& hole : region.holes) {
            collectFragmentedBases(hole, result);
        }
    }
    return result;
}

void orientLoop(Loop& loop, bool ccw) {
    if (loop.polygon.size() < 3 || loop.isCCW() == ccw) {
        return;
    }
    std::reverse(loop.wire.edges.begin(), loop.wire.edges.end());
    std::reverse(loop.wire.forward.begin(), loop.wire.forward.end());
    for (std::size_t i = 0; i < loop.wire.forward.size(); ++i) {
        loop.wire.forward[i] = !loop.wire.forward[i];
    }
    std::reverse(loop.polygon.begin(), loop.polygon.end());
    loop.signedArea = -loop.signedArea;
}

void normalizeCycle(std::vector<std::string>& tokens) {
    if (tokens.empty()) {
        return;
    }
    const auto lessRotation = [&](std::size_t lhs, std::size_t rhs) {
        for (std::size_t offset = 0; offset < tokens.size(); ++offset) {
            const std::string& a = tokens[(lhs + offset) % tokens.size()];
            const std::string& b = tokens[(rhs + offset) % tokens.size()];
            if (a != b) return a < b;
        }
        return false;
    };
    std::size_t first = 0;
    for (std::size_t i = 1; i < tokens.size(); ++i) {
        if (lessRotation(i, first)) first = i;
    }
    std::rotate(tokens.begin(), tokens.begin() + static_cast<std::ptrdiff_t>(first),
                tokens.end());
}

std::vector<std::string> loopTokens(
    const Loop& loop, const WireEdgeMapper& mapper,
    const std::unordered_set<sk::EntityID>& fragmented) {
    std::vector<std::string> result;
    result.reserve(loop.wire.edges.size());
    for (std::size_t i = 0; i < loop.wire.edges.size(); ++i) {
        const sk::EntityID& edgeId = loop.wire.edges[i];
        const sk::EntityID base = baseEdgeId(edgeId);
        std::string token = mapper(base);
        if (token.empty()) return {};
        if (fragmented.contains(base)) {
            const std::size_t suffix = edgeId.find("#seg");
            if (suffix == std::string::npos) return {};
            token += edgeId.substr(suffix);
        }
        const bool forward = i < loop.wire.forward.size() ? loop.wire.forward[i] : true;
        token += forward ? ":f" : ":r";
        if (result.empty() || result.back() != token) result.push_back(std::move(token));
    }
    if (result.size() > 1 && result.front() == result.back()) result.pop_back();
    normalizeCycle(result);
    return result;
}

std::string loopSignature(
    const Loop& loop, const WireEdgeMapper& mapper,
    const std::unordered_set<sk::EntityID>& fragmented) {
    const std::vector<std::string> tokens = loopTokens(loop, mapper, fragmented);
    std::string result;
    for (const std::string& token : tokens) {
        result += std::to_string(token.size()) + ":" + token + ";";
    }
    return result;
}

bool usesFragments(const Loop& loop,
                   const std::unordered_set<sk::EntityID>& fragmented) {
    return std::any_of(loop.wire.edges.begin(), loop.wire.edges.end(),
                       [&](const sk::EntityID& edgeId) {
                           return fragmented.contains(baseEdgeId(edgeId));
                       });
}

bool populateWireEdges(RegionDefinition& region, const WireEdgeMapper& mapper,
                       std::string& error) {
    region.outerWireEdges = mappedLoopEdges(region.outerLoop, mapper);
    if (region.outerWireEdges.empty()) {
        error = "region outer loop has no wire edge ids";
        return false;
    }
    region.holeWireEdges.clear();
    region.holeWireEdges.reserve(region.holes.size());
    for (const Loop& hole : region.holes) {
        std::vector<std::string> edges = mappedLoopEdges(hole, mapper);
        if (edges.empty()) {
            error = "region hole has no wire edge ids";
            return false;
        }
        region.holeWireEdges.push_back(std::move(edges));
    }
    return true;
}

std::string materialSignature(
    const RegionDefinition& region, const WireEdgeMapper& mapper,
    const std::unordered_set<sk::EntityID>& fragmented) {
    const std::string outer = loopSignature(region.outerLoop, mapper, fragmented);
    if (outer.empty()) return {};
    std::vector<std::string> holes;
    holes.reserve(region.holes.size());
    for (const Loop& hole : region.holes) {
        std::string signature = loopSignature(hole, mapper, fragmented);
        if (signature.empty()) return {};
        holes.push_back(std::move(signature));
    }
    std::sort(holes.begin(), holes.end());
    std::string result = "cell-v2|outer{" + outer + "}|holes{";
    for (const std::string& hole : holes) {
        result += std::to_string(hole.size()) + ":" + hole + ";";
    }
    return result + "}";
}

bool assignIdentity(RegionDefinition& region, const WireEdgeMapper& mapper,
                    const std::unordered_set<sk::EntityID>& fragmented,
                    std::string& error) {
    region.legacyId = onecad::region::derive_region_id(
        region.outerWireEdges, onecad::region::Winding::Ccw);
    if (region.holes.empty() && !usesFragments(region.outerLoop, fragmented)) {
        region.id = region.legacyId;
        return true;
    }
    const std::string signature = materialSignature(region, mapper, fragmented);
    if (signature.empty()) {
        error = "region material boundary has no canonical identity";
        return false;
    }
    region.id = onecad::region::derive_region_id(
        {signature}, onecad::region::Winding::Ccw);
    return true;
}

RegionTable failed(std::string message) {
    RegionTable result;
    result.errorMessage = std::move(message);
    return result;
}

}  // namespace

RegionTable buildRegionTable(const LoopDetectionResult& result,
                             const WireEdgeMapper& mapBaseEdge,
                             double tolerance) {
    if (!result.success) {
        return failed(result.errorMessage.empty() ? "loop detection failed"
                                                   : result.errorMessage);
    }
    if (!mapBaseEdge) return failed("region table requires an edge-id mapper");

    RegionTable table;
    table.regions = buildRegionDefinitions(result, tolerance);
    const auto fragmented = fragmentedBases(table.regions);
    std::unordered_set<std::string> uniqueIds;
    for (RegionDefinition& region : table.regions) {
        orientLoop(region.outerLoop, true);
        for (Loop& hole : region.holes) orientLoop(hole, false);
        std::string error;
        if (!populateWireEdges(region, mapBaseEdge, error) ||
            !assignIdentity(region, mapBaseEdge, fragmented, error)) {
            return failed(std::move(error));
        }
        if (!uniqueIds.insert(region.id).second) {
            return failed("region identity collision after canonical derivation");
        }
    }
    table.success = true;
    return table;
}

}  // namespace onecad::core::loop
