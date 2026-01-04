/**
 * @file ConstraintPanel.h
 * @brief Floating panel showing sketch constraints
 */

#ifndef ONECAD_UI_SKETCH_CONSTRAINTPANEL_H
#define ONECAD_UI_SKETCH_CONSTRAINTPANEL_H

#include <QWidget>

class QListWidget;
class QLabel;
class QVBoxLayout;

namespace onecad::core::sketch {
class Sketch;
}

namespace onecad::ui {

/**
 * @brief Floating panel displaying sketch constraints.
 *
 * Shows list of constraints with icons, type names, and status.
 * Visible only in sketch mode.
 */
class ConstraintPanel : public QWidget {
    Q_OBJECT

public:
    explicit ConstraintPanel(QWidget* parent = nullptr);
    ~ConstraintPanel() override = default;

    /**
     * @brief Set the sketch to display constraints for
     */
    void setSketch(core::sketch::Sketch* sketch);

    /**
     * @brief Refresh the constraint list
     */
    void refresh();

signals:
    /**
     * @brief Emitted when a constraint is selected
     */
    void constraintSelected(const QString& constraintId);

    /**
     * @brief Emitted when delete is requested for a constraint
     */
    void constraintDeleteRequested(const QString& constraintId);

private:
    void setupUi();
    void populateList();
    QString getConstraintIcon(int type) const;
    QString getConstraintTypeName(int type) const;

    core::sketch::Sketch* m_sketch = nullptr;
    QVBoxLayout* m_layout = nullptr;
    QLabel* m_titleLabel = nullptr;
    QListWidget* m_listWidget = nullptr;
    QLabel* m_emptyLabel = nullptr;
};

} // namespace onecad::ui

#endif // ONECAD_UI_SKETCH_CONSTRAINTPANEL_H
