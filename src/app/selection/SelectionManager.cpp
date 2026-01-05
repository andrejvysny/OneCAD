#include "SelectionManager.h"

#include <algorithm>
#include <cmath>

namespace onecad::app::selection {

namespace {
constexpr double kAmbiguityPixelEpsilon = 2.0;
constexpr int kClickCyclePixelThreshold = 3;
} // namespace

SelectionManager::SelectionManager(QObject* parent)
    : QObject(parent) {
}

void SelectionManager::setMode(SelectionMode mode) {
    if (mode_ == mode) {
        return;
    }
    mode_ = mode;
    clearSelection();
    setHoverItem(std::nullopt);
    lastClickCandidates_.clear();
    lastClickIndex_ = 0;
    lastClickPos_ = std::nullopt;
}

void SelectionManager::setFilter(const SelectionFilter& filter) {
    filter_ = filter;
}

ClickAction SelectionManager::handleClick(const PickResult& result,
                                          const ClickModifiers& modifiers,
                                          const QPoint& screenPos) {
    ClickAction action;
    std::vector<SelectionItem> hits = filterHits(result);

    if (hits.empty()) {
        if (!modifiers.shift && !modifiers.toggle && !selection_.empty()) {
            clearSelection();
            action.selectionChanged = true;
        }
        lastClickCandidates_.clear();
        lastClickIndex_ = 0;
        lastClickPos_ = std::nullopt;
        return action;
    }

    if (deepSelectEnabled_ && isAmbiguous(hits)) {
        action.needsDeepSelect = true;
        action.candidates = hits;
        lastClickCandidates_ = hits;
        lastClickIndex_ = 0;
        lastClickPos_ = screenPos;
        return action;
    }

    if (!deepSelectEnabled_ && isAmbiguous(hits)) {
        if (sameClickLocation(screenPos) && !lastClickCandidates_.empty()) {
            const SelectionItem previousSelection = lastClickCandidates_[lastClickIndex_];
            lastClickCandidates_ = hits;

            SelectionKey prevKey{previousSelection.kind, previousSelection.id};
            auto it = std::find_if(hits.begin(), hits.end(),
                                   [&prevKey](const SelectionItem& item) {
                                       return SelectionKey{item.kind, item.id} == prevKey;
                                   });
            if (it != hits.end()) {
                lastClickIndex_ = (static_cast<size_t>(std::distance(hits.begin(), it)) + 1) %
                                  hits.size();
            } else {
                lastClickIndex_ = 0;
            }
            lastClickPos_ = screenPos;
        } else {
            lastClickCandidates_ = hits;
            lastClickIndex_ = 0;
            lastClickPos_ = screenPos;
        }
        applySelectionInternal(lastClickCandidates_[lastClickIndex_], modifiers);
        action.selectionChanged = true;
        return action;
    }

    applySelectionInternal(hits.front(), modifiers);
    action.selectionChanged = true;
    lastClickCandidates_ = hits;
    lastClickIndex_ = 0;
    lastClickPos_ = screenPos;
    return action;
}

void SelectionManager::applySelectionCandidate(const SelectionItem& candidate,
                                               const ClickModifiers& modifiers,
                                               const QPoint& screenPos) {
    applySelectionInternal(candidate, modifiers);
    lastClickCandidates_.clear();
    lastClickIndex_ = 0;
    lastClickPos_ = screenPos;
}

std::optional<SelectionItem> SelectionManager::topCandidate(const PickResult& result) const {
    std::vector<SelectionItem> hits = filterHits(result);
    if (hits.empty()) {
        return std::nullopt;
    }
    return hits.front();
}

void SelectionManager::updateHover(const PickResult& result) {
    std::vector<SelectionItem> hits = filterHits(result);
    if (hits.empty()) {
        setHoverItem(std::nullopt);
        return;
    }
    setHoverItem(hits.front());
}

void SelectionManager::setHoverItem(const std::optional<SelectionItem>& item) {
    if (!hover_.has_value() && !item.has_value()) {
        return;
    }
    if (hover_.has_value() != item.has_value()) {
        hover_ = item;
        emit hoverChanged();
        return;
    }
    if (hover_.has_value() && item.has_value()) {
        SelectionKey a{hover_->kind, hover_->id};
        SelectionKey b{item->kind, item->id};
        if (a == b) {
            return;
        }
    }
    hover_ = item;
    emit hoverChanged();
}

void SelectionManager::clearSelection() {
    if (selection_.empty()) {
        return;
    }
    selection_.clear();
    emit selectionChanged();
}

std::vector<SelectionItem> SelectionManager::filterHits(const PickResult& result) const {
    std::vector<SelectionItem> hits;
    hits.reserve(result.hits.size());
    for (const auto& hit : result.hits) {
        if (filter_.allows(hit.kind)) {
            hits.push_back(hit);
        }
    }

    std::sort(hits.begin(), hits.end(), [](const SelectionItem& a, const SelectionItem& b) {
        if (a.priority != b.priority) {
            return a.priority < b.priority;
        }
        if (a.screenDistance != b.screenDistance) {
            return a.screenDistance < b.screenDistance;
        }
        return a.depth < b.depth;
    });
    return hits;
}

void SelectionManager::applySelectionInternal(const SelectionItem& item,
                                              const ClickModifiers& modifiers) {
    if (modifiers.toggle) {
        applySelectionToggle(item);
        return;
    }
    if (modifiers.shift) {
        applySelectionAdd(item);
        return;
    }
    applySelectionReplace(item);
}

void SelectionManager::applySelectionReplace(const SelectionItem& item) {
    selection_.clear();
    selection_.push_back(item);
    emit selectionChanged();
}

void SelectionManager::applySelectionToggle(const SelectionItem& item) {
    SelectionKey key{item.kind, item.id};
    auto it = findInSelection(key);
    if (it != selection_.end()) {
        selection_.erase(it);
    } else {
        selection_.push_back(item);
    }
    emit selectionChanged();
}

void SelectionManager::applySelectionAdd(const SelectionItem& item) {
    SelectionKey key{item.kind, item.id};
    auto it = findInSelection(key);
    if (it == selection_.end()) {
        selection_.push_back(item);
        emit selectionChanged();
    }
}

bool SelectionManager::isAmbiguous(const std::vector<SelectionItem>& hits) const {
    if (hits.size() < 2) {
        return false;
    }
    const auto& top = hits.front();
    const auto& second = hits[1];
    if (top.priority != second.priority) {
        return false;
    }
    return std::abs(top.screenDistance - second.screenDistance) <= kAmbiguityPixelEpsilon;
}

bool SelectionManager::sameClickLocation(const QPoint& screenPos) const {
    if (!lastClickPos_.has_value()) {
        return false;
    }
    QPoint delta = screenPos - *lastClickPos_;
    return std::abs(delta.x()) <= kClickCyclePixelThreshold &&
           std::abs(delta.y()) <= kClickCyclePixelThreshold;
}

std::vector<SelectionItem>::iterator SelectionManager::findInSelection(const SelectionKey& key) {
    return std::find_if(selection_.begin(), selection_.end(),
                        [&key](const SelectionItem& entry) {
                            return SelectionKey{entry.kind, entry.id} == key;
                        });
}

} // namespace onecad::app::selection
