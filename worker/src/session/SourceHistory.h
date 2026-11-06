#pragma once

#include <optional>
#include <vector>

#include <TopExp_Explorer.hxx>

#include "session/BodyStore.h"
#include "session/Signatures.h"
#include "session/TopologyOrigins.h"

namespace onecad::session {

inline const TopologyBodyHistory* unique_source_history(
    const std::vector<TopologyBodyHistory>& histories, const std::string& body) {
    const TopologyBodyHistory* result = nullptr;
    for (const auto& history : histories) {
        if (history.body_id != body) continue;
        if (result) return nullptr;
        result = &history;
    }
    return result && result->coverage == HistoryCoverage::Proven ? result : nullptr;
}

inline std::optional<TopoDS_Shape> source_history_image(const TopologyBodyHistory& history,
                                                        const TopoDS_Shape& before,
                                                        std::size_t& visits) {
    constexpr std::size_t kMaxVisits = 10000000;
    for (const auto& conflict : history.conflicts) {
        if (conflict.before.empty()) return {};
        for (const auto& prior : conflict.before) {
            if (++visits > kMaxVisits || (!prior.IsNull() && prior.IsSame(before))) return {};
        }
    }
    TopoDS_Shape image;
    std::size_t forward = 0;
    const auto consider = [&](const TopoDS_Shape& prior, const TopoDS_Shape& after) {
        if (++visits > kMaxVisits) return false;
        if (!prior.IsNull() && prior.IsEqual(before)) { ++forward; image = after; }
        return true;
    };
    for (const auto& edge : history.survivors) if (!consider(edge.before, edge.after)) return {};
    for (const auto& edge : history.successors) {
        for (const auto& prior : edge.before) if (!consider(prior, edge.after)) return {};
    }
    if (forward != 1 || image.IsNull() || image.ShapeType() != TopAbs_FACE) return {};
    std::size_t reverse = 0;
    for (const auto& edge : history.survivors) {
        if (++visits > kMaxVisits) return {};
        if (!edge.after.IsNull() && edge.after.IsSame(image)) ++reverse;
    }
    for (const auto& edge : history.successors) {
        if (++visits > kMaxVisits) return {};
        if (!edge.after.IsNull() && edge.after.IsSame(image)) reverse += edge.before.size();
    }
    for (const auto& birth : history.births) {
        if (++visits > kMaxVisits) return {};
        if (!birth.after.IsNull() && birth.after.IsSame(image)) return {};
    }
    for (const auto& conflict : history.conflicts) {
        if (++visits > kMaxVisits) return {};
        if (!conflict.after.IsNull() && conflict.after.IsSame(image)) return {};
    }
    return reverse == 1 ? std::optional<TopoDS_Shape>(image) : std::nullopt;
}

inline bool source_image_is_live(const BodyStore& bodies, const std::string& body,
                                 const TopoDS_Shape& image, std::size_t& visits) {
    const BodyRecord* record = bodies.get(body);
    if (!record) return false;
    std::size_t count = 0;
    for (TopExp_Explorer face(record->geom, TopAbs_FACE); face.More(); face.Next()) {
        if (++visits > 10000000) return false;
        if (face.Current().IsEqual(image)) ++count;
    }
    return count == 1;
}

inline bool apply_source_history(SourceLedger& ledger, const BodyStore& bodies,
                                 const std::vector<BodyEvent>& events,
                                 const std::vector<TopologyBodyHistory>& histories) {
    bool has_touched = false;
    for (const auto& row : ledger.rows()) {
        for (const auto& event : events) {
            if (row.birth->body_id == event.body_id) has_touched = true;
        }
    }
    if (!has_touched) return true;
    std::size_t visits = 0;
    return ledger.remap([&](const SourceLedgerRow& row) -> std::optional<TopoDS_Shape> {
        const auto& body = row.birth->body_id;
        bool touched = false;
        for (const auto& event : events) {
            if (event.body_id != body) continue;
            if (event.kind == "split" || event.kind == "merged" || event.kind == "deleted") return {};
            touched = true;
        }
        if (!touched) return row.live_face;
        const auto* history = unique_source_history(histories, body);
        if (!history) return {};
        const auto image = source_history_image(*history, row.live_face, visits);
        if (!image || !source_image_is_live(bodies, body, *image, visits)) return {};
        return image;
    });
}

}  // namespace onecad::session
