/**
 * @file FilletChamferTool.cpp
 */
#include "FilletChamferTool.h"

#include "../viewport/Viewport.h"
#include "../../app/commands/ModifyBodyCommand.h"
#include "../../app/commands/CommandProcessor.h"
#include "../../app/document/Document.h"
#include "../../core/modeling/EdgeChainer.h"
#include "../../render/Camera3D.h"

#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepExtrema_DistShapeShape.hxx>
#include <TopExp_Explorer.hxx>
#include <TopExp.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Vertex.hxx>
#include <BRep_Tool.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListIteratorOfListOfShape.hxx>

#include <QVector3D>

#include <algorithm>
#include <cmath>
#include <cstdlib>
#include <limits>

namespace onecad::ui::tools {

namespace {
constexpr double kMinValue = 1e-3;
constexpr double kMaxRadius = 1000.0;

TopoDS_Edge pickClosestEdge(const TopoDS_Shape& shape, const gp_Pnt& point) {
    if (shape.IsNull()) {
        return TopoDS_Edge();
    }

    TopoDS_Vertex probe = BRepBuilderAPI_MakeVertex(point);
    double bestDistance = std::numeric_limits<double>::max();
    TopoDS_Edge bestEdge;

    for (TopExp_Explorer exp(shape, TopAbs_EDGE); exp.More(); exp.Next()) {
        TopoDS_Edge edge = TopoDS::Edge(exp.Current());
        if (edge.IsNull()) {
            continue;
        }
        BRepExtrema_DistShapeShape dist(probe, edge);
        dist.Perform();
        if (!dist.IsDone()) {
            continue;
        }
        const double value = dist.Value();
        if (value < bestDistance) {
            bestDistance = value;
            bestEdge = edge;
        }
    }

    return bestEdge;
}
} // namespace

FilletChamferTool::FilletChamferTool(Viewport* viewport, app::Document* document)
    : viewport_(viewport), document_(document) {
}

void FilletChamferTool::setDocument(app::Document* document) {
    document_ = document;
}

void FilletChamferTool::setCommandProcessor(app::commands::CommandProcessor* processor) {
    commandProcessor_ = processor;
}

void FilletChamferTool::begin(const app::selection::SelectionItem& selection) {
    dragging_ = false;
    currentValue_ = 0.0;
    mode_ = Mode::Fillet;
    active_ = prepareInput(selection);
    if (!active_) {
        clearPreview();
    }
}

void FilletChamferTool::cancel() {
    clearPreview();
    active_ = false;
    dragging_ = false;
}

bool FilletChamferTool::handleMousePress(const QPoint& screenPos, Qt::MouseButton button) {
    if (!active_ || button != Qt::LeftButton) {
        return false;
    }

    // Only start dragging if the arrow indicator was clicked
    if (!viewport_ || !viewport_->isMouseOverIndicator(screenPos)) {
        return false;
    }

    dragStart_ = screenPos;
    currentValue_ = 0.0;
    dragging_ = true;
    return true;
}

bool FilletChamferTool::handleMouseMove(const QPoint& screenPos) {
    if (!active_ || !dragging_) {
        return false;
    }

    const double pixelScale = viewport_ ? viewport_->pixelScale() : 1.0;
    const double deltaX = static_cast<double>(screenPos.x() - dragStart_.x());
    const double deltaY = static_cast<double>(screenPos.y() - dragStart_.y());

    // Drag right = fillet (positive), drag left = chamfer (negative)
    const double signedValue = deltaX * pixelScale;
    detectMode(signedValue);
    double value = std::hypot(deltaX, deltaY) * pixelScale;

    if (std::abs(value - currentValue_) < 1e-6) {
        return true;
    }

    currentValue_ = std::clamp(value, 0.0, kMaxRadius);
    if (currentValue_ < kMinValue) {
        clearPreview();
        return true;
    }

    updatePreview(currentValue_);
    return true;
}

bool FilletChamferTool::handleMouseRelease(const QPoint& screenPos, Qt::MouseButton button) {
    if (!active_ || !dragging_ || button != Qt::LeftButton) {
        return false;
    }

    const double pixelScale = viewport_ ? viewport_->pixelScale() : 1.0;
    const double deltaX = static_cast<double>(screenPos.x() - dragStart_.x());
    const double deltaY = static_cast<double>(screenPos.y() - dragStart_.y());
    const double signedValue = deltaX * pixelScale;
    detectMode(signedValue);
    double value = std::hypot(deltaX, deltaY) * pixelScale;
    dragging_ = false;

    if (value < kMinValue) {
        clearPreview();
        return true;
    }

    value = std::clamp(value, kMinValue, kMaxRadius);

    TopoDS_Shape resultShape;
    if (mode_ == Mode::Fillet) {
        resultShape = buildFilletShape(value);
    } else {
        resultShape = buildChamferShape(value);
    }

    if (resultShape.IsNull()) {
        clearPreview();
        return true;
    }

    // Apply via ModifyBodyCommand
    if (document_ && !targetBodyId_.empty()) {
        if (commandProcessor_) {
            auto command = std::make_unique<app::commands::ModifyBodyCommand>(
                document_, targetBodyId_, resultShape);
            commandProcessor_->execute(std::move(command));
        } else {
            std::string name = document_->getBodyName(targetBodyId_);
            document_->removeBody(targetBodyId_);
            document_->addBodyWithId(targetBodyId_, resultShape, name);
        }
    }

    clearPreview();
    currentValue_ = 0.0;
    active_ = false;
    return true;
}

bool FilletChamferTool::prepareInput(const app::selection::SelectionItem& selection) {
    if (!document_) {
        return false;
    }

    targetBodyId_.clear();
    targetShape_.Nullify();
    selectedEdges_.clear();

    if (selection.kind != app::selection::SelectionKind::Edge) {
        return false;
    }

    targetBodyId_ = selection.id.ownerId;
    const TopoDS_Shape* bodyShape = document_->getBodyShape(targetBodyId_);
    if (!bodyShape || bodyShape->IsNull()) {
        return false;
    }
    targetShape_ = *bodyShape;

    TopoDS_Edge seedEdge;

    // Find the edge from elementMap
    const auto* entry = document_->elementMap().find(
        kernel::elementmap::ElementId{selection.id.elementId});
    if (entry && entry->kind == kernel::elementmap::ElementKind::Edge && !entry->shape.IsNull()) {
        seedEdge = TopoDS::Edge(entry->shape);
    }

    TopTools_IndexedMapOfShape edgeMap;
    TopExp::MapShapes(targetShape_, TopAbs_EDGE, edgeMap);

    // Ensure the edge belongs to the current body; fall back if needed.
    if (!seedEdge.IsNull() && !edgeMap.Contains(seedEdge)) {
        seedEdge.Nullify();
    }
    if (seedEdge.IsNull()) {
        const std::string prefix = targetBodyId_ + "/edge/";
        if (selection.id.elementId.rfind(prefix, 0) == 0) {
            const std::string indexStr = selection.id.elementId.substr(prefix.size());
            char* end = nullptr;
            long index = std::strtol(indexStr.c_str(), &end, 10);
            if (end && *end == '\0' && index >= 0) {
                int mapIndex = static_cast<int>(index) + 1;
                if (mapIndex >= 1 && mapIndex <= edgeMap.Extent()) {
                    seedEdge = TopoDS::Edge(edgeMap(mapIndex));
                }
            }
        }
    }
    if (seedEdge.IsNull()) {
        gp_Pnt pickPoint(selection.worldPos.x,
                         selection.worldPos.y,
                         selection.worldPos.z);
        seedEdge = pickClosestEdge(targetShape_, pickPoint);
    }
    if (seedEdge.IsNull()) {
        return false;
    }

    selectedEdges_.push_back(seedEdge);

    // Auto-expand to tangent chain
    expandEdgeChain();

    // Calculate midpoint of first edge for indicator
    if (!selectedEdges_.empty()) {
        BRepAdaptor_Curve curve(selectedEdges_.front());
        double mid = (curve.FirstParameter() + curve.LastParameter()) / 2.0;
        gp_Pnt midPt = curve.Value(mid);
        edgeMidpoint_ = midPt;
    }

    return !selectedEdges_.empty();
}

void FilletChamferTool::expandEdgeChain() {
    if (selectedEdges_.empty() || targetShape_.IsNull()) {
        return;
    }

    auto chainResult = core::modeling::EdgeChainer::buildChain(
        selectedEdges_.front(),
        targetShape_,
        0.9999  // ~0.8 deg tolerance
    );

    if (!chainResult.edges.empty()) {
        selectedEdges_ = std::move(chainResult.edges);
    }
}

void FilletChamferTool::detectMode(double dragDistance) {
    // Positive (right drag) = Fillet, Negative (left drag) = Chamfer
    mode_ = (dragDistance >= 0) ? Mode::Fillet : Mode::Chamfer;
}

void FilletChamferTool::toggleMode() {
    mode_ = (mode_ == Mode::Fillet) ? Mode::Chamfer : Mode::Fillet;
    if (active_ && currentValue_ > kMinValue) {
        updatePreview(currentValue_);
    }
}

void FilletChamferTool::updatePreview(double value) {
    if (!viewport_) {
        return;
    }

    TopoDS_Shape previewShape;
    if (mode_ == Mode::Fillet) {
        previewShape = buildFilletShape(value);
    } else {
        previewShape = buildChamferShape(value);
    }

    if (previewShape.IsNull()) {
        clearPreview();
        return;
    }

    previewElementMap_.clear();
    render::SceneMeshStore::Mesh mesh = previewTessellator_.buildMesh(
        "fillet_preview", previewShape, previewElementMap_);
    viewport_->setModelPreviewMeshes({std::move(mesh)});
    if (!targetBodyId_.empty()) {
        viewport_->setPreviewHiddenBody(targetBodyId_);
    }
}

void FilletChamferTool::clearPreview() {
    if (viewport_) {
        viewport_->clearModelPreviewMeshes();
    }
}

TopoDS_Shape FilletChamferTool::buildFilletShape(double radius) const {
    if (selectedEdges_.empty() || targetShape_.IsNull() || radius < kMinValue) {
        return TopoDS_Shape();
    }

    try {
        BRepFilletAPI_MakeFillet fillet(targetShape_);

        for (const auto& edge : selectedEdges_) {
            fillet.Add(radius, edge);
        }

        fillet.Build();
        if (fillet.IsDone()) {
            return fillet.Shape();
        }
    } catch (...) {
        // Fillet failed (radius too large, geometry issue)
    }

    return TopoDS_Shape();
}

TopoDS_Shape FilletChamferTool::buildChamferShape(double distance) const {
    if (selectedEdges_.empty() || targetShape_.IsNull() || distance < kMinValue) {
        return TopoDS_Shape();
    }

    try {
        BRepFilletAPI_MakeChamfer chamfer(targetShape_);

        // Build edge-to-face map for chamfer (needs reference face)
        TopTools_IndexedDataMapOfShapeListOfShape edgeFaceMap;
        TopExp::MapShapesAndAncestors(targetShape_, TopAbs_EDGE, TopAbs_FACE, edgeFaceMap);

        for (const auto& edge : selectedEdges_) {
            int idx = edgeFaceMap.FindIndex(edge);
            if (idx == 0) continue;

            const TopTools_ListOfShape& faces = edgeFaceMap(idx);
            if (faces.IsEmpty()) continue;

            TopoDS_Face refFace = TopoDS::Face(faces.First());
            chamfer.Add(distance, distance, edge, refFace);
        }

        chamfer.Build();
        if (chamfer.IsDone()) {
            return chamfer.Shape();
        }
    } catch (...) {
        // Chamfer failed
    }

    return TopoDS_Shape();
}

std::optional<ModelingTool::Indicator> FilletChamferTool::indicator() const {
    if (!active_ || selectedEdges_.empty()) {
        return std::nullopt;
    }

    QVector3D direction(1.0f, 0.0f, 0.0f);
    if (viewport_ && viewport_->camera()) {
        direction = viewport_->camera()->right();
    }
    if (direction.lengthSquared() < 1e-6f) {
        direction = QVector3D(1.0f, 0.0f, 0.0f);
    }

    Indicator ind;
    ind.origin = QVector3D(edgeMidpoint_.X(), edgeMidpoint_.Y(), edgeMidpoint_.Z());
    ind.direction = direction;  // Screen-right aligned
    ind.distance = currentValue_;
    ind.showDistance = dragging_ && currentValue_ >= kMinValue;
    ind.booleanMode = app::BooleanMode::NewBody;  // Not applicable
    ind.isDoubleSided = true; // Use double arrow
    return ind;
}

} // namespace onecad::ui::tools