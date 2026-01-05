/**
 * @file ModelingToolManager.h
 * @brief Manages active 3D modeling tools.
 */
#ifndef ONECAD_UI_TOOLS_MODELINGTOOLMANAGER_H
#define ONECAD_UI_TOOLS_MODELINGTOOLMANAGER_H

#include "ModelingTool.h"

#include <QObject>
#include <memory>

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
    void cancelActiveTool();

    bool handleMousePress(const QPoint& screenPos, Qt::MouseButton button);
    bool handleMouseMove(const QPoint& screenPos);
    bool handleMouseRelease(const QPoint& screenPos, Qt::MouseButton button);

private:
    Viewport* viewport_ = nullptr;
    app::Document* document_ = nullptr;
    app::commands::CommandProcessor* commandProcessor_ = nullptr;
    std::unique_ptr<ExtrudeTool> extrudeTool_;
    ModelingTool* activeTool_ = nullptr;
    app::selection::SelectionKey activeSelection_{};
};

} // namespace onecad::ui::tools

#endif // ONECAD_UI_TOOLS_MODELINGTOOLMANAGER_H
