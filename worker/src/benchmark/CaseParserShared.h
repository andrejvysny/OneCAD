#pragma once

// Primitives shared by the case-v1 and case-v2 parsers. Both formats bound the
// same resource/search blocks and use the same identifier and number rules, so
// they live here rather than being copied — a divergence between the two copies
// would let one version accept an input the other rejects.

#include <cmath>
#include <cstdint>
#include <regex>
#include <set>
#include <string>
#include <tuple>
#include <vector>

#include <nlohmann/json.hpp>

namespace onecad::benchmark::parser {

using json = nlohmann::json;

inline bool exact_fields(const json &value, const std::set<std::string> &allowed,
                         const std::set<std::string> &required,
                         std::string &error) {
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

inline bool safe_id(const std::string &value) {
  static const std::regex pattern("^[a-z0-9][a-z0-9._-]{0,127}$");
  return std::regex_match(value, pattern);
}

inline bool finite_number(const json &value, double min, double max,
                          double &out) {
  if (!value.is_number())
    return false;
  out = value.get<double>();
  return std::isfinite(out) && out >= min && out <= max;
}

inline bool bounded_number(const json &value, double minimum, double maximum,
                           bool exclusive_minimum = false) {
  double number = 0.0;
  if (!finite_number(value, minimum, maximum, number))
    return false;
  return !exclusive_minimum || number > minimum;
}

inline bool parse_vector(const json &value, std::size_t min_size,
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

inline bool parse_string_array(const json &value,
                               std::vector<std::string> &out) {
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

inline bool validate_search(const json &value, std::string &error) {
  const std::set<std::string> fields = {
      "parameter",         "knownSuccess",      "upperBound",
      "growthFactor",      "maxProbes",         "relativePrecision",
      "absolutePrecision", "relativeOffsets"};
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
      !value["relativeOffsets"].is_array() ||
      value["relativeOffsets"].size() != 4)
    return false;
  const std::vector<double> expected = {1.0e-2, 1.0e-4, 1.0e-6, 1.0e-8};
  for (std::size_t i = 0; i < expected.size(); ++i) {
    if (!value["relativeOffsets"][i].is_number() ||
        value["relativeOffsets"][i].get<double>() != expected[i])
      return false;
  }
  return true;
}

inline bool validate_limits(const json &value, std::string &error) {
  if (!exact_fields(value, {"resources", "quality"}, {"resources", "quality"},
                    error))
    return false;
  const json &resources = value["resources"];
  const std::set<std::string> resource_fields = {
      "wallTimeMs", "addressSpaceBytes", "stdoutBytes", "stderrBytes",
      "artifactBytes"};
  if (!exact_fields(resources, resource_fields, resource_fields, error))
    return false;
  const std::vector<std::tuple<const char *, std::uint64_t, std::uint64_t>>
      ranges = {{"wallTimeMs", 1, 3600000},
                {"addressSpaceBytes", 16777216, 68719476736ULL},
                {"stdoutBytes", 1024, 16777216},
                {"stderrBytes", 1024, 16777216},
                {"artifactBytes", 1024, 1073741824}};
  for (const auto &[key, minimum, maximum] : ranges) {
    if (!resources[key].is_number_unsigned() ||
        resources[key].get<std::uint64_t>() < minimum ||
        resources[key].get<std::uint64_t>() > maximum)
      return false;
  }
  const json &quality = value["quality"];
  const std::set<std::string> quality_fields = {
      "maxVertexTolerance", "maxEdgeTolerance", "maxFaceTolerance",
      "maxMicroEdges", "maxSliverFaces"};
  if (!exact_fields(quality, quality_fields, {}, error))
    return false;
  for (auto it = quality.begin(); it != quality.end(); ++it) {
    const bool count =
        it.key() == "maxMicroEdges" || it.key() == "maxSliverFaces";
    if ((count && !it.value().is_number_unsigned()) ||
        (!count && !bounded_number(it.value(), 0.0, 1.0e12)))
      return false;
  }
  return true;
}

} // namespace onecad::benchmark::parser
