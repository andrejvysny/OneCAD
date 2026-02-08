#include "SketchModePanel.h"
#include "../../core/sketch/Sketch.h"
#include "../../core/sketch/SketchTypes.h"

#include <QVBoxLayout>
#include <QHBoxLayout>
#include <QLabel>
#include <QPushButton>
#include <QFrame>

namespace onecad::ui {

SketchModePanel::SketchModePanel(QWidget* parent)
    : QWidget(parent)
{
    setupUi();
}

void SketchModePanel::setupUi() {
    setObjectName("ConstraintToolsPanel");
    setWindowFlag(Qt::FramelessWindowHint, true);
    setAttribute(Qt::WA_StyledBackground, true);
    setFixedWidth(180);

    // Container styling (background, border, radius) from ThemeManager#ConstraintToolsPanel
    setStyleSheet(R"(
        QLabel#title {
            font-weight: bold;
            font-size: 11px;
            padding: 8px;
            color: palette(text);
        }
        QLabel#section {
            font-size: 10px;
            padding: 4px 8px;
            color: palette(disabled, text);
        }
        QPushButton {
            text-align: left;
            padding: 6px 8px;
            border: none;
            border-radius: 3px;
            font-size: 11px;
        }
        QPushButton:hover {
            background-color: palette(midlight);
        }
        QPushButton:pressed {
            background-color: palette(highlight);
            color: palette(highlighted-text);
        }
        QPushButton:disabled {
            color: palette(disabled, text);
        }
        QFrame#separator {
            background-color: palette(mid);
            max-height: 1px;
            margin: 4px 8px;
        }
    )");

    m_layout = new QVBoxLayout(this);
    m_layout->setContentsMargins(8, 8, 8, 8);
    m_layout->setSpacing(2);

    // Title
    m_titleLabel = new QLabel(tr("Constraint tools"), this);
    m_titleLabel->setObjectName("title");
    m_layout->addWidget(m_titleLabel);

    // Geometric constraints section
    QLabel* geoLabel = new QLabel(tr("Geometric"), this);
    geoLabel->setObjectName("section");
    m_layout->addWidget(geoLabel);

    using CT = core::sketch::ConstraintType;

    // Define constraint buttons
    std::vector<std::tuple<CT, QString, QString, QString>> geometricConstraints = {
        {CT::Horizontal, QString::fromUtf8("\u22A3"), tr("Horizontal"), "H"},
        {CT::Vertical, QString::fromUtf8("\u22A4"), tr("Vertical"), "V"},
        {CT::Parallel, QString::fromUtf8("\u2225"), tr("Parallel"), "P"},
        {CT::Perpendicular, QString::fromUtf8("\u22A5"), tr("Perpendicular"), "N"},
        {CT::Tangent, QString::fromUtf8("\u25CB"), tr("Tangent"), "T"},
        {CT::Coincident, QString::fromUtf8("\u25CF"), tr("Coincident"), "C"},
        {CT::Equal, QString::fromUtf8("="), tr("Equal"), "E"},
        {CT::Midpoint, QString::fromUtf8("\u22C2"), tr("Midpoint"), "M"},
        {CT::Concentric, QString::fromUtf8("\u25CE"), tr("Concentric"), "O"},
        {CT::OnCurve, QString::fromUtf8("\u25CB\u2192"), tr("Point On Curve"), ""},
    };

    for (const auto& [type, icon, name, shortcut] : geometricConstraints) {
        QPushButton* btn = createButton(icon, name, shortcut,
            tr("Apply %1 constraint [%2]").arg(name, shortcut));
        m_constraintButtons.push_back({type, icon, name, shortcut, btn});
        m_layout->addWidget(btn);

        connect(btn, &QPushButton::clicked, this, [this, type]() {
            emit constraintRequested(type);
        });
    }

    // Separator
    QFrame* sep1 = new QFrame(this);
    sep1->setObjectName("separator");
    sep1->setFrameShape(QFrame::HLine);
    m_layout->addWidget(sep1);

    // Dimensional constraints section
    QLabel* dimLabel = new QLabel(tr("Dimensional"), this);
    dimLabel->setObjectName("section");
    m_layout->addWidget(dimLabel);

    std::vector<std::tuple<CT, QString, QString, QString>> dimensionalConstraints = {
        {CT::Distance, QString::fromUtf8("\u2194"), tr("Distance"), "D"},
        {CT::Angle, QString::fromUtf8("\u2220"), tr("Angle"), "A"},
        {CT::Radius, QString::fromUtf8("R"), tr("Radius"), "R"},
        {CT::Diameter, QString::fromUtf8("\u2300"), tr("Diameter"), ""},
    };

    for (const auto& [type, icon, name, shortcut] : dimensionalConstraints) {
        QString tooltip = shortcut.isEmpty()
            ? tr("Apply %1 constraint").arg(name)
            : tr("Apply %1 constraint [%2]").arg(name, shortcut);
        QPushButton* btn = createButton(icon, name, shortcut, tooltip);
        m_constraintButtons.push_back({type, icon, name, shortcut, btn});
        m_layout->addWidget(btn);

        connect(btn, &QPushButton::clicked, this, [this, type]() {
            emit constraintRequested(type);
        });
    }

    // Separator
    QFrame* sep2 = new QFrame(this);
    sep2->setObjectName("separator");
    sep2->setFrameShape(QFrame::HLine);
    m_layout->addWidget(sep2);

    // Fixed constraint
    QPushButton* fixBtn = createButton(
        QString::fromUtf8("\xF0\x9F\x94\x92"), // Lock emoji
        tr("Lock/Fix"), "F",
        tr("Fix selected point [F]"));
    m_constraintButtons.push_back({CT::Fixed, QString::fromUtf8("\xF0\x9F\x94\x92"),
                                   tr("Lock/Fix"), "F", fixBtn});
    m_layout->addWidget(fixBtn);

    connect(fixBtn, &QPushButton::clicked, this, [this]() {
        emit constraintRequested(CT::Fixed);
    });

    m_layout->addStretch();
}

QPushButton* SketchModePanel::createButton(const QString& icon, const QString& name,
                                            const QString& shortcut, const QString& tooltip) {
    QString text = QString("%1  %2").arg(icon, name);
    if (!shortcut.isEmpty()) {
        text += QString("  [%1]").arg(shortcut);
    }

    QPushButton* btn = new QPushButton(text, this);
    btn->setToolTip(tooltip);
    btn->setCursor(Qt::PointingHandCursor);
    return btn;
}

void SketchModePanel::setSketch(core::sketch::Sketch* sketch) {
    m_sketch = sketch;
    updateButtonStates();
}

void SketchModePanel::updateButtonStates() {
    // For now, enable all buttons
    // In future, disable based on selection validity
    for (auto& cb : m_constraintButtons) {
        if (cb.button) {
            cb.button->setEnabled(m_sketch != nullptr);
        }
    }
}

} // namespace onecad::ui
