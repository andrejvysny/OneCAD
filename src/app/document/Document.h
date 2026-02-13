/**
 * @file Document.h
 * @brief Document model for storing sketches and bodies
 */

#ifndef ONECAD_APP_DOCUMENT_DOCUMENT_H
#define ONECAD_APP_DOCUMENT_DOCUMENT_H

#include <QObject>
#include <QString>
#include <memory>
#include <optional>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include <TopoDS_Shape.hxx>

#include "OperationRecord.h"
#include "OperationMetadata.h"
#include "../../core/sketch/Sketch.h"
#include "../../kernel/elementmap/ElementMap.h"
#include "../../render/scene/SceneMeshStore.h"
#include "../../render/tessellation/TessellationCache.h"

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
     * @brief Add sketch with explicit ID (for undo/redo)
     * @return true if added successfully
     */
    bool addSketchWithId(const std::string& id,
                         std::unique_ptr<core::sketch::Sketch> sketch,
                         const std::string& name = {});

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
    bool updateBodyShape(const std::string& id, const TopoDS_Shape& shape,
                         bool emitSignal = true, const std::string& opId = {});
    const TopoDS_Shape* getBodyShape(const std::string& id) const;
    std::optional<core::sketch::SketchPlane> getSketchPlaneForFace(const std::string& bodyId,
                                                                    const std::string& faceId) const;
    bool ensureHostFaceBoundariesProjected(const std::string& sketchId);
    bool projectHostFaceBoundaries(core::sketch::Sketch& sketch,
                                   const std::string& bodyId,
                                   const std::string& faceId);
    std::vector<std::string> getBodyIds() const;
    bool removeBody(const std::string& id);
    bool removeBodyPreserveElementMap(const std::string& id);
    std::string getBodyName(const std::string& id) const;
    void setBodyName(const std::string& id, const std::string& name);
    size_t bodyCount() const { return bodies_.size(); }
    void setBaseBodyIds(const std::unordered_set<std::string>& ids);
    void addBaseBodyId(const std::string& id);
    bool isBaseBody(const std::string& id) const;
    const std::unordered_set<std::string>& baseBodyIds() const { return baseBodyIds_; }

    void addOperation(const OperationRecord& record);
    bool insertOperation(std::size_t index, const OperationRecord& record);
    bool updateOperationParams(const std::string& opId, const OperationParams& params);
    bool removeOperation(const std::string& opId);
    int operationIndex(const std::string& opId) const;
    OperationRecord* findOperation(const std::string& opId);
    const OperationRecord* findOperation(const std::string& opId) const;
    bool setOperationSuppressed(const std::string& opId, bool suppressed);
    bool isOperationSuppressed(const std::string& opId) const;
    std::unordered_map<std::string, bool> operationSuppressionState() const;
    void setOperationSuppressionState(const std::unordered_map<std::string, bool>& state);
    void setOperationFailed(const std::string& opId, const std::string& reason);
    void clearOperationFailed(const std::string& opId);
    void clearOperationFailures();
    bool isOperationFailed(const std::string& opId) const;
    std::string operationFailureReason(const std::string& opId) const;
    const std::unordered_map<std::string, std::string>& operationFailures() const {
        return operationFailures_;
    }
    bool setOperationMetadata(const std::string& opId, const OperationMetadata& metadata);
    std::optional<OperationMetadata> operationMetadata(const std::string& opId) const;
    void clearOperationMetadata(const std::string& opId);
    void clearAllOperationMetadata();
    void setAppliedOpCount(std::size_t count);
    std::size_t appliedOpCount() const { return appliedOpCount_; }
    const std::vector<OperationRecord>& operations() const { return operations_; }

    // Visibility management
    bool isBodyVisible(const std::string& id) const;
    void setBodyVisible(const std::string& id, bool visible);
    bool isSketchVisible(const std::string& id) const;
    void setSketchVisible(const std::string& id, bool visible);

    // Isolation management
    void isolateItem(const std::string& id);
    void clearIsolation();
    bool isIsolationActive() const { return !isolatedItemId_.empty(); }
    std::string isolatedItemId() const { return isolatedItemId_; }

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
    void bodyVisibilityChanged(const QString& id, bool visible);
    void sketchVisibilityChanged(const QString& id, bool visible);
    void isolationChanged();
    void modifiedChanged(bool modified);
    void documentCleared();
    void operationAdded(const QString& opId);
    void operationRemoved(const QString& opId);
    void operationUpdated(const QString& opId);
    void operationSuppressionChanged(const QString& opId, bool suppressed);
    void operationFailed(const QString& opId, const QString& reason);
    void operationSucceeded(const QString& opId);
    void appliedOpCountChanged(qulonglong appliedOpCount);

private:
    struct BodyEntry {
        TopoDS_Shape shape;
        bool visible = true;
    };

    void registerBodyElements(const std::string& bodyId, const TopoDS_Shape& shape);
    void updateBodyMesh(const std::string& bodyId, const TopoDS_Shape& shape, bool emitSignal = true);
    void rebuildElementMap();

    std::unordered_map<std::string, std::unique_ptr<core::sketch::Sketch>> sketches_;
    std::unordered_map<std::string, std::string> sketchNames_;  // id -> display name
    std::unordered_map<std::string, bool> sketchVisibility_;    // id -> visible
    std::unordered_map<std::string, BodyEntry> bodies_;
    std::unordered_map<std::string, std::string> bodyNames_;  // id -> display name
    std::unordered_map<std::string, bool> bodyVisibilityCache_;
    std::unordered_set<std::string> baseBodyIds_;

    // Isolation state (not persisted)
    std::string isolatedItemId_;
    std::unordered_map<std::string, bool> preIsolationBodyVisibility_;
    std::unordered_map<std::string, bool> preIsolationSketchVisibility_;
    std::vector<OperationRecord> operations_;
    std::unordered_set<std::string> suppressedOperations_;
    std::unordered_map<std::string, std::string> operationFailures_;
    std::unordered_map<std::string, OperationMetadata> operationMetadata_;
    std::size_t appliedOpCount_ = 0;
    kernel::elementmap::ElementMap elementMap_;
    std::unique_ptr<render::SceneMeshStore> sceneMeshStore_;
    std::unique_ptr<render::TessellationCache> tessellationCache_;
    bool modified_ = false;
    unsigned int nextSketchNumber_ = 1;
    unsigned int nextBodyNumber_ = 1;
};

} // namespace onecad::app

#endif // ONECAD_APP_DOCUMENT_DOCUMENT_H
