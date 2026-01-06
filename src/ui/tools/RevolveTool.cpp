/**
 * @file RevolveTool.cpp
 */
#include "RevolveTool.h"

#include "../viewport/Viewport.h"
#include "../../app/commands/AddBodyCommand.h"
#include "../../app/commands/ModifyBodyCommand.h"
#include "../../app/commands/CommandProcessor.h"
#include "../../app/document/Document.h"
#include "../../core/loop/FaceBuilder.h"
#include "../../core/loop/RegionUtils.h"
#include "../../core/modeling/BooleanOperation.h"
#include "../../core/sketch/Sketch.h"
#include "../../core/sketch/SketchLine.h"
#include "../../core/sketch/SketchPoint.h"

#include <BRepAdaptor_Surface.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <GeomAbs_CurveType.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <gp_Lin.hxx>
#include <gp_Vec.hxx>
#include <QUuid>
#include <QtMath>

namespace onecad::ui::tools {

namespace {
constexpr double kMinRevolveAngle = 1e-3;
constexpr double kMaxRevolveAngle = 360.0;
constexpr double kDefaultRevolveAngle = 360.0;
constexpr double kAnglePerPixel = 1.0;
} // namespace

RevolveTool::RevolveTool(Viewport* viewport, app::Document* document)
    : viewport_(viewport), document_(document) {
}

void RevolveTool::setDocument(app::Document* document) {
    document_ = document;
}

void RevolveTool::setCommandProcessor(app::commands::CommandProcessor* processor) {
    commandProcessor_ = processor;
}

void RevolveTool::begin(const app::selection::SelectionItem& selection) {
    // If selection is a region or face, we accept it as profile
    // and move to WaitingForAxis state.
    clearPreview();
    active_ = false;
    dragging_ = false;
    state_ = State::WaitingForProfile;
    axisValid_ = false;
    currentAngle_ = 0.0;
    dragStartAngle_ = 0.0;
    booleanMode_ = app::BooleanMode::NewBody;
    axisSelection_ = {};
    profileSelection_ = {};

    if (prepareProfile(selection)) {
        active_ = true;
        state_ = State::WaitingForAxis;
        profileSelection_ = selection;
        // Prompt user to select axis (UI feedback would go here)
    } else {
        cancel();
    }
}

void RevolveTool::cancel() {
    clearPreview();
    active_ = false;
    state_ = State::WaitingForProfile;
    dragging_ = false;
    baseFace_.Nullify();
    sketch_ = nullptr;
    targetBodyId_.clear();
    axisSelection_ = {};
    profileSelection_ = {};
    axisValid_ = false;
    currentAngle_ = 0.0;
    dragStartAngle_ = 0.0;
    booleanMode_ = app::BooleanMode::NewBody;
}

bool RevolveTool::handleMousePress(const QPoint& screenPos, Qt::MouseButton button) {
    if (!active_ || button != Qt::LeftButton) return false;
    
    // If waiting for axis, click should be handled by SelectionManager calling onSelectionChanged
    // But ModelingToolManager handles clicks first. 
    // If we are in WaitingForAxis, we shouldn't consume the click if it's meant for selection?
    // Actually, ModelingToolManager architecture: if tool active, it gets events.
    // If handleMousePress returns false, SelectionManager gets it.
    
    if (state_ == State::WaitingForAxis) {
        // Return false to let SelectionManager pick the axis.
        return false;
    }

    if (state_ == State::Dragging) {
        dragStart_ = screenPos;
        dragStartAngle_ = currentAngle_;
        dragging_ = true;
        return true;
    }

    // Should not reach here if logic is correct (transition from WaitingForAxis happens via onSelectionChanged)
    // Unless we allow re-picking axis?
    return false;
}

bool RevolveTool::handleMouseMove(const QPoint& screenPos) {
    if (!active_ || !dragging_) return false;

    const double deltaY = static_cast<double>(screenPos.y() - dragStart_.y());
    double angle = dragStartAngle_ + (-deltaY * kAnglePerPixel);

    if (angle > kMaxRevolveAngle) angle = kMaxRevolveAngle;
    if (angle < -kMaxRevolveAngle) angle = -kMaxRevolveAngle;

    if (std::abs(angle - currentAngle_) < 1e-2) {
        return true;
    }

    currentAngle_ = angle;
    if (std::abs(currentAngle_) < kMinRevolveAngle) {
        clearPreview();
        return true;
    }

    detectBooleanMode(currentAngle_);
    updatePreview(currentAngle_);
    return true;
}

bool RevolveTool::handleMouseRelease(const QPoint& screenPos, Qt::MouseButton button) {
    if (!active_ || !dragging_ || button != Qt::LeftButton) return false;

    const double deltaY = static_cast<double>(screenPos.y() - dragStart_.y());
    double angle = dragStartAngle_ + (-deltaY * kAnglePerPixel);
    if (angle > kMaxRevolveAngle) angle = kMaxRevolveAngle;
    if (angle < -kMaxRevolveAngle) angle = -kMaxRevolveAngle;

    dragging_ = false;
    currentAngle_ = angle;

    if (std::abs(currentAngle_) < kMinRevolveAngle) {
        clearPreview();
        return true;
    }

    detectBooleanMode(currentAngle_);

    TopoDS_Shape toolShape = buildRevolveShape(currentAngle_);
    if (toolShape.IsNull()) {
        clearPreview();
        return true;
    }

    std::string resultBodyId;
    bool success = false;
    
    if (document_) {
        // Simplified Logic similar to ExtrudeTool
        if (booleanMode_ == app::BooleanMode::NewBody || targetBodyId_.empty()) {
            if (commandProcessor_) {
                auto command = std::make_unique<app::commands::AddBodyCommand>(document_, toolShape);
                auto* cmdPtr = command.get();
                if (commandProcessor_->execute(std::move(command)) && cmdPtr) {
                    resultBodyId = cmdPtr->bodyId();
                    success = true;
                }
            } else {
                resultBodyId = document_->addBody(toolShape);
                success = !resultBodyId.empty();
            }
        } else {
             // Boolean logic
            const TopoDS_Shape* targetShape = document_->getBodyShape(targetBodyId_);
            if (targetShape) {
                TopoDS_Shape finalShape = core::modeling::BooleanOperation::perform(toolShape, *targetShape, booleanMode_);
                if (commandProcessor_) {
                    auto command = std::make_unique<app::commands::ModifyBodyCommand>(document_, targetBodyId_, finalShape);
                    if (commandProcessor_->execute(std::move(command))) {
                        resultBodyId = targetBodyId_;
                        success = true;
                    }
                } else {
                    std::string name = document_->getBodyName(targetBodyId_);
                    document_->removeBody(targetBodyId_);
                    document_->addBodyWithId(targetBodyId_, finalShape, name);
                    resultBodyId = targetBodyId_;
                    success = true;
                }
            }
        }

        if (success) {
            app::OperationRecord record;
            record.opId = QUuid::createUuid().toString(QUuid::WithoutBraces).toStdString();
            record.type = app::OperationType::Revolve;

            if (sketch_) {
                record.input = app::SketchRegionRef{profileSelection_.id.ownerId, profileSelection_.id.elementId};
            } else {
                record.input = app::FaceRef{profileSelection_.id.ownerId, profileSelection_.id.elementId};
            }

            app::RevolveParams params;
            params.angleDeg = currentAngle_;
            if (axisSelection_.kind == app::selection::SelectionKind::SketchEdge) {
                params.axis = app::SketchLineRef{axisSelection_.id.ownerId, axisSelection_.id.elementId};
            } else if (axisSelection_.kind == app::selection::SelectionKind::Edge) {
                params.axis = app::EdgeRef{axisSelection_.id.ownerId, axisSelection_.id.elementId};
            }
            params.booleanMode = booleanMode_;
            record.params = params;

            record.resultBodyIds.push_back(resultBodyId);
            document_->addOperation(record);
        }
    }

    if (success) {
        cancel(); // Reset after commit
    }
    return true;
}

void RevolveTool::onSelectionChanged(const std::vector<app::selection::SelectionItem>& selection) {
    if (state_ != State::WaitingForAxis || selection.empty()) return;
    
    // Check if selection is a valid axis
    const auto& item = selection.front();
    if (setAxis(item)) {
        state_ = State::Dragging;
        axisSelection_ = item;
        dragging_ = false;
        currentAngle_ = kDefaultRevolveAngle;
        detectBooleanMode(currentAngle_);
        updatePreview(currentAngle_);
    }
}

bool RevolveTool::prepareProfile(const app::selection::SelectionItem& selection) {
    if (!document_) return false;
    
    sketch_ = nullptr;
    targetBodyId_.clear();
    baseFace_.Nullify();

    if (selection.kind == app::selection::SelectionKind::SketchRegion) {
        sketch_ = document_->getSketch(selection.id.ownerId);
        if (!sketch_) return false;
        
        auto faceOpt = core::loop::resolveRegionFace(*sketch_, selection.id.elementId);
        if (!faceOpt) return false;
        
        core::loop::FaceBuilder builder;
        auto res = builder.buildFace(*faceOpt, *sketch_);
        if (!res.success) return false;
        
        baseFace_ = res.face;
        
    } else if (selection.kind == app::selection::SelectionKind::Face) {
        targetBodyId_ = selection.id.ownerId;
        // Retrieve face from ElementMap
        const auto* entry = document_->elementMap().find(kernel::elementmap::ElementId{selection.id.elementId});
        if (!entry || entry->kind != kernel::elementmap::ElementKind::Face || entry->shape.IsNull()) {
            return false;
        }
        baseFace_ = TopoDS::Face(entry->shape);
        BRepAdaptor_Surface surface(baseFace_, true);
        if (surface.GetType() != GeomAbs_Plane) {
            return false;
        }
    } else {
        return false;
    }
    
    // Compute center for indicator
    baseCenter_ = gp_Pnt(0.0, 0.0, 0.0);
    GProp_GProps props;
    BRepGProp::SurfaceProperties(baseFace_, props);
    if (props.Mass() > 0.0) {
        baseCenter_ = props.CentreOfMass();
    }
    
    return true;
}

bool RevolveTool::setAxis(const app::selection::SelectionItem& selection) {
    if (!document_) return false;

    axisValid_ = false;

    if (selection.kind == app::selection::SelectionKind::SketchEdge) {
        auto* sketch = document_->getSketch(selection.id.ownerId);
        if (!sketch) return false;
        
        // Retrieve line using getEntityAs
        auto* line = sketch->getEntityAs<core::sketch::SketchLine>(selection.id.elementId);
        if (!line) return false;

        auto* startP = sketch->getEntityAs<core::sketch::SketchPoint>(line->startPointId());
        auto* endP = sketch->getEntityAs<core::sketch::SketchPoint>(line->endPointId());
        
        if (!startP || !endP) return false;

        // Convert to World coordinates
        core::sketch::Vec2d p1_2d{startP->position().X(), startP->position().Y()};
        auto p1_vec = sketch->toWorld(p1_2d);
        
        core::sketch::Vec2d p2_2d{endP->position().X(), endP->position().Y()};
        auto p2_vec = sketch->toWorld(p2_2d);
        
        gp_Pnt p1(p1_vec.x, p1_vec.y, p1_vec.z);
        gp_Pnt p2(p2_vec.x, p2_vec.y, p2_vec.z);

        if (p1.Distance(p2) < 1e-6) return false;

        axis_ = gp_Ax1(p1, gp_Dir(gp_Vec(p1, p2)));
        axisValid_ = true;
        return true; 
    }

    if (selection.kind == app::selection::SelectionKind::Edge) {
        const auto* entry = document_->elementMap().find(kernel::elementmap::ElementId{selection.id.elementId});
        if (!entry || entry->kind != kernel::elementmap::ElementKind::Edge || entry->shape.IsNull()) {
            return false;
        }

        TopoDS_Edge edge = TopoDS::Edge(entry->shape);
        BRepAdaptor_Curve curve(edge);
        if (curve.GetType() != GeomAbs_Line) {
            return false;
        }

        gp_Lin line = curve.Line();
        axis_ = gp_Ax1(line.Location(), line.Direction());
        axisValid_ = true;
        return true;
    }
    return false;
}

void RevolveTool::updatePreview(double angle) {
    if (!viewport_) return;
    TopoDS_Shape shape = buildRevolveShape(angle);
    if (shape.IsNull()) {
        clearPreview();
        return;
    }
    render::SceneMeshStore::Mesh mesh = previewTessellator_.buildMesh("preview", shape, previewElementMap_);
    viewport_->setModelPreviewMeshes({std::move(mesh)});
}

void RevolveTool::clearPreview() {
    if (viewport_) viewport_->clearModelPreviewMeshes();
}

TopoDS_Shape RevolveTool::buildRevolveShape(double angle) const {
    if (baseFace_.IsNull() || !axisValid_) return TopoDS_Shape();
    if (std::abs(angle) < kMinRevolveAngle) return TopoDS_Shape();
    
    try {
        double rad = qDegreesToRadians(angle);
        BRepPrimAPI_MakeRevol revol(baseFace_, axis_, rad);
        return revol.Shape();
    } catch (...) {
        return TopoDS_Shape();
    }
}

void RevolveTool::detectBooleanMode(double angle) {
    if (!document_) {
        booleanMode_ = app::BooleanMode::NewBody;
        return;
    }
    if (!axisValid_ || std::abs(angle) < kMinRevolveAngle) {
        booleanMode_ = app::BooleanMode::NewBody;
        return;
    }

    TopoDS_Shape tool = buildRevolveShape(angle);
    if (tool.IsNull()) {
        booleanMode_ = app::BooleanMode::NewBody;
        return;
    }

    std::vector<TopoDS_Shape> targets;
    if (!targetBodyId_.empty()) {
        const TopoDS_Shape* body = document_->getBodyShape(targetBodyId_);
        if (body) {
            targets.push_back(*body);
        }
    }

    if (targets.empty()) {
        booleanMode_ = app::BooleanMode::NewBody;
        return;
    }

    booleanMode_ = core::modeling::BooleanOperation::detectMode(tool, targets, axis_.Direction());
}

std::optional<ModelingTool::Indicator> RevolveTool::indicator() const {
    if (!active_ || state_ == State::WaitingForProfile || !axisValid_) {
        return std::nullopt;
    }

    ModelingTool::Indicator ind;
    ind.origin = QVector3D(baseCenter_.X(), baseCenter_.Y(), baseCenter_.Z());
    ind.direction = QVector3D(axis_.Direction().X(), axis_.Direction().Y(), axis_.Direction().Z());
    ind.showDistance = false;
    ind.booleanMode = booleanMode_;
    return ind;
}

} // namespace onecad::ui::tools
