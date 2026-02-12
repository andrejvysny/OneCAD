#include "ConstraintPanel.h"

#include "../../core/sketch/Sketch.h"
#include "../../core/sketch/SketchConstraint.h"
#include "../theme/ThemeManager.h"

#include <QHBoxLayout>
#include <QLabel>
#include <QListWidget>
#include <QListWidgetItem>
#include <QPushButton>
#include <QVBoxLayout>

#include <algorithm>
#include <unordered_set>

namespace onecad::ui {

namespace {
QString formatSelectionCount(int count) {
    if (count == 1) {
        return QObject::tr("1 selected entity");
    }
    return QObject::tr("%1 selected entities").arg(count);
}
} // namespace

ConstraintPanel::ConstraintPanel(QWidget* parent)
    : QWidget(parent) {
    setupUi();
    m_themeConnection = connect(&ThemeManager::instance(), &ThemeManager::themeChanged,
                                this, &ConstraintPanel::updateTheme, Qt::UniqueConnection);
    updateTheme();
}

void ConstraintPanel::setupUi() {
    setObjectName("ConstraintListPanel");
    setWindowFlag(Qt::FramelessWindowHint, true);
    setAttribute(Qt::WA_StyledBackground, true);
    setFixedWidth(220);
    setMaximumHeight(340);

    setStyleSheet(R"(
        QLabel#title {
            font-weight: bold;
            font-size: 11px;
            padding: 4px 8px 0 8px;
            color: palette(text);
        }
        QLabel#subtitle {
            font-size: 10px;
            padding: 0 8px 6px 8px;
            color: palette(disabled, text);
        }
        QLabel#hint {
            font-size: 10px;
            padding: 6px 8px;
            color: palette(disabled, text);
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
        QPushButton[constraintAction="true"] {
            text-align: center;
            padding: 4px 6px;
            border: 1px solid palette(mid);
            border-radius: 6px;
            font-size: 10px;
        }
        QPushButton[constraintAction="true"]:hover {
            background-color: palette(midlight);
        }
        QPushButton[constraintAction="true"]:disabled {
            color: palette(disabled, text);
        }
    )");

    m_layout = new QVBoxLayout(this);
    m_layout->setContentsMargins(8, 8, 8, 8);
    m_layout->setSpacing(4);

    m_titleLabel = new QLabel(tr("Sketch constraints"), this);
    m_titleLabel->setObjectName("title");
    m_layout->addWidget(m_titleLabel);

    m_subtitleLabel = new QLabel(this);
    m_subtitleLabel->setObjectName("subtitle");
    m_layout->addWidget(m_subtitleLabel);

    m_listWidget = new QListWidget(this);
    m_listWidget->setSelectionMode(QAbstractItemView::SingleSelection);
    m_layout->addWidget(m_listWidget);

    m_hintLabel = new QLabel(this);
    m_hintLabel->setObjectName("hint");
    m_hintLabel->setAlignment(Qt::AlignLeft | Qt::AlignTop);
    m_hintLabel->setWordWrap(true);
    m_layout->addWidget(m_hintLabel);

    auto* actionRow = new QWidget(this);
    auto* actionLayout = new QHBoxLayout(actionRow);
    actionLayout->setContentsMargins(8, 0, 8, 0);
    actionLayout->setSpacing(6);

    m_inspectButton = new QPushButton(tr("Inspect"), actionRow);
    m_deleteButton = new QPushButton(tr("Delete"), actionRow);
    m_suppressButton = new QPushButton(tr("Suppress"), actionRow);
    m_inspectButton->setProperty("constraintAction", true);
    m_deleteButton->setProperty("constraintAction", true);
    m_suppressButton->setProperty("constraintAction", true);
    actionLayout->addWidget(m_inspectButton);
    actionLayout->addWidget(m_deleteButton);
    actionLayout->addWidget(m_suppressButton);
    m_layout->addWidget(actionRow);

    m_restoreSuppressedButton = new QPushButton(this);
    m_restoreSuppressedButton->setProperty("constraintAction", true);
    m_layout->addWidget(m_restoreSuppressedButton);

    connect(m_listWidget, &QListWidget::itemClicked, this, [this](QListWidgetItem* item) {
        const QString id = item->data(Qt::UserRole).toString();
        if (!id.isEmpty()) {
            emit constraintSelected(id);
        }
    });
    connect(m_inspectButton, &QPushButton::clicked, this, [this]() {
        if (!m_selectedConstraintId.isEmpty()) {
            emit constraintSelected(m_selectedConstraintId);
        }
    });
    connect(m_deleteButton, &QPushButton::clicked, this, [this]() {
        if (!m_selectedConstraintId.isEmpty()) {
            emit constraintDeleteRequested(m_selectedConstraintId);
        }
    });
    connect(m_suppressButton, &QPushButton::clicked, this, [this]() {
        if (!m_selectedConstraintId.isEmpty()) {
            emit constraintSuppressRequested(m_selectedConstraintId);
        }
    });
    connect(m_restoreSuppressedButton, &QPushButton::clicked, this, [this]() {
        emit restoreSuppressedRequested();
    });
}

