#include "DimensionEditor.h"
#include "../theme/ThemeManager.h"

#include <QKeyEvent>
#include <QFocusEvent>
#include <QRegularExpression>
#include <QStack>

#include <cmath>

namespace onecad::ui {

DimensionEditor::DimensionEditor(QWidget* parent)
    : QLineEdit(parent)
{
    setAlignment(Qt::AlignCenter);
    setFrame(true);

    m_themeConnection = connect(&ThemeManager::instance(), &ThemeManager::themeChanged,
                                this, &DimensionEditor::updateTheme, Qt::UniqueConnection);
    updateTheme();

    // Connect return pressed to confirmation
    connect(this, &QLineEdit::returnPressed, this, &DimensionEditor::confirmValue);
}

void DimensionEditor::showForConstraint(const QString& constraintId, double currentValue,
                                         const QString& units, const QPoint& screenPos) {
    showEditorWithPolicy(Mode::Constraint,
                         ValidationPolicy::PositiveOnly,
                         constraintId,
                         currentValue,
                         units,
                         screenPos);
}

void DimensionEditor::showForDraftParameter(const QString& parameterId, double currentValue,
                                            const QString& units, const QPoint& screenPos) {
    showEditorWithPolicy(Mode::DraftParameter,
                         ValidationPolicy::AnyFinite,
                         parameterId,
                         currentValue,
                         units,
                         screenPos);
}

void DimensionEditor::showEditorWithPolicy(Mode mode, ValidationPolicy validationPolicy,
                                           const QString& targetId, double currentValue,
                                           const QString& units, const QPoint& screenPos) {
    m_mode = mode;
    m_validationPolicy = validationPolicy;
    m_constraintId.clear();
    m_draftParameterId.clear();
    if (mode == Mode::Constraint) {
        m_constraintId = targetId;
    } else if (mode == Mode::DraftParameter) {
        m_draftParameterId = targetId;
    }
    m_originalValue = currentValue;
    m_units = units;

    QString valueText = QString::number(currentValue, 'f', 2);
    if (!units.isEmpty()) {
        valueText += " " + units;
    }

    setText(valueText);
    selectAll();

    adjustSize();
    int editorWidth = qMax(width(), 100);
    int editorHeight = height();
    move(screenPos.x() - editorWidth / 2, screenPos.y() - editorHeight / 2);

    show();
    setFocus();
    raise();
}

void DimensionEditor::cancel() {
    m_mode = Mode::None;
    m_constraintId.clear();
    m_draftParameterId.clear();
    m_originalValue = 0.0;
    hide();
    emit editCancelled();
}

void DimensionEditor::keyPressEvent(QKeyEvent* event) {
    if (event->key() == Qt::Key_Escape) {
        cancel();
        return;
    }
    if (m_mode == Mode::DraftParameter && event->key() == Qt::Key_Tab) {
        bool forward = !(event->modifiers() & Qt::ShiftModifier);
        emit tabNavigationRequested(forward);
        return;
    }
    QLineEdit::keyPressEvent(event);
}

void DimensionEditor::focusOutEvent(QFocusEvent* event) {
    // Cancel on focus loss (unless clicking another widget that takes focus)
    if (event->reason() != Qt::PopupFocusReason) {
        cancel();
    }
    QLineEdit::focusOutEvent(event);
}

void DimensionEditor::confirmValue() {
    if (m_mode == Mode::None) {
        cancel();
        return;
    }

    QString input = text().trimmed();

    // Remove units suffix if present
    if (!m_units.isEmpty()) {
        input.remove(QRegularExpression("\\s*" + QRegularExpression::escape(m_units) + "\\s*$",
                                         QRegularExpression::CaseInsensitiveOption));
    }

    double newValue = parseExpression(input);

    if (std::isnan(newValue) || std::isinf(newValue)) {
        if (m_mode == Mode::Constraint) {
            showForConstraint(m_constraintId, m_originalValue, m_units, pos());
        } else {
            showForDraftParameter(m_draftParameterId, m_originalValue, m_units, pos());
        }
        return;
    }

    if (m_validationPolicy == ValidationPolicy::PositiveOnly && newValue <= 0.0) {
        if (m_mode == Mode::Constraint) {
            showForConstraint(m_constraintId, m_originalValue, m_units, pos());
        } else {
            showForDraftParameter(m_draftParameterId, m_originalValue, m_units, pos());
        }
        return;
    }

    QString targetId = (m_mode == Mode::Constraint) ? m_constraintId : m_draftParameterId;
    Mode confirmedMode = m_mode;
    m_mode = Mode::None;
    m_constraintId.clear();
    m_draftParameterId.clear();
    hide();

    if (confirmedMode == Mode::Constraint) {
        emit valueConfirmed(targetId, newValue);
    } else if (confirmedMode == Mode::DraftParameter) {
        emit draftValueConfirmed(targetId, newValue);
    }
}

double DimensionEditor::parseExpression(const QString& text) const {
    // Simple expression parser supporting +, -, *, /, (, )
    // Using shunting-yard algorithm for proper operator precedence

    QString expr = text.simplified().replace(" ", "");
    if (expr.isEmpty()) {
        return m_originalValue;
    }

    // Tokenize
    QStringList tokens;
    QString currentNum;

    for (int i = 0; i < expr.length(); ++i) {
        QChar c = expr[i];

        if (c.isDigit() || c == '.') {
            currentNum += c;
        } else if (c == '-' && (tokens.isEmpty() || tokens.last() == "(" ||
                                 tokens.last() == "+" || tokens.last() == "-" ||
                                 tokens.last() == "*" || tokens.last() == "/")) {
            // Unary minus
            currentNum += c;
        } else {
            if (!currentNum.isEmpty()) {
                tokens << currentNum;
                currentNum.clear();
            }
            if (c == '+' || c == '-' || c == '*' || c == '/' || c == '(' || c == ')') {
                tokens << QString(c);
            } else {
                // Unrecognized character - invalid expression
                return std::nan("");
            }
        }
    }
    if (!currentNum.isEmpty()) {
        tokens << currentNum;
    }

    // Shunting-yard algorithm
    QStack<double> values;
    QStack<QChar> ops;

    auto precedence = [](QChar op) -> int {
        if (op == '+' || op == '-') return 1;
        if (op == '*' || op == '/') return 2;
        return 0;
    };

    auto applyOp = [](double a, double b, QChar op) -> double {
        switch (op.toLatin1()) {
            case '+': return a + b;
            case '-': return a - b;
            case '*': return a * b;
            case '/': return b != 0 ? a / b : std::nan("");
            default: return std::nan("");
        }
    };

    for (const QString& token : tokens) {
        if (token.length() == 1 && QString("+-*/()").contains(token)) {
            QChar op = token[0];

            if (op == '(') {
                ops.push(op);
            } else if (op == ')') {
                while (!ops.isEmpty() && ops.top() != '(') {
                    if (values.size() < 2) return std::nan("");
                    double b = values.pop();
                    double a = values.pop();
                    values.push(applyOp(a, b, ops.pop()));
                }
                if (!ops.isEmpty()) ops.pop(); // Remove '('
            } else {
                while (!ops.isEmpty() && ops.top() != '(' &&
                       precedence(ops.top()) >= precedence(op)) {
                    if (values.size() < 2) return std::nan("");
                    double b = values.pop();
                    double a = values.pop();
                    values.push(applyOp(a, b, ops.pop()));
                }
                ops.push(op);
            }
        } else {
            // Number
            bool ok;
            double val = token.toDouble(&ok);
            if (!ok) return std::nan("");
            values.push(val);
        }
    }

    // Apply remaining operators
    while (!ops.isEmpty()) {
        // Check for unmatched opening parenthesis
        if (ops.top() == '(') {
            return std::nan("");
        }
        if (values.size() < 2) return std::nan("");
        double b = values.pop();
        double a = values.pop();
        values.push(applyOp(a, b, ops.pop()));
    }

    // Final value stack must contain exactly one value
    if (values.size() != 1) {
        return std::nan("");
    }
    return values.top();
}

void DimensionEditor::updateTheme() {
    const ThemeDimensionEditorColors& colors = ThemeManager::instance().currentTheme().dimensionEditor;
    setStyleSheet(QStringLiteral(R"(
        QLineEdit {
            background-color: %1;
            border: 2px solid %2;
            border-radius: 4px;
            padding: 4px 8px;
            font-size: 12px;
            font-weight: bold;
            min-width: 80px;
        }
        QLineEdit:focus {
            border-color: %3;
        }
    )")
    .arg(toQssColor(colors.background),
         toQssColor(colors.border),
         toQssColor(colors.borderFocus)));
}

} // namespace onecad::ui
