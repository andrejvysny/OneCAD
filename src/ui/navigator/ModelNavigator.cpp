#include "ModelNavigator.h"
#include <QTreeWidget>
#include <QVBoxLayout>
#include <QHeaderView>

namespace onecad {
namespace ui {

ModelNavigator::ModelNavigator(QWidget* parent)
    : QDockWidget(tr("Navigator"), parent) {
    setupUi();
    createPlaceholderItems();
    
    // Start collapsed per specification
    setMinimumWidth(200);
    setMaximumWidth(400);
}

void ModelNavigator::setupUi() {
    QWidget* container = new QWidget(this);
    QVBoxLayout* layout = new QVBoxLayout(container);
    layout->setContentsMargins(0, 0, 0, 0);
    
    m_treeWidget = new QTreeWidget(container);
    m_treeWidget->setHeaderHidden(true);
    m_treeWidget->setIndentation(16);
    m_treeWidget->setAnimated(true);
    m_treeWidget->setExpandsOnDoubleClick(false);
    
    // Style
    m_treeWidget->setStyleSheet(R"(
        QTreeWidget {
            background-color: #2d2d30;
            color: #cccccc;
            border: none;
        }
        QTreeWidget::item:hover {
            background-color: #3e3e42;
        }
        QTreeWidget::item:selected {
            background-color: #094771;
        }
    )");
    
    connect(m_treeWidget, &QTreeWidget::itemClicked,
            this, &ModelNavigator::onItemClicked);
    connect(m_treeWidget, &QTreeWidget::itemDoubleClicked,
            this, &ModelNavigator::onItemDoubleClicked);
    
    layout->addWidget(m_treeWidget);
    setWidget(container);
}

void ModelNavigator::createPlaceholderItems() {
    // Bodies section
    m_bodiesRoot = new QTreeWidgetItem(m_treeWidget);
    m_bodiesRoot->setText(0, tr("📦 Bodies"));
    m_bodiesRoot->setExpanded(true);
    
    // Sketches section
    m_sketchesRoot = new QTreeWidgetItem(m_treeWidget);
    m_sketchesRoot->setText(0, tr("✏️ Sketches"));
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

void ModelNavigator::onItemClicked(QTreeWidgetItem* item, int column) {
    Q_UNUSED(column);
    if (item && item != m_bodiesRoot && item != m_sketchesRoot) {
        emit itemSelected(item->data(0, Qt::UserRole).toString());
    }
}

void ModelNavigator::onItemDoubleClicked(QTreeWidgetItem* item, int column) {
    Q_UNUSED(column);
    if (item && item != m_bodiesRoot && item != m_sketchesRoot) {
        emit itemDoubleClicked(item->data(0, Qt::UserRole).toString());
    }
}

} // namespace ui
} // namespace onecad
