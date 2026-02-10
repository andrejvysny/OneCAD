#include "sketch/Sketch.h"
#include "sketch/constraints/Constraints.h"
#include "sketch/solver/ConstraintSolver.h"
#include "sketch/solver/SolverAdapter.h"
#include "loop/RegionUtils.h"

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

    // Keep target consistent with the fixed-distance constraint to p2.
    Vec2d target{10.0, 10.0};
    SolveResult dragResult = sketch.solveWithDrag(p1, target);
    assert(dragResult.success);

    auto* p1Entity = sketch.getEntityAs<SketchPoint>(p1);
    assert(p1Entity);
    assert(approx(p1Entity->x(), target.x));
    assert(approx(p1Entity->y(), target.y));

    // Rectangle drag regression:
    // dragging one corner should keep the opposite corner fixed.
    Sketch rectangle;
    auto rp1 = rectangle.addPoint(0.0, 0.0);
    auto rp2 = rectangle.addPoint(10.0, 0.0);
    auto rp3 = rectangle.addPoint(10.0, 6.0);
    auto rp4 = rectangle.addPoint(0.0, 6.0);
    assert(!rp1.empty() && !rp2.empty() && !rp3.empty() && !rp4.empty());

    auto bottom = rectangle.addLine(rp1, rp2);
    auto right = rectangle.addLine(rp2, rp3);
    auto top = rectangle.addLine(rp3, rp4);
    auto left = rectangle.addLine(rp4, rp1);
    assert(!bottom.empty() && !right.empty() && !top.empty() && !left.empty());

    assert(!rectangle.addHorizontal(bottom).empty());
    assert(!rectangle.addHorizontal(top).empty());
    assert(!rectangle.addVertical(left).empty());
    assert(!rectangle.addVertical(right).empty());

    auto regionId = onecad::core::loop::getRegionIdContainingEntity(rectangle, rp1);
    assert(regionId.has_value());
    auto face = onecad::core::loop::resolveRegionFace(rectangle, *regionId);
    assert(face.has_value());
    auto boundaryPoints = onecad::core::loop::getOrderedBoundaryPointIds(rectangle, face->outerLoop);
    assert(boundaryPoints.size() == 4);
    assert(std::find(boundaryPoints.begin(), boundaryPoints.end(), rp1) != boundaryPoints.end());

    auto* rp3Before = rectangle.getEntityAs<SketchPoint>(rp3);
    assert(rp3Before);
    double oppositeX = rp3Before->x();
    double oppositeY = rp3Before->y();

    rectangle.beginPointDrag(rp1);
    SolveResult rectangleDrag = rectangle.solveWithDrag(rp1, Vec2d{-2.0, -1.0});
    rectangle.endPointDrag();
    assert(rectangleDrag.success);

    auto* rp1After = rectangle.getEntityAs<SketchPoint>(rp1);
    auto* rp2After = rectangle.getEntityAs<SketchPoint>(rp2);
    auto* rp3After = rectangle.getEntityAs<SketchPoint>(rp3);
    auto* rp4After = rectangle.getEntityAs<SketchPoint>(rp4);
    assert(rp1After && rp2After && rp3After && rp4After);

    assert(approx(rp3After->x(), oppositeX));
    assert(approx(rp3After->y(), oppositeY));
    assert(approx(rp2After->y(), rp1After->y()));
    assert(approx(rp4After->x(), rp1After->x()));
    assert(approx(rp3After->x(), rp2After->x()));
    assert(approx(rp3After->y(), rp4After->y()));

    // Drag rollback determinism regression:
    // if a drag session has at least one failed solve after a successful move,
    // endPointDrag() should rollback to drag-start pose.
    Sketch dragRollback;
    auto d1 = dragRollback.addPoint(0.0, 0.0);
    auto d2 = dragRollback.addPoint(10.0, 0.0);
    auto d3 = dragRollback.addPoint(10.0, 6.0);
    auto d4 = dragRollback.addPoint(0.0, 6.0);
    assert(!d1.empty() && !d2.empty() && !d3.empty() && !d4.empty());
    assert(!dragRollback.addLine(d1, d2).empty());
    assert(!dragRollback.addLine(d2, d3).empty());
    assert(!dragRollback.addLine(d3, d4).empty());
    assert(!dragRollback.addLine(d4, d1).empty());
    assert(!dragRollback.addHorizontal(d1, d2).empty());
    assert(!dragRollback.addHorizontal(d3, d4).empty());
    assert(!dragRollback.addVertical(d2, d3).empty());
    assert(!dragRollback.addVertical(d4, d1).empty());

    auto* d1Start = dragRollback.getEntityAs<SketchPoint>(d1);
    assert(d1Start);
    const double dragStartX = d1Start->x();
    const double dragStartY = d1Start->y();

    dragRollback.beginPointDrag(d1);
    SolveResult moveOk = dragRollback.solveWithDrag(d1, Vec2d{-2.0, -1.0});
    assert(moveOk.success);

    // Inject a hard lock after a successful move so next drag target is unsolved.
    assert(!dragRollback.addFixed(d1).empty());
    SolveResult moveFail = dragRollback.solveWithDrag(d1, Vec2d{-4.0, -3.0});
    assert(!moveFail.success);
    dragRollback.endPointDrag();

    auto* d1Final = dragRollback.getEntityAs<SketchPoint>(d1);
    assert(d1Final);
    assert(approx(d1Final->x(), dragStartX));
    assert(approx(d1Final->y(), dragStartY));

    std::cout << "Sketch solver adapter prototype: OK" << std::endl;
    return 0;
}
