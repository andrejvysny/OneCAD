#ifndef ONECAD_UI_TOOLBAR_CONTEXTTOOLBAR_H
#define ONECAD_UI_TOOLBAR_CONTEXTTOOLBAR_H

#include <QWidget>

class QHBoxLayout;
namespace onecad {
namespace ui {
class SidebarToolButton;
} // namespace ui
} // namespace onecad

namespace onecad {
namespace ui {

class ContextToolbar : public QWidget {
    Q_OBJECT

public:
    explicit ContextToolbar(QWidget* parent = nullptr);
    ~ContextToolbar() override = default;

    enum class Context {
        Default,
        Sketch,
        Body,
        Edge,
        Face
    };

public slots:
    void setContext(Context context);
    void setExtrudeActive(bool active);
    void setRevolveActive(bool active);

signals:
    void contextChanged();
    void newSketchRequested();
    void extrudeRequested();
    void revolveRequested();
    void exitSketchRequested();
    void importRequested();
    void lineToolActivated();
    void rectangleToolActivated();
    void circleToolActivated();
    void arcToolActivated();
    void ellipseToolActivated();
    void trimToolActivated();
    void mirrorToolActivated();

private:
    void setupUi();
    void updateVisibleButtons();

    Context m_currentContext = Context::Default;
    QHBoxLayout* m_layout = nullptr;
    SidebarToolButton* m_newSketchButton = nullptr;
    SidebarToolButton* m_extrudeButton = nullptr;
    SidebarToolButton* m_revolveButton = nullptr;
    SidebarToolButton* m_importButton = nullptr;
    SidebarToolButton* m_exitSketchButton = nullptr;
    SidebarToolButton* m_lineButton = nullptr;
    SidebarToolButton* m_rectangleButton = nullptr;
    SidebarToolButton* m_circleButton = nullptr;
    SidebarToolButton* m_arcButton = nullptr;
    SidebarToolButton* m_ellipseButton = nullptr;
    SidebarToolButton* m_trimButton = nullptr;
    SidebarToolButton* m_mirrorButton = nullptr;
};

} // namespace ui
} // namespace onecad

#endif // ONECAD_UI_TOOLBAR_CONTEXTTOOLBAR_H
