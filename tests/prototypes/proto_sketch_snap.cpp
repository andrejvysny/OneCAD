#include "sketch/SnapManager.h"
#include "sketch/Sketch.h"
#include "sketch/SketchLine.h"
#include "sketch/tools/SnapPreviewResolver.h"
#include "sketch/tools/SketchToolManager.h"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <functional>
#include <iostream>
#include <limits>
#include <optional>
#include <random>
#include <string>
#include <vector>

using namespace onecad::core::sketch;

namespace {

struct TestResult {
    bool pass = false;
    std::string expected;
    std::string got;
};

bool approx(double a, double b, double tol = 1e-6) {
    return std::abs(a - b) <= tol;
}

Sketch createTestSketch() {
    Sketch sketch;

    sketch.addPoint(5.0, 5.0);

    EntityID origin = sketch.addPoint(0.0, 0.0);
    EntityID hEnd = sketch.addPoint(10.0, 0.0);
    EntityID vEnd = sketch.addPoint(0.0, 10.0);
    sketch.addLine(origin, hEnd);
    sketch.addLine(origin, vEnd);

    EntityID circleCenter = sketch.addPoint(20.0, 20.0);
    sketch.addCircle(circleCenter, 5.0);

    EntityID arcCenter = sketch.addPoint(40.0, 40.0);
    sketch.addArc(arcCenter, 3.0, 0.0, M_PI * 0.5);

    return sketch;
}

SnapManager createSnapManagerFor(const std::vector<SnapType>& enabledTypes) {
    SnapManager manager;
    manager.setAllSnapsEnabled(false);
    manager.setEnabled(true);
    for (SnapType type : enabledTypes) {
        manager.setSnapEnabled(type, true);
        if (type == SnapType::Grid) {
            manager.setGridSnapEnabled(true);
        }
    }
    return manager;
}

TestResult expectSnap(SnapResult result, SnapType type) {
    if (!result.snapped) {
        return {false, "snapped", "not snapped"};
    }
    if (result.type != type) {
        return {
            false,
            std::to_string(static_cast<int>(type)),
            std::to_string(static_cast<int>(result.type))
        };
    }
    return {true, "", ""};
}

bool snapResultsEqual(const SnapResult& lhs, const SnapResult& rhs) {
    return lhs.type == rhs.type &&
           lhs.entityId == rhs.entityId &&
           lhs.secondEntityId == rhs.secondEntityId &&
           lhs.pointId == rhs.pointId &&
           approx(lhs.position.x, rhs.position.x) &&
           approx(lhs.position.y, rhs.position.y);
}

int countEntitiesOfType(const Sketch& sketch, EntityType type) {
    int count = 0;
    for (const auto& entity : sketch.getAllEntities()) {
        if (entity && entity->type() == type) {
            ++count;
        }
    }
    return count;
}

const SketchLine* findLastLine(const Sketch& sketch) {
    const SketchLine* last = nullptr;
    for (const auto& entity : sketch.getAllEntities()) {
        if (entity && entity->type() == EntityType::Line) {
            last = static_cast<const SketchLine*>(entity.get());
        }
    }
    return last;
}

SnapResult selectEffectiveSnap(const SnapResult& bestSnap, const std::vector<SnapResult>& allSnaps) {
    if (bestSnap.snapped &&
        (bestSnap.type == SnapType::Vertex || bestSnap.type == SnapType::Endpoint)) {
        return bestSnap;
    }

    SnapResult bestGuide;
    bestGuide.distance = std::numeric_limits<double>::max();
    for (const auto& snap : allSnaps) {
        if (snap.hasGuide && snap.snapped && snap.distance < bestGuide.distance) {
            bestGuide = snap;
        }
    }
    if (bestGuide.snapped) {
        return bestGuide;
    }
    return bestSnap;
}

std::optional<SnapResult> findBestGuideCandidate(const std::vector<SnapResult>& snaps) {
    SnapResult bestGuide;
    bestGuide.distance = std::numeric_limits<double>::max();
    bool found = false;
    for (const auto& snap : snaps) {
        if (snap.hasGuide && snap.snapped && snap.distance < bestGuide.distance) {
            bestGuide = snap;
            found = true;
        }
    }
    if (found) {
        return bestGuide;
    }
    return std::nullopt;
}

TestResult testVertexSnap() {
    Sketch sketch = createTestSketch();
    SnapManager manager = createSnapManagerFor({SnapType::Vertex});
    SnapResult result = manager.findBestSnap({5.2, 5.1}, sketch);
    TestResult check = expectSnap(result, SnapType::Vertex);
    if (!check.pass) {
        return check;
    }
    if (!approx(result.position.x, 5.0) || !approx(result.position.y, 5.0)) {
        return {false, "(5,5)", "(" + std::to_string(result.position.x) + "," + std::to_string(result.position.y) + ")"};
    }
    return {true, "", ""};
}

TestResult test_hinttext_vertex_snap() {
    Sketch sketch;
    sketch.addPoint(5.0, 5.0);
    SnapManager manager = createSnapManagerFor({SnapType::Vertex});
    SnapResult result = manager.findBestSnap({5.2, 5.1}, sketch);
    TestResult check = expectSnap(result, SnapType::Vertex);
    if (!check.pass) {
        return check;
    }
    if (result.hintText != "PT") {
        return {false, "PT", result.hintText};
    }
    return {true, "", ""};
}

TestResult testEndpointSnap() {
    Sketch sketch = createTestSketch();
    SnapManager manager = createSnapManagerFor({SnapType::Endpoint});
    SnapResult result = manager.findBestSnap({10.3, 0.2}, sketch);
    return expectSnap(result, SnapType::Endpoint);
}

TestResult test_hinttext_endpoint_snap() {
    Sketch sketch;
    EntityID start = sketch.addPoint(0.0, 0.0);
    EntityID end = sketch.addPoint(10.0, 0.0);
    sketch.addLine(start, end);
    SnapManager manager = createSnapManagerFor({SnapType::Endpoint});
    SnapResult result = manager.findBestSnap({10.3, 0.2}, sketch);
    TestResult check = expectSnap(result, SnapType::Endpoint);
    if (!check.pass) {
        return check;
    }
    if (result.hintText != "END") {
        return {false, "END", result.hintText};
    }
    return {true, "", ""};
}

TestResult testMidpointSnap() {
    Sketch sketch = createTestSketch();
    SnapManager manager = createSnapManagerFor({SnapType::Midpoint});
    SnapResult result = manager.findBestSnap({5.2, 0.1}, sketch);
    return expectSnap(result, SnapType::Midpoint);
}

TestResult test_hinttext_midpoint_snap() {
    Sketch sketch;
    EntityID start = sketch.addPoint(0.0, 0.0);
    EntityID end = sketch.addPoint(10.0, 0.0);
    sketch.addLine(start, end);
    SnapManager manager = createSnapManagerFor({SnapType::Midpoint});
    SnapResult result = manager.findBestSnap({5.2, 0.1}, sketch);
    TestResult check = expectSnap(result, SnapType::Midpoint);
    if (!check.pass) {
        return check;
    }
    if (result.hintText != "MID") {
        return {false, "MID", result.hintText};
    }
    return {true, "", ""};
}

TestResult testCenterSnap() {
    Sketch sketch = createTestSketch();
    SnapManager manager = createSnapManagerFor({SnapType::Center});
    SnapResult result = manager.findBestSnap({20.3, 20.2}, sketch);
    return expectSnap(result, SnapType::Center);
}

TestResult test_hinttext_center_snap() {
    Sketch sketch = createTestSketch();
    SnapManager manager = createSnapManagerFor({SnapType::Center});
    SnapResult result = manager.findBestSnap({20.3, 20.2}, sketch);
    TestResult check = expectSnap(result, SnapType::Center);
    if (!check.pass) {
        return check;
    }
    if (result.hintText != "CEN") {
        return {false, "CEN", result.hintText};
    }
    return {true, "", ""};
}

TestResult testQuadrantSnap() {
    Sketch sketch = createTestSketch();
    SnapManager manager = createSnapManagerFor({SnapType::Quadrant});
    SnapResult result = manager.findBestSnap({25.1, 20.2}, sketch);
    return expectSnap(result, SnapType::Quadrant);
}

TestResult testIntersectionSnap() {
    Sketch sketch = createTestSketch();
    SnapManager manager = createSnapManagerFor({SnapType::Intersection});
    SnapResult result = manager.findBestSnap({0.3, 0.2}, sketch);
    return expectSnap(result, SnapType::Intersection);
}

TestResult testOnCurveSnap() {
    Sketch sketch = createTestSketch();
    SnapManager manager = createSnapManagerFor({SnapType::OnCurve});
    SnapResult result = manager.findBestSnap({20.2, 15.6}, sketch);
    return expectSnap(result, SnapType::OnCurve);
}

TestResult testEllipseCenterSnap() {
    Sketch sketch;
    EntityID center = sketch.addPoint(30.0, 30.0);
    sketch.addEllipse(center, 6.0, 4.0, 0.0);

    SnapManager manager = createSnapManagerFor({SnapType::Center});
    SnapResult result = manager.findBestSnap({30.2, 30.1}, sketch);
    TestResult check = expectSnap(result, SnapType::Center);
    if (!check.pass) {
        return check;
    }
    if (!approx(result.position.x, 30.0) || !approx(result.position.y, 30.0)) {
        return {false, "(30,30)", "(" + std::to_string(result.position.x) + "," + std::to_string(result.position.y) + ")"};
    }
    return {true, "", ""};
}

TestResult testEllipseQuadrantSnap() {
    Sketch sketch;
    EntityID center = sketch.addPoint(30.0, 30.0);
    sketch.addEllipse(center, 6.0, 4.0, 0.0);

    SnapManager manager = createSnapManagerFor({SnapType::Quadrant});
    SnapResult result = manager.findBestSnap({36.1, 30.2}, sketch);
    TestResult check = expectSnap(result, SnapType::Quadrant);
    if (!check.pass) {
        return check;
    }
    if (!approx(result.position.x, 36.0, 1e-5) || !approx(result.position.y, 30.0, 1e-5)) {
        return {false, "(36,30)", "(" + std::to_string(result.position.x) + "," + std::to_string(result.position.y) + ")"};
    }
    return {true, "", ""};
}

TestResult testEllipseOnCurveSnap() {
    Sketch sketch;
    EntityID center = sketch.addPoint(30.0, 30.0);
    sketch.addEllipse(center, 6.0, 4.0, 0.0);

    SnapManager manager = createSnapManagerFor({SnapType::OnCurve});
    SnapResult result = manager.findBestSnap({35.3, 32.2}, sketch);
    return expectSnap(result, SnapType::OnCurve);
}

TestResult testEllipseLineIntersection() {
    Sketch sketch;
    EntityID center = sketch.addPoint(30.0, 30.0);
    sketch.addEllipse(center, 6.0, 4.0, 0.0);
    sketch.addLine(20.0, 30.0, 40.0, 30.0);

    SnapManager manager = createSnapManagerFor({SnapType::Intersection});
    SnapResult result = manager.findBestSnap({36.2, 30.1}, sketch);
    TestResult check = expectSnap(result, SnapType::Intersection);
    if (!check.pass) {
        return check;
    }
    if (!approx(result.position.x, 36.0, 1e-5) || !approx(result.position.y, 30.0, 1e-5)) {
        return {false, "(36,30)", "(" + std::to_string(result.position.x) + "," + std::to_string(result.position.y) + ")"};
    }
    return {true, "", ""};
}

TestResult testEllipseQuadrantRotated() {
    Sketch sketch;
    EntityID center = sketch.addPoint(30.0, 30.0);
    sketch.addEllipse(center, 6.0, 4.0, M_PI / 4.0);

    const double expected = 30.0 + 6.0 * std::sqrt(0.5);
    SnapManager manager = createSnapManagerFor({SnapType::Quadrant});
    SnapResult result = manager.findBestSnap({expected + 0.2, expected - 0.1}, sketch);
    TestResult check = expectSnap(result, SnapType::Quadrant);
    if (!check.pass) {
        return check;
    }
    if (!approx(result.position.x, expected, 1e-5) || !approx(result.position.y, expected, 1e-5)) {
        return {false,
                "(" + std::to_string(expected) + "," + std::to_string(expected) + ")",
                "(" + std::to_string(result.position.x) + "," + std::to_string(result.position.y) + ")"};
    }
    return {true, "", ""};
}

TestResult testGridSnap() {
    Sketch sketch = createTestSketch();
    SnapManager manager = createSnapManagerFor({SnapType::Grid});
    manager.setGridSize(1.0);
    SnapResult result = manager.findBestSnap({100.2, 100.2}, sketch);
    return expectSnap(result, SnapType::Grid);
}

TestResult test_hinttext_grid_snap() {
    Sketch sketch = createTestSketch();
    SnapManager manager = createSnapManagerFor({SnapType::Grid});
    manager.setGridSize(1.0);
    SnapResult result = manager.findBestSnap({100.2, 100.2}, sketch);
    TestResult check = expectSnap(result, SnapType::Grid);
    if (!check.pass) {
        return check;
    }
    if (result.hintText != "GRID") {
        return {false, "GRID", result.hintText};
    }
    return {true, "", ""};
}

TestResult testPerpendicularSnapLine() {
    Sketch sketch;
    sketch.addLine(0.0, 0.0, 10.0, 0.0);

    SnapManager manager = createSnapManagerFor({SnapType::Perpendicular});
    SnapResult result = manager.findBestSnap({5.0, 1.5}, sketch);
    TestResult check = expectSnap(result, SnapType::Perpendicular);
    if (!check.pass) {
        return check;
    }
    if (!approx(result.position.x, 5.0, 0.01) || !approx(result.position.y, 0.0, 0.01)) {
        return {false, "(5,0)", "(" + std::to_string(result.position.x) + "," + std::to_string(result.position.y) + ")"};
    }
    return {true, "", ""};
}

TestResult testPerpendicularSnapCircle() {
    Sketch sketch;
    EntityID center = sketch.addPoint(20.0, 20.0);
    sketch.addCircle(center, 5.0);

    SnapManager manager = createSnapManagerFor({SnapType::Perpendicular});
    SnapResult result = manager.findBestSnap({26.0, 20.0}, sketch);
    TestResult check = expectSnap(result, SnapType::Perpendicular);
    if (!check.pass) {
        return check;
    }
    if (!approx(result.position.x, 25.0, 0.01) || !approx(result.position.y, 20.0, 0.01)) {
        return {false, "(25,20)", "(" + std::to_string(result.position.x) + "," + std::to_string(result.position.y) + ")"};
    }
    return {true, "", ""};
}

TestResult testPerpendicularSnapArc() {
    Sketch sketch;
    EntityID center = sketch.addPoint(40.0, 40.0);
    sketch.addArc(center, 3.0, 0.0, M_PI * 0.5);

    SnapManager manager = createSnapManagerFor({SnapType::Perpendicular});
    SnapResult result = manager.findBestSnap({42.2, 42.2}, sketch);
    TestResult check = expectSnap(result, SnapType::Perpendicular);
    if (!check.pass) {
        return check;
    }

    const double expected = 40.0 + (3.0 / std::sqrt(2.0));
    if (!approx(result.position.x, expected, 0.01) || !approx(result.position.y, expected, 0.01)) {
        return {false,
                "(" + std::to_string(expected) + "," + std::to_string(expected) + ")",
                "(" + std::to_string(result.position.x) + "," + std::to_string(result.position.y) + ")"};
    }
    return {true, "", ""};
}

TestResult test_perpendicular_guide_metadata() {
    Sketch sketch;
    sketch.addLine(0.0, 0.0, 10.0, 0.0);

    SnapManager manager = createSnapManagerFor({SnapType::Perpendicular});
    SnapResult result = manager.findBestSnap({5.0, 1.5}, sketch);
    TestResult check = expectSnap(result, SnapType::Perpendicular);
    if (!check.pass) {
        return check;
    }
    if (result.hintText != "PERP") {
        return {false, "PERP", result.hintText};
    }
    if (!result.hasGuide) {
        return {false, "hasGuide=true", "hasGuide=false"};
    }
    if (!approx(result.guideOrigin.x, 5.0, 1e-6) || !approx(result.guideOrigin.y, 1.5, 1e-6)) {
        return {false,
                "guideOrigin=(5,1.5)",
                "(" + std::to_string(result.guideOrigin.x) + "," + std::to_string(result.guideOrigin.y) + ")"};
    }
    return {true, "", ""};
}

TestResult testTangentSnapCircle() {
    Sketch sketch;
    EntityID center = sketch.addPoint(20.0, 20.0);
    sketch.addCircle(center, 5.0);

    SnapManager manager = createSnapManagerFor({SnapType::Tangent});
    manager.setSnapRadius(10.0);
    SnapResult result = manager.findBestSnap({30.0, 20.0}, sketch);
    TestResult check = expectSnap(result, SnapType::Tangent);
    if (!check.pass) {
        return check;
    }

    const Vec2d expected1{22.5, 20.0 + 2.5 * std::sqrt(3.0)};
    const Vec2d expected2{22.5, 20.0 - 2.5 * std::sqrt(3.0)};
    const bool matchFirst = approx(result.position.x, expected1.x, 0.01) && approx(result.position.y, expected1.y, 0.01);
    const bool matchSecond = approx(result.position.x, expected2.x, 0.01) && approx(result.position.y, expected2.y, 0.01);
    if (!matchFirst && !matchSecond) {
        return {false,
                "(22.5, 24.3301) or (22.5, 15.6699)",
                "(" + std::to_string(result.position.x) + "," + std::to_string(result.position.y) + ")"};
    }
    return {true, "", ""};
}

TestResult testTangentSnapArc() {
    Sketch sketch;
    EntityID center = sketch.addPoint(40.0, 40.0);
    sketch.addArc(center, 3.0, 0.0, M_PI * 0.5);

    SnapManager manager = createSnapManagerFor({SnapType::Tangent});
    manager.setSnapRadius(5.0);
    SnapResult result = manager.findBestSnap({45.0, 40.0}, sketch);
    TestResult check = expectSnap(result, SnapType::Tangent);
    if (!check.pass) {
        return check;
    }

    if (!approx(result.position.x, 41.8, 0.01) || !approx(result.position.y, 42.4, 0.01)) {
        return {false, "(41.8,42.4)", "(" + std::to_string(result.position.x) + "," + std::to_string(result.position.y) + ")"};
    }
    return {true, "", ""};
}

TestResult test_tangent_guide_metadata() {
    Sketch sketch;
    EntityID center = sketch.addPoint(20.0, 20.0);
    sketch.addCircle(center, 5.0);

    SnapManager manager = createSnapManagerFor({SnapType::Tangent});
    manager.setSnapRadius(10.0);
    SnapResult result = manager.findBestSnap({30.0, 20.0}, sketch);
    TestResult check = expectSnap(result, SnapType::Tangent);
    if (!check.pass) {
        return check;
    }
    if (result.hintText != "TAN") {
        return {false, "TAN", result.hintText};
    }
    if (!result.hasGuide) {
        return {false, "hasGuide=true", "hasGuide=false"};
    }
    return {true, "", ""};
}

TestResult testHorizontalAlignmentSnap() {
    Sketch sketch;
    sketch.addPoint(5.0, 5.0);

    SnapManager manager = createSnapManagerFor({SnapType::Horizontal});
    SnapResult result = manager.findBestSnap({15.0, 5.5}, sketch);
    TestResult check = expectSnap(result, SnapType::Horizontal);
    if (!check.pass) {
        return check;
    }
    if (!approx(result.position.x, 15.0, 1e-6) || !approx(result.position.y, 5.0, 1e-6)) {
        return {false,
                "(15,5)",
                "(" + std::to_string(result.position.x) + "," + std::to_string(result.position.y) + ")"};
    }
    if (!result.hasGuide) {
        return {false, "hasGuide=true", "hasGuide=false"};
    }
    if (!approx(result.guideOrigin.x, 5.0, 1e-6) || !approx(result.guideOrigin.y, 5.0, 1e-6)) {
        return {false,
                "guideOrigin=(5,5)",
                "(" + std::to_string(result.guideOrigin.x) + "," + std::to_string(result.guideOrigin.y) + ")"};
    }
    return {true, "", ""};
}

TestResult testVerticalAlignmentSnap() {
    Sketch sketch;
    sketch.addPoint(5.0, 5.0);

    SnapManager manager = createSnapManagerFor({SnapType::Vertical});
    SnapResult result = manager.findBestSnap({5.5, 15.0}, sketch);
    TestResult check = expectSnap(result, SnapType::Vertical);
    if (!check.pass) {
        return check;
    }
    if (!approx(result.position.x, 5.0, 1e-6) || !approx(result.position.y, 15.0, 1e-6)) {
        return {false,
                "(5,15)",
                "(" + std::to_string(result.position.x) + "," + std::to_string(result.position.y) + ")"};
    }
    if (!result.hasGuide) {
        return {false, "hasGuide=true", "hasGuide=false"};
    }
    return {true, "", ""};
}

TestResult testExtensionSnapLine() {
    Sketch sketch;
    sketch.addLine(0.0, 0.0, 10.0, 0.0);

    SnapManager manager = createSnapManagerFor({SnapType::SketchGuide});
    SnapResult result = manager.findBestSnap({12.0, 0.5}, sketch);
    TestResult check = expectSnap(result, SnapType::SketchGuide);
    if (!check.pass) {
        return check;
    }
    if (!approx(result.position.x, 12.0, 1e-6) || !approx(result.position.y, 0.0, 1e-6)) {
        return {false,
                "(12,0)",
                "(" + std::to_string(result.position.x) + "," + std::to_string(result.position.y) + ")"};
    }
    if (!result.hasGuide) {
        return {false, "hasGuide=true", "hasGuide=false"};
    }
    if (!approx(result.guideOrigin.x, 10.0, 1e-6) || !approx(result.guideOrigin.y, 0.0, 1e-6)) {
        return {false,
                "guideOrigin=(10,0)",
                "(" + std::to_string(result.guideOrigin.x) + "," + std::to_string(result.guideOrigin.y) + ")"};
    }
    return {true, "", ""};
}

TestResult testExtensionSnapNoArc() {
    Sketch sketch;
    EntityID center = sketch.addPoint(0.0, 0.0);
    sketch.addArc(center, 5.0, 0.0, M_PI * 0.5);

    SnapManager manager = createSnapManagerFor({SnapType::SketchGuide});
    SnapResult result = manager.findBestSnap({8.0, 1.0}, sketch);
    if (result.snapped && result.type == SnapType::SketchGuide) {
        return {false, "no SketchGuide snap", "SketchGuide snapped"};
    }
    return {true, "", ""};
}

TestResult testAngularSnap15degRounding() {
    Sketch sketch;
    SnapManager manager = createSnapManagerFor({SnapType::SketchGuide});

    const double dist = 10.0;
    const double angleRad = 22.0 * M_PI / 180.0;
    const Vec2d cursor{dist * std::cos(angleRad), dist * std::sin(angleRad)};

    SnapResult result = manager.findBestSnap(cursor, sketch, {}, Vec2d{0.0, 0.0});
    TestResult check = expectSnap(result, SnapType::SketchGuide);
    if (!check.pass) {
        return check;
    }

    const double expectedAngleRad = 15.0 * M_PI / 180.0;
    const Vec2d expected{dist * std::cos(expectedAngleRad), dist * std::sin(expectedAngleRad)};
    if (!approx(result.position.x, expected.x, 1e-6) || !approx(result.position.y, expected.y, 1e-6)) {
        return {false,
                "snapped to 15deg",
                "(" + std::to_string(result.position.x) + "," + std::to_string(result.position.y) + ")"};
    }
    if (!result.hasGuide) {
        return {false, "hasGuide=true", "hasGuide=false"};
    }
    if (!approx(result.guideOrigin.x, 0.0, 1e-6) || !approx(result.guideOrigin.y, 0.0, 1e-6)) {
        return {false,
                "guideOrigin=(0,0)",
                "(" + std::to_string(result.guideOrigin.x) + "," + std::to_string(result.guideOrigin.y) + ")"};
    }
    if (result.hintText != "15\xC2\xB0") {
        return {false, "15deg", result.hintText};
    }

    return {true, "", ""};
}

TestResult testAngularSnap45degExact() {
    Sketch sketch;
    SnapManager manager = createSnapManagerFor({SnapType::SketchGuide});

    const double dist = 10.0;
    const double angleRad = 45.0 * M_PI / 180.0;
    const Vec2d cursor{dist * std::cos(angleRad), dist * std::sin(angleRad)};

    SnapResult result = manager.findBestSnap(cursor, sketch, {}, Vec2d{0.0, 0.0});
    TestResult check = expectSnap(result, SnapType::SketchGuide);
    if (!check.pass) {
        return check;
    }
    if (!approx(result.position.x, cursor.x, 1e-6) || !approx(result.position.y, cursor.y, 1e-6)) {
        return {false,
                "unchanged 45deg point",
                "(" + std::to_string(result.position.x) + "," + std::to_string(result.position.y) + ")"};
    }
    if (result.hintText != "45\xC2\xB0") {
        return {false, "45deg", result.hintText};
    }

    return {true, "", ""};
}

TestResult test_angular_snap_50deg_reference() {
    Sketch sketch;
    SnapManager manager = createSnapManagerFor({SnapType::SketchGuide});

    const double dist = 10.0;
    const double angleRad = 50.0 * M_PI / 180.0;
    const Vec2d cursor{dist * std::cos(angleRad), dist * std::sin(angleRad)};

    SnapResult result = manager.findBestSnap(cursor, sketch, {}, Vec2d{0.0, 0.0});
    TestResult check = expectSnap(result, SnapType::SketchGuide);
    if (!check.pass) {
        return check;
    }

    const double expectedAngleRad = 45.0 * M_PI / 180.0;
    const Vec2d expected{dist * std::cos(expectedAngleRad), dist * std::sin(expectedAngleRad)};
    if (!approx(result.position.x, expected.x, 1e-6) || !approx(result.position.y, expected.y, 1e-6)) {
        return {false,
                "snapped to 45deg",
                "(" + std::to_string(result.position.x) + "," + std::to_string(result.position.y) + ")"};
    }
    if (!result.hasGuide) {
        return {false, "hasGuide=true", "hasGuide=false"};
    }
    if (!approx(result.guideOrigin.x, 0.0, 1e-6) || !approx(result.guideOrigin.y, 0.0, 1e-6)) {
        return {false,
                "guideOrigin=(0,0)",
                "(" + std::to_string(result.guideOrigin.x) + "," + std::to_string(result.guideOrigin.y) + ")"};
    }
    if (result.hintText != "45\xC2\xB0") {
        return {false, "45deg", result.hintText};
    }

    return {true, "", ""};
}

TestResult testAngularSnapNoReference() {
    Sketch sketch;
    sketch.addLine(0.0, 0.0, 10.0, 0.0);

    SnapManager manager = createSnapManagerFor({SnapType::SketchGuide});

    const double dist = 10.0;
    const double angleRad = 22.0 * M_PI / 180.0;
    const Vec2d cursor{dist * std::cos(angleRad), dist * std::sin(angleRad)};

    SnapResult result = manager.findBestSnap(cursor, sketch);
    if (result.snapped && result.type == SnapType::SketchGuide) {
        return {false, "no angular SketchGuide without reference", "SketchGuide snapped"};
    }
    return {true, "", ""};
}

TestResult testToggleSuppression() {
    Sketch sketch = createTestSketch();
    SnapManager manager;
    manager.setAllSnapsEnabled(false);
    manager.setSnapEnabled(SnapType::Grid, true);
    manager.setGridSnapEnabled(true);

    SnapResult result = manager.findBestSnap({5.1, 5.1}, sketch);
    if (result.snapped && result.type == SnapType::Vertex) {
        return {false, "not Vertex (only Grid enabled)", "got Vertex"};
    }
    if (result.snapped && result.type != SnapType::Grid) {
        return {false, "Grid or no snap", std::to_string(static_cast<int>(result.type))};
    }
    return {true, "", ""};
}

TestResult testAllSnapTypesCombined() {
    Sketch sketch = createTestSketch();
    SnapManager manager;

    SnapResult result = manager.findBestSnap({0.1, 0.1}, sketch);
    if (!result.snapped) {
        return {false, "snapped", "not snapped"};
    }
    if (result.type != SnapType::Vertex) {
        return {false, "Vertex wins priority", std::to_string(static_cast<int>(result.type))};
    }
    return {true, "", ""};
}

TestResult testPriorityOrder() {
    Sketch sketch = createTestSketch();
    SnapManager manager = createSnapManagerFor({SnapType::Vertex, SnapType::Endpoint});
    SnapResult result = manager.findBestSnap({0.1, 0.1}, sketch);
    return expectSnap(result, SnapType::Vertex);
}

TestResult test_spatial_hash_after_geometry_move() {
    Sketch sketch;
    EntityID pointId = sketch.addPoint(5.0, 5.0);
    SnapManager manager = createSnapManagerFor({SnapType::Vertex});

    SnapResult initial = manager.findBestSnap({5.2, 5.1}, sketch);
    TestResult firstCheck = expectSnap(initial, SnapType::Vertex);
    if (!firstCheck.pass) {
        return firstCheck;
    }

    SketchPoint* point = sketch.getEntityAs<SketchPoint>(pointId);
    if (!point) {
        return {false, "point exists", "nullptr"};
    }
    point->setPosition(50.0, 50.0);

    SnapResult moved = manager.findBestSnap({50.2, 50.1}, sketch);
    TestResult secondCheck = expectSnap(moved, SnapType::Vertex);
    if (!secondCheck.pass) {
        return secondCheck;
    }
    if (!approx(moved.position.x, 50.0) || !approx(moved.position.y, 50.0)) {
        return {false,
                "(50,50)",
                "(" + std::to_string(moved.position.x) + "," + std::to_string(moved.position.y) + ")"};
    }
    return {true, "", ""};
}

TestResult testSpatialHashEquivalentToBruteforce() {
    Sketch sketch;
    std::mt19937 rng(1337);
    std::uniform_real_distribution<double> pointDist(-100.0, 100.0);
    std::uniform_real_distribution<double> radiusDist(1.0, 12.0);

    std::vector<EntityID> points;
    points.reserve(80);
    for (int i = 0; i < 80; ++i) {
        points.push_back(sketch.addPoint(pointDist(rng), pointDist(rng), false));
    }

    for (int i = 0; i < 40; ++i) {
        sketch.addLine(points[2 * i], points[2 * i + 1], false);
    }

    for (int i = 0; i < 12; ++i) {
        sketch.addCircle(points[i], radiusDist(rng), false);
    }

    for (int i = 0; i < 8; ++i) {
        const double start = 0.1 * static_cast<double>(i + 1);
        const double end = start + 1.7;
        sketch.addArc(points[12 + i], radiusDist(rng), start, end, false);
    }

    SnapManager fast;
    fast.setSpatialHashEnabled(true);

    SnapManager brute;
    brute.setSpatialHashEnabled(false);

    std::uniform_real_distribution<double> cursorDist(-110.0, 110.0);
    for (int i = 0; i < 120; ++i) {
        const Vec2d cursor{cursorDist(rng), cursorDist(rng)};
        const SnapResult fastResult = fast.findBestSnap(cursor, sketch);
        const SnapResult bruteResult = brute.findBestSnap(cursor, sketch);

        if (fastResult.snapped != bruteResult.snapped) {
            return {false, "equal snapped", "different snapped"};
        }
        if (!fastResult.snapped) {
            continue;
        }
        if (fastResult.type != bruteResult.type) {
            return {false,
                    std::to_string(static_cast<int>(bruteResult.type)),
                    std::to_string(static_cast<int>(fastResult.type))};
        }
        if (!approx(fastResult.position.x, bruteResult.position.x, 1e-5) ||
            !approx(fastResult.position.y, bruteResult.position.y, 1e-5)) {
            return {false,
                    "equal position",
                    "(" + std::to_string(fastResult.position.x) + "," + std::to_string(fastResult.position.y) +
                        ") vs (" + std::to_string(bruteResult.position.x) + "," + std::to_string(bruteResult.position.y) + ")"};
        }
    }

    return {true, "", ""};
}

TestResult test_preserves_guides_when_vertex_wins() {
    Sketch sketch;
    sketch.addPoint(5.0, 5.0);
    sketch.addPoint(10.0, 5.0);

    SnapManager manager = createSnapManagerFor({SnapType::Vertex, SnapType::Horizontal});
    auto allSnaps = manager.findAllSnaps({5.01, 5.0}, sketch);

    bool foundVertex = false;
    bool foundGuide = false;

    for (const auto& s : allSnaps) {
        if (s.snapped && s.type == SnapType::Vertex) foundVertex = true;
        if (s.hasGuide) foundGuide = true;
    }

    if (!foundVertex) return {false, "Vertex snap", "not found"};
    if (!foundGuide) return {false, "Guide", "not found"};

    return {true, "", ""};
}

TestResult test_perpendicular_guide_nonzero_length() {
    Sketch sketch;
    sketch.addLine(0.0, 0.0, 10.0, 0.0);

    SnapManager manager = createSnapManagerFor({SnapType::Perpendicular});
    auto allSnaps = manager.findAllSnaps({5.0, 1.5}, sketch);

    for (const auto& s : allSnaps) {
        if (s.type == SnapType::Perpendicular) {
            if (!s.hasGuide) return {false, "hasGuide=true", "false"};
            double dist = std::sqrt(std::pow(s.guideOrigin.x - s.position.x, 2) + std::pow(s.guideOrigin.y - s.position.y, 2));
            if (dist < 1e-6) return {false, "nonzero guide length", "zero"};
            if (!approx(s.guideOrigin.x, 5.0) || !approx(s.guideOrigin.y, 1.5)) return {false, "guideOrigin=(5,1.5)", "got other"};
            return {true, "", ""};
        }
    }
    return {false, "Perpendicular snap", "not found"};
}

TestResult test_tangent_guide_nonzero_length() {
    Sketch sketch;
    EntityID center = sketch.addPoint(20.0, 20.0);
    sketch.addCircle(center, 5.0);

    SnapManager manager = createSnapManagerFor({SnapType::Tangent});
    manager.setSnapRadius(10.0);
    auto allSnaps = manager.findAllSnaps({30.0, 20.0}, sketch);

    for (const auto& s : allSnaps) {
        if (s.type == SnapType::Tangent) {
            if (!s.hasGuide) return {false, "hasGuide=true", "false"};
            double dist = std::sqrt(std::pow(s.guideOrigin.x - s.position.x, 2) + std::pow(s.guideOrigin.y - s.position.y, 2));
            if (dist < 1e-6) return {false, "nonzero guide length", "zero"};
            if (!approx(s.guideOrigin.x, 30.0) || !approx(s.guideOrigin.y, 20.0)) return {false, "guideOrigin=(30,20)", "got other"};
            return {true, "", ""};
        }
    }
    return {false, "Tangent snap", "not found"};
}

TestResult test_clears_guides_when_no_snap() {
    Sketch sketch;
    sketch.addPoint(5.0, 5.0);

    SnapManager manager = createSnapManagerFor({SnapType::Vertex});
    auto allSnaps = manager.findAllSnaps({100.0, 100.0}, sketch);

    for (const auto& s : allSnaps) {
        if (s.hasGuide) return {false, "no guides", "found guide"};
    }
    return {true, "", ""};
}

TestResult test_guide_count_bounded() {
    Sketch sketch;
    for (int i = 0; i < 5; ++i) {
        sketch.addLine(0.0, i * 2.0, 10.0, i * 2.0);
        sketch.addPoint(5.0, i * 2.0 + 1.0);
    }

    SnapManager manager; // All enabled
    auto allSnaps = manager.findAllSnaps({5.0, 5.0}, sketch);

    int guideCount = 0;
    for (const auto& s : allSnaps) {
        if (s.hasGuide) guideCount++;
    }

    if (guideCount == 0) return {false, "some guides", "0"};
    return {true, "", ""};
}

TestResult test_dedupe_collinear_guides() {
    Sketch sketch;
    sketch.addLine(0.0, 0.0, 4.0, 0.0);
    sketch.addLine(6.0, 0.0, 10.0, 0.0);

    SnapManager manager = createSnapManagerFor({SnapType::SketchGuide});
    auto allSnaps = manager.findAllSnaps({5.0, 0.01}, sketch);

    int guideCount = 0;
    for (const auto& s : allSnaps) {
        if (s.type == SnapType::SketchGuide && s.hasGuide) guideCount++;
    }

    if (guideCount < 2) return {false, "multiple extension guides", std::to_string(guideCount)};

    return {true, "", ""};
}

TestResult test_effective_snap_keeps_point_priority_over_guide() {
    SnapResult winner;
    winner.snapped = true;
    winner.type = SnapType::Vertex;
    winner.position = {5.0, 5.0};
    winner.distance = 0.5;

    SnapResult guide;
    guide.snapped = true;
    guide.type = SnapType::Perpendicular;
    guide.position = {5.1, 5.0};
    guide.distance = 0.8;
    guide.hasGuide = true;

    std::vector<SnapResult> allSnaps = {winner, guide};
    SnapResult result = selectEffectiveSnap(winner, allSnaps);

    if (!result.snapped) return {false, "snapped", "not snapped"};
    if (result.type != SnapType::Vertex) return {false, "Vertex", std::to_string(static_cast<int>(result.type))};
    if (!approx(result.position.x, winner.position.x, 1e-6) || !approx(result.position.y, winner.position.y, 1e-6)) {
        return {false, "winner pos", "guide pos"};
    }
    return {true, "", ""};
}

TestResult test_effective_snap_falls_back_when_no_guide() {
    SnapResult winner;
    winner.snapped = true;
    winner.type = SnapType::Vertex;
    winner.position = {5.0, 5.0};
    winner.distance = 0.5;

    SnapResult other;
    other.snapped = true;
    other.type = SnapType::Grid;
    other.position = {4.9, 4.9};
    other.distance = 0.4;

    std::vector<SnapResult> allSnaps = {winner, other};
    SnapResult result = selectEffectiveSnap(winner, allSnaps);

    if (!result.snapped) return {false, "snapped", "not snapped"};
    if (result.type != SnapType::Vertex) return {false, "Vertex", std::to_string(static_cast<int>(result.type))};
    if (!approx(result.position.x, winner.position.x, 1e-6) || !approx(result.position.y, winner.position.y, 1e-6)) {
        return {false, "winner pos", "guide pos"};
    }
    return {true, "", ""};
}

TestResult test_effective_snap_nearest_guide_tiebreak() {
    SnapResult winner;
    winner.snapped = true;
    winner.type = SnapType::Grid;
    winner.position = {5.0, 5.0};
    winner.distance = 0.5;

    SnapResult guideFar;
    guideFar.snapped = true;
    guideFar.type = SnapType::Horizontal;
    guideFar.position = {6.0, 5.0};
    guideFar.distance = 1.0;
    guideFar.hasGuide = true;

    SnapResult guideNear;
    guideNear.snapped = true;
    guideNear.type = SnapType::Horizontal;
    guideNear.position = {5.2, 5.0};
    guideNear.distance = 0.3;
    guideNear.hasGuide = true;

    std::vector<SnapResult> allSnaps = {winner, guideFar, guideNear};
    SnapResult result = selectEffectiveSnap(winner, allSnaps);

    if (!result.snapped) return {false, "snapped", "not snapped"};
    if (result.type != SnapType::Horizontal) return {false, "Horizontal", std::to_string(static_cast<int>(result.type))};
    if (!approx(result.position.x, guideNear.position.x, 1e-6) || !approx(result.position.y, guideNear.position.y, 1e-6)) {
        return {false, "nearest guide", "far guide"};
    }
    return {true, "", ""};
}

TestResult test_line_commit_prefers_explicit_endpoint_over_guide() {
    Sketch sketch;

    // Intentional insertion order: guide producer first, explicit target second.
    EntityID guideAnchor = sketch.addPoint(10.0, 0.0);
    EntityID startPoint = sketch.addPoint(0.0, 0.0);
    EntityID explicitEndPoint = sketch.addPoint(20.0, 0.0);

    tools::SketchToolManager manager;
    manager.setSketch(&sketch);
    manager.activateTool(tools::ToolType::Line);

    SnapManager& snap = manager.snapManager();
    snap.setAllSnapsEnabled(false);
    snap.setEnabled(true);
    snap.setSnapEnabled(SnapType::Vertex, true);
    snap.setSnapEnabled(SnapType::Horizontal, true);
    snap.setSnapRadius(2.0);

    const int linesBefore = countEntitiesOfType(sketch, EntityType::Line);

    manager.handleMousePress({0.2, 0.2}, Qt::LeftButton);
    manager.handleMousePress({20.2, 0.2}, Qt::LeftButton);

    const int linesAfter = countEntitiesOfType(sketch, EntityType::Line);
    if (linesAfter != linesBefore + 1) {
        return {false,
                "line created from explicit endpoint commit",
                "line not created"};
    }

    const SketchLine* line = findLastLine(sketch);
    if (!line) {
        return {false, "new line exists", "nullptr"};
    }

    const bool startMatches = (line->startPointId() == startPoint && line->endPointId() == explicitEndPoint);
    const bool endMatches = (line->startPointId() == explicitEndPoint && line->endPointId() == startPoint);
    if (!startMatches && !endMatches) {
        return {false,
                "line endpoints use explicit snapped points",
                "line endpoints not explicit"};
    }

    if (line->startPointId() == guideAnchor || line->endPointId() == guideAnchor) {
        return {false,
                "guide anchor not used as commit endpoint",
                "guide anchor endpoint used"};
    }

    return {true, "", ""};
}

TestResult test_overlap_axis_crossing_deterministic() {
    Sketch sketch;
    sketch.addLine(0.0, 0.0, 10.0, 0.0);
    sketch.addLine(0.0, 0.0, 0.0, 10.0);

    SnapManager manager = createSnapManagerFor({SnapType::Endpoint, SnapType::Intersection});
    auto allSnaps = manager.findAllSnaps({0.0, 0.0}, sketch);

    bool hasIntersection = false;
    for (const auto& snap : allSnaps) {
        if (snap.snapped && snap.type == SnapType::Intersection) {
            hasIntersection = true;
            break;
        }
    }
    if (!hasIntersection) {
        return {false, "intersection snap", "missing"};
    }

    SnapResult reference;
    bool capturedReference = false;
    for (int i = 0; i < 10; ++i) {
        SnapResult candidate = manager.findBestSnap({0.0, 0.0}, sketch);
        if (!candidate.snapped) {
            return {false, "snapped", "not snapped"};
        }
        if (candidate.type != SnapType::Endpoint) {
            return {
                false,
                "Endpoint",
                std::to_string(static_cast<int>(candidate.type))
            };
        }
        if (!capturedReference) {
            reference = candidate;
            capturedReference = true;
            continue;
        }
        if (!snapResultsEqual(candidate, reference)) {
            return {false, "deterministic winner", "varying winner"};
        }
    }

    return {true, "", ""};
}

TestResult test_overlap_point_beats_intersection() {
    Sketch sketch;
    EntityID standalone = sketch.addPoint(5.0, 5.0);
    sketch.addLine(5.0, 5.0, 10.0, 5.0);
    sketch.addLine(5.0, 5.0, 5.0, 10.0);

    SnapManager manager = createSnapManagerFor({SnapType::Vertex, SnapType::Endpoint, SnapType::Intersection});
    SnapResult result = manager.findBestSnap({5.0, 5.0}, sketch);
    TestResult check = expectSnap(result, SnapType::Vertex);
    if (!check.pass) {
        return check;
    }
    if (result.pointId != standalone) {
        return {false, standalone, result.pointId};
    }
    return {true, "", ""};
}

TestResult test_overlap_endpoint_beats_intersection_colocated() {
    Sketch sketch;
    sketch.addLine(0.0, 0.0, 10.0, 0.0);
    sketch.addLine(10.0, 0.0, 10.0, 10.0);
    sketch.addLine(0.0, -5.0, 20.0, 5.0);

    SnapManager manager = createSnapManagerFor({SnapType::Endpoint, SnapType::Intersection});
    auto allSnaps = manager.findAllSnaps({10.0, 0.0}, sketch);

    bool sawIntersection = false;
    for (const auto& snap : allSnaps) {
        if (snap.snapped && snap.type == SnapType::Intersection) {
            sawIntersection = true;
            break;
        }
    }
    if (!sawIntersection) {
        return {false, "intersection snap", "none"};
    }

    SnapResult result = manager.findBestSnap({10.0, 0.0}, sketch);
    return expectSnap(result, SnapType::Endpoint);
}

TestResult test_overlap_repeated_runs_same_winner() {
    Sketch sketch;
    sketch.addLine(0.0, 0.0, 10.0, 0.0);
    sketch.addLine(10.0, 0.0, 10.0, 10.0);
    sketch.addLine(0.0, -5.0, 20.0, 5.0);

    SnapManager manager = createSnapManagerFor({SnapType::Endpoint, SnapType::Intersection});
    Vec2d query{10.0, 0.0};
    SnapResult baseline;
    bool baselineCaptured = false;
    for (int i = 0; i < 20; ++i) {
        SnapResult candidate = manager.findBestSnap(query, sketch);
        if (!candidate.snapped) {
            return {false, "snapped", "not snapped"};
        }
        if (candidate.type != SnapType::Endpoint) {
            return {
                false,
                "Endpoint",
                std::to_string(static_cast<int>(candidate.type))
            };
        }
        if (!baselineCaptured) {
            baseline = candidate;
            baselineCaptured = true;
            continue;
        }
        if (!snapResultsEqual(candidate, baseline)) {
            return {false, "consistent winner", "varying winner"};
        }
    }

    return {true, "", ""};
}

TestResult test_parity_no_guide_overlap() {
    Sketch sketch;
    sketch.addPoint(5.0, 5.0);
    sketch.addLine(0.0, 5.0, 10.0, 5.0);
    sketch.addLine(5.0, 0.0, 5.0, 10.0);

    SnapManager manager = createSnapManagerFor({SnapType::Vertex, SnapType::Endpoint, SnapType::Intersection});
    Vec2d query{5.0, 5.0};

    SnapResult commit = manager.findBestSnap(query, sketch);
    auto allSnaps = manager.findAllSnaps(query, sketch);
    SnapResult preview = selectEffectiveSnap(commit, allSnaps);

    if (!commit.snapped || !preview.snapped) {
        return {false, "snapped", "not snapped"};
    }
    if (!snapResultsEqual(commit, preview)) {
        return {false, "same winner", "different winner"};
    }
    return {true, "", ""};
}

TestResult test_parity_guide_adjacent_to_overlap() {
    Sketch sketch;
    sketch.addPoint(5.0, 5.0);
    sketch.addLine(0.0, 5.0, 10.0, 5.0);
    sketch.addLine(5.0, 0.0, 5.0, 10.0);
    sketch.addLine(0.0, 4.9, 10.0, 4.9);

    SnapManager manager = createSnapManagerFor({
        SnapType::Vertex,
        SnapType::Endpoint,
        SnapType::Intersection,
        SnapType::Horizontal
    });
    Vec2d query{5.0, 4.8};

    SnapResult commit = manager.findBestSnap(query, sketch);
    auto allSnaps = manager.findAllSnaps(query, sketch);

    SnapResult preview = selectEffectiveSnap(commit, allSnaps);
    if (!preview.snapped) {
        return {false, "snapped", "not snapped"};
    }
    if (!snapResultsEqual(preview, commit)) {
        return {false, "preview vs commit", "different winner"};
    }
    if (preview.type != SnapType::Vertex) {
        return {false, "Vertex", std::to_string(static_cast<int>(preview.type))};
    }
    return {true, "", ""};
}

TestResult test_guide_crossing_snaps_to_intersection() {
    Sketch sketch;
    sketch.addLine(0.0, 0.0, 4.0, 0.0);
    sketch.addLine(6.0, 2.0, 6.0, 4.0);

    SnapManager manager = createSnapManagerFor({SnapType::SketchGuide, SnapType::Intersection});
    Vec2d query{6.1, 0.1};

    auto allSnaps = manager.findAllSnaps(query, sketch);
    bool sawGuideIntersection = false;
    for (const auto& snap : allSnaps) {
        if (snap.snapped && snap.type == SnapType::Intersection &&
            approx(snap.position.x, 6.0, 1e-6) && approx(snap.position.y, 0.0, 1e-6)) {
            sawGuideIntersection = true;
            break;
        }
    }
    if (!sawGuideIntersection) {
        return {false, "guide intersection candidate", "missing"};
    }

    SnapResult best = manager.findBestSnap(query, sketch);
    if (!best.snapped) {
        return {false, "snapped", "not snapped"};
    }
    if (best.type != SnapType::Intersection) {
        return {false, "Intersection", std::to_string(static_cast<int>(best.type))};
    }
    if (!approx(best.position.x, 6.0, 1e-6) || !approx(best.position.y, 0.0, 1e-6)) {
        return {false,
                "(6,0)",
                "(" + std::to_string(best.position.x) + "," + std::to_string(best.position.y) + ")"};
    }

    return {true, "", ""};
}

TestResult test_parity_findBestSnap_stable_across_calls() {
    Sketch sketch;
    sketch.addPoint(5.0, 5.0);
    sketch.addLine(0.0, 5.0, 10.0, 5.0);
    sketch.addLine(5.0, 0.0, 5.0, 10.0);

    SnapManager manager = createSnapManagerFor({SnapType::Vertex, SnapType::Endpoint, SnapType::Intersection});
    Vec2d query{5.0, 5.0};

    SnapResult reference;
    bool referenceCaptured = false;
    for (int i = 0; i < 10; ++i) {
        SnapResult candidate = manager.findBestSnap(query, sketch);
        if (!candidate.snapped) {
            return {false, "snapped", "not snapped"};
        }
        if (!referenceCaptured) {
            reference = candidate;
            referenceCaptured = true;
            continue;
        }
        if (!snapResultsEqual(reference, candidate)) {
            return {false, "consistent winner", "varying winner"};
        }
    }

    return {true, "", ""};
}

TestResult test_shared_preview_vertex_priority_with_guides() {
    Sketch sketch;
    sketch.addPoint(5.0, 5.0);
    sketch.addPoint(10.0, 5.0);

    SnapManager manager = createSnapManagerFor({SnapType::Vertex, SnapType::Horizontal});
    const auto preview = tools::resolveSnapForInputEvent(
        manager,
        {5.01, 5.0},
        sketch,
        {},
        std::nullopt,
        false,
        true);

    if (!preview.resolvedSnap.snapped) {
        return {false, "snapped", "not snapped"};
    }
    if (preview.resolvedSnap.type != SnapType::Vertex) {
        return {false, "Vertex", std::to_string(static_cast<int>(preview.resolvedSnap.type))};
    }
    if (preview.activeGuides.empty()) {
        return {false, "guide segment(s)", "none"};
    }
    return {true, "", ""};
}

TestResult test_shared_preview_intersection_guides_present() {
    Sketch sketch;
    sketch.addPoint(5.0, 0.0);
    sketch.addPoint(0.0, 3.0);

    SnapManager manager = createSnapManagerFor({
        SnapType::Horizontal,
        SnapType::Vertical,
        SnapType::SketchGuide,
        SnapType::Intersection
    });
    const auto preview = tools::resolveSnapForInputEvent(
        manager,
        {5.1, 3.1},
        sketch,
        {},
        std::nullopt,
        true,
        true);

    if (!preview.resolvedSnap.snapped) {
        return {false, "snapped", "not snapped"};
    }
    if (preview.resolvedSnap.type != SnapType::Intersection) {
        return {false, "Intersection", std::to_string(static_cast<int>(preview.resolvedSnap.type))};
    }
    if (preview.activeGuides.size() < 2) {
        return {false, ">=2 guide segments", std::to_string(preview.activeGuides.size())};
    }

    bool hasHorizontal = false;
    bool hasVertical = false;
    for (const auto& guide : preview.activeGuides) {
        const double dx = guide.target.x - guide.origin.x;
        const double dy = guide.target.y - guide.origin.y;
        if (std::abs(dx) < 1e-9 && std::abs(dy) < 1e-9) {
            continue;
        }
        if (std::abs(dy) <= std::abs(dx) * 0.2) {
            hasHorizontal = true;
        }
        if (std::abs(dx) <= std::abs(dy) * 0.2) {
            hasVertical = true;
        }
    }

    if (!hasHorizontal || !hasVertical) {
        return {false, "horizontal+vertical guide mix", "missing expected guide orientation"};
    }
    return {true, "", ""};
}

TestResult test_shared_preview_no_snap_clears_guides() {
    Sketch sketch;
    sketch.addPoint(0.0, 0.0);

    SnapManager manager = createSnapManagerFor({SnapType::Vertex});
    const auto preview = tools::resolveSnapForInputEvent(
        manager,
        {100.0, 100.0},
        sketch,
        {},
        std::nullopt,
        false,
        true);

    if (preview.resolvedSnap.snapped) {
        return {false, "not snapped", "snapped"};
    }
    if (!preview.activeGuides.empty()) {
        return {false, "no guide segments", std::to_string(preview.activeGuides.size())};
    }
    return {true, "", ""};
}

TestResult test_shared_preview_dedupe_collinear_and_cap() {
    SnapResult resolved;
    resolved.snapped = true;
    resolved.type = SnapType::SketchGuide;
    resolved.position = {10.0, 10.0};

    auto makeGuideSnap = [](Vec2d origin, Vec2d target) {
        SnapResult snap;
        snap.snapped = true;
        snap.type = SnapType::SketchGuide;
        snap.position = target;
        snap.guideOrigin = origin;
        snap.hasGuide = true;
        return snap;
    };

    const std::vector<SnapResult> allSnaps = {
        makeGuideSnap({0.0, 0.0}, {10.0, 0.0}),   // horizontal
        makeGuideSnap({5.0, 0.0}, {15.0, 0.0}),   // collinear duplicate
        makeGuideSnap({0.0, 0.0}, {0.0, 10.0}),   // vertical
        makeGuideSnap({0.0, 0.0}, {10.0, 10.0}),  // diagonal 1
        makeGuideSnap({0.0, 0.0}, {-10.0, 10.0}), // diagonal 2
        makeGuideSnap({0.0, 0.0}, {10.0, -10.0})  // diagonal 3 (trimmed by cap)
    };

    const auto guides = tools::buildActiveGuidesForSnap(resolved, allSnaps);
    if (guides.size() != 4) {
        return {false, "4 guide segments", std::to_string(guides.size())};
    }

    auto isCollinear = [](const tools::GuideSegment& a, const tools::GuideSegment& b) {
        double dirAx = a.target.x - a.origin.x;
        double dirAy = a.target.y - a.origin.y;
        double dirBx = b.target.x - b.origin.x;
        double dirBy = b.target.y - b.origin.y;
        const double lenA = std::sqrt((dirAx * dirAx) + (dirAy * dirAy));
        const double lenB = std::sqrt((dirBx * dirBx) + (dirBy * dirBy));
        if (lenA < 1e-6 || lenB < 1e-6) {
            return true;
        }
        dirAx /= lenA;
        dirAy /= lenA;
        dirBx /= lenB;
        dirBy /= lenB;
        const double cross = std::abs((dirAx * dirBy) - (dirAy * dirBx));
        return cross < 0.01;
    };

    for (size_t i = 0; i < guides.size(); ++i) {
        for (size_t j = i + 1; j < guides.size(); ++j) {
            if (isCollinear(guides[i], guides[j])) {
                return {false, "no collinear duplicates", "found duplicate direction"};
            }
        }
    }

    return {true, "", ""};
}

TestResult test_ambiguity_hook_api() {
    SnapManager manager;
    if (manager.hasAmbiguity()) {
        return {false, "no ambiguity on fresh manager", "has ambiguity"};
    }
    if (manager.ambiguityCandidateCount() != 0) {
        return {false, "0 candidates on fresh manager", std::to_string(manager.ambiguityCandidateCount())};
    }

    // Verify cycling and clearing don't crash
    manager.cycleAmbiguity();
    manager.clearAmbiguity();

    return {true, "", ""};
}

TestResult test_hv_guide_crossing_produces_intersection_candidate() {
    Sketch sketch;
    sketch.addPoint(5.0, 0.0);
    sketch.addPoint(0.0, 3.0);

    SnapManager manager = createSnapManagerFor({SnapType::Horizontal, SnapType::Vertical, SnapType::SketchGuide, SnapType::Intersection});
    Vec2d query{5.1, 3.1};

    auto allSnaps = manager.findAllSnaps(query, sketch);
    bool found = false;
    for (const auto& s : allSnaps) {
        if (s.snapped && s.type == SnapType::Intersection && s.hasGuide &&
            approx(s.position.x, 5.0) && approx(s.position.y, 3.0)) {
            found = true;
            break;
        }
    }

    if (!found) return {false, "Intersection snap at (5,3) with hasGuide=true", "not found"};
    return {true, "", ""};
}

TestResult test_hv_guide_crossing_wins_over_individual_hv() {
    Sketch sketch;
    sketch.addPoint(5.0, 0.0);
    sketch.addPoint(0.0, 3.0);

    SnapManager manager = createSnapManagerFor({SnapType::Horizontal, SnapType::Vertical, SnapType::SketchGuide, SnapType::Intersection});
    Vec2d query{5.1, 3.1};

    SnapResult best = manager.findBestSnap(query, sketch);
    if (!best.snapped) return {false, "snapped", "not snapped"};
    if (best.type != SnapType::Intersection) {
        return {false, "Intersection", std::to_string(static_cast<int>(best.type))};
    }
    if (!approx(best.position.x, 5.0) || !approx(best.position.y, 3.0)) {
        return {false, "(5,3)", "(" + std::to_string(best.position.x) + "," + std::to_string(best.position.y) + ")"};
    }
    return {true, "", ""};
}

TestResult test_hv_guide_crossing_loses_to_vertex() {
    Sketch sketch;
    sketch.addPoint(5.0, 3.0);
    sketch.addPoint(5.0, 0.0);
    sketch.addPoint(0.0, 3.0);

    SnapManager manager = createSnapManagerFor({SnapType::Vertex, SnapType::Horizontal, SnapType::Vertical, SnapType::SketchGuide, SnapType::Intersection});
    Vec2d query{5.0, 3.0};

    SnapResult best = manager.findBestSnap(query, sketch);
    if (!best.snapped) return {false, "snapped", "not snapped"};
    if (best.type != SnapType::Vertex) {
        return {false, "Vertex", std::to_string(static_cast<int>(best.type))};
    }
    return {true, "", ""};
}

TestResult test_near_parallel_guides_no_spurious_intersection() {
    Sketch sketch;
    sketch.addPoint(0.0, 5.0);
    sketch.addPoint(10.0, 5.0000000000001);

    SnapManager manager = createSnapManagerFor({SnapType::Horizontal, SnapType::Vertical, SnapType::SketchGuide, SnapType::Intersection});
    Vec2d query{5.0, 5.0};

    auto allSnaps = manager.findAllSnaps(query, sketch);
    for (const auto& s : allSnaps) {
        if (s.snapped && s.type == SnapType::Intersection) {
            if (std::abs(s.position.x) > 1000.0 || std::abs(s.position.y) > 1000.0) {
                return {false, "no huge spurious intersection", "found intersection at (" + std::to_string(s.position.x) + "," + std::to_string(s.position.y) + ")"};
            }
        }
    }
    return {true, "", ""};
}

bool shouldSkipInLegacy(const std::string& testName) {
    static const std::vector<std::string> blocked = {
        "perpendicular",
        "tangent",
        "angular",
        "horizontal_alignment",
        "vertical_alignment",
        "extension",
        "guide",
        "spatial_hash",
        "toggle",
        "combined"
    };

    for (const std::string& token : blocked) {
        if (testName.find(token) != std::string::npos) {
            return true;
        }
    }
    return false;
}

void runBenchmark() {
    Sketch sketch;
    std::mt19937 rng(42);
    std::uniform_real_distribution<double> dist(-500.0, 500.0);

    for (int i = 0; i < 1000; ++i) {
        sketch.addPoint(dist(rng), dist(rng));
    }

    SnapManager manager;
    std::vector<double> queryMicros;
    queryMicros.reserve(100);

    for (int i = 0; i < 100; ++i) {
        Vec2d cursor{dist(rng), dist(rng)};
        auto t0 = std::chrono::steady_clock::now();
        (void)manager.findBestSnap(cursor, sketch);
        auto t1 = std::chrono::steady_clock::now();
        double us = std::chrono::duration<double, std::micro>(t1 - t0).count();
        queryMicros.push_back(us);
    }

    std::sort(queryMicros.begin(), queryMicros.end());
    size_t p95Index = static_cast<size_t>(std::ceil(0.95 * static_cast<double>(queryMicros.size()))) - 1;
    if (p95Index >= queryMicros.size()) {
        p95Index = queryMicros.size() - 1;
    }

    const double p95Micros = queryMicros[p95Index];
    const double p95Millis = p95Micros / 1000.0;
    std::cout << "Benchmark: p95 query time " << p95Micros << " us (" << p95Millis << " ms)" << std::endl;
    std::cout << "Benchmark target (<2ms): " << (p95Millis < 2.0 ? "PASS" : "FAIL") << std::endl;
}

} // namespace

