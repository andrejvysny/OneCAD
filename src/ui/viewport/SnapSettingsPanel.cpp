#include "SnapSettingsPanel.h"
#include "../theme/ThemeManager.h"

#include <QCheckBox>
#include <QGridLayout>
#include <QGroupBox>
#include <QLabel>
#include <QVBoxLayout>
#include <QFrame>

namespace onecad::ui {

SnapSettingsPanel::SnapSettingsPanel(QWidget* parent)
    : QWidget(parent) {
    setupUi();
    connect(&ThemeManager::instance(), &ThemeManager::themeChanged,
            this, &SnapSettingsPanel::updateTheme, Qt::UniqueConnection);
    updateTheme();
}

void SnapSettingsPanel::setupUi() {
    setObjectName("SnapSettingsPanel");
    setWindowFlag(Qt::FramelessWindowHint, true);
    setAttribute(Qt::WA_StyledBackground, true);
    setFixedWidth(260);

    // switch-like style for checkboxes
    setStyleSheet(R"(
        SnapSettingsPanel {
            background-color: palette(window);
            border: 1px solid palette(mid);
            border-radius: 8px;
        }
        QLabel.header {
            font-weight: bold;
            font-size: 11px;
            color: palette(text);
            padding: 4px 0px;
        }
        QCheckBox {
            font-size: 12px;
            padding: 4px;
        }
        QCheckBox::indicator {
            width: 32px;
            height: 18px;
            border-radius: 9px;
            background-color: #555; /* Default off */
        }
        QCheckBox::indicator:checked {
            background-color: #007AFF; /* Blue on */
        }
        QCheckBox::indicator:unchecked:hover {
            background-color: #666;
        }
        QCheckBox::indicator:checked:hover {
            background-color: #0066DD;
        }
        /* Knob */
        QCheckBox::indicator::subcontrol {
            background-color: white;
            border-radius: 7px;
            width: 14px;
            height: 14px;
            margin: 2px;
        }
        QCheckBox::indicator:checked::subcontrol {
            subcontrol-position: center right;
        }
        QCheckBox::indicator:unchecked::subcontrol {
            subcontrol-position: center left;
        }
        QFrame.separator {
            background-color: palette(mid);
            max-height: 1px;
            border: none;
        }
    )");

    auto* layout = new QVBoxLayout(this);
    layout->setContentsMargins(12, 12, 12, 12);
    layout->setSpacing(8);

    // Section: Snap to
    auto* snapLabel = new QLabel(tr("Snap to"), this);
    snapLabel->setProperty("class", "header"); // Doesn't work with qss directly without explicit support or workaround, using objectName or direct style usually better but let's try selector
    snapLabel->setStyleSheet("font-weight: bold; font-size: 11px; color: grey;"); 
    layout->addWidget(snapLabel);

    m_snapGrid = createToggle(tr("Grid"));
    layout->addWidget(m_snapGrid);

    m_snapSketchLines = createToggle(tr("Sketch Guide Lines"));
    layout->addWidget(m_snapSketchLines);

    m_snapSketchPoints = createToggle(tr("Sketch Guide Points"));
    layout->addWidget(m_snapSketchPoints);

    m_snap3DPoints = createToggle(tr("3D Guide Points"));
    layout->addWidget(m_snap3DPoints);

    m_snap3DEdges = createToggle(tr("Distant Edges"));
    layout->addWidget(m_snap3DEdges);

    // Separator
    auto* sep = new QFrame(this);
    sep->setObjectName("separator");
    sep->setFrameShape(QFrame::HLine);
    sep->setFrameShadow(QFrame::Sunken);
    sep->setFixedHeight(1);
    sep->setStyleSheet("background-color: #444; border: none;");
    layout->addWidget(sep);

    // Section: Show
    auto* showLabel = new QLabel(tr("Show"), this);
    showLabel->setStyleSheet("font-weight: bold; font-size: 11px; color: grey;");
    layout->addWidget(showLabel);

    m_showGuidePoints = createToggle(tr("Guide Points"));
    layout->addWidget(m_showGuidePoints);

    m_showHints = createToggle(tr("Snapping Hints"));
    layout->addWidget(m_showHints);
}

QCheckBox* SnapSettingsPanel::createToggle(const QString& text) {
    auto* box = new QCheckBox(text, this);
    box->setCursor(Qt::PointingHandCursor);
    // Move text to left and indicator to right? Layout direction?
    // QCheckBox default is text right, indicator left.
    // To mimic screenshot: Text Left, Toggle Right.
    box->setLayoutDirection(Qt::RightToLeft);
    
    connect(box, &QCheckBox::toggled, this, &SnapSettingsPanel::settingsChanged);
    return box;
}

void SnapSettingsPanel::updateTheme() {
    // Rely on palette updates
}

void SnapSettingsPanel::setSettings(const SnapSettings& settings) {
    QSignalBlocker b1(m_snapGrid);
    QSignalBlocker b2(m_snapSketchLines);
    QSignalBlocker b3(m_snapSketchPoints);
    QSignalBlocker b4(m_snap3DPoints);
    QSignalBlocker b5(m_snap3DEdges);
    QSignalBlocker b6(m_showGuidePoints);
    QSignalBlocker b7(m_showHints);

    m_snapGrid->setChecked(settings.grid);
    m_snapSketchLines->setChecked(settings.sketchGuideLines);
    m_snapSketchPoints->setChecked(settings.sketchGuidePoints);
    m_snap3DPoints->setChecked(settings.activeLayer3DPoints);
    m_snap3DEdges->setChecked(settings.activeLayer3DEdges);
    m_showGuidePoints->setChecked(settings.showGuidePoints);
    m_showHints->setChecked(settings.showSnappingHints);
}

SnapSettingsPanel::SnapSettings SnapSettingsPanel::settings() const {
    SnapSettings s;
    s.grid = m_snapGrid->isChecked();
    s.sketchGuideLines = m_snapSketchLines->isChecked();
    s.sketchGuidePoints = m_snapSketchPoints->isChecked();
    s.activeLayer3DPoints = m_snap3DPoints->isChecked();
    s.activeLayer3DEdges = m_snap3DEdges->isChecked();
    s.showGuidePoints = m_showGuidePoints->isChecked();
    s.showSnappingHints = m_showHints->isChecked();
    return s;
}

} // namespace onecad::ui
