#include "ContextToolbar.h"
#include <QAction>

namespace onecad {
namespace ui {

ContextToolbar::ContextToolbar(QWidget* parent)
    : QToolBar(tr("Tools"), parent) {
    setMovable(false);
    setIconSize(QSize(24, 24));
    setToolButtonStyle(Qt::ToolButtonTextUnderIcon);
    
    // Style
    setStyleSheet(R"(
        QToolBar {
            background-color: #2d2d30;
            border: none;
            border-bottom: 1px solid #3e3e42;
            spacing: 4px;
            padding: 4px;
        }
        QToolButton {
            background-color: transparent;
            border: 1px solid transparent;
            border-radius: 4px;
            padding: 6px 12px;
            color: #cccccc;
            min-width: 60px;
        }
        QToolButton:hover {
            background-color: #3e3e42;
            border-color: #4e4e52;
        }
        QToolButton:pressed {
            background-color: #094771;
        }
    )");
    
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
    // Default actions always visible
    m_newSketchAction->setVisible(true);
    m_importAction->setVisible(m_currentContext == Context::Default);
    
    // Sketch actions only in sketch mode
    bool inSketch = (m_currentContext == Context::Sketch);
    m_lineAction->setVisible(inSketch);
    m_rectangleAction->setVisible(inSketch);
    m_circleAction->setVisible(inSketch);
    m_arcAction->setVisible(inSketch);
}

} // namespace ui
} // namespace onecad
