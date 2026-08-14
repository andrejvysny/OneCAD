// RegionTable.cpp — canonical selectable planar-cell identity and boundaries.
#include "loop/RegionTable.h"

#include <algorithm>
#include <cmath>
#include <cstdint>
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

bool isFragmented(const CurveFragment& fragment) {
    constexpr double kParameterEpsilon = 1e-9;
    return std::abs(fragment.firstParameter - fragment.sourceFirstParameter) >
               kParameterEpsilon ||
           std::abs(fragment.lastParameter - fragment.sourceLastParameter) >
               kParameterEpsilon;
}

bool hasLegacyFragmentSuffix(const sk::EntityID& edgeId) {
    const std::size_t segment = edgeId.find("#seg");
    return segment != std::string::npos && edgeId.find("_p", segment) != std::string::npos;
}

char fragmentKindToken(CurveFragmentKind kind) {
    switch (kind) {
        case CurveFragmentKind::Line: return 'L';
        case CurveFragmentKind::Circle: return 'C';
        case CurveFragmentKind::Arc: return 'A';
        case CurveFragmentKind::Ellipse: return 'E';
    }
    return '?';
}

std::int64_t normalizedParameter(const CurveFragment& fragment, double parameter) {
    constexpr double kIdentityScale = 1'000'000'000.0;
    const double span = fragment.sourceLastParameter - fragment.sourceFirstParameter;
    if (!(span > 0.0) || !std::isfinite(parameter)) return 0;
    return static_cast<std::int64_t>(std::llround(
        (parameter - fragment.sourceFirstParameter) / span * kIdentityScale));
}

bool validV3Fragment(const CurveFragment& fragment) {
    const double sourceSpan =
        fragment.sourceLastParameter - fragment.sourceFirstParameter;
    return !fragment.baseEntityId.empty() && std::isfinite(sourceSpan) &&
           sourceSpan > 0.0 && std::isfinite(fragment.firstParameter) &&
           std::isfinite(fragment.lastParameter) &&
           fragment.lastParameter > fragment.firstParameter &&
           fragment.firstParameter >= fragment.sourceFirstParameter &&
           fragment.lastParameter <= fragment.sourceLastParameter + sourceSpan;
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
    std::reverse(loop.fragments.begin(), loop.fragments.end());
    for (CurveFragment& fragment : loop.fragments) {
        fragment.forward = !fragment.forward;
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

std::vector<std::string> loopTokens(const Loop& loop, const WireEdgeMapper& mapper,
                                    RegionIdentityVersion identityVersion) {
    std::vector<std::string> result;
    result.reserve(loop.wire.edges.size());
    if (identityVersion == RegionIdentityVersion::V3 &&
        loop.fragments.size() != loop.wire.edges.size()) {
        return {};
    }
    for (std::size_t i = 0; i < loop.wire.edges.size(); ++i) {
        const sk::EntityID& edgeId = loop.wire.edges[i];
        const sk::EntityID base = baseEdgeId(edgeId);
        std::string token = mapper(base);
        if (token.empty()) return {};
        if (identityVersion == RegionIdentityVersion::V3) {
            const CurveFragment& fragment = loop.fragments[i];
            if (!validV3Fragment(fragment) || fragment.baseEntityId != base) return {};
            const std::string provenance = mapper(fragment.baseEntityId);
            if (provenance.empty() || provenance != token) return {};
            token = provenance;
            token += ":";
            token += fragmentKindToken(fragment.kind);
            token += "@" + std::to_string(normalizedParameter(fragment, fragment.firstParameter));
            token += "-" + std::to_string(normalizedParameter(fragment, fragment.lastParameter));
        } else if (i < loop.fragments.size() && isFragmented(loop.fragments[i])) {
            const CurveFragment& fragment = loop.fragments[i];
            token += ":";
            token += fragmentKindToken(fragment.kind);
            token += "@" + std::to_string(normalizedParameter(fragment, fragment.firstParameter));
            token += "-" + std::to_string(normalizedParameter(fragment, fragment.lastParameter));
        } else if (loop.fragments.empty() && hasLegacyFragmentSuffix(edgeId)) {
            // Pre-P2 in-memory callers only carried synthetic split ids. Keep
            // their v1 canonical form for replay; live P2 loops carry analytic
            // parameters and never enter this branch.
            token += edgeId.substr(edgeId.find("#seg"));
        }
        const bool forward = i < loop.wire.forward.size() ? loop.wire.forward[i] : true;
        token += forward ? ":f" : ":r";
        if (result.empty() || result.back() != token) result.push_back(std::move(token));
    }
    if (result.size() > 1 && result.front() == result.back()) result.pop_back();
    normalizeCycle(result);
    return result;
}

std::string loopSignature(const Loop& loop, const WireEdgeMapper& mapper,
                          RegionIdentityVersion identityVersion) {
    const std::vector<std::string> tokens = loopTokens(loop, mapper, identityVersion);
    std::string result;
    for (const std::string& token : tokens) {
        result += std::to_string(token.size()) + ":" + token + ";";
    }
    return result;
}

bool usesFragments(const Loop& loop) {
    return std::any_of(loop.fragments.begin(), loop.fragments.end(), isFragmented) ||
           (loop.fragments.empty() && std::any_of(loop.wire.edges.begin(), loop.wire.edges.end(),
                                                  hasLegacyFragmentSuffix));
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

std::string materialSignature(const RegionDefinition& region, const WireEdgeMapper& mapper,
                              RegionIdentityVersion identityVersion) {
    const std::string outer = loopSignature(region.outerLoop, mapper, identityVersion);
    if (outer.empty()) return {};
    std::vector<std::string> holes;
    holes.reserve(region.holes.size());
    for (const Loop& hole : region.holes) {
        std::string signature = loopSignature(hole, mapper, identityVersion);
        if (signature.empty()) return {};
        holes.push_back(std::move(signature));
    }
    std::sort(holes.begin(), holes.end());
    std::string result = identityVersion == RegionIdentityVersion::V3
                             ? "cell-v3|outer{" + outer + "}|holes{"
                             : "cell-v2|outer{" + outer + "}|holes{";
    for (const std::string& hole : holes) {
        result += std::to_string(hole.size()) + ":" + hole + ";";
    }
    return result + "}";
}

bool assignIdentity(RegionDefinition& region, const WireEdgeMapper& mapper,
                    RegionIdentityVersion identityVersion, std::string& error) {
    region.legacyId = onecad::region::derive_region_id(
        region.outerWireEdges, onecad::region::Winding::Ccw);
    if (identityVersion == RegionIdentityVersion::V2 &&
        region.holes.empty() && !usesFragments(region.outerLoop)) {
        region.id = region.legacyId;
        return true;
    }
    const std::string signature = materialSignature(region, mapper, identityVersion);
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
                             double tolerance,
                             RegionIdentityVersion identityVersion) {
    if (!result.success) {
        return failed(result.errorMessage.empty() ? "loop detection failed"
                                                   : result.errorMessage);
    }
    if (!mapBaseEdge) return failed("region table requires an edge-id mapper");

    RegionTable table;
    table.regions = buildRegionDefinitions(result, tolerance);
    std::unordered_set<std::string> uniqueIds;
    for (RegionDefinition& region : table.regions) {
        orientLoop(region.outerLoop, true);
        for (Loop& hole : region.holes) orientLoop(hole, false);
        std::string error;
        if (!populateWireEdges(region, mapBaseEdge, error) ||
            !assignIdentity(region, mapBaseEdge, identityVersion, error)) {
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
