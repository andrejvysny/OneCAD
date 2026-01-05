#ifndef ONECAD_APP_SELECTION_SELECTIONMANAGER_H
#define ONECAD_APP_SELECTION_SELECTIONMANAGER_H

#include "SelectionTypes.h"

#include <QObject>
#include <QPoint>
#include <optional>
#include <vector>

namespace onecad::app::selection {

class SelectionManager : public QObject {
    Q_OBJECT

public:
    explicit SelectionManager(QObject* parent = nullptr);

    void setMode(SelectionMode mode);
    SelectionMode mode() const { return mode_; }

    void setFilter(const SelectionFilter& filter);
    const SelectionFilter& filter() const { return filter_; }

    void setDeepSelectEnabled(bool enabled) { deepSelectEnabled_ = enabled; }
    bool deepSelectEnabled() const { return deepSelectEnabled_; }

    ClickAction handleClick(const PickResult& result, const ClickModifiers& modifiers,
                            const QPoint& screenPos);
    void applySelectionCandidate(const SelectionItem& candidate,
                                 const ClickModifiers& modifiers,
                                 const QPoint& screenPos);
    std::optional<SelectionItem> topCandidate(const PickResult& result) const;

    void updateHover(const PickResult& result);
    void setHoverItem(const std::optional<SelectionItem>& item);

    const std::vector<SelectionItem>& selection() const { return selection_; }
    std::optional<SelectionItem> hover() const { return hover_; }

    void clearSelection();

signals:
    void selectionChanged();
    void hoverChanged();

private:
    std::vector<SelectionItem> filterHits(const PickResult& result) const;
    void applySelectionInternal(const SelectionItem& item, const ClickModifiers& modifiers);
    void applySelectionReplace(const SelectionItem& item);
    void applySelectionToggle(const SelectionItem& item);
    void applySelectionAdd(const SelectionItem& item);
    bool isAmbiguous(const std::vector<SelectionItem>& hits) const;
    bool sameClickLocation(const QPoint& screenPos) const;
    std::vector<SelectionItem>::iterator findInSelection(const SelectionKey& key);

    SelectionMode mode_ = SelectionMode::Model;
    SelectionFilter filter_{};
    bool deepSelectEnabled_ = true;

    std::vector<SelectionItem> selection_;
    std::optional<SelectionItem> hover_;

    std::optional<QPoint> lastClickPos_;
    std::vector<SelectionItem> lastClickCandidates_;
    size_t lastClickIndex_ = 0;
};

} // namespace onecad::app::selection

#endif // ONECAD_APP_SELECTION_SELECTIONMANAGER_H
