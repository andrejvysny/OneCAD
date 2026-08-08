#include "benchmark/CaseParser.h"

#include <regex>
#include <set>
#include <string>

#include <nlohmann/json.hpp>

#include "benchmark/CaseParserShared.h"
#include "benchmark/SelectorParser.h"

// case-v2 parsing. The rules here mirror `case_v2.rs::CaseV2::validate` — the
// Rust side is authoritative for what a case may CONTAIN, this side is
// authoritative for what the runner will EXECUTE, and the two must agree on
// every field or a case that validates would fail as an invalid request.

namespace onecad::benchmark {
namespace {

using json = nlohmann::json;
using parser::bounded_number;
using parser::exact_fields;
using parser::finite_number;
using parser::parse_string_array;
using parser::safe_id;

bool parse_generator_v2(const json &value, GeneratorSpec &out,
                        std::string &error) {
  const std::set<std::string> fields = {"family", "name", "version", "seed"};
  if (!exact_fields(value, fields, fields, error))
    return false;
  if (!value["family"].is_string() || value["family"] != "fillet" ||
      !value["name"].is_string() || !value["version"].is_number_integer() ||
      value["version"].get<int>() < 1 || !value["seed"].is_string()) {
    error = "unsupported generator family, name, or version";
    return false;
  }
  const std::set<std::string> names = {"fillet-foundation", "fillet-matrix"};
  if (names.count(value["name"].get<std::string>()) == 0) {
    error = "unsupported generator name";
    return false;
  }
  out = {value["family"], value["name"], value["version"], value["seed"]};
  if (!std::regex_match(out.seed, std::regex("^[0-9a-f]{16}$"))) {
    error = "generator seed must be 16 lowercase hex digits";
    return false;
  }
  return true;
}

bool positive(const json &value, const char *what, std::string &error,
              double &out) {
  if (!finite_number(value, 0.0, 1.0e12, out) || out <= 0.0) {
    error = std::string(what) + " must be finite and positive";
    return false;
  }
  return true;
}

bool parse_support(const json &value, const char *field, SupportSpec &out,
                   std::string &error) {
  const std::set<std::string> allowed = {"kind", "radius", "minorRadius",
                                          "halfAngleDegrees", "degree",
                                          "controlPoints"};
  if (!exact_fields(value, allowed, {"kind"}, error))
    return false;
  const std::set<std::string> kinds = {"plane", "cylinder", "cone",
                                       "sphere", "torus", "bspline", "other"};
  if (!value["kind"].is_string() ||
      kinds.count(value["kind"].get<std::string>()) == 0) {
    error = std::string(field) + ": unsupported surface kind";
    return false;
  }
  out.kind = value["kind"];
  double number = 0.0;
  if (value.contains("radius")) {
    if (!positive(value["radius"], "radius", error, number))
      return false;
    out.radius = number;
  }
  if (value.contains("minorRadius")) {
    if (!positive(value["minorRadius"], "minorRadius", error, number))
      return false;
    out.minor_radius = number;
  }
  if (value.contains("halfAngleDegrees")) {
    if (!finite_number(value["halfAngleDegrees"], 0.0, 90.0, number) ||
        number <= 0.0 || number >= 90.0) {
      error = std::string(field) + ": halfAngleDegrees must be in (0, 90)";
      return false;
    }
    out.half_angle_degrees = number;
  }
  if (value.contains("degree")) {
    if (!value["degree"].is_number_unsigned() ||
        value["degree"].get<int>() < 1 || value["degree"].get<int>() > 8) {
      error = std::string(field) + ": degree must be in 1..8";
      return false;
    }
    out.degree = value["degree"].get<int>();
  }
  if (value.contains("controlPoints")) {
    if (!value["controlPoints"].is_number_unsigned() ||
        value["controlPoints"].get<int>() < 2 ||
        value["controlPoints"].get<int>() > 256) {
      error = std::string(field) + ": controlPoints must be in 2..256";
      return false;
    }
    out.control_points = value["controlPoints"].get<int>();
  }
  // Which optionals a kind REQUIRES. A cylinder without a radius is not an
  // under-specified case, it is a case that describes nothing.
  const bool needs_radius = out.kind == "cylinder" || out.kind == "sphere" ||
                            out.kind == "cone" || out.kind == "torus";
  if ((needs_radius && !out.radius) ||
      (out.kind == "cone" && !out.half_angle_degrees) ||
      (out.kind == "torus" && !out.minor_radius) ||
      (out.kind == "bspline" && (!out.degree || !out.control_points))) {
    error = std::string(field) + ": surface kind is missing a required parameter";
    return false;
  }
  // A tube at least as fat as the ring self-intersects; that is not a support.
  if (out.kind == "torus" && *out.minor_radius >= *out.radius) {
    error = std::string(field) + ": torus minorRadius must be < radius";
    return false;
  }
  return true;
}

bool parse_parameters(const json &value, const std::string &recipe,
                      RecipeParameters &out, std::string &error) {
  const std::set<std::string> allowed = {
      "dimensions",      "featureIndex", "supportA",     "supportB",
      "dihedralDegrees", "valence",      "convexity",    "edgeLength",
      "neighborSize",    "seamOffset",   "consumptionRatio", "separation"};
  if (!exact_fields(value, allowed, {}, error))
    return false;
  if (value.contains("dimensions") &&
      !parser::parse_vector(value["dimensions"], 3, 3, out.dimensions)) {
    error = "dimensions must be three finite numbers";
    return false;
  }
  for (double dimension : out.dimensions) {
    if (dimension <= 0.0) {
      error = "dimensions must be positive";
      return false;
    }
  }
  if (value.contains("featureIndex")) {
    if (!value["featureIndex"].is_number_unsigned() ||
        value["featureIndex"].get<std::size_t>() > 65535) {
      error = "featureIndex exceeds limit";
      return false;
    }
    out.feature_index = value["featureIndex"].get<std::size_t>();
    out.has_feature_index = true;
  }
  if (value.contains("supportA")) {
    SupportSpec support;
    if (!parse_support(value["supportA"], "supportA", support, error))
      return false;
    out.support_a = support;
  }
  if (value.contains("supportB")) {
    SupportSpec support;
    if (!parse_support(value["supportB"], "supportB", support, error))
      return false;
    out.support_b = support;
  }
  double number = 0.0;
  if (value.contains("dihedralDegrees")) {
    if (!finite_number(value["dihedralDegrees"], 0.0, 180.0, number) ||
        number <= 0.0) {
      error = "dihedralDegrees must be in (0, 180]";
      return false;
    }
    out.dihedral_degrees = number;
  }
  if (value.contains("valence")) {
    if (!value["valence"].is_number_unsigned() ||
        value["valence"].get<int>() < 3 || value["valence"].get<int>() > 8) {
      error = "valence must be in 3..8";
      return false;
    }
    out.valence = value["valence"].get<int>();
  }
  if (value.contains("convexity")) {
    const std::set<std::string> convexities = {"convex", "concave", "mixed"};
    if (!value["convexity"].is_string() ||
        convexities.count(value["convexity"].get<std::string>()) == 0) {
      error = "unsupported convexity";
      return false;
    }
    out.convexity = value["convexity"];
  }
  const std::vector<std::pair<const char *, bool>> scalars = {
      {"edgeLength", true},   {"neighborSize", true},
      {"seamOffset", false},  {"consumptionRatio", false},
      {"separation", false}};
  for (const auto &[name, strict] : scalars) {
    if (!value.contains(name))
      continue;
    if (!finite_number(value[name], 0.0, 1.0e12, number) ||
        (strict && number <= 0.0)) {
      error = std::string(name) + " is out of range";
      return false;
    }
    if (std::string(name) == "edgeLength")
      out.edge_length = number;
    else if (std::string(name) == "neighborSize")
      out.neighbor_size = number;
    else if (std::string(name) == "seamOffset")
      out.seam_offset = number;
    else if (std::string(name) == "consumptionRatio")
      out.consumption_ratio = number;
    else
      out.separation = number;
  }
  // Per-recipe requirements, mirroring the Rust validator.
  const std::set<std::string> dimension_recipes = {
      "box", "valence3Corner", "valence4Corner", "overflowWedge"};
  if (dimension_recipes.count(recipe) > 0 &&
      (out.dimensions.empty() || !out.has_feature_index)) {
    error = "recipe requires dimensions and featureIndex";
    return false;
  }
  if (recipe == "supportPair" &&
      (!out.support_a || !out.support_b || !out.dihedral_degrees)) {
    error = "supportPair requires supportA, supportB, and dihedralDegrees";
    return false;
  }
  if (recipe == "valenceCorner" && !out.valence) {
    error = "valenceCorner requires valence";
    return false;
  }
  return true;
}

bool parse_geometry(const json &value, CaseSpec &out, std::string &error) {
  const std::set<std::string> allowed = {"recipe", "parameters", "tags",
                                          "scaleBand"};
  const std::set<std::string> required = {"recipe", "parameters", "tags"};
  if (!exact_fields(value, allowed, required, error))
    return false;
  const std::set<std::string> recipes = {
      "box",           "valence3Corner",     "valence4Corner",
      "overflowWedge", "supportPair",        "valenceCorner",
      "shortEdge",     "microEdge",          "sliverNeighborFace",
      "tinyNeighborFace", "periodicSeam",    "nearSeamEdge",
      "faceNearlyConsumed", "faceFullyConsumed", "blendCollision"};
  if (!value["recipe"].is_string() ||
      recipes.count(value["recipe"].get<std::string>()) == 0) {
    error = "unsupported geometry recipe";
    return false;
  }
  out.recipe = value["recipe"];
  if (!parse_string_array(value["tags"], out.geometry_tags)) {
    error = "geometry tags must be 1..32 unique safe identifiers";
    return false;
  }
  if (value.contains("scaleBand")) {
    const std::set<std::string> bands = {"micro", "unit", "kilo", "mega"};
    if (!value["scaleBand"].is_string() ||
        bands.count(value["scaleBand"].get<std::string>()) == 0) {
      error = "unsupported scaleBand";
      return false;
    }
    out.scale_band = value["scaleBand"];
  }
  return parse_parameters(value["parameters"], out.recipe, out.parameters,
                          error);
}

bool parse_radius_law(const json &value, RadiusLaw &out, std::string &error) {
  if (!value.is_object() || !value.contains("mode") ||
      !value["mode"].is_string()) {
    error = "radiusLaw requires a mode";
    return false;
  }
  out.mode = value["mode"];
  double number = 0.0;
  if (out.mode == "constant") {
    const std::set<std::string> fields = {"mode", "radius"};
    if (!exact_fields(value, fields, fields, error) ||
        !positive(value["radius"], "radius", error, number))
      return false;
    out.radius = number;
    return true;
  }
  if (out.mode == "linear") {
    const std::set<std::string> fields = {"mode", "startRadius", "endRadius"};
    if (!exact_fields(value, fields, fields, error) ||
        !positive(value["startRadius"], "startRadius", error, number))
      return false;
    out.start_radius = number;
    if (!positive(value["endRadius"], "endRadius", error, number))
      return false;
    out.end_radius = number;
    return true;
  }
  if (out.mode != "controlPoints") {
    error = "unsupported radiusLaw mode";
    return false;
  }
  const std::set<std::string> fields = {"mode", "points"};
  if (!exact_fields(value, fields, fields, error) ||
      !value["points"].is_array() || value["points"].size() < 2 ||
      value["points"].size() > 64) {
    error = "controlPoints needs 2..64 points";
    return false;
  }
  double previous = -1.0;
  for (const json &point : value["points"]) {
    const std::set<std::string> point_fields = {"parameter", "radius"};
    double parameter = 0.0;
    if (!exact_fields(point, point_fields, point_fields, error) ||
        !finite_number(point["parameter"], 0.0, 1.0, parameter) ||
        !positive(point["radius"], "controlPoint radius", error, number)) {
      error = error.empty() ? "invalid control point" : error;
      return false;
    }
    // Two radii at one contour parameter is not a law, it is an ambiguity the
    // kernel would resolve by accident.
    if (parameter <= previous) {
      error = "controlPoint parameters must strictly increase";
      return false;
    }
    previous = parameter;
    out.control_points.emplace_back(parameter, number);
  }
  return true;
}

bool parse_operation_v2(const json &value, CaseSpec &out, std::string &error) {
  const std::set<std::string> fields = {"type", "definition"};
  if (!exact_fields(value, fields, fields, error) || !value["type"].is_string() ||
      value["type"] != "fillet") {
    error = error.empty() ? "operation type must be fillet" : error;
    return false;
  }
  const json &definition = value["definition"];
  const std::set<std::string> allowed = {"radiusLaw", "continuity", "sizeType"};
  const std::set<std::string> required = {"radiusLaw", "continuity"};
  if (!exact_fields(definition, allowed, required, error) ||
      !definition["continuity"].is_string() ||
      definition["continuity"] != "g1") {
    error = error.empty() ? "only g1 continuity is executable" : error;
    return false;
  }
  if (definition.contains("sizeType") &&
      (!definition["sizeType"].is_string() ||
       definition["sizeType"] != "radius")) {
    error = "only radius sizeType is executable";
    return false;
  }
  if (!parse_radius_law(definition["radiusLaw"], out.radius_law, error))
    return false;
  // Variable radius arrives with M12, together with a validator that can prove
  // the produced blend follows the law. Refusing here surfaces as an invalid
  // request, which is what it is — the runner cannot execute this case.
  if (out.radius_law.mode != "constant") {
    error = "only constant radius laws are executable";
    return false;
  }
  out.radius = out.radius_law.radius;
  return true;
}

bool validate_validator_v2(const json &value, std::string &error) {
  if (!value.is_object() || !value.contains("type") ||
      !value["type"].is_string() || !value.contains("required") ||
      !value["required"].is_boolean()) {
    error = "invalid validator";
    return false;
  }
  const std::string type = value["type"];
  const std::set<std::string> simple = {
      "constantRadius",   "generatedBlendFace",     "cylindricalRadius",
      "g1BoundaryTangency", "materialChange",       "remoteSupportsUnchanged",
      "deepAudit",        "crossSectionProfile",    "supportTangency",
      "noSelfIntersection", "manifold",             "toleranceGrowth",
      "microTopology"};
  if (simple.count(type) > 0) {
    if (!exact_fields(value, {"type", "required", "direction"},
                      {"type", "required"}, error))
      return false;
    return !value.contains("direction") ||
           (value["direction"].is_string() &&
            (value["direction"] == "decrease" || value["direction"] == "increase"));
  }
  const std::set<std::string> tolerances = {"radiusTolerance", "tangencyTolerance",
                                            "materialTolerance",
                                            "crossSectionTolerance"};
  if (tolerances.count(type) > 0) {
    const std::set<std::string> fields = {"type", "required", "absolute",
                                          "relative"};
    return exact_fields(value, fields, fields, error) &&
           bounded_number(value["absolute"], 0.0, 1.0e12) &&
           bounded_number(value["relative"], 0.0, 1.0);
  }
  if (type == "metamorphicEquivalence") {
    const std::set<std::string> fields = {"type", "required", "surfaceSamples",
                                          "pointTolerance"};
    return exact_fields(value, fields, fields, error) &&
           value["surfaceSamples"].is_number_unsigned() &&
           value["surfaceSamples"].get<std::size_t>() >= 1 &&
           value["surfaceSamples"].get<std::size_t>() <= 4096 &&
           bounded_number(value["pointTolerance"], 0.0, 1.0e12);
  }
  if (type == "radiusLawFollowed") {
    const std::set<std::string> fields = {"type", "required", "samples",
                                          "absolute", "relative"};
    return exact_fields(value, fields, fields, error) &&
           value["samples"].is_number_unsigned() &&
           value["samples"].get<std::size_t>() >= 2 &&
           value["samples"].get<std::size_t>() <= 1024 &&
           bounded_number(value["absolute"], 0.0, 1.0e12) &&
           bounded_number(value["relative"], 0.0, 1.0);
  }
  error = "unsupported validator type";
  return false;
}

bool validate_validators_v2(const json &value, std::string &error) {
  if (!value.is_array() || value.empty() || value.size() > 48) {
    error = "validators must hold 1..48 entries";
    return false;
  }
  for (const json &validator : value) {
    if (!validate_validator_v2(validator, error))
      return false;
  }
  return true;
}

bool validate_metamorph_v2(const json &item, std::size_t anchors,
                           std::string &error) {
  if (!item.is_object() || !item.contains("type") || !item["type"].is_string()) {
    error = "metamorph requires a type";
    return false;
  }
  const std::string type = item["type"];
  std::vector<double> vector;
  double number = 0.0;
  if (type == "translation" || type == "farOriginTranslation") {
    return exact_fields(item, {"type", "vector"}, {"type", "vector"}, error) &&
           parser::parse_vector(item["vector"], 3, 3, vector);
  }
  if (type == "rotation") {
    const std::set<std::string> fields = {"type", "angleDegrees", "axis",
                                          "center"};
    return exact_fields(item, fields, fields, error) &&
           parser::parse_vector(item["axis"], 3, 3, vector) &&
           finite_number(item["angleDegrees"], -360.0, 360.0, number) &&
           item["center"] == "inputCentroid";
  }
  if (type == "mirror") {
    const std::set<std::string> fields = {"type", "normal", "center"};
    return exact_fields(item, fields, fields, error) &&
           parser::parse_vector(item["normal"], 3, 3, vector) &&
           item["center"] == "inputCentroid";
  }
  if (type == "uniformScale") {
    const std::set<std::string> fields = {"type", "factor", "center"};
    return exact_fields(item, fields, fields, error) &&
           finite_number(item["factor"], 0.0, 1.0e12, number) && number > 0.0 &&
           item["center"] == "inputCentroid";
  }
  if (type == "parameterEpsilon") {
    const std::set<std::string> fields = {"type", "parameter", "relativeDelta"};
    // A result that flips on ±0 is not a metamorph, it is the same run twice.
    return exact_fields(item, fields, fields, error) &&
           item["parameter"] == "operation.radius" &&
           finite_number(item["relativeDelta"], -0.01, 0.01, number) &&
           number != 0.0;
  }
  if (type == "edgeOrderPermutation") {
    const std::set<std::string> fields = {"type", "order"};
    if (!exact_fields(item, fields, fields, error) ||
        !item["order"].is_array() || item["order"].size() != anchors) {
      error = "edgeOrderPermutation must cover every selector anchor";
      return false;
    }
    std::set<std::size_t> seen;
    for (const json &index : item["order"]) {
      if (!index.is_number_unsigned() ||
          index.get<std::size_t>() >= anchors ||
          !seen.insert(index.get<std::size_t>()).second) {
        error = "edgeOrderPermutation is not a permutation of the anchors";
        return false;
      }
    }
    return true;
  }
  if (type == "contourSeed") {
    const std::set<std::string> fields = {"type", "anchorIndex"};
    return exact_fields(item, fields, fields, error) &&
           item["anchorIndex"].is_number_unsigned() &&
           item["anchorIndex"].get<std::size_t>() < anchors;
  }
  error = "unsupported metamorph type";
  return false;
}

bool validate_metamorphs_v2(const json &value, std::size_t anchors,
                            std::string &error) {
  if (!value.is_array() || value.size() > 32) {
    error = "metamorphs must hold at most 32 entries";
    return false;
  }
  for (const json &item : value) {
    if (!validate_metamorph_v2(item, anchors, error))
      return false;
  }
  return true;
}

} // namespace

bool parse_case_v2(const nlohmann::json &value, CaseSpec &out,
                   std::string &error) {
  const std::set<std::string> required = {
      "schemaVersion", "caseId",     "generator",  "geometry",
      "operation",     "selector",   "expectedDomain", "validators",
      "metamorphs",    "limits"};
  std::set<std::string> allowed = required;
  allowed.insert("search");
  if (!exact_fields(value, allowed, required, error) ||
      !value["caseId"].is_string() ||
      !safe_id(value["caseId"].get<std::string>())) {
    error = error.empty() ? "invalid caseId" : error;
    return false;
  }
  // Checked here as well as at the dispatch, so this entry point cannot be
  // called directly on a v1 document and quietly read it as v2.
  if (!value["schemaVersion"].is_number_integer() ||
      value["schemaVersion"].get<int>() != 2) {
    error = "case schemaVersion must be 2";
    return false;
  }
  out.schema_version = 2;
  out.case_id = value["caseId"];
  out.canonical = value;
  if (!parse_generator_v2(value["generator"], out.generator, error) ||
      !parse_geometry(value["geometry"], out, error) ||
      !parse_operation_v2(value["operation"], out, error) ||
      !parse_selector_v2(value["selector"], out.selector, error) ||
      !selector_matches_case_v2(out.selector, out.generator.name, out.recipe,
                                out.parameters.has_feature_index,
                                out.parameters.feature_index, error))
    return false;
  const std::set<std::string> domains = {"supported", "expectedLimit",
                                          "exploratory", "outsideProductDomain"};
  if (!value["expectedDomain"].is_string() ||
      domains.count(value["expectedDomain"].get<std::string>()) == 0) {
    error = "unsupported expectedDomain";
    return false;
  }
  if (!validate_validators_v2(value["validators"], error) ||
      !validate_metamorphs_v2(value["metamorphs"],
                              out.selector.anchor_points.size(), error) ||
      !parser::validate_limits(value["limits"], error) ||
      (value.contains("search") &&
       !parser::validate_search(value["search"], error))) {
    error = error.empty() ? "invalid validators, metamorphs, or limits" : error;
    return false;
  }
  out.expected_domain = value["expectedDomain"];
  out.validators = value["validators"];
  out.metamorphs = value["metamorphs"];
  out.search = value.value("search", json::object());
  out.limits = value["limits"];
  return true;
}

} // namespace onecad::benchmark
