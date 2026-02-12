/**
 * @file ConstraintPanel.h
 * @brief Floating panel showing sketch constraints
 */

#ifndef ONECAD_UI_SKETCH_CONSTRAINTPANEL_H
#define ONECAD_UI_SKETCH_CONSTRAINTPANEL_H

#include <QColor>
#include <QMetaObject>
#include <QWidget>
#include <string>
#include <vector>

class QListWidget;
class QLabel;
class QVBoxLayout;
class QPushButton;

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
     * @brief Update contextual data used to render the panel.
     */
    void setContext(const std::vector<std::string>& selectedEntityIds,
                    const QString& selectedConstraintId,
                    int dof,
                    int conflictCount,
                    int suppressedCount);

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

    /**
     * @brief Emitted when suppress-marker is requested for a constraint.
     */
    void constraintSuppressRequested(const QString& constraintId);

    /**
     * @brief Emitted when user requests restoring all hidden markers.
     */
    void restoreSuppressedRequested();

private:
    void setupUi();
    void populateContext();
    void populateConstraintSelection();
    void populateConstraintDetails();
    void populateSummaryState();
    void updateTheme();
    QString getConstraintIcon(int type) const;
    QString getConstraintTypeName(int type) const;

    core::sketch::Sketch* m_sketch = nullptr;
    std::vector<std::string> m_selectedEntityIds;
    QString m_selectedConstraintId;
    int m_currentDof = 0;
    int m_conflictCount = 0;
    int m_suppressedCount = 0;

    QVBoxLayout* m_layout = nullptr;
    QLabel* m_titleLabel = nullptr;
    QLabel* m_subtitleLabel = nullptr;
    QListWidget* m_listWidget = nullptr;
    QLabel* m_hintLabel = nullptr;
    QPushButton* m_inspectButton = nullptr;
    QPushButton* m_deleteButton = nullptr;
    QPushButton* m_suppressButton = nullptr;
    QPushButton* m_restoreSuppressedButton = nullptr;
    QColor m_unsatisfiedColor;
    QColor m_conflictColor;
    QMetaObject::Connection m_themeConnection;
};

} // namespace onecad::ui

#endif // ONECAD_UI_SKETCH_CONSTRAINTPANEL_H
