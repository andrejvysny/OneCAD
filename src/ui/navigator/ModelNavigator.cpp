#include "ModelNavigator.h"
#include <QTreeWidget>
#include <QVBoxLayout>

#include <QHeaderView>
#include <QFrame>

#include <QSizePolicy>
#include <QPropertyAnimation>
#include <QEasingCurve>

namespace onecad {
namespace ui {

ModelNavigator::ModelNavigator(QWidget* parent)
    : QWidget(parent) {
    setupUi();
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
    panelLayout->setContentsMargins(10, 10, 10, 10);
    panelLayout->setSpacing(8);

    m_treeWidget = new QTreeWidget(m_panel);
    m_treeWidget->setHeaderHidden(true);
    m_treeWidget->setIndentation(16);
    m_treeWidget->setAnimated(true);
    m_treeWidget->setExpandsOnDoubleClick(false);

    connect(m_treeWidget, &QTreeWidget::itemClicked,
            this, &ModelNavigator::onItemClicked);
    connect(m_treeWidget, &QTreeWidget::itemDoubleClicked,
            this, &ModelNavigator::onItemDoubleClicked);

    panelLayout->addWidget(m_treeWidget, 1);

    layout->addWidget(m_panel);
}

void ModelNavigator::createPlaceholderItems() {
    // Bodies section
    m_bodiesRoot = new QTreeWidgetItem(m_treeWidget);
    m_bodiesRoot->setText(0, tr("Bodies"));
    m_bodiesRoot->setExpanded(true);
    
    // Sketches section
    m_sketchesRoot = new QTreeWidgetItem(m_treeWidget);
    m_sketchesRoot->setText(0, tr("Sketches"));
    m_sketchesRoot->setExpanded(true);
    
    // Placeholder items (will be populated dynamically later)
    auto* placeholder = new QTreeWidgetItem(m_bodiesRoot);
    placeholder->setText(0, tr("(No bodies)"));
    placeholder->setFlags(placeholder->flags() & ~Qt::ItemIsSelectable);
    placeholder->setForeground(0, QColor(128, 128, 128));
    
    auto* sketchPlaceholder = new QTreeWidgetItem(m_sketchesRoot);
    sketchPlaceholder->setText(0, tr("(No sketches)"));
    sketchPlaceholder->setFlags(sketchPlaceholder->flags() & ~Qt::ItemIsSelectable);
    sketchPlaceholder->setForeground(0, QColor(128, 128, 128));
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
    item->setText(0, collection.namePrefix.arg(collection.counter));
    item->setData(0, Qt::UserRole, id);
    item->setFlags(item->flags() | Qt::ItemIsSelectable);

    collection.items[id.toStdString()] = item;
    collection.root->setExpanded(true);
}

void ModelNavigator::removeItem(ItemCollection& collection, const QString& id) {
    std::string stdId = id.toStdString();
    auto it = collection.items.find(stdId);
    if (it != collection.items.end()) {
        delete it->second;
        collection.items.erase(it);
    }

    if (collection.items.empty()) {
        auto* placeholder = new QTreeWidgetItem(collection.root);
        placeholder->setText(0, collection.placeholderText);
        placeholder->setFlags(placeholder->flags() & ~Qt::ItemIsSelectable);
        placeholder->setForeground(0, QColor(128, 128, 128));
    }
}

void ModelNavigator::renameItem(ItemCollection& collection,
                                const QString& id,
                                const QString& newName) {
    std::string stdId = id.toStdString();
    auto it = collection.items.find(stdId);
    if (it != collection.items.end()) {
        it->second->setText(0, newName);
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

        // Direct lookup instead of linear iteration
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
