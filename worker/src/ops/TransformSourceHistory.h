#pragma once

#include <algorithm>
#include <BRepBuilderAPI_Transform.hxx>
#include <TopAbs_ShapeEnum.hxx>

#include "session/TopologyOrigins.h"

namespace onecad::ops {

// The general history classifier indexes shapes without orientation. Certified
// source faces additionally require the transform builder's exact oriented image.
inline void certify_oriented_face_history(session::TopologyBodyHistory& history,
                                   BRepBuilderAPI_Transform& builder) {
    auto invalid_survivor = [&](const session::TopologySurvivor& survivor) {
        if (survivor.after.ShapeType() != TopAbs_FACE || survivor.before.IsEqual(survivor.after))
            return false;
        history.conflicts.push_back({survivor.after, {survivor.before}});
        return true;
    };
    auto& survivors = history.survivors;
    survivors.erase(std::remove_if(survivors.begin(), survivors.end(), invalid_survivor),
                    survivors.end());
    auto invalid = [&](const session::TopologySuccessor& successor) {
        if (successor.after.ShapeType() != TopAbs_FACE) return false;
        bool proven = successor.before.size() == 1;
        if (proven) {
            const auto& input = successor.before.front();
            std::size_t count = 0;
            for (const TopoDS_Shape& image : builder.Modified(input)) {
                ++count;
                if (!image.IsEqual(successor.after)) proven = false;
            }
            const TopoDS_Shape image = builder.ModifiedShape(input);
            proven = proven && count == 1 && !image.IsNull() && image.IsEqual(successor.after);
        }
        if (!proven) history.conflicts.push_back({successor.after, successor.before});
        return !proven;
    };
    auto& successors = history.successors;
    successors.erase(std::remove_if(successors.begin(), successors.end(), invalid), successors.end());
}

}  // namespace onecad::ops
