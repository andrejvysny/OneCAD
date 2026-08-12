#include "benchmark/CaseParser.h"

#include <cmath>
#include <regex>
#include <set>

#include <nlohmann/json.hpp>

#include "benchmark/CaseParserShared.h"
#include "benchmark/SelectorParser.h"

namespace onecad::benchmark {
namespace {

using json = nlohmann::json;
using parser::bounded_number;
using parser::exact_fields;
using parser::finite_number;
using parser::parse_string_array;
using parser::parse_vector;
using parser::safe_id;

bool parse_generator(const json &value, GeneratorSpec &out,
                     std::string &error) {
  const std::set<std::string> fields = {"name", "version", "seed"};
  if (!exact_fields(value, fields, fields, error))
    return false;
  if (!value["name"].is_string() || value["name"].get<std::string>() != "fillet-foundation" ||
      !value["version"].is_number_integer() || value["version"].get<int>() != 1 ||
      !value["seed"].is_string()) {
    error = "unsupported generator name or version";
    return false;
  }
  out = {"fillet", value["name"], value["version"], value["seed"]};
  if (!std::regex_match(out.seed, std::regex("^[0-9a-f]{16}$"))) {
    error = "generator seed must be 16 lowercase hex digits";
    return false;
  }
  return true;
}

bool parse_recipe(const json &value, CaseSpec &out, std::string &error) {
  const std::set<std::string> fields = {"type", "dimensions", "featureIndex"};
  if (!exact_fields(value, fields, fields, error) || !value["type"].is_string() ||
      !value["featureIndex"].is_number_unsigned()) {
    error = error.empty() ? "invalid geometry recipe" : error;
    return false;
  }
  out.recipe = value["type"];
  const std::set<std::string> supported = {"box", "valence3Corner",
                                            "valence4Corner", "overflowWedge"};
  if (supported.count(out.recipe) == 0 ||
      !parse_vector(value["dimensions"], 3, 3, out.parameters.dimensions)) {
    error = "unsupported or invalid geometry recipe";
    return false;
  }
  for (double dimension : out.parameters.dimensions) {
    if (dimension <= 1.0e-6) {
      error = "geometry dimensions must be positive";
      return false;
    }
  }
  out.parameters.feature_index = value["featureIndex"].get<std::size_t>();
  out.parameters.has_feature_index = true;
  return out.parameters.feature_index <= 65535
             ? true
             : (error = "featureIndex exceeds limit", false);
}

bool parse_operation(const json &value, CaseSpec &out, std::string &error) {
  const std::set<std::string> fields = {"type", "radius", "continuity",
                                         "radiusMode", "selector"};
  if (!exact_fields(value, fields, fields, error) || !value["type"].is_string() ||
      value["type"] != "fillet" || !value["continuity"].is_string() ||
      value["continuity"] != "g1" || !value["radiusMode"].is_string() ||
      value["radiusMode"] != "constant" ||
      !finite_number(value["radius"], 1.0e-12, 1.0e12, out.radius)) {
    error = error.empty() ? "operation must be a finite constant-radius G1 fillet" : error;
    return false;
  }
  out.radius_law.mode = "constant";
  out.radius_law.radius = out.radius;
  return parse_selector(value["selector"], out.selector, error);
}

bool validate_validator(const json &value, std::string &error) {
  if (!value.is_object() || !value.contains("type") || !value["type"].is_string() ||
      !value.contains("required") || !value["required"].is_boolean()) {
    error = "invalid validator";
    return false;
  }
  const std::string type = value["type"];
  const std::set<std::string> simple = {"constantRadius", "generatedBlendFace",
      "cylindricalRadius", "g1BoundaryTangency", "materialChange",
      "remoteSupportsUnchanged", "deepAudit"};
  if (simple.count(type) > 0) {
    if (!exact_fields(value, {"type", "required", "direction"},
                      {"type", "required"}, error))
      return false;
    return !value.contains("direction") ||
           (value["direction"].is_string() &&
            (value["direction"] == "decrease" || value["direction"] == "increase"));
  }
  if (type == "radiusTolerance" || type == "tangencyTolerance" ||
      type == "materialTolerance") {
    const std::set<std::string> fields = {"type", "required", "absolute", "relative"};
    return exact_fields(value, fields, fields, error) &&
           bounded_number(value["absolute"], 0.0, 1.0e12) &&
           bounded_number(value["relative"], 0.0, 1.0);
  }
  const std::set<std::string> fields = {"type", "required", "surfaceSamples",
                                         "pointTolerance"};
  if (type != "metamorphicEquivalence" || !exact_fields(value, fields, fields, error) ||
      !value["surfaceSamples"].is_number_unsigned() ||
      value["surfaceSamples"].get<std::size_t>() < 1 ||
      value["surfaceSamples"].get<std::size_t>() > 4096 ||
      !bounded_number(value["pointTolerance"], 0.0, 1.0e12)) {
    error = "unsupported or invalid validator";
    return false;
  }
  return true;
}

bool validate_validators(const json &value, std::string &error) {
  if (!value.is_array() || value.empty() || value.size() > 32) {
    error = "validators must be a non-empty bounded array";
    return false;
  }
  for (const json &validator : value) {
    if (!validate_validator(validator, error))
      return false;
  }
  return true;
}

bool validate_metamorphs(const json &value, std::string &error) {
  if (!value.is_array() || value.size() > 16) {
    error = "metamorphs must be a bounded array";
    return false;
  }
  for (const json &item : value) {
    if (!item.is_object() || !item.contains("type") || !item["type"].is_string())
      return false;
    if (item["type"] == "translation") {
      if (!exact_fields(item, {"type", "vector"}, {"type", "vector"}, error))
        return false;
      std::vector<double> vector;
      if (!parse_vector(item["vector"], 3, 3, vector))
        return false;
    } else {
      const std::set<std::string> fields = {"type", "angleDegrees", "axis", "center"};
      std::vector<double> axis;
      double angle = 0.0;
      if (item["type"] != "rotation" || !exact_fields(item, fields, fields, error) ||
          !parse_vector(item["axis"], 3, 3, axis) ||
          !finite_number(item["angleDegrees"], -360.0, 360.0, angle) ||
          item["center"] != "inputCentroid")
        return false;
    }
  }
  return true;
}

bool parse_case_v1(const json &value, CaseSpec &out, std::string &error) {
  const std::set<std::string> required = {"schemaVersion", "caseId", "generator",
      "geometryRecipe", "geometryTags", "operation", "expectedDomain",
      "validators", "metamorphs", "limits"};
  std::set<std::string> allowed = required;
  allowed.insert("search");
  allowed.insert("limits");
  if (!exact_fields(value, allowed, required, error) ||
      !value["caseId"].is_string() || !safe_id(value["caseId"])) {
    error = error.empty() ? "invalid case schemaVersion or caseId" : error;
    return false;
  }
  out.schema_version = 1;
  out.case_id = value["caseId"];
  out.canonical = value;
  if (!parse_generator(value["generator"], out.generator, error) ||
      !parse_recipe(value["geometryRecipe"], out, error) ||
      !parse_string_array(value["geometryTags"], out.geometry_tags) ||
      !parse_operation(value["operation"], out, error) ||
      !selector_matches_case(out.selector, out.generator.name, out.recipe,
                             out.parameters.feature_index, error))
    return false;
  const std::set<std::string> domains = {"supported", "expectedLimit", "exploratory",
                                          "outsideProductDomain"};
  if (!value["expectedDomain"].is_string() ||
      domains.count(value["expectedDomain"].get<std::string>()) == 0 ||
      !validate_validators(value["validators"], error) ||
      !validate_metamorphs(value["metamorphs"], error) ||
      !parser::validate_limits(value["limits"], error) ||
      (value.contains("search") && !parser::validate_search(value["search"], error))) {
    error = "invalid domain, validators, or metamorphs";
    return false;
  }
  out.expected_domain = value["expectedDomain"];
  out.validators = value["validators"];
  out.metamorphs = value["metamorphs"];
  out.search = value.value("search", json::object());
  out.limits = value.value("limits", json::object());
  return true;
}

bool parse_case(const json &value, CaseSpec &out, std::string &error) {
  if (!value.is_object() || !value.contains("schemaVersion") ||
      !value["schemaVersion"].is_number_integer()) {
    error = "case requires an integer schemaVersion";
    return false;
  }
  const int version = value["schemaVersion"].get<int>();
  if (version == 1)
    return parse_case_v1(value, out, error);
  if (version == 2)
    return parse_case_v2(value, out, error);
  error = "unsupported case schemaVersion";
  return false;
}

bool parse_rotation(const json &value, VariantSpec &out, std::string &error) {
  const std::set<std::string> fields = {"axis", "angleDegrees"};
  if (!exact_fields(value, fields, fields, error) ||
      !parse_vector(value["axis"], 3, 3, out.rotation_axis) ||
      !finite_number(value["angleDegrees"], -360.0, 360.0,
                     out.rotation_degrees)) {
    error = "invalid rotation variant";
    return false;
  }
  const double norm = std::hypot(out.rotation_axis[0],
                                 std::hypot(out.rotation_axis[1], out.rotation_axis[2]));
  return norm > 1.0e-12 ? true : (error = "rotation axis is zero", false);
}

bool parse_mirror(const json &value, VariantSpec &out, std::string &error) {
  const std::set<std::string> allowed = {"normal", "center"};
  if (!exact_fields(value, allowed, {"normal"}, error) ||
      !parse_vector(value["normal"], 3, 3, out.mirror_normal)) {
    error = "invalid mirror variant";
    return false;
  }
  const double norm = std::hypot(out.mirror_normal[0],
                                 std::hypot(out.mirror_normal[1], out.mirror_normal[2]));
  if (norm <= 1.0e-12) {
    error = "mirror normal is zero";
    return false;
  }
  if (value.contains("center") &&
      !parse_vector(value["center"], 3, 3, out.mirror_center)) {
    error = "invalid mirror center";
    return false;
  }
  return true;
}

bool parse_scale(const json &value, VariantSpec &out, std::string &error) {
  const std::set<std::string> allowed = {"factor", "center"};
  if (!exact_fields(value, allowed, {"factor"}, error) ||
      !finite_number(value["factor"], 1.0e-6, 1.0e6, out.scale_factor)) {
    error = "invalid scale variant";
    return false;
  }
  if (value.contains("center") &&
      !parse_vector(value["center"], 3, 3, out.scale_center)) {
    error = "invalid scale center";
    return false;
  }
  return true;
}

bool parse_parameter_epsilon(const json &value, VariantSpec &out,
                             std::string &error) {
  const std::set<std::string> fields = {"parameter", "relativeDelta"};
  if (!exact_fields(value, fields, fields, error) || !value["parameter"].is_string() ||
      value["parameter"] != "operation.radius" ||
      !finite_number(value["relativeDelta"], -0.01, 0.01, out.epsilon_relative_delta)) {
    error = "invalid parameterEpsilon variant";
    return false;
  }
  out.epsilon_parameter = value["parameter"];
  if (out.epsilon_relative_delta == 0.0) {
    error = "parameterEpsilon relativeDelta must be non-zero";
    return false;
  }
  return true;
}

bool parse_edge_order_permutation(const json &value, VariantSpec &out,
                                  std::string &error) {
  const std::set<std::string> fields = {"order"};
  if (!exact_fields(value, fields, fields, error) || !value["order"].is_array()) {
    error = "invalid edgeOrderPermutation variant";
    return false;
  }
  std::set<std::size_t> seen;
  for (const json &item : value["order"]) {
    if (!item.is_number_unsigned()) {
      error = "edgeOrderPermutation order entries must be unsigned integers";
      return false;
    }
    const std::size_t index = item.get<std::size_t>();
    if (!seen.insert(index).second) {
      error = "edgeOrderPermutation order has duplicate index";
      return false;
    }
    out.edge_order.push_back(index);
  }
  if (out.edge_order.empty()) {
    error = "edgeOrderPermutation order must not be empty";
    return false;
  }
  return true;
}

bool parse_contour_seed(const json &value, VariantSpec &out, std::string &error) {
  const std::set<std::string> fields = {"anchorIndex"};
  if (!exact_fields(value, fields, fields, error) ||
      !value["anchorIndex"].is_number_unsigned()) {
    error = "invalid contourSeed variant";
    return false;
  }
  out.contour_seed = value["anchorIndex"].get<std::size_t>();
  return true;
}

bool parse_variant(const json &value, VariantSpec &out, std::string &error) {
  const std::set<std::string> allowed = {
      "name",         "translation",        "rotation",
      "mirror",       "scale",              "parameterEpsilon",
      "edgeOrderPermutation", "contourSeed"};
  if (!exact_fields(value, allowed, {"name"}, error) || !value["name"].is_string())
    return false;
  out = VariantSpec{};
  out.name = value["name"];
  const std::set<std::string> known = {
      "base",           "translated",          "farOriginTranslated",
      "rotated",        "mirrored",            "scaled",
      "parameterEpsilon", "edgeOrderPermutation", "contourSeed"};
  if (!known.count(out.name)) {
    error = "unsupported variant";
    return false;
  }
  if (value.contains("translation") &&
      !parse_vector(value["translation"], 3, 3, out.translation)) {
    error = "invalid translation variant";
    return false;
  }
  if (value.contains("rotation") && !parse_rotation(value["rotation"], out, error))
    return false;
  if (value.contains("mirror") && !parse_mirror(value["mirror"], out, error))
    return false;
  if (value.contains("scale") && !parse_scale(value["scale"], out, error))
    return false;
  if (value.contains("parameterEpsilon") &&
      !parse_parameter_epsilon(value["parameterEpsilon"], out, error))
    return false;
  if (value.contains("edgeOrderPermutation") &&
      !parse_edge_order_permutation(value["edgeOrderPermutation"], out, error))
    return false;
  if (value.contains("contourSeed") &&
      !parse_contour_seed(value["contourSeed"], out, error))
    return false;

  if (out.name == "translated" || out.name == "farOriginTranslated") {
    if (out.translation.empty()) {
      error = "translated variant requires translation";
      return false;
    }
  } else if (out.name == "rotated") {
    if (out.rotation_axis.empty()) {
      error = "rotated variant requires rotation";
      return false;
    }
  } else if (out.name == "mirrored") {
    if (out.mirror_normal.empty()) {
      error = "mirrored variant requires mirror";
      return false;
    }
  } else if (out.name == "scaled") {
    if (out.scale_factor == 1.0 && out.scale_center.empty()) {
      error = "scaled variant requires scale";
      return false;
    }
  } else if (out.name == "parameterEpsilon") {
    if (out.epsilon_parameter.empty()) {
      error = "parameterEpsilon variant requires parameterEpsilon";
      return false;
    }
  } else if (out.name == "edgeOrderPermutation") {
    if (out.edge_order.empty()) {
      error = "edgeOrderPermutation variant requires edgeOrderPermutation";
      return false;
    }
  } else if (out.name == "contourSeed") {
    // Any anchor index is valid; empty/default is not emitted by producers.
  }
  return true;
}

} // namespace

bool parse_request(const nlohmann::json &input, Request &out,
                   std::string &error) {
  const std::set<std::string> required = {"schemaVersion", "case", "backend", "variant"};
  std::set<std::string> allowed = required;
  allowed.insert("artifactDir");
  if (!exact_fields(input, allowed, required, error) || input["schemaVersion"] != 1 ||
      !input["backend"].is_string()) {
    error = error.empty() ? "invalid execution request" : error;
    return false;
  }
  out.backend = input["backend"];
  if (out.backend != "raw-occt" && out.backend != "onecad") {
    error = "unsupported backend";
    return false;
  }
  if (input.contains("artifactDir")) {
    if (!input["artifactDir"].is_string() || input["artifactDir"].get<std::string>().size() > 4096) {
      error = "invalid artifactDir";
      return false;
    }
    out.artifact_dir = input["artifactDir"];
  }
  out.canonical = input;
  return parse_case(input["case"], out.benchmark_case, error) &&
         parse_variant(input["variant"], out.variant, error);
}

} // namespace onecad::benchmark
