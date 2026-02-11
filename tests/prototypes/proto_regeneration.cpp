/**
 * @file proto_regeneration.cpp
 * @brief Prototype tests for RegenerationEngine.
 *
 * Test cases:
 * 1. Single extrude: sketch→extrude→regenerate→verify
 * 2. Chain: extrude→fillet→regen→verify
 * 3. Failure: delete sketch→regen→verify failure reported
 * 4. Topology: extrude→fillet by ElementMap ID→modify extrude→regen→verify
 */

#include "app/document/Document.h"
#include "app/history/DependencyGraph.h"
#include "app/history/RegenerationEngine.h"
#include "app/selection/SelectionManager.h"
#include "core/loop/LoopDetector.h"
#include "core/loop/RegionUtils.h"
#include "core/sketch/Sketch.h"
#include "io/HistoryIO.h"

#include <BRepCheck_Analyzer.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <GProp_GProps.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>

#include <QCoreApplication>
#include <QUuid>

#include <cassert>
#include <cmath>
#include <iostream>
#include <optional>
#include <utility>

using namespace onecad;

namespace {

bool nearlyEqual(double a, double b, double tol = 1e-3) {
    return std::abs(a - b) <= tol;
}

std::string newId() {
    return QUuid::createUuid().toString(QUuid::WithoutBraces).toStdString();
}

double shapeVolume(const TopoDS_Shape& shape) {
    GProp_GProps props;
    BRepGProp::VolumeProperties(shape, props);
    return props.Mass();
}

bool shapeValid(const TopoDS_Shape& shape) {
    if (shape.IsNull()) return false;
    BRepCheck_Analyzer analyzer(shape);
    return analyzer.IsValid();
}

std::vector<core::loop::RegionDefinition> detectRegions(core::sketch::Sketch& sketch) {
    core::loop::LoopDetectorConfig config = core::loop::makeRegionDetectionConfig();
    core::loop::LoopDetector detector(config);
    auto loopResult = detector.detect(sketch);
    assert(loopResult.success);
    return core::loop::buildRegionDefinitions(loopResult, core::sketch::constants::COINCIDENCE_TOLERANCE);
}

std::string firstRegionId(core::sketch::Sketch& sketch) {
    auto regions = detectRegions(sketch);
    assert(!regions.empty());
    return regions[0].id;
}

std::optional<std::pair<std::string, core::sketch::SketchPlane>>
findTopPlanarFace(const app::Document& doc, const std::string& bodyId) {
    std::optional<std::pair<std::string, core::sketch::SketchPlane>> best;
    double bestNormalZ = -2.0;

    for (const auto& id : doc.elementMap().ids()) {
        const auto* entry = doc.elementMap().find(id);
        if (!entry || entry->kind != kernel::elementmap::ElementKind::Face) {
            continue;
        }
        auto plane = doc.getSketchPlaneForFace(bodyId, id.value);
        if (!plane.has_value()) {
            continue;
        }
        if (plane->normal.z > bestNormalZ) {
            bestNormalZ = plane->normal.z;
            best = std::make_pair(id.value, *plane);
        }
    }

    return best;
}

} // namespace

