/**
 * @file ExtrudeTool.cpp
 */
#include "ExtrudeTool.h"

#include "../viewport/Viewport.h"
#include "../../app/commands/AddBodyCommand.h"
#include "../../app/commands/ModifyBodyCommand.h"
#include "../../app/commands/CommandProcessor.h"
#include "../../app/document/Document.h"
#include "../../core/loop/FaceBuilder.h"
#include "../../core/loop/RegionUtils.h"
#include "../../core/modeling/BooleanOperation.h"
#include "../../render/Camera3D.h"

#include <BRepGProp.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepOffsetAPI_DraftAngle.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <GProp_GProps.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopAbs_Orientation.hxx>
#include <gp_Vec.hxx>

#include <QUuid>
#include <QVector3D>
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
    booleanMode_ = app::BooleanMode::NewBody;
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

    // Only start dragging if the arrow indicator was clicked
    if (!viewport_ || !viewport_->isMouseOverIndicator(screenPos)) {
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

    // Vector Projection Logic for Dragging:
    // 1. Get Camera matrices
    if (!viewport_ || !viewport_->camera()) return false;
    auto* camera = viewport_->camera();
    float aspectRatio = static_cast<float>(viewport_->width()) / static_cast<float>(viewport_->height());
    QMatrix4x4 viewProj = camera->projectionMatrix(aspectRatio) * camera->viewMatrix();

    // 2. Project extrusion start (base center) and end (along normal) to screen
    QVector3D startWorld(baseCenter_.X(), baseCenter_.Y(), baseCenter_.Z());
    QVector3D endWorld = startWorld + QVector3D(direction_.X(), direction_.Y(), direction_.Z());

    auto project = [&](const QVector3D& worldPos) -> std::optional<QVector2D> {
        QVector4D clip = viewProj * QVector4D(worldPos, 1.0f);
        if (clip.w() <= 1e-6f) return std::nullopt; // Behind camera
        QVector3D ndc = clip.toVector3D() / clip.w();
        float x = (ndc.x() * 0.5f + 0.5f) * viewport_->width();
        float y = (1.0f - (ndc.y() * 0.5f + 0.5f)) * viewport_->height();
        return QVector2D(x, y);
    };

    auto p0Opt = project(startWorld);
    auto p1Opt = project(endWorld);

    double distance = 0.0;
    
    // If projection failed or axis is perpendicular to view (degenerate 2D vector), fallback to pixel scaling directly
    bool projectionValid = false;
    if (p0Opt && p1Opt) {
        QVector2D axis = *p1Opt - *p0Opt;
        if (axis.lengthSquared() > 1e-4) {
            axis.normalize();
            QVector2D mouseDelta(screenPos.x() - dragStart_.x(), screenPos.y() - dragStart_.y());
            
            // Project mouse delta onto the visual axis
            // If dragging in direction of arrow, dot product is positive -> positive extrusion
            double pixelDelta = QVector2D::dotProduct(mouseDelta, axis);
            
            const double pixelScale = viewport_->pixelScale();
            distance = pixelDelta * pixelScale;
            projectionValid = true;
        }
    }

    if (!projectionValid) {
        // Fallback (e.g. looking straight down at the arrow)
        // Just use Y delta inverted as a best guess standard
        const double pixelScale = viewport_->pixelScale();
        const double deltaY = static_cast<double>(screenPos.y() - dragStart_.y());
        distance = -deltaY * pixelScale;
    }

    if (std::abs(distance - currentDistance_) < 1e-6) {
        return true;
    }

    currentDistance_ = distance;
    if (std::abs(currentDistance_) < kMinExtrudeDistance) {
        clearPreview();
        return true;
    }

    detectBooleanMode(distance);
    updatePreview(distance);
    return true;
}

