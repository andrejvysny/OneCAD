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

    detectBooleanMode(distance);
    updatePreview(distance);
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

    // Recalculate boolean mode one last time to be safe
    detectBooleanMode(distance);

    TopoDS_Shape toolShape = buildExtrudeShape(distance);
    TopoDS_Shape finalShape = toolShape;
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
        } else {
            // Boolean Operation logic
            // Find target shape
            std::string targetId = targetBodyId_; 
            // If targetBodyId_ is empty (sketch input), we might need to find which body we intersected
            // But detectBooleanMode implies we found something.
            // For now, let's assume we intersect the body we started from (if Face selection)
            // or perform a global boolean search?
            // "BooleanOperation::perform" takes a target.
            
            // NOTE: For v1b, if we started from a SKETCH, targetBodyId_ is empty.
            // We need to identify the target.
            // Simplified: Only support boolean ops if we have a valid target (e.g. from face selection or intersection).
            // For now, let's rely on targetBodyId_ (Face selection) for Cut/Join.
            // If Sketch selection, we default to NewBody unless we implement complex target picking.
            
            // Wait, detectBooleanMode sets the mode. If it detected Cut/Add, we must have a target.
            // But I didn't store the detected target in detectBooleanMode.
            // I should refine this.
            
            // For this implementation step, let's limit Boolean Ops to when we have a targetBodyId_ (Face input).
            // Or if we are extruding a sketch, stick to NewBody for now unless we do global search.
            
            if (!targetId.empty()) {
                const TopoDS_Shape* targetShape = document_->getBodyShape(targetId);
                if (targetShape) {
                    finalShape = core::modeling::BooleanOperation::perform(toolShape, *targetShape, booleanMode_);
                    
                    if (commandProcessor_) {
                        auto command = std::make_unique<app::commands::ModifyBodyCommand>(document_, targetId, finalShape);
                        if (commandProcessor_->execute(std::move(command))) {
                            resultBodyId = targetId;
                            success = true;
                        }
                    } else {
                        // Direct modification
                        std::string name = document_->getBodyName(targetId);
                        document_->removeBody(targetId);
                        document_->addBodyWithId(targetId, finalShape, name);
                        resultBodyId = targetId;
                        success = true;
                    }
                }
            } else {
                 // Fallback to NewBody if no target identified
                 booleanMode_ = app::BooleanMode::NewBody;
                 if (commandProcessor_) {
                    auto command = std::make_unique<app::commands::AddBodyCommand>(document_, toolShape);
                    auto* cmdPtr = command.get();
                    if (commandProcessor_->execute(std::move(command))) {
                         resultBodyId = cmdPtr->bodyId();
                         success = true;
                    }
                 } else {
                     resultBodyId = document_->addBody(toolShape);
                     success = !resultBodyId.empty();
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
        const auto* entry = document_->elementMap().find(kernel::elementmap::ElementId{selection.id.elementId});
        if (!entry || entry->kind != kernel::elementmap::ElementKind::Face || entry->shape.IsNull()) {
            return false;
        }
        baseFace_ = TopoDS::Face(entry->shape);
        
        BRepAdaptor_Surface surface(baseFace_, true);
        if (surface.GetType() == GeomAbs_Plane) {
            gp_Pln plane = surface.Plane();
            direction_ = plane.Axis().Direction();
            if (baseFace_.Orientation() == TopAbs_REVERSED) {
                direction_.Reverse();
            }
            neutralPlane_ = plane;
        } else {
            // Only planar faces supported for now
            return false;
        }
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

void ExtrudeTool::detectBooleanMode(double distance) {
    if (distance == 0.0) return;

    // If extruding a face of a body, check direction relative to body
    // If extruding a sketch, we might overlap.
    
    // For now, if we selected a Face, default to Union (Join) if pulling out, Cut if pushing in?
    // Or simpler: Use BooleanOperation::detectMode
    
    TopoDS_Shape tool = buildExtrudeShape(distance);
    if (tool.IsNull()) return;
    
    std::vector<TopoDS_Shape> targets;
    if (!targetBodyId_.empty()) {
        const TopoDS_Shape* body = document_->getBodyShape(targetBodyId_);
        if (body) {
            targets.push_back(*body);
        }
    } else {
        // Collect all bodies?
        // For performance, maybe skip global check for now and stick to NewBody for Sketch extrudes
        // unless they touch something.
        // Let's keep it simple: Face selection -> interact with owner. Sketch -> NewBody.
    }
    
    if (targets.empty()) {
        booleanMode_ = app::BooleanMode::NewBody;
        return;
    }
    
    gp_Dir extrudeDir = direction_;
    if (distance < 0) extrudeDir.Reverse();
    
    booleanMode_ = core::modeling::BooleanOperation::detectMode(tool, targets, extrudeDir);
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
    
    // If Cut/Add, visualize the result? 
    // Usually CAD shows the tool shape (translucent) + boolean indication.
    // Or the full boolean result? Full boolean result is heavy for preview.
    // Let's show the tool shape, but color it?
    // Current renderer doesn't support custom colors per preview mesh easily yet (it takes Mesh struct).
    // We can stick to showing the tool shape.
    
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
    if (dragging_ && currentDistance_ < 0.0) {
        dir = -dir;
    }
    if (dir.lengthSquared() < 1e-6f) {
        return std::nullopt;
    }

    Indicator indicator;
    indicator.origin = QVector3D(originPoint.X(), originPoint.Y(), originPoint.Z());
    indicator.direction = dir;
    indicator.distance = currentDistance_;
    indicator.showDistance = dragging_ && std::abs(currentDistance_) >= kMinExtrudeDistance;
    indicator.booleanMode = booleanMode_;
    return indicator;
}

} // namespace onecad::ui::tools