#include "PropertyInspector.h"
#include <QLabel>
#include <QVBoxLayout>
#include <QStackedWidget>

namespace onecad {
namespace ui {

PropertyInspector::PropertyInspector(QWidget* parent)
    : QDockWidget(tr("Inspector"), parent) {
    setupUi();
    showEmptyState();
    
    setMinimumWidth(250);
    setMaximumWidth(400);
}

void PropertyInspector::setupUi() {
    QWidget* container = new QWidget(this);
    QVBoxLayout* mainLayout = new QVBoxLayout(container);
    mainLayout->setContentsMargins(0, 0, 0, 0);
    
    m_stackedWidget = new QStackedWidget(container);
    
    createEmptyStateWidget();
    createPropertiesWidget();
    
    m_stackedWidget->addWidget(m_emptyWidget);
    m_stackedWidget->addWidget(m_propertiesWidget);
    
    mainLayout->addWidget(m_stackedWidget);
    setWidget(container);
    
    // Set object name for styling
    container->setObjectName("inspectorContainer");
}

void PropertyInspector::createEmptyStateWidget() {
    m_emptyWidget = new QWidget();
    QVBoxLayout* layout = new QVBoxLayout(m_emptyWidget);
    
    QLabel* iconLabel = new QLabel("🔍");
    iconLabel->setObjectName("inspectorIcon");
    iconLabel->setAlignment(Qt::AlignCenter);
    
    QLabel* titleLabel = new QLabel(tr("No Selection"));
    titleLabel->setObjectName("inspectorTitle");
    titleLabel->setAlignment(Qt::AlignCenter);
    
    QLabel* hintLabel = new QLabel(tr("Select an entity to view\nits properties"));
    hintLabel->setObjectName("inspectorHint");
    hintLabel->setAlignment(Qt::AlignCenter);
    
    QLabel* tipLabel = new QLabel(tr("💡 Tip: Double-click a face\nto start a new sketch"));
    tipLabel->setObjectName("inspectorTip");
    tipLabel->setAlignment(Qt::AlignCenter);
    
    layout->addWidget(iconLabel);
    layout->addWidget(titleLabel);
    layout->addWidget(hintLabel);
    layout->addWidget(tipLabel);
    layout->addStretch();
}

void PropertyInspector::createPropertiesWidget() {
    m_propertiesWidget = new QWidget();
    QVBoxLayout* layout = new QVBoxLayout(m_propertiesWidget);
    layout->setContentsMargins(12, 12, 12, 12);
    
    m_entityTypeLabel = new QLabel();
    m_entityTypeLabel->setObjectName("inspectorEntityTitle");
    
    m_entityIdLabel = new QLabel();
    m_entityIdLabel->setObjectName("inspectorEntityId");
    
    QLabel* separator = new QLabel();
    separator->setObjectName("inspectorSeparator");
    separator->setFixedHeight(1);
    
    QLabel* placeholderLabel = new QLabel(tr("Properties will appear here"));
    placeholderLabel->setObjectName("inspectorPlaceholder");
    
    layout->addWidget(m_entityTypeLabel);
    layout->addWidget(m_entityIdLabel);
    layout->addWidget(separator);
    layout->addSpacing(8);
    layout->addWidget(placeholderLabel);
    layout->addStretch();
}

void PropertyInspector::showEmptyState() {
    m_stackedWidget->setCurrentWidget(m_emptyWidget);
}

void PropertyInspector::showEntityProperties(const QString& entityType, const QString& entityId) {
    m_entityTypeLabel->setText(entityType);
    m_entityIdLabel->setText(tr("ID: %1").arg(entityId));
    m_stackedWidget->setCurrentWidget(m_propertiesWidget);
}

} // namespace ui
} // namespace onecad