bool ExtrudeTool::handleMouseRelease(const QPoint& screenPos, Qt::MouseButton button) {
    if (!active_ || !dragging_ || button != Qt::LeftButton) {
        return false;
    }

    // Re-calculate distance one last time to match mouseMove logic precisely
    double distance = currentDistance_;
    // If needed we could re-run the projection logic here, but using the last cached valid distance 
    // from mouseMove is usually safer and consistent (unless we want to support 'click-release' without move?).
    // Just to be safe, let's re-run the basic projection if we moved.
    
    if (screenPos != dragStart_) {
        // ... Duplicate projection logic or assume currentDistance_ is fresh from last specific move event move?
        // Actually, handleMouseMove updates currentDistance_. If we released at a different spot,
        // handleMouseMove might not have been called for that *exact* pixel if dropped quickly.
        // Let's copy the robust logic to be sure.
        
        if (viewport_ && viewport_->camera()) {
             auto* camera = viewport_->camera();
            float aspectRatio = static_cast<float>(viewport_->width()) / static_cast<float>(viewport_->height());
            QMatrix4x4 viewProj = camera->projectionMatrix(aspectRatio) * camera->viewMatrix();

            QVector3D startWorld(baseCenter_.X(), baseCenter_.Y(), baseCenter_.Z());
            QVector3D endWorld = startWorld + QVector3D(direction_.X(), direction_.Y(), direction_.Z());
            
            auto project = [&](const QVector3D& worldPos) -> std::optional<QVector2D> {
                QVector4D clip = viewProj * QVector4D(worldPos, 1.0f);
                if (clip.w() <= 1e-6f) return std::nullopt;
                QVector3D ndc = clip.toVector3D() / clip.w();
                float x = (ndc.x() * 0.5f + 0.5f) * viewport_->width();
                float y = (1.0f - (ndc.y() * 0.5f + 0.5f)) * viewport_->height();
                return QVector2D(x, y);
            };

            auto p0Opt = project(startWorld);
            auto p1Opt = project(endWorld);
            bool projectionValid = false;
             if (p0Opt && p1Opt) {
                QVector2D axis = *p1Opt - *p0Opt;
                if (axis.lengthSquared() > 1e-4) {
                    axis.normalize();
                    QVector2D mouseDelta(screenPos.x() - dragStart_.x(), screenPos.y() - dragStart_.y());
                    double pixelDelta = QVector2D::dotProduct(mouseDelta, axis);
                    distance = pixelDelta * viewport_->pixelScale();
                    projectionValid = true;
                }
            }
            if (!projectionValid) {
                const double deltaY = static_cast<double>(screenPos.y() - dragStart_.y());
                distance = -deltaY * viewport_->pixelScale(); 
            }
        }
    }
    
    dragging_ = false;

    if (std::abs(distance) < kMinExtrudeDistance) {
        clearPreview();
        return true;
    }

    // Recalculate boolean mode one last time to be safe
    detectBooleanMode(distance);

    TopoDS_Shape toolShape = buildExtrudeShape(distance);
    std::string resultBodyId;
    bool success = false;

    if (document_ && !toolShape.IsNull()) {
        if (booleanMode_ == app::BooleanMode::NewBody) {
            // Standard AddBody logic
            if (commandProcessor_) {
                auto command = std::make_unique<app::commands::AddBodyCommand>(document_, toolShape);
                auto* commandPtr = command.get();
                if (commandProcessor_->execute(std::move(command)) && commandPtr) {
                    resultBodyId = commandPtr->bodyId();
                    success = true;
                }
            } else {
                resultBodyId = document_->addBody(toolShape);
                success = !resultBodyId.empty();
            }
        } else if (!targetBodyId_.empty() && !targetShape_.IsNull()) {
            // Boolean Operation logic (Push/Pull)
            TopoDS_Shape resultShape = core::modeling::BooleanOperation::perform(
                toolShape, targetShape_, booleanMode_);

            if (!resultShape.IsNull()) {
                if (commandProcessor_) {
                    auto command = std::make_unique<app::commands::ModifyBodyCommand>(
                        document_, targetBodyId_, resultShape);
                    if (commandProcessor_->execute(std::move(command))) {
                        resultBodyId = targetBodyId_;
                        success = true;
                    }
                } else {
                    // Direct modification fallback (should rarely be used if processor exists)
                    std::string name = document_->getBodyName(targetBodyId_);
                    document_->removeBody(targetBodyId_);
                    document_->addBodyWithId(targetBodyId_, resultShape, name);
                    resultBodyId = targetBodyId_;
                    success = true;
                }
            }
        }

        if (success) {
            app::OperationRecord record;
            record.opId = QUuid::createUuid().toString(QUuid::WithoutBraces).toStdString();
            record.type = app::OperationType::Extrude;
            
            if (sketch_) {
                record.input = app::SketchRegionRef{selection_.id.ownerId, selection_.id.elementId};
            } else {
                record.input = app::FaceRef{selection_.id.ownerId, selection_.id.elementId};
            }
            
            app::ExtrudeParams params;
            params.distance = distance;
            params.draftAngleDeg = draftAngleDeg_;
            params.booleanMode = booleanMode_;
            record.params = params;
            
            record.resultBodyIds.push_back(resultBodyId);
            document_->addOperation(record);
        }
    }

    clearPreview();
    currentDistance_ = 0.0;
    return true;
}

