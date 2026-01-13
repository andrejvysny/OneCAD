#include "test_harness/TestHarness.h"
#include "sketch/Sketch.h"
#include "sketch/constraints/Constraints.h"

#include <QJsonObject>

#include <memory>
#include <numbers>

using namespace onecad::core::sketch;
using namespace onecad::core::sketch::constraints;

TEST_CASE(Positional_Fixed_Midpoint_Coincident) {
    Sketch sketch;

    auto p = sketch.addPoint(1.0, 2.0);
    FixedConstraint fixed(p, 1.0, 2.0);
    EXPECT_TRUE(fixed.isSatisfied(sketch, 1e-6));
    auto* pEnt = sketch.getEntityAs<SketchPoint>(p);
    EXPECT_TRUE(pEnt != nullptr);
    if (pEnt) {
        pEnt->setPosition(1.1, 2.0);
    }
    EXPECT_FALSE(fixed.isSatisfied(sketch, 1e-6));

    auto l0p1 = sketch.addPoint(0.0, 0.0);
    auto l0p2 = sketch.addPoint(4.0, 0.0);
    auto line = sketch.addLine(l0p1, l0p2);
    auto midPointId = sketch.addPoint(2.0, 0.0);
    MidpointConstraint midpointC(midPointId, line);
    EXPECT_TRUE(midpointC.isSatisfied(sketch, 1e-6));
    auto* midEnt = sketch.getEntityAs<SketchPoint>(midPointId);
    if (midEnt) {
        midEnt->setPosition(2.0, 0.1);
    }
    EXPECT_FALSE(midpointC.isSatisfied(sketch, 1e-6));

    auto degLineP = sketch.addPoint(1.0, 1.0);
    auto degLine = sketch.addLine(degLineP, degLineP);
    auto midDegPt = sketch.addPoint(1.0, 1.0);
    MidpointConstraint degMid(midDegPt, degLine);
    EXPECT_TRUE(degMid.isSatisfied(sketch, 1e-6));

    auto p1 = sketch.addPoint(0.0, 0.0);
    auto p2 = sketch.addPoint(0.0, 0.0);
    CoincidentConstraint coincident(p1, p2);
    EXPECT_TRUE(coincident.isSatisfied(sketch, 1e-6));
    auto* p2Ent = sketch.getEntityAs<SketchPoint>(p2);
    if (p2Ent) {
        p2Ent->setPosition(1.0, 0.0);
    }
    EXPECT_FALSE(coincident.isSatisfied(sketch, 1e-6));
}

