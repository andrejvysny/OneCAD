#include "Document.h"
#include "../../core/sketch/Sketch.h"
#include "../../core/sketch/FaceBoundaryProjector.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonArray>
#include <cmath>
#include <QUuid>

#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopAbs_Orientation.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>

#include <BRepAdaptor_Surface.hxx>
#include <GeomAbs_SurfaceType.hxx>

#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Vec.hxx>

namespace onecad::app {

Document::Document(QObject* parent)
    : QObject(parent)
{
    sceneMeshStore_ = std::make_unique<render::SceneMeshStore>();
    tessellationCache_ = std::make_unique<render::TessellationCache>();
}

Document::~Document() = default;

std::string Document::addSketch(std::unique_ptr<core::sketch::Sketch> sketch) {
    if (!sketch) {
        return {};
    }

    // Generate unique ID
    std::string id = QUuid::createUuid().toString(QUuid::WithoutBraces).toStdString();

    // Generate default name
    std::string name = "Sketch " + std::to_string(nextSketchNumber_++);
    sketchNames_[id] = name;
    sketchVisibility_[id] = true;

    sketches_[id] = std::move(sketch);
    setModified(true);

    emit sketchAdded(QString::fromStdString(id));
    return id;
}

bool Document::addSketchWithId(const std::string& id,
                                std::unique_ptr<core::sketch::Sketch> sketch,
                                const std::string& name) {
    if (!sketch || id.empty()) {
        return false;
    }
    if (sketches_.find(id) != sketches_.end()) {
        return false;  // ID already exists
    }

    std::string finalName = name;
    if (finalName.empty()) {
        finalName = "Sketch " + std::to_string(nextSketchNumber_++);
    }

    sketchNames_[id] = finalName;
    sketchVisibility_[id] = true;
    sketches_[id] = std::move(sketch);
    setModified(true);

    emit sketchAdded(QString::fromStdString(id));
    return true;
}

core::sketch::Sketch* Document::getSketch(const std::string& id) {
    auto it = sketches_.find(id);
    if (it != sketches_.end()) {
        return it->second.get();
    }
    return nullptr;
}

const core::sketch::Sketch* Document::getSketch(const std::string& id) const {
    auto it = sketches_.find(id);
    if (it != sketches_.end()) {
        return it->second.get();
    }
    return nullptr;
}

std::vector<std::string> Document::getSketchIds() const {
    std::vector<std::string> ids;
    ids.reserve(sketches_.size());
    for (const auto& [id, sketch] : sketches_) {
        ids.push_back(id);
    }
    return ids;
}

bool Document::removeSketch(const std::string& id) {
    auto it = sketches_.find(id);
    if (it == sketches_.end()) {
        return false;
    }

    sketches_.erase(it);
    sketchNames_.erase(id);
    sketchVisibility_.erase(id);
    setModified(true);

    emit sketchRemoved(QString::fromStdString(id));
    return true;
}

std::string Document::getSketchName(const std::string& id) const {
    auto it = sketchNames_.find(id);
    if (it != sketchNames_.end()) {
        return it->second;
    }
    return "Unnamed Sketch";
}

void Document::setSketchName(const std::string& id, const std::string& name) {
    if (sketches_.find(id) == sketches_.end()) {
        return;
    }

    // Validate name - use fallback for empty names
    std::string finalName = name;
    if (finalName.empty() || finalName.find_first_not_of(" \t\n\r") == std::string::npos) {
        finalName = "Untitled";
    }

    // Only update if name actually changed
    auto it = sketchNames_.find(id);
    if (it != sketchNames_.end() && it->second == finalName) {
        return;
    }

    sketchNames_[id] = finalName;
    setModified(true);
    emit sketchRenamed(QString::fromStdString(id), QString::fromStdString(finalName));
}

void Document::setModified(bool modified) {
    if (modified_ != modified) {
        modified_ = modified;
        emit modifiedChanged(modified);
    }
}

void Document::clear() {
    sketches_.clear();
    sketchNames_.clear();
    sketchVisibility_.clear();
    bodies_.clear();
    bodyNames_.clear();
    bodyVisibilityCache_.clear();
    baseBodyIds_.clear();
    operations_.clear();
    suppressedOperations_.clear();
    operationFailures_.clear();
    elementMap_.clear();
    if (sceneMeshStore_) {
        sceneMeshStore_->clear();
    }
    // Clear isolation state
    isolatedItemId_.clear();
    preIsolationBodyVisibility_.clear();
    preIsolationSketchVisibility_.clear();

    nextSketchNumber_ = 1;
    nextBodyNumber_ = 1;
    setModified(false);
    emit documentCleared();
}

std::string Document::toJson() const {
    QJsonObject root;

    // Serialize sketches
    QJsonArray sketchArray;
    for (const auto& [id, sketch] : sketches_) {
        QJsonObject sketchObj;
        sketchObj["id"] = QString::fromStdString(id);
        sketchObj["name"] = QString::fromStdString(getSketchName(id));

        // Use Sketch's own serialization with error handling
        QString sketchJson = QString::fromStdString(sketch->toJson());
        QJsonParseError parseError;
        QJsonDocument sketchDoc = QJsonDocument::fromJson(sketchJson.toUtf8(), &parseError);
        if (parseError.error != QJsonParseError::NoError) {
            // Skip invalid sketch data but continue with others
            continue;
        }
        sketchObj["data"] = sketchDoc.object();

        sketchArray.append(sketchObj);
    }
    root["sketches"] = sketchArray;
    root["nextSketchNumber"] = static_cast<int>(nextSketchNumber_);

    QJsonDocument doc(root);
    return doc.toJson(QJsonDocument::Indented).toStdString();
}

std::unique_ptr<Document> Document::fromJson(const std::string& json, QObject* parent) {
    QJsonParseError parseError;
    QJsonDocument doc = QJsonDocument::fromJson(QByteArray::fromStdString(json), &parseError);
    if (parseError.error != QJsonParseError::NoError || doc.isNull() || !doc.isObject()) {
        return nullptr;
    }

    auto document = std::make_unique<Document>(parent);
    QJsonObject root = doc.object();

    // Validate nextSketchNumber - must be at least 1
    int parsedNumber = root["nextSketchNumber"].toInt(1);
    document->nextSketchNumber_ = (parsedNumber < 1) ? 1 : static_cast<unsigned int>(parsedNumber);

    QJsonArray sketchArray = root["sketches"].toArray();
    for (const auto& sketchVal : sketchArray) {
        QJsonObject sketchObj = sketchVal.toObject();
        std::string id = sketchObj["id"].toString().toStdString();
        std::string name = sketchObj["name"].toString().toStdString();

        QJsonObject dataObj = sketchObj["data"].toObject();
        QJsonDocument dataDoc(dataObj);
        std::string sketchJson = dataDoc.toJson().toStdString();

        auto sketch = core::sketch::Sketch::fromJson(sketchJson);
        if (sketch) {
            document->sketches_[id] = std::move(sketch);
            document->sketchNames_[id] = name;
        }
    }

    return document;
}

std::string Document::addBody(const TopoDS_Shape& shape) {
    if (shape.IsNull()) {
        return {};
    }

    std::string id = QUuid::createUuid().toString(QUuid::WithoutBraces).toStdString();
    if (!addBodyWithId(id, shape)) {
        return {};
    }
    return id;
}

bool Document::addBodyWithId(const std::string& id,
                             const TopoDS_Shape& shape,
                             const std::string& name) {
    if (shape.IsNull() || id.empty()) {
        return false;
    }
    if (bodies_.find(id) != bodies_.end()) {
        return false;
    }

    std::string finalName = name;
    auto nameIt = bodyNames_.find(id);
    if (finalName.empty()) {
        if (nameIt != bodyNames_.end()) {
            finalName = nameIt->second;
        } else {
            finalName = "Body " + std::to_string(nextBodyNumber_++);
        }
    }

    bodyNames_[id] = finalName;

    BodyEntry entry;
    entry.shape = shape;
    auto visibilityIt = bodyVisibilityCache_.find(id);
    if (visibilityIt != bodyVisibilityCache_.end()) {
        entry.visible = visibilityIt->second;
        bodyVisibilityCache_.erase(visibilityIt);
    }
    bodies_[id] = entry;

    elementMap_.rebindBody(id, shape);
    updateBodyMesh(id, shape, false);

    setModified(true);
    emit bodyAdded(QString::fromStdString(id));
    return true;
}

bool Document::updateBodyShape(const std::string& id, const TopoDS_Shape& shape,
                               bool emitSignal, const std::string& opId) {
    if (shape.IsNull()) {
        return false;
    }
    auto it = bodies_.find(id);
    if (it == bodies_.end()) {
        return false;
    }

    it->second.shape = shape;
    elementMap_.rebindBody(id, shape, opId);
    updateBodyMesh(id, shape, emitSignal);
    setModified(true);
    return true;
}

const TopoDS_Shape* Document::getBodyShape(const std::string& id) const {
    auto it = bodies_.find(id);
    if (it == bodies_.end()) {
        return nullptr;
    }
    return &it->second.shape;
}

std::optional<core::sketch::SketchPlane> Document::getSketchPlaneForFace(const std::string& bodyId,
                                                                          const std::string& faceId) const {
    if (bodyId.empty() || faceId.empty()) {
        return std::nullopt;
    }

    const TopoDS_Shape* bodyShape = getBodyShape(bodyId);
    if (!bodyShape || bodyShape->IsNull()) {
        return std::nullopt;
    }

    const auto* faceEntry = elementMap_.find(kernel::elementmap::ElementId::From(faceId));
    if (!faceEntry || faceEntry->kind != kernel::elementmap::ElementKind::Face ||
        faceEntry->shape.IsNull()) {
        return std::nullopt;
    }

    TopoDS_Face face = TopoDS::Face(faceEntry->shape);
    bool belongsToBody = false;
    for (TopExp_Explorer explorer(*bodyShape, TopAbs_FACE); explorer.More(); explorer.Next()) {
        const TopoDS_Shape& candidateFace = explorer.Current();
        if (candidateFace.IsSame(face)) {
            belongsToBody = true;
            break;
        }
    }

    if (!belongsToBody) {
        return std::nullopt;
    }

    BRepAdaptor_Surface surface(face, true);
    if (surface.GetType() != GeomAbs_Plane) {
        return std::nullopt;
    }

    gp_Pln plane = surface.Plane();
    gp_Dir normal = plane.Axis().Direction();
    if (face.Orientation() == TopAbs_REVERSED) {
        normal.Reverse();
    }

    gp_Dir xSeed = plane.Position().XDirection();
    gp_Vec xVec(xSeed);
    gp_Vec normalVec(normal);
    xVec -= normalVec.Multiplied(xVec.Dot(normalVec));

    constexpr double kEpsilon = 1e-12;
    if (xVec.SquareMagnitude() < kEpsilon) {
        gp_Dir fallback = std::abs(normal.Z()) < 0.9 ? gp_Dir(0.0, 0.0, 1.0) : gp_Dir(0.0, 1.0, 0.0);
        xVec = gp_Vec(normal) ^ gp_Vec(fallback);
    }
    if (xVec.SquareMagnitude() < kEpsilon) {
        return std::nullopt;
    }
    xVec.Normalize();

    gp_Dir xAxis(xVec);
    gp_Dir yAxis = normal ^ xAxis;

    core::sketch::SketchPlane sketchPlane;
    sketchPlane.origin = {plane.Location().X(), plane.Location().Y(), plane.Location().Z()};
    sketchPlane.xAxis = {xAxis.X(), xAxis.Y(), xAxis.Z()};
    sketchPlane.yAxis = {yAxis.X(), yAxis.Y(), yAxis.Z()};
    sketchPlane.normal = {normal.X(), normal.Y(), normal.Z()};
    return sketchPlane;
}

bool Document::ensureHostFaceBoundariesProjected(const std::string& sketchId) {
    core::sketch::Sketch* sketch = getSketch(sketchId);
    if (!sketch) {
        return false;
    }

    const auto& hostFace = sketch->hostFaceAttachment();
    if (!hostFace || !hostFace->isValid()) {
        return false;
    }

    if (sketch->hasProjectedHostBoundaries()) {
        return false;
    }

    return projectHostFaceBoundaries(*sketch, hostFace->bodyId, hostFace->faceId);
}

bool Document::projectHostFaceBoundaries(core::sketch::Sketch& sketch,
                                         const std::string& bodyId,
                                         const std::string& faceId) {
    if (bodyId.empty() || faceId.empty()) {
        return false;
    }

    const auto plane = getSketchPlaneForFace(bodyId, faceId);
    if (!plane.has_value()) {
        return false;
    }

    const auto* faceEntry = elementMap_.find(kernel::elementmap::ElementId::From(faceId));
    if (!faceEntry || faceEntry->kind != kernel::elementmap::ElementKind::Face ||
        faceEntry->shape.IsNull()) {
        return false;
    }

    const auto* bodyShape = getBodyShape(bodyId);
    if (!bodyShape || bodyShape->IsNull()) {
        return false;
    }

    const TopoDS_Face face = TopoDS::Face(faceEntry->shape);
    bool belongsToBody = false;
    for (TopExp_Explorer explorer(*bodyShape, TopAbs_FACE); explorer.More(); explorer.Next()) {
        if (explorer.Current().IsSame(face)) {
            belongsToBody = true;
            break;
        }
    }
    if (!belongsToBody) {
        return false;
    }

    core::sketch::FaceBoundaryProjector::Options options;
    options.preferExactPrimitives = true;
    options.lockInsertedBoundaryPoints = true;
    options.pointMergeTolerance = 1e-5;
    options.radiusTolerance = 1e-5;
    options.fallbackSegmentsPerCurve = 32;

    auto projection = core::sketch::FaceBoundaryProjector::projectFaceBoundaries(face, sketch, options);
    if (!projection.success) {
        return false;
    }

    if (projection.hasClosedBoundary) {
        const bool insertedSketchData =
            projection.insertedPoints > 0 ||
            projection.insertedCurves > 0 ||
            projection.insertedFixedConstraints > 0;
        sketch.setProjectedHostBoundariesVersion(1);
        if (insertedSketchData) {
            setModified(true);
        }
        return insertedSketchData;
    }

    return false;
}

std::vector<std::string> Document::getBodyIds() const {
    std::vector<std::string> ids;
    ids.reserve(bodies_.size());
    for (const auto& [id, body] : bodies_) {
        (void)body;
        ids.push_back(id);
    }
    return ids;
}

bool Document::removeBody(const std::string& id) {
    auto it = bodies_.find(id);
    if (it == bodies_.end()) {
        return false;
    }

    bodyVisibilityCache_.erase(id);
    baseBodyIds_.erase(id);
    bodies_.erase(it);
    bodyNames_.erase(id);
    if (sceneMeshStore_) {
        sceneMeshStore_->removeBody(id);
    }
    elementMap_.removeElementsForBody(id);

    setModified(true);
    emit bodyRemoved(QString::fromStdString(id));
    return true;
}

bool Document::removeBodyPreserveElementMap(const std::string& id) {
    auto it = bodies_.find(id);
    if (it == bodies_.end()) {
        return false;
    }

    bodyVisibilityCache_[id] = it->second.visible;
    bodies_.erase(it);
    if (sceneMeshStore_) {
        sceneMeshStore_->removeBody(id);
    }

    setModified(true);
    emit bodyRemoved(QString::fromStdString(id));
    return true;
}

std::string Document::getBodyName(const std::string& id) const {
    auto it = bodyNames_.find(id);
    if (it != bodyNames_.end()) {
        return it->second;
    }
    return "Unnamed Body";
}

void Document::setBodyName(const std::string& id, const std::string& name) {
    if (bodies_.find(id) == bodies_.end()) {
        return;
    }

    std::string finalName = name;
    if (finalName.empty() || finalName.find_first_not_of(" \t\n\r") == std::string::npos) {
        finalName = "Untitled";
    }

    auto it = bodyNames_.find(id);
    if (it != bodyNames_.end() && it->second == finalName) {
        return;
    }

    bodyNames_[id] = finalName;
    setModified(true);
    emit bodyRenamed(QString::fromStdString(id), QString::fromStdString(finalName));
}

void Document::setBaseBodyIds(const std::unordered_set<std::string>& ids) {
    baseBodyIds_ = ids;
}

void Document::addBaseBodyId(const std::string& id) {
    if (!id.empty()) {
        baseBodyIds_.insert(id);
    }
}

bool Document::isBaseBody(const std::string& id) const {
    return baseBodyIds_.find(id) != baseBodyIds_.end();
}

void Document::addOperation(const OperationRecord& record) {
    insertOperation(operations_.size(), record);
}

bool Document::insertOperation(std::size_t index, const OperationRecord& record) {
    if (index > operations_.size()) {
        return false;
    }
    operations_.insert(operations_.begin() + static_cast<std::ptrdiff_t>(index), record);
    setModified(true);
    emit operationAdded(QString::fromStdString(record.opId));
    return true;
}

bool Document::updateOperationParams(const std::string& opId, const OperationParams& params) {
    for (auto& op : operations_) {
        if (op.opId == opId) {
            op.params = params;
            setModified(true);
            emit operationUpdated(QString::fromStdString(opId));
            return true;
        }
    }
    return false;
}

bool Document::removeOperation(const std::string& opId) {
    auto it = std::remove_if(operations_.begin(), operations_.end(),
                             [&](const OperationRecord& op) { return op.opId == opId; });
    if (it == operations_.end()) {
        return false;
    }
    operations_.erase(it, operations_.end());
    suppressedOperations_.erase(opId);
    operationFailures_.erase(opId);
    setModified(true);
    emit operationRemoved(QString::fromStdString(opId));
    return true;
}

int Document::operationIndex(const std::string& opId) const {
    for (std::size_t i = 0; i < operations_.size(); ++i) {
        if (operations_[i].opId == opId) {
            return static_cast<int>(i);
        }
    }
    return -1;
}

OperationRecord* Document::findOperation(const std::string& opId) {
    for (auto& op : operations_) {
        if (op.opId == opId) {
            return &op;
        }
    }
    return nullptr;
}

const OperationRecord* Document::findOperation(const std::string& opId) const {
    for (const auto& op : operations_) {
        if (op.opId == opId) {
            return &op;
        }
    }
    return nullptr;
}

bool Document::setOperationSuppressed(const std::string& opId, bool suppressed) {
    if (!findOperation(opId)) {
        return false;
    }
    const bool wasSuppressed = suppressedOperations_.count(opId) > 0;
    if (suppressed) {
        suppressedOperations_.insert(opId);
    } else {
        suppressedOperations_.erase(opId);
    }
    if (wasSuppressed != suppressed) {
        emit operationSuppressionChanged(QString::fromStdString(opId), suppressed);
    }
    return true;
}

bool Document::isOperationSuppressed(const std::string& opId) const {
    return suppressedOperations_.count(opId) > 0;
}

std::unordered_map<std::string, bool> Document::operationSuppressionState() const {
    std::unordered_map<std::string, bool> state;
    state.reserve(operations_.size());
    for (const auto& op : operations_) {
        state[op.opId] = suppressedOperations_.count(op.opId) > 0;
    }
    return state;
}

void Document::setOperationSuppressionState(const std::unordered_map<std::string, bool>& state) {
    suppressedOperations_.clear();
    for (const auto& [opId, suppressed] : state) {
        if (suppressed && findOperation(opId)) {
            suppressedOperations_.insert(opId);
        }
    }
}

void Document::setOperationFailed(const std::string& opId, const std::string& reason) {
    if (!findOperation(opId)) {
        return;
    }
    auto it = operationFailures_.find(opId);
    if (it != operationFailures_.end() && it->second == reason) {
        return;
    }
    operationFailures_[opId] = reason;
    emit operationFailed(QString::fromStdString(opId), QString::fromStdString(reason));
}

void Document::clearOperationFailed(const std::string& opId) {
    auto it = operationFailures_.find(opId);
    if (it == operationFailures_.end()) {
        return;
    }
    operationFailures_.erase(it);
    emit operationSucceeded(QString::fromStdString(opId));
}

void Document::clearOperationFailures() {
    for (const auto& [opId, reason] : operationFailures_) {
        (void)reason;
        emit operationSucceeded(QString::fromStdString(opId));
    }
    operationFailures_.clear();
}

bool Document::isOperationFailed(const std::string& opId) const {
    return operationFailures_.find(opId) != operationFailures_.end();
}

std::string Document::operationFailureReason(const std::string& opId) const {
    auto it = operationFailures_.find(opId);
    if (it == operationFailures_.end()) {
        return {};
    }
    return it->second;
}

// Visibility management

bool Document::isBodyVisible(const std::string& id) const {
    auto it = bodies_.find(id);
    if (it == bodies_.end()) {
        return false;
    }
    return it->second.visible;
}

void Document::setBodyVisible(const std::string& id, bool visible) {
    auto it = bodies_.find(id);
    if (it == bodies_.end()) {
        return;
    }
    if (it->second.visible == visible) {
        return;
    }
    it->second.visible = visible;
    emit bodyVisibilityChanged(QString::fromStdString(id), visible);
}

bool Document::isSketchVisible(const std::string& id) const {
    auto it = sketchVisibility_.find(id);
    if (it == sketchVisibility_.end()) {
        return true;  // Default visible if not found
    }
    return it->second;
}

void Document::setSketchVisible(const std::string& id, bool visible) {
    if (sketches_.find(id) == sketches_.end()) {
        return;
    }
    auto it = sketchVisibility_.find(id);
    if (it != sketchVisibility_.end() && it->second == visible) {
        return;
    }
    sketchVisibility_[id] = visible;
    emit sketchVisibilityChanged(QString::fromStdString(id), visible);
}

// Isolation management

void Document::isolateItem(const std::string& id) {
    // Check if item exists
    bool isBody = bodies_.find(id) != bodies_.end();
    bool isSketch = sketches_.find(id) != sketches_.end();
    if (!isBody && !isSketch) {
        return;
    }

    // If same item is already isolated, toggle off
    if (isolatedItemId_ == id) {
        clearIsolation();
        return;
    }

    // Save current visibility state before isolation
    preIsolationBodyVisibility_.clear();
    preIsolationSketchVisibility_.clear();

    for (const auto& [bodyId, entry] : bodies_) {
        preIsolationBodyVisibility_[bodyId] = entry.visible;
    }
    for (const auto& [sketchId, visible] : sketchVisibility_) {
        preIsolationSketchVisibility_[sketchId] = visible;
    }

    // Hide all items except the isolated one
    for (auto& [bodyId, entry] : bodies_) {
        bool shouldBeVisible = (bodyId == id);
        if (entry.visible != shouldBeVisible) {
            entry.visible = shouldBeVisible;
            emit bodyVisibilityChanged(QString::fromStdString(bodyId), shouldBeVisible);
        }
    }
    for (auto& [sketchId, visible] : sketchVisibility_) {
        bool shouldBeVisible = (sketchId == id);
        if (visible != shouldBeVisible) {
            visible = shouldBeVisible;
            emit sketchVisibilityChanged(QString::fromStdString(sketchId), shouldBeVisible);
        }
    }

    isolatedItemId_ = id;
    emit isolationChanged();
}

void Document::clearIsolation() {
    if (isolatedItemId_.empty()) {
        return;
    }

    // Restore pre-isolation visibility
    for (auto& [bodyId, entry] : bodies_) {
        auto it = preIsolationBodyVisibility_.find(bodyId);
        bool prevVisible = (it != preIsolationBodyVisibility_.end()) ? it->second : true;
        if (entry.visible != prevVisible) {
            entry.visible = prevVisible;
            emit bodyVisibilityChanged(QString::fromStdString(bodyId), prevVisible);
        }
    }
    for (auto& [sketchId, visible] : sketchVisibility_) {
        auto it = preIsolationSketchVisibility_.find(sketchId);
        bool prevVisible = (it != preIsolationSketchVisibility_.end()) ? it->second : true;
        if (visible != prevVisible) {
            visible = prevVisible;
            emit sketchVisibilityChanged(QString::fromStdString(sketchId), prevVisible);
        }
    }

    isolatedItemId_.clear();
    preIsolationBodyVisibility_.clear();
    preIsolationSketchVisibility_.clear();
    emit isolationChanged();
}

void Document::registerBodyElements(const std::string& bodyId, const TopoDS_Shape& shape) {
    using kernel::elementmap::ElementId;
    using kernel::elementmap::ElementKind;

    elementMap_.registerElement(ElementId::From(bodyId), ElementKind::Body, shape);

    TopTools_IndexedMapOfShape faceMap;
    TopTools_IndexedMapOfShape edgeMap;
    TopTools_IndexedMapOfShape vertexMap;
    TopExp::MapShapes(shape, TopAbs_FACE, faceMap);
    TopExp::MapShapes(shape, TopAbs_EDGE, edgeMap);
    TopExp::MapShapes(shape, TopAbs_VERTEX, vertexMap);

    for (int i = 1; i <= faceMap.Extent(); ++i) {
        TopoDS_Face face = TopoDS::Face(faceMap(i));
        std::string faceId = bodyId + "/face/" + std::to_string(i - 1);
        elementMap_.registerElement(ElementId::From(faceId), ElementKind::Face, face);
    }

    for (int i = 1; i <= edgeMap.Extent(); ++i) {
        TopoDS_Edge edge = TopoDS::Edge(edgeMap(i));
        std::string edgeId = bodyId + "/edge/" + std::to_string(i - 1);
        elementMap_.registerElement(ElementId::From(edgeId), ElementKind::Edge, edge);
    }

    for (int i = 1; i <= vertexMap.Extent(); ++i) {
        TopoDS_Vertex vertex = TopoDS::Vertex(vertexMap(i));
        std::string vertexId = bodyId + "/vertex/" + std::to_string(i - 1);
        elementMap_.registerElement(ElementId::From(vertexId), ElementKind::Vertex, vertex);
    }
}

void Document::updateBodyMesh(const std::string& bodyId,
                              const TopoDS_Shape& shape,
                              bool emitSignal) {
    if (!sceneMeshStore_ || !tessellationCache_) {
        return;
    }
    render::SceneMeshStore::Mesh mesh = tessellationCache_->buildMesh(bodyId, shape, elementMap_);
    sceneMeshStore_->setBodyMesh(bodyId, std::move(mesh));
    if (emitSignal) {
        emit bodyUpdated(QString::fromStdString(bodyId));
    }
}

void Document::rebuildElementMap() {
    elementMap_.clear();
    for (const auto& [id, body] : bodies_) {
        elementMap_.rebindBody(id, body.shape);
    }
}

} // namespace onecad::app
