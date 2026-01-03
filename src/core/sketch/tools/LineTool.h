/**
 * @file LineTool.h
 * @brief Line drawing tool with polyline mode
 */

#ifndef ONECAD_CORE_SKETCH_TOOLS_LINETOOL_H
#define ONECAD_CORE_SKETCH_TOOLS_LINETOOL_H

#include "SketchTool.h"

namespace onecad::core::sketch::tools {

/**
 * @brief Tool for drawing lines in polyline mode
 *
 * State machine:
 * - Idle: waiting for first click
 * - FirstClick: first point set, showing preview line
 * - Click again: creates line, continues from endpoint (polyline)
 * - ESC: cancels current operation, returns to Idle
 * - Right-click: finishes polyline, returns to Idle
 */
class LineTool : public SketchTool {
public:
    LineTool();
    ~LineTool() override = default;

    void onMousePress(const Vec2d& pos, Qt::MouseButton button) override;
    void onMouseMove(const Vec2d& pos) override;
    void onMouseRelease(const Vec2d& pos, Qt::MouseButton button) override;
    void onKeyPress(Qt::Key key) override;
    void cancel() override;
    void render(SketchRenderer& renderer) override;
    std::string name() const override { return "Line"; }

    /**
     * @brief Check if a line was just created (for signal emission)
     */
    bool wasLineCreated() const { return lineCreated_; }
    void clearLineCreatedFlag() { lineCreated_ = false; }

private:
    Vec2d startPoint_{0, 0};
    Vec2d currentPoint_{0, 0};
    EntityID lastPointId_;  // For coincident constraint on polyline continuation
    bool lineCreated_ = false;
};

} // namespace onecad::core::sketch::tools

#endif // ONECAD_CORE_SKETCH_TOOLS_LINETOOL_H
