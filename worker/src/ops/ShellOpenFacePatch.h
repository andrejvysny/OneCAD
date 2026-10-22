#pragma once

#include <deque>

#include <BRepAdaptor_Surface.hxx>
#include <BRep_Tool.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopTools_MapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <gp_Pln.hxx>

#include "kernel/validation/GeometryPrecision.h"
#include "util/Cancel.h"

namespace onecad::ops::shell_detail {

inline bool planar_frame(const TopoDS_Face& face, gp_Pln& plane, gp_Dir& outward) {
    BRepAdaptor_Surface surface(face, true);
    if (surface.GetType() != GeomAbs_Plane) return false;
    plane = surface.Plane();
    // Reflected planes may carry an indirect Ax3; its Z axis then opposes the
    // surface's parametric normal. Cross the actual surface directions instead.
    outward = plane.Position().XDirection().Crossed(plane.Position().YDirection());
    if (face.Orientation() == TopAbs_REVERSED) outward.Reverse();
    return true;
}

inline bool matches_seed(const TopoDS_Face& face, const gp_Pln& seed_plane,
                         const gp_Dir& seed_outward,
                         const kernel::validation::GeometryPrecisionContext& precision) {
    if (face.Orientation() != TopAbs_FORWARD && face.Orientation() != TopAbs_REVERSED) {
        return false;
    }
    gp_Pln plane;
    gp_Dir outward;
    if (!planar_frame(face, plane, outward) ||
        outward.Angle(seed_outward) > precision.semantic_angular() ||
        seed_plane.Distance(plane.Location()) > precision.semantic_length() ||
        plane.Distance(seed_plane.Location()) > precision.semantic_length()) return false;
    for (TopExp_Explorer it(face, TopAbs_VERTEX); it.More(); it.Next()) {
        if (seed_plane.Distance(BRep_Tool::Pnt(TopoDS::Vertex(it.Current()))) >
            precision.semantic_length()) return false;
    }
    return true;
}

inline bool collect_seed_patch(const TopoDS_Face& seed,
                               const TopTools_IndexedMapOfShape& body_faces,
                               const TopTools_IndexedDataMapOfShapeListOfShape& adjacency,
                               const kernel::validation::GeometryPrecisionContext& precision,
                               const CancelToken* cancel, TopTools_MapOfShape& selected) {
    gp_Pln plane;
    gp_Dir outward;
    if (!planar_frame(seed, plane, outward)) {
        selected.Add(seed);
        return true;
    }
    TopTools_MapOfShape visited;
    std::deque<TopoDS_Face> frontier{seed};
    visited.Add(seed);
    while (!frontier.empty()) {
        if (cancel && cancel->cancelled()) return false;
        const auto current = frontier.front();
        frontier.pop_front();
        selected.Add(current);
        for (TopExp_Explorer edge(current, TopAbs_EDGE); edge.More(); edge.Next()) {
            const int index = adjacency.FindIndex(edge.Current());
            if (!index) continue;
            for (TopTools_ListIteratorOfListOfShape it(adjacency(index)); it.More(); it.Next()) {
                const int face_index = body_faces.FindIndex(it.Value());
                if (!face_index) return false;
                const auto neighbor = TopoDS::Face(body_faces(face_index));
                if (visited.Add(neighbor) && matches_seed(neighbor, plane, outward, precision)) {
                    frontier.push_back(neighbor);
                }
            }
        }
    }
    return true;
}

// Expand only resolved original faces. The projection patch helper deliberately
// accepts opposite normals and looser tolerances; those semantics would open
// unrelated walls here. Each candidate is compared with its seed, never its neighbor.
inline bool expand_open_faces(const TopoDS_Shape& body, const TopTools_ListOfShape& seeds,
                              const CancelToken* cancel, TopTools_ListOfShape& expanded) {
    if (cancel && cancel->cancelled()) return false;
    try {
        TopTools_IndexedMapOfShape body_faces;
        TopExp::MapShapes(body, TopAbs_FACE, body_faces);
        TopTools_IndexedDataMapOfShapeListOfShape adjacency;
        TopExp::MapShapesAndAncestors(body, TopAbs_EDGE, TopAbs_FACE, adjacency);
        const auto precision = kernel::validation::precision_of(body);
        TopTools_MapOfShape selected;
        for (TopTools_ListIteratorOfListOfShape it(seeds); it.More(); it.Next()) {
            const int index = body_faces.FindIndex(it.Value());
            if (!index || !body_faces(index).IsEqual(it.Value())) return false;
            const auto seed = TopoDS::Face(body_faces(index));
            if (seed.Orientation() != TopAbs_FORWARD && seed.Orientation() != TopAbs_REVERSED) {
                return false;
            }
            if (!collect_seed_patch(seed, body_faces, adjacency, precision, cancel, selected)) {
                return false;
            }
        }
        TopTools_ListOfShape result;
        for (int i = 1; i <= body_faces.Extent(); ++i) {
            if (selected.Contains(body_faces(i))) result.Append(body_faces(i));
        }
        expanded = result;
        return !expanded.IsEmpty();
    } catch (...) {
        return false;
    }
}

} // namespace onecad::ops::shell_detail
