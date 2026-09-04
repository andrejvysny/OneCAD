#include "session/EdgePicks.h"

#include <algorithm>

#include <TopExp.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>

namespace onecad::session {

using nlohmann::json;
namespace em = onecad::elementmap;

namespace {

std::string get_str(const json &value, const char *key) {
  return value.is_object() && value.contains(key) && value[key].is_string()
             ? value[key].get<std::string>()
             : std::string{};
}

} // namespace

EdgePick resolve_edge_pick(const BodyStore &bodies,
                           const em::ElementMapPartition &part,
                           const json &value) {
  std::string body = get_str(value, "bodyId");
  std::string topo = get_str(value, "topoKey");
  const std::string element = get_str(value, "elementId");
  if (!element.empty()) {
    const em::PartitionEntry *entry = part.find(element);
    if (!entry)
      return {};
    body = entry->body_id;
    topo = entry->topo_key;
  }
  const BodyRecord *record = bodies.get(body);
  if (!record || topo.empty())
    return {};
  const TopoDS_Shape shape =
      em::ElementMapPartition::shape_for_topokey(record->geom, topo);
  if (shape.IsNull() || shape.ShapeType() != TopAbs_EDGE)
    return {};
  TopTools_IndexedMapOfShape edges;
  TopExp::MapShapes(record->geom, TopAbs_EDGE, edges);
  return {body, edges.FindIndex(shape)};
}

ResolvedEdgePicks resolve_edge_picks(const BodyStore &bodies,
                                     const em::ElementMapPartition &part,
                                     const json &values) {
  ResolvedEdgePicks result;
  for (const json &value : values) {
    const EdgePick pick = resolve_edge_pick(bodies, part, value);
    if (pick.body_id.empty() || pick.ordinal <= 0) {
      result.unresolved = true;
      return result;
    }
    if (result.body_id.empty())
      result.body_id = pick.body_id;
    if (result.body_id != pick.body_id) {
      result.cross_body = true;
      return result;
    }
    result.ordinals.push_back(pick.ordinal);
  }
  std::sort(result.ordinals.begin(), result.ordinals.end());
  result.ordinals.erase(
      std::unique(result.ordinals.begin(), result.ordinals.end()),
      result.ordinals.end());
  return result;
}

EdgeEvidenceMaps::EdgeEvidenceMaps(const TopoDS_Shape &body) {
  TopExp::MapShapes(body, TopAbs_EDGE, edges);
  TopExp::MapShapesAndAncestors(body, TopAbs_EDGE, TopAbs_FACE, edge_faces);
  TopExp::MapShapes(body, TopAbs_FACE, faces);
}

std::vector<int> adjacent_face_ordinals(
    const TopTools_IndexedDataMapOfShapeListOfShape &edge_faces,
    const TopTools_IndexedMapOfShape &face_map, const TopoDS_Edge &edge) {
  const int index = edge_faces.FindIndex(edge);
  if (index == 0)
    return {};
  std::vector<int> ordinals;
  for (TopTools_ListOfShape::Iterator it(edge_faces(index)); it.More();
       it.Next()) {
    const int ordinal = face_map.FindIndex(it.Value());
    if (ordinal > 0)
      ordinals.push_back(ordinal);
  }
  std::sort(ordinals.begin(), ordinals.end());
  ordinals.erase(std::unique(ordinals.begin(), ordinals.end()), ordinals.end());
  return ordinals;
}

std::vector<int> adjacent_face_ordinals(const TopoDS_Shape &body,
                                        const TopoDS_Edge &edge) {
  const EdgeEvidenceMaps maps(body);
  return adjacent_face_ordinals(maps.edge_faces, maps.faces, edge);
}

json edge_evidence_entry(const EdgeEvidenceMaps &maps, int ordinal, bool picked,
                         int contour) {
  const TopoDS_Edge edge = TopoDS::Edge(maps.edges(ordinal));
  const em::km::ElementDescriptor descriptor =
      em::ElementMapPartition::describe(edge);
  json adjacent = json::array();
  for (const int face_ordinal :
       adjacent_face_ordinals(maps.edge_faces, maps.faces, edge))
    adjacent.push_back("f:" + std::to_string(face_ordinal));
  return {{"topoKey", "e:" + std::to_string(ordinal)},
          {"picked", picked},
          {"anchor",
           {{"worldPoint", {descriptor.center.X(), descriptor.center.Y(),
                             descriptor.center.Z()}}}},
          {"descriptor",
           em::ElementMapPartition::descriptor_to_json(descriptor)},
          {"contour", contour},
          {"adjacentFaces", std::move(adjacent)}};
}

} // namespace onecad::session