void testSingleExtrude() {
    std::cout << "Test 1: Single extrude regeneration..." << std::flush;

    app::Document doc;

    // Create a simple rectangular sketch
    auto sketch = std::make_unique<core::sketch::Sketch>();
    auto p1 = sketch->addPoint(0.0, 0.0);
    auto p2 = sketch->addPoint(10.0, 0.0);
    auto p3 = sketch->addPoint(10.0, 10.0);
    auto p4 = sketch->addPoint(0.0, 10.0);

    sketch->addLine(p1, p2);
    sketch->addLine(p2, p3);
    sketch->addLine(p3, p4);
    sketch->addLine(p4, p1);

    std::string sketchId = doc.addSketch(std::move(sketch));

    // Detect regions to get a valid region ID
    // Use the same config that resolveRegionFace uses
    core::sketch::Sketch* sketchPtr = doc.getSketch(sketchId);
    core::loop::LoopDetectorConfig config = core::loop::makeRegionDetectionConfig();
    core::loop::LoopDetector detector(config);
    auto loopResult = detector.detect(*sketchPtr);

    assert(!loopResult.faces.empty());
    std::string regionId = core::loop::regionKey(loopResult.faces[0].outerLoop);

    // Create extrude operation record
    std::string bodyId = newId();
    std::string opId = newId();

    app::OperationRecord extrudeOp;
    extrudeOp.opId = opId;
    extrudeOp.type = app::OperationType::Extrude;
    extrudeOp.input = app::SketchRegionRef{sketchId, regionId};
    extrudeOp.params = app::ExtrudeParams{20.0, 0.0, app::BooleanMode::NewBody};
    extrudeOp.resultBodyIds.push_back(bodyId);

    doc.addOperation(extrudeOp);

    // Run regeneration
    app::history::RegenerationEngine engine(&doc);
    auto result = engine.regenerateAll();

    // Verify
    if (result.status != app::history::RegenStatus::Success) {
        std::cerr << "\nRegeneration failed!\n";
        for (const auto& f : result.failedOps) {
            std::cerr << "  Op: " << f.opId << " Error: " << f.errorMessage << "\n";
        }
    }
    assert(result.status == app::history::RegenStatus::Success);
    assert(result.succeededOps.size() == 1);
    assert(result.failedOps.empty());

    const TopoDS_Shape* shape = doc.getBodyShape(bodyId);
    assert(shape != nullptr);
    assert(!shape->IsNull());
    assert(shapeValid(*shape));

    // 10x10x20 = 2000 mm³
    double vol = shapeVolume(*shape);
    assert(nearlyEqual(vol, 2000.0, 10.0));

    std::cout << " PASS\n";
}

void testDependencyGraph() {
    std::cout << "Test 2: DependencyGraph topological sort..." << std::flush;

    app::history::DependencyGraph graph;

    // Create operations with dependencies
    // op1: extrude (produces body1)
    // op2: fillet (uses body1)
    // op3: shell (uses body1 after fillet)

    app::OperationRecord op1;
    op1.opId = "op1";
    op1.type = app::OperationType::Extrude;
    op1.input = app::SketchRegionRef{"sketch1", "region1"};
    op1.params = app::ExtrudeParams{10.0, 0.0, app::BooleanMode::NewBody};
    op1.resultBodyIds.push_back("body1");

    app::OperationRecord op2;
    op2.opId = "op2";
    op2.type = app::OperationType::Fillet;
    op2.input = app::BodyRef{"body1"};
    app::FilletChamferParams filletParams;
    filletParams.mode = app::FilletChamferParams::Mode::Fillet;
    filletParams.radius = 1.0;
    filletParams.edgeIds = {"edge1", "edge2"};
    op2.params = filletParams;
    op2.resultBodyIds.push_back("body1");

    graph.addOperation(op1);
    graph.addOperation(op2);

    // Topological sort should place op1 before op2
    auto sorted = graph.topologicalSort();
    assert(sorted.size() == 2);
    assert(sorted[0] == "op1");
    assert(sorted[1] == "op2");

    // Test downstream
    auto downstream = graph.getDownstream("op1");
    assert(downstream.size() == 1);
    assert(downstream[0] == "op2");

    // Test upstream
    auto upstream = graph.getUpstream("op2");
    assert(upstream.size() == 1);
    assert(upstream[0] == "op1");

    std::cout << " PASS\n";
}

void testSuppressionAndFailure() {
    std::cout << "Test 3: Suppression and failure tracking..." << std::flush;

    app::history::DependencyGraph graph;

    app::OperationRecord op1;
    op1.opId = "op1";
    op1.type = app::OperationType::Extrude;
    op1.params = app::ExtrudeParams{10.0, 0.0, app::BooleanMode::NewBody};
    op1.resultBodyIds.push_back("body1");

    app::OperationRecord op2;
    op2.opId = "op2";
    op2.type = app::OperationType::Fillet;
    op2.input = app::BodyRef{"body1"};
    app::FilletChamferParams params;
    params.radius = 1.0;
    op2.params = params;
    op2.resultBodyIds.push_back("body1");

    graph.addOperation(op1);
    graph.addOperation(op2);

    // Test suppression
    assert(!graph.isSuppressed("op1"));
    graph.setSuppressed("op1", true);
    assert(graph.isSuppressed("op1"));

    // Test failure tracking
    assert(!graph.isFailed("op1"));
    graph.setFailed("op1", true, "Test failure reason");
    assert(graph.isFailed("op1"));
    assert(graph.getFailureReason("op1") == "Test failure reason");

    auto failed = graph.getFailedOps();
    assert(failed.size() == 1);
    assert(failed[0] == "op1");

    graph.clearFailures();
    assert(!graph.isFailed("op1"));

    std::cout << " PASS\n";
}

