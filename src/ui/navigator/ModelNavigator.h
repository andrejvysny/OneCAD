#ifndef ONECAD_UI_NAVIGATOR_MODELNAVIGATOR_H
#define ONECAD_UI_NAVIGATOR_MODELNAVIGATOR_H

#include <QDockWidget>

class QTreeWidget;
class QTreeWidgetItem;

namespace onecad {
namespace ui {

/**
 * @brief Model navigator showing document structure.
 * 
 * Displays hierarchical tree of:
 * - Bodies
 * - Sketches  
 * - Feature History (when parametric mode)
 */
class ModelNavigator : public QDockWidget {
    Q_OBJECT

public:
    explicit ModelNavigator(QWidget* parent = nullptr);
    ~ModelNavigator() override = default;

signals:
    void itemSelected(const QString& itemId);
    void itemDoubleClicked(const QString& itemId);

private slots:
    void onItemClicked(QTreeWidgetItem* item, int column);
    void onItemDoubleClicked(QTreeWidgetItem* item, int column);

private:
    void setupUi();
    void createPlaceholderItems();

    QTreeWidget* m_treeWidget = nullptr;
    QTreeWidgetItem* m_bodiesRoot = nullptr;
    QTreeWidgetItem* m_sketchesRoot = nullptr;
};

} // namespace ui
} // namespace onecad

#endif // ONECAD_UI_NAVIGATOR_MODELNAVIGATOR_H
