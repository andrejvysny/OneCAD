// case-v2 parsing and the `supportPair` geometry generators.
//
// The generators are the reason case-v2 exists, so the assertions here are
// about GEOMETRY, not about the parser echoing its own input: each pair is
// checked for the shared edge landing where the selector's anchor claims, and
// for the dihedral the case asked for actually being the dihedral built.

#include <cmath>
#include <cstdlib>
#include <iostream>
#include <string>

#include <BRepAdaptor_Surface.hxx>
#include <BRepGProp.hxx>
#include <BRep_Tool.hxx>
#include <GeomAPI_ProjectPointOnSurf.hxx>
#include <Geom_Curve.hxx>
#include <GProp_GProps.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopoDS.hxx>
#include <NCollection_IndexedDataMap.hxx>
#include <NCollection_List.hxx>
#include <nlohmann/json.hpp>

#include "benchmark/CaseParser.h"
#include "benchmark/Execution.h"
#include "benchmark/Geometry.h"

namespace {

using nlohmann::json;
using onecad::benchmark::CaseSpec;
using onecad::benchmark::GeneratedGeometry;
using onecad::benchmark::VariantSpec;

void require(bool condition, const std::string &message) {
  if (!condition) {
    std::cerr << "kernelbench case-v2 fixture: " << message << '\n';
    std::exit(1);
  }
}

void run_fixture(const std::string &name, void (*fixture)()) {
  try {
    fixture();
  } catch (const Standard_Failure &failure) {
    require(false, name + ": " + failure.what());
  } catch (const std::exception &error) {
    require(false, name + ": " + error.what());
  }
}

constexpr const char *kTemplate = R"JSON({
  "schemaVersion": 2,
  "caseId": "fixture.support-pair",
  "generator": {"family":"fillet","name":"fillet-matrix","version":1,
                "seed":"0123456789abcdef"},
  "geometry": {
    "recipe": "supportPair",
    "parameters": {"edgeLength": 50.0, "neighborSize": 60.0},
    "tags": ["support-pair"]
  },
  "operation": {"type":"fillet","definition":{
    "radiusLaw": {"mode":"constant","radius":1.0}, "continuity":"g1"}},
  "selector": {
    "mode": "single",
    "topologyRole": "supportPairEdge",
    "provenance": {"generator":"fillet-matrix","recipe":"supportPair"},
    "anchors": [{"kind":"edgeMidpoint","point":[0,0,0],"frame":"recipeLocal"}],
    "surfaceDescriptors": [{"curveKind":"line","adjacentSurfaceKinds":["plane","plane"]}],
    "adjacency": {"relation":"single"}
  },
  "expectedDomain": "supported",
  "validators": [{"type":"deepAudit","required":true}],
  "metamorphs": [],
  "limits": {
    "resources": {"wallTimeMs":10000,"addressSpaceBytes":2147483648,
                  "stdoutBytes":1048576,"stderrBytes":1048576,
                  "artifactBytes":67108864},
    "quality": {}
  }
})JSON";

json support_pair_case(const json &support_a, const json &support_b,
                       double dihedral, const json &anchor,
                       const json &adjacent, const char *curve_kind) {
  json value = json::parse(kTemplate);
  json &parameters = value["geometry"]["parameters"];
  parameters["supportA"] = support_a;
  parameters["supportB"] = support_b;
  parameters["dihedralDegrees"] = dihedral;
  value["selector"]["anchors"][0]["point"] = anchor;
  value["selector"]["surfaceDescriptors"][0]["curveKind"] = curve_kind;
  value["selector"]["surfaceDescriptors"][0]["adjacentSurfaceKinds"] = adjacent;
  return value;
}

CaseSpec parse(const json &value) {
  CaseSpec spec;
  std::string error;
  require(onecad::benchmark::parse_case_v2(value, spec, error),
          "case must parse: " + error);
  return spec;
}

GeneratedGeometry generate(const CaseSpec &spec) {
  GeneratedGeometry geometry;
  std::string error;
  VariantSpec variant;
  variant.name = "base";
  require(onecad::benchmark::generate_geometry(spec, variant, geometry, error),
          "geometry must generate: " + error);
  require(geometry.selected_edges.size() == 1, "exactly one edge selected");
  return geometry;
}

