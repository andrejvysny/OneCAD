/**
 * @file HistoryPanel.cpp
 * @brief Implementation of feature history tree panel.
 */
#include "HistoryPanel.h"
#include "FeatureCard.h"
#include "EditParameterDialog.h"
#include "../../app/document/Document.h"
#include "../../app/document/OperationRecord.h"
#include "../../app/history/DependencyGraph.h"
#include "../viewport/Viewport.h"
#include "../theme/ThemeManager.h"

#include <QTreeWidget>
#include <QTreeWidgetItem>
#include <QVBoxLayout>
#include <QHBoxLayout>
#include <QFrame>
#include <QLabel>
#include <QToolButton>
#include <QMenu>
#include <QPropertyAnimation>
#include <QHeaderView>
#include <QFont>
#include <QSizePolicy>
#include <QEasingCurve>
#include <QPainter>

namespace onecad::ui {

HistoryPanel::HistoryPanel(QWidget* parent)
    : QWidget(parent) {
    setupUi();
    themeConnection_ = connect(&ThemeManager::instance(), &ThemeManager::themeChanged,
                               this, &HistoryPanel::updateTheme, Qt::UniqueConnection);
    updateTheme();
    applyCollapseState(false);
}

void HistoryPanel::setupUi() {
    setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Expanding);

    auto* mainLayout = new QVBoxLayout(this);
    mainLayout->setContentsMargins(0, 0, 0, 0);
    mainLayout->setSpacing(0);

    panel_ = new QFrame(this);
    panel_->setObjectName("historyPanel");
    panel_->setFrameShape(QFrame::NoFrame);

    auto* panelLayout = new QVBoxLayout(panel_);
    panelLayout->setContentsMargins(12, 12, 12, 12);
    panelLayout->setSpacing(10);

    panelLayout->addWidget(treeWidget_);

    mainLayout->addWidget(panel_);
    treeWidget_ = new QTreeWidget;
    treeWidget_->setObjectName("NavigatorTree");
    treeWidget_->setHeaderHidden(true);
    treeWidget_->setIndentation(12);
    treeWidget_->setRootIsDecorated(true);
    treeWidget_->setContextMenuPolicy(Qt::CustomContextMenu);
    treeWidget_->setUniformRowHeights(true);
    treeWidget_->setSelectionMode(QAbstractItemView::SingleSelection);
    treeWidget_->setSelectionBehavior(QAbstractItemView::SelectRows);
    treeWidget_->setFocusPolicy(Qt::NoFocus);
    treeWidget_->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);

    connect(treeWidget_, &QTreeWidget::itemClicked,
            this, &HistoryPanel::onItemClicked);
    connect(treeWidget_, &QTreeWidget::itemDoubleClicked,
            this, &HistoryPanel::onItemDoubleClicked);
    connect(treeWidget_, &QTreeWidget::customContextMenuRequested,
            this, &HistoryPanel::onCustomContextMenu);

    panelLayout->addWidget(treeWidget_);

    mainLayout->addWidget(panel_);

    setMinimumWidth(expandedWidth_);
    setMaximumWidth(expandedWidth_);
}

void HistoryPanel::updateTheme() {
    const auto& theme = ThemeManager::instance().currentTheme();

    panel_->setStyleSheet(QString("QFrame#historyPanel { background-color: %1; border-left: 1px solid %2; }")
        .arg(theme.ui.panelBackground.name(QColor::HexArgb))
        .arg(theme.ui.panelBorder.name(QColor::HexArgb)));

    treeWidget_->setStyleSheet(QString(R"(
        QTreeWidget {
            background-color: %1;
            border: none;
            color: %2;
        }
        QTreeWidget::item {
            height: 36px;
            padding: 0px;
        }
        QTreeWidget::item:selected {
            background-color: transparent; /* Widget handles selection */
        }
        QTreeWidget::item:hover:!selected {
            background-color: transparent;
        }
    )")
    .arg(theme.ui.treeBackground.name(QColor::HexArgb))
    .arg(theme.ui.treeText.name(QColor::HexArgb)));

    // Update Headers (items without widgets)
    QTreeWidgetItemIterator it(treeWidget_);
    while (*it) {
        if (!treeWidget_->itemWidget(*it, 0)) {
            (*it)->setForeground(0, theme.navigator.headerText);
        }
        ++it;
    }

    for (auto& entry : entries_) {
        if (entry.card) {
            entry.card->updateTheme();
        }
    }
}

