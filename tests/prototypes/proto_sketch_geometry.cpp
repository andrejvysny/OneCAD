#include "test_harness/TestHarness.h"
#include "sketch/SketchPoint.h"
#include "sketch/SketchLine.h"
#include "sketch/SketchArc.h"
#include "sketch/SketchCircle.h"

#include <cmath>
#include <algorithm>
#include <numbers>

using namespace onecad::core::sketch;

TEST_CASE(SketchPoint_basic) {
    SketchPoint point(1.0, 2.0);
    EXPECT_NEAR(point.x(), 1.0, 1e-6);
    EXPECT_NEAR(point.y(), 2.0, 1e-6);

    BoundingBox2d box = point.bounds();
    EXPECT_NEAR(box.minX, 1.0, 1e-6);
    EXPECT_NEAR(box.maxX, 1.0, 1e-6);
    EXPECT_NEAR(box.minY, 2.0, 1e-6);
    EXPECT_NEAR(box.maxY, 2.0, 1e-6);

    EXPECT_TRUE(point.isNear(gp_Pnt2d(1.0, 2.0), 1e-6));
    EXPECT_FALSE(point.isNear(gp_Pnt2d(10.0, 10.0), 0.1));
}

TEST_CASE(SketchLine_helpers) {
    gp_Pnt2d start(0.0, 0.0);
    gp_Pnt2d end(3.0, 4.0);

    EXPECT_NEAR(SketchLine::length(start, end), 5.0, 1e-6);

    gp_Vec2d dir = SketchLine::direction(start, end);
    EXPECT_NEAR(dir.X(), 0.6, 1e-6);
    EXPECT_NEAR(dir.Y(), 0.8, 1e-6);

    gp_Pnt2d mid = SketchLine::midpoint(start, end);
    EXPECT_NEAR(mid.X(), 1.5, 1e-6);
    EXPECT_NEAR(mid.Y(), 2.0, 1e-6);

    EXPECT_TRUE(SketchLine::isHorizontal(gp_Pnt2d(0.0, 1.0), gp_Pnt2d(5.0, 1.0)));
    EXPECT_TRUE(SketchLine::isVertical(gp_Pnt2d(2.0, -1.0), gp_Pnt2d(2.0, 3.0)));
}

TEST_CASE(SketchArc_properties) {
    SketchArc arc("center", 10.0, 0.0, std::numbers::pi_v<double> * 0.5);
    gp_Pnt2d center(0.0, 0.0);

    EXPECT_NEAR(arc.sweepAngle(), std::numbers::pi_v<double> * 0.5, 1e-6);
    EXPECT_NEAR(arc.arcLength(), 10.0 * std::numbers::pi_v<double> * 0.5, 1e-6);

    gp_Pnt2d start = arc.startPoint(center);
    gp_Pnt2d end = arc.endPoint(center);
    EXPECT_NEAR(start.X(), 10.0, 1e-6);
    EXPECT_NEAR(start.Y(), 0.0, 1e-6);
    EXPECT_NEAR(end.X(), 0.0, 1e-6);
    EXPECT_NEAR(end.Y(), 10.0, 1e-6);

    EXPECT_TRUE(arc.containsAngle(std::numbers::pi_v<double> * 0.25));
    EXPECT_FALSE(arc.containsAngle(std::numbers::pi_v<double>));

    BoundingBox2d box = arc.boundsWithCenter(center);
    EXPECT_NEAR(box.minX, 0.0, 1e-6);
    EXPECT_NEAR(box.minY, 0.0, 1e-6);
    EXPECT_NEAR(box.maxX, 10.0, 1e-6);
    EXPECT_NEAR(box.maxY, 10.0, 1e-6);
}

TEST_CASE(SketchCircle_properties) {
    SketchCircle circle("center", 5.0);
    gp_Pnt2d center(2.0, 3.0);

    EXPECT_NEAR(circle.circumference(), 2.0 * std::numbers::pi_v<double> * 5.0, 1e-6);
    gp_Pnt2d onCircle = circle.pointAtAngle(center, 0.0);
    EXPECT_NEAR(onCircle.X(), 7.0, 1e-6);
    EXPECT_NEAR(onCircle.Y(), 3.0, 1e-6);

    BoundingBox2d box = circle.boundsWithCenter(center);
    EXPECT_NEAR(box.minX, -3.0, 1e-6);
    EXPECT_NEAR(box.maxX, 7.0, 1e-6);
    EXPECT_NEAR(box.minY, -2.0, 1e-6);
    EXPECT_NEAR(box.maxY, 8.0, 1e-6);
}

int main() {
    return onecad::test::runAllTests();
}