/// Interior dihedral at the selected edge, measured from the two adjacent face
/// normals at the edge's own midpoint — the same quantity the case declares.
double measured_dihedral(const GeneratedGeometry &geometry) {
  NCollection_IndexedDataMap<TopoDS_Shape, NCollection_List<TopoDS_Shape>,
                             TopTools_ShapeMapHasher>
      map;
  TopExp::MapShapesAndAncestors(geometry.shape, TopAbs_EDGE, TopAbs_FACE, map);
  const TopoDS_Edge &edge = geometry.selected_edges.front();
  require(map.Contains(edge), "selected edge belongs to the shape");
  const auto &faces = map.FindFromKey(edge);
  require(faces.Extent() == 2, "selected edge borders exactly two faces");

  double first = 0.0;
  double last = 0.0;
  const Handle(Geom_Curve) curve = BRep_Tool::Curve(edge, first, last);
  const gp_Pnt point = curve->Value(0.5 * (first + last));

  gp_Dir normals[2];
  int index = 0;
  for (const TopoDS_Shape &item : faces) {
    const TopoDS_Face face = TopoDS::Face(item);
    // Outward normal: the surface normal flipped when the face is reversed.
    BRepAdaptor_Surface surface(face);
    Standard_Real u = 0.0;
    Standard_Real v = 0.0;
    GeomAPI_ProjectPointOnSurf projector(point, BRep_Tool::Surface(face));
    projector.LowerDistanceParameters(u, v);
    gp_Pnt position;
    gp_Vec du;
    gp_Vec dv;
    surface.D1(u, v, position, du, dv);
    gp_Dir normal(du.Crossed(dv));
    if (face.Orientation() == TopAbs_REVERSED)
      normal.Reverse();
    normals[index++] = normal;
  }
  const double between = normals[0].Angle(normals[1]) * 180.0 / M_PI;
  return 180.0 - between;
}

void prism_pairs_build_the_requested_dihedral() {
  const json planar = {{"kind", "plane"}};
  const json cylinder = {{"kind", "cylinder"}, {"radius", 20.0}};
  const struct {
    json a;
    json b;
    json adjacent;
  } pairs[] = {
      {planar, planar, json::array({"plane", "plane"})},
      {planar, cylinder, json::array({"plane", "cylinder"})},
      {cylinder, {{"kind", "cylinder"}, {"radius", 25.0}},
       json::array({"cylinder", "cylinder"})},
  };
  for (const auto &pair : pairs) {
    for (const double dihedral : {45.0, 90.0, 135.0}) {
      const CaseSpec spec = parse(support_pair_case(
          pair.a, pair.b, dihedral, json::array({0.0, 0.0, 25.0}),
          pair.adjacent, "line"));
      const GeneratedGeometry geometry = generate(spec);
      GProp_GProps volume;
      BRepGProp::VolumeProperties(geometry.shape, volume);
      require(volume.Mass() > 0.0, "support pair encloses material");
      const double measured = measured_dihedral(geometry);
      require(std::abs(measured - dihedral) < 1.0e-6,
              "dihedral " + std::to_string(dihedral) + " built as " +
                  std::to_string(measured));
    }
  }
}

void the_cone_pair_must_agree_with_its_half_angle() {
  const json planar = {{"kind", "plane"}};
  const json cone = {
      {"kind", "cone"}, {"radius", 40.0}, {"halfAngleDegrees", 30.0}};
  // 90 - 30 is the only dihedral this frustum can present at its base circle.
  const CaseSpec ok = parse(support_pair_case(planar, cone, 60.0,
                                              json::array({40.0, 0.0, 0.0}),
                                              json::array({"plane", "cone"}),
                                              "analyticCurve"));
  const GeneratedGeometry geometry = generate(ok);
  require(std::abs(measured_dihedral(geometry) - 60.0) < 1.0e-6,
          "cone base dihedral is 90 - halfAngle");

  CaseSpec mismatched = parse(support_pair_case(
      planar, cone, 75.0, json::array({40.0, 0.0, 0.0}),
      json::array({"plane", "cone"}), "analyticCurve"));
  GeneratedGeometry rejected;
  VariantSpec variant;
  variant.name = "base";
  std::string error;
  require(!onecad::benchmark::generate_geometry(mismatched, variant, rejected,
                                                error),
          "a dihedral the frustum cannot present must be refused");
  require(error.find("90 - halfAngleDegrees") != std::string::npos,
          "refusal names the constraint: " + error);
}

void unimplemented_supports_are_refused_by_name() {
  const json planar = {{"kind", "plane"}};
  const json sphere = {{"kind", "sphere"}, {"radius", 30.0}};
  CaseSpec spec = parse(support_pair_case(planar, sphere, 90.0,
                                          json::array({0.0, 0.0, 25.0}),
                                          json::array({"plane", "sphere"}),
                                          "analyticCurve"));
  GeneratedGeometry geometry;
  VariantSpec variant;
  variant.name = "base";
  std::string error;
  require(!onecad::benchmark::generate_geometry(spec, variant, geometry, error),
          "an ungenerated support kind must not silently produce something");
  require(error.find("sphere") != std::string::npos,
          "refusal names the kind: " + error);
}

