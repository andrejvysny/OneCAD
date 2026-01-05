#ifndef ONECAD_UI_NAVIGATOR_MODELNAVIGATOR_H
#define ONECAD_UI_NAVIGATOR_MODELNAVIGATOR_H

#include <QWidget>
#include <QString>
#include <unordered_map>
#include <string>

class QTreeWidget;
class QTreeWidgetItem;
class QFrame;
class QPropertyAnimation;

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
class ModelNavigator : public QWidget {
    Q_OBJECT

public:
    explicit ModelNavigator(QWidget* parent = nullptr);
    ~ModelNavigator() override = default;
    void setCollapsed(bool collapsed);
    bool isCollapsed() const { return m_collapsed; }

signals:
    void itemSelected(const QString& itemId);
    void itemDoubleClicked(const QString& itemId);
    void editSketchRequested(const QString& sketchId);
    void sketchSelected(const QString& sketchId);
    void bodySelected(const QString& bodyId);
    void collapsedChanged(bool collapsed);

public slots:
    // Document model integration
    void onSketchAdded(const QString& id);
    void onSketchRemoved(const QString& id);
    void onSketchRenamed(const QString& id, const QString& newName);
    void onBodyAdded(const QString& id);
    void onBodyRemoved(const QString& id);
    void onBodyRenamed(const QString& id, const QString& newName);

private slots:
    void onItemClicked(QTreeWidgetItem* item, int column);
    void onItemDoubleClicked(QTreeWidgetItem* item, int column);

private:
    struct ItemCollection {
        std::unordered_map<std::string, QTreeWidgetItem*>& items;
        unsigned int& counter;
        QTreeWidgetItem* root = nullptr;
        QString namePrefix;
        QString placeholderText;
    };

    void setupUi();
    void createPlaceholderItems();
    void applyCollapseState(bool animate);
    void addItem(ItemCollection& collection, const QString& id);
    void removeItem(ItemCollection& collection, const QString& id);
    void renameItem(ItemCollection& collection, const QString& id, const QString& newName);

    QFrame* m_panel = nullptr;
    QTreeWidget* m_treeWidget = nullptr;
    QTreeWidgetItem* m_bodiesRoot = nullptr;
    QTreeWidgetItem* m_sketchesRoot = nullptr;
    bool m_collapsed = false;
    QPropertyAnimation* m_widthAnimation = nullptr;
    int m_expandedWidth = 260;
    int m_collapsedWidth = 0;

    // Map sketch IDs to tree items
    std::unordered_map<std::string, QTreeWidgetItem*> m_sketchItems;
    std::unordered_map<std::string, QTreeWidgetItem*> m_bodyItems;

    // Counter for unique sketch naming
    unsigned int m_sketchCounter = 0;
    unsigned int m_bodyCounter = 0;
};

} // namespace ui
} // namespace onecad

#endif // ONECAD_UI_NAVIGATOR_MODELNAVIGATOR_H
