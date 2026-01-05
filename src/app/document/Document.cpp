#include "Document.h"
#include "../../core/sketch/Sketch.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonArray>
#include <QUuid>

#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>

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

    sketches_[id] = std::move(sketch);
    setModified(true);

    emit sketchAdded(QString::fromStdString(id));
    return id;
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
    bodies_.clear();
    bodyNames_.clear();
    operations_.clear();
    elementMap_.clear();
    if (sceneMeshStore_) {
        sceneMeshStore_->clear();
    }
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
    if (finalName.empty()) {
        finalName = "Body " + std::to_string(nextBodyNumber_++);
    }

    bodyNames_[id] = finalName;

    BodyEntry entry;
    entry.shape = shape;
    bodies_[id] = entry;

    registerBodyElements(id, shape);
    updateBodyMesh(id, shape, false);

    setModified(true);
    emit bodyAdded(QString::fromStdString(id));
    return true;
}

const TopoDS_Shape* Document::getBodyShape(const std::string& id) const {
    auto it = bodies_.find(id);
    if (it == bodies_.end()) {
        return nullptr;
    }
    return &it->second.shape;
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

void Document::addOperation(const OperationRecord& record) {
    operations_.push_back(record);
    setModified(true);
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
        registerBodyElements(id, body.shape);
    }
}

} // namespace onecad::app
