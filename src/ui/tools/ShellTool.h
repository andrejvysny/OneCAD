/**
 * @file ShellTool.h
 * @brief Shell tool for hollowing solid bodies.
 */
#ifndef ONECAD_UI_TOOLS_SHELLTOOL_H
#define ONECAD_UI_TOOLS_SHELLTOOL_H

#include "ModelingTool.h"

#include "../../render/tessellation/TessellationCache.h"
#include "../../kernel/elementmap/ElementMap.h"

#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>
#include <string>
#include <vector>

namespace onecad::app {
class Document;
}
namespace onecad::app::commands {
class CommandProcessor;
}

namespace onecad::ui {
class Viewport;
}

namespace onecad::ui::tools {

class ShellTool : public ModelingTool {
public:
    enum class State {
        WaitingForBody,
        WaitingForFaces,
        Dragging
    };

    explicit ShellTool(Viewport* viewport, app::Document* document);

    void setDocument(app::Document* document);
    void setCommandProcessor(app::commands::CommandProcessor* processor);

    void begin(const app::selection::SelectionItem& selection) override;
    void cancel() override;
    bool isActive() const override { return active_; }
    bool isDragging() const override { return dragging_; }

    bool handleMousePress(const QPoint& screenPos, Qt::MouseButton button) override;
    bool handleMouseMove(const QPoint& screenPos) override;
    bool handleMouseRelease(const QPoint& screenPos, Qt::MouseButton button) override;
    std::optional<Indicator> indicator() const override;

    // For multi-face selection (Shift+click)
    bool addOpenFace(const app::selection::SelectionItem& selection);
    void clearOpenFaces();
    void confirmFaceSelection();  // Called when Enter is pressed

    State currentState() const { return state_; }
    size_t openFaceCount() const { return openFaces_.size(); }
    const std::string& targetBodyId() const { return targetBodyId_; }

private:
    bool prepareBody(const app::selection::SelectionItem& selection);
    void updatePreview(double thickness);
    void clearPreview();
    TopoDS_Shape buildShellShape(double thickness) const;
    void commitOperation(double thickness);

    Viewport* viewport_ = nullptr;
    app::Document* document_ = nullptr;
    app::commands::CommandProcessor* commandProcessor_ = nullptr;

    State state_ = State::WaitingForBody;
    std::string targetBodyId_;
    TopoDS_Shape targetShape_;
    std::vector<TopoDS_Face> openFaces_;
    gp_Pnt indicatorOrigin_;

    bool active_ = false;
    bool dragging_ = false;
    QPoint dragStart_;
    double currentThickness_ = 0.0;

    render::TessellationCache previewTessellator_;
    kernel::elementmap::ElementMap previewElementMap_;
};

} // namespace onecad::ui::tools

#endif // ONECAD_UI_TOOLS_SHELLTOOL_H
