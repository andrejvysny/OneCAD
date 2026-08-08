#pragma once

#include <cmath>
#include <string>
#include <vector>

#include <BRepPrimAPI_MakeBox.hxx>
#include <BRep_Tool.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Vertex.hxx>

#include "elementmap/ElementMapPartition.h"
#include "kernel/fillet/FilletAnalyzer.h"
#include "nlohmann/json.hpp"

namespace onecad::tests::fillet {

namespace em = onecad::elementmap;
namespace kf = onecad::kernel::fillet;
namespace km = onecad::kernel::elementmap;

inline TopoDS_Shape box(double size = 10.0) {
  return BRepPrimAPI_MakeBox(size, size, size).Shape();
}

inline std::vector<TopoDS_Edge> all_edges(const TopoDS_Shape &shape) {
  TopTools_IndexedMapOfShape map;
  TopExp::MapShapes(shape, TopAbs_EDGE, map);
  std::vector<TopoDS_Edge> edges;
  for (int i = 1; i <= map.Extent(); ++i)
    edges.push_back(TopoDS::Edge(map(i)));
  return edges;
}

inline std::vector<TopoDS_Edge> vertical_edges(const TopoDS_Shape &shape) {
  std::vector<TopoDS_Edge> out;
  for (const TopoDS_Edge &edge : all_edges(shape)) {
    TopoDS_Vertex first;
    TopoDS_Vertex last;
    TopExp::Vertices(edge, first, last);
    if (first.IsNull() || last.IsNull())
      continue;
    const gp_Pnt a = BRep_Tool::Pnt(first);
    const gp_Pnt b = BRep_Tool::Pnt(last);
    if (std::abs(a.X() - b.X()) <= 1e-9 && std::abs(a.Y() - b.Y()) <= 1e-9 &&
        std::abs(a.Z() - b.Z()) > 1e-9) {
      out.push_back(edge);
    }
  }
  return out;
}

inline kf::ResolvedEdge resolved(const TopoDS_Shape &body,
                                 const TopoDS_Edge &edge,
                                 std::size_t index = 0) {
  return {"el_" + std::to_string(index),
          em::ElementMapPartition::topokey_for_shape(body, edge,
                                                     km::ElementKind::Edge),
          "op_fillet.input" + std::to_string(index), "body_box", edge};
}

inline std::vector<kf::ResolvedEdge>
resolved(const TopoDS_Shape &body, const std::vector<TopoDS_Edge> &edges) {
  std::vector<kf::ResolvedEdge> out;
  for (std::size_t i = 0; i < edges.size(); ++i)
    out.push_back(resolved(body, edges[i], i));
  return out;
}

inline nlohmann::json input(const TopoDS_Shape &, const TopoDS_Edge &edge,
                            const std::string &element_id) {
  const auto descriptor = em::ElementMapPartition::describe(edge);
  return {
      {"primary",
       {{"bodyId", "body_box"}, {"elementId", element_id}, {"kind", "edge"}}},
      {"intent",
       {{"kind", "edge"},
        {"descriptor",
         em::ElementMapPartition::descriptor_to_json(descriptor)}}},
      {"anchor",
       {{"worldPoint",
         {descriptor.center.X(), descriptor.center.Y(),
          descriptor.center.Z()}}}}};
}

inline nlohmann::json op(const TopoDS_Shape &body,
                         const std::vector<TopoDS_Edge> &edges, double radius) {
  nlohmann::json inputs = nlohmann::json::array();
  nlohmann::json edge_ids = nlohmann::json::array();
  for (std::size_t i = 0; i < edges.size(); ++i) {
    const std::string id = "el_" + std::to_string(i);
    inputs.push_back(input(body, edges[i], id));
    edge_ids.push_back(id);
  }
  return {{"opType", "Fillet"},
          {"opId", "op_fillet"},
          {"stepIndex", 0},
          {"inputs", std::move(inputs)},
          {"params",
           {{"mode", "Fillet"},
            {"radius", radius},
            {"edgeIds", std::move(edge_ids)}}}};
}

} // namespace onecad::tests::fillet
