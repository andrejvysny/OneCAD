#include "PointBranch.h"

#include "../ExactTruth.h"

#include <algorithm>
#include <cmath>

namespace onecad::core::sketch {
namespace {
double cross(const Vec2d& a, const Vec2d& b) { return a.x * b.y - a.y * b.x; }
double length(const Vec2d& a) { return std::hypot(a.x, a.y); }
}  // namespace

bool pointBranchContinues(const Vec2d& first_before, const Vec2d& second_before,
                          const Vec2d& first_after, const Vec2d& second_after) {
    const double initial = cross(first_before, second_before);
    const double span = std::max({length(first_before), length(second_before),
                                  length(first_after), length(second_after)});
    if (!std::isfinite(initial) || !std::isfinite(span) || span == 0) return false;
    const double sign = initial < 0 ? -1.0 : 1.0;
    const Vec2d first_delta{first_after.x-first_before.x, first_after.y-first_before.y};
    const Vec2d second_delta{second_after.x-second_before.x, second_after.y-second_before.y};
    const double quadratic = sign * cross(first_delta, second_delta);
    const double linear = sign * (cross(first_delta, second_before) + cross(first_before, second_delta));
    const double final = cross(first_after, second_after);
    if (!std::isfinite(quadratic) || !std::isfinite(linear) || !std::isfinite(final)) return false;
    double minimum = std::min(sign * initial, sign * final);
    if (quadratic > 0) {
        const double stationary = (-0.5 * linear) / quadratic;
        if (stationary > 0 && stationary < 1) {
            const double interior = sign * initial + linear * stationary + quadratic * stationary * stationary;
            if (!std::isfinite(interior)) return false;
            minimum = std::min(minimum, interior);
        }
    }
    return std::isfinite(minimum) && minimum > kExactLengthToleranceMm * span;
}

}  // namespace onecad::core::sketch
