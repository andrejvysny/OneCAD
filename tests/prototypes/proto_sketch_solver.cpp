#include "sketch/Sketch.h"
#include "sketch/constraints/Constraints.h"
#include "sketch/solver/ConstraintSolver.h"
#include "sketch/solver/SolverAdapter.h"

#include <cassert>
#include <cmath>
#include <algorithm>
#include <iostream>
#include <numbers>

using namespace onecad::core::sketch;
using namespace onecad::core::sketch::constraints;

namespace {

bool approx(double a, double b, double tol = 1e-6) {
    double diff = std::abs(a - b);
    double scale = std::max(std::abs(a), std::abs(b));
    return diff <= tol || diff <= tol * scale;
}

} // namespace

int main() {
    Sketch sketch;

    auto p1 = sketch.addPoint(0.0, 0.0);
    auto p2 = sketch.addPoint(10.0, 0.0);
    auto line = sketch.addLine(p1, p2);
    (void)line;

    auto circleCenter = sketch.addPoint(5.0, 5.0);
    auto circle = sketch.addCircle(circleCenter, 2.5);
    auto arcCenter = sketch.addPoint(-2.0, 2.0);
    auto arc = sketch.addArc(arcCenter, 3.0, 0.0, std::numbers::pi_v<double> * 0.5);
    (void)arc;

    auto distanceId = sketch.addDistance(p1, p2, 10.0);
    auto radiusId = sketch.addRadius(circle, 2.5);
    assert(!distanceId.empty());
    assert(!radiusId.empty());

    ConstraintSolver solver;
    SolverAdapter::populateSolver(sketch, solver);

    DOFResult dof = solver.calculateDOF();
    int expectedTotal = 10;
    assert(dof.total == expectedTotal);

    SolveResult solveResult = sketch.solve();
    assert(solveResult.success);

    Vec2d target{2.0, 3.0};
    SolveResult dragResult = sketch.solveWithDrag(p1, target);
    assert(dragResult.success);

    auto* p1Entity = sketch.getEntityAs<SketchPoint>(p1);
    assert(p1Entity);
    assert(approx(p1Entity->x(), target.x));
    assert(approx(p1Entity->y(), target.y));

    std::cout << "Sketch solver adapter prototype: OK" << std::endl;
    return 0;
}
