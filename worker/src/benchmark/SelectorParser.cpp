#include "benchmark/SelectorParser.h"

#include <algorithm>
#include <cmath>
#include <set>

#include <nlohmann/json.hpp>

namespace onecad::benchmark {
namespace {

using json = nlohmann::json;

bool exact_fields(const json &value, const std::set<std::string> &fields,
                  std::string &error) {
  if (!value.is_object()) {
    error = "selector block must be an object";
    return false;
  }
  for (auto it = value.begin(); it != value.end(); ++it) {
    if (fields.count(it.key()) == 0) {
      error = "unknown selector field: " + it.key();
      return false;
    }
  }
  for (const std::string &field : fields) {
    if (!value.contains(field)) {
      error = "missing selector field: " + field;
      return false;
    }
  }
  return true;
}

bool vector3(const json &value, std::vector<double> &out) {
  if (!value.is_array() || value.size() != 3)
    return false;
  for (const json &item : value) {
    if (!item.is_number())
      return false;
    const double number = item.get<double>();
    if (!std::isfinite(number) || std::abs(number) > 1.0e12)
      return false;
    out.push_back(number);
  }
  return true;
}

bool parse_provenance(const json &value, SelectorSpec &out,
                      std::string &error) {
  const std::set<std::string> fields = {"generator", "recipeType", "featureIndex"};
  if (!exact_fields(value, fields, error) || !value["generator"].is_string() ||
      !value["recipeType"].is_string() ||
      !value["featureIndex"].is_number_unsigned() ||
      value["featureIndex"].get<std::size_t>() > 65535)
    return false;
  out.provenance_generator = value["generator"];
  out.provenance_recipe_type = value["recipeType"];
  out.provenance_feature_index = value["featureIndex"];
  return true;
}

bool parse_anchors(const json &value, SelectorSpec &out, std::string &error) {
  if (!value.is_array() || value.empty() || value.size() > 64)
    return false;
  const std::set<std::string> fields = {"kind", "point", "frame"};
  for (const json &anchor : value) {
    std::vector<double> point;
    if (!exact_fields(anchor, fields, error) || !anchor["kind"].is_string() ||
        (anchor["kind"] != "edgeMidpoint" && anchor["kind"] != "commonVertex") ||
        anchor["frame"] != "recipeLocal" || !vector3(anchor["point"], point))
      return false;
    out.anchor_kinds.push_back(anchor["kind"]);
    out.anchor_points.push_back(std::move(point));
  }
  return true;
}

bool parse_surfaces(const json &value, SelectorSpec &out,
                    std::string &error) {
  if (!value.is_array() || value.empty() || value.size() > 64)
    return false;
  const std::set<std::string> fields = {"curveKind", "adjacentSurfaceKinds"};
  const std::set<std::string> curves = {"line", "analyticCurve"};
  const std::set<std::string> surfaces = {"plane", "cylinder", "cone", "other"};
  for (const json &descriptor : value) {
    if (!exact_fields(descriptor, fields, error) ||
        !descriptor["curveKind"].is_string() ||
        curves.count(descriptor["curveKind"].get<std::string>()) == 0 ||
        !descriptor["adjacentSurfaceKinds"].is_array() ||
        descriptor["adjacentSurfaceKinds"].size() != 2)
      return false;
    std::vector<std::string> adjacent;
    for (const json &surface : descriptor["adjacentSurfaceKinds"]) {
      if (!surface.is_string() || surfaces.count(surface.get<std::string>()) == 0)
        return false;
      adjacent.push_back(surface);
    }
    out.curve_kinds.push_back(descriptor["curveKind"]);
    out.adjacent_surface_kinds.push_back(std::move(adjacent));
  }
  return true;
}

bool parse_adjacency(const json &value, SelectorSpec &out,
                     std::string &error) {
  if (!exact_fields(value, {"relation"}, error) || !value["relation"].is_string())
    return false;
  const std::set<std::string> relations = {"single", "pairwiseDisconnected",
                                            "unconstrained", "incidentToCommonVertex"};
  out.adjacency_relation = value["relation"];
  return relations.count(out.adjacency_relation) > 0;
}

} // namespace

bool parse_selector(const json &value, SelectorSpec &out, std::string &error) {
  const std::set<std::string> fields = {"mode", "topologyRole", "provenance",
      "anchors", "surfaceDescriptors", "adjacency"};
  const std::set<std::string> modes = {"single", "disconnected", "multiple",
                                        "cornerIncident"};
  const std::set<std::string> roles = {"verticalEdge", "cornerIncidentEdge",
                                        "overflowEdge"};
  if (!exact_fields(value, fields, error) || !value["mode"].is_string() ||
      !value["topologyRole"].is_string())
    return false;
  out.mode = value["mode"];
  out.topology_role = value["topologyRole"];
  if (modes.count(out.mode) == 0 || roles.count(out.topology_role) == 0)
    return false;
  return parse_provenance(value["provenance"], out, error) &&
         parse_anchors(value["anchors"], out, error) &&
         parse_surfaces(value["surfaceDescriptors"], out, error) &&
         parse_adjacency(value["adjacency"], out, error);
}

bool selector_matches_case(const SelectorSpec &selector,
                           const std::string &generator,
                           const std::string &recipe_type,
                           std::size_t feature_index, std::string &error) {
  if (selector.provenance_generator != generator ||
      selector.provenance_recipe_type != recipe_type ||
      selector.provenance_feature_index != feature_index) {
    error = "selector provenance does not match generator recipe";
    return false;
  }
  const bool corner = selector.mode == "cornerIncident";
  const bool common_anchor = selector.anchor_kinds.size() == 1 &&
                             selector.anchor_kinds.front() == "commonVertex";
  if (corner != common_anchor ||
      (corner && selector.topology_role != "cornerIncidentEdge")) {
    error = "selector anchor does not match topology mode";
    return false;
  }
  if (!corner && (selector.anchor_points.size() != selector.curve_kinds.size() ||
                  std::any_of(selector.anchor_kinds.begin(), selector.anchor_kinds.end(),
                              [](const std::string &kind) {
                                return kind != "edgeMidpoint";
                              }))) {
    error = "edge selector descriptors do not match anchors";
    return false;
  }
  const std::string expected_role = recipe_type == "overflowWedge" ? "overflowEdge" :
      recipe_type.find("Corner") != std::string::npos ? "cornerIncidentEdge" :
                                                        "verticalEdge";
  if (selector.topology_role != expected_role) {
    error = "selector topology role does not match recipe";
    return false;
  }
  const std::string expected_relation = selector.mode == "single" ? "single" :
      selector.mode == "disconnected" ? "pairwiseDisconnected" :
      selector.mode == "cornerIncident" ? "incidentToCommonVertex" : "unconstrained";
  if (selector.adjacency_relation != expected_relation) {
    error = "selector adjacency does not match mode";
    return false;
  }
  return true;
}

} // namespace onecad::benchmark
