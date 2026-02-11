/**
 * @file RegenerationEngine.cpp
 * @brief Implementation of RegenerationEngine.
 */
#include "RegenerationEngine.h"

#include "../document/Document.h"
#include "../../core/loop/RegionUtils.h"
#include "../../core/loop/FaceBuilder.h"
#include "../../core/modeling/EdgeChainer.h"
#include "../../core/sketch/Sketch.h"
#include "../../core/sketch/SketchLine.h"
#include "../../core/sketch/SketchPoint.h"

#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepOffsetAPI_MakeThickSolid.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRepOffsetAPI_DraftAngle.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>

#include <algorithm>
#include <cmath>
#include <unordered_set>

namespace onecad::app::history {

namespace {
constexpr double kDraftAngleEpsilon = 1e-4;
constexpr double kSideFaceDotThreshold = 0.9;
constexpr double kMinValue = 1e-3;
constexpr double kMinAngleDeg = 1e-3;
} // namespace

RegenerationEngine::RegenerationEngine(Document* doc)
    : doc_(doc), graph_() {
    if (doc_) {
        graph_.rebuildFromOperations(doc_->operations());
        for (const auto& op : doc_->operations()) {
            if (doc_->isOperationSuppressed(op.opId)) {
                graph_.setSuppressed(op.opId, true);
            }
        }
    }
}

RegenResult RegenerationEngine::regenerateAll() {
    RegenResult result;

    if (!doc_) {
        result.status = RegenStatus::CriticalFailure;
        return result;
    }

    // Rebuild dependency graph from current operations
    graph_.rebuildFromOperations(doc_->operations());
    for (const auto& op : doc_->operations()) {
        if (doc_->isOperationSuppressed(op.opId)) {
            graph_.setSuppressed(op.opId, true);
        }
    }
    graph_.clearFailures();
    doc_->clearOperationFailures();

    // Get topological sort order
    std::vector<std::string> order = graph_.topologicalSort();
    if (order.empty() && graph_.size() > 0) {
        // Cycle detected
        result.status = RegenStatus::CriticalFailure;
        return result;
    }

    std::unordered_set<std::string> expectedBodies;
    expectedBodies.reserve(doc_->operations().size());
    for (const auto& op : doc_->operations()) {
        for (const auto& bodyId : op.resultBodyIds) {
            expectedBodies.insert(bodyId);
        }
    }
    for (const auto& bodyId : doc_->baseBodyIds()) {
        expectedBodies.insert(bodyId);
    }

    // Execute operations in order
    const int total = static_cast<int>(order.size());
    int current = 0;
    std::unordered_set<std::string> updatedBodies;
    for (const auto& bodyId : doc_->baseBodyIds()) {
        updatedBodies.insert(bodyId);
    }

    for (const auto& opId : order) {
        ++current;
        if (progressCallback_) {
            progressCallback_(current, total, opId);
        }

        // Skip suppressed operations
        if (graph_.isSuppressed(opId)) {
            result.skippedOps.push_back(opId);
            doc_->clearOperationFailed(opId);
            continue;
        }

        // Find the operation record
        const OperationRecord* opRecord = doc_->findOperation(opId);

        if (!opRecord) {
            result.skippedOps.push_back(opId);
            continue;
        }

        // Check if any upstream dependency failed
        bool upstreamFailed = false;
        for (const auto& upstreamId : graph_.getUpstream(opId)) {
            if (graph_.isFailed(upstreamId)) {
                upstreamFailed = true;
                break;
            }
        }

        if (upstreamFailed) {
            graph_.setFailed(opId, true, "Upstream operation failed");
            doc_->setOperationFailed(opId, "Upstream operation failed");
            result.skippedOps.push_back(opId);
            continue;
        }

        // Execute the operation
        std::string errorMsg;
        bool success = executeOperation(*opRecord, errorMsg);

        if (success) {
            result.succeededOps.push_back(opId);
            doc_->clearOperationFailed(opId);
            for (const auto& bodyId : opRecord->resultBodyIds) {
                updatedBodies.insert(bodyId);
            }
        } else {
            graph_.setFailed(opId, true, errorMsg);
            doc_->setOperationFailed(opId, errorMsg);
            FailedOp failedOp;
            failedOp.opId = opId;
            failedOp.type = opRecord->type;
            failedOp.errorMessage = errorMsg;
            failedOp.affectedDownstream = graph_.getDownstream(opId);
            result.failedOps.push_back(std::move(failedOp));
        }
    }

    // Determine overall status
    if (result.failedOps.empty()) {
        result.status = RegenStatus::Success;
    } else if (!result.succeededOps.empty()) {
        result.status = RegenStatus::PartialFailure;
    } else {
        result.status = RegenStatus::CriticalFailure;
    }

    // Remove bodies not produced during this regeneration.
    for (const auto& bodyId : doc_->getBodyIds()) {
        if (expectedBodies.find(bodyId) == expectedBodies.end()) {
            doc_->removeBody(bodyId);
            continue;
        }
        if (updatedBodies.find(bodyId) == updatedBodies.end()) {
            doc_->removeBodyPreserveElementMap(bodyId);
        }
    }

    return result;
}

RegenResult RegenerationEngine::regenerateFrom(const std::string& opId) {
    RegenResult result;

    if (!doc_) {
        result.status = RegenStatus::CriticalFailure;
        return result;
    }

    // Ensure graph is up to date
    graph_.rebuildFromOperations(doc_->operations());
    for (const auto& op : doc_->operations()) {
        if (doc_->isOperationSuppressed(op.opId)) {
            graph_.setSuppressed(op.opId, true);
        }
    }

    // Get ops that need regeneration: the op itself + all downstream
    std::vector<std::string> opsToRegen = {opId};
    auto downstream = graph_.getDownstream(opId);
    opsToRegen.insert(opsToRegen.end(), downstream.begin(), downstream.end());

    // Clear failure states for affected ops
    for (const auto& id : opsToRegen) {
        graph_.setFailed(id, false);
        doc_->clearOperationFailed(id);
    }

    // Get topological order of all ops
    std::vector<std::string> fullOrder = graph_.topologicalSort();
    if (fullOrder.empty() && graph_.size() > 0) {
        result.status = RegenStatus::CriticalFailure;
        return result;
    }

    // Execute only affected ops in topological order
    std::unordered_set<std::string> affectedSet(opsToRegen.begin(), opsToRegen.end());
    const int total = static_cast<int>(opsToRegen.size());
    int current = 0;
    std::unordered_set<std::string> updatedBodies;

    for (const auto& currentOpId : fullOrder) {
        if (affectedSet.find(currentOpId) == affectedSet.end()) {
            continue;
        }

        ++current;
        if (progressCallback_) {
            progressCallback_(current, total, currentOpId);
        }

        if (graph_.isSuppressed(currentOpId)) {
            result.skippedOps.push_back(currentOpId);
            doc_->clearOperationFailed(currentOpId);
            continue;
        }

        const OperationRecord* opRecord = doc_->findOperation(currentOpId);

        if (!opRecord) {
            result.skippedOps.push_back(currentOpId);
            continue;
        }

        bool upstreamFailed = false;
        for (const auto& upId : graph_.getUpstream(currentOpId)) {
            if (graph_.isFailed(upId)) {
                upstreamFailed = true;
                break;
            }
        }

        if (upstreamFailed) {
            graph_.setFailed(currentOpId, true, "Upstream operation failed");
            doc_->setOperationFailed(currentOpId, "Upstream operation failed");
            result.skippedOps.push_back(currentOpId);
            continue;
        }

        std::string errorMsg;
        bool success = executeOperation(*opRecord, errorMsg);

        if (success) {
            result.succeededOps.push_back(currentOpId);
            doc_->clearOperationFailed(currentOpId);
            for (const auto& bodyId : opRecord->resultBodyIds) {
                updatedBodies.insert(bodyId);
            }
        } else {
            graph_.setFailed(currentOpId, true, errorMsg);
            doc_->setOperationFailed(currentOpId, errorMsg);
            FailedOp failedOp;
            failedOp.opId = currentOpId;
            failedOp.type = opRecord->type;
            failedOp.errorMessage = errorMsg;
            failedOp.affectedDownstream = graph_.getDownstream(currentOpId);
            result.failedOps.push_back(std::move(failedOp));
        }
    }

    if (result.failedOps.empty()) {
        result.status = RegenStatus::Success;
    } else if (!result.succeededOps.empty()) {
        result.status = RegenStatus::PartialFailure;
    } else {
        result.status = RegenStatus::CriticalFailure;
    }

    return result;
}

RegenResult RegenerationEngine::previewFrom(const std::string& opId, const OperationParams& newParams) {
    if (!doc_) {
        RegenResult result;
        result.status = RegenStatus::CriticalFailure;
        return result;
    }

    // Backup current state
    backupCurrentState();
    previewActive_ = true;
    previewOpId_ = opId;

    // Find and temporarily modify the operation
    if (auto* op = doc_->findOperation(opId)) {
        previewOriginalParams_ = op->params;
        op->params = newParams;
    }

    // Regenerate from this op
    return regenerateFrom(opId);
}

void RegenerationEngine::commitPreview() {
    if (!previewActive_) {
        return;
    }

    // Clear backup (keep current state)
    backupShapes_.clear();
    previewActive_ = false;
    previewOpId_.clear();
}

void RegenerationEngine::discardPreview() {
    if (!previewActive_ || !doc_) {
        return;
    }

    // Restore original params
    if (auto* op = doc_->findOperation(previewOpId_)) {
        op->params = previewOriginalParams_;
    }

    // Restore body shapes
    restoreBackupState();

    previewActive_ = false;
    previewOpId_.clear();
    backupShapes_.clear();
}

std::optional<TopoDS_Shape> RegenerationEngine::resolveEdge(const std::string& edgeId) const {
    if (!doc_) {
        return std::nullopt;
    }

    const auto* entry = doc_->elementMap().find(kernel::elementmap::ElementId{edgeId});
    if (!entry || entry->kind != kernel::elementmap::ElementKind::Edge || entry->shape.IsNull()) {
        return std::nullopt;
    }

    return entry->shape;
}

std::optional<TopoDS_Shape> RegenerationEngine::resolveFace(const std::string& faceId) const {
    if (!doc_) {
        return std::nullopt;
    }

    const auto* entry = doc_->elementMap().find(kernel::elementmap::ElementId{faceId});
    if (!entry || entry->kind != kernel::elementmap::ElementKind::Face || entry->shape.IsNull()) {
        return std::nullopt;
    }

    return entry->shape;
}

std::optional<TopoDS_Shape> RegenerationEngine::resolveBody(const std::string& bodyId) const {
    if (!doc_) {
        return std::nullopt;
    }

    const TopoDS_Shape* shape = doc_->getBodyShape(bodyId);
    if (!shape || shape->IsNull()) {
        return std::nullopt;
    }

    return *shape;
}

bool RegenerationEngine::executeOperation(const OperationRecord& op, std::string& errorOut) {
    TopoDS_Shape result;

    switch (op.type) {
    case OperationType::Extrude:
        result = buildExtrude(op, errorOut);
        break;
    case OperationType::Revolve:
        result = buildRevolve(op, errorOut);
        break;
    case OperationType::Fillet:
        result = buildFillet(op, errorOut);
        break;
    case OperationType::Chamfer:
        result = buildChamfer(op, errorOut);
        break;
    case OperationType::Shell:
        result = buildShell(op, errorOut);
        break;
    case OperationType::Boolean:
        result = buildBoolean(op, errorOut);
        break;
    default:
        errorOut = "Unknown operation type";
        return false;
    }

    if (result.IsNull()) {
        if (errorOut.empty()) {
            errorOut = "Operation produced null shape";
        }
        return false;
    }

    // Apply result to document
    for (const auto& bodyId : op.resultBodyIds) {
        applyBodyResult(bodyId, result, op.opId);
    }

    return true;
}

TopoDS_Shape RegenerationEngine::buildExtrude(const OperationRecord& op, std::string& errorOut) {
    if (!std::holds_alternative<ExtrudeParams>(op.params)) {
        errorOut = "Invalid params for extrude";
        return {};
    }

    const auto& params = std::get<ExtrudeParams>(op.params);
    if (std::abs(params.distance) < kMinValue) {
        errorOut = "Extrude distance too small";
        return {};
    }

    // Resolve input face
    auto faceOpt = resolveFaceInput(op.input, errorOut);
    if (!faceOpt) {
        return {};
    }

    TopoDS_Face baseFace = *faceOpt;

    // Get direction from face normal
    BRepAdaptor_Surface surface(baseFace, true);
    if (surface.GetType() != GeomAbs_Plane) {
        errorOut = "Only planar faces supported for extrusion";
        return {};
    }

    gp_Pln plane = surface.Plane();
    gp_Dir direction = plane.Axis().Direction();
    if (baseFace.Orientation() == TopAbs_REVERSED) {
        direction.Reverse();
    }

    // Build prism
    gp_Vec prismVec(direction.X() * params.distance,
                    direction.Y() * params.distance,
                    direction.Z() * params.distance);

    BRepPrimAPI_MakePrism prism(baseFace, prismVec, true);
    TopoDS_Shape result = prism.Shape();

    // Apply draft angle if specified
    if (std::abs(params.draftAngleDeg) > kDraftAngleEpsilon) {
        const double angleRad = params.draftAngleDeg * M_PI / 180.0;
        gp_Dir draftDir = direction;
        if (params.distance < 0.0) {
            draftDir.Reverse();
        }

        BRepOffsetAPI_DraftAngle draft(result);
        gp_Pln neutralPlane = plane;

        for (TopExp_Explorer exp(result, TopAbs_FACE); exp.More(); exp.Next()) {
            TopoDS_Face face = TopoDS::Face(exp.Current());
            BRepAdaptor_Surface faceSurface(face, true);
            if (faceSurface.GetType() != GeomAbs_Plane) {
                continue;
            }
            gp_Pln facePlane = faceSurface.Plane();
            gp_Dir faceNormal = facePlane.Axis().Direction();
            if (face.Orientation() == TopAbs_REVERSED) {
                faceNormal.Reverse();
            }
            const double dot = std::abs(faceNormal.Dot(draftDir));
            if (dot > kSideFaceDotThreshold) {
                continue;
            }

            draft.Add(face, draftDir, angleRad, neutralPlane, true);
            if (!draft.AddDone()) {
                draft.Remove(face);
            }
        }

        draft.Build();
        if (draft.IsDone()) {
            result = draft.Shape();
        }
    }

    // Handle boolean mode
    if (params.booleanMode != BooleanMode::NewBody) {
        const std::string targetBodyId = resolveBooleanTargetBodyId(op, params.targetBodyId);
        if (targetBodyId.empty()) {
            errorOut = "Boolean target body is required for mode " +
                       std::string(booleanModeName(params.booleanMode));
            return {};
        }

        auto targetOpt = resolveBody(targetBodyId);
        if (!targetOpt) {
            errorOut = "Target body not found: " + targetBodyId;
            return {};
        }

        if (params.booleanMode == BooleanMode::Add) {
            BRepAlgoAPI_Fuse fuse(*targetOpt, result);
            if (fuse.IsDone()) {
                result = fuse.Shape();
            }
        } else if (params.booleanMode == BooleanMode::Cut) {
            BRepAlgoAPI_Cut cut(*targetOpt, result);
            if (cut.IsDone()) {
                result = cut.Shape();
            }
        } else if (params.booleanMode == BooleanMode::Intersect) {
            BRepAlgoAPI_Common common(*targetOpt, result);
            if (common.IsDone()) {
                result = common.Shape();
            }
        }
    }

    return result;
}

TopoDS_Shape RegenerationEngine::buildRevolve(const OperationRecord& op, std::string& errorOut) {
    if (!std::holds_alternative<RevolveParams>(op.params)) {
        errorOut = "Invalid params for revolve";
        return {};
    }

    const auto& params = std::get<RevolveParams>(op.params);
    if (std::abs(params.angleDeg) < kMinAngleDeg) {
        errorOut = "Revolve angle too small";
        return {};
    }

    auto faceOpt = resolveFaceInput(op.input, errorOut);
    if (!faceOpt) {
        return {};
    }

    TopoDS_Face baseFace = *faceOpt;

    // Resolve axis
    gp_Ax1 axis;
    bool axisResolved = false;

    if (std::holds_alternative<SketchLineRef>(params.axis)) {
        const auto& lineRef = std::get<SketchLineRef>(params.axis);
        core::sketch::Sketch* sketch = doc_->getSketch(lineRef.sketchId);
        if (!sketch) {
            errorOut = "Sketch not found: " + lineRef.sketchId;
            return {};
        }

        // Get line entity from sketch
        const auto* line = sketch->getEntityAs<core::sketch::SketchLine>(lineRef.lineId);
        if (!line) {
            errorOut = "Axis line not found: " + lineRef.lineId;
            return {};
        }

        // Get endpoints
        const auto* startPt = sketch->getEntityAs<core::sketch::SketchPoint>(line->startPointId());
        const auto* endPt = sketch->getEntityAs<core::sketch::SketchPoint>(line->endPointId());
        if (!startPt || !endPt) {
            errorOut = "Could not resolve axis line endpoints";
            return {};
        }

        const auto& sketchPlane = sketch->getPlane();
        const gp_Pnt2d& p1 = startPt->position();
        const gp_Pnt2d& p2 = endPt->position();

        gp_Pnt origin(sketchPlane.origin.x + p1.X() * sketchPlane.xAxis.x + p1.Y() * sketchPlane.yAxis.x,
                      sketchPlane.origin.y + p1.X() * sketchPlane.xAxis.y + p1.Y() * sketchPlane.yAxis.y,
                      sketchPlane.origin.z + p1.X() * sketchPlane.xAxis.z + p1.Y() * sketchPlane.yAxis.z);

        gp_Vec dir((p2.X() - p1.X()) * sketchPlane.xAxis.x + (p2.Y() - p1.Y()) * sketchPlane.yAxis.x,
                   (p2.X() - p1.X()) * sketchPlane.xAxis.y + (p2.Y() - p1.Y()) * sketchPlane.yAxis.y,
                   (p2.X() - p1.X()) * sketchPlane.xAxis.z + (p2.Y() - p1.Y()) * sketchPlane.yAxis.z);

        if (dir.Magnitude() > 1e-6) {
            axis = gp_Ax1(origin, gp_Dir(dir));
            axisResolved = true;
        }
    } else if (std::holds_alternative<EdgeRef>(params.axis)) {
        const auto& edgeRef = std::get<EdgeRef>(params.axis);
        auto edgeOpt = resolveEdge(edgeRef.edgeId);
        if (!edgeOpt) {
            errorOut = "Axis edge not found: " + edgeRef.edgeId;
            return {};
        }

        TopoDS_Edge edge = TopoDS::Edge(*edgeOpt);
        BRepAdaptor_Curve curve(edge);

        if (curve.GetType() == GeomAbs_Line) {
            gp_Lin line = curve.Line();
            axis = gp_Ax1(line.Location(), line.Direction());
            axisResolved = true;
        } else {
            errorOut = "Axis edge must be a straight line";
            return {};
        }
    }

    if (!axisResolved) {
        errorOut = "Could not resolve revolution axis";
        return {};
    }

    const double angleRad = params.angleDeg * M_PI / 180.0;
    BRepPrimAPI_MakeRevol revol(baseFace, axis, angleRad, true);

    if (!revol.IsDone()) {
        errorOut = "Revolve operation failed";
        return {};
    }

    TopoDS_Shape result = revol.Shape();

    if (params.booleanMode != BooleanMode::NewBody) {
        const std::string targetBodyId = resolveBooleanTargetBodyId(op, params.targetBodyId);
        if (targetBodyId.empty()) {
            errorOut = "Boolean target body is required for mode " +
                       std::string(booleanModeName(params.booleanMode));
            return {};
        }

        auto targetOpt = resolveBody(targetBodyId);
        if (!targetOpt) {
            errorOut = "Target body not found: " + targetBodyId;
            return {};
        }

        if (params.booleanMode == BooleanMode::Add) {
            BRepAlgoAPI_Fuse fuse(*targetOpt, result);
            if (fuse.IsDone()) {
                result = fuse.Shape();
            }
        } else if (params.booleanMode == BooleanMode::Cut) {
            BRepAlgoAPI_Cut cut(*targetOpt, result);
            if (cut.IsDone()) {
                result = cut.Shape();
            }
        } else if (params.booleanMode == BooleanMode::Intersect) {
            BRepAlgoAPI_Common common(*targetOpt, result);
            if (common.IsDone()) {
                result = common.Shape();
            }
        }
    }

    return result;
}

TopoDS_Shape RegenerationEngine::buildFillet(const OperationRecord& op, std::string& errorOut) {
    if (!std::holds_alternative<FilletChamferParams>(op.params)) {
        errorOut = "Invalid params for fillet";
        return {};
    }

    const auto& params = std::get<FilletChamferParams>(op.params);
    if (params.mode != FilletChamferParams::Mode::Fillet) {
        errorOut = "Expected Fillet mode";
        return {};
    }

    // Get target body
    std::string targetBodyId;
    if (std::holds_alternative<BodyRef>(op.input)) {
        targetBodyId = std::get<BodyRef>(op.input).bodyId;
    } else {
        errorOut = "Fillet requires body input";
        return {};
    }

    auto bodyOpt = resolveBody(targetBodyId);
    if (!bodyOpt) {
        errorOut = "Target body not found: " + targetBodyId;
        return {};
    }

    TopoDS_Shape targetShape = *bodyOpt;

    if (params.radius < kMinValue) {
        errorOut = "Fillet radius too small";
        return {};
    }

    try {
        BRepFilletAPI_MakeFillet fillet(targetShape);
        std::size_t addedEdges = 0;

        for (const auto& edgeId : params.edgeIds) {
            auto edgeOpt = resolveEdge(edgeId);
            if (!edgeOpt) {
                continue;
            }
            TopoDS_Edge edge = TopoDS::Edge(*edgeOpt);
            fillet.Add(params.radius, edge);
            ++addedEdges;
        }

        if (addedEdges == 0) {
            errorOut = "No valid edges for fillet";
            return {};
        }

        fillet.Build();
        if (fillet.IsDone()) {
            return fillet.Shape();
        }

        errorOut = "Fillet operation failed";
    } catch (...) {
        errorOut = "Fillet operation failed (radius too large?)";
    }

    return {};
}

TopoDS_Shape RegenerationEngine::buildChamfer(const OperationRecord& op, std::string& errorOut) {
    if (!std::holds_alternative<FilletChamferParams>(op.params)) {
        errorOut = "Invalid params for chamfer";
        return {};
    }

    const auto& params = std::get<FilletChamferParams>(op.params);
    if (params.mode != FilletChamferParams::Mode::Chamfer) {
        errorOut = "Expected Chamfer mode";
        return {};
    }

    std::string targetBodyId;
    if (std::holds_alternative<BodyRef>(op.input)) {
        targetBodyId = std::get<BodyRef>(op.input).bodyId;
    } else {
        errorOut = "Chamfer requires body input";
        return {};
    }

    auto bodyOpt = resolveBody(targetBodyId);
    if (!bodyOpt) {
        errorOut = "Target body not found: " + targetBodyId;
        return {};
    }

    TopoDS_Shape targetShape = *bodyOpt;

    if (params.radius < kMinValue) {
        errorOut = "Chamfer distance too small";
        return {};
    }

    try {
        BRepFilletAPI_MakeChamfer chamfer(targetShape);
        std::size_t addedEdges = 0;

        TopTools_IndexedDataMapOfShapeListOfShape edgeFaceMap;
        TopExp::MapShapesAndAncestors(targetShape, TopAbs_EDGE, TopAbs_FACE, edgeFaceMap);

        for (const auto& edgeId : params.edgeIds) {
            auto edgeOpt = resolveEdge(edgeId);
            if (!edgeOpt) {
                continue;
            }
            TopoDS_Edge edge = TopoDS::Edge(*edgeOpt);

            int idx = edgeFaceMap.FindIndex(edge);
            if (idx == 0) {
                continue;
            }

            const TopTools_ListOfShape& faces = edgeFaceMap(idx);
            if (faces.IsEmpty()) {
                continue;
            }

            TopoDS_Face refFace = TopoDS::Face(faces.First());
            chamfer.Add(params.radius, params.radius, edge, refFace);
            ++addedEdges;
        }

        if (addedEdges == 0) {
            errorOut = "No valid edges for chamfer";
            return {};
        }

        chamfer.Build();
        if (chamfer.IsDone()) {
            return chamfer.Shape();
        }

        errorOut = "Chamfer operation failed";
    } catch (...) {
        errorOut = "Chamfer operation failed";
    }

    return {};
}

TopoDS_Shape RegenerationEngine::buildShell(const OperationRecord& op, std::string& errorOut) {
    if (!std::holds_alternative<ShellParams>(op.params)) {
        errorOut = "Invalid params for shell";
        return {};
    }

    const auto& params = std::get<ShellParams>(op.params);

    std::string targetBodyId;
    if (std::holds_alternative<BodyRef>(op.input)) {
        targetBodyId = std::get<BodyRef>(op.input).bodyId;
    } else {
        errorOut = "Shell requires body input";
        return {};
    }

    auto bodyOpt = resolveBody(targetBodyId);
    if (!bodyOpt) {
        errorOut = "Target body not found: " + targetBodyId;
        return {};
    }

    TopoDS_Shape targetShape = *bodyOpt;

    if (params.thickness < kMinValue) {
        errorOut = "Shell thickness too small";
        return {};
    }

    try {
        TopTools_ListOfShape facesToRemove;
        std::size_t addedFaces = 0;
        for (const auto& faceId : params.openFaceIds) {
            auto faceOpt = resolveFace(faceId);
            if (faceOpt) {
                facesToRemove.Append(*faceOpt);
                ++addedFaces;
            }
        }

        if (addedFaces == 0) {
            errorOut = "No valid faces for shell";
            return {};
        }

        BRepOffsetAPI_MakeThickSolid thickSolid;
        thickSolid.MakeThickSolidByJoin(targetShape, facesToRemove, -params.thickness,
                                         1e-3, BRepOffset_Skin, false, false,
                                         GeomAbs_Arc, false);

        if (thickSolid.IsDone()) {
            return thickSolid.Shape();
        }

        errorOut = "Shell operation failed";
    } catch (...) {
        errorOut = "Shell operation failed";
    }

    return {};
}

TopoDS_Shape RegenerationEngine::buildBoolean(const OperationRecord& op, std::string& errorOut) {
    if (!std::holds_alternative<BooleanParams>(op.params)) {
        errorOut = "Invalid params for boolean";
        return {};
    }

    const auto& params = std::get<BooleanParams>(op.params);

    auto targetOpt = resolveBody(params.targetBodyId);
    if (!targetOpt) {
        errorOut = "Target body not found: " + params.targetBodyId;
        return {};
    }

    auto toolOpt = resolveBody(params.toolBodyId);
    if (!toolOpt) {
        errorOut = "Tool body not found: " + params.toolBodyId;
        return {};
    }

    try {
        switch (params.operation) {
        case BooleanParams::Op::Union: {
            BRepAlgoAPI_Fuse fuse(*targetOpt, *toolOpt);
            if (fuse.IsDone()) {
                return fuse.Shape();
            }
            break;
        }
        case BooleanParams::Op::Cut: {
            BRepAlgoAPI_Cut cut(*targetOpt, *toolOpt);
            if (cut.IsDone()) {
                return cut.Shape();
            }
            break;
        }
        case BooleanParams::Op::Intersect: {
            BRepAlgoAPI_Common common(*targetOpt, *toolOpt);
            if (common.IsDone()) {
                return common.Shape();
            }
            break;
        }
        }

        errorOut = "Boolean operation failed";
    } catch (...) {
        errorOut = "Boolean operation failed";
    }

    return {};
}

std::string RegenerationEngine::resolveLegacySketchHostBodyId(const OperationInput& input) const {
    if (!doc_ || !std::holds_alternative<SketchRegionRef>(input)) {
        return {};
    }

    const auto& ref = std::get<SketchRegionRef>(input);
    const core::sketch::Sketch* sketch = doc_->getSketch(ref.sketchId);
    if (!sketch) {
        return {};
    }

    const auto& hostFace = sketch->hostFaceAttachment();
    if (!hostFace || !hostFace->isValid()) {
        return {};
    }

    return hostFace->bodyId;
}

std::string RegenerationEngine::resolveBooleanTargetBodyId(const OperationRecord& op,
                                                           const std::string& explicitTargetBodyId) const {
    if (!explicitTargetBodyId.empty()) {
        return explicitTargetBodyId;
    }

    if (std::holds_alternative<FaceRef>(op.input)) {
        const auto& face = std::get<FaceRef>(op.input);
        if (!face.bodyId.empty()) {
            return face.bodyId;
        }
    }

    return resolveLegacySketchHostBodyId(op.input);
}

std::optional<TopoDS_Face> RegenerationEngine::resolveFaceInput(const OperationInput& input, std::string& errorOut) {
    if (std::holds_alternative<SketchRegionRef>(input)) {
        const auto& ref = std::get<SketchRegionRef>(input);
        return buildFaceFromSketchRegion(ref.sketchId, ref.regionId, errorOut);
    } else if (std::holds_alternative<FaceRef>(input)) {
        const auto& ref = std::get<FaceRef>(input);
        auto faceOpt = resolveFace(ref.faceId);
        if (!faceOpt) {
            errorOut = "Face not found: " + ref.faceId;
            return std::nullopt;
        }
        return TopoDS::Face(*faceOpt);
    }

    errorOut = "Invalid input type for face resolution";
    return std::nullopt;
}

std::optional<TopoDS_Face> RegenerationEngine::buildFaceFromSketchRegion(const std::string& sketchId,
                                                                          const std::string& regionId,
                                                                          std::string& errorOut) {
    if (!doc_) {
        errorOut = "No document";
        return std::nullopt;
    }

    core::sketch::Sketch* sketch = doc_->getSketch(sketchId);
    if (!sketch) {
        errorOut = "Sketch not found: " + sketchId;
        return std::nullopt;
    }

    auto faceOpt = core::loop::resolveRegionFace(*sketch, regionId);
    if (!faceOpt) {
        errorOut = "Region not found: " + regionId;
        return std::nullopt;
    }

    core::loop::FaceBuilder builder;
    auto faceResult = builder.buildFace(*faceOpt, *sketch);
    if (!faceResult.success) {
        errorOut = faceResult.errorMessage.empty() ? "Face build failed" : faceResult.errorMessage;
        return std::nullopt;
    }

    return faceResult.face;
}

void RegenerationEngine::backupCurrentState() {
    if (!doc_) {
        return;
    }

    backupShapes_.clear();
    for (const auto& bodyId : doc_->getBodyIds()) {
        const TopoDS_Shape* shape = doc_->getBodyShape(bodyId);
        if (shape && !shape->IsNull()) {
            backupShapes_[bodyId] = *shape;
        }
    }
}

void RegenerationEngine::restoreBackupState() {
    if (!doc_) {
        return;
    }

    // Remove current bodies
    for (const auto& bodyId : doc_->getBodyIds()) {
        doc_->removeBody(bodyId);
    }

    // Restore backed up bodies
    for (const auto& [bodyId, shape] : backupShapes_) {
        doc_->addBodyWithId(bodyId, shape);
    }
}

void RegenerationEngine::applyBodyResult(const std::string& bodyId, const TopoDS_Shape& shape,
                                          const std::string& opId) {
    if (!doc_) {
        return;
    }

    if (doc_->getBodyShape(bodyId)) {
        doc_->updateBodyShape(bodyId, shape, true, opId);
    } else {
        doc_->addBodyWithId(bodyId, shape);
        doc_->elementMap().rebindBody(bodyId, shape, opId);
    }
}

} // namespace onecad::app::history
