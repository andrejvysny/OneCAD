#include "sketch/Sketch.h"
#include "sketch/constraints/Constraints.h"

#include <QJsonObject>

#include <cassert>
#include <iostream>
#include <numbers>

using namespace onecad::core::sketch;
using namespace onecad::core::sketch::constraints;

int main() {
    Sketch sketch;

    auto p1 = sketch.addPoint(0.0, 0.0);
    auto p2 = sketch.addPoint(0.0, 0.0);
    CoincidentConstraint coincident(p1, p2);
    assert(coincident.isSatisfied(sketch, 1e-6));

    auto* p2Entity = sketch.getEntityAs<SketchPoint>(p2);
    assert(p2Entity);
    p2Entity->setPosition(1.0, 0.0);
    assert(!coincident.isSatisfied(sketch, 1e-6));

    auto hStart = sketch.addPoint(0.0, 2.0);
    auto hEnd = sketch.addPoint(5.0, 2.0);
    auto hLine = sketch.addLine(hStart, hEnd);
    HorizontalConstraint horizontal(hLine);
    assert(horizontal.isSatisfied(sketch, 1e-6));

    auto vStart = sketch.addPoint(3.0, -1.0);
    auto vEnd = sketch.addPoint(3.0, 4.0);
    auto vLine = sketch.addLine(vStart, vEnd);
    VerticalConstraint vertical(vLine);
    assert(vertical.isSatisfied(sketch, 1e-6));

    auto p3 = sketch.addPoint(0.0, 4.0);
    auto p4 = sketch.addPoint(5.0, 4.0);
    auto hLine2 = sketch.addLine(p3, p4);
    ParallelConstraint parallel(hLine, hLine2);
    assert(parallel.isSatisfied(sketch, 1e-6));

    PerpendicularConstraint perpendicular(hLine, vLine);
    assert(perpendicular.isSatisfied(sketch, 1e-6));

    auto npStart = sketch.addPoint(0.0, 0.0);
    auto npEnd = sketch.addPoint(5.0, 5.0);
    auto nonParallelLine = sketch.addLine(npStart, npEnd);
    ParallelConstraint notParallel(hLine, nonParallelLine);
    assert(!notParallel.isSatisfied(sketch, 1e-6));

    auto circleCenter = sketch.addPoint(0.0, 0.0);
    auto circle = sketch.addCircle(circleCenter, 5.0);
    auto tStart = sketch.addPoint(-10.0, 5.0);
    auto tEnd = sketch.addPoint(10.0, 5.0);
    auto tangentLine = sketch.addLine(tStart, tEnd);
    TangentConstraint tangent(tangentLine, circle);
    assert(tangent.isSatisfied(sketch, 1e-6));

    auto eqStart = sketch.addPoint(0.0, -3.0);
    auto eqEnd = sketch.addPoint(5.0, -3.0);
    auto eqLine = sketch.addLine(eqStart, eqEnd);
    EqualConstraint equal(hLine, eqLine);
    assert(equal.isSatisfied(sketch, 1e-6));

    DistanceConstraint distance(p1, p2, 1.0);
    assert(distance.isSatisfied(sketch, 1e-6));

    DistanceConstraint pointLineDistance(p1, hLine, 2.0);
    assert(pointLineDistance.isSatisfied(sketch, 1e-6));

    AngleConstraint angle(hLine, vLine, std::numbers::pi_v<double> * 0.5);
    assert(angle.isSatisfied(sketch, 1e-6));

    RadiusConstraint radius(circle, 5.0);
    assert(radius.isSatisfied(sketch, 1e-6));

    QJsonObject json;
    distance.serialize(json);
    auto recreated = ConstraintFactory::fromJson(json);
    assert(recreated);
    assert(recreated->type() == ConstraintType::Distance);
    auto* recreatedDistance = dynamic_cast<DistanceConstraint*>(recreated.get());
    assert(recreatedDistance);
    assert(recreatedDistance->entity1() == distance.entity1());
    assert(recreatedDistance->entity2() == distance.entity2());
    assert(recreatedDistance->distance() == distance.distance());

    std::cout << "Sketch constraints prototype: OK" << std::endl;
    return 0;
}
