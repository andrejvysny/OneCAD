#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <fstream>
#include <string>
#include <vector>

#include <BRepPrimAPI_MakeBox.hxx>
#include <BRep_Tool.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Vertex.hxx>

#include "elementmap/ElementMapPartition.h"
#include "kernel/fillet/FilletBuilder.h"

namespace em = onecad::elementmap;
namespace kf = onecad::kernel::fillet;
namespace km = onecad::kernel::elementmap;
using Clock = std::chrono::steady_clock;

namespace {

struct Case {
  std::string id;
  TopoDS_Shape body;
  std::vector<TopoDS_Edge> edges;
  double radius = 0.0;
};

struct Result {
  std::string id;
  std::vector<double> milliseconds;
  int success_count = 0;
  int refusal_count = 0;
  bool valid = true;
};

std::vector<TopoDS_Edge> vertical_edges(const TopoDS_Shape &shape) {
  TopTools_IndexedMapOfShape map;
  TopExp::MapShapes(shape, TopAbs_EDGE, map);
  std::vector<TopoDS_Edge> out;
  for (int i = 1; i <= map.Extent(); ++i) {
    const TopoDS_Edge edge = TopoDS::Edge(map(i));
    TopoDS_Vertex first;
    TopoDS_Vertex last;
    TopExp::Vertices(edge, first, last);
    if (first.IsNull() || last.IsNull())
      continue;
    const gp_Pnt a = BRep_Tool::Pnt(first);
    const gp_Pnt b = BRep_Tool::Pnt(last);
    if (std::abs(a.X() - b.X()) <= 1e-9 && std::abs(a.Y() - b.Y()) <= 1e-9 &&
        std::abs(a.Z() - b.Z()) > 1e-9) {
      out.push_back(edge);
    }
  }
  return out;
}

std::vector<kf::ResolvedEdge> resolved(const Case &benchmark) {
  std::vector<kf::ResolvedEdge> out;
  for (std::size_t i = 0; i < benchmark.edges.size(); ++i) {
    out.push_back(
        {"el_" + std::to_string(i),
         em::ElementMapPartition::topokey_for_shape(
             benchmark.body, benchmark.edges[i], km::ElementKind::Edge),
         benchmark.id + ".input" + std::to_string(i), "body_box",
         benchmark.edges[i]});
  }
  return out;
}

double percentile(std::vector<double> values, double fraction) {
  std::sort(values.begin(), values.end());
  const std::size_t index =
      static_cast<std::size_t>(fraction * (values.size() - 1) + 0.5);
  return values[index];
}

Result run_case(const Case &benchmark, int warmups, int iterations) {
  Result result;
  result.id = benchmark.id;
  for (int i = -warmups; i < iterations; ++i) {
    const auto start = Clock::now();
    kf::FilletBuilder builder(benchmark.body, resolved(benchmark),
                              benchmark.radius);
    const kf::FilletBuildResult built = builder.build();
    const auto end = Clock::now();
    if (i >= 0) {
      result.milliseconds.push_back(
          std::chrono::duration<double, std::milli>(end - start).count());
      if (built.ok) {
        ++result.success_count;
        result.valid = result.valid && built.output_audit.publishable();
      } else {
        ++result.refusal_count;
        result.valid = result.valid && !built.diagnostics.empty();
      }
    }
  }
  return result;
}

} // namespace

int main(int argc, char **argv) {
  bool quick = false;
  std::string output_path;
  for (int i = 1; i < argc; ++i) {
    const std::string arg = argv[i];
    if (arg == "--quick")
      quick = true;
    if (arg == "--out" && i + 1 < argc)
      output_path = argv[++i];
  }
  const int warmups = quick ? 1 : 5;
  const int iterations = quick ? 3 : 30;
  const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
  const std::vector<TopoDS_Edge> vertical = vertical_edges(box);
  const std::vector<Case> cases = {
      {"box-single-r1", box, {vertical.front()}, 1.0},
      {"box-four-r1", box, vertical, 1.0},
      {"box-boundary-r4.9", box, {vertical.front()}, 4.9},
      {"box-refusal-r11", box, {vertical.front()}, 11.0},
  };

  std::string report = "# filletbench\n\n| case | p50 ms | p95 ms | success | "
                       "refusal | valid |\n"
                       "|---|---:|---:|---:|---:|:---:|\n";
  bool valid = true;
  for (const Case &benchmark : cases) {
    const Result result = run_case(benchmark, warmups, iterations);
    char row[256];
    std::snprintf(row, sizeof(row), "| %s | %.3f | %.3f | %d | %d | %s |\n",
                  result.id.c_str(), percentile(result.milliseconds, 0.50),
                  percentile(result.milliseconds, 0.95), result.success_count,
                  result.refusal_count, result.valid ? "yes" : "no");
    report += row;
    valid = valid && result.valid;
  }
  std::fputs(report.c_str(), stdout);
  if (!output_path.empty()) {
    std::ofstream output(output_path, std::ios::trunc);
    output << report;
    valid = valid && output.good();
  }
  return valid ? 0 : 1;
}
