/**
 * @file Document.h
 * @brief Document model for storing sketches and bodies
 */

#ifndef ONECAD_APP_DOCUMENT_DOCUMENT_H
#define ONECAD_APP_DOCUMENT_DOCUMENT_H

#include <QObject>
#include <QString>
#include <memory>
#include <string>
#include <unordered_map>
#include <vector>

#include <TopoDS_Shape.hxx>

#include "OperationRecord.h"
#include "../../kernel/elementmap/ElementMap.h"
#include "../../render/scene/SceneMeshStore.h"
#include "../../render/tessellation/TessellationCache.h"

namespace onecad::core::sketch {
class Sketch;
}

namespace onecad::app {

/**
 * @brief Central document model owning all sketches and bodies
 *
 * Provides persistent storage for sketch data, emits signals when
 * content changes for UI updates (navigator, viewport, etc.)
 */
class Document : public QObject {
    Q_OBJECT

public:
    explicit Document(QObject* parent = nullptr);
    ~Document() override;

    // Sketch management
    /**
     * @brief Add a new sketch to the document
     * @param sketch Ownership transferred to document
     * @return ID of the added sketch
     */
    std::string addSketch(std::unique_ptr<core::sketch::Sketch> sketch);

    /**
     * @brief Get sketch by ID
     * @return Pointer to sketch or nullptr if not found
     */
    core::sketch::Sketch* getSketch(const std::string& id);
    const core::sketch::Sketch* getSketch(const std::string& id) const;

    /**
     * @brief Get all sketch IDs
     */
    std::vector<std::string> getSketchIds() const;

    /**
     * @brief Get number of sketches
     */
    size_t sketchCount() const { return sketches_.size(); }

    /**
     * @brief Remove sketch by ID
     * @return true if removed, false if not found
     */
    bool removeSketch(const std::string& id);

    /**
     * @brief Get sketch name (for navigator display)
     */
    std::string getSketchName(const std::string& id) const;

    /**
     * @brief Set sketch name
     */
    void setSketchName(const std::string& id, const std::string& name);

    // Document state
    bool isModified() const { return modified_; }
    void setModified(bool modified);
    void clear();

    // Serialization
    std::string toJson() const;
    static std::unique_ptr<Document> fromJson(const std::string& json, QObject* parent = nullptr);

    // Body management
    std::string addBody(const TopoDS_Shape& shape);
    bool addBodyWithId(const std::string& id, const TopoDS_Shape& shape, const std::string& name = {});
    const TopoDS_Shape* getBodyShape(const std::string& id) const;
    std::vector<std::string> getBodyIds() const;
    bool removeBody(const std::string& id);
    std::string getBodyName(const std::string& id) const;
    void setBodyName(const std::string& id, const std::string& name);
    size_t bodyCount() const { return bodies_.size(); }

    void addOperation(const OperationRecord& record);
    const std::vector<OperationRecord>& operations() const { return operations_; }

    render::SceneMeshStore& meshStore() { return *sceneMeshStore_; }
    const render::SceneMeshStore& meshStore() const { return *sceneMeshStore_; }
    kernel::elementmap::ElementMap& elementMap() { return elementMap_; }
    const kernel::elementmap::ElementMap& elementMap() const { return elementMap_; }

signals:
    void sketchAdded(const QString& id);
    void sketchRemoved(const QString& id);
    void sketchRenamed(const QString& id, const QString& newName);
    void bodyAdded(const QString& id);
    void bodyRemoved(const QString& id);
    void bodyRenamed(const QString& id, const QString& newName);
    void bodyUpdated(const QString& id);
    void modifiedChanged(bool modified);
    void documentCleared();

private:
    struct BodyEntry {
        // Placeholder for future metadata (material, visibility, transform).
        TopoDS_Shape shape;
    };

    void registerBodyElements(const std::string& bodyId, const TopoDS_Shape& shape);
    void updateBodyMesh(const std::string& bodyId, const TopoDS_Shape& shape, bool emitSignal = true);
    void rebuildElementMap();

    std::unordered_map<std::string, std::unique_ptr<core::sketch::Sketch>> sketches_;
    std::unordered_map<std::string, std::string> sketchNames_;  // id -> display name
    std::unordered_map<std::string, BodyEntry> bodies_;
    std::unordered_map<std::string, std::string> bodyNames_;  // id -> display name
    std::vector<OperationRecord> operations_;
    kernel::elementmap::ElementMap elementMap_;
    std::unique_ptr<render::SceneMeshStore> sceneMeshStore_;
    std::unique_ptr<render::TessellationCache> tessellationCache_;
    bool modified_ = false;
    unsigned int nextSketchNumber_ = 1;
    unsigned int nextBodyNumber_ = 1;
};

} // namespace onecad::app

#endif // ONECAD_APP_DOCUMENT_DOCUMENT_H
