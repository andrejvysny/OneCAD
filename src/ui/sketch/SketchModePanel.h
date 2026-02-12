/**
 * @file SketchModePanel.h
 * @brief Floating panel with constraint creation buttons for sketch mode
 */

#ifndef ONECAD_UI_SKETCH_SKETCHMODEPANEL_H
#define ONECAD_UI_SKETCH_SKETCHMODEPANEL_H

#include <QWidget>
#include <unordered_set>
#include <vector>

class QPushButton;
class QVBoxLayout;
class QLabel;

namespace onecad::core::sketch {
class Sketch;
enum class ConstraintType;
}

namespace onecad::ui {

/**
 * @brief Floating panel with constraint creation buttons.
 *
 * Shows buttons for applying constraints to selected entities.
 * Buttons are contextually enabled based on selection.
 */
class SketchModePanel : public QWidget {
    Q_OBJECT

public:
    explicit SketchModePanel(QWidget* parent = nullptr);
    ~SketchModePanel() override = default;

    /**
     * @brief Set the sketch to work with
     */
    void setSketch(core::sketch::Sketch* sketch);

    /**
     * @brief Set which constraint buttons are currently applicable for selection.
     */
    void setApplicableConstraints(
        const std::unordered_set<core::sketch::ConstraintType>& applicableConstraints);

    /**
     * @brief Update button states based on current selection
     */
    void updateButtonStates();

signals:
    /**
     * @brief Emitted when a constraint button is clicked
     */
    void constraintRequested(core::sketch::ConstraintType constraintType);

    /**
     * @brief Emitted when a tool button is clicked
     */
    void toolRequested(int toolType);

private:
    struct ConstraintButton {
        core::sketch::ConstraintType type;
        QString icon;
        QString name;
        QString shortcut;
        QPushButton* button = nullptr;
    };

    void setupUi();
    QPushButton* createButton(const QString& icon, const QString& name,
                              const QString& shortcut, const QString& tooltip);

    core::sketch::Sketch* m_sketch = nullptr;
    QVBoxLayout* m_layout = nullptr;
    QLabel* m_titleLabel = nullptr;
    std::unordered_set<core::sketch::ConstraintType> m_applicableConstraints;

    std::vector<ConstraintButton> m_constraintButtons;
};

} // namespace onecad::ui

#endif // ONECAD_UI_SKETCH_SKETCHMODEPANEL_H