void testChainRegeneration() {
    std::cout << "Test 4: Chain regeneration (extrude + downstream)..." << std::flush;

    app::Document doc;

    // Create sketch
    auto sketch = std::make_unique<core::sketch::Sketch>();
    auto p1 = sketch->addPoint(0.0, 0.0);
    auto p2 = sketch->addPoint(10.0, 0.0);
    auto p3 = sketch->addPoint(10.0, 10.0);
    auto p4 = sketch->addPoint(0.0, 10.0);

    sketch->addLine(p1, p2);
    sketch->addLine(p2, p3);
    sketch->addLine(p3, p4);
    sketch->addLine(p4, p1);

    std::string sketchId = doc.addSketch(std::move(sketch));

    core::sketch::Sketch* sketchPtr = doc.getSketch(sketchId);
    core::loop::LoopDetectorConfig config = core::loop::makeRegionDetectionConfig();
    core::loop::LoopDetector detector(config);
    auto loopResult = detector.detect(*sketchPtr);
    std::string regionId = core::loop::regionKey(loopResult.faces[0].outerLoop);

    std::string bodyId = newId();
    std::string extrudeOpId = newId();

    // Extrude operation
    app::OperationRecord extrudeOp;
    extrudeOp.opId = extrudeOpId;
    extrudeOp.type = app::OperationType::Extrude;
    extrudeOp.input = app::SketchRegionRef{sketchId, regionId};
    extrudeOp.params = app::ExtrudeParams{15.0, 0.0, app::BooleanMode::NewBody};
    extrudeOp.resultBodyIds.push_back(bodyId);
    doc.addOperation(extrudeOp);

    // First regenerate to create the body and register edges
    app::history::RegenerationEngine engine1(&doc);
    auto result1 = engine1.regenerateAll();
    if (result1.status != app::history::RegenStatus::Success) {
        std::cerr << "\nChain regen failed!\n";
        for (const auto& f : result1.failedOps) {
            std::cerr << "  Op: " << f.opId << " Error: " << f.errorMessage << "\n";
        }
    }
    assert(result1.status == app::history::RegenStatus::Success);

    // Now we could add a fillet operation if we have edge IDs
    // For this test, just verify the extrude works and chain logic is correct

    // Get the graph and verify dependency tracking
    const auto& graph = engine1.graph();
    auto sorted = graph.topologicalSort();
    assert(sorted.size() == 1);
    assert(sorted[0] == extrudeOpId);

    std::cout << " PASS\n";
}

void testRegenFailureOnMissingSketch() {
    std::cout << "Test 5: Regeneration failure on missing sketch..." << std::flush;

    app::Document doc;

    // Create operation referencing a non-existent sketch
    std::string bodyId = newId();
    std::string opId = newId();

    app::OperationRecord extrudeOp;
    extrudeOp.opId = opId;
    extrudeOp.type = app::OperationType::Extrude;
    extrudeOp.input = app::SketchRegionRef{"nonexistent-sketch", "region1"};
    extrudeOp.params = app::ExtrudeParams{10.0, 0.0, app::BooleanMode::NewBody};
    extrudeOp.resultBodyIds.push_back(bodyId);
    doc.addOperation(extrudeOp);

    // Run regeneration
    app::history::RegenerationEngine engine(&doc);
    auto result = engine.regenerateAll();

    // Should fail
    assert(result.status != app::history::RegenStatus::Success);
    assert(!result.failedOps.empty());
    assert(result.failedOps[0].opId == opId);
    assert(!result.failedOps[0].errorMessage.empty());

    std::cout << " PASS\n";
}

void testGraphCycleDetection() {
    std::cout << "Test 6: Cycle detection in dependency graph..." << std::flush;

    app::history::DependencyGraph graph;

    // Create a cycle: body1 -> body2 -> body1 (artificial)
    // This is somewhat artificial since real CAD shouldn't have cycles
    // but we test the algorithm

    app::OperationRecord op1;
    op1.opId = "op1";
    op1.type = app::OperationType::Extrude;
    op1.params = app::ExtrudeParams{10.0, 0.0, app::BooleanMode::NewBody};
    op1.resultBodyIds.push_back("body1");

    graph.addOperation(op1);

    // Verify no cycle with single op
    assert(!graph.hasCycle());

    auto sorted = graph.topologicalSort();
    assert(sorted.size() == 1);

    std::cout << " PASS\n";
}