TEST_CASE(Positional_Horizontal_Vertical_OnCurve) {
    Sketch sketch;

    auto hStart = sketch.addPoint(0.0, 2.0);
    auto hEnd = sketch.addPoint(5.0, 2.0);
    auto hLine = sketch.addLine(hStart, hEnd);
    HorizontalConstraint horizontal(hLine);
    EXPECT_TRUE(horizontal.isSatisfied(sketch, 1e-6));
    auto* hEndEnt = sketch.getEntityAs<SketchPoint>(hEnd);
    if (hEndEnt) hEndEnt->setPosition(5.0, 2.2);
    EXPECT_FALSE(horizontal.isSatisfied(sketch, 1e-6));

    auto vStart = sketch.addPoint(3.0, -1.0);
    auto vEnd = sketch.addPoint(3.0, 4.0);
    auto vLine = sketch.addLine(vStart, vEnd);
    VerticalConstraint vertical(vLine);
    EXPECT_TRUE(vertical.isSatisfied(sketch, 1e-6));
    auto* vEndEnt = sketch.getEntityAs<SketchPoint>(vEnd);
    if (vEndEnt) vEndEnt->setPosition(3.1, 4.0);
    EXPECT_FALSE(vertical.isSatisfied(sketch, 1e-6));

    // PointOnCurve: arc start, line arbitrary, circle arbitrary
    auto arcCenter = sketch.addPoint(0.0, 0.0);
    auto arc = sketch.addArc(arcCenter, 5.0, 0.0, std::numbers::pi_v<double> * 0.5);
    auto arcStartPt = sketch.addPoint(5.0, 0.0);
    PointOnCurveConstraint pocArcStart(arcStartPt, arc, CurvePosition::Start);
    EXPECT_TRUE(pocArcStart.isSatisfied(sketch, 1e-6));
    auto* arcStartEnt = sketch.getEntityAs<SketchPoint>(arcStartPt);
    if (arcStartEnt) arcStartEnt->setPosition(5.1, 0.0);
    EXPECT_FALSE(pocArcStart.isSatisfied(sketch, 1e-6));

    auto arcEndPt = sketch.addPoint(0.0, 5.0);
    PointOnCurveConstraint pocArcEnd(arcEndPt, arc, CurvePosition::End);
    EXPECT_TRUE(pocArcEnd.isSatisfied(sketch, 1e-6));

    auto lineP1 = sketch.addPoint(0.0, 0.0);
    auto lineP2 = sketch.addPoint(5.0, 0.0);
    auto lineId = sketch.addLine(lineP1, lineP2);
    auto onLinePt = sketch.addPoint(2.0, 0.0);
    PointOnCurveConstraint pocLine(onLinePt, lineId, CurvePosition::Arbitrary);
    EXPECT_TRUE(pocLine.isSatisfied(sketch, 1e-6));
    auto* onLineEnt = sketch.getEntityAs<SketchPoint>(onLinePt);
    if (onLineEnt) onLineEnt->setPosition(2.0, 0.5);
    EXPECT_FALSE(pocLine.isSatisfied(sketch, 1e-6));

    PointOnCurveConstraint pocLineEnd(lineP2, lineId, CurvePosition::End);
    EXPECT_TRUE(pocLineEnd.isSatisfied(sketch, 1e-6));

    auto circleCenter = sketch.addPoint(0.0, 0.0);
    auto circle = sketch.addCircle(circleCenter, 3.0);
    auto onCirclePt = sketch.addPoint(3.0, 0.0);
    PointOnCurveConstraint pocCircle(onCirclePt, circle, CurvePosition::Arbitrary);
    EXPECT_TRUE(pocCircle.isSatisfied(sketch, 1e-6));
    auto* onCircleEnt = sketch.getEntityAs<SketchPoint>(onCirclePt);
    if (onCircleEnt) onCircleEnt->setPosition(3.2, 0.0);
    EXPECT_FALSE(pocCircle.isSatisfied(sketch, 1e-6));

    auto zeroLineP = sketch.addPoint(-1.0, -1.0);
    auto zeroLine = sketch.addLine(zeroLineP, zeroLineP);
    auto zeroOnPt = sketch.addPoint(-1.0, -1.5);
    PointOnCurveConstraint pocZero(zeroOnPt, zeroLine, CurvePosition::Arbitrary);
    EXPECT_FALSE(pocZero.isSatisfied(sketch, 1e-6));
}