bool ExtrudeTool::prepareInput(const app::selection::SelectionItem& selection) {
    if (!document_) {
        return false;
    }
    
    // Reset state
    baseFace_.Nullify();
    sketch_ = nullptr;
    targetBodyId_.clear();
    targetShape_.Nullify();

    if (selection.kind == app::selection::SelectionKind::SketchRegion) {
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
        
    } else if (selection.kind == app::selection::SelectionKind::Face) {
        targetBodyId_ = selection.id.ownerId;
        const TopoDS_Shape* bodyShape = document_->getBodyShape(targetBodyId_);
        if (!bodyShape || bodyShape->IsNull()) {
            return false;
        }
        targetShape_ = *bodyShape;

        const auto* entry = document_->elementMap().find(kernel::elementmap::ElementId{selection.id.elementId});
        if (!entry || entry->kind != kernel::elementmap::ElementKind::Face || entry->shape.IsNull()) {
            return false;
        }
        baseFace_ = TopoDS::Face(entry->shape);
        
        if (!isPlanarFace(baseFace_)) {
            // Only planar faces supported for now
            return false;
        }

        BRepAdaptor_Surface surface(baseFace_, true);
        gp_Pln plane = surface.Plane();
        direction_ = plane.Axis().Direction();
        if (baseFace_.Orientation() == TopAbs_REVERSED) {
            direction_.Reverse();
        }
        neutralPlane_ = plane;
    } else {
        return false;
    }

    baseCenter_ = gp_Pnt(0,0,0);
    GProp_GProps props;
    BRepGProp::SurfaceProperties(baseFace_, props);
    if (props.Mass() > 0.0) {
        baseCenter_ = props.CentreOfMass();
    }
    return true;
}

bool ExtrudeTool::isPlanarFace(const TopoDS_Face& face) const {
    if (face.IsNull()) {
        return false;
    }
    BRepAdaptor_Surface surface(face, true);
    return surface.GetType() == GeomAbs_Plane;
}

void ExtrudeTool::detectBooleanMode(double distance) {
    if (distance == 0.0) return;

    if (!targetBodyId_.empty() && !targetShape_.IsNull()) {
        // Face push/pull logic
        // Pull out (positive along normal) = Add (Join)
        // Push in (negative along normal) = Cut
        booleanMode_ = (distance >= 0) ? app::BooleanMode::Add : app::BooleanMode::Cut;
    } else {
        // Sketch logic (NewBody by default)
        // Future: Check for intersection with existing bodies
        booleanMode_ = app::BooleanMode::NewBody;
    }
}


void ExtrudeTool::updatePreview(double distance) {
    if (!viewport_) {
        return;
    }
    TopoDS_Shape tool = buildExtrudeShape(distance);
    if (tool.IsNull()) {
        clearPreview();
        return;
    }
    
    render::SceneMeshStore::Mesh mesh = previewTessellator_.buildMesh("preview", tool, previewElementMap_);
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

std::optional<ModelingTool::Indicator> ExtrudeTool::indicator() const {
    if (!active_ || baseFace_.IsNull()) {
        return std::nullopt;
    }

    const double offset = dragging_ ? currentDistance_ : 0.0;
    const gp_Pnt originPoint = baseCenter_.Translated(gp_Vec(direction_.X() * offset,
                                                             direction_.Y() * offset,
                                                             direction_.Z() * offset));

    QVector3D dir(direction_.X(), direction_.Y(), direction_.Z());
    
    // Always use forward direction for the indicator logic;
    // visual rendering handles flipping or double-sided.
    // The previous logic was flipping dir if dragging backwards, which is correct for 
    // the single-sided arrow to point "backwards". 
    // For double-sided arrow, we want the "axis".
    // However, if we are dragging, we might want to emphasize the drag direction.
    // But since the user asked for double arrow, let's keep it consistent.
    
    if (dragging_ && currentDistance_ < 0.0) {
        dir = -dir;
    }
    
    if (dir.lengthSquared() < 1e-6f) {
        return std::nullopt;
    }

    Indicator indicator;
    indicator.origin = QVector3D(originPoint.X(), originPoint.Y(), originPoint.Z());
    indicator.direction = dir;
    indicator.distance = std::abs(currentDistance_);
    indicator.showDistance = dragging_ && std::abs(currentDistance_) >= kMinExtrudeDistance;
    indicator.booleanMode = booleanMode_;
    indicator.isDoubleSided = true; // Always double sided for extrude
    return indicator;
}

} // namespace onecad::ui::tools
