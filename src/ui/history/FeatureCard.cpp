/**
 * @file FeatureCard.cpp
 */
#include "FeatureCard.h"
#include "../theme/ThemeManager.h"

#include <QHBoxLayout>
#include <QLabel>
#include <QToolButton>
#include <QIcon>
#include <QPainter>
#include <QStyle>

namespace onecad::ui {

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

FeatureCard::FeatureCard(QWidget* parent)
    : QWidget(parent) {
    // Layout matching ModelNavigator
    auto* layout = new QHBoxLayout(this);
    layout->setContentsMargins(8, 6, 8, 8); // Slightly different to account for border? matching Navigator's 8,6,8,6
    layout->setSpacing(8);

    iconLabel_ = new QLabel(this);
    iconLabel_->setProperty("nav-item-icon", true);
    iconLabel_->setFixedSize(20, 20);
    iconLabel_->setScaledContents(true);
    iconLabel_->setAutoFillBackground(false);
    iconLabel_->setAttribute(Qt::WA_TranslucentBackground);
    layout->addWidget(iconLabel_);

    textLabel_ = new QLabel(this);
    textLabel_->setProperty("nav-item-label", true);
    textLabel_->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
    textLabel_->setAutoFillBackground(false);
    textLabel_->setAttribute(Qt::WA_TranslucentBackground);
    layout->addWidget(textLabel_, 1);

    statusButton_ = new QToolButton(this);
    statusButton_->setProperty("nav-inline", true);
    statusButton_->setAutoRaise(true);
    statusButton_->setCursor(Qt::PointingHandCursor);
    statusButton_->setFocusPolicy(Qt::NoFocus);
    layout->addWidget(statusButton_);

    overflowButton_ = new QToolButton(this);
    overflowButton_->setProperty("nav-inline", true);
    overflowButton_->setAutoRaise(true);
    overflowButton_->setCursor(Qt::PointingHandCursor);
    overflowButton_->setFocusPolicy(Qt::NoFocus);
    overflowButton_->setToolTip(tr("More actions"));
    layout->addWidget(overflowButton_);

    // Connect signals
    connect(statusButton_, &QToolButton::clicked, this, &FeatureCard::suppressToggled);
    connect(overflowButton_, &QToolButton::clicked, this, &FeatureCard::menuRequested);

    // Initial state
    statusButton_->hide();
    overflowButton_->hide();
    setProperty("nav-item", true);
    
    // Borders are handled by stylesheet in HistoryPanel or ThemeManager, but we can enforce local override if needed for 1px border requirement
    setAttribute(Qt::WA_Hover); // Ensure hover events work

}

void FeatureCard::setName(const QString& name) {
    name_ = name;
    updateText();
}

void FeatureCard::setDetails(const QString& details) {
    details_ = details;
    updateText();
}

void FeatureCard::setIconPath(const QString& path) {
    iconPath_ = path;
    updateStyle();
}

void FeatureCard::setFailed(bool failed, const QString& reason) {
    failed_ = failed;
    failureReason_ = reason;
    updateStyle();
}

void FeatureCard::setSuppressed(bool suppressed) {
    suppressed_ = suppressed;
    updateStyle();
    updateText();
}

void FeatureCard::setSelected(bool selected) {
    if (selected_ == selected) return;
    selected_ = selected;
    setProperty("nav-selected", selected);
    style()->unpolish(this);
    style()->polish(this);
    updateStyle();
}

void FeatureCard::updateTheme() {
    updateStyle();
}

void FeatureCard::enterEvent(QEnterEvent* event) {
    hovered_ = true;
    updateStyle();
    QWidget::enterEvent(event);
}

void FeatureCard::leaveEvent(QEvent* event) {
    hovered_ = false;
    updateStyle();
    QWidget::leaveEvent(event);
}

void FeatureCard::updateText() {
    const auto& theme = ThemeManager::instance().currentTheme();
    QColor textColor = selected_ ? theme.navigator.itemSelectedText : theme.navigator.itemText;
    QColor detailColor = theme.navigator.placeholderText;
    
    if (failed_) {
        textColor = theme.status.dofError;
        detailColor = theme.status.dofError;
    } else if (suppressed_) {
        textColor = theme.navigator.placeholderText;
    }

    QString html;
    if (suppressed_) {
        html = QString("<span style='color:%1; font-style:italic;'>%2</span> <span style='color:%3; font-style:italic;'>%4</span>")
            .arg(textColor.name())
            .arg(name_)
            .arg(detailColor.name())
            .arg(details_);
    } else if (failed_) {
         html = QString("<span style='color:%1; text-decoration:line-through;'>%2</span> <span style='color:%3; text-decoration:line-through;'>%4</span>")
            .arg(textColor.name())
            .arg(name_)
            .arg(detailColor.name())
            .arg(details_);       
    } else {
        html = QString("<span style='font-weight:600; color:%1;'>%2</span> <span style='color:%3;'>%4</span>")
            .arg(textColor.name())
            .arg(name_)
            .arg(detailColor.name())
            .arg(details_);
    }
    
    textLabel_->setText(html);
}

void FeatureCard::updateStyle() {
    const auto& theme = ThemeManager::instance().currentTheme();
    
    // Update background and border based on state
    // We want 1px border. standard nav-item property might just check selection.
    // Let's manually set the stylesheet to be precise about the 1px border and hover color
    
    QColor bgColor = Qt::transparent;
    QColor borderColor = theme.ui.panelBorder; // Or transparent if not wanted always? User said "all rows have 1px border"
    // Actually, usually lists have borders between items or around them.
    // If "all rows have ... 1px border", likely means a border-bottom or full border.
    // Given the screenshot, it looks like individual cards with spacing?
    // Let's assume full border for the card.
    
    if (selected_) {
        bgColor = theme.navigator.itemSelectedBackground;
        borderColor = theme.navigator.itemSelectedBackground; // Highlight border on select? Matching bg
    } else if (hovered_) {
         // "Item should be highlighted with primary color" -> usually this means text or icon, or a subtle background tint.
         // Consistent with Navigator: itemHoverBackground.
         bgColor = theme.navigator.itemHoverBackground;
         // "highlighted with primary color" -> using the selection color for border to indicate "primary" highlight
         borderColor = theme.navigator.itemSelectedBackground;
    } else {
        // Base state
        // "proper and same background color for all sections"
        bgColor = theme.ui.panelBackground; // Or a specific card background?
        borderColor = theme.ui.panelBorder; // Default border
    }

    // Apply Stylesheet to self
    setStyleSheet(QString(
        "QWidget[nav-item=\"true\"] { "
        "  background-color: %1; "
        "  border: 1px solid %2; "
        "  border-radius: 6px; " // Rounded corners matching screenshot look usually
        "}"
    ).arg(bgColor.name(QColor::HexArgb))
     .arg(borderColor.name(QColor::HexArgb)));


    QColor iconColor = selected_ ? theme.navigator.itemSelectedText : theme.navigator.itemIcon;
    if (hovered_ && !selected_) {
         // "highlighted with primary color" on hover
         // Use selected background color as the "primary" brand color
         iconColor = theme.navigator.itemSelectedBackground;
    }

    // Status Button (Suppress/Eye)
    if (failed_) {
        statusButton_->setText("!");
        statusButton_->setIcon(QIcon());
        statusButton_->setStyleSheet(QString("QToolButton { color: %1; font-weight: bold; border: none; background: transparent; }").arg(theme.status.dofError.name()));
        statusButton_->setToolTip(failureReason_.isEmpty() ? tr("Operation Failed") : failureReason_);
        statusButton_->show();
    } else {
        statusButton_->setText("");
        statusButton_->setStyleSheet("QToolButton { border: none; background: transparent; }");
        
        if (suppressed_) {
            statusButton_->setIcon(QIcon(tintIcon(":/icons/ic_eye_off.svg", theme.navigator.placeholderText)));
            statusButton_->setToolTip(tr("Unsuppress"));
            statusButton_->show();
        } else {
            statusButton_->setIcon(QIcon(tintIcon(":/icons/ic_eye_on.svg", iconColor)));
            statusButton_->setToolTip(tr("Suppress"));
            // "On hover icons should be shown" -> implies hidden otherwise?
            statusButton_->setVisible(hovered_ || selected_);
        }
    }

    // Overflow button
    overflowButton_->setIcon(QIcon(tintIcon(":/icons/ic_overflow.svg", iconColor)));
    overflowButton_->setVisible(hovered_ || selected_);

    // Update main icon
    if (!iconPath_.isEmpty()) {
        iconLabel_->setPixmap(tintIcon(iconPath_, iconColor));
    }
    
    updateText();
}

} // namespace onecad::ui
