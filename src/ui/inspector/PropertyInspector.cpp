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
    
    // Style
    container->setStyleSheet(R"(
        QWidget {
            background-color: #2d2d30;
            color: #cccccc;
        }
        QLabel {
            padding: 4px;
        }
    )");
}

void PropertyInspector::createEmptyStateWidget() {
    m_emptyWidget = new QWidget();
    QVBoxLayout* layout = new QVBoxLayout(m_emptyWidget);
    
    QLabel* iconLabel = new QLabel("🔍");
    iconLabel->setAlignment(Qt::AlignCenter);
    iconLabel->setStyleSheet("font-size: 32px; padding-top: 40px;");
    
    QLabel* titleLabel = new QLabel(tr("No Selection"));
    titleLabel->setAlignment(Qt::AlignCenter);
    titleLabel->setStyleSheet("font-weight: bold; font-size: 14px;");
    
    QLabel* hintLabel = new QLabel(tr("Select an entity to view\nits properties"));
    hintLabel->setAlignment(Qt::AlignCenter);
    hintLabel->setStyleSheet("color: #888888; font-size: 12px;");
    
    QLabel* tipLabel = new QLabel(tr("💡 Tip: Double-click a face\nto start a new sketch"));
    tipLabel->setAlignment(Qt::AlignCenter);
    tipLabel->setStyleSheet("color: #6b9f6b; font-size: 11px; padding-top: 20px;");
    
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
    m_entityTypeLabel->setStyleSheet("font-weight: bold; font-size: 14px;");
    
    m_entityIdLabel = new QLabel();
    m_entityIdLabel->setStyleSheet("color: #888888; font-size: 11px;");
    
    QLabel* separator = new QLabel();
    separator->setFixedHeight(1);
    separator->setStyleSheet("background-color: #3e3e42;");
    
    QLabel* placeholderLabel = new QLabel(tr("Properties will appear here"));
    placeholderLabel->setStyleSheet("color: #666666; font-style: italic;");
    
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
