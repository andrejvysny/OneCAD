#ifndef ONECAD_UI_SELECTION_DEEPSELECTPOPUP_H
#define ONECAD_UI_SELECTION_DEEPSELECTPOPUP_H

#include <QFrame>
#include <QStringList>

class QListWidget;
class QHideEvent;
class QPoint;

namespace onecad::ui::selection {

class DeepSelectPopup : public QFrame {
    Q_OBJECT

public:
    explicit DeepSelectPopup(QWidget* parent = nullptr);

    void setCandidateLabels(const QStringList& labels);
    void showAt(const QPoint& globalPos);
    void clearSelection();

signals:
    void candidateHovered(int index);
    void candidateSelected(int index);
    void popupClosed();

protected:
    void hideEvent(QHideEvent* event) override;

private:
    QListWidget* list_ = nullptr;
};

} // namespace onecad::ui::selection

#endif // ONECAD_UI_SELECTION_DEEPSELECTPOPUP_H
