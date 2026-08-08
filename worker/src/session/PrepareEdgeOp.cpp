#include "session/PrepareEdgeOp.h"

#include <algorithm>
#include <iterator>
#include <string>
#include <vector>

#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>

#include "elementmap/ElementMapPartition.h"
#include "kernel/fillet/EdgeContour.h"

namespace onecad::session {

using nlohmann::json;
using protocol::Envelope;
namespace em = onecad::elementmap;
namespace kf = onecad::kernel::fillet;

namespace {

Envelope fail(const Envelope &req, const char *code, const std::string &message) {
  return Envelope::error_response(
      req.id, protocol::ErrorInfo{code, message, false});
}

std::string get_str(const json &value, const char *key) {
  return value.is_object() && value.contains(key) && value[key].is_string()
             ? value[key].get<std::string>()
             : std::string{};
}

json refusal(const char *code, const std::string &message,
             const std::vector<int> &ordinals = {}) {
  json keys = json::array();
  for (const int ordinal : ordinals)
    keys.push_back("e:" + std::to_string(ordinal));
  return {{"code", code}, {"message", message}, {"edges", std::move(keys)}};
}

json response(std::uint64_t snapshot, const std::string &body, json edges,
              json refusal_value) {
  return {{"snapshotId", snapshot},
          {"targetBodyId", body},
          {"edges", std::move(edges)},
          {"refusal", std::move(refusal_value)}};
}

struct Pick {
  std::string body_id;
  int ordinal = 0;
};

Pick resolve_pick(const BodyStore &bodies, const em::ElementMapPartition &part,
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

json edge_entry(const TopoDS_Shape &body, int ordinal, bool picked) {
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

struct ResolvedPicks {
  std::string body_id;
  std::vector<int> ordinals;
  bool unresolved = false;
  bool cross_body = false;
};

ResolvedPicks resolve_picks(const BodyStore &bodies,
                            const em::ElementMapPartition &part,
                            const json &values) {
  ResolvedPicks result;
  for (const json &value : values) {
    const Pick pick = resolve_pick(bodies, part, value);
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

json prepared_result(std::uint64_t snapshot, const std::string &target,
                     const TopoDS_Shape &body, const std::vector<int> &picked,
                     kf::EdgeOpMode mode, bool chain) {
  TopTools_IndexedMapOfShape edge_map;
  TopExp::MapShapes(body, TopAbs_EDGE, edge_map);
  std::vector<TopoDS_Edge> seeds;
  for (const int ordinal : picked)
    seeds.push_back(TopoDS::Edge(edge_map(ordinal)));
  const kf::EdgeContourResult contours =
      kf::analyze_edge_contours(body, seeds, mode);
  if (!contours.ok)
    return response(snapshot, target, json::array(),
                    refusal("unsupportedEdge", contours.message, picked));
  std::vector<int> extra;
  std::set_difference(contours.closure_ordinals.begin(),
                      contours.closure_ordinals.end(), picked.begin(), picked.end(),
                      std::back_inserter(extra));
  json edges = json::array();
  for (const int ordinal : contours.closure_ordinals)
    edges.push_back(edge_entry(
        body, ordinal, std::binary_search(picked.begin(), picked.end(), ordinal)));
  if (!chain && !extra.empty())
    return response(snapshot, target, json::array(),
                    refusal("chainMismatch",
                            "chainTangentEdges:false cannot hold tangent edges fixed",
                            extra));
  return response(snapshot, target, std::move(edges), json(nullptr));
}

} // namespace

Envelope handle_prepare_edge_op(Session &session, const Envelope &req) {
  const json &args = req.args;
  if (!args.contains("snapshotId") ||
      !args["snapshotId"].is_number_unsigned())
    return fail(req, "PROTOCOL_ERROR", "PrepareEdgeOp: snapshotId is required");
  const std::uint64_t requested = args["snapshotId"].get<std::uint64_t>();
  const std::optional<PublishedStateSnapshot> published =
      session.published_state_at(requested);
  if (!published)
    return fail(req, "STALE_PREVIEW", "PrepareEdgeOp: stale snapshot");
  const std::string mode = get_str(args, "mode");
  if (mode != "Fillet" && mode != "Chamfer")
    return fail(req, "PROTOCOL_ERROR", "PrepareEdgeOp: unknown mode");
  if (!args.contains("pickedEdges") || !args["pickedEdges"].is_array() ||
      args["pickedEdges"].empty())
    return fail(req, "PROTOCOL_ERROR", "PrepareEdgeOp: pickedEdges is required");

  const ResolvedPicks picks =
      resolve_picks(published->bodies, published->partition, args["pickedEdges"]);
  if (picks.unresolved)
    return fail(req, "REF_UNRESOLVED", "PrepareEdgeOp: edge did not resolve");
  if (picks.cross_body)
    return Envelope::ok_response(
        req.id, response(published->snapshot_id, "", json::array(),
                         refusal("crossBody", "edge picks span multiple bodies")));
  const bool chain = !args.contains("chainTangentEdges") ||
                     !args["chainTangentEdges"].is_boolean() ||
                     args["chainTangentEdges"].get<bool>();
  const kf::EdgeOpMode op_mode = mode == "Fillet" ? kf::EdgeOpMode::Fillet
                                                   : kf::EdgeOpMode::Chamfer;
  return Envelope::ok_response(
      req.id,
      prepared_result(published->snapshot_id, picks.body_id,
                      published->bodies.get(picks.body_id)->geom, picks.ordinals,
                      op_mode, chain));
}

} // namespace onecad::session