QWidget* HistoryPanel::createSectionHeader(const QString& text) {
    auto* label = new QLabel(text);
    label->setProperty("nav-header", true);
    label->setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Fixed);
    
    // Style it immediately
    const auto& theme = ThemeManager::instance().currentTheme();
    label->setStyleSheet(QString(
        "QLabel[nav-header=\"true\"] { "
        "  color: %1; "
        "  font-weight: bold; "
        "  font-size: 11px; "
        "  text-transform: uppercase; "
        "  padding: 8px 4px; "
        "  background: transparent; "
        "}"
    ).arg(theme.navigator.headerText.name(QColor::HexArgb)));
    
    return label;
}

void HistoryPanel::setDocument(app::Document* doc) {
    document_ = doc;
    rebuild();
}

void HistoryPanel::setViewport(Viewport* viewport) {
    viewport_ = viewport;
}

void HistoryPanel::setCommandProcessor(app::commands::CommandProcessor* processor) {
    commandProcessor_ = processor;
}

void HistoryPanel::rebuild() {
    treeWidget_->clear();
    entries_.clear();

    if (!document_) {
        return;
    }

    const auto& ops = document_->operations();
    if (ops.empty()) {
        auto* placeholder = new QTreeWidgetItem(treeWidget_);
        placeholder->setText(0, "No operations");
        placeholder->setForeground(0, ThemeManager::instance().currentTheme().navigator.placeholderText);
        placeholder->setFlags(Qt::NoItemFlags);
        return;
    }

    // Build dependency graph for ordering
    app::history::DependencyGraph graph;
    graph.rebuildFromOperations(ops);

    auto sorted = graph.topologicalSort();
    if (sorted.empty() && !ops.empty()) {
        sorted.reserve(ops.size());
        for (const auto& op : ops) {
            sorted.push_back(op.opId);
        }
    }
    std::unordered_map<std::string, const app::OperationRecord*> opById;
    opById.reserve(ops.size());
    for (const auto& op : ops) {
        opById[op.opId] = &op;
    }

    std::unordered_map<std::string, QTreeWidgetItem*> sketchItems;
    std::unordered_map<std::string, QTreeWidgetItem*> opItems;
    std::unordered_map<std::string, std::string> bodyProducers;

    // Create items in topological order
    for (const auto& opId : sorted) {
        const app::OperationRecord* opRecord = nullptr;
        auto opIt = opById.find(opId);
        if (opIt != opById.end()) {
            opRecord = opIt->second;
        }
        if (!opRecord) {
            continue;
        }

        QTreeWidgetItem* parentItem = nullptr;
        
        // Group logic... (simplified for now to match old implementation structure)
        // Note: The original implementation had grouping logic which we preserve
        if (std::holds_alternative<app::SketchRegionRef>(opRecord->input)) {
            const auto& ref = std::get<app::SketchRegionRef>(opRecord->input);
            auto sketchIt = sketchItems.find(ref.sketchId);
            if (sketchIt == sketchItems.end()) {
                auto* sketchItem = new QTreeWidgetItem(treeWidget_);
                sketchItem->setFlags(Qt::ItemIsEnabled);
                
                // Use a widget for the header to match Navigator styling
                QString sketchName = QString::fromStdString(document_->getSketchName(ref.sketchId));
                QWidget* headerWidget = createSectionHeader(sketchName);
                treeWidget_->setItemWidget(sketchItem, 0, headerWidget);
                
                sketchItems[ref.sketchId] = sketchItem;
                parentItem = sketchItem;
            } else {
                parentItem = sketchIt->second;
            }
        } 
        // ... rest of parenting logic preserved implicitly by iterating sorted list ...
        // For simplicity and robustness, I'll just append to root if parent logic gets complex,
        // but let's try to maintain the existing logic if possible.
        // Actually, the previous implementation had logic to find parent item based on input.
        // We'll keep it simple for this refactor and just list them, or use the same logic if we want nesting.
        // Let's stick to flat list or minimal nesting for now to ensure robustness unless strict hierarchy is required.
        // The previous code had nesting logic. Let's keep it.
        else if (std::holds_alternative<app::FaceRef>(opRecord->input)) {
            const auto& ref = std::get<app::FaceRef>(opRecord->input);
            auto producerIt = bodyProducers.find(ref.bodyId);
            if (producerIt != bodyProducers.end()) {
                auto opItemIt = opItems.find(producerIt->second);
                if (opItemIt != opItems.end()) {
                    parentItem = opItemIt->second;
                }
            }
        } else if (std::holds_alternative<app::BodyRef>(opRecord->input)) {
            const auto& ref = std::get<app::BodyRef>(opRecord->input);
            auto producerIt = bodyProducers.find(ref.bodyId);
            if (producerIt != bodyProducers.end()) {
                auto opItemIt = opItems.find(producerIt->second);
                if (opItemIt != opItems.end()) {
                    parentItem = opItemIt->second;
                }
            }
        }
        
        // Boolean check
        if (opRecord->type == app::OperationType::Boolean &&
            std::holds_alternative<app::BooleanParams>(opRecord->params)) {
            const auto& params = std::get<app::BooleanParams>(opRecord->params);
            auto producerIt = bodyProducers.find(params.targetBodyId);
            if (producerIt != bodyProducers.end()) {
                auto opItemIt = opItems.find(producerIt->second);
                if (opItemIt != opItems.end()) {
                    parentItem = opItemIt->second;
                }
            }
        }

        ItemEntry entry;
        entry.opId = opId;
        entry.type = opRecord->type;
        entry.item = parentItem ? new QTreeWidgetItem(parentItem) : new QTreeWidgetItem(treeWidget_);
        entry.failed = document_->isOperationFailed(opId);
        entry.suppressed = document_->isOperationSuppressed(opId);
        entry.dirty = isDirty(opId);
        if (entry.failed) {
            entry.failureReason = document_->operationFailureReason(opId);
        }

        entry.card = createItemWidget(entry);
        treeWidget_->setItemWidget(entry.item, 0, entry.card);

        opItems[opId] = entry.item;
        for (const auto& bodyId : opRecord->resultBodyIds) {
            bodyProducers[bodyId] = opId;
        }

        entries_.push_back(std::move(entry));
    }
    
    // Expand all
    treeWidget_->expandAll();
}

