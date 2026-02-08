#include "sketch/SnapManager.h"
#include "sketch/Sketch.h"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <functional>
#include <iostream>
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

TestResult testEndpointSnap() {
    Sketch sketch = createTestSketch();
    SnapManager manager = createSnapManagerFor({SnapType::Endpoint});
    SnapResult result = manager.findBestSnap({10.3, 0.2}, sketch);
    return expectSnap(result, SnapType::Endpoint);
}

TestResult testMidpointSnap() {
    Sketch sketch = createTestSketch();
    SnapManager manager = createSnapManagerFor({SnapType::Midpoint});
    SnapResult result = manager.findBestSnap({5.2, 0.1}, sketch);
    return expectSnap(result, SnapType::Midpoint);
}

TestResult testCenterSnap() {
    Sketch sketch = createTestSketch();
    SnapManager manager = createSnapManagerFor({SnapType::Center});
    SnapResult result = manager.findBestSnap({20.3, 20.2}, sketch);
    return expectSnap(result, SnapType::Center);
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

TestResult testPriorityOrder() {
    Sketch sketch = createTestSketch();
    SnapManager manager = createSnapManagerFor({SnapType::Vertex, SnapType::Endpoint});
    SnapResult result = manager.findBestSnap({0.1, 0.1}, sketch);
    return expectSnap(result, SnapType::Vertex);
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

bool shouldSkipInLegacy(const std::string& testName) {
    static const std::vector<std::string> blocked = {
        "perpendicular",
        "tangent",
        "angular",
        "horizontal_alignment",
        "vertical_alignment",
        "extension",
        "spatial_hash"
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
        {"test_endpoint_snap", testEndpointSnap},
        {"test_midpoint_snap", testMidpointSnap},
        {"test_center_snap", testCenterSnap},
        {"test_quadrant_snap", testQuadrantSnap},
        {"test_intersection_snap", testIntersectionSnap},
        {"test_oncurve_snap", testOnCurveSnap},
        {"test_ellipse_center_snap", testEllipseCenterSnap},
        {"test_ellipse_quadrant_snap", testEllipseQuadrantSnap},
        {"test_ellipse_oncurve_snap", testEllipseOnCurveSnap},
        {"test_ellipse_line_intersection", testEllipseLineIntersection},
        {"test_ellipse_quadrant_rotated", testEllipseQuadrantRotated},
        {"test_grid_snap", testGridSnap},
        {"test_perpendicular_snap_line", testPerpendicularSnapLine},
        {"test_perpendicular_snap_circle", testPerpendicularSnapCircle},
        {"test_perpendicular_snap_arc", testPerpendicularSnapArc},
        {"test_tangent_snap_circle", testTangentSnapCircle},
        {"test_tangent_snap_arc", testTangentSnapArc},
        {"test_priority_order", testPriorityOrder},
        {"test_spatial_hash_equivalent_to_bruteforce", testSpatialHashEquivalentToBruteforce}
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
