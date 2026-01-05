#include "DeepSelectPopup.h"

#include <QHideEvent>
#include <QListWidget>
#include <QVBoxLayout>
#include <algorithm>

namespace onecad::ui::selection {

DeepSelectPopup::DeepSelectPopup(QWidget* parent)
    : QFrame(parent, Qt::Popup) {
    setFrameShape(QFrame::StyledPanel);
    setFrameShadow(QFrame::Raised);
    setAttribute(Qt::WA_DeleteOnClose, false);

    list_ = new QListWidget(this);
    list_->setSelectionMode(QAbstractItemView::SingleSelection);
    list_->setMouseTracking(true);
    list_->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    list_->setVerticalScrollBarPolicy(Qt::ScrollBarAsNeeded);
    list_->setFocusPolicy(Qt::NoFocus);

    auto* layout = new QVBoxLayout(this);
    layout->setContentsMargins(4, 4, 4, 4);
    layout->addWidget(list_);
    setLayout(layout);

    connect(list_, &QListWidget::itemEntered, this, [this](QListWidgetItem* item) {
        if (!item) return;
        emit candidateHovered(list_->row(item));
    });
    connect(list_, &QListWidget::itemClicked, this, [this](QListWidgetItem* item) {
        if (!item) return;
        emit candidateSelected(list_->row(item));
        hide();
    });
}

void DeepSelectPopup::setCandidateLabels(const QStringList& labels) {
    list_->clear();
    if (labels.isEmpty()) {
        return;
    }
    list_->addItems(labels);
    list_->setCurrentRow(0);
    int rows = labels.size();
    int rowHeight = list_->sizeHintForRow(0);
    int maxVisible = std::min(8, rows);
    int height = rowHeight * maxVisible + 8;
    list_->setFixedHeight(height);
    list_->setFixedWidth(std::max(180, list_->sizeHintForColumn(0) + 24));
}

void DeepSelectPopup::showAt(const QPoint& globalPos) {
    move(globalPos);
    show();
    raise();
}

void DeepSelectPopup::clearSelection() {
    list_->clearSelection();
}

void DeepSelectPopup::hideEvent(QHideEvent* event) {
    emit popupClosed();
    QFrame::hideEvent(event);
}

} // namespace onecad::ui::selection
