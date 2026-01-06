#include "ContextToolbar.h"
#include "../components/SidebarToolButton.h"

#include <QHBoxLayout>
#include <QSizePolicy>

namespace onecad {
namespace ui {

ContextToolbar::ContextToolbar(QWidget* parent)
    : QWidget(parent) {
    setObjectName("ContextToolbar");
    setAttribute(Qt::WA_StyledBackground, true);
    setFixedHeight(64);
    // Use Preferred size policy to allow layout to calculate proper width
    setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Fixed);
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

void ContextToolbar::setExtrudeActive(bool active) {
    if (!m_extrudeButton) {
        return;
    }
    m_extrudeButton->blockSignals(true);
    m_extrudeButton->setChecked(active);
    m_extrudeButton->blockSignals(false);
}

void ContextToolbar::setRevolveActive(bool active) {
    if (!m_revolveButton) {
        return;
    }
    m_revolveButton->blockSignals(true);
    m_revolveButton->setChecked(active);
    m_revolveButton->blockSignals(false);
}

void ContextToolbar::setupUi() {
    m_layout = new QHBoxLayout(this);
    m_layout->setContentsMargins(12, 8, 12, 8);  // Better padding
    m_layout->setSpacing(8);  // Clearer visual separation
    m_layout->setAlignment(Qt::AlignVCenter);  // Center buttons vertically

    m_layout->addStretch();

    m_newSketchButton = SidebarToolButton::fromSvgIcon(":/icons/ic_sketch.svg", tr("Create a new sketch (S)"), this);
    connect(m_newSketchButton, &SidebarToolButton::clicked, this, &ContextToolbar::newSketchRequested);

    m_extrudeButton = SidebarToolButton::fromSvgIcon(":/icons/ic_extrude.svg", tr("Extrude (E)"), this);
    m_extrudeButton->setCheckable(true);
    connect(m_extrudeButton, &SidebarToolButton::clicked, this, &ContextToolbar::extrudeRequested);

    m_revolveButton = SidebarToolButton::fromSvgIcon(":/icons/ic_revolve.svg", tr("Revolve"), this);
    m_revolveButton->setCheckable(true);
    connect(m_revolveButton, &SidebarToolButton::clicked, this, &ContextToolbar::revolveRequested);

    m_importButton = SidebarToolButton::fromSvgIcon(":/icons/ic_import.svg", tr("Import STEP file"), this);
    connect(m_importButton, &SidebarToolButton::clicked, this, &ContextToolbar::importRequested);

    m_exitSketchButton = SidebarToolButton::fromSvgIcon(":/icons/ic_close.svg", tr("Exit sketch mode (Esc)"), this);
    connect(m_exitSketchButton, &SidebarToolButton::clicked, this, &ContextToolbar::exitSketchRequested);

    m_lineButton = SidebarToolButton::fromSvgIcon(":/icons/ic_line.svg", tr("Draw line (L)"), this);
    connect(m_lineButton, &SidebarToolButton::clicked, this, &ContextToolbar::lineToolActivated);

    m_rectangleButton = SidebarToolButton::fromSvgIcon(":/icons/ic_rectangle.svg", tr("Draw rectangle (R)"), this);
    connect(m_rectangleButton, &SidebarToolButton::clicked, this, &ContextToolbar::rectangleToolActivated);

    m_circleButton = SidebarToolButton::fromSvgIcon(":/icons/ic_circle.svg", tr("Draw circle (C)"), this);
    connect(m_circleButton, &SidebarToolButton::clicked, this, &ContextToolbar::circleToolActivated);

    m_arcButton = SidebarToolButton::fromSvgIcon(":/icons/ic_arc.svg", tr("Draw arc (A)"), this);
    connect(m_arcButton, &SidebarToolButton::clicked, this, &ContextToolbar::arcToolActivated);

    m_ellipseButton = SidebarToolButton::fromSvgIcon(":/icons/ic_ellipse.svg", tr("Draw ellipse (E)"), this);
    connect(m_ellipseButton, &SidebarToolButton::clicked, this, &ContextToolbar::ellipseToolActivated);

    m_trimButton = SidebarToolButton::fromSvgIcon(":/icons/ic_trim.svg", tr("Trim entity (T)"), this);
    connect(m_trimButton, &SidebarToolButton::clicked, this, &ContextToolbar::trimToolActivated);

    m_mirrorButton = SidebarToolButton::fromSvgIcon(":/icons/ic_mirror.svg", tr("Mirror geometry (M)"), this);
    connect(m_mirrorButton, &SidebarToolButton::clicked, this, &ContextToolbar::mirrorToolActivated);

    m_layout->addWidget(m_newSketchButton);
    m_layout->addWidget(m_extrudeButton);
    m_layout->addWidget(m_revolveButton);
    m_layout->addWidget(m_importButton);
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
    if (m_extrudeButton) {
        m_extrudeButton->setVisible(!inSketch);
    }
    if (m_revolveButton) {
        m_revolveButton->setVisible(!inSketch);
    }
    if (m_importButton) {
        m_importButton->setVisible(m_currentContext == Context::Default);
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
