#include "benchmark/CaseParser.h"

#include <cmath>
#include <regex>
#include <set>
#include <tuple>

#include <nlohmann/json.hpp>

#include "benchmark/SelectorParser.h"

namespace onecad::benchmark {
namespace {

using json = nlohmann::json;

bool exact_fields(const json &value, const std::set<std::string> &allowed,
                  const std::set<std::string> &required, std::string &error) {
  if (!value.is_object()) {
    error = "required block must be an object";
    return false;
  }
  for (auto it = value.begin(); it != value.end(); ++it) {
    if (allowed.count(it.key()) == 0) {
      error = "unknown field: " + it.key();
      return false;
    }
  }
  for (const std::string &key : required) {
    if (!value.contains(key)) {
      error = "missing field: " + key;
      return false;
    }
  }
  return true;
}

bool safe_id(const std::string &value) {
  static const std::regex pattern("^[a-z0-9][a-z0-9._-]{0,127}$");
  return std::regex_match(value, pattern);
}

bool finite_number(const json &value, double min, double max, double &out) {
  if (!value.is_number())
    return false;
  out = value.get<double>();
  return std::isfinite(out) && out >= min && out <= max;
}

bool parse_vector(const json &value, std::size_t min_size,
                  std::size_t max_size, std::vector<double> &out) {
  if (!value.is_array() || value.size() < min_size || value.size() > max_size)
    return false;
  for (const json &item : value) {
    double number = 0.0;
    if (!finite_number(item, -1.0e12, 1.0e12, number))
      return false;
    out.push_back(number);
  }
  return true;
}

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
  out = {value["name"], value["version"], value["seed"]};
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
  out.recipe_type = value["type"];
  const std::set<std::string> supported = {"box", "valence3Corner",
                                            "valence4Corner", "overflowWedge"};
  if (supported.count(out.recipe_type) == 0 ||
      !parse_vector(value["dimensions"], 3, 3, out.dimensions)) {
    error = "unsupported or invalid geometry recipe";
    return false;
  }
  for (double dimension : out.dimensions) {
    if (dimension <= 1.0e-6) {
      error = "geometry dimensions must be positive";
      return false;
    }
  }
  out.feature_index = value["featureIndex"].get<std::size_t>();
  return out.feature_index <= 65535 ? true : (error = "featureIndex exceeds limit", false);
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
  return parse_selector(value["selector"], out.selector, error);
}

bool parse_string_array(const json &value, std::vector<std::string> &out) {
  if (!value.is_array() || value.empty() || value.size() > 32)
    return false;
  std::set<std::string> unique;
  for (const json &item : value) {
    if (!item.is_string() || !safe_id(item.get<std::string>()) ||
        !unique.insert(item.get<std::string>()).second)
      return false;
    out.push_back(item);
  }
  return true;
}

bool bounded_number(const json &value, double minimum, double maximum,
                    bool exclusive_minimum = false) {
  double number = 0.0;
  if (!finite_number(value, minimum, maximum, number))
    return false;
  return !exclusive_minimum || number > minimum;
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

bool validate_search(const json &value, std::string &error) {
  const std::set<std::string> fields = {"parameter", "knownSuccess", "upperBound",
      "growthFactor", "maxProbes", "relativePrecision", "absolutePrecision",
      "relativeOffsets"};
  if (!exact_fields(value, fields, fields, error) ||
      value["parameter"] != "operation.radius" ||
      !bounded_number(value["knownSuccess"], 0.0, 1.0e12, true) ||
      !bounded_number(value["upperBound"], 0.0, 1.0e12, true) ||
      !bounded_number(value["growthFactor"], 1.0, 16.0, true) ||
      !value["maxProbes"].is_number_unsigned() ||
      value["maxProbes"].get<std::size_t>() < 8 ||
      value["maxProbes"].get<std::size_t>() > 4096)
    return false;
  if (!bounded_number(value["relativePrecision"], 0.0, 0.01, true) ||
      !bounded_number(value["absolutePrecision"], 0.0, 1.0, true) ||
      !value["relativeOffsets"].is_array() || value["relativeOffsets"].size() != 4)
    return false;
  const std::vector<double> expected = {1.0e-2, 1.0e-4, 1.0e-6, 1.0e-8};
  for (std::size_t i = 0; i < expected.size(); ++i) {
    if (!value["relativeOffsets"][i].is_number() ||
        value["relativeOffsets"][i].get<double>() != expected[i])
      return false;
  }
  return true;
}

bool validate_limits(const json &value, std::string &error) {
  if (!exact_fields(value, {"resources", "quality"}, {"resources", "quality"}, error))
    return false;
  const json &resources = value["resources"];
  const std::set<std::string> resource_fields = {"wallTimeMs", "addressSpaceBytes",
      "stdoutBytes", "stderrBytes", "artifactBytes"};
  if (!exact_fields(resources, resource_fields, resource_fields, error))
    return false;
  const std::vector<std::tuple<const char *, std::uint64_t, std::uint64_t>> ranges = {
      {"wallTimeMs", 1, 3600000}, {"addressSpaceBytes", 16777216, 68719476736ULL},
      {"stdoutBytes", 1024, 16777216}, {"stderrBytes", 1024, 16777216},
      {"artifactBytes", 1024, 1073741824}};
  for (const auto &[key, minimum, maximum] : ranges) {
    if (!resources[key].is_number_unsigned() ||
        resources[key].get<std::uint64_t>() < minimum ||
        resources[key].get<std::uint64_t>() > maximum)
      return false;
  }
  const json &quality = value["quality"];
  const std::set<std::string> quality_fields = {"maxVertexTolerance", "maxEdgeTolerance",
      "maxFaceTolerance", "maxMicroEdges", "maxSliverFaces"};
  if (!exact_fields(quality, quality_fields, {}, error))
    return false;
  for (auto it = quality.begin(); it != quality.end(); ++it) {
    const bool count = it.key() == "maxMicroEdges" || it.key() == "maxSliverFaces";
    if ((count && !it.value().is_number_unsigned()) ||
        (!count && !bounded_number(it.value(), 0.0, 1.0e12)))
      return false;
  }
  return true;
}

bool parse_case(const json &value, CaseSpec &out, std::string &error) {
  const std::set<std::string> required = {"schemaVersion", "caseId", "generator",
      "geometryRecipe", "geometryTags", "operation", "expectedDomain",
      "validators", "metamorphs", "limits"};
  std::set<std::string> allowed = required;
  allowed.insert("search");
  allowed.insert("limits");
  if (!exact_fields(value, allowed, required, error) || value["schemaVersion"] != 1 ||
      !value["caseId"].is_string() || !safe_id(value["caseId"])) {
    error = error.empty() ? "invalid case schemaVersion or caseId" : error;
    return false;
  }
  out.case_id = value["caseId"];
  out.canonical = value;
  if (!parse_generator(value["generator"], out.generator, error) ||
      !parse_recipe(value["geometryRecipe"], out, error) ||
      !parse_string_array(value["geometryTags"], out.geometry_tags) ||
      !parse_operation(value["operation"], out, error) ||
      !selector_matches_case(out.selector, out.generator.name, out.recipe_type,
                             out.feature_index, error))
    return false;
  const std::set<std::string> domains = {"supported", "expectedLimit", "exploratory",
                                          "outsideProductDomain"};
  if (!value["expectedDomain"].is_string() ||
      domains.count(value["expectedDomain"].get<std::string>()) == 0 ||
      !validate_validators(value["validators"], error) ||
      !validate_metamorphs(value["metamorphs"], error) ||
      !validate_limits(value["limits"], error) ||
      (value.contains("search") && !validate_search(value["search"], error))) {
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

bool parse_variant(const json &value, VariantSpec &out, std::string &error) {
  const std::set<std::string> allowed = {"name", "translation", "rotation"};
  if (!exact_fields(value, allowed, {"name"}, error) || !value["name"].is_string())
    return false;
  out.name = value["name"];
  if (out.name != "base" && out.name != "translated" && out.name != "rotated") {
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
  if (out.name == "translated" && out.translation.empty()) {
    error = "translated variant requires translation";
    return false;
  }
  if (out.name == "rotated" && out.rotation_axis.empty()) {
    error = "rotated variant requires rotation";
    return false;
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