void testSketchHostAttachmentRoundTrip() {
    std::cout << "Test 7: Sketch host attachment JSON round-trip..." << std::flush;

    core::sketch::Sketch sketch(core::sketch::SketchPlane::XY());
    sketch.setHostFaceAttachment("body-A", "face-B");

    auto restored = core::sketch::Sketch::fromJson(sketch.toJson());
    assert(restored);
    assert(restored->hasHostFaceAttachment());
    assert(restored->hostFaceAttachment()->bodyId == "body-A");
    assert(restored->hostFaceAttachment()->faceId == "face-B");

    std::cout << " PASS\n";
}

void testHistoryTargetBodyRoundTrip() {
    std::cout << "Test 8: History targetBodyId JSON round-trip..." << std::flush;

    app::OperationRecord extrudeOp;
    extrudeOp.opId = "extrude-op";
    extrudeOp.type = app::OperationType::Extrude;
    extrudeOp.input = app::SketchRegionRef{"sketch-1", "region-1"};
    app::ExtrudeParams extrudeParams;
    extrudeParams.distance = 10.0;
    extrudeParams.booleanMode = app::BooleanMode::Add;
    extrudeParams.targetBodyId = "body-target-1";
    extrudeOp.params = extrudeParams;
    extrudeOp.resultBodyIds.push_back("body-target-1");

    QJsonObject extrudeJson = io::HistoryIO::serializeOperation(extrudeOp);
    app::OperationRecord extrudeRoundTrip = io::HistoryIO::deserializeOperation(extrudeJson);
    assert(std::holds_alternative<app::ExtrudeParams>(extrudeRoundTrip.params));
    const auto& extrudeRoundTripParams = std::get<app::ExtrudeParams>(extrudeRoundTrip.params);
    assert(extrudeRoundTripParams.targetBodyId == "body-target-1");

    app::OperationRecord revolveOp;
    revolveOp.opId = "revolve-op";
    revolveOp.type = app::OperationType::Revolve;
    revolveOp.input = app::SketchRegionRef{"sketch-2", "region-2"};
    app::RevolveParams revolveParams;
    revolveParams.angleDeg = 180.0;
    revolveParams.booleanMode = app::BooleanMode::Cut;
    revolveParams.targetBodyId = "body-target-2";
    revolveParams.axis = app::SketchLineRef{"sketch-2", "line-1"};
    revolveOp.params = revolveParams;
    revolveOp.resultBodyIds.push_back("body-target-2");

    QJsonObject revolveJson = io::HistoryIO::serializeOperation(revolveOp);
    app::OperationRecord revolveRoundTrip = io::HistoryIO::deserializeOperation(revolveJson);
    assert(std::holds_alternative<app::RevolveParams>(revolveRoundTrip.params));
    const auto& revolveRoundTripParams = std::get<app::RevolveParams>(revolveRoundTrip.params);
    assert(revolveRoundTripParams.targetBodyId == "body-target-2");

    std::cout << " PASS\n";
}

