#include "ContextToolbar.h"
#include "../components/SidebarToolButton.h"

#include <QVBoxLayout>
#include <QSizePolicy>

namespace onecad {
namespace ui {

ContextToolbar::ContextToolbar(QWidget* parent)
    : QWidget(parent) {
    setObjectName("ContextToolbar");
    setAttribute(Qt::WA_StyledBackground, true);
    setFixedWidth(64);
    // Use Preferred size policy to allow layout to calculate proper height
    setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Preferred);
    setupUi();
    setContext(Context::Default);
}

void ContextToolbar::setContext(Context context) {
    if (m_currentContext == context) {
        return;
    }

    m_currentContext = context;
    updateVisibleButtons();
    emit contextChanged();
}

void ContextToolbar::setupUi() {
    m_layout = new QVBoxLayout(this);
    m_layout->setContentsMargins(8, 12, 8, 12);  // Better padding
    m_layout->setSpacing(8);  // Clearer visual separation
    m_layout->setAlignment(Qt::AlignHCenter);  // Center buttons horizontally

    m_layout->addStretch();

    m_newSketchButton = SidebarToolButton::fromSvgIcon(":/icons/paper_pencil.svg", tr("Create a new sketch (S)"), this);
    connect(m_newSketchButton, &SidebarToolButton::clicked, this, &ContextToolbar::newSketchRequested);

    m_importButton = new SidebarToolButton("📂", tr("Import STEP file"), this);
    connect(m_importButton, &SidebarToolButton::clicked, this, &ContextToolbar::importRequested);

    m_debugBoxButton = new SidebarToolButton("📦", tr("Add debug box"), this);
    connect(m_debugBoxButton, &SidebarToolButton::clicked, this, &ContextToolbar::debugBoxRequested);

    m_exitSketchButton = new SidebarToolButton("✓", tr("Exit sketch mode (Esc)"), this);
    connect(m_exitSketchButton, &SidebarToolButton::clicked, this, &ContextToolbar::exitSketchRequested);

    m_lineButton = new SidebarToolButton("📏", tr("Draw line (L)"), this);
    connect(m_lineButton, &SidebarToolButton::clicked, this, &ContextToolbar::lineToolActivated);

    m_rectangleButton = new SidebarToolButton("⬜", tr("Draw rectangle (R)"), this);
    connect(m_rectangleButton, &SidebarToolButton::clicked, this, &ContextToolbar::rectangleToolActivated);

    m_circleButton = new SidebarToolButton("⭕", tr("Draw circle (C)"), this);
    connect(m_circleButton, &SidebarToolButton::clicked, this, &ContextToolbar::circleToolActivated);

    m_arcButton = new SidebarToolButton("◠", tr("Draw arc (A)"), this);
    connect(m_arcButton, &SidebarToolButton::clicked, this, &ContextToolbar::arcToolActivated);

    m_ellipseButton = new SidebarToolButton("⬭", tr("Draw ellipse (E)"), this);
    connect(m_ellipseButton, &SidebarToolButton::clicked, this, &ContextToolbar::ellipseToolActivated);

    m_trimButton = new SidebarToolButton("✂", tr("Trim entity (T)"), this);
    connect(m_trimButton, &SidebarToolButton::clicked, this, &ContextToolbar::trimToolActivated);

    m_mirrorButton = new SidebarToolButton("⇄", tr("Mirror geometry (M)"), this);
    connect(m_mirrorButton, &SidebarToolButton::clicked, this, &ContextToolbar::mirrorToolActivated);

    m_layout->addWidget(m_newSketchButton);
    m_layout->addWidget(m_importButton);
    m_layout->addWidget(m_debugBoxButton);
    m_layout->addWidget(m_exitSketchButton);
    m_layout->addWidget(m_lineButton);
    m_layout->addWidget(m_rectangleButton);
    m_layout->addWidget(m_circleButton);
    m_layout->addWidget(m_arcButton);
    m_layout->addWidget(m_ellipseButton);
    m_layout->addWidget(m_trimButton);
    m_layout->addWidget(m_mirrorButton);
    m_layout->addStretch();

    updateVisibleButtons();
    
    // Force size recalculation based on layout and content
    adjustSize();
}

void ContextToolbar::updateVisibleButtons() {
    const bool inSketch = (m_currentContext == Context::Sketch);

    if (m_newSketchButton) {
        m_newSketchButton->setVisible(!inSketch);
    }
    if (m_importButton) {
        m_importButton->setVisible(m_currentContext == Context::Default);
    }
    if (m_debugBoxButton) {
        m_debugBoxButton->setVisible(m_currentContext == Context::Default);
    }
    if (m_exitSketchButton) {
        m_exitSketchButton->setVisible(inSketch);
    }
    if (m_lineButton) {
        m_lineButton->setVisible(inSketch);
    }
    if (m_rectangleButton) {
        m_rectangleButton->setVisible(inSketch);
    }
    if (m_circleButton) {
        m_circleButton->setVisible(inSketch);
    }
    if (m_arcButton) {
        m_arcButton->setVisible(inSketch);
    }
    if (m_ellipseButton) {
        m_ellipseButton->setVisible(inSketch);
    }
    if (m_trimButton) {
        m_trimButton->setVisible(inSketch);
    }
    if (m_mirrorButton) {
        m_mirrorButton->setVisible(inSketch);
    }

    // Recalculate size when button visibility changes
    adjustSize();
}

} // namespace ui
} // namespace onecad
