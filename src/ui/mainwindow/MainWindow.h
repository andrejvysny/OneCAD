#ifndef ONECAD_UI_MAINWINDOW_H
#define ONECAD_UI_MAINWINDOW_H

#include <QMainWindow>
#include <memory>

class QLabel;
class QSlider;
class QEvent;

namespace onecad {
namespace core::sketch {
    class Sketch;
}
namespace ui {

class Viewport;
class ModelNavigator;
class ContextToolbar;

/**
 * @brief Main application window for OneCAD.
 * 
 * Layout per specification:
 * - Top: Menu bar + Context toolbar
 * - Left: Model navigator (collapsible dock)
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
    void onImport();
    void onMousePositionChanged(double x, double y, double z);

private:
    void setupMenuBar();
    void setupToolBar();
    void setupViewport();
    void setupNavigatorOverlay();
    void setupStatusBar();
    void applyTheme();
    void positionNavigatorOverlay();

    bool eventFilter(QObject* obj, QEvent* event) override;

    // UI Components
    Viewport* m_viewport = nullptr;
    ModelNavigator* m_navigator = nullptr;
    ContextToolbar* m_toolbar = nullptr;

    // Sketch state
    std::unique_ptr<core::sketch::Sketch> m_currentSketch;
    
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
