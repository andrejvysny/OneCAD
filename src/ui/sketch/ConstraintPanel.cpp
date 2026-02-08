#include "ConstraintPanel.h"
#include "../../core/sketch/Sketch.h"
#include "../../core/sketch/SketchConstraint.h"
#include "../theme/ThemeManager.h"

#include <QVBoxLayout>
#include <QLabel>
#include <QListWidget>
#include <QListWidgetItem>

namespace onecad::ui {

ConstraintPanel::ConstraintPanel(QWidget* parent)
    : QWidget(parent)
{
    setupUi();
    m_themeConnection = connect(&ThemeManager::instance(), &ThemeManager::themeChanged,
                                this, &ConstraintPanel::updateTheme, Qt::UniqueConnection);
    updateTheme();
}

void ConstraintPanel::setupUi() {
    setObjectName("ConstraintListPanel");
    setWindowFlag(Qt::FramelessWindowHint, true);
    setAttribute(Qt::WA_StyledBackground, true);
    setFixedWidth(200);
    setMaximumHeight(300);

    // Container styling (background, border, radius) from ThemeManager#ConstraintListPanel
    setStyleSheet(R"(
        QLabel#title {
            font-weight: bold;
            font-size: 11px;
            padding: 8px;
            color: palette(text);
        }
        QListWidget {
            border: none;
            background: transparent;
            font-size: 11px;
        }
        QListWidget::item {
            padding: 4px 8px;
        }
        QListWidget::item:hover {
            background-color: palette(midlight);
        }
        QListWidget::item:selected {
            background-color: palette(highlight);
            color: palette(highlighted-text);
        }
    )");

    m_layout = new QVBoxLayout(this);
    m_layout->setContentsMargins(8, 8, 8, 8);
    m_layout->setSpacing(0);

    // Title
    m_titleLabel = new QLabel(tr("Applied constraints"), this);
    m_titleLabel->setObjectName("title");
    m_layout->addWidget(m_titleLabel);

    // List widget
    m_listWidget = new QListWidget(this);
    m_listWidget->setSelectionMode(QAbstractItemView::SingleSelection);
    m_layout->addWidget(m_listWidget);

    // Empty state label
    m_emptyLabel = new QLabel(tr("No constraints"), this);
    m_emptyLabel->setAlignment(Qt::AlignCenter);
    m_emptyLabel->setStyleSheet("color: palette(disabled, text); padding: 20px;");
    m_layout->addWidget(m_emptyLabel);

    connect(m_listWidget, &QListWidget::itemClicked, this, [this](QListWidgetItem* item) {
        QString id = item->data(Qt::UserRole).toString();
        if (!id.isEmpty()) {
            emit constraintSelected(id);
        }
    });
}

void ConstraintPanel::setSketch(core::sketch::Sketch* sketch) {
    m_sketch = sketch;
    refresh();
}

void ConstraintPanel::refresh() {
    populateList();
}

void ConstraintPanel::populateList() {
    m_listWidget->clear();

    if (!m_sketch) {
        m_listWidget->setVisible(false);
        m_emptyLabel->setVisible(true);
        setFixedHeight(120);
        return;
    }

    const auto& constraints = m_sketch->getAllConstraints();

    if (constraints.empty()) {
        m_listWidget->setVisible(false);
        m_emptyLabel->setVisible(true);
        setFixedHeight(120);
        return;
    }

    m_listWidget->setVisible(true);
    m_emptyLabel->setVisible(false);

    for (const auto& constraint : constraints) {
        QString icon = getConstraintIcon(static_cast<int>(constraint->type()));
        QString typeName = getConstraintTypeName(static_cast<int>(constraint->type()));
        QString displayText = QString("%1 %2").arg(icon, typeName);

        auto* item = new QListWidgetItem(displayText, m_listWidget);
        item->setData(Qt::UserRole, QString::fromStdString(constraint->id()));

        // Color based on satisfaction
        bool satisfied = constraint->isSatisfied(*m_sketch, core::sketch::constants::SOLVER_TOLERANCE);
        if (!satisfied) {
            item->setForeground(m_unsatisfiedColor);
        }
    }

    // Update panel height based on content
    int contentHeight = 40 + constraints.size() * 28; // title + items
    setFixedHeight(qMin(contentHeight, 300));
}

void ConstraintPanel::updateTheme() {
    m_unsatisfiedColor = ThemeManager::instance().currentTheme().constraints.unsatisfiedText;
    populateList();
}

QString ConstraintPanel::getConstraintIcon(int type) const {
    using CT = core::sketch::ConstraintType;
    switch (static_cast<CT>(type)) {
        case CT::Horizontal:        return QString::fromUtf8("\u22A3"); // ⊣
        case CT::Vertical:          return QString::fromUtf8("\u22A4"); // ⊤
        case CT::Parallel:          return QString::fromUtf8("\u2225"); // ∥
        case CT::Perpendicular:     return QString::fromUtf8("\u22A5"); // ⊥
        case CT::Tangent:           return QString::fromUtf8("\u25CB"); // ○
        case CT::Coincident:        return QString::fromUtf8("\u25CF"); // ●
        case CT::Equal:             return QString::fromUtf8("=");
        case CT::Midpoint:          return QString::fromUtf8("\u22C2"); // ⟂
        case CT::Fixed:             return QString::fromUtf8("\xF0\x9F\x94\x92"); // Lock emoji (U+1F512)
        case CT::Distance:          return QString::fromUtf8("\u2194"); // ↔
        case CT::HorizontalDistance:return QString::fromUtf8("\u2194"); // ↔
        case CT::VerticalDistance:  return QString::fromUtf8("\u2195"); // ↕
        case CT::Angle:             return QString::fromUtf8("\u2220"); // ∠
        case CT::Radius:            return QString::fromUtf8("R");
        case CT::Diameter:          return QString::fromUtf8("\u2300"); // ⌀
        case CT::Concentric:        return QString::fromUtf8("\u25CE"); // ◎
        case CT::Symmetric:         return QString::fromUtf8("\u2016"); // ‖
        case CT::OnCurve:           return QString::fromUtf8("\u2229"); // ∩
        default:                    return QString::fromUtf8("?");
    }
}

QString ConstraintPanel::getConstraintTypeName(int type) const {
    using CT = core::sketch::ConstraintType;
    switch (static_cast<CT>(type)) {
        case CT::Horizontal:        return tr("Horizontal");
        case CT::Vertical:          return tr("Vertical");
        case CT::Parallel:          return tr("Parallel");
        case CT::Perpendicular:     return tr("Perpendicular");
        case CT::Tangent:           return tr("Tangent");
        case CT::Coincident:        return tr("Coincident");
        case CT::Equal:             return tr("Equal");
        case CT::Midpoint:          return tr("Midpoint");
        case CT::Fixed:             return tr("Fixed");
        case CT::Distance:          return tr("Distance");
        case CT::HorizontalDistance:return tr("H-Distance");
        case CT::VerticalDistance:  return tr("V-Distance");
        case CT::Angle:             return tr("Angle");
        case CT::Radius:            return tr("Radius");
        case CT::Diameter:          return tr("Diameter");
        case CT::Concentric:        return tr("Concentric");
        case CT::Symmetric:         return tr("Symmetric");
        case CT::OnCurve:           return tr("On Curve");
        default:                    return tr("Unknown");
    }
}

} // namespace onecad::ui