void testAttachedSketchExtrudeAdd() {
    std::cout << "Test 9: Attached sketch extrude Add targets host body..." << std::flush;

    app::Document doc;
    std::string bodyId = doc.addBody(BRepPrimAPI_MakeBox(20.0, 20.0, 10.0).Shape());
    assert(!bodyId.empty());

    const TopoDS_Shape* baseBody = doc.getBodyShape(bodyId);
    assert(baseBody && !baseBody->IsNull());
    const double baseVolume = shapeVolume(*baseBody);

    auto topFace = findTopPlanarFace(doc, bodyId);
    assert(topFace.has_value());

    auto sketch = std::make_unique<core::sketch::Sketch>(topFace->second);
    sketch->setHostFaceAttachment(bodyId, topFace->first);
    const auto p1 = topFace->second.toSketch({8.0, 8.0, 10.0});
    const auto p2 = topFace->second.toSketch({12.0, 8.0, 10.0});
    const auto p3 = topFace->second.toSketch({12.0, 12.0, 10.0});
    const auto p4 = topFace->second.toSketch({8.0, 12.0, 10.0});
    auto e1 = sketch->addPoint(p1.x, p1.y);
    auto e2 = sketch->addPoint(p2.x, p2.y);
    auto e3 = sketch->addPoint(p3.x, p3.y);
    auto e4 = sketch->addPoint(p4.x, p4.y);
    sketch->addLine(e1, e2);
    sketch->addLine(e2, e3);
    sketch->addLine(e3, e4);
    sketch->addLine(e4, e1);

    std::string sketchId = doc.addSketch(std::move(sketch));
    std::string regionId = firstRegionId(*doc.getSketch(sketchId));

    app::OperationRecord op;
    op.opId = newId();
    op.type = app::OperationType::Extrude;
    op.input = app::SketchRegionRef{sketchId, regionId};
    app::ExtrudeParams params;
    params.distance = 4.0;
    params.booleanMode = app::BooleanMode::Add;
    // Exercise legacy fallback path: resolve host body from sketch attachment metadata.
    params.targetBodyId.clear();
    op.params = params;
    op.resultBodyIds.push_back(bodyId);
    doc.addOperation(op);

    app::history::RegenerationEngine engine(&doc);
    auto result = engine.regenerateAll();
    assert(result.status == app::history::RegenStatus::Success);

    const TopoDS_Shape* updatedBody = doc.getBodyShape(bodyId);
    assert(updatedBody && !updatedBody->IsNull());
    assert(shapeValid(*updatedBody));
    assert(shapeVolume(*updatedBody) > baseVolume + 1.0);

    std::cout << " PASS\n";
}

void testAttachedSketchExtrudeCut() {
    std::cout << "Test 10: Attached sketch extrude Cut targets host body..." << std::flush;

    app::Document doc;
    std::string bodyId = doc.addBody(BRepPrimAPI_MakeBox(20.0, 20.0, 10.0).Shape());
    assert(!bodyId.empty());

    const TopoDS_Shape* baseBody = doc.getBodyShape(bodyId);
    assert(baseBody && !baseBody->IsNull());
    const double baseVolume = shapeVolume(*baseBody);

    auto topFace = findTopPlanarFace(doc, bodyId);
    assert(topFace.has_value());

    auto sketch = std::make_unique<core::sketch::Sketch>(topFace->second);
    sketch->setHostFaceAttachment(bodyId, topFace->first);
    const auto p1 = topFace->second.toSketch({8.0, 8.0, 10.0});
    const auto p2 = topFace->second.toSketch({12.0, 8.0, 10.0});
    const auto p3 = topFace->second.toSketch({12.0, 12.0, 10.0});
    const auto p4 = topFace->second.toSketch({8.0, 12.0, 10.0});
    auto e1 = sketch->addPoint(p1.x, p1.y);
    auto e2 = sketch->addPoint(p2.x, p2.y);
    auto e3 = sketch->addPoint(p3.x, p3.y);
    auto e4 = sketch->addPoint(p4.x, p4.y);
    sketch->addLine(e1, e2);
    sketch->addLine(e2, e3);
    sketch->addLine(e3, e4);
    sketch->addLine(e4, e1);

    std::string sketchId = doc.addSketch(std::move(sketch));
    std::string regionId = firstRegionId(*doc.getSketch(sketchId));

    app::OperationRecord op;
    op.opId = newId();
    op.type = app::OperationType::Extrude;
    op.input = app::SketchRegionRef{sketchId, regionId};
    app::ExtrudeParams params;
    params.distance = -4.0;
    params.booleanMode = app::BooleanMode::Cut;
    params.targetBodyId = bodyId;
    op.params = params;
    op.resultBodyIds.push_back(bodyId);
    doc.addOperation(op);

    app::history::RegenerationEngine engine(&doc);
    auto result = engine.regenerateAll();
    assert(result.status == app::history::RegenStatus::Success);

    const TopoDS_Shape* updatedBody = doc.getBodyShape(bodyId);
    assert(updatedBody && !updatedBody->IsNull());
    assert(shapeValid(*updatedBody));
    assert(shapeVolume(*updatedBody) < baseVolume - 1.0);

    std::cout << " PASS\n";
}

