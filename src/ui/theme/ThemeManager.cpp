#include "ThemeManager.h"
#include <QApplication>
#include <QStyleHints>
#include <QGuiApplication>
#include <QPalette>
#include <QSettings>

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
    
    QString modeName = settings.value("theme/mode", "System").toString();
    QString savedId = settings.value("theme/id", QString()).toString();

    if (modeName == "Fixed") {
        m_mode = ThemeMode::Fixed;
        m_themeId = savedId;
    } else if (modeName == "Light" || modeName == "Dark") {
        // Backward compatibility with legacy settings.
        m_mode = ThemeMode::Fixed;
        m_themeId = modeName;
    } else {
        m_mode = ThemeMode::System;
        m_themeId = savedId;
    }

    if (m_mode == ThemeMode::Fixed && !findTheme(m_themeId)) {
        m_mode = ThemeMode::System;
        m_themeId.clear();
    }
}

void ThemeManager::saveSettings() {
    QSettings settings("OneCAD", "OneCAD");
    
    settings.setValue("theme/mode", m_mode == ThemeMode::Fixed ? "Fixed" : "System");
    settings.setValue("theme/id", m_themeId);
    settings.sync(); // Force immediate write
}

// ============================================================================
// Public API
// ============================================================================

void ThemeManager::setThemeMode(ThemeMode mode) {
    if (m_mode != mode) {
        m_mode = mode;
        if (m_mode == ThemeMode::Fixed && !findTheme(m_themeId)) {
            m_themeId = systemTheme(systemPrefersDark()).id;
        }
        saveSettings();   // Persist immediately
        applyTheme();     // Apply and emit signal
    }
}

void ThemeManager::setThemeId(const QString& id) {
    if (m_themeId == id && m_mode == ThemeMode::Fixed) {
        return;
    }
    if (!findTheme(id)) {
        return;
    }
    m_themeId = id;
    m_mode = ThemeMode::Fixed;
    saveSettings();
    applyTheme();
}

QString ThemeManager::themeId() const {
    return m_themeId;
}

ThemeManager::ThemeMode ThemeManager::themeMode() const {
    return m_mode;
}

bool ThemeManager::isDark() const {
    if (m_mode == ThemeMode::System) {
        return systemPrefersDark();
    }
    return currentTheme().isDark;
}

const ThemeDefinition& ThemeManager::currentTheme() const {
    return resolveTheme();
}

const std::vector<ThemeDefinition>& ThemeManager::availableThemes() const {
    return themeCatalog().themes;
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
    
    qApp->setStyleSheet(buildStyleSheet(resolveTheme()));
}

const ThemeDefinition& ThemeManager::resolveTheme() const {
    if (m_mode == ThemeMode::System) {
        return systemTheme(systemPrefersDark());
    }
    if (const ThemeDefinition* theme = findTheme(m_themeId)) {
        return *theme;
    }
    return systemTheme(systemPrefersDark());
}

bool ThemeManager::systemPrefersDark() const {
#if QT_VERSION >= QT_VERSION_CHECK(6, 5, 0)
    if (QGuiApplication::styleHints()) {
        return QGuiApplication::styleHints()->colorScheme() == Qt::ColorScheme::Dark;
    }
#endif

    const QColor bgColor = qApp->palette().color(QPalette::Window);
    const int luminance = (299 * bgColor.red() + 587 * bgColor.green() + 114 * bgColor.blue()) / 1000;
    return luminance < 128;
}

