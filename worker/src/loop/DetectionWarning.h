// A non-fatal finding from loop detection, shared by the graph and the result.
#pragma once

#include "../sketch/SketchTypes.h"

#include <string>

namespace onecad::core::loop {

namespace sk = onecad::core::sketch;

/**
 * @brief One entity was dropped; the rest of the graph was still detected.
 *
 * The message is RENDERED from `entityId` rather than stored, so a caller that
 * remaps the id into its own id space (a wire uuid, say) re-renders correctly.
 */
struct DetectionWarning {
    /// Base sketch entity that was dropped.
    sk::EntityID entityId;

    /// Why it was dropped, e.g. "zero-length line".
    std::string reason;

    std::string text() const {
        return "sketch entity " + entityId + " is degenerate (" + reason +
               ") and was ignored by region detection";
    }
};

}  // namespace onecad::core::loop
