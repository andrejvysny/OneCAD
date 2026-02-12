#include "LineTool.h"
#include "../Sketch.h"
#include "../SketchRenderer.h"
#include "../AutoConstrainer.h"
#include "../IntersectionManager.h"
#include "../SketchLine.h"
#include "../SketchPoint.h"

#include <QLoggingCategory>
#include <QString>

#include <cstdio>
#include <string>
#include <cmath>

namespace onecad::core::sketch::tools {

Q_LOGGING_CATEGORY(logLineTool, "onecad.core.sketchtool.line")

LineTool::LineTool() = default;

std::string LineTool::name() const {
    return "Line";
}

std::optional<Vec2d> LineTool::getReferencePoint() const {
    if (state_ == State::FirstClick) {
        return startPoint_;
    }
    return std::nullopt;
}

void LineTool::onMousePress(const Vec2d& pos, Qt::MouseButton button) {
    qCDebug(logLineTool) << "onMousePress" << "state=" << static_cast<int>(state_)
                         << "pos=" << pos.x << pos.y << "button=" << static_cast<int>(button)
                         << "snapType=" << static_cast<int>(snapResult_.type)
                         << "snapped=" << snapResult_.snapped;
    if (button == Qt::RightButton) {
        // Right-click finishes polyline
        cancel();
        return;
    }

    if (button != Qt::LeftButton) {
        return;
    }

    lineCreated_ = false;
    lastRejectReason_ = RejectReason::None;

    if (state_ == State::Idle) {
        // First click - record start point
        startPoint_ = pos;
        currentPoint_ = pos;
        startPointId_.clear();
        if (snapResult_.snapped && !snapResult_.pointId.empty()) {
            startPointId_ = snapResult_.pointId;
        }
        state_ = State::FirstClick;
    } else if (state_ == State::FirstClick) {
        // Second click - create line
        if (!sketch_) {
            return;
        }

        // Check minimum length to avoid degenerate geometry
        double dx = pos.x - startPoint_.x;
        double dy = pos.y - startPoint_.y;
        double length = std::sqrt(dx * dx + dy * dy);

        if (length < constants::MIN_GEOMETRY_SIZE) {
            qCDebug(logLineTool) << "reject:too-short" << "length=" << length;
            lastRejectReason_ = RejectReason::TooShort;
            return;
        }

        EntityID startId = startPointId_;
        if (startId.empty()) {
            startId = sketch_->addPoint(startPoint_.x, startPoint_.y);
        }

        EntityID endId;
        if (snapResult_.snapped && !snapResult_.pointId.empty()) {
            endId = snapResult_.pointId;
        } else {
            endId = sketch_->addPoint(pos.x, pos.y);
        }

        if (startId.empty() || endId.empty()) {
            qCWarning(logLineTool) << "reject:invalid-endpoints"
                                  << "startId=" << QString::fromStdString(startId)
                                  << "endId=" << QString::fromStdString(endId);
            lastRejectReason_ = RejectReason::InvalidEndpoints;
            return;
        }

        // Guard against guide-driven endpoint collapse: if a guide snap reused
        // the start point ID but the geometric end position is distinct,
        // materialize a new endpoint at click position.
        if (startId == endId) {
            if (snapResult_.snapped && snapResult_.hasGuide) {
                endId = sketch_->addPoint(pos.x, pos.y);
            }
            if (endId.empty() || startId == endId) {
                qCDebug(logLineTool) << "reject:same-endpoint-after-guide";
                lastRejectReason_ = RejectReason::SameEndpoint;
                return;
            }
        }

        if (startId.empty() || endId.empty()) {
            lastRejectReason_ = RejectReason::InvalidEndpoints;
            return;
        }

        EntityID lineId = sketch_->addLine(startId, endId);

        qCDebug(logLineTool) << "line-create-attempt"
                             << "startId=" << QString::fromStdString(startId)
                             << "endId=" << QString::fromStdString(endId)
                             << "lineId=" << QString::fromStdString(lineId);

        if (!lineId.empty()) {
            lineCreated_ = true;

            // Process intersections - split existing entities at intersection points
            if (snapManager_) {
                IntersectionManager intersectionMgr;
                intersectionMgr.processIntersections(lineId, *sketch_, *snapManager_);
            }

            // Get the end point for polyline continuation
            // The line stores startPointId and endPointId
            auto* line = sketch_->getEntityAs<SketchLine>(lineId);
            if (line) {
                lastPointId_ = line->endPointId();
                startPointId_ = line->endPointId();

                // Apply inferred constraints
                if (autoConstrainer_ && autoConstrainer_->isEnabled()) {
                    DrawingContext context;
                    context.activeEntity = lineId;
                    context.previousEntity = lastCreatedLineId_;
                    context.startPoint = startPoint_;
                    context.currentPoint = pos;
                    context.isPolylineMode = !lastCreatedLineId_.empty();

                    auto constraints = autoConstrainer_->inferLineConstraints(
                        startPoint_, pos, lineId, *sketch_, context);

                    // Filter and apply high-confidence constraints
                    auto toApply = autoConstrainer_->filterForAutoApply(constraints);
                    qCDebug(logLineTool) << "auto-constraints"
                                         << "inferred=" << constraints.size()
                                         << "toApply=" << toApply.size()
                                         << "lineId=" << QString::fromStdString(lineId);
                    applyInferredConstraints(toApply, lineId);
                }

                lastCreatedLineId_ = lineId;
            }

            // Continue polyline: new start = old end
            startPoint_ = pos;
            currentPoint_ = pos;
            // Stay in FirstClick state for polyline mode
            lastRejectReason_ = RejectReason::None;
        }
    }
}

void LineTool::onMouseMove(const Vec2d& pos) {
    currentPoint_ = pos;

    // Update preview constraints
    updateInferredConstraints();
}

void LineTool::updateInferredConstraints() {
    qCDebug(logLineTool) << "updateInferredConstraints"
                         << "state=" << static_cast<int>(state_)
                         << "hasAutoConstrainer=" << (autoConstrainer_ != nullptr)
                         << "hasSketch=" << (sketch_ != nullptr);
    inferredConstraints_.clear();

    if (state_ != State::FirstClick || !autoConstrainer_ || !sketch_) {
        return;
    }

    DrawingContext context;
    context.activeEntity = {};  // Line not created yet
    context.previousEntity = lastCreatedLineId_;
    context.startPoint = startPoint_;
    context.currentPoint = currentPoint_;
    context.isPolylineMode = !lastCreatedLineId_.empty();

    // Use empty lineId since line doesn't exist yet
    inferredConstraints_ = autoConstrainer_->inferLineConstraints(
        startPoint_, currentPoint_, {}, *sketch_, context);
    qCDebug(logLineTool) << "updateInferredConstraints:done"
                         << "count=" << inferredConstraints_.size();
}

void LineTool::applyInferredConstraints(const std::vector<InferredConstraint>& constraints,
                                         EntityID lineId) {
    qCDebug(logLineTool) << "applyInferredConstraints"
                         << "lineId=" << QString::fromStdString(lineId)
                         << "count=" << constraints.size();
    if (!sketch_ || lineId.empty()) return;

    auto* line = sketch_->getEntityAs<SketchLine>(lineId);
    if (!line) return;

    for (const auto& constraint : constraints) {
        qCDebug(logLineTool) << "applyInferredConstraints:item"
                             << "type=" << static_cast<int>(constraint.type)
                             << "entity1=" << QString::fromStdString(constraint.entity1)
                             << "entity2=" << QString::fromStdString(constraint.entity2.value_or(std::string{}))
                             << "confidence=" << constraint.confidence;
        switch (constraint.type) {
            case ConstraintType::Horizontal:
                sketch_->addHorizontal(lineId);
                break;

            case ConstraintType::Vertical:
                sketch_->addVertical(lineId);
                break;

            case ConstraintType::Perpendicular:
                if (constraint.entity2) {
                    sketch_->addPerpendicular(lineId, *constraint.entity2);
                }
                break;

            case ConstraintType::Parallel:
                if (constraint.entity2) {
                    sketch_->addParallel(lineId, *constraint.entity2);
                }
                break;

            case ConstraintType::Coincident:
                if (!constraint.entity1.empty()) {
                    const auto* existing = sketch_->getEntityAs<SketchPoint>(constraint.entity1);
                    const auto* startPt = sketch_->getEntityAs<SketchPoint>(line->startPointId());
                    const auto* endPt = sketch_->getEntityAs<SketchPoint>(line->endPointId());
                    if (!existing || !startPt || !endPt) {
                        break;
                    }

                    if (constraint.entity1 == line->startPointId() ||
                        constraint.entity1 == line->endPointId()) {
                        break;
                    }

                    Vec2d existingPos{existing->position().X(), existing->position().Y()};
                    Vec2d startPos{startPt->position().X(), startPt->position().Y()};
                    Vec2d endPos{endPt->position().X(), endPt->position().Y()};
                    double tolerance = autoConstrainer_
                        ? autoConstrainer_->getConfig().coincidenceTolerance
                        : constants::SNAP_RADIUS_MM;

                    double startDist = std::hypot(existingPos.x - startPos.x, existingPos.y - startPos.y);
                    double endDist = std::hypot(existingPos.x - endPos.x, existingPos.y - endPos.y);

                    if (startDist <= tolerance) {
                        sketch_->addCoincident(line->startPointId(), constraint.entity1);
                    } else if (endDist <= tolerance) {
                        sketch_->addCoincident(line->endPointId(), constraint.entity1);
                    }
                }
                break;

            default:
                // Other constraint types not applicable to lines
                break;
        }
    }
}

void LineTool::onMouseRelease(const Vec2d& /*pos*/, Qt::MouseButton /*button*/) {
    // Line tool uses click-click, not drag, so nothing on release
}

void LineTool::onKeyPress(Qt::Key key) {
    if (key == Qt::Key_Escape) {
        cancel();
    }
}

void LineTool::cancel() {
    state_ = State::Idle;
    startPointId_.clear();
    lastPointId_.clear();
    lastCreatedLineId_.clear();
    lineCreated_ = false;
    lastRejectReason_ = RejectReason::None;
    inferredConstraints_.clear();
}

void LineTool::render(SketchRenderer& renderer) {
    if (state_ == State::FirstClick) {
        // Show preview line from start to current mouse position
        renderer.setPreviewLine(startPoint_, currentPoint_);

        // Calculate length for dimension
        double dx = currentPoint_.x - startPoint_.x;
        double dy = currentPoint_.y - startPoint_.y;
        double length = std::sqrt(dx * dx + dy * dy);

        if (length > constants::MIN_GEOMETRY_SIZE) {
            char buffer[32];
            std::snprintf(buffer, sizeof(buffer), "%.2f", length);
            
            Vec2d midPoint = {
                (startPoint_.x + currentPoint_.x) * 0.5,
                (startPoint_.y + currentPoint_.y) * 0.5
            };

            renderer.setPreviewDimensions({{midPoint, std::string(buffer)}});
        } else {
            renderer.clearPreviewDimensions();
        }
    } else {
        renderer.clearPreview();
    }
}

} // namespace onecad::core::sketch::tools
