/**
 * @file ExtrudeTool.cpp
 */
#include "ExtrudeTool.h"

#include "../viewport/Viewport.h"
#include "../../app/commands/AddBodyCommand.h"
#include "../../app/commands/CommandProcessor.h"
#include "../../app/document/Document.h"
#include "../../core/loop/FaceBuilder.h"
#include "../../core/loop/RegionUtils.h"

#include <BRepAdaptor_Surface.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepOffsetAPI_DraftAngle.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopAbs_Orientation.hxx>
#include <gp_Vec.hxx>

#include <QUuid>
#include <QtMath>

namespace onecad::ui::tools {

namespace {
constexpr double kMinExtrudeDistance = 1e-3;
constexpr double kDraftAngleEpsilon = 1e-4;
constexpr double kSideFaceDotThreshold = 0.9;
} // namespace

ExtrudeTool::ExtrudeTool(Viewport* viewport, app::Document* document)
    : viewport_(viewport), document_(document) {
}

void ExtrudeTool::setDocument(app::Document* document) {
    document_ = document;
}

void ExtrudeTool::setCommandProcessor(app::commands::CommandProcessor* processor) {
    commandProcessor_ = processor;
}

void ExtrudeTool::begin(const app::selection::SelectionItem& selection) {
    selection_ = selection;
    dragging_ = false;
    currentDistance_ = 0.0;
    active_ = prepareInput(selection);
    if (!active_) {
        clearPreview();
    }
}

void ExtrudeTool::cancel() {
    clearPreview();
    active_ = false;
    dragging_ = false;
}

bool ExtrudeTool::handleMousePress(const QPoint& screenPos, Qt::MouseButton button) {
    if (!active_ || button != Qt::LeftButton) {
        return false;
    }
    dragStart_ = screenPos;
    currentDistance_ = 0.0;
    dragging_ = true;
    return true;
}

bool ExtrudeTool::handleMouseMove(const QPoint& screenPos) {
    if (!active_ || !dragging_) {
        return false;
    }

    const double pixelScale = viewport_ ? viewport_->pixelScale() : 1.0;
    const double deltaY = static_cast<double>(screenPos.y() - dragStart_.y());
    const double distance = -deltaY * pixelScale;

    if (std::abs(distance - currentDistance_) < 1e-6) {
        return true;
    }

    currentDistance_ = distance;
    if (std::abs(currentDistance_) < kMinExtrudeDistance) {
        clearPreview();
        return true;
    }

    updatePreview(currentDistance_);
    return true;
}

bool ExtrudeTool::handleMouseRelease(const QPoint& screenPos, Qt::MouseButton button) {
    if (!active_ || !dragging_ || button != Qt::LeftButton) {
        return false;
    }

    const double pixelScale = viewport_ ? viewport_->pixelScale() : 1.0;
    const double deltaY = static_cast<double>(screenPos.y() - dragStart_.y());
    const double distance = -deltaY * pixelScale;
    dragging_ = false;

    if (std::abs(distance) < kMinExtrudeDistance) {
        clearPreview();
        return true;
    }

    TopoDS_Shape shape = buildExtrudeShape(distance);
    clearPreview();
    if (document_ && !shape.IsNull()) {
        std::string bodyId;
        if (commandProcessor_) {
            auto command = std::make_unique<app::commands::AddBodyCommand>(document_, shape);
            auto* commandPtr = command.get();
            if (commandProcessor_->execute(std::move(command)) && commandPtr) {
                bodyId = commandPtr->bodyId();
            }
        } else {
            bodyId = document_->addBody(shape);
        }

        if (!bodyId.empty()) {
            app::OperationRecord record;
            record.opId = QUuid::createUuid().toString(QUuid::WithoutBraces).toStdString();
            record.type = app::OperationType::Extrude;
            record.input = app::SketchRegionRef{selection_.id.ownerId, selection_.id.elementId};
            record.params.distance = distance;
            record.params.draftAngleDeg = draftAngleDeg_;
            record.params.booleanMode = app::BooleanMode::NewBody;
            record.resultBodyIds.push_back(bodyId);
            document_->addOperation(record);
        }
    }

    currentDistance_ = 0.0;
    return true;
}

bool ExtrudeTool::prepareInput(const app::selection::SelectionItem& selection) {
    if (!document_ || selection.kind != app::selection::SelectionKind::SketchRegion) {
        return false;
    }

    sketch_ = document_->getSketch(selection.id.ownerId);
    if (!sketch_) {
        return false;
    }

    auto faceOpt = core::loop::resolveRegionFace(*sketch_, selection.id.elementId);
    if (!faceOpt.has_value()) {
        return false;
    }

    core::loop::FaceBuilder builder;
    auto faceResult = builder.buildFace(faceOpt.value(), *sketch_);
    if (!faceResult.success) {
        return false;
    }

    baseFace_ = faceResult.face;
    const auto& plane = sketch_->getPlane();
    direction_ = gp_Dir(plane.normal.x, plane.normal.y, plane.normal.z);
    neutralPlane_ = gp_Pln(gp_Pnt(plane.origin.x, plane.origin.y, plane.origin.z), direction_);
    return true;
}

void ExtrudeTool::updatePreview(double distance) {
    if (!viewport_) {
        return;
    }
    TopoDS_Shape shape = buildExtrudeShape(distance);
    if (shape.IsNull()) {
        clearPreview();
        return;
    }

    render::SceneMeshStore::Mesh mesh = previewTessellator_.buildMesh("preview", shape, previewElementMap_);
    viewport_->setModelPreviewMeshes({std::move(mesh)});
}

void ExtrudeTool::clearPreview() {
    if (viewport_) {
        viewport_->clearModelPreviewMeshes();
    }
}

TopoDS_Shape ExtrudeTool::buildExtrudeShape(double distance) const {
    if (baseFace_.IsNull()) {
        return TopoDS_Shape();
    }

    gp_Vec prismVec(direction_.X() * distance,
                    direction_.Y() * distance,
                    direction_.Z() * distance);
    BRepPrimAPI_MakePrism prism(baseFace_, prismVec, true);
    TopoDS_Shape result = prism.Shape();

    if (std::abs(draftAngleDeg_) <= kDraftAngleEpsilon) {
        return result;
    }

    const double angleRad = qDegreesToRadians(draftAngleDeg_);
    gp_Dir draftDir = direction_;
    if (distance < 0.0) {
        draftDir.Reverse();
    }

    BRepOffsetAPI_DraftAngle draft(result);
    bool anyAdded = false;

    for (TopExp_Explorer exp(result, TopAbs_FACE); exp.More(); exp.Next()) {
        TopoDS_Face face = TopoDS::Face(exp.Current());
        BRepAdaptor_Surface surface(face, true);
        if (surface.GetType() != GeomAbs_Plane) {
            continue;
        }
        gp_Pln plane = surface.Plane();
        gp_Dir normal = plane.Axis().Direction();
        if (face.Orientation() == TopAbs_REVERSED) {
            normal.Reverse();
        }
        const double dot = std::abs(normal.Dot(draftDir));
        if (dot > kSideFaceDotThreshold) {
            continue;
        }

        draft.Add(face, draftDir, angleRad, neutralPlane_, true);
        if (draft.AddDone()) {
            anyAdded = true;
        } else {
            draft.Remove(face);
        }
    }

    if (anyAdded) {
        draft.Build();
        if (draft.IsDone()) {
            result = draft.Shape();
        }
    }

    return result;
}

} // namespace onecad::ui::tools
