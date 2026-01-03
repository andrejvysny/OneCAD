#ifndef ONECAD_UI_TOOLBAR_CONTEXTTOOLBAR_H
#define ONECAD_UI_TOOLBAR_CONTEXTTOOLBAR_H

#include <QToolBar>

class QAction;

namespace onecad {
namespace ui {

/**
 * @brief Context-sensitive toolbar that changes based on selection.
 * 
 * Shows different tools depending on current state:
 * - Nothing selected: New Sketch, Import
 * - In sketch mode: Line, Rectangle, Circle, etc.
 * - Bodies selected: Boolean operations
 */
class ContextToolbar : public QToolBar {
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

signals:
    void newSketchRequested();
    void exitSketchRequested();
    void importRequested();
    void lineToolActivated();
    void rectangleToolActivated();
    void circleToolActivated();

private:
    void setupDefaultActions();
    void setupSketchActions();
    void updateVisibleActions();

    Context m_currentContext = Context::Default;
    
    // Default context actions
    QAction* m_newSketchAction = nullptr;
    QAction* m_importAction = nullptr;
    
    // Sketch context actions
    QAction* m_exitSketchAction = nullptr;
    QAction* m_lineAction = nullptr;
    QAction* m_rectangleAction = nullptr;
    QAction* m_circleAction = nullptr;
    QAction* m_arcAction = nullptr;
};

} // namespace ui
} // namespace onecad

#endif // ONECAD_UI_TOOLBAR_CONTEXTTOOLBAR_H