TEST_CASE(Relational_Parallel_Perpendicular_Tangent_Equal_Concentric) {
    Sketch sketch;

    auto h1s = sketch.addPoint(0.0, 1.0);
    auto h1e = sketch.addPoint(5.0, 1.0);
    auto h1 = sketch.addLine(h1s, h1e);
    auto h2s = sketch.addPoint(0.0, 3.0);
    auto h2e = sketch.addPoint(5.0, 3.0);
    auto h2 = sketch.addLine(h2s, h2e);
    ParallelConstraint parallel(h1, h2);
    EXPECT_TRUE(parallel.isSatisfied(sketch, 1e-6));

    auto diagS = sketch.addPoint(0.0, 0.0);
    auto diagE = sketch.addPoint(3.0, 3.0);
    auto diag = sketch.addLine(diagS, diagE);
    ParallelConstraint notParallel(h1, diag);
    EXPECT_FALSE(notParallel.isSatisfied(sketch, 1e-6));

    auto v1s = sketch.addPoint(1.0, -1.0);
    auto v1e = sketch.addPoint(1.0, 4.0);
    auto v1 = sketch.addLine(v1s, v1e);
    PerpendicularConstraint perp(h1, v1);
    EXPECT_TRUE(perp.isSatisfied(sketch, 1e-6));

    auto tangLineS = sketch.addPoint(-10.0, 5.0);
    auto tangLineE = sketch.addPoint(10.0, 5.0);
    auto tangLine = sketch.addLine(tangLineS, tangLineE);
    auto tangCenter = sketch.addPoint(0.0, 0.0);
    auto tangCircle = sketch.addCircle(tangCenter, 5.0);
    TangentConstraint tangent(tangLine, tangCircle);
    EXPECT_TRUE(tangent.isSatisfied(sketch, 1e-6));

    auto tCircle2Center = sketch.addPoint(10.0, 0.0);
    auto tCircle2 = sketch.addCircle(tCircle2Center, 5.0);
    TangentConstraint tangentCirclesExternal(tangCircle, tCircle2);
    EXPECT_TRUE(tangentCirclesExternal.isSatisfied(sketch, 1e-6));

    auto tCircle3Center = sketch.addPoint(1.0, 0.0);
    auto tCircle3 = sketch.addCircle(tCircle3Center, 4.0);
    TangentConstraint tangentCirclesInternal(tangCircle, tCircle3);
    EXPECT_TRUE(tangentCirclesInternal.isSatisfied(sketch, 1e-6));

    auto eq1s = sketch.addPoint(0.0, -3.0);
    auto eq1e = sketch.addPoint(5.0, -3.0);
    auto eq1 = sketch.addLine(eq1s, eq1e);
    EqualConstraint equalLines(h1, eq1);
    EXPECT_TRUE(equalLines.isSatisfied(sketch, 1e-6));

    auto eqCenter1 = sketch.addPoint(0.0, -6.0);
    auto eqCircle1 = sketch.addCircle(eqCenter1, 2.0);
    auto eqCenter2 = sketch.addPoint(5.0, -6.0);
    auto eqCircle2 = sketch.addCircle(eqCenter2, 2.0);
    EqualConstraint equalCircles(eqCircle1, eqCircle2);
    EXPECT_TRUE(equalCircles.isSatisfied(sketch, 1e-6));

    auto c1Center = sketch.addPoint(0.0, 0.0);
    auto c1 = sketch.addCircle(c1Center, 2.0);
    auto c2Center = sketch.addPoint(1.0, 0.0);
    auto c2 = sketch.addCircle(c2Center, 2.0);
    ConcentricConstraint notConcentric(c1, c2);
    EXPECT_FALSE(notConcentric.isSatisfied(sketch, 1e-6));
    auto c3Center = sketch.addPoint(0.0, 0.0);
    auto c3 = sketch.addCircle(c3Center, 3.0);
    ConcentricConstraint concentric(c1, c3);
    EXPECT_TRUE(concentric.isSatisfied(sketch, 1e-6));
}

TEST_CASE(Dimensional_Distance_Angle_Radius_Diameter) {
    Sketch sketch;

    // Point-point distance
    auto p1 = sketch.addPoint(0.0, 0.0);
    auto p2 = sketch.addPoint(3.0, 4.0);
    DistanceConstraint ppDist(p1, p2, 5.0);
    EXPECT_TRUE(ppDist.isSatisfied(sketch, 1e-6));

    // Point-line distance
    auto lp1 = sketch.addPoint(0.0, 0.0);
    auto lp2 = sketch.addPoint(5.0, 0.0);
    auto line = sketch.addLine(lp1, lp2);
    auto offPoint = sketch.addPoint(0.0, 2.0);
    DistanceConstraint plDist(offPoint, line, 2.0);
    EXPECT_TRUE(plDist.isSatisfied(sketch, 1e-6));

    // Line-line distance (parallel)
    auto l2p1 = sketch.addPoint(0.0, 3.0);
    auto l2p2 = sketch.addPoint(5.0, 3.0);
    auto line2 = sketch.addLine(l2p1, l2p2);
    DistanceConstraint llDist(line, line2, 3.0);
    EXPECT_TRUE(llDist.isSatisfied(sketch, 1e-6));

    auto npl1s = sketch.addPoint(0.0, 0.0);
    auto npl1e = sketch.addPoint(4.0, 0.0);
    auto npl1 = sketch.addLine(npl1s, npl1e);
    auto npl2s = sketch.addPoint(0.0, 0.0);
    auto npl2e = sketch.addPoint(2.0, 2.0);
    auto npl2 = sketch.addLine(npl2s, npl2e);
    DistanceConstraint nonParallel(npl1, npl2, 1.0);
    EXPECT_FALSE(nonParallel.isSatisfied(sketch, 1e-6));

    auto zlPoint = sketch.addPoint(0.0, 0.0);
    auto zeroLenLine = sketch.addLine(zlPoint, zlPoint);
    auto offsetPt = sketch.addPoint(0.0, 1.0);
    DistanceConstraint zeroLineDist(offsetPt, zeroLenLine, 1.0);
    EXPECT_FALSE(zeroLineDist.isSatisfied(sketch, 1e-6));

    // Angle
    auto a1s = sketch.addPoint(0.0, 0.0);
    auto a1e = sketch.addPoint(5.0, 0.0);
    auto a1 = sketch.addLine(a1s, a1e);
    auto a2s = sketch.addPoint(0.0, 0.0);
    auto a2e = sketch.addPoint(0.0, 5.0);
    auto a2 = sketch.addLine(a2s, a2e);
    AngleConstraint angle(a1, a2, std::numbers::pi_v<double> * 0.5);
    EXPECT_TRUE(angle.isSatisfied(sketch, 1e-6));
    auto* a2EndEnt = sketch.getEntityAs<SketchPoint>(a2e);
    if (a2EndEnt) a2EndEnt->setPosition(1.0, 5.0);
    EXPECT_FALSE(angle.isSatisfied(sketch, 1e-6));

    // Radius
    auto rCenter = sketch.addPoint(0.0, 0.0);
    auto rCircle = sketch.addCircle(rCenter, 4.0);
    RadiusConstraint radius(rCircle, 4.0);
    EXPECT_TRUE(radius.isSatisfied(sketch, 1e-6));
    auto* rCircleEnt = sketch.getEntityAs<SketchCircle>(rCircle);
    if (rCircleEnt) rCircleEnt->setRadius(4.2);
    EXPECT_FALSE(radius.isSatisfied(sketch, 1e-6));

    // Diameter
    auto dCenter = sketch.addPoint(10.0, 0.0);
    auto dCircle = sketch.addCircle(dCenter, 2.0);
    DiameterConstraint diameter(dCircle, 4.0);
    EXPECT_TRUE(diameter.isSatisfied(sketch, 1e-6));
    auto* dCircleEnt = sketch.getEntityAs<SketchCircle>(dCircle);
    if (dCircleEnt) dCircleEnt->setRadius(2.5);
    EXPECT_FALSE(diameter.isSatisfied(sketch, 1e-6));
}

