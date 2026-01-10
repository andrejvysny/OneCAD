/**
 * @file ShellTool.cpp
 */
#include "ShellTool.h"

#include "../viewport/Viewport.h"
#include "../../app/commands/AddOperationCommand.h"
#include "../../app/commands/CommandProcessor.h"
#include "../../app/document/Document.h"
#include "../../app/document/OperationRecord.h"
#include "../../render/Camera3D.h"

#include <QUuid>

#include <BRepOffsetAPI_MakeThickSolid.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>

#include <QVector3D>

#include <algorithm>
#include <cmath>

namespace onecad::ui::tools {

namespace {
constexpr double kMinThickness = 1e-3;
constexpr double kMaxThickness = 100.0;
} // namespace

ShellTool::ShellTool(Viewport* viewport, app::Document* document)
    : viewport_(viewport), document_(document) {
}

void ShellTool::setDocument(app::Document* document) {
    document_ = document;
}

void ShellTool::setCommandProcessor(app::commands::CommandProcessor* processor) {
    commandProcessor_ = processor;
}

void ShellTool::begin(const app::selection::SelectionItem& selection) {
    state_ = State::WaitingForBody;
    dragging_ = false;
    currentThickness_ = 0.0;
    openFaces_.clear();

    active_ = prepareBody(selection);
    if (active_) {
        state_ = State::WaitingForFaces;
    } else {
        clearPreview();
    }
}

void ShellTool::cancel() {
    clearPreview();
    active_ = false;
    dragging_ = false;
    state_ = State::WaitingForBody;
    openFaces_.clear();
}

bool ShellTool::handleMousePress(const QPoint& screenPos, Qt::MouseButton button) {
    if (!active_ || button != Qt::LeftButton) {
        return false;
    }

    if (state_ == State::Dragging || state_ == State::WaitingForFaces) {
        // If in WaitingForFaces and clicking with no shift -> start dragging
        // This allows dragging immediately after body selection if no faces needed
        // BUT only if clicking the indicator, as requested
        if (viewport_ && viewport_->isMouseOverIndicator(screenPos)) {
            dragStart_ = screenPos;
            currentThickness_ = 0.0;
            dragging_ = true;
            state_ = State::Dragging;
            return true;
        }
    }

    return false;
}

bool ShellTool::handleMouseMove(const QPoint& screenPos) {
    if (!active_ || !dragging_) {
        return false;
    }

    const double pixelScale = viewport_ ? viewport_->pixelScale() : 1.0;
    const double deltaY = static_cast<double>(screenPos.y() - dragStart_.y());
    const double thickness = -deltaY * pixelScale;

    if (std::abs(thickness - currentThickness_) < 1e-6) {
        return true;
    }

    // Only positive thickness for shell (inward offset)
    currentThickness_ = std::clamp(std::abs(thickness), 0.0, kMaxThickness);

    if (currentThickness_ < kMinThickness) {
        clearPreview();
        return true;
    }

    updatePreview(currentThickness_);
    return true;
}

bool ShellTool::handleMouseRelease(const QPoint& screenPos, Qt::MouseButton button) {
    if (!active_ || !dragging_ || button != Qt::LeftButton) {
        return false;
    }

    const double pixelScale = viewport_ ? viewport_->pixelScale() : 1.0;
    const double deltaY = static_cast<double>(screenPos.y() - dragStart_.y());
    const double thickness = std::abs(-deltaY * pixelScale);
    dragging_ = false;

    if (thickness < kMinThickness) {
        clearPreview();
        state_ = State::WaitingForFaces;
        return true;
    }

    const double finalThickness = std::clamp(thickness, kMinThickness, kMaxThickness);
    
    commitOperation(finalThickness);
    return true;
}

bool ShellTool::prepareBody(const app::selection::SelectionItem& selection) {
    if (!document_) {
        return false;
    }

    targetBodyId_.clear();
    targetShape_.Nullify();
    openFaces_.clear();

    // Accept body selection
    if (selection.kind != app::selection::SelectionKind::Body) {
        return false;
    }

    targetBodyId_ = selection.id.ownerId.empty() ? selection.id.elementId : selection.id.ownerId;
    if (targetBodyId_.empty()) {
        targetBodyId_ = selection.id.elementId;
    }

    const TopoDS_Shape* bodyShape = document_->getBodyShape(targetBodyId_);
    if (!bodyShape || bodyShape->IsNull()) {
        return false;
    }
    targetShape_ = *bodyShape;

    // Calculate body center for indicator
    GProp_GProps props;
    BRepGProp::VolumeProperties(targetShape_, props);
    if (props.Mass() > 0.0) {
        indicatorOrigin_ = props.CentreOfMass();
    } else {
        indicatorOrigin_ = gp_Pnt(0, 0, 0);
    }

    return true;
}

bool ShellTool::addOpenFace(const app::selection::SelectionItem& selection) {
    if (!active_ || state_ != State::WaitingForFaces || !document_) {
        return false;
    }

    if (selection.kind != app::selection::SelectionKind::Face) {
        return false;
    }

    // Verify face belongs to our target body
    if (selection.id.ownerId != targetBodyId_) {
        return false;
    }

    // Find the face from elementMap
    const auto* entry = document_->elementMap().find(
        kernel::elementmap::ElementId{selection.id.elementId});
    if (!entry || entry->kind != kernel::elementmap::ElementKind::Face || entry->shape.IsNull()) {
        return false;
    }

    TopoDS_Face face = TopoDS::Face(entry->shape);

    // Check if already in list (toggle)
    for (auto it = openFaces_.begin(); it != openFaces_.end(); ++it) {
        if (it->IsSame(face)) {
            openFaces_.erase(it);
            return true;
        }
    }

    openFaces_.push_back(face);
    return true;
}

void ShellTool::clearOpenFaces() {
    openFaces_.clear();
}

void ShellTool::confirmFaceSelection() {
    if (state_ == State::WaitingForFaces) {
        // If Enter is pressed, commit with default thickness if none set,
        // or current thickness if we were somehow dragging without mouse release.
        double thickness = currentThickness_;
        if (thickness < kMinThickness) {
            thickness = 1.0; // Default 1mm thickness on Enter
        }
        commitOperation(thickness);
    }
}

void ShellTool::commitOperation(double thickness) {
    // Validate thickness
    if (thickness < kMinThickness) {
        clearPreview();
        state_ = State::WaitingForFaces;
        return;
    }

    TopoDS_Shape resultShape = buildShellShape(thickness);
    if (resultShape.IsNull()) {
        clearPreview();
        state_ = State::WaitingForFaces;
        return;
    }

    if (document_ && !targetBodyId_.empty()) {
        app::OperationRecord record;
        record.opId = QUuid::createUuid().toString(QUuid::WithoutBraces).toStdString();
        record.type = app::OperationType::Shell;
        record.input = app::BodyRef{targetBodyId_};

        app::ShellParams params;
        params.thickness = thickness;

        // Collect open face IDs from ElementMap
        for (const auto& face : openFaces_) {
            auto ids = document_->elementMap().findIdsByShape(face);
            if (!ids.empty()) {
                params.openFaceIds.push_back(ids.front().value);
            }
        }

        record.params = params;
        record.resultBodyIds.push_back(targetBodyId_);

        auto command = std::make_unique<app::commands::AddOperationCommand>(document_, record);
        if (commandProcessor_) {
            commandProcessor_->execute(std::move(command));
        } else {
            command->execute();
        }
    }

    clearPreview();
    currentThickness_ = 0.0;
    active_ = false;
    state_ = State::WaitingForBody;
    openFaces_.clear();
}

void ShellTool::updatePreview(double thickness) {
    if (!viewport_) {
        return;
    }

    TopoDS_Shape previewShape = buildShellShape(thickness);
    if (previewShape.IsNull()) {
        clearPreview();
        return;
    }

    previewElementMap_.clear();
    render::SceneMeshStore::Mesh mesh = previewTessellator_.buildMesh(
        "shell_preview", previewShape, previewElementMap_);
    viewport_->setModelPreviewMeshes({std::move(mesh)});
}

void ShellTool::clearPreview() {
    if (viewport_) {
        viewport_->clearModelPreviewMeshes();
    }
}

TopoDS_Shape ShellTool::buildShellShape(double thickness) const {
    if (targetShape_.IsNull() || thickness < kMinThickness) {
        return TopoDS_Shape();
    }

    try {
        TopTools_ListOfShape facesToRemove;
        for (const auto& face : openFaces_) {
            facesToRemove.Append(face);
        }

        BRepOffsetAPI_MakeThickSolid shell;
        shell.MakeThickSolidByJoin(
            targetShape_,
            facesToRemove,
            -thickness,         // Negative = shell inward
            1e-3,               // Tolerance
            BRepOffset_Skin,
            Standard_False,     // Intersection
            Standard_False,     // SelfInter
            GeomAbs_Arc         // Join type
        );

        shell.Build();
        if (shell.IsDone()) {
            return shell.Shape();
        }
    } catch (...) {
        // Shell operation failed
    }

    return TopoDS_Shape();
}

std::optional<ModelingTool::Indicator> ShellTool::indicator() const {
    if (!active_ || targetShape_.IsNull()) {
        return std::nullopt;
    }

    QVector3D direction(0.0f, 1.0f, 0.0f);
    if (viewport_ && viewport_->camera()) {
        direction = viewport_->camera()->up();
    }
    if (direction.lengthSquared() < 1e-6f) {
        direction = QVector3D(0.0f, 1.0f, 0.0f);
    }

    Indicator ind;
    ind.origin = QVector3D(indicatorOrigin_.X(), indicatorOrigin_.Y(), indicatorOrigin_.Z());
    ind.direction = direction;  // Screen-up aligned
    ind.distance = currentThickness_;
    ind.showDistance = dragging_ && currentThickness_ >= kMinThickness;
    ind.booleanMode = app::BooleanMode::NewBody;
    ind.isDoubleSided = true; // Double arrow
    return ind;
}

} // namespace onecad::ui::tools
