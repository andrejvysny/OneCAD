#include "ModelNavigator.h"
#include "../theme/ThemeManager.h"
#include <QFrame>
#include <QHeaderView>
#include <QHBoxLayout>
#include <QIcon>
#include <QLabel>
#include <QMenu>
#include <QPainter>
#include <QPropertyAnimation>
#include <QSizePolicy>
#include <QItemSelectionModel>
#include <QAbstractItemView>
#include <QToolButton>
#include <QTreeWidget>
#include <QVBoxLayout>
#include <QEasingCurve>
#include <QRect>
#include <algorithm>

namespace onecad {
namespace ui {

namespace {
const QSize kIconSize(18, 18);

QPixmap tintIcon(const QString& path, const QColor& color) {
    QIcon icon(path);
    QPixmap pixmap = icon.pixmap(kIconSize);
    if (pixmap.isNull()) {
        pixmap = QPixmap(kIconSize);
        pixmap.fill(Qt::transparent);
    }
    QPainter painter(&pixmap);
    painter.setCompositionMode(QPainter::CompositionMode_SourceIn);
    painter.fillRect(pixmap.rect(), color);
    return pixmap;
}
} // namespace

ModelNavigator::ModelNavigator(QWidget* parent)
    : QWidget(parent) {
    setupUi();
    setupRoots();
    m_themeConnection = connect(&ThemeManager::instance(), &ThemeManager::themeChanged,
                                this, &ModelNavigator::updateTheme, Qt::UniqueConnection);
    updateTheme();
    createPlaceholderItems();
    applyCollapseState(false);
}

void ModelNavigator::setupUi() {
    setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Expanding);

    QVBoxLayout* layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);

    m_panel = new QFrame(this);
    m_panel->setObjectName("NavigatorPanel");
    m_panel->setFrameShape(QFrame::NoFrame);
    m_panel->setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Expanding);

    QVBoxLayout* panelLayout = new QVBoxLayout(m_panel);
    panelLayout->setContentsMargins(12, 12, 12, 12);
    panelLayout->setSpacing(10);

    m_treeWidget = new QTreeWidget(m_panel);
    m_treeWidget->setObjectName("NavigatorTree");
    m_treeWidget->setHeaderHidden(true);
    m_treeWidget->setIndentation(12);
    m_treeWidget->setAnimated(true);
    m_treeWidget->setExpandsOnDoubleClick(false);
    m_treeWidget->setSelectionMode(QAbstractItemView::SingleSelection);
    m_treeWidget->setSelectionBehavior(QAbstractItemView::SelectRows);
    m_treeWidget->setRootIsDecorated(true);
    m_treeWidget->setUniformRowHeights(true);
    m_treeWidget->setContextMenuPolicy(Qt::CustomContextMenu);
    m_treeWidget->setFocusPolicy(Qt::NoFocus);
    m_treeWidget->header()->setSectionResizeMode(QHeaderView::Stretch);
    m_treeWidget->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);

    connect(m_treeWidget, &QTreeWidget::itemClicked,
            this, &ModelNavigator::onItemClicked);
    connect(m_treeWidget, &QTreeWidget::itemDoubleClicked,
            this, &ModelNavigator::onItemDoubleClicked);
    connect(m_treeWidget, &QTreeWidget::customContextMenuRequested, this, [this](const QPoint& pos) {
        if (QTreeWidgetItem* item = m_treeWidget->itemAt(pos)) {
            if (item != m_bodiesRoot && item != m_sketchesRoot) {
                showContextMenu(m_treeWidget->viewport()->mapToGlobal(pos), item);
            }
        }
    });
    connect(m_treeWidget->selectionModel(), &QItemSelectionModel::selectionChanged,
            this, [this]() { updateSelectionState(); });

    panelLayout->addWidget(m_treeWidget, 1);

    layout->addWidget(m_panel);
}

