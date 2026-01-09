/**
 * @file ModelingToolManager.h
 * @brief Manages active 3D modeling tools.
 */
#ifndef ONECAD_UI_TOOLS_MODELINGTOOLMANAGER_H
#define ONECAD_UI_TOOLS_MODELINGTOOLMANAGER_H

#include "ModelingTool.h"

#include <QObject>
#include <memory>
#include <optional>
#include <string>

namespace onecad::app {
class Document;
namespace commands {
class CommandProcessor;
}
}

namespace onecad::ui {
class Viewport;
}

namespace onecad::ui::tools {

class ExtrudeTool;
class RevolveTool;
class FilletChamferTool;
class ShellTool;

class ModelingToolManager : public QObject {
    Q_OBJECT

public:
    explicit ModelingToolManager(Viewport* viewport, QObject* parent = nullptr);
    ~ModelingToolManager() override;

    void setDocument(app::Document* document);
    void setCommandProcessor(app::commands::CommandProcessor* processor);

    bool hasActiveTool() const;
    bool isDragging() const;

    void activateExtrude(const app::selection::SelectionItem& selection);
    void activateRevolve(const app::selection::SelectionItem& selection);
    void activateFillet(const app::selection::SelectionItem& selection);
    void activateShell(const app::selection::SelectionItem& selection);
    void cancelActiveTool();

    // Pass selection changes to active tool (needed for Revolve's Axis selection)
    void onSelectionChanged(const std::vector<app::selection::SelectionItem>& selection);

    bool toggleFilletMode();
    bool toggleShellOpenFace(const app::selection::SelectionItem& selection);
    bool confirmShellFaceSelection();
    std::optional<std::string> activeShellBodyId() const;

    bool handleMousePress(const QPoint& screenPos, Qt::MouseButton button);
    bool handleMouseMove(const QPoint& screenPos);
    bool handleMouseRelease(const QPoint& screenPos, Qt::MouseButton button);
    std::optional<ModelingTool::Indicator> activeIndicator() const;

private:
    Viewport* viewport_ = nullptr;
    app::Document* document_ = nullptr;
    app::commands::CommandProcessor* commandProcessor_ = nullptr;
    std::unique_ptr<ExtrudeTool> extrudeTool_;
    std::unique_ptr<RevolveTool> revolveTool_;
    std::unique_ptr<FilletChamferTool> filletTool_;
    std::unique_ptr<ShellTool> shellTool_;
    ModelingTool* activeTool_ = nullptr;
    app::selection::SelectionKey activeSelection_{};
};

} // namespace onecad::ui::tools

#endif // ONECAD_UI_TOOLS_MODELINGTOOLMANAGER_H
