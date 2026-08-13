// The recipe-agnostic validators, on a curved support pair as well as a planar
// one.
//
// That is the whole point of M4: `cylindricalRadius` reads the blend off the
// output by surface type, which is wrong the moment a SUPPORT is a cylinder, and
// `g1BoundaryTangency` assumes a cylinder/plane pair. `supportTangency` and
// `crossSectionProfile` read the fillet builder's own generated and modified
// faces instead, so the cylinder-cylinder case below is not a special case for
// them. Before they were implemented every one of these reported
// `notApplicable`, which a required check must fail.

#include <cstdlib>
#include <iostream>
#include <string>

#include <Standard_Failure.hxx>
#include <nlohmann/json.hpp>

#include "benchmark/CaseParser.h"
#include "benchmark/Execution.h"

namespace {

using nlohmann::json;
using onecad::benchmark::Request;

void require(bool condition, const std::string &message) {
  if (!condition) {
    std::cerr << "kernelbench validator fixture: " << message << '\n';
    std::exit(1);
  }
}

constexpr const char *kCase = R"JSON({
  "schemaVersion": 2,
  "caseId": "fixture.validators",
  "generator": {"family":"fillet","name":"fillet-matrix","version":1,
                "seed":"0123456789abcdef"},
  "geometry": {
    "recipe": "supportPair",
    "parameters": {"edgeLength": 50.0, "neighborSize": 60.0,
                   "supportA": {"kind":"plane"}, "supportB": {"kind":"plane"},
                   "dihedralDegrees": 90.0},
    "tags": ["support-pair"]
  },
  "operation": {"type":"fillet","definition":{
    "radiusLaw": {"mode":"constant","radius":2.0}, "continuity":"g1"}},
  "selector": {
    "mode": "single",
    "topologyRole": "supportPairEdge",
    "provenance": {"generator":"fillet-matrix","recipe":"supportPair"},
    "anchors": [{"kind":"edgeMidpoint","point":[0,0,25],"frame":"recipeLocal"}],
    "surfaceDescriptors": [{"curveKind":"line","adjacentSurfaceKinds":["plane","plane"]}],
    "adjacency": {"relation":"single"}
  },
  "expectedDomain": "supported",
  "validators": [
    {"type":"supportTangency","required":true},
    {"type":"crossSectionProfile","required":true},
    {"type":"manifold","required":true},
    {"type":"noSelfIntersection","required":true},
    {"type":"microTopology","required":true},
    {"type":"toleranceGrowth","required":true}
  ],
  "metamorphs": [],
  "limits": {
    "resources": {"wallTimeMs":10000,"addressSpaceBytes":2147483648,
                  "stdoutBytes":1048576,"stderrBytes":1048576,
                  "artifactBytes":67108864},
    "quality": {"maxVertexTolerance":1.0e-6,"maxEdgeTolerance":1.0e-6,
                "maxFaceTolerance":1.0e-6,"maxMicroEdges":0,"maxSliverFaces":0}
  }
})JSON";

json execute(const json &supports, const json &adjacent, const char *curve_kind) {
  json input = {{"schemaVersion", 1},
                {"case", json::parse(kCase)},
                {"backend", "onecad"},
                {"variant", {{"name", "base"}}}};
  json &parameters = input["case"]["geometry"]["parameters"];
  parameters["supportA"] = supports[0];
  parameters["supportB"] = supports[1];
  input["case"]["selector"]["surfaceDescriptors"][0]["adjacentSurfaceKinds"] = adjacent;
  input["case"]["selector"]["surfaceDescriptors"][0]["curveKind"] = curve_kind;
  Request request;
  std::string error;
  require(onecad::benchmark::parse_request(input, request, error),
          "request must parse: " + error);
  return onecad::benchmark::execute_request(request);
}

double metric_of(const json &validator, const std::string &name) {
  for (const json &item : validator["metrics"]) {
    if (item.value("name", "") == name)
      return item.value("value", 0.0);
  }
  return -1.0;
}

const json &validator_of(const json &result, const std::string &kind) {
  for (const json &item : result["validators"]) {
    if (item.value("kind", "") == kind)
      return item;
  }
  require(false, "missing validator " + kind);
  std::abort();
}

void every_validator_reaches_a_verdict(const json &result, const std::string &label) {
  require(result["operationState"] == "success",
          label + " must succeed, got " + result.value("operationState", "?"));
  for (const json &item : result["validators"]) {
    const std::string kind = item.value("kind", "?");
    const std::string status = item.value("status", "?");
    // `notApplicable` is what an unimplemented validator returns, and a required
    // check must never be satisfied by silence.
    require(status == "pass", label + ": " + kind + " = " + status);
  }
}

void the_generic_validators_hold_on_planar_and_curved_supports() {
  const json plane = {{"kind", "plane"}};
  const json cylinder = {{"kind", "cylinder"}, {"radius", 20.0}};
  const json planar = execute({plane, plane}, json::array({"plane", "plane"}), "line");
  every_validator_reaches_a_verdict(planar, "plane-plane");
  const json curved = execute({cylinder, {{"kind", "cylinder"}, {"radius", 25.0}}},
                              json::array({"cylinder", "cylinder"}), "line");
  every_validator_reaches_a_verdict(curved, "cylinder-cylinder");

  for (const json &result : {planar, curved}) {
    const json &tangency = validator_of(result, "supportTangency");
    // Exactly the two supports of the selected edge, never the end caps it also
    // meets at a right angle.
    require(metric_of(tangency, "boundaryCount") == 2.0,
            "expected two support boundaries, got " +
                std::to_string(metric_of(tangency, "boundaryCount")));
    const json &section = validator_of(result, "crossSectionProfile");
    require(metric_of(section, "sampleCount") > 0.0, "section must sample the blend");
    // The blend's section radius IS the requested radius, on both pairs.
    require(std::abs(metric_of(section, "minimumSectionRadius") - 2.0) < 1.0e-6,
            "section radius must be the requested 2.0, got " +
                std::to_string(metric_of(section, "minimumSectionRadius")));
  }
}

} // namespace

int main() {
  try {
    the_generic_validators_hold_on_planar_and_curved_supports();
  } catch (const Standard_Failure &failure) {
    require(false, std::string("OCCT failure: ") + failure.what());
  } catch (const std::exception &error) {
    require(false, std::string("exception: ") + error.what());
  }
  std::cout << "kernelbench validator fixtures passed\n";
  return 0;
}
