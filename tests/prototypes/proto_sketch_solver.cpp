#include "test_harness/TestHarness.h"
#include "sketch/Sketch.h"
#include "sketch/constraints/Constraints.h"
#include "sketch/solver/ConstraintSolver.h"
#include "sketch/solver/SolverAdapter.h"

#include <cmath>
#include <algorithm>
#include <numbers>

using namespace onecad::core::sketch;
using namespace onecad::core::sketch::constraints;

TEST_CASE(Solver_basic_flow) {
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
    EXPECT_FALSE(distanceId.empty());
    EXPECT_FALSE(radiusId.empty());

    ConstraintSolver solver;
    SolverAdapter::populateSolver(sketch, solver);

    DOFResult dof = solver.calculateDOF();
    int expectedTotal = 10;
    EXPECT_EQ(dof.total, expectedTotal);

    SolveResult solveResult = sketch.solve();
    EXPECT_TRUE(solveResult.success);

    Vec2d target{2.0, 3.0};
    SolveResult dragResult = sketch.solveWithDrag(p1, target);
    EXPECT_TRUE(dragResult.success);

    auto* p1Entity = sketch.getEntityAs<SketchPoint>(p1);
    EXPECT_TRUE(p1Entity != nullptr);
    if (p1Entity) {
        EXPECT_NEAR(p1Entity->x(), target.x, 1e-6);
        EXPECT_NEAR(p1Entity->y(), target.y, 1e-6);
    }
}

int main() {
    return onecad::test::runAllTests();
}