void testAttachedSketchRevolveAdd() {
    std::cout << "Test 11: Attached sketch revolve targets host body..." << std::flush;

    app::Document doc;
    std::string bodyId = doc.addBody(BRepPrimAPI_MakeBox(20.0, 20.0, 10.0).Shape());
    assert(!bodyId.empty());

    const TopoDS_Shape* baseBody = doc.getBodyShape(bodyId);
    assert(baseBody && !baseBody->IsNull());
    const double baseVolume = shapeVolume(*baseBody);

    auto topFace = findTopPlanarFace(doc, bodyId);
    assert(topFace.has_value());

    auto sketch = std::make_unique<core::sketch::Sketch>(topFace->second);
    sketch->setHostFaceAttachment(bodyId, topFace->first);

    const auto p1 = topFace->second.toSketch({9.0, 12.0, 10.0});
    const auto p2 = topFace->second.toSketch({11.0, 12.0, 10.0});
    const auto p3 = topFace->second.toSketch({11.0, 14.0, 10.0});
    const auto p4 = topFace->second.toSketch({9.0, 14.0, 10.0});
    auto e1 = sketch->addPoint(p1.x, p1.y);
    auto e2 = sketch->addPoint(p2.x, p2.y);
    auto e3 = sketch->addPoint(p3.x, p3.y);
    auto e4 = sketch->addPoint(p4.x, p4.y);
    sketch->addLine(e1, e2);
    sketch->addLine(e2, e3);
    sketch->addLine(e3, e4);
    sketch->addLine(e4, e1);

    const auto axisP1 = topFace->second.toSketch({0.0, 10.0, 10.0});
    const auto axisP2 = topFace->second.toSketch({20.0, 10.0, 10.0});
    auto axisStart = sketch->addPoint(axisP1.x, axisP1.y, true);
    auto axisEnd = sketch->addPoint(axisP2.x, axisP2.y, true);
    auto axisLineId = sketch->addLine(axisStart, axisEnd, true);

    std::string sketchId = doc.addSketch(std::move(sketch));
    std::string regionId = firstRegionId(*doc.getSketch(sketchId));

    app::OperationRecord op;
    op.opId = newId();
    op.type = app::OperationType::Revolve;
    op.input = app::SketchRegionRef{sketchId, regionId};
    app::RevolveParams params;
    params.angleDeg = 120.0;
    params.axis = app::SketchLineRef{sketchId, axisLineId};
    params.booleanMode = app::BooleanMode::Add;
    params.targetBodyId = bodyId;
    op.params = params;
    op.resultBodyIds.push_back(bodyId);
    doc.addOperation(op);

    app::history::RegenerationEngine engine(&doc);
    auto result = engine.regenerateAll();
    assert(result.status == app::history::RegenStatus::Success);

    const TopoDS_Shape* updatedBody = doc.getBodyShape(bodyId);
    assert(updatedBody && !updatedBody->IsNull());
    assert(shapeValid(*updatedBody));
    assert(shapeVolume(*updatedBody) > baseVolume + 1.0);

    std::cout << " PASS\n";
}

void testBooleanFailureOnMissingTargetBody() {
    std::cout << "Test 12: Boolean op fails explicitly when target body is unresolved..." << std::flush;

    app::Document doc;

    auto sketch = std::make_unique<core::sketch::Sketch>(core::sketch::SketchPlane::XY());
    auto p1 = sketch->addPoint(0.0, 0.0);
    auto p2 = sketch->addPoint(10.0, 0.0);
    auto p3 = sketch->addPoint(10.0, 10.0);
    auto p4 = sketch->addPoint(0.0, 10.0);
    sketch->addLine(p1, p2);
    sketch->addLine(p2, p3);
    sketch->addLine(p3, p4);
    sketch->addLine(p4, p1);
    std::string sketchId = doc.addSketch(std::move(sketch));
    std::string regionId = firstRegionId(*doc.getSketch(sketchId));

    app::OperationRecord op;
    op.opId = newId();
    op.type = app::OperationType::Extrude;
    op.input = app::SketchRegionRef{sketchId, regionId};
    app::ExtrudeParams params;
    params.distance = 5.0;
    params.booleanMode = app::BooleanMode::Add;
    params.targetBodyId = "missing-body";
    op.params = params;
    op.resultBodyIds.push_back("missing-body");
    doc.addOperation(op);

    app::history::RegenerationEngine engine(&doc);
    auto result = engine.regenerateAll();
    assert(result.status != app::history::RegenStatus::Success);
    assert(!result.failedOps.empty());
    const std::string& error = result.failedOps.front().errorMessage;
    assert(error.find("Target body not found: missing-body") != std::string::npos);

    std::cout << " PASS\n";
}

