#ifndef ONECAD_UI_THEME_THEMEMANAGER_H
#define ONECAD_UI_THEME_THEMEMANAGER_H

#include <QObject>
#include <QString>
#include <QColor>

namespace onecad {
namespace ui {

/**
 * @brief Centralized theme management system for OneCAD.
 * 
 * Provides application-wide theming with support for:
 * - Light and Dark themes
 * - System theme synchronization (Qt 6.5+)
 * - Theme persistence across sessions
 * - Runtime theme switching with signal notifications
 * 
 * Usage:
 * @code
 * // Set theme
 * ThemeManager::instance().setThemeMode(ThemeManager::ThemeMode::Dark);
 * 
 * // React to theme changes
 * connect(&ThemeManager::instance(), &ThemeManager::themeChanged,
 *         this, &MyWidget::updateColors);
 * @endcode
 * 
 * @note This is a Meyers singleton - one instance per application.
 * @note Theme preference is automatically saved to QSettings.
 */
class ThemeManager : public QObject {
    Q_OBJECT

public:
    /**
     * @brief Available theme modes.
     */
    enum class ThemeMode {
        Light,   ///< Force light theme regardless of system setting
        Dark,    ///< Force dark theme regardless of system setting
        System   ///< Follow system appearance preference (default)
    };

    /**
     * @brief Get the singleton instance.
     * @return Reference to the global ThemeManager instance.
     */
    static ThemeManager& instance();

    /**
     * @brief Set the theme mode.
     * @param mode The desired theme mode
     * 
     * Automatically applies the theme and persists the choice.
     * Emits themeChanged() signal if mode actually changes.
     */
    void setThemeMode(ThemeMode mode);

    /**
     * @brief Get the current theme mode setting.
     * @return The configured theme mode (may differ from actual appearance in System mode)
     */
    ThemeMode themeMode() const;

    /**
     * @brief Check if the effective theme is dark.
     * @return true if dark theme is active, false for light theme
     * 
     * In System mode, this reflects the current OS appearance.
     */
    bool isDark() const;

    /**
     * @brief Force re-application of the current theme.
     * 
     * Useful for manual refresh or after widget tree changes.
     * Emits themeChanged() signal.
     */
    void applyTheme();

signals:
    /**
     * @brief Emitted when the active theme changes.
     * 
     * Connected widgets should update their styling in response.
     * This signal is emitted after stylesheets are applied.
     */
    void themeChanged();

private:
    /**
     * @brief Private constructor (singleton pattern).
     * 
     * Initializes theme from saved preferences, sets up system
     * theme monitoring (Qt 6.5+), and applies initial theme.
     */
    explicit ThemeManager();

    /**
     * @brief Destructor.
     */
    ~ThemeManager() override;

    // Delete copy constructor and assignment operator
    ThemeManager(const ThemeManager&) = delete;
    ThemeManager& operator=(const ThemeManager&) = delete;

    /**
     * @brief Load theme preference from persistent storage.
     */
    void loadSettings();

    /**
     * @brief Save theme preference to persistent storage.
     */
    void saveSettings();

    /**
     * @brief Apply stylesheets to the application.
     * 
     * Uses cached stylesheets for performance.
     */
    void updateAppStyle();

    /**
     * @brief Get the dark theme stylesheet.
     * @return Cached dark theme QSS string
     */
    static const QString& getDarkStyleSheet();

    /**
     * @brief Get the light theme stylesheet.
     * @return Cached light theme QSS string
     */
    static const QString& getLightStyleSheet();

    ThemeMode m_mode = ThemeMode::System;
};

} // namespace ui
} // namespace onecad

#endif // ONECAD_UI_THEME_THEMEMANAGER_H