FeatureCard* HistoryPanel::createItemWidget(ItemEntry& entry) {
    auto* card = new FeatureCard;
    
    // Retrieve operation record to get details
    const app::OperationRecord* opRecord = nullptr;
    for (const auto& op : document_->operations()) {
        if (op.opId == entry.opId) {
            opRecord = &op;
            break;
        }
    }

    if (opRecord) {
        QString displayName = getOperationName(entry.type);
        if (document_) {
            const auto metadata = document_->operationMetadata(entry.opId);
            if (metadata.has_value() && !metadata->uiAlias.isEmpty()) {
                displayName = metadata->uiAlias;
            }
        }
        card->setName(displayName);
        QString details = getOperationDetails(*opRecord);
        if (entry.dirty) {
            details = details.isEmpty()
                ? tr("Pending")
                : QString("%1 • %2").arg(details, tr("Pending"));
            card->setToolTip(tr("Pending regeneration: this step and later steps are not applied yet."));
        } else {
            card->setToolTip(QString());
        }
        card->setDetails(details);
    } else {
        card->setName("Unknown");
    }

    card->setIconPath(operationIconPath(entry.type));
    card->setFailed(entry.failed, QString::fromStdString(entry.failureReason));
    card->setSuppressed(entry.suppressed);
    
    // Connect signals from card to panel actions
    connect(card, &FeatureCard::menuRequested, this, [this, &entry]() {
        showContextMenu(QCursor::pos(), entry.item);
    });
    
    connect(card, &FeatureCard::suppressToggled, this, [this, &entry]() {
        emit suppressRequested(QString::fromStdString(entry.opId), !entry.suppressed);
    });

    updateItemState(entry);
    return card;
}

void HistoryPanel::updateItemState(ItemEntry& entry) {
    if (entry.card) {
        entry.card->setFailed(entry.failed, QString::fromStdString(entry.failureReason));
        entry.card->setSuppressed(entry.suppressed);
        entry.card->setSelected(entry.item->isSelected());
    }
}

QString HistoryPanel::getOperationName(app::OperationType type) const {
    switch (type) {
        case app::OperationType::Extrude: return "Extrude";
        case app::OperationType::Revolve: return "Revolve";
        case app::OperationType::Fillet: return "Fillet";
        case app::OperationType::Chamfer: return "Chamfer";
        case app::OperationType::Shell: return "Shell";
        case app::OperationType::Boolean: return "Boolean";
        default: return "Operation";
    }
}