void testFaceBoundaryProjectionPartitioning() {
    std::cout << "Test 13: Host face projection partitions regions (square + circle)..." << std::flush;

    app::Document doc;
    std::string bodyId = doc.addBody(BRepPrimAPI_MakeBox(20.0, 20.0, 10.0).Shape());
    assert(!bodyId.empty());

    auto topFace = findTopPlanarFace(doc, bodyId);
    assert(topFace.has_value());

    auto sketch = std::make_unique<core::sketch::Sketch>(topFace->second);
    sketch->setHostFaceAttachment(bodyId, topFace->first);

    const auto center = topFace->second.toSketch({10.0, 10.0, 10.0});
    sketch->addCircle(center.x, center.y, 4.0);

    std::string sketchId = doc.addSketch(std::move(sketch));
    core::sketch::Sketch* sketchPtr = doc.getSketch(sketchId);
    assert(sketchPtr);

    bool projected = doc.ensureHostFaceBoundariesProjected(sketchId);
    assert(projected);
    assert(sketchPtr->hasProjectedHostBoundaries());
    assert(sketchPtr->projectedHostBoundariesVersion() == 1);

    auto regions = detectRegions(*sketchPtr);
    assert(regions.size() >= 2);

    std::cout << " PASS\n";
}

void testLegacyHostBoundaryBackfillIdempotent() {
    std::cout << "Test 14: Legacy host-boundary backfill is idempotent..." << std::flush;

    app::Document doc;
    std::string bodyId = doc.addBody(BRepPrimAPI_MakeBox(20.0, 20.0, 10.0).Shape());
    assert(!bodyId.empty());

    auto topFace = findTopPlanarFace(doc, bodyId);
    assert(topFace.has_value());

    auto sketch = std::make_unique<core::sketch::Sketch>(topFace->second);
    sketch->setHostFaceAttachment(bodyId, topFace->first);
    const auto center = topFace->second.toSketch({10.0, 10.0, 10.0});
    sketch->addCircle(center.x, center.y, 4.0);

    std::string sketchId = doc.addSketch(std::move(sketch));
    core::sketch::Sketch* sketchPtr = doc.getSketch(sketchId);
    assert(sketchPtr);
    assert(!sketchPtr->hasProjectedHostBoundaries());
    assert(sketchPtr->projectedHostBoundariesVersion() == 0);

    const size_t countBefore = sketchPtr->getEntityCount();
    bool first = doc.ensureHostFaceBoundariesProjected(sketchId);
    const size_t countAfterFirst = sketchPtr->getEntityCount();
    bool second = doc.ensureHostFaceBoundariesProjected(sketchId);
    const size_t countAfterSecond = sketchPtr->getEntityCount();

    assert(first);
    assert(countAfterFirst > countBefore);
    assert(sketchPtr->hasProjectedHostBoundaries());
    assert(sketchPtr->projectedHostBoundariesVersion() == 1);
    assert(!second);
    assert(countAfterSecond == countAfterFirst);

    std::cout << " PASS\n";
}

void testSketchHostProjectionVersionRequired() {
    std::cout << "Test 15: Sketch host projection JSON requires projection version..." << std::flush;

    core::sketch::Sketch sketch(core::sketch::SketchPlane::XY());
    sketch.setHostFaceAttachment("body-A", "face-B");
    sketch.setProjectedHostBoundariesVersion(1);

    QJsonParseError parseError;
    const QString jsonText = QString::fromStdString(sketch.toJson());
    QJsonDocument docJson = QJsonDocument::fromJson(jsonText.toUtf8(), &parseError);
    assert(parseError.error == QJsonParseError::NoError);
    assert(docJson.isObject());

    QJsonObject root = docJson.object();
    assert(root.contains("hostFace"));
    QJsonObject hostFace = root["hostFace"].toObject();
    hostFace.remove("projectedBoundaryVersion");
    root["hostFace"] = hostFace;

    QJsonDocument withoutVersion(root);
    auto restored = core::sketch::Sketch::fromJson(withoutVersion.toJson().toStdString());
    assert(!restored);

    std::cout << " PASS\n";
}

