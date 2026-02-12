/**
 * @file DimensionEditor.h
 * @brief Inline editor for dimensional constraints
 */

#ifndef ONECAD_UI_SKETCH_DIMENSIONEDITOR_H
#define ONECAD_UI_SKETCH_DIMENSIONEDITOR_H

#include <QLineEdit>
#include <QMetaObject>
#include <QString>
#include <QPoint>

class QKeyEvent;
class QFocusEvent;

namespace onecad::ui {

/**
 * @brief Inline editor widget for editing dimensional constraint values.
 *
 * Appears when double-clicking on a dimensional constraint (Distance, Angle,
 * Radius, Diameter). Supports basic math expressions (+, -, *, /).
 *
 * Usage:
 * - Double-click constraint → editor appears at constraint position
 * - Enter value or expression → press Enter to confirm
 * - Press Escape to cancel
 */
class DimensionEditor : public QLineEdit {
    Q_OBJECT

public:
    enum class Mode {
        None,
        Constraint,
        DraftParameter
    };

    explicit DimensionEditor(QWidget* parent = nullptr);
    ~DimensionEditor() override = default;

    /**
     * @brief Show editor for a specific constraint
     * @param constraintId ID of the constraint to edit
     * @param currentValue Current value of the dimension
     * @param units Display units (mm, °, etc.)
     * @param screenPos Position in screen coordinates
     */
    void showForConstraint(const QString& constraintId, double currentValue,
                           const QString& units, const QPoint& screenPos);

    /**
     * @brief Show editor for an editable draft preview parameter.
     * @param parameterId Tool-defined stable preview dimension id
     * @param currentValue Current draft value
     * @param units Display units (mm, °, etc.)
     * @param screenPos Position in viewport-local screen coordinates
     */
    void showForDraftParameter(const QString& parameterId, double currentValue,
                               const QString& units, const QPoint& screenPos);

    /**
     * @brief Hide and reset the editor
     */
    void cancel();

    /**
     * @brief Get the constraint ID being edited
     */
    QString constraintId() const { return m_constraintId; }
    QString draftParameterId() const { return m_draftParameterId; }
    Mode mode() const { return m_mode; }

signals:
    /**
     * @brief Emitted when a new value is confirmed
     * @param constraintId ID of the edited constraint
     * @param newValue The new value
     */
    void valueConfirmed(const QString& constraintId, double newValue);

    /**
     * @brief Emitted when a draft preview value is confirmed.
     */
    void draftValueConfirmed(const QString& parameterId, double newValue);

    /**
     * @brief Emitted when editing is cancelled
     */
    void editCancelled();

    /**
     * @brief Emitted for Tab/Shift+Tab navigation in draft edit mode.
     * @param forward True for Tab, false for Shift+Tab.
     */
    void tabNavigationRequested(bool forward);

protected:
    void keyPressEvent(QKeyEvent* event) override;
    void focusOutEvent(QFocusEvent* event) override;

private:
    enum class ValidationPolicy {
        PositiveOnly,
        AnyFinite
    };

    void showEditorWithPolicy(Mode mode, ValidationPolicy validationPolicy,
                              const QString& targetId, double currentValue,
                              const QString& units, const QPoint& screenPos);
    void updateTheme();
    void confirmValue();
    double parseExpression(const QString& text) const;

    Mode m_mode = Mode::None;
    ValidationPolicy m_validationPolicy = ValidationPolicy::PositiveOnly;
    QString m_constraintId;
    QString m_draftParameterId;
    double m_originalValue = 0.0;
    QString m_units;
    QMetaObject::Connection m_themeConnection;
};

} // namespace onecad::ui

#endif // ONECAD_UI_SKETCH_DIMENSIONEDITOR_H
