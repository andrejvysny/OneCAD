/**
 * @file FilletChamferTool.h
 * @brief Combined fillet/chamfer tool for edge rounding and beveling.
 */
#ifndef ONECAD_UI_TOOLS_FILLETCHAMFERTOOL_H
#define ONECAD_UI_TOOLS_FILLETCHAMFERTOOL_H

#include "ModelingTool.h"

#include "../../render/tessellation/TessellationCache.h"
#include "../../kernel/elementmap/ElementMap.h"

#include <TopoDS_Edge.hxx>
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

class FilletChamferTool : public ModelingTool {
public:
    enum class Mode { Fillet, Chamfer };

    explicit FilletChamferTool(Viewport* viewport, app::Document* document);

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

    Mode currentMode() const { return mode_; }
    void toggleMode();  // Switch between fillet/chamfer

private:
    bool prepareInput(const app::selection::SelectionItem& selection);
    void expandEdgeChain();
    void updatePreview(double value);
    void clearPreview();
    TopoDS_Shape buildFilletShape(double radius) const;
    TopoDS_Shape buildChamferShape(double distance) const;
    void detectMode(double dragDistance);

    Viewport* viewport_ = nullptr;
    app::Document* document_ = nullptr;
    app::commands::CommandProcessor* commandProcessor_ = nullptr;

    std::string targetBodyId_;
    TopoDS_Shape targetShape_;
    std::vector<TopoDS_Edge> selectedEdges_;
    gp_Pnt edgeMidpoint_;

    Mode mode_ = Mode::Fillet;
    bool active_ = false;
    bool dragging_ = false;
    QPoint dragStart_;
    double currentValue_ = 0.0;  // Radius for fillet, distance for chamfer

    render::TessellationCache previewTessellator_;
    kernel::elementmap::ElementMap previewElementMap_;
};

} // namespace onecad::ui::tools

#endif // ONECAD_UI_TOOLS_FILLETCHAMFERTOOL_H
