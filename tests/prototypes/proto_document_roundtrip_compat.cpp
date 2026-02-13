#include "app/document/Document.h"
#include "app/history/RegenerationEngine.h"
#include "core/loop/LoopDetector.h"
#include "core/loop/RegionUtils.h"
#include "core/sketch/Sketch.h"
#include "io/OneCADFileIO.h"

#include <QDir>
#include <QFile>
#include <QUuid>

#include <cassert>
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

} // namespace

int main() {
    onecad::app::Document source;

    const std::string sketchId = buildClosedRegionSketch(source);
    auto* sketch = source.getSketch(sketchId);
    if (!sketch) {
        std::cerr << "Failed to create sketch\n";
        return 1;
    }

    const std::string regionId = detectFirstRegion(*sketch);
    if (regionId.empty()) {
        std::cerr << "Failed to detect a closed region\n";
        return 1;
    }

    onecad::app::OperationRecord op;
    op.opId = QUuid::createUuid().toString(QUuid::WithoutBraces).toStdString();
    op.type = onecad::app::OperationType::Extrude;
    op.input = onecad::app::SketchRegionRef{sketchId, regionId};
    op.params = onecad::app::ExtrudeParams{12.0, 0.0, onecad::app::BooleanMode::NewBody};
    const std::string bodyId = QUuid::createUuid().toString(QUuid::WithoutBraces).toStdString();
    op.resultBodyIds.push_back(bodyId);
    source.addOperation(op);

    onecad::app::OperationMetadata metadata;
    metadata.recordSchemaVersion = 1;
    metadata.stepIndex = 0;
    metadata.uiAlias = QStringLiteral("PushPull");
    metadata.replayOnly = false;
    metadata.determinism.parallel = false;
    metadata.determinism.solverPolicyHash = QStringLiteral("solver-v1");
    metadata.anchor.hasWorldPoint = true;
    metadata.anchor.x = 0.5;
    metadata.anchor.y = 0.5;
    metadata.anchor.z = 0.0;
    source.setOperationMetadata(op.opId, metadata);
    source.setAppliedOpCount(source.operations().size());

    onecad::app::history::RegenerationEngine regen(&source);
    const auto regenResult = regen.regenerateAll();
    if (regenResult.status == onecad::app::history::RegenStatus::CriticalFailure) {
        std::cerr << "Source regeneration failed\n";
        return 1;
    }

    const QString tempPath =
        QDir::temp().absoluteFilePath(QString("onecad_roundtrip_%1.onecad")
                                      .arg(QUuid::createUuid().toString(QUuid::WithoutBraces)));

    const auto saveResult = onecad::io::OneCADFileIO::save(tempPath, &source);
    if (!saveResult.success) {
        std::cerr << "Failed to save roundtrip file: " << saveResult.errorMessage.toStdString() << "\n";
        return 1;
    }

    QString loadError;
    auto loaded = onecad::io::OneCADFileIO::load(tempPath, loadError);
    QFile::remove(tempPath);

    if (!loaded) {
        std::cerr << "Failed to load roundtrip file: " << loadError.toStdString() << "\n";
        return 1;
    }

    if (loaded->sketchCount() != source.sketchCount()) {
        std::cerr << "Sketch count mismatch after roundtrip\n";
        return 1;
    }
    if (loaded->operations().size() != source.operations().size()) {
        std::cerr << "Operation count mismatch after roundtrip\n";
        return 1;
    }
    if (loaded->appliedOpCount() != source.appliedOpCount()) {
        std::cerr << "Applied operation cursor mismatch after roundtrip\n";
        return 1;
    }

    const auto loadedMeta = loaded->operationMetadata(op.opId);
    if (!loadedMeta.has_value() || loadedMeta->uiAlias != QStringLiteral("PushPull")) {
        std::cerr << "Operation metadata mismatch after roundtrip\n";
        return 1;
    }

    std::cout << "Document roundtrip compatibility test passed\n";
    return 0;
}