QString HistoryPanel::getOperationDetails(const app::OperationRecord& op) const {
    QString params;

    switch (op.type) {
        case app::OperationType::Extrude:
            if (std::holds_alternative<app::ExtrudeParams>(op.params)) {
                const auto& p = std::get<app::ExtrudeParams>(op.params);
                params = QString("%1mm").arg(p.distance, 0, 'f', 1);
            }
            break;
        case app::OperationType::Revolve:
            if (std::holds_alternative<app::RevolveParams>(op.params)) {
                const auto& p = std::get<app::RevolveParams>(op.params);
                params = QString("%1°").arg(p.angleDeg, 0, 'f', 0);
            }
            break;
        case app::OperationType::Fillet:
            if (std::holds_alternative<app::FilletChamferParams>(op.params)) {
                const auto& p = std::get<app::FilletChamferParams>(op.params);
                params = QString("R%1").arg(p.radius, 0, 'f', 1);
            }
            break;
        case app::OperationType::Chamfer:
            if (std::holds_alternative<app::FilletChamferParams>(op.params)) {
                const auto& p = std::get<app::FilletChamferParams>(op.params);
                params = QString("%1mm").arg(p.radius, 0, 'f', 1);
            }
            break;
        case app::OperationType::Shell:
            if (std::holds_alternative<app::ShellParams>(op.params)) {
                const auto& p = std::get<app::ShellParams>(op.params);
                params = QString("%1mm").arg(p.thickness, 0, 'f', 1);
            }
            break;
        case app::OperationType::Boolean:
            if (std::holds_alternative<app::BooleanParams>(op.params)) {
                const auto& p = std::get<app::BooleanParams>(op.params);
                switch (p.operation) {
                    case app::BooleanParams::Op::Union: params = "Union"; break;
                    case app::BooleanParams::Op::Cut: params = "Cut"; break;
                    case app::BooleanParams::Op::Intersect: params = "Intersect"; break;
                }
            }
            break;
    }

    return params;
}

QString HistoryPanel::operationIconPath(app::OperationType type) const {
    switch (type) {
        case app::OperationType::Extrude: return ":/icons/ic_extrude.svg";
        case app::OperationType::Revolve: return ":/icons/ic_revolve.svg";
        case app::OperationType::Fillet: return ":/icons/ic_fillet.svg";
        case app::OperationType::Chamfer: return ":/icons/ic_chamfer.svg";
        case app::OperationType::Shell: return ":/icons/ic_shell.svg";
        case app::OperationType::Boolean: return ":/icons/ic_boolean_union.svg"; // Default generic boolean
        default: return ":/icons/ic_settings.svg";
    }
}

bool HistoryPanel::isEditableType(app::OperationType type) const {
    return type == app::OperationType::Extrude ||
           type == app::OperationType::Revolve;
}

bool HistoryPanel::isReplayOnly(const std::string& opId) const {
    if (!document_) {
        return false;
    }
    const auto metadata = document_->operationMetadata(opId);
    return metadata.has_value() && metadata->replayOnly;
}

bool HistoryPanel::isDirty(const std::string& opId) const {
    if (!document_) {
        return false;
    }
    const int index = document_->operationIndex(opId);
    if (index < 0) {
        return false;
    }
    return static_cast<std::size_t>(index) >= document_->appliedOpCount();
}

void HistoryPanel::onItemClicked(QTreeWidgetItem* item, int) {
    // Update selection state for all cards
    for (auto& entry : entries_) {
        if (entry.card && entry.item) {
            entry.card->setSelected(entry.item->isSelected());
        }
    }

    auto* entry = entryForItem(item);
    if (entry) {
        emit operationSelected(QString::fromStdString(entry->opId));
    }
}

void HistoryPanel::onItemDoubleClicked(QTreeWidgetItem* item, int) {
    auto* entry = entryForItem(item);
    if (!entry || !document_) return;

    for (const auto& op : document_->operations()) {
        if (op.opId == entry->opId) {
            if (isEditableType(op.type) && !isReplayOnly(entry->opId) && !isDirty(entry->opId)) {
                showEditDialog(entry->opId);
            }
            break;
        }
    }
}

void HistoryPanel::showEditDialog(const std::string& opId) {
    if (!document_ || !viewport_) return;

    EditParameterDialog dialog(document_, viewport_, commandProcessor_, opId, this);
    if (dialog.exec() == QDialog::Accepted) {
        rebuild();
        viewport_->update();
    }
}

void HistoryPanel::onCustomContextMenu(const QPoint& pos) {
    QTreeWidgetItem* item = treeWidget_->itemAt(pos);
    if (item) {
        showContextMenu(treeWidget_->viewport()->mapToGlobal(pos), item);
    }
}

