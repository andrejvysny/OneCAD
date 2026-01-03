#include "ThemeManager.h"
#include <QApplication>
#include <QStyleHints>
#include <QGuiApplication>
#include <QPalette>
#include <QSettings>
#include <QVersionNumber>

namespace onecad {
namespace ui {

// ============================================================================
// Singleton
// ============================================================================

ThemeManager& ThemeManager::instance() {
    static ThemeManager instance;
    return instance;
}

// ============================================================================
// Constructor / Destructor
// ============================================================================

ThemeManager::ThemeManager() : QObject(nullptr) {
    // Load saved theme preference
    loadSettings();
    
#if QT_VERSION >= QT_VERSION_CHECK(6, 5, 0)
    // Watch for system theme changes (Qt 6.5+)
    // Using Qt::QueuedConnection to avoid potential recursion issues
    if (QGuiApplication::styleHints()) {
        connect(QGuiApplication::styleHints(), &QStyleHints::colorSchemeChanged,
                this, &ThemeManager::applyTheme, Qt::QueuedConnection);
    }
#endif
    
    // Apply initial theme
    applyTheme();
}

ThemeManager::~ThemeManager() = default;

// ============================================================================
// Settings Persistence
// ============================================================================

void ThemeManager::loadSettings() {
    QSettings settings("OneCAD", "OneCAD");
    
    QString themeName = settings.value("theme/mode", "System").toString();
    
    if (themeName == "Light") {
        m_mode = ThemeMode::Light;
    } else if (themeName == "Dark") {
        m_mode = ThemeMode::Dark;
    } else {
        m_mode = ThemeMode::System;
    }
}

void ThemeManager::saveSettings() {
    QSettings settings("OneCAD", "OneCAD");
    
    QString themeName;
    switch (m_mode) {
        case ThemeMode::Light:  themeName = "Light"; break;
        case ThemeMode::Dark:   themeName = "Dark"; break;
        case ThemeMode::System: themeName = "System"; break;
    }
    
    settings.setValue("theme/mode", themeName);
    settings.sync(); // Force immediate write
}

// ============================================================================
// Public API
// ============================================================================

void ThemeManager::setThemeMode(ThemeMode mode) {
    if (m_mode != mode) {
        m_mode = mode;
        saveSettings();   // Persist immediately
        applyTheme();     // Apply and emit signal
    }
}

ThemeManager::ThemeMode ThemeManager::themeMode() const {
    return m_mode;
}

bool ThemeManager::isDark() const {
    if (m_mode == ThemeMode::Dark) return true;
    if (m_mode == ThemeMode::Light) return false;
    
    // System mode - check OS preference
#if QT_VERSION >= QT_VERSION_CHECK(6, 5, 0)
    if (QGuiApplication::styleHints()) {
        return QGuiApplication::styleHints()->colorScheme() == Qt::ColorScheme::Dark;
    }
#endif
    
    // Fallback: check system palette (works on most platforms)
    // Light backgrounds have high luminance, dark backgrounds have low luminance
    const QColor bgColor = qApp->palette().color(QPalette::Window);
    const int luminance = (299 * bgColor.red() + 587 * bgColor.green() + 114 * bgColor.blue()) / 1000;
    return luminance < 128; // Dark if background luminance is low
}

void ThemeManager::applyTheme() {
    updateAppStyle();
    emit themeChanged();
}

// ============================================================================
// Internal Implementation
// ============================================================================

void ThemeManager::updateAppStyle() {
    if (!qApp) {
        // Safety check - should never happen in normal operation
        return;
    }
    
    // Use cached stylesheet - no regeneration overhead
    const QString& styleSheet = isDark() ? getDarkStyleSheet() : getLightStyleSheet();
    qApp->setStyleSheet(styleSheet);
}

// ============================================================================
// Cached Stylesheets
// ============================================================================

const QString& ThemeManager::getDarkStyleSheet() {
    // Cached as static const - constructed once, never regenerated
    static const QString darkStyleSheet = R"(
        QMainWindow {
            background-color: #1e1e1e;
        }
        QWidget {
            background-color: #1e1e1e;
            color: #cccccc;
        }
        QMenuBar {
            background-color: #2d2d30;
            color: #cccccc;
            border-bottom: 1px solid #3e3e42;
        }
        QMenuBar::item {
            background-color: transparent;
        }
        QMenuBar::item:selected {
            background-color: #094771;
        }
        QMenu {
            background-color: #2d2d30;
            color: #cccccc;
            border: 1px solid #3e3e42;
        }
        QMenu::item:selected {
            background-color: #094771;
        }
        QMenu::separator {
            height: 1px;
            background-color: #3e3e42;
            margin: 4px 0px;
        }
        QStatusBar {
            background-color: #007acc;
            color: white;
            border-top: 1px solid #3e3e42;
        }
        QStatusBar QLabel {
            color: white;
        }
        QDockWidget {
            color: #cccccc;
            titlebar-close-icon: none;
            border: 1px solid #3e3e42;
        }
        QDockWidget::title {
            background-color: #2d2d30;
            padding: 6px;
            border-bottom: 1px solid #3e3e42;
        }
        
        /* TreeWidget (Navigator) */
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

        /* Inspector */
        QWidget#inspectorContainer {
            background-color: #2d2d30;
            color: #cccccc;
        }
        QLabel#inspectorIcon {
            font-size: 32px; 
            padding-top: 40px;
        }
        QLabel#inspectorTitle {
            font-weight: bold; 
            font-size: 14px;
        }
        QLabel#inspectorHint {
            color: #888888; 
            font-size: 12px;
        }
        QLabel#inspectorTip {
            color: #6b9f6b; 
            font-size: 11px; 
            padding-top: 20px;
        }
        QLabel#inspectorEntityTitle {
            font-weight: bold;
            font-size: 14px;
        }
        QLabel#inspectorEntityId {
            color: #888888;
            font-size: 11px;
        }
        QLabel#inspectorSeparator {
            background-color: #3e3e42;
        }
        QLabel#inspectorPlaceholder {
            color: #666666;
            font-style: italic;
        }

        /* Toolbar */
        QToolBar {
            background-color: #2d2d30;
            border-bottom: 1px solid #3e3e42;
            spacing: 4px;
            padding: 4px;
        }
        QToolButton {
            background-color: transparent;
            border: 1px solid transparent;
            border-radius: 4px;
            padding: 6px 12px;
            color: #cccccc;
            min-width: 60px;
        }
        QToolButton:hover {
            background-color: #3e3e42;
            border: 1px solid #555555;
        }
        QToolButton:pressed {
            background-color: #007acc;
        }
        /* Scrollbars */
        QScrollBar:vertical {
            border: none;
            background: #1e1e1e;
            width: 14px;
            margin: 0px;
        }
        QScrollBar::handle:vertical {
            background: #424242;
            min-height: 20px;
            border-radius: 7px;
            margin: 2px;
        }
        QScrollBar::handle:vertical:hover {
            background: #686868;
        }
        QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical {
            height: 0px;
        }
        QScrollBar:horizontal {
            border: none;
            background: #1e1e1e;
            height: 14px;
            margin: 0px;
        }
        QScrollBar::handle:horizontal {
            background: #424242;
            min-width: 20px;
            border-radius: 7px;
            margin: 2px;
        }
        QScrollBar::handle:horizontal:hover {
            background: #686868;
        }
        QScrollBar::add-line:horizontal, QScrollBar::sub-line:horizontal {
            width: 0px;
        }
    )";
    
    return darkStyleSheet;
}

