// Semantic validation under the non-isometric metamorphs.
//
// `scaled` and `parameterEpsilon` change the radius the operation actually runs
// with (see `Execution.cpp`). A validator that measures the blend against the
// case's DECLARED radius therefore measures the wrong shape and reports a false
// red — which is exactly what `cylindricalRadius` and `g1BoundaryTangency` did
// before the effective radius was threaded through `validate_output`. The
// assertion is that every validator reaches the same verdict on the variant as
// it does on the base, because the same operation succeeded in both.

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
    std::cerr << "kernelbench metamorph fixture: " << message << '\n';
    std::exit(1);
  }
}

constexpr const char *kCase = R"JSON({
  "schemaVersion": 2,
  "caseId": "fixture.metamorph.plane-plane",
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
    {"type":"constantRadius","required":true},
    {"type":"generatedBlendFace","required":true},
    {"type":"cylindricalRadius","required":true},
    {"type":"g1BoundaryTangency","required":true},
    {"type":"radiusTolerance","required":true,"absolute":1.0e-9,"relative":1.0e-6},
    {"type":"deepAudit","required":true}
  ],
  "metamorphs": [],
  "limits": {
    "resources": {"wallTimeMs":10000,"addressSpaceBytes":2147483648,
                  "stdoutBytes":1048576,"stderrBytes":1048576,
                  "artifactBytes":67108864},
    "quality": {}
  }
})JSON";

json execute(const json &variant) {
  json input = {{"schemaVersion", 1},
                {"case", json::parse(kCase)},
                {"backend", "onecad"},
                {"variant", variant}};
  Request request;
  std::string error;
  require(onecad::benchmark::parse_request(input, request, error),
          "request must parse: " + error);
  return onecad::benchmark::execute_request(request);
}

/// Validator kind -> status, in declaration order.
std::string signature(const json &result) {
  std::string out;
  for (const json &item : result["validators"])
    out += item.value("kind", "?") + "=" + item.value("status", "?") + " ";
  return out;
}

void non_isometric_variants_validate_against_the_radius_they_ran_with() {
  const json base = execute({{"name", "base"}});
  require(base["operationState"] == "success",
          "base must succeed, got " + base.value("operationState", "?"));
  const std::string expected = signature(base);
  require(expected.find("=fail") == std::string::npos,
          "base must pass every validator: " + expected);

  const json scaled =
      execute({{"name", "scaled"}, {"scale", {{"factor", 2.0}}}});
  require(scaled["operationState"] == "success",
          "scaled must succeed, got " + scaled.value("operationState", "?"));
  require(signature(scaled) == expected,
          "scaled validators: " + signature(scaled) + "!= base: " + expected);

  const json nudged = execute(
      {{"name", "parameterEpsilon"},
       {"parameterEpsilon",
        {{"parameter", "operation.radius"}, {"relativeDelta", 1.0e-3}}}});
  require(nudged["operationState"] == "success",
          "parameterEpsilon must succeed, got " +
              nudged.value("operationState", "?"));
  require(signature(nudged) == expected,
          "parameterEpsilon validators: " + signature(nudged) +
              "!= base: " + expected);
}

} // namespace

int main() {
  try {
    non_isometric_variants_validate_against_the_radius_they_ran_with();
  } catch (const Standard_Failure &failure) {
    require(false, std::string("OCCT failure: ") + failure.what());
  } catch (const std::exception &error) {
    require(false, std::string("exception: ") + error.what());
  }
  std::cout << "kernelbench metamorph fixtures passed\n";
  return 0;
}
