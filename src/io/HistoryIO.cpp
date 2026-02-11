/**
 * @file HistoryIO.cpp
 * @brief Implementation of operation history serialization
 */

#include "HistoryIO.h"
#include "Package.h"
#include "JSONUtils.h"
#include "../app/document/Document.h"
#include "../app/document/OperationRecord.h"

#include <QJsonDocument>
#include <QJsonArray>
#include <QCryptographicHash>

namespace onecad::io {

using namespace app;

namespace {

QString operationTypeToString(OperationType type) {
    switch (type) {
        case OperationType::Extrude: return "Extrude";
        case OperationType::Revolve: return "Revolve";
        case OperationType::Fillet: return "Fillet";
        case OperationType::Chamfer: return "Chamfer";
        case OperationType::Shell: return "Shell";
        case OperationType::Boolean: return "Boolean";
        default: return "Unknown";
    }
}

OperationType stringToOperationType(const QString& str) {
    if (str == "Extrude") return OperationType::Extrude;
    if (str == "Revolve") return OperationType::Revolve;
    if (str == "Fillet") return OperationType::Fillet;
    if (str == "Chamfer") return OperationType::Chamfer;
    if (str == "Shell") return OperationType::Shell;
    if (str == "Boolean") return OperationType::Boolean;
    return OperationType::Extrude;  // Default
}

QString filletChamferModeToString(FilletChamferParams::Mode mode) {
    switch (mode) {
        case FilletChamferParams::Mode::Fillet: return "Fillet";
        case FilletChamferParams::Mode::Chamfer: return "Chamfer";
        default: return "Fillet";
    }
}

FilletChamferParams::Mode stringToFilletChamferMode(const QString& str) {
    if (str == "Chamfer") return FilletChamferParams::Mode::Chamfer;
    return FilletChamferParams::Mode::Fillet;
}

QString booleanOpToString(BooleanParams::Op op) {
    switch (op) {
        case BooleanParams::Op::Union: return "Union";
        case BooleanParams::Op::Cut: return "Cut";
        case BooleanParams::Op::Intersect: return "Intersect";
        default: return "Union";
    }
}

BooleanParams::Op stringToBooleanOp(const QString& str) {
    if (str == "Cut") return BooleanParams::Op::Cut;
    if (str == "Intersect") return BooleanParams::Op::Intersect;
    return BooleanParams::Op::Union;
}

QString booleanModeToString(BooleanMode mode) {
    switch (mode) {
        case BooleanMode::NewBody: return "NewBody";
        case BooleanMode::Add: return "Add";
        case BooleanMode::Cut: return "Cut";
        case BooleanMode::Intersect: return "Intersect";
        default: return "NewBody";
    }
}

BooleanMode stringToBooleanMode(const QString& str) {
    if (str == "NewBody") return BooleanMode::NewBody;
    if (str == "Add") return BooleanMode::Add;
    if (str == "Cut") return BooleanMode::Cut;
    if (str == "Intersect") return BooleanMode::Intersect;
    return BooleanMode::NewBody;
}

} // anonymous namespace

bool HistoryIO::saveHistory(Package* package,
                            const std::vector<OperationRecord>& operations,
                            const std::unordered_map<std::string, bool>& suppressionState) {
    // Write ops.jsonl - one JSON object per line
    QByteArray opsData;
    for (const auto& op : operations) {
        QJsonObject opJson = serializeOperation(op);
        QJsonDocument doc(opJson);
        opsData.append(doc.toJson(QJsonDocument::Compact));
        opsData.append('\n');
    }
    
    if (!package->writeFile("history/ops.jsonl", opsData)) {
        return false;
    }
    
    // Write state.json - undo/redo cursor
    QJsonObject stateJson;
    QJsonObject cursor;
    cursor["appliedOpCount"] = static_cast<int>(operations.size());
    if (!operations.empty()) {
        cursor["lastAppliedOpId"] = QString::fromStdString(operations.back().opId);
    }
    stateJson["cursor"] = cursor;
    QJsonArray suppressedOps;
    for (const auto& [opId, suppressed] : suppressionState) {
        if (suppressed) {
            suppressedOps.append(QString::fromStdString(opId));
        }
    }
    stateJson["suppressedOps"] = suppressedOps;
    
    return package->writeFile("history/state.json", JSONUtils::toCanonicalJson(stateJson));
}

bool HistoryIO::loadHistory(Package* package,
                            Document* document,
                            QString& errorMessage) {
    // Read ops.jsonl
    QByteArray opsData = package->readFile("history/ops.jsonl");
    if (opsData.isEmpty()) {
        // Not an error - new document may not have history
        return true;
    }
    
    // Parse JSONL (one JSON object per line)
    QList<QByteArray> lines = opsData.split('\n');
    for (const QByteArray& line : lines) {
        if (line.trimmed().isEmpty()) continue;
        
        QJsonParseError parseError;
        QJsonDocument doc = QJsonDocument::fromJson(line, &parseError);
        if (parseError.error != QJsonParseError::NoError) {
            errorMessage = QString("Invalid JSON in ops.jsonl: %1").arg(parseError.errorString());
            return false;
        }
        
        OperationRecord op = deserializeOperation(doc.object());
        document->addOperation(op);
    }

    // Read state.json (suppression + cursor)
    QByteArray stateData = package->readFile("history/state.json");
    if (!stateData.isEmpty()) {
        QJsonParseError stateError;
        QJsonDocument stateDoc = QJsonDocument::fromJson(stateData, &stateError);
        if (stateError.error == QJsonParseError::NoError && stateDoc.isObject()) {
            QJsonObject stateJson = stateDoc.object();
            QJsonArray suppressedOps = stateJson["suppressedOps"].toArray();
            std::unordered_map<std::string, bool> suppressionState;
            suppressionState.reserve(static_cast<size_t>(suppressedOps.size()));
            for (const auto& opVal : suppressedOps) {
                suppressionState[opVal.toString().toStdString()] = true;
            }
            document->setOperationSuppressionState(suppressionState);
        }
    }
    
    return true;
}

QJsonObject HistoryIO::serializeOperation(const OperationRecord& op) {
    QJsonObject json;
    
    json["opId"] = QString::fromStdString(op.opId);
    json["type"] = operationTypeToString(op.type);
    
    // Serialize input
    QJsonObject inputs;
    if (std::holds_alternative<SketchRegionRef>(op.input)) {
        const auto& ref = std::get<SketchRegionRef>(op.input);
        QJsonObject sketch;
        sketch["sketchId"] = QString::fromStdString(ref.sketchId);
        sketch["regionId"] = QString::fromStdString(ref.regionId);
        inputs["sketch"] = sketch;
    }
    else if (std::holds_alternative<FaceRef>(op.input)) {
        const auto& ref = std::get<FaceRef>(op.input);
        QJsonObject face;
        face["bodyId"] = QString::fromStdString(ref.bodyId);
        face["faceId"] = QString::fromStdString(ref.faceId);
        inputs["face"] = face;
    }
    else if (std::holds_alternative<BodyRef>(op.input)) {
        const auto& ref = std::get<BodyRef>(op.input);
        QJsonObject body;
        body["bodyId"] = QString::fromStdString(ref.bodyId);
        inputs["body"] = body;
    }
    json["inputs"] = inputs;
    
    // Serialize parameters
    QJsonObject params;
    if (std::holds_alternative<ExtrudeParams>(op.params)) {
        const auto& p = std::get<ExtrudeParams>(op.params);
        params["distance"] = p.distance;
        params["draftAngleDeg"] = p.draftAngleDeg;
        params["booleanMode"] = booleanModeToString(p.booleanMode);
        if (!p.targetBodyId.empty()) {
            params["targetBodyId"] = QString::fromStdString(p.targetBodyId);
        }
    }
    else if (std::holds_alternative<RevolveParams>(op.params)) {
        const auto& p = std::get<RevolveParams>(op.params);
        params["angleDeg"] = p.angleDeg;
        params["booleanMode"] = booleanModeToString(p.booleanMode);
        if (!p.targetBodyId.empty()) {
            params["targetBodyId"] = QString::fromStdString(p.targetBodyId);
        }

        // Serialize axis reference
        if (std::holds_alternative<SketchLineRef>(p.axis)) {
            const auto& axis = std::get<SketchLineRef>(p.axis);
            QJsonObject axisJson;
            axisJson["sketchId"] = QString::fromStdString(axis.sketchId);
            axisJson["lineId"] = QString::fromStdString(axis.lineId);
            params["axisSketchLine"] = axisJson;
        }
        else if (std::holds_alternative<EdgeRef>(p.axis)) {
            const auto& axis = std::get<EdgeRef>(p.axis);
            QJsonObject axisJson;
            axisJson["bodyId"] = QString::fromStdString(axis.bodyId);
            axisJson["edgeId"] = QString::fromStdString(axis.edgeId);
            params["axisEdge"] = axisJson;
        }
    }
    else if (std::holds_alternative<FilletChamferParams>(op.params)) {
        const auto& p = std::get<FilletChamferParams>(op.params);
        params["mode"] = filletChamferModeToString(p.mode);
        params["radius"] = p.radius;
        params["chainTangentEdges"] = p.chainTangentEdges;

        QJsonArray edgeIds;
        for (const auto& edgeId : p.edgeIds) {
            edgeIds.append(QString::fromStdString(edgeId));
        }
        params["edgeIds"] = edgeIds;
    }
    else if (std::holds_alternative<ShellParams>(op.params)) {
        const auto& p = std::get<ShellParams>(op.params);
        params["thickness"] = p.thickness;

        QJsonArray openFaceIds;
        for (const auto& faceId : p.openFaceIds) {
            openFaceIds.append(QString::fromStdString(faceId));
        }
        params["openFaceIds"] = openFaceIds;
    }
    else if (std::holds_alternative<BooleanParams>(op.params)) {
        const auto& p = std::get<BooleanParams>(op.params);
        params["operation"] = booleanOpToString(p.operation);
        params["targetBodyId"] = QString::fromStdString(p.targetBodyId);
        params["toolBodyId"] = QString::fromStdString(p.toolBodyId);
    }
    json["params"] = params;
    
    // Serialize outputs
    QJsonArray resultBodies;
    for (const auto& bodyId : op.resultBodyIds) {
        resultBodies.append(QString::fromStdString(bodyId));
    }
    json["resultBodyIds"] = resultBodies;
    
    return json;
}

OperationRecord HistoryIO::deserializeOperation(const QJsonObject& json) {
    OperationRecord op;
    
    op.opId = json["opId"].toString().toStdString();
    op.type = stringToOperationType(json["type"].toString());
    
    // Parse inputs
    QJsonObject inputs = json["inputs"].toObject();
    if (inputs.contains("sketch")) {
        QJsonObject sketch = inputs["sketch"].toObject();
        SketchRegionRef ref;
        ref.sketchId = sketch["sketchId"].toString().toStdString();
        ref.regionId = sketch["regionId"].toString().toStdString();
        op.input = ref;
    }
    else if (inputs.contains("face")) {
        QJsonObject face = inputs["face"].toObject();
        FaceRef ref;
        ref.bodyId = face["bodyId"].toString().toStdString();
        ref.faceId = face["faceId"].toString().toStdString();
        op.input = ref;
    }
    else if (inputs.contains("body")) {
        QJsonObject body = inputs["body"].toObject();
        BodyRef ref;
        ref.bodyId = body["bodyId"].toString().toStdString();
        op.input = ref;
    }

    // Parse parameters
    QJsonObject params = json["params"].toObject();
    
    if (op.type == OperationType::Extrude) {
        ExtrudeParams p;
        p.distance = params["distance"].toDouble();
        p.draftAngleDeg = params["draftAngleDeg"].toDouble();
        p.booleanMode = stringToBooleanMode(params["booleanMode"].toString());
        if (params.contains("targetBodyId")) {
            p.targetBodyId = params["targetBodyId"].toString().toStdString();
        }
        op.params = p;
    }
    else if (op.type == OperationType::Revolve) {
        RevolveParams p;
        p.angleDeg = params["angleDeg"].toDouble();
        p.booleanMode = stringToBooleanMode(params["booleanMode"].toString());
        if (params.contains("targetBodyId")) {
            p.targetBodyId = params["targetBodyId"].toString().toStdString();
        }

        if (params.contains("axisSketchLine")) {
            QJsonObject axisJson = params["axisSketchLine"].toObject();
            SketchLineRef axis;
            axis.sketchId = axisJson["sketchId"].toString().toStdString();
            axis.lineId = axisJson["lineId"].toString().toStdString();
            p.axis = axis;
        }
        else if (params.contains("axisEdge")) {
            QJsonObject axisJson = params["axisEdge"].toObject();
            EdgeRef axis;
            axis.bodyId = axisJson["bodyId"].toString().toStdString();
            axis.edgeId = axisJson["edgeId"].toString().toStdString();
            p.axis = axis;
        }

        op.params = p;
    }
    else if (op.type == OperationType::Fillet || op.type == OperationType::Chamfer) {
        FilletChamferParams p;
        if (params.contains("mode")) {
            p.mode = stringToFilletChamferMode(params["mode"].toString());
        } else {
            p.mode = (op.type == OperationType::Chamfer)
                         ? FilletChamferParams::Mode::Chamfer
                         : FilletChamferParams::Mode::Fillet;
        }
        p.radius = params["radius"].toDouble();
        p.chainTangentEdges = params["chainTangentEdges"].toBool(true);

        QJsonArray edgeIds = params["edgeIds"].toArray();
        for (const auto& edgeVal : edgeIds) {
            p.edgeIds.push_back(edgeVal.toString().toStdString());
        }

        op.params = p;
    }
    else if (op.type == OperationType::Shell) {
        ShellParams p;
        p.thickness = params["thickness"].toDouble();

        QJsonArray openFaceIds = params["openFaceIds"].toArray();
        for (const auto& faceVal : openFaceIds) {
            p.openFaceIds.push_back(faceVal.toString().toStdString());
        }

        op.params = p;
    }
    else if (op.type == OperationType::Boolean) {
        BooleanParams p;
        p.operation = stringToBooleanOp(params["operation"].toString());
        p.targetBodyId = params["targetBodyId"].toString().toStdString();
        p.toolBodyId = params["toolBodyId"].toString().toStdString();

        op.params = p;
    }

    // Parse result bodies
    QJsonArray resultBodies = json["resultBodyIds"].toArray();
    for (const auto& bodyVal : resultBodies) {
        op.resultBodyIds.push_back(bodyVal.toString().toStdString());
    }
    
    return op;
}

QString HistoryIO::computeOpsHash(const std::vector<OperationRecord>& operations) {
    QCryptographicHash hash(QCryptographicHash::Sha256);
    
    for (const auto& op : operations) {
        QJsonObject opJson = serializeOperation(op);
        QJsonDocument doc(opJson);
        hash.addData(doc.toJson(QJsonDocument::Compact));
    }
    
    return QString::fromLatin1(hash.result().toHex());
}

} // namespace onecad::io
