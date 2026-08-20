#include "elementmap/ComposedHistory.h"

#include <algorithm>

#include <TopExp.hxx>

namespace onecad::elementmap {

namespace {

bool list_contains(const TopTools_ListOfShape& list, const TopoDS_Shape& shape) {
    for (TopTools_ListIteratorOfListOfShape it(list); it.More(); it.Next()) {
        if (it.Value().IsSame(shape)) return true;
    }
    return false;
}

}  // namespace

ComposedHistory::ComposedHistory(const TopoDS_Shape& predecessor,
                                 const TopoDS_Shape& final_shape) {
    if (!predecessor.IsNull()) TopExp::MapShapes(predecessor, predecessor_subs_);
    if (!final_shape.IsNull()) TopExp::MapShapes(final_shape, final_subs_);
    myShape = final_shape;
    Done();
}

void ComposedHistory::add_stage(const occ::handle<BRepTools_History>& stage) {
    if (!stage.IsNull()) stages_.push_back(stage);
}

void ComposedHistory::add_modified(const TopoDS_Shape& predecessor_sub,
                                   const TopoDS_Shape& successor) {
    if (predecessor_sub.IsNull() || successor.IsNull()) return;
    if (!injected_.IsBound(predecessor_sub)) {
        injected_.Bind(predecessor_sub, TopTools_ListOfShape());
    }
    TopTools_ListOfShape& images = injected_.ChangeFind(predecessor_sub);
    for (TopTools_ListIteratorOfListOfShape it(images); it.More(); it.Next()) {
        if (it.Value().IsSame(successor)) return;
    }
    images.Append(successor);
}

bool ComposedHistory::answerable(const TopoDS_Shape& shape) const {
    // `IsSupportedType` is not a nicety: `BRepTools_History`'s readers assert on an
    // unsupported key, so a `_DEBUG` OCCT build would SIGTRAP on a WIRE or SHELL
    // query rather than answer an empty list. Screening here keeps the answer the
    // same in both build configurations — unanswerable, hence NeedsRepair.
    return !shape.IsNull() && BRepTools_History::IsSupportedType(shape) &&
           predecessor_subs_.Contains(shape);
}

ComposedHistory::Walk ComposedHistory::walk(const TopoDS_Shape& shape) const {
    Walk result;
    std::vector<Successor> frontier{Successor{shape, true}};

    for (const occ::handle<BRepTools_History>& stage : stages_) {
        std::vector<Successor> next;
        const auto add = [&next](const TopoDS_Shape& s, bool modified) {
            for (Successor& seen : next) {
                if (seen.shape.IsSame(s)) {
                    seen.modified = seen.modified || modified;
                    return;
                }
            }
            next.push_back(Successor{s, modified});
        };

        for (const Successor& current : frontier) {
            const TopTools_ListOfShape& modified_images = stage->Modified(current.shape);
            const TopTools_ListOfShape& generated_images = stage->Generated(current.shape);

            if (modified_images.IsEmpty() && generated_images.IsEmpty()) {
                // No successor of any kind. Either the stage consumed the shape
                // (positive deletion evidence) or it never touched it, in which case
                // it survives this stage verbatim.
                //
                // That PASSTHROUGH deliberately diverges from `BRepTools_History`'s
                // model, where an untouched shape is simply absent from every map and
                // `Modified` answers an empty list. `apply_history` needs the other
                // convention: its own no-successor branch reads "not deleted, not
                // modified" as "does the old shape survive verbatim?" and rebinds to
                // `old`. A chain must therefore carry an untouched shape ACROSS a
                // stage that ignored it, or a face nothing touched at stage 1 would
                // lose every later stage's rebinding and land in NeedsRepair.
                if (stage->IsRemoved(current.shape)) {
                    result.removal_evidence = true;
                } else {
                    add(current.shape, current.modified);
                }
                continue;
            }
            // A removed shape may still carry successors — OCCT's own example is a
            // fillet, whose seed edge generates the blend face and is then removed.
            // Following them is what `BRepTools_History::Merge` measurably does not.
            for (TopTools_ListIteratorOfListOfShape it(modified_images); it.More(); it.Next()) {
                add(it.Value(), current.modified);
            }
            for (TopTools_ListIteratorOfListOfShape it(generated_images); it.More(); it.Next()) {
                add(it.Value(), false);
            }
        }

        frontier.swap(next);
        if (frontier.empty()) break;
    }

    result.survivors = std::move(frontier);
    return result;
}

std::vector<TopoDS_Shape> ComposedHistory::publish(const TopoDS_Shape& shape,
                                                   bool want_modified) const {
    if (!answerable(shape)) return {};

    std::vector<Successor> candidates = walk(shape).survivors;
    const TopTools_ListOfShape* injected = injected_.Seek(shape);
    if (want_modified && injected != nullptr) {
        for (TopTools_ListIteratorOfListOfShape it(*injected); it.More(); it.Next()) {
            auto seen = std::find_if(candidates.begin(), candidates.end(),
                                     [&it](const Successor& s) { return s.shape.IsSame(it.Value()); });
            if (seen != candidates.end()) {
                seen->modified = true;
            } else {
                candidates.push_back(Successor{it.Value(), true});
            }
        }
    }

    // Contract (b): present in the final shape, deduplicated, ordinal-sorted.
    std::vector<std::pair<int, TopoDS_Shape>> keyed;
    for (const Successor& candidate : candidates) {
        if (candidate.modified != want_modified) continue;
        // Injection REPLACES the walk's classification for its pair. Promoting a
        // successor to `Modified` on one channel while the walk still publishes it
        // as `Generated` on the other would put one shape on both, which
        // `BRepTools_History` documents as impossible (G(Si) ^ M(Si) == 0) and which
        // would double-count the same candidate in `apply_history`'s split gate.
        if (!want_modified && injected != nullptr && list_contains(*injected, candidate.shape)) {
            continue;
        }
        const int ordinal = final_subs_.FindIndex(candidate.shape);
        if (ordinal == 0) continue;
        const bool duplicate = std::any_of(keyed.begin(), keyed.end(),
                                           [ordinal](const auto& kv) { return kv.first == ordinal; });
        if (!duplicate) keyed.emplace_back(ordinal, candidate.shape);
    }
    std::sort(keyed.begin(), keyed.end(),
              [](const auto& a, const auto& b) { return a.first < b.first; });

    std::vector<TopoDS_Shape> out;
    out.reserve(keyed.size());
    for (const auto& kv : keyed) out.push_back(kv.second);
    return out;
}

std::vector<TopoDS_Shape> ComposedHistory::modified(const TopoDS_Shape& shape) const {
    return publish(shape, /*want_modified=*/true);
}

std::vector<TopoDS_Shape> ComposedHistory::generated(const TopoDS_Shape& shape) const {
    return publish(shape, /*want_modified=*/false);
}

bool ComposedHistory::is_deleted(const TopoDS_Shape& shape) const {
    if (!answerable(shape)) return false;  // contract (a): unanswerable, not deleted
    if (injected_.IsBound(shape) && !injected_.Find(shape).IsEmpty()) return false;
    const Walk result = walk(shape);
    // Contract (c): POSITIVE evidence only. A live survivor that simply failed the
    // final-shape filter leaves `removal_evidence` false, so the caller reaches its
    // no-successor branch and reports NeedsRepair instead of erasing the id.
    return result.survivors.empty() && result.removal_evidence;
}

const TopTools_ListOfShape& ComposedHistory::Modified(const TopoDS_Shape& S) {
    modified_cache_.Clear();
    for (const TopoDS_Shape& s : modified(S)) modified_cache_.Append(s);
    return modified_cache_;
}

const TopTools_ListOfShape& ComposedHistory::Generated(const TopoDS_Shape& S) {
    generated_cache_.Clear();
    for (const TopoDS_Shape& s : generated(S)) generated_cache_.Append(s);
    return generated_cache_;
}

Standard_Boolean ComposedHistory::IsDeleted(const TopoDS_Shape& S) {
    return is_deleted(S) ? Standard_True : Standard_False;
}

}  // namespace onecad::elementmap