TEST_CASE(Serialization_Distance_Radius_PointOnCurve) {
    Sketch sketch;
    auto p1 = sketch.addPoint(0.0, 0.0);
    auto p2 = sketch.addPoint(1.0, 0.0);
    DistanceConstraint distance(p1, p2, 1.0);

    QJsonObject json;
    distance.serialize(json);
    auto recreated = ConstraintFactory::fromJson(json);
    EXPECT_TRUE(static_cast<bool>(recreated));
    if (recreated) {
        EXPECT_EQ(recreated->type(), ConstraintType::Distance);
        auto* recreatedDistance = dynamic_cast<DistanceConstraint*>(recreated.get());
        EXPECT_TRUE(recreatedDistance != nullptr);
        if (recreatedDistance) {
            EXPECT_EQ(recreatedDistance->entity1(), distance.entity1());
            EXPECT_EQ(recreatedDistance->entity2(), distance.entity2());
            EXPECT_NEAR(recreatedDistance->distance(), distance.distance(), 1e-9);
        }
    }

    auto cCenter = sketch.addPoint(0.0, 0.0);
    auto circle = sketch.addCircle(cCenter, 2.0);
    RadiusConstraint radius(circle, 2.0);
    QJsonObject radiusJson;
    radius.serialize(radiusJson);
    auto recreatedRadius = ConstraintFactory::fromJson(radiusJson);
    EXPECT_TRUE(static_cast<bool>(recreatedRadius));
    if (recreatedRadius) {
        EXPECT_EQ(recreatedRadius->type(), ConstraintType::Radius);
        auto* rc = dynamic_cast<RadiusConstraint*>(recreatedRadius.get());
        EXPECT_TRUE(rc != nullptr);
        if (rc) {
            EXPECT_EQ(rc->entityId(), radius.entityId());
            EXPECT_NEAR(rc->radius(), radius.radius(), 1e-9);
        }
    }

    auto dCenter = sketch.addPoint(5.0, 5.0);
    auto dCircle = sketch.addCircle(dCenter, 2.0);
    DiameterConstraint diameter(dCircle, 4.0);
    QJsonObject diaJson;
    diameter.serialize(diaJson);
    auto recreatedDia = ConstraintFactory::fromJson(diaJson);
    EXPECT_TRUE(static_cast<bool>(recreatedDia));
    if (recreatedDia) {
        EXPECT_EQ(recreatedDia->type(), ConstraintType::Diameter);
        auto* dc = dynamic_cast<DiameterConstraint*>(recreatedDia.get());
        EXPECT_TRUE(dc != nullptr);
        if (dc) {
            EXPECT_EQ(dc->entityId(), diameter.entityId());
            EXPECT_NEAR(dc->diameter(), diameter.diameter(), 1e-9);
        }
    }

    auto cc1Center = sketch.addPoint(-5.0, -5.0);
    auto cc1 = sketch.addCircle(cc1Center, 3.0);
    auto cc2Center = sketch.addPoint(-5.0, -5.0);
    auto cc2 = sketch.addCircle(cc2Center, 1.0);
    ConcentricConstraint concentric(cc1, cc2);
    QJsonObject concJson;
    concentric.serialize(concJson);
    auto recreatedConc = ConstraintFactory::fromJson(concJson);
    EXPECT_TRUE(static_cast<bool>(recreatedConc));
    if (recreatedConc) {
        EXPECT_EQ(recreatedConc->type(), ConstraintType::Concentric);
        auto* cc = dynamic_cast<ConcentricConstraint*>(recreatedConc.get());
        EXPECT_TRUE(cc != nullptr);
        if (cc) {
            EXPECT_EQ(cc->entity1(), concentric.entity1());
            EXPECT_EQ(cc->entity2(), concentric.entity2());
        }
    }

    auto pocCenter = sketch.addPoint(10.0, 10.0);
    auto pocCircle = sketch.addCircle(pocCenter, 1.0);
    auto pocPoint = sketch.addPoint(11.0, 10.0);
    PointOnCurveConstraint poc(pocPoint, pocCircle, CurvePosition::Start);
    QJsonObject pocJson;
    poc.serialize(pocJson);
    auto recreatedPoc = ConstraintFactory::fromJson(pocJson);
    EXPECT_TRUE(static_cast<bool>(recreatedPoc));
    if (recreatedPoc) {
        EXPECT_EQ(recreatedPoc->type(), ConstraintType::OnCurve);
        auto* pc = dynamic_cast<PointOnCurveConstraint*>(recreatedPoc.get());
        EXPECT_TRUE(pc != nullptr);
        if (pc) {
            EXPECT_EQ(pc->pointId(), poc.pointId());
            EXPECT_EQ(pc->curveId(), poc.curveId());
            EXPECT_EQ(static_cast<int>(pc->position()), static_cast<int>(poc.position()));
        }
    }
}