void ModelNavigator::setupRoots() {
    m_bodiesRoot = new QTreeWidgetItem(m_treeWidget);
    m_bodiesRoot->setFlags(m_bodiesRoot->flags() & ~Qt::ItemIsSelectable);
    m_bodiesRoot->setChildIndicatorPolicy(QTreeWidgetItem::ShowIndicator);
    m_bodiesRoot->setFirstColumnSpanned(true);
    m_bodiesRoot->setExpanded(true);
    m_treeWidget->setItemWidget(m_bodiesRoot, 0, createSectionHeader(tr("Bodies")));

    m_sketchesRoot = new QTreeWidgetItem(m_treeWidget);
    m_sketchesRoot->setFlags(m_sketchesRoot->flags() & ~Qt::ItemIsSelectable);
    m_sketchesRoot->setChildIndicatorPolicy(QTreeWidgetItem::ShowIndicator);
    m_sketchesRoot->setFirstColumnSpanned(true);
    m_sketchesRoot->setExpanded(true);
    m_treeWidget->setItemWidget(m_sketchesRoot, 0, createSectionHeader(tr("Sketches")));
}

QWidget* ModelNavigator::createSectionHeader(const QString& text) {
    auto* label = new QLabel(text);
    label->setProperty("nav-header", true);
    label->setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Fixed);
    return label;
}

void ModelNavigator::createPlaceholder(QTreeWidgetItem* root, const QString& text) {
    auto* placeholder = new QTreeWidgetItem(root);
    placeholder->setText(0, text);
    placeholder->setFlags(placeholder->flags() & ~Qt::ItemIsSelectable);
    placeholder->setForeground(0, m_placeholderColor);
    placeholder->setSizeHint(0, QSize(0, 32));
}

void ModelNavigator::createPlaceholderItems() {
    createPlaceholder(m_bodiesRoot, tr("(No bodies)"));
    createPlaceholder(m_sketchesRoot, tr("(No sketches)"));
}

QWidget* ModelNavigator::createItemWidget(ItemEntry& entry, const QString& text, ItemType type, bool visible) {
    auto* container = new QWidget();
    container->setProperty("nav-item", true);
    auto* layout = new QHBoxLayout(container);
    layout->setContentsMargins(8, 6, 8, 6);
    layout->setSpacing(8);

    entry.iconLabel = new QLabel(container);
    entry.iconLabel->setProperty("nav-item-icon", true);
    entry.iconLabel->setFixedSize(20, 20);
    entry.iconLabel->setScaledContents(true);
    entry.iconLabel->setAutoFillBackground(false);
    entry.iconLabel->setAttribute(Qt::WA_TranslucentBackground);

    entry.textLabel = new QLabel(text, container);
    entry.textLabel->setProperty("nav-item-label", true);
    entry.textLabel->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
    entry.textLabel->setAutoFillBackground(false);
    entry.textLabel->setAttribute(Qt::WA_TranslucentBackground);

    entry.eyeButton = new QToolButton(container);
    entry.eyeButton->setProperty("nav-inline", true);
    entry.eyeButton->setAutoRaise(true);
    entry.eyeButton->setCursor(Qt::PointingHandCursor);
    entry.eyeButton->setFocusPolicy(Qt::NoFocus);
    entry.eyeButton->setToolTip(tr("Toggle visibility"));

    entry.overflowButton = new QToolButton(container);
    entry.overflowButton->setProperty("nav-inline", true);
    entry.overflowButton->setAutoRaise(true);
    entry.overflowButton->setCursor(Qt::PointingHandCursor);
    entry.overflowButton->setFocusPolicy(Qt::NoFocus);
    entry.overflowButton->setToolTip(tr("More actions"));

    layout->addWidget(entry.iconLabel);
    layout->addWidget(entry.textLabel, 1);
    layout->addSpacing(8);
    layout->addWidget(entry.eyeButton);
    layout->addWidget(entry.overflowButton);

    entry.type = type;
    entry.visible = visible;
    entry.widget = container;
    return container;
}

