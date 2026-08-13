#include "benchmark/BooleanRun.h"

#include <memory>

#include <BOPAlgo_Operation.hxx>
#include <BRepAlgoAPI_BooleanOperation.hxx>
#include <BRepBuilderAPI_MakeShape.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <Standard_Failure.hxx>
#include <TopExp_Explorer.hxx>
#include <NCollection_List.hxx>

#include "modeling/BooleanMode.h"
#include "ops/OpCommon.h"

namespace onecad::benchmark {
namespace {

app::BooleanMode app_mode(const std::string &mode) {
  if (mode == "cut")
    return app::BooleanMode::Cut;
  if (mode == "common")
    return app::BooleanMode::Intersect;
  return app::BooleanMode::Add;
}

BOPAlgo_Operation raw_mode(const std::string &mode) {
  if (mode == "cut")
    return BOPAlgo_CUT;
  if (mode == "common")
    return BOPAlgo_COMMON;
  return BOPAlgo_FUSE;
}

bool valid_inputs(const GeneratedGeometry &geometry, AdapterResult &out) {
  if (geometry.shape.IsNull() || geometry.tool_shape.IsNull() ||
      !BRepCheck_Analyzer(geometry.shape).IsValid() ||
      !BRepCheck_Analyzer(geometry.tool_shape).IsValid()) {
    out.operation_state = "refused";
    out.failure_class = "invalid-input";
    out.message = "Boolean input failed BRep validation";
    return false;
  }
  return true;
}

void capture_history(BRepBuilderAPI_MakeShape &builder,
                     const GeneratedGeometry &geometry, AdapterResult &out) {
  for (const TopoDS_Shape &input : {geometry.shape, geometry.tool_shape}) {
    out.history_modified_count += builder.Modified(input).Size();
    out.history_generated_count += builder.Generated(input).Size();
    for (TopAbs_ShapeEnum type : {TopAbs_FACE, TopAbs_EDGE, TopAbs_VERTEX}) {
      for (TopExp_Explorer it(input, type); it.More(); it.Next()) {
        out.history_modified_count += builder.Modified(it.Current()).Size();
        out.history_generated_count += builder.Generated(it.Current()).Size();
      }
    }
  }
}

AdapterResult run_raw(const GeneratedGeometry &geometry,
                      const std::string &mode) {
  AdapterResult out;
  if (!valid_inputs(geometry, out))
    return out;
  try {
    BRepAlgoAPI_BooleanOperation builder;
    NCollection_List<TopoDS_Shape> arguments;
    NCollection_List<TopoDS_Shape> tools;
    arguments.Append(geometry.shape);
    tools.Append(geometry.tool_shape);
    builder.SetArguments(arguments);
    builder.SetTools(tools);
    builder.SetOperation(raw_mode(mode));
    builder.SetRunParallel(false);
    builder.SetNonDestructive(true);
    builder.SetCheckInverted(true);
    builder.Build();
    if (!builder.IsDone() || builder.HasErrors()) {
      out.operation_state = "refused";
      out.failure_class = "operation-refused";
      return out;
    }
    out.output = builder.Shape();
    capture_history(builder, geometry, out);
    out.success = !out.output.IsNull();
    out.operation_state = out.success ? "success" : "invalid-result";
    out.failure_class = out.success ? "none" : "invalid-output";
  } catch (const Standard_Failure &failure) {
    out.operation_state = "exception";
    out.failure_class = "kernel-exception";
    out.message = failure.what();
  }
  return out;
}

AdapterResult run_onecad(const GeneratedGeometry &geometry,
                         const std::string &mode) {
  AdapterResult out;
  if (!valid_inputs(geometry, out))
    return out;
  std::shared_ptr<BRepBuilderAPI_MakeShape> history;
  const auto built = onecad::ops::checked_boolean(
      geometry.shape, geometry.tool_shape, app_mode(mode), false,
      nlohmann::json::object(), nullptr, history);
  if (!built.error_code.empty()) {
    out.operation_state = "refused";
    out.failure_class = built.error_code;
    out.message = built.error_message;
    return out;
  }
  onecad::kernel::validation::PublicationPolicy policy;
  policy.name = "Boolean benchmark";
  policy.max_solid_count = -1;
  policy.tier = onecad::kernel::validation::PublicationTier::TierB;
  const auto decision = onecad::ops::publication_decision(built.shape, policy);
  out.diagnostics.push_back(decision.to_json());
  if (!decision.publishable()) {
    out.operation_state = "refused";
    out.failure_class = decision.code;
    out.message = decision.message;
    return out;
  }
  out.output = built.shape;
  if (history)
    capture_history(*history, geometry, out);
  out.success = true;
  out.operation_state = "success";
  out.failure_class = "none";
  return out;
}

} // namespace

AdapterResult run_boolean(const std::string &backend,
                          const GeneratedGeometry &geometry,
                          const std::string &mode) {
  return backend == "raw-occt" ? run_raw(geometry, mode)
                                : run_onecad(geometry, mode);
}

} // namespace onecad::benchmark
