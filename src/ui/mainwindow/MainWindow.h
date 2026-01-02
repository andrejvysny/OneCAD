#ifndef ONECAD_UI_MAINWINDOW_H
#define ONECAD_UI_MAINWINDOW_H

#include <QMainWindow>

class QLabel;

namespace onecad {
namespace ui {

class Viewport;
class ModelNavigator;
class PropertyInspector;
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
    void onImport();
    void onMousePositionChanged(double x, double y, double z);

private:
    void setupMenuBar();
    void setupToolBar();
    void setupDocks();
    void setupViewport();
    void setupStatusBar();
    void applyTheme();

    // UI Components
    Viewport* m_viewport = nullptr;
    ModelNavigator* m_navigator = nullptr;
    PropertyInspector* m_inspector = nullptr;
    ContextToolbar* m_toolbar = nullptr;
    
    // Status bar labels
    QLabel* m_toolStatus = nullptr;
    QLabel* m_dofStatus = nullptr;
    QLabel* m_coordStatus = nullptr;
};

} // namespace ui
} // namespace onecad

// For backward compatibility with existing main.cpp
using MainWindow = onecad::ui::MainWindow;

#endif // ONECAD_UI_MAINWINDOW_H
