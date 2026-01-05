/**
 * @file RegionUtils.h
 * @brief Shared utilities for sketch region identification.
 *
 * Provides stable region IDs derived from loop edge sets and helpers to
 * build region definitions (outer loop + holes) from loop detection results.
 */
#ifndef ONECAD_CORE_LOOP_REGIONUTILS_H
#define ONECAD_CORE_LOOP_REGIONUTILS_H

#include "LoopDetector.h"

#include <optional>
#include <string>
#include <vector>

namespace onecad::core::loop {

/**
 * @brief Region definition derived from loop detection.
 */
struct RegionDefinition {
    std::string id;
    Loop outerLoop;
    std::vector<Loop> holes;
};

/**
 * @brief Stable region key based on loop edge IDs.
 */
std::string regionKey(const Loop& loop);

/**
 * @brief Build region definitions (outer + holes) from loop detection result.
 */
std::vector<RegionDefinition> buildRegionDefinitions(const LoopDetectionResult& result,
                                                     double tolerance);

/**
 * @brief Find a region definition by region ID.
 */
std::optional<RegionDefinition> findRegionDefinition(const LoopDetectionResult& result,
                                                     const std::string& regionId,
                                                     double tolerance);

/**
 * @brief Default loop detector configuration for region selection.
 */
LoopDetectorConfig makeRegionDetectionConfig();

/**
 * @brief Resolve a sketch region ID into a loop::Face (outer + holes).
 */
std::optional<Face> resolveRegionFace(const sk::Sketch& sketch,
                                      const std::string& regionId);

/**
 * @brief Resolve a sketch region ID using a custom loop detector config.
 */
std::optional<Face> resolveRegionFace(const sk::Sketch& sketch,
                                      const std::string& regionId,
                                      const LoopDetectorConfig& config);

} // namespace onecad::core::loop

#endif // ONECAD_CORE_LOOP_REGIONUTILS_H