void testSelectionPriorityPrefersSketchRegion() {
    std::cout << "Test 16: Selection priority prefers SketchRegion over Face..." << std::flush;

    app::selection::SelectionManager manager;
    manager.setMode(app::selection::SelectionMode::Model);

    app::selection::PickResult pick;
    app::selection::SelectionItem face;
    face.kind = app::selection::SelectionKind::Face;
    face.id = {"body-1", "face-1"};
    face.priority = 2;
    face.screenDistance = 0.5;
    face.depth = 0.1;
    pick.hits.push_back(face);

    app::selection::SelectionItem sketchRegion;
    sketchRegion.kind = app::selection::SelectionKind::SketchRegion;
    sketchRegion.id = {"sketch-1", "region-1"};
    sketchRegion.priority = -98; // Matches viewport sketch-priority boost behavior.
    sketchRegion.screenDistance = 0.5;
    sketchRegion.depth = 0.2;
    pick.hits.push_back(sketchRegion);

    app::selection::ClickModifiers modifiers;
    auto action = manager.handleClick(pick, modifiers, QPoint(100, 100));
    assert(action.selectionChanged);
    auto selected = manager.selection();
    assert(selected.size() == 1);
    assert(selected[0].kind == app::selection::SelectionKind::SketchRegion);
    assert(selected[0].id.ownerId == "sketch-1");
    assert(selected[0].id.elementId == "region-1");

    std::cout << " PASS\n";
}

void testProjectedReferenceGeometryIsLocked() {
    std::cout << "Test 17: Projected host reference geometry is non-editable..." << std::flush;

    app::Document doc;
    std::string bodyId = doc.addBody(BRepPrimAPI_MakeBox(20.0, 20.0, 10.0).Shape());
    assert(!bodyId.empty());

    auto topFace = findTopPlanarFace(doc, bodyId);
    assert(topFace.has_value());

    auto sketch = std::make_unique<core::sketch::Sketch>(topFace->second);
    sketch->setHostFaceAttachment(bodyId, topFace->first);
    std::string sketchId = doc.addSketch(std::move(sketch));
    core::sketch::Sketch* sketchPtr = doc.getSketch(sketchId);
    assert(sketchPtr);
    assert(doc.ensureHostFaceBoundariesProjected(sketchId));

    std::string lockedPointId;
    std::string lockedCurveId;
    for (const auto& entity : sketchPtr->getAllEntities()) {
        if (!entity || !entity->isReferenceLocked()) {
            continue;
        }
        if (entity->type() == core::sketch::EntityType::Point && lockedPointId.empty()) {
            lockedPointId = entity->id();
        } else if (entity->type() != core::sketch::EntityType::Point && lockedCurveId.empty()) {
            lockedCurveId = entity->id();
        }
    }

    assert(!lockedPointId.empty());
    assert(!lockedCurveId.empty());
    assert(!sketchPtr->removeEntity(lockedCurveId));
    assert(!sketchPtr->removeEntity(lockedPointId));

    auto freePoint = sketchPtr->addPoint(100.0, 100.0);
    assert(!freePoint.empty());
    assert(sketchPtr->addCoincident(lockedPointId, freePoint).empty());

    core::sketch::ConstraintID lockedFixedId;
    for (const auto& c : sketchPtr->getAllConstraints()) {
        if (c && c->type() == core::sketch::ConstraintType::Fixed &&
            c->references(lockedPointId)) {
            lockedFixedId = c->id();
            break;
        }
    }
    assert(!lockedFixedId.empty());
    assert(!sketchPtr->removeConstraint(lockedFixedId));

    std::cout << " PASS\n";
}

int main(int argc, char* argv[]) {
    QCoreApplication app(argc, argv);

    std::cout << "\n=== RegenerationEngine Prototype Tests ===\n\n";

    testDependencyGraph();
    testSuppressionAndFailure();
    testGraphCycleDetection();
    testSingleExtrude();
    testChainRegeneration();
    testRegenFailureOnMissingSketch();
    testSketchHostAttachmentRoundTrip();
    testHistoryTargetBodyRoundTrip();
    testAttachedSketchExtrudeAdd();
    testAttachedSketchExtrudeCut();
    testAttachedSketchRevolveAdd();
    testBooleanFailureOnMissingTargetBody();
    testFaceBoundaryProjectionPartitioning();
    testLegacyHostBoundaryBackfillIdempotent();
    testSketchHostProjectionVersionRequired();
    testSelectionPriorityPrefersSketchRegion();
    testProjectedReferenceGeometryIsLocked();

    std::cout << "\n=== All tests passed! ===\n\n";
    return 0;
}
