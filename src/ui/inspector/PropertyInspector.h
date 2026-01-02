#ifndef ONECAD_UI_INSPECTOR_PROPERTYINSPECTOR_H
#define ONECAD_UI_INSPECTOR_PROPERTYINSPECTOR_H

#include <QDockWidget>

class QLabel;
class QVBoxLayout;
class QStackedWidget;

namespace onecad {
namespace ui {

/**
 * @brief Property inspector showing selected entity properties.
 * 
 * Context-dependent panel:
 * - No selection: tips and hints
 * - Entity selected: coordinates, dimensions
 * - Operation active: parameters with Apply/Cancel
 */
class PropertyInspector : public QDockWidget {
    Q_OBJECT

public:
    explicit PropertyInspector(QWidget* parent = nullptr);
    ~PropertyInspector() override = default;

public slots:
    void showEmptyState();
    void showEntityProperties(const QString& entityType, const QString& entityId);

private:
    void setupUi();
    void createEmptyStateWidget();
    void createPropertiesWidget();

    QStackedWidget* m_stackedWidget = nullptr;
    QWidget* m_emptyWidget = nullptr;
    QWidget* m_propertiesWidget = nullptr;
    QLabel* m_entityTypeLabel = nullptr;
    QLabel* m_entityIdLabel = nullptr;
};

} // namespace ui
} // namespace onecad

#endif // ONECAD_UI_INSPECTOR_PROPERTYINSPECTOR_H