void ConstraintPanel::setSketch(core::sketch::Sketch* sketch) {
    m_sketch = sketch;
    refresh();
}

void ConstraintPanel::setContext(const std::vector<std::string>& selectedEntityIds,
                                 const QString& selectedConstraintId,
                                 int dof,
                                 int conflictCount,
                                 int suppressedCount) {
    m_selectedEntityIds = selectedEntityIds;
    m_selectedConstraintId = selectedConstraintId;
    m_currentDof = dof;
    m_conflictCount = conflictCount;
    m_suppressedCount = suppressedCount;
    refresh();
}

void ConstraintPanel::refresh() {
    populateContext();
}

void ConstraintPanel::populateContext() {
    if (!m_sketch) {
        m_titleLabel->setText(tr("Sketch constraints"));
        m_subtitleLabel->setText(tr("No active sketch"));
        QPalette subtitlePalette = m_subtitleLabel->palette();
        subtitlePalette.setColor(QPalette::WindowText,
                                 palette().color(QPalette::Disabled, QPalette::Text));
        m_subtitleLabel->setPalette(subtitlePalette);
        m_hintLabel->setText(tr("Enter sketch mode to inspect constraints."));
        m_listWidget->setVisible(false);
        m_inspectButton->parentWidget()->setVisible(false);
        m_restoreSuppressedButton->setVisible(false);
        setFixedHeight(130);
        return;
    }

    if (!m_selectedConstraintId.isEmpty()) {
        populateConstraintDetails();
        return;
    }

    if (!m_selectedEntityIds.empty()) {
        populateConstraintSelection();
        return;
    }

    populateSummaryState();
}

void ConstraintPanel::populateConstraintSelection() {
    m_titleLabel->setText(tr("Constraints on selection"));
    m_subtitleLabel->setText(formatSelectionCount(static_cast<int>(m_selectedEntityIds.size())));
    QPalette subtitlePalette = m_subtitleLabel->palette();
    subtitlePalette.setColor(QPalette::WindowText,
                             palette().color(QPalette::Disabled, QPalette::Text));
    m_subtitleLabel->setPalette(subtitlePalette);
    m_listWidget->clear();

    std::vector<core::sketch::SketchConstraint*> selectedConstraints;
    selectedConstraints.reserve(m_selectedEntityIds.size() * 2);
    std::unordered_set<std::string> seen;
    for (const auto& entityId : m_selectedEntityIds) {
        auto constraints = m_sketch->getConstraintsForEntity(entityId);
        for (auto* constraint : constraints) {
            if (!constraint) {
                continue;
            }
            if (seen.insert(constraint->id()).second) {
                selectedConstraints.push_back(constraint);
            }
        }
    }

    std::sort(selectedConstraints.begin(), selectedConstraints.end(),
              [](const core::sketch::SketchConstraint* lhs,
                 const core::sketch::SketchConstraint* rhs) {
                  return lhs->id() < rhs->id();
              });

    for (const auto* constraint : selectedConstraints) {
        const QString icon = getConstraintIcon(static_cast<int>(constraint->type()));
        const QString typeName = getConstraintTypeName(static_cast<int>(constraint->type()));
        const QString displayText = QString("%1 %2").arg(icon, typeName);

        auto* item = new QListWidgetItem(displayText, m_listWidget);
        item->setData(Qt::UserRole, QString::fromStdString(constraint->id()));

        const bool satisfied =
            constraint->isSatisfied(*m_sketch, core::sketch::constants::SOLVER_TOLERANCE);
        if (!satisfied) {
            item->setForeground(m_unsatisfiedColor);
        }
    }

    const bool hasConstraints = !selectedConstraints.empty();
    m_listWidget->setVisible(hasConstraints);
    m_hintLabel->setVisible(true);
    m_hintLabel->setText(hasConstraints
                             ? tr("Click a constraint to inspect and manage it.")
                             : tr("No constraints on current selection."));
    m_inspectButton->parentWidget()->setVisible(false);

    if (m_suppressedCount > 0) {
        m_restoreSuppressedButton->setVisible(true);
        m_restoreSuppressedButton->setText(tr("Show hidden markers (%1)").arg(m_suppressedCount));
    } else {
        m_restoreSuppressedButton->setVisible(false);
    }

    const int contentHeight = hasConstraints
        ? qMin(310, 120 + static_cast<int>(selectedConstraints.size()) * 26)
        : 150;
    setFixedHeight(contentHeight);
}