void HistoryPanel::showContextMenu(const QPoint& pos, QTreeWidgetItem* item) {
    auto* entry = entryForItem(item);
    if (!entry || !document_) return;

    // Find the operation
    const app::OperationRecord* opRecord = nullptr;
    for (const auto& op : document_->operations()) {
        if (op.opId == entry->opId) {
            opRecord = &op;
            break;
        }
    }
    if (!opRecord) return;

    QMenu menu;

    if (isEditableType(opRecord->type) && !isReplayOnly(entry->opId) && !isDirty(entry->opId)) {
        QAction* editAction = menu.addAction(tr("Edit Parameters..."));
        connect(editAction, &QAction::triggered, this, [this, entry]() {
            showEditDialog(entry->opId);
        });
    } else if (isDirty(entry->opId)) {
        QAction* pendingAction = menu.addAction(tr("Edit Parameters (Pending Regeneration)"));
        pendingAction->setEnabled(false);
    }

    menu.addSeparator();

    QAction* rollbackAction = menu.addAction(tr("Rollback to Here"));
    connect(rollbackAction, &QAction::triggered, this, [this, entry]() {
        emit rollbackRequested(QString::fromStdString(entry->opId));
    });

    QString suppressText = entry->suppressed ? tr("Unsuppress") : tr("Suppress");
    QAction* suppressAction = menu.addAction(suppressText);
    connect(suppressAction, &QAction::triggered, this, [this, entry]() {
        emit suppressRequested(QString::fromStdString(entry->opId), !entry->suppressed);
    });

    menu.addSeparator();

    QAction* deleteAction = menu.addAction(tr("Delete"));
    deleteAction->setShortcut(QKeySequence::Delete);
    connect(deleteAction, &QAction::triggered, this, [this, entry]() {
        emit deleteRequested(QString::fromStdString(entry->opId));
    });

    menu.exec(pos);
}

HistoryPanel::ItemEntry* HistoryPanel::entryForItem(QTreeWidgetItem* item) {
    for (auto& entry : entries_) {
        if (entry.item == item) {
            return &entry;
        }
    }
    return nullptr;
}

HistoryPanel::ItemEntry* HistoryPanel::entryForId(const std::string& opId) {
    for (auto& entry : entries_) {
        if (entry.opId == opId) {
            return &entry;
        }
    }
    return nullptr;
}

void HistoryPanel::setCollapsed(bool collapsed) {
    if (collapsed_ == collapsed) return;
    collapsed_ = collapsed;
    applyCollapseState(true);
    emit collapsedChanged(collapsed_);
}

void HistoryPanel::applyCollapseState(bool animate) {
    const int targetWidth = collapsed_ ? collapsedWidth_ : expandedWidth_;

    if (!animate) {
        panel_->setVisible(!collapsed_);
        setMinimumWidth(targetWidth);
        setMaximumWidth(targetWidth);
        return;
    }

    if (!collapsed_) {
        panel_->setVisible(true);
    }

    setMinimumWidth(0);

    if (widthAnimation_) {
        widthAnimation_->stop();
        widthAnimation_->deleteLater();
    }

    widthAnimation_ = new QPropertyAnimation(this, "maximumWidth", this);
    widthAnimation_->setDuration(180);
    widthAnimation_->setEasingCurve(QEasingCurve::InOutCubic);
    widthAnimation_->setStartValue(width());
    widthAnimation_->setEndValue(targetWidth);

    connect(widthAnimation_, &QPropertyAnimation::finished, this, [this, targetWidth]() {
        if (collapsed_) {
            panel_->setVisible(false);
        }
        setMaximumWidth(targetWidth);
        setMinimumWidth(targetWidth);
    });

    widthAnimation_->start();
}

void HistoryPanel::onOperationAdded(const QString& opId) {
    rebuild();
}

void HistoryPanel::onOperationRemoved(const QString& opId) {
    rebuild();
}

void HistoryPanel::onOperationFailed(const QString& opId, const QString& reason) {
    auto* entry = entryForId(opId.toStdString());
    if (entry) {
        entry->failed = true;
        entry->failureReason = reason.toStdString();
        updateItemState(*entry);
    }
}

void HistoryPanel::onOperationSucceeded(const QString& opId) {
    auto* entry = entryForId(opId.toStdString());
    if (entry) {
        entry->failed = false;
        entry->failureReason.clear();
        updateItemState(*entry);
    }
}

void HistoryPanel::onOperationSuppressed(const QString& opId, bool suppressed) {
    auto* entry = entryForId(opId.toStdString());
    if (entry) {
        entry->suppressed = suppressed;
        updateItemState(*entry);
    }
}

} // namespace onecad::ui