const QString& ThemeManager::getLightStyleSheet() {
    // Cached as static const - constructed once, never regenerated
    static const QString lightStyleSheet = R"(
        QMainWindow {
            background-color: #f3f3f3;
        }
        QWidget {
            background-color: #f3f3f3;
            color: #333333;
        }
        QMenuBar {
            background-color: #eeeeee;
            color: #333333;
            border-bottom: 1px solid #cccccc;
        }
        QMenuBar::item {
            background-color: transparent;
        }
        QMenuBar::item:selected {
            background-color: #cce8ff;
            color: #000000;
        }
        QMenu {
            background-color: #ffffff;
            color: #333333;
            border: 1px solid #cccccc;
        }
        QMenu::item:selected {
            background-color: #cce8ff;
            color: #000000;
        }
        QMenu::separator {
            height: 1px;
            background-color: #dddddd;
            margin: 4px 0px;
        }
        QStatusBar {
            background-color: #007acc;
            color: white;
            border-top: 1px solid #cccccc;
        }
        QStatusBar QLabel {
            color: white;
        }
        QDockWidget {
            color: #333333;
            titlebar-close-icon: none;
            border: 1px solid #cccccc;
        }
        QDockWidget::title {
            background-color: #eeeeee;
            padding: 6px;
            border-bottom: 1px solid #cccccc;
        }

        /* TreeWidget (Navigator) */
        QTreeWidget {
            background-color: #ffffff;
            color: #333333;
            border: none;
        }
        QTreeWidget::item:hover {
            background-color: #f0f0f0;
        }
        QTreeWidget::item:selected {
            background-color: #cce8ff;
            color: #000000;
        }

        /* Inspector */
        QWidget#inspectorContainer {
            background-color: #ffffff;
            color: #333333;
        }
        QLabel#inspectorIcon {
            font-size: 32px; 
            padding-top: 40px;
        }
        QLabel#inspectorTitle {
            font-weight: bold; 
            font-size: 14px;
        }
        QLabel#inspectorHint {
            color: #666666; 
            font-size: 12px;
        }
        QLabel#inspectorTip {
            color: #3a8f3a; 
            font-size: 11px; 
            padding-top: 20px;
        }
        QLabel#inspectorEntityTitle {
            font-weight: bold;
            font-size: 14px;
        }
        QLabel#inspectorEntityId {
            color: #666666;
            font-size: 11px;
        }
        QLabel#inspectorSeparator {
            background-color: #cccccc;
        }
        QLabel#inspectorPlaceholder {
            color: #888888;
            font-style: italic;
        }

        /* Toolbar */
        QToolBar {
            background-color: #eeeeee;
            border-bottom: 1px solid #cccccc;
            spacing: 4px;
            padding: 4px;
        }
        QToolButton {
            background-color: transparent;
            border: 1px solid transparent;
            border-radius: 4px;
            padding: 6px 12px;
            color: #333333;
            min-width: 60px;
        }
        QToolButton:hover {
            background-color: #ffffff;
            border: 1px solid #bbbbbb;
        }
        QToolButton:pressed {
            background-color: #cce8ff;
        }
        /* Scrollbars */
        QScrollBar:vertical {
            border: none;
            background: #f3f3f3;
            width: 14px;
            margin: 0px;
        }
        QScrollBar::handle:vertical {
            background: #c1c1c1;
            min-height: 20px;
            border-radius: 7px;
            margin: 2px;
        }
        QScrollBar::handle:vertical:hover {
            background: #a8a8a8;
        }
        QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical {
            height: 0px;
        }
        QScrollBar:horizontal {
            border: none;
            background: #f3f3f3;
            height: 14px;
            margin: 0px;
        }
        QScrollBar::handle:horizontal {
            background: #c1c1c1;
            min-width: 20px;
            border-radius: 7px;
            margin: 2px;
        }
        QScrollBar::handle:horizontal:hover {
            background: #a8a8a8;
        }
        QScrollBar::add-line:horizontal, QScrollBar::sub-line:horizontal {
            width: 0px;
        }
    )";
    
    return lightStyleSheet;
}

} // namespace ui
} // namespace onecad