QString ThemeManager::buildStyleSheet(const ThemeDefinition& theme) {
    const ThemeUiColors& ui = theme.ui;
    QString styleSheet = QStringLiteral(R"(
        QMainWindow {
            background-color: @window-bg@;
        }
        QWidget {
            background-color: @widget-bg@;
            color: @widget-text@;
        }
        QWidget#NavigatorPanel {
            background-color: @navigator-bg@;
            border: 1px solid @navigator-divider@;
            border-radius: 14px;
        }
        QTreeWidget#NavigatorTree {
            background-color: transparent;
            color: @navigator-item-text@;
            border: none;
            outline: 0;
            padding: 0px 3px;
        }
        QTreeWidget#NavigatorTree::item {
            margin: 3px 0px;
            padding: 0px;
            height: 32px;
        }
        QTreeWidget#NavigatorTree::item:selected,
        QTreeWidget#NavigatorTree::item:hover,
        QTreeWidget#NavigatorTree::branch:selected,
        QTreeWidget#NavigatorTree::branch:hover {
            background: transparent;
            color: @navigator-item-text@;
        }
        QTreeWidget#NavigatorTree::branch,
        QTreeWidget#NavigatorTree::branch:has-children,
        QTreeWidget#NavigatorTree::branch:closed,
        QTreeWidget#NavigatorTree::branch:open,
        QTreeWidget#NavigatorTree::branch:has-children:!has-siblings:adjoins-item,
        QTreeWidget#NavigatorTree::branch:!has-children:!has-siblings:adjoins-item {
            background: transparent;
        }
        QWidget[nav-item="true"] {
            border-radius: 10px;
            background-color: transparent;
            min-height: 32px;
        }
        QWidget[nav-item="true"]:hover {
            background-color: @navigator-item-hover@;
        }
        QWidget[nav-item="true"][nav-selected="true"] {
            background-color: @navigator-item-selected@;
        }
        QLabel[nav-item-label="true"] {
            color: @navigator-item-text@;
            font-weight: 600;
            padding-left: 0px;
            background: transparent;
        }
        QWidget[nav-item="true"][nav-selected="true"] QLabel[nav-item-label="true"] {
            color: @navigator-item-selected-text@;
        }
        QLabel[nav-item-icon="true"] {
            background: transparent;
        }
        QLabel[nav-header="true"] {
            color: @navigator-header-text@;
            background-color: @navigator-header-bg@;
            padding: 6px 8px;
            border-radius: 8px;
            font-weight: 700;
            letter-spacing: 0.25px;
        }
        QFrame[nav-divider="true"] {
            background-color: @navigator-divider@;
            min-height: 1px;
        }
        QToolButton[nav-inline="true"] {
            background-color: transparent;
            border: none;
            border-radius: 6px;
            padding: 0px;
            icon-size: 18px;
            min-width: 24px;
            min-height: 24px;
        }
        QToolButton[nav-inline="true"]:hover {
            background-color: @navigator-inline-hover-bg@;
        }
        QToolButton[nav-inline="true"]:pressed {
            background-color: @navigator-inline-hover-bg@;
        }
        QDockWidget,
        QWidget#ContextToolbar,
        QWidget#inspectorContainer {
            background-color: @panel-bg@;
            border: 1px solid @panel-border@;
            border-radius: 12px;
        }
        QMenuBar {
            background-color: @menubar-bg@;
            color: @menubar-text@;
            border-bottom: 1px solid @menubar-border@;
        }
        QMenuBar::item {
            background-color: transparent;
        }
        QMenuBar::item:selected {
            background-color: @menu-item-selected-bg@;
            color: @menu-item-selected-text@;
        }
        QMenu {
            background-color: @menu-bg@;
            color: @menu-text@;
            border: 1px solid @menu-border@;
        }
        QMenu::item:selected {
            background-color: @menu-item-selected-bg@;
            color: @menu-item-selected-text@;
        }
        QMenu::separator {
            height: 1px;
            background-color: @menu-separator@;
            margin: 4px 0px;
        }
        QStatusBar {
            background-color: @status-bg@;
            color: @status-text@;
            border-top: 1px solid @status-border@;
        }
        QStatusBar QLabel {
            color: @status-text@;
        }
        QDockWidget {
            color: @widget-text@;
            titlebar-close-icon: none;
        }
        QDockWidget::title {
            background-color: @dock-title-bg@;
            padding: 6px;
            border-bottom: 1px solid @dock-title-border@;
            border-top-left-radius: 12px;
            border-top-right-radius: 12px;
        }
        QTreeWidget {
            background-color: @tree-bg@;
            color: @tree-text@;
        }
        QTreeWidget::item:hover {
            background-color: @tree-hover-bg@;
        }
        QTreeWidget::item:selected {
            background-color: @tree-selected-bg@;
            color: @tree-selected-text@;
        }
        QToolButton[sidebarButton="true"] {
            background-color: @sidebar-bg@;
            color: @sidebar-text@;
            border: 1px solid @sidebar-border@;
            border-radius: 10px;
            padding: 0px;
            min-width: 42px;
            min-height: 42px;
        }
        QToolButton[sidebarButton="true"]:hover {
            background-color: @sidebar-hover-bg@;
            border: 1px solid @sidebar-hover-border@;
        }
        QToolButton[sidebarButton="true"]:pressed,
        QToolButton[sidebarButton="true"]:checked {
            background-color: @sidebar-pressed-bg@;
            border: 1px solid @sidebar-pressed-border@;
        }

        QWidget#inspectorContainer {
            background-color: @inspector-bg@;
            color: @widget-text@;
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
            color: @inspector-hint@;
            font-size: 12px;
        }
        QLabel#inspectorTip {
            color: @inspector-tip@;
            font-size: 11px;
            padding-top: 20px;
        }
        QLabel#inspectorEntityTitle {
            font-weight: bold;
            font-size: 14px;
        }
        QLabel#inspectorEntityId {
            color: @inspector-entity-id@;
            font-size: 11px;
        }
        QLabel#inspectorSeparator {
            background-color: @inspector-separator@;
        }
        QLabel#inspectorPlaceholder {
            color: @inspector-placeholder@;
            font-style: italic;
        }

        QWidget#ContextToolbar {
            padding: 0px;
        }
        QToolButton {
            background-color: @toolbutton-bg@;
            border: 1px solid @toolbutton-border@;
            border-radius: 10px;
            padding: 6px 12px;
            color: @toolbutton-text@;
            min-width: 60px;
        }
        QToolButton:hover {
            background-color: @toolbutton-hover-bg@;
            border: 1px solid @toolbutton-hover-border@;
        }
        QToolButton:pressed {
            background-color: @toolbutton-pressed-bg@;
            border: 1px solid @toolbutton-pressed-border@;
        }
        QScrollBar:vertical {
            border: none;
            background: @scrollbar-track@;
            width: 14px;
            margin: 0px;
        }
        QScrollBar::handle:vertical {
            background: @scrollbar-handle@;
            min-height: 20px;
            border-radius: 7px;
            margin: 2px;
        }
        QScrollBar::handle:vertical:hover {
            background: @scrollbar-handle-hover@;
        }
        QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical {
            height: 0px;
        }
        QScrollBar:horizontal {
            border: none;
            background: @scrollbar-track@;
            height: 14px;
            margin: 0px;
        }
        QScrollBar::handle:horizontal {
            background: @scrollbar-handle@;
            min-width: 20px;
            border-radius: 7px;
            margin: 2px;
        }
        QScrollBar::handle:horizontal:hover {
            background: @scrollbar-handle-hover@;
        }
        QScrollBar::add-line:horizontal, QScrollBar::sub-line:horizontal {
            width: 0px;
        }
    )");

    styleSheet.replace("@window-bg@", toQssColor(ui.windowBackground));
    styleSheet.replace("@widget-bg@", toQssColor(ui.widgetBackground));
    styleSheet.replace("@widget-text@", toQssColor(ui.widgetText));
    styleSheet.replace("@navigator-bg@", toQssColor(ui.navigatorBackground));
    styleSheet.replace("@navigator-divider@", toQssColor(theme.navigator.divider));
    styleSheet.replace("@navigator-header-bg@", toQssColor(theme.navigator.headerBackground));
    styleSheet.replace("@navigator-header-text@", toQssColor(theme.navigator.headerText));
    styleSheet.replace("@navigator-item-text@", toQssColor(theme.navigator.itemText));
    styleSheet.replace("@navigator-item-hover@", toQssColor(theme.navigator.itemHoverBackground));
    styleSheet.replace("@navigator-item-selected@", toQssColor(theme.navigator.itemSelectedBackground));
    styleSheet.replace("@navigator-item-selected-text@", toQssColor(theme.navigator.itemSelectedText));
    styleSheet.replace("@navigator-inline-bg@", toQssColor(theme.navigator.inlineButtonBackground));
    styleSheet.replace("@navigator-inline-hover-bg@", toQssColor(theme.navigator.inlineButtonHoverBackground));
    styleSheet.replace("@navigator-inline-border@", toQssColor(theme.navigator.inlineButtonBorder));
    styleSheet.replace("@navigator-inline-hover-border@", toQssColor(theme.navigator.inlineButtonHoverBorder));
    styleSheet.replace("@panel-bg@", toQssColor(ui.panelBackground));
    styleSheet.replace("@inspector-bg@", toQssColor(ui.inspectorBackground));
    styleSheet.replace("@panel-border@", toQssColor(ui.panelBorder));
    styleSheet.replace("@menubar-bg@", toQssColor(ui.menuBarBackground));
    styleSheet.replace("@menubar-text@", toQssColor(ui.menuBarText));
    styleSheet.replace("@menubar-border@", toQssColor(ui.menuBarBorder));
    styleSheet.replace("@menu-item-selected-bg@", toQssColor(ui.menuItemSelectedBackground));
    styleSheet.replace("@menu-item-selected-text@", toQssColor(ui.menuItemSelectedText));
    styleSheet.replace("@menu-bg@", toQssColor(ui.menuBackground));
    styleSheet.replace("@menu-text@", toQssColor(ui.menuText));
    styleSheet.replace("@menu-border@", toQssColor(ui.menuBorder));
    styleSheet.replace("@menu-separator@", toQssColor(ui.menuSeparator));
    styleSheet.replace("@status-bg@", toQssColor(ui.statusBarBackground));
    styleSheet.replace("@status-text@", toQssColor(ui.statusBarText));
    styleSheet.replace("@status-border@", toQssColor(ui.statusBarBorder));
    styleSheet.replace("@dock-title-bg@", toQssColor(ui.dockTitleBackground));
    styleSheet.replace("@dock-title-border@", toQssColor(ui.dockTitleBorder));
    styleSheet.replace("@tree-bg@", toQssColor(ui.treeBackground));
    styleSheet.replace("@tree-text@", toQssColor(ui.treeText));
    styleSheet.replace("@tree-hover-bg@", toQssColor(ui.treeHoverBackground));
    styleSheet.replace("@tree-selected-bg@", toQssColor(ui.treeSelectedBackground));
    styleSheet.replace("@tree-selected-text@", toQssColor(ui.treeSelectedText));
    styleSheet.replace("@sidebar-bg@", toQssColor(ui.sidebarButtonBackground));
    styleSheet.replace("@sidebar-text@", toQssColor(ui.sidebarButtonText));
    styleSheet.replace("@sidebar-border@", toQssColor(ui.sidebarButtonBorder));
    styleSheet.replace("@sidebar-hover-bg@", toQssColor(ui.sidebarButtonHoverBackground));
    styleSheet.replace("@sidebar-hover-border@", toQssColor(ui.sidebarButtonHoverBorder));
    styleSheet.replace("@sidebar-pressed-bg@", toQssColor(ui.sidebarButtonPressedBackground));
    styleSheet.replace("@sidebar-pressed-border@", toQssColor(ui.sidebarButtonPressedBorder));
    styleSheet.replace("@toolbutton-bg@", toQssColor(ui.toolButtonBackground));
    styleSheet.replace("@toolbutton-text@", toQssColor(ui.toolButtonText));
    styleSheet.replace("@toolbutton-border@", toQssColor(ui.toolButtonBorder));
    styleSheet.replace("@toolbutton-hover-bg@", toQssColor(ui.toolButtonHoverBackground));
    styleSheet.replace("@toolbutton-hover-border@", toQssColor(ui.toolButtonHoverBorder));
    styleSheet.replace("@toolbutton-pressed-bg@", toQssColor(ui.toolButtonPressedBackground));
    styleSheet.replace("@toolbutton-pressed-border@", toQssColor(ui.toolButtonPressedBorder));
    styleSheet.replace("@inspector-hint@", toQssColor(ui.inspectorHintText));
    styleSheet.replace("@inspector-tip@", toQssColor(ui.inspectorTipText));
    styleSheet.replace("@inspector-entity-id@", toQssColor(ui.inspectorEntityIdText));
    styleSheet.replace("@inspector-separator@", toQssColor(ui.inspectorSeparator));
    styleSheet.replace("@inspector-placeholder@", toQssColor(ui.inspectorPlaceholderText));
    styleSheet.replace("@scrollbar-track@", toQssColor(ui.scrollbarTrack));
    styleSheet.replace("@scrollbar-handle@", toQssColor(ui.scrollbarHandle));
    styleSheet.replace("@scrollbar-handle-hover@", toQssColor(ui.scrollbarHandleHover));

    return styleSheet;
}

} // namespace ui
} // namespace onecad