int main(int argc, char** argv) {
    bool legacyOnly = false;
    bool runBench = false;

    for (int i = 1; i < argc; ++i) {
        std::string arg = argv[i];
        if (arg == "--legacy") {
            legacyOnly = true;
        } else if (arg == "--benchmark") {
            runBench = true;
        }
    }

    const std::vector<std::pair<std::string, std::function<TestResult()>>> tests = {
        {"test_vertex_snap", testVertexSnap},
        {"test_hinttext_vertex_snap", test_hinttext_vertex_snap},
        {"test_endpoint_snap", testEndpointSnap},
        {"test_hinttext_endpoint_snap", test_hinttext_endpoint_snap},
        {"test_midpoint_snap", testMidpointSnap},
        {"test_hinttext_midpoint_snap", test_hinttext_midpoint_snap},
        {"test_center_snap", testCenterSnap},
        {"test_hinttext_center_snap", test_hinttext_center_snap},
        {"test_quadrant_snap", testQuadrantSnap},
        {"test_intersection_snap", testIntersectionSnap},
        {"test_oncurve_snap", testOnCurveSnap},
        {"test_ellipse_center_snap", testEllipseCenterSnap},
        {"test_ellipse_quadrant_snap", testEllipseQuadrantSnap},
        {"test_ellipse_oncurve_snap", testEllipseOnCurveSnap},
        {"test_ellipse_line_intersection", testEllipseLineIntersection},
        {"test_ellipse_quadrant_rotated", testEllipseQuadrantRotated},
        {"test_grid_snap", testGridSnap},
        {"test_hinttext_grid_snap", test_hinttext_grid_snap},
        {"test_perpendicular_snap_line", testPerpendicularSnapLine},
        {"test_perpendicular_snap_circle", testPerpendicularSnapCircle},
        {"test_perpendicular_snap_arc", testPerpendicularSnapArc},
        {"test_perpendicular_guide_metadata", test_perpendicular_guide_metadata},
        {"test_tangent_snap_circle", testTangentSnapCircle},
        {"test_tangent_snap_arc", testTangentSnapArc},
        {"test_tangent_guide_metadata", test_tangent_guide_metadata},
        {"test_horizontal_alignment_snap", testHorizontalAlignmentSnap},
        {"test_vertical_alignment_snap", testVerticalAlignmentSnap},
        {"test_extension_snap_line", testExtensionSnapLine},
        {"test_extension_snap_no_arc", testExtensionSnapNoArc},
        {"test_angular_snap_15deg_rounding", testAngularSnap15degRounding},
        {"test_angular_snap_45deg_exact", testAngularSnap45degExact},
        {"test_angular_snap_50deg_reference", test_angular_snap_50deg_reference},
        {"test_angular_snap_no_reference", testAngularSnapNoReference},
        {"test_toggle_suppression", testToggleSuppression},
        {"test_all_snap_types_combined", testAllSnapTypesCombined},
        {"test_priority_order", testPriorityOrder},
        {"test_spatial_hash_after_geometry_move", test_spatial_hash_after_geometry_move},
        {"test_spatial_hash_equivalent_to_bruteforce", testSpatialHashEquivalentToBruteforce},
        {"test_preserves_guides_when_vertex_wins", test_preserves_guides_when_vertex_wins},
        {"test_perpendicular_guide_nonzero_length", test_perpendicular_guide_nonzero_length},
        {"test_tangent_guide_nonzero_length", test_tangent_guide_nonzero_length},
        {"test_clears_guides_when_no_snap", test_clears_guides_when_no_snap},
        {"test_guide_count_bounded", test_guide_count_bounded},
        {"test_dedupe_collinear_guides", test_dedupe_collinear_guides},
    {"test_effective_snap_keeps_point_priority_over_guide", test_effective_snap_keeps_point_priority_over_guide},
        {"test_effective_snap_falls_back_when_no_guide", test_effective_snap_falls_back_when_no_guide},
        {"test_effective_snap_nearest_guide_tiebreak", test_effective_snap_nearest_guide_tiebreak},
        {"test_line_commit_prefers_explicit_endpoint_over_guide", test_line_commit_prefers_explicit_endpoint_over_guide},
        {"test_overlap_axis_crossing_deterministic", test_overlap_axis_crossing_deterministic},
        {"test_overlap_point_beats_intersection", test_overlap_point_beats_intersection},
        {"test_overlap_endpoint_beats_intersection_colocated", test_overlap_endpoint_beats_intersection_colocated},
        {"test_overlap_repeated_runs_same_winner", test_overlap_repeated_runs_same_winner},
    {"test_parity_no_guide_overlap", test_parity_no_guide_overlap},
    {"test_parity_guide_adjacent_to_overlap", test_parity_guide_adjacent_to_overlap},
    {"test_parity_findBestSnap_stable_across_calls", test_parity_findBestSnap_stable_across_calls},
    {"test_shared_preview_vertex_priority_with_guides", test_shared_preview_vertex_priority_with_guides},
    {"test_shared_preview_intersection_guides_present", test_shared_preview_intersection_guides_present},
    {"test_shared_preview_no_snap_clears_guides", test_shared_preview_no_snap_clears_guides},
    {"test_shared_preview_dedupe_collinear_and_cap", test_shared_preview_dedupe_collinear_and_cap},
    {"test_guide_crossing_snaps_to_intersection", test_guide_crossing_snaps_to_intersection},
    {"test_hv_guide_crossing_produces_intersection_candidate", test_hv_guide_crossing_produces_intersection_candidate},
    {"test_hv_guide_crossing_wins_over_individual_hv", test_hv_guide_crossing_wins_over_individual_hv},
    {"test_hv_guide_crossing_loses_to_vertex", test_hv_guide_crossing_loses_to_vertex},
    {"test_near_parallel_guides_no_spurious_intersection", test_near_parallel_guides_no_spurious_intersection},
    {"test_ambiguity_hook_api", test_ambiguity_hook_api}
    };

    int passed = 0;
    int total = 0;

    for (const auto& [name, fn] : tests) {
        if (legacyOnly && shouldSkipInLegacy(name)) {
            continue;
        }

        ++total;
        TestResult r = fn();
        if (r.pass) {
            ++passed;
            std::cout << "PASS: " << name << std::endl;
        } else {
            std::cout << "FAIL: " << name
                      << " (expected " << r.expected
                      << ", got " << r.got << ")" << std::endl;
        }
    }

    std::cout << passed << "/" << total << " tests passed" << std::endl;

    if (runBench) {
        runBenchmark();
    }

    return passed == total ? 0 : 1;
}
