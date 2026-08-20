#include "session/EdgePicks.h"

#include <algorithm>

#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
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

json edge_evidence_entry(const TopoDS_Shape &body, int ordinal, bool picked) {
  TopTools_IndexedMapOfShape edges;
  TopExp::MapShapes(body, TopAbs_EDGE, edges);
  const TopoDS_Edge edge = TopoDS::Edge(edges(ordinal));
  const em::km::ElementDescriptor descriptor =
      em::ElementMapPartition::describe(edge);
  return {{"topoKey", "e:" + std::to_string(ordinal)},
          {"picked", picked},
          {"anchor",
           {{"worldPoint", {descriptor.center.X(), descriptor.center.Y(),
                             descriptor.center.Z()}}}},
          {"descriptor",
           em::ElementMapPartition::descriptor_to_json(descriptor)}};
}

} // namespace onecad::session