TEST_CASE(Solver_SimpleHorizontalDistance) {
    Sketch sketch;

    auto p1 = sketch.addPoint(0.0, 0.0);
    auto p2 = sketch.addPoint(2.0, 1.0);
    auto line = sketch.addLine(p1, p2);

    sketch.addHorizontal(line);
    sketch.addDistance(p1, p2, 5.0);

    auto result = sketch.solve();
    EXPECT_TRUE(result.success);
    EXPECT_TRUE(result.residual <= 1e-4);

    auto* p1Ent = sketch.getEntityAs<SketchPoint>(p1);
    auto* p2Ent = sketch.getEntityAs<SketchPoint>(p2);
    EXPECT_TRUE(p1Ent && p2Ent);
    if (p1Ent && p2Ent) {
        double dx = p2Ent->x() - p1Ent->x();
        double dy = p2Ent->y() - p1Ent->y();
        EXPECT_NEAR(std::hypot(dx, dy), 5.0, 1e-3);
        EXPECT_NEAR(dy, 0.0, 1e-5);
    }
}

TEST_CASE(Solver_DraggedPoint_TargetDistance) {
    Sketch sketch;

    auto anchor = sketch.addPoint(0.0, 0.0);
    auto movable = sketch.addPoint(1.0, 0.0);

    sketch.addConstraint(std::make_unique<FixedConstraint>(anchor, 0.0, 0.0));
    sketch.addDistance(anchor, movable, 5.0);

    auto dragResult = sketch.solveWithDrag(movable, {5.0, 0.0});
    EXPECT_TRUE(dragResult.success);
    EXPECT_TRUE(dragResult.residual <= 1e-4);

    auto* movableEnt = sketch.getEntityAs<SketchPoint>(movable);
    EXPECT_TRUE(movableEnt != nullptr);
    if (movableEnt) {
        EXPECT_NEAR(movableEnt->x(), 5.0, 1e-4);
        EXPECT_NEAR(movableEnt->y(), 0.0, 1e-4);
    }
}

int main() {
    return onecad::test::runAllTests();
}
