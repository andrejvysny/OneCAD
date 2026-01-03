#include "ContextToolbar.h"
#include <QAction>

namespace onecad {
namespace ui {

ContextToolbar::ContextToolbar(QWidget* parent)
    : QToolBar(tr("Tools"), parent) {
    setMovable(false);
    setIconSize(QSize(24, 24));
    setToolButtonStyle(Qt::ToolButtonTextUnderIcon);
    
    setupDefaultActions();
    setupSketchActions();
    updateVisibleActions();
}

void ContextToolbar::setupDefaultActions() {
    m_newSketchAction = new QAction(tr("✏️ New Sketch"), this);
    m_newSketchAction->setToolTip(tr("Create a new sketch (S)"));
    connect(m_newSketchAction, &QAction::triggered, 
            this, &ContextToolbar::newSketchRequested);
    addAction(m_newSketchAction);
    
    m_importAction = new QAction(tr("📂 Import"), this);
    m_importAction->setToolTip(tr("Import STEP file"));
    connect(m_importAction, &QAction::triggered,
            this, &ContextToolbar::importRequested);
    addAction(m_importAction);
}

void ContextToolbar::setupSketchActions() {
    addSeparator();

    m_exitSketchAction = new QAction(tr("✓ Done"), this);
    m_exitSketchAction->setToolTip(tr("Exit sketch mode (Escape)"));
    connect(m_exitSketchAction, &QAction::triggered,
            this, &ContextToolbar::exitSketchRequested);
    addAction(m_exitSketchAction);

    addSeparator();

    m_lineAction = new QAction(tr("📏 Line"), this);
    m_lineAction->setToolTip(tr("Draw line (L)"));
    connect(m_lineAction, &QAction::triggered,
            this, &ContextToolbar::lineToolActivated);
    addAction(m_lineAction);
    
    m_rectangleAction = new QAction(tr("⬜ Rectangle"), this);
    m_rectangleAction->setToolTip(tr("Draw rectangle (R)"));
    connect(m_rectangleAction, &QAction::triggered,
            this, &ContextToolbar::rectangleToolActivated);
    addAction(m_rectangleAction);
    
    m_circleAction = new QAction(tr("⭕ Circle"), this);
    m_circleAction->setToolTip(tr("Draw circle (C)"));
    connect(m_circleAction, &QAction::triggered,
            this, &ContextToolbar::circleToolActivated);
    addAction(m_circleAction);
    
    m_arcAction = new QAction(tr("◠ Arc"), this);
    m_arcAction->setToolTip(tr("Draw arc (A)"));
    addAction(m_arcAction);
}

void ContextToolbar::setContext(Context context) {
    if (m_currentContext == context) return;
    m_currentContext = context;
    updateVisibleActions();
}

void ContextToolbar::updateVisibleActions() {
    bool inSketch = (m_currentContext == Context::Sketch);

    // Default actions - hide New Sketch button when in sketch mode
    m_newSketchAction->setVisible(!inSketch);
    m_importAction->setVisible(m_currentContext == Context::Default);

    // Sketch actions only in sketch mode
    m_exitSketchAction->setVisible(inSketch);
    m_lineAction->setVisible(inSketch);
    m_rectangleAction->setVisible(inSketch);
    m_circleAction->setVisible(inSketch);
    m_arcAction->setVisible(inSketch);
}

} // namespace ui
} // namespace onecad
