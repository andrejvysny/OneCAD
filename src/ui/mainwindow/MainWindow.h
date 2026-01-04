#ifndef ONECAD_UI_MAINWINDOW_H
#define ONECAD_UI_MAINWINDOW_H

#include <QMainWindow>
#include <memory>
#include <string>

class QLabel;
class QSlider;
class QEvent;
namespace onecad {
namespace ui {
class SidebarToolButton;
}
}

namespace onecad {
namespace app {
    class Document;
}
namespace core::sketch {
    class Sketch;
}
namespace ui {

class Viewport;
class ModelNavigator;
class ContextToolbar;
class ConstraintPanel;

/**
 * @brief Main application window for OneCAD.
 * 
 * Layout per specification:
 * - Top: Menu bar
 * - Left: Docked navigator with floating action sidebar on the viewport
 * - Center: 3D Viewport
 * - Right: Property inspector (collapsible dock)
 * - Bottom: Status bar
 */
class MainWindow : public QMainWindow {
    Q_OBJECT

public:
    explicit MainWindow(QWidget* parent = nullptr);
    ~MainWindow() override;

private slots:
    void onNewSketch();
    void onExitSketch();
    void onSketchModeChanged(bool inSketchMode);
    void onSketchPlanePicked(int planeIndex);
    void onPlaneSelectionCancelled();
    void onImport();
    void onMousePositionChanged(double x, double y, double z);
    void onSketchUpdated();

private:
    void setupMenuBar();
    void setupToolBar();
    void setupViewport();
    void setupStatusBar();
    void applyTheme();
    void updateDofStatus(core::sketch::Sketch* sketch);
    void positionToolbarOverlay();
    void setupNavigatorOverlayButton();
    void positionNavigatorOverlayButton();
    void positionConstraintPanel();

    bool eventFilter(QObject* obj, QEvent* event) override;

    // UI Components
    Viewport* m_viewport = nullptr;
    ModelNavigator* m_navigator = nullptr;
    ContextToolbar* m_toolbar = nullptr;
    SidebarToolButton* m_navigatorOverlayButton = nullptr;
    ConstraintPanel* m_constraintPanel = nullptr;

    // Document model (owns all sketches)
    std::unique_ptr<app::Document> m_document;

    // Active editing state
    std::string m_activeSketchId;  // Currently editing sketch ID (empty if not in sketch mode)
    
    // Status bar labels
    QLabel* m_toolStatus = nullptr;
    QLabel* m_dofStatus = nullptr;
    QLabel* m_coordStatus = nullptr;

    // Camera angle control
    QLabel* m_cameraAngleLabel = nullptr;
    QSlider* m_cameraAngleSlider = nullptr;

    void loadSettings();
    void saveSettings();
};

} // namespace ui
} // namespace onecad

// For backward compatibility with existing main.cpp
using MainWindow = onecad::ui::MainWindow;

#endif // ONECAD_UI_MAINWINDOW_H
