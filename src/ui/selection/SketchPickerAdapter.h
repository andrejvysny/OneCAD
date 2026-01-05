#ifndef ONECAD_UI_SELECTION_SKETCHPICKERADAPTER_H
#define ONECAD_UI_SELECTION_SKETCHPICKERADAPTER_H

#include "../../app/selection/SelectionTypes.h"
#include "../../core/sketch/SketchTypes.h"

#include <string_view>

namespace onecad::core::sketch {
class Sketch;
class SketchRenderer;
}

namespace onecad::ui::selection {

class SketchPickerAdapter {
public:
    struct Options {
        bool allowConstraints = true;
        bool allowRegions = true;
    };

    app::selection::PickResult pick(const core::sketch::SketchRenderer& renderer,
                                    const core::sketch::Sketch& sketch,
                                    const core::sketch::Vec2d& sketchPos,
                                    std::string_view sketchId,
                                    double pixelScale,
                                    double tolerancePixels,
                                    Options options) const;
};

} // namespace onecad::ui::selection

#endif // ONECAD_UI_SELECTION_SKETCHPICKERADAPTER_H