void ModelNavigator::refreshItemWidget(ItemEntry& entry) {
    const auto& theme = ThemeManager::instance().currentTheme();
    const bool selected = entry.item && entry.item->isSelected();

    entry.widget->setProperty("nav-selected", selected);
    entry.widget->style()->unpolish(entry.widget);
    entry.widget->style()->polish(entry.widget);

    const QColor textColor = selected ? theme.navigator.itemSelectedText : theme.navigator.itemText;
    entry.textLabel->setStyleSheet(QStringLiteral("color: %1;").arg(textColor.name(QColor::HexArgb)));

    const QColor iconColor = selected ? theme.navigator.itemSelectedText : theme.navigator.itemIcon;
    const QString iconPath = (entry.type == ItemType::Body) ? QStringLiteral(":/icons/ic_body.svg")
                                                           : QStringLiteral(":/icons/ic_sketch.svg");
    entry.iconLabel->setPixmap(tintIcon(iconPath, iconColor));

    const QString eyePath = entry.visible ? QStringLiteral(":/icons/ic_eye_on.svg")
                                          : QStringLiteral(":/icons/ic_eye_off.svg");
    entry.eyeButton->setIcon(QIcon(tintIcon(eyePath, iconColor)));
    entry.eyeButton->setToolTip(entry.visible ? tr("Hide") : tr("Show"));

    entry.overflowButton->setIcon(QIcon(tintIcon(QStringLiteral(":/icons/ic_overflow.svg"), iconColor)));
}

void ModelNavigator::setupItemConnections(ItemEntry& entry) {
    QTreeWidgetItem* itemPtr = entry.item;
    QToolButton* eyeBtn = entry.eyeButton;
    QToolButton* overflowBtn = entry.overflowButton;

    connect(eyeBtn, &QToolButton::clicked, this, [this, itemPtr]() {
        if (ItemEntry* found = entryForItem(itemPtr)) {
            handleVisibilityToggle(*found);
        }
    });

    connect(overflowBtn, &QToolButton::clicked, this, [this, itemPtr, overflowBtn]() {
        if (ItemEntry* found = entryForItem(itemPtr)) {
            const QPoint globalPos = overflowBtn->mapToGlobal(overflowBtn->rect().bottomRight());
            showContextMenu(globalPos, found->item);
        }
    });
}

void ModelNavigator::addItem(ItemCollection& collection, const QString& id) {
    if (collection.items.empty() && collection.root->childCount() > 0) {
        QTreeWidgetItem* firstChild = collection.root->child(0);
        if (firstChild && !(firstChild->flags() & Qt::ItemIsSelectable)) {
            delete firstChild;
        }
    }

    ++collection.counter;
    auto* item = new QTreeWidgetItem(collection.root);
    const QString displayName = collection.namePrefix.arg(collection.counter);
    item->setText(0, QString());
    item->setData(0, Qt::UserRole, id);
    item->setFlags(item->flags() | Qt::ItemIsSelectable);
    item->setSizeHint(0, QSize(0, 32));

    ItemEntry entry;
    entry.item = item;
    entry.type = (collection.root == m_bodiesRoot) ? ItemType::Body : ItemType::Sketch;
    entry.visible = true;

    QWidget* widget = createItemWidget(entry, displayName, entry.type, entry.visible);
    m_entries.push_back(entry);
    ItemEntry& stored = m_entries.back();

    m_treeWidget->setItemWidget(item, 0, widget);
    collection.items[id.toStdString()] = item;
    collection.root->setExpanded(true);

    setupItemConnections(stored);
    refreshItemWidget(stored);
}

void ModelNavigator::removeItem(ItemCollection& collection, const QString& id) {
    std::string stdId = id.toStdString();
    auto it = collection.items.find(stdId);
    if (it != collection.items.end()) {
        QTreeWidgetItem* item = it->second;
        auto entryIt = std::remove_if(m_entries.begin(), m_entries.end(), [item](const ItemEntry& entry) {
            return entry.item == item;
        });
        m_entries.erase(entryIt, m_entries.end());

        delete item;
        collection.items.erase(it);
    }

    if (collection.items.empty()) {
        createPlaceholder(collection.root, collection.placeholderText);
    }
}