void ConstraintPanel::populateConstraintDetails() {
    auto* constraint = m_sketch->getConstraint(m_selectedConstraintId.toStdString());
    if (!constraint) {
        populateSummaryState();
        return;
    }

    const QString icon = getConstraintIcon(static_cast<int>(constraint->type()));
    const QString typeName = getConstraintTypeName(static_cast<int>(constraint->type()));
    const bool satisfied =
        constraint->isSatisfied(*m_sketch, core::sketch::constants::SOLVER_TOLERANCE);

    m_titleLabel->setText(tr("Constraint details"));
    m_subtitleLabel->setText(QString("%1 %2 • %3")
                                 .arg(icon, typeName, satisfied ? tr("Satisfied")
                                                                : tr("Unsatisfied")));
    QPalette subtitlePalette = m_subtitleLabel->palette();
    subtitlePalette.setColor(QPalette::WindowText, satisfied ? palette().color(QPalette::Disabled,
                                                                                QPalette::Text)
                                                             : m_unsatisfiedColor);
    m_subtitleLabel->setPalette(subtitlePalette);

    const auto referenced = constraint->referencedEntities();
    QStringList ids;
    ids.reserve(static_cast<int>(referenced.size()));
    for (const auto& id : referenced) {
        ids << QString::fromStdString(id);
    }
    m_hintLabel->setVisible(true);
    m_hintLabel->setText(tr("References: %1").arg(ids.join(", ")));

    m_listWidget->setVisible(false);
    m_inspectButton->parentWidget()->setVisible(true);
    m_inspectButton->setEnabled(true);
    m_deleteButton->setEnabled(true);
    m_suppressButton->setEnabled(true);

    if (m_suppressedCount > 0) {
        m_restoreSuppressedButton->setVisible(true);
        m_restoreSuppressedButton->setText(tr("Show hidden markers (%1)").arg(m_suppressedCount));
    } else {
        m_restoreSuppressedButton->setVisible(false);
    }

    setFixedHeight(190);
}

void ConstraintPanel::populateSummaryState() {
    m_titleLabel->setText(tr("Sketch constraints"));
    m_subtitleLabel->setText(tr("DOF: %1 • Conflicts: %2")
                                 .arg(m_currentDof)
                                 .arg(m_conflictCount));
    m_hintLabel->setText(tr("Select geometry to constrain."));

    QPalette subtitlePalette = m_subtitleLabel->palette();
    subtitlePalette.setColor(QPalette::WindowText,
                             m_conflictCount > 0 ? m_conflictColor
                                                 : palette().color(QPalette::Disabled,
                                                                   QPalette::Text));
    m_subtitleLabel->setPalette(subtitlePalette);

    m_listWidget->setVisible(false);
    m_hintLabel->setVisible(true);
    m_inspectButton->parentWidget()->setVisible(false);
    if (m_suppressedCount > 0) {
        m_restoreSuppressedButton->setVisible(true);
        m_restoreSuppressedButton->setText(tr("Show hidden markers (%1)").arg(m_suppressedCount));
    } else {
        m_restoreSuppressedButton->setVisible(false);
    }

    setFixedHeight(m_suppressedCount > 0 ? 150 : 130);
}

void ConstraintPanel::updateTheme() {
    const auto& theme = ThemeManager::instance().currentTheme();
    m_unsatisfiedColor = theme.constraints.unsatisfiedText;
    m_conflictColor = theme.viewport.selection.edgeHover;
    populateContext();
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
        case CT::Midpoint:          return QString::fromUtf8("\u22C2");
        case CT::Fixed:             return QString::fromUtf8("\xF0\x9F\x94\x92");
        case CT::Distance:          return QString::fromUtf8("\u2194");
        case CT::HorizontalDistance:return QString::fromUtf8("\u2194");
        case CT::VerticalDistance:  return QString::fromUtf8("\u2195");
        case CT::Angle:             return QString::fromUtf8("\u2220");
        case CT::Radius:            return QString::fromUtf8("R");
        case CT::Diameter:          return QString::fromUtf8("\u2300");
        case CT::Concentric:        return QString::fromUtf8("\u25CE");
        case CT::Symmetric:         return QString::fromUtf8("\u2016");
        case CT::OnCurve:           return QString::fromUtf8("\u2229");
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
