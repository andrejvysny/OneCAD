#include "app/commands/AddOperationCommand.h"
#include "app/commands/RollbackCommand.h"
#include "app/document/Document.h"
#include "app/history/RegenerationEngine.h"
#include "core/loop/LoopDetector.h"
#include "core/loop/RegionUtils.h"
#include "core/sketch/Sketch.h"

#include <QUuid>

#include <iostream>
#include <string>

namespace {

std::string buildClosedRegionSketch(onecad::app::Document& doc) {
    auto sketch = std::make_unique<onecad::core::sketch::Sketch>();
    const auto p1 = sketch->addPoint(0.0, 0.0);
    const auto p2 = sketch->addPoint(20.0, 0.0);
    const auto p3 = sketch->addPoint(20.0, 15.0);
    const auto p4 = sketch->addPoint(0.0, 15.0);

    sketch->addLine(p1, p2);
    sketch->addLine(p2, p3);
    sketch->addLine(p3, p4);
    sketch->addLine(p4, p1);

    return doc.addSketch(std::move(sketch));
}

std::string detectFirstRegion(onecad::core::sketch::Sketch& sketch) {
    onecad::core::loop::LoopDetectorConfig config = onecad::core::loop::makeRegionDetectionConfig();
    onecad::core::loop::LoopDetector detector(config);
    const auto result = detector.detect(sketch);
    if (!result.success || result.faces.empty()) {
        return {};
    }
    return onecad::core::loop::regionKey(result.faces[0].outerLoop);
}

onecad::app::OperationRecord makeExtrude(const std::string& sketchId,
                                         const std::string& regionId,
                                         double distance,
                                         const std::string& bodyId) {
    onecad::app::OperationRecord op;
    op.opId = QUuid::createUuid().toString(QUuid::WithoutBraces).toStdString();
    op.type = onecad::app::OperationType::Extrude;
    op.input = onecad::app::SketchRegionRef{sketchId, regionId};
    op.params = onecad::app::ExtrudeParams{distance, 0.0, onecad::app::BooleanMode::NewBody};
    op.resultBodyIds.push_back(bodyId);
    return op;
}

} // namespace

int main() {
    onecad::app::Document document;

    const std::string sketchId = buildClosedRegionSketch(document);
    auto* sketch = document.getSketch(sketchId);
    if (!sketch) {
        std::cerr << "Failed to create sketch\n";
        return 1;
    }

    const std::string regionId = detectFirstRegion(*sketch);
    if (regionId.empty()) {
        std::cerr << "Failed to detect closed region\n";
        return 1;
    }

    onecad::app::OperationRecord op1 = makeExtrude(
        sketchId,
        regionId,
        10.0,
        QUuid::createUuid().toString(QUuid::WithoutBraces).toStdString());
    onecad::app::OperationRecord op2 = makeExtrude(
        sketchId,
        regionId,
        5.0,
        QUuid::createUuid().toString(QUuid::WithoutBraces).toStdString());

    if (!onecad::app::commands::AddOperationCommand(&document, op1).execute()) {
        std::cerr << "Failed to add operation 1\n";
        return 1;
    }
    if (!onecad::app::commands::AddOperationCommand(&document, op2).execute()) {
        std::cerr << "Failed to add operation 2\n";
        return 1;
    }

    if (document.operations().size() != 2 || document.appliedOpCount() != 2) {
        std::cerr << "Unexpected baseline history size or applied cursor\n";
        return 1;
    }

    if (!onecad::app::commands::RollbackCommand(&document, op1.opId).execute()) {
        std::cerr << "Rollback command failed\n";
        return 1;
    }

    if (document.appliedOpCount() != 1) {
        std::cerr << "Rollback did not move applied cursor to target\n";
        return 1;
    }

    onecad::app::OperationRecord op3 = makeExtrude(
        sketchId,
        regionId,
        2.5,
        QUuid::createUuid().toString(QUuid::WithoutBraces).toStdString());
    if (!onecad::app::commands::AddOperationCommand(&document, op3).execute()) {
        std::cerr << "Failed to insert operation at rollback cursor\n";
        return 1;
    }

    if (document.operations().size() != 3) {
        std::cerr << "Expected three operations after insertion\n";
        return 1;
    }
    if (document.operationIndex(op1.opId) != 0 || document.operationIndex(op3.opId) != 1 ||
        document.operationIndex(op2.opId) != 2) {
        std::cerr << "Insert-at-rollback ordering mismatch\n";
        return 1;
    }
    if (document.appliedOpCount() != 2) {
        std::cerr << "Applied cursor should include only inserted op prefix\n";
        return 1;
    }

    onecad::app::history::RegenerationEngine regen(&document);
    const auto partial = regen.regenerateToAppliedCount(document.appliedOpCount());
    if (partial.status == onecad::app::history::RegenStatus::CriticalFailure) {
        std::cerr << "Partial regen failed after rollback insertion\n";
        return 1;
    }

    document.setAppliedOpCount(document.operations().size());
    const auto full = regen.regenerateToAppliedCount(document.appliedOpCount());
    if (full.status == onecad::app::history::RegenStatus::CriticalFailure || !full.failedOps.empty()) {
        std::cerr << "Full regen-to-end recovery failed\n";
        return 1;
    }

    std::cout << "Timeline rollback/dirty prototype passed\n";
    return 0;
}
