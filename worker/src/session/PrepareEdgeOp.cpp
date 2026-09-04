#include "session/PrepareEdgeOp.h"

#include <algorithm>
#include <iterator>
#include <map>
#include <string>
#include <vector>

#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>

#include "kernel/fillet/EdgeContour.h"
#include "session/EdgePicks.h"

namespace onecad::session {

using nlohmann::json;
using protocol::Envelope;
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
  // SCHEMA §7.6 `contour`: the 0-based index of the tangent contour an edge
  // belongs to. `analyze_edge_contours` reports contours in SEED order and the
  // seeds here are `picks.ordinals`, which `resolve_edge_picks` already sorted and
  // de-duplicated — so contour `k` is the one seeded by the k-th smallest picked
  // ordinal, a pure function of the pick set on this snapshot.
  std::map<int, int> contour_of;
  for (std::size_t index = 0; index < contours.contours.size(); ++index)
    for (const int ordinal : contours.contours[index])
      contour_of.emplace(ordinal, static_cast<int>(index));
  // One traversal of the body for the WHOLE closure, not one per edge.
  const EdgeEvidenceMaps maps(body);
  json edges = json::array();
  for (const int ordinal : contours.closure_ordinals) {
    const auto found = contour_of.find(ordinal);
    edges.push_back(edge_evidence_entry(
        maps, ordinal, std::binary_search(picked.begin(), picked.end(), ordinal),
        found == contour_of.end() ? 0 : found->second));
  }
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

  const ResolvedEdgePicks picks =
      resolve_edge_picks(published->bodies, published->partition, args["pickedEdges"]);
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