void the_two_case_versions_stay_disjoint() {
  json v2 = support_pair_case({{"kind", "plane"}}, {{"kind", "plane"}}, 90.0,
                              json::array({0.0, 0.0, 25.0}),
                              json::array({"plane", "plane"}), "line");
  CaseSpec spec;
  std::string error;
  v2["schemaVersion"] = 1;
  require(!onecad::benchmark::parse_case_v2(v2, spec, error),
          "a v1 schemaVersion must not parse as v2");

  v2["schemaVersion"] = 2;
  v2["geometryRecipe"] = json::object();
  require(!onecad::benchmark::parse_case_v2(v2, spec, error),
          "a v1 field must not be accepted inside a v2 case");
}

void a_variable_radius_law_is_refused_until_it_can_be_proven() {
  json value = support_pair_case({{"kind", "plane"}}, {{"kind", "plane"}}, 90.0,
                                 json::array({0.0, 0.0, 25.0}),
                                 json::array({"plane", "plane"}), "line");
  value["operation"]["definition"]["radiusLaw"] = {
      {"mode", "linear"}, {"startRadius", 1.0}, {"endRadius", 3.0}};
  CaseSpec spec;
  std::string error;
  require(!onecad::benchmark::parse_case_v2(value, spec, error),
          "a linear radius law must not execute as if it were constant");
}

json boolean_case(const std::string &mode) {
  json value = json::parse(kTemplate);
  value["caseId"] = "fixture.boolean-" + mode;
  value["generator"] = {{"family", "boolean"},
                        {"name", "boolean-foundation"},
                        {"version", 1},
                        {"seed", "0123456789abcdef"}};
  value["geometry"] = {
      {"recipe", "twoBoxes"},
      {"parameters", {{"targetDimensions", {10.0, 10.0, 10.0}},
                      {"toolDimensions", {10.0, 10.0, 10.0}},
                      {"toolOffset", {5.0, 0.0, 0.0}}}},
      {"tags", {"boolean", "overlap"}}};
  value["operation"] = {{"type", "boolean"},
                        {"definition", {{"mode", mode}}}};
  value["selector"] = {{"mode", "bodyRoles"},
                       {"target", "target"},
                       {"tools", {"tool"}}};
  value["validators"] = json::array(
      {{{"type", "deepAudit"}, {"required", true}},
       {{"type", "materialChange"}, {"required", true},
        {"direction", mode == "fuse" ? "increase" : "decrease"}},
       {{"type", "history"}, {"required", true}}});
  return value;
}

void boolean_foundation_runs_both_backends() {
  for (const std::string mode : {"fuse", "cut", "common"}) {
    const CaseSpec spec = parse(boolean_case(mode));
    GeneratedGeometry geometry;
    VariantSpec variant;
    variant.name = "base";
    std::string generation_error;
    require(onecad::benchmark::generate_geometry(spec, variant, geometry,
                                                 generation_error),
            "Boolean geometry must generate: " + generation_error);
    require(!geometry.tool_shape.IsNull(), "twoBoxes generates a tool body");
    require(geometry.selection_evidence.size() == 2,
            "body-role selector resolves target and tool");
    for (const std::string backend : {"raw-occt", "onecad"}) {
      onecad::benchmark::Request request;
      request.canonical = {{"variant", {{"name", "base"}}}};
      request.benchmark_case = spec;
      request.backend = backend;
      request.variant.name = "base";
      const json result = onecad::benchmark::execute_request(request);
      require(result["verdict"] == "pass",
              mode + "/" + backend + " verdict: " + result.dump());
      require(result["operationState"] == "success",
              mode + "/" + backend + " operation succeeds");
    }
  }

  json mismatched = boolean_case("fuse");
  mismatched["operation"]["definition"] = {
      {"radiusLaw", {{"mode", "constant"}, {"radius", 1.0}}},
      {"continuity", "g1"}};
  CaseSpec rejected;
  std::string error;
  require(!onecad::benchmark::parse_case_v2(mismatched, rejected, error),
          "Boolean type rejects a Fillet definition");
  json extra = boolean_case("fuse");
  extra["geometry"]["parameters"]["dimensions"] = {1.0, 1.0, 1.0};
  require(!onecad::benchmark::parse_case_v2(extra, rejected, error),
          "twoBoxes rejects another recipe's parameters");
}

} // namespace

int main() {
  run_fixture("prism-pairs", prism_pairs_build_the_requested_dihedral);
  run_fixture("cone-pair", the_cone_pair_must_agree_with_its_half_angle);
  run_fixture("unimplemented-supports", unimplemented_supports_are_refused_by_name);
  run_fixture("version-disjoint", the_two_case_versions_stay_disjoint);
  run_fixture("variable-radius", a_variable_radius_law_is_refused_until_it_can_be_proven);
  run_fixture("boolean-foundation", boolean_foundation_runs_both_backends);
  return 0;
}