void ModelNavigator::updateTheme() {
    m_placeholderColor = ThemeManager::instance().currentTheme().navigator.placeholderText;
    updatePlaceholderColors();
    for (auto& entry : m_entries) {
        refreshItemWidget(entry);
    }
}

void ModelNavigator::updatePlaceholderColors() {
    applyPlaceholderColor(m_bodiesRoot);
    applyPlaceholderColor(m_sketchesRoot);
}

void ModelNavigator::applyPlaceholderColor(QTreeWidgetItem* item) {
    if (!item) {
        return;
    }
    for (int i = 0; i < item->childCount(); ++i) {
        QTreeWidgetItem* child = item->child(i);
        if (child && !(child->flags() & Qt::ItemIsSelectable)) {
            child->setForeground(0, m_placeholderColor);
        }
    }
}

void ModelNavigator::renameItem(ItemCollection& collection,
                                const QString& id,
                                const QString& newName) {
    std::string stdId = id.toStdString();
    auto it = collection.items.find(stdId);
    if (it != collection.items.end()) {
        if (ItemEntry* entry = entryForItem(it->second)) {
            entry->textLabel->setText(newName);
            refreshItemWidget(*entry);
        }
    }
}

void ModelNavigator::setCollapsed(bool collapsed) {
    if (m_collapsed == collapsed) {
        return;
    }

    m_collapsed = collapsed;
    applyCollapseState(true);
    emit collapsedChanged(m_collapsed);
}

void ModelNavigator::applyCollapseState(bool animate) {
    const int targetWidth = m_collapsed ? m_collapsedWidth : m_expandedWidth;

    if (!animate) {
        m_panel->setVisible(!m_collapsed);
        setMinimumWidth(targetWidth);
        setMaximumWidth(targetWidth);
        return;
    }

    if (!m_collapsed) {
        m_panel->setVisible(true);
    }

    setMinimumWidth(0);

    if (m_widthAnimation) {
        m_widthAnimation->stop();
        m_widthAnimation->deleteLater();
    }

    m_widthAnimation = new QPropertyAnimation(this, "maximumWidth", this);
    m_widthAnimation->setDuration(180);
    m_widthAnimation->setEasingCurve(QEasingCurve::InOutCubic);
    m_widthAnimation->setStartValue(width());
    m_widthAnimation->setEndValue(targetWidth);

    connect(m_widthAnimation, &QPropertyAnimation::finished, this, [this, targetWidth]() {
        if (m_collapsed) {
            m_panel->setVisible(false);
        }
        setMaximumWidth(targetWidth);
        setMinimumWidth(targetWidth);
    });

    m_widthAnimation->start();
}

ModelNavigator::ItemEntry* ModelNavigator::entryForItem(QTreeWidgetItem* item) {
    for (auto& entry : m_entries) {
        if (entry.item == item) {
            return &entry;
        }
    }
    return nullptr;
}

ModelNavigator::ItemEntry* ModelNavigator::entryForId(const std::string& id) {
    auto bodyIt = m_bodyItems.find(id);
    if (bodyIt != m_bodyItems.end()) {
        return entryForItem(bodyIt->second);
    }
    auto sketchIt = m_sketchItems.find(id);
    if (sketchIt != m_sketchItems.end()) {
        return entryForItem(sketchIt->second);
    }
    return nullptr;
}

void ModelNavigator::updateSelectionState() {
    for (auto& entry : m_entries) {
        refreshItemWidget(entry);
    }
}

void ModelNavigator::handleVisibilityToggle(ItemEntry& entry) {
    entry.visible = !entry.visible;
    refreshItemWidget(entry);
    if (entry.item) {
        emit visibilityToggled(entry.item->data(0, Qt::UserRole).toString(), entry.visible);
    }
}

