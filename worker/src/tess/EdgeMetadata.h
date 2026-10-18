#pragma once

#include <cstdint>
#include <vector>

#include <BRepLib.hxx>
#include <BRep_Tool.hxx>
#include <GeomAbs_Shape.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>

namespace onecad::tess {

// Values are the normative MESH1 EDGE_CLASSES codes. Keep uncertain geometry
// visible: only established continuity suppresses an edge in clean display.
inline std::uint32_t classify_edge(const TopoDS_Edge& edge,
                                  const std::vector<TopoDS_Face>& faces) {
    if (BRep_Tool::Degenerated(edge)) return 5;
    if (faces.size() > 2) return 6;
    for (const auto& face : faces) {
        if (BRep_Tool::IsClosed(edge, face)) return 4;
    }
    if (faces.empty()) return 0;
    if (faces.size() == 1) return 1;
    try {
        if (BRep_Tool::Continuity(edge, faces[0], faces[1]) >= GeomAbs_G1) return 3;
        constexpr double kDisplayAngle = 0.001;
        return BRepLib::ContinuityOfFaces(edge, faces[0], faces[1], kDisplayAngle) >= GeomAbs_G1
                   ? 3 : 2;
    } catch (...) {
        return 0;
    }
}

}  // namespace onecad::tess
