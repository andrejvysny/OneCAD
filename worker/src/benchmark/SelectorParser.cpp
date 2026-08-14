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
  out.has_provenance_feature_index = true;
  return true;
}

bool parse_provenance_v2(const json &value, SelectorSpec &out,
                         std::string &error) {
  const std::set<std::string> allowed = {"generator", "recipe", "featureIndex"};
  if (!value.is_object()) {
    error = "selector provenance must be an object";
    return false;
  }
  for (auto it = value.begin(); it != value.end(); ++it) {
    if (allowed.count(it.key()) == 0) {
      error = "unknown selector provenance field: " + it.key();
      return false;
    }
  }
  if (!value.contains("generator") || !value["generator"].is_string() ||
      !value.contains("recipe") || !value["recipe"].is_string()) {
    error = "selector provenance requires generator and recipe";
    return false;
  }
  out.provenance_generator = value["generator"];
  out.provenance_recipe_type = value["recipe"];
  if (value.contains("featureIndex")) {
    if (!value["featureIndex"].is_number_unsigned() ||
        value["featureIndex"].get<std::size_t>() > 65535) {
      error = "selector provenance featureIndex out of range";
      return false;
    }
    out.provenance_feature_index = value["featureIndex"];
    out.has_provenance_feature_index = true;
  }
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

bool parse_surfaces_v2(const json &value, SelectorSpec &out,
                       std::string &error) {
  if (!value.is_array() || value.empty() || value.size() > 64) {
    error = "selector needs 1..64 surfaceDescriptors";
    return false;
  }
  const std::set<std::string> fields = {"curveKind", "adjacentSurfaceKinds"};
  const std::set<std::string> curves = {"line", "analyticCurve", "bsplineCurve"};
  const std::set<std::string> surfaces = {"plane", "cylinder", "cone", "sphere",
                                          "torus", "bspline", "other"};
  for (const json &descriptor : value) {
    if (!exact_fields(descriptor, fields, error) ||
        !descriptor["curveKind"].is_string() ||
        curves.count(descriptor["curveKind"].get<std::string>()) == 0 ||
        !descriptor["adjacentSurfaceKinds"].is_array() ||
        descriptor["adjacentSurfaceKinds"].size() != 2) {
      error = error.empty() ? "invalid surface descriptor" : error;
      return false;
    }
    std::vector<std::string> adjacent;
    for (const json &surface : descriptor["adjacentSurfaceKinds"]) {
      if (!surface.is_string() ||
          surfaces.count(surface.get<std::string>()) == 0) {
        error = "unsupported adjacent surface kind";
        return false;
      }
      adjacent.push_back(surface);
    }
    out.curve_kinds.push_back(descriptor["curveKind"]);
    out.adjacent_surface_kinds.push_back(std::move(adjacent));
  }
  return true;
}

bool parse_adjacency_v2(const json &value, SelectorSpec &out,
                        std::string &error) {
  if (!exact_fields(value, {"relation"}, error) || !value["relation"].is_string())
    return false;
  const std::set<std::string> relations = {
      "single",       "pairwiseDisconnected", "unconstrained",
      "tangentChain", "closedLoop",           "incidentToCommonVertex"};
  out.adjacency_relation = value["relation"];
  if (relations.count(out.adjacency_relation) == 0) {
    error = "unsupported selector adjacency relation";
    return false;
  }
  return true;
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

bool parse_selector_v2(const json &value, SelectorSpec &out,
                       std::string &error) {
  if (value.is_object() && value.value("mode", "") == "bodyRoles") {
    const std::set<std::string> fields = {"mode", "target", "tools"};
    if (!exact_fields(value, fields, error) || value["target"] != "target" ||
        !value["tools"].is_array() || value["tools"].size() != 1 ||
        value["tools"][0] != "tool") {
      error = error.empty() ? "bodyRoles needs target and exactly one tool" : error;
      return false;
    }
    out.body_roles = true;
    out.mode = "bodyRoles";
    out.target_role = "target";
    out.tool_roles = {"tool"};
    return true;
  }
  const std::set<std::string> allowed = {
      "mode",       "topologyRole", "convexity",          "vertexValence",
      "provenance", "anchors",      "surfaceDescriptors", "adjacency"};
  const std::set<std::string> required = {"mode", "topologyRole", "provenance",
                                          "anchors", "surfaceDescriptors",
                                          "adjacency"};
  if (!value.is_object()) {
    error = "selector must be an object";
    return false;
  }
  for (auto it = value.begin(); it != value.end(); ++it) {
    if (allowed.count(it.key()) == 0) {
      error = "unknown selector field: " + it.key();
      return false;
    }
  }
  for (const std::string &field : required) {
    if (!value.contains(field)) {
      error = "missing selector field: " + field;
      return false;
    }
  }
  const std::set<std::string> modes = {"single",  "disconnected",  "multiple",
                                       "chain",   "closedContour", "cornerIncident"};
  const std::set<std::string> roles = {"verticalEdge", "cornerIncidentEdge",
                                       "overflowEdge", "supportPairEdge",
                                       "seamEdge",     "shortEdge"};
  if (!value["mode"].is_string() || !value["topologyRole"].is_string()) {
    error = "selector mode and topologyRole must be strings";
    return false;
  }
  out.mode = value["mode"];
  out.topology_role = value["topologyRole"];
  if (modes.count(out.mode) == 0 || roles.count(out.topology_role) == 0) {
    error = "unsupported selector mode or topology role";
    return false;
  }
  if (value.contains("convexity")) {
    const std::set<std::string> convexities = {"convex", "concave", "mixed"};
    if (!value["convexity"].is_string() ||
        convexities.count(value["convexity"].get<std::string>()) == 0) {
      error = "unsupported selector convexity";
      return false;
    }
    out.convexity = value["convexity"];
  }
  if (value.contains("vertexValence")) {
    if (!value["vertexValence"].is_number_unsigned() ||
        value["vertexValence"].get<int>() < 3 ||
        value["vertexValence"].get<int>() > 8) {
      error = "selector vertexValence must be in 3..8";
      return false;
    }
    out.vertex_valence = value["vertexValence"].get<int>();
  }
  return parse_provenance_v2(value["provenance"], out, error) &&
         parse_anchors(value["anchors"], out, error) &&
         parse_surfaces_v2(value["surfaceDescriptors"], out, error) &&
         parse_adjacency_v2(value["adjacency"], out, error);
}

bool selector_matches_case_v2(const SelectorSpec &selector,
                              const std::string &generator,
                              const std::string &recipe,
                              bool has_feature_index,
                              std::size_t feature_index, std::string &error) {
  if (selector.body_roles)
    return generator == "boolean-foundation" && recipe == "twoBoxes";
  if (selector.provenance_generator != generator ||
      selector.provenance_recipe_type != recipe) {
    error = "selector provenance does not match generator recipe";
    return false;
  }
  // Only cross-checked when BOTH sides carry one; the recipes that have no
  // feature index simply omit it on both sides.
  if (has_feature_index && selector.has_provenance_feature_index &&
      selector.provenance_feature_index != feature_index) {
    error = "selector provenance featureIndex does not match geometry";
    return false;
  }
  // Mode fixes both the anchor count and the adjacency relation. Keeping them
  // in one table is what stops a case from claiming `chain` while carrying a
  // disconnected adjacency, which would benchmark a different selection than
  // the case names.
  std::size_t minimum = 1;
  std::size_t maximum = 1;
  std::string relation = "single";
  if (selector.mode == "disconnected") {
    minimum = 2, maximum = 64, relation = "pairwiseDisconnected";
  } else if (selector.mode == "multiple") {
    minimum = 2, maximum = 64, relation = "unconstrained";
  } else if (selector.mode == "chain") {
    minimum = 2, maximum = 64, relation = "tangentChain";
  } else if (selector.mode == "closedContour") {
    minimum = 3, maximum = 64, relation = "closedLoop";
  } else if (selector.mode == "cornerIncident") {
    relation = "incidentToCommonVertex";
  }
  if (selector.anchor_points.size() < minimum ||
      selector.anchor_points.size() > maximum) {
    error = "anchor count does not match selector mode";
    return false;
  }
  if (selector.adjacency_relation != relation) {
    error = "selector adjacency does not match mode";
    return false;
  }
  if (selector.mode == "cornerIncident") {
    if (selector.topology_role != "cornerIncidentEdge") {
      error = "cornerIncident selection must use the cornerIncidentEdge role";
      return false;
    }
    if (selector.anchor_kinds.front() != "commonVertex") {
      error = "cornerIncident selection anchors on the common vertex";
      return false;
    }
    return true;
  }
  for (const std::string &kind : selector.anchor_kinds) {
    if (kind != "edgeMidpoint") {
      error = "edge selection anchors on edge midpoints";
      return false;
    }
  }
  if (selector.anchor_points.size() != selector.curve_kinds.size()) {
    error = "edge selector descriptors do not match anchors";
    return false;
  }
  return true;
}

} // namespace onecad::benchmark