void ModelNavigator::showContextMenu(const QPoint& pos, QTreeWidgetItem* item) {
    ItemEntry* entry = entryForItem(item);
    if (!entry) {
        return;
    }

    QMenu menu(this);
    QAction* renameAction = menu.addAction(tr("Rename"));
    QAction* isolateAction = menu.addAction(tr("Isolate"));
    QAction* deleteAction = menu.addAction(tr("Delete"));
    menu.addSeparator();
    QAction* visibilityAction = menu.addAction(entry->visible ? tr("Hide") : tr("Show"));

    QAction* chosen = menu.exec(pos);
    if (!chosen) {
        return;
    }

    const QString id = item->data(0, Qt::UserRole).toString();
    if (chosen == renameAction) {
        emit renameRequested(id);
    } else if (chosen == isolateAction) {
        emit isolateRequested(id);
    } else if (chosen == deleteAction) {
        emit deleteRequested(id);
    } else if (chosen == visibilityAction) {
        handleVisibilityToggle(*entry);
    }
}

void ModelNavigator::onItemClicked(QTreeWidgetItem* item, int column) {
    Q_UNUSED(column);
    if (item && item != m_bodiesRoot && item != m_sketchesRoot) {
        QString itemId = item->data(0, Qt::UserRole).toString();
        emit itemSelected(itemId);

        std::string stdId = itemId.toStdString();
        auto sketchIt = m_sketchItems.find(stdId);
        if (sketchIt != m_sketchItems.end() && sketchIt->second == item) {
            emit sketchSelected(itemId);
        }
        auto bodyIt = m_bodyItems.find(stdId);
        if (bodyIt != m_bodyItems.end() && bodyIt->second == item) {
            emit bodySelected(itemId);
        }
    }
}

void ModelNavigator::onItemDoubleClicked(QTreeWidgetItem* item, int column) {
    Q_UNUSED(column);
    if (item && item != m_bodiesRoot && item != m_sketchesRoot) {
        QString itemId = item->data(0, Qt::UserRole).toString();
        emit itemDoubleClicked(itemId);

        std::string stdId = itemId.toStdString();
        auto it = m_sketchItems.find(stdId);
        if (it != m_sketchItems.end() && it->second == item) {
            emit editSketchRequested(itemId);
        }
    }
}

void ModelNavigator::onSketchAdded(const QString& id) {
    ItemCollection collection{m_sketchItems, m_sketchCounter, m_sketchesRoot,
                              QStringLiteral("Sketch %1"), tr("(No sketches)")};
    addItem(collection, id);
}

void ModelNavigator::onSketchRemoved(const QString& id) {
    ItemCollection collection{m_sketchItems, m_sketchCounter, m_sketchesRoot,
                              QStringLiteral("Sketch %1"), tr("(No sketches)")};
    removeItem(collection, id);
}

void ModelNavigator::onSketchRenamed(const QString& id, const QString& newName) {
    ItemCollection collection{m_sketchItems, m_sketchCounter, m_sketchesRoot,
                              QStringLiteral("Sketch %1"), tr("(No sketches)")};
    renameItem(collection, id, newName);
}

void ModelNavigator::onBodyAdded(const QString& id) {
    ItemCollection collection{m_bodyItems, m_bodyCounter, m_bodiesRoot,
                              QStringLiteral("Body %1"), tr("(No bodies)")};
    addItem(collection, id);
}

void ModelNavigator::onBodyRemoved(const QString& id) {
    ItemCollection collection{m_bodyItems, m_bodyCounter, m_bodiesRoot,
                              QStringLiteral("Body %1"), tr("(No bodies)")};
    removeItem(collection, id);
}

void ModelNavigator::onBodyRenamed(const QString& id, const QString& newName) {
    ItemCollection collection{m_bodyItems, m_bodyCounter, m_bodiesRoot,
                              QStringLiteral("Body %1"), tr("(No bodies)")};
    renameItem(collection, id, newName);
}

} // namespace ui
} // namespace onecad
