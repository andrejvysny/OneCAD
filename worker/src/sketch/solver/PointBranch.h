#pragma once

#include "../SketchTypes.h"

namespace onecad::core::sketch {

bool pointBranchContinues(const Vec2d& first_before, const Vec2d& second_before,
                          const Vec2d& first_after, const Vec2d& second_after);

}  // namespace onecad::core::sketch
